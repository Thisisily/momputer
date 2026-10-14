//! Mom's Sunday letter: once a week, a few warm lines about the week (space cleared, what
//! Autopilot did, how much got done) and the one thing most worth doing next, with its button.
//! It comes the first time mom looks on or after Sunday at ten in the morning, and the last one
//! can always be read again from "What I did". The very first week she only starts counting.

use crate::history::{self, kind, Entry};
use crate::paths::Paths;
use crate::util::{human_bytes, plural};
use chrono::{Datelike, Local, TimeZone};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const DAY: u64 = 86_400;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Letter {
    /// When it was written (Unix seconds).
    pub at: u64,
    /// The week it's about: from then to `at`.
    pub from: u64,
    /// Space cleared this week (deleted or sent to the Recycle Bin), and moved to the archive.
    pub freed: u64,
    pub moved: u64,
    /// What Autopilot cleared by itself.
    pub auto_freed: u64,
    /// Things done this week, all told.
    pub done: u64,
    /// The letter, a few short lines.
    pub lines: Vec<String>,
    /// The one thing most worth doing next, from the checkup.
    pub next: Option<Next>,
}

/// A finding from the checkup, with the button to do something about it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Next {
    pub title: String,
    pub detail: String,
    pub fix: Option<Button>,
}

/// A button as the checkup gives it: copy something, open a page or a Windows program, or show
/// a part of the widget.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Button {
    pub label: String,
    pub kind: String,
    pub value: String,
    pub hint: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct State {
    /// The last letter, or when mom started counting.
    last: u64,
    letter: Option<Letter>,
    /// The letter you last opened (its `at`).
    read: u64,
}

fn file(paths: &Paths) -> PathBuf {
    paths.state_dir().join("letter.json")
}

