//! HyperX: Cloud II Wireless (HP and Kingston), Cloud Stinger 2 Wireless, Cloud Flight,
//! Cloud Alpha Wireless, Cloud Alpha 2 Wireless, Cloud III Wireless and the Pulsefire
//! Dart mouse. Headset layouts as documented by the HeadsetControl project.

use super::Driver;
use crate::hid::HidInfo;

/// HyperX moved from Kingston to HP; both vendor ids are in use.
pub const VID_HP: u16 = 0x03F0;
pub const VID_KINGSTON: u16 = 0x0951;

const CLOUD2_PIDS: [u16; 3] = [0x018B, 0x0696, 0x0D93];
const CLOUD_STINGER2: u16 = 0x0D93;
const ALPHA_PIDS: [u16; 4] = [0x0286, 0x098D, 0x1743, 0x1765];
const CLOUD3_PIDS: [u16; 2] = [0x04A8, 0x0974];
/// Cloud Alpha 2 Wireless base station.
const ALPHA2: u16 = 0x08BE;
/// Kingston-era headsets.
const FLIGHT_PIDS: [u16; 2] = [0x16C4, 0x1723];
const CLOUD2_KINGSTON: u16 = 0x1718;
/// Pulsefire Dart: wireless dongle and wired mode.
const DART_DONGLE: u16 = 0x16E1;
const DART_WIRED: u16 = 0x16E2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Cloud2,
    Alpha,
    Alpha2,
}

