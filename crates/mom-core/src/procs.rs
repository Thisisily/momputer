//! A snapshot of every process, with the bits mom cares about: who started it, where, and
//! which agent's fingerprints are on it.

use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};

/// Environment variables worth remembering. Agents stamp these into everything they launch,
/// and the stamp survives even after the agent itself is gone.
const MARKER_KEYS: &[&str] = &[
    "AI_AGENT",
    "AGENT",
    "CLAUDECODE",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_PID",
    "CLAUDE_PROJECT_DIR",
    "CODEX_THREAD_ID",
    "CODEX_SANDBOX",
    "CODEX_CI",
    "CODEX_SANDBOX_NETWORK_DISABLED",
    "CURSOR_AGENT",
    "GEMINI_CLI",
    "COPILOT_AGENT",
    "COPILOT_CLI",
    "COPILOT_AGENT_SESSION_ID",
    "OPENCODE",
    "TERM_PROGRAM",
    "MOMPUTER_KEEP",
];

const SECRET_HINTS: &[&str] = &["KEY", "TOKEN", "SECRET", "PASS", "AUTH", "CREDENTIAL", "COOKIE"];

pub fn is_marker_key(key: &str) -> bool {
    let k = key.to_ascii_uppercase();
    if SECRET_HINTS.iter().any(|h| k.contains(h)) {
        return false;
    }
    MARKER_KEYS.contains(&k.as_str()) || k.starts_with("CODEX_") || k.starts_with("OPENCODE_")
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Proc {
    pub pid: u32,
    pub ppid: Option<u32>,
    pub name: String,
    pub exe: Option<String>,
    pub cmd: Vec<String>,
    pub cwd: Option<String>,
    /// Only agent marker variables, never the whole environment.
    pub env: HashMap<String, String>,
    /// Unix seconds.
    pub start: u64,
    /// Working set, bytes.
    pub mem: u64,
    /// Share of the whole machine's CPU since the previous sample, like Task Manager (0 on the first sample).
    pub cpu: f32,
    /// Total CPU time used, milliseconds.
    pub cpu_ms: u64,
    /// Windows session. 0 is where services run. On a Mac: 1 for your own processes, 0 for the
    /// system's and other people's.
    #[serde(skip)]
    pub session: u32,
    /// Its parent ended and the system took it in. (A Mac hands such a process to launchd, where
    /// Windows leaves it its dead parent's PID.)
    #[serde(skip)]
    pub adopted: bool,
    /// It wrote a title over its own arguments, as node's `process.title` and postgres do. `cmd`
    /// is then that title, in words: what it was started with is gone.
    #[serde(skip)]
    pub retitled: bool,
}

/// A program's name the way mom's tables know it on every system: lowercase, without Windows'
/// ".exe" ("node" for node.exe and for node).
pub fn stem_of(name: &str) -> String {
    let n = name.to_ascii_lowercase();
    match n.strip_suffix(".exe") {
        Some(stem) if !stem.is_empty() => stem.to_string(),
        _ => n,
    }
}

impl Proc {
    pub fn lname(&self) -> String {
        self.name.to_ascii_lowercase()
    }
    /// The name without Windows' ".exe", lowercase: what the tables of names are written in.
    pub fn stem(&self) -> String {
        stem_of(&self.name)
    }
    pub fn cmdline(&self) -> String {
        if self.cmd.is_empty() {
            self.name.clone()
        } else {
            self.cmd.join(" ")
        }
    }
    pub fn lexe(&self) -> String {
        self.exe.as_deref().unwrap_or("").replace('/', "\\").to_lowercase()
    }
    pub fn env(&self, key: &str) -> Option<&str> {
        self.env.get(key).map(String::as_str)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ProcTable {
    pub procs: HashMap<u32, Proc>,
    children: HashMap<u32, Vec<u32>>,
    pub taken_at: u64,
    /// The process doing the looking (mom herself). Never touched.
    pub self_pid: u32,
    pub total_mem: u64,
    pub used_mem: u64,
    pub cpus: usize,
}

/// Same-second tolerance for start-time comparisons.
const SLACK: u64 = 1;

impl ProcTable {
    pub fn from_procs(list: Vec<Proc>, self_pid: u32) -> Self {
        let procs: HashMap<u32, Proc> = list.into_iter().map(|p| (p.pid, p)).collect();
        let mut t = ProcTable {
            procs,
            children: HashMap::new(),
            taken_at: crate::util::now_secs(),
            self_pid,
            ..Default::default()
        };
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        for p in t.procs.values() {
            if let Some(parent) = t.valid_parent_pid(p) {
                children.entry(parent).or_default().push(p.pid);
            }
        }
        for v in children.values_mut() {
            v.sort_unstable();
        }
        t.children = children;
        t
    }

    pub fn get(&self, pid: u32) -> Option<&Proc> {
        self.procs.get(&pid)
    }

    /// The parent, but only if it is really the parent: Windows reuses PIDs, so a "parent"
    /// that started after its child is an impostor and the real parent is dead.
    fn valid_parent_pid(&self, p: &Proc) -> Option<u32> {
        let ppid = p.ppid?;
        if ppid == p.pid || ppid == 0 || p.adopted {
            return None;
        }
        let parent = self.procs.get(&ppid)?;
        if parent.start > p.start + SLACK {
            return None;
        }
        Some(ppid)
    }

    pub fn parent(&self, pid: u32) -> Option<&Proc> {
        let p = self.procs.get(&pid)?;
        self.valid_parent_pid(p).and_then(|pp| self.procs.get(&pp))
    }

    /// Has a parent PID, but that parent is gone (or its PID now belongs to someone else).
    pub fn is_orphan(&self, pid: u32) -> bool {
        match self.procs.get(&pid) {
            Some(p) => matches!(p.ppid, Some(pp) if pp != 0 && pp != p.pid) && self.valid_parent_pid(p).is_none(),
            None => false,
        }
    }

    /// Validated ancestor chain, nearest first.
    pub fn ancestors(&self, pid: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut cur = pid;
        while let Some(parent) = self.parent(cur) {
            if !seen.insert(parent.pid) || out.len() > 64 {
                break;
            }
            out.push(parent.pid);
            cur = parent.pid;
        }
        out
    }

    pub fn children(&self, pid: u32) -> &[u32] {
        self.children.get(&pid).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Every descendant, breadth first. `stop` prunes a subtree (it is not included).
    pub fn descendants_until(&self, pid: u32, stop: impl Fn(&Proc) -> bool) -> Vec<u32> {
        let mut out = Vec::new();
        let mut seen = HashSet::from([pid]);
        let mut queue: VecDeque<u32> = self.children(pid).iter().copied().collect();
        while let Some(c) = queue.pop_front() {
            if !seen.insert(c) {
                continue;
            }
            let Some(p) = self.procs.get(&c) else { continue };
            if stop(p) {
                continue;
            }
            out.push(c);
            queue.extend(self.children(c).iter().copied());
        }
        out
    }

    pub fn descendants(&self, pid: u32) -> Vec<u32> {
        self.descendants_until(pid, |_| false)
    }

    /// Mom and everyone above her (the agent and shell that ran `mom`). Never touched.
    pub fn self_and_ancestors(&self) -> HashSet<u32> {
        let mut s: HashSet<u32> = self.ancestors(self.self_pid).into_iter().collect();
        s.insert(self.self_pid);
        s
    }
}

/// The few details that cost a trip into another process's memory. Cached per process.
#[derive(Clone, Debug, Default)]
struct Details {
    exe: Option<String>,
    cmd: Vec<String>,
    cwd: Option<String>,
    env: HashMap<String, String>,
    /// `cmd` is a title the process gave itself, not what it was started with.
    retitled: bool,
    /// A helper an app keeps inside itself, started for it by the system: never an orphan.
    in_bundle: bool,
}

/// One row of the cheap, everything-at-once process list.
#[derive(Clone, Debug)]
pub struct RawProc {
    pub pid: u32,
    pub ppid: Option<u32>,
    pub name: String,
    pub start: u64,
    pub mem: u64,
    pub cpu_ms: u64,
    /// Windows session. 0 is where services live; agents never run there.
    pub session: u32,
    /// Its parent ended and the system took it in (see `Proc::adopted`).
    pub adopted: bool,
}

/// Names worth a closer look (command line, folder, environment). Everything else is only
/// counted, never opened, so mom stays light even with a thousand processes running.
const WORTH_A_LOOK: &[&str] = &[
    // runtimes
    "node", "node_repl", "nodemon", "bun", "bunx", "deno", "python", "pythonw", "py", "pyw", "uv", "uvx",
    "uvicorn", "streamlit", "jupyter", "jupyter-lab", "jupyter-notebook", "celery", "gunicorn", "hypercorn",
    "flask", "gradio", "java", "javaw", "dotnet", "go", "gopls", "air", "cargo", "rustc", "cargo-watch", "trunk",
    "ruby", "rails", "puma", "php", "php-cgi",
    // browsers driven by robots, and their drivers
    "chromedriver", "msedgedriver", "geckodriver", "chrome-headless-shell", "headless_shell",
    // databases, VMs, tunnels, media
    "postgres", "pg_ctl", "mysqld", "mariadbd", "mongod", "mongos", "redis-server", "memurai", "clickhouse",
    "clickhouse-server", "qdrant", "meilisearch", "typesense-server", "surreal", "etcd", "minio", "valkey-server",
    "vboxheadless", "virtualboxvm", "vmware-vmx", "ngrok", "cloudflared", "frpc", "bore", "localtunnel", "lt",
    "stripe", "ssh", "ffmpeg", "ffprobe", "ffplay", "magick", "handbrakecli", "yt-dlp", "sox", "whisper",
    "whisper-cli", "rife", "realesrgan-ncnn-vulkan", "blender",
    // tools agents leave running
    "esbuild", "turbo", "workerd", "ollama", "llama-server", "adb", "watchexec", "tail", "sleep", "watch", "caddy",
    "nginx", "httpd", "miniserve", "make", "mingw32-make", "tsc", "vite", "wrangler", "playwright", "supabase",
    // shells
    "bash", "sh", "zsh", "dash", "fish", "cmd", "powershell", "pwsh", "nu",
    // agents and the apps that host them
    "claude", "codex", "chatgpt", "cursor", "code", "code - insiders", "windsurf", "opencode", "opencode-cli", "goose", "aider", "droid",
    "antigravity", "agy",
];

const BROWSER_STEMS: &[&str] = &[
    "chrome", "msedge", "chromium", "firefox", "brave",
    // as a Mac names them
    "google chrome", "google chrome for testing", "microsoft edge", "brave browser",
];

/// "2.1.331": digits and dots, starting and ending with a digit.
pub(crate) fn is_version(name: &str) -> bool {
    name.contains('.')
        && name.chars().all(|c| c.is_ascii_digit() || c == '.')
        && name.starts_with(|c: char| c.is_ascii_digit())
        && name.ends_with(|c: char| c.is_ascii_digit())
}

/// Children of these are worth a look too (venv entry points under uv, browsers under node).
const LAUNCHERS: &[&str] = &["uv", "uvx", "python", "pythonw", "node", "bun", "deno", "claude", "codex", "agy", "opencode", "opencode-cli"];

const NEVER_INTERESTING: &[&str] = &[
    "conhost", "openconsole", "git", "rg", "grep", "findstr", "where", "whoami", "tasklist", "taskkill", "timeout",
    "choice", "reg", "netstat", "wmic", "msedgewebview2", "werfault", "cat", "ls", "head", "wc", "sed", "awk",
];

pub fn worth_a_look(r: &RawProc, by_pid: &HashMap<u32, RawProc>) -> bool {
    if r.session == 0 || r.pid <= 4 {
        return false;
    }
    let name = r.name.to_ascii_lowercase();
    let stem = name.trim_end_matches(".exe");
    if NEVER_INTERESTING.contains(&stem) {
        return false;
    }
    // A program kept under its version number ("2.1.331", which is how Claude Code installs
    // itself on a Mac): what it was run as says which program it is.
    if cfg!(unix) && is_version(stem) {
        return true;
    }
    let parent_name = r
        .ppid
        .and_then(|pp| by_pid.get(&pp))
        .filter(|p| p.start <= r.start + 1)
        .map(|p| p.name.to_ascii_lowercase())
        .unwrap_or_default();
    if WORTH_A_LOOK.contains(&stem) || stem.starts_with("python3") || stem.starts_with("qemu-system") || stem.starts_with("codex") {
        // IDE and app helpers share the main process's name; only the main one matters.
        if matches!(stem, "chatgpt" | "cursor" | "code" | "code - insiders" | "windsurf" | "antigravity") && parent_name == name {
            return false;
        }
        return true;
    }
    // On a Mac, VS Code and Windsurf run as "Electron", and a terminal's own program is worth
    // knowing by its path (Warp's is called "stable").
    if cfg!(target_os = "macos") && matches!(stem, "electron" | "stable") {
        return parent_name != name;
    }
    if BROWSER_STEMS.contains(&stem) {
        // The main browser process tells us if a robot is driving; its renderers don't.
        return parent_name != name;
    }
    LAUNCHERS.contains(&parent_name.trim_end_matches(".exe"))
}

/// Takes snapshots of the process table. Keep one around: it remembers the expensive details
/// between samples and computes CPU usage from the difference.
pub struct Sampler {
    sys: sysinfo::System,
    details: HashMap<(u32, u64), Details>,
    prev_cpu: HashMap<(u32, u64), u64>,
    prev_at: Option<std::time::Instant>,
    samples: u64,
    #[cfg(target_os = "macos")]
    jobs: mac::Jobs,
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_os = "macos"))]
fn details_of(p: &sysinfo::Process) -> Details {
    Details {
        exe: p.exe().map(|e| e.to_string_lossy().to_string()),
        cmd: p.cmd().iter().map(|a| a.to_string_lossy().to_string()).collect(),
        cwd: p.cwd().map(|c| c.to_string_lossy().to_string()).filter(|c| !c.is_empty()),
        env: p
            .environ()
            .iter()
            .filter_map(|kv| {
                let kv = kv.to_string_lossy();
                let (k, v) = kv.split_once('=')?;
                if k.is_empty() || !is_marker_key(k) {
                    return None;
                }
                Some((k.to_string(), crate::util::ellipsize(v, 300)))
            })
            .collect(),
        retitled: false,
        in_bundle: false,
    }
}

/// An empty sysinfo `System`. Since sysinfo 0.40 `System::new` returns a `Result`, but on
/// Windows, Linux and macOS it only fills in empty fields (on a Mac, it also asks for the size
/// of a page of memory) and never fails.
fn new_system() -> sysinfo::System {
    sysinfo::System::new().expect("sysinfo's System::new doesn't fail on Windows, Linux or macOS")
}

impl Sampler {
    pub fn new() -> Self {
        Sampler {
            sys: new_system(),
            details: HashMap::new(),
            prev_cpu: HashMap::new(),
            prev_at: None,
            samples: 0,
            #[cfg(target_os = "macos")]
            jobs: mac::Jobs::default(),
        }
    }

    pub fn sample(&mut self) -> ProcTable {
        let raw = Self::raw_list().unwrap_or_else(|| self.raw_from_sysinfo());
        self.build(raw)
    }

    /// Every process at once, without opening any of them.
    #[cfg(windows)]
    fn raw_list() -> Option<Vec<RawProc>> {
        nt::processes()
    }

    #[cfg(not(windows))]
    fn raw_list() -> Option<Vec<RawProc>> {
        None
    }

    fn raw_from_sysinfo(&mut self) -> Vec<RawProc> {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_memory().with_cpu(),
        );
        // Windows keeps services in a session of their own. Elsewhere, what isn't yours (the
        // system's own processes, and other people's) is told by who it runs as.
        #[cfg(unix)]
        let me = unsafe { libc::getuid() }; // SAFETY: takes nothing and always answers.
        self.sys
            .processes()
            .values()
            .map(|p| RawProc {
                pid: p.pid().as_u32(),
                ppid: p.parent().map(|pp| pp.as_u32()),
                name: p.name().to_string_lossy().to_string(),
                start: p.start_time(),
                mem: p.memory(),
                cpu_ms: p.accumulated_cpu_time(),
                #[cfg(unix)]
                session: u32::from(p.user_id().is_some_and(|uid| **uid == me)),
                #[cfg(not(unix))]
                session: 1,
                adopted: false,
            })
            .collect()
    }

    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    fn build(&mut self, mut raw: Vec<RawProc>) -> ProcTable {
        let now = std::time::Instant::now();
        let elapsed_ms = self.prev_at.map(|t| now.duration_since(t).as_millis() as u64).unwrap_or(0);
        let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        let by_pid: HashMap<u32, RawProc> = raw.iter().map(|r| (r.pid, r.clone())).collect();
        #[cfg(target_os = "macos")]
        self.jobs.mark_adopted(&mut raw, |r| worth_a_look(r, &by_pid));

        let missing: Vec<u32> = raw
            .iter()
            .filter(|r| worth_a_look(r, &by_pid) && !self.details.contains_key(&(r.pid, r.start)))
            .map(|r| r.pid)
            .collect();
        #[cfg(target_os = "macos")]
        for pid in &missing {
            let r = &by_pid[pid];
            self.details.insert((r.pid, r.start), mac::details(r.pid, r.start));
        }
        #[cfg(not(target_os = "macos"))]
        if !missing.is_empty() {
            use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, UpdateKind};
            let missing: Vec<Pid> = missing.iter().map(|pid| Pid::from_u32(*pid)).collect();
            let kind = ProcessRefreshKind::nothing()
                .with_exe(UpdateKind::Always)
                .with_cmd(UpdateKind::Always)
                .with_cwd(UpdateKind::Always)
                .with_environ(UpdateKind::Always);
            self.sys.refresh_processes_specifics(ProcessesToUpdate::Some(&missing), true, kind);
            for pid in &missing {
                let r = &by_pid[&pid.as_u32()];
                let d = match self.sys.process(*pid) {
                    Some(p) if p.start_time() == 0 || p.start_time().abs_diff(r.start) <= 2 => details_of(p),
                    _ => Details::default(),
                };
                self.details.insert((r.pid, r.start), d);
            }
        }
        let alive: HashSet<(u32, u64)> = raw.iter().map(|r| (r.pid, r.start)).collect();
        self.details.retain(|k, _| alive.contains(k));
        self.samples += 1;
        if self.samples.is_multiple_of(500) {
            // sysinfo remembers every process it was ever asked about; start it fresh now and then.
            self.sys = new_system();
        }

        let mut cpu_now = HashMap::with_capacity(raw.len());
        let list: Vec<Proc> = raw
            .into_iter()
            .map(|r| {
                let key = (r.pid, r.start);
                let cpu = match self.prev_cpu.get(&key) {
                    Some(prev) if elapsed_ms > 0 => {
                        r.cpu_ms.saturating_sub(*prev) as f32 * 100.0 / (elapsed_ms as f32 * cpus as f32)
                    }
                    _ => 0.0,
                };
                cpu_now.insert(key, r.cpu_ms);
                let d = self.details.get(&key).cloned().unwrap_or_default();
                Proc {
                    pid: r.pid,
                    ppid: r.ppid,
                    name: run_as(r.name, &d),
                    exe: d.exe,
                    cmd: d.cmd,
                    cwd: d.cwd,
                    env: d.env,
                    start: r.start,
                    mem: r.mem,
                    cpu: cpu.min(100.0),
                    cpu_ms: r.cpu_ms,
                    session: r.session,
                    adopted: r.adopted && !d.in_bundle,
                    retitled: d.retitled,
                }
            })
            .collect();
        self.prev_cpu = cpu_now;
        self.prev_at = Some(now);

        self.sys.refresh_memory();
        let mut t = ProcTable::from_procs(list, std::process::id());
        t.total_mem = self.sys.total_memory();
        t.used_mem = self.sys.used_memory();
        t.cpus = cpus;
        t
    }
}

