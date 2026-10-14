//! What mom did: every change she made on this PC, one line each, oldest first, in
//! ~/.momputer/state/history.jsonl, so "What I did" outlives a restart. A clean-up, a move to the
//! archive or back, a folder tidied or put back, a setting or registry fix and its undo, a program
//! stopped or started again, copies sent to the Recycle Bin.
//!
//! A line can say how to put it back, but only by pointing into mom's own records (the archive's
//! list, the tidy journal, a setting's or registry fix's backup, the servers she stopped), never
//! at a path someone hands her, and each of those checks its own record again before it does
//! anything. The file stays small: past 2 MB it becomes history.old.jsonl and a new one starts.
//! Writing it never gets in the way of what's being done. An administrator mom writes nothing
//! here: the folder is yours, and anything running as you could have put a link in her way.

use crate::paths::Paths;
use crate::trash::CleanReport;
use crate::util::{human_bytes, now_secs, plural};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

const FILE: &str = "history.jsonl";
const OLD: &str = "history.old.jsonl";
/// Past this, the file is put aside and a new one starts (one old one is kept).
const MAX_BYTES: u64 = 2 << 20;
/// What's read back from each file, at most: its newest part.
const READ_BYTES: u64 = 2 << 20;
/// A line longer than this isn't one mom wrote.
const MAX_LINE: usize = 4096;
const MAX_TEXT: usize = 200;

/// What sort of thing a line is about.
pub mod kind {
    pub const CLEAN: &str = "clean";
    pub const ADMIN_CLEAN: &str = "admin-clean";
    pub const RECYCLE: &str = "recycle";
    pub const EMPTY_BIN: &str = "empty-bin";
    pub const ARCHIVE: &str = "archive";
    pub const BRING_BACK: &str = "bring-back";
    pub const ARCHIVE_DELETE: &str = "archive-delete";
    pub const TIDY: &str = "tidy";
    pub const TIDY_UNDO: &str = "tidy-undo";
    pub const SETTING: &str = "setting";
    pub const SETTING_UNDO: &str = "setting-undo";
    pub const REGISTRY: &str = "registry";
    pub const REGISTRY_UNDO: &str = "registry-undo";
    pub const STOPPED: &str = "stopped";
    pub const STARTED: &str = "started";
    pub const PROGRAM: &str = "program";
    pub const COPIES: &str = "copies";
    pub const STARTUP: &str = "startup";
}

/// One thing mom did.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Entry {
    /// Its name in the record ("1791935078-1a2b0003"): what Undo asks for.
    pub id: String,
    /// Unix seconds.
    pub at: u64,
    /// What sort of thing it was (see `kind`).
    pub kind: String,
    /// One plain line: "Cleared npm cache · 1.2 GB".
    pub text: String,
    /// Space it freed or moved, where that applies.
    pub bytes: u64,
    /// How many things it took: files, folders, programs.
    pub items: u64,
    /// Autopilot did it, not you there and then.
    pub auto: bool,
    /// How to put it back, from mom's own records. None: it can't be.
    pub undo: Option<Undo>,
    /// For a putting-back: the line it put back.
    pub undoes: Option<String>,
}

/// Where in mom's own records the way back is.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Undo {
    /// "archive", "tidy", "setting", "registry", "start" or "recycle-bin".
    pub kind: String,
    /// What that record calls it: the clean-up that moved folders, or a setting's or a registry
    /// fix's id.
    pub key: String,
    /// When it happened, for records that note the time of each thing they hold (archive moves,
    /// stopped servers): what's from between these is this line's.
    pub from: u64,
    pub to: u64,
}

impl Undo {
    pub fn new(kind: &str, key: &str) -> Undo {
        Undo { kind: kind.into(), key: key.into(), from: 0, to: 0 }
    }

    pub fn between(kind: &str, key: &str, from: u64, to: u64) -> Undo {
        Undo { kind: kind.into(), key: key.into(), from, to }
    }
}

