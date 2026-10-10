//! Claude Code's scratch folders: which project and session each belongs to, and what the
//! session was about.
//!
//! Claude keeps scratch space in %TEMP%\claude\<slug>\<session id> and the session's transcript
//! in ~/.claude/projects/<slug>/<session id>.jsonl. The slug is the project's path with every
//! character that isn't a letter or digit turned into '-': C:\Workspace\storefront becomes
//! "C--Workspace-storefront".

use super::{ago, cands, entries, judge, name_of, tally_many, too_fresh, Cand, Found, TrashCtx, ENTRY_CAP, IN_USE};
use crate::util::{ellipsize, human_age_long, mtime_secs};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub(super) fn find(ctx: &TrashCtx, min_age: u64) -> Found {
    let root = ctx.paths.temp.join("claude");
    if !root.is_dir() {
        return Found::none();
    }
    let mut f = Found::at(root.clone());
    let transcripts = transcripts(&ctx.paths.claude_dir().join("projects"));
    // Loose files straight in claude\, and every session folder with its project's name.
    let mut loose = Vec::new();
    let mut sessions: Vec<(PathBuf, String)> = Vec::new();
    for slug in entries(&root) {
        if !slug.is_dir() {
            let n = name_of(&slug);
            loose.push((slug, n));
            continue;
        }
        let project = project_name(&name_of(&slug));
        sessions.extend(entries(&slug).into_iter().map(|s| (s, project.clone())));
    }
    f.cands = cands(loose, ctx.now).into_iter().map(|c| judge(c, ctx, min_age)).collect();
    let paths: Vec<PathBuf> = sessions.iter().map(|(s, _)| s.clone()).collect();
    for ((session, project), t) in sessions.into_iter().zip(tally_many(&paths, ENTRY_CAP, u64::MAX)) {
        let id = name_of(&session);
        let (transcript, talked) = match transcripts.get(&id.to_lowercase()) {
            Some((p, at)) => (Some(p.clone()), *at),
            None => (None, 0),
        };
        let short: String = id.chars().take(8).collect();
        let mut c = Cand::new(session.clone(), format!("{project} · Session {short}"), &t, ctx.now);
        c.age = ctx.now.saturating_sub(t.newest.max(talked));
        c.title_from = transcript.map(|p| (p, project.clone()));
        let c = if ctx.session_live(&id) {
            c.verdict(false, "Session is still running")
        } else if ctx.in_use(&session) {
            c.verdict(false, IN_USE)
        } else if c.age < min_age {
            let r = if talked > t.newest {
                format!("Session active {} ago (kept for {})", ago(c.age), human_age_long(min_age))
            } else {
                too_fresh(c.age, min_age)
            };
            c.verdict(false, r)
        } else {
            let r = format!("Session ended {} ago", ago(c.age));
            c.verdict(true, r)
        };
        f.cands.push(c);
    }
    f
}

/// Every transcript by session id, with when it was last written. A session's scratch folder
/// and transcript usually share a slug, but not when the session moved folders.
fn transcripts(projects: &Path) -> HashMap<String, (PathBuf, u64)> {
    let mut out = HashMap::new();
    for dir in entries(projects) {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if let Some(id) = n.strip_suffix(".jsonl") {
                // On Windows the listing already carries the time; no need to open each file.
                let at = e.metadata().map(|m| mtime_secs(&m)).unwrap_or(0);
                out.insert(id.to_lowercase(), (e.path(), at));
            }
        }
    }
    out
}

fn slugify(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// A short, human name for the project a slug stands for: "storefront", or
/// "Desktop-Client (worktree fix-dates)". Checks the disk so names with dashes survive.
pub(super) fn project_name(slug: &str) -> String {
    match resolve(slug) {
        Some(parts) if !parts.is_empty() => name_from_parts(&parts),
        _ => guess(slug),
    }
}

/// Walks the disk from the drive down, matching folder names against the slug. Returns the
/// folder names it found, plus whatever was left over (a folder that's gone now) as the last part.
fn resolve(slug: &str) -> Option<Vec<String>> {
    let (drive, rest) = slug.split_once("--")?;
    if drive.len() != 1 || !drive.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut cur = PathBuf::from(format!("{drive}:\\"));
    if !cur.is_dir() {
        return None;
    }
    let mut parts = Vec::new();
    let mut rest = rest.to_string();
    while !rest.is_empty() && parts.len() < 32 {
        let lower = rest.to_ascii_lowercase();
        let best = std::fs::read_dir(&cur)
            .ok()?
            .filter_map(Result::ok)
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| {
                let s = slugify(n).to_ascii_lowercase();
                lower == s || lower.starts_with(&format!("{s}-"))
            })
            .max_by_key(|n| slugify(n).len());
        match best {
            Some(n) => {
                rest = rest[slugify(&n).len()..].to_string();
                if let Some(r) = rest.strip_prefix('-') {
                    rest = r.to_string();
                }
                cur.push(&n);
                parts.push(n);
            }
            // Nothing on disk matched even the first folder: a guess beats a raw slug.
            None if parts.is_empty() => return None,
            None => {
                parts.push(rest);
                break;
            }
        }
    }
    Some(parts)
}