/// The name a process goes by. A program kept under its version number ("2.1.331", which is how
/// Claude Code installs itself on a Mac and on Linux) goes by what it was run as: "claude".
fn run_as(name: String, d: &Details) -> String {
    if cfg!(unix) && is_version(&name) && !d.retitled {
        if let Some(first) = d.cmd.first().map(|a| crate::util::file_name(a)).filter(|a| !a.is_empty() && !is_version(a)) {
            return first.to_string();
        }
    }
    name
}

#[cfg(windows)]
mod nt {
    use super::RawProc;
    use windows_sys::Wdk::System::SystemInformation::{NtQuerySystemInformation, SystemProcessInformation};
    use windows_sys::Win32::System::WindowsProgramming::SYSTEM_PROCESS_INFORMATION;

    /// One call to NtQuerySystemInformation: every process's PID, parent, name, start time,
    /// working set and CPU time. Nothing gets opened, so antivirus hooks don't slow it down.
    pub fn processes() -> Option<Vec<RawProc>> {
        let mut size: usize = 2 << 20;
        for _ in 0..8 {
            let mut buf: Vec<u64> = vec![0; size / 8 + 1];
            let mut needed: u32 = 0;
            let status = unsafe {
                NtQuerySystemInformation(SystemProcessInformation, buf.as_mut_ptr().cast(), (buf.len() * 8) as u32, &mut needed)
            };
            if status == 0 {
                return Some(parse(&buf));
            }
            if status as u32 == 0xC000_0004 {
                // STATUS_INFO_LENGTH_MISMATCH: the list grew; ask again with room to spare.
                size = (needed as usize).max(size) + (256 << 10);
                continue;
            }
            return None;
        }
        None
    }

