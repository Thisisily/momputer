//! Waiting for you: Claude Code and Codex tell mom when they stop or need you, and she says so.
//!
//! It's opt-in, like every integration. Switched on, it adds one hook to Claude Code (its
//! Notification and Stop events run `mom notify --agent claude`) and, only if Codex has no
//! `notify` program of its own, makes Codex run `mom notify --agent codex` when a turn ends.
//! Switched off, it takes out exactly what it added.
//!
//! Each event leaves one small record in ~/.momputer/state/waiting/: which agent, which session,
//! its folder, what kind of event, when. Nothing else from the event is kept. When a record is
//! half a minute old and the session hasn't moved since, mom says "Claude is waiting for you";
//! Running shows how long until the session moves again.

use crate::hooks::Integration;
use crate::paths::Paths;
use crate::util::now_secs;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// The integration's id (House rules, Settings).
pub const ID: &str = "agent-waiting";
/// The most an event may say; anything longer isn't read.
const MAX_INPUT: usize = 256 * 1024;
const MAX_RECORDS: usize = 40;
const MAX_RECORD_BYTES: u64 = 8 * 1024;
/// Records older than this are gone.
const KEEP_SECS: u64 = 2 * 86_400;
/// How long a session has been quiet before mom says it's waiting.
pub const NOTICE_AFTER: u64 = 30;
/// After this long it isn't shown as waiting any more: nobody's coming back to it soon.
const STALE_AFTER: u64 = 12 * 3600;
/// A log written this soon after the event is the agent finishing up, not moving on.
const SETTLE: u64 = 10;
const CLAUDE_EVENTS: &[&str] = &["Notification", "Stop"];
const CODEX_MARK: &str = "# momputer: tells mom when Codex finishes a turn (her \"Tell me when an agent is waiting\" switch).";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Waiting {
    /// "claude" or "codex".
    pub agent: String,
    /// Claude's session id, or Codex's thread id.
    pub session: String,
    pub cwd: String,
    pub project: Option<String>,
    /// "needs-you" (it asked for permission or input) or "done" (it finished its turn).
    pub kind: String,
    /// Unix seconds.
    pub at: u64,
    /// Claude's log for this session: when it changes, the session has moved on.
    pub transcript: Option<String>,
    /// Mom has said so already.
    pub told: bool,
}

impl Waiting {
    /// "Claude" or "Codex".
    pub fn label(&self) -> &'static str {
        if self.agent == "codex" { "Codex" } else { "Claude" }
    }
}

pub fn dir(paths: &Paths) -> PathBuf {
    paths.state_dir().join("waiting")
}

/// Ids are kept to letters, digits, dashes and underscores: they name a file.
fn safe_id(s: &str) -> Option<String> {
    let id: String = s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(80).collect();
    (!id.is_empty()).then_some(id)
}

