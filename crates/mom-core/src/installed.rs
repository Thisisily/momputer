//! What's installed: programs in Windows' uninstall list, apps in %LOCALAPPDATA%\Programs and
//! Program Files, and Store apps. Used to tell whether a data folder's app is still around.
//!
//! Matching a folder like `%APPDATA%\Bionic` or `~\.lmstudio` to a program is guesswork, so it
//! leans towards "installed": a wrong guess then only means mom leaves something alone.

use crate::paths::Paths;
use std::path::{Path, PathBuf};

#[derive(Default, Debug)]
pub struct Installed {
    /// Every name an installed program goes by, squashed ("visualstudiocode", "jetbrainstoolbox").
    names: Vec<String>,
    /// Single words of those names ("unity", "android"), matched only whole.
    words: Vec<String>,
    /// Where installed programs live.
    pub dirs: Vec<PathBuf>,
}

/// Lowercase letters and digits only: "JetBrains Toolbox" -> "jetbrainstoolbox".
pub fn squash(s: &str) -> String {
    s.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}

/// A name without a version on the end: "PyCharm2023.1" -> "PyCharm", "AndroidStudio2022.3" -> "AndroidStudio".
pub fn unversioned(name: &str) -> &str {
    let cut = name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.' || c == '-' || c == '_' || c == ' ');
    if cut.len() >= 3 { cut } else { name }
}

impl Installed {
    pub fn scan(paths: &Paths) -> Installed {
        let mut me = Installed::default();
        #[cfg(windows)]
        {
            use crate::health::reg::{string_values, subkeys, Hive};
            const UNINSTALL: &[(Hive, &str)] = &[
                (Hive::LocalMachine, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
                (Hive::LocalMachine, r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
                (Hive::CurrentUser, r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
            ];
            for (hive, root) in UNINSTALL {
                for key in subkeys(*hive, root) {
                    let values = string_values(*hive, &format!(r"{root}\{key}"));
                    let get = |n: &str| values.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.trim().trim_matches('"').to_string());
                    let Some(name) = get("DisplayName").filter(|n| !n.is_empty()) else { continue };
                    me.add_name(&name);
                    if let Some(p) = get("Publisher") {
                        me.add_name(&p);
                    }
                    if let Some(dir) = get("InstallLocation").filter(|d| !d.is_empty()) {
                        me.add_dir(PathBuf::from(dir));
                    }
                    // Its uninstaller usually sits in its own folder.
                    if let Some(u) = get("UninstallString") {
                        let exe = if let Some(rest) = u.strip_prefix('"') { rest.split('"').next().unwrap_or("") } else { u.split(".exe").next().unwrap_or("") };
                        if let Some(dir) = Path::new(exe).parent().filter(|d| d.components().count() > 1) {
                            me.add_dir(dir.to_path_buf());
                        }
                    }
                    if let Some(icon) = get("DisplayIcon") {
                        // "C:\Program Files\App\app.exe,0" -> its folder.
                        let exe = icon.split(',').next().unwrap_or("").trim_matches('"');
                        if let Some(dir) = Path::new(exe).parent().filter(|d| d.components().count() > 1) {
                            me.add_dir(dir.to_path_buf());
                        }
                    }
                }
            }
        }
        let program_files: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
            .iter()
            .filter_map(|k| std::env::var_os(k).map(PathBuf::from))
            .chain([paths.local_appdata.join("Programs")])
            .collect();
        for root in program_files {
            for app in dirs_in(&root) {
                // Vendors keep each product in its own folder: Google\Chrome, JetBrains\PyCharm 2024.2.
                for product in dirs_in(&app) {
                    me.add_name(&name_of(&product));
                }
                me.add_dir(app);
            }
        }
        // Package managers that don't register with Windows: Chocolatey, Scoop, winget.
        let choco = std::env::var_os("ChocolateyInstall").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\ProgramData\chocolatey"));
        for pkg in dirs_in(&choco.join("lib")).into_iter().chain(dirs_in(&paths.home.join("scoop").join("apps"))) {
            me.add_name(&name_of(&pkg));
        }
        for pkg in dirs_in(&paths.local_appdata.join("Microsoft").join("WinGet").join("Packages")) {
            // "nektos.act_Microsoft.Winget.Source_8wekyb3d8bbwe" -> "nektos", "act".
            for part in name_of(&pkg).split('_').next().unwrap_or("").split('.') {
                me.add_name(part);
            }
        }
        // Tools that are only on PATH (Flutter unzipped into C:\flutter, say).
        if let Some(path) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path) {
                let under = |d: &Path| d.components().count() > 2;
                for d in [Some(dir.as_path()), dir.parent()].into_iter().flatten().filter(|d| under(d)) {
                    let n = name_of(d);
                    if !n.eq_ignore_ascii_case("bin") {
                        me.add_name(&n);
                    }
                }
            }
        }
        // Store apps: "OpenAI.Codex_2p2nqsd0c76g0" -> "OpenAI", "Codex".
        for pkg in dirs_in(&paths.local_appdata.join("Packages")) {
            let family = name_of(&pkg);
            for part in family.split('_').next().unwrap_or("").split('.') {
                me.add_name(part);
            }
        }
        me.names.sort();
        me.names.dedup();
        me.words.sort();
        me.words.dedup();
        me
    }

