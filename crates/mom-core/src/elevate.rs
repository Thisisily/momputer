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
/// Is this a real folder or file (not a link) that the person running mom owns? Places anyone on
/// the PC can write to (the top of a second drive, say) hold other people's things too, and
/// links that lead elsewhere. Anything mom can't tell counts as no.
pub fn owned_by_me(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(m) if !m.is_symlink() => {}
        _ => return false,
    }
    #[cfg(windows)]
    {
        static ME: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        match (ME.get_or_init(acl::me), acl::owner(path)) {
            (Some(me), Some(owner)) => *me == owner,
            _ => false,
        }
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// The same question as `owned_by_me`, asked of a folder or file that is already open: the answer
/// is about that very thing, whatever happens to its path afterwards.
pub fn open_and_mine(file: &std::fs::File) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        static ME: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        match (ME.get_or_init(acl::me), acl::owner_of(file.as_raw_handle() as _)) {
            (Some(me), Some(owner)) => *me == owner,
            _ => false,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = file;
        true
    }
}

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

/// What to do when mom won't ask for an administrator from where she is.
pub const INSTALL_FOR_EVERYONE: &str =
    "Install momputer for all users (the installer puts it in Program Files), then mom can clean this as administrator.";

/// May mom ask Windows to run `exe` as administrator? Only if nothing running as you could have
/// swapped it for something else first: `exe` and its folder can be changed only by Windows and
/// administrators (as when installed for all users, in Program Files), nothing above them is a
/// link or could be renamed, and if `app` (momputer itself) is signed, `exe` is signed with the
/// very same certificate. Otherwise any program running as you could replace mom.exe and ride
/// along on your "yes".
pub fn safe_to_elevate(exe: &Path, app: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let refuse = |why: String| Err(format!("{why} {INSTALL_FOR_EVERYONE}"));
        match std::fs::symlink_metadata(exe) {
            Ok(m) if m.is_file() && m.file_attributes() & 0x400 == 0 => {}
            _ => return refuse(format!("{} isn't an ordinary file.", exe.display())),
        }
        let Some(dir) = exe.parent() else { return refuse(format!("{} isn't in a folder.", exe.display())) };
        // Write (on a folder: add a file or a folder), append, DELETE, WRITE_DAC, WRITE_OWNER,
        // GENERIC_WRITE, GENERIC_ALL. In the folder, no one else may add a DLL mom would load or
        // delete what's there (FILE_DELETE_CHILD).
        const CHANGE: u32 = 0x2 | 0x4 | 0x0001_0000 | 0x0004_0000 | 0x0008_0000 | 0x4000_0000 | 0x1000_0000;
        if !acl::only_trusted(exe, CHANGE) {
            return refuse(format!("Anything running as you could swap {}.", exe.display()));
        }
        if !acl::only_trusted(dir, CHANGE | 0x40) {
            return refuse(format!("Anything running as you could put files in {}.", dir.display()));
        }
        if admin_may_delete(exe).is_err() {
            return refuse(format!("Anything running as you could swap a folder {} is in.", dir.display()));
        }
        match (signature(app)?, signature(exe)?) {
            (None, _) => Ok(()), // a build you made yourself: where it is is what counts
            (Some(a), Some(b)) if a == b => Ok(()),
            (Some(_), Some(_)) => Err(format!("{} is signed by someone else than momputer, so mom won't run it as administrator.", exe.display())),
            (Some(_), None) => Err(format!("{} isn't signed but momputer is, so mom won't run it as administrator.", exe.display())),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (exe, app);
        Err("Only on Windows.".into())
    }
}

