//! How fast each drive is filling up: one reading a day, kept for 120 days, so mom can say "At
//! this rate C: is full in about 23 days". Nothing runs for it: whatever already looks at the
//! drives hands its numbers over, and only the first look of a day touches the file.
//!
//! The rate is the median of the day-to-day slopes over the last 30 days (Theil-Sen), so a noisy
//! day doesn't swing it. A big clean-up starts the count again from that day: space someone freed
//! on purpose says nothing about how fast the drive fills.

use crate::paths::Paths;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

/// Readings older than this are dropped.
const KEEP_DAYS: u64 = 120;
/// The rate is worked out over this many days.
const WINDOW_DAYS: u64 = 30;
/// It takes at least this many days of readings to say anything.
const MIN_DAYS: u64 = 7;
/// Only said when the drive would be full within this many days.
const WARN_DAYS: u64 = 60;
/// Free space growing by more than this in a day (or 5% of the drive) was a clean-up.
const CLEAN_UP_BYTES: u64 = 10 << 30;
/// Shrinking slower than this per day isn't filling up.
const MIN_RATE: f64 = 100.0 * 1024.0 * 1024.0;
const MAX_FILE_BYTES: u64 = 2 << 20;
const MAX_DRIVES: usize = 26;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct Reading {
    /// Days since 1970.
    day: u64,
    /// "C:\\".
    drive: String,
    free: u64,
    total: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    readings: Vec<Reading>,
}

/// In about how many days each drive is full, by root ("C:\\").
type Answers = HashMap<String, Option<u64>>;

/// Today's answers (and whose state folder they're for), so later looks the same day cost nothing.
static TODAY: Mutex<Option<(u64, std::path::PathBuf, Answers)>> = Mutex::new(None);

fn file_of(paths: &Paths) -> std::path::PathBuf {
    paths.state_dir().join("space.json")
}

