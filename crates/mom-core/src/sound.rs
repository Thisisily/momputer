//! Sound outputs and microphones, as Windows lists them: which are real, which an app added,
//! which go to a screen, and which belong to something that's gone. Read from the registry
//! (no administrator needed); nothing here changes a setting.

use crate::speed::Fix;
use crate::system::Check;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// In the list you pick from.
    On,
    /// Switched off in Sound settings ("Don't allow").
    Off,
    /// Known, but not plugged in or not connected right now (a Bluetooth headset).
    Unplugged,
    /// Its device or socket isn't there.
    Gone,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Endpoint {
    pub output: bool,
    /// "Speakers", "Microphone".
    pub name: String,
    /// What it belongs to: "Realtek(R) Audio", "Steam Streaming Speakers".
    pub adapter: String,
    pub state: State,
}

/// Windows' DeviceState: the low bits say where it stands, a high bit that the person switched
/// it off.
pub fn state_of(raw: u32) -> State {
    match (raw & 0xF, raw & 0x1000_0000 != 0) {
        (1, false) => State::On,
        (1, true) | (2, _) => State::Off,
        (8, _) => State::Unplugged,
        _ => State::Gone,
    }
}

/// Sound devices that aren't hardware: (what the device is called, the app that adds it, a
/// word its entry in Installed apps contains, or "" if there's nothing to look for).
const APP_MADE: &[(&str, &str, &str)] = &[
    ("steam streaming", "Steam", "steam"),
    // Driver-only tools: they never appear in Installed apps, so there is nothing to look for.
    ("voicemeeter", "Voicemeeter", ""),
    ("vb-audio", "VB-CABLE", ""),
    ("voice.ai", "Voice.ai", "voice.ai"),
    ("voicemod", "Voicemod", "voicemod"),
    ("steelseries sonar", "SteelSeries GG", "steelseries"),
    ("nvidia broadcast", "NVIDIA Broadcast", "nvidia broadcast"),
    ("nvidia virtual audio", "NVIDIA's game streaming", ""),
    ("oculus virtual audio", "the Meta Quest app", "oculus"),
    ("virtual desktop audio", "Virtual Desktop", "virtual desktop"),
    ("razer surround", "Razer Surround", "razer"),
    ("thx spatial", "THX Spatial Audio", "thx"),
    ("nahimic", "Nahimic", "nahimic"),
    ("wave link", "Elgato Wave Link", "wave link"),
    ("krisp", "Krisp", "krisp"),
    ("parsec virtual", "Parsec", "parsec"),
    ("fxsound", "FxSound", "fxsound"),
    ("droidcam", "DroidCam", "droidcam"),
    ("sonic studio virtual", "ASUS Sonic Studio", "sonic studio"),
    ("teamviewer", "TeamViewer", "teamviewer"),
];

/// Sound that goes down a screen's cable, played by the screen if it has speakers.
const SCREEN: &[&str] = &["nvidia high definition audio", "amd high definition audio", "intel(r) display audio", "display audio"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Real,
    Screen,
    /// (app, word to look for in Installed apps)
    App(&'static str, &'static str),
    /// Something calling itself virtual that mom doesn't know by name.
    Virtual,
}

fn kind_of(e: &Endpoint) -> Kind {
    let a = e.adapter.to_ascii_lowercase();
    if let Some((_, app, word)) = APP_MADE.iter().find(|(needle, _, _)| a.contains(needle)) {
        return Kind::App(app, word);
    }
    if SCREEN.iter().any(|s| a.contains(s)) {
        return Kind::Screen;
    }
    if a.contains("virtual") || a.contains("audio cable") {
        return Kind::Virtual;
    }
    Kind::Real
}

/// "Speakers (Realtek(R) Audio)"; an app's device goes by the app's own name for it.
fn label(e: &Endpoint) -> String {
    match kind_of(e) {
        Kind::App(..) | Kind::Virtual => e.adapter.clone(),
        _ if e.name.is_empty() => e.adapter.clone(),
        _ if e.adapter.is_empty() || e.name.to_ascii_lowercase().contains(&e.adapter.to_ascii_lowercase()) => e.name.clone(),
        _ => format!("{} ({})", e.name, e.adapter),
    }
}

