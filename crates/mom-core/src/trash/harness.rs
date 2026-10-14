//! Every agent tool mom knows: where it lives, what it leaves lying around (logs, temp files,
//! caches, snapshots, old builds), and where it keeps its conversations.
//!
//! Only the folders listed here are looked in, so a tool's settings, sign-ins, memory, skills
//! and databases are never touched. Leftovers are deleted once nobody has touched them for a
//! while; conversations are moved to another drive with a link left behind (see archive.rs),
//! and only ones that are a folder each (or a day each), so a folder an agent still writes new
//! chats into never moves.
//!
//! Teach mom another tool: add it to `agents()`.

use super::{ago, entries, name_of, tally_many, too_fresh, Cand, Found, TrashCtx, ENTRY_CAP, IN_USE};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Which clean-up a tool's leftovers show up in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Spot {
    Claude,
    Codex,
    Agents,
    Apps,
    Worktrees,
    Shaders,
    Games,
    Dev,
    Windows,
    AdminApps,
}

impl Spot {
    fn of(id: &str) -> Option<Spot> {
        Some(match id {
            "claude-leftovers" => Spot::Claude,
            "codex-temp" => Spot::Codex,
            "agent-leftovers" => Spot::Agents,
            "app-leftovers" => Spot::Apps,
            "cursor-worktrees" => Spot::Worktrees,
            "shader-cache" => Spot::Shaders,
            "game-junk" => Spot::Games,
            "dev-leftovers" => Spot::Dev,
            "windows-junk" => Spot::Windows,
            "admin-leftovers" => Spot::AdminApps,
            _ => return None,
        })
    }
}

/// One place a tool leaves things. Entries of `dir` (or `depth` levels further down) that `pick`
/// accepts are one item each.
struct Source {
    dir: PathBuf,
    /// What one of them is ("Log") and what a bunch are ("logs").
    kind: (&'static str, &'static str),
    pick: fn(&str) -> bool,
    /// What to call an entry, from its path under `dir`.
    name: fn(&Path, &[String]) -> String,
    depth: usize,
    files_only: bool,
    /// The newest this many stay (the build in use, say).
    keep_newest: usize,
    /// Each entry is a session's, named by its id; a running session's stays.
    sessions: bool,
    /// Only entries with nothing at all in them count (a folder an older version left behind).
    empty_only: bool,
    /// Everything found here is one thing to decide about, so one item (forty found.NNN folders).
    together: bool,
    /// The app makes this again by itself the next time it starts (a cache of its own code), so
    /// the newest one is kept because deleting it frees nothing, and mom says so.
    remade: bool,
    /// Visible for accounting, but only the owning tool can safely clean it.
    manual_cleanup: Option<&'static str>,
}

fn src(dir: PathBuf, kind: (&'static str, &'static str), pick: fn(&str) -> bool) -> Source {
    Source { dir, kind, pick, name: |_, rel| rel.join(" / "), depth: 0, files_only: false, keep_newest: 0, sessions: false, empty_only: false, together: false, remade: false, manual_cleanup: None }
}

impl Source {
    fn named(mut self, name: fn(&Path, &[String]) -> String) -> Source {
        self.name = name;
        self
    }
    fn depth(mut self, depth: usize) -> Source {
        self.depth = depth;
        self
    }
    fn files(mut self) -> Source {
        self.files_only = true;
        self
    }
    fn keep_newest(mut self, n: usize) -> Source {
        self.keep_newest = n;
        self
    }
    fn sessions(mut self) -> Source {
        self.sessions = true;
        self
    }
    fn empty_only(mut self) -> Source {
        self.empty_only = true;
        self
    }
    fn together(mut self) -> Source {
        self.together = true;
        self
    }
    fn remade(mut self) -> Source {
        self.remade = true;
        self
    }
    fn manual_cleanup(mut self, reason: &'static str) -> Source {
        self.manual_cleanup = Some(reason);
        self
    }
}

fn any(_: &str) -> bool {
    true
}

/// "D:\$WINDOWS.~TMP": something at the top of a drive, named with its drive.
fn at_the_top(drive: &Path, rel: &[String]) -> String {
    let d = drive.to_string_lossy();
    format!("{d}{}{}", if d.ends_with(['\\', '/']) { "" } else { "\\" }, rel.join(" / "))
}

/// Working folders Windows setup and the update assistant make at the top of a drive.
/// Their age alone does not establish that an update has finished using them.
const SETUP_FOLDERS: &[&str] = &["$WINDOWS.~TMP", "$WINDOWS.~BT", "$WINDOWS.~WS", "$GetCurrent", "$SysReset", "Windows10Upgrade", "WUDownloadCache"];

pub(super) fn setup_folder(n: &str) -> bool {
    SETUP_FOLDERS.iter().any(|f| f.eq_ignore_ascii_case(n))
}

/// found.000, found.001...: where a disk check (chkdsk) puts the pieces of files it recovered.
fn check_folder(n: &str) -> bool {
    n.len() == 9 && n.get(..6).is_some_and(|p| p.eq_ignore_ascii_case("found.")) && n.bytes().skip(6).all(|b| b.is_ascii_digit())
}

fn session(_: &Path, rel: &[String]) -> String {
    format!("Session {}", rel[0].chars().take(8).collect::<String>())
}

/// "5 October 2026" from YYYY/MM/DD folders.
fn dated(rel: &[String]) -> String {
    const MONTHS: [&str; 12] =
        ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    let month = |m: &str| m.parse::<usize>().ok().and_then(|m| MONTHS.get(m.wrapping_sub(1))).copied();
    match rel {
        [y, m] => month(m).map(|m| format!("{m} {y}")),
        [y, m, d] => month(m).zip(d.parse::<u32>().ok()).map(|(m, d)| format!("{d} {m} {y}")),
        _ => None,
    }
    .unwrap_or_else(|| rel.join(" / "))
}

/// Where a tool keeps conversations, one folder each (or one per day). Folders under `root`, up
/// to `depth` levels down and at least `min_depth`, move as a whole once nobody has opened
/// anything in them for a while: a whole month of Codex chats if all of it is old, else its old days.
struct Chats {
    root: PathBuf,
    depth: usize,
    min_depth: usize,
    /// Folders never moved (memory the agent reads every session, say).
    skip: &'static [&'static str],
    name: fn(&TrashCtx, &Path, &[String]) -> String,
}

fn chats(root: PathBuf, depth: usize) -> Chats {
    Chats { root, depth, min_depth: 1, skip: &[], name: |_, _, rel| rel.join(" / ") }
}

impl Chats {
    fn named(mut self, name: fn(&TrashCtx, &Path, &[String]) -> String) -> Chats {
        self.name = name;
        self
    }
    fn min_depth(mut self, d: usize) -> Chats {
        self.min_depth = d;
        self
    }
    fn skip(mut self, skip: &'static [&'static str]) -> Chats {
        self.skip = skip;
        self
    }
}

/// A tool: where it lives (any of `homes` existing means it's here), what it leaves, where it chats.
struct Agent {
    name: &'static str,
    spot: Spot,
    homes: Vec<PathBuf>,
    junk: Vec<Source>,
    chats: Vec<Chats>,
}

fn agent(name: &'static str, spot: Spot, homes: Vec<PathBuf>) -> Agent {
    Agent { name, spot, homes, junk: vec![], chats: vec![] }
}

impl Agent {
    fn junk(mut self, junk: Vec<Source>) -> Agent {
        self.junk = junk;
        self
    }
    fn chats(mut self, chats: Vec<Chats>) -> Agent {
        self.chats = chats;
        self
    }
}

/// Real folders in `dir` (links are already-moved conversations or shared skills: never followed).
fn subdirs(dir: &Path, skip: &[&str]) -> Vec<PathBuf> {
    entries(dir)
        .into_iter()
        .filter(|d| std::fs::symlink_metadata(d).map(|m| m.is_dir()).unwrap_or(false))
        .filter(|d| {
            let n = name_of(d);
            !skip.contains(&n.as_str()) && !n.contains(".momputer-")
        })
        .collect()
}

/// Microsoft Store apps keep their files in %LOCALAPPDATA%\Packages\<name>_<id>\LocalCache.
fn store_app(ctx: &TrashCtx, family: &str) -> Vec<PathBuf> {
    subdirs(&ctx.paths.local_appdata.join("Packages"), &[])
        .into_iter()
        .filter(|p| name_of(p).starts_with(family))
        .map(|p| p.join("LocalCache"))
        .collect()
}

/// Folders an env var moves, or the usual place.
fn env_or(ctx: &TrashCtx, key: &str, usual: PathBuf) -> PathBuf {
    ctx.paths.agent_env(key).map(Path::to_path_buf).unwrap_or(usual)
}

/// VS Code and the editors built from it: (name, folder in %APPDATA%, extensions folder in home).
const EDITORS: &[(&str, &str, &str)] = &[
    ("VS Code", "Code", ".vscode"),
    ("VS Code Insiders", "Code - Insiders", ".vscode-insiders"),
    ("VSCodium", "VSCodium", ".vscode-oss"),
    ("Cursor", "Cursor", ".cursor"),
    ("Windsurf", "Windsurf", ".windsurf"),
    ("Kiro", "Kiro", ".kiro"),
    ("Trae", "Trae", ".trae"),
    ("Trae CN", "Trae CN", ".trae-cn"),
    ("Qoder", "Qoder", ".qoder"),
    ("Antigravity", "Antigravity", ".antigravity"),
];

/// Agent extensions that keep one folder per task: (name, extension id).
const TASK_EXTENSIONS: &[(&str, &str)] =
    &[("Cline", "saoudrizwan.claude-dev"), ("Roo Code", "rooveterinaryinc.roo-cline"), ("Kilo Code", "kilocode.kilo-code")];

/// Tools built like Claude Code, with the same leftovers: (name, folder in home).
const CLAUDE_LIKE: &[(&str, &str)] = &[
    ("Neovate", ".neovate"),
    ("Kode", ".kode"),
    ("CodeBuddy", ".codebuddy"),
    ("Qoder CLI", ".qoder"),
    ("Command Code", ".commandcode"),
    ("iFlow", ".iflow"),
];

