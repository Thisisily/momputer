//! Programs: everything running, as people know it (every Chrome process is one Chrome), and the
//! careful ways to restart, quit or end one.
//!
//! Rules: Windows' own processes, security software, services, agents' sessions and momputer
//! itself are never offered. Quitting asks the program's windows to close, the way the X button
//! does. Ending is for what won't quit, and checks each process's start time first, like every
//! other stop in mom.

use crate::procs::{Proc, ProcTable};
use crate::util::plural;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Program {
    /// What actions name it by: "app:" and its name, lowercase ("app:razer"), or "mom".
    pub id: String,
    pub name: String,
    pub exe: Option<String>,
    /// Share of the whole machine's CPU over the last few seconds, like Task Manager.
    pub cpu: f32,
    /// The same, averaged over the last `avg_secs` seconds (0 when not known yet).
    pub avg_cpu: f32,
    pub avg_secs: u64,
    pub mem: u64,
    /// Percent of the graphics card it keeps busy (its busiest engine), like Task Manager.
    pub gpu: Option<f32>,
    pub count: u32,
    /// Its windows on screen.
    pub windows: u32,
    /// "app" (has a window), "background", "service", "windows" (part of Windows),
    /// "security", "agent" or "mom".
    pub kind: &'static str,
    /// Unix seconds its first process started.
    pub started: u64,
    /// Agent work running inside it, said plainly.
    pub inside: Option<String>,
    /// What mom may do with it: "restart", "quit", "end".
    pub can: Vec<&'static str>,
    /// Each of its processes, oldest first (at most MAX_LINES): what it runs, where, how busy.
    pub procs: Vec<ProcLine>,
    /// Its processes.
    #[serde(skip)]
    pub pids: Vec<u32>,
}

/// One of a program's processes, as its row lists them.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProcLine {
    pub pid: u32,
    pub start: u64,
    /// What it's running: its command, without the folder of its file ("bun test --watch").
    pub what: String,
    /// The project it works in, when mom can tell.
    pub place: Option<String>,
    pub cpu: f32,
    pub mem: u64,
    /// Mom may end this one (and what it started).
    pub can_end: bool,
}

/// Processes listed per program; the rest are counted.
const MAX_LINES: usize = 40;

/// A process's command in one plain line, its file named without its folder.
fn what_runs(p: &Proc) -> String {
    let line = match p.cmd.split_first() {
        Some((file, rest)) => {
            let file = file.rsplit(['\\', '/']).next().unwrap_or(file);
            std::iter::once(file).chain(rest.iter().map(String::as_str)).collect::<Vec<_>>().join(" ")
        }
        None => p.name.clone(),
    };
    let one: String = line.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    crate::util::ellipsize(one.split_whitespace().collect::<Vec<_>>().join(" ").as_str(), 160)
}

// ------------------------------------------------------------------ grouping

/// Processes that belong to whoever started them: a console window to its program, an app's
/// built-in browser to the app.
const HELPERS: &[&str] = &["conhost.exe", "openconsole.exe", "msedgewebview2.exe", "cefsharp.browsersubprocess.exe", "crashpad_handler.exe"];

/// Security software: never offered, whatever it costs.
const SECURITY: &[&str] = &[
    "msmpeng.exe", "nissrv.exe", "securityhealthsystray.exe", "securityhealthservice.exe", "mpdefendercoreservice.exe", "smartscreen.exe",
];
const SECURITY_DIRS: &[&str] = &[
    "\\bitdefender\\", "\\windows defender\\", "\\malwarebytes\\", "\\kaspersky", "\\avast", "\\avg\\", "\\norton", "\\mcafee", "\\eset\\", "\\sophos",
    "\\trend micro\\", "\\webroot\\", "\\crowdstrike\\", "\\sentinelone\\",
];

fn lower(s: &str) -> String {
    s.to_ascii_lowercase()
}

/// Where Windows lives ("c:\windows\"), lowercase.
fn windows_dir() -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    format!("{}\\", lower(root.trim_end_matches('\\')))
}

/// The process a helper belongs to: the first ancestor that isn't a helper itself.
fn owner<'a>(table: &'a ProcTable, p: &'a Proc) -> &'a Proc {
    let mut cur = p;
    for _ in 0..16 {
        if !HELPERS.contains(&p_lname(cur).as_str()) {
            return cur;
        }
        match table.parent(cur.pid) {
            Some(pp) => cur = pp,
            None => return cur,
        }
    }
    cur
}

fn p_lname(p: &Proc) -> String {
    lower(&p.name)
}

