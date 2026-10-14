//! Build folders in projects nobody is working on: node_modules, Rust's target, .next and the
//! like. They're big, they hold no code of yours, and `npm install` or `cargo build` makes them
//! again. Projects worked on in the last month, or with something running, are left alone.

use super::{ago, name_of, tally_many, Cand, Found, TrashCtx, ENTRY_CAP, IDLE_PROJECT_DAYS, IN_USE};
use crate::paths::Paths;
use crate::util::norm_path;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Build folders a JavaScript project (one with a package.json) can make again.
const JS_BUILD: &[&str] = &["node_modules", ".next", ".turbo", ".parcel-cache", ".svelte-kit"];

/// Files that change when someone works on a project.
const ACTIVITY: &[&str] = &[
    "package.json",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "bun.lockb",
    "bun.lock",
    "Cargo.toml",
    "Cargo.lock",
];

/// Never worth walking into while looking for projects.
const DONT_WALK: &[&str] = &["node_modules", "target", "dist", "build", "out", "coverage", "vendor", "venv", "__pycache__"];

/// Projects sit at most this many folders below a root.
const MAX_DEPTH: usize = 3;

/// Files mom will count across all build folders before she stops measuring the rest.
const BUDGET: u64 = 10_000_000;
/// Metadata entries across source trees, independent of the build-folder size budget.
const ACTIVITY_BUDGET: usize = 250_000;
const ACTIVITY_DEPTH: usize = 32;

/// Folder names people keep code under at the top of a drive.
const DRIVE_ROOTS: &[&str] = &["Workspace", "Projects", "Code", "dev", "Development", "src", "repos", "GitHub", "git", "work"];

/// The config's rooms plus the usual places people keep code, if they exist.
pub(super) fn default_roots(paths: &Paths, rooms: &[PathBuf]) -> Vec<PathBuf> {
    // Only rooms that are rooms: a whole drive (say) would mean walking everything on it.
    let mut v: Vec<PathBuf> = rooms.iter().filter_map(|r| crate::tidy::check_room(r, paths).ok()).collect();
    for rel in [
        "Workspace",
        "workspace",
        "Projects",
        "projects",
        "Code",
        "code",
        "dev",
        "src",
        "repos",
        "source/repos",
        "Documents/GitHub",
        "GitHub",
    ] {
        v.push(paths.home.join(rel));
    }
    // And the same names at the top of every drive that's built in: code often lives on a second
    // disk (D:\Workspace, E:\Projects). Only those names: a whole drive is never walked. And only
    // if the folder is yours: the top of a drive is a place anyone on the PC can make folders in.
    for drive in crate::util::fixed_drives() {
        for name in DRIVE_ROOTS {
            let dir = Path::new(&drive).join(name);
            if crate::elevate::owned_by_me(&dir) {
                v.push(dir);
            }
        }
    }
    // Each by where it really is: a Projects folder that is a link to another drive is searched
    // there, under its real name, so what mom finds and what she later opens are spelled alike.
    let mut seen = HashSet::new();
    v.into_iter().filter(|p| p.is_dir()).filter_map(|p| crate::util::real_path(&p)).filter(|p| seen.insert(norm_path(&p.to_string_lossy()))).collect()
}

struct Project {
    dir: PathBuf,
    /// Where it sits under its root: "storefront", "tools/old-cli", "storefront/apps/web".
    name: String,
    /// Newest sign of work, including the surrounding repository. None means some activity
    /// could not be checked; Some(0) is only the neutral value when no repository was found.
    active: Option<u64>,
    js: bool,
    rust: bool,
}

/// A real file (not a link, not a folder) that you own.
fn my_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).map(|m| m.is_file()).unwrap_or(false) && crate::elevate::owned_by_me(path)
}