fn agents(ctx: &TrashCtx) -> Vec<Agent> {
    let p = ctx.paths;
    let home = &p.home;
    let temp = &p.temp;
    let (roaming, local) = (&p.appdata, &p.local_appdata);
    let share = env_or(ctx, "XDG_DATA_HOME", home.join(".local").join("share"));
    let claude = p.claude_dir();
    let codex = p.codex_dir();
    let mut list = Vec::new();

    // ---- Claude Code, and the Claude app (Store or classic install).
    let nodejs_cache = local.join("claude-cli-nodejs").join("Cache");
    let mut claude_junk = vec![
        src(claude.join("debug"), ("Debug log", "debug logs"), |n| n != "latest"),
        src(claude.join("shell-snapshots"), ("Shell snapshot", "shell snapshots"), any),
        src(claude.join("file-history"), ("Rewind backups", "rewind backups"), any).named(session).sessions(),
        src(claude.join("session-env"), ("Session settings", "session settings"), any).named(session).sessions(),
        src(claude.join("paste-cache"), ("Pasted text", "pasted text"), any),
        src(home.clone(), ("Half-written settings", "half-written copies of .claude.json"), |n| n.starts_with(".claude.json.tmp.")).files(),
        src(temp.clone(), ("Temp folder", "temp folders"), |n| n.to_ascii_lowercase().starts_with("claude-")),
    ];
    for project in subdirs(&nodejs_cache, &[]) {
        claude_junk.push(
            src(project, ("MCP server logs", "MCP server logs"), |n| n.starts_with("mcp-logs-"))
                .named(|dir, rel| format!("{} · {} logs", super::claude::project_name(&name_of(dir)), rel[0].trim_start_matches("mcp-logs-"))),
        );
    }
    list.push(
        agent("Claude Code", Spot::Claude, vec![claude.clone()]).junk(claude_junk).chats(vec![chats(claude.join("projects"), 2)
            .min_depth(2)
            .skip(&["memory"])
            .named(claude_chat)]),
    );
    let mut app_homes: Vec<PathBuf> = store_app(ctx, "Claude_").into_iter().map(|c| c.join("Roaming").join("Claude")).collect();
    app_homes.push(roaming.join("Claude"));
    let mut app_junk = vec![src(local.join("Claude").join("logs"), ("Old log", "old logs"), |n| {
        n.trim_end_matches(".log").ends_with(|c: char| c.is_ascii_digit())
    })
    .files()];
    for h in &app_homes {
        app_junk.extend([
            src(h.join("logs"), ("Old log", "old logs"), any).files(),
            src(h.join("Crashpad").join("reports"), ("Crash report", "crash reports"), any),
            src(h.join("claude-code"), ("Built-in Claude Code version", "older built-in Claude Code versions"), any).keep_newest(1),
        ]);
    }
    list.push(agent("Claude app", Spot::Claude, app_homes).junk(app_junk));

    // ---- Codex, and the Codex app.
    list.push(
        agent("Codex", Spot::Codex, vec![codex.clone()])
            .junk(vec![
                src(temp.clone(), ("Search index", "old search indexes"), |n| n.to_ascii_lowercase().starts_with("codex-index-"))
                    .named(|_, _| "Codex search index".into()),
                src(temp.clone(), ("Clipboard image", "pasted clipboard images"), |n| n.to_ascii_lowercase().starts_with("codex-clipboard-"))
                    .named(|_, _| "Codex clipboard image".into()),
                src(codex.join(".tmp"), ("Temp folder", "temp folders"), any).named(|_, rel| format!("Codex temp: {}", rel[0])),
                src(codex.join("tmp"), ("Temp folder", "temp folders"), any).named(|_, rel| format!("Codex temp: {}", rel[0])),
                src(codex.join("log"), ("Log", "logs"), any).named(|_, rel| format!("Codex log {}", rel[0])),
                src(codex.clone(), ("Half-written file", "half-written state files"), |n| n.contains(".tmp-")).files().named(|_, rel| {
                    format!("Half-written copy of {}", rel[0].split(".tmp-").next().unwrap_or("").trim_start_matches('.'))
                }),
                src(codex.join(".sandbox"), ("Sandbox log", "sandbox logs"), |n| n.starts_with("sandbox.") && n.ends_with(".log"))
                    .named(|_, rel| format!("Sandbox log {}", rel[0])),
                src(codex.clone(), ("Sandbox log", "sandbox logs"), |n| n.starts_with("sandbox") && n.ends_with(".log"))
                    .files()
                    .named(|_, rel| format!("Sandbox log {}", rel[0])),
                src(codex.join(".sandbox-bin"), ("Old command runner", "old command runners"), |n| n.starts_with("codex-command-runner")).keep_newest(1),
                src(home.join(".cache").join("codex-runtimes"), ("Replaced runtime", "replaced runtimes"), |n| n.contains(".previous-")),
            ])
            .chats(vec![
                chats(codex.join("sessions"), 3).named(|_, _, rel| format!("chats from {}", dated(rel))),
                chats(codex.join("generated_images"), 1).named(|_, _, _| "images from one chat".into()),
            ]),
    );
    let codex_app: Vec<PathBuf> = store_app(ctx, "OpenAI.Codex_");
    list.push(
        agent("Codex app", Spot::Codex, codex_app.clone())
            .junk(codex_app.iter().map(|c| src(c.join("Local").join("Codex").join("Logs"), ("Logs", "app logs"), any).depth(2).named(|_, rel| format!("Codex app logs, {}", dated(rel)))).collect()),
    );

    // ---- Other agents.
    let opencode = share.join("opencode");
    list.push(agent("OpenCode", Spot::Agents, vec![opencode.clone(), home.join(".config").join("opencode")]).junk(vec![
        src(opencode.join("log"), ("Log", "logs"), any),
        src(opencode.join("snapshot"), ("Undo snapshots", "undo snapshots"), any).named(|_, rel| format!("undo snapshots {}", rel[0].chars().take(8).collect::<String>())),
        src(opencode.join("tool-output"), ("Saved tool output", "saved tool output"), any),
    ]));
    list.push(agent("OpenCode app", Spot::Agents, vec![roaming.join("ai.opencode.desktop")]).junk(vec![src(
        roaming.join("ai.opencode.desktop").join("logs"),
        ("Log", "logs"),
        any,
    )]));
    let kilo = share.join("kilo");
    list.push(agent("Kilo Code", Spot::Agents, vec![kilo.clone()]).junk(vec![
        src(kilo.join("log"), ("Log", "logs"), any),
        src(kilo.join("snapshot"), ("Undo snapshots", "undo snapshots"), any),
        src(kilo.join("tool-output"), ("Saved tool output", "saved tool output"), any),
        src(temp.join("kilo"), ("Temp file", "temp files"), any),
    ]));
    let claw = env_or(ctx, "OPENCLAW_STATE_DIR", home.join(".openclaw"));
    list.push(
        agent("OpenClaw", Spot::Agents, vec![claw.clone()])
            .junk(vec![
                src(claw.join("logs"), ("Log", "logs"), any),
                src(claw.clone(), ("Old config backup", "old config backups"), |n| n.starts_with("openclaw.json.bak")).files(),
                src(claw.clone(), ("Half-written file", "half-written update checks"), |n| n.starts_with("update-check.json.") && n.ends_with(".tmp")).files(),
                src(temp.join("openclaw"), ("Log", "logs"), any),
                src(temp.clone(), ("Temp folder", "temp folders"), |n| n.to_ascii_lowercase().starts_with("openclaw-")),
                // OpenClaw was Clawdbot, then Moltbot. Folders from then with nothing in them can go.
                src(home.clone(), ("Empty folder from an old name", "empty folders from its old names"), |n| n == ".clawdbot" || n == ".moltbot").empty_only(),
            ]),
    );
    let mut hermes_homes: Vec<PathBuf> = ctx.paths.agent_env("HERMES_HOME").map(Path::to_path_buf).into_iter().collect();
    for h in [home.join(".hermes"), local.join("hermes")] {
        if !hermes_homes.contains(&h) {
            hermes_homes.push(h);
        }
    }
    let mut hermes_junk = Vec::new();
    for h in &hermes_homes {
        let cache = h.join("cache");
        hermes_junk.extend([
            src(h.join("logs"), ("Log", "logs"), any),
            src(h.join("pastes"), ("Pasted text", "pasted text"), any),
            src(h.join("checkpoints"), ("Rollback checkpoint", "rollback checkpoints"), any),
            src(h.join("sessions"), ("Failed request dump", "failed request dumps"), |n| n.starts_with("request_dump_")).files(),
            src(cache.join("images"), ("Cached image", "cached images"), any),
            src(cache.join("audio"), ("Cached audio", "cached audio"), any),
            src(cache.join("documents"), ("Cached document", "cached documents"), any),
            src(cache.join("screenshots"), ("Browser screenshot", "browser screenshots"), any),
            src(h.join("image_cache"), ("Cached image", "cached images"), any),
            src(h.join("audio_cache"), ("Cached audio", "cached audio"), any),
            src(h.join("document_cache"), ("Cached document", "cached documents"), any),
            src(h.join("browser_screenshots"), ("Browser screenshot", "browser screenshots"), any),
        ]);
    }
    list.push(agent("Hermes", Spot::Agents, hermes_homes).junk(hermes_junk));
    let pi = env_or(ctx, "PI_CODING_AGENT_DIR", home.join(".pi").join("agent"));
    list.push(agent("Pi", Spot::Agents, vec![pi.clone()]).junk(vec![
        src(temp.clone(), ("Long command output", "long command output it saved"), |n| n.starts_with("pi-bash-")),
        src(temp.join("pi-extensions"), ("Trial extension install", "trial extension installs"), any),
        src(temp.clone(), ("Slack bot output", "Slack bot output"), |n| n.starts_with("mom-bash-")),
        src(pi, ("Debug log", "debug logs"), |n| n == "pi-debug.log").files(),
    ]));
    let copilot = home.join(".copilot");
    list.push(
        agent("Copilot CLI", Spot::Agents, vec![copilot.clone()])
            .junk(vec![src(copilot.join("logs"), ("Log", "logs"), any)])
            .chats(vec![chats(copilot.join("session-state"), 1)]),
    );
    let gemini = home.join(".gemini");
    let mut gemini_junk = Vec::new();
    for project in subdirs(&gemini.join("tmp"), &["bin"]) {
        gemini_junk.push(src(project.join("logs"), ("Log", "logs"), any));
        gemini_junk.push(src(project.join("tool-outputs"), ("Saved tool output", "saved tool output"), any));
    }
    list.push(agent("Gemini CLI", Spot::Agents, vec![gemini.join("tmp")]).junk(gemini_junk));
    let antigravity = gemini.join("antigravity");
    list.push(agent("Antigravity", Spot::Agents, vec![antigravity.clone()]).chats(vec![
        chats(antigravity.join("brain"), 1).named(|_, _, rel| format!("plans and screenshots {}", rel[0].chars().take(8).collect::<String>())),
        chats(antigravity.join("browser_recordings"), 1).named(|_, _, rel| format!("browser recording {}", rel[0].chars().take(8).collect::<String>())),
    ]));
    let qwen = home.join(".qwen");
    let mut qwen_junk = vec![src(qwen.join("debug"), ("Debug log", "debug logs"), any)];
    for project in subdirs(&qwen.join("tmp"), &[]) {
        qwen_junk.push(src(project.join("tool-results"), ("Saved tool output", "saved tool output"), any));
    }
    list.push(agent("Qwen Code", Spot::Agents, vec![qwen.join("tmp"), qwen.join("projects")]).junk(qwen_junk));
    let goose = roaming.join("Block").join("goose").join("data");
    list.push(agent("Goose", Spot::Agents, vec![goose.clone()]).junk(vec![
        src(goose.join("logs").join("cli"), ("Logs", "logs"), any),
        src(goose.join("logs").join("server"), ("Logs", "logs"), any),
    ]));
    let cline = home.join(".cline").join("data");
    list.push(
        agent("Cline CLI", Spot::Agents, vec![cline.clone()])
            .junk(vec![
                src(cline.join("logs"), ("Log", "logs"), any),
                src(cline.join("checkpoint-scratch"), ("Checkpoint scratch", "checkpoint scratch"), any),
            ])
            .chats(vec![chats(cline.join("sessions"), 1)]),
    );
    let cont = home.join(".continue");
    list.push(agent("Continue", Spot::Agents, vec![cont.join("sessions"), cont.join("index")]).junk(vec![
        src(cont.join("logs"), ("Log", "logs"), any),
        src(cont.clone(), ("Codebase index", "codebase indexes (rebuilt when needed)"), |n| n == "index" || n == "dev_data"),
    ]));
    let kimi = home.join(".kimi");
    list.push(
        agent("Kimi CLI", Spot::Agents, vec![kimi.join("sessions")])
            .junk(vec![src(kimi.join("logs"), ("Log", "logs"), any)])
            .chats(vec![chats(kimi.join("sessions"), 2).min_depth(2)]),
    );
    let openhands = home.join(".openhands");
    list.push(agent("OpenHands", Spot::Agents, vec![openhands.join("conversations")]).chats(vec![chats(openhands.join("conversations"), 1)]));
    let vibe = home.join(".vibe");
    list.push(
        agent("Mistral Vibe", Spot::Agents, vec![vibe.join("logs")])
            .junk(vec![src(vibe.join("logs"), ("Log", "logs"), |n| n.starts_with("vibe.log")).files()])
            .chats(vec![chats(vibe.join("logs").join("session"), 1)]),
    );
    let junie = home.join(".junie");
    list.push(
        agent("Junie", Spot::Agents, vec![junie.join("sessions"), junie.join("logs")])
            .junk(vec![src(junie.join("logs"), ("Log", "logs"), any)])
            .chats(vec![chats(junie.join("sessions"), 1)]),
    );
    for mux in [home.join(".mux"), home.join(".xum")] {
        list.push(agent("Mux", Spot::Agents, vec![mux.join("sessions"), mux.join("logs")]).junk(vec![
            src(mux.join("logs"), ("Log", "logs"), any),
            src(mux.join("debug_obj"), ("Debug output", "debug output"), any),
            src(mux.join("perf"), ("Performance trace", "performance traces"), any),
        ]));
    }
    list.push(agent("Crush", Spot::Agents, vec![local.join("crush")]).junk(vec![src(local.join("crush").join("cache"), ("Cache", "caches"), any)]));
    list.push(agent("Factory Droid", Spot::Agents, vec![home.join(".factory").join("sessions")]).junk(vec![src(
        home.join(".factory").join("logs"),
        ("Log", "logs"),
        any,
    )]));
    list.push(agent("Zed", Spot::Agents, vec![local.join("Zed")]).junk(vec![
        src(local.join("Zed").join("logs"), ("Old log", "old logs"), |n| n != "Zed.log"),
        src(local.join("Zed").join("hang_traces"), ("Hang trace", "hang traces"), any),
    ]));
    let warp = local.join("warp").join("Warp").join("data");
    list.push(agent("Warp", Spot::Agents, vec![warp.clone()]).junk(vec![src(warp.join("logs"), ("Log", "logs"), any)]));
    let codeium = home.join(".codeium");
    list.push(
        agent("Windsurf", Spot::Agents, vec![codeium.clone()])
            .junk(vec![
                src(codeium.join("database"), ("Code search index", "code search indexes (rebuilt when needed)"), any),
                src(codeium.join("windsurf").join("database"), ("Code search index", "code search indexes (rebuilt when needed)"), any),
            ])
            .chats(vec![chats(codeium.join("windsurf").join("brain"), 1)]),
    );
    list.push(agent("Kiro CLI", Spot::Agents, vec![home.join(".kiro").join("sessions")]).junk(vec![
        src(temp.join("kiro-log").join("logs"), ("Log", "logs"), any),
        src(temp.join("amazon-q").join("logs"), ("Log", "logs"), any),
    ]));
    list.push(agent("Aider", Spot::Agents, vec![home.join(".aider")]).junk(vec![src(home.join(".aider").join("caches"), ("Cache", "caches"), any)]));
    let grok = roaming.join("Grok Bot");
    list.push(agent("Grok Bot", Spot::Agents, vec![home.join(".grokbot"), grok.clone()]).junk(vec![
        src(home.join(".grokbot"), ("Log", "logs"), |n| n.ends_with(".log")).files(),
        src(grok.join("Crashpad").join("reports"), ("Crash report", "crash reports"), any),
    ]));
    let paperclip = env_or(ctx, "PAPERCLIP_HOME", home.join(".paperclip"));
    list.push(
        agent("Paperclip", Spot::Agents, vec![paperclip.clone()])
            .junk(subdirs(&paperclip.join("instances"), &[]).into_iter().map(|i| src(i.join("logs"), ("Log", "logs"), any)).collect()),
    );
    list.push(agent("Bionic", Spot::Agents, vec![roaming.join("Bionic")]).junk(vec![src(roaming.join("Bionic").join("logs"), ("Log", "logs"), any)]));
    for (name, dir) in CLAUDE_LIKE {
        let h = home.join(dir);
        list.push(agent(name, Spot::Agents, vec![h.join("projects"), h.join("logs"), h.join("file-history")]).junk(vec![
            src(h.join("file-history"), ("Rewind backups", "rewind backups"), any).named(session).sessions(),
            src(h.join("logs"), ("Log", "logs"), any),
            src(h.join("diagnostics"), ("Diagnostics", "diagnostics (memory dumps)"), any),
        ]));
    }

    // ---- VS Code and its family, and the agent extensions inside them.
    for (name, folder, ext) in EDITORS {
        let r = roaming.join(folder);
        let storage = r.join("User").join("globalStorage");
        list.push(agent(name, Spot::Apps, vec![r.clone()]).junk(vec![
            src(r.join("CachedData"), ("Old build's cache", "old builds' caches"), any).keep_newest(1).remade(),
            src(r.join("CachedExtensionVSIXs"), ("Downloaded extension package", "downloaded extension packages"), any),
            src(r.join("Crashpad").join("reports"), ("Crash report", "crash reports"), any),
            src(r.join("logs"), ("Logs", "logs"), any),
            src(home.join(ext).join("extensions"), ("Half-unpacked extension", "half-unpacked extensions"), |n| n.starts_with('.')),
        ]));
        for (tool, id) in TASK_EXTENSIONS {
            let e = storage.join(id);
            list.push(
                agent(tool, Spot::Agents, vec![e.clone()])
                    .junk(vec![src(e.join("checkpoints"), ("Checkpoints", "checkpoints of old workspaces"), any)])
                    .chats(vec![chats(e.join("tasks"), 1).named(|_, _, rel| format!("task {}", rel[0].chars().take(13).collect::<String>()))]),
            );
        }
    }
    // Cursor runs agents side by side, each in its own copy of the project:
    // ~/.cursor/worktrees/<project>/<worktree>.
    let worktrees = home.join(".cursor").join("worktrees");
    list.push(agent("Cursor", Spot::Worktrees, vec![worktrees.clone()]).junk(vec![src(worktrees, ("Worktree", "worktrees"), |_| true)
        .depth(1)
        .named(|_, rel| rel.join(" · "))]));
    // Cursor's agent keeps one folder per chat in ~/.cursor/projects/<project>/agent-transcripts.
    let cursor_projects = home.join(".cursor").join("projects");
    list.push(agent("Cursor", Spot::Agents, vec![cursor_projects.clone()]).chats(
        subdirs(&cursor_projects, &[])
            .into_iter()
            .map(|p| chats(p.join("agent-transcripts"), 1).named(|_, root, rel| {
                let project = root.parent().map(name_of).unwrap_or_default();
                format!("{} · chat {}", project.rsplit('-').next().unwrap_or(&project), rel[0].chars().take(8).collect::<String>())
            }))
            .collect(),
    ));
    // Older versions apps keep next to the one they run.
    let chrome = local.join("Google").join("Chrome").join("User Data");
    list.push(agent("Chrome", Spot::Apps, vec![chrome.join("Snapshots")]).junk(vec![src(
        chrome.join("Snapshots"),
        ("Snapshot of an older version", "snapshots of older versions"),
        any,
    )
    .keep_newest(1)
    .named(|_, rel| format!("version {} snapshot", rel[0]))]));
    // Apps installed with Squirrel (Slack, Discord, GitHub Desktop...) keep each version they
    // updated from, as app-<version>, and its package.
    for app in subdirs(local, &[]).into_iter().filter(|d| d.join("Update.exe").is_file() && !super::browsers::hands_off(&name_of(d))) {
        list.push(agent("Squirrel apps", Spot::Apps, vec![app.clone()]).junk(vec![
            src(app.clone(), ("Older version", "older versions"), |n| n.starts_with("app-"))
                .keep_newest(1)
                .named(|dir, rel| format!("{} {}", name_of(dir), rel[0].trim_start_matches("app-"))),
            src(app.join("packages"), ("Older package", "older update packages"), |n| n.ends_with(".nupkg"))
                .files()
                .keep_newest(1),
        ]));
    }
    // Adobe keeps plug-in data per major version of each app; the newest is the one in use.
    let uxp = roaming.join("Adobe").join("UXP").join("PluginsStorage");
    for app in subdirs(&uxp, &[]) {
        list.push(agent("Adobe", Spot::Apps, vec![app.clone()]).junk(vec![src(app, ("Older version's plug-in data", "older versions' plug-in data"), any)
            .keep_newest(1)
            .named(|dir, rel| format!("{} {} plug-in data", name_of(dir), rel[0]))]));
    }
    // Epic's webcache and overlay BrowserCache are complete browser profiles, including sign-ins.
    // Only their disposable subcaches are collected by browser-cache.
    list.push(agent("PowerToys", Spot::Apps, vec![local.join("Microsoft").join("PowerToys")]).junk(vec![src(
        local.join("Microsoft").join("PowerToys").join("Updates"),
        ("Downloaded update", "downloaded update installers"),
        any,
    )]));

    // ---- Graphics drivers' shader caches: per file. Age is last modification, not last use.
    let low = local.parent().map(|a| a.join("LocalLow")).unwrap_or_default();
    let mut nvidia = Vec::new();
    for base in [local.join("NVIDIA"), low.join("NVIDIA"), low.join("NVIDIA").join("PerDriverVersion")] {
        for cache in ["DXCache", "GLCache", "OptixCache"] {
            nvidia.push(src(base.join(cache), ("Shader", "shaders"), any).named(|dir, rel| format!("{} {}", name_of(dir), rel.join(" / "))));
        }
    }
    nvidia.push(src(roaming.join("NVIDIA").join("ComputeCache"), ("Compute shader", "compute shaders"), any));
    list.push(agent("NVIDIA", Spot::Shaders, vec![local.join("NVIDIA"), low.join("NVIDIA")]).junk(nvidia));
    let amd = local.join("AMD");
    list.push(agent("AMD", Spot::Shaders, vec![amd.clone()]).junk(
        ["DxCache", "DxcCache", "VkCache", "GLCache", "OglCache"].iter().map(|c| src(amd.join(c), ("Shader", "shaders"), any)).collect(),
    ));
    list.push(agent("Intel", Spot::Shaders, vec![local.join("Intel").join("ShaderCache")]).junk(vec![src(
        local.join("Intel").join("ShaderCache"),
        ("Shader", "shaders"),
        any,
    )]));
    list.push(agent("DirectX", Spot::Shaders, vec![local.join("D3DSCache")]).junk(vec![src(local.join("D3DSCache"), ("Shader cache", "shader caches"), any)]));

    // ---- Games: logs and crash reports, never saves.
    list.push(agent("Unity games", Spot::Games, vec![low.clone()]).junk(vec![src(low.clone(), ("Log", "logs"), |n| {
        n == "Player.log" || n == "Player-prev.log"
    })
    .depth(2)
    .files()
    .named(|_, rel| format!("{} · {}", rel[1], rel[2]))]));
    let mut unreal = Vec::new();
    for game in subdirs(local, &[]) {
        let saved = game.join("Saved");
        if saved.join("Logs").is_dir() || saved.join("Crashes").is_dir() {
            unreal.push(src(saved.join("Logs"), ("Log", "logs"), any).named(|dir, rel| format!("{} · {}", game_of(dir), rel[0])));
            unreal.push(src(saved.join("Crashes"), ("Crash report", "crash reports"), any).named(|dir, rel| format!("{} · crash {}", game_of(dir), rel[0])));
        }
    }
    list.push(agent("Unreal games", Spot::Games, vec![local.clone()]).junk(unreal));
    let godot = roaming.join("Godot").join("app_userdata");
    list.push(agent("Godot games", Spot::Games, vec![godot.clone()]).junk(
        subdirs(&godot, &[])
            .into_iter()
            .map(|g| src(g, ("Shader cache", "shader caches"), |n| n == "shader_cache" || n == "vulkan").named(|dir, rel| format!("{} · {}", name_of(dir), rel[0])))
            .collect(),
    ));

    // ---- Developer tools' caches without a clean-up of their own.
    let cache = home.join(".cache");
    list.push(agent("Developer tools", Spot::Dev, vec![home.clone()]).junk(vec![
        src(cache.clone(), ("GitHub Actions runner cache", "act's caches"), |n| n == "act" || n == "actcache").named(|_, rel| format!("act · {}", rel[0])),
        src(cache.join("firebase").join("emulators"), ("Firebase emulator", "Firebase emulators"), any).named(|_, rel| format!("Firebase · {}", rel[0])),
        src(local.join("Pub"), ("Dart package cache", "Dart and Flutter package caches"), |n| n == "Cache").named(|_, _| "Dart · package cache".into()),
        src(local.clone(), ("Dart analysis cache", "Dart analysis caches"), |n| n == ".dartServer").named(|_, _| "Dart · analysis server cache".into()),
        src(local.join("Cypress").join("Cache"), ("Cypress version", "older Cypress versions"), any).keep_newest(1).named(|_, rel| format!("Cypress {}", rel[0])),
        src(local.clone(), ("Playwright for Go", "Playwright for Go downloads"), |n| n == "ms-playwright-go").named(|_, _| "Playwright for Go".into()),
        src(local.join("node-gyp").join("Cache"), ("Node headers", "Node headers node-gyp downloaded"), any).named(|_, rel| format!("node-gyp · Node {}", rel[0])),
        src(roaming.clone(), ("Old npm cache", "an old npm cache nothing uses"), |n| n == "npm-cache").named(|_, _| "npm · old cache in AppData\\Roaming".into()),
        src(home.clone(), ("Old npm cache", "an old npm cache nothing uses"), |n| n == "npm-cache").named(|_, _| "npm · old cache in your home folder".into()),
        // Zero Install protects implementations with deny ACLs and may share files through
        // hard links. Its own cache manager must handle removal; ordinary deletion cannot.
        src(local.join("0install.net"), ("Zero Install download", "Zero Install downloads"), |n| n == "implementations")
            .named(|_, _| "Zero Install · downloaded apps".into())
            .manual_cleanup("Protected by Zero Install; open its cache manager to remove downloaded apps"),
        src(local.clone(), ("Installer temp files", "installer temp files"), |n| n == "SquirrelTemp").named(|_, _| "Squirrel installer temp files".into()),
        src(local.join("Microsoft"), ("Editor extension cache", "editor extension caches"), |n| n == "vscode-cpptools" || n == "TypeScript")
            .named(|_, rel| format!("VS Code · {} cache", rel[0])),
        src(roaming.join("UnityHub"), ("Downloaded installer", "Unity Hub downloads"), |n| n == "downloads").named(|_, _| "Unity Hub · downloads".into()),
        src(local.join("JxBrowser"), ("Unpacked browser", "unpacked JxBrowser runtimes"), any).named(|_, rel| format!("JxBrowser {}", rel[0])),
        src(local.join("cache"), ("Installer cache", "installer caches"), |n| n.starts_with("qt-unified")).named(|_, _| "Qt installer cache".into()),
        src(home.join(".gradle").join("wrapper").join("dists"), ("Gradle version", "Gradle versions projects downloaded"), any).named(|_, rel| format!("Gradle · {}", rel[0])),
    ]));

    // ---- Things only an administrator can delete (mom asks Windows for permission to clean them).
    let (win, data, root) = (&p.windows, &p.program_data, &p.system_drive);
    let wer = data.join("Microsoft").join("Windows").join("WER");
    list.push(agent("Windows Update", Spot::Windows, vec![win.join("SoftwareDistribution").join("Download")]).junk(vec![src(
        win.join("SoftwareDistribution").join("Download"),
        ("Downloaded update", "downloaded update files"),
        any,
    )
    .named(|_, rel| format!("update files {}", rel[0].chars().take(12).collect::<String>()))]));
    list.push(agent("Windows", Spot::Windows, vec![wer.clone()]).junk(vec![
        src(wer.join("ReportArchive"), ("Error report", "error reports"), any),
        src(wer.join("ReportQueue"), ("Error report", "error reports"), any),
    ]));
    list.push(agent("Installers", Spot::Windows, vec![root.join("AMD"), root.join("adobeTemp")]).junk(vec![src(
        root.clone(),
        ("Installer leftovers", "folders installers left at the root of the drive"),
        |n| n == "AMD" || n == "adobeTemp" || n.ends_with("-install-tmp"),
    )
    .named(at_the_top)]));
    // What Windows itself leaves at the top of a drive. Setup and the update assistant put their
    // working folders on whichever drive has room, and a disk check leaves what it recovered on
    // the drive it checked: so every drive built into the PC is looked at, not only C:. Only
    // these exact names, only at the very top, and a drive's other folders are never opened.
    for drive in std::iter::once(root).chain(&p.other_drives) {
        list.push(agent("Windows setup", Spot::Windows, SETUP_FOLDERS.iter().map(|n| drive.join(n)).collect()).junk(vec![src(
            drive.clone(),
            ("Setup leftovers", "folders Windows setup and the update assistant left behind"),
            setup_folder,
        )
        .named(at_the_top)]));
        list.push(agent("Disk check", Spot::Windows, vec![drive.join("found.000")]).junk(vec![src(
            drive.clone(),
            ("Recovered fragments", "pieces of files a disk check recovered and nobody claimed"),
            check_folder,
        )
        .named(at_the_top)
        .together()]));
    }
    let nvidia = data.join("NVIDIA Corporation");
    list.push(agent("NVIDIA", Spot::AdminApps, vec![nvidia.clone()]).junk(vec![
        src(nvidia.join("NVIDIA app").join("UpdateFramework").join("ota-artifacts").join("grd"), ("Driver installer cache", "cached driver installation files"), any)
            .keep_newest(1)
            .named(|_, rel| {
                if rel[0].eq_ignore_ascii_case("post-processing") {
                    "extracted driver installers".into()
                } else {
                    format!("driver installer download {}", rel[0])
                }
            }),
        src(nvidia.join("Downloader"), ("Download cache entry", "installer downloads and download metadata"), any)
            .keep_newest(1)
            .named(|_, rel| format!("download cache {}", rel[0])),
    ]));
    let epic = p.program_files_x86.join("Epic Games").join("Launcher").join("VaultCache");
    list.push(agent("Epic Games", Spot::AdminApps, vec![epic.clone()]).junk(vec![src(epic, ("Downloaded sample", "downloaded samples (the Launcher gets them again if asked)"), any)]));
    let glass = data.join("GlassWire");
    list.push(agent("GlassWire", Spot::AdminApps, vec![glass.clone()]).junk(vec![
        src(glass.clone(), ("Settings and traffic history backup", "settings and traffic history backups"), |n| n.contains(".backup_"))
            .named(|_, rel| format!("settings and traffic history backup {}", rel[0])),
        src(glass.join("service-full").join("log"), ("Log", "logs"), any),
    ]));
    let razer = data.join("Razer");
    list.push(agent("Razer", Spot::AdminApps, vec![razer.clone()]).junk(vec![
        src(razer.join("RazerCortex").join("Log"), ("Crash dump", "crash dumps"), |n| n.to_ascii_lowercase().ends_with(".dmp")).files(),
        src(razer.join("Razer Central").join("Logs"), ("Log", "logs"), any),
    ]));
    list.push(agent("Driver Booster", Spot::AdminApps, vec![data.join("IObit")]).junk(vec![src(
        data.join("IObit").join("Driver Booster").join("Download"),
        ("Downloaded driver", "downloaded drivers"),
        any,
    )]));
    list.push(agent("Debuggers", Spot::AdminApps, vec![data.join("Dbg")]).junk(vec![src(data.join("Dbg"), ("Symbol cache", "symbol caches"), any)]));
    list.push(agent("Logitech G HUB", Spot::AdminApps, vec![data.join("LGHUB")]).junk(vec![src(
        data.join("LGHUB").join("depots"),
        ("Older version", "older versions"),
        any,
    )
    .keep_newest(1)]));

    // Downloaded updates (apps built with electron-builder keep the last installer in <app>-updater).
    let updaters: Vec<PathBuf> = subdirs(local, &[]).into_iter().filter(|d| name_of(d).ends_with("-updater")).collect();
    list.push(
        agent("App updaters", Spot::Apps, updaters.clone()).junk(
            updaters
                .into_iter()
                .map(|u| src(u, ("Downloaded update", "downloaded update installers"), any).named(|dir, rel| format!("{} · {}", name_of(dir).trim_end_matches("-updater").trim_start_matches('@'), rel.join(" / "))))
                .collect(),
        ),
    );
    list
}

