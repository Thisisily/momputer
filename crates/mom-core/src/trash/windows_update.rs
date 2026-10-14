//! Read-only, fail-closed protection for Windows servicing files. A clean registry alone
//! cannot establish that there are no queued updates: also ask WUA's local catalog. Never
//! initiate an online scan, download, installation, restart, or service-control operation.

// Unit tests inject state instead of touching the host's registry, process list or WUA.
#![cfg_attr(test, allow(dead_code))]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum State {
    Clear,
    Pending,
    Unknown,
}

impl State {
    pub(super) fn reason(self) -> Option<&'static str> {
        match self {
            Self::Clear => None,
            Self::Pending => Some("Kept while Windows has updates, setup work, or a restart pending. Finish Windows Update and restart before cleaning these files."),
            Self::Unknown => Some("Kept because Mom couldn't verify Windows Update's status. Check Windows Update and try again later."),
        }
    }
}

fn combined(states: impl IntoIterator<Item = State>) -> State {
    let mut result = State::Clear;
    for state in states {
        match state {
            State::Pending => return State::Pending,
            State::Unknown => result = State::Unknown,
            State::Clear => {}
        }
    }
    result
}

/// Includes services in session zero. Names are deliberately conservative: a setup or
/// servicing process is enough reason to postpone deleting Windows' working files.
fn process_state(names: Option<&std::collections::HashSet<String>>) -> State {
    let Some(names) = names.filter(|names| !names.is_empty()) else { return State::Unknown };
    if names.iter().any(|name| UPDATE_PROCESSES.iter().any(|known| name.eq_ignore_ascii_case(known))) {
        State::Pending
    } else {
        State::Clear
    }
}

const UPDATE_PROCESSES: &[&str] = &[
    "setup.exe", "setuphost.exe", "setupprep.exe", "setupplatform.exe", "setupcl.exe",
    "windows10upgraderapp.exe", "windows10upgrade.exe", "windows11upgrade.exe",
    "windows11installationassistant.exe", "windowsupdatebox.exe", "updateassistant.exe",
    "updateassistantcheck.exe", "tiworker.exe", "trustedinstaller.exe", "dism.exe",
    "dismhost.exe", "mousocoreworker.exe", "usocoreworker.exe", "usoclient.exe",
    "wuauclt.exe", "wusa.exe", "musnotification.exe", "esdclient.exe",
];

/// The complete check can take up to eight seconds for WUA plus process/pipe cleanup.
/// Known native blockers short-circuit it, so a pending reboot does not launch PowerShell.
/// Do not reuse a Clear result for a later cleanup; pending state can change after a scan.
pub(super) fn probe() -> State {
    #[cfg(windows)]
    {
        let before = probe_fast();
        if before != State::Clear {
            return before;
        }
        let catalog = os::catalog_state();
        if catalog != State::Clear {
            return catalog;
        }
        // The local-catalog query takes time. Catch servicing that began during that query.
        probe_fast()
    }
    #[cfg(not(windows))]
    { State::Unknown }
}

/// Cheap fresh registry/process guard for the last moment before a mutation. This is
/// supplemental to a full probe in the same cleanup, never evidence about WUA's queue.
pub(super) fn probe_fast() -> State {
    #[cfg(windows)]
    {
        combined([os::registry_state(), process_state(crate::procs::running_names_now().as_ref())])
    }
    #[cfg(not(windows))]
    { State::Unknown }
}

fn catalog_output(ok: bool, output: &str) -> State {
    if !ok {
        return State::Unknown;
    }
    match output.trim() {
        "MOM_UPDATE_CLEAR_V1" => State::Clear,
        "MOM_UPDATE_PENDING_V1" => State::Pending,
        _ => State::Unknown,
    }
}

