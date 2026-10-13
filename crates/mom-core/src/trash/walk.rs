//! Walking folders fast: measuring them (size, file count, newest change, what kind of files)
//! and deleting them.
//!
//! On Windows every folder open can wait on the antivirus, so walking a big tree one folder at a
//! time takes minutes. Mom works on many folders at once on a small pool of threads instead
//! (about twenty times faster), and stops counting a folder that's absurdly big.

use crate::util::{mtime_secs, plural};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};

const KINDS: [&str; 12] = [
    "images",
    "videos",
    "audio",
    "logs",
    "archives",
    "installers",
    "code",
    "programs",
    "documents",
    "data files",
    "crash dumps",
    "other files",
];
const OTHER: usize = 11;

/// Threads that walk folders. They spend nearly all their time waiting on the disk (or the
/// antivirus), so more than the number of cores is fine.
const WALKERS: usize = 24;

/// The kind of file, from its extension, without allocating (this runs for every file).
fn kind_of_name(name: &Path) -> usize {
    let Some(ext) = name.extension().and_then(|e| e.to_str()) else { return OTHER };
    let mut buf = [0u8; 12];
    if ext.len() > buf.len() {
        return OTHER;
    }
    buf[..ext.len()].copy_from_slice(ext.as_bytes());
    buf[..ext.len()].make_ascii_lowercase();
    std::str::from_utf8(&buf[..ext.len()]).map(kind_of).unwrap_or(OTHER)
}

fn kind_of(ext: &str) -> usize {
    match ext {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "heic" | "avif" | "ico" | "svg" | "psd" => 0,
        "mp4" | "mov" | "webm" | "mkv" | "avi" | "m4v" | "wmv" | "flv" => 1,
        "mp3" | "wav" | "m4a" | "ogg" | "flac" | "aac" | "opus" => 2,
        "log" | "out" | "err" | "etl" | "trace" => 3,
        "zip" | "7z" | "rar" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "crate" | "whl" | "nupkg" | "jar" | "cab" => 4,
        "msi" | "msix" | "msixbundle" | "appx" | "appxbundle" => 5,
        "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "map" | "py" | "pyc" | "rs" | "go" | "java" | "kt" | "c" | "cc"
        | "cpp" | "h" | "hpp" | "cs" | "rb" | "php" | "sh" | "ps1" | "bat" | "css" | "scss" | "html" | "vue"
        | "svelte" | "rlib" | "rmeta" | "d" | "o" | "obj" => 6,
        "exe" | "dll" | "so" | "dylib" | "node" | "pdb" | "lib" | "wasm" | "pak" => 7,
        "pdf" | "doc" | "docx" | "md" | "txt" | "rtf" | "xls" | "xlsx" | "ppt" | "pptx" | "csv" => 8,
        "json" | "jsonl" | "sqlite" | "db" | "yaml" | "yml" | "toml" | "xml" | "bin" | "dat" | "idx" | "pack"
        | "safetensors" | "gguf" | "onnx" | "pt" | "ckpt" | "parquet" => 9,
        "dmp" | "mdmp" | "hdmp" | "wer" => 10,
        _ => OTHER,
    }
}

/// What's in a file or folder.
#[derive(Clone, Debug)]
pub(super) struct Tally {
    pub bytes: u64,
    pub files: u64,
    /// Newest file mtime; the folder's own mtime if it holds no files.
    pub newest: u64,
    /// False when mom stopped counting (too many files to count quickly).
    pub complete: bool,
    /// Links and junctions in it (or it is one). Never followed, so what they point at isn't
    /// counted, and a folder holding one isn't really empty.
    pub links: u64,
    /// Bytes per entry of KINDS.
    kinds: [u64; KINDS.len()],
}

impl Default for Tally {
    fn default() -> Self {
        Tally { bytes: 0, files: 0, newest: 0, complete: true, links: 0, kinds: [0; KINDS.len()] }
    }
}

impl Tally {
    /// A size someone else measured (Docker, a Recycle Bin record).
    pub fn of(bytes: u64, files: u64) -> Tally {
        Tally { bytes, files, ..Default::default() }
    }

    fn add_file(&mut self, name: &Path, meta: &std::fs::Metadata) {
        let len = meta.len();
        self.bytes += len;
        self.files += 1;
        self.newest = self.newest.max(mtime_secs(meta));
        self.kinds[kind_of_name(name)] += len;
    }

