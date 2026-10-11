//! Running a program as an administrator, for the clean-ups only one can do, and starting an
//! app's own uninstaller. Windows shows its usual "allow this app to make changes?" prompt;
//! mom itself never runs elevated.

use std::path::Path;

/// Runs `exe args` as an administrator, hidden, and waits for it. Its exit code, or why it
/// didn't run (the person said no, say).
pub fn run_as_admin(exe: &Path, args: &str) -> Result<u32, String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_CANCELLED};
        use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
        let (verb, file, params) = (wide("runas"), wide(&exe.to_string_lossy()), wide(args));
        let mut info = shell_info(&verb, &file, &params, 0);
        // SAFETY: every string is NUL-terminated and outlives the call; `info` is ours.
        if unsafe { windows_sys::Win32::UI::Shell::ShellExecuteExW(&mut info) } == 0 {
            // SAFETY: no arguments.
            let e = unsafe { GetLastError() };
            return Err(if e == ERROR_CANCELLED {
                "You said no, so nothing was cleaned.".into()
            } else {
                format!("Windows couldn't run it as administrator (error {e}).")
            });
        }
        if info.hProcess.is_null() {
            return Err("Windows didn't say how it went.".into());
        }
        let mut code = 0u32;
        // SAFETY: the handle is the elevated process's, open until we close it here.
        unsafe {
            WaitForSingleObject(info.hProcess, u32::MAX);
            GetExitCodeProcess(info.hProcess, &mut code);
            CloseHandle(info.hProcess);
        }
        Ok(code)
    }
    #[cfg(not(windows))]
    {
        let _ = (exe, args);
        Err("Only on Windows.".into())
    }
}

/// Starts a program the way double-clicking it would, without waiting (an uninstaller, say).
/// Windows asks for permission first if the program needs it.
pub fn open(exe: &Path, args: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_CANCELLED};
        let (verb, file, params) = (wide("open"), wide(&exe.to_string_lossy()), wide(args));
        let mut info = shell_info(&verb, &file, &params, 1); // SW_SHOWNORMAL
        // SAFETY: as above.
        if unsafe { windows_sys::Win32::UI::Shell::ShellExecuteExW(&mut info) } == 0 {
            // SAFETY: no arguments.
            let e = unsafe { GetLastError() };
            return Err(if e == ERROR_CANCELLED { "You said no.".into() } else { format!("Windows couldn't start it (error {e}).") });
        }
        if !info.hProcess.is_null() {
            // SAFETY: ours to close; we don't wait for it.
            unsafe { CloseHandle(info.hProcess) };
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (exe, args);
        Err("Only on Windows.".into())
    }
}

/// Is this process running as an administrator?
pub fn is_elevated() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: the current process's pseudo-handle; `token` receives a handle we close.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return false;
        }
        let mut e = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        // SAFETY: `e` is the size we say it is.
        let ok = unsafe {
            GetTokenInformation(token, TokenElevation, (&mut e as *mut TOKEN_ELEVATION).cast(), std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut len)
        };
        // SAFETY: opened above.
        unsafe { CloseHandle(token) };
        ok != 0 && e.TokenIsElevated != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// May mom, running as administrator, delete `target`? Only if nothing in its path could have
