//! An independent, deliberately narrow shader cleanup capability. General cleanup stays held.
//! Only explicit files in a short-lived, process-owned snapshot can be deleted. The wire view
//! is not a capability: identities, roots and ancestor identities never come back from the UI.
//! Windows only, fixed NTFS volumes, no elevation, permission changes, shell or recursive delete.

use crate::paths::Paths;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const MAX_ENTRIES: usize = 40_000;
const MAX_ROOTS: usize = 256;
const MAX_DEPTH: usize = 16;
const SCAN_TIME: Duration = Duration::from_secs(20);
const SNAPSHOT_LIFETIME: Duration = Duration::from_secs(600);
const CLEAN_TIME: Duration = Duration::from_secs(60);
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanFile {
    pub path: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanView {
    pub token: String,
    pub files: Vec<ScanFile>,
    /// Ineligible files or roots; not a count of all files in an unreadable root.
    pub skipped: u64,
}

/// Keep this only in native process memory. Do not add Deserialize or a disk restore path.
pub struct Snapshot {
    token: String,
    created: Instant,
    consumed: bool,
    entries: HashMap<String, Entry>,
    skipped: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity { volume: u32, index: u64 }

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    identity: Identity,
    bytes: u64,
    created: i64,
    written: i64,
    changed: i64,
}

struct Entry {
    path: PathBuf,
    root: PathBuf,
    parents: Vec<Identity>,
    stamp: Stamp,
}

impl Snapshot {
    pub fn view(&self) -> ScanView {
        let mut files: Vec<_> = if self.consumed { vec![] } else {
            self.entries.values().map(|e| ScanFile {
                path: e.path.to_string_lossy().into_owned(), bytes: e.stamp.bytes,
            }).collect()
        };
        files.sort_by(|a, b| a.path.cmp(&b.path));
        ScanView { token: self.token.clone(), files, skipped: self.skipped }
    }
}

/// A scan is a preview, never an automatic cleanup recommendation. Ownership labels and a
/// game's installation status do not authorize deletion. Kept paths are excluded here and
/// checked again using the latest settings when cleaning.
pub fn scan(paths: &Paths, keep: &[String]) -> Result<Snapshot, String> {
    platform::require_standard_user()?;
    scan_inner(paths, keep)
}

fn scan_inner(paths: &Paths, keep: &[String]) -> Result<Snapshot, String> {
    let mut snapshot = Snapshot {
        token: format!("{}-{}", std::process::id(), NEXT_TOKEN.fetch_add(1, Ordering::Relaxed)),
        created: Instant::now(), consumed: false, entries: HashMap::new(), skipped: 0,
    };
    let mut budget = MAX_ENTRIES;
    for root in roots(paths).into_iter().take(MAX_ROOTS) {
        if budget == 0 || snapshot.created.elapsed() > SCAN_TIME { snapshot.skipped += 1; break; }
        if super::is_kept(&root, keep) { snapshot.skipped += 1; continue; }
        let mut entries = Vec::new();
        // Incomplete directory enumeration or a protected marker invalidates this whole root.
        match platform::collect(&root, keep, &mut budget, snapshot.created, &mut entries) {
            Ok(skipped) => {
                snapshot.skipped += skipped;
                for entry in entries { snapshot.entries.insert(key(&entry.path), entry); }
            }
            Err(()) => snapshot.skipped += 1,
        }
    }
    Ok(snapshot)
}

/// Only exact frozen leaf paths are accepted; an empty selection never means "everything".
/// Every attempt with the current token consumes it, including invalid/empty selections and
/// partial failures. Repeating a click therefore requires a fresh preview and selection.
pub fn clean(snapshot: &mut Snapshot, token: &str, selected_paths: &[String], keep: &[String])
    -> Result<super::CleanReport, String>
{
    consume(snapshot, token)?;
    platform::require_standard_user()?;
    clean_selected(snapshot, selected_paths, keep)
}

fn consume(snapshot: &mut Snapshot, token: &str) -> Result<(), String> {
    if snapshot.consumed || token != snapshot.token { return Err("Scan shader caches again before cleaning.".into()); }
    snapshot.consumed = true;
    if snapshot.created.elapsed() > SNAPSHOT_LIFETIME { return Err("This shader-cache preview expired. Scan again.".into()); }
    Ok(())
}

fn clean_selected(snapshot: &Snapshot, selected_paths: &[String], keep: &[String]) -> Result<super::CleanReport, String> {
    if selected_paths.is_empty() { return Err("Select shader-cache files to delete.".into()); }
    if selected_paths.len() > MAX_ENTRIES { return Err("Too many shader-cache files were selected.".into()); }
    let mut selected = HashSet::new();
    // Validate the entire request before the first deletion. No folder expansion or fallback.
    for text in selected_paths {
        let path = Path::new(text);
        if !safe_path(path) || !snapshot.entries.contains_key(&key(path)) {
            return Err("The selection no longer matches this shader-cache preview. Scan again.".into());
        }
        selected.insert(key(path));
    }
    let mut report = super::CleanReport { id: "shader-cache".into(), ..Default::default() };
    let start = Instant::now();
    let mut budget = MAX_ENTRIES;
    let mut checked_roots = HashMap::new();
    // Recheck directory safety, including newly appeared profile/session markers. This may
    // remove eligibility, but its fresh files are deliberately never added to the selection.
    for selected_key in &selected {
        let entry = &snapshot.entries[selected_key];
        checked_roots.entry(key(&entry.root)).or_insert_with(|| {
            let mut ignored = Vec::new();
            platform::collect(&entry.root, keep, &mut budget, start, &mut ignored).is_ok()
        });
    }
    for selected_key in selected {
        let entry = &snapshot.entries[&selected_key];
        if start.elapsed() > CLEAN_TIME || !checked_roots[&key(&entry.root)]
            || super::is_kept(&entry.path, keep) || !platform::delete(entry) {
            report.skipped += 1;
        } else {
            report.deleted += 1;
            report.freed = report.freed.saturating_add(entry.stamp.bytes);
        }
    }
    report.message = format!("Deleted {} shader-cache files; skipped {}. Games may rebuild deleted shaders.", report.deleted, report.skipped);
    Ok(report)
}

fn roots(paths: &Paths) -> Vec<PathBuf> {
    let local = &paths.local_appdata;
    let mut bases = vec![local.join("NVIDIA")];
    if let Some(parent) = local.parent() {
        bases.push(parent.join("LocalLow").join("NVIDIA"));
        bases.push(parent.join("LocalLow").join("NVIDIA").join("PerDriverVersion"));
    }
    let mut roots = Vec::new();
    for base in bases {
        for name in ["DXCache", "GLCache", "OptixCache"] { roots.push(base.join(name)); }
    }
    roots.push(paths.appdata.join("NVIDIA").join("ComputeCache"));
    roots.extend(super::steam_shaders::discover(paths).into_iter().map(|c| c.path));
    // Discovery may use either Windows separator; freeze one ordinary spelling internally.
    for path in &mut roots { *path = PathBuf::from(path.to_string_lossy().replace('/', "\\")); }
    let mut seen = HashSet::new();
    roots.retain(|p| safe_path(p) && seen.insert(key(p)));
    roots
}

// Keep exact case in capabilities: NTFS can contain distinct A/a names in a case-sensitive
// directory. A UI selection must never alias the unticked file's snapshot entry.
fn key(path: &Path) -> String { path.to_string_lossy().replace('/', "\\") }

/// Reject aliases Windows might normalize differently from the preview, including ADS,
/// drive-relative paths, UNC/device namespaces, dot segments and DOS device names.
fn safe_path(path: &Path) -> bool {
    let Some(text) = path.to_str() else { return false };
    let bytes = text.as_bytes();
    if bytes.len() < 4 || bytes.len() > 30_000 || !bytes[0].is_ascii_alphabetic()
        || bytes[1..3] != *b":\\" { return false; }
    text[3..].split('\\').all(safe_component)
}

fn safe_component(part: &str) -> bool {
    if part.is_empty() || matches!(part, "." | "..") || part.ends_with([' ', '.'])
        || part.chars().any(|c| c.is_control() || "/:<>\"|?*".contains(c)) { return false; }
    let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
    !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        && !(stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

/// A browser's marks: a root holding one isn't only a shader cache (a profile was moved into
/// it, say), so nothing in it is touched. Only names that show a browser is here, never a list
/// of what a browser keeps, and nothing is opened to decide.
fn browser_mark(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "local state" | "preferences" | "secure preferences" | "prefs.js"
        | "profiles.ini" | "local storage" | "session storage" | "indexeddb" | "service worker" | "webstorage"
        | "transportsecurity" | "network persistent state")
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    pub(super) fn require_standard_user() -> Result<(), String> { Err("Shader-cache cleanup is available only on Windows.".into()) }
    pub(super) fn collect(_: &Path, _: &[String], _: &mut usize, _: Instant, _: &mut Vec<Entry>) -> Result<u64, ()> { Err(()) }
    pub(super) fn delete(_: &Entry) -> bool { false }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::fs::{self, File, OpenOptions};
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use std::time::{SystemTime, UNIX_EPOCH};
    use windows_sys::Win32::Storage::FileSystem::*;

    // Reparse, offline, encrypted, recall-on-open and recall-on-data-access. Read-only is
    // additionally rejected on files; the directory bit is often just a Windows UI setting.
    const UNSAFE_ATTRIBUTES: u32 = 0x400 | 0x1000 | 0x4000 | 0x40000 | 0x400000;

    pub(super) fn require_standard_user() -> Result<(), String> {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        let mut token = std::ptr::null_mut();
        // SAFETY: current process pseudo-handle; the out handle is closed below.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err("Could not verify that mom is running without administrator rights.".into());
        }
        let mut value = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut size = 0;
        // SAFETY: the output buffer and its declared size agree; token remains open.
        let ok = unsafe { GetTokenInformation(token, TokenElevation, (&mut value as *mut TOKEN_ELEVATION).cast(), std::mem::size_of_val(&value) as u32, &mut size) };
        // SAFETY: owned handle from OpenProcessToken.
        unsafe { CloseHandle(token) };
        if ok == 0 || size as usize != std::mem::size_of_val(&value) {
            Err("Could not verify that mom is running without administrator rights.".into())
        } else if value.TokenIsElevated != 0 {
            Err("Close the administrator copy of mom and open it normally to clean shader caches.".into())
        } else { Ok(()) }
    }

    fn open(path: &Path, directory: bool, deleting: bool) -> Result<File, ()> {
        // Attribute-only handles do not establish share reservations. FILE_READ_DATA is also
        // FILE_LIST_DIRECTORY: requesting it makes the no-write/no-delete sharing effective.
        // No file content is read by this module, and offline files must not be recalled.
        OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES | FILE_READ_DATA | if deleting { DELETE } else { 0 })
            // Ancestors deny writers and renames; a leaf denies all data/rename sharing.
            .share_mode(if directory { FILE_SHARE_READ } else { 0 })
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_NO_RECALL)
            .open(path).map_err(|_| ())
    }

    fn information(file: &File, path: &Path, directory: bool) -> Result<BY_HANDLE_FILE_INFORMATION, ()> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        let mut final_path = vec![0u16; 32_768];
        // SAFETY: both writable outputs are correctly sized and file owns a live handle.
        let (ok, len) = unsafe {
            (GetFileInformationByHandle(file.as_raw_handle(), &mut info),
             GetFinalPathNameByHandleW(file.as_raw_handle(), final_path.as_mut_ptr(), final_path.len() as u32, 0))
        };
        if ok == 0 || len == 0 || len as usize >= final_path.len()
            || info.dwFileAttributes & UNSAFE_ATTRIBUTES != 0
            || (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0) != directory { return Err(()); }
        let actual = String::from_utf16(&final_path[..len as usize]).map_err(|_| ())?;
        let actual = actual.strip_prefix(r"\\?\").unwrap_or(&actual);
        // A final name may canonicalize ASCII drive/directory casing. This comparison is
        // solely a handle-path check; capability lookup above remains exact and non-aliasing.
        if !key(Path::new(actual)).trim_end_matches('\\').eq_ignore_ascii_case(key(path).trim_end_matches('\\')) { return Err(()); }
        Ok(info)
    }

    fn identity(info: &BY_HANDLE_FILE_INFORMATION) -> Result<Identity, ()> {
        let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
        if index == 0 { return Err(()); }
        Ok(Identity { volume: info.dwVolumeSerialNumber, index })
    }

    /// File IDs/streams and deletion semantics here are intentionally limited to local NTFS.
    fn supported_volume(file: &File, path: &Path) -> bool {
        let Some(text) = path.to_str() else { return false };
        let drive: Vec<_> = text[..3].encode_utf16().chain(Some(0)).collect();
        let mut name = [0u16; 32];
        // SAFETY: valid NUL-terminated drive string and sized filesystem-name output.
        let ok = unsafe { GetDriveTypeW(drive.as_ptr()) == 3
            && GetVolumeInformationByHandleW(file.as_raw_handle(), std::ptr::null_mut(), 0,
                std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), name.as_mut_ptr(), name.len() as u32) != 0 };
        ok && String::from_utf16_lossy(&name[..name.iter().position(|c| *c == 0).unwrap_or(name.len())]) == "NTFS"
    }

    fn pin_directories(path: &Path) -> Result<(Vec<File>, Vec<Identity>), ()> {
        let mut handles = Vec::new();
        let mut identities = Vec::new();
        for part in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
            let handle = open(part, true, false)?;
            identities.push(identity(&information(&handle, part, true)?)?);
            if handles.is_empty() && !supported_volume(&handle, path) { return Err(()); }
            handles.push(handle);
        }
        Ok((handles, identities))
    }

    fn stamp(file: &File, path: &Path) -> Result<Stamp, ()> {
        let info = information(file, path, false)?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_READONLY != 0 || info.nNumberOfLinks != 1 { return Err(()); }
        let mut basic = FILE_BASIC_INFO::default();
        let mut streams = [0u64; 64];
        // SAFETY: aligned output buffers of the exact declared sizes; handle stays open.
        let ok = unsafe {
            GetFileInformationByHandleEx(file.as_raw_handle(), FileBasicInfo, (&mut basic as *mut FILE_BASIC_INFO).cast(), std::mem::size_of_val(&basic) as u32) != 0
                && GetFileInformationByHandleEx(file.as_raw_handle(), FileStreamInfo, streams.as_mut_ptr().cast(), std::mem::size_of_val(&streams) as u32) != 0
        };
        if !ok { return Err(()); }
        let bytes: Vec<_> = streams.into_iter().flat_map(u64::to_ne_bytes).collect();
        let next = u32::from_ne_bytes(bytes[0..4].try_into().map_err(|_| ())?);
        let len = u32::from_ne_bytes(bytes[4..8].try_into().map_err(|_| ())?) as usize;
        let expected: Vec<_> = "::$DATA".encode_utf16().flat_map(u16::to_ne_bytes).collect();
        if next != 0 || len != expected.len() || bytes.get(24..24 + len) != Some(expected.as_slice()) { return Err(()); }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| ())?.as_nanos() / 100 + 116_444_736_000_000_000;
        if [basic.CreationTime, basic.LastWriteTime, basic.ChangeTime].iter().any(|t| *t <= 0 || *t as u128 > now) { return Err(()); }
        Ok(Stamp { identity: identity(&info)?, bytes: (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow),
            created: basic.CreationTime, written: basic.LastWriteTime, changed: basic.ChangeTime })
    }

    pub(super) fn collect(root: &Path, keep: &[String], budget: &mut usize, start: Instant, entries: &mut Vec<Entry>) -> Result<u64, ()> {
        if !safe_path(root) { return Err(()); }
        // Keep ancestors pinned for the entire traversal, including path-based ReadDir calls.
        let (_ancestors, _) = pin_directories(root)?;
        visit(root, root, keep, budget, start, entries, 0)
    }

    fn visit(root: &Path, dir: &Path, keep: &[String], budget: &mut usize, start: Instant, entries: &mut Vec<Entry>, depth: usize) -> Result<u64, ()> {
        if depth > MAX_DEPTH || start.elapsed() > SCAN_TIME { return Err(()); }
        let handle = open(dir, true, false)?;
        information(&handle, dir, true)?;
        let mut skipped = 0;
        for child in fs::read_dir(dir).map_err(|_| ())? {
            if *budget == 0 || start.elapsed() > SCAN_TIME { return Err(()); }
            *budget -= 1;
            let child = child.map_err(|_| ())?;
            let path = child.path();
            let name = child.file_name();
            let name = name.to_str().ok_or(())?;
            if !safe_component(name) || browser_mark(name) { return Err(()); }
            let kind = child.file_type().map_err(|_| ())?;
            if kind.is_symlink() { return Err(()); }
            if kind.is_dir() {
                skipped += visit(root, &path, keep, budget, start, entries, depth + 1)?;
            } else if kind.is_file() && !super::super::is_kept(&path, keep) {
                let candidate = (|| {
                    let (_parents, parents) = pin_directories(path.parent().ok_or(())?)?;
                    let file = open(&path, false, false)?;
                    let stamp = stamp(&file, &path)?;
                    Ok::<_, ()>(Entry { path: path.clone(), root: root.to_path_buf(), parents, stamp })
                })();
                match candidate { Ok(entry) => entries.push(entry), Err(()) => skipped += 1 }
            } else { skipped += 1; }
        }
        Ok(skipped)
    }

    pub(super) fn delete(entry: &Entry) -> bool {
        let action = || -> Result<(), ()> {
            if !safe_path(&entry.path) || !entry.path.starts_with(&entry.root) || entry.path == entry.root { return Err(()); }
            let (_parents, identities) = pin_directories(entry.path.parent().ok_or(())?)?;
            if identities != entry.parents { return Err(()); }
            let file = open(&entry.path, false, true)?;
            if stamp(&file, &entry.path)? != entry.stamp { return Err(()); }
            let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
            // SAFETY: DELETE access was requested on this same verified exclusive handle.
            // Plain FileDispositionInfo honors read-only/sharing restrictions; no POSIX or
            // IGNORE_READONLY flags, attribute changes, shell call or path-based retry.
            if unsafe { SetFileInformationByHandle(file.as_raw_handle(), FileDispositionInfo,
                (&disposition as *const FILE_DISPOSITION_INFO).cast(), std::mem::size_of_val(&disposition) as u32) } == 0 { return Err(()); }
            drop(file); // Complete delete disposition before reporting success.
            Ok(())
        };
        action().is_ok()
    }

    #[cfg(test)]
    pub(super) fn assert_parent_pinning(path: &Path) {
        let moved = path.with_file_name("renamed-while-pinned");
        let (handles, _) = pin_directories(path).unwrap();
        assert!(fs::rename(path, &moved).is_err(), "An ancestor must not be replaceable while pinned");
        drop(handles);
        fs::rename(path, &moved).unwrap();
        fs::rename(&moved, path).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_reject_aliases_and_streams() {
        for path in [r"C:\cache\a.nvph", r"D:\SteamLibrary\steamapps\shadercache\123\a.foz"] { assert!(safe_path(Path::new(path))); }
        for path in [r"C:cache\a", r"\\server\share\a", r"\\?\C:\cache\a", r"C:\cache\..\a", r"C:\cache\a:secret", r"C:\cache\NUL.txt", r"C:\cache\a.", r"C:\cache\a ", "C:/cache/a"] { assert!(!safe_path(Path::new(path)), "{path}"); }
    }

    #[test]
    fn browser_marks_are_not_shader_entries() {
        for name in ["Local State", "Preferences", "prefs.js", "profiles.ini", "IndexedDB", "Local Storage", "TransportSecurity"] { assert!(browser_mark(name), "{name}"); }
        assert!(!browser_mark("abc123.nvph"));
    }

    #[test]
    fn snapshot_keys_do_not_merge_case_distinct_selections() {
        let paths = [r"C:\cache\A.nvph", r"C:\cache\a.nvph", r"C:\cache\İ.nvph", "C:\\cache\\i\u{307}.nvph"];
        let keys: HashSet<_> = paths.iter().map(|p| key(Path::new(p))).collect();
        assert_eq!(keys.len(), paths.len());
    }

    #[cfg(windows)]
    mod windows {
        use super::*;
        use std::fs;
        use std::os::windows::fs::OpenOptionsExt;
        struct Fixture { root: PathBuf, paths: Paths }
        impl Fixture {
            fn new() -> Self {
                let root = std::env::temp_dir().join(format!("mom-shader-clean-{}-{}", std::process::id(), NEXT_TOKEN.fetch_add(1, Ordering::Relaxed)));
                fs::create_dir_all(&root).unwrap();
                // GitHub's TEMP uses RUNNER~1. Exercise the ordinary long-path contract;
                // production deliberately rejects aliases instead of trusting their spelling.
                let canonical = fs::canonicalize(&root).unwrap().to_string_lossy().into_owned();
                let root = PathBuf::from(canonical.strip_prefix(r"\\?\").unwrap_or(&canonical));
                Self { paths: Paths::under(&root), root }
            }
            fn cache(&self) -> PathBuf { self.paths.local_appdata.join("NVIDIA").join("DXCache") }
            fn file(&self, name: &str) -> PathBuf {
                let p = self.cache().join(name);
                fs::create_dir_all(p.parent().unwrap()).unwrap();
                fs::write(&p, b"synthetic shader cache").unwrap();
                p
            }
            fn scan(&self) -> Snapshot { self.scan_keep(&[]) }
            fn scan_keep(&self, keep: &[String]) -> Snapshot {
                // CI runners can be administrators. Only these synthetic fixtures bypass the
                // public entrypoint's elevation refusal; production has no such switch.
                assert!(self.paths.local_appdata.starts_with(&self.root));
                assert!(self.paths.program_files_x86.starts_with(&self.root));
                scan_inner(&self.paths, keep).unwrap()
            }
            fn clean(&self, snapshot: &mut Snapshot, token: &str, selected: &[String], keep: &[String]) -> Result<super::super::super::CleanReport, String> {
                assert!(snapshot.entries.values().all(|e| e.path.starts_with(&self.root) && e.root.starts_with(&self.root)));
                consume(snapshot, token)?;
                clean_selected(snapshot, selected, keep)
            }
        }
        impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.root); } }
        fn text(path: &Path) -> String { path.to_string_lossy().replace('/', "\\") }

        #[test]
        fn frozen_selection_deletes_only_selected_leaf_and_cannot_repeat() {
            let f = Fixture::new(); let a = f.file("a.nvph"); let b = f.file("b.nvph");
            let mut s = f.scan(); assert_eq!(s.view().files.len(), 2);
            let new = f.file("new.nvph"); let token = s.view().token;
            let r = f.clean(&mut s, &token, &[text(&a), text(&a)], &[]).unwrap();
            assert_eq!(r.deleted, 1); assert!(!a.exists()); assert!(b.exists() && new.exists() && f.cache().is_dir());
            assert!(f.clean(&mut s, &token, &[text(&b)], &[]).is_err()); assert!(b.exists());
        }

        #[test]
        fn empty_folder_unscanned_and_wrong_tokens_never_expand_selection() {
            let f = Fixture::new(); let a = f.file("a.nvph");
            let mut s = f.scan(); let token = s.view().token;
            assert!(f.clean(&mut s, "wrong", &[text(&a)], &[]).is_err());
            assert!(f.clean(&mut s, &token, &[], &[]).is_err()); assert!(a.exists());
            for path in [f.cache(), f.root.join("outside.nvph"), f.file("new.nvph")] {
                let mut s = f.scan(); s.entries.remove(&key(&path)); let token = s.view().token;
                assert!(f.clean(&mut s, &token, &[text(&a), text(&path)], &[]).is_err()); assert!(a.exists());
            }
        }

        #[test]
        fn changed_replaced_kept_and_in_use_files_stay() {
            let f = Fixture::new(); let a = f.file("a.nvph"); let b = f.file("b.nvph"); let c = f.file("c.nvph"); let d = f.file("d.nvph");
            let mut s = f.scan(); assert_eq!(s.view().files.len(), 4); let token = s.view().token;
            fs::write(&a, b"changed size and contents").unwrap();
            fs::rename(&b, f.cache().join("old.nvph")).unwrap(); fs::write(&b, b"replacement").unwrap();
            let locked = fs::OpenOptions::new().read(true).share_mode(1).open(&c).unwrap();
            let r = f.clean(&mut s, &token, &[text(&a), text(&b), text(&c), text(&d)], &[text(&d)]).unwrap();
            assert_eq!(r.deleted, 0); assert_eq!(r.skipped, 4); assert!(a.exists() && b.exists() && c.exists() && d.exists()); drop(locked);
        }

        #[test]
        fn hardlinks_streams_readonly_and_browser_profile_markers_stay() {
            let f = Fixture::new(); let a = f.file("a.nvph"); fs::hard_link(&a, f.root.join("link.nvph")).unwrap();
            let b = f.file("b.nvph"); fs::write(format!("{}:private", b.display()), b"private stream").unwrap();
            let c = f.file("c.nvph"); let mut permissions = fs::metadata(&c).unwrap().permissions(); permissions.set_readonly(true); fs::set_permissions(&c, permissions).unwrap();
            assert!(f.scan().view().files.is_empty());
            // This fixture resets its own synthetic read-only attribute solely for teardown.
            let mut permissions = fs::metadata(&c).unwrap().permissions(); permissions.set_readonly(false); fs::set_permissions(&c, permissions).unwrap();
            f.file("profile/Local State"); assert!(f.scan().view().files.is_empty());
            assert!(a.exists() && b.exists() && c.exists());
        }

        #[test]
        fn replaced_ancestor_expired_snapshot_and_incomplete_tree_stay() {
            let f = Fixture::new(); let a = f.file("a.nvph"); let mut s = f.scan(); let token = s.view().token;
            let moved = f.cache().with_file_name("original"); fs::rename(f.cache(), &moved).unwrap(); fs::create_dir_all(f.cache()).unwrap();
            fs::rename(moved.join("a.nvph"), &a).unwrap();
            assert_eq!(f.clean(&mut s, &token, &[text(&a)], &[]).unwrap().deleted, 0); assert!(a.exists());
            let mut s = f.scan(); s.created = Instant::now() - SNAPSHOT_LIFETIME - Duration::from_secs(1); let token = s.view().token;
            assert!(f.clean(&mut s, &token, &[text(&a)], &[]).is_err()); assert!(a.exists());
            let deep = (0..MAX_DEPTH + 2).fold(String::new(), |mut p, _| { p.push_str("dir/"); p }); f.file(&format!("{deep}deep.nvph"));
            assert!(f.scan().view().files.is_empty());
        }

        #[test]
        fn steam_library_cache_only_never_installation_files() {
            let f = Fixture::new(); let apps = f.paths.program_files_x86.join("Steam/steamapps");
            let shader = apps.join("shadercache/123/sub/cache.foz"); let game = apps.join("common/Game/save.dat");
            for p in [&shader, &game] { fs::create_dir_all(p.parent().unwrap()).unwrap(); fs::write(p, b"synthetic fixture").unwrap(); }
            let mut s = f.scan(); assert_eq!(s.view().files.iter().map(|v| v.path.clone()).collect::<Vec<_>>(), vec![text(&shader)]); let token = s.view().token;
            assert_eq!(f.clean(&mut s, &token, &[text(&shader)], &[]).unwrap().deleted, 1); assert!(game.exists());
        }

        #[test]
        fn junctions_cannot_redirect_snapshot_or_expand_into_personal_data() {
            let f = Fixture::new(); let a = f.file("a.nvph"); let outside = f.root.join("personal");
            fs::create_dir_all(&outside).unwrap(); let important = outside.join("important.pdf"); fs::write(&important, b"keep this").unwrap();
            junction::create(&outside, f.cache().join("linked")).unwrap();
            assert!(f.scan().view().files.is_empty()); assert!(important.exists());
            junction::delete(f.cache().join("linked")).unwrap();
            let mut s = f.scan(); let token = s.view().token;
            let original = f.cache().with_file_name("original"); fs::rename(f.cache(), &original).unwrap();
            junction::create(&outside, f.cache()).unwrap();
            assert_eq!(f.clean(&mut s, &token, &[text(&a)], &[]).unwrap().deleted, 0);
            assert!(important.exists() && original.join("a.nvph").exists());
            junction::delete(f.cache()).unwrap();
        }

        #[test]
        fn valid_leaf_can_delete_while_new_readonly_leaf_and_kept_files_stay() {
            let f = Fixture::new(); let a = f.file("a.nvph"); let b = f.file("b.nvph"); let kept = f.file("kept.nvph");
            let mut s = f.scan_keep(&[text(&kept)]); assert_eq!(s.view().files.len(), 2); let token = s.view().token;
            let mut permissions = fs::metadata(&b).unwrap().permissions(); permissions.set_readonly(true); fs::set_permissions(&b, permissions).unwrap();
            let r = f.clean(&mut s, &token, &[text(&a), text(&b)], &[text(&kept)]).unwrap();
            assert_eq!((r.deleted, r.skipped), (1, 1)); assert!(!a.exists() && b.exists() && kept.exists());
            let mut permissions = fs::metadata(&b).unwrap().permissions(); permissions.set_readonly(false); fs::set_permissions(&b, permissions).unwrap();
        }

        #[test]
        fn new_session_marker_and_future_timestamp_prevent_deletion() {
            let f = Fixture::new(); let a = f.file("a.nvph"); let mut s = f.scan(); let token = s.view().token;
            f.file("profile/Local State");
            assert_eq!(f.clean(&mut s, &token, &[text(&a)], &[]).unwrap().deleted, 0); assert!(a.exists());
            fs::remove_file(f.cache().join("profile/Local State")).unwrap();
            let file = fs::OpenOptions::new().write(true).open(&a).unwrap();
            file.set_times(fs::FileTimes::new().set_modified(std::time::SystemTime::now() + Duration::from_secs(3600))).unwrap(); drop(file);
            assert!(f.scan().view().files.is_empty());
        }

        #[test]
        fn public_entrypoints_refuse_elevation_without_touching_fixture() {
            let f = Fixture::new(); let a = f.file("a.nvph");
            if platform::require_standard_user().is_err() {
                assert!(scan(&f.paths, &[]).is_err());
                let mut s = f.scan(); let token = s.view().token;
                assert!(super::super::clean(&mut s, &token, &[text(&a)], &[]).is_err());
                assert!(s.consumed && a.exists());
            } else {
                let mut s = scan(&f.paths, &[]).unwrap(); let token = s.view().token;
                assert_eq!(super::super::clean(&mut s, &token, &[text(&a)], &[]).unwrap().deleted, 1);
                assert!(!a.exists());
            }
        }

        #[test]
        fn open_parent_handles_really_prevent_rename_until_released() {
            let f = Fixture::new(); f.file("a.nvph");
            platform::assert_parent_pinning(&f.cache());
            assert!(f.cache().join("a.nvph").exists());
        }
    }
}
