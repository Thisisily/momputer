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

/// Windows' own PowerShell, resolved by the OS rather than PATH or environment variables.
/// Failure must not fall back to a same-named executable in a user-controlled directory.
pub(crate) fn windows_powershell() -> std::io::Result<String> {
    #[cfg(windows)]
    {
        let mut directory = [0u16; 32_768];
        // SAFETY: the owned buffer and its character count are valid for this call.
        let len = unsafe {
            windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW(directory.as_mut_ptr(), directory.len() as u32)
        } as usize;
        if len == 0 { return Err(std::io::Error::last_os_error()); }
        powershell_in_system_directory(&directory, len)
    }
    #[cfg(not(windows))]
    { Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "Windows PowerShell is only available on Windows.")) }
}

#[cfg(windows)]
fn powershell_in_system_directory(directory: &[u16], len: usize) -> std::io::Result<String> {
    let invalid = || std::io::Error::new(std::io::ErrorKind::InvalidData, "Windows did not return a valid system directory.");
    if len == 0 || len >= directory.len() || directory[len] != 0 || directory[..len].contains(&0) {
        return Err(invalid());
    }
    let directory = String::from_utf16(&directory[..len]).map_err(|_| invalid())?;
    if !Path::new(&directory).is_absolute() { return Err(invalid()); }
    Ok(Path::new(&directory).join(r"WindowsPowerShell\v1.0\powershell.exe").to_str().ok_or_else(invalid)?.to_owned())
}

/// Runs a program without flashing a console window, killing it if it takes too long.
pub fn run(program: &str, args: &[&str], timeout: Duration) -> std::io::Result<CmdOutput> {
    run_in(program, args, None, timeout)
}

/// `run`, started in a folder of your choosing.
///
/// The time limit is for the whole thing. When it's up, the program goes and so does everything
/// it started (they'd otherwise carry on, holding the pipes its output comes through). And once
/// the program itself has finished, mom waits only a moment for the rest of what it printed:
/// something it left running with those pipes in hand can't keep her waiting.
pub fn run_in(program: &str, args: &[&str], folder: Option<&std::path::Path>, timeout: Duration) -> std::io::Result<CmdOutput> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(folder) = folder {
        cmd.current_dir(folder);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn()?;
    let family = Family::of(&child);
    let (out, out_done) = gather(child.stdout.take());
    let (err, err_done) = gather(child.stderr.take());
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s.code(),
            Ok(None) if Instant::now() >= deadline => {
                timed_out = true;
                family.end();
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => {
                family.end();
                let _ = child.kill();
                return Err(e);
            }
        }
    };
    // What it printed is all there as soon as the pipes close, which is at once unless something
    // it started still holds them.
    let until = Instant::now() + LINGER;
    for done in [out_done, err_done] {
        let _ = done.recv_timeout(until.saturating_duration_since(Instant::now()));
    }
    let take = |buf: &std::sync::Mutex<Vec<u8>>| std::mem::take(&mut *buf.lock().unwrap_or_else(|e| e.into_inner()));
    Ok(CmdOutput { status, stdout: take(&out), stderr: take(&err), timed_out })
}

/// How long mom waits for the rest of a program's output once the program itself has finished.
const LINGER: Duration = Duration::from_secs(2);

