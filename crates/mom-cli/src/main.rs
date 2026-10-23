//! `mom`: momputer from the command line. For you, your scripts, and your agents.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use mom_core::agents::{AgentKind, AgentProc};
use mom_core::scan::{Group, Status};
use mom_core::util::{ellipsize, human_age, human_bytes, plural};
use mom_core::{hooks, kill, rules, tidy, trash, winsettings, Config, Paths, Snapshot};
use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "mom",
    version,
    about = "momputer: a mom for you, your computer and your AIs.\nShows what your AI agents left running, cleans up after them, and looks after the PC.",
    after_help = "Run `mom` with no arguments for a quick look."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Everything running that mom keeps an eye on.
    Ps {
        /// Include agents' helpers (MCP servers) and things you started yourself.
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    /// Only what was left behind.
    Leftovers {
        #[arg(long)]
        json: bool,
    },
    /// What *your* session left running (run this from inside an agent).
    Mine {
        #[arg(long)]
        json: bool,
    },
    /// Stop things: a PID, :port, group id, `leftovers` or `mine`.
    Stop {
        targets: Vec<String>,
        /// Don't ask.
        #[arg(short, long)]
        yes: bool,
        /// Show what would be stopped.
        #[arg(long)]
        dry_run: bool,
    },
    /// Agent sessions that are running right now.
    Sessions {
        #[arg(long)]
        json: bool,
    },
    /// Docker containers, WSL and VMs.
    Machines {
        #[arg(long)]
        json: bool,
    },
    /// Temp folders and caches: how big, and what can go. `mom trash <id>` shows what's inside.
    Trash {
        #[command(subcommand)]
        action: Option<TrashCmd>,
        /// A spot to look inside (ids from `mom trash`).
        id: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Copies of the same file and big files nobody has opened in a year, in your own folders.
    Files {
        #[arg(long)]
        json: bool,
    },
    /// Is there a newer momputer? Asks GitHub (the one thing mom asks the internet).
    Update {
        #[arg(long)]
        json: bool,
    },
    /// The skills that come with momputer: what each does, what to say, who has it, and whether
    /// their copy is the newest.
    Skills {
        #[arg(long)]
        json: bool,
    },
    /// How each agent is set up, and whether it holds together: MCP servers, hooks, settings and
    /// skills. Only looks: no server is started and no hook is run. `mom agents read <id>` shows
    /// one of the files, with keys blanked.
    Agents {
        #[command(subcommand)]
        action: Option<AgentsCmd>,
        #[arg(long, global = true)]
        json: bool,
        /// First ask Claude Code to try each of its MCP servers (`claude mcp list`: it starts each
        /// one for a moment), so the list says which answer and which are waiting for a sign-in.
        #[arg(long)]
        check: bool,
    },
    /// A checkup for the whole machine: disks, memory, restarts, startup apps.
    Health {
        #[arg(long)]
        json: bool,
    },
    /// What's slowing the PC down: apps that lost the graphics card, driver crashes, hardware
    /// errors, overclocking tools, helpers that run all day, and what's busy right now.
    Speed {
        #[arg(long)]
        json: bool,
    },
    /// What the PC is made of, and whether it's protected, up to date and running at its best
    /// (memory at its rated speed, Windows on an SSD), plus registry entries that point at
    /// nothing. Only looks.
    Pc {
        #[arg(long)]
        json: bool,
    },
    /// Your browsers, their profiles and every extension: what each can do, and whether it's used.
    /// Only looks.
    Browsers {
        #[arg(long)]
        json: bool,
    },
    /// Every program running, as one line each (all of Chrome's processes together): CPU,
    /// memory, graphics card and windows. Only looks.
    Programs {
        /// Windows' own processes and services too.
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    /// Windows settings that drifted from the defaults over the years, and leftovers pointing at
    /// things that are gone. `mom settings fix <id>` puts one back.
    Settings {
        #[command(subcommand)]
        action: Option<SettingsCmd>,
        #[arg(long)]
        json: bool,
    },
    /// Grade a messy folder and list what to fix.
    Tidy {
        /// Folder to inspect (default: current folder).
        dir: Option<PathBuf>,
        /// Do what mom suggests (only the actions she selected, or --only).
        #[arg(long)]
        apply: bool,
        /// Action ids to apply (see the report).
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        /// Print a chore list to paste to an agent instead.
        #[arg(long)]
        prompt: bool,
        #[arg(long)]
        json: bool,
    },
    /// Undo the last tidy-up.
    Undo,
    /// The house rules agents should follow.
    Rules {
        #[command(subcommand)]
        action: Option<RulesCmd>,
    },
    /// Install integrations: claude-hooks, codex-hooks, claude-skill, codex-skill, rules, cli, or all.
    Install { what: Vec<String> },
    /// Remove integrations (same names as install).
    Uninstall { what: Vec<String> },
    /// Which integrations are installed.
    Integrations {
        #[arg(long)]
        json: bool,
    },
    /// Hook entry point for agents (reads the event JSON on stdin).
    #[command(hide = true)]
    Hook { agent: String, event: String },
    /// What each agent session did this week, from its own log: files changed, deletions,
    /// installs, pushes, and how its last test or build went, beside what it said. Only looks.
    Homework {
        #[arg(long)]
        json: bool,
    },
    /// Agents tell mom they're waiting for you: Claude Code's hook passes the event on stdin,
    /// Codex's notify program as the last argument.
    #[command(hide = true)]
    Notify {
        #[arg(long)]
        agent: String,
        payload: Option<String>,
    },
    /// Where mom keeps her settings.
    Config,
    /// What mom did: clean-ups, moves, tidy-ups, fixes and programs she stopped, newest first.
    History {
        /// Everything on record, not just the latest 30.
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum AgentsCmd {
    /// Show one of the files `mom agents --json` lists (by its id), with keys blanked.
    Read { id: String },
}

#[derive(Subcommand)]
enum TrashCmd {
    /// Clean one or more spots (ids from `mom trash`), or `all` (every spot that's safe to clean
    /// without looking first).
    Clean {
        ids: Vec<String>,
        #[arg(short, long)]
        yes: bool,
        /// Run by the widget as administrator for the clean-ups that need one: no questions, no
        /// output, nothing written but the deleting. The widget sees what's gone by itself.
        #[arg(long, hide = true)]
        elevated: bool,
        /// What was ticked, as hex-encoded JSON.
        #[arg(long, hide = true)]
        pick_hex: Option<String>,
    },
    /// Measure, as administrator, what only an administrator can see (Windows' memory
    /// snapshots, say), so the widget can show their sizes. Run by the widget when "admin" is
    /// ticked next to Measure; it only measures.
    #[command(hide = true)]
    Look {
        #[arg(long)]
        elevated: bool,
    },
    /// Keep a file or folder from now on: no clean-up takes it. With no path, lists what's kept.
    Keep {
        paths: Vec<String>,
        /// Let these go again (with no path: all of them).
        #[arg(long)]
        release: bool,
    },
}

#[derive(Subcommand)]
enum SettingsCmd {
    /// Put one back the way Windows ships it (ids from `mom settings`).
    Fix {
        id: String,
        #[arg(short, long)]
        yes: bool,
        /// For the ones that need an administrator: run from a terminal opened with "Run as
        /// administrator" (the widget asks Windows for you).
        #[arg(long, hide = true)]
        elevated: bool,
    },
    /// Put back what the last fix of one changed (the ones mom can undo).
    Undo { id: String },
}

#[derive(Subcommand)]
enum RulesCmd {
    /// Print them.
    Show,
    /// Write them into an agent's instructions (claude, codex, gemini).
    Install { agent: String },
    /// Take them out again.
    Remove { agent: String },
    /// Go back to mom's default rules.
    Reset,
}

// ------------------------------------------------------------------ pretty output

struct Style {
    on: bool,
}

impl Style {
    fn detect() -> Style {
        let on = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none() && enable_ansi();
        Style { on }
    }
    fn paint(&self, code: &str, s: &str) -> String {
        if self.on {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
    fn bold(&self, s: &str) -> String {
        self.paint("1", s)
    }
    fn dim(&self, s: &str) -> String {
        self.paint("2", s)
    }
    fn red(&self, s: &str) -> String {
        self.paint("38;5;203", s)
    }
    fn yellow(&self, s: &str) -> String {
        self.paint("38;5;179", s)
    }
    fn green(&self, s: &str) -> String {
        self.paint("38;5;108", s)
    }
    fn pink(&self, s: &str) -> String {
        self.paint("38;5;211", s)
    }
}

#[cfg(windows)]
fn enable_ansi() -> bool {
    use windows_sys::Win32::System::Console::{
        GetConsoleMode, GetStdHandle, SetConsoleMode, ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_OUTPUT_HANDLE,
    };
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0;
        if GetConsoleMode(h, &mut mode) == 0 {
            return false;
        }
        SetConsoleMode(h, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
    }
}

#[cfg(not(windows))]
fn enable_ansi() -> bool {
    true
}

/// Fits `s` in a column of width `w`, always leaving at least one space after it.
fn pad(s: &str, w: usize) -> String {
    let fitted = ellipsize(s, w.saturating_sub(1).max(1));
    let n = fitted.chars().count();
    format!("{fitted}{}", " ".repeat(w.saturating_sub(n).max(1)))
}

fn who(g: &Group) -> String {
    match g.status {
        Status::LeftBehind => format!("{} (gone)", g.owner.agent.map(|a| a.nickname()).unwrap_or("?")),
        Status::Orphaned => "nobody".into(),
        Status::Yours => "you".into(),
        _ => g.owner.agent.map(|a| a.nickname().to_string()).unwrap_or_else(|| g.owner.label.clone()),
    }
}

fn print_groups(st: &Style, title: &str, groups: &[&Group]) {
    if groups.is_empty() {
        return;
    }
    let mem: u64 = groups.iter().map(|g| g.mem).sum();
    println!(
        "\n{}  {}",
        st.bold(title),
        st.dim(&format!("{} thing{} · {}", groups.len(), if groups.len() == 1 { "" } else { "s" }, human_bytes(mem)))
    );
    println!(
        "  {}",
        st.dim(&format!("{}{}{}{}{}{}{}", pad("PID", 8), pad("WHAT", 36), pad("WHERE", 18), pad("PORTS", 13), pad("MEM", 9), pad("AGE", 6), "WHO"))
    );
    for g in groups {
        let ports = g.ports.iter().map(|l| format!(":{}", l.port)).collect::<Vec<_>>().join(" ");
        let mut what = g.name.clone();
        if g.dupes > 0 {
            what = format!("{what} ×{}", g.dupes + 1);
        }
        let row = format!(
            "{}{}{}{}{}{}{}",
            pad(&g.root_pid.to_string(), 8),
            pad(&what, 36),
            pad(g.project.as_deref().unwrap_or("-"), 18),
            pad(if ports.is_empty() { "-" } else { &ports }, 13),
            pad(&human_bytes(g.mem), 9),
            pad(&human_age(g.age), 6),
            who(g)
        );
        let row = match g.status {
            Status::LeftBehind | Status::Orphaned if !g.kept => st.red(&row),
            Status::Forgotten => st.yellow(&row),
            _ => row,
        };
        println!("  {row}");
    }
}

fn print_snapshot(st: &Style, snap: &Snapshot, all: bool) {
    println!("{}  {}", st.pink(&st.bold("mom")), st.bold(&format!("\"{}\"", snap.mood.line)));
    let leftovers: Vec<&Group> = snap.groups.iter().filter(|g| g.is_leftover()).collect();
    let forgotten: Vec<&Group> = snap.groups.iter().filter(|g| g.status == Status::Forgotten && !g.kept).collect();
    let running: Vec<&Group> = snap.groups.iter().filter(|g| g.status == Status::Running || (g.kept && g.status != Status::Yours && g.status != Status::Helper)).collect();
    let helpers: Vec<&Group> = snap.groups.iter().filter(|g| g.status == Status::Helper).collect();
    let yours: Vec<&Group> = snap.groups.iter().filter(|g| g.status == Status::Yours).collect();
    print_groups(st, "LEFT BEHIND", &leftovers);
    print_groups(st, "FORGOTTEN?", &forgotten);
    print_groups(st, "IN USE", &running);
    if all {
        print_groups(st, "HELPERS", &helpers);
        print_groups(st, "YOURS", &yours);
    } else if !helpers.is_empty() || !yours.is_empty() {
        println!(
            "\n  {}",
            st.dim(&format!(
                "+ {} ({}) and {} of yours. `mom ps --all` to see them.",
                plural(helpers.len() as u64, "agent helper"),
                human_bytes(snap.summary.helper_mem),
                yours.len()
            ))
        );
    }
    if let Some(d) = &snap.machines.docker {
        if !d.containers.is_empty() {
            println!("\n{}  {}", st.bold("CONTAINERS"), st.dim(&format!("{} running", d.containers.len())));
            for c in &d.containers {
                println!("  {}{}{}", pad(&c.name, 28), pad(&c.image, 28), st.dim(&c.status));
            }
        }
    }
    if let Some(w) = &snap.machines.wsl {
        let names = w.running.iter().map(|d| d.name.as_str()).collect::<Vec<_>>().join(", ");
        println!("\n{}  {} {}", st.bold("WSL"), if names.is_empty() { "VM up".to_string() } else { names }, st.dim(&format!("· {}", human_bytes(w.vm_mem))));
    }
    if !leftovers.is_empty() {
        println!(
            "\n  {}  {}",
            st.bold("mom stop leftovers"),
            st.dim(&format!("stops all {} ({}).  mom stop <pid|:port> for one.", leftovers.len(), human_bytes(snap.summary.leftover_mem)))
        );
    }
}

// ------------------------------------------------------------------ helpers

fn paths() -> Paths {
    Paths::detect()
}

/// Which agent session is running this command, if any.
fn my_session(table: &mom_core::procs::ProcTable) -> (Option<AgentKind>, String, Option<u32>) {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let agent_pid_of = |kind: AgentKind| {
        table.ancestors(table.self_pid).into_iter().find(|a| {
            table
                .get(*a)
                .and_then(mom_core::agents::agent_process)
                .map(|(k, ap)| k == kind && ap == AgentProc::Session)
                .unwrap_or(false)
        })
    };
    if let Some(s) = env("CLAUDE_CODE_SESSION_ID") {
        return (Some(AgentKind::ClaudeCode), s, agent_pid_of(AgentKind::ClaudeCode));
    }
    if let Some(s) = env("CODEX_THREAD_ID") {
        return (Some(AgentKind::Codex), s, agent_pid_of(AgentKind::Codex));
    }
    for a in table.ancestors(table.self_pid) {
        if let Some((k, AgentProc::Session)) = table.get(a).and_then(mom_core::agents::agent_process) {
            return (Some(k), String::new(), Some(a));
        }
    }
    (None, String::new(), None)
}

fn confirm(question: &str) -> bool {
    if !std::io::stdin().is_terminal() {
        return false;
    }
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Each session's receipt, a few lines each, the ones worth a look marked.
fn print_homework(st: &Style, list: &[mom_core::homework::Receipt]) {
    if list.is_empty() {
        println!("No agent sessions in the last {} days.", mom_core::homework::DAYS);
        return;
    }
    let now = mom_core::util::now_secs();
    for r in list {
        let name = format!("{} · {} · {}", r.agent, r.project.as_deref().unwrap_or("?"), if r.title.is_empty() { "Untitled" } else { &r.title });
        println!("{}", if r.flags.is_empty() { st.bold(&name) } else { st.red(&name) });
        let mut facts = vec![format!("{} ago", human_age(now.saturating_sub(r.ended))), plural(r.commands as u64, "command")];
        if r.changed_count > 0 {
            facts.push(format!("{} changed", plural(r.changed_count as u64, "file")));
        }
        if r.deleted_count > 0 {
            facts.push(format!("{} deleted", r.deleted_count));
        }
        if !r.installs.is_empty() {
            facts.push(plural(r.installs.len() as u64, "install"));
        }
        if !r.pushes.is_empty() {
            facts.push(if r.pushes.len() == 1 { "1 push".into() } else { format!("{} pushes", r.pushes.len()) });
        }
        if let Some(c) = &r.last_check {
            facts.push(format!("last test {}", if c.passed { "passed" } else { "failed" }));
        }
        println!("  {}", st.dim(&facts.join(" · ")));
        for f in &r.flags {
            println!("  {}", st.red(&format!("! {f}")));
        }
    }
}

fn print_json<T: serde::Serialize>(v: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

fn parse_agent(s: &str) -> Result<AgentKind> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "claude" | "claude-code" => AgentKind::ClaudeCode,
        "codex" => AgentKind::Codex,
        "gemini" => AgentKind::Gemini,
        other => bail!("mom doesn't know an agent called {other:?} (try claude, codex or gemini)"),
    })
}

// ------------------------------------------------------------------ commands

fn main() {
    let cli = Cli::parse();
    let code = match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("mom: {e:#}");
            1
        }
    };
    std::process::exit(code);
}

/// What mom will do as administrator: look, and clean the spots that need one (trash::clean
/// refuses the rest). Everything else acts on files anything running as you can change (settings,
/// the journal, agents' folders), and those mustn't get to steer an administrator.
fn allowed_elevated(cmd: &Option<Cmd>) -> bool {
    match cmd {
        None
        | Some(Cmd::Ps { .. })
        | Some(Cmd::Leftovers { .. })
        | Some(Cmd::Mine { .. })
        | Some(Cmd::Sessions { .. })
        | Some(Cmd::Machines { .. })
        | Some(Cmd::Trash { .. })
        | Some(Cmd::Health { .. })
        | Some(Cmd::Update { .. })
        | Some(Cmd::Skills { .. })
        | Some(Cmd::Agents { check: false, .. })
        | Some(Cmd::Speed { .. })
        | Some(Cmd::Programs { .. })
        | Some(Cmd::Pc { .. })
        | Some(Cmd::Browsers { .. })
        | Some(Cmd::Files { .. })
        | Some(Cmd::Homework { .. })
        | Some(Cmd::Integrations { .. })
        | Some(Cmd::Config)
        | Some(Cmd::History { .. })
        | Some(Cmd::Settings { action: None, .. }) => true,
        Some(Cmd::Settings { action: Some(SettingsCmd::Fix { id, elevated: true, .. }), .. }) => winsettings::is_admin_fix(id),
        Some(Cmd::Settings { action: Some(SettingsCmd::Fix { .. } | SettingsCmd::Undo { .. }), .. }) => false,
        Some(Cmd::Agents { check: true, .. }) => false, // it starts programs: not as administrator
        Some(Cmd::Stop { dry_run, .. }) => *dry_run,
        Some(Cmd::Tidy { apply, .. }) => !*apply,
        Some(Cmd::Rules { action }) => matches!(action, None | Some(RulesCmd::Show)),
        Some(Cmd::Undo) | Some(Cmd::Install { .. }) | Some(Cmd::Uninstall { .. }) | Some(Cmd::Hook { .. }) | Some(Cmd::Notify { .. }) => false,
    }
}

fn run(cli: Cli) -> Result<i32> {
    if mom_core::elevate::is_elevated() && !allowed_elevated(&cli.cmd) {
        if matches!(cli.cmd, Some(Cmd::Hook { .. } | Cmd::Notify { .. })) {
            return Ok(0); // an agent running as administrator: stay out of the way
        }
        bail!("mom only looks when run as administrator (and cleans what needs one). Run this from an ordinary terminal.");
    }
    let st = Style::detect();
    let paths = paths();
    let cfg = Config::load(&paths);
    match cli.cmd {
        None => {
            let (_, snap) = mom_core::snapshot_now(&cfg, false);
            print_snapshot(&st, &snap, false);
        }
        Some(Cmd::Ps { all, json }) => {
            let (_, snap) = mom_core::snapshot_now(&cfg, true);
            if json {
                print_json(&snap)?;
            } else {
                print_snapshot(&st, &snap, all);
            }
        }
        Some(Cmd::Leftovers { json }) => {
            let (_, snap) = mom_core::snapshot_now(&cfg, false);
            let left: Vec<&Group> = snap.groups.iter().filter(|g| g.is_leftover()).collect();
            if json {
                print_json(&left)?;
            } else if left.is_empty() {
                println!("{}", st.green("Nothing left behind. Good kids."));
            } else {
                print_groups(&st, "LEFT BEHIND", &left);
                for g in &left {
                    println!("  {} {}", st.dim(&format!("{:>7}", g.root_pid)), st.dim(&g.why));
                }
            }
        }
        Some(Cmd::Mine { json }) => {
            let (table, snap) = mom_core::snapshot_now(&cfg, false);
            let (kind, session, agent_pid) = my_session(&table);
            let Some(kind) = kind else {
                bail!("you don't look like an agent session to me. Try `mom ps`.");
            };
            let mine = hooks::session_groups(&snap.groups, kind, &session, agent_pid);
            if json {
                print_json(&mine)?;
            } else if mine.is_empty() {
                println!("{}", st.green("You've got nothing running. Proud of you."));
            } else {
                print_groups(&st, &format!("STILL RUNNING FROM YOUR {} SESSION", kind.nickname().to_uppercase()), &mine);
                println!("\n  {}", st.dim("`mom stop mine` stops all of it. Leave anything the user asked to keep running, and tell them about it."));
            }
        }
        Some(Cmd::Stop { targets, yes, dry_run }) => return stop(&st, &paths, &cfg, targets, yes, dry_run),
        Some(Cmd::Sessions { json }) => {
            let (_, snap) = mom_core::snapshot_now(&cfg, false);
            if json {
                print_json(&serde_json::json!({ "sessions": snap.sessions, "codex": snap.codex }))?;
                return Ok(0);
            }
            if snap.sessions.is_empty() {
                println!("No agents running.");
            } else {
                println!("{}", st.dim(&format!("  {}{}{}{}{}{}", pad("PID", 8), pad("AGENT", 14), pad("PROJECT", 22), pad("HOST", 13), pad("AGE", 6), "HELPERS")));
                for s in &snap.sessions {
                    println!(
                        "  {}{}{}{}{}{}",
                        pad(&s.pid.to_string(), 8),
                        pad(&s.label, 14),
                        pad(s.project.as_deref().unwrap_or("-"), 22),
                        pad(if s.host.is_empty() { "-" } else { &s.host }, 13),
                        pad(&human_age(s.age), 6),
                        if s.helpers > 0 { format!("{} ({})", s.helpers, human_bytes(s.helper_mem)) } else { "-".into() }
                    );
                }
            }
            let codex = &snap.codex;
            if !codex.threads.is_empty() {
                println!("
{}", st.bold("CODEX THREADS"));
                for t in &codex.threads {
                    let when = if t.working { st.green("working now") } else { format!("active {} ago", human_age(mom_core::util::now_secs().saturating_sub(t.updated))) };
                    let name = if t.name.is_empty() { "(untitled)" } else { t.name.as_str() };
                    println!("  {}{}{}", pad(name, 36), pad(t.project.as_deref().unwrap_or("-"), 16), when);
                    if let Some(a) = &t.automation {
                        let next = a.next_run.map(|n| format!(", next around {}", mom_core::codex::clock(n))).unwrap_or_default();
                        println!("    {}", st.dim(&format!("scheduled: {} ({}{next})", a.name, a.schedule)));
                    }
                    if t.subagents > 0 {
                        println!("    {}", st.dim(&format!("{} busy", plural(t.subagents as u64, "sub-agent"))));
                    }
                    println!("    {}", st.dim(&t.link));
                }
            }
            if !codex.automations.is_empty() {
                println!("
{}", st.bold("CODEX AUTOMATIONS"));
                for a in &codex.automations {
                    let state = if a.active { st.green("active") } else { st.dim(&a.status.to_lowercase()) };
                    let next = a.next_run.map(|n| format!("next around {}", mom_core::codex::clock(n))).unwrap_or_default();
                    println!("  {}{}{}{}", pad(&a.name, 36), pad(&state, if st.on { 21 } else { 10 }), pad(&a.schedule, 32), next);
                }
            }
            if let Some(note) = &codex.note {
                println!("
  {}", st.dim(note));
            }
        }
        Some(Cmd::Machines { json }) => {
            let table = mom_core::procs::Sampler::new().sample();
            let m = mom_core::containers::machines(&table);
            if json {
                print_json(&m)?;
            } else {
                match &m.docker {
                    Some(d) if d.running => {
                        println!("{}  {} running", st.bold("Docker"), d.containers.len());
                        for c in &d.containers {
                            println!("  {}{}{}{}", pad(&c.id, 14), pad(&c.name, 26), pad(&c.image, 26), st.dim(&c.status));
                        }
                    }
                    Some(d) => println!("{}  not answering ({})", st.bold("Docker"), d.error.clone().unwrap_or_default()),
                    None => println!("{}  not running", st.bold("Docker")),
                }
                match &m.wsl {
                    Some(w) => println!(
                        "{}  {} · VM using {}",
                        st.bold("WSL"),
                        if w.running.is_empty() { "no distros".into() } else { w.running.iter().map(|d| d.name.clone()).collect::<Vec<_>>().join(", ") },
                        human_bytes(w.vm_mem)
                    ),
                    None => println!("{}  not running", st.bold("WSL")),
                }
                for v in &m.vms {
                    println!("{}  {} ({}, up {})", st.bold("Hyper-V"), v.name, human_bytes(v.mem), v.uptime);
                }
            }
        }
        Some(Cmd::Trash { action, id, json }) => {
            let table = mom_core::procs::Sampler::new().sample();
            let ctx = trash::TrashCtx::new(&paths, &cfg, &table);
            match (action, id) {
                (None, None) => {
                    if !json {
                        eprintln!("{}", st.dim("Measuring… (big folders take a moment)"));
                    }
                    let spots = trash::measure_all(|| trash::TrashCtx::new(&paths, &cfg, &table));
                    if json {
                        print_json(&spots)?;
                    } else {
                        print_trash(&st, &spots);
                    }
                }
                (None, Some(id)) => {
                    let Some(spot) = trash::measure(&id, &ctx) else {
                        bail!("mom doesn't know a spot called {id:?}. See `mom trash` for ids.");
                    };
                    if json {
                        print_json(&spot)?;
                    } else {
                        print_spot(&st, &spot);
                    }
                }
                (Some(TrashCmd::Look { elevated }), _) => {
                    if !elevated || !mom_core::elevate::is_elevated() {
                        bail!("This is the widget's: it asks Windows to run it as administrator when \"admin\" is ticked next to Measure.");
                    }
                    trash::look_as_admin(&ctx).map_err(anyhow::Error::msg)?;
                    return Ok(0);
                }
                (Some(TrashCmd::Keep { paths: which, release }), _) => {
                    let mut cfg = cfg.clone();
                    let before = cfg.trash.keep.clone();
                    if release && which.is_empty() {
                        cfg.trash.keep.clear();
                    }
                    for p in &which {
                        // The whole path, so it means the same thing wherever mom runs from.
                        let full = std::path::absolute(p).map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|_| p.clone());
                        if let Err(why) = trash::set_keep(&mut cfg.trash, &full, !release) {
                            bail!("{full}: {why}");
                        }
                    }
                    if cfg.trash.keep != before {
                        cfg.save(&paths)?;
                    }
                    if cfg.trash.keep.is_empty() {
                        println!("Nothing is kept. `mom trash keep <path>` keeps a file or folder out of every clean-up.");
                    } else {
                        println!("{}", st.bold("Kept out of every clean-up:"));
                        for k in &cfg.trash.keep {
                            println!("  {k}");
                        }
                    }
                }
                (Some(TrashCmd::Clean { ids, yes, elevated, pick_hex }), _) => {
                    if elevated {
                        // The widget asked, as administrator: one spot that needs one, what was ticked.
                        let [id] = ids.as_slice() else { bail!("--elevated takes one spot") };
                        if !trash::needs_admin(id) {
                            bail!("{id} doesn't need an administrator");
                        }
                        let pick: Option<trash::Pick> = match pick_hex {
                            Some(h) => Some(serde_json::from_str(&mom_core::elevate::from_hex(&h).context("bad --pick-hex")?)?),
                            None => None,
                        };
                        trash::clean(id, &ctx, pick.as_ref());
                        return Ok(0);
                    }
                    let ids: Vec<String> = if ids.iter().any(|i| i == "all") {
                        trash::spot_ids().into_iter().filter(|i| trash::cleans_without_asking(i)).map(String::from).collect()
                    } else {
                        ids
                    };
                    if ids.is_empty() {
                        bail!("which spot? See `mom trash` for ids.");
                    }
                    let spots: Vec<trash::Spot> = ids
                        .iter()
                        .filter_map(|i| {
                            let s = trash::measure(i, &ctx);
                            if s.is_none() {
                                eprintln!("mom: no spot called {i:?}; skipping it.");
                            }
                            s
                        })
                        .filter(|s| s.available && (s.reclaimable_items > 0 || s.action == "tool"))
                        .collect();
                    if spots.is_empty() {
                        println!("{}", st.green("Nothing to clean. Already spotless."));
                        return Ok(0);
                    }
                    for s in &spots {
                        println!("{}  {}", st.bold(&s.label), st.dim(&trash::describe(s)));
                        println!("{}", wrap(&s.effect, 96, "  "));
                        if let Some(n) = &s.note {
                            println!("{}", st.yellow(&wrap(n, 96, "  ")));
                        }
                    }
                    if let Some(s) = spots.iter().find(|s| s.admin) {
                        if !mom_core::elevate::is_elevated() {
                            bail!("{} needs an administrator: run this from a terminal opened with \"Run as administrator\".", s.id);
                        }
                    }
                    if !yes && !confirm("Go ahead?") {
                        if !std::io::stdin().is_terminal() {
                            eprintln!("mom: not a terminal, so pass --yes to confirm.");
                            return Ok(1);
                        }
                        println!("Fine. It stays.");
                        return Ok(0);
                    }
                    let mut freed = 0;
                    for s in &spots {
                        let since = mom_core::util::now_secs();
                        let r = trash::clean(&s.id, &ctx, None);
                        mom_core::history::cleaned(&paths, &r, None, false, since);
                        freed += r.freed;
                        println!("  {}{}", pad(&s.id, 20), r.message);
                    }
                    if spots.len() > 1 {
                        println!("{}", st.green(&mom_core::voice::cleaned(freed)));
                    }
                }
            }
        }
        Some(Cmd::Files { json }) => {
            let live = !json && std::io::stderr().is_terminal();
            if !json {
                eprintln!("{}", st.dim("Looking through your folders… (comparing big files takes a moment)"));
            }
            let opts = mom_core::files::ScanOpts::for_paths(&paths);
            let r = mom_core::files::scan(&paths, &opts, &std::sync::atomic::AtomicBool::new(false), &|p| {
                if live {
                    eprint!("\r  {} {} files, {}   ", p.stage, mom_core::util::human_count(p.files), human_bytes(p.bytes));
                }
            });
            if live {
                eprint!("\r{}\r", " ".repeat(60));
            }
            // As administrator mom only looks: the widget's copy of the results is yours to write.
            if !mom_core::elevate::is_elevated() {
                mom_core::files::save(&paths, &r);
            }
            if json {
                print_json(&r)?;
            } else {
                print_files(&st, &r);
            }
        }
        Some(Cmd::Settings { action, json }) => match action {
            None => {
                let found = winsettings::check(&paths);
                if json {
                    print_json(&found)?;
                } else {
                    print_settings(&st, &found);
                }
            }
            Some(SettingsCmd::Fix { id, yes, elevated }) => {
                if elevated {
                    // As administrator: only mom's own list, with her own values (see winsettings).
                    match winsettings::fix_elevated(&id) {
                        Ok(msg) => println!("{msg}"),
                        Err(why) => bail!("{why}"),
                    }
                    return Ok(0);
                }
                let Some(f) = winsettings::check(&paths).into_iter().find(|f| f.id == id) else {
                    println!("{}", st.green("Nothing to fix there: it's already fine."));
                    return Ok(0);
                };
                let Some(fix) = &f.fix else {
                    let how = f.command.as_deref().map(|c| format!(" Run it yourself: {c}")).or_else(|| f.settings.as_deref().map(|s| format!(" Change it here: {s}")));
                    bail!("mom can't change that one herself.{}", how.unwrap_or_default());
                };
                if fix.admin {
                    bail!(
                        "{} needs an administrator. The widget asks Windows for you (Upkeep, Windows), or run `mom settings fix {id} --elevated` from a terminal opened with \"Run as administrator\".",
                        f.title
                    );
                }
                println!("{}  {}", st.bold(&f.title), st.dim(&format!("{} now, {} by default", f.now, f.default)));
                println!("{}", wrap(&f.why, 96, "  "));
                if !yes && !confirm(&format!("{}?", fix.label)) {
                    if !std::io::stdin().is_terminal() {
                        eprintln!("mom: not a terminal, so pass --yes to confirm.");
                        return Ok(1);
                    }
                    println!("Fine. It stays.");
                    return Ok(0);
                }
                match winsettings::fix(&paths, &id) {
                    Ok(msg) => {
                        let line = format!("Put back a Windows setting: {}", f.title.trim_end_matches('.'));
                        if fix.undo {
                            mom_core::history::fixed(&paths, mom_core::history::kind::SETTING, "setting", &id, line);
                        } else {
                            mom_core::history::did(&paths, mom_core::history::kind::SETTING, line);
                        }
                        println!("{}", st.green(&msg))
                    }
                    Err(why) => bail!("{why}"),
                }
            }
            Some(SettingsCmd::Undo { id }) => match winsettings::undo(&paths, &id) {
                Ok(msg) => {
                    mom_core::history::put_back(&paths, mom_core::history::kind::SETTING_UNDO, "setting", Some(&id), msg.clone());
                    println!("{}", st.green(&msg))
                }
                Err(why) => bail!("{why}"),
            },
        },
        Some(Cmd::Update { json }) => {
            let u = mom_core::updates::check(&paths, env!("CARGO_PKG_VERSION"), true);
            if json {
                print_json(&u)?;
            } else if u.newer {
                println!("{}  {}", st.pink(&st.bold("mom")), st.bold(&format!("\"{} is out. You have {}.\"", u.latest.as_deref().unwrap_or_default(), u.current)));
                println!("  Get it: {}", u.get_it);
            } else if let Some(why) = &u.problem {
                println!("{}  {}", st.pink(&st.bold("mom")), st.bold(&format!("\"You have {}. {why}\"", u.current)));
            } else {
                println!("{}  {}", st.pink(&st.bold("mom")), st.bold(&format!("\"You have {}, the newest.\"", u.current)));
            }
        }
        Some(Cmd::Skills { json }) => {
            let list = mom_core::skills::list(&paths);
            if json {
                print_json(&list)?;
            } else {
                println!("{}  {}", st.bold("SKILLS"), st.dim(&format!("{} jobs you can hand your agent", list.len())));
                for s in &list {
                    let who = if s.has.is_empty() { "not installed".to_string() } else { format!("in {}", s.has.join(" and ")) };
                    println!("\n  {}  {}  {}", st.bold(&pad(&s.title, 20)), st.dim(&format!("{:<7}", s.version)), st.dim(&who));
                    println!("  {}", s.what);
                    println!("  {}", st.dim(&format!("Say: \"{}\"", s.prompts.join("\", \""))));
                    for c in s.copies.iter().filter(|c| c.behind) {
                        let have = if c.version.is_empty() { "an older copy".to_string() } else { format!("version {}", c.version) };
                        println!("  {}", st.yellow(&format!("{} has {have}; the newest is {}.", c.who, s.version)));
                    }
                }
                let late: Vec<_> = list.iter().filter(|s| s.behind).cloned().collect();
                for line in mom_core::skills::update_lines(&late) {
                    println!("\n{} {}", st.bold("To catch up, run:"), line);
                }
                if mom_core::skills::catch_up(&late).is_some() {
                    println!("{}", st.dim(&format!("Copied by hand? Replace the folder with the newest from {}", mom_core::skills::PAGE)));
                }
                println!("\n{}", st.dim(&format!("Get them all in Claude Code: {}", mom_core::skills::INSTALL)));
                println!("{}", st.dim(&format!("Anywhere else, copy a folder from {}", mom_core::skills::PAGE)));
            }
        }
        Some(Cmd::Agents { action, json, check }) => {
            if check {
                match mom_core::setup::ask_claude(&paths) {
                    Ok(n) if !json => eprintln!("{}", st.dim(&format!("Claude Code tried {n} of its servers."))),
                    Ok(_) => {}
                    Err(why) => eprintln!("mom: {why}"),
                }
            }
            let agents = mom_core::setup::check(&paths);
            match action {
                Some(AgentsCmd::Read { id }) => {
                    let doc = mom_core::setup::read(&agents, &id).map_err(|e| anyhow::anyhow!(e))?;
                    if json {
                        print_json(&doc)?;
                    } else {
                        println!("{}  {}", st.bold(&doc.title), st.dim(&doc.path));
                        if doc.hidden > 0 {
                            println!("{}", st.dim(&format!("{} blanked: mom doesn't show keys.", if doc.hidden == 1 { "1 key".to_string() } else { format!("{} keys", doc.hidden) })));
                        }
                        println!("\n{}", doc.text);
                        if doc.cut {
                            println!("\n{}", st.dim("(the rest is longer than mom shows)"));
                        }
                    }
                }
                None if json => print_json(&agents)?,
                None => {
                    if agents.is_empty() {
                        println!("No agent has a set-up on this PC yet.");
                    }
                    for a in &agents {
                        let look = if a.problems == 0 { st.green("all in order") } else { st.red(&format!("{} to look at", a.problems)) };
                        println!("{}  {}", st.bold(&a.name.to_uppercase()), look);
                        let light = |level: &str| match level {
                            "warn" => st.red("●"),
                            "note" => st.yellow("●"),
                            "off" | "unknown" => st.dim("○"),
                            _ => st.green("●"),
                        };
                        for f in &a.settings {
                            println!("  {} {}  {}", light(&f.level), f.title, st.dim(&f.detail));
                        }
                        if !a.servers.is_empty() {
                            let unchecked = a.servers.iter().filter(|s| s.level == "unknown").count();
                            let said = match (a.checked, a.can_check) {
                                (Some(at), _) => format!(", as Claude Code found them {} ago", human_age(mom_core::util::now_secs().saturating_sub(at))),
                                (None, true) if unchecked > 0 => ", not checked yet: `mom agents --check` asks Claude Code to try them".to_string(),
                                _ => String::new(),
                            };
                            println!("  {}", st.dim(&format!("MCP servers ({}{said})", a.servers.len())));
                        }
                        for s in &a.servers {
                            println!("  {} {}  {}", light(&s.level), pad(&s.name, 22), st.dim(&format!("{} · {}", s.what, s.from)));
                            if let Some(p) = &s.problem {
                                println!("{}", wrap(p, 92, "      "));
                            }
                        }
                        if !a.hooks.is_empty() {
                            println!("  {}", st.dim(&format!("Hooks ({})", a.hooks.len())));
                        }
                        for h in &a.hooks {
                            let when = if h.matcher.is_empty() { h.event.clone() } else { format!("{} ({})", h.event, h.matcher) };
                            let whose = if h.mine { "mom's check-in".to_string() } else { h.from.clone() };
                            let whose = if h.count > 1 { format!("{whose} · {} times", h.count) } else { whose };
                            println!("  {} {}  {}", light(&h.level), pad(&when, 22), st.dim(&format!("{} · {whose}", h.what)));
                            if let Some(p) = &h.problem {
                                println!("{}", wrap(p, 92, "      "));
                            }
                        }
                        println!("  {}\n", st.dim(&format!("{} skills, {} files to read (`mom agents --json` lists them)", a.skills.len(), a.docs.len())));
                    }
                }
            }
        }
        Some(Cmd::Health { json }) => {
            let table = mom_core::procs::Sampler::new().sample();
            let h = mom_core::health::check(&table, &paths);
            if json {
                print_json(&h)?;
            } else {
                print_health(&st, &h);
            }
        }
        Some(Cmd::Speed { json }) => {
            // Two looks a moment apart, so there's something to measure "busy right now" by.
            let mut sampler = mom_core::procs::Sampler::new();
            sampler.sample();
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let table = sampler.sample();
            let sp = mom_core::speed::check(&table, &paths);
            if json {
                print_json(&sp)?;
            } else {
                print_speed(&st, &sp);
            }
        }
        Some(Cmd::Browsers { json }) => {
            let table = mom_core::procs::Sampler::new().sample();
            let list = mom_core::browsers::list(&table, true);
            if json {
                print_json(&list)?;
            } else {
                for b in &list {
                    let state = if b.open { format!("open for {}, {} processes, {}", human_age(b.open_secs), b.processes, human_bytes(b.mem)) } else { "closed".into() };
                    println!("{}  {}", st.bold(&b.name.to_uppercase()), st.dim(&state));
                    for p in &b.profiles {
                        let on = p.extensions.iter().filter(|e| e.on).count();
                        let used = p.last_used.map(|t| format!("used {} ago", human_age(mom_core::util::now_secs().saturating_sub(t)))).unwrap_or_default();
                        println!("  {}  {}", st.bold(&p.name), st.dim(&format!("{} · {} · {on} of {} extensions on · cache {} · history {}", used, human_bytes(p.size), p.extensions.len(), human_bytes(p.cache), human_bytes(p.history))));
                        for n in &p.notes {
                            println!("    {} {}", st.yellow("~"), n.text);
                        }
                    }
                    println!();
                }
            }
        }
        Some(Cmd::Pc { json }) => {
            let sys = mom_core::system::get(true);
            if json {
                print_json(&sys)?;
            } else {
                println!("{}", st.bold("THIS PC"));
                for (k, v) in &sys.pc {
                    println!("  {}{}", pad(k, 12), v);
                }
                for area in ["Security", "Updates", "Hardware", "Performance", "Registry"] {
                    let list: Vec<_> = sys.checks.iter().filter(|c| c.area == area).collect();
                    if list.is_empty() {
                        continue;
                    }
                    println!("\n{}", st.bold(&area.to_uppercase()));
                    for c in list {
                        let mark = match c.level {
                            "warn" => st.red("!"),
                            "tip" => st.yellow("~"),
                            _ => st.green("✓"),
                        };
                        println!("  {mark} {}  {}", st.bold(&c.title), st.dim(&c.detail));
                    }
                }
            }
        }
        Some(Cmd::Programs { all, json }) => {
            let mut sampler = mom_core::procs::Sampler::new();
            sampler.sample();
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let table = sampler.sample();
            let list: Vec<_> = mom_core::programs::list(&table).into_iter().filter(|p| all || !matches!(p.kind, "windows" | "service")).collect();
            if json {
                print_json(&list)?;
            } else {
                println!("{}", st.bold("PROGRAMS"));
                for p in &list {
                    let gpu = p.gpu.filter(|g| *g >= 1.0).map(|g| format!(" · GPU {g:.0}%")).unwrap_or_default();
                    let win = if p.windows > 0 { format!(" · {} window{}", p.windows, if p.windows == 1 { "" } else { "s" }) } else { String::new() };
                    let inside = p.inside.as_ref().map(|i| format!(" · {i} inside")).unwrap_or_default();
                    println!(
                        "  {}{}{}{}",
                        pad(&format!("{:.1}%", p.cpu), 8),
                        pad(&human_bytes(p.mem), 10),
                        pad(&p.name, 34),
                        st.dim(&format!("{} · {} process{}{gpu}{win}{inside}", p.kind, p.count, if p.count == 1 { "" } else { "es" }))
                    );
                }
            }
        }
        Some(Cmd::Tidy { dir, apply, only, prompt, json }) => {
            let dir = match dir {
                Some(d) => d,
                None => std::env::current_dir()?,
            };
            if tidy::is_moms_room(&dir, &paths) {
                if apply || prompt {
                    bail!("That's mom's room. You don't tidy your mother's room, sweetie.");
                }
                let report = tidy::moms_room(&paths);
                if json {
                    print_json(&report)?;
                } else {
                    print_room(&st, &report);
                }
                return Ok(0);
            }
            let dir = match tidy::check_room(&dir, &paths) {
                Ok(d) => d,
                Err(why) => bail!("{why}"),
            };
            let report = tidy::inspect(&dir, &cfg.tidy)?;
            if prompt {
                println!("{}", tidy::chore_prompt(&report));
                return Ok(0);
            }
            if apply {
                let ids: Vec<String> = if only.is_empty() {
                    report.actions.iter().filter(|a| a.selected).map(|a| a.id.clone()).collect()
                } else {
                    only
                };
                let r = tidy::apply(&report, &ids, &paths.journal_dir());
                mom_core::history::tidied(&paths, &report.name, &r);
                if json {
                    print_json(&r)?;
                } else {
                    println!("{}", st.green(&r.message));
                    for f in &r.failed {
                        println!("  {}", st.red(f));
                    }
                }
                return Ok(if r.failed.is_empty() { 0 } else { 2 });
            }
            if json {
                print_json(&report)?;
            } else {
                print_room(&st, &report);
            }
        }
        Some(Cmd::Undo) => {
            let r = tidy::undo_last(&paths.journal_dir());
            if r.restored > 0 {
                mom_core::history::put_back(&paths, mom_core::history::kind::TIDY_UNDO, "tidy", None, r.message.clone());
            }
            println!("{}", r.message);
            for f in &r.failed {
                println!("  {}", st.red(f));
            }
        }
        Some(Cmd::History { all, json }) => {
            let list = mom_core::history::shown(&paths, if all { usize::MAX } else { 30 });
            if json {
                print_json(&list)?;
            } else if list.is_empty() {
                println!("{}", st.dim("Nothing yet. What mom clears, moves or fixes shows up here."));
            } else {
                let now = mom_core::util::now_secs();
                for e in &list {
                    let when = format!("{} ago", human_age(now.saturating_sub(e.entry.at)));
                    let who = if e.entry.auto { " (Autopilot)" } else { "" };
                    let after = match (e.undone, e.action) {
                        (true, _) => " · put back".to_string(),
                        (false, Some(a)) => format!(" · {a} from the widget"),
                        _ if e.in_bin => " · in the Recycle Bin".to_string(),
                        _ => String::new(),
                    };
                    println!("{}  {}{}{}", st.dim(&pad(&when, 10)), e.entry.text, st.dim(who), st.dim(&after));
                }
            }
        }
        Some(Cmd::Rules { action }) => match action {
            None | Some(RulesCmd::Show) => {
                println!("{}", rules::current(&paths));
                println!("{}", st.dim(&format!("Edit: {} · Installed in:", paths.rules_file().display())));
                for t in rules::targets(&paths) {
                    let state = if t.up_to_date { st.green("yes") } else if t.installed { st.yellow("outdated") } else { st.dim("no") };
                    println!("  {}{}", pad(&t.label, 16), state);
                }
            }
            Some(RulesCmd::Install { agent }) => println!("{}", rules::install(&paths, parse_agent(&agent)?)?),
            Some(RulesCmd::Remove { agent }) => println!("{}", rules::remove(&paths, parse_agent(&agent)?)?),
            Some(RulesCmd::Reset) => {
                rules::reset(&paths)?;
                println!("Back to mom's original rules.");
            }
        },
        Some(Cmd::Install { what }) => return install(&st, &paths, what, true),
        Some(Cmd::Uninstall { what }) => return install(&st, &paths, what, false),
        Some(Cmd::Integrations { json }) => {
            let list = hooks::integrations(&paths);
            if json {
                print_json(&list)?;
            } else {
                for i in &list {
                    let state = if i.installed { st.green("installed") } else if i.available { st.dim("not installed") } else { st.dim("agent not found") };
                    println!("  {}{}{}", pad(&i.id, 15), pad(&state, if st.on { 28 } else { 17 }), st.dim(&i.path));
                }
            }
        }
        Some(Cmd::Hook { agent, event }) => {
            let (Some(agent), Some(event)) = (hooks::HookAgent::parse(&agent), hooks::HookEvent::parse(&event)) else {
                return Ok(0); // unknown hook: stay out of the way
            };
            let mut input = String::new();
            let _ = std::io::stdin().read_to_string(&mut input);
            let out = hooks::handle(agent, event, &input, &paths);
            if !out.stdout.is_empty() {
                println!("{}", out.stdout);
            }
            return Ok(out.exit_code);
        }
        Some(Cmd::Homework { json }) => {
            let list = mom_core::homework::recent(&paths, true);
            if json {
                print_json(&list)?;
            } else {
                print_homework(&st, &list);
            }
        }
        Some(Cmd::Notify { agent, payload }) => {
            // Never in an agent's way: whatever happens, it's told all is well.
            let agent = agent.to_ascii_lowercase();
            let input = match payload {
                Some(p) => p,
                None if agent == "claude" && !std::io::stdin().is_terminal() => {
                    let mut s = String::new();
                    let _ = std::io::stdin().take(512 * 1024).read_to_string(&mut s);
                    s
                }
                None => String::new(),
            };
            let _ = mom_core::waiting::record(&paths, &agent, &input);
            return Ok(0);
        }
        Some(Cmd::Config) => {
            println!("{}", paths.config_file().display());
            println!("{}", st.dim(&format!("house rules: {}", paths.rules_file().display())));
            println!("{}", st.dim(&format!("journal:     {}", paths.journal_dir().display())));
        }
    }
    Ok(0)
}

/// Word-wraps `text` to `width` columns, every line starting with `indent`.
fn wrap(text: &str, width: usize, indent: &str) -> String {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && indent.len() + line.chars().count() + 1 + word.chars().count() > width {
            lines.push(format!("{indent}{line}"));
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(format!("{indent}{line}"));
    }
    lines.join("\n")
}

const GROUPS: &[(&str, &str)] = &[
    ("agents", "AGENTS' LEFTOVERS"),
    ("dev", "DEVELOPER CACHES AND BUILDS"),
    ("windows", "WINDOWS AND BROWSERS"),
    ("personal", "YOUR FILES"),
];

fn print_files(st: &Style, r: &mom_core::files::FilesReport) {
    let big: u64 = r.big.iter().map(|b| b.size).sum();
    let headline = match (r.wasted, big) {
        (0, 0) => "No copies and no big forgotten files. Tidy!".to_string(),
        (w, 0) => format!("Extra copies of your files take up {}.", human_bytes(w)),
        (0, b) => format!("{} in big files nobody has opened in a year.", human_bytes(b)),
        (w, b) => format!("Extra copies take up {}, and big files nobody has opened in a year {}.", human_bytes(w), human_bytes(b)),
    };
    println!("{}  {}", st.pink(&st.bold("mom")), st.bold(&format!("\"{headline}\"")));
    let roots: Vec<String> = r.roots.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).collect();
    println!(
        "{}",
        st.dim(&format!("  {} files ({}) in {}", mom_core::util::human_count(r.scanned_files), human_bytes(r.scanned_bytes), roots.join(", ")))
    );
    const GROUPS: usize = 15;
    if !r.groups.is_empty() {
        println!("\n{}", st.bold("Copies"));
        for g in r.groups.iter().take(GROUPS) {
            println!("  {}{}", pad(&format!("{} × {}", human_bytes(g.size), g.files.len()), 16), st.dim(&g.kind));
            for f in g.files.iter().take(4) {
                let tag = if f.keep { st.green("keep ") } else { st.dim("extra") };
                println!("    {tag} {}{}", pad(&f.folder, 11), f.path.display());
            }
            if g.files.len() > 4 {
                println!("    {}", st.dim(&format!("and {} more copies", g.files.len() - 4)));
            }
        }
        if r.groups.len() > GROUPS {
            println!("  {}", st.dim(&format!("…and {} more groups (`mom files --json` lists them all).", r.groups.len() - GROUPS)));
        }
    }
    if !r.big.is_empty() {
        println!("\n{}", st.bold("Big files nobody has opened in a year"));
        let now = mom_core::util::now_secs();
        for b in r.big.iter().take(20) {
            let (word, last) = if r.access_times { ("opened", b.accessed) } else { ("changed", b.modified) };
            println!(
                "  {}{}{}{}",
                pad(&human_bytes(b.size), 10),
                pad(&b.kind, 11),
                pad(&format!("{word} {} ago", human_age(now.saturating_sub(last))), 18),
                b.path.display()
            );
        }
        if !r.access_times {
            println!("{}", st.dim(&wrap("Windows doesn't record when files are opened here, so this goes by when they last changed.", 96, "  ")));
        }
    }
    if !r.complete {
        println!("\n{}", st.yellow("  Stopped before finishing, so this isn't everything."));
    }
    println!(
        "\n{}",
        st.dim(&wrap(
            "Nothing was touched. In the widget you tick what goes, and it goes to the Recycle Bin; the last copy of anything always stays.",
            96,
            "  "
        ))
    );
}

fn print_trash(st: &Style, spots: &[trash::Spot]) {
    let found: Vec<&trash::Spot> = spots.iter().filter(|s| s.available).collect();
    let total: u64 = found.iter().map(|s| s.reclaimable_bytes).sum();
    let upper = found.iter().any(|s| !s.exact && s.reclaimable_bytes > 0);
    println!(
        "{}  {}",
        st.pink(&st.bold("mom")),
        st.bold(&format!("\"I found {}{} that can go.\"", if upper { "up to " } else { "" }, human_bytes(total)))
    );
    for (group, title) in GROUPS {
        let rows: Vec<&&trash::Spot> = found.iter().filter(|s| s.group == *group).collect();
        if rows.is_empty() {
            continue;
        }
        println!("\n{}", st.bold(title));
        for s in rows {
            let mut line = format!("  {}{}{}", pad(&s.id, 20), pad(&s.label, 24), trash::describe(s));
            if s.reclaimable_items > 0 && s.action != "tool" {
                line.push_str(&format!(" · {} item{}", s.reclaimable_items, if s.reclaimable_items == 1 { "" } else { "s" }));
            }
            let line = if s.reclaimable_bytes > 1024 * 1024 * 1024 { st.yellow(&line) } else { line };
            let flag = match (s.safety.as_str(), s.action.as_str()) {
                ("review", _) => st.dim("  (look first)"),
                (_, "tool") => st.dim("  (runs a tool)"),
                _ => String::new(),
            };
            println!("{line}{flag}");
            if let Some(n) = &s.note {
                println!("  {}{}", " ".repeat(20), st.dim(n));
            }
        }
    }
    println!(
        "\n{}",
        st.dim(&wrap(
            "`mom trash <id>` shows what's inside and exactly what cleaning does. `mom trash clean <id>`, or `all` for everything that's safe to clean without looking.",
            96,
            "  "
        ))
    );
}

fn print_spot(st: &Style, s: &trash::Spot) {
    println!("{}  {}", st.bold(&s.label), st.dim(&trash::describe(s)));
    if let Some(p) = &s.path {
        println!("  {}", st.dim(p));
    }
    println!("\n{}", wrap(&s.what, 96, "  "));
    println!("\n  {}", st.bold("When you clean it"));
    println!("{}", wrap(&s.effect, 96, "  "));
    if let Some(n) = &s.note {
        println!("\n{}", st.yellow(&wrap(n, 96, "  ")));
    }
    let print_items = |title: &str, goes: bool, count: u64, bytes: u64| {
        let items: Vec<&trash::Item> = s.items.iter().filter(|i| i.goes == goes).collect();
        if items.is_empty() {
            return;
        }
        let shown = if (items.len() as u64) < count { format!(", biggest {} shown", items.len()) } else { String::new() };
        println!("\n  {}  {}", st.bold(title), st.dim(&format!("{count} · {}{shown}", human_bytes(bytes))));
        for i in items {
            let size = if i.unmeasured { "unknown".to_string() } else { human_bytes(i.bytes) };
            let line = format!("    {}{}{}", pad(&size, 10), pad(&i.name, 46), pad(i.sub.as_deref().unwrap_or(""), 34));
            println!("{line}{}", st.dim(&i.reason));
        }
    };
    print_items("GOES", true, s.reclaimable_items, s.reclaimable_bytes);
    print_items("STAYS", false, s.kept_items, s.kept_bytes);
    if s.available && (s.reclaimable_items > 0 || s.action == "tool") {
        println!("\n  {}", st.dim(&format!("`mom trash clean {}` to clean it.", s.id)));
    }
}

fn print_health(st: &Style, h: &mom_core::health::Health) {
    let warns = h.checks.iter().filter(|c| c.level == "warn").count();
    let line = match warns {
        0 => "Clean bill of health.".to_string(),
        1 => "One thing needs your attention, sweetie.".to_string(),
        n => format!("{n} things need your attention, sweetie."),
    };
    println!("{}  {}\n", st.pink(&st.bold("mom")), st.bold(&format!("\"{line}\"")));
    for c in &h.checks {
        let mark = match c.level {
            "warn" => st.red("!"),
            "tip" => st.yellow("~"),
            _ => st.green("✓"),
        };
        println!("  {mark} {}  {}", st.bold(&c.title), st.dim(&c.detail));
        if let Some(a) = &c.action {
            println!("      {}", st.dim(&format!("{}: {}", a.label, a.url)));
        }
    }
    println!("\n{}", st.bold("DRIVES"));
    for d in &h.drives {
        let pct = if d.total > 0 { d.total.saturating_sub(d.free) * 100 / d.total } else { 0 };
        let filled = (pct as usize * 20 / 100).min(20);
        let bar = format!("{}{}", "█".repeat(filled), "░".repeat(20 - filled));
        let bar = if pct >= 90 { st.red(&bar) } else { bar };
        let name = format!("{}{}", d.label, if d.system { " (Windows)" } else { "" });
        println!("  {}{}{}  {}", pad(&d.root, 5), pad(&name, 26), bar, st.dim(&format!("{} free of {}", human_bytes(d.free), human_bytes(d.total))));
        if let Some(days) = d.full_in_days {
            println!("       {}", st.yellow(&mom_core::space::full_in_words(&d.root, days)));
        }
    }
    println!(
        "\n{}  {}",
        st.bold("MEMORY"),
        st.dim(&format!("{} of {} used · up {}", human_bytes(h.memory.used), human_bytes(h.memory.total), human_age(h.uptime_secs)))
    );
    for p in &h.top_memory {
        let procs = if p.count == 1 { "1 process".to_string() } else { format!("{} processes", p.count) };
        println!("  {}{}{}", pad(&human_bytes(p.mem), 10), pad(&p.name, 30), st.dim(&procs));
    }
    let on = h.startup.iter().filter(|a| a.enabled).count();
    println!("\n{}  {}", st.bold("STARTS WITH WINDOWS"), st.dim(&format!("{on} on, {} off", h.startup.len() - on)));
    for a in &h.startup {
        let state = if a.enabled { st.green("on ") } else { st.dim("off") };
        println!("  {state} {}{}", pad(&a.name, 30), st.dim(&format!("{} · {}", a.location, ellipsize(&a.command, 60))));
    }
}

fn print_speed(st: &Style, sp: &mom_core::speed::Speed) {
    let warns = sp.findings.iter().filter(|f| f.level == "warn").count();
    let line = match warns {
        0 => "Nothing's dragging its feet. Good.".to_string(),
        1 => "One thing is slowing you down, sweetie.".to_string(),
        n => format!("{n} things are slowing you down, sweetie."),
    };
    println!("{}  {}
", st.pink(&st.bold("mom")), st.bold(&format!("\"{line}\"")));
    for f in &sp.findings {
        let mark = match f.level {
            "warn" => st.red("!"),
            "tip" => st.yellow("~"),
            _ => st.green("✓"),
        };
        println!("  {mark} {}  {}", st.bold(&f.title), st.dim(&f.detail));
        if let Some(x) = &f.fix {
            println!("      {}", st.dim(&format!("{}{}", x.label, x.hint.as_ref().map(|h| format!(": {h}")).unwrap_or_default())));
        }
    }
    if let Some(g) = &sp.gpu {
        let mut bits = vec![g.name.clone()];
        bits.extend(g.temp.map(|t| format!("{t} °C")));
        bits.extend(g.busy.map(|b| format!("{b}% busy")));
        bits.extend(g.power_limit.map(|w| format!("power limit {w:.0} W")));
        println!("
{}  {}", st.bold("GRAPHICS CARD"), st.dim(&bits.join(" · ")));
    }
    if let Some(s) = &sp.stability {
        let apps: Vec<String> = s.apps.iter().map(|(name, n)| format!("{name} {n}")).collect();
        let most = if apps.is_empty() { String::new() } else { format!(" Crashed most: {}.", apps.join(", ")) };
        println!("
{}  {}", st.bold("STABILITY"), st.dim(&format!("{}{most}", s.summary())));
    }
    println!("
{}", st.bold("BUSY RIGHT NOW"));
    for b in &sp.busy {
        let procs = if b.count == 1 { "1 process".to_string() } else { format!("{} processes", b.count) };
        println!("  {}{}{}", pad(&format!("{:.1}%", b.cpu), 8), pad(&b.name, 30), st.dim(&format!("{} · {procs}", human_bytes(b.mem))));
    }
}

fn print_settings(st: &Style, found: &[winsettings::Finding]) {
    let fix = found.iter().filter(|f| f.level == winsettings::Level::Fix).count();
    let line = match (found.len(), fix) {
        (0, _) => "Everything's how Windows ships it. Lovely.".to_string(),
        (_, 0) => "A few old tweaks and leftovers. Nothing urgent.".to_string(),
        (_, 1) => "One thing needs fixing, sweetie.".to_string(),
        (_, n) => format!("{n} things need fixing, sweetie."),
    };
    println!("{}  {}", st.pink(&st.bold("mom")), st.bold(&format!("\"{line}\"")));
    let mut areas: Vec<&str> = found.iter().map(|f| f.area).collect();
    areas.sort_unstable();
    areas.dedup();
    for area in areas {
        println!("\n{}", st.bold(&area.to_uppercase()));
        for f in found.iter().filter(|f| f.area == area) {
            let mark = match f.level {
                winsettings::Level::Fix => st.red("!"),
                winsettings::Level::Check => st.yellow("~"),
                winsettings::Level::Info => st.dim("·"),
            };
            let better = if f.area == "Terminal" { "better" } else { "default" };
            println!("  {mark} {}  {}", st.bold(&f.title), st.dim(&format!("now: {} · {better}: {}", f.now, f.default)));
            println!("{}", st.dim(&wrap(&f.why, 92, "      ")));
            match (&f.fix, &f.settings) {
                (Some(x), _) => {
                    let how = if x.admin { " (needs an administrator)" } else { "" };
                    println!("      {}", st.dim(&format!("{}{how}: mom settings fix {}", x.label, f.id)));
                }
                (None, Some(s)) => println!("      {}", st.dim(&format!("Change it in {s}"))),
                (None, None) => {}
            }
            if let Some(c) = &f.command {
                println!("      {}", st.dim(&format!("Run it yourself: {c}")));
            }
        }
    }
}

fn print_room(st: &Style, r: &tidy::RoomReport) {
    let grade = match r.grade {
        'A' | 'B' => st.green(&r.grade.to_string()),
        'C' => st.yellow(&r.grade.to_string()),
        _ => st.red(&r.grade.to_string()),
    };
    println!("{}  {} {}  {}", st.pink(&st.bold("mom")), st.bold(&r.name), st.bold(&format!("[{grade}]")), st.bold(&format!("\"{}\"", r.line)));
    if r.code_project {
        println!("  {}", st.dim("This is a code project. Moving things can break paths, so nothing is pre-selected; `--prompt` writes a chore list for your agent instead."));
    } else if r.projects >= 2 {
        println!(
            "  {}",
            st.dim(&format!("This folder holds {} projects. Mom never moves or groups them (tools and agents find them by path); she only tidies inside them.", r.projects))
        );
    }
    if r.personal {
        println!("  {}", st.dim("This is one of your own folders, so only what's plainly throwaway is ticked."));
    }
    if r.partial {
        println!("  {}", st.yellow("It's too big to look through in full, so this is what mom saw in the first part of it."));
    }
    for f in &r.findings {
        println!("\n  {} {}", st.bold("•"), st.bold(&f.title));
        println!("    {}", st.dim(&f.detail));
        for p in f.paths.iter().take(6) {
            let rel = std::path::Path::new(p).strip_prefix(&r.path).map(|x| x.to_string_lossy().to_string()).unwrap_or_else(|_| p.clone());
            println!("    {}", st.dim(&format!("  {rel}")));
        }
        if f.paths.len() > 6 {
            println!("    {}", st.dim(&format!("  …and {} more", f.paths.len() - 6)));
        }
    }
    if !r.actions.is_empty() {
        println!("\n  {}", st.bold("Mom's plan"));
        for a in &r.actions {
            let mark = if a.selected { st.green("[x]") } else { st.dim("[ ]") };
            println!("  {mark} {}  {}", st.dim(&a.id), a.title);
        }
        println!(
            "\n  {}",
            st.dim("`mom tidy --apply` does the checked ones, `--only a1,a3` picks. Moves can be undone (`mom undo`); deletions go to the Recycle Bin.")
        );
    }
}

fn stop(st: &Style, paths: &Paths, cfg: &Config, targets: Vec<String>, yes: bool, dry_run: bool) -> Result<i32> {
    if targets.is_empty() {
        bail!("stop what? A PID, :port, `leftovers` or `mine`.");
    }
    let (table, snap) = mom_core::snapshot_now(cfg, false);
    let mut chosen: Vec<&Group> = Vec::new();
    let mut agent_asking = false;
    for t in &targets {
        let t = t.trim();
        let found: Vec<&Group> = match t {
            "leftovers" | "all-leftovers" => snap.groups.iter().filter(|g| g.is_leftover()).collect(),
            "mine" => {
                let (kind, session, agent_pid) = my_session(&table);
                let Some(kind) = kind else { bail!("`mine` only works from inside an agent session.") };
                agent_asking = true;
                hooks::session_groups(&snap.groups, kind, &session, agent_pid)
            }
            _ if t.starts_with(':') => {
                let port: u16 = t[1..].parse().with_context(|| format!("{t} isn't a port"))?;
                snap.groups.iter().filter(|g| g.ports.iter().any(|l| l.port == port)).collect()
            }
            _ if t.chars().all(|c| c.is_ascii_digit()) => {
                let pid: u32 = t.parse()?;
                snap.groups.iter().filter(|g| g.root_pid == pid || g.pids.contains(&pid)).collect()
            }
            _ => snap.groups.iter().filter(|g| g.id == t).collect(),
        };
        if found.is_empty() && !matches!(t, "leftovers" | "all-leftovers" | "mine") {
            eprintln!("mom: nothing I'm watching matches {t:?}. (`mom ps --all` lists everything.)");
        }
        for g in found {
            if !chosen.iter().any(|c| c.id == g.id) {
                chosen.push(g);
            }
        }
    }
    if chosen.is_empty() {
        println!("{}", st.green("Nothing to stop."));
        return Ok(0);
    }
    print_groups(st, if dry_run { "WOULD STOP" } else { "STOPPING" }, &chosen);
    if dry_run {
        return Ok(0);
    }
    let risky = chosen.iter().any(|g| matches!(g.status, Status::Running | Status::Yours | Status::Helper | Status::Forgotten));
    let needs_ok = !yes && !agent_asking && (risky || chosen.len() > 1);
    if needs_ok && !confirm(&format!("Stop {}?", plural(chosen.len() as u64, "thing"))) {
        if !std::io::stdin().is_terminal() {
            eprintln!("mom: not a terminal, so pass --yes to confirm.");
            return Ok(1);
        }
        println!("Fine. They keep running.");
        return Ok(0);
    }
    let since = mom_core::util::now_secs();
    mom_core::restart::remember(paths, &table, &chosen);
    let r = kill::stop_groups(&table, &chosen);
    mom_core::history::stopped(paths, &chosen.iter().map(|g| g.name.clone()).collect::<Vec<_>>(), &r, false, since);
    println!("{}", st.green(&r.message));
    for f in &r.failed {
        println!("  {}", st.red(&format!("{} ({}): {}", f.pid, f.name, f.reason)));
    }
    Ok(if r.failed.is_empty() { 0 } else { 2 })
}

fn install(st: &Style, paths: &Paths, what: Vec<String>, on: bool) -> Result<i32> {
    let what: Vec<String> = if what.is_empty() || what.iter().any(|w| w == "all") {
        let list = hooks::integrations(paths);
        let mut v: Vec<String> = list.iter().filter(|i| i.available).map(|i| i.id.clone()).collect();
        if on {
            v.insert(0, "cli".into());
        }
        v
    } else {
        what
    };
    let exe = std::env::current_exe()?;
    for w in &what {
        let msg = match (w.as_str(), on) {
            ("cli", true) => format!("Copied mom to {}.", hooks::install_cli(paths, &exe)?.display()),
            ("cli", false) => {
                let p = paths.installed_cli();
                if p.exists() && p != exe {
                    std::fs::remove_file(&p)?;
                }
                format!("Removed {}.", p.display())
            }
            ("rules", true) | ("rules-claude", true) => rules::install(paths, AgentKind::ClaudeCode)?,
            ("rules", false) | ("rules-claude", false) => rules::remove(paths, AgentKind::ClaudeCode)?,
            ("rules-codex", true) => rules::install(paths, AgentKind::Codex)?,
            ("rules-codex", false) => rules::remove(paths, AgentKind::Codex)?,
            (id, true) => hooks::install(paths, id, &exe)?,
            (id, false) => hooks::uninstall(paths, id)?,
        };
        println!("{} {msg}", st.green("✓"));
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_administrator_mom_only_looks_and_cleans_what_needs_one() {
        let ok = |args: &[&str]| allowed_elevated(&Cli::try_parse_from(std::iter::once("mom").chain(args.iter().copied())).unwrap().cmd);
        assert!(ok(&[]));
        assert!(ok(&["ps", "--all"]));
        assert!(ok(&["trash", "clean", "windows-junk", "--yes", "--elevated"]));
        assert!(ok(&["trash", "look", "--elevated"]), "measuring as administrator is looking");
        assert!(ok(&["stop", "leftovers", "--dry-run"]));
        assert!(ok(&["rules", "show"]));
        assert!(ok(&["tidy", "--json"]));
        assert!(ok(&["files", "--json"]));
        assert!(!ok(&["stop", "leftovers", "--yes"]));
        assert!(!ok(&["tidy", "--apply"]));
        assert!(!ok(&["undo"]));
        assert!(!ok(&["install", "all"]));
        assert!(!ok(&["rules", "install", "claude"]));
        assert!(!ok(&["hook", "claude", "stop"]));
        assert!(ok(&["settings"]) && ok(&["settings", "--json"]));
        assert!(ok(&["settings", "fix", "pagefile", "--elevated"]));
        assert!(!ok(&["settings", "fix", "pagefile"]), "without --elevated it's the user-level path");
        assert!(!ok(&["settings", "fix", "auto-end-tasks", "--elevated"]), "your own settings aren't changed as administrator");
        assert!(!ok(&["settings", "fix", "made-up", "--elevated"]));
        assert!(!ok(&["settings", "undo", "proxy"]));
    }
}
