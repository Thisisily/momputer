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
    /// What ran, in `cwd`, the way you'd type it: "npm run dev", or the program and its
    /// arguments. For reading; what's started again is `argv`.
    pub command: String,
    /// The program and each of its arguments, exactly as the server had them. Starting from
    /// these, nothing in an argument (the `&` in an address, say) is ever read as a command.
    /// Empty for one an older mom remembered.
    #[serde(default)]
    pub argv: Vec<String>,
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
        let Some(Start { command, argv, cwd }) = start_of(table, g) else { continue };
        all.retain(|s| !(s.command == command && s.cwd.eq_ignore_ascii_case(&cwd)));
        all.insert(
            0,
            Stopped {
                id: format!("{}-{}", g.root_pid, g.root_start),
                name: g.name.clone(),
                project: g.project.clone(),
                command,
                argv,
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
    open_terminal(s)?;
    let _ = forget(paths, id);
    Ok(format!("Started {} again, in a new window.", s.name))
}

/// The rest of cmd's command line, and the variables to give it.
#[cfg(windows)]
type ForCmd = (String, Vec<(&'static str, String)>);

/// What to hand cmd so that it runs a server again: the rest of its command line, and the
/// variables to give it. `how` is `/k` (stay open afterwards) or, in tests, `/c`.
///
/// The program and its arguments reach cmd in two variables that it only opens up after it has
/// read the line (`/v:on` and `!NAME!`), so whatever is in them is handed on as it is. Written
/// into the line itself, the `&` in an address would end the command and start another one.
#[cfg(windows)]
fn for_cmd(s: &Stopped, how: &str) -> Result<ForCmd, String> {
    let by_hand = || format!("Mom can't hand this one to a terminal safely, so starting it is yours: {}", s.command);
    match s.argv.split_first() {
        Some((program, args)) => {
            // A batch file reads its arguments as commands all over again, so only plain ones go
            // to one. And nothing with a line break in it survives the trip.
            let batch = [".cmd", ".bat"].iter().any(|e| program.to_ascii_lowercase().ends_with(e));
            if program.contains(['"', '\r', '\n']) || args.iter().any(|a| a.contains(['\r', '\n']) || (batch && !plain(a))) {
                return Err(by_hand());
            }
            let line = format!(r#"/v:on /s {how} ""!MOM_START_PROGRAM!" !MOM_START_ARGUMENTS!""#);
            Ok((line, vec![("MOM_START_PROGRAM", program.clone()), ("MOM_START_ARGUMENTS", command_line(args))]))
        }
        // One an older mom remembered, as a line: only if cmd reads it the way it's written.
        None if plain(&s.command.replace('"', "")) => Ok((format!("{how} {}", s.command), vec![])),
        None => Err(by_hand()),
    }
}

/// Runs a server again in a terminal of its own, in its folder: a new console (in your default
/// terminal) that stays open when the server stops, so you can read why. Started by Windows
/// directly and handed nothing of mom's, so the window is the terminal's own whatever mom
/// herself was started from. Gives back the terminal's process.
#[cfg(windows)]
fn open_terminal(s: &Stopped) -> Result<std::os::windows::io::OwnedHandle, String> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    use windows_sys::Win32::System::Threading::{CreateProcessW, CREATE_NEW_CONSOLE, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTUPINFOW};
    let (line, ours) = for_cmd(s, "/k")?;
    let wide = |text: &str| -> Vec<u16> { text.encode_utf16().chain(Some(0)).collect() };
    // Windows' own cmd, by its whole path: never one that happens to lie in some folder on the way.
    let mut system = [0u16; 260];
    // SAFETY: the buffer is ours and its length is given.
    let n = unsafe { GetSystemDirectoryW(system.as_mut_ptr(), system.len() as u32) } as usize;
    if n == 0 || n >= system.len() {
        return Err("Windows couldn't start it: its own folder wasn't found.".into());
    }
    let cmd = wide(&format!(r"{}\cmd.exe", String::from_utf16_lossy(&system[..n])));
    let mut line = wide(&format!("cmd.exe {line}"));
    let dir = wide(&s.cwd);
    // Mom's own variables with the two added: NAME=value after NAME=value, each ended by a NUL,
    // and one more NUL to end the lot.
    let mut block: Vec<u16> = Vec::new();
    let mine = std::env::vars_os().filter(|(name, _)| !ours.iter().any(|(k, _)| name.eq_ignore_ascii_case(k)));
    for (name, value) in mine.chain(ours.iter().map(|(k, v)| (k.into(), v.into()))) {
        block.extend(name.encode_wide().chain(Some(b'=' as u16)).chain(value.encode_wide()).chain(Some(0)));
    }
    block.push(0);
    // SAFETY: zeroed is how both structures start out; every pointer is to something of ours
    // that outlives the call (the command line is one Windows may write into, so it's mutable);
    // the two handles it returns are ours to close.
    unsafe {
        let startup = STARTUPINFOW { cb: std::mem::size_of::<STARTUPINFOW>() as u32, ..std::mem::zeroed() };
        let mut made: PROCESS_INFORMATION = std::mem::zeroed();
        let flags = CREATE_NEW_CONSOLE | CREATE_UNICODE_ENVIRONMENT;
        if CreateProcessW(cmd.as_ptr(), line.as_mut_ptr(), std::ptr::null(), std::ptr::null(), 0, flags, block.as_ptr().cast(), dir.as_ptr(), &startup, &mut made) == 0 {
            return Err(format!("Windows couldn't start it: {}", std::io::Error::last_os_error()));
        }
        CloseHandle(made.hThread);
        Ok(std::os::windows::io::OwnedHandle::from_raw_handle(made.hProcess as _))
    }
}

#[cfg(not(windows))]
fn open_terminal(s: &Stopped) -> Result<std::process::Child, String> {
    let (program, args) = s.argv.split_first().ok_or_else(|| format!("Start it yourself: {}", s.command))?;
    std::process::Command::new(program).args(args).current_dir(&s.cwd).spawn().map_err(|e| format!("It didn't start: {e}"))
}

/// Does cmd read this the way it's written? Not if it holds one of the characters cmd acts on.
#[cfg(windows)]
fn plain(text: &str) -> bool {
    !text.contains(['&', '|', '<', '>', '^', '%', '!', '(', ')', '"', '\r', '\n'])
}

/// Arguments as one line that a Windows program reads back into the very same arguments.
#[cfg(windows)]
fn command_line(args: &[String]) -> String {
    let slashes = |n: usize| "\\".repeat(n);
    args.iter()
        .map(|a| {
            if !a.is_empty() && !a.contains([' ', '\t', '"']) {
                return a.clone();
            }
            // In quotes; a quote inside is written \" and the backslashes right before one (or
            // before the closing quote) are doubled, so they stay backslashes.
            let (mut out, mut run) = (String::from('"'), 0);
            for c in a.chars() {
                match c {
                    '\\' => run += 1,
                    '"' => {
                        out += &slashes(run * 2 + 1);
                        out.push('"');
                        run = 0;
                    }
                    c => {
                        out += &slashes(run);
                        out.push(c);
                        run = 0;
                    }
                }
            }
            out + &slashes(run * 2) + "\""
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// How a group was started: what to show, what to run, and where.
#[derive(Clone, Debug, PartialEq)]
pub struct Start {
    pub command: String,
    pub argv: Vec<String>,
    pub cwd: String,
}

/// The command that started a group, and its folder. Climbs from the group's first process
/// through `cmd /c` and `sh -c` wrappers to the package manager that ran it, if one did: what
/// you typed ("npm run dev") rather than what it became (node with a long path).
pub fn start_of(table: &ProcTable, g: &Group) -> Option<Start> {
    start_from(table, g.root_pid, g.cwd.as_ref())
}

fn start_from(table: &ProcTable, root_pid: u32, cwd: Option<&String>) -> Option<Start> {
    let root = table.get(root_pid)?;
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
    let cwd = best.cwd.clone().or_else(|| cwd.cloned())?;
    (!command.trim().is_empty() && best.cmd.first().is_some_and(|program| !program.trim().is_empty())).then(|| Start { command, argv: best.cmd.clone(), cwd })
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

/// Arguments back into one line to read, quoting the ones with spaces. (Not what's run: see `Stopped.argv`.)
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
        let s = |id: &str, cmd: &str| Stopped { id: id.into(), name: "vite".into(), project: None, command: cmd.into(), argv: v(&cmd.split(' ').collect::<Vec<_>>()), cwd: r"C:\p".into(), ports: vec![5173], at: 1 };
        save(&paths, &[s("a", "npm run dev"), s("b", "npm run preview")]).unwrap();
        assert_eq!(list(&paths).len(), 2);
        forget(&paths, "a").unwrap();
        assert_eq!(list(&paths).iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["b"]);
        assert!(start(&paths, "nope").is_err());
        // One an older mom wrote down has no arguments of its own, and still reads.
        std::fs::write(file(&paths), r#"[{"id":"old","name":"vite","project":null,"command":"npm run dev","cwd":"C:\\p","ports":[5173],"at":1}]"#).unwrap();
        assert_eq!(list(&paths)[0].argv, Vec::<String>::new());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// What `open_terminal` hands cmd, run to the end with what it printed caught instead of
    /// in a window of its own.
    #[cfg(windows)]
    fn through_cmd(s: &Stopped) -> Result<std::process::Command, String> {
        use std::os::windows::process::CommandExt;
        let (line, ours) = for_cmd(s, "/c")?;
        let mut cmd = std::process::Command::new("cmd.exe");
        cmd.raw_arg(line).envs(ours).current_dir(&s.cwd);
        Ok(cmd)
    }

    /// Not a test of anything: run by the test below as a program, it prints what it was given.
    #[test]
    fn prints_what_it_was_given() {
        let given: Vec<String> = std::env::args().skip_while(|a| a != "restart::tests::prints_what_it_was_given").skip(1).collect();
        println!("GIVEN {}", serde_json::to_string(&given).unwrap());
    }

    #[test]
    #[cfg(windows)]
    fn every_argument_arrives_as_it_was_whatever_is_in_it() {
        let me = std::env::current_exe().unwrap().to_string_lossy().to_string();
        let dir = std::env::temp_dir().join(format!("mom-restart-args & 100% (odd) ^ it's-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let stopped = |argv: Vec<String>| Stopped { id: "a".into(), name: "server".into(), project: None, command: quoted(&argv), argv, cwd: dir.to_string_lossy().to_string(), ports: vec![], at: 1 };
        // The test program run as a server: it prints its arguments after the test's own name.
        let run = |args: &[&str]| {
            let mut argv = v(&[&me, "--exact", "--nocapture", "--", "restart::tests::prints_what_it_was_given"]);
            argv.extend(v(args));
            let out = through_cmd(&stopped(argv)).unwrap().output().unwrap();
            let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
            let given = text.lines().find_map(|l| l.split_once("GIVEN ").map(|(_, json)| serde_json::from_str::<Vec<String>>(json.trim()).unwrap()));
            (given, text)
        };
        for args in [
            vec!["https://example.test/?a=1&echo SECOND_COMMAND_RAN"],
            vec!["a&b", "c|d", "e<f", "g>h", "i^j", "(k)", "100%", "%PATH%", "!PATH!", "wow!", "it's"],
            vec!["with space", "", r#"say "hi""#, r"back\slash\", r"trail space\", r#"q"&whoami&""#],
            vec!["^&", "&&", "||", "a;b", "a,b", "a=b", "@x", "%%", "!!", "^^!", "%OS%!OS!"],
            vec!["3000", r"C:\Program Files\App Dir\x.js", "naïve café", "tab\there"],
        ] {
            let (given, text) = run(&args);
            assert_eq!(given.as_deref(), Some(&v(&args)[..]), "{text}");
            assert!(!text.lines().any(|l| l.trim() == "SECOND_COMMAND_RAN"), "{text}");
        }
        // A batch file reads its arguments as commands again, so only plain ones are handed to one;
        // a line break can't make the trip; and neither is run some other way instead.
        for (argv, fine) in [
            (vec![r"C:\tools\serve.cmd", "--port", "3000"], true),
            (vec![r"C:\tools\serve.cmd", "https://example.test/?a=1&b=2"], false),
            (vec![r"C:\tools\Serve.BAT", "100%"], false),
            (vec![r"C:\tools\serve.exe", "two\nlines"], false),
        ] {
            let made = through_cmd(&stopped(v(&argv)));
            assert_eq!(made.is_ok(), fine, "{argv:?}");
            assert!(fine || made.unwrap_err().contains("starting it is yours"));
        }
        // What an older mom remembered as one line is run only if cmd reads it the way it's written.
        let old = |command: &str| through_cmd(&Stopped { argv: vec![], command: command.into(), ..stopped(vec![]) }).is_ok();
        assert!(old("npm run dev") && old(r#""C:\Program Files\nodejs\node.exe" server.js --port 3000"#));
        assert!(!old("node server.js https://example.test/?a=1&echo SECOND_COMMAND_RAN") && !old("node %TEMP%\\x.js") && !old("a | b"));
        assert_eq!(command_line(&v(&["plain", "", "two words", r#"a"b"#, r"end\", r"sp ace\", r#"\\"q"#])), r#"plain "" "two words" "a\"b" end\ "sp ace\\" "\\\\\"q""#);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Opens a real console window for a moment, so it's run by hand:
    /// cargo test -p mom-core the_window_stays_open -- --ignored
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn the_window_stays_open_when_the_server_stops() {
        let windows = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let argv = v(&[&format!(r"{windows}\System32\whoami.exe"), "/?", "a&b"]);
        let s = Stopped { id: "a".into(), name: "server".into(), project: None, command: quoted(&argv), argv, cwd: windows, ports: vec![], at: 1 };
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::Threading::{TerminateProcess, WaitForSingleObject};
        let window = open_terminal(&s).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(2));
        // The program finished long ago; the terminal it ran in is still there to be read.
        // SAFETY: the handle is the terminal's process, open until `window` is dropped.
        let still_open = unsafe { WaitForSingleObject(window.as_raw_handle() as _, 0) } == 258;
        unsafe { TerminateProcess(window.as_raw_handle() as _, 0) };
        assert!(still_open);
    }

    #[test]
    fn what_is_started_again_is_the_program_and_its_arguments_as_they_were() {
        use crate::procs::ProcTable;
        let proc_of = |pid: u32, ppid: u32, cmd: &[&str]| Proc { pid, ppid: Some(ppid), cmd: v(cmd), cwd: Some(r"C:\Workspace\storefront".into()), ..Default::default() };
        let procs = vec![
            proc_of(10, 1, &[r"C:\Program Files\nodejs\node.exe", r"C:\Program Files\nodejs\node_modules\npm\bin\npm-cli.js", "run", "dev", "--", "--open", "https://example.test/?a=1&b=2"]),
            proc_of(11, 10, &[r"C:\Windows\system32\cmd.exe", "/d", "/s", "/c", "vite --open"]),
            proc_of(12, 11, &["node", r"C:\Workspace\storefront\node_modules\vite\bin\vite.js", "--open"]),
        ];
        let table = ProcTable::from_procs(procs, 999);
        let start = start_from(&table, 12, None).unwrap();
        // Shown the way you'd type it; started from the arguments themselves, the & and all.
        assert_eq!(start.command, "npm run dev -- --open https://example.test/?a=1&b=2");
        assert_eq!(start.argv, table.get(10).unwrap().cmd);
        assert_eq!(start.cwd, r"C:\Workspace\storefront");
    }
}
