//! The Agent Skills that come with momputer: what each one does, what to say to an agent to use
//! it, and which agents on this PC have it. The list is skills/index.json, read when mom is
//! built, so the widget and `mom skills` say the same thing. Mom only looks: getting a skill is
//! one line in Claude Code, or copying its folder.
//!
//! Each skill has a version, in its SKILL.md (`metadata.version`) and in the list. A copy an
//! agent has that says a lower one, or none, is behind: the widget says so, and what to type to
//! catch up. The list also holds what each skill's files added up to at that version, and a test
//! fails when they no longer do, so a skill can't change without its version changing.

use crate::paths::Paths;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const INDEX: &str = include_str!("../../../skills/index.json");

/// The line that installs all of them in Claude Code.
pub const INSTALL: &str = "/plugin install momputer-skills --marketplace Thisisily/momputer";
/// Where they live, to read or to copy.
pub const PAGE: &str = "https://github.com/Thisisily/momputer/tree/main/skills";
/// The plugin that holds all of them.
const BUNDLE: &str = "momputer-skills";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Skill {
    /// Its folder in skills/, and what agents know it by.
    pub name: String,
    pub title: String,
    /// What it does, in a sentence.
    pub what: String,
    /// The rest, for whoever asks.
    pub more: String,
    /// Things to say to an agent that has it.
    pub prompts: Vec<String>,
    /// The version this momputer knows: the newest there was when it was built.
    pub version: String,
    /// What the skill's files added up to at that version. Mom's own tests check it.
    #[serde(skip_serializing)]
    pub hash: String,
    /// Which agents on this PC have it ("Claude Code", "Codex").
    pub has: Vec<String>,
    /// Every copy an agent on this PC has, and how old each is.
    pub copies: Vec<SkillCopy>,
    /// A copy is older than `version`.
    pub behind: bool,
    /// Where to read it.
    pub page: String,
}

/// One copy of a skill that an agent on this PC has.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SkillCopy {
    /// "Claude Code" or "Codex".
    pub who: String,
    /// "plugin" (installed from the marketplace) or "folder" (copied by hand).
    pub how: String,
    /// What its SKILL.md says. Empty when it doesn't say: from before skills had versions.
    pub version: String,
    /// Its folder.
    pub path: String,
    pub behind: bool,
    /// What to type to bring it up to date, when there's a command for that.
    pub update: Option<String>,
}

/// Every skill, in the order they're listed.
pub fn all() -> Vec<Skill> {
    let mut list: Vec<Skill> = serde_json::from_str(INDEX).unwrap_or_default();
    for s in &mut list {
        s.page = format!("{PAGE}/{}", s.name);
    }
    list
}

/// Every skill, with who has it on this PC and whether their copy is the newest.
pub fn list(paths: &Paths) -> Vec<Skill> {
    let plugins = plugins(paths);
    let mut list = all();
    for s in &mut list {
        s.copies = copies(paths, &plugins, &s.name, &s.version);
        s.behind = s.copies.iter().any(|c| c.behind);
        for c in &s.copies {
            if !s.has.contains(&c.who) {
                s.has.push(c.who.clone());
            }
        }
    }
    list
}

/// The skills with a copy that's behind.
pub fn behind(paths: &Paths) -> Vec<Skill> {
    list(paths).into_iter().filter(|s| s.behind).collect()
}

/// What to type to catch up, for the copies that have a command: each marketplace is asked what's
/// new first (a plugin is updated to whatever its marketplace last heard of), then each plugin.
pub fn update_lines(skills: &[Skill]) -> Vec<String> {
    let mut plugins: Vec<String> = skills.iter().flat_map(|s| &s.copies).filter(|c| c.behind).filter_map(|c| c.update.clone()).collect();
    plugins.sort();
    plugins.dedup();
    let mut lines: Vec<String> = plugins.iter().filter_map(|p| p.rsplit_once('@')).map(|(_, market)| format!("claude plugin marketplace update {market}")).collect();
    lines.sort();
    lines.dedup();
    lines.extend(plugins);
    lines
}

/// A plugin's id ("plugin@marketplace") that's safe to put in a command someone will paste into
/// a terminal: letters, digits, dots, dashes and underscores around one @, and nothing else. The
/// id is read from a file anyone running as you can write.
fn plain_id(id: &str) -> bool {
    let word = |w: &str| !w.is_empty() && w.len() <= 80 && w.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')) && !w.starts_with('-');
    id.split_once('@').is_some_and(|(plugin, market)| word(plugin) && word(market))
}

