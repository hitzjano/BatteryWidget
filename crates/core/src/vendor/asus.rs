//! ASUS ROG and TUF wireless mice, keyboards and headsets, directly (dongle or cable) or
//! through the ROG Omni receiver. Request and reply layouts as implemented by the
//! G-Helper project.
//!
//! ASUS uses its vendor id for laptops and many other things, so only the product ids
//! listed here are ever opened.

use super::Driver;
use crate::hid::HidInfo;

pub const VID_ASUS: u16 = 0x0B05;
/// ROG Omni receiver: one dongle, several paired mice and keyboards.
pub const OMNI_RECEIVER: u16 = 0x1ACE;
/// Harpe II Ace dongle; speaks the Omni channel layout but has a single mouse.
const HARPE_II_DONGLE: u16 = 0x1AD0;

/// Report id of the command channel on directly attached devices (none).
const DIRECT_REPORT: u8 = 0x00;
/// Report ids of the Omni receiver's channels: pairing list, keyboard, mouse.
const PAIRING_REPORT: u8 = 0x01;
const OMNI_KEYBOARD_REPORT: u8 = 0x02;
const OMNI_MOUSE_REPORT: u8 = 0x03;
/// Report id of the headset command channel.
const HEADSET_REPORT: u8 = 0xCC;

/// How a device reports its battery. Byte positions count from the first byte after the
/// report id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    /// `12 07`: level at `level_at`, in percent or quarters (0–4); charging at byte 9.
    Mouse { level_at: usize, quarters: bool },
    /// `12 01`: level in percent at `level_at`; charging when byte 8 is 1.
    Keyboard { level_at: usize },
    /// Report `CC`: `12 07` level at byte 5, `12 08` charging when byte 4 is 1.
    Headset,
    /// Like [`Protocol::Headset`], with the left and right bud at bytes 5 and 6.
    Earbuds,
}

const PERCENT: Protocol = Protocol::Mouse {
    level_at: 4,
    quarters: false,
};
const QUARTERS: Protocol = Protocol::Mouse {
    level_at: 4,
    quarters: true,
};
const KEYBOARD: Protocol = Protocol::Keyboard { level_at: 5 };
/// Falchion: a 0–10 gauge at byte 5, the real percentage at byte 10.
const FALCHION: Protocol = Protocol::Keyboard { level_at: 10 };

pub struct Model {
    pub name: &'static str,
    pub pids: &'static [u16],
    /// Product ids of the cable connection (the device charges).
    pub wired: &'static [u16],
    /// USB interface of the command channel; `None`: any vendor-defined collection.
    pub interface: Option<i32>,
    pub protocol: Protocol,
}

const fn dev(
    name: &'static str,
    pids: &'static [u16],
    wired: &'static [u16],
    interface: Option<i32>,
    protocol: Protocol,
) -> Model {
    Model {
        name,
        pids,
        wired,
        interface,
        protocol,
    }
}

