//! Cleanup must preserve browser authentication, site storage and session recovery data.
//! A folder named `webcache` can be a complete Chromium profile. Only the known disposable
//! cache children of a profile are eligible; a broad parent must never swallow that profile.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

pub(crate) const CHROMIUM_CACHE_DIRS: &[&str] = &[
    "Cache", "Code Cache", "GPUCache", "DawnCache", "DawnWebGPUCache",
    "DawnGraphiteCache", "GrShaderCache", "ShaderCache",
];

/// These names identify persistent browser data without reading its contents. Generic names
/// such as `Sessions`, `storage` and `Preferences` need profile context instead.
pub(crate) fn sensitive_entry(name: &OsStr, directory: bool) -> bool {
    let name = name.to_string_lossy().to_ascii_lowercase();
    if directory {
        matches!(name.as_str(), "local storage" | "session storage" | "indexeddb" | "service worker"
            | "webstorage" | "sessionstore-backups" | "local extension settings" | "sync extension settings")
    } else {
        ["cookies", "cookies.sqlite", "login data", "login data for account", "web data"]
            .iter().any(|base| name == *base || ["-journal", "-wal", "-shm"].iter().any(|suffix| name == format!("{base}{suffix}")))
            || matches!(name.as_str(), "local state" | "logins.json" | "key3.db" | "key4.db" | "cert9.db"
                | "sessionstore.jsonlz4" | "sessionstore.js" | "sessioncheckpoints.json"
                | "current session" | "current tabs" | "last session" | "last tabs" | "token service")
            || ["session_", "tabs_"].iter().any(|prefix| name.strip_prefix(prefix).is_some_and(|tail| !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit())))
    }
}

fn cache_name(name: &str) -> bool {
    CHROMIUM_CACHE_DIRS.iter().any(|n| n.eq_ignore_ascii_case(name)) || name.eq_ignore_ascii_case("cache2")
}

fn cache_relative(path: &Path) -> bool {
    let parts: Vec<_> = path.components().map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase()).collect();
    parts.first().is_some_and(|name| cache_name(name))
        || (parts.first().is_some_and(|name| name == "partitions") && parts.get(2).is_some_and(|name| cache_name(name)))
}

fn protected_boundary(path: &Path) -> bool {
    known_profile_home(path).is_some_and(|home| !path.strip_prefix(home).is_ok_and(cache_relative))
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

// A complete shallow listing avoids probing credential paths individually. Unknown or oversized
// listings do not establish a profile boundary: that boundary can permit a known cache child.
const PROFILE_ENTRY_LIMIT: usize = 4096;
const PROFILE_FILES: &[&str] = &["Cookies", "Cookies-journal", "Cookies-wal", "Login Data", "Local State",
    "cookies.sqlite", "cookies.sqlite-wal", "logins.json", "key3.db", "key4.db", "sessionstore.jsonlz4"];
const PROFILE_DIRECTORIES: &[&str] = &["Local Storage", "Session Storage", "IndexedDB", "Service Worker", "sessionstore-backups"];
const NETWORK_FILES: &[&str] = &["Cookies", "Cookies-journal", "Cookies-wal"];
const PROFILE_COMPANIONS: &[&str] = &["Preferences", "Code Cache", "Network"];

#[cfg(any(windows, test))]
fn marker_alias_blocks(name: &str, alias: &str, directory: bool, profile: bool) -> bool {
    if alias.is_empty() || alias.eq_ignore_ascii_case(name) { return false; }
    if alias.eq_ignore_ascii_case("Cookies") && directory { return false; }
    let files = if profile { PROFILE_FILES } else { NETWORK_FILES };
    files.iter().chain(if profile { PROFILE_DIRECTORIES } else { &[] }.iter())
        .chain(if profile { PROFILE_COMPANIONS } else { &[] }.iter())
        .any(|name| name.eq_ignore_ascii_case(alias))
}

// std::DirEntry omits Windows 8.3 names. A second bounded listing preserves those possible
// markers without probing a credential path or creating a new cache-permitting boundary.
#[cfg(windows)]
fn marker_aliases_are_clear(dir: &Path, profile: bool) -> Option<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{GetLastError, HANDLE, INVALID_HANDLE_VALUE, ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES};
    use windows_sys::Win32::Storage::FileSystem::{FindClose, FindFirstFileW, FindNextFileW, WIN32_FIND_DATAW};
    struct Search(HANDLE);
    impl Drop for Search {
        fn drop(&mut self) { unsafe { FindClose(self.0); } }
    }
    let mut pattern: Vec<u16> = dir.join("*").as_os_str().encode_wide().collect();
    if pattern.contains(&0) { return None; }
    pattern.push(0);
    let wide = |value: &[u16]| String::from_utf16_lossy(&value[..value.iter().position(|c| *c == 0).unwrap_or(value.len())]);
    unsafe {
        let mut data: WIN32_FIND_DATAW = std::mem::zeroed();
        let handle = FindFirstFileW(pattern.as_ptr(), &mut data);
        if handle == INVALID_HANDLE_VALUE {
            let error = GetLastError();
            return if error == ERROR_FILE_NOT_FOUND && plain_metadata(dir).is_some_and(|m| m.is_dir()) { Some(()) } else { None };
        }
        let search = Search(handle);
        let mut left = PROFILE_ENTRY_LIMIT;
        loop {
            let name = wide(&data.cFileName);
            if name != "." && name != ".." {
                if left == 0 { return None; }
                left -= 1;
                let alias = wide(&data.cAlternateFileName);
                if marker_alias_blocks(&name, &alias, data.dwFileAttributes & 0x10 != 0, profile) { return None; }
            }
            if FindNextFileW(search.0, &mut data) == 0 {
                let error = GetLastError();
                return (error == ERROR_NO_MORE_FILES).then_some(());
            }
        }
    }
}

