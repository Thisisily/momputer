//! What a drive says about itself, asked without administrator rights. An NVMe drive hands
//! its health page (wear, temperature, spare blocks, errors) to anyone who asks through
//! Windows' storage query; SATA drives keep theirs behind commands only an administrator may
//! send, so for those mom has Windows' own verdict and nothing more.

use serde::{Deserialize, Serialize};

/// An NVMe drive's SMART / health information page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Nvme {
    /// The drive's own warning flags (0: none). Bit 0 spare low, 1 temperature, 2 worn or
    /// unreliable, 3 read-only, 4 its power-loss backup failed.
    pub critical: u8,
    /// Degrees Celsius.
    pub temp: i32,
    /// Spare blocks left, in percent, and the level its maker calls too low.
    pub spare: u8,
    pub spare_min: u8,
    /// Rated life used, in percent. It keeps counting past 100.
    pub used: u8,
    pub written_bytes: u64,
    pub hours: u64,
    pub unsafe_shutdowns: u64,
    /// Data it couldn't read back or write correctly.
    pub media_errors: u64,
}

impl Nvme {
    /// What the warning flags mean, in plain words.
    pub fn warnings(&self) -> Vec<&'static str> {
        [
            (1, "its spare blocks are nearly used up"),
            (2, "it's outside its safe temperature"),
            (4, "it says it's worn or no longer reliable"),
            (8, "it has gone read-only"),
            (16, "its power-loss protection has failed"),
        ]
        .into_iter()
        .filter(|(bit, _)| self.critical & bit != 0)
        .map(|(_, text)| text)
        .collect()
    }
}

/// Reads the 512-byte health page (NVMe log page 02h).
pub fn parse_health(page: &[u8]) -> Option<Nvme> {
    if page.len() < 512 {
        return None;
    }
    // 128-bit counters; anything past 64 bits is more than a drive will ever count.
    let wide = |at: usize| -> u64 {
        if page[at + 8..at + 16].iter().any(|b| *b != 0) {
            u64::MAX
        } else {
            u64::from_le_bytes(page[at..at + 8].try_into().unwrap_or_default())
        }
    };
    let kelvin = u16::from_le_bytes([page[1], page[2]]);
    // A page of zeros is a drive that didn't answer.
    if kelvin == 0 && page[3] == 0 && page[5] == 0 {
        return None;
    }
    Some(Nvme {
        critical: page[0],
        temp: i32::from(kelvin) - 273,
        spare: page[3],
        spare_min: page[4],
        used: page[5],
        // Counted in thousands of 512-byte blocks.
        written_bytes: wide(48).saturating_mul(512_000),
        hours: wide(128),
        unsafe_shutdowns: wide(144),
        media_errors: wide(160),
    })
}

/// The health page of physical drive `number` ("\\.\PhysicalDrive1"), if it's an NVMe drive
/// that answers. Opens the drive with no access rights at all: only the question is asked.
#[cfg(windows)]
pub fn nvme_health(number: u32) -> Option<Nvme> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows_sys::Win32::System::IO::DeviceIoControl;
    const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D_1400;
    // STORAGE_PROPERTY_QUERY { StorageDeviceProtocolSpecificProperty, PropertyStandardQuery } followed by
    // STORAGE_PROTOCOL_SPECIFIC_DATA { ProtocolTypeNvme, NVMeDataTypeLogPage, page 2, sub-value 0,
    // data offset 40, data length 512, ... } and room for the page.
    let header: [u32; 12] = [50, 0, 3, 2, 2, 0, 40, 512, 0, 0, 0, 0];
    let mut buf = vec![0u8; 48 + 512];
    for (i, v) in header.iter().enumerate() {
        buf[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    let path = crate::util::wide(&format!(r"\\.\PhysicalDrive{number}"));
    // SAFETY: `path` is NUL-terminated; the handle is closed below; `buf` is both the request and
    // the reply and is as long as we say it is.
    unsafe {
        let h = CreateFileW(path.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut());
        if h == INVALID_HANDLE_VALUE || h.is_null() {
            return None;
        }
        let mut got = 0u32;
        let ok = DeviceIoControl(h, IOCTL_STORAGE_QUERY_PROPERTY, buf.as_ptr().cast(), buf.len() as u32, buf.as_mut_ptr().cast(), buf.len() as u32, &mut got, std::ptr::null_mut());
        CloseHandle(h);
        if ok == 0 || (got as usize) < buf.len() {
            return None;
        }
    }
    parse_health(&buf[48..])
}

#[cfg(not(windows))]
pub fn nvme_health(_: u32) -> Option<Nvme> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> Vec<u8> {
        let mut p = vec![0u8; 512];
        p[1..3].copy_from_slice(&(59u16 + 273).to_le_bytes());
        p[3] = 100;
        p[4] = 10;
        p[5] = 12;
        p[48..56].copy_from_slice(&337_890_625u64.to_le_bytes());
        p[128..136].copy_from_slice(&31_394u64.to_le_bytes());
        p[144..152].copy_from_slice(&107u64.to_le_bytes());
        p
    }

    #[test]
    fn a_health_page_read_plainly() {
        let n = parse_health(&page()).unwrap();
        assert_eq!((n.temp, n.spare, n.spare_min, n.used), (59, 100, 10, 12));
        assert_eq!(n.written_bytes, 337_890_625 * 512_000);
        assert_eq!((n.hours, n.unsafe_shutdowns, n.media_errors, n.critical), (31_394, 107, 0, 0));
        assert!(n.warnings().is_empty());
        assert!(parse_health(&[0u8; 512]).is_none(), "a page of zeros is no answer");
        assert!(parse_health(&[0u8; 100]).is_none());
    }

    #[test]
    fn warning_flags_and_counters_too_big_to_matter() {
        let mut p = page();
        p[0] = 0b0000_0101;
        p[160..168].copy_from_slice(&3u64.to_le_bytes());
        p[136] = 1; // hours beyond 64 bits
        let n = parse_health(&p).unwrap();
        assert_eq!(n.warnings(), vec!["its spare blocks are nearly used up", "it says it's worn or no longer reliable"]);
        assert_eq!(n.media_errors, 3);
        assert_eq!(n.hours, u64::MAX);
    }
}
