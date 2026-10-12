//! Files of your own that only take up room: exact copies of the same file, and big files nobody
//! has opened in a year. Mom looks in Downloads, Documents, Desktop, Pictures, Videos, Music and
//! OneDrive, never inside projects, links or files that are only in the cloud, and suggests which
//! copy to keep. Nothing goes without a tick, and what goes goes to the Recycle Bin.

use crate::paths::Paths;
use crate::util::{norm_path, now_secs};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// One copy of a file that exists more than once.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DupeFile {
    pub path: PathBuf,
    pub modified: u64,
    /// The copy mom suggests keeping. Every other copy in the group is an extra.
    pub keep: bool,
    /// Which of your folders it's in ("Downloads", "Pictures", ...).
    pub folder: String,
}

/// Files with exactly the same contents.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DupeGroup {
    pub size: u64,
    /// The contents' BLAKE3 hash, in hex.
    pub hash: String,
    /// "photos", "videos", "music", "documents", "archives", "installers" or "other".
    pub kind: String,
    pub files: Vec<DupeFile>,
}

/// A big file nobody has opened (or changed) in a long time.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BigFile {
    pub path: PathBuf,
    pub size: u64,
    /// When it was last opened, as Windows recorded it (seconds since 1970).
    pub accessed: u64,
    pub modified: u64,
    pub kind: String,
    pub folder: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesReport {
    /// Largest waste first.
    pub groups: Vec<DupeGroup>,
    /// What the extra copies take up.
    pub wasted: u64,
    /// Biggest first.
    pub big: Vec<BigFile>,
    pub scanned_files: u64,
    pub scanned_bytes: u64,
    pub roots: Vec<PathBuf>,
    /// False if the scan was stopped before it finished.
    pub complete: bool,
    /// Whether Windows records when files are opened. If it doesn't, "big and unused" goes by
    /// when files were last changed instead.
    pub access_times: bool,
    pub measured_at: u64,
}

pub struct ScanOpts {
    pub roots: Vec<PathBuf>,
    /// Smaller files aren't worth comparing.
    pub min_dupe_size: u64,
    pub big_min_size: u64,
    pub big_unused_days: u64,
}

impl ScanOpts {
    /// Your usual folders, copies of 1 MB and up, and files of 500 MB and up unopened for a year.
    pub fn for_paths(paths: &Paths) -> Self {
        ScanOpts { roots: default_roots(paths), min_dupe_size: 1024 * 1024, big_min_size: 500 * 1024 * 1024, big_unused_days: 365 }
    }
}

/// How far a scan has got, for a progress line.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub files: u64,
    /// Listing: how much the files seen so far add up to. Comparing: how much has been read.
    pub bytes: u64,
    /// "listing" or "comparing".
    pub stage: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    pub path: PathBuf,
    pub why: String,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecycleReport {
    pub recycled: Vec<PathBuf>,
    pub skipped: Vec<Skipped>,
    pub freed: u64,
    pub message: String,
}

/// Folders that are never worth looking in: tools' and projects' own stuff, not yours.
const SKIP_DIRS: &[&str] =
    &["node_modules", ".git", "target", ".venv", "venv", "__pycache__", "appdata", "$recycle.bin", ".next", ".turbo", "site-packages"];

/// A folder holding one of these is a project: its files are code and build output, where
/// copies are on purpose.
const PROJECT_MARKERS: &[&str] = &[
    "package.json",
    "cargo.toml",
    ".git",
    "pyproject.toml",
    "setup.py",
    "go.mod",
    "pubspec.yaml",
    "build.gradle",
    "build.gradle.kts",
    "pom.xml",
    "composer.json",
    "gemfile",
    "cmakelists.txt",
    "makefile",
];

/// Hidden, system, a link, or only in the cloud (reading it would download it).
const SKIP_ATTRS: u32 = 0x2 | 0x4 | 0x400 | 0x1000 | 0x40000 | 0x400000;

/// Should a file or folder with these attributes be left alone?
fn skip_attrs(attrs: u32) -> bool {
    attrs & SKIP_ATTRS != 0
}

/// How much of each end of a file to compare before reading all of it.
const EDGE: u64 = 64 * 1024;