fn read(paths: &Paths) -> File {
    let path = file_of(paths);
    if std::fs::metadata(&path).map(|m| m.len() > MAX_FILE_BYTES).unwrap_or(true) {
        return File::default();
    }
    std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Notes today's free space of each drive (root, free, total) and says, for each, in about how
/// many days it's full at the rate it's filling (None: not filling, or not within two months, or
/// too few days to tell). Reads and writes the file at most once a day.
pub fn note(paths: &Paths, drives: &[(String, u64, u64)], now: u64) -> HashMap<String, Option<u64>> {
    let today = now / 86_400;
    let mut cache = TODAY.lock().unwrap_or_else(|e| e.into_inner());
    let state = paths.state_dir();
    if let Some((day, whose, answers)) = cache.as_ref() {
        if *day == today && *whose == state && drives.iter().all(|(root, _, _)| answers.contains_key(&root.to_ascii_uppercase())) {
            return answers.clone();
        }
    }
    let mut file = read(paths);
    for (root, free, total) in drives.iter().take(MAX_DRIVES) {
        let drive = root.to_ascii_uppercase();
        file.readings.retain(|r| !(r.day == today && r.drive == drive));
        file.readings.push(Reading { day: today, drive, free: *free, total: *total });
    }
    file.readings.retain(|r| r.day + KEEP_DAYS > today && r.day <= today);
    let drive_names: std::collections::HashSet<String> = file.readings.iter().map(|r| r.drive.clone()).collect();
    if drive_names.len() > MAX_DRIVES {
        file.readings.retain(|r| drives.iter().any(|(root, _, _)| root.eq_ignore_ascii_case(&r.drive)));
    }
    file.readings.sort_by(|a, b| a.drive.cmp(&b.drive).then(a.day.cmp(&b.day)));
    if let Ok(json) = serde_json::to_string(&file) {
        let _ = crate::util::write_atomic(&file_of(paths), &json);
    }
    let answers: HashMap<String, Option<u64>> = drives
        .iter()
        .map(|(root, _, total)| {
            let drive = root.to_ascii_uppercase();
            let days: Vec<(u64, u64)> = file.readings.iter().filter(|r| r.drive == drive).map(|r| (r.day, r.free)).collect();
            (drive, full_in(&days, *total, today))
        })
        .collect();
    *cache = Some((today, state, answers.clone()));
    answers
}

/// "At this rate C: is full in about 23 days."
pub fn full_in_words(root: &str, days: u64) -> String {
    format!("At this rate {} is full in about {}.", root.trim_end_matches('\\'), crate::util::plural(days, "day"))
}

/// In about how many days a drive with these (day, free) readings is full, if that's within two
/// months.
fn full_in(readings: &[(u64, u64)], total: u64, today: u64) -> Option<u64> {
    let mut days: Vec<(u64, u64)> = readings.iter().copied().filter(|(d, _)| *d + WINDOW_DAYS > today && *d <= today).collect();
    days.sort_by_key(|(d, _)| *d);
    days.dedup_by_key(|(d, _)| *d);
    // Start again after the last clean-up: free space well above anything the few days before
    // had. (A dip that comes back, like a big download deleted the next day, isn't one.)
    let jump = CLEAN_UP_BYTES.min(total / 20).max(1 << 30);
    let cleaned = (1..days.len()).rev().find(|&i| {
        let before = days[i.saturating_sub(3)..i].iter().map(|(_, f)| *f).max().unwrap_or(0);
        days[i].1 > before + jump
    });
    if let Some(i) = cleaned {
        days.drain(..i);
    }
    let (first, last) = (days.first()?, days.last()?);
    if last.0 - first.0 + 1 < MIN_DAYS || days.len() < MIN_DAYS as usize {
        return None;
    }
    let mut slopes: Vec<f64> = Vec::with_capacity(days.len() * days.len() / 2);
    for (i, a) in days.iter().enumerate() {
        for b in &days[i + 1..] {
            slopes.push((b.1 as f64 - a.1 as f64) / (b.0 - a.0) as f64);
        }
    }
    slopes.sort_by(|a, b| a.total_cmp(b));
    let rate = -slopes[slopes.len() / 2];
    if rate < MIN_RATE {
        return None;
    }
    let left = (last.1 as f64 / rate).ceil() as u64;
    (left <= WARN_DAYS).then_some(left.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    fn days(from: u64, frees: &[u64]) -> Vec<(u64, u64)> {
        frees.iter().enumerate().map(|(i, f)| (from + i as u64, *f * GB)).collect()
    }

    #[test]
    fn a_drive_filling_steadily_says_when_it_is_full() {
        // 2 GB a day, 40 GB left: 20 days.
        let r = days(100, &[60, 58, 56, 54, 52, 50, 48, 46, 44, 42, 40]);
        assert_eq!(full_in(&r, 1000 * GB, 110), Some(20));
        // Flat or growing: nothing to say.
        assert_eq!(full_in(&days(100, &[40; 10]), 1000 * GB, 109), None);
        assert_eq!(full_in(&days(100, &[40, 41, 42, 43, 44, 45, 46, 47]), 1000 * GB, 107), None);
        // Filling, but not within two months.
        assert_eq!(full_in(&days(100, &[400, 399, 398, 397, 396, 395, 394, 393]), 1000 * GB, 107), None);
    }

    #[test]
    fn it_is_said_plainly() {
        assert_eq!(full_in_words("C:\\", 23), "At this rate C: is full in about 23 days.");
        assert_eq!(full_in_words("D:\\", 1), "At this rate D: is full in about 1 day.");
    }

    #[test]
    fn too_few_days_say_nothing() {
        assert_eq!(full_in(&days(100, &[60, 50, 40, 30, 20]), 1000 * GB, 104), None);
        assert_eq!(full_in(&[], 1000 * GB, 100), None);
        // Seven readings, but spread over only a few days, still too few.
        let same = vec![(100, 60 * GB), (100, 59 * GB), (101, 58 * GB), (102, 57 * GB)];
        assert_eq!(full_in(&same, 1000 * GB, 102), None);
    }

    #[test]
    fn noisy_days_do_not_swing_it() {
        // About 2 GB a day with a couple of odd days (a big download deleted the next day).
        let r = days(100, &[60, 58, 47, 54, 52, 50, 48, 46, 30, 42, 40]);
        let d = full_in(&r, 1000 * GB, 110).unwrap();
        assert!((18..=23).contains(&d), "{d}");
    }

    #[test]
    fn a_big_clean_up_starts_the_count_again() {
        // Filling fast, then a 100 GB clean-up, then only three days: too few to tell.
        let mut r = days(100, &[60, 55, 50, 45, 40, 35, 30, 25]);
        r.extend(days(108, &[125, 124, 123]));
        assert_eq!(full_in(&r, 1000 * GB, 110), None);
        // A week after it, slowly filling: far off, so nothing.
        r.extend(days(111, &[122, 121, 120, 119, 118]));
        assert_eq!(full_in(&r, 1000 * GB, 115), None);
        // A week after it, filling fast again: counted from the clean-up only.
        let mut r = days(100, &[60, 55, 50, 45, 40, 35, 30, 25]);
        r.extend(days(108, &[125, 120, 115, 110, 105, 100, 95]));
        assert_eq!(full_in(&r, 1000 * GB, 114), Some(19));
    }

    #[test]
    fn readings_are_kept_once_a_day_and_for_a_while() {
        let root = std::env::temp_dir().join(format!("mom-space-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::under(&root);
        let day = 86_400;
        for i in 0..130u64 {
            *TODAY.lock().unwrap() = None;
            note(&paths, &[("C:\\".into(), (500 - i) * GB, 1000 * GB)], 20_000 * day + i * day);
        }
        // Twice the same day: one reading.
        *TODAY.lock().unwrap() = None;
        let answers = note(&paths, &[("c:\\".into(), 360 * GB, 1000 * GB)], 20_129 * day + 3600);
        let file = read(&paths);
        assert_eq!(file.readings.len(), KEEP_DAYS as usize, "120 days kept");
        assert_eq!(file.readings.last().unwrap().free, 360 * GB);
        assert_eq!(answers.get("C:\\"), Some(&None), "1 GB a day with 360 GB left is far off");
        // Later the same day, the answer comes from memory.
        std::fs::remove_file(file_of(&paths)).unwrap();
        assert_eq!(note(&paths, &[("C:\\".into(), 1, 1000 * GB)], 20_129 * day + 7200).get("C:\\"), Some(&None));
        assert!(!file_of(&paths).exists());
        *TODAY.lock().unwrap() = None;
        std::fs::remove_dir_all(&root).unwrap();
    }
}
