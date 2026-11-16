//! Razer wireless mice and keyboards (the 90-byte feature report protocol documented by
//! the OpenRazer project).

use super::Driver;
use crate::hid::HidInfo;

pub const VID_RAZER: u16 = 0x1532;
/// Report id byte plus the 90-byte Razer report.
pub const FEATURE_LEN: usize = 91;
/// Different product generations expect different transaction ids.
pub const TRANSACTION_IDS: [u8; 5] = [0x1F, 0x3F, 0xFF, 0x9F, 0x08];
pub const CLASS_POWER: u8 = 0x07;
pub const CMD_BATTERY_LEVEL: u8 = 0x80;
pub const CMD_CHARGING_STATUS: u8 = 0x84;

const STATUS_BUSY: u8 = 0x01;
const STATUS_OK: u8 = 0x02;

fn matches(group: &[HidInfo]) -> bool {
    group
        .iter()
        .any(|i| i.vid == VID_RAZER && i.feature_len == FEATURE_LEN)
}

pub const DRIVER: Driver = Driver {
    name: "Razer",
    vendor_ids: &[VID_RAZER],
    silence_is_normal: true,
    matches,
    #[cfg(windows)]
    read,
};

/// Feature report for a "get" command with a two-byte argument area.
pub fn request(transaction_id: u8, class: u8, command: u8) -> [u8; FEATURE_LEN] {
    let mut packet = [0u8; FEATURE_LEN];
    // packet[0] is the HID report id (0); the Razer report starts at packet[1].
    let report = &mut packet[1..];
    report[1] = transaction_id;
    report[5] = 2; // data size
    report[6] = class;
    report[7] = command;
    report[88] = report[2..88].iter().fold(0, |crc, b| crc ^ b);
    packet
}

/// What a reply to [`request`] says.
#[derive(Debug, PartialEq, Eq)]
pub enum Response {
    /// The first argument byte after the reserved one, i.e. the requested value.
    Value(u8),
    Busy,
    Invalid,
}

pub fn parse(feature: &[u8], class: u8, command: u8) -> Response {
    let Some(report) = feature.get(1..FEATURE_LEN) else {
        return Response::Invalid;
    };
    match report[0] {
        STATUS_BUSY => Response::Busy,
        STATUS_OK if report[6] == class && report[7] == command => Response::Value(report[9]),
        _ => Response::Invalid,
    }
}

/// Raw level (0–255) to percent.
pub fn percent(raw: u8) -> u8 {
    ((u16::from(raw) * 100 + 127) / 255) as u8
}

#[cfg(windows)]
fn query(device: &crate::win::hid::HidDevice, transaction_id: u8, command: u8) -> Option<u8> {
    if !device.set_feature(&request(transaction_id, CLASS_POWER, command)) {
        return None;
    }
    for _ in 0..10 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        match parse(&device.get_feature(0)?, CLASS_POWER, command) {
            Response::Value(value) => return Some(value),
            Response::Busy => continue,
            Response::Invalid => return None,
        }
    }
    None
}

#[cfg(windows)]
fn read(group: &[HidInfo], product: &str) -> Vec<crate::model::Reading> {
    use crate::model::{Kind, Link, Reading};
    use crate::names::{clean_product_name, kind_from_name};
    use crate::win::hid::HidDevice;

    let mut candidates: Vec<&HidInfo> = group
        .iter()
        .filter(|i| i.feature_len == FEATURE_LEN)
        .collect();
    candidates.sort_by_key(|i| i.interface);
    for info in candidates {
        let Ok(device) = HidDevice::open_for_features(info) else {
            continue;
        };
        for transaction_id in TRANSACTION_IDS {
            let Some(raw) = query(&device, transaction_id, CMD_BATTERY_LEVEL) else {
                continue;
            };
            let charging = query(&device, transaction_id, CMD_CHARGING_STATUS) == Some(1);
            if raw == 0 && !charging {
                // The dongle answers for a mouse that is switched off.
                return Vec::new();
            }
            let name = clean_product_name(product, "Razer");
            let kind = match kind_from_name(&name) {
                Kind::Generic => Kind::Mouse,
                kind => kind,
            };
            // A wireless Razer device only charges while it is on its cable.
            let link = if charging { Link::Usb } else { Link::Rf24 };
            let id = format!("razer:{:04x}:{}", info.pid, super::instance_of(info));
            return vec![Reading::new(id, name, kind, link)
                .battery(Some(percent(raw)))
                .charging(charging)];
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_layout_and_checksum() {
        let p = request(0x1F, CLASS_POWER, CMD_BATTERY_LEVEL);
        assert_eq!(p.len(), 91);
        assert_eq!(p[0], 0, "HID report id");
        assert_eq!((p[2], p[6], p[7], p[8]), (0x1F, 2, 0x07, 0x80));
        assert_eq!(p[89], 0x02 ^ 0x07 ^ 0x80, "XOR over report bytes 2..88");
        assert_eq!(p[90], 0);
    }

    #[test]
    fn responses() {
        let mut reply = request(0x1F, CLASS_POWER, CMD_BATTERY_LEVEL);
        reply[1] = 0x02;
        reply[10] = 204;
        assert_eq!(
            parse(&reply, CLASS_POWER, CMD_BATTERY_LEVEL),
            Response::Value(204)
        );
        assert_eq!(
            parse(&reply, CLASS_POWER, CMD_CHARGING_STATUS),
            Response::Invalid
        );
        reply[1] = 0x01;
        assert_eq!(
            parse(&reply, CLASS_POWER, CMD_BATTERY_LEVEL),
            Response::Busy
        );
        reply[1] = 0x05;
        assert_eq!(
            parse(&reply, CLASS_POWER, CMD_BATTERY_LEVEL),
            Response::Invalid
        );
        assert_eq!(
            parse(&reply[..40], CLASS_POWER, CMD_BATTERY_LEVEL),
            Response::Invalid
        );
    }

    #[test]
    fn scale() {
        assert_eq!(percent(255), 100);
        assert_eq!(percent(204), 80);
        assert_eq!(percent(0), 0);
        assert_eq!(percent(1), 0);
        assert_eq!(percent(3), 1);
    }
}
