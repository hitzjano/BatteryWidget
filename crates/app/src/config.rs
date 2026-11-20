//! Settings, stored as JSON in `%LOCALAPPDATA%\BatteryWidget\config.json`.
//!
//! Loading never fails: a missing or damaged file yields the defaults, unknown keys are
//! ignored and every value is clamped to a sane range.

use std::path::{Path, PathBuf};

use battery_widget_core::{AlertSettings, HubOptions};
use serde::{Deserialize, Serialize};

use crate::layout::Display;

pub const APP_NAME: &str = "BatteryWidget";
pub const LOW_THRESHOLDS: [u8; 5] = [10, 15, 20, 25, 30];
const MAX_OFFSET: i32 = 4000;

/// Where the widget sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Position {
    /// Inside the taskbar, left of the notification area.
    #[default]
    TaskbarRight,
    /// Inside the taskbar, at its left edge.
    TaskbarLeft,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    /// Wherever the user dragged it.
    Free,
}

impl Position {
    pub const ALL: [(Position, &'static str); 7] = [
        (Position::TaskbarRight, "Taskbar, next to the tray"),
        (Position::TaskbarLeft, "Taskbar, left"),
        (Position::TopLeft, "Top left"),
        (Position::TopRight, "Top right"),
        (Position::BottomLeft, "Bottom left"),
        (Position::BottomRight, "Bottom right"),
        (Position::Free, "Free (drag to move)"),
    ];

    pub fn in_taskbar(self) -> bool {
        matches!(self, Position::TaskbarRight | Position::TaskbarLeft)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    /// Follow the Windows taskbar theme.
    #[default]
    Auto,
    Dark,
    Light,
}

impl ThemeMode {
    pub const ALL: [(ThemeMode, &'static str); 3] = [
        (ThemeMode::Auto, "Match Windows"),
        (ThemeMode::Dark, "Dark"),
        (ThemeMode::Light, "Light"),
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sources {
    pub bluetooth: bool,
    pub logitech_hidpp: bool,
    pub logitech_ghub: bool,
    pub vendor_hid: bool,
    pub game_controllers: bool,
}

impl Default for Sources {
    fn default() -> Self {
        Sources {
            bluetooth: true,
            logitech_hidpp: true,
            logitech_ghub: true,
            vendor_hid: true,
            game_controllers: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub show_widget: bool,
    pub position: Position,
    /// Extra horizontal distance from the anchor in taskbar modes (device-independent pixels).
    pub offset_x: i32,
    /// Screen coordinates used by [`Position::Free`].
    pub free_x: i32,
    pub free_y: i32,
    /// Index of the display to use; `0` is the primary display.
    pub monitor: usize,
    pub theme: ThemeMode,
    pub show_names: bool,
    /// What each pill shows.
    pub display: Display,
    pub sources: Sources,
    pub notify_low: bool,
    pub notify_full: bool,
    pub low_threshold: u8,
    pub notification_cooldown_min: u32,
    /// Set once "Start with Windows" has been switched on by default on the first run, so
    /// that switching it off later sticks.
    pub autostart_default_applied: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            show_widget: true,
            position: Position::default(),
            offset_x: 0,
            free_x: 200,
            free_y: 200,
            monitor: 0,
            theme: ThemeMode::default(),
            show_names: false,
            display: Display::default(),
            sources: Sources::default(),
            notify_low: true,
            notify_full: false,
            low_threshold: 20,
            notification_cooldown_min: 60,
            autostart_default_applied: false,
        }
    }
}

impl Config {
    /// Bring every value into its valid range.
    pub fn sanitized(mut self) -> Config {
        self.offset_x = self.offset_x.clamp(-MAX_OFFSET, MAX_OFFSET);
        self.monitor = self.monitor.min(15);
        self.low_threshold = self.low_threshold.clamp(1, 95);
        self.notification_cooldown_min = self.notification_cooldown_min.clamp(1, 24 * 60);
        self
    }

    pub fn parse(text: &str) -> Config {
        serde_json::from_str::<Config>(text)
            .map(Config::sanitized)
            .unwrap_or_else(|error| {
                log::warn!("config is not valid ({error}); using defaults");
                Config::default()
            })
    }

    pub fn load(path: &Path) -> Config {
        match std::fs::read_to_string(path) {
            Ok(text) => Config::parse(&text),
            Err(_) => Config::default(),
        }
    }

    /// Write atomically: a crash or power loss never leaves a half-written file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(
            &temp,
            serde_json::to_string_pretty(self).map_err(std::io::Error::other)?,
        )?;
        std::fs::rename(&temp, path)
    }

    pub fn hub_options(&self) -> HubOptions {
        HubOptions {
            bluetooth: self.sources.bluetooth,
            hidpp: self.sources.logitech_hidpp,
            ghub: self.sources.logitech_ghub,
            vendor: self.sources.vendor_hid,
            xbox: self.sources.game_controllers,
        }
    }

    pub fn alert_settings(&self) -> AlertSettings {
        AlertSettings {
            low_enabled: self.notify_low,
            full_enabled: self.notify_full,
            low_threshold: self.low_threshold,
            cooldown_s: f64::from(self.notification_cooldown_min) * 60.0,
        }
    }
}

/// `%LOCALAPPDATA%\BatteryWidget` (falls back to the directory of the executable).
pub fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(Path::to_path_buf))
        })
        .unwrap_or_default()
        .join(APP_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_survive_a_round_trip() {
        let text = serde_json::to_string(&Config::default()).unwrap();
        assert_eq!(Config::parse(&text), Config::default());
    }

    #[test]
    fn partial_and_unknown_keys() {
        let config = Config::parse(
            r#"{"position":"top_left","sources":{"bluetooth":false},"future_option":1}"#,
        );
        assert_eq!(config.position, Position::TopLeft);
        assert!(!config.sources.bluetooth && config.sources.logitech_hidpp);
        assert!(config.show_widget);
        assert_eq!(config.display, Display::Full);
        assert_eq!(
            Config::parse(r#"{"display":"bar_percent"}"#).display,
            Display::BarPercent
        );
    }

    #[test]
    fn autostart_default_is_applied_only_once() {
        assert!(!Config::default().autostart_default_applied);
        // Configs written by older versions lack the key: the default is applied once.
        assert!(!Config::parse(r#"{"show_names":true}"#).autostart_default_applied);
        assert!(Config::parse(r#"{"autostart_default_applied":true}"#).autostart_default_applied);
    }

    #[test]
    fn damaged_file_falls_back_to_defaults() {
        assert_eq!(Config::parse("{ not json"), Config::default());
        assert_eq!(
            Config::parse(r#"{"position":"on_the_moon"}"#),
            Config::default()
        );
    }

    #[test]
    fn values_are_clamped() {
        let config = Config::parse(
            r#"{"low_threshold":0,"offset_x":999999,"monitor":400,"notification_cooldown_min":0}"#,
        );
        assert_eq!(
            (config.low_threshold, config.offset_x, config.monitor),
            (1, MAX_OFFSET, 15)
        );
        assert_eq!(config.notification_cooldown_min, 1);
    }

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!("battery-widget-test-{}", std::process::id()));
        let path = dir.join("config.json");
        let config = Config {
            show_names: true,
            low_threshold: 15,
            ..Config::default()
        };
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path), config);
        assert_eq!(Config::load(&dir.join("missing.json")), Config::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
