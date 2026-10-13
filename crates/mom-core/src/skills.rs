//! The Agent Skills that come with momputer: what each one does, what to say to an agent to use
//! it, and which agents on this PC have it. The list is skills/index.json, read when mom is
//! built, so the widget and `mom skills` say the same thing. Mom only looks: getting a skill is
//! one line in Claude Code, or copying its folder.

use crate::paths::Paths;
use serde::{Deserialize, Serialize};
use std::path::Path;

const INDEX: &str = include_str!("../../../skills/index.json");

/// The line that installs all of them in Claude Code.
pub const INSTALL: &str = "/plugin install momputer-skills --marketplace Thisisily/momputer";
/// Where they live, to read or to copy.
pub const PAGE: &str = "https://github.com/Thisisily/momputer/tree/main/skills";

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
    /// Which agents on this PC have it ("Claude Code", "Codex").
    pub has: Vec<String>,
    /// Where to read it.
    pub page: String,
}

/// Every skill, in the order they're listed.
pub fn all() -> Vec<Skill> {
    let mut list: Vec<Skill> = serde_json::from_str(INDEX).unwrap_or_default();
    for s in &mut list {
        s.page = format!("{PAGE}/{}", s.name);
    }
    list
}

/// Every skill, with who has it on this PC.
pub fn list(paths: &Paths) -> Vec<Skill> {
    let mut list = all();
    for s in &mut list {
        s.has = who_has(paths, &s.name);
    }
    list
}

fn is_skill(dir: &Path) -> bool {
    dir.join("SKILL.md").is_file()
}

/// Folders one level inside `dir` (a plugin's versions).
fn inside(dir: &Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir).map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).collect()).unwrap_or_default()
}

fn who_has(paths: &Paths, name: &str) -> Vec<String> {
    let claude = paths.claude_dir();
    // Claude Code: copied into its skills folder, or installed as a plugin (all of them in one,
    // or this one alone). Plugins are kept by version.
    let plugins = claude.join("plugins").join("cache").join("momputer");
    let in_claude = is_skill(&claude.join("skills").join(name))
        || inside(&plugins.join("momputer-skills")).iter().any(|v| is_skill(&v.join(name)))
        || inside(&plugins.join(name)).iter().any(|v| is_skill(v));
    // Codex, and the other agents that read the shared skills folder.
    let in_codex = is_skill(&paths.agents_dir().join("skills").join(name)) || is_skill(&paths.codex_dir().join("skills").join(name));
    [(in_claude, "Claude Code"), (in_codex, "Codex")].into_iter().filter(|(has, _)| *has).map(|(_, who)| who.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_the_skills_folder() {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("skills");
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

    #[test]
    fn who_has_a_skill_is_read_from_where_agents_keep_them() {
        let root = std::env::temp_dir().join(format!("mom-skills-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::under(&root);
        let put = |dir: std::path::PathBuf| {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("SKILL.md"), "---\nname: x\n---\n").unwrap();
        };
        let has = |name: &str| list(&paths).into_iter().find(|s| s.name == name).unwrap().has;
        assert!(list(&paths).iter().all(|s| s.has.is_empty()), "nobody has anything yet");
        // Copied by hand into Claude Code; copied where Codex looks.
        put(paths.claude_dir().join("skills").join("tidy-folders"));
        put(paths.agents_dir().join("skills").join("tidy-folders"));
        // Installed as plugins: all of them in one, and one alone.
        let cache = paths.claude_dir().join("plugins").join("cache").join("momputer");
        put(cache.join("momputer-skills").join("6dd3d270c6cd").join("inbox-cleanup"));
        put(cache.join("desktop-icons").join("6dd3d270c6cd"));
        // A folder with the right name and nothing in it is not a skill.
        std::fs::create_dir_all(paths.claude_dir().join("skills").join("drive-audit")).unwrap();
        assert_eq!(has("tidy-folders"), ["Claude Code", "Codex"]);
        assert_eq!(has("inbox-cleanup"), ["Claude Code"]);
        assert_eq!(has("desktop-icons"), ["Claude Code"]);
        assert!(has("drive-audit").is_empty() && has("mom").is_empty());
        std::fs::remove_dir_all(&root).ok();
    }
}