/// An optional activity entry: only NotFound establishes absence. Links, the wrong entry
/// type, invalid dates and all other errors leave activity unknown.
fn activity_timestamp(metadata: std::io::Result<std::fs::Metadata>, directory: bool) -> Option<Option<u64>> {
    let meta = match metadata {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some(None),
        Err(_) => return None,
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 { return None; }
    }
    if meta.file_type().is_symlink() || if directory { !meta.is_dir() } else { !meta.is_file() } {
        return None;
    }
    super::cleanup_recency(&meta).map(Some)
}

/// Inspect a fixed set of Git metadata paths, including the directories that contain them.
/// Never recurse through Git objects or follow a linked/external worktree Git directory.
fn newest_activity(dir: &Path, project: bool) -> Option<u64> {
    let at = |path: &Path, directory| activity_timestamp(std::fs::symlink_metadata(path), directory);
    let mut newest = 0;
    let git = dir.join(".git");
    if let Some(time) = at(&git, true)? {
        newest = newest.max(time);
        for name in ["index", "HEAD"] {
            newest = newest.max(at(&git.join(name), false)?.unwrap_or(0));
        }
        let logs = git.join("logs");
        if let Some(time) = at(&logs, true)? {
            newest = newest.max(time).max(at(&logs.join("HEAD"), false)?.unwrap_or(0));
        }
    }
    if project {
        for name in ACTIVITY {
            newest = newest.max(at(&dir.join(name), false)?.unwrap_or(0));
        }
        // A project directory itself is required, unlike its optional lockfiles.
        newest = newest.max(at(dir, true)??);
    }
    Some(newest)
}

/// Edits to existing source files need not touch a manifest, Git index or parent directory.
/// Read only metadata, omit generated trees, and retain the project if evidence is incomplete.
fn source_activity(dir: &Path, budget: &mut usize) -> Option<u64> {
    let mut newest = 0;
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((path, depth)) = stack.pop() {
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        let meta = std::fs::symlink_metadata(&path).ok()?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return None;
            }
        }
        if meta.file_type().is_symlink() || (!meta.is_dir() && !meta.is_file()) {
            return None;
        }
        newest = newest.max(super::cleanup_recency(&meta)?);
        if !meta.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&path).ok()? {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if name == ".git" || name == ".venv" || DONT_WALK.contains(&name.as_str()) || JS_BUILD.contains(&name.as_str()) {
                continue;
            }
            if depth >= ACTIVITY_DEPTH || stack.len() >= *budget {
                return None;
            }
            stack.push((entry.path(), depth + 1));
        }
    }
    (newest > 0).then_some(newest)
}

fn walk(dir: &Path, rel: &str, depth: usize, inherited: Option<u64>, seen: &mut HashSet<String>, out: &mut Vec<Project>) {
    if !seen.insert(norm_path(&dir.to_string_lossy())) {
        return;
    }
    // What makes a folder a project is a file, and it has to be your file: in a folder other
    // people can add to, a package.json dropped next to your own "build" or "dist" folder would
    // otherwise make mom offer that folder up.
    let js = my_file(&dir.join("package.json"));
    let rust = my_file(&dir.join("Cargo.toml"));
    let project = js || rust;
    // Even an unreadable .git must protect descendant packages; exists() would hide errors.
    let active = inherited.zip(newest_activity(dir, project)).map(|(around, here)| around.max(here));
    // Only your own projects: under a shared folder, someone else's can sit next to yours.
    if project && crate::elevate::owned_by_me(dir) {
        let name = if rel.is_empty() { name_of(dir) } else { rel.to_string() };
        out.push(Project { dir: dir.to_path_buf(), name, active, js, rust });
    }
    if depth >= MAX_DEPTH {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut kids: Vec<PathBuf> = rd
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_dir() && !t.is_symlink()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| {
            let n = name_of(p);
            !n.starts_with('.') && !n.starts_with('$') && !DONT_WALK.contains(&n.to_ascii_lowercase().as_str())
        })
        .collect();
    kids.sort();
    for kid in kids {
        let n = name_of(&kid);
        let kid_rel = if rel.is_empty() { n } else { format!("{rel}/{n}") };
        walk(&kid, &kid_rel, depth + 1, active, seen, out);
    }
}

