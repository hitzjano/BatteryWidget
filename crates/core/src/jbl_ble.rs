//! JBL portable speakers (Charge, Flip, Xtreme, Pulse, Boombox, Go, Clip, Tuner) over
//! their Bluetooth LE control service.
//!
//! Windows does not show a battery level for these speakers (they have no hands-free
//! profile and no standard battery service). The speaker answers the "speaker info"
//! query `AA 11 00` on its own GATT service with a burst of `AA 12 <len> 00 <token> …`
//! notifications; token `0x44` is the battery: low 7 bits percent, bit 7 charging.
//! Layout as documented by the jbl-charge5 and JBLControl projects (both MIT).

/// Control service and its characteristics.
pub const SERVICE_UUID: u128 = 0x65786365_6c70_6f69_6e74_2e636f6d0000;
pub const NOTIFY_UUID: u128 = 0x65786365_6c70_6f69_6e74_2e636f6d0001;
pub const WRITE_UUID: u128 = 0x65786365_6c70_6f69_6e74_2e636f6d0002;

/// Bluetooth SIG company id Harman uses in its advertisements.
pub const COMPANY_HARMAN: u16 = 0x0057;

/// "Speaker info" request. The length byte is required by some firmware.
pub const INFO_REQUEST: [u8; 3] = [0xAA, 0x11, 0x00];
const INFO_REPLY: u8 = 0x12;
const TOKEN_BATTERY: u8 = 0x44;

pub struct Model {
    pub name: &'static str,
    /// Product ids in the advertised manufacturer data (some models had several).
    pub pids: &'static [u16],
}

const fn model(name: &'static str, pids: &'static [u16]) -> Model {
    Model { name, pids }
}

/// Every speaker that the JBL Portable app reaches over Bluetooth LE with the `AA` frame
/// protocol (model data as extracted by JBLControl). Left out: Wi-Fi models and the
/// classic-Bluetooth-only Flip 3 / Xtreme (no LE link), and the newest generation
/// (Boombox 4, Xtreme 5, Go 5, Grip, Essential 3 SE), whose `DD 00` protocol has no
/// documented battery query.
pub const MODELS: [Model; 20] = [
    // Verified on hardware by jbl-charge5 / JBLControl.
    model("JBL Charge 5", &[0x2040]),
    model("JBL Charge 6", &[0x20E3]),
    model("JBL Flip 7", &[0x2107]),
    // Same protocol family, not yet confirmed on hardware.
    model("JBL Flip 6", &[0x204F]),
    model("JBL Xtreme 3", &[0x202F]),
    model("JBL Pulse 5", &[0x2050]),
    model("JBL Boombox 2", &[0x1F53]),
    model("JBL Boombox 3", &[0x206D]),
    model("JBL Xtreme 4", &[0x20DC]),
    // Go 4 Duo: left and right speaker of a pair.
    model("JBL Go 4", &[0x20E4, 0x210A, 0x215C, 0x2181]),
    model("JBL Clip 5", &[0x20F5]),
    model("JBL Tuner 3", &[0x214C]),
    // Older models that only offer battery and device info.
    model("JBL Flip 4", &[0x1ED1]),
    model("JBL Flip 5", &[0x1F31]),
    model("JBL Charge 3", &[0x1EBC]),
    model("JBL Charge 4", &[0x1F17, 0x1F29]),
    model("JBL Pulse 3", &[0x1ED2]),
    model("JBL Pulse 4", &[0x1F56]),
    model("JBL Xtreme 2", &[0x1EFC, 0x1F26, 0x2038]),
    model("JBL Boombox", &[0x1EE7, 0x1F27]),
];

impl Model {
    /// Lower-case name without the brand: `"charge 5"`.
    fn key(&self) -> String {
        self.name.trim_start_matches("JBL ").to_lowercase()
    }
}

/// The model a paired Bluetooth device name belongs to. The longest model name that
/// starts the device name wins, and only if no further model number follows, so that
/// "JBL Boombox 4" is not taken for the original Boombox.
pub fn model_for_name(name: &str) -> Option<&'static Model> {
    let lower = name.trim().to_lowercase();
    let rest = lower.strip_prefix("jbl")?.trim_start();
    if rest.contains("wi-fi") || rest.contains("wifi") {
        return None;
    }
    MODELS
        .iter()
        .filter(|m| {
            rest.strip_prefix(m.key().as_str()).is_some_and(|tail| {
                tail.is_empty()
                    || (tail.starts_with([' ', '-', '_', '('])
                        && !tail
                            .trim_start_matches([' ', '-', '_'])
                            .starts_with(|c: char| c.is_ascii_digit()))
            })
        })
        .max_by_key(|m| m.key().len())
}

/// The model announced by an advertisement's manufacturer data. `data` excludes the
/// company id (Windows hands it over separately); the product id follows little endian.
pub fn model_for_advertisement(company: u16, data: &[u8]) -> Option<&'static Model> {
    if company != COMPANY_HARMAN || data.len() < 2 {
        return None;
    }
    let pid = u16::from_le_bytes([data[0], data[1]]);
    MODELS.iter().find(|m| m.pids.contains(&pid))
}

/// Battery from the concatenated notifications: `Some((percent, charging))`.
pub fn parse_battery(stream: &[u8]) -> Option<(u8, bool)> {
    let mut i = 0;
    while i + 3 <= stream.len() {
        if stream[i] != 0xAA {
            i += 1;
            continue;
        }
        let (kind, len) = (stream[i + 1], usize::from(stream[i + 2]));
        let Some(payload) = stream.get(i + 3..i + 3 + len) else {
            break;
        };
        if kind == INFO_REPLY {
            if let Some(value) = battery_token(payload) {
                return decode(value);
            }
        }
        i += 3 + len;
    }
    None
}