/// The game a ...\<Game>\Saved\Logs folder belongs to.
fn game_of(logs: &Path) -> String {
    logs.parent().and_then(Path::parent).map(name_of).unwrap_or_default()
}

fn claude_chat(ctx: &TrashCtx, _root: &Path, rel: &[String]) -> String {
    let project = super::claude::project_name(&rel[0]);
    match rel.get(1) {
        None => project,
        Some(id) => {
            let transcript = ctx.paths.claude_dir().join("projects").join(&rel[0]).join(format!("{id}.jsonl"));
            let what = super::claude::session_title(&transcript).unwrap_or_else(|| format!("session {}", id.chars().take(8).collect::<String>()));
            format!("{project} · {what}")
        }
    }
}

/// Every agent tool's and app's folders, so other clean-ups leave them to these. (Not the
/// shader, game and developer-tool entries: they stand for whole areas like LocalLow.)
pub(super) fn homes(ctx: &TrashCtx) -> Vec<PathBuf> {
    agents(ctx)
        .into_iter()
        .filter(|a| !matches!(a.spot, Spot::Shaders | Spot::Games | Spot::Dev | Spot::Windows | Spot::AdminApps))
        .flat_map(|a| a.homes.into_iter().chain(a.chats.into_iter().map(|c| c.root)))
        .filter(|h| h.exists())
        .collect()
}

