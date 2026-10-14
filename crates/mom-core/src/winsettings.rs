//! Windows settings that drifted from the defaults over the years: old speed-up tweaks that do
//! nothing (or worse) today, things switched off that you'd want on, and leftovers pointing at
//! programs and folders that are gone. Mom only reads, unless you press a fix:
//!
//! - Fixes to your own settings (HKCU) back up what was there first, and can be undone.
//! - Fixes that need an administrator put back a value written into mom herself, never one read
//!   from a file, and run in an elevated `mom` that writes nothing but the registry value.

use crate::Paths;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How much a finding matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    /// Hurting you now.
    Fix,
    /// Probably an old tweak or a leftover.
    Check,
    /// A choice, fine either way.
    Info,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// Stable, so the UI and `mom settings fix <id>` can name it.
    pub id: String,
    /// "Memory", "Display", "Startup", "Services", "Shutdown", "Network", "Paths" or "Safety".
    pub area: &'static str,
    pub level: Level,
    pub title: String,
    /// How it's set now.
    pub now: String,
    /// How Windows ships it.
    pub default: String,
    /// What it does to you, in at most two short sentences.
    pub why: String,
    /// What mom's button would do, if she can do it.
    pub fix: Option<Fix>,
    /// Where to change it by hand: an `ms-settings:` page or a program to run.
    pub settings: Option<String>,
    /// A command that fixes it, to copy and run yourself (an install mom won't do for you).
    pub command: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fix {
    pub label: String,
    /// Needs an administrator (Windows asks first).
    pub admin: bool,
    /// Mom keeps a backup and can put it back.
    pub undo: bool,
    /// When it takes effect, if not right away.
    pub after: Option<String>,
}

/// Everything that drifted, most pressing first.
pub fn check(paths: &Paths) -> Vec<Finding> {
    check_with(&real(paths))
}

pub(crate) fn check_with(m: &dyn Machine) -> Vec<Finding> {
    let mut out = Vec::new();
    memory(m, &mut out);
    services(m, &mut out);
    safety(m, &mut out);
    shutdown(m, &mut out);
    display(m, &mut out);
    network(m, &mut out);
    terminal(m, &mut out);
    leftovers(m, &mut out);
    out.sort_by_key(|f| match f.level {
        Level::Fix => 0,
        Level::Check => 1,
        Level::Info => 2,
    });
    out
}

// ------------------------------------------------------------------ whole keys, for other parts of mom

/// Reading, removing and putting back whole registry keys (for registry leftovers).
pub(crate) mod keys {
    pub(crate) use super::{Hive, Raw};

    pub(crate) fn values(hive: Hive, key: &str) -> Vec<(String, Raw)> {
        super::os::values(hive, key)
    }

    pub(crate) fn subkeys(hive: Hive, key: &str) -> Vec<String> {
        super::os::subkeys(hive, key)
    }

    /// Removes a key with no keys inside it.
    pub(crate) fn delete(hive: Hive, key: &str) -> Result<(), String> {
        if !subkeys(hive, key).is_empty() {
            return Err("it has keys inside it".into());
        }
        super::os::delete_key(hive, key)
    }

    /// Makes the key again, with these values.
    pub(crate) fn restore(hive: Hive, key: &str, values: &[(String, Raw)]) -> Result<(), String> {
        super::os::create_key(hive, key)?;
        for (name, raw) in values {
            super::os::set(hive, key, name, raw)?;
        }
        Ok(())
    }
}

// ------------------------------------------------------------------ what mom reads

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum Hive {
    #[serde(rename = "HKCU")]
    CurrentUser,
    #[serde(rename = "HKLM")]
    LocalMachine,
}
use Hive::{CurrentUser as CU, LocalMachine as LM};

const REG_SZ: u32 = 1;
const REG_EXPAND_SZ: u32 = 2;
const REG_DWORD: u32 = 4;
const REG_MULTI_SZ: u32 = 7;

/// A registry value as stored: its type and bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Raw {
    pub kind: u32,
    pub data: Vec<u8>,
}

impl Raw {
    fn dword(n: u32) -> Raw {
        Raw { kind: REG_DWORD, data: n.to_le_bytes().to_vec() }
    }
    fn text(kind: u32, s: &str) -> Raw {
        Raw { kind, data: s.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect() }
    }
    fn string(s: &str) -> Raw {
        Raw::text(REG_SZ, s)
    }
    fn multi(items: &[&str]) -> Raw {
        let mut units: Vec<u16> = Vec::new();
        for i in items {
            units.extend(i.encode_utf16());
            units.push(0);
        }
        units.push(0);
        Raw { kind: REG_MULTI_SZ, data: units.into_iter().flat_map(u16::to_le_bytes).collect() }
    }
    fn as_dword(&self) -> Option<u32> {
        (self.kind == REG_DWORD && self.data.len() >= 4).then(|| u32::from_le_bytes([self.data[0], self.data[1], self.data[2], self.data[3]]))
    }
    fn units(&self) -> Vec<u16> {
        self.data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
    }
    fn as_string(&self) -> Option<String> {
        if self.kind != REG_SZ && self.kind != REG_EXPAND_SZ {
            return None;
        }
        let units: Vec<u16> = self.units().into_iter().take_while(|u| *u != 0).collect();
        Some(String::from_utf16_lossy(&units))
    }
    fn as_multi(&self) -> Option<Vec<String>> {
        if self.kind == REG_SZ {
            return self.as_string().map(|s| vec![s]);
        }
        if self.kind != REG_MULTI_SZ {
            return None;
        }
        Some(self.units().split(|u| *u == 0).take_while(|s| !s.is_empty()).map(String::from_utf16_lossy).collect())
    }
}

/// A screen, as Windows drives it now and at its best.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Display {
    /// "\\.\DISPLAY1", for ids.
    pub device: String,
    /// What to call it ("Generic PnP Monitor" isn't much, but it's what Windows knows).
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub hz: u32,
    /// The fastest refresh rate it offers at the current resolution.
    pub best_hz: u32,
    pub max_width: u32,
    pub max_height: u32,
}

/// Everything the checks look at, so tests can hand them a made-up PC.
pub(crate) trait Machine {
    fn raw(&self, hive: Hive, key: &str, name: &str) -> Option<Raw>;
    /// Every value of a key: (name, value).
    fn raw_values(&self, hive: Hive, key: &str) -> Vec<(String, Raw)>;
    fn subkeys(&self, hive: Hive, key: &str) -> Vec<String>;
    fn exists(&self, path: &Path) -> bool;
    fn env(&self, name: &str) -> Option<String>;
    fn displays(&self) -> Vec<Display>;
    /// The hosts file's text.
    fn hosts(&self) -> Option<String>;
    /// Is something on this PC listening on this TCP port?
    fn listening(&self, port: u16) -> bool;
    /// "C:\".
    fn system_drive(&self) -> String;
    /// A small settings file's text.
    fn read_text(&self, _path: &Path) -> Option<String> {
        None
    }
    /// The names in a folder.
    fn list_dir(&self, _path: &Path) -> Vec<String> {
        Vec::new()
    }
    /// A file's size in bytes.
    fn size(&self, _path: &Path) -> Option<u64> {
        None
    }
    /// A readable executable's major file version; a bare file or inert app alias has none.
    fn executable_major(&self, _path: &Path) -> Option<u32> {
        None
    }
    /// PowerShell MSIX folders registered for this user, without launching a shell.
    fn powershell_packages(&self) -> Vec<PathBuf> {
        Vec::new()
    }
}

/// Changing values, for fixes. Separate from reading so checks can never write.
pub(crate) trait Writer {
    fn set(&self, hive: Hive, key: &str, name: &str, value: &Raw) -> Result<(), String>;
    fn delete(&self, hive: Hive, key: &str, name: &str) -> Result<(), String>;
}

fn dword(m: &dyn Machine, hive: Hive, key: &str, name: &str) -> Option<u32> {
    m.raw(hive, key, name)?.as_dword()
}
fn string(m: &dyn Machine, hive: Hive, key: &str, name: &str) -> Option<String> {
    m.raw(hive, key, name)?.as_string()
}
/// A number stored as text ("5000"), the way Control Panel keeps timeouts.
fn number(m: &dyn Machine, hive: Hive, key: &str, name: &str) -> Option<u32> {
    match m.raw(hive, key, name)? {
        r if r.kind == REG_DWORD => r.as_dword(),
        r => r.as_string()?.trim().parse().ok(),
    }
}
fn text_values(m: &dyn Machine, hive: Hive, key: &str) -> Vec<(String, String)> {
    m.raw_values(hive, key).into_iter().filter_map(|(n, r)| Some((n, r.as_string()?))).collect()
}

const MEM: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management";
const PREFETCH: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management\PrefetchParameters";
const PRIORITY: &str = r"SYSTEM\CurrentControlSet\Control\PriorityControl";
const CONTROL: &str = r"SYSTEM\CurrentControlSet\Control";
const SERVICES: &str = r"SYSTEM\CurrentControlSet\Services";
const UAC: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System";
const UPDATE_POLICY: &str = r"SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU";
const DESKTOP: &str = r"Control Panel\Desktop";
const VISUAL_FX: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\VisualEffects";
const INTERNET: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";
const MULTIMEDIA: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Multimedia\SystemProfile";
const INTERFACES: &str = r"SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces";
const USER_ENV: &str = "Environment";
const SYSTEM_ENV: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
const LAYERS: &str = r"Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers";
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const CONSOLE: &str = "Console";
const CONSOLE_STARTUP: &str = r"Console\%%Startup";
const CMD_PROCESSOR: &str = r"Software\Microsoft\Command Processor";
const NT_VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
const NLS_CODEPAGE: &str = r"SYSTEM\CurrentControlSet\Control\Nls\CodePage";
/// Windows Terminal's registration as the default terminal (from its own manifest).
const WT_CONSOLE: &str = "{2EACA947-7F5F-4CFA-BA87-8F7FBEEFBE69}";
const WT_TERMINAL: &str = "{E12CFF52-A866-4C77-9A90-F570A7AA2C6B}";
const CONHOST: &str = "{B23D10C0-E52E-411E-9D5B-C09FDF709C7D}";
/// Console fonts that can't draw much beyond plain English.
const WEAK_FONTS: &[&str] = &["lucida console", "terminal", "raster fonts", "fixedsys", "courier new"];
const RUN_32: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run";

const ENV_EDITOR: &str = "rundll32.exe sysdm.cpl,EditEnvironmentVariables";
const PERFORMANCE: &str = "SystemPropertiesPerformance.exe";

/// Windows' own settings programs (in System32) that a finding's button may open: the ones the
/// checkups name, and no others. A finding that names one that isn't here has a button that
/// does nothing, so a test reads the checkups for every name.
pub const PROGRAMS: &[&str] = &[
    ENV_EDITOR,
    "SystemPropertiesPerformance.exe",
    "SystemPropertiesAdvanced.exe",
    "UserAccountControlSettings.exe",
    "cttune.exe",
    "eventvwr.exe",
    "dfrgui.exe",
    "devmgmt.msc",
    "services.msc",
];

/// The program in System32, and what to hand it, that one of the names above stands for.
/// Nothing for a name that isn't there: only mom's own list opens anything.
pub fn program(name: &str) -> Option<(&'static str, &'static str)> {
    let known = PROGRAMS.iter().find(|p| **p == name)?;
    Some(known.split_once(' ').unwrap_or((known, "")))
}
const AFTER_RESTART: &str = "Takes effect after a restart.";
const AFTER_SIGN_IN: &str = "Takes effect the next time you sign in.";

fn finding(id: &str, area: &'static str, level: Level, title: String, now: String, default: &str, why: &str) -> Finding {
    Finding { id: id.into(), area, level, title, now, default: default.into(), why: why.into(), fix: None, settings: None, command: None }
}

impl Finding {
    fn fix(mut self, label: &str, admin: bool, after: Option<&str>) -> Finding {
        self.fix = Some(Fix { label: label.into(), admin, undo: !admin, after: after.map(String::from) });
        self
    }
    fn settings(mut self, s: &str) -> Finding {
        self.settings = Some(s.into());
        self
    }
    fn command(mut self, c: &str) -> Finding {
        self.command = Some(c.into());
        self
    }
}

// ------------------------------------------------------------------ the checks

