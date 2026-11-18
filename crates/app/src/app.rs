//! The application: one layered window (the widget), the tray icon, the menu and the
//! glue between the device hub and the screen.

use std::cell::RefCell;
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use battery_widget_core::{Alert, AlertManager, Device, Hub, Kind, Link};
use windows::core::{w, HSTRING, PCSTR, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::System::LibraryLoader::{
    GetModuleHandleW, GetProcAddress, LoadLibraryExW, SetDefaultDllDirectories,
    LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentThreadId};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::autostart;
use crate::config::{self, Config, Position, ThemeMode, APP_NAME, LOW_THRESHOLDS};
use crate::icon;
use crate::layout::{self, Display, Layout};
use crate::probe::Probe;
use crate::render::Renderer;
use crate::taskbar::{self, Monitor, Taskbar};
use crate::theme;
use crate::tray::{self, Tray};

const CLASS_NAME: PCWSTR = w!("BatteryWidget.Window");
const WM_APP_DEVICES: u32 = WM_APP + 1;
const WM_APP_TRAY: u32 = WM_APP + 2;
const WM_APP_FOREGROUND: u32 = WM_APP + 3;
const TIMER_TICK: usize = 1;
/// The taskbar moves, hides and changes theme without telling anyone, so the placement
/// is re-checked periodically. A check that finds nothing changed draws nothing.
const TICK_MS: u32 = 1500;
/// Tray callback events (NOTIFYICON_VERSION_4).
const NIN_SELECT: u32 = WM_USER;
const NIN_KEYSELECT: u32 = WM_USER + 1;
/// Inside the taskbar the widget never takes more than this share of the display width.
const MAX_WIDTH_SHARE: f32 = 0.45;
/// Outside the taskbar there is nothing to collide with.
const MAX_FLOATING_SHARE: f32 = 0.9;
/// Distance kept from neighbours inside the taskbar (device-independent pixels).
const TASKBAR_MARGIN: f32 = 8.0;
/// A gap between taskbar buttons narrower than this is not worth using.
const MIN_SLOT: f32 = 110.0;

const CMD_TOGGLE_WIDGET: usize = 100;
const CMD_SHOW_NAMES: usize = 101;
const CMD_AUTOSTART: usize = 102;
const CMD_REFRESH: usize = 103;
const CMD_OPEN_FOLDER: usize = 104;
const CMD_EXIT: usize = 105;
const CMD_NOTIFY_LOW: usize = 106;
const CMD_NOTIFY_FULL: usize = 107;
const CMD_POSITION: usize = 200;
const CMD_THEME: usize = 220;
const CMD_THRESHOLD: usize = 240;
const CMD_SOURCE: usize = 260;
const CMD_MONITOR: usize = 280;
const CMD_DISPLAY: usize = 300;

const SOURCE_LABELS: [&str; 5] = [
    "Bluetooth",
    "Logitech receivers (HID++)",
    "Logitech G HUB",
    "Other brands (USB receivers)",
    "Game controllers",
];

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}
static MAIN_WINDOW: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
/// Command id of the menu item currently highlighted (0 = none), for the Enter key.
static MENU_SELECTION: AtomicU32 = AtomicU32::new(0);
/// Root of the context menu while it is open.
static MENU_ROOT: AtomicIsize = AtomicIsize::new(0);
/// Window message a menu window answers with its `HMENU`.
const MN_GETHMENU: u32 = 0x01E1;
const VK_RETURN: usize = 0x0D;

/// Run `f` on the application state. Returns `None` if the state is already in use
/// further up the stack (a re-entrant window message), in which case the message is
/// simply dropped: every handler is idempotent and the next tick catches up.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| {
        cell.try_borrow_mut()
            .ok()
            .and_then(|mut app| app.as_mut().map(f))
    })
}

/// Everything that determines how the widget is drawn (as opposed to where).
#[derive(Clone, Copy, PartialEq)]
struct Style {
    scale: f32,
    light: bool,
    solid: bool,
    max_width: f32,
    show_names: bool,
    display: Display,
    low_threshold: u8,
}

struct App {
    hwnd: HWND,
    config: Config,
    config_path: PathBuf,
    /// Latest snapshot from the hub, and the subset shown in the widget.
    devices: Vec<Device>,
    shown: Vec<Device>,
    inbox: Arc<Mutex<Option<Vec<Device>>>>,
    hub: Option<Hub>,
    alerts: AlertManager,
    started: Instant,
    renderer: Renderer,
    tray: Tray,
    style: Option<Style>,
    layout: Layout,
    /// The content changed and has to be drawn again.
    dirty: bool,
    visible: bool,
    origin: POINT,
    dragging: bool,
    probe: Arc<Probe>,
    /// Free stretch of the taskbar the widget currently lives in (taskbar modes).
    slot: Option<(i32, i32)>,
    /// Show sample devices instead of scanning (`--demo`), for screenshots and UI work.
    demo: bool,
}

