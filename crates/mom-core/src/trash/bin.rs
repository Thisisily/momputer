//! The Recycle Bin: what's in it, read from Windows' own records, and emptying it.
//!
//! Each drive keeps the bin in X:\$Recycle.Bin\<user's SID>. Every deleted thing is a pair of
//! files: `$R<id>.ext` is the thing itself and `$I<id>.ext` says where it came from, how big it
//! was and when it was deleted.

use super::{ago, entries, name_of, tally_many, Cand, CleanReport, Found, Tally, TrashCtx};
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
        c.age = now.saturating_sub(rec.deleted);
        let folder = original.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
        c.sub = Some(if is_dir { format!("Folder with {}, from {folder}", plural(t.files, "file")) } else { format!("From {folder}") });
        let r = format!("Deleted {} ago", ago(c.age));
        f.cands.push(c.verdict(true, r));
    }
    f
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

/// Empties the whole bin with Windows' own call (no confirmation, no progress window, no sound).
pub(super) fn empty(ctx: &TrashCtx, mut r: CleanReport) -> CleanReport {
    let before = find(ctx);
    let (n, bytes) = (before.cands.len() as u64, before.cands.iter().map(|c| c.bytes).sum::<u64>());
    if n == 0 {
        r.message = "The Recycle Bin is already empty.".into();
        return r;
    }
    match empty_all() {
        Ok(()) => {
            let after = find(ctx);
            let left = after.cands.len() as u64;
            r.deleted = n.saturating_sub(left);
            r.skipped = left.min(n);
            r.freed = bytes.saturating_sub(after.cands.iter().map(|c| c.bytes).sum::<u64>());
            r.message = format!("Emptied the Recycle Bin: {} gone for good.", crate::util::human_bytes(r.freed));
            if left > 0 {
                r.message.push_str(&format!(" {} couldn't be deleted (in use).", plural(left, "item")));
            }
        }
        Err(e) => r.message = format!("Couldn't empty the Recycle Bin: {e}"),
    }
    r
}

#[cfg(windows)]
fn empty_all() -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::{SHEmptyRecycleBinW, SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND};
    // SAFETY: null window and null root (every drive) are documented values.
    let hr = unsafe {
        SHEmptyRecycleBinW(std::ptr::null_mut(), std::ptr::null(), SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND)
    };
    const E_UNEXPECTED: i32 = 0x8000_FFFFu32 as i32; // what it says when the bin is already empty
    if hr >= 0 || hr == E_UNEXPECTED {
        Ok(())
    } else {
        Err(format!("Windows said no (0x{:08X})", hr as u32))
    }
}

#[cfg(not(windows))]
fn empty_all() -> Result<(), String> {
    Ok(())
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
}
