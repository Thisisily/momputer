//! Is a program still on this PC? Asked for the programs graphics drivers name in their shader
//! caches (lanternfall-win64-shipping.exe, or a Store app's package name), so a cache whose
//! game was uninstalled can go, under the game's real name, whatever its age.
//!
//! What counts: Steam's own records (which games the account has, which are installed, and the
//! programs each one launches), Epic's install manifests, and a bounded look for the program
//! file in the places programs and games live: Program Files, the folders installed programs
//! report, Windows' own folders, and top-level game folders on every drive ("Games", "itch",
//! "Epic Games"...). Nothing is read inside the programs; only names and folders.
//!
//! "Gone" is said only on a record's word: Steam's that the game isn't installed, or Windows'
//! that a Store app's folder is gone. A program file the look didn't find is only "not found":
//! portable programs live anywhere. Then the cache is judged by its age, as before.

use super::steam_appinfo;
use super::steam_shaders;
use crate::paths::Paths;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum State {
    Installed,
    /// Uninstalled, or not on this PC any more.
    Gone,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Found {
    /// The program's proper name, when something says it ("Lanternfall: Night Shift").
    pub title: Option<String>,
    pub state: State,
    /// One short line for the row.
    pub detail: &'static str,
}

/// Top-level folders on a drive that hold games, by lowercase name.
const GAME_ROOTS: &[&str] = &[
    "games", "game", "itch", "itch games", "itch.io", "epic games", "gog games", "gog galaxy", "xboxgames", "riot games", "ubisoft",
    "ubisoft games", "ea games", "origin games", "battle.net",
];
const MAX_ENTRIES: usize = 1_200_000;
const MAX_TIME: Duration = Duration::from_secs(10);
/// Folders that never hold the programs a cache names, only lots of files: a game's assets,
/// Steam's own libraries (their games are looked up through Steam), Windows' component store.
const SKIP: &[&str] = &[
    "steamapps", "windowsapps", "winsxs", "installer", "assembly", "servicing", "node_modules", ".git", "content", "paks",
    "streamingassets", "locales", "__pycache__", "shadercache",
];
const APPINFO_BYTES: u64 = 96 << 20;
const EPIC_MANIFEST_BYTES: u64 = 1 << 20;
const MAX_EPIC_MANIFESTS: usize = 2_000;

#[derive(Default)]
pub(super) struct Programs {
    /// Program file name (lowercase) -> the proper name of what it belongs to, if known.
    files: HashMap<String, Option<String>>,
    /// Program file name (lowercase) -> the Steam games that launch it: (app id, name).
    steam: HashMap<String, Vec<(u32, String)>>,
    /// A Steam game's folder name (lowercase) -> (app id, name), for programs Steam's records
    /// don't list by name (an Unreal game's -Win64-Shipping.exe beside its launcher).
    steam_folders: HashMap<String, Vec<(u32, String)>>,
    steam_installed: HashSet<u32>,
    /// Every Steam game's name, by app id.
    steam_names: HashMap<u32, String>,
    /// Steam's records were read whole: a game it lists but hasn't installed is uninstalled.
    steam_known: bool,
    /// Where Store apps keep their data, if it could be listed.
    packages: Option<HashSet<String>>,
    /// The whole look finished: a program it didn't find isn't in any program folder.
    complete: bool,
}

impl Programs {
    pub(super) fn find(paths: &Paths) -> Programs {
        let mut me = Programs { complete: true, ..Default::default() };
        let fixture = paths.system_drive.parent().is_some();
        // Where programs live, the likeliest first: if time runs out, it's in the game folders.
        let mut roots: Vec<(PathBuf, usize, Title)> = vec![(paths.windows.clone(), 1, Title::None)];
        // Each installed game's own folder, looked in last: the most files for the fewest programs.
        let mut game_dirs: Vec<(PathBuf, usize, Title)> = Vec::new();
        for sub in ["System32", "SysWOW64"] {
            roots.push((paths.windows.join(sub), 1, Title::None));
        }
        roots.push((paths.windows.join("SystemApps"), 2, Title::None));
        roots.push((paths.local_appdata.join("Programs"), 4, Title::None));
        roots.push((paths.system_drive.join("Program Files"), 5, Title::None));
        roots.push((paths.program_files_x86.clone(), 5, Title::None));
        roots.push((paths.appdata.join("itch").join("apps"), 5, Title::FolderUnder));
        for drive in std::iter::once(&paths.system_drive).chain(paths.other_drives.iter()) {
            let Ok(entries) = std::fs::read_dir(drive) else { continue };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                if GAME_ROOTS.contains(&name.as_str()) && entry.file_type().is_ok_and(|t| t.is_dir()) {
                    roots.push((entry.path(), 5, Title::FolderUnder));
                }
            }
        }

        // ---- Steam: what's installed, and what the account has.
        if let Some(games) = steam_shaders::games(paths) {
            for (id, (name, dir)) in &games.installed {
                me.steam_installed.insert(*id);
                game_dirs.push((dir.clone(), 4, Title::Given(name.clone())));
            }
            match read_capped(&games.root.join("appcache").join("appinfo.vdf"), APPINFO_BYTES).and_then(|b| steam_appinfo::parse(&b)) {
                Some(apps) => {
                    me.steam_known = games.complete;
                    for (id, app) in apps {
                        me.steam_names.insert(id, app.name.clone());
                        for program in &app.programs {
                            me.steam.entry(program.clone()).or_default().push((id, app.name.clone()));
                        }
                        if !app.folder.is_empty() {
                            me.steam_folders.entry(app.folder.to_ascii_lowercase()).or_default().push((id, app.name.clone()));
                        }
                    }
                }
                None => me.complete = false,
            }
        }

        // ---- Epic: its manifests say where each game went, and its name.
        let manifests = paths.program_data.join("Epic").join("EpicGamesLauncher").join("Data").join("Manifests");
        if let Ok(entries) = std::fs::read_dir(&manifests) {
            for entry in entries.flatten().take(MAX_EPIC_MANIFESTS) {
                let path = entry.path();
                if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("item")) {
                    continue;
                }
                let Some(text) = read_capped(&path, EPIC_MANIFEST_BYTES).and_then(|b| String::from_utf8(b).ok()) else { continue };
                let Ok(item) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
                let (Some(dir), Some(name)) = (item["InstallLocation"].as_str(), item["DisplayName"].as_str()) else { continue };
                game_dirs.push((PathBuf::from(dir), 5, Title::Given(name.chars().filter(|c| !c.is_control()).take(200).collect())));
            }
        }

        // Only this PC's own drives: a folder from a manifest or the registry could name a network
        // share, and just listing it would make Windows sign in to that server as you.
        let drives: Vec<PathBuf> = match paths.system_drive.parent() {
            Some(fixture_root) => vec![fixture_root.to_path_buf()],
            None => std::iter::once(paths.system_drive.clone()).chain(paths.other_drives.iter().cloned()).collect(),
        };
        let start = Instant::now();
        let mut left = MAX_ENTRIES;
        let mut walked = HashSet::new();
        let walk_all = |me: &mut Programs, roots: Vec<(PathBuf, usize, Title)>, walked: &mut HashSet<String>, left: &mut usize| {
            for (root, depth, title) in roots {
                if !on_these_drives(&root, &drives) {
                    me.complete = false;
                    continue;
                }
                if !walked.insert(crate::util::norm_path(&root.to_string_lossy())) {
                    continue;
                }
                if !me.walk(&root, depth, &title, left, start) {
                    me.complete = false;
                }
            }
        };
        walk_all(&mut me, roots, &mut walked, &mut left);
        // The folders installed programs report: a help, never a reason to doubt the rest. Some
        // name a whole drive or a folder of yours, which isn't where a program lives.
        if !fixture {
            let wide = [&paths.home, &paths.local_appdata, &paths.appdata, &paths.program_files_x86, &paths.windows];
            for dir in crate::installed::Installed::scan(paths).dirs {
                let key = crate::util::norm_path(&dir.to_string_lossy());
                let too_wide = dir.parent().is_none()
                    || wide.iter().any(|w| within(w, &dir))
                    || within(&paths.system_drive.join("Program Files"), &dir);
                if too_wide || !on_these_drives(&dir, &drives) || !walked.insert(key) {
                    continue;
                }
                let finished = me.walk(&dir, 4, &Title::None, &mut left, start);
                if !finished && (left == 0 || start.elapsed() > MAX_TIME) {
                    me.complete = false;
                }
            }
        }
        walk_all(&mut me, game_dirs, &mut walked, &mut left);

        // ---- Store apps keep a data folder for as long as they're installed.
        me.packages = std::fs::read_dir(paths.local_appdata.join("Packages"))
            .ok()
            .map(|entries| entries.flatten().map(|e| e.file_name().to_string_lossy().to_ascii_lowercase()).collect());
        me
    }

    /// Notes every program file under `root`, down to `depth` folders, shallowest first, without
    /// following links. False if it couldn't finish (out of time or entries, or the root couldn't
    /// be read). A root that isn't there is fine: nothing is installed there. A folder inside it
    /// that you may not open holds nothing you run, so it's passed over.
    fn walk(&mut self, root: &Path, depth: usize, title: &Title, left: &mut usize, start: Instant) -> bool {
        match std::fs::symlink_metadata(root) {
            Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
            Ok(_) => return true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) => return false,
        }
        let mut whole = true;
        let mut queue = std::collections::VecDeque::from([(root.to_path_buf(), 0usize, None::<String>)]);
        while let Some((dir, level, under)) = queue.pop_front() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                whole &= level > 0;
                continue;
            };
            for entry in entries {
                if *left == 0 || start.elapsed() > MAX_TIME {
                    return false;
                }
                *left -= 1;
                let Ok(entry) = entry else {
                    whole = false;
                    continue;
                };
                let Ok(kind) = entry.file_type() else { continue };
                if kind.is_symlink() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if kind.is_dir() {
                    if level + 1 < depth && !SKIP.contains(&name.to_ascii_lowercase().as_str()) {
                        let under = under.clone().or_else(|| matches!(title, Title::FolderUnder).then(|| name.clone()));
                        queue.push_back((entry.path(), level + 1, under));
                    }
                } else if name.len() > 4 && name.to_ascii_lowercase().ends_with(".exe") {
                    let named = match title {
                        Title::Given(t) => Some(t.clone()),
                        Title::FolderUnder => under.clone(),
                        Title::None => None,
                    };
                    let slot = self.files.entry(name.to_ascii_lowercase()).or_default();
                    if slot.is_none() {
                        *slot = named;
                    }
                }
            }
        }
        whole
    }

    /// What's known about the program a cache names: an .exe file name, or a Store app's
    /// package name ("example.notes_0123456789abc").
    pub(super) fn judge(&self, program: &str) -> Found {
        let key = program.trim().to_ascii_lowercase();
        if !key.ends_with(".exe") {
            return match &self.packages {
                Some(here) if here.contains(&key) => Found { title: None, state: State::Installed, detail: "Installed" },
                Some(_) => Found { title: None, state: State::Gone, detail: "Not on this PC any more" },
                None => Found { title: None, state: State::Unknown, detail: "Installation unknown" },
            };
        }
        let steam = self.steam_apps(&key);
        let installed_steam = steam.iter().find(|(id, _)| self.steam_installed.contains(id));
        let steam_title = installed_steam.or(steam.first()).map(|(_, name)| name.clone());
        if let Some(title) = self.files.get(&key) {
            return Found { title: steam_title.or_else(|| title.clone()), state: State::Installed, detail: "Installed" };
        }
        if installed_steam.is_some() {
            return Found { title: steam_title, state: State::Installed, detail: "Installed in Steam" };
        }
        if !steam.is_empty() && self.steam_known {
            return Found { title: steam_title, state: State::Gone, detail: "Uninstalled from Steam" };
        }
        let detail = if self.complete { "Not found in program folders" } else { "Installation unknown" };
        Found { title: steam_title, state: State::Unknown, detail }
    }

    /// A Steam game's name, from Steam's records.
    pub(super) fn steam_name(&self, id: u32) -> Option<&str> {
        self.steam_names.get(&id).map(String::as_str)
    }

    /// The Steam games behind a program: ones that launch it by name, or, for an Unreal
    /// game's "<name>-win64-shipping.exe", ones that launch "<name>.exe" or live in "<name>".
    fn steam_apps(&self, program: &str) -> Vec<(u32, String)> {
        let mut out: Vec<(u32, String)> = self.steam.get(program).cloned().unwrap_or_default();
        if let Some(stem) = program.strip_suffix("-win64-shipping.exe").or_else(|| program.strip_suffix(".exe")) {
            if let Some(more) = self.steam.get(&format!("{stem}.exe")) {
                out.extend(more.iter().cloned());
            }
            if let Some(more) = self.steam_folders.get(stem) {
                out.extend(more.iter().cloned());
            }
        }
        let mut seen = HashSet::new();
        out.retain(|(id, _)| seen.insert(*id));
        out
    }
}

