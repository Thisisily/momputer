//! What agent tools leave behind outside the Temp scratch Claude and Codex have their own
//! clean-ups for: logs, temp files, caches, undo snapshots, backups and worktrees. Each tool
//! gets a clean-up of its own, and only the folders listed here are looked in, so settings,
//! sign-ins, sessions, history and memory are never touched.
//!
//! Want mom to clean up after another tool? Add a `Def` in trash.rs and its sources here.

use super::{ago, cands, entries, name_of, too_fresh, Cand, Found, TrashCtx, IN_USE};
use std::path::PathBuf;

/// One place a tool leaves things. Every entry in `dir` that `pick` accepts is one item.
struct Source {
    dir: PathBuf,
    /// What one of them is ("Log") and what a bunch are ("logs").
    kind: (&'static str, &'static str),
    pick: fn(&str) -> bool,
    /// What to call an entry, from its file name.
    name: fn(&str) -> String,
    /// Each entry belongs to a session, named by its id; a running session's stays.
    sessions: bool,
    /// Only empty entries count (a folder an older version left behind).
    empty_only: bool,
}

fn src(dir: PathBuf, kind: (&'static str, &'static str), pick: fn(&str) -> bool) -> Source {
    Source { dir, kind, pick, name: |n| n.to_string(), sessions: false, empty_only: false }
}

