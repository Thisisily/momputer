//! Homework check: what each agent session did, from its own log, next to what it said.
//!
//! Claude Code keeps a log per session (`~/.claude/projects/<project>/<session>.jsonl`) and Codex
//! one per thread (`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`). From the last week's newest
//! sessions mom writes a receipt: the files it changed, what it deleted (and whether that was
//! outside its project), what it installed and pushed, how many commands it ran, and how its last
//! test or build went. When its last words say the tests pass and its last test run failed, the
//! receipt says so; so does one that switched a feature or tests off in the code, threw away
//! changes that weren't committed, put files back to an older version, changed a release on
//! GitHub, skipped the checks before a commit or changed Microsoft Defender's settings.
//!
//! Facts only: mom can't prove anything, she puts the facts beside the claim. Commands are read
//! as text and never run. Every log of the week is read, several at a time, line by line, at most
//! its last 128 MB.

use crate::paths::Paths;
use crate::util::{ellipsize, norm_path, now_secs};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How far back, and how many sessions.
pub const DAYS: u64 = 7;
const MAX_SESSIONS: usize = 600;
/// The most of one log that's read: its latest part.
const MAX_BYTES: u64 = 128 << 20;
/// Lines longer than this (pictures, huge outputs) are passed over unread.
const MAX_LINE: usize = 2 << 20;
/// Paths, installs and pushes kept per receipt. The counts go on.
const KEEP: usize = 50;
/// The most of a command's output that's searched for signs of failure: its end.
const OUTPUT_TAIL: usize = 20_000;
/// A whole look takes at most this long; what's left waits for the next one.
const MAX_TIME: Duration = Duration::from_secs(45);
/// Receipts are kept this long before mom reads the logs again.
const KEEP_FOR: u64 = 60;

pub const FLAG_OUTSIDE: &str = "Deleted something outside its project";
pub const FLAG_FORCE: &str = "Force-pushed";
pub const FLAG_CLAIM: &str = "Said tests pass; the last test run failed";
pub const FLAG_TESTS_OFF: &str = "Turned tests off";
pub const FLAG_DISCARDED: &str = "Threw away changes that weren't committed";
pub const FLAG_DEFENDER: &str = "Changed Microsoft Defender's settings";
pub const FLAG_NO_VERIFY: &str = "Skipped the checks before committing";
pub const FLAG_ROLLED_BACK: &str = "Put files back to an older version";
pub const FLAG_RELEASE: &str = "Changed a release on GitHub";

/// A feature turned off in the code: "Switched FILE_CLEANUP_AVAILABLE off".
pub fn flag_switched_off(name: &str) -> String {
    format!("Switched {name} off")
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    /// "Claude Code" or "Codex".
    pub agent: String,
    /// The session's (or Codex thread's) id.
    pub id: String,
    /// The folder it worked in.
    pub cwd: String,
    pub project: Option<String>,
    pub title: String,
    /// Unix seconds: its first and last line read.
    pub started: u64,
    pub ended: u64,
    /// Shell commands it ran.
    pub commands: u32,
    /// Files it wrote or edited, newest last.
    pub changed: Vec<String>,
    pub changed_count: u32,
    pub deleted: Vec<Deletion>,
    pub deleted_count: u32,
    /// Install commands, as it typed them.
    pub installs: Vec<String>,
    pub pushes: Vec<Push>,
    /// Its last test or build, and how it went.
    pub last_check: Option<CheckRun>,
    /// What deserves a look, in plain words (the FLAG_ constants).
    pub flags: Vec<String>,
    /// The log was long: only its latest part was read.
    pub partial: bool,
    /// Opens it in its app, when there's a way (Codex threads).
    pub link: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Deletion {
    pub path: String,
    /// Outside its project folder (and not in Temp).
    pub outside: bool,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Push {
    pub command: String,
    pub force: bool,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CheckRun {
    pub command: String,
    pub passed: bool,
    pub at: u64,
}

// ------------------------------------------------------------------ the look, kept a minute

/// Each log's receipt, by path, with the size and time it had: unchanged logs aren't read again.
type Seen = HashMap<PathBuf, (u64, u64, Receipt)>;

struct Cache {
    at: u64,
    list: Vec<Receipt>,
    files: Seen,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// The newest sessions of the last week, newest first. From the last minute unless `fresh`.
pub fn recent(paths: &Paths, fresh: bool) -> Vec<Receipt> {
    let now = now_secs();
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = guard.as_ref() {
        if !fresh && now.saturating_sub(c.at) < KEEP_FOR {
            return c.list.clone();
        }
    }
    let old = guard.take().map(|c| c.files).unwrap_or_default();
    let (list, files) = look(paths, now, old);
    *guard = Some(Cache { at: now, list: list.clone(), files });
    list
}

fn look(paths: &Paths, now: u64, old: Seen) -> (Vec<Receipt>, Seen) {
    let since = now.saturating_sub(DAYS * 86_400);
    let mut logs = claude_logs(paths, since);
    logs.extend(codex_logs(paths, since));
    logs.sort_by(|a, b| b.2.cmp(&a.2));
    logs.truncate(MAX_SESSIONS);
    let ctx = Places::new(paths);
    // Logs that changed since the last look are read again, several at a time; what isn't
    // reached in time waits for the next look.
    let fresh: Vec<usize> = (0..logs.len()).filter(|&i| !old.get(&logs[i].1).is_some_and(|(l, m, _)| *l == logs[i].3 && *m == logs[i].2)).collect();
    let fresh_paths: Vec<PathBuf> = fresh.iter().map(|&i| logs[i].1.clone()).collect();
    let agents: HashMap<&Path, (Agent, u64)> = logs.iter().map(|(a, p, _, len)| (p.as_path(), (*a, *len))).collect();
    let read = crate::repos::each(&fresh_paths, Instant::now() + MAX_TIME, |p| {
        let (agent, len) = agents[p];
        Some(match agent {
            Agent::Claude => read_claude(p, len, &ctx),
            Agent::Codex => read_codex(p, len, &ctx),
        })
    });
    let mut read: HashMap<PathBuf, Receipt> = fresh_paths.into_iter().zip(read).filter_map(|(p, r)| Some((p, r?))).collect();
    let mut files = HashMap::new();
    let mut list = Vec::new();
    for (_, path, mtime, len) in logs {
        let receipt = match old.get(&path) {
            Some((l, m, r)) if *l == len && *m == mtime => r.clone(),
            _ => match read.remove(&path) {
                Some(r) => r,
                None => continue, // the next look reads it
            },
        };
        files.insert(path, (len, mtime, receipt.clone()));
        // A session that did nothing (a review that only read, one that only talked) isn't listed.
        if receipt.commands == 0 && receipt.changed_count == 0 && receipt.deleted_count == 0 {
            continue;
        }
        list.push(receipt);
    }
    // Codex threads get the names Codex gave them.
    let ids: Vec<String> = list.iter().filter(|r| r.agent == CODEX).map(|r| r.id.clone()).collect();
    if !ids.is_empty() {
        let names = crate::codex::thread_names(&paths.codex_dir(), &ids);
        for r in list.iter_mut().filter(|r| r.agent == CODEX) {
            if let Some(n) = names.get(&r.id).filter(|n| !n.trim().is_empty()) {
                r.title = plain(n, 90);
            }
        }
    }
    list.sort_by(|a, b| b.ended.cmp(&a.ended));
    (list, files)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Agent {
    Claude,
    Codex,
}

const CLAUDE: &str = "Claude Code";
const CODEX: &str = "Codex";

fn stamp(path: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(path).ok()?;
    m.is_file().then(|| (crate::util::mtime_secs(&m), m.len()))
}

/// Claude Code's session logs changed since `since`: (agent, path, changed, size).
fn claude_logs(paths: &Paths, since: u64) -> Vec<(Agent, PathBuf, u64, u64)> {
    let mut out = Vec::new();
    let Ok(projects) = std::fs::read_dir(paths.claude_dir().join("projects")) else { return out };
    for project in projects.flatten().take(2000) {
        let Ok(files) = std::fs::read_dir(project.path()) else { continue };
        for f in files.flatten().take(5000) {
            let name = f.file_name().to_string_lossy().to_string();
            if !name.ends_with(".jsonl") || name.starts_with("agent-") {
                continue;
            }
            let path = f.path();
            if let Some((mtime, len)) = stamp(&path).filter(|(m, _)| *m >= since) {
                out.push((Agent::Claude, path, mtime, len));
            }
        }
    }
    out
}

/// Codex's thread logs changed since `since`, from the day folders that can hold them.
fn codex_logs(paths: &Paths, since: u64) -> Vec<(Agent, PathBuf, u64, u64)> {
    use chrono::{Datelike, TimeZone};
    let mut out = Vec::new();
    let root = paths.codex_dir().join("sessions");
    let Some(first) = chrono::Local.timestamp_opt(since as i64, 0).single() else { return out };
    // A thread lives in the folder of the day it began; a long one may have begun earlier.
    let mut day = first.date_naive() - chrono::Duration::days(DAYS as i64);
    let today = chrono::Local::now().date_naive();
    while day <= today {
        let dir = root.join(format!("{:04}", day.year())).join(format!("{:02}", day.month())).join(format!("{:02}", day.day()));
        if let Ok(files) = std::fs::read_dir(&dir) {
            for f in files.flatten().take(5000) {
                let name = f.file_name().to_string_lossy().to_string();
                if !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
                    continue;
                }
                let path = f.path();
                if let Some((mtime, len)) = stamp(&path).filter(|(m, _)| *m >= since) {
                    out.push((Agent::Codex, path, mtime, len));
                }
            }
        }
        day += chrono::Duration::days(1);
    }
    out
}

/// Opens a log at its latest MAX_BYTES, on a line boundary. True if anything before was skipped.
fn open_tail(path: &Path, len: u64) -> Option<(std::io::BufReader<std::io::Take<std::fs::File>>, bool)> {
    let mut f = std::fs::File::open(path).ok()?;
    let partial = len > MAX_BYTES;
    if partial {
        f.seek(SeekFrom::Start(len - MAX_BYTES)).ok()?;
    }
    let mut reader = std::io::BufReader::new(f.take(MAX_BYTES));
    if partial {
        let mut skip = Vec::new();
        let _ = reader.read_until(b'\n', &mut skip); // the first line is cut in half
    }
    Some((reader, partial))
}

/// Calls `each` with every line short enough to read.
fn lines(reader: &mut impl BufRead, mut each: impl FnMut(&[u8])) {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) if buf.len() > MAX_LINE => continue,
            Ok(_) => each(&buf),
        }
    }
}

fn contains(line: &[u8], needle: &str) -> bool {
    line.windows(needle.len()).any(|w| w == needle.as_bytes())
}

/// A line's own "timestamp", without reading the rest of it.
fn quick_ts(line: &[u8]) -> Option<u64> {
    let key = b"\"timestamp\":\"";
    let at = line.windows(key.len()).position(|w| w == key)? + key.len();
    let end = line.get(at..)?.iter().take(64).position(|&c| c == b'"')? + at;
    let s = std::str::from_utf8(&line[at..end]).ok()?;
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp().max(0) as u64)
}