/// Sample devices covering every visual state.
fn demo_devices() -> Vec<Device> {
    let device = |name: &str, kind, link, battery, charging: bool, idle: bool| Device {
        id: format!("demo:{name}"),
        name: name.to_string(),
        battery,
        kind,
        link,
        connected: true,
        charging,
        full: false,
        charging_estimated: false,
        idle,
    };
    vec![
        device(
            "MX Keys",
            Kind::Keyboard,
            Link::Bolt,
            Some(42),
            false,
            false,
        ),
        device(
            "MX Master 3S",
            Kind::Mouse,
            Link::Bolt,
            Some(85),
            false,
            false,
        ),
        device(
            "WH-1000XM4",
            Kind::Headset,
            Link::Bluetooth,
            Some(60),
            true,
            false,
        ),
        device(
            "DualSense",
            Kind::Gamepad,
            Link::Usb,
            Some(15),
            false,
            false,
        ),
        device("G502", Kind::Mouse, Link::Lightspeed, Some(70), false, true),
    ]
}

impl App {
    fn save(&self) {
        if let Err(error) = self.config.save(&self.config_path) {
            log::warn!("could not save settings: {error}");
        }
    }

    fn start_hub(&mut self) {
        // Stop the old workers first; the aggregator is joined, so no stale update follows.
        self.hub = None;
        let inbox = self.inbox.clone();
        let window = self.hwnd.0 as isize;
        if self.demo {
            *inbox.lock().unwrap_or_else(|p| p.into_inner()) = Some(demo_devices());
            // SAFETY: posting to our own window.
            unsafe {
                let _ = PostMessageW(Some(self.hwnd), WM_APP_DEVICES, WPARAM(0), LPARAM(0));
            }
            return;
        }
        self.hub = Some(Hub::start(self.config.hub_options(), move |devices| {
            *inbox.lock().unwrap_or_else(|p| p.into_inner()) = Some(devices);
            // SAFETY: posting to a window handle is safe from any thread; if the window
            // is gone the call just fails.
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(window as *mut c_void)),
                    WM_APP_DEVICES,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }));
    }

    fn on_devices(&mut self) {
        let Some(devices) = self.inbox.lock().unwrap_or_else(|p| p.into_inner()).take() else {
            return;
        };
        let now = self.started.elapsed().as_secs_f64();
        let alerts = self
            .alerts
            .check(&devices, &self.config.alert_settings(), now);
        self.devices = devices;
        self.shown = self
            .devices
            .iter()
            .filter(|d| d.is_active())
            .cloned()
            .collect();
        self.dirty = true;
        self.update_tray();
        self.refresh();
        self.announce(&alerts);
    }

    fn announce(&self, alerts: &[Alert]) {
        let low: Vec<String> = alerts
            .iter()
            .filter_map(|a| match a {
                Alert::Low { name, level } => Some(format!("{name}: {level}%")),
                Alert::Full { .. } => None,
            })
            .collect();
        let full: Vec<&str> = alerts
            .iter()
            .filter_map(|a| match a {
                Alert::Full { name } => Some(name.as_str()),
                Alert::Low { .. } => None,
            })
            .collect();
        if !low.is_empty() {
            self.tray.notify("Battery low", &low.join("\n"), true);
        } else if !full.is_empty() {
            self.tray.notify("Fully charged", &full.join("\n"), false);
        }
    }

    fn light_theme(&self) -> bool {
        match self.config.theme {
            ThemeMode::Auto => taskbar::taskbar_is_light(),
            ThemeMode::Dark => false,
            ThemeMode::Light => true,
        }
    }

    fn update_tray(&mut self) {
        let summary = icon::summary_level(&self.devices);
        let size = tray::icon_size();
        let pixels = icon::battery_icon(
            size,
            summary.map(|(level, _)| level),
            summary.is_some_and(|(_, charging)| charging),
            taskbar::taskbar_is_light(),
            self.config.low_threshold,
        );
        let mut tooltip = APP_NAME.to_string();
        for device in &self.shown {
            let line = format!("\n{}: {}", device.name, layout::percent_label(device));
            // The shell limits tooltips to 127 characters.
            if tooltip.encode_utf16().count() + line.encode_utf16().count() > 120 {
                tooltip.push_str("\n…");
                break;
            }
            tooltip.push_str(&line);
        }
        self.tray.update(&pixels, size, &tooltip);
    }

    fn hide(&mut self) {
        if std::mem::replace(&mut self.visible, false) {
            // SAFETY: our own window.
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
        }
    }

    /// Right end of the space usable in the taskbar: the left edge of the notification area.
    fn tray_anchor(monitor: &Monitor, bar: &Taskbar) -> i32 {
        // Secondary taskbars have no measurable notification area; leave room for the
        // clock they show.
        bar.tray_left
            .unwrap_or(bar.rect.right - (120.0 * monitor.scale()).round() as i32)
    }

    /// The stretch of taskbar the widget may use, given where its buttons are: the gap
    /// in front of the notification area, or the first sufficiently large gap from the
    /// left (which skips the weather button of a centred Windows 11 taskbar).
    fn taskbar_slot(
        &self,
        monitor: &Monitor,
        bar: &Taskbar,
        occupied: &[(i32, i32)],
    ) -> (i32, i32) {
        let anchor = Self::tray_anchor(monitor, bar);
        let useful = (MIN_SLOT * monitor.scale()).round() as i32;
        let gaps = layout::free_gaps(bar.rect.left, anchor, occupied);
        let mut usable = gaps
            .iter()
            .copied()
            .filter(|(left, right)| right - left >= useful);
        let chosen = match self.config.position {
            Position::TaskbarLeft => usable.next(),
            _ => usable.next_back(),
        };
        // No decent gap: take the widest there is rather than nothing.
        chosen
            .or_else(|| {
                gaps.iter()
                    .copied()
                    .max_by_key(|(left, right)| right - left)
            })
            .unwrap_or((anchor, anchor))
    }

    /// Top-left corner of the widget for the current settings.
    fn place(&self, monitor: &Monitor, bar: Option<&Taskbar>) -> POINT {
        let (width, height) = (
            self.layout.width.ceil() as i32,
            self.layout.height.ceil() as i32,
        );
        let px = |dip: f32| (dip * monitor.scale()).round() as i32;
        let offset = px(self.config.offset_x as f32);
        let work = monitor.work;
        let edge = px(12.0);
        let corner = |right: bool, bottom: bool| POINT {
            x: if right {
                work.right - width - edge
            } else {
                work.left + edge
            },
            y: if bottom {
                work.bottom - height - edge
            } else {
                work.top + edge
            },
        };
        let mut point = match (self.config.position, bar) {
            (Position::TaskbarRight, Some(bar)) => POINT {
                x: self
                    .slot
                    .map_or(Self::tray_anchor(monitor, bar), |(_, right)| right)
                    - width
                    - px(TASKBAR_MARGIN)
                    - offset,
                y: bar.rect.top + (bar.rect.bottom - bar.rect.top - height) / 2,
            },
            (Position::TaskbarLeft, Some(bar)) => POINT {
                x: self.slot.map_or(bar.rect.left, |(left, _)| left) + px(TASKBAR_MARGIN) + offset,
                y: bar.rect.top + (bar.rect.bottom - bar.rect.top - height) / 2,
            },
            (Position::Free, _) => {
                return self.clamp_to_desktop(POINT {
                    x: self.config.free_x,
                    y: self.config.free_y,
                })
            }
            (Position::TopLeft, _) => corner(false, false),
            (Position::TopRight, _) => corner(true, false),
            (Position::BottomLeft, _) => corner(false, true),
            // Also the fallback when a taskbar mode is selected but there is no
            // horizontal taskbar on that display.
            (Position::BottomRight | Position::TaskbarLeft | Position::TaskbarRight, _) => {
                corner(true, true)
            }
        };
        point.x = point.x.clamp(
            monitor.rect.left,
            (monitor.rect.right - width).max(monitor.rect.left),
        );
        point
    }

    /// Keep a dragged widget reachable even if the display layout changed since.
    fn clamp_to_desktop(&self, point: POINT) -> POINT {
        // SAFETY: plain metric queries.
        let (left, top, width, height) = unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            )
        };
        let (w, h) = (
            self.layout.width.ceil() as i32,
            self.layout.height.ceil() as i32,
        );
        POINT {
            x: point.x.clamp(left, (left + width - w).max(left)),
            y: point.y.clamp(top, (top + height - h).max(top)),
        }
    }

    /// Bring the window in line with devices, settings and the state of the desktop.
    /// Cheap when nothing changed: no drawing, at most one `SetWindowPos`.
    fn refresh(&mut self) {
        if self.dragging {
            return;
        }
        let Some(monitor) = taskbar::monitor(self.config.monitor) else {
            return self.hide();
        };
        let bar = taskbar::find(&monitor).filter(Taskbar::is_horizontal);
        let in_taskbar = self.config.position.in_taskbar() && bar.is_some();
        // Keep clear of the taskbar buttons: the widget only gets the gap they leave.
        let watched = bar
            .as_ref()
            .filter(|_| in_taskbar && self.config.show_widget && !self.shown.is_empty());
        self.probe
            .watch(watched.map(|b| (b.hwnd, Self::tray_anchor(&monitor, b))));
        let share = if in_taskbar {
            MAX_WIDTH_SHARE
        } else {
            MAX_FLOATING_SHARE
        };
        let mut max_width = (monitor.rect.right - monitor.rect.left) as f32 * share;
        self.slot = None;
        if let Some(bar) = watched {
            match self.probe.spans() {
                Some(occupied) => {
                    let slot = self.taskbar_slot(&monitor, bar, &occupied);
                    let margin = (TASKBAR_MARGIN * monitor.scale()).round() as i32;
                    let offset = (self.config.offset_x as f32 * monitor.scale()).round() as i32;
                    let free = slot.1 - slot.0 - 2 * margin - offset.max(0);
                    max_width = max_width.min(free.max(0) as f32);
                    self.slot = Some(slot);
                }
                // Not measured yet: showing the widget now could put it on top of
                // taskbar buttons. The probe reports back within a moment.
                None if !self.probe.is_unavailable() => return self.hide(),
                None => {}
            }
        }
        let style = Style {
            scale: monitor.scale(),
            light: self.light_theme(),
            solid: !in_taskbar,
            max_width,
            show_names: self.config.show_names,
            display: self.config.display,
            low_threshold: self.config.low_threshold,
        };
        if self.dirty || self.style != Some(style) {
            self.style = Some(style);
            self.dirty = true;
            self.layout = match self.renderer.label_format(style.scale) {
                Ok(format) => layout::compute(
                    &self.shown,
                    style.show_names,
                    style.display,
                    style.max_width,
                    style.scale,
                    &|text| self.renderer.text_width(&format, text),
                ),
                Err(error) => {
                    log::error!("DirectWrite unavailable: {error}");
                    Layout::default()
                }
            };
        }

        let taskbar_hidden = in_taskbar && bar.is_some_and(|b| b.is_hidden(&monitor));
        let show = self.config.show_widget
            && !self.layout.pills.is_empty()
            && !taskbar_hidden
            && !taskbar::fullscreen_app_on(&monitor, self.hwnd);
        if !show {
            return self.hide();
        }

        let origin = self.place(&monitor, bar.as_ref());
        let moved = origin.x != self.origin.x || origin.y != self.origin.y;
        self.origin = origin;
        if self.dirty {
            let palette = theme::palette(style.light, style.solid);
            match self.renderer.present(
                self.hwnd,
                origin,
                &self.layout,
                &self.shown,
                &palette,
                style.low_threshold,
            ) {
                Ok(()) => self.dirty = false,
                Err(error) => log::error!("drawing failed: {error}"),
            }
        }
        // SAFETY: our own window. The taskbar is topmost too and raises itself when
        // clicked, so the widget re-asserts its place on every pass.
        unsafe {
            let mut flags = SWP_NOACTIVATE | SWP_NOSIZE;
            if !moved {
                flags |= SWP_NOMOVE;
            }
            if !self.visible {
                flags |= SWP_SHOWWINDOW;
            }
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                origin.x,
                origin.y,
                0,
                0,
                flags,
            );
        }
        self.visible = true;
    }

    /// The user finished dragging: remember where the widget was dropped.
    fn end_drag(&mut self) {
        self.dragging = false;
        let mut rect = RECT::default();
        // SAFETY: valid out-pointer, our own window.
        if unsafe { GetWindowRect(self.hwnd, &mut rect) }.is_ok() {
            (self.config.free_x, self.config.free_y) = (rect.left, rect.top);
            self.origin = POINT {
                x: rect.left,
                y: rect.top,
            };
            self.save();
        }
    }

    fn build_menu(&self) -> Option<HMENU> {
        let add = |menu: HMENU, flags: MENU_ITEM_FLAGS, id: usize, text: &str| {
            // SAFETY: the menu handle is valid and the string outlives the call.
            unsafe {
                let _ = AppendMenuW(menu, flags, id, &HSTRING::from(text));
            }
        };
        let check = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
        let separator = |menu: HMENU| add(menu, MF_SEPARATOR, 0, "");
        let submenu = |parent: HMENU, title: &str| -> Option<HMENU> {
            // SAFETY: a sub-menu attached to a parent is destroyed together with it.
            let menu = unsafe { CreatePopupMenu() }.ok()?;
            add(parent, MF_POPUP, menu.0 as usize, title);
            Some(menu)
        };

        // SAFETY: destroyed by the caller after use.
        let menu = unsafe { CreatePopupMenu() }.ok()?;
        if self.devices.is_empty() {
            add(menu, MF_STRING | MF_DISABLED, 0, "No devices found");
        }
        for device in &self.devices {
            let state = if device.idle {
                " · asleep"
            } else if !device.connected {
                " · not connected"
            } else if device.full {
                " · full"
            } else if device.charging {
                " · charging"
            } else {
                ""
            };
            let line = format!(
                "{}\t{}{} · {}",
                device.name,
                layout::percent_label(device),
                state,
                device.link.label()
            );
            add(menu, MF_STRING | MF_DISABLED, 0, &line);
        }
        separator(menu);
        add(
            menu,
            MF_STRING | check(self.config.show_widget),
            CMD_TOGGLE_WIDGET,
            "Show widget",
        );
        add(
            menu,
            MF_STRING | check(self.config.show_names),
            CMD_SHOW_NAMES,
            "Show device names",
        );

        let style = submenu(menu, "Style")?;
        for (index, (value, label)) in Display::ALL.iter().enumerate() {
            let flags = MF_STRING | check(self.config.display == *value);
            add(style, flags, CMD_DISPLAY + index, label);
        }
        let position = submenu(menu, "Position")?;
        for (index, (value, label)) in Position::ALL.iter().enumerate() {
            add(
                position,
                MF_STRING | check(self.config.position == *value),
                CMD_POSITION + index,
                label,
            );
        }
        let monitors = taskbar::monitors();
        if monitors.len() > 1 {
            let display = submenu(menu, "Display")?;
            for index in 0..monitors.len() {
                let label = if index == 0 {
                    "Primary display".to_string()
                } else {
                    format!("Display {}", index + 1)
                };
                add(
                    display,
                    MF_STRING | check(self.config.monitor == index),
                    CMD_MONITOR + index,
                    &label,
                );
            }
        }
        let appearance = submenu(menu, "Theme")?;
        for (index, (value, label)) in ThemeMode::ALL.iter().enumerate() {
            add(
                appearance,
                MF_STRING | check(self.config.theme == *value),
                CMD_THEME + index,
                label,
            );
        }

        let notifications = submenu(menu, "Notifications")?;
        add(
            notifications,
            MF_STRING | check(self.config.notify_low),
            CMD_NOTIFY_LOW,
            "Battery low",
        );
        add(
            notifications,
            MF_STRING | check(self.config.notify_full),
            CMD_NOTIFY_FULL,
            "Fully charged",
        );
        separator(notifications);
        for (index, threshold) in LOW_THRESHOLDS.iter().enumerate() {
            let label = format!("Low at {threshold}%");
            add(
                notifications,
                MF_STRING | check(self.config.low_threshold == *threshold),
                CMD_THRESHOLD + index,
                &label,
            );
        }

        let sources = submenu(menu, "Sources")?;
        let s = &self.config.sources;
        let enabled = [
            s.bluetooth,
            s.logitech_hidpp,
            s.logitech_ghub,
            s.vendor_hid,
            s.game_controllers,
        ];
        for (index, label) in SOURCE_LABELS.iter().enumerate() {
            add(
                sources,
                MF_STRING | check(enabled[index]),
                CMD_SOURCE + index,
                label,
            );
        }

        separator(menu);
        add(
            menu,
            MF_STRING | check(autostart::is_enabled()),
            CMD_AUTOSTART,
            "Start with Windows",
        );
        add(menu, MF_STRING, CMD_REFRESH, "Refresh now");
        add(menu, MF_STRING, CMD_OPEN_FOLDER, "Open settings folder");
        separator(menu);
        add(
            menu,
            MF_STRING | MF_DISABLED,
            0,
            &format!("{APP_NAME} {}", env!("CARGO_PKG_VERSION")),
        );
        add(menu, MF_STRING, CMD_EXIT, "Exit");
        Some(menu)
    }

    /// Check mark of a menu command; `None` for commands that are not settings.
    fn checked(&self, id: usize) -> Option<bool> {
        let c = &self.config;
        let s = &c.sources;
        let pick = |base: usize, len: usize| (base..base + len).contains(&id).then(|| id - base);
        Some(match id {
            CMD_TOGGLE_WIDGET => c.show_widget,
            CMD_SHOW_NAMES => c.show_names,
            CMD_NOTIFY_LOW => c.notify_low,
            CMD_NOTIFY_FULL => c.notify_full,
            CMD_AUTOSTART => autostart::is_enabled(),
            _ => {
                if let Some(i) = pick(CMD_POSITION, Position::ALL.len()) {
                    c.position == Position::ALL[i].0
                } else if let Some(i) = pick(CMD_DISPLAY, Display::ALL.len()) {
                    c.display == Display::ALL[i].0
                } else if let Some(i) = pick(CMD_THEME, ThemeMode::ALL.len()) {
                    c.theme == ThemeMode::ALL[i].0
                } else if let Some(i) = pick(CMD_THRESHOLD, LOW_THRESHOLDS.len()) {
                    c.low_threshold == LOW_THRESHOLDS[i]
                } else if let Some(i) = pick(CMD_SOURCE, SOURCE_LABELS.len()) {
                    [
                        s.bluetooth,
                        s.logitech_hidpp,
                        s.logitech_ghub,
                        s.vendor_hid,
                        s.game_controllers,
                    ][i]
                } else {
                    c.monitor == pick(CMD_MONITOR, 16)?
                }
            }
        })
    }

    /// Bring the check marks of an open menu in line with the settings.
    fn sync_checks(&self, menu: HMENU) {
        // SAFETY: `menu` is the open context menu or one of its sub-menus.
        unsafe {
            for position in 0..GetMenuItemCount(Some(menu)).max(0) {
                let sub = GetSubMenu(menu, position);
                if !sub.is_invalid() {
                    self.sync_checks(sub);
                } else if let Some(on) = self.checked(GetMenuItemID(menu, position) as usize) {
                    let state = if on { MF_CHECKED } else { MF_UNCHECKED };
                    CheckMenuItem(menu, position as u32, (MF_BYPOSITION | state).0);
                }
            }
        }
    }

    /// Execute a menu command. Returns `true` if the application should exit.
    fn command(&mut self, id: usize) -> bool {
        let mut restart_hub = false;
        match id {
            CMD_EXIT => return true,
            CMD_TOGGLE_WIDGET => self.config.show_widget = !self.config.show_widget,
            CMD_SHOW_NAMES => self.config.show_names = !self.config.show_names,
            CMD_NOTIFY_LOW => self.config.notify_low = !self.config.notify_low,
            CMD_NOTIFY_FULL => self.config.notify_full = !self.config.notify_full,
            CMD_AUTOSTART => {
                if !autostart::set_enabled(!autostart::is_enabled()) {
                    log::warn!("could not change the autostart entry");
                }
            }
            CMD_REFRESH => {
                if let Some(hub) = &self.hub {
                    hub.refresh();
                }
            }
            CMD_OPEN_FOLDER => {
                let folder = HSTRING::from(config::data_dir().as_os_str());
                // SAFETY: all strings outlive the call.
                unsafe {
                    ShellExecuteW(
                        None,
                        w!("open"),
                        &folder,
                        PCWSTR::null(),
                        PCWSTR::null(),
                        SW_SHOWNORMAL,
                    );
                }
            }
            _ if (CMD_POSITION..CMD_POSITION + Position::ALL.len()).contains(&id) => {
                let position = Position::ALL[id - CMD_POSITION].0;
                if position == Position::Free
                    && self.config.position != Position::Free
                    && self.visible
                {
                    // Start dragging from where the widget currently is.
                    (self.config.free_x, self.config.free_y) = (self.origin.x, self.origin.y);
                }
                self.config.position = position;
            }
            _ if (CMD_DISPLAY..CMD_DISPLAY + Display::ALL.len()).contains(&id) => {
                self.config.display = Display::ALL[id - CMD_DISPLAY].0;
            }
            _ if (CMD_THEME..CMD_THEME + ThemeMode::ALL.len()).contains(&id) => {
                self.config.theme = ThemeMode::ALL[id - CMD_THEME].0;
            }
            _ if (CMD_THRESHOLD..CMD_THRESHOLD + LOW_THRESHOLDS.len()).contains(&id) => {
                self.config.low_threshold = LOW_THRESHOLDS[id - CMD_THRESHOLD];
            }
            _ if (CMD_SOURCE..CMD_SOURCE + SOURCE_LABELS.len()).contains(&id) => {
                let s = &mut self.config.sources;
                let flag = match id - CMD_SOURCE {
                    0 => &mut s.bluetooth,
                    1 => &mut s.logitech_hidpp,
                    2 => &mut s.logitech_ghub,
                    3 => &mut s.vendor_hid,
                    _ => &mut s.game_controllers,
                };
                *flag = !*flag;
                restart_hub = true;
            }
            _ if (CMD_MONITOR..CMD_MONITOR + 16).contains(&id) => {
                self.config.monitor = id - CMD_MONITOR
            }
            _ => return false,
        }
        self.save();
        if restart_hub {
            self.start_hub();
        }
        self.dirty = true;
        self.update_tray();
        self.refresh();
        false
    }
}

