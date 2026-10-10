//! Messy rooms: folders agents have cluttered with dated report folders, `-v2`/`-v3`/`-final`
//! copies, screenshot piles, temp files and empty shells.
//!
//! Mom proposes a plan; nothing happens until it's applied. Moves are journaled and can be
//! undone, deletions go to the Recycle Bin, and empty folders are the only thing removed
//! outright (they're empty).

use crate::config::TidyConfig;
use crate::util::{human_bytes, now_secs};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FindingKind {
    LooseReports,
    DatedFolders,
    Family,
    ScreenshotPile,
    TempDebris,
    EmptyFolders,
    Misspelled,
    Crowded,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub kind: FindingKind,
    pub title: String,
    pub detail: String,
    pub paths: Vec<String>,
    pub bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Op {
    /// Move every target into the `to` folder.
    Move,
    /// Send every target to the Recycle Bin.
    Recycle,
    /// Remove folders that hold no files at all.
    RemoveEmpty,
    /// Rename the single target to `to`.
    Rename,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    pub id: String,
    pub op: Op,
    pub targets: Vec<String>,
    pub to: Option<String>,
    pub title: String,
    pub reason: String,
    pub bytes: u64,
    /// Mom's suggestion for whether to include it. The user decides.
    pub selected: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomReport {
    pub path: String,
    pub name: String,
    pub grade: char,
    pub score: u32,
    pub line: String,
    pub entries: usize,
    /// Has code in it (package.json, .git, ...). Moving things can break paths, so mom is careful.
    pub code_project: bool,
    pub findings: Vec<Finding>,
    pub actions: Vec<Action>,
    pub scanned_at: u64,
}

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).unwrap())
}

fn date_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"(?:19|20)\d{2}[-_.]?(?:0[1-9]|1[0-2])[-_.]?(?:0[1-9]|[12]\d|3[01])")
}

fn shot_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(
        &R,
        r"(?i)(screenshot|screen[-_ ]?shot|^shot|[-_]shot|capture|snap|^page[-_]?\d|^step[-_]?\d|^frame[-_]?\d|playwright|^\d+$|^img[-_]?\d+|^image[-_]?\d+|^test[-_])",
    )
}

pub(crate) const REPORT_WORDS: &[&str] = &[
    "report", "summary", "plan", "notes", "analysis", "review", "audit", "uploads", "status", "results",
    "findings", "handoff", "recap", "retro", "investigation", "debug", "output", "export", "log",
];

pub(crate) const DOC_EXT: &[&str] = &["md", "txt", "json", "csv", "html", "htm", "pdf", "log", "yaml", "yml", "xlsx", "docx", "tsv"];
pub(crate) const IMG_EXT: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp"];
const MEDIA_EXT: &[&str] = &[
    "mp4", "mov", "mkv", "webm", "avi", "wav", "mp3", "flac", "aac", "m4a", "ogg", "blend", "psd", "aep", "prproj",
    "drp", "exr", "tif", "tiff", "raw", "cr2", "nef", "fbx", "obj", "glb", "gltf", "usd", "usdz", "kra", "afphoto",
];
const SKIP_DIRS: &[&str] = &[
    ".git", "node_modules", ".venv", "venv", "target", ".next", "dist", "build", ".turbo", ".cache", "__pycache__",
    ".pnpm-store", ".svelte-kit", ".nuxt", ".output", "vendor", ".idea", ".vscode",
];
const ARTIFACT_DIRS: &[&str] = &["test-results", "playwright-report", ".playwright-mcp", ".pytest_cache", ".nyc_output", "coverage", "__pycache__"];

pub(crate) fn is_project_file(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    const EXACT: &[&str] = &[
        "agents.md", "claude.md", "gemini.md", "package.json", "package-lock.json", "pnpm-lock.yaml",
        "pnpm-workspace.yaml", "yarn.lock", "bun.lock", "bun.lockb", "jsconfig.json", "cargo.toml", "cargo.lock",
        "pyproject.toml", "setup.py", "setup.cfg", "pipfile", "pipfile.lock", "poetry.lock", "uv.lock", "makefile",
        "dockerfile", "vercel.json", "netlify.toml", "wrangler.toml", "wrangler.json", "wrangler.jsonc",
        "firebase.json", "manifest.json", "components.json", "biome.json", "deno.json", "deno.jsonc", "go.mod",
        "go.sum", "gemfile", "gemfile.lock", "composer.json", "composer.lock", "mkdocs.yml", "mint.json", "docs.json",
        "turbo.json", "nx.json", "lerna.json", "renovate.json", "index.html", "robots.txt", "sitemap.xml",
        ".gitignore", ".gitattributes", ".editorconfig", ".npmrc", ".nvmrc", ".prettierrc", "requirements.txt",
        "skill.md",
    ];
    EXACT.contains(&n.as_str())
        || ["readme", "license", "licence", "changelog", "contributing", "code_of_conduct", "security", "tsconfig", "docker-compose", "compose.", "eslint", ".eslintrc", ".prettierrc", ".env", "requirements"]
            .iter()
            .any(|p| n.starts_with(p))
        || n.contains(".config.")
}

