//! Puts it all together: which runtimes are running, who started them, and who left them behind.
//!
//! The model:
//! * A **group** is one thing that was started (a dev server, a watcher, a headless browser),
//!   with all its child processes. The root is the topmost runtime-ish process in the tree.
//! * Every group has an **owner**: an agent (from markers or ancestry), You (started from a
//!   terminal or app you're using), or Nobody (its parent is gone and nothing marks it).
//! * The **status** says what mom thinks: left behind (the agent that started it is gone),
//!   orphaned (nobody's watching it), forgotten (running for ages, doing nothing), running,
//!   helper (an MCP server the agent manages), or yours.

use crate::agents::{self, AgentKind, AgentMark, AgentProc, Role};
use crate::classify::{self, Category, Kind};
use crate::config::Config;
use crate::ports::Listener;
use crate::procs::{Proc, ProcTable};
use crate::util::{human_age_long, norm_path};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OwnerKind {
    Agent,
    You,
    Nobody,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Owner {
    pub kind: OwnerKind,
    pub agent: Option<AgentKind>,
    pub label: String,
    pub session: Option<String>,
    pub role: Role,
    /// The agent process that started it is still running.
    pub alive: bool,
    /// The session is still open somewhere, maybe resumed in another window.
    pub session_alive: bool,
    /// PID of the live agent process responsible for it, if any.
    pub agent_pid: Option<u32>,
    /// Worked out from circumstantial evidence (same folder, same time) rather than markers.
    pub guessed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    LeftBehind,
    Orphaned,
    Forgotten,
    Running,
    Helper,
    Yours,
}

impl Status {
    pub fn is_leftover(self) -> bool {
        matches!(self, Status::LeftBehind | Status::Orphaned)
    }
    pub fn label(self) -> &'static str {
        match self {
            Status::LeftBehind => "left behind",
            Status::Orphaned => "orphaned",
            Status::Forgotten => "forgotten?",
            Status::Running => "in use",
            Status::Helper => "helper",
            Status::Yours => "yours",
        }
    }
    fn rank(self) -> u8 {
        match self {
            Status::LeftBehind => 0,
            Status::Orphaned => 1,
            Status::Forgotten => 2,
            Status::Running => 3,
            Status::Yours => 4,
            Status::Helper => 5,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    /// `<root pid>-<root start>`: unique even when Windows reuses the PID.
    pub id: String,
    pub root_pid: u32,
    pub root_start: u64,
    pub name: String,
    pub kind: Kind,
    pub category: Category,
    pub project: Option<String>,
    pub cwd: Option<String>,
    pub command: String,
    pub pids: Vec<u32>,
    pub ports: Vec<Listener>,
    pub mem: u64,
    pub cpu: f32,
    pub started: u64,
    pub age: u64,
    pub owner: Owner,
    pub status: Status,
    /// Other groups running the exact same command in the same folder.
    pub dupes: u32,
    /// Stable identity across restarts (command + folder). Used for "keep".
    pub signature: String,
    pub kept: bool,
    pub why: String,
}

impl Group {
    pub fn is_leftover(&self) -> bool {
        self.status.is_leftover() && !self.kept
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub agent: AgentKind,
    pub label: String,
    pub pid: u32,
    pub started: u64,
    pub age: u64,
    pub project: Option<String>,
    pub project_dir: Option<String>,
    pub entry: Option<String>,
    /// Where it runs: "Claude app", "Codex app", "Terminal", "VS Code"...
    pub host: String,
    /// Hosted by a desktop app that manages its lifetime: mom won't offer to end it.
    pub managed: bool,
    pub mem: u64,
    pub helpers: u32,
    pub helper_mem: u64,
    pub running: u32,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    pub sessions: Vec<Session>,
    pub groups: Vec<Group>,
}

struct Facts {
    mark: Option<AgentMark>,
    agent: Option<(AgentKind, AgentProc)>,
    kind: Option<Kind>,
}

fn is_human_shell_host(name: &str) -> bool {
    matches!(
        name,
        "explorer.exe" | "windowsterminal.exe" | "openconsole.exe" | "conhost.exe" | "wezterm-gui.exe"
            | "alacritty.exe" | "hyper.exe" | "tabby.exe" | "warp.exe" | "mintty.exe" | "conemu64.exe"
    )
}

fn host_label(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::ClaudeCode => "Claude app",
        AgentKind::Codex => "Codex app",
        AgentKind::Cursor => "Cursor",
        AgentKind::Windsurf => "Windsurf",
        AgentKind::Copilot => "VS Code",
        _ => "an app",
    }
}

/// A shell started to run one command (`cmd /c ...`, `bash -c ...`), as opposed to an
/// interactive terminal someone is typing into.
fn is_wrapper_shell(p: &Proc) -> bool {
    p.cmd.iter().skip(1).any(|a| {
        let a = a.to_ascii_lowercase();
        matches!(a.as_str(), "/c" | "-c" | "/k" | "-command" | "-file" | "-encodedcommand" | "-ec")
    })
}

/// Folders that say nothing about what a process is for (its own install folder, Windows).
fn is_junk_dir(dir: &str) -> bool {
    let d = norm_path(dir);
    d.starts_with("c:\\program files")
        || d.starts_with("c:\\windows")
        || d.contains("\\windowsapps\\")
        || d.contains("\\appdata\\local\\microsoft\\")
        || d.len() <= 3
}

/// Claude Code's scratch folders are named after the session: %TEMP%\claude\<project>\<session-id>\...
pub fn claude_scratch_session(text: &str) -> Option<String> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"(?i)[\\/]temp[\\/]claude[\\/][^\\/]+[\\/]([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})").unwrap()
    });
    re.captures(text).map(|c| c[1].to_lowercase())
}

