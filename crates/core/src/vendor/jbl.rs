//! JBL Quantum wireless headsets on their USB dongle. Two protocol generations exist:
//!
//! - Quantum 810 / 910P: one small feature report per value; the level is feature
//!   report `0x49` (`[0x49, percent]`), as documented by jbl-quantum-engine-linux and
//!   HeadsetControlLinux.
//! - Quantum 950: 64-byte `88 00 DD …` feature requests answered by `86 00 DD …` input
//!   reports, as implemented by MyJBxHeadsetControl.
//!
//! The dongles do not report whether the headset is charging.

use super::Driver;
use crate::hid::HidInfo;

/// Harman International (JBL).
pub const VID_HARMAN: u16 = 0x0ECB;
/// The vendor-defined control channel lives on this USB interface.
const CONTROL_INTERFACE: i32 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// Feature report `0x49` holds the level.
    Simple,
    /// `88 00 DD` command channel.
    Command,
}

pub struct Model {
    pub name: &'static str,
    pub pid: u16,
    pub family: Family,
}

pub const MODELS: [Model; 3] = [
    Model {
        name: "JBL Quantum 810 Wireless",
        pid: 0x2069,
        family: Family::Simple,
    },
    Model {
        name: "JBL Quantum 910P Wireless",
        pid: 0x20A2,
        family: Family::Simple,
    },
    Model {
        name: "JBL Quantum 950 Wireless",
        pid: 0x2127,
        family: Family::Command,
    },
];

pub fn model_for(pid: u16) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.pid == pid)
}

fn matches(group: &[HidInfo]) -> bool {
    group
        .first()
        .is_some_and(|i| i.vid == VID_HARMAN && !i.bluetooth && model_for(i.pid).is_some())
}

pub const DRIVER: Driver = Driver {
    name: "JBL Quantum",
    vendor_ids: &[VID_HARMAN],
    silence_is_normal: true,
    matches,
    #[cfg(windows)]
    read,
};

/// Feature report of the level on the 810 / 910P.
pub const BATTERY_FEATURE: u8 = 0x49;

/// Level from feature report `0x49`. The dongle also answers unknown report ids, so the
/// echoed id is checked. `None` when the headset is off (level 0) or the value is invalid.
pub fn parse_simple(report: &[u8]) -> Option<u8> {
    match report {
        [BATTERY_FEATURE, level, ..] if (1..=100).contains(level) => Some(*level),
        _ => None,
    }
}

/// `GET` of one value on the 950 command channel.
pub fn command_request(code: [u8; 2]) -> [u8; 11] {
    [
        0x88, 0x00, 0xDD, 0x01, 0x00, 0x01, 0x00, 0x02, 0x00, code[0], code[1],
    ]
}

pub const CODE_BATTERY: [u8; 2] = [0x0D, 0x00];
/// Link between dongle and headset; the value `0x80` means connected. The dongle answers
/// other questions from its cache even while the headset is off.
pub const CODE_LINK: [u8; 2] = [0x35, 0x00];
const LINK_CONNECTED: u8 = 0x80;

/// The value of a `86 00 DD` reply (answer or event) for `code`.
pub fn command_value(reply: &[u8], code: [u8; 2]) -> Option<u8> {
    if reply.len() < 16 || reply[..3] != [0x86, 0x00, 0xDD] || reply[11..13] != code {
        return None;
    }
    Some(reply[15])
}

pub fn parse_command_level(value: u8) -> Option<u8> {
    (1..=100).contains(&value).then_some(value)
}

#[cfg(windows)]
fn read(group: &[HidInfo], _product: &str) -> Vec<crate::model::Reading> {
    use crate::model::{Kind, Link, Reading};
    use crate::win::hid::{HidDevice, Session, Via};
    use std::time::Duration;

    let Some(model) = group.first().and_then(|i| model_for(i.pid)) else {
        return Vec::new();
    };
    let on_control = |i: &&HidInfo| i.interface == CONTROL_INTERFACE && i.usage_page >= 0xFF00;
    let new = |info: &HidInfo, level: u8| {
        let id = format!("jbl:{:04x}:{}", info.pid, super::instance_of(info));
        vec![Reading::new(id, model.name, Kind::Headset, Link::Rf24).battery(Some(level))]
    };
    match model.family {
        Family::Simple => {
            for info in group
                .iter()
                .filter(on_control)
                .filter(|i| i.feature_len >= 2)
            {
                let Ok(device) = HidDevice::open_for_features(info) else {
                    continue;
                };
                let Some(report) = device.get_feature(BATTERY_FEATURE) else {
                    continue;
                };
                return parse_simple(&report).map_or_else(Vec::new, |level| new(info, level));
            }
        }
        Family::Command => {
            let writers = group
                .iter()
                .filter(on_control)
                .filter(|i| i.usage_page == 0xFF12 && i.feature_len >= 64);
            for info in writers {
                let Some(mut session) = Session::open(group, info) else {
                    continue;
                };
                let mut ask = |code: [u8; 2]| {
                    session
                        .ask(
                            &command_request(code),
                            Via::Feature,
                            Duration::from_millis(900),
                            |r| command_value(r, code).is_some(),
                        )
                        .and_then(|r| command_value(&r, code))
                };
                let Some(link) = ask(CODE_LINK) else { continue };
                if link != LINK_CONNECTED {
                    // Dongle plugged in, headset switched off.
                    return Vec::new();
                }
                return ask(CODE_BATTERY)
                    .and_then(parse_command_level)
                    .map_or_else(Vec::new, |level| new(info, level));
            }
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vendor::collection;

    #[test]
    fn matching() {
        let q910 = [collection(VID_HARMAN, 0x20A2, 0xFF13, 1, 5, (64, 0, 2))];
        assert!(matches(&q910));
        let unknown = [collection(VID_HARMAN, 0x1234, 0xFF13, 1, 5, (64, 0, 2))];
        assert!(!matches(&unknown), "other Harman products are left alone");
    }

    #[test]
    fn simple_level() {
        assert_eq!(parse_simple(&[0x49, 95]), Some(95));
        assert_eq!(parse_simple(&[0x49, 50, 0xAA, 0xBB]), Some(50), "padding");
        assert_eq!(parse_simple(&[0x49, 0]), None, "headset off");
        assert_eq!(parse_simple(&[0x49, 101]), None);
        assert_eq!(parse_simple(&[0x48, 95]), None, "wrong report echoed");
        assert_eq!(parse_simple(&[0x49]), None);
    }

    #[test]
    fn command_channel() {
        assert_eq!(
            command_request(CODE_BATTERY),
            [0x88, 0x00, 0xDD, 0x01, 0x00, 0x01, 0x00, 0x02, 0x00, 0x0D, 0x00]
        );
        let mut reply = [0u8; 64];
        reply[..4].copy_from_slice(&[0x86, 0x00, 0xDD, 0x01]);
        reply[11..13].copy_from_slice(&CODE_BATTERY);
        reply[15] = 70;
        assert_eq!(command_value(&reply, CODE_BATTERY), Some(70));
        assert_eq!(command_value(&reply, CODE_LINK), None);
        assert_eq!(parse_command_level(70), Some(70));
        assert_eq!(parse_command_level(0), None);
        assert_eq!(parse_command_level(200), None);
        reply[0] = 0x87;
        assert_eq!(command_value(&reply, CODE_BATTERY), None);
    }
}
