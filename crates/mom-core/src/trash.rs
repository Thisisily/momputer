//! Taking out the trash: temp folders, caches and leftovers that pile up and never get emptied.
//!
//! Every spot says exactly what its files are, lists them, and says what cleaning does. Most of
//! it is disposable by definition (it regenerates, or was scratch space for a session that's
//! over), so it is deleted for real. Things a person might want back (old installers) go to the
//! Recycle Bin, and "review" spots ask the user to look first. Mom skips anything a running
//! process is using and anything touched recently.

pub(crate) mod bin;
mod browsers;
mod builds;
mod claude;
mod forgotten;
mod harness;
mod shader_owner;
mod steam_shaders;
mod walk;
mod windows_update;

use crate::agents::AgentKind;
use crate::config::{Config, TrashConfig};
use crate::paths::Paths;
use crate::procs::ProcTable;
use crate::util::{human_age_long, human_bytes, norm_path, now_secs, plural};
use walk::{remove_all, remove_owned, remove_without_following, tally, tally_many, Tally};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spot {
    pub id: String,
    pub label: String,
    /// One short line. `what` and `effect` say more.
    pub detail: String,
    /// "agents", "dev", "windows" or "personal".
    pub group: String,
    /// What these files are, in plain English.
    pub what: String,
    /// Exactly what the clean button does, with the numbers.
    pub effect: String,
    /// "delete" (gone for good), "recycle" (to the Recycle Bin), "tool" (runs a program that
    /// decides) or "empty-bin" (empties the Recycle Bin).
    pub action: String,
    /// "safe" (regenerates, or scratch of finished work) or "review" (look at the list first;
    /// items can be unticked).
    pub safety: String,
    pub path: Option<String>,
    pub owner: Option<AgentKind>,
    /// Everything in this spot. For exact spots, total = reclaimable + kept.
    pub total_bytes: u64,
    pub reclaimable_bytes: u64,
    pub reclaimable_items: u64,
    pub kept_bytes: u64,
    pub kept_items: u64,
    /// The part of what's kept that's kept only for being recent (`Item.optional`), which the
    /// user can tick.
    #[serde(default)]
    pub new_bytes: u64,
    #[serde(default)]
    pub new_items: u64,
    /// Only an administrator can clean it; Windows asks first.
    #[serde(default)]
    pub admin: bool,
    /// The sizes are exact. False when a tool decides what to prune (reclaimable is then an
    /// upper bound) or a folder was too big to count in full.
    pub exact: bool,
    pub available: bool,
    pub measured_at: u64,
    /// A blocker or a tip ("Close Edge to clear its cache.").
    pub note: Option<String>,
    /// Biggest first: up to 300 that go, then up to 150 that stay (what you said to keep comes
    /// first among those). More of each when the widget asked to see more.
    pub items: Vec<Item>,
}

/// One thing in a spot: a folder, a file, a browser profile's cache, a deleted file.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub name: String,
    /// What's inside ("38 files, mostly images"), or where it came from.
    pub sub: Option<String>,
    /// Where it lives. This is what a `Pick` names it by. Tool spots use made-up paths
    /// ("docker:build-cache") that are only good for picking.
    pub path: String,
    pub bytes: u64,
    pub files: u64,
    /// Since it last changed (or, in the Recycle Bin, since it was deleted).
    pub age_secs: u64,
    /// False when the age could not be established; zero seconds then means unknown, not new.
    #[serde(default)]
    pub age_known: bool,
    /// Cleaning takes it (or, in a review spot, takes it unless the user unticks it).
    pub goes: bool,
    /// Stays to be careful (it's recent, or the newest older version kept as a fallback) and
    /// nothing is using it, so the user may tick it to clean it anyway. Only what a running
    /// program is using, or Docker or a database still needs, is never optional.
    #[serde(default)]
    pub optional: bool,
    /// Why it goes or stays ("Last changed 3 days ago", "Edge is open").
    pub reason: String,
    /// What sort of thing it is, plural ("images", "logs"), in spots that hold several sorts.
    #[serde(default)]
    pub kind: Option<String>,
    /// You said to keep it (`TrashConfig.keep`), so it stays until you say otherwise.
    #[serde(default)]
    pub kept: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanReport {
    pub id: String,
    pub freed: u64,
    /// Items deleted, recycled or emptied.
    pub deleted: u64,
    /// Items that couldn't be removed (in use or blocked by permissions) or no longer qualified.
    pub skipped: u64,
    pub message: String,
}

/// A clean-up as it happens: how far along it is, what just went, and a way to stop it. The app
/// keeps one, hands it to `clean` through `TrashCtx::live` and reads it while the clean runs.
#[derive(Default)]
pub struct Live {
    stop: AtomicBool,
    done: AtomicU64,
    total: AtomicU64,
    freed: AtomicU64,
    /// Files deleted so far, counted inside big folders too, so a long one still moves.
    files: AtomicU64,
    total_files: AtomicU64,
    /// Items gone so far, by path, in order.
    gone: Mutex<Vec<String>>,
    /// The item being cleaned right now.
    current: Mutex<String>,
    /// In tests: ask to stop once this many items have finished (0: never), the way someone
    /// pressing Stop partway would.
    #[cfg(test)]
    stop_after: AtomicU64,
}

/// How a clean-up is going, for the widget.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveNow {
    pub done: u64,
    pub total: u64,
    pub freed: u64,
    pub files: u64,
    pub total_files: u64,
    pub current: String,
    /// Paths gone since the `since` the widget asked with.
    pub gone: Vec<String>,
    /// How many have gone in all (what to ask from next time).
    pub gone_count: usize,
    pub stopping: bool,
}

impl Live {
    /// Asks the clean-up to stop after what it's doing now.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Ready for a new clean-up.
    pub fn reset(&self) {
        self.stop.store(false, Ordering::SeqCst);
        for n in [&self.done, &self.total, &self.freed, &self.files, &self.total_files] {
            n.store(0, Ordering::SeqCst);
        }
        self.gone.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.current.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    pub fn now(&self, since: usize) -> LiveNow {
        let gone = self.gone.lock().unwrap_or_else(|e| e.into_inner());
        LiveNow {
            done: self.done.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
            freed: self.freed.load(Ordering::Relaxed),
            files: self.files.load(Ordering::Relaxed),
            total_files: self.total_files.load(Ordering::Relaxed),
            current: self.current.lock().unwrap_or_else(|e| e.into_inner()).clone(),
            gone: gone.get(since..).map(<[String]>::to_vec).unwrap_or_default(),
            gone_count: gone.len(),
            stopping: self.stopped(),
        }
    }

    fn start(&self, chosen: &[&Cand]) {
        self.total.store(chosen.len() as u64, Ordering::SeqCst);
        self.total_files.store(chosen.iter().map(|c| c.files.max(1)).sum(), Ordering::SeqCst);
    }

    fn working_on(&self, c: &Cand) {
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) = c.name.clone();
    }

    fn finished(&self, c: &Cand, gone: bool, freed: u64) {
        let done = self.done.fetch_add(1, Ordering::Relaxed) + 1;
        self.freed.fetch_add(freed, Ordering::Relaxed);
        if gone {
            self.gone.lock().unwrap_or_else(|e| e.into_inner()).push(c.path.to_string_lossy().to_string());
        }
        #[cfg(test)]
        if self.stop_after.load(Ordering::Relaxed) == done {
            self.stop();
        }
        let _ = done;
    }

    /// A file inside an item went (counted by the remover as it goes).
    pub(super) fn file_gone(&self) {
        self.files.fetch_add(1, Ordering::Relaxed);
    }

    /// A batch of files went at once.
    fn files_gone(&self, n: u64) {
        self.files.fetch_add(n, Ordering::Relaxed);
    }
}

/// How many bytes are in each of these folders (links inside are never followed; a folder too
/// big to count quickly comes back as what was counted).
pub fn folder_bytes(folders: &[PathBuf]) -> Vec<u64> {
    tally_many(folders, ENTRY_CAP, u64::MAX).iter().map(|t| t.bytes).collect()
}

/// What mom knows about the machine right now, so she doesn't throw out anything in use.
pub struct TrashCtx<'a> {
    pub paths: &'a Paths,
    pub cfg: &'a TrashConfig,
    /// Agent session ids that are still alive (their temp folders are off limits).
    pub live_sessions: HashSet<String>,
    /// Normalized exe paths, working folders and command lines of running processes, one per
    /// line (a single string is much faster to search than thousands of little ones).
    pub in_use: String,
    /// Lowercased image names of everything running ("msedge.exe").
    pub running_exes: HashSet<String>,
    /// Where to look for projects' build folders: the rooms from the config plus the usual
    /// workspace folders that exist.
    pub project_roots: Vec<PathBuf>,
    pub docker_running: bool,
    pub now: u64,
    /// Running as administrator: then only the spots that need one are cleaned (see `clean`).
    pub elevated: bool,
    /// What's installed, for the clean-ups that ask. None: they look it up themselves when they
    /// need to. Tests say what's installed, so they don't depend on the PC they run on.
    pub installed: Option<crate::installed::Installed>,
    /// Where a clean-up reports how it's going, and learns it should stop.
    pub live: Option<&'a Live>,
    /// How many times the usual number of items a spot lists (1, unless someone asked to see more).
    pub list_scale: usize,
    /// Isolated tests supply update transitions without querying or changing the host PC.
    #[cfg(test)]
    windows_update_probe: Option<&'a dyn Fn() -> windows_update::State>,
}

impl<'a> TrashCtx<'a> {
    pub fn new(paths: &'a Paths, cfg: &'a Config, table: &ProcTable) -> Self {
        let mut live_sessions = HashSet::new();
        let mut in_use = String::new();
        let mut running_exes = HashSet::new();
        let uuid = regex::Regex::new(r"(?i)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").unwrap();
        for p in table.procs.values() {
            running_exes.insert(p.lname());
            for key in ["CLAUDE_CODE_SESSION_ID", "CODEX_THREAD_ID"] {
                if let Some(s) = p.env(key) {
                    live_sessions.insert(s.to_string());
                }
            }
            let cmd = p.cmdline();
            for m in uuid.find_iter(&cmd) {
                live_sessions.insert(m.as_str().to_lowercase());
            }
            for s in [p.exe.as_deref(), p.cwd.as_deref(), Some(cmd.as_str())].into_iter().flatten() {
                in_use.push_str(&norm_path(s));
                in_use.push('\n');
            }
        }
        TrashCtx {
            paths,
            cfg: &cfg.trash,
            live_sessions,
            in_use,
            running_exes,
            project_roots: builds::default_roots(paths, &cfg.rooms),
            docker_running: crate::containers::docker_running(table),
            now: now_secs(),
            elevated: crate::elevate::is_elevated(),
            installed: None,
            live: None,
            list_scale: 1,
            #[cfg(test)]
            windows_update_probe: None,
        }
    }

    fn in_use(&self, path: &Path) -> bool {
        let p = norm_path(&path.to_string_lossy());
        self.in_use.contains(&p)
    }

    fn session_live(&self, id: &str) -> bool {
        self.live_sessions.contains(id) || self.live_sessions.contains(&id.to_lowercase())
    }

    fn running(&self, exe: &str) -> bool {
        self.running_exes.contains(exe)
    }
}

const IN_USE: &str = "In use by a running program";
const UNKNOWN_AGE: &str = "Age unknown; could not check every file and folder";
const BROWSER_DATA: &str = "Contains browser sign-ins or saved site data";

fn windows_update_state(ctx: &TrashCtx) -> windows_update::State {
    #[cfg(test)]
    { ctx.windows_update_probe.map_or(windows_update::State::Clear, |probe| probe()) }
    #[cfg(not(test))]
    { let _ = ctx; windows_update::probe() }
}

fn windows_update_fast_state(ctx: &TrashCtx) -> windows_update::State {
    #[cfg(test)]
    { ctx.windows_update_probe.map_or(windows_update::State::Clear, |probe| probe()) }
    #[cfg(not(test))]
    { let _ = ctx; windows_update::probe_fast() }
}

/// Identify actual update locations, not arbitrary folders with similar names. The tool
/// label also covers a candidate whose configured root uses a filesystem alias.
fn windows_update_candidate(c: &Cand, ctx: &TrashCtx) -> bool {
    matches!(c.tool, Some("Windows Update" | "Windows setup"))
        || c.targets.iter().any(|path| windows_update_target(path, ctx.paths))
}

fn windows_update_target(path: &Path, paths: &Paths) -> bool {
    let key = norm_path(&path.to_string_lossy());
    let download = norm_path(&paths.windows.join("SoftwareDistribution").join("Download").to_string_lossy());
    if key == download || key.starts_with(&format!("{download}\\")) { return true; }
    std::iter::once(&paths.system_drive).chain(&paths.other_drives).any(|drive| {
        let root = norm_path(&drive.to_string_lossy());
        key.strip_prefix(&format!("{root}\\"))
            .and_then(|rel| rel.split('\\').next()).is_some_and(harness::setup_folder)
    })
}
/// Projects untouched this long count as idle; their build folders can go.
const IDLE_PROJECT_DAYS: u64 = 30;
/// Installers in Downloads older than this can go.
const OLD_INSTALLER_DAYS: u64 = 30;
/// Crash reports younger than this stay, in case someone wants to send one.
const CRASH_MIN_AGE: u64 = 86_400;
/// Render folders are only needed while the render runs; a day covers a long one.
const RENDER_MIN_AGE: u64 = 86_400;
/// Stop counting a single folder after this many entries (and say the size is a floor).
const ENTRY_CAP: u64 = 2_000_000;

/// How much each path holds, and when anything in it last changed: (bytes, newest mtime). Many
/// at once on the walker pool, for other parts of mom (browser profiles, say).
pub(crate) fn sizes(paths: &[PathBuf]) -> Vec<(u64, u64)> {
    tally_many(paths, ENTRY_CAP, u64::MAX).into_iter().map(|t| (t.bytes, if t.complete { t.newest } else { 0 })).collect()
}

/// Conservative cleanup recency, not last use. Windows creation time prevents a newly copied
/// or extracted file from inheriting the age of its old contents. Other targets use modified
/// time; portable Rust cannot require or backdate creation times there.
pub(crate) fn cleanup_recency(meta: &std::fs::Metadata) -> Option<u64> {
    fn timestamp(at: std::time::SystemTime) -> Option<u64> {
        let secs = at.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
        (secs > 0).then_some(secs)
    }
    let modified = timestamp(meta.modified().ok()?)?;
    #[cfg(windows)]
    { Some(modified.max(timestamp(meta.created().ok()?)?)) }
    #[cfg(not(windows))]
    { Some(modified) }
}

struct Def {
    id: &'static str,
    label: &'static str,
    detail: &'static str,
    what: &'static str,
    group: &'static str,
    action: &'static str,
    safety: &'static str,
    owner: Option<AgentKind>,
}

