//! Downloads that have done their job: a zip, 7z or rar file you've already unpacked (its folder
//! sits right next to it), and installers for programs that are installed. Old disk images can be
//! ticked. Exports of your own data (ChatGPT, Google Takeout) are pointed out and stay: they may
//! be your only copy. Everything goes to the Recycle Bin, so it can come back.
//!
//! Mom looks in your Downloads folder and in a Downloads folder at the top of each drive, one
//! level deep. Nothing inside an archive or an installer is run; a zip's own table of contents
//! is read to count its files, and an installer's name and version label say what it installs.

use super::{ago, bin, count_size, entries, name_of, tally, Cand, Found, TrashCtx, IN_USE};
use crate::installed::{squash, Installed};
use crate::paths::Paths;
use crate::util::norm_path;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Disk images unchanged for this long can be ticked.
const OLD_IMAGE_DAYS: u64 = 90;
/// A zip's table of contents bigger than this isn't read.
const MAX_ZIP_DIRECTORY: u64 = 64 << 20;
const MAX_ZIP_ENTRIES: u64 = 1_000_000;
/// Words in installers' names and labels that don't name the program.
const NOISE: &[&str] = &[
    "setup", "installer", "install", "installation", "web", "full", "offline", "online", "user", "system", "x64", "x86", "amd64",
    "arm64", "win64", "win32", "windows", "win", "64bit", "32bit", "64", "32", "bit", "release", "stable", "latest", "edition",
];

/// Your Downloads folder, and a Downloads folder at the top of each drive, each once.
pub(super) fn roots(paths: &Paths) -> Vec<PathBuf> {
    let mut out = vec![paths.downloads.clone()];
    for drive in std::iter::once(&paths.system_drive).chain(&paths.other_drives) {
        out.push(drive.join("Downloads"));
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|p| p.is_dir() && seen.insert(norm_path(&p.to_string_lossy())));
    out
}

fn ext_of(name: &str) -> String {
    Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// A data export: ChatGPT's ("<64 hex>-2026-01-30-20-47-27-<32 hex>", a folder or its zip) or
/// Google Takeout's ("takeout-20240101T120000Z-001.zip").
pub(super) fn export_kind(name: &str) -> Option<&'static str> {
    static OPENAI: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)^[0-9a-f]{64}-\d{4}-\d{2}-\d{2}-\d{2}-\d{2}-\d{2}-[0-9a-f]{32}(\.zip)?$").unwrap()
    });
    static TAKEOUT: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)^takeout-\d{8}T\d{6}Z-\d{3}\.(zip|tgz)$").unwrap());
    if OPENAI.is_match(name) {
        Some("ChatGPT")
    } else if TAKEOUT.is_match(name) {
        Some("Google Takeout")
    } else {
        None
    }
}

/// Names of files a zip holds that are checked against its folder.
const ZIP_SAMPLE: usize = 64;

