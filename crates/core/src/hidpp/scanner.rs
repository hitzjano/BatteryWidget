//! Walks HID++ receivers and devices and turns them into [`Reading`]s.
//!
//! The scanner talks to hardware only through [`Channel`], so the whole discovery logic
//! runs against a scripted fake in the unit tests.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use super::protocol::*;
use crate::model::{Kind, Reading};
use crate::names::kind_from_name;

const REQUEST_TIMEOUT: Duration = Duration::from_millis(1000);
const PING_TIMEOUT: Duration = Duration::from_millis(1500);

#[derive(Debug)]
pub enum ChannelError {
    /// The receiver or device answered with a HID++ error code.
    Protocol(u8),
    /// The underlying HID handle failed; the endpoint is most likely unplugged.
    Io(String),
}

/// `Ok(None)` means "no answer in time".
pub type ChannelResult = Result<Option<Vec<u8>>, ChannelError>;

/// One opened HID++ endpoint (receiver, dongle or directly attached device).
pub trait Channel {
    fn product_id(&self) -> u16;
    fn is_bluetooth(&self) -> bool;
    /// HID++ 2.0 request; returns the payload after the 4-byte header.
    fn request(
        &mut self,
        device: u8,
        feature_index: u8,
        function: u8,
        params: &[u8],
        timeout: Duration,
    ) -> ChannelResult;
    /// HID++ 1.0 register access; returns the payload after the 4-byte header.
    fn request_rap(
        &mut self,
        device: u8,
        sub_id: u8,
        register: u8,
        params: &[u8],
        timeout: Duration,
    ) -> ChannelResult;
}

/// What we remember about one device index of one endpoint.
struct Slot {
    /// `(feature id, feature index)` of the battery feature, once found.
    feature: Option<(u16, u8)>,
    name: String,
    kind: Kind,
    unit_id: String,
    last_battery: Option<u8>,
    identified: bool,
    hidpp1: bool,
}

/// Outcome of a ping.
enum Presence {
    /// Answers; `true` when it only speaks HID++ 1.0.
    Online {
        hidpp1: bool,
    },
    Offline,
}

pub struct Scanner<C: Channel> {
    channels: BTreeMap<String, C>,
    endpoints: HashMap<String, Endpoint>,
    slots: HashMap<(String, u8), Slot>,
}

impl<C: Channel> Default for Scanner<C> {
    fn default() -> Self {
        Scanner {
            channels: BTreeMap::new(),
            endpoints: HashMap::new(),
            slots: HashMap::new(),
        }
    }
}

impl<C: Channel> Scanner<C> {
    pub fn has_channel(&self, path: &str) -> bool {
        self.channels.contains_key(path)
    }

    pub fn paths(&self) -> Vec<String> {
        self.channels.keys().cloned().collect()
    }

    pub fn channels_mut(&mut self) -> impl Iterator<Item = (&String, &mut C)> {
        self.channels.iter_mut()
    }

    pub fn add_channel(&mut self, path: String, channel: C) {
        self.channels.insert(path, channel);
    }

    pub fn remove_channel(&mut self, path: &str) {
        self.channels.remove(path);
        self.endpoints.remove(path);
        self.slots.retain(|(p, _), _| p != path);
    }

    /// Does an unsolicited report announce a connection or battery change?
    pub fn is_relevant_notification(&mut self, path: &str, report: &[u8]) -> bool {
        if report.len() < 4 || !matches!(report[0], REPORT_SHORT | REPORT_LONG) {
            return false;
        }
        let key = (path.to_string(), report[1]);
        if report[2] == 0x41 {
            // Device connection notification (e.g. the charging cable was plugged in):
            // identify the device again, but keep its last level until it reports a new one.
            if let Some(slot) = self.slots.get_mut(&key) {
                slot.feature = None;
                slot.identified = false;
            }
            return true;
        }
        self.slots
            .get(&key)
            .and_then(|slot| slot.feature)
            .is_some_and(|(_, index)| report[2] == index && report[3] & 0x0F == 0)
    }

    /// Query every reachable device. Endpoints whose handle fails are dropped.
    pub fn scan(&mut self) -> Vec<Reading> {
        let mut results = Vec::new();
        for path in self.paths() {
            if let Err(error) = self.scan_endpoint(&path, &mut results) {
                log::info!("HID++ endpoint gone ({error})");
                self.remove_channel(&path);
            }
        }
        results
    }