fn safe_cwd(s: &str) -> String {
    let s = s.trim_start_matches(r"\\?\");
    if s.len() > 1024 || s.chars().any(char::is_control) { String::new() } else { s.to_string() }
}

/// What an agent's event says, as a record, or None for an event that isn't about waiting.
/// `payload` is Claude's hook input or Codex's notify argument.
pub fn parse(paths: &Paths, agent: &str, payload: &str) -> Option<Waiting> {
    if payload.len() > MAX_INPUT {
        return None;
    }
    let v: Value = serde_json::from_str(payload).ok()?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default();
    let (session, cwd, kind, transcript) = match agent {
        "claude" => {
            let kind = match s("hook_event_name") {
                "Notification" => "needs-you",
                "Stop" => "done",
                _ => return None,
            };
            // Only Claude's own log, in its own folder.
            let projects = crate::util::norm_path(&paths.claude_dir().join("projects").to_string_lossy());
            let t = s("transcript_path");
            let transcript = (t.len() <= 1024 && t.to_ascii_lowercase().ends_with(".jsonl") && crate::util::norm_path(t).starts_with(&format!("{projects}\\")))
                .then(|| t.to_string());
            (s("session_id").to_string(), s("cwd").to_string(), kind, transcript)
        }
        "codex" => {
            if s("type") != "agent-turn-complete" {
                return None;
            }
            let thread = if s("thread-id").is_empty() { s("thread_id") } else { s("thread-id") };
            (thread.to_string(), s("cwd").to_string(), "done", None)
        }
        _ => return None,
    };
    let cwd = safe_cwd(&cwd);
    Some(Waiting {
        agent: agent.to_string(),
        session: safe_id(&session)?,
        project: crate::scan::project_name(&cwd).or_else(|| Path::new(&cwd).file_name().map(|n| n.to_string_lossy().to_string())),
        cwd,
        kind: kind.to_string(),
        at: now_secs(),
        transcript,
        told: false,
    })
}

/// `mom notify`: keeps the event's record. Never fails loudly: an agent must never be held up.
pub fn record(paths: &Paths, agent: &str, payload: &str) -> Option<Waiting> {
    let w = parse(paths, agent, payload)?;
    let file = dir(paths).join(format!("{}-{}.json", w.agent, w.session));
    crate::util::write_atomic(&file, &serde_json::to_string(&w).ok()?).ok()?;
    prune(paths);
    Some(w)
}

fn read_all(paths: &Paths) -> Vec<(PathBuf, Waiting)> {
    let Ok(entries) = std::fs::read_dir(dir(paths)) else { return Vec::new() };
    let mut out = Vec::new();
    for e in entries.flatten().take(MAX_RECORDS * 4) {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") || e.metadata().map(|m| m.len() > MAX_RECORD_BYTES).unwrap_or(true) {
            continue;
        }
        if let Some(w) = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Waiting>(&t).ok()) {
            out.push((path, w));
        }
    }
    out.sort_by(|a, b| b.1.at.cmp(&a.1.at));
    out
}