/// Lists everything running, busiest first.
pub fn list(table: &ProcTable) -> Vec<Program> {
    let paths = os::image_paths(table);
    note_cpu(table);
    let (avg, avg_secs) = averages(table);
    let windows = os::windows();
    let gpu = os::gpu_by_pid();
    let win_dir = windows_dir();
    let me = std::process::id();
    let mine: HashSet<u32> = table.descendants(me).into_iter().chain([me]).collect();

    struct Acc<'a> {
        procs: Vec<&'a Proc>,
        gpu: HashMap<String, f32>,
        windows: u32,
        /// Windows per file, to pick the one to start again.
        files: HashMap<String, u32>,
        name: String,
    }
    let mut groups: HashMap<String, Acc> = HashMap::new();
    for p in table.procs.values() {
        if p.pid == 0 {
            continue;
        }
        let (id, name, file) = identify(table, p, &paths, &mine);
        let a = groups.entry(id).or_insert_with(|| Acc { procs: vec![], gpu: HashMap::new(), windows: 0, files: HashMap::new(), name });
        a.procs.push(p);
        let w = windows.get(&p.pid).map_or(0, |w| w.len() as u32);
        a.windows += w;
        if let Some(f) = file {
            *a.files.entry(f).or_default() += w;
        }
        if let Some(engines) = gpu.get(&p.pid) {
            for (engine, v) in engines {
                *a.gpu.entry(engine.clone()).or_default() += v;
            }
        }
    }
    let agent_work = agent_family(table);
    let session = table.get(me).map(|p| p.session);

    let mut out = Vec::with_capacity(groups.len());
    for (key, a) in groups {
        let root = a.procs.iter().min_by_key(|p| p.start).copied().unwrap_or(a.procs[0]);
        // The file to start again: the one with windows, or the one that started first.
        let exe = if key == "mom" {
            std::env::current_exe().ok().map(|e| e.to_string_lossy().to_string())
        } else {
            a.files.iter().max_by_key(|(_, w)| **w).filter(|(_, w)| **w > 0).map(|(f, _)| f.clone()).or_else(|| paths.get(&root.pid).cloned())
        };
        let lexe = exe.as_deref().map(lower).unwrap_or_default();
        let name = a.name.clone();
        let agent = a.procs.iter().any(|p| agent_work.contains(&p.pid));
        let stray_helper = HELPERS.contains(&p_lname(owner(table, root)).as_str());
        let kind = if key == "mom" {
            "mom"
        } else if a.procs.iter().any(|p| SECURITY.contains(&p_lname(p).as_str())) || SECURITY_DIRS.iter().any(|d| lexe.contains(d)) {
            "security"
        } else if root.pid <= 4 || crate::health::not_a_program(&root.name) || (!lexe.is_empty() && lexe.starts_with(&win_dir)) || a.procs.iter().any(|p| crate::classify::is_critical(p)) {
            "windows"
        } else if agent {
            "agent"
        } else if a.procs.iter().all(|p| p.session == 0) {
            "service"
        } else if a.windows > 0 && !stray_helper {
            "app"
        } else {
            "background"
        };
        // Buttons only when there's a process of its that mom may touch.
        let touchable = a.procs.iter().any(|p| !agent_work.contains(&p.pid) && actionable(p, paths.get(&p.pid).map(String::as_str), session, &win_dir));
        let can: Vec<&'static str> = match kind {
            _ if stray_helper || !touchable => vec![],
            "app" => {
                let mut c = vec!["quit", "end"];
                if relaunchable(&lexe) {
                    c.insert(0, "restart");
                }
                c
            }
            "background" => {
                let mut c = vec!["end"];
                if relaunchable(&lexe) {
                    c.insert(0, "restart");
                }
                c
            }
            _ => vec![],
        };
        let pids: HashSet<u32> = a.procs.iter().map(|p| p.pid).collect();
        let cores: f32 = a.procs.iter().filter_map(|p| avg.get(&(p.pid, p.start))).sum();
        let mut lines: Vec<&Proc> = a.procs.clone();
        lines.sort_by_key(|p| (p.start, p.pid)); // oldest first, so lines stay where they are
        let procs = lines
            .into_iter()
            .take(MAX_LINES)
            .map(|p| ProcLine {
                pid: p.pid,
                start: p.start,
                what: what_runs(p),
                place: p.cwd.as_deref().and_then(crate::scan::project_name),
                cpu: p.cpu,
                mem: p.mem,
                can_end: can.contains(&"end")
                    && !agent_work.contains(&p.pid)
                    && !mine.contains(&p.pid)
                    && actionable(p, paths.get(&p.pid).map(String::as_str), session, &win_dir),
            })
            .collect();
        out.push(Program {
            inside: if kind == "app" || kind == "background" { inside(table, &pids) } else { None },
            id: key,
            name,
            exe,
            cpu: a.procs.iter().map(|p| p.cpu).sum(),
            avg_cpu: if avg_secs > 0 { cores * 100.0 / table.cpus.max(1) as f32 } else { 0.0 },
            avg_secs,
            mem: a.procs.iter().map(|p| p.mem).sum(),
            gpu: (!a.gpu.is_empty()).then(|| a.gpu.values().copied().fold(0.0, f32::max).min(100.0)),
            count: a.procs.len() as u32,
            windows: a.windows,
            kind,
            started: root.start,
            can,
            procs,
            pids: a.procs.iter().map(|p| p.pid).collect(),
        });
    }
    out.sort_by(|a, b| (b.avg_cpu.max(b.cpu)).total_cmp(&a.avg_cpu.max(a.cpu)).then_with(|| b.mem.cmp(&a.mem)));
    out
}

/// What a process is part of: (the program's id, its name, its file). Helpers count as whoever
/// started them, and programs are told apart by the name people know them by.
fn identify(table: &ProcTable, p: &Proc, paths: &HashMap<u32, String>, mine: &HashSet<u32>) -> (String, String, Option<String>) {
    if mine.contains(&p.pid) {
        return ("mom".into(), "momputer".into(), None);
    }
    let o = owner(table, p);
    let file = paths.get(&o.pid).cloned();
    let name = if HELPERS.contains(&p_lname(o).as_str()) {
        crate::health::program_name(&o.name)
    } else {
        display_name(&o.name, file.as_deref())
    };
    (format!("app:{}", lower(&name)), name, file)
}

