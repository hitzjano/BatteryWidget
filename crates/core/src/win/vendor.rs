//! Worker for vendor HID devices and game controllers, with per-device back-off.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::hid::{enumerate, interface_paths, product_string};
use super::xbox::Xbox;
use crate::hid::{vid_pid_from_path, HidInfo};
use crate::model::{Kind, Reading};
use crate::vendor::{driver_for, is_known_vendor};
use crate::wake::Wake;

const SCAN_INTERVAL: Duration = Duration::from_secs(60);
/// While a device is connected its level and charging state are read this often.
const ACTIVE_INTERVAL: Duration = Duration::from_secs(20);
/// How often the list of HID interfaces is checked for new or removed devices (plugging
/// in a charging cable often makes a device appear under a second, wired product id).
const WATCH_INTERVAL: Duration = Duration::from_secs(3);
/// A device that ignores its driver's query is not asked again for this long …
const BACKOFF: Duration = Duration::from_secs(600);
/// … unless staying silent is normal for it (asleep, headset switched off).
const SHORT_BACKOFF: Duration = Duration::from_secs(120);
/// Only a query that ran into its timeout counts as "ignored".
const SLOW_QUERY: Duration = Duration::from_millis(800);
/// A sleeping mouse or keyboard keeps its last level: dimmed after this long …
const ASLEEP_DIM_AFTER: Duration = Duration::from_secs(180);
/// … and removed after this long.
const ASLEEP_GRACE: Duration = Duration::from_secs(600);

#[derive(Default)]
struct VendorScanner {
    backoff: HashMap<String, Instant>,
    last_read: HashMap<String, (Instant, Vec<Reading>)>,
}

impl VendorScanner {
    /// Last readings of a receiver that is still plugged in but currently silent.
    fn asleep(&self, key: &str, now: Instant) -> Vec<Reading> {
        let Some((seen, readings)) = self.last_read.get(key) else {
            return Vec::new();
        };
        let age = now.duration_since(*seen);
        if age >= ASLEEP_GRACE {
            return Vec::new();
        }
        readings
            .iter()
            .filter(|r| matches!(r.kind, Kind::Mouse | Kind::Keyboard) && !r.link.is_bluetooth())
            .cloned()
            .map(|mut reading| {
                if age >= ASLEEP_DIM_AFTER {
                    reading.idle = true;
                    reading.charging = false;
                    reading.full = false;
                }
                reading
            })
            .collect()
    }

    /// Query every supported device once. Never fails: problems only shorten the result.
    fn scan(&mut self) -> Vec<Reading> {
        let mut groups: BTreeMap<String, Vec<HidInfo>> = BTreeMap::new();
        for info in enumerate(is_known_vendor) {
            groups.entry(info.group()).or_default().push(info);
        }
        self.last_read.retain(|key, _| groups.contains_key(key));
        self.backoff.retain(|key, _| groups.contains_key(key));

        let mut results = Vec::new();
        for (key, group) in &groups {
            let Some(driver) = driver_for(group) else {
                continue;
            };
            let now = Instant::now();
            if self.backoff.get(key).is_some_and(|until| *until > now) {
                results.extend(self.asleep(key, now));
                continue;
            }
            let product = product_string(&group[0].path);
            let found = (driver.read)(group, &product);
            if !found.is_empty() {
                self.last_read
                    .insert(key.clone(), (Instant::now(), found.clone()));
                results.extend(found);
                continue;
            }
            let wait = match (driver.silence_is_normal, self.last_read.contains_key(key)) {
                (false, _) => Some(BACKOFF),
                (true, false) => Some(SHORT_BACKOFF),
                // It answered before, so it is merely asleep: keep asking.
                (true, true) => None,
            };
            if let Some(wait) = wait.filter(|_| now.elapsed() > SLOW_QUERY) {
                log::info!(
                    "{} {key} does not answer; retrying in {} min",
                    driver.name,
                    wait.as_secs() / 60
                );
                self.backoff.insert(key.clone(), now + wait);
            }
            results.extend(self.asleep(key, now));
        }
        results
    }
}

/// Paths of all HID interfaces of supported vendors. Cheap: no device is opened.
fn vendor_paths() -> Vec<String> {
    let mut paths: Vec<String> = interface_paths()
        .into_iter()
        .filter(|path| vid_pid_from_path(path).is_some_and(|(vid, _)| is_known_vendor(vid)))
        .collect();
    paths.sort();
    paths
}

/// Worker loop: scan at start, every 20 s while a device is connected (every minute
/// otherwise), on request, when a controller (dis)connects and when a HID device of a
/// supported vendor appears or disappears.
pub fn run(wake: Arc<Wake>, vendor_hid: bool, controllers: bool, emit: impl Fn(Vec<Reading>)) {
    super::init_winrt_thread();
    let mut scanner = VendorScanner::default();
    let mut xbox = Xbox::default();
    while !wake.is_stopped() {
        let known = if vendor_hid {
            vendor_paths()
        } else {
            Vec::new()
        };
        let mut readings = if vendor_hid {
            scanner.scan()
        } else {
            Vec::new()
        };
        if controllers {
            readings.extend(xbox.read(&wake));
        }
        let active = readings.iter().any(|r| r.connected && !r.idle);
        emit(readings);

        let deadline = Instant::now()
            + if active {
                ACTIVE_INTERVAL
            } else {
                SCAN_INTERVAL
            };
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            if wake.sleep(remaining.min(WATCH_INTERVAL)) {
                break;
            }
            if vendor_hid && vendor_paths() != known {
                // Give the new interface a moment to finish enumerating.
                wake.sleep(Duration::from_millis(500));
                break;
            }
        }
    }
}
