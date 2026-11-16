//! Logitech HID++ wire format and battery decoding. Pure functions only.

use crate::model::{Kind, Link};

pub const LOGITECH_VID: u16 = 0x046D;
pub const REPORT_SHORT: u8 = 0x10;
pub const REPORT_LONG: u8 = 0x11;
pub const SHORT_LEN: usize = 7;
pub const LONG_LEN: usize = 20;
/// Software id placed in the low nibble of the function byte, so that our replies can be
/// told apart from those meant for G HUB or Options+ running at the same time.
pub const SOFTWARE_ID: u8 = 0x0B;

pub const FEATURE_ROOT: u8 = 0x00;
pub const FEATURE_FW_INFO: u16 = 0x0003;
pub const FEATURE_NAME: u16 = 0x0005;
pub const FEATURE_CENTURION_SOC: u16 = 0x0104;
pub const FEATURE_BATTERY_STATUS: u16 = 0x1000;
pub const FEATURE_BATTERY_VOLTAGE: u16 = 0x1001;
pub const FEATURE_UNIFIED_BATTERY: u16 = 0x1004;
pub const FEATURE_ADC_MEASUREMENT: u16 = 0x1F20;

/// Battery features in order of preference.
pub const BATTERY_FEATURES: [u16; 5] = [
    FEATURE_UNIFIED_BATTERY,
    FEATURE_BATTERY_STATUS,
    FEATURE_CENTURION_SOC,
    FEATURE_ADC_MEASUREMENT,
    FEATURE_BATTERY_VOLTAGE,
];

/// HID++ 1.0 error: the sub-id is unknown, i.e. the device does not speak HID++ 2.0.
pub const ERR_INVALID_SUBID: u8 = 1;
/// The device is paired but switched off or asleep.
pub const ERR_CONNECT_FAIL: u8 = 4;

pub const RAP_GET_REGISTER: u8 = 0x81;
pub const RAP_GET_LONG_REGISTER: u8 = 0x83;
pub const REG_BATTERY_STATUS: u8 = 0x07;
pub const REG_BATTERY_MILEAGE: u8 = 0x0D;
pub const REG_PAIRING_INFO: u8 = 0xB5;
/// Device index that addresses the receiver itself (or a directly attached device).
pub const INDEX_SELF: u8 = 0xFF;

/// Function byte: function number in the high nibble, our software id in the low one.
pub fn function_byte(function: u8) -> u8 {
    (function & 0x0F) << 4 | SOFTWARE_ID
}

/// Build a zero-padded HID++ 2.0 long request.
pub fn long_request(device: u8, feature_index: u8, function: u8, params: &[u8]) -> [u8; LONG_LEN] {
    let mut packet = [0u8; LONG_LEN];
    packet[..4].copy_from_slice(&[REPORT_LONG, device, feature_index, function_byte(function)]);
    let n = params.len().min(LONG_LEN - 4);
    packet[4..4 + n].copy_from_slice(&params[..n]);
    packet
}

/// Build a HID++ 1.0 register access request (always a short report).
pub fn rap_request(device: u8, sub_id: u8, register: u8, params: &[u8]) -> [u8; SHORT_LEN] {
    let mut packet = [0u8; SHORT_LEN];
    packet[..4].copy_from_slice(&[REPORT_SHORT, device, sub_id, register]);
    let n = params.len().min(3);
    packet[4..4 + n].copy_from_slice(&params[..n]);
    packet
}

/// How an incoming report relates to a pending request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The answer; payload starts after the 4-byte header.
    Data(Vec<u8>),
    /// The device or receiver rejected the request with this error code.
    Error(u8),
    /// Something else (notification, reply to another application).
    Unrelated,
}

/// Match a report against a pending HID++ 2.0 request.
pub fn match_reply(report: &[u8], device: u8, feature_index: u8, function: u8) -> Reply {
    let func = function_byte(function);
    if report.len() < 5 || report[1] != device {
        return Reply::Unrelated;
    }
    let code = report.get(5).copied().unwrap_or(0);
    if report[0] == REPORT_LONG && report[2] == feature_index && report[3] == func {
        Reply::Data(report[4..].to_vec())
    } else if report[3] == feature_index
        && report[4] == func
        // Error reports: `0xFF` in HID++ 2.0, `0x8F` (short report) in HID++ 1.0.
        && (report[2] == 0xFF || (report[0] == REPORT_SHORT && report[2] == 0x8F))
    {
        Reply::Error(code)
    } else {
        Reply::Unrelated
    }
}

