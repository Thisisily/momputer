//! Steam's per-game shader folders. Ownership comes from the numeric app id, never a
//! guessed game name. An incomplete catalog can establish presence, but never absence.

use crate::paths::Paths;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

pub(super) struct Cache {
    pub path: PathBuf,
    pub id: u32,
    pub name: String,
    /// `name` is the game's, from its install record (not "Steam app 123").
    pub named: bool,
    /// Every library was read and none has the game installed.
    pub uninstalled: bool,
    pub detail: String,
}

const LIST_BYTES: u64 = 4 << 20;
const MANIFEST_BYTES: u64 = 1 << 20;
const CATALOG_BYTES: u64 = 32 << 20;
const MAX_LIBRARIES: usize = 128;
const MAX_ENTRIES: usize = 20_000;
const INSTALLED: &str = "Installed in Steam";
const MISSING: &str = "Installation folder missing; game may have moved";
const NOT_FOUND: &str = "Not installed in Steam";
const UNKNOWN: &str = "Installation status unknown";

/// The fixture layout has a folder standing in for the system drive. It must never read
/// this PC's registry or discover libraries outside the supplied layout.
pub(super) fn discover(paths: &Paths) -> Vec<Cache> {
    let Some((root, allowed)) = located(paths) else { return vec![] };
    discover_at(&root, &allowed)
}

/// Steam's own folder, when it's on a drive built into this PC.
pub(super) fn root(paths: &Paths) -> Option<PathBuf> {
    located(paths).map(|(root, _)| root)
}

/// Steam's folder, and the drives its libraries may be on.
fn located(paths: &Paths) -> Option<(PathBuf, Vec<PathBuf>)> {
    let fixture = paths.system_drive.parent().is_some();
    let root = if fixture {
        Some(paths.program_files_x86.join("Steam"))
    } else {
        steam_root()
    };
    let root = root.and_then(|p| normalized(&p))?;
    let mut allowed = vec![paths.system_drive.clone()];
    allowed.extend(paths.other_drives.iter().cloned());
    if fixture {
        // Paths::under puts Program Files beside, rather than inside, its fake C: drive.
        allowed.push(paths.program_files_x86.clone());
    }
    let allowed: Vec<PathBuf> = allowed.into_iter().filter_map(|p| normalized(&p)).collect();
    allowed.iter().any(|d| beneath(&root, d)).then_some((root, allowed))
}

/// What Steam says is installed: its own folder, and each installed game by app id with its
/// name and folder. Used to name a game's caches elsewhere and to tell whether it's still here.
pub(super) struct Games {
    pub root: PathBuf,
    /// App id -> (name, install folder). Only games whose folder is there.
    pub installed: HashMap<u32, (String, PathBuf)>,
    /// Every library and manifest was read: a game missing from `installed` isn't installed
    /// through this Steam.
    pub complete: bool,
}

