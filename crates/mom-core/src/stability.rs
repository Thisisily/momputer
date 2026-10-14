//! How steady the PC has been: thirty days of blue screens, unexpected shutdowns, graphics
//! driver resets, hardware errors and app crashes, from Windows' event logs. And a week's watch
//! after the memory speed or the BIOS changes ("Since your memory went to 3600 on 14 Oct: no
//! hardware errors"), because that's when a too-ambitious setting shows itself. Looks only;
//! the only thing written is mom's note of the memory speed and BIOS version it last saw.

use crate::eventlog::{local_midnight, query, short_date, Ev};
use crate::speed::Fix;
use crate::system::{check, plain, Check};
use crate::util::{now_secs, plural};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

const DAYS: u64 = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    BlueScreen,
    Shutdown,
    GpuReset,
    Hardware,
    App,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stability {
    /// Unix seconds: the first day's midnight, and now.
    pub from: u64,
    pub to: u64,
    /// One for each day, oldest first.
    pub days: Vec<Day>,
    pub blue_screens: u32,
    pub shutdowns: u32,
    pub gpu_resets: u32,
    pub hardware_errors: u32,
    pub app_crashes: u32,
    /// The apps that crashed most: (program, times), at most five.
    pub apps: Vec<(String, u32)>,
    /// Every moment, for the watch after a change.
    #[serde(skip)]
    pub(crate) marks: Vec<(u64, Kind)>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Day {
    /// Its midnight, Unix seconds.
    pub at: u64,
    pub blue_screens: u32,
    pub shutdowns: u32,
    pub gpu_resets: u32,
    pub hardware_errors: u32,
    pub app_crashes: u32,
}

static CACHE: Mutex<Option<Stability>> = Mutex::new(None);
const KEEP_FOR: u64 = 300;

/// The last thirty days, read at most every five minutes.
pub fn get() -> Stability {
    let now = now_secs();
    if let Some(s) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().filter(|s| now.saturating_sub(s.to) < KEEP_FOR) {
        return s.clone();
    }
    let ms = DAYS * 86_400 * 1000;
    let drivers: Vec<String> = crate::speed::GPU_DRIVERS.iter().map(|d| format!("Provider[@Name='{d}']")).collect();
    let system = format!(
        "*[System[(((Provider[@Name='Microsoft-Windows-WER-SystemErrorReporting'] or Provider[@Name='BugCheck']) and EventID=1001) or (Provider[@Name='Microsoft-Windows-Kernel-Power'] and EventID=41) or (Provider[@Name='Display'] and EventID=4101) or Provider[@Name='Microsoft-Windows-WHEA-Logger'] or {}) and TimeCreated[timediff(@SystemTime) <= {ms}]]]",
        drivers.join(" or ")
    );
    let mut events = query("System", &system, 1_000).unwrap_or_default();
    events.extend(query("Application", &format!("*[System[Provider[@Name='Application Error'] and EventID=1000 and TimeCreated[timediff(@SystemTime) <= {ms}]]]"), 1_000).unwrap_or_default());
    let s = build(&events, now);
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.clone());
    s
}

