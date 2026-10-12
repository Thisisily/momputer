//! The rest of the Windows checkup: what the PC is made of and whether it runs at its best
//! (memory at its rated speed, Windows on a fast disk), whether it's protected (which antivirus
//! is really on, firewall, memory integrity) and kept up to date, and registry entries that point
//! at things that are gone. Looks only.
//!
//! The hardware and security facts come from Windows' own management data (one PowerShell call,
//! several seconds), so they're gathered on a thread of their own and kept for half an hour.

use crate::speed::Fix;
use crate::util::{human_bytes, now_secs, run};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct System {
    /// What the PC is: ("Memory", "64 GB: 4 × 16 GB at 2666 MT/s"), in order.
    pub pc: Vec<(String, String)>,
    pub checks: Vec<Check>,
    /// Unix seconds.
    pub at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub id: String,
    /// "Security", "Updates", "Hardware", "Performance" or "Registry".
    pub area: &'static str,
    /// "warn", "tip" or "ok".
    pub level: &'static str,
    pub title: String,
    pub detail: String,
    pub fix: Option<Fix>,
}

fn check(id: &str, area: &'static str, level: &'static str, title: String, detail: String, fix: Option<Fix>) -> Check {
    Check { id: id.into(), area, level, title, detail, fix }
}

fn open(label: &str, url: &str) -> Option<Fix> {
    Some(Fix { label: label.into(), kind: "open", value: url.into(), hint: None })
}

// ------------------------------------------------------------------ gathering, kept a while

static CACHE: Mutex<Option<System>> = Mutex::new(None);
static BUSY: Mutex<bool> = Mutex::new(false);
const KEEP_FOR: u64 = 1800;

/// The checkup: from the last half hour, or gathered now (several seconds).
pub fn get(fresh: bool) -> System {
    if !fresh {
        if let Some(s) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().filter(|s| now_secs().saturating_sub(s.at) < KEEP_FOR) {
            return s.clone();
        }
    }
    let s = gather();
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.clone());
    s
}

/// What's known already, without waiting; starts gathering on a thread of its own if it's stale.
pub fn cached() -> Option<System> {
    let have = CACHE.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let stale = have.as_ref().is_none_or(|s| now_secs().saturating_sub(s.at) >= KEEP_FOR);
    if stale {
        let mut busy = BUSY.lock().unwrap_or_else(|e| e.into_inner());
        if !*busy {
            *busy = true;
            std::thread::spawn(|| {
                let s = gather();
                *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some(s);
                *BUSY.lock().unwrap_or_else(|e| e.into_inner()) = false;
            });
        }
    }
    have
}

/// One PowerShell call for everything Windows keeps in its management data.
const SCRIPT: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
$o = [ordered]@{}
$d = { param($x) if ($x) { ([datetime]$x).ToString('o') } }
$o.mem = @(Get-CimInstance Win32_PhysicalMemory | ForEach-Object { @{ cap = [int64]$_.Capacity; speed = $_.Speed; conf = $_.ConfiguredClockSpeed; slot = "$($_.DeviceLocator)"; part = "$($_.PartNumber)".Trim(); maker = "$($_.Manufacturer)".Trim() } })
$o.disks = @(Get-CimInstance -Namespace root/Microsoft/Windows/Storage -ClassName MSFT_PhysicalDisk | ForEach-Object { @{ id = "$($_.DeviceId)"; name = "$($_.FriendlyName)".Trim(); media = $_.MediaType; bus = $_.BusType; health = $_.HealthStatus; size = [int64]$_.Size } })
$o.sysdisk = "$((Get-CimInstance -Namespace root/Microsoft/Windows/Storage -ClassName MSFT_Partition -Filter "DriveLetter='$($env:SystemDrive[0])'").DiskNumber)"
$o.av = @(Get-CimInstance -Namespace root/SecurityCenter2 -ClassName AntiVirusProduct | ForEach-Object { @{ name = $_.displayName; state = $_.productState } })
$o.fw = @(Get-CimInstance -Namespace root/SecurityCenter2 -ClassName FirewallProduct | ForEach-Object { @{ name = $_.displayName; state = $_.productState } })
$mp = Get-MpComputerStatus
if ($mp) { $o.mp = @{ mode = "$($mp.AMRunningMode)"; on = [bool]$mp.RealTimeProtectionEnabled; sig = & $d $mp.AntivirusSignatureLastUpdated } }
$o.avsvc = @(Get-Service | Where-Object { $_.DisplayName -match 'Bitdefender|Avast|AVG |Norton|McAfee|Kaspersky|ESET|Malwarebytes|Sophos|Trend Micro|Webroot|Panda|Avira' } | ForEach-Object { @{ name = $_.DisplayName; running = ($_.Status -eq 'Running'); start = "$($_.StartType)" } })
$b = Get-CimInstance Win32_BIOS; $o.bios = @{ ver = "$($b.SMBIOSBIOSVersion)"; date = & $d $b.ReleaseDate }
$m = Get-CimInstance Win32_BaseBoard; $o.board = @{ maker = "$($m.Manufacturer)".Trim(); product = "$($m.Product)".Trim() }
$c = Get-CimInstance Win32_Processor | Select-Object -First 1; $o.cpu = @{ name = "$($c.Name)".Trim(); cores = $c.NumberOfCores; threads = $c.NumberOfLogicalProcessors }
$o.gpu = @(Get-CimInstance Win32_VideoController | ForEach-Object { @{ name = "$($_.Name)"; driver = "$($_.DriverVersion)"; date = & $d $_.DriverDate } })
$g = Get-CimInstance -Namespace root/Microsoft/Windows/DeviceGuard -ClassName Win32_DeviceGuard; $o.dg = @{ running = @($g.SecurityServicesRunning) }
$o.qfe = & $d ((Get-CimInstance Win32_QuickFixEngineering | Where-Object InstalledOn | Sort-Object InstalledOn -Descending | Select-Object -First 1).InstalledOn)
$o | ConvertTo-Json -Depth 4 -Compress
"#;

fn gather() -> System {
    let facts: Value = run("powershell.exe", &["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", SCRIPT], Duration::from_secs(40))
        .ok()
        .filter(|o| o.ok())
        .and_then(|o| serde_json::from_str(o.text().trim()).ok())
        .unwrap_or(Value::Null);
    let reg = Reg::read();
    let now = now_secs();
    let gpu = crate::speed::gpu_now();
    let installed: Vec<String> = crate::installed::programs().into_iter().map(|a| a.name).collect();
    let mut checks = Vec::new();
    checks.extend(security(&facts, &reg, &installed, now));
    checks.extend(updates(&facts, &reg, now));
    checks.extend(hardware(&facts, gpu.as_ref(), now));
    checks.extend(performance(&reg));
    checks.extend(registry());
    System { pc: describe(&facts, &reg, gpu.as_ref()), checks, at: now }
}

