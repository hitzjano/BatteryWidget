//! JBL speaker battery over Bluetooth LE (protocol in [`crate::jbl_ble`]).
//!
//! Kept deliberately cheap: nothing happens unless such a speaker is paired and
//! connected, the speaker is asked at most every few minutes, the LE link is closed
//! right after the answer, and a short advertisement scan only runs while its LE
//! address is unknown (JBL speakers rotate it).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::{Interface, Result, GUID, HSTRING};
use windows::Devices::Bluetooth::Advertisement::{
    BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementType,
    BluetoothLEAdvertisementWatcher, BluetoothLEScanningMode,
};
use windows::Devices::Bluetooth::GenericAttributeProfile::{
    GattCharacteristic, GattClientCharacteristicConfigurationDescriptorValue,
    GattCommunicationStatus, GattDeviceService, GattSession, GattValueChangedEventArgs,
    GattWriteOption,
};
use windows::Devices::Bluetooth::{
    BluetoothAddressType, BluetoothCacheMode, BluetoothDevice, BluetoothLEDevice,
};
use windows::Devices::Enumeration::DeviceInformation;
use windows::Foundation::TypedEventHandler;
use windows::Storage::Streams::{DataReader, DataWriter, IBuffer};
use windows_collections::IIterable;

use crate::jbl_ble::{
    model_for_advertisement, model_for_name, parse_battery, Model, INFO_REQUEST, NOTIFY_UUID,
    SERVICE_UUID, WRITE_UUID,
};
use crate::model::{Kind, Link, Reading};

/// A speaker battery changes slowly.
const REFRESH: Duration = Duration::from_secs(300);
/// After a failed attempt (speaker out of range, LE link refused, …); doubled with every
/// further failure up to [`MAX_RETRY`], so a speaker that never answers costs nearly nothing.
const RETRY: Duration = Duration::from_secs(600);
const MAX_RETRY: Duration = Duration::from_secs(3600);
/// A level older than this is no longer shown.
const KEEP: Duration = Duration::from_secs(900);
const SCAN_TIME: Duration = Duration::from_secs(6);
const REPLY_TIME: Duration = Duration::from_secs(3);
/// How often the paired device list is checked for a newly connected speaker.
const PAIRED_CHECK: Duration = Duration::from_secs(60);

/// LE address and its type (public or random), as advertised.
#[derive(Clone, Copy)]
struct Address {
    value: u64,
    kind: BluetoothAddressType,
}

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let b = self.value.to_be_bytes();
        let kind = if self.kind == BluetoothAddressType::Random {
            "random"
        } else {
            "public"
        };
        write!(
            f,
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} ({kind})",
            b[2], b[3], b[4], b[5], b[6], b[7]
        )
    }
}

struct Speaker {
    /// Name of the paired (classic) device, used to merge with the Bluetooth entry.
    paired_name: String,
    address: Option<Address>,
    reading: Option<(Instant, Reading)>,
    next: Instant,
    failures: u32,
}

#[derive(Default)]
pub struct JblSpeakers {
    speakers: HashMap<u16, Speaker>,
    next_check: Option<Instant>,
}

impl JblSpeakers {
    /// Current readings; queries the speakers that are due.
    pub fn read(&mut self) -> Vec<Reading> {
        let now = Instant::now();
        let due = self.next_check.is_none_or(|at| at <= now)
            || self.speakers.values().any(|s| s.next <= now);
        if due {
            self.next_check = Some(now + PAIRED_CHECK);
            self.refresh(now);
        }
        self.speakers
            .values()
            .filter_map(|s| s.reading.as_ref())
            .filter(|(at, _)| at.elapsed() < KEEP)
            .map(|(_, reading)| reading.clone())
            .collect()
    }