pub(super) fn games(paths: &Paths) -> Option<Games> {
    let fixture = paths.system_drive.parent().is_some();
    let root = if fixture { Some(paths.program_files_x86.join("Steam")) } else { steam_root() };
    let root = root.and_then(|p| normalized(&p))?;
    let mut allowed = vec![paths.system_drive.clone()];
    allowed.extend(paths.other_drives.iter().cloned());
    if fixture {
        allowed.push(paths.program_files_x86.clone());
    }
    let allowed: Vec<PathBuf> = allowed.into_iter().filter_map(|p| normalized(&p)).collect();
    if !allowed.iter().any(|d| beneath(&root, d)) {
        return None;
    }
    let mut budget = CATALOG_BYTES;
    let mut complete = true;
    let mut libraries = vec![root.clone()];
    match read_text(&root.join("steamapps").join("libraryfolders.vdf"), LIST_BYTES, &mut budget).and_then(|s| library_paths(&s)) {
        Ok((more, read_all)) => {
            complete &= read_all;
            libraries.extend(more);
        }
        Err(()) => complete = false,
    }
    let mut seen = HashSet::new();
    libraries.retain(|library| seen.insert(path_key(library)));
    libraries.truncate(MAX_LIBRARIES);
    let mut installed = HashMap::new();
    let mut entries_left = MAX_ENTRIES;
    for library in libraries {
        if !allowed.iter().any(|drive| beneath(&library, drive)) {
            complete = false;
            continue;
        }
        let apps = library.join("steamapps");
        let Ok(entries) = directory(&apps) else {
            complete = false;
            continue;
        };
        for entry in entries {
            if entries_left == 0 {
                complete = false;
                break;
            }
            entries_left -= 1;
            let Ok(entry) = entry else { complete = false; continue };
            let filename = entry.file_name().to_string_lossy().to_ascii_lowercase();
            let Some(id) = filename.strip_prefix("appmanifest_").and_then(|n| n.strip_suffix(".acf")) else { continue };
            let Ok((name, folder)) = read_text(&entry.path(), MANIFEST_BYTES, &mut budget).and_then(|s| manifest(&s, id)) else {
                complete = false;
                continue;
            };
            let dir = apps.join("common").join(folder);
            match (id.parse::<u32>(), install_state(&dir)) {
                (Ok(n), InstallState::Present) => {
                    installed.insert(n, (name, dir));
                }
                (Ok(_), InstallState::Missing) => {}
                _ => complete = false,
            }
        }
    }
    Some(Games { root, installed, complete })
}

#[cfg(windows)]
fn steam_root() -> Option<PathBuf> {
    use crate::health::reg::{string_values, Hive};
    string_values(Hive::CurrentUser, r"Software\Valve\Steam")
        .into_iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("SteamPath"))
        .map(|(_, value)| PathBuf::from(value.trim().trim_matches('"')))
}

#[cfg(not(windows))]
fn steam_root() -> Option<PathBuf> {
    None
}

/// Normalize separators and redundant current-directory components, but reject traversal,
/// relative paths and Windows network/device namespaces before touching the filesystem.
fn normalized(path: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    let path = PathBuf::from(path.to_string_lossy().replace('/', "\\"));
    #[cfg(not(windows))]
    let path = path.to_path_buf();
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return None;
    }
    #[cfg(windows)]
    if !matches!(path.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), std::path::Prefix::Disk(_))) {
        return None;
    }
    Some(path.components().filter(|c| !matches!(c, Component::CurDir)).collect())
}

fn path_key(path: &Path) -> String {
    let key = path.to_string_lossy().to_string();
    if cfg!(windows) { key.to_ascii_lowercase() } else { key }
}

fn beneath(path: &Path, root: &Path) -> bool {
    Path::new(&path_key(path)).starts_with(Path::new(&path_key(root)))
}

/// Check ancestors from the drive down, so metadata is never read through a known link.
/// A read error is unknown, not a missing or empty directory.
fn real_metadata(path: &Path) -> Result<std::fs::Metadata, ()> {
    let mut parts: Vec<&Path> = path.ancestors().collect();
    parts.reverse();
    let mut leaf = None;
    for part in parts {
        let meta = std::fs::symlink_metadata(part).map_err(|_| ())?;
        if meta.is_symlink() {
            return Err(());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 { // FILE_ATTRIBUTE_REPARSE_POINT
                return Err(());
            }
        }
        leaf = Some(meta);
    }
    leaf.ok_or(())
}

fn directory(path: &Path) -> Result<std::fs::ReadDir, ()> {
    if !real_metadata(path)?.is_dir() {
        return Err(());
    }
    std::fs::read_dir(path).map_err(|_| ())
}

