//! Is there a newer momputer? One question to GitHub, at most once a day, and only while the
//! person leaves it switched on. Nothing about them or their PC goes with it: the request is the
//! same from every copy. It's the only thing momputer ever asks the internet.

use crate::paths::Paths;
use crate::util::{now_secs, run};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Where GitHub says what the newest release is.
const LATEST: &str = "https://api.github.com/repos/Thisisily/momputer/releases/latest";
/// Where the update is fetched from: the install page. Always this address, never one taken
/// from GitHub's reply.
pub const GET_IT: &str = "https://thisisily.github.io/momputer/";
/// A day between questions; an hour before trying again when there was no answer.
const EVERY: u64 = 24 * 3600;
const RETRY: u64 = 3600;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Update {
    /// The version that's running.
    pub current: String,
    /// The newest version published, if GitHub has ever answered.
    pub latest: Option<String>,
    /// `latest` is newer than `current`.
    pub newer: bool,
    /// Unix seconds: when GitHub last answered, and when it was last asked.
    pub checked: u64,
    pub asked: u64,
    /// Why there was no answer the last time, in plain words.
    pub problem: Option<String>,
    /// Where to get the newest version.
    pub get_it: String,
}

/// "v0.2.1" or "0.2.1" as numbers. Anything else (a name, a pre-release, something odd) is no version.
pub fn version(tag: &str) -> Option<[u32; 3]> {
    let t = tag.trim();
    let t = t.strip_prefix('v').unwrap_or(t);
    let mut parts = t.split('.');
    let mut out = [0u32; 3];
    for slot in &mut out {
        let p = parts.next()?;
        if p.is_empty() || p.len() > 5 || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *slot = p.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((version(latest), version(current)), (Some(l), Some(c)) if l > c)
}

/// The version GitHub's reply names, if it's a proper release (not a draft or a pre-release).
pub fn latest_in(reply: &str) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(reply).map_err(|_| "GitHub's answer made no sense.".to_string())?;
    if v.get("draft").and_then(serde_json::Value::as_bool) == Some(true) || v.get("prerelease").and_then(serde_json::Value::as_bool) == Some(true) {
        return Err("The newest release isn't finished yet.".into());
    }
    let tag = v.get("tag_name").and_then(serde_json::Value::as_str).unwrap_or_default();
    let n = version(tag).ok_or_else(|| match v.get("message").and_then(serde_json::Value::as_str) {
        Some(m) if m.to_ascii_lowercase().contains("rate limit") => "GitHub is busy. Mom will ask again later.".to_string(),
        _ => "GitHub didn't name a version.".to_string(),
    })?;
    Ok(format!("{}.{}.{}", n[0], n[1], n[2]))
}

fn file(paths: &Paths) -> std::path::PathBuf {
    paths.state_dir().join("update.json")
}

/// What mom last heard, measured against the version that's running now.
pub fn known(paths: &Paths, current: &str) -> Update {
    let mut u: Update = std::fs::read(file(paths)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    // A version is three numbers or it's nothing: the file is on disk, where anything can write.
    u.latest = u.latest.filter(|l| version(l).is_some());
    u.current = current.to_string();
    u.newer = u.latest.as_deref().is_some_and(|l| is_newer(l, current));
    u.get_it = GET_IT.to_string();
    u
}

/// Asks GitHub, if it's time to (or `now` says to), and remembers the answer.
pub fn check(paths: &Paths, current: &str, now: bool) -> Update {
    let mut u = known(paths, current);
    let t = now_secs();
    let due = t.saturating_sub(u.checked) >= EVERY && t.saturating_sub(u.asked) >= RETRY;
    if !now && !due {
        return u;
    }
    u.asked = t;
    match ask().and_then(|reply| latest_in(&reply)) {
        Ok(latest) => {
            u.newer = is_newer(&latest, current);
            u.latest = Some(latest);
            u.checked = t;
            u.problem = None;
        }
        Err(why) => u.problem = Some(why),
    }
    let _ = std::fs::create_dir_all(paths.state_dir());
    if let Ok(json) = serde_json::to_vec_pretty(&u) {
        let _ = std::fs::write(file(paths), json);
    }
    u
}

/// One request, with Windows' own curl: https only, ten seconds at most, a small answer at most.
fn ask() -> Result<String, String> {
    let windows = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).unwrap_or_else(|| r"C:\Windows".into());
    let curl = windows.join("System32").join("curl.exe");
    if !curl.is_file() {
        return Err("This Windows has no curl to ask with.".into());
    }
    let args = ["-sS", "-m", "10", "--proto", "=https", "--max-filesize", "400000", "-H", "User-Agent: momputer", "-H", "Accept: application/vnd.github+json", LATEST];
    match run(&curl.to_string_lossy(), &args, Duration::from_secs(14)) {
        Ok(o) if o.ok() => Ok(o.text()),
        _ => Err("Couldn't reach GitHub. Mom will ask again later.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert_eq!(version("v0.2.1"), Some([0, 2, 1]));
        assert_eq!(version("1.10.0"), Some([1, 10, 0]));
        for odd in ["", "v1.2", "1.2.3.4", "v1.2.x", "1.2.3-beta", "latest", "v1..3", "999999.1.1", "1.2.3; rm"] {
            assert_eq!(version(odd), None, "{odd}");
        }
        assert!(is_newer("0.10.0", "0.9.9") && is_newer("v1.0.0", "0.99.99"));
        assert!(!is_newer("0.2.0", "0.2.0") && !is_newer("0.1.9", "0.2.0") && !is_newer("nonsense", "0.2.0"));
    }

    #[test]
    fn only_a_finished_release_counts() {
        assert_eq!(latest_in(r#"{"tag_name":"v0.3.0","draft":false,"prerelease":false,"html_url":"https://evil.example/"}"#).as_deref(), Ok("0.3.0"));
        assert!(latest_in(r#"{"tag_name":"v0.3.0","prerelease":true}"#).is_err());
        assert!(latest_in(r#"{"tag_name":"v0.3.0","draft":true}"#).is_err());
        assert_eq!(latest_in(r#"{"message":"API rate limit exceeded for 203.0.113.9"}"#), Err("GitHub is busy. Mom will ask again later.".into()));
        assert!(latest_in("<html>").is_err() && latest_in(r#"{"tag_name":"nightly"}"#).is_err());
    }

    #[test]
    fn what_she_heard_is_measured_against_what_is_running() {
        let dir = std::env::temp_dir().join(format!("mom-update-{}", std::process::id()));
        let paths = Paths::under(&dir);
        assert_eq!(known(&paths, "0.2.0"), Update { current: "0.2.0".into(), get_it: GET_IT.into(), ..Update::default() });
        std::fs::create_dir_all(paths.state_dir()).unwrap();
        // The file names a newer version, and tries to send the button somewhere else.
        std::fs::write(file(&paths), r#"{"latest":"0.3.0","checked":5,"getIt":"https://evil.example/","current":"9.9.9","newer":false}"#).unwrap();
        let u = known(&paths, "0.2.0");
        assert_eq!((u.latest.as_deref(), u.newer, u.get_it.as_str(), u.current.as_str()), (Some("0.3.0"), true, GET_IT, "0.2.0"));
        assert!(!known(&paths, "0.3.0").newer, "once you have it, it isn't news");
        std::fs::write(file(&paths), r#"{"latest":"<script>","checked":5}"#).unwrap();
        assert_eq!(known(&paths, "0.2.0").latest, None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