/// Short project name for a folder: the git repo it's in, or the folder under a workspace root.
pub struct ProjectNamer {
    cache: HashMap<String, Option<String>>,
}

impl Default for ProjectNamer {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectNamer {
    pub fn new() -> Self {
        ProjectNamer { cache: HashMap::new() }
    }

    pub fn name(&mut self, dir: &str) -> Option<String> {
        let key = norm_path(dir);
        if let Some(v) = self.cache.get(&key) {
            return v.clone();
        }
        let v = project_name(dir);
        self.cache.insert(key, v.clone());
        v
    }
}

pub fn project_name(dir: &str) -> Option<String> {
    if is_junk_dir(dir) {
        return None;
    }
    let lower = norm_path(dir);
    if [".codex", ".claude", ".agents", ".gemini", ".cursor"]
        .iter()
        .any(|a| lower.contains(&format!("\\{a}\\")) || lower.ends_with(&format!("\\{a}")))
    {
        return None;
    }
    if let Some(home) = dirs::home_dir() {
        if norm_path(dir) == norm_path(&home.to_string_lossy()) {
            return Some("~".into());
        }
    }
    let d = dir.replace('/', "\\");
    let lower = d.to_lowercase();
    // Claude Code scratch space: %TEMP%\claude\C--Workspace-storefront\<session>\scratchpad
    if let Some(i) = lower.find("\\temp\\claude\\") {
        let rest = &d[i + "\\temp\\claude\\".len()..];
        let slug = rest.split('\\').next().unwrap_or("");
        let after_drive = slug.split_once("--").map(|(_, r)| r).unwrap_or(slug);
        let mut name = after_drive.to_string();
        for prefix in ["Workspace-", "workspace-", "Users-"] {
            if let Some(r) = name.strip_prefix(prefix) {
                name = r.to_string();
            }
        }
        return Some(format!("{name} (scratch)"));
    }
    let path = std::path::Path::new(&d);
    let mut git_root: Option<&std::path::Path> = None;
    for anc in path.ancestors() {
        if anc.parent().is_none() {
            break;
        }
        if anc.join(".git").exists() {
            git_root = Some(anc);
        }
    }
    if let Some(root) = git_root {
        return root.file_name().map(|n| n.to_string_lossy().to_string());
    }
    let comps: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().to_string()),
            _ => None,
        })
        .collect();
    const WORKSPACES: &[&str] = &["workspace", "workspaces", "projects", "code", "dev", "src", "repos", "github", "git", "sites"];
    for (i, c) in comps.iter().enumerate() {
        if WORKSPACES.contains(&c.to_lowercase().as_str()) && i + 1 < comps.len() {
            return Some(comps[i + 1].clone());
        }
    }
    comps.last().cloned()
}

fn category_priority(c: Category) -> u8 {
    match c {
        Category::DevServer => 0,
        Category::HeadlessBrowser => 1,
        Category::Database => 2,
        Category::Tunnel => 3,
        Category::Vm => 4,
        Category::MediaJob => 5,
        Category::Server => 6,
        Category::Watcher => 7,
        Category::TestRunner => 8,
        Category::McpServer => 9,
        Category::LogFollower => 10,
        Category::Build => 11,
        Category::Repl => 12,
        Category::Script => 13,
        Category::Sleeper => 14,
        Category::Shell => 15,
    }
}