// ------------------------------------------------------------------ registry facts

#[derive(Clone, Debug, Default)]
struct Reg {
    build: u32,
    edition: String,
    version: String,
    pause_until: Option<u64>,
    updates_off_by_policy: bool,
    defender_off_by_policy: bool,
    firewall_off: bool,
    memory_integrity: Option<bool>,
    secure_boot: Option<bool>,
    game_mode_off: bool,
    max_cpu: Option<u32>,
    reboot_pending: bool,
}

impl Reg {
    #[cfg(windows)]
    fn read() -> Reg {
        use crate::health::reg::{dword_value, string_values, Hive::LocalMachine as Lm};
        let s = |key: &str, name: &str| string_values(Lm, key).into_iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v);
        let cv = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
        let fw = r"SYSTEM\CurrentControlSet\Services\SharedAccess\Parameters\FirewallPolicy";
        let profiles = ["DomainProfile", "StandardProfile", "PublicProfile"].map(|p| dword_value(Lm, &format!(r"{fw}\{p}"), "EnableFirewall"));
        Reg {
            build: s(cv, "CurrentBuild").and_then(|b| b.parse().ok()).unwrap_or(0),
            edition: s(cv, "EditionID").unwrap_or_default(),
            version: s(cv, "DisplayVersion").unwrap_or_default(),
            pause_until: s(r"SOFTWARE\Microsoft\WindowsUpdate\UX\Settings", "PauseUpdatesExpiryTime").and_then(|t| crate::speed::parse_time(&t)),
            updates_off_by_policy: dword_value(Lm, r"SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU", "NoAutoUpdate") == Some(1)
                || dword_value(Lm, r"SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate", "DisableWindowsUpdateAccess") == Some(1),
            defender_off_by_policy: dword_value(Lm, r"SOFTWARE\Policies\Microsoft\Windows Defender", "DisableAntiSpyware") == Some(1),
            firewall_off: profiles.iter().all(|p| *p == Some(0)),
            memory_integrity: dword_value(Lm, r"SYSTEM\CurrentControlSet\Control\DeviceGuard\Scenarios\HypervisorEnforcedCodeIntegrity", "Enabled").map(|v| v == 1),
            secure_boot: dword_value(Lm, r"SYSTEM\CurrentControlSet\Control\SecureBoot\State", "UEFISecureBootEnabled").map(|v| v == 1),
            game_mode_off: dword_value(crate::health::reg::Hive::CurrentUser, r"Software\Microsoft\GameBar", "AutoGameModeEnabled") == Some(0),
            max_cpu: max_processor_state(),
            reboot_pending: crate::health::reboot_pending(),
        }
    }

    #[cfg(not(windows))]
    fn read() -> Reg {
        Reg::default()
    }
}

/// The power plan's "maximum processor state" when plugged in, in percent.
fn max_processor_state() -> Option<u32> {
    let o = run("powercfg", &["/query", "SCHEME_CURRENT", "SUB_PROCESSOR", "PROCTHROTTLEMAX"], Duration::from_secs(5)).ok()?;
    let text = o.text();
    let line = text.lines().find(|l| l.contains("Current AC Power Setting Index"))?;
    u32::from_str_radix(line.rsplit("0x").next()?.trim(), 16).ok()
}

// ------------------------------------------------------------------ what the PC is

fn arr(v: &Value, key: &str) -> Vec<Value> {
    match v.get(key) {
        Some(Value::Array(a)) => a.clone(),
        Some(Value::Object(_)) => vec![v[key].clone()],
        _ => vec![],
    }
}

fn text(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn num(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(|x| x.as_i64().or_else(|| x.as_str().and_then(|s| s.parse().ok()))).unwrap_or(0)
}

fn date(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(Value::as_str).and_then(crate::speed::parse_time)
}

fn year(t: u64) -> i64 {
    1970 + (t / 31_556_952) as i64
}

fn windows_name(build: u32, version: &str, edition: &str) -> String {
    let os = if build >= 22000 { "Windows 11" } else { "Windows 10" };
    let edition = match edition {
        "Professional" => "Pro",
        "Core" => "Home",
        e => e,
    };
    format!("{os} {edition} {version} (build {build})").replace("  ", " ")
}

fn describe(f: &Value, reg: &Reg, gpu: Option<&crate::speed::Gpu>) -> Vec<(String, String)> {
    let mut pc = Vec::new();
    let cpu = &f["cpu"];
    if !text(cpu, "name").is_empty() {
        pc.push(("Processor".into(), format!("{}, {} cores, {} threads", text(cpu, "name"), num(cpu, "cores"), num(cpu, "threads"))));
    }
    let mem = arr(f, "mem");
    if !mem.is_empty() {
        let total: i64 = mem.iter().map(|m| num(m, "cap")).sum();
        let each = num(&mem[0], "cap");
        let same = mem.iter().all(|m| num(m, "cap") == each);
        let speed = mem.iter().map(|m| num(m, "conf")).min().unwrap_or(0);
        let sticks = if same { format!("{} × {}", mem.len(), size(each as u64)) } else { format!("{} sticks", mem.len()) };
        pc.push(("Memory".into(), format!("{}: {sticks} at {speed} MT/s", size(total as u64))));
    }
    let gpus = arr(f, "gpu");
    if let Some(g) = gpus.iter().find(|g| !text(g, "name").to_ascii_lowercase().contains("basic")) {
        let extra = gpu.map(|g| g.temp.map(|t| format!(", {t} °C")).unwrap_or_default()).unwrap_or_default();
        let driver = date(g, "date").map(|d| format!(", driver from {}", year(d))).unwrap_or_default();
        pc.push(("Graphics".into(), format!("{}{driver}{extra}", text(g, "name"))));
    }
    let disks = arr(f, "disks");
    if !disks.is_empty() {
        let sys = text(f, "sysdisk");
        let list: Vec<String> = disks
            .iter()
            .map(|d| format!("{} ({}, {}{})", text(d, "name"), size(num(d, "size") as u64), media(d), if text(d, "id") == sys { ", Windows" } else { "" }))
            .collect();
        pc.push(("Disks".into(), list.join(" · ")));
    }
    let board = &f["board"];
    if !text(board, "product").is_empty() {
        let bios = date(&f["bios"], "date").map(|d| format!(", BIOS {} from {}", text(&f["bios"], "ver"), year(d))).unwrap_or_default();
        pc.push(("Board".into(), format!("{} {}{bios}", tidy_maker(&text(board, "maker")), text(board, "product"))));
    }
    if reg.build > 0 {
        pc.push(("Windows".into(), windows_name(reg.build, &reg.version, &reg.edition)));
    }
    pc
}

/// "64 GB", "16 GB", "1.8 TB": whole numbers where they're whole.
fn size(b: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let gb = b as f64 / GB;
    if gb >= 1000.0 {
        let tb = gb / 1024.0;
        if (tb - tb.round()).abs() < 0.05 { format!("{:.0} TB", tb) } else { format!("{tb:.1} TB") }
    } else if gb >= 1.0 && ((gb - gb.round()).abs() < 0.05 || gb >= 10.0) {
        format!("{:.0} GB", gb)
    } else {
        human_bytes(b)
    }
}

fn tidy_maker(m: &str) -> String {
    match m {
        "ASUSTeK COMPUTER INC." => "ASUS".into(),
        "Micro-Star International Co., Ltd." => "MSI".into(),
        "Gigabyte Technology Co., Ltd." => "Gigabyte".into(),
        "Crucial Technology" => "Crucial".into(),
        m => m.to_string(),
    }
}

/// MSFT_PhysicalDisk's MediaType and BusType, said plainly.
fn media(d: &Value) -> &'static str {
    match (num(d, "media"), num(d, "bus")) {
        (_, 17) => "NVMe SSD",
        (4, 7) => "USB SSD",
        (_, 7) => "USB drive",
        (4, _) => "SSD",
        (3, _) => "hard disk",
        _ => "drive",
    }
}

