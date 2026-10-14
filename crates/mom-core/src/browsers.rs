//! Browsers: every Chromium browser on the PC (Chrome, Edge, Brave, Vivaldi), each profile in it,
//! and every extension in each profile, said plainly: what it is, what it can do, and whether
//! it's used. Plus each profile's storage sizes, without querying cookie databases. Reads browser
//! configuration and extension metadata; cookie sizes come from filesystem metadata. Changes nothing.

use crate::procs::ProcTable;
use crate::util::{now_secs, plural};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Browser {
    /// "chrome", "edge", "brave", "vivaldi".
    pub id: String,
    pub name: String,
    pub open: bool,
    /// Seconds it's been open, its processes and their memory, when it's open.
    pub open_secs: u64,
    pub processes: u32,
    pub mem: u64,
    /// The page that restarts it with every tab back ("chrome://restart").
    pub restart: String,
    /// Where it keeps its extensions' pages ("chrome://extensions").
    pub extensions_page: String,
    pub profiles: Vec<Profile>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    /// The folder's name ("Default", "Profile 3").
    pub dir: String,
    pub path: String,
    pub name: String,
    /// Unix seconds it was last used, when the browser says.
    pub last_used: Option<u64>,
    /// Everything in its folder, and what's cache, history and cookies.
    pub size: u64,
    pub cache: u64,
    pub history: u64,
    pub cookies_bytes: u64,
    /// Legacy fields kept for API compatibility; always None. Inventory never reads cookie
    /// database contents to count cookies or sites, even when the browser is closed.
    pub cookies: Option<u64>,
    pub cookie_sites: Option<u64>,
    pub extensions: Vec<Extension>,
    /// What's worth knowing, said plainly.
    pub notes: Vec<Note>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    /// "warn", "tip" or "ok".
    pub level: &'static str,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Extension {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub on: bool,
    /// A Chrome app (not an extension): Chrome stopped running these on Windows, and its
    /// extensions page no longer lists them.
    pub app: bool,
    /// A theme: Chrome lists it under Appearance, not with the extensions.
    pub theme: bool,
    /// The theme the browser shows now: it isn't left over, whatever its age.
    pub in_use: bool,
    /// The site a Chrome app opens ("desmos.com"), when it's one that just opens a site.
    pub site: Option<String>,
    /// What sort of thing it is: "ad blocker", "crypto wallet", "VPN", ...
    pub kind: Option<&'static str>,
    /// What it's allowed to do, in plain words, the most far-reaching first.
    pub can: Vec<String>,
    /// It can read and change every site you visit.
    pub everywhere: bool,
    /// Unix seconds: when it was added, and the newest observed local storage modification.
    /// Missing storage evidence is not evidence that the extension is unused.
    pub added: Option<u64>,
    pub last_saved: Option<u64>,
    pub size: u64,
    /// Its own page in the browser (where it's switched off or removed): the Apps page for a
    /// Chrome app, Appearance for a theme.
    pub page: String,
    /// Its picture, as a data: URI.
    pub icon: Option<String>,
}

/// The Chromium browsers mom knows: (id, name, user data folder under %LOCALAPPDATA%, process
/// name, page scheme).
const BROWSERS: &[(&str, &str, &str, &str, &str)] = &[
    ("chrome", "Chrome", r"Google\Chrome\User Data", "chrome.exe", "chrome"),
    ("edge", "Edge", r"Microsoft\Edge\User Data", "msedge.exe", "edge"),
    ("brave", "Brave", r"BraveSoftware\Brave-Browser\User Data", "brave.exe", "brave"),
    ("vivaldi", "Vivaldi", r"Vivaldi\User Data", "vivaldi.exe", "vivaldi"),
];

static CACHE: Mutex<Option<(u64, Vec<Browser>)>> = Mutex::new(None);

/// Every browser, its profiles and extensions. Folder sizes take a moment, so it's kept for five
/// minutes unless `fresh`.
pub fn list(table: &ProcTable, fresh: bool) -> Vec<Browser> {
    let now = now_secs();
    if !fresh {
        if let Some((at, b)) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            if now.saturating_sub(*at) < 300 {
                return with_running(b.clone(), table, now);
            }
        }
    }
    let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else { return vec![] };
    let mut out = Vec::new();
    for (id, name, folder, _, scheme) in BROWSERS {
        let root = local.join(folder);
        if !root.join("Local State").is_file() {
            continue;
        }
        out.push(Browser {
            id: (*id).into(),
            name: (*name).into(),
            open: false,
            open_secs: 0,
            processes: 0,
            mem: 0,
            restart: format!("{scheme}://restart"),
            extensions_page: format!("{scheme}://extensions"),
            profiles: profiles(&root, scheme, now),
        });
    }
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some((now, out.clone()));
    with_running(out, table, now)
}