fn profile_marker(dir: &Path) -> Option<bool> {
    if !plain_metadata(dir)?.is_dir() { return None; }
    #[cfg(windows)]
    marker_aliases_are_clear(dir, true)?;
    let mut left = PROFILE_ENTRY_LIMIT;
    profile_marker_entries(std::fs::read_dir(dir).ok()?, &mut left, true)
}

fn profile_marker_entries(
    entries: impl IntoIterator<Item = std::io::Result<std::fs::DirEntry>>,
    left: &mut usize,
    profile: bool,
) -> Option<bool> {
    // Preserve the existing vocabulary and its depth. More marker names could create a
    // nearer cache-permitting boundary inside a broader protected browser container.
    let files = if profile { PROFILE_FILES } else { NETWORK_FILES };
    let (mut marker, mut preferences, mut companion) = (false, false, false);
    let mut network = None;
    for entry in entries {
        if *left == 0 { return None; }
        *left -= 1;
        let entry = entry.ok()?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let relevant = files.iter().chain(if profile { PROFILE_DIRECTORIES } else { &[] }.iter())
            .chain(if profile { PROFILE_COMPANIONS } else { &[] }.iter());
        if !relevant.clone().any(|n| n.eq_ignore_ascii_case(name)) { continue; }
        // DirEntry metadata does not follow links. On Windows it comes from the directory
        // enumeration without an extra metadata query to each named credential file.
        let meta = entry.metadata().ok()?;
        let linked = meta.file_type().is_symlink();
        let directory = meta.is_dir();
        #[cfg(windows)]
        let (linked, directory) = {
            use std::os::windows::fs::MetadataExt;
            (linked || meta.file_attributes() & 0x400 != 0, directory || meta.file_attributes() & 0x10 != 0)
        };
        // The legacy user-home Cookies directory/junction is not a cookie database.
        if name.eq_ignore_ascii_case("Cookies") && directory { continue; }
        // Case-sensitive Windows directories and Unix paths must not gain new permissive
        // profile boundaries merely because a spelling resembles a marker.
        if !relevant.clone().any(|n| *n == name) {
            return None;
        }
        if files.contains(&name) {
            if linked { return None; }
            marker |= meta.is_file();
        }
        if !profile { continue; }
        if PROFILE_DIRECTORIES.contains(&name) {
            let recognized = meta.is_dir() || meta.file_type().is_symlink();
            if linked && !recognized { return None; }
            marker |= recognized;
        }
        if name == "Preferences" {
            if linked { return None; }
            preferences = meta.is_file();
        }
        companion |= matches!(name, "Code Cache" | "Network");
        if name == "Network" {
            // Never follow a linked Network, and do not mistake ENOTDIR for absent cookies.
            if linked || !meta.is_dir() { return None; }
            network = Some(entry.path());
        }
    }
    if let Some(network) = network {
        if !plain_metadata(&network)?.is_dir() { return None; }
        #[cfg(windows)]
        marker_aliases_are_clear(&network, false)?;
        marker |= profile_marker_entries(std::fs::read_dir(network).ok()?, left, false)?;
    }
    Some(marker || (preferences && companion))
}

/// Browser boundaries that remain protected even when the cookie database is absent or locked.
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
        if (part == "saved" && i > 0 && parts[i - 1] == "epicgameslauncher")
            || (part == "eosoverlay" && i > 0 && parts[i - 1] == "epic games") {
            if parts.get(i + 1).is_some_and(|p| p == "webcache" || p.starts_with("webcache_") || p == "browsercache") {
                return Some(i + 1);
            }
        }
    }
    None
}

