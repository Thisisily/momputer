//! Autopilot: once a week, while you're away and the PC is on mains power, mom clears the
//! clean-ups that are safe without looking (caches and scratch that come back by themselves), and
//! only what she'd clear if you pressed their buttons: never what you said to keep, nothing kept
//! for being recent, nothing in use, and nothing that needs an administrator or a look first.
//! Off unless you switch it on. Each clean-up goes into "What I did", marked as Autopilot's.

use super::TrashCtx;
use crate::paths::Paths;
use crate::util::{human_bytes, now_secs};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// At most once in this long.
pub const WEEK: u64 = 7 * 86_400;
/// You've been away from the keyboard and mouse this long.
pub const AWAY_SECS: u64 = 10 * 60;

/// What a run did.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Run {
    /// Unix seconds.
    pub at: u64,
    pub freed: u64,
    pub items: u64,
    /// The clean-ups that took something, by name.
    pub spots: Vec<String>,
}

fn file(paths: &Paths) -> PathBuf {
    paths.state_dir().join("autopilot.json")
}

/// The last run, if there was one.
pub fn last(paths: &Paths) -> Option<Run> {
    let f = file(paths);
    std::fs::metadata(&f).ok().filter(|m| m.is_file() && m.len() < 64 << 10)?;
    std::fs::read(f).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

fn save(paths: &Paths, run: &Run) {
    if let Ok(json) = serde_json::to_string(run) {
        let _ = crate::util::write_atomic(&file(paths), &json);
    }
}

/// The clean-ups it may run: the ones safe to clear without looking, decided by mom and not a
/// tool, and done as you.
pub fn spots() -> Vec<&'static str> {
    super::spot_ids().into_iter().filter(|id| super::cleans_without_asking(id)).collect()
}

/// Whether it's time: switched on, a week since it last ran, you've been away a while, and the
/// PC is on mains power. What can't be told counts as no.
pub fn due(on: bool, last: Option<u64>, now: u64, away_secs: Option<u64>, mains: Option<bool>) -> bool {
    on && last.is_none_or(|t| now.saturating_sub(t) >= WEEK) && away_secs.is_some_and(|a| a >= AWAY_SECS) && mains == Some(true)
}

/// Runs if it's time (see `due`), and says what it did.
pub fn run_if_due(ctx: &TrashCtx, away_secs: Option<u64>, mains: Option<bool>) -> Option<Run> {
    let last = last(ctx.paths).map(|r| r.at);
    if !due(ctx.cfg.autopilot, last, now_secs(), away_secs, mains) || ctx.elevated {
        return None;
    }
    Some(run(ctx))
}

/// Cleans each of `spots`, the way its button would with nothing changed (only what goes), notes
/// each in "What I did", and remembers it ran.
pub fn run(ctx: &TrashCtx) -> Run {
    let mut done = Run { at: now_secs(), ..Default::default() };
    if ctx.elevated {
        return done;
    }
    for id in spots() {
        let since = now_secs();
        let r = super::clean(id, ctx, None);
        if r.deleted == 0 && r.freed == 0 {
            continue;
        }
        crate::history::cleaned(ctx.paths, &r, None, true, since);
        done.freed += r.freed;
        done.items += r.deleted;
        if let Some((label, _)) = super::about(id) {
            done.spots.push(label.to_string());
        }
    }
    save(ctx.paths, &done);
    done
}

/// What to tell you: "I tidied 3.2 GB: npm cache, old temp files…". None when it found nothing.
pub fn said(run: &Run) -> Option<String> {
    if run.freed == 0 && run.items == 0 {
        return None;
    }
    let names: Vec<String> = run.spots.iter().map(|s| crate::history::in_sentence(s)).collect();
    let more = if names.len() > 3 { "…" } else { "" };
    Some(format!("I tidied {}: {}{more}", human_bytes(run.freed), names.iter().take(3).cloned().collect::<Vec<_>>().join(", ")))
}

/// Seconds since the last key press or mouse move. None if Windows won't say.
#[cfg(windows)]
pub fn away_secs() -> Option<u64> {
    #[repr(C)]
    struct LastInput {
        size: u32,
        time: u32,
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetLastInputInfo(info: *mut LastInput) -> i32;
    }
    let mut info = LastInput { size: std::mem::size_of::<LastInput>() as u32, time: 0 };
    // SAFETY: `info` is ours, sized as the call expects.
    if unsafe { GetLastInputInfo(&mut info) } == 0 {
        return None;
    }
    // SAFETY: no arguments.
    let now = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() };
    Some(u64::from(now.wrapping_sub(info.time)) / 1000)
}

#[cfg(not(windows))]
pub fn away_secs() -> Option<u64> {
    None
}