/// What a zip says it holds, from its own table of contents: how many files (not folders), and
/// the first few of their names. None for anything that isn't a zip mom can read within bounds.
pub(super) fn zip_files(path: &Path) -> Option<(u64, Vec<String>)> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    if len < 22 {
        return None;
    }
    let tail_len = len.min(22 + 65_535);
    let mut tail = vec![0u8; tail_len as usize];
    f.seek(SeekFrom::Start(len - tail_len)).ok()?;
    f.read_exact(&mut tail).ok()?;
    let at = (0..=tail.len() - 22).rev().find(|&i| tail[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])?;
    let u16_at = |b: &[u8], i: usize| -> Option<u64> { Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?) as u64) };
    let u32_at = |b: &[u8], i: usize| -> Option<u64> { Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?) as u64) };
    let u64_at = |b: &[u8], i: usize| -> Option<u64> { b.get(i..i + 8)?.try_into().ok().map(u64::from_le_bytes) };
    let (mut total, mut size, mut offset) = (u16_at(&tail, at + 10)?, u32_at(&tail, at + 12)?, u32_at(&tail, at + 16)?);
    if total == 0xffff || size == 0xffff_ffff || offset == 0xffff_ffff {
        // Zip64: a locator just before points at the bigger record.
        let loc = at.checked_sub(20)?;
        if tail.get(loc..loc + 4)? != [0x50, 0x4b, 0x06, 0x07] {
            return None;
        }
        let record = u64_at(&tail, loc + 8)?;
        let mut big = [0u8; 56];
        f.seek(SeekFrom::Start(record)).ok()?;
        f.read_exact(&mut big).ok()?;
        if big[..4] != [0x50, 0x4b, 0x06, 0x06] {
            return None;
        }
        (total, size, offset) = (u64_at(&big, 32)?, u64_at(&big, 40)?, u64_at(&big, 48)?);
    }
    if total == 0 || total > MAX_ZIP_ENTRIES || size > MAX_ZIP_DIRECTORY || offset.checked_add(size)? > len {
        return None;
    }
    let mut dir = vec![0u8; size as usize];
    f.seek(SeekFrom::Start(offset)).ok()?;
    f.read_exact(&mut dir).ok()?;
    let (mut i, mut seen, mut files, mut names) = (0usize, 0u64, 0u64, Vec::new());
    while seen < total {
        if dir.get(i..i + 4)? != [0x50, 0x4b, 0x01, 0x02] {
            return None;
        }
        let (name_len, extra, comment) = (u16_at(&dir, i + 28)? as usize, u16_at(&dir, i + 30)? as usize, u16_at(&dir, i + 32)? as usize);
        let name = dir.get(i + 46..i + 46 + name_len)?;
        if !matches!(name.last(), Some(b'/' | b'\\')) {
            files += 1;
            if names.len() < ZIP_SAMPLE {
                if let Ok(n) = std::str::from_utf8(name) {
                    names.push(n.to_string());
                }
            }
        }
        i += 46 + name_len + extra + comment;
        seen += 1;
    }
    Some((files, names))
}

/// A name from a zip as a path inside its folder: plain parts only, never one that climbs out
/// or names a drive.
fn inside(folder: &Path, name: &str) -> Option<PathBuf> {
    let mut out = folder.to_path_buf();
    for part in name.split(['/', '\\']).filter(|p| !p.is_empty()) {
        if part == "." || part == ".." || part.contains(':') || part.chars().any(char::is_control) {
            return None;
        }
        out.push(part);
    }
    (out != folder).then_some(out)
}

/// Are the zip's files there in the folder, either as they are or one level down (the zip's own
/// top folder unpacked as the folder)? Nearly all of the ones looked at must be.
fn names_there(folder: &Path, names: &[String]) -> bool {
    let checked: Vec<&String> = names.iter().filter(|n| inside(folder, n).is_some()).collect();
    if checked.is_empty() {
        return false;
    }
    let found = checked
        .iter()
        .filter(|n| {
            let as_is = inside(folder, n).is_some_and(|p| p.is_file());
            let below = n.split_once(['/', '\\']).and_then(|(_, rest)| inside(folder, rest)).is_some_and(|p| p.is_file());
            as_is || below
        })
        .count();
    found * 10 >= checked.len() * 9
}

/// Does `folder` look like `archive` unpacked? For a zip, its file count is close to what the zip
/// holds and the files it names are there. For 7z and rar (whose contents mom doesn't read), the folder appeared after the
/// archive and holds at least about as much.
fn unpacked_into(archive: &Path, archive_bytes: u64, folder: &Path) -> bool {
    let t = tally(folder);
    if !t.complete || t.links > 0 || t.files == 0 {
        return false;
    }
    if ext_of(&name_of(archive)) == "zip" {
        let Some((n, names)) = zip_files(archive) else { return false };
        return n > 0 && t.files * 10 >= n * 9 && t.files <= n + n / 5 + 2 && names_there(folder, &names);
    }
    let created = |p: &Path| std::fs::symlink_metadata(p).ok().and_then(|m| super::cleanup_recency(&m));
    let (Some(made), Some(downloaded)) = (created(folder), std::fs::symlink_metadata(archive).ok().and_then(|m| m.modified().ok())) else { return false };
    let downloaded = downloaded.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(u64::MAX);
    made + 120 >= downloaded && t.bytes * 10 >= archive_bytes * 9
}