pub(crate) fn ext_of(name: &str) -> String {
    name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

pub(crate) fn has_date(name: &str) -> bool {
    date_re().is_match(name)
}

#[derive(Default, Clone, Copy)]
struct Profile {
    bytes: u64,
    files: u64,
    media_bytes: u64,
    docs: u64,
    images: u64,
}

fn profile(dir: &Path) -> Profile {
    let mut p = Profile::default();
    for e in walkdir::WalkDir::new(dir)
        .max_depth(4)
        .into_iter()
        .filter_entry(|e| !SKIP_DIRS.contains(&e.file_name().to_string_lossy().to_ascii_lowercase().as_str()))
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .take(5000)
    {
        let len = e.metadata().map(|m| m.len()).unwrap_or(0);
        let ext = ext_of(&e.file_name().to_string_lossy());
        p.bytes += len;
        p.files += 1;
        if MEDIA_EXT.contains(&ext.as_str()) {
            p.media_bytes += len;
        } else if DOC_EXT.contains(&ext.as_str()) {
            p.docs += 1;
        } else if IMG_EXT.contains(&ext.as_str()) {
            p.images += 1;
        }
    }
    p
}

impl Profile {
    /// Looks like a write-up or a work log rather than real content.
    fn report_like(&self) -> bool {
        self.files > 0 && self.docs > 0 && self.media_bytes * 20 <= self.bytes.max(1) && self.bytes < 300 * 1024 * 1024
    }
}

fn tokens(name: &str) -> Vec<String> {
    let without_date = date_re().replace_all(name, " ");
    without_date
        .to_ascii_lowercase()
        .split(['-', '_', ' ', '.'])
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect()
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Groups look-alike sibling folders: `radio-night-show-v3-2026-10-09`, `...-v4-...`, ...
/// Returns (family name, member names).
pub fn families(names: &[String], min: usize) -> Vec<(String, Vec<String>)> {
    let toks: Vec<(String, Vec<String>)> = names.iter().map(|n| (n.clone(), tokens(n))).collect();
    let mut candidates: BTreeMap<(usize, String), Vec<String>> = BTreeMap::new();
    for (name, t) in &toks {
        for k in 1..=t.len().min(4) {
            let prefix = t[..k].join("-");
            if prefix.len() < 4 || prefix.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            candidates.entry((k, prefix)).or_default().push(name.clone());
        }
    }
    let mut ordered: Vec<((usize, String), Vec<String>)> = candidates.into_iter().filter(|(_, m)| m.len() >= min).collect();
    // Most specific first, then biggest.
    ordered.sort_by(|a, b| b.0 .0.cmp(&a.0 .0).then(b.1.len().cmp(&a.1.len())));
    let mut taken: HashSet<String> = HashSet::new();
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for ((k, prefix), members) in ordered {
        let free: Vec<String> = members.into_iter().filter(|m| !taken.contains(m)).collect();
        if free.len() < min {
            continue;
        }
        // A family made only of a parent token ("podcast") must not swallow a more specific one.
        for m in &free {
            taken.insert(m.clone());
        }
        let mut fam = free;
        if k >= 2 {
            // Stragglers that share the first word ("radio-room-concepts" next to the
            // "radio-night-show-*" family) come along, if there are only a couple.
            let first = prefix.split('-').next().unwrap_or("").to_string();
            let stragglers: Vec<String> = toks
                .iter()
                .filter(|(n, t)| !taken.contains(n) && t.first() == Some(&first))
                .map(|(n, _)| n.clone())
                .collect();
            if !stragglers.is_empty() && stragglers.len() <= 2 {
                for s in stragglers {
                    taken.insert(s.clone());
                    fam.push(s);
                }
            }
        }
        fam.sort();
        out.push((prefix, fam));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

pub fn inspect(room: &Path, cfg: &TidyConfig) -> anyhow::Result<RoomReport> {
    anyhow::ensure!(room.is_dir(), "{} is not a folder", room.display());
    let mut findings: Vec<Finding> = Vec::new();
    let mut actions: Vec<Action> = Vec::new();
    let s = |p: &Path| p.to_string_lossy().to_string();
    let mut aid = 0usize;
    let mut next_id = || {
        aid += 1;
        format!("a{aid}")
    };

    let mut dirs: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    for e in std::fs::read_dir(room)?.filter_map(Result::ok) {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') && !ARTIFACT_DIRS.contains(&name.as_str()) {
            continue;
        }
        match e.file_type() {
            Ok(t) if t.is_dir() => dirs.push(name),
            Ok(_) => files.push(name),
            Err(_) => {}
        }
    }
    dirs.sort();
    files.sort();
    let entries = dirs.len() + files.len();
    let code_project = ["package.json", "Cargo.toml", "pyproject.toml", "go.mod", "deno.json", "requirements.txt"]
        .iter()
        .any(|f| room.join(f).exists())
        || room.join(".git").exists();

    // Where reports belong. Reuse an existing folder that matches, even with odd casing.
    let reports_name = dirs
        .iter()
        .find(|d| d.eq_ignore_ascii_case(&cfg.reports_folder))
        .cloned()
        .unwrap_or_else(|| cfg.reports_folder.clone());
    let reports_dir = room.join(&reports_name);

    // A near-miss of the reports folder ("Docuements") is worth pointing out.
    for d in &dirs {
        let dist = levenshtein(&d.to_lowercase(), &cfg.reports_folder.to_lowercase());
        if dist > 0 && dist <= 2 && d.len() >= 5 && !d.eq_ignore_ascii_case(&cfg.reports_folder) {
            findings.push(Finding {
                kind: FindingKind::Misspelled,
                title: format!("\"{d}\" looks like a typo of \"{}\"", cfg.reports_folder),
                detail: "Agents copy folder names exactly. A typo now means reports end up in two places later.".into(),
                paths: vec![s(&room.join(d))],
                bytes: 0,
            });
            if !room.join(&cfg.reports_folder).exists() {
                actions.push(Action {
                    id: next_id(),
                    op: Op::Rename,
                    targets: vec![s(&room.join(d))],
                    to: Some(s(&room.join(&cfg.reports_folder))),
                    title: format!("Rename {d} to {}", cfg.reports_folder),
                    reason: "Fix the folder name so everyone files reports in the same place.".into(),
                    bytes: 0,
                    selected: false,
                });
            }
        }
    }

    // Loose reports at the top level.
    let loose: Vec<String> = files
        .iter()
        .filter(|f| {
            let ext = ext_of(f);
            if !DOC_EXT.contains(&ext.as_str()) || is_project_file(f) {
                return false;
            }
            let l = f.to_ascii_lowercase();
            has_date(f) || REPORT_WORDS.iter().any(|w| l.contains(w))
        })
        .cloned()
        .collect();
    if !loose.is_empty() {
        let paths: Vec<String> = loose.iter().map(|f| s(&room.join(f))).collect();
        let bytes = paths.iter().map(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)).sum();
        findings.push(Finding {
            kind: FindingKind::LooseReports,
            title: format!("{} report file(s) lying around the top level", loose.len()),
            detail: format!("Reports, plans and logs belong in {reports_name}/."),
            paths: paths.clone(),
            bytes,
        });
        actions.push(Action {
            id: next_id(),
            op: Op::Move,
            targets: paths,
            to: Some(s(&reports_dir)),
            title: format!("Move {} report file(s) into {reports_name}/", loose.len()),
            reason: "Reports go in one clearly named place.".into(),
            bytes,
            selected: !code_project,
        });
    }

    // Folder families and dated folders.
    let candidates: Vec<String> = dirs
        .iter()
        .filter(|d| !d.eq_ignore_ascii_case(&reports_name) && !ARTIFACT_DIRS.contains(&d.as_str()))
        .cloned()
        .collect();
    let profiles: BTreeMap<String, Profile> = candidates.iter().map(|d| (d.clone(), profile(&room.join(d)))).collect();
    let fams = families(&candidates, cfg.min_family.max(2));
    let mut in_family: HashSet<String> = HashSet::new();
    for (fam, members) in &fams {
        let all_reports = members.iter().all(|m| has_date(m) && profiles[m].report_like());
        let target = if all_reports { reports_dir.join(fam) } else { room.join(fam) };
        let movers: Vec<String> = members
            .iter()
            .filter(|m| !m.eq_ignore_ascii_case(fam))
            .map(|m| s(&room.join(m)))
            .collect();
        let bytes = members.iter().map(|m| profiles[m].bytes).sum();
        for m in members {
            in_family.insert(m.clone());
        }
        findings.push(Finding {
            kind: FindingKind::Family,
            title: format!("{} look-alike folders: {fam}-*", members.len()),
            detail: if all_reports {
                format!("Dated work logs. They belong together in {reports_name}/{fam}/.")
            } else {
                format!("Versions and variants of the same thing. One folder, {fam}/, keeps them together.")
            },
            paths: members.iter().map(|m| s(&room.join(m))).collect(),
            bytes,
        });
        if !movers.is_empty() {
            let shown = target.strip_prefix(room).unwrap_or(&target).to_string_lossy().replace('\\', "/");
            actions.push(Action {
                id: next_id(),
                op: Op::Move,
                targets: movers.clone(),
                to: Some(s(&target)),
                title: format!("Group {} folder(s) into {shown}/", movers.len()),
                reason: "One thing, one folder.".into(),
                bytes,
                selected: !code_project,
            });
        }
    }
    let dated: Vec<String> = candidates
        .iter()
        .filter(|d| has_date(d) && !in_family.contains(*d))
        .cloned()
        .collect();
    if !dated.is_empty() {
        let (reports, content): (Vec<String>, Vec<String>) = dated.into_iter().partition(|d| profiles[d].report_like());
        if !reports.is_empty() {
            let paths: Vec<String> = reports.iter().map(|d| s(&room.join(d))).collect();
            let bytes = reports.iter().map(|d| profiles[d].bytes).sum();
            findings.push(Finding {
                kind: FindingKind::DatedFolders,
                title: format!("{} dated report folder(s) at the top level", reports.len()),
                detail: format!("Write-ups named after the day they were made. They go in {reports_name}/."),
                paths: paths.clone(),
                bytes,
            });
            actions.push(Action {
                id: next_id(),
                op: Op::Move,
                targets: paths,
                to: Some(s(&reports_dir)),
                title: format!("Move {} dated report folder(s) into {reports_name}/", reports.len()),
                reason: "Reports go in one place, not scattered by date.".into(),
                bytes,
                selected: !code_project,
            });
        }
        if !content.is_empty() {
            findings.push(Finding {
                kind: FindingKind::DatedFolders,
                title: format!("{} dated folder(s) with real content", content.len()),
                detail: "Named by date instead of by what they are. Worth renaming or merging into the project they belong to.".into(),
                paths: content.iter().map(|d| s(&room.join(d))).collect(),
                bytes: content.iter().map(|d| profiles[d].bytes).sum(),
            });
        }
    }

    // Screenshot piles, artifact folders, temp debris and empty folders (walk the tree once).
    let mut images_by_dir: BTreeMap<PathBuf, (Vec<PathBuf>, u64, usize)> = BTreeMap::new();
    let mut debris: Vec<(PathBuf, u64)> = Vec::new();
    let mut maybe_debris: Vec<(PathBuf, u64)> = Vec::new();
    let mut artifacts: Vec<(PathBuf, u64)> = Vec::new();
    let now = now_secs();
    let walker = walkdir::WalkDir::new(room).max_depth(5).into_iter().filter_entry(|e| {
        let n = e.file_name().to_string_lossy().to_ascii_lowercase();
        e.depth() == 0 || !(SKIP_DIRS.contains(&n.as_str()) && n != "__pycache__")
    });
    for e in walker.filter_map(Result::ok) {
        let name = e.file_name().to_string_lossy().to_string();
        let lname = name.to_ascii_lowercase();
        if e.file_type().is_dir() {
            if e.depth() > 0 && ARTIFACT_DIRS.contains(&lname.as_str()) {
                artifacts.push((e.path().to_path_buf(), crate::util::dir_stats(e.path()).bytes));
            }
            continue;
        }
        let len = e.metadata().map(|m| m.len()).unwrap_or(0);
        if e.path().components().any(|c| ARTIFACT_DIRS.contains(&c.as_os_str().to_string_lossy().to_ascii_lowercase().as_str())) {
            continue; // counted with its artifact folder
        }
        let ext = ext_of(&name);
        if IMG_EXT.contains(&ext.as_str()) {
            let stem = name.rsplit_once('.').map(|(a, _)| a).unwrap_or(&name);
            let entry = images_by_dir.entry(e.path().parent().unwrap_or(room).to_path_buf()).or_default();
            entry.2 += 1;
            if shot_re().is_match(stem) {
                entry.0.push(e.path().to_path_buf());
                entry.1 += len;
            }
        } else if lname.starts_with("~$")
            || ext == "tmp"
            || ext == "temp"
            || ext == "momtmp"
            || lname == ".ds_store"
            || lname == "thumbs.db"
            || lname == "nohup.out"
            || ext == "orig"
            || ext == "rej"
        {
            debris.push((e.path().to_path_buf(), len));
        } else if ext == "bak" || (ext == "log" && e.depth() == 1 && crate::trash::age_of(e.path(), now) > 7 * 86_400) {
            maybe_debris.push((e.path().to_path_buf(), len));
        }
    }
    for (dir, (shots, bytes, total_images)) in &images_by_dir {
        if shots.len() < cfg.screenshot_pile {
            continue;
        }
        let dname = dir.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let obvious = ["screenshots", "shots", "captures", "screens", "snapshots-tmp"].contains(&dname.as_str());
        let rel = dir.strip_prefix(room).unwrap_or(dir).to_string_lossy().replace('\\', "/");
        let place = if rel.is_empty() { "the top level".to_string() } else { format!("{rel}/") };
        findings.push(Finding {
            kind: FindingKind::ScreenshotPile,
            title: format!("{} screenshots piled up in {place}", shots.len()),
            detail: format!(
                "{} of captures{}. Delete them once they've done their job (video uploaded, bug fixed).",
                human_bytes(*bytes),
                if *total_images > shots.len() { format!(", out of {total_images} images there") } else { String::new() }
            ),
            paths: vec![s(dir)],
            bytes: *bytes,
        });
        actions.push(Action {
            id: next_id(),
            op: Op::Recycle,
            targets: shots.iter().map(|p| s(p)).collect(),
            to: None,
            title: format!("Recycle {} screenshots in {place} ({})", shots.len(), human_bytes(*bytes)),
            reason: "Screenshots are temporary. The Recycle Bin keeps them for a while, just in case.".into(),
            bytes: *bytes,
            selected: obvious,
        });
    }
    // Artifact folders: test results, reports, caches. Keep only the outermost.
    artifacts.sort();
    let mut outer: Vec<(PathBuf, u64)> = Vec::new();
    for a in artifacts {
        if !outer.iter().any(|(o, _)| a.0.starts_with(o)) {
            outer.push(a);
        }
    }
    if !outer.is_empty() {
        let bytes = outer.iter().map(|a| a.1).sum();
        findings.push(Finding {
            kind: FindingKind::TempDebris,
            title: format!("{} test/cache folder(s) left behind", outer.len()),
            detail: "test-results, playwright reports, coverage and Python caches. All regenerate.".into(),
            paths: outer.iter().map(|a| s(&a.0)).collect(),
            bytes,
        });
        actions.push(Action {
            id: next_id(),
            op: Op::Recycle,
            targets: outer.iter().map(|a| s(&a.0)).collect(),
            to: None,
            title: format!("Recycle {} test/cache folder(s) ({})", outer.len(), human_bytes(bytes)),
            reason: "Generated output, safe to regenerate.".into(),
            bytes,
            selected: true,
        });
    }
    if !debris.is_empty() {
        let bytes = debris.iter().map(|d| d.1).sum();
        findings.push(Finding {
            kind: FindingKind::TempDebris,
            title: format!("{} temp file(s)", debris.len()),
            detail: "*.tmp, ~$ lock files, .DS_Store, Thumbs.db, nohup.out.".into(),
            paths: debris.iter().map(|d| s(&d.0)).collect(),
            bytes,
        });
        actions.push(Action {
            id: next_id(),
            op: Op::Recycle,
            targets: debris.iter().map(|d| s(&d.0)).collect(),
            to: None,
            title: format!("Recycle {} temp file(s)", debris.len()),
            reason: "Temp files are temporary.".into(),
            bytes,
            selected: true,
        });
    }
    if !maybe_debris.is_empty() {
        let bytes = maybe_debris.iter().map(|d| d.1).sum();
        actions.push(Action {
            id: next_id(),
            op: Op::Recycle,
            targets: maybe_debris.iter().map(|d| s(&d.0)).collect(),
            to: None,
            title: format!("Recycle {} backup/old log file(s)", maybe_debris.len()),
            reason: "*.bak files and week-old logs. Probably safe, but have a look.".into(),
            bytes,
            selected: false,
        });
    }
    let empties = empty_dirs(room);
    if !empties.is_empty() {
        findings.push(Finding {
            kind: FindingKind::EmptyFolders,
            title: format!("{} empty folder(s)", empties.len()),
            detail: "Shells left after files were moved or deleted. They make a room look full when it isn't.".into(),
            paths: empties.iter().map(|p| s(p)).collect(),
            bytes: 0,
        });
        actions.push(Action {
            id: next_id(),
            op: Op::RemoveEmpty,
            targets: empties.iter().map(|p| s(p)).collect(),
            to: None,
            title: format!("Remove {} empty folder(s)", empties.len()),
            reason: "Leave no empty folders behind.".into(),
            bytes: 0,
            selected: !code_project,
        });
    }
    if entries > 30 {
        findings.push(Finding {
            kind: FindingKind::Crowded,
            title: format!("{entries} things at the top level"),
            detail: "Past about 30, nobody can find anything. Group related folders.".into(),
            paths: vec![],
            bytes: 0,
        });
    }

    let mut score: u32 = 0;
    for f in &findings {
        let n = f.paths.len() as u32;
        score += match f.kind {
            FindingKind::LooseReports => 3 * n,
            FindingKind::DatedFolders => 3 * n,
            FindingKind::Family => n + 2,
            FindingKind::ScreenshotPile => 4 + (f.bytes / (100 * 1024 * 1024)) as u32,
            FindingKind::TempDebris => 1 + n / 10,
            FindingKind::EmptyFolders => 1 + n / 3,
            FindingKind::Misspelled => 2,
            FindingKind::Crowded => (entries.saturating_sub(25) / 5) as u32,
        };
    }
    let grade = match score {
        0..=3 => 'A',
        4..=10 => 'B',
        11..=25 => 'C',
        26..=50 => 'D',
        _ => 'F',
    };
    Ok(RoomReport {
        path: s(room),
        name: room.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| s(room)),
        grade,
        score,
        line: crate::voice::room(grade).to_string(),
        entries,
        code_project,
        findings,
        actions,
        scanned_at: now,
    })
}

/// Outermost folders under `root` that contain no files at any depth.
pub fn empty_dirs(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) -> bool {
        // Returns true if `dir` holds no files (recursively).
        let Ok(rd) = std::fs::read_dir(dir) else { return false };
        let mut empty = true;
        let mut empty_children = Vec::new();
        for e in rd.filter_map(Result::ok) {
            let ft = match e.file_type() {
                Ok(t) => t,
                Err(_) => {
                    empty = false;
                    continue;
                }
            };
            if ft.is_dir() {
                let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                if SKIP_DIRS.contains(&name.as_str()) || depth >= 8 {
                    empty = false;
                    continue;
                }
                if walk(&e.path(), depth + 1, out) {
                    empty_children.push(e.path());
                } else {
                    empty = false;
                }
            } else {
                empty = false;
            }
        }
        if !empty {
            out.extend(empty_children);
        }
        empty
    }
    let mut out = Vec::new();
    if walk(root, 0, &mut out) {
        // The whole room is empty; don't suggest deleting the room itself.
        out.clear();
    }
    out.sort();
    out
}

