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
    /// Newest creation/modification evidence, including every directory. Cleanup age uses this,
    /// while `newest` keeps its raw file-modification meaning for other consumers.
    pub recency: u64,
    /// Sign-in or site-storage markers found anywhere in the measured tree.
    pub contains_browser_data: bool,
    /// False when anything could not be inspected, timed, or counted within the limits.
    pub complete: bool,
    /// Links and junctions in it (or it is one). Never followed, so what they point at isn't
    /// counted, and a folder holding one isn't really empty.
    pub links: u64,
    /// Bytes per entry of KINDS.
    kinds: [u64; KINDS.len()],
}

impl Default for Tally {
    fn default() -> Self {
        Tally { bytes: 0, files: 0, newest: 0, recency: 0, contains_browser_data: false, complete: true, links: 0, kinds: [0; KINDS.len()] }
    }
}

impl Tally {
    /// A size someone else measured (Docker, a Recycle Bin record).
    pub fn of(bytes: u64, files: u64) -> Tally {
        Tally { bytes, files, ..Default::default() }
    }

    fn observe(&mut self, meta: &std::fs::Metadata) {
        match super::cleanup_recency(meta) {
            Some(at) => self.recency = self.recency.max(at),
            None => self.complete = false,
        }
    }

    fn add_file(&mut self, name: &Path, meta: &std::fs::Metadata) {
        if let Some(name) = name.file_name() {
            self.contains_browser_data |= crate::browser_data::sensitive_entry(name, false);
        }
        self.observe(meta);
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
        self.recency = self.recency.max(other.recency);
        self.contains_browser_data |= other.contains_browser_data;
        self.complete &= other.complete;
        self.links += other.links;
        for (k, v) in self.kinds.iter_mut().zip(other.kinds) {
            *k += v;
        }
    }

    /// "38 files, mostly images".
    pub fn sub(&self) -> Option<String> {
        if self.files == 0 {
            return Some(if self.complete { "Empty".into() } else { "Contents could not be fully checked".into() });
        }
        let count = if self.complete { plural(self.files, "file") } else { format!("At least {}; partly checked", plural(self.files, "file")) };
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
        if cut[i].load(Ordering::Relaxed) {
            return;
        }
        let t = mine.entry(i).or_default();
        // A root is an entry too. Descendants were reserved while enumerating their parent.
        if dir == paths[i] && (seen[i].fetch_add(1, Ordering::Relaxed) >= cap || spent.fetch_add(1, Ordering::Relaxed) >= budget) {
            cut[i].store(true, Ordering::Relaxed);
            return;
        }
        match std::fs::symlink_metadata(dir) {
            Ok(m) if m.is_symlink() => {
                t.links += 1;
                return; // a link: never followed
            }
            Ok(m) if m.is_file() && dir == paths[i] => return t.add_file(dir, &m),
            Ok(m) if m.is_dir() => {
                if let Some(name) = dir.file_name() {
                    t.contains_browser_data |= crate::browser_data::sensitive_entry(name, true);
                }
                t.observe(&m);
                if dir == paths[i] { own_mtime[i].store(mtime_secs(&m), Ordering::Relaxed); }
            }
            _ => { t.complete = false; return; }
        }
        let Ok(rd) = std::fs::read_dir(dir) else { t.complete = false; return };
        for entry in rd {
            let Ok(e) = entry else { t.complete = false; continue };
            if seen[i].fetch_add(1, Ordering::Relaxed) >= cap || spent.fetch_add(1, Ordering::Relaxed) >= budget {
                cut[i].store(true, Ordering::Relaxed);
                break;
            }
            let Ok(ft) = e.file_type() else { t.complete = false; continue };
            t.contains_browser_data |= crate::browser_data::sensitive_entry(&e.file_name(), ft.is_dir());
            if ft.is_dir() {
                more.push((i, e.path())); // links and junctions aren't dirs here, so never followed
            } else if ft.is_file() {
                if let Ok(m) = e.metadata() {
                    t.add_file(Path::new(&e.file_name()), &m);
                } else { t.complete = false; }
            } else if ft.is_symlink() {
                t.links += 1;
            } else {
                t.complete = false;
            }
        }
    });
    for part in found {
        for (i, t) in part {
            out[i].add(&t);
        }
    }
    for (i, t) in out.iter_mut().enumerate() {
        t.complete &= !cut[i].load(Ordering::Relaxed);
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
    if crate::browser_data::protects_entry_name(p, false) || crate::browser_data::protects_entry_name(p, true) { return false; }
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_symlink() => remove_link(p),
        Ok(m) if m.is_dir() => {
            if !crate::browser_data::tree_is_safe(p) { return false; }
            let _ = std::fs::remove_dir_all(p);
        }
        Ok(_) => {
            if crate::browser_data::protects_path(p) { return false; }
            remove_file(p);
        },
        Err(_) => {}
    }
    std::fs::symlink_metadata(p).is_err()
}

