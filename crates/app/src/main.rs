//! BatteryWidget: battery levels of wireless peripherals in the Windows taskbar.
//!
//! The portable modules (settings, layout, colours, icon pixels) build and test on any
//! platform; everything that touches Win32 is compiled for Windows only.

#![cfg_attr(windows, windows_subsystem = "windows")]
#![cfg_attr(not(windows), allow(dead_code))]

mod config;
mod icon;
mod layout;
mod theme;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod autostart;
#[cfg(windows)]
mod logger;
#[cfg(windows)]
mod probe;
#[cfg(windows)]
mod render;
#[cfg(windows)]
mod taskbar;
#[cfg(windows)]
mod tray;

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("BatteryWidget only runs on Windows.");
    std::process::exit(1);
}
