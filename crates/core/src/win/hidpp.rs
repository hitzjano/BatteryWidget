//! HID++ over Windows HID: channel implementation and the worker loop.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::hid::{enumerate, wait_any, HidDevice};
use crate::hid::HidInfo;
use crate::hidpp::protocol::*;
use crate::hidpp::scanner::{Channel, ChannelError, ChannelResult, Scanner};
use crate::model::Reading;
use crate::wake::Wake;

const SCAN_INTERVAL: Duration = Duration::from_secs(60);
const SYNC_INTERVAL: Duration = Duration::from_secs(10);
/// A receiver that cannot be opened (held exclusively by another program) is left alone
/// for this long.
const DEAD_RETRY: Duration = Duration::from_secs(600);
/// After a receiver notification, give the device a moment before asking it anything.
const SETTLE: Duration = Duration::from_millis(800);
const MAX_UNSOLICITED: usize = 32;
/// Vendor usage pages Logitech uses for HID++ (USB, Bluetooth LE, Centurion headsets).
const USAGE_PAGES: [u16; 4] = [0xFF00, 0xFF43, 0xFFA0, 0xFF01];

/// The short and long HID++ collections of one physical interface.
struct Interface {
    pid: u16,
    bluetooth: bool,
    short: Option<HidInfo>,
    long: Option<HidInfo>,
}

/// Both collections of an interface share everything in the path except `&colNN`.
fn interface_key(path: &str) -> String {
    let lower = path.to_lowercase();
    let parts: Vec<&str> = lower.split('#').collect();
    match parts.as_slice() {
        [_, hardware, instance, ..] => {
            let hardware = hardware.split("&col").next().unwrap_or(hardware);
            let instance = instance
                .rsplit_once('&')
                .map_or(*instance, |(head, _)| head);
            format!("{hardware}|{instance}")
        }
        _ => lower,
    }
}

/// HID++ interfaces currently present, keyed by the path of their long collection.
fn find_interfaces() -> HashMap<String, Interface> {
    let mut found: HashMap<String, Interface> = HashMap::new();
    for info in enumerate(|vid| vid == LOGITECH_VID) {
        if !USAGE_PAGES.contains(&info.usage_page) {
            continue;
        }
        let entry = found.entry(interface_key(&info.path)).or_insert(Interface {
            pid: info.pid,
            bluetooth: info.bluetooth,
            short: None,
            long: None,
        });
        // Usage 2 (0x0202 over Bluetooth LE) is the long HID++ collection. Other large
        // collections (e.g. the DJ collection of Unifying receivers) are only a fallback.
        let is_long = matches!(info.usage, 2 | 0x0202);
        if info.usage == 1 && info.output_len == SHORT_LEN {
            entry.short = Some(info);
        } else if is_long || (info.output_len >= LONG_LEN && entry.long.is_none()) {
            entry.long = Some(info);
        }
    }
    found
        .into_values()
        .filter_map(|interface| Some((interface.long.as_ref()?.path.clone(), interface)))
        .collect()
}

struct WinChannel {
    pid: u16,
    bluetooth: bool,
    long: HidDevice,
    short: Option<HidDevice>,
    /// Reports that arrived while waiting for a reply and were not that reply.
    unsolicited: Vec<Vec<u8>>,
}

impl WinChannel {
    fn open(interface: &Interface) -> std::io::Result<WinChannel> {
        let long = interface
            .long
            .as_ref()
            .ok_or_else(|| std::io::Error::other("no long collection"))?;
        Ok(WinChannel {
            pid: interface.pid,
            bluetooth: interface.bluetooth,
            long: HidDevice::open(long)?,
            // The short collection is optional: Bluetooth devices only have the long one.
            short: interface
                .short
                .as_ref()
                .and_then(|info| HidDevice::open(info).ok()),
            unsolicited: Vec::new(),
        })
    }

    fn devices(&mut self) -> Vec<&mut HidDevice> {
        std::iter::once(&mut self.long)
            .chain(self.short.as_mut())
            .collect()
    }

    fn arm(&mut self) -> Result<(), ChannelError> {
        for device in self.devices() {
            device.arm().map_err(|e| ChannelError::Io(e.to_string()))?;
        }
        Ok(())
    }

    /// Wait for the report that `classify` accepts; everything else is kept for later.
    fn wait_reply(
        &mut self,
        timeout: Duration,
        classify: impl Fn(&[u8]) -> Reply,
    ) -> ChannelResult {
        let deadline = Instant::now() + timeout;
        loop {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Ok(None);
            };
            let report = match wait_any(&mut self.devices(), remaining) {
                Ok(Some((_, report))) => report,
                Ok(None) => return Ok(None),
                Err(_) => return Err(ChannelError::Io("read failed".into())),
            };
            match classify(&report) {
                Reply::Data(data) => return Ok(Some(data)),
                Reply::Error(code) => return Err(ChannelError::Protocol(code)),
                Reply::Unrelated if self.unsolicited.len() < MAX_UNSOLICITED => {
                    self.unsolicited.push(report)
                }
                Reply::Unrelated => {}
            }
        }
    }
}

