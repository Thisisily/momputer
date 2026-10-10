//! Small helpers shared by everything else.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn mtime_secs(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// "2.4 GB", "882 MB", "12 KB".
pub fn human_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else if v >= 100.0 || i <= 2 {
        format!("{v:.0} {}", UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// "45s", "12m", "3h", "6d".
pub fn human_age(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=172_799 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

/// Longer form for sentences: "6 days", "3 hours", "12 minutes".
pub fn human_age_long(secs: u64) -> String {
    fn plural(n: u64, unit: &str) -> String {
        if n == 1 {
            format!("1 {unit}")
        } else {
            format!("{n} {unit}s")
        }
    }
    match secs {
        0..=59 => "a moment".to_string(),
        60..=3599 => plural(secs / 60, "minute"),
        3600..=172_799 => plural(secs / 3600, "hour"),
        _ => plural(secs / 86_400, "day"),
    }
}

pub struct CmdOutput {
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
}

impl CmdOutput {
    pub fn ok(&self) -> bool {
        !self.timed_out && self.status == Some(0)
    }
    pub fn text(&self) -> String {
        decode_text(&self.stdout)
    }
    pub fn err_text(&self) -> String {
        decode_text(&self.stderr)
    }
}

/// Runs a program without flashing a console window, killing it if it takes too long.
pub fn run(program: &str, args: &[&str], timeout: Duration) -> std::io::Result<CmdOutput> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn()?;
    let mut out = child.stdout.take();
    let mut err = child.stderr.take();
    let out_t = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(o) = out.as_mut() {
            let _ = o.read_to_end(&mut buf);
        }
        buf
    });
    let err_t = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(e) = err.as_mut() {
            let _ = e.read_to_end(&mut buf);
        }
        buf
    });
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait()? {
            Some(s) => break s.code(),
            None if Instant::now() >= deadline => {
                timed_out = true;
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(25)),
        }
    };
    Ok(CmdOutput {
        status,
        stdout: out_t.join().unwrap_or_default(),
        stderr: err_t.join().unwrap_or_default(),
        timed_out,
    })
}

/// Windows tools like `wsl.exe` print UTF-16; everything else prints UTF-8.
pub fn decode_text(bytes: &[u8]) -> String {
    let looks_utf16 = bytes.len() >= 2
        && (bytes.starts_with(&[0xFF, 0xFE]) || {
            let zeros = bytes.iter().skip(1).step_by(2).filter(|b| **b == 0).count();
            zeros * 3 > bytes.len() / 2
        });
    if looks_utf16 {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
            .trim_start_matches('\u{feff}')
            .to_string()
    } else {
        String::from_utf8_lossy(bytes).to_string()
    }
}

/// Total size, file count and newest file modification time under `path`.
#[derive(Clone, Copy, Debug, Default)]
pub struct DirStats {
    pub bytes: u64,
    pub files: u64,
    /// Newest file mtime; the folder's own mtime if it holds no files.
    pub newest: u64,
}

pub fn dir_stats(path: &Path) -> DirStats {
    let mut s = DirStats::default();
    let own_mtime = match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_file() => {
            s.bytes = m.len();
            s.files = 1;
            s.newest = mtime_secs(&m);
            return s;
        }
        Ok(m) => mtime_secs(&m),
        Err(_) => return s,
    };
    for entry in walkdir::WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.file_type().is_file() {
            if let Ok(m) = entry.metadata() {
                s.newest = s.newest.max(mtime_secs(&m));
                s.bytes += m.len();
                s.files += 1;
            }
        }
    }
    if s.files == 0 {
        s.newest = own_mtime;
    }
    s
}