    fn scan_endpoint(&mut self, path: &str, out: &mut Vec<Reading>) -> Result<(), String> {
        let endpoint = self.classify(path)?;
        match endpoint {
            Endpoint::Headset => {
                // Depending on the model the headset answers on a different index.
                let mut fallback = None;
                for (index, direct) in [(1, false), (INDEX_SELF, true), (2, false)] {
                    match self.scan_slot(path, index, direct)? {
                        Some(r) if r.connected => {
                            out.push(r);
                            return Ok(());
                        }
                        Some(r) => fallback = fallback.or(Some(r)),
                        None => {}
                    }
                }
                out.extend(fallback);
            }
            Endpoint::Direct => out.extend(self.scan_slot(path, INDEX_SELF, true)?),
            Endpoint::Receiver | Endpoint::Unknown => {
                for index in 1..=6 {
                    out.extend(self.scan_slot(path, index, false)?);
                }
            }
        }
        Ok(())
    }

    fn classify(&mut self, path: &str) -> Result<Endpoint, String> {
        if let Some(known) = self.endpoints.get(path) {
            return Ok(*known);
        }
        let channel = self.channels.get_mut(path).ok_or("channel closed")?;
        let endpoint = match classify_pid(channel.product_id()) {
            Endpoint::Unknown => match ping(channel, INDEX_SELF, Duration::from_millis(1000)) {
                // Anything that answers on 0xFF itself (even "asleep") is a device.
                Ok(Some(_)) | Err(ChannelError::Protocol(ERR_CONNECT_FAIL)) => Endpoint::Direct,
                Ok(None) | Err(ChannelError::Protocol(_)) => Endpoint::Receiver,
                Err(ChannelError::Io(e)) => return Err(e),
            },
            known => known,
        };
        self.endpoints.insert(path.to_string(), endpoint);
        Ok(endpoint)
    }

    fn scan_slot(
        &mut self,
        path: &str,
        index: u8,
        direct: bool,
    ) -> Result<Option<Reading>, String> {
        let channel = self.channels.get_mut(path).ok_or("channel closed")?;
        let pid = channel.product_id();
        let link = link_for(pid, direct, channel.is_bluetooth());
        let key = (path.to_string(), index);
        let slot = self.slots.entry(key.clone()).or_insert_with(|| {
            let headset = is_headset_pid(pid);
            Slot {
                feature: None,
                name: if headset {
                    headset_name(pid).to_string()
                } else {
                    "Logitech".to_string()
                },
                kind: if headset {
                    Kind::Headset
                } else {
                    Kind::Generic
                },
                unit_id: format!("{pid:04X}-{index:02X}"),
                last_battery: None,
                identified: false,
                hidpp1: false,
            }
        });

        let presence = match ping(channel, index, PING_TIMEOUT) {
            Ok(Some(_)) => Presence::Online { hidpp1: false },
            Err(ChannelError::Protocol(ERR_INVALID_SUBID)) => Presence::Online { hidpp1: true },
            Ok(None) | Err(ChannelError::Protocol(_)) => Presence::Offline,
            Err(ChannelError::Io(e)) => return Err(e),
        };

        let mut battery = None;
        if let Presence::Online { hidpp1 } = presence {
            slot.hidpp1 = hidpp1;
            battery = if hidpp1 {
                read_hidpp1(channel, index, slot)?
            } else {
                read_hidpp2(channel, index, slot)?
            };
        }

        let Some(battery) = battery else {
            if !slot.identified && slot.last_battery.is_none() {
                // Empty slot, or a device we never learned anything about.
                self.slots.remove(&key);
                return Ok(None);
            }
            let offline = Reading::new(
                format!("logi:{}", slot.unit_id),
                slot.name.clone(),
                slot.kind,
                link,
            )
            .battery(slot.last_battery)
            .connected(false);
            return Ok(Some(offline));
        };

        // While charging many devices cannot measure. Then the level is shown as unknown
        // (just the charging bolt): an old value would look as if charging did nothing.
        // Without charging, an unknown level keeps the last real one.
        if battery.percent.is_some() {
            slot.last_battery = battery.percent;
        }
        let level = if battery.charging {
            battery.percent
        } else {
            slot.last_battery
        };
        Ok(Some(
            Reading::new(
                format!("logi:{}", slot.unit_id),
                slot.name.clone(),
                slot.kind,
                link,
            )
            .battery(level)
            .charging(battery.charging)
            .full(battery.full),
        ))
    }
}

fn io<T>(result: Result<T, ChannelError>) -> Result<Option<T>, String> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(ChannelError::Protocol(_)) => Ok(None),
        Err(ChannelError::Io(e)) => Err(e),
    }
}

/// Root feature, function 1. Returns the protocol version payload.
fn ping<C: Channel>(channel: &mut C, index: u8, timeout: Duration) -> ChannelResult {
    channel.request(index, FEATURE_ROOT, 1, &[0, 0, 0x5A], timeout)
}