#[cfg(windows)]
mod os {
    use super::{catalog_output, combined, State};
    use crate::util::{run, wide};
    use std::time::Duration;
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE,
        KEY_QUERY_VALUE, KEY_WOW64_64KEY, REG_DWORD, REG_MULTI_SZ, REG_SZ,
    };

    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: this is the successful open's handle, closed exactly once.
            unsafe { RegCloseKey(self.0) };
        }
    }

    /// Absence is distinct from access denied and all other registry failures.
    fn open(path: &str) -> Result<Option<Key>, ()> {
        let path = wide(path);
        let mut key = std::ptr::null_mut();
        // SAFETY: path is NUL-terminated and key is a valid output pointer.
        let result = unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, path.as_ptr(), 0, KEY_QUERY_VALUE | KEY_WOW64_64KEY, &mut key) };
        match result {
            0 => Ok(Some(Key(key))),
            ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(None),
            _ => Err(()),
        }
    }

    fn exists(path: &str) -> State {
        match open(path) {
            Ok(Some(_)) => State::Pending,
            Ok(None) => State::Clear,
            Err(()) => State::Unknown,
        }
    }

    fn value(path: &str, name: &str, multi: bool) -> State {
        let key = match open(path) {
            Ok(Some(key)) => key,
            Ok(None) => return State::Clear,
            Err(()) => return State::Unknown,
        };
        let name = wide(name);
        let mut kind = 0;
        let mut len = 0;
        // SAFETY: this first query asks for the type and byte count without a data buffer.
        let result = unsafe { RegQueryValueExW(key.0, name.as_ptr(), std::ptr::null(), &mut kind, std::ptr::null_mut(), &mut len) };
        if result == ERROR_FILE_NOT_FOUND {
            return State::Clear;
        }
        if result != 0 || len > 1_048_576 {
            return State::Unknown;
        }
        let mut data = vec![0u8; len as usize];
        // SAFETY: the buffer is exactly the reported length. Growth produces an error,
        // which fails closed; shrinking is limited by the returned length below.
        let result = unsafe { RegQueryValueExW(key.0, name.as_ptr(), std::ptr::null(), &mut kind, data.as_mut_ptr(), &mut len) };
        if result != 0 || len as usize > data.len() {
            return State::Unknown;
        }
        data.truncate(len as usize);
        registry_value(kind, &data, multi)
    }

    fn registry_value(kind: u32, data: &[u8], multi: bool) -> State {
        let pending = if multi && kind == REG_MULTI_SZ && data.len().is_multiple_of(2) {
            // An empty or all-NUL multi-string has no pending path operations. A truncated
            // nonempty list still blocks cleanup; it must never be interpreted as clear.
            Some(data.iter().any(|byte| *byte != 0))
        } else if !multi && kind == REG_DWORD && data.len() == 4 {
            Some(u32::from_le_bytes(data.try_into().unwrap()) != 0)
        } else if !multi && kind == REG_SZ && data.len().is_multiple_of(2) {
            let units: Vec<_> = data.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
            String::from_utf16(&units).ok().and_then(|s| s.trim_end_matches('\0').trim().parse::<u32>().ok()).map(|n| n != 0)
        } else {
            None
        };
        match pending {
            Some(true) => State::Pending,
            Some(false) => State::Clear,
            None => State::Unknown,
        }
    }

    pub(super) fn registry_state() -> State {
        let keys = [
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired",
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending",
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootInProgress",
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\PackagesPending",
        ];
        let values = [
            (r"SYSTEM\Setup", "SystemSetupInProgress", false),
            (r"SYSTEM\Setup", "OOBEInProgress", false),
            (r"SYSTEM\Setup", "UpgradeInProgress", false),
            (r"SOFTWARE\Microsoft\Updates", "UpdateExeVolatile", false),
            (r"SYSTEM\CurrentControlSet\Control\Session Manager", "PendingFileRenameOperations", true),
            (r"SYSTEM\CurrentControlSet\Control\Session Manager", "PendingFileRenameOperations2", true),
        ];
        combined(keys.into_iter().map(exists).chain(values.into_iter().map(|(path, name, multi)| value(path, name, multi))))
    }

    // Online=false limits Search to locally cached update metadata. Automatic WUA upgrades
    // are disabled too. No download/installer methods or update service controls are used.
    // A partial Search result (SucceededWithErrors) cannot establish a clear queue.
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
try {
    $info = New-Object -ComObject Microsoft.Update.SystemInfo
    if ($info.RebootRequired) { [Console]::Out.WriteLine('MOM_UPDATE_PENDING_V1'); exit 0 }
    $session = New-Object -ComObject Microsoft.Update.Session
    $searcher = $session.CreateUpdateSearcher()
    $searcher.Online = $false
    $searcher.CanAutomaticallyUpgradeService = $false
    $result = $searcher.Search('IsInstalled=0 and IsHidden=0')
    if ($result.ResultCode -ne 2 -or $result.Warnings.Count -ne 0) { exit 1 }
    if ($result.Updates.Count -gt 0) { [Console]::Out.WriteLine('MOM_UPDATE_PENDING_V1') }
    else { [Console]::Out.WriteLine('MOM_UPDATE_CLEAR_V1') }
} catch { exit 1 }
"#;

    pub(super) fn catalog_state() -> State {
        let mut windows = [0u16; 32_768];
        // SAFETY: the owned output buffer and its length are valid for the call.
        let len = unsafe { windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW(windows.as_mut_ptr(), windows.len() as u32) } as usize;
        if len == 0 || len >= windows.len() {
            return State::Unknown;
        }
        let Ok(windows) = String::from_utf16(&windows[..len]) else { return State::Unknown };
        // Resolve the Windows binary explicitly, without PATH, an environment override,
        // profiles, script files, or an ExecutionPolicy override.
        let powershell = std::path::PathBuf::from(windows).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        let Some(powershell) = powershell.to_str() else { return State::Unknown };
        match run(powershell, &["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", SCRIPT], Duration::from_secs(8)) {
            Ok(output) => catalog_output(output.ok() && output.stderr.is_empty(), &output.text()),
            Err(_) => State::Unknown,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn registry_values_preserve_pending_and_unknown() {
            assert_eq!(registry_value(REG_DWORD, &0u32.to_le_bytes(), false), State::Clear);
            assert_eq!(registry_value(REG_DWORD, &1u32.to_le_bytes(), false), State::Pending);
            assert_eq!(registry_value(REG_DWORD, &[0, 0], false), State::Unknown);
            assert_eq!(registry_value(REG_MULTI_SZ, &[0, 0, 0, 0], true), State::Clear);
            assert_eq!(registry_value(REG_MULTI_SZ, &[97, 0, 0, 0], true), State::Pending);
            assert_eq!(registry_value(REG_MULTI_SZ, &[0], true), State::Unknown);
            assert_eq!(registry_value(REG_DWORD, &0u32.to_le_bytes(), true), State::Unknown);
            assert_eq!(registry_value(REG_SZ, &[48, 0, 0, 0], false), State::Clear);
            assert_eq!(registry_value(REG_SZ, &[49, 0, 0, 0], false), State::Pending);
            assert_eq!(registry_value(REG_SZ, &[120, 0, 0, 0], false), State::Unknown);
        }

        #[test]
        fn catalog_script_is_offline_and_requires_complete_success() {
            assert!(SCRIPT.contains("$searcher.Online = $false"));
            assert!(SCRIPT.contains("$searcher.CanAutomaticallyUpgradeService = $false"));
            assert!(SCRIPT.contains("$result.ResultCode -ne 2"));
            assert!(SCRIPT.contains("$result.Warnings.Count -ne 0"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_all_clear_facts_allow_cleanup() {
        assert_eq!(combined([State::Clear, State::Clear]), State::Clear);
        assert_eq!(combined([State::Clear, State::Unknown]), State::Unknown);
        assert_eq!(combined([State::Unknown, State::Pending]), State::Pending);
        assert_eq!(combined([State::Pending, State::Unknown]), State::Pending);
        assert!(State::Pending.reason().is_some());
        assert!(State::Unknown.reason().is_some());
        assert!(State::Clear.reason().is_none());
    }

    #[test]
    fn servicing_processes_block_regardless_of_case_or_session() {
        let names = ["explorer.exe", "TiWorker.EXE"].into_iter().map(str::to_string).collect();
        assert_eq!(process_state(Some(&names)), State::Pending);
        let names = ["explorer.exe", "notepad.exe"].into_iter().map(str::to_string).collect();
        assert_eq!(process_state(Some(&names)), State::Clear);
        assert_eq!(process_state(Some(&Default::default())), State::Unknown);
        assert_eq!(process_state(None), State::Unknown);
    }

    #[test]
    fn catalog_failure_or_ambiguous_output_never_allows_cleanup() {
        assert_eq!(catalog_output(true, "MOM_UPDATE_CLEAR_V1\r\n"), State::Clear);
        assert_eq!(catalog_output(true, "MOM_UPDATE_PENDING_V1"), State::Pending);
        for text in ["", "0", "MOM_UPDATE_CLEAR_V1\nwarning", "MOM_UPDATE_CLEAR_V1\nMOM_UPDATE_PENDING_V1"] {
            assert_eq!(catalog_output(true, text), State::Unknown, "{text}");
        }
        assert_eq!(catalog_output(false, "MOM_UPDATE_CLEAR_V1"), State::Unknown);
    }
}
