//! The Mac's side of the checkup: what the Mac is made of, whether it's protected, kept up to
//! date and backed up, how its disk and battery are doing, and how steady it has been.
//!
//! Looks only. Everything is asked of macOS's own tools, by their full paths, the way System
//! Information and System Settings show it: `system_profiler` for the hardware, `fdesetup`,
//! `spctl` and `csrutil` for protection, `tmutil` for backups. None of them needs an
//! administrator to answer, and none changes anything.

use crate::health::Drive;
use crate::speed::Fix;
use crate::stability::{Day, Stability};
use crate::system::{check, Check, System};
use crate::util::{now_secs, run};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

// ------------------------------------------------------------------ small readings

/// One number the kernel keeps under a name ("hw.memsize").
fn sysctl_number(name: &str) -> Option<i64> {
    let name = std::ffi::CString::new(name).ok()?;
    let mut value: i64 = 0;
    let mut size = std::mem::size_of::<i64>();
    // SAFETY: a NUL-terminated name, and eight bytes of ours for the answer (it writes four of
    // them for a 32-bit number, and says how many it wrote).
    let ok = unsafe { libc::sysctlbyname(name.as_ptr(), std::ptr::addr_of_mut!(value).cast(), &mut size, std::ptr::null_mut(), 0) } == 0;
    match (ok, size) {
        (true, 8) => Some(value),
        (true, 4) => Some(i64::from(value as i32)),
        _ => None,
    }
}

/// A line of text the kernel keeps under a name ("machdep.cpu.brand_string").
fn sysctl_text(name: &str) -> Option<String> {
    let name = std::ffi::CString::new(name).ok()?;
    let mut size = 0usize;
    // SAFETY: first how long it is, then a buffer of ours of that length.
    unsafe {
        if libc::sysctlbyname(name.as_ptr(), std::ptr::null_mut(), &mut size, std::ptr::null_mut(), 0) != 0 || size == 0 || size > 4096 {
            return None;
        }
        let mut buf = vec![0u8; size];
        if libc::sysctlbyname(name.as_ptr(), buf.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) != 0 {
            return None;
        }
        buf.truncate(size);
        let text = String::from_utf8_lossy(&buf).trim_end_matches('\0').trim().to_string();
        (!text.is_empty()).then_some(text)
    }
}

/// How long since the Mac started.
pub(crate) fn uptime_secs() -> u64 {
    let Ok(name) = std::ffi::CString::new("kern.boottime") else { return 0 };
    // SAFETY: plain data of the size we say; the kernel fills it or says it didn't.
    let started = unsafe {
        let mut at: libc::timeval = std::mem::zeroed();
        let mut size = std::mem::size_of::<libc::timeval>();
        if libc::sysctlbyname(name.as_ptr(), std::ptr::addr_of_mut!(at).cast(), &mut size, std::ptr::null_mut(), 0) != 0 {
            return 0;
        }
        at.tv_sec.max(0) as u64
    };
    if started == 0 {
        0
    } else {
        now_secs().saturating_sub(started)
    }
}

/// How hard macOS is working to find memory: "normal", "tight" (it's squeezing and swapping) or
/// "critical". This, not how much is "used", says whether a Mac is short of memory: macOS fills
/// what's free with things it can drop in a moment.
pub(crate) fn memory_pressure() -> Option<&'static str> {
    match sysctl_number("kern.memorystatus_vm_pressure_level")? {
        1 => Some("normal"),
        2 => Some("tight"),
        4 => Some("critical"),
        _ => None,
    }
}

// ------------------------------------------------------------------ disks you can see

fn c_text(bytes: &[libc::c_char]) -> String {
    let bytes: Vec<u8> = bytes.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&bytes).to_string()
}

/// What a disk is called: the startup disk has a link to it in /Volumes under its name, and
/// every other disk is in /Volumes under its own.
fn volume_name(mount: &str) -> String {
    if mount != "/" {
        return Path::new(mount).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| mount.to_string());
    }
    std::fs::read_dir("/Volumes")
        .ok()
        .and_then(|dir| {
            dir.flatten().find(|e| std::fs::read_link(e.path()).is_ok_and(|to| to == Path::new("/"))).map(|e| e.file_name().to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "Macintosh HD".into())
}

/// The disks Finder shows: the startup disk, and any other that's this Mac's own or plugged into
/// it and can be written to. Not the hidden parts macOS keeps for itself, not disk images (an
/// app you downloaded, opened to drag out), and nothing on the network.
pub(crate) fn drives() -> Vec<Drive> {
    // SAFETY: first how many are mounted, then a buffer of ours with room for that many.
    let mounts: Vec<libc::statfs> = unsafe {
        let n = libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT);
        if n <= 0 {
            return Vec::new();
        }
        let mut list: Vec<libc::statfs> = vec![std::mem::zeroed(); n as usize + 4];
        let got = libc::getfsstat(list.as_mut_ptr(), (list.len() * std::mem::size_of::<libc::statfs>()) as libc::c_int, libc::MNT_NOWAIT);
        list.truncate(got.max(0) as usize);
        list
    };
    let space = |m: &libc::statfs| (m.f_blocks.saturating_mul(u64::from(m.f_bsize)), m.f_bavail.saturating_mul(u64::from(m.f_bsize)));
    // The startup disk is two volumes that share their space: the system's, which nothing
    // writes to, and yours. Yours has the true count of what's free.
    let data = mounts.iter().find(|m| c_text(&m.f_mntonname) == "/System/Volumes/Data").map(space);
    let mut out = Vec::new();
    for m in &mounts {
        let mount = c_text(&m.f_mntonname);
        let flags = m.f_flags;
        let system = mount == "/";
        let local = flags & libc::MNT_LOCAL as u32 != 0;
        let hidden = flags & libc::MNT_DONTBROWSE as u32 != 0;
        let read_only = flags & libc::MNT_RDONLY as u32 != 0;
        let kind = c_text(&m.f_fstypename);
        if !system && (!local || hidden || read_only || !mount.starts_with("/Volumes/") || !matches!(kind.as_str(), "apfs" | "hfs" | "exfat" | "msdos" | "ntfs")) {
            continue;
        }
        let (total, free) = if system { data.unwrap_or_else(|| space(m)) } else { space(m) };
        if total == 0 {
            continue;
        }
        out.push(Drive { label: volume_name(&mount), root: mount, total, free, system, full_in_days: None });
    }
    out.sort_by(|a, b| b.system.cmp(&a.system).then_with(|| a.root.cmp(&b.root)));
    out
}

// ------------------------------------------------------------------ what the Mac is made of

const QUICK: Duration = Duration::from_secs(8);

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").trim().to_string()
}

fn number(v: &Value, key: &str) -> i64 {
    match v.get(key) {
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
        Some(Value::String(s)) => s.trim().trim_end_matches('%').parse().unwrap_or(0),
        _ => 0,
    }
}

fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

/// What System Information knows, as it would save it. A second or two.
fn profile() -> Value {
    let kinds = ["SPHardwareDataType", "SPDisplaysDataType", "SPStorageDataType", "SPPowerDataType", "SPMemoryDataType", "SPNVMeDataType", "SPSerialATADataType", "SPAudioDataType"];
    let mut args = vec!["-json", "-detailLevel", "mini"];
    args.extend(kinds);
    run("/usr/sbin/system_profiler", &args, Duration::from_secs(40))
        .ok()
        .filter(|o| o.ok())
        .and_then(|o| serde_json::from_str(o.text().trim()).ok())
        .unwrap_or(Value::Null)
}

/// "macOS 26.1", from the system's own note of its version.
fn macos_version() -> Option<String> {
    let plist = std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist").ok()?;
    let value = |key: &str| {
        let after = plist.split(&format!("<key>{key}</key>")).nth(1)?;
        let start = after.find("<string>")? + "<string>".len();
        let end = after[start..].find("</string>")? + start;
        Some(after[start..end].trim().to_string())
    };
    let version = value("ProductVersion")?;
    Some(match value("ProductBuildVersion") {
        Some(build) => format!("macOS {version} ({build})"),
        None => format!("macOS {version}"),
    })
}

/// A size as the box it came in says it: "500 GB", "1 TB" (in thousands, as disks are sold).
fn sold_as(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 950.0 {
        let tb = gb / 1000.0;
        if (tb - tb.round()).abs() < 0.06 {
            format!("{:.0} TB", tb)
        } else {
            format!("{tb:.1} TB")
        }
    } else {
        format!("{:.0} GB", gb)
    }
}

