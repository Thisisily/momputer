//! What starts with Windows: named the way people know the programs, with what each costs while
//! it runs, and switched on or off the way Task Manager does it (its StartupApproved flag), so
//! nothing is deleted and Windows' own Settings agree.

use crate::health::StartupApp;
use crate::paths::Paths;
use crate::procs::ProcTable;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Startup {
    /// What the switch names it by: where it's listed and its entry ("run-user:Steam").
    pub id: String,
    pub name: String,
    /// The entry's own name, when the program's name differs ("EADM").
    pub entry: String,
    pub command: String,
    pub exe: Option<String>,
    /// "Run (you)", "Run (all users)" or "Startup folder".
    pub location: String,
    pub enabled: bool,
    /// Mom can switch it herself: it's yours, not every user's.
    pub yours: bool,
    /// Its processes right now.
    pub running: bool,
    pub mem: u64,
    /// Average share of the whole machine's CPU over the last minutes (0 when not known).
    pub cpu: f32,
    /// Why to leave it on, if it's one to leave on.
    pub keep: Option<String>,
}

/// Everything that starts with Windows: what's on first, the heaviest first.
pub fn list(paths: &Paths, table: &ProcTable) -> Vec<Startup> {
    let files = crate::programs::image_paths(table);
    crate::programs::note_cpu(table);
    let (avg, secs) = crate::programs::averages(table);
    let mut out: Vec<Startup> = crate::health::startup_apps(paths)
        .into_iter()
        .map(|a| {
            let exe = exe_of(&a.command);
            let lexe = exe.as_deref().map(str::to_ascii_lowercase);
            let procs: Vec<_> = table.procs.values().filter(|p| lexe.is_some() && files.get(&p.pid).map(|f| f.to_ascii_lowercase()) == lexe).collect();
            let cores: f32 = procs.iter().filter_map(|p| avg.get(&(p.pid, p.start))).sum();
            let name = exe.as_deref().and_then(crate::programs::description).unwrap_or_else(|| a.name.clone());
            Startup {
                id: id_of(&a),
                keep: keep(exe.as_deref().unwrap_or(&a.command)),
                entry: a.entry.clone(),
                name,
                command: a.command.clone(),
                exe,
                yours: a.location != "Run (all users)" && !a.command.to_ascii_lowercase().contains(r"\programdata\microsoft\windows\start menu"),
                location: a.location,
                enabled: a.enabled,
                running: !procs.is_empty(),
                mem: procs.iter().map(|p| p.mem).sum(),
                cpu: if secs > 0 { cores * 100.0 / table.cpus.max(1) as f32 } else { 0.0 },
            }
        })
        .collect();
    out.sort_by(|a, b| b.enabled.cmp(&a.enabled).then(b.mem.cmp(&a.mem)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

/// "run-user:Steam", "folder-machine:Tailscale.lnk": the hive and list it's in, and its entry.
fn id_of(a: &StartupApp) -> String {
    match a.location.as_str() {
        "Startup folder" if a.command.to_ascii_lowercase().contains(r"\programdata\") => format!("folder-machine:{}", a.entry),
        "Startup folder" => format!("folder-user:{}", a.entry),
        "Run (all users)" => format!("run-machine:{}", a.entry),
        _ => format!("run-user:{}", a.entry),
    }
}

/// The program a startup command runs: "C:\...\steam.exe" from `"C:\...\steam.exe" -silent`.
pub fn exe_of(command: &str) -> Option<String> {
    let c = command.trim();
    let path = if let Some(rest) = c.strip_prefix('"') {
        rest.split('"').next()?.to_string()
    } else {
        let lower = c.to_ascii_lowercase();
        let end = lower.find(".exe").map(|i| i + 4)?;
        c[..end].to_string()
    };
    let expanded = expand(&path);
    expanded.to_ascii_lowercase().ends_with(".exe").then_some(expanded)
}

/// %windir%, %ProgramFiles% and friends, filled in.
fn expand(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let var = &after[..end];
                out.push_str(&std::env::var(var).unwrap_or_else(|_| format!("%{var}%")));
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

/// The ones to leave on: security software, and the helpers that make hardware work.
fn keep(exe: &str) -> Option<String> {
    let l = exe.to_ascii_lowercase();
    const SECURITY: &[&str] = &["securityhealth", "\\bitdefender\\", "\\windows defender\\", "\\malwarebytes\\", "\\kaspersky", "\\avast", "\\avg\\", "\\norton", "\\mcafee", "\\eset\\"];
    if SECURITY.iter().any(|s| l.contains(s)) {
        return Some("Your antivirus. Keep it on.".into());
    }
    const HARDWARE: &[(&str, &str)] = &[
        ("rtkauduservice", "Your sound card's helper"),
        ("realtek", "Your sound card's helper"),
        ("igfx", "Your graphics driver's helper"),
        ("synaptics", "Your touchpad's helper"),
        ("etdctrl", "Your touchpad's helper"),
        ("onedrive", "Keeps OneDrive in sync"),
    ];
    HARDWARE.iter().find(|(k, _)| l.contains(k)).map(|(_, why)| format!("{why}. Keep it on unless you know you don't need it."))
}

const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

/// Switches one on or off, as Task Manager does: a flag Windows reads at sign-in. Nothing else
/// changes; switching it back on undoes it exactly.
pub fn set(paths: &Paths, id: &str, on: bool) -> Result<String, String> {
    let item = crate::health::startup_apps(paths).into_iter().find(|a| id_of(a) == id).ok_or("That one isn't in the startup list any more.")?;
    let (list, entry) = id.split_once(':').ok_or("Mom doesn't know that startup entry.")?;
    let sub = match list {
        "run-user" => "Run",
        "folder-user" => "StartupFolder",
        _ => return Err(format!("{} starts for every user, so only an administrator can switch it. Settings > Apps > Startup can.", item.name)),
    };
    os::set_flag(&format!(r"{APPROVED}\{sub}"), entry, &flag(on, crate::util::now_secs()))?;
    Ok(if on {
        format!("{} will start with Windows again.", item.name)
    } else {
        format!("{} won't start with Windows any more. It's still installed; open it whenever you like.", item.name)
    })
}

/// Task Manager's flag: 02 is on, 03 is off, then three zero bytes, then (when off) the moment
/// it was switched off as a FILETIME.
fn flag(on: bool, now: u64) -> Vec<u8> {
    let mut v = vec![if on { 0x02 } else { 0x03 }, 0, 0, 0];
    let filetime = if on { 0 } else { (now + 11_644_473_600) * 10_000_000 };
    v.extend_from_slice(&filetime.to_le_bytes());
    v
}

#[cfg(windows)]
mod os {
    use crate::util::wide;
    use windows_sys::Win32::System::Registry::{RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_BINARY, REG_OPTION_NON_VOLATILE};

    pub fn set_flag(key: &str, name: &str, data: &[u8]) -> Result<(), String> {
        let k = wide(key);
        let n = wide(name);
        let mut h: HKEY = std::ptr::null_mut();
        // SAFETY: NUL-terminated strings; the key handle is closed before returning.
        unsafe {
            let rc = RegCreateKeyExW(HKEY_CURRENT_USER, k.as_ptr(), 0, std::ptr::null(), REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, std::ptr::null(), &mut h, std::ptr::null_mut());
            if rc != 0 {
                return Err(format!("Windows wouldn't open the startup list (error {rc})."));
            }
            let rc = RegSetValueExW(h, n.as_ptr(), 0, REG_BINARY, data.as_ptr(), data.len() as u32);
            RegCloseKey(h);
            if rc != 0 {
                return Err(format!("Windows wouldn't change it (error {rc})."));
            }
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod os {
    pub fn set_flag(_: &str, _: &str, _: &[u8]) -> Result<(), String> {
        Err("Only on Windows.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_program_in_a_startup_command() {
        assert_eq!(exe_of(r#""C:\Program Files (x86)\Steam\steam.exe" -silent"#).as_deref(), Some(r"C:\Program Files (x86)\Steam\steam.exe"));
        assert_eq!(exe_of(r"C:\Program Files\OpenVPN\bin\openvpn-gui.exe").as_deref(), Some(r"C:\Program Files\OpenVPN\bin\openvpn-gui.exe"));
        assert_eq!(exe_of(r"C:\Users\me\AppData\Local\Poe\app-1.1.21\Poe.exe --launchedAtLogin").as_deref(), Some(r"C:\Users\me\AppData\Local\Poe\app-1.1.21\Poe.exe"));
        assert_eq!(exe_of(r"C:\Users\me\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup\DeepL.lnk"), None);
        let windir = std::env::var("windir").unwrap_or_default();
        if !windir.is_empty() {
            assert_eq!(exe_of(r"%windir%\system32\SecurityHealthSystray.exe"), Some(format!(r"{windir}\system32\SecurityHealthSystray.exe")));
        }
    }

    #[test]
    fn the_flag_is_task_managers() {
        assert_eq!(flag(true, 0), vec![2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let off = flag(false, 1_700_000_000);
        assert_eq!((off[0], off.len()), (3, 12));
        assert!(!crate::health::approved(Some(&off)));
        assert!(crate::health::approved(Some(&flag(true, 0))));
    }

    #[test]
    fn says_which_to_keep() {
        assert!(keep(r"C:\WINDOWS\system32\SecurityHealthSystray.exe").unwrap().contains("antivirus"));
        assert!(keep(r"C:\Program Files\Bitdefender\Bitdefender Security App\bdagent.exe").is_some());
        assert!(keep(r"C:\WINDOWS\System32\RtkAudUService64.exe").unwrap().contains("sound"));
        assert!(keep(r"C:\Program Files (x86)\Steam\steam.exe").is_none());
    }

    #[test]
    fn ids_name_the_list_and_the_entry() {
        let a = |location: &str, entry: &str, command: &str| StartupApp {
            name: "shown".into(),
            entry: entry.into(),
            command: command.into(),
            location: location.into(),
            enabled: true,
        };
        assert_eq!(id_of(&a("Run (you)", "com.squirrel.slack.slack", "slack.exe")), "run-user:com.squirrel.slack.slack");
        assert_eq!(id_of(&a("Run (all users)", "RazerCortex", "x.exe")), "run-machine:RazerCortex");
        assert_eq!(id_of(&a("Startup folder", "FocuSee.lnk", r"C:\Users\me\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup\FocuSee.lnk")), "folder-user:FocuSee.lnk");
        assert_eq!(id_of(&a("Startup folder", "Tailscale.lnk", r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\StartUp\Tailscale.lnk")), "folder-machine:Tailscale.lnk");
    }
}
