//! Browser caches: the saved copies of pages, images and scripts that make sites load faster.
//!
//! Only the cache folders are touched (Cache, Code Cache and GPUCache for Chromium browsers,
//! cache2 for Firefox). Cookies, logins, history, bookmarks and passwords live elsewhere in the
//! profile and are never read or deleted. A browser that's open keeps its cache: it holds the
//! files open, and pulling them out from under it can upset it.
//!
//! Apps built on Electron (Cursor, VS Code, the Claude and Codex apps, Slack, Discord...) are
//! browsers inside, with the same caches, plus one set per built-in browser tab (Partitions).
//! The same rules apply: cache folders only, never Cookies, Local Storage or IndexedDB, and only
//! while the app is closed.

use super::{entries, name_of, tally_many, Cand, Found, Tally, TrashCtx, ENTRY_CAP};
use std::path::{Path, PathBuf};

/// Chromium browsers: name, process, where the profiles live under %LOCALAPPDATA%.
const CHROMIUM: &[(&str, &str, &[&str])] = &[
    ("Chrome", "chrome.exe", &["Google", "Chrome", "User Data"]),
    ("Edge", "msedge.exe", &["Microsoft", "Edge", "User Data"]),
    ("Brave", "brave.exe", &["BraveSoftware", "Brave-Browser", "User Data"]),
    ("Vivaldi", "vivaldi.exe", &["Vivaldi", "User Data"]),
];

const CACHE_DIRS: &[&str] = &["Cache", "Code Cache", "GPUCache"];

/// The caches of an Electron app: the browser ones, and the graphics ones it rebuilds on start.
const APP_CACHE_DIRS: &[&str] =
    &["Cache", "Code Cache", "GPUCache", "DawnCache", "DawnWebGPUCache", "DawnGraphiteCache", "GrShaderCache", "ShaderCache"];

/// Electron apps whose process isn't called after their folder: (folder, processes).
const APP_EXES: &[(&str, &[&str])] = &[
    ("Code", &["code.exe"]),
    ("Code - Insiders", &["code - insiders.exe"]),
    ("Claude", &["claude.exe"]),
    ("Codex", &["codex.exe", "chatgpt.exe"]),
    ("discord", &["discord.exe"]),
    ("Microsoft Teams", &["teams.exe", "ms-teams.exe"]),
    ("ai.opencode.desktop", &["opencode.exe", "opencode-desktop.exe"]),
];

/// Folders in %APPDATA% that aren't apps' own (or are handled elsewhere).
const NOT_APPS: &[&str] = &["Opera Software", "Microsoft", "Mozilla", "npm", "npm-cache"];

/// Wallet and password apps: their caches hold no keys, but mom leaves them be all the same.
const HANDS_OFF: &[&str] = &[
    "wallet", "coin", "ledger", "trezor", "keepkey", "atomic", "exodus", "electrum", "metamask", "binance", "kraken",
    "bitcoin", "crypto", "guarda", "bitwarden", "keepass", "1password", "lastpass", "dashlane", "authy",
];

pub(super) fn hands_off(name: &str) -> bool {
    let n: String = name.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
    HANDS_OFF.iter().any(|w| n.contains(w))
}

/// One browser profile's cache, spread over one or more folders.
struct Profile {
    browser: String,
    /// Its processes: while any runs, the cache stays.
    exes: Vec<String>,
    label: String,
    home: PathBuf,
    targets: Vec<PathBuf>,
}