/// The rows of "This Mac", in the order the PC's are in, under the names the desk in BIG MODE
/// knows its parts by.
fn describe(p: &Value, holds: Option<u32>) -> Vec<(String, String)> {
    let mut pc: Vec<(String, String)> = Vec::new();
    let hw = list(p, "SPHardwareDataType").first().cloned().unwrap_or(Value::Null);

    // An Intel Mac names its processor; one with Apple's own chip names the chip.
    let cores = sysctl_number("hw.physicalcpu").unwrap_or(0);
    let threads = sysctl_number("hw.logicalcpu").unwrap_or(0);
    let chip = sysctl_text("machdep.cpu.brand_string").or_else(|| Some(text(&hw, "chip_type")).filter(|c| !c.is_empty())).or_else(|| Some(text(&hw, "cpu_type")).filter(|c| !c.is_empty()));
    if let Some(chip) = chip {
        let chip = chip.replace("(R)", "").replace("(TM)", "").split_whitespace().collect::<Vec<_>>().join(" ");
        pc.push(("Processor".into(), if cores > 0 && threads > cores { format!("{chip}, {cores} cores, {threads} threads") } else if cores > 0 { format!("{chip}, {cores} cores") } else { chip }));
    }

    let total = sysctl_number("hw.memsize").unwrap_or(0).max(0) as u64;
    let sticks: Vec<&Value> = list(p, "SPMemoryDataType").iter().flat_map(|bank| list(bank, "_items")).filter(|s| !text(s, "dimm_size").is_empty() && text(s, "dimm_size") != "empty").collect();
    if total > 0 {
        let size = format!("{} GB", total >> 30);
        pc.push((
            "Memory".into(),
            match sticks.first() {
                // Sticks you could take out (an Intel Mac): how many, how big, how fast.
                Some(first) => {
                    let each = text(first, "dimm_size");
                    let same = sticks.iter().all(|s| text(s, "dimm_size") == each);
                    let kind = text(first, "dimm_type");
                    let speed = text(first, "dimm_speed");
                    let count = if same { format!("{} × {each}", sticks.len()) } else { format!("{} sticks", sticks.len()) };
                    format!("{size}{}: {count}{}", if kind.is_empty() { String::new() } else { format!(" {kind}") }, if speed.is_empty() { String::new() } else { format!(" at {speed}") })
                }
                // Apple's own chips have their memory built in.
                None => format!("{size}, built into the chip"),
            },
        ));
    }

    let cards: Vec<String> = list(p, "SPDisplaysDataType").iter().map(|g| text(g, "sppci_model")).filter(|n| !n.is_empty()).collect();
    if !cards.is_empty() {
        pc.push(("Graphics".into(), cards.join(" · ")));
    }

    // The disks inside it. (The startup disk is the one "/" is on.)
    let startup_disk = list(p, "SPStorageDataType").iter().find(|v| text(v, "mount_point") == "/").map(|v| text(&v["physical_drive"], "device_name")).unwrap_or_default();
    let mut disks: Vec<String> = Vec::new();
    for (kind, how) in [("SPNVMeDataType", "NVMe SSD"), ("SPSerialATADataType", "")] {
        for controller in list(p, kind) {
            for d in list(controller, "_items") {
                let name = text(d, "_name");
                let bytes = number(d, "size_in_bytes").max(0) as u64;
                if name.is_empty() || bytes == 0 || text(d, "removable_media") == "yes" {
                    continue;
                }
                let how = if !how.is_empty() {
                    how
                } else if text(d, "spsata_medium_type").to_ascii_lowercase().contains("rotational") {
                    "hard disk"
                } else {
                    "SSD"
                };
                disks.push(format!("{name} ({}, {how}{})", sold_as(bytes), if name == startup_disk { ", macOS" } else { "" }));
            }
        }
    }
    // A Mac with Apple's own chip doesn't list its disk as a part: the startup disk says what it is.
    if disks.is_empty() {
        if let Some(v) = list(p, "SPStorageDataType").iter().find(|v| text(v, "mount_point") == "/") {
            let d = &v["physical_drive"];
            let how = if text(d, "medium_type") == "ssd" { "SSD" } else { "hard disk" };
            disks.push(format!("{} ({}, {how}, macOS)", text(d, "device_name"), sold_as(number(v, "size_in_bytes").max(0) as u64)));
        }
    }
    if !disks.is_empty() {
        pc.push(("Disks".into(), disks.join(" · ")));
    }

    let screens: Vec<String> = list(p, "SPDisplaysDataType")
        .iter()
        .flat_map(|g| list(g, "spdisplays_ndrvs"))
        .map(|s| {
            let name = match text(s, "_name").as_str() {
                "Color LCD" | "" => "Built-in display".to_string(),
                other => other.to_string(),
            };
            let pixels = text(s, "_spdisplays_pixels");
            if pixels.is_empty() {
                name
            } else {
                format!("{name}, {}", pixels.replace(" x ", " × "))
            }
        })
        .collect();
    if !screens.is_empty() {
        pc.push((if screens.len() > 1 { "Screens" } else { "Screen" }.into(), screens.join(" · ")));
    }

    let speakers = list(p, "SPAudioDataType").iter().flat_map(|a| list(a, "_items")).find(|d| text(d, "coreaudio_default_audio_output_device") == "spaudio_yes").map(|d| text(d, "_name"));
    if let Some(speakers) = speakers.filter(|s| !s.is_empty()) {
        pc.push(("Sound".into(), speakers));
    }

    let model = text(&hw, "machine_name");
    if !model.is_empty() {
        let id = text(&hw, "machine_model");
        pc.push(("Model".into(), if id.is_empty() { model } else { format!("{model} ({id})") }));
    }
    if let Some((_, said)) = battery(p, holds) {
        pc.push(("Battery".into(), said));
    }
    if let Some(version) = macos_version() {
        pc.push(("macOS".into(), version));
    }
    pc
}

/// The battery's own account of itself: (its condition as macOS words it, a line for "This Mac").
fn battery(p: &Value, measured: Option<u32>) -> Option<(String, String)> {
    let b = list(p, "SPPowerDataType").iter().find(|v| text(v, "_name") == "spbattery_information")?;
    let health = &b["sppower_battery_health_info"];
    let condition = text(health, "sppower_battery_health");
    let cycles = number(health, "sppower_battery_cycle_count");
    if condition.is_empty() && cycles == 0 {
        return None;
    }
    // Macs with Apple's own chip also say how much of its first capacity it still holds.
    // An Intel Mac doesn't, so there it's worked out from the battery's own counters.
    let holds = Some(text(health, "sppower_battery_health_maximum_capacity")).filter(|h| !h.is_empty()).or_else(|| measured.map(|n| format!("{n}%"))).unwrap_or_default();
    let mut said = vec![format!("{} cycles", cycles)];
    if !holds.is_empty() {
        said.push(format!("holds {holds} of what it held new"));
    }
    if !condition.is_empty() {
        said.push(format!("condition: {}", condition.to_lowercase()));
    }
    Some((condition, said.join(", ")))
}

/// How much of the capacity it was built with the battery still holds, in percent, from the
/// battery's own counters ("AppleRawMaxCapacity" = 6800 and "DesignCapacity" = 8000, in mAh).
fn battery_holds(said: &str) -> Option<u32> {
    let number = |key: &str| said.split(&format!("\"{key}\" = ")).nth(1)?.split(|c: char| !c.is_ascii_digit()).next()?.parse::<u64>().ok();
    let (now, new) = (number("AppleRawMaxCapacity")?, number("DesignCapacity")?);
    (new > 0 && now > 0 && now <= new * 2).then(|| (now * 100 / new).min(100) as u32)
}

// ------------------------------------------------------------------ the checks

/// A page of System Settings.
fn settings(label: &str, page: &str) -> Option<Fix> {
    Some(Fix { label: label.into(), kind: "open", value: format!("x-apple.systempreferences:{page}"), hint: None })
}

/// What one of macOS's own tools says, on one line. None if it didn't answer.
fn ask(program: &str, args: &[&str]) -> Option<String> {
    let out = run(program, args, QUICK).ok()?;
    let said = format!("{} {}", out.text(), out.err_text());
    let said = said.split_whitespace().collect::<Vec<_>>().join(" ");
    (!said.is_empty()).then_some(said)
}

