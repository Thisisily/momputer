//! Keys where they shouldn't be: pasted into a shell's history, or in a file git keeps.
//!
//! A key is known by its shape: the formats services give their keys (sk-ant-, ghp_, AKIA...),
//! an allow-list, never a guess at what "looks random". Mom never shows a key, keeps one or
//! writes one down: she says how many, and whose. Blanking one rereads the file, puts
//! "[removed by mom]" where each key was, and swaps the file in whole; no copy is kept.

use crate::paths::Paths;
use crate::speed::Fix;
use crate::system::{Check, Item};
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// What a key is replaced with.
pub const REMOVED: &str = "[removed by mom]";
/// The biggest history file mom reads, and the biggest file in a project.
const MAX_HISTORY: u64 = 32 << 20;
const MAX_FILE: u64 = 1 << 20;
/// Files read per project, and in all.
const MAX_FILES: usize = 50;
const MAX_TOTAL: u64 = 64 << 20;
const ALL_TIME: Duration = Duration::from_secs(20);

/// The formats keys come in: whose, and the shape. A private key's whole block is one key.
fn shapes() -> &'static [(&'static str, Regex)] {
    static SHAPES: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    SHAPES.get_or_init(|| {
        [
            ("Anthropic", r"\bsk-ant-[A-Za-z0-9_\-]{20,}"),
            ("OpenAI", r"\bsk-(?:proj|svcacct|admin)-[A-Za-z0-9_\-]{20,}"),
            ("OpenAI", r"\bsk-[A-Za-z0-9]{32,}"),
            ("GitHub", r"\bgh[pousr]_[A-Za-z0-9]{36,}"),
            ("GitHub", r"\bgithub_pat_[A-Za-z0-9_]{22,}"),
            ("AWS", r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b"),
            ("Slack", r"\bxox[bpas]-[A-Za-z0-9\-]{10,}"),
            ("Google", r"\bAIza[0-9A-Za-z_\-]{35}"),
            ("Stripe", r"\b(?:sk|rk)_live_[0-9A-Za-z]{16,}"),
            ("Hugging Face", r"\bhf_[A-Za-z0-9]{30,}"),
            ("npm", r"\bnpm_[A-Za-z0-9]{36}\b"),
            // The whole block when it ends where it should; otherwise the line that starts it and
            // the lines of key that follow. Never more: the block's body is only what a key is made of.
            ("Private key", r"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY-----(?:[A-Za-z0-9+/=\s:,\-]*?-----END (?:[A-Z0-9]+ )*PRIVATE KEY-----|(?:\r?\n[A-Za-z0-9+/=]{16,})*)"),
        ]
        .into_iter()
        .map(|(who, re)| (who, Regex::new(re).expect("a fixed pattern")))
        .collect()
    })
}

/// Whose keys are in `text`, one entry per key, in order of where they are.
pub fn keys_in(text: &str) -> Vec<&'static str> {
    let mut found: Vec<(usize, usize, &'static str)> = Vec::new();
    for (who, re) in shapes() {
        for m in re.find_iter(text) {
            // One key, one count: an OpenAI shape inside an Anthropic one is the same key.
            if !found.iter().any(|(s, e, _)| m.start() < *e && *s < m.end()) {
                found.push((m.start(), m.end(), who));
            }
        }
    }
    found.sort();
    found.into_iter().map(|(_, _, who)| who).collect()
}

/// `text` with each key replaced, and how many there were.
pub fn blank(text: &str) -> (String, usize) {
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for (_, re) in shapes() {
        for m in re.find_iter(text) {
            spans.push((m.start(), m.end()));
        }
    }
    spans.sort();
    // Overlapping finds are one key.
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (s, e) in spans {
        match merged.last_mut() {
            Some(last) if s < last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (s, e) in &merged {
        out.push_str(&text[at..*s]);
        out.push_str(REMOVED);
        at = *e;
    }
    out.push_str(&text[at..]);
    (out, merged.len())
}

/// "OpenAI and GitHub", "Anthropic, OpenAI and AWS".
fn whose(kinds: &[&str]) -> String {
    let mut k: Vec<&str> = Vec::new();
    for x in kinds {
        if !k.contains(x) {
            k.push(x);
        }
    }
    match k.as_slice() {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

// ------------------------------------------------------------------ shell histories

/// A history file: what mom calls it when asked to blank it, and what it's called on screen.
#[derive(Debug, Clone, PartialEq)]
struct History {
    id: String,
    label: String,
    path: PathBuf,
}

/// PowerShell keeps its history (one file per host: the console, VS Code...) in PSReadLine's
/// folder; bash and zsh keep theirs in your home.
fn histories(paths: &Paths) -> Vec<History> {
    let mut out = Vec::new();
    let psreadline = psreadline_dir(paths);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&psreadline).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    files.sort();
    for f in files.into_iter().take(20) {
        let Some(name) = f.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
        let Some(host) = name.strip_suffix("_history.txt") else { continue };
        let label = match host {
            "ConsoleHost" => "PowerShell history".to_string(),
            "Visual Studio Code Host" => "VS Code's PowerShell history".to_string(),
            other => format!("{} PowerShell history", other.chars().filter(|c| !c.is_control()).take(40).collect::<String>()),
        };
        out.push(History { id: format!("psreadline:{name}"), label, path: f });
    }
    for (file, label) in [(".bash_history", "Bash history"), (".zsh_history", "Zsh history")] {
        let path = paths.home.join(file);
        if path.is_file() {
            out.push(History { id: format!("home:{file}"), label: label.into(), path });
        }
    }
    out
}

fn psreadline_dir(paths: &Paths) -> PathBuf {
    paths.appdata.join("Microsoft").join("Windows").join("PowerShell").join("PSReadLine")
}

/// A real file (not a link) no bigger than `max`.
fn read_small(path: &Path, max: u64) -> Option<String> {
    let m = std::fs::symlink_metadata(path).ok()?;
    if !m.is_file() || m.len() > max {
        return None;
    }
    String::from_utf8(std::fs::read(path).ok()?).ok()
}

/// Takes the keys out of one history file. Only a history mom lists herself (`id` from
/// `histories`), never a path from elsewhere. Rereads the file, writes the blanked copy beside
/// it and swaps it in, unless the file changed meanwhile (then tries again).
pub fn blank_history(paths: &Paths, id: &str) -> Result<String, String> {
    let Some(h) = histories(paths).into_iter().find(|h| h.id == id) else {
        return Err("Mom doesn't know that history file.".into());
    };
    for _ in 0..3 {
        let before = std::fs::symlink_metadata(&h.path).map_err(|_| "The file isn't there any more.".to_string())?;
        if !before.is_file() || before.len() > MAX_HISTORY {
            return Err("Mom won't change that file.".into());
        }
        let Some(text) = read_small(&h.path, MAX_HISTORY) else {
            return Err("It isn't plain text, so mom won't change it.".into());
        };
        let (blanked, n) = blank(&text);
        if n == 0 {
            return Ok("Nothing to take out any more.".into());
        }
        let tmp = fresh_file(&h.path, blanked.as_bytes()).map_err(|e| format!("Couldn't write beside it: {e}"))?;
        let now = std::fs::symlink_metadata(&h.path).ok();
        let same = now.is_some_and(|m| m.len() == before.len() && m.modified().ok() == before.modified().ok());
        if !same {
            let _ = std::fs::remove_file(&tmp);
            continue; // a shell wrote to it meanwhile: read it again
        }
        return match std::fs::rename(&tmp, &h.path) {
            Ok(()) => Ok(format!("Took {} out of your {}.", if n == 1 { "1 key".to_string() } else { format!("{n} keys") }, h.label)),
            Err(_) => {
                let _ = std::fs::remove_file(&tmp);
                Err("A window has it open. Close PowerShell and terminal windows, then press again.".into())
            }
        };
    }
    Err("It kept changing while mom worked. Close PowerShell and terminal windows, then press again.".into())
}

/// Writes `bytes` to a new file beside `path`, under a name nobody could have guessed, creating
/// it fresh: never through a link someone left there, and never in a folder that is itself a
/// link to somewhere else. Returns where it went.
fn fresh_file(path: &Path, bytes: &[u8]) -> std::io::Result<PathBuf> {
    use std::io::Write;
    let refuse = |what: &str| std::io::Error::new(std::io::ErrorKind::PermissionDenied, what.to_string());
    let dir = path.parent().ok_or_else(|| refuse("no folder"))?;
    let meta = std::fs::symlink_metadata(dir)?;
    let mut linked = meta.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        linked |= meta.file_attributes() & 0x400 != 0; // FILE_ATTRIBUTE_REPARSE_POINT
    }
    if linked || !meta.is_dir() {
        return Err(refuse("its folder is a link"));
    }
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}-{nanos:x}.momtmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT: a link is never followed
    }
    let mut file = options.open(&tmp)?;
    let written = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(tmp)
}

fn history_check(h: &History, text: &str) -> Option<Check> {
    let mut lines = 0;
    let mut kinds: Vec<&str> = Vec::new();
    for line in text.lines() {
        let k = keys_in(line);
        if !k.is_empty() {
            lines += 1;
            kinds.extend(k);
        }
    }
    if lines == 0 {
        return None;
    }
    let what = if lines == 1 { "1 line with a key".to_string() } else { format!("{lines} lines with keys") };
    Some(Check {
        id: format!("secrets:{}", h.id),
        area: "Security",
        level: "warn",
        title: format!("Your {} has {what} ({})", h.label, whose(&kinds)),
        detail: "Anything running as you can read that file. Blank them, and swap any key that still works for a new one.".into(),
        fix: Some(Fix { label: "Blank them".into(), kind: "secrets", value: h.id.clone(), hint: Some("Mom takes each key out and keeps no copy. Close PowerShell windows first.".into()) }),
        more: Vec::new(),
        items: Vec::new(),
    })
}

// ------------------------------------------------------------------ files git keeps

/// Names that say a file holds secrets: env files, keys, certificates' private halves.
const PATHSPECS: &[&str] = &[
    ":(glob,icase)**/.env",
    ":(glob,icase)**/.env.*",
    ":(glob,icase)**/*.env",
    ":(glob,icase)**/*.pem",
    ":(glob,icase)**/*.key",
    ":(glob,icase)**/*key*.json",
    ":(glob,icase)**/*secret*",
    ":(glob,icase)**/*credential*",
    ":(glob,icase)**/id_rsa*",
    ":(glob,icase)**/id_ed25519*",
];

/// One tracked file holding keys: which project, where in it, and whose keys.
#[derive(Debug, Clone, PartialEq)]
struct Tracked {
    repo: PathBuf,
    file: String,
    kinds: Vec<&'static str>,
}

/// `rel` (a path git lists) inside `repo`, when every folder on the way is a real folder in the
/// project: none is a link or junction that leads somewhere else.
fn in_project(repo: &Path, rel: &str) -> Option<PathBuf> {
    use std::path::Component;
    let rel = Path::new(rel);
    if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    let mut dir = repo.to_path_buf();
    for part in rel.parent()?.components() {
        dir.push(part);
        let m = std::fs::symlink_metadata(&dir).ok()?;
        if crate::repos::linked(&m) || !m.is_dir() {
            return None;
        }
    }
    Some(repo.join(rel))
}

fn tracked_in(git: &Path, repo: &Path, budget: &std::sync::atomic::AtomicU64) -> Vec<Tracked> {
    use std::sync::atomic::Ordering;
    // Only a project mom asks git about anyway (see repos::askable).
    if !crate::repos::askable(repo) {
        return vec![];
    }
    let mut args = vec!["ls-files", "-z", "--"];
    args.extend_from_slice(PATHSPECS);
    let Some(listed) = crate::repos::ask(git, repo, &args) else { return vec![] };
    let mut out = Vec::new();
    for rel in listed.split(|b| *b == 0).filter(|p| !p.is_empty()).take(MAX_FILES) {
        let rel = String::from_utf8_lossy(rel).into_owned();
        let Some(path) = in_project(repo, &rel) else { continue };
        let Ok(m) = std::fs::symlink_metadata(&path) else { continue };
        if !m.is_file() || crate::repos::linked(&m) || m.len() > MAX_FILE || budget.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(m.len())).is_err() {
            continue;
        }
        let Some(text) = read_small(&path, MAX_FILE) else { continue };
        let mut kinds = keys_in(&text);
        if test_data(&rel) {
            kinds.retain(|k| *k != "Private key");
        }
        if !kinds.is_empty() {
            out.push(Tracked { repo: repo.to_path_buf(), file: rel, kinds });
        }
    }
    out
}