/// Match a report against a pending HID++ 1.0 register request.
pub fn match_rap_reply(report: &[u8], device: u8, sub_id: u8, register: u8) -> Reply {
    if report.len() < 4 || report[1] != device {
        return Reply::Unrelated;
    }
    let is_hidpp = report[0] == REPORT_SHORT || report[0] == REPORT_LONG;
    if is_hidpp && report[2] == sub_id && report[3] == register {
        Reply::Data(report[4..].to_vec())
    } else if report[0] == REPORT_SHORT && report[2] == 0x8F && report[3] == sub_id {
        Reply::Error(report.get(5).copied().unwrap_or(0))
    } else {
        Reply::Unrelated
    }
}

/// Decoded battery state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
    pub full: bool,
}

/// Typical single-cell Li-ion discharge curve (millivolts → percent).
const VOLTAGE_CURVE: [(u16, u8); 13] = [
    (4186, 100),
    (4067, 90),
    (3989, 80),
    (3922, 70),
    (3859, 60),
    (3811, 50),
    (3778, 40),
    (3751, 30),
    (3717, 20),
    (3671, 10),
    (3646, 5),
    (3579, 2),
    (3500, 0),
];

/// Linear interpolation on the discharge curve.
pub fn voltage_to_percent(millivolts: u16) -> u8 {
    if millivolts >= VOLTAGE_CURVE[0].0 {
        return 100;
    }
    for pair in VOLTAGE_CURVE.windows(2) {
        let ((v_hi, p_hi), (v_lo, p_lo)) = (pair[0], pair[1]);
        if millivolts >= v_lo {
            let span = f32::from(v_hi - v_lo);
            let part = f32::from(millivolts - v_lo) / span;
            return (f32::from(p_lo) + f32::from(p_hi - p_lo) * part).round() as u8;
        }
    }
    0
}

/// Which function of a battery feature returns the current state.
pub fn battery_function(feature: u16) -> u8 {
    if feature == FEATURE_UNIFIED_BATTERY {
        1
    } else {
        0
    }
}

/// Interpret the payload of a battery feature reply.
pub fn decode_battery(feature: u16, data: &[u8]) -> Option<Battery> {
    if data.len() < 3 {
        return None;
    }
    let done = |percent: u8, charging: bool, full: bool| {
        Some(Battery {
            percent,
            charging,
            full: full && percent >= 98,
        })
    };
    match feature {
        FEATURE_UNIFIED_BATTERY => {
            let (mut soc, level, status) = (data[0], data[1], data[2]);
            if soc == 0 {
                // Devices without a fuel gauge only report a coarse level bit.
                soc = match level {
                    l if l & 8 != 0 => 100,
                    l if l & 4 != 0 => 60,
                    l if l & 2 != 0 => 20,
                    l if l & 1 != 0 => 5,
                    _ => 0,
                };
            }
            done(soc.min(100), matches!(status, 1 | 2), status == 3)
        }
        FEATURE_BATTERY_STATUS => {
            done(data[0].min(100), matches!(data[2], 1 | 2 | 4), data[2] == 3)
        }
        FEATURE_CENTURION_SOC => done(data[0].min(100), matches!(data[2], 1 | 2), data[2] == 3),
        FEATURE_ADC_MEASUREMENT => {
            let value = u16::from_be_bytes([data[0], data[1]]);
            let percent = if value <= 100 {
                value as u8
            } else {
                voltage_to_percent(value)
            };
            let charging = data[2] & 0x02 != 0;
            done(percent, charging, charging)
        }
        FEATURE_BATTERY_VOLTAGE => {
            let percent = voltage_to_percent(u16::from_be_bytes([data[0], data[1]]));
            let external_power = data[2] & 0x80 != 0;
            let state = data[2] & 0x03;
            done(
                percent,
                external_power && state == 0,
                external_power && state == 1,
            )
        }
        _ => None,
    }
}

/// HID++ 1.0 register `0x0D` (battery mileage): `[0]` percent, `[2]` high nibble is the state.
pub fn decode_rap_mileage(data: &[u8]) -> Option<Battery> {
    let percent = *data.first().filter(|p| **p <= 100)?;
    let state = data.get(2)? & 0xF0;
    Some(Battery {
        percent,
        charging: state == 0x50,
        full: state == 0x90 && percent >= 98,
    })
}

/// HID++ 1.0 register `0x07` (battery status): `[0]` coarse level, `[1]` charging state.
pub fn decode_rap_status(data: &[u8]) -> Option<Battery> {
    let (level, state) = (*data.first()?, *data.get(1)?);
    let percent = match level {
        7.. => 90,
        5..=6 => 60,
        3..=4 => 30,
        1..=2 => 10,
        0 => return None,
    };
    Some(Battery {
        percent,
        charging: state & 0x21 == 0x21,
        full: false,
    })
}

/// Device kind from the HID++ 2.0 device type (feature `0x0005`, function 2).
pub fn kind_from_device_type(code: u8) -> Kind {
    match code {
        0 | 2 => Kind::Keyboard,
        3..=5 => Kind::Mouse,
        7 | 12 => Kind::Gamepad,
        8 | 9 | 11 => Kind::Headset,
        10 | 14 => Kind::Speaker,
        _ => Kind::Generic,
    }
}

