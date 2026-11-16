//! Low-battery and fully-charged notifications with hysteresis and a cool-down.

use std::collections::HashMap;

use crate::model::Device;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlertSettings {
    pub low_enabled: bool,
    pub full_enabled: bool,
    /// Notify at or below this percentage.
    pub low_threshold: u8,
    /// Minimum time between two low-battery notifications for the same device.
    pub cooldown_s: f64,
}

impl Default for AlertSettings {
    fn default() -> Self {
        AlertSettings {
            low_enabled: true,
            full_enabled: false,
            low_threshold: 20,
            cooldown_s: 3600.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Alert {
    Low { name: String, level: u8 },
    Full { name: String },
}

#[derive(Default)]
pub struct AlertManager {
    low_last: HashMap<String, f64>,
    low_disarmed: HashMap<String, bool>,
    full_disarmed: HashMap<String, bool>,
    was_charging: HashMap<String, bool>,
}

impl AlertManager {
    /// Evaluate a new device list. `now` is a monotonic time in seconds.
    pub fn check(&mut self, devices: &[Device], settings: &AlertSettings, now: f64) -> Vec<Alert> {
        let mut alerts = Vec::new();
        for d in devices {
            let Some(level) = d.battery.filter(|_| d.connected) else {
                self.was_charging.remove(&d.id);
                continue;
            };
            let low = settings.low_threshold;
            // Re-arm once the device charges or climbs clearly above the threshold.
            if d.charging || (level >= low.saturating_add(2).min(100) && level > low) {
                self.low_disarmed.remove(&d.id);
            } else if settings.low_enabled
                && !d.idle
                && level <= low
                && !self.low_disarmed.contains_key(&d.id)
                && self
                    .low_last
                    .get(&d.id)
                    .is_none_or(|last| now - last > settings.cooldown_s)
            {
                self.low_disarmed.insert(d.id.clone(), true);
                self.low_last.insert(d.id.clone(), now);
                alerts.push(Alert::Low {
                    name: d.name.clone(),
                    level,
                });
            }

            if level <= 90 {
                self.full_disarmed.remove(&d.id);
            }
            let charged = level >= 100 || (d.full && level >= 99);
            let was_charging = self.was_charging.get(&d.id).copied().unwrap_or(false);
            if settings.full_enabled
                && charged
                && (was_charging || d.charging)
                && !self.full_disarmed.contains_key(&d.id)
            {
                self.full_disarmed.insert(d.id.clone(), true);
                alerts.push(Alert::Full {
                    name: d.name.clone(),
                });
            }
            self.was_charging.insert(d.id.clone(), d.charging);
        }
        alerts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Kind, Link};

    fn dev(level: u8, charging: bool) -> Device {
        Device {
            id: "x".into(),
            name: "Mouse".into(),
            battery: Some(level),
            kind: Kind::Mouse,
            link: Link::Bolt,
            connected: true,
            charging,
            full: false,
            charging_estimated: false,
            idle: false,
        }
    }

    #[test]
    fn low_alert_fires_once_and_rearms_with_hysteresis() {
        let mut m = AlertManager::default();
        let s = AlertSettings::default();
        assert!(m.check(&[dev(25, false)], &s, 0.0).is_empty());
        assert_eq!(
            m.check(&[dev(20, false)], &s, 1.0),
            [Alert::Low {
                name: "Mouse".into(),
                level: 20
            }]
        );
        assert!(m.check(&[dev(19, false)], &s, 2.0).is_empty());
        assert!(
            m.check(&[dev(21, false)], &s, 3.0).is_empty(),
            "21 % is inside the hysteresis band"
        );
        assert!(m.check(&[dev(20, false)], &s, 4.0).is_empty());
        assert!(m.check(&[dev(22, false)], &s, 5.0).is_empty());
        assert!(
            m.check(&[dev(20, false)], &s, 6.0).is_empty(),
            "cool-down still running"
        );
        assert_eq!(m.check(&[dev(22, false),], &s, 4000.0).len(), 0);
        assert_eq!(m.check(&[dev(18, false)], &s, 4001.0).len(), 1);
    }

    #[test]
    fn full_alert_requires_previous_charging() {
        let mut m = AlertManager::default();
        let s = AlertSettings {
            full_enabled: true,
            ..Default::default()
        };
        assert!(m.check(&[dev(100, false)], &s, 0.0).is_empty());
        assert!(m.check(&[dev(80, true)], &s, 1.0).is_empty());
        assert_eq!(
            m.check(&[dev(100, true)], &s, 2.0),
            [Alert::Full {
                name: "Mouse".into()
            }]
        );
        assert!(m.check(&[dev(100, false)], &s, 3.0).is_empty());
    }

    #[test]
    fn disabled_alerts_stay_silent() {
        let mut m = AlertManager::default();
        let s = AlertSettings {
            low_enabled: false,
            ..Default::default()
        };
        assert!(m.check(&[dev(5, false)], &s, 0.0).is_empty());
    }
}