/// In a folder of tests or fixtures: a private key there is a test's own, made for it and often
/// public (Godot keeps tests/data/crypto/in.key), not anyone's secret. Keys to services still count.
fn test_data(rel: &str) -> bool {
    rel.split(['/', '\\']).rev().skip(1).any(|part| {
        matches!(
            part.to_ascii_lowercase().as_str(),
            "test" | "tests" | "testdata" | "test_data" | "test-data" | "testing" | "fixtures" | "fixture" | "__tests__" | "__fixtures__" | "spec" | "specs"
        )
    })
}

/// A name or path, safe to put in a sentence or a prompt.
fn quoted(s: &str, max: usize) -> String {
    let one: String = s.chars().map(|c| if c.is_control() || c == '`' { ' ' } else { c }).collect();
    let one = one.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > max {
        format!("{}…", one.chars().take(max).collect::<String>())
    } else {
        one
    }
}

fn tracked_check(found: &[Tracked]) -> Option<Check> {
    if found.is_empty() {
        return None;
    }
    let project = |t: &Tracked| quoted(&t.repo.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), 60);
    let list: Vec<String> = found
        .iter()
        .map(|t| format!("- `{}` in the project at `{}` (looks like {} {})", quoted(&t.file, 200), quoted(&t.repo.to_string_lossy(), 300), whose(&t.kinds), if t.kinds.len() == 1 { "key" } else { "keys" }))
        .collect();
    let n = found.len();
    Some(Check {
        id: "secrets:tracked".into(),
        area: "Security",
        level: "warn",
        title: if n == 1 { "A file git keeps holds a key".to_string() } else { format!("{n} files git keeps hold keys") },
        detail: "If the project is pushed or shared, the keys go with it.".into(),
        fix: Some(Fix {
            label: "Copy for your agent".into(),
            kind: "copy",
            value: format!(
                "These files are tracked by git in my projects and look like they hold keys:\n{}\n\nFor each one: stop tracking it (git rm --cached, keeping the file), add it to .gitignore, and commit that. Then check whether the file is in commits that were already pushed, and if it is, tell me which keys to swap for new ones. Don't rewrite history or force-push without asking me first, and never print the keys themselves.\n",
                list.join("\n")
            ),
            hint: Some("Paste it to your agent.".into()),
        }),
        more: Vec::new(),
        items: found.iter().take(20).map(|t| Item { name: format!("{}: {}", project(t), quoted(&t.file, 80)), note: format!("{} {}", whose(&t.kinds), if t.kinds.len() == 1 { "key" } else { "keys" }), led: "red" }).collect(),
    })
}

