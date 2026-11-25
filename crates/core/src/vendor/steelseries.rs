//! SteelSeries Arctis headsets and wireless mice. Each family uses a different request
//! and reply layout, so the driver is table-driven (headset layouts as documented by the
//! HeadsetControl project, mouse layouts by rivalcfg).

use super::Driver;
use crate::hid::HidInfo;
use crate::model::Kind;

pub const VID_STEELSERIES: u16 = 0x1038;

/// How the raw level byte becomes a percentage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// Linear: the raw values that mean 0 % and 100 %.
    Scale(u8, u8),
    /// Aerox / Prime mice: bit 7 charging, `(raw - 1) * 5` percent.
    Steps5,
}

pub struct Model {
    pub name: &'static str,
    pub kind: Kind,
    pub pids: &'static [u16],
    /// Cable product ids (the device charges); the others are the wireless dongle.
    pub wired: &'static [u16],
    /// USB interface that accepts the request.
    pub interface: i32,
    /// Required usage page, `0` for any.
    pub usage_page: u16,
    pub request: &'static [u8],
    pub min_len: usize,
    /// Index of the level in the reply.
    pub battery_byte: usize,
    pub level: Level,
    /// `(index, value)`: the device is switched off when the reply has `value` at `index`.
    pub offline: Option<(usize, u8)>,
    /// `(index, values)`: the device is charging when the reply has one of `values` there.
    pub charging: Option<(usize, &'static [u8])>,
}

const NOVA_STATUS: &[u8] = &[0x00, 0xB0];

// One positional row per headset keeps the table compact and close to its source.
#[allow(clippy::too_many_arguments)]
const fn headset(
    name: &'static str,
    pids: &'static [u16],
    interface: i32,
    usage_page: u16,
    request: &'static [u8],
    min_len: usize,
    battery_byte: usize,
    level: Level,
    offline: Option<(usize, u8)>,
    charging: Option<(usize, &'static [u8])>,
) -> Model {
    Model {
        name,
        kind: Kind::Headset,
        pids,
        wired: &[],
        interface,
        usage_page,
        request,
        min_len,
        battery_byte,
        level,
        offline,
        charging,
    }
}

/// Arctis Nova 7 generation: status `00 B0`, level at byte 2, byte 3 off / charging.
const fn nova7(name: &'static str, pids: &'static [u16], level: Level) -> Model {
    headset(
        name,
        pids,
        3,
        0xFFC0,
        NOVA_STATUS,
        4,
        2,
        level,
        Some((3, 0x00)),
        Some((3, &[0x01, 0x02])),
    )
}

/// Arctis Nova 3P / 5: status `00 B0`, level at byte 3, byte 1 = 2 off, byte 4 = 1 charging.
const fn nova5(name: &'static str, pids: &'static [u16], min_len: usize) -> Model {
    headset(
        name,
        pids,
        3,
        0xFFC0,
        NOVA_STATUS,
        min_len,
        3,
        Level::Scale(0, 100),
        Some((1, 0x02)),
        Some((4, &[0x01])),
    )
}

/// Mouse; `wireless` selects the dongle request of the Aerox / Prime family.
const fn mouse(
    name: &'static str,
    pids: &'static [u16],
    wired: &'static [u16],
    interface: i32,
    request: &'static [u8],
    family_b: bool,
) -> Model {
    Model {
        name,
        kind: Kind::Mouse,
        pids,
        wired,
        interface,
        usage_page: 0,
        request,
        min_len: if family_b { 3 } else { 2 },
        battery_byte: if family_b { 0 } else { 1 },
        level: if family_b {
            Level::Scale(0, 100)
        } else {
            Level::Steps5
        },
        offline: None,
        charging: if family_b { Some((2, &[0x01])) } else { None },
    }
}

const PCT: Level = Level::Scale(0, 100);
const QUARTERS: Level = Level::Scale(0, 4);
/// Aerox / Prime battery query, wired and with the wireless flag (bit 6).
const AEROX_WIRED: &[u8] = &[0x00, 0x92];
const AEROX_WIRELESS: &[u8] = &[0x00, 0xD2];
/// Rival 3 Wireless / Rival 650 battery query.
const RIVAL: &[u8] = &[0x00, 0xAA, 0x01];

pub const MODELS: [Model; 25] = [
    headset(
        "SteelSeries Arctis 7",
        &[0x1260, 0x12AD, 0x1252, 0x1280],
        5,
        0,
        &[0x06, 0x18],
        3,
        2,
        PCT,
        None,
        None,
    ),
    headset(
        "SteelSeries Arctis 1 Wireless",
        &[0x12B3, 0x12B6, 0x12D5, 0x12D7],
        3,
        0,
        &[0x06, 0x12],
        4,
        3,
        PCT,
        Some((2, 0x01)),
        None,
    ),
    headset(
        "SteelSeries Arctis 9",
        &[0x12C2, 0x12C4],
        0,
        0,
        &[0x00, 0x20],
        5,
        3,
        Level::Scale(0x64, 0x9A),
        Some((4, 0xFF)),
        Some((4, &[0x01])),
    ),
    headset(
        "SteelSeries Arctis Nova Pro Wireless",
        &[0x12E0, 0x12E5],
        4,
        0,
        &[0x06, 0xB0],
        16,
        6,
        Level::Scale(0, 8),
        Some((15, 0x01)),
        Some((15, &[0x02])),
    ),
    headset(
        "SteelSeries Arctis 7+",
        &[0x220E, 0x2212, 0x2216, 0x2236],
        3,
        0xFFC0,
        NOVA_STATUS,
        4,
        2,
        QUARTERS,
        Some((1, 0x01)),
        Some((3, &[0x01])),
    ),
    // Older Nova 7 firmware reports the level in quarters, newer ones in percent.
    nova7(
        "SteelSeries Arctis Nova 7",
        &[0x2202, 0x223A, 0x227A],
        QUARTERS,
    ),
    nova7("SteelSeries Arctis Nova 7", &[0x22A1, 0x227E, 0x22A9], PCT),
    nova7("SteelSeries Arctis Nova 7X", &[0x2206, 0x22A4], QUARTERS),
    nova7(
        "SteelSeries Arctis Nova 7X",
        &[0x2258, 0x229E, 0x22AD, 0x22A5],
        PCT,
    ),
    nova7("SteelSeries Arctis Nova 7P", &[0x220A], QUARTERS),
    nova7("SteelSeries Arctis Nova 7P", &[0x22A7, 0x2298], PCT),
    nova5("SteelSeries Arctis Nova 3P Wireless", &[0x2269, 0x226D], 5),
    nova5("SteelSeries Arctis Nova 5", &[0x2232, 0x2253], 16),
    // Mice, dongle and cable.
    mouse(
        "SteelSeries Aerox 3 Wireless",
        &[0x1838, 0x1878],
        &[],
        3,
        AEROX_WIRELESS,
        false,
    ),
    mouse(
        "SteelSeries Aerox 3 Wireless",
        &[0x183A, 0x187A],
        &[0x183A, 0x187A],
        3,
        AEROX_WIRED,
        false,
    ),
    mouse(
        "SteelSeries Aerox 5 Wireless",
        &[0x1852, 0x185C, 0x1860],
        &[],
        3,
        AEROX_WIRELESS,
        false,
    ),
    mouse(
        "SteelSeries Aerox 5 Wireless",
        &[0x1854, 0x185E, 0x1862],
        &[0x1854, 0x185E, 0x1862],
        3,
        AEROX_WIRED,
        false,
    ),
    mouse(
        "SteelSeries Aerox 9 Wireless",
        &[0x1858, 0x1874],
        &[],
        3,
        AEROX_WIRELESS,
        false,
    ),
    mouse(
        "SteelSeries Aerox 9 Wireless",
        &[0x185A, 0x1876],
        &[0x185A, 0x1876],
        3,
        AEROX_WIRED,
        false,
    ),
    mouse(
        "SteelSeries Prime Wireless",
        &[0x1840, 0x1848],
        &[],
        3,
        AEROX_WIRELESS,
        false,
    ),
    mouse(
        "SteelSeries Prime Wireless",
        &[0x1842, 0x184A],
        &[0x1842, 0x184A],
        3,
        AEROX_WIRED,
        false,
    ),
    mouse(
        "SteelSeries Rival 3 Wireless",
        &[0x1830],
        &[],
        3,
        RIVAL,
        true,
    ),
    mouse(
        "SteelSeries Rival 3 Wireless Gen 2",
        &[0x1872],
        &[],
        3,
        RIVAL,
        true,
    ),
    mouse(
        "SteelSeries Rival 650 Wireless",
        &[0x1726],
        &[],
        0,
        RIVAL,
        true,
    ),
    mouse(
        "SteelSeries Rival 650 Wireless",
        &[0x172B],
        &[0x172B],
        0,
        RIVAL,
        true,
    ),
];

pub fn model_for(pid: u16) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.pids.contains(&pid))
}