impl Entry {
    pub fn new(kind: &str, text: impl Into<String>) -> Entry {
        Entry { kind: kind.into(), text: text.into(), ..Default::default() }
    }
    pub fn bytes(mut self, n: u64) -> Entry {
        self.bytes = n;
        self
    }
    pub fn items(mut self, n: u64) -> Entry {
        self.items = n;
        self
    }
    pub fn auto(mut self, on: bool) -> Entry {
        self.auto = on;
        self
    }
    pub fn undo(mut self, u: Undo) -> Entry {
        self.undo = Some(u);
        self
    }
    pub fn undoes(mut self, id: Option<String>) -> Entry {
        self.undoes = id;
        self
    }
}

/// Writes one line. It never fails what was done: a line that can't be written isn't.
pub fn record(paths: &Paths, entry: Entry) {
    if crate::elevate::is_elevated() {
        return;
    }
    let _ = write(paths, entry, now_secs());
}

fn write(paths: &Paths, mut e: Entry, at: u64) -> std::io::Result<Entry> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    e.at = at;
    e.id = format!("{at}-{:04x}{:04x}", std::process::id() & 0xffff, NEXT.fetch_add(1, Ordering::Relaxed) & 0xffff);
    e.text = one_line(&e.text);
    let dir = paths.state_dir();
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(FILE);
    if std::fs::metadata(&file).is_ok_and(|m| m.len() > MAX_BYTES) {
        // Replaces the older one: two files at most.
        let _ = std::fs::rename(&file, dir.join(OLD));
    }
    let line = serde_json::to_string(&e)?;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&file)?;
    f.write_all(format!("{line}\n").as_bytes())?;
    Ok(e)
}

/// One short line, whatever it was handed.
fn one_line(text: &str) -> String {
    let flat: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= MAX_TEXT {
        return flat;
    }
    format!("{}…", flat.chars().take(MAX_TEXT - 1).collect::<String>())
}

/// The newest part of one file, line by line. What isn't a line mom wrote is passed over.
fn read_file(path: &Path) -> Vec<Entry> {
    let Ok(mut f) = std::fs::File::open(path) else { return vec![] };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(READ_BYTES);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return vec![];
    }
    let mut buf = Vec::new();
    let _ = f.take(READ_BYTES).read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    let mut lines = text.lines();
    if start > 0 {
        lines.next(); // the end of a line that began before what was read
    }
    lines.filter(|l| l.len() <= MAX_LINE).filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// Everything on record, oldest first.
pub fn entries(paths: &Paths) -> Vec<Entry> {
    let dir = paths.state_dir();
    let mut all = read_file(&dir.join(OLD));
    all.extend(read_file(&dir.join(FILE)));
    all
}

/// A line as "What I did" shows it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shown {
    #[serde(flatten)]
    pub entry: Entry,
    /// Put back since.
    pub undone: bool,
    /// The button there is to press now, if any: "Undo", "Bring back" or "Start again".
    pub action: Option<&'static str>,
    /// It went to the Recycle Bin: that's where to look for it.
    pub in_bin: bool,
}

/// What mom did, newest first (at most `limit`), with what can still be put back.
pub fn shown(paths: &Paths, limit: usize) -> Vec<Shown> {
    shown_with(paths, &entries(paths), limit)
}