/// Agents' sessions and everything they started: theirs to manage, in the Agents tab.
fn agent_family(table: &ProcTable) -> HashSet<u32> {
    let mut out = HashSet::new();
    for p in table.procs.values() {
        if matches!(crate::agents::agent_process(p), Some((_, crate::agents::AgentProc::Session))) {
            out.insert(p.pid);
            out.extend(table.descendants(p.pid));
        }
    }
    out
}

/// A program mom can start again by itself: a file she knows, or a Store app (by its app ID).
fn relaunchable(lexe: &str) -> bool {
    !lexe.is_empty()
}

/// Agent sessions running inside a program ("2 Claude Code sessions").
fn inside(table: &ProcTable, pids: &HashSet<u32>) -> Option<String> {
    let mut by: HashMap<&'static str, u64> = HashMap::new();
    for p in table.procs.values() {
        if pids.contains(&p.pid) {
            continue;
        }
        let Some((kind, crate::agents::AgentProc::Session)) = crate::agents::agent_process(p) else { continue };
        // Only the outermost session of a chain: a session's own children don't count twice.
        let ancestors = table.ancestors(p.pid);
        if !ancestors.iter().any(|a| pids.contains(a)) {
            continue;
        }
        if ancestors.iter().take_while(|a| !pids.contains(a)).any(|a| table.get(*a).is_some_and(|q| crate::agents::agent_process(q).is_some())) {
            continue;
        }
        *by.entry(kind.label()).or_default() += 1;
    }
    if by.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = by.into_iter().map(|(label, n)| plural(n, &format!("{label} session"))).collect();
    parts.sort();
    Some(parts.join(" and "))
}

/// "Microsoft® Visual Studio®" -> "Microsoft Visual Studio", "SnippingTool.exe" -> "SnippingTool".
fn tidy_name(s: &str) -> String {
    let s = s.replace(['®', '™'], "").replace("(R)", "").replace("(TM)", "");
    let s = s.trim();
    s.strip_suffix(".exe").or_else(|| s.strip_suffix(".EXE")).unwrap_or(s).trim().to_string()
}

/// A name people recognise: a known one, the file's own description, or the file name.
fn display_name(proc_name: &str, exe: Option<&str>) -> String {
    let lexe = exe.map(lower).unwrap_or_default();
    let stem = lower(proc_name.trim_end_matches(".exe").trim_end_matches(".EXE"));
    if stem == "claude" && (lexe.contains("\\.local\\bin\\") || lexe.contains("\\npm\\") || lexe.contains("node_modules")) {
        return "Claude Code".into();
    }
    if let Some(known) = crate::health::known_name(proc_name) {
        return known.to_string();
    }
    if let Some(desc) = exe.and_then(os::description).map(|d| tidy_name(&d)).filter(|d| !d.is_empty()) {
        return desc;
    }
    proc_name.strip_suffix(".exe").or_else(|| proc_name.strip_suffix(".EXE")).unwrap_or(proc_name).to_string()
}

/// Every process's file, where Windows will say (for startup apps, say).
pub(crate) fn image_paths(table: &ProcTable) -> HashMap<u32, String> {
    os::image_paths(table)
}

/// A program file's own name for itself ("EA app"), tidied.
pub(crate) fn description(path: &str) -> Option<String> {
    os::description(path).map(|d| tidy_name(&d)).filter(|d| !d.is_empty())
}

// ------------------------------------------------------------------ CPU over the last minutes

/// Snapshots of every process's CPU time: (when, (pid, start) -> CPU milliseconds).
type Snap = (u64, HashMap<(u32, u64), u64>);
static HISTORY: Mutex<VecDeque<Snap>> = Mutex::new(VecDeque::new());

/// Keeps a note of CPU times, at most every 5 s, for the last six minutes.
pub(crate) fn note_cpu(table: &ProcTable) {
    let now = crate::util::now_secs();
    let mut h = HISTORY.lock().unwrap_or_else(|e| e.into_inner());
    if h.back().is_some_and(|(t, _)| now.saturating_sub(*t) < 5) {
        return;
    }
    h.push_back((now, table.procs.values().map(|p| ((p.pid, p.start), p.cpu_ms)).collect()));
    while h.len() > 2 && h.front().is_some_and(|(t, _)| now.saturating_sub(*t) > 360) {
        h.pop_front();
    }
}

/// Each process's average CPU (in cores) since the oldest note of the last five minutes, and
/// over how many seconds. Nothing until there's a note at least 20 s old.
pub(crate) fn averages(table: &ProcTable) -> (HashMap<(u32, u64), f32>, u64) {
    let now = crate::util::now_secs();
    let h = HISTORY.lock().unwrap_or_else(|e| e.into_inner());
    let Some((then, before)) = h.iter().find(|(t, _)| now.saturating_sub(*t) <= 300 && now.saturating_sub(*t) >= 20) else {
        return (HashMap::new(), 0);
    };
    let secs = now.saturating_sub(*then).max(1);
    let mut out = HashMap::new();
    for p in table.procs.values() {
        let key = (p.pid, p.start);
        let used = match before.get(&key) {
            Some(was) => p.cpu_ms.saturating_sub(*was),
            // Started since: all of its CPU time is from the window.
            None if p.start >= *then => p.cpu_ms,
            None => continue,
        };
        out.insert(key, used as f32 / 1000.0 / secs as f32);
    }
    (out, secs)
}