/// Devices attached directly: their dongle or their cable.
pub const DIRECT: [Model; 36] = [
    // Mice on interface 0.
    dev(
        "ROG Gladius III Wireless",
        &[0x197F, 0x197D],
        &[0x197D],
        Some(0),
        PERCENT,
    ),
    dev(
        "ROG Gladius III Wireless AimPoint",
        &[0x1A72, 0x1A70],
        &[0x1A70],
        Some(0),
        PERCENT,
    ),
    dev(
        "ROG Gladius III Wireless AimPoint EVA-02",
        &[0x1B0C, 0x1B0A],
        &[0x1B0A],
        Some(0),
        PERCENT,
    ),
    dev(
        "ROG Keris Wireless",
        &[0x1960, 0x195E],
        &[0x195E],
        Some(0),
        QUARTERS,
    ),
    dev(
        "ROG Keris Wireless EVA Edition",
        &[0x1A59, 0x1A57],
        &[0x1A57],
        Some(0),
        QUARTERS,
    ),
    dev(
        "ROG Keris Wireless AimPoint",
        &[0x1A68, 0x1A66],
        &[0x1A66],
        Some(0),
        PERCENT,
    ),
    dev("ROG Keris II Ace", &[0x1B16], &[0x1B16], Some(0), PERCENT),
    dev(
        "ROG Keris II Origin",
        &[0x1C0C, 0x1D4C],
        &[0x1C0C, 0x1D4C],
        Some(0),
        PERCENT,
    ),
    dev(
        "ROG Harpe Ace Aim Lab Edition",
        &[0x1A94, 0x1A92],
        &[0x1A92],
        Some(0),
        PERCENT,
    ),
    dev(
        "ROG Harpe Ace Extreme",
        &[0x1B67],
        &[0x1B67],
        Some(0),
        PERCENT,
    ),
    dev("ROG Harpe Ace Mini", &[0x1B63], &[0x1B63], Some(0), PERCENT),
    dev("ROG Harpe II Ace", &[0x1C69], &[0x1C69], Some(0), PERCENT),
    dev(
        "ROG Chakram X",
        &[0x1A1A, 0x1A18],
        &[0x1A18],
        Some(0),
        PERCENT,
    ),
    dev(
        "ROG Chakram",
        &[0x18E5, 0x18E3],
        &[0x18E3],
        Some(0),
        QUARTERS,
    ),
    dev(
        "ROG Pugio II",
        &[0x1908, 0x1906],
        &[0x1906],
        Some(0),
        QUARTERS,
    ),
    dev(
        "ROG Spatha X",
        &[0x1979, 0x1977],
        &[0x1977],
        Some(0),
        PERCENT,
    ),
    dev(
        "ROG Strix Impact II Wireless",
        &[0x1949, 0x1947],
        &[0x1947],
        Some(0),
        QUARTERS,
    ),
    dev(
        "TUF Gaming M4 Wireless",
        &[0x19F4, 0x1A8D],
        &[],
        Some(0),
        PERCENT,
    ),
    dev(
        "TX Gaming Mini",
        &[0x1AF5, 0x1AF3],
        &[0x1AF3],
        Some(0),
        PERCENT,
    ),
    dev(
        "TUF Gaming Mini Miku Edition",
        &[0x1C57, 0x1C56],
        &[0x1C56],
        Some(0),
        PERCENT,
    ),
    // Mice on other interfaces.
    dev("ROG Gladius II Wireless", &[0x18A0], &[], Some(2), QUARTERS),
    dev("ASUS MD200", &[0x1A24], &[], Some(2), PERCENT),
    dev("ASUS MD301", &[0x1CD3, 0x1CD5], &[0x1CD5], Some(3), PERCENT),
    dev(
        "ROG Strix Carry",
        &[0x18B4],
        &[],
        Some(1),
        Protocol::Mouse {
            level_at: 6,
            quarters: true,
        },
    ),
    // Keyboards.
    dev("ROG Azoth", &[0x1A85, 0x1A83], &[0x1A83], Some(1), KEYBOARD),
    dev("ROG Azoth Extreme", &[0x1B3F], &[], Some(1), KEYBOARD),
    dev("ROG Azoth Extreme SE", &[0x1CEF], &[], Some(1), KEYBOARD),
    dev(
        "ROG Azoth X",
        &[0x1C25, 0x1C24],
        &[0x1C24],
        Some(1),
        KEYBOARD,
    ),
    dev(
        "ROG Falchion",
        &[0x193E, 0x193C],
        &[0x193C],
        Some(1),
        FALCHION,
    ),
    dev(
        "ROG Falchion RX Low Profile",
        &[0x1B04],
        &[],
        Some(1),
        FALCHION,
    ),
    dev(
        "ROG Strix Scope II 96 Wireless",
        &[0x1AAE],
        &[],
        Some(1),
        KEYBOARD,
    ),
    dev(
        "ROG Strix Scope II 96 RX Wireless",
        &[0x1B78],
        &[],
        Some(1),
        KEYBOARD,
    ),
    dev("ROG Claymore II", &[0x196B], &[], None, KEYBOARD),
    dev(
        "ROG Strix Scope RX TKL Wireless",
        &[0x1A07],
        &[],
        None,
        KEYBOARD,
    ),
    // Headsets.
    dev(
        "ROG Delta II",
        &[0x1AFA, 0x1D41],
        &[],
        None,
        Protocol::Headset,
    ),
    dev("ROG Pelta", &[0x1B84], &[], None, Protocol::Headset),
];