fn shown_with(paths: &Paths, all: &[Entry], limit: usize) -> Vec<Shown> {
    let undone: HashSet<&str> = all.iter().filter_map(|e| e.undoes.as_deref()).collect();
    // Archive moves and stopped servers still on their records, read once.
    let moved = if all.iter().any(|e| e.undo.as_ref().is_some_and(|u| u.kind == "archive")) { crate::archive::list(paths) } else { vec![] };
    let stopped = if all.iter().any(|e| e.undo.as_ref().is_some_and(|u| u.kind == "start")) { crate::restart::list(paths) } else { vec![] };
    // The newest line of each kind of fix that can be undone: only that one can be, since undoing
    // takes the newest backup.
    let newest = |u: &Undo| {
        all.iter().rev().find(|e| e.undo.as_ref().is_some_and(|o| o.kind == u.kind && (u.kind == "tidy" || o.key == u.key))).map(|e| e.id.as_str())
    };
    all.iter()
        .rev()
        .take(limit)
        .map(|e| {
            let gone_back = undone.contains(e.id.as_str());
            let action = match e.undo.as_ref() {
                _ if gone_back => None,
                Some(u) if u.kind == "archive" => {
                    moved.iter().any(|m| m.spot == u.key && (u.from..=u.to).contains(&m.moved_at)).then_some("Bring back")
                }
                Some(u) if u.kind == "start" => stopped.iter().any(|s| (u.from..=u.to).contains(&s.at)).then_some("Start again"),
                Some(u) if matches!(u.kind.as_str(), "tidy" | "setting" | "registry") => (newest(u) == Some(e.id.as_str())).then_some("Undo"),
                _ => None,
            };
            Shown { in_bin: e.undo.as_ref().is_some_and(|u| u.kind == "recycle-bin"), undone: gone_back, action, entry: e.clone() }
        })
        .collect()
}

/// The newest line with this kind of undo (and key) that hasn't been put back: what an undo done
/// elsewhere in the widget puts back.
pub fn last_undoable(paths: &Paths, undo_kind: &str, key: Option<&str>) -> Option<String> {
    let all = entries(paths);
    let undone: HashSet<&str> = all.iter().filter_map(|e| e.undoes.as_deref()).collect();
    all.iter()
        .rev()
        .find(|e| e.undo.as_ref().is_some_and(|u| u.kind == undo_kind && key.is_none_or(|k| u.key == k)))
        .filter(|e| !undone.contains(e.id.as_str()))
        .map(|e| e.id.clone())
}

/// Puts back what one line did, from mom's own records, and notes that it did.
pub fn undo(paths: &Paths, id: &str) -> Result<String, String> {
    let all = entries(paths);
    let shown = shown_with(paths, &all, usize::MAX);
    let line = shown.iter().find(|s| s.entry.id == id).ok_or("That isn't in mom's record.")?;
    let (Some(_), Some(u)) = (line.action, line.entry.undo.clone()) else {
        return Err("That one can't be put back now.".into());
    };
    let (kind, text, bytes, items) = match u.kind.as_str() {
        "archive" => {
            let (mut back, mut bytes, mut why) = (0u64, 0u64, None);
            for m in crate::archive::list(paths).into_iter().filter(|m| m.spot == u.key && (u.from..=u.to).contains(&m.moved_at)) {
                match crate::archive::bring_back(paths, &m.from) {
                    Ok(m) => {
                        back += 1;
                        bytes += m.bytes;
                    }
                    Err(e) => why = why.or(Some(e)),
                }
            }
            if back == 0 {
                return Err(why.unwrap_or_else(|| "None of it is in the archive any more.".into()));
            }
            (kind::BRING_BACK, format!("Brought {} back from the archive · {}", plural(back, "folder"), human_bytes(bytes)), bytes, back)
        }
        "tidy" => {
            let r = crate::tidy::undo_last(&paths.journal_dir());
            if r.restored == 0 {
                return Err(r.message);
            }
            (kind::TIDY_UNDO, r.message, 0, u64::from(r.restored))
        }
        "setting" => (kind::SETTING_UNDO, crate::winsettings::undo(paths, &u.key)?, 0, 1),
        "registry" => (kind::REGISTRY_UNDO, crate::system::undo_registry(paths, &u.key)?, 0, 1),
        "start" => {
            let mut started = Vec::new();
            let mut why = None;
            for s in crate::restart::list(paths).into_iter().filter(|s| (u.from..=u.to).contains(&s.at)) {
                match crate::restart::start(paths, &s.id) {
                    Ok(_) => started.push(s.name),
                    Err(e) => why = why.or(Some(e)),
                }
            }
            if started.is_empty() {
                return Err(why.unwrap_or_else(|| "There's nothing of it left to start.".into()));
            }
            (kind::STARTED, format!("Started {} again", names(&started)), 0, started.len() as u64)
        }
        _ => return Err("That one can't be put back.".into()),
    };
    record(paths, Entry::new(kind, text.clone()).bytes(bytes).items(items).undoes(Some(id.to_string())));
    Ok(text)
}

