//! Memory snapshots Windows saves in LiveKernelReports when something stops responding (the
//! graphics driver, say). They can be tens of gigabytes, and only an administrator can look in
//! that folder: as you, mom can't list it or see how big a file in it is.
//!
//! Windows does note in the registry when it last saved a full snapshot of each sort, and it
//! names the file for the minute it started writing it ("WATCHDOG-20261018-1316.dmp"). Asking
//! for a file there by name says "access denied" when it's there and "not found" when it isn't.
//! That's enough to list the big ones by name. Their size stays unknown until an administrator
//! cleans them.

use std::path::{Path, PathBuf};

const KEY: &str = r"SYSTEM\CurrentControlSet\Control\CrashControl\FullLiveKernelReports";

/// A big snapshot takes minutes to write, and Windows notes it when it's done: look this many
/// minutes back for the minute it started.
const WRITING_MINUTES: i64 = 120;

/// Full snapshots Windows noted that are still in `reports`: each one's path, and when it was
/// saved (seconds since 1970).
pub(super) fn noted(reports: &Path) -> Vec<(PathBuf, u64)> {
    let mut out: Vec<(PathBuf, u64)> = Vec::new();
    for (sort, at, named) in notes() {
        let guessed = (0..=WRITING_MINUTES).filter_map(|m| file_name(&sort, at.saturating_sub(m as u64 * 60)));
        for name in named.into_iter().chain(guessed) {
            let path = reports.join(&name);
            if out.iter().all(|(p, _)| p != &path) && super::still_there(&path) {
                out.push((path, saved_at(&name).unwrap_or(at)));
            }
        }
    }
    out
}

/// What the registry says about each sort of full snapshot: its name ("WATCHDOG"), when Windows
/// last saved one, and any file names it noted.
#[cfg(windows)]
fn notes() -> Vec<(String, u64, Vec<String>)> {
    use crate::health::reg::{qword_value, subkeys, Hive};
    subkeys(Hive::LocalMachine, KEY)
        .into_iter()
        .filter(|sort| plain(sort))
        .filter_map(|sort| {
            let key = format!(r"{KEY}\{sort}");
            let at = qword_value(Hive::LocalMachine, &key, "LastFullLiveReport").and_then(unix_secs)?;
            let named = subkeys(Hive::LocalMachine, &key).into_iter().filter(|n| dump_name(n)).collect();
            Some((sort, at, named))
        })
        .collect()
}

#[cfg(not(windows))]
fn notes() -> Vec<(String, u64, Vec<String>)> {
    vec![]
}

/// A sort's name goes into a file name, so only letters, digits and underscores.
fn plain(sort: &str) -> bool {
    !sort.is_empty() && sort.len() <= 64 && sort.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// "WATCHDOG-20261018-1316.dmp", and nothing else.
fn dump_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".dmp").or_else(|| name.strip_suffix(".DMP")) else { return false };
    let Some((sort, when)) = stem.split_once('-') else { return false };
    plain(sort) && when.len() == 13 && when.bytes().enumerate().all(|(i, b)| if i == 8 { b == b'-' } else { b.is_ascii_digit() })
}

/// A Windows file time (100 ns since 1601) as seconds since 1970.
fn unix_secs(filetime: u64) -> Option<u64> {
    (filetime / 10_000_000).checked_sub(11_644_473_600).filter(|s| *s > 0)
}

/// The file Windows would have named for a snapshot started at `at`, in this PC's time zone.
fn file_name(sort: &str, at: u64) -> Option<String> {
    use chrono::TimeZone;
    let when = chrono::Local.timestamp_opt(i64::try_from(at).ok()?, 0).single()?;
    Some(format!("{sort}-{}.dmp", when.format("%Y%m%d-%H%M")))
}

/// When a snapshot was started, from its name.
fn saved_at(name: &str) -> Option<u64> {
    use chrono::TimeZone;
    let stem = name.get(..name.len().checked_sub(4)?)?;
    let when = stem.rsplit_once('-').and_then(|(rest, hm)| rest.rsplit_once('-').map(|(_, ymd)| format!("{ymd}{hm}")))?;
    let local = chrono::NaiveDateTime::parse_from_str(&when, "%Y%m%d%H%M").ok()?;
    u64::try_from(chrono::Local.from_local_datetime(&local).earliest()?.timestamp()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_names_windows_gives_snapshots() {
        assert!(dump_name("WATCHDOG-20261018-1316.dmp"));
        assert!(dump_name("ResourceTimeout-20260911-1412.dmp"));
        assert!(!dump_name("WATCHDOG-20261018-1316.dmp.txt"));
        assert!(!dump_name(r"..\WATCHDOG-20261018-1316.dmp"));
        assert!(!dump_name("WATCHDOG-2026101-81316.dmp"));
        assert!(!dump_name("WATCH DOG-20261018-1316.dmp"));
        assert!(plain("GreLKD") && !plain("..") && !plain(r"a\b") && !plain(""));
    }

    #[test]
    fn a_name_and_its_time_go_both_ways() {
        let at = saved_at("WATCHDOG-20261018-1316.dmp").unwrap();
        assert_eq!(file_name("WATCHDOG", at).as_deref(), Some("WATCHDOG-20261018-1316.dmp"));
        assert_eq!(file_name("WATCHDOG", at + 59).as_deref(), Some("WATCHDOG-20261018-1316.dmp"));
        assert_eq!(unix_secs(0x01dd_5eea_31cb_0ec3).map(|s| s / 86_400), Some(20_744), "18 October 2026");
        assert_eq!(unix_secs(0), None);
    }

    #[test]
    fn noted_finds_nothing_in_an_empty_folder() {
        let dir = std::env::temp_dir().join(format!("mom-dumps-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(noted(&dir).is_empty(), "only files that are there");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