/// PowerShell and cmd: where they open, how they draw, what they speak.
fn terminal(m: &dyn Machine, out: &mut Vec<Finding>) {
    let local = m.env("LOCALAPPDATA").unwrap_or_default();
    let wt = !local.is_empty() && m.exists(&Path::new(&local).join(r"Microsoft\WindowsApps\wt.exe"));
    let delegation = string(m, CU, CONSOLE_STARTUP, "DelegationConsole").unwrap_or_default().to_ascii_uppercase();
    let forced = delegation == CONHOST;
    // "Let Windows decide" means Windows Terminal from Windows 11 22H2 (build 22621) on, the old
    // console window before it.
    let build: Option<u32> = string(m, LM, NT_VERSION, "CurrentBuildNumber").and_then(|b| b.trim().parse().ok());
    let decides_old = !delegation.starts_with("{") || delegation == "{00000000-0000-0000-0000-000000000000}";
    let old_console = !wt || forced || (decides_old && build.is_some_and(|b| b < 22621));
    // Only people who use PowerShell hear about it (its history file is there).
    let appdata = m.env("APPDATA").unwrap_or_default();
    let history = Path::new(&appdata).join(r"Microsoft\Windows\PowerShell\PSReadLine\ConsoleHost_history.txt");
    let uses_shell = !appdata.is_empty() && m.exists(&history);
    if wt && old_console {
        out.push(
            finding(
                "terminal-default",
                "Terminal",
                if forced { Level::Fix } else { Level::Check },
                "PowerShell and cmd can open in the old console window".into(),
                if forced { "The old console window (Windows Console Host)".into() } else { "Let Windows decide".into() },
                "Windows Terminal",
                "The old window draws slowly and can't show many symbols, so long Claude Code chats crawl and some text comes out wrong or cut off. Windows Terminal is installed, and much faster.",
            )
            .fix("Use Windows Terminal", false, Some("Windows you open from now on use it."))
            .settings("ms-settings:developers"),
        );
    }

    let windir = m.env("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    let good = if m.exists(&Path::new(&windir).join(r"Fonts\CascadiaMono.ttf")) { "Cascadia Mono" } else { "Consolas" };
    let weak: Vec<(String, String)> = console_keys(m)
        .into_iter()
        .filter(|_| uses_shell && old_console)
        .filter_map(|k| string(m, CU, &k, "FaceName").filter(|f| WEAK_FONTS.contains(&f.to_ascii_lowercase().as_str())).map(|f| (k, f)))
        .collect();
    if !weak.is_empty() {
        let mut faces: Vec<&str> = weak.iter().map(|(_, f)| f.as_str()).collect();
        faces.sort_unstable();
        faces.dedup();
        out.push(
            finding(
                "console-font",
                "Terminal",
                Level::Check,
                format!("The console window uses {}", faces.join(" and ")),
                format!("{} in {}", faces.join(", "), plural_windows(weak.len())),
                good,
                "An old font that can't draw many of the symbols today's tools print (Claude Code's boxes and dots, emoji), so they show up as boxes or push the layout out of line.",
            )
            .fix(&format!("Use {good}"), false, Some("Windows you open from now on use it.")),
        );
    }

    let oem = string(m, LM, NLS_CODEPAGE, "OEMCP").unwrap_or_default();
    let cp = dword(m, CU, CONSOLE, "CodePage");
    if uses_shell && old_console && oem != "65001" && cp != Some(65001) {
        out.push(
            finding(
                "console-utf8",
                "Terminal",
                Level::Check,
                "The console doesn't speak UTF-8".into(),
                format!("Code page {}", cp.map(|c| c.to_string()).unwrap_or(oem)),
                "UTF-8 (65001)",
                "Anything a program prints beyond plain English (accents, symbols, emoji) can come out garbled. New console windows can speak UTF-8; a few very old batch files prefer the old code page.",
            )
            .fix("Switch to UTF-8", false, Some("Windows you open from now on use it.")),
        );
    }

    let program_files = m.env("ProgramFiles").unwrap_or_else(|| r"C:\Program Files".into());
    let pwsh = powershell_7(m, &program_files, &local);
    let profile = m.env("USERPROFILE").unwrap_or_default();
    let readline = [Path::new(&program_files).join(r"WindowsPowerShell\Modules\PSReadLine"), Path::new(&profile).join(r"Documents\WindowsPowerShell\Modules\PSReadLine")]
        .iter()
        .flat_map(|d| m.list_dir(d))
        .filter_map(|v| version(&v))
        .max();
    let old_readline = readline.as_ref().is_some_and(|v| *v < vec![2, 2]);
    let shown = |v: &Vec<u64>| v.iter().map(u64::to_string).collect::<Vec<_>>().join(".");
    if uses_shell && !pwsh {
        out.push(
            finding(
                "powershell-7",
                "Terminal",
                Level::Check,
                "PowerShell 7 wasn't found".into(),
                match &readline {
                    Some(v) => format!("Windows PowerShell 5.1, line editor {}", shown(v)),
                    None => "Windows PowerShell 5.1".into(),
                },
                "PowerShell 7 alongside it",
                "PowerShell 7 includes a newer line editor and uses UTF-8 by default for text output. It installs alongside Windows PowerShell 5.1; existing windows keep using their current shell.",
            )
            .command("winget install --id Microsoft.PowerShell --exact --source winget"),
        );
    } else if uses_shell && old_readline {
        out.push(
            finding(
                "psreadline-old",
                "Terminal",
                Level::Check,
                "Windows PowerShell's line editor is out of date".into(),
                readline.as_ref().map(|v| format!("PSReadLine {}", shown(v))).unwrap_or_default(),
                "PSReadLine 2.3 or newer",
                "The version Windows ships has known bugs: lines that wrap are drawn wrong and long sessions slow down.",
            )
            .command("Install-Module PSReadLine -Scope CurrentUser -Force -SkipPublisherCheck"),
        );
    }

    // Windows Terminal's own settings.
    let wt_settings = Path::new(&local).join(r"Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json");
    if let Some(json) = m.read_text(&wt_settings).and_then(|t| serde_json::from_str::<serde_json::Value>(&strip_comments(&t)).ok()) {
        let truthy = |k: &str| json.get(k).and_then(serde_json::Value::as_bool) == Some(true);
        if truthy("experimental.rendering.forceFullRepaint") || truthy("experimental.rendering.software") || json.get("rendering.graphicsAPI").and_then(serde_json::Value::as_str) == Some("warp") {
            out.push(finding(
                "terminal-rendering",
                "Terminal",
                Level::Check,
                "Windows Terminal is set to draw the slow way".into(),
                "Full repaints, or drawing without the graphics card".into(),
                "Off",
                "A troubleshooting setting: every frame is drawn from scratch, or on the processor. Fine for chasing a bug, slow for everything else. Settings > Rendering in Windows Terminal.",
            ));
        }
        let mut sizes: Vec<u64> = json.pointer("/profiles/defaults/historySize").and_then(serde_json::Value::as_u64).into_iter().collect();
        if let Some(list) = json.pointer("/profiles/list").and_then(serde_json::Value::as_array) {
            sizes.extend(list.iter().filter_map(|p| p.get("historySize").and_then(serde_json::Value::as_u64)));
        }
        if let Some(big) = sizes.into_iter().max().filter(|n| *n > 100_000) {
            out.push(finding(
                "terminal-history",
                "Terminal",
                Level::Check,
                "Windows Terminal keeps a very long history".into(),
                format!("{big} lines per tab"),
                "9,001 lines",
                "Every line kept costs memory and time, and long sessions get slower and slower. A few thousand lines is plenty.",
            ));
        }
    }

    // PowerShell's saved history, read at every start.
    if let Some(len) = m.size(&history).filter(|n| *n > 5 * 1024 * 1024) {
        out.push(finding(
            "powershell-history",
            "Terminal",
            Level::Check,
            "PowerShell's history file is huge".into(),
            format!("{} MB", len / (1024 * 1024)),
            "A few hundred KB",
            "PowerShell reads all of it every time a window opens. Trimming it (or setting MaximumHistoryCount) makes new windows open faster.",
        ));
    }

    // Something run every time cmd starts.
    if let Some(cmd) = string(m, CU, CMD_PROCESSOR, "AutoRun").filter(|c| !c.trim().is_empty()) {
        out.push(
            finding(
                "cmd-autorun-user",
                "Terminal",
                Level::Check,
                "Something runs every time cmd starts".into(),
                cmd,
                "Nothing",
                "Usually conda's or a similar tool's set-up. It slows every cmd window and can break scripts that run cmd behind the scenes.",
            )
            .fix("Stop running it", false, None),
        );
    }
}

/// Installed MSI/MSIX copies and accessible portable copies, without starting PowerShell.
fn powershell_7(m: &dyn Machine, program_files: &str, local: &str) -> bool {
    let mut candidates = vec![Path::new(program_files).join(r"PowerShell\7\pwsh.exe")];
    if !local.is_empty() {
        candidates.push(Path::new(local).join(r"Microsoft\WindowsApps\pwsh.exe"));
    }
    candidates.extend(m.powershell_packages().into_iter().map(|p| p.join("pwsh.exe")));
    // A portable installation is usable too. Never search the working directory or a share.
    for folder in m.env("PATH").unwrap_or_default().split(';') {
        let folder = folder.trim().trim_matches('"');
        let b = folder.as_bytes();
        if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/') && m.exists(Path::new(&format!("{}:\\", b[0] as char))) {
            candidates.push(Path::new(folder).join("pwsh.exe"));
        }
    }
    candidates.into_iter().any(|p| m.executable_major(&p).is_some_and(|major| major >= 7))
}

/// HKCU\Console and the per-program keys under it (a shortcut's own settings), not the default
/// terminal's.
fn console_keys(m: &dyn Machine) -> Vec<String> {
    std::iter::once(CONSOLE.to_string())
        .chain(m.subkeys(CU, CONSOLE).into_iter().filter(|k| !k.starts_with("%%")).map(|k| format!(r"{CONSOLE}\{k}")))
        .collect()
}

fn plural_windows(n: usize) -> String {
    if n == 1 { "1 console setting".into() } else { format!("{n} console settings") }
}

/// "2.0.0" -> [2, 0, 0].
fn version(s: &str) -> Option<Vec<u64>> {
    s.split('.').map(|p| p.parse().ok()).collect::<Option<Vec<u64>>>().filter(|v| !v.is_empty())
}

/// Windows Terminal allows // comments in its settings; JSON doesn't.
fn strip_comments(text: &str) -> String {
    text.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
}

fn memory(m: &dyn Machine, out: &mut Vec<Finding>) {
    let system = m.system_drive().chars().next().unwrap_or('C').to_ascii_uppercase();
    if let Some(files) = m.raw(LM, MEM, "PagingFiles").and_then(|r| r.as_multi()) {
        // "C:\pagefile.sys 4096 8192" (fixed), "C:\pagefile.sys 0 0" (Windows sizes it on C:),
        // "?:\pagefile.sys" (Windows manages it everywhere).
        let files: Vec<(char, Option<(u64, u64)>)> = files
            .iter()
            .filter_map(|f| {
                let mut parts = f.split_whitespace();
                let drive = parts.next()?.chars().next()?.to_ascii_uppercase();
                let size = match (parts.next().and_then(|n| n.parse().ok()), parts.next().and_then(|n| n.parse().ok())) {
                    (Some(0), Some(0)) | (None, _) | (_, None) => None,
                    (Some(lo), Some(hi)) => Some((lo, hi)),
                };
                Some((drive, size))
            })
            .collect();
        let fix = |f: Finding| f.fix("Let Windows manage it", true, Some(AFTER_RESTART)).settings(PERFORMANCE);
        if files.is_empty() {
            out.push(fix(finding(
                "pagefile",
                "Memory",
                Level::Check,
                "There's no page file".into(),
                "None".into(),
                "Managed by Windows",
                "When memory runs out, programs crash instead of slowing down, and Windows can't save what went wrong.",
            )));
        } else if !files.iter().any(|(d, _)| *d == system || *d == '?') {
            let on: Vec<String> = files.iter().map(|(d, _)| format!("{d}:")).collect();
            out.push(fix(finding(
                "pagefile",
                "Memory",
                Level::Check,
                format!("The page file isn't on {system}:"),
                format!("Only on {}", on.join(" and ")),
                "Managed by Windows",
                "Windows needs one on its own drive to save what went wrong after a crash, and an older, slower drive slows everything down when memory is tight.",
            )));
        } else if let Some((d, (lo, hi))) = files.iter().find_map(|(d, s)| s.map(|s| (*d, s))) {
            let mb = |n: u64| crate::util::human_count(n);
            let now = if lo == hi { format!("Fixed at {} MB on {d}:", mb(lo)) } else { format!("{} to {} MB on {d}:", mb(lo), mb(hi)) };
            out.push(fix(finding(
                "pagefile",
                "Memory",
                Level::Check,
                "The page file has a fixed size".into(),
                now,
                "Managed by Windows",
                "A size picked years ago can run out now that apps use more memory, and then programs crash. Windows grows it as needed.",
            )));
        }
    }
    let on = |key: &str, name: &str| dword(m, LM, key, name) == Some(1);
    if on(MEM, "ClearPageFileAtShutdown") {
        out.push(
            finding(
                "clear-pagefile",
                "Memory",
                Level::Check,
                "Windows wipes the page file at every shutdown".into(),
                "On".into(),
                "Off",
                "Every shutdown and restart takes longer. Some workplaces want it; at home it's rarely worth it.",
            )
            .fix("Turn it off", true, Some(AFTER_RESTART)),
        );
    }
    if on(MEM, "DisablePagingExecutive") {
        out.push(
            finding(
                "paging-executive",
                "Memory",
                Level::Check,
                "Windows is told to keep all its own code in memory".into(),
                "On".into(),
                "Off",
                "An old tweak from the days of 512 MB. Today it does nothing useful and takes memory from your apps.",
            )
            .fix("Put back Windows' default", true, Some(AFTER_RESTART)),
        );
    }
    if on(MEM, "LargeSystemCache") {
        out.push(
            finding(
                "large-system-cache",
                "Memory",
                Level::Check,
                "Windows favours its file cache over your apps".into(),
                "On (a file server setting)".into(),
                "Off",
                "Meant for file servers. On a PC it can push your programs out of memory and make them stutter.",
            )
            .fix("Put back Windows' default", true, Some(AFTER_RESTART)),
        );
    }
    let myths: Vec<&str> = MYTHS.iter().filter(|(key, name)| m.raw(LM, key, name).is_some()).map(|(_, name)| *name).collect();
    if !myths.is_empty() {
        out.push(
            finding(
                "memory-myths",
                "Memory",
                Level::Check,
                "Old speed-up tweaks that do nothing".into(),
                myths.join(", "),
                "Not set",
                "Tweak guides from the XP days set these. Windows 10 and 11 ignore them, so they're just clutter.",
            )
            .fix("Remove them", true, None),
        );
    }
    let prefetcher = dword(m, LM, PREFETCH, "EnablePrefetcher") == Some(0);
    let superfetch = dword(m, LM, PREFETCH, "EnableSuperfetch") == Some(0);
    if prefetcher || superfetch {
        let now = match (prefetcher, superfetch) {
            (true, true) => "Prefetch and Superfetch off",
            (true, false) => "Prefetch off",
            _ => "Superfetch off",
        };
        out.push(
            finding(
                "prefetch-off",
                "Memory",
                Level::Check,
                "Windows doesn't prepare your apps to start".into(),
                now.into(),
                "On",
                "Windows uses it to boot and open your apps faster, on SSDs too. Turning it off was an old SSD myth.",
            )
            .fix("Turn it back on", true, Some(AFTER_RESTART)),
        );
    }
}

/// Values tweak guides set that Windows 10 and 11 ignore.
const MYTHS: &[(&str, &str)] = &[(MEM, "SecondLevelDataCache"), (MEM, "IoPageLockLimit"), (PRIORITY, "IRQ8Priority")];

struct Service {
    name: &'static str,
    label: &'static str,
    /// How Windows ships it: 2 starts with Windows, 3 when something needs it.
    start: u32,
    level: Level,
    why: &'static str,
    /// Mom may turn it back on.
    fix: bool,
}

