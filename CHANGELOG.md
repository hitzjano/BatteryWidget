# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-11-16

### Added

- Taskbar widget: borderless, per-pixel transparent, never takes focus, drawn with
  Direct2D/DirectWrite. Positions: next to the tray, taskbar left, four screen corners,
  or freely draggable. Per-monitor DPI aware, follows the Windows dark/light theme,
  hides behind full-screen applications and with an auto-hiding taskbar.
- The widget measures where the taskbar buttons are (UI Automation) and only uses a
  free gap between them: in front of the notification area, or the first gap from the
  left (after the weather button of a centred taskbar). Devices that do not fit
  collapse into a "+N" pill, keeping the weakest batteries visible; device names are
  shortened before they are dropped.
- Four display styles: icon + battery + percentage, icon + battery, icon + percentage,
  and icon + battery with the percentage inside.
- `--demo` switch showing sample devices.
- Tray icon showing the lowest battery level, with all settings in its context menu.
  The menu stays open while settings are changed, so several can be tried in a row.
- Bluetooth Classic and LE battery levels through the Windows device property store,
  event driven, with estimated charging state.
- Logitech HID++ 2.0 (Unified Battery, Battery Status, Battery Voltage, ADC Measurement,
  Centurion SoC) and HID++ 1.0 registers, for Bolt, Unifying and LIGHTSPEED receivers
  as well as wired and Bluetooth devices.
- Logitech G HUB local WebSocket API as an additional source.
- Vendor protocols: PlayStation DualSense / DualSense Edge / DualShock 4, Razer,
  Corsair (Void/HS/Virtuoso headsets, Slipstream devices), HyperX (Cloud II, Cloud
  Alpha, Cloud III, Pulsefire Dart), SteelSeries Arctis 1 / 7 / Nova 7.
- Xbox and other controllers through `Windows.Gaming.Input`, with XInput as fallback.
- De-duplication of devices seen by several sources, hot-plug handling, grace period
  for sleeping devices.
- Low-battery and fully-charged notifications with hysteresis and cool-down.
- Optional start with Windows (per-user `Run` key).

[Unreleased]: https://github.com/hitzjano/BatteryWidget/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hitzjano/BatteryWidget/releases/tag/v0.1.0