// ------------------------------------------------------------------ restart, quit, end

/// What happened, for the widget to say.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActReport {
    pub ok: bool,
    pub message: String,
}

/// Restarts, quits or ends a program from `list`, by its id. Refuses anything `list` doesn't
/// offer that action for.
pub fn act(table: &ProcTable, id: &str, action: &str) -> ActReport {
    let fail = |m: String| ActReport { ok: false, message: m };
    let Some(prog) = list(table).into_iter().find(|p| p.id == id) else {
        return fail("That program isn't running any more.".into());
    };
    if !prog.can.contains(&action) {
        return fail(match prog.kind {
            "windows" => format!("{} is part of Windows. Mom leaves it alone.", prog.name),
            "security" => format!("{} keeps your PC safe. Mom leaves it alone.", prog.name),
            "service" => format!("{} runs as a Windows service; stop it from Windows' Services if you must.", prog.name),
            "agent" => format!("{} is an agent at work. Stop it from the Agents tab.", prog.name),
            "mom" => "That's me, sweetie.".into(),
            _ => format!("Mom can't {action} {}.", prog.name),
        });
    }
    let name = prog.name.clone();
    match action {
        "quit" => {
            let closed = close_and_wait(table, &prog, 6);
            if closed {
                ActReport { ok: true, message: format!("Closed {name}.") }
            } else {
                fail(format!("{name} is still running (in the tray, probably). End stops it for good."))
            }
        }
        "end" => {
            let (stopped, failed) = end_all(table, &prog);
            if failed == 0 {
                ActReport { ok: true, message: format!("Ended {name} ({}).", plural(stopped as u64, "process")) }
            } else {
                fail(format!("Ended {stopped} of {name}'s processes; {failed} wouldn't budge."))
            }
        }
        "restart" => restart(table, &prog),
        _ => fail(format!("Mom doesn't know how to {action} a program.")),
    }
}

/// The processes that are the program in `table`: the same key as when it was listed.
///
/// A program is grouped by its name, so one can mix your own processes with others (Razer's apps
/// and Razer's services, say). Only the ones `actionable` allows are ever members: what's
/// protected stays protected, process by process, whatever group it's in.
fn members(table: &ProcTable, prog: &Program) -> Vec<Proc> {
    let paths = os::image_paths(table);
    let me = std::process::id();
    let mine: HashSet<u32> = table.descendants(me).into_iter().chain([me]).collect();
    let agents = agent_family(table);
    let session = table.get(me).map(|p| p.session);
    let win_dir = windows_dir();
    table
        .procs
        .values()
        .filter(|p| {
            !mine.contains(&p.pid)
                && !agents.contains(&p.pid)
                && actionable(p, paths.get(&p.pid).map(String::as_str), session, &win_dir)
                && identify(table, p, &paths, &mine).0 == prog.id
        })
        .cloned()
        .collect()
}

/// Whether mom may close or end this one process: it's in your own session (not a service in
/// session 0, not another user's), and it isn't Windows, security software, or critical.
fn actionable(p: &Proc, path: Option<&str>, session: Option<u32>, win_dir: &str) -> bool {
    let lpath = path.map(lower).unwrap_or_default();
    p.pid > 4
        && p.session != 0
        && session == Some(p.session)
        && !crate::classify::is_critical(p)
        && !crate::health::not_a_program(&p.name)
        && !SECURITY.contains(&p_lname(p).as_str())
        && !SECURITY_DIRS.iter().any(|d| lpath.contains(d))
        && !(lpath.is_empty() || lpath.starts_with(win_dir))
}

/// Asks the program's windows to close, and waits up to `secs` for its processes to go.
fn close_and_wait(table: &ProcTable, prog: &Program, secs: u64) -> bool {
    let procs = members(table, prog);
    os::close_windows(&procs.iter().map(|p| p.pid).collect());
    gone_within(&procs, secs)
}

