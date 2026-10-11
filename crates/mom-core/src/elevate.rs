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
    fn tests_dont_run_elevated() {
        // Not a promise about CI, just that asking doesn't crash.
        let _ = is_elevated();
    }
}
