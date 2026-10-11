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
use std::path::{Path, PathBuf};

/// Which clean-up a tool's leftovers show up in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Spot {
    Claude,
    Codex,
    Agents,
    Apps,
    Worktrees,
}

impl Spot {
    fn of(id: &str) -> Option<Spot> {
        Some(match id {
            "claude-leftovers" => Spot::Claude,
            "codex-temp" => Spot::Codex,
            "agent-leftovers" => Spot::Agents,
            "app-leftovers" => Spot::Apps,
            "cursor-worktrees" => Spot::Worktrees,
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
}

fn src(dir: PathBuf, kind: (&'static str, &'static str), pick: fn(&str) -> bool) -> Source {
    Source { dir, kind, pick, name: |_, rel| rel.join(" / "), depth: 0, files_only: false, keep_newest: 0, sessions: false, empty_only: false }
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
}

fn any(_: &str) -> bool {
    true
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
            src(r.join("CachedData"), ("Old build", "old builds"), any).keep_newest(1),
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
    let mixed = matches!(spot, Spot::Agents | Spot::Apps | Spot::Claude);
    let mut list = Vec::new();
    let mut from: Vec<(&Agent, &Source)> = Vec::new();
    for a in &all {
        for s in &a.junk {
            for (path, rel) in walk(&s.dir, s.depth, s.files_only) {
                if !(s.pick)(rel.last().map(String::as_str).unwrap_or("")) {
                    continue;
                }
                let name = (s.name)(&s.dir, &rel);
                list.push((path, if mixed { format!("{} · {name}", a.name) } else { name }));
                from.push((a, s));
            }
        }
    }
    let mut found: Vec<(Cand, &Agent, &Source)> =
        super::cands(list, ctx.now).into_iter().zip(from).map(|(c, (a, s))| (c, a, s)).collect();
    // "Keep the newest": the newest few of each such source stay, whatever their age.
    found.sort_by_key(|(c, _, s)| (s.dir.clone(), c.age));
    let mut kept_per: std::collections::HashMap<PathBuf, usize> = std::collections::HashMap::new();
    for (mut c, a, s) in found {
        if s.empty_only && (c.files > 0 || c.links > 0 || c.bytes > 0) {
            continue; // not empty after all: it isn't a leftover
        }
        c.sub = Some(match c.sub.take() {
            Some(inside) => format!("{}, {inside}", s.kind.0),
            None => s.kind.0.to_string(),
        });
        c.kind = Some(s.kind.1);
        c.tool = Some(a.name);
        if s.empty_only {
            f.cands.push(c.verdict(true, "Empty, from an older version"));
            continue;
        }
        if s.keep_newest > 0 {
            let n = kept_per.entry(s.dir.clone()).or_default();
            if *n < s.keep_newest {
                *n += 1;
                f.cands.push(c.verdict(false, "The newest one, kept"));
                continue;
            }
        }
        f.cands.push(judge(c, ctx, min_age, s.sessions));
    }
    f.available = f.available || !f.cands.is_empty();
    f
}

/// Agent folders hold links to shared skills; one with a link inside is never a leftover.
const LINKED: &str = "Holds a link to another folder (a shared skill, say)";

fn judge(c: Cand, ctx: &TrashCtx, min_age: u64, sessions: bool) -> Cand {
    if c.links > 0 {
        c.verdict(false, LINKED)
    } else if sessions && ctx.session_live(&name_of(&c.path)) {
        c.verdict(false, "Session is still running")
    } else if ctx.in_use(&c.path) {
        c.verdict(false, IN_USE)
    } else if c.age < min_age {
        let r = too_fresh(c.age, min_age);
        c.kept_while_new(r)
    } else {
        let r = format!("Untouched for {}", ago(c.age));
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

/// What a clean-up never touches, for its explanation.
pub(super) fn untouched(id: &str) -> &'static str {
    match id {
        "claude-leftovers" => "Your chats, projects, settings, plugins and memory aren't touched.",
        "app-leftovers" => "The builds they run now, your extensions, settings and unsaved files aren't touched.",
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
                let age = ctx.now.saturating_sub(t.newest);
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
    if c.links > 0 {
        c.verdict(false, LINKED)
    } else if ctx.in_use(&c.path) {
        c.verdict(false, IN_USE)
    } else if c.age < 86_400 {
        // Today's might still be being written.
        let r = format!("Opened {} ago", ago(c.age));
        c.verdict(false, r)
    } else if c.age < min_age {
        let r = format!("Opened {} ago (kept here for {})", ago(c.age), super::human_age_long(min_age));
        c.kept_while_new(r)
    } else {
        let r = format!("Not opened for {}", ago(c.age));
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
        assert_eq!(apps.items.iter().find(|i| i.name == "VS Code · bbb").unwrap().reason, "The newest one, kept");

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
