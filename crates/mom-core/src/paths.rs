//! Where things live. Everything takes a `Paths` so tests can point mom at a fake home.

use crate::util::env_path;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Paths {
    /// The user's home folder. Agent config lives here (~/.claude, ~/.codex, ...).
    pub home: PathBuf,
    /// Mom's own folder (~/.momputer): config, house rules, journal, state.
    pub mom: PathBuf,
    /// %TEMP%
    pub temp: PathBuf,
    /// %LOCALAPPDATA%
    pub local_appdata: PathBuf,
    /// %APPDATA%
    pub appdata: PathBuf,
    /// The Downloads folder (wherever the user moved it).
    pub downloads: PathBuf,
    /// System folders, for clean-ups that need an administrator: C:\Windows, C:\ProgramData,
    /// C:\Program Files (x86) and the root of the system drive. Inside the test folder in tests.
    pub windows: PathBuf,
    pub program_data: PathBuf,
    pub program_files_x86: PathBuf,
    pub system_drive: PathBuf,
    /// The other drives built into this PC (D:\, E:\...). Windows setup leaves its working
    /// folders at the top of whichever drive has room, and a disk check leaves what it recovered
    /// on the drive it checked. None in tests, unless one adds a folder to stand in for a drive.
    pub other_drives: Vec<PathBuf>,
    /// Folders agents were told to use instead of their usual one (HERMES_HOME and the like),
    /// read from the environment once. Empty in tests, so they never reach the real ones.
    pub agent_env: Vec<(&'static str, PathBuf)>,
}

/// Where Windows is, as Windows says (C:\Windows).
fn windows_dir() -> PathBuf {
    #[cfg(windows)]
    {
        let mut buf = [0u16; 260];
        // SAFETY: the buffer and its length are ours.
        let n = unsafe { windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
        if n > 0 && n < buf.len() {
            return PathBuf::from(String::from_utf16_lossy(&buf[..n]));
        }
    }
    PathBuf::from(r"C:\Windows")
}

/// Environment variables that move an agent's folder.
const AGENT_ENV: &[&str] = &["HERMES_HOME", "OPENCLAW_STATE_DIR", "PI_CODING_AGENT_DIR", "XDG_DATA_HOME"];

impl Paths {
    /// Real locations. `MOMPUTER_HOME` and `MOMPUTER_DIR` override home and mom's folder.
    pub fn detect() -> Self {
        let home = env_path("MOMPUTER_HOME")
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let mom = env_path("MOMPUTER_DIR").unwrap_or_else(|| home.join(".momputer"));
        let local_appdata = dirs::data_local_dir().unwrap_or_else(|| home.join("AppData").join("Local"));
        let appdata = dirs::data_dir().unwrap_or_else(|| home.join("AppData").join("Roaming"));
        let downloads = dirs::download_dir().unwrap_or_else(|| home.join("Downloads"));
        // System folders from Windows itself, not from environment variables anyone can set.
        let windows = windows_dir();
        let system_drive = windows.ancestors().last().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("C:\\"));
        let other_drives = crate::util::fixed_drives()
            .into_iter()
            .map(PathBuf::from)
            .filter(|d| !d.to_string_lossy().eq_ignore_ascii_case(&system_drive.to_string_lossy()))
            .collect();
        Paths {
            program_data: system_drive.join("ProgramData"),
            program_files_x86: system_drive.join("Program Files (x86)"),
            windows,
            system_drive,
            other_drives,
            home,
            mom,
            temp: std::env::temp_dir(),
            local_appdata,
            appdata,
            downloads,
            agent_env: AGENT_ENV.iter().filter_map(|k| env_path(k).map(|p| (*k, p))).collect(),
        }
    }

    /// A self-contained layout under one folder. Used by tests.
    pub fn under(root: &Path) -> Self {
        Paths {
            home: root.join("home"),
            mom: root.join("home").join(".momputer"),
            temp: root.join("temp"),
            local_appdata: root.join("local"),
            appdata: root.join("roaming"),
            downloads: root.join("home").join("Downloads"),
            windows: root.join("windows"),
            program_data: root.join("programdata"),
            program_files_x86: root.join("program files (x86)"),
            system_drive: root.join("c"),
            other_drives: vec![],
            agent_env: vec![],
        }
    }

    /// An agent's folder as set by `key` (one of AGENT_ENV), if it was.
    pub fn agent_env(&self, key: &str) -> Option<&Path> {
        self.agent_env.iter().find(|(k, _)| *k == key).map(|(_, p)| p.as_path())
    }

    pub fn config_file(&self) -> PathBuf {
        self.mom.join("config.toml")
    }
    pub fn rules_file(&self) -> PathBuf {
        self.mom.join("house-rules.md")
    }
    pub fn state_dir(&self) -> PathBuf {
        self.mom.join("state")
    }
    pub fn journal_dir(&self) -> PathBuf {
        self.mom.join("journal")
    }
    pub fn backups_dir(&self) -> PathBuf {
        self.mom.join("backups")
    }
    pub fn bin_dir(&self) -> PathBuf {
        self.mom.join("bin")
    }
    pub fn installed_cli(&self) -> PathBuf {
        self.bin_dir().join(if cfg!(windows) { "mom.exe" } else { "mom" })
    }
    pub fn claude_dir(&self) -> PathBuf {
        self.home.join(".claude")
    }
    pub fn codex_dir(&self) -> PathBuf {
        self.home.join(".codex")
    }
    /// Shared skills folder that Codex (and other Agent Skills hosts) read.
    pub fn agents_dir(&self) -> PathBuf {
        self.home.join(".agents")
    }
    pub fn gemini_dir(&self) -> PathBuf {
        self.home.join(".gemini")
    }
}