/// Big files listed at most.
const BIG_CAP: usize = 100;

/// Your usual folders: Downloads, Documents, Desktop, Pictures, Videos, Music and OneDrive, where
/// they exist (wherever you moved them). In tests, the same names under the test home.
pub fn default_roots(paths: &Paths) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = known_folders(paths).into_iter().map(|(p, _)| p).collect();
    roots.retain(|r| r.is_dir());
    let mut seen = HashSet::new();
    roots.retain(|r| seen.insert(norm_path(&r.to_string_lossy())));
    roots
}

/// Each usual folder with its name.
fn known_folders(paths: &Paths) -> Vec<(PathBuf, &'static str)> {
    let real = dirs::home_dir().is_some_and(|h| h == paths.home);
    let known = |f: fn() -> Option<PathBuf>, name: &str| (if real { f() } else { None }).unwrap_or_else(|| paths.home.join(name));
    let onedrive = if real {
        crate::util::env_path("OneDrive").or_else(|| crate::util::env_path("OneDriveConsumer")).unwrap_or_else(|| paths.home.join("OneDrive"))
    } else {
        paths.home.join("OneDrive")
    };
    vec![
        (paths.downloads.clone(), "Downloads"),
        (known(dirs::document_dir, "Documents"), "Documents"),
        (known(dirs::desktop_dir, "Desktop"), "Desktop"),
        (known(dirs::picture_dir, "Pictures"), "Pictures"),
        (known(dirs::video_dir, "Videos"), "Videos"),
        (known(dirs::audio_dir, "Music"), "Music"),
        (onedrive, "OneDrive"),
    ]
}

/// A file mom might compare: one of at least the minimum size.
struct Seen {
    path: PathBuf,
    size: u64,
    modified: u64,
    folder: usize,
}

/// Calls the progress callback at most every 200 ms.
struct Ticker<'a> {
    last: Instant,
    progress: &'a dyn Fn(Progress),
}

impl Ticker<'_> {
    fn tick(&mut self, p: Progress) {
        if self.last.elapsed() >= Duration::from_millis(200) {
            self.last = Instant::now();
            (self.progress)(p);
        }
    }
}