/// Deletes a folder that sits where other people on this PC may be able to write, and only if
/// it's yours. The check and the delete are about the same folder, with nothing in between:
/// mom opens it and keeps it open in a way that stops anyone renaming, replacing or deleting it
/// (or any folder above it) while she works. She asks the open folder itself whether it's a
/// real folder and whose it is, then empties it on many threads at once, each folder inside
/// held the same way while its own contents go (see `Held`), never following a link. Then she
/// lets go and removes the then-empty folder. If someone swaps something in at that last
/// instant, all that can go is an empty folder or a link, never what a link points to.
/// Counts each file to `live` as it goes, and goes no deeper once `live` is told to stop.
#[cfg(windows)]
pub(super) fn remove_owned(p: &Path, live: Option<&super::Live>) -> bool {
    let Some(held) = hold(p) else { return false };
    if !crate::elevate::open_and_mine(&held) {
        return false;
    }
    // And it must be the very folder that was found, not another one the same path leads to now:
    // opening a path follows any link in the folders above it. Asking the open folder where it
    // really is, and getting the path it was opened by, means nothing above it is a link. With
    // it held open, none of those folders can be renamed or replaced either, so the path keeps
    // meaning this folder for as long as mom works in it.
    if !opened_as_written(&held, p) {
        return false;
    }
    let top = std::sync::Arc::new(Held { open: Some(held), path: p.to_path_buf(), above: None, stays: true });
    // The top folder is read here; every folder below it is a job for the pool.
    let mut first = Vec::new();
    empty(&top, live, &mut first);
    let _: Vec<()> = pooled(first, |_: &mut (), (above, dir): (std::sync::Arc<Held>, PathBuf), more: &mut Vec<(std::sync::Arc<Held>, PathBuf)>| {
        if live.is_some_and(super::Live::stopped) {
            return;
        }
        match hold(&dir) {
            Some(open) => empty(&std::sync::Arc::new(Held { open: Some(open), path: dir, above: Some(above), stays: false }), live, more),
            // Not a real folder any more (a link now: the link goes, not what it leads to), or it
            // can't be held (something else has it open its own way): one at a time, the way
            // that never follows a link either.
            None => {
                remove_without_following(&dir);
            }
        }
    });
    // Every folder below has let go by now. One that wasn't quite empty when it tried to remove
    // itself (Windows finishes deleting a file only when the antivirus lets go of it) is still
    // there: those go now, one at a time, the way that never follows a link either.
    if !live.is_some_and(super::Live::stopped) {
        for left in std::fs::read_dir(p).into_iter().flatten().flatten() {
            remove_without_following(&left.path());
        }
    }
    drop(top); // and this lets go of the folder that was checked
    let _ = std::fs::remove_dir(p);
    std::fs::symlink_metadata(p).is_err()
}

/// A folder being emptied, held open so that neither it nor any folder above it can be renamed,
/// replaced or deleted meanwhile. Everything waiting to be done inside it keeps it alive; when
/// the last of that is done it lets go and, now empty, removes itself (unless something in it
/// was in use: then it stays). Only after that is the folder above let go of.
#[cfg(windows)]
struct Held {
    open: Option<std::fs::File>,
    path: PathBuf,
    /// Never read: it keeps the folder above held for as long as this one is.
    #[allow(dead_code)]
    above: Option<std::sync::Arc<Held>>,
    /// The folder the clean-up started from: its caller removes it.
    stays: bool,
}

#[cfg(windows)]
impl Drop for Held {
    fn drop(&mut self) {
        self.open.take(); // let go first: Windows won't remove a folder that is held
        if !self.stays {
            // If someone swaps it for a link at this instant, the link goes, not what it leads to.
            let _ = std::fs::remove_dir(&self.path);
        }
    }
}

/// Opens a folder as itself (a link by that name is opened as the link, never followed), for
/// reading and shared for reading and writing but not deleting, which is what keeps it in place.
/// None unless it is a real folder: not a link, not a file, not something that can't be opened.
#[cfg(windows)]
fn hold(p: &Path) -> Option<std::fs::File> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    const SHARE_READ_WRITE: u32 = 0x1 | 0x2; // not delete: that is what holds it in place
    const BACKUP_SEMANTICS: u32 = 0x0200_0000; // lets a folder be opened at all
    const OPEN_REPARSE_POINT: u32 = 0x0020_0000; // a link is opened as the link, not followed
    const REPARSE_POINT: u32 = 0x400;
    let open = std::fs::OpenOptions::new().read(true).share_mode(SHARE_READ_WRITE).custom_flags(BACKUP_SEMANTICS | OPEN_REPARSE_POINT).open(p).ok()?;
    let meta = open.metadata().ok()?;
    (meta.is_dir() && meta.file_attributes() & REPARSE_POINT == 0).then_some(open)
}