    fn parse(buf: &[u64]) -> Vec<RawProc> {
        let base = buf.as_ptr() as *const u8;
        let total = buf.len() * 8;
        let entry = std::mem::size_of::<SYSTEM_PROCESS_INFORMATION>();
        let mut out = Vec::with_capacity(1024);
        let mut offset = 0usize;
        while offset + entry <= total {
            // SAFETY: the kernel filled `buf` with a chain of SYSTEM_PROCESS_INFORMATION records,
            // each 8-byte aligned, linked by NextEntryOffset; we stay within the buffer.
            let info = unsafe { &*(base.add(offset) as *const SYSTEM_PROCESS_INFORMATION) };
            let pid = info.UniqueProcessId as usize as u32;
            let ppid = info.Reserved2 as usize as u32;
            let name = if info.ImageName.Buffer.is_null() || info.ImageName.Length == 0 {
                if pid == 0 { "Idle".to_string() } else { "System".to_string() }
            } else {
                let chars = unsafe { std::slice::from_raw_parts(info.ImageName.Buffer, info.ImageName.Length as usize / 2) };
                String::from_utf16_lossy(chars)
            };
            // Reserved1 holds private working set (8 bytes), hard faults (4), thread high-water
            // mark (4), cycle time (8), then CreateTime, UserTime and KernelTime as 100ns ticks.
            let r = &info.Reserved1;
            let at = |o: usize| i64::from_le_bytes(r[o..o + 8].try_into().unwrap_or([0; 8]));
            let (create, user, kernel) = (at(24), at(32), at(40));
            out.push(RawProc {
                pid,
                ppid: (ppid != 0).then_some(ppid),
                name,
                start: if create > 0 { (create as u64 / 10_000_000).saturating_sub(11_644_473_600) } else { 0 },
                mem: info.WorkingSetSize as u64,
                cpu_ms: (user.max(0) + kernel.max(0)) as u64 / 10_000,
                session: info.SessionId,
                adopted: false,
            });
            if info.NextEntryOffset == 0 {
                break;
            }
            offset += info.NextEntryOffset as usize;
        }
        out
    }
}

