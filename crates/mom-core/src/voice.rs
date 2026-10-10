//! Mom's voice. Warm, a little sassy, never in the way.
//!
//! Lines are picked deterministically from the numbers so the widget doesn't change its mind
//! every three seconds.

use crate::util::human_bytes;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Face {
    Proud,
    Fine,
    Hmm,
    Disappointed,
    Furious,
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
                &[
                    "All tidy. I'm proud of you, sweetie.",
                    "Clean house. Go play.",
                    "Nothing left running. Who raised you? Oh right, me.",
                ],
                (leftover_mem / 4096) as usize,
            )
            .to_string(),
        ),
        0 => (
            Face::Fine,
            1,
            if forgotten == 1 {
                "Tidy enough. One thing has been running a long time though.".to_string()
            } else {
                format!("Tidy enough. {forgotten} things have been running a long time though.")
            },
        ),
        1 => (Face::Hmm, 1, "Somebody left the lights on.".to_string()),
        2 => (Face::Hmm, 1, "Two things left running. I'll pretend I didn't see. Once.".to_string()),
        3..=9 => (
            Face::Disappointed,
            2,
            pick(
                &[
                    "Who left {n} things running? I'm not mad. I'm disappointed.",
                    "{n} leftovers. Were you raised in a barn?",
                    "{n} things running and nobody's using them. Lights. Off.",
                ],
                n,
            )
            .replace("{n}", &n.to_string()),
        ),
        10..=24 => (
            Face::Disappointed,
            3,
            pick(
                &[
                    "{n} leftovers eating {mem}. Does memory grow on trees?",
                    "{n} things running for nobody. That's {mem} of RAM, young agent.",
                ],
                n,
            )
            .replace("{n}", &n.to_string())
            .replace("{mem}", &mem),
        ),
        _ => (
            Face::Furious,
            4,
            "{n} leftovers hogging {mem}. I'm counting to three."
                .replace("{n}", &n.to_string())
                .replace("{mem}", &mem),
        ),
    };
    Mood { face, level, line }
}

pub fn stopped(n: usize, freed: u64) -> String {
    match n {
        0 => "Nothing needed turning off.".to_string(),
        1 => format!("Turned it off. {} back. You're welcome.", human_bytes(freed)),
        _ => format!("Turned off {n} things. {} back. You're welcome.", human_bytes(freed)),
    }
}

pub fn cleaned(freed: u64) -> String {
    if freed == 0 {
        "Already spotless.".to_string()
    } else {
        format!("Took out {} of trash.", human_bytes(freed))
    }
}

pub fn room(grade: char) -> &'static str {
    match grade {
        'A' => "Spotless. Who are you and what have you done with my kid?",
        'B' => "Not bad. Make your bed.",
        'C' => "I can see the floor. Barely.",
        'D' => "This room is a disaster. We're cleaning it before dinner.",
        _ => "I'm not cleaning this. You are.",
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
        assert_eq!(mood(40, 0, 0).face, Face::Furious);
    }

    #[test]
    fn same_numbers_same_line() {
        assert_eq!(mood(7, 1234, 0).line, mood(7, 1234, 0).line);
    }
}