    pub fn add(&mut self, other: &Tally) {
        self.bytes += other.bytes;
        self.files += other.files;
        self.newest = self.newest.max(other.newest);
        self.complete &= other.complete;
        self.links += other.links;
        for (k, v) in self.kinds.iter_mut().zip(other.kinds) {
            *k += v;
        }
    }

    /// "38 files, mostly images".
    pub fn sub(&self) -> Option<String> {
        if self.files == 0 {
            return Some(if self.complete { "Empty".into() } else { "Too big to count".into() });
        }
        let count = if self.complete { plural(self.files, "file") } else { format!("Over {}", plural(self.files, "file")) };
        let (top, top_bytes) = self.kinds.iter().enumerate().max_by_key(|(_, b)| **b).map(|(i, b)| (i, *b)).unwrap_or((OTHER, 0));
        if self.files > 1 && top != OTHER && top_bytes * 2 > self.bytes {
            Some(format!("{count}, mostly {}", KINDS[top]))
        } else {
            Some(count)
        }
    }
}

pub(super) fn tally(path: &Path) -> Tally {
    tally_many(&[path.to_path_buf()], super::ENTRY_CAP, u64::MAX).pop().unwrap_or_default()
}

/// Measures every path (files or folders), walking all the folders at once. Each path stops
/// being counted after `cap` entries, and everything stops after `budget` entries in total;
/// those come back with `complete: false`. Earlier paths are walked first.
pub(super) fn tally_many(paths: &[PathBuf], cap: u64, budget: u64) -> Vec<Tally> {
    let mut out: Vec<Tally> = vec![Tally::default(); paths.len()];
    let own_mtime: Vec<AtomicU64> = paths.iter().map(|_| AtomicU64::new(0)).collect();
    let seen: Vec<AtomicU64> = paths.iter().map(|_| AtomicU64::new(0)).collect();
    let cut: Vec<AtomicBool> = paths.iter().map(|_| AtomicBool::new(false)).collect();
    let spent = AtomicU64::new(0);
    // The paths themselves go through the pool too: even looking one up can wait on the antivirus.
    let start: Vec<(usize, PathBuf)> = paths.iter().cloned().enumerate().rev().collect();
    let found = in_parallel(start, |mine: &mut HashMap<usize, Tally>, i, dir, more| {
        if cut[i].load(Ordering::Relaxed) || seen[i].load(Ordering::Relaxed) > cap || spent.load(Ordering::Relaxed) > budget {
            cut[i].store(true, Ordering::Relaxed);
            return;
        }
        if dir == paths[i] {
            match std::fs::symlink_metadata(dir) {
                Ok(m) if m.is_file() => return mine.entry(i).or_default().add_file(dir, &m),
                Ok(m) if m.is_dir() => own_mtime[i].store(mtime_secs(&m), Ordering::Relaxed),
                Ok(m) if m.is_symlink() => {
                    mine.entry(i).or_default().links += 1;
                    return; // a link: never followed
                }
                _ => return, // gone
            }
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let t = mine.entry(i).or_default();
        let mut n = 0;
        for e in rd.flatten() {
            n += 1;
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                more.push((i, e.path())); // links and junctions aren't dirs here, so never followed
            } else if ft.is_file() {
                if let Ok(m) = e.metadata() {
                    t.add_file(Path::new(&e.file_name()), &m);
                }
            } else if ft.is_symlink() {
                t.links += 1;
            }
        }
        // Counted per folder, not per file: two dozen threads bumping one counter slow each other down.
        seen[i].fetch_add(n, Ordering::Relaxed);
        spent.fetch_add(n, Ordering::Relaxed);
    });
    for part in found {
        for (i, t) in part {
            out[i].add(&t);
        }
    }
    for (i, t) in out.iter_mut().enumerate() {
        t.complete = !cut[i].load(Ordering::Relaxed);
        if t.files == 0 {
            t.newest = own_mtime[i].load(Ordering::Relaxed);
        }
    }
    out
}

