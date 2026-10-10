//! Browser caches: the saved copies of pages, images and scripts that make sites load faster.
//!
//! Only the cache folders are touched (Cache, Code Cache and GPUCache for Chromium browsers,
//! cache2 for Firefox). Cookies, logins, history, bookmarks and passwords live elsewhere in the
//! profile and are never read or deleted. A browser that's open keeps its cache: it holds the
//! files open, and pulling them out from under it can upset it.

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

/// One browser profile's cache, spread over one or more folders.
struct Profile {
    browser: String,
    exe: &'static str,
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
        let c = if ctx.running(p.exe) {
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
            out.push(Profile { browser: browser.to_string(), exe, label, home: dir, targets });
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
            out.push(Profile { browser: browser.clone(), exe: "opera.exe", label: default_label(&profile), home, targets });
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
            exe: "firefox.exe",
            label: format!("{shown} profile"),
            home: dir.clone(),
            targets: vec![cache],
        });
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

        let cfg = TrashConfig::default();
        let mut ctx = ctx(&paths, &cfg);
        ctx.running_exes.insert("msedge.exe".into());
        ctx.running_exes.insert("msedgewebview2.exe".into()); // apps' web views aren't Edge
        let spot = measure("browser-cache", &ctx).unwrap();
        let names: Vec<(&str, bool)> = spot.items.iter().map(|i| (i.name.as_str(), i.goes)).collect();
        assert_eq!(
            names,
            vec![("Chrome, Work (Profile 2)", true), ("Firefox, default-release profile", true), ("Opera, Default profile", true), ("Edge, Default profile", false)]
        );
        assert_eq!(spot.items[3].reason, "Edge is open");
        assert_eq!(spot.note.as_deref(), Some("Close Edge to clear its cache."));
        assert!(spot.effect.contains("Edge is open, so its cache stays"), "{}", spot.effect);
        assert_eq!((spot.reclaimable_bytes, spot.kept_bytes), (2900, 4000));

        let r = clean("browser-cache", &ctx, None);
        assert_eq!((r.deleted, r.freed), (3, 2900));
        assert!(!chrome.join("Profile 2").join("Code Cache").exists());
        assert!(chrome.join("Local State").exists());
        assert!(edge.join("Default").join("Cache").exists(), "Edge is open");
        assert!(edge.join("Default").join("Cookies").exists());
        assert!(ff.is_dir() && !ff.join("cache2").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
