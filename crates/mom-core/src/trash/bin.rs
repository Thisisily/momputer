//! The Recycle Bin: what's in it, read from Windows' own records, and emptying it.
//!
//! Each drive keeps the bin in X:\$Recycle.Bin\<user's SID>. Every deleted thing is a pair of
//! files: `$R<id>.ext` is the thing itself and `$I<id>.ext` says where it came from, how big it
//! was and when it was deleted.

use super::{ago, entries, name_of, tally_many, Cand, Found, Tally, TrashCtx};
use crate::util::plural;
use std::path::{Path, PathBuf};

/// One `$I` record.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Record {
    /// Full path the thing was deleted from.
    pub original: String,
    pub size: u64,
    /// Unix seconds.
    pub deleted: u64,
}

/// Reads a `$I` file. Version 1 (Vista to 8.1) has a fixed 260-character path at offset 24;
/// version 2 (Windows 10 on) has a 4-byte length in characters at 24, then the path.
pub(super) fn parse_record(b: &[u8]) -> Option<Record> {
    let u64_at = |o: usize| b.get(o..o + 8).map(|s| u64::from_le_bytes(s.try_into().unwrap()));
    let version = u64_at(0)?;
    let size = u64_at(8)?;
    let filetime = u64_at(16)?;
    let (start, chars) = match version {
        1 => (24, 260),
        2 => (28, u32::from_le_bytes(b.get(24..28)?.try_into().ok()?) as usize),
        _ => return None,
    };
    let bytes = b.get(start..)?;
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .take(chars)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|u| *u != 0)
        .collect();
    if units.is_empty() {
        return None;
    }
    Some(Record {
        original: String::from_utf16_lossy(&units),
        size,
        deleted: (filetime / 10_000_000).saturating_sub(11_644_473_600),
    })
}

pub(super) fn find(ctx: &TrashCtx) -> Found {
    if !cfg!(windows) {
        return Found::none();
    }
    let dirs = bin_dirs();
    let mut f = scan(&dirs, ctx.now);
    f.root = dirs.first().cloned();
    f
}

/// Everything in the given bin folders.
fn scan(dirs: &[PathBuf], now: u64) -> Found {
    let mut f = Found::none();
    f.available = true;
    let mut found: Vec<(PathBuf, PathBuf, Record, bool)> = Vec::new();
    for dir in dirs {
        for i_path in entries(dir) {
            let n = name_of(&i_path);
            let Some(id) = n.strip_prefix("$I") else { continue };
            let r_path = dir.join(format!("$R{id}"));
            let Ok(meta) = std::fs::symlink_metadata(&r_path) else { continue }; // a record without its file
            let Some(rec) = std::fs::read(&i_path).ok().and_then(|b| parse_record(&b)) else { continue };
            found.push((r_path, i_path, rec, meta.is_dir()));
        }
    }
    // Deleted folders: count what's inside, all at once.
    let folders: Vec<PathBuf> = found.iter().filter(|x| x.3).map(|x| x.0.clone()).collect();
    let mut inside = tally_many(&folders, 200_000, u64::MAX).into_iter();
    for (r_path, i_path, rec, is_dir) in found {
        let original = Path::new(&rec.original);
        let mut t = Tally::of(rec.size, 1);
        if is_dir {
            let counted = inside.next().unwrap_or_default();
            t.files = counted.files;
            t.complete = counted.complete;
            if t.bytes == 0 {
                t.bytes = counted.bytes;
            }
        }
        let name = original.file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_else(|| rec.original.clone());
        let mut c = Cand::new(r_path.clone(), name, &t, now);
        c.targets = vec![r_path, i_path];
        c.age_known = rec.deleted > 0 && rec.deleted <= now;
        c.age = if c.age_known { now - rec.deleted } else { 0 };
        let folder = original.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
        c.sub = Some(if is_dir { format!("Folder with {}, from {folder}", plural(t.files, "file")) } else { format!("From {folder}") });
        let r = if c.age_known { format!("Deleted {} ago", ago(c.age)) } else { "Deletion date unavailable".into() };
        f.cands.push(c.verdict(true, r));
    }
    f
}

