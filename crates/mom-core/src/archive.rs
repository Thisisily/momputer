//! Moving old conversations to another drive without the agents noticing.
//!
//! A folder is copied to the archive (mirroring where it came from, so a month of Codex chats
//! lands in `F:\momputer-archive\C\Users\me\.codex\sessions\2026\07`), checked file by file, and
//! then swapped for a link that points at the copy, so the agent opens it from the same path as
//! before. Only then is the original deleted. If any step fails, the ones before it are undone.
//! Bringing a folder back runs the same steps the other way.
//!
//! The links are junctions, Windows' folder links: they need no special rights and everything
//! that opens files sees straight through them. Mom's own walkers and cleaners never follow them
//! (see trash::walk), so nothing in the archive is ever counted or deleted by a clean-up.

use crate::paths::Paths;
use crate::util::{now_secs, write_atomic};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// One folder mom moved.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Moved {
    /// Where it was, and where the link to it now is.
    pub from: PathBuf,
    /// Where it is now.
    pub to: PathBuf,
    /// The archive it's in (`to` is somewhere inside).
    pub archive: PathBuf,
    /// What mom calls it ("Codex · July 2026").
    pub name: String,
    /// The clean-up that moved it ("old-conversations").
    #[serde(default)]
    pub spot: String,
    pub bytes: u64,
    pub files: u64,
    pub moved_at: u64,
}

/// A moved folder and how it's doing.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MovedNow {
    #[serde(flatten)]
    pub moved: Moved,
    /// The link is still in its old place.
    pub linked: bool,
    /// The drive it's on is connected and the folder is there.
    pub reachable: bool,
}

/// The list of everything moved, kept with mom's state and, per archive, next to the folders.
fn record_file(paths: &Paths) -> PathBuf {
    paths.state_dir().join("archive.json")
}

const MANIFEST: &str = "momputer-archive.json";

