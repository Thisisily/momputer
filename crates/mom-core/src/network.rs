//! The network link: a cable linked at 100 Mbps on a port that does 1 Gbps (a worn cable or a
//! loose plug), and Wi-Fi on the slow 2.4 GHz band or with a weak signal. Network names are
//! read only to match one list against another; they're never shown or kept. Looks only.

use crate::speed::Fix;
use crate::system::{check, Check};
use serde_json::Value;
use std::time::Duration;

/// A network adapter, from the checkup's facts (`net`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Adapter {
    pub wired: bool,
    pub wifi: bool,
    pub up: bool,
    /// Bits per second it's linked at now.
    pub bps: u64,
    /// It can link at 1 Gbps or more: its speed settings offer it, or its name says so.
    pub gigabit: bool,
}

pub(crate) fn adapters(f: &Value) -> Vec<Adapter> {
    let list = match f.get("net") {
        Some(Value::Array(a)) => a.clone(),
        Some(o @ Value::Object(_)) => vec![o.clone()],
        _ => vec![],
    };
    list.iter()
        .take(16)
        .map(|a| {
            let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
            let media = s("media").to_ascii_lowercase();
            let options: Vec<String> = match a.get("options") {
                Some(Value::Array(o)) => o.iter().filter_map(Value::as_str).map(str::to_ascii_lowercase).collect(),
                Some(Value::String(o)) => vec![o.to_ascii_lowercase()],
                _ => vec![],
            };
            let desc = s("desc").to_ascii_lowercase();
            let gigabit = options.iter().any(|o| o.contains("gbps") || o.contains("gb ")) || ["gigabit", "gbe", "2.5g", "2.5 g", "10g"].iter().any(|w| desc.contains(w));
            Adapter {
                wired: media.contains("802.3"),
                wifi: media.contains("802.11") || media.contains("wireless"),
                up: a.get("up").and_then(Value::as_bool).unwrap_or(false),
                bps: a.get("bps").and_then(Value::as_u64).unwrap_or(0),
                gigabit,
            }
        })
        .collect()
}

/// What Wi-Fi itself says: the band, the signal, and whether the same network is also on 5 GHz.
/// Windows asks for location permission (or an administrator) before telling, so often it's None.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Air {
    pub ghz: f32,
    /// Percent.
    pub signal: u32,
    pub five_nearby: bool,
}

/// "Band : 2.4 GHz" lines and the rest of `netsh wlan show interfaces`: (key, value) in order.
fn pairs(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            Some((k.trim().to_ascii_lowercase(), v.trim().to_string()))
        })
        .collect()
}

fn band(value: &str, channel: Option<u32>) -> Option<f32> {
    let v = value.to_ascii_lowercase();
    if v.starts_with("2.4") {
        Some(2.4)
    } else if v.starts_with('5') {
        Some(5.0)
    } else if v.starts_with('6') {
        Some(6.0)
    } else {
        channel.map(|c| if c <= 14 { 2.4 } else { 5.0 })
    }
}

/// The connected interface, from `netsh wlan show interfaces`: (its network, band, signal).
pub(crate) fn parse_interface(text: &str) -> Option<(String, f32, u32)> {
    let p = pairs(text);
    let get = |k: &str| p.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    if !get("state")?.eq_ignore_ascii_case("connected") {
        return None;
    }
    let ssid = get("ssid")?.to_string();
    let channel = get("channel").and_then(|c| c.parse().ok());
    let ghz = band(get("band").unwrap_or_default(), channel)?;
    let signal = get("signal")?.trim_end_matches('%').trim().parse().ok()?;
    Some((ssid, ghz, signal))
}

/// Whether `ssid` is also on 5 GHz or 6 GHz, from `netsh wlan show networks mode=bssid`.
pub(crate) fn five_nearby(text: &str, ssid: &str) -> bool {
    let mut mine = false;
    for line in text.lines().take(5_000) {
        let t = line.trim();
        if let Some((k, v)) = t.split_once(':') {
            let k = k.trim().to_ascii_lowercase();
            if k.starts_with("ssid ") {
                mine = v.trim() == ssid;
            } else if mine && (k == "band" || k == "channel") {
                let ghz = if k == "band" { band(v.trim(), None) } else { v.trim().parse().ok().and_then(|c: u32| band("", Some(c))) };
                if ghz.is_some_and(|g| g >= 5.0) {
                    return true;
                }
            }
        }
    }
    false
}

pub(crate) fn read_air() -> Option<Air> {
    let netsh = crate::eventlog::system_program("netsh.exe")?;
    let o = crate::util::run(&netsh, &["wlan", "show", "interfaces"], Duration::from_secs(5)).ok().filter(|o| o.ok())?;
    let (ssid, ghz, signal) = parse_interface(&o.text())?;
    let five = ghz < 5.0
        && crate::util::run(&netsh, &["wlan", "show", "networks", "mode=bssid"], Duration::from_secs(8)).ok().filter(|o| o.ok()).is_some_and(|o| five_nearby(&o.text(), &ssid));
    Some(Air { ghz, signal, five_nearby: five })
}

fn mbps(bps: u64) -> String {
    format!("{} Mbps", bps / 1_000_000)
}