// ------------------------------------------------------------------ apply & undo

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEntry {
    pub op: Op,
    pub from: String,
    pub to: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Journal {
    pub room: String,
    pub at: u64,
    pub entries: Vec<JournalEntry>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyReport {
    pub moved: u32,
    pub recycled: u32,
    pub removed: u32,
    pub failed: Vec<String>,
    pub journal: Option<String>,
    pub message: String,
}

fn unique_dest(dir: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let n = name.to_string_lossy();
    let (stem, ext) = match n.rsplit_once('.') {
        Some((a, b)) if !a.is_empty() => (a.to_string(), format!(".{b}")),
        _ => (n.to_string(), String::new()),
    };
    (2..1000)
        .map(|i| dir.join(format!("{stem} ({i}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

fn has_files(dir: &Path) -> bool {
    walkdir::WalkDir::new(dir).into_iter().filter_map(Result::ok).any(|e| !e.file_type().is_dir())
}

/// Applies the selected actions (by id) of a report. Everything is journaled for `undo`.
pub fn apply(report: &RoomReport, ids: &[String], journal_dir: &Path) -> ApplyReport {
    let mut r = ApplyReport::default();
    let mut journal = Journal { room: report.path.clone(), at: now_secs(), entries: vec![] };
    let room = Path::new(&report.path);
    for a in report.actions.iter().filter(|a| ids.contains(&a.id)) {
        match a.op {
            Op::Move => {
                let Some(to) = &a.to else { continue };
                let to = PathBuf::from(to);
                if !to.starts_with(room) {
                    r.failed.push(format!("{}: destination outside the room", to.display()));
                    continue;
                }
                if let Err(e) = std::fs::create_dir_all(&to) {
                    r.failed.push(format!("{}: {e}", to.display()));
                    continue;
                }
                for t in &a.targets {
                    let src = PathBuf::from(t);
                    let Some(name) = src.file_name() else { continue };
                    if !src.exists() || to.starts_with(&src) {
                        continue;
                    }
                    let dest = unique_dest(&to, name);
                    match std::fs::rename(&src, &dest) {
                        Ok(()) => {
                            r.moved += 1;
                            journal.entries.push(JournalEntry { op: Op::Move, from: t.clone(), to: Some(dest.to_string_lossy().to_string()) });
                        }
                        Err(e) => r.failed.push(format!("{t}: {e}")),
                    }
                }
            }
            Op::Rename => {
                let (Some(t), Some(to)) = (a.targets.first(), &a.to) else { continue };
                if Path::new(to).exists() {
                    r.failed.push(format!("{to} already exists"));
                    continue;
                }
                match std::fs::rename(t, to) {
                    Ok(()) => {
                        r.moved += 1;
                        journal.entries.push(JournalEntry { op: Op::Rename, from: t.clone(), to: Some(to.clone()) });
                    }
                    Err(e) => r.failed.push(format!("{t}: {e}")),
                }
            }
            Op::Recycle => {
                for t in &a.targets {
                    if !Path::new(t).exists() {
                        continue;
                    }
                    match trash::delete(t) {
                        Ok(()) => {
                            r.recycled += 1;
                            journal.entries.push(JournalEntry { op: Op::Recycle, from: t.clone(), to: None });
                        }
                        Err(e) => r.failed.push(format!("{t}: {e}")),
                    }
                }
            }
            Op::RemoveEmpty => {
                for t in &a.targets {
                    let p = Path::new(t);
                    if !p.is_dir() || has_files(p) {
                        continue; // something moved in since the scan; leave it
                    }
                    // Remember the whole empty tree so undo can rebuild it exactly.
                    let tree: Vec<String> = walkdir::WalkDir::new(p)
                        .into_iter()
                        .filter_map(Result::ok)
                        .map(|e| e.path().to_string_lossy().to_string())
                        .collect();
                    match std::fs::remove_dir_all(p) {
                        Ok(()) => {
                            r.removed += 1;
                            for d in tree {
                                journal.entries.push(JournalEntry { op: Op::RemoveEmpty, from: d, to: None });
                            }
                        }
                        Err(e) => r.failed.push(format!("{t}: {e}")),
                    }
                }
            }
        }
    }
    if !journal.entries.is_empty() {
        let file = journal_dir.join(format!("tidy-{}.json", journal.at));
        if std::fs::create_dir_all(journal_dir).is_ok() {
            if let Ok(text) = serde_json::to_string_pretty(&journal) {
                if std::fs::write(&file, text).is_ok() {
                    r.journal = Some(file.to_string_lossy().to_string());
                }
            }
        }
    }
    let mut parts = Vec::new();
    if r.moved > 0 {
        parts.push(format!("put away {} thing(s)", r.moved));
    }
    if r.recycled > 0 {
        parts.push(format!("recycled {}", r.recycled));
    }
    if r.removed > 0 {
        parts.push(format!("removed {} empty folder(s)", r.removed));
    }
    r.message = if parts.is_empty() {
        "Nothing to do.".to_string()
    } else {
        let mut m = parts.join(", ");
        m = format!("{}{}. Changed your mind? Undo is right there.", m[..1].to_uppercase(), &m[1..]);
        m
    };
    r
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoReport {
    pub restored: u32,
    pub failed: Vec<String>,
    pub message: String,
}

/// Reverses the most recent tidy-up.
pub fn undo_last(journal_dir: &Path) -> UndoReport {
    let mut r = UndoReport::default();
    let mut files: Vec<PathBuf> = std::fs::read_dir(journal_dir)
        .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default();
    files.retain(|f| {
        let n = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        n.starts_with("tidy-") && n.ends_with(".json") && !n.ends_with(".undone.json")
    });
    files.sort();
    let Some(file) = files.pop() else {
        r.message = "Nothing to undo.".into();
        return r;
    };
    let journal: Journal = match std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()) {
        Some(j) => j,
        None => {
            r.message = "Couldn't read the last tidy-up's journal.".into();
            return r;
        }
    };
    let recycled: Vec<&JournalEntry> = journal.entries.iter().filter(|e| e.op == Op::Recycle).collect();
    for e in journal.entries.iter().rev() {
        match e.op {
            Op::Move | Op::Rename => {
                let Some(to) = &e.to else { continue };
                if let Some(parent) = Path::new(&e.from).parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::rename(to, &e.from) {
                    Ok(()) => r.restored += 1,
                    Err(err) => r.failed.push(format!("{to}: {err}")),
                }
            }
            Op::RemoveEmpty => {
                if std::fs::create_dir_all(&e.from).is_ok() {
                    r.restored += 1;
                }
            }
            Op::Recycle => {}
        }
    }
    if !recycled.is_empty() {
        let (ok, failed) = restore_from_recycle_bin(&recycled, journal.at);
        r.restored += ok;
        r.failed.extend(failed);
    }
    // Remove folders the tidy-up created that are now empty again.
    for e in &journal.entries {
        if let Some(to) = &e.to {
            if let Some(parent) = Path::new(to).parent() {
                if parent.is_dir() && !has_files(parent) && parent != Path::new(&journal.room) {
                    let _ = std::fs::remove_dir_all(parent);
                }
            }
        }
    }
    let _ = std::fs::rename(&file, file.with_extension("undone.json"));
    r.message = if r.failed.is_empty() {
        format!("Put back {} thing(s), exactly as they were.", r.restored)
    } else {
        format!("Put back {} thing(s); {} couldn't be restored.", r.restored, r.failed.len())
    };
    r
}

#[cfg(any(windows, all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"))))]
fn restore_from_recycle_bin(entries: &[&JournalEntry], since: u64) -> (u32, Vec<String>) {
    use trash::os_limited;
    let Ok(items) = os_limited::list() else {
        return (0, vec!["couldn't read the Recycle Bin; restore recycled items from it by hand".into()]);
    };
    let wanted: HashSet<String> = entries.iter().map(|e| crate::util::norm_path(&e.from)).collect();
    let mut matches = Vec::new();
    for item in items {
        let original = item.original_path();
        let key = crate::util::norm_path(&original.to_string_lossy());
        if wanted.contains(&key) && item.time_deleted >= since as i64 - 5 {
            matches.push(item);
        }
    }
    let n = matches.len() as u32;
    let mut failed = Vec::new();
    if n < entries.len() as u32 {
        failed.push(format!("{} recycled item(s) weren't found in the Recycle Bin", entries.len() as u32 - n));
    }
    match os_limited::restore_all(matches) {
        Ok(()) => (n, failed),
        Err(e) => {
            failed.push(format!("restoring from the Recycle Bin failed: {e}"));
            (0, failed)
        }
    }
}

#[cfg(not(any(windows, all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android")))))]
fn restore_from_recycle_bin(entries: &[&JournalEntry], _since: u64) -> (u32, Vec<String>) {
    (0, vec![format!("{} item(s) are in the Trash; restore them from there", entries.len())])
}

/// A message to paste to an agent so it tidies up (and fixes any paths that referenced the mess).
pub fn chore_prompt(report: &RoomReport) -> String {
    let mut out = format!(
        "Mom here. Your room ({}) got a {} — {}\n\nPlease tidy it up:\n",
        report.path, report.grade, report.line
    );
    for f in &report.findings {
        out.push_str(&format!("- {}. {}\n", f.title, f.detail));
        for p in f.paths.iter().take(12) {
            let rel = Path::new(p).strip_prefix(&report.path).map(|r| r.to_string_lossy().to_string()).unwrap_or_else(|_| p.clone());
            out.push_str(&format!("    {rel}\n"));
        }
        if f.paths.len() > 12 {
            out.push_str(&format!("    …and {} more\n", f.paths.len() - 12));
        }
    }
    out.push_str(
        "\nRules: reports go in Documents/ with clear names. One thing, one folder (no -v2/-v3/-trial siblings). \
         Delete temp files and screenshots that have done their job. Remove folders you empty. \
         Update any scripts, configs or docs that point at paths you move, then tell me exactly what you changed.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mkdir(p: &Path) {
        std::fs::create_dir_all(p).unwrap();
    }
    fn write(p: &Path, s: &str) {
        mkdir(p.parent().unwrap());
        std::fs::write(p, s).unwrap();
    }

    /// The folder from the conversation that started momputer.
    fn field_notes(root: &Path) {
        for d in [
            "shorts", "spiders", "survival", "swears", "turks", "beige", "garden", "blender", "brits", "danes",
            "egyptians", "eu", "found", "germans", "glassholes", "greeks", "groupchat", "hiding", "ikea", "indians",
            "iranians", "irkutsk", "irkutsk-v2", "italians", "lab", "multiverse", "persians", "plagues", "putin", "red",
        ] {
            write(&root.join(d).join("short.mp4"), &"v".repeat(2048));
        }
        for d in [
            "social-setup-2026-09-28", "podcast-ai-wording-cleanup-2026-09-30", "podcast-copy-rewrite-2026-10-01",
            "podcast-guest-tags-2026-10-01", "podcast-schedule-2026-09-30", "podcast-setup-2026-09-26",
            "podcast-upload-2026-09-29", "podcast-watermark-update-2026-09-30", "analytics-2026-10-06",
            "covers-review-2026-09-30", "publishing-2026-10-05", "review-2026-10-04", "2026-09-24",
        ] {
            write(&root.join(d).join("REPORT.md"), "# report");
            write(&root.join(d).join("data.json"), "{}");
        }
        for d in [
            "radio-night-show-2h-2026-10-09", "radio-night-show-expanded-2026-10-09",
            "radio-night-show-music-2026-10-09", "radio-night-show-trial-2026-10-09",
            "radio-night-show-v3-2026-10-09", "radio-night-show-v4-2026-10-09",
            "radio-night-show-v5-2026-10-09", "radio-room-concepts-2026-10-08",
        ] {
            write(&root.join(d).join("track.wav"), &"a".repeat(4096));
            write(&root.join(d).join("notes.md"), "notes");
        }
        mkdir(&root.join("micro-2026-10-08").join("renders"));
        write(&root.join("micro-2026-10-08").join("renders").join("a.mp4"), &"m".repeat(4096));
        mkdir(&root.join("dalir-ad"));
        write(&root.join("dalir-ad").join("ad.mp4"), &"d".repeat(1024));
        write(&root.join("COPY.md"), "copy");
        write(&root.join("UPLOADS-2026-10-04.md"), "uploads");
        write(&root.join("podcast-plan-2026-10-04.json"), "{}");
        for i in 0..40 {
            write(&root.join("garden").join("shots").join(format!("shot-{i:02}.png")), "png");
        }
        mkdir(&root.join("found").join("old").join("empty-after-cleanup"));
        mkdir(&root.join("Docuements"));
    }

    #[test]
    fn families_match_what_a_human_would_do() {
        let names: Vec<String> = [
            "radio-night-show-2h-2026-10-09", "radio-night-show-expanded-2026-10-09", "radio-night-show-v3-2026-10-09",
            "radio-night-show-v4-2026-10-09", "radio-room-concepts-2026-10-08", "irkutsk", "irkutsk-v2", "greeks",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let f = families(&names, 3);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].0, "radio-night-show");
        assert_eq!(f[0].1.len(), 5, "the straggler radio-room-concepts comes along");
        // Two isn't a family at min 3.
        assert!(!f.iter().any(|(n, _)| n == "irkutsk"));
    }

    #[test]
    fn the_field_notes_folder_gets_the_grade_it_deserves() {
        let root = std::env::temp_dir().join(format!("mom-tidy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        field_notes(&root);
        let r = inspect(&root, &TidyConfig::default()).unwrap();
        assert_eq!(r.grade, 'F');
        assert!(!r.code_project);
        let kinds: Vec<FindingKind> = r.findings.iter().map(|f| f.kind).collect();
        for k in [FindingKind::LooseReports, FindingKind::Family, FindingKind::DatedFolders, FindingKind::ScreenshotPile, FindingKind::EmptyFolders, FindingKind::Misspelled, FindingKind::Crowded] {
            assert!(kinds.contains(&k), "missing {k:?}: {kinds:?}");
        }
        let radio = r.actions.iter().find(|a| a.to.as_deref().map(|t| t.ends_with("radio-night-show")).unwrap_or(false)).expect("radio show family move");
        assert_eq!(radio.targets.len(), 8);
        let yt = r.actions.iter().find(|a| a.to.as_deref().map(|t| t.ends_with("podcast")).unwrap_or(false)).expect("podcast reports");
        assert!(yt.to.as_ref().unwrap().contains("Documents"), "dated podcast work logs go to Documents/podcast");
        assert_eq!(yt.targets.len(), 7);
        let loose = r.findings.iter().find(|f| f.kind == FindingKind::LooseReports).unwrap();
        assert_eq!(loose.paths.len(), 2, "COPY.md is a working file; the dated ones are reports");
        // Content folders (videos) are never shuffled around.
        assert!(!r.actions.iter().any(|a| a.targets.iter().any(|t| t.ends_with("micro-2026-10-08"))));
        assert!(!r.actions.iter().any(|a| a.targets.iter().any(|t| t.ends_with("greeks"))));

        // Apply everything mom selected, then undo it all.
        let ids: Vec<String> = r.actions.iter().filter(|a| a.selected && a.op != Op::Recycle).map(|a| a.id.clone()).collect();
        let journal = root.join("_journal");
        let applied = apply(&r, &ids, &journal);
        assert!(applied.failed.is_empty(), "{:?}", applied.failed);
        assert!(root.join("radio-night-show").join("radio-night-show-v5-2026-10-09").join("track.wav").exists());
        assert!(root.join("Documents").join("podcast").join("podcast-upload-2026-09-29").exists());
        assert!(root.join("Documents").join("UPLOADS-2026-10-04.md").exists());
        assert!(!root.join("found").join("old").exists());

        let after = inspect(&root, &TidyConfig::default()).unwrap();
        assert!(after.score < r.score / 2, "much tidier: {} -> {}", r.score, after.score);

        let undone = undo_last(&journal);
        assert!(undone.failed.is_empty(), "{:?}", undone.failed);
        assert!(root.join("radio-night-show-v5-2026-10-09").join("track.wav").exists());
        assert!(root.join("podcast-upload-2026-09-29").join("REPORT.md").exists());
        assert!(root.join("UPLOADS-2026-10-04.md").exists());
        assert!(root.join("found").join("old").join("empty-after-cleanup").exists());
        assert!(!root.join("radio-night-show").exists(), "folders mom created are gone again");

        let prompt = chore_prompt(&r);
        assert!(prompt.contains("radio-night-show") && prompt.contains("Documents/"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn empty_dirs_reports_outermost_only() {
        let root = std::env::temp_dir().join(format!("mom-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        mkdir(&root.join("a").join("b").join("c"));
        write(&root.join("keep").join("file.txt"), "x");
        mkdir(&root.join("keep").join("hollow"));
        let e = empty_dirs(&root);
        assert_eq!(e, vec![root.join("a"), root.join("keep").join("hollow")]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
