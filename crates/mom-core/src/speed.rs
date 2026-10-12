//! Speed: what makes the PC slow, from the things people don't think to check. An app that lost
//! the graphics card (so video and scrolling are drawn by the CPU), a graphics driver that keeps
//! crashing, hardware errors (an overclock or memory setting that's too much), overclocking tools,
//! helpers that run all the time, and which programs are busy right now. Looks only; it changes
//! nothing.

use crate::paths::Paths;
use crate::procs::{Proc, ProcTable};
use crate::programs::Program;
use crate::util::{human_age_long, human_bytes, now_secs, plural, run};
use serde::Serialize;

use std::sync::Mutex;
use std::time::Duration;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Speed {
    pub findings: Vec<Finding>,
    /// The busiest programs right now, every process of a program added together.
    pub busy: Vec<Busy>,
    pub gpu: Option<Gpu>,
    /// Unix seconds.
    pub at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub id: String,
    /// "warn", "tip" or "ok".
    pub level: &'static str,
    pub title: String,
    pub detail: String,
    pub fix: Option<Fix>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fix {
    pub label: String,
    /// "copy": put `value` on the clipboard (a page like chrome://restart, which only the browser
    /// itself can open). "open": open `value`, a Windows Settings page or a driver download page.
    /// "tab": show a part of the widget ("running:startup"). "program": restart the program
    /// whose id is `value` (from `programs::list`), after a "Sure?".
    pub kind: &'static str,
    pub value: String,
    /// What to say once it's copied, or what restarting it also ends.
    pub hint: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Busy {
    pub name: String,
    /// Share of the whole machine's CPU over the last few seconds, like Task Manager.
    pub cpu: f32,
    pub mem: u64,
    pub count: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gpu {
    pub name: String,
    /// Degrees Celsius.
    pub temp: Option<u32>,
    /// Percent busy.
    pub busy: Option<u32>,
    /// Watts.
    pub power_limit: Option<f32>,
    pub default_power_limit: Option<f32>,
}

/// Looks the machine over. The event logs, installed tools and graphics card are read at most
/// every five minutes (about a second's work); the rest comes from `table`.
pub fn check(table: &ProcTable, paths: &Paths) -> Speed {
    let now = now_secs();
    let slow = slow_facts(now);
    let programs = crate::programs::list(table);
    let mut findings = Vec::new();
    findings.extend(lost_gpu(table, &programs, now));
    findings.extend(busy_for_minutes(&programs, table.cpus));
    findings.extend(memory_full(&programs));
    findings.extend(driver_trouble(&slow.events, now).map(|mut f| {
        f.fix = slow.gpu.as_ref().and_then(|g| driver_page(&g.name)).or_else(|| driver_page(&slow.driver_vendor));
        f
    }));
    findings.extend(hardware_errors(&slow.events, now));
    let trouble = findings.iter().any(|f| f.id == "gpu-driver" || f.id == "hardware-errors");
    findings.extend(tuners(&slow.tuners, table, slow.gpu.as_ref(), trouble));
    findings.extend(extras(table, now));
    findings.extend(slow.extensions.as_ref().and_then(too_many_extensions));
    let starting = crate::health::startup_apps(paths).iter().filter(|a| a.enabled).count();
    if starting >= 12 {
        findings.push(Finding {
            id: "startup".into(),
            level: "tip",
            title: format!("{starting} apps start with Windows"),
            detail: "Each one slows signing in and keeps running in the background. Switch off the ones you don't need straight away; they still open when you start them.".into(),
            fix: Some(Fix { label: "Startup apps".into(), kind: "tab", value: "running:startup".into(), hint: None }),
        });
    }
    if slow.power_saver {
        findings.push(Finding {
            id: "power-plan".into(),
            level: "tip",
            title: "Windows is on Power saver".into(),
            detail: "It holds the processor back to save power. On a desktop, Balanced is just as quiet and lets it speed up when it's needed.".into(),
            fix: Some(Fix { label: "Power settings".into(), kind: "open", value: "ms-settings:powersleep".into(), hint: None }),
        });
    }
    Speed { findings, busy: busy(&programs, 6), gpu: slow.gpu, at: now }
}

// ------------------------------------------------------------------ apps that lost the graphics card

/// Browsers and apps built on Chromium, which draw with the graphics card: (process name, what
/// people call it, the page that restarts it with every tab back, for a browser).
const CHROMIUM: &[(&str, &str, Option<&str>)] = &[
    ("chrome.exe", "Chrome", Some("chrome://restart")),
    ("msedge.exe", "Edge", Some("edge://restart")),
    ("brave.exe", "Brave", Some("brave://restart")),
    ("vivaldi.exe", "Vivaldi", Some("vivaldi://restart")),
    ("opera.exe", "Opera", None),
    ("claude.exe", "Claude", None),
    ("chatgpt.exe", "ChatGPT", None),
    ("code.exe", "VS Code", None),
    ("cursor.exe", "Cursor", None),
    ("windsurf.exe", "Windsurf", None),
    ("discord.exe", "Discord", None),
    ("slack.exe", "Slack", None),
    ("spotify.exe", "Spotify", None),
    ("ms-teams.exe", "Teams", None),
    ("notion.exe", "Notion", None),
    ("figma.exe", "Figma", None),
    ("obsidian.exe", "Obsidian", None),
    ("steamwebhelper.exe", "Steam", None),
];

/// How many of an app's newest helpers to read. A tab or window is the usual helper, so the
/// newest few nearly always include one; reading every one would open a hundred processes.
const HELPERS_READ: usize = 4;

/// A browser open this long is worth restarting anyway.
const LONG_OPEN: u64 = 2 * 86_400;

/// Chromium apps drawing without the graphics card, and browsers open for days.
fn lost_gpu(table: &ProcTable, programs: &[Program], now: u64) -> Vec<Finding> {
    let mut out = Vec::new();
    for (exe, label, restart) in CHROMIUM {
        let all: Vec<&Proc> = table.procs.values().filter(|p| p.name.eq_ignore_ascii_case(exe)).collect();
        // The app itself: the one whose parent isn't the app. Robots' browsers don't count.
        let roots: Vec<&Proc> = all
            .iter()
            .copied()
            .filter(|p| table.parent(p.pid).is_none_or(|pp| !pp.name.eq_ignore_ascii_case(exe)))
            .filter(|p| !driven(p))
            .collect();
        for root in roots {
            let mut helpers: Vec<&Proc> = all.iter().copied().filter(|p| p.ppid == Some(root.pid) && p.start >= root.start).collect();
            if helpers.len() < 2 {
                continue;
            }
            helpers.sort_by(|a, b| b.start.cmp(&a.start));
            let read: Vec<u32> = helpers.iter().take(HELPERS_READ).map(|p| p.pid).collect();
            let lines = crate::procs::command_lines(table, &read);
            if lines.values().any(|l| driven_line(l)) {
                continue;
            }
            let software = lines.values().any(|l| draws_without_gpu(l));
            let family: Vec<&Proc> = all.iter().copied().filter(|p| p.pid == root.pid || table.ancestors(p.pid).contains(&root.pid)).collect();
            let mem: u64 = family.iter().map(|p| p.mem).sum();
            let open_for = now.saturating_sub(root.start);
            let long_open = restart.is_some() && open_for >= LONG_OPEN;
            if !software && !long_open {
                continue;
            }
            let fix = match restart {
                Some(page) => Some(Fix {
                    label: format!("Copy {page}"),
                    kind: "copy",
                    value: (*page).to_string(),
                    hint: Some(format!("Paste it into {label}'s address bar. Your tabs come back.")),
                }),
                None => restart_fix(programs, root.pid),
            };
            let open = format!("{label} has been open {} with {} and {} of memory", human_age_long(open_for), processes(family.len()), human_bytes(mem));
            out.push(if software {
                Finding {
                    id: format!("no-gpu-{}", exe.trim_end_matches(".exe")),
                    level: "warn",
                    title: format!("{label} is drawing without your graphics card"),
                    detail: format!(
                        "{label} draws video, scrolling and animation on the processor instead, which is why videos stutter. It happens when the graphics driver crashes or updates while {label} is open, and lasts until {label} restarts. {}",
                        if restart.is_some() { format!("{open}; a restart brings every tab back.") } else { format!("Restarting {label} fixes it.") }
                    ),
                    fix,
                }
            } else {
                Finding {
                    id: format!("long-open-{}", exe.trim_end_matches(".exe")),
                    level: "tip",
                    title: format!("{label} has been open for {}", human_age_long(open_for)),
                    detail: format!("{open}. Browsers get slower the longer they stay open; a restart frees the memory and brings every tab back."),
                    fix,
                }
            });
            break;
        }
    }
    out
}

/// The Restart button for the program a process belongs to, when mom can restart it, with what
/// runs inside it said up front.
fn restart_fix(programs: &[Program], pid: u32) -> Option<Fix> {
    let prog = programs.iter().find(|p| p.pids.contains(&pid) && p.can.contains(&"restart"))?;
    Some(Fix {
        label: format!("Restart {}", prog.name),
        kind: "program",
        value: prog.id.clone(),
        hint: prog.inside.as_ref().map(|i| format!("{i} run inside it, and stop when it closes.")),
    })
}

/// Programs that have kept the processor busy for minutes, not just this second.
fn busy_for_minutes(programs: &[Program], cpus: usize) -> Vec<Finding> {
    let mut out = Vec::new();
    for p in programs {
        if p.avg_secs < 60 {
            continue;
        }
        let cores = p.avg_cpu * cpus.max(1) as f32 / 100.0;
        let mins = human_age_long(p.avg_secs);
        match p.kind {
            "security" if cores >= 0.5 && p.name.contains("Defender") => out.push(Finding {
                id: "defender-busy".into(),
                level: "tip",
                title: "Microsoft Defender is scanning".into(),
                detail: format!(
                    "It's kept {cores:.1} processor cores busy for the last {mins}. A scan finishes by itself; if it's always busy while you build code, a Dev Drive (Settings > System > Storage) is scanned far less."
                ),
                fix: Some(Fix { label: "Windows Security".into(), kind: "open", value: "windowsdefender:".into(), hint: None }),
            }),
            "windows" if cores >= 0.5 && p.name.to_ascii_lowercase().contains("search") => out.push(Finding {
                id: "search-busy".into(),
                level: "tip",
                title: "Windows Search is indexing".into(),
                detail: format!("It's kept {cores:.1} processor cores busy for the last {mins}, reading your files so search is quick. It settles down by itself; you can leave big folders out of it."),
                fix: Some(Fix { label: "Search settings".into(), kind: "open", value: "ms-settings:cortana-windowssearch".into(), hint: None }),
            }),
            "app" | "background" if cores >= 0.75 => out.push(Finding {
                id: format!("busy-{}", p.id),
                level: if cores >= 1.5 { "warn" } else { "tip" },
                title: format!("{} is busy all the time", p.name),
                detail: format!(
                    "It's kept {cores:.1} processor cores busy for the last {mins}{}. If it isn't doing something for you right now, it's stuck; a restart usually settles it.",
                    p.inside.as_ref().map(|i| format!(" ({i} run inside it)")).unwrap_or_default()
                ),
                fix: if p.can.contains(&"restart") {
                    Some(Fix {
                        label: format!("Restart {}", p.name),
                        kind: "program",
                        value: p.id.clone(),
                        hint: p.inside.as_ref().map(|i| format!("{i} run inside it, and stop when it closes.")),
                    })
                } else {
                    Some(Fix { label: "Programs".into(), kind: "tab", value: "running:programs".into(), hint: None })
                },
            }),
            _ => {}
        }
    }
    out
}

/// Memory nearly full: Windows starts moving things to disk, and everything waits on it.
fn memory_full(programs: &[Program]) -> Option<Finding> {
    let m = os::memory()?;
    let used = m.total.saturating_sub(m.available);
    let commit_used = m.commit_limit.saturating_sub(m.commit_available);
    let ram = used as f64 / m.total.max(1) as f64;
    let commit = commit_used as f64 / m.commit_limit.max(1) as f64;
    if ram < 0.9 && commit < 0.9 {
        return None;
    }
    let mut top: Vec<&Program> = programs.iter().filter(|p| !matches!(p.kind, "windows" | "security")).collect();
    top.sort_by(|a, b| b.mem.cmp(&a.mem));
    let names: Vec<String> = top.iter().take(3).map(|p| format!("{} ({})", p.name, human_bytes(p.mem))).collect();
    Some(Finding {
        id: "memory-full".into(),
        level: "warn",
        title: if commit >= 0.9 && ram < 0.9 {
            "Windows is nearly out of memory to hand out".into()
        } else {
            format!("Memory is nearly full: {} of {}", human_bytes(used), human_bytes(m.total))
        },
        detail: format!(
            "When it fills up, Windows moves things to the disk and everything waits on it. The biggest users: {}. Closing one you're not using frees it straight away.",
            names.join(", ")
        ),
        fix: Some(Fix { label: "Programs".into(), kind: "tab", value: "running:programs".into(), hint: None }),
    })
}

/// Where to get a graphics driver, by who made the card.
fn driver_page(card: &str) -> Option<Fix> {
    let c = card.to_ascii_lowercase();
    let (who, url) = if c.contains("nvidia") || c.contains("geforce") || c.contains("rtx") || c.contains("gtx") || c.starts_with("nvlddmkm") {
        ("NVIDIA", "https://www.nvidia.com/en-us/drivers/")
    } else if c.contains("radeon") || c.contains("amd") || c.starts_with("amdkmdag") || c.starts_with("amdwddmg") {
        ("AMD", "https://www.amd.com/en/support/download/drivers.html")
    } else if c.contains("intel") || c.starts_with("igfx") || c.starts_with("igdkmd") {
        ("Intel", "https://www.intel.com/content/www/us/en/support/detect.html")
    } else {
        return None;
    };
    Some(Fix { label: format!("{who} drivers"), kind: "open", value: url.into(), hint: None })
}

/// A browser a robot is driving (an agent's Playwright, a test run): it turns the graphics card
/// off on purpose.
fn driven(p: &Proc) -> bool {
    let exe = p.exe.as_deref().unwrap_or("").to_ascii_lowercase();
    exe.contains("ms-playwright") || exe.contains("puppeteer") || driven_line(&p.cmdline())
}

fn driven_line(cmd: &str) -> bool {
    ["--headless", "--enable-automation", "--remote-debugging-pipe", "--disable-gpu ", "--disable-gpu\""].iter().any(|f| cmd.contains(f)) || cmd.ends_with("--disable-gpu")
}

/// A Chromium tab or window started without the graphics card: the browser hands
/// --disable-gpu-compositing to each new one once it has given up on the card.
fn draws_without_gpu(cmd: &str) -> bool {
    cmd.contains("--type=renderer") && cmd.contains("--disable-gpu-compositing")
}

// ------------------------------------------------------------------ the event log

#[derive(Clone, Debug, Default, PartialEq)]
struct Event {
    provider: String,
    id: u32,
    /// Unix seconds.
    at: u64,
    /// The driver a "driver installed" event is about, if any.
    service: Option<String>,
}

/// How far back the event log is read.
const LOOK_BACK: u64 = 14 * 86_400;

const GPU_DRIVERS: &[&str] = &["nvlddmkm", "amdkmdag", "amdwddmg", "igfx", "igfxn", "igdkmd64"];

fn read_events() -> Vec<Event> {
    let ms = LOOK_BACK * 1000;
    let drivers: Vec<String> = GPU_DRIVERS.iter().map(|d| format!("Provider[@Name='{d}']")).collect();
    let query = format!(
        "*[System[({} or (Provider[@Name='Display'] and EventID=4101) or Provider[@Name='Microsoft-Windows-WHEA-Logger'] or (Provider[@Name='Microsoft-Windows-UserPnp'] and EventID=20003)) and TimeCreated[timediff(@SystemTime) <= {ms}]]]",
        drivers.join(" or ")
    );
    let q = format!("/q:{query}");
    match run("wevtutil", &["qe", "System", &q, "/f:xml", "/c:400", "/rd:true"], Duration::from_secs(8)) {
        Ok(o) if o.ok() => parse_events(&o.text()),
        _ => vec![],
    }
}

/// Reads `wevtutil qe /f:xml` output: one <Event> after another.
fn parse_events(xml: &str) -> Vec<Event> {
    let attr = |s: &str, key: &str| -> Option<String> {
        let at = s.find(key)? + key.len();
        let quote = s[at..].chars().next()?;
        let rest = &s[at + 1..];
        Some(rest[..rest.find(quote)?].to_string())
    };
    let tag = |s: &str, name: &str| -> Option<String> {
        let open = s.find(&format!("<{name}"))?;
        let body = &s[open..];
        let start = body.find('>')? + 1;
        let end = body.find(&format!("</{name}>"))?;
        Some(body[start..end].trim().to_string())
    };
    xml.split("<Event ")
        .skip(1)
        .filter_map(|e| {
            let provider = attr(e, "Provider Name=")?;
            let id = tag(e, "EventID")?.parse().ok()?;
            let at = parse_time(&attr(e, "SystemTime=")?)?;
            let service = e.find("<Data Name='DriverName'>").or_else(|| e.find("<Data Name='ServiceName'>")).and_then(|i| {
                let rest = &e[i..];
                let start = rest.find('>')? + 1;
                Some(rest[start..rest.find("</Data>")?].trim().to_ascii_lowercase())
            });
            let service = service.or_else(|| {
                let lower = e.to_ascii_lowercase();
                GPU_DRIVERS.iter().find(|d| lower.contains(*d)).map(|d| (*d).to_string())
            });
            Some(Event { provider, id, at, service })
        })
        .collect()
}

/// "2026-10-05T03:39:17.1234567Z" -> unix seconds.
fn parse_time(s: &str) -> Option<u64> {
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    // Days since 1970-01-01, for the proleptic Gregorian calendar.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + hh * 3600 + mm * 60 + ss).ok()
}

/// "1 process", "79 processes".
fn processes(n: usize) -> String {
    if n == 1 { "1 process".into() } else { format!("{n} processes") }
}

fn when(at: u64, now: u64) -> String {
    let ago = now.saturating_sub(at);
    if ago < 86_400 {
        format!("{} ago", human_age_long(ago.max(60)))
    } else {
        format!("{} days ago", ago / 86_400)
    }
}

/// The graphics driver crashing or being reset, and updates that cut every app off from the card.
fn driver_trouble(events: &[Event], now: u64) -> Option<Finding> {
    let resets: Vec<&Event> = events
        .iter()
        .filter(|e| GPU_DRIVERS.contains(&e.provider.to_ascii_lowercase().as_str()) || (e.provider == "Display" && e.id == 4101))
        .collect();
    let updates: Vec<&Event> = events
        .iter()
        .filter(|e| e.provider == "Microsoft-Windows-UserPnp" && e.service.as_deref().is_some_and(|s| GPU_DRIVERS.iter().any(|d| s.starts_with(d))))
        .collect();
    let latest_update = updates.iter().map(|e| e.at).max();
    if resets.is_empty() {
        return latest_update.filter(|t| now.saturating_sub(*t) < 3 * 86_400).map(|t| Finding {
            id: "gpu-driver".into(),
            level: "tip",
            title: format!("Your graphics driver was updated {}", when(t, now)),
            detail: "Apps that were open during the update can lose the graphics card until they restart. If one stutters, restart it.".into(),
            fix: None,
        });
    }
    let last = resets.iter().map(|e| e.at).max().unwrap_or(now);
    let first = resets.iter().map(|e| e.at).min().unwrap_or(now);
    let n = resets.len() as u64;
    let times = if n == 1 {
        format!("It happened {}", when(last, now))
    } else if when(first, now) == when(last, now) {
        format!("All {n} were {}", when(last, now))
    } else {
        format!("The last was {}, the first {}", when(last, now), when(first, now))
    };
    let updated = latest_update.filter(|t| *t > last).map(|t| format!(" The driver was also updated {}.", when(t, now))).unwrap_or_default();
    Some(Finding {
        id: "gpu-driver".into(),
        level: if n >= 3 || now.saturating_sub(last) < 3 * 86_400 { "warn" } else { "tip" },
        title: format!("Your graphics driver crashed {} in two weeks", if n == 1 { "once".to_string() } else { format!("{n} times") }),
        detail: format!(
            "{times}.{updated} Each time, Windows restarts it, and apps that were open can lose the graphics card until they restart. An overclock on the graphics card is the usual cause; otherwise, reinstall the driver from the maker's site."
        ),
        fix: None,
    })
}

/// WHEA: the processor or memory reporting errors. Almost always an overclock, undervolt or
/// memory speed (XMP/EXPO) set higher than the parts handle.
fn hardware_errors(events: &[Event], now: u64) -> Option<Finding> {
    let whea: Vec<&Event> = events.iter().filter(|e| e.provider == "Microsoft-Windows-WHEA-Logger").collect();
    if whea.is_empty() {
        return None;
    }
    let fatal = whea.iter().filter(|e| matches!(e.id, 18 | 20 | 46)).count() as u64;
    let n = whea.len() as u64;
    let last = whea.iter().map(|e| e.at).max().unwrap_or(now);
    Some(Finding {
        id: "hardware-errors".into(),
        level: if fatal > 0 || n >= 3 { "warn" } else { "tip" },
        title: if fatal > 0 {
            format!("Your PC had {} it couldn't recover from", plural(fatal, "hardware error"))
        } else {
            format!("Your processor or memory reported {}", plural(n, "hardware error"))
        },
        detail: format!(
            "{} in two weeks, the last {}. That's almost always an overclock, an undervolt, or a memory speed (XMP or EXPO in the BIOS) set higher than the parts can take. Setting them back to default stops it.",
            plural(n, "error"),
            when(last, now)
        ),
        fix: None,
    })
}

// ------------------------------------------------------------------ overclocking tools

/// Overclocking and tuning tools: (a word in their name in the uninstall list, their processes).
const TUNERS: &[(&str, &[&str])] = &[
    ("GPU Tweak", &["gputweakiii", "gputweakii", "gputweak"]),
    ("Afterburner", &["msiafterburner"]),
    ("Ryzen Master", &["amdryzenmaster", "ryzenmaster"]),
    ("AI Suite", &["aisuite3", "aisuite"]),
    ("Precision X", &["precisionx_x64", "precisionx1", "precisionx"]),
    ("Extreme Tuning", &["xtu3service", "perftune"]),
    ("ThrottleStop", &["throttlestop"]),
    ("AORUS ENGINE", &["aorusengine"]),
    ("FireStorm", &["firestorm"]),
    ("Thunder Master", &["thundermaster"]),
];

fn stem(name: &str) -> String {
    name.to_ascii_lowercase().trim_end_matches(".exe").to_string()
}

fn tuners(installed: &[String], table: &ProcTable, gpu: Option<&Gpu>, trouble: bool) -> Option<Finding> {
    if installed.is_empty() {
        return None;
    }
    let running: Vec<&str> = TUNERS
        .iter()
        .filter(|(word, procs)| installed.iter().any(|n| n.contains(word)) && table.procs.values().any(|p| procs.contains(&stem(&p.name).as_str())))
        .map(|(word, _)| *word)
        .collect();
    let names = installed.join(" and ");
    let raised = gpu.and_then(|g| Some((g.power_limit?, g.default_power_limit?))).filter(|(now, default)| now > &(default + 1.0));
    let card = match (gpu, raised) {
        (Some(g), Some((now, default))) => format!(" Your {}'s power limit is raised to {now:.0} W (it comes set to {default:.0} W).", g.name),
        (Some(g), None) if g.power_limit.is_some() => format!(" Your {} runs at its standard power limit ({:.0} W).", g.name, g.power_limit.unwrap_or_default()),
        _ => String::new(),
    };
    let (level, title, detail) = if running.is_empty() && raised.is_none() {
        (
            if trouble { "tip" } else { "ok" },
            format!("{names} {} installed, not running", if installed.len() == 1 { "is" } else { "are" }),
            format!(
                "So nothing is overclocked right now.{card}{}",
                if trouble { " If you overclocked with it when the crashes above happened, that's the likely cause: leave it at default." } else { "" }
            ),
        )
    } else {
        (
            if trouble { "warn" } else { "tip" },
            format!("{} {} running", if running.is_empty() { names.clone() } else { running.join(" and ") }, if running.len() > 1 { "are" } else { "is" }),
            format!(
                "It can overclock your hardware every time it starts.{card} {}",
                if trouble {
                    "With the crashes above, set it back to default (most have a Reset or Default button) and see if they stop."
                } else {
                    "If apps crash, video stutters or the screen goes black for a moment, set it back to default."
                }
            ),
        )
    };
    Some(Finding { id: "tuners".into(), level, title, detail, fix: None })
}

// ------------------------------------------------------------------ helpers that run all the time

/// Helpers that come with hardware and utilities and run all day: (who, what they do, process
/// names without .exe, lowercase).
const EXTRAS: &[(&str, &str, &[&str])] = &[
    ("IObit Uninstaller", "watches every program you install", &["uninstallmonitor", "iobituninstaler", "iobitunsvr"]),
    (
        "Razer Synapse and Cortex",
        "lighting, mouse settings and a game booster",
        &[
            "razerappengine", "razer central", "razercentralservice", "razercortex", "cortexlauncherservice", "gamemanagerservice3",
            "razer_elevation_service", "rzactionsvc", "razerwdl", "rzchromasdkservice", "rzsdkserver", "rzsdkservice", "chromastreamserver",
        ],
    ),
    (
        "Adobe Creative Cloud",
        "keeps Creative Cloud apps and fonts in sync",
        &["adobe desktop service", "creative cloud", "ccxprocess", "coresync", "adobeipcbroker", "adobeupdateservice", "agsservice", "agmservice", "cclibrary"],
    ),
    (
        "ASUS Armoury Crate and Aura",
        "lighting and fan control",
        &["armourycrate.service", "armourycrate.usersessionhelper", "armourycratecontrolinterface", "asuscertservice", "lightingservice", "atkexcomsvc", "asus_framework", "ledcontrolservice"],
    ),
    ("Logitech G HUB", "mouse, keyboard and lighting settings", &["lghub", "lghub_agent", "lghub_updater", "lghub_system_tray"]),
    ("Corsair iCUE", "lighting and fan control", &["icue", "corsair.service", "corsairdeviceservice", "icue.service"]),
    ("NZXT CAM", "lighting and fan control", &["nzxt cam", "nzxtcam"]),
    ("SteelSeries GG", "device settings", &["steelseriesgg", "steelseriesengine", "steelseriesggclient"]),
    ("MSI Center", "lighting and fan control", &["msi.centralserver", "msi_central_service", "mysticlight_service", "msi.centralserver.service"]),
    ("Wallpaper Engine", "moving wallpapers", &["wallpaper32", "wallpaper64", "webwallpaper32"]),
];

/// Worth mentioning: a quarter of a core all day long (on average since it started), or this
/// much memory.
const EXTRA_CORES: f64 = 0.25;
const EXTRA_MEM: u64 = 400 * 1024 * 1024;

fn extras(table: &ProcTable, now: u64) -> Option<Finding> {
    let mut lines = Vec::new();
    for (who, what, names) in EXTRAS {
        let procs: Vec<&Proc> = table.procs.values().filter(|p| names.contains(&stem(&p.name).as_str())).collect();
        if procs.is_empty() {
            continue;
        }
        // Average since each started: a helper that's busy all day, not one busy this second.
        let cores: f64 = procs.iter().map(|p| p.cpu_ms as f64 / 1000.0 / now.saturating_sub(p.start).max(60) as f64).sum();
        let mem: u64 = procs.iter().map(|p| p.mem).sum();
        if cores < EXTRA_CORES && mem < EXTRA_MEM {
            continue;
        }
        let cpu = if cores >= 0.05 { format!(", {:.0}% of a processor core all day", cores * 100.0) } else { String::new() };
        lines.push((cores, format!("{who} ({what}): {}, {}{cpu}", processes(procs.len()), human_bytes(mem))));
    }
    if lines.is_empty() {
        return None;
    }
    lines.sort_by(|a, b| b.0.total_cmp(&a.0));
    let text: Vec<String> = lines.into_iter().map(|(_, l)| l).collect();
    Some(Finding {
        id: "extras".into(),
        level: "tip",
        title: format!("{} running all the time", plural(text.len() as u64, "helper")),
        detail: format!("{}. If you don't need them, stop them starting with Windows.", text.join(". ")),
        fix: Some(Fix { label: "Startup apps".into(), kind: "tab", value: "running:startup".into(), hint: None }),
    })
}

// ------------------------------------------------------------------ busy right now

fn busy(programs: &[Program], n: usize) -> Vec<Busy> {
    let mut v: Vec<Busy> = programs
        .iter()
        .filter(|p| p.cpu >= 0.3 && p.name != "System Idle Process")
        .map(|p| Busy { name: p.name.clone(), cpu: p.cpu, mem: p.mem, count: p.count })
        .collect();
    v.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then_with(|| a.name.cmp(&b.name)));
    v.truncate(n);
    v
}

// ------------------------------------------------------------------ the slow facts, kept a while

#[derive(Clone, Default)]
struct Slow {
    at: u64,
    events: Vec<Event>,
    /// Tuning tools in the uninstall list, by their usual name.
    tuners: Vec<String>,
    gpu: Option<Gpu>,
    power_saver: bool,
    /// Chrome's busiest profile: (its name, extensions on, the ad blockers among them).
    extensions: Option<(String, usize, Vec<String>)>,
    /// The graphics driver named in the event log ("nvlddmkm"), for when there's no card info.
    driver_vendor: String,
}

static SLOW: Mutex<Option<Slow>> = Mutex::new(None);

/// How long the event log, the installed tools and the graphics card are trusted.
const SLOW_FOR: u64 = 300;

/// Forgets the slow facts, so the next look reads the event log and the rest again.
pub fn forget() {
    *SLOW.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

fn slow_facts(now: u64) -> Slow {
    if let Some(s) = SLOW.lock().unwrap_or_else(|e| e.into_inner()).as_ref().filter(|s| now.saturating_sub(s.at) < SLOW_FOR) {
        return s.clone();
    }
    let events = read_events();
    let driver_vendor = events.iter().find(|e| GPU_DRIVERS.contains(&e.provider.to_ascii_lowercase().as_str())).map(|e| e.provider.to_ascii_lowercase()).unwrap_or_default();
    let s = Slow { at: now, events, tuners: installed_tuners(), gpu: read_gpu(), power_saver: power_saver(), extensions: chrome_extensions(), driver_vendor };
    *SLOW.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.clone());
    s
}

fn installed_tuners() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for app in crate::installed::programs() {
        if let Some((word, _)) = TUNERS.iter().find(|(word, _)| app.name.contains(word)) {
            // "GPU Tweak III Install" and "GPU Tweak III" are one tool.
            let name = app.name.trim_end_matches(" Install").trim().to_string();
            if !out.iter().any(|n| n.contains(word)) {
                out.push(name);
            }
        }
    }
    out
}

// ------------------------------------------------------------------ browser extensions

/// Chrome's profile with the most extensions switched on: (its name, how many, the ad blockers
/// among them, by name). Every extension is another program running inside every tab.
fn chrome_extensions() -> Option<(String, usize, Vec<String>)> {
    let data = std::env::var("LOCALAPPDATA").ok()?;
    let root = std::path::Path::new(&data).join("Google").join("Chrome").join("User Data");
    let state: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("Local State")).ok()?).ok()?;
    let profiles = state.pointer("/profile/info_cache")?.as_object()?;
    let mut best: Option<(String, usize, Vec<String>)> = None;
    for (dir, info) in profiles {
        let prefs = root.join(dir).join("Secure Preferences");
        let Ok(bytes) = std::fs::read(&prefs) else { continue };
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else { continue };
        let Some(settings) = json.pointer("/extensions/settings").and_then(|v| v.as_object()) else { continue };
        let mut on = Vec::new();
        for (id, e) in settings {
            // Web Store (1) and unpacked (4) extensions; Chrome's own components don't count.
            let location = e.get("location").and_then(|v| v.as_i64()).unwrap_or(0);
            let disabled = e.get("disable_reasons").is_some_and(|r| r.as_array().is_some_and(|a| !a.is_empty()) || r.as_i64().is_some_and(|n| n != 0))
                || e.get("state").and_then(|v| v.as_i64()) == Some(0);
            if matches!(location, 1 | 4) && !disabled {
                on.push(extension_name(&root.join(dir).join("Extensions").join(id)).unwrap_or_default());
            }
        }
        let blockers: Vec<String> = on
            .iter()
            .filter(|n| {
                let l = n.to_ascii_lowercase();
                l.contains("adblock") || l.contains("ad block") || l.contains("ublock") || l.contains("adguard") || l.contains("ghostery")
            })
            .cloned()
            .collect();
        let name = info.get("name").and_then(|v| v.as_str()).unwrap_or(dir).to_string();
        if best.as_ref().is_none_or(|b| on.len() > b.1) {
            best = Some((name, on.len(), blockers));
        }
    }
    best
}

