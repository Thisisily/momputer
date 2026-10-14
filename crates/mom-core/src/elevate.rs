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
        acl::owner(path).is_some_and(|owner| mine(&owner))
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// Is this owner the person running mom? Their own account, or whoever Windows names as the
/// owner of what they create right now: a member of Administrators running elevated (an
/// administrator's terminal, a build machine) makes files that belong to the Administrators
/// group, and those are as much theirs.
#[cfg(windows)]
fn mine(owner: &str) -> bool {
    static ME: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    ME.get_or_init(acl::me).iter().any(|sid| sid == owner)
}

/// The same question as `owned_by_me`, asked of a folder or file that is already open: the answer
/// is about that very thing, whatever happens to its path afterwards.
pub fn open_and_mine(file: &std::fs::File) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        acl::owner_of(file.as_raw_handle() as _).is_some_and(|owner| mine(&owner))
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

/// A program mom is about to run, held open: until this is dropped nothing can write to it,
/// delete it, rename it, or rename a folder it's in.
#[derive(Debug)]
pub struct Held {
    _file: std::fs::File,
    /// Where it really is, with no links on the way: the path to run.
    pub path: String,
}

/// Opens `program` for mom to run for you, if Windows says it's signed by `org` (of `country`):
/// the organisation named in a certificate Windows trusts, chain and all. A name found on the
/// PATH could otherwise be anything somebody dropped into a folder that's searched first.
///
/// What Windows checks is the open file itself, and the file stays open in the `Held`, so what
/// runs is what was checked: nothing can swap it in between. Who else may write where it lies
/// doesn't come into it, with one exception: a program loads what lies next to it, so its
/// folder has to be yours or Windows', and not one a crowd (Everyone, Users, whoever is signed
/// in) can put things in. An account let into that folder by name still can, as it can for the
/// same program run from your own terminal.
pub fn hold_signed(program: &Path, org: &str, country: &str) -> Result<Held, String> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let name = program.display();
        if !program.is_absolute() {
            return Err(format!("{name} isn't a whole path, so mom won't run it."));
        }
        // Others may read it meanwhile (Windows has to, to run it); nobody may write to it,
        // delete it or rename it.
        let file = std::fs::OpenOptions::new().read(true).share_mode(1).open(program).map_err(|e| format!("Mom couldn't open {name}: {e}"))?;
        let path = match (file.metadata(), final_path(&file)) {
            (Ok(m), Some(p)) if m.is_file() && p.to_ascii_lowercase().ends_with(".exe") => p,
            _ => return Err(format!("{name} isn't a program on this PC, so mom won't run it.")),
        };
        match signer(Path::new(&path), Some(&file))? {
            Some(s) if s.org == org && s.country == country => {}
            _ => return Err(format!("{name} isn't signed by {org}, so mom won't run it.")),
        }
        // Add a file or a folder, DELETE, WRITE_DAC, WRITE_OWNER, GENERIC_WRITE, GENERIC_ALL,
        // and deleting what's in it (FILE_DELETE_CHILD).
        const CHANGE: u32 = 0x2 | 0x4 | 0x0001_0000 | 0x0004_0000 | 0x0008_0000 | 0x4000_0000 | 0x1000_0000 | 0x40;
        if !Path::new(&path).parent().is_some_and(|dir| acl::yours(dir, CHANGE)) {
            return Err(format!("{name} is in a folder anybody on this PC can put things in, so mom won't run it."));
        }
        Ok(Held { _file: file, path })
    }
    #[cfg(not(windows))]
    {
        let _ = (program, org, country);
        Err("Only on Windows.".into())
    }
}