fn list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// What to say about one direction (outputs, or microphones).
fn direction(all: &[Endpoint], output: bool) -> Option<Check> {
    let mine: Vec<&Endpoint> = all.iter().filter(|e| e.output == output).collect();
    let on: Vec<&Endpoint> = mine.iter().copied().filter(|e| e.state == State::On).collect();
    let made: Vec<&Endpoint> = on.iter().copied().filter(|e| matches!(kind_of(e), Kind::App(..) | Kind::Virtual)).collect();
    let screens: Vec<&Endpoint> = on.iter().copied().filter(|e| kind_of(e) == Kind::Screen).collect();
    let real: Vec<&Endpoint> = on.iter().copied().filter(|e| kind_of(e) == Kind::Real).collect();
    // Nothing to tidy: only real ones, and not many.
    if made.is_empty() && (screens.is_empty() || on.len() <= 3) && on.len() <= 5 {
        return None;
    }
    let labels = |v: &[&Endpoint]| {
        let mut names: Vec<String> = v.iter().map(|e| label(e)).collect();
        names.sort();
        names.dedup();
        list(&names)
    };
    let (things, thing, real_word) = if output { ("sound outputs", "speakers", "The ones you can hear through") } else { ("microphones", "microphones", "The real ones") };
    let title = if made.is_empty() {
        format!("{} {things} are switched on", on.len())
    } else if made.len() == 1 {
        format!("{} {things} are switched on, and 1 of them isn't {}", on.len(), if output { "a speaker" } else { "a microphone" })
    } else {
        format!("{} {things} are switched on, and {} of them aren't {thing}", on.len(), made.len())
    };
    let mut detail = String::new();
    if !made.is_empty() {
        detail.push_str(&format!("Made by apps, to record sound or pass it along: {}. ", labels(&made)));
    }
    if !screens.is_empty() {
        detail.push_str(&format!("Going down a screen's cable (silent if the screen has no speakers): {}. ", labels(&screens)));
    }
    if !real.is_empty() {
        detail.push_str(&format!("{real_word}: {}. ", labels(&real)));
    }
    detail.push_str("To take one off the list, open it in Sound settings and press Don't allow. That only hides it; the same switch brings it back.");
    let hidden = mine.len() - on.len();
    if hidden > 0 {
        detail.push_str(&format!(" (Windows keeps {hidden} more that are switched off, unplugged or gone; those don't show in the list.)"));
    }
    Some(Check {
        id: if output { "sound-outputs".into() } else { "sound-inputs".into() },
        area: "Sound",
        level: "tip",
        title,
        detail,
        fix: Some(Fix { label: "Sound settings".into(), kind: "open", value: "ms-settings:sound".into(), hint: None }),
    })
}

/// What mom has to say about the sound devices: the app-made ones in the way, and the ones an
/// uninstalled app left behind.
pub fn checks(all: &[Endpoint], installed: &[String]) -> Vec<Check> {
    let mut out = Vec::new();
    out.extend(direction(all, true));
    out.extend(direction(all, false));
    // An app's sound device still here when the app isn't: its driver was left behind.
    let apps: Vec<String> = installed.iter().map(|a| a.to_ascii_lowercase()).collect();
    let mut told: Vec<&str> = Vec::new();
    for e in all.iter().filter(|e| e.state != State::Gone) {
        let Kind::App(app, word) = kind_of(e) else { continue };
        if word.is_empty() || told.contains(&app) || apps.iter().any(|a| a.contains(word)) {
            continue;
        }
        told.push(app);
        out.push(Check {
            id: format!("sound-leftover-{}", app.to_ascii_lowercase().replace([' ', '.', '\''], "-")),
            area: "Sound",
            level: "tip",
            title: format!("{app} is gone, but its sound device isn't"),
            detail: format!(
                "{} is still among your sound devices, and {app} isn't installed any more. Device Manager can remove it: find it under Sound, video and game controllers, right-click it and choose Uninstall device.",
                e.adapter
            ),
            fix: Some(Fix { label: "Device Manager".into(), kind: "launch", value: "devmgmt.msc".into(), hint: None }),
        });
    }
    out
}

/// One line for "This PC": the outputs you can actually hear through.
pub fn summary(all: &[Endpoint]) -> Option<String> {
    let on: Vec<&Endpoint> = all.iter().filter(|e| e.output && e.state == State::On).collect();
    if on.is_empty() {
        return None;
    }
    let mut real: Vec<String> = on.iter().filter(|e| kind_of(e) == Kind::Real).map(|e| label(e)).collect();
    real.dedup();
    let other = on.len() - on.iter().filter(|e| kind_of(e) == Kind::Real).count();
    let extra = if other == 0 { String::new() } else { format!("{}{other} more made by apps or going to a screen", if real.is_empty() { "" } else { " · " }) };
    Some(format!("{}{extra}", real.join(" · ")))
}