/// Looks through `opts.roots` for copies and big unused files. Stops early (with `complete`
/// false) when `cancel` is set. Reading a file to compare it doesn't count as opening it: the
/// time it was last opened stays as it was.
pub fn scan(paths: &Paths, opts: &ScanOpts, cancel: &AtomicBool, progress: &dyn Fn(Progress)) -> FilesReport {
    let now = now_secs();
    let access_times = access_times_recorded();
    let names = folder_names(paths, &opts.roots);
    let mut report = FilesReport { roots: opts.roots.clone(), access_times, measured_at: now, ..Default::default() };
    let mut ticker = Ticker { last: Instant::now(), progress };
    let mut seen: Vec<Seen> = Vec::new();
    let unused_secs = opts.big_unused_days * 86_400;

    // Listing. Roots inside other roots (Documents inside OneDrive) are walked once, as part of
    // the outer one, but their files are still named after the inner one.
    let norm: Vec<String> = opts.roots.iter().map(|r| norm_path(&r.to_string_lossy())).collect();
    let mut stopped = false;
    for (i, root) in opts.roots.iter().enumerate() {
        let inside_another = norm.iter().enumerate().any(|(j, o)| j != i && norm[i].starts_with(&format!("{o}\\")));
        if inside_another || !root.is_dir() {
            continue;
        }
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            if cancel.load(Ordering::Relaxed) {
                stopped = true;
                break;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            let entries: Vec<std::fs::DirEntry> = entries.flatten().collect();
            let is_project = dir != *root
                && entries.iter().any(|e| PROJECT_MARKERS.contains(&e.file_name().to_string_lossy().to_lowercase().as_str()));
            if is_project {
                continue;
            }
            for e in entries {
                let Ok(meta) = e.metadata() else { continue };
                if skip_attrs(attributes(&meta)) || meta.file_type().is_symlink() {
                    continue;
                }
                let name = e.file_name().to_string_lossy().to_lowercase();
                if meta.is_dir() {
                    if !SKIP_DIRS.contains(&name.as_str()) && !name.starts_with('.') {
                        stack.push(e.path());
                    }
                    continue;
                }
                if !meta.is_file() {
                    continue;
                }
                let size = meta.len();
                report.scanned_files += 1;
                report.scanned_bytes += size;
                let modified = secs(meta.modified());
                let path = e.path();
                let folder = folder_of(&names, &path);
                if size >= opts.big_min_size {
                    let accessed = secs(meta.accessed()).max(modified);
                    let last = if access_times { accessed } else { modified };
                    if now.saturating_sub(last) >= unused_secs {
                        report.big.push(BigFile {
                            path: path.clone(),
                            size,
                            accessed,
                            modified,
                            kind: kind_of(&path).into(),
                            folder: names[folder].1.clone(),
                        });
                    }
                }
                if size >= opts.min_dupe_size.max(1) {
                    seen.push(Seen { path, size, modified, folder });
                }
                ticker.tick(Progress { files: report.scanned_files, bytes: report.scanned_bytes, stage: "listing" });
            }
        }
        if stopped {
            break;
        }
    }
    report.big.sort_by(|a, b| b.size.cmp(&a.size));
    report.big.truncate(BIG_CAP);

    // Comparing: same size, then the same first and last 64 KB, then the same everything.
    let mut by_size: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, s) in seen.iter().enumerate() {
        by_size.entry(s.size).or_default().push(i);
    }
    let mut read: u64 = 0;
    let mut groups = Vec::new();
    let mut sizes: Vec<(u64, Vec<usize>)> = by_size.into_iter().filter(|(_, v)| v.len() > 1).collect();
    sizes.sort_by(|a, b| b.0.cmp(&a.0));
    'sizes: for (size, same_size) in sizes {
        let mut by_edges: HashMap<[u8; 32], Vec<usize>> = HashMap::new();
        for i in same_size {
            if stopped || cancel.load(Ordering::Relaxed) {
                stopped = true;
                break 'sizes;
            }
            if let Some(h) = edges_hash(&seen[i].path, size) {
                read += (2 * EDGE).min(size);
                by_edges.entry(h).or_default().push(i);
            }
            ticker.tick(Progress { files: report.scanned_files, bytes: read, stage: "comparing" });
        }
        for (edges, same_edges) in by_edges.into_iter().filter(|(_, v)| v.len() > 1) {
            let mut by_hash: HashMap<[u8; 32], Vec<usize>> = HashMap::new();
            for i in same_edges {
                // Small enough that the edges were all of it.
                let full = if size <= 2 * EDGE {
                    Some(edges)
                } else {
                    match full_hash(&seen[i].path, cancel, &mut |n| {
                        read += n;
                        ticker.tick(Progress { files: report.scanned_files, bytes: read, stage: "comparing" });
                    }) {
                        Hashed::Done(h) => Some(h),
                        Hashed::Failed => None,
                        Hashed::Stopped => {
                            stopped = true;
                            break 'sizes;
                        }
                    }
                };
                if let Some(h) = full {
                    by_hash.entry(h).or_default().push(i);
                }
            }
            for (hash, copies) in by_hash.into_iter().filter(|(_, v)| v.len() > 1) {
                groups.push(group(&seen, &names, size, &hash, &copies));
            }
        }
    }
    groups.sort_by(|a, b| waste(b).cmp(&waste(a)).then_with(|| a.hash.cmp(&b.hash)));
    report.wasted = groups.iter().map(waste).sum();
    report.groups = groups;
    report.complete = !stopped;
    progress(Progress { files: report.scanned_files, bytes: read, stage: "comparing" });
    report
}

/// What a group's extra copies take up.
fn waste(g: &DupeGroup) -> u64 {
    g.size * g.files.iter().filter(|f| !f.keep).count() as u64
}

