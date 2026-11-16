//! Bluetooth via WinRT device enumeration: paired devices, their connection state and
//! the battery level the Windows Bluetooth stack has already collected.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use windows::core::{IInspectable, Interface, Result, HSTRING};
use windows::Devices::Bluetooth::{
    BluetoothAdapter, BluetoothConnectionStatus, BluetoothDevice, BluetoothLEDevice,
};
use windows::Devices::Enumeration::{
    DeviceInformation, DeviceInformationCollection, DeviceInformationKind, DeviceInformationUpdate,
    DeviceWatcher,
};
use windows::Devices::Radios::{Radio, RadioKind, RadioState};
use windows::Foundation::{IPropertyValue, PropertyType, TypedEventHandler};
use windows_collections::{IIterable, IMapView};

use crate::bluetooth::{
    addresses_in, assemble, kind_from_class, BatteryNode, Endpoint, BATTERY_PROPERTY,
};
use crate::model::{Kind, Reading};
use crate::names::kind_from_name;
use crate::wake::Wake;

/// Scan interval when nothing happens. Changes arrive through the watchers; this only
/// covers events Windows fails to raise.
const SAFETY_INTERVAL: Duration = Duration::from_secs(15);
/// Scan interval when the watchers could not be started.
const FALLBACK_INTERVAL: Duration = Duration::from_secs(5);
/// Windows raises events in bursts; wait this long so that one scan covers them all.
const COALESCE: Duration = Duration::from_millis(250);
/// After the radio comes back, devices reconnect one by one.
const FOLLOW_UP_SCANS: u32 = 4;
const FOLLOW_UP_INTERVAL: Duration = Duration::from_millis(800);

const AEP_PROPERTIES: [&str; 6] = [
    "System.Devices.Aep.IsConnected",
    "System.Devices.Aep.DeviceAddress",
    "System.Devices.Aep.ContainerId",
    "System.Devices.Aep.Bluetooth.Cod.Major",
    "System.Devices.Aep.Bluetooth.Cod.Minor",
    "System.Devices.Aep.Bluetooth.Le.Appearance",
];
const NODE_PROPERTIES: [&str; 4] = [
    BATTERY_PROPERTY,
    "System.Devices.ContainerId",
    "System.Devices.Present",
    "System.ItemNameDisplay",
];
/// Device nodes that can carry a battery level.
const NODE_FILTER: &str = "System.Devices.DeviceInstanceId:~<\"BTH\" OR \
                           System.Devices.DeviceInstanceId:~<\"HID\" OR \
                           System.Devices.DeviceInstanceId:~<\"USB\"";

type Properties = IMapView<HSTRING, IInspectable>;

fn property_list(names: &[&str]) -> Result<IIterable<HSTRING>> {
    let names: Vec<HSTRING> = names.iter().map(|n| HSTRING::from(*n)).collect();
    Ok(IIterable::<HSTRING>::from(names))
}

fn property(properties: &Properties, key: &str) -> Option<IPropertyValue> {
    properties.Lookup(&HSTRING::from(key)).ok()?.cast().ok()
}

fn bool_property(properties: &Properties, key: &str) -> Option<bool> {
    property(properties, key)?.GetBoolean().ok()
}

fn number_property(properties: &Properties, key: &str) -> Option<u32> {
    let value = property(properties, key)?;
    match value.Type().ok()? {
        PropertyType::UInt8 => value.GetUInt8().ok().map(u32::from),
        PropertyType::UInt16 => value.GetUInt16().ok().map(u32::from),
        PropertyType::UInt32 => value.GetUInt32().ok(),
        PropertyType::Int16 => value.GetInt16().ok().and_then(|v| u32::try_from(v).ok()),
        PropertyType::Int32 => value.GetInt32().ok().and_then(|v| u32::try_from(v).ok()),
        _ => None,
    }
}

fn string_property(properties: &Properties, key: &str) -> Option<String> {
    let value = property(properties, key)?;
    match value.Type().ok()? {
        PropertyType::String => value.GetString().ok().map(|s| s.to_string()),
        PropertyType::Guid => value.GetGuid().ok().map(|g| format!("{g:?}")),
        _ => None,
    }
}