fn name_from_parts(parts: &[String]) -> String {
    let lower: Vec<String> = parts.iter().map(|p| p.to_lowercase()).collect();
    // Claude's worktrees: <project>\.claude\worktrees\<name>
    if let Some(i) = lower.iter().position(|p| p == ".claude") {
        if lower.get(i + 1).map(String::as_str) == Some("worktrees") && i > 0 {
            if let Some(w) = parts.get(i + 2) {
                return format!("{} (worktree {w})", parts[i - 1]);
            }
        }
    }
    if lower.len() == 2 && lower[0] == "users" {
        return "home folder".into();
    }
    if lower.iter().any(|p| p == "scratch-workspaces") {
        return "Claude scratch space".into();
    }
    parts.last().cloned().unwrap_or_default()
}

/// When the folders are gone: drop the drive and the usual workspace prefixes.
fn guess(slug: &str) -> String {
    let rest = slug.split_once("--").map(|(_, r)| r).unwrap_or(slug);
    for prefix in ["Workspace-", "workspace-", "Projects-", "projects-", "Code-", "code-", "dev-", "src-", "repos-"] {
        if let Some(r) = rest.strip_prefix(prefix) {
            return r.to_string();
        }
    }
    if let Some(r) = rest.strip_prefix("Users-") {
        if let Some((_, after_user)) = r.split_once('-') {
            return after_user.to_string();
        }
        return "home folder".into();
    }
    rest.to_string()
}

const HEAD_BYTES: u64 = 256 * 1024;
const TAIL_BYTES: u64 = 64 * 1024;

/// What a session was about, from its transcript: the title the user gave it, else Claude's own
/// title, else the summary, else the first thing the user typed. Reads at most the first 256 KB
/// (and the last 64 KB, where renamed titles end up).
pub(super) fn session_title(transcript: &Path) -> Option<String> {
    let mut file = std::fs::File::open(transcript).ok()?;
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut head = Vec::new();
    file.by_ref().take(HEAD_BYTES).read_to_end(&mut head).ok()?;
    let mut found = Titles::default();
    for line in head.split(|b| *b == b'\n').take(200) {
        found.look(line);
    }
    if found.custom.is_none() && len > HEAD_BYTES {
        let start = len.saturating_sub(TAIL_BYTES).max(HEAD_BYTES);
        let mut tail = Vec::new();
        if file.seek(SeekFrom::Start(start)).is_ok() && file.take(TAIL_BYTES).read_to_end(&mut tail).is_ok() {
            // The first line is probably cut off; the rest are whole. Later titles win.
            let mut late = Titles::default();
            for line in tail.split(|b| *b == b'\n').skip(1) {
                late.look_for_titles(line);
            }
            found.custom = late.custom.or(found.custom);
            found.ai = late.ai.or(found.ai);
        }
    }
    found.custom.or(found.ai).or(found.summary).or(found.first_prompt).map(|t| tidy_title(&t))
}

#[derive(Default)]
struct Titles {
    custom: Option<String>,
    ai: Option<String>,
    summary: Option<String>,
    first_prompt: Option<String>,
}

fn has(line: &[u8], needle: &str) -> bool {
    line.windows(needle.len()).any(|w| w == needle.as_bytes())
}

fn parse(line: &[u8]) -> Option<serde_json::Value> {
    serde_json::from_slice(line).ok()
}