/// A path is protected if it is browser data, a whole profile, or cannot be checked without
/// following links. Known cache leaves are allowed even though their parent profile is not.
pub(crate) fn protects_path(path: &Path) -> bool {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return true;
    }
    let known_home = known_profile_home(path);
    if known_home.as_ref().is_some_and(|home| !path.strip_prefix(home).is_ok_and(cache_relative)) { return true; }
    // Validate the entire ancestry before allowing a leaf below the nearest profile.
    let Some(ancestors) = path.ancestors().map(|ancestor| plain_metadata(ancestor).map(|meta| (ancestor, meta))).collect::<Option<Vec<_>>>() else { return true };
    for (ancestor, meta) in &ancestors {
        if ancestor.file_name().is_some_and(|name| sensitive_entry(name, meta.is_dir())) { return true; }
    }
    for (ancestor, meta) in ancestors {
        if !meta.is_dir() { continue; }
        // A recognized profile need not have cookies yet. Its enclosing User Data has
        // Local State, but that container must not override the cache leaf already checked
        // against this nearer profile. All ancestor links and sensitive names were checked
        // above; unexpected profile markers inside the cache are still checked below.
        if known_home.as_deref() == Some(ancestor) { break; }
        match profile_marker(ancestor) {
            Some(false) => {},
            Some(true) => {
                if !path.strip_prefix(ancestor).is_ok_and(cache_relative) { return true; }
                // This is the nearest profile. Its enclosing app may have another profile;
                // structural browser-boundary checks above already cover the full path.
                break;
            },
            None => return true,
        }
    }
    false
}

/// Cheap per-entry backstop during deletion. This also catches session files that appear
/// after preflight; ordinary folders called Sessions or Preferences remain ordinary data.
pub(crate) fn protects_entry_name(path: &Path, directory: bool) -> bool {
    protected_boundary(path)
        || path.file_name().is_some_and(|name| sensitive_entry(name, directory))
        || path.ancestors().skip(1).any(|ancestor| ancestor.file_name().is_some_and(|name| sensitive_entry(name, true)))
}

/// A move must not create data in a browser profile either. Check prospective names and the
/// closest existing ancestor, without requiring an as-yet-uncreated destination to exist.
pub(crate) fn protects_destination(path: &Path, directory: bool) -> bool {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) || protected_boundary(path) {
        return true;
    }
    let mut first = true;
    for ancestor in path.ancestors() {
        if ancestor.file_name().is_some_and(|name| sensitive_entry(name, !first || directory)) { return true; }
        first = false;
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => return protects_path(ancestor),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(_) => return true,
        }
    }
    true
}

