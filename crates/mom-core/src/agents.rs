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
    Antigravity,
    Goose,
    Amp,
    OpenCode,
    Aider,
    Pi,
    OpenClaw,
    Hermes,
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
            AgentKind::Antigravity => "Antigravity",
            AgentKind::Goose => "Goose",
            AgentKind::Amp => "Amp",
            AgentKind::OpenCode => "OpenCode",
            AgentKind::Aider => "Aider",
            AgentKind::Pi => "Pi",
            AgentKind::OpenClaw => "OpenClaw",
            AgentKind::Hermes => "Hermes Agent",
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
            AgentKind::Antigravity => "Antigravity",
            AgentKind::Goose => "Goose",
            AgentKind::Amp => "Amp",
            AgentKind::OpenCode => "OpenCode",
            AgentKind::Aider => "Aider",
            AgentKind::Pi => "Pi",
            AgentKind::OpenClaw => "OpenClaw",
            AgentKind::Hermes => "Hermes",
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
            AgentKind::Antigravity => "antigravity",
            AgentKind::Goose => "goose",
            AgentKind::Amp => "amp",
            AgentKind::OpenCode => "opencode",
            AgentKind::Aider => "aider",
            AgentKind::Pi => "pi",
            AgentKind::OpenClaw => "openclaw",
            AgentKind::Hermes => "hermes",
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
        } else if starts("antigravity") || n == "agy" || starts("agy-") || starts("agy_") {
            AgentKind::Antigravity
        } else if starts("goose") {
            AgentKind::Goose
        } else if starts("amp") {
            AgentKind::Amp
        } else if starts("opencode") {
            AgentKind::OpenCode
        } else if starts("aider") {
            AgentKind::Aider
        } else if n == "pi" || starts("pi-") || starts("pi_") {
            AgentKind::Pi
        } else if starts("openclaw") || starts("clawdbot") || starts("moltbot") {
            AgentKind::OpenClaw
        } else if starts("hermes") {
            AgentKind::Hermes
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

/// The app itself, on a Mac: the program an app's window belongs to (`X.app/Contents/MacOS/X`),
/// as opposed to a command-line tool of the same name, or one the app carries inside it.
fn is_mac_app(exe: &str) -> bool {
    exe.contains(".app\\contents\\macos\\")
}

pub fn agent_process(p: &Proc) -> Option<(AgentKind, AgentProc)> {
    let name = p.stem();
    let exe = p.lexe();
    let cmd = p.cmdline().to_ascii_lowercase().replace('\\', "/");
    let app = |kind: AgentKind| (!is_electron_helper(p)).then_some((kind, AgentProc::Host));
    match name.as_str() {
        "claude" => {
            if is_electron_helper(p) {
                return None;
            }
            // The Claude desktop app is also called claude; it hosts Claude Code sessions.
            if exe.contains("\\windowsapps\\claude_") || exe.contains("\\anthropicclaude\\") || is_mac_app(&exe) {
                Some((AgentKind::ClaudeCode, AgentProc::Host))
            } else {
                Some((AgentKind::ClaudeCode, AgentProc::Session))
            }
        }
        // On a Mac the Codex desktop app is Codex.app; the `codex` it carries inside does the work.
        "codex" if is_mac_app(&exe) => app(AgentKind::Codex),
        "codex" => Some((AgentKind::Codex, AgentProc::Session)),
        n if n.starts_with("codex-") && n.contains("sandbox") && !n.contains("setup") => {
            Some((AgentKind::Codex, AgentProc::Sandbox))
        }
        n if n.starts_with("codex-") && !n.contains("setup") => Some((AgentKind::Codex, AgentProc::Session)),
        // The Codex desktop app ships as ChatGPT.exe on Windows. (A Mac's ChatGPT.app is ChatGPT.)
        "chatgpt" if !is_mac_app(&exe) => app(AgentKind::Codex),
        "cursor" => app(AgentKind::Cursor),
        "windsurf" => app(AgentKind::Windsurf),
        // Google's Antigravity: the editor hosts its agents; `agy` is the one for the terminal.
        // Neither marks what it starts, so its work is known only while it's still running.
        "antigravity" => app(AgentKind::Antigravity),
        "agy" => Some((AgentKind::Antigravity, AgentProc::Session)),
        "code" | "code - insiders" => app(AgentKind::Copilot),
        // On a Mac, VS Code and Windsurf run as "Electron" inside their own app.
        "electron" if exe.contains("\\visual studio code") => app(AgentKind::Copilot),
        "electron" if exe.contains("\\windsurf") => app(AgentKind::Windsurf),
        // The OpenCode desktop app hosts sessions, and does its work through a copy of the
        // command-line one it keeps for itself (`opencode-cli serve`). On its own, that's a session.
        "opencode" if is_electron_helper(p) => None,
        "opencode" if is_mac_app(&exe) => app(AgentKind::OpenCode),
        "opencode" | "opencode-cli" => Some((AgentKind::OpenCode, AgentProc::Session)),
        "goose" => Some((AgentKind::Goose, AgentProc::Session)),
        "aider" => Some((AgentKind::Aider, AgentProc::Session)),
        // Hermes Agent's launcher in its Python environment.
        "hermes" => Some((AgentKind::Hermes, AgentProc::Session)),
        "droid" => Some((AgentKind::Other, AgentProc::Session)),
        "node" | "bun" => {
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
            } else if cmd.contains("pi-coding-agent") {
                Some((AgentKind::Pi, AgentProc::Session))
            } else if ["/openclaw/", "/clawdbot/", "/moltbot/", "openclaw.mjs"].iter().any(|m| cmd.contains(m)) {
                Some((AgentKind::OpenClaw, AgentProc::Session))
            } else if cmd.contains("@qwen-code/") {
                Some((AgentKind::Other, AgentProc::Session))
            } else {
                None
            }
        }
        n if n == "python" || n == "pythonw" || n == "python3" || n.starts_with("python3.") => {
            let aider = p
                .cmd
                .iter()
                .skip(1)
                .take(3)
                .any(|a| a == "aider" || a.ends_with("\\aider.exe") || a.ends_with("/aider"));
            if aider {
                Some((AgentKind::Aider, AgentProc::Session))
            } else if cmd.contains("hermes_cli") || cmd.contains("hermes-agent/") {
                Some((AgentKind::Hermes, AgentProc::Session))
            } else {
                None
            }
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

        let pi = fake(8, 0, "node.exe", r"node C:\npm\node_modules\@mariozechner\pi-coding-agent\dist\cli.js", 2);
        assert_eq!(agent_process(&pi), Some((AgentKind::Pi, AgentProc::Session)));
        let claw = fake(9, 0, "node.exe", r"node C:\npm\node_modules\openclaw\openclaw.mjs gateway", 2);
        assert_eq!(agent_process(&claw), Some((AgentKind::OpenClaw, AgentProc::Session)));
        let hermes = fake(10, 0, "python.exe", r"python C:\Users\me\AppData\Local\hermes\hermes-agent\cli.py", 2);
        assert_eq!(agent_process(&hermes), Some((AgentKind::Hermes, AgentProc::Session)));
        assert_eq!(agent_process(&fake(11, 0, "hermes.exe", "hermes chat", 2)), Some((AgentKind::Hermes, AgentProc::Session)));
        assert_eq!(agent_process(&fake(12, 0, "python.exe", "python manage.py runserver", 2)), None);
        assert_eq!(AgentKind::from_name("pi"), AgentKind::Pi);
        assert_eq!(AgentKind::from_name("pip"), AgentKind::Other);
    }

    #[test]
    fn recognises_agent_processes_on_a_mac() {
        let at = |pid: u32, name: &str, cmd: &str, exe: &str| {
            let mut p = fake(pid, 1, name, cmd, 1);
            p.exe = Some(exe.into());
            p
        };
        // The desktop app hosts sessions; its helpers are neither.
        let desktop = at(1, "Claude", "/Applications/Claude.app/Contents/MacOS/Claude", "/Applications/Claude.app/Contents/MacOS/Claude");
        assert_eq!(agent_process(&desktop), Some((AgentKind::ClaudeCode, AgentProc::Host)));
        let helper = at(2, "Claude Helper (Renderer)", "x --type=renderer", "/Applications/Claude.app/Contents/Frameworks/Claude Helper (Renderer).app/Contents/MacOS/Claude Helper (Renderer)");
        assert_eq!(agent_process(&helper), None);
        // Claude Code keeps itself under its version number and is run as "claude".
        let cli = at(3, "claude", "claude --dangerously-skip-permissions", "/Users/me/.local/share/claude/versions/2.1.331");
        assert_eq!(agent_process(&cli), Some((AgentKind::ClaudeCode, AgentProc::Session)));
        // The one the desktop app runs for a session of its own.
        let hosted = at(4, "claude", "claude --output-format stream-json", "/Users/me/Library/Application Support/Claude/claude-code/2.1.295/claude");
        assert_eq!(agent_process(&hosted), Some((AgentKind::ClaudeCode, AgentProc::Session)));

        let cursor = at(5, "Cursor", "/Applications/Cursor.app/Contents/MacOS/Cursor", "/Applications/Cursor.app/Contents/MacOS/Cursor");
        assert_eq!(agent_process(&cursor), Some((AgentKind::Cursor, AgentProc::Host)));
        let code = at(6, "Electron", "/Applications/Visual Studio Code.app/Contents/MacOS/Electron", "/Applications/Visual Studio Code.app/Contents/MacOS/Electron");
        assert_eq!(agent_process(&code), Some((AgentKind::Copilot, AgentProc::Host)));
        let some_app = at(7, "Electron", "/Applications/Other.app/Contents/MacOS/Electron", "/Applications/Other.app/Contents/MacOS/Electron");
        assert_eq!(agent_process(&some_app), None);

        let codex_app = at(8, "Codex", "/Applications/Codex.app/Contents/MacOS/Codex", "/Applications/Codex.app/Contents/MacOS/Codex");
        assert_eq!(agent_process(&codex_app), Some((AgentKind::Codex, AgentProc::Host)));
        let codex = at(9, "codex", "codex app-server", "/Applications/Codex.app/Contents/Resources/codex");
        assert_eq!(agent_process(&codex), Some((AgentKind::Codex, AgentProc::Session)));
        let opencode_app = at(10, "OpenCode", "/Applications/OpenCode.app/Contents/MacOS/OpenCode", "/Applications/OpenCode.app/Contents/MacOS/OpenCode");
        assert_eq!(agent_process(&opencode_app), Some((AgentKind::OpenCode, AgentProc::Host)));
        assert_eq!(agent_process(&at(11, "opencode", "opencode", "/opt/homebrew/bin/opencode")), Some((AgentKind::OpenCode, AgentProc::Session)));
        // The engine the OpenCode app runs for itself, wherever the app was opened from.
        let engine = at(14, "opencode-cli", "opencode-cli serve --service", "/Users/me/Library/Application Support/ai.opencode.desktop/cli/2.4.2-x64/opencode-cli");
        assert_eq!(agent_process(&engine), Some((AgentKind::OpenCode, AgentProc::Session)));
        let moved = at(15, "OpenCode", "x", "/private/var/folders/sb/T/AppTranslocation/E92F/d/OpenCode.app/Contents/MacOS/OpenCode");
        assert_eq!(agent_process(&moved), Some((AgentKind::OpenCode, AgentProc::Host)));

        // Antigravity: the editor, and `agy` in a terminal.
        let editor = at(16, "Antigravity", "x", "/Applications/Antigravity.app/Contents/MacOS/Antigravity");
        assert_eq!(agent_process(&editor), Some((AgentKind::Antigravity, AgentProc::Host)));
        assert_eq!(agent_process(&at(17, "agy", "agy --dangerously-skip-permissions", "/Users/me/.local/bin/agy")), Some((AgentKind::Antigravity, AgentProc::Session)));
        assert_eq!(agent_process(&at(18, "Antigravity Helper", "x --type=renderer", "/Applications/Antigravity.app/Contents/Frameworks/Antigravity Helper.app/Contents/MacOS/Antigravity Helper")), None);
        assert_eq!(AgentKind::from_name("agy"), AgentKind::Antigravity);

        let gemini = at(12, "node", "node /opt/homebrew/lib/node_modules/@google/gemini-cli/dist/index.js", "/opt/homebrew/bin/node");
        assert_eq!(agent_process(&gemini), Some((AgentKind::Gemini, AgentProc::Session)));
        let aider = at(13, "python3.12", "/Users/me/.local/pipx/venvs/aider-chat/bin/python /Users/me/.local/bin/aider", "/opt/homebrew/Cellar/python@3.12/3.12.4/bin/python3.12");
        assert_eq!(agent_process(&aider), Some((AgentKind::Aider, AgentProc::Session)));
    }
}