/// Container ids arrive as GUID values; normalise to lower case without braces.
fn container_property(properties: &Properties, key: &str) -> Option<String> {
    let id = string_property(properties, key)?
        .trim_matches(['{', '}'])
        .to_lowercase();
    (!id.is_empty()).then_some(id)
}

fn items(collection: &DeviceInformationCollection) -> Vec<DeviceInformation> {
    let count = collection.Size().unwrap_or(0);
    (0..count)
        .filter_map(|i| collection.GetAt(i).ok())
        .collect()
}

/// Is the Bluetooth radio switched on? Unknown counts as on.
fn radio_is_on() -> bool {
    let radios = || -> Result<Vec<Radio>> {
        let all = Radio::GetRadiosAsync()?.join()?;
        Ok((0..all.Size()?).filter_map(|i| all.GetAt(i).ok()).collect())
    };
    let bluetooth: Vec<Radio> = radios()
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.Kind().is_ok_and(|k| k == RadioKind::Bluetooth))
        .collect();
    bluetooth.is_empty()
        || bluetooth
            .iter()
            .any(|r| r.State().is_ok_and(|s| s == RadioState::On))
}

struct Paired {
    endpoint: Endpoint,
    id: HSTRING,
    low_energy: bool,
    /// `None` when Windows did not say whether the endpoint is connected.
    connected: Option<bool>,
}

fn paired_selectors() -> Result<[(HSTRING, bool); 2]> {
    Ok([
        (
            BluetoothDevice::GetDeviceSelectorFromPairingState(true)?,
            false,
        ),
        (
            BluetoothLEDevice::GetDeviceSelectorFromPairingState(true)?,
            true,
        ),
    ])
}

/// Paired devices, grouped by container (a dual-mode device has two endpoints).
fn paired_devices() -> Result<Vec<Paired>> {
    let wanted = property_list(&AEP_PROPERTIES)?;
    let mut paired: Vec<Paired> = Vec::new();
    for (selector, low_energy) in paired_selectors()? {
        let found =
            DeviceInformation::FindAllAsyncAqsFilterAndAdditionalProperties(&selector, &wanted)?
                .join()?;
        for device in items(&found) {
            let (Ok(id), Ok(name), Ok(properties)) =
                (device.Id(), device.Name(), device.Properties())
            else {
                continue;
            };
            let name = name.to_string();
            let address = string_property(&properties, "System.Devices.Aep.DeviceAddress")
                .map(|a| a.replace(':', "").to_lowercase())
                .or_else(|| addresses_in(&id.to_string()).pop())
                .unwrap_or_default();
            let container = container_property(&properties, "System.Devices.Aep.ContainerId")
                .unwrap_or_else(|| {
                    if address.is_empty() {
                        id.to_string()
                    } else {
                        address.clone()
                    }
                });
            let connected = bool_property(&properties, "System.Devices.Aep.IsConnected");
            let kind = kind_from_class(
                number_property(&properties, "System.Devices.Aep.Bluetooth.Cod.Major"),
                number_property(&properties, "System.Devices.Aep.Bluetooth.Cod.Minor"),
                number_property(&properties, "System.Devices.Aep.Bluetooth.Le.Appearance"),
                &name,
            );
            if let Some(existing) = paired
                .iter_mut()
                .find(|p| p.endpoint.container == container)
            {
                existing.endpoint.connected |= connected == Some(true);
                if existing.endpoint.address.is_empty() {
                    existing.endpoint.address = address;
                }
                if matches!(existing.endpoint.kind, None | Some(Kind::Generic)) {
                    existing.endpoint.kind = Some(kind);
                }
                continue;
            }
            paired.push(Paired {
                endpoint: Endpoint {
                    container,
                    name,
                    address,
                    connected: connected == Some(true),
                    kind: Some(kind),
                },
                id,
                low_energy,
                connected,
            });
        }
    }
    Ok(paired)
}

/// Ask the device object itself, for endpoints whose connection state Windows left open.
fn is_connected(id: &HSTRING, low_energy: bool) -> Result<bool> {
    let status = if low_energy {
        BluetoothLEDevice::FromIdAsync(id)?
            .join()?
            .ConnectionStatus()?
    } else {
        BluetoothDevice::FromIdAsync(id)?
            .join()?
            .ConnectionStatus()?
    };
    Ok(status == BluetoothConnectionStatus::Connected)
}