/// A group of copies, with the one to keep picked: one in Documents, Pictures, Videos or Music
/// over one elsewhere, over the Desktop, over Downloads; then the oldest; then the shortest path.
fn group(seen: &[Seen], names: &[(String, String)], size: u64, hash: &[u8; 32], copies: &[usize]) -> DupeGroup {
    let rank = |folder: &str| match folder {
        "Documents" | "Pictures" | "Videos" | "Music" => 0,
        "Desktop" => 2,
        "Downloads" => 3,
        _ => 1,
    };
    let mut files: Vec<DupeFile> = copies
        .iter()
        .map(|&i| DupeFile { path: seen[i].path.clone(), modified: seen[i].modified, keep: false, folder: names[seen[i].folder].1.clone() })
        .collect();
    files.sort_by(|a, b| {
        rank(&a.folder)
            .cmp(&rank(&b.folder))
            .then(a.modified.cmp(&b.modified))
            .then(a.path.as_os_str().len().cmp(&b.path.as_os_str().len()))
            .then(a.path.cmp(&b.path))
    });
    files[0].keep = true;
    DupeGroup { size, hash: hex(hash), kind: kind_of(&files[0].path).into(), files }
}

/// Each root's normalized path and name: the usual folders by their own names, others by the
/// last part of their path.
fn folder_names(paths: &Paths, roots: &[PathBuf]) -> Vec<(String, String)> {
    let known: Vec<(String, &str)> = known_folders(paths).into_iter().map(|(p, n)| (norm_path(&p.to_string_lossy()), n)).collect();
    let mut names: Vec<(String, String)> = roots
        .iter()
        .map(|r| {
            let n = norm_path(&r.to_string_lossy());
            let name = known
                .iter()
                .find(|(k, _)| *k == n)
                .map(|(_, name)| name.to_string())
                .unwrap_or_else(|| r.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| n.clone()));
            (n, name)
        })
        .collect();
    names.push((String::new(), "Other".into()));
    names
}

/// Which root a file is in: the deepest one that holds it.
fn folder_of(names: &[(String, String)], path: &Path) -> usize {
    let p = norm_path(&path.to_string_lossy());
    names
        .iter()
        .enumerate()
        .filter(|(_, (root, _))| !root.is_empty() && p.starts_with(&format!("{root}\\")))
        .max_by_key(|(_, (root, _))| root.len())
        .map(|(i, _)| i)
        .unwrap_or(names.len() - 1)
}

fn kind_of(path: &Path) -> &'static str {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "heic" | "heif" | "webp" | "bmp" | "tif" | "tiff" | "raw" | "cr2" | "cr3" | "nef" | "arw" | "dng"
        | "psd" | "avif" => "photos",
        "mp4" | "mov" | "mkv" | "avi" | "wmv" | "webm" | "m4v" | "mts" | "m2ts" | "3gp" | "flv" | "mpg" | "mpeg" => "videos",
        "mp3" | "flac" | "wav" | "m4a" | "aac" | "ogg" | "opus" | "wma" | "aiff" | "aif" | "alac" => "music",
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp" | "txt" | "rtf" | "epub" | "md" | "csv"
        | "key" | "pages" | "numbers" => "documents",
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "iso" | "dmg" | "img" | "zst" => "archives",
        "exe" | "msi" | "msix" | "msixbundle" | "appx" | "appxbundle" | "apk" => "installers",
        _ => "other",
    }
}

fn secs(t: std::io::Result<SystemTime>) -> u64 {
    t.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

fn hex(h: &[u8; 32]) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(windows)]
fn attributes(meta: &std::fs::Metadata) -> u32 {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes()
}

#[cfg(not(windows))]
fn attributes(_: &std::fs::Metadata) -> u32 {
    0
}