// ------------------------------------------------------------------ leftovers

/// Entries `depth` levels under `dir`, with their path under it.
fn walk(dir: &Path, depth: usize, files_only: bool) -> Vec<(PathBuf, Vec<String>)> {
    let mut level: Vec<(PathBuf, Vec<String>)> = vec![(dir.to_path_buf(), vec![])];
    for _ in 0..depth {
        level = level.into_iter().flat_map(|(d, rel)| subdirs(&d, &[]).into_iter().map(move |s| {
            let mut r = rel.clone();
            r.push(name_of(&s));
            (s, r)
        })).collect();
    }
    level
        .into_iter()
        .flat_map(|(d, rel)| {
            entries(&d).into_iter().map(move |e| {
                let mut r = rel.clone();
                r.push(name_of(&e));
                (e, r)
            })
        })
        .filter(|(e, _)| !files_only || std::fs::symlink_metadata(e).map(|m| m.is_file()).unwrap_or(false))
        .collect()
}

pub(super) fn find(id: &str, ctx: &TrashCtx, min_age: u64) -> Found {
    let Some(spot) = Spot::of(id) else {
        return Found::none();
    };
    let all: Vec<Agent> = agents(ctx).into_iter().filter(|a| a.spot == spot).collect();
    let here: Vec<&Agent> = all.iter().filter(|a| a.homes.iter().any(|h| h.exists())).collect();
    let mut f = match (spot, here.first()) {
        (Spot::Claude, _) => Found::at(ctx.paths.claude_dir()),
        (Spot::Codex, _) => Found::at(ctx.paths.codex_dir()),
        (Spot::Worktrees, _) => Found::at(ctx.paths.home.join(".cursor").join("worktrees")),
        _ => Found::none(),
    };
    f.available = !here.is_empty();
    // One tool's clean-up names items plainly; one covering several says whose each is.
    let mixed = !matches!(spot, Spot::Codex | Spot::Worktrees);
    let mut list = Vec::new();
    let mut from: Vec<(&Agent, &Source)> = Vec::new();
    // What only an administrator may clean is never looked for through a link: one there could
    // point the clean-up anywhere (and the elevated clean checks again before deleting).
    let careful = matches!(spot, Spot::Windows | Spot::AdminApps);
    let linked = |d: &Path| d.ancestors().any(|a| std::fs::symlink_metadata(a).map(|m| m.is_symlink()).unwrap_or(false));
    for a in &all {
        for s in &a.junk {
            if careful && linked(&s.dir) {
                continue;
            }
            let mut picked: Vec<(PathBuf, Vec<String>)> =
                walk(&s.dir, s.depth, s.files_only).into_iter().filter(|(_, rel)| (s.pick)(rel.last().map(String::as_str).unwrap_or(""))).collect();
            if s.together {
                picked.sort();
            }
            for (path, rel) in picked {
                let name = (s.name)(&s.dir, &rel);
                list.push((path, if mixed { format!("{} · {name}", a.name) } else { name }));
                from.push((a, s));
            }
        }
    }
    let each: Vec<(Cand, &Agent, &Source)> = super::cands(list, ctx.now).into_iter().zip(from).map(|(c, (a, s))| (c.age_sensitive(), a, s)).collect();
    // What a source keeps together becomes one item: everything it found, named from the first to
    // the last, as new as its newest part. Cleaning it takes them all.
    let mut found: Vec<(Cand, &Agent, &Source)> = Vec::new();
    for (c, a, s) in each {
        match found.iter_mut().find(|(_, _, other)| s.together && std::ptr::eq(*other, s)) {
            Some((first, _, _)) => {
                first.bytes += c.bytes;
                first.files += c.files;
                first.links += c.links;
                first.age = first.age.min(c.age);
                first.age_known &= c.age_known;
                first.browser_data |= c.browser_data;
                first.complete &= c.complete;
                first.targets.extend(c.targets);
            }
            None => found.push((c, a, s)),
        }
    }
    for (c, _, _) in found.iter_mut().filter(|(c, _, _)| c.targets.len() > 1) {
        c.name = format!("{} to {} ({} folders)", c.name, name_of(&c.targets[c.targets.len() - 1]), c.targets.len());
        c.sub = None;
    }
    // "Keep the newest": the newest few of each such source stay, whatever their age. Newest by
    // version where the names have one (Photoshop 27 over 26, even if 26 was touched later), else
    // by when it last changed.
    found.sort_by(|(a, _, sa), (b, _, sb)| {
        sa.dir.cmp(&sb.dir)
            .then_with(|| version_in(&name_of(&b.path)).cmp(&version_in(&name_of(&a.path))))
            .then_with(|| b.age_known.cmp(&a.age_known))
            .then_with(|| if a.age_known && b.age_known { a.age.cmp(&b.age) } else { std::cmp::Ordering::Equal })
    });
    let mut kept_per: std::collections::HashMap<PathBuf, usize> = std::collections::HashMap::new();
    // What's installed, looked up only if something asks (most clean-ups never do).
    let looked_up = std::cell::OnceCell::new();
    let installed = || match &ctx.installed {
        Some(i) => i,
        None => looked_up.get_or_init(|| crate::installed::Installed::scan(ctx.paths)),
    };
    // Query Windows only if update-related files are actually present, and share the result
    // across setup folders on every drive and the downloaded update entries in this scan.
    let windows_update = std::cell::OnceCell::new();
    // Metadata is descriptive only. Bound extra I/O even for unexpectedly large caches.
    let mut shader_reads = 0;
    let mut shader_metadata_budget = 64 << 20;
    // Which programs are on this PC, and which program each NVIDIA app ID is: looked up once,
    // only for shader caches.
    let on_this_pc = std::cell::OnceCell::new();
    let programs = || on_this_pc.get_or_init(|| super::programs::Programs::find(ctx.paths));
    let mut nvidia_names: HashMap<PathBuf, HashMap<String, String>> = HashMap::new();
    for (mut c, a, s) in found {
        let mut gone = None;
        if s.empty_only && (c.files > 0 || c.links > 0 || c.bytes > 0) {
            continue; // not empty after all: it isn't a leftover
        }
        // The newest of each sort is usually what the app runs now (GitHub Desktop's app-3.6.6
        // folder is GitHub Desktop), so it's never offered, and it isn't called "older".
        let newest = !s.empty_only && s.keep_newest > 0 && {
            let n = kept_per.entry(s.dir.clone()).or_default();
            *n += 1;
            *n <= s.keep_newest
        };
        // Cache retention is a precaution, not a check of the installed driver version.
        let nvidia_cache = a.name == "NVIDIA" && a.spot == Spot::AdminApps;
        let label = if newest && nvidia_cache { "Kept cache entry" } else if newest { "Newest version" } else { s.kind.0 };
        c.sub = Some(match c.sub.take() {
            Some(inside) => format!("{label}, {inside}"),
            None => label.to_string(),
        });
        // A folder only an administrator may open measures as nothing. Say that, not "0 B".
        let shut = |t: &PathBuf| std::fs::symlink_metadata(t).map(|m| m.is_dir()).unwrap_or(false) && std::fs::read_dir(t).is_err();
        if careful && c.files == 0 && c.targets.iter().any(shut) {
            c.sub = Some(format!("{label}, size unknown: only an administrator can look inside"));
        }
        c.kind = Some(s.kind.1);
        c.tool = Some(a.name);
        if spot == Spot::Windows && matches!(a.name, "Windows Update" | "Windows setup") {
            if let Some(reason) = windows_update.get_or_init(|| super::windows_update_state(ctx)).reason() {
                // Pending or unknown state is a hard stop, including for recent folders the
                // generic age rule would otherwise let the user explicitly tick.
                c.optional = false;
                f.note.get_or_insert_with(|| reason.to_string());
                f.cands.push(c.verdict(false, reason));
                continue;
            }
        }
        if spot == Spot::Shaders {
            let owner = if c.links == 0 && shader_reads < 4096 && matches!(a.name, "NVIDIA" | "DirectX") {
                shader_reads += 1;
                super::shader_owner::describe(&c.path, &mut shader_metadata_budget)
            } else {
                None
            };
            // Some files don't say their program (older drivers', the biggest ones), but NVIDIA
            // names every file by its app's ID, and other files with that ID say it.
            let program = owner.as_ref().map(|o| o.name.clone()).or_else(|| {
                let id = (a.name == "NVIDIA" && c.links == 0).then(|| super::shader_owner::nvidia_app_id(&c.path)).flatten()?;
                let dir = c.path.parent()?.to_path_buf();
                let names = nvidia_names.entry(dir.clone()).or_insert_with(|| super::shader_owner::nvidia_names(&dir, &mut shader_metadata_budget));
                names.get(&id).cloned()
            });
            let tail = c.name.strip_prefix(&format!("{} · ", a.name)).unwrap_or(&c.name).to_string();
            let (name, detail) = match &program {
                Some(program) => {
                    let found = programs().judge(program);
                    let name = found.title.unwrap_or_else(|| program.clone());
                    if found.state == super::programs::State::Gone {
                        gone = Some(name.clone());
                    }
                    (name, found.detail.to_string())
                }
                None => ("App unknown".into(), "The driver didn't note which app".into()),
            };
            c.name = format!("{} · {name} · {tail}", a.name);
            c.sub = Some(match c.sub.take() {
                Some(sub) => format!("{detail} · {sub}"),
                None => detail,
            });
        }
        if let Some(reason) = s.manual_cleanup {
            f.note.get_or_insert_with(|| reason.to_string());
            f.cands.push(c.verdict(false, reason));
            continue;
        }
        if s.empty_only {
            f.cands.push(c.verdict(true, "Empty, from an older version"));
            continue;
        }
        if newest {
            let one_app = !MANY_APPS.contains(&a.name);
            let gone = one_app && c.age >= NEWEST_UNUSED && !installed().has(a.name);
            f.cands.push(if gone {
                c.kept_but_tickable(format!("{} isn't installed any more", a.name))
            } else if s.remade && one_app {
                c.verdict(false, format!("{} makes it again when it starts", a.name))
            } else {
                c.verdict(false, if nvidia_cache { "Kept as a precaution" } else { NEWEST })
            });
            continue;
        }
        c = judge(c, ctx, min_age, s.sessions);
        if spot == Spot::Shaders && c.goes {
            c.reason = format!("Last changed {} ago", ago(c.age));
        }
        f.cands.push(uninstalled(c, gone));
    }
    if spot == Spot::Shaders {
        let caches = super::steam_shaders::discover(ctx.paths);
        let name = |cache: &super::steam_shaders::Cache| match cache.named {
            false => programs().steam_name(cache.id).unwrap_or(&cache.name).to_string(),
            true => cache.name.clone(),
        };
        let paths = caches.iter().map(|cache| (cache.path.clone(), format!("Steam · {} · Shader cache", name(cache)))).collect();
        for (mut c, cache) in super::cands(paths, ctx.now).into_iter().zip(caches) {
            if c.files == 0 && c.links == 0 && c.complete {
                continue;
            }
            let gone = cache.uninstalled.then(|| name(&cache));
            c.sub = Some(match c.sub.take() {
                Some(sub) => format!("{} · {sub}", cache.detail),
                None => cache.detail,
            });
            c.kind = Some("shader caches");
            c.tool = Some("Steam");
            c = judge(c, ctx, min_age, false);
            // An installed game's cache stays: the game would only build it again. One whose
            // game Steam no longer has installed goes.
            if c.goes && gone.is_none() {
                let reason = format!("Last changed {} ago; select to clear", ago(c.age));
                c = c.kept_but_tickable(reason);
            }
            f.cands.push(uninstalled(c, gone));
        }
    }
    f.available = f.available || !f.cands.is_empty();
    f
}

