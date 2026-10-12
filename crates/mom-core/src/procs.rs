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
}

impl Proc {
    pub fn lname(&self) -> String {
        self.name.to_ascii_lowercase()
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
        if ppid == p.pid || ppid == 0 {
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
    "claude", "codex", "chatgpt", "cursor", "code", "code - insiders", "windsurf", "opencode", "goose", "aider", "droid",
];

const BROWSER_STEMS: &[&str] = &["chrome", "msedge", "chromium", "firefox", "brave"];

/// Children of these are worth a look too (venv entry points under uv, browsers under node).
const LAUNCHERS: &[&str] = &["uv", "uvx", "python", "pythonw", "node", "bun", "deno", "claude", "codex"];

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
    let parent_name = r
        .ppid
        .and_then(|pp| by_pid.get(&pp))
        .filter(|p| p.start <= r.start + 1)
        .map(|p| p.name.to_ascii_lowercase())
        .unwrap_or_default();
    if WORTH_A_LOOK.contains(&stem) || stem.starts_with("python3") || stem.starts_with("qemu-system") || stem.starts_with("codex") {
        // IDE and app helpers share the main process's name; only the main one matters.
        if matches!(stem, "chatgpt" | "cursor" | "code" | "code - insiders" | "windsurf") && parent_name == name {
            return false;
        }
        return true;
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
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}

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
    }
}

impl Sampler {
    pub fn new() -> Self {
        Sampler {
            sys: sysinfo::System::new(),
            details: HashMap::new(),
            prev_cpu: HashMap::new(),
            prev_at: None,
            samples: 0,
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
                session: 1,
            })
            .collect()
    }

    fn build(&mut self, raw: Vec<RawProc>) -> ProcTable {
        use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, UpdateKind};
        let now = std::time::Instant::now();
        let elapsed_ms = self.prev_at.map(|t| now.duration_since(t).as_millis() as u64).unwrap_or(0);
        let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        let by_pid: HashMap<u32, RawProc> = raw.iter().map(|r| (r.pid, r.clone())).collect();

        let missing: Vec<Pid> = raw
            .iter()
            .filter(|r| worth_a_look(r, &by_pid) && !self.details.contains_key(&(r.pid, r.start)))
            .map(|r| Pid::from_u32(r.pid))
            .collect();
        if !missing.is_empty() {
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
            self.sys = sysinfo::System::new();
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
                    name: r.name,
                    exe: d.exe,
                    cmd: d.cmd,
                    cwd: d.cwd,
                    env: d.env,
                    start: r.start,
                    mem: r.mem,
                    cpu: cpu.min(100.0),
                    cpu_ms: r.cpu_ms,
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
            });
            if info.NextEntryOffset == 0 {
                break;
            }
            offset += info.NextEntryOffset as usize;
        }
        out
    }
}

/// The command lines of a few processes, read now. The table only reads them for processes
/// worth a look (opening every process is slow with some antivirus), so this is for the odd
/// one more: a browser's newest tabs, say. A PID now running something else is left out.
pub fn command_lines(table: &ProcTable, pids: &[u32]) -> HashMap<u32, String> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, UpdateKind};
    let mut out = HashMap::new();
    let mut unread = Vec::new();
    for pid in pids {
        match table.get(*pid) {
            Some(p) if !p.cmd.is_empty() => {
                out.insert(*pid, p.cmdline());
            }
            Some(_) => unread.push(Pid::from_u32(*pid)),
            None => {}
        }
    }
    if unread.is_empty() {
        return out;
    }
    let mut sys = sysinfo::System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&unread), true, ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always));
    for pid in unread {
        let (Some(p), Some(known)) = (sys.process(pid), table.get(pid.as_u32())) else { continue };
        if p.start_time() != 0 && p.start_time().abs_diff(known.start) > 2 {
            continue;
        }
        let line: Vec<String> = p.cmd().iter().map(|a| a.to_string_lossy().to_string()).collect();
        out.insert(pid.as_u32(), line.join(" "));
    }
    out
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
