//! Pictures for lists, as `data:` URIs (the widget's pages show only pictures they carry): a
//! program's own icon from its .exe, an extension's own picture from its folder.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

/// Program icons by lowercase .exe path, kept for as long as mom runs (they hardly change).
static CACHE: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

/// Each program's icon, 32 px, by its .exe path. At most 300 at a time.
pub fn programs(exes: &[String]) -> HashMap<String, Option<String>> {
    let mut out = HashMap::new();
    for exe in exes.iter().take(300) {
        let key = exe.to_lowercase();
        let hit = CACHE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).get(&key).cloned();
        let icon = match hit {
            Some(icon) => icon,
            None => {
                let icon = of_exe(Path::new(exe));
                CACHE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(key, icon.clone());
                icon
            }
        };
        out.insert(exe.clone(), icon);
    }
    out
}

fn of_exe(path: &Path) -> Option<String> {
    if !path.is_absolute() || !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) || !path.is_file() {
        return None;
    }
    let (w, h, rgba) = os::exe_icon(path, 32)?;
    Some(data_uri("image/png", &png(w, h, &rgba)))
}

/// An extension's picture: the smallest of its icons that's at least 32 px (or its biggest), if
/// it's a small PNG, JPEG or WebP inside its own folder.
pub fn extension(dir: &Path, manifest: &serde_json::Value) -> Option<String> {
    let mut sizes: Vec<(u32, &str)> = ["/icons", "/action/default_icon", "/browser_action/default_icon", "/page_action/default_icon"]
        .iter()
        .filter_map(|k| manifest.pointer(k).and_then(serde_json::Value::as_object))
        .flat_map(|o| o.iter().filter_map(|(size, file)| Some((size.parse().ok()?, file.as_str()?))))
        .collect();
    sizes.sort_unstable();
    let (_, file) = sizes.iter().find(|(s, _)| *s >= 32).or(sizes.last())?;
    let path = dir.join(file.trim_start_matches(['/', '\\']));
    // Only files inside the extension's folder.
    if !path.canonicalize().ok()?.starts_with(dir.canonicalize().ok()?) {
        return None;
    }
    let mime = match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => return None,
    };
    let bytes = std::fs::read(&path).ok().filter(|b| !b.is_empty() && b.len() <= 48 * 1024)?;
    Some(data_uri(mime, &bytes))
}

/// `data:<mime>;base64,...`.
pub fn data_uri(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", base64(bytes))
}

fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = u32::from(chunk[0]) << 16 | u32::from(*chunk.get(1).unwrap_or(&0)) << 8 | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            out.push(if i <= chunk.len() { ABC[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// A PNG of RGBA pixels, stored without compression: icons are tiny.
pub fn png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let row = width as usize * 4;
    let mut raw = Vec::with_capacity((row + 1) * height as usize);
    for line in rgba.chunks(row.max(1)).take(height as usize) {
        raw.push(0);
        raw.extend_from_slice(line);
    }
    // zlib: a header, stored blocks of up to 65,535 bytes, then Adler-32.
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = if raw.is_empty() { vec![&[]] } else { raw.chunks(65_535).collect() };
    for (i, block) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        let len = block.len() as u16;
        z.extend(len.to_le_bytes());
        z.extend((!len).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend(adler32(&raw).to_be_bytes());
    let mut header = Vec::with_capacity(13);
    header.extend(width.to_be_bytes());
    header.extend(height.to_be_bytes());
    header.extend([8, 6, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend(kind);
    out.extend(data);
    let crc = crc32(&out[start..]);
    out.extend(crc.to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in bytes {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in bytes {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    b << 16 | a
}

#[cfg(windows)]
mod os {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::null_mut;
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
    };
    use windows_sys::Win32::UI::Shell::SHDefExtractIconW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

    /// The .exe's first icon at `size` px, as RGBA rows.
    pub fn exe_icon(path: &Path, size: u32) -> Option<(u32, u32, Vec<u8>)> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let mut icon: HICON = null_mut();
        // SAFETY: `wide` is NUL-terminated and outlives the call; only the large icon is asked for.
        let hr = unsafe { SHDefExtractIconW(wide.as_ptr(), 0, 0, &mut icon, null_mut(), size) };
        if hr != 0 || icon.is_null() {
            return None;
        }
        let px = pixels(icon);
        // SAFETY: the icon was made for us above.
        unsafe { DestroyIcon(icon) };
        px
    }

    fn pixels(icon: HICON) -> Option<(u32, u32, Vec<u8>)> {
        // SAFETY: ICONINFO is plain data; GetIconInfo fills it and hands us both bitmaps to free.
        let mut info: ICONINFO = unsafe { std::mem::zeroed() };
        if unsafe { GetIconInfo(icon, &mut info) } == 0 {
            return None;
        }
        let color = read(info.hbmColor);
        let mask = read(info.hbmMask);
        // SAFETY: both bitmaps are ours to delete (null is ignored).
        unsafe {
            DeleteObject(info.hbmColor);
            DeleteObject(info.hbmMask);
        }
        let (w, h, mut px) = color?;
        // BGRA to RGBA. Old icons without alpha take it from their mask (black there is opaque).
        let has_alpha = px.chunks(4).any(|p| p[3] != 0);
        for (i, p) in px.chunks_mut(4).enumerate() {
            p.swap(0, 2);
            if !has_alpha {
                let opaque = mask.as_ref().is_some_and(|(_, _, m)| m.get(i * 4) == Some(&0));
                p[3] = if opaque { 255 } else { 0 };
            }
        }
        Some((w, h, px))
    }

    /// A bitmap's pixels, top row first, 32 bits each.
    fn read(bm: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
        if bm.is_null() {
            return None;
        }
        // SAFETY: BITMAP and BITMAPINFO are plain data; GetObjectW writes at most size_of::<BITMAP>().
        let mut b: BITMAP = unsafe { std::mem::zeroed() };
        if unsafe { GetObjectW(bm, std::mem::size_of::<BITMAP>() as i32, (&mut b as *mut BITMAP).cast()) } == 0 {
            return None;
        }
        let (w, h) = (b.bmWidth, b.bmHeight);
        if !(1..=256).contains(&w) || !(1..=512).contains(&h) {
            return None;
        }
        let mut bmi: BITMAPINFO = unsafe { std::mem::zeroed() };
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..unsafe { std::mem::zeroed() }
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        // SAFETY: `buf` holds exactly h rows of w 32-bit pixels, as `bmi` describes.
        let lines = unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let n = GetDIBits(dc, bm, 0, h as u32, buf.as_mut_ptr().cast(), &mut bmi, DIB_RGB_COLORS);
            DeleteDC(dc);
            n
        };
        (lines == h).then_some((w as u32, h as u32, buf))
    }
}

#[cfg(not(windows))]
mod os {
    pub fn exe_icon(_: &std::path::Path, _: u32) -> Option<(u32, u32, Vec<u8>)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_pads_like_everyone_else() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn a_png_with_the_right_checksums() {
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        let p = png(2, 1, &[255, 0, 0, 255, 0, 0, 255, 128]);
        assert!(p.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(p.ends_with(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]));
        // Each row gets its filter byte: 2 rows of 1 px is 10 bytes stored.
        let tall = png(1, 2, &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(u16::from_le_bytes([tall[8 + 25 + 8 + 3], tall[8 + 25 + 8 + 4]]), 10);
    }

    #[test]
    fn an_extension_picture_stays_inside_its_folder() {
        let root = std::env::temp_dir().join(format!("mom-icons-{}", std::process::id()));
        let dir = root.join("ext");
        std::fs::create_dir_all(dir.join("img")).unwrap();
        std::fs::write(dir.join("img").join("48.png"), png(1, 1, &[0, 0, 0, 255])).unwrap();
        std::fs::write(root.join("outside.png"), png(1, 1, &[0, 0, 0, 255])).unwrap();
        let manifest = serde_json::json!({ "icons": { "16": "img/16.png", "48": "img/48.png", "128": "../outside.png" } });
        assert!(extension(&dir, &manifest).is_some_and(|u| u.starts_with("data:image/png;base64,iVBORw0KGgo")));
        let outside = serde_json::json!({ "icons": { "128": "../outside.png" } });
        assert!(extension(&dir, &outside).is_none());
        std::fs::remove_dir_all(&root).ok();
    }
}