/// "vite and 2 more".
fn names(list: &[String]) -> String {
    match list {
        [] => String::new(),
        [one] => one.clone(),
        [first, rest @ ..] => format!("{first} and {} more", rest.len()),
    }
}

/// A clean-up's name in the middle of a sentence: "Old temp files" is "old temp files", but
/// Yarn, Cargo and Docker stay who they are.
pub(crate) fn in_sentence(label: &str) -> String {
    const COMMON: &[&str] = &["old", "other", "browser", "crash", "graphics", "game", "forgotten", "apps'"];
    let first = label.split(' ').next().unwrap_or("").to_lowercase();
    if !COMMON.contains(&first.as_str()) {
        return label.to_string();
    }
    let mut c = label.chars();
    c.next().map(|f| f.to_lowercase().chain(c).collect()).unwrap_or_default()
}

/// A clean-up as it went: what was cleared, sent to the Recycle Bin or moved to the archive.
/// `since`: when it started, so a move to the archive can be brought back from its own list.
pub fn cleaned(paths: &Paths, r: &CleanReport, pick: Option<&crate::trash::Pick>, auto: bool, since: u64) {
    if r.deleted == 0 && r.freed == 0 {
        return;
    }
    let Some(e) = clean_entry(r, pick, since, now_secs()) else { return };
    record(paths, e.auto(auto));
}

fn clean_entry(r: &CleanReport, pick: Option<&crate::trash::Pick>, since: u64, now: u64) -> Option<Entry> {
    let (label, action) = crate::trash::about(&r.id)?;
    let label = in_sentence(label);
    let size = if r.freed > 0 { format!(" · {}", human_bytes(r.freed)) } else { String::new() };
    let discard = pick.is_some_and(|p| p.discard);
    let admin = crate::trash::needs_admin(&r.id);
    let recycled = action == "recycle" || (pick.is_some_and(|p| p.recycle) && (action == "delete" || discard) && !admin);
    let e = if action == "empty-bin" {
        Entry::new(kind::EMPTY_BIN, format!("Emptied the Recycle Bin{size}"))
    } else if action == "move" && !discard {
        Entry::new(kind::ARCHIVE, format!("Moved {label} to the archive{size}")).undo(Undo::between("archive", &r.id, since, now))
    } else if recycled {
        Entry::new(kind::RECYCLE, format!("Sent {label} to the Recycle Bin{size}")).undo(Undo::new("recycle-bin", ""))
    } else if admin {
        Entry::new(kind::ADMIN_CLEAN, format!("Cleared {label}{size}"))
    } else {
        Entry::new(kind::CLEAN, format!("Cleared {label}{size}"))
    };
    Some(e.bytes(r.freed).items(r.deleted))
}

/// Programs mom turned off. `since`: when the stop began, so the servers among them can be
/// started again from the list of stopped ones.
pub fn stopped(paths: &Paths, who: &[String], r: &crate::kill::KillReport, auto: bool, since: u64) {
    if r.groups == 0 {
        return;
    }
    let shown: Vec<String> = who.iter().take(r.groups as usize).cloned().collect();
    let memory = if r.freed_mem > 0 { format!(" · {} of memory back", human_bytes(r.freed_mem)) } else { String::new() };
    let mut e = Entry::new(kind::STOPPED, format!("Turned off {}{memory}", names(&shown))).items(u64::from(r.groups)).auto(auto);
    let now = now_secs();
    if crate::restart::list(paths).iter().any(|s| (since..=now).contains(&s.at)) {
        e = e.undo(Undo::between("start", "", since, now));
    }
    record(paths, e);
}