/// Does Windows record when files are opened? It's the NtfsDisableLastAccessUpdate setting:
/// the low bit set means it doesn't (0x80000000 and 0x80000001 are the "system managed" kinds).
/// When the setting can't be read, mom assumes it doesn't, and goes by when files changed.
fn access_times_recorded() -> bool {
    #[cfg(windows)]
    {
        use crate::health::reg::{dword_value, Hive};
        dword_value(Hive::LocalMachine, r"SYSTEM\CurrentControlSet\Control\FileSystem", "NtfsDisableLastAccessUpdate").is_some_and(|v| v & 1 == 0)
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// Opens a file to read without Windows noting that it was opened, so comparing files doesn't
/// make them look used. Falls back to a plain open where that isn't allowed.
fn open_quietly(path: &Path) -> std::io::Result<std::fs::File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::FILETIME;
        // GENERIC_READ and FILE_WRITE_ATTRIBUTES (needed to say "don't note access").
        if let Ok(f) = std::fs::OpenOptions::new().access_mode(0x8000_0000 | 0x100).open(path) {
            let keep = FILETIME { dwLowDateTime: u32::MAX, dwHighDateTime: u32::MAX };
            // SAFETY: the handle is the open file's; a last-access time of all ones tells Windows
            // not to update it for this handle.
            unsafe { windows_sys::Win32::Storage::FileSystem::SetFileTime(f.as_raw_handle(), std::ptr::null(), &keep, std::ptr::null()) };
            return Ok(f);
        }
    }
    std::fs::File::open(path)
}

/// BLAKE3 of the size and the first and last 64 KB.
fn edges_hash(path: &Path, size: u64) -> Option<[u8; 32]> {
    let mut f = open_quietly(path).ok()?;
    if size <= 2 * EDGE {
        // Small enough to read whole: this is its full hash.
        let mut all = Vec::with_capacity(size as usize);
        f.read_to_end(&mut all).ok()?;
        return (all.len() as u64 == size).then(|| *blake3::hash(&all).as_bytes());
    }
    let mut h = blake3::Hasher::new();
    h.update(&size.to_le_bytes());
    let mut buf = vec![0u8; EDGE as usize];
    f.read_exact(&mut buf).ok()?;
    h.update(&buf);
    f.seek(SeekFrom::Start(size - EDGE)).ok()?;
    f.read_exact(&mut buf).ok()?;
    h.update(&buf);
    Some(*h.finalize().as_bytes())
}

enum Hashed {
    Done([u8; 32]),
    Failed,
    Stopped,
}

/// BLAKE3 of the whole file, read a megabyte at a time, telling `read` how much each time.
fn full_hash(path: &Path, cancel: &AtomicBool, read: &mut dyn FnMut(u64)) -> Hashed {
    let Ok(mut f) = open_quietly(path) else { return Hashed::Failed };
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Hashed::Stopped;
        }
        match f.read(&mut buf) {
            Ok(0) => return Hashed::Done(*h.finalize().as_bytes()),
            Ok(n) => {
                h.update(&buf[..n]);
                read(n as u64);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Hashed::Failed,
        }
    }
}

// ------------------------------------------------------------------ letting files go

/// Sends ticked files to the Recycle Bin, checked against the last scan first: a copy only if
/// it still has the same contents and another copy with them stays (one not ticked too), a big
/// file only if it's unchanged. Never deletes anything for good. The saved scan is updated.
pub fn recycle(paths: &Paths, files: &[PathBuf]) -> RecycleReport {
    recycle_with(paths, files, &mut |p| ::trash::delete(p).is_ok())
}

