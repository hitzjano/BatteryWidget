//! HyperX: Cloud II Wireless, Cloud Stinger 2 Wireless, Cloud Alpha Wireless,
//! Cloud III Wireless and the Pulsefire Dart mouse.

use super::Driver;
use crate::hid::HidInfo;

/// HyperX moved from Kingston to HP; both vendor ids are in use.
pub const VID_HP: u16 = 0x03F0;
pub const VID_KINGSTON: u16 = 0x0951;

const CLOUD2_PIDS: [u16; 4] = [0x018B, 0x0696, 0x0D93, 0x1718];
const CLOUD_STINGER2: u16 = 0x0D93;
const ALPHA_PIDS: [u16; 4] = [0x0286, 0x098D, 0x1743, 0x1765];
const CLOUD3_PIDS: [u16; 2] = [0x04A8, 0x0974];
/// Pulsefire Dart: wireless dongle and wired mode.
const DART_DONGLE: u16 = 0x16E1;
const DART_WIRED: u16 = 0x16E2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Cloud2,
    Alpha,
}

fn family(group: &[HidInfo]) -> Option<Family> {
    let first = group.first().filter(|i| i.vid == VID_HP)?;
    if CLOUD2_PIDS.contains(&first.pid) {
        Some(Family::Cloud2)
    } else if ALPHA_PIDS.contains(&first.pid) || CLOUD3_PIDS.contains(&first.pid) {
        Some(Family::Alpha)
    } else {
        None
    }
}

fn is_headset(group: &[HidInfo]) -> bool {
    family(group).is_some()
}

fn is_pulsefire(group: &[HidInfo]) -> bool {
    group
        .first()
        .is_some_and(|i| i.vid == VID_KINGSTON && matches!(i.pid, DART_DONGLE | DART_WIRED))
}

pub const DRIVER: Driver = Driver {
    name: "HyperX headset",
    vendor_ids: &[VID_HP],
    silence_is_normal: true,
    matches: is_headset,
    #[cfg(windows)]
    read: read_headset,
};

pub const PULSEFIRE: Driver = Driver {
    name: "HyperX Pulsefire",
    vendor_ids: &[VID_KINGSTON],
    silence_is_normal: true,
    matches: is_pulsefire,
    #[cfg(windows)]
    read: read_pulsefire,
};

/// Cloud II family: `06 FF BB <command>`; command 2 = level, 3 = charging.
pub fn cloud2_request(command: u8) -> [u8; 5] {
    [0x06, 0xFF, 0xBB, command, 0x00]
}

pub fn is_cloud2_reply(reply: &[u8], command: u8) -> bool {
    reply.len() >= 8 && reply[..4] == [0x06, 0xFF, 0xBB, command]
}

/// Cloud Alpha family: `21 BB <command>`; 3 = link state, 0x0B = level, 0x0C = charging.
pub fn alpha_request(command: u8) -> [u8; 3] {
    [0x21, 0xBB, command]
}

pub fn is_alpha_reply(reply: &[u8]) -> bool {
    reply.len() >= 4 && reply[0] == 0x21
}

/// Pulsefire Dart heartbeat: `51`, answered with `51 00 00 03 <level> <charging>`.
pub const PULSEFIRE_REQUEST: [u8; 2] = [0x00, 0x51];

/// `(percent, charging)`, `None` if the level is implausible (mouse asleep).
pub fn parse_pulsefire(reply: &[u8]) -> Option<(u8, bool)> {
    if reply.len() < 6 || reply[0] != 0x51 || reply[3] != 0x03 {
        return None;
    }
    (1..=100)
        .contains(&reply[4])
        .then(|| (reply[4], reply[5] == 1))
}