/// Deletes the files and links in a held folder, and hands its folders on as jobs that keep it
/// held until they are done. What the listing calls a folder is opened and checked again by
/// whoever takes the job; a file that became a link since the listing is removed as the link.
#[cfg(windows)]
fn empty(dir: &std::sync::Arc<Held>, live: Option<&super::Live>, more: &mut Vec<(std::sync::Arc<Held>, PathBuf)>) {
    if crate::browser_data::protects_entry_name(&dir.path, true) { return; }
    let Ok(rd) = std::fs::read_dir(&dir.path) else { return };
    for e in rd.flatten() {
        let Ok(kind) = e.file_type() else { continue };
        if kind.is_dir() {
            if !crate::browser_data::protects_entry_name(&e.path(), true) {
                more.push((dir.clone(), e.path()));
            }
        } else if kind.is_symlink() {
            remove_link(&e.path());
        } else {
            remove_file(&e.path());
            if let Some(l) = live {
                l.file_gone();
            }
        }
    }
}

/// Does this open folder really live at `path`, spelled that way (but for capitals)?
#[cfg(windows)]
fn opened_as_written(held: &std::fs::File, path: &Path) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
    let mut buf = vec![0u16; 32_768];
    // SAFETY: the handle is open for the length of the call, and `buf` is as long as we say.
    let n = unsafe { GetFinalPathNameByHandleW(held.as_raw_handle() as _, buf.as_mut_ptr(), buf.len() as u32, 0) } as usize;
    if n == 0 || n >= buf.len() {
        return false;
    }
    let real = String::from_utf16_lossy(&buf[..n]);
    let plain = |s: &str| s.strip_prefix(r"\\?\").unwrap_or(s).trim_end_matches('\\').to_ascii_lowercase();
    plain(&real) == plain(&path.to_string_lossy())
}

#[cfg(not(windows))]
pub(super) fn remove_owned(p: &Path, live: Option<&super::Live>) -> bool {
    let _ = live;
    remove_without_following(p)
}

fn remove_file(p: &Path) {
    if crate::browser_data::protects_entry_name(p, false) { return; }
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
    pooled(start, |mine: &mut S, (tag, dir): (usize, PathBuf), more: &mut Vec<(usize, PathBuf)>| visit(mine, tag, &dir, more))
}

