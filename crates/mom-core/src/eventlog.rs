//! Windows' event logs, read for the checkup: one query at a time with `wevtutil`, from System32
//! (never a same-named program found on PATH), a cap on how many events come back, and the
//! answer "can't read it" kept apart from "nothing happened".

use crate::util::{run, windows_powershell};
use std::time::Duration;

/// One event, as much of it as the checks use.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Ev {
    pub provider: String,
    pub id: u32,
    /// Unix seconds.
    pub at: u64,
    /// Its named data fields, in order.
    pub data: Vec<(String, String)>,
    /// Its unnamed data fields, joined: some programs (chkdsk) write their whole message there.
    pub text: String,
}

impl Ev {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.data.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

/// The longest a data field is kept: messages can run to pages.
const FIELD_CHARS: usize = 2_000;

/// A program in System32, by the path Windows reports, not PATH.
pub(crate) fn system_program(name: &str) -> Option<String> {
    let ps = windows_powershell().ok()?;
    let system32 = std::path::Path::new(&ps).ancestors().nth(3)?;
    Some(system32.join(name).to_str()?.to_owned())
}

/// The newest `max` events of `log` that match `xpath`. None when the log can't be read (most
/// need no administrator, some do); Some(empty) when nothing matched.
pub(crate) fn query(log: &str, xpath: &str, max: u32) -> Option<Vec<Ev>> {
    let wevtutil = system_program("wevtutil.exe")?;
    let q = format!("/q:{xpath}");
    let c = format!("/c:{}", max.clamp(1, 2_000));
    let out = run(&wevtutil, &["qe", log, &q, "/f:xml", &c, "/rd:true"], Duration::from_secs(10)).ok()?;
    if !out.ok() {
        return None;
    }
    Some(parse(&out.text()))
}

/// Reads `wevtutil qe /f:xml` output: one <Event> after another.
pub(crate) fn parse(xml: &str) -> Vec<Ev> {
    xml.split("<Event ").skip(1).filter_map(one).collect()
}

fn one(e: &str) -> Option<Ev> {
    let provider = attr(e, "Provider Name=")?;
    let id = between(e, "<EventID", "</EventID>").and_then(|s| s.rsplit('>').next()?.trim().parse().ok())?;
    let at = crate::speed::parse_time(&attr(e, "SystemTime=")?)?;
    let mut data = Vec::new();
    let mut text = Vec::new();
    let mut rest = e;
    while let Some(i) = rest.find("<Data") {
        rest = &rest[i + 5..];
        // <Data/> and <Data Name='x'/> are empty.
        let Some(close) = rest.find('>') else { break };
        let head = &rest[..close];
        if head.ends_with('/') {
            rest = &rest[close + 1..];
            continue;
        }
        let body_start = close + 1;
        let Some(end) = rest[body_start..].find("</Data>") else { break };
        let value: String = unescape(rest[body_start..body_start + end].trim()).chars().take(FIELD_CHARS).collect();
        match attr(head, "Name=") {
            Some(name) => data.push((name, value)),
            None => text.push(value),
        }
        rest = &rest[body_start + end..];
    }
    Some(Ev { provider, id, at, data, text: text.join(" ") })
}

fn attr(s: &str, key: &str) -> Option<String> {
    let at = s.find(key)? + key.len();
    let quote = s[at..].chars().next().filter(|c| *c == '\'' || *c == '"')?;
    let rest = &s[at + 1..];
    Some(unescape(&rest[..rest.find(quote)?]))
}

fn between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = s.find(open)?;
    let end = s[start..].find(close)? + start;
    Some(&s[start..end])
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&#xD;", "").replace("&#xA;", " ").replace("&amp;", "&")
}

/// A day on the PC's clock, for grouping: the Unix second its midnight was.
pub(crate) fn local_midnight(at: u64) -> u64 {
    use chrono::{Local, TimeZone};
    let Some(t) = Local.timestamp_opt(at as i64, 0).single() else { return at - at % 86_400 };
    let Some(midnight) = t.date_naive().and_hms_opt(0, 0, 0) else { return at - at % 86_400 };
    Local.from_local_datetime(&midnight).earliest().map(|m| m.timestamp().max(0) as u64).unwrap_or(at - at % 86_400)
}

/// The hour (0–23) on the PC's clock.
pub(crate) fn local_hour(at: u64) -> u32 {
    use chrono::{Local, TimeZone, Timelike};
    Local.timestamp_opt(at as i64, 0).single().map(|t| t.hour()).unwrap_or(12)
}

/// "14 Oct", on the PC's clock.
pub(crate) fn short_date(at: u64) -> String {
    use chrono::{Local, TimeZone};
    Local.timestamp_opt(at as i64, 0).single().map(|t| t.format("%-d %b").to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_read_with_named_and_unnamed_data() {
        let xml = r#"<Events><Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='Microsoft-Windows-Ntfs' Guid='{x}'/><EventID>98</EventID><TimeCreated SystemTime='2026-10-10T08:00:00.0000000Z'/></System><EventData><Data Name='DriveName'>F:</Data><Data Name='CorruptionActionState'>2</Data><Data Name='Empty'/></EventData></Event>
<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='Chkdsk'/><EventID Qualifiers='0'>26226</EventID><TimeCreated SystemTime='2026-10-11T09:30:00.000Z'/></System><EventData><Data>Checking file system on \Device\HarddiskVolume7&#xD;&#xA;Volume label is Nimbus &amp; Co.</Data></EventData></Event></Events>"#;
        let e = parse(xml);
        assert_eq!(e.len(), 2);
        assert_eq!((e[0].provider.as_str(), e[0].id, e[0].get("DriveName"), e[0].get("CorruptionActionState")), ("Microsoft-Windows-Ntfs", 98, Some("F:"), Some("2")));
        assert_eq!(e[0].get("Empty"), None, "an empty field is skipped");
        assert_eq!((e[1].provider.as_str(), e[1].id), ("Chkdsk", 26226));
        assert_eq!(e[1].text, r"Checking file system on \Device\HarddiskVolume7 Volume label is Nimbus & Co.");
        assert!(e[1].at > e[0].at);
        assert!(parse("Access is denied.").is_empty());
    }

    #[test]
    fn days_and_hours_follow_the_clock() {
        let t = 1_791_900_000; // a Tuesday in October 2026
        let m = local_midnight(t);
        assert!(m <= t && t - m < 86_400);
        assert_eq!(local_hour(m), 0);
        assert!(!short_date(t).is_empty());
    }
}