    fn add_name(&mut self, name: &str) {
        let s = squash(unversioned(name));
        if s.len() >= 3 {
            self.names.push(s);
        }
        // Each word too, for "Unity Hub" or "Android Studio".
        for w in name.split(|c: char| !c.is_ascii_alphanumeric()) {
            let w = squash(unversioned(w));
            if w.len() >= 4 {
                self.words.push(w);
            }
        }
    }

    /// Its folder's name and the vendor folder above it ("Android" for Android\Android Studio),
    /// unless that's just Program Files or the like.
    fn add_dir(&mut self, dir: PathBuf) {
        self.add_name(&name_of(&dir));
        if let Some(parent) = dir.parent() {
            let n = name_of(parent);
            let plain = ["program files", "program files (x86)", "programs", "local", "appdata", "users", "tools", "bin", ""];
            if !plain.contains(&n.to_ascii_lowercase().as_str()) {
                self.add_name(&n);
            }
        }
        self.dirs.push(dir);
    }

    /// Is something installed that goes by this folder's name? `~\.lmstudio`, `%APPDATA%\Bionic`,
    /// `JetBrains\PyCharm2023.1`. Too short to tell (under 3 letters) counts as installed.
    pub fn has(&self, folder: &str) -> bool {
        let want = squash(unversioned(folder.trim_start_matches('.')));
        if want.len() < 3 {
            return true;
        }
        // A whole name either way round ("jetbrainstoolbox" has "jetbrains"; "pycharmce" has
        // "pycharm"), but a single word only as itself, or "studio" would claim "lmstudio".
        self.words.contains(&want)
            || self.names.iter().any(|n| n == &want || (want.len() >= 4 && n.contains(&want)) || (n.len() >= 5 && want.contains(n.as_str())))
    }

    /// For tests and for apps mom knows by other names.
    pub fn with(names: &[&str]) -> Installed {
        let mut me = Installed::default();
        for n in names {
            me.add_name(n);
        }
        me
    }
}

fn dirs_in(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(Result::ok).filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false)).map(|e| e.path()).collect())
        .unwrap_or_default()
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_match_programs_by_name_leaning_towards_installed() {
        let i = Installed::with(&["Visual Studio Code", "JetBrains Toolbox", "PyCharm 2024.2.1", "Bionic", "Unity Hub 3.7.0"]);
        assert!(i.has("PyCharm2023.1"), "an older version's folder belongs to the same program");
        assert!(i.has("Bionic"));
        assert!(i.has("JetBrains"));
        assert!(i.has("UnityHub"));
        assert!(i.has("go"), "too short to tell");
        assert!(!i.has(".lmstudio"));
        assert!(!i.has("Genymobile"));
        assert_eq!(unversioned("AndroidStudio2022.3"), "AndroidStudio");
        assert_eq!(squash("Code - Insiders"), "codeinsiders");
    }

    #[test]
    fn this_pc_has_something_installed() {
        let i = Installed::scan(&Paths::detect());
        #[cfg(windows)]
        assert!(!i.names.is_empty() && !i.words.is_empty());
        let _ = i;
    }
}
