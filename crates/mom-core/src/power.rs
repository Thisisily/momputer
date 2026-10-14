//! Starting and sleeping: how long Windows takes to start and what keeps slowing it (Windows'
//! own boot diagnostics, which only an administrator may read on many PCs: then nothing is
//! said), and what keeps waking the PC at night. Looks only.

use crate::eventlog::{local_hour, query, Ev};
use crate::speed::Fix;
use crate::system::{check, plain, Check};

// ------------------------------------------------------------------ starting

const BOOT_LOG: &str = "Microsoft-Windows-Diagnostics-Performance/Operational";

/// One start of Windows: how long it took (ms) and what slowed it, with how much each added (ms).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Boot {
    pub at: u64,
    pub ms: u64,
    pub slowed_by: Vec<(String, u64)>,
}

pub(crate) fn read_boots() -> Option<Vec<Boot>> {
    let events = query(BOOT_LOG, "*[System[(EventID>=100 and EventID<=110) and TimeCreated[timediff(@SystemTime) <= 2592000000]]]", 400)?;
    Some(boots(&events))
}

/// Event 100 is a start; 101–110, logged with it, are what slowed it.
pub(crate) fn boots(events: &[Ev]) -> Vec<Boot> {
    let mut out: Vec<Boot> = events
        .iter()
        .filter(|e| e.id == 100)
        .filter_map(|e| Some(Boot { at: e.at, ms: e.get("BootTime")?.parse().ok()?, slowed_by: Vec::new() }))
        .collect();
    out.sort_by_key(|b| b.at);
    for e in events.iter().filter(|e| (101..=110).contains(&e.id)) {
        let name = plain(e.get("FriendlyName").filter(|n| !n.trim().is_empty()).or(e.get("Name")).unwrap_or_default());
        let ms: u64 = e.get("DegradationTime").and_then(|d| d.parse().ok()).unwrap_or(0);
        if name.is_empty() {
            continue;
        }
        if let Some(b) = out.iter_mut().min_by_key(|b| b.at.abs_diff(e.at)).filter(|b| b.at.abs_diff(e.at) < 600) {
            if !b.slowed_by.iter().any(|(n, _)| n == &name) {
                b.slowed_by.push((name, ms));
            }
        }
    }
    out
}

fn secs(ms: u64) -> u64 {
    (ms + 500) / 1000
}

/// `startup` are the names of what starts with Windows, for the button.
pub(crate) fn boot_checks(boots: &[Boot], startup: &[String]) -> Vec<Check> {
    let recent: Vec<&Boot> = boots.iter().rev().take(5).collect();
    if recent.is_empty() {
        return vec![];
    }
    let last3: Vec<u64> = recent.iter().take(3).map(|b| b.ms).collect();
    let average = last3.iter().sum::<u64>() / last3.len() as u64;
    let slow = average > 60_000;
    // What slowed several of the last starts, the worst first.
    let mut seen: Vec<(String, usize, u64)> = Vec::new();
    for b in &recent {
        for (name, ms) in &b.slowed_by {
            match seen.iter_mut().find(|(n, _, _)| n == name) {
                Some(s) => {
                    s.1 += 1;
                    s.2 += ms;
                }
                None => seen.push((name.clone(), 1, *ms)),
            }
        }
    }
    seen.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));
    let repeat = seen.into_iter().find(|(_, n, _)| *n >= 2);
    let startup_fix = Fix { label: "Startup apps".into(), kind: "tab", value: "running:startup".into(), hint: None };
    match repeat {
        Some((name, n, total)) => {
            let starts = startup.iter().any(|s| {
                let (s, n) = (s.to_ascii_lowercase(), name.to_ascii_lowercase());
                !s.is_empty() && (s.contains(&n) || n.contains(&s))
            });
            let title = if n == recent.len() { format!("{name} slows every start") } else { format!("{name} slowed {n} of your last {} starts", recent.len()) };
            let mut detail = format!("It added about {} s each time.", secs(total / n as u64).max(1));
            if slow {
                detail.push_str(&format!(" Starting Windows takes {} s.", secs(average)));
            }
            let fix = if starts {
                startup_fix
            } else {
                let prompt = format!(
                    "Windows' boot diagnostics say \"{}\" slowed {n} of my last {} starts, by about {} s each time; starting Windows takes {} s. Help me find out what it is and whether it can start later or not at all, one step at a time. Change nothing else.\n",
                    name.replace(['"', '`'], "'"),
                    recent.len(),
                    secs(total / n as u64).max(1),
                    secs(average)
                );
                Fix { label: "Copy for your agent".into(), kind: "copy", value: prompt, hint: Some("Paste it to your agent.".into()) }
            };
            vec![check("boot-slowed", "Performance", if slow { "warn" } else { "tip" }, title, detail, Some(fix))]
        }
        None if slow => vec![check(
            "boot-slow",
            "Performance",
            "warn",
            format!("Starting Windows takes {} s", secs(average)),
            "Fewer apps starting with Windows usually helps most.".into(),
            Some(startup_fix),
        )],
        None => vec![],
    }
}

