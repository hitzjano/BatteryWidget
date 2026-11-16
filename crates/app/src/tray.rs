//! Notification-area icon: dynamic battery icon, tooltip and balloon notifications.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO,
    NIIF_RESPECT_QUIET_TIME, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, GetSystemMetrics, HICON, ICONINFO, SM_CXSMICON,
};

const ICON_ID: u32 = 1;

/// Copy `text` into a fixed, NUL-terminated UTF-16 buffer, truncating at a character boundary.
fn copy_text<const N: usize>(target: &mut [u16; N], text: &str) {
    let mut length = 0;
    for unit in text.encode_utf16() {
        if length + 1 >= N {
            break;
        }
        target[length] = unit;
        length += 1;
    }
    // Do not leave half of a surrogate pair behind.
    if length > 0 && (0xD800..0xDC00).contains(&target[length - 1]) {
        length -= 1;
    }
    target[length] = 0;
}

/// Side length of a tray icon at the system DPI.
pub fn icon_size() -> u32 {
    // SAFETY: plain metric query.
    (unsafe { GetSystemMetrics(SM_CXSMICON) }).clamp(16, 64) as u32
}

/// Build an icon from premultiplied BGRA pixels (`size * size * 4` bytes, top-down).
fn make_icon(pixels: &[u8], size: u32) -> Option<HICON> {
    if pixels.len() != (size * size * 4) as usize {
        return None;
    }
    let side = size as i32;
    // The mask is ignored for 32-bit icons but must exist; rows are 16-bit aligned.
    let mask_bits = vec![0u8; (size.div_ceil(16) * 2 * size) as usize];
    // SAFETY: both buffers have exactly the size the bitmaps describe; the bitmaps are
    // deleted after the icon has taken its own copies.
    unsafe {
        let color = CreateBitmap(side, side, 1, 32, Some(pixels.as_ptr().cast()));
        let mask = CreateBitmap(side, side, 1, 1, Some(mask_bits.as_ptr().cast()));
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

pub struct Tray {
    hwnd: HWND,
    callback_message: u32,
    icon: Option<HICON>,
    tooltip: String,
}

impl Tray {
    /// Tray events are delivered to `hwnd` as `callback_message`.
    pub fn new(hwnd: HWND, callback_message: u32) -> Tray {
        Tray {
            hwnd,
            callback_message,
            icon: None,
            tooltip: String::new(),
        }
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            uFlags: NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: self.callback_message,
            ..Default::default()
        };
        if let Some(icon) = self.icon {
            data.uFlags |= NIF_ICON;
            data.hIcon = icon;
        }
        copy_text(&mut data.szTip, &self.tooltip);
        data
    }

    /// (Re-)create the icon. Also called when Explorer restarts and forgets all icons.
    pub fn add(&self) {
        let mut data = self.data();
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        // SAFETY: `data` is fully initialised and outlives the calls.
        unsafe {
            if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
                // Already present (e.g. after a spurious "TaskbarCreated").
                let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
            }
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
        }
    }

    /// Replace image and tooltip.
    pub fn update(&mut self, pixels: &[u8], size: u32, tooltip: &str) {
        let previous = self.icon;
        if let Some(icon) = make_icon(pixels, size) {
            self.icon = Some(icon);
        }
        self.tooltip = tooltip.to_string();
        // SAFETY: `data` is fully initialised; the old icon is destroyed only after the
        // shell has been given the new one.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &self.data());
            if let Some(old) = previous.filter(|old| Some(*old) != self.icon) {
                let _ = DestroyIcon(old);
            }
        }
    }

    /// Show a notification. Windows presents it as a toast and honours focus assist.
    pub fn notify(&self, title: &str, text: &str, warning: bool) {
        let mut data = self.data();
        data.uFlags |= NIF_INFO;
        data.dwInfoFlags = if warning { NIIF_WARNING } else { NIIF_INFO } | NIIF_RESPECT_QUIET_TIME;
        copy_text(&mut data.szInfoTitle, title);
        copy_text(&mut data.szInfo, text);
        // SAFETY: `data` is fully initialised and outlives the call.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        // SAFETY: removes our own icon; the handle is destroyed exactly once.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data());
            if let Some(icon) = self.icon.take() {
                let _ = DestroyIcon(icon);
            }
        }
    }
}