/// An extension's name from its manifest, with "__MSG_appName__" looked up in English.
fn extension_name(dir: &std::path::Path) -> Option<String> {
    let version = std::fs::read_dir(dir).ok()?.filter_map(Result::ok).map(|e| e.path()).find(|p| p.is_dir())?;
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(version.join("manifest.json")).ok()?).ok()?;
    let name = manifest.get("name")?.as_str()?.to_string();
    let Some(key) = name.strip_prefix("__MSG_").and_then(|n| n.strip_suffix("__")) else { return Some(name) };
    for locale in ["en", "en_US", "en_GB"] {
        if let Ok(bytes) = std::fs::read(version.join("_locales").join(locale).join("messages.json")) {
            if let Ok(m) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if let Some(obj) = m.as_object() {
                    if let Some((_, v)) = obj.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
                        return v.get("message").and_then(|s| s.as_str()).map(str::to_string);
                    }
                }
            }
        }
    }
    None
}

fn too_many_extensions(e: &(String, usize, Vec<String>)) -> Option<Finding> {
    let (profile, n, blockers) = e;
    if *n < 20 && blockers.len() < 2 {
        return None;
    }
    let doubled = if blockers.len() >= 2 { format!(" {} of them block ads ({}); one does the job.", blockers.len(), blockers.join(", ")) } else { String::new() };
    Some(Finding {
        id: "chrome-extensions".into(),
        level: "tip",
        title: format!("{n} Chrome extensions are on in {profile}"),
        detail: format!("Each one runs in the background and inside the pages you open, so every tab does more work.{doubled} Switch off the ones you don't use."),
        fix: Some(Fix {
            label: "Copy chrome://extensions".into(),
            kind: "copy",
            value: "chrome://extensions".into(),
            hint: Some("Paste it into Chrome's address bar.".into()),
        }),
    })
}

