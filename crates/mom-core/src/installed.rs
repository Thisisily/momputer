//! What's installed: programs in Windows' uninstall list, apps in %LOCALAPPDATA%\Programs and
//! Program Files, and Store apps. Used to tell whether a data folder's app is still around.
//!
//! Matching a folder like `%APPDATA%\Bionic` or `~\.lmstudio` to a program is guesswork, so it
//! leans towards "installed": a wrong guess then only means mom leaves something alone.

use crate::paths::Paths;
use crate::procs::ProcTable;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Default, Debug)]
pub struct Installed {
    /// Every name an installed program goes by, squashed ("visualstudiocode", "jetbrainstoolbox").
    names: Vec<String>,
    /// Single words of those names ("unity", "android"), matched only whole.
    words: Vec<String>,
    /// Where installed programs live.
    pub dirs: Vec<PathBuf>,
}

/// Lowercase letters and digits only: "JetBrains Toolbox" -> "jetbrainstoolbox".
pub fn squash(s: &str) -> String {
    s.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}

/// A name without a version on the end: "PyCharm2023.1" -> "PyCharm", "AndroidStudio2022.3" -> "AndroidStudio".
pub fn unversioned(name: &str) -> &str {
    let cut = name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.' || c == '-' || c == '_' || c == ' ');
    if cut.len() >= 3 { cut } else { name }
}

impl Installed {
    pub fn scan(paths: &Paths) -> Installed {
        let mut me = Installed::default();
        #[cfg(windows)]
        {
            use crate::health::reg::{string_values, subkeys, Hive};
            const UNINSTALL: &[(Hive, &str)] = &[
                (Hive::LocalMachine, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
                (Hive::LocalMachine, r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
                (Hive::CurrentUser, r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
            ];
            for (hive, root) in UNINSTALL {
                for key in subkeys(*hive, root) {
                    let values = string_values(*hive, &format!(r"{root}\{key}"));
                    let get = |n: &str| values.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.trim().trim_matches('"').to_string());
                    let Some(name) = get("DisplayName").filter(|n| !n.is_empty()) else { continue };
                    me.add_name(&name);
                    if let Some(p) = get("Publisher") {
                        me.add_name(&p);
                    }
                    if let Some(dir) = get("InstallLocation").filter(|d| !d.is_empty()) {
                        me.add_dir(PathBuf::from(dir));
                    }
                    // Its uninstaller usually sits in its own folder.
                    if let Some(u) = get("UninstallString") {
                        let exe = if let Some(rest) = u.strip_prefix('"') { rest.split('"').next().unwrap_or("") } else { u.split(".exe").next().unwrap_or("") };
                        if let Some(dir) = Path::new(exe).parent().filter(|d| d.components().count() > 1) {
                            me.add_dir(dir.to_path_buf());
                        }
                    }
                    if let Some(icon) = get("DisplayIcon") {
                        // "C:\Program Files\App\app.exe,0" -> its folder.
                        let exe = icon.split(',').next().unwrap_or("").trim_matches('"');
                        if let Some(dir) = Path::new(exe).parent().filter(|d| d.components().count() > 1) {
                            me.add_dir(dir.to_path_buf());
                        }
                    }
                }
            }
        }
        let program_files: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
            .iter()
            .filter_map(|k| std::env::var_os(k).map(PathBuf::from))
            .chain([paths.local_appdata.join("Programs")])
            .collect();
        for root in program_files {
            for app in dirs_in(&root) {
                // Vendors keep each product in its own folder: Google\Chrome, JetBrains\PyCharm 2024.2.
                for product in dirs_in(&app) {
                    me.add_name(&name_of(&product));
                }
                me.add_dir(app);
            }
        }
        // Package managers that don't register with Windows: Chocolatey, Scoop, winget.
        let choco = std::env::var_os("ChocolateyInstall").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\ProgramData\chocolatey"));
        for pkg in dirs_in(&choco.join("lib")).into_iter().chain(dirs_in(&paths.home.join("scoop").join("apps"))) {
            me.add_name(&name_of(&pkg));
        }
        for pkg in dirs_in(&paths.local_appdata.join("Microsoft").join("WinGet").join("Packages")) {
            // "nektos.act_Microsoft.Winget.Source_8wekyb3d8bbwe" -> "nektos", "act".
            for part in name_of(&pkg).split('_').next().unwrap_or("").split('.') {
                me.add_name(part);
            }
        }
        // Tools that are only on PATH (Flutter unzipped into C:\flutter, say).
        if let Some(path) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path) {
                let under = |d: &Path| d.components().count() > 2;
                for d in [Some(dir.as_path()), dir.parent()].into_iter().flatten().filter(|d| under(d)) {
                    let n = name_of(d);
                    if !n.eq_ignore_ascii_case("bin") {
                        me.add_name(&n);
                    }
                }
            }
        }
        // Store apps: "OpenAI.Codex_2p2nqsd0c76g0" -> "OpenAI", "Codex".
        for pkg in dirs_in(&paths.local_appdata.join("Packages")) {
            let family = name_of(&pkg);
            for part in family.split('_').next().unwrap_or("").split('.') {
                me.add_name(part);
            }
        }
        me.names.sort();
        me.names.dedup();
        me.words.sort();
        me.words.dedup();
        me
    }