fn read_text(path: &Path, limit: u64, budget: &mut u64) -> Result<String, ()> {
    let meta = real_metadata(path)?;
    let limit = limit.min(*budget);
    if !meta.is_file() || meta.len() > limit || limit == 0 {
        return Err(());
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    let read = std::fs::File::open(path).map_err(|_| ())?.take(limit + 1).read_to_end(&mut bytes);
    // Invalid UTF-8 and partial I/O failures still spent part of the scan's read budget.
    *budget = budget.saturating_sub(bytes.len() as u64);
    read.map_err(|_| ())?;
    if bytes.len() as u64 > limit { Err(()) } else { String::from_utf8(bytes).map_err(|_| ()) }
}

fn app_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 10 && value.bytes().all(|b| b.is_ascii_digit()) && value.parse::<u32>().is_ok_and(|n| n > 0)
}

#[derive(Debug)]
enum Value {
    Text(String),
    Object(Vec<(String, Value)>),
}

#[derive(Debug)]
enum Token {
    Text(String),
    Open,
    Close,
}

/// The quoted-key subset Steam writes, including comments and escaped Windows paths.
/// Structural parsing prevents a nested or duplicate field from impersonating an app id.
fn vdf(text: &str) -> Result<Vec<(String, Value)>, ()> {
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    let mut tokens = Vec::new();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {}
            '/' if chars.peek() == Some(&'/') => {
                chars.next();
                for c in chars.by_ref() {
                    if c == '\n' { break; }
                }
            }
            '{' => tokens.push(Token::Open),
            '}' => tokens.push(Token::Close),
            '"' => {
                let mut value = String::new();
                let mut ended = false;
                while let Some(c) = chars.next() {
                    if c == '"' {
                        ended = true;
                        break;
                    }
                    if c == '\\' && matches!(chars.peek(), Some('\\' | '"')) {
                        value.push(chars.next().ok_or(())?);
                    } else {
                        value.push(c);
                    }
                    if value.len() > 32_768 { return Err(()); }
                }
                if !ended { return Err(()); }
                tokens.push(Token::Text(value));
            }
            _ => return Err(()),
        }
        if tokens.len() > 200_000 { return Err(()); }
    }
    fn object(tokens: &mut std::vec::IntoIter<Token>, nested: bool, depth: usize) -> Result<Vec<(String, Value)>, ()> {
        if depth > 16 { return Err(()); }
        let mut out = Vec::new();
        while let Some(token) = tokens.next() {
            let key = match token {
                Token::Close if nested => return Ok(out),
                Token::Text(key) => key,
                _ => return Err(()),
            };
            let value = match tokens.next() {
                Some(Token::Text(value)) => Value::Text(value),
                Some(Token::Open) => Value::Object(object(tokens, true, depth + 1)?),
                _ => return Err(()),
            };
            out.push((key, value));
        }
        if nested { Err(()) } else { Ok(out) }
    }
    object(&mut tokens.into_iter(), false, 0)
}

fn one<'a>(values: &'a [(String, Value)], name: &str) -> Result<&'a Value, ()> {
    let mut found = values.iter().filter(|(key, _)| key.eq_ignore_ascii_case(name));
    let (_, value) = found.next().ok_or(())?;
    if found.next().is_some() { Err(()) } else { Ok(value) }
}

fn text_value<'a>(values: &'a [(String, Value)], name: &str) -> Result<&'a str, ()> {
    match one(values, name)? {
        Value::Text(value) => Ok(value),
        _ => Err(()),
    }
}

fn document<'a>(values: &'a [(String, Value)], name: &str) -> Result<&'a [(String, Value)], ()> {
    if values.len() != 1 { return Err(()); }
    match one(values, name)? {
        Value::Object(object) => Ok(object),
        _ => Err(()),
    }
}

fn library_paths(text: &str) -> Result<(Vec<PathBuf>, bool), ()> {
    let parsed = vdf(text)?;
    let folders = document(&parsed, "libraryfolders")?;
    let mut roots = Vec::new();
    let mut complete = true;
    let mut seen = HashSet::new();
    for (key, value) in folders {
        if key.is_empty() || !key.bytes().all(|b| b.is_ascii_digit()) { continue; }
        if !seen.insert(key) { complete = false; }
        let path = match value {
            Value::Text(path) => Ok(path.as_str()), // older Steam library lists
            Value::Object(fields) => text_value(fields, "path"),
        };
        match path.ok().filter(|p| !p.trim().is_empty()).and_then(|p| normalized(Path::new(p))) {
            Some(path) => roots.push(path),
            None => complete = false,
        }
    }
    Ok((roots, complete && !seen.is_empty()))
}