const DEFS: &[Def] = &[
    Def {
        id: "claude-temp",
        label: "Claude Code scratch",
        detail: "Scratchpads and task output from Claude Code sessions that have ended.",
        what: "Scratchpads, screenshots and task output Claude Code saved in your Temp folder while it worked. \
               Every session gets its own folder, and once the session is over nothing reads it again.",
        group: "agents",
        action: "delete",
        safety: "safe",
        owner: Some(AgentKind::ClaudeCode),
    },
    Def {
        id: "codex-temp",
        label: "Codex leftovers",
        detail: "Codex index and clipboard temp files, half-written state files and old sandbox logs.",
        what: "Temporary files Codex makes as it works: search indexes of your projects, pasted clipboard images, \
               its temp folders, half-written copies of its state files and sandbox logs.",
        group: "agents",
        action: "delete",
        safety: "safe",
        owner: Some(AgentKind::Codex),
    },
    Def {
        id: "claude-leftovers",
        label: "Claude leftovers",
        detail: "Debug logs, shell snapshots, rewind backups and pasted text from old sessions, and the Claude app's old logs.",
        what: "What Claude Code keeps besides your chats: debug logs, snapshots of your shell, backups of files it \
               edited (so a session can be rewound), pasted text, MCP server logs, half-written copies of its settings \
               file and its folders in Temp. Also the Claude app's old logs, crash reports and older built-in versions.",
        group: "agents",
        action: "delete",
        safety: "safe",
        owner: Some(AgentKind::ClaudeCode),
    },
    Def {
        id: "agent-leftovers",
        label: "Other agents' leftovers",
        detail: "Logs, temp files, caches and snapshots other agent tools leave behind.",
        what: "Logs, temp files, caches, undo snapshots and debug output from the other agent tools on this PC: \
               OpenCode, OpenClaw, Hermes, Pi, Copilot, Gemini, Qwen, Goose, Cline, Roo, Kilo, Continue, Kimi, Vibe, \
               Junie, Mux, Crush, Droid, Zed, Warp, Windsurf, Kiro, Aider and others. Only their leftover folders are \
               looked in.",
        group: "agents",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "app-leftovers",
        label: "Old app versions and updates",
        detail: "Caches of old versions, downloaded extension packages, crash reports and update installers apps keep.",
        what: "VS Code and the editors built on it (Cursor, Windsurf, Kiro, Trae, Qoder, Antigravity) keep a start-up \
               cache for every build they've run, every extension package they downloaded, crash reports and logs. \
               Apps that update themselves keep the versions they updated from and the last installer they \
               downloaded. None of it is used again.",
        group: "dev",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "old-conversations",
        label: "Old conversations",
        detail: "Chats that haven't changed in a while, moved to another drive with a link left in their place.",
        what: "Your agents' past conversations: Codex's and Claude Code's chats, images Codex made, and other agents' \
               sessions. They add up to gigabytes. Moving the old ones to another drive frees this one, and a link in \
               each one's place means the agents still find them where they were.",
        group: "agents",
        action: "move",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "cursor-worktrees",
        label: "Old Cursor worktrees",
        detail: "Copies of your projects Cursor's agents worked in.",
        what: "When Cursor runs agents side by side, each gets its own copy of the project (a Git worktree) in \
               ~/.cursor/worktrees. Cursor doesn't always remove them when the work is done.",
        group: "agents",
        action: "recycle",
        safety: "review",
        owner: Some(AgentKind::Cursor),
    },
    Def {
        id: "render-temp",
        label: "Render leftovers",
        detail: "Remotion and bundler build folders left in Temp after video renders.",
        what: "Copies of your project that Remotion and other bundlers put in your Temp folder while rendering a video. \
               They're only needed while the render runs.",
        group: "dev",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "npx-cache",
        label: "npx downloads",
        detail: "Packages agents fetched with npx. Unused ones are re-downloaded if ever needed.",
        what: "Packages downloaded to run a one-off command with npx. Agents do this a lot (MCP servers, CLIs). \
               Each folder is one command's packages.",
        group: "dev",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "npm-cache",
        label: "npm cache",
        detail: "npm's download cache. Safe to clear; it refills as you install.",
        what: "A copy of every package npm has downloaded, kept so the next install is faster. \
               Your projects don't depend on it.",
        group: "dev",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "pnpm-store",
        label: "pnpm store",
        detail: "Runs `pnpm store prune`, which removes packages no project uses.",
        what: "pnpm keeps one shared copy of every package your projects use and links projects to it. \
               Packages no project links to any more are dead weight.",
        group: "dev",
        action: "tool",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "yarn-cache",
        label: "Yarn cache",
        detail: "Yarn's download cache. Safe to clear; it refills as you install.",
        what: "A copy of every package Yarn has downloaded, kept so the next install is faster.",
        group: "dev",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "bun-cache",
        label: "bun cache",
        detail: "bun's package cache. Safe to clear; it refills as you install.",
        what: "A copy of every package bun has downloaded, kept so the next install is faster.",
        group: "dev",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "pip-cache",
        label: "pip cache",
        detail: "pip's download cache. Safe to clear.",
        what: "Python packages pip has downloaded, kept so the next install is faster.",
        group: "dev",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "uv-cache",
        label: "uv cache",
        detail: "Runs `uv cache prune`, which keeps what your environments still use.",
        what: "uv's shared store of Python packages. Your virtual environments use some of it; \
               the rest is old downloads.",
        group: "dev",
        action: "tool",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "cargo-cache",
        label: "Cargo downloads",
        detail: "Crate archives and git checkouts Cargo downloaded. Re-downloaded when needed.",
        what: "Rust crates Cargo downloaded (as compressed archives) and git dependencies it checked out. \
               The unpacked sources your builds read are kept separately.",
        group: "dev",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "go-build-cache",
        label: "Go build cache",
        detail: "Compiled Go packages, kept to make the next build faster.",
        what: "Compiled pieces of Go code, kept so the next build is faster.",
        group: "dev",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "nuget-cache",
        label: "NuGet cache",
        detail: "NuGet's HTTP cache of package lists and downloads.",
        what: ".NET package lists and downloads NuGet cached from the internet.",
        group: "dev",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "gradle-cache",
        label: "Gradle cache",
        detail: "Gradle's downloaded dependencies and build caches.",
        what: "Dependencies Gradle downloaded and the build caches it keeps for Java and Android projects.",
        group: "dev",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "electron-cache",
        label: "Electron downloads",
        detail: "Electron runtimes downloaded while installing Electron apps.",
        what: "Electron runtimes downloaded while installing or building Electron apps.",
        group: "dev",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "playwright-old",
        label: "Old Playwright browsers",
        detail: "Older browser builds Playwright downloaded. The newest of each is kept.",
        what: "Browsers Playwright downloaded to run tests and drive web pages. Every update downloads a new build \
               and leaves the old one behind.",
        group: "dev",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "huggingface-cache",
        label: "Hugging Face models",
        detail: "AI models and datasets downloaded from Hugging Face.",
        what: "AI models and datasets that Python libraries (transformers, diffusers and friends) downloaded \
               from Hugging Face. They can be huge.",
        group: "dev",
        action: "delete",
        safety: "review",
        owner: None,
    },
    Def {
        id: "old-build-folders",
        label: "Old build folders",
        detail: "node_modules and build output in projects with no recorded changes for a month.",
        what: "Installed packages (node_modules) and build output (Rust's target, .next, .turbo and the like) inside \
               projects with no recorded changes for a month. Your code isn't in them; they're rebuilt from it.",
        group: "dev",
        action: "delete",
        safety: "review",
        owner: None,
    },
    Def {
        id: "docker",
        label: "Docker junk",
        detail: "Build cache, untagged images, and images and stopped containers you pick. Volumes are kept.",
        what: "Images Docker keeps (each one listed with the containers that use it), stopped containers, and the \
               build cache. Volumes, where databases keep their data, are never touched.",
        group: "dev",
        action: "delete",
        safety: "review",
        owner: None,
    },
    Def {
        id: "old-temp",
        label: "Old temp files",
        detail: "Temp files with no creation or modification recorded in the last week.",
        what: "Files programs and installers left in your Temp folder and never cleaned up.",
        group: "windows",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "browser-cache",
        label: "Browser caches",
        detail: "Saved copies of web pages, images and scripts. Browsers and apps rebuild them as you go.",
        what: "Copies of web pages, images and scripts your browsers saved so sites load faster, and the same caches \
               inside apps built like browsers (Cursor, VS Code, Claude, Codex, Slack, Discord and Epic Games). \
               Cookies, sign-ins, site storage and session recovery data are kept.",
        group: "windows",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "crash-dumps",
        label: "Crash reports",
        detail: "Memory dumps and error reports from apps that crashed.",
        what: "Memory snapshots and error reports Windows saves when an app crashes. \
               They're only useful to the app's developers.",
        group: "windows",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "recycle-bin",
        label: "Recycle Bin",
        detail: "Files you deleted. They take up space until the bin is emptied.",
        what: "Files and folders you deleted. They sit in the Recycle Bin, taking up space, until it's emptied.",
        group: "windows",
        action: "empty-bin",
        safety: "review",
        owner: None,
    },
    Def {
        id: "shader-cache",
        label: "Graphics shader caches",
        detail: "Compiled shaders, with game or app names where the cache records them. Cleared shaders are rebuilt when needed.",
        what: "When a game or app draws something new, the graphics driver (NVIDIA, AMD, Intel) or DirectX compiles \
               a shader for it and keeps it, so next time is quicker. Shaders of games you no longer play just sit \
               there; these caches easily reach gigabytes. Mom shows an app name when the cache records it, or a game name \
               from Steam's library records. A missing executable or Steam entry can mean an uninstall, a move or an update; \
               it is not proof the app was removed. Unknown owners stay marked unknown. Steam caches start unticked.",
        group: "windows",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "game-junk",
        label: "Game logs and crash reports",
        detail: "Logs and crash reports games write every time they run.",
        what: "Unity games write a Player.log every time they run, Unreal games keep logs and crash reports, and \
               Godot games keep shader caches. Your saves and settings live next to them and are never touched.",
        group: "windows",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "dev-leftovers",
        label: "Other developer caches",
        detail: "Caches of Dart, Flutter, Cypress, act, Firebase, node-gyp and VS Code's language tools.",
        what: "Developer tools without a clean-up of their own keep caches too: Dart and Flutter's package and \
               analysis caches, Cypress and Playwright downloads, act's runner images, Firebase emulators, node-gyp's \
               Node headers, VS Code's C++ and TypeScript caches, Gradle versions, and installer leftovers.",
        group: "dev",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "windows-junk",
        label: "Windows leftovers",
        detail: "Windows update downloads, old error reports, and setup folders at the top of your drives.",
        what: "Windows Update keeps downloaded files, and Windows keeps error reports \
               about crashes. Windows setup and the update assistant leave their working folders at the top of \
               whichever drive had room, a disk check leaves the pieces it recovered there (found.000), and a few \
               installers (AMD, Adobe) leave a folder on C:. Mom looks at the top of every drive built into the PC. \
               Update and setup files stay while updates or a restart are pending, or their status cannot be checked. \
               Cleaning eligible items needs an administrator, so Windows asks you first.",
        group: "windows",
        action: "delete",
        safety: "review",
        owner: None,
    },
    Def {
        id: "admin-leftovers",
        label: "Apps' leftovers that need admin",
        detail: "Installer caches, downloaded samples, settings and history backups, crash dumps and logs in ProgramData.",
        what: "Some apps keep these files where only an administrator can delete them: NVIDIA's downloaded and extracted \
               installers, Epic's sample downloads, GlassWire's settings and traffic history backups, Razer's crash dumps, \
               Driver Booster's downloads, the debuggers' symbol cache and G HUB's older versions. NVIDIA's cache can \
               include the currently installed driver version; removing it may require another download for reinstall \
               or rollback. Deleting a backup discards the settings and history saved in it. Windows asks you first.",
        group: "windows",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "forgotten-apps",
        label: "Forgotten app data",
        detail: "Big folders of apps you uninstalled, older versions, and data with no recent changes.",
        what: "Big folders apps keep in AppData and your home folder: the data of apps you've since uninstalled, \
               folders of an app's older versions, and data with no recently recorded changes. Mom guesses which \
               app each belongs to, so review the folders first. Archive moves them to your archive drive with a \
               link left behind. You can instead send selected folders to the Recycle Bin or delete them for good. \
               Keys, passwords, wallets and browser profiles are never offered.",
        group: "personal",
        action: "move",
        safety: "review",
        owner: None,
    },
    Def {
        id: "old-installers",
        label: "Old installers",
        detail: "Setup files in Downloads older than a month.",
        what: "Setup files (.exe, .msi, .msix) in your Downloads folder that are over a month old. \
               Once a program is installed, its installer isn't needed.",
        group: "personal",
        action: "recycle",
        safety: "review",
        owner: None,
    },
];

/// The last measurement, saved so a restart doesn't mean measuring everything again.
fn saved_file(paths: &Paths) -> PathBuf {
    paths.state_dir().join("trash.json")
}

/// The mom that's running. What goes, what stays and what can be ticked is her verdict, and a
/// newer mom may judge differently, so a measurement is saved with the version that made it.
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Changes whenever stored measurement evidence/verdicts become incompatible.
const SCAN_VERSION: u64 = 3;

pub fn save(paths: &Paths, spots: &[Spot]) {
    save_as(paths, spots, VERSION);
}

fn save_as(paths: &Paths, spots: &[Spot], version: &str) {
    // The usual number of items each, however many the widget is showing: the file is read at
    // every start.
    let spots: Vec<Spot> = spots
        .iter()
        .map(|s| {
            let mut s = s.clone();
            let (goes, stays): (Vec<Item>, Vec<Item>) = std::mem::take(&mut s.items).into_iter().partition(|i| i.goes);
            s.items = goes.into_iter().take(MAX_GOES).chain(stays.into_iter().take(MAX_STAYS)).collect();
            s
        })
        .collect();
    if let Ok(json) = serde_json::to_string(&serde_json::json!({ "version": version, "scanVersion": SCAN_VERSION, "spots": spots })) {
        let _ = crate::util::write_atomic(&saved_file(paths), &json);
    }
}

/// What was measured last time (each spot says when). Spots mom no longer has are dropped, and
/// so is everything another version measured: after an update she looks again, or the widget
/// would go on showing the old version's verdicts.
pub fn load_saved(paths: &Paths) -> Vec<Spot> {
    let ids = spot_ids();
    std::fs::read_to_string(saved_file(paths))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .filter(|v| v.get("version").and_then(serde_json::Value::as_str) == Some(VERSION))
        .filter(|v| v.get("scanVersion").and_then(serde_json::Value::as_u64) == Some(SCAN_VERSION))
        .and_then(|mut v| serde_json::from_value::<Vec<Spot>>(v["spots"].take()).ok())
        .map(|v| v.into_iter().filter(|s| ids.contains(&s.id.as_str())).collect())
        .unwrap_or_default()
}

pub fn spot_ids() -> Vec<&'static str> {
    DEFS.iter().map(|d| d.id).collect()
}

fn def(id: &str) -> Option<&'static Def> {
    DEFS.iter().find(|d| d.id == id)
}

/// Spots `mom trash clean all` (and anything else that cleans without asking) may take:
/// safe, and decided by mom rather than a tool.
pub fn cleans_without_asking(id: &str) -> bool {
    def(id).map(|d| d.safety == "safe" && !matches!(d.action, "tool" | "move") && !needs_admin(d.id)).unwrap_or(false)
}

/// Spots only an administrator can clean: the app asks Windows to run `mom` elevated for them.
const ADMIN: &[&str] = &["windows-junk", "admin-leftovers"];

pub fn needs_admin(id: &str) -> bool {
    ADMIN.contains(&id)
}

// ------------------------------------------------------------------ finding things

/// Something mom found in a spot, before the list is trimmed for the UI.
struct Cand {
    /// What the UI shows and a `Pick` names it by.
    path: PathBuf,
    /// What actually gets removed. Usually just `path`; a browser profile lists its cache folders,
    /// a whole cache lists its contents (so the cache folder itself stays).
    targets: Vec<PathBuf>,
    name: String,
    sub: Option<String>,
    bytes: u64,
    files: u64,
    age: u64,
    /// Age-based cleanup needs complete, valid timestamp evidence. Other cleanups (a browser
    /// cache or a Recycle Bin record, for example) have their own basis for removal.
    age_known: bool,
    age_required: bool,
    browser_data: bool,
    goes: bool,
    /// Kept only for being recent: the user can tick it.
    optional: bool,
    /// What sort of thing it is, plural ("logs"), for spots that hold several sorts.
    kind: Option<&'static str>,
    /// Which tool left it, for spots that cover several.
    tool: Option<&'static str>,
    /// Links inside it (or it is one). Agent folders hold links to shared skills; a folder with
    /// one is never treated as empty or as a leftover.
    links: u64,
    reason: String,
    complete: bool,
    /// A Claude transcript to read the session's title from, and the project name to put in
    /// front. Only read for items that get shown.
    title_from: Option<(PathBuf, String)>,
    /// You said to keep it.
    kept: bool,
}

impl Cand {
    fn new(path: PathBuf, name: impl Into<String>, t: &Tally, now: u64) -> Cand {
        Cand {
            targets: vec![path.clone()],
            path,
            name: name.into(),
            sub: t.sub(),
            bytes: t.bytes,
            files: t.files,
            age: if t.complete && t.recency > 0 { now.saturating_sub(t.recency) } else { 0 },
            age_known: t.complete && t.recency > 0,
            age_required: false,
            browser_data: t.contains_browser_data,
            goes: false,
            optional: false,
            kind: None,
            tool: None,
            links: t.links,
            reason: String::new(),
            complete: t.complete,
            title_from: None,
            kept: false,
        }
    }

    fn verdict(mut self, goes: bool, reason: impl Into<String>) -> Cand {
        if self.browser_data {
            self.goes = false;
            self.optional = false;
            self.reason = BROWSER_DATA.into();
            return self;
        }
        if self.age_required && (!self.age_known || !self.complete) {
            self.goes = false;
            self.optional = false;
            self.reason = UNKNOWN_AGE.into();
            return self;
        }
        self.goes = goes;
        self.reason = reason.into();
        self
    }

    fn age_sensitive(mut self) -> Cand {
        self.age_required = true;
        self
    }

    /// Stays to be careful (it's recent, say), but the user can tick it.
    fn kept_but_tickable(mut self, reason: impl Into<String>) -> Cand {
        self.optional = true;
        self.verdict(false, reason)
    }

    fn into_item(self) -> Item {
        let mut name = self.name;
        if let Some((transcript, project)) = &self.title_from {
            if let Some(title) = claude::session_title(transcript) {
                name = format!("{project} · {title}");
            }
        }
        Item {
            name,
            sub: self.sub,
            path: self.path.to_string_lossy().to_string(),
            bytes: self.bytes,
            files: self.files,
            age_secs: self.age,
            age_known: self.age_known,
            goes: self.goes,
            optional: self.optional,
            reason: self.reason,
            kind: self.kind.map(str::to_string),
            kept: self.kept,
        }
    }
}

struct Found {
    root: Option<PathBuf>,
    cands: Vec<Cand>,
    exact: bool,
    available: bool,
    note: Option<String>,
    /// Tool spots: (bytes on disk, what the tool could give back at most).
    tool: Option<(u64, u64)>,
}

impl Found {
    fn none() -> Self {
        Found { root: None, cands: vec![], exact: true, available: false, note: None, tool: None }
    }

    fn at(root: PathBuf) -> Self {
        Found { root: Some(root), cands: vec![], exact: true, available: true, note: None, tool: None }
    }
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default()
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn is_render_leftover(name: &str) -> bool {
    static BUNDLE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"-bundle-(?:[a-z0-9]+-)*\d+-\d+$").unwrap());
    let n = name.to_ascii_lowercase();
    n.starts_with("remotion-") || BUNDLE.is_match(&n)
}

fn is_codex_temp(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("codex-index-") || n.starts_with("codex-clipboard-")
}

/// "Changed 2 hours ago (kept for 24 hours)".
fn too_fresh(age: u64, min: u64) -> String {
    format!("Changed {} ago (kept for {})", ago(age), human_age_long(min))
}

fn ago(secs: u64) -> String {
    human_age_long(secs)
}

/// Measures a batch of (path, name) all at once and makes candidates of them (no verdict yet).
fn cands(list: Vec<(PathBuf, String)>, now: u64) -> Vec<Cand> {
    let paths: Vec<PathBuf> = list.iter().map(|(p, _)| p.clone()).collect();
    let tallies = tally_many(&paths, ENTRY_CAP, u64::MAX);
    list.into_iter().zip(tallies).map(|((p, name), t)| Cand::new(p, name, &t, now)).collect()
}

/// The usual verdict for scratch: in use stays, too fresh stays, otherwise it goes.
fn judge(c: Cand, ctx: &TrashCtx, min_age: u64) -> Cand {
    let c = c.age_sensitive();
    if ctx.in_use(&c.path) {
        c.verdict(false, IN_USE)
    } else if c.age < min_age {
        let r = too_fresh(c.age, min_age);
        c.kept_but_tickable(r)
    } else {
        let r = format!("Last changed {} ago", ago(c.age));
        c.verdict(true, r)
    }
}

/// A whole cache folder as one item. Its contents go; the folder itself stays.
fn whole_dir(root: PathBuf, name: &str, ctx: &TrashCtx) -> Found {
    if !root.is_dir() {
        return Found::none();
    }
    let mut f = Found::at(root.clone());
    if let Some(c) = cache_item(root, name, ctx) {
        f.exact = c.complete;
        f.cands.push(c);
    }
    f
}

fn cache_item(root: PathBuf, name: &str, ctx: &TrashCtx) -> Option<Cand> {
    let kids = entries(&root);
    let mut t = Tally::default();
    for k in tally_many(&kids, ENTRY_CAP, u64::MAX) {
        t.add(&k);
    }
    if t.files == 0 {
        return None; // nothing in it but empty folders
    }
    let mut c = Cand::new(root, name, &t, ctx.now);
    c.targets = kids;
    Some(if ctx.in_use(&c.path) { c.verdict(false, IN_USE) } else { c.verdict(true, "Refills by itself when needed") })
}

/// Why something you said to keep stays, and why a folder with such a thing inside it does.
const KEPT: &str = "You said to keep this";
const HOLDS_KEPT: &str = "Holds something you said to keep";

/// How many things can be kept, and how long a path may be: the list is in a file anyone
/// running as you can write, and it's read every time mom measures.
const MAX_KEPT: usize = 2_000;
const MAX_KEPT_PATH: usize = 1_024;