fn protection() -> Vec<Check> {
    let mut out = Vec::new();
    let privacy = "com.apple.settings.PrivacySecurity.extension";
    if let Some(said) = ask("/usr/bin/fdesetup", &["status"]) {
        out.push(if said.contains("FileVault is On") {
            check("filevault", "Security", "ok", "The disk is locked with FileVault".into(), "If this Mac is lost or stolen, nobody reads what's on it without your password.".into(), None)
        } else if said.contains("FileVault is Off") {
            check(
                "filevault",
                "Security",
                "tip",
                "FileVault is off".into(),
                "Anyone who gets hold of this Mac can read its disk without your password. FileVault locks it; turning it on takes a while and doesn't interrupt you.".into(),
                settings("Open Privacy & Security", &format!("{privacy}?FileVault")),
            )
        } else {
            return out;
        });
    }
    if let Some(said) = ask("/usr/sbin/spctl", &["--status"]) {
        out.push(if said.contains("assessments enabled") {
            check("gatekeeper", "Security", "ok", "Gatekeeper is on".into(), "macOS checks an app you downloaded before it first opens.".into(), None)
        } else {
            check(
                "gatekeeper",
                "Security",
                "warn",
                "Gatekeeper is off".into(),
                "macOS isn't checking apps you download before they open. Something switched that off; `sudo spctl --master-enable` in Terminal turns it back on.".into(),
                settings("Open Privacy & Security", privacy),
            )
        });
    }
    if let Some(said) = ask("/usr/bin/csrutil", &["status"]) {
        if said.contains("status: enabled") {
            out.push(check("sip", "Security", "ok", "System Integrity Protection is on".into(), "Nothing can change macOS's own files, not even with your password.".into(), None));
        } else if said.contains("status: disabled") {
            out.push(check(
                "sip",
                "Security",
                "warn",
                "System Integrity Protection is off".into(),
                "Anything running as an administrator can change macOS's own files. It's switched back on from Recovery: restart holding the power button (or ⌘R on an Intel Mac), open Terminal and run `csrutil enable`.".into(),
                None,
            ));
        }
    }
    if let Some(said) = ask("/usr/libexec/ApplicationFirewall/socketfilterfw", &["--getglobalstate"]) {
        out.push(if said.contains("State = 0") || said.contains("disabled") {
            check(
                "firewall",
                "Security",
                "tip",
                "The firewall is off".into(),
                "It ships off. At home behind a router that's fine; on a network you share with strangers (a café, a hotel), switch it on so nothing there can reach the servers you run.".into(),
                settings("Open Network settings", "com.apple.Network-Settings.extension?Firewall"),
            )
        } else {
            check("firewall", "Security", "ok", "The firewall is on".into(), "Other computers on the network can't reach what's running here unless you allowed it.".into(), None)
        });
    }
    out
}

/// Whether macOS keeps itself up to date, from its own settings for that.
fn updates() -> Vec<Check> {
    let read = |key: &str| ask("/usr/bin/defaults", &["read", "/Library/Preferences/com.apple.SoftwareUpdate", key]);
    let page = "com.apple.Software-Update-Settings.extension";
    let mut out = Vec::new();
    // The list Software Update shows. (The count macOS keeps beside it is of the last time it
    // looked, and stays after those updates are installed.)
    let names = updates_waiting(&read("RecommendedUpdates").unwrap_or_default());
    let waiting = names.len();
    if waiting > 0 {
        out.push(check(
            "updates-waiting",
            "Updates",
            "tip",
            format!("{} waiting", crate::util::plural(waiting as u64, "update")),
            format!("macOS found {} and hasn't installed {} yet. Security fixes come this way.", names.join(", "), if waiting == 1 { "it" } else { "them" }),
            settings("Open Software Update", page),
        ));
    }
    // A setting nobody ever changed isn't written down, and macOS checks unless it's told not to.
    let checking = read("AutomaticCheckEnabled").map(|said| if said.contains("does not exist") { "1".to_string() } else { said });
    match (checking.as_deref(), read("CriticalUpdateInstall").as_deref()) {
        (Some("0"), _) => out.push(check(
            "updates-auto",
            "Updates",
            "warn",
            "macOS isn't checking for updates".into(),
            "Checking automatically is switched off, so security fixes only arrive when you go and look.".into(),
            settings("Open Software Update", page),
        )),
        (_, Some("0")) => out.push(check(
            "updates-auto",
            "Updates",
            "tip",
            "Security responses aren't installed automatically".into(),
            "macOS checks for updates, but the urgent security fixes Apple sends between them wait for you.".into(),
            settings("Open Software Update", page),
        )),
        (Some("1"), _) if waiting == 0 => out.push(check("updates-auto", "Updates", "ok", "macOS keeps itself up to date".into(), "It checks for updates by itself, and nothing is waiting.".into(), None)),
        _ => {}
    }
    out
}

