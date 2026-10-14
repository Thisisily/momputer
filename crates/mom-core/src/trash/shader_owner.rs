//! Optional labels from cache metadata, never evidence that a cache is safe to delete.
//!
//! NVIDIA's on-disk format is undocumented. Only the header variant observed here is read;
//! unknown variants stay unnamed. D3DSCache's SQLite `app_id` record provides a full path.
//! Neither an executable basename nor a missing path proves an application was uninstalled.

use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Owner {
    pub name: String,
    pub detail: String,
}

/// Describes an individual NVIDIA file or one Windows DirectX cache directory/file.
/// This deliberately does not provide a deletion recommendation or installed-app verdict.
/// `budget` is shared across the scan and bounds attempted metadata reads, including failures.
pub(super) fn describe(path: &Path, budget: &mut u64) -> Option<Owner> {
    #[cfg(windows)]
    { windows::describe(path, budget) }
    #[cfg(not(windows))]
    { let _ = (path, budget); None }
}

/// The program each NVIDIA app ID in `dir` belongs to, from the files that say it. NVIDIA names
/// its cache files <driver part, 8 hex digits><app ID, 4><shader, 4>: an app keeps its ID across
/// driver versions, and some files (older drivers' among them) don't record the program. An ID
/// two files give different programs for is left out.
pub(super) fn nvidia_names(dir: &Path, budget: &mut u64) -> std::collections::HashMap<String, String> {
    #[cfg(windows)]
    { windows::nvidia_names(dir, budget) }
    #[cfg(not(windows))]
    { let _ = (dir, budget); Default::default() }
}

/// The app ID part of an NVIDIA cache file's name ("0002a9cb[c7c9]09d5.nvph").
pub(super) fn nvidia_app_id(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    (stem.len() == 16 && stem.bytes().all(|b| b.is_ascii_hexdigit())).then(|| stem[8..12].to_ascii_lowercase())
}

const NVIDIA_HEADER_BYTES: usize = 512;
const MAX_IDENTITY_BYTES: usize = 160;
const APP_ID_SCHEMA: &str = "CREATE TABLE app_id(id INTEGER PRIMARY KEY, exe_path TEXT, app_name TEXT, engine_name TEXT, app_version INTEGER, engine_version INTEGER, app_profile_version INTEGER)";

/// Observed NVPH layouts: magic, zero version/reserved fields and a size, then a 0/1 marker
/// and a NUL-terminated executable basename or Store package family, at byte 17 (current
/// drivers) or byte 13 (some older ones). Never search shader payloads.
fn nvidia_identity(bytes: &[u8]) -> Option<&str> {
    if bytes.len() < 19 || bytes.get(..8)? != b"nvph\0\0\0\0" {
        return None;
    }
    if bytes.get(12..16)? == [0, 0, 0, 0] && matches!(bytes[16], 0 | 1) {
        if let Some(name) = identity_at(bytes, 17) {
            return Some(name);
        }
    }
    if matches!(bytes[12], 0 | 1) {
        return identity_at(bytes, 13);
    }
    None
}

fn identity_at(bytes: &[u8], at: usize) -> Option<&str> {
    let field = bytes.get(at..bytes.len().min(at + MAX_IDENTITY_BYTES + 1))?;
    let end = field.iter().position(|&b| b == 0)?;
    let name = std::str::from_utf8(&field[..end]).ok()?;
    if name.is_empty() || name.len() > MAX_IDENTITY_BYTES || name.trim() != name
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"_. ()-".contains(&b))
    {
        return None;
    }
    let exe = name.to_ascii_lowercase().ends_with(".exe") && name.len() > 4;
    let package = name.rsplit_once('_').is_some_and(|(app, publisher)| {
        !app.is_empty() && app.bytes().all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
            && publisher.len() == 13 && publisher.bytes().all(|b| b.is_ascii_alphanumeric())
    });
    (exe || package).then_some(name)
}

