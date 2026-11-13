//! uv's cache, and who is using it.
//!
//! Mom empties uv's cache with `uv cache clean`, the way she empties pip's and npm's: uv copies
//! (or hard-links) what an environment needs into the environment, so the environments uv made
//! keep working and uv downloads packages again when it needs them. Only when uv is set to
//! symlink environments into its cache does she merely prune it (`uv cache prune`).
//!
//! Either command takes uv's lock on the cache for itself, and every other uv command holds a
//! share of that lock for as long as it lives. A `uv run` stays alive for as long as the program
//! it started, so one long-lived server started that way makes uv wait for good. Mom looks at
//! who is running first: programs that live in the cache keep it, and uv wrappers that only wait
//! for a project's own program don't.

use super::TrashCtx;
use crate::paths::Paths;
use crate::procs::{Proc, ProcTable};
use crate::util::{file_name, norm_path};
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

const NOT_ON_PATH: &str = "uv isn't on your PATH, so mom can't run it.";
const BUSY_NOTE: &str = "uv is busy right now. Clean this when it's done.";
const BUSY_CLEAN: &str = "uv is busy right now, so mom left its cache alone. Clean again when it's done.";
const SOMEONE: &str = "Something is using uv's cache right now, so mom left it alone. Clean again in a minute.";
const LINKED_NOTE: &str = "uv is set to link environments into its cache, so mom only removes what nothing uses.";

/// Options of `uv` and `uv run` that take one value.
const WITH_VALUE: &[&str] = &[
    "--directory", "--project", "--python", "-p", "--package", "--extra", "--group", "--only-group", "--no-group",
    "--env-file", "--color", "--config-file", "--cache-dir", "--python-preference", "--allow-insecure-host",
];

/// Options of `uv` and `uv run` that take none.
const NO_VALUE: &[&str] = &[
    "-q", "--quiet", "-v", "--verbose", "--offline", "--no-progress", "--no-config", "--frozen", "--locked", "--no-sync",
    "--no-dev", "--all-extras", "--all-groups", "--no-env-file", "--native-tls", "--system-certs", "--managed-python",
    "--no-managed-python", "--no-python-downloads", "--module", "-m", "--no-editable", "--exact", "--active",
];

/// Where uv keeps its cache: the folder mom measures, probes and cleans.
pub(super) fn cache_dir(paths: &Paths) -> PathBuf {
    paths.local_appdata.join("uv").join("cache")
}

/// Who is running, as far as uv's cache goes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    /// Running programs whose own files are in the cache, or that a uv started in a way that may
    /// lean on it.
    pub from_cache: Vec<String>,
    /// uv processes doing something themselves right now.
    pub working: usize,
    /// Programs that uv started for a project and now only waits for.
    pub waiting_on: Vec<String>,
    /// Every uv process the sample saw, by (PID, start time), so the clean can tell a uv that
    /// started since.
    pub uvs: BTreeSet<(u32, u64)>,
}

/// What a clean does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Clear,
    /// Clear without waiting for uv's lock.
    ClearAnyway,
    /// Leave the cache alone, and say why.
    Stay(String),
}

/// How a uv process was started.
#[derive(Debug, PartialEq, Eq)]
enum Started {
    /// `uv run <command>` for a project, with options mom knows.
    Plain,
    /// Anything else: it may use the cache, so it counts as using it.
    Other,
    /// About another cache folder altogether.
    OtherCache,
}

/// Who in `table` is using the cache at `cache`.
pub fn seen(table: &ProcTable, cache: &Path) -> Seen {
    let roots = cache_roots(cache);
    let mine = table.self_and_ancestors();
    let mut from_cache = BTreeSet::new();
    let mut waiting_on = BTreeSet::new();
    let mut working = 0;
    let mut uvs = BTreeSet::new();
    for p in table.procs.values() {
        if is_uv(p) {
            uvs.insert((p.pid, p.start));
        }
        if mine.contains(&p.pid) {
            continue;
        }
        if inside(p, &roots) {
            from_cache.insert(stem(&p.name));
        }
        if !is_uv(p) {
            continue;
        }
        let started = started(&p.cmd, &roots);
        if started == Started::OtherCache {
            continue;
        }
        let kids: Vec<&Proc> = table
            .children(p.pid)
            .iter()
            .filter_map(|c| table.get(*c))
            .filter(|c| !matches!(stem(&c.name).to_ascii_lowercase().as_str(), "conhost" | "openconsole"))
            .collect();
        // A uv that started another uv is judged by that one.
        if kids.iter().any(|c| is_uv(c)) {
            continue;
        }
        let kids: Vec<&Proc> = kids.into_iter().filter(|c| !stem(&c.name).to_ascii_lowercase().starts_with("git")).collect();
        let names = kids.iter().map(|c| stem(&c.name));
        if kids.is_empty() {
            working += 1;
        } else if started == Started::Plain && kids.iter().all(|c| lives_outside(c, &roots)) {
            waiting_on.extend(names);
        } else {
            // Mom can't tell what such a program leans on, so the cache stays until it's closed.
            from_cache.extend(names);
        }
    }
    Seen { from_cache: named(from_cache), working, waiting_on: waiting_on.into_iter().collect(), uvs }
}