fn matches(group: &[HidInfo]) -> bool {
    group
        .first()
        .is_some_and(|i| i.vid == VID_STEELSERIES && model_for(i.pid).is_some())
}

pub const DRIVER: Driver = Driver {
    name: "SteelSeries",
    vendor_ids: &[VID_STEELSERIES],
    silence_is_normal: true,
    matches,
    #[cfg(windows)]
    read,
};

/// `Some((percent, charging))`, or `None` when the device is switched off or the reply
/// is too short.
pub fn parse(model: &Model, reply: &[u8]) -> Option<(u8, bool)> {
    if reply.len() < model.min_len.max(model.battery_byte + 1) {
        return None;
    }
    if model
        .offline
        .is_some_and(|(index, value)| reply.get(index) == Some(&value))
    {
        return None;
    }
    let mut charging = model
        .charging
        .is_some_and(|(index, values)| reply.get(index).is_some_and(|v| values.contains(v)));
    let raw = reply[model.battery_byte];
    let percent = match model.level {
        Level::Scale(lo, hi) => {
            let (lo, hi) = (f64::from(lo), f64::from(hi));
            (f64::from(raw) - lo) * 100.0 / (hi - lo)
        }
        Level::Steps5 => {
            charging = raw & 0x80 != 0;
            let steps = raw & 0x7F;
            if !(1..=21).contains(&steps) {
                return None;
            }
            f64::from(steps - 1) * 5.0
        }
    };
    if model.kind == Kind::Mouse && !(0.0..=100.0).contains(&percent) {
        return None;
    }
    Some((crate::model::clamp_percent(percent), charging))
}