/// The largest char boundary at or before `i`.
fn boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ts(v: &Value) -> Option<u64> {
    let t = v.get("timestamp")?.as_str()?;
    chrono::DateTime::parse_from_rfc3339(t).ok().map(|d| d.timestamp().max(0) as u64)
}

/// Text from a message's content: a string, or a list of parts with "text".
fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

// ------------------------------------------------------------------ building a receipt

#[derive(Default)]
struct Build {
    r: Receipt,
    /// Every deletion with the folder its command ran in, and whether git looks after it: whether
    /// each was outside the project is decided once the whole log is read.
    deletions: Vec<(String, String, bool)>,
    seq: u64,
    /// The last test or build, with when it ran (in lines).
    check: Option<(CheckRun, u64)>,
    /// Its last words, with when they were said.
    last_words: Option<(String, u64)>,
    first_ask: Option<String>,
    /// The flags its commands and edits earned, besides the ones decided at the end, in order.
    noticed: Vec<String>,
}

impl Build {
    fn time(&mut self, at: Option<u64>) {
        if let Some(t) = at {
            if self.r.started == 0 || t < self.r.started {
                self.r.started = t;
            }
            self.r.ended = self.r.ended.max(t);
        }
    }
    fn changed(&mut self, path: &str) {
        let p = plain(path, 260);
        if p.is_empty() {
            return;
        }
        if let Some(i) = self.r.changed.iter().position(|x| x.eq_ignore_ascii_case(&p)) {
            self.r.changed.remove(i); // newest last
        } else {
            self.r.changed_count += 1;
        }
        self.r.changed.push(p);
        if self.r.changed.len() > KEEP {
            self.r.changed.remove(0);
        }
    }
    fn deleted(&mut self, path: &str, cwd: &str, git: bool) {
        self.r.deleted_count += 1;
        if self.deletions.len() < 2000 {
            self.deletions.push((plain(path, 260), cwd.to_string(), git));
        }
    }
    /// One command line it ran. Returns the test or build in it, if any.
    fn command(&mut self, cmd: &str, cwd: &str, places: &Places) -> Option<String> {
        self.r.commands += 1;
        let mut check = None;
        for seg in segments(cmd) {
            let words = unwrap(tokens(&seg.text));
            if words.is_empty() {
                continue;
            }
            if let Some(inner) = shell_script(&words) {
                if let Some(c) = self.inner_command(&inner, cwd, places) {
                    check = Some(c);
                }
                continue;
            }
            for (path, git) in deletions(&words) {
                self.deleted(&path, cwd, git);
            }
            if is_install(&words) && self.r.installs.len() < KEEP {
                self.r.installs.push(plain(&seg.text, 160));
            }
            if let Some(force) = push(&words) {
                if self.r.pushes.len() < KEEP {
                    self.r.pushes.push(Push { command: plain(&seg.text, 160), force });
                }
            }
            if is_check(&words) {
                check = Some(plain(&seg.text, 160));
            }
            if let Some(flag) = risky(&words) {
                self.notice(flag);
            }
        }
        check
    }
    fn notice(&mut self, flag: impl Into<String>) {
        let flag = flag.into();
        if !self.noticed.contains(&flag) {
            self.noticed.push(flag);
        }
    }
    /// Lines a file gained (from an edit, a new file or a patch): a feature or tests switched off.
    fn added<'a>(&mut self, path: &str, lines: impl Iterator<Item = &'a str>) {
        if !is_code(path) {
            return;
        }
        for line in lines.take(20_000) {
            if line.len() > 400 {
                continue;
            }
            if let Some(name) = switched_off(line) {
                self.notice(flag_switched_off(&plain(name, 60)));
            }
            if tests_off(line) {
                self.notice(FLAG_TESTS_OFF);
            }
        }
    }
    /// A script handed to a shell inside a command (powershell -Command "…"): the same, without
    /// counting another command.
    fn inner_command(&mut self, cmd: &str, cwd: &str, places: &Places) -> Option<String> {
        let found = self.command(cmd, cwd, places);
        self.r.commands -= 1;
        found
    }
    fn check_result(&mut self, command: String, passed: bool, at: Option<u64>) {
        self.seq += 1;
        let run = CheckRun { command, passed, at: at.unwrap_or(self.r.ended) };
        self.check = Some((run, self.seq));
    }
    fn words(&mut self, text: &str) {
        let t = text.trim();
        if !t.is_empty() {
            self.seq += 1;
            self.last_words = Some((ellipsize(t, 4000), self.seq));
        }
    }
    fn finish(mut self, places: &Places) -> Receipt {
        // Outside means outside both the folder its command ran in and the session's project.
        let project = self.r.cwd.clone();
        let mut deleted: Vec<Deletion> = std::mem::take(&mut self.deletions)
            .into_iter()
            .map(|(path, cwd, git)| {
                let base = if cwd.is_empty() { &project } else { &cwd };
                let outside = !git && !project.is_empty() && places.place(&path, base) == Place::Outside && places.place(&path, &project) != Place::Inside;
                Deletion { path, outside }
            })
            .collect();
        deleted.sort_by_key(|d| !d.outside); // the ones worth a look first
        deleted.truncate(KEEP);
        self.r.deleted = deleted;
        self.r.project = crate::scan::project_name(&self.r.cwd)
            .or_else(|| Path::new(&self.r.cwd).file_name().map(|n| n.to_string_lossy().to_string()));
        if self.r.title.is_empty() {
            self.r.title = self.first_ask.take().map(|t| plain(&t, 90)).unwrap_or_default();
        }
        let failed_last = self.check.as_ref().filter(|(c, _)| !c.passed).map(|(_, at)| *at);
        if self.r.deleted.first().is_some_and(|d| d.outside) {
            self.r.flags.push(FLAG_OUTSIDE.into());
        }
        if self.r.pushes.iter().any(|p| p.force) {
            self.r.flags.push(FLAG_FORCE.into());
        }
        if let (Some(fail_at), Some((words, said_at))) = (failed_last, &self.last_words) {
            if *said_at > fail_at && claims_pass(words) {
                self.r.flags.push(FLAG_CLAIM.into());
            }
        }
        // The rest by how much each deserves a look: a feature or tests switched off first.
        let rank = |f: &String| {
            [FLAG_TESTS_OFF, FLAG_DISCARDED, FLAG_DEFENDER, FLAG_NO_VERIFY, FLAG_ROLLED_BACK, FLAG_RELEASE].iter().position(|x| x == f).map_or(0, |i| i + 1)
        };
        let mut noticed = std::mem::take(&mut self.noticed);
        noticed.sort_by_key(rank);
        self.r.flags.extend(noticed);
        self.r.last_check = self.check.map(|(c, _)| c);
        self.r
    }
}

