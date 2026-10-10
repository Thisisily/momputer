//! House rules: one text, delivered to every agent.
//!
//! The rules live in ~/.momputer/house-rules.md (falling back to the built-in copy). Mom can
//! write them into each agent's global instructions file inside a clearly marked block, and
//! take them out again without touching anything else in the file.

use crate::agents::AgentKind;
use crate::paths::Paths;
use serde::Serialize;
use std::path::PathBuf;

pub const DEFAULT_RULES: &str = include_str!("../../../skills/mom/house-rules.md");
pub const SKILL_MD: &str = include_str!("../../../skills/mom/SKILL.md");

const BEGIN: &str = "<!-- momputer:house-rules:begin (managed by momputer; edit them in the momputer app or ~/.momputer/house-rules.md) -->";
const BEGIN_PREFIX: &str = "<!-- momputer:house-rules:begin";
const END: &str = "<!-- momputer:house-rules:end -->";

const CLI_HINT: &str = "If the `mom` command is available: `mom mine` lists what you left running, `mom stop mine` stops it, and `mom tidy <folder>` grades a folder and lists what to fix.";

pub fn current(paths: &Paths) -> String {
    std::fs::read_to_string(paths.rules_file())
        .ok()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_RULES.to_string())
}

pub fn is_customized(paths: &Paths) -> bool {
    paths.rules_file().exists()
}

pub fn save(paths: &Paths, text: &str) -> anyhow::Result<()> {
    crate::util::write_atomic(&paths.rules_file(), text)?;
    Ok(())
}

pub fn reset(paths: &Paths) -> anyhow::Result<()> {
    let f = paths.rules_file();
    if f.exists() {
        std::fs::remove_file(f)?;
    }
    Ok(())
}

/// The block as it appears inside an agent's instructions file.
pub fn block(rules: &str) -> String {
    format!("{BEGIN}\n{}\n\n{CLI_HINT}\n{END}", rules.trim())
}

/// Inserts or replaces mom's block, leaving everything else exactly as it was.
pub fn upsert_block(existing: &str, rules: &str) -> String {
    let crlf = existing.contains("\r\n");
    let body = block(rules);
    let body = if crlf { body.replace('\n', "\r\n") } else { body };
    let nl = if crlf { "\r\n" } else { "\n" };
    if let (Some(start), Some(end_rel)) = (existing.find(BEGIN_PREFIX), existing.find(END)) {
        if end_rel > start {
            let end = end_rel + END.len();
            return format!("{}{}{}", &existing[..start], body, &existing[end..]);
        }
    }
    if existing.trim().is_empty() {
        return format!("{body}{nl}");
    }
    let sep = if existing.ends_with(&format!("{nl}{nl}")) {
        ""
    } else if existing.ends_with(nl) {
        nl
    } else {
        // no trailing newline at all
        if crlf { "\r\n\r\n" } else { "\n\n" }
    };
    format!("{existing}{sep}{body}{nl}")
}