/// Device kind from the HID++ 1.0 pairing information (low nibble of byte 7).
pub fn kind_from_pairing_info(code: u8) -> Kind {
    match code & 0x0F {
        1 => Kind::Keyboard,
        2 | 8 | 9 => Kind::Mouse,
        _ => Kind::Generic,
    }
}

/// What kind of HID++ endpoint a product id is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endpoint {
    /// USB receiver with up to six paired devices on indexes 1–6.
    Receiver,
    /// The device itself (cable or Bluetooth), addressed with index `0xFF`.
    Direct,
    /// Wireless headset dongle; the headset answers on index 1, `0xFF` or 2.
    Headset,
    /// Not in any table: probe index `0xFF` to find out.
    Unknown,
}

const BOLT_RECEIVER: u16 = 0xC548;
const UNIFYING_RECEIVERS: [u16; 8] = [
    0xC517, 0xC518, 0xC51A, 0xC51B, 0xC525, 0xC526, 0xC52B, 0xC532,
];
const NANO_RECEIVERS: [u16; 7] = [0xC52E, 0xC52F, 0xC531, 0xC534, 0xC537, 0xC538, 0xC544];

const HEADSET_NAMES: [(u16, &str); 14] = [
    (0x0A5B, "Logitech G933"),
    (0x0A5C, "Logitech G933"),
    (0x0A66, "Logitech G533"),
    (0x0A87, "Logitech G935"),
    (0x0AA8, "Logitech G535"),
    (0x0AC4, "Logitech G535"),
    (0x0AAA, "Logitech PRO X Wireless"),
    (0x0AAB, "Logitech PRO X Wireless"),
    (0x0AB5, "Logitech G733"),
    (0x0AB6, "Logitech G733"),
    (0x0AB8, "Logitech G735"),
    (0x0ABA, "Logitech G435"),
    (0x0ABB, "Logitech G435"),
    (0x0AC0, "Logitech PRO X 2"),
];

pub fn is_headset_pid(pid: u16) -> bool {
    (0x0A00..=0x0BFF).contains(&pid)
}

pub fn headset_name(pid: u16) -> &'static str {
    HEADSET_NAMES
        .iter()
        .find(|(p, _)| *p == pid)
        .map(|(_, n)| *n)
        .unwrap_or("Logitech Headset")
}

pub fn classify_pid(pid: u16) -> Endpoint {
    if is_headset_pid(pid) {
        Endpoint::Headset
    } else if (0xC500..=0xC5FF).contains(&pid) {
        Endpoint::Receiver
    } else if (0x4000..=0x4FFF).contains(&pid)
        || (0xC200..=0xC2FF).contains(&pid)
        || (0xB000..=0xB3FF).contains(&pid)
    {
        Endpoint::Direct
    } else {
        Endpoint::Unknown
    }
}

