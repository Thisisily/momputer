//! Your screens: what each one is (from what it says about itself), how Windows drives it, and
//! the settings that make it look its best. A 240 Hz screen left at 60 Hz after a driver update,
//! or a 4K screen shown at 1080p, is a finding; the rest is shown in BIG MODE's monitor. Looks only.

use crate::speed::Fix;
use crate::system::{check, Check};
use serde::Serialize;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Screen {
    /// What the monitor calls itself ("Samsung LC32G7xT"), or what Windows calls it ("Generic PnP
    /// Monitor" when it doesn't know).
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// Hertz now, and the most it offers at this resolution.
    pub now: u32,
    pub best: u32,
    /// The resolution the panel is made for, from the monitor itself (or its biggest mode).
    pub native: Option<(u32, u32)>,
    /// Its diagonal, and the year it was made, from the monitor itself.
    pub inches: Option<u32>,
    pub year: Option<u32>,
    /// How it's plugged in: "DisplayPort", "HDMI", "built in"…
    pub link: Option<&'static str>,
    /// Whether HDR is on, when the screen can do HDR at all.
    pub hdr: Option<bool>,
    /// Bits per colour, as the screen is driven now.
    pub bits: Option<u32>,
    /// Scale, in percent.
    pub scale: Option<u32>,
    /// The main screen (the one with the taskbar's clock, at the desktop's top left).
    pub main: bool,
}

/// Settings that apply to every screen.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Everywhere {
    /// "Optimizations for windowed games": less lag in games that run in a window.
    pub windowed_games: Option<bool>,
    pub auto_hdr: Option<bool>,
    /// Windows' text size, in percent (100 unless changed).
    pub text: u32,
}

/// One screen as BIG MODE's monitor shows it: a name, a line about it, and its settings.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Shown {
    pub title: String,
    pub sub: String,
    pub rows: Vec<Row>,
}

/// A setting: what it is, how it's set now, and the Settings page that changes it.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub label: String,
    pub value: String,
    /// Not what it could be (a 240 Hz screen at 60 Hz): its light is red.
    pub low: bool,
    pub open: Option<String>,
}

// ------------------------------------------------------------------ reading them

/// Every screen on the desktop: what it is and how Windows drives it.
#[cfg(windows)]
pub(crate) fn screens() -> Vec<Screen> {
    let mut out = os::configured();
    if out.is_empty() {
        out = os::gdi();
    }
    out
}

#[cfg(not(windows))]
pub(crate) fn screens() -> Vec<Screen> {
    Vec::new()
}

pub(crate) fn everywhere() -> Everywhere {
    use crate::winsettings::{read_raw, Hive};
    let dx = read_raw(Hive::CurrentUser, r"Software\Microsoft\DirectX\UserGpuPreferences", "DirectXUserGlobalSettings")
        .and_then(|r| r.as_string())
        .unwrap_or_default();
    let flag = |name: &str| dx.split(';').find_map(|kv| kv.trim().strip_prefix(name)?.strip_prefix('=')).map(|v| v.trim() == "1");
    let text = read_raw(Hive::CurrentUser, r"Software\Microsoft\Accessibility", "TextScaleFactor").and_then(|r| r.as_dword()).filter(|t| (100..=225).contains(t));
    Everywhere { windowed_games: flag("SwapEffectUpgradeEnable"), auto_hdr: flag("AutoHDREnable"), text: text.unwrap_or(100) }
}

/// What a monitor's EDID says about it: who made it, its name, size, year and resolution.
#[derive(Debug, Default, PartialEq)]
struct Edid {
    maker: Option<&'static str>,
    model: Option<String>,
    inches: Option<u32>,
    year: Option<u32>,
    native: Option<(u32, u32)>,
}