/// Removes mom's block (and the blank line she added before it).
pub fn strip_block(existing: &str) -> String {
    let (Some(start), Some(end_rel)) = (existing.find(BEGIN_PREFIX), existing.find(END)) else {
        return existing.to_string();
    };
    if end_rel < start {
        return existing.to_string();
    }
    let mut end = end_rel + END.len();
    let bytes = existing.as_bytes();
    if existing[end..].starts_with("\r\n") {
        end += 2;
    } else if end < bytes.len() && bytes[end] == b'\n' {
        end += 1;
    }
    let head = existing[..start].trim_end_matches(['\n', '\r']);
    let tail = &existing[end..];
    if head.is_empty() {
        return tail.to_string();
    }
    let nl = if existing.contains("\r\n") { "\r\n" } else { "\n" };
    if tail.is_empty() {
        format!("{head}{nl}")
    } else {
        format!("{head}{nl}{nl}{tail}")
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub agent: AgentKind,
    pub label: String,
    pub path: String,
    /// The agent seems to be installed (its config folder exists).
    pub available: bool,
    pub installed: bool,
    pub up_to_date: bool,
}

fn target_path(paths: &Paths, agent: AgentKind) -> Option<PathBuf> {
    match agent {
        AgentKind::ClaudeCode => Some(paths.claude_dir().join("CLAUDE.md")),
        AgentKind::Codex => Some(paths.codex_dir().join("AGENTS.md")),
        AgentKind::Gemini => Some(paths.gemini_dir().join("GEMINI.md")),
        _ => None,
    }
}

pub fn targets(paths: &Paths) -> Vec<Target> {
    let rules = current(paths);
    let want = block(&rules);
    [AgentKind::ClaudeCode, AgentKind::Codex, AgentKind::Gemini]
        .into_iter()
        .filter_map(|a| {
            let p = target_path(paths, a)?;
            let text = std::fs::read_to_string(&p).unwrap_or_default().replace("\r\n", "\n");
            let installed = text.contains(BEGIN_PREFIX);
            Some(Target {
                agent: a,
                label: a.label().to_string(),
                path: p.to_string_lossy().to_string(),
                available: p.parent().map(|d| d.is_dir()).unwrap_or(false),
                installed,
                up_to_date: installed && text.contains(&want),
            })
        })
        .collect()
}

fn backup(paths: &Paths, file: &std::path::Path, agent: AgentKind) {
    if !file.exists() {
        return;
    }
    let dir = paths.backups_dir();
    if std::fs::create_dir_all(&dir).is_ok() {
        let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let _ = std::fs::copy(file, dir.join(format!("{}-{}-{}.bak", agent.slug(), name, crate::util::now_secs())));
    }
}

/// Writes (or refreshes) the house rules into an agent's global instructions file.
pub fn install(paths: &Paths, agent: AgentKind) -> anyhow::Result<String> {
    let file = target_path(paths, agent).ok_or_else(|| anyhow::anyhow!("mom doesn't know where {} keeps its instructions", agent.label()))?;
    let existing = std::fs::read_to_string(&file).unwrap_or_default();
    let updated = upsert_block(&existing, &current(paths));
    if updated == existing {
        return Ok(format!("{} already knows the house rules.", agent.label()));
    }
    backup(paths, &file, agent);
    crate::util::write_atomic(&file, &updated)?;
    Ok(format!("Taught {} the house rules ({}).", agent.label(), file.display()))
}

pub fn remove(paths: &Paths, agent: AgentKind) -> anyhow::Result<String> {
    let file = target_path(paths, agent).ok_or_else(|| anyhow::anyhow!("unknown agent"))?;
    let Ok(existing) = std::fs::read_to_string(&file) else {
        return Ok(format!("{} never had the house rules.", agent.label()));
    };
    let updated = strip_block(&existing);
    if updated == existing {
        return Ok(format!("{} never had the house rules.", agent.label()));
    }
    backup(paths, &file, agent);
    crate::util::write_atomic(&file, &updated)?;
    Ok(format!("Took the house rules out of {}.", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_is_idempotent_and_preserves_the_rest() {
        let original = "# My rules\n\nBe nice.\n";
        let once = upsert_block(original, "1. Stop what you start.");
        assert!(once.starts_with("# My rules\n\nBe nice.\n\n<!-- momputer"));
        assert!(once.contains("1. Stop what you start."));
        let twice = upsert_block(&once, "1. Stop what you start.");
        assert_eq!(once, twice);
        let changed = upsert_block(&once, "1. Changed.");
        assert!(changed.contains("1. Changed.") && !changed.contains("Stop what you start"));
        assert_eq!(strip_block(&changed), original);
    }

    #[test]
    fn handles_empty_files_and_crlf() {
        let fresh = upsert_block("", "rules");
        assert!(fresh.starts_with("<!-- momputer"));
        assert_eq!(strip_block(&fresh), "");
        let crlf = "line one\r\nline two\r\n";
        let with = upsert_block(crlf, "a\nb");
        assert!(!with.replace("\r\n", "").contains('\n'), "all CRLF: {with:?}");
        assert_eq!(strip_block(&with), crlf);
    }

    #[test]
    fn install_and_remove_in_a_fake_home() {
        let root = std::env::temp_dir().join(format!("mom-rules-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::under(&root);
        std::fs::create_dir_all(paths.claude_dir()).unwrap();
        std::fs::write(paths.claude_dir().join("CLAUDE.md"), "# Mine\n").unwrap();
        let t = targets(&paths);
        assert!(t.iter().find(|t| t.agent == AgentKind::ClaudeCode).unwrap().available);
        assert!(!t.iter().find(|t| t.agent == AgentKind::Codex).unwrap().available);

        install(&paths, AgentKind::ClaudeCode).unwrap();
        let t = targets(&paths);
        let c = t.iter().find(|t| t.agent == AgentKind::ClaudeCode).unwrap();
        assert!(c.installed && c.up_to_date);
        assert!(std::fs::read_dir(paths.backups_dir()).unwrap().count() == 1, "backed up first");

        save(&paths, "1. New rule.").unwrap();
        assert!(!targets(&paths).iter().find(|t| t.agent == AgentKind::ClaudeCode).unwrap().up_to_date);

        remove(&paths, AgentKind::ClaudeCode).unwrap();
        assert_eq!(std::fs::read_to_string(paths.claude_dir().join("CLAUDE.md")).unwrap(), "# Mine\n");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
