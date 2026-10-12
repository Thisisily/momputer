//! Servers mom stopped, and how to start each one again: the command that started it and its
//! folder, kept in `stopped.json` (the last 12).

use crate::classify::Category;
use crate::paths::Paths;
use crate::procs::{Proc, ProcTable};
use crate::scan::Group;
use crate::util::now_secs;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Stopped {
    pub id: String,
    pub name: String,
    pub project: Option<String>,
    /// What to run, in `cwd`: "npm run dev", or the program and its arguments.
    pub command: String,
    pub cwd: String,
    pub ports: Vec<u16>,
    /// Unix seconds.
    pub at: u64,
}

const KEEP: usize = 12;

fn file(paths: &Paths) -> std::path::PathBuf {
    paths.state_dir().join("stopped.json")
}

/// Every server mom stopped and can start again, newest first.
pub fn list(paths: &Paths) -> Vec<Stopped> {
    std::fs::read(file(paths)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save(paths: &Paths, list: &[Stopped]) -> anyhow::Result<()> {
    std::fs::create_dir_all(paths.state_dir())?;
    std::fs::write(file(paths), serde_json::to_vec_pretty(list)?)?;
    Ok(())
}

/// Notes the servers among `groups` (about to be stopped) with how each was started.
pub fn remember(paths: &Paths, table: &ProcTable, groups: &[&Group]) {
    let now = now_secs();
    let mut all = list(paths);
    for g in groups.iter().filter(|g| matches!(g.category, Category::DevServer | Category::Server | Category::Watcher)) {
        let Some((command, cwd)) = start_of(table, g) else { continue };
        all.retain(|s| !(s.command == command && s.cwd.eq_ignore_ascii_case(&cwd)));
        all.insert(
            0,
            Stopped {
                id: format!("{}-{}", g.root_pid, g.root_start),
                name: g.name.clone(),
                project: g.project.clone(),
                command,
                cwd,
                ports: g.ports.iter().map(|l| l.port).collect(),
                at: now,
            },
        );
    }
    all.truncate(KEEP);
    let _ = save(paths, &all);
}

/// Forgets one.
pub fn forget(paths: &Paths, id: &str) -> anyhow::Result<()> {
    let mut all = list(paths);
    all.retain(|s| s.id != id);
    save(paths, &all)
}

/// Starts one again in a new terminal window, in its folder, and forgets it.
pub fn start(paths: &Paths, id: &str) -> Result<String, String> {
    let all = list(paths);
    let s = all.iter().find(|s| s.id == id).ok_or("mom doesn't remember that one any more.")?;
    if !std::path::Path::new(&s.cwd).is_dir() {
        return Err(format!("Its folder is gone: {}", s.cwd));
    }
    launch(&s.command, &s.cwd).map_err(|e| format!("Windows couldn't start it: {e}"))?;
    let _ = forget(paths, id);
    Ok(format!("Started {} again, in a new window.", s.name))
}

/// A new console running `cmd /k <command>` in `cwd`: it opens in your default terminal, and
/// stays open when the server stops so you can read why.
#[cfg(windows)]
fn launch(command: &str, cwd: &str) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    std::process::Command::new("cmd.exe").raw_arg(format!("/k {command}")).current_dir(cwd).creation_flags(CREATE_NEW_CONSOLE).spawn().map(|_| ())
}

#[cfg(not(windows))]
fn launch(command: &str, cwd: &str) -> std::io::Result<()> {
    std::process::Command::new("sh").arg("-c").arg(command).current_dir(cwd).spawn().map(|_| ())
}

/// The command that started a group, and its folder. Climbs from the group's first process
/// through `cmd /c` and `sh -c` wrappers to the package manager that ran it, if one did: what
/// you typed ("npm run dev") rather than what it became (node with a long path).
pub fn start_of(table: &ProcTable, g: &Group) -> Option<(String, String)> {
    let root = table.get(g.root_pid)?;
    let mut best: &Proc = root;
    let mut cur = root.pid;
    for _ in 0..6 {
        let Some(parent) = table.parent(cur) else { break };
        if runner(&parent.cmd).is_some() {
            best = parent;
        } else if !wrapper(&parent.cmd) {
            break;
        }
        cur = parent.pid;
    }
    let command = runner(&best.cmd).unwrap_or_else(|| quoted(&best.cmd));
    let cwd = best.cwd.clone().or_else(|| g.cwd.clone())?;
    (!command.trim().is_empty()).then_some((command, cwd))
}

/// "npm run dev" from `node …\npm-cli.js run dev`, and the same for npx, pnpm, yarn and bun.
fn runner(cmd: &[String]) -> Option<String> {
    let base = |s: &str| s.rsplit(['\\', '/']).next().unwrap_or(s).to_ascii_lowercase();
    let first = base(cmd.first()?);
    let (name, rest) = if first == "node.exe" || first == "node" {
        let script = base(cmd.get(1)?);
        let name = match script.as_str() {
            "npm-cli.js" => "npm",
            "npx-cli.js" => "npx",
            "pnpm.cjs" | "pnpm.js" | "pnpm.mjs" => "pnpm",
            "yarn.js" | "yarn.cjs" => "yarn",
            _ => return None,
        };
        (name, &cmd[2..])
    } else {
        let name = match first.trim_end_matches(".exe").trim_end_matches(".cmd") {
            "bun" => "bun",
            "pnpm" => "pnpm",
            "yarn" => "yarn",
            "npm" => "npm",
            _ => return None,
        };
        (name, &cmd[1..])
    };
    // Only a script run ("npm run dev", "pnpm dev", "bun run start"), not an install.
    let verb = rest.first().map(|v| v.to_ascii_lowercase()).unwrap_or_default();
    if matches!(verb.as_str(), "install" | "i" | "add" | "ci" | "update" | "remove" | "uninstall" | "exec" | "x") || verb.is_empty() {
        return None;
    }
    Some(std::iter::once(name.to_string()).chain(std::iter::once(quoted(rest))).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" "))
}