/// A room tidied: what moved, what went to the Recycle Bin.
pub fn tidied(paths: &Paths, room: &str, r: &crate::tidy::ApplyReport) {
    let items = u64::from(r.moved + r.recycled + r.removed);
    if items == 0 {
        return;
    }
    record(paths, Entry::new(kind::TIDY, format!("Tidied {room} · {}", plural(items, "thing"))).items(items).undo(Undo::new("tidy", "")));
}

/// Copies and old files sent to the Recycle Bin from Files.
pub fn copies(paths: &Paths, r: &crate::files::RecycleReport) {
    if r.recycled.is_empty() {
        return;
    }
    let n = r.recycled.len() as u64;
    let text = format!("Sent {} to the Recycle Bin · {}", plural(n, "file"), human_bytes(r.freed));
    record(paths, Entry::new(kind::COPIES, text).bytes(r.freed).items(n).undo(Undo::new("recycle-bin", "")));
}

/// Anything else mom did, said in a line.
pub fn did(paths: &Paths, kind: &str, text: impl Into<String>) {
    record(paths, Entry::new(kind, text).items(1));
}

/// A fix mom can undo (a setting or a registry fix), under its id.
pub fn fixed(paths: &Paths, kind: &str, undo_kind: &str, id: &str, text: impl Into<String>) {
    record(paths, Entry::new(kind, text).items(1).undo(Undo::new(undo_kind, id)));
}