    fn add_name(&mut self, name: &str) {
        let s = squash(unversioned(name));
        if s.len() >= 3 {
            self.names.push(s);
        }
        // Each word too, for "Unity Hub" or "Android Studio".
        for w in name.split(|c: char| !c.is_ascii_alphanumeric()) {
            let w = squash(unversioned(w));
            if w.len() >= 4 {
                self.words.push(w);
            }
        }
    }

    /// Its folder's name and the vendor folder above it ("Android" for Android\Android Studio),
    /// unless that's just Program Files or the like.
    fn add_dir(&mut self, dir: PathBuf) {
        self.add_name(&name_of(&dir));
        if let Some(parent) = dir.parent() {
            let n = name_of(parent);
            let plain = ["program files", "program files (x86)", "programs", "local", "appdata", "users", "tools", "bin", ""];
            if !plain.contains(&n.to_ascii_lowercase().as_str()) {
                self.add_name(&n);
            }
        }
        self.dirs.push(dir);
    }

    /// Is something installed that goes by this folder's name? `~\.lmstudio`, `%APPDATA%\Bionic`,
    /// `JetBrains\PyCharm2023.1`. Too short to tell (under 3 letters) counts as installed.
    pub fn has(&self, folder: &str) -> bool {
        let want = squash(unversioned(folder.trim_start_matches('.')));
        if want.len() < 3 {
            return true;
        }
        // A whole name either way round ("jetbrainstoolbox" has "jetbrains"; "pycharmce" has
        // "pycharm"), but a single word only as itself, or "studio" would claim "lmstudio".
        self.words.contains(&want)
            || self.names.iter().any(|n| n == &want || (want.len() >= 4 && n.contains(&want)) || (n.len() >= 5 && want.contains(n.as_str())))
    }

    /// For tests and for apps mom knows by other names.
    pub fn with(names: &[&str]) -> Installed {
        let mut me = Installed::default();
        for n in names {
            me.add_name(n);
        }
        me
    }
}

// ------------------------------------------------------------------ programs you don't use

/// A program in Windows' uninstall list.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct App {
    /// Its entry in the registry ("HKCU\\Software\\...\\Uninstall\\slack"). The Uninstall button
    /// names it by this, and mom reads the uninstaller from there, never from the widget.
    pub key: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
    /// Bytes, as the program says (0 when it doesn't).
    pub size: u64,
    /// When you last opened it, as far as Windows and its data folders tell (unix seconds).
    pub last_used: Option<u64>,
    #[serde(skip)]
    install_dir: Option<PathBuf>,
}