/// The names in Software Update's list of what's waiting, as `defaults` prints it:
/// `( { "Display Name" = "macOS Tahoe 26.8"; "Display Version" = "26.8"; ... } )`.
fn updates_waiting(said: &str) -> Vec<String> {
    said.split("\"Display Name\" = ")
        .skip(1)
        .filter_map(|rest| rest.split(';').next())
        .map(|name| name.trim().trim_matches('"').to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

/// Whether something answers on one of this Mac's own ports.
fn answers(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_millis(300)).is_ok()
}

/// What this Mac offers other computers on its network: the three in Sharing settings that let
/// someone in. Each counts as on when macOS hasn't switched it off and it answers.
fn sharing() -> Vec<Check> {
    let off = switched_off("system");
    let on: Vec<&str> = [("File Sharing", "com.apple.smbd", 445u16), ("Remote Login", "com.openssh.sshd", 22), ("Screen Sharing", "com.apple.screensharing", 5900)]
        .into_iter()
        .filter(|(_, label, port)| !off.contains(*label) && answers(*port))
        .map(|(name, _, _)| name)
        .collect();
    // "guest access: 1" on a shared folder: it opens without a password.
    let guests = on.contains(&"File Sharing") && ask("/usr/sbin/sharing", &["-l"]).is_some_and(|said| said.contains("guest access: 1"));
    vec![sharing_check(&on, guests)]
}

fn sharing_check(on: &[&str], guests: bool) -> Check {
    if on.is_empty() {
        return check("sharing", "Security", "ok", "Nothing here is shared with the network".into(), "File Sharing, Remote Login and Screen Sharing are off, so other computers can't connect to this Mac.".into(), None);
    }
    let can: Vec<&str> = on
        .iter()
        .map(|name| match *name {
            "File Sharing" => "open this Mac's shared folders",
            "Remote Login" => "log in to it with your password",
            _ => "see and control its screen with your password",
        })
        .collect();
    let list = |words: &[&str]| match words {
        [one] => one.to_string(),
        [first @ .., last] => format!("{} and {last}", first.join(", ")),
        [] => String::new(),
    };
    check(
        "sharing",
        "Security",
        "tip",
        format!("{} {} on", list(on), if on.len() == 1 { "is" } else { "are" }),
        format!(
            "Other computers on the same network can {}.{} Handy at home. On a network you share with strangers it's a way in, so switch off what you don't use.",
            list(&can),
            if guests { " Your Public folder opens for anyone, without a password." } else { "" }
        ),
        settings("Open Sharing settings", "com.apple.Sharing-Settings.extension"),
    )
}

/// "5 minutes", "1 hour", "30 seconds".
fn how_long(secs: u64) -> String {
    match secs {
        s if s >= 3_600 && s % 3_600 == 0 => crate::util::plural(s / 3_600, "hour"),
        s if s >= 60 => crate::util::plural(s / 60, "minute"),
        s => crate::util::plural(s, "second"),
    }
}

/// Whether waking the Mac takes your password, and how soon, from what `sysadminctl -screenLock
/// status` says: "screenLock is off", "screenLock delay is immediate" or "... is 300 seconds".
fn lock_check(said: &str) -> Option<Check> {
    let page = settings("Open Lock Screen settings", "com.apple.Lock-Screen-Settings.extension");
    if said.contains("screenLock is off") {
        return Some(check(
            "screen-lock",
            "Security",
            "warn",
            "Waking this Mac doesn't take a password".into(),
            "Anyone who opens the lid is in, with everything you're signed in to. Lock Screen settings can ask for your password once the screen has gone dark.".into(),
            page,
        ));
    }
    let delay = said.split("screenLock delay is ").nth(1)?;
    let secs = if delay.starts_with("immediate") { 0 } else { delay.split_whitespace().next()?.parse::<u64>().ok()? };
    Some(match secs {
        0 => check("screen-lock", "Security", "ok", "Waking this Mac takes your password".into(), "It asks as soon as the screen goes dark.".into(), None),
        s if s < 900 => check("screen-lock", "Security", "ok", "Waking this Mac takes your password".into(), format!("It asks {} after the screen goes dark. Lock Screen settings can make that sooner.", how_long(s)), None),
        s => check(
            "screen-lock",
            "Security",
            "tip",
            format!("This Mac waits {} before it locks", how_long(s)),
            "For that long after the screen goes dark, anyone who opens the lid is in without your password. Lock Screen settings can make it ask sooner.".into(),
            page,
        ),
    })
}

fn hardware(p: &Value, holds: Option<u32>) -> Vec<Check> {
    let mut out = Vec::new();
    // What each disk says about its own health.
    let mut seen = Vec::new();
    for kind in ["SPNVMeDataType", "SPSerialATADataType"] {
        for d in list(p, kind).iter().flat_map(|c| list(c, "_items")) {
            let (name, status) = (text(d, "_name"), text(d, "smart_status"));
            if name.is_empty() || status.is_empty() || seen.contains(&name) {
                continue;
            }
            seen.push(name.clone());
            let id = format!("disk-health-{}", seen.len());
            out.push(match status.as_str() {
                "Verified" => check(&id, "Hardware", "ok", format!("{name} is healthy"), "The disk checks itself and reports no problems.".into(), None),
                "Not Supported" => continue,
                other => check(
                    &id,
                    "Hardware",
                    "warn",
                    format!("{name} says it's failing"),
                    format!("The disk's own check says \"{other}\". Back up what's on it now, before anything else, and have it replaced."),
                    settings("Open Time Machine settings", "com.apple.Time-Machine-Settings.extension"),
                ),
            });
        }
    }
    if let Some((condition, said)) = battery(p, holds) {
        let page = settings("Open Battery settings", "com.apple.Battery-Settings.extension");
        out.push(match condition.as_str() {
            "Good" | "Normal" | "" => check("battery", "Hardware", "ok", "The battery is in good shape".into(), format!("{said}."), None),
            "Fair" => check("battery", "Hardware", "tip", "The battery is wearing".into(), format!("{said}. It holds less than it did, so a charge doesn't last as long. Nothing to do yet."), page),
            _ => check(
                "battery",
                "Hardware",
                "warn",
                "The battery needs service".into(),
                format!("{said}. It holds much less than when it was new and may shut the Mac down without warning. Apple or a repair shop can replace it."),
                page,
            ),
        });
    }
    out
}

fn backups() -> Vec<Check> {
    let page = settings("Open Time Machine settings", "com.apple.Time-Machine-Settings.extension");
    let Some(said) = ask("/usr/bin/tmutil", &["destinationinfo"]) else { return Vec::new() };
    if said.contains("No destinations configured") {
        return vec![check(
            "time-machine",
            "Backups",
            "tip",
            "Time Machine isn't set up".into(),
            "Nothing on this Mac is being backed up by macOS. With a disk plugged in (or one on your network), Time Machine keeps a copy of everything, hour by hour. If you back up another way, never mind.".into(),
            page,
        )];
    }
    let names: Vec<String> = said.split("Name").skip(1).filter_map(|rest| rest.trim_start_matches([' ', ':']).split(" Kind").next().map(|n| n.trim().to_string())).filter(|n| !n.is_empty()).collect();
    let to = if names.is_empty() { "a backup disk".to_string() } else { names.join(" and ") };
    vec![check("time-machine", "Backups", "ok", "Time Machine is set up".into(), format!("This Mac backs up to {to}."), None)]
}

/// The Mac's checkup. A couple of seconds (System Information takes its time).
pub(crate) fn system() -> System {
    let yours = std::thread::spawn(|| {
        let paths = crate::paths::Paths::detect();
        std::thread::scope(|s| {
            let keys = s.spawn(|| crate::secrets::checks(&paths));
            let work = crate::repos::checks(&paths);
            (keys.join().unwrap_or_default(), work)
        })
    });
    let asked = std::thread::spawn(|| {
        let mut checks = protection();
        checks.extend(sharing());
        checks.extend(ask("/usr/sbin/sysadminctl", &["-screenLock", "status"]).as_deref().and_then(lock_check));
        checks.extend(updates());
        checks.extend(backups());
        checks
    });
    let p = profile();
    let holds = ask("/usr/sbin/ioreg", &["-r", "-n", "AppleSmartBattery"]).as_deref().and_then(battery_holds);
    let mut checks = asked.join().unwrap_or_default();
    checks.extend(hardware(&p, holds));
    if let Some(level) = memory_pressure().filter(|l| *l != "normal") {
        checks.push(check(
            "memory-pressure",
            "Performance",
            if level == "critical" { "warn" } else { "tip" },
            if level == "critical" { "The Mac is out of memory".into() } else { "Memory is tight".into() },
            "macOS is squeezing and swapping to keep up, which makes everything slower. Closing apps and browser tabs you're not using gives it room; Activity Monitor's Memory tab says which use the most.".into(),
            None,
        ));
    }
    let (secrets, repos) = yours.join().unwrap_or_default();
    checks.extend(secrets);
    checks.extend(repos);
    System { pc: describe(&p, holds), checks, screens: Vec::new(), at: now_secs() }
}

// ------------------------------------------------------------------ what slows it down

/// What's holding the Mac back right now, as the Mac itself says: it's hot and has slowed its
/// processor to cool off, it's saving the battery, or its disk has no room left to work in.
pub(crate) fn speed_findings() -> Vec<crate::speed::Finding> {
    use crate::speed::Finding;
    let mut out = Vec::new();
    // "CPU_Speed_Limit = 62": the share of its speed macOS lets the processor use just now.
    if let Some(said) = ask("/usr/bin/pmset", &["-g", "therm"]) {
        let limit = said.split("CPU_Speed_Limit").nth(1).and_then(|rest| rest.trim_start_matches([' ', '=']).split_whitespace().next()?.parse::<u32>().ok());
        if let Some(limit) = limit.filter(|l| *l < 95) {
            out.push(Finding {
                id: "mac-hot".into(),
                level: if limit < 70 { "warn" } else { "tip" },
                title: format!("The Mac is hot, and running at {limit}% of its speed"),
                detail: "macOS slows the processor down when it gets too warm, until it cools. Something busy is heating it (see below); a blocked vent, a soft surface under it or a hot room make it worse.".into(),
                fix: None,
                more: vec![],
                timeline: None,
            });
        }
    }
    if let Some(said) = ask("/usr/bin/pmset", &["-g"]) {
        if said.split("lowpowermode").nth(1).and_then(|rest| rest.split_whitespace().next()) == Some("1") {
            out.push(Finding {
                id: "low-power".into(),
                level: "tip",
                title: "Low Power Mode is on".into(),
                detail: "macOS holds the processor back to make the battery last. Fine on the go; plugged in, switching it off lets the Mac run at full speed.".into(),
                fix: settings("Battery settings", "com.apple.Battery-Settings.extension"),
                more: vec![],
                timeline: None,
            });
        }
    }
    if let (Some(used), Some(memory)) = (ask("/usr/sbin/sysctl", &["-n", "vm.swapusage"]).as_deref().and_then(swap_used), sysctl_number("hw.memsize")) {
        let memory = memory.max(0) as u64;
        // A third of the memory again, moved out to the disk: the open apps want more than there is.
        if used >= memory / 3 && used >= 4 << 30 {
            out.push(Finding {
                id: "swap".into(),
                level: "tip",
                title: format!("{} of memory is parked on the disk", crate::util::human_bytes(used)),
                detail: format!(
                    "The apps that are open want more than the {} of memory this Mac has, so macOS moved what it could to the disk. Going back to one of them waits while it's read in again. Quitting the big ones, or restarting, clears it.",
                    crate::util::human_bytes(memory)
                ),
                fix: Some(Fix { label: "Programs".into(), kind: "tab", value: "running:programs".into(), hint: None }),
                more: vec![],
                timeline: None,
            });
        }
    }
    if let Some(d) = drives().into_iter().find(|d| d.system) {
        // Under a twentieth free, or under 10 GB: macOS has nowhere to put what memory can't hold.
        if d.free.saturating_mul(20) < d.total || d.free < 10 << 30 {
            out.push(Finding {
                id: "disk-full".into(),
                level: "warn",
                title: format!("{} is nearly full", d.label),
                detail: format!("Only {} is free. macOS uses free disk space as extra memory, so with none left everything waits. Storage, under Upkeep, says what can go.", crate::util::human_bytes(d.free)),
                fix: Some(Fix { label: "Storage".into(), kind: "tab", value: "upkeep:storage".into(), hint: None }),
                more: vec![],
                timeline: None,
            });
        }
    }
    out
}

/// How much memory macOS has moved to the disk, from `sysctl vm.swapusage`:
/// "total = 8192.00M used = 6144.00M free = 2048.00M (encrypted)".
fn swap_used(said: &str) -> Option<u64> {
    let used = said.split("used = ").nth(1)?.split_whitespace().next()?;
    let (number, unit) = used.split_at(used.len().checked_sub(1)?);
    let each: f64 = match unit {
        "K" => 1024.0,
        "M" => 1_048_576.0,
        "G" => 1_073_741_824.0,
        _ => return None,
    };
    Some((number.parse::<f64>().ok()? * each) as u64)
}

// ------------------------------------------------------------------ what only Finder can move

/// Asks Finder to move something to the Trash. For what belongs to the whole Mac (an app from
/// the App Store) nothing running as you can move it; Finder can, once it has your password.
/// The first time, macOS asks whether mom may ask Finder. When Finder won't do it for her, she
/// shows the thing selected in a Finder window, where ⌘⌫ does it and Finder asks for the password.
pub(crate) fn finder_trash(path: &Path, name: &str) -> Result<(), String> {
    let Some(target) = path.to_str() else { return Err(format!("Finder can't be told where {name} is.")) };
    let mut script: Vec<&str> = FINDER_TRASH.iter().flat_map(|line| ["-e", line]).collect();
    script.push(target);
    let said = match run("/usr/bin/osascript", &script, Duration::from_secs(320)) {
        Ok(out) => format!("{} {}", out.text(), out.err_text()),
        Err(e) => e.to_string(),
    };
    if std::fs::symlink_metadata(path).is_err() {
        return Ok(());
    }
    let (why, show) = finder_said(name, &said);
    if show {
        let _ = run("/usr/bin/open", &["-R", target], QUICK);
    }
    Err(why)
}

/// What Finder is told. The path goes in as an argument, never as part of the script. Finder
/// comes to the front first: a password dialog it raises from behind other windows is one
/// nobody sees. The thing is called `momsThing` because inside `tell application "Finder"` a
/// plainer name can be one of Finder's own words: `target` is a window's, and Finder answered
/// "Can't get target" without ever looking at what it was asked to move.
const FINDER_TRASH: &[&str] = &[
    "on run argv",
    "set momsThing to POSIX file (item 1 of argv) as alias",
    "with timeout of 300 seconds",
    "tell application \"Finder\"",
    "activate",
    "delete momsThing",
    "end tell",
    "end timeout",
    "end run",
];

/// Why Finder didn't move something, in words, and whether to show it in Finder for you to
/// move yourself (not when you were asked and said no).
fn finder_said(name: &str, said: &str) -> (String, bool) {
    let yourself = "It's selected in Finder now: press ⌘⌫ there (Command and Delete) and Finder asks for your password.";
    if said.contains("-128") || said.contains("User canceled") {
        return (format!("{name} is still there: Finder didn't get your go-ahead."), false);
    }
    if said.contains("-1743") || said.contains("Not authorized to send Apple events") {
        return (format!("macOS didn't let mom ask Finder to move {name}. {yourself} To let her ask next time: System Settings, Privacy & Security, Automation, momputer, Finder."), true);
    }
    // Finder's own words, when it gave any: "Finder got an error: ... (-8087)".
    let words = said.split("execution error:").nth(1).map(|w| w.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|w| !w.is_empty());
    let why = match words {
        Some(w) => format!(" ({})", w.chars().take(160).collect::<String>()),
        None => String::new(),
    };
    (format!("{name} belongs to the whole Mac, and Finder wouldn't move it for mom{why}. {yourself}"), true)
}

// ------------------------------------------------------------------ when an app was last used

/// When an app was last opened, as macOS noted it for Spotlight. None if it never said, which
/// is most of the time: the note doesn't outlive an update that replaces the app.
fn last_opened(app: &Path) -> Option<u64> {
    let path = app.to_str()?;
    let said = run("/usr/bin/mdls", &["-name", "kMDItemLastUsedDate", "-name", "kMDItemUseCount", path], QUICK).ok().filter(|o| o.ok())?.text();
    spotlight_last_used(&said)
}

/// The date in what `mdls` says, when it's a record of use: one that comes with a count of how
/// often. For something Spotlight never indexed, `mdls` gives the date the file was last read
/// instead (and no count), which says nothing about anyone opening the app.
///
/// ```text
/// kMDItemLastUsedDate = 2025-03-01 12:00:00 +0000
/// kMDItemUseCount     = 12
/// ```
fn spotlight_last_used(said: &str) -> Option<u64> {
    let value = |key: &str| said.lines().find_map(|l| l.trim().strip_prefix(key)).map(|rest| rest.trim_start_matches([' ', '=']).trim());
    value("kMDItemUseCount")?.parse::<u64>().ok().filter(|n| *n > 0)?;
    chrono::DateTime::parse_from_str(value("kMDItemLastUsedDate")?, "%Y-%m-%d %H:%M:%S %z").ok().map(|t| t.timestamp().max(0) as u64)
}

/// What an app's own things are filed under in the Library: its bundle's id
/// ("com.example.notes") and the names it goes by.
fn bundle_names(app: &Path) -> (Option<String>, Vec<String>) {
    let plain = |s: &str| !s.is_empty() && s != "." && s != ".." && !s.contains(['/', '\0']);
    let mut names: Vec<String> = app.file_stem().map(|n| n.to_string_lossy().to_string()).into_iter().filter(|n| plain(n)).collect();
    let plist = app.join("Contents").join("Info.plist");
    let Some(file) = plist.to_str() else { return (None, names) };
    let read = |args: &[&str]| run("/usr/bin/plutil", args, QUICK).ok().filter(|o| o.ok()).map(|o| o.text());
    let info: Value = read(&["-convert", "json", "-o", "-", file]).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    let mut id = text(&info, "CFBundleIdentifier");
    if id.is_empty() {
        // An Info.plist with a date in it won't turn into JSON; its id still reads.
        id = read(&["-extract", "CFBundleIdentifier", "raw", "-o", "-", file]).map(|t| t.trim().to_string()).unwrap_or_default();
    }
    for key in ["CFBundleName", "CFBundleDisplayName", "CFBundleExecutable"] {
        let name = text(&info, key);
        if plain(&name) && name.len() >= 3 && !names.contains(&name) {
            names.push(name);
        }
    }
    (Some(id).filter(|i| plain(i) && i.contains('.') && !i.contains("..")), names)
}

/// Folders in an app's data that fill up without the app being used: crash handlers, and where
/// other programs register themselves with a browser.
const OTHERS_WRITE: &[&str] = &["crashpad", "nativemessaginghosts", "external extensions"];

/// The signs of use mom found for one app: the newest date among them, and whether macOS
/// wouldn't show her one (then there's no telling, and she says nothing about the app).
#[derive(Default)]
struct Signs {
    newest: Option<u64>,
    hidden: bool,
}

impl Signs {
    fn saw(&mut self, at: u64) {
        if at > 0 {
            self.newest = Some(self.newest.map_or(at, |n| n.max(at)));
        }
    }

    fn told(&mut self, e: &std::io::Error) {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            self.hidden = true;
        }
    }

    /// When one file or folder last changed. Only its date is asked for: nothing is opened.
    fn dated(&mut self, path: &Path) {
        match std::fs::symlink_metadata(path) {
            Ok(m) if !m.file_type().is_symlink() => self.saw(crate::util::mtime_secs(&m)),
            Ok(_) => {}
            Err(e) => self.told(&e),
        }
    }

    /// The newest file in a folder and `depth` folders down. A folder's own date doesn't count
    /// (emptying a cache changes it, and that's a cleaner's doing, not the app being used); nor
    /// do caches, links, the notes Finder leaves in a folder it showed, or the folders where
    /// other programs put files of their own (a browser's native messaging hosts).
    fn files(&mut self, dir: &Path, depth: u32, budget: &mut u32) {
        match std::fs::symlink_metadata(dir) {
            Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
            Ok(_) => return,
            Err(e) => return self.told(&e),
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => return self.told(&e),
        };
        for entry in entries.flatten() {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            let Ok(m) = std::fs::symlink_metadata(entry.path()) else { continue };
            if name.starts_with('.') || m.file_type().is_symlink() {
                continue;
            }
            if m.is_file() {
                self.saw(crate::util::mtime_secs(&m));
            } else if m.is_dir() && depth > 0 && !name.contains("cache") && !OTHERS_WRITE.contains(&name.as_str()) {
                self.files(&entry.path(), depth - 1, budget);
            }
        }
    }
}