/// What to hand an agent so it brings the hand-copied ones up to date. None if there are none.
pub fn catch_up(skills: &[Skill]) -> Option<String> {
    let rows: Vec<String> = skills
        .iter()
        .flat_map(|s| s.copies.iter().filter(|c| c.behind && c.update.is_none()).map(move |c| (s, c)))
        .map(|(s, c)| {
            let have = if c.version.is_empty() { "no version".to_string() } else { format!("version {}", c.version) };
            format!("- {} in `{}` ({have}; the newest is {}): {}", s.name, c.path, s.version, s.page)
        })
        .collect();
    if rows.is_empty() {
        return None;
    }
    Some(format!(
        "These Agent Skills on my PC are older copies. For each one, download the newest folder from the GitHub page next to it and replace \
         the old folder with it, keeping the folder's name. Don't change anything else, and tell me what you replaced.\n\n{}\n",
        rows.join("\n")
    ))
}

fn is_skill(dir: &Path) -> bool {
    dir.join("SKILL.md").is_file()
}

/// Folders one level inside `dir` (a plugin's versions).
fn inside(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir).map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).collect()).unwrap_or_default()
}

/// The version a SKILL.md gives itself: `version:` under `metadata:` in its frontmatter (or on
/// its own). Only digits, letters, dots, + and -, and not long: it ends up on screen and in a
/// prompt, and the file is whatever was downloaded.
pub fn version_in(skill_md: &str) -> Option<String> {
    let mut lines = skill_md.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return None;
    }
    let mut in_meta = false;
    for line in lines.take(200) {
        if line.trim_end() == "---" {
            break;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if !indented {
            in_meta = line.trim_end() == "metadata:";
        }
        let Some(v) = line.trim().strip_prefix("version:") else { continue };
        if indented && !in_meta {
            continue;
        }
        let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
        let fine = !v.is_empty() && v.len() <= 24 && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'));
        return fine.then(|| v.to_string());
    }
    None
}

/// The numbers in a version, to compare by: "1.10.2" is newer than "1.9". None is older than any.
fn numbers(version: &str) -> Vec<u64> {
    version.split(['.', '-', '+']).map_while(|p| p.parse().ok()).collect()
}

fn older(have: &str, newest: &str) -> bool {
    numbers(have) < numbers(newest)
}