/// Runs `work` on every job, starting from `start`, on WALKERS threads. `work` gets its thread's
/// own state, a job, and a list to push the jobs that one leads to. Returns every thread's state.
fn pooled<J, S, F>(start: Vec<J>, work: F) -> Vec<S>
where
    J: Send,
    S: Default + Send,
    F: Fn(&mut S, J, &mut Vec<J>) + Sync,
{
    if start.is_empty() {
        return Vec::new();
    }
    // Jobs waiting to be done, and how many are being done right now.
    let queue: Mutex<(Vec<J>, usize)> = Mutex::new((start, 0));
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
            let Some(job) = job else {
                wake.notify_all();
                return mine;
            };
            let mut more = Vec::new();
            work(&mut mine, job, &mut more);
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

    #[cfg(windows)]
    #[test]
    fn the_careful_delete_empties_a_folder_on_many_threads_and_follows_no_link() {
        let root = fixture("owned");
        std::fs::create_dir_all(&root).unwrap();
        // Spelled as it really is: the careful delete refuses a folder that isn't where its path says.
        let root = crate::util::real_path(&root).unwrap();
        let tree = root.join("node_modules");
        let mut files = 0;
        for pkg in 0..40 {
            for (sub, n) in [("", 3), ("lib", 4), ("lib/deep/deeper", 2)] {
                for f in 0..n {
                    write(&tree.join(format!("pkg{pkg}")).join(sub).join(format!("f{f}.js")), 10, 1);
                    files += 1;
                }
            }
        }
        std::fs::create_dir_all(tree.join("empty").join("emptier")).unwrap();
        let ro = tree.join("pkg0").join("readonly.txt");
        write(&ro, 5, 1);
        files += 1;
        let mut perms = std::fs::metadata(&ro).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&ro, perms).unwrap();
        // A junction deep inside, to a folder that must survive with what's in it.
        write(&root.join("keep").join("precious.txt"), 9, 1);
        let link = tree.join("pkg1").join("lib").join("linked");
        let made = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J", &link.to_string_lossy(), &root.join("keep").to_string_lossy()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(made, "couldn't make a link to test with");
        let live = crate::trash::Live::default();
        assert!(remove_owned(&tree, Some(&live)), "the whole tree went");
        assert!(!tree.exists() && root.join("keep").join("precious.txt").exists(), "and nothing a link led to");
        assert_eq!(live.now(0).files, files, "every file was counted as it went");
        // A link is not a folder of yours, however it is spelled; nor is a file, or something that isn't there.
        let again = root.join("again");
        write(&again.join("a.txt"), 1, 1);
        let alias = root.join("alias");
        assert!(std::process::Command::new("cmd").args(["/c", "mklink", "/J", &alias.to_string_lossy(), &again.to_string_lossy()]).output().is_ok_and(|o| o.status.success()));
        assert!(!remove_owned(&alias, None) && !remove_owned(&alias.join("a.txt"), None) && !remove_owned(&root.join("missing"), None));
        assert!(again.join("a.txt").exists(), "what the link leads to is untouched");
        // Told to stop, it goes no deeper: what was there stays.
        let stopped = crate::trash::Live::default();
        stopped.stop();
        write(&root.join("later").join("deep").join("x.txt"), 1, 1);
        assert!(!remove_owned(&root.join("later"), Some(&stopped)) && root.join("later").join("deep").join("x.txt").exists());
        let _ = std::fs::remove_dir(&alias);
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
        assert!(capped[0].sub().unwrap().starts_with("At least") || capped[0].files == 0);
        let broke = tally_many(&paths[..2], u64::MAX, 5);
        assert!(!broke[0].complete || !broke[1].complete);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cleanup_recency_includes_directories_and_keeps_raw_file_mtime_separate() {
        use crate::trash::tests::{touch_directory, touch_old};
        let root = fixture("recency-dirs");
        let dir = root.join("old");
        let file = dir.join("nested").join("old.bin");
        write(&file, 1, 100);
        let before = tally(&dir);
        assert!(before.complete && before.recency > 0);
        assert!(crate::util::now_secs().saturating_sub(before.recency) >= 99 * 86_400);
        std::fs::create_dir_all(dir.join("nested").join("new-empty")).unwrap();
        // Even when the target directory itself stayed old, a new empty descendant is activity.
        touch_directory(&dir, 100 * 86_400);
        let after = tally(&dir);
        assert_eq!(after.newest, before.newest, "raw file mtime did not change");
        assert!(after.complete && crate::util::now_secs().saturating_sub(after.recency) < 60);
        touch_old(&file, 100);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_invalid_and_capped_evidence_is_incomplete_including_a_flat_folder() {
        let root = fixture("recency-unknown");
        for n in 0..5 { write(&root.join("flat").join(format!("{n}.bin")), 1, 100); }
        let limited = tally_many(&[root.join("flat")], 3, u64::MAX).pop().unwrap();
        assert!(!limited.complete && limited.files <= 2, "the root also consumes one entry");
        let missing = tally(&root.join("missing"));
        assert!(!missing.complete && missing.recency == 0);
        assert_ne!(missing.sub().as_deref(), Some("Empty"));
        let invalid = root.join("invalid.bin");
        std::fs::write(&invalid, b"x").unwrap();
        std::fs::File::options().write(true).open(&invalid).unwrap().set_modified(std::time::UNIX_EPOCH).unwrap();
        let invalid = tally(&invalid);
        assert!(!invalid.complete && invalid.files == 1 && invalid.recency == 0);
        let mut aggregate = tally(&root.join("flat"));
        aggregate.add(&missing);
        assert!(!aggregate.complete, "a valid sibling cannot erase missing evidence");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn copied_file_creation_protects_it_even_if_every_modified_time_is_old() {
        use crate::trash::tests::touch_directory;
        let root = fixture("recency-birth");
        let dir = root.join("copied");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("download.bin");
        std::fs::write(&file, b"x").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(100 * 86_400);
        std::fs::File::options().write(true).open(&file).unwrap().set_modified(old).unwrap();
        touch_directory(&dir, 100 * 86_400);
        let t = tally(&dir);
        let now = crate::util::now_secs();
        assert!(t.complete && now.saturating_sub(t.newest) >= 99 * 86_400);
        assert!(now.saturating_sub(t.recency) < 60, "new creation survives restored mtimes");
        assert!(crate::trash::age_of(&file, now) < 60, "Room log cleanup uses the same birth evidence");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
