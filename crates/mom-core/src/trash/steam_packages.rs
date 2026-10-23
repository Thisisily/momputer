//! Steam's update packages (Steam\package). Each version of Steam comes as a set of zips that a
//! manifest names. Steam installs a downloaded set when it restarts and clears out older sets by
//! itself, so the sets its manifests name are Steam's, and only a zip none of them names is left
//! over.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

/// What a manifest is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Set {
    /// The version Steam runs (`steam_client_win64.manifest`).
    Installed,
    /// One it downloaded and installs when it restarts (`steam_client_win64`, no extension).
    Waiting,
}

/// Steam's packages: each set its manifests name (the manifest, and the zips that are there),
/// and the zips no manifest names. None when the folder can't be read in full, since then a zip
/// can't be called a leftover.
pub(super) struct Packages {
    pub sets: Vec<(Set, PathBuf, Vec<PathBuf>)>,
    pub leftovers: Vec<PathBuf>,
}

const MANIFEST_BYTES: u64 = 1 << 20;
const MAX_FILES: usize = 2_000;

pub(super) fn read(package: &Path) -> Option<Packages> {
    let mut files = Vec::new();
    for e in std::fs::read_dir(package).ok()? {
        let e = e.ok()?;
        if e.file_type().ok()?.is_file() {
            files.push(e.file_name().to_string_lossy().to_string());
        }
        if files.len() > MAX_FILES {
            return None;
        }
    }
    files.sort();
    let mut named: HashSet<String> = HashSet::new();
    let mut sets = Vec::new();
    // The installed set first, so a zip both name counts as the one Steam runs. Each kind is one
    // set, whatever number of manifests (64-bit and 32-bit) name its zips.
    for set in [Set::Installed, Set::Waiting] {
        let manifests: Vec<&String> = files.iter().filter(|f| kind_of(f) == Some(set)).collect();
        let mut zips = Vec::new();
        for manifest in &manifests {
            let names = zips_in(&package.join(manifest))?;
            zips.extend(names.into_iter().filter(|z| files.contains(z) && named.insert(z.to_ascii_lowercase())).map(|z| package.join(z)));
        }
        if let (Some(first), false) = (manifests.first(), zips.is_empty()) {
            sets.push((set, package.join(first), zips));
        }
    }
    let leftovers = files.iter().filter(|f| f.contains(".zip") && !named.contains(&f.to_ascii_lowercase())).map(|f| package.join(f)).collect();
    Some(Packages { sets, leftovers })
}

/// "steam_client_win64.manifest" is the installed version's manifest, "steam_client_win64" a
/// downloaded one's. Steam's other records (".installed") aren't manifests.
fn kind_of(name: &str) -> Option<Set> {
    let rest = name.strip_prefix("steam_client_")?;
    match rest.strip_suffix(".manifest") {
        Some(platform) if plain(platform) => Some(Set::Installed),
        Some(_) => None,
        None => plain(rest).then_some(Set::Waiting),
    }
}

fn plain(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The files a manifest names: its `"file"` and `"zipvz"` lines. None if it can't be read.
fn zips_in(manifest: &Path) -> Option<Vec<String>> {
    let mut text = String::new();
    std::fs::File::open(manifest).ok()?.take(MANIFEST_BYTES).read_to_string(&mut text).ok()?;
    Some(
        text.lines()
            .filter_map(|line| {
                let mut quoted = line.split('"').skip(1).step_by(2);
                let key = quoted.next()?;
                let value = quoted.next()?;
                (key == "file" || key == "zipvz").then(|| value.to_string())
            })
            // A plain name in the same folder, never a path.
            .filter(|v| !v.is_empty() && !v.contains(['/', '\\', ':']) && v != "." && v != "..")
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(names: &[&str]) -> String {
        let mut s = String::from("\"win64\"\n{\n\t\"version\"\t\t\"1\"\n");
        for (i, n) in names.iter().enumerate() {
            s.push_str(&format!("\t\"part{i}\"\n\t{{\n\t\t\"file\"\t\t\"{n}.unpacked\"\n\t\t\"zipvz\"\t\t\"{n}\"\n\t}}\n"));
        }
        s + "}\n"
    }

    #[test]
    fn sets_steam_uses_stay_and_a_zip_no_manifest_names_is_left_over() {
        let dir = std::env::temp_dir().join(format!("mom-steam-packages-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let w = |n: &str, s: &str| std::fs::write(dir.join(n), s).unwrap();
        w("steam_client_win64.manifest", &manifest(&["bins.zip.vz.aaa_1", "ui.zip.vz.bbb_1"]));
        w("steam_client_win64", &manifest(&["bins.zip.vz.ccc_1", "ui.zip.vz.bbb_1", r"..\escape.zip"]));
        w("steam_client_win64.installed", "bin\\,-1;0;0");
        for z in ["bins.zip.vz.aaa_1", "ui.zip.vz.bbb_1", "bins.zip.vz.ccc_1", "old.zip.vz.ddd_1"] {
            w(z, "zip");
        }
        let p = read(&dir).unwrap();
        let names = |v: &[PathBuf]| v.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect::<Vec<_>>();
        assert_eq!(p.sets.len(), 2);
        assert_eq!((p.sets[0].0, names(&p.sets[0].2)), (Set::Installed, vec!["bins.zip.vz.aaa_1".to_string(), "ui.zip.vz.bbb_1".into()]));
        assert_eq!((p.sets[1].0, names(&p.sets[1].2)), (Set::Waiting, vec!["bins.zip.vz.ccc_1".to_string()]), "a shared zip is the installed one's");
        assert_eq!(names(&p.leftovers), vec!["old.zip.vz.ddd_1"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_steam_client_manifests_count() {
        assert_eq!(kind_of("steam_client_win64.manifest"), Some(Set::Installed));
        assert_eq!(kind_of("steam_client_win64"), Some(Set::Waiting));
        assert_eq!(kind_of("steam_client_win64.installed"), None);
        assert_eq!(kind_of("bins_win64.zip.vz.aaa_1"), None);
    }
}