const SERVICES_WATCHED: &[Service] = &[
    Service {
        name: "wuauserv",
        label: "Windows Update",
        start: 3,
        level: Level::Fix,
        why: "You're not getting security fixes.",
        fix: true,
    },
    Service {
        name: "UsoSvc",
        label: "Update Orchestrator",
        start: 2,
        level: Level::Fix,
        why: "Windows Update can't install anything without it, security fixes included.",
        fix: true,
    },
    Service {
        name: "AudioSrv",
        label: "Windows Audio",
        start: 2,
        level: Level::Fix,
        why: "There's no sound while it's off.",
        fix: false,
    },
    Service {
        name: "SysMain",
        label: "SysMain (Superfetch)",
        start: 2,
        level: Level::Check,
        why: "It preloads the apps you use so they open faster. Turning it off was common advice for SSDs, but it helps on them too.",
        fix: true,
    },
    Service {
        name: "WSearch",
        label: "Windows Search",
        start: 2,
        level: Level::Check,
        why: "Searching the Start menu, File Explorer and Outlook gets slow or finds nothing.",
        fix: true,
    },
    Service {
        name: "BITS",
        label: "Background downloads (BITS)",
        start: 3,
        level: Level::Check,
        why: "Windows Update, the Store and some apps download through it, and stall while it's off.",
        fix: true,
    },
    Service {
        name: "Themes",
        label: "Themes",
        start: 2,
        level: Level::Check,
        why: "Windows falls back to a plain look, and some apps draw oddly.",
        fix: false,
    },
    Service {
        name: "Spooler",
        label: "Print Spooler",
        start: 2,
        level: Level::Info,
        why: "Fine if you never print; some turn it off for security. Printing won't work while it's off.",
        fix: false,
    },
];

fn services(m: &dyn Machine, out: &mut Vec<Finding>) {
    for s in SERVICES_WATCHED {
        if dword(m, LM, &format!(r"{SERVICES}\{}", s.name), "Start") != Some(4) {
            continue;
        }
        let default = if s.start == 2 { "Starts with Windows" } else { "Starts when needed" };
        let mut f = finding(&format!("service-off:{}", s.name), "Services", s.level, format!("{} is turned off", s.label), "Disabled".into(), default, s.why)
            .settings("services.msc");
        if s.fix {
            f = f.fix("Turn it back on", true, Some(AFTER_RESTART));
        }
        out.push(f);
    }
}

fn safety(m: &dyn Machine, out: &mut Vec<Finding>) {
    let uac_off = dword(m, LM, UAC, "EnableLUA") == Some(0);
    if uac_off {
        out.push(
            finding(
                "uac-off",
                "Safety",
                Level::Fix,
                "User Account Control is off".into(),
                "Off".into(),
                "On",
                "Every program runs with full control of your PC, so anything you open can change anything. Windows asks first by default.",
            )
            .settings("UserAccountControlSettings.exe"),
        );
    } else if dword(m, LM, UAC, "ConsentPromptBehaviorAdmin") == Some(0) {
        out.push(
            finding(
                "uac-silent",
                "Safety",
                Level::Check,
                "Apps become administrators without asking".into(),
                "Never asks".into(),
                "Asks first",
                "Anything you run can quietly give itself full control. Windows asks before that by default.",
            )
            .settings("UserAccountControlSettings.exe"),
        );
    }
    if dword(m, LM, UPDATE_POLICY, "NoAutoUpdate") == Some(1) {
        out.push(
            finding(
                "update-policy-off",
                "Safety",
                Level::Fix,
                "A policy turned automatic updates off".into(),
                "Off (by policy)".into(),
                "On",
                "Security fixes don't install by themselves. Privacy and tweak tools often set this.",
            )
            .fix("Remove the policy", true, Some(AFTER_RESTART))
            .settings("ms-settings:windowsupdate"),
        );
    }
}

fn shutdown(m: &dyn Machine, out: &mut Vec<Finding>) {
    let seconds = |ms: u32| {
        let s = ms as f64 / 1000.0;
        if s == 1.0 {
            "1 second".to_string()
        } else {
            format!("{} seconds", (s * 10.0).round() / 10.0)
        }
    };
    if string(m, CU, DESKTOP, "AutoEndTasks").as_deref().map(str::trim) == Some("1") {
        out.push(
            finding(
                "auto-end-tasks",
                "Shutdown",
                Level::Check,
                "Apps are closed without asking you to save".into(),
                "On".into(),
                "Off",
                "At shutdown, apps with unsaved work are closed anyway, and the work is lost.",
            )
            .fix("Put back Windows' default", false, Some(AFTER_SIGN_IN)),
        );
    }
    if let Some(ms) = number(m, CU, DESKTOP, "WaitToKillAppTimeout").filter(|ms| *ms < 5000) {
        out.push(
            finding(
                "app-shutdown-wait",
                "Shutdown",
                Level::Check,
                "Apps get very little time to close at shutdown".into(),
                seconds(ms),
                "Not set (Windows decides)",
                "Apps that are still saving can be cut off, and lose your work.",
            )
            .fix("Put back Windows' default", false, Some(AFTER_SIGN_IN)),
        );
    }
    if let Some(ms) = number(m, CU, DESKTOP, "HungAppTimeout").filter(|ms| *ms < 3000) {
        out.push(
            finding(
                "hung-app-timeout",
                "Shutdown",
                Level::Check,
                "Windows gives up on busy apps quickly".into(),
                seconds(ms),
                "5 seconds",
                "Apps that are busy for a moment get called not responding, or are closed at shutdown before they finish.",
            )
            .fix("Put back Windows' default", false, Some(AFTER_SIGN_IN)),
        );
    }
    if let Some(ms) = number(m, LM, CONTROL, "WaitToKillServiceTimeout") {
        let f = |title: &str, why: &str| {
            finding("service-shutdown-wait", "Shutdown", Level::Check, title.into(), seconds(ms), "5 seconds", why)
                .fix("Put back Windows' default", true, Some(AFTER_RESTART))
        };
        if ms < 5000 {
            out.push(f(
                "Services get very little time to stop at shutdown",
                "Services that are still writing can be cut off, which can damage what they were saving.",
            ));
        } else if ms > 20_000 {
            out.push(f("Shutdowns wait a long time for services", "One stuck service holds up every shutdown for that long."));
        }
    }
}

fn display(m: &dyn Machine, out: &mut Vec<Finding>) {
    for d in m.displays() {
        let id = d.device.trim_start_matches(r"\\.\").to_ascii_lowercase();
        if d.best_hz >= d.hz + 5 {
            out.push(
                finding(
                    &format!("refresh:{id}"),
                    "Display",
                    Level::Check,
                    format!("{} runs at {} Hz but can do {} Hz", d.name, d.hz, d.best_hz),
                    format!("{}×{} at {} Hz", d.width, d.height, d.hz),
                    &format!("{} Hz", d.best_hz),
                    "Everything on screen moves less smoothly than it could. Often left over from an old screen or cable.",
                )
                .settings("ms-settings:display-advanced"),
            );
        }
        if (d.width as u64 * d.height as u64) < (d.max_width as u64 * d.max_height as u64) {
            out.push(
                finding(
                    &format!("resolution:{id}"),
                    "Display",
                    Level::Info,
                    format!("{} isn't at its full resolution", d.name),
                    format!("{}×{}", d.width, d.height),
                    &format!("{}×{}", d.max_width, d.max_height),
                    "Text and pictures look softer than they could. Fine if you chose it for bigger text, though scaling does that more sharply.",
                )
                .settings("ms-settings:display"),
            );
        }
    }
    let custom = dword(m, CU, DESKTOP, "Win8DpiScaling") == Some(1);
    if let Some(dpi) = dword(m, CU, DESKTOP, "LogPixels").filter(|d| custom && *d != 96) {
        out.push(
            finding(
                "custom-scaling",
                "Display",
                Level::Check,
                "One custom scaling size is forced on every screen".into(),
                format!("{}%", dpi * 100 / 96),
                "Picked for each screen",
                "Some apps look blurry, and Windows can't adjust when you plug in another screen.",
            )
            .settings("ms-settings:display"),
        );
    }
    let smoothing = string(m, CU, DESKTOP, "FontSmoothing");
    let kind = dword(m, CU, DESKTOP, "FontSmoothingType");
    let text = if smoothing.as_deref().map(str::trim) == Some("0") {
        Some(("Off", "Text has jagged edges and is harder to read."))
    } else if kind == Some(1) {
        Some(("Standard, not ClearType", "Text looks rougher than it could on most screens."))
    } else {
        None
    };
    if let Some((now, why)) = text {
        out.push(
            finding("font-smoothing", "Display", Level::Check, "Text smoothing isn't ClearType".into(), now.into(), "ClearType", why)
                .fix("Turn ClearType back on", false, Some(AFTER_SIGN_IN))
                .settings("cttune.exe"),
        );
    }
    if dword(m, CU, VISUAL_FX, "VisualFXSetting") == Some(2) {
        out.push(
            finding(
                "visual-effects",
                "Display",
                Level::Info,
                "Windows' animations and shadows are off".into(),
                "Best performance".into(),
                "Let Windows choose",
                "A choice from slower PCs. Today's graphics handle the effects easily, but it's fine if you like it plain.",
            )
            .settings(PERFORMANCE),
        );
    }
}

fn network(m: &dyn Machine, out: &mut Vec<Finding>) {
    if dword(m, CU, INTERNET, "ProxyEnable") == Some(1) {
        let server = string(m, CU, INTERNET, "ProxyServer").unwrap_or_default();
        let (host, port) = proxy_target(&server);
        let local = matches!(host.to_ascii_lowercase().as_str(), "127.0.0.1" | "localhost" | "::1" | "[::1]");
        let (level, title, why) = if local && port.is_some_and(|p| !m.listening(p)) {
            (
                Level::Fix,
                "Windows' proxy points at a program that isn't running",
                "Apps that use Windows' proxy setting can't connect. Usually left behind by a debugging tool like Fiddler or Charles.",
            )
        } else if local {
            (Level::Info, "Windows' proxy points at a program on this PC", "Something here is reading your web traffic, a debugging tool probably. Fine while you use it.")
        } else {
            (Level::Info, "A proxy is set", "Fine if your network needs it. If it doesn't, apps go the long way round or fail to connect.")
        };
        out.push(
            finding("proxy", "Network", level, title.into(), if server.is_empty() { "On".into() } else { server.clone() }, "No proxy", why)
                .fix("Turn the proxy off", false, None)
                .settings("ms-settings:network-proxy"),
        );
    }
    let throttling = dword(m, LM, MULTIMEDIA, "NetworkThrottlingIndex") == Some(0xffff_ffff);
    let responsiveness = dword(m, LM, MULTIMEDIA, "SystemResponsiveness") == Some(0);
    if throttling || responsiveness {
        let set: Vec<&str> = [(throttling, "NetworkThrottlingIndex"), (responsiveness, "SystemResponsiveness")]
            .into_iter()
            .filter_map(|(on, name)| on.then_some(name))
            .collect();
        out.push(finding(
            "multimedia-tweaks",
            "Network",
            Level::Info,
            "Gaming-guide tweaks are set".into(),
            set.join(", "),
            "Not changed",
            "They do little on today's Windows. Harmless, but not what the guide promised.",
        ));
    }
    let nagle =
        m.subkeys(LM, INTERFACES).iter().filter(|i| ["TcpAckFrequency", "TCPNoDelay"].iter().any(|n| m.raw(LM, &format!(r"{INTERFACES}\{i}"), n).is_some())).count();
    if nagle > 0 {
        out.push(finding(
            "nagle-tweaks",
            "Network",
            Level::Info,
            "Old online-game network tweaks are set".into(),
            format!("On {}", crate::util::plural(nagle as u64, "network adapter")),
            "Not set",
            "Meant for games from years ago. They do little today.",
        ));
    }
    if let Some(hosts) = m.hosts() {
        let entries: Vec<(String, String)> = hosts
            .lines()
            .filter_map(|l| {
                let l = l.split('#').next()?.trim();
                let mut parts = l.split_whitespace();
                Some((parts.next()?.to_string(), parts.next()?.to_ascii_lowercase()))
            })
            .collect();
        let blocked = entries
            .iter()
            .filter(|(ip, host)| matches!(ip.as_str(), "0.0.0.0" | "127.0.0.1" | "::1") && UPDATE_HOSTS.iter().any(|u| host.contains(u)))
            .count();
        if blocked > 0 {
            out.push(finding(
                "hosts-blocks-updates",
                "Network",
                Level::Check,
                "The hosts file blocks Windows Update".into(),
                format!("{} blocked", crate::util::plural(blocked as u64, "update server")),
                "Nothing blocked",
                "Security fixes and Store apps can't download. Privacy tools often add these.",
            ));
        }
        if entries.len() > 50 {
            out.push(finding(
                "hosts-long",
                "Network",
                Level::Info,
                format!("The hosts file has {} entries", crate::util::human_count(entries.len() as u64)),
                format!("{} entries in drivers\\etc\\hosts", crate::util::human_count(entries.len() as u64)),
                "None",
                "Usually an ad or tracker blocklist. Fine if you added it on purpose.",
            ));
        }
    }
}

/// Servers Windows Update and the Store download from.
const UPDATE_HOSTS: &[&str] = &["windowsupdate", "update.microsoft.com", "delivery.mp.microsoft.com"];

/// "127.0.0.1:8888", or "http=127.0.0.1:8888;https=127.0.0.1:8888": the (first) host and port.
fn proxy_target(server: &str) -> (String, Option<u16>) {
    let first = server.split(';').find(|s| s.starts_with("http=")).or_else(|| server.split(';').next()).unwrap_or("");
    let target = first.split_once('=').map(|(_, t)| t).unwrap_or(first);
    let target = target.trim_start_matches("http://").trim_start_matches("https://").trim_end_matches('/');
    match target.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => (host.to_string(), port.parse().ok()),
        _ => (target.to_string(), None),
    }
}