/// Earbuds kept apart: their dongle reports two buds.
pub const EARBUDS: [Model; 1] = [dev(
    "ROG Cetra SpeedNova",
    &[0x1AD3],
    &[],
    None,
    Protocol::Earbuds,
)];

/// Devices as they appear in the Omni receiver's pairing list.
pub const OMNI: [Model; 15] = [
    dev(
        "ROG Harpe Ace Aim Lab Edition",
        &[0x1A94],
        &[],
        None,
        PERCENT,
    ),
    dev(
        "ROG Harpe Ace Extreme",
        &[0x1B68, 0x1B69],
        &[],
        None,
        PERCENT,
    ),
    dev("ROG Harpe Ace Mini", &[0x1B65], &[], None, PERCENT),
    dev(
        "ROG Gladius III Wireless AimPoint",
        &[0x1A72],
        &[],
        None,
        PERCENT,
    ),
    dev(
        "ROG Keris Wireless AimPoint",
        &[0x1A68, 0x1A6A],
        &[],
        None,
        PERCENT,
    ),
    dev("ROG Keris II Ace", &[0x1B1A, 0x1B18], &[], None, PERCENT),
    dev("ROG Keris II Origin", &[0x1C0E, 0x1D4E], &[], None, PERCENT),
    dev(
        "ROG Strix Impact III Wireless",
        &[0x1AD7],
        &[],
        None,
        PERCENT,
    ),
    dev("ROG Azoth", &[0x1A85], &[], None, KEYBOARD),
    dev("ROG Azoth Extreme", &[0x1B42], &[], None, KEYBOARD),
    dev("ROG Azoth Extreme SE", &[0x1CF1], &[], None, KEYBOARD),
    dev(
        "ROG Strix Scope II 96 Wireless",
        &[0x1AB0],
        &[],
        None,
        KEYBOARD,
    ),
    dev(
        "ROG Strix Scope II 96 RX Wireless",
        &[0x1B7A],
        &[],
        None,
        KEYBOARD,
    ),
    dev(
        "ROG Falchion RX Low Profile",
        &[0x1B06],
        &[],
        None,
        FALCHION,
    ),
    // Only the mouse channel is known for the Harpe II dongle; listed for completeness.
    dev("ROG Harpe II Ace", &[], &[], None, PERCENT),
];

pub fn direct_model(pid: u16) -> Option<&'static Model> {
    DIRECT
        .iter()
        .chain(EARBUDS.iter())
        .find(|m| m.pids.contains(&pid))
}

pub fn omni_model(pid: u16) -> Option<&'static Model> {
    OMNI.iter().find(|m| m.pids.contains(&pid))
}

fn matches(group: &[HidInfo]) -> bool {
    group.first().is_some_and(|i| {
        i.vid == VID_ASUS
            && (direct_model(i.pid).is_some() || matches!(i.pid, OMNI_RECEIVER | HARPE_II_DONGLE))
    })
}

pub const DRIVER: Driver = Driver {
    name: "ASUS ROG",
    vendor_ids: &[VID_ASUS],
    silence_is_normal: true,
    matches,
    #[cfg(windows)]
    read,
};

/// The battery query of a protocol on a channel with `report_id`.
pub fn battery_request(protocol: Protocol, report_id: u8) -> [u8; 3] {
    match protocol {
        Protocol::Mouse { .. } => [report_id, 0x12, 0x07],
        Protocol::Keyboard { .. } => [report_id, 0x12, 0x01],
        Protocol::Headset | Protocol::Earbuds => [HEADSET_REPORT, 0x12, 0x07],
    }
}

