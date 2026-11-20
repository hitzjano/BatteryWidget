# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.3] - 2026-11-20

### Fixed

- A `config.json` that starts with a UTF-8 byte order mark (as written by some Windows
  editors) is read normally instead of being rejected, which used to reset all settings.

## [0.1.2] - 2026-11-20

### Fixed

- The widget no longer vanishes from the taskbar while the Start menu, the notification
  centre or another flyout is open (Windows 11 lifts the taskbar into a higher z-order
  band then, which no topmost window can get above, and sometimes leaves it there). In the
  taskbar positions the widget is now embedded in the taskbar as a child window instead
  of floating on top of it; it is recreated automatically after Explorer restarts and
  slides in and out together with an auto-hiding taskbar.
- The widget no longer stays visible on top of a full-screen window that covers the
  taskbar but does not have the focus (for example a full-screen Remote Desktop session
  on the second display while working on the first). Embedded in the taskbar, it is
  covered together with it.
- On a compact taskbar ("small taskbar buttons", 32 px instead of 48 px) the widget
  shrinks to fit instead of filling almost the whole height.

### Changed

- *Start with Windows* is now on by default: the first start switches it on once. Turning
  it off in the menu is remembered and is not undone by later starts or updates.
- The taskbar probe ignores the widget's own window, and choosing a new position or
  display in the menu now closes the menu.
- `--verbose` logs why the widget is hidden.

## [0.1.1] - 2026-11-18

### Security

- Hardened against DLL search-order hijacking: `uxtheme.dll` is now loaded from System32
  only (`LoadLibraryExW` with `LOAD_LIBRARY_SEARCH_SYSTEM32`), the process restricts
  run-time DLL loads to System32 at start-up (`SetDefaultDllDirectories`), and statically
  imported dependencies are pinned to System32 via `/DEPENDENTLOADFLAG:0x800`.
- Enabled Control Flow Guard (`-C control-flow-guard`) for the MSVC build.

### Changed

- CI now runs a RustSec dependency audit (`cargo audit`) on every push and pull request.

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

[Unreleased]: https://github.com/hitzjano/BatteryWidget/compare/v0.1.3...HEAD
[0.1.3]: https://github.com/hitzjano/BatteryWidget/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/hitzjano/BatteryWidget/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/hitzjano/BatteryWidget/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/hitzjano/BatteryWidget/releases/tag/v0.1.0