/// Permanently removes one already-deleted item. Keep its Windows record until all of
/// the payload is gone, so a failed removal does not strand an invisible Bin item.
pub(super) fn remove_record(c: &Cand) -> bool {
    remove_record_with(c, remove_bin_entry)
}

fn remove_record_with(c: &Cand, mut remove: impl FnMut(&Path) -> bool) -> bool {
    let Some((data, record)) = record_pair(c) else { return false };
    if !remove(data) || !matches!(std::fs::symlink_metadata(data), Err(e) if e.kind() == std::io::ErrorKind::NotFound) {
        return false;
    }
    // Recheck the parent and record after removing the payload. This route may bypass
    // browser-data guards only for the exact pair that Windows already put in the Bin.
    if record_pair(c).is_none() {
        return false;
    }
    remove(record)
}

fn record_pair(c: &Cand) -> Option<(&Path, &Path)> {
    let [data, record] = c.targets.as_slice() else { return None };
    if c.path != *data || !data.is_absolute() || !record.is_absolute() || data.parent() != record.parent() {
        return None;
    }
    for path in [data, record] {
        if path.components().any(|part| matches!(part, std::path::Component::ParentDir | std::path::Component::CurDir)) {
            return None;
        }
    }
    let id = data.file_name()?.to_str()?.strip_prefix("$R")?;
    if id.is_empty() || record.file_name()?.to_str()?.strip_prefix("$I")? != id {
        return None;
    }
    let parent = data.parent()?;
    let sid = parent.file_name()?.to_str()?;
    if !sid.starts_with("S-1-") || !sid[4..].split('-').all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    if !parent.parent()?.file_name()?.to_str()?.eq_ignore_ascii_case("$Recycle.Bin") {
        return None;
    }
    for ancestor in parent.ancestors() {
        let meta = std::fs::symlink_metadata(ancestor).ok()?;
        if !meta.is_dir() || is_link(&meta) {
            return None;
        }
    }
    let meta = std::fs::symlink_metadata(record).ok()?;
    (meta.is_file() && !is_link(&meta)).then_some((data.as_path(), record.as_path()))
}

fn is_link(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_type().is_symlink() || meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

/// Only used after record_pair proves this is an already-deleted Bin payload/record.
/// Unlike ordinary cleanup, browser contents are allowed here. remove_dir_all removes
/// links themselves without following their targets; a root directory link is unlinked.
fn remove_bin_entry(path: &Path) -> bool {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) => return e.kind() == std::io::ErrorKind::NotFound,
    };
    let result = if is_link(&meta) {
        #[cfg(windows)]
        let directory = {
            use std::os::windows::fs::MetadataExt;
            meta.file_attributes() & 0x10 != 0
        };
        #[cfg(not(windows))]
        let directory = false;
        if directory { std::fs::remove_dir(path) } else { std::fs::remove_file(path) }
    } else if meta.is_dir() {
        std::fs::remove_dir_all(path)
    } else if meta.is_file() {
        std::fs::remove_file(path)
    } else {
        return false;
    };
    result.is_ok() || result.is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
}

/// The current user's bin folder on every fixed drive that has one.
fn bin_dirs() -> Vec<PathBuf> {
    let sid = current_sid();
    let mut out = Vec::new();
    for root in crate::util::fixed_drives() {
        let bin = Path::new(&root).join("$Recycle.Bin");
        match &sid {
            Some(sid) => {
                let d = bin.join(sid);
                if d.is_dir() {
                    out.push(d);
                }
            }
            // Couldn't ask Windows who we are: take the user folders we're allowed to read.
            None => out.extend(
                entries(&bin)
                    .into_iter()
                    .filter(|d| name_of(d).starts_with("S-1-5-21-") && std::fs::read_dir(d).is_ok()),
            ),
        }
    }
    out
}

/// Tells Explorer the bin changed, so its icon shows empty (or not) right away.
pub(super) fn refresh_icon() {
    #[cfg(windows)]
    for dir in bin_dirs() {
        use windows_sys::Win32::UI::Shell::{SHChangeNotify, SHCNE_UPDATEDIR, SHCNF_PATHW};
        let w: Vec<u16> = dir.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
        // SAFETY: a NUL-terminated path that outlives the call; it only tells Explorer to look again.
        unsafe { SHChangeNotify(SHCNE_UPDATEDIR as i32, SHCNF_PATHW, w.as_ptr().cast(), std::ptr::null()) };
    }
}