// ------------------------------------------------------------------ security

/// Security Center's productState: the middle byte says whether it's on, the low one whether
/// it's up to date.
fn av_on(state: i64) -> bool {
    (state >> 8) & 0xF0 == 0x10
}

fn security(f: &Value, reg: &Reg, installed: &[String], now: u64) -> Vec<Check> {
    let mut out = Vec::new();
    let mp = &f["mp"];
    let defender_on = text(mp, "mode") == "Normal" && mp.get("on").and_then(Value::as_bool) == Some(true);
    let others: Vec<String> = arr(f, "av")
        .iter()
        .filter(|a| !text(a, "name").to_ascii_lowercase().contains("defender") && av_on(num(a, "state")))
        .map(|a| text(a, "name"))
        .collect();
    let sig_age = date(mp, "sig").map(|t| now.saturating_sub(t) / 86_400);
    let has_mp = mp.is_object();
    if !others.is_empty() && defender_on {
        out.push(check(
            "two-antivirus",
            "Security",
            "warn",
            format!("Two antiviruses check every file: {} and Microsoft Defender", others.join(", ")),
            "Each one opens every file before you do, so everything is slower, and they can trip over each other. Normally Defender steps aside by itself; switch its real-time protection off in Windows Security, or remove the other one.".into(),
            open("Windows Security", "windowsdefender:"),
        ));
    } else if !others.is_empty() {
        out.push(check("antivirus", "Security", "ok", format!("{} protects this PC", others.join(", ")), "Microsoft Defender stands by in case it stops.".into(), None));
    } else if defender_on {
        let stale = sig_age.filter(|d| *d > 3);
        out.push(match stale {
            Some(days) => check(
                "antivirus",
                "Security",
                "warn",
                format!("Microsoft Defender's virus list is {days} days old"),
                "It only catches what it knows about. Check for updates in Windows Security.".into(),
                open("Windows Security", "windowsdefender:"),
            ),
            None => check("antivirus", "Security", "ok", "Microsoft Defender protects this PC".into(), "Real-time protection is on and its virus list is up to date.".into(), None),
        });
    } else if has_mp || reg.defender_off_by_policy {
        out.push(check(
            "antivirus",
            "Security",
            "warn",
            "Nothing is protecting this PC from viruses".into(),
            if reg.defender_off_by_policy {
                "A tweak switched Microsoft Defender off with a policy, and no other antivirus is on. Switch it back on in Windows Security.".into()
            } else {
                "Microsoft Defender's real-time protection is off, and no other antivirus is on. Switch it on in Windows Security.".into()
            },
            open("Windows Security", "windowsdefender:"),
        ));
    }
    // An antivirus that was uninstalled, but not all of it: its services still run, it isn't the
    // one protecting you, and all that's left in the app list is its agent or updater.
    let active = others.iter().map(|o| o.to_ascii_lowercase()).collect::<Vec<_>>().join(" ");
    let mut vendors: Vec<String> = Vec::new();
    for s in arr(f, "avsvc") {
        let vendor = text(&s, "name").split_whitespace().next().unwrap_or("").to_string();
        let running = s.get("running").and_then(Value::as_bool) == Some(true);
        if !vendor.is_empty() && running && !active.contains(&vendor.to_ascii_lowercase()) && !vendors.contains(&vendor) {
            vendors.push(vendor);
        }
    }
    for vendor in vendors {
        let theirs: Vec<&String> = installed.iter().filter(|n| n.to_ascii_lowercase().contains(&vendor.to_ascii_lowercase())).collect();
        let helper = |n: &str| ["agent", "update", "installer", "helper"].iter().any(|w| n.to_ascii_lowercase().contains(w));
        if theirs.iter().any(|n| !helper(n)) {
            continue; // the product itself is installed: it's yours to have
        }
        let left = theirs.first().map(|n| n.as_str());
        out.push(check(
            &format!("av-leftover-{}", vendor.to_ascii_lowercase()),
            "Security",
            "tip",
            format!("{vendor}'s antivirus is gone, but its helpers aren't"),
            match left {
                Some(name) => format!("{name} still runs at every start, with nothing left for it to look after. Uninstall it from Installed apps."),
                None => format!("Its services still run at every start, with nothing left for them to look after. {vendor}'s removal tool takes the rest away."),
            },
            if left.is_some() { open("Installed apps", "ms-settings:appsfeatures") } else { None },
        ));
    }
    let third_party_fw = arr(f, "fw").iter().any(|w| av_on(num(w, "state")));
    if reg.firewall_off && !third_party_fw {
        out.push(check(
            "firewall",
            "Security",
            "warn",
            "The firewall is off".into(),
            "Anything on your network can reach the programs listening on this PC. Switch Windows Firewall back on.".into(),
            open("Firewall settings", "windowsdefender://network/"),
        ));
    }
    if reg.memory_integrity == Some(false) {
        out.push(check(
            "memory-integrity",
            "Security",
            "tip",
            "Memory integrity is off".into(),
            "It stops a kind of attack that hides inside drivers. Some old drivers and anti-cheat software need it off; if nothing does, switch it on in Windows Security under Core isolation.".into(),
            open("Windows Security", "windowsdefender:"),
        ));
    }
    if reg.secure_boot == Some(false) {
        out.push(check(
            "secure-boot",
            "Security",
            "tip",
            "Secure Boot is off".into(),
            "It makes sure nothing tampered with Windows before it starts, and some games' anti-cheat needs it. It's switched on in the BIOS.".into(),
            None,
        ));
    }
    out
}

