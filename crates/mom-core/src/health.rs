//! A quick checkup of the whole machine: disks, memory, how long since a restart, what starts
//! with Windows, and a couple of settings worth knowing about.
//!
//! Strictly read-only. It reads the registry and asks Windows for numbers; it never changes a
//! setting. Each finding comes with a link to the Settings page where the user can act on it.

use crate::paths::Paths;
use crate::procs::ProcTable;
use crate::util::{human_age_long, human_bytes, plural};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub drives: Vec<Drive>,
    pub memory: Memory,
    pub uptime_secs: u64,
    /// Worst first: "warn", then "tip", then "ok".
    pub checks: Vec<Check>,
    pub startup: Vec<StartupApp>,
    /// The six programs using the most memory, all their processes added up.
    pub top_memory: Vec<ProcMem>,
    /// Programs nobody has opened in a year, biggest first.
    pub unused_apps: Vec<crate::installed::App>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Drive {
    /// "C:\\"
    pub root: String,
    /// The volume label, or "Local Disk".
    pub label: String,
    pub total: u64,
    pub free: u64,
    /// Windows lives here.
    pub system: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
    pub total: u64,
    pub used: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupApp {
    pub name: String,
    pub command: String,
    /// "Run (you)", "Run (all users)" or "Startup folder".
    pub location: String,
    /// Switched on in Task Manager / Settings > Startup apps.
    pub enabled: bool,
    /// The entry exactly as Windows lists it: the Run value's name, or the Startup folder file's.
    #[serde(skip)]
    pub entry: String,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcMem {
    /// "Chrome", "VS Code", or the program's file name without .exe.
    pub name: String,
    /// Working set, bytes, all processes together.
    pub mem: u64,
    pub count: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub id: String,
    /// "ok", "tip" or "warn".
    pub level: &'static str,
    pub title: String,
    pub detail: String,
    pub action: Option<CheckAction>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckAction {
    pub label: String,
    /// Always an `ms-settings:` link.
    pub url: String,
}

const GB: u64 = 1024 * 1024 * 1024;

/// Sizes for sentences: "12 GB", "476 GB", "5.1 GB" (decimals only where they matter).
fn size(b: u64) -> String {
    if b >= 10 * GB {
        format!("{:.0} GB", b as f64 / GB as f64)
    } else {
        human_bytes(b)
    }
}

/// Looks the machine over. Takes a few milliseconds; runs nothing.
pub fn check(table: &ProcTable, paths: &Paths) -> Health {
    let drives = os::drives();
    let memory = os::memory().unwrap_or(Memory { total: table.total_mem, used: table.used_mem });
    let uptime_secs = os::uptime_secs();
    let startup = startup_apps(paths);
    let top_memory = top_memory(table, 6);
    let facts = Facts {
        drives: &drives,
        memory: &memory,
        uptime_secs,
        reboot_pending: os::reboot_pending(),
        storage_sense: os::storage_sense_on(),
        startup_enabled: startup.iter().filter(|s| s.enabled).count(),
        top: top_memory.first(),
    };
    let mut checks = checks(&facts);
    let unused_apps = crate::installed::unused(paths, table, crate::util::now_secs());
    if !unused_apps.is_empty() {
        let size: u64 = unused_apps.iter().map(|a| a.size).sum();
        let names: Vec<&str> = unused_apps.iter().take(3).map(|a| a.name.as_str()).collect();
        checks.push(Check {
            id: "unused-apps".into(),
            level: "tip",
            title: {
                let games = unused_apps.iter().filter(|a| a.from.is_some()).count() as u64;
                let apps = unused_apps.len() as u64 - games;
                let what = match (apps, games) {
                    (a, 0) => crate::util::plural(a, "app"),
                    (0, g) => crate::util::plural(g, "game"),
                    (a, g) => format!("{} and {}", crate::util::plural(a, "app"), crate::util::plural(g, "game")),
                };
                format!("{what} you haven't opened in a year")
            },
            detail: format!(
                "{}{}. Uninstalling the ones you don't need {}; they're listed below.",
                names.join(", "),
                if unused_apps.len() > 3 { " and more" } else { "" },
                if size > 0 { format!("frees about {}", crate::util::human_bytes(size)) } else { "tidies up".to_string() }
            ),
            action: None,
        });
    }
    Health { drives, memory, uptime_secs, checks, startup, top_memory, unused_apps }
}

/// The numbers the checks are made from, so they can be tested without a real machine.
struct Facts<'a> {
    drives: &'a [Drive],
    memory: &'a Memory,
    uptime_secs: u64,
    reboot_pending: bool,
    storage_sense: bool,
    startup_enabled: usize,
    top: Option<&'a ProcMem>,
}

fn action(label: &str, url: &str) -> Option<CheckAction> {
    Some(CheckAction { label: label.into(), url: url.into() })
}

fn check_item(id: &str, level: &'static str, title: String, detail: String, action: Option<CheckAction>) -> Check {
    Check { id: id.into(), level, title, detail, action }
}

/// Nearly full: under 10% free, or under 15 GB free on a drive big enough for that to matter.
fn drive_low(d: &Drive) -> bool {
    d.free.saturating_mul(10) < d.total || (d.total >= 30 * GB && d.free < 15 * GB)
}

fn checks(f: &Facts) -> Vec<Check> {
    let mut out = Vec::new();
    for d in f.drives {
        let letter = d.root.trim_end_matches('\\');
        let id = format!("drive-{}", letter.trim_end_matches(':').to_ascii_lowercase());
        let numbers = format!("{} free of {}.", size(d.free), size(d.total));
        out.push(if drive_low(d) {
            let why = if d.system { " Windows needs room to update, and everything slows down when it runs out." } else { "" };
            check_item(
                &id,
                "warn",
                format!("Drive {letter} is nearly full"),
                format!("{numbers}{why} Mom's Trash tab can make room."),
                action("Open Storage settings", "ms-settings:storagesense"),
            )
        } else {
            check_item(&id, "ok", format!("Drive {letter} has room"), numbers, None)
        });
    }

    let days = f.uptime_secs / 86_400;
    let up = human_age_long(f.uptime_secs);
    out.push(if f.reboot_pending {
        check_item(
            "restart",
            "warn",
            "Windows needs a restart".into(),
            format!("Updates are waiting to finish installing. Up {up}; restart when you get a chance."),
            action("Open Windows Update", "ms-settings:windowsupdate"),
        )
    } else if days > 14 {
        check_item(
            "restart",
            "warn",
            format!("Up {up} without a restart"),
            "That's a long time, sweetie. A restart clears out memory leaks and finishes updates.".into(),
            None,
        )
    } else if days > 7 {
        check_item("restart", "tip", format!("Up {up}"), "A restart clears out memory leaks and finishes updates.".into(), None)
    } else {
        check_item("restart", "ok", format!("Up {up}"), "Restarted recently enough.".into(), None)
    });

    if f.memory.total > 0 {
        let pct = (f.memory.used as f64 * 100.0 / f.memory.total as f64).round() as u64;
        let top = f
            .top
            .map(|t| format!("{} is using {}.", t.name, size(t.mem)))
            .unwrap_or_default();
        let level = if pct > 90 {
            "warn"
        } else if pct > 80 {
            "tip"
        } else {
            "ok"
        };
        let (title, detail) = match level {
            "ok" => (
                "Memory is fine".to_string(),
                format!("{pct}% used ({} of {}). {top}", size(f.memory.used), size(f.memory.total)).trim().to_string(),
            ),
            _ => (
                format!("Memory is {pct}% full"),
                format!("{top} Closing what you're not using makes everything else faster.").trim().to_string(),
            ),
        };
        out.push(check_item("memory", level, title, detail, None));
    }

    let n = f.startup_enabled;
    out.push(if n > 8 {
        check_item(
            "startup",
            "tip",
            format!("{n} apps start with Windows"),
            "Each one slows down logging in. Turn off the ones you don't need straight away.".into(),
            action("Open Startup apps", "ms-settings:startupapps"),
        )
    } else {
        let title = if n == 0 { "Nothing extra starts with Windows".to_string() } else { format!("{} start with Windows", plural(n as u64, "app")) };
        check_item("startup", "ok", title, "A light start. Good.".into(), None)
    });

    out.push(if f.storage_sense {
        check_item(
            "storage-sense",
            "ok",
            "Storage Sense is on".into(),
            "Windows empties old temp files and the Recycle Bin by itself.".into(),
            None,
        )
    } else {
        check_item(
            "storage-sense",
            "tip",
            "Storage Sense is off".into(),
            "Windows can empty old temp files and the Recycle Bin by itself.".into(),
            action("Turn on Storage Sense", "ms-settings:storagepolicies"),
        )
    });

    let rank = |l: &str| match l {
        "warn" => 0,
        "tip" => 1,
        _ => 2,
    };
    out.sort_by_key(|c| rank(c.level));
    out
}

/// Processes that are Windows itself, not programs anyone chose to run.
const NOT_PROGRAMS: &[&str] = &["system", "idle", "registry", "memory compression", "secure system"];

/// Windows' own bookkeeping, not a program anyone chose to run.
pub(crate) fn not_a_program(name: &str) -> bool {
    NOT_PROGRAMS.contains(&name.to_ascii_lowercase().as_str())
}

/// A name people recognise for a program's process.
pub(crate) fn program_name(exe: &str) -> String {
    match known_name(exe) {
        Some(known) => known.to_string(),
        None => exe.strip_suffix(".exe").or_else(|| exe.strip_suffix(".EXE")).unwrap_or(exe).to_string(),
    }
}

/// The name people know a program by, for the ones whose file name says it badly.
pub(crate) fn known_name(exe: &str) -> Option<&'static str> {
    let lower = exe.to_ascii_lowercase();
    let stem = lower.trim_end_matches(".exe");
    let known = match stem {
        "chrome" => "Chrome",
        "msedge" => "Edge",
        "msedgewebview2" => "Edge WebView (inside apps)",
        "firefox" => "Firefox",
        "brave" => "Brave",
        "opera" => "Opera",
        "vivaldi" => "Vivaldi",
        "code" => "VS Code",
        "cursor" => "Cursor",
        "windsurf" => "Windsurf",
        "claude" => "Claude",
        "codex" => "Codex",
        "chatgpt" => "ChatGPT",
        "node" => "Node.js",
        "bun" => "bun",
        "python" | "pythonw" | "py" => "Python",
        "java" | "javaw" => "Java",
        "dotnet" => ".NET",
        "explorer" => "File Explorer",
        "vmmem" | "vmmemwsl" => "WSL and virtual machines",
        "docker desktop" => "Docker",
        "discord" => "Discord",
        "slack" => "Slack",
        "ms-teams" | "teams" => "Teams",
        "spotify" => "Spotify",
        "steam" | "steamwebhelper" => "Steam",
        "dwm" => "Desktop Window Manager",
        "svchost" => "Windows services",
        "msmpeng" => "Microsoft Defender",
        "searchhost" | "searchindexer" => "Windows Search",
        "onedrive" => "OneDrive",
        "outlook" | "olk" => "Outlook",
        "photoshop" => "Photoshop",
        "blender" => "Blender",
        "uninstallmonitor" | "iobituninstaler" | "iobitunsvr" => "IObit Uninstaller",
        "razerappengine" | "razer central" | "razercentralservice" | "razercortex" | "cortexlauncherservice" | "gamemanagerservice3" | "razerwdl" | "rzactionsvc" => "Razer",
        "lghub" | "lghub_agent" | "lghub_updater" | "lghub_system_tray" => "Logitech G HUB",
        "icue" => "Corsair iCUE",
        "momputer" => "momputer",
        "snippingtool" => "Snipping Tool",
        "whatsapp" | "whatsapp.root" => "WhatsApp",
        "telegram" => "Telegram",
        "steamservice" => "Steam",
        "nvcontainer" | "nvdisplay.container" => "NVIDIA",
        "com.docker.backend" | "com.docker.build" | "docker-agent" | "com.docker.service" => "Docker",
        _ => "",
    };
    (!known.is_empty()).then_some(known)
}

/// The programs using the most memory, every process of a program added together.
pub fn top_memory(table: &ProcTable, n: usize) -> Vec<ProcMem> {
    let mut by: HashMap<String, ProcMem> = HashMap::new();
    for p in table.procs.values() {
        if p.pid == 0 || p.pid == 4 || not_a_program(&p.name) {
            continue;
        }
        let name = program_name(&p.name);
        let e = by.entry(name.clone()).or_insert(ProcMem { name, mem: 0, count: 0 });
        e.mem += p.mem;
        e.count += 1;
    }
    let mut v: Vec<ProcMem> = by.into_values().collect();
    v.sort_by(|a, b| b.mem.cmp(&a.mem).then_with(|| a.name.cmp(&b.name)));
    v.truncate(n);
    v
}

/// StartupApproved values: the first byte is even when the app may start (02, 06) and odd when
/// someone turned it off in Task Manager or Settings (03, 07). No value means it was never touched,
/// which means on.
pub fn approved(value: Option<&[u8]>) -> bool {
    match value.and_then(|v| v.first()) {
        Some(b) => b & 1 == 0,
        None => true,
    }
}

/// "com.squirrel.Poe.Poe" -> "Poe", "MicrosoftEdgeAutoLaunch_EE7D16C6…" -> "MicrosoftEdgeAutoLaunch".
fn startup_name(value_name: &str) -> String {
    let mut n = value_name.trim();
    if let Some(rest) = n.strip_prefix("electron.app.") {
        n = rest;
    }
    if let Some(rest) = n.strip_prefix("com.squirrel.") {
        n = rest.rsplit('.').next().unwrap_or(rest);
    }
    if let Some((head, tail)) = n.rsplit_once('_') {
        if tail.len() >= 16 && tail.chars().all(|c| c.is_ascii_hexdigit()) {
            n = head;
        }
    }
    n.to_string()
}

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN32: &str = r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

/// Everything set to start with Windows: the Run keys and the Startup folders.
pub fn startup_apps(paths: &Paths) -> Vec<StartupApp> {
    use os::Hive::{CurrentUser as Cu, LocalMachine as Lm};
    let mut out = Vec::new();
    for (hive, key, location, approved_key) in [
        (Cu, RUN, "Run (you)", "Run"),
        (Lm, RUN, "Run (all users)", "Run"),
        (Lm, RUN32, "Run (all users)", "Run32"),
    ] {
        let flags = os::binary_values(hive, &format!(r"{APPROVED}\{approved_key}"));
        for (name, command) in os::string_values(hive, key) {
            if command.trim().is_empty() {
                continue; // does nothing at all
            }
            let enabled = approved(flags.get(&name).map(Vec::as_slice));
            out.push(StartupApp { name: startup_name(&name), entry: name, command, location: location.into(), enabled });
        }
    }
    let programs = Path::new("Microsoft").join("Windows").join("Start Menu").join("Programs");
    let common = crate::util::env_path("ProgramData").map(|d| d.join(&programs).join("StartUp"));
    for (hive, dir) in [(Cu, Some(paths.appdata.join(&programs).join("Startup"))), (Lm, common)] {
        let Some(dir) = dir else { continue };
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let flags = os::binary_values(hive, &format!(r"{APPROVED}\StartupFolder"));
        for e in rd.filter_map(Result::ok) {
            let file = e.file_name().to_string_lossy().to_string();
            if file.eq_ignore_ascii_case("desktop.ini") {
                continue;
            }
            let name = Path::new(&file).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| file.clone());
            let enabled = approved(flags.get(&file).map(Vec::as_slice));
            out.push(StartupApp { name, entry: file, command: e.path().to_string_lossy().to_string(), location: "Startup folder".into(), enabled });
        }
    }
    out
}