/// Old records go, and only the newest few are kept.
fn prune(paths: &Paths) {
    let now = now_secs();
    for (i, (path, w)) in read_all(paths).into_iter().enumerate() {
        if i >= MAX_RECORDS || now.saturating_sub(w.at) > KEEP_SECS {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// When the session's own log last changed, if mom can tell.
fn log_changed(paths: &Paths, w: &Waiting) -> Option<u64> {
    let log = match &w.transcript {
        Some(t) => PathBuf::from(t),
        None if w.agent == "codex" => codex_log(paths, &w.session)?,
        None => return None,
    };
    std::fs::metadata(log).ok().map(|m| crate::util::mtime_secs(&m))
}

/// A Codex thread's log, from the last week's day folders.
fn codex_log(paths: &Paths, thread: &str) -> Option<PathBuf> {
    use chrono::Datelike;
    let root = paths.codex_dir().join("sessions");
    let end = format!("-{thread}.jsonl");
    let mut day = chrono::Local::now().date_naive();
    for _ in 0..8 {
        let dir = root.join(format!("{:04}", day.year())).join(format!("{:02}", day.month())).join(format!("{:02}", day.day()));
        if let Ok(files) = std::fs::read_dir(&dir) {
            if let Some(f) = files.flatten().take(5000).find(|f| f.file_name().to_string_lossy().ends_with(&end)) {
                return Some(f.path());
            }
        }
        day = day.pred_opt()?;
    }
    None
}

fn moved(paths: &Paths, w: &Waiting) -> bool {
    log_changed(paths, w).is_some_and(|changed| changed > w.at + SETTLE)
}

/// Sessions waiting for you now, newest first.
pub fn current(paths: &Paths) -> Vec<Waiting> {
    let now = now_secs();
    read_all(paths).into_iter().map(|(_, w)| w).filter(|w| now.saturating_sub(w.at) < STALE_AFTER && !moved(paths, w)).collect()
}

/// The waits mom should mention now: quiet for half a minute and not mentioned yet. Each is
/// marked as mentioned, and records of sessions that moved on are let go.
pub fn due(paths: &Paths) -> Vec<Waiting> {
    let now = now_secs();
    let mut out = Vec::new();
    for (path, mut w) in read_all(paths) {
        if now.saturating_sub(w.at) >= STALE_AFTER || moved(paths, &w) {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        if w.told || now.saturating_sub(w.at) < NOTICE_AFTER || out.len() >= 3 {
            continue;
        }
        w.told = true;
        if let Ok(text) = serde_json::to_string(&w) {
            let _ = crate::util::write_atomic(&path, &text);
        }
        out.push(w);
    }
    out
}

/// What mom says about one wait.
pub fn sentence(w: &Waiting) -> (String, String) {
    let title = format!("{} is waiting for you", w.label());
    (title, w.project.clone().unwrap_or_else(|| "Its session".into()))
}

// ------------------------------------------------------------------ the switch

fn claude_file(paths: &Paths) -> PathBuf {
    paths.claude_dir().join("settings.json")
}

fn codex_file(paths: &Paths) -> PathBuf {
    paths.codex_dir().join("config.toml")
}

fn command(exe: &Path, agent: &str) -> String {
    format!("\"{}\" notify --agent {agent}", exe.to_string_lossy().replace('\\', "/"))
}

fn is_ours(cmd: &str, agent: &str) -> bool {
    let c = cmd.to_ascii_lowercase();
    (c.contains("mom.exe\"") || c.contains("mom\"")) && c.contains(&format!("notify --agent {agent}"))
}

fn strip_claude(hooks: &mut serde_json::Map<String, Value>) {
    for (_, groups) in hooks.iter_mut() {
        let Some(arr) = groups.as_array_mut() else { continue };
        for g in arr.iter_mut() {
            if let Some(inner) = g.get_mut("hooks").and_then(|h| h.as_array_mut()) {
                inner.retain(|h| !h.get("command").and_then(Value::as_str).is_some_and(|c| is_ours(c, "claude")));
            }
        }
        arr.retain(|g| g.get("hooks").and_then(Value::as_array).map(|h| !h.is_empty()).unwrap_or(true));
    }
    hooks.retain(|_, v| v.as_array().map(|a| !a.is_empty()).unwrap_or(true));
}

fn claude_installed(paths: &Paths) -> bool {
    let Ok(v) = crate::hooks::read_json(&claude_file(paths)) else { return false };
    v.get("hooks").and_then(Value::as_object).is_some_and(|h| {
        h.values().flat_map(|g| g.as_array().cloned().unwrap_or_default()).any(|g| {
            g.get("hooks").and_then(Value::as_array).is_some_and(|inner| inner.iter().any(|h| h.get("command").and_then(Value::as_str).is_some_and(|c| is_ours(c, "claude"))))
        })
    })
}

fn install_claude(paths: &Paths, exe: &Path) -> anyhow::Result<()> {
    let file = claude_file(paths);
    let mut v = crate::hooks::read_json(&file)?;
    let Some(root) = v.as_object_mut() else { anyhow::bail!("{} isn't a JSON object; not touching it", file.display()) };
    let hooks = root.entry("hooks").or_insert_with(|| json!({}));
    let Some(map) = hooks.as_object_mut() else { anyhow::bail!("\"hooks\" in {} isn't an object; not touching it", file.display()) };
    crate::hooks::backup(paths, &file, "claude-waiting");
    strip_claude(map);
    for event in CLAUDE_EVENTS {
        let handler = json!({ "type": "command", "command": command(exe, "claude"), "timeout": 5 });
        if let Some(arr) = map.entry(event.to_string()).or_insert_with(|| json!([])).as_array_mut() {
            arr.push(json!({ "hooks": [handler] }));
        }
    }
    crate::util::write_atomic(&file, &format!("{}\n", serde_json::to_string_pretty(&v)?))?;
    Ok(())
}

fn uninstall_claude(paths: &Paths) -> anyhow::Result<()> {
    let file = claude_file(paths);
    if !claude_installed(paths) {
        return Ok(());
    }
    let mut v = crate::hooks::read_json(&file)?;
    crate::hooks::backup(paths, &file, "claude-waiting");
    if let Some(map) = v.get_mut("hooks").and_then(Value::as_object_mut) {
        strip_claude(map);
    }
    if v.get("hooks").and_then(Value::as_object).is_some_and(|h| h.is_empty()) {
        if let Some(root) = v.as_object_mut() {
            root.remove("hooks");
        }
    }
    crate::util::write_atomic(&file, &format!("{}\n", serde_json::to_string_pretty(&v)?))?;
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum CodexNotify {
    /// No notify program: mom may set hers.
    Free,
    Ours,
    /// Codex already runs someone's notify program: left alone.
    Theirs,
    /// The settings file couldn't be read: left alone.
    Unreadable,
}

fn codex_line(exe: &Path) -> String {
    let exe = toml::Value::String(exe.to_string_lossy().replace('\\', "/"));
    format!("notify = [{exe}, \"notify\", \"--agent\", \"codex\"]")
}

fn is_our_line(line: &str) -> bool {
    let l = line.trim_start();
    l.starts_with("notify") && l.contains("\"notify\", \"--agent\", \"codex\"") && (l.contains("mom.exe\"") || l.contains("mom\""))
}

fn codex_text(paths: &Paths) -> Option<String> {
    let file = codex_file(paths);
    match std::fs::metadata(&file) {
        Ok(m) if m.len() > 1 << 20 => None,
        Ok(_) => std::fs::read_to_string(&file).ok(),
        Err(_) => Some(String::new()),
    }
}

fn codex_state(paths: &Paths) -> CodexNotify {
    let Some(text) = codex_text(paths) else { return CodexNotify::Unreadable };
    let Ok(table) = text.trim_start_matches('\u{feff}').parse::<toml::Table>() else { return CodexNotify::Unreadable };
    match table.get("notify") {
        None => CodexNotify::Free,
        Some(_) if text.lines().any(is_our_line) => CodexNotify::Ours,
        Some(_) => CodexNotify::Theirs,
    }
}

/// Puts mom's notify line at the top of Codex's settings, where a top-level key belongs, and
/// checks the result still reads before writing it.
fn install_codex(paths: &Paths, exe: &Path) -> anyhow::Result<CodexNotify> {
    let state = codex_state(paths);
    if matches!(state, CodexNotify::Theirs | CodexNotify::Unreadable) {
        return Ok(state);
    }
    let file = codex_file(paths);
    let text = codex_text(paths).unwrap_or_default();
    let (bom, rest) = if let Some(r) = text.strip_prefix('\u{feff}') { ("\u{feff}", r) } else { ("", text.as_str()) };
    let kept: Vec<&str> = rest.lines().filter(|l| !is_our_line(l) && l.trim() != CODEX_MARK).collect();
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let new = format!("{bom}{CODEX_MARK}{nl}{}{nl}{}{}", codex_line(exe), kept.join(nl), if kept.is_empty() { "" } else { nl });
    let table = new.trim_start_matches('\u{feff}').parse::<toml::Table>().map_err(|e| anyhow::anyhow!("Codex's settings wouldn't read with the change ({e}); left them alone"))?;
    anyhow::ensure!(table.get("notify").is_some(), "Codex's settings didn't take the change; left them alone");
    crate::hooks::backup(paths, &file, "codex-waiting");
    crate::util::write_atomic(&file, &new)?;
    Ok(CodexNotify::Ours)
}

fn uninstall_codex(paths: &Paths) -> anyhow::Result<()> {
    if codex_state(paths) != CodexNotify::Ours {
        return Ok(());
    }
    let file = codex_file(paths);
    let text = codex_text(paths).unwrap_or_default();
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let kept: Vec<&str> = text.lines().filter(|l| !is_our_line(l) && l.trim() != CODEX_MARK).collect();
    let new = if kept.is_empty() { String::new() } else { format!("{}{nl}", kept.join(nl)) };
    new.trim_start_matches('\u{feff}').parse::<toml::Table>().map_err(|e| anyhow::anyhow!("Codex's settings wouldn't read without mom's line ({e}); left them alone"))?;
    crate::hooks::backup(paths, &file, "codex-waiting");
    crate::util::write_atomic(&file, &new)?;
    Ok(())
}

pub fn integration(paths: &Paths) -> Integration {
    Integration {
        id: ID.into(),
        label: "Tell me when an agent is waiting".into(),
        detail: "A note when Claude or Codex stops or needs you. Mom adds a small hook to them.".into(),
        available: paths.claude_dir().is_dir() || paths.codex_dir().is_dir(),
        installed: claude_installed(paths) || codex_state(paths) == CodexNotify::Ours,
        path: claude_file(paths).to_string_lossy().to_string(),
    }
}

pub fn install(paths: &Paths, mom_exe: &Path) -> anyhow::Result<String> {
    let exe = crate::hooks::install_cli(paths, mom_exe)?;
    let mut agents = Vec::new();
    let mut note = String::new();
    if paths.claude_dir().is_dir() {
        install_claude(paths, &exe)?;
        agents.push("Claude");
    }
    if paths.codex_dir().is_dir() {
        match install_codex(paths, &exe)? {
            CodexNotify::Ours => agents.push("Codex"),
            CodexNotify::Theirs => note = " Codex already runs its own notify program, so mom left it alone.".into(),
            _ => note = " Codex's settings couldn't be read, so mom left them alone.".into(),
        }
    }
    if agents.is_empty() {
        anyhow::bail!("No Claude Code or Codex here to listen to.{note}");
    }
    Ok(format!("Mom will tell you when {} is waiting for you. New sessions pick it up.{note}", agents.join(" or ")))
}

pub fn uninstall(paths: &Paths) -> anyhow::Result<String> {
    uninstall_claude(paths)?;
    uninstall_codex(paths)?;
    Ok("Mom won't mention waiting agents any more. Her hooks are out.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_home(tag: &str) -> (PathBuf, Paths) {
        let root = std::env::temp_dir().join(format!("mom-waiting-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::under(&root);
        std::fs::create_dir_all(paths.claude_dir()).unwrap();
        std::fs::create_dir_all(paths.codex_dir()).unwrap();
        (root, paths)
    }

    #[test]
    fn the_switch_adds_hooks_beside_theirs_and_takes_out_only_its_own() {
        let (root, paths) = fake_home("switch");
        let theirs = r#"{
  "model": "opus",
  "hooks": { "Stop": [ { "hooks": [ { "type": "command", "command": "say done" } ] } ] }
}"#;
        std::fs::write(claude_file(&paths), theirs).unwrap();
        std::fs::write(codex_file(&paths), "# my settings\nmodel = \"gpt\"\n\n[projects.'C:\\work']\ntrust_level = \"trusted\"\n").unwrap();
        let exe = root.join("mom.exe");
        std::fs::write(&exe, b"fake").unwrap();
        assert!(!integration(&paths).installed);
        let msg = install(&paths, &exe).unwrap();
        assert!(msg.contains("Claude or Codex"), "{msg}");
        install(&paths, &exe).unwrap(); // twice: still one of each
        let v: Value = serde_json::from_str(&std::fs::read_to_string(claude_file(&paths)).unwrap()).unwrap();
        assert_eq!(v["model"], "opus");
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2, "theirs and ours");
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], "say done");
        assert!(v["hooks"]["Notification"][0]["hooks"][0]["command"].as_str().unwrap().ends_with("mom.exe\" notify --agent claude"));
        let toml_text = std::fs::read_to_string(codex_file(&paths)).unwrap();
        assert_eq!(toml_text.lines().filter(|l| is_our_line(l)).count(), 1);
        let table: toml::Table = toml_text.parse().unwrap();
        assert_eq!(table["notify"].as_array().unwrap().len(), 4);
        assert_eq!(table["model"].as_str(), Some("gpt"), "their settings are untouched");
        assert!(toml_text.contains("# my settings"));
        assert!(integration(&paths).installed);

        uninstall(&paths).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(claude_file(&paths)).unwrap()).unwrap();
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert!(v["hooks"].get("Notification").is_none());
        assert_eq!(std::fs::read_to_string(codex_file(&paths)).unwrap(), "# my settings\nmodel = \"gpt\"\n\n[projects.'C:\\work']\ntrust_level = \"trusted\"\n");
        assert!(!integration(&paths).installed);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn codex_keeps_its_own_notify_program() {
        let (root, paths) = fake_home("theirs");
        let mine = "notify = [\"python\", \"C:/tools/ping.py\"]\n";
        std::fs::write(codex_file(&paths), mine).unwrap();
        let exe = root.join("mom.exe");
        std::fs::write(&exe, b"fake").unwrap();
        let msg = install(&paths, &exe).unwrap();
        assert!(msg.contains("left it alone"), "{msg}");
        assert_eq!(std::fs::read_to_string(codex_file(&paths)).unwrap(), mine);
        uninstall(&paths).unwrap();
        assert_eq!(std::fs::read_to_string(codex_file(&paths)).unwrap(), mine);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn events_become_small_records_and_waits_are_told_once() {
        let (root, paths) = fake_home("records");
        let log = paths.claude_dir().join("projects").join("C--work-atlas").join("s-9.jsonl");
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        std::fs::write(&log, "{}\n").unwrap();
        // When Claude last wrote its log, in seconds from now.
        let set_time = |offset: i64| {
            let (now, d) = (std::time::SystemTime::now(), std::time::Duration::from_secs(offset.unsigned_abs()));
            let t = if offset < 0 { now - d } else { now + d };
            std::fs::File::options().write(true).open(&log).unwrap().set_modified(t).unwrap();
        };
        set_time(-120);
        let ev = json!({"session_id": "s-9", "cwd": root.join("work").join("atlas").to_string_lossy(), "hook_event_name": "Notification",
            "transcript_path": log.to_string_lossy(), "message": "Claude needs your permission to use Bash"});
        let w = record(&paths, "claude", &ev.to_string()).unwrap();
        assert_eq!((w.kind.as_str(), w.session.as_str()), ("needs-you", "s-9"));
        assert!(w.project.is_some() && w.cwd.ends_with("atlas"));
        let saved = std::fs::read_to_string(dir(&paths).join("claude-s-9.json")).unwrap();
        assert!(!saved.contains("permission"), "only the safe fields are kept");
        // A log outside Claude's folder isn't kept; an odd event isn't a record.
        let elsewhere = json!({"session_id": "s-8", "cwd": "C:\\x", "hook_event_name": "Stop", "transcript_path": "C:\\Windows\\win.ini"});
        assert_eq!(record(&paths, "claude", &elsewhere.to_string()).unwrap().transcript, None);
        assert!(record(&paths, "claude", r#"{"session_id":"s","hook_event_name":"PreToolUse"}"#).is_none());
        assert!(record(&paths, "codex", r#"{"type":"agent-turn-complete","thread-id":"../../evil","cwd":"C:\\x"}"#).unwrap().session == "evil");
        assert!(record(&paths, "codex", "not json").is_none());

        // Just now: waiting, but not worth a note yet.
        assert_eq!(current(&paths).len(), 3);
        assert!(due(&paths).is_empty());
        // Half a minute later it's said once.
        for (path, mut w) in read_all(&paths) {
            w.at -= NOTICE_AFTER + 5;
            std::fs::write(path, serde_json::to_string(&w).unwrap()).unwrap();
        }
        assert_eq!(due(&paths).len(), 3);
        assert!(due(&paths).is_empty(), "never twice");
        // The session moves on: it isn't waiting any more, and its record goes.
        set_time(SETTLE as i64 + 60);
        assert!(current(&paths).iter().all(|w| w.session != "s-9"));
        due(&paths);
        assert!(!dir(&paths).join("claude-s-9.json").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
