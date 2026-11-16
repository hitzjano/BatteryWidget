//! DualSense, DualSense Edge and DualShock 4 over USB, Bluetooth or the Sony USB adapter.
//!
//! Purely passive: the battery state is part of the regular input report.

use super::Driver;
use crate::hid::HidInfo;

pub const VID_SONY: u16 = 0x054C;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    DualSense,
    DualShock4,
}

struct Model {
    pid: u16,
    name: &'static str,
    family: Family,
    /// Name under which Windows lists the controller when paired over Bluetooth.
    bluetooth_name: &'static str,
}

/// Product id of the DualShock 4 USB wireless adapter.
const DS4_ADAPTER: u16 = 0x0BA0;

const MODELS: [Model; 5] = [
    Model {
        pid: 0x0CE6,
        name: "DualSense",
        family: Family::DualSense,
        bluetooth_name: "dualsense wireless controller",
    },
    Model {
        pid: 0x0DF2,
        name: "DualSense Edge",
        family: Family::DualSense,
        bluetooth_name: "dualsense edge wireless controller",
    },
    Model {
        pid: 0x05C4,
        name: "DualShock 4",
        family: Family::DualShock4,
        bluetooth_name: "wireless controller",
    },
    Model {
        pid: 0x09CC,
        name: "DualShock 4",
        family: Family::DualShock4,
        bluetooth_name: "wireless controller",
    },
    Model {
        pid: DS4_ADAPTER,
        name: "DualShock 4",
        family: Family::DualShock4,
        bluetooth_name: "wireless controller",
    },
];

fn model(pid: u16) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.pid == pid)
}

fn matches(group: &[HidInfo]) -> bool {
    group
        .first()
        .is_some_and(|i| i.vid == VID_SONY && model(i.pid).is_some())
}

pub const DRIVER: Driver = Driver {
    name: "PlayStation",
    vendor_ids: &[VID_SONY],
    silence_is_normal: false,
    matches,
    #[cfg(windows)]
    read,
};

/// `(percent, charging, full)`
pub type State = (u8, bool, bool);

/// DualSense: report `0x01` (USB, byte 53) or `0x31` (Bluetooth, byte 54).
/// Low nibble is the level in 10 % steps, high nibble the power state.
pub fn parse_dualsense(report: &[u8]) -> Option<State> {
    let status = match report.first()? {
        0x01 if report.len() >= 64 => report[53],
        0x31 if report.len() >= 78 => report[54],
        _ => return None,
    };
    let percent = ((status & 0x0F) * 10 + 5).min(100);
    match status >> 4 {
        0 => Some((percent, false, false)),
        1 => Some((percent, true, false)),
        2 => Some((100, false, true)),
        _ => None,
    }
}

/// DualShock 4: report `0x01` (USB, byte 30) or `0x11` (Bluetooth, byte 32).
/// Bit 4 means "cable attached"; with a cable, level 10 is "almost full" and 11 "full".
pub fn parse_ds4(report: &[u8]) -> Option<State> {
    let status = match report.first()? {
        0x01 if report.len() >= 64 => report[30],
        0x11 if report.len() >= 78 => report[32],
        _ => return None,
    };
    let level = status & 0x0F;
    let cable = status & 0x10 != 0;
    match (cable, level) {
        (_, 0..=9) => Some((level * 10 + 5, cable, false)),
        (true, 10) => Some((100, true, false)),
        (true, 11) => Some((100, false, true)),
        (false, _) => Some((100, false, false)),
        (true, _) => None,
    }
}

#[cfg(windows)]
fn read(group: &[HidInfo], _product: &str) -> Vec<crate::model::Reading> {
    use crate::model::{Kind, Link, Reading};
    use crate::win::hid::HidDevice;
    use std::time::Duration;

    let Some(model) = group.first().and_then(|i| model(i.pid)) else {
        return Vec::new();
    };
    let parse = if model.family == Family::DualSense {
        parse_dualsense
    } else {
        parse_ds4
    };
    let gamepads = group
        .iter()
        .filter(|i| i.usage_page == 1 && i.usage == 5 && i.input_len > 0);
    for info in gamepads {
        let Ok(mut device) = HidDevice::open(info) else {
            continue;
        };
        let mut enabled = false;
        for _ in 0..8 {
            let Ok(Some(report)) = device.read(Duration::from_millis(300)) else {
                break;
            };
            if let Some((percent, charging, full)) = parse(&report) {
                let link = if info.bluetooth {
                    Link::Bluetooth
                } else if info.pid == DS4_ADAPTER {
                    Link::Rf24
                } else {
                    Link::Usb
                };
                let id = format!("ps:{:04x}:{}", info.pid, super::instance_of(info));
                return vec![Reading::new(id, model.name, Kind::Gamepad, link)
                    .battery(Some(percent))
                    .charging(charging)
                    .full(full)
                    .aliases(&[model.bluetooth_name])];
            }
            if info.bluetooth && !enabled {
                // Over Bluetooth the controller sends short reports until a feature
                // report has been requested once.
                let _ = device.get_feature(if model.family == Family::DualSense {
                    0x05
                } else {
                    0x02
                });
                enabled = true;
            }
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(id: u8, len: usize, at: usize, value: u8) -> Vec<u8> {
        let mut r = vec![0u8; len];
        r[0] = id;
        r[at] = value;
        r
    }

    #[test]
    fn dualsense() {
        assert_eq!(
            parse_dualsense(&report(0x01, 64, 53, 0x07)),
            Some((75, false, false))
        );
        assert_eq!(
            parse_dualsense(&report(0x01, 64, 53, 0x1A)),
            Some((100, true, false))
        );
        assert_eq!(
            parse_dualsense(&report(0x31, 78, 54, 0x20)),
            Some((100, false, true))
        );
        assert_eq!(
            parse_dualsense(&report(0x31, 78, 54, 0x03)),
            Some((35, false, false))
        );
        assert_eq!(
            parse_dualsense(&report(0x01, 10, 5, 0)),
            None,
            "short Bluetooth report"
        );
        assert_eq!(
            parse_dualsense(&report(0x01, 64, 53, 0xF0)),
            None,
            "error state"
        );
        assert_eq!(parse_dualsense(&[]), None);
    }

    #[test]
    fn dualshock4() {
        assert_eq!(
            parse_ds4(&report(0x01, 64, 30, 0x05)),
            Some((55, false, false))
        );
        assert_eq!(
            parse_ds4(&report(0x01, 64, 30, 0x13)),
            Some((35, true, false))
        );
        assert_eq!(
            parse_ds4(&report(0x01, 64, 30, 0x1A)),
            Some((100, true, false))
        );
        assert_eq!(
            parse_ds4(&report(0x11, 78, 32, 0x1B)),
            Some((100, false, true))
        );
        assert_eq!(
            parse_ds4(&report(0x11, 78, 32, 0x0A)),
            Some((100, false, false))
        );
        assert_eq!(parse_ds4(&report(0x11, 20, 5, 0)), None);
    }
}
