//! SteelSeries Arctis headsets. Each family uses a different request and reply layout,
//! so the driver is table-driven (layouts as documented by the HeadsetControl project).

use super::Driver;
use crate::hid::HidInfo;

pub const VID_STEELSERIES: u16 = 0x1038;

pub struct Model {
    pub name: &'static str,
    pub pids: &'static [u16],
    /// USB interface that accepts the request.
    pub interface: i32,
    /// Required usage page, `0` for any.
    pub usage_page: u16,
    pub request: &'static [u8],
    pub min_len: usize,
    /// Index of the level in the reply and the raw values that mean 0 % and 100 %.
    pub battery_byte: usize,
    pub scale: (u8, u8),
    /// `(index, value)`: the headset is switched off when the reply has `value` at `index`.
    pub offline: Option<(usize, u8)>,
    /// `(index, value)`: the headset is charging when the reply has `value` at `index`.
    pub charging: Option<(usize, u8)>,
}

pub const MODELS: [Model; 3] = [
    Model {
        name: "SteelSeries Arctis 7",
        pids: &[0x1260, 0x12AD, 0x1252, 0x1280],
        interface: 5,
        usage_page: 0,
        request: &[0x06, 0x18],
        min_len: 3,
        battery_byte: 2,
        scale: (0, 100),
        offline: None,
        charging: None,
    },
    Model {
        name: "SteelSeries Arctis 1 Wireless",
        pids: &[0x12B3, 0x12B6, 0x12D5, 0x12D7],
        interface: 3,
        usage_page: 0,
        request: &[0x06, 0x12],
        min_len: 4,
        battery_byte: 3,
        scale: (0, 100),
        offline: Some((2, 0x01)),
        charging: None,
    },
    Model {
        name: "SteelSeries Arctis Nova 7",
        pids: &[0x2202, 0x2206, 0x220A],
        interface: 3,
        usage_page: 0xFFC0,
        request: &[0x00, 0xB0],
        min_len: 4,
        battery_byte: 2,
        scale: (0, 4),
        offline: Some((3, 0x00)),
        charging: Some((3, 0x01)),
    },
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
    name: "SteelSeries Arctis",
    vendor_ids: &[VID_STEELSERIES],
    silence_is_normal: true,
    matches,
    #[cfg(windows)]
    read,
};

/// `Some((percent, charging))`, or `None` when the headset is switched off or the reply
/// is too short.
pub fn parse(model: &Model, reply: &[u8]) -> Option<(u8, bool)> {
    if reply.len() < model.min_len.max(model.battery_byte + 1) {
        return None;
    }
    let at = |spec: Option<(usize, u8)>| {
        spec.is_some_and(|(index, value)| reply.get(index) == Some(&value))
    };
    if at(model.offline) {
        return None;
    }
    let (lo, hi) = (f64::from(model.scale.0), f64::from(model.scale.1));
    let percent = (f64::from(reply[model.battery_byte]) - lo) * 100.0 / (hi - lo);
    Some((crate::model::clamp_percent(percent), at(model.charging)))
}

#[cfg(windows)]
fn read(group: &[HidInfo], _product: &str) -> Vec<crate::model::Reading> {
    use crate::model::{Kind, Link, Reading};
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
        return vec![Reading::new(id, model.name, Kind::Headset, Link::Rf24)
            .battery(Some(percent))
            .charging(charging)];
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
}
