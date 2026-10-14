//! Files of your own that only take up room: exact copies of the same file, and big files nobody
//! has opened in a year. Mom looks in Downloads, Documents, Desktop, Pictures, Videos, Music and
//! OneDrive, and any drive or folder you add; never inside projects, links, files that are only in
//! the cloud, or Windows, programs and game libraries. She suggests which copy
//! to keep. Nothing goes without a tick; what goes goes to the Recycle Bin, and only after it's
//! checked byte for byte against the copy that stays.

use crate::paths::Paths;
use crate::util::{norm_path, now_secs};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod names;

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
    /// You said to keep it (Settings, "Always kept"), so it can't be ticked. Worked out each
    /// time the scan is shown (`mark_kept`), never saved.
    #[serde(default, skip_deserializing)]
    pub kept: bool,
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
    /// Latest recorded access/write time (seconds since 1970), not proof of a human opening it.
    pub accessed: u64,
    pub modified: u64,
    /// Newest creation/modification evidence when this item was measured.
    #[serde(default)]
    pub recency: u64,
    pub kind: String,
    pub folder: String,
    /// You said to keep it (see `DupeFile.kept`).
    #[serde(default, skip_deserializing)]
    pub kept: bool,
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
    /// Whether recorded access times are enabled on the local NTFS volumes being scanned.
    /// Other volumes use creation/modification evidence instead.
    pub access_times: bool,
    pub measured_at: u64,
    /// Older saved scans did not account for creation time or unknown timestamps.
    #[serde(default)]
    age_evidence_version: u32,
}

const AGE_EVIDENCE_VERSION: u32 = 1;

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

    /// The same, in the places you chose: usual folders not switched off, and what you added.
    pub fn for_config(paths: &Paths, cfg: &crate::config::FilesConfig) -> Self {
        ScanOpts { roots: chosen_roots(paths, cfg), ..Self::for_paths(paths) }
    }
}

/// A place the Files scan can look, as the widget lists it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub path: String,
    pub name: String,
    /// "folder" (one of your usual folders), "drive" or "added".
    pub kind: &'static str,
    pub on: bool,
    /// Drives: size and free space, bytes.
    pub total: u64,
    pub free: u64,
}

fn same_path(a: &Path, b: &Path) -> bool {
    norm_path(&a.to_string_lossy()) == norm_path(&b.to_string_lossy())
}

/// "D:\\" is a drive's root.
fn is_drive_root(p: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        let mut parts = p.components();
        matches!(parts.next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
            && matches!(parts.next(), Some(Component::RootDir)) && parts.next().is_none()
    }
    #[cfg(not(windows))]
    { p.has_root() && p.parent().is_none() }
}

/// Every place mom can look: your usual folders, each drive, and the folders you added.
pub fn places(paths: &Paths, cfg: &crate::config::FilesConfig) -> Vec<Place> {
    let mut out: Vec<Place> = known_folders(paths)
        .into_iter()
        .filter(|(p, _)| p.is_dir())
        .map(|(p, name)| Place { path: p.to_string_lossy().to_string(), name: name.into(), kind: "folder", on: !cfg.skip.iter().any(|s| s == name), total: 0, free: 0 })
        .collect();
    let mut seen = HashSet::new();
    out.retain(|p| seen.insert(norm_path(&p.path)));
    for (root, label, total, free) in drives() {
        let on = cfg.places.iter().any(|p| same_path(p, Path::new(&root)));
        out.push(Place { name: format!("{} ({label})", root.trim_end_matches('\\')), path: root, kind: "drive", on, total, free });
    }
    for p in &cfg.places {
        if !is_drive_root(p) && p.is_dir() {
            out.push(Place { path: p.to_string_lossy().to_string(), name: p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(), kind: "added", on: true, total: 0, free: 0 });
        }
    }
    out
}

/// Where the next scan looks.
pub fn chosen_roots(paths: &Paths, cfg: &crate::config::FilesConfig) -> Vec<PathBuf> {
    places(paths, cfg).into_iter().filter(|p| p.on).map(|p| PathBuf::from(p.path)).collect()
}

/// Fixed drives with their label, size and free space.
fn drives() -> Vec<(String, String, u64, u64)> {
    crate::health::drives().into_iter().map(|d| (d.root, d.label, d.total, d.free)).collect()
}

/// Switches a place on or off: a usual folder by name, a drive or folder by path. A folder
/// that's added must exist and mustn't be Windows' or programs' own.
pub fn set_place(paths: &Paths, cfg: &mut crate::config::FilesConfig, path: &Path, on: bool) -> Result<(), String> {
    if let Some((_, name)) = known_folders(paths).into_iter().find(|(p, _)| same_path(p, path)) {
        cfg.skip.retain(|s| s != name);
        if !on {
            cfg.skip.push(name.to_string());
        }
        return Ok(());
    }
    cfg.places.retain(|p| !same_path(p, path));
    if !on {
        return Ok(());
    }
    if !path.is_dir() {
        return Err(format!("{} isn't a folder mom can look in.", path.display()));
    }
    let l = norm_path(&path.to_string_lossy());
    let windows = norm_path(&std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()));
    let off_limits = [windows, norm_path(r"C:\Program Files"), norm_path(r"C:\Program Files (x86)"), norm_path(r"C:\ProgramData")];
    if off_limits.iter().any(|o| l == *o || l.starts_with(&format!("{o}\\"))) {
        return Err("That's Windows' or your programs' own folder. Copies in there are on purpose, so mom doesn't look.".into());
    }
    cfg.places.push(path.to_path_buf());
    Ok(())
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
const SKIP_DIRS: &[&str] = &[
    "node_modules", ".git", "target", ".venv", "venv", "__pycache__", "appdata", "$recycle.bin", ".next", ".turbo", "site-packages", "steamapps",
];

/// At the top of any drive: Windows, programs, game libraries and the system's own folders.
/// Their copies are on purpose (every game ships the same libraries).
const SKIP_AT_DRIVE_ROOT: &[&str] = &[
    "windows", "program files", "program files (x86)", "programdata", "system volume information", "recovery", "perflogs",
    "$windows.~bt", "$windows.~ws", "$windows.~tmp", "$getcurrent", "$sysreset", "$winreagent", "windows10upgrade", "wudownloadcache", "windows.old",
    "windowsapps", "msocache", "xboxgames", "steamlibrary", "steam", "epic games",
    "riot games", "battle.net", "gog games", "ea games", "origin games", "ubisoft", "msi", "drivers", "intel", "amd", "nvidia",
];

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

fn protected_directory_name(path: &Path) -> bool {
    let Some(name) = path.file_name() else { return false };
    let name = name.to_string_lossy().to_lowercase();
    name.starts_with('.') || SKIP_DIRS.contains(&name.as_str())
        || (path.parent().is_some_and(is_drive_root) && SKIP_AT_DRIVE_ROOT.contains(&name.as_str()))
}

