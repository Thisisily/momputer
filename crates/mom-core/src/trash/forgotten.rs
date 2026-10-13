//! Big folders apps left behind: the data of apps you've uninstalled, folders of an app's older
//! versions, and ones nobody has touched in a long time.
//!
//! Matching a folder to its app is a guess (see installed.rs), so everything here moves to the
//! archive drive with a link left behind instead of being deleted: a wrong guess is undone with
//! Bring back, and the rest can be deleted for good from there once you're sure. Anything that
//! looks like keys, passwords, wallets or browser profiles is never offered.

use super::{ago, entries, name_of, tally_many, Cand, Found, TrashCtx, ENTRY_CAP, IN_USE};
use crate::installed::{squash, unversioned, Installed};
use std::path::{Path, PathBuf};

/// Smaller than this isn't worth a link.
const MIN_SIZE: u64 = 100 << 20;
/// An uninstalled app's data untouched this long is ticked; from 67 days it can be.
const LONG_AGO: u64 = 180 * 86_400;
const A_WHILE: u64 = 67 * 86_400;
/// An installed app's data untouched this long can be ticked.
const VERY_LONG_AGO: u64 = 365 * 86_400;
/// How many files mom counts at most, over everything, so measuring stays quick.
const BUDGET: u64 = 30_000_000;

/// Folders that belong to Windows, are shared by many apps, or have a clean-up of their own.
const SKIP: &[&str] = &[
    "Microsoft", "Packages", "Programs", "Temp", "Comms", "ConnectedDevicesPlatform", "D3DSCache", "CrashDumps",
    "IsolatedStorage", "PeerDistRepub", "PlaceholderTileLogoFolder", "Publishers", "VirtualStore", "History",
    "npm-cache", "npm", "pip", "pnpm", "pnpm-cache", "Yarn", "uv", "ms-playwright", "electron", "NuGet", "go-build",
    "node-gyp", "Docker", "Docker Desktop", "momputer", "claude-cli-nodejs", "OpenAI", "hermes", "Claude",
    ".cache", ".local", ".config", ".bun", ".cargo", ".rustup", ".npm", ".nuget", ".gradle", ".m2", ".vscode",
    ".vscode-insiders", ".cursor", ".claude", ".codex", ".agents", ".momputer", ".git", ".dotnet", ".docker",
    ".android", ".ollama", ".lmstudio",
    // looked after by other clean-ups (shader caches, developer caches, games, older versions)
    "NVIDIA", "NVIDIA Corporation", "AMD", "Intel", "Pub", ".dartServer", "Cypress", "ms-playwright-go", "SquirrelTemp",
    "0install.net", "JxBrowser", "UnityHub", "cache", "EpicGamesLauncher", "Epic Games", "Godot", "itch",
    // needed to uninstall or repair programs
    "Package Cache", "WinGet",
];

/// Anything named like this might hold keys, passwords, crypto wallets or browsing data: never
/// offered, however old. (Generous on purpose: a false match only means mom says nothing.)
const PRECIOUS: &[&str] = &[
    // keys and passwords
    "credential", "secret", "password", "bitwarden", "keepass", "1password", "lastpass", "dashlane", "authenticator",
    "authy", "keystore", "keychain", "gnupg", "ssh", "gpg", "pgp", "vault", "certs", "backup", "aws", "azure", "kube",
    "gcloud",
    // crypto wallets
    "wallet", "coin", "ledger", "trezor", "keepkey", "atomic", "exodus", "electrum", "metamask", "binance", "kraken",
    "bitcoin", "btc", "ethereum", "eth", "monero", "litecoin", "dogecoin", "zcash", "dash", "crypto", "chain", "guarda",
    "jaxx", "mycelium", "wasabi", "sparrow", "daedalus", "yoroi", "keplr", "phantom", "solflare", "tonkeeper", "rabby",
    "trust", "safepal", "bitbox", "armory", "bisq",
    // browsers and mail
    "mozilla", "firefox", "chrome", "chromium", "bravesoftware", "opera", "vivaldi", "edge", "thunderbird", "outlook",
    "mail", "shift",
    // messages
    "signal", "telegram", "whatsapp", "discord", "slack",
];

/// Vendors that keep each product (often each version) in a folder of its own.
const VENDORS: &[&str] = &["Google", "JetBrains", "Adobe", "Unity", "Unity Technologies", "Epic", "Autodesk", "Apple", "Qt", "QtProject", "NVIDIA"];

pub(super) fn precious(name: &str) -> bool {
    let n = squash(name);
    PRECIOUS.iter().any(|p| n.contains(p))
}

