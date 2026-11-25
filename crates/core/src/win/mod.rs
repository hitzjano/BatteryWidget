//! Windows back ends: raw HID, WinRT Bluetooth enumeration, game controllers.

pub mod bluetooth;
pub mod hid;
pub mod hidpp;
pub mod jbl_ble;
pub mod vendor;
pub mod xbox;

/// Put the calling worker thread into the multithreaded COM apartment (WinRT needs one).
pub(crate) fn init_winrt_thread() {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    // Failure only means the thread already has an apartment, which is fine.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
}
