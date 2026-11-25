# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.5] - 2026-11-25

### Added

- ASUS ROG and TUF wireless mice, keyboards (Azoth, Falchion, Strix Scope, Claymore II)
  and headsets (Delta II, Pelta, Cetra SpeedNova) on their dongle, cable or the ROG Omni
  receiver, using the protocol implemented by G-Helper.
- SteelSeries: Arctis 7+, Arctis 9, Arctis Nova 3P / 5 / 7X / 7P, Nova Pro Wireless and
  the newer Nova 7 firmware; Aerox 3 / 5 / 9 Wireless, Prime Wireless, Rival 3 Wireless
  and Rival 650 Wireless mice.
- HyperX Cloud Flight, Cloud Alpha 2 Wireless and the Kingston Cloud II Wireless.
- Corsair Void Wireless v2, Virtuoso MAX Wireless and HS80 MAX Wireless receivers.
- JBL Quantum 810, 910P and 950 Wireless headsets on their USB dongle.
- JBL portable speakers (Charge 3/4/5/6, Flip 4/5/6/7, Xtreme 2/3/4, Pulse 3/4/5,
  Boombox/2/3, Go 4 incl. Duo, Clip 5, Tuner 3) over their Bluetooth LE control service,
  which Windows itself does not read (no battery under *Bluetooth & devices*).

### Fixed

- The Kingston-branded HyperX Cloud II Wireless (product id `0x1718`) was listed under
  the HP vendor id and therefore never queried; it now uses its own protocol.

## [0.1.4] - 2026-11-23

### Fixed

- Updating with `install.ps1` while BatteryWidget is running no longer fails now and
  then with "file is being used by another process": the installer waits until Windows
  has released the old executable.
- A device that is charging no longer shows 0 %. Many devices cannot measure their level
  for a while after the charging cable is plugged in and report 0 then; this is now
  treated as "unknown" for Logitech (HID++ and G HUB) and Razer devices, and Logitech
  headsets no longer show readings that the headset itself marks as invalid.
- A 0 % or outdated level from one source no longer hides the correct level that a
  second source reports for the same device (e.g. G HUB and direct HID++, or a mouse that
  moved from its receiver to the charging cable).
- A Logitech device that reconnects (as many do when the charging cable is plugged in)
  keeps its level instead of dropping it until the next successful reading.
- Charging is shown sooner for Razer, SteelSeries, Corsair, HyperX and PlayStation
  devices: a connected device is read every 20 seconds instead of every minute, and a
  newly attached or removed device (such as a mouse switching to its cable) is picked up
  within a few seconds.

### Changed

- While a device is charging and cannot report its level, the widget shows only the
  charging bolt instead of a percentage, so an old value does not look as if charging
  had no effect. In *Icon and battery with percentage inside* the bolt sits inside the
  empty battery; the tooltip says "charging". Once the device reports a level again, it
  is shown as usual. Without charging, a briefly unknown level keeps the last real one.
- `--demo` includes a charging device with an unknown level.

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

[Unreleased]: https://github.com/hitzjano/BatteryWidget/compare/v0.1.5...HEAD
[0.1.5]: https://github.com/hitzjano/BatteryWidget/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/hitzjano/BatteryWidget/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/hitzjano/BatteryWidget/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/hitzjano/BatteryWidget/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/hitzjano/BatteryWidget/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/hitzjano/BatteryWidget/releases/tag/v0.1.0