/// Firmware, log and file names made safe to show and to put in a prompt: one line, no control
/// characters, not long.
fn plain(s: &str, max: usize) -> String {
    let one: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    ellipsize(one.split_whitespace().collect::<Vec<_>>().join(" ").as_str(), max)
}

// ------------------------------------------------------------------ Claude Code

fn read_claude(path: &Path, len: u64, places: &Places) -> Receipt {
    let mut b = Build::default();
    b.r.agent = CLAUDE.into();
    b.r.id = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let Some((mut reader, partial)) = open_tail(path, len) else { return b.r };
    b.r.partial = partial;
    // Tests and builds waiting for their result, by tool call id.
    let mut pending: HashMap<String, String> = HashMap::new();
    lines(&mut reader, |line| {
        let assistant = contains(line, "\"type\":\"assistant\"");
        let result = contains(line, "\"tool_result\"") && pending.keys().any(|id| contains(line, id));
        let user_text = b.first_ask.is_none() && contains(line, "\"type\":\"user\"") && !contains(line, "\"tool_result\"");
        if !assistant && !result && !user_text {
            b.time(quick_ts(line));
            return;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else { return };
        let at = ts(&v);
        b.time(at);
        if b.r.cwd.is_empty() {
            if let Some(c) = v.get("cwd").and_then(Value::as_str).filter(|c| !c.is_empty()) {
                b.r.cwd = c.trim_start_matches(r"\\?\").to_string();
            }
        }
        let content = v.pointer("/message/content");
        if assistant {
            for part in content.and_then(Value::as_array).into_iter().flatten() {
                match part.get("type").and_then(Value::as_str) {
                    Some("tool_use") => {
                        let input = part.get("input").unwrap_or(&Value::Null);
                        match part.get("name").and_then(Value::as_str).unwrap_or_default() {
                            "Bash" | "PowerShell" => {
                                let cmd = input.get("command").and_then(Value::as_str).unwrap_or_default();
                                let cwd = b.r.cwd.clone();
                                let check = b.command(cmd, &cwd, places);
                                let background = input.get("run_in_background").and_then(Value::as_bool).unwrap_or(false);
                                if let (Some(c), false, Some(id)) = (check, background, part.get("id").and_then(Value::as_str)) {
                                    if pending.len() < 64 {
                                        pending.insert(id.to_string(), c);
                                    }
                                }
                            }
                            "Edit" | "Write" | "MultiEdit" => {
                                if let Some(p) = input.get("file_path").and_then(Value::as_str) {
                                    b.changed(p);
                                    let edits = input.get("edits").and_then(Value::as_array).cloned().unwrap_or_else(|| vec![input.clone()]);
                                    for e in &edits {
                                        let old = e.get("old_string").and_then(Value::as_str).unwrap_or_default();
                                        let new = e.get("new_string").or_else(|| e.get("content")).and_then(Value::as_str).unwrap_or_default();
                                        b.added(p, new.lines().filter(|l| !old.contains(l.trim())));
                                    }
                                }
                            }
                            "NotebookEdit" => {
                                if let Some(p) = input.get("notebook_path").and_then(Value::as_str) {
                                    b.changed(p);
                                }
                            }
                            _ => {}
                        }
                    }
                    Some("text") => b.words(part.get("text").and_then(Value::as_str).unwrap_or_default()),
                    _ => {}
                }
            }
        } else if result {
            for part in content.and_then(Value::as_array).into_iter().flatten() {
                let Some(id) = part.get("tool_use_id").and_then(Value::as_str) else { continue };
                let Some(command) = pending.remove(id) else { continue };
                let out = text_of(part.get("content").unwrap_or(&Value::Null));
                if out.contains("Command running in background") {
                    continue; // its result comes later, somewhere mom doesn't read
                }
                let error = part.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                let passed = !error && exit_code(&out).is_none_or(|c| c == 0) && !looks_failed(&out);
                b.check_result(command, passed, at);
            }
        } else if user_text && v.get("isMeta").and_then(Value::as_bool) != Some(true) {
            let t = content.map(text_of).unwrap_or_default();
            if !t.trim().is_empty() && !t.trim_start().starts_with('<') {
                b.first_ask = Some(t);
            }
        }
    });
    if let Some(t) = crate::trash::claude_session_title(path) {
        b.r.title = plain(&t, 90);
    }
    b.finish(places)
}

/// "Exit code 2" at the start of a tool's output.
fn exit_code(out: &str) -> Option<i64> {
    out.trim_start().strip_prefix("Exit code ")?.split_whitespace().next()?.parse().ok()
}

/// Signs of a failed test or build in the end of its output, for runs piped into something else
/// (whose own exit code says nothing).
fn looks_failed(out: &str) -> bool {
    let tail = &out[boundary(out, out.len().saturating_sub(OUTPUT_TAIL))..];
    static FAILED: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = FAILED.get_or_init(|| {
        regex::Regex::new(
            r"(?m)test result: FAILED|^# fail [1-9]|\b[1-9]\d* (?:failed|failing)\b|Tests?:\s+[1-9]\d* failed|npm ERR!|error\[E\d{4}\]|error: could not compile|Build FAILED|BUILD FAILED|FAILURES!|^FAILED ",
        )
        .expect("a fixed pattern")
    });
    re.is_match(tail)
}

/// Its last words say the tests pass, or that it's all fixed and working.
fn claims_pass(words: &str) -> bool {
    static CLAIM: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = CLAIM.get_or_init(|| {
        regex::Regex::new(
            r"(?i)\b(?:(?:all|the|every)\s+)?(?:tests?|checks?|specs?)\s+(?:now\s+|all\s+|still\s+)?(?:pass(?:es|ed|ing)?|are\s+(?:passing|green)|(?:is|are)\s+green|succeed(?:s|ed)?)\b|\ball\s+green\b|\beverything\s+(?:works|passes|is\s+working|is\s+green)\b|\b(?:it|this)\s+works\s+now\b|\bworks\s+now\b|\b(?:is|are|all|now)\s+fixed\b|\bfixed\s+(?:it|the|this|that)\b",
        )
        .expect("a fixed pattern")
    });
    re.find_iter(words).any(|m| {
        // "tests don't pass", "not all tests pass": a sentence that says no isn't a claim.
        let start = boundary(words, m.start().saturating_sub(40));
        let before = words[start..m.start()].to_ascii_lowercase();
        let clause = before.rsplit(['.', '!', '?', '\n']).next().unwrap_or_default();
        !["not ", "n't", "no ", "fail", "never", "nothing", "until", "once ", "if ", "whether"].iter().any(|w| clause.contains(w))
            && !words[m.end()..].trim_start().starts_with('?')
    })
}

// ------------------------------------------------------------------ Codex

fn read_codex(path: &Path, len: u64, places: &Places) -> Receipt {
    let mut b = Build::default();
    b.r.agent = CODEX.into();
    // "rollout-2026-10-14T21-57-46-<thread id>.jsonl"
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    b.r.id = stem.get(stem.len().saturating_sub(36)..).unwrap_or_default().to_string();
    let Some((mut reader, partial)) = open_tail(path, len) else { return b.r };
    b.r.partial = partial;
    // Older Codex wrote its events differently; they're used only until the newer ones turn up.
    let mut newer = false;
    lines(&mut reader, |line| {
        let item = contains(line, "\"item_completed\"");
        let wanted = item
            || contains(line, "\"session_meta\"")
            || contains(line, "\"turn_context\"")
            || contains(line, "\"task_complete\"")
            || contains(line, "\"exec_command_end\"")
            || contains(line, "\"patch_apply_end\"")
            || contains(line, "\"agent_message\"")
            || contains(line, "\"user_message\"");
        if !wanted {
            b.time(quick_ts(line));
            return;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else { return };
        let at = ts(&v);
        b.time(at);
        let p = v.get("payload").unwrap_or(&Value::Null);
        match (v.get("type").and_then(Value::as_str), p.get("type").and_then(Value::as_str)) {
            (Some("session_meta"), _) => {
                if let Some(id) = p.get("id").and_then(Value::as_str).filter(|id| id.len() <= 64) {
                    b.r.id = id.to_string();
                }
                if let Some(c) = p.get("cwd").and_then(Value::as_str).filter(|c| !c.is_empty()) {
                    b.r.cwd = c.trim_start_matches(r"\\?\").to_string();
                }
            }
            (Some("turn_context"), _) => {
                if b.r.cwd.is_empty() {
                    if let Some(c) = p.get("cwd").and_then(Value::as_str) {
                        b.r.cwd = c.trim_start_matches(r"\\?\").to_string();
                    }
                }
            }
            (_, Some("item_completed")) => {
                newer = true;
                codex_item(&mut b, p.get("item").unwrap_or(&Value::Null), at, places);
            }
            (_, Some("task_complete")) => {
                if let Some(t) = p.get("last_agent_message").and_then(Value::as_str) {
                    b.words(t);
                }
            }
            (Some("event_msg"), Some("exec_command_end" | "patch_apply_end" | "agent_message" | "user_message")) if !newer => {
                codex_older(&mut b, p, places);
            }
            _ => {}
        }
    });
    let mut r = b.finish(places);
    if r.id.len() == 36 && r.id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        r.link = Some(format!("codex://threads/{}", r.id));
    }
    r
}

/// The script a command list runs: `["powershell.exe", "-Command", "…"]` is "…".
fn command_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => {
            let words: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
            words.iter().map(|w| if w.contains(' ') { format!("\"{w}\"") } else { w.to_string() }).collect::<Vec<_>>().join(" ")
        }
        _ => String::new(),
    }
}

fn codex_item(b: &mut Build, item: &Value, at: Option<u64>, places: &Places) {
    match item.get("type").and_then(Value::as_str).unwrap_or_default() {
        "CommandExecution" => {
            let cmd = command_text(item.get("command").unwrap_or(&Value::Null));
            let cwd = item.get("cwd").and_then(Value::as_str).map(|c| c.trim_start_matches(r"\\?\").to_string()).unwrap_or_else(|| b.r.cwd.clone());
            if let Some(check) = b.command(&cmd, &cwd, places) {
                let code = item.get("exit_code").and_then(Value::as_i64);
                let failed_status = item.get("status").and_then(Value::as_str) == Some("failed");
                let out = item.get("aggregated_output").and_then(Value::as_str).unwrap_or_default();
                let passed = code.is_none_or(|c| c == 0) && !failed_status && !looks_failed(out);
                b.check_result(check, passed, at);
            }
        }
        "FileChange" => {
            for (path, change) in item.get("changes").and_then(Value::as_object).into_iter().flatten().take(2000) {
                file_change(b, path, change);
            }
        }
        "AgentMessage" => b.words(&text_of(item.get("content").unwrap_or(&Value::Null))),
        "UserMessage" => {
            if b.first_ask.is_none() {
                let t = text_of(item.get("content").unwrap_or(&Value::Null));
                if !t.trim().is_empty() && !t.trim_start().starts_with('<') {
                    b.first_ask = Some(t);
                }
            }
        }
        _ => {}
    }
}

/// One file in a patch: `{"type": "add" | "update" | "delete", "move_path": …}`, or the older
/// `{"add": {…}}` / `{"update": {…}}` / `{"delete": {…}}`.
fn file_change(b: &mut Build, path: &str, change: &Value) {
    let kind = change
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| change.as_object().and_then(|o| o.keys().find(|k| ["add", "update", "delete"].contains(&k.as_str())).cloned()))
        .unwrap_or_default();
    if kind == "delete" {
        let cwd = b.r.cwd.clone();
        b.deleted(path, &cwd, false);
    } else {
        let moved = change.get("move_path").or_else(|| change.pointer("/update/move_path")).and_then(Value::as_str);
        b.changed(moved.unwrap_or(path));
        let diff = change.get("unified_diff").or_else(|| change.pointer("/update/unified_diff")).and_then(Value::as_str);
        let content = change.get("content").or_else(|| change.pointer("/add/content")).and_then(Value::as_str);
        if let Some(d) = diff {
            b.added(path, d.lines().filter(|l| l.starts_with('+') && !l.starts_with("+++")).map(|l| &l[1..]));
        } else if let Some(c) = content.filter(|_| kind == "add") {
            b.added(path, c.lines());
        }
    }
}

fn codex_older(b: &mut Build, p: &Value, places: &Places) {
    match p.get("type").and_then(Value::as_str).unwrap_or_default() {
        "exec_command_end" => {
            let cmd = command_text(p.get("command").unwrap_or(&Value::Null));
            let cwd = b.r.cwd.clone();
            if let Some(check) = b.command(&cmd, &cwd, places) {
                let code = p.get("exit_code").and_then(Value::as_i64);
                let out = p.get("aggregated_output").or_else(|| p.get("stdout")).and_then(Value::as_str).unwrap_or_default();
                b.check_result(check, code.is_none_or(|c| c == 0) && !looks_failed(out), None);
            }
        }
        "patch_apply_end" => {
            for (path, change) in p.get("changes").and_then(Value::as_object).into_iter().flatten().take(2000) {
                file_change(b, path, change);
            }
        }
        "agent_message" => b.words(p.get("message").and_then(Value::as_str).unwrap_or_default()),
        "user_message" => {
            if b.first_ask.is_none() {
                let t = p.get("message").and_then(Value::as_str).unwrap_or_default();
                if !t.trim().is_empty() && !t.trim_start().starts_with('<') {
                    b.first_ask = Some(t.to_string());
                }
            }
        }
        _ => {}
    }
}

// ------------------------------------------------------------------ reading commands

/// One piece of a command line, between `&&`, `||`, `;`, `|` and line breaks.
struct Segment {
    text: String,
}

/// A command line's pieces, split outside quotes.
fn segments(cmd: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                cur.push(c);
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => {
                    quote = Some(c);
                    cur.push(c);
                }
                ';' | '\n' | '|' | '&' => {
                    if c == '&' && chars.peek() != Some(&'&') {
                        cur.push(c); // "2>&1", "& 'C:\\x.exe'"
                        continue;
                    }
                    if (c == '|' && chars.peek() == Some(&'|')) || c == '&' {
                        chars.next();
                    }
                    if !cur.trim().is_empty() {
                        out.push(Segment { text: cur.trim().to_string() });
                    }
                    cur.clear();
                }
                _ => cur.push(c),
            },
        }
        if out.len() > 200 {
            break;
        }
    }
    if !cur.trim().is_empty() {
        out.push(Segment { text: cur.trim().to_string() });
    }
    out
}