/// Where an open file really is ("C:\..."), every link on the way followed. Nothing for a file
/// that isn't on one of this PC's lettered drives (one on another computer, say).
#[cfg(windows)]
fn final_path(file: &std::fs::File) -> Option<String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
    let mut buf = vec![0u16; 1024];
    // SAFETY: the handle is open for the length of the call; `buf` is ours, and its length is given.
    let n = unsafe { GetFinalPathNameByHandleW(file.as_raw_handle() as _, buf.as_mut_ptr(), buf.len() as u32, 0) } as usize;
    if n == 0 || n >= buf.len() {
        return None;
    }
    let full = String::from_utf16(&buf[..n]).ok()?;
    let plain = full.strip_prefix(r"\\?\")?;
    let b = plain.as_bytes();
    (b.len() > 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\').then(|| plain.to_string())
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
    Ok(signer(file, None)?.map(|s| s.cert))
}

/// Who signed a file: the certificate, and the organisation and country it names.
#[derive(Clone, Debug, Default, PartialEq)]
struct Signer {
    cert: Vec<u8>,
    org: String,
    country: String,
}

/// Who signed `file`, if it's signed (checked by Windows, chain and all). Given the file
/// `open`, Windows reads it through that handle: what's checked is the file that's held open,
/// whatever its path leads to by then.
fn signer(file: &Path, open: Option<&std::fs::File>) -> Result<Option<Signer>, String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{TRUST_E_NOSIGNATURE, TRUST_E_PROVIDER_UNKNOWN, TRUST_E_SUBJECT_FORM_UNKNOWN};
        use windows_sys::Win32::Security::Cryptography::{CertGetNameStringW, CERT_NAME_ATTR_TYPE};
        use windows_sys::Win32::Security::WinTrust::{
            WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA,
            WINTRUST_FILE_INFO, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
        };
        let path = wide(&file.to_string_lossy());
        let mut info = WINTRUST_FILE_INFO { cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32, pcwszFilePath: path.as_ptr(), ..Default::default() };
        if let Some(open) = open {
            use std::os::windows::io::AsRawHandle;
            info.hFile = open.as_raw_handle() as _;
        }
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
                // One part of the name the certificate was issued to, by its number ("2.5.4.10").
                let named = |oid: &[u8]| {
                    let mut buf = [0u16; 256];
                    let n = CertGetNameStringW(leaf, CERT_NAME_ATTR_TYPE, 0, oid.as_ptr().cast(), buf.as_mut_ptr(), buf.len() as u32) as usize;
                    String::from_utf16_lossy(&buf[..n.saturating_sub(1).min(buf.len())])
                };
                Some(Signer {
                    cert: std::slice::from_raw_parts((*leaf).pbCertEncoded, (*leaf).cbCertEncoded as usize).to_vec(),
                    org: named(b"2.5.4.10\0"),
                    country: named(b"2.5.4.6\0"),
                })
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
        let _ = (file, open);
        Ok(None)
    }
}

#[cfg(windows)]
mod acl {
    use std::path::Path;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        GetAce, IsValidSid, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, INHERIT_ONLY_ACE, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
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