/// Has a uv started since the sample? The same PID counts as the same process when its start
/// times agree to within a second (two readings can differ by that much); a uv that has gone
/// since is fine.
fn new_uv(sampled: &BTreeSet<(u32, u64)>, now: &HashSet<(u32, u64)>) -> bool {
    now.iter().any(|(pid, start)| !sampled.iter().any(|(p, s)| p == pid && s.abs_diff(*start) <= 1))
}

/// Is uv's lock on `cache` held by someone? Without a lock file (older uv has none) it isn't.
/// Anything mom can't check counts as held.
pub fn held(cache: &Path) -> bool {
    let lock = cache.join(".lock");
    match std::fs::symlink_metadata(&lock) {
        Ok(m) if plain_file(&m) => {}
        Ok(_) => return true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
        Err(_) => return true,
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT: never follow a link swapped in
    }
    let Ok(file) = options.open(&lock) else { return true };
    if !file.metadata().is_ok_and(|m| plain_file(&m)) {
        return true;
    }
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(_) => true,
    }
}

/// What to do about the cache, given who is running and whether its lock is held.
pub fn step(seen: &Seen, held: bool) -> Step {
    if !seen.from_cache.is_empty() {
        let (names, many) = listed(&seen.from_cache);
        let (verb, them) = if many { ("are", "them") } else { ("is", "it") };
        Step::Stay(format!("{names} {verb} running from uv's cache, so mom left it alone. Close {them} and clean again."))
    } else if seen.working > 0 {
        Step::Stay(BUSY_CLEAN.into())
    } else if !held {
        Step::Clear
    } else if !seen.waiting_on.is_empty() {
        Step::ClearAnyway
    } else {
        Step::Stay(SOMEONE.into())
    }
}

/// What the look says when a clean would leave the cache alone, or only prune it. It never
/// touches the lock.
pub(super) fn note(seen: &Seen, linked: bool) -> Option<String> {
    if !seen.from_cache.is_empty() {
        let (names, many) = listed(&seen.from_cache);
        let (verb, them) = if many { ("are", "them") } else { ("is", "it") };
        Some(format!("{names} {verb} running from uv's cache. Close {them} to clean this."))
    } else if seen.working > 0 {
        Some(BUSY_NOTE.into())
    } else if linked {
        Some(LINKED_NOTE.into())
    } else {
        None
    }
}

/// Is uv set to symlink environments into its cache, here? Then emptying the cache would break
/// them. Mom reads the environment and the user's and machine's uv.toml; a project that sets
/// this in its own pyproject.toml or uv.toml isn't seen.
pub(super) fn linked_here(paths: &Paths) -> bool {
    let env = std::env::var_os("UV_LINK_MODE").map(|v| v.to_string_lossy().into_owned());
    let configs = [paths.appdata.join("uv").join("uv.toml"), paths.program_data.join("uv").join("uv.toml")];
    linked(env.as_deref(), &configs)
}

/// `linked_here`, from the variable's value and the config files. A config file mom can't read
/// counts as saying so; one that isn't there doesn't.
fn linked(env: Option<&str>, configs: &[PathBuf]) -> bool {
    if env.is_some_and(|v| v.trim().eq_ignore_ascii_case("symlink")) {
        return true;
    }
    configs.iter().any(|path| match std::fs::read_to_string(path) {
        Ok(text) => match text.parse::<toml::Table>() {
            // `[pip]` has its own, for `uv pip install`.
            Ok(t) => [Some(&t), t.get("pip").and_then(|p| p.as_table())]
                .into_iter()
                .flatten()
                .any(|t| t.get("link-mode").and_then(|v| v.as_str()) == Some("symlink")),
            Err(_) => true,
        },
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    })
}

/// The command line for a step that runs uv: only constants and mom's own cache folder. None for
/// a step that stays.
fn args(cache: &Path, linked: bool, step: &Step) -> Option<Vec<String>> {
    let force = match step {
        Step::Clear => false,
        // The only uv processes alive are `uv run` wrappers for projects, each waiting for a
        // program that doesn't live in the cache. Their programs have their own copies outside
        // it and nothing running lives in it, so emptying it needn't wait for them.
        Step::ClearAnyway => true,
        Step::Stay(_) => return None,
    };
    let verb = if linked { "prune" } else { "clean" };
    let mut out: Vec<String> = vec!["cache".into(), verb.into(), "--cache-dir".into(), cache.to_string_lossy().into_owned()];
    if force {
        out.push("--force".into());
    }
    Some(out)
}

/// What a clean did.
pub(super) struct Cleaned {
    /// Bytes that really came back.
    pub freed: u64,
    /// Mom only pruned, because uv links environments into its cache.
    pub linked: bool,
}

