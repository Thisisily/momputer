//! What Codex is doing, from Codex's own records.
//!
//! One Codex app process hosts every thread (conversation), so processes alone can't say
//! "Codex is working on *Ship the launch video* in storefront". Codex keeps that in
//! `~/.codex/state_*.sqlite` (threads: name, folder, last activity) and keeps its scheduled
//! automations in `~/.codex/automations/<id>/automation.toml`. Mom reads both, read-only.

use crate::util::now_secs;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Automation {
    pub id: String,
    pub name: String,
    /// ACTIVE, PAUSED, ...
    pub status: String,
    pub active: bool,
    /// "14 times a day, 9:00 to 22:30", "every hour at :18".
    pub schedule: String,
    /// Next wake, unix seconds (Codex adds a little jitter, so "around").
    pub next_run: Option<u64>,
    pub target_thread: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,
    pub name: String,
    pub cwd: Option<String>,
    pub project: Option<String>,
    pub created: u64,
    /// Last time Codex wrote to it.
    pub updated: u64,
    /// Codex touched it in the last few minutes: it's working right now.
    pub working: bool,
    /// Sub-agent threads it started that are busy too.
    pub subagents: u32,
    pub automation: Option<Automation>,
    /// Opens the thread in the Codex app.
    pub link: String,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexState {
    pub threads: Vec<Thread>,
    pub automations: Vec<Automation>,
    /// Why the thread list may be incomplete.
    pub note: Option<String>,
}

/// A thread counts as awake if Codex wrote to it this recently.
const AWAKE_SECS: u64 = 20 * 60;
/// ...and as working right now if this recently.
const WORKING_SECS: u64 = 3 * 60;

pub fn state(codex_dir: &Path) -> CodexState {
    if !codex_dir.is_dir() {
        return CodexState::default();
    }
    let now = now_secs();
    let automations = automations(codex_dir, now);
    let targets: Vec<String> = automations
        .iter()
        .filter(|a| a.active)
        .filter_map(|a| a.target_thread.clone())
        .collect();
    let (mut threads, note) = match threads_from_db(codex_dir, now, &targets) {
        Ok(t) => (t, None),
        Err(e) => (threads_from_rollouts(codex_dir, now, &targets), Some(format!("Codex's thread database: {e}"))),
    };
    for t in &mut threads {
        t.automation = automations
            .iter()
            .find(|a| a.target_thread.as_deref() == Some(t.id.as_str()))
            .cloned();
    }
    // Working threads first, then scheduled ones, newest first.
    threads.sort_by(|a, b| (b.working, b.automation.is_some(), b.updated).cmp(&(a.working, a.automation.is_some(), a.updated)));
    CodexState { threads, automations, note }
}

fn clean_path(p: &str) -> String {
    p.trim_start_matches(r"\\?\").to_string()
}

fn thread(id: String, name: String, cwd: Option<String>, created: u64, updated: u64, now: u64) -> Thread {
    let cwd = cwd.map(|c| clean_path(&c)).filter(|c| !c.is_empty());
    Thread {
        link: format!("codex://threads/{id}"),
        project: cwd.as_deref().and_then(crate::scan::project_name),
        working: now.saturating_sub(updated) <= WORKING_SECS,
        id,
        name,
        cwd,
        created,
        updated,
        subagents: 0,
        automation: None,
    }
}

fn newest_state_db(codex_dir: &Path) -> Option<PathBuf> {
    let mut dbs: Vec<(u32, PathBuf)> = std::fs::read_dir(codex_dir)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let n = name.strip_prefix("state_")?.strip_suffix(".sqlite")?.parse().ok()?;
            Some((n, e.path()))
        })
        .collect();
    dbs.sort();
    dbs.pop().map(|(_, p)| p)
}