fn load(paths: &Paths) -> Option<State> {
    let f = file(paths);
    std::fs::metadata(&f).ok().filter(|m| m.is_file() && m.len() < 256 << 10)?;
    std::fs::read(f).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

fn save(paths: &Paths, st: &State) {
    if let Ok(json) = serde_json::to_string(st) {
        let _ = crate::util::write_atomic(&file(paths), &json);
    }
}

/// The last letter, to read again.
pub fn latest(paths: &Paths) -> Option<Letter> {
    load(paths).and_then(|s| s.letter)
}

/// The last letter hasn't been opened yet.
pub fn unread(paths: &Paths) -> bool {
    load(paths).is_some_and(|s| s.letter.as_ref().is_some_and(|l| l.at > s.read))
}

/// You opened the last letter.
pub fn mark_read(paths: &Paths) {
    if let Some(mut st) = load(paths) {
        if let Some(at) = st.letter.as_ref().map(|l| l.at) {
            st.read = at;
            save(paths, &st);
        }
    }
}

/// Ten in the morning, local time, on the most recent Sunday at or before `now`.
pub fn sunday_before(now: u64) -> u64 {
    let Some(t) = Local.timestamp_opt(now as i64, 0).single() else { return now };
    let date = t.date_naive() - chrono::Days::new(u64::from(t.weekday().num_days_from_sunday()));
    let at = date
        .and_hms_opt(10, 0, 0)
        .and_then(|ten| Local.from_local_datetime(&ten).earliest())
        .map(|d| d.timestamp().max(0) as u64)
        .unwrap_or(now);
    if at > now { at.saturating_sub(7 * DAY) } else { at }
}

/// Whether this week's letter is due now. The first time, mom only starts counting, so the
/// first letter has a week to tell.
pub fn due(paths: &Paths, now: u64) -> bool {
    match load(paths) {
        Some(st) if st.last > 0 => st.last < sunday_before(now),
        _ => {
            save(paths, &State { last: now, ..Default::default() });
            false
        }
    }
}

/// Writes this week's letter from "What I did" and the checkup, keeps it, and returns it.
pub fn write(paths: &Paths, now: u64, checks: &[crate::system::Check]) -> Letter {
    let st = load(paths).unwrap_or_default();
    let from = st.last.max(now.saturating_sub(7 * DAY));
    let week: Vec<Entry> = history::entries(paths).into_iter().filter(|e| (from..=now).contains(&e.at)).collect();
    let letter = compose(&week, from, now, checks);
    save(paths, &State { last: now, letter: Some(letter.clone()), read: st.read });
    letter
}

fn compose(week: &[Entry], from: u64, now: u64, checks: &[crate::system::Check]) -> Letter {
    let done: Vec<&Entry> = week.iter().filter(|e| e.undoes.is_none()).collect();
    let cleared = |e: &&&Entry| matches!(e.kind.as_str(), kind::CLEAN | kind::ADMIN_CLEAN | kind::RECYCLE | kind::EMPTY_BIN | kind::COPIES);
    let freed: u64 = done.iter().filter(cleared).map(|e| e.bytes).sum();
    let auto_freed: u64 = done.iter().filter(cleared).filter(|e| e.auto).map(|e| e.bytes).sum();
    let moved: u64 = done.iter().filter(|e| e.kind == kind::ARCHIVE).map(|e| e.bytes).sum();
    let mut lines = Vec::new();
    lines.push(if freed > 0 { format!("This week I cleared {} for you.", human_bytes(freed)) } else { "A quiet week: nothing needed clearing.".into() });
    if moved > 0 {
        lines.push(format!("{} went to your archive drive, safe and sound.", human_bytes(moved)));
    }
    if auto_freed > 0 {
        lines.push(format!("Autopilot did {} of that by itself.", human_bytes(auto_freed)));
    }
    if !done.is_empty() {
        lines.push(format!("That's {} done together.", plural(done.len() as u64, "thing")));
    }
    lines.push("Love, mom c:".into());
    Letter { at: now, from, freed, moved, auto_freed, done: done.len() as u64, lines, next: next_thing(checks) }
}

/// The finding most worth doing something about: a warning with a button, else a tip with one.
/// A button that acts on something only true when the checkup ran (a program, a registry fix)
/// becomes "Show me", which opens the checkup, since a letter is read later.
fn next_thing(checks: &[crate::system::Check]) -> Option<Next> {
    let pick = checks
        .iter()
        .filter(|c| c.fix.is_some())
        .find(|c| c.level == "warn")
        .or_else(|| checks.iter().filter(|c| c.fix.is_some()).find(|c| c.level == "tip"))?;
    let fix = pick.fix.as_ref().map(|f| match f.kind {
        "copy" | "open" | "tab" | "launch" => Button { label: f.label.clone(), kind: f.kind.into(), value: f.value.clone(), hint: f.hint.clone() },
        _ => Button { label: "Show me".into(), kind: "tab".into(), value: "upkeep:windows".into(), hint: None },
    });
    let short = |s: &str, n: usize| s.chars().take(n).collect::<String>();
    Some(Next { title: short(&pick.title, 160), detail: short(&pick.detail, 400), fix })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::speed::Fix;
    use crate::system::Check;
    use chrono::Timelike;

    fn check(level: &'static str, title: &str, fix: Option<Fix>) -> Check {
        Check { id: title.into(), area: "Hardware", level, title: title.into(), detail: "One line.".into(), fix, more: vec![], items: vec![] }
    }

    #[test]
    fn the_letter_tells_the_week_and_the_next_thing_to_do() {
        let e = |kind: &str, bytes: u64, auto: bool| Entry { kind: kind.into(), bytes, auto, at: 50, ..Default::default() };
        let week = [
            e(kind::CLEAN, 2 << 30, true),
            e(kind::RECYCLE, 1 << 30, false),
            e(kind::ARCHIVE, 4 << 30, false),
            e(kind::STOPPED, 0, false),
            Entry { kind: kind::BRING_BACK.into(), undoes: Some("x".into()), at: 60, ..Default::default() },
        ];
        let copy = Fix { label: "Copy for your agent".into(), kind: "copy", value: "My memory runs at 3200…".into(), hint: None };
        let regfix = Fix { label: "Remove them".into(), kind: "regfix", value: "registry-run-user".into(), hint: None };
        let checks = [check("tip", "A tip", Some(copy.clone())), check("warn", "Old registry entries", Some(regfix)), check("warn", "No button", None)];
        let l = compose(&week, 0, 100, &checks);
        assert_eq!(
            l.lines,
            [
                "This week I cleared 3.0 GB for you.",
                "4.0 GB went to your archive drive, safe and sound.",
                "Autopilot did 2.0 GB of that by itself.",
                "That's 4 things done together.",
                "Love, mom c:"
            ]
        );
        assert_eq!((l.freed, l.moved, l.done), (3 << 30, 4 << 30, 4));
        let next = l.next.unwrap();
        assert_eq!(next.title, "Old registry entries", "a warning first");
        assert_eq!(next.fix.unwrap(), Button { label: "Show me".into(), kind: "tab".into(), value: "upkeep:windows".into(), hint: None });
        // A quiet week, with only a tip.
        let l = compose(&[], 0, 100, &checks[..1]);
        assert_eq!(l.lines, ["A quiet week: nothing needed clearing.", "Love, mom c:"]);
        assert_eq!(l.next.unwrap().fix.unwrap().kind, "copy");
        assert!(compose(&[], 0, 100, &[]).next.is_none());
    }

    #[test]
    fn it_comes_on_sundays_after_ten_and_the_first_week_only_counts() {
        let now = 1_791_935_078;
        let sunday = sunday_before(now);
        let t = Local.timestamp_opt(sunday as i64, 0).single().unwrap();
        assert_eq!((t.weekday(), t.hour(), t.minute()), (chrono::Weekday::Sun, 10, 0));
        assert!(sunday <= now && now - sunday < 7 * DAY + 3600);
        assert_eq!(sunday_before(sunday), sunday, "ten o'clock itself counts");
        assert!(sunday_before(sunday - 1) < sunday, "a minute before ten is still last week");

        let root = std::env::temp_dir().join(format!("mom-letter-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::under(&root);
        assert!(!due(&paths, now), "the first look only starts counting");
        assert!(!due(&paths, now + 60));
        let next_sunday = sunday + 7 * DAY;
        assert!(due(&paths, next_sunday + 60));
        let l = write(&paths, next_sunday + 60, &[]);
        assert_eq!(latest(&paths).unwrap(), l);
        assert!(unread(&paths));
        mark_read(&paths);
        assert!(!unread(&paths));
        assert!(!due(&paths, next_sunday + 3600), "one a week");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
