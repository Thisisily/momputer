//! Speed: what makes the PC slow, from the things people don't think to check. An app that lost
//! the graphics card (so video and scrolling are drawn by the CPU), a graphics driver that keeps
//! crashing, hardware errors (an overclock or memory setting that's too much), overclocking tools,
//! helpers that run all the time, and which programs are busy right now. Looks only; it changes
//! nothing.

use crate::procs::{Proc, ProcTable};
use crate::util::{human_age_long, human_bytes, now_secs, plural, run};
use serde::Serialize;
use std::collections::HashMap;
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
    /// itself can open). "open": open `value`, an ms-settings: link. "tab": show a part of the
    /// widget ("upkeep:apps").
    pub kind: &'static str,
    pub value: String,
    /// What to say once it's copied.
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
pub fn check(table: &ProcTable) -> Speed {
    let now = now_secs();
    let slow = slow_facts(now);
    let mut findings = Vec::new();
    findings.extend(lost_gpu(table, now));
    findings.extend(driver_trouble(&slow.events, now));
    findings.extend(hardware_errors(&slow.events, now));
    let trouble = findings.iter().any(|f| f.id == "gpu-driver" || f.id == "hardware-errors");
    findings.extend(tuners(&slow.tuners, table, slow.gpu.as_ref(), trouble));
    findings.extend(extras(table, now));
    if slow.power_saver {
        findings.push(Finding {
            id: "power-plan".into(),
            level: "tip",
            title: "Windows is on Power saver".into(),
            detail: "It holds the processor back to save power. On a desktop, Balanced is just as quiet and lets it speed up when it's needed.".into(),
            fix: Some(Fix { label: "Power settings".into(), kind: "open", value: "ms-settings:powersleep".into(), hint: None }),
        });
    }
    Speed { findings, busy: busy(table, 8), gpu: slow.gpu, at: now }
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
fn lost_gpu(table: &ProcTable, now: u64) -> Vec<Finding> {
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
            let fix = restart.map(|page| Fix {
                label: format!("Copy {page}"),
                kind: "copy",
                value: (*page).to_string(),
                hint: Some(format!("Paste it into {label}'s address bar. Your tabs come back.")),
            });
            let open = format!("{label} has been open {} with {} and {} of memory", human_age_long(open_for), processes(family.len()), human_bytes(mem));
            out.push(if software {
                Finding {
                    id: format!("no-gpu-{}", exe.trim_end_matches(".exe")),
                    level: "warn",
                    title: format!("{label} is drawing without your graphics card"),
                    detail: format!(
                        "{label} draws video, scrolling and animation on the processor instead, which is why videos stutter. It happens when the graphics driver crashes or updates while {label} is open, and lasts until {label} restarts. {}",
                        if restart.is_some() { format!("{open}; a restart brings every tab back.") } else { format!("Quit {label} and open it again.") }
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
        fix: Some(Fix { label: "Startup apps".into(), kind: "tab", value: "upkeep:apps".into(), hint: None }),
    })
}

// ------------------------------------------------------------------ busy right now

fn busy(table: &ProcTable, n: usize) -> Vec<Busy> {
    let mut by: HashMap<String, Busy> = HashMap::new();
    for p in table.procs.values() {
        if p.pid == 0 || p.pid == 4 || crate::health::not_a_program(&p.name) {
            continue;
        }
        let name = crate::health::program_name(&p.name);
        let e = by.entry(name.clone()).or_insert(Busy { name, cpu: 0.0, mem: 0, count: 0 });
        e.cpu += p.cpu;
        e.mem += p.mem;
        e.count += 1;
    }
    let mut v: Vec<Busy> = by.into_values().filter(|b| b.cpu >= 0.3).collect();
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
    let s = Slow { at: now, events: read_events(), tuners: installed_tuners(), gpu: read_gpu(), power_saver: power_saver() };
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

    #[test]
    fn busy_adds_up_each_program() {
        let mut a = fake(30, 1, "chrome.exe", "chrome", 1);
        a.cpu = 3.0;
        let mut b = fake(31, 30, "chrome.exe", "chrome --type=renderer", 2);
        b.cpu = 2.5;
        let mut c = fake(32, 1, "Idle", "", 1);
        c.cpu = 90.0;
        let table = ProcTable::from_procs(vec![a, b, c], 999);
        let v = busy(&table, 8);
        assert_eq!(v.len(), 1);
        assert_eq!((v[0].name.as_str(), v[0].count), ("Chrome", 2));
        assert!((v[0].cpu - 5.5).abs() < 0.01);
    }
}