/// How much the Recycle Bin on each drive takes. Windows deletes anything bigger than that for
/// good, without asking, so mom recycles only what fits and leaves the rest where it is.
#[derive(Default)]
pub(crate) struct Room(std::collections::HashMap<String, Option<u64>>);

impl Room {
    /// Whether `bytes` at `path` would land in the bin rather than be deleted for good.
    pub(crate) fn fits(&mut self, path: &Path, bytes: u64) -> bool {
        let Some(root) = drive_root(path) else { return false };
        let cap = *self.0.entry(root.clone()).or_insert_with(|| capacity(&root));
        cap.is_some_and(|c| bytes < c)
    }
}

/// "C:\" for a path on drive C. Network paths have no Recycle Bin.
fn drive_root(path: &Path) -> Option<String> {
    use std::path::{Component, Prefix};
    match path.components().next()? {
        Component::Prefix(p) => match p.kind() {
            Prefix::Disk(d) | Prefix::VerbatimDisk(d) => Some(format!("{}:\\", (d as char).to_ascii_uppercase())),
            _ => None,
        },
        _ => None,
    }
}

/// The biggest thing the bin on a drive takes, or None when the drive has none: it's switched
/// off there ("Don't move files to the Recycle Bin"), or it isn't a fixed disk.
#[cfg(windows)]
fn capacity(root: &str) -> Option<u64> {
    use crate::health::reg::{dword_value, Hive};
    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetVolumeNameForVolumeMountPointW};
    if !crate::util::fixed_drives().iter().any(|d| d.eq_ignore_ascii_case(root)) {
        return None;
    }
    let w = crate::util::wide(root);
    let mut name = [0u16; 64];
    // SAFETY: `w` is NUL-terminated; `name` holds as many chars as passed.
    let named = unsafe { GetVolumeNameForVolumeMountPointW(w.as_ptr(), name.as_mut_ptr(), name.len() as u32) } != 0;
    // "\\?\Volume{1b2c…}\" -> "{1b2c…}", the key Windows keeps the drive's bin settings under.
    let volume = named.then(|| String::from_utf16_lossy(&name[..name.iter().position(|&c| c == 0).unwrap_or(0)])).and_then(|v| {
        let start = v.find('{')?;
        Some(v[start..].trim_end_matches('\\').to_string())
    });
    let key = volume.map(|g| format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Volume\{g}"));
    let setting = |value: &str| key.as_deref().and_then(|k| dword_value(Hive::CurrentUser, k, value));
    if setting("NukeOnDelete") == Some(1) {
        return None;
    }
    if let Some(mb) = setting("MaxCapacity") {
        return Some(u64::from(mb) * 1024 * 1024);
    }
    // Never changed: Windows gives the bin 5% of the drive.
    let mut total = 0u64;
    // SAFETY: `w` is NUL-terminated; the one out pointer we want is ours, the others are optional.
    let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), std::ptr::null_mut(), &mut total, std::ptr::null_mut()) } != 0;
    ok.then_some(total / 20)
}

#[cfg(not(windows))]
fn capacity(_: &str) -> Option<u64> {
    None
}

/// The current user's SID ("S-1-5-21-…-1001"), which names their folder in every bin.
#[cfg(windows)]
fn current_sid() -> Option<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    // SAFETY: plain Win32 calls with buffers we own; the token handle and the SID string are
    // released before returning.
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let mut buf = vec![0u64; 128]; // 1 KB, 8-byte aligned for TOKEN_USER
        let mut len = 0u32;
        let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), (buf.len() * 8) as u32, &mut len);
        CloseHandle(token);
        if ok == 0 {
            return None;
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut s) == 0 || s.is_null() {
            return None;
        }
        let mut n = 0;
        while *s.add(n) != 0 {
            n += 1;
        }
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(s, n));
        LocalFree(s.cast());
        Some(sid)
    }
}

