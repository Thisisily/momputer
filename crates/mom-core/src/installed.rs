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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LastUsedSource {
    Played,
    Updated,
    Activity,
}

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
    /// Bytes: as the program (or Steam) says, or measured from its folder when it doesn't say.
    /// 0 only when there's no telling.
    pub size: u64,
    /// Latest recorded evidence, in unix seconds. The source distinguishes play from an
    /// update or filesystem/launch activity; none proves that no later use occurred.
    pub last_used: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_source: Option<LastUsedSource>,
    /// Where it's installed, when that's known: its own folder, never one it shares.
    pub dir: Option<String>,
    /// The launcher it belongs to, when it has one ("Steam"): that's who removes it.
    pub from: Option<String>,
    #[serde(skip)]
    install_dir: Option<PathBuf>,
    /// A Steam game's number.
    #[serde(skip)]
    steam: Option<String>,
}

/// Where a program is installed: where it says, or the folder of its own program file (its icon,
/// when the icon is the program itself) or of its own uninstaller. Never a folder many programs
/// share: Steam's icon cache, Windows' installer cache, a launcher's or Windows' own uninstaller.
/// A shared folder says nothing about one program, and its name can match something unrelated.
fn own_folder(location: &str, icon: &str, uninstall: &str) -> Option<PathBuf> {
    let said = Some(location.trim().trim_matches('"')).filter(|d| !d.is_empty()).map(PathBuf::from);
    let icon_file = Path::new(icon.split(',').next().unwrap_or("").trim().trim_matches('"'));
    let by_icon = icon_file.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")).then(|| icon_file.parent().map(Path::to_path_buf)).flatten();
    let by_uninstaller = crate::elevate::split_command(uninstall).and_then(|(exe, _)| {
        let exe = PathBuf::from(exe);
        let name = exe.file_name()?.to_string_lossy().to_ascii_lowercase();
        let shared = ["msiexec.exe", "msiexec", "steam.exe", "rundll32.exe", "cmd.exe", "powershell.exe"].contains(&name.as_str());
        (!shared).then(|| exe.parent().map(Path::to_path_buf)).flatten()
    });
    let shared = |d: &PathBuf| {
        let l = d.to_string_lossy().to_ascii_lowercase();
        let l = l.trim_end_matches('\\');
        l.contains(r"\windows\installer") || l.contains(r"\package cache") || l.ends_with(r"\steam\steam\games") || l.ends_with(r"\windows") || l.ends_with(r"\system32")
    };
    [said, by_icon, by_uninstaller].into_iter().flatten().find(|d| d.components().count() > 2 && !shared(d))
}

/// The number Steam knows a game by, from its uninstall entry ("Steam App 2996080", or an
/// uninstaller that is `steam.exe steam://uninstall/2996080`).
fn steam_id(key: &str, uninstall: &str) -> Option<String> {
    let digits = |s: &str| s.chars().take_while(char::is_ascii_digit).collect::<String>();
    let id = match uninstall.split_once("steam://uninstall/") {
        Some((_, rest)) => digits(rest),
        None => digits(key.strip_prefix("Steam App ")?),
    };
    (!id.is_empty() && id.len() <= 10).then_some(id)
}

/// What Steam itself records about an installed game.
#[derive(Clone, Debug, PartialEq)]
struct SteamGame {
    /// What Steam calls it.
    name: String,
    dir: PathBuf,
    size: u64,
    /// When it was last played; never played, when it was installed or last updated.
    last: u64,
    source: LastUsedSource,
}

