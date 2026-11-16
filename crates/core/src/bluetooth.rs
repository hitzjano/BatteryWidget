//! Bluetooth battery levels as Windows already knows them.
//!
//! The Bluetooth stack reads the GATT Battery Service (LE) or the HFP indicators (Classic)
//! itself and publishes the result as a PnP device property. This module holds the pure
//! part: matching those battery nodes to paired devices. The WinRT queries live in
//! `win::bluetooth`.

use std::collections::HashMap;

use crate::model::{Kind, Link, Reading};
use crate::names::{clean_pnp_name, kind_from_name};

/// PnP property that carries the battery percentage (`DEVPKEY_Bluetooth_Battery`-style key).
pub const BATTERY_PROPERTY: &str = "{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2";

/// Last group of the Bluetooth base UUID. It is 12 hex digits long, just like an address.
const BASE_UUID_TAIL: &str = "00805f9b34fb";

/// A paired Bluetooth device (association endpoint), grouped by container.
#[derive(Clone, Debug, Default)]
pub struct Endpoint {
    /// Container id (lower-case GUID without braces) or any other stable key.
    pub container: String,
    pub name: String,
    /// 12 lower-case hex digits, empty when unknown.
    pub address: String,
    pub connected: bool,
    pub kind: Option<Kind>,
}

/// A PnP device node that reports a battery level.
#[derive(Clone, Debug, Default)]
pub struct BatteryNode {
    pub instance_id: String,
    pub name: String,
    pub level: u8,
    pub present: bool,
    pub container: Option<String>,
}

/// Bluetooth addresses embedded in a device instance id.
pub fn addresses_in(instance_id: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut run = String::new();
    for c in instance_id.chars().chain(std::iter::once('\0')) {
        if c.is_ascii_hexdigit() {
            run.push(c.to_ascii_lowercase());
            if run.len() == 12 {
                if run != BASE_UUID_TAIL {
                    found.push(std::mem::take(&mut run));
                }
                run.clear();
            }
        } else {
            run.clear();
        }
    }
    found
}

/// Device kind from the Class of Device (Classic) or the GAP appearance (LE).
pub fn kind_from_class(
    major: Option<u32>,
    minor: Option<u32>,
    appearance: Option<u32>,
    name: &str,
) -> Kind {
    let minor = minor.unwrap_or(0);
    match major {
        Some(1) => {
            return match minor {
                6 => Kind::Watch,
                7 => Kind::Tablet,
                _ => Kind::Computer,
            }
        }
        Some(2) => return Kind::Phone,
        Some(4) => {
            return match minor {
                5 | 7 | 10 => Kind::Speaker,
                4 => Kind::Microphone,
                9 | 14 | 15 => Kind::Tv,
                12 | 13 => Kind::Camera,
                _ => Kind::Headset,
            }
        }
        Some(5) => match (minor & 0x0F, minor & 0x30) {
            (1 | 2, _) => return Kind::Gamepad,
            (3, _) => return Kind::Remote,
            (5 | 7, _) => return Kind::Pen,
            (_, 0x20) => return Kind::Mouse,
            (_, 0x10 | 0x30) => return Kind::Keyboard,
            _ => {}
        },
        Some(6) if minor & 0x08 != 0 => return Kind::Camera,
        Some(6) if minor & 0x04 != 0 => return Kind::Tv,
        Some(7) if minor == 1 => return Kind::Watch,
        _ => {}
    }
    if let Some(appearance) = appearance.filter(|a| *a != 0) {
        let (category, sub) = (appearance >> 6, appearance & 0x3F);
        match (category, sub) {
            (1, _) => return Kind::Phone,
            (2, 7) => return Kind::Tablet,
            (2, _) => return Kind::Computer,
            (3, _) => return Kind::Watch,
            (5, _) | (40, _) => return Kind::Tv,
            (6, _) => return Kind::Remote,
            (15, 1) => return Kind::Keyboard,
            (15, 2) => return Kind::Mouse,
            (15, 3 | 4) => return Kind::Gamepad,
            (15, 5 | 7) => return Kind::Pen,
            (33, _) => return Kind::Speaker,
            (34, 1) => return Kind::Microphone,
            (37, _) => return Kind::Headset,
            _ => {}
        }
    }
    kind_from_name(name)
}

