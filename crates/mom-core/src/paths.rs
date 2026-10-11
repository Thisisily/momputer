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
    /// Folders agents were told to use instead of their usual one (HERMES_HOME and the like),
    /// read from the environment once. Empty in tests, so they never reach the real ones.
    pub agent_env: Vec<(&'static str, PathBuf)>,
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
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let system_drive = PathBuf::from(format!("{}\\", drive.trim_end_matches('\\')));
        Paths {
            windows: env_path("SystemRoot").unwrap_or_else(|| system_drive.join("Windows")),
            program_data: env_path("ProgramData").unwrap_or_else(|| system_drive.join("ProgramData")),
            program_files_x86: env_path("ProgramFiles(x86)").unwrap_or_else(|| system_drive.join("Program Files (x86)")),
            system_drive,
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