    fn refresh(&mut self, now: Instant) {
        let connected = connected_speakers().unwrap_or_else(|error| {
            log::debug!("JBL speakers: paired device query failed: {error}");
            Vec::new()
        });
        self.speakers
            .retain(|key, _| connected.iter().any(|(m, _)| m.pids[0] == *key));
        // One speaker per round keeps the worker responsive with several speakers.
        let mut asked = false;
        for (model, paired_name) in connected {
            let speaker = self
                .speakers
                .entry(model.pids[0])
                .or_insert_with(|| Speaker {
                    paired_name: paired_name.clone(),
                    address: None,
                    reading: None,
                    next: now,
                    failures: 0,
                });
            if speaker.next > now || asked {
                continue;
            }
            asked = true;
            match query(model, speaker) {
                Some((percent, charging)) => {
                    let id = format!("jbl-ble:{:04x}", model.pids[0]);
                    let reading = Reading::new(id, model.name, Kind::Speaker, Link::Bluetooth)
                        .battery(Some(percent))
                        .charging(charging && percent < 100)
                        .full(charging && percent >= 100)
                        .aliases(&[speaker.paired_name.as_str()]);
                    speaker.reading = Some((Instant::now(), reading));
                    speaker.next = Instant::now() + REFRESH;
                    speaker.failures = 0;
                }
                None => {
                    // The address may have rotated; look for it again next time.
                    speaker.address = None;
                    let wait = RETRY
                        .saturating_mul(1 << speaker.failures.min(3))
                        .min(MAX_RETRY);
                    speaker.failures += 1;
                    speaker.next = Instant::now() + wait;
                }
            }
        }
    }
}

/// Paired supported JBL speakers that are connected to this PC, with their Windows name.
fn connected_speakers() -> Result<Vec<(&'static Model, String)>> {
    const CONNECTED: &str = "System.Devices.Aep.IsConnected";
    let selector = BluetoothDevice::GetDeviceSelectorFromPairingState(true)?;
    let wanted = IIterable::<HSTRING>::from(vec![HSTRING::from(CONNECTED)]);
    let found =
        DeviceInformation::FindAllAsyncAqsFilterAndAdditionalProperties(&selector, &wanted)?
            .join()?;
    let mut speakers = Vec::new();
    for i in 0..found.Size()? {
        let device = found.GetAt(i)?;
        let name = device.Name()?.to_string();
        let Some(model) = model_for_name(&name) else {
            continue;
        };
        let connected = device
            .Properties()?
            .Lookup(&HSTRING::from(CONNECTED))
            .ok()
            .and_then(|v| v.cast::<windows::Foundation::IPropertyValue>().ok())
            .and_then(|v| v.GetBoolean().ok())
            .unwrap_or(false);
        if connected
            && !speakers
                .iter()
                .any(|(m, _): &(&Model, String)| std::ptr::eq(*m, model))
        {
            speakers.push((model, name));
        }
    }
    Ok(speakers)
}

fn query(model: &'static Model, speaker: &mut Speaker) -> Option<(u8, bool)> {
    let address = match speaker.address {
        Some(address) => address,
        None => {
            let found = scan_for(model).inspect_err(|e| log::debug!("LE scan failed: {e}"));
            let Some(address) = found.ok().flatten() else {
                log::debug!("{}: no LE advertisement seen", model.name);
                return None;
            };
            speaker.address = Some(address);
            address
        }
    };
    match ask_battery(address) {
        Ok(Some(level)) => Some(level),
        Ok(None) => {
            log::debug!("{}: no battery in the reply", model.name);
            None
        }
        Err(error) => {
            log::debug!("{}: LE query failed: {error}", model.name);
            None
        }
    }
}