/// Combine paired devices and battery nodes into readings.
///
/// A battery node belongs to an endpoint with the same container id, or failing that with
/// the same Bluetooth address. Nodes that are present are preferred over stale ones.
/// Present battery nodes that are not Bluetooth at all (HID devices on a 2.4 GHz dongle
/// that implement the HID battery usage) are reported as separate `pnp:` devices.
pub fn assemble(endpoints: &[Endpoint], nodes: &[BatteryNode], bt_enabled: bool) -> Vec<Reading> {
    let mut by_container: HashMap<&str, (u8, bool)> = HashMap::new();
    let mut by_address: HashMap<String, (u8, bool)> = HashMap::new();
    for node in nodes {
        let entry = (node.level.min(100), node.present);
        let better =
            |old: Option<&(u8, bool)>| old.is_none_or(|(_, present)| node.present && !present);
        if let Some(container) = node.container.as_deref() {
            if better(by_container.get(container)) {
                by_container.insert(container, entry);
            }
        }
        for address in addresses_in(&node.instance_id) {
            if better(by_address.get(&address)) {
                by_address.insert(address, entry);
            }
        }
    }

    let mut results: Vec<Reading> = Vec::new();
    for ep in endpoints {
        let level = by_container
            .get(ep.container.as_str())
            .or_else(|| {
                by_address
                    .get(&ep.address)
                    .filter(|_| !ep.address.is_empty())
            })
            .map(|(level, _)| *level);
        let key = if ep.address.is_empty() {
            &ep.container
        } else {
            &ep.address
        };
        let kind = ep
            .kind
            .filter(|k| *k != Kind::Generic)
            .unwrap_or_else(|| kind_from_name(&ep.name));
        results.push(
            Reading::new(format!("bt:{key}"), ep.name.clone(), kind, Link::Bluetooth)
                .battery(level)
                .connected(ep.connected && bt_enabled),
        );
    }

    for node in nodes.iter().filter(|n| n.present) {
        let id = node.instance_id.to_lowercase();
        if id.contains("bth") || id.contains("bluetooth") || id.contains(BASE_UUID_TAIL) {
            continue;
        }
        let container = node.container.as_deref();
        if container.is_some_and(|c| endpoints.iter().any(|ep| ep.container == c)) {
            continue;
        }
        let addresses = addresses_in(&node.instance_id);
        if endpoints
            .iter()
            .any(|ep| !ep.address.is_empty() && addresses.contains(&ep.address))
        {
            continue;
        }
        let name = clean_pnp_name(&node.name);
        if name.chars().count() < 2 {
            continue;
        }
        let key = container
            .map(str::to_string)
            .or_else(|| addresses.first().cloned())
            .unwrap_or(id);
        let dev_id = format!("pnp:{key}");
        if results.iter().any(|r| r.id == dev_id) {
            continue;
        }
        let kind = kind_from_name(&name);
        results.push(Reading::new(dev_id, name, kind, Link::Rf24).battery(Some(node.level)));
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(container: &str, name: &str, address: &str, connected: bool) -> Endpoint {
        Endpoint {
            container: container.into(),
            name: name.into(),
            address: address.into(),
            connected,
            kind: None,
        }
    }

    fn node(
        id: &str,
        name: &str,
        level: u8,
        present: bool,
        container: Option<&str>,
    ) -> BatteryNode {
        BatteryNode {
            instance_id: id.into(),
            name: name.into(),
            level,
            present,
            container: container.map(Into::into),
        }
    }

    #[test]
    fn addresses() {
        let id = r"BTHENUM\{0000111E-0000-1000-8000-00805F9B34FB}_VID&0001004C\7&1A2B3C&0&AABBCCDDEEFF_C00000000";
        assert_eq!(addresses_in(id), ["aabbccddeeff"]);
        assert!(addresses_in(r"HID\VID_046D&PID_C52B\6&1234").is_empty());
    }

    #[test]
    fn class_of_device() {
        assert_eq!(kind_from_class(Some(4), Some(1), None, ""), Kind::Headset);
        assert_eq!(kind_from_class(Some(4), Some(5), None, ""), Kind::Speaker);
        assert_eq!(kind_from_class(Some(5), Some(0x20), None, ""), Kind::Mouse);
        assert_eq!(
            kind_from_class(Some(5), Some(0x10), None, ""),
            Kind::Keyboard
        );
        assert_eq!(
            kind_from_class(Some(5), Some(0x02), None, ""),
            Kind::Gamepad
        );
        assert_eq!(kind_from_class(Some(2), None, None, ""), Kind::Phone);
        assert_eq!(kind_from_class(None, None, Some(962), ""), Kind::Mouse);
        assert_eq!(kind_from_class(None, None, Some(961), ""), Kind::Keyboard);
        assert_eq!(
            kind_from_class(None, None, None, "Xbox Wireless Controller"),
            Kind::Gamepad
        );
    }

    #[test]
    fn battery_by_container_and_by_address() {
        let endpoints = [
            endpoint("c1", "WH-1000XM4", "aabbccddeeff", true),
            endpoint("c2", "MX Anywhere 3", "112233445566", true),
            endpoint("c3", "Old Keyboard", "", false),
        ];
        let nodes = [
            node(
                r"BTHENUM\{..}\7&0&AABBCCDDEEFF_C0",
                "WH-1000XM4 Hands-Free AG",
                70,
                true,
                Some("c1"),
            ),
            node(
                r"BTHLE\DEV_112233445566\8&1",
                "MX Anywhere 3",
                55,
                true,
                None,
            ),
        ];
        let out = assemble(&endpoints, &nodes, true);
        assert_eq!(out.len(), 3);
        assert_eq!(
            (out[0].id.as_str(), out[0].battery, out[0].kind),
            ("bt:aabbccddeeff", Some(70), Kind::Headset)
        );
        assert_eq!((out[1].battery, out[1].kind), (Some(55), Kind::Mouse));
        assert_eq!(
            (out[2].id.as_str(), out[2].battery, out[2].connected),
            ("bt:c3", None, false)
        );
    }

    #[test]
    fn present_node_beats_stale_node() {
        let endpoints = [endpoint("c1", "Buds", "aabbccddeeff", true)];
        let nodes = [
            node("BTHENUM\\a", "Buds", 20, false, Some("c1")),
            node("BTHENUM\\b", "Buds", 90, true, Some("c1")),
            node("BTHENUM\\c", "Buds", 30, false, Some("c1")),
        ];
        assert_eq!(assemble(&endpoints, &nodes, true)[0].battery, Some(90));
    }

    #[test]
    fn radio_off_marks_everything_disconnected() {
        let endpoints = [endpoint("c1", "Buds", "aabbccddeeff", true)];
        assert!(!assemble(&endpoints, &[], false)[0].connected);
    }

    #[test]
    fn non_bluetooth_battery_node_becomes_pnp_device() {
        let nodes = [
            node(
                r"HID\VID_1234&PID_5678\9&1",
                "Fancy Keyboard",
                64,
                true,
                Some("c9"),
            ),
            node(
                r"HID\VID_1234&PID_5678\9&2",
                "Fancy Keyboard",
                64,
                true,
                Some("c9"),
            ),
            node(
                r"HID\VID_1234&PID_0001\1",
                "Unplugged",
                10,
                false,
                Some("c8"),
            ),
        ];
        let out = assemble(&[], &nodes, true);
        assert_eq!(out.len(), 1);
        assert_eq!(
            (out[0].id.as_str(), out[0].link, out[0].battery),
            ("pnp:c9", Link::Rf24, Some(64))
        );
    }
}