enum Title {
    None,
    Given(String),
    /// The name of the folder just under the root ("E:\Games\<this>\...").
    FolderUnder,
}

/// A name to show for a program: a Store app's package name says less than its last part
/// ("openai.codex_2p2nqsd0c76g0" is Codex); a program file's name stays as it is.
pub(super) fn display(program: &str) -> String {
    if program.to_ascii_lowercase().ends_with(".exe") {
        return program.to_string();
    }
    let app = program.rsplit_once('_').map_or(program, |(app, _)| app);
    let last = app.rsplit('.').next().unwrap_or(app);
    let mut chars = last.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => program.to_string(),
    }
}

/// `path` is an ordinary local path (a drive letter, no "..") on one of `drives`.
fn on_these_drives(path: &Path, drives: &[PathBuf]) -> bool {
    use std::path::Component;
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return false;
    }
    #[cfg(windows)]
    if !matches!(path.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_))) {
        return false;
    }
    drives.iter().any(|d| within(path, d))
}

/// `path` is `root` or inside it, whatever the letter case or slashes.
fn within(path: &Path, root: &Path) -> bool {
    let key = |p: &Path| {
        let text = p.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
        text.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(text).trim_end_matches('\\').to_string()
    };
    let (path, root) = (key(path), key(root));
    path == root || path.starts_with(&format!("{root}\\"))
}