/// Where Windows keeps the uninstall list: (hive name, path).
const UNINSTALL_ROOTS: &[(&str, &str)] = &[
    ("HKLM", r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
    ("HKLM", r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
    ("HKCU", r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
];

/// Wallets, password managers and messengers: what's in them can't be got back, so mom never
/// suggests removing them or anything of theirs.
pub fn hands_off(name: &str) -> bool {
    const WORDS: &[&str] = &[
        "wallet", "coin", "ledger", "trezor", "keepkey", "atomic", "exodus", "electrum", "metamask", "binance", "kraken",
        "bitcoin", "crypto", "guarda", "gnosis", "safe electron", "firefly", "iota", "monero", "sparrow", "wasabi",
        "bitwarden", "keepass", "1password", "lastpass", "dashlane", "authy", "authenticator",
        "signal", "whatsapp", "telegram", "discord", "slack", "element", "threema", "wire",
    ];
    let n = name.to_ascii_lowercase();
    WORDS.iter().any(|w| n.contains(w))
}

/// Runtimes, drivers and pieces of Windows: other programs need them, so they're never "unused".
fn is_plumbing(name: &str, publisher: &str) -> bool {
    let (n, p) = (name.to_ascii_lowercase(), publisher.to_ascii_lowercase());
    const PUBLISHERS: &[&str] = &[
        "microsoft", "nvidia", "intel", "advanced micro devices", "amd", "realtek", "python software foundation", "oracle",
        "node.js", "git development", "apple inc", "qualcomm", "broadcom", "mediatek", "synaptics", "dolby", "waves",
    ];
    const WORDS: &[&str] = &[
        "runtime", "redistributable", "redist", "driver", "sdk", "toolkit", "build tools", "update", "service", "helper",
        "visual c++", ".net", "webview", "windows kit", "framework", "library", "plug-in", "plugin", "add-in", "extension",
        "codec", "directx", "vulkan", "physx", "python", "java", "node.js", "git ", "firmware", "bios", "chipset", "audio",
        // used from Explorer's right-click menu, which leaves no trace of use
        "winrar", "7-zip", "peazip", "bandizip",
    ];
    PUBLISHERS.iter().any(|x| p.contains(x)) || WORDS.iter().any(|x| n.contains(x))
}

/// Every program in the uninstall list a person installed (not updates or system parts).
pub fn programs() -> Vec<App> {
    let mut out: Vec<App> = Vec::new();
    #[cfg(windows)]
    {
        use crate::health::reg::{dword_value, string_values, subkeys, Hive};
        for (hive_name, root) in UNINSTALL_ROOTS {
            let hive = if *hive_name == "HKLM" { Hive::LocalMachine } else { Hive::CurrentUser };
            for sub in subkeys(hive, root) {
                let path = format!(r"{root}\{sub}");
                let values = string_values(hive, &path);
                let get = |n: &str| values.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.trim().to_string()).unwrap_or_default();
                let name = get("DisplayName");
                if name.is_empty() || get("UninstallString").is_empty() || !get("ParentKeyName").is_empty() || !get("ReleaseType").is_empty() {
                    continue;
                }
                if dword_value(hive, &path, "SystemComponent") == Some(1) || get("UninstallString").starts_with("steam://") {
                    continue;
                }
                if out.iter().any(|a| a.name == name) {
                    continue;
                }
                // Where it's installed: as it says, or where its icon or uninstaller is.
                let icon = get("DisplayIcon");
                let icon_dir = Path::new(icon.split(',').next().unwrap_or("").trim_matches('"')).parent().map(|d| d.to_string_lossy().to_string());
                let dir = Some(get("InstallLocation").trim_matches('"').to_string())
                    .filter(|d| !d.is_empty())
                    .or(icon_dir.filter(|d| !d.is_empty()))
                    .or_else(|| crate::elevate::split_command(&get("UninstallString")).and_then(|(exe, _)| Path::new(&exe).parent().map(|d| d.to_string_lossy().to_string())))
                    .map(PathBuf::from)
                    .filter(|d| d.components().count() > 2);
                out.push(App {
                    key: format!(r"{hive_name}\{path}"),
                    publisher: get("Publisher"),
                    version: get("DisplayVersion"),
                    size: dword_value(hive, &path, "EstimatedSize").map(|kb| kb as u64 * 1024).unwrap_or(0),
                    last_used: None,
                    install_dir: dir,
                    name,
                });
            }
        }
    }
    out
}

/// Programs launched from Start or Explorer, and when, from Windows' own record (UserAssist).
/// (Path or shortcut, unix seconds.)
fn launches() -> Vec<(String, u64)> {
    let mut out = Vec::new();
    #[cfg(windows)]
    {
        use crate::health::reg::{binary_values, subkeys, Hive};
        let root = r"Software\Microsoft\Windows\CurrentVersion\Explorer\UserAssist";
        // Known folders it writes as GUIDs.
        let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
        let pf86 = std::env::var("ProgramFiles(x86)").unwrap_or_else(|_| r"C:\Program Files (x86)".into());
        let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
        let known = [
            ("{6D809377-6AF0-444B-8957-A3773F02200E}", pf),
            ("{7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}", pf86),
            ("{F1B32785-6FBA-4FCF-9D55-7B8E7F157091}", local),
        ];
        for guid in subkeys(Hive::CurrentUser, root) {
            for (name, data) in binary_values(Hive::CurrentUser, &format!(r"{root}\{guid}\Count")) {
                if data.len() < 68 {
                    continue;
                }
                let ft = u64::from_le_bytes(data[60..68].try_into().unwrap_or([0; 8]));
                if ft < 116_444_736_000_000_000 {
                    continue; // never, or not a date
                }
                let when = (ft - 116_444_736_000_000_000) / 10_000_000;
                // The names are ROT13'd.
                let mut path: String = name
                    .chars()
                    .map(|c| match c {
                        'a'..='z' => (((c as u8 - b'a' + 13) % 26) + b'a') as char,
                        'A'..='Z' => (((c as u8 - b'A' + 13) % 26) + b'A') as char,
                        _ => c,
                    })
                    .collect();
                for (g, dir) in &known {
                    if path.starts_with(g) {
                        path = format!("{dir}{}", &path[g.len()..]);
                    }
                }
                out.push((path, when));
            }
        }
    }
    out
}

/// Newest change among a folder and what's directly in it: cheap, and an app that runs
/// writes something near the top (settings, logs).
fn touched(dir: &Path) -> u64 {
    let own = std::fs::metadata(dir).map(|m| crate::util::mtime_secs(&m)).unwrap_or(0);
    std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(Result::ok).filter_map(|e| e.metadata().ok()).map(|m| crate::util::mtime_secs(&m)).max().unwrap_or(0))
        .unwrap_or(0)
        .max(own)
}

/// When each program was last used: now if it's running, else the latest of when Windows saw
/// it launched and when its data folders changed. None when nothing says.
fn last_used(apps: &mut [App], paths: &Paths, table: &ProcTable, now: u64) {
    let launched = launches();
    let low = paths.local_appdata.parent().map(|a| a.join("LocalLow")).unwrap_or_default();
    let data_dirs: Vec<(String, PathBuf)> = [&paths.appdata, &paths.local_appdata, &low]
        .iter()
        .flat_map(|r| dirs_in(r))
        .map(|d| (squash(unversioned(&name_of(&d))), d))
        .filter(|(n, _)| n.len() >= 4)
        .collect();
    for app in apps.iter_mut() {
        let me = squash(unversioned(&app.name));
        let dir_name = app.install_dir.as_deref().map(|d| squash(&name_of(d))).unwrap_or_default();
        let dir_lower = app.install_dir.as_deref().map(|d| d.to_string_lossy().to_ascii_lowercase());
        let mut best: Option<u64> = None;
        let mut saw = |t: u64| best = Some(best.map_or(t, |b: u64| b.max(t)));
        if let Some(dir) = &dir_lower {
            if table.procs.values().any(|p| p.exe.as_deref().is_some_and(|e| e.to_ascii_lowercase().starts_with(dir.as_str()))) {
                saw(now);
            }
        }
        for (what, when) in &launched {
            let w = what.to_ascii_lowercase();
            let by_dir = dir_lower.as_deref().is_some_and(|d| w.starts_with(d));
            let by_shortcut = w.ends_with(".lnk") && {
                let stem = squash(Path::new(&w).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default().as_str());
                stem.len() >= 4 && (stem == me || me.starts_with(&stem) || stem.starts_with(&me))
            };
            if by_dir || by_shortcut {
                saw(*when);
            }
        }
        for (n, d) in &data_dirs {
            if *n == me || (me.len() >= 5 && n.starts_with(&me)) || (!dir_name.is_empty() && *n == dir_name) {
                saw(touched(d));
            }
        }
        app.last_used = best;
    }
}

/// Programs nobody has opened in a year, biggest first. Runtimes, drivers, system parts, command
/// line tools (on PATH) and anything with no sign either way are left out.
static CACHE: std::sync::Mutex<Option<(u64, Vec<App>)>> = std::sync::Mutex::new(None);

/// Forget the list, so the next look sees what was uninstalled.
pub fn forget_unused() {
    if let Ok(mut c) = CACHE.lock() {
        *c = None;
    }
}

pub fn unused(paths: &Paths, table: &ProcTable, now: u64) -> Vec<App> {
    if let Ok(c) = CACHE.lock() {
        if let Some((at, apps)) = c.as_ref() {
            if now.saturating_sub(*at) < 600 {
                return apps.clone();
            }
        }
    }
    let on_path: Vec<String> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.to_string_lossy().to_ascii_lowercase()).collect())
        .unwrap_or_default();
    let mut apps: Vec<App> = programs()
        .into_iter()
        .filter(|a| !is_plumbing(&a.name, &a.publisher) && !hands_off(&a.name) && !hands_off(&a.publisher))
        .filter(|a| {
            // A command line tool is used from a terminal, which leaves no trace: anything on PATH
            // stays off the list, matched by its folder or, failing that, its name.
            let d = a.install_dir.as_deref().map(|d| d.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            let me = squash(unversioned(&a.name));
            let named = |p: &String| {
                let p = Path::new(p);
                [p.file_name(), p.parent().and_then(Path::file_name)].into_iter().flatten().any(|n| {
                    let n = squash(&n.to_string_lossy());
                    n.len() >= 4 && (n == me || me.starts_with(&n))
                })
            };
            !on_path.iter().any(|p| (!d.is_empty() && p.starts_with(&d)) || named(p))
        })
        .collect();
    last_used(&mut apps, paths, table, now);
    let mut out: Vec<App> = apps.into_iter().filter(|a| a.last_used.is_some_and(|t| now.saturating_sub(t) >= 365 * 86_400)).collect();
    out.sort_by(|a, b| b.size.cmp(&a.size).then(a.last_used.cmp(&b.last_used)));
    out.truncate(30);
    if let Ok(mut c) = CACHE.lock() {
        *c = Some((now, out.clone()));
    }
    out
}