/// Reads a pipe to its end on a thread of its own, into a buffer that can be taken before the
/// end comes; the receiver hears when it has.
fn gather<P: Read + Send + 'static>(pipe: Option<P>) -> (std::sync::Arc<std::sync::Mutex<Vec<u8>>>, std::sync::mpsc::Receiver<()>) {
    let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let (done, heard) = std::sync::mpsc::channel();
    let into = buf.clone();
    std::thread::spawn(move || {
        if let Some(mut pipe) = pipe {
            let mut chunk = vec![0u8; 16 * 1024];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => into.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(&chunk[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        }
        let _ = done.send(());
    });
    (buf, heard)
}

/// A program mom started, together with whatever it starts in turn, so that all of it can be
/// ended at once. (On Windows, a job; elsewhere there's only the program itself.) Dropping it
/// ends nothing: what a program leaves running on purpose stays.
struct Family {
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}

impl Family {
    fn of(child: &std::process::Child) -> Family {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW};
            // SAFETY: a new, unnamed job; the child's handle is open for the length of the call.
            // If either call fails there's no job, and only the program itself can be ended.
            let job = unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if !job.is_null() && AssignProcessToJobObject(job, child.as_raw_handle() as _) == 0 {
                    windows_sys::Win32::Foundation::CloseHandle(job);
                    std::ptr::null_mut()
                } else {
                    job
                }
            };
            Family { job }
        }
        #[cfg(not(windows))]
        {
            let _ = child;
            Family {}
        }
    }

    /// Ends the program and everything it started.
    fn end(&self) {
        #[cfg(windows)]
        if !self.job.is_null() {
            // SAFETY: the job handle is ours and open.
            unsafe { windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 1) };
        }
    }
}

impl Drop for Family {
    fn drop(&mut self) {
        #[cfg(windows)]
        if !self.job.is_null() {
            // SAFETY: the job handle is ours; closing it leaves what's in the job running.
            unsafe { windows_sys::Win32::Foundation::CloseHandle(self.job) };
        }
    }
}

/// Moves (or renames) `from` to `to` only if nothing is at `to`. The check and the move are one
/// step, done by the system, so something that turns up there in the meantime is never
/// replaced. (`std::fs::rename` replaces a file that's in the way.)
pub fn move_new(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
        let wide = |p: &Path| -> Vec<u16> { long_path(p).as_os_str().encode_wide().chain(Some(0)).collect() };
        let (from, to) = (wide(from), wide(to));
        // SAFETY: both are NUL-terminated and outlive the call. No flags: not onto anything that's there.
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        if to.symlink_metadata().is_ok() {
            return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "something is already there"));
        }
        std::fs::rename(from, to)
    }
}

/// Did a move fail because something is already where the thing was going?
pub fn in_the_way(e: &std::io::Error) -> bool {
    // ERROR_FILE_EXISTS and ERROR_ALREADY_EXISTS.
    e.kind() == std::io::ErrorKind::AlreadyExists || matches!(e.raw_os_error(), Some(80 | 183) if cfg!(windows))
}