/// `00 44 <value>`, or with an explicit value length `00 44 01 <value>`; any further
/// bytes after the value are ignored (as JBLControl does).
fn battery_token(payload: &[u8]) -> Option<u8> {
    match payload {
        [_, TOKEN_BATTERY, 1, value] => Some(*value),
        [_, TOKEN_BATTERY, value, ..] => Some(*value),
        _ => None,
    }
}

fn decode(value: u8) -> Option<(u8, bool)> {
    let percent = value & 0x7F;
    (percent <= 100).then_some((percent, value & 0x80 != 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(name: &str) -> Option<u16> {
        model_for_name(name).map(|m| m.pids[0])
    }

    #[test]
    fn names() {
        assert_eq!(pid("JBL Charge 5"), Some(0x2040));
        assert_eq!(pid("jbl charge 6"), Some(0x20E3));
        assert_eq!(pid("JBL Flip 6"), Some(0x204F));
        assert_eq!(pid("JBL Flip 5"), Some(0x1F31));
        assert_eq!(pid("JBL Xtreme 3"), Some(0x202F));
        assert_eq!(pid("JBL GO 4"), Some(0x20E4));
        assert_eq!(pid("JBL GO 4 DUO"), Some(0x20E4), "Duo is a Go 4 pair");
        assert_eq!(pid("JBL Boombox 3"), Some(0x206D), "longest name wins");
        assert_eq!(pid("JBL Boombox"), Some(0x1EE7));
        assert_eq!(pid("JBL Pulse 4 (2)"), Some(0x1F56));
    }

    #[test]
    fn names_left_out() {
        assert_eq!(pid("JBL Boombox 4"), None, "DD 00 generation");
        assert_eq!(pid("JBL Xtreme 5"), None);
        assert_eq!(pid("JBL Xtreme"), None, "classic Bluetooth only");
        assert_eq!(pid("JBL Flip 3"), None);
        assert_eq!(pid("JBL Flip Essential 3 SE"), None);
        assert_eq!(pid("JBL Charge 5 Wi-Fi"), None, "no LE link");
        assert_eq!(pid("JBL Charge 50"), None);
        assert_eq!(pid("JBL Quantum 910"), None, "headset");
        assert_eq!(pid("Charge 5 Cable"), None, "not a JBL");
    }

    #[test]
    fn advertisements() {
        // Captured Charge 5 manufacturer data `57 00 40 20 01 A8 1E FB 04 4E`.
        let data = [0x40, 0x20, 0x01, 0xA8, 0x1E, 0xFB, 0x04, 0x4E];
        let name = |company, data: &[u8]| model_for_advertisement(company, data).map(|m| m.name);
        assert_eq!(name(0x0057, &data), Some("JBL Charge 5"));
        assert_eq!(name(0x0057, &[0xE3, 0x20]), Some("JBL Charge 6"));
        assert_eq!(
            name(0x0057, &[0x81, 0x21]),
            Some("JBL Go 4"),
            "Go 4 Duo (R)"
        );
        assert_eq!(name(0x0057, &[0x4E, 0x21]), None, "Boombox 4");
        assert_eq!(name(0x004C, &data), None, "other company");
        assert_eq!(name(0x0057, &[0x40]), None);
    }

    #[test]
    fn pids_are_unique() {
        let mut all: Vec<u16> = MODELS.iter().flat_map(|m| m.pids.iter().copied()).collect();
        let count = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), count);
    }

    #[test]
    fn battery_capture() {
        // Captured: battery 100 %, and 20 % while charging.
        assert_eq!(
            parse_battery(&[0xAA, 0x12, 0x03, 0x00, 0x44, 0x64]),
            Some((100, false))
        );
        assert_eq!(
            parse_battery(&[0xAA, 0x12, 0x03, 0x00, 0x44, 0x94]),
            Some((20, true))
        );
    }

    #[test]
    fn battery_in_a_burst() {
        let burst = [
            0xAA, 0x12, 0x04, 0x00, 0x42, 0x20, 0x40, // product id
            0xAA, 0x12, 0x03, 0x00, 0x44, 0x8A, // battery: charging, 10 %
            0xAA, 0x12, 0x03, 0x00, 0x46, 0x01,
        ];
        assert_eq!(parse_battery(&burst), Some((10, true)));
        assert_eq!(
            parse_battery(&[0xAA, 0x12, 0x04, 0x00, 0x44, 0x01, 0x32]),
            Some((50, false))
        );
        assert_eq!(
            parse_battery(&[0xAA, 0x12, 0x04, 0x00, 0x44, 0x3C, 0x00]),
            Some((60, false)),
            "trailing byte"
        );
    }

    #[test]
    fn battery_rejects_noise() {
        assert_eq!(parse_battery(&[]), None);
        assert_eq!(
            parse_battery(&[0xAA, 0x12, 0x03, 0x00, 0x44]),
            None,
            "truncated"
        );
        assert_eq!(
            parse_battery(&[0xAA, 0x13, 0x03, 0x00, 0x44, 0x50]),
            None,
            "other reply"
        );
        assert_eq!(
            parse_battery(&[0xAA, 0x12, 0x03, 0x00, 0x44, 0x7F]),
            None,
            "127 %"
        );
        assert_eq!(
            parse_battery(&[0x01, 0x02, 0xAA, 0x12, 0x03, 0x00, 0x44, 0x3C]),
            Some((60, false)),
            "leading garbage"
        );
    }
}