// ------------------------------------------------------------------ waking at night

/// One time the PC woke from sleep: when, and what woke it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Wake {
    pub at: u64,
    pub by: String,
}

pub(crate) fn read_wakes() -> Vec<Wake> {
    query("System", "*[System[Provider[@Name='Microsoft-Windows-Power-Troubleshooter'] and EventID=1 and TimeCreated[timediff(@SystemTime) <= 604800000]]]", 300)
        .map(|e| wakes(&e))
        .unwrap_or_default()
}

pub(crate) fn wakes(events: &[Ev]) -> Vec<Wake> {
    events
        .iter()
        .filter(|e| e.provider == "Microsoft-Windows-Power-Troubleshooter" && e.id == 1)
        .map(|e| Wake { at: e.get("WakeTime").and_then(crate::speed::parse_time).unwrap_or(e.at), by: source(e.get("WakeSourceText").unwrap_or_default()) })
        .collect()
}

/// What woke it, in a few words: "Timer - Windows will execute 'NT TASK\...\Reboot' scheduled
/// task..." is "the Reboot task"; "Device -USB Composite Device" is "USB Composite Device".
fn source(text: &str) -> String {
    let t = text.trim();
    if t.to_ascii_lowercase().starts_with("timer") {
        if let Some(task) = t.split('\'').nth(1).and_then(|q| q.rsplit('\\').next()).map(plain).filter(|s| !s.is_empty()) {
            return format!("the {task} task");
        }
        return "a timer".into();
    }
    let t = t.strip_prefix("Device -").or_else(|| t.strip_prefix("Device-")).unwrap_or(t);
    let s = plain(t);
    if s.is_empty() || s.eq_ignore_ascii_case("unknown") { "something Windows didn't name".into() } else { s }
}