fn recycle_with(paths: &Paths, files: &[PathBuf], remove: &mut dyn FnMut(&Path) -> bool) -> RecycleReport {
    let mut report = load_saved(paths).unwrap_or_default();
    let mut r = RecycleReport::default();
    let ticked: HashSet<String> = files.iter().map(|f| norm_path(&f.to_string_lossy())).collect();
    let never_stopped = AtomicBool::new(false);
    // Groups whose staying copy has been checked this time: the copy that stays, or None.
    let mut checked: HashMap<usize, bool> = HashMap::new();
    let mut gone: HashSet<String> = HashSet::new();
    for file in files {
        let key = norm_path(&file.to_string_lossy());
        let skip = |why: &str| Skipped { path: file.clone(), why: why.into() };
        let in_group = report.groups.iter().enumerate().find_map(|(gi, g)| {
            g.files.iter().find(|f| norm_path(&f.path.to_string_lossy()) == key).map(|f| (gi, f.modified))
        });
        if let Some((gi, modified)) = in_group {
            let g = &report.groups[gi];
            let same = |p: &Path| matches!(full_hash(p, &never_stopped, &mut |_| {}), Hashed::Done(h) if hex(&h) == g.hash);
            if !unchanged(file, g.size, modified) || !same(file) {
                r.skipped.push(skip("It changed since mom looked, so it may not be a copy any more."));
                continue;
            }
            let stays = *checked.entry(gi).or_insert_with(|| {
                g.files
                    .iter()
                    .filter(|f| !ticked.contains(&norm_path(&f.path.to_string_lossy())))
                    .any(|f| std::fs::metadata(&f.path).is_ok_and(|m| m.len() == g.size) && same(&f.path))
            });
            if !stays {
                r.skipped.push(skip(
                    "No other copy would be left (every copy was ticked, or the others changed or are gone), so this one stays.",
                ));
                continue;
            }
            if remove(file) {
                r.freed += g.size;
                r.recycled.push(file.clone());
                gone.insert(key);
            } else {
                r.skipped.push(skip("Windows couldn't move it to the Recycle Bin (it may be open)."));
            }
            continue;
        }
        if let Some(b) = report.big.iter().find(|b| norm_path(&b.path.to_string_lossy()) == key) {
            if !unchanged(file, b.size, b.modified) {
                r.skipped.push(skip("It changed since mom looked."));
            } else if remove(file) {
                r.freed += b.size;
                r.recycled.push(file.clone());
                gone.insert(key);
            } else {
                r.skipped.push(skip("Windows couldn't move it to the Recycle Bin (it may be open)."));
            }
            continue;
        }
        r.skipped.push(skip("Mom didn't find this one in her last look, so she left it."));
    }
    // What went is no longer a copy or a big file.
    if !gone.is_empty() {
        let went = |p: &Path| gone.contains(&norm_path(&p.to_string_lossy()));
        for g in &mut report.groups {
            g.files.retain(|f| !went(&f.path));
            if !g.files.iter().any(|f| f.keep) {
                if let Some(f) = g.files.first_mut() {
                    f.keep = true;
                }
            }
        }
        report.groups.retain(|g| g.files.len() > 1);
        report.wasted = report.groups.iter().map(waste).sum();
        report.big.retain(|b| !went(&b.path));
        save(paths, &report);
    }
    r.message = match (r.recycled.len(), r.skipped.len()) {
        (0, 0) => "Nothing was ticked.".into(),
        (0, s) => format!("Nothing went: {} stayed.", crate::util::plural(s as u64, "file")),
        (n, 0) => format!("Moved {} ({}) to the Recycle Bin.", crate::util::plural(n as u64, "file"), crate::util::human_bytes(r.freed)),
        (n, s) => format!(
            "Moved {} ({}) to the Recycle Bin; {} stayed.",
            crate::util::plural(n as u64, "file"),
            crate::util::human_bytes(r.freed),
            crate::util::plural(s as u64, "file")
        ),
    };
    r
}

/// Is the file still there, the same size and not changed since?
fn unchanged(path: &Path, size: u64, modified: u64) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.len() == size && secs(m.modified()) == modified)
}

fn saved_file(paths: &Paths) -> PathBuf {
    paths.state_dir().join("files.json")
}

/// Keeps the last scan, so the widget can show it straight away next time.
pub fn save(paths: &Paths, report: &FilesReport) {
    if let Ok(json) = serde_json::to_string(report) {
        let _ = crate::util::write_atomic(&saved_file(paths), &json);
    }
}