/// Words of one piece, with quotes taken off.
fn tokens(seg: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in seg.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                started = true;
            }
            None if c.is_whitespace() => {
                if started || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            None => cur.push(c),
        }
        if out.len() > 400 {
            break;
        }
    }
    if started || !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn base(word: &str) -> String {
    let w = word.rsplit(['/', '\\']).next().unwrap_or(word).to_ascii_lowercase();
    w.strip_suffix(".exe").or_else(|| w.strip_suffix(".cmd")).map(str::to_string).unwrap_or(w)
}

/// Leaves out what only wraps a command: `sudo`, `time`, `&`, `cd x;` and the like.
fn unwrap(mut words: Vec<String>) -> Vec<String> {
    while let Some(first) = words.first() {
        let b = base(first);
        if ["sudo", "time", "nohup", "command", "exec", "&", "."].contains(&b.as_str()) {
            words.remove(0);
        } else {
            break;
        }
    }
    words
}

/// The script given to a shell inside the command: `powershell -Command "…"`, `bash -lc "…"`,
/// `cmd /c …`.
fn shell_script(words: &[String]) -> Option<String> {
    let b = base(words.first()?);
    let flag = |list: &[&str]| words.iter().position(|w| list.contains(&w.to_ascii_lowercase().as_str()));
    let at = match b.as_str() {
        "powershell" | "pwsh" => flag(&["-command", "-c"])?,
        "bash" | "sh" | "zsh" | "wsl" => flag(&["-c", "-lc", "-ic"])?,
        "cmd" => flag(&["/c", "/k"])?,
        _ => return None,
    };
    let rest = &words[at + 1..];
    (!rest.is_empty()).then(|| rest.join(" "))
}

