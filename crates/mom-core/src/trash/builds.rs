//! Build folders in projects nobody is working on: node_modules, Rust's target, .next and the
//! like. They're big, they hold no code of yours, and `npm install` or `cargo build` makes them
//! again. Projects worked on in the last month, or with something running, are left alone.

use super::{ago, name_of, tally_many, Cand, Found, TrashCtx, ENTRY_CAP, IDLE_PROJECT_DAYS, IN_USE};
use crate::paths::Paths;
use crate::util::{mtime_secs, norm_path};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Build folders a JavaScript project (one with a package.json) can make again.
const JS_BUILD: &[&str] = &["node_modules", ".next", ".turbo", ".parcel-cache", ".svelte-kit"];

/// Files that change when someone works on a project.
const ACTIVITY: &[&str] = &[
    ".git/index",
    ".git/HEAD",
    ".git/logs/HEAD",
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
    if cfg!(windows) {
        v.push(PathBuf::from(r"C:\Workspace"));
        v.push(PathBuf::from(r"D:\Workspace"));
    }
    let mut seen = HashSet::new();
    v.into_iter().filter(|p| p.is_dir() && seen.insert(norm_path(&p.to_string_lossy()))).collect()
}

struct Project {
    dir: PathBuf,
    /// Where it sits under its root: "storefront", "tools/old-cli", "storefront/apps/web".
    name: String,
    /// Newest sign of work, unix seconds (including the work on a repo or project around it).
    active: u64,
    js: bool,
    rust: bool,
}

fn newest_activity(dir: &Path, project: bool) -> u64 {
    let mut newest = ACTIVITY
        .iter()
        .filter_map(|f| std::fs::metadata(dir.join(f)).ok())
        .map(|m| mtime_secs(&m))
        .max()
        .unwrap_or(0);
    if project {
        newest = newest.max(std::fs::metadata(dir).map(|m| mtime_secs(&m)).unwrap_or(0));
    }
    newest
}

fn walk(dir: &Path, rel: &str, depth: usize, inherited: u64, seen: &mut HashSet<String>, out: &mut Vec<Project>) {
    if !seen.insert(norm_path(&dir.to_string_lossy())) {
        return;
    }
    let js = dir.join("package.json").is_file();
    let rust = dir.join("Cargo.toml").is_file();
    let project = js || rust;
    let active = if project || dir.join(".git").exists() { newest_activity(dir, project).max(inherited) } else { inherited };
    if project {
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
        walk(root, "", 0, 0, &mut seen, &mut out);
    }
    out
}

fn build_dirs(p: &Project) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if p.js {
        v.extend(JS_BUILD.iter().map(|b| p.dir.join(b)));
    }
    if p.rust {
        v.push(p.dir.join("target"));
    }
    v.into_iter()
        .filter(|d| std::fs::symlink_metadata(d).map(|m| m.is_dir()).unwrap_or(false))
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
    list.sort_by_key(|(p, _)| p.active);
    let dirs: Vec<PathBuf> = list.iter().map(|(_, d)| d.clone()).collect();
    let tallies = tally_many(&dirs, ENTRY_CAP, BUDGET);
    for ((p, dir), t) in list.into_iter().zip(tallies) {
        let mut c = Cand::new(dir.clone(), format!("{} · {}", p.name, name_of(&dir)), &t, ctx.now);
        c.age = ctx.now.saturating_sub(p.active);
        let c = if ctx.in_use(&p.dir) {
            c.verdict(false, IN_USE)
        } else if c.age < idle_after {
            let r = format!("Worked on {} ago", ago(c.age));
            c.kept_but_tickable(r)
        } else {
            let r = format!("Not worked on for {}", ago(c.age));
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
        let ws = root.join("Workspace");
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
        assert_eq!(storefront.reason, "Worked on 2 days ago");
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

    /// Folders get "now" as their mtime when files are made in them; age them like their files.
    fn age_dir(dir: &Path, days: u64) {
        let mut o = std::fs::OpenOptions::new();
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            o.write(true).custom_flags(0x0200_0000); // FILE_FLAG_BACKUP_SEMANTICS: lets us open a folder
        }
        #[cfg(not(windows))]
        o.read(true);
        let f = o.open(dir).unwrap();
        f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(days * 86_400)).unwrap();
    }
}