/// Empties (or, when environments link into it, prunes) the cache mom measured, when nothing in
/// it is in use.
pub(super) fn clean(ctx: &TrashCtx) -> Result<Cleaned, String> {
    let uv = crate::util::on_path("uv").ok_or(NOT_ON_PATH)?;
    let cache = cache_dir(ctx.paths);
    let linked = linked_here(ctx.paths);
    // Say why at once, before the slow count.
    if let Step::Stay(why) = step(&ctx.uv, held(&cache)) {
        return Err(why);
    }
    let before = only_here(&cache);
    // And ask again: the count takes a while, and a uv that started meanwhile isn't in the sample.
    match crate::procs::running_named_now(&["uv", "uvx"]) {
        None => return Err(SOMEONE.into()),
        Some(now) if new_uv(&ctx.uv.uvs, &now) => return Err(BUSY_CLEAN.into()),
        Some(_) => {}
    }
    let step = step(&ctx.uv, held(&cache));
    let Some(args) = args(&cache, linked, &step) else {
        return Err(match step { Step::Stay(why) => why, _ => SOMEONE.into() });
    };
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match crate::util::run(&uv.to_string_lossy(), &args, Duration::from_secs(600)) {
        Ok(out) if out.ok() => Ok(Cleaned { freed: before.saturating_sub(only_here(&cache)), linked }),
        Ok(out) if out.timed_out || waited(&out.err_text()) => Err(SOMEONE.into()),
        Ok(out) => Err(format!("uv cache {} failed: {}", args[1], last_line(&out.err_text()))),
        Err(e) => Err(format!("couldn't run uv: {e}")),
    }
}

/// uv's own files at the top of its cache. A clear leaves the lock, and uv puts the others back.
const MARKERS: &[&str] = &[".lock", ".gitignore", "CACHEDIR.TAG"];

/// The bytes of uv's marker files at the top of `cache` (plain files only), which the look leaves
/// out so an emptied cache reads as empty.
pub(super) fn marker_bytes(cache: &Path) -> u64 {
    MARKERS
        .iter()
        .filter_map(|name| std::fs::symlink_metadata(cache.join(name)).ok())
        .filter(|m| plain(m) && m.is_file())
        .map(|m| m.len())
        .sum()
}

/// The bytes in `cache` that live only there: files with one name on disk. A file that is also
/// hard-linked into an environment frees nothing when the cache's name for it goes. Links inside
/// are never followed; a cache folder that is itself a link is counted where it leads, as uv
/// cleans it there.
fn only_here(cache: &Path) -> u64 {
    let root = std::fs::canonicalize(cache).unwrap_or_else(|_| cache.to_path_buf());
    let mut total = 0;
    let mut folders = vec![root];
    while let Some(dir) = folders.pop() {
        let Ok(list) = std::fs::read_dir(&dir) else { continue };
        for e in list.flatten() {
            let path = e.path();
            let Ok(m) = std::fs::symlink_metadata(&path) else { continue };
            if !plain(&m) {
                continue;
            }
            if m.is_dir() {
                folders.push(path);
            } else if m.is_file() && one_name(&path) {
                total += m.len();
            }
        }
    }
    total
}

/// A file or folder, not a link or any other reparse point.
fn plain(m: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    !m.file_type().is_symlink()
}

/// Does the file have exactly one name on disk? Anything mom can't ask about doesn't count.
#[cfg(windows)]
fn one_name(path: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
    let file = std::fs::OpenOptions::new()
        .access_mode(0x80) // FILE_READ_ATTRIBUTES: enough to ask, even of a file in use
        .share_mode(0x7)
        .custom_flags(0x0020_0000) // FILE_FLAG_OPEN_REPARSE_POINT
        .open(path);
    let Ok(file) = file else { return false };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `info` is a valid output buffer and the file stays open for the call.
    unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) != 0 && info.nNumberOfLinks == 1 }
}

#[cfg(not(windows))]
fn one_name(_: &Path) -> bool {
    true
}

/// uv's own words for waiting on the cache, or for giving up waiting.
fn waited(err: &str) -> bool {
    let err = err.to_lowercase();
    err.contains("in-use") || err.contains("when waiting for lock")
}

fn last_line(text: &str) -> &str {
    text.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("")
}

/// A regular file, not a link or any other reparse point.
fn plain_file(m: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    m.file_type().is_file()
}

/// "notes-server.exe" is "notes-server".
fn stem(name: &str) -> String {
    let n = file_name(name);
    match n.len().checked_sub(4) {
        Some(at) if n.get(at..).is_some_and(|e| e.eq_ignore_ascii_case(".exe")) => n[..at].to_string(),
        _ => n.to_string(),
    }
}

fn is_uv(p: &Proc) -> bool {
    matches!(stem(&p.name).to_ascii_lowercase().as_str(), "uv" | "uvx")
}

fn pythonish(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "python" || n == "pythonw" || n.starts_with("python3")
}