fn projects(roots: &[PathBuf]) -> Vec<Project> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for root in roots {
        walk(root, "", 0, Some(0), &mut seen, &mut out);
    }
    out
}

/// Names too plain to say whose a folder is: half the projects there are have a part called this.
const PLAIN_NAMES: &[&str] = &[
    "app", "apps", "web", "site", "website", "docs", "client", "server", "desktop", "mobile", "api", "core", "main", "test",
    "tests", "demo", "example", "examples", "src", "lib", "packages", "frontend", "backend", "landing", "tools", "scripts",
    "electron", "tauri", "srctauri", "project", "template", "starter", "name",
];

/// One of your own projects, and what an app built from it calls itself.
pub(super) struct Mine {
    /// Where it sits under its root ("storefront", "tools/old-cli").
    pub name: String,
    names: Vec<String>,
}

impl Mine {
    /// Does a folder called `folder` (in AppData, say) go by one of this project's names? By an
    /// app identifier exactly ("com.storefront.app"), or by a name that is the folder's whole name
    /// or one part of it ("storefront" in "com.storefront.app").
    pub(super) fn owns(&self, folder: &str) -> bool {
        use crate::installed::{squash, unversioned};
        let whole = squash(unversioned(folder));
        let parts: Vec<String> = folder.split('.').map(squash).collect();
        self.names.iter().any(|n| {
            if n.contains('.') && n.eq_ignore_ascii_case(folder) {
                return true;
            }
            let n = squash(n);
            n.len() >= 4 && !PLAIN_NAMES.contains(&n.as_str()) && (n == whole || parts.contains(&n))
        })
    }
}

/// A small file of yours, as text (a project's package.json, say). Not a link, not someone else's.
fn small_text(path: &Path) -> Option<String> {
    if !my_file(path) || std::fs::metadata(path).ok()?.len() > 1 << 20 {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// Your own projects under `roots`, each with the names an app built from it keeps its data
/// under: the folder's name, and the names and app identifiers in its package.json, Cargo.toml
/// and Tauri config. An app you're building is never "installed", and it isn't gone.
pub(super) fn my_projects(roots: &[PathBuf]) -> Vec<Mine> {
    let text_at = |v: &serde_json::Value, path: &[&str]| path.iter().try_fold(v, |v, k| v.get(k)).and_then(|v| v.as_str()).map(str::to_string);
    projects(roots)
        .into_iter()
        .map(|p| {
            let mut names = vec![name_of(&p.dir)];
            // A Tauri app's Rust half sits in src-tauri: the project is the folder around it.
            let tauri_half = name_of(&p.dir).eq_ignore_ascii_case("src-tauri");
            if tauri_half {
                names.extend(p.dir.parent().map(name_of));
            }
            if let Some(v) = small_text(&p.dir.join("package.json")).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
                names.extend([&["name"][..], &["productName"], &["build", "appId"], &["build", "productName"]].iter().filter_map(|k| text_at(&v, k)));
            }
            for conf in [p.dir.join("tauri.conf.json"), p.dir.join("src-tauri").join("tauri.conf.json")] {
                if let Some(v) = small_text(&conf).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
                    let keys = [&["identifier"][..], &["productName"], &["package", "productName"], &["tauri", "bundle", "identifier"]];
                    names.extend(keys.iter().filter_map(|k| text_at(&v, k)));
                }
            }
            if let Some(t) = small_text(&p.dir.join("Cargo.toml")).and_then(|t| t.parse::<toml::Table>().ok()) {
                names.extend(t.get("package").and_then(|p| p.get("name")).and_then(|n| n.as_str()).map(str::to_string));
            }
            names.retain(|n| !n.trim().is_empty() && n.len() <= 200);
            let name = if tauri_half { p.name.rsplit_once('/').map(|(around, _)| around.to_string()).unwrap_or(p.name) } else { p.name };
            Mine { name, names }
        })
        .collect()
}