impl Source {
    fn named(mut self, name: fn(&str) -> String) -> Source {
        self.name = name;
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

fn session(n: &str) -> String {
    format!("Session {}", n.chars().take(8).collect::<String>())
}

/// Where a tool lives (any of them existing means it's installed) and what it leaves.
struct Tool {
    homes: Vec<PathBuf>,
    sources: Vec<Source>,
}

fn tool(id: &str, ctx: &TrashCtx) -> Option<Tool> {
    let p = ctx.paths;
    let home = &p.home;
    let temp = &p.temp;
    Some(match id {
        "claude-leftovers" => {
            let c = p.claude_dir();
            Tool {
                homes: vec![c.clone()],
                sources: vec![
                    src(c.join("debug"), ("Debug log", "debug logs"), |n| n != "latest"),
                    src(c.join("shell-snapshots"), ("Shell snapshot", "shell snapshots"), any),
                    src(c.join("file-history"), ("Rewind backups", "rewind backups"), any).named(session).sessions(),
                    src(c.join("session-env"), ("Session settings", "session settings"), any).named(session).sessions(),
                    src(c.join("paste-cache"), ("Pasted text", "pasted text"), any),
                    src(home.clone(), ("Half-written settings", "half-written copies of .claude.json"), |n| {
                        n.starts_with(".claude.json.tmp.")
                    }),
                    src(temp.clone(), ("Temp folder", "temp folders"), |n| n.to_ascii_lowercase().starts_with("claude-")),
                ],
            }
        }
        "opencode-leftovers" => {
            let share = p.agent_env("XDG_DATA_HOME").map(|d| d.to_path_buf()).unwrap_or_else(|| home.join(".local").join("share"));
            let data = share.join("opencode");
            Tool {
                homes: vec![data.clone(), home.join(".config").join("opencode")],
                sources: vec![
                    src(data.join("log"), ("Log", "logs"), any),
                    src(data.join("snapshot"), ("Undo snapshots", "undo snapshots"), any)
                        .named(|n| format!("Undo snapshots {}", n.chars().take(8).collect::<String>())),
                    src(data.join("tool-output"), ("Saved tool output", "saved tool output"), any),
                ],
            }
        }
        "openclaw-leftovers" => {
            let state = p.agent_env("OPENCLAW_STATE_DIR").map(|d| d.to_path_buf()).unwrap_or_else(|| home.join(".openclaw"));
            Tool {
                homes: vec![state.clone()],
                sources: vec![
                    src(state.join("logs"), ("Log", "logs"), any),
                    src(state.clone(), ("Old config backup", "old config backups"), |n| n.starts_with("openclaw.json.bak")),
                    src(temp.join("openclaw"), ("Log", "logs"), any),
                    src(temp.clone(), ("Temp folder", "temp folders"), |n| n.to_ascii_lowercase().starts_with("openclaw-")),
                    // OpenClaw was called Clawdbot, then Moltbot. Empty folders from then can go.
                    src(home.clone(), ("Empty folder from an old name", "empty folders from its old names"), |n| {
                        n == ".clawdbot" || n == ".moltbot"
                    })
                    .empty_only(),
                ],
            }
        }
        "hermes-leftovers" => {
            let mut homes: Vec<PathBuf> = p.agent_env("HERMES_HOME").map(|d| d.to_path_buf()).into_iter().collect();
            for h in [home.join(".hermes"), p.local_appdata.join("hermes")] {
                if !homes.contains(&h) {
                    homes.push(h);
                }
            }
            let mut sources = Vec::new();
            for h in &homes {
                let cache = h.join("cache");
                sources.extend([
                    src(h.join("logs"), ("Log", "logs"), any),
                    src(h.join("pastes"), ("Pasted text", "pasted text"), any),
                    src(h.join("checkpoints"), ("Rollback checkpoint", "rollback checkpoints"), any),
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
            Tool { homes, sources }
        }
        "pi-leftovers" => {
            let agent = p.agent_env("PI_CODING_AGENT_DIR").map(|d| d.to_path_buf()).unwrap_or_else(|| home.join(".pi").join("agent"));
            Tool {
                homes: vec![home.join(".pi"), agent.clone()],
                sources: vec![
                    src(temp.clone(), ("Bash output", "bash output dumps"), |n| n.starts_with("pi-bash-")),
                    src(temp.join("pi-extensions"), ("Trial extension install", "trial extension installs"), any),
                    src(temp.clone(), ("Slack bot output", "Slack bot output dumps"), |n| n.starts_with("mom-bash-")),
                    src(agent, ("Debug log", "debug logs"), |n| n == "pi-debug.log"),
                ],
            }
        }
        "cursor-worktrees" => {
            let root = home.join(".cursor").join("worktrees");
            // ~/.cursor/worktrees/<project>/<worktree>: one item per worktree.
            let sources = entries(&root)
                .into_iter()
                .filter(|d| d.is_dir())
                .map(|d| src(d, ("Worktree", "worktrees"), any))
                .collect();
            Tool { homes: vec![root], sources }
        }
        _ => return None,
    })
}

pub(super) fn find(id: &str, ctx: &TrashCtx, min_age: u64) -> Found {
    let Some(t) = tool(id, ctx) else {
        return Found::none();
    };
    let mut f = Found::at(t.homes[0].clone());
    f.available = t.homes.iter().any(|h| h.exists());
    let mut list = Vec::new();
    let mut from = Vec::new();
    for (i, s) in t.sources.iter().enumerate() {
        for p in entries(&s.dir) {
            let n = name_of(&p);
            if (s.pick)(&n) {
                let name = if id == "cursor-worktrees" { format!("{} · {n}", name_of(&s.dir)) } else { (s.name)(&n) };
                list.push((p, name));
                from.push(i);
            }
        }
    }
    for (mut c, i) in cands(list, ctx.now).into_iter().zip(from) {
        let s = &t.sources[i];
        if s.empty_only && c.bytes > 0 {
            continue;
        }
        c.sub = Some(match c.sub.take() {
            Some(inside) => format!("{}, {inside}", s.kind.0),
            None => s.kind.0.to_string(),
        });
        c.kind = Some(s.kind.1);
        if s.empty_only {
            // Nothing in it, so nothing to lose, however new it is.
            f.cands.push(c.verdict(true, "Empty, from an older version"));
            continue;
        }
        f.cands.push(judge(c, ctx, min_age, s.sessions));
    }
    f.available = f.available || !f.cands.is_empty();
    f
}

fn judge(c: Cand, ctx: &TrashCtx, min_age: u64, sessions: bool) -> Cand {
    if sessions && ctx.session_live(&name_of(&c.path)) {
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

/// Things in %TEMP% these clean-ups own, so "Old temp files" doesn't count them twice.
pub(super) fn owns_temp(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["claude-", "openclaw", "pi-bash-", "pi-extensions", "mom-bash-"].iter().any(|p| n.starts_with(p))
}

/// The tool's name, and what its clean-up never touches.
pub(super) fn about(id: &str) -> Option<(&'static str, &'static str)> {
    Some(match id {
        "claude-leftovers" => ("Claude Code", "Your chats, projects, settings, plugins and memory aren't touched."),
        "opencode-leftovers" => (
            "OpenCode",
            "OpenCode's sessions, settings and sign-ins aren't touched; old projects' undo snapshots go, so their \
             changes can no longer be rewound.",
        ),
        "openclaw-leftovers" => ("OpenClaw", "OpenClaw's agents, memory, workspace, sign-ins and settings aren't touched."),
        "hermes-leftovers" => ("Hermes", "Hermes's sessions, memories, skills and settings aren't touched."),
        "pi-leftovers" => ("Pi", "Pi's sessions, settings and sign-ins aren't touched."),
        _ => return None,
    })
}

/// "logs, undo snapshots and pasted text", from what's going.
pub(super) fn kinds(goes: &[Cand]) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for k in goes.iter().filter_map(|c| c.kind) {
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
        write(&home.join(".hermes").join("sessions").join("s1.json"), 999, 300);
        write(&home.join(".hermes").join("memories").join("me.md"), 999, 300);
        // Pi: overflow output in Temp, never its sessions.
        write(&temp.join("pi-bash-0a1b2c3d.log"), 200, 70);
        write(&home.join(".pi").join("agent").join("sessions").join("x").join("s.jsonl"), 999, 300);
        // OpenCode: logs and old undo snapshots.
        let data = home.join(".local").join("share").join("opencode");
        write(&data.join("log").join("2026-01-01T000000.log"), 50, 100);
        write(&data.join("snapshot").join("965945dc5b8d").join("objects").join("pack"), 5000, 100);
        write(&data.join("opencode.db"), 999, 100);
        // OpenClaw: a log, a temp folder, an old backup, and the empty folder of an old name.
        write(&home.join(".openclaw").join("logs").join("commands.log"), 60, 70);
        write(&home.join(".openclaw").join("openclaw.json.bak.2"), 70, 70);
        write(&home.join(".openclaw").join("openclaw.json"), 999, 70);
        write(&temp.join("openclaw-img-x1").join("a.png"), 80, 70);
        std::fs::create_dir_all(home.join(".moltbot")).unwrap();
        // Claude Code: rewind backups of a running session stay; a half-written settings copy goes.
        write(&home.join(".claude").join("file-history").join("0191f20b-ae85").join("a.ts"), 500, 90);
        write(&home.join(".claude").join("file-history").join("e9c3412f-ef20").join("b.ts"), 500, 90);
        write(&home.join(".claude.json.tmp.1234.abcd"), 30, 90);
        write(&home.join(".claude").join("projects").join("p").join("s.jsonl"), 999, 300);

        let cfg = TrashConfig::default();
        let mut ctx = ctx(&paths, &cfg);
        ctx.live_sessions.insert("e9c3412f-ef20".into());

        let hermes = measure("hermes-leftovers", &ctx).unwrap();
        assert!(hermes.available);
        assert_eq!((hermes.reclaimable_items, hermes.reclaimable_bytes), (2, 700));
        assert_eq!((hermes.new_items, hermes.new_bytes), (1, 100), "today's log can be ticked");
        assert!(hermes.effect.contains("logs and cached images"), "{}", hermes.effect);
        assert!(hermes.effect.contains("67 days"), "{}", hermes.effect);

        let pi = measure("pi-leftovers", &ctx).unwrap();
        assert_eq!((pi.reclaimable_items, pi.reclaimable_bytes), (1, 200));

        let opencode = measure("opencode-leftovers", &ctx).unwrap();
        assert_eq!((opencode.reclaimable_items, opencode.reclaimable_bytes), (2, 5050));

        let claw = measure("openclaw-leftovers", &ctx).unwrap();
        assert_eq!(claw.reclaimable_items, 4, "{:#?}", claw.items);

        let claude = measure("claude-leftovers", &ctx).unwrap();
        assert_eq!((claude.reclaimable_items, claude.reclaimable_bytes), (2, 530));
        let running = claude.items.iter().find(|i| i.name == "Session e9c3412f").unwrap();
        assert_eq!((running.goes, running.optional), (false, false));

        // Nothing here is in Old temp files twice.
        let old = measure("old-temp", &ctx).unwrap();
        assert_eq!(old.reclaimable_items + old.kept_items, 0, "{:#?}", old.items);

        for id in ["hermes-leftovers", "pi-leftovers", "opencode-leftovers", "openclaw-leftovers", "claude-leftovers"] {
            clean(id, &ctx, None);
        }
        assert!(home.join(".hermes").join("logs").join("gateway.log").exists());
        assert!(!home.join(".hermes").join("logs").join("agent.log").exists());
        assert!(home.join(".hermes").join("sessions").join("s1.json").exists());
        assert!(home.join(".hermes").join("memories").join("me.md").exists());
        assert!(home.join(".pi").join("agent").join("sessions").join("x").join("s.jsonl").exists());
        assert!(data.join("opencode.db").exists() && !data.join("snapshot").join("965945dc5b8d").exists());
        assert!(home.join(".openclaw").join("openclaw.json").exists() && !home.join(".moltbot").exists());
        assert!(home.join(".claude").join("file-history").join("e9c3412f-ef20").exists());
        assert!(home.join(".claude").join("projects").join("p").join("s.jsonl").exists());
        assert!(!home.join(".claude.json.tmp.1234.abcd").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_tool_that_isnt_installed_has_nothing_to_show() {
        let root = fixture("no-harness");
        let paths = Paths::under(&root);
        std::fs::create_dir_all(&paths.home).unwrap();
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        for id in ["opencode-leftovers", "openclaw-leftovers", "hermes-leftovers", "pi-leftovers", "cursor-worktrees"] {
            assert!(!measure(id, &ctx).unwrap().available, "{id}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }
}