/// One of Steam's own records of an installed game (steamapps\appmanifest_<id>.acf), read as
/// the plain "key" "value" lines it is.
fn steam_game(library: &Path, manifest: &str) -> Option<SteamGame> {
    let value = |key: &str| {
        manifest.lines().find_map(|l| {
            let mut parts = l.trim().split('"');
            (parts.nth(1)? == key).then(|| parts.nth(1).map(str::to_string)).flatten()
        })
    };
    let folder = value("installdir").filter(|d| !d.is_empty() && !d.contains(['\\', '/', ':']) && d != "." && d != "..")?;
    let number = |key: &str| value(key).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let played = number("LastPlayed");
    Some(SteamGame {
        name: value("name").unwrap_or_default(),
        dir: library.join("steamapps").join("common").join(folder),
        size: number("SizeOnDisk"),
        last: if played > 0 { played } else { number("LastUpdated") },
        source: if played > 0 { LastUsedSource::Played } else { LastUsedSource::Updated },
    })
}

/// Where Steam is installed: Windows keeps that in the registry; on a Mac it's in the Library.
fn steam_root() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use crate::health::reg::{string_values, Hive};
        string_values(Hive::CurrentUser, r"Software\Valve\Steam")
            .into_iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("SteamPath"))
            .map(|(_, v)| PathBuf::from(v.trim().replace('/', "\\")))
    }
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().map(|h| h.join("Library").join("Application Support").join("Steam"))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

/// Every game Steam has installed, by its number: read from Steam's library list and the record
/// it keeps for each game. Only libraries on drives built into this PC are looked in, and
/// nothing that is a link is opened: Steam's folders can be written by other accounts.
fn steam_games() -> std::collections::HashMap<String, SteamGame> {
    let Some(steam) = steam_root() else { return std::collections::HashMap::new() };
    #[cfg(windows)]
    let drives: Vec<String> = crate::util::fixed_drives().iter().map(|d| d.to_ascii_lowercase()).collect();
    #[cfg(windows)]
    let on_this_pc = |p: &Path| {
        let s = p.to_string_lossy().to_ascii_lowercase();
        drives.iter().any(|d| s.starts_with(d.as_str()))
    };
    // On a Mac a disk you plugged in is under /Volumes.
    #[cfg(not(windows))]
    let on_this_pc = |p: &Path| !p.starts_with("/Volumes");
    steam_games_in(&steam, on_this_pc)
}

/// When each game was last played on this Steam account, by its number, from the file Steam
/// keeps for each person who signed in here (userdata/<person>/config/localconfig.vdf):
///
/// ```text
/// "apps" { "440" { "LastPlayed" "1790809334" ... } ... }
/// ```
///
/// Steam on a Mac leaves a game's own record saying it was never played, so this is where its
/// last play is written down. It's the account's, so a game played on another computer counts.
#[cfg(any(target_os = "macos", test))]
fn steam_plays(localconfig: &str) -> std::collections::HashMap<String, u64> {
    let mut out = std::collections::HashMap::new();
    // The blocks we're inside, outermost first, and the name waiting for its "{".
    let mut inside: Vec<&str> = Vec::new();
    let mut named = "";
    for line in localconfig.lines() {
        let line = line.trim();
        match line {
            "{" => inside.push(std::mem::take(&mut named)),
            "}" => {
                inside.pop();
            }
            _ => {
                let mut parts = line.split('"');
                let (Some(key), value) = (parts.nth(1), parts.nth(1)) else { continue };
                match value {
                    None => named = key,
                    Some(value) if key == "LastPlayed" => {
                        let [.., apps, id] = inside.as_slice() else { continue };
                        if apps.eq_ignore_ascii_case("apps") && !id.is_empty() && id.len() <= 10 && id.bytes().all(|b| b.is_ascii_digit()) {
                            let at = value.parse::<u64>().unwrap_or(0);
                            let best = out.entry(id.to_string()).or_insert(0);
                            *best = (*best).max(at);
                        }
                    }
                    Some(_) => {}
                }
            }
        }
    }
    out
}