/// The certificate `file` is signed with, if it's signed (checked by Windows, chain and all).
/// A signature that doesn't check out is an error, not "unsigned".
pub fn signature(file: &Path) -> Result<Option<Vec<u8>>, String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{TRUST_E_NOSIGNATURE, TRUST_E_PROVIDER_UNKNOWN, TRUST_E_SUBJECT_FORM_UNKNOWN};
        use windows_sys::Win32::Security::WinTrust::{
            WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA,
            WINTRUST_FILE_INFO, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
        };
        let path = wide(&file.to_string_lossy());
        let mut info = WINTRUST_FILE_INFO { cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32, pcwszFilePath: path.as_ptr(), ..Default::default() };
        let mut data = WINTRUST_DATA {
            cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
            dwUIChoice: WTD_UI_NONE,
            // Who signed it is the question here, not whether the certificate was revoked since
            // (which would mean asking the internet).
            fdwRevocationChecks: WTD_REVOKE_NONE,
            dwUnionChoice: WTD_CHOICE_FILE,
            dwStateAction: WTD_STATEACTION_VERIFY,
            ..Default::default()
        };
        data.Anonymous.pFile = &mut info;
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        // SAFETY: `data` and `info` (and the path they point to) outlive both calls; the state
        // WinVerifyTrust keeps is released by the second call.
        let rc = unsafe { WinVerifyTrust(-1isize as _, &mut action, (&mut data as *mut WINTRUST_DATA).cast()) };
        let cert = (rc == 0).then(|| {
            // SAFETY: the state is valid until closed below; each pointer is checked before use.
            unsafe {
                let prov = WTHelperProvDataFromStateData(data.hWVTStateData);
                let signer = if prov.is_null() { std::ptr::null_mut() } else { WTHelperGetProvSignerFromChain(prov, 0, 0, 0) };
                if signer.is_null() || (*signer).csCertChain == 0 || (*signer).pasCertChain.is_null() {
                    return None;
                }
                let leaf = (*(*signer).pasCertChain).pCert;
                if leaf.is_null() || (*leaf).pbCertEncoded.is_null() {
                    return None;
                }
                Some(std::slice::from_raw_parts((*leaf).pbCertEncoded, (*leaf).cbCertEncoded as usize).to_vec())
            }
        });
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        // SAFETY: as above.
        unsafe { WinVerifyTrust(-1isize as _, &mut action, (&mut data as *mut WINTRUST_DATA).cast()) };
        match rc {
            0 => cert.flatten().map(Some).ok_or_else(|| format!("Windows couldn't say who signed {}.", file.display())),
            TRUST_E_NOSIGNATURE | TRUST_E_SUBJECT_FORM_UNKNOWN | TRUST_E_PROVIDER_UNKNOWN => Ok(None),
            e => Err(format!("{}'s signature doesn't check out (error {:#x}).", file.display(), e as u32)),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = file;
        Ok(None)
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

    /// Who owns `path`, as a SID string.
    pub fn owner(path: &Path) -> Option<String> {
        let w: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
        let mut owner: PSID = std::ptr::null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `w` is NUL-terminated; the out pointers are ours; `sd` is freed below.
        let rc = unsafe {
            GetNamedSecurityInfoW(w.as_ptr(), SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION, &mut owner, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), &mut sd)
        };
        if rc != 0 || sd.is_null() {
            return None;
        }
        let sid = if owner.is_null() { None } else { sid_string(owner) };
        // SAFETY: allocated by GetNamedSecurityInfoW.
        unsafe { LocalFree(sd) };
        sid
    }

    /// Who owns what this open handle is to, as a SID string.
    pub fn owner_of(handle: windows_sys::Win32::Foundation::HANDLE) -> Option<String> {
        use windows_sys::Win32::Security::Authorization::GetSecurityInfo;
        let mut owner: PSID = std::ptr::null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `handle` is open for the length of the call; the out pointers are ours; `sd` is freed below.
        let rc = unsafe { GetSecurityInfo(handle, SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION, &mut owner, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), &mut sd) };
        if rc != 0 || sd.is_null() {
            return None;
        }
        let sid = if owner.is_null() { None } else { sid_string(owner) };
        // SAFETY: allocated by GetSecurityInfo.
        unsafe { LocalFree(sd) };
        sid
    }

    /// The person running mom, as a SID string.
    pub fn me() -> Option<String> {
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        // SAFETY: plain Win32 calls with a buffer we own; the token handle is closed before returning,
        // and the SID is read while the buffer is alive.
        unsafe {
            let mut token: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return None;
            }
            let mut buf = vec![0u64; 128]; // 1 KB, 8-byte aligned for TOKEN_USER
            let mut len = 0u32;
            let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), (buf.len() * 8) as u32, &mut len);
            CloseHandle(token);
            if ok == 0 {
                return None;
            }
            let user = &*(buf.as_ptr() as *const TOKEN_USER);
            sid_string(user.User.Sid)
        }
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
    fn only_what_you_made_is_yours() {
        let dir = std::env::temp_dir().join(format!("mom-mine-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("project")).unwrap();
        std::fs::write(dir.join("project").join("file.txt"), "x").unwrap();
        assert!(owned_by_me(&dir.join("project")));
        assert!(owned_by_me(&dir.join("project").join("file.txt")));
        assert!(!owned_by_me(&dir.join("not-there")), "what can't be looked at isn't yours");
        #[cfg(windows)]
        {
            let windows = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).unwrap_or_else(|| r"C:\Windows".into());
            assert!(!owned_by_me(&windows), "Windows' own folder belongs to Windows");
            // A link to your own folder is still a link: where it leads can change.
            let link = dir.join("link");
            if std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(dir.join("project")).output().is_ok_and(|o| o.status.success()) {
                assert!(!owned_by_me(&link));
                let _ = std::fs::remove_dir(&link);
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }

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

    #[cfg(windows)]
    #[test]
    fn asks_for_an_administrator_only_for_a_mom_nothing_else_could_have_swapped() {
        // A copy in a folder you own: no.
        let dir = std::env::temp_dir().join("mom-elevate-check");
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("mom.exe");
        std::fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
        let err = safe_to_elevate(&exe, &exe).unwrap_err();
        assert!(err.contains("could swap") && err.ends_with(INSTALL_FOR_EVERYONE), "{err}");
        std::fs::remove_dir_all(&dir).unwrap();
        // Not there at all: no.
        assert!(safe_to_elevate(&exe, &exe).unwrap_err().contains("isn't an ordinary file"));
    }

    #[cfg(windows)]
    #[test]
    fn tells_signed_from_unsigned_from_tampered() {
        // This test program isn't signed, and a text file can't be.
        assert_eq!(signature(&std::env::current_exe().unwrap()), Ok(None));
        let dir = std::env::temp_dir().join("mom-signature-check");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("note.txt"), "hello").unwrap();
        assert_eq!(signature(&dir.join("note.txt")), Ok(None));
        // Edge comes signed wherever it's installed. Change one byte and it no longer checks out.
        let edge = std::path::PathBuf::from(std::env::var("ProgramFiles(x86)").unwrap_or_default()).join(r"Microsoft\Edge\Application\msedge.exe");
        if let Ok(mut bytes) = std::fs::read(&edge) {
            assert!(signature(&edge).unwrap().is_some_and(|c| !c.is_empty()));
            let mid = bytes.len() / 2;
            bytes[mid] ^= 0xff;
            std::fs::write(dir.join("msedge.exe"), bytes).unwrap();
            let err = signature(&dir.join("msedge.exe")).unwrap_err();
            assert!(err.contains("doesn't check out"), "{err}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tests_dont_run_elevated() {
        // Not a promise about CI, just that asking doesn't crash.
        let _ = is_elevated();
    }
}