fn leftovers(m: &dyn Machine, out: &mut Vec<Finding>) {
    let list = |items: &[String]| {
        let shown: Vec<&str> = items.iter().take(4).map(String::as_str).collect();
        match items.len() {
            n if n > 4 => format!("{} and {} more", shown.join(", "), n - 4),
            _ => shown.join(", "),
        }
    };
    // Folders on PATH that are gone.
    for (hive, key, id, whose) in [(CU, USER_ENV, "user-path-missing", "your"), (LM, SYSTEM_ENV, "system-path-missing", "Windows'")] {
        let Some(path) = string(m, hive, key, "Path") else { continue };
        let gone: Vec<String> = path.split(';').filter_map(|e| gone(m, e)).collect();
        if gone.is_empty() {
            continue;
        }
        // A missing folder right under a drive's root (C:\Python27) can be made by any program,
        // and Windows would then run what's in it instead of the real tools.
        let open = hive == LM && gone.iter().any(|g| Path::new(g).parent().is_some_and(|p| p.parent().is_none()));
        let (level, why) = if open {
            (
                Level::Fix,
                "Left by programs you uninstalled. Any program could create one of these folders at the root of a drive and have its files run instead of the real ones.",
            )
        } else {
            (Level::Check, "Left by programs you uninstalled. Every program search walks past them, and some tools trip over them.")
        };
        let admin = hive == LM;
        out.push(
            finding(id, "Paths", level, format!("{} in {whose} PATH {} gone", plural_folders(gone.len()), if gone.len() == 1 { "is" } else { "are" }), list(&gone), "Only folders that exist", why)
                .fix("Remove them from PATH", admin, Some(if admin { AFTER_RESTART } else { AFTER_SIGN_IN }))
                .settings(ENV_EDITOR),
        );
    }
    // Variables pointing at folders or files that are gone (JAVA_HOME and the like).
    let mut vars: Vec<String> = Vec::new();
    for (hive, key) in [(CU, USER_ENV), (LM, SYSTEM_ENV)] {
        for (name, value) in text_values(m, hive, key) {
            if SKIP_VARS.iter().any(|s| s.eq_ignore_ascii_case(&name)) || value.contains(';') {
                continue;
            }
            if let Some(g) = gone(m, &value) {
                vars.push(format!("{name} → {g}"));
            }
        }
    }
    if !vars.is_empty() {
        out.push(
            finding(
                "env-missing",
                "Paths",
                Level::Check,
                format!("{} {} at things that are gone", crate::util::plural(vars.len() as u64, "variable"), if vars.len() == 1 { "points" } else { "point" }),
                list(&vars),
                "Only what exists",
                "Left by programs you uninstalled. Tools that read them (Java, Android, build scripts) fail with confusing errors.",
            )
            .settings(ENV_EDITOR),
        );
    }
    // Compatibility settings and startup entries for programs that are gone. Yours are yours to
    // remove; the ones for every user of the PC need an administrator, so Windows asks first.
    let remove = |n: usize| if n == 1 { "Remove it" } else { "Remove them" };
    for (hive, id) in [(CU, "compat-orphans-user"), (LM, "compat-orphans-machine")] {
        let gone: Vec<String> = if hive == LM { machine_compat_orphans(m) } else { m.raw_values(hive, LAYERS).into_iter().filter_map(|(exe, _)| gone(m, &exe)).collect() };
        if gone.is_empty() {
            continue;
        }
        let mut f = finding(
            id,
            "Paths",
            Level::Check,
            format!("Compatibility settings for {} that {} gone", crate::util::plural(gone.len() as u64, "program"), if gone.len() == 1 { "is" } else { "are" }),
            list(&gone.iter().map(|g| crate::util::file_name(g).to_string()).collect::<Vec<_>>()),
            "None",
            "Leftovers from programs you uninstalled. Harmless clutter.",
        );
        f = f.fix(remove(gone.len()), hive == LM, None);
        out.push(f);
    }
    for (hive, id) in [(CU, "startup-orphans-user"), (LM, "startup-orphans-machine")] {
        let gone: Vec<String> = if hive == LM {
            machine_startup_orphans(m).into_iter().map(|(_, name)| name).collect()
        } else {
            text_values(m, CU, RUN).into_iter().filter_map(|(name, cmd)| run_target_gone(m, &cmd).map(|_| name)).collect()
        };
        if gone.is_empty() {
            continue;
        }
        let mut f = finding(
            id,
            "Startup",
            Level::Check,
            if gone.len() == 1 { "1 startup entry points at a program that's gone".into() } else { format!("{} startup entries point at programs that are gone", gone.len()) },
            list(&gone),
            "None",
            "Windows tries to start them at every sign-in and quietly fails. Left by programs you uninstalled.",
        );
        f = f.fix(remove(gone.len()), hive == LM, None);
        out.push(f);
    }
}

/// Variables that mean the same place for every user of the PC. A path that goes through any
/// other one (%USERPROFILE%, %APPDATA%...) is somewhere else for the administrator who'd do the
/// removing than for you, so mom doesn't judge it "gone" for the whole PC.
const SAME_FOR_EVERYONE: &[&str] = &[
    "ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "CommonProgramFiles", "CommonProgramFiles(x86)", "CommonProgramW6432", "ProgramData", "ALLUSERSPROFILE", "PUBLIC",
    "SystemRoot", "windir", "SystemDrive",
];

/// Does `text` mean the same thing whoever reads it: no %VARIABLE% but the ones every user shares?
fn same_for_everyone(text: &str) -> bool {
    text.split('%').skip(1).step_by(2).all(|var| SAME_FOR_EVERYONE.iter().any(|v| v.eq_ignore_ascii_case(var)))
}

/// Startup entries for every user (the two Run keys only administrators can change) whose
/// program is gone: (key, name).
fn machine_startup_orphans(m: &dyn Machine) -> Vec<(&'static str, String)> {
    [RUN, RUN_32]
        .into_iter()
        .flat_map(|key| text_values(m, LM, key).into_iter().map(move |(name, cmd)| (key, name, cmd)))
        .filter(|(_, _, cmd)| same_for_everyone(cmd) && run_target_gone(m, cmd).is_some())
        .map(|(key, name, _)| (key, name))
        .collect()
}

/// Compatibility settings for every user whose program is gone: the programs, by path.
fn machine_compat_orphans(m: &dyn Machine) -> Vec<String> {
    m.raw_values(LM, LAYERS).into_iter().map(|(exe, _)| exe).filter(|exe| same_for_everyone(exe) && gone(m, exe).is_some()).collect()
}

fn plural_folders(n: usize) -> String {
    if n == 1 {
        "1 folder".into()
    } else {
        format!("{n} folders")
    }
}

/// Variables that aren't a single path, or that Windows itself owns.
const SKIP_VARS: &[&str] = &["Path", "PATHEXT", "PSModulePath", "TEMP", "TMP", "windir", "ComSpec", "OS", "PROCESSOR_ARCHITECTURE", "NUMBER_OF_PROCESSORS"];

/// The program a startup command runs, if it's gone. Bare names (`rundll32.exe ...`) are found
/// on PATH, so they're never called missing.
fn run_target_gone(m: &dyn Machine, cmd: &str) -> Option<String> {
    let (exe, _) = crate::elevate::split_command(cmd)?;
    if !exe.contains('\\') && !exe.contains('/') {
        return None;
    }
    gone(m, &exe)
}

/// `entry`, with %VARIABLES% filled in, if it's an absolute path on a drive that's there but
/// the path itself isn't. Relative paths, network paths, unknown variables and drives that
/// aren't plugged in are never called gone.
fn gone(m: &dyn Machine, entry: &str) -> Option<String> {
    let entry = entry.trim().trim_matches('"');
    if entry.is_empty() {
        return None;
    }
    let path = expand(m, entry)?;
    let b = path.as_bytes();
    if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || !matches!(b[2], b'\\' | b'/') {
        return None;
    }
    let root = format!("{}:\\", (b[0] as char).to_ascii_uppercase());
    (m.exists(Path::new(&root)) && !m.exists(Path::new(&path))).then_some(path)
}

/// Fills in %VARIABLES%; None if one isn't set (so mom can't tell what it means).
fn expand(m: &dyn Machine, s: &str) -> Option<String> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let j = after.find('%')?;
        out.push_str(&m.env(&after[..j])?);
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    Some(out)
}

// ------------------------------------------------------------------ fixes

/// One value to change: set it, or (None) delete it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Change {
    hive: Hive,
    key: String,
    name: String,
    to: Option<Raw>,
}

fn set(hive: Hive, key: &str, name: &str, to: Raw) -> Change {
    Change { hive, key: key.into(), name: name.into(), to: Some(to) }
}
fn delete(hive: Hive, key: &str, name: &str) -> Change {
    Change { hive, key: key.into(), name: name.into(), to: None }
}

/// The keys user-level fixes change (and undo may put back), nothing else.
const USER_KEYS: &[&str] = &[DESKTOP, INTERNET, USER_ENV, LAYERS, RUN, CMD_PROCESSOR];