pub fn list(paths: &Paths) -> Vec<Moved> {
    std::fs::read_to_string(record_file(paths)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn status(paths: &Paths) -> Vec<MovedNow> {
    list(paths)
        .into_iter()
        .map(|m| MovedNow { linked: is_link(&m.from), reachable: m.to.is_dir(), moved: m })
        .collect()
}

fn save(paths: &Paths, all: &[Moved]) {
    if let Ok(json) = serde_json::to_string_pretty(all) {
        let _ = write_atomic(&record_file(paths), &json);
    }
    // Each archive also says what's in it and where it came from, in case this PC is ever gone.
    let mut roots: Vec<&PathBuf> = all.iter().map(|m| &m.archive).collect();
    roots.sort();
    roots.dedup();
    for root in roots {
        let here: Vec<&Moved> = all.iter().filter(|m| &m.archive == root).collect();
        if let Ok(json) = serde_json::to_string_pretty(&here) {
            let _ = write_atomic(&root.join(MANIFEST), &json);
        }
    }
}

/// Where a folder goes in an archive: the archive, then the drive letter, then the rest of its path.
pub fn destination(root: &Path, from: &Path) -> PathBuf {
    let mut to = root.to_path_buf();
    for c in from.components() {
        match c {
            Component::Prefix(p) => to.push(match p.kind() {
                std::path::Prefix::Disk(d) | std::path::Prefix::VerbatimDisk(d) => (d as char).to_ascii_uppercase().to_string(),
                _ => "elsewhere".to_string(),
            }),
            Component::Normal(n) => to.push(n),
            _ => {}
        }
    }
    to
}

pub fn is_link(p: &Path) -> bool {
    std::fs::symlink_metadata(p).map(|m| m.is_symlink()).unwrap_or(false)
}

/// Is `a` on the same drive as `b`? Moving there wouldn't free any space.
pub fn same_drive(a: &Path, b: &Path) -> bool {
    let drive = |p: &Path| match p.components().next() {
        Some(Component::Prefix(x)) => Some(x.as_os_str().to_ascii_uppercase()),
        _ => None,
    };
    drive(a).is_some() && drive(a) == drive(b)
}

/// Files and bytes in a folder (through a link at the top, never through links inside). A link
/// inside means it can't be moved safely, so that's an error.
fn tally(dir: &Path) -> Result<(u64, u64), String> {
    let (mut files, mut bytes) = (0, 0);
    for e in walkdir::WalkDir::new(dir).follow_links(false) {
        let e = e.map_err(|e| format!("Couldn't read {}: {e}", dir.display()))?;
        if e.depth() > 0 && e.path_is_symlink() {
            return Err(format!("{} has a link inside it, so mom leaves it where it is.", dir.display()));
        }
        if e.file_type().is_file() {
            files += 1;
            bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    Ok((files, bytes))
}

/// Copies a folder, keeping each file's modified time. Says how many files and bytes it copied.
fn copy_tree(src: &Path, dst: &Path) -> Result<(u64, u64), String> {
    let (mut files, mut bytes) = (0, 0);
    std::fs::create_dir_all(dst).map_err(|e| format!("Couldn't make {}: {e}", dst.display()))?;
    for e in walkdir::WalkDir::new(src).follow_links(false).min_depth(1) {
        let e = e.map_err(|e| format!("Couldn't read {}: {e}", src.display()))?;
        let rel = e.path().strip_prefix(src).map_err(|e| e.to_string())?;
        let target = dst.join(rel);
        if e.path_is_symlink() {
            return Err(format!("{} has a link inside it, so mom leaves it where it is.", src.display()));
        }
        if e.file_type().is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| format!("Couldn't make {}: {e}", target.display()))?;
            continue;
        }
        let meta = e.metadata().map_err(|e| e.to_string())?;
        let n = std::fs::copy(e.path(), &target).map_err(|err| format!("Couldn't copy {}: {err}", e.path().display()))?;
        if n != meta.len() {
            return Err(format!("{} didn't copy in full.", e.path().display()));
        }
        if let Ok(when) = meta.modified() {
            if let Ok(f) = std::fs::File::options().write(true).open(&target) {
                let _ = f.set_modified(when);
            }
        }
        files += 1;
        bytes += n;
    }
    Ok((files, bytes))
}

fn sibling(p: &Path, tag: &str) -> PathBuf {
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    p.with_file_name(format!("{name}.{tag}"))
}

fn make_link(link: &Path, target: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        junction::create(target, link)
    }
    #[cfg(not(windows))]
    {
        std::os::unix::fs::symlink(target, link)
    }
}

/// Removes a link to a folder, never the folder.
fn remove_link(link: &Path) -> std::io::Result<()> {
    if !is_link(link) {
        return Err(std::io::Error::other("not a link"));
    }
    std::fs::remove_dir(link).or_else(|_| std::fs::remove_file(link))
}

fn remove_tree(p: &Path) {
    if !is_link(p) {
        let _ = std::fs::remove_dir_all(p);
    }
}

/// Free space on the drive `p` is on.
pub fn free_space(p: &Path) -> Option<u64> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let mut dir = p.to_path_buf();
        while !dir.exists() {
            dir = dir.parent()?.to_path_buf();
        }
        let w: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
        let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
        (unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, &mut total, &mut free) } != 0).then_some(avail)
    }
    #[cfg(not(windows))]
    {
        let _ = p;
        None
    }
}

/// What's kept free on a drive after a move, so mom never fills it to the brim.
const MARGIN: u64 = 1 << 30;

/// Moves a folder into the archive at `root` and leaves a link in its place.
pub fn move_out(paths: &Paths, from: &Path, root: &Path, name: &str, spot: &str) -> Result<Moved, String> {
    let meta = std::fs::symlink_metadata(from).map_err(|_| format!("{name} isn't there any more."))?;
    if meta.is_symlink() {
        return Err(format!("{name} has already been moved."));
    }
    if !meta.is_dir() {
        return Err(format!("{name} is a file; mom only moves folders, so she can leave a link."));
    }
    if root.starts_with(from) {
        return Err("The archive can't be inside what's being moved.".into());
    }
    let to = destination(root, from);
    if to.exists() {
        return Err(format!("There's already something at {}; mom won't overwrite it.", to.display()));
    }
    let (files, bytes) = tally(from)?;
    if let Some(free) = free_space(root) {
        if free < bytes + MARGIN {
            return Err(format!(
                "Not enough room on {} for {name}: it needs {}, and there's {} free.",
                root.display(),
                crate::util::human_bytes(bytes + MARGIN),
                crate::util::human_bytes(free)
            ));
        }
    }
    // Copy next to where it'll go, check it, then give it its real name.
    let partial = sibling(&to, "momputer-partial");
    remove_tree(&partial);
    let undo_copy = |why: String| {
        remove_tree(&partial);
        remove_tree(&to);
        Err(why)
    };
    match copy_tree(from, &partial) {
        Ok(copied) if copied == (files, bytes) => {}
        Ok(_) => return undo_copy(format!("{name} changed while it was being copied; nothing was moved.")),
        Err(e) => return undo_copy(e),
    }
    if let Err(e) = std::fs::rename(&partial, &to) {
        return undo_copy(format!("Couldn't finish the copy of {name}: {e}"));
    }
    // Swap the folder for a link to the copy.
    let aside = sibling(from, "momputer-moving");
    if std::fs::rename(from, &aside).is_err() {
        return undo_copy(format!("Something is using {name} right now, so it stays. Try again later."));
    }
    let undo_swap = |why: String| {
        let _ = remove_link(from);
        let _ = std::fs::rename(&aside, from);
        undo_copy(why)
    };
    if let Err(e) = make_link(from, &to) {
        return undo_swap(format!("Couldn't link {name} to its new place: {e}"));
    }
    // Does it open through the link, all of it?
    match tally(from) {
        Ok(seen) if seen == (files, bytes) => {}
        _ => return undo_swap(format!("{name} didn't open through its link, so it was put back.")),
    }
    let moved = Moved {
        from: from.to_path_buf(),
        to,
        archive: root.to_path_buf(),
        name: name.to_string(),
        spot: spot.to_string(),
        bytes,
        files,
        moved_at: now_secs(),
    };
    let mut all = list(paths);
    all.retain(|m| m.from != moved.from);
    all.push(moved.clone());
    save(paths, &all);
    remove_tree(&aside);
    Ok(moved)
}