/// Is `inner` the same place as `outer`, or somewhere inside it? Both as `norm_path` spells them.
fn within(inner: &str, outer: &str) -> bool {
    inner == outer || (inner.len() > outer.len() && inner.starts_with(outer) && (outer.ends_with('\\') || inner.as_bytes()[outer.len()] == b'\\'))
}

/// Did you say to keep `path`: the thing itself, or something it's inside of? (`keep` is
/// `TrashConfig.keep`.) Every clean-up asks, right before it takes anything.
pub fn is_kept(path: &Path, keep: &[String]) -> bool {
    let p = norm_path(&path.to_string_lossy());
    keep.iter().filter(|k| !k.trim().is_empty()).any(|k| within(&p, &norm_path(k)))
}

/// What you said to keep never goes and can't be ticked: the thing itself, anything inside it,
/// and anything it is inside of (deleting that would take it along).
fn keep_yours(mut f: Found, keep: &[String]) -> Found {
    let keep: Vec<String> = keep.iter().filter(|k| !k.trim().is_empty()).map(|k| norm_path(k)).collect();
    if keep.is_empty() {
        return f;
    }
    for c in &mut f.cands {
        let places: Vec<String> = std::iter::once(&c.path).chain(&c.targets).map(|p| norm_path(&p.to_string_lossy())).collect();
        let yours = places.iter().any(|p| keep.iter().any(|k| within(p, k)));
        let holds = !yours && places.iter().any(|p| keep.iter().any(|k| within(k, p)));
        if yours || holds {
            c.goes = false;
            c.optional = false;
            c.kept = yours;
            c.reason = if yours { KEPT } else { HOLDS_KEPT }.to_string();
        }
    }
    f
}

/// Says to keep `path` from now on (or, with `on` false, not to any more). Ok(true) if that
/// changed anything, Ok(false) if it was so already. An error when it can't be kept: whoever
/// asked must not go on believing it is. Paths are compared the way Windows does, whatever their
/// capitals and slashes.
pub fn set_keep(cfg: &mut TrashConfig, path: &str, on: bool) -> Result<bool, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("That has no path to keep it by.".into());
    }
    let want = norm_path(path);
    let had = cfg.keep.iter().any(|k| norm_path(k) == want);
    if on == had {
        return Ok(false);
    }
    if on && path.len() > MAX_KEPT_PATH {
        return Err("That path is too long to keep.".into());
    }
    if on && cfg.keep.len() >= MAX_KEPT {
        return Err(format!("That's more than mom can keep ({MAX_KEPT}). Let go of some in Settings first."));
    }
    cfg.keep.retain(|k| norm_path(k) != want);
    if on {
        cfg.keep.push(path.to_string());
    }
    Ok(true)
}

/// Everything a spot holds right now, with what you said to keep already marked as staying.
fn find(id: &str, ctx: &TrashCtx) -> Found {
    let keep = &ctx.cfg.keep[..ctx.cfg.keep.len().min(MAX_KEPT)];
    let mut found = find_raw(id, ctx);
    if id != "docker" && id != "recycle-bin" {
        for c in &mut found.cands {
            if c.browser_data || c.targets.iter().any(|p| crate::browser_data::protects_path(p)) {
                c.browser_data = true;
                c.goes = false;
                c.optional = false;
                c.reason = BROWSER_DATA.into();
            } else if !c.complete || c.links > 0 {
                c.goes = false;
                c.optional = false;
                c.reason = "Could not safely check every file and folder; kept".into();
            }
        }
    }
    keep_yours(found, keep)
}

fn find_raw(id: &str, ctx: &TrashCtx) -> Found {
    let agent_age = ctx.cfg.agent_scratch_days * 86_400;
    let la = &ctx.paths.local_appdata;
    match id {
        "claude-temp" => claude::find(ctx, agent_age),
        "codex-temp" | "claude-leftovers" | "agent-leftovers" | "app-leftovers" | "cursor-worktrees" | "shader-cache" | "game-junk"
        | "dev-leftovers" | "windows-junk" | "admin-leftovers" => harness::find(id, ctx, agent_age),
        "old-conversations" => harness::conversations(ctx, ctx.cfg.conversations_after_days * 86_400),
        "forgotten-apps" => forgotten::find(ctx, &harness::homes(ctx)),
        "render-temp" => {
            let mut f = Found::at(ctx.paths.temp.clone());
            let mut list = Vec::new();
            for p in entries(&ctx.paths.temp) {
                let n = name_of(&p);
                if !is_render_leftover(&n) {
                    continue;
                }
                list.push((p, render_name(&n)));
            }
            f.cands = cands(list, ctx.now).into_iter().map(|c| judge(c, ctx, RENDER_MIN_AGE)).collect();
            f
        }
        "old-temp" => {
            let min = ctx.cfg.temp_min_age_days * 86_400;
            let mut f = Found::at(ctx.paths.temp.clone());
            let owners = harness::temp_owners(ctx);
            let list: Vec<(PathBuf, String)> = entries(&ctx.paths.temp)
                .into_iter()
                .map(|p| {
                    let n = name_of(&p);
                    (p, n)
                })
                // these are counted by their own spots
                .filter(|(_, n)| !n.eq_ignore_ascii_case("claude") && !is_codex_temp(n) && !is_render_leftover(n) && !owners.owns(n))
                .collect();
            f.cands = cands(list, ctx.now).into_iter().map(|c| judge(c, ctx, min)).collect();
            f
        }
        "npx-cache" => {
            let root = la.join("npm-cache").join("_npx");
            if !root.is_dir() {
                return Found::none();
            }
            let mut f = Found::at(root.clone());
            let list: Vec<(PathBuf, String)> = entries(&root).into_iter().map(|p| {
                let n = npx_name(&p);
                (p, n)
            }).collect();
            let deps: Vec<Vec<String>> = list.iter().map(|(p, _)| npx_deps(p)).collect();
            // npx never writes to a folder it runs from again, so its age is when it was fetched,
            // not when it last ran. Hold them as long as agents' other leftovers, and keep any that
            // your shell history or an MCP server set-up names.
            let hold = ctx.cfg.agent_scratch_days * 86_400;
            let signs = npx_signs(ctx.paths);
            for (c, deps) in cands(list, ctx.now).into_iter().zip(deps) {
                let c = c.age_sensitive();
                let named = deps.iter().find_map(|d| {
                    let d = d.to_lowercase();
                    signs.iter().find(|(_, text)| text.contains(&d)).map(|(by, _)| *by)
                });
                let c = if ctx.in_use(&c.path) {
                    c.verdict(false, IN_USE)
                } else if let Some(by) = named {
                    let r = format!("Last changed {} ago; {by}", ago(c.age));
                    c.verdict(false, r)
                } else if c.age < hold {
                    let r = format!("Last changed {} ago (kept for {})", ago(c.age), human_age_long(hold));
                    c.kept_but_tickable(r)
                } else {
                    let r = format!("Last changed {} ago, and nothing mom can see runs it", ago(c.age));
                    c.verdict(true, r)
                };
                f.cands.push(c);
            }
            f
        }
        "npm-cache" => whole_dir(la.join("npm-cache").join("_cacache"), "npm download cache", ctx),
        "pnpm-store" => {
            let root = la.join("pnpm").join("store");
            if !root.is_dir() || crate::util::on_path("pnpm").is_none() {
                return Found::none();
            }
            let t = tally(&root);
            let mut f = Found::at(root);
            f.exact = false;
            f.tool = Some((t.bytes, t.bytes));
            f
        }
        "yarn-cache" => whole_dir(la.join("Yarn").join("Cache"), "Yarn package cache", ctx),
        "bun-cache" => whole_dir(ctx.paths.home.join(".bun").join("install").join("cache"), "bun package cache", ctx),
        "pip-cache" => whole_dir(la.join("pip").join("cache"), "pip download cache", ctx),
        "uv-cache" => {
            let root = la.join("uv").join("cache");
            if !root.is_dir() {
                return Found::none();
            }
            let t = tally(&root);
            let mut f = Found::at(root);
            f.exact = false;
            f.tool = Some((t.bytes, t.bytes));
            if crate::util::on_path("uv").is_none() {
                f.note = Some("uv isn't on your PATH, so mom can't run it.".into());
            }
            f
        }
        "cargo-cache" => {
            let cargo = ctx.paths.home.join(".cargo");
            let parts = [
                (cargo.join("registry").join("cache"), "Downloaded crates"),
                (cargo.join("git").join("checkouts"), "Git checkouts"),
            ];
            if !parts.iter().any(|(p, _)| p.is_dir()) {
                return Found::none();
            }
            let mut f = Found::at(cargo);
            let busy = ctx.running("cargo.exe") || ctx.running("rustc.exe") || ctx.running("cargo") || ctx.running("rustc");
            for (dir, name) in parts {
                if let Some(c) = cache_item(dir, name, ctx) {
                    f.exact &= c.complete;
                    f.cands.push(if busy && c.goes { c.verdict(false, "Cargo is running") } else { c });
                }
            }
            if busy {
                f.note = Some("Cargo is building something. Clean this when it's done.".into());
            }
            f
        }
        "go-build-cache" => whole_dir(la.join("go-build"), "Go build cache", ctx),
        "nuget-cache" => whole_dir(la.join("NuGet").join("v3-cache"), "NuGet HTTP cache", ctx),
        "gradle-cache" => whole_dir(ctx.paths.home.join(".gradle").join("caches"), "Gradle caches", ctx),
        "electron-cache" => whole_dir(la.join("electron").join("Cache"), "Electron downloads", ctx),
        "playwright-old" => {
            let root = la.join("ms-playwright");
            if !root.is_dir() {
                return Found::none();
            }
            let mut f = Found::at(root.clone());
            let mut builds: Vec<(String, u64, Cand)> = Vec::new();
            let list = entries(&root).into_iter().map(|p| {
                let n = name_of(&p);
                (p, n)
            });
            for c in cands(list.collect(), ctx.now) {
                match c.name.rsplit_once('-').and_then(|(family, rev)| Some((family.to_string(), rev.parse::<u64>().ok()?))) {
                    Some((family, rev)) => {
                        let label = format!("{} build {rev}", playwright_family(&family));
                        builds.push((family, rev, Cand { name: label, ..c }));
                    }
                    None => f.cands.push(c.verdict(false, "Playwright's own records")),
                }
            }
            let newest: Vec<(String, u64)> = builds.iter().map(|b| (b.0.clone(), b.1)).collect();
            for (family, rev, c) in builds {
                let latest = newest.iter().filter(|b| b.0 == family).map(|b| b.1).max().unwrap_or(rev);
                f.cands.push(if ctx.in_use(&c.path) {
                    c.verdict(false, IN_USE)
                } else if rev >= latest {
                    c.kept_but_tickable("Newest version, kept. Tests download it again if it's gone")
                } else {
                    c.verdict(true, format!("Replaced by build {latest}"))
                });
            }
            f
        }
        "huggingface-cache" => {
            let root = ctx.paths.home.join(".cache").join("huggingface").join("hub");
            if !root.is_dir() {
                return Found::none();
            }
            let mut f = Found::at(root.clone());
            let mut list = Vec::new();
            let mut kinds = Vec::new();
            for p in entries(&root) {
                let Some((kind, repo)) = hf_repo(&name_of(&p)) else { continue };
                list.push((p, repo));
                kinds.push(kind);
            }
            for (mut c, kind) in cands(list, ctx.now).into_iter().zip(kinds) {
                if kind != "model" {
                    c.sub = c.sub.map(|s| format!("{s} ({kind})"));
                }
                // Filesystem dates do not establish when a model was last used. The user picks.
                let c = if ctx.in_use(&c.path) {
                    c.verdict(false, IN_USE)
                } else {
                    let r = if c.age_known {
                        format!("Last changed {} ago; tick it if you're done with it", ago(c.age))
                    } else { "Age unknown; tick it only if you're done with it".into() };
                    c.kept_but_tickable(r)
                };
                f.cands.push(c);
            }
            f
        }
        "old-build-folders" => builds::find(ctx),
        "docker" => {
            if !ctx.docker_running {
                let installed = crate::util::on_path("docker").is_some();
                if !installed {
                    return Found::none();
                }
                let mut f = Found::none();
                f.available = true;
                f.exact = false;
                f.tool = Some((0, 0));
                f.note = Some("Docker isn't running. Start Docker Desktop and mom can see what it would prune.".into());
                return f;
            }
            let mut f = Found::none();
            f.available = true;
            let Some((images, stopped)) = crate::containers::docker_inventory() else {
                f.exact = false;
                f.tool = Some((0, 0));
                f.note = Some("Docker didn't answer. Give it a moment and check again.".into());
                return f;
            };
            let thing = |path: String, name: String, bytes: u64, kind: &'static str| {
                let mut c = Cand::new(PathBuf::from(path), name, &Tally::of(bytes, 1), ctx.now);
                c.age = 0;
                c.kind = Some(kind);
                c
            };
            for i in images {
                let mut c = thing(format!("docker:image:{}", i.id), i.name.clone(), i.size, "images");
                c.sub = Some(format!("Image, made {}", i.created));
                f.cands.push(if i.used_running {
                    c.verdict(false, format!("{} runs on it", i.used_by.join(", ")))
                } else if !i.used_by.is_empty() {
                    c.verdict(false, format!("Used by {} (stopped)", i.used_by.join(", ")))
                } else if i.name == "<none>" {
                    c.verdict(true, "An untagged leftover of an image that was rebuilt or pulled again")
                } else {
                    c.kept_but_tickable("No container uses it. Pulled ones download again; ones built here need building again")
                });
            }
            for s in stopped {
                let mut c = thing(format!("docker:container:{}", s.id), s.name.clone(), 0, "stopped containers");
                c.sub = Some(match &s.project {
                    Some(p) => format!("{}, compose project {p}", s.status),
                    None => format!("{}, from {}", s.status, s.image),
                });
                f.cands.push(c.kept_but_tickable(if s.project.is_some() {
                    "Stopped. `docker compose up` makes it again; its volumes stay"
                } else {
                    "Stopped. Removing it loses anything saved inside it (not in a volume)"
                }));
            }
            let df = crate::containers::docker_df().unwrap_or_default();
            for d in df {
                if d.kind.eq_ignore_ascii_case("Build Cache") && d.reclaimable > 0 {
                    let mut c = thing("docker:build-cache".into(), "Build cache".into(), d.reclaimable, "build cache");
                    c.sub = Some(format!("{} {}", d.count, if d.count == 1 { "entry" } else { "entries" }));
                    f.cands.push(c.verdict(true, "Made again by the next build that needs it"));
                } else if d.kind.eq_ignore_ascii_case("Local Volumes") && d.count > 0 {
                    let mut c = thing("docker:volumes".into(), "Volumes".into(), d.size, "volumes");
                    c.sub = Some(plural(d.count, "volume"));
                    f.cands.push(c.verdict(false, "Where databases keep their data. Mom never touches them"));
                }
            }
            f
        }
        "browser-cache" => browsers::find(ctx),
        "crash-dumps" => {
            let dumps = la.join("CrashDumps");
            let wer = la.join("Microsoft").join("Windows").join("WER");
            let mut found = Vec::new();
            for p in entries(&dumps) {
                let n = name_of(&p);
                found.push((p, crashed_app_from_dump(&n), "Crash dump"));
            }
            for sub in ["ReportArchive", "ReportQueue"] {
                for p in entries(&wer.join(sub)) {
                    let n = name_of(&p);
                    found.push((p, crashed_app_from_report(&n), "Error report"));
                }
            }
            if found.is_empty() && !dumps.is_dir() && !wer.is_dir() {
                return Found::none();
            }
            let mut f = Found::at(dumps);
            let kinds: Vec<&str> = found.iter().map(|x| x.2).collect();
            let list = found.into_iter().map(|(p, app, _)| (p, app)).collect();
            for (mut c, kind) in cands(list, ctx.now).into_iter().zip(kinds) {
                c = c.age_sensitive();
                c.sub = Some(if c.files > 1 { format!("{kind}, {}", plural(c.files, "file")) } else { kind.to_string() });
                let c = if c.age < CRASH_MIN_AGE {
                    let r = format!("Last changed {} ago (kept for a day)", ago(c.age));
                    c.kept_but_tickable(r)
                } else {
                    let r = format!("Last changed {} ago", ago(c.age));
                    c.verdict(true, r)
                };
                f.cands.push(c);
            }
            f
        }
        "recycle-bin" => bin::find(ctx),
        "old-installers" => {
            let dir = &ctx.paths.downloads;
            if !dir.is_dir() {
                return Found::none();
            }
            let mut f = Found::at(dir.clone());
            let min = OLD_INSTALLER_DAYS * 86_400;
            let list = entries(dir)
                .into_iter()
                .map(|p| {
                    let n = name_of(&p);
                    (p, n)
                })
                .filter(|(p, n)| is_installer(n) && p.is_file());
            for mut c in cands(list.collect(), ctx.now) {
                c = c.age_sensitive();
                c.sub = Some(installer_kind(&c.name).to_string());
                let c = if ctx.in_use(&c.path) {
                    c.verdict(false, "Running right now")
                } else if c.age < min {
                    let r = format!("Last changed {} ago (kept for {OLD_INSTALLER_DAYS} days)", ago(c.age));
                    c.kept_but_tickable(r)
                } else {
                    let r = format!("Last changed {} ago", ago(c.age));
                    c.verdict(true, r)
                };
                f.cands.push(c);
            }
            f
        }
        _ => Found::none(),
    }
}