/// NVIDIA's own tool, when it's there. Other cards: nothing (no harm done).
fn read_gpu() -> Option<Gpu> {
    let o = run(
        "nvidia-smi",
        &["--query-gpu=name,temperature.gpu,utilization.gpu,power.limit,power.default_limit", "--format=csv,noheader,nounits"],
        Duration::from_secs(5),
    )
    .ok()
    .filter(|o| o.ok())?;
    parse_gpu(&o.text())
}

fn parse_gpu(text: &str) -> Option<Gpu> {
    let line = text.lines().find(|l| !l.trim().is_empty())?;
    let f: Vec<&str> = line.split(',').map(str::trim).collect();
    let name = f.first()?.trim_start_matches("NVIDIA ").trim_start_matches("GeForce ").to_string();
    Some(Gpu {
        name,
        temp: f.get(1).and_then(|v| v.parse().ok()),
        busy: f.get(2).and_then(|v| v.parse().ok()),
        power_limit: f.get(3).and_then(|v| v.parse().ok()),
        default_power_limit: f.get(4).and_then(|v| v.parse().ok()),
    })
}

/// Windows' own Power saver plan is on.
fn power_saver() -> bool {
    run("powercfg", &["/getactivescheme"], Duration::from_secs(4))
        .map(|o| o.text().to_ascii_lowercase().contains("a1841308-3541-4fab-bc81-f71556f20b4a"))
        .unwrap_or(false)
}