fn build_dirs(p: &Project) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if p.js {
        v.extend(JS_BUILD.iter().map(|b| p.dir.join(b)));
    }
    if p.rust {
        v.push(p.dir.join("target"));
    }
    // A real folder (never a link to one), and yours.
    v.into_iter()
        .filter(|d| std::fs::symlink_metadata(d).map(|m| m.is_dir()).unwrap_or(false) && crate::elevate::owned_by_me(d))
        .collect()
}

pub(super) fn find(ctx: &TrashCtx) -> Found {
    if ctx.project_roots.is_empty() {
        return Found::none();
    }
    let mut f = Found::none(); // spread over every project, so no single root
    f.available = true;
    let idle_after = IDLE_PROJECT_DAYS * 86_400;
    let projects = projects(&ctx.project_roots);
    let mut list: Vec<(&Project, PathBuf)> =
        projects.iter().flat_map(|p| build_dirs(p).into_iter().map(move |d| (p, d))).collect();
    // Measure what could go first, so a huge active project can't use up the budget.
    list.sort_by_key(|(p, _)| p.active.unwrap_or(u64::MAX));
    let dirs: Vec<PathBuf> = list.iter().map(|(_, d)| d.clone()).collect();
    let tallies = tally_many(&dirs, ENTRY_CAP, BUDGET);
    let mut budget = ACTIVITY_BUDGET;
    let mut activity: HashMap<PathBuf, Option<u64>> = HashMap::new();
    for (p, _) in &list {
        activity.entry(p.dir.clone()).or_insert_with(|| p.active.and_then(|around| source_activity(&p.dir, &mut budget).map(|t| t.max(around))));
    }
    // A fresh build protects every generated folder in its project, including dependencies.
    // A partial measurement cannot establish that the project's artifacts are all old.
    for ((p, _), t) in list.iter().zip(&tallies) {
        let active = activity.get_mut(&p.dir).unwrap();
        if !t.complete || t.recency == 0 {
            *active = None;
        } else if let Some(newest) = active {
            *newest = (*newest).max(t.recency);
        }
    }
    for ((p, dir), t) in list.into_iter().zip(tallies) {
        let mut c = Cand::new(dir.clone(), format!("{} · {}", p.name, name_of(&dir)), &t, ctx.now).age_sensitive();
        let active = activity[&p.dir];
        c.age_known &= active.is_some();
        c.age = active.map_or(0, |t| ctx.now.saturating_sub(t));
        let c = if ctx.in_use(&p.dir) {
            c.verdict(false, IN_USE)
        } else if active.is_none() {
            c.verdict(false, "Project activity could not be fully checked")
        } else if c.age < idle_after {
            let r = format!("Project activity recorded {} ago", ago(c.age));
            c.kept_but_tickable(r)
        } else {
            let r = format!("No newer project activity recorded for {}", ago(c.age));
            c.verdict(true, r)
        };
        f.cands.push(c);
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TrashConfig;
    use crate::trash::tests::{ctx, fixture, write};
    use crate::trash::{clean, measure};

    #[test]
    fn idle_projects_lose_their_build_folders_active_ones_keep_them() {
        let root = fixture("builds");
        // Named the way mom names every folder she searches: by where it really is (see
        // default_roots). A temp folder can be spelled another way, with a short name in it
        // (C:\Users\RUNNER~1 on a build machine) or through a link, and a folder that isn't
        // where its path says is one mom won't delete.
        std::fs::create_dir_all(root.join("Workspace")).unwrap();
        let ws = crate::util::real_path(&root.join("Workspace")).unwrap();
        // An old JS project nobody touched for 90 days.
        let old = ws.join("old-site");
        write(&old.join("package.json"), 20, 90);
        write(&old.join("src").join("index.js"), 10, 90);
        write(&old.join("node_modules").join("left-pad").join("index.js"), 3000, 90);
        write(&old.join(".next").join("cache").join("x"), 500, 90);
        // An old Rust project, and a target folder in a project without Cargo.toml (not ours).
        let crate_dir = ws.join("tools").join("old-cli");
        write(&crate_dir.join("Cargo.toml"), 20, 60);
        write(&crate_dir.join("target").join("debug").join("old-cli.exe"), 7000, 60);
        write(&ws.join("game").join("target").join("level.dat"), 999, 90);
        write(&ws.join("game").join("README.md"), 5, 90);
        // An active project: committed two days ago.
        let live = ws.join("storefront");
        write(&live.join("package.json"), 20, 90);
        write(&live.join(".git").join("index"), 20, 2);
        write(&live.join("node_modules").join("react").join("index.js"), 4000, 90);
        // Something running from another old project protects it.
        let busy = ws.join("busy-app");
        write(&busy.join("package.json"), 20, 90);
        write(&busy.join("node_modules").join("vite").join("bin.js"), 600, 90);
        for p in [&old, &crate_dir, &ws.join("tools"), &live, &busy, &ws.join("game")] {
            age_dir(p, 90);
        }

        let paths = Paths::under(&root);
        let cfg = TrashConfig::default();
        let mut ctx = ctx(&paths, &cfg);
        ctx.project_roots = vec![ws.clone()];
        ctx.in_use.push_str(&norm_path(&busy.to_string_lossy()));
        let spot = measure("old-build-folders", &ctx).unwrap();
        let mut verdicts: Vec<(String, bool)> = spot.items.iter().map(|i| (i.name.clone(), i.goes)).collect();
        verdicts.sort();
        assert_eq!(
            verdicts,
            vec![
                ("busy-app · node_modules".to_string(), false),
                ("old-site · .next".to_string(), true),
                ("old-site · node_modules".to_string(), true),
                ("storefront · node_modules".to_string(), false),
                ("tools/old-cli · target".to_string(), true),
            ]
        );
        let storefront = spot.items.iter().find(|i| i.name.starts_with("storefront")).unwrap();
        assert_eq!(storefront.reason, "Project activity recorded 2 days ago");
        assert_eq!(spot.items.iter().find(|i| i.name.starts_with("busy")).unwrap().reason, IN_USE);
        assert_eq!(spot.reclaimable_bytes, 10_500);
        assert!(spot.effect.contains("from 2 projects"), "{}", spot.effect);
        assert_eq!((spot.group.as_str(), spot.safety.as_str()), ("dev", "review"));

        let r = clean("old-build-folders", &ctx, None);
        assert_eq!((r.deleted, r.freed), (3, 10_500));
        assert!(!old.join("node_modules").exists() && old.join("src").join("index.js").exists());
        assert!(live.join("node_modules").exists());
        assert!(ws.join("game").join("target").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn edits_and_new_artifacts_protect_every_build_folder_without_a_manifest_change() {
        let root = fixture("build-activity");
        let ws = root.join("Workspace");
        for name in ["source-edit", "new-build"] {
            let dir = ws.join(name);
            write(&dir.join("package.json"), 20, 90);
            write(&dir.join("src").join("main.js"), 20, if name == "source-edit" { 2 } else { 90 });
            write(&dir.join("node_modules").join("dep").join("index.js"), 40, 90);
            write(&dir.join(".next").join("cache").join("output"), 40, if name == "new-build" { 2 } else { 90 });
            age_dir(&dir, 90);
        }
        let paths = Paths::under(&root);
        let cfg = TrashConfig::default();
        let mut context = ctx(&paths, &cfg);
        context.project_roots = vec![ws];
        let found = find(&context);
        assert_eq!(found.cands.len(), 4);
        assert!(found.cands.iter().all(|c| !c.goes && c.age < 3 * 86_400), "{:?}", found.cands.iter().map(|c| (&c.name, c.age, &c.reason)).collect::<Vec<_>>());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn incomplete_source_activity_never_becomes_an_old_project() {
        let root = fixture("build-activity-limit");
        write(&root.join("src").join("main.js"), 20, 90);
        assert_eq!(source_activity(&root, &mut 0), None);
        assert_eq!(source_activity(&root, &mut 1), None);
        assert_eq!(source_activity(&root.join("missing"), &mut 10), None);
        // Generated dependencies do not consume the source scan's entry budget.
        let small = root.join("tiny");
        write(&small.join("package.json"), 20, 90);
        write(&small.join("node_modules").join("many").join("files.js"), 20, 90);
        assert!(source_activity(&small, &mut 2).is_some());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn newly_created_git_activity_protects_direct_and_nested_projects_with_old_write_times() {
        use std::os::windows::fs::FileTimesExt;
        use std::time::{Duration, SystemTime};

        let root = fixture("build-git-created");
        let ws = root.join("Workspace");
        let mut indexes = Vec::new();
        for name in ["direct", "nested"] {
            let repo = ws.join(name);
            let project = if name == "nested" { repo.join("packages").join("web") } else { repo.clone() };
            let index = repo.join(".git").join("index");
            write(&index, 20, 90);
            write(&project.join("package.json"), 20, 90);
            write(&project.join("node_modules").join("dep.js"), 40, 90);
            indexes.push(index);
        }
        let paths = Paths::under(&root);
        let cfg = TrashConfig::default();
        let mut context = ctx(&paths, &cfg);
        context.project_roots = vec![ws];
        let before = find(&context);
        assert_eq!(before.cands.len(), 2);
        assert!(before.cands.iter().all(|c| c.goes));

        let old = SystemTime::now() - Duration::from_secs(90 * 86_400);
        for index in indexes {
            std::fs::File::options().write(true).open(index).unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(old).set_created(SystemTime::now())).unwrap();
        }
        let after = find(&context);
        assert_eq!(after.cands.len(), 2);
        assert!(after.cands.iter().all(|c| c.age_known && !c.goes && c.age < 86_400));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn invalid_repository_activity_stays_unknown_in_descendant_projects() {
        let root = fixture("build-git-unknown");
        let ws = root.join("Workspace");
        let repo = ws.join("repo");
        let project = repo.join("packages").join("web");
        write(&project.join("package.json"), 20, 90);
        write(&project.join("node_modules").join("dep.js"), 40, 90);
        // A directory cannot stand in for the Git index. Its old contents must not make this
        // malformed/uncheckable repository look inactive to its nested package.
        write(&repo.join(".git").join("index").join("unexpected"), 20, 90);
        assert_eq!(newest_activity(&repo, false), None);
        let paths = Paths::under(&root);
        let cfg = TrashConfig::default();
        let mut context = ctx(&paths, &cfg);
        context.project_roots = vec![ws];
        let found = find(&context);
        assert_eq!(found.cands.len(), 1);
        let candidate = &found.cands[0];
        assert!(!candidate.age_known && !candidate.goes && !candidate.optional);
        let pick = crate::trash::Pick { new: true, ticked: vec![candidate.path.to_string_lossy().into_owned()], ..Default::default() };
        assert!(crate::trash::choose(&found, Some(&pick)).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn activity_absence_is_distinct_from_errors_and_invalid_dates() {
        use std::io::{Error, ErrorKind};
        assert_eq!(activity_timestamp(Err(Error::from(ErrorKind::NotFound)), false), Some(None));
        for kind in [ErrorKind::PermissionDenied, ErrorKind::NotADirectory, ErrorKind::Other] {
            assert_eq!(activity_timestamp(Err(Error::from(kind)), false), None);
        }
        let root = fixture("build-git-invalid-date");
        let index = root.join(".git").join("index");
        write(&index, 20, 90);
        std::fs::File::options().write(true).open(&index).unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH)).unwrap();
        assert_eq!(newest_activity(&root, false), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Folders get "now" as their mtime when files are made in them; age them like their files.
    fn age_dir(dir: &Path, days: u64) {
        crate::trash::tests::touch_directory(dir, days * 86_400);
    }
}
