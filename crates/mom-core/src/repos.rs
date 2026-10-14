//! Work that exists only on this PC: projects in your code folders whose latest work isn't
//! anywhere else yet. A project with no remote at all, commits that were never pushed, changes
//! never committed, stashes set aside and forgotten. If the drive fails, that work goes with it.
//!
//! Mom only asks git, and only questions: `for-each-ref`, `log`, `status` and `stash list`. A
//! project's own settings can name programs for git to run (a filter for every file it
//! compares, an fsmonitor, a signature checker) or pull in other settings files, so git is only
//! asked about a project whose settings are all ones mom knows are plain data; any other is
//! left alone. Each question runs with --no-optional-locks (so git doesn't even refresh its
//! index), with a time limit, and several projects at a time.

use crate::paths::Paths;
use crate::speed::Fix;
use crate::system::{Check, Item};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// At most this many projects are looked at, found this deep under each code folder.
const MAX_REPOS: usize = 300;
const DEPTH: usize = 3;
/// Each question to git, and the whole look.
const GIT_TIME: Duration = Duration::from_secs(5);
const ALL_TIME: Duration = Duration::from_secs(30);
const WORKERS: usize = 8;
/// Changes nobody has touched in a week, and a stash nobody has come back to in two, are
/// forgotten rather than in progress.
const OLD_CHANGES: u64 = 7 * 86_400;
const OLD_STASH: u64 = 14 * 86_400;
/// Changed files whose dates are read, per project.
const MAX_CHANGED: usize = 500;
/// Projects with a finding of their own; the rest share one.
const MAX_CHECKS: usize = 12;

/// Folders that are never a project's root, and never worth walking into.
const SKIP: &[&str] = &["node_modules", "target", "dist", "build", ".venv", "venv", "__pycache__", ".cache", ".next", "vendor"];

/// The projects (git repositories) in the places code is kept: the usual code folders in your
/// home and at the top of each drive, and your rooms.
pub fn find(paths: &Paths) -> Vec<PathBuf> {
    let rooms = crate::config::Config::load(paths).rooms;
    let mut roots = crate::trash::project_roots(paths, &rooms);
    // A made-up layout (tests) looks only inside itself, never at the drives of this PC.
    if let Some(fixture) = paths.system_drive.parent() {
        roots.retain(|r| r.starts_with(fixture));
    }
    find_in(&roots)
}

/// The repositories under `roots`, `DEPTH` folders down at most. A repository's own folders
/// aren't searched (the projects inside one are its business), and links aren't followed.
pub fn find_in(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut queue: std::collections::VecDeque<(PathBuf, usize)> = roots.iter().map(|r| (r.clone(), 0)).collect();
    while let Some((dir, depth)) = queue.pop_front() {
        if out.len() >= MAX_REPOS {
            break;
        }
        if !seen.insert(crate::util::norm_path(&dir.to_string_lossy())) || !plain_dir(&dir) {
            continue;
        }
        if dir.join(".git").exists() {
            out.push(dir);
            continue;
        }
        if depth >= DEPTH {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut subs: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                !name.starts_with('.') && !SKIP.contains(&name.as_str())
            })
            .map(|e| e.path())
            .collect();
        subs.sort();
        queue.extend(subs.into_iter().take(2_000).map(|d| (d, depth + 1)));
    }
    out
}

/// A folder of its own: not a link or junction to somewhere else.
fn plain_dir(dir: &Path) -> bool {
    match std::fs::symlink_metadata(dir) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                m.file_attributes() & 0x400 == 0 // FILE_ATTRIBUTE_REPARSE_POINT
            }
            #[cfg(not(windows))]
            true
        }
        _ => false,
    }
}

/// Runs `look` on each item, several at a time, until `deadline`. What wasn't reached in time
/// comes back as None, in its place.
pub(crate) fn each<T: Send>(items: &[PathBuf], deadline: Instant, look: impl Fn(&Path) -> Option<T> + Sync) -> Vec<Option<T>> {
    let next = AtomicUsize::new(0);
    let mut found: Vec<(usize, T)> = Vec::new();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..WORKERS.min(items.len().max(1)))
            .map(|_| {
                scope.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::SeqCst);
                        if i >= items.len() || Instant::now() >= deadline {
                            break;
                        }
                        if let Some(t) = look(&items[i]) {
                            mine.push((i, t));
                        }
                    }
                    mine
                })
            })
            .collect();
        for w in workers {
            found.extend(w.join().unwrap_or_default());
        }
    });
    let mut out: Vec<Option<T>> = items.iter().map(|_| None).collect();
    for (i, t) in found {
        out[i] = Some(t);
    }
    out
}