/// "remotion-v4.0.529-assets27url9" -> "Remotion 4.0.529 render assets",
/// "garden-bundle-mine-1790000000-12" -> "garden render bundle".
fn render_name(folder: &str) -> String {
    let lower = folder.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("remotion-v") {
        if let Some((version, _)) = rest.split_once("-assets") {
            return format!("Remotion {version} render assets");
        }
    }
    if lower.starts_with("remotion-webpack-bundle") {
        return "Remotion webpack bundle".into();
    }
    if lower.starts_with("remotion-") {
        return "Remotion render files".into();
    }
    let project = folder.split("-bundle-").next().unwrap_or(folder);
    format!("{project} render bundle")
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// The packages an npx folder holds ("@stripe/cli").
fn npx_deps(dir: &Path) -> Vec<String> {
    read_json(&dir.join("package.json"))
        .and_then(|v| v.get("dependencies").and_then(|d| d.as_object()).map(|o| o.keys().cloned().collect()))
        .unwrap_or_default()
}

/// Where someone runs npx packages: shell histories, and the MCP server set-ups of agents and
/// editors. Each with how to say it, lowercased.
fn npx_signs(paths: &Paths) -> Vec<(&'static str, String)> {
    let read = |p: PathBuf| std::fs::read_to_string(p).map(|s| s.to_lowercase()).unwrap_or_default();
    let mut out = Vec::new();
    let history = paths.appdata.join("Microsoft").join("Windows").join("PowerShell").join("PSReadLine");
    let mut shells: String = entries(&history).into_iter().filter(|p| name_of(p).ends_with("_history.txt")).map(read).collect();
    for f in [".bash_history", ".zsh_history"] {
        shells.push_str(&read(paths.home.join(f)));
    }
    out.push(("your shell history runs it", shells));
    let mcp = [
        paths.home.join(".claude.json"),
        paths.home.join(".claude").join("settings.json"),
        paths.home.join(".cursor").join("mcp.json"),
        paths.home.join(".codex").join("config.toml"),
        paths.home.join(".gemini").join("settings.json"),
        paths.appdata.join("Claude").join("claude_desktop_config.json"),
        paths.appdata.join("Code").join("User").join("mcp.json"),
        paths.appdata.join("Code").join("User").join("settings.json"),
        paths.appdata.join("Cursor").join("User").join("settings.json"),
    ];
    let mut servers: String = mcp.into_iter().map(read).collect();
    // Plugins bring their own MCP servers (~/.claude/plugins/.../.mcp.json).
    let plugins = paths.home.join(".claude").join("plugins");
    for e in walkdir::WalkDir::new(&plugins).max_depth(6).into_iter().filter_map(Result::ok) {
        if e.file_name() == ".mcp.json" || e.file_name() == "plugin.json" {
            servers.push_str(&read(e.path().to_path_buf()));
        }
    }
    out.push(("an MCP server is set up to run it", servers));
    out
}

/// What an npx folder ran, from its package.json, with the version it got ("wrangler 4.42.0").
fn npx_name(dir: &Path) -> String {
    let deps: Vec<String> = read_json(&dir.join("package.json"))
        .and_then(|v| v.get("dependencies").and_then(|d| d.as_object()).map(|o| o.keys().cloned().collect()))
        .unwrap_or_default();
    if deps.is_empty() {
        return format!("npx download {}", name_of(dir));
    }
    deps.iter()
        .map(|d| {
            let installed = read_json(&dir.join("node_modules").join(d).join("package.json"));
            match installed.as_ref().and_then(|v| v.get("version")).and_then(|v| v.as_str()) {
                Some(version) => format!("{d} {version}"),
                None => d.clone(),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn playwright_family(family: &str) -> String {
    match family {
        "chromium" => "Chromium".into(),
        "chromium_headless_shell" => "Chromium headless shell".into(),
        "chromium_tip_of_tree" => "Chromium tip-of-tree".into(),
        "firefox" => "Firefox".into(),
        "webkit" => "WebKit".into(),
        "ffmpeg" => "ffmpeg".into(),
        other => other.replace('_', " "),
    }
}

/// "models--openai--whisper-large-v3" -> ("model", "openai/whisper-large-v3").
fn hf_repo(folder: &str) -> Option<(&'static str, String)> {
    let (kind, rest) = folder.split_once("--")?;
    let kind = match kind {
        "models" => "model",
        "datasets" => "dataset",
        "spaces" => "space",
        _ => return None,
    };
    Some((kind, rest.replace("--", "/")))
}

/// "node.exe.12345.dmp" -> "node.exe".
fn crashed_app_from_dump(file: &str) -> String {
    let stem = file.strip_suffix(".dmp").unwrap_or(file);
    match stem.rsplit_once('.') {
        Some((app, pid)) if pid.chars().all(|c| c.is_ascii_digit()) => app.to_string(),
        _ => stem.to_string(),
    }
}

/// "AppCrash_foo.exe_8d3c..._cab_1234" -> "foo.exe".
fn crashed_app_from_report(folder: &str) -> String {
    let rest = folder.split_once('_').map(|(_, r)| r).unwrap_or(folder);
    let lower = rest.to_ascii_lowercase();
    if let Some(i) = lower.find(".exe") {
        return rest[..i + 4].to_string();
    }
    rest.split('_').next().unwrap_or(rest).to_string()
}

const INSTALLER_EXTS: &[&str] = &["exe", "msi", "msix", "msixbundle", "appx", "appxbundle"];

fn is_installer(name: &str) -> bool {
    let ext = Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    INSTALLER_EXTS.contains(&ext.as_str())
}

fn installer_kind(name: &str) -> &'static str {
    let ext = Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "msi" => "Windows Installer package",
        "exe" => "Setup program",
        _ => "App package",
    }
}

// ------------------------------------------------------------------ measuring

pub fn measure(id: &str, ctx: &TrashCtx) -> Option<Spot> {
    let def = def(id)?;
    let f = find(id, ctx);
    Some(spot_from(def, f, ctx))
}

pub fn measure_all(ctx: &TrashCtx) -> Vec<Spot> {
    DEFS.iter().filter_map(|d| measure(d.id, ctx)).collect()
}

/// How many items a spot lists of each kind (the rest are summed up): enough for the widget to
/// group them by app or game.
const MAX_GOES: usize = 300;
const MAX_STAYS: usize = 150;
/// "Show more" lists four times as many each time it's pressed, up to this many times the usual.
pub const MORE: usize = 4;
pub const MAX_SCALE: usize = 64;

fn by_size(a: &Cand, b: &Cand) -> std::cmp::Ordering {
    b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name))
}

fn spot_from(def: &Def, f: Found, ctx: &TrashCtx) -> Spot {
    let (mut goes, mut stays): (Vec<Cand>, Vec<Cand>) = f.cands.into_iter().partition(|c| c.goes);
    goes.sort_by(by_size);
    // What you said to keep comes first, so it's always in the list and can be let go again.
    stays.sort_by(|a, b| b.kept.cmp(&a.kept).then_with(|| by_size(a, b)));
    let go_bytes: u64 = goes.iter().map(|c| c.bytes).sum();
    let stay_bytes: u64 = stays.iter().map(|c| c.bytes).sum();
    let (total, reclaimable) = match f.tool {
        Some((total, reclaim)) => (total.max(reclaim), reclaim),
        None => (go_bytes + stay_bytes, go_bytes),
    };
    let effect = effect_of(def.id, &goes, &stays, reclaimable, ctx);
    let exact = f.exact && goes.iter().chain(&stays).all(|c| c.complete);
    let (n_go, n_stay) = (goes.len() as u64, stays.len() as u64);
    let new_bytes = stays.iter().filter(|c| c.optional).map(|c| c.bytes).sum();
    let new_items = stays.iter().filter(|c| c.optional).count() as u64;
    let scale = ctx.list_scale.clamp(1, MAX_SCALE);
    let items = goes.into_iter().take(MAX_GOES * scale).chain(stays.into_iter().take(MAX_STAYS * scale)).map(Cand::into_item).collect();
    Spot {
        id: def.id.into(),
        label: def.label.into(),
        detail: def.detail.into(),
        group: def.group.into(),
        what: def.what.split_whitespace().collect::<Vec<_>>().join(" "),
        effect,
        action: def.action.into(),
        safety: def.safety.into(),
        path: f.root.map(|r| r.to_string_lossy().to_string()),
        owner: def.owner,
        total_bytes: total,
        reclaimable_bytes: reclaimable,
        reclaimable_items: n_go,
        kept_bytes: total.saturating_sub(reclaimable),
        kept_items: n_stay,
        new_bytes,
        new_items,
        admin: needs_admin(def.id),
        exact,
        available: f.available,
        measured_at: ctx.now,
        note: f.note,
        items,
    }
}

/// "212 scratch folders (1.2 GB)". `word` is the plural, `one` the singular.
fn count_size(n: usize, one: &str, word: &str, bytes: u64) -> String {
    let what = if n == 1 { format!("1 {one}") } else { format!("{} {word}", crate::util::human_count(n as u64)) };
    if bytes == 0 && n > 0 {
        return format!("{what} ({})", if n == 1 { "empty" } else { "all empty" });
    }
    format!("{what} ({})", human_bytes(bytes))
}

/// Exactly what cleaning a spot does, from what was measured.
fn effect_of(id: &str, goes: &[Cand], stays: &[Cand], reclaimable: u64, ctx: &TrashCtx) -> String {
    let n = goes.len();
    let bytes = reclaimable;
    let kept = human_age_long(ctx.cfg.agent_scratch_days * 86_400);
    let render_age = human_age_long(RENDER_MIN_AGE);
    let forever = "permanently (they don't go to the Recycle Bin)";
    let nothing = |why: &str| format!("Nothing to clean right now: {why}");
    match id {
        "claude-temp" if n == 0 => nothing(&format!(
            "no scratch folders with complete checks qualify for the {kept} cutoff. Recent, running, protected or unverified folders stay."
        )),
        "claude-temp" => format!(
            "Deletes {} from Claude Code sessions that have ended, {forever}. Folders of sessions that are still \
             running, or changed in the last {kept}, are left alone. Your projects and Claude's chat history aren't touched.",
            count_size(n, "scratch folder", "scratch folders", bytes)
        ),
        "codex-temp" if n == 0 => nothing(&format!("no Codex leftovers with complete checks qualify for the {kept} cutoff. Recent, in-use, protected or unverified items stay.")),
        "codex-temp" => format!(
            "Deletes {} that Codex left behind with no recorded changes in {kept}, {forever}: {}. Codex's threads, \
             settings and memories aren't touched; anything newer stays.",
            count_size(n, "temp file", "temp files and folders", bytes),
            harness::kinds(goes)
        ),
        "render-temp" if n == 0 => nothing(&format!("no render folders with complete checks qualify for the {render_age} cutoff.")),
        "render-temp" => format!(
            "Deletes {} from your Temp folder, {forever}. They're copies made while rendering; your projects and \
             finished videos aren't touched. Anything with recorded changes in the last {render_age} stays.",
            count_size(n, "render folder", "render folders", bytes)
        ),
        "old-conversations" => {
            let days = human_age_long(ctx.cfg.conversations_after_days * 86_400);
            let place = ctx.cfg.archive_dir.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| "a drive you choose".into());
            if n == 0 {
                nothing(&format!("no conversation is known to have stayed unchanged for {days}, or they have been moved already."))
            } else {
                let tools = harness::tools(goes);
                format!(
                    "Moves {} of {tools} conversations with no recorded changes in {days} to {place}, and leaves a link in each \
                     one's place, so they still open from where they were. Nothing is deleted, and Bring back puts any of \
                     them back here. While that drive is unplugged, those conversations can't be opened.",
                    count_size(n, "folder", "folders", bytes)
                )
            }
        }
        "forgotten-apps" => {
            let place = ctx.cfg.archive_dir.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| "a drive you choose".into());
            if n == 0 {
                nothing("no big folders of uninstalled apps, older versions or long-forgotten things.")
            } else {
                format!(
                    "Archive moves {} to {place}, with a link left in each one's place: data of {}. Bring back puts \
                     them back. You can instead choose Recycle Bin or Delete for good for the folders you select; \
                     deleting for good cannot be undone. Mom guesses which app a folder belongs to, so look through the list first.",
                    count_size(n, "folder", "folders", bytes),
                    harness::tools(goes)
                )
            }
        }
        "cursor-worktrees" if n == 0 => nothing(&format!("no worktrees with complete checks qualify for the {kept} cutoff. Recent, in-use, protected or unverified worktrees stay.")),
        "cursor-worktrees" => format!(
            "Moves {} that Cursor's agents made, with no recorded changes in {kept}, to the Recycle Bin. Changes in them \
             that were never committed go too, so look through the list first; restore any from the Recycle Bin. Your \
             projects aren't touched (Git keeps a note of each worktree until you run `git worktree prune`).",
            count_size(n, "worktree", "worktrees", bytes)
        ),
        "shader-cache" => {
            if n == 0 {
                "Mom has not selected any shader caches by default. Steam caches start unticked; you can select them after reviewing their app details.".into()
            } else {
                format!(
                    "Deletes {} of compiled shaders, {forever}. Games and apps rebuild shaders they need; the first run may stutter. \
                     Cache age is when files last changed, not when a game was last played.",
                    count_size(n, "item", "items", bytes)
                )
            }
        }
        "claude-leftovers" | "agent-leftovers" | "app-leftovers" | "game-junk" | "dev-leftovers" | "windows-junk"
        | "admin-leftovers" => {
            if n == 0 {
                nothing(&format!("no eligible leftovers qualify for the {kept} cutoff. Recent, in-use, protected or unverified items stay."))
            } else {
                format!(
                    "Deletes {} that {} left behind with no recorded changes in {kept}, {forever}: {}. {} \
                     Anything newer, or in use, stays.",
                    count_size(n, "item", "items", bytes),
                    harness::tools(goes),
                    harness::kinds(goes),
                    harness::untouched(id)
                )
            }
        }
        "old-temp" => {
            let days = human_age_long(ctx.cfg.temp_min_age_days * 86_400);
            if n == 0 {
                nothing(&format!("no Temp items with complete checks qualify for the {days} cutoff. Recent, in-use, protected or unverified items stay."))
            } else {
                format!(
                    "Deletes {} from your Temp folder with no recorded changes in {days}, {forever}. Anything newer, or in \
                     use by a running program, stays. Programs make new temp files when they need them.",
                    count_size(n, "item", "items", bytes)
                )
            }
        }
        "npx-cache" if n == 0 => nothing(&format!(
            "npx doesn't note when a package last ran, so mom keeps everything fetched in the last {kept}, and anything \
             your shell history or an MCP server set-up names."
        )),
        "npx-cache" => format!(
            "Deletes {} that npx fetched over {kept} ago and that nothing mom can see runs (not your shell history, not an \
             MCP server set-up), {forever}. If one is needed again, npx fetches it again in a few seconds.",
            count_size(n, "package folder", "package folders", bytes)
        ),
        "npm-cache" | "yarn-cache" | "bun-cache" => {
            let tool = match id {
                "npm-cache" => "npm",
                "yarn-cache" => "Yarn",
                _ => "bun",
            };
            whole_cache_effect(
                n,
                bytes,
                &format!("{tool}'s download cache"),
                &format!("Your projects keep their installed packages; {tool} downloads packages again the next time you install them."),
            )
        }
        "pip-cache" => whole_cache_effect(n, bytes, "pip's download cache", "Installed packages stay installed; pip downloads again when it needs to."),
        "go-build-cache" => whole_cache_effect(n, bytes, "Go's build cache", "The next build of each Go project compiles from scratch, so it takes longer once."),
        "nuget-cache" => whole_cache_effect(n, bytes, "NuGet's HTTP cache", "Installed packages stay; NuGet fetches package lists again as needed."),
        "gradle-cache" => whole_cache_effect(n, bytes, "Gradle's caches", "The next build downloads its dependencies again, so it takes longer once."),
        "electron-cache" => whole_cache_effect(n, bytes, "the Electron download cache", "Electron downloads again the next time a project needs it."),
        "cargo-cache" if n == 0 => nothing("Cargo is busy, or there's nothing downloaded."),
        "cargo-cache" => format!(
            "Deletes Cargo's downloaded crate archives and git checkouts ({}), {forever}. Your projects, their builds and \
             the unpacked sources builds use aren't touched; Cargo downloads crates again when a build needs them.",
            human_bytes(bytes)
        ),
        "uv-cache" => format!(
            "Runs `uv cache prune`, which deletes cache entries no environment uses any more (up to {}). uv decides \
             exactly what goes; your virtual environments keep working.",
            human_bytes(bytes)
        ),
        "pnpm-store" => format!(
            "Runs `pnpm store prune`, which removes packages no project links to any more (up to {}). pnpm decides \
             exactly what goes; your projects keep working, and pnpm downloads anything it needs again.",
            human_bytes(bytes)
        ),
        "docker" if !ctx.docker_running => "Docker isn't running, so mom can't see what it keeps. Start Docker Desktop and \
             check again: she'll list every image and stopped container, and what uses what."
            .to_string(),
        "docker" if n == 0 => nothing("no build cache or untagged images. Tick images or stopped containers below if you're done with them."),
        "docker" => format!(
            "Has Docker remove {}: the build cache and untagged leftovers, plus any images and stopped containers you \
             tick. Images a container uses can't be ticked, and volumes (where databases keep their data) are never \
             touched. Images share layers, so a little less may come back.",
            count_size(n, "thing", "things", bytes)
        ),
        "playwright-old" if n == 0 => nothing("only the newest build of each browser is installed."),
        "playwright-old" => format!(
            "Deletes {} that Playwright downloaded, {forever}. The newest build of each browser stays, so tests and \
             browser tools keep working; `npx playwright install` brings an old one back if a project needs it.",
            count_size(n, "older browser build", "older browser builds", bytes)
        ),
        "huggingface-cache" if stays.is_empty() => nothing("no downloaded models."),
        "huggingface-cache" if n == 0 => "Nothing is picked for you: nothing records when a model was last used, so mom can't \
             tell the ones you use from the ones you don't. Tick the models you're done with; they're deleted for good, and \
             download again if your code asks for them."
            .to_string(),
        "huggingface-cache" => format!(
            "Deletes {} {forever}. Models can be very large and slow to download again, so untick any you still use. \
             Your code, and models you saved somewhere else, aren't touched.",
            count_size(n, "downloaded model", "downloaded models and datasets", bytes)
        ),
        "old-build-folders" if n == 0 => {
            if stays.is_empty() {
                nothing("no build folders found in your projects.")
            } else {
                nothing("no build folders have sufficient evidence of old project activity; recent, active or incompletely checked projects stay.")
            }
        }
        "old-build-folders" => {
            let projects: HashSet<&Path> = goes.iter().filter_map(|c| c.path.parent()).collect();
            format!(
                "Deletes {} from {} with no recorded changes for {IDLE_PROJECT_DAYS} days, {forever}. Your code isn't touched: \
                 `npm install` (or pnpm, yarn, bun) and `cargo build` bring them back. Recent changes, running programs \
                 or incomplete checks keep a project's folders in place.",
                count_size(n, "build folder", "build folders", bytes),
                plural(projects.len() as u64, "project")
            )
        }
        "browser-cache" => {
            let open: Vec<&str> = browsers::open_names(stays);
            let mut s = if n == 0 {
                nothing("no browser caches to clear.")
            } else {
                format!(
                    "Deletes cached pages, images and scripts for {}, {forever}. Pages may load more slowly at first. \
                     Cookies, sign-ins, site storage, history, bookmarks, passwords and session recovery data are kept.",
                    count_size(n, "browser profile or app", "browser profiles and apps", bytes)
                )
            };
            match open.as_slice() {
                [] => {}
                [one] => s.push_str(&format!(" {one} is open, so its cache stays until you close it.")),
                many => s.push_str(&format!(" {} are open, so their caches stay until you close them.", and_list(many))),
            }
            s
        }
        "crash-dumps" if n == 0 => nothing("no crash reports older than a day."),
        "crash-dumps" => format!(
            "Deletes {} from apps that crashed, {forever}. They're only useful for sending to a developer; reports from \
             the last day stay in case you want to.",
            count_size(n, "crash report", "crash dumps and error reports", bytes)
        ),
        "recycle-bin" if n == 0 => nothing("the Recycle Bin is empty."),
        "recycle-bin" => format!(
            "Empties the Recycle Bin: {} are deleted permanently. This can't be undone, so look through the list first \
             and untick anything you might want back.",
            count_size(n, "item", "items", bytes)
        ),
        "old-installers" if n == 0 => nothing(&format!("no installers in Downloads older than {OLD_INSTALLER_DAYS} days.")),
        "old-installers" => format!(
            "Moves {} from Downloads to the Recycle Bin. They're setup files over {OLD_INSTALLER_DAYS} days old for programs \
             you've probably installed already. Changed your mind? Restore them from the Recycle Bin. Newer downloads, and \
             everything that isn't an installer, stay.",
            count_size(n, "installer", "installers", bytes)
        ),
        _ => format!("Deletes {} {forever}.", count_size(n, "item", "items", bytes)),
    }
}