/// What a process was started with, from the kernel's copy of it: its arguments, then the agent
/// markers in its environment (never the rest of it). `retitled` when the process wrote a title
/// over its own arguments: `args` is then that title, in words.
///
/// The copy is a count of arguments, the path the program was run as, some padding, the
/// arguments, then the environment, each ending in a NUL. A program that renames itself (node's
/// `process.title`, which npm sets; postgres; nginx) writes the new name where its arguments were
/// and blanks what's left. Read by the count alone, such a process's "arguments" run on into its
/// environment, keys and all. So a run of blanks followed by something that reads like an
/// environment variable ends the arguments.
#[cfg(any(target_os = "macos", test))]
fn parse_procargs(buf: &[u8]) -> Option<(Vec<String>, HashMap<String, String>, bool)> {
    let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?).max(0) as usize;
    let rest = &buf[4..];
    // The path it was run as, then padding up to the first argument.
    let end = rest.iter().position(|b| *b == 0)?;
    let mut rest = &rest[end..];
    while rest.first() == Some(&0) {
        rest = &rest[1..];
    }
    let mut strings: Vec<&[u8]> = rest.split(|b| *b == 0).collect();
    // What follows the last NUL isn't a whole string.
    strings.pop();
    let text = |b: &[u8]| String::from_utf8_lossy(b).to_string();
    let env_like = |b: &[u8]| {
        let Some(eq) = b.iter().position(|c| *c == b'=') else { return false };
        eq > 0 && !b[0].is_ascii_digit() && b[..eq].iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
    };
    const USUAL: &[&[u8]] = &[b"HOME=", b"PATH=", b"USER=", b"LOGNAME=", b"SHELL=", b"TMPDIR=", b"PWD=", b"__CF_USER_TEXT_ENCODING=", b"XPC_SERVICE_NAME=", b"XPC_FLAGS=", b"SSH_AUTH_SOCK=", b"LANG=", b"TERM="];
    let usual = |b: &[u8]| USUAL.iter().any(|k| b.starts_with(k));

    let mut args: Vec<&[u8]> = Vec::new();
    let mut retitled = false;
    let mut i = 0;
    while args.len() < argc && i < strings.len() {
        if !strings[i].is_empty() {
            args.push(strings[i]);
            i += 1;
            continue;
        }
        // Blanks: empty arguments (`sed -i ''`), or what's left of arguments written over.
        let next = (i..strings.len()).find(|j| !strings[*j].is_empty());
        let blanks = next.unwrap_or(strings.len()) - i;
        let over = next.is_none_or(|j| env_like(strings[j])) && !args.is_empty();
        if over || blanks > argc - args.len() {
            retitled = !args.is_empty();
            i = next.unwrap_or(strings.len());
            break;
        }
        args.extend(std::iter::repeat_n(&[][..], blanks));
        i += blanks;
    }
    // A title exactly as long as the arguments it replaced leaves no blanks: then every
    // "argument" after it is one of the environment's.
    if !retitled && args.len() > 1 && args[1..].iter().all(|a| env_like(a)) && args[1..].iter().any(|a| usual(a)) && args[0].contains(&b' ') {
        i -= args.len() - 1;
        args.truncate(1);
        retitled = true;
    }
    let args: Vec<String> = if retitled {
        text(args[0]).split_whitespace().map(String::from).collect()
    } else {
        args.into_iter().map(text).collect()
    };
    let mut env = HashMap::new();
    for entry in strings[i.min(strings.len())..].iter().skip_while(|e| e.is_empty()) {
        // The environment ends at the first blank.
        if entry.is_empty() {
            break;
        }
        let entry = text(entry);
        let Some((k, v)) = entry.split_once('=') else { continue };
        if !k.is_empty() && is_marker_key(k) {
            env.insert(k.to_string(), crate::util::ellipsize(v, 300));
        }
    }
    Some((args, env, retitled))
}