/// `cmd /c …` and `sh -c …`: shells that only ran one command.
fn wrapper(cmd: &[String]) -> bool {
    let first = cmd.first().map(|s| s.rsplit(['\\', '/']).next().unwrap_or(s).to_ascii_lowercase()).unwrap_or_default();
    let flag = |f: &str| cmd.iter().skip(1).any(|a| a.eq_ignore_ascii_case(f));
    ((first == "cmd.exe" || first == "cmd") && flag("/c")) || (matches!(first.as_str(), "sh.exe" | "sh" | "bash.exe" | "bash") && flag("-c"))
}

/// Arguments back into one line, quoting the ones with spaces.
fn quoted(args: &[String]) -> String {
    args.iter()
        .map(|a| if a.is_empty() || a.contains([' ', '\t']) { format!("\"{}\"", a.replace('"', "\\\"")) } else { a.clone() })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn what_you_typed_not_what_it_became() {
        assert_eq!(runner(&v(&[r"C:\Program Files\nodejs\node.exe", r"C:\Program Files\nodejs\node_modules\npm\bin\npm-cli.js", "run", "dev"])).as_deref(), Some("npm run dev"));
        assert_eq!(runner(&v(&["node", "/usr/lib/node_modules/pnpm/bin/pnpm.cjs", "dev", "--port", "3000"])).as_deref(), Some("pnpm dev --port 3000"));
        assert_eq!(runner(&v(&[r"C:\Users\me\.bun\bin\bun.exe", "run", "start"])).as_deref(), Some("bun run start"));
        assert_eq!(runner(&v(&["node", "npm-cli.js", "install"])), None, "an install isn't a server");
        assert_eq!(runner(&v(&["node", "server.js"])), None);
        assert!(wrapper(&v(&["C:\\Windows\\system32\\cmd.exe", "/d", "/s", "/c", "vite"])));
        assert!(!wrapper(&v(&["powershell.exe", "-NoExit"])));
        assert_eq!(quoted(&v(&[r"C:\Program Files\nodejs\node.exe", "server.js", ""])), r#""C:\Program Files\nodejs\node.exe" server.js """#);
    }

    #[test]
    fn remembered_newest_first_without_repeats() {
        let dir = std::env::temp_dir().join(format!("mom-restart-{}", std::process::id()));
        let paths = Paths::under(&dir);
        let s = |id: &str, cmd: &str| Stopped { id: id.into(), name: "vite".into(), project: None, command: cmd.into(), cwd: r"C:\p".into(), ports: vec![5173], at: 1 };
        save(&paths, &[s("a", "npm run dev"), s("b", "npm run preview")]).unwrap();
        assert_eq!(list(&paths).len(), 2);
        forget(&paths, "a").unwrap();
        assert_eq!(list(&paths).iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["b"]);
        assert!(start(&paths, "nope").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