/// An installer's product name and description, from its version label. Read the way Explorer
/// reads it for a tooltip: nothing in the file runs.
#[cfg(windows)]
fn version_label(path: &Path) -> Vec<String> {
    use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    let w = crate::util::wide(&path.to_string_lossy());
    let mut handle = 0u32;
    // SAFETY: `w` is NUL-terminated and outlives the call.
    let size = unsafe { GetFileVersionInfoSizeW(w.as_ptr(), &mut handle) };
    if size == 0 || size > 1 << 20 {
        return vec![];
    }
    let mut data = vec![0u8; size as usize];
    // SAFETY: `data` holds `size` bytes, as passed.
    if unsafe { GetFileVersionInfoW(w.as_ptr(), 0, size, data.as_mut_ptr().cast()) } == 0 {
        return vec![];
    }
    let query = |key: &str| -> Option<Vec<u8>> {
        let k = crate::util::wide(key);
        let mut ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        // SAFETY: `data` is the block GetFileVersionInfoW filled; the result points inside it.
        if unsafe { VerQueryValueW(data.as_ptr().cast(), k.as_ptr(), &mut ptr, &mut len) } == 0 || ptr.is_null() {
            return None;
        }
        let start = (ptr as usize).checked_sub(data.as_ptr() as usize)?;
        let bytes = if key.starts_with("\\StringFileInfo") { len as usize * 2 } else { len as usize };
        data.get(start..start.checked_add(bytes)?).map(<[u8]>::to_vec)
    };
    let mut languages: Vec<String> = query("\\VarFileInfo\\Translation")
        .map(|t| t.chunks_exact(4).take(8).map(|c| format!("{:04x}{:04x}", u16::from_le_bytes([c[0], c[1]]), u16::from_le_bytes([c[2], c[3]]))).collect())
        .unwrap_or_default();
    languages.extend(["040904b0".to_string(), "040904e4".to_string()]);
    let mut out = Vec::new();
    for lang in languages {
        for field in ["ProductName", "FileDescription"] {
            if let Some(raw) = query(&format!("\\StringFileInfo\\{lang}\\{field}")) {
                let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|u| *u != 0).take(200).collect();
                let s = String::from_utf16_lossy(&units).trim().to_string();
                if !s.is_empty() && !out.contains(&s) {
                    out.push(s);
                }
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    out
}

#[cfg(not(windows))]
fn version_label(_: &Path) -> Vec<String> {
    vec![]
}

/// A name with installer words, versions and architectures taken out: "VSCodeUserSetup-x64-1.95.3"
/// -> "VSCodeUser", "Google Chrome Installer" -> "Google Chrome".
pub(super) fn program_name(s: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    for w in s.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()) {
        let lower = w.to_ascii_lowercase();
        // "DiscordSetup", "SteamInstaller": the word stuck on the end.
        let stem = ["installer", "install", "setup"].iter().find_map(|end| lower.strip_suffix(end).map(|s| &w[..s.len()])).filter(|s| s.len() >= 3).unwrap_or(w);
        let l = stem.to_ascii_lowercase();
        let version = l.trim_start_matches('v').chars().all(|c| c.is_ascii_digit());
        if NOISE.contains(&l.as_str()) || version {
            continue;
        }
        words.push(stem.to_string());
    }
    words.join(" ")
}

/// Says setup in its name or its label.
fn says_setup(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    ["setup", "install"].iter().any(|w| l.contains(w))
}

/// The installed program an installer in Downloads is for, if mom can tell: a .msi, or a .exe
/// that says it's a setup program, whose name matches something installed.
pub(super) fn installed_app(path: &Path, installed: &Installed) -> Option<String> {
    let name = name_of(path);
    let ext = ext_of(&name);
    if !matches!(ext.as_str(), "exe" | "msi") {
        return None;
    }
    let stem = Path::new(&name).file_stem()?.to_string_lossy().to_string();
    let label = if ext == "exe" { version_label(path) } else { vec![] };
    if ext == "exe" && !says_setup(&stem) && !label.iter().any(|l| says_setup(l)) {
        return None;
    }
    label.iter().map(|l| program_name(l)).chain(std::iter::once(program_name(&stem))).find(|n| squash(n).len() >= 4 && installed.has(n))
}

/// Everything in your Downloads folders mom has a word about.
pub(super) fn find(ctx: &TrashCtx) -> Found {
    let roots = roots(ctx.paths);
    if roots.is_empty() {
        return Found::none();
    }
    let looked_up;
    let installed = match &ctx.installed {
        Some(i) => i,
        None => {
            looked_up = Installed::scan(ctx.paths);
            &looked_up
        }
    };
    let mut f = Found::at(ctx.paths.downloads.clone());
    let mut room = bin::Room::default();
    for root in roots {
        let list = entries(&root);
        // Folders here, by name, for "already unpacked".
        let folders: HashMap<String, PathBuf> = list
            .iter()
            .filter(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()))
            .map(|p| (name_of(p).to_ascii_lowercase(), p.clone()))
            .collect();
        for path in list {
            let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
            if meta.file_type().is_symlink() {
                continue;
            }
            let name = name_of(&path);
            if meta.is_dir() {
                // An unpacked export: pointed out, and it stays.
                if let Some(kind) = export_kind(&name) {
                    let mut c = Cand::new(path.clone(), name, &tally(&path), ctx.now);
                    c.kind = Some("data exports");
                    f.cands.push(c.verdict(false, format!("Your {kind} export. It may be your only copy, so it stays")));
                }
                continue;
            }
            let ext = ext_of(&name);
            let stem = Path::new(&name).file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            let t = tally(&path);
            let mut c = Cand::new(path.clone(), name.clone(), &t, ctx.now).age_sensitive();
            let judged = if matches!(ext.as_str(), "zip" | "7z" | "rar") && folders.get(&stem).is_some_and(|d| unpacked_into(&path, t.bytes, d)) {
                c.kind = Some("unpacked archives");
                let into = folders.get(&stem).map(|d| name_of(d)).unwrap_or_default();
                Some(c.verdict(true, format!("Already unpacked into {into}")))
            } else if let Some(kind) = export_kind(&name) {
                c.kind = Some("data exports");
                Some(c.verdict(false, format!("Your {kind} export. It may be your only copy, so it stays")))
            } else if let Some(app) = installed_app(&path, installed) {
                c.kind = Some("installers");
                Some(c.verdict(true, format!("{app} is installed")))
            } else if matches!(ext.as_str(), "iso" | "img") && c.age_known && c.age >= OLD_IMAGE_DAYS * 86_400 {
                c.kind = Some("disk images");
                let r = format!("Unchanged for {}; tick it if you're done with it", ago(c.age));
                Some(c.kept_but_tickable(r))
            } else {
                None
            };
            let Some(mut c) = judged else { continue };
            if (c.goes || c.optional) && ctx.in_use(&path) {
                c.optional = false;
                c = c.verdict(false, IN_USE);
            } else if (c.goes || c.optional) && !room.fits(&path, c.bytes) {
                // Windows would delete it for good rather than keep it in the bin.
                c.optional = false;
                c = c.verdict(false, "Too big for the Recycle Bin, so mom leaves it to you");
            }
            f.cands.push(c);
        }
    }
    f
}

