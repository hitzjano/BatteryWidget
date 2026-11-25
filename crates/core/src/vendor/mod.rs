//! Battery protocols of non-Logitech peripherals, spoken over their USB receiver or cable.
//!
//! Only documented *read* requests are sent, and only to endpoints identified by vendor
//! id, product id and report layout. Every driver consists of a pure part (matching,
//! packet building, decoding; unit-tested everywhere) and a Windows-only `read` function.

use crate::hid::HidInfo;
#[cfg(windows)]
use crate::model::Reading;

pub mod asus;
pub mod corsair;
pub mod hyperx;
pub mod jbl;
pub mod playstation;
pub mod razer;
pub mod steelseries;

/// One vendor protocol.
pub struct Driver {
    pub name: &'static str,
    pub vendor_ids: &'static [u16],
    /// The device legitimately stays silent at times (asleep, headset switched off), so
    /// an unanswered query must not lock it out for long.
    pub silence_is_normal: bool,
    /// Does this driver handle the device made of these collections?
    pub matches: fn(&[HidInfo]) -> bool,
    /// Query the device. `product` is its USB product string (may be empty).
    #[cfg(windows)]
    pub read: fn(&[HidInfo], &str) -> Vec<Reading>,
}

pub static DRIVERS: [Driver; 11] = [
    corsair::HEADSET,
    corsair::V2W,
    corsair::BRAGI,
    razer::DRIVER,
    steelseries::DRIVER,
    hyperx::DRIVER,
    hyperx::PULSEFIRE,
    hyperx::KINGSTON_HEADSET,
    playstation::DRIVER,
    asus::DRIVER,
    jbl::DRIVER,
];

/// Is any driver interested in devices of this vendor?
pub fn is_known_vendor(vid: u16) -> bool {
    DRIVERS.iter().any(|d| d.vendor_ids.contains(&vid))
}

/// The driver responsible for a device, given all of its HID collections.
pub fn driver_for(group: &[HidInfo]) -> Option<&'static Driver> {
    let vid = group.first()?.vid;
    DRIVERS
        .iter()
        .find(|d| d.vendor_ids.contains(&vid) && (d.matches)(group))
}

/// Stable per-device suffix for ids: the instance part of the interface path.
pub(crate) fn instance_of(info: &HidInfo) -> String {
    info.path.split('#').nth(2).unwrap_or("0").to_lowercase()
}

#[cfg(test)]
pub(crate) fn collection(
    vid: u16,
    pid: u16,
    usage_page: u16,
    usage: u16,
    interface: i32,
    lens: (usize, usize, usize),
) -> HidInfo {
    HidInfo {
        path: format!(
            r"\\?\hid#vid_{vid:04x}&pid_{pid:04x}&mi_{interface:02x}#7&abc&0&0000#{{guid}}"
        ),
        vid,
        pid,
        interface,
        usage_page,
        usage,
        input_len: lens.0,
        output_len: lens.1,
        feature_len: lens.2,
        bluetooth: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drivers_are_selected_by_vendor_and_layout() {
        let dualsense = [collection(0x054C, 0x0CE6, 1, 5, 3, (64, 48, 64))];
        assert_eq!(driver_for(&dualsense).unwrap().name, "PlayStation");

        let razer = [collection(0x1532, 0x007B, 1, 2, 0, (9, 0, 91))];
        assert_eq!(driver_for(&razer).unwrap().name, "Razer");

        let void_headset = [collection(0x1B1C, 0x0A14, 0xFFC5, 1, 3, (64, 64, 0))];
        assert_eq!(driver_for(&void_headset).unwrap().name, "Corsair headset");

        let slipstream = [collection(0x1B1C, 0x1BA6, 0xFF42, 1, 1, (65, 65, 0))];
        assert_eq!(driver_for(&slipstream).unwrap().name, "Corsair Bragi");

        let unknown_sony = [collection(0x054C, 0x1234, 1, 5, 0, (64, 0, 0))];
        assert!(driver_for(&unknown_sony).is_none());
        let keris = [collection(0x0B05, 0x1960, 0xFF01, 1, 0, (65, 65, 0))];
        assert_eq!(driver_for(&keris).unwrap().name, "ASUS ROG");

        let quantum = [collection(0x0ECB, 0x2127, 0xFF12, 1, 5, (64, 0, 64))];
        assert_eq!(driver_for(&quantum).unwrap().name, "JBL Quantum");

        assert!(driver_for(&[]).is_none());
        assert!(is_known_vendor(0x1038) && !is_known_vendor(0x046D));
    }

    #[test]
    fn instance_suffix() {
        let info = collection(0x054C, 0x0CE6, 1, 5, 3, (64, 48, 64));
        assert_eq!(instance_of(&info), "7&abc&0&0000");
    }
}
