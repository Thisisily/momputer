//! Backups: whether anything keeps a copy of Documents, Desktop and Pictures (OneDrive, File
//! History), and drives that keep needing disk repairs, which is how a drive that's wearing out
//! usually shows itself first. Looks only.

use crate::eventlog::{query, Ev};
use crate::speed::Fix;
use crate::system::{check, plain, Check};
use serde_json::Value;
use std::path::{Path, PathBuf};

// ------------------------------------------------------------------ your folders

/// Where Documents, Desktop and Pictures are, and what looks after them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Cover {
    /// Each folder: (what people call it, inside the OneDrive folder).
    pub folders: Vec<(&'static str, bool)>,
    pub onedrive_installed: bool,
    pub onedrive_running: bool,
    pub file_history: bool,
}

#[cfg(windows)]
pub(crate) fn cover(f: &Value) -> Cover {
    use crate::health::reg::{string_values, Hive};
    let shell = string_values(Hive::CurrentUser, r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders");
    let roots: Vec<String> = ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .filter(|r| r.len() > 3)
        .map(|r| r.trim_end_matches('\\').to_ascii_lowercase())
        .collect();
    let folders = [("Documents", "Personal"), ("Desktop", "Desktop"), ("Pictures", "My Pictures")]
        .into_iter()
        .map(|(label, key)| {
            let path = shell.iter().find(|(n, _)| n.eq_ignore_ascii_case(key)).map(|(_, p)| crate::system::expand_env(p).to_ascii_lowercase()).unwrap_or_default();
            (label, !path.is_empty() && roots.iter().any(|r| path == *r || path.starts_with(&format!("{r}\\"))))
        })
        .collect();
    let env_dir = |v: &str| std::env::var_os(v).map(PathBuf::from);
    let exe = [env_dir("LOCALAPPDATA").map(|d| d.join(r"Microsoft\OneDrive\OneDrive.exe")), env_dir("ProgramFiles").map(|d| d.join(r"Microsoft OneDrive\OneDrive.exe")), env_dir("ProgramFiles(x86)").map(|d| d.join(r"Microsoft OneDrive\OneDrive.exe"))];
    Cover {
        folders,
        onedrive_installed: exe.iter().flatten().any(|p| p.is_file()),
        onedrive_running: f.get("onedrive").and_then(Value::as_bool).unwrap_or(false),
        file_history: env_dir("LOCALAPPDATA").is_some_and(|d| d.join(r"Microsoft\Windows\FileHistory\Configuration\Config1.xml").is_file()),
    }
}

#[cfg(not(windows))]
pub(crate) fn cover(_: &Value) -> Cover {
    Cover::default()
}

fn names(list: &[&str]) -> String {
    match list {
        [] => String::new(),
        [one] => (*one).to_string(),
        [a, b] => format!("{a} and {b}"),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

pub(crate) fn cover_checks(c: &Cover) -> Vec<Check> {
    let backup = Some(Fix { label: "Windows Backup".into(), kind: "open", value: "ms-settings:backup".into(), hint: None });
    let in_onedrive: Vec<&str> = c.folders.iter().filter(|(_, o)| *o).map(|(n, _)| *n).collect();
    let outside: Vec<&str> = c.folders.iter().filter(|(_, o)| !*o).map(|(n, _)| *n).collect();
    if c.folders.is_empty() {
        return vec![];
    }
    let prompt = |what: &str| Fix {
        label: "Copy for your agent".into(),
        kind: "copy",
        value: format!(
            "{what} Help me set up a backup that keeps working on its own, one step at a time: OneDrive (Windows Backup) for my Documents, Desktop and Pictures, or File History onto a second drive. Don't move or delete any of my files.\n"
        ),
        hint: Some("Paste it to your agent.".into()),
    };
    let mut out = Vec::new();
    if !in_onedrive.is_empty() && !c.onedrive_running {
        let why = if c.onedrive_installed { "OneDrive isn't running" } else { "OneDrive isn't installed any more" };
        let mut k = check(
            "backup-onedrive-off",
            "Backups",
            "warn",
            format!("Your {} aren't backed up", names(&in_onedrive)),
            format!("They're in your OneDrive folder, but {why}, so nothing syncs."),
            backup.clone(),
        );
        k.more.push(prompt(&format!("My {} live in my OneDrive folder, but {why}, so nothing syncs.", names(&in_onedrive))));
        out.push(k);
    } else if !in_onedrive.is_empty() {
        out.push(check("backup-onedrive", "Backups", "ok", format!("OneDrive backs up your {}", names(&in_onedrive)), "Each change is copied to the cloud as you work.".into(), None));
    }
    if !outside.is_empty() && !c.file_history {
        let mut k = check(
            "backup-none",
            "Backups",
            if in_onedrive.is_empty() { "warn" } else { "tip" },
            format!("Nothing backs up your {}", names(&outside)),
            "If this drive fails, they're gone. Windows Backup or File History keeps a copy.".into(),
            backup.clone(),
        );
        k.more.push(prompt(&format!("Nothing backs up my {} on this PC.", names(&outside))));
        out.push(k);
    } else if !outside.is_empty() {
        out.push(check("backup-file-history", "Backups", "ok", "File History backs up your files".into(), "It copies them to another drive every hour.".into(), None));
    }
    out
}

// ------------------------------------------------------------------ drives that keep needing repairs

/// A drive and the repairs it has needed: when each was (Unix seconds), and how many `found.NNN`
/// folders of recovered pieces sit at its top.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Repairs {
    pub letter: char,
    pub label: String,
    pub times: Vec<u64>,
    pub found: usize,
}

const LOOK_BACK_MS: u64 = 90 * 86_400 * 1000;

/// The drives in the checkup's facts (`vols`: letter, label), their repairs from the event logs,
/// and the `found.NNN` folders chkdsk left at their top.
pub(crate) fn repairs(f: &Value) -> Vec<Repairs> {
    let vols = match f.get("vols") {
        Some(Value::Array(a)) => a.clone(),
        Some(o @ Value::Object(_)) => vec![o.clone()],
        _ => vec![],
    };
    let mut drives: Vec<Repairs> = vols
        .iter()
        .take(26)
        .filter_map(|v| {
            let letter = v.get("letter").and_then(Value::as_str)?.chars().next().filter(char::is_ascii_alphabetic)?.to_ascii_uppercase();
            Some(Repairs { letter, label: plain(v.get("label").and_then(Value::as_str).unwrap_or_default()), ..Default::default() })
        })
        .collect();
    let mut events = query("System", &format!("*[System[Provider[@Name='Microsoft-Windows-Ntfs'] and (EventID=55 or EventID=98 or EventID=130) and TimeCreated[timediff(@SystemTime) <= {LOOK_BACK_MS}]]]"), 1_500).unwrap_or_default();
    events.extend(query("Application", &format!("*[System[(Provider[@Name='Chkdsk'] or (Provider[@Name='Microsoft-Windows-Wininit'] and EventID=1001)) and TimeCreated[timediff(@SystemTime) <= {LOOK_BACK_MS}]]]"), 300).unwrap_or_default());
    let devices: Vec<(char, String)> = drives.iter().filter_map(|d| device_path(d.letter).map(|p| (d.letter, p))).collect();
    for e in &events {
        if let Some(letter) = drive_of(e, &devices) {
            if let Some(d) = drives.iter_mut().find(|d| d.letter == letter) {
                d.times.push(e.at);
            }
        }
    }
    for d in &mut drives {
        d.found = found_folders(Path::new(&format!("{}:\\", d.letter)));
        d.times = crate::speed::moments(std::mem::take(&mut d.times), 3_600);
    }
    drives
}

/// Which drive an event says needed a repair, if it says one did. NTFS names the drive; chkdsk
/// and the start-up check name the volume's device ("\Device\HarddiskVolume7").
pub(crate) fn drive_of(e: &Ev, devices: &[(char, String)]) -> Option<char> {
    let letter = |s: &str| {
        let s = s.trim();
        (s.len() == 2 && s.ends_with(':')).then(|| s.chars().next()).flatten().filter(char::is_ascii_alphabetic).map(|c| c.to_ascii_uppercase())
    };
    match (e.provider.as_str(), e.id) {
        ("Microsoft-Windows-Ntfs", 55 | 130) => letter(e.get("DriveName")?),
        // 98 is logged for every drive at every start; only a state other than 0 is a repair.
        ("Microsoft-Windows-Ntfs", 98) => (e.get("CorruptionActionState").is_some_and(|s| s.trim() != "0")).then(|| letter(e.get("DriveName")?)).flatten(),
        ("Chkdsk", _) | ("Microsoft-Windows-Wininit", 1001) => {
            let text = e.text.to_ascii_lowercase();
            // The whole device name: volume 7 isn't volume 70.
            let names = |dev: &str| {
                let dev = dev.to_ascii_lowercase();
                text.match_indices(&dev).any(|(i, _)| !text[i + dev.len()..].starts_with(|c: char| c.is_ascii_digit()))
            };
            devices.iter().find(|(_, dev)| names(dev)).map(|(l, _)| *l).or_else(|| {
                // The start-up check also says "Checking file system on C:".
                let at = text.find("file system on ")? + "file system on ".len();
                letter(text.get(at..at + 2)?)
            })
        }
        _ => None,
    }
}

/// "\Device\HarddiskVolume7" for F.
#[cfg(windows)]
fn device_path(letter: char) -> Option<String> {
    let name: Vec<u16> = format!("{letter}:").encode_utf16().chain(Some(0)).collect();
    let mut buf = vec![0u16; 1024];
    // SAFETY: both buffers are ours; the length is the target's.
    let n = unsafe { windows_sys::Win32::Storage::FileSystem::QueryDosDeviceW(name.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 {
        return None;
    }
    let end = buf[..n.min(buf.len())].iter().position(|c| *c == 0).unwrap_or(n.min(buf.len()));
    Some(String::from_utf16_lossy(&buf[..end])).filter(|p| p.starts_with(r"\Device\"))
}

#[cfg(not(windows))]
fn device_path(_: char) -> Option<String> {
    None
}

/// `found.000`, `found.001`...: what chkdsk recovered, one folder per repair.
fn found_folders(root: &Path) -> usize {
    std::fs::read_dir(root)
        .map(|rd| {
            rd.flatten()
                .take(5_000)
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                    n.len() == 9 && n.starts_with("found.") && n[6..].bytes().all(|b| b.is_ascii_digit()) && e.file_type().is_ok_and(|t| t.is_dir())
                })
                .count()
        })
        .unwrap_or(0)
}

pub(crate) fn repair_checks(drives: &[Repairs]) -> Vec<Check> {
    let mut out = Vec::new();
    for d in drives {
        let n = d.times.len();
        if n < 3 && d.found < 3 {
            continue;
        }
        // Most of these say Windows found damage (and wants a repair), not that it repaired it.
        let title = if n >= d.found {
            format!("Windows found damage on {}: {n} times in 3 months", d.letter)
        } else {
            format!("Windows has repaired {}: {} times", d.letter, d.found)
        };
        let label = if d.label.is_empty() { String::new() } else { format!(" (\"{}\")", d.label.replace(['"', '`'], "'")) };
        let found = match d.found {
            0 => String::new(),
            1 => " One found.000 folder of recovered pieces sits at its top.".to_string(),
            many => format!(" {many} found.NNN folders of recovered pieces sit at its top."),
        };
        let prompt = format!(
            "Windows found file system damage on my drive {}:{label} {n} times in the last three months.{found} Help me, one step at a time: first make it read-only and copy what matters on it to another drive, then check its health (its SMART data, for example with CrystalDiskInfo) and whether it keeps losing power or its connection (a loose or weak USB port, the cable, unplugging without ejecting), and tell me whether to replace it. Nothing may write to {}: until the copy is done.\n",
            d.letter, d.letter
        );
        let mut c = check(
            &format!("drive-repairs-{}", d.letter.to_ascii_lowercase()),
            "Backups",
            "warn",
            title,
            "Usually from losing power or its connection while saving, sometimes a drive wearing out. Copy what matters somewhere else first.".into(),
            Some(Fix { label: format!("Show {}:", d.letter), kind: "reveal", value: format!("{}:\\", d.letter), hint: None }),
        );
        c.more.push(Fix { label: "Copy for your agent".into(), kind: "copy", value: prompt, hint: Some("Paste it to your agent.".into()) });
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cover(folders: &[(&'static str, bool)], installed: bool, running: bool, fh: bool) -> Cover {
        Cover { folders: folders.to_vec(), onedrive_installed: installed, onedrive_running: running, file_history: fh }
    }

    #[test]
    fn folders_in_onedrive_without_onedrive_are_not_backed_up() {
        let all = [("Documents", true), ("Desktop", true), ("Pictures", true)];
        let c = cover_checks(&cover(&all, false, false, false));
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].level, c[0].title.as_str()), ("warn", "Your Documents, Desktop and Pictures aren't backed up"));
        assert_eq!(c[0].detail, "They're in your OneDrive folder, but OneDrive isn't installed any more, so nothing syncs.");
        assert_eq!(cover_checks(&cover(&all, true, false, false))[0].detail, "They're in your OneDrive folder, but OneDrive isn't running, so nothing syncs.");
        assert_eq!(cover_checks(&cover(&all, true, true, false))[0].level, "ok");
        let none = cover_checks(&cover(&[("Documents", false), ("Desktop", false), ("Pictures", false)], false, false, false));
        assert_eq!((none[0].level, none[0].title.as_str()), ("warn", "Nothing backs up your Documents, Desktop and Pictures"));
        let fh = cover_checks(&cover(&[("Documents", false), ("Desktop", false), ("Pictures", false)], false, false, true));
        assert_eq!((fh[0].level, fh[0].title.as_str()), ("ok", "File History backs up your files"));
        let some = cover_checks(&cover(&[("Documents", true), ("Desktop", false), ("Pictures", true)], true, true, false));
        assert_eq!(some.iter().map(|c| (c.level, c.title.as_str())).collect::<Vec<_>>(), [("ok", "OneDrive backs up your Documents and Pictures"), ("tip", "Nothing backs up your Desktop")]);
    }

    fn ev(provider: &str, id: u32, data: &[(&str, &str)], text: &str) -> Ev {
        Ev { provider: provider.into(), id, at: 0, data: data.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), text: text.into() }
    }

    #[test]
    fn repairs_are_told_apart_from_the_daily_all_clear() {
        let devices = vec![('F', r"\Device\HarddiskVolume7".to_string()), ('C', r"\Device\HarddiskVolume3".to_string())];
        assert_eq!(drive_of(&ev("Microsoft-Windows-Ntfs", 98, &[("DriveName", "F:"), ("CorruptionActionState", "2")], ""), &devices), Some('F'));
        assert_eq!(drive_of(&ev("Microsoft-Windows-Ntfs", 98, &[("DriveName", "C:"), ("CorruptionActionState", "0")], ""), &devices), None, "healthy at start-up");
        assert_eq!(drive_of(&ev("Microsoft-Windows-Ntfs", 55, &[("DriveName", "f:")], ""), &devices), Some('F'));
        assert_eq!(drive_of(&ev("Microsoft-Windows-Ntfs", 98, &[("DriveName", r"\\?\Volume{0000}"), ("CorruptionActionState", "1")], ""), &devices), None, "a volume without a letter");
        assert_eq!(drive_of(&ev("Chkdsk", 26226, &[], r"Checking file system on \Device\HarddiskVolume7 Volume label is Nimbus."), &devices), Some('F'));
        assert_eq!(drive_of(&ev("Chkdsk", 26226, &[], r"Checking file system on \Device\HarddiskVolume70 Volume label is Atlas."), &devices), None, "volume 70 isn't volume 7");
        assert_eq!(drive_of(&ev("Microsoft-Windows-Wininit", 1001, &[], "Checking file system on C: The type of the file system is NTFS."), &devices), Some('C'));
    }

    #[test]
    fn a_drive_that_keeps_needing_repairs_says_so() {
        let worn = Repairs { letter: 'F', label: "Nimbus \"Backup\"".into(), times: vec![1, 2, 3, 4], found: 0 };
        let c = repair_checks(&[worn]);
        assert_eq!((c[0].level, c[0].title.as_str()), ("warn", "Windows found damage on F: 4 times in 3 months"));
        assert_eq!((c[0].fix.as_ref().unwrap().kind, c[0].fix.as_ref().unwrap().value.as_str()), ("reveal", "F:\\"));
        let prompt = &c[0].more[0].value;
        assert!(prompt.contains("damage on my drive F: (\"Nimbus 'Backup'\") 4 times"), "{prompt}");
        assert!(!prompt.contains("found."), "no recovered pieces, so they aren't mentioned: {prompt}");
        let found = Repairs { letter: 'D', label: String::new(), times: vec![], found: 40 };
        let c = repair_checks(&[found]);
        assert_eq!(c[0].title, "Windows has repaired D: 40 times");
        assert!(c[0].more[0].value.contains("40 found.NNN folders"));
        assert!(repair_checks(&[Repairs { letter: 'C', label: String::new(), times: vec![1], found: 1 }]).is_empty());
        let dir = std::env::temp_dir().join(format!("mom-found-{}", std::process::id()));
        for n in ["found.000", "found.001", "FOUND.017", "found.things", "found.0001"] {
            std::fs::create_dir_all(dir.join(n)).unwrap();
        }
        std::fs::write(dir.join("found.002"), b"a file, not a folder").unwrap();
        assert_eq!(found_folders(&dir), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