/// The same, for everyone who signed in to Steam here: the newest play of each game.
#[cfg(target_os = "macos")]
fn steam_plays_in(steam: &Path) -> std::collections::HashMap<String, u64> {
    let mut out = std::collections::HashMap::new();
    let real = |p: &Path, dir: bool| std::fs::symlink_metadata(p).is_ok_and(|m| !m.is_symlink() && m.is_dir() == dir);
    for person in dirs_in(&steam.join("userdata")).into_iter().take(16) {
        let file = person.join("config").join("localconfig.vdf");
        if !real(&person, true) || !real(&file, false) || std::fs::metadata(&file).is_ok_and(|m| m.len() > 32 << 20) {
            continue;
        }
        for (id, at) in steam_plays(&String::from_utf8_lossy(&std::fs::read(&file).unwrap_or_default())) {
            let best = out.entry(id).or_insert(0);
            *best = (*best).max(at);
        }
    }
    out
}

fn steam_games_in(steam: &Path, on_this_pc: impl Fn(&Path) -> bool) -> std::collections::HashMap<String, SteamGame> {
    let mut out = std::collections::HashMap::new();
    let real = |p: &Path, dir: bool| std::fs::symlink_metadata(p).is_ok_and(|m| !m.is_symlink() && m.is_dir() == dir);
    let mut libraries = vec![steam.to_path_buf()];
    let list = steam.join("steamapps").join("libraryfolders.vdf");
    if real(&list, false) && std::fs::metadata(&list).is_ok_and(|m| m.len() <= 4 << 20) {
        for line in std::fs::read_to_string(&list).unwrap_or_default().lines() {
            let mut parts = line.trim().split('"');
            if parts.nth(1) == Some("path") {
                if let Some(p) = parts.nth(1) {
                    libraries.push(PathBuf::from(p.replace(r"\\", "\\")));
                }
            }
        }
    }
    for lib in libraries.iter().filter(|l| on_this_pc(l)) {
        let apps = lib.join("steamapps");
        if !real(lib, true) || !real(&apps, true) {
            continue;
        }
        for e in std::fs::read_dir(&apps).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let Some(id) = name.strip_prefix("appmanifest_").and_then(|n| n.strip_suffix(".acf")) else { continue };
            if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) || !real(&e.path(), false) {
                continue;
            }
            if e.metadata().is_ok_and(|m| m.len() > 1 << 20) {
                continue; // these are a few hundred bytes
            }
            if let Some(g) = std::fs::read_to_string(e.path()).ok().and_then(|t| steam_game(lib, &t)) {
                out.entry(id.to_string()).or_insert(g);
            }
        }
    }
    out
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
                if dword_value(hive, &path, "SystemComponent") == Some(1) {
                    continue;
                }
                if out.iter().any(|a| a.name == name) {
                    continue;
                }
                // A Steam game's entry says nothing about where it is: Steam's own record does
                // (see `unused`). Anything else: where it says, or where its own files are.
                let steam = steam_id(&sub, &get("UninstallString"));
                let dir = if steam.is_some() { None } else { own_folder(&get("InstallLocation"), &get("DisplayIcon"), &get("UninstallString")) };
                out.push(App {
                    key: format!(r"{hive_name}\{path}"),
                    publisher: get("Publisher"),
                    version: get("DisplayVersion"),
                    size: dword_value(hive, &path, "EstimatedSize").map(|kb| kb as u64 * 1024).unwrap_or(0),
                    last_used: None,
                    last_used_source: None,
                    dir: None,
                    from: steam.as_ref().map(|_| "Steam".to_string()),
                    install_dir: dir,
                    steam,
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

/// Observable activity near the top of an app's data folder. It is supporting evidence,
/// not a complete record of app use. Failed reads must not turn into ancient activity.
fn touched(dir: &Path) -> Option<u64> {
    fn recorded(path: &Path) -> Option<u64> {
        let meta = std::fs::symlink_metadata(path).ok()?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return None;
            }
        }
        if meta.file_type().is_symlink() {
            return None;
        }
        crate::trash::cleanup_recency(&meta)
    }
    let mut newest = recorded(dir)?;
    for (i, entry) in std::fs::read_dir(dir).ok()?.enumerate() {
        if i >= 4096 {
            return None;
        }
        let at = recorded(&entry.ok()?.path())?;
        newest = newest.max(at);
    }
    Some(newest)
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
    // Folder names that say nothing about which program is in them.
    const ANYBODYS: &[&str] = &[
        "games", "game", "apps", "application", "applications", "program", "programs", "client", "launcher", "tools", "software",
        "common", "data", "files", "install", "setup", "update", "updater", "current", "release", "win64", "win32", "x64",
        "x86", "binaries", "shipping",
    ];
    for app in apps.iter_mut() {
        if app.steam.is_some() {
            continue; // Steam's own record says when it was last played (set where the list is made)
        }
        let me = squash(unversioned(&app.name));
        let dir_name = app.install_dir.as_deref().map(|d| squash(&name_of(d))).filter(|n| n.len() >= 5 && !ANYBODYS.contains(&n.as_str())).unwrap_or_default();
        let dir_lower = app.install_dir.as_deref().map(|d| d.to_string_lossy().to_ascii_lowercase());
        let mut best: Option<u64> = None;
        let mut saw = |t: u64| {
            if t > 0 {
                best = Some(best.map_or(t, |b: u64| b.max(t)));
            }
        };
        let mut unavailable = false;
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
                match touched(d) {
                    Some(t) => saw(t),
                    None => unavailable = true,
                }
            }
        }
        app.last_used = if unavailable { None } else { best };
        app.last_used_source = app.last_used.map(|_| LastUsedSource::Activity);
    }
}