/// The signs an app leaves in the Library when it's used: its settings, the windows it had open,
/// what it keeps in Application Support, its logs.
fn app_signs(library: &Path, id: Option<&str>, names: &[String]) -> Signs {
    let mut signs = Signs::default();
    let mut budget = 3_000u32;
    if let Some(id) = id {
        signs.dated(&library.join("Preferences").join(format!("{id}.plist")));
        signs.dated(&library.join("Cookies").join(format!("{id}.binarycookies")));
        signs.dated(&library.join("HTTPStorages").join(format!("{id}.binarycookies")));
        signs.files(&library.join("Saved Application State").join(format!("{id}.savedState")), 0, &mut budget);
        signs.files(&library.join("HTTPStorages").join(id), 0, &mut budget);
        // An app from the App Store keeps all of that in a container of its own, and macOS asks
        // before one app looks around in another's. So mom doesn't look around: she asks only
        // for the dates of the few things an app writes whenever it's used.
        let data = library.join("Containers").join(id).join("Data");
        let inside = data.join("Library");
        for known in [
            inside.join("Preferences"),
            inside.join("Preferences").join(format!("{id}.plist")),
            inside.join("Saved Application State").join(format!("{id}.savedState")),
            inside.join("HTTPStorages").join(id),
            inside.join("Application Support"),
            data.join("Documents"),
        ] {
            signs.dated(&known);
        }
    }
    for name in names.iter().map(String::as_str).chain(id) {
        signs.files(&library.join("Application Support").join(name), 2, &mut budget);
        signs.files(&library.join("Logs").join(name), 1, &mut budget);
    }
    signs
}