struct Mem {
    total: u64,
    available: u64,
    commit_limit: u64,
    commit_available: u64,
}

#[cfg(windows)]
mod os {
    pub(super) fn memory() -> Option<super::Mem> {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        // SAFETY: MEMORYSTATUSEX is plain data; dwLength is set before the call.
        unsafe {
            let mut m: MEMORYSTATUSEX = std::mem::zeroed();
            m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
            (GlobalMemoryStatusEx(&mut m) != 0).then_some(super::Mem {
                total: m.ullTotalPhys,
                available: m.ullAvailPhys,
                commit_limit: m.ullTotalPageFile,
                commit_available: m.ullAvailPageFile,
            })
        }
    }
}

#[cfg(not(windows))]
mod os {
    pub(super) fn memory() -> Option<super::Mem> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procs::fake;

    #[test]
    fn reads_the_event_log() {
        let xml = r#"<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='nvlddmkm'/><EventID Qualifiers='49322'>153</EventID><TimeCreated SystemTime='2026-10-05T03:39:17.5000000Z'/></System></Event>
<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='Microsoft-Windows-UserPnp' Guid='{96f4a050}'/><EventID>20003</EventID><TimeCreated SystemTime='2026-10-12T01:28:12.0Z'/></System><UserData><InstallDeviceID><DriverName>nvlddmkm</DriverName></InstallDeviceID></UserData></Event>
<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='Microsoft-Windows-WHEA-Logger'/><EventID>19</EventID><TimeCreated SystemTime='2026-10-11T10:00:00Z'/></System></Event>"#;
        let e = parse_events(xml);
        assert_eq!(e.len(), 3);
        assert_eq!((e[0].provider.as_str(), e[0].id), ("nvlddmkm", 153));
        assert_eq!(e[0].at, parse_time("2026-10-05T03:39:17Z").unwrap());
        assert_eq!(e[1].service.as_deref(), Some("nvlddmkm"));
        assert_eq!(e[2].id, 19);
        assert_eq!(parse_time("1970-01-02T00:00:00Z"), Some(86_400));
        assert_eq!(parse_time("2000-03-01T00:00:00Z"), Some(951_868_800));
    }