/// been swapped for a link by an ordinary user: every folder above it (below the drive's root)
/// must be a real folder, owned by Windows or administrators, that ordinary users can't delete,
/// rename or re-permission, in a parent they can't delete things from. Otherwise a program
/// running as you could point the deletion somewhere else (into C:\Windows, say).
///
/// What's inside `target` is removed without following links (see trash::walk), so the target
/// itself may be anyone's.
pub fn admin_may_delete(target: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let refuse = |why: &str| Err(format!("{} {why}, so mom won't delete it as administrator.", target.display()));
        // Folders above the target, deepest first, then their parents (the root's included).
        let above: Vec<&Path> = target.ancestors().skip(1).filter(|a| a.parent().is_some()).collect();
        for dir in &above {
            match std::fs::symlink_metadata(dir) {
                Ok(m) if m.is_dir() && m.file_attributes() & 0x400 == 0 => {}
                Ok(_) => return refuse(&format!("is inside {}, which is a link", dir.display())),
                Err(_) => return refuse(&format!("is inside {}, which mom can't read", dir.display())),
            }
            // DELETE, WRITE_DAC, WRITE_OWNER, GENERIC_ALL on the folder itself.
            if !acl::only_trusted(dir, 0x0001_0000 | 0x0004_0000 | 0x0008_0000 | 0x1000_0000) {
                return refuse(&format!("is inside {}, which other users can change", dir.display()));
            }
            // FILE_DELETE_CHILD (and the rest) on its parent: can't be swapped out from above.
            if let Some(parent) = dir.parent() {
                if !acl::only_trusted(parent, 0x40 | 0x0004_0000 | 0x0008_0000 | 0x1000_0000) {
                    return refuse(&format!("is inside {}, which other users can change", parent.display()));
                }
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = target;
        Err("Only on Windows.".into())
    }
}

#[cfg(windows)]
mod acl {
    use std::path::Path;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        GetAce, ACCESS_ALLOWED_ACE, ACL, DACL_SECURITY_INFORMATION, INHERIT_ONLY_ACE, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        PSID,
    };

    /// Windows itself, administrators and services: who may change system folders.
    fn trusted(sid: &str) -> bool {
        matches!(sid, "S-1-5-18" | "S-1-5-19" | "S-1-5-20" | "S-1-5-32-544" | "S-1-3-0" | "S-1-3-1") || sid.starts_with("S-1-5-80-")
    }

    fn sid_string(sid: PSID) -> Option<String> {
        let mut out: windows_sys::core::PWSTR = std::ptr::null_mut();
        // SAFETY: `sid` points into a security descriptor we hold; `out` receives a LocalAlloc'd string.
        if unsafe { ConvertSidToStringSidW(sid, &mut out) } == 0 || out.is_null() {
            return None;
        }
        // SAFETY: `out` is NUL-terminated.
        let len = (0..).take_while(|&i| unsafe { *out.add(i) } != 0).count();
        // SAFETY: `len` u16s are readable.
        let s = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(out, len) });
        // SAFETY: allocated by ConvertSidToStringSidW.
        unsafe { LocalFree(out.cast()) };
        Some(s)
    }

    /// Is `path` owned by someone trusted, with no one else allowed any of `rights` on it?
    /// Anything mom can't read counts as no.
    pub fn only_trusted(path: &Path, rights: u32) -> bool {
        let w: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
        let mut owner: PSID = std::ptr::null_mut();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `w` is NUL-terminated; the out pointers are ours; `sd` is freed below.
        let rc = unsafe {
            GetNamedSecurityInfoW(
                w.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                std::ptr::null_mut(),
                &mut dacl,
                std::ptr::null_mut(),
                &mut sd,
            )
        };
        if rc != 0 || sd.is_null() {
            return false;
        }
        let ok = (|| {
            if owner.is_null() || !sid_string(owner).is_some_and(|s| trusted(&s)) {
                return false;
            }
            if dacl.is_null() {
                return false; // no list at all: everyone may do anything
            }
            // SAFETY: `dacl` points into `sd`.
            let count = unsafe { (*dacl).AceCount };
            for i in 0..count as u32 {
                let mut ace: *mut core::ffi::c_void = std::ptr::null_mut();
                // SAFETY: `i` is below the count; `ace` points into `sd`.
                if unsafe { GetAce(dacl, i, &mut ace) } == 0 || ace.is_null() {
                    return false;
                }
                let a = ace as *const ACCESS_ALLOWED_ACE;
                // SAFETY: every ACE starts with a header; allowed ACEs (type 0) are ACCESS_ALLOWED_ACE.
                let (kind, flags, mask) = unsafe { ((*a).Header.AceType, (*a).Header.AceFlags as u32, (*a).Mask) };
                if kind != 0 || flags & INHERIT_ONLY_ACE != 0 || mask & rights == 0 {
                    continue; // a deny, only for what's created inside, or not the rights in question
                }
                // SAFETY: the SID starts at SidStart, inside the ACE.
                let sid = unsafe { std::ptr::addr_of!((*a).SidStart) } as PSID;
                if !sid_string(sid).is_some_and(|s| trusted(&s)) {
                    return false;
                }
            }
            true
        })();
        // SAFETY: allocated by GetNamedSecurityInfoW.
        unsafe { LocalFree(sd) };
        ok
    }
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

