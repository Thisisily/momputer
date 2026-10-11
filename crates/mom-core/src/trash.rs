//! Taking out the trash: temp folders, caches and leftovers that pile up and never get emptied.
//!
//! Every spot says exactly what its files are, lists them, and says what cleaning does. Most of
//! it is disposable by definition (it regenerates, or was scratch space for a session that's
//! over), so it is deleted for real. Things a person might want back (old installers) go to the
//! Recycle Bin, and "review" spots ask the user to look first. Mom skips anything a running
//! process is using and anything touched recently.

mod bin;
mod browsers;
mod builds;
mod claude;
mod walk;

use crate::agents::AgentKind;
use crate::config::{Config, TrashConfig};
use crate::paths::Paths;
use crate::procs::ProcTable;
use crate::util::{human_age_long, human_bytes, mtime_secs, norm_path, now_secs, plural};
use walk::{remove_all, tally, tally_many, Tally};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Spot {
    pub id: String,
    pub label: String,
    /// One short line. `what` and `effect` say more.
    pub detail: String,
    /// "agents", "dev", "windows" or "personal".
    pub group: &'static str,
    /// What these files are, in plain English.
    pub what: String,
    /// Exactly what the clean button does, with the numbers.
    pub effect: String,
    /// "delete" (gone for good), "recycle" (to the Recycle Bin), "tool" (runs a program that
    /// decides) or "empty-bin" (empties the Recycle Bin).
    pub action: &'static str,
    /// "safe" (regenerates, or scratch of finished work) or "review" (look at the list first;
    /// items can be unticked).
    pub safety: &'static str,
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
    pub new_bytes: u64,
    pub new_items: u64,
    /// The sizes are exact. False when a tool decides what to prune (reclaimable is then an
    /// upper bound) or a folder was too big to count in full.
    pub exact: bool,
    pub available: bool,
    pub measured_at: u64,
    /// A blocker or a tip ("Close Edge to clear its cache.").
    pub note: Option<String>,
    /// Biggest first: up to 40 that go, then up to 20 that stay.
    pub items: Vec<Item>,
}

/// One thing in a spot: a folder, a file, a browser profile's cache, a deleted file.
#[derive(Clone, Debug, Serialize)]
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
    /// Cleaning takes it (or, in a review spot, takes it unless the user unticks it).
    pub goes: bool,
    /// Stays only because it's recent, and nothing is using it, so the user may tick it to clean
    /// it anyway (`Pick.add`). In-use things, and fresh temp files, are never optional.
    pub optional: bool,
    /// Why it goes or stays ("Session ended 3 days ago", "Edge is open").
    pub reason: String,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanReport {
    pub id: String,
    pub freed: u64,
    /// Items deleted, recycled or emptied.
    pub deleted: u64,
    /// Items that couldn't be removed (in use) or no longer qualified.
    pub skipped: u64,
    pub message: String,
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
/// Projects untouched this long count as idle; their build folders can go.
const IDLE_PROJECT_DAYS: u64 = 30;
/// Installers in Downloads older than this can go.
const OLD_INSTALLER_DAYS: u64 = 30;
/// Crash reports younger than this stay, in case someone wants to send one.
const CRASH_MIN_AGE: u64 = 86_400;
/// Stop counting a single folder after this many entries (and say the size is a floor).
const ENTRY_CAP: u64 = 2_000_000;

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
               half-written copies of its state files and old sandbox logs.",
        group: "agents",
        action: "delete",
        safety: "safe",
        owner: Some(AgentKind::Codex),
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
        detail: "node_modules and build output in projects nobody has worked on for a month.",
        what: "Installed packages (node_modules) and build output (Rust's target, .next, .turbo and the like) inside \
               projects nobody has worked on for a month. Your code isn't in them; they're rebuilt from it.",
        group: "dev",
        action: "delete",
        safety: "review",
        owner: None,
    },
    Def {
        id: "docker",
        label: "Docker junk",
        detail: "Runs `docker system prune`: stopped containers, dangling images, build cache. Volumes are kept.",
        what: "Stopped containers, networks nothing uses, untagged images and build cache that Docker keeps around.",
        group: "dev",
        action: "tool",
        safety: "review",
        owner: None,
    },
    Def {
        id: "old-temp",
        label: "Old temp files",
        detail: "Anything in your Temp folder nobody has touched for a week.",
        what: "Files programs and installers left in your Temp folder and never cleaned up.",
        group: "windows",
        action: "delete",
        safety: "safe",
        owner: None,
    },
    Def {
        id: "browser-cache",
        label: "Browser caches",
        detail: "Saved copies of web pages, images and scripts. Browsers rebuild them as you browse.",
        what: "Copies of web pages, images and scripts your browsers saved so sites load faster. \
               Browsers rebuild them as you browse.",
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

pub fn spot_ids() -> Vec<&'static str> {
    DEFS.iter().map(|d| d.id).collect()
}