/// With the radio off Windows still lists paired devices, just without live properties.
fn paired_devices_radio_off() -> Result<Vec<Endpoint>> {
    let mut endpoints: Vec<Endpoint> = Vec::new();
    for (selector, _) in paired_selectors()? {
        let found = DeviceInformation::FindAllAsyncAqsFilter(&selector)?.join()?;
        for device in items(&found) {
            let (Ok(id), Ok(name)) = (device.Id(), device.Name()) else {
                continue;
            };
            let (id, name) = (id.to_string(), name.to_string());
            let address = addresses_in(&id).pop().unwrap_or_default();
            let container = if address.is_empty() {
                id
            } else {
                address.clone()
            };
            if endpoints.iter().any(|e| e.container == container) {
                continue;
            }
            let kind = Some(kind_from_name(&name));
            endpoints.push(Endpoint {
                container,
                name,
                address,
                connected: false,
                kind,
            });
        }
    }
    Ok(endpoints)
}

fn battery_nodes() -> Result<Vec<BatteryNode>> {
    let found = DeviceInformation::FindAllAsyncWithKindAqsFilterAndAdditionalProperties(
        &HSTRING::from(NODE_FILTER),
        &property_list(&NODE_PROPERTIES)?,
        DeviceInformationKind::Device,
    )?
    .join()?;
    let mut nodes = Vec::new();
    for device in items(&found) {
        let Ok(properties) = device.Properties() else {
            continue;
        };
        let Some(level) = number_property(&properties, BATTERY_PROPERTY) else {
            continue;
        };
        nodes.push(BatteryNode {
            instance_id: device.Id().map(|s| s.to_string()).unwrap_or_default(),
            name: string_property(&properties, "System.ItemNameDisplay")
                .or_else(|| device.Name().ok().map(|s| s.to_string()))
                .unwrap_or_default(),
            level: level.min(100) as u8,
            present: bool_property(&properties, "System.Devices.Present").unwrap_or(false),
            container: container_property(&properties, "System.Devices.ContainerId"),
        });
    }
    Ok(nodes)
}

/// One full scan: `(devices, radio is on)`.
fn scan() -> Result<(Vec<Reading>, bool)> {
    if !radio_is_on() {
        return Ok((assemble(&paired_devices_radio_off()?, &[], false), false));
    }
    let endpoints: Vec<Endpoint> = paired_devices()?
        .into_iter()
        .map(|mut paired| {
            if paired.connected.is_none() && !paired.endpoint.connected {
                paired.endpoint.connected =
                    is_connected(&paired.id, paired.low_energy).unwrap_or(false);
            }
            paired.endpoint
        })
        .collect();
    // A failing battery query must not hide the devices themselves.
    let nodes = battery_nodes().unwrap_or_else(|error| {
        log::debug!("battery node query failed: {error}");
        Vec::new()
    });
    Ok((assemble(&endpoints, &nodes, true), true))
}

/// Device watchers and radio listeners. They only wake the worker; the worker rescans.
struct Watchers {
    watchers: Vec<DeviceWatcher>,
    radios: Vec<(Radio, i64)>,
    stopped: Arc<AtomicBool>,
}