/// A path over Windows' usual 260 characters, spelled the way that lifts the limit (`\\?\`).
/// Only a whole, plain path can be: one with a drive letter, backslashes and no `.` or `..`.
#[cfg(windows)]
fn long_path(p: &Path) -> PathBuf {
    use std::path::Component;
    let s = p.to_string_lossy();
    let plain = p.components().all(|c| matches!(c, Component::Prefix(_) | Component::RootDir | Component::Normal(_)));
    let lettered = s.len() > 3 && s.as_bytes()[1] == b':' && s.as_bytes()[2] == b'\\';
    if s.len() >= 240 && plain && lettered && !s.contains('/') {
        return PathBuf::from(format!(r"\\?\{s}"));
    }
    p.to_path_buf()
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

/// A few things to ask of a window by its handle (as a number), in screen pixels. Rectangles are
/// (x, y, width, height).
#[cfg(windows)]
pub mod window {
    use windows_sys::Win32::Foundation::{POINT, RECT};
    use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_CLOAK, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, GetWindowRect, SetWindowPos, ShowWindow, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE};

    const EMPTY: RECT = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    fn shape(r: RECT) -> (i32, i32, i32, i32) {
        (r.left, r.top, r.right - r.left, r.bottom - r.top)
    }

    /// Moves and resizes it in one go, leaving alone which window is in front.
    pub fn place(hwnd: isize, x: i32, y: i32, width: i32, height: i32) -> bool {
        // SAFETY (here and below): a window handle and plain numbers or a struct of ours to fill;
        // Windows checks the handle itself.
        unsafe { SetWindowPos(hwnd as _, 0 as _, x, y, width, height, SWP_NOZORDER | SWP_NOACTIVATE) != 0 }
    }

    /// Its whole rectangle, with the unseen edge Windows keeps around a window to grab it by.
    pub fn whole(hwnd: isize) -> Option<(i32, i32, i32, i32)> {
        let mut r = EMPTY;
        (unsafe { GetWindowRect(hwnd as _, &mut r) } != 0).then(|| shape(r))
    }

    /// The part of it you can see.
    pub fn seen(hwnd: isize) -> Option<(i32, i32, i32, i32)> {
        let mut r = EMPTY;
        let size = std::mem::size_of::<RECT>() as u32;
        (unsafe { DwmGetWindowAttribute(hwnd as _, DWMWA_EXTENDED_FRAME_BOUNDS as u32, std::ptr::addr_of_mut!(r).cast(), size) } == 0).then(|| shape(r))
    }

    /// Where its page is on the screen (what's inside its edges).
    pub fn page(hwnd: isize) -> Option<(i32, i32, i32, i32)> {
        let mut r = EMPTY;
        let mut corner = POINT { x: 0, y: 0 };
        let ok = unsafe { GetClientRect(hwnd as _, &mut r) != 0 && ClientToScreen(hwnd as _, &mut corner) != 0 };
        ok.then(|| (corner.x, corner.y, r.right - r.left, r.bottom - r.top))
    }

    /// Takes it off the screen without closing, hiding or leaving it (it keeps the keyboard, its
    /// place in the taskbar and its place among the windows), or puts it back.
    pub fn cloak(hwnd: isize, on: bool) -> bool {
        let on = i32::from(on);
        unsafe { DwmSetWindowAttribute(hwnd as _, DWMWA_CLOAK as u32, std::ptr::addr_of!(on).cast(), 4) == 0 }
    }

    /// Shows it without making it the window you're in, or hides it.
    pub fn show_quietly(hwnd: isize, on: bool) {
        unsafe { ShowWindow(hwnd as _, if on { SW_SHOWNOACTIVATE } else { SW_HIDE }) };
    }
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

/// Where a folder really is, with every link on the way followed and every name spelled as it
/// is on disk: "C:\Users\you\code" for a Projects folder that is a link to it. None for a place
/// on the network, or one that can't be looked at.
pub fn real_path(p: &std::path::Path) -> Option<std::path::PathBuf> {
    let real = std::fs::canonicalize(p).ok()?;
    let s = real.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        // "\\?\C:\..." is how Windows spells a real local path; "\\?\UNC\..." is a network share.
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => Some(std::path::PathBuf::from(rest)),
        Some(_) => None,
        None if s.starts_with(r"\\") => None,
        None => Some(real),
    }
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
    #[cfg(windows)]
    fn a_system_powershell_path_requires_a_complete_absolute_directory() {
        let directory: Vec<u16> = r"D:\Windows\System32".encode_utf16().chain(Some(0)).collect();
        assert_eq!(powershell_in_system_directory(&directory, directory.len() - 1).unwrap(), r"D:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");
        for len in [0, directory.len(), directory.len() + 1] {
            assert!(powershell_in_system_directory(&directory, len).is_err());
        }
        for invalid in ["relative", r"D:relative", r"\System32", "D:\\Windows\0\\System32"] {
            let directory: Vec<u16> = invalid.encode_utf16().chain(Some(0)).collect();
            assert!(powershell_in_system_directory(&directory, directory.len() - 1).is_err());
        }
        assert!(powershell_in_system_directory(&[0xd800, 0], 1).is_err());
    }

    #[test]
    fn a_move_never_replaces_what_is_there() {
        let dir = std::env::temp_dir().join(format!("mom-move-new-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("folder")).unwrap();
        let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&a, "first").unwrap();
        std::fs::write(&b, "second").unwrap();
        // Onto a file, onto a folder, and a folder onto a file: refused, and both are as they were.
        for (from, to) in [(&a, &b), (&a, &dir.join("folder")), (&dir.join("folder"), &b)] {
            let e = move_new(from, to).unwrap_err();
            assert!(in_the_way(&e), "{e}");
        }
        assert_eq!((std::fs::read_to_string(&a).unwrap(), std::fs::read_to_string(&b).unwrap()), ("first".to_string(), "second".to_string()));
        assert!(dir.join("folder").is_dir());
        // To a free name it's an ordinary move, for a file and for a folder with things in it.
        move_new(&a, &dir.join("c.txt")).unwrap();
        assert!(!a.exists() && std::fs::read_to_string(dir.join("c.txt")).unwrap() == "first");
        std::fs::write(dir.join("folder").join("inside.txt"), "x").unwrap();
        move_new(&dir.join("folder"), &dir.join("moved")).unwrap();
        assert!(dir.join("moved").join("inside.txt").is_file() && !dir.join("folder").exists());
        // Something that isn't there can't be moved, and that isn't "in the way".
        assert!(!in_the_way(&move_new(&dir.join("nothing"), &dir.join("d.txt")).unwrap_err()));
        // A very long path moves like any other.
        let deep = (0..12).fold(dir.join("deep"), |p, i| p.join(format!("a-folder-with-a-long-name-{i:02}")));
        std::fs::create_dir_all(&deep).unwrap();
        assert!(deep.to_string_lossy().len() > 300);
        std::fs::write(deep.join("here.txt"), "deep").unwrap();
        move_new(&deep.join("here.txt"), &deep.join("there.txt")).unwrap();
        assert!(move_new(&dir.join("c.txt"), &deep.join("there.txt")).is_err());
        assert_eq!(std::fs::read_to_string(deep.join("there.txt")).unwrap(), "deep");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// How many programs are running with `mark` somewhere in their command line.
    #[cfg(windows)]
    fn running_with(mark: &str) -> usize {
        let ask = format!("@(Get-CimInstance Win32_Process | Where-Object {{ $_.CommandLine -like '*{mark}*' -and $_.Name -eq 'ping.exe' }}).Count");
        windows_powershell().and_then(|program| run(&program, &["-NoProfile", "-Command", &ask], Duration::from_secs(60)))
            .map(|o| o.text().trim().parse().unwrap_or(99)).unwrap_or(99)
    }

    #[test]
    #[cfg(windows)]
    fn a_time_limit_ends_everything_the_program_started() {
        // cmd starts a second program that holds the same pipes and would run for a minute.
        let mark = format!("127.0.0.{}", 2 + std::process::id() % 200);
        let began = Instant::now();
        let out = run("cmd", &["/d", "/c", &format!("ping -n 60 {mark} & rem")], Duration::from_millis(600)).unwrap();
        assert!(out.timed_out && out.status.is_none());
        assert!(began.elapsed() < Duration::from_secs(5), "took {:?}", began.elapsed());
        assert!(out.text().contains("Pinging"), "what it printed before the limit is kept: {:?}", out.text());
        assert_eq!(running_with(&format!("-n 60 {mark}")), 0, "the program cmd started went with it");
    }

    #[test]
    #[cfg(windows)]
    fn output_left_open_by_something_still_running_isnt_waited_for() {
        // cmd finishes at once; what it started in the background keeps the pipes for a while.
        let mark = format!("127.0.1.{}", 2 + std::process::id() % 200);
        let began = Instant::now();
        let out = run("cmd", &["/d", "/c", &format!("echo done & start /b ping -n 30 {mark}")], Duration::from_secs(60)).unwrap();
        assert!(!out.timed_out && out.status == Some(0));
        assert!(out.text().contains("done"));
        assert!(began.elapsed() < Duration::from_secs(6), "took {:?}", began.elapsed());
        // It wasn't ended either: only a time limit does that.
        assert_eq!(running_with(&format!("-n 30 {mark}")), 1);
    }

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