#[cfg(not(windows))]
fn current_sid() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trash::tests::fixture;

    #[test]
    fn only_what_fits_a_drives_bin_is_recycled() {
        assert_eq!(drive_root(Path::new(r"c:\Users\me\x")), Some(r"C:\".to_string()));
        assert_eq!(drive_root(Path::new(r"\\?\D:\big")), Some(r"D:\".to_string()));
        assert_eq!(drive_root(Path::new(r"\\server\share\x")), None);
        let mut room = Room::default();
        // A network share has no Recycle Bin: Windows would delete for good, so nothing fits.
        assert!(!room.fits(Path::new(r"\\server\share\x"), 1));
        #[cfg(windows)]
        {
            let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
            let p = PathBuf::from(format!(r"{system}\momputer-test-never-made"));
            if capacity(&format!(r"{system}\")).is_some() {
                assert!(room.fits(&p, 1), "a byte fits");
                assert!(!room.fits(&p, u64::MAX), "nothing fits a bin bigger than the drive");
            }
        }
    }

    fn filetime(unix: u64) -> u64 {
        (unix + 11_644_473_600) * 10_000_000
    }

    fn v2(path: &str, size: u64, deleted: u64) -> Vec<u8> {
        let units: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        let mut b = Vec::new();
        b.extend(2u64.to_le_bytes());
        b.extend(size.to_le_bytes());
        b.extend(filetime(deleted).to_le_bytes());
        b.extend((units.len() as u32).to_le_bytes());
        for u in units {
            b.extend(u.to_le_bytes());
        }
        b
    }

    fn v1(path: &str, size: u64, deleted: u64) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(1u64.to_le_bytes());
        b.extend(size.to_le_bytes());
        b.extend(filetime(deleted).to_le_bytes());
        let mut units: Vec<u16> = path.encode_utf16().collect();
        units.resize(260, 0);
        for u in units {
            b.extend(u.to_le_bytes());
        }
        b
    }

    #[test]
    fn reads_both_kinds_of_recycle_bin_records() {
        let at = 1_790_000_000;
        let r = parse_record(&v2(r"C:\Users\ilya\Downloads\Setup Ünïcode.exe", 123_456, at)).unwrap();
        assert_eq!(r, Record { original: r"C:\Users\ilya\Downloads\Setup Ünïcode.exe".into(), size: 123_456, deleted: at });
        let r = parse_record(&v1(r"D:\old\notes.txt", 42, at)).unwrap();
        assert_eq!((r.original.as_str(), r.size, r.deleted), (r"D:\old\notes.txt", 42, at));
        assert_eq!(parse_record(&[0u8; 10]), None, "too short");
        let mut bad = v2(r"C:\x", 1, at);
        bad[0] = 9;
        assert_eq!(parse_record(&bad), None, "unknown version");
        let mut cut = v2(r"C:\folder\file.txt", 1, at);
        cut.truncate(28 + 10); // length says more than is there: take what's there
        assert_eq!(parse_record(&cut).unwrap().original, r"C:\fo");
    }

    #[test]
    fn lists_what_is_in_a_bin_folder() {
        let root = fixture("bin");
        let dir = root.join("S-1-5-21-1-2-3-1001");
        std::fs::create_dir_all(dir.join("$RABC123")).unwrap();
        std::fs::write(dir.join("$RABC123").join("a.txt"), b"hello").unwrap();
        std::fs::write(dir.join("$IABC123"), v2(r"C:\Work\old-project", 5, 1_790_000_000)).unwrap();
        std::fs::write(dir.join("$RXYZ.png"), vec![0u8; 300]).unwrap();
        std::fs::write(dir.join("$IXYZ.png"), v2(r"C:\Users\me\Pictures\cat.png", 300, 1_790_000_000)).unwrap();
        std::fs::write(dir.join("$IGONE.txt"), v2(r"C:\gone.txt", 7, 1_790_000_000)).unwrap(); // no $R: skipped
        std::fs::write(dir.join("desktop.ini"), b"[.ShellClassInfo]").unwrap();
        let f = scan(std::slice::from_ref(&dir), 1_790_086_400);
        assert_eq!(f.cands.len(), 2);
        let cat = f.cands.iter().find(|c| c.name == "cat.png").unwrap();
        assert_eq!((cat.bytes, cat.goes, cat.sub.as_deref()), (300, true, Some(r"From C:\Users\me\Pictures")));
        assert_eq!(cat.reason, "Deleted 24 hours ago");
        assert_eq!(cat.targets, vec![dir.join("$RXYZ.png"), dir.join("$IXYZ.png")]);
        let folder = f.cands.iter().find(|c| c.name == "old-project").unwrap();
        assert_eq!(folder.sub.as_deref(), Some(r"Folder with 1 file, from C:\Work"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn invalid_bin_dates_do_not_claim_an_age() {
        let root = fixture("bin-unknown-age");
        std::fs::create_dir_all(&root).unwrap();
        let now = 1_790_086_400;
        for (id, deleted) in [("ZERO", 0), ("FUTURE", now + 86_400)] {
            std::fs::write(root.join(format!("$R{id}")), b"discarded").unwrap();
            std::fs::write(root.join(format!("$I{id}")), v2(r"C:\Work\old-file", 9, deleted)).unwrap();
        }
        let found = scan(std::slice::from_ref(&root), now);
        assert_eq!(found.cands.len(), 2);
        for c in found.cands {
            assert!(!c.age_known);
            assert_eq!(c.age, 0);
            assert_eq!(c.reason, "Deletion date unavailable");
            assert!(c.goes, "emptying already discarded data does not depend on age");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    fn deletion_fixture(name: &str) -> (PathBuf, Cand) {
        let root = fixture(name);
        let dir = root.join("$Recycle.Bin").join("S-1-5-21-1-2-3-1001");
        let data = dir.join("$RABC123");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("a.txt"), b"hello").unwrap();
        std::fs::write(dir.join("$IABC123"), v2(r"C:\Work\old-project", 5, 1_790_000_000)).unwrap();
        let c = scan(&[dir], 1_790_086_400).cands.pop().unwrap();
        (root, c)
    }

    #[test]
    fn failed_payload_removal_preserves_its_bin_record() {
        let (root, c) = deletion_fixture("bin-failed-removal");
        let mut attempted = Vec::new();
        assert!(!remove_record_with(&c, |path| {
            attempted.push(path.to_path_buf());
            false
        }));
        assert_eq!(attempted, vec![c.targets[0].clone()]);
        assert!(c.targets[0].join("a.txt").is_file());
        assert!(c.targets[1].is_file(), "Windows must still be able to list and restore the item");
        // A deletion reporting success without removing the payload must also retain $I.
        assert!(!remove_record_with(&c, |_| true));
        assert!(c.targets[1].is_file());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn browser_data_already_in_the_bin_can_be_removed() {
        let (root, c) = deletion_fixture("bin-browser-data");
        std::fs::write(c.path.join("Local State"), b"already deleted profile").unwrap();
        std::fs::create_dir_all(c.path.join("Local Storage")).unwrap();
        std::fs::write(c.path.join("Local Storage").join("data"), b"already deleted storage").unwrap();
        assert!(!crate::browser_data::tree_is_safe(&c.path), "ordinary cleanup protects this content");
        assert!(remove_record(&c));
        assert!(!c.targets[0].exists());
        assert!(!c.targets[1].exists());
        assert!(c.path.parent().unwrap().is_dir(), "remove only the selected pair");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn bin_removal_rejects_mismatched_and_foreign_targets() {
        let (root, mut c) = deletion_fixture("bin-invalid-pair");
        let record = c.targets[1].clone();
        let reject = |c: &Cand| assert!(!remove_record_with(c, |_| panic!("invalid pair must never reach deletion")));
        c.targets[1] = record.with_file_name("$IOTHER");
        reject(&c);
        c.targets[1] = root.join("$IABC123");
        reject(&c);
        c.targets[1] = record.clone();
        c.targets.push(root.join("foreign.txt"));
        reject(&c);
        c.targets.pop();
        c.path = root.join("other");
        reject(&c);
        let foreign = root.join("ordinary-folder");
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::write(foreign.join("$IABC123"), b"not a Bin item").unwrap();
        c.path = foreign.join("$RABC123");
        c.targets = vec![c.path.clone(), foreign.join("$IABC123")];
        reject(&c);
        assert!(record.is_file());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
