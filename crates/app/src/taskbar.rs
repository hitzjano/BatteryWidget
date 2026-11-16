//! Where the taskbar is, which displays exist and whether something is running full screen.

use windows::core::BOOL;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, HDC, HMONITOR,
    MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetClassNameW, GetForegroundWindow, GetWindowLongPtrW,
    GetWindowRect, IsWindowVisible, GWL_STYLE, WS_CAPTION,
};

#[derive(Clone, Copy, Debug)]
pub struct Monitor {
    pub handle: HMONITOR,
    pub rect: RECT,
    pub work: RECT,
    pub dpi: u32,
}

impl Monitor {
    pub fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }
}

fn describe(handle: HMONITOR) -> Option<Monitor> {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    // SAFETY: out-pointers reference live locals of the right type.
    unsafe {
        if !GetMonitorInfoW(handle, &mut info).as_bool() {
            return None;
        }
        let _ = GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
    }
    Some(Monitor {
        handle,
        rect: info.rcMonitor,
        work: info.rcWork,
        dpi: dpi_x.max(48),
    })
}

unsafe extern "system" fn collect(handle: HMONITOR, _: HDC, _: *mut RECT, list: LPARAM) -> BOOL {
    // SAFETY: `list` is the pointer to the Vec passed to EnumDisplayMonitors below, which
    // is alive for the whole (synchronous) enumeration.
    let list = unsafe { &mut *(list.0 as *mut Vec<HMONITOR>) };
    list.push(handle);
    true.into()
}

/// All displays, the primary one first, the others left to right.
pub fn monitors() -> Vec<Monitor> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    // SAFETY: the callback only runs during this call and only touches `handles`.
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut handles as *mut _ as isize),
        );
    }
    let primary = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    let mut list: Vec<Monitor> = handles.into_iter().filter_map(describe).collect();
    list.sort_by_key(|m| (m.handle != primary, m.rect.left, m.rect.top));
    list
}

/// The display with this index (0 = primary), or the primary one if it does not exist.
pub fn monitor(index: usize) -> Option<Monitor> {
    let all = monitors();
    all.get(index).or(all.first()).copied()
}

#[derive(Clone, Copy, Debug)]
pub struct Taskbar {
    pub hwnd: HWND,
    pub rect: RECT,
    /// Left edge of the notification area (clock, tray icons), if it could be measured.
    pub tray_left: Option<i32>,
}

impl Taskbar {
    pub fn is_horizontal(&self) -> bool {
        self.rect.right - self.rect.left > self.rect.bottom - self.rect.top
    }

    /// An auto-hiding taskbar slides off the screen, leaving a strip of a few pixels.
    pub fn is_hidden(&self, monitor: &Monitor) -> bool {
        let visible_height =
            self.rect.bottom.min(monitor.rect.bottom) - self.rect.top.max(monitor.rect.top);
        let visible_width =
            self.rect.right.min(monitor.rect.right) - self.rect.left.max(monitor.rect.left);
        visible_height < (self.rect.bottom - self.rect.top) / 2
            || visible_width < (self.rect.right - self.rect.left) / 2
    }
}

fn window_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    // SAFETY: `rect` is a valid out-pointer.
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok().map(|_| rect)
}

fn child(parent: HWND, class: PCWSTR) -> Option<HWND> {
    // SAFETY: plain window lookup.
    unsafe { FindWindowExW(Some(parent), None, class, PCWSTR::null()) }
        .ok()
        .filter(|h| !h.is_invalid())
}

/// The taskbar on `monitor`: `Shell_TrayWnd` on the primary display,
/// a `Shell_SecondaryTrayWnd` on the others.
pub fn find(monitor: &Monitor) -> Option<Taskbar> {
    let on_monitor =
        |hwnd: HWND| unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) } == monitor.handle;
    // SAFETY: plain window lookups; handles are only used while valid for queries.
    unsafe {
        if let Ok(primary) = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) {
            if !primary.is_invalid() && on_monitor(primary) {
                let tray_left = child(primary, w!("TrayNotifyWnd"))
                    .and_then(window_rect)
                    .map(|r| r.left);
                return window_rect(primary).map(|rect| Taskbar {
                    hwnd: primary,
                    rect,
                    tray_left,
                });
            }
        }
        let mut previous: Option<HWND> = None;
        while let Ok(secondary) =
            FindWindowExW(None, previous, w!("Shell_SecondaryTrayWnd"), PCWSTR::null())
        {
            if secondary.is_invalid() {
                break;
            }
            if on_monitor(secondary) {
                return window_rect(secondary).map(|rect| Taskbar {
                    hwnd: secondary,
                    rect,
                    tray_left: None,
                });
            }
            previous = Some(secondary);
        }
    }
    None
}

fn registry_dword(key: PCWSTR, value: PCWSTR) -> Option<u32> {
    let mut data = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: `data` is a u32 and `size` says so.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            value,
            RRF_RT_REG_DWORD,
            None,
            Some((&mut data as *mut u32).cast()),
            Some(&mut size),
        )
    };
    status.is_ok().then_some(data)
}

/// Does the taskbar use the light theme? (Windows default is dark.)
pub fn taskbar_is_light() -> bool {
    registry_dword(
        w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
        w!("SystemUsesLightTheme"),
    ) == Some(1)
}

fn class_name(hwnd: HWND) -> String {
    let mut buffer = [0u16; 64];
    // SAFETY: the slice length bounds what the function writes.
    let length = unsafe { GetClassNameW(hwnd, &mut buffer) }.max(0) as usize;
    String::from_utf16_lossy(&buffer[..length.min(buffer.len())])
}

/// Is a full-screen application (game, video, presentation) in the foreground on `monitor`?
///
/// A maximised window does not count: it has a caption and leaves the taskbar visible.
pub fn fullscreen_app_on(monitor: &Monitor, own: HWND) -> bool {
    // SAFETY: window queries on a handle that may vanish at any time; every call copes
    // with an invalid handle by failing.
    unsafe {
        let foreground = GetForegroundWindow();
        if foreground.is_invalid() || foreground == own || !IsWindowVisible(foreground).as_bool() {
            return false;
        }
        if matches!(
            class_name(foreground).as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        ) {
            return false;
        }
        if MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST) != monitor.handle {
            return false;
        }
        let Some(rect) = window_rect(foreground) else {
            return false;
        };
        let covers = rect.left <= monitor.rect.left
            && rect.top <= monitor.rect.top
            && rect.right >= monitor.rect.right
            && rect.bottom >= monitor.rect.bottom;
        let style = GetWindowLongPtrW(foreground, GWL_STYLE) as u32;
        covers && style & WS_CAPTION.0 != WS_CAPTION.0
    }
}