/// The fixed drives, with their labels and space.
pub(crate) fn drives() -> Vec<Drive> {
    os::drives()
}

/// Windows is waiting for a restart to finish installing something.
pub(crate) fn reboot_pending() -> bool {
    os::reboot_pending()
}

/// Reading the registry, for other parts of mom (installed programs, say).
#[cfg(windows)]
pub(crate) mod reg {
    pub(crate) use super::os::{binary_values, dword_value, string_values, subkeys, Hive};
}

#[cfg(windows)]
mod os {
    use super::{Drive, Memory};
    use crate::util::wide;
    use std::collections::HashMap;
    use windows_sys::Win32::Foundation::ERROR_MORE_DATA;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ,
        REG_BINARY, REG_DWORD, REG_EXPAND_SZ, REG_SZ,
    };

    #[derive(Clone, Copy)]
    pub enum Hive {
        CurrentUser,
        LocalMachine,
    }

    /// An open registry key, closed when dropped.
    struct Key(HKEY);

    impl Key {
        fn open(hive: Hive, path: &str) -> Option<Key> {
            let root = match hive {
                Hive::CurrentUser => HKEY_CURRENT_USER,
                Hive::LocalMachine => HKEY_LOCAL_MACHINE,
            };
            let w = wide(path);
            let mut key: HKEY = std::ptr::null_mut();
            // SAFETY: `w` is NUL-terminated and outlives the call; `key` receives the handle.
            let rc = unsafe { RegOpenKeyExW(root, w.as_ptr(), 0, KEY_READ, &mut key) };
            (rc == 0).then_some(Key(key))
        }

        /// Every value: (name, type, data).
        fn values(&self) -> Vec<(String, u32, Vec<u8>)> {
            let mut out = Vec::new();
            let mut name = vec![0u16; 16_384];
            let mut data = vec![0u8; 4096];
            let mut i = 0u32;
            loop {
                let mut name_len = name.len() as u32;
                let mut data_len = data.len() as u32;
                let mut kind = 0u32;
                // SAFETY: buffers and their lengths are ours and valid for the call.
                let rc = unsafe {
                    RegEnumValueW(self.0, i, name.as_mut_ptr(), &mut name_len, std::ptr::null(), &mut kind, data.as_mut_ptr(), &mut data_len)
                };
                if rc == ERROR_MORE_DATA {
                    data.resize(data_len as usize + 2, 0);
                    continue; // same index, bigger buffer
                }
                if rc != 0 {
                    break; // ERROR_NO_MORE_ITEMS, or something we can't read
                }
                out.push((String::from_utf16_lossy(&name[..name_len as usize]), kind, data[..data_len as usize].to_vec()));
                i += 1;
            }
            out
        }

        /// Names of the keys inside this one.
        fn subkeys(&self) -> Vec<String> {
            let mut out = Vec::new();
            let mut name = vec![0u16; 512];
            let mut i = 0u32;
            loop {
                let mut len = name.len() as u32;
                // SAFETY: the buffer and its length are ours and valid for the call.
                let rc = unsafe {
                    RegEnumKeyExW(self.0, i, name.as_mut_ptr(), &mut len, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut())
                };
                if rc != 0 {
                    break; // ERROR_NO_MORE_ITEMS
                }
                out.push(String::from_utf16_lossy(&name[..len as usize]));
                i += 1;
            }
            out
        }

        fn dword(&self, value: &str) -> Option<u32> {
            let w = wide(value);
            let mut kind = 0u32;
            let mut data = 0u32;
            let mut len = 4u32;
            // SAFETY: `data` is 4 bytes, as `len` says.
            let rc = unsafe { RegQueryValueExW(self.0, w.as_ptr(), std::ptr::null(), &mut kind, (&mut data as *mut u32).cast(), &mut len) };
            (rc == 0 && kind == REG_DWORD).then_some(data)
        }
    }

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: we opened it and close it once.
            unsafe { RegCloseKey(self.0) };
        }
    }

    fn utf16_string(data: &[u8]) -> String {
        let units: Vec<u16> = data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|u| *u != 0).collect();
        String::from_utf16_lossy(&units)
    }

    /// A number value of a key.
    pub fn dword_value(hive: Hive, path: &str, value: &str) -> Option<u32> {
        Key::open(hive, path)?.dword(value)
    }

    /// Names of the keys inside a key.
    pub fn subkeys(hive: Hive, path: &str) -> Vec<String> {
        Key::open(hive, path).map(|k| k.subkeys()).unwrap_or_default()
    }

    /// Text values of a key, with %VARIABLES% left as they are.
    pub fn string_values(hive: Hive, path: &str) -> Vec<(String, String)> {
        let Some(key) = Key::open(hive, path) else { return vec![] };
        key.values()
            .into_iter()
            .filter(|(_, kind, _)| *kind == REG_SZ || *kind == REG_EXPAND_SZ)
            .map(|(name, _, data)| (name, utf16_string(&data)))
            .collect()
    }

    pub fn binary_values(hive: Hive, path: &str) -> HashMap<String, Vec<u8>> {
        let Some(key) = Key::open(hive, path) else { return HashMap::new() };
        key.values().into_iter().filter(|(_, kind, _)| *kind == REG_BINARY).map(|(n, _, d)| (n, d)).collect()
    }

    pub fn reboot_pending() -> bool {
        Key::open(Hive::LocalMachine, r"SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired").is_some()
            || Key::open(Hive::LocalMachine, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending").is_some()
    }

    pub fn storage_sense_on() -> bool {
        Key::open(Hive::CurrentUser, r"Software\Microsoft\Windows\CurrentVersion\StorageSense\Parameters\StoragePolicy")
            .and_then(|k| k.dword("01"))
            == Some(1)
    }

    pub fn uptime_secs() -> u64 {
        // SAFETY: no arguments.
        unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() / 1000 }
    }

    pub fn memory() -> Option<Memory> {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        // SAFETY: MEMORYSTATUSEX is plain data; dwLength must be set before the call.
        unsafe {
            let mut m: MEMORYSTATUSEX = std::mem::zeroed();
            m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
            if GlobalMemoryStatusEx(&mut m) == 0 {
                return None;
            }
            Some(Memory { total: m.ullTotalPhys, used: m.ullTotalPhys.saturating_sub(m.ullAvailPhys) })
        }
    }

    pub fn drives() -> Vec<Drive> {
        use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetVolumeInformationW};
        let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()).to_ascii_uppercase();
        crate::util::fixed_drives()
            .into_iter()
            .filter_map(|root| {
                let w = wide(&root);
                let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
                // SAFETY: `w` is NUL-terminated; the out pointers are ours.
                if unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, &mut total, &mut free) } == 0 {
                    return None; // not ready
                }
                let mut label = vec![0u16; 261];
                // SAFETY: `label` holds 261 chars, as passed; the other outputs are optional.
                let ok = unsafe {
                    GetVolumeInformationW(
                        w.as_ptr(),
                        label.as_mut_ptr(),
                        label.len() as u32,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        0,
                    )
                };
                let end = label.iter().position(|u| *u == 0).unwrap_or(label.len());
                let label = if ok != 0 { String::from_utf16_lossy(&label[..end]) } else { String::new() };
                Some(Drive {
                    system: root.to_ascii_uppercase().starts_with(&system),
                    label: if label.trim().is_empty() { "Local Disk".into() } else { label },
                    root,
                    total,
                    free: avail.min(free),
                })
            })
            .collect()
    }
}