/// Waits up to `secs` for every one of `procs` to exit (by PID and start time, so a recycled
/// PID doesn't count as still there).
fn gone_within(procs: &[Proc], secs: u64) -> bool {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        let Some(now) = crate::procs::running_now() else { return false };
        if !procs.iter().any(|p| now.contains(&(p.pid, p.start))) {
            return true;
        }
        if std::time::Instant::now() >= until {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
}

/// Ends every process of the program, children first. Whoever asked, Windows and agents'
/// sessions are skipped, as everywhere else in mom; an app that hosts agents is the program
/// itself here, so it may go (the widget said what runs inside it first).
fn end_all(table: &ProcTable, prog: &Program) -> (usize, usize) {
    end_these(table, &members(table, prog))
}

/// Ends one of a program's processes, and those it started that are part of the same program, by
/// the line `list` gave for it. Refuses a line `list` doesn't offer to end, or a process that
/// isn't the one it was (its number taken by another since).
pub fn end_one(table: &ProcTable, id: &str, pid: u32, start: u64) -> ActReport {
    let fail = |m: &str| ActReport { ok: false, message: m.into() };
    let Some(prog) = list(table).into_iter().find(|p| p.id == id) else {
        return fail("That program isn't running any more.");
    };
    let Some(line) = prog.procs.iter().find(|l| l.pid == pid && l.start.abs_diff(start) <= 2).cloned() else {
        return fail("That process isn't running any more.");
    };
    if !line.can_end {
        return fail("Mom leaves that one alone.");
    }
    let procs = members(table, &prog);
    let own: HashSet<u32> = procs.iter().map(|p| p.pid).collect();
    let mut chosen: HashSet<u32> = table.descendants(pid).into_iter().filter(|d| own.contains(d)).collect();
    chosen.insert(pid);
    let these: Vec<Proc> = procs.into_iter().filter(|p| chosen.contains(&p.pid)).collect();
    if these.is_empty() {
        return fail("That process isn't running any more.");
    }
    let (stopped, failed) = end_these(table, &these);
    let what = crate::util::ellipsize(&line.what, 60);
    match (stopped, failed) {
        (0, 0) => fail("That process isn't running any more."),
        (_, 0) => ActReport { ok: true, message: format!("Stopped {what}.") },
        _ => ActReport { ok: false, message: format!("Stopped {stopped} of them; {failed} wouldn't budge.") },
    }
}

/// Ends these processes, deepest first, checking each again right before.
fn end_these(table: &ProcTable, procs: &[Proc]) -> (usize, usize) {
    let protected = crate::kill::untouchable(table);
    let asked = table.self_and_ancestors();
    let paths = os::image_paths(table);
    let session = table.get(std::process::id()).map(|p| p.session);
    let win_dir = windows_dir();
    let own: HashSet<u32> = procs.iter().map(|p| p.pid).collect();
    let mut order: Vec<&Proc> = procs.iter().collect();
    // Deepest first, so nothing restarts what's just been ended.
    order.sort_by_key(|p| std::cmp::Reverse(table.ancestors(p.pid).iter().filter(|a| own.contains(a)).count()));
    let (mut stopped, mut failed) = (0, 0);
    for p in order {
        let host = matches!(crate::agents::agent_process(p), Some((_, crate::agents::AgentProc::Host)));
        // Checked again right before: never whoever asked, never anything protected.
        if asked.contains(&p.pid) || (protected.contains(&p.pid) && !host) || !actionable(p, paths.get(&p.pid).map(String::as_str), session, &win_dir) {
            continue;
        }
        match crate::kill::terminate(p.pid, p.start) {
            Ok(true) => stopped += 1,
            Ok(false) => {}
            Err(_) => failed += 1,
        }
    }
    (stopped, failed)
}

fn restart(table: &ProcTable, prog: &Program) -> ActReport {
    let fail = |m: String| ActReport { ok: false, message: m };
    let name = prog.name.clone();
    let Some(exe) = prog.exe.clone() else { return fail(format!("Mom doesn't know where {name} lives, so she can't start it again.")) };
    let procs = members(table, prog);
    // Store apps can't be started from their file; Windows starts them by their app ID.
    let target = if lower(&exe).contains(r"\windowsapps\") {
        match procs.iter().min_by_key(|p| p.start).and_then(|p| os::app_id(p.pid)) {
            Some(id) => format!(r"shell:AppsFolder\{id}"),
            None => return fail(format!("Windows wouldn't say how to start {name} again, so mom left it running. Quit it and open it yourself.")),
        }
    } else {
        exe
    };
    if !close_and_wait(table, prog, 6) {
        // It closed to the tray and is still there: end it.
        end_all(table, prog);
        if !gone_within(&procs, 5) {
            return fail(format!("{name} wouldn't close, so mom didn't start another one."));
        }
    }
    // Through Explorer, so it starts as you'd start it, not as mom's child.
    match std::process::Command::new("explorer.exe").arg(&target).spawn() {
        Ok(_) => ActReport { ok: true, message: format!("Restarted {name}.") },
        Err(e) => fail(format!("Closed {name}, but couldn't start it again ({e}). Open it from the Start menu.")),
    }
}

// ------------------------------------------------------------------ Windows

#[cfg(windows)]
mod os {
    use crate::procs::ProcTable;
    use std::collections::{HashMap, HashSet};
    use std::ffi::c_void;
    use std::sync::{Mutex, OnceLock};

    #[repr(C)]
    struct UnicodeString {
        length: u16,
        maximum_length: u16,
        buffer: *mut u16,
    }

    #[repr(C)]
    struct ProcessIdInfo {
        process_id: *mut c_void,
        image_name: UnicodeString,
    }

    /// SystemProcessIdInformation: a process's file, asked of the kernel by PID, without opening
    /// the process (which antivirus makes slow, and some processes refuse).
    const SYSTEM_PROCESS_ID_INFORMATION: i32 = 88;

    /// Each process's file, by (PID, start time), read once.
    type Paths = HashMap<(u32, u64), Option<String>>;
    static PATHS: Mutex<Option<Paths>> = Mutex::new(None);

    /// Every process's file, "C:\Program Files\...", where Windows will say.
    pub fn image_paths(table: &ProcTable) -> HashMap<u32, String> {
        let mut cache = PATHS.lock().unwrap_or_else(|e| e.into_inner());
        let cache = cache.get_or_insert_with(HashMap::new);
        let alive: HashSet<(u32, u64)> = table.procs.values().map(|p| (p.pid, p.start)).collect();
        cache.retain(|k, _| alive.contains(k));
        let mut out = HashMap::new();
        for p in table.procs.values() {
            let path = cache.entry((p.pid, p.start)).or_insert_with(|| image_path(p.pid)).clone();
            if let Some(path) = path {
                out.insert(p.pid, path);
            }
        }
        out
    }

    fn image_path(pid: u32) -> Option<String> {
        use windows_sys::Wdk::System::SystemInformation::NtQuerySystemInformation;
        if pid <= 4 {
            return None;
        }
        let mut buf = vec![0u16; 520];
        for _ in 0..2 {
            let mut info = ProcessIdInfo {
                process_id: pid as usize as *mut c_void,
                image_name: UnicodeString { length: 0, maximum_length: (buf.len() * 2).min(65_534) as u16, buffer: buf.as_mut_ptr() },
            };
            // SAFETY: `info` points at our buffer and says how big it is; the kernel writes the
            // name into it and its length into `info`.
            let status = unsafe {
                NtQuerySystemInformation(SYSTEM_PROCESS_ID_INFORMATION, (&mut info as *mut ProcessIdInfo).cast(), std::mem::size_of::<ProcessIdInfo>() as u32, std::ptr::null_mut())
            };
            if status == 0 {
                let n = (info.image_name.length as usize / 2).min(buf.len());
                return dos_path(&String::from_utf16_lossy(&buf[..n]));
            }
            if status as u32 == 0xC000_0004 && (info.image_name.maximum_length as usize) > buf.len() * 2 {
                buf = vec![0u16; info.image_name.maximum_length as usize / 2 + 1];
                continue;
            }
            return None;
        }
        None
    }

    /// "\Device\HarddiskVolume3\Program Files\x.exe" -> "C:\Program Files\x.exe".
    fn dos_path(nt: &str) -> Option<String> {
        static DRIVES: OnceLock<Vec<(String, String)>> = OnceLock::new();
        let drives = DRIVES.get_or_init(|| {
            use windows_sys::Win32::Storage::FileSystem::QueryDosDeviceW;
            let mut out = Vec::new();
            for letter in b'A'..=b'Z' {
                let name = crate::util::wide(&format!("{}:", letter as char));
                let mut target = vec![0u16; 1024];
                // SAFETY: `name` is NUL-terminated; `target` holds as many chars as passed.
                let n = unsafe { QueryDosDeviceW(name.as_ptr(), target.as_mut_ptr(), target.len() as u32) };
                if n > 0 {
                    let end = target.iter().position(|c| *c == 0).unwrap_or(0);
                    out.push((String::from_utf16_lossy(&target[..end]).to_ascii_lowercase(), format!("{}:", letter as char)));
                }
            }
            out
        });
        let lower = nt.to_ascii_lowercase();
        for (device, letter) in drives {
            if let Some(rest) = lower.strip_prefix(device.as_str()) {
                if rest.starts_with('\\') {
                    return Some(format!("{letter}{}", &nt[device.len()..]));
                }
            }
        }
        None
    }

    /// The visible, titled top-level windows of each process: what the X button would close.
    pub fn windows() -> HashMap<u32, Vec<isize>> {
        use windows_sys::Win32::Foundation::{HWND, LPARAM};
        use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindow, GetWindowTextLengthW, GetWindowThreadProcessId, IsWindowVisible, GW_OWNER};
        unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> i32 {
            // SAFETY: lparam is the map passed to EnumWindows below, alive for its whole call.
            let map = unsafe { &mut *(lparam as *mut HashMap<u32, Vec<isize>>) };
            // SAFETY: plain queries on a window handle Windows just gave us.
            unsafe {
                if IsWindowVisible(hwnd) != 0 && GetWindow(hwnd, GW_OWNER).is_null() && GetWindowTextLengthW(hwnd) > 0 {
                    let mut pid = 0u32;
                    GetWindowThreadProcessId(hwnd, &mut pid);
                    map.entry(pid).or_default().push(hwnd as isize);
                }
            }
            1
        }
        let mut map: HashMap<u32, Vec<isize>> = HashMap::new();
        // SAFETY: the callback only touches `map`, which outlives the call.
        unsafe { EnumWindows(Some(each), &mut map as *mut _ as LPARAM) };
        map
    }

    /// Asks every window of these processes to close, like pressing its X.
    pub fn close_windows(pids: &HashSet<u32>) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
        for (pid, hwnds) in windows() {
            if pids.contains(&pid) {
                for h in hwnds {
                    // SAFETY: posting a message to a window handle; harmless if it's gone.
                    unsafe { PostMessageW(h as *mut c_void, WM_CLOSE, 0, 0) };
                }
            }
        }
    }

    /// A Store app's ID ("OpenAI.Codex_2p2nqsd0c76g0!App"), which starts it again.
    pub fn app_id(pid: u32) -> Option<String> {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
        // SAFETY: the handle is closed before returning; the buffer holds as many chars as passed.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut len = 256u32;
            let mut buf = vec![0u16; len as usize];
            let rc = GetApplicationUserModelId(h, &mut len, buf.as_mut_ptr());
            CloseHandle(h);
            if rc != 0 {
                return None;
            }
            let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
            Some(String::from_utf16_lossy(&buf[..end])).filter(|s| !s.is_empty())
        }
    }

    /// A program file's own description ("Google Chrome", "EA app"), from its version info.
    pub fn description(path: &str) -> Option<String> {
        static CACHE: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        let cache = cache.get_or_insert_with(HashMap::new);
        cache.entry(path.to_ascii_lowercase()).or_insert_with(|| read_description(path)).clone()
    }

    fn read_description(path: &str) -> Option<String> {
        use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
        let w = crate::util::wide(path);
        // SAFETY: plain calls with buffers we own; VerQueryValueW points into `data`, which
        // outlives every read through it.
        unsafe {
            let size = GetFileVersionInfoSizeW(w.as_ptr(), std::ptr::null_mut());
            if size == 0 {
                return None;
            }
            let mut data = vec![0u8; size as usize];
            if GetFileVersionInfoW(w.as_ptr(), 0, size, data.as_mut_ptr().cast()) == 0 {
                return None;
            }
            let query = |sub: &str| -> Option<String> {
                let q = crate::util::wide(sub);
                let mut ptr: *mut c_void = std::ptr::null_mut();
                let mut len = 0u32;
                if VerQueryValueW(data.as_ptr().cast(), q.as_ptr(), &mut ptr, &mut len) == 0 || ptr.is_null() || len == 0 {
                    return None;
                }
                let s = std::slice::from_raw_parts(ptr as *const u16, len as usize);
                let end = s.iter().position(|c| *c == 0).unwrap_or(s.len());
                Some(String::from_utf16_lossy(&s[..end]).trim().to_string()).filter(|s| !s.is_empty())
            };
            let mut langs = Vec::new();
            {
                let q = crate::util::wide(r"\VarFileInfo\Translation");
                let mut ptr: *mut c_void = std::ptr::null_mut();
                let mut len = 0u32;
                if VerQueryValueW(data.as_ptr().cast(), q.as_ptr(), &mut ptr, &mut len) != 0 && !ptr.is_null() {
                    let pairs = std::slice::from_raw_parts(ptr as *const u16, len as usize / 2);
                    for c in pairs.chunks_exact(2) {
                        langs.push(format!("{:04x}{:04x}", c[0], c[1]));
                    }
                }
            }
            langs.extend(["040904b0".to_string(), "040904e4".to_string(), "000004b0".to_string()]);
            for l in langs {
                for field in ["FileDescription", "ProductName"] {
                    if let Some(v) = query(&format!(r"\StringFileInfo\{l}\{field}")) {
                        // Some files just say "Electron" or repeat the file name; not useful.
                        if !v.eq_ignore_ascii_case("electron") && v.len() <= 60 {
                            return Some(v);
                        }
                    }
                }
            }
            None
        }
    }

    /// How busy each process keeps the graphics card, per engine ("3D", "VideoDecode"), in
    /// percent: Windows' own GPU Engine counters, as Task Manager reads them.
    pub fn gpu_by_pid() -> HashMap<u32, HashMap<String, f32>> {
        use windows_sys::Win32::System::Performance::{
            PdhAddEnglishCounterW, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE,
        };
        struct Query {
            query: isize,
            counter: isize,
        }
        static QUERY: Mutex<Option<Query>> = Mutex::new(None);
        let mut q = QUERY.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: PDH handles we open once and keep; buffers sized as PDH asks.
        unsafe {
            if q.is_none() {
                let mut query = std::ptr::null_mut();
                if PdhOpenQueryW(std::ptr::null(), 0, &mut query) != 0 {
                    return HashMap::new();
                }
                let path = crate::util::wide(r"\GPU Engine(*)\Utilization Percentage");
                let mut counter = std::ptr::null_mut();
                if PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter) != 0 {
                    return HashMap::new();
                }
                // A rate needs two readings; the first is the starting point.
                PdhCollectQueryData(query);
                std::thread::sleep(std::time::Duration::from_millis(400));
                *q = Some(Query { query: query as isize, counter: counter as isize });
            }
            let Some(Query { query, counter }) = q.as_ref() else { return HashMap::new() };
            if PdhCollectQueryData(*query as *mut c_void) != 0 {
                return HashMap::new();
            }
            let mut size = 0u32;
            let mut count = 0u32;
            PdhGetFormattedCounterArrayW(*counter as *mut c_void, PDH_FMT_DOUBLE, &mut size, &mut count, std::ptr::null_mut());
            if size == 0 {
                return HashMap::new();
            }
            let mut buf: Vec<u64> = vec![0; size as usize / 8 + 1];
            let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            if PdhGetFormattedCounterArrayW(*counter as *mut c_void, PDH_FMT_DOUBLE, &mut size, &mut count, items) != 0 {
                return HashMap::new();
            }
            let mut out: HashMap<u32, HashMap<String, f32>> = HashMap::new();
            for i in 0..count as usize {
                let item = &*items.add(i);
                let value = item.FmtValue.Anonymous.doubleValue;
                if value <= 0.0 || item.szName.is_null() {
                    continue;
                }
                let mut n = 0;
                while *item.szName.add(n) != 0 {
                    n += 1;
                }
                let name = String::from_utf16_lossy(std::slice::from_raw_parts(item.szName, n));
                if let Some((pid, engine)) = super::parse_engine(&name) {
                    *out.entry(pid).or_default().entry(engine).or_default() += value as f32;
                }
            }
            out
        }
    }
}