#[cfg(target_os = "macos")]
mod mac {
    use super::{Details, RawProc};
    use std::collections::HashSet;
    use std::ffi::{c_int, c_void};
    use std::time::{Duration, Instant};

    /// One row of the quick list: enough to know a program is running, and which one.
    pub(super) struct Running {
        pub pid: u32,
        pub start: u64,
        /// The program file's own name (its version number, for one installed that way).
        pub name: String,
    }

    fn bsd_info(pid: u32) -> Option<libc::proc_bsdinfo> {
        // SAFETY: plain data, filled by the kernel up to the size given; the size it returns says
        // whether it filled it all.
        unsafe {
            let mut info: libc::proc_bsdinfo = std::mem::zeroed();
            let size = std::mem::size_of::<libc::proc_bsdinfo>() as c_int;
            (libc::proc_pidinfo(pid as c_int, libc::PROC_PIDTBSDINFO, 0, std::ptr::addr_of_mut!(info).cast::<c_void>(), size) == size).then_some(info)
        }
    }

    /// When it started, unless it's gone or has ended and is only waiting to be collected.
    pub(super) fn started(pid: u32) -> Option<u64> {
        const ZOMBIE: u32 = 5; // SZOMB
        bsd_info(pid).filter(|info| info.pbi_status != ZOMBIE).map(|info| info.pbi_start_tvsec)
    }