#[cfg(not(windows))]
mod os {
    use super::{Drive, Memory};
    use std::collections::HashMap;

    #[derive(Clone, Copy)]
    pub enum Hive {
        CurrentUser,
        LocalMachine,
    }
    pub fn string_values(_: Hive, _: &str) -> Vec<(String, String)> {
        vec![]
    }
    pub fn binary_values(_: Hive, _: &str) -> HashMap<String, Vec<u8>> {
        HashMap::new()
    }
    pub fn reboot_pending() -> bool {
        false
    }
    pub fn storage_sense_on() -> bool {
        false
    }
    pub fn uptime_secs() -> u64 {
        0
    }
    pub fn memory() -> Option<Memory> {
        None
    }
    pub fn drives() -> Vec<Drive> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procs::{fake, ProcTable};

    fn drive(letter: &str, free_gb: u64, total_gb: u64) -> Drive {
        Drive { root: format!("{letter}:\\"), label: "Local Disk".into(), total: total_gb * GB, free: free_gb * GB, system: letter == "C" }
    }

    fn facts<'a>(drives: &'a [Drive], memory: &'a Memory, top: Option<&'a ProcMem>) -> Facts<'a> {
        Facts { drives, memory, uptime_secs: 2 * 86_400, reboot_pending: false, storage_sense: true, startup_enabled: 3, top }
    }

    fn find<'c>(checks: &'c [Check], id: &str) -> &'c Check {
        checks.iter().find(|c| c.id == id).unwrap()
    }

    #[test]
    fn checks_follow_the_numbers() {
        let drives = [drive("C", 12, 476), drive("D", 300, 931), drive("E", 50, 1000)];
        let memory = Memory { total: 32 * GB, used: 27 * GB + GB / 2 };
        let chrome = ProcMem { name: "Chrome".into(), mem: 5 * GB + GB / 10, count: 31 };
        let mut f = facts(&drives, &memory, Some(&chrome));
        let c = checks(&f);
        let d = find(&c, "drive-c");
        assert_eq!((d.level, d.title.as_str()), ("warn", "Drive C: is nearly full"));
        assert!(d.detail.starts_with("12 GB free of 476 GB."), "{}", d.detail);
        assert_eq!(d.action.as_ref().unwrap().url, "ms-settings:storagesense");
        assert_eq!(find(&c, "drive-d").level, "ok");
        assert_eq!(find(&c, "drive-e").level, "warn", "5% free is nearly full even with 50 GB left");
        let m = find(&c, "memory");
        assert_eq!((m.level, m.title.as_str()), ("tip", "Memory is 86% full"));
        assert!(m.detail.starts_with("Chrome is using 5.1 GB."), "{}", m.detail);
        assert_eq!(find(&c, "restart").level, "ok");
        assert_eq!(find(&c, "startup").level, "ok");
        assert_eq!(find(&c, "storage-sense").level, "ok");
        assert_eq!(c[0].level, "warn", "worst first");
        assert_eq!(c.last().unwrap().level, "ok");

        f.uptime_secs = 9 * 86_400 + 3600;
        f.startup_enabled = 12;
        f.storage_sense = false;
        let c = checks(&f);
        let r = find(&c, "restart");
        assert_eq!((r.level, r.title.as_str()), ("tip", "Up 9 days"));
        let s = find(&c, "startup");
        assert_eq!((s.level, s.title.as_str()), ("tip", "12 apps start with Windows"));
        assert_eq!(s.action.as_ref().unwrap().url, "ms-settings:startupapps");
        let ss = find(&c, "storage-sense");
        assert_eq!((ss.level, ss.action.as_ref().unwrap().url.as_str()), ("tip", "ms-settings:storagepolicies"));

        f.uptime_secs = 20 * 86_400;
        assert_eq!(find(&checks(&f), "restart").level, "warn");
        f.reboot_pending = true;
        f.uptime_secs = 3600;
        let c = checks(&f);
        let r = find(&c, "restart");
        assert_eq!((r.level, r.action.as_ref().unwrap().url.as_str()), ("warn", "ms-settings:windowsupdate"));

        let full = Memory { total: 16 * GB, used: 15 * GB };
        assert_eq!(find(&checks(&facts(&[], &full, None)), "memory").level, "warn");
        let fine = Memory { total: 16 * GB, used: 8 * GB };
        assert_eq!(find(&checks(&facts(&[], &fine, None)), "memory").level, "ok");
        for c in checks(&f) {
            if let Some(a) = &c.action {
                assert!(a.url.starts_with("ms-settings:"), "{}", a.url);
            }
        }
    }

    #[test]
    fn startup_approved_bytes() {
        assert!(approved(None), "never touched means on");
        assert!(approved(Some(&[0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])));
        assert!(approved(Some(&[0x06, 0, 0, 0])));
        assert!(!approved(Some(&[0x03, 0, 0, 0, 0xA8, 0x73, 0x8F, 0x0F, 0x39, 0xB3, 0xD6, 0x01])));
        assert!(!approved(Some(&[0x07])));
        assert!(approved(Some(&[])));
        assert_eq!(startup_name("com.squirrel.Poe.Poe"), "Poe");
        assert_eq!(startup_name("electron.app.Loom"), "Loom");
        assert_eq!(startup_name("MicrosoftEdgeAutoLaunch_EE7D16C6E9CDB24D9F587FED317293B9"), "MicrosoftEdgeAutoLaunch");
        assert_eq!(startup_name("Steam"), "Steam");
    }

    #[test]
    fn top_memory_adds_up_each_program() {
        let mut procs = vec![
            fake(4, 0, "System", "", 1),
            fake(100, 0, "chrome.exe", "chrome", 1),
            fake(101, 100, "chrome.exe", "chrome --type=renderer", 2),
            fake(102, 0, "Code.exe", "code", 3),
            fake(103, 0, "Memory Compression", "", 1),
            fake(104, 0, "steamwebhelper.exe", "", 1),
            fake(105, 0, "steam.exe", "", 1),
        ];
        procs[1].mem = 3 * GB;
        procs[2].mem = 2 * GB;
        procs[3].mem = GB;
        procs[4].mem = 9 * GB;
        let t = ProcTable::from_procs(procs, 1);
        let top = top_memory(&t, 6);
        assert_eq!(top[0].name, "Chrome");
        assert_eq!((top[0].mem, top[0].count), (5 * GB, 2));
        assert_eq!(top[1].name, "VS Code");
        assert!(top.iter().any(|p| p.name == "Steam" && p.count == 2));
        assert!(!top.iter().any(|p| p.name.contains("System") || p.name.contains("Compression")));
    }
}