impl Channel for WinChannel {
    fn product_id(&self) -> u16 {
        self.pid
    }

    fn is_bluetooth(&self) -> bool {
        self.bluetooth
    }

    fn request(
        &mut self,
        device: u8,
        feature_index: u8,
        function: u8,
        params: &[u8],
        timeout: Duration,
    ) -> ChannelResult {
        self.arm()?;
        self.long
            .write(&long_request(device, feature_index, function, params))
            .map_err(|e| ChannelError::Io(e.to_string()))?;
        self.wait_reply(timeout, |report| {
            match_reply(report, device, feature_index, function)
        })
    }

    fn request_rap(
        &mut self,
        device: u8,
        sub_id: u8,
        register: u8,
        params: &[u8],
        timeout: Duration,
    ) -> ChannelResult {
        self.arm()?;
        let packet = rap_request(device, sub_id, register, params);
        let Some(short) = self.short.as_mut() else {
            // Register access needs the short report; without it the device is HID++ 2.0 only.
            return Ok(None);
        };
        if short.write(&packet).is_err() {
            return Ok(None);
        }
        self.wait_reply(timeout, |report| {
            match_rap_reply(report, device, sub_id, register)
        })
    }
}

/// Open newly attached endpoints, close removed ones. Returns `true` if one was added.
fn sync(scanner: &mut Scanner<WinChannel>, dead: &mut HashMap<String, Instant>) -> bool {
    let interfaces = find_interfaces();
    for path in scanner.paths() {
        if !interfaces.contains_key(&path) {
            scanner.remove_channel(&path);
        }
    }
    let now = Instant::now();
    dead.retain(|path, until| *until > now && interfaces.contains_key(path));
    let mut added = false;
    for (path, interface) in interfaces {
        if scanner.has_channel(&path) || dead.contains_key(&path) {
            continue;
        }
        match WinChannel::open(&interface) {
            Ok(channel) => {
                log::info!("HID++ endpoint {:04x} opened", interface.pid);
                scanner.add_channel(path, channel);
                added = true;
            }
            Err(error) => {
                log::info!("HID++ endpoint {:04x} unavailable: {error}", interface.pid);
                dead.insert(path, now + DEAD_RETRY);
            }
        }
    }
    added
}

/// Wait up to `timeout` for a receiver notification. Returns `true` if a rescan is due.
fn wait_for_events(scanner: &mut Scanner<WinChannel>, wake: &Wake, timeout: Duration) -> bool {
    let mut backlog = Vec::new();
    for (path, channel) in scanner.channels_mut() {
        backlog.extend(
            channel
                .unsolicited
                .drain(..)
                .map(|report| (path.clone(), report)),
        );
    }
    let mut relevant = false;
    for (path, report) in backlog {
        relevant |= scanner.is_relevant_notification(&path, &report);
    }
    if relevant {
        return true;
    }

    let mut owners = Vec::new();
    let mut devices = Vec::new();
    for (path, channel) in scanner.channels_mut() {
        for device in channel.devices() {
            owners.push(path.clone());
            devices.push(device);
        }
    }
    if devices.is_empty() {
        return wake.sleep(timeout);
    }
    let outcome = wait_any(&mut devices, timeout);
    drop(devices);
    match outcome {
        Ok(Some((index, report))) => scanner.is_relevant_notification(&owners[index], &report),
        Ok(None) => false,
        Err(index) => {
            scanner.remove_channel(&owners[index]);
            true
        }
    }
}

/// Worker loop: scan at start, every minute, on request and when a receiver reports a change.
pub fn run(wake: Arc<Wake>, emit: impl Fn(Vec<Reading>)) {
    let mut scanner = Scanner::<WinChannel>::default();
    let mut dead = HashMap::new();
    let mut next_scan = Instant::now();
    let mut next_sync = Instant::now();
    let mut forced = true;
    while !wake.is_stopped() {
        forced |= wake.take_refresh();
        let now = Instant::now();
        if forced || now >= next_scan {
            if forced || now >= next_sync {
                sync(&mut scanner, &mut dead);
                next_sync = now + SYNC_INTERVAL;
            }
            forced = false;
            emit(scanner.scan());
            next_scan = Instant::now() + SCAN_INTERVAL;
            continue;
        }
        if wait_for_events(&mut scanner, &wake, Duration::from_secs(1)) {
            if !scanner.paths().is_empty() {
                wake.sleep(SETTLE);
            }
            forced = true;
        } else if now >= next_sync {
            forced = sync(&mut scanner, &mut dead);
            next_sync = now + SYNC_INTERVAL;
        }
    }
}