/// Root feature, function 0: index of a feature id, `None` if the device lacks it.
fn feature_index<C: Channel>(
    channel: &mut C,
    device: u8,
    feature: u16,
) -> Result<Option<u8>, String> {
    let reply = io(channel.request(
        device,
        FEATURE_ROOT,
        0,
        &feature.to_be_bytes(),
        REQUEST_TIMEOUT,
    ))?;
    Ok(reply
        .flatten()
        .and_then(|r| r.first().copied())
        .filter(|index| *index != 0))
}

fn call<C: Channel>(
    channel: &mut C,
    device: u8,
    index: u8,
    function: u8,
    params: &[u8],
) -> Result<Option<Vec<u8>>, String> {
    Ok(io(channel.request(device, index, function, params, REQUEST_TIMEOUT))?.flatten())
}

fn read_hidpp2<C: Channel>(
    channel: &mut C,
    device: u8,
    slot: &mut Slot,
) -> Result<Option<Battery>, String> {
    if slot.feature.is_none() {
        for feature in BATTERY_FEATURES {
            if let Some(index) = feature_index(channel, device, feature)? {
                slot.feature = Some((feature, index));
                break;
            }
        }
    }
    if !slot.identified {
        identify_hidpp2(channel, device, slot)?;
    }
    let Some((feature, index)) = slot.feature else {
        return Ok(None);
    };
    let reply = call(channel, device, index, battery_function(feature), &[])?;
    Ok(reply.and_then(|data| decode_battery(feature, &data)))
}

fn identify_hidpp2<C: Channel>(channel: &mut C, device: u8, slot: &mut Slot) -> Result<(), String> {
    let mut named = false;
    if let Some(index) = feature_index(channel, device, FEATURE_NAME)? {
        if let Some(length) = call(channel, device, index, 0, &[])?.and_then(|r| r.first().copied())
        {
            let length = usize::from(length).min(64);
            let mut raw = Vec::with_capacity(length);
            while raw.len() < length {
                match call(channel, device, index, 1, &[raw.len() as u8])? {
                    Some(part) if !part.is_empty() => {
                        let take = (length - raw.len()).min(part.len());
                        raw.extend_from_slice(&part[..take]);
                    }
                    _ => break,
                }
            }
            let name = String::from_utf8_lossy(&raw)
                .trim_matches(|c: char| c == '\0' || c.is_whitespace())
                .to_string();
            if !name.is_empty() {
                slot.name = name;
                named = true;
            }
        }
        if let Some(code) = call(channel, device, index, 2, &[])?.and_then(|r| r.first().copied()) {
            match kind_from_device_type(code) {
                Kind::Generic => {}
                kind => slot.kind = kind,
            }
        }
    }
    if let Some(index) = feature_index(channel, device, FEATURE_FW_INFO)? {
        if let Some(info) = call(channel, device, index, 0, &[])?.filter(|r| r.len() >= 5) {
            let unit = &info[1..5];
            if unit != [0; 4] && unit != [0xFF; 4] {
                slot.unit_id = unit.iter().map(|b| format!("{b:02X}")).collect();
            }
        }
    }
    if slot.kind == Kind::Generic {
        slot.kind = kind_from_name(&slot.name);
    }
    slot.identified = named || slot.feature.is_some();
    Ok(())
}