/// Preflight the actual deletion target again, without reading a cookie, token or stored page.
/// Unknown, linked, oversized or partly unreadable trees stay in place.
pub(crate) fn tree_is_safe(path: &Path) -> bool {
    if protects_path(path) { return false; }
    let mut stack = vec![path.to_path_buf()];
    let mut left = 2_000_000usize;
    while let Some(path) = stack.pop() {
        if left == 0 { return false; }
        left -= 1;
        let Some(meta) = plain_metadata(&path) else { return false };
        if path.file_name().is_some_and(|name| sensitive_entry(name, meta.is_dir())) { return false; }
        if !meta.is_dir() { continue; }
        if protected_boundary(&path) { return false; }
        let Ok(entries) = std::fs::read_dir(&path) else { return false };
        let mut preferences = false;
        let mut profile_companion = false;
        for entry in entries {
            let Ok(entry) = entry else { return false };
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            preferences |= name == "preferences";
            profile_companion |= matches!(name.as_str(), "code cache" | "network");
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

    fn write(path: &Path, bytes: usize, _days: u64) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    #[test]
    fn windows_alternate_names_cannot_hide_profile_markers_or_create_cache_permission() {
        for alias in ["COOKIES", "KEY3.DB", "KEY4.DB", "Network"] {
            assert!(marker_alias_blocks("custom-long-name", alias, false, true), "{alias}");
        }
        assert!(marker_alias_blocks("custom-long-name", "NETWORK", true, true));
        assert!(!marker_alias_blocks("custom-long-name", "COOKIES", true, true), "a legacy Cookies directory is not a database");
        assert!(marker_alias_blocks("custom-long-name", "COOKIES", false, false));
        assert!(!marker_alias_blocks("custom-long-name", "KEY3.DB", false, false), "Network's marker scope is unchanged");
        assert!(!marker_alias_blocks("custom-long-name", "DOCUME~1", true, true));
        assert!(!marker_alias_blocks("custom-long-name", "", false, true));
        assert!(!marker_alias_blocks("key3.db", "KEY3.DB", false, true), "same-name aliases add no hidden identity");
        assert!(!marker_alias_blocks("Cookies", "COOKIES", false, true));
    }

    #[test]
    fn shallow_listing_preserves_root_network_and_cookie_free_profile_markers() {
        let root = fixture("profile-marker-listing");
        for (i, marker) in ["Cookies", "Cookies-journal", "Cookies-wal", "Network/Cookies", "Network/Cookies-journal",
            "Network/Cookies-wal", "Login Data", "Local State", "cookies.sqlite", "cookies.sqlite-wal", "logins.json",
            "key3.db", "key4.db", "sessionstore.jsonlz4"].iter().enumerate() {
            let home = root.join(format!("profile-{i}"));
            write(&home.join(marker), 10, 0);
            assert_eq!(profile_marker(&home), Some(true), "{marker}");
        }
        for marker in ["Local Storage", "Session Storage", "IndexedDB", "Service Worker", "sessionstore-backups"] {
            let home = root.join(marker.replace(' ', "-"));
            std::fs::create_dir_all(home.join(marker)).unwrap();
            assert_eq!(profile_marker(&home), Some(true), "{marker}");
        }
        for companion in ["Network", "Code Cache"] {
            let home = root.join(format!("session-only-{}", companion.replace(' ', "-")));
            write(&home.join("Preferences"), 10, 0);
            write(&home.join("Sessions/Session_1"), 10, 0);
            std::fs::create_dir_all(home.join(companion)).unwrap();
            assert_eq!(profile_marker(&home), Some(true));
            assert!(protects_path(&home.join("Sessions/Session_1")));
        }
        let companion_file = root.join("companion-file");
        write(&companion_file.join("Preferences"), 10, 0);
        write(&companion_file.join("Code Cache"), 10, 0);
        assert_eq!(profile_marker(&companion_file), Some(true), "existing companion need not be a directory");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shallow_listing_does_not_create_a_nearer_permissive_profile_boundary() {
        let root = fixture("profile-marker-boundary");
        let outer = root.join("UnknownApp");
        write(&outer.join("Local State"), 10, 0);
        let inner = outer.join("ArbitraryChild");
        write(&inner.join("Sessions/Session_1"), 10, 0);
        write(&inner.join("Cookies-shm"), 10, 0);
        write(&inner.join("Cache/data"), 10, 0);
        assert_eq!(profile_marker(&inner), Some(false), "sensitive entries are not automatically profile boundaries");
        assert!(protects_path(&inner.join("Cache")), "the outer profile still protects this nested cache");
        write(&inner.join("Network/Login Data"), 10, 0);
        assert_eq!(profile_marker(&inner), Some(false), "Network keeps its original cookie-only marker scope");
        assert!(protects_path(&inner.join("Cache")));
        write(&inner.join("lOcAl StAtE"), 10, 0);
        assert_eq!(profile_marker(&inner), None, "ambiguous casing cannot add a permissive boundary");
        assert!(protects_path(&inner.join("Cache")));
        let ordinary = root.join("ordinary");
        write(&ordinary.join("Sessions/history.json"), 10, 0);
        std::fs::create_dir_all(ordinary.join("Cookies")).unwrap();
        assert_eq!(profile_marker(&ordinary), Some(false));
        assert!(tree_is_safe(&ordinary));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn incomplete_or_over_budget_listings_never_establish_a_profile_boundary() {
        let root = fixture("profile-marker-errors");
        write(&root.join("Cookies"), 10, 0);
        let mut left = 10;
        let entries = std::fs::read_dir(&root).unwrap().chain(std::iter::once(Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied, "synthetic enumeration failure after marker"))));
        assert_eq!(profile_marker_entries(entries, &mut left, true), None);
        let mut left = 0;
        assert_eq!(profile_marker_entries(std::fs::read_dir(&root).unwrap(), &mut left, true), None);
        let nested = root.join("nested");
        write(&nested.join("Network/Cookies"), 10, 0);
        let mut left = 1;
        assert_eq!(profile_marker_entries(std::fs::read_dir(&nested).unwrap(), &mut left, true), None,
            "the root and Network share the same entry budget");
        let malformed = root.join("malformed");
        write(&malformed.join("Preferences"), 10, 0);
        write(&malformed.join("Network"), 10, 0);
        assert_eq!(profile_marker(&malformed), None);
        assert_eq!(profile_marker(&root.join("missing")), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn legacy_cookies_junction_is_ignored_but_network_junction_is_not_followed() {
        let root = fixture("profile-marker-junctions");
        let home = root.join("home");
        let legacy = root.join("legacy");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&legacy).unwrap();
        junction::create(&legacy, home.join("Cookies")).unwrap();
        write(&home.join("Documents/notes.txt"), 10, 0);
        assert_eq!(profile_marker(&home), Some(false));
        assert!(!protects_path(&home.join("Documents/notes.txt")));
        let network = root.join("network");
        write(&network.join("Cookies"), 10, 0);
        junction::create(&network, home.join("Network")).unwrap();
        assert_eq!(profile_marker(&home), None);
        junction::delete(home.join("Network")).unwrap();
        junction::delete(home.join("Cookies")).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn full_profiles_and_site_data_stay_but_known_cache_children_are_allowed() {
        let root = fixture("browser-protection");
        for home in [root.join("Google/Chrome/User Data/Default"), root.join("EpicGamesLauncher/Saved/webcache_4430"), root.join("SomeApp/Partitions/site")] {
            for secret in ["Network/Cookies", "Login Data", "Local Storage/leveldb/token", "Session Storage/token", "IndexedDB/site/db", "Service Worker/CacheStorage/site/data", "Sessions/Session_1", "Preferences"] {
                write(&home.join(secret), 10, 100);
            }
            write(&home.join("Cache/Cache_Data/data_0"), 10, 100);
            assert!(protects_path(&home));
            assert!(protects_path(&home.join("Sessions/Session_1")));
            assert!(protects_path(&home.join("Preferences")));
            assert!(!protects_path(&home.join("Cache")));
            assert!(tree_is_safe(&home.join("Cache")));
            assert!(!tree_is_safe(&home));
        }
        assert!(!tree_is_safe(&root), "a broad parent contains browser data");
        // Coding conversations with a generic Sessions name aren't browser data by name alone.
        write(&root.join("ordinary-project/Sessions/history.json"), 10, 100);
        assert!(tree_is_safe(&root.join("ordinary-project")));
        let nested = root.join("Google/Chrome/User Data/Default/Partitions/site/Cache");
        write(&nested.join("data"), 10, 100);
        assert!(tree_is_safe(&nested));
        assert!(protects_destination(&root.join("Google/Chrome/User Data/Default/Sessions/new-session"), false));
        assert!(!protects_destination(&root.join("ordinary-project/new-folder/new-file"), false));
        std::fs::create_dir_all(root.join("ordinary-home/Cookies")).unwrap();
        let note = root.join("ordinary-home/Documents/notes.txt");
        write(&note, 10, 100);
        assert!(!protects_path(&note), "a legacy Cookies directory is not a cookie database");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn unexpected_session_data_inside_cache_and_unreadable_paths_are_rejected() {
        let root = fixture("browser-cache-protection");
        let cache = root.join("Google/Chrome/User Data/Default/Cache");
        write(&cache.join("Cookies"), 10, 100);
        assert!(!tree_is_safe(&cache));
        assert!(!tree_is_safe(&root.join("missing")));
        assert!(!tree_is_safe(Path::new("relative/Cache")));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn local_state_above_a_cookie_free_profile_does_not_block_only_its_cache_leaves() {
        let root = fixture("cookie-free-profile");
        let data = root.join("Google/Chrome/User Data");
        let home = data.join("Profile 2");
        write(&data.join("Local State"), 20, 0);
        for name in ["Cache", "Code Cache", "GPUCache"] {
            let cache = home.join(name);
            write(&cache.join("data"), 10, 0);
            assert!(!protects_path(&cache));
            assert!(tree_is_safe(&cache));
        }
        assert!(!home.join("Cookies").exists() && !home.join("Network/Cookies").exists());
        assert!(protects_path(&data.join("Local State")));
        assert!(protects_path(&data) && protects_path(&home));
        assert!(!tree_is_safe(&data) && !tree_is_safe(&home));
        let nested_auth = home.join("Cache/nested/Network/Cookies");
        write(&nested_auth, 10, 0);
        assert!(protects_path(&nested_auth));
        assert!(!tree_is_safe(&home.join("Cache")), "cache names cannot shelter authentication data");
        // An unrecognized app container with Local State still protects all non-cache data.
        let generic = root.join("UnknownApp");
        write(&generic.join("Local State"), 20, 0);
        write(&generic.join("ArbitraryChild/Cache/data"), 10, 0);
        assert!(protects_path(&generic.join("ArbitraryChild/Cache")));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