    #[test]
    fn driver_crashes_and_hardware_errors_are_told_plainly() {
        let now = parse_time("2026-10-12T20:00:00Z").unwrap();
        let ev = |provider: &str, id: u32, at: &str| Event { provider: provider.into(), id, at: parse_time(at).unwrap(), service: None };
        let resets: Vec<Event> = (1..=5).map(|i| ev("nvlddmkm", 153, &format!("2026-10-05T03:3{i}:00Z"))).collect();
        let f = driver_trouble(&resets, now).unwrap();
        assert_eq!((f.level, f.title.as_str()), ("warn", "Your graphics driver crashed 5 times in two weeks"));
        assert!(f.detail.starts_with("All 5 were 7 days ago."), "{}", f.detail);
        assert!(driver_trouble(&[], now).is_none());
        let update = Event { provider: "Microsoft-Windows-UserPnp".into(), id: 20003, at: now - 3600, service: Some("nvlddmkm".into()) };
        assert_eq!(driver_trouble(&[update], now).unwrap().level, "tip");

        let one = hardware_errors(&[ev("Microsoft-Windows-WHEA-Logger", 19, "2026-10-11T10:00:00Z")], now).unwrap();
        assert_eq!(one.level, "tip");
        let fatal = hardware_errors(&[ev("Microsoft-Windows-WHEA-Logger", 18, "2026-10-11T10:00:00Z")], now).unwrap();
        assert_eq!(fatal.level, "warn");
        assert!(fatal.title.contains("couldn't recover"));
    }

