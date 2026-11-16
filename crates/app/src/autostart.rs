//! "Start with Windows" through the per-user `Run` registry key. No service, no
//! scheduled task, no administrator rights.

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const VALUE: PCWSTR = w!("BatteryWidget");
/// Passed on automatic start, so that the program stays quiet.
pub const AUTOSTART_FLAG: &str = "--autostart";

fn command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" {AUTOSTART_FLAG}", exe.display()))
}

fn registered_command() -> Option<String> {
    let mut buffer = [0u16; 1024];
    let mut size = size_of_val(&buffer) as u32;
    // SAFETY: `size` is the buffer size in bytes; RRF_RT_REG_SZ guarantees NUL termination.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    status.is_ok().then(|| {
        let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        String::from_utf16_lossy(&buffer[..end])
    })
}

/// Enabled, and pointing at this very executable?
pub fn is_enabled() -> bool {
    registered_command().is_some_and(|registered| {
        command().is_some_and(|own| registered.eq_ignore_ascii_case(&own))
    })
}

pub fn set_enabled(enable: bool) -> bool {
    // SAFETY: all strings are NUL-terminated wide strings that outlive the calls.
    unsafe {
        if !enable {
            let status = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE);
            return status.is_ok() || registered_command().is_none();
        }
        let Some(command) = command() else {
            return false;
        };
        let wide = HSTRING::from(command);
        // Size in bytes including the terminating NUL.
        let bytes = (wide.len() + 1) * size_of::<u16>();
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
            REG_SZ.0,
            Some(wide.as_ptr().cast()),
            bytes as u32,
        )
        .is_ok()
    }
}

/// If the executable was moved, keep the autostart entry pointing at it.
pub fn refresh_path() {
    if registered_command().is_some() && !is_enabled() {
        set_enabled(true);
    }
}