// ------------------------------------------------------------------ updates

/// Windows releases and the day Home and Pro stop getting security updates.
const RELEASES: &[(u32, &str)] = &[
    (19044, "2023-06-13"),
    (19045, "2025-10-14"),
    (22000, "2023-10-10"),
    (22621, "2024-10-08"),
    (22631, "2025-11-11"),
    (26100, "2026-10-13"),
    (26200, "2027-10-12"),
];

fn support_ends(build: u32) -> Option<u64> {
    if build > 0 && build < 19044 {
        return Some(0);
    }
    RELEASES.iter().find(|(b, _)| *b == build).and_then(|(_, d)| crate::speed::parse_time(&format!("{d}T00:00:00Z")))
}

fn day(t: u64) -> String {
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    let days = (t / 86_400) as i64;
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{d} {} {y}", MONTHS[(m - 1) as usize])
}

fn updates(f: &Value, reg: &Reg, now: u64) -> Vec<Check> {
    let mut out = Vec::new();
    let name = windows_name(reg.build, &reg.version, &reg.edition);
    let wu = || open("Windows Update", "ms-settings:windowsupdate");
    match support_ends(reg.build) {
        Some(end) if end <= now => out.push(check(
            "windows-support",
            "Updates",
            "warn",
            format!("{name} doesn't get security updates any more"),
            if end == 0 {
                "Microsoft stopped fixing security holes in it years ago. Updating to the current version is free in Windows Update.".into()
            } else {
                format!("Microsoft stopped fixing its security holes on {}. Updating to the current version is free in Windows Update.", day(end))
            },
            wu(),
        )),
        Some(end) if end.saturating_sub(now) < 60 * 86_400 => out.push(check(
            "windows-support",
            "Updates",
            "tip",
            format!("{name} gets its last security updates on {}", day(end)),
            "After that, security holes stay open. Updating to the current version is free in Windows Update.".into(),
            wu(),
        )),
        Some(end) => out.push(check("windows-support", "Updates", "ok", format!("{name} gets security updates"), format!("Until {}.", day(end)), None)),
        None => {}
    }
    if reg.updates_off_by_policy {
        out.push(check(
            "updates-policy",
            "Updates",
            "warn",
            "A tweak switched automatic updates off".into(),
            "A policy, usually from a tweaking tool, stops Windows Update from installing anything, security fixes included. Remove it and Windows updates as normal.".into(),
            wu(),
        ));
    }
    if let Some(until) = reg.pause_until.filter(|t| *t > now) {
        out.push(check("updates-paused", "Updates", "tip", format!("Updates are paused until {}", day(until)), "Security fixes wait too. Resume them whenever you're ready.".into(), wu()));
    }
    if let Some(last) = date(f, "qfe") {
        let days = now.saturating_sub(last) / 86_400;
        if days > 60 {
            out.push(check(
                "updates-old",
                "Updates",
                "warn",
                format!("Windows hasn't installed an update in {days} days"),
                "Security fixes come out every month. Something is stopping them: a paused update, a full disk, or one that keeps failing. Windows Update says which.".into(),
                wu(),
            ));
        }
    }
    if reg.reboot_pending {
        out.push(check(
            "restart-pending",
            "Updates",
            "tip",
            "Windows is waiting to restart".into(),
            "An update is half installed until it does, and some things run slower meanwhile. Restart when it suits you.".into(),
            wu(),
        ));
    }
    if let Some(g) = arr(f, "gpu").iter().find(|g| !text(g, "name").to_ascii_lowercase().contains("basic")) {
        if let Some(d) = date(g, "date").filter(|d| now.saturating_sub(*d) > 400 * 86_400) {
            out.push(check(
                "gpu-driver-old",
                "Updates",
                "tip",
                format!("Your graphics driver is from {}", year(d)),
                format!("{}'s newer drivers fix crashes and make games run better.", text(g, "name")),
                crate::speed::driver_page(&text(g, "name")),
            ));
        }
    }
    out
}

// ------------------------------------------------------------------ hardware

/// The speed a memory stick is made for, from its part number: Crucial Ballistix
/// "BL16G36C16U4B" is 3600, G.Skill "F4-3600C16D", Corsair "CMK16GX4M2B3200C16", Kingston
/// Fury "KF436C17BBK2/16", Team "TF3D416G3600HC18J".
pub fn rated_speed(part: &str) -> Option<u32> {
    static PATTERNS: std::sync::LazyLock<Vec<(regex::Regex, u32)>> = std::sync::LazyLock::new(|| {
        [
            (r"^BL\d+G(\d{2})C\d", 100),
            (r"^BLS\d+G4D(\d{2})", 100),
            (r"^F[45]-(\d{4})", 1),
            (r"^CM[A-Z0-9]*X[45]M\d[A-Z](\d{4})C", 1),
            (r"^K[FH][45](\d{2})C", 100),
            (r"^HX[45](\d{2})C", 100),
            (r"^T[A-Z0-9]*?(\d{4})HC\d", 1),
        ]
        .into_iter()
        .filter_map(|(p, mul)| regex::Regex::new(p).ok().map(|r| (r, mul)))
        .collect()
    });
    let p = part.trim().to_ascii_uppercase();
    PATTERNS.iter().find_map(|(re, mul)| re.captures(&p).and_then(|c| c[1].parse::<u32>().ok()).map(|n| n * mul)).filter(|s| (1600..=9000).contains(s))
}

/// The memory channel a slot is on: "DIMM_A1" -> 'A', "ChannelB-DIMM0" -> 'B'.
fn channel(slot: &str) -> Option<char> {
    let s = slot.to_ascii_uppercase();
    let after = s.split("CHANNEL").nth(1).or_else(|| s.split("DIMM_").nth(1)).or_else(|| s.split("DIMM").nth(1))?;
    after.trim_start_matches(['_', ' ', '-']).chars().next().filter(char::is_ascii_alphabetic)
}

/// What the memory profile is called in this board's BIOS.
fn profile_name(board_maker: &str, cpu: &str, ddr5: bool) -> &'static str {
    let amd = cpu.to_ascii_uppercase().contains("AMD");
    let maker = board_maker.to_ascii_uppercase();
    match (amd, ddr5) {
        (false, _) => "XMP",
        (true, true) => "EXPO",
        (true, false) if maker.contains("ASUS") => "D.O.C.P.",
        (true, false) if maker.contains("MICRO-STAR") || maker.contains("MSI") => "A-XMP",
        (true, false) => "XMP",
    }
}