/// When an app was last used, and how mom knows: the newest of when macOS saw it opened and
/// when its own settings and data last changed. With no sign it was ever opened, when it was
/// put here or last updated. None when macOS won't show its traces, or nothing has a date.
pub(crate) fn app_last_used(home: &Path, app: &Path) -> Option<(u64, crate::installed::LastUsedSource)> {
    use crate::installed::LastUsedSource;
    let (id, names) = bundle_names(app);
    let mut signs = app_signs(&home.join("Library"), id.as_deref(), &names);
    if signs.hidden {
        return None;
    }
    if let Some(at) = last_opened(app) {
        signs.saw(at);
    }
    if let Some(at) = signs.newest {
        return Some((at, LastUsedSource::Activity));
    }
    let mut bundle = Signs::default();
    bundle.dated(app);
    bundle.dated(&app.join("Contents").join("Info.plist"));
    if let Some(born) = std::fs::symlink_metadata(app).ok().and_then(|m| m.created().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()) {
        bundle.saw(born.as_secs());
    }
    bundle.newest.map(|at| (at, LastUsedSource::Updated))
}

// ------------------------------------------------------------------ what starts with the Mac

/// What a Mac starts by itself: launch agents (yours, and everyone's) and launch daemons, each
/// a small file that names a program and says when to run it. Apple's own aren't listed. Apps
/// you set to open at login are kept elsewhere, where only System Settings shows them.
pub(crate) fn startup_apps(paths: &crate::paths::Paths) -> Vec<crate::health::StartupApp> {
    let yours_off = switched_off(&format!("gui/{}", paths.uid));
    let system_off = switched_off("system");
    let mut out = Vec::new();
    for (dir, location, off) in [
        (paths.library().join("LaunchAgents"), LOCATION_YOURS, &yours_off),
        (PathBuf::from("/Library/LaunchAgents"), LOCATION_EVERYONE, &yours_off),
        (PathBuf::from("/Library/LaunchDaemons"), LOCATION_DAEMON, &system_off),
    ] {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "plist")).collect()).unwrap_or_default();
        files.sort();
        for file in files.into_iter().take(200) {
            let Some(job) = launch_job(&file) else { continue };
            if job.label.starts_with("com.apple.") {
                continue;
            }
            let quoted = if job.args.is_empty() { format!("\"{}\"", job.program) } else { format!("\"{}\" {}", job.program, job.args.join(" ")) };
            out.push(crate::health::StartupApp {
                name: startup_name(&job.program, &job.label),
                command: quoted,
                location: location.to_string(),
                enabled: !job.disabled && !off.contains(&job.label),
                entry: job.label,
            });
        }
    }
    out
}

pub(crate) const LOCATION_YOURS: &str = "Launch agent (you)";
pub(crate) const LOCATION_EVERYONE: &str = "Launch agent (all users)";
pub(crate) const LOCATION_DAEMON: &str = "Launch daemon (all users)";

struct LaunchJob {
    label: String,
    program: String,
    args: Vec<String>,
    disabled: bool,
}

/// One launch agent's or daemon's file, read with macOS's own reader of them.
fn launch_job(file: &Path) -> Option<LaunchJob> {
    let meta = std::fs::symlink_metadata(file).ok()?;
    if !meta.is_file() || meta.len() > 1 << 20 {
        return None;
    }
    let path = file.to_str()?;
    let v: Value = run("/usr/bin/plutil", &["-convert", "json", "-o", "-", "--", path], QUICK).ok().filter(|o| o.ok()).and_then(|o| serde_json::from_str(o.text().trim()).ok())?;
    let label = text(&v, "Label");
    let mut words: Vec<String> = list(&v, "ProgramArguments").iter().filter_map(|a| a.as_str().map(str::to_string)).collect();
    let program = Some(text(&v, "Program")).filter(|p| !p.is_empty()).or_else(|| (!words.is_empty()).then(|| words.remove(0)))?;
    if label.is_empty() || program.is_empty() {
        return None;
    }
    // (With a Program, the first argument is only the name the program is given.)
    if !text(&v, "Program").is_empty() && !words.is_empty() {
        words.remove(0);
    }
    let one_line = |a: String| a.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>();
    Some(LaunchJob { label, program: one_line(program), args: words.into_iter().take(12).map(one_line).collect(), disabled: v.get("Disabled").and_then(Value::as_bool) == Some(true) })
}

/// The jobs someone switched off in one of launchd's domains ("gui/501" is yours).
fn switched_off(domain: &str) -> std::collections::HashSet<String> {
    let Some(said) = run("/bin/launchctl", &["print-disabled", domain], QUICK).ok().filter(|o| o.ok()).map(|o| o.text()) else { return Default::default() };
    said.lines()
        .filter_map(|l| {
            let (label, state) = l.trim().split_once("=>")?;
            matches!(state.trim(), "disabled" | "true").then(|| label.trim().trim_matches('"').to_string())
        })
        .collect()
}

/// What to call one: the app it's part of, else its program's own name, else the telling part
/// of its label ("homebrew.mxcl.mongodb-community" is "mongodb-community").
fn startup_name(program: &str, label: &str) -> String {
    if let Some(app) = crate::programs::description(program) {
        return app;
    }
    let file = crate::util::file_name(program);
    const SAYS_NOTHING: &[&str] = &["sh", "bash", "zsh", "env", "open", "python", "python3", "node", "ruby", "perl", "osascript", "launchctl"];
    // (A helper is often filed under its whole label, which says no more than the label does.)
    if !file.is_empty() && !SAYS_NOTHING.contains(&file) && file != label {
        return file.to_string();
    }
    let mut rest = label;
    for start in ["homebrew.mxcl.", "com.", "org.", "net.", "io.", "us.", "local."] {
        if let Some(r) = rest.strip_prefix(start) {
            rest = r;
            break;
        }
    }
    rest.to_string()
}

/// Switches one of your launch agents off or on: launchd keeps a note, and reads it the next
/// time you log in. The agent's file isn't touched, and what's running now keeps running.
pub(crate) fn set_startup(uid: u32, label: &str, on: bool) -> Result<(), String> {
    if label.is_empty() || label.len() > 200 || !label.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')) {
        return Err("Mom doesn't know that startup entry.".into());
    }
    let target = format!("gui/{uid}/{label}");
    match run("/bin/launchctl", &[if on { "enable" } else { "disable" }, &target], QUICK) {
        Ok(out) if out.ok() => Ok(()),
        Ok(out) => Err(format!("macOS wouldn't change it: {}", out.err_text().trim())),
        Err(e) => Err(format!("Mom couldn't ask macOS to change it: {e}")),
    }
}

// ------------------------------------------------------------------ how steady it has been

const DAYS: u64 = 30;
/// A crash report's first line is its summary; this much of the file holds it.
const HEAD: usize = 4096;

/// What a report is about: an app that crashed, the whole Mac stopping (a kernel panic), or
/// the Mac going off without shutting down.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Fell {
    App,
    Mac,
    PowerOff,
}

/// One crash, from the report macOS wrote about it: (when, what crashed, what kind it was).
fn crash(path: &Path) -> Option<(u64, String, Fell)> {
    use std::io::Read;
    let name = path.file_name()?.to_string_lossy().to_string();
    let lower = name.to_ascii_lowercase();
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let when = crate::util::mtime_secs(&meta);
    if !lower.ends_with(".panic") && !lower.ends_with(".ips") && !lower.ends_with(".crash") {
        return None;
    }
    let mut head = vec![0u8; HEAD];
    let n = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
    let first = String::from_utf8_lossy(&head[..n]);
    // The whole Mac stopping: macOS calls it a panic. The chip that watches over a Mac files
    // one too when the power went without a shutdown ("Unexpected S5 entry": the battery ran
    // flat, the power button was held, or the power failed), and that isn't macOS crashing.
    if lower.ends_with(".panic") {
        return Some((when, "macOS".into(), if first.contains("Unexpected S5 entry") { Fell::PowerOff } else { Fell::Mac }));
    }
    let first = first.lines().next().unwrap_or("");
    // Reports are filed under a number for what happened: 309 is an app crashing, 210 the Mac.
    // The rest (an app using too much processor, a slow shutdown) aren't crashes.
    let summary: Value = serde_json::from_str(first).unwrap_or(Value::Null);
    let kind = text(&summary, "bug_type");
    let at = chrono::DateTime::parse_from_str(&text(&summary, "timestamp"), "%Y-%m-%d %H:%M:%S%.f %z").map(|t| t.timestamp().max(0) as u64).unwrap_or(when);
    match kind.as_str() {
        "309" => {
            let app = Some(text(&summary, "app_name")).filter(|a| !a.is_empty()).unwrap_or_else(|| name.split(['-', '_']).next().unwrap_or(&name).to_string());
            Some((at, crate::system::plain(&app), Fell::App))
        }
        "210" => Some((at, "macOS".into(), Fell::Mac)),
        // An old-style report has no summary line: the file's name says it's a crash.
        "" if lower.ends_with(".crash") => Some((when, crate::system::plain(name.split('_').next().unwrap_or(&name)), Fell::App)),
        _ => None,
    }
}