/// Puts a moved folder back where it was and removes it from the archive.
pub fn bring_back(paths: &Paths, from: &Path) -> Result<Moved, String> {
    let mut all = list(paths);
    let Some(m) = all.iter().find(|m| m.from == from).cloned() else {
        return Err("Mom didn't move that one.".into());
    };
    if !m.to.is_dir() {
        return Err(format!("{} can't be reached. Is that drive connected?", m.to.display()));
    }
    let here = std::fs::symlink_metadata(from).ok();
    if here.as_ref().is_some_and(|h| !h.is_symlink()) {
        return Err(format!("Something new is at {} now; mom won't overwrite it.", from.display()));
    }
    let (files, bytes) = tally(&m.to)?;
    if let Some(free) = free_space(from) {
        if free < bytes + MARGIN {
            return Err(format!(
                "Not enough room to bring {} back: it needs {}, and there's {} free.",
                m.name,
                crate::util::human_bytes(bytes + MARGIN),
                crate::util::human_bytes(free)
            ));
        }
    }
    let back = sibling(from, "momputer-back");
    remove_tree(&back);
    match copy_tree(&m.to, &back) {
        Ok(copied) if copied == (files, bytes) => {}
        Ok(_) => {
            remove_tree(&back);
            return Err(format!("{} changed while it was being copied back; it's still on the other drive.", m.name));
        }
        Err(e) => {
            remove_tree(&back);
            return Err(e);
        }
    }
    if here.is_some() {
        if let Err(e) = remove_link(from) {
            remove_tree(&back);
            return Err(format!("Couldn't remove the link to {}: {e}", m.name));
        }
    }
    if let Err(e) = std::fs::rename(&back, from) {
        let _ = make_link(from, &m.to);
        remove_tree(&back);
        return Err(format!("Couldn't put {} back: {e}", m.name));
    }
    all.retain(|x| x.from != m.from);
    save(paths, &all);
    if all.iter().all(|x| x.archive != m.archive) {
        let _ = std::fs::remove_file(m.archive.join(MANIFEST));
    }
    remove_tree(&m.to);
    Ok(m)
}

