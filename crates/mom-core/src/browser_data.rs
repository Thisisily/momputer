//! Cleanup must preserve everything a browser keeps for you: sign-ins, site storage, session
//! recovery and whatever else it stores, known or not. Mom doesn't keep a list of the files
//! that hold those. She recognises a browser's folders by their shape and protects all of each
//! one, except an allow-list of cache folders the browser rebuilds by itself.
//!
//! A browser folder is a folder under a known browser root (Chromium's `User Data`, Firefox
//! profiles, Opera, Epic's web caches; a folder named webcache can be a complete Chromium
//! profile), or any folder that shows a browser's marks: Chromium's `Local State`, Firefox's
//! `prefs.js` or `profiles.ini`, web storage folders, a `Network` folder holding Chromium's
//! network state, or a `Preferences` file next to another profile folder.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

pub(crate) const CHROMIUM_CACHE_DIRS: &[&str] = &[
    "Cache", "Code Cache", "GPUCache", "DawnCache", "DawnWebGPUCache",
    "DawnGraphiteCache", "GrShaderCache", "ShaderCache",
];

/// Firefox's caches, in its local profile.
const FIREFOX_CACHE_DIRS: &[&str] = &["cache2", "startupCache"];

/// Folders where sites keep data in a browser. Their names mark a browser folder.
const SITE_STORAGE_DIRS: &[&str] = &["Local Storage", "IndexedDB", "Service Worker", "webstorage"];

/// Files whose names alone mark a browser's data folder: Chromium's `Local State`, and a
/// Firefox profile's preferences or the list of its profiles.
const MARKER_FILES: &[&str] = &["Local State", "prefs.js", "profiles.ini"];

/// Network state Chromium keeps in its `Network` folder (security policies and connection
/// hints), which shows that folder belongs to a browser profile.
const NETWORK_STATE: &[&str] = &["TransportSecurity", "Network Persistent State"];

/// Folders that sit next to `Preferences` in a Chromium profile.
const PROFILE_COMPANIONS: &[&str] = &["Code Cache", "Network", "GPUCache"];

fn one_of(list: &[&str], name: &str) -> bool {
    list.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// Does this entry, by its name (and, for network state, its folder's), show a browser's data
/// is here? Never by naming what a browser stores: only the marks listed above.
pub(crate) fn marks_browser_data(path: &Path, directory: bool) -> bool {
    let Some(name) = path.file_name().map(OsStr::to_string_lossy) else { return false };
    if directory {
        return one_of(SITE_STORAGE_DIRS, &name);
    }
    one_of(MARKER_FILES, &name)
        || (one_of(NETWORK_STATE, &name)
            && path.parent().and_then(Path::file_name).is_some_and(|parent| parent.eq_ignore_ascii_case("Network")))
}

fn cache_name(name: &str) -> bool {
    one_of(CHROMIUM_CACHE_DIRS, name) || one_of(FIREFOX_CACHE_DIRS, name)
}

fn cache_relative(path: &Path) -> bool {
    let parts: Vec<_> = path.components().map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase()).collect();
    parts.first().is_some_and(|name| cache_name(name))
        || (parts.first().is_some_and(|name| name == "partitions") && parts.get(2).is_some_and(|name| cache_name(name)))
}

fn protected_boundary(path: &Path) -> bool {
    known_profile_home(path).is_some_and(|home| !path.strip_prefix(home).is_ok_and(cache_relative))
}

/// Is `path` in (or at) a real browser's own folder, such as Chrome's User Data or a Firefox
/// profile, by its name and place?
pub(crate) fn in_browser_home(path: &Path) -> bool {
    known_profile_home(path).is_some()
}

fn known_profile_home(path: &Path) -> Option<PathBuf> {
    let parts: Vec<String> = path.components().map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase()).collect();
    known_profile_boundary(&parts).map(|home| path.components().take(home + 1).collect())
}

fn plain_metadata(path: &Path) -> Option<std::fs::Metadata> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() { return None; }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 { return None; }
    }
    (meta.is_file() || meta.is_dir()).then_some(meta)
}

/// What is at `path`: Some(true) for a file, Some(false) for a folder or link, None if nothing
/// is there, Err if it can't be told.
fn entry_kind(path: &Path) -> Result<Option<bool>, ()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => Ok(Some(meta.is_file())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(()),
    }
}

