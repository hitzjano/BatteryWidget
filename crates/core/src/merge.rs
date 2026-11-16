//! Fusion of all sources into one de-duplicated, sorted device list.
//!
//! Everything in here is pure (no I/O, time is passed in), which keeps it unit-testable.

use std::collections::HashMap;

use crate::model::{Device, Kind, Link, Reading};
use crate::names::{is_same_physical_device, norm_device_name, short_name};

/// A Logitech device that stopped answering stays listed as "idle" for this long.
pub const IDLE_GRACE_S: f64 = 600.0;
/// A Bluetooth level has to rise at least this long after connecting to count as charging.
pub const BT_CHARGE_WARMUP_S: f64 = 90.0;
/// The charging estimate expires when the level has not risen for this long.
pub const BT_CHARGE_TIMEOUT_S: f64 = 1800.0;

/// Windows does not expose the charging state of Bluetooth devices. A level that rises
/// while the device stays connected is the only available signal.
#[derive(Default)]
pub struct ChargeEstimator {
    state: HashMap<String, ChargeState>,
}

struct ChargeState {
    level: u8,
    since: f64,
    rose_at: Option<f64>,
}

impl ChargeEstimator {
    pub fn update(&mut self, id: &str, level: Option<u8>, connected: bool, now: f64) -> bool {
        let Some(level) = level.filter(|_| connected) else {
            self.state.remove(id);
            return false;
        };
        let Some(st) = self.state.get_mut(id) else {
            self.state.insert(
                id.to_string(),
                ChargeState {
                    level,
                    since: now,
                    rose_at: None,
                },
            );
            return false;
        };
        if level > st.level && now - st.since >= BT_CHARGE_WARMUP_S {
            st.rose_at = Some(now);
        } else if level < st.level {
            st.rose_at = None;
        }
        st.level = level;
        match st.rose_at {
            Some(at) if level < 100 && now - at <= BT_CHARGE_TIMEOUT_S => true,
            _ => {
                st.rose_at = None;
                false
            }
        }
    }

    fn retain(&mut self, ids: &[&str]) {
        self.state.retain(|id, _| ids.contains(&id.as_str()));
    }
}

/// The latest output of every source.
#[derive(Clone, Debug, Default)]
pub struct Sources {
    pub ghub: Vec<Reading>,
    pub hidpp: Vec<Reading>,
    pub vendor: Vec<Reading>,
    pub bluetooth: Vec<Reading>,
    /// `false` while the Bluetooth radio is switched off.
    pub bt_enabled: bool,
}

/// State carried between merges.
#[derive(Default)]
pub struct MergeState {
    /// Last connected snapshot of Logitech devices, with the time it was taken.
    memory: HashMap<String, (Device, f64)>,
    charge: ChargeEstimator,
}

fn device_from(r: &Reading, name: String, bt_enabled: bool) -> Device {
    let connected = r.connected && (!r.link.is_bluetooth() || bt_enabled);
    Device {
        id: r.id.clone(),
        name,
        battery: r.battery,
        kind: r.kind,
        link: r.link,
        connected,
        charging: r.charging && connected,
        full: r.full && connected && r.battery.is_some_and(|b| b >= 98),
        charging_estimated: false,
        idle: r.idle && connected,
    }
}

/// Fill gaps of `target` with what a second source knows about the same device.
fn fill_from(target: &mut Device, other: &Reading) {
    if target.battery.is_none() {
        target.battery = other.battery;
    }
    if other.connected && !target.connected {
        target.connected = true;
    }
    if other.charging && target.connected {
        target.charging = true;
    }
    if other.full && target.battery.is_some_and(|b| b >= 98) {
        target.full = true;
    }
}

fn is_radio_or_wired(link: Link) -> bool {
    !link.is_bluetooth()
}

/// Is a Bluetooth-linked vendor device (e.g. a DualSense) connected according to the
/// Windows Bluetooth scan? `None` when the scan does not know the device.
fn bt_connection(bt: &[Reading], name: &str, aliases: &[String]) -> Option<bool> {
    let mut candidates = vec![name.trim().to_lowercase(), norm_device_name(name)];
    candidates.extend(aliases.iter().cloned());
    bt.iter()
        .find(|b| {
            let b_name = b.name.trim().to_lowercase();
            candidates.iter().any(|c| {
                *c == b_name
                    || (c.len() >= 4 && (b_name.contains(c.as_str()) || c.contains(&b_name)))
            })
        })
        .map(|b| b.connected)
}