/// The names to show: Python and console hosts only when there's nothing better to say.
fn named(stems: BTreeSet<String>) -> Vec<String> {
    let other: Vec<String> =
        stems.iter().filter(|n| !pythonish(n) && !n.eq_ignore_ascii_case("conhost")).cloned().collect();
    if !other.is_empty() {
        other
    } else if stems.iter().any(|n| pythonish(n)) {
        vec!["A Python program".into()]
    } else {
        stems.into_iter().collect()
    }
}

/// "a", "a and b", "a, b and c", "a, b and 3 more", and whether that's more than one.
fn listed(names: &[String]) -> (String, bool) {
    let text = if names.len() > 3 {
        format!("{}, {} and {} more", names[0], names[1], names.len() - 2)
    } else {
        super::and_list(&names.iter().map(String::as_str).collect::<Vec<_>>())
    };
    (text, names.len() > 1)
}

/// A path as compared: normalised, without the `\\?\` that `canonicalize` adds.
fn compared(p: &str) -> String {
    let p = match p.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => p.strip_prefix(r"\\?\").unwrap_or(p).to_string(),
    };
    norm_path(&p)
}

/// The path with every link resolved, or as spelled when that fails.
fn real(p: &str) -> String {
    std::fs::canonicalize(p).map(|r| compared(&r.to_string_lossy())).unwrap_or_else(|_| compared(p))
}

/// The cache folder as spelled and as it really is (they differ when it's a link): a path in
/// either counts as inside.
fn cache_roots(cache: &Path) -> Vec<String> {
    let spelled = compared(&cache.to_string_lossy());
    let real = real(&cache.to_string_lossy());
    if real == spelled { vec![spelled] } else { vec![spelled, real] }
}

fn under(path: &str, roots: &[String]) -> bool {
    roots.iter().any(|r| path.strip_prefix(r.as_str()).is_some_and(|rest| rest.starts_with('\\')))
}

/// Do the process's own files live in the cache? Its exe, or failing that the first word of its
/// command line, as spelled.
fn inside(p: &Proc, roots: &[String]) -> bool {
    match p.exe.as_deref().filter(|e| !e.is_empty()) {
        Some(exe) => under(&compared(exe), roots),
        None => p.cmd.first().is_some_and(|c| under(&compared(c.trim_matches('"')), roots)),
    }
}

/// Does mom know the program lives outside the cache? Without an exe path she doesn't.
fn lives_outside(p: &Proc, roots: &[String]) -> bool {
    // Links are resolved only here: opening every process's exe is slow with some antivirus, and
    // this is the one place a path spelled outside the cache decides whether to prune anyway.
    p.exe.as_deref().filter(|e| !e.is_empty()).is_some_and(|exe| !under(&compared(exe), roots) && !under(&real(exe), roots))
}

