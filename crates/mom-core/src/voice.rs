//! Mom's voice. Warm, and on your side: she notices, she offers to help, and she says well
//! done. She never tells anyone off, about leftovers or anything else.
//!
//! Lines are picked deterministically from the numbers so the widget doesn't change its mind
//! every three seconds.

use crate::util::human_bytes;
use serde::Serialize;

/// How she looks, by how much there is to help with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Face {
    /// Nothing left running.
    Proud,
    /// Nothing left behind, though something has been running a long while.
    Fine,
    /// One or two things.
    Hmm,
    /// A handful, or a couple of dozen: she's ready to help.
    Helping,
    /// A lot: sleeves rolled up.
    Busy,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mood {
    pub face: Face,
    pub level: u8,
    pub line: String,
}

fn pick<'a>(options: &[&'a str], seed: usize) -> &'a str {
    options[seed % options.len()]
}

pub fn mood(leftovers: usize, leftover_mem: u64, forgotten: usize) -> Mood {
    let mem = human_bytes(leftover_mem);
    let n = leftovers;
    let (face, level, line) = match n {
        0 if forgotten == 0 => (
            Face::Proud,
            0,
            pick(
                &["All tidy. I'm proud of you, sweetie.", "Clean house. Go play.", "Nothing left running. Lovely work."],
                (leftover_mem / 4096) as usize,
            )
            .to_string(),
        ),
        0 => (
            Face::Fine,
            1,
            if forgotten == 1 {
                "Looking good. One thing has been running a long while, if you'd like a look.".to_string()
            } else {
                format!("Looking good. {forgotten} things have been running a long while, if you'd like a look.")
            },
        ),
        1 => (Face::Hmm, 1, "One thing is still running. I can turn it off for you.".to_string()),
        2 => (Face::Hmm, 1, "Two things are still running. Want me to turn them off?".to_string()),
        3..=9 => (
            Face::Helping,
            2,
            pick(
                &[
                    "{n} things are still running. I'll tidy up whenever you're ready.",
                    "{n} leftovers. One tap and they're gone.",
                    "{n} things running that nobody's using. Happy to turn them off.",
                ],
                n,
            )
            .replace("{n}", &n.to_string()),
        ),
        10..=24 => (
            Face::Helping,
            3,
            pick(
                &[
                    "{n} leftovers are using {mem}. Let's get that memory back.",
                    "{n} things running for nobody, using {mem}. I've got this whenever you say.",
                ],
                n,
            )
            .replace("{n}", &n.to_string())
            .replace("{mem}", &mem),
        ),
        _ => (
            Face::Busy,
            4,
            "{n} leftovers are using {mem}. Busy day! Let's clear them together."
                .replace("{n}", &n.to_string())
                .replace("{mem}", &mem),
        ),
    };
    Mood { face, level, line }
}

pub fn stopped(n: usize, freed: u64) -> String {
    match n {
        0 => "Nothing needed turning off.".to_string(),
        1 => format!("Turned it off. {} back for you.", human_bytes(freed)),
        _ => format!("Turned off {n} things. {} back for you.", human_bytes(freed)),
    }
}

pub fn cleaned(freed: u64) -> String {
    if freed == 0 {
        "Already spotless.".to_string()
    } else {
        format!("Took out {} of trash.", human_bytes(freed))
    }
}

/// What mom says about a room. Whatever the grade, she's glad to help.
pub fn room(grade: char, name: &str) -> String {
    let this = if name.eq_ignore_ascii_case("desktop") { "this desktop" } else { "this folder" };
    match grade {
        'A' => "Spotless. Beautifully kept.".into(),
        'B' => "Looking good. Just a few things to put away.".into(),
        'C' => "A good start. Let's put a few more things away.".into(),
        'D' => format!("We'll have {this} sorted in no time c:"),
        _ => format!("Let's clean {this} together c:"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mood_scales_with_the_mess() {
        assert_eq!(mood(0, 0, 0).face, Face::Proud);
        assert_eq!(mood(1, 10, 0).face, Face::Hmm);
        assert!(mood(5, 0, 0).line.contains('5'));
        let m = mood(14, 3 * 1024 * 1024 * 1024, 0);
        assert!(m.line.contains("14") && m.line.contains("3.0 GB"), "{}", m.line);
        assert_eq!(mood(40, 0, 0).face, Face::Busy);
        assert_eq!((mood(5, 0, 0).face, mood(14, 0, 0).face), (Face::Helping, Face::Helping));
    }

    #[test]
    fn she_never_tells_anyone_off() {
        // Whatever the numbers, what she says helps: no blame, no sarcasm, no telling-off.
        const NEVER: &[&str] = &[
            "mad", "disappoint", "barn", "raised", "young", "counting", "hogging", "who left", "pretend", "grow on trees", "lights. off",
            "you're welcome", "mess", "shame", "bad", "again?", "how many", "told you",
        ];
        let mut said: Vec<String> = Vec::new();
        for n in 0..60 {
            for mem in [0u64, 4096, 8192, 3 << 30] {
                for forgotten in [0, 1, 4] {
                    said.push(mood(n, mem, forgotten).line);
                }
            }
            said.push(stopped(n, 1 << 20));
        }
        said.extend(['A', 'B', 'C', 'D', 'F'].iter().flat_map(|g| [room(*g, "Desktop"), room(*g, "Downloads")]));
        said.push(cleaned(0));
        said.push(cleaned(5 << 20));
        for line in &said {
            let low = line.to_lowercase();
            assert!(!NEVER.iter().any(|w| low.contains(w)), "{line}");
        }
    }

    #[test]
    fn same_numbers_same_line() {
        assert_eq!(mood(7, 1234, 0).line, mood(7, 1234, 0).line);
    }
}