impl Watchers {
    fn start(wake: &Arc<Wake>) -> Result<Watchers> {
        let stopped = Arc::new(AtomicBool::new(false));
        let no_properties = property_list(&[])?;
        let connection = property_list(&["System.Devices.Aep.IsConnected"])?;
        let endpoint = DeviceInformationKind::AssociationEndpoint;
        let (classic, low_energy) = (
            BluetoothDevice::GetDeviceSelectorFromPairingState(true)?,
            BluetoothLEDevice::GetDeviceSelectorFromPairingState(true)?,
        );
        let created = [
            DeviceInformation::CreateWatcherWithKindAqsFilterAndAdditionalProperties(
                &classic,
                &connection,
                endpoint,
            ),
            DeviceInformation::CreateWatcherWithKindAqsFilterAndAdditionalProperties(
                &low_energy,
                &connection,
                endpoint,
            ),
            DeviceInformation::CreateWatcherWithKindAqsFilterAndAdditionalProperties(
                &HSTRING::from(NODE_FILTER),
                &property_list(&[BATTERY_PROPERTY])?,
                DeviceInformationKind::Device,
            ),
            // Adapter added/removed (USB Bluetooth dongles).
            DeviceInformation::CreateWatcherAqsFilterAndAdditionalProperties(
                &BluetoothAdapter::GetDeviceSelector()?,
                &no_properties,
            ),
        ];

        let mut watchers = Vec::new();
        for watcher in created.into_iter().flatten() {
            let on_added = wake.clone();
            watcher.Added(&TypedEventHandler::<DeviceWatcher, DeviceInformation>::new(
                move |_, _| {
                    on_added.notify();
                    Ok(())
                },
            ))?;
            let on_updated = wake.clone();
            watcher.Updated(
                &TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(move |_, _| {
                    on_updated.notify();
                    Ok(())
                }),
            )?;
            let on_removed = wake.clone();
            watcher.Removed(
                &TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(move |_, _| {
                    on_removed.notify();
                    Ok(())
                }),
            )?;
            // "Stopped" without us asking means the enumeration service restarted.
            let (on_stopped, flag) = (wake.clone(), stopped.clone());
            watcher.Stopped(&TypedEventHandler::<DeviceWatcher, IInspectable>::new(
                move |_, _| {
                    flag.store(true, Ordering::Relaxed);
                    on_stopped.notify();
                    Ok(())
                },
            ))?;
            watcher.Start()?;
            watchers.push(watcher);
        }
        if watchers.is_empty() {
            return Err(windows::core::Error::empty());
        }

        let mut radios = Vec::new();
        if let Ok(all) = Radio::GetRadiosAsync().and_then(|operation| operation.join()) {
            for radio in (0..all.Size().unwrap_or(0)).filter_map(|i| all.GetAt(i).ok()) {
                if !radio.Kind().is_ok_and(|k| k == RadioKind::Bluetooth) {
                    continue;
                }
                let on_change = wake.clone();
                let token = radio.StateChanged(&TypedEventHandler::<Radio, IInspectable>::new(
                    move |_, _| {
                        on_change.notify();
                        Ok(())
                    },
                ));
                if let Ok(token) = token {
                    radios.push((radio, token));
                }
            }
        }
        Ok(Watchers {
            watchers,
            radios,
            stopped,
        })
    }

    fn needs_restart(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }
}

impl Drop for Watchers {
    fn drop(&mut self) {
        for (radio, token) in &self.radios {
            let _ = radio.RemoveStateChanged(*token);
        }
        for watcher in &self.watchers {
            let _ = watcher.Stop();
        }
    }
}

/// Worker loop. `emit` receives the device list and whether the radio is on.
pub fn run(wake: Arc<Wake>, emit: impl Fn(Vec<Reading>, bool)) {
    super::init_winrt_thread();
    let start_watchers = |wake: &Arc<Wake>| {
        Watchers::start(wake)
            .inspect_err(|error| {
                log::warn!("Bluetooth events unavailable ({error}); polling instead")
            })
            .ok()
    };
    let mut watchers = start_watchers(&wake);
    let mut failures = 0u32;
    let mut radio_was_on = true;
    let mut follow_ups = 0;
    while !wake.is_stopped() {
        if watchers.as_ref().is_some_and(Watchers::needs_restart) || failures == 4 {
            log::info!("restarting Bluetooth watchers");
            // Stop the old watchers before creating new ones.
            drop(watchers.take());
            watchers = start_watchers(&wake);
        }
        match scan() {
            Ok((devices, radio_on)) => {
                if radio_on && !radio_was_on {
                    follow_ups = FOLLOW_UP_SCANS;
                }
                radio_was_on = radio_on;
                failures = 0;
                emit(devices, radio_on);
            }
            Err(error) => {
                failures += 1;
                if failures == 1 || failures % 100 == 0 {
                    log::warn!("Bluetooth scan failed: {error}");
                }
                if failures == 1 {
                    // Do not keep showing devices as connected on stale data.
                    emit(Vec::new(), false);
                }
            }
        }
        let pause = if follow_ups > 0 {
            follow_ups -= 1;
            FOLLOW_UP_INTERVAL
        } else if failures > 0 {
            Duration::from_secs(u64::from(failures * 2).min(30))
        } else if watchers.is_some() {
            SAFETY_INTERVAL
        } else {
            FALLBACK_INTERVAL
        };
        if wake.sleep(pause) && !wake.is_stopped() {
            std::thread::sleep(COALESCE);
            wake.take_refresh();
        }
    }
}
