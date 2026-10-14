//! Old versions of drivers in Windows' driver store. Every driver update adds a package to
//! Windows\System32\DriverStore\FileRepository, and Windows keeps the old ones in case the new
//! one goes wrong. An old version no device uses, of a driver whose newer version is right there,
//! is only taking room.
//!
//! What's in the store comes from Windows' driver database in the registry (readable as you, and
//! the same in every language, unlike pnputil's printout). Which ones devices use comes from
//! Windows' device list (Win32_PnPSignedDriver). Only an administrator can remove a package, so
//! the elevated mom looks everything up again herself, and hands Windows' own pnputil nothing but
//! a published name ("oem42.inf") she checked. pnputil refuses a package a device still needs.

use super::{keep_yours, name_of, tally, Cand, CleanReport, Found, Pick, TrashCtx, MAX_KEPT};
use crate::paths::Paths;
use crate::util::{human_bytes, norm_path, plural};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;

/// One package in the driver store.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Package {
    /// Its published name: "oem42.inf".
    pub published: String,
    /// The .inf it was made from: "nv_dispi.inf".
    pub original: String,
    pub provider: String,
    /// Packed as Windows packs it: major, minor, build and revision, 16 bits each.
    pub version: u64,
    /// Its folder in FileRepository: "nv_dispi.inf_amd64_8a3c2d...".
    pub folder: String,
}

/// What's in the store, and which published names devices use. `in_use` is None when Windows
/// couldn't be asked: then nothing is offered.
pub(super) struct Store {
    pub packages: Vec<Package>,
    pub in_use: Option<HashSet<String>>,
}

/// The registry lists a few hundred packages on a typical PC; this many is plenty.
#[cfg_attr(test, allow(dead_code))]
const MAX_PACKAGES: usize = 5_000;
/// Which drivers devices use, asked at most this often while measuring (it takes a few seconds).
#[cfg_attr(test, allow(dead_code))]
const IN_USE_FOR: u64 = 1_800;
#[cfg_attr(test, allow(dead_code))]
static IN_USE: Mutex<Option<(u64, HashSet<String>)>> = Mutex::new(None);

/// "oem42.inf", and nothing else: the only thing that ever goes to pnputil.
pub(super) fn published_ok(name: &str) -> bool {
    let Some(digits) = name.strip_prefix("oem").and_then(|n| n.strip_suffix(".inf")) else { return false };
    !digits.is_empty() && digits.len() <= 6 && digits.bytes().all(|b| b.is_ascii_digit())
}

/// A package folder's name as Windows writes it ("nv_dispi.inf_amd64_8a3c2d..."): plain
/// characters, no path in it.
fn folder_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && name.contains(".inf_")
        && !name.contains("..")
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// The version packed in the registry's Version value: a header, the class, the driver date and
/// then the version, 8 bytes, little-endian. 0 when it isn't there.
fn version_of(data: &[u8]) -> u64 {
    data.get(32..40).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes).unwrap_or(0)
}

/// "32.0.15.6094".
pub(super) fn version_text(v: u64) -> String {
    format!("{}.{}.{}.{}", v >> 48, (v >> 32) & 0xffff, (v >> 16) & 0xffff, v & 0xffff)
}

fn repository(paths: &Paths) -> PathBuf {
    paths.windows.join("System32").join("DriverStore").join("FileRepository")
}

