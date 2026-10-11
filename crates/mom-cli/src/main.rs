//! `mom`: momputer from the command line. For you, your scripts, and your agents.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use mom_core::agents::{AgentKind, AgentProc};
use mom_core::scan::{Group, Status};
use mom_core::util::{ellipsize, human_age, human_bytes};
use mom_core::{hooks, kill, rules, tidy, trash, Config, Paths, Snapshot};
use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "mom",
    version,
    about = "momputer: a mom for your AI.\nShows what your AI agents left running and cleans up after them.",
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
    /// A checkup for the whole machine: disks, memory, restarts, startup apps.
    Health {
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
    /// Where mom keeps her settings.
    Config,
}

#[derive(Subcommand)]
enum TrashCmd {
    /// Clean one or more spots (ids from `mom trash`), or `all` (every spot that's safe to clean
    /// without looking first).
    Clean {
        ids: Vec<String>,
        #[arg(short, long)]
        yes: bool,
        /// What was ticked, as JSON (a file written by the widget).
        #[arg(long, hide = true)]
        pick: Option<PathBuf>,
        /// Write what happened, as JSON, here instead of printing it (for the widget, which runs
        /// mom as an administrator for the clean-ups that need it).
        #[arg(long, hide = true)]
        report: Option<PathBuf>,
    },
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
                "+ {} agent helper(s) ({}) and {} of yours. `mom ps --all` to see them.",
                helpers.len(),
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

fn run(cli: Cli) -> Result<i32> {
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
        Some(Cmd::Stop { targets, yes, dry_run }) => return stop(&st, &cfg, targets, yes, dry_run),
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
                        println!("    {}", st.dim(&format!("{} sub-agent(s) busy", t.subagents)));
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
                    let spots = trash::measure_all(&ctx);
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
                (Some(TrashCmd::Clean { ids, yes, pick, report }), _) => {
                    if let Some(out) = report {
                        // The widget asked: one spot, what was ticked, no questions.
                        let [id] = ids.as_slice() else { bail!("--report takes one spot") };
                        let pick: Option<trash::Pick> = match pick {
                            Some(f) => Some(serde_json::from_str(&std::fs::read_to_string(&f)?)?),
                            None => None,
                        };
                        let r = trash::clean(id, &ctx, pick.as_ref());
                        std::fs::write(&out, serde_json::to_string(&r)?)?;
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
                        let r = trash::clean(&s.id, &ctx, None);
                        freed += r.freed;
                        println!("  {}{}", pad(&s.id, 20), r.message);
                    }
                    if spots.len() > 1 {
                        println!("{}", st.green(&mom_core::voice::cleaned(freed)));
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
        Some(Cmd::Tidy { dir, apply, only, prompt, json }) => {
            let dir = match dir {
                Some(d) => d,
                None => std::env::current_dir()?,
            };
            let dir = dir.canonicalize().with_context(|| format!("can't find {}", dir.display()))?;
            let dir = PathBuf::from(dir.to_string_lossy().trim_start_matches(r"\\?\"));
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
            println!("{}", r.message);
            for f in &r.failed {
                println!("  {}", st.red(f));
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
            let line = format!("    {}{}{}", pad(&human_bytes(i.bytes), 10), pad(&i.name, 46), pad(i.sub.as_deref().unwrap_or(""), 34));
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
        0 => "Clean bill of health. Mostly.".to_string(),
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

fn print_room(st: &Style, r: &tidy::RoomReport) {
    let grade = match r.grade {
        'A' | 'B' => st.green(&r.grade.to_string()),
        'C' => st.yellow(&r.grade.to_string()),
        _ => st.red(&r.grade.to_string()),
    };
    println!("{}  {} {}  {}", st.pink(&st.bold("mom")), st.bold(&r.name), st.bold(&format!("[{grade}]")), st.bold(&format!("\"{}\"", r.line)));
    if r.code_project {
        println!("  {}", st.dim("This is a code project. Moving things can break paths, so nothing is pre-selected; `--prompt` writes a chore list for your agent instead."));
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

fn stop(st: &Style, cfg: &Config, targets: Vec<String>, yes: bool, dry_run: bool) -> Result<i32> {
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
    if needs_ok && !confirm(&format!("Stop {} thing(s)?", chosen.len())) {
        if !std::io::stdin().is_terminal() {
            eprintln!("mom: not a terminal, so pass --yes to confirm.");
            return Ok(1);
        }
        println!("Fine. They keep running.");
        return Ok(0);
    }
    let r = kill::stop_groups(&table, &chosen);
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
