//! Corsair: Void / HS / Virtuoso headsets and Slipstream ("Bragi") mice and keyboards.

use super::Driver;
use crate::hid::HidInfo;

pub const VID_CORSAIR: u16 = 0x1B1C;

const HEADSET_PIDS: [u16; 20] = [
    0x0A14, 0x0A16, 0x0A17, 0x0A1A, 0x0A1D, 0x0A2B, 0x0A38, 0x0A4F, 0x0A51, 0x0A52, 0x0A55, 0x0A56,
    0x0A63, 0x0A64, 0x0A65, 0x0A75, 0x1B23, 0x1B27, 0x1B29, 0x1B2A,
];
const HEADSET_USAGE_PAGE: u16 = 0xFFC5;
const BRAGI_USAGE_PAGE: u16 = 0xFF42;

const BRAGI_NAMES: [(u16, &str); 15] = [
    (0x1B4C, "Corsair Ironclaw RGB Wireless"),
    (0x1B66, "Corsair Ironclaw RGB Wireless"),
    (0x1B5E, "Corsair Harpoon RGB Wireless"),
    (0x1B65, "Corsair Harpoon RGB Wireless"),
    (0x1B80, "Corsair Dark Core RGB PRO"),
    (0x1B81, "Corsair Dark Core RGB PRO"),
    (0x1B7E, "Corsair Dark Core RGB PRO SE"),
    (0x1B7F, "Corsair Dark Core RGB PRO SE"),
    (0x1B6E, "Corsair K57 RGB Wireless"),
    (0x1B62, "Corsair K57 RGB Wireless"),
    (0x1B86, "Corsair Katar Elite Wireless"),
    (0x1BA8, "Corsair M65 RGB ULTRA Wireless"),
    (0x1BAD, "Corsair Nightsabre Wireless"),
    (0x1BAE, "Corsair Scimitar Elite Wireless"),
    (0x1B88, "Corsair K70 PRO Mini Wireless"),
];

fn is_headset(group: &[HidInfo]) -> bool {
    group
        .first()
        .is_some_and(|i| i.vid == VID_CORSAIR && HEADSET_PIDS.contains(&i.pid))
}

fn is_bragi(group: &[HidInfo]) -> bool {
    !is_headset(group)
        && group
            .iter()
            .any(|i| i.vid == VID_CORSAIR && i.usage_page == BRAGI_USAGE_PAGE)
}

pub const HEADSET: Driver = Driver {
    name: "Corsair headset",
    vendor_ids: &[VID_CORSAIR],
    silence_is_normal: false,
    matches: is_headset,
    #[cfg(windows)]
    read: read_headset,
};

pub const BRAGI: Driver = Driver {
    name: "Corsair Bragi",
    vendor_ids: &[VID_CORSAIR],
    silence_is_normal: true,
    matches: is_bragi,
    #[cfg(windows)]
    read: read_bragi,
};

/// Headset battery request: report id `0xC9`, command `0x64`.
pub const HEADSET_REQUEST: [u8; 2] = [0xC9, 0x64];

/// `(percent, charging)` from the headset reply, `None` while the headset is off.
/// Byte 2 carries the level (bit 7 is the microphone-boom flag), byte 4 the state.
pub fn parse_headset(reply: &[u8]) -> Option<(u8, bool)> {
    if reply.len() < 5 {
        return None;
    }
    match reply[4] {
        1 | 2 => Some((reply[2] & 0x7F, false)),
        4 | 5 => Some((reply[2] & 0x7F, true)),
        _ => None,
    }
}

/// Bragi property ids.
pub const PROP_BATTERY_LEVEL: u8 = 0x0F;
pub const PROP_BATTERY_STATUS: u8 = 0x10;
pub const PROP_PRODUCT_ID: u8 = 0x12;
pub const PROP_SUBDEVICES: u8 = 0x36;

/// "Get property" for the receiver itself (`device` 0) or one of its paired devices (1–7).
pub fn bragi_get(device: u8, property: u8) -> [u8; 4] {
    [0x00, 0x08 | device, 0x02, property]
}

pub fn is_bragi_reply(reply: &[u8], device: u8) -> bool {
    reply.len() >= 6 && reply[1] == 0x02 && reply[0] & 0x07 == device
}

/// Little-endian 24-bit property value, `None` if the device reported an error.
pub fn bragi_value(reply: &[u8]) -> Option<u32> {
    (reply.len() >= 6 && reply[2] == 0)
        .then(|| u32::from(reply[3]) | u32::from(reply[4]) << 8 | u32::from(reply[5]) << 16)
}

/// Indexes of the devices paired with a receiver, from the sub-device bit field.
pub fn bragi_subdevices(bitfield: Option<u32>) -> Vec<u8> {
    let paired: Vec<u8> = (1..8)
        .filter(|i| bitfield.is_some_and(|b| b >> i & 1 == 1))
        .collect();
    if paired.is_empty() {
        vec![0]
    } else {
        paired
    }
}