/// Programs whose newest available activity evidence is at least a year old, biggest first.
/// Runtimes, drivers, command line tools and programs without evidence are left out.
static CACHE: std::sync::Mutex<Option<(u64, Vec<App>)>> = std::sync::Mutex::new(None);

/// A WinGet "portable" package: a command line tool, reached through a link folder that's on
/// PATH while its own folder isn't. (`dir` in lower case.)
fn winget_tool(dir: &str) -> bool {
    dir.contains(r"\microsoft\winget\packages\")
}

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
    #[cfg(target_os = "macos")]
    if paths.is_mac() {
        let out = mac_unused(paths, table, now);
        if let Ok(mut c) = CACHE.lock() {
            *c = Some((now, out.clone()));
        }
        return out;
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
            !winget_tool(&d) && !on_path.iter().any(|p| (!d.is_empty() && p.starts_with(&d)) || named(p))
        })
        .collect();
    // Steam games: where, how big and when last played are Steam's to say. One Steam no longer
    // has installed is a stale entry, and is left out.
    if apps.iter().any(|a| a.steam.is_some()) {
        let games = steam_games();
        apps.retain_mut(|a| {
            let Some(id) = &a.steam else { return true };
            let Some(g) = games.get(id) else { return false };
            let running = table.procs.values().any(|p| p.exe.as_deref().is_some_and(|e| Path::new(e).starts_with(&g.dir)));
            a.install_dir = Some(g.dir.clone());
            a.size = g.size;
            a.last_used = Some(if running { now } else { g.last }).filter(|t| *t > 0);
            a.last_used_source = a.last_used.map(|_| if running { LastUsedSource::Activity } else { g.source });
            true
        });
    }
    last_used(&mut apps, paths, table, now);
    let mut out: Vec<App> = apps.into_iter().filter(|a| a.last_used.is_some_and(|t| now.saturating_sub(t) >= 365 * 86_400)).collect();
    for a in &mut out {
        a.dir = a.install_dir.as_deref().filter(|d| d.is_dir()).map(|d| d.to_string_lossy().to_string());
    }
    // One that doesn't say how big it is gets measured, if it has a folder of its own.
    let unsized_: Vec<PathBuf> = out.iter().filter(|a| a.size == 0).filter_map(|a| a.dir.as_ref().map(PathBuf::from)).collect();
    let measured = crate::trash::folder_bytes(&unsized_);
    for a in out.iter_mut().filter(|a| a.size == 0) {
        if let Some(i) = a.dir.as_ref().and_then(|d| unsized_.iter().position(|u| u == Path::new(d))) {
            a.size = measured[i];
        }
    }
    out.sort_by(|a, b| b.size.cmp(&a.size).then(a.last_used.cmp(&b.last_used)));
    out.truncate(400); // a Steam library can hold hundreds of games nobody has played in a year
    if let Ok(mut c) = CACHE.lock() {
        *c = Some((now, out.clone()));
    }
    out
}