/// Connection label for a device behind the endpoint with this product id.
pub fn link_for(pid: u16, direct: bool, bluetooth: bool) -> Link {
    if bluetooth {
        Link::Bluetooth
    } else if is_headset_pid(pid) {
        Link::Lightspeed
    } else if direct {
        Link::Usb
    } else if pid == BOLT_RECEIVER {
        Link::Bolt
    } else if UNIFYING_RECEIVERS.contains(&pid) {
        Link::Unifying
    } else if NANO_RECEIVERS.contains(&pid) {
        Link::Rf24
    } else {
        Link::Lightspeed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_layout() {
        let p = long_request(1, 0, 1, &[0, 0, 0x5A]);
        assert_eq!(&p[..7], &[0x11, 0x01, 0x00, 0x1B, 0x00, 0x00, 0x5A]);
        assert_eq!(p.len(), 20);
        let p = long_request(1, 0, 0, &0x1004u16.to_be_bytes());
        assert_eq!(&p[..6], &[0x11, 0x01, 0x00, 0x0B, 0x10, 0x04]);
        assert_eq!(
            rap_request(2, 0x81, 0x0D, &[]),
            [0x10, 0x02, 0x81, 0x0D, 0, 0, 0]
        );
    }

    #[test]
    fn reply_matching() {
        let ok = [0x11, 1, 5, 0x1B, 80, 8, 0, 0];
        assert_eq!(match_reply(&ok, 1, 5, 1), Reply::Data(vec![80, 8, 0, 0]));
        assert_eq!(match_reply(&ok, 2, 5, 1), Reply::Unrelated);
        assert_eq!(
            match_reply(&ok, 1, 5, 0),
            Reply::Unrelated,
            "other software id or function"
        );
        assert_eq!(
            match_reply(&[0x11, 1, 0xFF, 5, 0x1B, 4, 0], 1, 5, 1),
            Reply::Error(4)
        );
        assert_eq!(
            match_reply(&[0x10, 1, 0x8F, 0, 0x1B, 1, 0], 1, 0, 1),
            Reply::Error(1)
        );
        assert_eq!(
            match_reply(&[0x10, 1, 0x41, 0, 0, 0, 0], 1, 0, 1),
            Reply::Unrelated
        );
        assert_eq!(
            match_rap_reply(&[0x10, 1, 0x81, 0x0D, 55, 0, 0x30], 1, 0x81, 0x0D),
            Reply::Data(vec![55, 0, 0x30])
        );
        assert_eq!(
            match_rap_reply(&[0x10, 1, 0x8F, 0x81, 0x0D, 2, 0], 1, 0x81, 0x0D),
            Reply::Error(2)
        );
    }

    #[test]
    fn voltage_curve() {
        assert_eq!(voltage_to_percent(4300), 100);
        assert_eq!(voltage_to_percent(4186), 100);
        assert_eq!(voltage_to_percent(3811), 50);
        assert_eq!(voltage_to_percent(3835), 55);
        assert_eq!(voltage_to_percent(3500), 0);
        assert_eq!(voltage_to_percent(3000), 0);
    }

    #[test]
    fn unified_battery() {
        let b = decode_battery(FEATURE_UNIFIED_BATTERY, &[73, 4, 0]).unwrap();
        assert_eq!(
            b,
            Battery {
                percent: 73,
                charging: false,
                full: false
            }
        );
        let b = decode_battery(FEATURE_UNIFIED_BATTERY, &[0, 2, 1]).unwrap();
        assert_eq!(
            b,
            Battery {
                percent: 20,
                charging: true,
                full: false
            }
        );
        let b = decode_battery(FEATURE_UNIFIED_BATTERY, &[100, 8, 3]).unwrap();
        assert!(b.full && !b.charging);
        let b = decode_battery(FEATURE_UNIFIED_BATTERY, &[90, 8, 3]).unwrap();
        assert!(!b.full, "'full' below 98 % is not trusted");
        assert!(decode_battery(FEATURE_UNIFIED_BATTERY, &[1, 2]).is_none());
    }

    #[test]
    fn other_battery_features() {
        let b = decode_battery(FEATURE_BATTERY_STATUS, &[45, 30, 4]).unwrap();
        assert_eq!((b.percent, b.charging), (45, true));
        let b = decode_battery(FEATURE_BATTERY_VOLTAGE, &[0x0F, 0x58, 0x80]).unwrap();
        assert_eq!((b.percent, b.charging), (voltage_to_percent(3928), true));
        let b = decode_battery(FEATURE_BATTERY_VOLTAGE, &[0x10, 0x68, 0x81]).unwrap();
        assert!(b.full && !b.charging);
        let b = decode_battery(FEATURE_ADC_MEASUREMENT, &[0x00, 0x40, 0x00]).unwrap();
        assert_eq!(b.percent, 64);
        let b = decode_battery(FEATURE_ADC_MEASUREMENT, &[0x0E, 0xE3, 0x02]).unwrap();
        assert_eq!((b.percent, b.charging), (50, true));
    }

    #[test]
    fn hidpp1_registers() {
        assert_eq!(
            decode_rap_mileage(&[55, 0, 0x30]),
            Some(Battery {
                percent: 55,
                charging: false,
                full: false
            })
        );
        assert!(decode_rap_mileage(&[80, 0, 0x50]).unwrap().charging);
        assert!(decode_rap_mileage(&[100, 0, 0x90]).unwrap().full);
        assert!(decode_rap_mileage(&[200, 0, 0]).is_none());
        assert_eq!(decode_rap_status(&[7, 0]).unwrap().percent, 90);
        assert!(decode_rap_status(&[3, 0x25]).unwrap().charging);
        assert!(decode_rap_status(&[0, 0]).is_none());
    }

    #[test]
    fn endpoints_and_links() {
        assert_eq!(classify_pid(0xC548), Endpoint::Receiver);
        assert_eq!(classify_pid(0x0AB5), Endpoint::Headset);
        assert_eq!(classify_pid(0xC08B), Endpoint::Unknown);
        assert_eq!(classify_pid(0xB023), Endpoint::Direct);
        assert_eq!(link_for(0xC548, false, false), Link::Bolt);
        assert_eq!(link_for(0xC52B, false, false), Link::Unifying);
        assert_eq!(link_for(0xC534, false, false), Link::Rf24);
        assert_eq!(link_for(0xC539, false, false), Link::Lightspeed);
        assert_eq!(link_for(0xC08B, true, false), Link::Usb);
        assert_eq!(link_for(0xB023, true, true), Link::Bluetooth);
        assert_eq!(headset_name(0x0AB5), "Logitech G733");
    }
}