#[cfg(windows)]
fn read_headset(group: &[HidInfo], _product: &str) -> Vec<crate::model::Reading> {
    use crate::model::{Kind, Link, Reading};
    use crate::win::hid::{Session, Via};
    use std::time::Duration;

    let Some(family) = family(group) else {
        return Vec::new();
    };
    let pid = group[0].pid;
    let new = |info: &HidInfo, name: &str| {
        let id = format!("hyperx:{:04x}:{}", info.pid, super::instance_of(info));
        Reading::new(id, name, Kind::Headset, Link::Rf24)
    };
    match family {
        Family::Cloud2 => {
            let name = if pid == CLOUD_STINGER2 {
                "HyperX Cloud Stinger 2 Wireless"
            } else {
                "HyperX Cloud II Wireless"
            };
            let writers = group
                .iter()
                .filter(|i| i.usage_page == 0xFF90 && i.usage == 0x0303 && i.output_len > 0);
            for info in writers {
                let Some(mut session) = Session::open(group, info) else {
                    continue;
                };
                let mut ask = |command: u8| {
                    session.ask(
                        &cloud2_request(command),
                        Via::Output,
                        Duration::from_millis(900),
                        |r| is_cloud2_reply(r, command),
                    )
                };
                let Some(level) = ask(2) else { continue };
                let charging = ask(3).is_some_and(|r| r[4] == 1);
                if !(1..=100).contains(&level[7]) {
                    return Vec::new();
                }
                return vec![new(info, name).battery(Some(level[7])).charging(charging)];
            }
        }
        Family::Alpha => {
            let name = if CLOUD3_PIDS.contains(&pid) {
                "HyperX Cloud III Wireless"
            } else {
                "HyperX Cloud Alpha Wireless"
            };
            let writers = group.iter().filter(|i| {
                matches!(i.interface, 0 | -1) && i.output_len >= 4 && i.usage_page >= 0xFF00
            });
            for info in writers {
                let Some(mut session) = Session::open(group, info) else {
                    continue;
                };
                let mut ask = |command: u8| {
                    session.ask(
                        &alpha_request(command),
                        Via::Output,
                        Duration::from_millis(1500),
                        is_alpha_reply,
                    )
                };
                let Some(link) = ask(0x03) else { continue };
                if link[3] == 1 {
                    // The dongle is plugged in but the headset is switched off.
                    return Vec::new();
                }
                let charging = ask(0x0C).is_some_and(|r| r[3] == 1);
                // While charging the reported level is meaningless.
                let level = if charging {
                    None
                } else {
                    ask(0x0B).map(|r| r[3]).filter(|l| *l <= 100)
                };
                return vec![new(info, name).battery(level).charging(charging)];
            }
        }
    }
    Vec::new()
}

#[cfg(windows)]
fn read_pulsefire(group: &[HidInfo], _product: &str) -> Vec<crate::model::Reading> {
    use crate::model::{Kind, Link, Reading};
    use crate::win::hid::{Session, Via};
    use std::time::Duration;

    let writers = group.iter().filter(|i| {
        matches!((i.usage_page, i.interface), (0xFF00, 2) | (0xFF13, 1)) && i.output_len >= 2
    });
    for info in writers {
        let Some(mut session) = Session::open(group, info) else {
            continue;
        };
        let is_reply = |r: &[u8]| r.len() >= 6 && r[0] == 0x51 && r[3] == 0x03;
        let Some(reply) = session.ask(
            &PULSEFIRE_REQUEST,
            Via::Output,
            Duration::from_millis(900),
            is_reply,
        ) else {
            continue;
        };
        let Some((percent, charging)) = parse_pulsefire(&reply) else {
            return Vec::new();
        };
        let link = if info.pid == DART_WIRED {
            Link::Usb
        } else {
            Link::Rf24
        };
        let id = format!("hyperx:{:04x}:{}", info.pid, super::instance_of(info));
        return vec![Reading::new(id, "HyperX Pulsefire Dart", Kind::Mouse, link)
            .battery(Some(percent))
            .charging(charging && percent < 100)
            .full(charging && percent >= 100)];
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vendor::collection;

    #[test]
    fn matching() {
        assert!(is_headset(&[collection(
            VID_HP,
            0x018B,
            0xFF90,
            0x0303,
            0,
            (64, 64, 0)
        )]));
        assert!(is_headset(&[collection(
            VID_HP,
            0x098D,
            0xFF00,
            1,
            0,
            (64, 64, 0)
        )]));
        assert!(
            !is_headset(&[collection(VID_HP, 0x1234, 0xFF00, 1, 0, (64, 64, 0))]),
            "an HP printer is not a headset"
        );
        assert!(is_pulsefire(&[collection(
            VID_KINGSTON,
            0x16E1,
            0xFF00,
            1,
            2,
            (64, 64, 0)
        )]));
        assert!(
            !is_pulsefire(&[collection(VID_KINGSTON, 0x1666, 0xFF00, 1, 2, (64, 64, 0))]),
            "a USB stick is not a mouse"
        );
    }

    #[test]
    fn packets() {
        assert_eq!(cloud2_request(2), [0x06, 0xFF, 0xBB, 0x02, 0x00]);
        assert!(is_cloud2_reply(&[0x06, 0xFF, 0xBB, 0x02, 0, 0, 0, 80], 2));
        assert!(!is_cloud2_reply(&[0x06, 0xFF, 0xBB, 0x03, 0, 0, 0, 80], 2));
        assert_eq!(alpha_request(0x0B), [0x21, 0xBB, 0x0B]);
        assert!(is_alpha_reply(&[0x21, 0xBB, 0x0B, 64]));
    }

    #[test]
    fn pulsefire() {
        assert_eq!(parse_pulsefire(&[0x51, 0, 0, 3, 64, 0]), Some((64, false)));
        assert_eq!(parse_pulsefire(&[0x51, 0, 0, 3, 100, 1]), Some((100, true)));
        assert_eq!(parse_pulsefire(&[0x51, 0, 0, 3, 0, 0]), None);
        assert_eq!(parse_pulsefire(&[0x51, 0, 0, 4, 64, 0]), None);
    }
}
