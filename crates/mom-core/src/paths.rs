//! Where things live. Everything takes a `Paths` so tests can point mom at a fake home.

use crate::util::env_path;
use std::path::{Path, PathBuf};

/// How a system lays things out: where tools keep their caches, where the Temp folder is, what
/// the bin is called. Real paths follow the system mom runs on; tests pick, so what's written
/// for one system is tested on the other too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Windows,
    Mac,
}

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
    /// Which system's layout these are.
    pub layout: Layout,
    /// On a Mac, the temp folder everyone shares (/private/tmp), where Claude Code keeps its
    /// scratch space in a folder named for your user number. `temp` is your own ($TMPDIR), which
    /// macOS clears out by itself. On Windows there's only the one, and this is it too.
    pub shared_temp: PathBuf,
    /// Your user number on a Mac (501 for the first person set up on it). 0 on Windows.
    pub uid: u32,
}

/// A place that can't exist, for folders one system has and the other doesn't: nothing is ever
/// found there, and nothing can be put there.
fn nowhere() -> PathBuf {
    PathBuf::from("/var/empty/not-on-this-system")
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

/// What mom says differently on a Mac, longest first.
const MAC_WORDS: &[(&str, &str)] = &[
    ("Setup files (.exe, .msi, .msix)", "Disk images and installer packages (.dmg, .pkg)"),
    ("Windows saves", "macOS saves"),
    ("the Recycle Bin", "the Trash"),
    ("Recycle Bin", "Trash"),
    ("your Temp folder", "the temporary folder"),
    ("the Temp folder", "the temporary folder"),
    ("Temp folder", "temporary folder"),
    ("folders in Temp", "temporary folders"),
    ("in Temp ", "in the temporary folder "),
    ("File Explorer", "Finder"),
    ("Task Manager", "Activity Monitor"),
    ("on this PC", "on this Mac"),
    ("this PC", "this Mac"),
    ("your PC", "your Mac"),
    ("the PC", "the Mac"),
    ("a PC", "a Mac"),
    ("PC's", "Mac's"),
    ("another drive", "another disk"),
];

/// Environment variables that move an agent's folder.
const AGENT_ENV: &[&str] = &["HERMES_HOME", "OPENCLAW_STATE_DIR", "PI_CODING_AGENT_DIR", "XDG_DATA_HOME"];

impl Paths {
    /// Real locations. `MOMPUTER_HOME` and `MOMPUTER_DIR` override home and mom's folder.
    pub fn detect() -> Self {
        let home = env_path("MOMPUTER_HOME")
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let mom = env_path("MOMPUTER_DIR").unwrap_or_else(|| home.join(".momputer"));
        if cfg!(target_os = "macos") {
            let library = home.join("Library");
            #[cfg(unix)]
            let uid = unsafe { libc::getuid() }; // SAFETY: takes nothing and always answers.
            #[cfg(not(unix))]
            let uid = 0;
            return Paths {
                // Apps keep on a Mac in one folder what Windows splits over two.
                local_appdata: library.join("Application Support"),
                appdata: library.join("Application Support"),
                downloads: dirs::download_dir().unwrap_or_else(|| home.join("Downloads")),
                temp: crate::util::temp_dir(),
                shared_temp: PathBuf::from("/private/tmp"),
                windows: nowhere().join("Windows"),
                program_data: nowhere().join("ProgramData"),
                program_files_x86: nowhere().join("Program Files (x86)"),
                system_drive: nowhere().join("C"),
                other_drives: vec![],
                agent_env: AGENT_ENV.iter().filter_map(|k| env_path(k).map(|p| (*k, p))).collect(),
                layout: Layout::Mac,
                uid,
                home,
                mom,
            };
        }
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
            temp: crate::util::temp_dir(),
            shared_temp: crate::util::temp_dir(),
            local_appdata,
            appdata,
            downloads,
            agent_env: AGENT_ENV.iter().filter_map(|k| env_path(k).map(|p| (*k, p))).collect(),
            layout: Layout::Windows,
            uid: 0,
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
            layout: Layout::Windows,
            shared_temp: root.join("temp"),
            uid: 0,
        }
    }

    /// The same, laid out like a Mac. Used by tests.
    pub fn under_mac(root: &Path) -> Self {
        let home = root.join("Users").join("me");
        Paths {
            mom: home.join(".momputer"),
            temp: root.join("var").join("folders").join("T"),
            shared_temp: root.join("private").join("tmp"),
            local_appdata: home.join("Library").join("Application Support"),
            appdata: home.join("Library").join("Application Support"),
            downloads: home.join("Downloads"),
            windows: root.join("not-on-a-mac").join("Windows"),
            program_data: root.join("not-on-a-mac").join("ProgramData"),
            program_files_x86: root.join("not-on-a-mac").join("Program Files (x86)"),
            system_drive: root.join("not-on-a-mac").join("C"),
            other_drives: vec![],
            agent_env: vec![],
            layout: Layout::Mac,
            uid: 501,
            home,
        }
    }

    pub fn is_mac(&self) -> bool {
        self.layout == Layout::Mac
    }

    /// ~/Library on a Mac: where apps keep their settings, caches and logs.
    pub fn library(&self) -> PathBuf {
        self.home.join("Library")
    }

    /// Where apps and tools keep what they can make again: ~/Library/Caches on a Mac,
    /// %LOCALAPPDATA% on Windows.
    pub fn caches(&self) -> PathBuf {
        match self.layout {
            Layout::Mac => self.library().join("Caches"),
            Layout::Windows => self.local_appdata.clone(),
        }
    }

    /// Where Claude Code keeps its scratch space, a folder for each project and session in it:
    /// %TEMP%\claude on Windows, /tmp/claude-<your user number> on a Mac.
    pub fn claude_scratch(&self) -> PathBuf {
        match self.layout {
            Layout::Mac => self.shared_temp.join(format!("claude-{}", self.uid)),
            Layout::Windows => self.temp.join("claude"),
        }
    }

    /// Mom's words for this system: "Recycle Bin" is "Trash" on a Mac, "your PC" is "your Mac".
    /// Only ever for what mom says herself, never for a name or a path she found.
    pub fn say(&self, text: &str) -> String {
        match self.layout {
            Layout::Windows => text.to_string(),
            Layout::Mac => MAC_WORDS.iter().fold(text.to_string(), |t, (from, to)| t.replace(from, to)),
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