#[cfg(windows)]
const TIMEOUT: std::time::Duration = std::time::Duration::from_millis(900);

#[cfg(windows)]
fn read_headset(group: &[HidInfo], product: &str) -> Vec<crate::model::Reading> {
    use crate::model::{Kind, Link, Reading};
    use crate::win::hid::{Session, Via};

    let writers = group
        .iter()
        .filter(|i| i.usage_page == HEADSET_USAGE_PAGE && i.interface == 3 && i.output_len > 0);
    for info in writers {
        let Some(mut session) = Session::open(group, info) else {
            continue;
        };
        let Some(reply) = session.ask(&HEADSET_REQUEST, Via::Output, TIMEOUT, |r| r.len() >= 5)
        else {
            continue;
        };
        let Some((percent, charging)) = parse_headset(&reply) else {
            return Vec::new();
        };
        let name = crate::names::clean_product_name(product, "Corsair Void");
        let id = format!("corsair:{:04x}:{}", info.pid, super::instance_of(info));
        return vec![Reading::new(id, name, Kind::Headset, Link::Rf24)
            .battery(Some(percent))
            .charging(charging)];
    }
    Vec::new()
}

#[cfg(windows)]
fn read_bragi(group: &[HidInfo], product: &str) -> Vec<crate::model::Reading> {
    use crate::model::{clamp_percent, Kind, Link, Reading};
    use crate::names::{clean_product_name, kind_from_name};
    use crate::win::hid::{Session, Via};

    let writers = group
        .iter()
        .filter(|i| i.usage_page == BRAGI_USAGE_PAGE && matches!(i.output_len, 65 | 1025));
    for info in writers {
        let Some(mut session) = Session::open(group, info) else {
            continue;
        };
        let mut get = |device: u8, property: u8| {
            session
                .ask(&bragi_get(device, property), Via::Output, TIMEOUT, |r| {
                    is_bragi_reply(r, device)
                })
                .and_then(|reply| bragi_value(&reply))
        };
        let bitfield = get(0, PROP_SUBDEVICES);
        let mut results = Vec::new();
        for device in bragi_subdevices(bitfield) {
            let (Some(level), Some(status)) = (
                get(device, PROP_BATTERY_LEVEL),
                get(device, PROP_BATTERY_STATUS),
            ) else {
                continue;
            };
            if !(1..=3).contains(&status) {
                continue;
            }
            let pid = if device == 0 {
                None
            } else {
                get(device, PROP_PRODUCT_ID)
            }
            .and_then(|p| u16::try_from(p).ok())
            .unwrap_or(info.pid);
            let name = BRAGI_NAMES
                .iter()
                .find(|(p, _)| *p == pid)
                .map(|(_, n)| n.to_string())
                .unwrap_or_else(|| clean_product_name(product, "Corsair"));
            let kind = match kind_from_name(&name) {
                Kind::Generic => Kind::Mouse,
                kind => kind,
            };
            // The level is reported in tenths of a percent.
            let percent = clamp_percent(f64::from(level) / 10.0);
            let link = if device == 0 { Link::Usb } else { Link::Rf24 };
            let id = format!(
                "corsair:{:04x}:{}:{device}",
                info.pid,
                super::instance_of(info)
            );
            results.push(
                Reading::new(id, name, kind, link)
                    .battery(Some(percent))
                    .charging(status == 1)
                    .full(status == 3),
            );
        }
        if !results.is_empty() || bitfield.is_some() {
            return results;
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headset_reply() {
        assert_eq!(
            parse_headset(&[0x64, 0, 0x80 | 77, 0, 1]),
            Some((77, false))
        );
        assert_eq!(parse_headset(&[0x64, 0, 55, 0, 5]), Some((55, true)));
        assert_eq!(
            parse_headset(&[0x64, 0, 0, 0, 0]),
            None,
            "headset switched off"
        );
        assert_eq!(parse_headset(&[0x64, 0, 55]), None);
    }

    #[test]
    fn bragi_packets() {
        assert_eq!(bragi_get(0, PROP_SUBDEVICES), [0x00, 0x08, 0x02, 0x36]);
        assert_eq!(bragi_get(1, PROP_BATTERY_LEVEL), [0x00, 0x09, 0x02, 0x0F]);
        let reply = [0x01, 0x02, 0x00, 0xE8, 0x03, 0x00];
        assert!(is_bragi_reply(&reply, 1) && !is_bragi_reply(&reply, 2));
        assert_eq!(bragi_value(&reply), Some(1000));
        assert_eq!(
            bragi_value(&[0x01, 0x02, 0x05, 0, 0, 0]),
            None,
            "error status"
        );
    }

    #[test]
    fn subdevices() {
        assert_eq!(bragi_subdevices(Some(0b0000_0110)), [1, 2]);
        assert_eq!(bragi_subdevices(Some(0)), [0]);
        assert_eq!(
            bragi_subdevices(None),
            [0],
            "wired device answers for itself"
        );
    }
}