/// Merge all sources. Order of precedence: G HUB, HID++, vendor HID, Bluetooth.
pub fn build_device_list(src: &Sources, state: &mut MergeState, now: f64) -> Vec<Device> {
    let mut list: Vec<Device> = Vec::new();
    let mut aliases: Vec<String> = Vec::new();

    // 1. Logitech: G HUB first (exact values), direct HID++ fills gaps or adds devices.
    for r in src.ghub.iter().chain(&src.hidpp) {
        let name = short_name(&r.name);
        let norm = norm_device_name(&name);
        match list
            .iter_mut()
            .find(|d| d.id == r.id || norm_device_name(&d.name) == norm)
        {
            Some(existing) => fill_from(existing, r),
            None => list.push(device_from(r, name, src.bt_enabled)),
        }
    }

    // 2. Vendor HID and controllers.
    for r in &src.vendor {
        let norm = norm_device_name(&r.name);
        if let Some(existing) = list
            .iter_mut()
            .find(|d| d.id == r.id || norm_device_name(&d.name) == norm)
        {
            fill_from(existing, r);
            continue;
        }
        let mut dev = device_from(r, r.name.clone(), src.bt_enabled);
        if r.link.is_bluetooth() && src.bt_enabled {
            if let Some(connected) = bt_connection(&src.bluetooth, &r.name, &r.aliases) {
                dev.connected = connected;
                dev.charging &= connected;
                dev.full &= connected;
            }
        }
        aliases.extend(r.aliases.iter().cloned());
        list.push(dev);
    }

    // 3. Logitech devices that went quiet keep their last level for a grace period.
    for dev in list.iter_mut().filter(|d| d.id.starts_with("logi:")) {
        if dev.connected {
            state.memory.insert(dev.id.clone(), (dev.clone(), now));
        } else if let Some((last, seen)) = state.memory.get(&dev.id) {
            if dev.battery.is_none() {
                dev.battery = last.battery;
            }
            if !dev.link.is_bluetooth() && now - seen < IDLE_GRACE_S {
                dev.connected = true;
                dev.idle = true;
            }
        }
    }
    let present: Vec<String> = list.iter().map(|d| d.id.clone()).collect();
    let active_names: Vec<String> = list.iter().map(|d| norm_device_name(&d.name)).collect();
    state.memory.retain(|id, (last, seen)| {
        if present.contains(id) {
            return true;
        }
        let renamed = active_names.contains(&norm_device_name(&last.name));
        let keep = !renamed && !last.link.is_bluetooth() && now - *seen < IDLE_GRACE_S;
        if keep {
            let mut ghost = last.clone();
            ghost.idle = true;
            ghost.charging = false;
            ghost.full = false;
            list.push(ghost);
        }
        keep
    });

    // 4. Bluetooth: update devices already known from a better source, append the rest.
    let mut bt_ids: Vec<&str> = Vec::new();
    for r in &src.bluetooth {
        let connected = r.connected && (!r.link.is_bluetooth() || src.bt_enabled);
        let lower = r.name.trim().to_lowercase();
        let norm = norm_device_name(&r.name);
        let existing = list.iter_mut().find(|d| {
            is_same_physical_device(&d.name, d.kind, &r.name, r.kind)
                || norm_device_name(&d.name) == norm
                || (r.kind == d.kind && aliases.contains(&lower))
        });
        if let Some(d) = existing {
            if d.link.is_bluetooth() && d.connected != connected {
                d.connected = connected;
                if !connected {
                    d.charging = false;
                    d.full = false;
                }
            }
            if d.battery.is_none() && connected {
                d.battery = r.battery;
            }
            continue;
        }
        bt_ids.push(&r.id);
        let estimated = state.charge.update(&r.id, r.battery, connected, now);
        let mut dev = device_from(r, r.name.trim().to_string(), src.bt_enabled);
        dev.charging = estimated;
        dev.charging_estimated = estimated;
        list.push(dev);
    }
    state.charge.retain(&bt_ids);

    // 5. Collapse remaining duplicates (same device seen over two links).
    let mut merged: Vec<Device> = Vec::with_capacity(list.len());
    for dev in list {
        let twin = merged
            .iter_mut()
            .find(|m| is_same_physical_device(&m.name, m.kind, &dev.name, dev.kind));
        let Some(slot) = twin else {
            merged.push(dev);
            continue;
        };
        let dev_wins = (dev.connected && !slot.connected)
            || (dev.connected == slot.connected && dev.battery.is_some() && slot.battery.is_none());
        let (mut winner, loser) = if dev_wins {
            (dev, slot.clone())
        } else {
            (slot.clone(), dev)
        };
        if winner.battery.is_none() {
            winner.battery = loser.battery;
        }
        winner.charging |= loser.charging && winner.connected;
        winner.full |= loser.full && winner.battery.is_some_and(|b| b >= 98);
        if winner.link.is_bluetooth() && is_radio_or_wired(loser.link) && loser.connected {
            winner.link = loser.link;
        }
        if winner.kind == Kind::Generic {
            winner.kind = loser.kind;
        }
        *slot = winner;
    }

    merged.sort_by(|a, b| {
        (!a.is_active(), a.kind, a.name.to_lowercase()).cmp(&(
            !b.is_active(),
            b.kind,
            b.name.to_lowercase(),
        ))
    });
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(id: &str, name: &str, kind: Kind, link: Link, battery: Option<u8>) -> Reading {
        Reading::new(id, name, kind, link).battery(battery)
    }

    fn sources() -> Sources {
        Sources {
            bt_enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn ghub_and_hidpp_do_not_duplicate() {
        let mut src = sources();
        src.ghub = vec![reading(
            "logi:AAAA0001",
            "G502 LIGHTSPEED Wireless Gaming Mouse",
            Kind::Mouse,
            Link::Lightspeed,
            None,
        )];
        src.hidpp = vec![reading(
            "logi:AAAA0001",
            "G502 LIGHTSPEED",
            Kind::Mouse,
            Link::Lightspeed,
            Some(64),
        )
        .charging(true)];
        let out = build_device_list(&src, &mut MergeState::default(), 0.0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "G502");
        assert_eq!(out[0].battery, Some(64));
        assert!(out[0].charging);
    }

    #[test]
    fn bluetooth_duplicate_of_receiver_device_is_dropped() {
        let mut src = sources();
        src.hidpp = vec![reading(
            "logi:1",
            "MX Keys",
            Kind::Keyboard,
            Link::Bolt,
            Some(80),
        )];
        src.bluetooth = vec![
            reading("bt:aa", "MX Keys", Kind::Keyboard, Link::Bluetooth, None).connected(false),
            reading(
                "bt:bb",
                "WH-1000XM4",
                Kind::Headset,
                Link::Bluetooth,
                Some(70),
            ),
        ];
        let out = build_device_list(&src, &mut MergeState::default(), 0.0);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name, "MX Keys");
        assert_eq!(out[0].link, Link::Bolt);
        assert!(out[0].connected);
        assert_eq!(out[1].name, "WH-1000XM4");
    }

    #[test]
    fn radio_off_disconnects_bluetooth_only() {
        let mut src = sources();
        src.bt_enabled = false;
        src.hidpp = vec![reading(
            "logi:1",
            "MX Master 3S",
            Kind::Mouse,
            Link::Bolt,
            Some(50),
        )];
        src.bluetooth = vec![reading(
            "bt:bb",
            "WH-1000XM4",
            Kind::Headset,
            Link::Bluetooth,
            Some(70),
        )];
        let out = build_device_list(&src, &mut MergeState::default(), 0.0);
        assert!(out[0].connected);
        assert!(!out[1].connected);
    }

    #[test]
    fn sleeping_logitech_device_stays_idle_then_drops() {
        let mut state = MergeState::default();
        let mut src = sources();
        src.hidpp = vec![reading(
            "logi:1",
            "MX Master 3S",
            Kind::Mouse,
            Link::Bolt,
            Some(50),
        )];
        build_device_list(&src, &mut state, 0.0);

        src.hidpp =
            vec![reading("logi:1", "MX Master 3S", Kind::Mouse, Link::Bolt, None).connected(false)];
        let out = build_device_list(&src, &mut state, 100.0);
        assert!(out[0].idle && out[0].is_active());
        assert_eq!(out[0].battery, Some(50));

        let out = build_device_list(&src, &mut state, IDLE_GRACE_S + 1.0);
        assert!(!out[0].idle && !out[0].is_active());

        // Receiver unplugged: the device vanishes from the source, the ghost expires too.
        src.hidpp.clear();
        assert!(build_device_list(&src, &mut state, IDLE_GRACE_S + 2.0).is_empty());
    }

    #[test]
    fn unplugged_receiver_keeps_ghost_during_grace() {
        let mut state = MergeState::default();
        let mut src = sources();
        src.hidpp = vec![reading(
            "logi:1",
            "G915 TKL",
            Kind::Keyboard,
            Link::Lightspeed,
            Some(40),
        )];
        build_device_list(&src, &mut state, 0.0);
        src.hidpp.clear();
        let out = build_device_list(&src, &mut state, 60.0);
        assert_eq!(out.len(), 1);
        assert!(out[0].idle);
        assert!(build_device_list(&src, &mut state, 700.0).is_empty());
    }

    #[test]
    fn charge_estimation_needs_warmup_and_rise() {
        let mut est = ChargeEstimator::default();
        assert!(!est.update("bt:1", Some(50), true, 0.0));
        assert!(
            !est.update("bt:1", Some(60), true, 10.0),
            "rise during warm-up is ignored"
        );
        assert!(est.update("bt:1", Some(70), true, 100.0));
        assert!(
            est.update("bt:1", Some(70), true, 200.0),
            "holds while level is steady"
        );
        assert!(
            !est.update("bt:1", Some(69), true, 300.0),
            "a drop ends the estimate"
        );
        assert!(est.update("bt:1", Some(80), true, 400.0));
        assert!(!est.update("bt:1", Some(80), true, 400.0 + BT_CHARGE_TIMEOUT_S + 1.0));
        assert!(!est.update("bt:1", Some(100), true, 5000.0));
        assert!(!est.update("bt:1", None, false, 5001.0));
    }

    #[test]
    fn bluetooth_controller_uses_scan_for_connection_state() {
        let mut src = sources();
        src.vendor = vec![reading(
            "ps:0ce6",
            "DualSense",
            Kind::Gamepad,
            Link::Bluetooth,
            Some(75),
        )
        .aliases(&["dualsense wireless controller"])];
        src.bluetooth = vec![reading(
            "bt:cc",
            "DualSense Wireless Controller",
            Kind::Gamepad,
            Link::Bluetooth,
            None,
        )];
        let out = build_device_list(&src, &mut MergeState::default(), 0.0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "DualSense");
        assert_eq!(out[0].battery, Some(75));
        assert!(out[0].connected);
    }

    #[test]
    fn sorted_by_activity_kind_and_name() {
        let mut src = sources();
        src.bluetooth = vec![
            reading(
                "bt:1",
                "Zeta Buds",
                Kind::Headset,
                Link::Bluetooth,
                Some(10),
            ),
            reading(
                "bt:2",
                "Alpha Mouse X",
                Kind::Mouse,
                Link::Bluetooth,
                Some(10),
            ),
            reading(
                "bt:3",
                "Beta Keyboard X",
                Kind::Keyboard,
                Link::Bluetooth,
                Some(10),
            )
            .connected(false),
            reading(
                "bt:4",
                "Alpha Buds",
                Kind::Headset,
                Link::Bluetooth,
                Some(10),
            ),
        ];
        let names: Vec<String> = build_device_list(&src, &mut MergeState::default(), 0.0)
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert_eq!(
            names,
            [
                "Alpha Mouse X",
                "Alpha Buds",
                "Zeta Buds",
                "Beta Keyboard X"
            ]
        );
    }
}