/// A whole small file, if it's an ordinary file no bigger than `cap`.
fn read_capped(path: &Path, cap: u64) -> Option<Vec<u8>> {
    let m = std::fs::symlink_metadata(path).ok()?;
    if !m.is_file() || m.len() > cap {
        return None;
    }
    std::fs::read(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
    }

    #[test]
    fn store_apps_show_their_last_name_part() {
        assert_eq!(display("example.notes_0123456789abc"), "Notes");
        assert_eq!(display("nimbus_0123456789abc"), "Nimbus");
        assert_eq!(display("Lanternfall.exe"), "Lanternfall.exe");
    }

    #[cfg(windows)]
    #[test]
    fn only_folders_on_this_pcs_drives_are_looked_in() {
        let drives = [PathBuf::from(r"C:\"), PathBuf::from(r"E:\")];
        assert!(on_these_drives(Path::new(r"C:\Program Files\Lanternfall"), &drives));
        assert!(on_these_drives(Path::new(r"e:\Games"), &drives));
        assert!(!on_these_drives(Path::new(r"\\server.example\share\Games"), &drives));
        assert!(!on_these_drives(Path::new(r"Z:\Games"), &drives), "a mapped drive isn't one of this PC's");
        assert!(!on_these_drives(Path::new(r"C:\Games\..\..\x"), &drives));
        assert!(!on_these_drives(Path::new(r"Games"), &drives));
    }

    #[test]
    fn games_are_found_named_and_told_apart_from_uninstalled_ones() {
        let root = std::env::temp_dir().join(format!("mom-programs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut paths = Paths::under(&root);
        let e = root.join("e");
        paths.other_drives = vec![e.clone()];
        // Steam on C:, a library on E: with one installed game.
        let steam = paths.program_files_x86.join("Steam");
        std::fs::create_dir_all(steam.join("steamapps")).unwrap();
        std::fs::create_dir_all(steam.join("appcache")).unwrap();
        let library = e.join("SteamLibrary");
        std::fs::write(
            steam.join("steamapps").join("libraryfolders.vdf"),
            format!("\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{}\" }} }}", steam.display().to_string().replace('\\', "\\\\"), library.display().to_string().replace('\\', "\\\\")),
        )
        .unwrap();
        std::fs::create_dir_all(library.join("steamapps")).unwrap();
        std::fs::write(library.join("steamapps").join("appmanifest_480.acf"), r#""AppState" { "appid" "480" "name" "Lanternfall" "installdir" "Lanternfall" }"#).unwrap();
        put(&library.join("steamapps").join("common").join("Lanternfall").join("Lanternfall").join("Binaries").join("Win64").join("Lanternfall-Win64-Shipping.exe"));
        std::fs::write(
            steam.join("appcache").join("appinfo.vdf"),
            steam_appinfo::tests::file(&[
                (480, "Lanternfall", "Lanternfall", &["Lanternfall.exe"]),
                (970, "Copperline, Night Shift", "Copperline", &[r"Copperline\Binaries\Win64\Copperline-Win64-Shipping.exe"]),
                (1210, "Nimbus Tales", "NimbusTales", &["NimbusTales.exe"]),
            ]),
        )
        .unwrap();
        // A game from elsewhere, in a games folder on E:.
        put(&e.join("Games").join("Atlas Run").join("atlas run.exe"));
        // Windows' own programs, and a Store app's data folder.
        put(&paths.windows.join("System32").join("taskmgr.exe"));
        std::fs::create_dir_all(paths.local_appdata.join("Packages").join("example.notes_0123456789abc")).unwrap();

        let p = Programs::find(&paths);
        let judge = |name: &str| p.judge(name);
        let installed = judge("lanternfall-win64-shipping.exe");
        assert_eq!((installed.title.as_deref(), installed.state), (Some("Lanternfall"), State::Installed));
        let gone = judge("copperline-win64-shipping.exe");
        assert_eq!((gone.title.as_deref(), gone.state, gone.detail), (Some("Copperline, Night Shift"), State::Gone, "Uninstalled from Steam"));
        assert_eq!(judge("NimbusTales.exe").state, State::Gone, "owned, never installed here");
        let elsewhere = judge("atlas run.exe");
        assert_eq!((elsewhere.title.as_deref(), elsewhere.state), (Some("Atlas Run"), State::Installed));
        assert_eq!(judge("taskmgr.exe").state, State::Installed);
        assert_eq!(judge("example.notes_0123456789abc").state, State::Installed);
        assert_eq!(judge("example.gone_0123456789abc").state, State::Gone);
        // Not found anywhere it looked: only "not found", since a portable program lives anywhere.
        let unknown = judge("somethingelse.exe");
        assert_eq!((unknown.title, unknown.state, unknown.detail), (None, State::Unknown, "Not found in program folders"));

        // A manifest naming a network share isn't looked in, so the look isn't whole.
        let manifests = paths.program_data.join("Epic").join("EpicGamesLauncher").join("Data").join("Manifests");
        std::fs::create_dir_all(&manifests).unwrap();
        std::fs::write(manifests.join("x.item"), r#"{"InstallLocation": "\\\\server.example\\games\\Atlas", "DisplayName": "Atlas"}"#).unwrap();
        assert_eq!(Programs::find(&paths).judge("somethingelse.exe").detail, "Installation unknown");
        std::fs::remove_dir_all(paths.program_data.join("Epic")).unwrap();

        // Without Steam's records, nothing says a game was uninstalled.
        std::fs::remove_file(steam.join("appcache").join("appinfo.vdf")).unwrap();
        let p = Programs::find(&paths);
        assert_eq!(p.judge("copperline-win64-shipping.exe").state, State::Unknown, "Steam's records unreadable: unsure");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