#[cfg(not(windows))]
mod os {
    use crate::procs::ProcTable;
    use std::collections::{HashMap, HashSet};
    pub fn image_paths(table: &ProcTable) -> HashMap<u32, String> {
        table.procs.values().filter_map(|p| Some((p.pid, p.exe.clone()?))).collect()
    }
    pub fn windows() -> HashMap<u32, Vec<isize>> {
        HashMap::new()
    }
    pub fn close_windows(_: &HashSet<u32>) {}
    pub fn app_id(_: u32) -> Option<String> {
        None
    }
    pub fn description(_: &str) -> Option<String> {
        None
    }
    pub fn gpu_by_pid() -> HashMap<u32, HashMap<String, f32>> {
        HashMap::new()
    }
}

/// "pid_1234_luid_0x0_0x9ba5_phys_0_eng_0_engtype_3D" -> (1234, "3D").
fn parse_engine(name: &str) -> Option<(u32, String)> {
    let pid = name.strip_prefix("pid_")?.split('_').next()?.parse().ok()?;
    let engine = name.rsplit_once("engtype_")?.1.to_string();
    Some((pid, engine))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procs::fake;

    #[test]
    fn reads_gpu_engine_names() {
        assert_eq!(parse_engine("pid_23684_luid_0x00000001_0x9BA5A9CB_phys_0_eng_0_engtype_3D"), Some((23684, "3D".into())));
        assert_eq!(parse_engine("pid_8_luid_0x0_0x0_phys_0_eng_12_engtype_VideoDecode"), Some((8, "VideoDecode".into())));
        assert_eq!(parse_engine("_Total"), None);
    }

    #[test]
    fn helpers_belong_to_the_program_that_started_them() {
        let app = fake(10, 1, "momputer.exe", "momputer", 100);
        let mut view = fake(11, 10, "msedgewebview2.exe", "msedgewebview2", 101);
        view.session = 1;
        let inner = fake(12, 11, "msedgewebview2.exe", "msedgewebview2 --type=renderer", 102);
        let console = fake(13, 20, "conhost.exe", "conhost", 103);
        let tool = fake(20, 1, "node.exe", "node", 99);
        let table = ProcTable::from_procs(vec![app, view, inner, console, tool], 999);
        assert_eq!(owner(&table, table.get(12).unwrap()).pid, 10);
        assert_eq!(owner(&table, table.get(13).unwrap()).pid, 20);
        assert_eq!(owner(&table, table.get(20).unwrap()).pid, 20);
    }

    #[test]
    fn agent_sessions_inside_an_app_are_counted_once() {
        let mut host = fake(10, 1, "claude.exe", "Claude.exe", 100);
        host.exe = Some(r"C:\Program Files\WindowsApps\Claude_2.0_x64__abc\app\Claude.exe".into());
        let mut s1 = fake(20, 10, "claude.exe", "claude --resume", 110);
        s1.exe = Some(r"C:\Users\me\.local\bin\claude.exe".into());
        let mut s2 = fake(21, 10, "claude.exe", "claude", 111);
        s2.exe = s1.exe.clone();
        // A session's own child session doesn't count again.
        let mut sub = fake(22, 20, "claude.exe", "claude -p", 112);
        sub.exe = s1.exe.clone();
        let table = ProcTable::from_procs(vec![host, s1, s2, sub], 999);
        let pids: HashSet<u32> = [10].into_iter().collect();
        assert_eq!(inside(&table, &pids).as_deref(), Some("2 Claude Code sessions"));
        assert_eq!(inside(&table, &[20, 21, 22].into_iter().collect()), None);
    }

    #[test]
    fn only_your_own_processes_can_be_ended() {
        let win = r"c:\windows\";
        let mut app = fake(30, 1, "RazerAppEngine.exe", "RazerAppEngine", 1);
        app.session = 1;
        let app_path = Some(r"C:\Program Files\Razer\RazerAppEngine\RazerAppEngine.exe");
        assert!(actionable(&app, app_path, Some(1), win));
        // The same group's service, in session 0: never.
        let mut service = fake(31, 1, "RzActionSvc.exe", "RzActionSvc", 1);
        service.session = 0;
        assert!(!actionable(&service, Some(r"C:\Program Files (x86)\Razer\RzActionSvc.exe"), Some(1), win));
        // Another user's copy, in their session: never.
        let mut theirs = app.clone();
        theirs.session = 2;
        assert!(!actionable(&theirs, app_path, Some(1), win));
        // Windows, security software, critical processes, and anything whose file Windows won't name.
        assert!(!actionable(&app, Some(r"C:\Windows\System32	askhostw.exe"), Some(1), win));
        assert!(!actionable(&app, Some(r"C:\Program Files\Bitdefender\Bitdefender Security Appdagent.exe"), Some(1), win));
        let mut explorer = fake(32, 1, "explorer.exe", "explorer", 1);
        explorer.session = 1;
        assert!(!actionable(&explorer, Some(r"D:\explorer.exe"), Some(1), win));
        assert!(!actionable(&app, None, Some(1), win));
        // Without knowing our own session, nothing.
        assert!(!actionable(&app, app_path, None, win));
    }

    #[test]
    fn claude_code_is_told_from_the_claude_app() {
        assert_eq!(display_name("claude.exe", Some(r"C:\Users\me\.local\bin\claude.exe")), "Claude Code");
        assert_eq!(display_name("chrome.exe", Some(r"C:\Program Files\Google\Chrome\Application\chrome.exe")), "Chrome");
    }
}