fn edid(b: &[u8]) -> Option<Edid> {
    if b.len() < 128 || b[..8] != [0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0] {
        return None;
    }
    let id: String = [(b[8] >> 2) & 0x1F, ((b[8] & 3) << 3) | (b[9] >> 5), b[9] & 0x1F].iter().map(|c| (c + 64) as char).collect();
    let mut e = Edid { maker: maker(&id), ..Default::default() };
    if b[17] > 0 {
        e.year = Some(1990 + b[17] as u32);
    }
    let (w, h) = (b[21] as f64, b[22] as f64);
    if w > 0.0 && h > 0.0 {
        e.inches = Some(((w * w + h * h).sqrt() / 2.54).round() as u32);
    }
    for at in [54, 72, 90, 108] {
        let d = &b[at..at + 18];
        if d[0] != 0 || d[1] != 0 {
            // A detailed timing; the first is the panel's own.
            if e.native.is_none() {
                let wide = d[2] as u32 | ((d[4] as u32 & 0xF0) << 4);
                let tall = d[5] as u32 | ((d[7] as u32 & 0xF0) << 4);
                if wide > 0 && tall > 0 {
                    e.native = Some((wide, tall));
                }
            }
        } else if d[3] == 0xFC {
            let name: String = d[5..].iter().take_while(|c| **c != 0x0A && **c != 0).map(|c| *c as char).collect();
            let name = name.trim().to_string();
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
                e.model = Some(name);
            }
        }
    }
    Some(e)
}

/// The maker behind a monitor's three-letter EDID id, for the common ones.
fn maker(id: &str) -> Option<&'static str> {
    Some(match id {
        "SAM" | "SEC" => "Samsung",
        "GSM" | "LGD" => "LG",
        "DEL" => "Dell",
        "ACR" => "Acer",
        "AUS" => "ASUS",
        "BNQ" => "BenQ",
        "AOC" => "AOC",
        "HWP" | "HPN" => "HP",
        "LEN" => "Lenovo",
        "MSI" => "MSI",
        "PHL" => "Philips",
        "VSC" => "ViewSonic",
        "GBT" => "Gigabyte",
        "SNY" => "Sony",
        "APP" => "Apple",
        "IVM" => "iiyama",
        "EIZ" | "ENC" => "EIZO",
        "NEC" => "NEC",
        "XMI" => "Xiaomi",
        "HKC" => "HKC",
        "MEI" | "PAN" => "Panasonic",
        "TCL" => "TCL",
        "HSD" => "HannStar",
        "AUO" => "AU Optronics",
        "BOE" => "BOE",
        "SHP" => "Sharp",
        "CMN" => "Innolux",
        _ => return None,
    })
}

/// "Samsung LC32G7xT" from a maker and the monitor's own name; Windows' name when it has neither.
fn full_name(maker: Option<&str>, model: Option<&str>, windows: &str) -> String {
    match (maker, model) {
        (Some(m), Some(n)) if n.to_ascii_lowercase().starts_with(&m.to_ascii_lowercase()) => n.to_string(),
        (Some(m), Some(n)) => format!("{m} {n}"),
        (None, Some(n)) => n.to_string(),
        _ => windows.to_string(),
    }
}

#[cfg(windows)]
mod os {
    use super::{edid, full_name, Screen};
    use crate::winsettings::{read_raw, Hive};
    use windows_sys::Win32::Devices::Display::{
        DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig, DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
        DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
        DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE, DISPLAYCONFIG_PATH_INFO,
        DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
    };
    use windows_sys::Win32::Graphics::Gdi::{
        EnumDisplayDevicesW, EnumDisplaySettingsW, DEVMODEW, DISPLAY_DEVICEW, DISPLAY_DEVICE_ATTACHED_TO_DESKTOP, DM_INTERLACED,
        ENUM_CURRENT_SETTINGS,
    };

    fn text(w: &[u16]) -> String {
        String::from_utf16_lossy(&w[..w.iter().position(|c| *c == 0).unwrap_or(w.len())]).trim().to_string()
    }

    /// The most hertz a GDI device offers at a resolution, and its biggest resolution.
    fn modes(device: &[u16], width: u32, height: u32) -> (u32, Option<(u32, u32)>) {
        let (mut best, mut biggest) = (0, None::<(u32, u32)>);
        for mode in 0..4_000 {
            let mut m = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
            // SAFETY: `device` is NUL-terminated (it came from Windows); `m` is ours, sized in dmSize.
            if unsafe { EnumDisplaySettingsW(device.as_ptr(), mode, &mut m) } == 0 {
                break;
            }
            // SAFETY: plain integers in a union Windows filled in.
            let interlaced = unsafe { m.Anonymous2.dmDisplayFlags } & DM_INTERLACED != 0;
            if interlaced || m.dmDisplayFrequency >= 1_000 {
                continue;
            }
            if m.dmPelsWidth == width && m.dmPelsHeight == height {
                best = best.max(m.dmDisplayFrequency);
            }
            if biggest.is_none_or(|(w, h)| m.dmPelsWidth * m.dmPelsHeight > w * h) {
                biggest = Some((m.dmPelsWidth, m.dmPelsHeight));
            }
        }
        (best, biggest)
    }