/// Apply a setting chosen in the open menu without closing it. Returns `false` if `id`
/// is not a setting (the menu then handles the click as usual and closes).
fn apply_in_open_menu(id: usize) -> bool {
    let applied = with_app(|app| {
        if app.checked(id).is_none() {
            return false;
        }
        app.command(id);
        app.sync_checks(HMENU(MENU_ROOT.load(Ordering::Relaxed) as *mut c_void));
        true
    });
    if applied != Some(true) {
        return false;
    }
    unsafe extern "system" fn repaint(window: HWND, _: LPARAM) -> windows::core::BOOL {
        let mut class = [0u16; 16];
        // SAFETY: the buffer length bounds the write; repainting any window is harmless.
        unsafe {
            let length = GetClassNameW(window, &mut class).max(0) as usize;
            if String::from_utf16_lossy(&class[..length]) == "#32768" {
                let _ = InvalidateRect(Some(window), None, true);
            }
        }
        true.into()
    }
    // SAFETY: enumerates this thread's own windows; menu windows have class #32768.
    unsafe {
        let _ = EnumThreadWindows(GetCurrentThreadId(), Some(repaint), LPARAM(0));
    }
    true
}

/// Message filter that runs while the context menu is open. A normal menu closes on
/// every click; settings should not, so clicks (and Enter) on a setting are applied
/// here and then withheld from the menu.
unsafe extern "system" fn menu_filter(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == MSGF_MENU as i32 {
        // SAFETY: for MSGF_MENU, lparam points to the MSG being processed.
        let message = unsafe { &*(lparam.0 as *const MSG) };
        let target = match message.message {
            WM_LBUTTONUP => unsafe {
                // While a menu is open the mouse is captured by its owner, so the message
                // does not name the menu window; the cursor position does.
                let under_cursor = WindowFromPoint(message.pt);
                let menu =
                    HMENU(SendMessageW(under_cursor, MN_GETHMENU, None, None).0 as *mut c_void);
                let position = if menu.is_invalid() {
                    -1
                } else {
                    MenuItemFromPoint(None, menu, message.pt)
                };
                let enabled = position >= 0
                    && GetMenuState(menu, position as u32, MF_BYPOSITION)
                        & (MF_DISABLED | MF_GRAYED).0
                        == 0;
                if enabled {
                    GetMenuItemID(menu, position) as usize
                } else {
                    0
                }
            },
            WM_KEYDOWN if message.wParam.0 == VK_RETURN => {
                MENU_SELECTION.load(Ordering::Relaxed) as usize
            }
            _ => 0,
        };
        if target != 0 && target != u32::MAX as usize && apply_in_open_menu(target) {
            return LRESULT(1);
        }
    }
    // SAFETY: passing the message on unchanged.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Show the context menu at the cursor and run the chosen command.
fn show_menu(hwnd: HWND) {
    // The menu runs its own message loop, during which other handlers need the state;
    // it is therefore only borrowed to build the menu and to execute the result.
    let Some(Some(menu)) = with_app(|app| app.build_menu()) else {
        return;
    };
    let mut cursor = POINT::default();
    // SAFETY: valid handles and out-pointer; the menu is destroyed right after use.
    let chosen = unsafe {
        let _ = GetCursorPos(&mut cursor);
        // Without foreground status the menu would not close on a click elsewhere.
        let _ = SetForegroundWindow(hwnd);
        MENU_ROOT.store(menu.0 as isize, Ordering::Relaxed);
        MENU_SELECTION.store(0, Ordering::Relaxed);
        let filter = SetWindowsHookExW(WH_MSGFILTER, Some(menu_filter), None, GetCurrentThreadId());
        // Opened on the widget, the menu goes above it instead of covering it, so the
        // effect of a setting is visible while the menu stays open.
        let mut flags = TPM_RETURNCMD | TPM_RIGHTBUTTON;
        let mut widget = RECT::default();
        let on_widget = IsWindowVisible(hwnd).as_bool()
            && GetWindowRect(hwnd, &mut widget).is_ok()
            && (widget.left..widget.right).contains(&cursor.x)
            && (widget.top..widget.bottom).contains(&cursor.y);
        if on_widget && widget.top > GetSystemMetrics(SM_CYSCREEN) / 2 {
            cursor.y = widget.top - 4;
            flags |= TPM_BOTTOMALIGN;
        }
        let chosen = TrackPopupMenu(menu, flags, cursor.x, cursor.y, None, hwnd, None);
        if let Ok(filter) = filter {
            let _ = UnhookWindowsHookEx(filter);
        }
        MENU_ROOT.store(0, Ordering::Relaxed);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        chosen.0 as usize
    };
    if chosen != 0 && with_app(|app| app.command(chosen)) == Some(true) {
        // SAFETY: our own window; WM_DESTROY ends the message loop.
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_APP_DEVICES => {
            with_app(App::on_devices);
        }
        WM_APP_FOREGROUND => {
            with_app(|app| {
                app.probe.poke();
                app.refresh();
            });
        }
        WM_TIMER if wparam.0 == TIMER_TICK => {
            with_app(App::refresh);
        }
        WM_APP_TRAY => match (lparam.0 as u32) & 0xFFFF {
            WM_CONTEXTMENU => show_menu(hwnd),
            NIN_SELECT | NIN_KEYSELECT => {
                with_app(|app| app.command(CMD_TOGGLE_WIDGET));
            }
            _ => {}
        },
        WM_RBUTTONUP | WM_NCRBUTTONUP => show_menu(hwnd),
        // In free mode the whole widget is a drag handle.
        WM_NCHITTEST => {
            let free = with_app(|app| app.config.position == Position::Free).unwrap_or(false);
            return LRESULT(if free { HTCAPTION } else { HTCLIENT } as isize);
        }
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        // Remember the highlighted item so that Enter can be handled like a click.
        WM_MENUSELECT => {
            let flags = ((wparam.0 >> 16) & 0xFFFF) as u32;
            let selectable =
                lparam.0 != 0 && flags & (MF_POPUP | MF_DISABLED | MF_GRAYED | MF_SEPARATOR).0 == 0;
            let id = if selectable {
                (wparam.0 & 0xFFFF) as u32
            } else {
                0
            };
            MENU_SELECTION.store(id, Ordering::Relaxed);
        }
        WM_ENTERSIZEMOVE => {
            with_app(|app| app.dragging = true);
        }
        WM_EXITSIZEMOVE => {
            with_app(App::end_drag);
        }
        // Theme, taskbar settings, resolution or DPI changed: draw again.
        WM_SETTINGCHANGE | WM_DISPLAYCHANGE | WM_DPICHANGED | WM_THEMECHANGED => {
            with_app(|app| {
                app.dirty = true;
                app.update_tray();
                app.refresh();
            });
        }
        WM_POWERBROADCAST => {
            // Devices reconnect after resume; do not wait for the next poll.
            with_app(|app| {
                if let Some(hub) = &app.hub {
                    hub.refresh();
                }
            });
            return LRESULT(1);
        }
        WM_ENDSESSION => {
            with_app(|app| app.save());
        }
        WM_CLOSE => {
            // SAFETY: our own window.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
        WM_DESTROY => {
            // SAFETY: ends the message loop of this thread.
            unsafe { PostQuitMessage(0) };
        }
        _ if message != 0 && message == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            // Explorer restarted and lost every tray icon.
            with_app(|app| {
                app.tray.add();
                app.update_tray();
                app.dirty = true;
                app.refresh();
            });
        }
        // SAFETY: forwarding unhandled messages unchanged.
        _ => return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
    LRESULT(0)
}

/// Foreground changes are when full-screen applications appear and when the taskbar
/// raises itself above the widget.
unsafe extern "system" fn foreground_changed(
    _: HWINEVENTHOOK,
    _: u32,
    _: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    let window = MAIN_WINDOW.load(Ordering::Relaxed);
    if window != 0 {
        // SAFETY: posting to a possibly stale handle merely fails.
        unsafe {
            let _ = PostMessageW(
                Some(HWND(window as *mut c_void)),
                WM_APP_FOREGROUND,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

/// Let context menus follow the system's dark mode. These uxtheme exports have no
/// names; if they are missing the menus simply stay light.
fn allow_dark_menus() {
    // SAFETY: ordinals 135 (SetPreferredAppMode) and 136 (FlushMenuThemes) have had these
    // signatures since Windows 10 1903; both are looked up and may be absent.
    unsafe {
        // Load from System32 only: uxtheme is not a KnownDLL, so a plain name would also
        // search the (user-writable) directory of the executable first.
        let Ok(uxtheme) = LoadLibraryExW(w!("uxtheme.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)
        else {
            return;
        };
        if let Some(function) = GetProcAddress(uxtheme, PCSTR(135 as *const u8)) {
            let set_preferred_app_mode: unsafe extern "system" fn(i32) -> i32 =
                std::mem::transmute(function);
            set_preferred_app_mode(1);
        }
        if let Some(function) = GetProcAddress(uxtheme, PCSTR(136 as *const u8)) {
            let flush_menu_themes: unsafe extern "system" fn() = std::mem::transmute(function);
            flush_menu_themes();
        }
    }
}

pub fn run() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let data_dir = config::data_dir();
    crate::logger::init(
        &data_dir.join("app.log"),
        arguments.iter().any(|a| a == "--verbose"),
    );
    log::info!("{APP_NAME} {} starting", env!("CARGO_PKG_VERSION"));

    // SAFETY: Win32 start-up sequence on the main thread. Handles created here live
    // until the process exits.
    unsafe {
        // Restrict run-time DLL loads to System32, closing the search-order gap for every
        // library this process loads after start-up (the manifest-driven implicit imports
        // are additionally pinned at link time via /DEPENDENTLOADFLAG).
        let _ = SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32);

        // One instance per session. The mutex is released by the system on exit.
        let _instance = CreateMutexW(None, true, w!("Local\\BatteryWidget.SingleInstance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            log::info!("already running; exiting");
            return;
        }
        // Normally set by the manifest; this covers builds without embedded resources.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        allow_dark_menus();
        autostart::refresh_path();

        let Ok(module) = GetModuleHandleW(None) else {
            return;
        };
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: module.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            log::error!("could not register the window class");
            return;
        }
        // Layered: per-pixel alpha. Tool window: no taskbar button, not in Alt+Tab.
        // No-activate: clicking the widget never steals focus from a game or editor.
        let created = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            CLASS_NAME,
            w!("BatteryWidget"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(module.into()),
            None,
        );
        let Ok(hwnd) = created else {
            log::error!("could not create the window");
            return;
        };
        let renderer = match Renderer::new() {
            Ok(renderer) => renderer,
            Err(error) => {
                log::error!("Direct2D unavailable: {error}");
                return;
            }
        };

        MAIN_WINDOW.store(hwnd.0 as isize, Ordering::Relaxed);
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(w!("TaskbarCreated")),
            Ordering::Relaxed,
        );
        let config_path = data_dir.join("config.json");
        let mut app = App {
            hwnd,
            config: Config::load(&config_path),
            config_path,
            devices: Vec::new(),
            shown: Vec::new(),
            inbox: Arc::new(Mutex::new(None)),
            hub: None,
            alerts: AlertManager::default(),
            started: Instant::now(),
            renderer,
            tray: Tray::new(hwnd, WM_APP_TRAY),
            style: None,
            layout: Layout::default(),
            dirty: true,
            visible: false,
            origin: POINT::default(),
            dragging: false,
            slot: None,
            probe: Probe::start(hwnd.0 as isize, WM_APP_FOREGROUND),
            demo: arguments.iter().any(|a| a == "--demo"),
        };
        app.tray.add();
        app.update_tray();
        app.start_hub();
        APP.with(|cell| *cell.borrow_mut() = Some(app));

        SetTimer(Some(hwnd), TIMER_TICK, TICK_MS, None);
        let hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(foreground_changed),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );

        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        let _ = UnhookWinEvent(hook);
        MAIN_WINDOW.store(0, Ordering::Relaxed);
        // Dropping the state stops the workers and removes the tray icon.
        if let Some(app) = APP.with(|cell| cell.borrow_mut().take()) {
            app.probe.stop();
            app.save();
        }
    }
    log::info!("{APP_NAME} stopped");
}