/// Reject network/device/relative paths, ADS, traversal and Windows name normalization traps.
fn local_exe_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() < 7 || bytes.len() > 1024 || !bytes[0].is_ascii_alphabetic()
        || bytes[1..3] != *b":\\" || !text.to_ascii_lowercase().ends_with(".exe")
    {
        return false;
    }
    valid_components(&text[3..])
}

fn valid_components(tail: &str) -> bool {
    tail.split('\\').all(|part| {
        if part.is_empty() || matches!(part, "." | "..") || part.ends_with([' ', '.'])
            || part.chars().any(|c| c.is_control() || "/:<>\"|?*".contains(c))
        {
            return false;
        }
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            && !(stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    })
}

/// Read the specific metadata table, not arbitrary SQLite strings or executable-looking data.
fn directx_identity(connection: &rusqlite::Connection) -> Option<String> {
    let schema: String = connection.query_row(
        "SELECT sql FROM main.sqlite_schema WHERE type = 'table' AND name = 'app_id'",
        [], |row| row.get(0),
    ).ok()?;
    if schema != APP_ID_SCHEMA { return None; }
    let mut statement = connection.prepare(
        "SELECT id, CASE WHEN typeof(exe_path) = 'text' AND length(exe_path) BETWEEN 7 AND 1024 THEN exe_path ELSE NULL END FROM main.app_id LIMIT 2",
    ).ok()?;
    let mut rows = statement.query([]).ok()?;
    let row = rows.next().ok()??;
    if row.get::<_, i64>(0).ok()? != 0 { return None; }
    let path: String = row.get(1).ok()?;
    if rows.next().ok()?.is_some() || !local_exe_path(&path) { return None; }
    Some(path)
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::fs::{self, File, Metadata, OpenOptions};
    use std::io::{ErrorKind, Read};
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::path::PathBuf;
    use windows_sys::Win32::Storage::FileSystem::{
        GetDriveTypeW, FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAG_SEQUENTIAL_SCAN,
    };

    const MAX_DATABASE_BYTES: u64 = 8 << 20;
    const MAX_NVIDIA_FILES: usize = 20_000;
    const MAX_DIRECTORY_ENTRIES: usize = 8;
    const REPARSE_POINT: u32 = 0x400;

    pub(super) fn nvidia_names(dir: &Path, budget: &mut u64) -> std::collections::HashMap<String, String> {
        let mut names: std::collections::HashMap<String, Option<String>> = Default::default();
        let Ok(entries) = fs::read_dir(dir) else { return Default::default() };
        for entry in entries.flatten().take(MAX_NVIDIA_FILES) {
            let path = entry.path();
            if !nvidia_file(&path) || checked_metadata(&path).is_none_or(|m| !m.is_file()) {
                continue;
            }
            let Some(id) = nvidia_app_id(&path) else { continue };
            if reserve(budget, NVIDIA_HEADER_BYTES as u64).is_none() {
                break;
            }
            let mut bytes = Vec::with_capacity(NVIDIA_HEADER_BYTES);
            let Some(file) = open_cache(&path) else { continue };
            if file.take(NVIDIA_HEADER_BYTES as u64).read_to_end(&mut bytes).is_err() {
                continue;
            }
            let Some(name) = nvidia_identity(&bytes) else { continue };
            let slot = names.entry(id).or_insert_with(|| Some(name.to_owned()));
            if slot.as_deref().is_some_and(|prior| !prior.eq_ignore_ascii_case(name)) {
                *slot = None;
            }
        }
        names.into_iter().filter_map(|(id, name)| Some((id, name?))).collect()
    }

    pub(super) fn describe(path: &Path, budget: &mut u64) -> Option<Owner> {
        let metadata = checked_metadata(path)?;
        if metadata.is_file() && nvidia_file(path) {
            reserve(budget, NVIDIA_HEADER_BYTES as u64)?;
            let mut bytes = Vec::with_capacity(NVIDIA_HEADER_BYTES);
            open_cache(path)?.take(NVIDIA_HEADER_BYTES as u64).read_to_end(&mut bytes).ok()?;
            let name = nvidia_identity(&bytes)?.to_owned();
            let kind = if name.to_ascii_lowercase().ends_with(".exe") { "Executable name" } else { "Store package identity" };
            return Some(Owner { name, detail: format!("{kind} from cache; installation unknown") });
        }
        let files = if metadata.is_file() && directx_file(path) {
            vec![path.to_path_buf()]
        } else if metadata.is_dir() && directx_directory(path) {
            let mut files = Vec::new();
            for entry in fs::read_dir(path).ok()?.take(MAX_DIRECTORY_ENTRIES + 1) {
                let entry = entry.ok()?;
                if files.len() == MAX_DIRECTORY_ENTRIES || !directx_file(&entry.path()) {
                    return None;
                }
                files.push(entry.path());
            }
            if files.is_empty() { return None; }
            files
        } else {
            return None;
        };
        let mut identity: Option<String> = None;
        for file in files {
            let found = directx_file_identity(&file, budget)?;
            if identity.as_ref().is_some_and(|prior| !prior.eq_ignore_ascii_case(&found)) {
                return None;
            }
            identity = Some(found);
        }
        let executable = identity?;
        let name = executable.rsplit('\\').next()?.to_owned();
        let detail = match executable_status(Path::new(&executable)) {
            ExeStatus::Present => "Executable path from cache is present",
            ExeStatus::Missing => "Executable missing; app may have moved or updated",
            ExeStatus::Unavailable => "Executable path from cache; status unavailable",
            ExeStatus::Rejected => return None,
        };
        Some(Owner { name, detail: detail.to_owned() })
    }

    fn reserve(budget: &mut u64, bytes: u64) -> Option<()> {
        match budget.checked_sub(bytes) {
            Some(remaining) => { *budget = remaining; Some(()) }
            None => { *budget = 0; None }
        }
    }

    fn hex_name(path: &Path, len: usize) -> bool {
        path.file_stem().and_then(|n| n.to_str()).is_some_and(|n| {
            n.len() == len && n.bytes().all(|b| b.is_ascii_hexdigit())
        })
    }

    fn named(path: Option<&Path>, expected: &str) -> bool {
        path.and_then(Path::file_name).and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case(expected))
    }

    fn nvidia_file(path: &Path) -> bool {
        path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("nvph"))
            && hex_name(path, 16) && named(path.parent(), "DXCache")
            && named(path.parent().and_then(Path::parent), "NVIDIA")
    }

    fn directx_directory(path: &Path) -> bool {
        hex_name(path, 16) && named(path.parent(), "D3DSCache")
    }

    fn directx_file(path: &Path) -> bool {
        let guid = path.file_stem().and_then(|n| n.to_str()).unwrap_or("");
        path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("dxcache"))
            && guid.len() == 36 && guid.bytes().enumerate().all(|(i, b)| {
                if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() }
            }) && path.parent().is_some_and(directx_directory)
    }

    fn drive_root(path: &Path) -> Option<PathBuf> {
        let text = path.to_str()?;
        let bytes = text.as_bytes();
        if bytes.len() < 3 || bytes.len() > 4096 || !bytes[0].is_ascii_alphabetic()
            || bytes[1..3] != *b":\\" || (bytes.len() > 3 && !valid_components(&text[3..]))
        {
            return None;
        }
        let root = &text[..3];
        // Only removable, fixed or RAM disks; mapped network drives are excluded too.
        if !matches!(drive_type(root), 2 | 3 | 6) { return None; }
        Some(PathBuf::from(root))
    }

    fn drive_type(root: &str) -> u32 {
        let wide: Vec<u16> = root.encode_utf16().chain(Some(0)).collect();
        unsafe { GetDriveTypeW(wide.as_ptr()) }
    }

    fn reparse(metadata: &Metadata) -> bool {
        metadata.file_type().is_symlink() || metadata.file_attributes() & REPARSE_POINT != 0
    }

    fn checked_metadata(path: &Path) -> Option<Metadata> {
        drive_root(path)?;
        let mut result = None;
        for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
            let metadata = fs::symlink_metadata(ancestor).ok()?;
            if reparse(&metadata) || (ancestor != path && !metadata.is_dir()) { return None; }
            result = Some(metadata);
        }
        result
    }

    fn open_cache(path: &Path) -> Option<File> {
        let before = checked_metadata(path)?;
        if !before.is_file() { return None; }
        let file = OpenOptions::new().read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_SEQUENTIAL_SCAN)
            .open(path).ok()?;
        let opened = file.metadata().ok()?;
        if !opened.is_file() || reparse(&opened) || opened.len() != before.len() { return None; }
        Some(file)
    }

    fn directx_file_identity(path: &Path, budget: &mut u64) -> Option<String> {
        let before = checked_metadata(path)?;
        if !(100..=MAX_DATABASE_BYTES).contains(&before.len()) { return None; }
        // Charge the whole bounded database before opening it, including failed/unknown parses.
        // Most queries read only a few pages, so this is deliberately a conservative budget.
        reserve(budget, before.len())?;
        let mut file = open_cache(path)?;
        if file.metadata().ok()?.len() != before.len() { return None; }
        let mut magic = [0; 16];
        file.read_exact(&mut magic).ok()?;
        if &magic != b"SQLite format 3\0" { return None; }
        // Immutable reads never create journals or shared-memory files. Refuse live WAL/journal
        // state instead of reporting an old app_id from the main database alone.
        for suffix in ["-wal", "-journal"] {
            let mut sibling = path.as_os_str().to_os_string();
            sibling.push(suffix);
            match fs::symlink_metadata(&sibling) {
                Ok(metadata) if metadata.is_file() && metadata.len() == 0 && !reparse(&metadata) => {}
                Ok(_) => return None,
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(_) => return None,
            }
        }
        let connection = rusqlite::Connection::open_with_flags(
            immutable_uri(path)?,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ).ok()?;
        let identity = directx_identity(&connection)?;
        let after = checked_metadata(path)?;
        if before.len() != after.len() || before.modified().ok()? != after.modified().ok()? {
            return None;
        }
        Some(identity)
    }

    fn immutable_uri(path: &Path) -> Option<String> {
        let mut uri = String::from("file:///");
        for byte in path.to_str()?.replace('\\', "/").bytes() {
            if byte.is_ascii_alphanumeric() || b"/:._-~".contains(&byte) {
                uri.push(byte as char);
            } else {
                use std::fmt::Write;
                write!(&mut uri, "%{byte:02X}").ok()?;
            }
        }
        uri.push_str("?mode=ro&immutable=1");
        Some(uri)
    }

    #[derive(Debug, PartialEq, Eq)]
    enum ExeStatus { Present, Missing, Unavailable, Rejected }

    fn executable_status(path: &Path) -> ExeStatus {
        if drive_root(path).is_none() {
            // A mapped network drive has ordinary C:\\-style syntax; it is still nonlocal.
            return if path.to_str().and_then(|text| text.get(..3)).is_some_and(|root| drive_type(root) == 4) {
                ExeStatus::Rejected
            } else { ExeStatus::Unavailable };
        }
        let ancestors: Vec<_> = path.ancestors().collect();
        for ancestor in ancestors.iter().rev() {
            let metadata = match fs::symlink_metadata(ancestor) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    // All earlier ancestors, including the volume, were accessible directories.
                    return if ancestor.parent().is_some() { ExeStatus::Missing } else { ExeStatus::Unavailable };
                }
                Err(_) => return ExeStatus::Unavailable,
            };
            if reparse(&metadata) { return ExeStatus::Rejected; }
            if *ancestor == path {
                return if metadata.is_file() { ExeStatus::Present } else { ExeStatus::Unavailable };
            }
            if !metadata.is_dir() || fs::read_dir(ancestor).is_err() { return ExeStatus::Unavailable; }
        }
        ExeStatus::Unavailable
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::atomic::{AtomicU64, Ordering};

        struct Fixture(PathBuf);
        impl Fixture {
            fn new() -> Self {
                static NEXT: AtomicU64 = AtomicU64::new(0);
                let path = std::env::temp_dir().join(format!(
                    "mom-shader-owner-{}-{}-{}", std::process::id(),
                    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
                    NEXT.fetch_add(1, Ordering::Relaxed),
                ));
                fs::create_dir(&path).unwrap();
                Self(path)
            }
        }
        impl Drop for Fixture {
            fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
        }

        #[test]
        fn reports_present_and_missing_paths_without_installation_claims() {
            let fixture = Fixture::new();
            let exe = fixture.0.join("game.exe");
            fs::write(&exe, b"fixture").unwrap();
            assert_eq!(executable_status(&exe), ExeStatus::Present);
            fs::remove_file(&exe).unwrap();
            assert_eq!(executable_status(&exe), ExeStatus::Missing);
            assert_eq!(executable_status(&fixture.0.join("gone").join("game.exe")), ExeStatus::Missing);
            assert_eq!(executable_status(&fixture.0), ExeStatus::Unavailable);
        }

        #[test]
        fn refuses_junction_ancestors_for_cache_and_executable() {
            let fixture = Fixture::new();
            let actual = fixture.0.join("actual");
            fs::create_dir(&actual).unwrap();
            fs::write(actual.join("game.exe"), b"fixture").unwrap();
            let link = fixture.0.join("linked");
            junction::create(&actual, &link).unwrap();
            assert!(checked_metadata(&link.join("game.exe")).is_none());
            assert_eq!(executable_status(&link.join("game.exe")), ExeStatus::Rejected);
            junction::delete(&link).unwrap();
        }

        #[test]
        fn reads_directx_metadata_without_creating_sidecars_and_rejects_disagreement() {
            let fixture = Fixture::new();
            let directory = fixture.0.join("D3DSCache").join("0123456789abcdef");
            fs::create_dir_all(&directory).unwrap();
            let first = directory.join("F4EB2D6C-ED2B-4BDD-AD9D-F913287E6768.dxcache");
            let exe = fixture.0.join("Game & tool #1.exe");
            {
                let connection = rusqlite::Connection::open(&first).unwrap();
                connection.execute_batch(APP_ID_SCHEMA).unwrap();
                connection.execute("INSERT INTO app_id(id, exe_path) VALUES (0, ?1)", [exe.to_str().unwrap()]).unwrap();
            }
            let mut budget = 64 << 20;
            let owner = describe(&directory, &mut budget).unwrap();
            assert_eq!(owner.name, "Game & tool #1.exe");
            assert!(owner.detail.contains("missing"));
            assert_eq!(budget, (64 << 20) - fs::metadata(&first).unwrap().len());
            assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
            let mut too_small = fs::metadata(&first).unwrap().len() - 1;
            assert!(describe(&directory, &mut too_small).is_none());
            assert_eq!(too_small, 0);
            let second = directory.join("F4EB2D6C-ED2B-4BDD-AD9D-F913287E6769.dxcache");
            {
                let connection = rusqlite::Connection::open(second).unwrap();
                connection.execute_batch(APP_ID_SCHEMA).unwrap();
                connection.execute("INSERT INTO app_id(id, exe_path) VALUES (0, ?1)", [fixture.0.join("other.exe").to_str().unwrap()]).unwrap();
            }
            let mut budget = 64 << 20;
            assert!(describe(&directory, &mut budget).is_none());
        }

        #[test]
        fn budget_counts_failed_supported_probes_but_not_unrelated_files() {
            let fixture = Fixture::new();
            let directory = fixture.0.join("NVIDIA").join("DXCache");
            fs::create_dir_all(&directory).unwrap();
            let unrelated = directory.join("notes.txt");
            fs::write(&unrelated, b"not a cache").unwrap();
            let mut budget = 1024;
            assert!(describe(&unrelated, &mut budget).is_none());
            assert_eq!(budget, 1024);
            let invalid = directory.join("0123456789abcdef.nvph");
            fs::write(&invalid, b"unknown header").unwrap();
            assert!(describe(&invalid, &mut budget).is_none());
            assert_eq!(budget, 512);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nvph(identity: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; NVIDIA_HEADER_BYTES];
        bytes[..4].copy_from_slice(b"nvph");
        bytes[16] = 1;
        bytes[17..17 + identity.len()].copy_from_slice(identity);
        bytes
    }

    #[test]
    fn reads_only_known_nvph_identity_field() {
        assert_eq!(nvidia_identity(&nvph(b"example-win64-shipping.exe")), Some("example-win64-shipping.exe"));
        assert_eq!(nvidia_identity(&nvph(b"openai.codex_2p2nqsd0c76g0")), Some("openai.codex_2p2nqsd0c76g0"));
        let mut bytes = nvph(b"not-an-identity");
        bytes[256..264].copy_from_slice(b"game.exe");
        assert!(nvidia_identity(&bytes).is_none());
        for offset in [0, 4, 12, 16] {
            let mut bytes = nvph(b"game.exe");
            bytes[offset] = 42;
            assert!(nvidia_identity(&bytes).is_none());
        }
        assert!(nvidia_identity(&nvph(b"C:\\Game\\game.exe")).is_none());
        assert!(nvidia_identity(&nvph(b"game.exe")[..24]).is_none());
    }

    #[test]
    fn older_drivers_layout_names_too_and_app_ids_come_from_file_names() {
        // Name at byte 13, after a marker at byte 12 (no size field before it).
        let mut older = vec![0; NVIDIA_HEADER_BYTES];
        older[..4].copy_from_slice(b"nvph");
        older[8..12].copy_from_slice(&[0x35, 0x28, 0, 0]);
        older[12] = 1;
        older[13..13 + 7].copy_from_slice(b"vlc.exe");
        assert_eq!(nvidia_identity(&older), Some("vlc.exe"));
        // A file that only keeps a number where the name would be stays unnamed.
        let mut hashed = vec![0; NVIDIA_HEADER_BYTES];
        hashed[..4].copy_from_slice(b"nvph");
        hashed[8..12].copy_from_slice(&[0x80, 0x9b, 0xad, 0x19]);
        hashed[16] = 1;
        hashed[20..24].copy_from_slice(&[0x0a, 0x7b, 0x3e, 0x90]);
        assert_eq!(nvidia_identity(&hashed), None);
        assert_eq!(nvidia_app_id(Path::new(r"C:\x\NVIDIA\DXCache\0002a9cbC7C909d5.nvph")).as_deref(), Some("c7c9"));
        assert_eq!(nvidia_app_id(Path::new("short.nvph")), None);
        assert_eq!(nvidia_app_id(Path::new("0002a9cbzzzz09d5.nvph")), None);
    }

    #[test]
    fn rejects_nonlocal_and_ambiguous_windows_paths() {
        assert!(local_exe_path(r"C:\Games\Example\game.exe"));
        for path in [r"\\server\game.exe", r"\\?\C:\game.exe", r"C:game.exe", r"C:\..\game.exe",
            r"C:\game.exe:stream.exe", r"C:\folder.\game.exe", r"C:\CON\game.exe", r"C:\\game.exe"]
        {
            assert!(!local_exe_path(path), "{path}");
        }
    }

    #[test]
    fn requires_exact_app_metadata_schema_and_a_single_valid_record() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.execute_batch(APP_ID_SCHEMA).unwrap();
        assert!(directx_identity(&connection).is_none());
        connection.execute("INSERT INTO app_id(id, exe_path) VALUES (0, ?1)", [r"C:\Games\game.exe"]).unwrap();
        assert_eq!(directx_identity(&connection).as_deref(), Some(r"C:\Games\game.exe"));
        connection.execute("INSERT INTO app_id(id, exe_path) VALUES (1, ?1)", [r"C:\Other\game.exe"]).unwrap();
        assert!(directx_identity(&connection).is_none());
        let unknown = rusqlite::Connection::open_in_memory().unwrap();
        unknown.execute_batch("CREATE TABLE app_id(exe_path TEXT); INSERT INTO app_id VALUES ('C:\\game.exe');").unwrap();
        assert!(directx_identity(&unknown).is_none());
    }
}
