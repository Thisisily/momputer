//! Steam's own list of the apps an account has come across (`appcache\appinfo.vdf`): each app's
//! name, install folder and the programs it launches. It's how a game's leftover cache gets its
//! real name ("Lanternfall: Night Shift" rather than lanternfall-win64-shipping.exe), and how mom
//! tells a game that was uninstalled from one that's merely somewhere she didn't look.
//!
//! The file is Steam's binary key-value format, versions 28 and 29 (29 keeps keys in a string
//! table at the end). Everything is read with bounds checks; anything unexpected stops the read,
//! and then nothing from it is used.

use std::collections::HashMap;

#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct App {
    pub name: String,
    /// Its folder under steamapps\common.
    pub folder: String,
    /// The programs it launches, as lowercase file names ("game-win64-shipping.exe").
    pub programs: Vec<String>,
}

const V28: u32 = 0x0756_4428;
const V29: u32 = 0x0756_4429;
/// Info state, last updated, access token, text hash, change number, binary hash.
const ENTRY_HEADER: usize = 4 + 4 + 8 + 20 + 4 + 20;
const MAX_DEPTH: usize = 32;
const MAX_KEYS: usize = 4_000_000;
const MAX_APPS: usize = 500_000;

/// Every app in the file, by app id. Nothing if the file isn't one this reads.
pub(super) fn parse(bytes: &[u8]) -> Option<HashMap<u32, App>> {
    let mut r = Reader { b: bytes, p: 0 };
    let magic = r.u32()?;
    if magic != V28 && magic != V29 {
        return None;
    }
    let _universe = r.u32()?;
    let table = if magic == V29 { Some(string_table(bytes, usize::try_from(r.u64()?).ok()?)?) } else { None };
    let mut apps = HashMap::new();
    loop {
        let id = r.u32()?;
        if id == 0 {
            break;
        }
        if apps.len() >= MAX_APPS {
            return None;
        }
        let size = r.u32()? as usize;
        let end = r.p.checked_add(size)?;
        if end > bytes.len() || size < ENTRY_HEADER {
            return None;
        }
        let mut inner = Reader { b: &bytes[..end], p: r.p + ENTRY_HEADER };
        let mut app = App::default();
        let mut keys = Vec::new();
        object(&mut inner, table.as_deref(), &mut keys, &mut app, 0)?;
        if !app.name.is_empty() {
            app.programs.sort();
            app.programs.dedup();
            apps.insert(id, app);
        }
        r.p = end;
    }
    Some(apps)
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let end = self.p.checked_add(n)?;
        let s = self.b.get(self.p..end)?;
        self.p = end;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    /// A NUL-terminated string.
    fn text(&mut self) -> Option<&str> {
        let rest = self.b.get(self.p..)?;
        let n = rest.iter().position(|&c| c == 0)?;
        let s = std::str::from_utf8(&rest[..n]).ok();
        self.p += n + 1;
        Some(s.unwrap_or(""))
    }
}

fn string_table(bytes: &[u8], at: usize) -> Option<Vec<String>> {
    let mut r = Reader { b: bytes, p: at };
    let count = r.u32()? as usize;
    if count > MAX_KEYS {
        return None;
    }
    let mut out = Vec::with_capacity(count.min(1 << 16));
    for _ in 0..count {
        out.push(r.text()?.to_string());
    }
    Some(out)
}

/// Reads one object's fields up to its end marker, noting the ones an `App` keeps. `keys` is the
/// path of keys to this object (lowercase).
fn object(r: &mut Reader, table: Option<&[String]>, keys: &mut Vec<String>, app: &mut App, depth: usize) -> Option<()> {
    if depth > MAX_DEPTH {
        return None;
    }
    loop {
        let kind = r.u8()?;
        if kind == 0x08 || kind == 0x0B {
            return Some(());
        }
        let key = match table {
            Some(t) => t.get(r.u32()? as usize)?.to_ascii_lowercase(),
            None => r.text()?.to_ascii_lowercase(),
        };
        match kind {
            0x00 => {
                keys.push(key);
                object(r, table, keys, app, depth + 1)?;
                keys.pop();
            }
            0x01 => {
                let value = r.text()?;
                keep(keys, &key, value, app);
            }
            0x02 | 0x03 | 0x04 | 0x06 => {
                r.take(4)?;
            }
            0x07 | 0x0A => {
                r.take(8)?;
            }
            0x05 => {
                // A wide string: UTF-16 up to a NUL pair.
                loop {
                    if r.take(2)? == [0, 0] {
                        break;
                    }
                }
            }
            _ => return None,
        }
    }
}