fn threads_from_db(codex_dir: &Path, now: u64, targets: &[String]) -> anyhow::Result<Vec<Thread>> {
    use rusqlite::{Connection, OpenFlags};
    let db = newest_state_db(codex_dir).ok_or_else(|| anyhow::anyhow!("not found"))?;
    let con = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    con.busy_timeout(std::time::Duration::from_millis(300))?;
    // Codex changes its schema now and then; only ask for columns that exist.
    let cols: Vec<String> = con
        .prepare("SELECT name FROM pragma_table_info('threads')")?
        .query_map([], |r| r.get::<_, String>(0))?
        .filter_map(Result::ok)
        .collect();
    let has = |c: &str| cols.iter().any(|x| x == c);
    anyhow::ensure!(has("id") && has("cwd"), "unexpected layout");
    let updated = if has("updated_at_ms") { "updated_at_ms" } else if has("updated_at") { "updated_at * 1000" } else { anyhow::bail!("no update time") };
    let created = if has("created_at_ms") { "created_at_ms" } else if has("created_at") { "created_at * 1000" } else { "0" };
    let name = match (has("name"), has("title")) {
        (true, true) => "COALESCE(NULLIF(name, ''), NULLIF(title, ''), '')",
        (true, false) => "COALESCE(name, '')",
        (false, true) => "COALESCE(title, '')",
        _ => "''",
    };
    let not_archived = if has("archived") { "AND archived = 0" } else { "" };
    let since_ms = (now.saturating_sub(AWAKE_SECS) * 1000) as i64;
    let target_list = targets
        .iter()
        .filter(|t| t.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(",");
    let target_clause = if target_list.is_empty() { String::new() } else { format!("OR id IN ({target_list})") };
    let sql = format!(
        "SELECT id, cwd, {name}, {created}, {updated} FROM threads \
         WHERE ({updated} >= ?1 {target_clause}) {not_archived} ORDER BY {updated} DESC LIMIT 40"
    );
    let rows: Vec<(String, Option<String>, String, i64, i64)> = con
        .prepare(&sql)?
        .query_map([since_ms], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
        .filter_map(Result::ok)
        .collect();
    let mut threads: Vec<Thread> = rows
        .into_iter()
        .map(|(id, cwd, name, c, u)| thread(id, crate::util::ellipsize(name.trim(), 80), cwd, (c.max(0) / 1000) as u64, (u.max(0) / 1000) as u64, now))
        .collect();
    // Sub-agents: child threads that are awake roll up into their parent.
    let has_edges: bool = con
        .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='thread_spawn_edges'", [], |r| r.get::<_, i64>(0))
        .map(|n| n > 0)
        .unwrap_or(false);
    if has_edges && threads.len() > 1 {
        let ids: Vec<String> = threads.iter().map(|t| t.id.clone()).collect();
        let mut children: Vec<String> = Vec::new();
        for t in threads.iter_mut() {
            let kids: Vec<String> = con
                .prepare("SELECT child_thread_id FROM thread_spawn_edges WHERE parent_thread_id = ?1")?
                .query_map([&t.id], |r| r.get::<_, String>(0))?
                .filter_map(Result::ok)
                .filter(|k| ids.contains(k))
                .collect();
            t.subagents = kids.len() as u32;
            children.extend(kids);
        }
        threads.retain(|t| !children.contains(&t.id));
    }
    Ok(threads)
}

/// Fallback when the database can't be read: recent rollout logs carry the folder in their first line.
fn threads_from_rollouts(codex_dir: &Path, now: u64, targets: &[String]) -> Vec<Thread> {
    let mut out = Vec::new();
    let sessions = codex_dir.join("sessions");
    for e in walkdir::WalkDir::new(&sessions).max_depth(4).into_iter().filter_map(Result::ok) {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
            continue;
        }
        let Ok(meta) = e.metadata() else { continue };
        let updated = crate::util::mtime_secs(&meta);
        let is_target = targets.iter().any(|t| name.contains(t.as_str()));
        if now.saturating_sub(updated) > AWAKE_SECS && !is_target {
            continue;
        }
        let Some((id, cwd, created)) = rollout_meta(e.path()) else { continue };
        out.push(thread(id, String::new(), cwd, created, updated, now));
    }
    out
}

fn rollout_meta(path: &Path) -> Option<(String, Option<String>, u64)> {
    use std::io::{BufRead, Read};
    let f = std::fs::File::open(path).ok()?;
    let mut line = String::new();
    std::io::BufReader::new(f.take(4 << 20)).read_line(&mut line).ok()?;
    let v: serde_json::Value = serde_json::from_str(&line).ok()?;
    let p = v.get("payload").unwrap_or(&v);
    let id = p.get("id")?.as_str()?.to_string();
    let cwd = p.get("cwd").and_then(|c| c.as_str()).map(String::from);
    let created = p
        .get("timestamp")
        .and_then(|t| t.as_str())
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|d| d.timestamp().max(0) as u64)
        .unwrap_or(0);
    Some((id, cwd, created))
}