struct Game {
    name: String,
    state: InstallState,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InstallState {
    Present,
    Missing,
    Unknown,
}

fn install_state(path: &Path) -> InstallState {
    // Only an explicit missing child of an accessible, unlinked parent establishes missing.
    // An offline drive, missing ancestor, denied access or reparse point remains unknown.
    if !path.parent().is_some_and(|parent| directory(parent).is_ok()) {
        return InstallState::Unknown;
    }
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => InstallState::Missing,
        Ok(_) if directory(path).is_ok() => InstallState::Present,
        _ => InstallState::Unknown,
    }
}

fn combined_state(a: InstallState, b: InstallState) -> InstallState {
    match (a, b) {
        (InstallState::Present, _) | (_, InstallState::Present) => InstallState::Present,
        (InstallState::Missing, InstallState::Missing) => InstallState::Missing,
        _ => InstallState::Unknown,
    }
}

fn manifest(text: &str, expected_id: &str) -> Result<(String, String), ()> {
    let parsed = vdf(text)?;
    let state = document(&parsed, "AppState")?;
    if text_value(state, "appid")? != expected_id || !app_id(expected_id) { return Err(()); }
    let name = text_value(state, "name")?.trim();
    let folder = text_value(state, "installdir")?;
    if name.is_empty() || name.len() > 1024 || name.chars().any(char::is_control)
        || folder.is_empty() || folder.trim() != folder || folder.ends_with('.')
        || folder.contains(['\\', '/', ':']) || folder == "." || folder == ".." || folder.chars().any(char::is_control)
    {
        return Err(());
    }
    Ok((name.to_string(), folder.to_string()))
}