/// Deletes a moved folder from the archive for good, and its link. For things nobody needs
/// back (the data of an app that's gone, say).
pub fn delete_moved(paths: &Paths, from: &Path) -> Result<Moved, String> {
    let mut all = list(paths);
    let Some(m) = all.iter().find(|m| m.from == from).cloned() else {
        return Err("Mom didn't move that one.".into());
    };
    if is_link(from) {
        remove_link(from).map_err(|e| format!("Couldn't remove the link to {}: {e}", m.name))?;
    }
    if m.to.exists() {
        if let Err(e) = std::fs::remove_dir_all(&m.to) {
            // Put the link back, so what's left still opens from where it was.
            let _ = make_link(from, &m.to);
            return Err(format!("Couldn't delete {} from {}: {e}", m.name, m.archive.display()));
        }
    }
    all.retain(|x| x.from != m.from);
    save(paths, &all);
    if all.iter().all(|x| x.archive != m.archive) {
        let _ = std::fs::remove_file(m.archive.join(MANIFEST));
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> (PathBuf, Paths) {
        let root = std::env::temp_dir().join(format!("mom-archive-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::under(&root);
        (root, paths)
    }

    fn write(p: &Path, bytes: usize, days_old: u64) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![b'x'; bytes]).unwrap();
        let f = std::fs::File::options().write(true).open(p).unwrap();
        f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(days_old * 86_400)).unwrap();
    }

    #[test]
    fn moves_a_folder_leaves_a_link_and_brings_it_back() {
        let (root, paths) = fixture("move");
        let month = paths.home.join(".codex").join("sessions").join("2026").join("07");
        write(&month.join("01").join("rollout-a.jsonl"), 3000, 100);
        write(&month.join("02").join("rollout-b.jsonl"), 2000, 99);
        let archive = root.join("F").join("momputer-archive");
        std::fs::create_dir_all(&archive).unwrap();

        let m = move_out(&paths, &month, &archive, "Codex · July 2026", "old-conversations").unwrap();
        assert_eq!((m.files, m.bytes), (2, 5000));
        assert!(m.to.starts_with(&archive) && m.to.ends_with("sessions/2026/07"), "{}", m.to.display());
        // The agent still finds everything where it was, with its dates.
        assert!(is_link(&month));
        let through = month.join("01").join("rollout-a.jsonl");
        assert_eq!(std::fs::read(&through).unwrap().len(), 3000);
        let age = std::fs::metadata(&through).unwrap().modified().unwrap().elapsed().unwrap().as_secs();
        assert!(age > 99 * 86_400, "kept its modified time");
        assert!(!sibling(&month, "momputer-moving").exists(), "the original is gone");
        // It's on the list, here and in the archive.
        assert_eq!(list(&paths), vec![m.clone()]);
        assert!(archive.join(MANIFEST).exists());
        let now = status(&paths);
        assert!(now[0].linked && now[0].reachable);
        // Moving it again does nothing.
        assert!(move_out(&paths, &month, &archive, "again", "old-conversations").is_err());

        let back = bring_back(&paths, &month).unwrap();
        assert_eq!(back.from, month);
        assert!(!is_link(&month) && month.join("02").join("rollout-b.jsonl").is_file());
        assert!(!m.to.exists(), "gone from the archive");
        assert!(list(&paths).is_empty());

        // Or, moved again and then deleted for good: the link and the copy go, nothing else.
        let m = move_out(&paths, &month, &archive, "Codex · July 2026", "old-conversations").unwrap();
        delete_moved(&paths, &month).unwrap();
        assert!(!month.exists() && !m.to.exists());
        assert!(paths.home.join(".codex").join("sessions").join("2026").is_dir());
        assert!(list(&paths).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_folder_in_use_or_with_links_inside_stays_put() {
        let (root, paths) = fixture("busy");
        let day = paths.home.join("chats").join("day");
        write(&day.join("a.jsonl"), 100, 100);
        let archive = root.join("F").join("momputer-archive");
        std::fs::create_dir_all(&archive).unwrap();
        // An open file keeps Windows from renaming its folder; everything is undone.
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let held = std::fs::OpenOptions::new().read(true).share_mode(1).open(day.join("a.jsonl")).unwrap();
            let err = move_out(&paths, &day, &archive, "day", "old-conversations").unwrap_err();
            assert!(err.contains("using"), "{err}");
            drop(held);
            assert!(!is_link(&day) && day.join("a.jsonl").is_file());
            assert!(!destination(&archive, &day).exists(), "the copy was cleaned up");
            assert!(list(&paths).is_empty());
        }
        // A link inside (an earlier move, say) means it's left alone.
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        make_link(&day.join("linked"), &elsewhere).unwrap();
        assert!(move_out(&paths, &day, &archive, "day", "old-conversations").unwrap_err().contains("link inside"));
        remove_link(&day.join("linked")).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn archive_paths_mirror_where_things_came_from() {
        let to = destination(Path::new(r"F:\momputer-archive"), Path::new(r"C:\Users\me\.codex\sessions\2026\07"));
        #[cfg(windows)]
        assert_eq!(to, Path::new(r"F:\momputer-archive\C\Users\me\.codex\sessions\2026\07"));
        let _ = to;
        #[cfg(windows)]
        {
            assert!(same_drive(Path::new(r"C:\Users\me"), Path::new(r"c:\archive")));
            assert!(!same_drive(Path::new(r"C:\Users\me"), Path::new(r"F:\archive")));
        }
    }
}