fn hardware(f: &Value, gpu: Option<&crate::speed::Gpu>, now: u64) -> Vec<Check> {
    let mut out = Vec::new();
    let mem = arr(f, "mem");
    if let Some(first) = mem.first() {
        let running = mem.iter().map(|m| num(m, "conf")).filter(|s| *s > 0).min().unwrap_or(0) as u32;
        let rated = mem.iter().filter_map(|m| rated_speed(&text(m, "part"))).min();
        let ddr5 = running >= 4000 || rated.is_some_and(|r| r >= 4400);
        if let Some(rated) = rated.filter(|r| running > 0 && *r as f32 > running as f32 * 1.08) {
            let profile = profile_name(&text(&f["board"], "maker"), &text(&f["cpu"], "name"), ddr5);
            let four = if mem.len() >= 4 { format!(" With {} sticks, if {rated} isn't stable, the next step down is.", mem.len()) } else { String::new() };
            out.push(check(
                "memory-speed",
                "Hardware",
                "warn",
                format!("Your memory runs at {running} MT/s, but it's made for {rated}"),
                format!(
                    "{} {} sticks run at the safe standard speed until their profile is switched on in the BIOS (it's called {profile}). {}It's one setting and costs nothing.{four}",
                    tidy_maker(&text(first, "maker")),
                    text(first, "part").split('.').next().unwrap_or_default(),
                    if text(&f["cpu"], "name").to_ascii_uppercase().contains("RYZEN") { "On a Ryzen, part of the processor runs at memory speed too, so it's real speed. " } else { "" }
                ),
                None,
            ));
        }
        let channels: std::collections::HashSet<char> = mem.iter().filter_map(|m| channel(&text(m, "slot"))).collect();
        if mem.len() == 1 {
            out.push(check(
                "memory-channels",
                "Hardware",
                "tip",
                "One memory stick does the work of two".into(),
                "Memory works in pairs: with one stick it has half the lanes. A second, matching stick makes games and big apps faster.".into(),
                None,
            ));
        } else if mem.len() >= 2 && channels.len() == 1 {
            out.push(check(
                "memory-channels",
                "Hardware",
                "tip",
                "Your memory sticks share one channel".into(),
                "They're in slots on the same channel, so they take turns. The board's manual says which slots to use for two (usually A2 and B2).".into(),
                None,
            ));
        }
    }
    let sys = text(f, "sysdisk");
    for d in arr(f, "disks") {
        let name = text(&d, "name");
        if text(&d, "id") == sys && num(&d, "media") == 3 {
            out.push(check(
                "windows-on-hdd",
                "Hardware",
                "warn",
                "Windows is on a hard disk".into(),
                format!("{name} spins; everything Windows does waits on it. Moving Windows to an SSD makes the whole PC several times faster to start and to use."),
                None,
            ));
        }
        match num(&d, "health") {
            1 | 2 => out.push(check(
                &format!("disk-health-{}", text(&d, "id")),
                "Hardware",
                "warn",
                format!("{name} says it's failing"),
                "Windows reports a problem with it. Copy anything you'd miss somewhere else now, while it still reads.".into(),
                None,
            )),
            _ => {}
        }
    }
    if let Some(g) = gpu {
        if let (Some(now_w), Some(max_w)) = (g.link_width, g.link_width_max) {
            if now_w < max_w && now_w <= 8 {
                out.push(check(
                    "gpu-slot",
                    "Hardware",
                    "tip",
                    format!("Your graphics card talks to the PC over {now_w} lanes, not {max_w}"),
                    "It's in a slower slot, or one that's sharing its lanes. The top long slot usually has all 16.".into(),
                    None,
                ));
            }
        }
    }
    if let Some(d) = date(&f["bios"], "date").filter(|d| now.saturating_sub(*d) > 3 * 365 * 86_400) {
        out.push(check(
            "bios-old",
            "Hardware",
            "tip",
            format!("Your BIOS is from {}", year(d)),
            format!("{} {} has newer ones that fix stability and memory compatibility, and security holes. The board maker's support page has them.", tidy_maker(&text(&f["board"], "maker")), text(&f["board"], "product")),
            None,
        ));
    }
    out
}

// ------------------------------------------------------------------ performance settings

fn performance(reg: &Reg) -> Vec<Check> {
    let mut out = Vec::new();
    if let Some(max) = reg.max_cpu.filter(|m| *m < 100) {
        out.push(check(
            "cpu-capped",
            "Performance",
            "warn",
            format!("The power plan holds your processor to {max}%"),
            "\"Maximum processor state\" is set below 100%, so it never runs at full speed. Usually a leftover from a battery or temperature tweak.".into(),
            open("Power settings", "ms-settings:powersleep"),
        ));
    }
    if reg.game_mode_off {
        out.push(check(
            "game-mode",
            "Performance",
            "tip",
            "Game Mode is off".into(),
            "It keeps Windows Update and background work out of the way while you play. On is the default.".into(),
            open("Game Mode", "ms-settings:gaming-gamemode"),
        ));
    }
    out
}

// ------------------------------------------------------------------ registry entries pointing at nothing

/// The program or library a registry entry runs: quoted, or up to ".exe"/".dll".
fn target(s: &str) -> Option<String> {
    let c = s.trim();
    let raw = if let Some(rest) = c.strip_prefix('"') {
        rest.split('"').next()?.to_string()
    } else {
        let l = c.to_ascii_lowercase();
        let end = [".exe", ".dll"].iter().filter_map(|e| l.find(e).map(|i| i + e.len())).min()?;
        c[..end].to_string()
    };
    let expanded = expand_env(&raw);
    let l = expanded.to_ascii_lowercase();
    // Only full paths: a bare name is found on PATH, and \SystemRoot\ is a driver's.
    (l.len() > 3 && l.as_bytes()[1] == b':' && (l.ends_with(".exe") || l.ends_with(".dll"))).then_some(expanded)
}

fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                out.push_str(&std::env::var(&after[..end]).unwrap_or_else(|_| format!("%{}%", &after[..end])));
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Gone for sure: its drive is there, and it isn't. (On a drive that's unplugged it may well
/// come back.)
fn gone(path: &str) -> bool {
    let p = Path::new(path);
    let drive = path.get(..3).map(Path::new);
    drive.is_some_and(Path::exists) && !p.exists()
}