/// What a piece deletes: (path, inside git's own care). `git clean` and `git rm` stay in the repo.
fn deletions(words: &[String]) -> Vec<(String, bool)> {
    let verb = base(&words[0]);
    let lower: Vec<String> = words.iter().map(|w| w.to_ascii_lowercase()).collect();
    if verb == "git" {
        let sub = lower.iter().skip(1).find(|w| !w.starts_with('-')).map(String::as_str).unwrap_or_default();
        return match sub {
            "clean" if lower.iter().any(|w| w.starts_with('-') && !w.starts_with("--") && w.contains('f') || w == "--force") => {
                vec![("untracked files (git clean)".into(), true)]
            }
            "rm" => lower
                .iter()
                .enumerate()
                .skip_while(|(_, w)| w.as_str() != "rm")
                .skip(1)
                .filter(|(_, w)| !w.starts_with('-'))
                .map(|(i, _)| (words[i].clone(), true))
                .take(20)
                .collect(),
            _ => Vec::new(),
        };
    }
    let is_delete = matches!(verb.as_str(), "rm" | "rmdir" | "rd" | "del" | "erase" | "unlink" | "shred" | "remove-item" | "ri");
    if !is_delete {
        return Vec::new();
    }
    // PowerShell parameters that take a value which isn't a path.
    let takes_value = ["-include", "-exclude", "-filter", "-credential", "-stream"];
    let mut out = Vec::new();
    let mut skip_next = false;
    for (i, w) in words.iter().enumerate().skip(1) {
        let l = &lower[i];
        if skip_next {
            skip_next = false;
            continue;
        }
        if takes_value.contains(&l.as_str()) {
            skip_next = true;
            continue;
        }
        if l.starts_with('-') || (l.starts_with('/') && l.len() <= 3 && (verb == "del" || verb == "rd" || verb == "rmdir" || verb == "erase")) {
            continue; // flags: -rf, -Recurse, -Force, -Path (its value is a path), /s /q
        }
        if l.starts_with('>') || l.starts_with("2>") || l == "$null" {
            continue;
        }
        out.push((w.trim_end_matches(',').to_string(), false));
        if out.len() >= 20 {
            break;
        }
    }
    out
}

fn is_install(words: &[String]) -> bool {
    let l: Vec<String> = words.iter().map(|w| w.to_ascii_lowercase()).collect();
    let verb = base(&l[0]);
    let sub = |i: usize| l.get(i).map(String::as_str).unwrap_or_default();
    match verb.as_str() {
        "npm" | "pnpm" => matches!(sub(1), "i" | "install" | "add") && l.iter().skip(2).any(|w| !w.starts_with('-')) || l.iter().any(|w| w == "-g" || w == "--global") && matches!(sub(1), "i" | "install" | "add"),
        "yarn" => sub(1) == "add" || (sub(1) == "global" && sub(2) == "add"),
        "bun" => matches!(sub(1), "add" | "i" | "install") && l.iter().skip(2).any(|w| !w.starts_with('-')),
        "pip" | "pip3" | "pipx" => sub(1) == "install",
        "python" | "python3" | "py" => sub(1) == "-m" && sub(2) == "pip" && sub(3) == "install",
        "uv" => (sub(1) == "pip" && sub(2) == "install") || sub(1) == "add" || (sub(1) == "tool" && sub(2) == "install"),
        "cargo" => sub(1) == "install",
        "winget" | "choco" | "scoop" | "gem" => sub(1) == "install",
        "go" => sub(1) == "install",
        "dotnet" => sub(1) == "tool" && sub(2) == "install",
        _ => false,
    }
}

/// A push, and whether it forces.
fn push(words: &[String]) -> Option<bool> {
    if base(&words[0]) != "git" {
        return None;
    }
    let l: Vec<String> = words.iter().map(|w| w.to_ascii_lowercase()).collect();
    // git -C <dir> push …
    let mut i = 1;
    while i < l.len() && l[i].starts_with('-') {
        i += if l[i] == "-c" { 2 } else { 1 }; // -C <dir> and -c <setting> take a value
    }
    if l.get(i).map(String::as_str) != Some("push") {
        return None;
    }
    let force = l.iter().skip(i + 1).any(|w| w == "--force" || w == "-f" || w.starts_with("--force-with-lease") || w.starts_with("--force-if-includes") || (w.starts_with('+') && w.len() > 1));
    Some(force)
}

/// What a command does that deserves a look, if anything: throwing away changes that weren't
/// committed, putting files back to an older version, skipping the checks before a commit,
/// changing a release on GitHub, or changing Microsoft Defender's settings.
fn risky(words: &[String]) -> Option<&'static str> {
    let l: Vec<String> = words.iter().map(|w| w.to_ascii_lowercase()).collect();
    match base(&l[0]).as_str() {
        "git" => {
            // git -C <dir> -c <setting> <what> …
            let mut i = 1;
            while i < l.len() && l[i].starts_with('-') {
                i += if l[i] == "-c" { 2 } else { 1 };
            }
            let rest = l.get(i + 1..).unwrap_or_default();
            let has = |w: &str| rest.iter().any(|x| x == w);
            let starts = |w: &str| rest.iter().any(|x| x.starts_with(w));
            match l.get(i).map(String::as_str).unwrap_or_default() {
                "reset" if has("--hard") => Some(FLAG_DISCARDED),
                "checkout" => {
                    let dashes = rest.iter().position(|w| w == "--");
                    let before: Vec<&String> = rest[..dashes.unwrap_or(rest.len())].iter().filter(|w| !w.starts_with('-')).collect();
                    match dashes {
                        // `git checkout <commit> -- <files>`: those files as they were then.
                        Some(_) if !before.is_empty() => Some(FLAG_ROLLED_BACK),
                        // `git checkout -- <files>`: their changes thrown away.
                        Some(_) => Some(FLAG_DISCARDED),
                        None if before.first().is_some_and(|w| *w == ".") => Some(FLAG_DISCARDED),
                        None => None,
                    }
                }
                "restore" if starts("--source") || has("-s") => Some(FLAG_ROLLED_BACK),
                "restore" if !has("--staged") && !has("-s") && !has("-S") && rest.iter().any(|w| !w.starts_with('-')) => Some(FLAG_DISCARDED),
                "stash" if matches!(rest.first().map(String::as_str), Some("drop" | "clear")) => Some(FLAG_DISCARDED),
                "commit" if has("--no-verify") || has("-n") => Some(FLAG_NO_VERIFY),
                "push" | "merge" | "rebase" if has("--no-verify") => Some(FLAG_NO_VERIFY),
                _ => None,
            }
        }
        "gh" if l.get(1).map(String::as_str) == Some("release") && matches!(l.get(2).map(String::as_str), Some("create" | "edit" | "delete" | "upload" | "delete-asset")) => {
            Some(FLAG_RELEASE)
        }
        "add-mppreference" | "set-mppreference" | "remove-mppreference" => Some(FLAG_DEFENDER),
        _ => None,
    }
}