/// What a fix to your own settings changes, worked out from the PC as it is now.
fn user_changes(m: &dyn Machine, id: &str) -> Option<Vec<Change>> {
    Some(match id {
        "auto-end-tasks" => vec![delete(CU, DESKTOP, "AutoEndTasks")],
        "app-shutdown-wait" => vec![delete(CU, DESKTOP, "WaitToKillAppTimeout")],
        "hung-app-timeout" => vec![delete(CU, DESKTOP, "HungAppTimeout")],
        "font-smoothing" => vec![set(CU, DESKTOP, "FontSmoothing", Raw::string("2")), set(CU, DESKTOP, "FontSmoothingType", Raw::dword(2))],
        "proxy" => vec![set(CU, INTERNET, "ProxyEnable", Raw::dword(0))],
        "user-path-missing" => vec![without_gone(m, CU, USER_ENV)?],
        "compat-orphans-user" => {
            m.raw_values(CU, LAYERS).into_iter().filter(|(exe, _)| gone(m, exe).is_some()).map(|(exe, _)| delete(CU, LAYERS, &exe)).collect()
        }
        "terminal-default" => vec![
            set(CU, CONSOLE_STARTUP, "DelegationConsole", Raw::string(WT_CONSOLE)),
            set(CU, CONSOLE_STARTUP, "DelegationTerminal", Raw::string(WT_TERMINAL)),
        ],
        "console-font" => {
            let windir = m.env("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
            let good = if m.exists(&Path::new(&windir).join(r"Fonts\CascadiaMono.ttf")) { "Cascadia Mono" } else { "Consolas" };
            console_keys(m)
                .into_iter()
                .filter(|k| string(m, CU, k, "FaceName").is_some_and(|f| WEAK_FONTS.contains(&f.to_ascii_lowercase().as_str())))
                .map(|k| set(CU, &k, "FaceName", Raw::string(good)))
                .collect()
        }
        "console-utf8" => vec![set(CU, CONSOLE, "CodePage", Raw::dword(65001))],
        "cmd-autorun-user" => vec![delete(CU, CMD_PROCESSOR, "AutoRun")],
        "startup-orphans-user" => {
            text_values(m, CU, RUN).into_iter().filter(|(_, cmd)| run_target_gone(m, cmd).is_some()).map(|(name, _)| delete(CU, RUN, &name)).collect()
        }
        _ => return None,
    })
}

/// What a fix that needs an administrator changes. Every value is written into mom; the only
/// thing read from the PC is which PATH folders are gone (from a key only administrators can
/// change).
fn admin_changes(m: &dyn Machine, id: &str) -> Option<Vec<Change>> {
    let service = |name: &str, start: u32| vec![set(LM, &format!(r"{SERVICES}\{name}"), "Start", Raw::dword(start))];
    Some(match id {
        "pagefile" => vec![set(LM, MEM, "PagingFiles", Raw::multi(&[r"?:\pagefile.sys"]))],
        "clear-pagefile" => vec![set(LM, MEM, "ClearPageFileAtShutdown", Raw::dword(0))],
        "paging-executive" => vec![set(LM, MEM, "DisablePagingExecutive", Raw::dword(0))],
        "large-system-cache" => vec![set(LM, MEM, "LargeSystemCache", Raw::dword(0))],
        "memory-myths" => MYTHS.iter().map(|(key, name)| delete(LM, key, name)).collect(),
        "prefetch-off" => vec![set(LM, PREFETCH, "EnablePrefetcher", Raw::dword(3)), set(LM, PREFETCH, "EnableSuperfetch", Raw::dword(3))],
        "service-shutdown-wait" => vec![set(LM, CONTROL, "WaitToKillServiceTimeout", Raw::string("5000"))],
        "update-policy-off" => vec![delete(LM, UPDATE_POLICY, "NoAutoUpdate")],
        "system-path-missing" => vec![without_gone(m, LM, SYSTEM_ENV)?],
        // Read from keys only administrators can change, and judged again by the administrator's mom.
        "startup-orphans-machine" => machine_startup_orphans(m).into_iter().map(|(key, name)| delete(LM, key, &name)).collect(),
        "compat-orphans-machine" => machine_compat_orphans(m).into_iter().map(|exe| delete(LM, LAYERS, &exe)).collect(),
        _ => {
            let name = id.strip_prefix("service-off:")?;
            let s = SERVICES_WATCHED.iter().find(|s| s.fix && s.name == name)?;
            service(s.name, s.start)
        }
    })
}

/// Can this finding only be fixed by an administrator? (What an elevated `mom` may do.)
pub fn is_admin_fix(id: &str) -> bool {
    matches!(
        id,
        "pagefile"
            | "clear-pagefile"
            | "paging-executive"
            | "large-system-cache"
            | "memory-myths"
            | "prefetch-off"
            | "service-shutdown-wait"
            | "update-policy-off"
            | "system-path-missing"
            | "startup-orphans-machine"
            | "compat-orphans-machine"
    ) || id.strip_prefix("service-off:").is_some_and(|n| SERVICES_WATCHED.iter().any(|s| s.fix && s.name == n))
}

/// PATH (in `key`) without the folders that are gone, keeping its order and type.
fn without_gone(m: &dyn Machine, hive: Hive, key: &str) -> Option<Change> {
    let raw = m.raw(hive, key, "Path")?;
    let path = raw.as_string()?;
    let kept: Vec<&str> = path.split(';').filter(|e| gone(m, e).is_none()).collect();
    let cleaned = kept.join(";");
    (cleaned != path).then(|| set(hive, key, "Path", Raw::text(raw.kind, &cleaned)))
}

/// What was there before a fix, so undo can put it back.
#[derive(Serialize, Deserialize)]
struct Backup {
    id: String,
    at: u64,
    values: Vec<Saved>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    hive: Hive,
    key: String,
    name: String,
    /// None: the value wasn't there.
    was: Option<Raw>,
}

fn backups(paths: &Paths) -> PathBuf {
    paths.backups_dir().join("settings")
}

fn file_id(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect()
}

/// Puts one of your own settings back the way Windows ships it, after backing it up.
pub fn fix(paths: &Paths, id: &str) -> Result<String, String> {
    if crate::elevate::is_elevated() {
        return Err("Run this as yourself, not as administrator: it changes your own settings.".into());
    }
    let m = real(paths);
    fix_with(paths, &m, &m, id)
}

fn fix_with(paths: &Paths, m: &dyn Machine, w: &dyn Writer, id: &str) -> Result<String, String> {
    let found = check_with(m).into_iter().find(|f| f.id == id).ok_or_else(|| "Nothing to fix there: it's already fine.".to_string())?;
    let Some(fix) = &found.fix else { return Err("Mom can't change that one. Its settings page can.".into()) };
    if fix.admin {
        return Err("That one needs an administrator.".into());
    }
    let changes = user_changes(m, id).filter(|c| !c.is_empty()).ok_or_else(|| "Nothing to fix there: it's already fine.".to_string())?;
    let backup = Backup {
        id: id.into(),
        at: crate::util::now_secs(),
        values: changes.iter().map(|c| Saved { hive: c.hive, key: c.key.clone(), name: c.name.clone(), was: m.raw(c.hive, &c.key, &c.name) }).collect(),
    };
    let file = backups(paths).join(format!("{}-{}.json", file_id(id), backup.at));
    let json = serde_json::to_string_pretty(&backup).map_err(|e| e.to_string())?;
    crate::util::write_atomic(&file, &json).map_err(|e| format!("Mom couldn't save a backup first, so nothing changed ({e})."))?;
    if let Err(e) = apply(w, &changes) {
        // Put back whatever went through before the failure.
        let _ = restore(w, &backup.values);
        return Err(format!("Windows didn't let mom change it ({e}). Nothing changed."));
    }
    let after = fix.after.as_deref().map(|a| format!(" {a}")).unwrap_or_default();
    Ok(format!("Fixed: {}.{after} You can undo it.", found.title))
}

/// Puts back what the last fix of `id` changed.
pub fn undo(paths: &Paths, id: &str) -> Result<String, String> {
    if crate::elevate::is_elevated() {
        return Err("Run this as yourself, not as administrator.".into());
    }
    let m = real(paths);
    undo_with(paths, &m, id)
}

fn undo_with(paths: &Paths, w: &dyn Writer, id: &str) -> Result<String, String> {
    let prefix = format!("{}-", file_id(id));
    let latest = std::fs::read_dir(backups(paths))
        .map_err(|_| "There's nothing to undo.".to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|p| {
            let stem = p.file_stem()?.to_str()?.to_string();
            let at: u64 = stem.strip_prefix(&prefix)?.parse().ok()?;
            Some((at, p))
        })
        .max_by_key(|(at, _)| *at)
        .ok_or_else(|| "There's nothing to undo.".to_string())?
        .1;
    let backup: Backup = serde_json::from_str(&std::fs::read_to_string(&latest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    // Only ever your own settings, in the keys fixes change.
    let allowed = |k: &str| USER_KEYS.contains(&k) || k == CONSOLE || k.starts_with(r"Console\");
    if backup.id != id || backup.values.iter().any(|v| v.hive != CU || !allowed(v.key.as_str())) {
        return Err("That backup isn't one mom made, so she won't use it.".into());
    }
    restore(w, &backup.values).map_err(|e| format!("Windows didn't let mom put it back ({e})."))?;
    let _ = std::fs::rename(&latest, latest.with_extension("undone"));
    Ok("Put back the way it was.".into())
}

fn apply(w: &dyn Writer, changes: &[Change]) -> Result<(), String> {
    for c in changes {
        match &c.to {
            Some(v) => w.set(c.hive, &c.key, &c.name, v)?,
            None => w.delete(c.hive, &c.key, &c.name)?,
        }
    }
    Ok(())
}

fn restore(w: &dyn Writer, saved: &[Saved]) -> Result<(), String> {
    for s in saved {
        match &s.was {
            Some(v) => w.set(s.hive, &s.key, &s.name, v)?,
            None => w.delete(s.hive, &s.key, &s.name)?,
        }
    }
    Ok(())
}

/// Puts back Windows' default for something only an administrator can change. Run by an
/// elevated `mom settings fix <id> --elevated`: refuses unless elevated, refuses anything not on
/// mom's list, only acts on what it finds itself, and writes nothing but the registry. What was
/// there before is in the message (the app records it beforehand with `record_admin_fix`).
pub fn fix_elevated(id: &str) -> Result<String, String> {
    if !crate::elevate::is_elevated() {
        return Err("That needs an administrator.".into());
    }
    if !is_admin_fix(id) {
        return Err(format!("{id} isn't something mom changes as administrator."));
    }
    let m = real_detect();
    fix_elevated_with(&m, &m, id)
}

fn fix_elevated_with(m: &dyn Machine, w: &dyn Writer, id: &str) -> Result<String, String> {
    let found = check_with(m).into_iter().find(|f| f.id == id).ok_or_else(|| "Nothing to fix there: it's already fine.".to_string())?;
    if !found.fix.as_ref().is_some_and(|f| f.admin) {
        return Err(format!("{id} isn't something mom changes as administrator."));
    }
    let changes = admin_changes(m, id).filter(|c| !c.is_empty()).ok_or_else(|| "Nothing to fix there: it's already fine.".to_string())?;
    let was: Vec<String> = changes.iter().map(|c| format!("{} was {}", c.name, describe(m.raw(c.hive, &c.key, &c.name).as_ref()))).collect();
    apply(w, &changes).map_err(|e| format!("Windows didn't let mom change it ({e})."))?;
    let after = found.fix.and_then(|f| f.after).map(|a| format!(" {a}")).unwrap_or_default();
    Ok(format!("Done.{after} Before: {}.", was.join("; ")))
}

/// Notes what an administrator fix is about to change, before Windows is asked, so it can be
/// put back by hand. Run as you (the elevated `mom` writes no files).
pub fn record_admin_fix(paths: &Paths, id: &str) -> Result<PathBuf, String> {
    if !is_admin_fix(id) {
        return Err(format!("{id} isn't something mom changes as administrator."));
    }
    let m = real(paths);
    let changes = admin_changes(&m, id).unwrap_or_default();
    let at = crate::util::now_secs();
    let backup = Backup {
        id: id.into(),
        at,
        values: changes.iter().map(|c| Saved { hive: c.hive, key: c.key.clone(), name: c.name.clone(), was: m.raw(c.hive, &c.key, &c.name) }).collect(),
    };
    // A different extension: undo never reads these.
    let file = backups(paths).join(format!("{}-{at}.admin-before", file_id(id)));
    let json = serde_json::to_string_pretty(&backup).map_err(|e| e.to_string())?;
    crate::util::write_atomic(&file, &json).map_err(|e| e.to_string())?;
    Ok(file)
}

/// A value for a message: "4", "\"C:\\pagefile.sys 4096 4096\"", "not set".
fn describe(v: Option<&Raw>) -> String {
    match v {
        None => "not set".into(),
        Some(r) => r
            .as_dword()
            .map(|n| n.to_string())
            .or_else(|| r.as_multi().map(|m| format!("\"{}\"", m.join(" | "))))
            .unwrap_or_else(|| "something else".into()),
    }
}

// ------------------------------------------------------------------ the real PC

fn real(paths: &Paths) -> Real {
    Real { hosts: paths.windows.join(r"System32\drivers\etc\hosts"), system_drive: paths.system_drive.to_string_lossy().to_string() }
}

/// For the elevated run: Windows' own folders from Windows, nothing from the environment.
fn real_detect() -> Real {
    real(&Paths::detect())
}

pub(crate) struct Real {
    hosts: PathBuf,
    system_drive: String,
}

impl Machine for Real {
    fn raw(&self, hive: Hive, key: &str, name: &str) -> Option<Raw> {
        os::raw(hive, key, name)
    }
    fn raw_values(&self, hive: Hive, key: &str) -> Vec<(String, Raw)> {
        os::values(hive, key)
    }
    fn subkeys(&self, hive: Hive, key: &str) -> Vec<String> {
        os::subkeys(hive, key)
    }
    fn exists(&self, path: &Path) -> bool {
        // A drive's root counts only if it's a fixed drive: a network drive that's gone away can
        // keep mom waiting, and a USB stick may just be unplugged.
        let s = path.to_string_lossy();
        if s.len() == 3 && s.ends_with(":\\") {
            return crate::util::fixed_drives().iter().any(|d| d.eq_ignore_ascii_case(&s));
        }
        std::fs::symlink_metadata(path).is_ok()
    }
    fn env(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
    fn displays(&self) -> Vec<Display> {
        os::displays()
    }
    fn hosts(&self) -> Option<String> {
        std::fs::read(&self.hosts).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
    }
    fn listening(&self, port: u16) -> bool {
        crate::ports::listeners().values().flatten().any(|l| l.port == port)
    }
    fn system_drive(&self) -> String {
        self.system_drive.clone()
    }
    fn read_text(&self, path: &Path) -> Option<String> {
        // Settings files are small; a huge one isn't a settings file.
        (self.size(path)? <= 4 * 1024 * 1024).then(|| std::fs::read(path).ok().map(|b| String::from_utf8_lossy(&b).into_owned()))?
    }
    fn list_dir(&self, path: &Path) -> Vec<String> {
        std::fs::read_dir(path).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default()
    }
    fn size(&self, path: &Path) -> Option<u64> {
        std::fs::metadata(path).ok().map(|m| m.len())
    }
    fn executable_major(&self, path: &Path) -> Option<u32> {
        os::executable_major(path)
    }
    fn powershell_packages(&self) -> Vec<PathBuf> {
        os::powershell_packages()
    }
}

impl Writer for Real {
    fn set(&self, hive: Hive, key: &str, name: &str, value: &Raw) -> Result<(), String> {
        os::set(hive, key, name, value)
    }
    fn delete(&self, hive: Hive, key: &str, name: &str) -> Result<(), String> {
        os::delete(hive, key, name)
    }
}

#[cfg(windows)]
mod os {
    use super::{Display, Hive, Raw};
    use crate::util::wide;
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY,
    };

    fn fixed_file_major(data: &[u8], returned: *const std::ffi::c_void, len: usize) -> Option<u32> {
        use windows_sys::Win32::Storage::FileSystem::VS_FIXEDFILEINFO;
        if returned.is_null() || len < std::mem::size_of::<VS_FIXEDFILEINFO>() {
            return None;
        }
        // VerQueryValue returns a location inside `data`. Use it only to locate a
        // checked slice of our allocation, never to dereference a foreign pointer.
        let offset = (returned as usize).checked_sub(data.as_ptr() as usize)?;
        let bytes = data.get(offset..offset.checked_add(len)?)?;
        let signature = u32::from_ne_bytes(bytes.get(..4)?.try_into().ok()?);
        let version_offset = std::mem::offset_of!(VS_FIXEDFILEINFO, dwFileVersionMS);
        let version = u32::from_ne_bytes(bytes.get(version_offset..version_offset + 4)?.try_into().ok()?);
        (signature == 0xfeef04bd).then_some(version >> 16)
    }

    /// Include the terminator so this checked slice can be passed back to Win32.
    fn terminated_utf16(value: &[u16]) -> Option<&[u16]> {
        let end = value.iter().position(|&c| c == 0)?;
        (end > 0).then(|| &value[..=end])
    }

    fn package_name(buffer: &[u16], returned: *const u16) -> Option<&[u16]> {
        if returned.is_null() {
            return None;
        }
        let offset = (returned as usize).checked_sub(buffer.as_ptr() as usize)?;
        if offset % std::mem::size_of::<u16>() != 0 {
            return None;
        }
        terminated_utf16(buffer.get(offset / std::mem::size_of::<u16>()..)?)
    }

    pub fn executable_major(path: &std::path::Path) -> Option<u32> {
        use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
        let name = wide(&path.to_string_lossy());
        // SAFETY: Windows fills owned buffers of the supplied size. The returned
        // location and length are checked against `data` before parsing its bytes.
        unsafe {
            let size = GetFileVersionInfoSizeW(name.as_ptr(), std::ptr::null_mut());
            if size == 0 || size > 1024 * 1024 {
                return None;
            }
            let mut data = vec![0u8; size as usize];
            if GetFileVersionInfoW(name.as_ptr(), 0, size, data.as_mut_ptr().cast()) == 0 {
                return None;
            }
            let mut ptr = std::ptr::null_mut();
            let mut len = 0;
            if VerQueryValueW(data.as_ptr().cast(), wide("\\").as_ptr(), &mut ptr, &mut len) == 0 {
                return None;
            }
            fixed_file_major(&data, ptr, len as usize)
        }
    }

    pub fn powershell_packages() -> Vec<std::path::PathBuf> {
        use windows_sys::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
        use windows_sys::Win32::Storage::Packaging::Appx::{GetPackagePathByFullName, GetPackagesByPackageFamily};
        let family = wide("Microsoft.PowerShell_8wekyb3d8bbwe");
        let mut out = Vec::new();
        // SAFETY: the package API fills owned arrays; string sizes are UTF-16 character
        // counts, including NUL. Returned names are checked against the live string
        // buffer before passing a pointer derived from its checked slice back to Win32.
        unsafe {
            let (mut count, mut len) = (0, 0);
            if GetPackagesByPackageFamily(family.as_ptr(), &mut count, std::ptr::null_mut(), &mut len, std::ptr::null_mut()) != ERROR_INSUFFICIENT_BUFFER || count == 0 || count > 256 || len == 0 || len > 65536 {
                return out;
            }
            let mut names = vec![std::ptr::null_mut(); count as usize];
            let mut buffer = vec![0u16; len as usize];
            if GetPackagesByPackageFamily(family.as_ptr(), &mut count, names.as_mut_ptr(), &mut len, buffer.as_mut_ptr()) != 0 {
                return out;
            }
            let (Some(names), Some(buffer)) = (names.get(..count as usize), buffer.get(..len as usize)) else {
                return out;
            };
            for &name in names {
                let Some(name) = package_name(buffer, name) else { continue };
                let mut len = 0;
                if GetPackagePathByFullName(name.as_ptr(), &mut len, std::ptr::null_mut()) != ERROR_INSUFFICIENT_BUFFER || len == 0 || len > 32768 {
                    continue;
                }
                let mut path = vec![0u16; len as usize];
                if GetPackagePathByFullName(name.as_ptr(), &mut len, path.as_mut_ptr()) == 0 {
                    if let Some(path) = path.get(..len as usize).and_then(terminated_utf16) {
                        out.push(std::path::PathBuf::from(String::from_utf16_lossy(&path[..path.len() - 1])));
                    }
                }
            }
        }
        out
    }

    #[cfg(test)]
    mod version_and_package_tests {
        use super::{fixed_file_major, package_name, terminated_utf16};
        use windows_sys::Win32::Storage::FileSystem::VS_FIXEDFILEINFO;

        #[test]
        fn version_info_is_read_from_a_bounded_slice_without_alignment_requirements() {
            let size = std::mem::size_of::<VS_FIXEDFILEINFO>();
            let mut data = vec![0u8; size + 1];
            data[1..5].copy_from_slice(&0xfeef04bdu32.to_ne_bytes());
            let version = 1 + std::mem::offset_of!(VS_FIXEDFILEINFO, dwFileVersionMS);
            data[version..version + 4].copy_from_slice(&((7u32 << 16) | 6).to_ne_bytes());
            let returned = data.as_ptr().wrapping_add(1).cast();
            assert_eq!(fixed_file_major(&data, returned, size), Some(7));
            assert_eq!(fixed_file_major(&data, returned, size - 1), None);
            assert_eq!(fixed_file_major(&data, returned, size + 1), None);
            assert_eq!(fixed_file_major(&data, returned, usize::MAX), None);
            assert_eq!(fixed_file_major(&data, std::ptr::null(), size), None);
            assert_eq!(fixed_file_major(&data, data.as_ptr().wrapping_sub(1).cast(), size), None);
            assert_eq!(fixed_file_major(&data, data.as_ptr().wrapping_add(data.len()).cast(), size), None);
            data[1] ^= 1;
            assert_eq!(fixed_file_major(&data, returned, size), None);
        }

        #[test]
        fn package_names_must_be_aligned_terminated_and_inside_the_returned_buffer() {
            let buffer = [b'A' as u16, 0, b'B' as u16, b'C' as u16, 0];
            assert_eq!(package_name(&buffer, buffer.as_ptr()), Some(&buffer[..2]));
            assert_eq!(package_name(&buffer, buffer.as_ptr().wrapping_add(2)), Some(&buffer[2..]));
            assert_eq!(package_name(&buffer, std::ptr::null()), None);
            assert_eq!(package_name(&buffer, buffer.as_ptr().wrapping_sub(1)), None);
            assert_eq!(package_name(&buffer, buffer.as_ptr().wrapping_add(buffer.len())), None);
            assert_eq!(package_name(&buffer, buffer.as_ptr().cast::<u8>().wrapping_add(1).cast()), None);
            assert_eq!(package_name(&buffer[..4], buffer.as_ptr().wrapping_add(2)), None);
            assert_eq!(package_name(&buffer, buffer.as_ptr().wrapping_add(1)), None);
        }

        #[test]
        fn package_paths_require_a_nonempty_name_and_a_terminator_within_reported_length() {
            let path = [b'C' as u16, b':' as u16, b'\\' as u16, 0, b'X' as u16];
            assert_eq!(path.get(..4).and_then(terminated_utf16), Some(&path[..4]));
            assert_eq!(path.get(..3).and_then(terminated_utf16), None);
            assert_eq!(path.get(..6).and_then(terminated_utf16), None);
            assert_eq!(terminated_utf16(&[0]), None);
            assert_eq!(terminated_utf16(&[]), None);
        }
    }

    /// An open registry key, closed when dropped.
    struct Key(HKEY);

    impl Key {
        fn open(hive: Hive, path: &str, access: u32) -> Result<Key, u32> {
            let root = match hive {
                Hive::CurrentUser => HKEY_CURRENT_USER,
                Hive::LocalMachine => HKEY_LOCAL_MACHINE,
            };
            let w = wide(path);
            let mut key: HKEY = std::ptr::null_mut();
            // SAFETY: `w` is NUL-terminated and outlives the call; `key` receives the handle.
            let rc = unsafe { RegOpenKeyExW(root, w.as_ptr(), 0, access | KEY_WOW64_64KEY, &mut key) };
            if rc == 0 {
                Ok(Key(key))
            } else {
                Err(rc)
            }
        }
    }

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: we opened it and close it once.
            unsafe { RegCloseKey(self.0) };
        }
    }

    pub fn raw(hive: Hive, key: &str, name: &str) -> Option<Raw> {
        let k = Key::open(hive, key, KEY_READ).ok()?;
        let w = wide(name);
        let mut data = vec![0u8; 512];
        loop {
            let mut kind = 0u32;
            let mut len = data.len() as u32;
            // SAFETY: `data` holds `len` bytes; the name is NUL-terminated.
            let rc = unsafe { RegQueryValueExW(k.0, w.as_ptr(), std::ptr::null(), &mut kind, data.as_mut_ptr(), &mut len) };
            if rc == ERROR_MORE_DATA {
                data.resize(len as usize + 2, 0);
                continue;
            }
            if rc != 0 {
                return None;
            }
            data.truncate(len as usize);
            return Some(Raw { kind, data });
        }
    }

    pub fn values(hive: Hive, key: &str) -> Vec<(String, Raw)> {
        let Ok(k) = Key::open(hive, key, KEY_READ) else { return vec![] };
        let mut out = Vec::new();
        let mut name = vec![0u16; 16_384];
        let mut data = vec![0u8; 4096];
        let mut i = 0u32;
        loop {
            let mut name_len = name.len() as u32;
            let mut data_len = data.len() as u32;
            let mut kind = 0u32;
            // SAFETY: buffers and their lengths are ours and valid for the call.
            let rc = unsafe { RegEnumValueW(k.0, i, name.as_mut_ptr(), &mut name_len, std::ptr::null(), &mut kind, data.as_mut_ptr(), &mut data_len) };
            if rc == ERROR_MORE_DATA {
                data.resize(data_len as usize + 2, 0);
                continue; // same index, bigger buffer
            }
            if rc != 0 {
                break; // ERROR_NO_MORE_ITEMS, or something we can't read
            }
            out.push((String::from_utf16_lossy(&name[..name_len as usize]), Raw { kind, data: data[..data_len as usize].to_vec() }));
            i += 1;
        }
        out
    }

    pub fn subkeys(hive: Hive, key: &str) -> Vec<String> {
        let Ok(k) = Key::open(hive, key, KEY_READ) else { return vec![] };
        let mut out = Vec::new();
        let mut name = vec![0u16; 512];
        let mut i = 0u32;
        loop {
            let mut len = name.len() as u32;
            // SAFETY: the buffer and its length are ours and valid for the call.
            let rc = unsafe {
                RegEnumKeyExW(k.0, i, name.as_mut_ptr(), &mut len, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut())
            };
            if rc != 0 {
                break;
            }
            out.push(String::from_utf16_lossy(&name[..len as usize]));
            i += 1;
        }
        out
    }

    pub fn set(hive: Hive, key: &str, name: &str, value: &Raw) -> Result<(), String> {
        let k = Key::open(hive, key, KEY_SET_VALUE).map_err(|e| format!("error {e}"))?;
        let w = wide(name);
        // SAFETY: `value.data` holds the bytes we pass, with its length.
        let rc = unsafe { RegSetValueExW(k.0, w.as_ptr(), 0, value.kind, value.data.as_ptr(), value.data.len() as u32) };
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("error {rc}"))
        }
    }

    fn root(hive: Hive) -> windows_sys::Win32::System::Registry::HKEY {
        match hive {
            Hive::CurrentUser => HKEY_CURRENT_USER,
            Hive::LocalMachine => HKEY_LOCAL_MACHINE,
        }
    }

    /// Removes a key (one with no keys inside it).
    pub fn delete_key(hive: Hive, key: &str) -> Result<(), String> {
        use windows_sys::Win32::System::Registry::RegDeleteKeyExW;
        let w = wide(key);
        // SAFETY: `w` is NUL-terminated.
        match unsafe { RegDeleteKeyExW(root(hive), w.as_ptr(), KEY_WOW64_64KEY, 0) } {
            0 | ERROR_FILE_NOT_FOUND => Ok(()),
            rc => Err(format!("error {rc}")),
        }
    }

    /// Makes a key (and any missing above it).
    pub fn create_key(hive: Hive, key: &str) -> Result<(), String> {
        use windows_sys::Win32::System::Registry::{RegCreateKeyExW, REG_OPTION_NON_VOLATILE};
        let w = wide(key);
        let mut h: HKEY = std::ptr::null_mut();
        // SAFETY: `w` is NUL-terminated; the handle is closed right after.
        let rc = unsafe { RegCreateKeyExW(root(hive), w.as_ptr(), 0, std::ptr::null(), REG_OPTION_NON_VOLATILE, KEY_SET_VALUE | KEY_WOW64_64KEY, std::ptr::null(), &mut h, std::ptr::null_mut()) };
        if rc != 0 {
            return Err(format!("error {rc}"));
        }
        // SAFETY: we just opened it.
        unsafe { RegCloseKey(h) };
        Ok(())
    }

    pub fn delete(hive: Hive, key: &str, name: &str) -> Result<(), String> {
        let k = match Key::open(hive, key, KEY_SET_VALUE) {
            Ok(k) => k,
            Err(ERROR_FILE_NOT_FOUND) => return Ok(()), // no key, no value
            Err(e) => return Err(format!("error {e}")),
        };
        let w = wide(name);
        // SAFETY: the name is NUL-terminated.
        match unsafe { RegDeleteValueW(k.0, w.as_ptr()) } {
            0 | ERROR_FILE_NOT_FOUND => Ok(()),
            rc => Err(format!("error {rc}")),
        }
    }

    fn utf16(units: &[u16]) -> String {
        let end = units.iter().position(|u| *u == 0).unwrap_or(units.len());
        String::from_utf16_lossy(&units[..end])
    }

    /// Every screen on the desktop: how it's driven now, and the best it offers.
    pub fn displays() -> Vec<Display> {
        use windows_sys::Win32::Graphics::Gdi::{
            EnumDisplayDevicesW, EnumDisplaySettingsW, DEVMODEW, DISPLAY_DEVICEW, DISPLAY_DEVICE_ATTACHED_TO_DESKTOP, DISPLAY_DEVICE_MIRRORING_DRIVER,
            DM_INTERLACED, ENUM_CURRENT_SETTINGS,
        };
        let mut out = Vec::new();
        for i in 0..16u32 {
            let mut dev = DISPLAY_DEVICEW { cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
            // SAFETY: `dev.cb` is set; a null device name asks for adapters.
            if unsafe { EnumDisplayDevicesW(std::ptr::null(), i, &mut dev, 0) } == 0 {
                break;
            }
            if dev.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP == 0 || dev.StateFlags & DISPLAY_DEVICE_MIRRORING_DRIVER != 0 {
                continue;
            }
            let device = utf16(&dev.DeviceName);
            let name_w = wide(&device);
            let mode = |n: u32| {
                let mut dm = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
                // SAFETY: `dm.dmSize` is set; the device name is NUL-terminated.
                (unsafe { EnumDisplaySettingsW(name_w.as_ptr(), n, &mut dm) } != 0).then_some(dm)
            };
            let Some(now) = mode(ENUM_CURRENT_SETTINGS) else { continue };
            let (width, height, hz) = (now.dmPelsWidth, now.dmPelsHeight, now.dmDisplayFrequency);
            let (mut best_hz, mut max_width, mut max_height) = (hz, width, height);
            let mut n = 0;
            while let Some(dm) = mode(n) {
                n += 1;
                // SAFETY: plain data in a union read as the integer it holds.
                let interlaced = unsafe { dm.Anonymous2.dmDisplayFlags } & DM_INTERLACED != 0;
                if interlaced || dm.dmBitsPerPel < 24 {
                    continue;
                }
                if dm.dmPelsWidth == width && dm.dmPelsHeight == height {
                    best_hz = best_hz.max(dm.dmDisplayFrequency);
                }
                if dm.dmPelsWidth as u64 * dm.dmPelsHeight as u64 > max_width as u64 * max_height as u64 {
                    (max_width, max_height) = (dm.dmPelsWidth, dm.dmPelsHeight);
                }
            }
            // The screen's own name, from the monitor behind the adapter.
            let mut mon = DISPLAY_DEVICEW { cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
            // SAFETY: as above, asking for the first monitor on this adapter.
            let name = if unsafe { EnumDisplayDevicesW(name_w.as_ptr(), 0, &mut mon, 0) } != 0 { utf16(&mon.DeviceString) } else { String::new() };
            let name = if name.is_empty() || name.eq_ignore_ascii_case("Generic PnP Monitor") {
                format!("Screen {}", device.trim_start_matches(r"\\.\DISPLAY"))
            } else {
                name
            };
            out.push(Display { device, name, width, height, hz, best_hz, max_width, max_height });
        }
        out
    }
}

#[cfg(not(windows))]
mod os {
    use super::{Display, Hive, Raw};
    pub fn executable_major(_: &std::path::Path) -> Option<u32> {
        None
    }
    pub fn powershell_packages() -> Vec<std::path::PathBuf> {
        vec![]
    }
    pub fn raw(_: Hive, _: &str, _: &str) -> Option<Raw> {
        None
    }
    pub fn values(_: Hive, _: &str) -> Vec<(String, Raw)> {
        vec![]
    }
    pub fn subkeys(_: Hive, _: &str) -> Vec<String> {
        vec![]
    }
    pub fn set(_: Hive, _: &str, _: &str, _: &Raw) -> Result<(), String> {
        Err("Only on Windows.".into())
    }
    pub fn delete(_: Hive, _: &str, _: &str) -> Result<(), String> {
        Err("Only on Windows.".into())
    }
    pub fn delete_key(_: Hive, _: &str) -> Result<(), String> {
        Err("Only on Windows.".into())
    }
    pub fn create_key(_: Hive, _: &str) -> Result<(), String> {
        Err("Only on Windows.".into())
    }
    pub fn displays() -> Vec<Display> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opens a real window (the environment variables editor), so it's run by hand, and the
    /// window is yours to close: cargo test -p mom-core the_variable_editor_opens -- --ignored
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn the_variable_editor_opens() {
        let (exe, hand) = program(ENV_EDITOR).unwrap();
        let windows = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap());
        crate::elevate::open(&windows.join("System32").join(exe), hand).unwrap();
    }

    #[test]
    fn every_settings_program_a_finding_names_can_be_opened() {
        // Read from the checkups themselves: `.settings("x.exe")` here, and `kind: "launch", value: "x.msc"` elsewhere.
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let named = regex::Regex::new(r#"(?:\.settings\(|kind: "launch", value: )"([^"]+)""#).unwrap();
        let mut seen = Vec::new();
        for file in std::fs::read_dir(&src).unwrap().filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "rs")) {
            let text = std::fs::read_to_string(&file).unwrap();
            let body = text.split("#[cfg(test)]").next().unwrap_or(&text).to_string();
            for m in named.captures_iter(&body) {
                let name = m[1].to_string();
                if !name.starts_with("ms-settings:") {
                    assert!(PROGRAMS.contains(&name.as_str()), "{} names {name}, which the button isn't allowed to open", file.display());
                    seen.push(name);
                }
            }
        }
        for name in ["services.msc", "cttune.exe", "devmgmt.msc", "eventvwr.exe", "dfrgui.exe"] {
            assert!(seen.iter().any(|s| s == name), "the test no longer finds {name}: has the way findings name programs changed?");
        }
        // Some are named through a constant: each of those is on the list, and there are no others.
        assert!(PROGRAMS.contains(&PERFORMANCE) && PROGRAMS.contains(&ENV_EDITOR));
        let here = std::fs::read_to_string(src.join("winsettings.rs")).unwrap();
        let by_constant = regex::Regex::new(r#"\.settings\(([^")]+)\)"#).unwrap();
        for m in by_constant.captures_iter(here.split("#[cfg(test)]").next().unwrap()) {
            assert!(["PERFORMANCE", "ENV_EDITOR"].contains(&&m[1]), "a finding names its settings program as `{}`: is it on the list?", &m[1]);
        }
        // Each is a bare program name (never a path), and what's handed to it is plain.
        for name in PROGRAMS {
            let (exe, hand) = program(name).unwrap();
            assert!(exe.chars().all(|c| c.is_ascii_alphanumeric() || c == '.') && (exe.ends_with(".exe") || exe.ends_with(".msc")), "{name}");
            assert!(hand.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == ','), "{name}");
        }
        assert_eq!(program(ENV_EDITOR), Some(("rundll32.exe", "sysdm.cpl,EditEnvironmentVariables")));
        assert_eq!(program("services.msc"), Some(("services.msc", "")));
        for not_one in ["cmd.exe", "rundll32.exe", "rundll32.exe evil.dll,Run", r"..\services.msc", "services.msc /s", ""] {
            assert_eq!(program(not_one), None, "{not_one}");
        }
    }
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};

    /// Registry keys (lowercased) and their values.
    type Keys = HashMap<(Hive, String), Vec<(String, Raw)>>;

    /// A made-up PC.
    #[derive(Default)]
    struct Fake {
        reg: RefCell<Keys>,
        files: HashSet<String>,
        executable_versions: HashMap<String, u32>,
        powershell_packages: Vec<PathBuf>,
        env: HashMap<String, String>,
        displays: Vec<Display>,
        hosts: Option<String>,
        ports: HashSet<u16>,
    }

    impl Fake {
        fn new() -> Fake {
            let mut f = Fake::default();
            f.files.insert(r"c:\".into());
            f
        }
        fn put(&self, hive: Hive, key: &str, name: &str, v: Raw) {
            let mut reg = self.reg.borrow_mut();
            let values = reg.entry((hive, key.to_ascii_lowercase())).or_default();
            values.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
            values.push((name.into(), v));
        }
        fn file(mut self, p: &str) -> Fake {
            self.files.insert(p.to_ascii_lowercase());
            self
        }
    }

    impl Machine for Fake {
        fn raw(&self, hive: Hive, key: &str, name: &str) -> Option<Raw> {
            self.reg.borrow().get(&(hive, key.to_ascii_lowercase()))?.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone())
        }
        fn raw_values(&self, hive: Hive, key: &str) -> Vec<(String, Raw)> {
            self.reg.borrow().get(&(hive, key.to_ascii_lowercase())).cloned().unwrap_or_default()
        }
        fn subkeys(&self, hive: Hive, key: &str) -> Vec<String> {
            let prefix = format!("{}\\", key.to_ascii_lowercase());
            let mut out: Vec<String> =
                self.reg.borrow().keys().filter(|(h, k)| *h == hive && k.starts_with(&prefix)).filter_map(|(_, k)| k[prefix.len()..].split('\\').next().map(String::from)).collect();
            out.sort();
            out.dedup();
            out
        }
        fn exists(&self, path: &Path) -> bool {
            self.files.contains(&path.to_string_lossy().to_ascii_lowercase())
        }
        fn env(&self, name: &str) -> Option<String> {
            self.env.get(name).cloned()
        }
        fn displays(&self) -> Vec<Display> {
            self.displays.clone()
        }
        fn hosts(&self) -> Option<String> {
            self.hosts.clone()
        }
        fn listening(&self, port: u16) -> bool {
            self.ports.contains(&port)
        }
        fn system_drive(&self) -> String {
            r"C:\".into()
        }
        fn executable_major(&self, path: &Path) -> Option<u32> {
            self.executable_versions.get(&path.to_string_lossy().to_ascii_lowercase()).copied()
        }
        fn powershell_packages(&self) -> Vec<PathBuf> {
            self.powershell_packages.clone()
        }
    }

    #[test]
    fn terminals_open_in_windows_terminal_draw_with_a_proper_font_and_speak_utf8() {
        let mut m = Fake::new();
        m.env.insert("LOCALAPPDATA".into(), r"C:\Users\me\AppData\Local".into());
        m.env.insert("SystemRoot".into(), r"C:\Windows".into());
        m.env.insert("ProgramFiles".into(), r"C:\Program Files".into());
        m.files.insert(r"c:\users\me\appdata\local\microsoft\windowsapps\wt.exe".into());
        m.files.insert(r"c:\windows\fonts\cascadiamono.ttf".into());
        m.put(CU, CONSOLE_STARTUP, "DelegationConsole", Raw::string("{00000000-0000-0000-0000-000000000000}"));
        m.put(CU, r"Console\%SystemRoot%_System32_WindowsPowerShell_v1.0_powershell.exe", "FaceName", Raw::string("Lucida Console"));
        m.put(LM, NLS_CODEPAGE, "OEMCP", Raw::string("437"));
        m.put(LM, NT_VERSION, "CurrentBuildNumber", Raw::string("19045"));
        m.env.insert("APPDATA".into(), r"C:\Users\me\AppData\Roaming".into());
        m.files.insert(r"c:\users\me\appdata\roaming\microsoft\windows\powershell\psreadline\consolehost_history.txt".into());
        let found = check_with(&m);
        let ids: Vec<&str> = found.iter().filter(|f| f.area == "Terminal").map(|f| f.id.as_str()).collect();
        assert_eq!(ids, vec!["terminal-default", "console-font", "console-utf8", "powershell-7"], "{ids:?}");
        let font = found.iter().find(|f| f.id == "console-font").unwrap();
        assert_eq!(font.title, "The console window uses Lucida Console");
        assert_eq!(font.default, "Cascadia Mono");
        assert!(found.iter().find(|f| f.id == "powershell-7").unwrap().command.as_deref().unwrap().starts_with("winget install"));

        let changes = user_changes(&m, "console-font").unwrap();
        assert_eq!(changes.len(), 1);
        assert!(changes[0].key.eq_ignore_ascii_case(r"Console\%SystemRoot%_System32_WindowsPowerShell_v1.0_powershell.exe"));
        let d = user_changes(&m, "terminal-default").unwrap();
        assert_eq!(d[0].to.as_ref().unwrap().as_string().as_deref(), Some(WT_CONSOLE));

        // Windows 11 lets "Let Windows decide" mean Windows Terminal, where the old console's font
        // and code page don't apply: nothing to say but PowerShell 7.
        m.put(LM, NT_VERSION, "CurrentBuildNumber", Raw::string("26200"));
        let terminal = |m: &Fake| check_with(m).into_iter().filter(|f| f.area == "Terminal").map(|f| f.id).collect::<Vec<String>>();
        assert_eq!(terminal(&m), vec!["powershell-7".to_string()]);
        // Unless the old console is chosen by name.
        m.put(CU, CONSOLE_STARTUP, "DelegationConsole", Raw::string(CONHOST));
        assert_eq!(get(&m, "terminal-default").level, Level::Fix);
        // Windows Terminal chosen: the same as Windows deciding.
        m.put(CU, CONSOLE_STARTUP, "DelegationConsole", Raw::string(WT_CONSOLE));
        assert_eq!(terminal(&m), vec!["powershell-7".to_string()]);
        // PowerShell 7 there, Windows PowerShell's line editor old.
        m.env.insert("ProgramFiles".into(), r"C:\Program Files".into());
        m.executable_versions.insert(r"c:\program files\powershell\7\pwsh.exe".into(), 7);
        assert!(terminal(&m).is_empty(), "the Fake lists no PSReadLine folders");
    }

    #[test]
    fn powershell_7_is_found_for_msi_msix_aliases_and_portable_installs() {
        let pf = r"C:\Program Files";
        let local = r"C:\Users\me\AppData\Local";
        let package = r"C:\Program Files\WindowsApps\Microsoft.PowerShell_7.6.6.0_x64__8wekyb3d8bbwe";
        let cases = [
            (r"C:\Program Files\PowerShell\7\pwsh.exe".to_string(), false, ""),
            (format!(r"{package}\pwsh.exe"), true, ""),
            (format!(r"{local}\Microsoft\WindowsApps\pwsh.exe"), false, ""),
            (r"C:\Tools\PowerShell\pwsh.exe".to_string(), false, r#".;"C:\Tools\PowerShell""#),
        ];
        for (exe, registered, path) in cases {
            let mut m = Fake::new();
            m.executable_versions.insert(exe.to_ascii_lowercase(), 7);
            m.env.insert("PATH".into(), path.into());
            if registered {
                m.powershell_packages.push(package.into());
            }
            assert!(powershell_7(&m, pf, local), "{exe}");
            m.executable_versions.clear();
            m.files.insert(exe.to_ascii_lowercase());
            assert!(!powershell_7(&m, pf, local), "an inert file or stale registration isn't an installation: {exe}");
            m.executable_versions.insert(exe.to_ascii_lowercase(), 6);
            assert!(!powershell_7(&m, pf, local), "PowerShell 6 isn't PowerShell 7: {exe}");
        }
        let mut m = Fake::new();
        m.env.insert("PATH".into(), r".;tools;C:relative;\rooted;\\server\share".into());
        for dir in [".", "tools", "C:relative", r"\rooted", r"\\server\share"] {
            m.executable_versions.insert(Path::new(dir).join("pwsh.exe").to_string_lossy().to_ascii_lowercase(), 7);
        }
        assert!(!powershell_7(&m, pf, local), "never search relative paths or network shares");
    }

    #[cfg(windows)]
    #[test]
    fn executable_version_rejects_an_inert_pwsh_file() {
        let root = std::env::temp_dir().join(format!("mom-pwsh-probe-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("pwsh.exe");
        std::fs::write(&path, b"not an executable").unwrap();
        assert_eq!(os::executable_major(&path), None);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&root).unwrap();
    }

    impl Writer for Fake {
        fn set(&self, hive: Hive, key: &str, name: &str, value: &Raw) -> Result<(), String> {
            self.put(hive, key, name, value.clone());
            Ok(())
        }
        fn delete(&self, hive: Hive, key: &str, name: &str) -> Result<(), String> {
            if let Some(values) = self.reg.borrow_mut().get_mut(&(hive, key.to_ascii_lowercase())) {
                values.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
            }
            Ok(())
        }
    }

    fn ids(m: &Fake) -> Vec<String> {
        check_with(m).into_iter().map(|f| f.id).collect()
    }
    fn get(m: &Fake, id: &str) -> Finding {
        check_with(m).into_iter().find(|f| f.id == id).unwrap_or_else(|| panic!("no {id} in {:?}", ids(m)))
    }

    #[test]
    fn a_pc_as_windows_ships_it_has_nothing_to_say() {
        let m = Fake::new();
        m.put(LM, MEM, "PagingFiles", Raw::multi(&[r"?:\pagefile.sys"]));
        m.put(LM, PREFETCH, "EnablePrefetcher", Raw::dword(3));
        m.put(LM, &format!(r"{SERVICES}\SysMain"), "Start", Raw::dword(2));
        m.put(LM, UAC, "EnableLUA", Raw::dword(1));
        m.put(LM, UAC, "ConsentPromptBehaviorAdmin", Raw::dword(5));
        m.put(LM, CONTROL, "WaitToKillServiceTimeout", Raw::string("5000"));
        m.put(CU, DESKTOP, "FontSmoothing", Raw::string("2"));
        m.put(CU, DESKTOP, "FontSmoothingType", Raw::dword(2));
        m.put(CU, DESKTOP, "AutoEndTasks", Raw::string("0"));
        assert!(ids(&m).is_empty(), "{:?}", ids(&m));
        // The same with a page file Windows sizes on C:.
        m.put(LM, MEM, "PagingFiles", Raw::multi(&[r"C:\pagefile.sys 0 0"]));
        assert!(ids(&m).is_empty(), "{:?}", ids(&m));
    }

    #[test]
    fn the_page_file_fixed_missing_or_elsewhere() {
        let m = Fake::new();
        m.put(LM, MEM, "PagingFiles", Raw::multi(&[r"C:\pagefile.sys 4096 4096"]));
        let f = get(&m, "pagefile");
        assert_eq!((f.title.as_str(), f.now.as_str()), ("The page file has a fixed size", "Fixed at 4,096 MB on C:"));
        assert!(f.fix.as_ref().is_some_and(|x| x.admin && !x.undo));
        m.put(LM, MEM, "PagingFiles", Raw::multi(&[]));
        assert_eq!(get(&m, "pagefile").title, "There's no page file");
        m.put(LM, MEM, "PagingFiles", Raw::multi(&[r"D:\pagefile.sys 0 0"]));
        assert_eq!(get(&m, "pagefile").now, "Only on D:");
        assert_eq!(admin_changes(&m, "pagefile").unwrap(), vec![set(LM, MEM, "PagingFiles", Raw::multi(&[r"?:\pagefile.sys"]))]);
    }

    #[test]
    fn old_tweaks_disabled_services_and_switched_off_safety() {
        let m = Fake::new();
        m.put(LM, MEM, "ClearPageFileAtShutdown", Raw::dword(1));
        m.put(LM, MEM, "SecondLevelDataCache", Raw::dword(1024));
        m.put(LM, PRIORITY, "IRQ8Priority", Raw::dword(1));
        m.put(LM, PREFETCH, "EnablePrefetcher", Raw::dword(0));
        m.put(LM, &format!(r"{SERVICES}\wuauserv"), "Start", Raw::dword(4));
        m.put(LM, &format!(r"{SERVICES}\Spooler"), "Start", Raw::dword(4));
        m.put(LM, &format!(r"{SERVICES}\WSearch"), "Start", Raw::dword(3)); // manual: not flagged
        m.put(LM, UAC, "EnableLUA", Raw::dword(0));
        m.put(LM, UPDATE_POLICY, "NoAutoUpdate", Raw::dword(1));
        let all = check_with(&m);
        let ids: Vec<&str> = all.iter().map(|f| f.id.as_str()).collect();
        for id in ["clear-pagefile", "memory-myths", "prefetch-off", "service-off:wuauserv", "service-off:Spooler", "uac-off", "update-policy-off"] {
            assert!(ids.contains(&id), "missing {id}: {ids:?}");
        }
        assert!(!ids.contains(&"service-off:WSearch"));
        // What hurts comes first; a choice comes last.
        assert_eq!(all[0].level, Level::Fix);
        assert_eq!(all.last().unwrap().id, "service-off:Spooler");
        assert_eq!(get(&m, "memory-myths").now, "SecondLevelDataCache, IRQ8Priority");
        // The print spooler is a choice: no button. Windows Update comes back as it ships.
        assert!(get(&m, "service-off:Spooler").fix.is_none() && !is_admin_fix("service-off:Spooler"));
        assert_eq!(admin_changes(&m, "service-off:wuauserv").unwrap(), vec![set(LM, &format!(r"{SERVICES}\wuauserv"), "Start", Raw::dword(3))]);
        // UAC is the user's to turn on, in its own settings.
        assert!(get(&m, "uac-off").fix.is_none());
    }

    #[test]
    fn shutdown_timeouts_screens_and_text() {
        let m = Fake {
            displays: vec![Display {
                device: r"\\.\DISPLAY1".into(),
                name: "Studio Screen".into(),
                width: 2560,
                height: 1440,
                hz: 60,
                best_hz: 144,
                max_width: 2560,
                max_height: 1440,
            }],
            ..Fake::new()
        };
        m.put(CU, DESKTOP, "AutoEndTasks", Raw::string("1"));
        m.put(CU, DESKTOP, "WaitToKillAppTimeout", Raw::string("1000"));
        m.put(CU, DESKTOP, "HungAppTimeout", Raw::string("5000")); // default: fine
        m.put(LM, CONTROL, "WaitToKillServiceTimeout", Raw::string("2000"));
        m.put(CU, DESKTOP, "FontSmoothing", Raw::string("0"));
        m.put(CU, DESKTOP, "Win8DpiScaling", Raw::dword(1));
        m.put(CU, DESKTOP, "LogPixels", Raw::dword(120));
        let ids = ids(&m);
        assert!(!ids.contains(&"hung-app-timeout".to_string()));
        assert_eq!(get(&m, "app-shutdown-wait").now, "1 second");
        assert_eq!(get(&m, "service-shutdown-wait").now, "2 seconds");
        assert_eq!(get(&m, "refresh:display1").title, "Studio Screen runs at 60 Hz but can do 144 Hz");
        assert!(!ids.contains(&"resolution:display1".to_string()));
        assert_eq!(get(&m, "custom-scaling").now, "125%");
        assert_eq!(get(&m, "font-smoothing").now, "Off");
        assert_eq!(user_changes(&m, "auto-end-tasks").unwrap(), vec![delete(CU, DESKTOP, "AutoEndTasks")]);
    }

    #[test]
    fn a_proxy_nobody_answers_and_a_hosts_file_that_blocks_updates() {
        let mut m = Fake::new();
        m.put(CU, INTERNET, "ProxyEnable", Raw::dword(1));
        m.put(CU, INTERNET, "ProxyServer", Raw::string("http=127.0.0.1:8888;https=127.0.0.1:8888"));
        m.hosts = Some("# comment\n0.0.0.0 fe2.update.microsoft.com\n0.0.0.0 download.windowsupdate.com\n127.0.0.1 example.test\n".into());
        let f = get(&m, "proxy");
        assert_eq!((f.level, f.title.as_str()), (Level::Fix, "Windows' proxy points at a program that isn't running"));
        assert_eq!(get(&m, "hosts-blocks-updates").now, "2 update servers blocked");
        assert!(!ids(&m).contains(&"hosts-long".to_string()));
        // While the debugging tool runs, it's just a note.
        m.ports.insert(8888);
        assert_eq!(get(&m, "proxy").level, Level::Info);
        assert_eq!(proxy_target("proxy.example.test:3128"), ("proxy.example.test".into(), Some(3128)));
    }

    #[test]
    fn leftovers_pointing_at_things_that_are_gone() {
        let mut m = Fake::new().file(r"C:\Tools\bin").file(r"C:\Users\pat\AppData\Local\Kept\kept.exe");
        m.env.insert("USERPROFILE".into(), r"C:\Users\pat".into());
        m.put(CU, USER_ENV, "Path", Raw::text(REG_EXPAND_SZ, r"C:\Tools\bin;%USERPROFILE%\.oldtool\bin;;\\server\share;%NOT_SET%\x;E:\usb\bin"));
        m.put(LM, SYSTEM_ENV, "Path", Raw::text(REG_EXPAND_SZ, r"C:\Tools\bin;C:\Python27"));
        m.put(CU, USER_ENV, "JAVA_HOME", Raw::string(r"C:\Program Files\Java\jdk-old"));
        m.put(CU, LAYERS, r"C:\Games\Old\game.exe", Raw::string("~ RUNASADMIN"));
        m.put(CU, RUN, "Updater", Raw::string(r#""C:\Program Files\GoneApp\updater.exe" --background"#));
        m.put(CU, RUN, "Kept", Raw::string(r"%USERPROFILE%\AppData\Local\Kept\kept.exe"));
        m.put(CU, RUN, "Helper", Raw::string(r"rundll32.exe C:\gone.dll,Start"));
        let user = get(&m, "user-path-missing");
        assert_eq!(user.now, r"C:\Users\pat\.oldtool\bin", "unknown variables, network paths and absent drives aren't called gone");
        assert_eq!(user.level, Level::Check);
        // A missing folder at a drive's root, in Windows' PATH, is something to fix.
        let system = get(&m, "system-path-missing");
        assert_eq!((system.level, system.now.as_str()), (Level::Fix, r"C:\Python27"));
        assert!(system.fix.as_ref().is_some_and(|f| f.admin));
        assert_eq!(get(&m, "env-missing").now, r"JAVA_HOME → C:\Program Files\Java\jdk-old");
        assert_eq!(get(&m, "compat-orphans-user").now, "game.exe");
        assert_eq!(get(&m, "startup-orphans-user").now, "Updater");
        assert!(get(&m, "startup-orphans-user").fix.is_some_and(|f| !f.admin && f.label == "Remove it"));
        // Removing the gone folders keeps the rest, in order, as the same type.
        let fix = user_changes(&m, "user-path-missing").unwrap();
        assert_eq!(fix, vec![set(CU, USER_ENV, "Path", Raw::text(REG_EXPAND_SZ, r"C:\Tools\bin;;\\server\share;%NOT_SET%\x;E:\usb\bin"))]);
        m.files.insert(r"c:\users\pat\.oldtool\bin".into());
        assert!(!ids(&m).contains(&"user-path-missing".to_string()));
    }

    #[test]
    fn leftovers_for_every_user_go_too_with_an_administrator() {
        let mut m = Fake::new().file(r"C:\Program Files\Atlas\atlas.exe");
        m.env.insert("ProgramFiles".into(), r"C:\Program Files".into());
        m.env.insert("USERPROFILE".into(), r"C:\Users\pat".into());
        // In the startup list for every user: two whose program is gone (one of them in the 32-bit
        // list, one spelled with a variable every user shares), and four that stay.
        m.put(LM, RUN, "NimbusAgent", Raw::string(r#""C:\Program Files\Nimbus\Security App\agent.exe" --tray"#));
        m.put(LM, RUN_32, "CopperlineUpdate", Raw::text(REG_EXPAND_SZ, r"%ProgramFiles%\Copperline\update.exe"));
        m.put(LM, RUN, "Atlas", Raw::string(r"C:\Program Files\Atlas\atlas.exe"));
        m.put(LM, RUN, "PerUser", Raw::text(REG_EXPAND_SZ, r"%USERPROFILE%\AppData\Local\Gone\gone.exe"));
        m.put(LM, RUN, "SecurityHealth", Raw::string(r"rundll32.exe C:\gone.dll,Start"));
        m.put(LM, RUN, "OnAStick", Raw::string(r"E:\portable\tool.exe"));
        m.put(LM, LAYERS, r"C:\Games\Old\game.exe", Raw::string("~ RUNASADMIN"));
        m.put(LM, LAYERS, r"C:\Program Files\Atlas\atlas.exe", Raw::string("~ HIGHDPIAWARE"));
        let startup = get(&m, "startup-orphans-machine");
        assert_eq!(startup.now, "NimbusAgent, CopperlineUpdate");
        assert!(startup.fix.as_ref().is_some_and(|f| f.admin && f.label == "Remove them"), "something to do, and Windows asks first");
        let compat = get(&m, "compat-orphans-machine");
        assert_eq!(compat.now, "game.exe");
        assert!(compat.fix.as_ref().is_some_and(|f| f.admin && f.label == "Remove it"));
        assert!(is_admin_fix("startup-orphans-machine") && is_admin_fix("compat-orphans-machine") && !is_admin_fix("startup-orphans-user"));
        // What an administrator's mom would change: exactly those, each in its own list.
        assert_eq!(admin_changes(&m, "startup-orphans-machine").unwrap(), vec![delete(LM, RUN, "NimbusAgent"), delete(LM, RUN_32, "CopperlineUpdate")]);
        assert_eq!(admin_changes(&m, "compat-orphans-machine").unwrap(), vec![delete(LM, LAYERS, r"C:\Games\Old\game.exe")]);
        // Done, the findings are gone and everything else is as it was.
        let said = fix_elevated_with(&m, &m, "startup-orphans-machine").unwrap();
        assert!(said.contains("NimbusAgent was") && said.contains("agent.exe"), "{said}");
        fix_elevated_with(&m, &m, "compat-orphans-machine").unwrap();
        assert!(!ids(&m).iter().any(|id| id.ends_with("-orphans-machine")));
        let left: Vec<String> = text_values(&m, LM, RUN).into_iter().map(|(name, _)| name).collect();
        assert_eq!(left, ["Atlas", "PerUser", "SecurityHealth", "OnAStick"]);
        assert!(text_values(&m, LM, RUN_32).is_empty() && m.raw_values(LM, LAYERS).len() == 1);
        // A path through a variable that's yours alone is never judged for the whole PC.
        assert!(same_for_everyone(r"%ProgramFiles%\x.exe") && same_for_everyone(r"C:\x.exe") && same_for_everyone(r"%programdata%\a\%SystemRoot%"));
        assert!(!same_for_everyone(r"%USERPROFILE%\x.exe") && !same_for_everyone(r"%ProgramFiles%\%APPDATA%\x.exe") && !same_for_everyone(r"%LOCALAPPDATA%\x"));
    }

    #[test]
    fn a_fix_backs_up_first_and_undo_puts_it_back() {
        let root = std::env::temp_dir().join(format!("mom-winsettings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::under(&root);
        let m = Fake::new();
        m.put(CU, DESKTOP, "AutoEndTasks", Raw::string("1"));
        m.put(CU, DESKTOP, "FontSmoothing", Raw::string("0"));
        let msg = fix_with(&paths, &m, &m, "auto-end-tasks").unwrap();
        assert!(msg.contains("next time you sign in"), "{msg}");
        assert!(m.raw(CU, DESKTOP, "AutoEndTasks").is_none());
        // Font smoothing: one setting changed, one that wasn't there added; undo restores both.
        fix_with(&paths, &m, &m, "font-smoothing").unwrap();
        assert_eq!(string(&m, CU, DESKTOP, "FontSmoothing").as_deref(), Some("2"));
        assert_eq!(dword(&m, CU, DESKTOP, "FontSmoothingType"), Some(2));
        undo_with(&paths, &m, "font-smoothing").unwrap();
        assert_eq!(string(&m, CU, DESKTOP, "FontSmoothing").as_deref(), Some("0"));
        assert!(m.raw(CU, DESKTOP, "FontSmoothingType").is_none());
        undo_with(&paths, &m, "auto-end-tasks").unwrap();
        assert_eq!(string(&m, CU, DESKTOP, "AutoEndTasks").as_deref(), Some("1"));
        // Each backup is used once.
        assert!(undo_with(&paths, &m, "auto-end-tasks").is_err());
        // Nothing to fix, admin fixes and made-up ids are refused.
        assert!(fix_with(&paths, &m, &m, "proxy").is_err());
        m.put(LM, MEM, "LargeSystemCache", Raw::dword(1));
        assert!(fix_with(&paths, &m, &m, "large-system-cache").unwrap_err().contains("administrator"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn undo_only_trusts_backups_of_your_own_settings() {
        let root = std::env::temp_dir().join(format!("mom-winsettings-undo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::under(&root);
        let m = Fake::new();
        let forged = Backup {
            id: "proxy".into(),
            at: 1,
            values: vec![Saved { hive: LM, key: UAC.into(), name: "EnableLUA".into(), was: Some(Raw::dword(0)) }],
        };
        crate::util::write_atomic(&backups(&paths).join("proxy-1.json"), &serde_json::to_string(&forged).unwrap()).unwrap();
        assert!(undo_with(&paths, &m, "proxy").unwrap_err().contains("isn't one mom made"));
        assert!(m.raw(LM, UAC, "EnableLUA").is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn as_administrator_only_mom_s_own_list_with_her_own_values() {
        let m = Fake::new();
        m.put(LM, MEM, "PagingFiles", Raw::multi(&[r"C:\pagefile.sys 2048 2048"]));
        m.put(LM, &format!(r"{SERVICES}\SysMain"), "Start", Raw::dword(4));
        let msg = fix_elevated_with(&m, &m, "pagefile").unwrap();
        assert!(msg.contains(r#"PagingFiles was "C:\pagefile.sys 2048 2048""#) && msg.contains("restart"), "{msg}");
        assert_eq!(m.raw(LM, MEM, "PagingFiles"), Some(Raw::multi(&[r"?:\pagefile.sys"])));
        fix_elevated_with(&m, &m, "service-off:SysMain").unwrap();
        assert_eq!(dword(&m, LM, &format!(r"{SERVICES}\SysMain"), "Start"), Some(2));
        // Already fine, not admin, or not on the list: no.
        assert!(fix_elevated_with(&m, &m, "pagefile").is_err());
        m.put(CU, DESKTOP, "AutoEndTasks", Raw::string("1"));
        assert!(fix_elevated_with(&m, &m, "auto-end-tasks").is_err());
        assert!(!is_admin_fix("auto-end-tasks") && !is_admin_fix("service-off:Themes") && !is_admin_fix("uac-off"));
        assert!(is_admin_fix("service-off:SysMain") && is_admin_fix("system-path-missing"));
        // Outside an elevated process, the real entry point refuses.
        if !crate::elevate::is_elevated() {
            assert!(fix_elevated("pagefile").is_err());
        }
    }

    #[test]
    fn registry_values_round_trip() {
        assert_eq!(Raw::multi(&["a b", "c"]).as_multi(), Some(vec!["a b".to_string(), "c".to_string()]));
        assert_eq!(Raw::multi(&[]).as_multi(), Some(vec![]));
        assert_eq!(Raw::string("5000").as_string().as_deref(), Some("5000"));
        assert_eq!(Raw::dword(7).as_dword(), Some(7));
        assert_eq!(describe(Some(&Raw::dword(4))), "4");
        assert_eq!(describe(None), "not set");
    }
}