// ------------------------------------------------------------------ asking git

/// Git itself, where its installer puts it, or the first git.exe in a folder on PATH (a full
/// path, never the current folder).
pub(crate) fn git() -> Option<PathBuf> {
    let name = if cfg!(windows) { "git.exe" } else { "git" };
    let mut places: Vec<PathBuf> = Vec::new();
    for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
        if let Some(dir) = std::env::var_os(var) {
            places.push(PathBuf::from(dir).join("Git").join("cmd").join(name));
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        places.push(PathBuf::from(local).join("Programs").join("Git").join("cmd").join(name));
    }
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        if dir.is_absolute() {
            places.push(dir.join(name));
        }
    }
    places.into_iter().find(|p| p.is_file())
}

/// What git says about `repo`, if it answers in time and without an error.
pub(crate) fn ask(git: &Path, repo: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let repo = repo.to_str()?;
    let program = git.to_str()?;
    let mut all = vec!["--no-optional-locks", "-c", "core.fsmonitor=false", "-c", "core.untrackedCache=false", "-c", "log.showSignature=false", "-c", "protocol.allow=never", "-C", repo];
    all.extend_from_slice(args);
    crate::util::run(program, &all, GIT_TIME).ok().filter(|o| o.ok()).map(|o| o.stdout)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// One branch whose latest commits are only here.
#[derive(Debug, Clone, PartialEq)]
struct Unpushed {
    branch: String,
    /// How far ahead of where it was pushed, when it was pushed somewhere.
    commits: Option<u32>,
    /// It was never pushed at all (as opposed to being ahead of what was).
    never: bool,
    /// When its newest commit was made.
    at: u64,
}

/// What one project holds that isn't anywhere else.
#[derive(Debug, Clone, Default, PartialEq)]
struct Look {
    repo: PathBuf,
    name: String,
    no_remote: bool,
    /// The newest commit on any branch.
    last_commit: u64,
    unpushed: Vec<Unpushed>,
    /// Commits on no remote at all, counted once however many branches they're on.
    unpushed_total: u32,
    /// Changed files, and when the newest change was made, when it was a while ago.
    changes: Option<(usize, u64)>,
    /// Stashes set aside a while ago: how many, and the newest of them.
    stashes: Option<(usize, u64)>,
}

impl Look {
    fn any(&self) -> bool {
        self.no_remote || !self.unpushed.is_empty() || self.changes.is_some() || self.stashes.is_some()
    }

    /// The newest sign of the work at risk: the project people were in last comes first.
    fn newest(&self) -> u64 {
        let mut t = self.unpushed.iter().map(|u| u.at).max().unwrap_or(0);
        if self.no_remote {
            t = t.max(self.last_commit);
        }
        t.max(self.changes.map_or(0, |c| c.1)).max(self.stashes.map_or(0, |s| s.1))
    }
}

/// "[ahead 2]", "[ahead 1, behind 3]", "[behind 4]", "[gone]" or nothing.
fn ahead(track: &str) -> Option<u32> {
    track.trim_matches(|c| c == '[' || c == ']').split(',').find_map(|part| part.trim().strip_prefix("ahead ")?.trim().parse().ok())
}

/// Paths `git status --porcelain=v1 -z` lists as changed (for a rename, its new name).
fn changed_paths(z: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut parts = z.split(|b| *b == 0).filter(|p| !p.is_empty());
    while let Some(entry) = parts.next() {
        if entry.len() < 4 {
            continue;
        }
        let (code, path) = (&entry[..2], &entry[3..]);
        out.push(text(path));
        // A rename or copy is followed by the name it had before.
        if code.contains(&b'R') || code.contains(&b'C') {
            parts.next();
        }
    }
    out
}

/// The project's own settings file, when it's an ordinary one: a real .git folder (not a link,
/// and not a worktree's pointer to settings elsewhere) on this PC's own disks, holding a small
/// readable config. None for anything else, and then mom doesn't ask git about it at all.
fn settings(repo: &Path) -> Option<String> {
    let dot = repo.join(".git");
    if !crate::util::local_disk(&dot) {
        return None;
    }
    let meta = std::fs::symlink_metadata(&dot).ok()?;
    if linked(&meta) || !meta.is_dir() {
        return None;
    }
    // Git reads more than this file from .git: a common folder named elsewhere (then its settings
    // are there, not here), or objects borrowed from another folder, which could be on another
    // computer. Either, and mom leaves the project alone.
    for elsewhere in ["commondir", "gitdir", "config.worktree", "objects/info/alternates", "objects/info/http-alternates"] {
        if std::fs::symlink_metadata(dot.join(elsewhere)).is_ok() {
            return None;
        }
    }
    // Its folders must be real folders here, not links that lead somewhere else.
    for folder in ["objects", "objects/info", "objects/pack", "refs", "info", "logs"] {
        if std::fs::symlink_metadata(dot.join(folder)).is_ok_and(|m| linked(&m) || !m.is_dir()) {
            return None;
        }
    }
    let packs = std::fs::read_dir(dot.join("objects").join("pack")).into_iter().flatten().take(5_000);
    for entry in packs {
        if entry.ok().and_then(|e| std::fs::symlink_metadata(e.path()).ok()).is_none_or(|m| linked(&m)) {
            return None;
        }
    }
    let config = dot.join("config");
    std::fs::symlink_metadata(&config).ok().filter(|m| m.is_file() && m.len() <= 64 << 10)?;
    std::fs::read_to_string(&config).ok()
}

/// A link or junction: something that leads elsewhere.
fn linked(meta: &std::fs::Metadata) -> bool {
    let mut yes = meta.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        yes |= meta.file_attributes() & 0x400 != 0; // FILE_ATTRIBUTE_REPARSE_POINT
    }
    yes
}