/// Sorts the events into moments: a crash logs several events, and a blue screen's restart is
/// also an unexpected shutdown, which isn't counted twice.
pub(crate) fn build(events: &[Ev], now: u64) -> Stability {
    let from = local_midnight(now.saturating_sub((DAYS - 1) * 86_400));
    let recent = |e: &&Ev| e.at >= from && e.at <= now + 60;
    let gpu = |e: &Ev| crate::speed::GPU_DRIVERS.contains(&e.provider.to_ascii_lowercase().as_str()) || (e.provider == "Display" && e.id == 4101);
    let times = |pick: &dyn Fn(&Ev) -> bool, gap: u64| crate::speed::moments(events.iter().filter(recent).filter(|e| pick(e)).map(|e| e.at).collect(), gap);
    let blue = times(&|e| e.id == 1001 && (e.provider == "BugCheck" || e.provider == "Microsoft-Windows-WER-SystemErrorReporting"), 600);
    let shutdowns: Vec<u64> = times(&|e| e.provider == "Microsoft-Windows-Kernel-Power" && e.id == 41, 600)
        .into_iter()
        .filter(|t| !blue.iter().any(|b| b.abs_diff(*t) < 1_800))
        .collect();
    let resets = times(&gpu, 120);
    let hardware = times(&|e| e.provider == "Microsoft-Windows-WHEA-Logger", 60);
    let apps: Vec<&Ev> = events.iter().filter(recent).filter(|e| e.provider == "Application Error" && e.id == 1000).collect();
    let mut marks: Vec<(u64, Kind)> = Vec::new();
    marks.extend(blue.iter().map(|t| (*t, Kind::BlueScreen)));
    marks.extend(shutdowns.iter().map(|t| (*t, Kind::Shutdown)));
    marks.extend(resets.iter().map(|t| (*t, Kind::GpuReset)));
    marks.extend(hardware.iter().map(|t| (*t, Kind::Hardware)));
    marks.extend(apps.iter().map(|e| (e.at, Kind::App)));
    marks.sort_by_key(|m| m.0);
    let mut days: Vec<Day> = (0..DAYS).map(|i| Day { at: local_midnight(from + i * 86_400 + 3_600 * 3), ..Day::default() }).collect();
    for (at, kind) in &marks {
        let Some(d) = days.iter_mut().rev().find(|d| d.at <= *at) else { continue };
        match kind {
            Kind::BlueScreen => d.blue_screens += 1,
            Kind::Shutdown => d.shutdowns += 1,
            Kind::GpuReset => d.gpu_resets += 1,
            Kind::Hardware => d.hardware_errors += 1,
            Kind::App => d.app_crashes += 1,
        }
    }
    let mut by_app: Vec<(String, u32)> = Vec::new();
    for e in &apps {
        let name = plain(e.get("AppName").filter(|n| !n.is_empty()).unwrap_or(e.text.split_whitespace().next().unwrap_or_default()));
        // "bad_module_info" is Windows' stand-in for a crash it couldn't put a name to.
        if name.is_empty() || name.eq_ignore_ascii_case("bad_module_info") {
            continue;
        }
        match by_app.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(&name)) {
            Some(a) => a.1 += 1,
            None => by_app.push((name, 1)),
        }
    }
    by_app.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    by_app.truncate(5);
    Stability {
        from,
        to: now,
        days,
        blue_screens: blue.len() as u32,
        shutdowns: shutdowns.len() as u32,
        gpu_resets: resets.len() as u32,
        hardware_errors: hardware.len() as u32,
        app_crashes: apps.len() as u32,
        apps: by_app,
        marks,
    }
}

impl Stability {
    /// The thirty days in a sentence: "1 blue screen and 62 app crashes in 30 days."
    pub fn summary(&self) -> String {
        let parts: Vec<String> = [
            (self.blue_screens, "blue screen", "blue screens"),
            (self.shutdowns, "unexpected shutdown", "unexpected shutdowns"),
            (self.gpu_resets, "graphics driver reset", "graphics driver resets"),
            (self.hardware_errors, "hardware error", "hardware errors"),
            (self.app_crashes, "app crash", "app crashes"),
        ]
        .iter()
        .filter(|(n, _, _)| *n > 0)
        .map(|(n, one, many)| if *n == 1 { format!("1 {one}") } else { format!("{} {many}", crate::util::human_count(*n as u64)) })
        .collect();
        match parts.as_slice() {
            [] => format!("Nothing crashed in {DAYS} days."),
            [one] => format!("{one} in {DAYS} days."),
            [rest @ .., last] => format!("{} and {last} in {DAYS} days.", rest.join(", ")),
        }
    }
}

// ------------------------------------------------------------------ the week after a change