/// The PC runs on mains power (a desktop always does). None if Windows won't say.
#[cfg(windows)]
pub fn on_mains() -> Option<bool> {
    #[repr(C)]
    struct PowerStatus {
        ac_line: u8,
        battery: u8,
        percent: u8,
        saver: u8,
        life: u32,
        full_life: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetSystemPowerStatus(status: *mut PowerStatus) -> i32;
    }
    let mut s = PowerStatus { ac_line: 255, battery: 255, percent: 255, saver: 0, life: 0, full_life: 0 };
    // SAFETY: `s` is ours, laid out as SYSTEM_POWER_STATUS.
    if unsafe { GetSystemPowerStatus(&mut s) } == 0 {
        return None;
    }
    match (s.ac_line, s.battery) {
        (1, _) => Some(true),
        (0, _) => Some(false),
        (_, 128) => Some(true), // unknown, but there's no battery at all
        _ => None,
    }
}

#[cfg(not(windows))]
pub fn on_mains() -> Option<bool> {
    None
}

#[cfg(test)]
mod tests {
    use super::super::tests::{ctx, fixture, write};
    use super::*;
    use crate::config::TrashConfig;

    #[test]
    fn it_only_takes_what_safe_clean_ups_would_and_only_once_a_week() {
        let root = fixture("autopilot");
        let paths = Paths::under(&root);
        let (old, new, kept) = (paths.temp.join("old.bin"), paths.temp.join("new.bin"), paths.temp.join("kept.bin"));
        write(&old, 100, 100);
        write(&new, 100, 1); // kept for being recent: you may tick it, Autopilot never does
        write(&kept, 100, 100);
        // A clean-up that wants a look first, and one that needs an administrator.
        let model = paths.home.join(".cache").join("huggingface").join("hub").join("models--example--lanternfall").join("blobs").join("a.bin");
        write(&model, 100, 400);
        let upgrade = paths.system_drive.join("Windows10Upgrade").join("setup.exe");
        write(&upgrade, 100, 400);
        let mut cfg = TrashConfig { autopilot: true, ..TrashConfig::default() };
        cfg.keep.push(kept.to_string_lossy().to_string());
        let ctx = ctx(&paths, &cfg);
        assert!(spots().iter().all(|id| {
            let (_, action) = super::super::about(id).unwrap();
            super::super::cleans_without_asking(id) && matches!(action, "delete" | "recycle") && !super::super::needs_admin(id)
        }));
        assert!(!spots().contains(&"huggingface-cache") && !spots().contains(&"windows-junk") && !spots().contains(&"recycle-bin"));

        let ran = run_if_due(&ctx, Some(AWAY_SECS), Some(true)).expect("never ran before, so it's due");
        assert!(!old.exists(), "old temp files go");
        assert!(new.exists() && kept.exists(), "recent and kept things stay");
        assert!(model.exists() && upgrade.exists(), "clean-ups that want a look or an administrator are left alone");
        assert_eq!((ran.items, ran.spots.clone()), (1, vec!["Old temp files".to_string()]));
        assert_eq!(said(&ran).unwrap(), "I tidied 100 B: old temp files");
        assert_eq!(last(&paths).unwrap(), ran);

        // Not again within the week, whatever else is true.
        write(&old, 100, 100);
        assert!(run_if_due(&ctx, Some(AWAY_SECS * 10), Some(true)).is_none());
        assert!(old.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn it_waits_for_you_to_be_away_on_mains_power_and_switched_on() {
        let now = 10 * WEEK;
        assert!(due(true, None, now, Some(AWAY_SECS), Some(true)));
        assert!(due(true, Some(now - WEEK), now, Some(AWAY_SECS), Some(true)));
        assert!(!due(true, Some(now - WEEK + 60), now, Some(AWAY_SECS), Some(true)), "less than a week");
        assert!(!due(false, None, now, Some(AWAY_SECS), Some(true)), "off");
        assert!(!due(true, None, now, Some(AWAY_SECS - 1), Some(true)), "you're still here");
        assert!(!due(true, None, now, None, Some(true)), "can't tell whether you're away");
        assert!(!due(true, None, now, Some(AWAY_SECS), Some(false)), "on battery");
        assert!(!due(true, None, now, Some(AWAY_SECS), None), "can't tell the power");
        assert_eq!(said(&Run::default()), None);
        let many = Run { freed: 3 << 30, items: 9, spots: ["npm cache", "Old temp files", "pip cache", "Crash reports"].map(String::from).to_vec(), ..Default::default() };
        assert_eq!(said(&many).unwrap(), "I tidied 3.0 GB: npm cache, old temp files, pip cache…");
    }
}