/// Listen to advertisements until a connectable one of `model` shows up; returns its LE
/// address. A non-connectable one (some speakers also send beacons) is only a fallback.
fn scan_for(model: &'static Model) -> Result<Option<Address>> {
    let found: Arc<Mutex<Option<(Address, bool)>>> = Arc::default();
    let watcher = BluetoothLEAdvertisementWatcher::new()?;
    // Active: the manufacturer data may only be in the scan response.
    watcher.SetScanningMode(BluetoothLEScanningMode::Active)?;
    let sink = found.clone();
    let token = watcher.Received(&TypedEventHandler::<
        BluetoothLEAdvertisementWatcher,
        BluetoothLEAdvertisementReceivedEventArgs,
    >::new(move |_, args| {
        let Some(args) = args.as_ref() else {
            return Ok(());
        };
        let advertisement = args.Advertisement()?;
        let entries = advertisement.ManufacturerData()?;
        for i in 0..entries.Size()? {
            let entry = entries.GetAt(i)?;
            let data = bytes(&entry.Data()?)?;
            if model_for_advertisement(entry.CompanyId()?, &data)
                .is_some_and(|m| std::ptr::eq(m, model))
            {
                let kind = args.AdvertisementType()?;
                let connectable = kind == BluetoothLEAdvertisementType::ConnectableUndirected
                    || kind == BluetoothLEAdvertisementType::ConnectableDirected;
                let address = Address {
                    value: args.BluetoothAddress()?,
                    kind: args
                        .BluetoothAddressType()
                        .unwrap_or(BluetoothAddressType::Unspecified),
                };
                log::debug!("JBL advertisement from {address}, type {kind:?}");
                let mut slot = sink.lock().unwrap_or_else(|p| p.into_inner());
                if !slot.is_some_and(|(_, c)| c) {
                    *slot = Some((address, connectable));
                }
            }
        }
        Ok(())
    }))?;
    watcher.Start()?;
    let deadline = Instant::now() + SCAN_TIME;
    let address = loop {
        let address = *found.lock().unwrap_or_else(|p| p.into_inner());
        if address.is_some_and(|(_, connectable)| connectable) || Instant::now() >= deadline {
            break address.map(|(a, _)| a);
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let _ = watcher.Stop();
    let _ = watcher.RemoveReceived(token);
    Ok(address)
}

/// Connect, ask for the speaker info, collect the notification burst, disconnect.
fn ask_battery(address: Address) -> Result<Option<(u8, bool)>> {
    let device = if address.kind == BluetoothAddressType::Unspecified {
        BluetoothLEDevice::FromBluetoothAddressAsync(address.value)?.join()?
    } else {
        BluetoothLEDevice::FromBluetoothAddressWithBluetoothAddressTypeAsync(
            address.value,
            address.kind,
        )?
        .join()?
    };
    // Without this Windows may drop the idle LE link before the notifications arrive.
    let session = GattSession::FromDeviceIdAsync(&device.BluetoothDeviceId()?)
        .and_then(|op| op.join())
        .inspect_err(|e| log::debug!("GATT session failed: {e}"))
        .ok();
    if let Some(session) = &session {
        let _ = session.SetMaintainConnection(true);
    }
    let result = (|| {
        let Some(service) = control_service(&device)? else {
            log::debug!(
                "JBL: control service not found at {address}; services: {}",
                service_list(&device)
            );
            return Ok(None);
        };
        let outcome = (|| {
            let (Some(rx), Some(tx)) = (
                characteristic(&service, NOTIFY_UUID)?,
                characteristic(&service, WRITE_UUID)?,
            ) else {
                log::debug!("JBL: control characteristics not found");
                return Ok(None);
            };
            exchange(&rx, &tx)
        })();
        let _ = service.Close();
        outcome
    })();
    // Closing the device object drops the LE link once nothing else uses it.
    if let Some(session) = session {
        let _ = session.SetMaintainConnection(false);
        let _ = session.Close();
    }
    let _ = device.Close();
    result
}

/// The control service. Asked for by its UUID right after connecting, Windows can answer
/// with an empty list (seen on a Charge 5 paired as a classic device) while the complete
/// list, read next, contains it; so the complete list is the fallback.
fn control_service(device: &BluetoothLEDevice) -> Result<Option<GattDeviceService>> {
    let wanted = GUID::from_u128(SERVICE_UUID);
    let by_uuid = device
        .GetGattServicesForUuidWithCacheModeAsync(wanted, BluetoothCacheMode::Uncached)?
        .join()?;
    if by_uuid.Status()? == GattCommunicationStatus::Success && by_uuid.Services()?.Size()? > 0 {
        return Ok(Some(by_uuid.Services()?.GetAt(0)?));
    }
    for mode in [BluetoothCacheMode::Uncached, BluetoothCacheMode::Cached] {
        let all = device.GetGattServicesWithCacheModeAsync(mode)?.join()?;
        if all.Status()? != GattCommunicationStatus::Success {
            continue;
        }
        let services = all.Services()?;
        for i in 0..services.Size()? {
            let service = services.GetAt(i)?;
            if service.Uuid()? == wanted {
                return Ok(Some(service));
            }
            let _ = service.Close();
        }
    }
    Ok(None)
}

/// Every service the device offers, for the debug log.
fn service_list(device: &BluetoothLEDevice) -> String {
    let list = || -> Result<String> {
        let all = device
            .GetGattServicesWithCacheModeAsync(BluetoothCacheMode::Uncached)?
            .join()?;
        let services = all.Services()?;
        let mut uuids = Vec::new();
        for i in 0..services.Size()? {
            let uuid = services.GetAt(i)?.Uuid()?;
            uuids.push(format!("{:032x}", uuid.to_u128()));
        }
        Ok(format!("{:?} [{}]", all.Status()?, uuids.join(", ")))
    };
    list().unwrap_or_else(|e| format!("query failed: {e}"))
}

/// A characteristic of the control service; like [`control_service`], the complete list
/// is the fallback when the lookup by UUID comes back empty.
fn characteristic(service: &GattDeviceService, uuid: u128) -> Result<Option<GattCharacteristic>> {
    let wanted = GUID::from_u128(uuid);
    let by_uuid = service
        .GetCharacteristicsForUuidWithCacheModeAsync(wanted, BluetoothCacheMode::Uncached)?
        .join()?;
    if by_uuid.Status()? == GattCommunicationStatus::Success
        && by_uuid.Characteristics()?.Size()? > 0
    {
        return Ok(Some(by_uuid.Characteristics()?.GetAt(0)?));
    }
    for mode in [BluetoothCacheMode::Uncached, BluetoothCacheMode::Cached] {
        let all = service.GetCharacteristicsWithCacheModeAsync(mode)?.join()?;
        if all.Status()? != GattCommunicationStatus::Success {
            continue;
        }
        let list = all.Characteristics()?;
        for i in 0..list.Size()? {
            let characteristic = list.GetAt(i)?;
            if characteristic.Uuid()? == wanted {
                return Ok(Some(characteristic));
            }
        }
    }
    Ok(None)
}

fn exchange(rx: &GattCharacteristic, tx: &GattCharacteristic) -> Result<Option<(u8, bool)>> {
    let received: Arc<Mutex<Vec<u8>>> = Arc::default();
    let sink = received.clone();
    let token = rx.ValueChanged(&TypedEventHandler::<
        GattCharacteristic,
        GattValueChangedEventArgs,
    >::new(move |_, args| {
        if let Some(args) = args.as_ref() {
            let chunk = bytes(&args.CharacteristicValue()?)?;
            sink.lock().unwrap_or_else(|p| p.into_inner()).extend(chunk);
        }
        Ok(())
    }))?;
    let outcome = (|| {
        let status = rx
            .WriteClientCharacteristicConfigurationDescriptorAsync(
                GattClientCharacteristicConfigurationDescriptorValue::Notify,
            )?
            .join()?;
        if status != GattCommunicationStatus::Success {
            log::debug!("JBL: enabling notifications failed: {status:?}");
            return Ok(None);
        }
        let writer = DataWriter::new()?;
        writer.WriteBytes(&INFO_REQUEST)?;
        let status = tx
            .WriteValueWithOptionAsync(&writer.DetachBuffer()?, GattWriteOption::WriteWithResponse)?
            .join()?;
        if status != GattCommunicationStatus::Success {
            log::debug!("JBL: writing the request failed: {status:?}");
            return Ok(None);
        }
        let deadline = Instant::now() + REPLY_TIME;
        loop {
            let stream = received.lock().unwrap_or_else(|p| p.into_inner()).clone();
            let level = parse_battery(&stream);
            if level.is_some() || Instant::now() >= deadline {
                if level.is_none() {
                    log::debug!("JBL reply: {} bytes: {}", stream.len(), hex(&stream));
                }
                return Ok(level);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    })();
    let _ = rx.RemoveValueChanged(token);
    if let Ok(op) = rx.WriteClientCharacteristicConfigurationDescriptorAsync(
        GattClientCharacteristicConfigurationDescriptorValue::None,
    ) {
        let _ = op.join();
    }
    outcome
}

/// The first bytes as hex, for the debug log.
fn hex(data: &[u8]) -> String {
    let shown: Vec<String> = data.iter().take(96).map(|b| format!("{b:02X}")).collect();
    let more = if data.len() > 96 { " …" } else { "" };
    format!("{}{more}", shown.join(" "))
}

fn bytes(buffer: &IBuffer) -> Result<Vec<u8>> {
    let mut data = vec![0u8; buffer.Length()? as usize];
    if !data.is_empty() {
        DataReader::FromBuffer(buffer)?.ReadBytes(&mut data)?;
    }
    Ok(data)
}
