//! Device discovery and battery protocols for BatteryWidget.
//!
//! The crate is split into pure logic that builds and tests on any platform (naming,
//! merging, HID++ and G HUB protocol handling, alerts) and Windows back ends in `win`
//! that perform the actual I/O. The UI talks to [`Hub`] only.

#![cfg_attr(not(windows), allow(dead_code))]

pub mod alerts;
pub mod bluetooth;
pub mod ghub;
pub mod hid;
pub mod hidpp;
pub mod hub;
pub mod jbl_ble;
pub mod merge;
pub mod model;
pub mod names;
pub mod vendor;
pub mod wake;
#[cfg(windows)]
pub mod win;

pub use alerts::{Alert, AlertManager, AlertSettings};
pub use hub::{Hub, HubOptions};
pub use model::{Device, Kind, Link, Reading};