fn family(group: &[HidInfo]) -> Option<Family> {
    let first = group.first().filter(|i| i.vid == VID_HP)?;
    if CLOUD2_PIDS.contains(&first.pid) {
        Some(Family::Cloud2)
    } else if ALPHA_PIDS.contains(&first.pid) || CLOUD3_PIDS.contains(&first.pid) {
        Some(Family::Alpha)
    } else if first.pid == ALPHA2 {
        Some(Family::Alpha2)
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

fn is_kingston_headset(group: &[HidInfo]) -> bool {
    group.first().is_some_and(|i| {
        i.vid == VID_KINGSTON && (FLIGHT_PIDS.contains(&i.pid) || i.pid == CLOUD2_KINGSTON)
    })
}

pub const KINGSTON_HEADSET: Driver = Driver {
    name: "HyperX Kingston headset",
    vendor_ids: &[VID_KINGSTON],
    silence_is_normal: true,
    matches: is_kingston_headset,
    #[cfg(windows)]
    read: read_kingston_headset,
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

/// Cloud Alpha 2 Wireless: `[command, sub]`, answered by `[command + 1, sub, …]`.
pub const ALPHA2_LINK: [u8; 2] = [0x52, 0x01];
pub const ALPHA2_BATTERY: [u8; 2] = [0x50, 0x02];

pub fn is_alpha2_reply(reply: &[u8], request: [u8; 2]) -> bool {
    reply.len() >= 4 && reply[0] == request[0] + 1 && reply[1] == request[1]
}

/// `(percent, charging)` from the Alpha 2 battery reply.
pub fn parse_alpha2(reply: &[u8]) -> Option<(u8, bool)> {
    (is_alpha2_reply(reply, ALPHA2_BATTERY) && reply[2] <= 100)
        .then(|| (reply[2], reply[3] == 0x01))
}

/// Cloud Flight battery query.
pub const FLIGHT_REQUEST: [u8; 3] = [0x21, 0xFF, 0x05];
/// Above this voltage (mV) the Cloud Flight is on its charger.
const FLIGHT_CHARGING_MV: u16 = 0x100B;

/// Battery voltage (mV, big endian at bytes 3–4) of a Cloud Flight reply, if plausible.
pub fn flight_voltage(reply: &[u8]) -> Option<u16> {
    let mv = u16::from_be_bytes([*reply.get(3)?, *reply.get(4)?]);
    (3000..=4600).contains(&mv).then_some(mv)
}

/// HeadsetControl's discharge curve of the Cloud Flight.
pub fn flight_percent(mv: u16) -> u8 {
    let v = f64::from(mv);
    let percent = if mv <= 3648 {
        0.00125 * v
    } else if mv > 3975 {
        100.0
    } else {
        2.547505e-8 * v.powi(4) - 3.900299e-4 * v.powi(3) + 2.238321 * v.powi(2) - 5706.256 * v
            + 5452299.0
    };
    crate::model::clamp_percent(percent)
}

/// Kingston Cloud II Wireless: fixed prefix, then the command.
pub fn cloud2_kingston_request(command: u8) -> [u8; 17] {
    [
        0x06, 0x00, 0x02, 0x00, 0x9A, 0x00, 0x00, 0x68, 0x4A, 0x8E, 0x0A, 0x00, 0x00, 0x00, 0xBB,
        command, 0x00,
    ]
}

pub fn is_cloud2_kingston_reply(reply: &[u8], command: u8) -> bool {
    reply.len() >= 8 && reply[..4] == [0x0B, 0x00, 0xBB, command]
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
        Family::Alpha2 => {
            let writers = group
                .iter()
                .filter(|i| i.usage_page == 0xFF13 && i.usage == 0xFF00 && i.output_len >= 2);
            for info in writers {
                let Some(mut session) = Session::open(group, info) else {
                    continue;
                };
                let mut ask = |request: [u8; 2]| {
                    session.ask(&request, Via::Output, Duration::from_millis(1000), |r| {
                        is_alpha2_reply(r, request)
                    })
                };
                let Some(link) = ask(ALPHA2_LINK) else {
                    continue;
                };
                if link[2] == 0x00 {
                    return Vec::new(); // headset not connected to its base
                }
                let Some((level, charging)) = ask(ALPHA2_BATTERY).and_then(|r| parse_alpha2(&r))
                else {
                    return Vec::new();
                };
                return vec![new(info, "HyperX Cloud Alpha 2 Wireless")
                    .battery(Some(level))
                    .charging(charging && level < 100)
                    .full(charging && level >= 100)];
            }
        }
    }
    Vec::new()
}

#[cfg(windows)]
fn read_kingston_headset(group: &[HidInfo], _product: &str) -> Vec<crate::model::Reading> {
    use crate::model::{Kind, Link, Reading};
    use crate::win::hid::{Session, Via};
    use std::time::Duration;

    let Some(pid) = group.first().map(|i| i.pid) else {
        return Vec::new();
    };
    let writers = group
        .iter()
        .filter(|i| matches!(i.interface, 0 | -1) && i.output_len >= 3);
    for info in writers {
        let Some(mut session) = Session::open(group, info) else {
            continue;
        };
        let id = format!("hyperx:{:04x}:{}", info.pid, super::instance_of(info));
        if pid == CLOUD2_KINGSTON {
            if info.output_len < 17 {
                continue;
            }
            let mut ask = |command: u8| {
                session.ask(
                    &cloud2_kingston_request(command),
                    Via::Output,
                    Duration::from_millis(1000),
                    |r| is_cloud2_kingston_reply(r, command),
                )
            };
            let Some(level) = ask(2) else { continue };
            let charging = ask(3).is_some_and(|r| r[4] == 1);
            if !(1..=100).contains(&level[7]) {
                return Vec::new();
            }
            return vec![
                Reading::new(id, "HyperX Cloud II Wireless", Kind::Headset, Link::Rf24)
                    .battery(Some(level[7]))
                    .charging(charging),
            ];
        }
        let Some(mv) = session
            .ask(
                &FLIGHT_REQUEST,
                Via::Output,
                Duration::from_millis(2000),
                |r| flight_voltage(r).is_some(),
            )
            .and_then(|r| flight_voltage(&r))
        else {
            continue;
        };
        // On the charger the voltage says nothing about the level.
        let charging = mv > FLIGHT_CHARGING_MV;
        let level = (!charging).then(|| flight_percent(mv));
        return vec![
            Reading::new(id, "HyperX Cloud Flight", Kind::Headset, Link::Rf24)
                .battery(level)
                .charging(charging),
        ];
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
    fn newer_headsets() {
        assert!(is_kingston_headset(&[collection(
            VID_KINGSTON,
            0x16C4,
            0xFF00,
            1,
            0,
            (32, 20, 0)
        )]));
        assert!(!is_headset(&[collection(
            VID_HP,
            0x1718,
            0xFF00,
            1,
            0,
            (64, 64, 0)
        )]));
        assert!(is_headset(&[collection(
            VID_HP,
            0x08BE,
            0xFF13,
            0xFF00,
            2,
            (64, 64, 0)
        )]));
        assert_eq!(parse_alpha2(&[0x51, 0x02, 77, 0x01]), Some((77, true)));
        assert_eq!(parse_alpha2(&[0x51, 0x02, 140, 0x00]), None);
        assert_eq!(parse_alpha2(&[0x53, 0x01, 1, 0]), None, "link reply");
        assert!(is_alpha2_reply(&[0x53, 0x01, 1, 0], ALPHA2_LINK));
        assert_eq!(cloud2_kingston_request(2)[14..16], [0xBB, 0x02]);
        assert!(is_cloud2_kingston_reply(
            &[0x0B, 0, 0xBB, 2, 0, 0, 0, 80],
            2
        ));
    }

    #[test]
    fn cloud_flight_curve() {
        assert_eq!(flight_voltage(&[0, 0, 0, 0x0F, 0x3C]), Some(3900));
        assert_eq!(flight_voltage(&[0, 0, 0, 0x00, 0x10]), None);
        assert_eq!(flight_percent(4000), 100);
        assert_eq!(flight_percent(3000), 4);
        let mid = flight_percent(3800);
        assert!((20..=90).contains(&mid), "{mid}");
        assert!(flight_percent(3700) <= flight_percent(3900));
    }

    #[test]
    fn pulsefire() {
        assert_eq!(parse_pulsefire(&[0x51, 0, 0, 3, 64, 0]), Some((64, false)));
        assert_eq!(parse_pulsefire(&[0x51, 0, 0, 3, 100, 1]), Some((100, true)));
        assert_eq!(parse_pulsefire(&[0x51, 0, 0, 3, 0, 0]), None);
        assert_eq!(parse_pulsefire(&[0x51, 0, 0, 4, 64, 0]), None);
    }
}