/// Deletes files and folders for real, many at once: the files on a pool of threads, then the
/// folders from the deepest up. Links and junctions are removed, never followed. Says, for each
/// path, whether it's completely gone (something in use stays behind). Counts each file to
/// `live` as it goes, and stops going deeper once `live` is told to stop.
pub(super) fn remove_all(paths: &[PathBuf], live: Option<&super::Live>) -> Vec<bool> {
    let mut start = Vec::new();
    for p in paths {
        match std::fs::symlink_metadata(p) {
            Ok(m) if m.is_dir() => start.push((0usize, p.clone())),
            Ok(m) if m.is_symlink() => remove_link(p),
            Ok(_) => remove_file(p),
            Err(_) => {}
        }
    }
    let mut folders: Vec<(usize, PathBuf)> = start.clone();
    let found = in_parallel(start, |mine: &mut Vec<(usize, PathBuf)>, depth, dir, more| {
        if live.is_some_and(super::Live::stopped) {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                mine.push((depth + 1, e.path()));
                more.push((depth + 1, e.path()));
            } else if ft.is_symlink() {
                remove_link(&e.path());
            } else {
                remove_file(&e.path());
                if let Some(l) = live {
                    l.file_gone();
                }
            }
        }
    });
    folders.extend(found.into_iter().flatten());
    folders.sort_by(|a, b| b.0.cmp(&a.0));
    // Folders of the same depth don't contain each other, so each level can go at once.
    for level in folders.chunk_by(|a, b| a.0 == b.0) {
        let per = level.len().div_ceil(WALKERS).max(1);
        std::thread::scope(|s| {
            for part in level.chunks(per) {
                s.spawn(move || {
                    for (_, d) in part {
                        let _ = std::fs::remove_dir(d);
                    }
                });
            }
        });
    }
    paths.iter().map(|p| std::fs::symlink_metadata(p).is_err()).collect()
}

/// Deletes one file or folder the careful way, for when mom runs as administrator: one at a
/// time with the standard library's remove_dir_all, which opens everything relative to the
/// folder it's in and never follows a link, even one swapped in while it works. (remove_all
/// above goes by path, which is fine as you but not as an administrator.)
pub(super) fn remove_without_following(p: &Path) -> bool {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_symlink() => remove_link(p),
        Ok(m) if m.is_dir() => {
            let _ = std::fs::remove_dir_all(p);
        }
        Ok(_) => remove_file(p),
        Err(_) => {}
    }
    std::fs::symlink_metadata(p).is_err()
}

/// Deletes a folder that sits where other people on this PC may be able to write, and only if
/// it's yours. The check and the delete are about the same folder, with nothing in between:
/// mom opens it and keeps it open in a way that stops anyone renaming, replacing or deleting it
/// (or any folder above it) while she works. She asks the open folder itself whether it's a
/// real folder and whose it is, empties it without following a link, lets go, and removes the
/// then-empty folder. If someone swaps something in at that last instant, all that can go is an
/// empty folder or a link, never what a link points to.
#[cfg(windows)]
pub(super) fn remove_owned(p: &Path) -> bool {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    const SHARE_READ_WRITE: u32 = 0x1 | 0x2; // not delete: that is what holds it in place
    const BACKUP_SEMANTICS: u32 = 0x0200_0000; // lets a folder be opened at all
    const OPEN_REPARSE_POINT: u32 = 0x0020_0000; // a link is opened as the link, not followed
    const REPARSE_POINT: u32 = 0x400;
    let Ok(held) = std::fs::OpenOptions::new().read(true).share_mode(SHARE_READ_WRITE).custom_flags(BACKUP_SEMANTICS | OPEN_REPARSE_POINT).open(p) else {
        return false;
    };
    let Ok(meta) = held.metadata() else { return false };
    if !meta.is_dir() || meta.file_attributes() & REPARSE_POINT != 0 || !crate::elevate::open_and_mine(&held) {
        return false;
    }
    let Ok(inside) = std::fs::read_dir(p) else { return false };
    for entry in inside.flatten() {
        remove_without_following(&entry.path());
    }
    drop(held);
    let _ = std::fs::remove_dir(p);
    std::fs::symlink_metadata(p).is_err()
}

#[cfg(not(windows))]
pub(super) fn remove_owned(p: &Path) -> bool {
    remove_without_following(p)
}

fn remove_file(p: &Path) {
    if std::fs::remove_file(p).is_ok() {
        return;
    }
    // Read-only files won't go until they're writable.
    if let Ok(m) = std::fs::symlink_metadata(p) {
        let mut perms = m.permissions();
        if perms.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            if std::fs::set_permissions(p, perms).is_ok() {
                let _ = std::fs::remove_file(p);
            }
        }
    }
}

/// A link to a folder is removed like a folder on Windows; never what it points to.
fn remove_link(p: &Path) {
    if std::fs::remove_file(p).is_err() {
        let _ = std::fs::remove_dir(p);
    }
}

