//! Runs every source on its own thread and publishes the merged device list.
//!
//! [`Aggregator`] is the pure part (apply an update, get a new list if it changed);
//! [`Hub`] owns the threads. The UI only ever sees complete, immutable snapshots.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::merge::{build_device_list, MergeState, Sources};
use crate::model::{Device, Reading};
use crate::wake::Wake;

/// Devices in their grace period change state without any source reporting, so the list
/// is rebuilt at least this often.
const REBUILD_INTERVAL: Duration = Duration::from_secs(30);

/// Which sources to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HubOptions {
    pub bluetooth: bool,
    pub hidpp: bool,
    pub ghub: bool,
    pub vendor: bool,
    pub xbox: bool,
}

impl Default for HubOptions {
    fn default() -> Self {
        HubOptions {
            bluetooth: true,
            hidpp: true,
            ghub: true,
            vendor: true,
            xbox: true,
        }
    }
}

/// The latest complete result of one source.
#[derive(Clone, Debug)]
pub enum Update {
    Ghub(Vec<Reading>),
    Hidpp(Vec<Reading>),
    Vendor(Vec<Reading>),
    Bluetooth {
        devices: Vec<Reading>,
        radio_on: bool,
    },
    /// Nothing new, but time has passed.
    Tick,
}

/// Keeps the latest output of each source and merges on demand.
pub struct Aggregator {
    sources: Sources,
    state: MergeState,
    published: Option<Vec<Device>>,
}

impl Default for Aggregator {
    fn default() -> Self {
        Aggregator {
            sources: Sources {
                bt_enabled: true,
                ..Default::default()
            },
            state: MergeState::default(),
            published: None,
        }
    }
}

impl Aggregator {
    /// Apply an update at monotonic time `now` (seconds). Returns the new device list if
    /// it differs from the last one returned.
    pub fn apply(&mut self, update: Update, now: f64) -> Option<Vec<Device>> {
        match update {
            Update::Ghub(readings) => self.sources.ghub = readings,
            Update::Hidpp(readings) => self.sources.hidpp = readings,
            Update::Vendor(readings) => self.sources.vendor = readings,
            Update::Bluetooth { devices, radio_on } => {
                if radio_on || !devices.is_empty() {
                    self.sources.bluetooth = devices;
                }
                self.sources.bt_enabled = radio_on;
            }
            Update::Tick => {}
        }
        let devices = build_device_list(&self.sources, &mut self.state, now);
        if self.published.as_ref() == Some(&devices) {
            return None;
        }
        self.published = Some(devices.clone());
        Some(devices)
    }
}

/// Handle to the running workers. Dropping it stops them.
pub struct Hub {
    wakes: Vec<Arc<Wake>>,
    aggregator: Option<JoinHandle<()>>,
    stop: Sender<Option<Update>>,
}

impl Hub {
    /// Start all enabled sources. `on_change` is called from a background thread with a
    /// full snapshot every time the merged list changes.
    pub fn start(options: HubOptions, on_change: impl Fn(Vec<Device>) + Send + 'static) -> Hub {
        let (tx, rx) = channel::<Option<Update>>();
        let started = Instant::now();
        let aggregator = thread::Builder::new()
            .name("aggregator".into())
            .spawn(move || {
                let mut aggregator = Aggregator::default();
                loop {
                    let update = match rx.recv_timeout(REBUILD_INTERVAL) {
                        Ok(Some(update)) => update,
                        Ok(None) | Err(RecvTimeoutError::Disconnected) => break,
                        Err(RecvTimeoutError::Timeout) => Update::Tick,
                    };
                    if let Some(devices) = aggregator.apply(update, started.elapsed().as_secs_f64())
                    {
                        on_change(devices);
                    }
                }
            })
            .ok();

        let mut hub = Hub {
            wakes: Vec::new(),
            aggregator,
            stop: tx.clone(),
        };
        if options.ghub {
            let tx = tx.clone();
            hub.spawn("ghub", move |wake| {
                crate::ghub::run(wake, |r| drop(tx.send(Some(Update::Ghub(r)))));
            });
        }
        #[cfg(windows)]
        {
            if options.bluetooth {
                let tx = tx.clone();
                hub.spawn("bluetooth", move |wake| {
                    crate::win::bluetooth::run(wake, |devices, radio_on| {
                        drop(tx.send(Some(Update::Bluetooth { devices, radio_on })))
                    });
                });
            }
            if options.hidpp {
                let tx = tx.clone();
                hub.spawn("hidpp", move |wake| {
                    crate::win::hidpp::run(wake, |r| drop(tx.send(Some(Update::Hidpp(r)))));
                });
            }
            if options.vendor || options.xbox {
                let tx = tx.clone();
                hub.spawn("vendor", move |wake| {
                    crate::win::vendor::run(wake, options.vendor, options.xbox, |r| {
                        drop(tx.send(Some(Update::Vendor(r))))
                    });
                });
            }
        }
        hub
    }

