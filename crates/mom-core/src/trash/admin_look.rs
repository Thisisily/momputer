//! Measuring what only an administrator can see, when you ask. With "admin" ticked next to the
//! widget's Measure button, Windows asks first, then runs `mom trash look --elevated`. That copy
//! of mom measures the clean-ups that need an administrator and writes down what it saw in
//! HKLM\SOFTWARE\momputer, which only an administrator can change and anyone can read. Measuring
//! as you then fills in what you can't open from there, for as long as it's still there.
//!
//! It only measures: nothing is deleted, and a clean-up checks everything again as administrator
//! before it takes anything.

use super::{norm_path, TrashCtx};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Component, Path};

const KEY: &str = r"SOFTWARE\momputer";
const VALUE: &str = "AdminLook";
/// The clean-ups an administrator measures when asked.
pub const LOOKS: &[&str] = &["windows-junk", "admin-leftovers"];
const MAX_ITEMS: usize = 5_000;

/// What the administrator saw, and when.
#[derive(Serialize, Deserialize, Default)]
struct Look {
    at: u64,
    items: Vec<Seen>,
}

/// One thing it measured.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub(super) struct Seen {
    pub path: String,
    pub bytes: u64,
    pub files: u64,
    /// When it last changed (seconds since 1970), 0 if unknown.
    pub recency: u64,
    /// A file, not a folder.
    pub file: bool,
}

/// As administrator: measures the clean-ups in `LOOKS` and writes down what it saw. How many
/// things that was, or why it couldn't.
pub fn look(ctx: &TrashCtx) -> Result<usize, String> {
    if !ctx.elevated {
        return Err("Only an administrator's copy of mom looks here. The widget asks Windows for one.".into());
    }
    let mut items = Vec::new();
    for id in LOOKS {
        for c in super::find(id, ctx).cands.into_iter().filter(|c| c.complete) {
            let file = std::fs::symlink_metadata(&c.path).is_ok_and(|m| m.is_file());
            let recency = if c.age_known { ctx.now.saturating_sub(c.age) } else { 0 };
            items.push(Seen { path: c.path.to_string_lossy().to_string(), bytes: c.bytes, files: c.files, recency, file });
        }
    }
    items.truncate(MAX_ITEMS);
    let n = items.len();
    let text = serde_json::to_string(&Look { at: ctx.now, items }).map_err(|e| e.to_string())?;
    write(&text)?;
    Ok(n)
}

/// What the last administrator look saw: when, and each thing by its path as `norm_path` spells
/// it. Only whole paths with no ".." in them.
pub(super) fn seen() -> Option<(u64, HashMap<String, Seen>)> {
    let look: Look = serde_json::from_str(&read()?).ok()?;
    let plain = |p: &str| {
        let p = Path::new(p);
        p.is_absolute() && p.components().all(|c| !matches!(c, Component::ParentDir | Component::CurDir))
    };
    let items = look.items.into_iter().filter(|s| plain(&s.path)).map(|s| (norm_path(&s.path), s)).collect();
    Some((look.at, items))
}

#[cfg(windows)]
fn read() -> Option<String> {
    use crate::health::reg::{string_values, Hive};
    string_values(Hive::LocalMachine, KEY).into_iter().find(|(name, _)| name == VALUE).map(|(_, text)| text)
}

#[cfg(not(windows))]
fn read() -> Option<String> {
    None
}

#[cfg(windows)]
fn write(text: &str) -> Result<(), String> {
    use crate::util::wide;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_SET_VALUE, KEY_WOW64_64KEY, REG_OPTION_NON_VOLATILE, REG_SZ,
    };
    let (key_name, value, data) = (wide(KEY), wide(VALUE), wide(text));
    let bytes = u32::try_from(data.len() * 2).map_err(|_| "That's too much to write down.".to_string())?;
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: the strings are NUL-terminated and outlive the call; `key` receives the handle.
    let rc = unsafe {
        RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            key_name.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE | KEY_WOW64_64KEY,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if rc != 0 {
        return Err(format!("Windows wouldn't let mom write down what she saw (error {rc})."));
    }
    // SAFETY: `key` is open; `data` holds `bytes` bytes, its NUL included.
    let rc = unsafe { RegSetValueExW(key, value.as_ptr(), 0, REG_SZ, data.as_ptr().cast(), bytes) };
    // SAFETY: we opened it and close it once.
    unsafe { RegCloseKey(key) };
    if rc == 0 {
        Ok(())
    } else {
        Err(format!("Windows wouldn't let mom write down what she saw (error {rc})."))
    }
}

#[cfg(not(windows))]
fn write(_: &str) -> Result<(), String> {
    Err("Only on Windows.".into())
}

#[cfg(test)]
mod tests {
    #[test]
    fn as_you_it_never_looks_or_writes() {
        let root = crate::trash::tests::fixture("admin-look");
        let paths = crate::paths::Paths::under(&root);
        let cfg = crate::config::TrashConfig::default();
        let ctx = crate::trash::tests::ctx(&paths, &cfg);
        assert!(!ctx.elevated);
        assert!(super::look(&ctx).is_err());
    }
}