/// Thirty days of crashes, from the reports macOS keeps: yours, and the ones for the whole Mac.
pub(crate) fn stability(home: &Path, now: u64) -> Stability {
    stability_in(&[home.join("Library/Logs/DiagnosticReports"), PathBuf::from("/Library/Logs/DiagnosticReports")], now)
}

fn stability_in(reports: &[PathBuf], now: u64) -> Stability {
    let from = crate::eventlog::local_midnight(now.saturating_sub((DAYS - 1) * 86_400));
    let mut days: Vec<Day> = (0..DAYS).map(|i| Day { at: crate::eventlog::local_midnight(from + i * 86_400 + 3_600 * 3), ..Day::default() }).collect();
    let mut s = Stability { from, to: now, ..Stability::default() };
    let mut apps: Vec<(String, u32)> = Vec::new();
    let mut folders: Vec<PathBuf> = Vec::new();
    for base in reports {
        folders.push(base.join("Retired"));
        folders.push(base.clone());
    }
    let mut seen = std::collections::HashSet::new();
    for folder in folders {
        let Ok(dir) = std::fs::read_dir(&folder) else { continue };
        for entry in dir.flatten().take(5_000) {
            let Some((at, app, fell)) = crash(&entry.path()) else { continue };
            // The same report can be in both places, once sent.
            if at < from || at > now + 60 || !seen.insert((at, app.clone(), fell)) {
                continue;
            }
            let Some(day) = days.iter_mut().rev().find(|d| d.at <= at) else { continue };
            match fell {
                Fell::Mac => {
                    day.blue_screens += 1;
                    s.blue_screens += 1;
                }
                Fell::PowerOff => {
                    day.shutdowns += 1;
                    s.shutdowns += 1;
                }
                Fell::App => {
                    day.app_crashes += 1;
                    s.app_crashes += 1;
                    match apps.iter_mut().find(|(name, _)| *name == app) {
                        Some((_, n)) => *n += 1,
                        None => apps.push((app, 1)),
                    }
                }
            }
        }
    }
    apps.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    apps.truncate(5);
    s.apps = apps;
    s.days = std::mem::take(&mut days);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mac_says_what_it_is() {
        assert!(uptime_secs() > 0);
        assert!(sysctl_number("hw.memsize").unwrap() > 1 << 30);
        let drives = drives();
        let startup = drives.iter().find(|d| d.system).expect("the startup disk is listed");
        assert_eq!(startup.root, "/");
        assert!(startup.total > startup.free && startup.free > 0);
        assert!(!startup.label.is_empty());
        assert!(macos_version().unwrap().starts_with("macOS "));
    }

    #[test]
    fn a_profile_becomes_the_rows_of_this_mac() {
        let p: Value = serde_json::from_str(
            r#"{"SPHardwareDataType":[{"machine_name":"MacBook Pro","machine_model":"MacBookPro15,1","cpu_type":"6-Core Intel Core i7"}],
                "SPMemoryDataType":[{"_items":[{"dimm_size":"8 GB","dimm_speed":"2667 MHz","dimm_type":"DDR4"},{"dimm_size":"8 GB","dimm_speed":"2667 MHz","dimm_type":"DDR4"}]}],
                "SPDisplaysDataType":[{"sppci_model":"Intel UHD Graphics 630"},{"sppci_model":"AMD Radeon Pro 560X","spdisplays_ndrvs":[{"_name":"Color LCD","_spdisplays_pixels":"3072 x 1920"}]}],
                "SPNVMeDataType":[{"_items":[{"_name":"APPLE SSD AP0512M","size_in_bytes":500277792768,"smart_status":"Verified","removable_media":"no"}]}],
                "SPStorageDataType":[{"mount_point":"/","size_in_bytes":499963174912,"physical_drive":{"device_name":"APPLE SSD AP0512M","medium_type":"ssd"}}],
                "SPAudioDataType":[{"_items":[{"_name":"MacBook Pro Speakers","coreaudio_default_audio_output_device":"spaudio_yes"}]}],
                "SPPowerDataType":[{"_name":"spbattery_information","sppower_battery_health_info":{"sppower_battery_cycle_count":318,"sppower_battery_health":"Good"}}]}"#,
        )
        .unwrap();
        let rows = describe(&p, None);
        let row = |key: &str| rows.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()).unwrap_or("");
        assert!(row("Memory").contains("DDR4: 2 × 8 GB at 2667 MHz"), "{}", row("Memory"));
        assert_eq!(row("Graphics"), "Intel UHD Graphics 630 · AMD Radeon Pro 560X");
        assert_eq!(row("Disks"), "APPLE SSD AP0512M (500 GB, NVMe SSD, macOS)");
        assert_eq!(row("Screen"), "Built-in display, 3072 × 1920");
        assert_eq!(row("Sound"), "MacBook Pro Speakers");
        assert_eq!(row("Model"), "MacBook Pro (MacBookPro15,1)");
        assert_eq!(row("Battery"), "318 cycles, condition: good");
        let checks = hardware(&p, None);
        assert!(checks.iter().any(|c| c.id == "disk-health-1" && c.level == "ok"));
        assert!(checks.iter().any(|c| c.id == "battery" && c.level == "ok"));

        // A battery macOS says needs service, and a disk that says it's failing.
        let bad: Value = serde_json::from_str(
            r#"{"SPNVMeDataType":[{"_items":[{"_name":"Disk","size_in_bytes":1,"smart_status":"Failing"}]}],
                "SPPowerDataType":[{"_name":"spbattery_information","sppower_battery_health_info":{"sppower_battery_cycle_count":1400,"sppower_battery_health":"Service Recommended","sppower_battery_health_maximum_capacity":"71%"}}]}"#,
        )
        .unwrap();
        let checks = hardware(&bad, None);
        assert!(checks.iter().all(|c| c.level == "warn"), "{checks:?}");
        assert!(checks.iter().any(|c| c.detail.contains("holds 71% of what it held new")));
        assert_eq!(sold_as(2_000_398_934_016), "2 TB");
    }

    #[test]
    fn what_macos_says_is_read_the_way_it_says_it() {
        // Software Update's own list, not the count it keeps of the last time it looked.
        let waiting = r#"( { "Display Name" = "macOS Tahoe 26.8"; "Display Version" = "26.8"; Identifier = "MSU_UPDATE_25H100_patch_26.8"; "Product Key" = "MSU_UPDATE_25H100_patch_26.8"; }, { "Display Name" = "Command Line Tools for Xcode"; "Display Version" = "26.3"; Identifier = "Command Line Tools for Xcode"; "Product Key" = "072-12345"; } )"#;
        assert_eq!(updates_waiting(waiting), ["macOS Tahoe 26.8", "Command Line Tools for Xcode"]);
        assert!(updates_waiting("( )").is_empty());
        assert!(updates_waiting("The domain/default pair of (/Library/Preferences/com.apple.SoftwareUpdate, RecommendedUpdates) does not exist").is_empty());

        let lock = |said: &str| lock_check(said).map(|c| (c.level, c.detail));
        assert_eq!(lock("2026-01-01 09:00:00.000 sysadminctl[100:2000] screenLock delay is 300 seconds").unwrap(), ("ok", "It asks 5 minutes after the screen goes dark. Lock Screen settings can make that sooner.".to_string()));
        assert_eq!(lock("sysadminctl[1:2] screenLock delay is immediate").unwrap().0, "ok");
        assert_eq!(lock_check("sysadminctl[1:2] screenLock delay is 3600 seconds").unwrap().title, "This Mac waits 1 hour before it locks");
        assert_eq!(lock("sysadminctl[1:2] screenLock is off").unwrap().0, "warn");
        assert!(lock("sysadminctl: unknown option").is_none());
        assert_eq!((how_long(30), how_long(90), how_long(7_200)), ("30 seconds".to_string(), "1 minute".to_string(), "2 hours".to_string()));

        let nothing = sharing_check(&[], false);
        assert_eq!((nothing.level, nothing.fix.is_none()), ("ok", true));
        let files = sharing_check(&["File Sharing"], true);
        assert_eq!((files.level, files.title.as_str()), ("tip", "File Sharing is on"));
        assert!(files.detail.contains("open this Mac's shared folders.") && files.detail.contains("Your Public folder opens for anyone"), "{}", files.detail);
        let three = sharing_check(&["File Sharing", "Remote Login", "Screen Sharing"], false);
        assert_eq!(three.title, "File Sharing, Remote Login and Screen Sharing are on");
        assert!(!three.detail.contains("Public folder") && three.detail.contains("log in to it with your password and see and control its screen"), "{}", three.detail);

        assert_eq!(battery_holds(r#"| "DesignCapacity" = 8000 | "AppleRawMaxCapacity" = 6800 | "CycleCount" = 318"#), Some(85));
        assert_eq!(battery_holds(r#""DesignCapacity" = 0 "AppleRawMaxCapacity" = 6800"#), None);
        assert_eq!(battery_holds("no battery here"), None);
        assert_eq!(swap_used("total = 8192.00M used = 6144.00M free = 2048.00M (encrypted)"), Some(6_442_450_944));
        assert_eq!(swap_used("total = 2.00G used = 1.50G free = 0.50G"), Some(1_610_612_736));
        assert_eq!(swap_used("nonsense"), None);

        // What Finder is told to move goes by a name that is nobody's word but mom's.
        let told = FINDER_TRASH.join("\n");
        assert!(told.contains("delete momsThing") && told.contains("set momsThing to POSIX file (item 1 of argv) as alias"));
        for finders_word in ["target", "selection", "container", "desktop", "trash", "home", "startup disk"] {
            assert!(!told.contains(finders_word), "{finders_word} means something of its own to Finder");
        }
        // Finder wouldn't move an app: why, and whether to show it in Finder to move by hand.
        let (why, show) = finder_said("Nimbus Notes", "119:132: execution error: Finder got an error: The operation can\u{2019}t be completed because you don\u{2019}t have permission. (-8087)");
        assert!(show && why.contains("(Finder got an error: The operation can\u{2019}t be completed") && why.contains("press ⌘⌫"), "{why}");
        let (why, show) = finder_said("Nimbus Notes", "execution error: Not authorized to send Apple events to Finder. (-1743)");
        assert!(show && why.contains("Automation") && why.contains("selected in Finder"), "{why}");
        let (why, show) = finder_said("Nimbus Notes", "execution error: User canceled. (-128)");
        assert!(!show && why.contains("still there"), "you said no: nothing is opened");
        let (why, show) = finder_said("Nimbus Notes", "");
        assert!(show && why.starts_with("Nimbus Notes belongs to the whole Mac, and Finder wouldn't move it for mom. It's selected"), "{why}");

        assert_eq!(spotlight_last_used("kMDItemLastUsedDate = 2025-03-01 12:00:00 +0000\nkMDItemUseCount     = 12\n"), Some(1740830400));
        assert_eq!(spotlight_last_used("kMDItemLastUsedDate = 2026-11-26 18:07:22 +0000\nkMDItemUseCount     = (null)\n"), None, "the date a file was last read is nobody opening the app");
        assert_eq!(spotlight_last_used("kMDItemLastUsedDate = (null)\nkMDItemUseCount     = (null)\n"), None);
    }

    #[test]
    fn an_apps_last_use_is_the_newest_sign_it_left() {
        let dir = crate::util::temp_dir().join(format!("mom-mac-signs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let library = dir.join("Library");
        let now = now_secs();
        let put = |path: &Path, days_ago: u64| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "x").unwrap();
            std::fs::File::open(path).unwrap().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(now - days_ago * 86_400)).unwrap();
        };
        let about = |signs: &Signs, days_ago: u64| signs.newest.is_some_and(|at| at.abs_diff(now - days_ago * 86_400) < 5);
        let names = ["Notes".to_string()];
        let id = Some("com.example.notes");
        assert!(app_signs(&library, id, &names).newest.is_none(), "an app that was never opened left nothing");

        // Its settings, saved long ago, and something it wrote since.
        put(&library.join("Preferences/com.example.notes.plist"), 900);
        put(&library.join("Application Support/Notes/Local Storage/db.log"), 500);
        // None of these is the app being used: a cleaner emptied its cache yesterday, another
        // program registered itself with it last week, and Finder looked in its folder.
        put(&library.join("Application Support/Notes/Cache/data_0"), 1);
        put(&library.join("Application Support/Notes/Code Cache/js/index"), 1);
        put(&library.join("Application Support/Notes/NativeMessagingHosts/com.other.json"), 7);
        put(&library.join("Application Support/Notes/.DS_Store"), 2);
        let signs = app_signs(&library, id, &names);
        assert!(about(&signs, 500) && !signs.hidden, "{:?}", signs.newest.map(|at| (now - at) / 86_400));
        // Its settings under a name of its own count too, and only the names it goes by do.
        put(&library.join("Application Support/Other App/state.json"), 3);
        assert!(about(&app_signs(&library, id, &names), 500));

        // An App Store app keeps its settings in a container: the folder's date says when.
        let inside = library.join("Containers/com.example.notes/Data/Library/Preferences");
        put(&inside.join("com.example.notes.plist"), 600);
        std::fs::File::open(&inside).unwrap().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(now - 400 * 86_400)).unwrap();
        assert!(about(&app_signs(&library, id, &names), 400));

        // With no trace at all, an app goes by when it was put here, and says so.
        let app = dir.join("Applications/Ghost.app");
        put(&app.join("Contents/Info.plist"), 0);
        std::fs::write(app.join("Contents/Info.plist"), "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>com.example.ghost</string><key>CFBundleName</key><string>Ghost Writer</string></dict></plist>").unwrap();
        assert_eq!(bundle_names(&app), (Some("com.example.ghost".to_string()), vec!["Ghost".to_string(), "Ghost Writer".to_string()]));
        let (at, how) = app_last_used(&dir, &app).unwrap();
        assert!(now.abs_diff(at) < 60 && how == crate::installed::LastUsedSource::Updated, "{at} {now} {how:?}");
        put(&library.join("Application Support/Ghost Writer/settings.json"), 450);
        let (at, how) = app_last_used(&dir, &app).unwrap();
        assert!(at.abs_diff(now - 450 * 86_400) < 5 && how == crate::installed::LastUsedSource::Activity);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn crashes_are_counted_from_the_reports() {
        let dir = crate::util::temp_dir().join(format!("mom-mac-crashes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let reports = dir.join("Library/Logs/DiagnosticReports");
        std::fs::create_dir_all(reports.join("Retired")).unwrap();
        let now = now_secs();
        let at = |ago: u64| chrono::DateTime::from_timestamp((now - ago) as i64, 0).unwrap().format("%Y-%m-%d %H:%M:%S.00 +0000").to_string();
        let report = |file: &str, app: &str, kind: &str, ago: u64| {
            std::fs::write(reports.join(file), format!("{{\"app_name\":\"{app}\",\"timestamp\":\"{}\",\"bug_type\":\"{kind}\"}}\n{{\"more\":1}}\n", at(ago))).unwrap()
        };
        report("Steam Helper-1.ips", "Steam Helper", "309", 3_600);
        report("Steam Helper-2.ips", "Steam Helper", "309", 2 * 86_400);
        report("node-1.ips", "node", "309", 5 * 86_400);
        report("Retired/old.ips", "node", "309", 90 * 86_400); // too long ago
        report("busy.diag", "Chrome", "202", 3_600); // not a crash
        report("cpu.ips", "Chrome", "142", 3_600); // nor this
        let s = stability_in(std::slice::from_ref(&reports), now);
        assert_eq!((s.app_crashes, s.blue_screens, s.shutdowns), (3, 0, 0));
        // The power going without a shutdown is filed as a panic, and isn't one. A real one is.
        std::fs::write(reports.join(".contents.panic"), r#"{"log_path":"/Library/Logs/DiagnosticReports/ProxiedDevice-Bridge/panic-full.ips","panic_string":"Unexpected S5 entry\nGRC0 = 0x00000000\r\n"}"#).unwrap();
        let s = stability_in(std::slice::from_ref(&reports), now);
        assert_eq!((s.blue_screens, s.shutdowns), (0, 1));
        std::fs::write(reports.join("Kernel-1.panic"), "panic(cpu 2 caller 0xffffff8012345678): Kernel trap at 0xffffff7f9e2a1b3c, type 14=page fault\n").unwrap();
        let s = stability_in(std::slice::from_ref(&reports), now);
        assert_eq!((s.blue_screens, s.shutdowns, s.app_crashes), (1, 1, 3));
        assert_eq!(s.apps, vec![("Steam Helper".to_string(), 2), ("node".to_string(), 1)]);
        assert_eq!(s.days.len(), 30);
        assert_eq!(s.days.iter().map(|d| d.app_crashes).sum::<u32>(), 3);
        assert_eq!(s.days.last().unwrap().app_crashes + s.days[s.days.len() - 2].app_crashes, 1, "the newest is today or last night");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