pub fn load_saved(paths: &Paths) -> Option<FilesReport> {
    std::fs::read_to_string(saved_file(paths)).ok().and_then(|s| serde_json::from_str(&s).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::FileTimes;

    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("mom-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    fn put(path: &Path, contents: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn age(path: &Path, days: u64) {
        let t = SystemTime::now() - Duration::from_secs(days * 86_400);
        let f = std::fs::File::options().write(true).open(path).unwrap();
        f.set_times(FileTimes::new().set_accessed(t).set_modified(t)).unwrap();
    }

    fn opts(paths: &Paths) -> ScanOpts {
        ScanOpts { roots: default_roots(paths), min_dupe_size: 1000, big_min_size: 100_000, big_unused_days: 365 }
    }

    fn look(paths: &Paths) -> FilesReport {
        scan(paths, &opts(paths), &AtomicBool::new(false), &|_| {})
    }

    /// Made-up contents of a given size, different for each seed.
    fn stuff(len: usize, seed: u8) -> Vec<u8> {
        (0..len).map(|i| (i as u32).wrapping_mul(31).wrapping_add(seed as u32) as u8).collect()
    }

    #[test]
    fn copies_across_folders_are_found_and_the_one_in_pictures_is_kept() {
        let root = fixture("dupes");
        let paths = Paths::under(&root);
        let photo = stuff(200_000, 1);
        put(&paths.downloads.join("beach trip.jpg"), &photo);
        put(&paths.home.join("Pictures").join("Summer").join("beach trip.jpg"), &photo);
        put(&paths.home.join("Desktop").join("beach (1).jpg"), &photo);
        // Same size, different contents: not a copy.
        put(&paths.downloads.join("other.jpg"), &stuff(200_000, 2));
        let r = look(&paths);
        assert!(r.complete);
        assert_eq!(r.groups.len(), 1, "{:?}", r.groups);
        let g = &r.groups[0];
        assert_eq!((g.size, g.kind.as_str(), g.files.len()), (200_000, "photos", 3));
        let kept: Vec<&DupeFile> = g.files.iter().filter(|f| f.keep).collect();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].folder, "Pictures");
        assert_eq!(r.wasted, 400_000);
        assert_eq!(r.scanned_files, 4);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn files_that_only_share_their_ends_are_not_copies() {
        let root = fixture("edges");
        let paths = Paths::under(&root);
        let a = stuff(400_000, 7);
        let mut b = a.clone();
        b[200_000] ^= 0xff; // the middle differs, the first and last 64 KB don't
        put(&paths.home.join("Documents").join("report v1.pdf"), &a);
        put(&paths.home.join("Documents").join("report v2.pdf"), &b);
        let r = look(&paths);
        assert!(r.groups.is_empty(), "{:?}", r.groups);
        // A true copy is still found.
        put(&paths.home.join("Documents").join("report v1 copy.pdf"), &a);
        let r = look(&paths);
        assert_eq!(r.groups.len(), 1);
        assert_eq!(r.groups[0].files.len(), 2);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn projects_hidden_files_and_small_files_are_left_out() {
        let root = fixture("skips");
        let paths = Paths::under(&root);
        let data = stuff(50_000, 3);
        put(&paths.downloads.join("asset.png"), &data);
        // Inside a project: copies there are on purpose.
        put(&paths.home.join("Documents").join("site").join("package.json"), b"{}");
        put(&paths.home.join("Documents").join("site").join("public").join("asset.png"), &data);
        // Tool folders.
        put(&paths.home.join("Documents").join(".cache").join("asset.png"), &data);
        put(&paths.home.join("Documents").join("node_modules").join("asset.png"), &data);
        // Too small to bother with.
        put(&paths.downloads.join("tiny.txt"), b"hello");
        put(&paths.home.join("Desktop").join("tiny.txt"), b"hello");
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use std::io::Write;
            let hidden = paths.home.join("Desktop").join("asset.png");
            let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).attributes(0x2).open(&hidden).unwrap();
            f.write_all(&data).unwrap();
        }
        let r = look(&paths);
        assert!(r.groups.is_empty(), "{:?}", r.groups);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cloud_only_hidden_and_linked_files_are_never_read() {
        assert!(skip_attrs(0x400000), "only in the cloud: reading it would download it");
        assert!(skip_attrs(0x40000) && skip_attrs(0x1000) && skip_attrs(0x400) && skip_attrs(0x2) && skip_attrs(0x4));
        assert!(!skip_attrs(0x20) && !skip_attrs(0x80) && !skip_attrs(0x1), "archive, normal and read-only files are fine");
    }

    #[cfg(windows)]
    #[test]
    fn links_are_not_followed() {
        let root = fixture("links");
        let paths = Paths::under(&root);
        let data = stuff(30_000, 5);
        put(&paths.downloads.join("song.mp3"), &data);
        let elsewhere = root.join("elsewhere");
        put(&elsewhere.join("song.mp3"), &data);
        std::fs::create_dir_all(paths.home.join("Music")).unwrap();
        junction::create(&elsewhere, paths.home.join("Music").join("linked")).unwrap();
        let r = look(&paths);
        assert!(r.groups.is_empty(), "{:?}", r.groups);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn big_files_nobody_opened_in_a_year_are_listed_and_comparing_doesnt_count_as_opening() {
        let root = fixture("big");
        let paths = Paths::under(&root);
        let movie = stuff(150_000, 9);
        put(&paths.home.join("Videos").join("old movie.mkv"), &movie);
        put(&paths.home.join("Videos").join("old movie copy.mkv"), &movie);
        put(&paths.home.join("Videos").join("new movie.mkv"), &stuff(160_000, 4));
        put(&paths.home.join("Videos").join("small.mkv"), &stuff(5_000, 4));
        age(&paths.home.join("Videos").join("old movie.mkv"), 400);
        age(&paths.home.join("Videos").join("old movie copy.mkv"), 400);
        age(&paths.home.join("Videos").join("small.mkv"), 400);
        let r = look(&paths);
        let big: Vec<String> = r.big.iter().map(|b| b.path.file_name().unwrap().to_string_lossy().to_string()).collect();
        if r.access_times {
            assert_eq!(big.len(), 2, "{big:?}");
            assert!(big.iter().all(|b| b.starts_with("old movie")), "{big:?}");
        }
        assert_eq!(r.groups.len(), 1, "the two old movies are copies");
        assert_eq!(r.big[0].kind, "videos");
        // Reading them to compare didn't make them look opened.
        let r = look(&paths);
        if r.access_times {
            assert_eq!(r.big.len(), 2, "{:?}", r.big);
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_last_copy_never_goes_and_changed_files_stay() {
        let root = fixture("recycle");
        let paths = Paths::under(&root);
        let a = stuff(20_000, 11);
        let keep = paths.home.join("Documents").join("notes.pdf");
        let extra = paths.downloads.join("notes.pdf");
        let extra2 = paths.home.join("Desktop").join("notes (2).pdf");
        put(&keep, &a);
        put(&extra, &a);
        put(&extra2, &a);
        let report = look(&paths);
        save(&paths, &report);
        let mut removed = Vec::new();
        let mut fake = |p: &Path| {
            removed.push(p.to_path_buf());
            std::fs::remove_file(p).is_ok()
        };
        // Every copy ticked: none goes.
        let r = recycle_with(&paths, &[keep.clone(), extra.clone(), extra2.clone()], &mut fake);
        assert!(r.recycled.is_empty() && r.skipped.len() == 3, "{r:?}");
        assert!(keep.exists() && extra.exists() && extra2.exists());
        // One extra changed since: it stays; the other goes, and the saved scan drops it.
        std::fs::write(&extra2, stuff(20_000, 12)).unwrap();
        let r = recycle_with(&paths, &[extra.clone(), extra2.clone()], &mut fake);
        assert_eq!(r.recycled, vec![extra.clone()], "{r:?}");
        assert_eq!(r.freed, 20_000);
        assert!(r.skipped[0].why.contains("changed"));
        assert!(!extra.exists() && keep.exists() && extra2.exists());
        let saved = load_saved(&paths).unwrap();
        assert_eq!(saved.groups.len(), 1);
        assert_eq!(saved.groups[0].files.len(), 2, "{:?}", saved.groups);
        // With the staying copy changed, the last unchanged one stays too.
        std::fs::write(&keep, stuff(20_000, 13)).unwrap();
        let r = recycle_with(&paths, std::slice::from_ref(&extra2), &mut fake);
        assert!(r.recycled.is_empty(), "{r:?}");
        // Something mom never found isn't touched.
        let r = recycle_with(&paths, &[root.join("whatever.txt")], &mut fake);
        assert!(r.recycled.is_empty() && r.skipped[0].why.contains("didn't find"));
        assert_eq!(removed, vec![extra.clone()]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_stopped_scan_says_so() {
        let root = fixture("stop");
        let paths = Paths::under(&root);
        put(&paths.downloads.join("a.zip"), &stuff(10_000, 1));
        let r = scan(&paths, &opts(&paths), &AtomicBool::new(true), &|_| {});
        assert!(!r.complete);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