/// A chosen subfolder cannot bypass the exclusions applied when its parent is scanned.
/// Recheck the same boundary before recycling, including newly hidden/linked ancestors.
fn directory_boundary(path: &Path) -> bool {
    if !path.is_absolute() { return true; }
    for (depth, dir) in path.ancestors().enumerate() {
        if depth >= 256 || protected_directory_name(dir) { return true; }
        let Ok(meta) = std::fs::symlink_metadata(dir) else { return true };
        if !meta.is_dir() || meta.file_type().is_symlink() { return true; }
        // Windows marks volume roots Hidden|System. That is not a reason to skip the volume.
        let attrs = if is_drive_root(dir) { attributes(&meta) & !(0x2 | 0x4) } else { attributes(&meta) };
        if skip_attrs(attrs) { return true; }
    }
    false
}

fn written_path_key(path: &Path) -> String {
    let text = path.to_string_lossy();
    norm_path(text.strip_prefix(r"\\?\").unwrap_or(&text))
}

/// Resolve the root only after checking its written ancestors for links/protected folders.
/// Short names and trailing-dot aliases must not hide a protected directory's real name.
fn scan_root_key(root: &Path) -> Option<String> {
    if directory_boundary(root) || project_ancestor(root, true) { return None; }
    let resolved = std::fs::canonicalize(root).ok()?;
    let key = written_path_key(root);
    (key == written_path_key(&resolved)).then_some(key)
}

/// An explicitly added project (or one of its subfolders) has the same exclusions as a
/// project discovered under Documents. Unknown marker metadata is not evidence of absence.
fn project_ancestor(path: &Path, directory: bool) -> bool {
    let ancestors = if directory { Some(path) } else { path.parent() };
    let Some(start) = ancestors else { return true };
    for (depth, dir) in start.ancestors().enumerate() {
        if depth >= 256 { return true; }
        for name in PROJECT_MARKERS {
            match std::fs::symlink_metadata(dir.join(name)) {
                Ok(_) => return true,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
                Err(_) => return true,
            }
        }
    }
    false
}

fn file_path_is_safe(path: &Path) -> bool {
    !crate::browser_data::protects_path(path)
        && path.parent().is_some_and(|parent| !directory_boundary(parent))
        && !project_ancestor(path, false)
        && std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink() && !skip_attrs(attributes(&m)))
}

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
    let access_times = access_times_recorded(&opts.roots);
    let names = folder_names(paths, &opts.roots);
    let mut report = FilesReport { roots: opts.roots.clone(), access_times, measured_at: now, age_evidence_version: AGE_EVIDENCE_VERSION, ..Default::default() };
    let mut ticker = Ticker { last: Instant::now(), progress };
    let mut seen: Vec<Seen> = Vec::new();
    let mut identities = HashSet::new();
    let unused_secs = opts.big_unused_days * 86_400;

    // Listing. Roots inside other roots (Documents inside OneDrive) are walked once, as part of
    // the outer one, but their files are still named after the inner one.
    let norm: Vec<Option<String>> = opts.roots.iter().map(|r| scan_root_key(r)).collect();
    let mut stopped = false;
    for (i, root) in opts.roots.iter().enumerate() {
        let Some(key) = &norm[i] else { continue };
        let inside_another = norm.iter().enumerate().any(|(j, o)| j != i && o.as_ref().is_some_and(|o| key.starts_with(&format!("{o}\\"))));
        if inside_another || norm[..i].iter().flatten().any(|prior| prior == key) {
            continue;
        }
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            if cancel.load(Ordering::Relaxed) {
                stopped = true;
                break;
            }
            // Profile contents are neither duplicate-cleanup candidates nor "unused" files.
            // Check once per directory; file names below are screened without extra stat calls.
            if crate::browser_data::protects_path(&dir) {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            let Ok(entries) = entries.collect::<Result<Vec<std::fs::DirEntry>, _>>() else { continue };
            let is_project = entries.iter().any(|e| PROJECT_MARKERS.contains(&e.file_name().to_string_lossy().to_lowercase().as_str()));
            if is_project {
                continue;
            }
            for e in entries {
                let Ok(meta) = e.metadata() else { continue };
                if skip_attrs(attributes(&meta)) || meta.file_type().is_symlink() {
                    continue;
                }
                if meta.is_dir() {
                    if !protected_directory_name(&e.path()) {
                        stack.push(e.path());
                    }
                    continue;
                }
                if !meta.is_file() || crate::browser_data::sensitive_entry(&e.file_name(), false) {
                    continue;
                }
                let size = meta.len();
                report.scanned_files += 1;
                report.scanned_bytes += size;
                let Some(modified) = secs(meta.modified()) else { continue };
                let path = e.path();
                if size >= opts.min_dupe_size.max(1) || size >= opts.big_min_size {
                    // Multiple names for one filesystem object are not reclaimable copies.
                    // Named streams can contain distinct data that the main-file hash misses.
                    let Some(identity) = open_quietly(&path).ok().and_then(|f| ordinary_file(&f)) else { continue };
                    if !identities.insert(identity) { continue; }
                }
                let folder = folder_of(&names, &path);
                if size >= opts.big_min_size {
                    let accessed = secs(meta.accessed());
                    let recency = crate::trash::cleanup_recency(&meta);
                    let last = recency.and_then(|r| if access_times { accessed.map(|a| a.max(r)) } else { Some(r) });
                    if last.is_some_and(|last| now.saturating_sub(last) >= unused_secs) {
                        report.big.push(BigFile {
                            path: path.clone(),
                            size,
                            accessed: accessed.unwrap_or(modified).max(modified),
                            modified,
                            recency: recency.unwrap(),
                            kind: kind_of(&path).into(),
                            folder: names[folder].1.clone(),
                            kept: false,
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
        .map(|&i| DupeFile { path: seen[i].path.clone(), modified: seen[i].modified, keep: false, folder: names[seen[i].folder].1.clone(), kept: false })
        .collect();
    files.sort_by(|a, b| {
        rank(&a.folder)
            .cmp(&rank(&b.folder))
            .then(a.modified.cmp(&b.modified))
            .then(a.path.as_os_str().len().cmp(&b.path.as_os_str().len()))
            .then(a.path.cmp(&b.path))
    });
    let preferred = names::preferred_version(&files.iter().map(|f| f.path.as_path()).collect::<Vec<_>>()).unwrap_or(0);
    files[preferred].keep = true;
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
                .unwrap_or_else(|| r.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| format!("{} drive", r.to_string_lossy().trim_end_matches('\\'))));
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

fn secs(t: std::io::Result<SystemTime>) -> Option<u64> {
    t.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).filter(|t| *t > 0)
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

/// Local NTFS policy cannot establish access-time behavior on a remote or other filesystem.
/// A mixed scan falls back to creation/modification evidence for every root.
fn access_times_recorded(roots: &[PathBuf]) -> bool {
    #[cfg(windows)]
    {
        use crate::health::reg::{dword_value, Hive};
        use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumeInformationW, GetVolumePathNameW};
        if !dword_value(Hive::LocalMachine, r"SYSTEM\CurrentControlSet\Control\FileSystem", "NtfsDisableLastAccessUpdate").is_some_and(|v| v & 1 == 0) {
            return false;
        }
        roots.iter().all(|root| {
            use std::os::windows::ffi::OsStrExt;
            let path: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
            let mut volume = vec![0u16; 32_768];
            let mut filesystem = [0u16; 32];
            // SAFETY: input paths are NUL-terminated; both output buffers have the declared sizes.
            unsafe {
                if GetVolumePathNameW(path.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) == 0
                    || GetDriveTypeW(volume.as_ptr()) != 3 // DRIVE_FIXED; remote policy is unknown
                    || GetVolumeInformationW(volume.as_ptr(), std::ptr::null_mut(), 0, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), filesystem.as_mut_ptr(), filesystem.len() as u32) == 0
                {
                    return false;
                }
            }
            let len = filesystem.iter().position(|c| *c == 0).unwrap_or(filesystem.len());
            String::from_utf16_lossy(&filesystem[..len]).eq_ignore_ascii_case("NTFS")
        })
    }
    #[cfg(not(windows))]
    {
        let _ = roots;
        false // mount options such as noatime are not established here
    }
}

/// Opens a file to read without Windows noting that it was opened, so comparing files doesn't
/// manufacture recent access dates. If suppression is unavailable, leave that file unexamined.
fn open_quietly(path: &Path) -> std::io::Result<std::fs::File> {
    open_for_comparison(path, None)
}

fn open_for_comparison(path: &Path, keeper: Option<bool>) -> std::io::Result<std::fs::File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::FILETIME;
        // GENERIC_READ and FILE_WRITE_ATTRIBUTES (needed to say "don't note access").
        let mut options = std::fs::OpenOptions::new();
        options.access_mode(0x8000_0000 | 0x100).custom_flags(0x0020_0000); // OPEN_REPARSE_POINT
        if let Some(keeper) = keeper {
            // A keeper cannot be written, renamed or deleted until recycling finishes. The
            // source allows deletion so Windows can recycle it, but does not allow writes.
            options.share_mode(if keeper { 0x1 } else { 0x1 | 0x4 });
        }
        let f = options.open(path)?;
        let keep = FILETIME { dwLowDateTime: u32::MAX, dwHighDateTime: u32::MAX };
        // SAFETY: the handle is the open file's; a last-access time of all ones tells Windows
        // not to update it for this handle.
        if unsafe { windows_sys::Win32::Storage::FileSystem::SetFileTime(f.as_raw_handle(), std::ptr::null(), &keep, std::ptr::null()) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(f)
    }
    #[cfg(not(windows))]
    { let _ = keeper; std::fs::File::open(path) }
}

/// Only an ordinary single-link file with no extra data streams can be called an independent
/// byte-for-byte copy. Failure to inspect either property keeps it out of cleanup.
fn ordinary_file(file: &std::fs::File) -> Option<(u64, u64)> {
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.file_type().is_symlink() || skip_attrs(attributes(&meta)) { return None; }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, GetFileInformationByHandleEx, BY_HANDLE_FILE_INFORMATION, FileStreamInfo};
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // A fixed aligned buffer is ample for the one unnamed stream we accept. An overflow
        // or unsupported query is unknown, not permission to discard unexamined stream data.
        let mut streams = [0u64; 64];
        // SAFETY: both output buffers have the sizes passed and the file stays open.
        unsafe {
            if GetFileInformationByHandle(file.as_raw_handle(), &mut info) == 0 || info.nNumberOfLinks != 1
                || GetFileInformationByHandleEx(file.as_raw_handle(), FileStreamInfo, streams.as_mut_ptr().cast(), std::mem::size_of_val(&streams) as u32) == 0
            { return None; }
        }
        // FILE_STREAM_INFO: next offset, UTF-16 name byte length, two i64 sizes, then name.
        let bytes: Vec<u8> = streams.into_iter().flat_map(u64::to_ne_bytes).collect();
        let next = u32::from_ne_bytes(bytes[0..4].try_into().ok()?);
        let len = u32::from_ne_bytes(bytes[4..8].try_into().ok()?) as usize;
        let expected: Vec<u8> = "::$DATA".encode_utf16().flat_map(u16::to_ne_bytes).collect();
        if next != 0 || len != expected.len() || bytes.get(24..24 + len)? != expected.as_slice() { return None; }
        let index = ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64;
        (index != 0).then_some((info.dwVolumeSerialNumber as u64, index))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        (meta.nlink() == 1).then_some((meta.dev(), meta.ino()))
    }
    #[cfg(not(any(windows, unix)))]
    { None }
}

/// The handle and its parent directories stay open through the recycling call. The parent's
/// path cannot be replaced while a keeper is being relied on. The source still uses Windows'
/// path-based Recycle Bin API, so a final identity/policy check is also required.
struct PinnedFile {
    file: std::fs::File,
    identity: (u64, u64),
    _parents: Vec<std::fs::File>,
}

impl PinnedFile {
    fn open(path: &Path, keeper: bool) -> Option<Self> {
        if !file_path_is_safe(path) { return None; }
        let mut parents = Vec::new();
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            for parent in path.ancestors().skip(1).collect::<Vec<_>>().into_iter().rev() {
                let file = std::fs::OpenOptions::new().read(true).share_mode(0x1 | 0x2)
                    .custom_flags(0x0200_0000 | 0x0020_0000).open(parent).ok()?;
                let meta = file.metadata().ok()?;
                if !meta.is_dir() || attributes(&meta) & 0x400 != 0 || !opened_as_written(&file, parent) { return None; }
                parents.push(file);
            }
        }
        let file = open_for_comparison(path, Some(keeper)).ok()?;
        let identity = ordinary_file(&file)?;
        if !opened_as_written(&file, path) || !file_path_is_safe(path) { return None; }
        Some(Self { file, identity, _parents: parents })
    }

    fn still_here(&self, path: &Path) -> bool {
        if !file_path_is_safe(path) || ordinary_file(&self.file) != Some(self.identity) { return false; }
        // Metadata-only access works while the data handle denies other writers/deleters.
        let mut options = std::fs::OpenOptions::new();
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.access_mode(0x80).share_mode(0x1 | 0x2 | 0x4).custom_flags(0x0020_0000);
        }
        #[cfg(not(windows))]
        options.read(true);
        options.open(path).is_ok_and(|file| ordinary_file(&file) == Some(self.identity) && opened_as_written(&file, path))
    }
}