    fn info<T>(packet: &mut T) -> bool {
        // SAFETY: every packet starts with its header, whose size and type the caller set.
        unsafe { DisplayConfigGetDeviceInfo(packet as *mut T as *mut DISPLAYCONFIG_DEVICE_INFO_HEADER) == 0 }
    }

    fn header<T>(kind: i32, adapter: windows_sys::Win32::Foundation::LUID, id: u32) -> DISPLAYCONFIG_DEVICE_INFO_HEADER {
        DISPLAYCONFIG_DEVICE_INFO_HEADER { r#type: kind, size: std::mem::size_of::<T>() as u32, adapterId: adapter, id }
    }

    /// The monitor's EDID, from where Windows keeps it for this device path.
    fn edid_of(device_path: &str) -> Option<Vec<u8>> {
        // \\?\DISPLAY#SAM7058#5&12a61067&0&UID37121#{e6f07b5f-…}
        let parts: Vec<&str> = device_path.trim_start_matches(r"\\?\").split('#').collect();
        let [kind, model, instance, _] = parts.as_slice() else { return None };
        if [kind, model, instance].iter().any(|p| p.is_empty() || p.contains('\\') || p.contains('/')) {
            return None;
        }
        let key = format!(r"SYSTEM\CurrentControlSet\Enum\{kind}\{model}\{instance}\Device Parameters");
        read_raw(Hive::LocalMachine, &key, "EDID").map(|r| r.data)
    }

    /// Its scale, in percent: the monitor at `x, y`, asked as a program that knows about scale.
    fn scale_at(x: i32, y: i32) -> Option<u32> {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONULL};
        use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI};
        // SAFETY: plain calls; this thread's awareness is put back as it was.
        unsafe {
            let before = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONULL);
            let (mut dx, mut dy) = (0u32, 0u32);
            let ok = !monitor.is_null() && GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) >= 0;
            if !before.is_null() {
                SetThreadDpiAwarenessContext(before);
            }
            (ok && dx >= 48).then(|| (dx * 100 + 48) / 96)
        }
    }

    /// The screens as Windows has set them up: each active path from a picture to a monitor.
    pub(super) fn configured() -> Vec<Screen> {
        let (mut np, mut nm) = (0u32, 0u32);
        // SAFETY: two counts for Windows to fill in.
        if unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) } != 0 || np == 0 || np > 64 || nm > 256 {
            return Vec::new();
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes_ = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        // SAFETY: the arrays hold the counts given; no topology is asked for with these flags.
        if unsafe { QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes_.as_mut_ptr(), std::ptr::null_mut()) } != 0 {
            return Vec::new();
        }
        paths.truncate(np as usize);
        modes_.truncate(nm as usize);
        let mut out = Vec::new();
        for p in &paths {
            let (s, t) = (&p.sourceInfo, &p.targetInfo);
            // SAFETY: without virtual-mode awareness this union holds a plain index.
            let at = unsafe { s.Anonymous.modeInfoIdx } as usize;
            let Some(mode) = modes_.get(at).filter(|m| m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE) else { continue };
            // SAFETY: a source mode, as its type says.
            let src = unsafe { mode.Anonymous.sourceMode };
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME { header: header::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>(DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, s.adapterId, s.id), ..Default::default() };
            let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME { header: header::<DISPLAYCONFIG_TARGET_DEVICE_NAME>(DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, t.adapterId, t.id), ..Default::default() };
            let mut colour = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO { header: header::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>(DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO, t.adapterId, t.id), ..Default::default() };
            let gdi = if info(&mut source) { Some(source.viewGdiDeviceName) } else { None };
            let named = info(&mut target);
            let coloured = info(&mut colour);

            let rate = t.refreshRate;
            let now = if rate.Denominator > 0 { ((rate.Numerator as f64 / rate.Denominator as f64).round()) as u32 } else { 0 };
            let (best, biggest) = gdi.map(|g| modes(&g, src.width, src.height)).unwrap_or((now, None));
            let e = if named { edid_of(&text(&target.monitorDevicePath)).as_deref().and_then(edid) } else { None }.unwrap_or_default();
            let friendly = if named { text(&target.monitorFriendlyDeviceName) } else { String::new() };
            let windows_name = gdi.map(|g| gdi_monitor_name(&g)).unwrap_or_default();
            let model = e.model.clone().or_else(|| (!friendly.is_empty()).then(|| friendly.clone()));
            let name = full_name(e.maker, model.as_deref(), if windows_name.is_empty() { "Screen" } else { &windows_name });
            // SAFETY: plain integers in a union Windows filled in.
            let flags = unsafe { colour.Anonymous.value };
            out.push(Screen {
                name: crate::system::plain(&name),
                width: src.width,
                height: src.height,
                now,
                best: best.max(now),
                native: e.native.or(biggest),
                inches: e.inches,
                year: e.year,
                link: link(if named { target.outputTechnology } else { t.outputTechnology }),
                hdr: (coloured && flags & 1 != 0).then_some(flags & 2 != 0),
                bits: (coloured && (6..=16).contains(&colour.bitsPerColorChannel)).then_some(colour.bitsPerColorChannel),
                scale: scale_at(src.position.x + src.width as i32 / 2, src.position.y + src.height as i32 / 2),
                main: src.position.x == 0 && src.position.y == 0,
            });
        }
        out
    }

    fn link(tech: i32) -> Option<&'static str> {
        Some(match tech {
            0 => "VGA",
            4 => "DVI",
            5 => "HDMI",
            10 => "DisplayPort",
            18 => "USB-C",
            11 | 13 | 6 | i32::MIN => "built in",
            15 => "Miracast",
            16 | 17 => "a USB adapter",
            _ => return None,
        })
    }

    /// What Windows calls the first monitor on a GDI device.
    fn gdi_monitor_name(device: &[u16]) -> String {
        let mut monitor = DISPLAY_DEVICEW { cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
        // SAFETY: `device` is NUL-terminated; `monitor` is ours, sized in `cb`.
        if unsafe { EnumDisplayDevicesW(device.as_ptr(), 0, &mut monitor, 0) } != 0 {
            text(&monitor.DeviceString)
        } else {
            String::new()
        }
    }

    /// Every screen, the older way: when Windows won't say how it has them set up.
    pub(super) fn gdi() -> Vec<Screen> {
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
            let (best, biggest) = modes(&dd.DeviceName, width, height);
            out.push(Screen {
                name: crate::system::plain(&gdi_monitor_name(&dd.DeviceName)),
                width,
                height,
                now: hz,
                best: best.max(hz),
                native: biggest,
                ..Default::default()
            });
        }
        out
    }
}