fn discover_at(root: &Path, allowed: &[PathBuf]) -> Vec<Cache> {
    let mut budget = CATALOG_BYTES;
    let mut complete = true;
    let mut libraries = vec![root.to_path_buf()];
    match read_text(&root.join("steamapps").join("libraryfolders.vdf"), LIST_BYTES, &mut budget).and_then(|s| library_paths(&s)) {
        Ok((more, read_all)) => {
            complete &= read_all;
            libraries.extend(more);
        }
        Err(()) => complete = false,
    }
    let mut seen = HashSet::new();
    libraries.retain(|library| seen.insert(path_key(library)));
    if libraries.len() > MAX_LIBRARIES {
        libraries.truncate(MAX_LIBRARIES);
        complete = false;
    }
    let mut games: HashMap<String, Game> = HashMap::new();
    let mut caches = Vec::new();
    let mut entries_left = MAX_ENTRIES;
    for library in libraries {
        if entries_left == 0 { complete = false; break; }
        if !allowed.iter().any(|drive| beneath(&library, drive)) {
            complete = false;
            continue;
        }
        let apps = library.join("steamapps");
        let Ok(entries) = directory(&apps) else {
            complete = false;
            continue;
        };
        for entry in entries {
            if entries_left == 0 { complete = false; break; }
            entries_left -= 1;
            let Ok(entry) = entry else { complete = false; continue };
            let filename = entry.file_name().to_string_lossy().to_ascii_lowercase();
            let Some(id) = filename.strip_prefix("appmanifest_").and_then(|n| n.strip_suffix(".acf")) else { continue };
            if !app_id(id) { complete = false; continue; }
            let record = read_text(&entry.path(), MANIFEST_BYTES, &mut budget).and_then(|s| manifest(&s, id));
            let Ok((name, folder)) = record else { complete = false; continue };
            let state = install_state(&apps.join("common").join(folder));
            games.entry(id.to_string()).and_modify(|game| {
                if state == InstallState::Present { game.name = name.clone(); }
                game.state = combined_state(game.state, state);
            }).or_insert(Game { name, state });
        }
        let shader_root = apps.join("shadercache");
        let Ok(entries) = directory(&shader_root) else { continue };
        for entry in entries {
            if entries_left == 0 { complete = false; break; }
            entries_left -= 1;
            let Ok(entry) = entry else { continue };
            let id = entry.file_name().to_string_lossy().to_string();
            if !app_id(&id) { continue; }
            let path = entry.path();
            // No recursive measuring here. Empty folders contribute no reclaimable bytes;
            // the ordinary trash tally measures and judges the remaining candidates.
            if directory(&path).is_ok_and(|mut children| matches!(children.next(), Some(Ok(_)))) {
                caches.push((path, id));
            }
        }
    }
    caches.sort_by(|a, b| a.0.cmp(&b.0));
    caches.into_iter().map(|(path, id)| {
        let game = games.get(&id);
        let name = game.map(|g| g.name.clone()).unwrap_or_else(|| format!("Steam app {id}"));
        let status = match game {
            Some(g) if g.state == InstallState::Present => INSTALLED,
            Some(g) if g.state == InstallState::Missing => MISSING,
            None if complete => NOT_FOUND,
            _ => UNKNOWN,
        };
        let uninstalled = game.is_none() && complete;
        let detail = format!("Steam app {id} · {status}");
        Cache { path, id: id.parse().unwrap_or(0), name, named: game.is_some(), uninstalled, detail }
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture { root: PathBuf, paths: Paths }

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("mom-steam-shaders-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(&root).unwrap();
            let paths = Paths::under(&root);
            let me = Self { root, paths };
            std::fs::create_dir_all(me.steam().join("steamapps")).unwrap();
            me
        }
        fn steam(&self) -> PathBuf { self.paths.program_files_x86.join("Steam") }
        fn libraries(&self, roots: &[PathBuf]) {
            let entries: String = roots.iter().enumerate().map(|(i, path)| {
                format!("\"{i}\" {{ \"path\" \"{}\" }}\n", path.to_string_lossy().replace('\\', "\\\\"))
            }).collect();
            std::fs::write(self.steam().join("steamapps/libraryfolders.vdf"), format!("\"libraryfolders\" {{ {entries} }}")).unwrap();
        }
        fn cache(&self, library: &Path, id: &str) -> PathBuf {
            let path = library.join("steamapps/shadercache").join(id);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("cache.bin"), b"shader").unwrap();
            path
        }
        fn game(&self, library: &Path, id: &str, name: &str, folder: &str, installed: bool) {
            let apps = library.join("steamapps");
            std::fs::create_dir_all(apps.join("common")).unwrap();
            std::fs::write(apps.join(format!("appmanifest_{id}.acf")), format!(
                "\"AppState\" {{ \"appid\" \"{id}\" \"name\" \"{name}\" \"installdir\" \"{folder}\" }}"
            )).unwrap();
            if installed { std::fs::create_dir_all(apps.join("common").join(folder)).unwrap(); }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
    }

    #[test]
    fn cache_matches_game_in_another_library_and_duplicate_roots_are_counted_once() {
        let mut f = Fixture::new();
        let other = f.root.join("other-drive");
        let library = other.join("SteamLibrary");
        f.paths.other_drives.push(other);
        f.libraries(&[f.steam(), library.clone(), library.join(".")]);
        let cache = f.cache(&f.steam(), "123");
        f.game(&f.steam(), "123", "Old Game", "Moved Out", false);
        f.game(&library, "123", "A Game", "A Game", true);
        let found = discover(&f.paths);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, cache);
        assert_eq!(found[0].name, "A Game");
        assert!(found[0].detail.contains(INSTALLED));
    }

    #[test]
    fn missing_library_or_missing_catalog_never_proves_absence() {
        let mut f = Fixture::new();
        let other = f.root.join("unavailable-drive");
        f.paths.other_drives.push(other.clone());
        f.libraries(&[f.steam(), other.join("SteamLibrary")]);
        f.cache(&f.steam(), "456");
        assert!(discover(&f.paths)[0].detail.contains(UNKNOWN));
        std::fs::remove_file(f.steam().join("steamapps/libraryfolders.vdf")).unwrap();
        assert!(discover(&f.paths)[0].detail.contains(UNKNOWN));
    }

    #[test]
    fn complete_catalog_can_report_not_found_and_omits_empty_or_nonnumeric_caches() {
        let f = Fixture::new();
        f.libraries(&[f.steam()]);
        f.cache(&f.steam(), "789");
        f.cache(&f.steam(), "not-a-game");
        std::fs::create_dir_all(f.steam().join("steamapps/shadercache/987")).unwrap();
        let found = discover(&f.paths);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Steam app 789");
        assert!(found[0].detail.contains(NOT_FOUND));
    }

    #[test]
    fn missing_install_directory_is_distinct_from_an_incomplete_catalog() {
        let f = Fixture::new();
        f.libraries(&[f.steam()]);
        f.cache(&f.steam(), "123");
        f.cache(&f.steam(), "456");
        f.game(&f.steam(), "123", "Moved Game", "Moved Game", false);
        let found = discover(&f.paths);
        assert!(found.iter().any(|c| c.name == "Moved Game" && c.detail.contains(MISSING)));
        assert!(found.iter().any(|c| c.name == "Steam app 456" && c.detail.contains(NOT_FOUND)));
        std::fs::write(f.steam().join("steamapps/appmanifest_123.acf"), "\"AppState\" { broken").unwrap();
        assert!(discover(&f.paths).iter().all(|c| c.detail.contains(UNKNOWN)));
    }

    #[test]
    fn bad_utf8_consumes_the_read_budget_and_present_copy_wins() {
        let f = Fixture::new();
        let path = f.steam().join("steamapps/bad.acf");
        std::fs::write(&path, [0xff, 0xfe, 0xff, 0xfe]).unwrap();
        let mut budget = 6;
        assert!(read_text(&path, MANIFEST_BYTES, &mut budget).is_err());
        assert_eq!(budget, 2);
        assert!(read_text(&path, MANIFEST_BYTES, &mut budget).is_err());
        assert_eq!(budget, 2, "oversized files must not be read");
        assert!(combined_state(InstallState::Missing, InstallState::Present) == InstallState::Present);
        assert!(combined_state(InstallState::Present, InstallState::Unknown) == InstallState::Present);
        assert!(combined_state(InstallState::Missing, InstallState::Unknown) == InstallState::Unknown);
    }

    #[test]
    fn rejects_traversal_mismatched_ids_duplicate_fields_and_incomplete_vdf() {
        for folder in ["..", "../elsewhere", "a/b", r"a\b", "C:elsewhere", "game."] {
            let text = format!("\"AppState\" {{ \"appid\" \"123\" \"name\" \"Game\" \"installdir\" \"{folder}\" }}");
            assert!(manifest(&text, "123").is_err(), "{folder}");
        }
        assert!(manifest(r#""AppState" { "appid" "999" "name" "Game" "installdir" "Game" }"#, "123").is_err());
        assert!(manifest(r#""AppState" { "appid" "123" "appid" "123" "name" "Game" "installdir" "Game" }"#, "123").is_err());
        assert!(vdf(r#""libraryfolders" { "0" { "path" "C:\\Steam" }"#).is_err());
        assert!(normalized(Path::new("relative/Steam")).is_none());
        assert!(normalized(&std::env::temp_dir().join("../elsewhere")).is_none());
        let f = Fixture::new();
        f.libraries(&[f.steam(), f.root.join("outside-allowed-drives")]);
        f.cache(&f.steam(), "123");
        assert!(discover(&f.paths)[0].detail.contains(UNKNOWN));
    }
}