/// Runs `visit` on every folder, starting from `start`, on WALKERS threads. `visit` gets its
/// thread's own state, the folder's tag and path, and a list to push subfolders onto (with their
/// tags). Returns every thread's state.
fn in_parallel<S, F>(start: Vec<(usize, PathBuf)>, visit: F) -> Vec<S>
where
    S: Default + Send,
    F: Fn(&mut S, usize, &Path, &mut Vec<(usize, PathBuf)>) + Sync,
{
    if start.is_empty() {
        return Vec::new();
    }
    // Folders waiting to be read, and how many are being read right now.
    let queue: Mutex<(Vec<(usize, PathBuf)>, usize)> = Mutex::new((start, 0));
    let wake = Condvar::new();
    let worker = || {
        let mut mine = S::default();
        loop {
            let job = {
                let mut q = queue.lock().unwrap_or_else(|e| e.into_inner());
                loop {
                    if let Some(job) = q.0.pop() {
                        q.1 += 1;
                        break Some(job);
                    }
                    if q.1 == 0 {
                        break None;
                    }
                    q = wake.wait(q).unwrap_or_else(|e| e.into_inner());
                }
            };
            let Some((tag, dir)) = job else {
                wake.notify_all();
                return mine;
            };
            let mut more = Vec::new();
            visit(&mut mine, tag, &dir, &mut more);
            let mut q = queue.lock().unwrap_or_else(|e| e.into_inner());
            q.0.extend(more);
            q.1 -= 1;
            wake.notify_all();
        }
    };
    std::thread::scope(|s| {
        let workers: Vec<_> = (0..WALKERS).map(|_| s.spawn(worker)).collect();
        workers.into_iter().filter_map(|w| w.join().ok()).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trash::tests::{fixture, write};

    #[test]
    fn removes_whole_trees_but_not_what_links_point_to() {
        let root = fixture("remove");
        for d in 0..8 {
            write(&root.join("tree").join(format!("pkg{d}")).join("lib").join("index.js"), 50, 1);
        }
        write(&root.join("tree").join("top.txt"), 5, 1);
        let ro = root.join("tree").join("pkg0").join("readonly.txt");
        write(&ro, 5, 1);
        let mut perms = std::fs::metadata(&ro).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&ro, perms).unwrap();
        write(&root.join("keep").join("precious.txt"), 9, 1);
        write(&root.join("loose.log"), 3, 1);
        // A junction (Windows) or symlink inside the tree, pointing at a folder that must survive.
        let link = root.join("tree").join("pkg1").join("linked");
        #[cfg(windows)]
        let made = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J", &link.to_string_lossy(), &root.join("keep").to_string_lossy()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(root.join("keep"), &link).is_ok();
        let gone = remove_all(&[root.join("tree"), root.join("loose.log"), root.join("missing")], None);
        assert_eq!(gone, vec![true, true, true]);
        assert!(made, "couldn't make a link to test with");
        assert!(root.join("keep").join("precious.txt").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn counts_everything_once_and_stops_at_the_cap() {
        let root = fixture("tally");
        for d in 0..6 {
            for f in 0..5 {
                write(&root.join("big").join(format!("d{d}")).join("deeper").join(format!("f{f}.png")), 100, 2);
            }
        }
        write(&root.join("small").join("a.log"), 10, 2);
        write(&root.join("one.zip"), 7, 2);
        std::fs::create_dir_all(root.join("empty")).unwrap();
        let paths = vec![root.join("big"), root.join("small"), root.join("one.zip"), root.join("empty"), root.join("missing")];
        let t = tally_many(&paths, u64::MAX, u64::MAX);
        assert_eq!((t[0].bytes, t[0].files, t[0].complete), (3000, 30, true));
        assert_eq!(t[0].sub().as_deref(), Some("30 files, mostly images"));
        assert_eq!((t[1].bytes, t[2].bytes, t[2].files), (10, 7, 1));
        assert_eq!((t[3].files, t[3].sub().as_deref()), (0, Some("Empty")));
        assert!(t[3].newest > 0, "an empty folder's age is its own");
        assert_eq!(t[4].bytes, 0);
        let capped = tally_many(&paths[..1], 10, u64::MAX);
        assert!(!capped[0].complete && capped[0].files < 30);
        assert!(capped[0].sub().unwrap().starts_with("Over") || capped[0].files == 0);
        let broke = tally_many(&paths[..2], u64::MAX, 5);
        assert!(!broke[0].complete || !broke[1].complete);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