/// What mom saw last time, and the last change she noticed.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
struct Seen {
    memory: u32,
    bios: String,
    change: Option<Change>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct Change {
    /// "memory" or "bios".
    pub what: String,
    pub from: String,
    pub to: String,
    /// When mom first saw it, Unix seconds.
    pub at: u64,
}

const WATCH_FOR: u64 = 7 * 86_400;

/// Notes the memory speed (MT/s, 0 when unknown) and BIOS version, and returns the change being
/// watched, if one happened in the last week. The first look only takes notes.
pub(crate) fn watch(state_dir: &Path, memory: u32, bios: &str, now: u64) -> Option<Change> {
    let file = state_dir.join("watch.json");
    let read = std::fs::metadata(&file).ok().filter(|m| m.is_file() && m.len() < 64 * 1024).and_then(|_| std::fs::read_to_string(&file).ok());
    let first = read.is_none();
    let mut seen: Seen = read.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let bios = plain(bios);
    let before = seen.clone();
    if memory > 0 {
        if !first && seen.memory > 0 && seen.memory != memory {
            seen.change = Some(Change { what: "memory".into(), from: seen.memory.to_string(), to: memory.to_string(), at: now });
        }
        seen.memory = memory;
    }
    if !bios.is_empty() {
        if !first && !seen.bios.is_empty() && seen.bios != bios {
            seen.change = Some(Change { what: "bios".into(), from: seen.bios.clone(), to: bios.clone(), at: now });
        }
        seen.bios = bios;
    }
    if first || seen != before {
        let _ = std::fs::create_dir_all(state_dir);
        if let Ok(text) = serde_json::to_string(&seen) {
            let _ = crate::util::write_atomic(&file, &text);
        }
    }
    seen.change.filter(|c| now.saturating_sub(c.at) <= WATCH_FOR)
}

/// How the week since a change is going.
pub(crate) fn watch_check(change: &Change, s: &Stability) -> Check {
    let since = |k: Kind| s.marks.iter().filter(|(at, kind)| *at >= change.at && *kind == k).count() as u64;
    let (hardware, blue) = (since(Kind::Hardware), since(Kind::BlueScreen));
    let when = short_date(change.at);
    let subject = if change.what == "memory" { format!("Since your memory went to {} on {when}", change.to) } else { format!("Since the BIOS update to {} on {when}", change.to) };
    if hardware == 0 && blue == 0 {
        return check("watch-change", "Hardware", "ok", format!("{subject}: no hardware errors"), "Mom keeps an eye on it for a week.".into(), None);
    }
    let what = match (hardware, blue) {
        (h, 0) => plural(h, "hardware error"),
        (0, b) => plural(b, "blue screen"),
        (h, b) => format!("{} and {}", plural(h, "hardware error"), plural(b, "blue screen")),
    };
    let (detail, prompt) = if change.what == "memory" {
        (
            "The new speed may be too much. One step down in the BIOS usually settles it.".to_string(),
            format!(
                "I changed my PC's memory speed from {} to {} MT/s in the BIOS on {when}. Since then Windows has logged {what}. Help me step it down one speed at a time in the BIOS and test each with TestMem5 (anta777 extreme config) or MemTest86, until there are no errors. Change nothing else.\n",
                change.from, change.to
            ),
        )
    } else {
        (
            "If they keep coming, loading the BIOS defaults puts memory and processor settings back.".to_string(),
            format!(
                "My PC's BIOS was updated from {} to {} on {when}. Since then Windows has logged {what}. Help me find out why, one step at a time: check the memory and processor settings the update may have changed, and whether loading the BIOS defaults helps. Change nothing else.\n",
                plain(&change.from).replace(['"', '`'], "'"),
                plain(&change.to).replace(['"', '`'], "'")
            ),
        )
    };
    check(
        "watch-change",
        "Hardware",
        "warn",
        format!("{subject}: {what}"),
        detail,
        Some(Fix { label: "Copy for your agent".into(), kind: "copy", value: prompt, hint: Some("Paste it to your agent.".into()) }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(provider: &str, id: u32, at: u64, app: Option<&str>) -> Ev {
        Ev { provider: provider.into(), id, at, data: app.map(|a| vec![("AppName".to_string(), a.to_string())]).unwrap_or_default(), text: String::new() }
    }

    #[test]
    fn thirty_days_of_trouble_sorted_into_moments() {
        let now = 1_792_000_000;
        let day = 86_400;
        let events = vec![
            // A blue screen, and the unexpected restart it caused: one moment, not two.
            ev("Microsoft-Windows-WER-SystemErrorReporting", 1001, now - 3 * day, None),
            ev("Microsoft-Windows-Kernel-Power", 41, now - 3 * day - 120, None),
            // A power cut on its own.
            ev("Microsoft-Windows-Kernel-Power", 41, now - 10 * day, None),
            // A driver reset logs several events.
            ev("Display", 4101, now - 5 * day, None),
            ev("nvlddmkm", 153, now - 5 * day + 5, None),
            ev("Microsoft-Windows-WHEA-Logger", 19, now - day, None),
            ev("Application Error", 1000, now - 2 * day, Some("lanternfall.exe")),
            ev("Application Error", 1000, now - 2 * day + 50, Some("lanternfall.exe")),
            ev("Application Error", 1000, now - 4 * day, Some("copperline.exe")),
            ev("Application Error", 1000, now - 6 * day, Some("bad_module_info")),
            // Too old to count.
            ev("Microsoft-Windows-WHEA-Logger", 19, now - 40 * day, None),
        ];
        let s = build(&events, now);
        assert_eq!((s.blue_screens, s.shutdowns, s.gpu_resets, s.hardware_errors, s.app_crashes), (1, 1, 1, 1, 4));
        assert_eq!(s.apps, vec![("lanternfall.exe".to_string(), 2), ("copperline.exe".to_string(), 1)], "a crash Windows couldn't name isn't an app");
        assert_eq!(s.days.len(), 30);
        assert_eq!(s.days.iter().map(|d| d.blue_screens + d.shutdowns + d.gpu_resets + d.hardware_errors + d.app_crashes).sum::<u32>(), 8);
        assert!(s.days.windows(2).all(|w| w[0].at < w[1].at));
        assert_eq!(s.summary(), "1 blue screen, 1 unexpected shutdown, 1 graphics driver reset, 1 hardware error and 4 app crashes in 30 days.");
        let quiet = build(&[], now);
        assert_eq!((quiet.app_crashes, quiet.days.len()), (0, 30));
        assert_eq!(quiet.summary(), "Nothing crashed in 30 days.");
    }

    #[test]
    fn a_change_is_watched_for_a_week() {
        let dir = std::env::temp_dir().join(format!("mom-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let now = 1_792_000_000;
        assert_eq!(watch(&dir, 3200, "4604", now), None, "the first look only takes notes");
        assert_eq!(watch(&dir, 3200, "4604", now + 60), None, "nothing changed");
        let c = watch(&dir, 3600, "4604", now + 3600).expect("the memory speed changed");
        assert_eq!((c.what.as_str(), c.from.as_str(), c.to.as_str()), ("memory", "3200", "3600"));
        assert_eq!(watch(&dir, 0, "", now + 7200), Some(c.clone()), "unknown readings don't count as changes");
        assert_eq!(watch(&dir, 3600, "4604", now + 3600 + 8 * 86_400), None, "a week later, the watch is over");
        let b = watch(&dir, 3600, "4645", now + 9 * 86_400).unwrap();
        assert_eq!((b.what.as_str(), b.to.as_str()), ("bios", "4645"));
        std::fs::remove_dir_all(&dir).unwrap();

        // What the watch says, quiet and otherwise.
        let mut s = build(&[], now + 4 * 86_400);
        let ok = watch_check(&c, &s);
        assert_eq!(ok.level, "ok");
        assert!(ok.title.starts_with("Since your memory went to 3600 on ") && ok.title.ends_with(": no hardware errors"), "{}", ok.title);
        s.marks = vec![(c.at + 600, Kind::Hardware), (c.at + 900, Kind::Hardware), (c.at - 600, Kind::BlueScreen)];
        let bad = watch_check(&c, &s);
        assert_eq!(bad.level, "warn");
        assert!(bad.title.ends_with(": 2 hardware errors"), "{}", bad.title);
        assert!(bad.fix.unwrap().value.contains("from 3200 to 3600 MT/s"));
    }
}