/// The apps in a Mac's Applications folders (everyone's and yours), and in the folders one
/// level inside them. Not the stand-ins a browser or Steam leaves there for a site or a game,
/// and not what's really part of macOS (Safari only looks as if it's in Applications).
#[cfg(target_os = "macos")]
fn mac_apps(home: &Path) -> Vec<PathBuf> {
    let is_app = |p: &Path| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("app"));
    let real_dir = |p: &Path| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink());
    let mut out = Vec::new();
    for base in [PathBuf::from("/Applications"), home.join("Applications")] {
        let mut folders = vec![base.clone()];
        for e in std::fs::read_dir(&base).into_iter().flatten().flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            // "Chrome Apps.localized" holds a shortcut for each site you installed, not apps.
            if real_dir(&p) && !is_app(&p) && !name.starts_with('.') && !name.contains(" Apps.") {
                folders.push(p);
            }
        }
        for folder in folders {
            for e in std::fs::read_dir(&folder).into_iter().flatten().flatten().take(2_000) {
                let p = e.path();
                if !is_app(&p) || !real_dir(&p) {
                    continue;
                }
                // Steam's shortcut to a game: a script that asks Steam to start it.
                if p.join("Contents").join("MacOS").join("run.sh").is_file() {
                    continue;
                }
                let really = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
                if really.starts_with("/System") || !p.join("Contents").join("Info.plist").is_file() {
                    continue;
                }
                out.push(p);
            }
        }
    }
    out
}