/// Fills in which browsers are open now.
fn with_running(mut list: Vec<Browser>, table: &ProcTable, now: u64) -> Vec<Browser> {
    for b in &mut list {
        let Some((_, _, _, exe, _)) = BROWSERS.iter().find(|x| x.0 == b.id) else { continue };
        let procs: Vec<_> = table.procs.values().filter(|p| p.name.eq_ignore_ascii_case(exe)).collect();
        b.open = !procs.is_empty();
        b.processes = procs.len() as u32;
        b.mem = procs.iter().map(|p| p.mem).sum();
        b.open_secs = procs.iter().map(|p| p.start).filter(|s| *s > 0).min().map(|s| now.saturating_sub(s)).unwrap_or(0);
    }
    list
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Chrome's time: microseconds since 1601. "13291816019293602" -> unix seconds.
fn chrome_time(v: &Value) -> Option<u64> {
    let n: u64 = v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_u64())?;
    (n / 1_000_000).checked_sub(11_644_473_600).filter(|t| *t > 0)
}

/// Chromium records profile activity as Unix seconds. Malformed or future values do not
/// establish activity and must not be silently cast to zero or a huge integer.
fn profile_activity(v: &Value, now: u64) -> Option<u64> {
    let seconds = v.as_f64()?;
    (seconds.is_finite() && seconds >= 1.0 && seconds <= now as f64).then_some(seconds as u64)
}

/// Site-managed storage (including Service Worker/CacheStorage) is not disposable browser cache.
fn cache_part(name: &str) -> bool {
    crate::browser_data::CHROMIUM_CACHE_DIRS.iter().any(|cache| cache.eq_ignore_ascii_case(name))
}

fn profiles(root: &Path, scheme: &str, now: u64) -> Vec<Profile> {
    let Some(state) = read_json(&root.join("Local State")) else { return vec![] };
    let Some(cache) = state.pointer("/profile/info_cache").and_then(Value::as_object) else { return vec![] };
    let dirs: Vec<(String, Value)> = cache.iter().filter(|(k, _)| root.join(k).is_dir()).map(|(k, v)| (k.clone(), v.clone())).collect();

    // One pass over every profile's folders and files, all at once on the walker pool.
    let mut parts: Vec<(usize, String, PathBuf)> = Vec::new();
    for (i, (dir, _)) in dirs.iter().enumerate() {
        for e in std::fs::read_dir(root.join(dir)).into_iter().flatten().flatten() {
            parts.push((i, e.file_name().to_string_lossy().to_lowercase(), e.path()));
        }
    }
    let part_sizes = crate::trash::sizes(&parts.iter().map(|p| p.2.clone()).collect::<Vec<_>>());
    let mut size = vec![0u64; dirs.len()];
    let mut cache_size = vec![0u64; dirs.len()];
    for ((i, name, _), (bytes, _)) in parts.iter().zip(&part_sizes) {
        size[*i] += bytes;
        if cache_part(name) {
            cache_size[*i] += bytes;
        }
    }

    // Each profile's extensions, then one pass over their folders and what they've saved.
    let mut exts: Vec<Vec<(Extension, Vec<PathBuf>)>> = dirs.iter().map(|(dir, _)| extensions(&root.join(dir), scheme)).collect();
    let mut places: Vec<PathBuf> = Vec::new();
    for list in &exts {
        for (_, ps) in list {
            places.extend(ps.iter().cloned());
        }
    }
    let place_sizes = crate::trash::sizes(&places);
    let mut k = 0;
    for list in &mut exts {
        for (e, ps) in list.iter_mut() {
            let found = &place_sizes[k..k + ps.len()];
            k += ps.len();
            e.size = found.first().map_or(0, |s| s.0);
            e.last_saved = ps.iter().zip(found).skip(1).filter(|(p, _)| p.is_dir()).map(|(_, s)| s.1).filter(|t| *t > 0).max();
        }
    }

    let mut out: Vec<Profile> = dirs
        .iter()
        .zip(exts)
        .enumerate()
        .map(|(i, ((dir, info), list))| {
            let path = root.join(dir);
            // The profile's name, never its account (that's an email address).
            let mut name = info.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).unwrap_or(dir).to_string();
            if info.get("is_using_default_name").and_then(Value::as_bool) == Some(true) || name == "Your Chrome" {
                name = format!("{name} ({dir})");
            }
            let last_used = info.get("active_time").and_then(|v| profile_activity(v, now));
            let file_len = |p: PathBuf| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            let cookies_path = path.join("Network").join("Cookies");
            let extensions: Vec<Extension> = list.into_iter().map(|(e, _)| e).collect();
            Profile {
                dir: dir.clone(),
                path: path.to_string_lossy().to_string(),
                notes: notes(&extensions, last_used, now),
                name,
                last_used,
                size: size[i],
                cache: cache_size[i],
                history: file_len(path.join("History")),
                cookies_bytes: file_len(cookies_path),
                cookies: None,
                cookie_sites: None,
                extensions,
            }
        })
        .collect();
    out.sort_by(|a, b| b.last_used.cmp(&a.last_used).then_with(|| a.name.cmp(&b.name)));
    out
}