fn keep(keys: &[String], key: &str, value: &str, app: &mut App) {
    let at = |path: &[&str]| keys.len() == path.len() && keys.iter().zip(path).all(|(a, b)| a == b);
    if key == "name" && at(&["appinfo", "common"]) && app.name.is_empty() {
        app.name = clean(value);
    } else if key == "installdir" && at(&["appinfo", "config"]) {
        app.folder = clean(value);
    } else if key == "executable" && keys.len() == 4 && keys[..3] == ["appinfo", "config", "launch"] {
        let file = value.rsplit(['\\', '/']).next().unwrap_or("").trim().to_ascii_lowercase();
        if file.ends_with(".exe") && file.len() > 4 && file.len() <= 260 && !file.chars().any(char::is_control) {
            app.programs.push(file);
        }
    }
}

/// A name to show: one line, not absurdly long.
fn clean(value: &str) -> String {
    value.chars().filter(|c| !c.is_control()).take(200).collect::<String>().trim().to_string()
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// Binary key-values, v29 style: keys go in a string table.
    #[derive(Default)]
    struct Kv {
        keys: Vec<String>,
        out: Vec<u8>,
    }

    impl Kv {
        fn key(&mut self, k: &str) {
            let i = self.keys.iter().position(|x| x == k).unwrap_or_else(|| {
                self.keys.push(k.to_string());
                self.keys.len() - 1
            });
            self.out.extend((i as u32).to_le_bytes());
        }
        fn open(&mut self, k: &str) {
            self.out.push(0x00);
            self.key(k);
        }
        fn text(&mut self, k: &str, v: &str) {
            self.out.push(0x01);
            self.key(k);
            self.out.extend(v.as_bytes());
            self.out.push(0);
        }
        fn int(&mut self, k: &str, v: u32) {
            self.out.push(0x02);
            self.key(k);
            self.out.extend(v.to_le_bytes());
        }
        fn long(&mut self, k: &str, v: u64) {
            self.out.push(0x07);
            self.key(k);
            self.out.extend(v.to_le_bytes());
        }
        fn end(&mut self) {
            self.out.push(0x08);
        }
    }

    /// A v29 file with these apps: (id, name, install folder, launched programs).
    pub(crate) fn file(apps: &[(u32, &str, &str, &[&str])]) -> Vec<u8> {
        let mut kv = Kv::default();
        let mut body = Vec::new();
        for (id, name, folder, programs) in apps {
            kv.out.clear();
            kv.open("appinfo");
            kv.int("appid", *id);
            kv.open("common");
            kv.text("name", name);
            kv.long("gameid", u64::from(*id));
            kv.end();
            kv.open("config");
            kv.text("installdir", folder);
            kv.open("launch");
            for (i, program) in programs.iter().enumerate() {
                kv.open(&i.to_string());
                kv.text("executable", program);
                kv.end();
            }
            kv.end(); // launch
            kv.end(); // config
            kv.end(); // appinfo
            kv.end(); // root
            body.extend(id.to_le_bytes());
            body.extend(((ENTRY_HEADER + kv.out.len()) as u32).to_le_bytes());
            body.extend([0u8; ENTRY_HEADER]);
            body.extend(&kv.out);
        }
        body.extend(0u32.to_le_bytes());
        let table_at = 16 + body.len();
        let mut out = Vec::new();
        out.extend(V29.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        out.extend((table_at as u64).to_le_bytes());
        out.extend(body);
        out.extend((kv.keys.len() as u32).to_le_bytes());
        for k in &kv.keys {
            out.extend(k.as_bytes());
            out.push(0);
        }
        out
    }

    #[test]
    fn names_folders_and_launched_programs_are_read() {
        let bytes = file(&[
            (480, "Lanternfall", "Lanternfall", &[r"Lanternfall\Binaries\Win64\Lanternfall-Win64-Shipping.exe", "Launcher.exe"]),
            (970, "Copperline: Night Shift", "CopperlineNS", &["copperline/game.EXE"]),
        ]);
        let apps = parse(&bytes).unwrap();
        assert_eq!(apps.len(), 2);
        let a = &apps[&480];
        assert_eq!((a.name.as_str(), a.folder.as_str()), ("Lanternfall", "Lanternfall"));
        assert_eq!(a.programs, ["lanternfall-win64-shipping.exe", "launcher.exe"]);
        assert_eq!(apps[&970].programs, ["game.exe"]);
    }

    #[test]
    fn anything_unexpected_reads_as_nothing() {
        let good = file(&[(480, "Lanternfall", "Lanternfall", &["Lanternfall.exe"])]);
        assert!(parse(&good[..good.len() - 3]).is_none(), "cut short");
        let mut wrong = good.clone();
        wrong[0] = 0x27;
        assert!(parse(&wrong).is_none(), "another format");
        let mut bad_type = good.clone();
        let at = 16 + 8 + ENTRY_HEADER; // the first field's type byte
        bad_type[at] = 0x42;
        assert!(parse(&bad_type).is_none(), "a type it doesn't know");
        assert!(parse(&[]).is_none() && parse(&[0x29, 0x44]).is_none());
    }
}