pub(crate) fn checks(list: &[Adapter], air: Option<&Air>) -> Vec<Check> {
    let mut out = Vec::new();
    if let Some(a) = list.iter().find(|a| a.wired && a.up && a.gigabit && a.bps > 0 && a.bps <= 100_000_000) {
        out.push(check(
            "network-cable",
            "Hardware",
            "warn",
            format!("Your network cable runs at {}", mbps(a.bps)),
            "The port can do 1 Gbps; a worn cable or a loose plug is usually why. A Cat 5e or newer cable fixes it.".into(),
            Some(Fix { label: "Network settings".into(), kind: "open", value: "ms-settings:network-ethernet".into(), hint: None }),
        ));
    }
    let wifi = Some(Fix { label: "Wi-Fi settings".into(), kind: "open", value: "ms-settings:network-wifi".into(), hint: None });
    match air {
        Some(a) if a.ghz < 5.0 && a.five_nearby => {
            let mut c = check(
                "network-wifi",
                "Hardware",
                "tip",
                "Your Wi-Fi is on 2.4 GHz, though your network also has 5 GHz".into(),
                "5 GHz is several times faster. Setting the Wi-Fi adapter's Preferred Band to 5 GHz in Device Manager makes Windows pick it.".into(),
                Some(Fix { label: "Device Manager".into(), kind: "launch", value: "devmgmt.msc".into(), hint: None }),
            );
            c.more.extend(wifi);
            out.push(c);
        }
        Some(a) if a.ghz < 5.0 && a.signal < 60 => out.push(check(
            "network-wifi",
            "Hardware",
            "tip",
            format!("Your Wi-Fi signal is weak ({}%) on 2.4 GHz", a.signal),
            "Moving closer to the router, or lifting it off the floor, usually helps.".into(),
            wifi,
        )),
        Some(_) => {}
        None => {
            if let Some(a) = list.iter().find(|a| a.wifi && a.up && a.bps > 0 && a.bps < 100_000_000) {
                out.push(check(
                    "network-wifi",
                    "Hardware",
                    "tip",
                    format!("Your Wi-Fi connects at only {}", mbps(a.bps)),
                    "Usually a weak signal or the slower 2.4 GHz band. Moving closer to the router helps.".into(),
                    wifi,
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_gigabit_port_on_a_slow_link_says_so() {
        let f = json!({"net": [
            {"up": true, "media": "802.3", "bps": 100000000_u64, "desc": "Lanternfall PCIe GbE Family Controller", "options": ["Auto Negotiation", "10 Mbps Full Duplex", "100 Mbps Full Duplex", "1.0 Gbps Full Duplex"]},
            {"up": true, "media": "Native 802.11", "bps": 1200000000_u64, "desc": "Copperline Wi-Fi 6 AX200"}
        ]});
        let c = checks(&adapters(&f), None);
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].id.as_str(), c[0].title.as_str()), ("network-cable", "Your network cable runs at 100 Mbps"));
        // A 100 Mbps-only port, or one that's unplugged, is fine.
        let slow = json!({"net": {"up": true, "media": "802.3", "bps": 100000000_u64, "desc": "Nimbus Fast Ethernet", "options": ["10 Mbps Full Duplex", "100 Mbps Full Duplex"]}});
        assert!(checks(&adapters(&slow), None).is_empty());
        let off = json!({"net": {"up": false, "media": "802.3", "bps": 0, "desc": "Lanternfall PCIe GbE Family Controller"}});
        assert!(checks(&adapters(&off), None).is_empty());
    }

    #[test]
    fn wifi_band_and_signal_come_from_netsh_when_windows_allows() {
        let interfaces = "\nThere is 1 interface on the system:\n\n    Name                   : Wi-Fi\n    State                  : connected\n    SSID                   : Nimbus Home\n    Radio type             : 802.11n\n    Band                   : 2.4 GHz\n    Channel                : 6\n    Receive rate (Mbps)    : 72\n    Signal                 : 42%\n";
        assert_eq!(parse_interface(interfaces), Some(("Nimbus Home".into(), 2.4, 42)));
        let networks = "SSID 1 : Atlas Guest\n    BSSID 1                 : 00:11\n         Band               : 5 GHz\nSSID 2 : Nimbus Home\n    BSSID 1                 : 00:22\n         Band               : 2.4 GHz\n    BSSID 2                 : 00:23\n         Channel            : 44\n";
        assert!(five_nearby(networks, "Nimbus Home"));
        assert!(!five_nearby(networks, "Copperline"), "another network's 5 GHz doesn't count");
        let five = checks(&[], Some(&Air { ghz: 2.4, signal: 80, five_nearby: true }));
        assert_eq!(five[0].title, "Your Wi-Fi is on 2.4 GHz, though your network also has 5 GHz");
        let weak = checks(&[], Some(&Air { ghz: 2.4, signal: 42, five_nearby: false }));
        assert_eq!(weak[0].title, "Your Wi-Fi signal is weak (42%) on 2.4 GHz");
        assert!(checks(&[], Some(&Air { ghz: 5.0, signal: 40, five_nearby: false })).is_empty());
        // Older Windows has no Band line: the channel tells.
        let old = interfaces.replace("    Band                   : 2.4 GHz\n", "");
        assert_eq!(parse_interface(&old).map(|a| a.1), Some(2.4));
        assert_eq!(parse_interface("    State : disconnected\n"), None);
    }

    #[test]
    fn without_netsh_a_slow_wifi_link_is_said_from_its_speed() {
        let f = json!({"net": {"up": true, "media": "Native 802.11", "bps": 72000000_u64, "desc": "Copperline Wireless"}});
        let c = checks(&adapters(&f), None);
        assert_eq!(c[0].title, "Your Wi-Fi connects at only 72 Mbps");
        let fast = json!({"net": {"up": true, "media": "Native 802.11", "bps": 866000000_u64}});
        assert!(checks(&adapters(&fast), None).is_empty());
    }
}