#[cfg(windows)]
fn shell_info(verb: &[u16], file: &[u16], params: &[u16], show: i32) -> windows_sys::Win32::UI::Shell::SHELLEXECUTEINFOW {
    use windows_sys::Win32::UI::Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: verb.as_ptr(),
        lpFile: file.as_ptr(),
        lpParameters: params.as_ptr(),
        nShow: show,
        ..Default::default()
    }
}

/// Text as hex, to pass on a command line without any quoting trouble.
pub fn to_hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

pub fn from_hex(h: &str) -> Option<String> {
    if !h.len().is_multiple_of(2) {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..h.len()).step_by(2).map(|i| u8::from_str_radix(h.get(i..i + 2)?, 16).ok()).collect();
    String::from_utf8(bytes?).ok()
}

/// Splits a command line as Windows stores it for uninstallers: `"C:\a b\un.exe" /x`,
/// `C:\a b\unins000.exe`, `MsiExec.exe /I{GUID}` (made `/X`, uninstall rather than change).
pub fn split_command(cmd: &str) -> Option<(String, String)> {
    let cmd = cmd.trim();
    let (exe, args) = if let Some(rest) = cmd.strip_prefix('"') {
        let (exe, args) = rest.split_once('"')?;
        (exe.to_string(), args.trim().to_string())
    } else {
        let lower = cmd.to_ascii_lowercase();
        let end = lower.find(".exe").map(|i| i + 4).unwrap_or_else(|| cmd.find(' ').unwrap_or(cmd.len()));
        (cmd[..end].trim().to_string(), cmd[end..].trim().to_string())
    };
    if exe.is_empty() {
        return None;
    }
    let msi = exe.to_ascii_lowercase().ends_with("msiexec.exe") || exe.eq_ignore_ascii_case("msiexec");
    let args = if msi { args.replacen("/I{", "/X{", 1).replacen("/i{", "/X{", 1) } else { args };
    Some((exe, args))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uninstall_commands_split_the_way_windows_runs_them() {
        assert_eq!(split_command(r#""C:\Program Files\Foo\uninst.exe" /x"#), Some((r"C:\Program Files\Foo\uninst.exe".into(), "/x".into())));
        assert_eq!(split_command(r"C:\Program Files\Foo\unins000.exe"), Some((r"C:\Program Files\Foo\unins000.exe".into(), String::new())));
        assert_eq!(split_command("MsiExec.exe /I{1234-ABCD}"), Some(("MsiExec.exe".into(), "/X{1234-ABCD}".into())));
        assert_eq!(
            split_command(r"C:\Users\me\AppData\Local\slack\Update.exe --uninstall -s"),
            Some((r"C:\Users\me\AppData\Local\slack\Update.exe".into(), "--uninstall -s".into()))
        );
        assert_eq!(split_command(""), None);
    }

    #[test]
    fn hex_round_trips() {
        let s = r#"{"ticked":["C:\a b\c"]}"#;
        assert_eq!(from_hex(&to_hex(s)).as_deref(), Some(s));
        assert_eq!(from_hex("zz"), None);
    }

    #[cfg(windows)]
    #[test]
    fn deletes_as_administrator_only_where_nobody_else_can_swap_folders() {
        // Inside a folder you own (Temp): no.
        let mine = std::env::temp_dir().join("mom-admin-check").join("x");
        std::fs::create_dir_all(mine.parent().unwrap()).unwrap();
        let err = admin_may_delete(&mine).unwrap_err();
        assert!(err.contains("other users can change"), "{err}");
        std::fs::remove_dir_all(mine.parent().unwrap()).unwrap();
        // Inside Windows' own folders: yes.
        let windows = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        admin_may_delete(&std::path::Path::new(&windows).join("System32").join("x")).unwrap();
    }

    #[test]
    fn tests_dont_run_elevated() {
        // Not a promise about CI, just that asking doesn't crash.
        let _ = is_elevated();
    }
}