#[cfg(windows)]
#[cfg_attr(test, allow(dead_code))]
fn read_packages() -> Vec<Package> {
    use crate::health::reg::{binary_values, string_values, subkeys, Hive};
    const ROOT: &str = r"SYSTEM\DriverDatabase\DriverPackages";
    let mut out = Vec::new();
    for key in subkeys(Hive::LocalMachine, ROOT).into_iter().take(MAX_PACKAGES) {
        if !folder_ok(&key) {
            continue;
        }
        let path = format!(r"{ROOT}\{key}");
        let strings = string_values(Hive::LocalMachine, &path);
        let get = |n: &str| strings.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.trim().to_string()).unwrap_or_default();
        let published = get("").to_ascii_lowercase();
        if !published_ok(&published) {
            continue; // Windows' own drivers have no published name; they're never offered
        }
        let version = binary_values(Hive::LocalMachine, &path).get("Version").map(|v| version_of(v)).unwrap_or(0);
        out.push(Package { published, original: get("InfName").to_ascii_lowercase(), provider: get("Provider"), version, folder: key });
    }
    out
}

/// Published names of the drivers devices use, from Windows' device list.
#[cfg(windows)]
#[cfg_attr(test, allow(dead_code))]
fn read_in_use() -> Option<HashSet<String>> {
    let ps = crate::util::windows_powershell().ok()?;
    let out = crate::util::run(
        &ps,
        &["-NoProfile", "-NonInteractive", "-Command", "Get-CimInstance Win32_PnPSignedDriver -Property InfName | ForEach-Object { $_.InfName }"],
        std::time::Duration::from_secs(60),
    )
    .ok()?;
    if !out.ok() {
        return None;
    }
    let used: HashSet<String> = out.text().lines().take(100_000).map(|l| l.trim().to_ascii_lowercase()).filter(|l| published_ok(l)).collect();
    // Every PC has devices on drivers that came from outside Windows; none at all means the
    // answer is missing something, not that nothing's used.
    (!used.is_empty()).then_some(used)
}

/// A store tests make up: its packages, and which published names devices use.
#[cfg(test)]
type Fixture = (Vec<Package>, Option<HashSet<String>>);