/// A file of code, where a switch or a skipped test means something (not a note or a doc).
fn is_code(path: &str) -> bool {
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "py" | "go" | "java" | "kt" | "cs" | "rb" | "php" | "swift" | "c" | "cc" | "cpp" | "h" | "hpp" | "vue" | "svelte" | "dart" | "scala"
    )
}

/// A line that sets a feature's switch to off: `pub const FILE_CLEANUP_AVAILABLE: bool = false;`,
/// `export const syncEnabled = false`, `UPLOADS_ALLOWED = False`. Its name, if so.
fn switched_off(line: &str) -> Option<&str> {
    static OFF: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = OFF.get_or_init(|| {
        regex::Regex::new(
            r"^\s*(?:(?:pub(?:\([a-z]+\))?|export|public|private|static|final|readonly)\s+)*(?:(?:const|static|let|var|val)\s+(?:mut\s+)?)?([A-Za-z_][A-Za-z0-9_]*)\s*(?::\s*bool(?:ean)?\s*)?=\s*(?:false|False|FALSE)\s*[;,]?\s*(?://.*|#.*)?$",
        )
        .expect("a fixed pattern")
    });
    let found = re.captures(line)?.get(1)?;
    let name = found.as_str();
    let l = name.to_ascii_lowercase();
    // A constant (not a variable that starts off false), or a CONSTANT_NAME; and the name has to
    // sound like a switch.
    let declared = line[..found.start()].split_whitespace().any(|w| matches!(w, "const" | "static" | "final" | "readonly"));
    let switch = ["available", "enabled", "enable", "allowed", "allow", "supported", "active", "feature"].iter().any(|w| l.contains(w));
    (switch && (declared || name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))).then_some(name)
}

/// A line that turns a test off: `#[ignore]`, `it.skip(`, `xit(`, `@pytest.mark.skip`, `.only(`
/// (which skips all the others). Not when it's only written about, in quotes or after `//`.
fn tests_off(line: &str) -> bool {
    static SKIP: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = SKIP.get_or_init(|| {
        regex::Regex::new(r"^\s*#\[ignore\b|\b(?:it|test|describe|context|suite)\.(?:skip|only)\(|^\s*x(?:it|describe|test)\(|@pytest\.mark\.skip|@unittest\.skip|\bt\.Skip\(")
            .expect("a fixed pattern")
    });
    re.find_iter(line).any(|m| {
        let before = &line[..m.start()];
        before.matches('"').count().is_multiple_of(2) && before.matches('\'').count().is_multiple_of(2) && !before.contains("//")
    })
}

/// A test or build run.
fn is_check(words: &[String]) -> bool {
    let l: Vec<String> = words.iter().map(|w| w.to_ascii_lowercase()).collect();
    let verb = base(&l[0]);
    let sub = |i: usize| l.get(i).map(String::as_str).unwrap_or_default();
    match verb.as_str() {
        "cargo" => matches!(sub(1), "test" | "build" | "nextest"),
        "npm" | "pnpm" | "yarn" | "bun" => {
            let s = if sub(1) == "run" { sub(2) } else { sub(1) };
            matches!(s, "test" | "t" | "build" | "test:unit" | "test:e2e" | "typecheck") || s.starts_with("test")
        }
        "npx" | "pnpx" | "bunx" => matches!(sub(1), "vitest" | "jest" | "mocha" | "playwright" | "tsc"),
        "vitest" | "jest" | "pytest" | "mocha" => true,
        "python" | "python3" | "py" => sub(1) == "-m" && matches!(sub(2), "pytest" | "unittest"),
        "node" => sub(1) == "--test",
        "go" => matches!(sub(1), "test" | "build"),
        "dotnet" => matches!(sub(1), "test" | "build"),
        "gradle" | "gradlew" => l.iter().skip(1).any(|w| w == "test" || w == "build" || w == "check"),
        "mvn" | "mvnw" => l.iter().skip(1).any(|w| matches!(w.as_str(), "test" | "package" | "verify" | "install")),
        _ => false,
    }
}

// ------------------------------------------------------------------ where a path is

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Place {
    Inside,
    Temp,
    Outside,
    /// A variable or a path mom can't place: never called outside.
    Unknown,
}

struct Places {
    home: String,
    temps: Vec<String>,
}

impl Places {
    fn new(paths: &Paths) -> Self {
        let mut temps = vec![norm_path(&paths.temp.to_string_lossy())];
        // A made-up layout (tests) has only its own Temp.
        let fixture = paths.system_drive.parent().is_some();
        for t in [std::env::temp_dir(), paths.local_appdata.join("Temp")].into_iter().filter(|_| !fixture) {
            let n = norm_path(&t.to_string_lossy());
            if !temps.contains(&n) {
                temps.push(n);
            }
        }
        Places { home: paths.home.to_string_lossy().to_string(), temps }
    }

    fn place(&self, target: &str, cwd: &str) -> Place {
        let t = target.trim().trim_matches(['"', '\'', '`']);
        if t.is_empty() {
            return Place::Unknown;
        }
        let lower = t.to_ascii_lowercase();
        let temp_vars = ["$env:temp", "$env:tmp", "%temp%", "%tmp%", "$temp", "$tmp", "${temp}", "${tmp}", "/tmp/", "$tmpdir", "${tmpdir}"];
        if temp_vars.iter().any(|v| lower.starts_with(v)) || lower == "/tmp" {
            return Place::Temp;
        }
        let home_vars = ["~", "$home", "${home}", "$env:userprofile", "%userprofile%", "$env:home"];
        let full = if let Some(v) = home_vars.iter().find(|v| lower == **v || lower.starts_with(&format!("{v}/")) || lower.starts_with(&format!("{v}\\"))) {
            format!("{}{}", self.home, &t[v.len()..])
        } else if t.starts_with('$') || t.starts_with('%') {
            return Place::Unknown;
        } else if let Some(rest) = drive_from_unix(t) {
            rest
        } else if is_absolute(t) {
            t.to_string()
        } else if t.starts_with('/') {
            return Place::Unknown; // a Linux path (WSL, a container)
        } else if cwd.is_empty() {
            return Place::Unknown;
        } else {
            format!("{cwd}\\{t}")
        };
        let full = resolve_dots(&full);
        let under = |root: &str| {
            let root = resolve_dots(root);
            !root.is_empty() && (full == root || full.starts_with(&format!("{}\\", root.trim_end_matches('\\'))))
        };
        if !cwd.is_empty() && under(cwd) {
            Place::Inside
        } else if self.temps.iter().any(|t| under(t)) {
            Place::Temp
        } else if cwd.is_empty() {
            Place::Unknown // where its project is isn't known (yet): never called outside
        } else {
            Place::Outside
        }
    }
}

fn is_absolute(t: &str) -> bool {
    let b = t.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')) || t.starts_with("\\\\")
}

/// `/c/Users/x` (Git Bash) and `/mnt/c/Users/x` (WSL) as `C:\Users\x`.
fn drive_from_unix(t: &str) -> Option<String> {
    let rest = t.strip_prefix("/mnt/").unwrap_or(t.strip_prefix('/')?);
    let mut chars = rest.chars();
    let d = chars.next().filter(char::is_ascii_alphabetic)?;
    let after = chars.as_str();
    (after.is_empty() || after.starts_with('/')).then(|| format!("{}:\\{}", d.to_ascii_uppercase(), after.trim_start_matches('/').replace('/', "\\")))
}