// ------------------------------------------------------------------ extensions

/// A profile's extensions, each with the folders to measure: its own first, then where it saves
/// things (sizes and dates are filled in afterwards, all at once).
fn extensions(profile: &Path, scheme: &str) -> Vec<(Extension, Vec<PathBuf>)> {
    let Some(prefs) = read_json(&profile.join("Secure Preferences")).or_else(|| read_json(&profile.join("Preferences"))) else { return vec![] };
    let Some(settings) = prefs.pointer("/extensions/settings").and_then(Value::as_object) else { return vec![] };
    // The theme in use is named in the ordinary Preferences, whichever file holds the settings.
    let current_theme = read_json(&profile.join("Preferences"))
        .and_then(|p| p.pointer("/extensions/theme/id").and_then(Value::as_str).map(str::to_string));
    let mut out = Vec::new();
    for (id, e) in settings {
        // From the store (1) or unpacked (4); the browser's own components don't count.
        let location = e.get("location").and_then(Value::as_i64).unwrap_or(0);
        if !matches!(location, 1 | 4) {
            continue;
        }
        let dir = profile.join("Extensions").join(id);
        let version_dir = std::fs::read_dir(&dir).ok().and_then(|rd| rd.filter_map(Result::ok).map(|d| d.path()).filter(|p| p.is_dir()).max());
        let manifest = version_dir.as_ref().and_then(|v| read_json_lenient(&v.join("manifest.json"))).or_else(|| e.get("manifest").cloned()).unwrap_or(Value::Null);
        let text = |key: &str| manifest.get(key).and_then(Value::as_str).map(|s| localize(s, version_dir.as_deref(), &manifest)).unwrap_or_default();
        let name = text("name");
        if name.is_empty() {
            continue;
        }
        let disabled = e.get("disable_reasons").is_some_and(|r| r.as_array().is_some_and(|a| !a.is_empty()) || r.as_i64().is_some_and(|n| n != 0))
            || e.get("state").and_then(Value::as_i64) == Some(0);
        let (can, everywhere) = permissions(&manifest);
        let app = manifest.get("app").is_some();
        let theme = manifest.get("theme").is_some();
        let page = if app {
            format!("{scheme}://apps")
        } else if theme {
            format!("{scheme}://settings/appearance")
        } else {
            format!("{scheme}://extensions/?id={id}")
        };
        let measure = vec![
            dir.clone(),
            profile.join("Local Extension Settings").join(id),
            profile.join("Sync Extension Settings").join(id),
            profile.join("IndexedDB").join(format!("chrome-extension_{id}_0.indexeddb.leveldb")),
        ];
        out.push((Extension {
            id: id.clone(),
            kind: kind_of(&name, &can),
            description: text("description"),
            version: manifest.get("version").and_then(Value::as_str).unwrap_or_default().to_string(),
            on: !disabled,
            app,
            theme,
            in_use: theme && current_theme.as_deref() == Some(id.as_str()),
            site: manifest.pointer("/app/launch/web_url").and_then(Value::as_str).and_then(site_of),
            can,
            everywhere,
            added: e.get("first_install_time").or_else(|| e.get("install_time")).and_then(chrome_time),
            last_saved: None,
            size: 0,
            page,
            icon: version_dir.as_deref().and_then(|v| crate::icons::extension(v, &manifest)),
            name,
        }, measure));
    }
    out.sort_by(|(a, _), (b, _)| b.on.cmp(&a.on).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

/// "https://www.desmos.com/calculator" -> "desmos.com".
fn site_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#', ':']).next()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    (!host.is_empty() && host.len() <= 253 && host.bytes().all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))).then(|| host.to_string())
}

/// Manifests sometimes start with a byte-order mark.
fn read_json_lenient(path: &Path) -> Option<Value> {
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

/// "__MSG_appName__" looked up in the extension's own translations: English, then its default.
fn localize(s: &str, dir: Option<&Path>, manifest: &Value) -> String {
    let Some(key) = s.strip_prefix("__MSG_").and_then(|k| k.strip_suffix("__")) else { return s.trim().to_string() };
    let Some(dir) = dir else { return String::new() };
    let default = manifest.get("default_locale").and_then(Value::as_str).unwrap_or("en");
    for locale in ["en", "en_US", "en_GB", default] {
        if let Some(messages) = read_json_lenient(&dir.join("_locales").join(locale).join("messages.json")) {
            if let Some((_, v)) = messages.as_object().and_then(|o| o.iter().find(|(k, _)| k.eq_ignore_ascii_case(key))) {
                if let Some(m) = v.get("message").and_then(Value::as_str) {
                    return m.trim().to_string();
                }
            }
        }
    }
    String::new()
}

/// What a manifest's permissions allow, in plain words, the most far-reaching first, and
/// whether that includes every site.
fn permissions(manifest: &Value) -> (Vec<String>, bool) {
    let list = |key: &str| -> Vec<String> { manifest.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default() };
    let mut perms = list("permissions");
    perms.extend(list("host_permissions"));
    let scripts_everywhere = manifest.get("content_scripts").and_then(Value::as_array).is_some_and(|cs| {
        cs.iter().any(|c| c.get("matches").and_then(Value::as_array).is_some_and(|m| m.iter().any(|x| x.as_str().is_some_and(all_sites))))
    });
    let everywhere = scripts_everywhere || perms.iter().any(|p| all_sites(p));
    const PLAIN: &[(&str, &str)] = &[
        ("debugger", "Can take over pages to inspect and change them"),
        ("proxy", "Sends your browsing through its own servers"),
        ("history", "Reads your browsing history"),
        ("cookies", "Reads your cookies"),
        ("webRequest", "Watches what every page loads"),
        ("declarativeNetRequest", "Blocks or changes what pages load"),
        ("clipboardRead", "Reads your clipboard"),
        ("nativeMessaging", "Talks to a program on your PC"),
        ("management", "Manages your other extensions"),
        ("tabs", "Sees your open tabs and their addresses"),
        ("webNavigation", "Sees every page you go to"),
        ("bookmarks", "Reads your bookmarks"),
        ("downloads", "Manages your downloads"),
        ("geolocation", "Knows where you are"),
        ("privacy", "Changes your privacy settings"),
        ("topSites", "Sees the sites you visit most"),
    ];
    let mut can = Vec::new();
    if everywhere {
        can.push("Reads and changes every site you visit".to_string());
    }
    for (perm, words) in PLAIN {
        if perms.iter().any(|p| p == perm) {
            can.push((*words).to_string());
        }
    }
    (can, everywhere)
}

fn all_sites(p: &str) -> bool {
    matches!(p, "<all_urls>" | "*://*/*" | "http://*/*" | "https://*/*" | "*://*/" | "http://*/" | "https://*/")
}

/// What sort of extension it is, from its name (and what it may do).
fn kind_of(name: &str, can: &[String]) -> Option<&'static str> {
    let n = name.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if has(&["adblock", "ad block", "ublock", "adguard", "ghostery", "ad blocker"]) {
        Some("Ad blocker")
    } else if has(&["wallet", "metamask", "phantom", "keplr", "yoroi", "nami", "unisat", "polkadot", "petra", "rabby", "coinbase", "trust", "exodus", "ledger"]) {
        Some("Crypto wallet")
    } else if has(&["vpn", "proxy"]) || can.iter().any(|c| c.starts_with("Sends your browsing")) {
        Some("VPN")
    } else if has(&["chatgpt", "claude", "gemini", "copilot", "grammarly", "ai "]) {
        Some("AI assistant")
    } else if has(&["dark mode", "dark reader", "material dark", "night"]) {
        Some("Dark mode")
    } else if has(&["tab manager", "tabs", "toby", "workona", "session"]) {
        Some("Tab manager")
    } else if has(&["translate"]) {
        Some("Translator")
    } else if has(&["speed control", "youtube", "return youtube"]) {
        Some("YouTube")
    } else {
        None
    }
}

/// What's worth knowing about a profile's extensions, said plainly.
fn notes(ext: &[Extension], last_used: Option<u64>, now: u64) -> Vec<Note> {
    let mut out = Vec::new();
    let on: Vec<&Extension> = ext.iter().filter(|e| e.on).collect();
    let year = 365 * 86_400;
    if let Some(t) = last_used.filter(|t| now.saturating_sub(*t) > year) {
        out.push(Note { level: "tip", text: format!("The browser last recorded this profile as active {} days ago.", now.saturating_sub(t) / 86_400) });
    }
    let apps: Vec<&str> = ext.iter().filter(|e| e.app).map(|e| e.name.as_str()).collect();
    if !apps.is_empty() {
        let (they, is) = if apps.len() == 1 { ("it", "is") } else { ("they", "are") };
        out.push(Note { level: "tip", text: format!("{} {is} still installed: {}. Chrome stopped running Chrome apps, so {they} can go.", plural(apps.len() as u64, "old Chrome app"), names(&apps)) });
    }
    let unknown: Vec<&str> = on.iter().filter(|e| !e.app && !e.theme && e.last_saved.is_none()).map(|e| e.name.as_str()).collect();
    if unknown.len() >= 2 {
        out.push(Note { level: "tip", text: format!("No local storage-write date is available for {}: {}. This does not show whether you use them.", plural(unknown.len() as u64, "extension"), names(&unknown)) });
    }
    let blockers: Vec<&str> = on.iter().filter(|e| e.kind == Some("Ad blocker")).map(|e| e.name.as_str()).collect();
    if blockers.len() >= 2 {
        out.push(Note { level: "tip", text: format!("{} ad blockers do the same job twice: {}. One is enough, and each slows every page.", blockers.len(), names(&blockers)) });
    }
    let vpn: Vec<&str> = on.iter().filter(|e| e.kind == Some("VPN")).map(|e| e.name.as_str()).collect();
    if !vpn.is_empty() {
        out.push(Note { level: "tip", text: format!("{} can send your browsing through {} servers. Fine if you meant to; it slows pages when it's on.", names(&vpn), if vpn.len() == 1 { "its own" } else { "their own" }) });
    }
    let everywhere = on.iter().filter(|e| e.everywhere).count();
    if everywhere >= 10 {
        out.push(Note { level: "tip", text: format!("{everywhere} extensions can read and change every site you visit, your bank's included. Keep the ones you trust and use.") });
    }
    if on.len() >= 25 {
        out.push(Note { level: "tip", text: format!("{} extensions are on. Each runs in the background or inside pages; fewer means a faster browser.", on.len()) });
    }
    out
}

fn names(list: &[&str]) -> String {
    let more = if list.len() > 4 { format!(" and {} more", list.len() - 4) } else { String::new() };
    format!("{}{more}", list.iter().take(4).copied().collect::<Vec<_>>().join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn inventory_reports_cookie_file_size_without_collecting_cookie_or_site_counts() {
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
        }
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/mom-test-fixtures");
        std::fs::create_dir_all(&base).unwrap();
        let base = std::fs::canonicalize(base).unwrap();
        let root = base.join(format!("browser-inventory-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        assert_eq!(root.parent(), Some(base.as_path()));
        std::fs::create_dir(&root).unwrap();
        let fixture = Fixture(root);
        let profile = fixture.0.join("Default");
        std::fs::create_dir_all(profile.join("Network")).unwrap();
        std::fs::write(fixture.0.join("Local State"), r#"{"profile":{"info_cache":{"Default":{"name":"Test profile"}}}}"#).unwrap();
        let cookies = profile.join("Network").join("Cookies");
        // A valid, closed database makes the former count query succeed. Inventory must still
        // return unknown counts; it must not inspect these synthetic cookie/site records.
        {
            let connection = rusqlite::Connection::open(&cookies).unwrap();
            connection.execute_batch("CREATE TABLE cookies (host_key TEXT); INSERT INTO cookies VALUES ('one.example'), ('one.example'), ('two.example');").unwrap();
        }
        let before = std::fs::read(&cookies).unwrap();
        let found = profiles(&fixture.0, "chrome", 1_800_000_000);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].cookies_bytes, before.len() as u64);
        assert_eq!(found[0].cookies, None);
        assert_eq!(found[0].cookie_sites, None);
        let serialized = serde_json::to_value(&found[0]).unwrap();
        assert_eq!(serialized["cookies"], Value::Null);
        assert_eq!(serialized["cookieSites"], Value::Null);
        assert_eq!(std::fs::read(&cookies).unwrap(), before);
        assert_eq!(std::fs::read_dir(profile.join("Network")).unwrap().count(), 1, "inventory must not create SQLite sidecars");
    }

    #[test]
    fn permissions_in_plain_words() {
        let m = json!({"permissions": ["storage", "tabs", "proxy", "history"], "host_permissions": ["<all_urls>"]});
        let (can, everywhere) = permissions(&m);
        assert!(everywhere);
        assert_eq!(can[0], "Reads and changes every site you visit");
        assert!(can.contains(&"Sends your browsing through its own servers".to_string()));
        assert!(can.contains(&"Reads your browsing history".to_string()));
        assert!(!can.iter().any(|c| c.contains("storage")), "harmless ones aren't listed");
        let scripts = json!({"content_scripts": [{"matches": ["https://*/*"]}]});
        assert!(permissions(&scripts).1);
        assert!(!permissions(&json!({"permissions": ["activeTab"]})).1);
    }

    #[test]
    fn kinds_and_times() {
        assert_eq!(kind_of("AdBlocker Ultimate", &[]), Some("Ad blocker"));
        assert_eq!(kind_of("Phantom", &[]), Some("Crypto wallet"));
        assert_eq!(kind_of("VPN for Chrome: NordVPN proxy protection", &[]), Some("VPN"));
        assert_eq!(kind_of("Desmos Graphing Calculator", &[]), None);
        assert_eq!(chrome_time(&json!("13291816019293602")), Some(1_647_342_419));
        assert_eq!(site_of("https://www.desmos.com/calculator?x=1").as_deref(), Some("desmos.com"));
        assert_eq!(site_of("http://Example.org:8080").as_deref(), Some("example.org"));
        assert_eq!(site_of("chrome-extension://abc/index.html"), None);
        assert_eq!(profile_activity(&json!(1_647_342_419.5), 1_800_000_000), Some(1_647_342_419));
        assert_eq!(profile_activity(&json!(-1), 1_800_000_000), None);
        assert_eq!(profile_activity(&json!(0), 1_800_000_000), None);
        assert_eq!(profile_activity(&json!(1_900_000_000), 1_800_000_000), None);
        assert_eq!(profile_activity(&json!("unknown"), 1_800_000_000), None);
    }

    #[test]
    fn site_storage_is_not_counted_as_disposable_cache() {
        assert!(cache_part("Cache") && cache_part("code cache") && cache_part("GPUCache"));
        for name in ["Service Worker", "CacheStorage", "Local Storage", "IndexedDB", "Sessions", "Site Data"] {
            assert!(!cache_part(name), "{name}");
        }
    }

    #[test]
    fn notes_say_what_matters() {
        let now = 1_800_000_000;
        let ext = |name: &str, kind: Option<&'static str>, app: bool, saved: Option<u64>| Extension {
            id: name.into(),
            name: name.into(),
            description: String::new(),
            version: "1".into(),
            on: true,
            app,
            theme: false,
            in_use: false,
            site: None,
            kind,
            can: vec![],
            everywhere: false,
            added: Some(now - 400 * 86_400),
            last_saved: saved,
            size: 0,
            page: String::new(),
            icon: None,
        };
        let list = vec![
            ext("AdBlock", Some("Ad blocker"), false, Some(now)),
            ext("AdBlocker Ultimate", Some("Ad blocker"), false, Some(now)),
            ext("Desmos", None, true, None),
            ext("Psykopaint", None, false, None),
            ext("Sketchpad", None, false, None),
        ];
        let n = notes(&list, Some(now - 500 * 86_400), now);
        let all: Vec<&str> = n.iter().map(|x| x.text.as_str()).collect();
        assert!(all[0].starts_with("The browser last recorded this profile as active 500 days ago"), "{all:?}");
        assert!(all.iter().any(|t| t.starts_with("1 old Chrome app is still installed: Desmos")), "{all:?}");
        assert!(all.iter().any(|t| t.starts_with("No local storage-write date is available for 2 extensions: Psykopaint, Sketchpad")), "{all:?}");
        assert!(!all.iter().any(|t| t.contains("unused") || t.contains("never saved")), "{all:?}");
        assert!(all.iter().any(|t| t.starts_with("2 ad blockers do the same job twice")), "{all:?}");
    }
}