#[cfg(windows)]
fn read(group: &[HidInfo], _product: &str) -> Vec<crate::model::Reading> {
    use crate::model::{Link, Reading};
    use crate::win::hid::{Session, Via};
    use std::time::Duration;

    let Some(model) = group.first().and_then(|i| model_for(i.pid)) else {
        return Vec::new();
    };
    let writers = group.iter().filter(|i| {
        (i.interface == -1 || i.interface == model.interface)
            && i.output_len > 0
            && (model.usage_page == 0 || i.usage_page == model.usage_page)
    });
    for info in writers {
        let Some(mut session) = Session::open(group, info) else {
            continue;
        };
        let reply = session.ask(
            model.request,
            Via::Output,
            Duration::from_millis(900),
            |r| r.len() >= model.min_len,
        );
        let Some(reply) = reply else { continue };
        let Some((percent, charging)) = parse(model, &reply) else {
            return Vec::new();
        };
        let id = format!("steelseries:{:04x}:{}", info.pid, super::instance_of(info));
        let link = if model.wired.contains(&info.pid) {
            Link::Usb
        } else {
            Link::Rf24
        };
        return vec![Reading::new(id, model.name, model.kind, link)
            .battery(Some(percent))
            .charging(charging && percent < 100)
            .full(charging && percent >= 100)];
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arctis_7() {
        let m = model_for(0x12AD).unwrap();
        assert_eq!(parse(m, &[0x06, 0x18, 83, 0]), Some((83, false)));
        assert_eq!(parse(m, &[0x06, 0x18]), None);
    }

    #[test]
    fn arctis_1_offline() {
        let m = model_for(0x12B3).unwrap();
        assert_eq!(parse(m, &[0x06, 0x12, 0x03, 60]), Some((60, false)));
        assert_eq!(parse(m, &[0x06, 0x12, 0x01, 0]), None);
    }

    #[test]
    fn nova_7_scale_and_charging() {
        let m = model_for(0x2202).unwrap();
        assert_eq!(parse(m, &[0xB0, 0, 4, 3]), Some((100, false)));
        assert_eq!(parse(m, &[0xB0, 0, 2, 1]), Some((50, true)));
        assert_eq!(parse(m, &[0xB0, 0, 1, 3]), Some((25, false)));
        assert_eq!(parse(m, &[0xB0, 0, 0, 0]), None, "headset off");
        assert!(model_for(0xFFFF).is_none());
    }

    #[test]
    fn newer_headsets() {
        let nova7_new = model_for(0x22A1).unwrap();
        assert_eq!(parse(nova7_new, &[0xB0, 0, 83, 2]), Some((83, true)));
        let pro = model_for(0x12E0).unwrap();
        let mut r = [0u8; 16];
        r[6] = 4;
        r[15] = 0x03;
        assert_eq!(parse(pro, &r), Some((50, false)));
        r[15] = 0x01;
        assert_eq!(parse(pro, &r), None, "headset off");
        let nova5 = model_for(0x2232).unwrap();
        let mut r = [0u8; 16];
        r[3] = 70;
        r[4] = 1;
        assert_eq!(parse(nova5, &r), Some((70, true)));
        let arctis9 = model_for(0x12C2).unwrap();
        assert_eq!(parse(arctis9, &[0, 0, 0, 0x9A, 0]), Some((100, false)));
        assert_eq!(parse(arctis9, &[0, 0, 0, 0x7F, 0xFF]), None);
    }

    #[test]
    fn mice() {
        let aerox = model_for(0x1838).unwrap();
        assert_eq!(aerox.request, &[0x00, 0xD2]);
        assert_eq!(model_for(0x183A).unwrap().request, &[0x00, 0x92]);
        assert_eq!(parse(aerox, &[0xD2, 21]), Some((100, false)));
        assert_eq!(parse(aerox, &[0xD2, 0x80 | 11]), Some((50, true)));
        assert_eq!(parse(aerox, &[0xD2, 0]), None, "mouse off");
        assert_eq!(parse(aerox, &[0xD2, 30]), None);
        let rival = model_for(0x1830).unwrap();
        assert_eq!(parse(rival, &[64, 0, 0]), Some((64, false)));
        assert_eq!(parse(rival, &[64, 0, 1]), Some((64, true)));
        assert_eq!(parse(rival, &[180, 0, 0]), None);
    }

    #[test]
    fn product_ids_are_unique() {
        let mut all: Vec<u16> = MODELS.iter().flat_map(|m| m.pids.iter().copied()).collect();
        let count = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), count);
        for m in &MODELS {
            assert!(m.wired.iter().all(|w| m.pids.contains(w)), "{}", m.name);
        }
    }
}