/// Headset charging query.
pub const HEADSET_CHARGING_REQUEST: [u8; 3] = [HEADSET_REPORT, 0x12, 0x08];

/// Pairing list query on the Omni receiver.
pub const PAIRING_REQUEST: [u8; 4] = [PAIRING_REPORT, 0xA0, 0x00, 0x00];

/// The reply without its report id. Windows already drops a zero report id.
fn body(reply: &[u8], report_id: u8) -> &[u8] {
    match reply.split_first() {
        Some((first, rest)) if report_id != 0 && *first == report_id => rest,
        _ => reply,
    }
}

/// Is this the answer to [`battery_request`] (or the device's error reply `FF AA`)?
pub fn is_battery_reply(protocol: Protocol, reply: &[u8], report_id: u8) -> bool {
    match protocol {
        Protocol::Mouse { .. } | Protocol::Keyboard { .. } => {
            let header = battery_request(protocol, report_id);
            let b = body(reply, report_id);
            (b.len() >= 11 && b[..2] == header[1..]) || (b.len() >= 2 && b[..2] == [0xFF, 0xAA])
        }
        Protocol::Headset | Protocol::Earbuds => reply.len() >= 9 && reply[0] == HEADSET_REPORT,
    }
}

/// `Some((percent, charging))`; `None` on an error reply or when only the dongle answers
/// because the device is asleep or switched off. Headsets report charging separately
/// (see [`parse_headset_charging`]), so `charging` is `false` for them.
pub fn parse_battery(protocol: Protocol, reply: &[u8], report_id: u8) -> Option<(u8, bool)> {
    if !is_battery_reply(protocol, reply, report_id) {
        return None;
    }
    let b = body(reply, report_id);
    let (raw, charging, quarters) = match protocol {
        Protocol::Mouse { level_at, quarters } => {
            if b[..2] != [0x12, 0x07] {
                return None;
            }
            (*b.get(level_at)?, b[9] > 0, quarters)
        }
        Protocol::Keyboard { level_at } => {
            if b[..2] != [0x12, 0x01] {
                return None;
            }
            (*b.get(level_at)?, b[8] == 1, false)
        }
        Protocol::Headset => (reply[6], false, false),
        Protocol::Earbuds => (reply[6].min(reply[7]), false, false),
    };
    if raw == 0 && !charging {
        return None;
    }
    let percent = if quarters {
        u16::from(raw) * 25
    } else {
        u16::from(raw)
    };
    (percent <= 100).then_some((percent as u8, charging))
}

pub fn parse_headset_charging(protocol: Protocol, reply: &[u8]) -> bool {
    match (protocol, reply.get(5)) {
        (Protocol::Headset, Some(state)) => *state == 1,
        (Protocol::Earbuds, Some(state)) => state & 0x03 != 0,
        _ => false,
    }
}

/// Product ids of the devices paired with an Omni receiver, in slot order.
pub fn parse_pairing(reply: &[u8]) -> Vec<u16> {
    if reply.first() != Some(&PAIRING_REPORT) {
        return Vec::new();
    }
    reply
        .get(5..)
        .unwrap_or_default()
        .chunks_exact(4)
        .map(|slot| u16::from_le_bytes([slot[0], slot[1]]))
        .take_while(|pid| *pid != 0)
        .collect()
}

fn path_has(info: &HidInfo, part: &str) -> bool {
    info.path.to_ascii_lowercase().contains(part)
}

/// The collection that takes the commands of a directly attached device.
fn direct_writer<'a>(group: &'a [HidInfo], model: &Model) -> Option<&'a HidInfo> {
    group.iter().find(|i| {
        i.output_len >= 64
            && match (model.protocol, model.interface) {
                (Protocol::Headset | Protocol::Earbuds, _) => i.usage_page == 0xFF00,
                (_, Some(n)) => i.interface == n || (n == 0 && i.interface == -1),
                (_, None) => i.usage_page >= 0xFF00,
            }
    })
}

