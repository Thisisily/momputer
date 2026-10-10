//! Who's who: recognising agents, and the fingerprints they leave on what they launch.
//!
//! Two independent signals:
//! 1. **Markers**: environment variables an agent stamps into every child process
//!    (`CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`, `AI_AGENT`, `CODEX_THREAD_ID`, ...). They are
//!    inherited by grandchildren and survive the agent's death, so they identify leftovers.
//! 2. **Ancestry**: the agent process itself (claude.exe, codex.exe, ...) found by walking up
//!    the process tree. This tells us whether the agent is still around.
//!
//! Want mom to recognise another agent? Add it to `mark_from_env` and `agent_process`.

use crate::procs::Proc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentKind {
    ClaudeCode,
    Codex,
    Cursor,
    Gemini,
    Copilot,
    Windsurf,
    Goose,
    Amp,
    OpenCode,
    Aider,
    Other,
}

impl AgentKind {
    pub fn label(self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "Claude Code",
            AgentKind::Codex => "Codex",
            AgentKind::Cursor => "Cursor",
            AgentKind::Gemini => "Gemini CLI",
            AgentKind::Copilot => "GitHub Copilot",
            AgentKind::Windsurf => "Windsurf",
            AgentKind::Goose => "Goose",
            AgentKind::Amp => "Amp",
            AgentKind::OpenCode => "OpenCode",
            AgentKind::Aider => "Aider",
            AgentKind::Other => "An agent",
        }
    }

    /// What mom calls them: "Claude left this running".
    pub fn nickname(self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "Claude",
            AgentKind::Codex => "Codex",
            AgentKind::Cursor => "Cursor",
            AgentKind::Gemini => "Gemini",
            AgentKind::Copilot => "Copilot",
            AgentKind::Windsurf => "Windsurf",
            AgentKind::Goose => "Goose",
            AgentKind::Amp => "Amp",
            AgentKind::OpenCode => "OpenCode",
            AgentKind::Aider => "Aider",
            AgentKind::Other => "Some agent",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "claude-code",
            AgentKind::Codex => "codex",
            AgentKind::Cursor => "cursor",
            AgentKind::Gemini => "gemini",
            AgentKind::Copilot => "copilot",
            AgentKind::Windsurf => "windsurf",
            AgentKind::Goose => "goose",
            AgentKind::Amp => "amp",
            AgentKind::OpenCode => "opencode",
            AgentKind::Aider => "aider",
            AgentKind::Other => "other",
        }
    }

    fn from_name(name: &str) -> AgentKind {
        let n = name.to_ascii_lowercase();
        let starts = |p: &str| n.starts_with(p);
        if starts("claude") {
            AgentKind::ClaudeCode
        } else if starts("codex") {
            AgentKind::Codex
        } else if starts("cursor") {
            AgentKind::Cursor
        } else if starts("gemini") {
            AgentKind::Gemini
        } else if starts("copilot") || starts("github-copilot") {
            AgentKind::Copilot
        } else if starts("windsurf") || starts("codeium") {
            AgentKind::Windsurf
        } else if starts("goose") {
            AgentKind::Goose
        } else if starts("amp") {
            AgentKind::Amp
        } else if starts("opencode") {
            AgentKind::OpenCode
        } else if starts("aider") {
            AgentKind::Aider
        } else {
            AgentKind::Other
        }
    }
}

/// Did the agent's machinery start this (an MCP server, a hook), or did the agent itself
/// decide to run it (a dev server via its shell tool)?
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    Harness,
    Task,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMark {
    pub kind: AgentKind,
    pub session: Option<String>,
    /// PID of the agent process that launched this (Claude Code sets CLAUDE_PID).
    pub agent_pid: Option<u32>,
    pub role: Role,
    pub project: Option<String>,
    pub version: Option<String>,
    pub entry: Option<String>,
}