/// Wakes between midnight and 7 in the morning, over the last week.
pub(crate) fn wake_checks(wakes: &[Wake], now: u64) -> Vec<Check> {
    let night: Vec<&Wake> = wakes.iter().filter(|w| now.saturating_sub(w.at) <= 7 * 86_400 && local_hour(w.at) < 7).collect();
    let per_night = night.len() as f64 / 7.0;
    if per_night < 3.0 {
        return vec![];
    }
    let mut by: Vec<(&str, usize)> = Vec::new();
    for w in &night {
        match by.iter_mut().find(|(s, _)| *s == w.by) {
            Some(b) => b.1 += 1,
            None => by.push((&w.by, 1)),
        }
    }
    by.sort_by(|a, b| b.1.cmp(&a.1));
    let top = by.first().map(|(s, _)| *s).unwrap_or("something Windows didn't name");
    let prompt = format!(
        "My PC wakes up at night: {} times in the last week between midnight and 7 am, mostly because of \"{}\". Help me find what wakes it (powercfg /lastwake, powercfg /waketimers and powercfg /devicequery wake_armed, in a terminal opened as administrator) and stop it, one step at a time. Keep Windows Update working; only stop it from waking the PC.\n",
        night.len(),
        top.replace(['"', '`'], "'")
    );
    let mut c = check(
        "night-wakes",
        "Performance",
        "tip",
        format!("Your PC woke {} times a night, mostly for {top}", per_night.round() as u64),
        "Windows Update, the mouse and the network card are the usual causes; each can be told not to.".into(),
        Some(Fix { label: "Power & sleep".into(), kind: "open", value: "ms-settings:powersleep".into(), hint: None }),
    );
    c.more.push(Fix { label: "Copy for your agent".into(), kind: "copy", value: prompt, hint: Some("Paste it to your agent.".into()) });
    vec![c]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: u32, at: u64, data: &[(&str, &str)]) -> Ev {
        Ev { provider: "Microsoft-Windows-Diagnostics-Performance".into(), id, at, data: data.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), text: String::new() }
    }

    #[test]
    fn what_slows_several_starts_is_named_with_its_button() {
        let day = 86_400;
        let mut events = Vec::new();
        for (i, ms) in [72_000, 81_000, 65_000].iter().enumerate() {
            let at = 1_791_000_000 + i as u64 * day;
            events.push(ev(100, at, &[("BootTime", &ms.to_string())]));
            events.push(ev(101, at + 2, &[("Name", "lanternfall.exe"), ("FriendlyName", "Lanternfall Updater"), ("DegradationTime", "9000")]));
        }
        events.push(ev(102, 1_791_000_000 + 3, &[("Name", "copperline.sys"), ("FriendlyName", ""), ("DegradationTime", "2000")]));
        let b = boots(&events);
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].slowed_by, vec![("Lanternfall Updater".to_string(), 9000), ("copperline.sys".to_string(), 2000)]);
        let c = boot_checks(&b, &["Lanternfall Updater".into()]);
        assert_eq!((c[0].level, c[0].title.as_str()), ("warn", "Lanternfall Updater slows every start"));
        assert_eq!(c[0].detail, "It added about 9 s each time. Starting Windows takes 73 s.");
        assert_eq!(c[0].fix.as_ref().unwrap().kind, "tab", "it starts with Windows: the Startup list switches it off");
        let c = boot_checks(&b, &[]);
        assert_eq!(c[0].fix.as_ref().unwrap().kind, "copy");
        assert!(c[0].fix.as_ref().unwrap().value.contains("\"Lanternfall Updater\" slowed 3 of my last 3 starts"));
    }

    #[test]
    fn quick_starts_say_nothing_and_slow_ones_say_how_slow() {
        let quick: Vec<Ev> = (0..3).map(|i| ev(100, 1_791_000_000 + i * 86_400, &[("BootTime", "25000")])).collect();
        assert!(boot_checks(&boots(&quick), &[]).is_empty());
        let slow: Vec<Ev> = (0..3).map(|i| ev(100, 1_791_000_000 + i * 86_400, &[("BootTime", "95000")])).collect();
        let c = boot_checks(&boots(&slow), &[]);
        assert_eq!(c[0].title, "Starting Windows takes 95 s");
        assert!(boot_checks(&[], &[]).is_empty());
    }

    #[test]
    fn night_wakes_are_counted_and_their_cause_named() {
        let now = 1_792_000_000;
        // Find a moment at 3 in the morning, PC time, a few days ago.
        let mut three_am = crate::eventlog::local_midnight(now - 3 * 86_400) + 3 * 3600;
        while local_hour(three_am) != 3 {
            three_am += 3600;
        }
        let mut list: Vec<Wake> = (0..7u64)
            .flat_map(|d| (0..3u64).map(move |k| (d, k)))
            .map(|(d, k)| Wake { at: three_am - (d % 3) * 86_400 + k * 600, by: source("Timer - Windows will execute 'NT TASK\\Lanternfall\\Nightly Scan' scheduled task that requested waking the computer.") })
            .collect();
        list.push(Wake { at: three_am + 900, by: source("Device -Copperline Mouse") });
        let c = wake_checks(&list, now);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].title, "Your PC woke 3 times a night, mostly for the Nightly Scan task");
        assert!(c[0].more[0].value.contains("22 times in the last week"));
        assert_eq!(source("Device -Copperline Mouse"), "Copperline Mouse");
        assert_eq!(source(""), "something Windows didn't name");
        // A daytime wake or two is nothing.
        assert!(wake_checks(&list[..2], now).is_empty());
    }
}