/// Starts a program's own uninstaller (Windows asks for permission if it needs it). The
/// uninstaller comes from the registry entry `key` names, which must be in the uninstall list.
pub fn uninstall(key: &str) -> Result<String, String> {
    #[cfg(windows)]
    {
        use crate::health::reg::{string_values, Hive};
        let (hive_name, path) = key.split_once('\\').ok_or("Not an uninstall entry.")?;
        if !UNINSTALL_ROOTS.iter().any(|(h, r)| *h == hive_name && path.to_ascii_lowercase().starts_with(&format!("{}\\", r.to_ascii_lowercase()))) {
            return Err("Not an uninstall entry.".into());
        }
        let hive = if hive_name == "HKLM" { Hive::LocalMachine } else { Hive::CurrentUser };
        let values = string_values(hive, path);
        let get = |n: &str| values.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.trim().to_string()).unwrap_or_default();
        let name = get("DisplayName");
        let (exe, args) = crate::elevate::split_command(&get("UninstallString")).ok_or("It has no uninstaller.")?;
        crate::elevate::open(Path::new(&exe), &args)?;
        Ok(format!("{name}'s uninstaller is open. Follow it to the end."))
    }
    #[cfg(not(windows))]
    {
        let _ = key;
        Err("Only on Windows.".into())
    }
}