#[cfg(test)]
thread_local! {
    /// What tests say is in the store: they never read this PC's drivers.
    static FIXTURE: std::cell::RefCell<Option<Fixture>> = const { std::cell::RefCell::new(None) };
    /// What tests' cleans asked pnputil to remove.
    static REMOVED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// What's in the store now. `fresh`: ask Windows which drivers are used again rather than use
/// the last answer (the administrator run always does).
fn store(fresh: bool, now: u64) -> Option<Store> {
    #[cfg(test)]
    {
        let _ = (fresh, now);
        FIXTURE.with(|f| f.borrow().clone()).map(|(packages, in_use)| Store { packages, in_use })
    }
    #[cfg(all(windows, not(test)))]
    {
        let packages = read_packages();
        if packages.is_empty() {
            return None;
        }
        let cached = (!fresh)
            .then(|| IN_USE.lock().unwrap_or_else(|e| e.into_inner()).clone())
            .flatten()
            .filter(|(at, _)| now.saturating_sub(*at) < IN_USE_FOR)
            .map(|(_, used)| used);
        let in_use = cached.or_else(|| {
            let used = read_in_use()?;
            *IN_USE.lock().unwrap_or_else(|e| e.into_inner()) = Some((now, used.clone()));
            Some(used)
        });
        Some(Store { packages, in_use })
    }
    #[cfg(all(not(windows), not(test)))]
    {
        let _ = (fresh, now);
        None
    }
}

/// Each package of a driver that has more than one version here, with whether it can go and why.
/// Only a version older than the newest, that no device uses, goes. A driver with one version
/// here is never listed.
pub(super) fn judge(packages: &[Package], in_use: &HashSet<String>) -> Vec<(Package, bool, String)> {
    let mut groups: HashMap<(String, String), Vec<&Package>> = HashMap::new();
    for p in packages.iter().filter(|p| !p.original.is_empty()) {
        groups.entry((p.original.clone(), p.provider.to_ascii_lowercase())).or_default().push(p);
    }
    let mut out = Vec::new();
    for list in groups.into_values().filter(|l| l.len() > 1) {
        let newest = list.iter().map(|p| p.version).max().unwrap_or(0);
        for p in list {
            let (goes, why) = if in_use.contains(&p.published) {
                (false, "A device uses it".to_string())
            } else if p.version == 0 || newest == 0 {
                (false, "Its version couldn't be read".to_string())
            } else if p.version >= newest {
                (false, "The newest version, kept".to_string())
            } else {
                (true, format!("Version {} replaced it, and no device uses it", version_text(newest)))
            };
            out.push((p.clone(), goes, why));
        }
    }
    out
}

/// The spot as it is now, and the store it was judged from.
fn scan(ctx: &TrashCtx, fresh: bool) -> (Found, Option<Store>) {
    let Some(store) = store(fresh, ctx.now) else { return (Found::none(), None) };
    let repo = repository(ctx.paths);
    let mut f = Found::at(repo.clone());
    let Some(in_use) = &store.in_use else {
        f.note = Some("Windows didn't say which drivers its devices use, so mom offers none. Check again in a moment.".into());
        return (f, Some(store));
    };
    for (p, goes, why) in judge(&store.packages, in_use) {
        let folder = repo.join(&p.folder);
        let maker = if p.provider.is_empty() { "Unknown maker" } else { p.provider.as_str() };
        let mut c = Cand::new(folder.clone(), format!("{maker} · {} {}", p.original, version_text(p.version)), &tally(&folder), ctx.now);
        c.sub = Some(format!("Driver package {}", p.published));
        c.kind = Some("old driver versions");
        // The newer version has to be really there for an older one to go.
        let c = if goes && !folder.is_dir() {
            c.verdict(false, "Its folder isn't where Windows says")
        } else {
            c.verdict(goes, why)
        };
        f.cands.push(c);
    }
    (f, Some(store))
}

pub(super) fn find(ctx: &TrashCtx) -> Found {
    // The administrator run is a fresh mom, so this is a fresh answer there anyway.
    scan(ctx, ctx.elevated).0
}

/// pnputil, from Windows' own folder (never PATH).
#[cfg(windows)]
#[cfg_attr(test, allow(dead_code))]
fn pnputil() -> Option<PathBuf> {
    let mut dir = [0u16; 32_768];
    // SAFETY: the owned buffer and its length are valid for the call.
    let len = unsafe { windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW(dir.as_mut_ptr(), dir.len() as u32) } as usize;
    if len == 0 || len >= dir.len() {
        return None;
    }
    let dir = PathBuf::from(String::from_utf16(&dir[..len]).ok()?);
    Some(dir.join("pnputil.exe")).filter(|p| dir.is_absolute() && p.is_file())
}

/// What pnputil is told: remove one package, without /force, so Windows refuses one a device
/// still needs. None for anything but a published name.
pub(super) fn pnputil_args(published: &str) -> Option<[&str; 2]> {
    published_ok(published).then_some(["/delete-driver", published])
}

/// Asks Windows to remove one package. True only if it said yes.
fn remove(published: &str) -> bool {
    let Some(args) = pnputil_args(published) else { return false };
    #[cfg(test)]
    {
        let _ = args;
        REMOVED.with(|r| r.borrow_mut().push(published.to_string()));
        true
    }
    #[cfg(all(windows, not(test)))]
    {
        let Some(tool) = pnputil() else { return false };
        crate::util::run(&tool.to_string_lossy(), &args, std::time::Duration::from_secs(120)).is_ok_and(|o| o.ok())
    }
    #[cfg(all(not(windows), not(test)))]
    {
        let _ = args;
        false
    }
}

/// Removes the ticked old versions. Only as administrator: as you there's nothing to do (the
/// widget runs the administrator part next). Everything is looked up again here; the pick only
/// says which of what mom finds now were ticked.
pub(super) fn clean(ctx: &TrashCtx, pick: Option<&Pick>, mut r: CleanReport) -> CleanReport {
    if !ctx.elevated {
        r.message = "Removing drivers needs an administrator, so Windows asks you first.".into();
        return r;
    }
    let (f, store) = scan(ctx, true);
    let Some(store) = store else {
        r.message = "Mom couldn't read Windows' driver store, so nothing was removed.".into();
        return r;
    };
    let keep = &ctx.cfg.keep[..ctx.cfg.keep.len().min(MAX_KEPT)];
    let mut f = keep_yours(f, keep);
    if let Some(p) = pick {
        f = keep_yours(f, &p.keep[..p.keep.len().min(MAX_KEPT)]);
    }
    for c in &mut f.cands {
        if !c.complete || c.links > 0 {
            c.goes = false;
            c.optional = false;
        }
    }
    let chosen = super::choose(&f, pick);
    if let Some(p) = pick {
        let taken: HashSet<String> = chosen.iter().map(|c| norm_path(&c.path.to_string_lossy())).collect();
        r.skipped += p.ticked.iter().filter(|t| !taken.contains(&norm_path(t))).count() as u64;
    }
    let published: HashMap<String, &str> = store.packages.iter().map(|p| (p.folder.to_ascii_lowercase(), p.published.as_str())).collect();
    if let Some(l) = ctx.live {
        l.start(&chosen);
    }
    for c in &chosen {
        if ctx.live.is_some_and(super::Live::stopped) {
            break;
        }
        if let Some(l) = ctx.live {
            l.working_on(c);
        }
        let name = published.get(&name_of(&c.path).to_ascii_lowercase()).copied().unwrap_or_default();
        let ok = remove(name) && {
            #[cfg(test)]
            {
                let _ = std::fs::remove_dir_all(&c.path);
            }
            !c.path.exists()
        };
        if ok {
            r.deleted += 1;
            r.freed += c.bytes;
        } else {
            r.skipped += 1;
        }
        if let Some(l) = ctx.live {
            l.files_gone(c.files.max(1));
            l.finished(c, ok, if ok { c.bytes } else { 0 });
        }
    }
    r.message = match (r.deleted, r.skipped) {
        (0, 0) => "Already spotless.".into(),
        (0, _) => "Nothing was removed: Windows still needs them, or they changed since mom looked.".into(),
        (n, 0) => format!("Removed {} ({}).", plural(n, "old driver version"), human_bytes(r.freed)),
        (n, k) => format!("Removed {} ({}). {} stayed: Windows still needs them, or they changed since mom looked.", plural(n, "old driver version"), human_bytes(r.freed), plural(k, "version")),
    };
    r
}

/// What cleaning does, for the spot's sheet.
pub(super) fn effect(n: usize, bytes: u64, listed: usize) -> String {
    if n == 0 {
        return if listed == 0 {
            "Nothing to clean right now: every driver here has just one version, or Windows didn't say which ones are in use.".into()
        } else {
            "Nothing to clean right now: each older version here is still used by a device.".into()
        };
    }
    format!(
        "Has Windows remove {} ({}) that no device uses. The newest version of each driver, and every version a \
         device uses, stays. Windows asks you first, and refuses any a device still needs.",
        plural(n as u64, "old driver version"),
        human_bytes(bytes)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TrashConfig;
    use crate::trash::tests::{ctx, fixture, write};
    use crate::trash::{clean, measure, plan};

    fn pkg(published: &str, original: &str, provider: &str, version: (u64, u64, u64, u64), folder: &str) -> Package {
        Package {
            published: published.into(),
            original: original.into(),
            provider: provider.into(),
            version: (version.0 << 48) | (version.1 << 32) | (version.2 << 16) | version.3,
            folder: folder.into(),
        }
    }

    fn store_of(packages: Vec<Package>, used: &[&str]) {
        let used = used.iter().map(|s| s.to_string()).collect();
        FIXTURE.with(|f| *f.borrow_mut() = Some((packages, Some(used))));
        REMOVED.with(|r| r.borrow_mut().clear());
    }

    #[test]
    fn names_and_versions_are_read_strictly() {
        assert!(published_ok("oem0.inf") && published_ok("oem123456.inf"));
        for bad in ["oem.inf", "oem1234567.inf", "OEM1.INF ", "oem1.inf /force", "oem1.inf&calc", "nv_dispi.inf", "..\\oem1.inf", "oem1.infx", "oem-1.inf"] {
            assert!(!published_ok(bad), "{bad}");
            assert_eq!(pnputil_args(bad), None, "{bad}");
        }
        assert_eq!(pnputil_args("oem42.inf"), Some(["/delete-driver", "oem42.inf"]), "never /force, never /uninstall");
        assert!(folder_ok("lanternfall.inf_amd64_0123456789abcdef"));
        assert!(!folder_ok("..\\windows") && !folder_ok("a.inf_x\\..\\b") && !folder_ok("plain"));
        // The registry's Version value: 8 header bytes, a 16-byte class, the date, the version.
        let mut data = vec![0u8; 48];
        data[32..40].copy_from_slice(&((10u64 << 48) | (15063 << 16)).to_le_bytes());
        assert_eq!(version_text(version_of(&data)), "10.0.15063.0");
        assert_eq!(version_of(&data[..20]), 0, "too short: unknown");
    }

    #[test]
    fn only_older_versions_nobody_uses_go() {
        let list = vec![
            pkg("oem1.inf", "lanternfall.inf", "Lanternfall", (2, 0, 0, 1), "lanternfall.inf_amd64_a1"),
            pkg("oem2.inf", "lanternfall.inf", "Lanternfall", (2, 1, 0, 0), "lanternfall.inf_amd64_a2"),
            pkg("oem3.inf", "lanternfall.inf", "Lanternfall", (3, 0, 0, 0), "lanternfall.inf_amd64_a3"),
            // An older version a device still uses stays, as does the newest nobody uses.
            pkg("oem4.inf", "copperline.inf", "Copperline", (1, 0, 0, 0), "copperline.inf_amd64_b1"),
            pkg("oem5.inf", "copperline.inf", "Copperline", (1, 5, 0, 0), "copperline.inf_amd64_b2"),
            // One version alone is never listed; the same .inf from another maker isn't the same driver.
            pkg("oem6.inf", "nimbus.inf", "Nimbus", (4, 0, 0, 0), "nimbus.inf_amd64_c1"),
            pkg("oem7.inf", "lanternfall.inf", "Atlas", (1, 0, 0, 0), "lanternfall.inf_amd64_d1"),
        ];
        let used: HashSet<String> = ["oem3.inf", "oem4.inf"].iter().map(|s| s.to_string()).collect();
        let mut got: Vec<(String, bool)> = judge(&list, &used).into_iter().map(|(p, goes, _)| (p.published, goes)).collect();
        got.sort();
        assert_eq!(got, vec![
            ("oem1.inf".into(), true),
            ("oem2.inf".into(), true),
            ("oem3.inf".into(), false),
            ("oem4.inf".into(), false),
            ("oem5.inf".into(), false),
        ]);
        let why = judge(&list, &used).into_iter().find(|(p, _, _)| p.published == "oem1.inf").unwrap().2;
        assert_eq!(why, "Version 3.0.0.0 replaced it, and no device uses it");
    }

    #[test]
    fn the_spot_lists_measures_and_removes_only_what_it_found_itself() {
        let root = fixture("drivers");
        let paths = Paths::under(&root);
        let repo = repository(&paths);
        for (folder, bytes) in [("lanternfall.inf_amd64_a1", 3000), ("lanternfall.inf_amd64_a2", 4000), ("copperline.inf_amd64_b1", 500), ("copperline.inf_amd64_b2", 600)] {
            write(&repo.join(folder).join("driver.sys"), bytes, 400);
        }
        store_of(
            vec![
                pkg("oem1.inf", "lanternfall.inf", "Lanternfall", (1, 0, 0, 0), "lanternfall.inf_amd64_a1"),
                pkg("oem2.inf", "lanternfall.inf", "Lanternfall", (2, 0, 0, 0), "lanternfall.inf_amd64_a2"),
                pkg("oem4.inf", "copperline.inf", "Copperline", (1, 0, 0, 0), "copperline.inf_amd64_b1"),
                pkg("oem5.inf", "copperline.inf", "Copperline", (1, 5, 0, 0), "copperline.inf_amd64_b2"),
            ],
            &["oem2.inf", "oem4.inf"],
        );
        let cfg = TrashConfig::default();
        let mut c = ctx(&paths, &cfg);
        let spot = measure("old-drivers", &c).unwrap();
        assert!(spot.admin && spot.available);
        assert_eq!((spot.reclaimable_items, spot.reclaimable_bytes), (1, 3000));
        assert_eq!(spot.kept_items, 3);
        let old = spot.items.iter().find(|i| i.goes).unwrap();
        assert_eq!(old.name, "Lanternfall · lanternfall.inf 1.0.0.0");
        assert!(spot.effect.contains("1 old driver version"), "{}", spot.effect);
        assert_eq!(plan("old-drivers", &c, None).len(), 1);
        // As you, nothing happens: the administrator run does it.
        let r = clean("old-drivers", &c, None);
        assert_eq!(r.deleted, 0);
        assert!(REMOVED.with(|r| r.borrow().is_empty()));
        // As administrator, only the one that goes, by its published name.
        c.elevated = true;
        let r = clean("old-drivers", &c, None);
        assert_eq!((r.deleted, r.freed), (1, 3000));
        assert_eq!(REMOVED.with(|r| r.borrow().clone()), vec!["oem1.inf".to_string()]);
        assert!(!repo.join("lanternfall.inf_amd64_a1").exists() && repo.join("lanternfall.inf_amd64_a2").exists());
        // A ticked path that isn't one of them is never removed.
        let pick = Pick { goes: false, ticked: vec![repo.join("copperline.inf_amd64_b1").to_string_lossy().to_string()], ..Default::default() };
        let r = clean("old-drivers", &c, Some(&pick));
        assert_eq!((r.deleted, r.skipped), (0, 1));
        assert_eq!(REMOVED.with(|r| r.borrow().len()), 1);
        // What you said to keep stays.
        write(&repo.join("lanternfall.inf_amd64_a1").join("driver.sys"), 3000, 400);
        let keep = TrashConfig { keep: vec![repo.join("lanternfall.inf_amd64_a1").to_string_lossy().to_string()], ..TrashConfig::default() };
        let mut c = ctx(&paths, &keep);
        c.elevated = true;
        assert_eq!(clean("old-drivers", &c, None).deleted, 0);
        FIXTURE.with(|f| *f.borrow_mut() = None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn without_knowing_what_devices_use_nothing_is_offered() {
        let root = fixture("drivers-unknown");
        let paths = Paths::under(&root);
        FIXTURE.with(|f| *f.borrow_mut() = Some((vec![
            pkg("oem1.inf", "lanternfall.inf", "Lanternfall", (1, 0, 0, 0), "lanternfall.inf_amd64_a1"),
            pkg("oem2.inf", "lanternfall.inf", "Lanternfall", (2, 0, 0, 0), "lanternfall.inf_amd64_a2"),
        ], None)));
        let cfg = TrashConfig::default();
        let spot = measure("old-drivers", &ctx(&paths, &cfg)).unwrap();
        assert_eq!(spot.reclaimable_items, 0);
        assert!(spot.note.as_deref().is_some_and(|n| n.contains("didn't say")));
        FIXTURE.with(|f| *f.borrow_mut() = None);
        assert!(!measure("old-drivers", &ctx(&paths, &cfg)).unwrap().available, "no store: not available");
        let _ = std::fs::remove_dir_all(&root);
    }
}