/// The settings git keeps per project that are only data: names, addresses, branch links,
/// switches. Anything else (a filter or fsmonitor program, a signature checker, an include of
/// other settings, a setting mom doesn't know) means git might run something, so mom doesn't ask.
const PLAIN: &[(&str, &[&str])] = &[
    ("core", &["repositoryformatversion", "filemode", "bare", "logallrefupdates", "symlinks", "ignorecase", "precomposeunicode", "autocrlf", "eol", "safecrlf", "longpaths", "quotepath", "hidedotfiles", "checkstat", "trustctime", "fscache", "sparsecheckout", "sparsecheckoutcone"]),
    ("remote", &["url", "pushurl", "fetch", "push", "tagopt", "prune", "prunetags", "mirror", "skipdefaultupdate", "skipfetchall"]),
    ("branch", &["remote", "merge", "rebase", "pushremote", "description", "vscode-merge-base", "github-pr-owner-number"]),
    ("user", &["name", "email"]),
    ("gui", &["wmstate", "geometry"]),
    ("pull", &["rebase", "ff"]),
    ("push", &["default", "autosetupremote", "followtags"]),
    ("fetch", &["prune", "prunetags"]),
    ("init", &["defaultbranch"]),
    ("submodule", &["url", "active", "branch"]),
    ("lfs", &["repositoryformatversion"]),
    ("extensions", &["objectformat"]),
    ("maintenance", &["auto", "strategy"]),
    ("blame", &["ignorerevsfile", "markignoredlines", "markunblamablelines"]),
];

/// Every setting in `config` is plain data (see PLAIN). A line mom can't read counts as no.
fn plain(config: &str) -> bool {
    let mut section = String::new();
    for raw in config.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.ends_with('\\') {
            return false; // a value carried on to the next line
        }
        if let Some(head) = line.strip_prefix('[') {
            let Some(end) = head.find(']') else { return false };
            if !head[end + 1..].trim().is_empty() {
                return false;
            }
            // "[remote \"origin\"]" or "[remote.origin]": the section is the first word.
            let name = head[..end].trim();
            section = name.split([' ', '.', '"']).next().unwrap_or("").to_ascii_lowercase();
            if !PLAIN.iter().any(|(s, _)| *s == section) {
                return false;
            }
            continue;
        }
        let key = line.split('=').next().unwrap_or("").trim().to_ascii_lowercase();
        let Some((_, keys)) = PLAIN.iter().find(|(s, _)| *s == section) else { return false };
        if !keys.contains(&key.as_str()) {
            return false;
        }
    }
    true
}

