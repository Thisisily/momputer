//! ~/.momputer/config.toml

use crate::paths::Paths;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
#[derive(Default)]
pub struct Config {
    pub rooms: Vec<PathBuf>,
    pub leftovers: LeftoverConfig,
    pub hooks: HookConfig,
    pub tidy: TidyConfig,
    pub trash: TrashConfig,
    pub files: FilesConfig,
    pub ui: UiConfig,
    /// Substrings (matched against the command line, case-insensitive) that mom never touches.
    pub protect: Vec<String>,
    /// Group signatures the user said to keep. Managed by the app.
    pub kept: Vec<String>,
}


#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct LeftoverConfig {
    /// An agent's process still running after this many hours without doing anything is "forgotten".
    pub forgotten_after_hours: f64,
    /// Treat dev runtimes whose parent is gone (and carry no agent marker) as leftovers too.
    pub include_unknown_orphans: bool,
    /// The widget stops things agents left behind on its own, after a grace period.
    pub auto_stop: bool,
    /// How long something must have been left behind before auto-stop touches it.
    pub auto_stop_after_minutes: u64,
    /// Auto-stop also takes orphans nobody can be traced to (not just agents' leftovers).
    pub auto_stop_orphans: bool,
}

impl Default for LeftoverConfig {
    fn default() -> Self {
        LeftoverConfig {
            forgotten_after_hours: 12.0,
            include_unknown_orphans: true,
            auto_stop: false,
            auto_stop_after_minutes: 20,
            auto_stop_orphans: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct HookConfig {
    /// Master switch. When false every hook exits immediately and silently.
    pub enabled: bool,
    /// Stop hook: remind the agent about processes it left running.
    pub nag_processes: bool,
    /// Stop hook: remind the agent about clutter it added to the project root.
    pub nag_clutter: bool,
    /// SessionStart hook: hand the agent the house rules.
    pub share_rules: bool,
    /// SessionEnd hook: stop whatever the session left running (off by default).
    pub stop_on_session_end: bool,
    /// Ignore processes younger than this (seconds) so quick commands aren't flagged.
    pub min_age_secs: u64,
}

impl Default for HookConfig {
    fn default() -> Self {
        HookConfig {
            enabled: true,
            nag_processes: true,
            nag_clutter: true,
            share_rules: true,
            stop_on_session_end: false,
            min_age_secs: 5,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct TidyConfig {
    /// Where reports go.
    pub reports_folder: String,
    /// How many look-alike folders make a "family" worth grouping.
    pub min_family: usize,
    /// How many screenshots in one place count as a pile.
    pub screenshot_pile: usize,
}

impl Default for TidyConfig {
    fn default() -> Self {
        TidyConfig {
            reports_folder: "Documents".into(),
            min_family: 3,
            screenshot_pile: 10,
        }
    }
}

/// Where the Files scan looks, beyond and instead of the usual folders.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct FilesConfig {
    /// Drives and folders added to the usual ones ("D:\\", "E:\\Photos").
    pub places: Vec<PathBuf>,
    /// Usual folders switched off, by name ("OneDrive").
    pub skip: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct TrashConfig {
    /// Items in %TEMP% older than this many days are fair game.
    pub temp_min_age_days: u64,
    /// What agents leave behind (scratch, logs, snapshots, worktrees) is kept this many days
    /// before mom suggests clearing it. Anything in use is always kept. A week covers a session
    /// left alone while you work on something else (or another account) for a few days.
    pub agent_scratch_days: u64,
    /// Where old conversations are moved to (a folder on another drive). Unset until the user
    /// picks one.
    pub archive_dir: Option<PathBuf>,
    /// Conversations nobody has opened in this many days are suggested for moving.
    pub conversations_after_days: u64,
    /// Things you said to keep, by path: no clean-up ever takes them (or anything inside them, or
    /// anything they're inside of), whatever is ticked. Managed by the app and `mom trash keep`.
    pub keep: Vec<String>,
    /// Autopilot: once a week, while you're away and the PC is on mains power, mom clears the
    /// clean-ups that are safe without looking. Off unless you switch it on.
    pub autopilot: bool,
}

impl Default for TrashConfig {
    fn default() -> Self {
        TrashConfig {
            temp_min_age_days: 7,
            agent_scratch_days: 7,
            archive_dir: None,
            conversations_after_days: 67,
            keep: vec![],
            autopilot: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct UiConfig {
    pub always_on_top: bool,
    /// The master switch for every notification.
    pub notifications: bool,
    /// "Somebody left the lights on": things agents left running.
    pub notify_leftovers: bool,
    /// Leftovers burning CPU.
    pub notify_meltdown: bool,
    /// What mom turned off by herself.
    pub notify_cleanups: bool,
    /// An agent's copy of one of mom's skills is older than the newest.
    pub notify_skills: bool,
    /// Mom's weekly letter is here (on Sundays).
    pub notify_letter: bool,
    /// Agents often tidy up a minute later, so a leftover has to sit this long before mom says anything.
    pub notify_after_minutes: u64,
    /// At most one leftover notification this often; anything new in between waits for the next one.
    pub notify_every_minutes: u64,
    pub scan_interval_secs: u64,
    /// Leftovers using more than this share of the machine's CPU set off the meltdown alarm.
    pub cpu_alert_percent: f32,
    /// Ask GitHub once a day whether there's a newer momputer. The only thing she asks the internet.
    pub check_updates: bool,
    /// Measure as administrator too: pressing Measure asks Windows first, so mom can see the size
    /// of what only an administrator can open (Windows' memory snapshots, say).
    pub measure_as_admin: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig {
            always_on_top: false,
            notifications: true,
            notify_leftovers: true,
            notify_meltdown: true,
            notify_cleanups: true,
            notify_skills: true,
            notify_letter: true,
            notify_after_minutes: 10,
            notify_every_minutes: 30,
            scan_interval_secs: 3,
            cpu_alert_percent: 25.0,
            check_updates: true,
            measure_as_admin: false,
        }
    }
}

impl Config {
    /// Loads the config, falling back to defaults. A broken file is left alone, never overwritten.
    pub fn load(paths: &Paths) -> Config {
        Self::try_load(paths).unwrap_or_default()
    }

    pub fn try_load(paths: &Paths) -> anyhow::Result<Config> {
        let file = paths.config_file();
        if !file.exists() {
            return Ok(Config::default());
        }
        let text = std::fs::read_to_string(&file)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn save(&self, paths: &Paths) -> anyhow::Result<()> {
        let text = toml::to_string_pretty(self)?;
        let header = "# momputer settings. The app edits this file too; comments may not survive.\n\n";
        crate::util::write_atomic(&paths.config_file(), &format!("{header}{text}"))?;
        Ok(())
    }

    pub fn is_protected(&self, cmdline: &str) -> bool {
        let l = cmdline.to_lowercase();
        self.protect
            .iter()
            .any(|p| !p.trim().is_empty() && l.contains(&p.to_lowercase()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let dir = std::env::temp_dir().join(format!("mom-config-{}", std::process::id()));
        let paths = Paths::under(&dir);
        let mut c = Config::default();
        c.rooms.push(PathBuf::from(r"C:\Workspace\storefront"));
        c.protect.push("ollama serve".into());
        c.save(&paths).unwrap();
        let back = Config::try_load(&paths).unwrap();
        assert_eq!(back, c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_keys_fall_back_to_defaults() {
        let c: Config = toml::from_str("[tidy]\nreportsFolder = \"Reports\"\n").unwrap();
        assert_eq!(c.tidy.reports_folder, "Reports");
        assert_eq!(c.tidy.min_family, 3);
        assert!(c.hooks.enabled);
    }
}