fn read_hidpp1<C: Channel>(
    channel: &mut C,
    device: u8,
    slot: &mut Slot,
) -> Result<Option<Battery>, String> {
    let register = |channel: &mut C, target: u8, sub_id: u8, reg: u8, params: &[u8]| {
        io(channel.request_rap(target, sub_id, reg, params, REQUEST_TIMEOUT)).map(Option::flatten)
    };
    if !slot.identified {
        // Older receivers keep name, kind and serial of each paired device themselves.
        let n = device.wrapping_sub(1) & 0x0F;
        if let Some(r) = register(
            channel,
            INDEX_SELF,
            RAP_GET_LONG_REGISTER,
            REG_PAIRING_INFO,
            &[0x40 + n],
        )? {
            let length = usize::from(r.get(1).copied().unwrap_or(0));
            if (1..=16).contains(&length) && r.len() >= 2 + length {
                let name = String::from_utf8_lossy(&r[2..2 + length])
                    .trim()
                    .to_string();
                if !name.is_empty() {
                    slot.name = name;
                }
            }
        }
        if let Some(r) = register(
            channel,
            INDEX_SELF,
            RAP_GET_LONG_REGISTER,
            REG_PAIRING_INFO,
            &[0x20 + n],
        )? {
            if let Some(code) = r.get(7) {
                slot.kind = kind_from_pairing_info(*code);
            }
        }
        if let Some(r) = register(
            channel,
            INDEX_SELF,
            RAP_GET_LONG_REGISTER,
            REG_PAIRING_INFO,
            &[0x30 + n],
        )? {
            if r.len() >= 5 && r[1..5] != [0; 4] {
                slot.unit_id = r[1..5].iter().map(|b| format!("{b:02X}")).collect();
            }
        }
        if slot.kind == Kind::Generic {
            slot.kind = kind_from_name(&slot.name);
        }
        slot.identified = true;
    }
    if let Some(r) = register(channel, device, RAP_GET_REGISTER, REG_BATTERY_MILEAGE, &[])? {
        if let Some(battery) = decode_rap_mileage(&r) {
            return Ok(Some(battery));
        }
    }
    let status = register(channel, device, RAP_GET_REGISTER, REG_BATTERY_STATUS, &[])?;
    Ok(status.and_then(|r| decode_rap_status(&r)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Link;

    /// A scripted receiver: one HID++ 2.0 mouse on slot 1, one HID++ 1.0 keyboard on
    /// slot 2, a sleeping device on slot 3 and empty slots otherwise.
    struct FakeReceiver {
        pid: u16,
        mouse_online: bool,
        /// Payload of the mouse's unified battery reply.
        mouse_battery: Vec<u8>,
        requests: usize,
    }

    const MOUSE_NAME: &[u8] = b"MX Master 3S";

    impl Channel for FakeReceiver {
        fn product_id(&self) -> u16 {
            self.pid
        }
        fn is_bluetooth(&self) -> bool {
            false
        }
        fn request(
            &mut self,
            device: u8,
            feature_index: u8,
            function: u8,
            params: &[u8],
            _: Duration,
        ) -> ChannelResult {
            self.requests += 1;
            match device {
                1 if !self.mouse_online => Err(ChannelError::Protocol(ERR_CONNECT_FAIL)),
                1 => Ok(Some(match (feature_index, function) {
                    (0, 1) => vec![4, 2, 0x5A],
                    (0, 0) => match u16::from_be_bytes([params[0], params[1]]) {
                        FEATURE_UNIFIED_BATTERY => vec![6],
                        FEATURE_NAME => vec![2],
                        FEATURE_FW_INFO => vec![3],
                        _ => vec![0],
                    },
                    (2, 0) => vec![MOUSE_NAME.len() as u8],
                    (2, 1) => MOUSE_NAME[usize::from(params[0])..]
                        .iter()
                        .copied()
                        .take(16)
                        .collect(),
                    (2, 2) => vec![3],
                    (3, 0) => vec![1, 0xDE, 0xAD, 0xBE, 0xEF, 0, 0],
                    (6, 1) => self.mouse_battery.clone(),
                    other => panic!("unexpected request {other:?}"),
                })),
                2 => Err(ChannelError::Protocol(ERR_INVALID_SUBID)),
                3 => Err(ChannelError::Protocol(ERR_CONNECT_FAIL)),
                0xFF => Ok(None),
                _ => Err(ChannelError::Protocol(8)),
            }
        }
        fn request_rap(
            &mut self,
            device: u8,
            sub_id: u8,
            register: u8,
            params: &[u8],
            _: Duration,
        ) -> ChannelResult {
            match (device, sub_id, register, params.first().copied()) {
                (0xFF, RAP_GET_LONG_REGISTER, REG_PAIRING_INFO, Some(0x41)) => {
                    Ok(Some([&[0x41, 4][..], b"K800"].concat()))
                }
                (0xFF, RAP_GET_LONG_REGISTER, REG_PAIRING_INFO, Some(0x21)) => {
                    Ok(Some(vec![0x21, 0, 0, 0, 0, 0, 0, 0x01]))
                }
                (0xFF, RAP_GET_LONG_REGISTER, REG_PAIRING_INFO, Some(0x31)) => {
                    Ok(Some(vec![0x31, 0x11, 0x22, 0x33, 0x44]))
                }
                (2, RAP_GET_REGISTER, REG_BATTERY_MILEAGE, _) => Ok(Some(vec![42, 0, 0x30])),
                _ => Err(ChannelError::Protocol(2)),
            }
        }
    }

    fn scanner(pid: u16) -> Scanner<FakeReceiver> {
        let mut scanner = Scanner::default();
        scanner.add_channel(
            "path".into(),
            FakeReceiver {
                pid,
                mouse_online: true,
                mouse_battery: vec![85, 8, 1],
                requests: 0,
            },
        );
        scanner
    }

    #[test]
    fn receiver_with_hidpp2_and_hidpp1_devices() {
        let mut scanner = scanner(0xC548);
        let found = scanner.scan();
        assert_eq!(found.len(), 2, "{found:?}");

        let mouse = &found[0];
        assert_eq!(mouse.id, "logi:DEADBEEF");
        assert_eq!(mouse.name, "MX Master 3S");
        assert_eq!(mouse.kind, Kind::Mouse);
        assert_eq!(mouse.battery, Some(85));
        assert!(mouse.charging && mouse.connected);
        assert_eq!(mouse.link, Link::Bolt);

        let keyboard = &found[1];
        assert_eq!(keyboard.id, "logi:11223344");
        assert_eq!(keyboard.name, "K800");
        assert_eq!(keyboard.kind, Kind::Keyboard);
        assert_eq!(keyboard.battery, Some(42));
    }

    #[test]
    fn second_scan_only_pings_and_reads_battery() {
        let mut scanner = scanner(0xC548);
        scanner.scan();
        let before = scanner.channels["path"].requests;
        scanner.scan();
        let per_scan = scanner.channels["path"].requests - before;
        assert_eq!(
            per_scan,
            6 + 1,
            "six pings plus one battery read, everything else is cached"
        );
    }

    #[test]
    fn sleeping_device_keeps_last_level_as_disconnected() {
        let mut scanner = scanner(0xC548);
        scanner.scan();
        scanner.channels.get_mut("path").unwrap().mouse_online = false;
        let found = scanner.scan();
        let mouse = found
            .iter()
            .find(|r| r.id == "logi:DEADBEEF")
            .expect("still listed");
        assert!(!mouse.connected);
        assert_eq!(mouse.battery, Some(85));
    }

    #[test]
    fn unknown_pid_is_probed_and_treated_as_receiver() {
        let mut scanner = scanner(0xC099);
        assert_eq!(scanner.scan().len(), 2);
        assert_eq!(scanner.endpoints["path"], Endpoint::Receiver);
    }

    #[test]
    fn notifications() {
        let mut scanner = scanner(0xC548);
        scanner.scan();
        assert!(
            scanner.is_relevant_notification("path", &[0x11, 1, 6, 0x00, 80, 8, 0]),
            "battery broadcast"
        );
        assert!(
            !scanner.is_relevant_notification("path", &[0x11, 1, 6, 0x1B, 80, 8, 0]),
            "reply to a request"
        );
        assert!(!scanner.is_relevant_notification("path", &[0x11, 1, 9, 0x00, 0, 0, 0]));
        assert!(
            scanner.is_relevant_notification("path", &[0x10, 1, 0x41, 0, 0, 0, 0]),
            "connection change"
        );
        let slot = &scanner.slots[&("path".to_string(), 1)];
        assert!(
            !slot.identified && slot.feature.is_none(),
            "slot is identified again"
        );
        assert_eq!(slot.last_battery, Some(85), "but keeps its last level");
    }

    #[test]
    fn unknown_level_while_charging_is_shown_as_unknown() {
        let mut scanner = scanner(0xC548);
        scanner.scan();
        // Charging cable plugged in: the device reconnects and cannot measure for a while.
        assert!(scanner.is_relevant_notification("path", &[0x10, 1, 0x41, 0, 0, 0, 0]));
        scanner.channels.get_mut("path").unwrap().mouse_battery = vec![0, 0, 1];
        let found = scanner.scan();
        let mouse = found.iter().find(|r| r.id == "logi:DEADBEEF").unwrap();
        assert_eq!(mouse.battery, None, "neither 0 % nor the stale level");
        assert!(mouse.charging && mouse.connected);
        // Charging stopped but the level is still unknown: fall back to the last one.
        scanner.channels.get_mut("path").unwrap().mouse_battery = vec![0, 0, 0];
        let found = scanner.scan();
        let mouse = found.iter().find(|r| r.id == "logi:DEADBEEF").unwrap();
        assert_eq!((mouse.battery, mouse.charging), (Some(85), false));
    }

    #[test]
    fn device_asleep_after_reconnect_notification_stays_listed() {
        let mut scanner = scanner(0xC548);
        scanner.scan();
        scanner.is_relevant_notification("path", &[0x10, 1, 0x41, 0x40, 0, 0, 0]);
        scanner.channels.get_mut("path").unwrap().mouse_online = false;
        let found = scanner.scan();
        let mouse = found.iter().find(|r| r.id == "logi:DEADBEEF").unwrap();
        assert!(!mouse.connected);
        assert_eq!(mouse.battery, Some(85));
    }
}