pub(super) fn find(ctx: &TrashCtx) -> Found {
    let profiles = profiles(&ctx.paths.local_appdata, &ctx.paths.appdata);
    if profiles.is_empty() {
        return Found::none();
    }
    let mut f = Found::none(); // spread over several browsers' folders, so no single root
    f.available = true;
    let mut open: Vec<String> = Vec::new();
    let all: Vec<PathBuf> = profiles.iter().flat_map(|p| p.targets.iter().cloned()).collect();
    let mut measured = tally_many(&all, ENTRY_CAP, u64::MAX).into_iter();
    for p in profiles {
        let mut t = Tally::default();
        for _ in &p.targets {
            t.add(&measured.next().unwrap_or_default());
        }
        if t.files == 0 {
            continue;
        }
        let mut c = Cand::new(p.home.clone(), format!("{}, {}", p.browser, p.label), &t, ctx.now);
        c.targets = p.targets;
        let c = if p.exes.iter().any(|e| ctx.running(e)) {
            if !open.contains(&p.browser) {
                open.push(p.browser.clone());
            }
            c.verdict(false, format!("{} is open", p.browser))
        } else {
            c.verdict(true, "Rebuilt as you browse")
        };
        f.exact &= c.complete;
        f.cands.push(c);
    }
    f.note = match open.as_slice() {
        [] => None,
        [one] => Some(format!("Close {one} to clear its cache.")),
        many => {
            let names: Vec<&str> = many.iter().map(String::as_str).collect();
            Some(format!("Close {} to clear their caches.", super::and_list(&names)))
        }
    };
    f
}

/// Browsers that kept their cache because they're open, from the items that stayed.
pub(super) fn open_names(stays: &[Cand]) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for c in stays {
        if let Some(b) = c.reason.strip_suffix(" is open") {
            if !out.contains(&b) {
                out.push(b);
            }
        }
    }
    out
}

fn existing(dirs: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    dirs.into_iter().filter(|d| d.is_dir()).collect()
}

fn is_profile_dir(name: &str) -> bool {
    name == "Default" || name == "Guest Profile" || name.starts_with("Profile ")
}

/// Every browser profile with a cache, Chromium browsers first, then Opera, then Firefox.
fn profiles(local: &Path, roaming: &Path) -> Vec<Profile> {
    let mut out = Vec::new();
    for (browser, exe, rel) in CHROMIUM {
        let data = rel.iter().fold(local.to_path_buf(), |p, r| p.join(r));
        let names = profile_names(&data);
        for dir in entries(&data) {
            let n = name_of(&dir);
            if !dir.is_dir() || !is_profile_dir(&n) {
                continue;
            }
            let targets = existing(CACHE_DIRS.iter().map(|c| dir.join(c)));
            if targets.is_empty() {
                continue;
            }
            let label = match names.iter().find(|(k, _)| *k == n) {
                Some((_, shown)) if !shown.is_empty() && *shown != n => format!("{shown} ({n})"),
                _ => default_label(&n),
            };
            out.push(Profile { browser: browser.to_string(), exes: vec![exe.to_string()], label, home: dir, targets });
        }
    }
    // Opera keeps its cache in Local and the rest of the profile in Roaming, and older versions
    // kept the default profile straight in the edition's folder (that counts as Default).
    let mut editions: Vec<String> = Vec::new();
    for base in [local, roaming] {
        for e in entries(&base.join("Opera Software")) {
            let n = name_of(&e);
            if e.is_dir() && !editions.contains(&n) {
                editions.push(n);
            }
        }
    }
    for edition in editions {
        let browser = match edition.as_str() {
            "Opera Stable" => "Opera".to_string(),
            "Opera GX Stable" => "Opera GX".to_string(),
            other => other.to_string(),
        };
        let mut by_profile: Vec<(String, Vec<PathBuf>)> = Vec::new();
        for base in [local, roaming] {
            let ed = base.join("Opera Software").join(&edition);
            let mut add = |profile: String, dir: &Path| {
                let targets = existing(CACHE_DIRS.iter().map(|c| dir.join(c)));
                if targets.is_empty() {
                    return;
                }
                match by_profile.iter_mut().find(|(p, _)| *p == profile) {
                    Some((_, t)) => t.extend(targets),
                    None => by_profile.push((profile, targets)),
                }
            };
            add("Default".into(), &ed);
            for dir in entries(&ed) {
                let n = name_of(&dir);
                if dir.is_dir() && is_profile_dir(&n) {
                    add(n, &dir);
                }
            }
        }
        for (profile, targets) in by_profile {
            let home = targets[0].parent().map(Path::to_path_buf).unwrap_or_default();
            out.push(Profile { browser: browser.clone(), exes: vec!["opera.exe".into()], label: default_label(&profile), home, targets });
        }
    }
    let ff = local.join("Mozilla").join("Firefox").join("Profiles");
    for dir in entries(&ff) {
        let cache = dir.join("cache2");
        if !cache.is_dir() {
            continue;
        }
        let n = name_of(&dir);
        let shown = n.split_once('.').map(|(_, p)| p).unwrap_or(&n);
        out.push(Profile {
            browser: "Firefox".into(),
            exes: vec!["firefox.exe".into()],
            label: format!("{shown} profile"),
            home: dir.clone(),
            targets: vec![cache],
        });
    }
    out.extend(apps(local, roaming));
    out
}

