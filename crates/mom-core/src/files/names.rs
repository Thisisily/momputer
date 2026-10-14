//! Conservative display preference among files already proved byte-identical.
//!
//! A version in a filename says nothing about whether a path is safe to remove.
//! This helper must never create duplicate groups or enable cleanup selection.

use std::path::Path;

/// The unique highest explicit version, only when every path belongs to one filename
/// family in the same parent directory. Otherwise preserve the caller's usual preference.
///
/// Call this once on the whole group after its ordinary sort. A conditional pairwise
/// version comparator would not be transitive when mixed with unrelated names/folders.
pub(super) fn preferred_version(paths: &[&Path]) -> Option<usize> {
    if paths.len() < 2 {
        return None;
    }
    let parent = paths[0].parent()?;
    let mut parsed = Vec::with_capacity(paths.len());
    for path in paths {
        // Exact lexical equality is deliberately conservative: this pure helper cannot
        // establish that aliases, differently spelled parents, or links are one directory.
        if path.parent()? != parent {
            return None;
        }
        parsed.push(parse(path)?);
    }
    if parsed.iter().any(|p| p.family != parsed[0].family || p.extension != parsed[0].extension) {
        return None;
    }
    let highest = parsed.iter().map(|p| &p.version).max()?;
    let mut matches = parsed.iter().enumerate().filter(|(_, p)| &p.version == highest);
    let (index, _) = matches.next()?;
    matches.next().is_none().then_some(index)
}

struct NamedVersion {
    family: (String, String),
    extension: Option<String>,
    version: Vec<u64>,
}

fn parse(path: &Path) -> Option<NamedVersion> {
    let extension = match path.extension() {
        Some(extension) => Some(extension.to_str()?.to_lowercase()),
        None => None,
    };
    // An extension made only of numbers might really be the last dotted version part.
    if extension.as_ref().is_some_and(|x| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let stem = path.file_stem()?.to_str()?.to_lowercase();
    let mut found = None;
    for (start, ch) in stem.char_indices() {
        if !matches!(ch, 'v' | 'r') || stem[..start].chars().next_back().is_some_and(char::is_alphanumeric) {
            continue;
        }
        let tail = &stem[start..];
        let Some(marker) = ["version", "ver", "rev", "v"].into_iter().find(|marker| tail.starts_with(*marker)) else { continue };
        let mut end = start + marker.len();
        while stem.as_bytes().get(end).is_some_and(u8::is_ascii_whitespace) {
            end += 1;
        }
        if !stem.as_bytes().get(end).is_some_and(u8::is_ascii_digit) {
            continue;
        }
        let mut version = Vec::new();
        loop {
            let number = end;
            while stem.as_bytes().get(end).is_some_and(u8::is_ascii_digit) {
                end += 1;
            }
            version.push(stem[number..end].parse::<u64>().ok()?);
            if stem.as_bytes().get(end) != Some(&b'.') {
                break;
            }
            end += 1;
            if !stem.as_bytes().get(end).is_some_and(u8::is_ascii_digit) {
                return None; // a malformed/ambiguous dotted version
            }
        }
        if stem[end..].chars().next().is_some_and(char::is_alphanumeric) {
            return None; // v2beta is not an unambiguous numeric version token
        }
        if found.is_some() {
            return None; // e.g. "report v2 rev3": more than one possible version axis
        }
        while version.len() > 1 && version.last() == Some(&0) {
            version.pop();
        }
        let family = (stem[..start].to_string(), stem[end..].to_string());
        if !family.0.chars().chain(family.1.chars()).any(char::is_alphanumeric) {
            return None; // "v1.pdf" alone does not identify a document family
        }
        found = Some(NamedVersion { family, extension: extension.clone(), version });
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn prefer(names: &[&str]) -> Option<usize> {
        let paths: Vec<PathBuf> = names.iter().map(|name| Path::new("same-folder").join(name)).collect();
        preferred_version(&paths.iter().map(PathBuf::as_path).collect::<Vec<_>>())
    }

    #[test]
    fn explicit_numeric_versions_choose_the_unique_highest() {
        assert_eq!(prefer(&["report-v2.pdf", "report-v10.pdf", "report-v9.pdf"]), Some(1));
        assert_eq!(prefer(&["report_v1.9.8.pdf", "report_v1.10.pdf", "report_v1.2.10.pdf"]), Some(1));
        assert_eq!(prefer(&["Report version 2.PDF", "report ver 3.pdf", "REPORT rev4.pdf"]), Some(2));
        assert_eq!(prefer(&["drawing rev2-final.pdf", "drawing rev3-final.pdf"]), Some(1));
    }

    #[test]
    fn download_suffixes_and_unversioned_copies_have_no_preference() {
        assert_eq!(prefer(&["report (1).pdf", "report (2).pdf"]), None);
        assert_eq!(prefer(&["report.pdf", "report (1).pdf"]), None);
        assert_eq!(prefer(&["report v2.pdf", "report.pdf"]), None);
        assert_eq!(prefer(&["report 2.pdf", "report 10.pdf"]), None);
        assert_eq!(prefer(&["report v1 (1).pdf", "report v2 (2).pdf"]), None);
    }

    #[test]
    fn folders_families_and_extensions_must_all_match() {
        let a = Path::new("working/report v1.pdf");
        let b = Path::new("backup/report v2.pdf");
        assert_eq!(preferred_version(&[a, b]), None);
        assert_eq!(prefer(&["report v1.pdf", "contract v2.pdf"]), None);
        assert_eq!(prefer(&["report v1.pdf", "report v2.docx"]), None);
        assert_eq!(prefer(&["report v1-draft.pdf", "report v2-final.pdf"]), None);
    }

    #[test]
    fn ties_multiple_tokens_and_malformed_versions_are_ambiguous() {
        assert_eq!(prefer(&["report v2.pdf", "report v2.0.pdf"]), None);
        assert_eq!(prefer(&["report v02.pdf", "report v2.pdf"]), None);
        assert_eq!(prefer(&["report v1 rev2.pdf", "report v2 rev3.pdf"]), None);
        assert_eq!(prefer(&["report v1..2.pdf", "report v2..2.pdf"]), None);
        assert_eq!(prefer(&["report v1beta.pdf", "report v2beta.pdf"]), None);
        assert_eq!(prefer(&["report v18446744073709551616.pdf", "report v2.pdf"]), None);
        assert_eq!(prefer(&["v1.pdf", "v2.pdf"]), None);
        assert_eq!(prefer(&["report-v1.2", "report-v2.2"]), None);
    }

    #[test]
    fn version_markers_need_a_word_boundary_and_allow_unicode_families() {
        assert_eq!(prefer(&["overview1.pdf", "overview2.pdf"]), None);
        assert_eq!(prefer(&["résumé v1.pdf", "résumé v2.pdf"]), Some(1));
        assert_eq!(prefer(&["report prev1.pdf", "report prev2.pdf"]), None);
    }

    #[test]
    fn group_preference_is_independent_of_existing_display_order() {
        for names in [
            ["report v1.pdf", "report v10.pdf", "report v2.pdf"],
            ["report v2.pdf", "report v1.pdf", "report v10.pdf"],
            ["report v10.pdf", "report v2.pdf", "report v1.pdf"],
        ] {
            assert_eq!(names[prefer(&names).unwrap()], "report v10.pdf");
        }
        assert_eq!(prefer(&[]), None);
        assert_eq!(prefer(&["report v1.pdf"]), None);
    }
}