pub fn analyze(table: &ProcTable, listeners: &HashMap<u32, Vec<Listener>>, cfg: &Config) -> Analysis {
    let now = table.taken_at;
    let me = table.self_and_ancestors();
    let facts: HashMap<u32, Facts> = table
        .procs
        .values()
        .map(|p| {
            (
                p.pid,
                Facts {
                    mark: agents::mark_from_env(&p.env),
                    agent: agents::agent_process(p),
                    kind: classify::kind_of(p),
                },
            )
        })
        .collect();
    let agent_of = |pid: u32| facts.get(&pid).and_then(|f| f.agent);

    // Nearest agent (or agent host) above a process.
    let nearest_agent = |pid: u32| -> Option<(u32, AgentKind, AgentProc)> {
        table
            .ancestors(pid)
            .into_iter()
            .find_map(|a| agent_of(a).map(|(k, ap)| (a, k, ap)))
    };

    let live_session_kinds: HashSet<AgentKind> = facts
        .values()
        .filter_map(|f| f.agent.filter(|(_, ap)| *ap == AgentProc::Session).map(|(k, _)| k))
        .collect();

    // Is the agent process that launched `p` (carrying mark `m`) still running?
    let instance_alive = |p: &Proc, m: &AgentMark| -> (bool, Option<u32>) {
        if let Some(apid) = m.agent_pid {
            let ok = table
                .get(apid)
                .map(|a| a.start <= p.start + 1 && agent_of(apid).map(|(k, _)| k == m.kind).unwrap_or(false))
                .unwrap_or(false);
            return (ok, ok.then_some(apid));
        }
        match nearest_agent(p.pid) {
            Some((apid, k, AgentProc::Sandbox)) if k == m.kind => {
                (live_session_kinds.contains(&k), Some(apid))
            }
            Some((apid, k, _)) if k == m.kind => (true, Some(apid)),
            _ => (false, None),
        }
    };

    // Which sessions are alive, and what we know about each live agent process.
    let mut live_sessions: HashSet<(AgentKind, String)> = HashSet::new();
    let mut session_of_agent: HashMap<u32, String> = HashMap::new();
    let mut project_of_agent: HashMap<u32, String> = HashMap::new();
    let mut entry_of_agent: HashMap<u32, String> = HashMap::new();
    for p in table.procs.values() {
        let Some(m) = facts.get(&p.pid).and_then(|f| f.mark.as_ref()) else { continue };
        let (alive, apid) = instance_alive(p, m);
        if !alive {
            continue;
        }
        if let Some(sid) = &m.session {
            live_sessions.insert((m.kind, sid.clone()));
        }
        if let Some(apid) = apid {
            if let Some(sid) = &m.session {
                session_of_agent.entry(apid).or_insert_with(|| sid.clone());
            }
            if let Some(pr) = &m.project {
                project_of_agent.entry(apid).or_insert_with(|| pr.clone());
            }
            if let Some(e) = &m.entry {
                entry_of_agent.entry(apid).or_insert_with(|| e.clone());
            }
        }
    }

    // Live agent sessions and the folder each works in.
    let home = dirs::home_dir().map(|h| norm_path(&h.to_string_lossy())).unwrap_or_default();
    let live_agents: Vec<(u32, AgentKind, u64, String)> = table
        .procs
        .values()
        .filter_map(|p| match agent_of(p.pid) {
            Some((k, AgentProc::Session)) => {
                let dir = project_of_agent.get(&p.pid).cloned().or_else(|| p.cwd.clone())?;
                let dir = norm_path(&dir);
                // A session rooted at home or a drive root is too broad to vouch for anything.
                if dir == home || dir.len() <= 3 {
                    return None;
                }
                Some((p.pid, k, p.start, dir))
            }
            _ => None,
        })
        .collect();
    let agent_of_session: HashMap<String, u32> = session_of_agent.iter().map(|(pid, s)| (s.clone(), *pid)).collect();

    let is_system = |pid: u32| -> bool {
        for a in table.ancestors(pid) {
            if agent_of(a).is_some() {
                return false;
            }
            let n = table.procs[&a].lname();
            if n == "services.exe" || n == "wininit.exe" || n == "smss.exe" {
                return true;
            }
        }
        false
    };

    let candidates: HashSet<u32> = table
        .procs
        .values()
        .filter(|p| {
            let f = &facts[&p.pid];
            if f.kind.is_none() || f.agent.is_some() || me.contains(&p.pid) || classify::is_critical(p) || p.start == 0 {
                return false;
            }
            if f.mark.is_none() && (classify::is_app_bundled(p) || is_system(p.pid)) {
                return false;
            }
            true
        })
        .map(|p| p.pid)
        .collect();

    let mut roots: BTreeSet<u32> = BTreeSet::new();
    for &pid in &candidates {
        let mut cur = pid;
        while let Some(parent) = table.parent(cur) {
            if candidates.contains(&parent.pid) {
                cur = parent.pid;
            } else {
                break;
            }
        }
        roots.insert(cur);
    }

    let mut namer = ProjectNamer::new();
    let mut groups: Vec<Group> = Vec::new();
    for root in roots {
        let rp = &table.procs[&root];
        let mut members = vec![root];
        members.extend(table.descendants_until(root, |d| agent_of(d.pid).is_some() || me.contains(&d.pid)));

        let mut ports: Vec<Listener> = Vec::new();
        for m in &members {
            for l in listeners.get(m).into_iter().flatten() {
                if !ports.iter().any(|x| x.port == l.port) {
                    ports.push(l.clone());
                }
            }
        }
        ports.sort_by_key(|l| l.port);

        let runtime_members: Vec<u32> = members
            .iter()
            .copied()
            .filter(|m| facts[m].kind.map(|k| k.is_runtime()).unwrap_or(false))
            .collect();
        let has_runtime = !runtime_members.is_empty();
        let only_tools = !has_runtime && members.iter().any(|m| facts[m].kind == Some(Kind::Tool));
        // What was launched: the topmost real runtime (members are breadth-first).
        let headline_pid = runtime_members
            .first()
            .copied()
            .or_else(|| members.iter().copied().find(|m| facts[m].kind == Some(Kind::Tool)))
            .unwrap_or(root);
        let head = &table.procs[&headline_pid];

        let mark = members
            .iter()
            .find_map(|m| facts[m].mark.as_ref().filter(|mk| mk.role == Role::Task).map(|mk| (*m, mk)))
            .or_else(|| members.iter().find_map(|m| facts[m].mark.as_ref().map(|mk| (*m, mk))));
        let role_hint = mark.map(|(_, m)| m.role).unwrap_or(Role::Unknown);

        let category = members
            .iter()
            .filter(|m| facts[*m].kind.is_some())
            .map(|m| {
                let p = &table.procs[m];
                let own_ports = listeners.get(m).map(|v| !v.is_empty()).unwrap_or(false);
                classify::category_of(p, own_ports || (*m == headline_pid && !ports.is_empty()), role_hint)
            })
            .min_by_key(|c| category_priority(*c))
            .unwrap_or(Category::Script);

        let orphan = table.is_orphan(root);
        let ancestors = table.ancestors(root);
        let nearest = ancestors.iter().find_map(|a| agent_of(*a).map(|(k, ap)| (*a, k, ap)));
        let cwd = [headline_pid, root]
            .into_iter()
            .chain(members.iter().copied())
            .find_map(|m| table.procs[&m].cwd.clone().filter(|c| !is_junk_dir(c)));

        let owner = if let Some((carrier, m)) = mark {
            let (alive, apid) = instance_alive(&table.procs[&carrier], m);
            let session_alive = match &m.session {
                Some(s) => live_sessions.contains(&(m.kind, s.clone())),
                None => alive,
            };
            Owner {
                kind: OwnerKind::Agent,
                agent: Some(m.kind),
                label: m.kind.label().to_string(),
                session: m.session.clone(),
                role: if m.role == Role::Unknown && category == Category::McpServer { Role::Harness } else { m.role },
                alive,
                session_alive,
                agent_pid: apid,
                guessed: false,
            }
        } else if let Some((apid, k, ap)) = nearest.filter(|(_, _, ap)| *ap != AgentProc::Host) {
            let alive = ap != AgentProc::Sandbox || live_session_kinds.contains(&k);
            let direct_child = ancestors.first() == Some(&apid);
            let root_is_shell = facts[&root].kind == Some(Kind::Shell);
            // Plugin and MCP servers the agent runs for itself, often through a `cmd /c` wrapper.
            let machinery = members.iter().any(|m| {
                let c = table.procs[m].cmdline().to_ascii_lowercase().replace('/', "\\");
                classify::mentions_mcp(&c) || c.contains(r"\.codex\plugins\") || c.contains(r"\.claude\plugins\")
            });
            Owner {
                kind: OwnerKind::Agent,
                agent: Some(k),
                label: k.label().to_string(),
                session: session_of_agent.get(&apid).cloned(),
                // Started by the agent's machinery itself rather than through its shell tool.
                role: if (direct_child && (!root_is_shell || machinery)) || category == Category::McpServer { Role::Harness } else { Role::Task },
                alive,
                session_alive: alive,
                agent_pid: alive.then_some(apid),
                guessed: false,
            }
        } else if let Some((apid, k, _)) = nearest.filter(|(apid, k, ap)| {
            // The Claude and Codex desktop apps launch preview servers and their own MCP
            // servers directly. Those are the app's, not yours. (Shells under them are your
            // terminal tabs, and IDEs' children are yours.)
            *ap == AgentProc::Host
                && matches!(k, AgentKind::ClaudeCode | AgentKind::Codex)
                && ancestors.first() == Some(apid)
                && (facts[&root].kind != Some(Kind::Shell) || is_wrapper_shell(&table.procs[&root]))
        }) {
            Owner {
                kind: OwnerKind::Agent,
                agent: Some(k),
                label: host_label(k).to_string(),
                session: None,
                role: if category == Category::McpServer { Role::Harness } else { Role::Task },
                alive: true,
                session_alive: true,
                agent_pid: Some(apid),
                guessed: false,
            }
        } else if !orphan
            && (nearest.is_some() || ancestors.iter().any(|a| is_human_shell_host(&table.procs[a].lname())))
        {
            let via = nearest.map(|(_, k, _)| host_label(k)).unwrap_or("a terminal");
            Owner {
                kind: OwnerKind::You,
                agent: None,
                label: format!("You ({via})"),
                session: None,
                role: Role::Unknown,
                alive: true,
                session_alive: true,
                agent_pid: None,
                guessed: false,
            }
        } else if orphan {
            // No markers (Git Bash's sleep/tail can't carry them) and the parent is gone.
            // Look for circumstantial evidence before calling it nobody's.
            let fingerprint = members.iter().find_map(|m| {
                let p = &table.procs[m];
                claude_scratch_session(&p.cmdline()).or_else(|| p.cwd.as_deref().and_then(claude_scratch_session))
            });
            let adopted = cwd.as_deref().map(norm_path).and_then(|c| {
                live_agents
                    .iter()
                    .filter(|(_, _, start, dir)| *start <= rp.start + 1 && (c == *dir || c.starts_with(&format!("{dir}\\"))))
                    .max_by_key(|(_, _, start, _)| *start)
                    .cloned()
            });
            if let Some(sid) = fingerprint {
                let apid = agent_of_session.get(&sid).copied();
                let alive = live_sessions.contains(&(AgentKind::ClaudeCode, sid.clone()))
                    && apid.map(|a| table.procs[&a].start <= rp.start + 1).unwrap_or(false);
                Owner {
                    kind: OwnerKind::Agent,
                    agent: Some(AgentKind::ClaudeCode),
                    label: AgentKind::ClaudeCode.label().to_string(),
                    session: Some(sid),
                    role: Role::Task,
                    alive,
                    session_alive: alive,
                    agent_pid: if alive { apid } else { None },
                    guessed: true,
                }
            } else if let Some((apid, k, _, _)) = adopted {
                Owner {
                    kind: OwnerKind::Agent,
                    agent: Some(k),
                    label: k.label().to_string(),
                    session: session_of_agent.get(&apid).cloned(),
                    role: Role::Task,
                    alive: true,
                    session_alive: true,
                    agent_pid: Some(apid),
                    guessed: true,
                }
            } else {
                if !cfg.leftovers.include_unknown_orphans {
                    continue;
                }
                Owner {
                    kind: OwnerKind::Nobody,
                    agent: None,
                    label: "Nobody".into(),
                    session: None,
                    role: Role::Unknown,
                    alive: false,
                    session_alive: false,
                    agent_pid: None,
                    guessed: false,
                }
            }
        } else {
            // Started by some other app (a launcher, a game client). Not mom's business.
            continue;
        };

        let age = now.saturating_sub(rp.start);
        let mem: u64 = members.iter().map(|m| table.procs[m].mem).sum();
        let cpu: f32 = members.iter().map(|m| table.procs[m].cpu).sum();
        let forgotten_after = (cfg.leftovers.forgotten_after_hours * 3600.0) as u64;
        let status = match owner.kind {
            OwnerKind::Agent if owner.alive => {
                if owner.role == Role::Harness {
                    Status::Helper
                } else if forgotten_after > 0 && age > forgotten_after && cpu < 0.5 {
                    Status::Forgotten
                } else {
                    Status::Running
                }
            }
            OwnerKind::Agent => Status::LeftBehind,
            OwnerKind::You => Status::Yours,
            OwnerKind::Nobody => {
                if cfg.leftovers.include_unknown_orphans {
                    Status::Orphaned
                } else {
                    continue;
                }
            }
        };

        // Skip the noise: quick commands, idle shells, the agent's own plumbing in flight.
        let noteworthy = !ports.is_empty()
            || (has_runtime && age >= 10)
            || (only_tools && age >= if status.is_leftover() { 120 } else { 600 })
            || (!has_runtime && !only_tools && status.is_leftover() && age >= 600);
        if !noteworthy {
            continue;
        }

        let project_dir = mark.and_then(|(_, m)| m.project.clone()).or_else(|| cwd.clone());
        let project = project_dir.as_deref().and_then(|d| namer.name(d));
        let command = head.cmdline();
        let signature = format!(
            "{}|{}",
            command.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" "),
            cwd.as_deref().map(norm_path).unwrap_or_default()
        );
        let kept = cfg.kept.contains(&signature)
            || members.iter().any(|m| {
                let p = &table.procs[m];
                cfg.is_protected(&p.cmdline()) || p.env("MOMPUTER_KEEP") == Some("1")
            });

        let mut g = Group {
            id: format!("{}-{}", root, rp.start),
            root_pid: root,
            root_start: rp.start,
            name: classify::display_name(head),
            kind: facts[&headline_pid].kind.unwrap_or(Kind::Tool),
            category,
            project,
            cwd,
            command,
            pids: members.clone(),
            ports,
            mem,
            cpu,
            started: rp.start,
            age,
            owner,
            status,
            dupes: 0,
            signature,
            kept,
            why: String::new(),
        };
        g.why = why(&g);
        groups.push(g);
    }

    // Same command, same folder, more than once: somebody kept starting it.
    // (Helpers don't count: every session legitimately runs its own MCP servers.)
    let mut counts: HashMap<String, u32> = HashMap::new();
    for g in groups.iter().filter(|g| g.status != Status::Helper) {
        *counts.entry(g.signature.clone()).or_default() += 1;
    }
    for g in groups.iter_mut().filter(|g| g.status != Status::Helper) {
        g.dupes = counts[&g.signature].saturating_sub(1);
        if g.dupes > 0 {
            g.why = format!("{} Started {} times — same command, same folder.", g.why, g.dupes + 1);
        }
    }
    groups.sort_by(|a, b| {
        (a.kept, a.status.rank())
            .cmp(&(b.kept, b.status.rank()))
            .then(b.mem.cmp(&a.mem))
    });

    // Sessions: one per live agent session process (nested processes of the same agent fold in).
    let mut sessions: Vec<Session> = Vec::new();
    for p in table.procs.values() {
        let Some((kind, AgentProc::Session)) = agent_of(p.pid) else { continue };
        // Helpers inside another session of the same agent fold into it (Codex starts nested
        // app-servers from its REPL, Claude starts sub-agents).
        if table
            .ancestors(p.pid)
            .into_iter()
            .any(|a| matches!(agent_of(a), Some((k, AgentProc::Session)) if k == kind))
        {
            continue;
        }
        // Codex's own machinery (exec server, code-mode host, computer-use bridge) isn't a session.
        if kind == AgentKind::Codex && (p.lname() != "codex.exe" || p.cmd.iter().any(|a| a == "exec-server")) {
            continue;
        }
        let host_agent = table
            .ancestors(p.pid)
            .into_iter()
            .find_map(|a| agent_of(a).filter(|(_, ap)| *ap == AgentProc::Host).map(|(k, _)| k));
        let host = match host_agent {
            Some(k) => host_label(k).to_string(),
            None => {
                let terminal = table.ancestors(p.pid).iter().any(|a| {
                    let n = table.procs[a].lname();
                    is_human_shell_host(&n) || classify::kind_of(&table.procs[a]) == Some(Kind::Shell)
                });
                if terminal { "Terminal".to_string() } else { String::new() }
            }
        };
        let mine: Vec<&Group> = groups.iter().filter(|g| g.owner.agent_pid == Some(p.pid)).collect();
        let helpers: Vec<&&Group> = mine.iter().filter(|g| g.status == Status::Helper).collect();
        // One Codex app-server hosts every thread; its own folder says nothing. Threads come
        // from Codex's records instead (see codex.rs).
        let hosts_threads = kind == AgentKind::Codex && p.cmd.iter().any(|a| a == "app-server");
        let project_dir = if hosts_threads { None } else { project_of_agent.get(&p.pid).cloned().or_else(|| p.cwd.clone()) };
        sessions.push(Session {
            id: session_of_agent.get(&p.pid).cloned().unwrap_or_else(|| format!("pid-{}", p.pid)),
            agent: kind,
            label: if hosts_threads && host_agent.is_some() { "Codex app".to_string() } else { kind.label().to_string() },
            pid: p.pid,
            started: p.start,
            age: now.saturating_sub(p.start),
            project: project_dir.as_deref().and_then(|d| namer.name(d)),
            project_dir,
            entry: entry_of_agent.get(&p.pid).cloned(),
            managed: host_agent.is_some(),
            host,
            mem: p.mem,
            helpers: helpers.len() as u32,
            helper_mem: helpers.iter().map(|g| g.mem).sum(),
            running: mine.iter().filter(|g| matches!(g.status, Status::Running | Status::Forgotten)).count() as u32,
        });
    }
    sessions.sort_by(|a, b| a.agent.cmp(&b.agent).then(b.started.cmp(&a.started)));

    Analysis { sessions, groups }
}