/// Is `dir` a browser's data folder (a profile, or the folder that holds profiles)? None when
/// it can't be told, which callers treat as yes.
fn browser_folder(dir: &Path) -> Option<bool> {
    for name in MARKER_FILES {
        // Only a file counts: a folder or link by one of these names isn't the browser's.
        if entry_kind(&dir.join(name)).ok()? == Some(true) { return Some(true); }
    }
    for name in SITE_STORAGE_DIRS {
        if entry_kind(&dir.join(name)).ok()? == Some(false) { return Some(true); }
    }
    for name in NETWORK_STATE {
        if entry_kind(&dir.join("Network").join(name)).ok()?.is_some() { return Some(true); }
    }
    // A new profile may hold nothing else yet; a Preferences file alone is too common to count.
    if entry_kind(&dir.join("Preferences")).ok()? == Some(true) {
        for name in PROFILE_COMPANIONS {
            if entry_kind(&dir.join(name)).ok()?.is_some() { return Some(true); }
        }
    }
    Some(false)
}

/// Browser boundaries that are protected by where they are, whatever they hold.
fn known_profile_boundary(parts: &[String]) -> Option<usize> {
    for (i, part) in parts.iter().enumerate() {
        if part == "user data" {
            if parts.len() <= i + 1 { return Some(i); }
            // Snapshots contain backup profiles too; they are not disposable app binaries.
            return Some(if parts[i + 1] == "snapshots" { i } else { i + 1 });
        }
        if part == "profiles" && i >= 2 && parts[i - 1] == "firefox" && parts[i - 2] == "mozilla" {
            return Some((i + 1).min(parts.len() - 1));
        }
        if part == "opera software" {
            let edition = (i + 1).min(parts.len() - 1);
            let profile = edition + 1;
            return Some(if parts.get(profile).is_some_and(|p| p == "default" || p == "guest profile" || p.starts_with("profile ")) { profile } else { edition });
        }
        if ((part == "saved" && i > 0 && parts[i - 1] == "epicgameslauncher")
            || (part == "eosoverlay" && i > 0 && parts[i - 1] == "epic games"))
            && parts.get(i + 1).is_some_and(|p| p == "webcache" || p.starts_with("webcache_") || p == "browsercache")
        {
            return Some(i + 1);
        }
    }
    None
}

/// A path is protected if it is in a browser folder (anything there but its caches), is one of
/// a browser's marks, or can't be checked without following links. Cache folders of the nearest
/// browser folder, and what's inside them, are allowed.
pub(crate) fn protects_path(path: &Path) -> bool {
    protects_path_with(path, &mut Looked::default())
}

/// What `protects_path` found out about folders, kept while one list is checked: thousands of
/// items share the folders above them (C:\, C:\Users, … Temp), and each of those is looked at
/// once instead of once per item.
#[derive(Default)]
pub(crate) struct Looked {
    /// A real folder (true) or file (false) here; None for a link or anything else.
    kind: std::collections::HashMap<PathBuf, Option<bool>>,
    browser: std::collections::HashMap<PathBuf, Option<bool>>,
}

impl Looked {
    fn kind(&mut self, p: &Path) -> Option<bool> {
        if let Some(k) = self.kind.get(p) {
            return *k;
        }
        let k = plain_metadata(p).map(|m| m.is_dir());
        self.kind.insert(p.to_path_buf(), k);
        k
    }

    fn browser(&mut self, p: &Path) -> Option<bool> {
        if let Some(b) = self.browser.get(p) {
            return *b;
        }
        let b = browser_folder(p);
        self.browser.insert(p.to_path_buf(), b);
        b
    }
}

/// `protects_path`, remembering in `looked` what it found out about folders.
pub(crate) fn protects_path_with(path: &Path, looked: &mut Looked) -> bool {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return true;
    }
    let known_home = known_profile_home(path);
    if known_home.as_ref().is_some_and(|home| !path.strip_prefix(home).is_ok_and(cache_relative)) { return true; }
    // Validate the entire ancestry before allowing a leaf below the nearest browser folder.
    let Some(ancestors) = path.ancestors().map(|ancestor| looked.kind(ancestor).map(|dir| (ancestor, dir))).collect::<Option<Vec<_>>>() else { return true };
    for (ancestor, dir) in &ancestors {
        if marks_browser_data(ancestor, *dir) { return true; }
    }
    for (ancestor, dir) in ancestors {
        if !dir { continue; }
        // A known profile was checked above; the folder holding it (with its Local State)
        // must not override the cache leaf already allowed in this nearer profile.
        if known_home.as_deref() == Some(ancestor) { break; }
        match looked.browser(ancestor) {
            Some(false) => {},
            Some(true) => {
                if !path.strip_prefix(ancestor).is_ok_and(cache_relative) { return true; }
                // This is the nearest browser folder, and the path is one of its caches.
                break;
            },
            None => return true,
        }
    }
    false
}