fn dirs_in(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(Result::ok).filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false)).map(|e| e.path()).collect())
        .unwrap_or_default()
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_match_programs_by_name_leaning_towards_installed() {
        let i = Installed::with(&["Visual Studio Code", "JetBrains Toolbox", "PyCharm 2024.2.1", "Bionic", "Unity Hub 3.7.0"]);
        assert!(i.has("PyCharm2023.1"), "an older version's folder belongs to the same program");
        assert!(i.has("Bionic"));
        assert!(i.has("JetBrains"));
        assert!(i.has("UnityHub"));
        assert!(i.has("go"), "too short to tell");
        assert!(!i.has(".lmstudio"));
        assert!(!i.has("Genymobile"));
        assert_eq!(unversioned("AndroidStudio2022.3"), "AndroidStudio");
        assert_eq!(squash("Code - Insiders"), "codeinsiders");
    }

    #[test]
    fn runtimes_and_drivers_are_never_unused() {
        assert!(is_plumbing("Microsoft Visual C++ 2015-2022 Redistributable (x64)", "Microsoft Corporation"));
        assert!(is_plumbing("NVIDIA Graphics Driver 566.03", "NVIDIA Corporation"));
        assert!(is_plumbing("Python 3.11.5 (64-bit)", "Python Software Foundation"));
        assert!(!is_plumbing("Atlas Recorder", "Atlas"));
        assert!(!is_plumbing("Nimbus Notes", "Nimbus Labs"));
        assert!(hands_off("Gnosis Safe 2.0") && hands_off("Pocket Wallet") && hands_off("Signal 5.33.0") && !hands_off("Atlas Recorder"));
    }

    #[test]
    fn only_entries_in_the_uninstall_list_can_be_uninstalled() {
        assert!(uninstall(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run").is_err());
        assert!(uninstall("not a key").is_err());
    }

    #[test]
    fn this_pc_has_something_installed() {
        let i = Installed::scan(&Paths::detect());
        #[cfg(windows)]
        assert!(!i.names.is_empty() && !i.words.is_empty());
        let _ = i;
    }
}