fn whole_cache_effect(n: usize, bytes: u64, what: &str, comes_back: &str) -> String {
    if n == 0 {
        return format!("Nothing to clean right now: {what} is empty or in use.");
    }
    format!("Empties {what} ({}), permanently (it doesn't go to the Recycle Bin). {comes_back}", human_bytes(bytes))
}

/// "Edge", "Edge and Chrome", "Edge, Chrome and Brave".
fn and_list(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

// ------------------------------------------------------------------ cleaning

/// What the user ticked in a spot's two lists before cleaning it. Each list starts all ticked
/// or all unticked (so "Tick all" also covers items too small to be listed), and `ticked` and
/// `unticked` name the listed items changed from that. Paths as given in `Spot.items`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Pick {
    /// Whether what would go starts ticked. "Untick all" makes it false.
    pub goes: bool,
    /// Whether what's kept only for being new (`Item.optional`) starts ticked ("Tick all").
    pub new: bool,
    pub ticked: Vec<String>,
    pub unticked: Vec<String>,
    /// Send it to the Recycle Bin instead of deleting it for good (delete spots, or `discard`).
    pub recycle: bool,
    /// Forgotten app data only: discard these exact folders instead of archiving them.
    /// Requires both list defaults off; `ticked` is the complete, explicit selection.
    pub discard: bool,
    /// Things never to take, whatever else is ticked: what you said to keep, handed along to the
    /// administrator run, which may be reading another account's settings.
    pub keep: Vec<String>,
}

impl Default for Pick {
    fn default() -> Pick {
        Pick { goes: true, new: false, ticked: vec![], unticked: vec![], recycle: false, discard: false, keep: vec![] }
    }
}

impl Pick {
    fn is_default(&self) -> bool {
        self.goes && !self.new && !self.recycle && !self.discard && self.ticked.is_empty() && self.unticked.is_empty() && self.keep.is_empty()
    }

    fn discard_error(&self, id: &str) -> Option<&'static str> {
        if !self.discard {
            None
        } else if id != "forgotten-apps" {
            Some("Direct removal is only available for Forgotten app data.")
        } else if self.goes || self.new {
            Some("Choose the exact app data folders to remove; direct removal cannot include an entire unseen list.")
        } else {
            None
        }
    }
}

/// What a spot holds, for a clean-up: as `find`, and minus what the pick says to keep.
fn find_for(id: &str, ctx: &TrashCtx, pick: Option<&Pick>) -> Found {
    let f = find(id, ctx);
    match pick {
        Some(p) => keep_yours(f, &p.keep[..p.keep.len().min(MAX_KEPT)]),
        None => f,
    }
}

/// Items deleted together: many at once is much faster than one by one, and in batches the
/// widget can show how it's going and stop between them.
const BATCH_ITEMS: usize = 48;
const BATCH_FILES: u64 = 4_000;

/// How many items go in one batch when `total` are going: few enough that the count the widget
/// shows moves at least a dozen times ("0 of 53", and then all of them at once, tells nobody
/// anything), many enough that thousands of small things still go fast.
fn batch_items(total: usize) -> usize {
    (total / 16).clamp(1, BATCH_ITEMS)
}

/// Cleans a spot: what qualifies right now, as ticked in `pick` (by default, everything that
/// goes). In-use things never go. Tool spots ignore `pick`. Reports how it's going to
/// `ctx.live`, and stops between items (or batches of small ones) when asked.
pub fn clean(id: &str, ctx: &TrashCtx, pick: Option<&Pick>) -> CleanReport {
    let mut r = CleanReport { id: id.to_string(), ..Default::default() };
    let Some(def) = def(id) else {
        r.message = format!("Mom doesn't know a spot called {id:?}.");
        return r;
    };
    if let Some(reason) = pick.and_then(|p| p.discard_error(id)) {
        r.message = reason.into();
        return r;
    }
    let pick = pick.filter(|p| !p.is_default());
    let discard = pick.is_some_and(|p| p.discard);
    // As administrator, mom only cleans what needs one. Your own files are cleaned as you.
    let elevated = ctx.elevated;
    // Asked for the Recycle Bin instead: only where mom deletes your own files as you.
    let recycle = pick.is_some_and(|p| p.recycle) && (def.action == "delete" || discard) && !elevated && !needs_admin(id);
    if elevated && !needs_admin(id) {
        r.message = "Run this as yourself, not as administrator: it cleans your own files.".into();
        return r;
    }
    match def.action {
        "tool" => return run_tool(id, ctx, r),
        _ if id == "docker" => return docker_clean(ctx, pick, r),
        _ => {}
    }
    let f = find_for(id, ctx, pick);
    let chosen = choose(&f, pick);
    let update_reason = std::cell::Cell::new(
        [windows_update::State::Pending, windows_update::State::Unknown].into_iter()
            .filter_map(|state| state.reason())
            .find(|reason| f.note.as_deref() == Some(*reason)
                || f.cands.iter().any(|c| c.reason == *reason && windows_update_candidate(c, ctx)))
    );
    let update_checked = std::cell::Cell::new(false);
    // Refresh the local update catalog before the first affected operation, then check
    // native servicing/restart/process state before later items without launching WUA for
    // every small cache entry. Never reuse a Clear result across cleanup requests. Once
    // pending or unknown, keep the rest of this request even if the state changes again.
    let update_block_reason = |c: &Cand| {
        if id == "recycle-bin" || !windows_update_candidate(c, ctx) { return None; }
        let reason = update_reason.get().or_else(|| {
            if update_checked.replace(true) { windows_update_fast_state(ctx) }
            else { windows_update_state(ctx) }.reason()
        });
        if reason.is_some() { update_reason.set(reason); }
        reason
    };
    let update_allowed = |c: &Cand| update_block_reason(c).is_none();
    if let Some(p) = pick {
        // Ticked, but can't go now (in use since the user looked, or already gone).
        let taken: HashSet<String> = chosen.iter().map(|c| norm_path(&c.path.to_string_lossy())).collect();
        r.skipped += p.ticked.iter().filter(|t| !taken.contains(&norm_path(t))).count() as u64;
    }
    let live = ctx.live;
    if let Some(l) = live {
        l.start(&chosen);
    }
    if def.action == "move" && !discard {
        return move_chosen(&chosen, ctx, r, &update_block_reason);
    }
    let stopped = || live.is_some_and(Live::stopped);
    // What an item's going (or not) freed, told to whoever is watching as soon as it's known: a
    // long clean-up counts up as it goes, not all at once when the last item is done. `told`
    // remembers what was said, by place in `chosen`, so the tally at the end doesn't say it twice.
    let told: std::cell::RefCell<std::collections::HashMap<usize, u64>> = Default::default();
    let settle = |i: usize, ok: bool, count_files: bool| {
        let c = chosen[i];
        let freed = if ok {
            c.bytes
        } else {
            // Something in it was in use (or the clean was stopped halfway); count what did go.
            let left: u64 = tally_many(&c.targets, ENTRY_CAP, u64::MAX).iter().map(|t| t.bytes).sum();
            c.bytes.saturating_sub(left)
        };
        if let Some(l) = live {
            if count_files {
                l.files_gone(c.files.max(1)); // nothing counted its files one by one
            }
            l.finished(c, ok, freed);
        }
        told.borrow_mut().insert(i, freed);
    };
    // One item at a time, for the ways that can't take many items at once. `counted`: the way
    // counts each file to `live` itself as it goes.
    let each = |counted: bool, remove: &dyn Fn(&Cand) -> bool| -> Vec<Option<bool>> {
        chosen
            .iter()
            .enumerate()
            .map(|(i, c)| {
                if stopped() {
                    return None;
                }
                if let Some(l) = live {
                    l.working_on(c);
                }
                let safe = (id == "recycle-bin" || c.targets.iter().all(|t| crate::browser_data::tree_is_safe(t)))
                    && (id != "browser-cache" || browsers::still_closed(&c.path, ctx))
                    && update_allowed(c);
                let ok = safe && remove(c);
                settle(i, ok, !counted);
                Some(ok)
            })
            .collect()
    };
    // What was too big for the Recycle Bin, and so stayed.
    let mut big = vec![false; chosen.len()];
    let per_batch = batch_items(chosen.len());
    let gone: Vec<Option<bool>> = if id == "recycle-bin" {
        // The data and its restore record are a pair. Keep the record if data could not go.
        each(false, &bin::remove_record)
    } else if def.action == "recycle" {
        each(false, &|c: &Cand| c.targets.iter().all(|t| !t.exists() || ::trash::delete(t).is_ok()))
    } else if recycle {
        recycle_chosen(&chosen, live, &mut big, &|c| (id != "browser-cache" || browsers::still_closed(&c.path, ctx)) && update_allowed(c), &|i, ok| match ok {
            Some(ok) => settle(i, ok, false), // it counts the files of each batch itself
            None => {
                // Too big for the Recycle Bin, so it stays: said as soon as it's known.
                if let Some(l) = live {
                    l.finished(chosen[i], false, 0);
                }
                told.borrow_mut().insert(i, 0);
            }
        })
    } else if elevated {
        // As administrator: only where nobody else could have swapped a folder for a link, and
        // one at a time, never following one.
        each(false, &|c: &Cand| c.targets.iter().all(|t| crate::elevate::admin_may_delete(t).is_ok() && remove_without_following(t)))
    } else if needs_admin(id) {
        // As you: what's in folders anyone can change is yours to clean anyway. The rest waits
        // for the administrator run.
        each(false, &|c: &Cand| c.targets.iter().all(|t| crate::elevate::admin_may_delete(t).is_err() && remove_without_following(t)))
    } else if id == "old-build-folders" || id == "browser-cache" || discard {
        // Projects can sit outside your own folders, where other people on this PC can write
        // (the top of a second drive), and a folder in your profile can be a link to such a
        // place. So each is held open while mom checks it's a real folder of yours and empties
        // it. Direct removal of forgotten app data uses the same protection: the check and the
        // delete are about the same folder, with no moment between them for anyone to swap it.
        // One folder at a time, but everything inside it at once, each
        // folder in it held the same way while its own contents go.
        each(true, &|c: &Cand| c.targets.iter().all(|t| remove_owned(t, live)))
    } else {
        // In batches: a big folder on its own, small things many at a time.
        let mut out: Vec<Option<bool>> = Vec::with_capacity(chosen.len());
        let mut start = 0;
        while start < chosen.len() {
            let mut end = start;
            let mut files = 0;
            while end < chosen.len() && end - start < per_batch && (end == start || files + chosen[end].files <= BATCH_FILES) {
                files += chosen[end].files;
                end += 1;
            }
            if stopped() {
                out.extend(std::iter::repeat_n(None, chosen.len() - start));
                break;
            }
            if let Some(l) = live {
                l.working_on(chosen[start]);
            }
            let targets: Vec<PathBuf> = chosen[start..end].iter().flat_map(|c| c.targets.iter().cloned()).collect();
            let safe: Vec<bool> = chosen[start..end].iter().flat_map(|c| {
                let allowed = update_allowed(c);
                c.targets.iter().map(move |t| allowed && (id == "recycle-bin" || crate::browser_data::tree_is_safe(t)))
            }).collect();
            let allowed: Vec<PathBuf> = targets.iter().zip(&safe).filter(|(_, safe)| **safe).map(|(p, _)| p.clone()).collect();
            let mut removed = remove_all(&allowed, live).into_iter();
            let mut done = safe.into_iter().map(|safe| safe && removed.next().unwrap_or(false));
            for (i, c) in chosen.iter().enumerate().take(end).skip(start) {
                let ok = c.targets.iter().map(|_| done.next().unwrap_or(false)).fold(true, |a, b| a & b);
                settle(i, ok, false); // remove_all counted the files as they went
                out.push(Some(ok));
            }
            start = end;
        }
        out
    };
    let mut left_alone = 0u64;
    let too_big = big.iter().filter(|b| **b).count() as u64;
    for (i, ((c, ok), big)) in chosen.iter().zip(gone).zip(big).enumerate() {
        if big {
            if !told.borrow().contains_key(&i) {
                if let Some(l) = live {
                    l.finished(c, false, 0);
                }
            }
            continue;
        }
        let Some(ok) = ok else {
            left_alone += 1;
            continue;
        };
        if ok {
            r.deleted += 1;
        } else {
            r.skipped += 1;
        }
        // Already told as it went (the usual case), or told now (what went to the Recycle Bin).
        if !told.borrow().contains_key(&i) {
            settle(i, ok, false);
        }
        r.freed += told.borrow().get(&i).copied().unwrap_or(0);
    }
    if def.action == "empty-bin" {
        bin::refresh_icon();
    }
    if id == "claude-temp" {
        // Leave no empty folders behind.
        if let Some(root) = &f.root {
            for slug in entries(root) {
                if slug.is_dir() && entries(&slug).is_empty() {
                    let _ = std::fs::remove_dir(&slug);
                }
            }
        }
    }
    r.message = if recycle { recycled_message(&r) } else if discard { discarded_message(&r) } else { cleaned_message(def, &r) };
    if let Some(reason) = update_reason.get() {
        if r.deleted == 0 && r.freed == 0 {
            r.message = format!("Nothing was removed. {reason}");
        } else {
            r.message.push_str(&format!(" {reason}"));
        }
    }
    if too_big > 0 {
        r.skipped += too_big;
        r.message.push_str(&format!(
            " {} {} too big for the Recycle Bin, so {} stayed: Windows would have deleted {} for good.",
            plural(too_big, "item"),
            if too_big == 1 { "was" } else { "were" },
            if too_big == 1 { "it" } else { "they" },
            if too_big == 1 { "it" } else { "them" },
        ));
    }
    if left_alone > 0 {
        r.message = format!("Stopped. {} {} left as they were.", r.message, plural(left_alone, "item"));
    }
    r
}

/// Sends what's chosen to the Recycle Bin, in batches like deleting. Only what fits in the bin
/// on its drive goes (Windows deletes anything bigger for good, without asking); the rest is
/// marked in `big` and stays. Links are recycled as links, never followed.
/// Sends what was chosen to the Recycle Bin, a batch at a time. `done(i, Some(gone))` is called
/// for each item as its batch finishes, and `done(i, None)` for one too big for the bin.
fn recycle_chosen(chosen: &[&Cand], live: Option<&Live>, big: &mut [bool], allowed: &dyn Fn(&Cand) -> bool, done: &dyn Fn(usize, Option<bool>)) -> Vec<Option<bool>> {
    let stopped = || live.is_some_and(Live::stopped);
    let per_batch = batch_items(chosen.len());
    let mut room = bin::Room::default();
    for (i, c) in chosen.iter().enumerate() {
        big[i] = !c.targets.iter().all(|t| room.fits(t, c.bytes));
    }
    let mut out: Vec<Option<bool>> = vec![None; chosen.len()];
    let mut start = 0;
    while start < chosen.len() {
        if big[start] {
            out[start] = Some(false);
            done(start, None);
            start += 1;
            continue;
        }
        if stopped() {
            break;
        }
        let mut end = start;
        let mut files = 0;
        while end < chosen.len() && !big[end] && end - start < per_batch && (end == start || files + chosen[end].files <= BATCH_FILES) {
            files += chosen[end].files;
            end += 1;
        }
        if let Some(l) = live {
            l.working_on(chosen[start]);
        }
        let targets: Vec<&PathBuf> = chosen[start..end].iter()
            .filter(|c| c.targets.iter().all(|t| crate::browser_data::tree_is_safe(t)) && allowed(c))
            .flat_map(|c| c.targets.iter()).collect();
        if !targets.is_empty() {
            // Whatever couldn't go (in use) shows up below as still there.
            let _ = ::trash::delete_all(&targets);
        }
        if let Some(l) = live {
            l.files_gone(files);
        }
        for (i, c) in chosen.iter().enumerate().take(end).skip(start) {
            let gone = c.targets.iter().all(|t| std::fs::symlink_metadata(t).is_err());
            out[i] = Some(gone);
            done(i, Some(gone));
        }
        start = end;
    }
    bin::refresh_icon();
    out
}

fn discarded_message(r: &CleanReport) -> String {
    let mut message = if r.deleted > 0 {
        format!("Deleted {} ({}) for good.", plural(r.deleted, "app data folder"), human_bytes(r.freed))
    } else if r.freed > 0 {
        format!("Deleted {} from app data, but some folders remain.", human_bytes(r.freed))
    } else {
        "No app data was deleted.".into()
    };
    if r.skipped > 0 {
        message.push_str(&format!(" {} stayed: in use, kept, linked, blocked by permissions, or changed since mom looked.", plural(r.skipped, "folder")));
    }
    message
}

fn recycled_message(r: &CleanReport) -> String {
    let mut msg = if r.deleted == 0 {
        "Nothing went to the Recycle Bin.".to_string()
    } else {
        format!("Moved {} ({}) to the Recycle Bin. Changed your mind? It's all in there.", plural(r.deleted, "item"), human_bytes(r.freed))
    };
    if r.skipped > 0 {
        msg.push_str(&format!(" {} stayed: in use, blocked by permissions, or changed since mom looked.", plural(r.skipped, "item")));
    }
    msg
}