/// A folder worth a look: where it is, what to call it, who it belongs to.
struct Spot {
    path: PathBuf,
    name: String,
    /// The name to look the app up by (the product for a vendor's folder).
    owner: String,
    /// A newer version of the same thing is right next to it.
    newer: Option<String>,
    /// The vendor's folder it's in (Adobe for Adobe\UXP): if anything of theirs is installed, it
    /// may share this.
    vendor: Option<String>,
    /// One thing inside an app's folder (a model, a game), and what it is.
    thing: Option<&'static str>,
    /// In LocalLow: games keep their saves there, so it's never ticked for you.
    saves: bool,
}

/// Big things that live inside an app's own folder, each worth a look on its own:
/// (folder, how deep the things are, what they are, the apps they belong to).
fn things(ctx: &TrashCtx) -> Vec<(PathBuf, usize, &'static str, &'static str)> {
    let p = ctx.paths;
    vec![
        // LM Studio's (and Bionic's) downloaded models: models\<publisher>\<model>.
        (p.home.join(".lmstudio").join("models"), 2, "model", "LM Studio|Bionic"),
        // Games installed through itch.
        (p.appdata.join("itch").join("apps"), 1, "game", "itch"),
    ]
}

/// Every candidate: top-level folders in AppData and home's dot-folders, and each product in
/// a vendor's folder, minus what's skipped.
fn spots(ctx: &TrashCtx, agent_homes: &[PathBuf]) -> Vec<Spot> {
    let p = ctx.paths;
    let low = p.local_appdata.parent().map(|a| a.join("LocalLow")).unwrap_or_default();
    let roots: [(&Path, &str); 4] =
        [(&p.appdata, "AppData\\Roaming"), (&p.local_appdata, "AppData\\Local"), (&low, "AppData\\LocalLow"), (&p.home, "your home folder")];
    let mut out = Vec::new();
    let skipped = |d: &Path| {
        let n = name_of(d);
        SKIP.iter().any(|s| s.eq_ignore_ascii_case(&n))
            || precious(&n)
            || agent_homes.iter().any(|h| h.starts_with(d) || d.starts_with(h))
            || !std::fs::symlink_metadata(d).map(|m| m.is_dir()).unwrap_or(false)
    };
    for (root, where_) in roots {
        for d in entries(root) {
            let n = name_of(&d);
            if root == p.home.as_path() && !n.starts_with('.') {
                continue; // home's other folders are yours (Documents, projects), not an app's
            }
            if skipped(&d) {
                continue;
            }
            // LocalLow is always <company>\<game or app>, so every folder there is a vendor's.
            if VENDORS.iter().any(|v| v.eq_ignore_ascii_case(&n)) || where_.ends_with("LocalLow") {
                // Each product separately, and older versions of one marked as such.
                let products: Vec<PathBuf> = entries(&d).into_iter().filter(|x| !skipped(x)).collect();
                for x in &products {
                    let pn = name_of(x);
                    let base = squash(unversioned(&pn));
                    let newer = products
                        .iter()
                        .map(|y| name_of(y))
                        .filter(|y| *y != pn && squash(unversioned(y)) == base && version_of(y) > version_of(&pn))
                        .max_by(|a, b| version_of(a).cmp(&version_of(b)));
                    let saves = where_.ends_with("LocalLow");
                    out.push(Spot { path: x.clone(), name: format!("{n}\\{pn} ({where_})"), owner: pn.clone(), newer, vendor: Some(n.clone()), thing: None, saves });
                }
                continue;
            }
            // A Squirrel app's install (Update.exe and app-<version>) isn't data; its older
            // versions have a clean-up of their own.
            if d.join("Update.exe").is_file() {
                continue;
            }
            let saves = where_.ends_with("LocalLow");
            out.push(Spot { path: d.clone(), name: format!("{n} ({where_})"), owner: n, newer: None, vendor: None, thing: None, saves });
        }
    }
    for (root, depth, what, owners) in things(ctx) {
        let mut level = vec![(root.clone(), Vec::<String>::new())];
        for _ in 0..depth {
            level = level
                .into_iter()
                .flat_map(|(d, rel)| {
                    entries(&d).into_iter().filter(|x| x.is_dir()).map(move |x| {
                        let mut r = rel.clone();
                        r.push(name_of(&x));
                        (x, r)
                    })
                })
                .collect();
        }
        for (path, rel) in level {
            let name = format!("{} · {}", owners.split('|').next().unwrap_or(owners), rel.join(" / "));
            out.push(Spot { path, name, owner: owners.to_string(), newer: None, vendor: None, thing: Some(what), saves: false });
        }
    }
    out
}

/// "2023.1" from "PyCharm2023.1", as numbers to compare.
fn version_of(name: &str) -> Vec<u64> {
    let v = &name[unversioned(name).len()..];
    v.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect()
}

