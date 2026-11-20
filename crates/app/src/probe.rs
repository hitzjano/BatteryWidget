//! Finds out which parts of the taskbar are occupied by buttons.
//!
//! The Windows 11 taskbar is a single XAML surface: its buttons are not windows, so only
//! UI Automation knows where they are. Such a query crosses into Explorer and can take
//! tens of milliseconds, so it runs on its own thread and the UI reads the last result.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use battery_widget_core::wake::Wake;
use windows::core::Result;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, TreeScope_Descendants, UIA_BoundingRectanglePropertyId,
    UIA_ProcessIdPropertyId,
};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, PostMessageW};

/// How often the taskbar is measured while the widget lives in it.
const INTERVAL: Duration = Duration::from_secs(3);
/// While there is no result yet the widget stays hidden, so a failed measurement is
/// repeated quickly …
const RETRY: Duration = Duration::from_millis(300);
/// … and after this many failures in a row the widget is shown without it.
const MAX_FAILURES: u32 = 10;
/// Anything wider than this share of the taskbar is a container, not a button.
const MAX_BUTTON_SHARE: i32 = 3;

/// Horizontal spans `(left, right)` in screen coordinates.
pub type Spans = Vec<(i32, i32)>;

/// Shared between the UI thread (sets the target, reads the result) and the probe thread.
pub struct Probe {
    wake: Arc<Wake>,
    /// Taskbar window to measure; 0 pauses the probe.
    taskbar: AtomicIsize,
    /// Elements right of this x belong to the notification area and are ignored.
    limit: AtomicI32,
    /// Result of the last measurement; `None` until the current target was measured.
    spans: Mutex<Option<Spans>>,
    unavailable: AtomicBool,
}

impl Probe {
    /// Start the probe thread. `window` receives `message` whenever the result changes.
    pub fn start(window: isize, message: u32) -> Arc<Probe> {
        let probe = Arc::new(Probe {
            wake: Arc::new(Wake::default()),
            taskbar: AtomicIsize::new(0),
            limit: AtomicI32::new(i32::MAX),
            spans: Mutex::new(None),
            unavailable: AtomicBool::new(false),
        });
        let worker = probe.clone();
        let spawned = std::thread::Builder::new()
            .name("taskbar-probe".into())
            .spawn(move || worker.run(window, message));
        if let Err(error) = spawned {
            log::warn!("taskbar probe not started: {error}");
            probe.unavailable.store(true, Ordering::Relaxed);
        }
        probe
    }

    fn lock(&self) -> MutexGuard<'_, Option<Spans>> {
        self.spans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Tell the probe which taskbar to watch (`None` pauses it).
    pub fn watch(&self, taskbar: Option<(HWND, i32)>) {
        let (window, limit) =
            taskbar.map_or((0, i32::MAX), |(hwnd, limit)| (hwnd.0 as isize, limit));
        self.limit.store(limit, Ordering::Relaxed);
        if self.taskbar.swap(window, Ordering::Relaxed) != window {
            *self.lock() = None;
            self.wake.notify();
        }
    }

    /// Measure again soon (something on the desktop changed).
    pub fn poke(&self) {
        self.wake.notify();
    }

    pub fn stop(&self) {
        self.wake.stop();
    }

    /// Occupied spans of the watched taskbar, once it has been measured.
    pub fn spans(&self) -> Option<Spans> {
        self.lock().clone()
    }

    /// UI Automation cannot be used at all; there is no point in waiting for results.
    pub fn is_unavailable(&self) -> bool {
        self.unavailable.load(Ordering::Relaxed)
    }

    fn notify(window: isize, message: u32) {
        // SAFETY: posting to a window handle is safe from any thread.
        unsafe {
            let _ = PostMessageW(
                Some(HWND(window as *mut c_void)),
                message,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }

    fn run(&self, window: isize, message: u32) {
        // SAFETY: COM initialisation for this thread only.
        let automation: Result<IUIAutomation> = unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
        };
        let automation = match automation {
            Ok(automation) => automation,
            Err(error) => {
                log::warn!(
                    "UI Automation unavailable ({error}); the widget cannot avoid taskbar buttons"
                );
                self.unavailable.store(true, Ordering::Relaxed);
                Self::notify(window, message);
                return;
            }
        };
        let mut failures = 0u32;
        while !self.wake.is_stopped() {
            let taskbar = self.taskbar.load(Ordering::Relaxed);
            let mut pause = INTERVAL;
            if taskbar != 0 {
                let limit = self.limit.load(Ordering::Relaxed);
                match measure(&automation, HWND(taskbar as *mut c_void), limit) {
                    Ok(found) => {
                        failures = 0;
                        let recovered = self.unavailable.swap(false, Ordering::Relaxed);
                        let mut current = self.lock();
                        // The target may have changed while we were measuring.
                        let still_wanted = self.taskbar.load(Ordering::Relaxed) == taskbar;
                        if still_wanted && (recovered || current.as_ref() != Some(&found)) {
                            *current = Some(found);
                            drop(current);
                            Self::notify(window, message);
                        }
                    }
                    Err(error) => {
                        failures += 1;
                        if failures == 1 {
                            log::info!("taskbar measurement failed: {error}");
                        }
                        if self.lock().is_none() {
                            pause = RETRY;
                            if failures == MAX_FAILURES {
                                log::warn!("taskbar cannot be measured; showing the widget anyway");
                                self.unavailable.store(true, Ordering::Relaxed);
                                Self::notify(window, message);
                            }
                        }
                    }
                }
            }
            self.wake.sleep(pause);
        }
    }
}

/// One cross-process call: the horizontal spans of everything inside the taskbar, sorted.
/// Containers, elements right of `limit` (the notification area) and the widget itself
/// (which lives inside the taskbar) are left out.
fn measure(automation: &IUIAutomation, taskbar: HWND, limit: i32) -> Result<Spans> {
    let mut bar = RECT::default();
    // SAFETY: COM calls on interfaces owned by this thread; out-pointers are live locals.
    unsafe {
        GetWindowRect(taskbar, &mut bar)?;
        let cache = automation.CreateCacheRequest()?;
        cache.AddProperty(UIA_BoundingRectanglePropertyId)?;
        cache.AddProperty(UIA_ProcessIdPropertyId)?;
        let own = GetCurrentProcessId() as i32;
        let root = automation.ElementFromHandle(taskbar)?;
        let everything = automation.CreateTrueCondition()?;
        let elements = root.FindAllBuildCache(TreeScope_Descendants, &everything, &cache)?;
        let max_width = (bar.right - bar.left) / MAX_BUTTON_SHARE;
        let mut spans = Spans::new();
        for index in 0..elements.Length()? {
            let Ok(element) = elements.GetElement(index) else {
                continue;
            };
            if element.CachedProcessId().is_ok_and(|pid| pid == own) {
                continue;
            }
            let Ok(rect) = element.CachedBoundingRectangle() else {
                continue;
            };
            let width = rect.right - rect.left;
            let inside =
                rect.left >= bar.left && rect.top >= bar.top - 2 && rect.bottom <= bar.bottom + 2;
            if width > 0 && width <= max_width && inside && rect.right <= limit {
                spans.push((rect.left, rect.right));
            }
        }
        spans.sort_unstable();
        spans.dedup();
        Ok(spans)
    }
}