/// Cleans Docker: exactly the containers, images and build cache that are ticked (by default,
/// just the build cache and untagged leftovers), each with Docker's own command.
fn docker_clean(ctx: &TrashCtx, pick: Option<&Pick>, mut r: CleanReport) -> CleanReport {
    if !ctx.docker_running {
        r.message = "Docker isn't running, so there's nothing mom can ask it to remove.".into();
        return r;
    }
    let f = find("docker", ctx);
    let chosen = choose(&f, pick);
    if let Some(l) = ctx.live {
        l.start(&chosen);
    }
    let (mut containers, mut images, mut cache) = (vec![], vec![], false);
    for c in &chosen {
        let p = c.path.to_string_lossy();
        if let Some(id) = p.strip_prefix("docker:container:") {
            containers.push(id.to_string());
        } else if let Some(id) = p.strip_prefix("docker:image:") {
            images.push(id.to_string());
        } else if p == "docker:build-cache" {
            cache = true;
        }
    }
    if chosen.is_empty() {
        r.message = "Nothing ticked, so Docker was left alone.".into();
        return r;
    }
    let mut why = None;
    for (what, result) in crate::containers::docker_remove(&containers, &images, cache) {
        let Some(c) = chosen.iter().find(|c| c.path.to_string_lossy().ends_with(&what) || (what == "build cache" && c.path.to_string_lossy() == "docker:build-cache")) else {
            continue;
        };
        let ok = result.is_ok();
        match result {
            Ok(()) => {
                r.deleted += 1;
                r.freed += c.bytes;
            }
            Err(e) => {
                r.skipped += 1;
                why.get_or_insert(e);
            }
        }
        if let Some(l) = ctx.live {
            l.finished(c, ok, if ok { c.bytes } else { 0 });
        }
    }
    r.message = if r.deleted == 0 {
        format!("Docker didn't remove anything: {}", why.clone().unwrap_or_default())
    } else {
        format!(
            "Docker removed {}, about {} (images share layers, so a little less may come back). Volumes are untouched.",
            plural(r.deleted, "thing"),
            human_bytes(r.freed)
        )
    };
    if r.deleted > 0 && r.skipped > 0 {
        r.message.push_str(&format!(" {} stayed: {}", plural(r.skipped, "thing"), why.unwrap_or_default()));
    }
    r
}

/// What a clean takes: whatever is ticked, out of what goes and what's kept only for being new.
/// Verdicts are fresh, so something that came into use since the user looked is left alone.
fn choose<'f>(f: &'f Found, pick: Option<&Pick>) -> Vec<&'f Cand> {
    let usual = Pick::default();
    let pick = pick.unwrap_or(&usual);
    let paths = |v: &[String]| v.iter().map(|p| norm_path(p)).collect::<HashSet<String>>();
    let (on, off) = (paths(&pick.ticked), paths(&pick.unticked));
    f.cands
        .iter()
        .filter(|c| {
            if c.browser_data { return false; }
            if c.age_required && (!c.age_known || !c.complete) { return false; }
            let starts_ticked = match (c.goes, c.optional) {
                (true, _) => pick.goes,
                (false, true) => pick.new,
                (false, false) => return false,
            };
            let p = norm_path(&c.path.to_string_lossy());
            if starts_ticked {
                !off.contains(&p)
            } else {
                on.contains(&p)
            }
        })
        .collect()
}

/// Moves each folder to the archive, leaving a link behind. One that can't move stays put.
fn move_chosen(chosen: &[&Cand], ctx: &TrashCtx, mut r: CleanReport, update_block_reason: &dyn Fn(&Cand) -> Option<&'static str>) -> CleanReport {
    let Some(root) = ctx.cfg.archive_dir.clone() else {
        r.message = "Choose where to keep them first.".into();
        return r;
    };
    let mut why = None;
    let mut left_alone = 0;
    for c in chosen {
        if ctx.live.is_some_and(Live::stopped) {
            left_alone += 1;
            continue;
        }
        if !c.targets.iter().all(|t| crate::browser_data::tree_is_safe(t)) {
            r.skipped += 1;
            why.get_or_insert_with(|| BROWSER_DATA.to_string());
            continue;
        }
        if let Some(reason) = update_block_reason(c) {
            r.skipped += 1;
            why.get_or_insert_with(|| reason.to_string());
            continue;
        }
        if let Some(l) = ctx.live {
            l.working_on(c);
        }
        let (ok, freed) = match crate::archive::move_out(ctx.paths, &c.path, &root, &c.name, &r.id) {
            Ok(m) => {
                r.freed += m.bytes;
                r.deleted += 1;
                (true, m.bytes)
            }
            Err(e) => {
                r.skipped += 1;
                why.get_or_insert(e);
                (false, 0)
            }
        };
        if let Some(l) = ctx.live {
            l.finished(c, ok, freed);
        }
    }
    r.message = if r.deleted == 0 {
        why.clone().unwrap_or_else(|| "Nothing to move.".into())
    } else {
        format!(
            "Moved {} ({}) to {}. They still open from where they were.",
            plural(r.deleted, "folder"),
            human_bytes(r.freed),
            root.display()
        )
    };
    if r.deleted > 0 && r.skipped > 0 {
        r.message.push_str(&format!(" {} stayed: {}", plural(r.skipped, "folder"), why.unwrap_or_default()));
    }
    if left_alone > 0 {
        r.message = format!("Stopped. {} {} left where they were.", r.message, plural(left_alone, "folder"));
    }
    r
}

/// What cleaning a spot would take right now: each item's paths and size. The app checks these
/// afterwards to tell what an administrator clean-up did, so the elevated `mom` never has to
/// write anything back.
pub fn plan(id: &str, ctx: &TrashCtx, pick: Option<&Pick>) -> Vec<(Vec<PathBuf>, u64)> {
    let Some(def) = def(id) else { return vec![] };
    if pick.is_some_and(|p| p.discard_error(id).is_some()) {
        return vec![];
    }
    if def.action == "tool" {
        return vec![];
    }
    let pick = pick.filter(|p| !p.is_default());
    let f = find_for(id, ctx, pick);
    choose(&f, pick).into_iter().map(|c| (c.targets.clone(), c.bytes)).collect()
}

/// The sentence for a clean-up's result.
pub fn result_message(id: &str, r: &CleanReport) -> String {
    def(id).map(|d| cleaned_message(d, r)).unwrap_or_default()
}

fn cleaned_message(def: &Def, r: &CleanReport) -> String {
    let mut msg = if r.deleted == 0 {
        if r.freed > 0 {
            format!("Took out {} of trash, but some items remain.", human_bytes(r.freed))
        } else if r.skipped > 0 {
            "Nothing was removed.".to_string()
        } else {
            "Already spotless.".to_string()
        }
    } else {
        match def.action {
            "recycle" => format!(
                "Moved {} ({}) to the Recycle Bin. Changed your mind? It's all in there.",
                plural(r.deleted, "item"),
                human_bytes(r.freed)
            ),
            "empty-bin" => format!("Deleted {} from the Recycle Bin: {} gone for good.", plural(r.deleted, "item"), human_bytes(r.freed)),
            _ if r.freed == 0 => format!("Took out {}, all empty.", plural(r.deleted, "item")),
            _ => format!("Took out {} of trash ({}).", human_bytes(r.freed), plural(r.deleted, "item")),
        }
    };
    if r.skipped > 0 {
        let why = if needs_admin(def.id) {
            "in use, blocked by permissions, changed since mom looked, or in a folder other users can change"
        } else {
            "in use, blocked by permissions, or changed since mom looked"
        };
        msg.push_str(&format!(" {} stayed: {why}.", plural(r.skipped, "item")));
    }
    msg
}

fn run_tool(id: &str, ctx: &TrashCtx, mut r: CleanReport) -> CleanReport {
    let before = find(id, ctx).tool.map(|t| t.0).unwrap_or(0);
    let after_total = |ctx: &TrashCtx| find(id, ctx).tool.map(|t| t.0).unwrap_or(0);
    let result: Result<(), String> = match id {
        "uv-cache" => match crate::util::run("uv", &["cache", "prune"], Duration::from_secs(600)) {
            Ok(out) if out.ok() => Ok(()),
            Ok(out) => Err(format!("uv cache prune failed: {}", out.err_text().trim())),
            Err(e) => Err(format!("couldn't run uv: {e}")),
        },
        "pnpm-store" => match crate::util::on_path("pnpm") {
            Some(pnpm) => match crate::util::run(&pnpm.to_string_lossy(), &["store", "prune"], Duration::from_secs(600)) {
                Ok(out) if out.ok() => Ok(()),
                Ok(out) => Err(format!("pnpm store prune failed: {}", out.err_text().trim())),
                Err(e) => Err(format!("couldn't run pnpm: {e}")),
            },
            None => Err("pnpm isn't on your PATH.".into()),
        },
        _ => Err(format!("mom doesn't know how to clean {id}")),
    };
    match result {
        Ok(()) => {
            r.freed = before.saturating_sub(after_total(ctx));
            r.deleted = u64::from(r.freed > 0);
            r.message = match (id, r.freed) {
                (_, 0) => "Ran it. Nothing needed pruning; already spotless.".to_string(),
                ("uv-cache", f) => format!("uv tidied its cache: {} back.", human_bytes(f)),
                (_, f) => format!("pnpm pruned its store: {} back.", human_bytes(f)),
            };
        }
        Err(e) => r.message = e,
    }
    r
}

/// For the CLI: "1.2 GB can go of 15 GB".
pub fn describe(s: &Spot) -> String {
    if !s.available {
        return "not found".into();
    }
    if s.total_bytes == 0 && s.reclaimable_bytes == 0 {
        return "empty".into();
    }
    let (go, total) = (human_bytes(s.reclaimable_bytes), human_bytes(s.total_bytes));
    if s.action == "tool" {
        format!("up to {go} can go of {total} (the tool decides)")
    } else if s.exact {
        format!("{go} can go of {total}")
    } else {
        format!("{go}+ can go of {total}+ (too many files to count them all)")
    }
}