pub(super) fn find(ctx: &TrashCtx, agent_homes: &[PathBuf]) -> Found {
    find_with(ctx, agent_homes, &Installed::scan(ctx.paths), &super::builds::my_projects(&ctx.project_roots), MIN_SIZE)
}

fn find_with(ctx: &TrashCtx, agent_homes: &[PathBuf], installed: &Installed, mine: &[super::builds::Mine], min_size: u64) -> Found {
    let spots = spots(ctx, agent_homes);
    let paths: Vec<PathBuf> = spots.iter().map(|s| s.path.clone()).collect();
    let tallies = tally_many(&paths, ENTRY_CAP, BUDGET);
    let mut f = Found::none();
    f.available = true;
    f.root = ctx.cfg.archive_dir.clone();
    for (s, t) in spots.into_iter().zip(tallies) {
        // Too small, not fully counted (so its age isn't known), or holding links: left alone.
        if t.bytes < min_size.max(1) || !t.complete || t.links > 0 {
            continue;
        }
        let c = Cand::new(s.path.clone(), s.name.clone(), &t, ctx.now);
        let age = c.age;
        let here = s.owner.split('|').any(|o| installed.has(o)) || s.vendor.as_deref().is_some_and(|v| installed.has(v));
        // An app you're building was never installed, and it isn't gone: its data is your
        // project's, kept in AppData under the app's name or identifier.
        let project = if here || s.thing.is_some() { None } else { mine.iter().find(|m| m.owns(&s.owner)).map(|m| m.name.as_str()) };
        let c = if ctx.in_use(&s.path) {
            c.verdict(false, IN_USE)
        } else if let Some(newer) = &s.newer {
            let r = format!("An older version; {newer} is the one in use. Untouched for {}", ago(age));
            if age >= A_WHILE { c.verdict(true, r) } else { c.kept_but_tickable(r) }
        } else if let Some(what) = s.thing {
            // A model or a game: still yours to use, so only ever offered, never ticked for you.
            let r = match what {
                "model" => format!("A model downloaded {} ago", ago(age)),
                _ => format!("A {what} nobody has opened in {}", ago(age)),
            };
            if age >= LONG_AGO || (!here && age >= A_WHILE) { c.kept_but_tickable(r) } else { continue }
        } else if let Some(project) = project {
            // Like an installed app's: only offered, and only after a year untouched.
            if age < VERY_LONG_AGO {
                continue;
            }
            c.kept_but_tickable(format!("From your project {project}. Untouched for {}", ago(age)))
        } else if s.saves && !here && age >= A_WHILE {
            c.kept_but_tickable(format!("Probably a game's saves and settings; the game isn't installed any more. Untouched for {}", ago(age)))
        } else if s.saves {
            continue;
        } else if !here && age >= LONG_AGO {
            c.verdict(true, format!("Its app isn't installed any more. Untouched for {}", ago(age)))
        } else if !here && age >= A_WHILE {
            c.kept_but_tickable(format!("Its app isn't installed any more. Untouched for {}", ago(age)))
        } else if here && age >= VERY_LONG_AGO {
            c.kept_but_tickable(format!("Its app is still installed, but nobody has touched this in {}", ago(age)))
        } else {
            continue; // in use lately, or its app is: nothing to say
        };
        let mut c = c;
        c.tool = Some(match (s.thing, here, project) {
            (Some("model"), _, _) => "models you downloaded",
            (Some(_), _, _) => "games you installed",
            (None, true, _) => "apps you still have",
            (None, false, Some(_)) => "your own projects",
            (None, false, None) => "apps you've uninstalled",
        });
        f.cands.push(c);
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TrashConfig;
    use crate::paths::Paths;
    use crate::trash::tests::{ctx, fixture, write};

    #[test]
    fn uninstalled_apps_data_is_ticked_installed_apps_and_saves_only_offered_wallets_never() {
        let root = fixture("forgotten");
        let paths = Paths::under(&root);
        let (r, l) = (paths.appdata.clone(), paths.local_appdata.clone());
        let low = root.join("LocalLow");
        write(&r.join("Nimbus Studio").join("data.bin"), 900, 400); // uninstalled: ticked
        write(&r.join("Figma").join("cache.bin"), 800, 400); // installed, a year untouched: offered
        write(&r.join("Figma Recent").join("x.bin"), 800, 30); // used lately: nothing to say
        write(&r.join("Coinpocket").join("wallet.dat"), 700, 900); // a wallet: never
        write(&low.join("Pixel Forge").join("Swordship").join("save.dat"), 600, 400); // saves: only offered
        write(&l.join("Slack").join("Update.exe"), 10, 400); // a Squirrel install, not data
        write(&l.join("Slack").join("app-4.40.0").join("slack.exe"), 600, 400);
        write(&r.join("Google").join("AndroidStudio2021.2").join("options.xml"), 500, 100); // an older version
        write(&r.join("Google").join("AndroidStudio2024.2").join("options.xml"), 500, 10);
        write(&paths.home.join(".lmstudio").join("models").join("acme").join("tiny-7b").join("m.gguf"), 900, 200);
        let installed = Installed::with(&["Figma", "Bionic", "Android Studio"]);
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let f = find_with(&ctx, &[], &installed, &[], 100);
        let state = |part: &str| {
            f.cands.iter().find(|c| c.name.contains(part)).map(|c| (c.goes, c.optional)).unwrap_or_else(|| panic!("no {part} in {:#?}", f.cands.iter().map(|c| &c.name).collect::<Vec<_>>()))
        };
        assert_eq!(state("Nimbus Studio"), (true, false));
        assert_eq!(state("Figma (AppData"), (false, true));
        assert_eq!(state("Swordship"), (false, true));
        assert_eq!(state("AndroidStudio2021.2"), (true, false));
        assert_eq!(state("tiny-7b"), (false, true));
        for never in ["Coinpocket", "Figma Recent", "Slack", "AndroidStudio2024.2"] {
            assert!(!f.cands.iter().any(|c| c.name.contains(never)), "{never} shouldn't be offered");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_app_you_are_building_is_not_an_uninstalled_one() {
        let root = fixture("forgotten-mine");
        let paths = Paths::under(&root);
        let l = paths.local_appdata.clone();
        write(&l.join("com.lanternfall.app").join("EBWebView").join("cache.bin"), 900, 300); // by its Tauri identifier
        write(&l.join("Copperline").join("data.bin"), 900, 300); // by its folder's name
        write(&l.join("atlas-desk").join("data.bin"), 900, 300); // by the name in its package.json
        write(&l.join("com.lanternfall.beta").join("data.bin"), 900, 500); // over a year on: offered, never ticked
        write(&l.join("Nimbus Studio").join("data.bin"), 900, 300); // nobody's project: an uninstalled app's
        write(&l.join("com.elsewhere.app").join("data.bin"), 900, 300); // "app" says nothing about whose it is
        // Named the way mom names every folder she searches: by where it really is.
        std::fs::create_dir_all(root.join("Workspace")).unwrap();
        let ws = crate::util::real_path(&root.join("Workspace")).unwrap();
        let put = |p: PathBuf, text: &str| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        put(ws.join("lanternfall").join("package.json"), r#"{"name":"ui"}"#);
        put(ws.join("lanternfall").join("src-tauri").join("tauri.conf.json"), r#"{"identifier":"com.lanternfall.app","productName":"Lanternfall"}"#);
        put(ws.join("copperline").join("Cargo.toml"), "[package]\nname = \"cl\"\nversion = \"0.1.0\"\n");
        put(ws.join("tools").join("desk").join("package.json"), r#"{"name":"atlas-desk","build":{"appId":"dev.atlas.desk"}}"#);
        put(ws.join("app").join("package.json"), r#"{"name":"app"}"#);
        let cfg = TrashConfig::default();
        let ctx = ctx(&paths, &cfg);
        let nothing = Installed::with(&[]);
        let look = |mine: &[crate::trash::builds::Mine]| -> Vec<(String, bool, bool, String)> {
            let mut v: Vec<_> = find_with(&ctx, &[], &nothing, mine, 100).cands.iter().map(|c| (c.name.clone(), c.goes, c.optional, c.reason.clone())).collect();
            v.sort();
            v
        };
        // With no projects to go by, every one of them looks like an uninstalled app's.
        let before = look(&[]);
        assert_eq!(before.len(), 6, "{before:#?}");
        assert!(before.iter().all(|(_, goes, _, why)| *goes && why.starts_with("Its app isn't installed any more")), "{before:#?}");
        let mine = crate::trash::builds::my_projects(std::slice::from_ref(&ws));
        let after = look(&mine);
        let names: Vec<&str> = after.iter().map(|(n, ..)| n.as_str()).collect();
        assert_eq!(names, ["Nimbus Studio (AppData\\Local)", "com.elsewhere.app (AppData\\Local)", "com.lanternfall.beta (AppData\\Local)"], "{after:#?}");
        assert!(after[0].1 && after[1].1, "nobody's project: still an uninstalled app's");
        let (_, goes, optional, why) = &after[2];
        assert!(!goes && *optional && why.starts_with("From your project lanternfall. Untouched for"), "{after:#?}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn versions_compare_as_numbers() {
        assert!(version_of("PyCharm2024.2") > version_of("PyCharm2023.10"));
        assert!(version_of("AndroidStudio2022.3") > version_of("AndroidStudio4.2"));
        assert!(precious("Ledger Live") && precious("atomic") && precious(".ssh") && precious("Coinpocket") && !precious("Genymobile"));
    }
}