// ------------------------------------------------------------------ what mom says

/// A name worth saying: Windows' stand-in names say nothing.
fn named(s: &Screen) -> Option<&str> {
    let n = s.name.to_ascii_lowercase();
    (!n.is_empty() && n != "screen" && !n.starts_with("generic") && !n.starts_with("default monitor")).then_some(s.name.as_str())
}

/// The panel's own resolution, when the screen is shown at less than it.
fn below_native(s: &Screen) -> Option<(u32, u32)> {
    s.native.filter(|(w, h)| *w > s.width && *h > s.height)
}

pub(crate) fn checks(screens: &[Screen]) -> Vec<Check> {
    let mut out = Vec::new();
    for (i, s) in screens.iter().enumerate() {
        let which = |hz: bool| match (named(s), screens.len()) {
            (Some(name), _) => format!("Your {name}"),
            (None, 1) if hz => format!("Your {} Hz screen", s.best),
            (None, 1) => "Your screen".to_string(),
            (None, _) if hz => format!("Screen {} ({} Hz)", i + 1, s.best),
            (None, _) => format!("Screen {}", i + 1),
        };
        // 59 against 60 is the same thing said two ways; only a real step counts.
        if s.best >= 75 && s.now + 2 <= s.best {
            out.push(check(
                &format!("display-refresh-{i}"),
                "Hardware",
                "warn",
                format!("{} runs at {} Hz", which(true), s.now),
                format!("It can do {} Hz at {}×{}. Pick it under Advanced display; everything moves smoother.", s.best, s.width, s.height),
                Some(Fix { label: "Display settings".into(), kind: "open", value: "ms-settings:display-advanced".into(), hint: None }),
            ));
        }
        if let Some((w, h)) = below_native(s) {
            out.push(check(
                &format!("display-resolution-{i}"),
                "Hardware",
                "warn",
                format!("{} shows {}×{}, but it's made for {w}×{h}", which(false), s.width, s.height),
                format!("Text and pictures look soft. Pick {w}×{h} under Display resolution."),
                Some(Fix { label: "Display settings".into(), kind: "open", value: "ms-settings:display".into(), hint: None }),
            ));
        }
    }
    out
}