pub fn automations(codex_dir: &Path, now: u64) -> Vec<Automation> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(codex_dir.join("automations")) else { return out };
    for e in rd.filter_map(Result::ok) {
        let file = e.path().join("automation.toml");
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        let Ok(v) = text.parse::<toml::Table>() else { continue };
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
        let status = s("status").unwrap_or_else(|| "UNKNOWN".into());
        let rrule = s("rrule").unwrap_or_default();
        let active = status.eq_ignore_ascii_case("active");
        out.push(Automation {
            id: s("id").unwrap_or_else(|| e.file_name().to_string_lossy().to_string()),
            name: s("name").unwrap_or_else(|| "Automation".into()),
            schedule: describe_rrule(&rrule),
            next_run: if active { next_run(&rrule, now) } else { None },
            target_thread: s("target_thread_id"),
            status,
            active,
        });
    }
    out.sort_by(|a, b| b.active.cmp(&a.active).then(a.name.cmp(&b.name)));
    out
}

#[derive(Default, Debug, PartialEq)]
struct Rule {
    freq: String,
    hours: Vec<u32>,
    minutes: Vec<u32>,
    days: Vec<u32>,
    interval: u32,
}

fn parse_rrule(rrule: &str) -> Rule {
    let mut r = Rule { interval: 1, ..Default::default() };
    let body = rrule.trim().trim_start_matches("RRULE:");
    for part in body.split(';') {
        let Some((k, v)) = part.split_once('=') else { continue };
        let nums = || -> Vec<u32> { v.split(',').filter_map(|x| x.trim().parse().ok()).collect() };
        match k.trim().to_ascii_uppercase().as_str() {
            "FREQ" => r.freq = v.trim().to_ascii_uppercase(),
            "BYHOUR" => r.hours = nums(),
            "BYMINUTE" => r.minutes = nums(),
            "INTERVAL" => r.interval = v.trim().parse().unwrap_or(1).max(1),
            "BYDAY" => {
                r.days = v
                    .split(',')
                    .filter_map(|d| {
                        let d = d.trim().to_ascii_uppercase();
                        ["MO", "TU", "WE", "TH", "FR", "SA", "SU"].iter().position(|x| d.ends_with(x)).map(|p| p as u32)
                    })
                    .collect()
            }
            _ => {}
        }
    }
    r.hours.sort_unstable();
    r.minutes.sort_unstable();
    r
}

/// Next time the rule fires after `now`, in the machine's local time.
pub fn next_run(rrule: &str, now: u64) -> Option<u64> {
    use chrono::{Datelike, Local, TimeZone, Timelike};
    let r = parse_rrule(rrule);
    if r.freq.is_empty() {
        return None;
    }
    let start = Local.timestamp_opt(now as i64, 0).single()?;
    let base = start.with_second(0)?;
    for step in 1..=(60 * 24 * 8) {
        let t = base + chrono::Duration::minutes(step);
        let minute_ok = if r.minutes.is_empty() { t.minute() == 0 || r.freq == "MINUTELY" } else { r.minutes.contains(&t.minute()) };
        let hour_ok = r.hours.is_empty() || r.hours.contains(&t.hour());
        let day_ok = r.days.is_empty() || r.days.contains(&t.weekday().num_days_from_monday());
        let freq_ok = match r.freq.as_str() {
            "MINUTELY" => r.interval <= 1 || (t.timestamp() / 60) % r.interval as i64 == 0,
            "HOURLY" => r.interval <= 1 || (t.timestamp() / 3600) % r.interval as i64 == 0,
            "DAILY" | "WEEKLY" => true,
            _ => return None,
        };
        if minute_ok && hour_ok && day_ok && freq_ok {
            return Some(t.timestamp() as u64);
        }
    }
    None
}

pub fn describe_rrule(rrule: &str) -> String {
    let r = parse_rrule(rrule);
    let fmt = |h: u32, m: u32| format!("{h}:{m:02}");
    match r.freq.as_str() {
        "DAILY" if !r.hours.is_empty() => {
            let minutes = if r.minutes.is_empty() { vec![0] } else { r.minutes.clone() };
            let n = r.hours.len() * minutes.len();
            let first = fmt(r.hours[0], minutes[0]);
            let last = fmt(*r.hours.last().unwrap(), *minutes.last().unwrap());
            if n == 1 {
                format!("every day at {first}")
            } else {
                format!("{n} times a day, {first} to {last}")
            }
        }
        "HOURLY" => {
            let m = r.minutes.first().copied().unwrap_or(0);
            if r.interval > 1 {
                format!("every {} hours at :{m:02}", r.interval)
            } else {
                format!("every hour at :{m:02}")
            }
        }
        "MINUTELY" => format!("every {} minutes", r.interval),
        "WEEKLY" => "every week".into(),
        "" => "on its own schedule".into(),
        _ => rrule.trim_start_matches("RRULE:").to_string(),
    }
}