/// Every sound output and microphone Windows knows.
#[cfg(windows)]
pub fn endpoints() -> Vec<Endpoint> {
    use crate::health::reg::{dword_value, string_values, subkeys, Hive::LocalMachine as Lm};
    const BASE: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio";
    // The endpoint's own name, and the name of the device it's part of.
    const NAME: &str = "{a45c254e-df1c-4efd-8020-67d146a850e0},2";
    const ADAPTER: &str = "{b3f8fa53-0004-438e-9003-51a46e139bfc},6";
    let mut out = Vec::new();
    for (flow, output) in [("Render", true), ("Capture", false)] {
        for id in subkeys(Lm, &format!(r"{BASE}\{flow}")) {
            let key = format!(r"{BASE}\{flow}\{id}");
            let Some(raw) = dword_value(Lm, &key, "DeviceState") else { continue };
            let props = string_values(Lm, &format!(r"{key}\Properties"));
            let get = |name: &str| props.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string()).unwrap_or_default();
            let (name, adapter) = (get(NAME), get(ADAPTER));
            if name.is_empty() && adapter.is_empty() {
                continue;
            }
            out.push(Endpoint { output, name, adapter, state: state_of(raw) });
        }
    }
    out.sort_by(|a, b| (b.output, &a.adapter, &a.name).cmp(&(a.output, &b.adapter, &b.name)));
    out
}

#[cfg(not(windows))]
pub fn endpoints() -> Vec<Endpoint> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(output: bool, name: &str, adapter: &str, raw: u32) -> Endpoint {
        Endpoint { output, name: name.into(), adapter: adapter.into(), state: state_of(raw) }
    }

    #[test]
    fn states_as_windows_writes_them() {
        assert_eq!(state_of(1), State::On);
        assert_eq!(state_of(0x1000_0001), State::Off);
        assert_eq!(state_of(8), State::Unplugged);
        assert_eq!(state_of(4), State::Gone);
        assert_eq!(state_of(0x3100_0004), State::Gone);
    }

    #[test]
    fn app_made_outputs_are_told_from_real_ones() {
        let all = vec![
            e(true, "Speakers", "Lanternfall Soundbar", 1),
            e(true, "Speakers", "Realtek(R) Audio", 1),
            e(true, "Speakers", "Steam Streaming Speakers", 1),
            e(true, "Speakers", "Steam Streaming Microphone", 1),
            e(true, "CABLE Input", "VB-Audio Virtual Cable", 1),
            e(true, "Speakers", "Voice.ai Audio Cable", 1),
            e(true, "Screen", "NVIDIA High Definition Audio", 0x1000_0001),
            e(true, "NVIDIA Output", "NVIDIA High Definition Audio", 4),
            e(true, "Headphones", "Copperline Buds", 8),
            e(false, "Microphone", "Realtek(R) Audio", 1),
        ];
        let c = checks(&all, &["Steam".into()]);
        let outputs = c.iter().find(|c| c.id == "sound-outputs").unwrap();
        assert_eq!(outputs.title, "6 sound outputs are switched on, and 4 of them aren't speakers");
        assert!(outputs.detail.starts_with("Made by apps, to record sound or pass it along: Steam Streaming Microphone, Steam Streaming Speakers, VB-Audio Virtual Cable and Voice.ai Audio Cable. "), "{}", outputs.detail);
        assert!(outputs.detail.contains("The ones you can hear through: Speakers (Lanternfall Soundbar) and Speakers (Realtek(R) Audio)."), "{}", outputs.detail);
        assert!(outputs.detail.contains("Windows keeps 3 more"), "{}", outputs.detail);
        assert!(!c.iter().any(|c| c.id == "sound-inputs"), "one real microphone needs no words");
        // Voice.ai isn't installed and Steam is. VB-CABLE is only ever a driver: nothing is claimed about it.
        let left: Vec<&str> = c.iter().filter(|c| c.id.starts_with("sound-leftover")).map(|c| c.title.as_str()).collect();
        assert_eq!(left, vec!["Voice.ai is gone, but its sound device isn't"]);
        assert_eq!(summary(&all).unwrap(), "Speakers (Lanternfall Soundbar) · Speakers (Realtek(R) Audio) · 4 more made by apps or going to a screen");
    }

    #[test]
    fn a_plain_pc_has_nothing_to_tidy() {
        let all = vec![e(true, "Speakers", "Realtek(R) Audio", 1), e(true, "Screen", "NVIDIA High Definition Audio", 1), e(false, "Microphone", "Realtek(R) Audio", 1)];
        assert!(checks(&all, &[]).is_empty());
        assert_eq!(summary(&all).unwrap(), "Speakers (Realtek(R) Audio) · 1 more made by apps or going to a screen");
        assert!(summary(&[]).is_none());
    }
}