    fn spawn(&mut self, name: &'static str, work: impl FnOnce(Arc<Wake>) + Send + 'static) {
        let wake = Arc::new(Wake::default());
        let handle = wake.clone();
        let spawned = thread::Builder::new().name(name.into()).spawn(move || {
            // A bug in one protocol must not take the other sources (or the UI) down.
            if catch_unwind(AssertUnwindSafe(|| work(handle))).is_err() {
                log::error!("worker '{name}' panicked and was stopped");
            }
        });
        match spawned {
            Ok(_) => self.wakes.push(wake),
            Err(e) => log::error!("could not start worker '{name}': {e}"),
        }
    }

    /// Ask every source to rescan now.
    pub fn refresh(&self) {
        for wake in &self.wakes {
            wake.notify();
        }
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        // Workers may be in the middle of a device timeout; they exit on their own and are
        // not joined. The aggregator is, so no callback fires after `drop` returns.
        for wake in &self.wakes {
            wake.stop();
        }
        let _ = self.stop.send(None);
        if let Some(handle) = self.aggregator.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Kind, Link};

    fn mouse(level: u8) -> Reading {
        Reading::new("logi:1", "MX Master 3S", Kind::Mouse, Link::Bolt).battery(Some(level))
    }

    #[test]
    fn emits_only_on_change() {
        let mut a = Aggregator::default();
        assert_eq!(
            a.apply(Update::Tick, 0.0),
            Some(vec![]),
            "first result is always published"
        );
        assert_eq!(a.apply(Update::Tick, 1.0), None);
        assert_eq!(
            a.apply(Update::Hidpp(vec![mouse(50)]), 2.0).unwrap().len(),
            1
        );
        assert_eq!(a.apply(Update::Hidpp(vec![mouse(50)]), 3.0), None);
        assert_eq!(
            a.apply(Update::Hidpp(vec![mouse(49)]), 4.0).unwrap()[0].battery,
            Some(49)
        );
    }

    #[test]
    fn radio_off_keeps_known_bluetooth_devices_as_disconnected() {
        let mut a = Aggregator::default();
        let buds = Reading::new("bt:1", "Buds", Kind::Headset, Link::Bluetooth).battery(Some(80));
        a.apply(
            Update::Bluetooth {
                devices: vec![buds],
                radio_on: true,
            },
            0.0,
        );
        let out = a
            .apply(
                Update::Bluetooth {
                    devices: vec![],
                    radio_on: false,
                },
                1.0,
            )
            .unwrap();
        assert_eq!(out.len(), 1);
        assert!(!out[0].connected);
    }

    #[test]
    fn hub_starts_and_stops_cleanly() {
        let options = HubOptions {
            bluetooth: false,
            hidpp: false,
            ghub: false,
            vendor: false,
            xbox: false,
        };
        let (tx, rx) = channel();
        let hub = Hub::start(options, move |devices| drop(tx.send(devices)));
        hub.stop.send(Some(Update::Hidpp(vec![mouse(42)]))).unwrap();
        let devices = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(devices[0].battery, Some(42));
        drop(hub);
    }
}
