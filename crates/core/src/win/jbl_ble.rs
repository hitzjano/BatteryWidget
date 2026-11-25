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
    BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher,
    BluetoothLEScanningMode,
};
use windows::Devices::Bluetooth::GenericAttributeProfile::{
    GattCharacteristic, GattClientCharacteristicConfigurationDescriptorValue,
    GattCommunicationStatus, GattValueChangedEventArgs, GattWriteOption,
};
use windows::Devices::Bluetooth::{BluetoothCacheMode, BluetoothDevice, BluetoothLEDevice};
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

struct Speaker {
    /// Name of the paired (classic) device, used to merge with the Bluetooth entry.
    paired_name: String,
    address: Option<u64>,
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

/// Listen to advertisements until one of `model` shows up; returns its LE address.
fn scan_for(model: &'static Model) -> Result<Option<u64>> {
    let found: Arc<Mutex<Option<u64>>> = Arc::default();
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
                *sink.lock().unwrap_or_else(|p| p.into_inner()) = Some(args.BluetoothAddress()?);
            }
        }
        Ok(())
    }))?;
    watcher.Start()?;
    let deadline = Instant::now() + SCAN_TIME;
    let address = loop {
        let address = *found.lock().unwrap_or_else(|p| p.into_inner());
        if address.is_some() || Instant::now() >= deadline {
            break address;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let _ = watcher.Stop();
    let _ = watcher.RemoveReceived(token);
    Ok(address)
}

/// Connect, ask for the speaker info, collect the notification burst, disconnect.
fn ask_battery(address: u64) -> Result<Option<(u8, bool)>> {
    let device = BluetoothLEDevice::FromBluetoothAddressAsync(address)?.join()?;
    let result = (|| {
        let services = device
            .GetGattServicesForUuidWithCacheModeAsync(
                GUID::from_u128(SERVICE_UUID),
                BluetoothCacheMode::Uncached,
            )?
            .join()?;
        if services.Status()? != GattCommunicationStatus::Success
            || services.Services()?.Size()? == 0
        {
            return Ok(None);
        }
        let service = services.Services()?.GetAt(0)?;
        let outcome = (|| {
            let (Some(rx), Some(tx)) = (
                characteristic(&service, NOTIFY_UUID)?,
                characteristic(&service, WRITE_UUID)?,
            ) else {
                return Ok(None);
            };
            exchange(&rx, &tx)
        })();
        let _ = service.Close();
        outcome
    })();
    // Closing the device object drops the LE link once nothing else uses it.
    let _ = device.Close();
    result
}

fn characteristic(
    service: &windows::Devices::Bluetooth::GenericAttributeProfile::GattDeviceService,
    uuid: u128,
) -> Result<Option<GattCharacteristic>> {
    let found = service
        .GetCharacteristicsForUuidAsync(GUID::from_u128(uuid))?
        .join()?;
    if found.Status()? != GattCommunicationStatus::Success {
        return Ok(None);
    }
    let all = found.Characteristics()?;
    Ok(if all.Size()? > 0 {
        Some(all.GetAt(0)?)
    } else {
        None
    })
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
            return Ok(None);
        }
        let writer = DataWriter::new()?;
        writer.WriteBytes(&INFO_REQUEST)?;
        let status = tx
            .WriteValueWithOptionAsync(&writer.DetachBuffer()?, GattWriteOption::WriteWithResponse)?
            .join()?;
        if status != GattCommunicationStatus::Success {
            return Ok(None);
        }
        let deadline = Instant::now() + REPLY_TIME;
        loop {
            let level = parse_battery(&received.lock().unwrap_or_else(|p| p.into_inner()));
            if level.is_some() || Instant::now() >= deadline {
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

fn bytes(buffer: &IBuffer) -> Result<Vec<u8>> {
    let mut data = vec![0u8; buffer.Length()? as usize];
    if !data.is_empty() {
        DataReader::FromBuffer(buffer)?.ReadBytes(&mut data)?;
    }
    Ok(data)
}