fn opened_as_written(file: &std::fs::File, path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
        let mut buf = vec![0u16; 32_768];
        // SAFETY: the open handle and output buffer remain valid for this call.
        let len = unsafe { GetFinalPathNameByHandleW(file.as_raw_handle(), buf.as_mut_ptr(), buf.len() as u32, 0) } as usize;
        if len == 0 || len >= buf.len() { return false; }
        written_path_key(Path::new(&String::from_utf16_lossy(&buf[..len]))) == written_path_key(path)
    }
    #[cfg(not(windows))]
    { let _ = (file, path); true }
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
/// file only if it's unchanged. Nothing you said to keep (`keep`: `TrashConfig.keep`) goes,
/// whatever is ticked. Never deletes anything for good. The saved scan is updated.
pub fn recycle(paths: &Paths, files: &[PathBuf], keep: &[String]) -> RecycleReport {
    let mut room = crate::trash::bin::Room::default();
    recycle_with(paths, files, keep, &mut |p, size| room.fits(p, size), &mut |p| ::trash::delete(p).is_ok())
}

/// Marks what you said to keep in a scan that's about to be shown, so it can't be ticked. In a
/// group of copies, a kept one is a copy that stays.
pub fn mark_kept(report: &mut FilesReport, keep: &[String]) {
    for g in &mut report.groups {
        for f in &mut g.files {
            f.kept = crate::trash::is_kept(&f.path, keep);
        }
        if g.files.iter().any(|f| f.kept) {
            for f in &mut g.files {
                f.keep = f.kept;
            }
        }
    }
    report.wasted = report.groups.iter().map(waste).sum();
    for b in &mut report.big {
        b.kept = crate::trash::is_kept(&b.path, keep);
    }
}