/// One line for "This PC": each screen's name, size and how it runs.
pub(crate) fn summary(screens: &[Screen]) -> Option<String> {
    let each: Vec<String> = screens
        .iter()
        .map(|s| {
            let inches = s.inches.map(|i| format!(" {i}″")).unwrap_or_default();
            format!("{}{inches}, {}×{} at {} Hz", s.name, s.width, s.height, s.now)
        })
        .collect();
    (!each.is_empty()).then(|| each.join(" · "))
}

const DISPLAY: &str = "ms-settings:display";

/// Each screen as BIG MODE's monitor shows it, and the settings every screen shares last.
pub fn shown(screens: &[Screen], all: &Everywhere) -> Vec<Shown> {
    let row = |label: &str, value: String, low: bool, open: Option<&str>| Row { label: label.into(), value, low, open: open.map(String::from) };
    let mut out: Vec<Shown> = screens
        .iter()
        .map(|s| {
            let mut rows = Vec::new();
            let resolution = match below_native(s) {
                Some((w, h)) => (format!("{}×{}, made for {w}×{h}", s.width, s.height), true),
                None => (format!("{}×{}", s.width, s.height), false),
            };
            rows.push(row("Resolution", resolution.0, resolution.1, Some(DISPLAY)));
            let slow = s.best >= 75 && s.now + 2 <= s.best;
            let hz = if slow { format!("{} Hz, can do {}", s.now, s.best) } else { format!("{} Hz", s.now) };
            rows.push(row("Refresh rate", hz, slow, Some("ms-settings:display-advanced")));
            if let Some(scale) = s.scale {
                rows.push(row("Scale", format!("{scale}%"), false, Some(DISPLAY)));
            }
            if let Some(on) = s.hdr {
                rows.push(row("HDR", if on { "On" } else { "Off" }.into(), false, Some("ms-settings:display-hdr")));
            }
            if let Some(bits) = s.bits {
                rows.push(row("Colour", format!("{bits} bits"), false, Some("ms-settings:display-advanced")));
            }
            if let Some(link) = s.link {
                rows.push(row("Plugged in by", link.into(), false, None));
            }
            if screens.len() > 1 {
                rows.push(row("Main screen", if s.main { "Yes" } else { "No" }.into(), false, Some(DISPLAY)));
            }
            let sub = [s.inches.map(|i| format!("{i} inches")), s.native.map(|(w, h)| format!("made for {w}×{h}")), s.year.map(|y| format!("from {y}"))]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            Shown { title: s.name.clone(), sub, rows }
        })
        .collect();
    let mut rows = vec![row("Night light", "Warmer colours at night".into(), false, Some("ms-settings:nightlight"))];
    if let Some(on) = all.windowed_games {
        rows.push(row("Games in a window", if on { "Optimized" } else { "Not optimized" }.into(), false, Some("ms-settings:display-advancedgraphics")));
    }
    if let (Some(on), true) = (all.auto_hdr, screens.iter().any(|s| s.hdr.is_some())) {
        rows.push(row("Auto HDR in games", if on { "On" } else { "Off" }.into(), false, Some("ms-settings:display-hdr")));
    }
    rows.push(row("Text size", format!("{}%", all.text), false, Some("ms-settings:easeofaccess-display")));
    if !out.is_empty() {
        out.push(Shown { title: if screens.len() > 1 { "Every screen".into() } else { "Windows".into() }, sub: String::new(), rows });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(name: &str, now: u32, best: u32) -> Screen {
        Screen { name: name.into(), width: 2560, height: 1440, now, best, ..Default::default() }
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
        let native = Screen { native: Some((2560, 1440)), ..screen("Lanternfall LF27", 144, 144) };
        assert!(checks(&[native]).is_empty(), "at its own resolution");
        assert!(checks(&[]).is_empty());
    }

    #[test]
    fn a_screen_shown_below_what_its_made_for_says_so() {
        let s = Screen { width: 1920, height: 1080, native: Some((3840, 2160)), ..screen("Copperline 27U", 60, 60) };
        let c = checks(&[s.clone()]);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].title, "Your Copperline 27U shows 1920×1080, but it's made for 3840×2160");
        assert_eq!(c[0].level, "warn");
        let shown = shown(&[s], &Everywhere { text: 100, ..Default::default() });
        assert_eq!(shown[0].rows[0].value, "1920×1080, made for 3840×2160");
        assert!(shown[0].rows[0].low);
    }

    #[test]
    fn the_monitor_shows_each_setting_with_where_to_change_it() {
        let s = Screen {
            inches: Some(32),
            year: Some(2021),
            native: Some((2560, 1440)),
            link: Some("DisplayPort"),
            hdr: Some(false),
            bits: Some(8),
            scale: Some(100),
            main: true,
            ..screen("Lanternfall LF32", 240, 240)
        };
        let all = Everywhere { windowed_games: Some(true), auto_hdr: Some(true), text: 100 };
        let v = shown(&[s], &all);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].title, "Lanternfall LF32");
        assert_eq!(v[0].sub, "32 inches · made for 2560×1440 · from 2021");
        let rows: Vec<(&str, &str)> = v[0].rows.iter().map(|r| (r.label.as_str(), r.value.as_str())).collect();
        assert_eq!(rows, [("Resolution", "2560×1440"), ("Refresh rate", "240 Hz"), ("Scale", "100%"), ("HDR", "Off"), ("Colour", "8 bits"), ("Plugged in by", "DisplayPort")]);
        assert!(v[0].rows.iter().all(|r| !r.low));
        assert_eq!(v[1].title, "Windows");
        assert!(v[1].rows.iter().any(|r| r.label == "Auto HDR in games" && r.value == "On"));
        assert!(v[1].rows.iter().all(|r| r.open.is_some()));
        assert!(shown(&[], &all).is_empty());
    }

    #[test]
    fn a_monitor_says_who_made_it_its_size_and_resolution() {
        let mut b = vec![0u8; 128];
        b[..8].copy_from_slice(&[0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0]);
        // "SAM": S=19, A=1, M=13 in five bits each.
        let id: u16 = (19 << 10) | (1 << 5) | 13;
        b[8..10].copy_from_slice(&id.to_be_bytes());
        b[17] = 31; // 2021
        b[21] = 70;
        b[22] = 40;
        // A detailed timing of 2560×1440.
        b[54] = 0x10;
        b[55] = 0x20;
        b[56] = (2560 & 0xFF) as u8;
        b[58] = ((2560 >> 8) << 4) as u8;
        b[59] = (1440 & 0xFF) as u8;
        b[61] = ((1440 >> 8) << 4) as u8;
        // Its name.
        b[72..77].copy_from_slice(&[0, 0, 0, 0xFC, 0]);
        b[77..86].copy_from_slice(b"LC32G7xT\n");
        let e = edid(&b).unwrap();
        assert_eq!(e, Edid { maker: Some("Samsung"), model: Some("LC32G7xT".into()), inches: Some(32), year: Some(2021), native: Some((2560, 1440)) });
        assert_eq!(full_name(e.maker, e.model.as_deref(), "Generic PnP Monitor"), "Samsung LC32G7xT");
        assert_eq!(full_name(Some("LG"), Some("LG ULTRAGEAR"), "x"), "LG ULTRAGEAR");
        assert_eq!(full_name(None, None, "Generic PnP Monitor"), "Generic PnP Monitor");
        assert_eq!(edid(&b[..100]), None);
        b[0] = 1;
        assert_eq!(edid(&b), None);
    }
}