fn kind_of(protocol: Protocol) -> crate::model::Kind {
    use crate::model::Kind;
    match protocol {
        Protocol::Mouse { .. } => Kind::Mouse,
        Protocol::Keyboard { .. } => Kind::Keyboard,
        Protocol::Headset | Protocol::Earbuds => Kind::Headset,
    }
}

#[cfg(windows)]
fn read(group: &[HidInfo], product: &str) -> Vec<crate::model::Reading> {
    use crate::model::Link;

    let Some(first) = group.first() else {
        return Vec::new();
    };
    let pid = first.pid;
    if let Some(model) = direct_model(pid) {
        let link = if model.wired.contains(&pid) {
            Link::Usb
        } else {
            Link::Rf24
        };
        let writer = direct_writer(group, model);
        return writer
            .and_then(|w| ask(group, w, model, model.name, pid, DIRECT_REPORT, link))
            .into_iter()
            .collect();
    }
    let mouse_channel = group
        .iter()
        .find(|i| path_has(i, "&mi_02&col03") && i.output_len >= 4);
    if pid == HARPE_II_DONGLE {
        let name = if product.to_ascii_uppercase().contains("EXTREME") {
            "ROG Harpe II Ace Extreme Edition"
        } else {
            "ROG Harpe II Ace"
        };
        let model = &OMNI[OMNI.len() - 1];
        return mouse_channel
            .and_then(|w| ask(group, w, model, name, pid, OMNI_MOUSE_REPORT, Link::Rf24))
            .into_iter()
            .collect();
    }
    // Omni receiver: every supported device in its pairing list.
    let keyboard_channel = group
        .iter()
        .find(|i| path_has(i, "&mi_02&col02") && i.output_len >= 4);
    let mut readings = Vec::new();
    for paired in omni_paired(group) {
        let Some(model) = omni_model(paired) else {
            continue;
        };
        let (channel, report_id) = match model.protocol {
            Protocol::Keyboard { .. } => (keyboard_channel, OMNI_KEYBOARD_REPORT),
            _ => (mouse_channel, OMNI_MOUSE_REPORT),
        };
        if readings
            .iter()
            .any(|r: &crate::model::Reading| r.kind == kind_of(model.protocol))
        {
            continue; // one mouse and one keyboard per receiver
        }
        if let Some(reading) =
            channel.and_then(|w| ask(group, w, model, model.name, paired, report_id, Link::Rf24))
        {
            readings.push(reading);
        }
    }
    readings
}

#[cfg(windows)]
fn ask(
    group: &[HidInfo],
    writer: &HidInfo,
    model: &Model,
    name: &str,
    pid: u16,
    report_id: u8,
    link: crate::model::Link,
) -> Option<crate::model::Reading> {
    use crate::model::Reading;
    use crate::win::hid::{Session, Via};
    use std::time::Duration;

    let protocol = model.protocol;
    let mut session = Session::open(group, writer)?;
    let reply = session.ask(
        &battery_request(protocol, report_id),
        Via::Output,
        Duration::from_millis(900),
        |r| is_battery_reply(protocol, r, report_id),
    )?;
    let (percent, mut charging) = parse_battery(protocol, &reply, report_id)?;
    if matches!(protocol, Protocol::Headset | Protocol::Earbuds) {
        charging = session
            .ask(
                &HEADSET_CHARGING_REQUEST,
                Via::Output,
                Duration::from_millis(900),
                |r| r.len() >= 6 && r[0] == HEADSET_REPORT,
            )
            .is_some_and(|r| parse_headset_charging(protocol, &r));
    }
    let id = format!("asus:{:04x}:{}", pid, super::instance_of(writer));
    Some(
        Reading::new(id, name, kind_of(protocol), link)
            .battery(Some(percent))
            .charging(charging && percent < 100)
            .full(charging && percent >= 100),
    )
}