fn recycle_with(paths: &Paths, files: &[PathBuf], keep: &[String], fits: &mut dyn FnMut(&Path, u64) -> bool, remove: &mut dyn FnMut(&Path) -> bool) -> RecycleReport {
    let mut report = load_saved(paths).unwrap_or_default();
    let mut r = RecycleReport::default();
    let ticked: HashSet<String> = files.iter().map(|f| norm_path(&f.to_string_lossy())).collect();
    let mut gone: HashSet<String> = HashSet::new();
    let mut attempted = HashSet::new();
    for file in files {
        let key = norm_path(&file.to_string_lossy());
        if !attempted.insert(key.clone()) { continue; }
        let skip = |why: &str| Skipped { path: file.clone(), why: why.into() };
        if crate::trash::is_kept(file, keep) {
            r.skipped.push(skip("You said to keep this (Settings, Always kept), so it stays."));
            continue;
        }
        let in_group = report.groups.iter().enumerate().find_map(|(gi, g)| {
            g.files.iter().find(|f| norm_path(&f.path.to_string_lossy()) == key).map(|f| (gi, f.modified))
        });
        let big = report.big.iter().find(|b| norm_path(&b.path.to_string_lossy()) == key);
        if (in_group.is_some() || big.is_some()) && !file_path_is_safe(file) {
            r.skipped.push(skip("Projects, browser data, links and protected files stay. This path could not be safely checked."));
            continue;
        }
        if let Some((gi, modified)) = in_group {
            let g = &report.groups[gi];
            let Some(mut source) = PinnedFile::open(file, false) else {
                r.skipped.push(skip("It is open, linked, has extra data streams, or could not be held safely for comparison."));
                continue;
            };
            if !unchanged(file, g.size, modified) {
                r.skipped.push(skip("It changed since mom looked, so it may not be a copy any more."));
                continue;
            }
            let stays = g.files.iter()
                .filter(|f| !ticked.contains(&norm_path(&f.path.to_string_lossy())))
                .find_map(|f| {
                    let mut held = PinnedFile::open(&f.path, true)?;
                    (held.identity != source.identity && identical_open(&mut source.file, &mut held.file, g.size, &g.hash))
                        .then_some((f.path.clone(), held))
                });
            let Some((keeper, mut held)) = stays else {
                r.skipped.push(skip(
                    "No unchanged copy could be held safely to stay (copies were ticked, changed, open or gone), so this one stays.",
                ));
                continue;
            };
            if !fits(file, g.size) {
                r.skipped.push(skip(NO_BIN));
                continue;
            }
            // Keep both comparison handles and the keeper's no-delete/no-write protection
            // alive through the callback. The byte comparison can take time, so check paths,
            // protection markers and streams/hardlinks after it, just before recycling.
            if !identical_open(&mut source.file, &mut held.file, g.size, &g.hash)
                || !source.still_here(file) || !held.still_here(&keeper) || !unchanged(file, g.size, modified)
            {
                r.skipped.push(skip("A file or its protection changed while mom checked it, so it stays."));
                continue;
            }
            let removed = remove(file);
            // A successful unlink can remain delete-pending until this data handle closes.
            // The keeper and both sets of parent handles are still held during confirmation.
            drop(source.file);
            if removed && missing(file) {
                r.freed += g.size;
                r.recycled.push(file.clone());
                gone.insert(key);
            } else {
                r.skipped.push(skip("Windows couldn't move it to the Recycle Bin (it may be open)."));
            }
            continue;
        }
        if let Some(b) = big {
            let Some(source) = PinnedFile::open(file, false) else {
                r.skipped.push(skip("It is open, linked, has extra data streams, or could not be held safely."));
                continue;
            };
            if !big_unchanged(file, b, report.access_times) {
                r.skipped.push(skip("It changed or was accessed since mom looked, or its dates could not be checked."));
            } else if !fits(file, b.size) {
                r.skipped.push(skip(NO_BIN));
            } else if !source.still_here(file) || !big_unchanged(file, b, report.access_times) {
                r.skipped.push(skip("A file or its protection changed while mom checked it, so it stays."));
            } else {
                let removed = remove(file);
                drop(source.file);
                if removed && missing(file) {
                    r.freed += b.size;
                    r.recycled.push(file.clone());
                    gone.insert(key);
                } else {
                    r.skipped.push(skip("Windows couldn't move it to the Recycle Bin (it may be open)."));
                }
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
/// Why something stays when the Recycle Bin can't take it: Windows would delete it for good.
const NO_BIN: &str = "Too big for this drive's Recycle Bin (or the drive has none), so Windows would delete it for good. Mom left it.";

fn missing(path: &Path) -> bool {
    matches!(std::fs::symlink_metadata(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

/// Compare the pinned objects, including their expected saved content hash; never reopen by
/// name halfway through the comparison or let a keeper go before its extra is recycled.
fn identical_open(fa: &mut std::fs::File, fb: &mut std::fs::File, size: u64, expected: &str) -> bool {
    if fa.metadata().map(|m| m.len()).ok() != Some(size) || fb.metadata().map(|m| m.len()).ok() != Some(size)
        || fa.seek(SeekFrom::Start(0)).is_err() || fb.seek(SeekFrom::Start(0)).is_err()
    {
        return false;
    }
    let mut hash = blake3::Hasher::new();
    let mut ba = vec![0u8; 1 << 20];
    let mut bb = vec![0u8; 1 << 20];
    loop {
        let n = match fa.read(&mut ba) {
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return false,
        };
        if n == 0 {
            return fb.read(&mut bb[..1]).is_ok_and(|m| m == 0) && hex(hash.finalize().as_bytes()) == expected;
        }
        if fb.read_exact(&mut bb[..n]).is_err() || ba[..n] != bb[..n] {
            return false;
        }
        hash.update(&ba[..n]);
    }
}

fn unchanged(path: &Path, size: u64, modified: u64) -> bool {
    modified > 0 && std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.len() == size && secs(m.modified()) == Some(modified))
}

fn big_unchanged(path: &Path, file: &BigFile, access_times: bool) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return false };
    meta.is_file()
        && meta.len() == file.size
        && file.modified > 0
        && secs(meta.modified()) == Some(file.modified)
        && crate::trash::cleanup_recency(&meta).is_some_and(|r| file.recency > 0 && r <= file.recency)
        && (!access_times || secs(meta.accessed()).is_some_and(|a| file.accessed > 0 && a <= file.accessed))
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
    std::fs::read_to_string(saved_file(paths))
        .ok()
        .and_then(|s| serde_json::from_str::<FilesReport>(&s).ok())
        .filter(|r| r.age_evidence_version == AGE_EVIDENCE_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::FileTimes;

    fn fixture(name: &str) -> PathBuf {
        // Windows Temp is under AppData, an intentional production exclusion. Keep these
        // isolated fixtures outside that boundary so they exercise the real policy.
        let root = dirs::home_dir().unwrap_or_else(std::env::temp_dir).join(format!("mom-files-{name}-{}", std::process::id()));
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
        let times = FileTimes::new().set_accessed(t).set_modified(t);
        #[cfg(windows)]
        let times = {
            use std::os::windows::fs::FileTimesExt;
            times.set_created(t)
        };
        f.set_times(times).unwrap();
    }

    fn opts(paths: &Paths) -> ScanOpts {
        ScanOpts { roots: default_roots(paths), min_dupe_size: 1000, big_min_size: 100_000, big_unused_days: 365 }
    }

    fn look(paths: &Paths) -> FilesReport {
        scan(paths, &opts(paths), &AtomicBool::new(false), &|_| {})
    }

    fn identical(a: &Path, b: &Path) -> bool {
        let (Ok(mut fa), Ok(mut fb)) = (open_quietly(a), open_quietly(b)) else { return false };
        let Ok(meta) = fa.metadata() else { return false };
        let Hashed::Done(hash) = full_hash(a, &AtomicBool::new(false), &mut |_| {}) else { return false };
        identical_open(&mut fa, &mut fb, meta.len(), &hex(&hash))
    }

    #[test]
    fn copies_are_checked_byte_for_byte_and_places_are_yours_to_choose() {
        let root = fixture("places");
        let (a, b, c) = (root.join("a.bin"), root.join("b.bin"), root.join("c.bin"));
        put(&a, &stuff(300_000, 1));
        put(&b, &stuff(300_000, 1));
        let mut differs = stuff(300_000, 1);
        differs[299_999] ^= 1;
        put(&c, &differs);
        assert!(identical(&a, &b));
        assert!(!identical(&a, &c), "one byte apart");
        assert!(!identical(&a, &root.join("missing.bin")));

        let paths = Paths::under(&root);
        let mut cfg = crate::config::FilesConfig::default();
        assert!(set_place(&paths, &mut cfg, Path::new(r"C:\Windows\System32"), true).is_err());
        assert!(set_place(&paths, &mut cfg, Path::new(r"C:\Program Files"), true).is_err());
        let photos = root.join("Photos");
        std::fs::create_dir_all(&photos).unwrap();
        set_place(&paths, &mut cfg, &photos, true).unwrap();
        assert!(chosen_roots(&paths, &cfg).iter().any(|r| same_path(r, &photos)));
        set_place(&paths, &mut cfg, &photos, false).unwrap();
        assert!(!chosen_roots(&paths, &cfg).iter().any(|r| same_path(r, &photos)));
        // A usual folder is switched off by its name.
        std::fs::create_dir_all(&paths.downloads).unwrap();
        set_place(&paths, &mut cfg, &paths.downloads, false).unwrap();
        assert_eq!(cfg.skip, vec!["Downloads".to_string()]);
        assert!(!chosen_roots(&paths, &cfg).iter().any(|r| same_path(r, &paths.downloads)));
        set_place(&paths, &mut cfg, &paths.downloads, true).unwrap();
        assert!(cfg.skip.is_empty());
        assert!(is_drive_root(Path::new(r"D:\")) && !is_drive_root(&photos));
        std::fs::remove_dir_all(&root).unwrap();
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
    fn missing_dates_are_unknown_and_legacy_scans_require_another_look() {
        assert_eq!(secs(Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))), None);
        assert_eq!(secs(Ok(UNIX_EPOCH)), None);
        assert_eq!(secs(Ok(UNIX_EPOCH - Duration::from_secs(1))), None);
        let root = fixture("legacy-age");
        let paths = Paths::under(&root);
        save(&paths, &FilesReport::default());
        assert!(load_saved(&paths).is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_big_file_accessed_after_the_scan_is_not_recycled() {
        let root = fixture("recent-access");
        let paths = Paths::under(&root);
        let movie = paths.home.join("Videos").join("old.mkv");
        put(&movie, &stuff(150_000, 9));
        age(&movie, 400);
        let mut report = look(&paths);
        assert_eq!(report.big.len(), 1);
        report.access_times = true; // exercise the access-time policy regardless of this PC's setting
        save(&paths, &report);
        std::fs::File::options().write(true).open(&movie).unwrap()
            .set_times(FileTimes::new().set_accessed(SystemTime::now())).unwrap();
        let mut called = false;
        let result = recycle_with(&paths, std::slice::from_ref(&movie), &[], &mut |_, _| true, &mut |_| { called = true; true });
        assert!(!called && result.recycled.is_empty());
        assert!(result.skipped[0].why.contains("accessed"));
        assert!(movie.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn a_new_copy_with_old_write_and_access_times_is_not_old() {
        let root = fixture("recent-creation");
        let paths = Paths::under(&root);
        let file = paths.home.join("Videos").join("copied.mkv");
        put(&file, &stuff(150_000, 7));
        let old = SystemTime::now() - Duration::from_secs(400 * 86_400);
        std::fs::File::options().write(true).open(&file).unwrap()
            .set_times(FileTimes::new().set_accessed(old).set_modified(old)).unwrap();
        assert!(look(&paths).big.is_empty(), "the new creation time protects this copy");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn browser_profiles_are_not_scanned_as_duplicate_or_unused_files() {
        let root = fixture("browser-scan");
        let paths = Paths::under(&root);
        let docs = paths.home.join("Documents");
        let data = stuff(150_000, 7);
        let first = docs.join("first.bin");
        let second = docs.join("second.bin");
        let cookie = docs.join("profile-backup").join("Network").join("Cookies");
        let session = docs.join("profile-backup").join("Sessions").join("Session_1");
        for path in [&first, &second, &cookie, &session] {
            put(path, &data);
            age(path, 400);
        }
        let report = look(&paths);
        assert_eq!(report.groups.len(), 1);
        assert_eq!(report.groups[0].files.len(), 2);
        assert!(report.groups[0].files.iter().all(|f| f.path == first || f.path == second));
        assert!(report.big.iter().all(|f| f.path == first || f.path == second));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_stale_saved_scan_cannot_recycle_files_that_are_now_browser_data() {
        let root = fixture("browser-stale");
        let paths = Paths::under(&root);
        let app = paths.home.join("Documents").join("app-backup");
        let duplicate = app.join("session.bin");
        let big = app.join("saved-page.bin");
        put(&duplicate, &stuff(20_000, 3));
        put(&paths.downloads.join("copy.bin"), &stuff(20_000, 3));
        put(&big, &stuff(150_000, 4));
        age(&big, 400);
        let report = look(&paths);
        assert_eq!((report.groups.len(), report.big.len()), (1, 1));
        save(&paths, &report);
        // The files' own contents, lengths and mtimes are unchanged; the enclosing folder
        // now has browser-profile evidence, which must be checked again at mutation time.
        put(&app.join("Cookies"), b"fixture cookie marker");
        let mut calls = Vec::new();
        let result = recycle_with(&paths, &[duplicate.clone(), big.clone()], &[], &mut |_, _| true, &mut |p| { calls.push(p.to_path_buf()); true });
        assert!(calls.is_empty() && result.recycled.is_empty());
        assert_eq!(result.skipped.len(), 2);
        assert!(duplicate.exists() && big.exists() && app.join("Cookies").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn duplicate_roots_and_hardlink_aliases_are_not_extra_copies() {
        let root = fixture("file-identities");
        let paths = Paths::under(&root);
        let docs = paths.home.join("Documents");
        let a = docs.join("a.pdf");
        let b = docs.join("b.pdf");
        let data = stuff(20_000, 6);
        put(&a, &data);
        put(&b, &data);
        let mut options = opts(&paths);
        options.roots = vec![docs.clone(), docs.clone()];
        let report = scan(&paths, &options, &AtomicBool::new(false), &|_| {});
        assert_eq!(report.scanned_files, 2);
        assert_eq!((report.groups.len(), report.groups[0].files.len(), report.wasted), (1, 2, 20_000));
        save(&paths, &report);
        std::fs::hard_link(&a, docs.join("alias.pdf")).unwrap();
        assert!(scan(&paths, &options, &AtomicBool::new(false), &|_| {}).groups.is_empty());
        let mut called = false;
        let result = recycle_with(&paths, &[a], &[], &mut |_, _| true, &mut |_| { called = true; true });
        assert!(!called && result.recycled.is_empty(), "a stale scan cannot remove a newly hardlinked file");
        assert!(b.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn explicitly_added_projects_and_their_subfolders_are_excluded() {
        let root = fixture("explicit-project-root");
        let paths = Paths::under(&root);
        let repo = paths.downloads.join("my-project");
        let assets = repo.join("assets");
        put(&repo.join("package.json"), b"{}");
        put(&assets.join("a.pdf"), &stuff(20_000, 2));
        put(&assets.join("b.pdf"), &stuff(20_000, 2));
        for selected in [repo, assets] {
            let mut options = opts(&paths);
            options.roots = vec![selected];
            let report = scan(&paths, &options, &AtomicBool::new(false), &|_| {});
            assert!(report.groups.is_empty() && report.big.is_empty());
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn explicitly_added_tool_and_game_subfolders_keep_their_boundaries() {
        let root = fixture("explicit-tool-roots");
        let paths = Paths::under(&root);
        for boundary in ["node_modules", "steamapps", ".tool-cache"] {
            let base = paths.downloads.join(boundary);
            let assets = base.join("some-package").join("assets");
            put(&assets.join("a.pdf"), &stuff(20_000, 2));
            put(&assets.join("b.pdf"), &stuff(20_000, 2));
            for selected in [base, assets] {
                let mut options = opts(&paths);
                options.roots = vec![selected];
                let report = scan(&paths, &options, &AtomicBool::new(false), &|_| {});
                assert!(report.groups.is_empty() && report.big.is_empty());
                assert_eq!(report.scanned_files, 0);
            }
        }
        // These names alone are not protected when used for ordinary nested folders.
        let ordinary = paths.downloads.join("photos").join("Nvidia");
        put(&ordinary.join("a.pdf"), &stuff(20_000, 2));
        put(&ordinary.join("b.pdf"), &stuff(20_000, 2));
        let mut options = opts(&paths);
        options.roots = vec![ordinary];
        assert_eq!(scan(&paths, &options, &AtomicBool::new(false), &|_| {}).groups.len(), 1);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn system_directory_names_only_apply_at_real_drive_roots_on_every_drive() {
        for drive in ["C", "D", "Z"] {
            for name in SKIP_AT_DRIVE_ROOT {
                assert!(protected_directory_name(Path::new(&format!(r"{drive}:\{name}"))));
                assert!(!protected_directory_name(Path::new(&format!(r"{drive}:\photos\{name}"))));
            }
        }
        assert!(protected_directory_name(Path::new(r"D:\tools\node_modules")));
        assert!(!protected_directory_name(Path::new(r"D:\tools\node_modules-old")));
        assert!(!protected_directory_name(Path::new(r"D:Windows")));
        assert!(!protected_directory_name(Path::new(r"\\server\share\Windows")));
    }

    #[cfg(windows)]
    #[test]
    fn windows_setup_folders_are_not_user_duplicate_or_big_file_roots() {
        // Pure path checks: never create, scan, or remove real system folders. The same
        // boundary is used for explicit scan roots and stale saved deletion candidates.
        for drive in ["C", "D", "Z"] {
            for name in ["$WINDOWS.~TMP", "$WINDOWS.~BT", "$WINDOWS.~WS", "$GetCurrent", "$SysReset", "Windows10Upgrade", "WUDownloadCache", "Windows.old"] {
                let folder = PathBuf::from(format!(r"{drive}:\{name}"));
                assert!(protected_directory_name(&folder), "{folder:?}");
                assert!(directory_boundary(&folder), "explicit setup roots stay protected");
                assert!(scan_root_key(&folder).is_none());
                assert!(!file_path_is_safe(&folder.join("setup.exe")), "a stale saved setup candidate stays protected");
                assert!(!protected_directory_name(Path::new(&format!(r"{drive}:\photos\{name}"))));
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn scan_roots_reject_filesystem_aliases_but_allow_case_and_trailing_separators() {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;
        let root = fixture("root-spelling");
        let paths = Paths::under(&root);
        put(&paths.downloads.join("a.pdf"), &stuff(20_000, 2));
        put(&paths.downloads.join("b.pdf"), &stuff(20_000, 2));
        let ordinary = PathBuf::from(format!("{}\\", paths.downloads.to_string_lossy().to_ascii_uppercase()));
        assert_eq!(scan_root_key(&ordinary), scan_root_key(&paths.downloads));
        assert!(scan_root_key(&ordinary).is_some());
        let mut options = opts(&paths);
        options.roots = vec![ordinary, paths.downloads.clone()];
        let report = scan(&paths, &options, &AtomicBool::new(false), &|_| {});
        assert_eq!((report.scanned_files, report.groups.len()), (2, 1));

        let tools = paths.downloads.join("node_modules");
        put(&tools.join("a.pdf"), &stuff(20_000, 2));
        put(&tools.join("b.pdf"), &stuff(20_000, 2));
        let dotted = PathBuf::from(format!("{}.", tools.display()));
        assert!(dotted.is_dir(), "ordinary Win32 trailing-dot spelling resolves to the real folder");
        assert!(scan_root_key(&dotted).is_none());
        options.roots = vec![dotted];
        assert_eq!(scan(&paths, &options, &AtomicBool::new(false), &|_| {}).scanned_files, 0);

        // Volumes may disable 8.3 names. When one exists, it must be rejected too.
        let wide: Vec<u16> = tools.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut short = vec![0u16; 32_768];
        let len = unsafe { GetShortPathNameW(wide.as_ptr(), short.as_mut_ptr(), short.len() as u32) } as usize;
        if len > 0 && len < short.len() {
            let alias = PathBuf::from(String::from_utf16_lossy(&short[..len]));
            if written_path_key(&alias) != written_path_key(&tools) {
                assert!(scan_root_key(&alias).is_none());
                options.roots = vec![alias];
                assert_eq!(scan(&paths, &options, &AtomicBool::new(false), &|_| {}).scanned_files, 0);
            }
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn a_newly_hidden_parent_blocks_saved_candidates_and_is_rechecked_after_capacity() {
        use std::os::windows::ffi::OsStrExt;
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::SetFileAttributesW;
        let root = fixture("late-hidden-parent");
        let paths = Paths::under(&root);
        let keeper = paths.home.join("Documents").join("a.pdf");
        let extra = paths.downloads.join("a.pdf");
        put(&keeper, &stuff(20_000, 2));
        put(&extra, &stuff(20_000, 2));
        let report = look(&paths);
        assert_eq!(report.groups.len(), 1);
        save(&paths, &report);
        let original = std::fs::metadata(&paths.downloads).unwrap().file_attributes();
        let wide: Vec<u16> = paths.downloads.as_os_str().encode_wide().chain(Some(0)).collect();
        let set = |attrs| assert_ne!(unsafe { SetFileAttributesW(wide.as_ptr(), attrs) }, 0);
        set(original | 0x2);
        let mut called = false;
        let result = recycle_with(&paths, std::slice::from_ref(&extra), &[], &mut |_, _| true, &mut |_| { called = true; true });
        set(original);
        assert!(!called && result.recycled.is_empty());
        let result = recycle_with(&paths, std::slice::from_ref(&extra), &[], &mut |_, _| {
            set(original | 0x2);
            true
        }, &mut |_| { called = true; true });
        set(original);
        assert!(!called && result.recycled.is_empty());
        assert!(keeper.exists() && extra.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn newly_protected_paths_are_rechecked_after_capacity_work() {
        let root = fixture("late-project-marker");
        let paths = Paths::under(&root);
        let keeper = paths.home.join("Documents").join("a.pdf");
        let extra = paths.downloads.join("a.pdf");
        put(&keeper, &stuff(20_000, 2));
        put(&extra, &stuff(20_000, 2));
        let report = look(&paths);
        assert_eq!(report.groups.len(), 1);
        save(&paths, &report);
        let mut called = false;
        let result = recycle_with(&paths, std::slice::from_ref(&extra), &[], &mut |_, _| {
            put(&paths.downloads.join("package.json"), b"{}");
            true
        }, &mut |_| { called = true; true });
        assert!(!called && result.recycled.is_empty());
        assert!(keeper.exists() && extra.exists());
        // It also remains blocked at the start of a later request using that saved report.
        let result = recycle_with(&paths, &[extra], &[], &mut |_, _| true, &mut |_| { called = true; true });
        assert!(!called && result.recycled.is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn named_streams_are_not_discarded_as_identical_main_file_contents() {
        let root = fixture("file-streams");
        let paths = Paths::under(&root);
        let keeper = paths.home.join("Documents").join("a.pdf");
        let extra = paths.downloads.join("a.pdf");
        put(&keeper, &stuff(20_000, 3));
        put(&extra, &stuff(20_000, 3));
        let report = look(&paths);
        assert_eq!(report.groups.len(), 1);
        save(&paths, &report);
        std::fs::write(format!("{}:Zone.Identifier", extra.display()), b"[ZoneTransfer]\nZoneId=3").unwrap();
        assert!(look(&paths).groups.is_empty());
        let mut called = false;
        let result = recycle_with(&paths, &[extra.clone()], &[], &mut |_, _| true, &mut |_| { called = true; true });
        assert!(!called && result.recycled.is_empty());
        assert!(keeper.exists() && extra.exists());
        std::fs::remove_file(format!("{}:Zone.Identifier", extra.display())).unwrap();
        // Stream sharing is independent of the unnamed data handle, so recheck after fits.
        let result = recycle_with(&paths, &[extra.clone()], &[], &mut |_, _| {
            std::fs::write(format!("{}:new-data", extra.display()), b"data added during capacity work").unwrap();
            true
        }, &mut |_| { called = true; true });
        assert!(!called && result.recycled.is_empty());
        assert!(extra.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn the_keeper_and_its_parent_remain_pinned_until_recycling_finishes() {
        let root = fixture("pinned-keeper");
        let paths = Paths::under(&root);
        let keeper = paths.home.join("Documents").join("a.pdf");
        let extra = paths.downloads.join("a.pdf");
        put(&keeper, &stuff(20_000, 5));
        put(&extra, &stuff(20_000, 5));
        save(&paths, &look(&paths));
        let mut calls = 0;
        let result = recycle_with(&paths, &[extra.clone(), extra.clone()], &[], &mut |_, _| true, &mut |path| {
            calls += 1;
            assert!(std::fs::write(&keeper, b"changed").is_err());
            assert!(std::fs::remove_file(&keeper).is_err());
            assert!(std::fs::rename(keeper.parent().unwrap(), root.join("moved-documents")).is_err());
            assert!(std::fs::write(path, b"changed").is_err());
            std::fs::remove_file(path).is_ok()
        });
        assert_eq!(calls, 1, "duplicate request entries only reach the recycler once");
        assert_eq!(result.recycled, vec![extra]);
        assert_eq!(result.freed, 20_000);
        assert_eq!(std::fs::read(&keeper).unwrap(), stuff(20_000, 5));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn replacing_a_source_path_during_capacity_work_never_recycles_the_replacement() {
        let root = fixture("source-replaced");
        let paths = Paths::under(&root);
        let keeper = paths.home.join("Documents").join("a.pdf");
        let extra = paths.downloads.join("a.pdf");
        let moved = paths.downloads.join("moved-original.pdf");
        put(&keeper, &stuff(20_000, 4));
        put(&extra, &stuff(20_000, 4));
        save(&paths, &look(&paths));
        let mut called = false;
        let result = recycle_with(&paths, std::slice::from_ref(&extra), &[], &mut |_, _| {
            std::fs::rename(&extra, &moved).unwrap();
            put(&extra, &stuff(20_000, 4)); // even identical replacement bytes are a different object
            true
        }, &mut |_| { called = true; true });
        assert!(!called && result.recycled.is_empty());
        assert!(keeper.exists() && extra.exists() && moved.exists());
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
        let r = recycle_with(&paths, &[keep.clone(), extra.clone(), extra2.clone()], &[], &mut |_, _| true, &mut fake);
        assert!(r.recycled.is_empty() && r.skipped.len() == 3, "{r:?}");
        assert!(keep.exists() && extra.exists() && extra2.exists());
        // One extra changed since: it stays; the other goes, and the saved scan drops it.
        std::fs::write(&extra2, stuff(20_000, 12)).unwrap();
        let r = recycle_with(&paths, &[extra.clone(), extra2.clone()], &[], &mut |_, _| true, &mut fake);
        assert_eq!(r.recycled, vec![extra.clone()], "{r:?}");
        assert_eq!(r.freed, 20_000);
        assert!(r.skipped[0].why.contains("changed"));
        assert!(!extra.exists() && keep.exists() && extra2.exists());
        let saved = load_saved(&paths).unwrap();
        assert_eq!(saved.groups.len(), 1);
        assert_eq!(saved.groups[0].files.len(), 2, "{:?}", saved.groups);
        // With the staying copy changed, the last unchanged one stays too.
        std::fs::write(&keep, stuff(20_000, 13)).unwrap();
        let r = recycle_with(&paths, std::slice::from_ref(&extra2), &[], &mut |_, _| true, &mut fake);
        assert!(r.recycled.is_empty(), "{r:?}");
        // Something mom never found isn't touched.
        let r = recycle_with(&paths, &[root.join("whatever.txt")], &[], &mut |_, _| true, &mut fake);
        assert!(r.recycled.is_empty() && r.skipped[0].why.contains("didn't find"));
        assert_eq!(removed, vec![extra.clone()]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn what_you_said_to_keep_never_goes_whatever_is_ticked() {
        let root = fixture("kept");
        let paths = Paths::under(&root);
        let a = stuff(20_000, 21);
        let first = paths.home.join("Documents").join("notes.pdf");
        let kept_copy = paths.home.join("Pictures").join("album").join("notes.pdf");
        let extra = paths.downloads.join("notes.pdf");
        put(&first, &a);
        put(&kept_copy, &a);
        put(&extra, &a);
        let old = paths.home.join("Videos").join("holiday.mp4");
        put(&old, &stuff(30_000, 22));
        let mut report = look(&paths);
        save(&paths, &report);
        // You said to keep a whole folder (spelled your own way) and one file.
        let keep = vec![paths.home.join("Pictures").join("ALBUM").to_string_lossy().replace('\\', "/"), old.to_string_lossy().to_string(), "  ".to_string()];
        let mut removed = Vec::new();
        let mut fake = |p: &Path| {
            removed.push(p.to_path_buf());
            std::fs::remove_file(p).is_ok()
        };
        // Ticked or not, what's kept stays, and says why; the copy that isn't kept still goes.
        let r = recycle_with(&paths, &[kept_copy.clone(), old.clone(), extra.clone()], &keep, &mut |_, _| true, &mut fake);
        assert_eq!(r.recycled, vec![extra.clone()], "{r:?}");
        assert_eq!(r.skipped.iter().map(|s| s.path.clone()).collect::<Vec<_>>(), [kept_copy.clone(), old.clone()]);
        assert!(r.skipped.iter().all(|s| s.why.contains("You said to keep this")), "{:?}", r.skipped);
        assert!(kept_copy.exists() && old.exists() && first.exists() && !extra.exists());
        // Shown, a kept copy is the one that stays and can't be ticked; so is a kept big file.
        mark_kept(&mut report, &keep);
        let g = &report.groups[0];
        let of = |p: &Path| g.files.iter().find(|f| f.path == p).unwrap();
        assert!(of(&kept_copy).kept && of(&kept_copy).keep && !of(&first).kept && !of(&first).keep && !of(&extra).kept);
        assert!(report.big.iter().filter(|b| b.path == old).all(|b| b.kept));
        // With nothing kept, nothing is marked, and the first choice of a copy to keep stands.
        let mut plain = look(&paths);
        mark_kept(&mut plain, &[]);
        assert!(plain.groups.iter().flat_map(|g| &g.files).all(|f| !f.kept) && plain.groups.iter().all(|g| g.files.iter().filter(|f| f.keep).count() == 1));
        // The mark is worked out when the scan is shown, never read back from the saved one.
        save(&paths, &report);
        assert!(load_saved(&paths).unwrap().groups.iter().flat_map(|g| &g.files).all(|f| !f.kept));
        assert_eq!(removed, vec![extra]);
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