fn text_field(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

impl Titles {
    /// Later lines overwrite: the newest title wins.
    fn look_for_titles(&mut self, line: &[u8]) {
        if has(line, "\"custom-title\"") {
            if let Some(t) = parse(line).and_then(|v| text_field(&v, "customTitle")) {
                self.custom = Some(t);
            }
        } else if has(line, "\"ai-title\"") {
            if let Some(t) = parse(line).and_then(|v| text_field(&v, "aiTitle")) {
                self.ai = Some(t);
            }
        }
    }

    fn look(&mut self, line: &[u8]) {
        self.look_for_titles(line);
        if self.summary.is_none() && has(line, "\"summary\"") {
            if let Some(v) = parse(line).filter(|v| v.get("type").and_then(|t| t.as_str()) == Some("summary")) {
                self.summary = text_field(&v, "summary");
            }
        }
        if self.first_prompt.is_none() && has(line, "\"user\"") {
            self.first_prompt = parse(line).and_then(|v| first_prompt(&v));
        }
    }
}

/// The user's own words from a "user" line: plain text, not a tool result or a system note.
fn first_prompt(v: &serde_json::Value) -> Option<String> {
    if v.get("type").and_then(|t| t.as_str()) != Some("user") || v.get("isMeta").and_then(|m| m.as_bool()) == Some(true) {
        return None;
    }
    let content = v.get("message")?.get("content")?;
    let text = match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(blocks) => blocks
            .iter()
            .find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
            .and_then(|b| b.get("text"))
            .and_then(|t| t.as_str())?
            .to_string(),
        _ => return None,
    };
    let text = text.trim();
    if text.is_empty() || text.starts_with('<') || text.starts_with("Caveat:") || text.starts_with('[') {
        return None;
    }
    Some(text.to_string())
}

/// First line only, no runs of spaces, about 60 characters.
fn tidy_title(s: &str) -> String {
    let line = s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
    ellipsize(&line, 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trash::tests::fixture;

    fn transcript(dir: &Path, name: &str, lines: &[&str]) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, lines.join("\n")).unwrap();
        p
    }

    #[test]
    fn titles_come_from_the_transcript() {
        let root = fixture("titles");
        let meta = r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"Caveat: local commands"}}"#;
        let tool = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"ok"}]}}"#;
        let prompt = r#"{"type":"user","message":{"role":"user","content":"Make the login page   work again\nIt broke yesterday."}}"#;
        let summary = r#"{"type":"summary","summary":"Login page repair","leafUuid":"x"}"#;
        let ai = r#"{"type":"ai-title","aiTitle":"Fixing login","sessionId":"s"}"#;
        let custom = r#"{"type":"custom-title","customTitle":"Login rescue","sessionId":"s"}"#;
        let p = transcript(&root, "a.jsonl", &["{\"type\":\"queue-operation\"}", meta, tool, prompt]);
        assert_eq!(session_title(&p).as_deref(), Some("Make the login page work again"));
        let p = transcript(&root, "b.jsonl", &[prompt, summary]);
        assert_eq!(session_title(&p).as_deref(), Some("Login page repair"));
        let p = transcript(&root, "c.jsonl", &[prompt, summary, ai]);
        assert_eq!(session_title(&p).as_deref(), Some("Fixing login"));
        let p = transcript(&root, "d.jsonl", &[custom, prompt, ai]);
        assert_eq!(session_title(&p).as_deref(), Some("Login rescue"));
        // Text blocks count as the user's words too; long ones are trimmed.
        let long = format!(r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"text","text":"{}"}}]}}}}"#, "word ".repeat(40));
        let p = transcript(&root, "e.jsonl", &[&long]);
        let t = session_title(&p).unwrap();
        assert!(t.chars().count() <= 60 && t.ends_with('…'), "{t}");
        // A title written late in a long transcript is still found.
        let filler = format!(r#"{{"type":"assistant","message":{{"content":"{}"}}}}"#, "x".repeat(1000));
        let mut lines = vec![prompt];
        lines.extend(std::iter::repeat_n(filler.as_str(), 400));
        lines.push(custom);
        let p = transcript(&root, "f.jsonl", &lines);
        assert_eq!(session_title(&p).as_deref(), Some("Login rescue"));
        assert_eq!(session_title(&root.join("missing.jsonl")), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn slugs_become_project_names() {
        assert_eq!(guess("C--Workspace-storefront"), "storefront");
        assert_eq!(guess("C--Users-me-side-project"), "side-project");
        assert_eq!(guess("C--Users-me"), "home folder");
        // Same answer whether or not C:\Workspace exists on this machine.
        assert_eq!(project_name("C--Workspace-nosuchproject-mom"), "nosuchproject-mom");
        let parts = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(name_from_parts(&parts(&["Workspace", "Desktop-Client", ".claude", "worktrees", "fix-dates"])), "Desktop-Client (worktree fix-dates)");
        assert_eq!(name_from_parts(&parts(&["Workspace", "storefront"])), "storefront");
        // On disk: a folder with dashes and dots in its name resolves exactly.
        let root = fixture("slugs");
        std::fs::create_dir_all(root.join("my-app.v2").join("web")).unwrap();
        let full = std::fs::canonicalize(root.join("my-app.v2").join("web")).unwrap();
        let full = full.to_string_lossy().trim_start_matches(r"\\?\").to_string();
        let slug = slugify(&full);
        let found = resolve(&slug).unwrap();
        assert_eq!(found[found.len() - 2..], parts(&["my-app.v2", "web"]));
        assert_eq!(project_name(&slug), "web");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