    #[test]
    fn tells_a_browser_that_lost_the_graphics_card_from_a_robot() {
        assert!(draws_without_gpu(r#""C:\Chrome\chrome.exe" --type=renderer --disable-gpu-compositing --lang=en-US"#));
        assert!(!draws_without_gpu(r#""C:\Chrome\chrome.exe" --type=renderer --lang=en-US"#));
        assert!(!draws_without_gpu(r#""C:\Chrome\chrome.exe" --type=gpu-process --gpu-recent-crash-count=1"#));
        assert!(driven_line(r#"chrome.exe --headless=new --remote-debugging-port=9222"#));
        assert!(driven_line(r#"chrome.exe --enable-automation"#));
        assert!(!driven_line(r#"chrome.exe --type=renderer --disable-gpu-compositing"#));
        let mut robot = fake(10, 1, "chrome.exe", "chrome.exe", 100);
        robot.exe = Some(r"C:\Users\me\AppData\Local\ms-playwright\chromium-1200\chrome-win\chrome.exe".into());
        assert!(driven(&robot));
    }

    #[test]
    fn tuners_say_whether_they_overclock_now() {
        let table = ProcTable::from_procs(vec![fake(1, 0, "explorer.exe", "explorer", 1)], 999);
        let gpu = Gpu { name: "RTX 2070 SUPER".into(), temp: Some(41), busy: Some(3), power_limit: Some(215.0), default_power_limit: Some(215.0) };
        let idle = tuners(&["GPU Tweak III".into()], &table, Some(&gpu), false).unwrap();
        assert_eq!(idle.level, "ok");
        assert_eq!(idle.title, "GPU Tweak III is installed, not running");
        assert!(idle.detail.contains("standard power limit (215 W)"), "{}", idle.detail);
        assert_eq!(tuners(&["GPU Tweak III".into()], &table, Some(&gpu), true).unwrap().level, "tip");
        let raised = Gpu { power_limit: Some(240.0), ..gpu.clone() };
        let r = tuners(&["GPU Tweak III".into()], &table, Some(&raised), true).unwrap();
        assert_eq!(r.level, "warn");
        assert!(r.detail.contains("raised to 240 W"), "{}", r.detail);
        let running = ProcTable::from_procs(vec![fake(5, 0, "GPUTweakIII.exe", "GPUTweakIII", 1)], 999);
        assert_eq!(tuners(&["GPU Tweak III".into()], &running, Some(&gpu), false).unwrap().title, "GPU Tweak is running");
        assert!(tuners(&[], &table, Some(&gpu), true).is_none());
        assert_eq!(parse_gpu("NVIDIA GeForce RTX 2070 SUPER, 41, 30, 215.00, 215.00\n").unwrap().name, "RTX 2070 SUPER");
    }

    #[test]
    fn helpers_that_run_all_day_are_counted_by_their_average() {
        let now = 1_000_000;
        let mut monitor = fake(20, 1, "UninstallMonitor.exe", "UninstallMonitor", now - 100_000);
        monitor.cpu_ms = 27_000_000; // 0.27 of a core for 100,000 s
        let mut quiet = fake(21, 1, "lghub_agent.exe", "lghub_agent", now - 100_000);
        quiet.cpu_ms = 1_000;
        quiet.mem = 50 * 1024 * 1024;
        let table = ProcTable::from_procs(vec![monitor, quiet], 999);
        let f = extras(&table, now).unwrap();
        assert_eq!(f.title, "1 helper running all the time");
        assert!(f.detail.starts_with("IObit Uninstaller (watches every program you install): 1 process"), "{}", f.detail);
        assert!(f.detail.contains("27% of a processor core all day"), "{}", f.detail);
    }

    fn program(name: &str, kind: &'static str, avg_cpu: f32, can: Vec<&'static str>) -> Program {
        Program {
            id: format!("app:{}", name.to_lowercase()),
            name: name.into(),
            exe: None,
            cpu: avg_cpu,
            avg_cpu,
            avg_secs: 240,
            mem: 1 << 30,
            gpu: None,
            count: 3,
            windows: 1,
            kind,
            started: 0,
            inside: None,
            can,
            pids: vec![77],
        }
    }

    #[test]
    fn programs_busy_for_minutes_get_a_restart() {
        // 16 threads: 10% of the machine is 1.6 cores.
        let stuck = program("Claude", "app", 10.0, vec!["restart", "quit", "end"]);
        let calm = program("Spotify", "app", 1.0, vec!["restart", "quit", "end"]);
        let scan = program("Microsoft Defender", "security", 4.0, vec![]);
        let f = busy_for_minutes(&[stuck, calm, scan], 16);
        assert_eq!(f.len(), 2, "{f:#?}");
        assert_eq!((f[0].level, f[0].title.as_str()), ("warn", "Claude is busy all the time"));
        assert!(f[0].detail.contains("1.6 processor cores") && f[0].detail.contains("4 minutes"), "{}", f[0].detail);
        let fix = f[0].fix.as_ref().unwrap();
        assert_eq!((fix.kind, fix.value.as_str(), fix.label.as_str()), ("program", "app:claude", "Restart Claude"));
        assert_eq!(f[1].id, "defender-busy");
        let mut inside = program("ChatGPT", "app", 1.0, vec!["restart"]);
        inside.inside = Some("3 Codex sessions".into());
        let fix = restart_fix(&[inside], 77).unwrap();
        assert_eq!(fix.hint.as_deref(), Some("3 Codex sessions run inside it, and stop when it closes."));
    }

    #[test]
    fn extensions_and_drivers() {
        assert!(too_many_extensions(&("Default".into(), 12, vec![])).is_none());
        let f = too_many_extensions(&("Default".into(), 38, vec!["AdBlock".into(), "AdBlocker Ultimate".into()])).unwrap();
        assert_eq!(f.title, "38 Chrome extensions are on in Default");
        assert!(f.detail.contains("2 of them block ads (AdBlock, AdBlocker Ultimate)"), "{}", f.detail);
        assert_eq!(driver_page("RTX 2070 SUPER").unwrap().label, "NVIDIA drivers");
        assert_eq!(driver_page("amdkmdag").unwrap().label, "AMD drivers");
        assert!(driver_page("Mystery card").is_none());
    }
}
