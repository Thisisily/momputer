//! Screens running below the refresh rate they can do: a 240 Hz screen left at 60 Hz after a
//! driver update, a new cable or a resolution change. Windows says both numbers; the fix is one
//! setting. Looks only.

use crate::speed::Fix;
use crate::system::{check, Check};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Screen {
    /// What Windows calls the monitor ("Generic PnP Monitor" when it doesn't know).
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// Hertz now, and the most it offers at this resolution.
    pub now: u32,
    pub best: u32,
}

/// Every screen on the desktop, with its refresh rate now and the best one at its resolution.
#[cfg(windows)]
pub(crate) fn screens() -> Vec<Screen> {
    use windows_sys::Win32::Graphics::Gdi::{
        EnumDisplayDevicesW, EnumDisplaySettingsW, DEVMODEW, DISPLAY_DEVICEW, DISPLAY_DEVICE_ATTACHED_TO_DESKTOP, DM_INTERLACED,
        ENUM_CURRENT_SETTINGS,
    };
    let text = |w: &[u16]| String::from_utf16_lossy(&w[..w.iter().position(|c| *c == 0).unwrap_or(w.len())]).trim().to_string();
    let mut out = Vec::new();
    for i in 0..16 {
        let mut dd = DISPLAY_DEVICEW { cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
        // SAFETY: `dd` is ours, sized in `cb`.
        if unsafe { EnumDisplayDevicesW(std::ptr::null(), i, &mut dd, 0) } == 0 {
            break;
        }
        if dd.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP == 0 {
            continue;
        }
        let mut now = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
        // SAFETY: DeviceName is NUL-terminated by Windows; `now` is ours, sized in dmSize.
        if unsafe { EnumDisplaySettingsW(dd.DeviceName.as_ptr(), ENUM_CURRENT_SETTINGS, &mut now) } == 0 || now.dmDisplayFrequency <= 1 {
            continue;
        }
        let (width, height, hz) = (now.dmPelsWidth, now.dmPelsHeight, now.dmDisplayFrequency);
        let mut best = hz;
        for mode in 0..4_000 {
            let mut m = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
            // SAFETY: as above.
            if unsafe { EnumDisplaySettingsW(dd.DeviceName.as_ptr(), mode, &mut m) } == 0 {
                break;
            }
            // SAFETY: plain integers in a union Windows filled in.
            let interlaced = unsafe { m.Anonymous2.dmDisplayFlags } & DM_INTERLACED != 0;
            if m.dmPelsWidth == width && m.dmPelsHeight == height && !interlaced && m.dmDisplayFrequency < 1_000 {
                best = best.max(m.dmDisplayFrequency);
            }
        }
        let mut monitor = DISPLAY_DEVICEW { cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
        // SAFETY: as above; the first monitor on this output.
        let name = if unsafe { EnumDisplayDevicesW(dd.DeviceName.as_ptr(), 0, &mut monitor, 0) } != 0 { text(&monitor.DeviceString) } else { String::new() };
        out.push(Screen { name: crate::system::plain(&name), width, height, now: hz, best });
    }
    out
}

#[cfg(not(windows))]
pub(crate) fn screens() -> Vec<Screen> {
    Vec::new()
}

/// A name worth saying: Windows' stand-in names say nothing.
fn named(s: &Screen) -> Option<&str> {
    let n = s.name.to_ascii_lowercase();
    (!n.is_empty() && !n.starts_with("generic") && !n.starts_with("default monitor")).then_some(s.name.as_str())
}

pub(crate) fn checks(screens: &[Screen]) -> Vec<Check> {
    let mut out = Vec::new();
    for (i, s) in screens.iter().enumerate() {
        // 59 against 60 is the same thing said two ways; only a real step counts.
        if s.best < 75 || s.now + 2 > s.best {
            continue;
        }
        let which = match (named(s), screens.len()) {
            (Some(name), _) => format!("Your {name}"),
            (None, 1) => format!("Your {} Hz screen", s.best),
            (None, _) => format!("Screen {} ({} Hz)", i + 1, s.best),
        };
        out.push(check(
            &format!("display-refresh-{i}"),
            "Hardware",
            "warn",
            format!("{which} runs at {} Hz", s.now),
            format!(
                "It can do {} Hz at {}×{}. Pick it under Advanced display; everything moves smoother.",
                s.best, s.width, s.height
            ),
            Some(Fix { label: "Display settings".into(), kind: "open", value: "ms-settings:display-advanced".into(), hint: None }),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(name: &str, now: u32, best: u32) -> Screen {
        Screen { name: name.into(), width: 2560, height: 1440, now, best }
    }

    #[test]
    fn a_fast_screen_held_back_says_so() {
        let c = checks(&[screen("Generic PnP Monitor", 60, 240)]);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].title, "Your 240 Hz screen runs at 60 Hz");
        assert!(c[0].detail.starts_with("It can do 240 Hz at 2560×1440"));
        assert_eq!(c[0].fix.as_ref().unwrap().value, "ms-settings:display-advanced");
        let named = checks(&[screen("Lanternfall LF27", 144, 165), screen("Generic PnP Monitor", 60, 60)]);
        assert_eq!(named.len(), 1);
        assert_eq!(named[0].title, "Your Lanternfall LF27 runs at 144 Hz");
        let two = checks(&[screen("Generic PnP Monitor", 240, 240), screen("Default Monitor", 60, 144)]);
        assert_eq!(two[0].title, "Screen 2 (144 Hz) runs at 60 Hz");
    }

    #[test]
    fn ordinary_and_full_speed_screens_say_nothing() {
        assert!(checks(&[screen("Generic PnP Monitor", 60, 60)]).is_empty(), "a 60 Hz screen");
        assert!(checks(&[screen("Generic PnP Monitor", 59, 60)]).is_empty(), "59.94 shown as 59");
        assert!(checks(&[screen("Generic PnP Monitor", 239, 240)]).is_empty(), "already at its best");
        assert!(checks(&[screen("Generic PnP Monitor", 60, 70)]).is_empty(), "too small a step to bother");
        assert!(checks(&[]).is_empty());
    }
}