/// Short local clock time for a unix timestamp: "10:00", or "Tue 9:00" if not today.
pub fn clock(ts: u64) -> String {
    use chrono::{Local, TimeZone};
    let Some(t) = Local.timestamp_opt(ts as i64, 0).single() else { return String::new() };
    let today = Local::now().date_naive();
    if t.date_naive() == today {
        t.format("%-H:%M").to_string()
    } else {
        t.format("%a %-H:%M").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_real_schedules() {
        assert_eq!(
            describe_rrule("RRULE:FREQ=DAILY;BYHOUR=9,13,16,18,19,21,22;BYMINUTE=0,30;BYSECOND=0"),
            "14 times a day, 9:00 to 22:30"
        );
        assert_eq!(describe_rrule("FREQ=HOURLY;BYMINUTE=18;BYSECOND=0"), "every hour at :18");
        assert_eq!(describe_rrule("FREQ=DAILY;BYHOUR=7;BYMINUTE=15"), "every day at 7:15");
    }

    #[test]
    fn next_run_lands_on_a_scheduled_minute() {
        use chrono::{Local, TimeZone, Timelike};
        let now = now_secs();
        let next = next_run("RRULE:FREQ=DAILY;BYHOUR=9,13,16,18,19,21,22;BYMINUTE=0,30;BYSECOND=0", now).unwrap();
        assert!(next > now && next <= now + 86_400);
        let t = Local.timestamp_opt(next as i64, 0).unwrap();
        assert!([9, 13, 16, 18, 19, 21, 22].contains(&t.hour()));
        assert!(t.minute() == 0 || t.minute() == 30);
        let hourly = next_run("FREQ=HOURLY;BYMINUTE=18", now).unwrap();
        assert_eq!(Local.timestamp_opt(hourly as i64, 0).unwrap().minute(), 18);
        assert!(hourly - now <= 3600);
    }

    #[test]
    fn reads_automations_and_falls_back_to_rollouts() {
        let root = std::env::temp_dir().join(format!("mom-codex-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let auto = root.join("automations").join("publishing");
        std::fs::create_dir_all(&auto).unwrap();
        std::fs::write(
            auto.join("automation.toml"),
            "version = 1\nid = \"publishing\"\nkind = \"heartbeat\"\nname = \"Daily standup\"\nprompt = \"secret instructions\"\nstatus = \"ACTIVE\"\nrrule = \"RRULE:FREQ=DAILY;BYHOUR=9;BYMINUTE=0\"\ntarget_thread_id = \"0199c2f4-1a2b-7c3d-8e4f-5a6b7c8d9e0f\"\n",
        )
        .unwrap();
        let day = root.join("sessions").join("2026").join("10").join("09");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(
            day.join("rollout-2026-10-09T18-38-01-0199c2f4-1a2b-7c3d-8e4f-5a6b7c8d9e0f_x.jsonl"),
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"0199c2f4-1a2b-7c3d-8e4f-5a6b7c8d9e0f\",\"timestamp\":\"2026-10-09T15:38:01.020Z\",\"cwd\":\"\\\\\\\\?\\\\C:\\\\Workspace\\\\storefront\"}}\n{\"more\":1}\n",
        )
        .unwrap();
        let s = state(&root);
        assert_eq!(s.automations.len(), 1);
        assert_eq!(s.automations[0].schedule, "every day at 9:00");
        assert!(s.automations[0].next_run.is_some());
        assert!(s.note.is_some(), "no database here, so it fell back to the logs");
        assert_eq!(s.threads.len(), 1);
        let t = &s.threads[0];
        assert_eq!(t.cwd.as_deref(), Some(r"C:\Workspace\storefront"));
        assert_eq!(t.project.as_deref(), Some("storefront"));
        assert!(t.working);
        assert_eq!(t.automation.as_ref().unwrap().name, "Daily standup");
        assert_eq!(t.link, "codex://threads/0199c2f4-1a2b-7c3d-8e4f-5a6b7c8d9e0f");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