    /// The person running mom, as SID strings: their own account, and the owner Windows gives
    /// to what they create (the same account, or Administrators when they run elevated).
    pub fn me() -> Vec<String> {
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        use windows_sys::Win32::Security::{GetTokenInformation, TokenOwner, TokenUser, TOKEN_OWNER, TOKEN_QUERY, TOKEN_USER};
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        let mut out = Vec::new();
        // SAFETY: plain Win32 calls with buffers we own; the token handle is closed before returning,
        // and each SID is read while its buffer is alive.
        unsafe {
            let mut token: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return out;
            }
            let mut buf = vec![0u64; 128]; // 1 KB, 8-byte aligned for both structures
            let mut len = 0u32;
            if GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), (buf.len() * 8) as u32, &mut len) != 0 {
                out.extend(sid_string((*(buf.as_ptr() as *const TOKEN_USER)).User.Sid));
            }
            let mut buf = vec![0u64; 128];
            if GetTokenInformation(token, TokenOwner, buf.as_mut_ptr().cast(), (buf.len() * 8) as u32, &mut len) != 0 {
                out.extend(sid_string((*(buf.as_ptr() as *const TOKEN_OWNER)).Owner));
            }
            CloseHandle(token);
        }
        out.dedup();
        out
    }

    /// One account or group this PC (or its domain) knows by name, rather than a crowd: not
    /// Everyone, Users or whoever is signed in, and not the groups every account is put in
    /// (the guest, "None" or Domain Users, Domain Guests, Domain Computers).
    pub fn named(sid: &str) -> bool {
        sid.starts_with("S-1-5-21-") && !["-501", "-513", "-514", "-515"].iter().any(|crowd| sid.ends_with(crowd))
    }

    /// Is `path` owned by someone trusted, with no one else allowed any of `rights` on it?
    /// Anything mom can't read counts as no.
    pub fn only_trusted(path: &Path, rights: u32) -> bool {
        only(path, rights, &trusted, &trusted)
    }

    /// Is `path` owned by the person running mom (or someone trusted), with none of `rights` on
    /// it for a crowd? Accounts let in by name may have them.
    pub fn yours(path: &Path, rights: u32) -> bool {
        let me = me();
        let mine = |sid: &str| trusted(sid) || me.iter().any(|m| m == sid);
        !me.is_empty() && only(path, rights, &mine, &|sid| mine(sid) || named(sid))
    }

    /// Whether one ACE can be ruled out as a grant of `rights` to an untrusted account.
    ///
    /// SAFETY: `ace` must point to a DWORD-aligned ACE in a live allocation containing both
    /// its header and the `AceSize` bytes it declares, as returned by a successful GetAce.
    unsafe fn ace_is_restricted(ace: *const ACE_HEADER, rights: u32, allowed: &dyn Fn(&str) -> bool) -> bool {
        // Read only the common header until the type and its minimum size are established.
        let header = unsafe { *ace };
        if (header.AceSize as usize) < std::mem::size_of::<ACE_HEADER>() || header.AceSize % 4 != 0 {
            return false;
        }
        if header.AceFlags as u32 & INHERIT_ONLY_ACE != 0 {
            return true; // affects children, not this object
        }
        match header.AceType {
            1 | 6 | 10 | 12 => return true, // basic, object, callback and callback-object deny ACEs cannot grant access
            0 => {}, // ACCESS_ALLOWED_ACE
            _ => return false, // object/callback grants and unfamiliar types must not silently pass
        }
        const SID_HEADER_BYTES: usize = 8;
        let sid_offset = std::mem::offset_of!(ACCESS_ALLOWED_ACE, SidStart);
        if (header.AceSize as usize) < sid_offset + SID_HEADER_BYTES {
            return false;
        }
        // SAFETY: this is a basic allow ACE, large enough for its mask and fixed SID header.
        let a = ace.cast::<ACCESS_ALLOWED_ACE>();
        let (mask, sid_start) = unsafe { ((*a).Mask, (*a).SidStart.to_le_bytes()) };
        if mask & rights == 0 {
            return true;
        }
        // A SID contains an eight-byte header followed by SubAuthorityCount DWORDs. Bound
        // the entire SID by this ACE before Windows is asked to validate or stringify it.
        let sid_bytes = SID_HEADER_BYTES + usize::from(sid_start[1]) * 4;
        if sid_bytes > header.AceSize as usize - sid_offset {
            return false;
        }
        let sid = unsafe { std::ptr::addr_of!((*a).SidStart) } as PSID;
        // SAFETY: the complete SID is inside the still-live, aligned ACE allocation.
        (unsafe { IsValidSid(sid) != 0 }) && sid_string(sid).is_some_and(|s| allowed(&s))
    }

    /// Is `path` owned by an account `owner` passes, with any of `rights` on it only for
    /// accounts `allowed` passes? (Both are asked with a SID string.)
    fn only(path: &Path, rights: u32, owner_ok: &dyn Fn(&str) -> bool, allowed: &dyn Fn(&str) -> bool) -> bool {
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
            if owner.is_null() || !sid_string(owner).is_some_and(|s| owner_ok(&s)) {
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
                // SAFETY: successful GetAce returned an aligned ACE inside the descriptor;
                // `sd` remains allocated until every ACE and its SID have been inspected.
                if !unsafe { ace_is_restricted(ace.cast(), rights, allowed) } {
                    return false;
                }
            }
            true
        })();
        // SAFETY: allocated by GetNamedSecurityInfoW.
        unsafe { LocalFree(sd) };
        ok
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        // A DWORD-aligned synthetic basic allow ACE, containing one complete SID. No file's
        // security descriptor or permissions are changed by these parser regressions.
        fn ace(kind: u8, flags: u8, mask: u32, authority: u8, account: u32) -> [u32; 5] {
            [u32::from_le_bytes([kind, flags, 20, 0]), mask, u32::from_le_bytes([1, 1, 0, 0]),
                u32::from_le_bytes([0, 0, 0, authority]), account]
        }

        fn restricted(ace: &[u32; 5]) -> bool {
            // SAFETY: the owned array is DWORD-aligned and holds the complete declared ACE;
            // malformed fixtures keep the declared ACE within that allocation.
            unsafe { ace_is_restricted(ace.as_ptr().cast(), 2, &trusted) }
        }

        #[test]
        fn basic_grants_are_checked_and_denies_cannot_grant_access() {
            assert!(restricted(&ace(0, 0, 2, 5, 18)), "LocalSystem may write");
            assert!(!restricted(&ace(0, 0, 2, 1, 0)), "Everyone must not be allowed to write");
            assert!(restricted(&ace(0, 0, 1, 1, 0)), "Everyone may read without receiving write rights");
            for kind in [1, 6, 10, 12] {
                assert!(restricted(&ace(kind, 0, 2, 1, 0)), "deny type {kind} cannot grant access");
            }
            assert!(restricted(&ace(0, INHERIT_ONLY_ACE as u8, 2, 1, 0)), "inherit-only grants do not affect this object");
        }

        #[test]
        fn unsupported_grants_fail_closed_without_assuming_a_basic_allow_layout() {
            for kind in [4, 5, 9, 11, 255] {
                let mut entry = ace(kind, 0, 2, 1, 0);
                entry[0] = u32::from_le_bytes([kind, 0, 4, 0]);
                assert!(!restricted(&entry), "unsupported type {kind} must not bypass the ACL check");
            }
        }

        #[test]
        fn short_or_invalid_basic_allow_sids_fail_closed() {
            for size in [0, 4, 8, 12, 16, 18] {
                let mut entry = ace(0, 0, 2, 5, 18);
                entry[0] = u32::from_le_bytes([0, 0, size, 0]);
                assert!(!restricted(&entry), "size {size} cannot hold this ACE and its complete SID");
            }
            let mut entry = ace(0, 0, 2, 5, 18);
            entry[2] = u32::from_le_bytes([1, 15, 0, 0]);
            assert!(!restricted(&entry), "a SID must not run into the next ACE");
            entry[2] = u32::from_le_bytes([0, 1, 0, 0]);
            assert!(!restricted(&entry), "an invalid SID revision must not be accepted");
        }
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

    /// A folder of the test's own under the temp folder, with only the person running the tests
    /// let in, whoever the temp folder itself lets in.
    #[cfg(windows)]
    fn own_folder(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("mom-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert!(icacls(&root, &["/inheritance:r", "/grant:r", &format!("*{}:(OI)(CI)F", acl::me()[0])]));
        root
    }

    #[cfg(windows)]
    fn icacls(path: &Path, args: &[&str]) -> bool {
        let mut all = vec![path.to_str().unwrap()];
        all.extend(args);
        crate::util::run("icacls", &all, std::time::Duration::from_secs(20)).map(|o| o.ok()).unwrap_or(false)
    }

    /// Edge comes signed (by Microsoft Corporation, US) wherever it's installed.
    #[cfg(windows)]
    fn edge() -> Option<std::path::PathBuf> {
        let edge = std::path::PathBuf::from(std::env::var("ProgramFiles(x86)").unwrap_or_default()).join(r"Microsoft\Edge\Application\msedge.exe");
        edge.is_file().then_some(edge)
    }

    #[test]
    #[cfg(windows)]
    fn a_program_is_run_only_if_its_signer_is_who_it_should_be() {
        // This test program isn't signed; a path that isn't whole, a folder and nothing at all aren't programs.
        let me = std::env::current_exe().unwrap();
        let refused = |p: &Path| hold_signed(p, "Microsoft Corporation", "US").unwrap_err();
        assert!(refused(&me).contains("isn't signed by Microsoft Corporation"), "{}", refused(&me));
        assert!(refused(Path::new("msedge.exe")).contains("isn't a whole path"));
        assert!(refused(me.parent().unwrap()).contains("won't run it") || refused(me.parent().unwrap()).contains("couldn't open"));
        assert!(refused(&me.with_file_name("nothing-here.exe")).contains("couldn't open"));
        let Some(edge) = edge() else { return };
        let root = own_folder("signed");
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let exe = root.join("bin").join("claude.exe");
        std::fs::copy(&edge, &exe).unwrap();
        // Signed by who it should be: held, and the path to run is where it really is.
        let held = hold_signed(&exe, "Microsoft Corporation", "US").unwrap();
        assert!(Path::new(&held.path).is_absolute() && !held.path.starts_with(r"\\") && held.path.to_lowercase().ends_with(r"\bin\claude.exe"), "{}", held.path);
        // While it's held it can be read (Windows has to, to run it) and nothing else: not written
        // to, deleted or renamed, and neither can the folders it's in be renamed.
        assert!(std::fs::File::open(&exe).is_ok());
        assert!(std::fs::OpenOptions::new().write(true).open(&exe).is_err());
        assert!(std::fs::remove_file(&exe).is_err());
        assert!(std::fs::rename(&exe, root.join("bin").join("other.exe")).is_err());
        assert!(std::fs::rename(root.join("bin"), root.join("elsewhere")).is_err());
        assert!(std::fs::rename(&root, root.with_extension("moved")).is_err());
        assert!(exe.is_file() && signature(&exe).unwrap().is_some());
        drop(held);
        assert!(std::fs::OpenOptions::new().write(true).open(&exe).is_ok(), "let go of once it's dropped");
        // Signed, but not by who it should be: the organisation and the country both count.
        for (org, country) in [("Anthropic, PBC", "US"), ("Microsoft Corporation", "DE"), ("Microsoft", "US"), ("", "")] {
            let why = hold_signed(&exe, org, country).unwrap_err();
            assert!(why.contains("isn't signed by"), "{why}");
        }
        // Reached through a link, it's the real file that's held and its real path that's run.
        let link = root.join("link");
        if junction::create(root.join("bin"), &link).is_ok() {
            let held = hold_signed(&link.join("claude.exe"), "Microsoft Corporation", "US").unwrap();
            assert!(held.path.to_lowercase().ends_with(r"\bin\claude.exe"), "{}", held.path);
            drop(held);
            let _ = junction::delete(&link);
            let _ = std::fs::remove_dir(&link);
        }
        // One changed byte and Windows no longer vouches for it.
        let mut bytes = std::fs::read(&exe).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xff;
        std::fs::write(&exe, bytes).unwrap();
        assert!(hold_signed(&exe, "Microsoft Corporation", "US").unwrap_err().contains("doesn't check out"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn a_signed_program_isnt_run_from_a_folder_anybody_can_put_things_in() {
        let Some(edge) = edge() else { return };
        let root = own_folder("crowd");
        // A program loads what lies next to it: in a folder everyone on the PC may put things in,
        // even the right program isn't run.
        for (name, crowd) in [("everyone", "S-1-1-0"), ("users", "S-1-5-32-545"), ("signed-in", "S-1-5-11")] {
            let open = root.join(name);
            std::fs::create_dir_all(&open).unwrap();
            assert!(icacls(&open, &["/grant", &format!("*{crowd}:(OI)(CI)M")]), "{name}");
            std::fs::copy(&edge, open.join("claude.exe")).unwrap();
            let why = hold_signed(&open.join("claude.exe"), "Microsoft Corporation", "US").unwrap_err();
            assert!(why.contains("anybody on this PC can put things in"), "{name}: {why}");
        }
        // A crowd that may only look and run is no trouble.
        let shown = root.join("shown");
        std::fs::create_dir_all(&shown).unwrap();
        assert!(icacls(&shown, &["/grant", "*S-1-5-32-545:(OI)(CI)RX"]));
        std::fs::copy(&edge, shown.join("claude.exe")).unwrap();
        assert!(hold_signed(&shown.join("claude.exe"), "Microsoft Corporation", "US").is_ok());
        // One account or group let in by name isn't a crowd; the groups everyone is in are.
        assert!(acl::named("S-1-5-21-1111-2222-3333-1004"));
        for crowd in ["S-1-1-0", "S-1-5-11", "S-1-5-32-545", "S-1-5-4", "S-1-15-2-1", "S-1-5-21-1111-2222-3333-513", "S-1-5-21-1111-2222-3333-501"] {
            assert!(!acl::named(crowd), "{crowd}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn only_what_you_made_is_yours() {
        let dir = std::env::temp_dir().join(format!("mom-mine-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("project")).unwrap();
        std::fs::write(dir.join("project").join("file.txt"), "x").unwrap();
        // Running as administrator (a build machine does), what you make belongs to the
        // Administrators group: still yours.
        #[cfg(windows)]
        let who = format!("its owner is {:?}, you are {:?}", acl::owner(&dir.join("project")), acl::me());
        #[cfg(not(windows))]
        let who = "";
        assert!(owned_by_me(&dir.join("project")), "{who}");
        assert!(owned_by_me(&dir.join("project").join("file.txt")), "{who}");
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