/// Reads a uv command line by what mom knows: known options, `run`, known options, then the
/// command. Anything else is not a plain project run.
fn started(cmd: &[String], roots: &[String]) -> Started {
    let Some((program, args)) = cmd.split_first() else { return Started::Other };
    if !stem(program.trim_matches('"')).eq_ignore_ascii_case("uv") {
        return Started::Other;
    }
    let mut run = false;
    let mut other_cache = false;
    let mut i = 0;
    while let Some(a) = args.get(i) {
        i += 1;
        if a.starts_with('-') {
            let (name, value) = match a.split_once('=') {
                Some((n, v)) if a.starts_with("--") => (n, Some(v.to_string())),
                _ => (a.as_str(), None),
            };
            if WITH_VALUE.contains(&name) {
                let value = match value {
                    Some(v) => v,
                    None => match args.get(i) {
                        Some(v) => {
                            i += 1;
                            v.clone()
                        }
                        None => return Started::Other,
                    },
                };
                if name == "--cache-dir" && !roots.contains(&compared(value.trim_matches('"'))) {
                    other_cache = true;
                }
            } else if value.is_some() || !NO_VALUE.contains(&name) {
                return Started::Other;
            }
        } else if !run && a == "run" {
            run = true;
        } else if !run || a.is_empty() {
            return Started::Other;
        } else {
            if other_cache {
                return Started::OtherCache;
            }
            let a = a.to_ascii_lowercase();
            // A script's environment may be a cached one, and so may one fetched from the web.
            let script = a.ends_with(".py") || a.ends_with(".pyw") || a.contains("://");
            return if script { Started::Other } else { Started::Plain };
        }
    }
    if other_cache { Started::OtherCache } else { Started::Other }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trash::tests::{fixture, link, unlink};

    const CACHE: &str = r"C:\Users\Someone\AppData\Local\uv\cache";
    const PROJECT: &str = r"C:\Users\Someone\tools\notes-server";

    fn pr(pid: u32, ppid: u32, name: &str, exe: Option<&str>, cmd: &[&str], start: u64) -> Proc {
        let mut p = crate::procs::fake(pid, ppid, name, "", start);
        p.exe = exe.map(String::from);
        p.cmd = cmd.iter().map(|s| s.to_string()).collect();
        p
    }

    fn uv(pid: u32, cmd: &[&str]) -> Proc {
        pr(pid, 1, "uv.exe", Some(r"C:\Users\Someone\.local\bin\uv.exe"), cmd, 10)
    }

    fn table(list: Vec<Proc>) -> ProcTable {
        ProcTable::from_procs(list, 9999)
    }

    fn seen_of(list: Vec<Proc>) -> Seen {
        seen(&table(list), Path::new(CACHE))
    }

    /// A uv with one child program that lives in the project, not the cache.
    fn wrapper(cmd: &[&str]) -> Seen {
        seen_of(vec![
            uv(100, cmd),
            pr(101, 100, "conhost.exe", None, &[], 11),
            pr(102, 100, "server.exe", Some(r"C:\Users\Someone\proj\.venv\Scripts\server.exe"), &["server"], 11),
        ])
    }

    #[test]
    fn nothing_running_is_nothing_seen() {
        assert_eq!(seen_of(vec![]), Seen::default());
    }

    #[test]
    fn project_wrappers_only_wait() {
        let mut list = Vec::new();
        for n in 0..6u32 {
            let base = 100 + n * 10;
            list.push(uv(base, &["uv", "--directory", PROJECT, "run", "notes-server"]));
            list.push(pr(base + 1, base, "conhost.exe", None, &[], 11));
            let exe = format!(r"{PROJECT}\.venv\Scripts\notes-server.exe");
            list.push(pr(base + 2, base, "notes-server.exe", Some(&exe), &[&exe], 11));
        }
        let s = seen_of(list);
        let uvs = (0..6u32).map(|n| (100 + n * 10, 10)).collect();
        assert_eq!(s, Seen { from_cache: vec![], working: 0, waiting_on: vec!["notes-server".into()], uvs });
    }

    #[test]
    fn the_sample_keeps_every_uv_it_saw() {
        let s = seen_of(vec![
            uv(100, &["uv", "--cache-dir", r"D:\other-cache", "run", "server"]),
            pr(101, 1, "UVX.EXE", None, &[], 12),
            pr(102, 1, "python.exe", None, &[], 12),
        ]);
        assert_eq!(s.uvs, BTreeSet::from([(100, 10), (101, 12)]));
    }

    #[cfg(windows)]
    #[test]
    fn a_fresh_look_finds_processes_by_name() {
        let me = std::env::current_exe().unwrap();
        let name = me.file_name().unwrap().to_string_lossy().to_uppercase();
        let stem = name.trim_end_matches(".EXE").to_string();
        for asked in [name.as_str(), stem.as_str()] {
            let found = crate::procs::running_named_now(&[asked]).expect("the snapshot works");
            assert!(found.iter().any(|(pid, _)| *pid == std::process::id()), "{asked}");
        }
        assert!(crate::procs::running_named_now(&["no-such-program-anywhere"]).unwrap().is_empty());
    }

    #[test]
    fn a_uv_started_since_the_sample_is_new() {
        let sampled = BTreeSet::from([(100, 50), (200, 60)]);
        let now = |list: &[(u32, u64)]| list.iter().copied().collect::<HashSet<_>>();
        assert!(!new_uv(&sampled, &now(&[(100, 50), (200, 60)])), "the same");
        assert!(!new_uv(&sampled, &now(&[(100, 50)])), "one gone");
        assert!(new_uv(&sampled, &now(&[(100, 50), (300, 70)])), "one new");
        assert!(new_uv(&sampled, &now(&[(100, 52), (200, 60)])), "a reused PID");
        assert!(!new_uv(&sampled, &now(&[(100, 51), (200, 59)])), "a second apart is the same one");
    }

    #[test]
    fn a_uv_with_only_a_console_is_working() {
        let s = seen_of(vec![
            uv(100, &["uv", "sync"]),
            pr(101, 100, "conhost.exe", None, &[], 11),
            pr(102, 100, "git.exe", None, &["git", "fetch"], 11),
        ]);
        assert_eq!(s.working, 1);
        assert!(s.waiting_on.is_empty() && s.from_cache.is_empty());
    }

    #[test]
    fn a_tool_in_the_cache_is_named_and_its_uvx_is_not_working() {
        let tool = format!(r"{CACHE}\archive-v0\abc\Scripts\ruff-lsp.exe");
        let s = seen_of(vec![
            pr(100, 1, "uvx.exe", Some(r"C:\bin\uvx.exe"), &["uvx", "ruff-lsp"], 10),
            pr(101, 100, "uv.exe", Some(r"C:\bin\uv.exe"), &["uv", "tool", "uvx", "ruff-lsp"], 11),
            pr(102, 101, "ruff-lsp.exe", Some(&tool), &[&tool], 12),
        ]);
        assert_eq!(s.from_cache, vec!["ruff-lsp".to_string()]);
        assert_eq!(s.working, 0);
        assert_eq!(step(&s, false), Step::Stay(
            "ruff-lsp is running from uv's cache, so mom left it alone. Close it and clean again.".into()));
    }

    #[test]
    fn only_python_in_the_cache_is_a_python_program() {
        let py = format!(r"{CACHE}\environments-v2\x\Scripts\python.exe");
        let s = seen_of(vec![pr(100, 1, "python.exe", Some(&py), &["python", "app.py"], 10)]);
        assert_eq!(s.from_cache, vec!["A Python program".to_string()]);
        // and without an exe, the first word of the command line says where it runs from
        let s = seen_of(vec![pr(100, 1, "python3.12.exe", None, &[&format!("\"{py}\""), "app.py"], 10)]);
        assert_eq!(s.from_cache, vec!["A Python program".to_string()]);
    }

    #[test]
    fn a_sibling_folder_with_the_same_start_is_outside() {
        let exe = format!(r"{CACHE}-old\tool.exe");
        assert_eq!(seen_of(vec![pr(100, 1, "tool.exe", Some(&exe), &[], 10)]), Seen::default());
    }

    #[test]
    fn a_reused_pid_is_not_a_child() {
        // 101 started before its "parent" 100, so the real parent is gone: 100 has no children
        let s = seen_of(vec![
            uv(100, &["uv", "run", "server"]),
            pr(101, 100, "server.exe", Some(r"C:\proj\server.exe"), &["server"], 5),
        ]);
        assert_eq!(s.working, 1);
        assert!(s.waiting_on.is_empty());
    }

    #[test]
    fn extras_and_odd_starts_count_as_from_the_cache() {
        for cmd in [
            &["uv", "run", "--with", "x", "server"][..],
            &["uv", "run", "--with=x", "server"],
            &["uv", "run", "--isolated", "server"],
            &["uv", "tool", "run", "x"],
            &["uv", "run", "script.py"],
            &["uv", "run", "Script.PYW"],
            &["uv", "run", "https://example.com/tool"],
            &["uv", "run", "--frobnicate", "server"],
            &["uv", "--quiet=1", "run", "server"],
            &["uv", "run"],
            &["uvx", "server"],
            &[],
        ] {
            let s = wrapper(cmd);
            assert_eq!(s.from_cache, vec!["server".to_string()], "{cmd:?}");
            assert!(s.waiting_on.is_empty(), "{cmd:?}");
        }
    }

    #[test]
    fn plain_project_runs_wait() {
        let eq = format!("--directory={PROJECT}");
        for cmd in [
            &["uv", "--directory", PROJECT, "run", "server"][..],
            &["uv", &eq, "run", "server"],
            &[r"C:\bin\UV.EXE", "-q", "run", "--frozen", "-p", "3.12", "-m", "server"],
            &["uv", "run", "server", "--with-cache", "--isolated", "x.py"],
            &["uv", "--cache-dir", CACHE, "run", "server"],
        ] {
            let s = wrapper(cmd);
            assert_eq!(s.waiting_on, vec!["server".to_string()], "{cmd:?}");
            assert!(s.from_cache.is_empty(), "{cmd:?}");
        }
    }

    #[test]
    fn a_wrapper_child_mom_cannot_place_keeps_the_cache() {
        let s = seen_of(vec![
            uv(100, &["uv", "run", "server"]),
            pr(101, 100, "server.exe", None, &["server"], 11),
            pr(102, 100, "helper.exe", Some(r"C:\proj\helper.exe"), &["helper"], 11),
        ]);
        assert_eq!(s.from_cache, vec!["helper".to_string(), "server".to_string()]);
        assert!(s.waiting_on.is_empty());
        // and one whose exe is in the cache is from the cache, not waited on
        let tool = format!(r"{CACHE}\builds-v0\x\Scripts\server.exe");
        let s = seen_of(vec![uv(100, &["uv", "run", "server"]), pr(101, 100, "server.exe", Some(&tool), &["server"], 11)]);
        assert_eq!(s.from_cache, vec!["server".to_string()]);
        assert!(s.waiting_on.is_empty());
    }

    #[test]
    fn a_uv_for_another_cache_is_left_out() {
        // only the list of uvs the sample saw has it
        let alone = Seen { uvs: BTreeSet::from([(100, 10)]), ..Default::default() };
        assert_eq!(wrapper(&["uv", "--cache-dir", r"D:\other-cache", "run", "server"]), alone);
        assert_eq!(wrapper(&["uv", "run", r"--cache-dir=D:\other-cache", "server"]), alone);
    }

    #[test]
    fn a_program_reached_through_a_link_into_the_cache_is_from_the_cache() {
        let root = fixture("uv-linked-venv");
        let cache = root.join("cache");
        let env = cache.join("environments-v2").join("proj");
        let exe = env.join("Scripts").join("server.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"x").unwrap();
        let venv = root.join("proj").join(".venv");
        std::fs::create_dir_all(venv.parent().unwrap()).unwrap();
        link(&venv, &env);
        let spelled = venv.join("Scripts").join("server.exe").to_string_lossy().to_string();
        let t = table(vec![
            uv(100, &["uv", "run", "server"]),
            pr(101, 100, "server.exe", Some(&spelled), &["server"], 11),
        ]);
        let s = seen(&t, &cache);
        unlink(&venv);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(s.from_cache, vec!["server".to_string()]);
    }

    #[test]
    fn steps_in_order() {
        let from = Seen { from_cache: vec!["tool".into()], working: 1, waiting_on: vec!["server".into()], ..Default::default() };
        assert!(matches!(step(&from, false), Step::Stay(_)), "in use wins even when the lock is free");
        let busy = Seen { working: 1, waiting_on: vec!["server".into()], ..Default::default() };
        assert_eq!(step(&busy, true), Step::Stay(BUSY_CLEAN.into()), "working wins over waiting");
        assert_eq!(step(&busy, false), Step::Stay(BUSY_CLEAN.into()));
        let waiting = Seen { waiting_on: vec!["server".into()], ..Default::default() };
        assert_eq!(step(&waiting, false), Step::Clear);
        assert_eq!(step(&waiting, true), Step::ClearAnyway);
        assert_eq!(step(&Seen::default(), false), Step::Clear);
        assert_eq!(step(&Seen::default(), true), Step::Stay(SOMEONE.into()));
    }

    #[test]
    fn notes_only_when_the_clean_would_stay_or_only_prune() {
        let waiting = Seen { waiting_on: vec!["server".into()], ..Default::default() };
        assert_eq!(note(&waiting, false), None);
        assert_eq!(note(&waiting, true).as_deref(), Some(LINKED_NOTE));
        assert_eq!(note(&Seen { working: 2, ..Default::default() }, true).as_deref(), Some(BUSY_NOTE), "busy comes first");
    }

    #[test]
    fn four_command_shapes() {
        let cache = Path::new(r"C:\Users\Someone\AppData\Local\uv\cache");
        let shape = |linked, step: Step| args(cache, linked, &step).map(|a| a.join(" "));
        let c = cache.to_string_lossy();
        assert_eq!(shape(false, Step::Clear), Some(format!("cache clean --cache-dir {c}")));
        assert_eq!(shape(false, Step::ClearAnyway), Some(format!("cache clean --cache-dir {c} --force")));
        assert_eq!(shape(true, Step::Clear), Some(format!("cache prune --cache-dir {c}")));
        assert_eq!(shape(true, Step::ClearAnyway), Some(format!("cache prune --cache-dir {c} --force")));
        assert_eq!(shape(false, Step::Stay("x".into())), None);
        // the folder stays one argument, spaces and all
        let spaced = Path::new(r"C:\Users\Some One\uv cache");
        assert_eq!(args(spaced, false, &Step::Clear).unwrap()[3], r"C:\Users\Some One\uv cache");
    }

    #[test]
    fn linked_from_the_variable() {
        assert!(linked(Some("symlink"), &[]));
        assert!(linked(Some("Symlink "), &[]));
        assert!(!linked(Some("hardlink"), &[]));
        assert!(!linked(None, &[]));
    }

    #[test]
    fn linked_from_the_config_files() {
        let root = fixture("uv-link-config");
        std::fs::create_dir_all(&root).unwrap();
        let file = |name: &str, text: &str| {
            let p = root.join(name);
            std::fs::write(&p, text).unwrap();
            p
        };
        let sym = file("sym.toml", "link-mode = \"symlink\"\n");
        let copy = file("copy.toml", "link-mode = \"copy\"\ncache-dir = \"D:/x\"\n");
        let pip = file("pip.toml", "[pip]\nlink-mode = \"symlink\"\n");
        let tool = file("tool.toml", "[tool]\nlink-mode = \"symlink\"\n");
        let broken = file("broken.toml", "link-mode = \n");
        let missing = root.join("missing.toml");
        let folder = root.join("folder.toml");
        std::fs::create_dir_all(&folder).unwrap();
        let results = [
            linked(None, &[missing.clone(), sym]),
            linked(None, &[copy]),
            linked(None, &[pip]),
            linked(None, &[tool]),
            linked(None, &[broken]),
            linked(None, &[missing]),
            linked(None, &[folder]),
        ];
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(results, [true, false, true, false, true, false, true]);
    }

    #[test]
    fn only_files_that_live_only_in_the_cache_count() {
        let root = fixture("uv-only-here");
        let cache = root.join("cache");
        let outside = root.join("outside");
        std::fs::create_dir_all(cache.join("wheels")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(cache.join("wheels").join("only.bin"), [0u8; 5]).unwrap();
        std::fs::write(outside.join("shared.bin"), [0u8; 7]).unwrap();
        std::fs::hard_link(outside.join("shared.bin"), cache.join("wheels").join("shared.bin")).unwrap();
        std::fs::write(outside.join("elsewhere.bin"), [0u8; 11]).unwrap();
        link(&cache.join("env"), &outside);
        let n = only_here(&cache);
        unlink(&cache.join("env"));
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(n, 5);
    }

    #[test]
    fn no_lock_file_is_not_held() {
        let root = fixture("uv-lock-missing");
        std::fs::create_dir_all(&root).unwrap();
        assert!(!held(&root));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_shared_lock_is_held_until_it_goes() {
        let root = fixture("uv-lock-held");
        std::fs::create_dir_all(&root).unwrap();
        let lock = root.join(".lock");
        std::fs::write(&lock, b"").unwrap();
        assert!(!held(&root));
        let other = std::fs::File::open(&lock).unwrap();
        other.lock_shared().unwrap();
        let while_held = held(&root);
        drop(other);
        let after = held(&root);
        let _ = std::fs::remove_dir_all(&root);
        assert!(while_held);
        assert!(!after);
    }

    #[test]
    fn a_lock_that_is_a_folder_counts_as_held() {
        let root = fixture("uv-lock-folder");
        std::fs::create_dir_all(root.join(".lock")).unwrap();
        let h = held(&root);
        let _ = std::fs::remove_dir_all(&root);
        assert!(h);
    }

    #[test]
    fn sentences() {
        let one = Seen { from_cache: vec!["ruff".into()], ..Default::default() };
        assert_eq!(note(&one, true).unwrap(), "ruff is running from uv's cache. Close it to clean this.");
        let two = Seen { from_cache: vec!["a".into(), "b".into()], ..Default::default() };
        assert_eq!(note(&two, false).unwrap(), "a and b are running from uv's cache. Close them to clean this.");
        assert_eq!(step(&two, false),
            Step::Stay("a and b are running from uv's cache, so mom left it alone. Close them and clean again.".into()));
        let five = Seen { from_cache: ["a", "b", "c", "d", "e"].map(String::from).to_vec(), ..Default::default() };
        assert_eq!(note(&five, false).unwrap(), "a, b and 3 more are running from uv's cache. Close them to clean this.");
        assert_eq!(listed(&["a", "b", "c"].map(String::from)).0, "a, b and c");
        assert_eq!(LINKED_NOTE, "uv is set to link environments into its cache, so mom only removes what nothing uses.");
    }

    #[test]
    fn uvs_own_markers_at_the_top_dont_count() {
        let root = fixture("uv-markers");
        let paths = Paths::under(&root);
        let cache = cache_dir(&paths);
        std::fs::create_dir_all(cache.join("wheels-v5").join("pkg")).unwrap();
        std::fs::write(cache.join(".lock"), [0u8; 0]).unwrap();
        std::fs::write(cache.join(".gitignore"), [0u8; 1]).unwrap();
        std::fs::write(cache.join("CACHEDIR.TAG"), [0u8; 43]).unwrap();
        let cfg = crate::config::TrashConfig::default();
        let ctx = crate::trash::tests::ctx(&paths, &cfg);
        let measured = || {
            let s = crate::trash::measure("uv-cache", &ctx).unwrap();
            (s.total_bytes, s.reclaimable_bytes)
        };
        let only_markers = measured();
        std::fs::write(cache.join("wheels-v5").join("pkg").join("a.whl"), [0u8; 100]).unwrap();
        let with_a_file = measured();
        std::fs::write(cache.join("wheels-v5").join("pkg").join(".gitignore"), [0u8; 5]).unwrap();
        let with_a_deeper_marker = measured();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(only_markers, (0, 0));
        assert_eq!(with_a_file, (100, 100));
        assert_eq!(with_a_deeper_marker, (105, 105));
    }

    #[test]
    fn the_spot_says_what_the_clean_does() {
        let root = fixture("uv-spot-words");
        let paths = Paths::under(&root);
        std::fs::create_dir_all(cache_dir(&paths)).unwrap();
        std::fs::write(cache_dir(&paths).join("a.whl"), [0u8; 3]).unwrap();
        let cfg = crate::config::TrashConfig::default();
        let ctx = crate::trash::tests::ctx(&paths, &cfg);
        let spot = crate::trash::measure("uv-cache", &ctx).unwrap();
        let linked = linked_here(&paths);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(spot.detail, "Runs uv's own cache clean-up. uv downloads packages again when it needs them.");
        assert_eq!(spot.what, "Python packages uv has downloaded and built, kept so the next install is faster.");
        if !linked {
            assert_eq!(spot.effect, "Runs `uv cache clean`, which empties uv's cache (up to 3 B), permanently (it doesn't go \
                to the Recycle Bin). Environments uv already made keep working; uv downloads packages again when it needs them.");
        }
    }

    #[test]
    fn waiting_is_read_from_uvs_words() {
        assert!(waited("Cache is currently in-use, waiting for other uv processes to finish (use `--force` to override)"));
        assert!(waited("error: Timeout (300s) when waiting for lock on `C:\\x\\.lock`"));
        assert!(!waited("error: Access is denied."));
        assert_eq!(last_line("warning: a\n  error: b  \n\n"), "error: b");
    }
}