/// A registry key that points at something gone: where it is, and what to call it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Leftover {
    /// Yours (HKCU) or every user's (HKLM).
    user: bool,
    key: String,
    shown: String,
}

/// The kinds of leftover, in the order they're shown: (id, the leftovers).
#[cfg(windows)]
fn leftovers() -> Vec<(&'static str, Vec<Leftover>)> {
    use crate::health::reg::{dword_value, string_values, subkeys, Hive, Hive::CurrentUser as Cu, Hive::LocalMachine as Lm};
    let default = |hive: Hive, key: &str| string_values(hive, key).into_iter().find(|(n, _)| n.is_empty()).map(|(_, v)| v);
    let user = |h: Hive| matches!(h, Hive::CurrentUser);

    // Right-click menu handlers whose library is gone.
    let mut menus = Vec::new();
    for (hive, classes) in [(Cu, r"Software\Classes"), (Lm, r"SOFTWARE\Classes")] {
        for kind in ["*", "Directory", r"Directory\Background", "Folder", "AllFilesystemObjects", "Drive"] {
            let list = format!(r"{classes}\{kind}\shellex\ContextMenuHandlers");
            for name in subkeys(hive, &list) {
                let clsid = if name.starts_with('{') { name.clone() } else { default(hive, &format!(r"{list}\{name}")).unwrap_or_default() };
                if !clsid.starts_with('{') {
                    continue;
                }
                let lib = [(Cu, r"Software\Classes"), (Lm, r"SOFTWARE\Classes")]
                    .iter()
                    .find_map(|(h, c)| default(*h, &format!(r"{c}\CLSID\{clsid}\InprocServer32")))
                    .and_then(|v| target(&v));
                if lib.as_deref().is_some_and(gone) {
                    menus.push(Leftover { user: user(hive), key: format!(r"{list}\{name}"), shown: name.trim_matches(['{', '}']).to_string() });
                }
            }
        }
    }

    // Program shortcuts Windows keeps (App Paths) for programs that are gone.
    let mut paths = Vec::new();
    for (hive, key) in [(Cu, r"Software\Microsoft\Windows\CurrentVersion\App Paths"), (Lm, r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths")] {
        for name in subkeys(hive, key) {
            if default(hive, &format!(r"{key}\{name}")).and_then(|v| target(&v)).is_some_and(|t| gone(&t)) {
                paths.push(Leftover { user: user(hive), key: format!(r"{key}\{name}"), shown: name });
            }
        }
    }

    // Programs in the uninstall list whose uninstaller is gone.
    let mut uninstall = Vec::new();
    for (hive, key) in [
        (Cu, r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
        (Lm, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
        (Lm, r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
    ] {
        for sub in subkeys(hive, key) {
            let k = format!(r"{key}\{sub}");
            if dword_value(hive, &k, "SystemComponent") == Some(1) {
                continue;
            }
            let values = string_values(hive, &k);
            let get = |n: &str| values.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.clone());
            let Some(name) = get("DisplayName").filter(|n| !n.is_empty()) else { continue };
            let Some(cmd) = get("UninstallString") else { continue };
            if !cmd.to_ascii_lowercase().contains("msiexec") && target(&cmd).is_some_and(|t| gone(&t)) {
                uninstall.push(Leftover { user: user(hive), key: k, shown: name });
            }
        }
    }

    // Services set to start whose program is gone.
    let mut services = Vec::new();
    let root = r"SYSTEM\CurrentControlSet\Services";
    for svc in subkeys(Lm, root) {
        let k = format!(r"{root}\{svc}");
        let kind = dword_value(Lm, &k, "Type").unwrap_or(0);
        if kind & 0x30 == 0 || dword_value(Lm, &k, "Start") != Some(2) {
            continue;
        }
        let values = string_values(Lm, &k);
        let get = |n: &str| values.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.clone());
        if get("ImagePath").and_then(|i| target(&i)).is_some_and(|t| gone(&t)) {
            let shown = get("DisplayName").filter(|v| !v.starts_with('@')).unwrap_or_else(|| svc.clone());
            services.push(Leftover { user: false, key: svc, shown });
        }
    }
    vec![("menus", menus), ("app-paths", paths), ("uninstall", uninstall), ("services", services)]
}

#[cfg(not(windows))]
fn leftovers() -> Vec<(&'static str, Vec<Leftover>)> {
    vec![]
}

fn listing(list: &[Leftover]) -> String {
    let mut v: Vec<&str> = list.iter().map(|l| l.shown.as_str()).collect();
    v.sort_unstable();
    v.dedup();
    let more = if v.len() > 4 { format!(" and {} more", v.len() - 4) } else { String::new() };
    format!("{}{more}", v.into_iter().take(4).collect::<Vec<_>>().join(", "))
}

/// The command that fixes every user's leftovers, for Terminal (Admin): each key saved to a .reg
/// file first, then removed; services switched off (not removed), so they can be switched back.
fn admin_command(kind: &str, list: &[Leftover]) -> String {
    if kind == "services" {
        return list.iter().map(|l| format!("sc.exe config \"{}\" start= disabled", l.key)).collect::<Vec<_>>().join("; ");
    }
    let mut parts = vec![r#"$b = "$env:USERPROFILE\.momputer\backups\registry"; New-Item -ItemType Directory -Force $b | Out-Null"#.to_string()];
    for (i, l) in list.iter().enumerate() {
        let key = format!(r"HKLM\{}", l.key);
        parts.push(format!(r#"reg.exe export "{key}" "$b\{kind}-{i}.reg" /y; reg.exe delete "{key}" /f"#));
    }
    parts.join("; ")
}

fn registry() -> Vec<Check> {
    let mut out = Vec::new();
    for (kind, all) in leftovers() {
        for user in [true, false] {
            let list: Vec<Leftover> = all.iter().filter(|l| l.user == user).cloned().collect();
            if list.is_empty() {
                continue;
            }
            let n = list.len();
            let one = n == 1;
            let (title, detail, level) = match kind {
                "menus" => (
                    format!("{n} right-click menu {} {} at programs that are gone", if one { "entry" } else { "entries" }, if one { "points" } else { "point" }),
                    format!("{}. Explorer still tries each one when you right-click. Left by programs you uninstalled.", listing(&list)),
                    "tip",
                ),
                "app-paths" => (
                    format!("Windows still knows {} that {} gone", if one { "1 program".to_string() } else { format!("{n} programs") }, if one { "is" } else { "are" }),
                    format!("{}. Typing {} in Run finds nothing. Clutter from uninstalls.", listing(&list), if one { "its name" } else { "their names" }),
                    "tip",
                ),
                "uninstall" => (
                    format!("{} in Windows' app list can't be uninstalled", if one { "1 program".to_string() } else { format!("{n} programs") }),
                    format!(
                        "{}. {} (deleted by hand, most likely), so Settings lists {} but can't remove {}.",
                        listing(&list),
                        if one { "Its uninstaller is gone" } else { "Their uninstallers are gone" },
                        if one { "it" } else { "them" },
                        if one { "it" } else { "them" }
                    ),
                    "tip",
                ),
                _ => (
                    format!("{} set to start with Windows, but {} gone", if one { "1 service is".to_string() } else { format!("{n} services are") }, if one { "its program is" } else { "their programs are" }),
                    format!("{}. Windows tries to start {} at every boot, waits, and gives up, which slows starting up.", listing(&list), if one { "it" } else { "each one" }),
                    "warn",
                ),
            };
            let id = format!("registry-{kind}-{}", if user { "user" } else { "machine" });
            let fix = if user {
                Fix { label: if one { "Remove it".into() } else { "Remove them".into() }, kind: "regfix", value: id.clone(), hint: Some("Mom keeps a copy, and Undo puts it back.".into()) }
            } else {
                Fix {
                    label: "Copy the fix".into(),
                    kind: "copy",
                    value: admin_command(kind, &list),
                    hint: Some(if kind == "services" {
                        "Paste it into Terminal (Admin). It switches the service off; nothing is deleted.".into()
                    } else {
                        "Paste it into Terminal (Admin). It saves a .reg copy of each entry in .momputer\\backups first.".into()
                    }),
                }
            };
            let detail = if user {
                detail
            } else {
                format!("{detail} {} for every user, so only an administrator can change {}.", if one { "It's" } else { "They're" }, if one { "it" } else { "them" })
            };
            out.push(check(&id, "Registry", level, title, detail, Some(fix)));
        }
    }
    out
}

/// What a registry fix saved before removing anything.
#[derive(Serialize, Deserialize)]
struct RegBackup {
    id: String,
    at: u64,
    keys: Vec<(String, Vec<(String, crate::winsettings::keys::Raw)>)>,
}

/// The keys your own registry fixes may remove, and undo may put back.
const USER_ROOTS: &[&str] = &[r"Software\Classes\", r"Software\Microsoft\Windows\CurrentVersion\App Paths\", r"Software\Microsoft\Windows\CurrentVersion\Uninstall\"];

fn reg_backups(paths: &crate::paths::Paths) -> std::path::PathBuf {
    paths.backups_dir().join("registry")
}

/// Removes your own registry leftovers of one kind ("registry-uninstall-user"), found again now,
/// after saving each key's values. Never every user's: those need an administrator.
pub fn fix_registry(paths: &crate::paths::Paths, id: &str) -> Result<String, String> {
    use crate::winsettings::keys::{self, Hive};
    let kind = id.strip_prefix("registry-").and_then(|k| k.strip_suffix("-user")).ok_or("Mom only removes your own leftovers.")?;
    let list: Vec<Leftover> = leftovers().into_iter().find(|(k, _)| *k == kind).map(|(_, l)| l).unwrap_or_default().into_iter().filter(|l| l.user).collect();
    if list.is_empty() {
        return Err("Nothing left to remove: it's already clean.".into());
    }
    if list.iter().any(|l| !USER_ROOTS.iter().any(|r| l.key.starts_with(r))) {
        return Err("That isn't a key mom removes.".into());
    }
    let backup = RegBackup { id: id.into(), at: now_secs(), keys: list.iter().map(|l| (l.key.clone(), keys::values(Hive::CurrentUser, &l.key))).collect() };
    let file = reg_backups(paths).join(format!("{id}-{}.json", backup.at));
    let json = serde_json::to_string_pretty(&backup).map_err(|e| e.to_string())?;
    crate::util::write_atomic(&file, &json).map_err(|e| format!("Mom couldn't save a copy first, so nothing changed ({e})."))?;
    let mut removed = 0;
    let mut kept = Vec::new();
    for l in &list {
        match keys::delete(Hive::CurrentUser, &l.key) {
            Ok(()) => removed += 1,
            Err(_) => kept.push(l.shown.clone()),
        }
    }
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    Ok(if kept.is_empty() {
        format!("Removed {}. Undo puts {} back.", if removed == 1 { "it".to_string() } else { format!("all {removed}") }, if removed == 1 { "it" } else { "them" })
    } else {
        format!("Removed {removed}; {} stayed: {}.", kept.len(), kept.join(", "))
    })
}

/// Puts back what the last `fix_registry(id)` removed.
pub fn undo_registry(paths: &crate::paths::Paths, id: &str) -> Result<String, String> {
    use crate::winsettings::keys::{self, Hive};
    let prefix = format!("{id}-");
    let latest = std::fs::read_dir(reg_backups(paths))
        .map_err(|_| "There's nothing to undo.".to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|p| Some((p.file_stem()?.to_str()?.strip_prefix(&prefix)?.parse::<u64>().ok()?, p)))
        .max_by_key(|(at, _)| *at)
        .ok_or("There's nothing to undo.")?
        .1;
    let backup: RegBackup = serde_json::from_str(&std::fs::read_to_string(&latest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if backup.id != id || backup.keys.iter().any(|(k, _)| !USER_ROOTS.iter().any(|r| k.starts_with(r))) {
        return Err("That copy isn't one mom made, so she won't use it.".into());
    }
    for (key, values) in &backup.keys {
        keys::restore(Hive::CurrentUser, key, values).map_err(|e| format!("Windows didn't let mom put it back ({e})."))?;
    }
    let _ = std::fs::rename(&latest, latest.with_extension("undone"));
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    Ok("Put back the way it was.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn memory_part_numbers_say_their_speed() {
        assert_eq!(rated_speed("BL16G36C16U4BL.M8FB1"), Some(3600));
        assert_eq!(rated_speed("BLS16G4D30AESB.M16FE"), Some(3000));
        assert_eq!(rated_speed("F4-3600C16-16GTZNC"), Some(3600));
        assert_eq!(rated_speed("F5-6000J3038F16G"), Some(6000));
        assert_eq!(rated_speed("CMK16GX4M2B3200C16"), Some(3200));
        assert_eq!(rated_speed("KF436C17BBK2/16"), Some(3600));
        assert_eq!(rated_speed("KF552C40BBK2-32"), Some(5200));
        assert_eq!(rated_speed("TF3D416G3600HC18J"), Some(3600));
        assert_eq!(rated_speed("M378A2K43CB1-CTD"), None);
        assert_eq!(channel("DIMM_A1"), Some('A'));
        assert_eq!(channel("ChannelB-DIMM0"), Some('B'));
        assert_eq!(channel("DIMM 1"), None);
    }

    #[test]
    fn slow_memory_is_found_and_named_for_the_board() {
        let f = json!({
            "mem": [
                {"cap": 17179869184_i64, "speed": 2666, "conf": 2666, "slot": "DIMM_A1", "part": "BL16G36C16U4BL.M8FB1", "maker": "Crucial Technology"},
                {"cap": 17179869184_i64, "speed": 2666, "conf": 2666, "slot": "DIMM_B1", "part": "BL16G36C16U4BL.M8FB1", "maker": "Crucial Technology"}
            ],
            "board": {"maker": "ASUSTeK COMPUTER INC.", "product": "TUF B450-PLUS GAMING"},
            "cpu": {"name": "AMD Ryzen 7 3800X 8-Core Processor", "cores": 8, "threads": 16},
            "disks": [{"id": "1", "name": "Samsung SSD 970 EVO Plus 1TB", "media": 4, "bus": 17, "health": 0, "size": 1000204886016_i64}],
            "sysdisk": "1",
            "bios": {"ver": "4604", "date": "2024-03-22T03:00:00.0000000+00:00"}
        });
        let checks = hardware(&f, None, crate::speed::parse_time("2026-10-12T00:00:00Z").unwrap());
        let mem = checks.iter().find(|c| c.id == "memory-speed").unwrap();
        assert_eq!(mem.title, "Your memory runs at 2666 MT/s, but it's made for 3600");
        assert!(mem.detail.contains("D.O.C.P.") && mem.detail.contains("Ryzen"), "{}", mem.detail);
        assert!(!checks.iter().any(|c| c.id == "memory-channels"), "A and B: two channels");
        assert!(!checks.iter().any(|c| c.id == "windows-on-hdd" || c.id == "bios-old"));
        let pc = describe(&f, &Reg { build: 26200, version: "25H2".into(), edition: "Professional".into(), ..Reg::default() }, None);
        assert_eq!(pc[1], ("Memory".to_string(), "32 GB: 2 × 16 GB at 2666 MT/s".to_string()));
        assert!(pc.iter().any(|(k, v)| k == "Disks" && v.contains("NVMe SSD, Windows")));
        assert_eq!(pc.last().unwrap().1, "Windows 11 Pro 25H2 (build 26200)");
    }

    #[test]
    fn says_who_protects_the_pc() {
        let now = crate::speed::parse_time("2026-10-12T12:00:00Z").unwrap();
        let reg = Reg::default();
        let defender = json!({"mp": {"mode": "Normal", "on": true, "sig": "2026-10-12T07:29:28.0000000+01:00"},
            "av": [{"name": "Windows Defender", "state": 393472}],
            "avsvc": [{"name": "Bitdefender Agent RedLine Service", "running": true, "start": "Automatic"}, {"name": "Bitdefender Safepay Service", "running": false, "start": "Manual"}]});
        let s = security(&defender, &reg, &["Bitdefender Agent".into(), "Malwarebytes version 5.6.5.306".into()], now);
        assert_eq!((s[0].level, s[0].title.as_str()), ("ok", "Microsoft Defender protects this PC"));
        assert_eq!(s[1].title, "Bitdefender's antivirus is gone, but its helpers aren't");
        assert!(s[1].detail.starts_with("Bitdefender Agent still runs"), "{}", s[1].detail);
        assert_eq!(s.len(), 2);
        // Malwarebytes, installed and on demand, is just installed.
        let mbam = json!({"mp": {"mode": "Normal", "on": true}, "av": [], "avsvc": [{"name": "Malwarebytes Service", "running": true, "start": "Automatic"}]});
        assert_eq!(security(&mbam, &reg, &["Malwarebytes version 5.6.5.306".into()], now).len(), 1);
        let both = json!({"mp": {"mode": "Normal", "on": true}, "av": [{"name": "Avast Antivirus", "state": 266240}], "avsvc": [{"name": "Avast Antivirus", "running": true, "start": "Automatic"}]});
        let s = security(&both, &reg, &[], now);
        assert_eq!(s[0].id, "two-antivirus");
        assert_eq!(s.len(), 1, "an active one isn't a leftover");
        let none = json!({"mp": {"mode": "Normal", "on": false}, "av": []});
        assert_eq!(security(&none, &reg, &[], now)[0].title, "Nothing is protecting this PC from viruses");
    }

    #[test]
    fn windows_versions_and_dates() {
        let now = crate::speed::parse_time("2026-10-12T12:00:00Z").unwrap();
        let at = |build: u32| updates(&json!({}), &Reg { build, version: "x".into(), edition: "Professional".into(), ..Reg::default() }, now);
        assert_eq!(at(26200)[0].level, "ok");
        assert_eq!(at(26100)[0].level, "tip", "24H2 ends tomorrow");
        assert!(at(26100)[0].title.ends_with("on 13 October 2026"), "{}", at(26100)[0].title);
        assert_eq!(at(22631)[0].level, "warn");
        assert_eq!(at(19045)[0].title, "Windows 10 Pro x (build 19045) doesn't get security updates any more");
        assert_eq!(day(0), "1 January 1970");
        assert_eq!(day(crate::speed::parse_time("2024-02-29T10:00:00Z").unwrap()), "29 February 2024");
    }

    #[test]
    fn every_users_leftovers_come_as_a_safe_command() {
        let svc = vec![Leftover { user: false, key: "asComSvc".into(), shown: "ASUS Com Service".into() }];
        assert_eq!(admin_command("services", &svc), r#"sc.exe config "asComSvc" start= disabled"#);
        let app = vec![Leftover { user: false, key: r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\devenv.exe".into(), shown: "devenv.exe".into() }];
        let cmd = admin_command("app-paths", &app);
        assert!(cmd.contains(r#"reg.exe export "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\devenv.exe""#), "{cmd}");
        let export = cmd.find("reg.exe export").unwrap();
        let delete = cmd.find("reg.exe delete").unwrap();
        assert!(export < delete, "a copy is saved before anything is removed");
        assert!(!gone(r"Q:\nowhere\x.exe"), "a drive that isn't there may come back");
    }

    #[test]
    fn registry_targets() {
        assert_eq!(target(r#""C:\Program Files\X\x.exe" --run"#).as_deref(), Some(r"C:\Program Files\X\x.exe"));
        assert_eq!(target(r"C:\Program Files\X\menu.dll").as_deref(), Some(r"C:\Program Files\X\menu.dll"));
        assert_eq!(target(r"\SystemRoot\System32\drivers\x.sys"), None);
        assert_eq!(target("rundll32.exe x.dll"), None, "found on PATH, not a full path");
    }
}