    fn c_text(bytes: &[libc::c_char]) -> String {
        let bytes: Vec<u8> = bytes.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
        String::from_utf8_lossy(&bytes).to_string()
    }

    /// Every process, without opening any: PID, start time and name.
    pub(super) fn running() -> Option<Vec<Running>> {
        // SAFETY: the first call asks how many there are; the second fills a buffer of ours that
        // is as long as we say.
        let pids = unsafe {
            let n = libc::proc_listallpids(std::ptr::null_mut(), 0);
            if n <= 0 {
                return None;
            }
            let mut pids: Vec<c_int> = vec![0; n as usize + 64];
            let got = libc::proc_listallpids(pids.as_mut_ptr().cast::<c_void>(), (pids.len() * std::mem::size_of::<c_int>()) as c_int);
            if got <= 0 {
                return None;
            }
            pids.truncate(got as usize);
            pids
        };
        Some(
            pids.into_iter()
                .filter(|pid| *pid > 0)
                .filter_map(|pid| {
                    let info = bsd_info(pid as u32)?;
                    let name = c_text(&info.pbi_name);
                    Some(Running { pid: pid as u32, start: info.pbi_start_tvsec, name: if name.is_empty() { c_text(&info.pbi_comm) } else { name } })
                })
                .collect(),
        )
    }

    /// Where the program's file really is, with links followed.
    pub(super) fn exe_path(pid: u32) -> Option<String> {
        let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        // SAFETY: the buffer is ours and as long as we say.
        let n = unsafe { libc::proc_pidpath(pid as c_int, buf.as_mut_ptr().cast::<c_void>(), buf.len() as u32) };
        (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).to_string())
    }

    /// The folder it's working in.
    fn cwd(pid: u32) -> Option<String> {
        // SAFETY: plain data, filled by the kernel up to the size given.
        unsafe {
            let mut info: libc::proc_vnodepathinfo = std::mem::zeroed();
            let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as c_int;
            if libc::proc_pidinfo(pid as c_int, libc::PROC_PIDVNODEPATHINFO, 0, std::ptr::addr_of_mut!(info).cast::<c_void>(), size) != size {
                return None;
            }
            // The path is declared as rows of 32 characters; it's one string of 1024.
            let path = std::slice::from_raw_parts(info.pvi_cdir.vip_path.as_ptr().cast::<libc::c_char>(), std::mem::size_of_val(&info.pvi_cdir.vip_path));
            Some(c_text(path)).filter(|p| !p.is_empty())
        }
    }

    /// The kernel's copy of what a process was started with (see `parse_procargs`). Only for
    /// your own processes: the kernel won't show anyone else's.
    fn procargs(pid: u32) -> Option<Vec<u8>> {
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as c_int];
        let mut size: libc::size_t = 0;
        // SAFETY: first the size, then a buffer of ours of that size; `size` comes back as what
        // was written.
        unsafe {
            if libc::sysctl(mib.as_mut_ptr(), 3, std::ptr::null_mut(), &mut size, std::ptr::null_mut(), 0) != 0 || size == 0 {
                return None;
            }
            let mut buf = vec![0u8; size];
            if libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr().cast::<c_void>(), &mut size, std::ptr::null_mut(), 0) != 0 {
                return None;
            }
            buf.truncate(size);
            Some(buf)
        }
    }

    /// Inside an app, or one of the system's own: helpers the system starts for an app on its
    /// own account, with launchd as their parent from the start.
    fn in_bundle(exe: &str) -> bool {
        let e = exe.to_ascii_lowercase();
        [".app/contents/", ".xpc/contents/", ".appex/contents/", ".framework/"].iter().any(|m| e.contains(m))
            || ["/system/", "/usr/libexec/", "/library/apple/"].iter().any(|m| e.starts_with(m))
    }

    /// The details of one process, if it's still the one that started at `start`.
    pub(super) fn details(pid: u32, start: u64) -> Details {
        if !bsd_info(pid).is_some_and(|info| info.pbi_start_tvsec.abs_diff(start) <= 2) {
            return Details::default();
        }
        let exe = exe_path(pid);
        let (cmd, env, retitled) = procargs(pid).and_then(|buf| super::parse_procargs(&buf)).unwrap_or_default();
        Details { in_bundle: exe.as_deref().is_some_and(in_bundle), exe, cmd, cwd: cwd(pid), env, retitled }
    }

    /// The processes launchd started itself, as jobs of yours: apps opened from the Dock or
    /// Finder, agents, `brew services`. Anything else of yours whose parent is launchd was handed
    /// to it when its real parent ended, which is how a Mac shows an orphan.
    #[derive(Default)]
    pub(super) struct Jobs {
        pids: HashSet<u32>,
        asked: Option<Instant>,
        /// Launchd's children that weren't among its jobs when it was last asked.
        strays: HashSet<(u32, u64)>,
    }

    /// Launchd is asked again when one of its children is new, and this often regardless (a job's
    /// PID can pass to something else).
    const ASK_AGAIN: Duration = Duration::from_secs(60);

    impl Jobs {
        /// Marks launchd's children that aren't its jobs as adopted, among those `matters` to.
        pub(super) fn mark_adopted(&mut self, raw: &mut [RawProc], matters: impl Fn(&RawProc) -> bool) {
            let children: Vec<(u32, u64)> = raw.iter().filter(|r| r.ppid == Some(1) && r.session != 0 && matters(r)).map(|r| (r.pid, r.start)).collect();
            let news = children.iter().any(|c| !self.pids.contains(&c.0) && !self.strays.contains(c));
            if !children.is_empty() && (news || self.asked.is_none_or(|at| at.elapsed() > ASK_AGAIN)) {
                match jobs() {
                    Some(pids) => {
                        self.pids = pids;
                        self.asked = Some(Instant::now());
                    }
                    // Launchd didn't answer: nothing new is called an orphan on a guess.
                    None => return,
                }
            }
            self.strays = children.into_iter().filter(|c| !self.pids.contains(&c.0)).collect();
            for r in raw.iter_mut() {
                r.adopted = self.strays.contains(&(r.pid, r.start));
            }
        }
    }

    /// `launchctl list`: a PID (or "-"), a status and a label on each line.
    fn jobs() -> Option<HashSet<u32>> {
        let out = crate::util::run("/bin/launchctl", &["list"], Duration::from_secs(4)).ok()?;
        if !out.ok() {
            return None;
        }
        Some(out.text().lines().filter_map(|l| l.split_whitespace().next()?.parse().ok()).collect())
    }
}