/// The version in a name: "27", "app-4.51.180", "slack-4.51.180-full.nupkg", "webcache_4430",
/// "151.0.7922.174". A commit hash or a plain word has none.
fn version_in(name: &str) -> Option<Vec<u64>> {
    let nums = |v: &str| v.split('.').map(|x| x.parse::<u64>().ok()).collect::<Option<Vec<u64>>>();
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_digit()) {
        return nums(name);
    }
    // A dotted run of numbers anywhere: 4.51.180.
    let b = name.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() && (i == 0 || !b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'v') {
            let start = i;
            while i < b.len() && (b[i].is_ascii_digit() || (b[i] == b'.' && i + 1 < b.len() && b[i + 1].is_ascii_digit())) {
                i += 1;
            }
            let run = &name[start..i];
            if run.contains('.') {
                return nums(run);
            }
        }
        i += 1;
    }
    // A number after an underscore at the end: webcache_4430.
    name.rsplit_once('_').map(|(_, n)| n).filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())).and_then(nums)
}

/// What keeps the newest of each sort.
const NEWEST: &str = "The app uses this one";

/// The newest of each sort is locked because the app is using it. An app that isn't installed
/// any more uses nothing: once its newest has gone untouched this long as well, it can be ticked.
/// (Age alone says nothing: an app in use can leave its newest untouched for a year.)
const NEWEST_UNUSED: u64 = 180 * 86_400;

/// Entries that stand for many apps at once: there's no one app to ask after, so their newest
/// always stays locked. (A Squirrel app's newest folder is the app itself.)
const MANY_APPS: &[&str] = &["Squirrel apps", "Developer tools", "App updaters"];

/// Agent folders hold links to shared skills; one with a link inside is never a leftover.
const LINKED: &str = "Holds a link to another folder (a shared skill, say)";

/// A cache whose program isn't installed any more goes, however new, unless something still has
/// it open or links into it.
fn uninstalled(mut c: Cand, program: Option<String>) -> Cand {
    match program {
        Some(program) if c.goes || c.optional => {
            c.optional = false;
            c.verdict(true, format!("{program} isn't installed any more"))
        }
        _ => c,
    }
}

fn judge(c: Cand, ctx: &TrashCtx, min_age: u64, sessions: bool) -> Cand {
    let c = c.age_sensitive();
    if c.links > 0 {
        c.verdict(false, LINKED)
    } else if sessions && ctx.session_live(&name_of(&c.path)) {
        c.verdict(false, "Session is still running")
    } else if ctx.in_use(&c.path) {
        c.verdict(false, IN_USE)
    } else if c.age < min_age {
        let r = too_fresh(c.age, min_age);
        c.kept_but_tickable(r)
    } else {
        let r = format!("Last changed {} ago", ago(c.age));
        c.verdict(true, r)
    }
}

/// Things in %TEMP% these clean-ups own, so "Old temp files" doesn't count them twice. Built
/// once, then asked about every entry in Temp.
pub(super) struct TempOwners {
    picks: Vec<fn(&str) -> bool>,
    folders: Vec<String>,
}

impl TempOwners {
    pub(super) fn owns(&self, name: &str) -> bool {
        self.picks.iter().any(|p| p(name)) || self.folders.iter().any(|f| f.eq_ignore_ascii_case(name))
    }
}

pub(super) fn temp_owners(ctx: &TrashCtx) -> TempOwners {
    let temp = &ctx.paths.temp;
    let mut owners = TempOwners { picks: vec![], folders: vec![] };
    for s in agents(ctx).iter().flat_map(|a| &a.junk) {
        if &s.dir == temp {
            owners.picks.push(s.pick);
        } else if let Some(first) = s.dir.strip_prefix(temp).ok().and_then(|r| r.components().next()) {
            owners.folders.push(first.as_os_str().to_string_lossy().to_string());
        }
    }
    owners
}

/// What a clean-up preserves, and what removing its contents means.
pub(super) fn untouched(id: &str) -> &'static str {
    match id {
        "claude-leftovers" => "Your chats, projects, settings, plugins and memory aren't touched.",
        "app-leftovers" => "The builds they run now, your extensions, settings and unsaved files aren't touched.",
        "shader-cache" => "Games and apps keep working: a shader one needs is made again the first time it's used.",
        "game-junk" => "Saves and settings aren't touched.",
        "dev-leftovers" => "Your projects and tools aren't touched; anything needed is downloaded or built again.",
        "windows-junk" => "Update downloads and setup folders stay when an update is pending or Windows update status cannot be checked. Review other leftovers before deleting them.",
        "admin-leftovers" => "Installed drivers stay. Cached installers may need to be downloaded again for reinstall or rollback. Deleting backups discards the settings and history saved in them.",
        _ => "Their chats, sessions, settings, sign-ins, skills and memory aren't touched.",
    }
}