/// A Mac's apps and Steam games with no sign of use for a year. An app that's open counts as
/// used now. Each is dated by the newest sign there is: when macOS last saw it opened, or when
/// its own settings and data last changed. One that left no trace at all goes by when it was
/// installed or last updated, and one whose traces macOS won't show isn't listed.
#[cfg(target_os = "macos")]
fn mac_unused(paths: &Paths, table: &ProcTable, now: u64) -> Vec<App> {
    const YEAR: u64 = 365 * 86_400;
    let running = crate::programs::image_paths(table);
    // An app used from a terminal leaves none of these traces: one with a command on PATH
    // (in a folder of its own, or a link from /usr/local/bin) stays off the list.
    let mut commands: Vec<PathBuf> = Vec::new();
    for dir in std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default().into_iter().take(64) {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten().take(4_000) {
            if let Ok(to) = std::fs::read_link(e.path()) {
                commands.push(if to.is_absolute() { to } else { dir.join(to) });
            }
        }
        commands.push(dir);
    }
    let candidates: Vec<PathBuf> = mac_apps(&paths.home)
        .into_iter()
        .filter(|app| {
            let name = app.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            // An app in a folder of its own inside Applications ("Python 3.11") is a part of that.
            let folder = app.parent().filter(|f| f.file_name().is_some_and(|n| n != "Applications")).map(name_of).unwrap_or_default();
            let inside = format!("{}/", app.to_string_lossy());
            !name.is_empty()
                && !hands_off(&name)
                && !is_plumbing(&name, "")
                && !is_plumbing(&folder, "")
                && !name.eq_ignore_ascii_case("momputer")
                && !running.values().any(|exe| exe.starts_with(&inside))
                && !commands.iter().any(|c| c.starts_with(app))
        })
        .collect();
    // Asking macOS about an app takes a moment, so several are asked about at once.
    let dated: Vec<Option<(u64, LastUsedSource)>> = std::thread::scope(|s| {
        let home = &paths.home;
        let workers: Vec<_> = candidates
            .chunks(candidates.len().div_ceil(8).max(1))
            .map(|some| s.spawn(move || some.iter().map(|app| crate::macos::app_last_used(home, app)).collect::<Vec<_>>()))
            .collect();
        workers.into_iter().flat_map(|w| w.join().unwrap_or_default()).collect()
    });
    let mut out: Vec<App> = Vec::new();
    for (app, dated) in candidates.into_iter().zip(dated) {
        let Some((last, source)) = dated.filter(|(t, _)| now.saturating_sub(*t) >= YEAR) else { continue };
        out.push(App {
            key: app.to_string_lossy().to_string(),
            name: app.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            publisher: String::new(),
            version: String::new(),
            size: 0,
            last_used: Some(last),
            last_used_source: Some(source),
            dir: Some(app.to_string_lossy().to_string()),
            from: None,
            install_dir: Some(app),
            steam: None,
        });
    }
    // Steam's games: where, how big and when last played are Steam's to say. On a Mac the last
    // play is in the account's own file; a game put on this Mac within the year isn't unused.
    let plays = steam_root().map(|steam| steam_plays_in(&steam)).unwrap_or_default();
    for (id, mut g) in steam_games() {
        let Some(folder) = std::fs::symlink_metadata(&g.dir).ok().filter(|m| m.is_dir() && !m.file_type().is_symlink()) else { continue };
        let put_here = folder.created().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        let played = plays.get(&id).copied().unwrap_or(0).max(if g.source == LastUsedSource::Played { g.last } else { 0 });
        if played > 0 {
            (g.last, g.source) = (played, LastUsedSource::Played);
        }
        let open = running.values().any(|exe| Path::new(exe).starts_with(&g.dir));
        if open || g.last == 0 || now.saturating_sub(g.last.max(put_here)) < YEAR {
            continue;
        }
        out.push(App {
            key: format!("steam://uninstall/{id}"),
            name: if g.name.is_empty() { name_of(&g.dir) } else { g.name },
            publisher: String::new(),
            version: String::new(),
            size: g.size,
            last_used: Some(g.last),
            last_used_source: Some(g.source),
            dir: Some(g.dir.to_string_lossy().to_string()),
            from: Some("Steam".into()),
            install_dir: Some(g.dir),
            steam: Some(id),
        });
    }
    let unsized_: Vec<PathBuf> = out.iter().filter(|a| a.size == 0).filter_map(|a| a.install_dir.clone()).collect();
    let measured = crate::trash::folder_bytes(&unsized_);
    for a in out.iter_mut().filter(|a| a.size == 0) {
        if let Some(i) = a.install_dir.as_ref().and_then(|d| unsized_.iter().position(|u| u == d)) {
            a.size = measured[i];
        }
    }
    out.sort_by(|a, b| b.size.cmp(&a.size).then(a.last_used.cmp(&b.last_used)));
    out.truncate(400);
    out
}