/// A path with `.` and `..` worked out, normalized for comparing.
fn resolve_dots(p: &str) -> String {
    let n = norm_path(p);
    let mut parts: Vec<&str> = Vec::new();
    for part in n.split('\\') {
        match part {
            "." | "" if !parts.is_empty() => {}
            ".." => {
                if parts.len() > 1 {
                    parts.pop();
                }
            }
            _ => parts.push(part),
        }
    }
    parts.join("\\")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture(tag: &str) -> (PathBuf, Paths) {
        let root = std::env::temp_dir().join(format!("mom-homework-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::under(&root);
        (root, paths)
    }

    fn write_lines(path: &Path, lines: &[Value]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn a_claude_session_gets_its_receipt_and_the_claim_beside_the_facts() {
        let (root, paths) = fixture("claude");
        let project = root.join("work").join("lanternfall");
        let cwd = project.to_string_lossy().to_string();
        let elsewhere = root.join("work").join("copperline").join("old").to_string_lossy().replace('\\', "/");
        let t = |s: u32| format!("2026-10-14T10:{s:02}:00.000Z");
        let use_ = |id: &str, name: &str, input: Value, s: u32| {
            json!({"type": "assistant", "timestamp": t(s), "cwd": cwd, "sessionId": "s1", "message": {"role": "assistant", "content": [{"type": "tool_use", "id": id, "name": name, "input": input}]}})
        };
        let result = |id: &str, error: bool, content: &str, s: u32| {
            json!({"type": "user", "timestamp": t(s), "cwd": cwd, "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": id, "is_error": error, "content": content}]}})
        };
        let log = paths.claude_dir().join("projects").join("C--work-lanternfall").join("s1.jsonl");
        write_lines(&log, &[
            json!({"type": "user", "timestamp": t(0), "cwd": cwd, "message": {"role": "user", "content": "Fix the checkout page"}}),
            use_("e1", "Edit", json!({"file_path": format!("{cwd}\\src\\checkout.ts")}), 1),
            use_("e2", "Write", json!({"file_path": format!("{cwd}\\src\\cart.ts")}), 2),
            use_("e3", "Edit", json!({"file_path": format!("{cwd}\\src\\checkout.ts")}), 3),
            use_("b1", "Bash", json!({"command": format!("rm -rf dist && rm -rf {elsewhere}")}), 4),
            use_("b2", "Bash", json!({"command": "npm install left-pad@1.3.0"}), 5),
            use_("b3", "Bash", json!({"command": "git push --force origin main"}), 6),
            use_("b4", "Bash", json!({"command": "cargo test 2>&1 | tail -20"}), 7),
            result("b4", false, "test checkout ... FAILED\ntest result: FAILED. 3 passed; 1 failed", 8),
            use_("b5", "PowerShell", json!({"command": "Remove-Item -Recurse -Force $env:TEMP\\lanternfall-scratch"}), 9),
            json!({"type": "assistant", "timestamp": t(10), "cwd": cwd, "message": {"role": "assistant", "content": [{"type": "text", "text": "Done! All tests pass now and the checkout is fixed."}]}}),
        ]);
        let list = recent(&paths, true);
        assert_eq!(list.len(), 1);
        let r = &list[0];
        assert_eq!((r.agent.as_str(), r.id.as_str(), r.cwd.as_str(), r.title.as_str()), (CLAUDE, "s1", cwd.as_str(), "Fix the checkout page"));
        assert_eq!(r.commands, 5);
        assert_eq!((r.changed_count, r.changed.last().unwrap().ends_with("checkout.ts")), (2, true));
        assert_eq!(r.deleted.len(), 3, "dist, the folder elsewhere, and the scratch in Temp");
        assert_eq!(r.deleted.iter().map(|d| d.outside).collect::<Vec<_>>(), [true, false, false], "the one outside first: {:?}", r.deleted);
        assert!(r.deleted[0].path.ends_with("copperline/old"));
        assert_eq!(r.installs, ["npm install left-pad@1.3.0"]);
        assert!(r.pushes[0].force);
        let check = r.last_check.as_ref().unwrap();
        assert!(!check.passed && check.command.starts_with("cargo test"));
        assert_eq!(r.flags, [FLAG_OUTSIDE, FLAG_FORCE, FLAG_CLAIM]);
        assert_eq!((r.started, r.ended - r.started), (chrono::DateTime::parse_from_rfc3339(&t(0)).unwrap().timestamp() as u64, 600));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_codex_thread_gets_its_receipt_from_items_and_patches() {
        let (root, paths) = fixture("codex");
        let cwd = root.join("work").join("storefront").to_string_lossy().to_string();
        let id = "0199c2f4-1a2b-7c3d-8e4f-5a6b7c8d9e0f";
        let day = chrono::Local::now().format("%Y/%m/%d").to_string();
        let log = paths.codex_dir().join("sessions").join(day.replace('/', "\\")).join(format!("rollout-2026-10-14T10-00-00-{id}.jsonl"));
        let item = |item: Value, s: u32| json!({"timestamp": format!("2026-10-14T10:{s:02}:00Z"), "type": "event_msg", "payload": {"type": "item_completed", "item": item}});
        write_lines(&log, &[
            json!({"timestamp": "2026-10-14T10:00:00Z", "type": "session_meta", "payload": {"id": id, "cwd": format!("\\\\?\\{cwd}")}}),
            item(json!({"type": "UserMessage", "content": [{"type": "Text", "text": "Make the Nimbus banner smaller\nand faster"}]}), 1),
            item(json!({"type": "CommandExecution", "command": ["powershell.exe", "-Command", "Remove-Item -Recurse -Force .\\build; npm i -D vitest"], "cwd": cwd, "exit_code": 0, "status": "completed"}), 2),
            item(json!({"type": "FileChange", "changes": {
                format!("{cwd}\\src\\banner.css"): {"type": "update", "unified_diff": ""},
                format!("{cwd}\\src\\old-banner.css"): {"type": "delete", "content": ""},
                root.join("work").join("notes.txt").to_string_lossy(): {"type": "delete", "content": ""},
            }, "status": "completed"}), 3),
            item(json!({"type": "CommandExecution", "command": ["bash", "-lc", "npm test"], "cwd": cwd, "exit_code": 1, "status": "failed", "aggregated_output": "2 failing"}), 4),
            item(json!({"type": "AgentMessage", "phase": "final_answer", "content": [{"type": "Text", "text": "The banner is smaller. Tests aren't passing yet: two snapshot tests fail."}]}), 5),
        ]);
        let list = recent(&paths, true);
        assert_eq!(list.len(), 1);
        let r = &list[0];
        assert_eq!((r.agent.as_str(), r.id.as_str(), r.cwd.as_str()), (CODEX, id, cwd.as_str()));
        assert_eq!(r.title, "Make the Nimbus banner smaller and faster", "no thread database here: the first ask");
        assert_eq!(r.link.as_deref(), Some("codex://threads/0199c2f4-1a2b-7c3d-8e4f-5a6b7c8d9e0f"));
        assert_eq!(r.commands, 2);
        assert_eq!(r.changed, [format!("{cwd}\\src\\banner.css")]);
        assert_eq!(r.deleted_count, 3);
        assert_eq!(
            r.deleted.iter().map(|d| (d.path.rsplit(['\\', '/']).next().unwrap().to_string(), d.outside)).collect::<Vec<_>>(),
            [("notes.txt".into(), true), ("build".into(), false), ("old-banner.css".into(), false)],
            "a patch says no folder of its own: it's judged by the project's"
        );
        assert_eq!(r.installs, ["npm i -D vitest"]);
        assert!(!r.last_check.as_ref().unwrap().passed);
        assert_eq!(r.flags, [FLAG_OUTSIDE], "it said the tests aren't passing: no claim to set beside them");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn claims_are_told_from_honest_reports() {
        for yes in ["All tests pass.", "Tests are green now", "Everything works.", "The bug is fixed and it works now.", "all green!", "I fixed the parser."] {
            assert!(claims_pass(yes), "{yes}");
        }
        for no in ["Tests don't pass yet.", "Not all tests pass.", "The tests still fail.", "Once the tests pass, I'll push.", "Do the tests pass?", "Nothing is fixed yet."] {
            assert!(!claims_pass(no), "{no}");
        }
        assert!(looks_failed("running 3 tests\ntest result: FAILED. 2 passed; 1 failed") && looks_failed("# fail 2") && !looks_failed("test result: ok. 3 passed; 0 failed"));
        assert_eq!(exit_code("Exit code 101\nerror"), Some(101));
    }

    #[test]
    fn commands_are_read_as_text_into_what_they_do() {
        let words = |s: &str| unwrap(tokens(s));
        assert_eq!(deletions(&words(r#"rm -rf "my dir" other"#)), [("my dir".to_string(), false), ("other".to_string(), false)]);
        assert_eq!(deletions(&words("Remove-Item -Path C:\\x -Recurse -Force -Include *.log")), [("C:\\x".to_string(), false)]);
        assert_eq!(deletions(&words("rd /s /q build")), [("build".to_string(), false)]);
        assert_eq!(deletions(&words("git clean -fdx")), [("untracked files (git clean)".to_string(), true)]);
        assert!(deletions(&words("git clean -n")).is_empty() && deletions(&words("ls -la")).is_empty());
        assert_eq!(push(&words("git -C repo push origin +main")), Some(true));
        assert_eq!(push(&words("git push -u origin feature")), Some(false));
        assert_eq!(push(&words("git pull")), None);
        assert!(is_install(&words("pip install requests")) && is_install(&words("cargo install ripgrep")) && is_install(&words("npm i -g pnpm")));
        assert!(!is_install(&words("npm install")) && !is_install(&words("npm ci")), "installing what the lockfile says isn't new");
        assert!(is_check(&words("cargo test -p mom-core")) && is_check(&words("npm run build")) && is_check(&words("python -m pytest -q")) && is_check(&words("node --test tests/*.cjs")));
        assert!(!is_check(&words("cargo fmt")) && !is_check(&words("npm run dev")));
        assert_eq!(segments("a && b || c; d | e\nf 2>&1").iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), ["a", "b", "c", "d", "e", "f 2>&1"]);
        assert_eq!(segments("echo 'a && b'").len(), 1);
        assert_eq!(shell_script(&words(r#"powershell.exe -NoProfile -Command "Remove-Item x""#)).as_deref(), Some("Remove-Item x"));
    }

    #[test]
    fn what_deserves_a_look_is_read_from_commands_and_edits() {
        let words = |s: &str| unwrap(tokens(s));
        assert_eq!(risky(&words("git reset --hard origin/main")), Some(FLAG_DISCARDED));
        assert_eq!(risky(&words("git checkout -- src/app.ts")), Some(FLAG_DISCARDED));
        assert_eq!(risky(&words("git checkout .")), Some(FLAG_DISCARDED));
        assert_eq!(risky(&words("git restore src/app.ts")), Some(FLAG_DISCARDED));
        assert_eq!(risky(&words("git stash drop")), Some(FLAG_DISCARDED));
        assert_eq!(risky(&words("git restore --source=77c4a78 -- Cargo.toml")), Some(FLAG_ROLLED_BACK));
        assert_eq!(risky(&words("git checkout 77c4a78 -- src/lib.rs")), Some(FLAG_ROLLED_BACK));
        assert_eq!(risky(&words("git -C repo commit -n -m wip")), Some(FLAG_NO_VERIFY));
        assert_eq!(risky(&words("gh release edit v0.3.1 --title held")), Some(FLAG_RELEASE));
        assert_eq!(risky(&words("Add-MpPreference -ExclusionPath C:\\x")), Some(FLAG_DEFENDER));
        for fine in ["git checkout main", "git checkout -b feature", "git restore --staged a.ts", "git reset HEAD~1", "git stash", "gh release view v1", "rg -n Add-MpPreference crates", "git commit -m 'no-verify docs'"] {
            assert_eq!(risky(&words(fine)), None, "{fine}");
        }
        assert_eq!(switched_off("pub const FILE_CLEANUP_AVAILABLE: bool = false;"), Some("FILE_CLEANUP_AVAILABLE"));
        assert_eq!(switched_off("export const syncEnabled = false"), Some("syncEnabled"));
        assert_eq!(switched_off("UPLOADS_ALLOWED = False  # for now"), Some("UPLOADS_ALLOWED"));
        assert_eq!(switched_off("static PREVIEW_FEATURE: bool = false;"), Some("PREVIEW_FEATURE"));
        for fine in ["const FILE_CLEANUP_AVAILABLE: bool = true;", "enabled: false,", "let done = false;", "if !FILE_CLEANUP_AVAILABLE {", "isEnabled = false", "    let mut enabled = false;"] {
            assert_eq!(switched_off(fine), None, "{fine}");
        }
        assert!(tests_off("    #[ignore]") && tests_off("it.skip('works', () => {") && tests_off("describe.only('x', () => {") && tests_off("xit('x')") && tests_off("@pytest.mark.skip(reason='flaky')"));
        assert!(!tests_off("// it.skips the first") && !tests_off("let ignore = true;"));
        assert!(!tests_off(r#"assert!(tests_off("it.skip('works')"));"#) && !tests_off("// it.only('x') while debugging"), "only written about");
        assert!(is_code("src/lib.rs") && !is_code("docs/notes.md"));
    }

    #[test]
    fn a_session_that_switched_a_feature_off_and_rolled_files_back_says_so() {
        let (root, paths) = fixture("switched");
        let cwd = root.join("work").join("atlas").to_string_lossy().to_string();
        let id = "0199c2f4-1a2b-7c3d-8e4f-5a6b7c8d9e01";
        let day = chrono::Local::now().format("%Y/%m/%d").to_string();
        let log = paths.codex_dir().join("sessions").join(day.replace('/', "\\")).join(format!("rollout-2026-10-14T10-00-00-{id}.jsonl"));
        let item = |item: Value, s: u32| json!({"timestamp": format!("2026-10-14T10:{s:02}:00Z"), "type": "event_msg", "payload": {"type": "item_completed", "item": item}});
        write_lines(&log, &[
            json!({"timestamp": "2026-10-14T10:00:00Z", "type": "session_meta", "payload": {"id": id, "cwd": cwd}}),
            item(json!({"type": "FileChange", "changes": {
                format!("{cwd}\\src\\lib.rs"): {"type": "update", "unified_diff": "@@ -1,2 +1,2 @@\n-pub const UPLOADS_AVAILABLE: bool = true;\n+pub const UPLOADS_AVAILABLE: bool = false;\n"},
                format!("{cwd}\\docs\\notes.md"): {"type": "add", "content": "const NOTES_ENABLED = false\n"},
            }, "status": "completed"}), 1),
            item(json!({"type": "CommandExecution", "command": "pwsh.exe -Command \"git restore --source=77c4a78 -- Cargo.toml; gh release edit v1.2.0 --draft\"", "cwd": cwd, "exit_code": 0, "status": "completed"}), 2),
            // A Claude-style session elsewhere would say the same of an edit that skips a test.
        ]);
        let r = &recent(&paths, true)[0];
        assert_eq!(r.flags, [flag_switched_off("UPLOADS_AVAILABLE"), FLAG_ROLLED_BACK.to_string(), FLAG_RELEASE.to_string()], "a note isn't code");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_session_that_did_nothing_is_not_listed() {
        let (root, paths) = fixture("quiet");
        let log = paths.claude_dir().join("projects").join("C--work-nimbus").join("s2.jsonl");
        write_lines(&log, &[
            json!({"type": "user", "timestamp": "2026-10-14T10:00:00.000Z", "cwd": "C:\\work\\nimbus", "message": {"role": "user", "content": "Review this diff"}}),
            json!({"type": "assistant", "timestamp": "2026-10-14T10:01:00.000Z", "cwd": "C:\\work\\nimbus", "message": {"role": "assistant", "content": [{"type": "text", "text": "It looks fine."}]}}),
        ]);
        assert!(recent(&paths, true).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn places_tell_inside_from_temp_from_outside() {
        let (root, paths) = fixture("places");
        let p = Places::new(&paths);
        let cwd = root.join("work").join("atlas").to_string_lossy().to_string();
        assert_eq!(p.place("dist", &cwd), Place::Inside);
        assert_eq!(p.place("./src/../dist", &cwd), Place::Inside);
        assert_eq!(p.place("../other", &cwd), Place::Outside);
        assert_eq!(p.place(&paths.temp.join("x").to_string_lossy(), &cwd), Place::Temp);
        assert_eq!(p.place("$env:TEMP\\x", &cwd), Place::Temp);
        assert_eq!(p.place("~/Documents", &cwd), Place::Outside);
        assert_eq!(p.place("$somewhere", &cwd), Place::Unknown);
        assert_eq!(p.place("/var/tmp/x", &cwd), Place::Unknown);
        assert_eq!(p.place(&cwd, ""), Place::Unknown, "no project folder known: can't be outside it");
        assert_eq!(drive_from_unix("/c/Users/x").as_deref(), Some("C:\\Users\\x"));
        assert_eq!(resolve_dots(r"C:\a\b\..\c\.\d"), r"c:\a\c\d");
    }
}