/// Electron apps: any folder in %APPDATA% (or a Store app's copy of it) with a browser cache in
/// it. One item per app, its built-in browser tabs' caches included.
fn apps(local: &Path, roaming: &Path) -> Vec<Profile> {
    let mut homes: Vec<PathBuf> = entries(roaming).into_iter().filter(|d| !NOT_APPS.contains(&name_of(d).as_str())).collect();
    for pkg in entries(&local.join("Packages")) {
        homes.extend(entries(&pkg.join("LocalCache").join("Roaming")));
    }
    let mut out = Vec::new();
    for home in homes {
        if !std::fs::symlink_metadata(&home).map(|m| m.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = name_of(&home);
        if hands_off(&name) {
            continue;
        }
        // Only Electron apps have these two side by side.
        if !home.join("Code Cache").is_dir() && !home.join("Partitions").is_dir() {
            continue;
        }
        let mut targets = existing(APP_CACHE_DIRS.iter().map(|c| home.join(c)));
        for part in entries(&home.join("Partitions")) {
            targets.extend(existing(APP_CACHE_DIRS.iter().map(|c| part.join(c))));
        }
        if targets.is_empty() {
            continue;
        }
        let exes = match APP_EXES.iter().find(|(n, _)| n.eq_ignore_ascii_case(&name)) {
            Some((_, e)) => e.iter().map(|x| x.to_string()).collect(),
            None => vec![format!("{}.exe", name.to_ascii_lowercase())],
        };
        let label = if home.join("Partitions").is_dir() { "app and built-in browser" } else { "app" };
        out.push(Profile { browser: name, exes, label: label.into(), home, targets });
    }
    out
}

fn default_label(dir_name: &str) -> String {
    match dir_name {
        "Default" => "Default profile".into(),
        "Guest Profile" => "Guest profile".into(),
        other => other.to_string(),
    }
}

/// Profile folder -> the name people see in the browser ("Work", "Ilya"), from Local State.
fn profile_names(user_data: &Path) -> Vec<(String, String)> {
    let Ok(text) = std::fs::read_to_string(user_data.join("Local State")) else { return vec![] };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return vec![] };
    v.get("profile")
        .and_then(|p| p.get("info_cache"))
        .and_then(|c| c.as_object())
        .map(|o| {
            o.iter()
                .map(|(k, info)| (k.clone(), info.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use crate::config::TrashConfig;
    use crate::paths::Paths;
    use crate::trash::tests::{ctx, fixture, write};
    use crate::trash::{clean, measure};

    #[test]
    fn caches_of_closed_browsers_go_open_ones_stay() {
        let root = fixture("browsers");
        let paths = Paths::under(&root);
        let edge = paths.local_appdata.join("Microsoft").join("Edge").join("User Data");
        let chrome = paths.local_appdata.join("Google").join("Chrome").join("User Data");
        write(&edge.join("Default").join("Cache").join("Cache_Data").join("f_0001"), 4000, 1);
        write(&edge.join("Default").join("Cookies"), 100, 1); // never touched
        write(&chrome.join("Profile 2").join("Code Cache").join("js").join("a"), 2000, 1);
        write(&chrome.join("Profile 2").join("GPUCache").join("data_1"), 500, 1);
        write(&chrome.join("System Profile").join("Cache").join("x"), 999, 1); // not a person's profile
        std::fs::write(chrome.join("Local State"), r#"{"profile":{"info_cache":{"Profile 2":{"name":"Work"}}}}"#).unwrap();
        let ff = paths.local_appdata.join("Mozilla").join("Firefox").join("Profiles").join("ab12cd.default-release");
        write(&ff.join("cache2").join("entries").join("E1"), 300, 1);
        write(&paths.appdata.join("Opera Software").join("Opera Stable").join("Default").join("GPUCache").join("g"), 70, 1);
        write(&paths.local_appdata.join("Opera Software").join("Opera Stable").join("Default").join("Cache").join("c"), 30, 1);

        // Cursor: its caches and its built-in browser's, never cookies or storage.
        let cursor = paths.appdata.join("Cursor");
        write(&cursor.join("Code Cache").join("js").join("a"), 800, 1);
        write(&cursor.join("Partitions").join("cursor-browser").join("Cache").join("Cache_Data").join("b"), 1200, 1);
        write(&cursor.join("Partitions").join("cursor-browser").join("Cookies"), 10, 1);
        write(&cursor.join("Local Storage").join("leveldb").join("c"), 10, 1);
        // A wallet app's cache is left alone.
        write(&paths.appdata.join("Ledger Live").join("Code Cache").join("e"), 600, 1);
        // Slack is open, so its cache stays.
        write(&paths.appdata.join("Slack").join("Code Cache").join("d"), 600, 1);

        let cfg = TrashConfig::default();
        let mut ctx = ctx(&paths, &cfg);
        ctx.running_exes.insert("slack.exe".into());
        ctx.running_exes.insert("msedge.exe".into());
        ctx.running_exes.insert("msedgewebview2.exe".into()); // apps' web views aren't Edge
        let spot = measure("browser-cache", &ctx).unwrap();
        let mut names: Vec<(&str, bool)> = spot.items.iter().map(|i| (i.name.as_str(), i.goes)).collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                ("Chrome, Work (Profile 2)", true),
                ("Cursor, app and built-in browser", true),
                ("Edge, Default profile", false),
                ("Firefox, default-release profile", true),
                ("Opera, Default profile", true),
                ("Slack, app", false),
            ]
        );
        assert_eq!(spot.items.iter().find(|i| i.name.starts_with("Edge")).unwrap().reason, "Edge is open");
        assert_eq!(spot.note.as_deref(), Some("Close Edge and Slack to clear their caches."));
        assert_eq!((spot.reclaimable_bytes, spot.kept_bytes), (4900, 4600));

        let r = clean("browser-cache", &ctx, None);
        assert_eq!((r.deleted, r.freed), (4, 4900));
        assert!(!cursor.join("Code Cache").exists() && !cursor.join("Partitions").join("cursor-browser").join("Cache").exists());
        assert!(cursor.join("Partitions").join("cursor-browser").join("Cookies").exists());
        assert!(cursor.join("Local Storage").join("leveldb").join("c").exists());
        assert!(paths.appdata.join("Slack").join("Code Cache").exists(), "Slack is open");
        assert!(!chrome.join("Profile 2").join("Code Cache").exists());
        assert!(chrome.join("Local State").exists());
        assert!(edge.join("Default").join("Cache").exists(), "Edge is open");
        assert!(edge.join("Default").join("Cookies").exists());
        assert!(ff.is_dir() && !ff.join("cache2").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