/// On a Mac, uninstalling an app is moving it to the Trash: `key` is the app, which must be a
/// real folder in Applications (yours or everyone's, or a folder inside), and not open. A Steam
/// game is Steam's to remove: `key` is its uninstall address, and Steam asks before it does.
#[cfg(target_os = "macos")]
fn mac_uninstall(key: &str) -> Result<String, String> {
    if let Some(id) = key.strip_prefix("steam://uninstall/") {
        if id.is_empty() || id.len() > 10 || !id.bytes().all(|b| b.is_ascii_digit()) {
            return Err("That isn't a Steam game.".into());
        }
        crate::util::run("/usr/bin/open", &[key], std::time::Duration::from_secs(10)).map_err(|_| "Steam didn't open.".to_string())?;
        return Ok("Steam is open. It asks before it removes the game.".into());
    }
    let app = Path::new(key);
    let home = dirs::home_dir().ok_or("That isn't an app in Applications.")?;
    if !app.is_absolute() || app.components().any(|c| matches!(c, std::path::Component::ParentDir)) || !mac_apps(&home).iter().any(|a| a == app) {
        return Err("That isn't an app in Applications.".into());
    }
    let name = app.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if hands_off(&name) {
        return Err(format!("Mom doesn't remove {name}: what's in it can't be got back."));
    }
    let table = crate::procs::Sampler::new().sample();
    let inside = format!("{key}/");
    if crate::programs::image_paths(&table).values().any(|exe| exe.starts_with(&inside)) {
        return Err(format!("{name} is open. Quit it first."));
    }
    // An app from the App Store, or one an installer put there, belongs to the whole Mac: mom
    // can't move it, and Finder can, once you've given it your password or your fingerprint.
    let yours = {
        use std::os::unix::fs::MetadataExt;
        std::fs::symlink_metadata(app).is_ok_and(|m| m.uid() == unsafe { libc::geteuid() })
    };
    if !yours || crate::util::to_bin(app).is_err() {
        crate::macos::finder_trash(app, &name)?;
    }
    Ok(format!("{name} is in the Trash. Empty the Trash to get its space back, or put it back from there."))
}