fn mtime(path: &Path) -> Option<u64> {
    let m = std::fs::symlink_metadata(path).ok()?;
    Some(m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs())
}

fn look(git: &Path, repo: &Path, now: u64) -> Option<Look> {
    let name = repo.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| repo.to_string_lossy().to_string());
    let mut l = Look { repo: repo.to_path_buf(), name, ..Default::default() };
    // Only a project whose settings are plain data is asked about (see PLAIN).
    let config = settings(repo)?;
    if !plain(&config) {
        return None;
    }
    l.no_remote = !config.lines().any(|line| line.trim_start().to_ascii_lowercase().starts_with("[remote "));

    // Every branch, where it was pushed to and how far ahead it is; and whether anything's stashed.
    let refs = text(&ask(git, repo, &["for-each-ref", "--format=%(refname)%09%(upstream:short)%09%(upstream:track)%09%(committerdate:unix)", "refs/heads", "refs/stash"])?);
    let mut branches: Vec<(String, bool, Option<u32>, u64)> = Vec::new();
    let mut stashed = false;
    for line in refs.lines() {
        let mut f = line.split('\t');
        let (Some(full), Some(upstream), Some(track), Some(at)) = (f.next(), f.next(), f.next(), f.next()) else { continue };
        if full == "refs/stash" {
            stashed = true;
            continue;
        }
        let Some(branch) = full.strip_prefix("refs/heads/") else { continue };
        let at: u64 = at.trim().parse().unwrap_or(0);
        l.last_commit = l.last_commit.max(at);
        let pushed = !upstream.is_empty() && !track.contains("gone");
        branches.push((branch.to_string(), pushed, if pushed { ahead(track) } else { None }, at));
    }

    // What's on no remote at all: each commit once, and the branches whose newest commits are
    // among them. Only asked when a branch is ahead or was never pushed. (A project with no
    // remote is only here as a whole: that's said instead.)
    let maybe = branches.iter().any(|(_, pushed, ahead, _)| !pushed || ahead.is_some_and(|n| n > 0));
    if !l.no_remote && maybe {
        if let Some(log) = ask(git, repo, &["log", "--branches", "--not", "--remotes", "--max-count=5000", "--format=%ct%x09%D"]) {
            let log = text(&log);
            l.unpushed_total = log.lines().count() as u32;
            let tips: Vec<&str> = log
                .lines()
                .filter_map(|line| line.split_once('\t').map(|(_, deco)| deco))
                .flat_map(|deco| deco.split(", "))
                .map(|d| d.trim().trim_start_matches("HEAD -> "))
                .filter(|d| !d.is_empty() && *d != "HEAD" && !d.starts_with("tag: "))
                .collect();
            for (branch, pushed, commits, at) in &branches {
                if tips.contains(&branch.as_str()) {
                    l.unpushed.push(Unpushed { branch: branch.clone(), commits: *commits, never: !pushed, at: *at });
                }
            }
            l.unpushed.sort_by(|a, b| b.at.cmp(&a.at));
        }
    }

    // Changes not committed, when nobody has touched any of them for a while.
    if let Some(z) = ask(git, repo, &["status", "--porcelain=v1", "-z", "--ignore-submodules=all"]) {
        let changed = changed_paths(&z);
        if !changed.is_empty() {
            let newest = changed.iter().take(MAX_CHANGED).filter_map(|p| mtime(&repo.join(p))).max().unwrap_or(0);
            if newest > 0 && now.saturating_sub(newest) >= OLD_CHANGES {
                l.changes = Some((changed.len(), newest));
            }
        }
    }

    // Stashes set aside and not come back to.
    if let Some(list) = ask(git, repo, &["stash", "list", "--format=%ct"]).filter(|_| stashed) {
        let old: Vec<u64> = text(&list).lines().filter_map(|t| t.trim().parse().ok()).filter(|t| now.saturating_sub(*t) >= OLD_STASH).collect();
        if !old.is_empty() {
            l.stashes = Some((old.len(), old.iter().copied().max().unwrap_or(0)));
        }
    }
    Some(l)
}