/// Age of a file in seconds (0 if unknown).
pub fn age_of(path: &Path, now: u64) -> u64 {
    std::fs::symlink_metadata(path).ok().filter(|m| !m.is_symlink())
        .and_then(|m| cleanup_recency(&m)).map(|at| now.saturating_sub(at)).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TrashConfig;

    pub(super) fn touch_old(path: &Path, days: u64) {
        touch_secs(path, days * 86_400);
    }

    pub(super) fn touch_secs(path: &Path, secs: u64) {
        let t = std::time::SystemTime::now() - Duration::from_secs(secs);
        let root = path.ancestors().find(|p| p.file_name().is_some_and(|n| {
            let n = n.to_string_lossy();
            n.starts_with("mom-trash-") && n.ends_with(&format!("-{}", std::process::id()))
        })).expect("only backdate an isolated trash fixture");
        set_fixture_time(path, t);
        if path == root { return; }
        // Creating children changes directory dates. Model a genuinely old tree by setting each
        // ancestor to its newest child's evidence, stopping at the isolated fixture boundary.
        for dir in path.ancestors().skip(1) {
            let at = std::fs::read_dir(dir).unwrap().map(|e| {
                let e = e.unwrap();
                cleanup_recency(&std::fs::symlink_metadata(e.path()).unwrap()).unwrap()
            }).max().unwrap_or_else(|| t.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());
            set_fixture_time(dir, std::time::UNIX_EPOCH + Duration::from_secs(at));
            if dir == root { break; }
        }
    }

    pub(super) fn touch_directory(path: &Path, secs: u64) {
        assert!(path.ancestors().any(|p| p.file_name().is_some_and(|n| {
            let n = n.to_string_lossy();
            n.starts_with("mom-trash-") && n.ends_with(&format!("-{}", std::process::id()))
        })), "only backdate an isolated trash fixture directory");
        set_fixture_time(path, std::time::SystemTime::now() - Duration::from_secs(secs));
    }

    fn set_fixture_time(path: &Path, at: std::time::SystemTime) {
        let mut options = std::fs::OpenOptions::new();
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.write(true).custom_flags(0x0200_0000); // directories need BACKUP_SEMANTICS
        }
        #[cfg(not(windows))]
        { options.read(true); }
        let file = options.open(path).unwrap();
        let times = std::fs::FileTimes::new().set_modified(at);
        #[cfg(windows)]
        let times = {
            use std::os::windows::fs::FileTimesExt;
            times.set_created(at)
        };
        file.set_times(times).unwrap();
    }

    pub(super) fn write(path: &Path, bytes: usize, days_old: u64) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![b'x'; bytes]).unwrap();
        touch_old(path, days_old);
    }

    /// A fake machine with nothing running.
    pub(super) fn ctx<'a>(paths: &'a Paths, cfg: &'a TrashConfig) -> TrashCtx<'a> {
        TrashCtx {
            paths,
            cfg,
            live_sessions: HashSet::new(),
            in_use: String::new(),
            running_exes: HashSet::new(),
            project_roots: vec![],
            docker_running: false,
            now: now_secs(),
            elevated: false, // tests may well run as administrator (CI does)
            installed: Some(crate::installed::Installed::with(&[])), // nothing, unless a test says otherwise
            live: None,
            list_scale: 1,
            windows_update_probe: None,
        }
    }

    /// A link to a folder, the way the skills CLI makes them in agent folders.
    pub(super) fn link(link: &Path, target: &Path) {
        #[cfg(windows)]
        junction::create(target, link).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    pub(super) fn unlink(link: &Path) {
        let _ = std::fs::remove_dir(link).or_else(|_| std::fs::remove_file(link));
    }

    pub(super) fn fixture(name: &str) -> PathBuf {
        assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')), "fixture name must be one path component");
        // Failed/interrupted fixtures must never appear in a user's real Old temp files scan.
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/mom-test-fixtures");
        std::fs::create_dir_all(&base).unwrap();
        let base = std::fs::canonicalize(base).unwrap();
        let root = base.join(format!("mom-trash-{name}-{}", std::process::id()));
        assert_eq!(root.parent(), Some(base.as_path()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    fn item<'s>(spot: &'s Spot, name_part: &str) -> &'s Item {
        spot.items.iter().find(|i| i.name.contains(name_part)).unwrap_or_else(|| panic!("no item like {name_part:?} in {:#?}", spot.items))
    }

    #[test]
    fn recent_folder_activity_keeps_old_contents_in_temp_and_claude() {
        let root = fixture("recent-containers");
        let paths = Paths::under(&root);
        let temp = paths.temp.join("unpacked-yesterday");
        let session = paths.temp.join("claude").join("C--Workspace-demo").join("11111111-2222-3333-4444-555555555555");
        write(&temp.join("old-payload.bin"), 3, 100);
        write(&session.join("scratchpad").join("old.txt"), 3, 100);
        touch_directory(&temp, 0);
        touch_directory(&session, 0);
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let temp_spot = measure("old-temp", &ctx).unwrap();
        let claude_spot = measure("claude-temp", &ctx).unwrap();
        let temp_item = item(&temp_spot, "unpacked-yesterday");
        let claude_item = item(&claude_spot, "Session 11111111");
        for candidate in [temp_item, claude_item] {
            assert!(candidate.age_known && candidate.age_secs < 60);
            assert!(!candidate.goes, "a fresh container cannot inherit old contents' age");
        }
        assert!(plan("old-temp", &ctx, None).is_empty());
        assert!(plan("claude-temp", &ctx, None).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn incomplete_age_cannot_be_selected_even_if_a_verdict_or_pick_requests_it() {
        let root = fixture("unknown-age-picks");
        let paths = Paths::under(&root);
        let dir = paths.temp.join("partial");
        write(&dir.join("one.bin"), 3, 100);
        write(&dir.join("two.bin"), 3, 100);
        let t = tally_many(std::slice::from_ref(&dir), 2, u64::MAX).pop().unwrap();
        assert!(!t.complete);
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let mut c = judge(Cand::new(dir.clone(), "partial", &t, ctx.now), &ctx, 86_400);
        assert!(!c.age_known && !c.goes && !c.optional);
        assert_eq!(c.reason, UNKNOWN_AGE);
        c = c.kept_but_tickable("a caller tried to override unknown evidence");
        assert!(!c.optional);
        let item = Cand::new(dir.clone(), "partial", &t, ctx.now).into_item();
        assert_eq!(serde_json::to_value(item).unwrap()["ageKnown"], false);
        // Defense at selection time also rejects a future collector accidentally setting flags.
        c.goes = true;
        c.optional = true;
        let mut f = Found::at(dir.clone());
        f.cands.push(c);
        let pick = Pick { new: true, ticked: vec![dir.to_string_lossy().into_owned()], ..Default::default() };
        assert!(choose(&f, None).is_empty() && choose(&f, Some(&pick)).is_empty());
        assert_eq!(sizes(&[root.join("missing")]), vec![(0, 0)]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cached_scans_need_current_age_evidence_and_legacy_items_have_unknown_age() {
        let root = fixture("scan-evidence-version");
        let paths = Paths::under(&root);
        write(&paths.temp.join("old.bin"), 3, 100);
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let spot = measure("old-temp", &ctx).unwrap();
        let mut legacy = serde_json::to_value(&spot.items[0]).unwrap();
        legacy.as_object_mut().unwrap().remove("ageKnown");
        assert!(!serde_json::from_value::<Item>(legacy).unwrap().age_known);
        save(&paths, &[spot]);
        assert_eq!(load_saved(&paths).len(), 1);
        let saved = saved_file(&paths);
        let mut old: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
        old.as_object_mut().unwrap().remove("scanVersion");
        std::fs::write(&saved, serde_json::to_vec(&old).unwrap()).unwrap();
        assert!(load_saved(&paths).is_empty(), "a matching app version cannot revive old age evidence");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn broad_temp_cleanup_keeps_nested_browser_data_even_when_explicitly_picked() {
        let root = fixture("temp-browser-data");
        let paths = Paths::under(&root);
        let profile = paths.temp.join("portable-tool");
        write(&profile.join("nested").join("Network").join("Cookies"), 3, 100);
        let disposable = paths.temp.join("ordinary-cache");
        write(&disposable.join("Cache").join("data_1"), 3, 100);
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let found = find("old-temp", &ctx);
        let protected = found.cands.iter().find(|c| c.path == profile).unwrap();
        assert!(protected.browser_data && !protected.goes && !protected.optional);
        assert_eq!(protected.reason, BROWSER_DATA);
        let pick = Pick { goes: false, new: true, ticked: vec![profile.to_string_lossy().into_owned()], ..Default::default() };
        assert!(choose(&found, Some(&pick)).is_empty());
        let chosen = choose(&found, None);
        assert_eq!(chosen.len(), 1);
        assert_eq!(chosen[0].path, disposable);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn claude_scratch_of_dead_sessions_goes_live_sessions_stay() {
        let root = fixture("claude");
        let paths = Paths::under(&root);
        let claude = paths.temp.join("claude").join("C--Workspace-storefront");
        let dead = "0191f20b-ae85-40af-b308-93c34777b8cb";
        let live = "e9c3412f-ef20-4ba5-a0c7-2d71609fde82";
        let fresh = "11111111-2222-3333-4444-555555555555";
        write(&claude.join(dead).join("scratchpad").join("shot.png"), 5000, 70);
        write(&claude.join(live).join("scratchpad").join("notes.md"), 700, 3);
        write(&claude.join(fresh).join("tasks").join("out.txt"), 300, 0);
        // The dead session's transcript has a title.
        let transcript = paths.claude_dir().join("projects").join("C--Workspace-storefront").join(format!("{dead}.jsonl"));
        std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
        std::fs::write(
            &transcript,
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"Fix the login bug\"}}\n",
        )
        .unwrap();
        touch_old(&transcript, 70);
        // Codex: a stale half-written state file and an old index folder in Temp.
        write(&paths.codex_dir().join("..codex-global-state.json.tmp-1790-abc"), 1000, 70);
        write(&paths.codex_dir().join(".codex-global-state.json"), 1000, 70);
        write(&paths.temp.join("codex-index-1234").join("idx.bin"), 2000, 70);
        // A week-old random temp folder and a fresh one.
        write(&paths.temp.join("someinstaller").join("a.log"), 100, 30);
        write(&paths.temp.join("busy").join("b.log"), 100, 0);

        let cfg = TrashConfig::default();
        let mut ctx = ctx(&paths, &cfg);
        ctx.live_sessions.insert(live.to_string());
        let spot = measure("claude-temp", &ctx).unwrap();
        assert_eq!(spot.total_bytes, 6000);
        assert_eq!(spot.reclaimable_bytes, 5000);
        assert_eq!(spot.reclaimable_items, 1);
        assert_eq!((spot.kept_bytes, spot.kept_items), (1000, 2));
        assert_eq!((spot.group.as_str(), spot.action.as_str(), spot.safety.as_str()), ("agents", "delete", "safe"));
        assert!(spot.effect.starts_with("Deletes 1 scratch folder (5 KB) from Claude Code sessions that have ended, permanently"), "{}", spot.effect);

        // Every item says why it goes or stays.
        let gone = item(&spot, "Fix the login bug");
        assert!(gone.goes);
        assert!(gone.name.starts_with("storefront · "), "{}", gone.name);
        assert_eq!(gone.sub.as_deref(), Some("1 file"));
        assert!(gone.reason.starts_with("Last changed 70 days ago"), "{}", gone.reason);
        let running = item(&spot, "Session e9c3412f");
        assert!(!running.goes);
        assert_eq!(running.reason, "Session is still running");
        let new = item(&spot, "Session 11111111");
        assert!(!new.goes);
        assert!(new.reason.contains("(kept for 67 days)"), "{}", new.reason);
        assert_eq!(spot.items[0].name, gone.name, "what goes is listed first");

        let codex = measure("codex-temp", &ctx).unwrap();
        assert_eq!(codex.reclaimable_bytes, 3000, "tmp write + index, never the real state file");

        let old = measure("old-temp", &ctx).unwrap();
        assert_eq!(old.reclaimable_items, 1);

        // Something running from the old folder protects it.
        ctx.in_use.push_str(&norm_path(&paths.temp.join("someinstaller").join("tool.exe").to_string_lossy()));
        let old = measure("old-temp", &ctx).unwrap();
        assert_eq!(old.reclaimable_items, 0);
        assert_eq!(item(&old, "someinstaller").reason, IN_USE);
        ctx.in_use.clear();

        let r = clean("claude-temp", &ctx, None);
        assert_eq!(r.deleted, 1);
        assert_eq!(r.freed, 5000);
        assert!(!claude.join(dead).exists());
        assert!(claude.join(live).exists());
        assert!(claude.join(fresh).exists());

        let r = clean("codex-temp", &ctx, None);
        assert_eq!(r.deleted, 2);
        assert!(paths.codex_dir().join(".codex-global-state.json").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn ticking_and_unticking_with_things_in_use_locked() {
        let root = fixture("pick");
        let paths = Paths::under(&root);
        write(&paths.temp.join("old-a").join("a.log"), 100, 30);
        write(&paths.temp.join("old-b").join("b.log"), 200, 30);
        write(&paths.temp.join("new-c").join("c.log"), 300, 0);
        write(&paths.temp.join("busy-d").join("d.log"), 400, 0);
        let cfg = TrashConfig::default();
        let mut ctx = ctx(&paths, &cfg);
        ctx.in_use = norm_path(&paths.temp.join("busy-d").to_string_lossy());
        let temp = measure("old-temp", &ctx).unwrap();
        assert!(item(&temp, "new-c").optional, "new, but nothing uses it: can be ticked");
        assert!(!item(&temp, "busy-d").optional, "in use: locked");
        assert_eq!((temp.new_items, temp.new_bytes), (1, 300));
        let path = |n: &str| paths.temp.join(n).to_string_lossy().to_string();
        // Untick one that goes; tick the new one, and the busy one (which stays anyway).
        let pick = Pick { unticked: vec![path("old-b")], ticked: vec![path("new-c"), path("busy-d")], ..Default::default() };
        let r = clean("old-temp", &ctx, Some(&pick));
        assert_eq!((r.deleted, r.skipped, r.freed), (2, 1, 400));
        assert!(!paths.temp.join("old-a").exists());
        assert!(paths.temp.join("old-b").exists());
        assert!(!paths.temp.join("new-c").exists());
        assert!(paths.temp.join("busy-d").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Reach the real Forgotten app data threshold without allocating a large byte buffer.
    fn forgotten_folder(paths: &Paths, name: &str) -> PathBuf {
        let folder = paths.local_appdata.join(name);
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("data.bin");
        std::fs::File::create(&file).unwrap().set_len(100 << 20).unwrap();
        touch_old(&file, 400);
        folder
    }

    #[test]
    fn forgotten_discard_takes_only_exact_folders_without_an_archive_and_default_still_moves() {
        let root = fixture("forgotten-discard-exact");
        let paths = Paths::under(&root);
        let selected = forgotten_folder(&paths, "Antigravity Fixture");
        let unseen = forgotten_folder(&paths, "Unseen App Fixture");
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let spot = measure("forgotten-apps", &ctx).unwrap();
        assert_eq!(spot.action, "move");
        assert!(cfg.archive_dir.is_none());
        assert_eq!(spot.reclaimable_items, 2);
        let default = clean("forgotten-apps", &ctx, None);
        assert_eq!(default.deleted, 0);
        assert_eq!(default.message, "Choose where to keep them first.");
        assert!(selected.exists() && unseen.exists());
        // Recycle alone does not silently change an existing archive action.
        assert_eq!(clean("forgotten-apps", &ctx, Some(&Pick { recycle: true, ..Default::default() })).deleted, 0);
        let pick = Pick {
            goes: false,
            new: false,
            ticked: vec![selected.to_string_lossy().to_string()],
            discard: true,
            ..Default::default()
        };
        let planned = plan("forgotten-apps", &ctx, Some(&pick));
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].0, vec![selected.clone()]);
        // Recycle uses the same exact, freshly revalidated targets without invoking it here.
        let recycled = Pick { recycle: true, ..pick.clone() };
        assert_eq!(plan("forgotten-apps", &ctx, Some(&recycled)), planned);
        let r = clean("forgotten-apps", &ctx, Some(&pick));
        assert_eq!((r.deleted, r.skipped, r.freed), (1, 0, 100 << 20), "{r:?}");
        assert!(r.message.contains("for good"));
        assert!(!selected.exists() && unseen.join("data.bin").exists());
        assert!(crate::archive::list(&paths).is_empty(), "direct removal creates no archive record");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn forgotten_discard_rechecks_kept_busy_recent_linked_and_unknown_paths() {
        let root = fixture("forgotten-discard-blocked");
        let paths = Paths::under(&root);
        let busy = forgotten_folder(&paths, "Busy App Fixture");
        let kept = forgotten_folder(&paths, "Kept App Fixture");
        let recent = forgotten_folder(&paths, "Recent App Fixture");
        let linked = forgotten_folder(&paths, "Linked App Fixture");
        let mut cfg = TrashConfig::default();
        let before = measure("forgotten-apps", &ctx(&paths, &cfg)).unwrap();
        assert_eq!(before.reclaimable_items, 4);
        let outside = root.join("outside");
        write(&outside.join("precious.txt"), 20, 400);
        let inside = linked.join("elsewhere");
        link(&inside, &outside);
        touch_old(&recent.join("data.bin"), 0);
        cfg.keep.push(kept.to_string_lossy().to_string());
        let mut ctx = ctx(&paths, &cfg);
        ctx.in_use = norm_path(&busy.join("app.exe").to_string_lossy());
        let mut ticked: Vec<String> = before.items.iter().filter(|i| i.goes).map(|i| i.path.clone()).collect();
        ticked.push(outside.to_string_lossy().to_string());
        let pick = Pick { goes: false, new: false, discard: true, ticked, ..Default::default() };
        assert!(plan("forgotten-apps", &ctx, Some(&pick)).is_empty());
        let r = clean("forgotten-apps", &ctx, Some(&pick));
        assert_eq!((r.deleted, r.skipped, r.freed), (0, 5, 0), "{r:?}");
        for folder in [&busy, &kept, &recent, &linked] {
            assert!(folder.join("data.bin").exists());
        }
        assert!(outside.join("precious.txt").exists());
        unlink(&inside);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn forgotten_discard_rejects_broad_selection_and_other_spots_before_cleaning() {
        let root = fixture("forgotten-discard-validation");
        let paths = Paths::under(&root);
        let folder = paths.temp.join("old-data");
        write(&folder.join("data.bin"), 20, 400);
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let exact = Pick { goes: false, new: false, discard: true, ticked: vec![folder.to_string_lossy().to_string()], ..Default::default() };
        let r = clean("old-temp", &ctx, Some(&exact));
        assert_eq!(r.deleted, 0);
        assert!(r.message.contains("only available for Forgotten app data"));
        assert!(plan("old-temp", &ctx, Some(&exact)).is_empty());
        for broad in [Pick { goes: true, ..exact.clone() }, Pick { new: true, ..exact.clone() }] {
            assert!(plan("forgotten-apps", &ctx, Some(&broad)).is_empty());
            let r = clean("forgotten-apps", &ctx, Some(&broad));
            assert_eq!(r.deleted, 0);
            assert!(r.message.contains("exact app data folders"));
        }
        assert!(folder.join("data.bin").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn legacy_picks_never_enable_discard_and_explicit_mode_survives_serialization() {
        let legacy: Pick = serde_json::from_str(r#"{"goes":false,"new":false,"ticked":["C:\\old-app"],"recycle":true}"#).unwrap();
        assert!(!legacy.discard);
        let empty: Pick = serde_json::from_str("{}").unwrap();
        assert!(empty.is_default() && !empty.discard);
        let explicit = Pick { discard: true, ..legacy };
        assert!(!explicit.is_default());
        let back: Pick = serde_json::from_str(&serde_json::to_string(&explicit).unwrap()).unwrap();
        assert!(back.discard && back.recycle && !back.goes && !back.new);
        assert_eq!(back.ticked, explicit.ticked);
    }

    #[test]
    fn failed_and_partial_cleanups_are_not_reported_as_spotless() {
        let mut r = CleanReport { id: "dev-leftovers".into(), skipped: 1, ..Default::default() };
        let failed = result_message(&r.id, &r);
        assert!(failed.starts_with("Nothing was removed."), "{failed}");
        assert!(failed.contains("blocked by permissions"), "{failed}");
        assert!(!failed.contains("spotless"), "{failed}");

        // A folder can lose some files while the folder itself cannot be removed.
        r.freed = 2048;
        let partial = result_message(&r.id, &r);
        assert!(partial.contains(&human_bytes(r.freed)) && partial.contains("some items remain"), "{partial}");
        assert!(!partial.contains("spotless"), "{partial}");

        let empty = CleanReport { id: r.id.clone(), ..Default::default() };
        assert_eq!(result_message(&empty.id, &empty), "Already spotless.");
    }

    #[test]
    fn skipped_recycling_and_admin_cleanup_include_permission_restrictions() {
        let r = CleanReport { skipped: 1, ..Default::default() };
        let recycled = recycled_message(&r);
        assert!(recycled.starts_with("Nothing went to the Recycle Bin."), "{recycled}");
        assert!(recycled.contains("blocked by permissions"), "{recycled}");
        let admin = result_message("windows-junk", &r);
        assert!(admin.starts_with("Nothing was removed.") && admin.contains("blocked by permissions"), "{admin}");
        assert!(admin.contains("a folder other users can change"), "{admin}");
    }

    #[test]
    fn what_was_ticked_travels_to_the_elevated_mom_which_only_cleans_admin_spots() {
        // The app hands the administrator `mom` what was ticked, as hex on its command line.
        let pick = Pick { goes: false, new: true, ticked: vec![r"C:\x".into()], ..Default::default() };
        let hex = crate::elevate::to_hex(&serde_json::to_string(&pick).unwrap());
        let back: Pick = serde_json::from_str(&crate::elevate::from_hex(&hex).unwrap()).unwrap();
        assert_eq!((back.goes, back.new, back.ticked), (false, true, vec![r"C:\x".to_string()]));
        assert!(needs_admin("windows-junk") && needs_admin("admin-leftovers") && !needs_admin("old-temp"));
        assert!(!cleans_without_asking("windows-junk"), "never cleaned without asking");
        let r = CleanReport { id: "windows-junk".into(), freed: 5, deleted: 1, skipped: 2, message: String::new() };
        assert!(result_message("windows-junk", &r).contains("a folder other users can change"), "{}", result_message("windows-junk", &r));
    }

    #[test]
    fn as_administrator_your_own_files_are_left_alone() {
        let root = fixture("elevated");
        let paths = Paths::under(&root);
        write(&paths.temp.join("old-a").join("a.log"), 100, 30);
        let cfg = TrashConfig::default();
        let mut ctx = ctx(&paths, &cfg);
        ctx.elevated = true;
        let r = clean("old-temp", &ctx, None);
        assert_eq!(r.deleted, 0);
        assert!(r.message.contains("not as administrator"), "{}", r.message);
        assert!(paths.temp.join("old-a").join("a.log").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn what_you_said_to_keep_never_goes_whatever_is_ticked() {
        let root = fixture("keep");
        let paths = Paths::under(&root);
        let (a, b, c) = (paths.temp.join("old-a"), paths.temp.join("old-b"), paths.temp.join("old-c"));
        write(&a.join("a.log"), 100, 30);
        write(&b.join("b.log"), 200, 30);
        write(&c.join("deep").join("c.log"), 300, 30);
        let text = |p: &Path| p.to_string_lossy().to_string();
        let mut cfg = TrashConfig::default();
        // Spelled another way (capitals, the other slash): the same place.
        assert_eq!(set_keep(&mut cfg, &text(&a).to_uppercase().replace('\\', "/"), true), Ok(true));
        assert_eq!(set_keep(&mut cfg, &text(&a), true), Ok(false), "kept already");
        // Something inside a folder that would go: the folder stays too, or it would go along.
        assert_eq!(set_keep(&mut cfg, &text(&c.join("deep").join("c.log")), true), Ok(true));
        // What can't be kept says so: nobody goes on thinking it is.
        assert!(set_keep(&mut cfg, "  ", true).is_err() && set_keep(&mut cfg, &"x".repeat(2_000), true).is_err());
        let mut full = TrashConfig { keep: (0..MAX_KEPT).map(|i| format!(r"C:\kept\{i}")).collect(), ..Default::default() };
        assert!(set_keep(&mut full, r"C:\one-more", true).is_err() && full.keep.len() == MAX_KEPT);
        assert_eq!(set_keep(&mut full, r"c:/KEPT/7", false), Ok(true), "letting go always works");
        assert_eq!(cfg.keep.len(), 2);
        {
            let ctx = ctx(&paths, &cfg);
            let spot = measure("old-temp", &ctx).unwrap();
            let say = |n: &str| {
                let i = item(&spot, n);
                (i.goes, i.optional, i.kept, i.reason.clone())
            };
            assert_eq!(say("old-a"), (false, false, true, KEPT.to_string()));
            assert_eq!(say("old-c"), (false, false, false, HOLDS_KEPT.to_string()));
            assert!(item(&spot, "old-b").goes);
            assert_eq!((spot.reclaimable_items, spot.reclaimable_bytes, spot.kept_items, spot.new_items), (1, 200, 2, 0));
            // What you keep leads the list of what stays, so it can always be let go from there.
            assert_eq!(spot.items.iter().find(|i| !i.goes).unwrap().name, "old-a");
            // Ticked anyway (named one by one, and "tick all" in both lists): nothing of yours goes.
            let pick = Pick { new: true, ticked: vec![text(&a), text(&c)], ..Default::default() };
            let r = clean("old-temp", &ctx, Some(&pick));
            assert_eq!(r.deleted, 1, "{r:?}");
            assert!(a.join("a.log").exists() && c.join("deep").join("c.log").exists() && !b.exists());
        }
        // Let go again, it goes like anything else.
        assert_eq!((set_keep(&mut cfg, &text(&a), false), set_keep(&mut cfg, &text(&a), false)), (Ok(true), Ok(false)));
        {
            let ctx = ctx(&paths, &cfg);
            assert!(item(&measure("old-temp", &ctx).unwrap(), "old-a").goes);
            // A pick can say what to keep by itself: the administrator run is told that way.
            let pick = Pick { keep: vec![text(&a)], ..Default::default() };
            assert!(plan("old-temp", &ctx, Some(&pick)).is_empty());
            let r = clean("old-temp", &ctx, Some(&pick));
            assert_eq!(r.deleted, 0, "{r:?}");
            assert!(a.exists() && c.exists());
            assert_eq!(clean("old-temp", &ctx, None).deleted, 1);
            assert!(!a.exists() && c.join("deep").join("c.log").exists());
        }
        // A folder whose name only starts the same isn't inside it.
        assert!(within(r"c:\temp\old-a\x", r"c:\temp\old-a") && within(r"c:\temp\old-a", r"c:\temp\old-a"));
        assert!(!within(r"c:\temp\old-ab", r"c:\temp\old-a") && !within(r"c:\temp", r"c:\temp\old-a") && within(r"d:\x", "d:\\"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_spot_lists_more_when_asked_and_saves_the_usual_number() {
        let root = fixture("list-more");
        let paths = Paths::under(&root);
        let n = MAX_GOES + 20;
        for i in 0..n {
            write(&paths.temp.join(format!("old-{i:04}")).join("a.log"), 10 + i, 30);
        }
        let cfg = TrashConfig::default();
        let mut ctx = ctx(&paths, &cfg);
        let usual = measure("old-temp", &ctx).unwrap();
        assert_eq!((usual.items.len(), usual.reclaimable_items as usize), (MAX_GOES, n));
        ctx.list_scale = MORE;
        let more = measure("old-temp", &ctx).unwrap();
        assert_eq!((more.items.len(), more.reclaimable_items as usize), (n, n));
        // However many are showing, the file read at every start holds the usual number.
        save(&paths, &[more]);
        assert_eq!(load_saved(&paths)[0].items.len(), MAX_GOES);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_measurement_survives_a_restart() {
        let root = fixture("saved");
        let paths = Paths::under(&root);
        write(&paths.temp.join("old-a").join("a.log"), 100, 30);
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        assert!(load_saved(&paths).is_empty());
        let spots = vec![measure("old-temp", &ctx).unwrap()];
        save(&paths, &spots);
        let back = load_saved(&paths);
        assert_eq!(back.len(), 1);
        assert_eq!((back[0].id.as_str(), back[0].reclaimable_bytes, back[0].measured_at), ("old-temp", 100, spots[0].measured_at));
        assert_eq!(back[0].items[0].name, "old-a");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_measurement_from_another_version_is_measured_again() {
        let root = fixture("saved-by-another");
        let paths = Paths::under(&root);
        write(&paths.temp.join("old-a").join("a.log"), 100, 30);
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let spots = vec![measure("old-temp", &ctx).unwrap()];
        // Saved by the version before: its verdicts aren't this one's.
        save_as(&paths, &spots, "0.0.1");
        assert!(load_saved(&paths).is_empty());
        // Saved the way mom used to, as a bare list with no version.
        crate::util::write_atomic(&saved_file(&paths), &serde_json::to_string(&spots).unwrap()).unwrap();
        assert!(load_saved(&paths).is_empty());
        // Anything else that isn't a measurement.
        crate::util::write_atomic(&saved_file(&paths), "{\"version\":7,\"spots\":\"x\"}").unwrap();
        assert!(load_saved(&paths).is_empty());
        save(&paths, &spots);
        assert_eq!(load_saved(&paths).len(), 1);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn tick_all_and_untick_all_cover_unlisted_items_and_empty_folders_count() {
        let root = fixture("all");
        let paths = Paths::under(&root);
        write(&paths.temp.join("old-a").join("a.log"), 100, 30);
        for i in 0..3 {
            std::fs::create_dir_all(paths.temp.join(format!("empty-{i}"))).unwrap();
        }
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let temp = measure("old-temp", &ctx).unwrap();
        assert_eq!((temp.new_items, temp.new_bytes), (3, 0), "new empty folders: kept, but can be ticked");
        // Untick all that goes, tick all that's new, without naming any of them.
        let r = clean("old-temp", &ctx, Some(&Pick { goes: false, new: true, ..Default::default() }));
        assert_eq!((r.deleted, r.freed), (3, 0));
        assert!(paths.temp.join("old-a").exists());
        assert!(!paths.temp.join("empty-0").exists());
        assert_eq!(count_size(4634, "item", "items", 0), "4,634 items (all empty)");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn old_installers_go_to_the_recycle_bin_new_ones_stay() {
        let root = fixture("installers");
        let paths = Paths::under(&root);
        write(&paths.downloads.join("OldSetup.exe"), 4000, 60);
        write(&paths.downloads.join("tool.msi"), 2000, 45);
        write(&paths.downloads.join("NewSetup.exe"), 3000, 2);
        write(&paths.downloads.join("photo.jpg"), 9000, 400);
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let spot = measure("old-installers", &ctx).unwrap();
        assert_eq!((spot.action.as_str(), spot.safety.as_str(), spot.group.as_str()), ("recycle", "review", "personal"));
        assert_eq!(spot.reclaimable_items, 2);
        assert_eq!(spot.reclaimable_bytes, 6000);
        assert_eq!(spot.kept_items, 1, "the photo isn't an installer at all");
        assert!(item(&spot, "OldSetup.exe").goes);
        assert!(item(&spot, "tool.msi").goes);
        let new = item(&spot, "NewSetup.exe");
        assert!(!new.goes);
        assert!(new.optional, "only kept for being new, so it can be ticked");
        assert!(new.reason.contains("kept for 30 days"), "{}", new.reason);
        assert!(spot.effect.contains("Recycle Bin"), "{}", spot.effect);
        // What clean would hand to the Recycle Bin (not actually recycled in tests).
        let f = find("old-installers", &ctx);
        let names = |cs: Vec<&Cand>| {
            let mut v: Vec<String> = cs.iter().flat_map(|c| c.targets.iter().map(|t| name_of(t))).collect();
            v.sort();
            v
        };
        assert_eq!(names(choose(&f, None)), vec!["OldSetup.exe", "tool.msi"]);
        // Untick an old one, tick the new one.
        let pick = Pick { unticked: vec![item(&spot, "OldSetup.exe").path.clone()], ticked: vec![new.path.clone()], ..Default::default() };
        assert_eq!(names(choose(&f, Some(&pick))), vec!["NewSetup.exe", "tool.msi"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn recent_things_stay_unless_ticked() {
        let root = fixture("ticked");
        let paths = Paths::under(&root);
        write(&paths.local_appdata.join("CrashDumps").join("app.exe.42.dmp"), 500, 0);
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let dumps = measure("crash-dumps", &ctx).unwrap();
        let dump = item(&dumps, "app.exe");
        assert!(!dump.goes && dump.optional);
        assert_eq!(clean("crash-dumps", &ctx, None).deleted, 0);
        let r = clean("crash-dumps", &ctx, Some(&Pick { ticked: vec![dump.path.clone()], ..Default::default() }));
        assert_eq!((r.deleted, r.freed), (1, 500));
        assert!(!paths.local_appdata.join("CrashDumps").join("app.exe.42.dmp").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn npx_downloads_wait_as_long_as_agent_leftovers_and_what_you_run_stays() {
        let root = fixture("npx");
        let paths = Paths::under(&root);
        let npx = paths.local_appdata.join("npm-cache").join("_npx");
        let package = |hash: &str, dep: &str, days: u64| {
            let dir = npx.join(hash);
            write(&dir.join("package.json"), 0, days);
            std::fs::write(dir.join("package.json"), format!(r#"{{"dependencies":{{"{dep}":"*"}}}}"#)).unwrap();
            touch_old(&dir.join("package.json"), days);
            write(&dir.join("node_modules").join(dep).join("index.js"), 100, days);
        };
        package("aaa", "storefront-cli", 90); // old, and nothing runs it
        package("bbb", "nimbus-lint", 20); // fetched recently: kept while new
        package("ccc", "atlas-gen", 90); // old, but you run it
        let history = paths.appdata.join("Microsoft").join("Windows").join("PowerShell").join("PSReadLine");
        std::fs::create_dir_all(&history).unwrap();
        std::fs::write(history.join("ConsoleHost_history.txt"), "git status\nnpx atlas-gen --watch\n").unwrap();
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let spot = measure("npx-cache", &ctx).unwrap();
        let old = spot.items.iter().find(|i| i.path.ends_with("aaa")).unwrap();
        let new = spot.items.iter().find(|i| i.path.ends_with("bbb")).unwrap();
        let used = spot.items.iter().find(|i| i.path.ends_with("ccc")).unwrap();
        assert!(old.goes && old.reason.starts_with("Last changed 90 days ago"), "{}", old.reason);
        assert!(!new.goes && new.optional && new.reason.contains("kept for"), "{}", new.reason);
        assert!(!used.goes && !used.optional && used.reason.contains("shell history"), "{}", used.reason);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn hugging_face_models_are_only_ever_picked_by_you() {
        let root = fixture("hf");
        let paths = Paths::under(&root);
        let hub = paths.home.join(".cache").join("huggingface").join("hub");
        write(&hub.join("models--nimbus--depth-small").join("blobs").join("a"), 5000, 400);
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let spot = measure("huggingface-cache", &ctx).unwrap();
        let model = item(&spot, "depth-small");
        assert!(!model.goes && model.optional, "even a year-old model waits for a tick");
        assert_eq!(clean("huggingface-cache", &ctx, None).deleted, 0);
        assert!(spot.effect.contains("Nothing is picked for you"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_stopped_clean_leaves_the_rest_and_progress_says_what_went() {
        let root = fixture("live");
        let paths = Paths::under(&root);
        for n in 0..5 {
            write(&paths.temp.join(format!("old-{n}")).join("a.log"), 100, 30);
        }
        let cfg = TrashConfig::default();
        let mut ctx = ctx(&paths, &cfg);
        let live = Live::default();
        ctx.live = Some(&live);
        live.stop();
        let r = clean("old-temp", &ctx, None);
        assert_eq!(r.deleted, 0);
        assert!(r.message.starts_with("Stopped."), "{}", r.message);
        assert_eq!(entries(&paths.temp).len(), 5, "nothing went");
        live.reset();
        let r = clean("old-temp", &ctx, None);
        assert_eq!(r.deleted, 5);
        let now = live.now(0);
        assert_eq!((now.done, now.total, now.freed, now.gone.len()), (5, 5, 500, 5));
        assert_eq!(live.now(3).gone.len(), 2, "only what's new since the widget last asked");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_clean_that_goes_in_batches_counts_up_as_it_goes_too() {
        // Fifty-three small things went as one batch of 48 and one of 5: "0 of 53", then all of it.
        let root = fixture("live-batch");
        let paths = Paths::under(&root);
        for n in 0..53 {
            write(&paths.temp.join(format!("old-{n:02}")).join("a.log"), 100, 30);
        }
        let cfg = TrashConfig::default();
        let mut ctx = ctx(&paths, &cfg);
        let live = Live::default();
        ctx.live = Some(&live);
        live.stop_after.store(1, Ordering::Relaxed); // someone presses Stop when the count first moves
        let r = clean("old-temp", &ctx, None);
        let now = live.now(0);
        assert_eq!(batch_items(53), 3);
        assert_eq!((r.deleted, now.done, now.total), (3, 3, 53), "the count moved after the first few, and the rest were left: {}", r.message);
        assert_eq!((batch_items(1), batch_items(15), batch_items(400), batch_items(100_000)), (1, 1, 25, BATCH_ITEMS));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_clean_that_goes_one_item_at_a_time_counts_up_as_it_goes() {
        // Windows leftovers go one at a time (as do old build folders and anything recycled). The
        // widget showed "0 of 116 items · 0 B" until the very last one was gone.
        let root = fixture("live-each");
        let paths = Paths::under(&root);
        for (name, bytes) in [("AMD", 100), ("adobeTemp", 200), ("Windows10Upgrade", 300)] {
            write(&paths.system_drive.join(name).join("a.bin"), bytes, 200);
            write(&paths.system_drive.join(name).join("b.bin"), 1, 200);
        }
        let cfg = TrashConfig::default();
        let mut ctx = ctx(&paths, &cfg);
        let live = Live::default();
        ctx.live = Some(&live);
        // Someone presses Stop as soon as the counter says one is done. If nothing is said until
        // the end, nobody can, and all three go.
        live.stop_after.store(1, Ordering::Relaxed);
        let r = clean("windows-junk", &ctx, None);
        let now = live.now(0);
        assert_eq!((r.deleted, now.done, now.total, now.gone.len()), (1, 1, 3, 1), "{}", r.message);
        assert_eq!((now.freed, now.files, now.total_files), (r.freed, 2, 6), "what went is counted as it goes: bytes and files");
        assert_eq!(entries(&paths.system_drive).len(), 2, "the other two were left when it stopped");
        live.reset();
        live.stop_after.store(0, Ordering::Relaxed);
        let r = clean("windows-junk", &ctx, None);
        let now = live.now(0);
        assert_eq!((r.deleted, now.done, now.total, now.files, now.total_files), (2, 2, 2, 4, 4));
        assert_eq!(now.freed, r.freed);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn windows_update_that_becomes_pending_after_discovery_blocks_actual_removal() {
        use windows_update::State;
        for elevated in [false, true] {
            for blocked in [State::Pending, State::Unknown] {
                let root = fixture(&format!("update-race-{elevated}-{blocked:?}"));
                let mut paths = Paths::under(&root);
                paths.other_drives.push(root.join("d"));
                let setup = paths.system_drive.join("Windows10Upgrade");
                let other = paths.other_drives[0].join("$GetCurrent");
                let cache = paths.windows.join("SoftwareDistribution").join("Download").join("old-update");
                for path in [&setup, &other, &cache] { write(&path.join("payload.bin"), 100, 200); }
                let cfg = TrashConfig::default();
                let calls = std::cell::Cell::new(0);
                let probe = || {
                    calls.set(calls.get() + 1);
                    if calls.get() == 1 { State::Clear } else { blocked }
                };
                let mut ctx = ctx(&paths, &cfg);
                ctx.elevated = elevated;
                ctx.windows_update_probe = Some(&probe);
                let pick = Pick { new: true, ticked: [&setup, &other, &cache].iter().map(|p| p.to_string_lossy().into_owned()).collect(), ..Default::default() };
                let result = clean("windows-junk", &ctx, Some(&pick));
                assert_eq!(calls.get(), 2, "fresh discovery clears, first mutation blocks, later items stay blocked");
                assert_eq!((result.deleted, result.freed, result.skipped), (0, 0, 3));
                assert!(result.message.contains(blocked.reason().unwrap()));
                assert!(!result.message.contains("spotless"));
                for path in [&setup, &other, &cache] { assert_eq!(std::fs::read(path.join("payload.bin")).unwrap(), vec![b'x'; 100]); }
                std::fs::remove_dir_all(&root).unwrap();
            }
        }
    }

    #[test]
    fn windows_update_that_starts_mid_cleanup_keeps_remaining_items() {
        use windows_update::State;
        for blocked in [State::Pending, State::Unknown] {
            let root = fixture(&format!("update-mid-clean-{blocked:?}"));
            let paths = Paths::under(&root);
            let targets: Vec<_> = ["$GetCurrent", "Windows10Upgrade", "$WINDOWS.~TMP"]
                .into_iter().map(|name| paths.system_drive.join(name)).collect();
            for path in &targets { write(&path.join("payload.bin"), 100, 200); }
            let cfg = TrashConfig::default();
            let calls = std::cell::Cell::new(0);
            let probe = || {
                calls.set(calls.get() + 1);
                if calls.get() <= 2 { State::Clear } else { blocked }
            };
            let mut ctx = ctx(&paths, &cfg);
            ctx.windows_update_probe = Some(&probe);
            let result = clean("windows-junk", &ctx, Some(&Pick { new: true, ..Default::default() }));
            assert_eq!(calls.get(), 3, "discovery and the first operation clear; the next native check blocks and latches");
            assert_eq!((result.deleted, result.freed, result.skipped), (1, 100, 2));
            assert!(result.message.contains(blocked.reason().unwrap()));
            let remaining: Vec<_> = targets.iter().filter(|path| path.exists()).collect();
            assert_eq!(remaining.len(), 2);
            for path in remaining { assert_eq!(std::fs::read(path.join("payload.bin")).unwrap(), vec![b'x'; 100]); }
            std::fs::remove_dir_all(&root).unwrap();
        }
    }

    #[test]
    fn windows_update_paths_use_folder_boundaries_on_each_configured_drive() {
        let mut paths = Paths::under(Path::new("fixture"));
        paths.other_drives.push(PathBuf::from("other-drive"));
        for drive in std::iter::once(&paths.system_drive).chain(&paths.other_drives) {
            for name in ["$GetCurrent", "Windows10Upgrade", "$WINDOWS.~BT", "$WINDOWS.~WS", "$WINDOWS.~TMP", "$SysReset", "WUDownloadCache"] {
                assert!(windows_update_target(&drive.join(name), &paths));
                assert!(windows_update_target(&drive.join(name).join("inside").join("data.bin"), &paths));
                assert!(!windows_update_target(&drive.join(format!("{name}-backup")), &paths));
                assert!(!windows_update_target(&drive.join("photos").join(name), &paths));
            }
        }
        let downloads = paths.windows.join("SoftwareDistribution").join("Download");
        assert!(windows_update_target(&downloads.join("queued").join("payload.cab"), &paths));
        assert!(!windows_update_target(&downloads.with_file_name("Download-old").join("payload.cab"), &paths));
    }

    #[test]
    fn whole_caches_keep_their_folder_and_tools_report_upper_bounds() {
        let root = fixture("caches");
        let paths = Paths::under(&root);
        let cache = paths.local_appdata.join("npm-cache").join("_cacache");
        write(&cache.join("content-v2").join("a.tgz"), 1000, 1);
        write(&cache.join("index-v5").join("b"), 50, 1);
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let spot = measure("npm-cache", &ctx).unwrap();
        assert_eq!((spot.reclaimable_bytes, spot.total_bytes, spot.items.len()), (1050, 1050, 1));
        assert_eq!(spot.items[0].sub.as_deref(), Some("2 files, mostly archives"));
        let r = clean("npm-cache", &ctx, None);
        assert_eq!(r.freed, 1050);
        assert!(cache.is_dir() && entries(&cache).is_empty(), "the cache folder itself stays");
        assert!(!measure("yarn-cache", &ctx).unwrap().available);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn crash_dumps_name_the_app_and_keep_fresh_ones() {
        let root = fixture("crash");
        let paths = Paths::under(&root);
        write(&paths.local_appdata.join("CrashDumps").join("node.exe.12345.dmp"), 800, 3);
        write(&paths.local_appdata.join("CrashDumps").join("Code.exe.777.dmp"), 900, 0);
        let wer = paths.local_appdata.join("Microsoft").join("Windows").join("WER").join("ReportArchive");
        write(&wer.join("AppCrash_my_app.exe_8d3c1f_cab_0a1b2c").join("Report.wer"), 100, 10);
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let spot = measure("crash-dumps", &ctx).unwrap();
        assert_eq!(spot.reclaimable_items, 2);
        assert!(item(&spot, "node.exe").goes);
        assert!(item(&spot, "my_app.exe").goes);
        assert!(!item(&spot, "Code.exe").goes);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn names_from_folder_names() {
        assert!(is_render_leftover("remotion-v4.0.348-assetsabcdef"));
        assert!(is_render_leftover("garden-bundle-mine-1790000000-12"));
        assert_eq!(render_name("remotion-v4.0.529-assets27url9enpf"), "Remotion 4.0.529 render assets");
        assert_eq!(render_name("garden-bundle-mine-1790000000-12"), "garden render bundle");
        assert!(!is_render_leftover("bundle"));
        assert_eq!(crashed_app_from_dump("chrome-headless-shell.exe.144060.dmp"), "chrome-headless-shell.exe");
        assert_eq!(crashed_app_from_report("AppHang_Razer Central.exe_1f2e_abc"), "Razer Central.exe");
        assert_eq!(hf_repo("models--depth-anything--Depth-Anything-V2-Base-hf"), Some(("model", "depth-anything/Depth-Anything-V2-Base-hf".into())));
        assert_eq!(hf_repo("version.txt"), None);
        assert_eq!(and_list(&["Edge", "Chrome", "Brave"]), "Edge, Chrome and Brave");
        assert!(cleans_without_asking("claude-temp"));
        assert!(!cleans_without_asking("recycle-bin"));
        assert!(!cleans_without_asking("uv-cache"));
    }
}