/// When a process started, if it's running now: `Some(None)` when it's gone (or has ended and
/// only waits to be collected), and `None` on a system where mom has no quick way to tell.
#[cfg(unix)]
pub(crate) fn started(pid: u32) -> Option<Option<u64>> {
    #[cfg(target_os = "macos")]
    {
        Some(mac::started(pid))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        None
    }
}

/// Where a process's program file is, on a Mac, for any process (the table only keeps it for
/// the ones it reads in full).
#[cfg(target_os = "macos")]
pub(crate) fn exe_path(pid: u32) -> Option<String> {
    mac::exe_path(pid)
}

/// Every process running this moment, by (PID, start time): one cheap call, for waiting on a
/// program to exit.
pub fn running_now() -> Option<HashSet<(u32, u64)>> {
    #[cfg(windows)]
    {
        nt::processes().map(|list| list.into_iter().map(|r| (r.pid, r.start)).collect())
    }
    #[cfg(target_os = "macos")]
    {
        mac::running().map(|list| list.into_iter().map(|r| (r.pid, r.start)).collect())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

/// A fresh process-name snapshot for cleanup guards. Failure means unknown, never "closed".
pub(crate) fn running_names_now() -> Option<HashSet<String>> {
    #[cfg(windows)]
    { nt::processes().map(|list| list.into_iter().map(|r| r.name.to_ascii_lowercase()).collect()) }
    #[cfg(target_os = "macos")]
    { mac::running().map(|list| list.into_iter().map(|r| r.name.to_ascii_lowercase()).collect()) }
    #[cfg(not(any(windows, target_os = "macos")))]
    { None }
}

/// The (PID, start time) of every process running now under one of `names` ("uv" or "uv.exe",
/// any case). Failure means unknown, never "none running".
pub(crate) fn running_named_now(names: &[&str]) -> Option<HashSet<(u32, u64)>> {
    #[cfg(any(windows, target_os = "macos"))]
    {
        let wanted: HashSet<String> = names.iter().map(|n| stem_of(n)).collect();
        #[cfg(windows)]
        let list = nt::processes();
        #[cfg(target_os = "macos")]
        let list = mac::running();
        list.map(|list| list.into_iter().filter(|r| wanted.contains(&stem_of(&r.name))).map(|r| (r.pid, r.start)).collect())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    { let _ = names; None }
}

/// The command lines of a few processes, read now. The table only reads them for processes
/// worth a look (opening every process is slow with some antivirus), so this is for the odd
/// one more: a browser's newest tabs, say. A PID now running something else is left out.
pub fn command_lines(table: &ProcTable, pids: &[u32]) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    let mut unread = Vec::new();
    for pid in pids {
        match table.get(*pid) {
            Some(p) if !p.cmd.is_empty() => {
                out.insert(*pid, p.cmdline());
            }
            Some(_) => unread.push(*pid),
            None => {}
        }
    }
    if unread.is_empty() {
        return out;
    }
    #[cfg(target_os = "macos")]
    for pid in unread {
        let Some(known) = table.get(pid) else { continue };
        let d = mac::details(pid, known.start);
        if !d.cmd.is_empty() {
            out.insert(pid, d.cmd.join(" "));
        }
    }
    #[cfg(not(target_os = "macos"))]
    read_command_lines(table, unread, &mut out);
    out
}

#[cfg(not(target_os = "macos"))]
fn read_command_lines(table: &ProcTable, unread: Vec<u32>, out: &mut HashMap<u32, String>) {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, UpdateKind};
    let unread: Vec<Pid> = unread.into_iter().map(Pid::from_u32).collect();
    let mut sys = new_system();
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&unread), true, ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always));
    for pid in unread {
        let (Some(p), Some(known)) = (sys.process(pid), table.get(pid.as_u32())) else { continue };
        if p.start_time() != 0 && p.start_time().abs_diff(known.start) > 2 {
            continue;
        }
        let line: Vec<String> = p.cmd().iter().map(|a| a.to_string_lossy().to_string()).collect();
        out.insert(pid.as_u32(), line.join(" "));
    }
}

