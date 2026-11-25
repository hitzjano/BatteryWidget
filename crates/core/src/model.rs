//! Shared data model: what a source reports ([`Reading`]) and what the UI shows ([`Device`]).

use serde::{Deserialize, Serialize};

/// Device category. The declaration order is the display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Keyboard,
    Mouse,
    Headset,
    Gamepad,
    Speaker,
    Phone,
    Tablet,
    Watch,
    Computer,
    Pen,
    Remote,
    Microphone,
    Camera,
    Tv,
    Generic,
}

/// How a device is connected to the PC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Link {
    Bluetooth,
    Lightspeed,
    Bolt,
    Unifying,
    /// Generic 2.4 GHz USB receiver.
    Rf24,
    Usb,
    XboxWireless,
}

impl Link {
    pub fn label(self) -> &'static str {
        match self {
            Link::Bluetooth => "Bluetooth",
            Link::Lightspeed => "LIGHTSPEED",
            Link::Bolt => "Logi Bolt",
            Link::Unifying => "Unifying",
            Link::Rf24 => "2.4 GHz",
            Link::Usb => "USB",
            Link::XboxWireless => "Xbox Wireless",
        }
    }

    pub fn is_bluetooth(self) -> bool {
        self == Link::Bluetooth
    }
}

/// One device as reported by a single source, before merging.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reading {
    /// Stable, source-prefixed identifier (`bt:…`, `logi:…`, `ps:…`).
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// Charge in percent, `None` when the source cannot tell right now.
    pub battery: Option<u8>,
    pub charging: bool,
    pub full: bool,
    pub connected: bool,
    pub link: Link,
    /// Receiver is present but the device stopped answering (asleep).
    pub idle: bool,
    /// Lower-case names under which Windows may list the same device via Bluetooth.
    pub aliases: Vec<String>,
}

impl Reading {
    pub fn new(id: impl Into<String>, name: impl Into<String>, kind: Kind, link: Link) -> Self {
        Reading {
            id: id.into(),
            name: name.into(),
            kind,
            battery: None,
            charging: false,
            full: false,
            connected: true,
            link,
            idle: false,
            aliases: Vec::new(),
        }
    }

    pub fn battery(mut self, level: Option<u8>) -> Self {
        self.battery = level.map(|v| v.min(100));
        self
    }

    pub fn charging(mut self, charging: bool) -> Self {
        self.charging = charging;
        self
    }

    pub fn full(mut self, full: bool) -> Self {
        self.full = full;
        self
    }

    pub fn connected(mut self, connected: bool) -> Self {
        self.connected = connected;
        self
    }

    pub fn aliases(mut self, aliases: &[&str]) -> Self {
        self.aliases = aliases.iter().map(|a| a.to_lowercase()).collect();
        self
    }
}

/// One physical device after merging all sources. Immutable snapshot handed to the UI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub battery: Option<u8>,
    pub kind: Kind,
    pub link: Link,
    pub connected: bool,
    pub charging: bool,
    pub full: bool,
    /// `charging` was inferred from a rising level (Windows does not report it for Bluetooth).
    pub charging_estimated: bool,
    /// Still listed with its last known level, but currently asleep.
    pub idle: bool,
}

impl Device {
    /// Should the device appear in the compact widget?
    pub fn is_active(&self) -> bool {
        self.connected || self.idle
    }

    /// On its charger: charging, or full while still plugged in. Shown with the bolt.
    pub fn plugged_in(&self) -> bool {
        self.charging || self.full
    }
}

/// Clamp any numeric level into a valid percentage.
pub fn clamp_percent(value: f64) -> u8 {
    value.round().clamp(0.0, 100.0) as u8
}