/// Starts a program's own uninstaller (Windows asks for permission if it needs it). The
/// uninstaller comes from the registry entry `key` names, which must be in the uninstall list.
pub fn uninstall(key: &str) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        mac_uninstall(key)
    }
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
    #[cfg(not(any(windows, target_os = "macos")))]
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
    fn a_winget_command_line_tool_is_never_unused() {
        assert!(winget_tool(r"c:\users\sam\appdata\local\microsoft\winget\packages\ninja-build.ninja_microsoft.winget.source_8wekyb3d8bbwe"));
        assert!(!winget_tool(r"c:\program files\nimbus notes"));
        assert!(!winget_tool(""));
    }

    #[test]
    fn unavailable_app_activity_is_unknown_instead_of_the_epoch() {
        let root = crate::util::temp_dir().join(format!("mom-app-activity-{}", std::process::id()));
        assert!(!root.exists());
        assert_eq!(touched(&root), None);
        std::fs::create_dir_all(&root).unwrap();
        assert!(touched(&root).is_some_and(|t| t > 0));
        let file = root.join("settings.json");
        std::fs::write(&file, "{}").unwrap();
        assert_eq!(touched(&file), None, "a folder that became a file cannot prove old activity");
        std::fs::remove_dir_all(&root).unwrap();
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

    // About Windows' own folders, read by Windows' own rules for paths.
    #[cfg(windows)]
    #[test]
    fn a_programs_folder_is_its_own_never_one_it_shares() {
        let f = |loc: &str, icon: &str, un: &str| own_folder(loc, icon, un).map(|p| p.to_string_lossy().to_string());
        // Where it says; else where its own program file is; else where its own uninstaller is.
        assert_eq!(f(r#""D:\Apps\Atlas Recorder""#, "", "").as_deref(), Some(r"D:\Apps\Atlas Recorder"));
        assert_eq!(f("", r"C:\Program Files (x86)\Nimbus\Nimbus.exe,0", "").as_deref(), Some(r"C:\Program Files (x86)\Nimbus"));
        assert_eq!(f("", "", r"C:\Program Files (x86)\Nimbus\Uninstall.exe").as_deref(), Some(r"C:\Program Files (x86)\Nimbus"));
        // An icon that is only a picture sits in a cache every program shares, as does a launcher's or Windows' uninstaller.
        assert_eq!(f("", r"C:\Program Files (x86)\Steam\steam\games\71f34eaa.ico", r#""C:\Program Files (x86)\Steam\steam.exe" steam://uninstall/2996080"#), None);
        assert_eq!(f("", r"C:\Windows\Installer\{1F2E}\icon.ico", "MsiExec.exe /I{1F2E}"), None);
        assert_eq!(f("", r"C:\Program Files (x86)\Steam\steam\games\app.exe", ""), None, "even an .exe, in a folder they all share");
        assert_eq!(f(r"C:\", "", ""), None, "a whole drive is nobody's own folder");
    }

    #[test]
    fn steam_games_are_read_from_steams_own_record() {
        assert_eq!(steam_id("Steam App 2996080", r#""C:\Program Files (x86)\Steam\steam.exe" steam://uninstall/2996080"#).as_deref(), Some("2996080"));
        assert_eq!(steam_id("Steam App 440", "").as_deref(), Some("440"));
        assert_eq!(steam_id("Retro Racer", r"C:\Games\Retro Racer\unins000.exe"), None);
        assert_eq!(steam_id("Steam App x", "steam://uninstall/"), None);
        let record = "\"AppState\"\n{\n\t\"appid\"\t\t\"2996080\"\n\t\"name\"\t\t\"Retro Racer\"\n\t\"installdir\"\t\t\"Retro Racer\"\n\t\"LastUpdated\"\t\t\"1791823126\"\n\t\"LastPlayed\"\t\t\"1790809334\"\n\t\"SizeOnDisk\"\t\t\"534902844\"\n}\n";
        let lib = Path::new(r"E:\SteamLibrary");
        assert_eq!(
            steam_game(lib, record),
            Some(SteamGame { name: "Retro Racer".into(), dir: lib.join("steamapps").join("common").join("Retro Racer"), size: 534_902_844, last: 1_790_809_334, source: LastUsedSource::Played })
        );
        // Never played: when it was installed or last updated, so a game from last week isn't "unused".
        let never = record.replace("1790809334", "0");
        assert_eq!(steam_game(lib, &never).unwrap().last, 1_791_823_126);
        assert_eq!(steam_game(lib, &never).unwrap().source, LastUsedSource::Updated);
        assert_eq!(serde_json::to_value(LastUsedSource::Updated).unwrap(), "updated");
        // A record that names a path instead of a folder is not followed anywhere.
        for odd in [r"..\\..\\Windows", "C:/Windows", ".."] {
            assert_eq!(steam_game(lib, &record.replace("\"installdir\"\t\t\"Retro Racer\"", &format!("\"installdir\"\t\t\"{odd}\""))), None, "{odd}");
        }
        assert_eq!(steam_game(lib, "not a record"), None);
    }

    #[test]
    fn a_games_last_play_is_read_from_the_accounts_own_file() {
        let file = "\"UserLocalConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"apps\"\n\t\t\t\t{\n\
            \t\t\t\t\t\"900001\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LastPlayed\"\t\t\"1600000000\"\n\t\t\t\t\t\t\"Playtime\"\t\t\"310\"\n\t\t\t\t\t}\n\
            \t\t\t\t\t\"900002\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"cloud\"\n\t\t\t\t\t\t{\n\t\t\t\t\t\t\t\"last_sync_state\"\t\t\"synchronized\"\n\t\t\t\t\t\t}\n\t\t\t\t\t\t\"LastPlayed\"\t\t\"1780000000\"\n\t\t\t\t\t}\n\
            \t\t\t\t\t\"440\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"Playtime\"\t\t\"0\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n\
            \t\"friends\"\n\t{\n\t\t\"7\"\n\t\t{\n\t\t\t\"LastPlayed\"\t\t\"1799999999\"\n\t\t}\n\t}\n}\n";
        let plays = steam_plays(file);
        assert_eq!(plays.get("900001"), Some(&1_600_000_000));
        assert_eq!(plays.get("900002"), Some(&1_780_000_000), "a block inside the game's own doesn't hide what follows it");
        assert_eq!(plays.get("440"), None, "never played");
        assert_eq!(plays.get("7"), None, "a number that isn't under apps is nobody's game");
        assert!(steam_plays("not a record").is_empty());
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