/// Deletes a file or folder for real. Returns bytes freed (best effort: locked files are skipped).
pub fn delete_path(path: &Path) -> (u64, bool) {
    let before = dir_stats(path).bytes;
    let ok = match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => {
            if std::fs::remove_dir_all(path).is_ok() {
                true
            } else {
                // Something inside is in use. Delete what we can.
                for entry in walkdir::WalkDir::new(path)
                    .contents_first(true)
                    .into_iter()
                    .filter_map(Result::ok)
                {
                    let p = entry.path();
                    if entry.file_type().is_dir() {
                        let _ = std::fs::remove_dir(p);
                    } else {
                        let _ = std::fs::remove_file(p);
                    }
                }
                !path.exists()
            }
        }
        Ok(_) => std::fs::remove_file(path).is_ok(),
        Err(_) => return (0, true),
    };
    let after = if ok { 0 } else { dir_stats(path).bytes };
    (before.saturating_sub(after), ok)
}

/// Normalizes a Windows-ish path for comparisons: lowercase, backslashes, no trailing slash.
pub fn norm_path(p: &str) -> String {
    let mut s = p.replace('/', "\\").to_lowercase();
    while s.ends_with('\\') && s.len() > 3 {
        s.pop();
    }
    s
}

pub fn file_name(p: &str) -> &str {
    p.rsplit(['\\', '/']).next().unwrap_or(p)
}

pub fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// "1 file", "38 files", "1,204 files".
pub fn plural(n: u64, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{} {word}s", human_count(n))
    }
}

/// "48,211".
pub fn human_count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Full path of a program on PATH. On Windows tries .exe, .cmd and .bat (npm installs tools
/// like pnpm as .cmd files, which `Command::new("pnpm")` can't find on its own).
pub fn on_path(name: &str) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat"] } else { &[""] };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .find_map(|dir| exts.iter().map(|e| dir.join(format!("{name}{e}"))).find(|p| p.is_file()))
}

/// NUL-terminated UTF-16, for Win32 calls.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Roots of the fixed drives ("C:\\", "D:\\"): no USB sticks, DVDs or network shares.
#[cfg(windows)]
pub fn fixed_drives() -> Vec<String> {
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    use windows_sys::Win32::System::WindowsProgramming::DRIVE_FIXED;
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| format!("{}:\\", (b'A' + i) as char))
        .filter(|root| {
            let w = wide(root);
            // SAFETY: `w` is a NUL-terminated UTF-16 string that outlives the call.
            unsafe { GetDriveTypeW(w.as_ptr()) == DRIVE_FIXED }
        })
        .collect()
}

#[cfg(not(windows))]
pub fn fixed_drives() -> Vec<String> {
    Vec::new()
}

/// Truncates on a char boundary, adding an ellipsis.
pub fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Write a file atomically-ish: write to a sibling temp file, then rename over the target.
pub fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!(
        "{}.momtmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    std::fs::write(&tmp, contents)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Mom cleans up her own temp files.
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_read_like_a_human_wrote_them() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(882 * 1024 * 1024), "882 MB");
        assert_eq!(human_bytes(2_576_980_378), "2.4 GB");
        assert_eq!(human_bytes(140 * 1024 * 1024 * 1024), "140 GB");
    }

    #[test]
    fn ages() {
        assert_eq!(human_age(5), "5s");
        assert_eq!(human_age(3 * 3600 + 5), "3h");
        assert_eq!(human_age(6 * 86_400), "6d");
        assert_eq!(human_age_long(2 * 86_400), "2 days");
        assert_eq!(human_age_long(3600), "1 hour");
    }

    #[test]
    fn utf16_output_from_wsl_is_decoded() {
        let s = "Ubuntu\r\n";
        let mut bytes = vec![0xFF, 0xFE];
        for u in s.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(decode_text(&bytes), s);
        assert_eq!(decode_text(b"hello"), "hello");
    }

    #[test]
    fn counts_read_like_a_human_wrote_them() {
        assert_eq!(human_count(7), "7");
        assert_eq!(human_count(48_211), "48,211");
        assert_eq!(human_count(2_000_000), "2,000,000");
        assert_eq!(plural(1, "file"), "1 file");
        assert_eq!(plural(1204, "file"), "1,204 files");
    }

    #[test]
    fn ellipsize_counts_chars() {
        assert_eq!(ellipsize("abcdef", 4), "abc…");
        assert_eq!(ellipsize("abc", 4), "abc");
    }
}
