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
//! while the app is closed. An app whose processes cannot be identified stays locked.

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
const APP_CACHE_DIRS: &[&str] = crate::browser_data::CHROMIUM_CACHE_DIRS;

/// Known Electron app process identities. A matching cache layout alone cannot establish
/// whether an unknown app is closed, so it is displayed but never selected for cleanup.
const APP_EXES: &[(&str, &[&str])] = &[
    ("Code", &["code.exe"]),
    ("Code - Insiders", &["code - insiders.exe"]),
    ("Cursor", &["cursor.exe"]),
    ("Slack", &["slack.exe"]),
    ("Claude", &["claude.exe"]),
    ("Codex", &["codex.exe", "chatgpt.exe"]),
    ("discord", &["discord.exe"]),
    ("Microsoft Teams", &["teams.exe", "ms-teams.exe"]),
    ("ai.opencode.desktop", &["opencode.exe", "opencode-desktop.exe"]),
];

const EPIC_EXES: &[&str] = &[
    "epicgameslauncher.exe", "epicwebhelper.exe", "eosoverlayrenderer-win64-shipping.exe", "eosoverlayrenderer-win32-shipping.exe",
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
    // Never traverse a cache through a link or an unverified parent, even just to measure it.
    let safe: Vec<bool> = profiles.iter().map(|p| {
        real_directory(&p.home) && p.targets.iter().all(|t| real_directory(t) && !crate::browser_data::protects_path(t))
    }).collect();
    let all: Vec<PathBuf> = profiles.iter().zip(&safe).filter(|(_, safe)| **safe).flat_map(|(p, _)| p.targets.iter().cloned()).collect();
    let mut measured = tally_many(&all, ENTRY_CAP, u64::MAX).into_iter();
    for (p, safe) in profiles.into_iter().zip(safe) {
        let mut t = Tally::default();
        if safe {
            for _ in &p.targets {
                t.add(&measured.next().unwrap_or_default());
            }
        } else {
            t.complete = false;
        }
        if t.files == 0 && t.links == 0 && t.complete {
            continue;
        }
        let mut c = Cand::new(p.home.clone(), format!("{}, {}", p.browser, p.label), &t, ctx.now);
        c.targets = p.targets;
        let c = if !safe {
            c.verdict(false, "Cache path is linked, protected, or could not be checked")
        } else if !t.complete || t.links > 0 {
            c.verdict(false, "Cache contents could not be fully checked or contain links")
        } else if p.exes.is_empty() {
            c.verdict(false, "App process identity is unknown; clear its cache in the app")
        } else if p.exes.iter().any(|e| ctx.running(e)) {
            if !open.contains(&p.browser) {
                open.push(p.browser.clone());
            }
            c.verdict(false, format!("{} is open", p.browser))
        } else if ctx.in_use(&p.home) || c.targets.iter().any(|t| ctx.in_use(t)) {
            c.verdict(false, super::IN_USE)
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

/// A size scan can take long enough for the app to start. Recheck immediately before each
/// browser cleanup item, rather than relying only on the process snapshot that began it.
pub(super) fn still_closed(home: &Path, ctx: &TrashCtx) -> bool {
    let Some(profile) = profiles(&ctx.paths.local_appdata, &ctx.paths.appdata).into_iter().find(|p| p.home == home) else { return false };
    if profile.exes.is_empty() || ctx.in_use(home) { return false; }
    #[cfg(not(test))]
    let running = crate::procs::running_names_now();
    // Tests use their isolated fake process table, never the person's real browsers.
    #[cfg(test)]
    let running = Some(ctx.running_exes.clone());
    running.is_some_and(|names| profile.exes.iter().all(|exe| !names.contains(exe)))
}

fn existing(dirs: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    dirs.into_iter().filter(|d| match std::fs::symlink_metadata(d) {
        Ok(m) => m.is_dir() || linked(&m),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }).collect()
}

fn linked(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 { return true; }
    }
    meta.file_type().is_symlink()
}

/// Checking the leaf alone misses a cache reached through a linked profile or app folder.
fn real_directory(path: &Path) -> bool {
    path.is_absolute() && path.ancestors().all(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir() && !linked(&m)))
}

fn real_entries(dir: &Path) -> Vec<PathBuf> {
    if real_directory(dir) { entries(dir) } else { Vec::new() }
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
        for dir in real_entries(&data) {
            let n = name_of(&dir);
            if !real_directory(&dir) || !is_profile_dir(&n) {
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
        for e in real_entries(&base.join("Opera Software")) {
            let n = name_of(&e);
            if real_directory(&e) && !editions.contains(&n) {
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
                if !real_directory(dir) { return; }
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
            for dir in real_entries(&ed) {
                let n = name_of(&dir);
                if real_directory(&dir) && is_profile_dir(&n) {
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
    for dir in real_entries(&ff) {
        if !real_directory(&dir) { continue; }
        let cache = dir.join("cache2");
        if existing([cache.clone()]).is_empty() {
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
    out.extend(epic(local));
    out
}

/// Electron apps: any folder in %APPDATA% (or a Store app's copy of it) with a browser cache in
/// it. One item per app, its built-in browser tabs' caches included.
fn apps(local: &Path, roaming: &Path) -> Vec<Profile> {
    let mut homes: Vec<PathBuf> = real_entries(roaming).into_iter().filter(|d| !NOT_APPS.contains(&name_of(d).as_str())).collect();
    for pkg in real_entries(&local.join("Packages")) {
        homes.extend(real_entries(&pkg.join("LocalCache").join("Roaming")));
    }
    let mut out = Vec::new();
    for home in homes {
        if !real_directory(&home) {
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
        for part in real_entries(&home.join("Partitions")) {
            if real_directory(&part) {
                targets.extend(existing(APP_CACHE_DIRS.iter().map(|c| part.join(c))));
            }
        }
        if targets.is_empty() {
            continue;
        }
        let exes = match APP_EXES.iter().find(|(n, _)| n.eq_ignore_ascii_case(&name)) {
            Some((_, e)) => e.iter().map(|x| x.to_string()).collect(),
            None => Vec::new(),
        };
        let label = if home.join("Partitions").is_dir() { "app and built-in browser" } else { "app" };
        out.push(Profile { browser: name, exes, label: label.into(), home, targets });
    }
    out
}

/// Epic's `webcache` and `BrowserCache` are complete embedded-browser profiles, despite
/// their names. Only their direct disposable cache children belong in this collector.
fn epic(local: &Path) -> Vec<Profile> {
    let mut out = Vec::new();
    let saved = local.join("EpicGamesLauncher").join("Saved");
    for home in real_entries(&saved) {
        let name = name_of(&home);
        let is_webcache = name == "webcache" || name.strip_prefix("webcache_").is_some_and(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()));
        if !is_webcache || !real_directory(&home) { continue; }
        let targets = existing(APP_CACHE_DIRS.iter().map(|c| home.join(c)));
        if !targets.is_empty() {
            out.push(Profile { browser: "Epic Games launcher".into(), exes: EPIC_EXES.iter().map(|s| s.to_string()).collect(), label: name, home, targets });
        }
    }
    let home = local.join("Epic Games").join("EOSOverlay").join("BrowserCache");
    if real_directory(&home) {
        let targets = existing(APP_CACHE_DIRS.iter().map(|c| home.join(c)));
        if !targets.is_empty() {
            out.push(Profile { browser: "Epic Games overlay".into(), exes: EPIC_EXES.iter().map(|s| s.to_string()).collect(), label: "browser cache".into(), home, targets });
        }
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
    if !real_directory(user_data) || !std::fs::symlink_metadata(user_data.join("Local State")).is_ok_and(|m| m.is_file() && !linked(&m) && m.len() <= 8 * 1024 * 1024) {
        return vec![];
    }
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
    use super::{Path, PathBuf, EPIC_EXES, profiles};
    use crate::config::TrashConfig;
    use crate::paths::Paths;
    use crate::trash::tests::{ctx, fixture, link, unlink, write};
    use crate::trash::{clean, measure, Pick};

    fn session_files(home: &Path) -> Vec<PathBuf> {
        ["Network/made-up-store", "Local Storage/leveldb/site", "Site Data/made-up-record", "IndexedDB/site", "Service Worker/CacheStorage/site", "Sessions/made-up-session", "made-up-sign-in.db", "Preferences"]
            .iter().map(|name| {
                let path = home.join(name);
                write(&path, 100, 400);
                path
            }).collect()
    }

    #[test]
    fn caches_of_closed_browsers_go_open_ones_stay() {
        let root = fixture("browsers");
        let paths = Paths::under(&root);
        let edge = paths.local_appdata.join("Microsoft").join("Edge").join("User Data");
        let chrome = paths.local_appdata.join("Google").join("Chrome").join("User Data");
        write(&edge.join("Default").join("Cache").join("Cache_Data").join("f_0001"), 4000, 1);
        write(&edge.join("Default").join("made-up-sign-in.db"), 100, 1); // never touched
        // A profile may have caches before any cookies. User Data's Local State must not
        // prevent cleaning these disposable children or itself become a cleanup target.
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
        write(&cursor.join("Partitions").join("cursor-browser").join("made-up-sign-in.db"), 10, 1);
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
        assert!(cursor.join("Partitions").join("cursor-browser").join("made-up-sign-in.db").exists());
        assert!(cursor.join("Local Storage").join("leveldb").join("c").exists());
        assert!(paths.appdata.join("Slack").join("Code Cache").exists(), "Slack is open");
        assert!(!chrome.join("Profile 2").join("Code Cache").exists());
        assert_eq!(std::fs::read_to_string(chrome.join("Local State")).unwrap(), r#"{"profile":{"info_cache":{"Profile 2":{"name":"Work"}}}}"#);
        assert!(edge.join("Default").join("Cache").exists(), "Edge is open");
        assert!(edge.join("Default").join("made-up-sign-in.db").exists());
        assert!(ff.is_dir() && !ff.join("cache2").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn epic_profiles_offer_only_disposable_children_in_browser_caches() {
        let root = fixture("epic-browser-leaves");
        let paths = Paths::under(&root);
        let saved = paths.local_appdata.join("EpicGamesLauncher").join("Saved");
        let overlay = paths.local_appdata.join("Epic Games").join("EOSOverlay").join("BrowserCache");
        let homes = [saved.join("webcache"), saved.join("webcache_4430"), saved.join("webcache_9999"), overlay];
        let mut protected = Vec::new();
        for home in &homes {
            protected.extend(session_files(home));
            write(&home.join("Cache").join("data"), 200, 400);
            write(&home.join("Code Cache").join("js").join("data"), 300, 400);
            write(&home.join("GPUCache").join("data"), 100, 400);
        }
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let spot = measure("browser-cache", &ctx).unwrap();
        assert_eq!((spot.reclaimable_items, spot.reclaimable_bytes), (4, 2400));
        assert!(spot.items.iter().all(|i| i.name.starts_with("Epic Games launcher,") || i.name.starts_with("Epic Games overlay,")));
        let old_apps = measure("app-leftovers", &ctx).unwrap();
        assert!(!old_apps.items.iter().any(|i| i.path.contains("EpicGamesLauncher") || i.path.contains("EOSOverlay")));
        let report = clean("browser-cache", &ctx, None);
        assert_eq!((report.deleted, report.freed), (4, 2400));
        assert!(homes.iter().all(|home| home.is_dir()), "a webcache name is a profile, never a deletion target");
        assert!(protected.iter().all(|path| std::fs::metadata(path).is_ok_and(|m| m.len() == 100)), "all synthetic login/session markers survive");
        assert!(homes.iter().all(|home| !home.join("Cache").exists() && !home.join("Code Cache").exists() && !home.join("GPUCache").exists()));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn every_epic_process_and_in_use_path_blocks_a_stale_explicit_pick() {
        let root = fixture("epic-browser-open");
        let paths = Paths::under(&root);
        let home = paths.local_appdata.join("EpicGamesLauncher").join("Saved").join("webcache_4430");
        let cache = home.join("Cache").join("data");
        write(&cache, 200, 400);
        let cfg = TrashConfig::default();
        let mut ctx = ctx(&paths, &cfg);
        assert_eq!(measure("browser-cache", &ctx).unwrap().reclaimable_items, 1);
        let pick = Pick { goes: false, ticked: vec![home.to_string_lossy().into()], ..Default::default() };
        for exe in EPIC_EXES {
            ctx.running_exes.clear();
            ctx.running_exes.insert((*exe).into());
            let spot = measure("browser-cache", &ctx).unwrap();
            assert!(spot.items.iter().all(|i| !i.goes && !i.optional), "{exe}");
            assert_eq!(clean("browser-cache", &ctx, Some(&pick)).deleted, 0, "{exe}");
            assert!(cache.exists());
        }
        ctx.running_exes.clear();
        ctx.in_use = crate::util::norm_path(&cache.to_string_lossy());
        assert_eq!(clean("browser-cache", &ctx, Some(&pick)).deleted, 0);
        assert!(cache.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn unknown_electron_process_identity_cannot_be_overridden_by_tick_all() {
        let root = fixture("browser-unknown-app");
        let paths = Paths::under(&root);
        let home = paths.appdata.join("An Unknown App");
        let cache = home.join("Code Cache").join("js").join("data");
        write(&cache, 200, 400);
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let spot = measure("browser-cache", &ctx).unwrap();
        assert_eq!(spot.items.len(), 1);
        assert!(!spot.items[0].goes && !spot.items[0].optional);
        assert!(spot.items[0].reason.contains("process identity is unknown"));
        assert_eq!(clean("browser-cache", &ctx, Some(&Pick { new: true, ticked: vec![home.to_string_lossy().into()], ..Default::default() })).deleted, 0);
        assert!(cache.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cache_links_and_linked_profile_ancestors_never_reach_their_target() {
        let root = fixture("browser-linked-cache");
        let paths = Paths::under(&root);
        let home = paths.local_appdata.join("EpicGamesLauncher").join("Saved").join("webcache_4430");
        let elsewhere = root.join("elsewhere");
        write(&elsewhere.join("session"), 200, 400);
        std::fs::create_dir_all(&home).unwrap();
        let cache_link = home.join("Cache");
        link(&cache_link, &elsewhere);
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        assert_eq!(clean("browser-cache", &ctx, Some(&Pick { new: true, ..Default::default() })).deleted, 0);
        assert!(elsewhere.join("session").exists());
        unlink(&cache_link);
        write(&cache_link.join("data"), 100, 400);
        let nested_link = cache_link.join("linked");
        link(&nested_link, &elsewhere);
        let spot = measure("browser-cache", &ctx).unwrap();
        assert!(spot.items.iter().all(|i| !i.goes && !i.optional));
        assert_eq!(clean("browser-cache", &ctx, Some(&Pick { new: true, ..Default::default() })).deleted, 0);
        assert!(cache_link.join("data").exists() && elsewhere.join("session").exists());
        unlink(&nested_link);
        std::fs::remove_dir_all(&cache_link).unwrap();
        std::fs::remove_dir(&home).unwrap();
        link(&home, &elsewhere);
        assert!(profiles(&paths.local_appdata, &paths.appdata).is_empty());
        assert!(elsewhere.join("session").exists());
        unlink(&home);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_profile_mark_inside_a_cache_locks_the_entire_candidate() {
        let root = fixture("browser-cache-profile-marker");
        let paths = Paths::under(&root);
        let home = paths.local_appdata.join("EpicGamesLauncher").join("Saved").join("webcache");
        let protected = home.join("Cache").join("nested").join("Network").join("TransportSecurity");
        write(&protected, 100, 400);
        write(&home.join("Code Cache").join("data"), 200, 400);
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let spot = measure("browser-cache", &ctx).unwrap();
        assert!(spot.items.iter().all(|i| !i.goes && !i.optional));
        assert_eq!(clean("browser-cache", &ctx, Some(&Pick { new: true, ..Default::default() })).deleted, 0);
        assert!(protected.exists());
        assert!(home.join("Code Cache").join("data").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