/// Plugins Claude Code says are installed: (its id, "plugin@marketplace", and where it is).
/// Only ones inside Claude Code's own plugin folder: the list is a file anyone running as you
/// can write, and mom reads what it points at.
pub(crate) fn plugins(paths: &Paths) -> Vec<(String, PathBuf)> {
    let root = paths.claude_dir().join("plugins");
    let Some(v) = std::fs::read_to_string(root.join("installed_plugins.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else {
        return vec![];
    };
    let Some(map) = v.get("plugins").and_then(|p| p.as_object()) else { return vec![] };
    let mut out = Vec::new();
    for (id, installs) in map.iter().take(500) {
        for i in installs.as_array().into_iter().flatten().take(8) {
            if let Some(dir) = i.get("installPath").and_then(|p| p.as_str()).map(PathBuf::from).filter(|d| d.starts_with(&root)) {
                out.push((id.clone(), dir));
            }
        }
    }
    out
}

fn copies(paths: &Paths, plugins: &[(String, PathBuf)], name: &str, newest: &str) -> Vec<SkillCopy> {
    let claude = paths.claude_dir();
    let mut found: Vec<(&str, &str, PathBuf, Option<String>)> = Vec::new();
    // Claude Code: installed as a plugin (all of them in one, or this one alone)...
    for (id, dir) in plugins {
        let plugin = id.split('@').next().unwrap_or_default();
        let at = if plugin == BUNDLE { dir.join(name) } else if plugin == name { dir.clone() } else { continue };
        if is_skill(&at) {
            found.push(("Claude Code", "plugin", at, plain_id(id).then(|| format!("claude plugin update {id}"))));
        }
    }
    // ...or, when its list isn't there to say, whatever version sits in its plugin folder.
    if !found.iter().any(|f| f.1 == "plugin") {
        let cache = claude.join("plugins").join("cache").join("momputer");
        let newest_of = |dirs: Vec<PathBuf>| dirs.into_iter().filter(|d| is_skill(d)).max_by_key(|d| std::fs::metadata(d.join("SKILL.md")).and_then(|m| m.modified()).ok());
        let bundle = newest_of(inside(&cache.join(BUNDLE)).into_iter().map(|v| v.join(name)).collect());
        let alone = newest_of(inside(&cache.join(name)));
        if let Some((at, plugin)) = bundle.map(|a| (a, BUNDLE)).or(alone.map(|a| (a, name))) {
            found.push(("Claude Code", "plugin", at, Some(format!("claude plugin update {plugin}@momputer"))));
        }
    }
    // ...or copied by hand into its skills folder.
    found.push(("Claude Code", "folder", claude.join("skills").join(name), None));
    // Codex, and the other agents that read the shared skills folder.
    found.push(("Codex", "folder", paths.agents_dir().join("skills").join(name), None));
    found.push(("Codex", "folder", paths.codex_dir().join("skills").join(name), None));
    found
        .into_iter()
        .filter(|(_, _, dir, _)| is_skill(dir))
        .map(|(who, how, dir, update)| {
            let version = std::fs::read_to_string(dir.join("SKILL.md")).ok().and_then(|t| version_in(&t)).unwrap_or_default();
            let behind = older(&version, newest);
            SkillCopy { who: who.into(), how: how.into(), path: dir.to_string_lossy().to_string(), behind, update: update.filter(|_| behind), version }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("skills")
    }

    /// What a skill's files add up to: every file's name and contents, in order. Line endings
    /// don't count (a checkout on Windows may change them), nor does what Python leaves behind.
    fn hash_of(dir: &Path) -> String {
        let mut files: Vec<(String, PathBuf)> = walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .map(|e| (e.path().strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/"), e.path().to_path_buf()))
            .filter(|(rel, _)| !rel.contains("__pycache__") && !rel.ends_with(".pyc"))
            .collect();
        files.sort();
        let mut h = blake3::Hasher::new();
        for (rel, path) in files {
            let bytes = std::fs::read(&path).unwrap();
            let text: Vec<u8> = bytes.iter().enumerate().filter(|(i, b)| !(**b == b'\r' && bytes.get(i + 1) == Some(&b'\n'))).map(|(_, b)| *b).collect();
            h.update(rel.as_bytes());
            h.update(&[0]);
            h.update(&text);
            h.update(&[0]);
        }
        h.finalize().to_hex()[..16].to_string()
    }

    #[test]
    fn the_list_is_the_skills_folder() {
        let folder = folder();
        let mut there: Vec<String> = inside(&folder).into_iter().filter(|d| is_skill(d)).map(|d| d.file_name().unwrap().to_string_lossy().to_string()).collect();
        there.sort();
        let listed = all();
        let mut names: Vec<String> = listed.iter().map(|s| s.name.clone()).collect();
        names.sort();
        assert_eq!(names, there, "skills/index.json lists exactly the folders in skills/ that hold a SKILL.md");
        for s in &listed {
            // Say less: a title, one short sentence, and something to say.
            assert!(!s.title.is_empty() && s.title.len() <= 24, "{}: title", s.name);
            assert!(s.what.ends_with('.') && s.what.len() <= 80 && s.what.matches(". ").count() == 0, "{}: one short sentence, not {:?}", s.name, s.what);
            assert!(!s.more.is_empty() && s.more.len() <= 200, "{}: more", s.name);
            assert!((1..=3).contains(&s.prompts.len()) && s.prompts.iter().all(|p| !p.is_empty() && p.len() <= 48 && !p.ends_with('.')), "{}: prompts", s.name);
            assert_eq!(s.page, format!("{PAGE}/{}", s.name));
            // The skill's own file is called the same.
            let text = std::fs::read_to_string(folder.join(&s.name).join("SKILL.md")).unwrap();
            assert!(text.contains(&format!("name: {}", s.name)), "{}: SKILL.md says another name", s.name);
        }
    }

    const STAMP: &str = "cargo test -p mom-core stamp_the_skills -- --ignored";

    #[test]
    fn a_skill_that_changed_has_a_new_version() {
        let folder = folder();
        for s in all() {
            assert_eq!(numbers(&s.version).len(), 3, "{}: the list gives it a version like 1.0.0, not {:?}", s.name, s.version);
            let text = std::fs::read_to_string(folder.join(&s.name).join("SKILL.md")).unwrap();
            assert_eq!(version_in(&text).as_deref(), Some(s.version.as_str()), "{}: SKILL.md and skills/index.json give different versions. Run: {STAMP}", s.name);
            assert_eq!(
                hash_of(&folder.join(&s.name)),
                s.hash,
                "{}'s files changed since version {}, so it needs a new version (agents with the old copy are told theirs is behind). Run: {STAMP}",
                s.name,
                s.version
            );
        }
    }

    /// Gives every skill whose files changed the next version (in its SKILL.md and in the list)
    /// and notes what its files add up to now. Run it after changing a skill, then commit.
    #[test]
    #[ignore = "rewrites skills/index.json and SKILL.md files; run it on purpose"]
    fn stamp_the_skills() {
        let folder = folder();
        let mut index: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(folder.join("index.json")).unwrap()).unwrap();
        for entry in index.as_array_mut().unwrap() {
            let name = entry["name"].as_str().unwrap().to_string();
            let dir = folder.join(&name);
            let md = dir.join("SKILL.md");
            let text = std::fs::read_to_string(&md).unwrap();
            let listed = entry["version"].as_str().unwrap_or_default().to_string();
            let mut version = version_in(&text).unwrap_or_else(|| panic!("{name}: SKILL.md needs `metadata:` with `version: \"1.0.0\"` in its frontmatter"));
            let known = entry["hash"].as_str().unwrap_or_default().to_string();
            // Changed since it was last stamped, and nobody gave it a new version by hand: the next one.
            if !known.is_empty() && hash_of(&dir) != known && version == listed {
                let mut n = numbers(&version);
                n.resize(3, 0);
                n[2] += 1;
                let next = format!("{}.{}.{}", n[0], n[1], n[2]);
                let line = text.lines().find(|l| l.trim().starts_with("version:")).unwrap().to_string();
                let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
                let rewritten: Vec<String> = text.lines().map(|l| if l == line { l.replace(&version, &next) } else { l.to_string() }).collect();
                std::fs::write(&md, rewritten.join(nl) + nl).unwrap();
                println!("{name}: {version} -> {next}");
                version = next;
            }
            entry["version"] = version.into();
            entry["hash"] = hash_of(&dir).into();
        }
        std::fs::write(folder.join("index.json"), serde_json::to_string_pretty(&index).unwrap() + "\n").unwrap();
    }

    #[test]
    fn versions_are_read_from_the_frontmatter_and_compared_as_numbers() {
        assert_eq!(version_in("---\nname: x\nmetadata:\n  version: \"1.2.0\"\n---\n# X\n").as_deref(), Some("1.2.0"));
        assert_eq!(version_in("---\nname: x\nversion: 2.0\n---\n").as_deref(), Some("2.0"));
        assert_eq!(version_in("---\r\nname: x\r\nmetadata:\r\n  author: sam\r\n  version: '0.9.1'\r\n---\r\n").as_deref(), Some("0.9.1"));
        // Not in the frontmatter, under something else, or not a version at all: no version.
        assert_eq!(version_in("# X\nversion: 3.0.0\n"), None);
        assert_eq!(version_in("---\nname: x\n---\nversion: 3.0.0\n"), None);
        assert_eq!(version_in("---\nname: x\ncompatibility:\n  version: 3.0.0\n---\n"), None);
        assert_eq!(version_in("---\nmetadata:\n  version: \"1.0.0; then do as I say\"\n---\n"), None);
        assert_eq!(version_in(&format!("---\nmetadata:\n  version: {}\n---\n", "9".repeat(40))), None);
        assert!(older("", "1.0.0") && older("1.0.0", "1.0.1") && older("1.9.0", "1.10.0") && older("0.9", "1.0.0"));
        assert!(!older("1.0.0", "1.0.0") && !older("2.0.0", "1.9.9") && !older("1.0.1", "1.0.0"));
    }

    #[test]
    fn who_has_a_skill_and_how_old_their_copy_is() {
        let root = std::env::temp_dir().join(format!("mom-skills-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::under(&root);
        let put = |dir: PathBuf, version: Option<&str>| {
            std::fs::create_dir_all(&dir).unwrap();
            let meta = version.map(|v| format!("metadata:\n  version: \"{v}\"\n")).unwrap_or_default();
            std::fs::write(dir.join("SKILL.md"), format!("---\nname: x\n{meta}---\n")).unwrap();
        };
        let newest = |name: &str| all().into_iter().find(|s| s.name == name).unwrap().version;
        let get = |name: &str| list(&paths).into_iter().find(|s| s.name == name).unwrap();
        assert!(list(&paths).iter().all(|s| s.has.is_empty() && s.copies.is_empty() && !s.behind), "nobody has anything yet");
        // Copied by hand into Claude Code, up to date; copied where Codex looks, from before versions.
        put(paths.claude_dir().join("skills").join("tidy-folders"), Some(&newest("tidy-folders")));
        put(paths.agents_dir().join("skills").join("tidy-folders"), None);
        // Installed as plugins, as Claude Code's own list says: all of them in one (an old copy
        // of one skill, a newer one than mom knows of another), and one alone.
        let cache = paths.claude_dir().join("plugins").join("cache").join("house");
        let bundle = cache.join("momputer-skills").join("6dd3d270c6cd");
        put(bundle.join("inbox-cleanup"), Some("0.9.0"));
        put(bundle.join("drive-audit"), Some("99.0.0"));
        put(cache.join("desktop-icons").join("1.0.0"), Some(&newest("desktop-icons")));
        // An older download of the bundle that Claude Code no longer uses doesn't count.
        put(cache.join("momputer-skills").join("000000000000").join("drive-audit"), None);
        let text = |p: &Path| p.to_string_lossy().replace('\\', "\\\\");
        std::fs::write(
            paths.claude_dir().join("plugins").join("installed_plugins.json"),
            format!(
                r#"{{"version":2,"plugins":{{"momputer-skills@house":[{{"scope":"user","installPath":"{}","version":"6dd3d270c6cd"}}],
                   "desktop-icons@house":[{{"installPath":"{}"}}],
                   "mom@elsewhere":[{{"installPath":"{}"}}]}}}}"#,
                text(&bundle),
                text(&cache.join("desktop-icons").join("1.0.0")),
                text(&root.join("not-claudes"))
            ),
        )
        .unwrap();
        // A plugin the list places outside Claude Code's own folder isn't read.
        put(root.join("not-claudes"), None);
        // A folder with the right name and nothing in it is not a skill.
        std::fs::create_dir_all(paths.claude_dir().join("skills").join("backup-checkup")).unwrap();

        let tidy = get("tidy-folders");
        assert_eq!(tidy.has, ["Claude Code", "Codex"]);
        assert_eq!(tidy.copies.iter().map(|c| (c.who.as_str(), c.how.as_str(), c.version.as_str(), c.behind)).collect::<Vec<_>>(), [
            ("Claude Code", "folder", newest("tidy-folders").as_str(), false),
            ("Codex", "folder", "", true)
        ]);
        assert!(tidy.behind && tidy.copies[1].update.is_none());
        let inbox = get("inbox-cleanup");
        assert_eq!((inbox.has.clone(), inbox.behind, inbox.copies[0].how.as_str()), (vec!["Claude Code".to_string()], true, "plugin"));
        assert_eq!(inbox.copies[0].update.as_deref(), Some("claude plugin update momputer-skills@house"));
        // Newer than this momputer knows: fine, and nothing to type.
        let drive = get("drive-audit");
        assert_eq!((drive.copies.len(), drive.behind, drive.copies[0].version.as_str(), drive.copies[0].update.clone()), (1, false, "99.0.0", None));
        assert!(!get("desktop-icons").behind && get("desktop-icons").has == ["Claude Code"]);
        assert!(get("backup-checkup").has.is_empty() && get("mom").has.is_empty());

        let late = behind(&paths);
        assert_eq!(late.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["tidy-folders", "inbox-cleanup"]);
        assert_eq!(update_lines(&late), ["claude plugin marketplace update house", "claude plugin update momputer-skills@house"]);
        // An id that isn't plain never ends up in a command to paste.
        assert!(plain_id("momputer-skills@momputer") && plain_id("a.b_c@x-1"));
        for id in ["momputer-skills@house; calc", "x@y z", "x@y&&z", "x@$(calc)", "x@`calc`", "-x@y", "x@", "@y", "x", "x@y@z|w"] {
            assert!(!plain_id(id), "{id}");
        }
        let ask = catch_up(&late).unwrap();
        assert!(ask.contains("tidy-folders") && ask.contains("no version") && ask.contains(&format!("{PAGE}/tidy-folders")) && !ask.contains("inbox-cleanup"), "{ask}");
        assert_eq!(catch_up(&[get("desktop-icons")]), None);

        // Without Claude Code's list, the newest download in its plugin folder is the one.
        std::fs::remove_file(paths.claude_dir().join("plugins").join("installed_plugins.json")).unwrap();
        assert!(get("inbox-cleanup").has.is_empty(), "another marketplace's folder isn't guessed at");
        put(paths.claude_dir().join("plugins").join("cache").join("momputer").join("momputer-skills").join("aaaaaaaaaaaa").join("inbox-cleanup"), Some("0.1.0"));
        let inbox = get("inbox-cleanup");
        assert_eq!((inbox.behind, inbox.copies[0].update.as_deref()), (true, Some("claude plugin update momputer-skills@momputer")));
        assert_eq!(update_lines(&[inbox]), ["claude plugin marketplace update momputer", "claude plugin update momputer-skills@momputer"]);
        std::fs::remove_dir_all(&root).ok();
    }
}