// ------------------------------------------------------------------ what's said

/// A name or path from the disk, safe to put in a sentence or a prompt: one line, no
/// backticks, not long.
fn quoted(s: &str, max: usize) -> String {
    let one: String = s.chars().map(|c| if c.is_control() || c == '`' { ' ' } else { c }).collect();
    let one = one.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > max {
        format!("{}…", one.chars().take(max).collect::<String>())
    } else {
        one
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

fn ago(now: u64, at: u64) -> String {
    crate::util::human_age_long(now.saturating_sub(at))
}

/// One line for each thing in a project that's only here.
fn items(l: &Look, now: u64) -> Vec<Item> {
    let mut out = Vec::new();
    if l.no_remote {
        out.push(Item { name: "No remote".into(), note: format!("Last commit {} ago", ago(now, l.last_commit)), led: "red" });
    }
    for u in l.unpushed.iter().take(6) {
        let note = match (u.never, u.commits) {
            (false, Some(n)) => format!("{} not pushed", plural(n as usize, "commit", "commits")),
            _ => "Never pushed".to_string(),
        };
        out.push(Item { name: quoted(&u.branch, 60), note, led: "red" });
    }
    if let Some((n, at)) = l.changes {
        out.push(Item { name: "Changes".into(), note: format!("{} not committed, last touched {} ago", plural(n, "file", "files"), ago(now, at)), led: "off" });
    }
    if let Some((n, at)) = l.stashes {
        out.push(Item { name: "Stashes".into(), note: format!("{} set aside, the newest {} ago", n, ago(now, at)), led: "off" });
    }
    out
}

/// What to hand an agent so the work gets a copy somewhere else, without anything lost.
fn prompt(l: &Look, now: u64) -> String {
    let lines: Vec<String> = items(l, now).iter().map(|i| format!("- {}: {}", i.name, i.note)).collect();
    let mut asks = Vec::new();
    if l.no_remote {
        asks.push("help me make a private remote for it (GitHub, say) and push every branch to it");
    }
    if !l.unpushed.is_empty() {
        asks.push("push the branches with commits that aren't pushed");
    }
    if l.changes.is_some() {
        asks.push("go through the changes that aren't committed with me, and commit the ones worth keeping");
    }
    if l.stashes.is_some() {
        asks.push("show me what's in the old stashes, so I can keep or drop each one");
    }
    format!(
        "My project `{}` at `{}` has work that exists only on this PC:\n{}\n\nPlease {}. Don't delete, reset, rebase or force-push anything, and show me each command before you run it.\n",
        quoted(&l.name, 80),
        quoted(&l.repo.to_string_lossy(), 300),
        lines.join("\n"),
        asks.join(", then ")
    )
}

fn check_of(l: &Look, now: u64) -> Check {
    let name = quoted(&l.name, 60);
    let pushed = l.unpushed_total;
    let (level, title, detail) = if l.no_remote {
        ("warn", format!("{name} is only on this PC"), "It has no remote, so a failing drive would take it with it. A private remote keeps a copy.".to_string())
    } else if pushed > 0 {
        ("warn", format!("{name} has {} that aren't pushed", plural(pushed as usize, "commit", "commits")), "They're only on this PC until they are. Pushing keeps a copy.".to_string())
    } else if let Some((n, at)) = l.changes {
        ("tip", format!("{name} has changes not committed for {}", ago(now, at)), format!("{} with work in them, only on this PC.", plural(n, "file", "files")))
    } else {
        let (n, at) = l.stashes.unwrap_or_default();
        ("tip", format!("{name} has {} set aside {} ago", plural(n, "stash", "stashes"), ago(now, at)), "Stashes stay on this PC. Worth a look before they're forgotten.".to_string())
    };
    Check {
        id: format!("repo:{}", crate::util::norm_path(&l.repo.to_string_lossy())),
        area: "Backups",
        level,
        title,
        detail,
        fix: Some(Fix { label: "Copy for your agent".into(), kind: "copy", value: prompt(l, now), hint: Some("Paste it to your agent.".into()) }),
        more: vec![Fix { label: "Open folder".into(), kind: "folder", value: l.repo.to_string_lossy().to_string(), hint: None }],
        items: items(l, now),
    }
}

fn newest_first(looks: &mut [Look]) {
    looks.sort_by(|a, b| b.newest().cmp(&a.newest()).then_with(|| a.name.cmp(&b.name)));
}

/// The checks for every project's work that's only on this PC, newest first, or one saying
/// all of it has a copy somewhere else.
pub fn checks(paths: &Paths) -> Vec<Check> {
    let Some(git) = git() else { return vec![] };
    checks_in(&git, &find(paths), crate::util::now_secs())
}

fn checks_in(git: &Path, repos: &[PathBuf], now: u64) -> Vec<Check> {
    if repos.is_empty() {
        return vec![];
    }
    let deadline = Instant::now() + ALL_TIME;
    let looked = each(repos, deadline, |r| look(git, r, now));
    let unchecked = looked.iter().filter(|l| l.is_none()).count();
    let mut risky: Vec<Look> = looked.into_iter().flatten().filter(Look::any).collect();
    newest_first(&mut risky);
    let mut out: Vec<Check> = risky.iter().take(MAX_CHECKS).map(|l| check_of(l, now)).collect();
    let rest = &risky[risky.len().min(MAX_CHECKS)..];
    if !rest.is_empty() {
        let list: Vec<String> = rest.iter().map(|l| format!("- `{}` at `{}`", quoted(&l.name, 80), quoted(&l.repo.to_string_lossy(), 300))).collect();
        out.push(Check {
            id: "repos-more".into(),
            area: "Backups",
            level: "tip",
            title: format!("{} more have work only on this PC", plural(rest.len(), "project", "projects")),
            detail: "Older ones: commits, changes or stashes with no copy elsewhere.".into(),
            fix: Some(Fix {
                label: "Copy for your agent".into(),
                kind: "copy",
                value: format!(
                    "These projects of mine have work that exists only on this PC (commits not pushed, changes not committed, or stashes):\n{}\n\nGo through them with me one at a time and help me get a copy of what's worth keeping onto a remote. Don't delete, reset, rebase or force-push anything, and show me each command before you run it.\n",
                    list.join("\n")
                ),
                hint: Some("Paste it to your agent.".into()),
            }),
            more: Vec::new(),
            items: rest.iter().take(20).map(|l| Item { name: quoted(&l.name, 60), note: items(l, now).first().map(|i| i.note.clone()).unwrap_or_default(), led: "off" }).collect(),
        });
    }
    if out.is_empty() {
        let n = repos.len() - unchecked;
        if n > 0 {
            out.push(Check {
                id: "repos-ok".into(),
                area: "Backups",
                level: "ok",
                title: format!("{} {} a copy elsewhere", plural(n, "project", "projects"), if n == 1 { "has" } else { "have" }),
                detail: "Every commit is pushed, and nothing has been left uncommitted for long.".into(),
                fix: None,
                more: Vec::new(),
                items: Vec::new(),
            });
        }
    }
    out
}

#[cfg(test)]
mod settings_tests {
    use super::plain;

    #[test]
    fn only_settings_that_are_plain_data_let_git_be_asked() {
        let ordinary = "[core]\n\trepositoryformatversion = 0\n\tfilemode = false\n\tbare = false\n[remote \"origin\"]\n\turl = https://git.example/nimbus.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n[branch \"main\"]\n\tremote = origin\n\tmerge = refs/heads/main\n";
        assert!(plain(ordinary));
        for program in [
            "[core]\n\tfsmonitor = run-me.exe\n",
            "[filter \"lfs\"]\n\tclean = run-me.exe %f\n",
            "[include]\n\tpath = elsewhere.cfg\n",
            "[includeIf \"gitdir:x\"]\n\tpath = elsewhere.cfg\n",
            "[gpg]\n\tprogram = run-me.exe\n",
            "[log]\n\tshowSignature = true\n",
            "[core]\n\tsshCommand = run-me.exe\n",
            "[submodule \"x\"]\n\tupdate = !run-me.exe\n",
            "[core]\n\tbare = false \\\n",
            "[core] fsmonitor = run-me.exe\n",
            "[remote \"origin\"]\n\tpromisor = true\n\tpartialclonefilter = blob:none\n",
        ] {
            assert!(!plain(&format!("{ordinary}{program}")), "{program}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("mom-repos-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn run(git: &Path, dir: &Path, args: &[&str]) {
        let mut all = vec!["-C", dir.to_str().unwrap(), "-c", "user.name=Nimbus", "-c", "user.email=nimbus@example.com", "-c", "commit.gpgsign=false"];
        all.extend_from_slice(args);
        let o = crate::util::run(git.to_str().unwrap(), &all, Duration::from_secs(20)).unwrap();
        assert!(o.ok(), "git {args:?}: {}", o.err_text());
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn age(path: &Path, days: u64) {
        let when = std::time::SystemTime::now() - Duration::from_secs(days * 86_400);
        std::fs::File::options().write(true).open(path).unwrap().set_modified(when).unwrap();
    }

    #[test]
    fn a_remote_is_read_from_the_projects_settings() {
        let root = fixture("remote");
        let git_dir = root.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(git_dir.join("config"), "[core]\n\tbare = false\n").unwrap();
        assert!(settings(&root).is_some_and(|c| plain(&c) && !c.contains("[remote ")));
        std::fs::write(git_dir.join("config"), "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = https://example.com/nimbus.git\n").unwrap();
        assert!(settings(&root).is_some_and(|c| plain(&c) && c.contains("[remote ")));
        std::fs::remove_dir_all(&git_dir).unwrap();
        std::fs::write(root.join(".git"), "gitdir: ../elsewhere/.git/worktrees/x\n").unwrap();
        assert_eq!(settings(&root), None, "a worktree: its settings are elsewhere, so mom leaves it alone");
        std::fs::remove_file(root.join(".git")).unwrap();
        std::fs::create_dir_all(git_dir.join("objects").join("info")).unwrap();
        std::fs::write(git_dir.join("config"), "[core]\n\tbare = false\n").unwrap();
        assert!(settings(&root).is_some(), "an ordinary project");
        std::fs::write(git_dir.join("objects").join("info").join("alternates"), "C:\\elsewhere\\objects\n").unwrap();
        assert_eq!(settings(&root), None, "objects borrowed from elsewhere");
        std::fs::remove_file(git_dir.join("objects").join("info").join("alternates")).unwrap();
        std::fs::write(git_dir.join("commondir"), "../elsewhere\n").unwrap();
        assert_eq!(settings(&root), None, "settings in a common folder elsewhere");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn little_helpers() {
        assert_eq!(ahead("[ahead 2]"), Some(2));
        assert_eq!(ahead("[ahead 1, behind 3]"), Some(1));
        assert_eq!((ahead("[behind 4]"), ahead("[gone]"), ahead("")), (None, None, None));
        assert_eq!(changed_paths(b" M src/a.rs\0?? notes.txt\0R  new.rs\0old.rs\0"), ["src/a.rs", "notes.txt", "new.rs"]);
        assert_eq!(quoted("Copper`line\nNight", 80), "Copper line Night");
    }

    #[test]
    fn the_project_worked_on_last_comes_first() {
        let at = |name: &str, commit: u64, changed: Option<u64>| Look { name: name.into(), no_remote: true, last_commit: commit, changes: changed.map(|t| (1, t)), ..Default::default() };
        let mut looks = vec![at("atlas", 100, None), at("copperline", 50, Some(300)), at("lanternfall", 200, None)];
        newest_first(&mut looks);
        let names: Vec<&str> = looks.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["copperline", "lanternfall", "atlas"]);
    }

    #[test]
    fn projects_are_found_without_walking_into_them_or_their_build_folders() {
        let root = fixture("find");
        std::fs::create_dir_all(root.join("storefront").join(".git")).unwrap();
        std::fs::create_dir_all(root.join("storefront").join("packages").join("inner").join(".git")).unwrap();
        std::fs::create_dir_all(root.join("tools").join("atlas").join(".git")).unwrap();
        std::fs::create_dir_all(root.join("node_modules").join("left").join(".git")).unwrap();
        std::fs::create_dir_all(root.join("a").join("b").join("c").join("too-deep").join(".git")).unwrap();
        let mut found: Vec<String> = find_in(std::slice::from_ref(&root)).iter().map(|p| p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/")).collect();
        found.sort();
        assert_eq!(found, ["storefront", "tools/atlas"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn work_only_here_is_found_and_said() {
        let Some(git) = git() else { return }; // no git here: nothing to test with
        let root = fixture("look");
        let now = crate::util::now_secs();
        // A remote, and a project cloned from it.
        let origin = root.join("origin.git");
        std::fs::create_dir_all(&origin).unwrap();
        run(&git, &origin, &["init", "--bare", "-q", "-b", "main"]);
        let lantern = root.join("lanternfall");
        std::fs::create_dir_all(&lantern).unwrap();
        run(&git, &lantern, &["init", "-q", "-b", "main"]);
        write(&lantern.join("readme.md"), "Lanternfall\n");
        run(&git, &lantern, &["add", "."]);
        run(&git, &lantern, &["commit", "-q", "-m", "first"]);
        run(&git, &lantern, &["remote", "add", "origin", origin.to_str().unwrap()]);
        run(&git, &lantern, &["push", "-q", "-u", "origin", "main"]);
        // Two commits not pushed, and a branch never pushed.
        for n in 1..=2 {
            write(&lantern.join(format!("part{n}.md")), "more\n");
            run(&git, &lantern, &["add", "."]);
            run(&git, &lantern, &["commit", "-q", "-m", "more"]);
        }
        run(&git, &lantern, &["branch", "experiment"]);
        // A project with no remote, and a change nobody's touched in ten days.
        let copper = root.join("copperline");
        std::fs::create_dir_all(&copper).unwrap();
        run(&git, &copper, &["init", "-q", "-b", "main"]);
        write(&copper.join("notes.md"), "one\n");
        run(&git, &copper, &["add", "."]);
        run(&git, &copper, &["commit", "-q", "-m", "first"]);
        write(&copper.join("notes.md"), "two\n");
        age(&copper.join("notes.md"), 10);
        // All pushed and tidy.
        let atlas = root.join("atlas");
        std::fs::create_dir_all(&atlas).unwrap();
        run(&git, &atlas, &["init", "-q", "-b", "main"]);
        write(&atlas.join("a.md"), "a\n");
        run(&git, &atlas, &["add", "."]);
        run(&git, &atlas, &["commit", "-q", "-m", "first"]);
        run(&git, &atlas, &["remote", "add", "origin", origin.to_str().unwrap()]);
        run(&git, &atlas, &["push", "-q", "-u", "origin", "main:atlas-main"]);
        run(&git, &atlas, &["branch", "--set-upstream-to=origin/atlas-main"]);

        let l = look(&git, &lantern, now).unwrap();
        assert!(!l.no_remote && l.changes.is_none() && l.stashes.is_none());
        let mut branches: Vec<(String, Option<u32>, bool)> = l.unpushed.iter().map(|u| (u.branch.clone(), u.commits, u.never)).collect();
        branches.sort();
        assert_eq!(branches, [("experiment".to_string(), None, true), ("main".to_string(), Some(2), false)]);
        let c = look(&git, &copper, now).unwrap();
        assert!(c.no_remote);
        assert_eq!(c.changes.map(|x| x.0), Some(1));
        assert!(!look(&git, &atlas, now).unwrap().any());

        assert_eq!(l.unpushed_total, 2, "the same two commits, on two branches, are counted once");
        let mut checks = checks_in(&git, &[atlas.clone(), copper.clone(), lantern.clone()], now);
        checks.sort_by(|a, b| b.title.cmp(&a.title));
        let titles: Vec<&str> = checks.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["lanternfall has 2 commits that aren't pushed", "copperline is only on this PC"]);
        assert!(checks.iter().all(|c| c.area == "Backups" && c.level == "warn"));
        let fix = checks[0].fix.as_ref().unwrap();
        assert!(fix.value.starts_with("My project `lanternfall` at `") && fix.value.contains("push the branches") && fix.value.contains("Don't delete"), "{}", fix.value);
        assert_eq!((checks[1].more[0].kind, checks[1].more[0].value.as_str()), ("folder", copper.to_str().unwrap()));
        // Nothing at risk: one line saying so.
        let fine = checks_in(&git, std::slice::from_ref(&atlas), now);
        assert_eq!((fine[0].level, fine[0].title.as_str()), ("ok", "1 project has a copy elsewhere"));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