/// An undo done elsewhere in the widget: the newest line of that kind it put back.
pub fn put_back(paths: &Paths, kind: &str, undo_kind: &str, key: Option<&str>, text: impl Into<String>) {
    let undoes = last_undoable(paths, undo_kind, key);
    record(paths, Entry::new(kind, text).items(1).undoes(undoes));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> (std::path::PathBuf, Paths) {
        let root = std::env::temp_dir().join(format!("mom-history-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::under(&root);
        (root, paths)
    }

    #[test]
    fn lines_are_kept_newest_first_with_what_can_be_put_back() {
        let (root, paths) = fixture("lines");
        let clean = CleanReport { id: "npm-cache".into(), freed: 1_288_490_189, deleted: 40, skipped: 0, message: String::new() };
        write(&paths, clean_entry(&clean, None, 0, 1).unwrap(), 100).unwrap();
        let temp = CleanReport { id: "old-temp".into(), freed: 2048, deleted: 3, ..Default::default() };
        let pick = crate::trash::Pick { recycle: true, ..Default::default() };
        write(&paths, clean_entry(&temp, Some(&pick), 0, 1).unwrap(), 200).unwrap();
        write(&paths, Entry::new(kind::SETTING, "Put back Fast Startup").undo(Undo::new("setting", "fast-startup")), 300).unwrap();
        write(&paths, Entry::new(kind::SETTING, "Put back the mouse speed").undo(Undo::new("setting", "fast-startup")), 400).unwrap();
        write(&paths, Entry::new(kind::TIDY, "Tidied Downloads · 4 things").undo(Undo::new("tidy", "")), 500).unwrap();
        let list = shown(&paths, 50);
        let texts: Vec<&str> = list.iter().map(|s| s.entry.text.as_str()).collect();
        assert_eq!(texts, ["Tidied Downloads · 4 things", "Put back the mouse speed", "Put back Fast Startup", "Sent old temp files to the Recycle Bin · 2 KB", "Cleared npm cache · 1.2 GB"]);
        assert_eq!(list[0].action, Some("Undo"));
        assert_eq!(list[1].action, Some("Undo"), "the newest fix of a setting can be undone");
        assert_eq!(list[2].action, None, "an older one can't: undo takes the newest backup");
        assert!(list[3].in_bin && list[3].action.is_none());
        assert_eq!((list[4].entry.kind.as_str(), list[4].entry.bytes, list[4].entry.items), ("clean", 1_288_490_189, 40));
        // Something put back elsewhere in the widget marks its line as undone.
        let undoes = last_undoable(&paths, "tidy", None);
        write(&paths, Entry::new(kind::TIDY_UNDO, "Put 4 things back").undoes(undoes), 600).unwrap();
        let list = shown(&paths, 50);
        assert!(list[1].undone && list[1].action.is_none());
        assert_eq!(undo(&paths, &list[1].entry.id).unwrap_err(), "That one can't be put back now.");
        assert_eq!(undo(&paths, "nothing-like-it").unwrap_err(), "That isn't in mom's record.");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_record_stays_small_and_skips_what_it_did_not_write() {
        let (root, paths) = fixture("rotate");
        let dir = paths.state_dir();
        std::fs::create_dir_all(&dir).unwrap();
        // A full file is put aside before the next line, and only one old file is kept.
        std::fs::write(dir.join(FILE), vec![b'x'; (MAX_BYTES + 1) as usize]).unwrap();
        write(&paths, Entry::new(kind::CLEAN, "first"), 1).unwrap();
        assert!(dir.join(OLD).exists());
        assert_eq!(std::fs::read_to_string(dir.join(FILE)).unwrap().lines().count(), 1);
        // Junk, an overlong line and a broken one are passed over.
        let mut f = std::fs::OpenOptions::new().append(true).open(dir.join(FILE)).unwrap();
        writeln!(f, "not json at all").unwrap();
        writeln!(f, "{{\"text\":\"{}\"}}", "y".repeat(MAX_LINE)).unwrap();
        writeln!(f, "{{\"text\": ").unwrap();
        drop(f);
        write(&paths, Entry::new(kind::CLEAN, "second\nline\twith\u{7}controls ".repeat(40)), 2).unwrap();
        let all = entries(&paths);
        assert_eq!(all.iter().map(|e| e.at).collect::<Vec<_>>(), [1, 2]);
        assert!(all[1].text.chars().count() <= MAX_TEXT && !all[1].text.chars().any(char::is_control));
        assert!(all[1].text.starts_with("second line with controls second"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn names_read_naturally() {
        assert_eq!(in_sentence("Old temp files"), "old temp files");
        assert_eq!(in_sentence("Yarn cache"), "Yarn cache");
        assert_eq!(in_sentence("Apps' leftovers that need admin"), "apps' leftovers that need admin");
        assert_eq!(names(&["vite".into(), "next".into(), "atlas".into()]), "vite and 2 more");
        let bin = CleanReport { id: "recycle-bin".into(), freed: 5 << 30, deleted: 9, ..Default::default() };
        assert_eq!(clean_entry(&bin, None, 0, 1).unwrap().text, "Emptied the Recycle Bin · 5.0 GB");
        let moved = CleanReport { id: "old-conversations".into(), freed: 3 << 30, deleted: 2, ..Default::default() };
        let e = clean_entry(&moved, None, 10, 20).unwrap();
        assert_eq!((e.text.as_str(), e.undo), ("Moved old conversations to the archive · 3.0 GB", Some(Undo::between("archive", "old-conversations", 10, 20))));
        let discarded = crate::trash::Pick { discard: true, goes: false, ..Default::default() };
        let gone = CleanReport { id: "forgotten-apps".into(), freed: 10, deleted: 1, ..Default::default() };
        assert_eq!(clean_entry(&gone, Some(&discarded), 0, 1).unwrap().kind, kind::CLEAN, "discarded, not moved");
        let admin = CleanReport { id: "windows-junk".into(), freed: 10, deleted: 1, ..Default::default() };
        assert_eq!(clean_entry(&admin, None, 0, 1).unwrap().kind, kind::ADMIN_CLEAN);
        assert!(clean_entry(&CleanReport { id: "no-such-spot".into(), ..Default::default() }, None, 0, 1).is_none());
    }
}
