//! Platform-neutral description of a HID top-level collection.

/// One HID top-level collection as enumerated by the operating system.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HidInfo {
    pub path: String,
    pub vid: u16,
    pub pid: u16,
    /// USB interface number (`&mi_xx` in the path), `-1` if the device has only one.
    pub interface: i32,
    pub usage_page: u16,
    pub usage: u16,
    /// Report lengths in bytes including the report id; `0` if the collection has none.
    pub input_len: usize,
    pub output_len: usize,
    pub feature_len: usize,
    pub bluetooth: bool,
}

impl HidInfo {
    /// Key of the physical device: all its collections and interfaces share it.
    pub fn group(&self) -> String {
        format!(
            "{:04x}:{:04x}:{}",
            self.vid,
            self.pid,
            if self.bluetooth { "bt" } else { "usb" }
        )
    }
}

/// Vendor and product id from a device interface path
/// (`\\?\hid#vid_046d&pid_c548&mi_02&col01#…`, Bluetooth uses `vid&0002054c_pid&0ce6`).
pub fn vid_pid_from_path(path: &str) -> Option<(u16, u16)> {
    let lower = path.to_ascii_lowercase();
    let hex4 = |s: &str| s.get(..4).and_then(|h| u16::from_str_radix(h, 16).ok());
    let after = |haystack: &str, key: &str| -> Option<String> {
        let rest = &haystack[haystack.find(key)? + key.len()..];
        let rest = rest.trim_start_matches(['_', '&']);
        Some(rest.to_string())
    };
    let vid_part = after(&lower, "vid")?;
    // Bluetooth paths prefix the vendor id with its source (`0002054c`, `02046d`).
    let vid_digits: String = vid_part
        .chars()
        .take_while(char::is_ascii_hexdigit)
        .collect();
    let vid = hex4(vid_digits.get(vid_digits.len().checked_sub(4)?..)?)?;
    let pid = hex4(&after(&vid_part, "pid")?)?;
    Some((vid, pid))
}

/// USB interface number from a device interface path.
pub fn interface_from_path(path: &str) -> i32 {
    let lower = path.to_ascii_lowercase();
    lower
        .find("&mi_")
        .and_then(|at| lower.get(at + 4..at + 6))
        .and_then(|digits| i32::from_str_radix(digits, 16).ok())
        .unwrap_or(-1)
}

/// Is this collection reached over Bluetooth (Classic HID profile or LE HOGP)?
pub fn is_bluetooth_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("{00001124-0000-1000-8000-00805f9b34fb}")
        || lower.contains("{00001812-0000-1000-8000-00805f9b34fb}")
        || lower.contains("bthledevice")
}

/// Drop the leading zero that Windows puts in front of reports of devices without report ids.
pub fn strip_report_id_zero(report: &[u8]) -> &[u8] {
    match report.split_first() {
        Some((0, rest)) => rest,
        _ => report,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_path() {
        let p = r"\\?\hid#vid_046d&pid_c548&mi_02&col01#8&2f4a&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_eq!(vid_pid_from_path(p), Some((0x046D, 0xC548)));
        assert_eq!(interface_from_path(p), 2);
        assert!(!is_bluetooth_path(p));
    }

    #[test]
    fn bluetooth_path() {
        let p = r"\\?\hid#{00001124-0000-1000-8000-00805f9b34fb}_vid&0002054c_pid&0ce6&col01#9&1&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_eq!(vid_pid_from_path(p), Some((0x054C, 0x0CE6)));
        assert_eq!(interface_from_path(p), -1);
        assert!(is_bluetooth_path(p));
    }

    #[test]
    fn bluetooth_le_path() {
        let p = r"\\?\hid#{00001812-0000-1000-8000-00805f9b34fb}_dev_vid&02046d_pid&b023_rev&0007_d1bfa1d6f2e3&col02#a&1&0&0001#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_eq!(vid_pid_from_path(p), Some((0x046D, 0xB023)));
        assert!(is_bluetooth_path(p));
    }

    #[test]
    fn garbage_path() {
        assert_eq!(vid_pid_from_path(r"\\?\root#system#0000"), None);
    }

    #[test]
    fn report_id_zero() {
        assert_eq!(strip_report_id_zero(&[0, 1, 2]), &[1, 2]);
        assert_eq!(strip_report_id_zero(&[0x31, 1, 2]), &[0x31, 1, 2]);
        assert!(strip_report_id_zero(&[]).is_empty());
    }
}