/// Product ids paired with an Omni receiver.
#[cfg(windows)]
fn omni_paired(group: &[HidInfo]) -> Vec<u16> {
    use crate::win::hid::{Session, Via};
    use std::time::Duration;

    let Some(channel) = group
        .iter()
        .find(|i| path_has(i, "&mi_02&col01") && i.output_len >= PAIRING_REQUEST.len())
    else {
        return Vec::new();
    };
    let Some(mut session) = Session::open(group, channel) else {
        return Vec::new();
    };
    let paired = session
        .ask(
            &PAIRING_REQUEST,
            Via::Output,
            Duration::from_millis(900),
            |r| r.len() >= 9 && r[0] == PAIRING_REPORT,
        )
        .map(|reply| parse_pairing(&reply))
        .unwrap_or_default();
    if !paired.is_empty() && paired.iter().all(|p| omni_model(*p).is_none()) {
        log::debug!("ASUS Omni receiver: no supported device among {paired:04x?}");
    }
    paired
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vendor::collection;

    #[test]
    fn matching_is_limited_to_known_devices() {
        let keris = [collection(VID_ASUS, 0x1960, 0xFF01, 1, 0, (65, 65, 0))];
        assert!(matches(&keris));
        let omni = [collection(
            VID_ASUS,
            OMNI_RECEIVER,
            0xFF01,
            1,
            2,
            (64, 64, 0),
        )];
        assert!(matches(&omni));
        let laptop_keyboard = [collection(VID_ASUS, 0x19B6, 0xFF31, 0x76, 0, (64, 64, 0))];
        assert!(!matches(&laptop_keyboard), "an ASUS laptop keyboard");
        let wired_only = [collection(VID_ASUS, 0x197B, 0xFF01, 1, 0, (65, 65, 0))];
        assert!(!matches(&wired_only), "Gladius III without battery");
        let tuf_k3 = [collection(VID_ASUS, 0x194B, 0xFF00, 1, 1, (64, 64, 0))];
        assert!(!matches(&tuf_k3), "wired keyboard");
    }

    #[test]
    fn tables_are_consistent() {
        let mut pids: Vec<u16> = DIRECT
            .iter()
            .chain(EARBUDS.iter())
            .flat_map(|m| m.pids.iter().copied())
            .collect();
        for model in DIRECT.iter() {
            for wired in model.wired {
                assert!(model.pids.contains(wired), "{}", model.name);
            }
        }
        let count = pids.len();
        pids.sort_unstable();
        pids.dedup();
        assert_eq!(pids.len(), count, "direct product ids are unique");
        assert_eq!(direct_model(0x1A57).unwrap().protocol, QUARTERS);
        assert_eq!(omni_model(0x1B69).unwrap().name, "ROG Harpe Ace Extreme");
        assert_eq!(omni_model(0x1D4E).unwrap().name, "ROG Keris II Origin");
        assert_eq!(omni_model(0x1B06).unwrap().protocol, FALCHION);
    }

    #[test]
    fn requests() {
        assert_eq!(battery_request(PERCENT, DIRECT_REPORT), [0x00, 0x12, 0x07]);
        assert_eq!(
            battery_request(PERCENT, OMNI_MOUSE_REPORT),
            [0x03, 0x12, 0x07]
        );
        assert_eq!(
            battery_request(KEYBOARD, OMNI_KEYBOARD_REPORT),
            [0x02, 0x12, 0x01]
        );
        assert_eq!(battery_request(Protocol::Headset, 0), [0xCC, 0x12, 0x07]);
    }

    fn mouse_reply(level: u8, charging: u8) -> [u8; 64] {
        // Windows strips the zero report id: 12 07 ?? ?? <level> … <charging at 9>.
        let mut r = [0u8; 64];
        r[..2].copy_from_slice(&[0x12, 0x07]);
        r[4] = level;
        r[9] = charging;
        r
    }

    #[test]
    fn mouse_battery() {
        assert_eq!(
            parse_battery(PERCENT, &mouse_reply(76, 0), 0),
            Some((76, false))
        );
        assert_eq!(
            parse_battery(PERCENT, &mouse_reply(76, 1), 0),
            Some((76, true))
        );
        assert_eq!(
            parse_battery(PERCENT, &mouse_reply(0, 1), 0),
            Some((0, true))
        );
        assert_eq!(
            parse_battery(PERCENT, &mouse_reply(0, 0), 0),
            None,
            "asleep"
        );
        assert_eq!(
            parse_battery(QUARTERS, &mouse_reply(3, 0), 0),
            Some((75, false))
        );
        assert_eq!(parse_battery(QUARTERS, &mouse_reply(5, 0), 0), None);
        let error = [0xFF, 0xAA, 0, 0];
        assert!(is_battery_reply(PERCENT, &error, 0));
        assert_eq!(parse_battery(PERCENT, &error, 0), None);
    }

    #[test]
    fn strix_carry_level_position() {
        let carry = direct_model(0x18B4).unwrap().protocol;
        let mut r = mouse_reply(9, 0);
        r[6] = 2;
        assert_eq!(parse_battery(carry, &r, 0), Some((50, false)));
    }

    #[test]
    fn omni_mouse_keeps_report_id() {
        let mut r = [0u8; 64];
        r[..3].copy_from_slice(&[0x03, 0x12, 0x07]);
        r[5] = 64;
        assert_eq!(
            parse_battery(PERCENT, &r, OMNI_MOUSE_REPORT),
            Some((64, false))
        );
        assert_eq!(
            parse_battery(KEYBOARD, &r, OMNI_MOUSE_REPORT),
            None,
            "wrong command"
        );
    }

    #[test]
    fn keyboard_battery() {
        // G-Helper: response[6] level, response[9] == 1 charging (with report id).
        let mut r = [0u8; 64];
        r[..3].copy_from_slice(&[0x02, 0x12, 0x01]);
        r[6] = 88;
        r[9] = 1;
        assert_eq!(
            parse_battery(KEYBOARD, &r, OMNI_KEYBOARD_REPORT),
            Some((88, true))
        );
        // Falchion: gauge at [6], percentage at [11].
        let mut f = [0u8; 64];
        f[..2].copy_from_slice(&[0x12, 0x01]);
        f[5] = 7;
        f[10] = 68;
        assert_eq!(parse_battery(FALCHION, &f, 0), Some((68, false)));
    }

    #[test]
    fn headset_battery() {
        let mut r = [0u8; 64];
        r[0] = 0xCC;
        r[6] = 55;
        assert_eq!(parse_battery(Protocol::Headset, &r, 0), Some((55, false)));
        r[7] = 40;
        assert_eq!(parse_battery(Protocol::Earbuds, &r, 0), Some((40, false)));
        r[6] = 101;
        assert_eq!(parse_battery(Protocol::Headset, &r, 0), None);
        let mut c = [0u8; 8];
        c[0] = 0xCC;
        c[5] = 1;
        assert!(parse_headset_charging(Protocol::Headset, &c));
        c[5] = 2;
        assert!(!parse_headset_charging(Protocol::Headset, &c));
        assert!(parse_headset_charging(Protocol::Earbuds, &c));
        assert!(!is_battery_reply(Protocol::Headset, &[0x00; 9], 0));
    }

    #[test]
    fn pairing_list() {
        let mut r = [0u8; 64];
        r[0] = PAIRING_REPORT;
        r[5..9].copy_from_slice(&[0x85, 0x1A, 0x01, 0x00]); // Azoth keyboard
        r[9..13].copy_from_slice(&[0x94, 0x1A, 0x02, 0x00]); // Harpe Ace Aim Lab
        assert_eq!(parse_pairing(&r), vec![0x1A85, 0x1A94]);
        let names: Vec<&str> = parse_pairing(&r)
            .iter()
            .filter_map(|p| omni_model(*p).map(|m| m.name))
            .collect();
        assert_eq!(names, ["ROG Azoth", "ROG Harpe Ace Aim Lab Edition"]);
        assert!(parse_pairing(&[0x02, 0, 0, 0, 0, 0x94, 0x1A, 0, 0]).is_empty());
    }
}