fn non_empty(env: &HashMap<String, String>, key: &str) -> Option<String> {
    env.get(key).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// Reads an agent's fingerprints from a process environment.
pub fn mark_from_env(env: &HashMap<String, String>) -> Option<AgentMark> {
    if env.is_empty() {
        return None;
    }
    let ai_agent = non_empty(env, "AI_AGENT").or_else(|| {
        non_empty(env, "AGENT").filter(|v| {
            v.len() <= 64
                && v.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_@.".contains(c))
        })
    });
    let base = |kind| AgentMark {
        kind,
        session: None,
        agent_pid: None,
        role: Role::Task,
        project: None,
        version: None,
        entry: None,
    };

    // Claude Code: CLAUDECODE=1 plus AI_AGENT=claude-code_<version>_<harness|agent>.
    let claude_flag = env.get("CLAUDECODE").map(|v| v == "1").unwrap_or(false);
    let claude_ai = ai_agent
        .as_deref()
        .map(|a| a.to_ascii_lowercase().starts_with("claude-code"))
        .unwrap_or(false);
    if claude_flag || claude_ai || env.contains_key("CLAUDE_CODE_SESSION_ID") {
        let mut m = base(AgentKind::ClaudeCode);
        m.session = non_empty(env, "CLAUDE_CODE_SESSION_ID");
        m.agent_pid = non_empty(env, "CLAUDE_PID").and_then(|v| v.parse().ok());
        m.project = non_empty(env, "CLAUDE_PROJECT_DIR");
        m.entry = non_empty(env, "CLAUDE_CODE_ENTRYPOINT");
        let tag = ai_agent.as_deref().unwrap_or("").to_ascii_lowercase();
        m.role = if tag.ends_with("_harness") {
            Role::Harness
        } else if tag.ends_with("_agent") || m.agent_pid.is_some() {
            Role::Task
        } else {
            Role::Unknown
        };
        m.version = tag
            .strip_prefix("claude-code_")
            .and_then(|rest| rest.split('_').next())
            .map(|v| v.replace('-', "."))
            .filter(|v| !v.is_empty());
        return Some(m);
    }

    // Codex: CODEX_THREAD_ID / CODEX_SANDBOX / CODEX_CI.
    let codex_ai = ai_agent
        .as_deref()
        .map(|a| a.to_ascii_lowercase().starts_with("codex"))
        .unwrap_or(false);
    if codex_ai
        || ["CODEX_THREAD_ID", "CODEX_SANDBOX", "CODEX_CI", "CODEX_SANDBOX_NETWORK_DISABLED"]
            .iter()
            .any(|k| env.contains_key(*k))
    {
        let mut m = base(AgentKind::Codex);
        m.session = non_empty(env, "CODEX_THREAD_ID");
        return Some(m);
    }

    if env.contains_key("CURSOR_AGENT") {
        return Some(base(AgentKind::Cursor));
    }
    if env.contains_key("GEMINI_CLI") {
        return Some(base(AgentKind::Gemini));
    }
    if ["COPILOT_AGENT", "COPILOT_CLI", "COPILOT_AGENT_SESSION_ID"]
        .iter()
        .any(|k| env.contains_key(*k))
    {
        let mut m = base(AgentKind::Copilot);
        m.session = non_empty(env, "COPILOT_AGENT_SESSION_ID");
        return Some(m);
    }
    if env.get("OPENCODE").map(|v| v == "1").unwrap_or(false) {
        return Some(base(AgentKind::OpenCode));
    }
    if let Some(a) = ai_agent {
        let mut m = base(AgentKind::from_name(&a));
        m.version = Some(a);
        return Some(m);
    }
    None
}

/// Is this process an agent (or an app that hosts agents)?
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentProc {
    /// An agent session: everything it spawns is the agent's doing.
    Session,
    /// An app or IDE that hosts agents but also has a human at the keyboard. Things under it
    /// without agent markers are the human's.
    Host,
    /// A long-lived helper (like Codex's Windows sandbox service) that runs commands for an
    /// agent. What it runs is the agent's, and only "alive" while a session of that agent is.
    Sandbox,
}

fn is_electron_helper(p: &Proc) -> bool {
    p.cmd.iter().any(|a| a.starts_with("--type="))
}

pub fn agent_process(p: &Proc) -> Option<(AgentKind, AgentProc)> {
    let name = p.lname();
    let exe = p.lexe();
    let cmd = p.cmdline().to_ascii_lowercase().replace('\\', "/");
    match name.as_str() {
        "claude.exe" | "claude" => {
            if is_electron_helper(p) {
                return None;
            }
            // The Claude desktop app is also called claude.exe; it hosts Claude Code sessions.
            if exe.contains("\\windowsapps\\claude_") || exe.contains("\\anthropicclaude\\") {
                Some((AgentKind::ClaudeCode, AgentProc::Host))
            } else {
                Some((AgentKind::ClaudeCode, AgentProc::Session))
            }
        }
        "codex.exe" | "codex" => Some((AgentKind::Codex, AgentProc::Session)),
        n if n.starts_with("codex-") && n.contains("sandbox") && !n.contains("setup") => {
            Some((AgentKind::Codex, AgentProc::Sandbox))
        }
        n if n.starts_with("codex-") && !n.contains("setup") => Some((AgentKind::Codex, AgentProc::Session)),
        // The Codex desktop app ships as ChatGPT.exe.
        "chatgpt.exe" => (!is_electron_helper(p)).then_some((AgentKind::Codex, AgentProc::Host)),
        "cursor.exe" => (!is_electron_helper(p)).then_some((AgentKind::Cursor, AgentProc::Host)),
        "windsurf.exe" => (!is_electron_helper(p)).then_some((AgentKind::Windsurf, AgentProc::Host)),
        "code.exe" | "code - insiders.exe" => {
            (!is_electron_helper(p)).then_some((AgentKind::Copilot, AgentProc::Host))
        }
        "opencode.exe" | "opencode" => Some((AgentKind::OpenCode, AgentProc::Session)),
        "goose.exe" | "goose" => Some((AgentKind::Goose, AgentProc::Session)),
        "aider.exe" | "aider" => Some((AgentKind::Aider, AgentProc::Session)),
        "droid.exe" => Some((AgentKind::Other, AgentProc::Session)),
        "node.exe" | "node" | "bun.exe" => {
            if cmd.contains("@anthropic-ai/claude-code") {
                Some((AgentKind::ClaudeCode, AgentProc::Session))
            } else if cmd.contains("@google/gemini-cli") || cmd.contains("/gemini-cli/") {
                Some((AgentKind::Gemini, AgentProc::Session))
            } else if cmd.contains("@openai/codex") {
                Some((AgentKind::Codex, AgentProc::Session))
            } else if cmd.contains("@sourcegraph/amp") {
                Some((AgentKind::Amp, AgentProc::Session))
            } else if cmd.contains("@github/copilot") {
                Some((AgentKind::Copilot, AgentProc::Session))
            } else if cmd.contains("@qwen-code/") {
                Some((AgentKind::Other, AgentProc::Session))
            } else {
                None
            }
        }
        "python.exe" | "python3.exe" | "pythonw.exe" => {
            let aider = p
                .cmd
                .iter()
                .skip(1)
                .take(3)
                .any(|a| a == "aider" || a.ends_with("\\aider.exe") || a.ends_with("/aider"));
            aider.then_some((AgentKind::Aider, AgentProc::Session))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procs::fake;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn claude_task_process_from_real_world_env() {
        let m = mark_from_env(&env(&[
            ("AI_AGENT", "claude-code_2-1-288_agent"),
            ("CLAUDECODE", "1"),
            ("CLAUDE_CODE_ENTRYPOINT", "cli"),
            ("CLAUDE_CODE_SESSION_ID", "0191f20b-ae85-40af-b308-93c34777b8cb"),
            ("CLAUDE_PID", "134696"),
        ]))
        .unwrap();
        assert_eq!(m.kind, AgentKind::ClaudeCode);
        assert_eq!(m.role, Role::Task);
        assert_eq!(m.agent_pid, Some(134696));
        assert_eq!(m.version.as_deref(), Some("2.1.288"));
        assert_eq!(m.session.as_deref(), Some("0191f20b-ae85-40af-b308-93c34777b8cb"));
    }

    #[test]
    fn claude_mcp_server_is_harness() {
        let m = mark_from_env(&env(&[
            ("AI_AGENT", "claude-code_2-1-295_harness"),
            ("CLAUDECODE", "1"),
            ("CLAUDE_CODE_SESSION_ID", "e9c3412f"),
            ("CLAUDE_PROJECT_DIR", r"C:\Workspace\momputer"),
        ]))
        .unwrap();
        assert_eq!(m.role, Role::Harness);
        assert_eq!(m.agent_pid, None);
        assert_eq!(m.project.as_deref(), Some(r"C:\Workspace\momputer"));
    }

    #[test]
    fn other_agents() {
        let c = mark_from_env(&env(&[("CODEX_THREAD_ID", "thr_123")])).unwrap();
        assert_eq!((c.kind, c.session.as_deref()), (AgentKind::Codex, Some("thr_123")));
        assert_eq!(mark_from_env(&env(&[("CURSOR_AGENT", "1")])).unwrap().kind, AgentKind::Cursor);
        assert_eq!(mark_from_env(&env(&[("GEMINI_CLI", "1")])).unwrap().kind, AgentKind::Gemini);
        assert_eq!(mark_from_env(&env(&[("AI_AGENT", "goose")])).unwrap().kind, AgentKind::Goose);
        assert_eq!(mark_from_env(&env(&[("AGENT", "amp")])).unwrap().kind, AgentKind::Amp);
        assert!(mark_from_env(&env(&[("AGENT", "some value with spaces")])).is_none());
        assert!(mark_from_env(&env(&[("TERM_PROGRAM", "vscode")])).is_none());
    }

    #[test]
    fn recognises_agent_processes() {
        let mut desktop = fake(1, 0, "claude.exe", "claude.exe", 1);
        desktop.exe = Some(r"C:\Program Files\WindowsApps\Claude_2.31226.1.0_x64__pzs8sxrjxfjjc\app\claude.exe".into());
        assert_eq!(agent_process(&desktop), Some((AgentKind::ClaudeCode, AgentProc::Host)));

        let mut cli = fake(2, 1, "claude.exe", "claude.exe --output-format stream-json", 2);
        cli.exe = Some(r"C:\Users\me\AppData\Roaming\Claude\claude-code\2.1.295\claude.exe".into());
        assert_eq!(agent_process(&cli), Some((AgentKind::ClaudeCode, AgentProc::Session)));

        let helper = fake(3, 1, "claude.exe", "claude.exe --type=renderer", 2);
        assert_eq!(agent_process(&helper), None);

        let npm_claude = fake(4, 0, "node.exe", r"node C:\npm\node_modules\@anthropic-ai\claude-code\cli.js", 2);
        assert_eq!(agent_process(&npm_claude), Some((AgentKind::ClaudeCode, AgentProc::Session)));

        assert_eq!(
            agent_process(&fake(5, 0, "codex.exe", "codex app-server", 2)),
            Some((AgentKind::Codex, AgentProc::Session))
        );
        assert_eq!(
            agent_process(&fake(6, 0, "codex-windows-sandbox-service.exe", "x", 2)),
            Some((AgentKind::Codex, AgentProc::Sandbox))
        );
        assert_eq!(agent_process(&fake(7, 0, "node.exe", "node vite.js", 2)), None);
    }
}