/// "logs, undo snapshots and pasted text", from what's going.
pub(super) fn kinds(goes: &[Cand]) -> String {
    and_list(goes.iter().filter_map(|c| c.kind))
}

/// "Codex, Claude Code and Pi", from what's going.
pub(super) fn tools(goes: &[Cand]) -> String {
    and_list(goes.iter().filter_map(|c| c.tool))
}

fn and_list<'a>(items: impl Iterator<Item = &'a str>) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for k in items {
        if !seen.contains(&k) {
            seen.push(k);
        }
    }
    match seen.len() {
        0 => String::new(),
        1 => seen[0].to_string(),
        n => format!("{} and {}", seen[..n - 1].join(", "), seen[n - 1]),
    }
}

// ------------------------------------------------------------------ conversations

/// Folders of old conversations, each to be moved as one.
pub(super) fn conversations(ctx: &TrashCtx, min_age: u64) -> Found {
    let all = agents(ctx);
    let stores: Vec<(&Agent, &Chats)> =
        all.iter().flat_map(|a| a.chats.iter().map(move |c| (a, c))).filter(|(_, c)| c.root.is_dir()).collect();
    let mut f = Found::at(ctx.paths.home.clone());
    f.available = !stores.is_empty();
    f.root = ctx.cfg.archive_dir.clone();
    for (a, s) in stores {
        // A whole folder goes if all of it is old; otherwise look one level down, until `depth`.
        let mut level: Vec<PathBuf> = subdirs(&s.root, s.skip);
        for d in 1..=s.depth {
            let tallies = tally_many(&level, ENTRY_CAP, u64::MAX);
            let mut next = Vec::new();
            for (dir, t) in level.into_iter().zip(tallies) {
                if t.files == 0 && t.links == 0 {
                    continue; // nothing in it worth a link
                }
                let age = ctx.now.saturating_sub(t.recency);
                // (A folder too big to count in full isn't known to be old.)
                if d >= s.min_depth && ((age >= min_age && t.complete) || d == s.depth) {
                    let rel: Vec<String> = dir
                        .strip_prefix(&s.root)
                        .map(|r| r.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect())
                        .unwrap_or_default();
                    let mut c = Cand::new(dir.clone(), format!("{} · {}", a.name, (s.name)(ctx, &s.root, &rel)), &t, ctx.now);
                    c.tool = Some(a.name);
                    f.cands.push(chat_verdict(c, ctx, min_age));
                } else if d < s.depth {
                    next.extend(subdirs(&dir, s.skip));
                }
            }
            level = next;
        }
    }
    f
}

fn chat_verdict(c: Cand, ctx: &TrashCtx, min_age: u64) -> Cand {
    let c = c.age_sensitive();
    if c.links > 0 {
        c.verdict(false, LINKED)
    } else if ctx.in_use(&c.path) {
        c.verdict(false, IN_USE)
    } else if c.age < 86_400 {
        // Today's might still be being written.
        let r = format!("Last changed {} ago", ago(c.age));
        c.verdict(false, r)
    } else if c.age < min_age {
        let r = format!("Last changed {} ago (kept here for {})", ago(c.age), super::human_age_long(min_age));
        c.kept_but_tickable(r)
    } else {
        let r = format!("Last changed {} ago", ago(c.age));
        c.verdict(true, r)
    }
}

#[cfg(test)]
mod tests {
    use crate::config::TrashConfig;
    use crate::paths::Paths;
    use crate::trash::tests::{ctx, fixture, write};
    use crate::trash::{clean, measure};