/// The Security checks: keys in shell histories, and in files your projects' git keeps. One
/// line saying so when there are none.
pub fn checks(paths: &Paths) -> Vec<Check> {
    let mut out = Vec::new();
    let hs = histories(paths);
    for h in &hs {
        if let Some(text) = read_small(&h.path, MAX_HISTORY) {
            out.extend(history_check(h, &text));
        }
    }
    let repos = crate::repos::find(paths);
    let tracked: Vec<Tracked> = match crate::repos::git() {
        Some(git) if !repos.is_empty() => {
            let budget = std::sync::atomic::AtomicU64::new(MAX_TOTAL);
            crate::repos::each(&repos, Instant::now() + ALL_TIME, |r| Some(tracked_in(&git, r, &budget))).into_iter().flatten().flatten().collect()
        }
        _ => vec![],
    };
    out.extend(tracked_check(&tracked));
    if out.is_empty() && (!hs.is_empty() || !repos.is_empty()) {
        let places = match (hs.is_empty(), repos.is_empty()) {
            (false, false) => "your shell history or the files git keeps",
            (false, true) => "your shell history",
            _ => "the files git keeps",
        };
        out.push(Check {
            id: "secrets".into(),
            area: "Security",
            level: "ok",
            title: format!("No keys in {places}"),
            detail: "Mom looks for the shapes services give their keys.".into(),
            fix: None,
            more: Vec::new(),
            items: Vec::new(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up key in a given shape, put together here so no key-shaped text sits in the source.
    fn fake(prefix: &str, len: usize) -> String {
        let body: String = "Nimbus0Atlas1Copper2line3Lantern4fall5".chars().cycle().take(len).collect();
        format!("{prefix}{body}")
    }

    #[test]
    fn keys_are_known_by_their_shape_and_counted_once() {
        let ant = fake(&["sk", "ant", "api03", ""].join("-"), 40);
        let oai = fake(&["sk", "proj", ""].join("-"), 40);
        let gh = fake(&["gh", "p_"].concat(), 36);
        let aws = format!("{}{}", ["AK", "IA"].concat(), "NIMBUSATLAS12345");
        let line = format!("$env:A=\"{ant}\"; curl -H 'Authorization: {oai}' ; git clone https://{gh}@example.com ; {aws}");
        assert_eq!(keys_in(&line), ["Anthropic", "OpenAI", "GitHub", "AWS"]);
        // Words that only start alike aren't keys.
        assert!(keys_in("sk-learn is a library; ghp is a word; AKIA alone; hf_small").is_empty());
        let pem = ["-----BEGIN ", "OPENSSH PRIVATE KEY-----\nabc\n-----END ", "OPENSSH PRIVATE KEY-----"].concat();
        assert_eq!(keys_in(&pem), ["Private key"]);
        // A key's start with no end takes only the lines of key after it, not the rest of the file.
        let cut = format!("{}\n{}\nls -la\ncd storefront\n", ["-----BEGIN ", "RSA PRIVATE KEY-----"].concat(), "QUJDREVGR0hJSktMTU5PUFFSU1RVVldY");
        assert_eq!(blank(&cut).0, format!("{REMOVED}\nls -la\ncd storefront\n"));
        assert_eq!(whose(&["OpenAI", "GitHub", "OpenAI", "AWS"]), "OpenAI, GitHub and AWS");
    }

    #[test]
    fn blanking_takes_out_each_key_and_nothing_else() {
        let a = fake(&["gh", "o_"].concat(), 40);
        let b = fake(&["sk", "ant", ""].join("-"), 30);
        let text = format!("git push\r\n$env:TOKEN = '{a}'\r\nclaude --key {b} --verbose\r\n");
        let (out, n) = blank(&text);
        assert_eq!(n, 2);
        assert_eq!(out, format!("git push\r\n$env:TOKEN = '{REMOVED}'\r\nclaude --key {REMOVED} --verbose\r\n"));
        assert!(!out.contains(&a) && !out.contains(&b));
    }

    #[test]
    fn a_history_file_is_said_in_counts_and_blanked_in_place() {
        let root = std::env::temp_dir().join(format!("mom-secrets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::under(&root);
        let dir = psreadline_dir(&paths);
        std::fs::create_dir_all(&dir).unwrap();
        let key = fake(&["sk", "proj", ""].join("-"), 40);
        let history = dir.join("ConsoleHost_history.txt");
        std::fs::write(&history, format!("cd storefront\n$env:OPENAI_API_KEY='{key}'\nnpm run dev\n")).unwrap();
        std::fs::write(dir.join("notes.txt"), format!("{key}\n")).unwrap(); // not a history: never looked at
        let checks = checks(&paths);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].title, "Your PowerShell history has 1 line with a key (OpenAI)");
        assert!(checks.iter().all(|c| !c.title.contains(&key) && !c.detail.contains(&key)));
        let fix = checks[0].fix.as_ref().unwrap();
        assert_eq!((fix.kind, fix.value.as_str()), ("secrets", "psreadline:ConsoleHost_history.txt"));
        // Only a history mom listed herself, by its id.
        assert!(blank_history(&paths, "psreadline:notes.txt").is_err());
        assert!(blank_history(&paths, r"..\..\elsewhere.txt").is_err());
        assert_eq!(blank_history(&paths, &fix.value).unwrap(), "Took 1 key out of your PowerShell history.");
        assert_eq!(std::fs::read_to_string(&history).unwrap(), format!("cd storefront\n$env:OPENAI_API_KEY='{REMOVED}'\nnpm run dev\n"));
        assert!(std::fs::read_dir(&dir).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".momtmp")), "no copy left behind");
        assert_eq!(blank_history(&paths, &fix.value).unwrap(), "Nothing to take out any more.");
        assert_eq!(checks_ok(&paths), "No keys in your shell history");
        std::fs::remove_dir_all(&root).unwrap();
    }

    fn checks_ok(paths: &Paths) -> String {
        let c = checks(paths);
        assert_eq!((c.len(), c[0].level), (1, "ok"));
        c[0].title.clone()
    }

    #[test]
    fn a_private_key_made_for_a_test_is_not_a_secret() {
        assert!(test_data("tests/data/crypto/in.key") && test_data(r"src\__fixtures__\server.pem") && test_data("spec/keys/id_rsa"));
        assert!(!test_data("config/server.key") && !test_data("tests.key") && !test_data("backend/.env"));
    }

    #[test]
    fn only_files_in_real_folders_of_the_project_are_read() {
        let root = std::env::temp_dir().join(format!("mom-secrets-inside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("copperline");
        std::fs::create_dir_all(repo.join("config")).unwrap();
        assert_eq!(in_project(&repo, "config/.env"), Some(repo.join("config").join(".env")));
        assert_eq!(in_project(&repo, ".env"), Some(repo.join(".env")));
        assert_eq!(in_project(&repo, "../elsewhere/.env"), None);
        assert_eq!(in_project(&repo, "missing/.env"), None);
        #[cfg(windows)]
        {
            let target = root.join("elsewhere");
            std::fs::create_dir_all(&target).unwrap();
            let link = repo.join("linked");
            let made = std::process::Command::new(std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into()))
                .args(["/d", "/c", "mklink", "/J"])
                .arg(&link)
                .arg(&target)
                .output()
                .is_ok_and(|o| o.status.success());
            if made {
                assert_eq!(in_project(&repo, "linked/.env"), None, "a junction leads elsewhere");
                std::fs::remove_dir(&link).unwrap();
            }
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn files_git_keeps_are_looked_at_by_name_and_said_without_their_keys() {
        let Some(git) = crate::repos::git() else { return };
        let root = std::env::temp_dir().join(format!("mom-secrets-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("storefront");
        std::fs::create_dir_all(repo.join("config")).unwrap();
        let key = fake(&["sk", "ant", ""].join("-"), 30);
        std::fs::write(repo.join(".env"), format!("ANTHROPIC_API_KEY={key}\n")).unwrap();
        std::fs::write(repo.join("config").join("settings.json"), format!("{{\"key\":\"{key}\"}}\n")).unwrap(); // not named like a secret
        std::fs::write(repo.join(".env.example"), "ANTHROPIC_API_KEY=\n").unwrap();
        let run = |args: &[&str]| {
            let mut all = vec!["-C", repo.to_str().unwrap(), "-c", "user.name=Nimbus", "-c", "user.email=nimbus@example.com", "-c", "commit.gpgsign=false"];
            all.extend_from_slice(args);
            assert!(crate::util::run(git.to_str().unwrap(), &all, Duration::from_secs(20)).unwrap().ok());
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "first"]);
        let budget = std::sync::atomic::AtomicU64::new(MAX_TOTAL);
        let found = tracked_in(&git, &repo, &budget);
        assert_eq!(found.iter().map(|t| (t.file.as_str(), t.kinds.clone())).collect::<Vec<_>>(), [(".env", vec!["Anthropic"])]);
        let c = tracked_check(&found).unwrap();
        assert_eq!(c.title, "A file git keeps holds a key");
        let prompt = &c.fix.as_ref().unwrap().value;
        assert!(prompt.contains("`.env`") && prompt.contains("git rm --cached") && !prompt.contains(&key), "{prompt}");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