/// Builds a table by hand. Used by tests and by the UI's mock data generator.
pub fn fake(pid: u32, ppid: u32, name: &str, cmd: &str, start: u64) -> Proc {
    Proc {
        pid,
        ppid: if ppid == 0 { None } else { Some(ppid) },
        name: name.to_string(),
        exe: None,
        cmd: cmd.split_whitespace().map(String::from).collect(),
        cwd: None,
        env: HashMap::new(),
        start,
        mem: 50 * 1024 * 1024,
        cpu: 0.0,
        cpu_ms: 0,
        session: 1,
        adopted: false,
        retitled: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reused_pid_is_not_a_parent() {
        // 100 started at t=10 and launched 200 at t=20. 100 died; its PID was reused at t=30.
        let t = ProcTable::from_procs(
            vec![
                fake(100, 1, "explorer.exe", "explorer.exe", 30),
                fake(200, 100, "node.exe", "node server.js", 20),
            ],
            9999,
        );
        assert!(t.parent(200).is_none());
        assert!(t.is_orphan(200));
        assert!(t.children(100).is_empty());
    }

    #[test]
    fn real_parent_is_kept() {
        let t = ProcTable::from_procs(
            vec![
                fake(100, 0, "claude.exe", "claude", 10),
                fake(200, 100, "bash.exe", "bash", 11),
                fake(300, 200, "node.exe", "node a.js", 12),
            ],
            9999,
        );
        assert_eq!(t.ancestors(300), vec![200, 100]);
        assert_eq!(t.descendants(100), vec![200, 300]);
        assert!(!t.is_orphan(300));
        assert!(!t.is_orphan(100)); // no parent at all is not the same as a dead parent
    }

    /// The kernel's copy of what a process was started with: a count, the path it was run as,
    /// some padding, then each string with a NUL after it.
    fn procargs(argc: i32, exec: &str, rest: &[u8]) -> Vec<u8> {
        let mut b = argc.to_ne_bytes().to_vec();
        b.extend_from_slice(exec.as_bytes());
        b.extend_from_slice(&[0, 0, 0, 0]);
        b.extend_from_slice(rest);
        b
    }

    #[test]
    fn arguments_stop_where_the_environment_starts() {
        // Ordinary: three arguments, then the environment. Only the agent markers are kept.
        let (args, env, retitled) = parse_procargs(&procargs(3, "/usr/local/bin/node", b"node\0server.js\0--port=3000\0HOME=/Users/me\0CLAUDECODE=1\0OPENAI_API_KEY=sk-secret\0\0junk")).unwrap();
        assert_eq!((args, retitled), (vec!["node".to_string(), "server.js".into(), "--port=3000".into()], false));
        assert_eq!(env, HashMap::from([("CLAUDECODE".to_string(), "1".to_string())]));

        // An empty argument is an argument (`sed -i ''`), and so is one that reads like a setting.
        let (args, _, retitled) = parse_procargs(&procargs(5, "/usr/bin/sed", b"sed\0-i\0\0s/a/b/\0FOO=bar\0HOME=/Users/me\0")).unwrap();
        assert_eq!((args, retitled), (vec!["sed".to_string(), "-i".into(), String::new(), "s/a/b/".into(), "FOO=bar".into()], false));

        // A program that renamed itself (npm does) wrote its title over its five arguments and
        // blanked the rest. Its arguments end there: what follows is its environment, keys and all.
        let mut renamed = b"npm exec @remotion/mcp@latest".to_vec();
        renamed.extend_from_slice(&[0; 40]);
        renamed.extend_from_slice(b"HOME=/Users/me\0LOGNAME=me\0CLAUDECODE=1\0CLAUDE_CODE_SESSION_ID=abc\0ANTHROPIC_API_KEY=sk-secret\0\0");
        let (args, env, retitled) = parse_procargs(&procargs(5, "/usr/local/bin/node", &renamed)).unwrap();
        assert_eq!((args, retitled), (vec!["npm".to_string(), "exec".into(), "@remotion/mcp@latest".into()], true));
        assert_eq!(env.get("CLAUDE_CODE_SESSION_ID").map(String::as_str), Some("abc"));
        assert!(!env.contains_key("ANTHROPIC_API_KEY") && !env.contains_key("HOME"));

        // A title exactly as long as the arguments it replaced leaves no gap to tell by.
        let (args, env, retitled) = parse_procargs(&procargs(3, "/opt/homebrew/bin/postgres", b"postgres: checkpointer\0HOME=/Users/me\0PATH=/usr/bin\0CODEX_THREAD_ID=t1\0\0")).unwrap();
        assert_eq!((args, retitled), (vec!["postgres:".to_string(), "checkpointer".into()], true));
        assert_eq!(env.get("CODEX_THREAD_ID").map(String::as_str), Some("t1"));

        // Nothing sensible: no crash, no arguments.
        assert!(parse_procargs(&[1, 0]).is_none());
        assert_eq!(parse_procargs(&procargs(0, "/bin/x", b"")).unwrap().0, Vec::<String>::new());
        assert_eq!(parse_procargs(&procargs(9, "/bin/x", b"x\0")).unwrap().0, ["x"]);
    }

    #[test]
    fn a_program_kept_under_its_version_goes_by_what_it_was_run_as() {
        assert!(is_version("2.1.331") && !is_version("node") && !is_version("3") && !is_version("python3.12") && !is_version("1.2."));
        let d = Details { cmd: vec!["claude".into(), "--print".into()], ..Details::default() };
        assert_eq!(run_as("2.1.331".into(), &d), if cfg!(unix) { "claude" } else { "2.1.331" });
        assert_eq!(run_as("node".into(), &d), "node");
        assert_eq!(stem_of("Node.EXE"), "node");
        assert_eq!(stem_of("Google Chrome"), "google chrome");
        assert_eq!(stem_of(".exe"), ".exe");
    }

    #[test]
    fn secrets_never_count_as_markers() {
        assert!(is_marker_key("CLAUDE_CODE_SESSION_ID"));
        assert!(is_marker_key("CODEX_THREAD_ID"));
        assert!(!is_marker_key("CLAUDE_CODE_MESSAGING_TOKEN"));
        assert!(!is_marker_key("CODEX_API_KEY"));
        assert!(!is_marker_key("PATH"));
    }
}

#[cfg(all(test, windows))]
mod nt_tests {
    #[test]
    fn the_fast_list_agrees_with_windows_about_us() {
        let list = super::nt::processes().expect("NtQuerySystemInformation works");
        let me = list.iter().find(|p| p.pid == std::process::id()).expect("we're in the list");
        let now = crate::util::now_secs();
        assert!(me.start > now - 3600 && me.start <= now + 1, "start time {} vs now {now}", me.start);
        assert!(me.mem > 0);
        assert!(me.name.to_ascii_lowercase().ends_with(".exe"));
        let parent = std::process::Command::new("cmd").args(["/c", "echo"]).output().unwrap();
        assert!(parent.status.success());
        assert!(list.len() > 20);
    }
}
