//! Agent integrations: hooks that let mom check in with Claude Code and Codex, plus the
//! installers for those hooks and for the `mom` skill.
//!
//! * **SessionStart** remembers what the project folder looked like and hands the agent the
//!   house rules (unless they're already in its instructions file).
//! * **Stop** runs when the agent thinks it's done. If it left servers running or dropped new
//!   clutter in the project root, mom asks it to deal with that first. She asks once per
//!   thing, and never twice in a row, so an agent can't get stuck.
//! * **SessionEnd** notes what the session left behind (and can stop it, if you opt in).
//!
//! Hooks must never break an agent: any error means "carry on", silently.

use crate::agents::{AgentKind, AgentProc};
use crate::config::Config;
use crate::paths::Paths;
use crate::scan::{self, Group, Status};
use crate::util::{human_age, now_secs};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookAgent {
    Claude,
    Codex,
}

impl HookAgent {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "claude" | "claude-code" => Some(HookAgent::Claude),
            "codex" => Some(HookAgent::Codex),
            _ => None,
        }
    }
    pub fn kind(self) -> AgentKind {
        match self {
            HookAgent::Claude => AgentKind::ClaudeCode,
            HookAgent::Codex => AgentKind::Codex,
        }
    }
    pub fn slug(self) -> &'static str {
        match self {
            HookAgent::Claude => "claude",
            HookAgent::Codex => "codex",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookEvent {
    SessionStart,
    Stop,
    SessionEnd,
}

impl HookEvent {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().replace('_', "-").as_str() {
            "session-start" | "sessionstart" => Some(HookEvent::SessionStart),
            "stop" => Some(HookEvent::Stop),
            "session-end" | "sessionend" => Some(HookEvent::SessionEnd),
            _ => None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct HookInput {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    stop_hook_active: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SessionState {
    cwd: String,
    started: u64,
    baseline: Vec<String>,
    nagged: Vec<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct HookOutput {
    pub stdout: String,
    pub exit_code: i32,
}

fn state_file(paths: &Paths, agent: HookAgent, session: &str) -> PathBuf {
    let safe: String = session
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(80)
        .collect();
    paths.state_dir().join("sessions").join(format!("{}-{}.json", agent.slug(), safe))
}

fn load_state(file: &Path) -> SessionState {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_state(file: &Path, st: &SessionState) {
    if let Ok(text) = serde_json::to_string(st) {
        let _ = crate::util::write_atomic(file, &text);
    }
}

fn top_level(dir: &str) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().to_string()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// The path agents should use to run mom, as a quoted string that works in bash and cmd.
pub fn mom_command(paths: &Paths) -> String {
    let installed = paths.installed_cli();
    let exe = if installed.exists() {
        installed
    } else {
        std::env::current_exe().unwrap_or(installed)
    };
    format!("\"{}\"", exe.to_string_lossy().replace('\\', "/"))
}

/// Runs a hook. Never panics, never returns an error: agents must not be broken by mom.
pub fn handle(agent: HookAgent, event: HookEvent, input: &str, paths: &Paths) -> HookOutput {
    let input = input.to_string();
    let paths = paths.clone();
    std::panic::catch_unwind(move || handle_inner(agent, event, &input, &paths)).unwrap_or_default()
}

fn handle_inner(agent: HookAgent, event: HookEvent, input: &str, paths: &Paths) -> HookOutput {
    let cfg = Config::load(paths);
    if !cfg.hooks.enabled {
        return HookOutput::default();
    }
    let inp: HookInput = serde_json::from_str(input).unwrap_or_default();
    let file = state_file(paths, agent, if inp.session_id.is_empty() { "unknown" } else { &inp.session_id });
    match event {
        HookEvent::SessionStart => session_start(agent, &inp, &cfg, paths, &file),
        HookEvent::Stop => stop(agent, &inp, &cfg, paths, &file),
        HookEvent::SessionEnd => session_end(agent, &inp, &cfg, paths, &file),
    }
}

fn session_start(agent: HookAgent, inp: &HookInput, cfg: &Config, paths: &Paths, file: &Path) -> HookOutput {
    let mut st = load_state(file);
    if st.baseline.is_empty() || st.cwd != inp.cwd {
        st.cwd = inp.cwd.clone();
        st.started = now_secs();
        st.baseline = top_level(&inp.cwd);
        save_state(file, &st);
    }
    if !cfg.hooks.share_rules {
        return HookOutput::default();
    }
    let already = crate::rules::targets(paths)
        .iter()
        .any(|t| t.agent == agent.kind() && t.installed);
    if already {
        return HookOutput::default();
    }
    let context = format!(
        "{}\n\nThe `mom` command is available at {}: `mom mine` lists what you left running, `mom stop mine` stops it, `mom tidy <folder>` lists clutter to fix.",
        crate::rules::current(paths).trim(),
        mom_command(paths)
    );
    let out = json!({
        "hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": context,
        }
    });
    HookOutput { stdout: out.to_string(), exit_code: 0 }
}

/// The live agent process that ran this hook (mom's nearest ancestor of that agent).
fn calling_agent_pid(table: &crate::procs::ProcTable, kind: AgentKind) -> Option<u32> {
    table.ancestors(table.self_pid).into_iter().find(|a| {
        table
            .get(*a)
            .and_then(crate::agents::agent_process)
            .map(|(k, ap)| k == kind && ap == AgentProc::Session)
            .unwrap_or(false)
    })
}

/// Groups that belong to this session (or the agent process that ran the hook).
pub fn session_groups<'a>(groups: &'a [Group], kind: AgentKind, session: &str, agent_pid: Option<u32>) -> Vec<&'a Group> {
    groups
        .iter()
        .filter(|g| {
            !g.kept
                && g.owner.agent == Some(kind)
                && !matches!(g.status, Status::Helper | Status::Yours)
                && ((!session.is_empty() && g.owner.session.as_deref() == Some(session))
                    || (agent_pid.is_some() && g.owner.agent_pid == agent_pid))
        })
        .collect()
}

fn clutter_reason(cwd: &str, name: &str) -> Option<String> {
    let p = Path::new(cwd).join(name);
    let is_dir = p.is_dir();
    let lower = name.to_ascii_lowercase();
    let ext = crate::tidy::ext_of(name);
    if name.starts_with('.') || (!is_dir && crate::tidy::is_project_file(name)) {
        return None;
    }
    if crate::tidy::has_date(name) {
        return Some(if is_dir {
            "a folder named by date at the project root. Reports go in Documents/, real work in a folder named for what it is".into()
        } else {
            "a dated file at the project root. Reports go in Documents/".into()
        });
    }
    if !is_dir && crate::tidy::DOC_EXT.contains(&ext.as_str()) && crate::tidy::REPORT_WORDS.iter().any(|w| lower.contains(w)) {
        return Some("a report at the project root. Reports go in Documents/".into());
    }
    let sprawl = regex::Regex::new(r"(?i)[-_ ](v\d+|final|trial|new|old|copy|backup|bak|draft|wip|test\d*|tmp|temp)$").ok()?;
    let stem = if is_dir { name } else { name.rsplit_once('.').map(|(a, _)| a).unwrap_or(name) };
    if sprawl.is_match(stem) {
        return Some("another copy of something. One thing, one folder".into());
    }
    if ["tmp", "temp", "scratch", "out", "output"].contains(&lower.as_str()) || ext == "tmp" || ext == "log" {
        return Some("temp output at the project root. Delete it when you're done with it".into());
    }
    None
}

fn stop(agent: HookAgent, inp: &HookInput, cfg: &Config, paths: &Paths, file: &Path) -> HookOutput {
    if inp.stop_hook_active || (!cfg.hooks.nag_processes && !cfg.hooks.nag_clutter) {
        return HookOutput::default();
    }
    let mut st = load_state(file);
    let mut keys: Vec<String> = Vec::new();
    let mut running: Vec<String> = Vec::new();
    if cfg.hooks.nag_processes {
        let table = crate::procs::Sampler::new().sample();
        let listeners = crate::ports::listeners();
        let analysis = scan::analyze(&table, &listeners, cfg);
        let me = calling_agent_pid(&table, agent.kind());
        for g in session_groups(&analysis.groups, agent.kind(), &inp.session_id, me) {
            if g.age < cfg.hooks.min_age_secs {
                continue;
            }
            let key = format!("proc:{}", g.id);
            if st.nagged.contains(&key) {
                continue;
            }
            keys.push(key);
            let ports = g.ports.iter().map(|l| l.url()).collect::<Vec<_>>().join(", ");
            let place = g.cwd.as_deref().map(|c| format!(" in {c}")).unwrap_or_default();
            running.push(format!(
                "  • {} ({}){} — PID {}, started {} ago{}",
                g.name,
                g.category.label(),
                if ports.is_empty() { String::new() } else { format!(" on {ports}") },
                g.root_pid,
                human_age(g.age),
                place
            ));
        }
    }
    let mut mess: Vec<String> = Vec::new();
    if cfg.hooks.nag_clutter && !inp.cwd.is_empty() && !st.baseline.is_empty() && st.cwd == inp.cwd {
        let mut shots = 0usize;
        for name in top_level(&inp.cwd) {
            if st.baseline.contains(&name) {
                continue;
            }
            let key = format!("mess:{name}");
            if st.nagged.contains(&key) {
                continue;
            }
            if crate::tidy::IMG_EXT.contains(&crate::tidy::ext_of(&name).as_str()) {
                shots += 1;
                keys.push(key);
                continue;
            }
            if let Some(reason) = clutter_reason(&inp.cwd, &name) {
                keys.push(key);
                let suffix = if Path::new(&inp.cwd).join(&name).is_dir() { "/" } else { "" };
                mess.push(format!("  • {name}{suffix} — {reason}"));
            }
        }
        if shots >= 3 {
            mess.push(format!("  • {shots} new images at the project root — delete screenshots once they've done their job"));
        }
    }
    if running.is_empty() && mess.is_empty() {
        if !keys.is_empty() {
            st.nagged.extend(keys);
            save_state(file, &st);
        }
        return HookOutput::default();
    }
    st.nagged.extend(keys);
    if st.cwd.is_empty() {
        st.cwd = inp.cwd.clone();
    }
    save_state(file, &st);

    let mom = mom_command(paths);
    let mut reason = String::from("Mom here. Before you call it done:\n");
    if !running.is_empty() {
        running.truncate(10);
        reason.push_str("\nStill running from this session:\n");
        reason.push_str(&running.join("\n"));
        reason.push_str(&format!(
            "\nStop whatever the user doesn't need anymore ({mom} stop mine, or stop those PIDs and their children). \
             If the user wants something kept running, leave it and tell them what it is, where it is and how to stop it.\n"
        ));
    }
    if !mess.is_empty() {
        mess.truncate(10);
        reason.push_str(&format!("\nNew in {} and not put away yet:\n", inp.cwd));
        reason.push_str(&mess.join("\n"));
        reason.push_str("\nPut it where it belongs (and fix anything that points at the old path), or delete it if it has no further use.\n");
    }
    reason.push_str("\nThen finish up. I'll only mention each thing once.");
    let out = json!({ "decision": "block", "reason": reason });
    HookOutput { stdout: out.to_string(), exit_code: 0 }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LeftoverNote {
    pub at: u64,
    pub agent: AgentKind,
    pub session: String,
    pub cwd: String,
    pub things: Vec<String>,
    pub stopped: bool,
}

fn session_end(agent: HookAgent, inp: &HookInput, cfg: &Config, paths: &Paths, file: &Path) -> HookOutput {
    let table = crate::procs::Sampler::new().sample();
    let listeners = crate::ports::listeners();
    let analysis = scan::analyze(&table, &listeners, cfg);
    let me = calling_agent_pid(&table, agent.kind());
    let mine = session_groups(&analysis.groups, agent.kind(), &inp.session_id, me);
    if !mine.is_empty() {
        let stopped = cfg.hooks.stop_on_session_end;
        if stopped {
            crate::kill::stop_groups(&table, &mine);
        }
        let note = LeftoverNote {
            at: now_secs(),
            agent: agent.kind(),
            session: inp.session_id.clone(),
            cwd: inp.cwd.clone(),
            things: mine
                .iter()
                .map(|g| {
                    let ports = g.ports.iter().map(|l| format!(":{}", l.port)).collect::<Vec<_>>().join(" ");
                    format!("{} {}", g.name, ports).trim().to_string()
                })
                .collect(),
            stopped,
        };
        let log = paths.state_dir().join("leftovers.jsonl");
        if let Ok(line) = serde_json::to_string(&note) {
            let _ = std::fs::create_dir_all(paths.state_dir());
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log) {
                let _ = writeln!(f, "{line}");
            }
        }
    }
    // The session is over; so is its bookkeeping.
    let _ = std::fs::remove_file(file);
    HookOutput::default()
}

/// Notes written by SessionEnd hooks, newest last.
pub fn leftover_notes(paths: &Paths, since: u64) -> Vec<LeftoverNote> {
    std::fs::read_to_string(paths.state_dir().join("leftovers.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<LeftoverNote>(l).ok())
        .filter(|n| n.at >= since)
        .collect()
}

// ------------------------------------------------------------------ installers

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Integration {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub available: bool,
    pub installed: bool,
    pub path: String,
}

const HOOK_EVENTS: &[(&str, &str, u64)] = &[
    ("SessionStart", "session-start", 10),
    ("Stop", "stop", 30),
    ("SessionEnd", "session-end", 3),
];

fn hook_command(exe: &Path, agent: HookAgent, sub: &str) -> String {
    format!("\"{}\" hook {} {}", exe.to_string_lossy().replace('\\', "/"), agent.slug(), sub)
}

fn is_our_hook(cmd: &str, agent: HookAgent) -> bool {
    let c = cmd.to_ascii_lowercase();
    (c.contains("mom.exe\"") || c.contains("mom\"") || c.starts_with("mom ")) && c.contains(&format!("hook {} ", agent.slug()))
}

fn hooks_file(paths: &Paths, agent: HookAgent) -> PathBuf {
    match agent {
        HookAgent::Claude => paths.claude_dir().join("settings.json"),
        HookAgent::Codex => paths.codex_dir().join("hooks.json"),
    }
}

fn read_json(file: &Path) -> anyhow::Result<Value> {
    if !file.exists() {
        return Ok(json!({}));
    }
    let text = std::fs::read_to_string(file)?;
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    // Refuse to touch a file we can't parse rather than clobber someone's settings.
    serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("{} isn't valid JSON ({e}); not touching it", file.display()))
}

fn strip_ours(hooks: &mut serde_json::Map<String, Value>, agent: HookAgent) {
    for (_, groups) in hooks.iter_mut() {
        let Some(arr) = groups.as_array_mut() else { continue };
        for g in arr.iter_mut() {
            if let Some(inner) = g.get_mut("hooks").and_then(|h| h.as_array_mut()) {
                inner.retain(|h| !h.get("command").and_then(|c| c.as_str()).map(|c| is_our_hook(c, agent)).unwrap_or(false));
            }
        }
        arr.retain(|g| g.get("hooks").and_then(|h| h.as_array()).map(|h| !h.is_empty()).unwrap_or(true));
    }
    hooks.retain(|_, v| v.as_array().map(|a| !a.is_empty()).unwrap_or(true));
}

fn hooks_installed(paths: &Paths, agent: HookAgent) -> bool {
    let Ok(v) = read_json(&hooks_file(paths, agent)) else { return false };
    v.get("hooks")
        .and_then(|h| h.as_object())
        .map(|h| {
            h.values().flat_map(|g| g.as_array().cloned().unwrap_or_default()).any(|g| {
                g.get("hooks")
                    .and_then(|x| x.as_array())
                    .map(|inner| inner.iter().any(|h| h.get("command").and_then(|c| c.as_str()).map(|c| is_our_hook(c, agent)).unwrap_or(false)))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn backup(paths: &Paths, file: &Path, tag: &str) {
    if file.exists() {
        let dir = paths.backups_dir();
        if std::fs::create_dir_all(&dir).is_ok() {
            let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let _ = std::fs::copy(file, dir.join(format!("{tag}-{name}-{}.bak", now_secs())));
        }
    }
}

/// Copies the mom CLI to ~/.momputer/bin so hooks keep working when the app moves or updates.
pub fn install_cli(paths: &Paths, from: &Path) -> anyhow::Result<PathBuf> {
    let to = paths.installed_cli();
    if from == to {
        return Ok(to);
    }
    std::fs::create_dir_all(paths.bin_dir())?;
    if let Err(e) = std::fs::copy(from, &to) {
        // A hook may be running the old copy right now; keep using it.
        if !to.exists() {
            return Err(e.into());
        }
    }
    Ok(to)
}

pub fn install_hooks(paths: &Paths, agent: HookAgent, mom_exe: &Path) -> anyhow::Result<String> {
    let file = hooks_file(paths, agent);
    let mut v = read_json(&file)?;
    if !v.is_object() {
        anyhow::bail!("{} isn't a JSON object; not touching it", file.display());
    }
    backup(paths, &file, &format!("{}-hooks", agent.slug()));
    let root = v.as_object_mut().unwrap();
    if agent == HookAgent::Codex && !root.contains_key("description") {
        root.insert("description".into(), json!("Lifecycle hooks. momputer's entries let mom check for leftovers."));
    }
    let hooks = root.entry("hooks").or_insert_with(|| json!({}));
    if !hooks.is_object() {
        anyhow::bail!("\"hooks\" in {} isn't an object; not touching it", file.display());
    }
    let map = hooks.as_object_mut().unwrap();
    strip_ours(map, agent);
    for (event, sub, timeout) in HOOK_EVENTS {
        let cmd = hook_command(mom_exe, agent, sub);
        let mut handler = json!({ "type": "command", "command": cmd, "timeout": timeout });
        if agent == HookAgent::Codex {
            handler["commandWindows"] = json!(cmd);
            if *event == "Stop" {
                handler["statusMessage"] = json!("Mom is checking your room");
            }
        } else if *event == "SessionEnd" {
            handler["timeout"] = json!(15);
        }
        let groups = map.entry(event.to_string()).or_insert_with(|| json!([]));
        if let Some(arr) = groups.as_array_mut() {
            arr.push(json!({ "hooks": [handler] }));
        }
    }
    let text = serde_json::to_string_pretty(&v)?;
    crate::util::write_atomic(&file, &format!("{text}\n"))?;
    Ok(match agent {
        HookAgent::Claude => format!("Mom now checks in with Claude Code ({}). New sessions pick it up.", file.display()),
        HookAgent::Codex => format!(
            "Mom's hooks are in {}. Codex asks you to trust new hooks: open Codex CLI and run /hooks once.",
            file.display()
        ),
    })
}

pub fn uninstall_hooks(paths: &Paths, agent: HookAgent) -> anyhow::Result<String> {
    let file = hooks_file(paths, agent);
    if !file.exists() || !hooks_installed(paths, agent) {
        return Ok("Mom's hooks weren't installed.".into());
    }
    let mut v = read_json(&file)?;
    backup(paths, &file, &format!("{}-hooks", agent.slug()));
    if let Some(map) = v.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        strip_ours(map, agent);
    }
    if v.get("hooks").and_then(|h| h.as_object()).map(|h| h.is_empty()).unwrap_or(false) {
        v.as_object_mut().unwrap().remove("hooks");
    }
    let text = serde_json::to_string_pretty(&v)?;
    crate::util::write_atomic(&file, &format!("{text}\n"))?;
    Ok(format!("Took mom's hooks out of {}.", file.display()))
}

fn skill_dirs(paths: &Paths) -> [(AgentKind, PathBuf); 2] {
    [
        (AgentKind::ClaudeCode, paths.claude_dir().join("skills").join("mom")),
        (AgentKind::Codex, paths.agents_dir().join("skills").join("mom")),
    ]
}

fn skill_installed(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("SKILL.md")).map(|t| t.contains("name: mom")).unwrap_or(false)
}

pub fn install_skill(paths: &Paths, agent: AgentKind) -> anyhow::Result<String> {
    let (_, dir) = skill_dirs(paths)
        .into_iter()
        .find(|(a, _)| *a == agent)
        .ok_or_else(|| anyhow::anyhow!("no skill folder for {}", agent.label()))?;
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("SKILL.md"), crate::rules::SKILL_MD)?;
    std::fs::write(dir.join("house-rules.md"), crate::rules::current(paths))?;
    Ok(format!("Installed the mom skill for {} ({}).", agent.label(), dir.display()))
}

pub fn uninstall_skill(paths: &Paths, agent: AgentKind) -> anyhow::Result<String> {
    let Some((_, dir)) = skill_dirs(paths).into_iter().find(|(a, _)| *a == agent) else {
        return Ok("Nothing to remove.".into());
    };
    if !skill_installed(&dir) {
        return Ok("The mom skill wasn't installed there.".into());
    }
    std::fs::remove_dir_all(&dir)?;
    Ok(format!("Removed the mom skill from {}.", dir.display()))
}

pub fn integrations(paths: &Paths) -> Vec<Integration> {
    let claude_ok = paths.claude_dir().is_dir();
    let codex_ok = paths.codex_dir().is_dir();
    let [(_, claude_skill), (_, codex_skill)] = skill_dirs(paths);
    vec![
        Integration {
            id: "claude-hooks".into(),
            label: "Claude Code check-ins".into(),
            detail: "Hooks: house rules at session start, a reminder about leftovers before Claude finishes.".into(),
            available: claude_ok,
            installed: hooks_installed(paths, HookAgent::Claude),
            path: hooks_file(paths, HookAgent::Claude).to_string_lossy().to_string(),
        },
        Integration {
            id: "codex-hooks".into(),
            label: "Codex check-ins".into(),
            detail: "Same hooks for Codex. Codex asks you to trust them once (/hooks in the CLI).".into(),
            available: codex_ok,
            installed: hooks_installed(paths, HookAgent::Codex),
            path: hooks_file(paths, HookAgent::Codex).to_string_lossy().to_string(),
        },
        Integration {
            id: "claude-skill".into(),
            label: "mom skill for Claude".into(),
            detail: "Teaches Claude how to clean up after itself and use the mom command.".into(),
            available: claude_ok,
            installed: skill_installed(&claude_skill),
            path: claude_skill.to_string_lossy().to_string(),
        },
        Integration {
            id: "codex-skill".into(),
            label: "mom skill for Codex".into(),
            detail: "The same skill, in ~/.agents/skills where Codex looks.".into(),
            available: codex_ok,
            installed: skill_installed(&codex_skill),
            path: codex_skill.to_string_lossy().to_string(),
        },
    ]
}

pub fn install(paths: &Paths, id: &str, mom_exe: &Path) -> anyhow::Result<String> {
    match id {
        "claude-hooks" => {
            let exe = install_cli(paths, mom_exe)?;
            install_hooks(paths, HookAgent::Claude, &exe)
        }
        "codex-hooks" => {
            let exe = install_cli(paths, mom_exe)?;
            install_hooks(paths, HookAgent::Codex, &exe)
        }
        "claude-skill" => install_skill(paths, AgentKind::ClaudeCode),
        "codex-skill" => install_skill(paths, AgentKind::Codex),
        _ => anyhow::bail!("unknown integration {id}"),
    }
}

pub fn uninstall(paths: &Paths, id: &str) -> anyhow::Result<String> {
    match id {
        "claude-hooks" => uninstall_hooks(paths, HookAgent::Claude),
        "codex-hooks" => uninstall_hooks(paths, HookAgent::Codex),
        "claude-skill" => uninstall_skill(paths, AgentKind::ClaudeCode),
        "codex-skill" => uninstall_skill(paths, AgentKind::Codex),
        _ => anyhow::bail!("unknown integration {id}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_home(tag: &str) -> (PathBuf, Paths) {
        let root = std::env::temp_dir().join(format!("mom-hooks-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::under(&root);
        std::fs::create_dir_all(paths.claude_dir()).unwrap();
        std::fs::create_dir_all(paths.codex_dir()).unwrap();
        (root, paths)
    }

    #[test]
    fn installs_next_to_existing_settings_and_removes_cleanly() {
        let (root, paths) = fake_home("install");
        let existing = r#"{
  "model": "opus",
  "hooks": {
    "Stop": [
      { "hooks": [ { "type": "command", "command": "say done" } ] }
    ]
  }
}"#;
        std::fs::write(paths.claude_dir().join("settings.json"), existing).unwrap();
        let exe = PathBuf::from(r"C:\Users\me\.momputer\bin\mom.exe");
        install_hooks(&paths, HookAgent::Claude, &exe).unwrap();
        install_hooks(&paths, HookAgent::Claude, &exe).unwrap(); // twice: still one copy
        let v: Value = serde_json::from_str(&std::fs::read_to_string(paths.claude_dir().join("settings.json")).unwrap()).unwrap();
        assert_eq!(v["model"], "opus");
        let stop = v["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2, "theirs and ours");
        assert_eq!(stop[0]["hooks"][0]["command"], "say done");
        assert_eq!(stop[1]["hooks"][0]["command"], "\"C:/Users/me/.momputer/bin/mom.exe\" hook claude stop");
        assert!(hooks_installed(&paths, HookAgent::Claude));

        uninstall_hooks(&paths, HookAgent::Claude).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(paths.claude_dir().join("settings.json")).unwrap()).unwrap();
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert!(v["hooks"].get("SessionStart").is_none());
        assert!(!hooks_installed(&paths, HookAgent::Claude));

        // Codex gets its own hooks.json with Windows commands.
        install_hooks(&paths, HookAgent::Codex, &exe).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(paths.codex_dir().join("hooks.json")).unwrap()).unwrap();
        assert!(v["hooks"]["SessionEnd"][0]["hooks"][0]["commandWindows"].as_str().unwrap().ends_with("hook codex session-end"));
        assert_eq!(v["hooks"]["SessionEnd"][0]["hooks"][0]["timeout"], 3);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn refuses_to_clobber_broken_json() {
        let (root, paths) = fake_home("broken");
        std::fs::write(paths.claude_dir().join("settings.json"), "{ not json").unwrap();
        assert!(install_hooks(&paths, HookAgent::Claude, Path::new("mom.exe")).is_err());
        assert_eq!(std::fs::read_to_string(paths.claude_dir().join("settings.json")).unwrap(), "{ not json");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn stop_hook_nags_about_new_clutter_once_and_never_loops() {
        let (root, paths) = fake_home("stop");
        let project = root.join("project");
        std::fs::create_dir_all(project.join("src")).unwrap();
        std::fs::write(project.join("README.md"), "hi").unwrap();
        let input = |active: bool| {
            json!({ "session_id": "s-1", "cwd": project.to_string_lossy(), "stop_hook_active": active, "hook_event_name": "Stop" }).to_string()
        };
        // Processes are covered elsewhere; keep this test about clutter.
        let mut cfg = Config::default();
        cfg.hooks.nag_processes = false;
        cfg.save(&paths).unwrap();

        let start = handle(HookAgent::Claude, HookEvent::SessionStart, &input(false), &paths);
        assert!(start.stdout.contains("additionalContext"), "rules are shared at session start");

        // Nothing new yet: carry on.
        assert_eq!(handle(HookAgent::Claude, HookEvent::Stop, &input(false), &paths), HookOutput::default());

        // The agent makes a mess.
        std::fs::create_dir_all(project.join("podcast-schedule-2026-10-10")).unwrap();
        std::fs::write(project.join("UPLOAD-REPORT.md"), "report").unwrap();
        std::fs::create_dir_all(project.join("src-v2")).unwrap();
        for i in 0..4 {
            std::fs::write(project.join(format!("shot-{i}.png")), "png").unwrap();
        }
        std::fs::write(project.join("helper.py"), "print(1)").unwrap();

        // Already continued once this turn: never block again.
        assert_eq!(handle(HookAgent::Claude, HookEvent::Stop, &input(true), &paths), HookOutput::default());

        let out = handle(HookAgent::Claude, HookEvent::Stop, &input(false), &paths);
        let v: Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(v["decision"], "block");
        let reason = v["reason"].as_str().unwrap();
        assert!(reason.contains("podcast-schedule-2026-10-10/"));
        assert!(reason.contains("UPLOAD-REPORT.md"));
        assert!(reason.contains("src-v2/"));
        assert!(reason.contains("4 new images"));
        assert!(!reason.contains("helper.py"), "not everything new is a mess");

        // Mentioned once. Next time, mom lets it go.
        assert_eq!(handle(HookAgent::Claude, HookEvent::Stop, &input(false), &paths), HookOutput::default());

        // Session over: bookkeeping is cleaned up too.
        handle(HookAgent::Claude, HookEvent::SessionEnd, &input(false), &paths);
        assert!(!state_file(&paths, HookAgent::Claude, "s-1").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn disabled_means_silent() {
        let (root, paths) = fake_home("off");
        let mut cfg = Config::default();
        cfg.hooks.enabled = false;
        cfg.save(&paths).unwrap();
        let out = handle(HookAgent::Codex, HookEvent::SessionStart, r#"{"session_id":"x","cwd":"."}"#, &paths);
        assert_eq!(out, HookOutput::default());
        // Garbage in, silence out.
        cfg.hooks.enabled = true;
        cfg.save(&paths).unwrap();
        assert_eq!(handle(HookAgent::Codex, HookEvent::Stop, "not json at all", &paths).exit_code, 0);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
