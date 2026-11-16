//! Xbox and other game controllers through `Windows.Gaming.Input`, with XInput as a
//! fallback for systems where the WinRT API reports nothing.

use std::collections::HashMap;
use std::sync::Arc;

use windows::core::Result;
use windows::Foundation::EventHandler;
use windows::Gaming::Input::RawGameController;
use windows::System::Power::BatteryStatus;
use windows::Win32::UI::Input::XboxController::{
    XInputGetBatteryInformation, BATTERY_DEVTYPE_GAMEPAD, BATTERY_TYPE_ALKALINE, BATTERY_TYPE_NIMH,
    XINPUT_BATTERY_INFORMATION,
};

use super::hid::enumerate;
use crate::model::{clamp_percent, Kind, Link, Reading};
use crate::vendor::playstation::VID_SONY;
use crate::wake::Wake;

const VID_MICROSOFT: u16 = 0x045E;
/// Product ids Xbox controllers use when paired over Bluetooth.
const BLUETOOTH_PIDS: [u16; 6] = [0x02E0, 0x02FD, 0x0B05, 0x0B13, 0x0B21, 0x0B22];
const MODEL_NAMES: [(u16, &str); 12] = [
    (0x028E, "Xbox 360 Controller"),
    (0x028F, "Xbox 360 Wireless Controller"),
    (0x02D1, "Xbox One Controller"),
    (0x02DD, "Xbox One Controller"),
    (0x02E0, "Xbox Wireless Controller"),
    (0x02E3, "Xbox Elite Wireless Controller"),
    (0x02EA, "Xbox One S Controller"),
    (0x02FD, "Xbox Wireless Controller"),
    (0x0B00, "Xbox Elite Wireless Controller Series 2"),
    (0x0B05, "Xbox Elite Wireless Controller Series 2"),
    (0x0B12, "Xbox Wireless Controller"),
    (0x0B13, "Xbox Wireless Controller"),
];
/// Controllers over the Xbox Wireless protocol report exactly 10 % whenever they do not
/// know better (right after connecting, or with AA batteries).
const UNRELIABLE_LEVEL: u8 = 10;

#[derive(Default)]
pub struct Xbox {
    listening: bool,
    /// Last trustworthy level per controller.
    last_levels: HashMap<String, u8>,
}

fn is_generic_name(name: &str) -> bool {
    let n = name.trim().to_lowercase();
    n.is_empty() || n.contains("hid-compliant") || n.contains("game controller")
}