    #[test]
    fn each_tool_loses_only_its_old_leftovers() {
        let root = fixture("harness");
        let paths = Paths::under(&root);
        let (home, temp) = (paths.home.clone(), paths.temp.clone());
        // Hermes: an old log goes, a fresh one stays, sessions and memories are never looked at.
        write(&home.join(".hermes").join("logs").join("agent.log"), 400, 90);
        write(&home.join(".hermes").join("logs").join("gateway.log"), 100, 1);
        write(&home.join(".hermes").join("cache").join("images").join("a.png"), 300, 80);
        write(&home.join(".hermes").join("sessions").join("session_1.json"), 999, 300);
        write(&home.join(".hermes").join("sessions").join("request_dump_1.json"), 50, 300);
        write(&home.join(".hermes").join("memories").join("me.md"), 999, 300);
        // Pi: overflow output in Temp, never its sessions.
        write(&temp.join("pi-bash-0a1b2c3d.log"), 200, 70);
        write(&home.join(".pi").join("agent").join("sessions").join("x").join("s.jsonl"), 999, 300);
        // OpenCode: logs and old undo snapshots, never its database.
        let data = home.join(".local").join("share").join("opencode");
        write(&data.join("log").join("2026-01-01T000000.log"), 50, 100);
        write(&data.join("snapshot").join("965945dc5b8d").join("objects").join("pack"), 5000, 100);
        write(&data.join("opencode.db"), 999, 100);
        // OpenClaw: a log, a temp folder, an old backup, the empty folder of an old name, and one
        // that only holds a link to a shared skill (which must stay).
        write(&home.join(".openclaw").join("logs").join("commands.log"), 60, 70);
        write(&home.join(".openclaw").join("openclaw.json.bak.2"), 70, 70);
        write(&home.join(".openclaw").join("openclaw.json"), 999, 70);
        write(&temp.join("openclaw-img-x1").join("a.png"), 80, 70);
        std::fs::create_dir_all(home.join(".clawdbot")).unwrap();
        let skill = root.join("shared-skill");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::create_dir_all(home.join(".moltbot").join("skills")).unwrap();
        crate::trash::tests::link(&home.join(".moltbot").join("skills").join("find-skills"), &skill);
        // Claude Code: rewind backups of a running session stay; a half-written settings copy goes.
        write(&home.join(".claude").join("file-history").join("0191f20b-ae85").join("a.ts"), 500, 90);
        write(&home.join(".claude").join("file-history").join("e9c3412f-ef20").join("b.ts"), 500, 90);
        write(&home.join(".claude.json.tmp.1234.abcd"), 30, 90);
        write(&home.join(".claude").join("projects").join("p").join("s.jsonl"), 999, 300);
        // An editor: old builds go, the newest stays however old.
        let code = paths.appdata.join("Code");
        write(&code.join("CachedData").join("aaa").join("x.bin"), 700, 200);
        write(&code.join("CachedData").join("bbb").join("x.bin"), 800, 100);
        write(&home.join(".vscode").join("extensions").join(".8f2b1c").join("p.json"), 40, 100);
        write(&home.join(".vscode").join("extensions").join("ms-python.python-1").join("p.json"), 40, 100);

        let cfg = TrashConfig::default();
        let mut ctx = ctx(&paths, &cfg);
        ctx.live_sessions.insert("e9c3412f-ef20".into());

        let agents = measure("agent-leftovers", &ctx).unwrap();
        assert!(agents.available);
        let mut going: Vec<&str> = agents.items.iter().filter(|i| i.goes).map(|i| i.name.as_str()).collect();
        going.sort();
        assert_eq!(
            going,
            vec![
                "Hermes · a.png",
                "Hermes · agent.log",
                "Hermes · request_dump_1.json",
                "OpenClaw · .clawdbot",
                "OpenClaw · commands.log",
                "OpenClaw · openclaw-img-x1",
                "OpenClaw · openclaw.json.bak.2",
                "OpenCode · 2026-01-01T000000.log",
                "OpenCode · undo snapshots 965945dc",
                "Pi · pi-bash-0a1b2c3d.log",
            ]
        );
        assert!(!agents.items.iter().any(|i| i.name.contains(".moltbot")), "a folder holding a link isn't empty");
        assert!(agents.items.iter().any(|i| i.name == "Hermes · gateway.log" && i.optional), "today's log can be ticked");
        assert!(agents.effect.contains("Hermes, OpenClaw, OpenCode and Pi") || agents.effect.contains("67 days"), "{}", agents.effect);

        let claude = measure("claude-leftovers", &ctx).unwrap();
        assert_eq!((claude.reclaimable_items, claude.reclaimable_bytes), (2, 530));
        let running = claude.items.iter().find(|i| i.name == "Claude Code · Session e9c3412f").unwrap();
        assert_eq!((running.goes, running.optional), (false, false));

        let apps = measure("app-leftovers", &ctx).unwrap();
        let mut going: Vec<&str> = apps.items.iter().filter(|i| i.goes).map(|i| i.name.as_str()).collect();
        going.sort();
        assert_eq!(going, vec!["VS Code · .8f2b1c", "VS Code · aaa"]);
        // The newest is a cache the editor would only make again: it stays, and says so.
        let newest = apps.items.iter().find(|i| i.name == "VS Code · bbb").unwrap();
        assert_eq!((newest.goes, newest.optional, newest.reason.as_str()), (false, false, "VS Code makes it again when it starts"));
        assert_eq!(apps.items.iter().find(|i| i.name == "VS Code · aaa").unwrap().sub.as_deref(), Some("Old build's cache, 1 file"));
        // The newest build of an editor that was uninstalled over a year ago: kept, but it can be
        // ticked now. Of one that is still installed, never, however long it has sat untouched.
        write(&paths.appdata.join("Kiro").join("CachedData").join("only").join("x.bin"), 900, 400);
        let kiro = |ctx: &crate::trash::TrashCtx| measure("app-leftovers", ctx).unwrap().items.into_iter().find(|i| i.name == "Kiro · only").unwrap();
        let gone = kiro(&ctx);
        assert!(!gone.goes && gone.optional && gone.reason == "Kiro isn't installed any more", "{gone:?}");
        ctx.installed = Some(crate::installed::Installed::with(&["Kiro"]));
        let here = kiro(&ctx);
        assert_eq!((here.goes, here.optional, here.reason.as_str()), (false, false, "Kiro makes it again when it starts"));
        ctx.installed = Some(crate::installed::Installed::with(&[]));
        let _ = std::fs::remove_dir_all(paths.appdata.join("Kiro"));

        // Nothing here is in Old temp files twice.
        let old = measure("old-temp", &ctx).unwrap();
        assert_eq!(old.reclaimable_items + old.kept_items, 0, "{:#?}", old.items);

        for id in ["agent-leftovers", "claude-leftovers", "app-leftovers"] {
            clean(id, &ctx, None);
        }
        assert!(home.join(".hermes").join("logs").join("gateway.log").exists());
        assert!(!home.join(".hermes").join("logs").join("agent.log").exists());
        assert!(home.join(".hermes").join("sessions").join("session_1.json").exists());
        assert!(home.join(".hermes").join("memories").join("me.md").exists());
        assert!(home.join(".pi").join("agent").join("sessions").join("x").join("s.jsonl").exists());
        assert!(data.join("opencode.db").exists() && !data.join("snapshot").join("965945dc5b8d").exists());
        assert!(home.join(".openclaw").join("openclaw.json").exists() && !home.join(".clawdbot").exists());
        assert!(home.join(".moltbot").join("skills").join("find-skills").exists(), "the shared skill's link survives");
        assert!(home.join(".claude").join("file-history").join("e9c3412f-ef20").exists());
        assert!(home.join(".claude").join("projects").join("p").join("s.jsonl").exists());
        assert!(!home.join(".claude.json.tmp.1234.abcd").exists());
        assert!(code.join("CachedData").join("bbb").exists() && !code.join("CachedData").join("aaa").exists());
        assert!(home.join(".vscode").join("extensions").join("ms-python.python-1").exists());
        crate::trash::tests::unlink(&home.join(".moltbot").join("skills").join("find-skills"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn old_conversations_move_whole_months_or_single_days_and_leave_links() {
        let root = fixture("chats");
        let paths = Paths::under(&root);
        let sessions = paths.home.join(".codex").join("sessions").join("2026");
        // July is old; in August only the 3rd is; today's file is still being written.
        write(&sessions.join("07").join("01").join("rollout-a.jsonl"), 3000, 100);
        write(&sessions.join("07").join("02").join("rollout-b.jsonl"), 1000, 99);
        write(&sessions.join("08").join("03").join("rollout-c.jsonl"), 500, 70);
        write(&sessions.join("08").join("20").join("rollout-d.jsonl"), 200, 10);
        write(&sessions.join("08").join("21").join("rollout-e.jsonl"), 100, 0);
        // Claude: only a session's own folder moves, never a whole project (new chats go there)
        // and never memory.
        let projects = paths.home.join(".claude").join("projects");
        write(&projects.join("C--Workspace-old").join("s1.jsonl"), 700, 90);
        write(&projects.join("C--Workspace-busy").join("s2.jsonl"), 50, 0);
        write(&projects.join("C--Workspace-busy").join("0191f20b-ae85").join("tool-results").join("x.txt"), 900, 80);
        write(&projects.join("C--Workspace-busy").join("memory").join("MEMORY.md"), 40, 200);
        // OpenClaw keeps every agent's sessions in one folder it writes to; that never moves.
        write(&paths.home.join(".openclaw").join("agents").join("main").join("sessions").join("a.jsonl"), 300, 200);
        write(&paths.home.join(".openclaw").join("agents").join("main").join("agent").join("auth-profiles.json"), 30, 200);
        let archive = root.join("archive");
        std::fs::create_dir_all(&archive).unwrap();

        let mut cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        assert!(!crate::trash::cleans_without_asking("old-conversations"), "never moved without asking");
        let spot = measure("old-conversations", &ctx).unwrap();
        let mut going: Vec<&str> = spot.items.iter().filter(|i| i.goes).map(|i| i.name.as_str()).collect();
        going.sort();
        assert_eq!(going, vec!["Claude Code · busy · session 0191f20b", "Codex · chats from 3 August 2026", "Codex · chats from July 2026"]);
        let today = spot.items.iter().find(|i| i.name == "Codex · chats from 21 August 2026").unwrap();
        assert!(!today.goes && !today.optional, "today's stays put");
        assert!(spot.items.iter().find(|i| i.name == "Codex · chats from 20 August 2026").unwrap().optional);
        assert!(!spot.items.iter().any(|i| i.name.contains("memory") || i.name.contains("OpenClaw") || i.name.ends_with("· old")));
        assert!(spot.effect.contains("Codex and Claude Code") || spot.effect.contains("Claude Code and Codex"), "{}", spot.effect);

        // Nowhere to put them yet.
        assert_eq!(clean("old-conversations", &ctx, None).deleted, 0);
        cfg.archive_dir = Some(archive.clone());
        let ctx = crate::trash::tests::ctx(&paths, &cfg);
        let r = clean("old-conversations", &ctx, None);
        assert_eq!((r.deleted, r.skipped, r.freed), (3, 0, 5400), "{}", r.message);
        assert!(crate::archive::is_link(&sessions.join("07")));
        assert_eq!(std::fs::read(sessions.join("07").join("01").join("rollout-a.jsonl")).unwrap().len(), 3000);
        assert!(!crate::archive::is_link(&sessions.join("08")) && crate::archive::is_link(&sessions.join("08").join("03")));
        assert!(projects.join("C--Workspace-busy").join("memory").join("MEMORY.md").is_file());
        assert!(!crate::archive::is_link(&projects.join("C--Workspace-busy")));
        // Moved ones aren't offered again, and nothing else counts them.
        let again = measure("old-conversations", &ctx).unwrap();
        assert_eq!(again.reclaimable_items, 0);
        assert_eq!(crate::archive::list(&paths).len(), 3);
        for m in crate::archive::list(&paths) {
            crate::archive::bring_back(&paths, &m.from).unwrap();
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn zero_install_is_visible_but_never_sent_to_generic_deletion() {
        let root = fixture("zero-install");
        let paths = Paths::under(&root);
        let cache = paths.local_appdata.join("0install.net").join("implementations");
        let app = cache.join("sha256new_example").join("app.exe");
        write(&app, 500, 400);
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let spot = measure("dev-leftovers", &ctx).unwrap();
        let item = spot.items.iter().find(|i| i.path == cache.to_string_lossy()).unwrap();
        assert_eq!((item.goes, item.optional), (false, false));
        assert!(item.reason.contains("cache manager"));
        assert_eq!((spot.reclaimable_bytes, spot.kept_bytes), (0, 500));
        assert!(spot.note.as_deref().unwrap().contains("Zero Install"));
        // Even an explicit request from a stale UI cannot bypass the fresh verdict.
        let pick = crate::trash::Pick { ticked: vec![item.path.clone()], new: true, ..Default::default() };
        let report = clean("dev-leftovers", &ctx, Some(&pick));
        assert_eq!((report.deleted, report.skipped), (0, 1));
        assert!(app.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn steam_caches_of_installed_games_stay_and_uninstalled_ones_go() {
        let root = fixture("steam-shader-selection");
        // The shared fixture canonicalizes its base, which gives Windows a verbatim
        // prefix. Steam's discovery deliberately accepts ordinary local drive paths
        // only, so model the spelling Steam writes without weakening that boundary.
        #[cfg(windows)]
        let root = {
            let ordinary = root.to_string_lossy();
            assert!(matches!(root.components().next(), Some(std::path::Component::Prefix(p))
                if matches!(p.kind(), std::path::Prefix::VerbatimDisk(_))));
            std::path::PathBuf::from(ordinary.strip_prefix(r"\\?\").unwrap())
        };
        let paths = Paths::under(&root);
        let steam = paths.program_files_x86.join("Steam");
        let apps = steam.join("steamapps");
        let installed = apps.join("shadercache").join("123").join("cache.bin");
        let leftover = apps.join("shadercache").join("456").join("cache.bin");
        write(&installed, 300, 400);
        // Uninstalled last week: its cache goes however new it is.
        write(&leftover, 500, 3);
        std::fs::create_dir_all(steam.join("appcache")).unwrap();
        std::fs::write(steam.join("appcache").join("appinfo.vdf"), crate::trash::steam_appinfo::tests::file(&[
            (123, "Example Game", "Example Game", &["Example.exe"]),
            (456, "Copperline", "Copperline", &["Copperline.exe"]),
        ])).unwrap();
        std::fs::create_dir_all(apps.join("common").join("Example Game")).unwrap();
        std::fs::write(apps.join("libraryfolders.vdf"), format!(
            "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} }}",
            steam.to_string_lossy().replace('\\', "\\\\")
        )).unwrap();
        std::fs::write(apps.join("appmanifest_123.acf"),
            r#""AppState" { "appid" "123" "name" "Example Game" "installdir" "Example Game" }"#
        ).unwrap();
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let spot = measure("shader-cache", &ctx).unwrap();
        assert_eq!(spot.items.len(), 2);
        let kept = spot.items.iter().find(|item| item.name == "Steam · Example Game · Shader cache").unwrap();
        assert!(!kept.goes && kept.optional, "an installed game's cache stays, tickable");
        let gone = spot.items.iter().find(|item| item.name == "Steam · Copperline · Shader cache").unwrap();
        assert!(gone.goes && !gone.optional);
        assert_eq!(gone.reason, "Copperline isn't installed any more");
        assert!(gone.sub.as_deref().is_some_and(|s| s.contains("Not installed in Steam")));
        assert_eq!(spot.reclaimable_bytes, 500);
        clean("shader-cache", &ctx, None);
        assert!(installed.exists() && !leftover.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn nvidia_caches_are_named_by_app_id_and_an_uninstalled_games_go() {
        use crate::trash::tests::touch_old;
        let root = fixture("nvidia-owners");
        let root = {
            let ordinary = root.to_string_lossy();
            std::path::PathBuf::from(ordinary.strip_prefix(r"\\?\").unwrap_or(&ordinary))
        };
        let paths = Paths::under(&root);
        let cache = paths.local_appdata.join("NVIDIA").join("DXCache");
        // Current drivers put the program's name at byte 17; the biggest files keep only a number.
        let nvph = |file: &str, program: Option<&str>, days: u64| {
            let mut bytes = vec![0u8; 4096];
            bytes[..4].copy_from_slice(b"nvph");
            bytes[8] = 0x40;
            bytes[16] = 1;
            match program {
                Some(p) => bytes[17..17 + p.len()].copy_from_slice(p.as_bytes()),
                None => bytes[20..24].copy_from_slice(&[0x0a, 0x7b, 0x3e, 0x90]),
            }
            let path = cache.join(file);
            std::fs::create_dir_all(&cache).unwrap();
            std::fs::write(&path, bytes).unwrap();
            touch_old(&path, days);
        };
        nvph("0002a9cbc7c909d5.nvph", Some("Copperline-Win64-Shipping.exe"), 2);
        nvph("0003a9cbc7c9aaaa.nvph", None, 2);
        nvph("0002a9cb1111aaaa.nvph", Some("Lanternfall.exe"), 2);
        nvph("0002a9cb2222aaaa.nvph", None, 2);
        let steam = paths.program_files_x86.join("Steam");
        std::fs::create_dir_all(steam.join("steamapps")).unwrap();
        std::fs::write(steam.join("steamapps").join("libraryfolders.vdf"), format!(
            "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} }}",
            steam.to_string_lossy().replace('\\', "\\\\")
        )).unwrap();
        std::fs::create_dir_all(steam.join("appcache")).unwrap();
        std::fs::write(steam.join("appcache").join("appinfo.vdf"), crate::trash::steam_appinfo::tests::file(&[
            (970, "Copperline, Night Shift", "Copperline", &[r"Copperline\Binaries\Win64\Copperline-Win64-Shipping.exe"]),
        ])).unwrap();
        write(&paths.system_drive.join("Program Files").join("Lanternfall").join("Lanternfall.exe"), 10, 30);

        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let spot = measure("shader-cache", &ctx).unwrap();
        let row = |file: &str| spot.items.iter().find(|i| i.name.ends_with(file)).unwrap_or_else(|| panic!("{file} in {:#?}", spot.items));
        for file in ["0002a9cbc7c909d5.nvph", "0003a9cbc7c9aaaa.nvph"] {
            let r = row(file);
            assert_eq!(r.name, format!("NVIDIA · Copperline, Night Shift · DXCache {file}"));
            assert!(r.goes, "{file}: uninstalled from Steam, so it goes though it's new");
            assert_eq!(r.reason, "Copperline, Night Shift isn't installed any more");
            assert!(r.sub.as_deref().is_some_and(|s| s.starts_with("Uninstalled from Steam")));
        }
        let installed = row("0002a9cb1111aaaa.nvph");
        assert_eq!(installed.name, "NVIDIA · Lanternfall.exe · DXCache 0002a9cb1111aaaa.nvph");
        assert!(!installed.goes, "installed and new: stays");
        let unknown = row("0002a9cb2222aaaa.nvph");
        assert!(unknown.name.contains("App unknown") && !unknown.goes);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn shaders_game_logs_dev_caches_and_older_versions_go_saves_and_newest_stay() {
        let root = fixture("more");
        let paths = Paths::under(&root);
        let (l, low) = (paths.local_appdata.clone(), root.join("LocalLow"));
        write(&l.join("NVIDIA").join("DXCache").join("old.toc"), 500, 90);
        write(&l.join("NVIDIA").join("DXCache").join("new.toc"), 400, 1);
        write(&low.join("Pixel Forge").join("Swordship").join("Player.log"), 300, 90);
        write(&low.join("Pixel Forge").join("Swordship").join("save.dat"), 300, 400);
        write(&l.join("Atlas").join("Saved").join("Logs").join("Atlas.log"), 200, 90);
        write(&l.join("Atlas").join("Saved").join("SaveGames").join("slot1.sav"), 200, 400);
        write(&l.join("Pub").join("Cache").join("hosted").join("pkg"), 100, 100);
        write(&l.join("Slack").join("Update.exe"), 10, 400);
        write(&l.join("Slack").join("app-4.40.0").join("slack.exe"), 600, 200);
        write(&l.join("Slack").join("app-4.41.0").join("slack.exe"), 600, 5);
        let snaps = l.join("Google").join("Chrome").join("User Data").join("Snapshots");
        write(&snaps.join("151.0.1").join("x"), 70, 100);
        write(&snaps.join("154.0.2").join("x"), 70, 10);
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let going = |id: &str| {
            let mut v: Vec<String> = measure(id, &ctx).unwrap().items.into_iter().filter(|i| i.goes).map(|i| i.name).collect();
            v.sort();
            v
        };
        assert_eq!(going("shader-cache"), vec!["NVIDIA · App unknown · DXCache old.toc"]);
        assert_eq!(going("game-junk"), vec!["Unity games · Swordship · Player.log", "Unreal games · Atlas · Atlas.log"]);
        assert_eq!(going("dev-leftovers"), vec!["Developer tools · Dart · package cache"]);
        assert_eq!(going("app-leftovers"), vec!["Squirrel apps · Slack 4.40.0"]);
        // Chrome's snapshots back up whole profiles, including sign-ins. Even the older
        // version is protected; an old/stale selection must not turn it into app junk.
        let app_leftovers = measure("app-leftovers", &ctx).unwrap();
        let snapshots: Vec<_> = app_leftovers.items.iter().filter(|i| i.name.starts_with("Chrome · ")).collect();
        assert_eq!(snapshots.len(), 2);
        assert!(snapshots.iter().all(|i| !i.goes && !i.optional && i.reason == crate::trash::BROWSER_DATA));
        let stale = crate::trash::Pick {
            goes: false,
            ticked: snapshots.iter().map(|i| i.path.clone()).collect(),
            new: true,
            ..Default::default()
        };
        let refused = clean("app-leftovers", &ctx, Some(&stale));
        assert_eq!((refused.deleted, refused.skipped, refused.freed), (0, 2, 0));
        for id in ["shader-cache", "game-junk", "dev-leftovers", "app-leftovers"] {
            clean(id, &ctx, None);
        }
        assert!(low.join("Pixel Forge").join("Swordship").join("save.dat").exists());
        assert!(l.join("Atlas").join("Saved").join("SaveGames").join("slot1.sav").exists());
        assert!(l.join("NVIDIA").join("DXCache").join("new.toc").exists());
        assert!(l.join("Slack").join("app-4.41.0").exists() && !l.join("Slack").join("app-4.40.0").exists());
        assert_eq!(std::fs::read(snaps.join("151.0.1").join("x")).unwrap(), vec![b'x'; 70]);
        assert_eq!(std::fs::read(snaps.join("154.0.2").join("x")).unwrap(), vec![b'x'; 70]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn windows_leftovers_are_found_at_the_top_of_every_drive() {
        let root = fixture("drives");
        let mut paths = Paths::under(&root);
        let (c, d) = (paths.system_drive.clone(), root.join("d"));
        paths.other_drives = vec![d.clone()];
        // An upgrade from long ago left its working folder on the second drive; the update
        // assistant left its own on the first.
        write(&d.join("$WINDOWS.~TMP").join("Rollback").join("old.wim"), 6000, 400);
        write(&c.join("Windows10Upgrade").join("setup.exe"), 300, 200);
        write(&c.join("AMD").join("driver").join("x.cab"), 200, 200);
        // Last week's upgrade can still be gone back from: its files stay.
        write(&c.join("$WINDOWS.~BT").join("Sources").join("install.esd"), 900, 5);
        // What a disk check recovered, three times: one thing to decide about.
        for n in ["found.000", "found.001", "found.002"] {
            write(&d.join(n).join("file0000.chk"), 100, 300);
        }
        // Never these: a name that only looks alike, a folder of your own that happens to be
        // called what an installer calls its own on C:, your games, and Windows' own folder.
        write(&d.join("found.things").join("mine.txt"), 50, 300);
        write(&d.join("found.0001").join("mine.txt"), 50, 300);
        write(&d.join("AMD").join("my notes.txt"), 50, 300);
        write(&d.join("Games").join("save.dat"), 50, 300);
        write(&d.join("System Volume Information").join("tracking.log"), 50, 300);

        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let spot = measure("windows-junk", &ctx).unwrap();
        let mut items: Vec<(String, bool, u64)> = spot.items.iter().map(|i| (i.name.clone(), i.goes, i.bytes)).collect();
        items.sort();
        let at = |drive: &std::path::Path, name: &str| drive.join(name).display().to_string();
        assert_eq!(
            items,
            vec![
                (format!("Disk check · {} to found.002 (3 folders)", at(&d, "found.000")), true, 300),
                (format!("Installers · {}", at(&c, "AMD")), true, 200),
                (format!("Windows setup · {}", at(&c, "$WINDOWS.~BT")), false, 900),
                (format!("Windows setup · {}", at(&c, "Windows10Upgrade")), true, 300),
                (format!("Windows setup · {}", at(&d, "$WINDOWS.~TMP")), true, 6000),
            ]
        );
        assert_eq!(super::at_the_top(std::path::Path::new("D:\\"), &["$WINDOWS.~TMP".into()]), "D:\\$WINDOWS.~TMP");
        assert!(super::check_folder("FOUND.017") && !super::check_folder("found.17") && !super::check_folder("found.0é"));

        // In a made-up folder everything is yours to delete, so the clean goes ahead as you.
        clean("windows-junk", &ctx, None);
        assert!(!d.join("$WINDOWS.~TMP").exists() && !c.join("Windows10Upgrade").exists());
        assert!(["found.000", "found.001", "found.002"].iter().all(|n| !d.join(n).exists()), "all three went, as one");
        assert!(c.join("$WINDOWS.~BT").join("Sources").join("install.esd").exists(), "a go-back that's still good stays");
        for kept in ["found.things", "found.0001", "AMD", "Games", "System Volume Information"] {
            assert!(d.join(kept).exists(), "{kept}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn pending_or_unknown_windows_updates_lock_setup_and_downloads_on_every_drive() {
        use crate::trash::windows_update::State;
        for (name, state) in [("pending", State::Pending), ("unknown", State::Unknown)] {
            let root = fixture(&format!("windows-update-{name}"));
            let mut paths = Paths::under(&root);
            let other_drive = root.join("d");
            paths.other_drives = vec![other_drive.clone()];
            let protected = [
                paths.system_drive.join("$GetCurrent"),
                other_drive.join("Windows10Upgrade"),
                other_drive.join("$WINDOWS.~BT"),
                paths.windows.join("SoftwareDistribution").join("Download").join("download-id"),
            ];
            for (i, path) in protected.iter().enumerate() {
                // Recent setup data must be locked too, rather than merely unticked.
                write(&path.join("data.bin"), 100, if i == 2 { 1 } else { 200 });
            }
            let installer = paths.system_drive.join("AMD");
            let report = paths.program_data.join("Microsoft").join("Windows").join("WER").join("ReportArchive").join("old-report");
            write(&installer.join("driver.cab"), 50, 200);
            write(&report.join("report.wer"), 60, 200);

            let cfg = TrashConfig::default();
            let calls = std::cell::Cell::new(0);
            let probe = || {
                calls.set(calls.get() + 1);
                state
            };
            let mut ctx = ctx(&paths, &cfg);
            ctx.windows_update_probe = Some(&probe);
            let spot = measure("windows-junk", &ctx).unwrap();
            assert_eq!(calls.get(), 1, "one probe serves all update candidates in a scan");
            assert_eq!(spot.note.as_deref(), state.reason());
            for path in &protected {
                let item = spot.items.iter().find(|item| item.path == path.to_string_lossy()).unwrap();
                assert!(!item.goes && !item.optional, "{} must stay locked: {item:?}", path.display());
                assert_eq!(Some(item.reason.as_str()), state.reason());
            }
            for path in [&installer, &report] {
                assert!(spot.items.iter().any(|item| item.path == path.to_string_lossy() && item.goes), "unrelated old files retain their verdict");
            }

            // A stale UI or explicit tick of a recent item cannot turn the hard stop into a
            // deletion. This also exercises the fresh finding used by clean(), not only UI.
            let pick = crate::trash::Pick {
                goes: false,
                new: true,
                ticked: protected.iter().map(|path| path.to_string_lossy().into_owned()).collect(),
                ..Default::default()
            };
            let result = clean("windows-junk", &ctx, Some(&pick));
            assert_eq!((result.deleted, result.skipped), (0, protected.len() as u64));
            assert!(protected.iter().all(|path| path.join("data.bin").exists()));
            assert!(installer.exists() && report.exists());
            std::fs::remove_dir_all(&root).unwrap();
        }
    }

    #[test]
    fn clear_windows_update_state_preserves_normal_age_verdicts() {
        use crate::trash::windows_update::State;
        let root = fixture("windows-update-clear");
        let paths = Paths::under(&root);
        let old = paths.system_drive.join("$GetCurrent");
        let recent = paths.system_drive.join("Windows10Upgrade");
        let download = paths.windows.join("SoftwareDistribution").join("Download").join("old-download");
        write(&old.join("data.bin"), 100, 200);
        write(&recent.join("data.bin"), 100, 1);
        write(&download.join("update.cab"), 100, 200);
        let cfg = TrashConfig::default();
        let calls = std::cell::Cell::new(0);
        let probe = || {
            calls.set(calls.get() + 1);
            State::Clear
        };
        let mut ctx = ctx(&paths, &cfg);
        ctx.windows_update_probe = Some(&probe);
        let spot = measure("windows-junk", &ctx).unwrap();
        assert_eq!(calls.get(), 1);
        assert!(spot.note.is_none());
        for path in [&old, &download] {
            assert!(spot.items.iter().any(|item| item.path == path.to_string_lossy() && item.goes));
        }
        let item = spot.items.iter().find(|item| item.path == recent.to_string_lossy()).unwrap();
        assert!(!item.goes && item.optional);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn unrelated_windows_leftovers_do_not_probe_update_state() {
        use crate::trash::windows_update::State;
        let root = fixture("windows-update-unrelated");
        let paths = Paths::under(&root);
        let installer = paths.system_drive.join("AMD");
        let report = paths.program_data.join("Microsoft").join("Windows").join("WER").join("ReportArchive").join("old-report");
        write(&installer.join("driver.cab"), 50, 200);
        write(&report.join("report.wer"), 60, 200);
        let cfg = TrashConfig::default();
        let calls = std::cell::Cell::new(0);
        let probe = || {
            calls.set(calls.get() + 1);
            State::Unknown
        };
        let mut ctx = ctx(&paths, &cfg);
        ctx.windows_update_probe = Some(&probe);
        let spot = measure("windows-junk", &ctx).unwrap();
        assert_eq!(calls.get(), 0, "no update data means no update-state query");
        assert_eq!(spot.reclaimable_items, 2);
        assert!(spot.note.is_none());
        let result = clean("windows-junk", &ctx, None);
        assert_eq!(result.deleted, 2);
        assert_eq!(calls.get(), 0);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_newest_version_is_kept_by_number_not_by_date() {
        use super::version_in;
        assert!(version_in("27") > version_in("26"));
        assert!(version_in("app-4.51.180") > version_in("app-4.50.143"));
        assert!(version_in("slack-4.51.180-full.nupkg") > version_in("slack-4.50.143-full.nupkg"));
        assert!(version_in("153.0.8010.54") > version_in("152.0.7977.84"));
        assert!(version_in("webcache_4430") > version_in("webcache_4147"));
        assert!(version_in("webcache_4147") > version_in("webcache"));
        assert_eq!(version_in("0fb762053c34788bb7760d5673f8a6d4c8589d50"), None);

        // Photoshop 27 is in use, though 26 was touched more recently: 25 and 26 go, 27 stays.
        let root = fixture("versions");
        let paths = Paths::under(&root);
        let phsp = paths.appdata.join("Adobe").join("UXP").join("PluginsStorage").join("PHSP");
        write(&phsp.join("25").join("a"), 100, 560);
        write(&phsp.join("26").join("a"), 100, 100);
        write(&phsp.join("27").join("a"), 100, 380);
        let cfg = TrashConfig::default();
        let mut ctx = ctx(&paths, &cfg);
        ctx.installed = Some(crate::installed::Installed::with(&["Adobe Photoshop 2026"]));
        let spot = measure("app-leftovers", &ctx).unwrap();
        let state = |n: &str| spot.items.iter().find(|i| i.name.contains(n)).map(|i| (i.goes, i.reason.clone())).unwrap();
        assert_eq!(state("PHSP 27"), (false, super::NEWEST.to_string()));
        assert!(state("PHSP 26").0 && state("PHSP 25").0);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_tool_that_isnt_installed_has_nothing_to_show() {
        let root = fixture("no-harness");
        let paths = Paths::under(&root);
        std::fs::create_dir_all(&paths.home).unwrap();
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        for id in ["agent-leftovers", "app-leftovers", "cursor-worktrees", "old-conversations"] {
            assert!(!measure(id, &ctx).unwrap().available, "{id}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }
}