/// Mom's one-line explanation for a group.
pub fn why(g: &Group) -> String {
    let age = human_age_long(g.age);
    let place = g.project.as_deref().map(|p| format!(" in {p}")).unwrap_or_default();
    let nick = g.owner.agent.map(|a| a.nickname()).unwrap_or("Someone");
    match g.status {
        Status::LeftBehind if g.owner.session_alive => format!(
            "{nick} started this {age} ago{place} from a window that's gone. The session was reopened since, so check before stopping."
        ),
        Status::LeftBehind => format!("{nick} left this running {age} ago{place}. That session is over."),
        Status::Orphaned => format!("Nobody's watching this one. Whoever started it{place} is gone ({age} ago)."),
        Status::Forgotten => format!("{nick} started this {age} ago{place} and it's just sitting there. Still need it?"),
        Status::Running => format!("{nick} is still using this{place}."),
        Status::Helper => format!("{nick}'s helper. It goes away when the session closes."),
        Status::Yours => "You started this. I won't touch it unless you ask.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procs::{fake, ProcTable};

    fn with_env<K: AsRef<str>, V: AsRef<str>>(mut p: Proc, pairs: &[(K, V)]) -> Proc {
        for (k, v) in pairs {
            p.env.insert(k.as_ref().to_string(), v.as_ref().to_string());
        }
        p
    }

    fn claude_task(session: &str, claude_pid: u32) -> Vec<(String, String)> {
        vec![
            ("AI_AGENT".into(), "claude-code_2-1-295_agent".into()),
            ("CLAUDECODE".into(), "1".into()),
            ("CLAUDE_CODE_SESSION_ID".into(), session.to_string()),
            ("CLAUDE_PID".into(), claude_pid.to_string()),
        ]
    }

    /// A machine with: a live Claude session running vite (in use), an MCP helper, a dead
    /// session's bun watcher (left behind), an unmarked orphan node server, a user's own
    /// terminal server, Adobe's bundled node, and mom herself.
    fn machine() -> ProcTable {
        let now = crate::util::now_secs();
        let t0 = now - 7 * 86_400 - 3600;
        let live = claude_task("live-session", 100);
        let dead = claude_task("dead-session", 555);
        let harness = vec![
            ("AI_AGENT", "claude-code_2-1-295_harness"),
            ("CLAUDECODE", "1"),
            ("CLAUDE_CODE_SESSION_ID", "live-session"),
            ("CLAUDE_PROJECT_DIR", r"C:\Workspace\storefront"),
        ];
        let mut claude = fake(100, 50, "claude.exe", "claude.exe --print", t0);
        claude.exe = Some(r"C:\Users\me\.local\bin\claude.exe".into());
        let mut adobe = fake(900, 1, "node.exe", "node main.js", t0);
        adobe.exe = Some(r"C:\Program Files\Adobe\CCX\libs\node.exe".into());
        let mut user_node = fake(820, 810, "node.exe", "node server.js", t0 + 10);
        user_node.cwd = Some(r"C:\Workspace\side".into());
        let mut vite = with_env(fake(120, 110, "node.exe", r"node C:\Workspace\storefront\node_modules\vite\bin\vite.js", t0 + 60), &live);
        vite.cwd = Some(r"C:\Workspace\storefront".into());
        let list = vec![
            fake(1, 0, "explorer.exe", "explorer.exe", t0 - 100),
            fake(50, 1, "powershell.exe", "powershell", t0 - 10),
            claude,
            with_env(fake(110, 100, "bash.exe", "bash -c x", t0 + 50), &live),
            vite,
            with_env(fake(130, 100, "node.exe", "node C:/Users/me/.claude/mcp-servers/imagegen/index.js", t0 + 5), &harness),
            // Dead session: sh.exe's parent (pid 444) is gone.
            fake(300, 444, "sh.exe", "sh /c/npm/bun --watch server/index.ts", t0 + 100),
            with_env(fake(310, 300, "bun.exe", "bun --watch server/index.ts", t0 + 101), &dead),
            with_env(fake(311, 300, "bun.exe", "bun --watch server/index.ts", t0 + 101), &dead),
            fake(400, 4444, "node.exe", "node orphan-server.js", t0 + 200),
            fake(810, 1, "powershell.exe", "powershell", t0),
            user_node,
            adobe,
            // mom, run by the live agent's bash
            fake(999, 110, "mom.exe", "mom ps", now),
        ];
        ProcTable::from_procs(list, 999)
    }

    #[test]
    fn sorts_the_world_into_the_right_piles() {
        let t = machine();
        let mut listeners = HashMap::new();
        listeners.insert(120, vec![Listener { port: 5173, addr: "127.0.0.1".into(), public: false }]);
        let a = analyze(&t, &listeners, &Config::default());
        let by_root: HashMap<u32, &Group> = a.groups.iter().map(|g| (g.root_pid, g)).collect();

        let vite = by_root[&120];
        assert_eq!(vite.status, Status::Forgotten, "7 days old, idle, live session: forgotten");
        assert_eq!(vite.name, "vite");
        assert_eq!(vite.ports[0].port, 5173);
        assert_eq!(vite.category, Category::DevServer);
        assert_eq!(vite.owner.session.as_deref(), Some("live-session"));

        let helper = by_root[&130];
        assert_eq!(helper.status, Status::Helper);
        assert_eq!(helper.project.as_deref(), Some("storefront"));

        let left = by_root[&300];
        assert_eq!(left.status, Status::LeftBehind);
        assert_eq!(left.owner.agent, Some(AgentKind::ClaudeCode));
        assert_eq!(left.pids.len(), 3);
        assert!(left.why.contains("Claude left this running 7 days ago"), "{}", left.why);

        assert_eq!(by_root[&400].status, Status::Orphaned);
        let yours = a.groups.iter().find(|g| g.pids.contains(&820)).unwrap();
        assert_eq!(yours.status, Status::Yours);
        assert_eq!(yours.name, "server.js");
        assert!(!by_root.contains_key(&900), "Adobe's node is none of our business");
        assert!(!by_root.contains_key(&110), "the shell running mom is never a group");
        assert!(!a.groups.iter().any(|g| g.pids.contains(&999)), "mom never lists herself");

        assert_eq!(a.sessions.len(), 1);
        let s = &a.sessions[0];
        assert_eq!(s.id, "live-session");
        assert_eq!(s.helpers, 1);
        assert_eq!(s.host, "Terminal");
        assert_eq!(s.project.as_deref(), Some("storefront"));

        // Leftovers are listed first.
        assert!(a.groups[0].status.is_leftover());
    }

    #[test]
    fn duplicates_are_called_out() {
        let now = crate::util::now_secs();
        let mut list = vec![];
        for i in 0..3u32 {
            let mut p = fake(500 + i, 4000 + i, "bun.exe", "bun --no-env-file scripts/preview-setup.ts", now - 3600);
            p.cwd = Some(r"C:\Workspace\storefront".into());
            list.push(p);
        }
        let t = ProcTable::from_procs(list, 1);
        let a = analyze(&t, &HashMap::new(), &Config::default());
        assert_eq!(a.groups.len(), 3);
        assert!(a.groups.iter().all(|g| g.dupes == 2));
        assert!(a.groups[0].why.contains("Started 3 times"));
    }

    #[test]
    fn kept_things_are_not_leftovers() {
        let t = machine();
        let mut cfg = Config::default();
        cfg.protect.push("orphan-server".into());
        let a = analyze(&t, &HashMap::new(), &cfg);
        let g = a.groups.iter().find(|g| g.root_pid == 400).unwrap();
        assert!(g.kept && !g.is_leftover());
    }

    #[test]
    fn codex_machinery_is_a_helper_but_its_shell_work_is_not() {
        let now = crate::util::now_secs();
        let t0 = now - 3600;
        let mut codex = fake(10, 1, "codex.exe", "codex.exe app-server --analytics-default-enabled", t0);
        codex.exe = Some(r"C:\Users\me\AppData\Local\OpenAI\Codex\bin\x\codex.exe".into());
        let mut plugin = fake(20, 10, "cmd.exe", r"cmd.exe /d /s /c call ./scripts/launch_codex_app_tools_mcp.cmd ./serve", t0 + 1);
        plugin.cwd = Some(r"C:\Users\me\.codex\plugins\cache\openai-bundled\codex-app-tools\0.1.6".into());
        let plugin_node = fake(21, 20, "node.exe", r"node C:\Users\me\.codex\plugins\cache\x\server.mjs", t0 + 2);
        let mut shell = fake(30, 10, "powershell.exe", "powershell.exe -Command npm run dev", t0 + 60);
        shell.cwd = Some(r"Z:\Workspace\storefront".into());
        let mut vite = fake(31, 30, "node.exe", r"node Z:\Workspace\storefront\node_modules\vite\bin\vite.js", t0 + 61);
        vite.cwd = Some(r"Z:\Workspace\storefront".into());
        let t = ProcTable::from_procs(
            vec![fake(1, 0, "explorer.exe", "explorer.exe", t0 - 9), codex, plugin, plugin_node, shell, vite],
            999,
        );
        let a = analyze(&t, &HashMap::new(), &Config::default());
        let by_root: HashMap<u32, &Group> = a.groups.iter().map(|g| (g.root_pid, g)).collect();
        assert_eq!(by_root[&20].status, Status::Helper);
        assert_eq!(by_root[&20].project, None, "Codex's own folders are nobody's project");
        assert_eq!(by_root[&30].status, Status::Running);
        assert_eq!(by_root[&30].owner.agent, Some(AgentKind::Codex));
        assert_eq!(by_root[&30].project.as_deref(), Some("storefront"));
    }

    #[test]
    fn project_names() {
        assert_eq!(
            project_name(r"C:\Users\me\AppData\Local\Temp\claude\C--Workspace-storefront\0191f20b\scratchpad").as_deref(),
            Some("storefront (scratch)")
        );
        assert_eq!(project_name(r"Z:\Workspace\atlas\apps\web").as_deref(), Some("atlas"));
        assert_eq!(project_name(r"Z:\Elsewhere\thing").as_deref(), Some("thing"));
    }
}