/// What cleaning does, for the spot's sheet.
pub(super) fn effect(n: usize, bytes: u64) -> String {
    if n == 0 {
        return "Nothing to tidy right now: no archives you've already unpacked, and no installers for programs that are installed.".into();
    }
    format!(
        "Moves {} to the Recycle Bin: archives you've already unpacked (the folders stay) and installers for programs \
         that are installed, plus any old disk images you tick. Changed your mind? Restore them from the Recycle Bin.",
        count_size(n, "file", "files", bytes)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TrashConfig;
    use crate::trash::tests::{ctx, fixture, write};
    use crate::trash::{choose, find as find_spot, measure, Pick};

    /// A small zip with these files (stored, no compression), built by hand.
    fn zip(path: &Path, names: &[&str]) {
        let mut body = Vec::new();
        let mut directory = Vec::new();
        for n in names {
            let offset = body.len() as u32;
            body.extend([0x50, 0x4b, 0x03, 0x04]);
            body.extend([0u8; 22]);
            body.extend((n.len() as u16).to_le_bytes());
            body.extend(0u16.to_le_bytes());
            body.extend(n.as_bytes());
            directory.extend([0x50, 0x4b, 0x01, 0x02]);
            directory.extend([0u8; 24]);
            directory.extend((n.len() as u16).to_le_bytes());
            directory.extend([0u8; 12]);
            directory.extend(offset.to_le_bytes());
            directory.extend(n.as_bytes());
        }
        let mut out = body.clone();
        let at = out.len() as u32;
        out.extend(&directory);
        out.extend([0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0]);
        out.extend((names.len() as u16).to_le_bytes());
        out.extend((names.len() as u16).to_le_bytes());
        out.extend((directory.len() as u32).to_le_bytes());
        out.extend(at.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn zips_say_how_many_files_they_hold() {
        let root = fixture("zips");
        let z = root.join("lanternfall.zip");
        zip(&z, &["lanternfall/", "lanternfall/readme.txt", "lanternfall/data/a.bin", "lanternfall/data/b.bin"]);
        let (n, names) = zip_files(&z).unwrap();
        assert_eq!(n, 3, "folders aren't files");
        assert_eq!(names, ["lanternfall/readme.txt", "lanternfall/data/a.bin", "lanternfall/data/b.bin"]);
        // Its names are looked for in its folder, as they are or one level down, never outside it.
        let unpacked = root.join("lanternfall");
        for n in ["readme.txt", "data/a.bin", "data/b.bin"] {
            std::fs::create_dir_all(unpacked.join(n).parent().unwrap()).unwrap();
            std::fs::write(unpacked.join(n), b"x").unwrap();
        }
        assert!(names_there(&unpacked, &names));
        assert!(!names_there(&root.join("elsewhere"), &names));
        assert_eq!(inside(&unpacked, "../../windows/system.ini"), None);
        assert_eq!(inside(&unpacked, "C:/windows"), None);
        assert!(!names_there(&unpacked, &["../lanternfall.zip".to_string()]), "a name that climbs out is never looked at");
        std::fs::write(root.join("broken.zip"), b"PK\x05\x06 not really").unwrap();
        assert_eq!(zip_files(&root.join("broken.zip")), None);
        std::fs::write(root.join("tiny.zip"), b"PK").unwrap();
        assert_eq!(zip_files(&root.join("tiny.zip")), None);
        let mut cut = std::fs::read(&z).unwrap();
        let len = cut.len();
        cut[len - 6..len - 2].copy_from_slice(&u32::MAX.to_le_bytes()); // a table of contents past the end
        std::fs::write(root.join("cut.zip"), cut).unwrap();
        assert_eq!(zip_files(&root.join("cut.zip")), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn names_say_what_they_install() {
        assert_eq!(program_name("VSCodeUserSetup-x64-1.95.3"), "VSCodeUser");
        assert_eq!(program_name("DiscordSetup"), "Discord");
        assert_eq!(program_name("Google Chrome Installer"), "Google Chrome");
        assert_eq!(program_name("node-v20.10.0-x64"), "node");
        assert_eq!(program_name("Lanternfall Setup 2.4.1"), "Lanternfall");
        assert_eq!(export_kind("95f53c838998ab9bfaddafae0e4695df093a793e983560f080a2812388ba9c8e-2026-01-30-20-47-27-23caa8a1242d404e83d7680f24be726a"), Some("ChatGPT"));
        assert_eq!(export_kind("takeout-20240101T120000Z-001.zip"), Some("Google Takeout"));
        assert_eq!(export_kind("holiday-2026-01-30.zip"), None);
    }

    #[test]
    fn done_downloads_go_to_the_bin_and_exports_stay() {
        let root = fixture("downloads");
        let mut paths = Paths::under(&root);
        let d = paths.downloads.clone();
        // Unpacked: the zip goes, its folder stays.
        zip(&d.join("Lanternfall.zip"), &["a.txt", "b.txt", "c.txt"]);
        for n in ["a.txt", "b.txt", "c.txt"] {
            write(&d.join("Lanternfall").join(n), 10, 30);
        }
        // A zip whose folder holds something else entirely stays.
        zip(&d.join("Copperline.zip"), &["one.txt", "two.txt", "three.txt", "four.txt", "five.txt", "six.txt", "seven.txt", "eight.txt"]);
        write(&d.join("Copperline").join("notes.txt"), 10, 30);
        // Installers: one for something installed, one for something that isn't.
        write(&d.join("NimbusSetup.exe"), 400, 3);
        write(&d.join("AtlasRunSetup.exe"), 300, 3);
        write(&d.join("storefront-tools-1.2.0-x64.msi"), 200, 3);
        // An old disk image can be ticked; a new one isn't listed.
        write(&d.join("windows.iso"), 900, 200);
        write(&d.join("fresh.iso"), 900, 5);
        // A ChatGPT export stays, pointed out.
        let export = "95f53c838998ab9bfaddafae0e4695df093a793e983560f080a2812388ba9c8e-2026-01-30-20-47-27-23caa8a1242d404e83d7680f24be726a";
        write(&d.join(export).join("conversations.json"), 50, 30);
        // A Downloads folder at the top of another drive is looked at too.
        let e = root.join("e");
        paths.other_drives = vec![e.clone()];
        write(&e.join("Downloads").join("NimbusSetup-2.exe"), 100, 3);
        let cfg = TrashConfig::default();
        let mut c = ctx(&paths, &cfg);
        c.installed = Some(Installed::with(&["Nimbus Tools", "storefront tools"]));
        let spot = measure("downloads", &c).unwrap();
        assert_eq!((spot.action.as_str(), spot.safety.as_str(), spot.group.as_str()), ("recycle", "review", "personal"));
        let state = |n: &str| spot.items.iter().find(|i| i.name == n).map(|i| (i.goes, i.optional));
        assert_eq!(state("Lanternfall.zip"), Some((true, false)));
        assert_eq!(state("Copperline.zip"), None, "not really unpacked: not listed");
        assert_eq!(state("NimbusSetup.exe"), Some((true, false)));
        assert_eq!(state("NimbusSetup-2.exe"), Some((true, false)));
        assert_eq!(state("storefront-tools-1.2.0-x64.msi"), Some((true, false)));
        assert_eq!(state("AtlasRunSetup.exe"), None, "not installed: old installers looks after it");
        assert_eq!(state("windows.iso"), Some((false, true)));
        assert_eq!(state("fresh.iso"), None);
        assert_eq!(state(export), Some((false, false)));
        assert!(spot.items.iter().find(|i| i.name == "NimbusSetup.exe").unwrap().reason.contains("is installed"));
        assert!(spot.effect.contains("Recycle Bin"), "{}", spot.effect);
        // What clean would hand to the Recycle Bin (not recycled in tests): the zip, never its folder.
        let f = find_spot("downloads", &c);
        let mut names: Vec<String> = choose(&f, None).iter().map(|c| name_of(&c.path)).collect();
        names.sort();
        assert_eq!(names, vec!["Lanternfall.zip", "NimbusSetup-2.exe", "NimbusSetup.exe", "storefront-tools-1.2.0-x64.msi"]);
        let pick = Pick { new: true, ..Default::default() };
        assert!(choose(&f, Some(&pick)).iter().any(|c| name_of(&c.path) == "windows.iso"), "Tick all takes the old image too");
        // Old installers leaves installed programs' installers to this spot.
        let old = measure("old-installers", &c).unwrap();
        assert!(old.items.iter().all(|i| i.name != "NimbusSetup.exe"));
        assert!(old.items.iter().any(|i| i.name == "AtlasRunSetup.exe"));
        // What you said to keep stays.
        let keep = TrashConfig { keep: vec![d.join("Lanternfall.zip").to_string_lossy().to_string()], ..TrashConfig::default() };
        let mut c = ctx(&paths, &keep);
        c.installed = Some(Installed::with(&[]));
        let spot = measure("downloads", &c).unwrap();
        assert!(spot.items.iter().find(|i| i.name == "Lanternfall.zip").is_some_and(|i| !i.goes && i.kept));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
