# Protocols

How BatteryWidget obtains battery levels. Everything here is read-only.

## Bluetooth

No GATT or HFP traffic is generated. The Windows Bluetooth stack already reads the GATT
Battery Service (LE) or the HFP battery indicator (Classic) and publishes the result as
the PnP device property `{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2` (a byte, 0 to 100).

1. Paired devices: `DeviceInformation.FindAllAsync` with the selectors of
   `BluetoothDevice` and `BluetoothLEDevice` (`GetDeviceSelectorFromPairingState(true)`),
   requesting `System.Devices.Aep.IsConnected`, `.DeviceAddress`, `.ContainerId`,
   `.Bluetooth.Cod.Major/Minor` and `.Bluetooth.Le.Appearance`.
2. Battery nodes: `FindAllAsync` with `DeviceInformationKind.Device` over instance ids
   starting with `BTH`, `HID` or `USB`, requesting the battery property,
   `System.Devices.ContainerId` and `System.Devices.Present`.
3. A battery node belongs to the paired device with the same container id, otherwise
   to the one whose address appears in the node's instance id.

Four `DeviceWatcher`s and `Radio.StateChanged` trigger a rescan; a safety scan runs
every 15 s. Charging is estimated: the level rises while the device has been connected
for at least 90 s; the estimate expires at 100 % or after 30 min without a further rise.

## Logitech HID++

Collections with vendor id `0x046D` and usage page `0xFF00` (`0xFF43` over Bluetooth LE).
Usage 1 is the short collection (report `0x10`, 7 bytes), usage 2 the long one (report
`0x11`, 20 bytes).

Request (HID++ 2.0, always sent as a long report):

| Byte | 0 | 1 | 2 | 3 | 4… |
| --- | --- | --- | --- | --- | --- |
| | `0x11` | device index | feature index | `function << 4 \| 0x0B` | parameters |

The low nibble of byte 3 is a software id, which separates our replies from those meant
for other software. Receivers address paired devices as index 1 to 6; a wired or
Bluetooth device is `0xFF`. Errors come back as `11 idx FF feat fn code` or
`10 idx 8F feat fn code` (code 4: paired but asleep; 1: device only speaks HID++ 1.0).

Sequence per device: ping (root function 1) → look up the battery feature (root
function 0) → read name and type (feature `0x0005`) and unit id (`0x0003`) once → read
the battery.

| Feature | Reply | Percent | Charging |
| --- | --- | --- | --- |
| `0x1004` Unified Battery (function 1) | soc, level bits, status | soc; if 0, from level bits (8→100, 4→60, 2→20, 1→5) | status 1, 2 |
| `0x1000` Battery Status | percent, next, status | byte 0 | status 1, 2, 4 |
| `0x0104` Centurion SoC | soc, ?, status | byte 0 | status 1, 2 |
| `0x1F20` ADC Measurement | mV (big endian), flags | ≤ 100 is a percentage, else voltage curve | flags & 2 |
| `0x1001` Battery Voltage | mV (big endian), flags | voltage curve | flags & 0x80 and (flags & 3) = 0 |

"Full" is status 3 and is only trusted at 98 % or more. Voltage curve (mV → %, linear
in between): 4186→100, 4067→90, 3989→80, 3922→70, 3859→60, 3811→50, 3778→40, 3751→30,
3717→20, 3671→10, 3646→5, 3579→2, 3500→0.

HID++ 1.0 (short report `10 idx sub reg p0 p1 p2`): battery from register `0x0D`
(`81 0D`: percent in byte 0, state in the high nibble of byte 2: `0x50` charging, `0x90`
full) or `0x07` (coarse level 1/3/5/7). Name, kind and serial come from the receiver's
pairing register `0xB5` (`83 B5 4n`, `2n`, `3n`).

A receiver notification with sub-id `0x41` (connection change) or a battery broadcast
triggers a rescan after 0.8 s; otherwise devices are polled once a minute.

## Logitech G HUB

`ws://127.0.0.1:9010`, headers `Origin: file://` and `Sec-WebSocket-Protocol: json`.

```
→ {"msgId":"","verb":"SUBSCRIBE","path":"/battery/state/changed"}
→ {"msgId":"","verb":"SUBSCRIBE","path":"/devices/state/changed"}
→ {"msgId":"list","verb":"GET","path":"/devices/list"}
← {"path":"/devices/list","payload":{"deviceInfos":[{"id",…,"deviceUnitId",…}]}}
→ {"msgId":"","verb":"GET","path":"/battery/<id>/state"}
← {"path":"/battery/…","payload":{"deviceId","percentage","charging","fullyCharged"}}
```

`deviceUnitId` equals the HID++ unit id, which is how both sources are merged. All
input is treated as untrusted: types are checked and levels clamped to 0–100.

## Game controllers

- **PlayStation** (passive, from the regular input report). DualSense: report `0x01`
  byte 53 (USB) or `0x31` byte 54 (Bluetooth); low nibble × 10 + 5 %, high nibble 0
  discharging, 1 charging, 2 full. DualShock 4: report `0x01` byte 30 or `0x11` byte 32;
  bit 4 means cable attached. Over Bluetooth one feature report (`0x05` / `0x02`) is
  requested to switch the controller to full reports.
- **Xbox**: `RawGameController.TryGetBatteryReport()`, remaining / full capacity. A
  wireless controller reporting exactly 10 % without charging is treated as "unknown"
  and the last trustworthy value is used. Fallback: `XInputGetBatteryInformation`.

## Vendor HID

| Device family | Request | Reply |
| --- | --- | --- |
| Razer mice/keyboards | 90-byte feature report, class `0x07`, command `0x80` (level) / `0x84` (charging), XOR checksum over bytes 2–87 | byte 9: level 0–255, or 1 = charging |
| Corsair Void/HS/Virtuoso | `C9 64` on usage page `0xFFC5` | byte 2 & `0x7F` percent, byte 4: 1/2 normal, 4/5 charging |
| Corsair Slipstream (Bragi) | `00 <08\|dev> 02 <prop>` on usage page `0xFF42` | 24-bit value; prop `0x0F` level in ‰, `0x10` status, `0x36` paired devices |
| HyperX Cloud II | `06 FF BB 02` / `…03` | byte 7 percent / byte 4 = 1 charging |
| HyperX Cloud Alpha, Cloud III | `21 BB 0B` / `0C` / `03` | byte 3: percent / charging / link state |
| HyperX Pulsefire Dart | `51` | `51 00 00 03 <percent> <charging>` |
| SteelSeries Arctis 7 | `06 18` | byte 2 percent |
| SteelSeries Arctis 1 | `06 12` | byte 3 percent, byte 2 = 1 means off |
| SteelSeries Arctis Nova 7 | `00 B0` | byte 2 level 0–4, byte 3: 0 off, 1 charging |

A device that does not answer is not asked again for 10 minutes (2 minutes for devices
that are expected to sleep), so a misbehaving device cannot be flooded.

## Merging

Order of precedence: G HUB, HID++, vendor HID, Bluetooth. Two entries are the same
device if their normalised names match (brand prefixes and marketing suffixes removed)
and their kinds do not contradict. A radio or cable link wins over Bluetooth, a
connected entry over a disconnected one. Logitech devices that stop answering stay
listed as idle for 10 minutes.