fn def(id: &str) -> Option<&'static Def> {
    DEFS.iter().find(|d| d.id == id)
}

/// Spots `mom trash clean all` (and anything else that cleans without asking) may take:
/// safe, and decided by mom rather than a tool.
pub fn cleans_without_asking(id: &str) -> bool {
    def(id).map(|d| d.safety == "safe" && d.action != "tool").unwrap_or(false)
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
    goes: bool,
    /// Kept only for being recent: the user can tick it.
    optional: bool,
    reason: String,
    complete: bool,
    /// A Claude transcript to read the session's title from, and the project name to put in
    /// front. Only read for items that get shown.
    title_from: Option<(PathBuf, String)>,
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
            age: now.saturating_sub(t.newest),
            goes: false,
            optional: false,
            reason: String::new(),
            complete: t.complete,
            title_from: None,
        }
    }

    fn verdict(mut self, goes: bool, reason: impl Into<String>) -> Cand {
        self.goes = goes;
        self.reason = reason.into();
        self
    }

    /// Stays for now because it's recent, but the user can tick it.
    fn kept_while_new(mut self, reason: impl Into<String>) -> Cand {
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
            goes: self.goes,
            optional: self.optional,
            reason: self.reason,
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
    if ctx.in_use(&c.path) {
        c.verdict(false, IN_USE)
    } else if c.age < min_age {
        let r = too_fresh(c.age, min_age);
        c.kept_while_new(r)
    } else {
        let r = format!("Untouched for {}", ago(c.age));
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

fn find(id: &str, ctx: &TrashCtx) -> Found {
    let agent_age = ctx.cfg.agent_temp_min_age_hours * 3600;
    let la = &ctx.paths.local_appdata;
    match id {
        "claude-temp" => claude::find(ctx, agent_age),
        "codex-temp" => {
            let codex = ctx.paths.codex_dir();
            let mut f = Found::at(codex.clone());
            f.available = codex.is_dir();
            let mut list: Vec<(PathBuf, String)> = Vec::new();
            let mut min_ages: Vec<u64> = Vec::new();
            let mut consider = |p: PathBuf, name: String, min_age: u64| {
                list.push((p, name));
                min_ages.push(min_age);
            };
            for p in entries(&ctx.paths.temp) {
                let n = name_of(&p).to_ascii_lowercase();
                if n.starts_with("codex-index-") {
                    consider(p, "Codex search index".into(), agent_age);
                } else if n.starts_with("codex-clipboard-") {
                    consider(p, "Codex clipboard image".into(), agent_age);
                }
            }
            for p in entries(&codex.join(".tmp")) {
                let n = format!("Codex temp: {}", name_of(&p));
                consider(p, n, agent_age);
            }
            for p in entries(&codex) {
                let n = name_of(&p);
                if p.is_file() && n.contains(".tmp-") {
                    let base = n.split(".tmp-").next().unwrap_or("").trim_start_matches('.');
                    consider(p, format!("Half-written copy of {base}"), agent_age);
                }
            }
            for p in entries(&codex.join(".sandbox")) {
                let n = name_of(&p);
                if n.starts_with("sandbox.") && n.ends_with(".log") {
                    consider(p, format!("Sandbox log {n}"), 14 * 86_400);
                }
            }
            for (c, min_age) in cands(list, ctx.now).into_iter().zip(min_ages) {
                f.cands.push(judge(c, ctx, min_age));
            }
            f.available = f.available || !f.cands.is_empty();
            f
        }
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
            f.cands = cands(list, ctx.now).into_iter().map(|c| judge(c, ctx, agent_age)).collect();
            f
        }
        "old-temp" => {
            let min = ctx.cfg.temp_min_age_days * 86_400;
            let mut f = Found::at(ctx.paths.temp.clone());
            let list: Vec<(PathBuf, String)> = entries(&ctx.paths.temp)
                .into_iter()
                .map(|p| {
                    let n = name_of(&p);
                    (p, n)
                })
                // these are counted by their own spots
                .filter(|(_, n)| !n.eq_ignore_ascii_case("claude") && !is_codex_temp(n) && !is_render_leftover(n))
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
            let list = entries(&root).into_iter().map(|p| {
                let n = npx_name(&p);
                (p, n)
            });
            for c in cands(list.collect(), ctx.now) {
                let c = if ctx.in_use(&c.path) {
                    c.verdict(false, IN_USE)
                } else if c.age < 7 * 86_400 {
                    let r = format!("Used {} ago (kept for a week)", ago(c.age));
                    c.kept_while_new(r)
                } else {
                    let r = format!("Not used for {}", ago(c.age));
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
                f.cands.push(if rev >= latest {
                    c.verdict(false, "Newest version, kept")
                } else if ctx.in_use(&c.path) {
                    c.verdict(false, IN_USE)
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
                let c = if ctx.in_use(&c.path) {
                    c.verdict(false, IN_USE)
                } else {
                    let r = format!("Downloaded {} ago", ago(c.age));
                    c.verdict(true, r)
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
            f.exact = false; // df counts every unused image; prune only takes the untagged ones
            let rows = crate::containers::docker_df().unwrap_or_default();
            let mut total = 0;
            let mut reclaim = 0;
            for d in rows {
                total += d.size;
                let volumes = d.kind.eq_ignore_ascii_case("Local Volumes");
                let name = match d.kind.to_ascii_lowercase().as_str() {
                    "images" => "Unused images",
                    "containers" => "Stopped containers",
                    "local volumes" => "Volumes",
                    "build cache" => "Build cache",
                    _ => d.kind.as_str(),
                }
                .to_string();
                let path = PathBuf::from(format!("docker:{}", d.kind.to_ascii_lowercase().replace(' ', "-")));
                let t = Tally::of(if volumes { d.size } else { d.reclaimable }, d.count);
                let mut c = Cand::new(path, name, &t, ctx.now);
                c.age = 0;
                c.sub = Some(plural(d.count, "item"));
                if volumes {
                    f.cands.push(c.verdict(false, "Volumes hold data; prune leaves them alone"));
                } else {
                    reclaim += d.reclaimable;
                    if d.reclaimable > 0 {
                        f.cands.push(c.verdict(true, "Nothing running uses it"));
                    }
                }
            }
            f.tool = Some((total, reclaim));
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
                c.sub = Some(if c.files > 1 { format!("{kind}, {}", plural(c.files, "file")) } else { kind.to_string() });
                let c = if c.age < CRASH_MIN_AGE {
                    let r = format!("Crashed {} ago (kept for a day)", ago(c.age));
                    c.kept_while_new(r)
                } else {
                    let r = format!("Crashed {} ago", ago(c.age));
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
                c.sub = Some(installer_kind(&c.name).to_string());
                let c = if ctx.in_use(&c.path) {
                    c.verdict(false, "Running right now")
                } else if c.age < min {
                    let r = format!("Downloaded {} ago (kept for {OLD_INSTALLER_DAYS} days)", ago(c.age));
                    c.kept_while_new(r)
                } else {
                    let r = format!("Downloaded {} ago", ago(c.age));
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

fn by_size(a: &Cand, b: &Cand) -> std::cmp::Ordering {
    b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name))
}

fn spot_from(def: &Def, f: Found, ctx: &TrashCtx) -> Spot {
    let (mut goes, mut stays): (Vec<Cand>, Vec<Cand>) = f.cands.into_iter().partition(|c| c.goes);
    goes.sort_by(by_size);
    stays.sort_by(by_size);
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
    let items = goes.into_iter().take(40).chain(stays.into_iter().take(20)).map(Cand::into_item).collect();
    Spot {
        id: def.id.into(),
        label: def.label.into(),
        detail: def.detail.into(),
        group: def.group,
        what: def.what.split_whitespace().collect::<Vec<_>>().join(" "),
        effect,
        action: def.action,
        safety: def.safety,
        path: f.root.map(|r| r.to_string_lossy().to_string()),
        owner: def.owner,
        total_bytes: total,
        reclaimable_bytes: reclaimable,
        reclaimable_items: n_go,
        kept_bytes: total.saturating_sub(reclaimable),
        kept_items: n_stay,
        new_bytes,
        new_items,
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
    let hours = human_age_long(ctx.cfg.agent_temp_min_age_hours * 3600);
    let forever = "permanently (they don't go to the Recycle Bin)";
    let nothing = |why: &str| format!("Nothing to clean right now: {why}");
    match id {
        "claude-temp" if n == 0 => nothing(&format!(
            "every scratch folder belongs to a session that's still running or was used in the last {hours}."
        )),
        "claude-temp" => format!(
            "Deletes {} from Claude Code sessions that have ended, {forever}. Folders of sessions that are still \
             running, or changed in the last {hours}, are left alone. Your projects and Claude's chat history aren't touched.",
            count_size(n, "scratch folder", "scratch folders", bytes)
        ),
        "codex-temp" if n == 0 => nothing(&format!("everything Codex left is either in use or from the last {hours}.")),
        "codex-temp" => format!(
            "Deletes {} that Codex left behind, {forever}: old search indexes, pasted clipboard images, half-written \
             state files and sandbox logs older than two weeks. Codex's threads, settings and memories aren't touched; \
             anything changed in the last {hours} stays.",
            count_size(n, "temp file", "temp files and folders", bytes)
        ),
        "render-temp" if n == 0 => nothing(&format!("no render folders older than {hours}.")),
        "render-temp" => format!(
            "Deletes {} from your Temp folder, {forever}. They're copies made while rendering; your projects and \
             finished videos aren't touched. Anything used in the last {hours} stays.",
            count_size(n, "render folder", "render folders", bytes)
        ),
        "old-temp" => {
            let days = human_age_long(ctx.cfg.temp_min_age_days * 86_400);
            if n == 0 {
                nothing(&format!("everything in Temp was used in the last {days} or is in use."))
            } else {
                format!(
                    "Deletes {} from your Temp folder that nobody has touched in {days}, {forever}. Anything newer, or in \
                     use by a running program, stays. Programs make new temp files when they need them.",
                    count_size(n, "item", "items", bytes)
                )
            }
        }
        "npx-cache" if n == 0 => nothing("every npx download was used in the last week."),
        "npx-cache" => format!(
            "Deletes {} that npx downloaded and nobody has run in a week, {forever}. If something needs one again, \
             npx downloads it again (a few seconds).",
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
        "docker" if !ctx.docker_running => "Docker isn't running, so there's nothing to prune right now. When it is, this runs \
             `docker system prune`: stopped containers, networks nothing uses, untagged images and build cache go; running \
             containers, images in use and volumes stay."
            .to_string(),
        "docker" => format!(
            "Runs `docker system prune` (up to {}): removes stopped containers, networks nothing uses, untagged images \
             and build cache. Running containers, images in use and volumes (where databases keep their data) aren't \
             touched. Stopped containers are gone for good, with anything saved inside them.",
            human_bytes(bytes)
        ),
        "playwright-old" if n == 0 => nothing("only the newest build of each browser is installed."),
        "playwright-old" => format!(
            "Deletes {} that Playwright downloaded, {forever}. The newest build of each browser stays, so tests and \
             browser tools keep working; `npx playwright install` brings an old one back if a project needs it.",
            count_size(n, "older browser build", "older browser builds", bytes)
        ),
        "huggingface-cache" if n == 0 => nothing("no downloaded models."),
        "huggingface-cache" => format!(
            "Deletes {} {forever}. Models can be very large and slow to download again, so untick any you still use. \
             Your code, and models you saved somewhere else, aren't touched.",
            count_size(n, "downloaded model", "downloaded models and datasets", bytes)
        ),
        "old-build-folders" if n == 0 => {
            if stays.is_empty() {
                nothing("no build folders found in your projects.")
            } else {
                nothing(&format!("every project with build folders was worked on in the last {IDLE_PROJECT_DAYS} days or has something running."))
            }
        }
        "old-build-folders" => {
            let projects: HashSet<&Path> = goes.iter().filter_map(|c| c.path.parent()).collect();
            format!(
                "Deletes {} from {} nobody has worked on for {IDLE_PROJECT_DAYS} days, {forever}. Your code isn't touched: \
                 `npm install` (or pnpm, yarn, bun) and `cargo build` bring them back. Projects you worked on recently, \
                 or that have something running, are left alone.",
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
                    "Deletes the saved web data of {}, {forever}. Pages load a bit slower the first time you visit them \
                     again. Logins, history, bookmarks, passwords and open tabs aren't touched.",
                    count_size(n, "browser profile", "browser profiles", bytes)
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
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Pick {
    /// Whether what would go starts ticked. "Untick all" makes it false.
    pub goes: bool,
    /// Whether what's kept only for being new (`Item.optional`) starts ticked ("Tick all").
    pub new: bool,
    pub ticked: Vec<String>,
    pub unticked: Vec<String>,
}

impl Default for Pick {
    fn default() -> Pick {
        Pick { goes: true, new: false, ticked: vec![], unticked: vec![] }
    }
}

impl Pick {
    fn is_default(&self) -> bool {
        self.goes && !self.new && self.ticked.is_empty() && self.unticked.is_empty()
    }
}

/// Cleans a spot: what qualifies right now, as ticked in `pick` (by default, everything that
/// goes). In-use things never go. Tool spots ignore `pick`. The Recycle Bin is emptied with
/// Windows' own call unless `pick` changes the list, and then its items are deleted one by one.
pub fn clean(id: &str, ctx: &TrashCtx, pick: Option<&Pick>) -> CleanReport {
    let mut r = CleanReport { id: id.to_string(), ..Default::default() };
    let Some(def) = def(id) else {
        r.message = format!("Mom doesn't know a spot called {id:?}.");
        return r;
    };
    let pick = pick.filter(|p| !p.is_default());
    match def.action {
        "tool" => return run_tool(id, ctx, r),
        "empty-bin" if pick.is_none() => return bin::empty(ctx, r),
        _ => {}
    }
    let f = find(id, ctx);
    let chosen = choose(&f, pick);
    if let Some(p) = pick {
        // Ticked, but can't go now (in use since the user looked, or already gone).
        let taken: HashSet<String> = chosen.iter().map(|c| norm_path(&c.path.to_string_lossy())).collect();
        r.skipped += p.ticked.iter().filter(|t| !taken.contains(&norm_path(t))).count() as u64;
    }
    let gone: Vec<bool> = if def.action == "recycle" {
        chosen.iter().map(|c| c.targets.iter().all(|t| !t.exists() || ::trash::delete(t).is_ok())).collect()
    } else {
        // Everything at once (much faster than one by one), then sorted back out per item.
        let targets: Vec<PathBuf> = chosen.iter().flat_map(|c| c.targets.iter().cloned()).collect();
        let mut done = remove_all(&targets).into_iter();
        chosen.iter().map(|c| c.targets.iter().map(|_| done.next().unwrap_or(false)).fold(true, |a, b| a & b)).collect()
    };
    for (c, ok) in chosen.iter().zip(gone) {
        if ok {
            r.freed += c.bytes;
            r.deleted += 1;
        } else {
            // Something in it was in use; count what did go.
            let left: u64 = tally_many(&c.targets, ENTRY_CAP, u64::MAX).iter().map(|t| t.bytes).sum();
            r.freed += c.bytes.saturating_sub(left);
            r.skipped += 1;
        }
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
    r.message = cleaned_message(def, &r);
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

fn cleaned_message(def: &Def, r: &CleanReport) -> String {
    let mut msg = if r.deleted == 0 {
        "Already spotless.".to_string()
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
        msg.push_str(&format!(" {} stayed: in use, or changed since mom looked.", plural(r.skipped, "item")));
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
        "docker" if !ctx.docker_running => Err("Docker isn't running.".into()),
        "docker" => crate::containers::docker_prune().map(|_| ()).map_err(|e| e.to_string()),
        _ => Err(format!("mom doesn't know how to clean {id}")),
    };
    match result {
        Ok(()) => {
            r.freed = before.saturating_sub(after_total(ctx));
            r.deleted = u64::from(r.freed > 0);
            r.message = match (id, r.freed) {
                (_, 0) => "Ran it. Nothing needed pruning; already spotless.".to_string(),
                ("docker", f) => format!("Docker pruned {}. Volumes are untouched.", human_bytes(f)),
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
    std::fs::metadata(path).map(|m| now.saturating_sub(mtime_secs(&m))).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TrashConfig;

    pub(super) fn touch_old(path: &Path, days: u64) {
        touch_secs(path, days * 86_400);
    }

    pub(super) fn touch_secs(path: &Path, secs: u64) {
        let f = std::fs::File::options().write(true).open(path).unwrap();
        let t = std::time::SystemTime::now() - Duration::from_secs(secs);
        f.set_modified(t).unwrap();
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
        }
    }

    pub(super) fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("mom-trash-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    fn item<'s>(spot: &'s Spot, name_part: &str) -> &'s Item {
        spot.items.iter().find(|i| i.name.contains(name_part)).unwrap_or_else(|| panic!("no item like {name_part:?} in {:#?}", spot.items))
    }

    #[test]
    fn claude_scratch_of_dead_sessions_goes_live_sessions_stay() {
        let root = fixture("claude");
        let paths = Paths::under(&root);
        let claude = paths.temp.join("claude").join("C--Workspace-storefront");
        let dead = "0191f20b-ae85-40af-b308-93c34777b8cb";
        let live = "e9c3412f-ef20-4ba5-a0c7-2d71609fde82";
        let fresh = "11111111-2222-3333-4444-555555555555";
        write(&claude.join(dead).join("scratchpad").join("shot.png"), 5000, 3);
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
        touch_old(&transcript, 3);
        // Codex: a stale half-written state file and an old index folder in Temp.
        write(&paths.codex_dir().join("..codex-global-state.json.tmp-1790-abc"), 1000, 5);
        write(&paths.codex_dir().join(".codex-global-state.json"), 1000, 5);
        write(&paths.temp.join("codex-index-1234").join("idx.bin"), 2000, 5);
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
        assert_eq!((spot.group, spot.action, spot.safety), ("agents", "delete", "safe"));
        assert!(spot.effect.starts_with("Deletes 1 scratch folder (5 KB) from Claude Code sessions that have ended, permanently"), "{}", spot.effect);

        // Every item says why it goes or stays.
        let gone = item(&spot, "Fix the login bug");
        assert!(gone.goes);
        assert!(gone.name.starts_with("storefront · "), "{}", gone.name);
        assert_eq!(gone.sub.as_deref(), Some("1 file"));
        assert!(gone.reason.starts_with("Session ended 3 days ago"), "{}", gone.reason);
        let running = item(&spot, "Session e9c3412f");
        assert!(!running.goes);
        assert_eq!(running.reason, "Session is still running");
        let new = item(&spot, "Session 11111111");
        assert!(!new.goes);
        assert!(new.reason.contains("(kept for 24 hours)"), "{}", new.reason);
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
        assert_eq!((spot.action, spot.safety, spot.group), ("recycle", "review", "personal"));
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