/// Cheap per-entry backstop during deletion: also catches browser data that appears after the
/// tree was checked.
pub(crate) fn protects_entry_name(path: &Path, directory: bool) -> bool {
    protected_boundary(path)
        || marks_browser_data(path, directory)
        || path.ancestors().skip(1).any(|ancestor| marks_browser_data(ancestor, true))
}

/// A move must not create data in a browser folder either. Check prospective names and the
/// closest existing ancestor, without requiring an as-yet-uncreated destination to exist.
pub(crate) fn protects_destination(path: &Path, directory: bool) -> bool {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) || protected_boundary(path) {
        return true;
    }
    let mut first = true;
    for ancestor in path.ancestors() {
        if marks_browser_data(ancestor, !first || directory) { return true; }
        first = false;
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => return protects_path(ancestor),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(_) => return true,
        }
    }
    true
}

/// Preflight the actual deletion target again, by names and folder shapes only: no file is
/// opened. A tree that holds a browser folder anywhere, or that is unknown, linked, oversized
/// or partly unreadable, stays in place.
pub(crate) fn tree_is_safe(path: &Path) -> bool {
    if protects_path(path) { return false; }
    let mut stack = vec![path.to_path_buf()];
    let mut left = 2_000_000usize;
    while let Some(path) = stack.pop() {
        if left == 0 { return false; }
        left -= 1;
        let Some(meta) = plain_metadata(&path) else { return false };
        if marks_browser_data(&path, meta.is_dir()) { return false; }
        if !meta.is_dir() { continue; }
        if protected_boundary(&path) { return false; }
        let Ok(entries) = std::fs::read_dir(&path) else { return false };
        let mut preferences = false;
        let mut profile_companion = false;
        for entry in entries {
            let Ok(entry) = entry else { return false };
            let name = entry.file_name().to_string_lossy().into_owned();
            preferences |= name.eq_ignore_ascii_case("Preferences");
            profile_companion |= one_of(PROFILE_COMPANIONS, &name);
            if preferences && profile_companion { return false; }
            if stack.len() >= left { return false; }
            stack.push(entry.path());
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/mom-test-fixtures").join(format!("{name}-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        std::fs::create_dir_all(&root).unwrap();
        // Resolve the test-only ../ components; production guards reject ambiguous paths.
        std::fs::canonicalize(root).unwrap()
    }

    fn write(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
    }

    /// Files a browser might keep that mom has never heard of: protected all the same.
    const UNKNOWN: &[&str] = &["made-up-store.db", "Unknown Folder/made-up-record", "Sessions/made-up-session", "Partitions/site/made-up-partition-data"];

    #[test]
    fn whole_browser_folders_stay_but_their_cache_folders_are_allowed() {
        let root = fixture("browser-protection");
        // A known root, a known web cache that is a whole profile, and an app's profile
        // recognised only by its marks.
        let homes = [root.join("Google/Chrome/User Data/Default"), root.join("EpicGamesLauncher/Saved/webcache_4430"), root.join("SomeApp/Partitions/site")];
        for home in &homes {
            for unknown in UNKNOWN {
                write(&home.join(unknown));
            }
            write(&home.join("Preferences"));
            write(&home.join("Network/TransportSecurity"));
            write(&home.join("Local Storage/leveldb/000003.log"));
            write(&home.join("Cache/Cache_Data/data_0"));
            write(&home.join("Code Cache/js/index"));
            assert!(protects_path(home));
            for unknown in UNKNOWN {
                assert!(protects_path(&home.join(unknown)), "{unknown}");
            }
            assert!(protects_path(&home.join("Preferences")) && protects_path(&home.join("Network")));
            assert!(!protects_path(&home.join("Cache")) && !protects_path(&home.join("Code Cache/js/index")));
            assert!(tree_is_safe(&home.join("Cache")) && tree_is_safe(&home.join("Code Cache")));
            assert!(!tree_is_safe(home));
            assert!(protects_destination(&home.join("Unknown Folder/new-file"), false));
        }
        assert!(!tree_is_safe(&root), "a broad parent contains browser folders");
        // Ordinary folders with the same everyday names aren't browser data by name alone.
        write(&root.join("ordinary-project/Sessions/history.json"));
        write(&root.join("ordinary-project/Network/settings.json"));
        write(&root.join("ordinary-settings/Preferences"));
        assert!(tree_is_safe(&root.join("ordinary-project")) && tree_is_safe(&root.join("ordinary-settings")));
        assert!(!protects_destination(&root.join("ordinary-project/new-folder/new-file"), false));
        let nested = root.join("Google/Chrome/User Data/Default/Partitions/site/Cache");
        write(&nested.join("data"));
        assert!(tree_is_safe(&nested));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_profile_hiding_inside_a_cache_and_unreadable_paths_are_rejected() {
        let root = fixture("browser-cache-protection");
        let cache = root.join("Google/Chrome/User Data/Default/Cache");
        write(&cache.join("Cache_Data/data_0"));
        assert!(tree_is_safe(&cache));
        // Each of a browser folder's marks, nested inside a cache, locks the whole cache.
        for (i, mark) in ["nested/Local State", "nested/prefs.js", "nested/Local Storage/leveldb/log", "nested/Network/Network Persistent State", "nested/Network/TransportSecurity"].iter().enumerate() {
            let cache = root.join(format!("App {i}/User Data/Default/Cache"));
            write(&cache.join("Cache_Data/data_0"));
            write(&cache.join(mark));
            assert!(!tree_is_safe(&cache), "{mark}");
            assert!(protects_path(&cache.join(mark)), "{mark}");
        }
        let both = root.join("App 9/User Data/Default/Cache/nested");
        write(&both.join("Preferences"));
        std::fs::create_dir_all(both.join("GPUCache")).unwrap();
        assert!(!tree_is_safe(both.parent().unwrap()));
        assert!(!tree_is_safe(&root.join("missing")));
        assert!(!tree_is_safe(Path::new("relative/Cache")));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn local_state_above_a_new_profile_does_not_block_only_its_cache_leaves() {
        let root = fixture("new-profile");
        let data = root.join("Google/Chrome/User Data");
        let home = data.join("Profile 2");
        write(&data.join("Local State"));
        for name in ["Cache", "Code Cache", "GPUCache"] {
            let cache = home.join(name);
            write(&cache.join("data"));
            assert!(!protects_path(&cache));
            assert!(tree_is_safe(&cache));
        }
        assert!(protects_path(&data.join("Local State")));
        assert!(protects_path(&data) && protects_path(&home));
        assert!(!tree_is_safe(&data) && !tree_is_safe(&home));
        // An app folder recognised only by its Local State protects everything but caches of
        // the folder itself; a cache further down isn't one of its caches.
        let generic = root.join("UnknownApp");
        write(&generic.join("Local State"));
        write(&generic.join("ArbitraryChild/Cache/data"));
        write(&generic.join("Cache/data"));
        assert!(protects_path(&generic.join("ArbitraryChild/Cache")));
        assert!(!protects_path(&generic.join("Cache")) && tree_is_safe(&generic.join("Cache")));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn firefox_profiles_keep_everything_but_their_caches_wherever_they_are() {
        let root = fixture("firefox-protection");
        let standard = root.join("Mozilla/Firefox/Profiles/ab12cd.default-release");
        let portable = root.join("PortableFox/Data/profile");
        write(&portable.join("prefs.js"));
        for home in [&standard, &portable] {
            for unknown in UNKNOWN {
                write(&home.join(unknown));
            }
            write(&home.join("cache2/entries/E1"));
            write(&home.join("startupCache/scriptCache.bin"));
            for unknown in UNKNOWN {
                assert!(protects_path(&home.join(unknown)), "{unknown}");
            }
            assert!(!protects_path(&home.join("cache2")) && tree_is_safe(&home.join("cache2")));
            assert!(!protects_path(&home.join("startupCache")) && tree_is_safe(&home.join("startupCache")));
            assert!(!tree_is_safe(home));
        }
        let profiles = root.join("SomeMailApp");
        write(&profiles.join("profiles.ini"));
        write(&profiles.join("Profiles/xyz.default/made-up-record"));
        assert!(protects_path(&profiles.join("Profiles/xyz.default/made-up-record")));
        assert!(!tree_is_safe(&profiles));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn marks_are_read_from_names_and_folders_only() {
        assert!(marks_browser_data(Path::new("C:/a/Local State"), false));
        assert!(!marks_browser_data(Path::new("C:/a/Local State"), true), "a folder named Local State isn't Chromium's file");
        assert!(marks_browser_data(Path::new("C:/a/IndexedDB"), true));
        assert!(marks_browser_data(Path::new("C:/a/Network/TransportSecurity"), false));
        assert!(!marks_browser_data(Path::new("C:/a/TransportSecurity"), false), "only in a Network folder");
        assert!(!marks_browser_data(Path::new("C:/a/Preferences"), false), "too common on its own");
        assert!(!marks_browser_data(Path::new("C:/a/Sessions"), true));
    }
}