impl Xbox {
    /// Wake the worker when a controller is connected or removed.
    fn listen(&mut self, wake: &Arc<Wake>) {
        if std::mem::replace(&mut self.listening, true) {
            return;
        }
        let (added, removed) = (wake.clone(), wake.clone());
        let _ = RawGameController::RawGameControllerAdded(&EventHandler::<RawGameController>::new(
            move |_, _| {
                added.notify();
                Ok(())
            },
        ));
        let _ = RawGameController::RawGameControllerRemoved(
            &EventHandler::<RawGameController>::new(move |_, _| {
                removed.notify();
                Ok(())
            }),
        );
        // The controller list is populated asynchronously after the first access.
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    pub fn read(&mut self, wake: &Arc<Wake>) -> Vec<Reading> {
        self.listen(wake);
        let controllers = self.read_gaming_input().unwrap_or_else(|error| {
            log::debug!("Windows.Gaming.Input unavailable: {error}");
            Vec::new()
        });
        if controllers.is_empty() {
            read_xinput()
        } else {
            controllers
        }
    }

    fn read_gaming_input(&mut self) -> Result<Vec<Reading>> {
        let controllers = RawGameController::RawGameControllers()?;
        let mut results = Vec::new();
        for controller in (0..controllers.Size()?).filter_map(|i| controllers.GetAt(i).ok()) {
            if let Ok(Some(reading)) = self.describe(&controller) {
                results.push(reading);
            }
        }
        Ok(results)
    }

    fn describe(&mut self, controller: &RawGameController) -> Result<Option<Reading>> {
        let (vid, pid) = (
            controller.HardwareVendorId()?,
            controller.HardwareProductId()?,
        );
        if vid == VID_SONY {
            // PlayStation controllers are read directly, with finer resolution.
            return Ok(None);
        }
        let report = controller.TryGetBatteryReport()?;
        let status = report.Status()?;
        if status == BatteryStatus::NotPresent {
            return Ok(None);
        }
        let capacity = |value: Result<windows::Foundation::IReference<i32>>| {
            value.and_then(|v| v.Value()).ok()
        };
        let full_capacity = capacity(report.FullChargeCapacityInMilliwattHours());
        let remaining = capacity(report.RemainingCapacityInMilliwattHours());
        let mut level = match (remaining, full_capacity) {
            (Some(remaining), Some(full)) if full > 0 => Some(clamp_percent(
                f64::from(remaining) * 100.0 / f64::from(full),
            )),
            _ => None,
        };

        let charging = status == BatteryStatus::Charging;
        let wireless = controller.IsWireless().unwrap_or(false);
        let id = controller
            .NonRoamableId()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let id = id
            .trim_matches(|c: char| c == '\0' || c.is_whitespace())
            .to_string();
        if level == Some(UNRELIABLE_LEVEL) && !charging {
            level = self.last_levels.get(&id).copied();
        } else if let Some(level) = level {
            self.last_levels.insert(id.clone(), level);
        }

        let display_name = controller
            .DisplayName()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let name = match MODEL_NAMES
            .iter()
            .find(|(p, _)| vid == VID_MICROSOFT && *p == pid)
        {
            Some((_, name)) => name.to_string(),
            None if vid == VID_MICROSOFT && wireless => "Xbox Wireless Controller".to_string(),
            None if vid == VID_MICROSOFT => "Xbox Controller".to_string(),
            None if !is_generic_name(&display_name) => display_name.trim().to_string(),
            None => "Gamepad".to_string(),
        };
        let link = if !wireless {
            Link::Usb
        } else if BLUETOOTH_PIDS.contains(&pid)
            || enumerate(|v| v == vid)
                .iter()
                .any(|i| i.pid == pid && i.bluetooth)
        {
            Link::Bluetooth
        } else {
            Link::XboxWireless
        };
        let full = status == BatteryStatus::Idle && level.is_some_and(|l| l >= 99);
        Ok(Some(
            Reading::new(format!("gip:{id}"), name, Kind::Gamepad, link)
                .battery(level)
                .charging(charging)
                .full(full)
                .aliases(&[
                    "xbox wireless controller",
                    "xbox controller",
                    display_name.trim(),
                ]),
        ))
    }
}

/// XInput only knows four coarse levels and no controller names.
fn read_xinput() -> Vec<Reading> {
    let mut results = Vec::new();
    for slot in 0..4u32 {
        let mut info = XINPUT_BATTERY_INFORMATION::default();
        // SAFETY: `info` is a valid out-pointer for the duration of the call.
        if unsafe { XInputGetBatteryInformation(slot, BATTERY_DEVTYPE_GAMEPAD, &mut info) } != 0 {
            continue;
        }
        // Wired pads have no battery; "unknown" and "disconnected" say nothing useful.
        if info.BatteryType != BATTERY_TYPE_ALKALINE && info.BatteryType != BATTERY_TYPE_NIMH {
            continue;
        }
        let percent = match info.BatteryLevel.0 {
            0 => 10,
            1 => 35,
            2 => 70,
            _ => 100,
        };
        let name = if slot == 0 {
            "Xbox Controller".to_string()
        } else {
            format!("Xbox Controller {}", slot + 1)
        };
        results.push(
            Reading::new(
                format!("xinput:{slot}"),
                name,
                Kind::Gamepad,
                Link::XboxWireless,
            )
            .battery(Some(percent))
            .aliases(&["xbox wireless controller", "xbox controller"]),
        );
    }
    results
}
