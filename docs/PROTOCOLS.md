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
| HyperX Cloud II (Kingston) | `06 00 02 00 9A 00 00 68 4A 8E 0A 00 00 00 BB <cmd>` | `0B 00 BB <cmd>`; cmd 2: byte 7 percent, cmd 3: byte 4 = 1 charging |
| HyperX Cloud Flight | `21 FF 05` | bytes 3–4 battery mV (big endian); above 4107 mV charging, else HeadsetControl's discharge curve |
| HyperX Cloud Alpha 2 | `52 01` / `50 02` on usage page `0xFF13` | `53 01 <connected>` / `51 02 <percent> <1 = charging>` |
| Corsair Wireless v2 receivers | `00 02 09 02 0F` on interface 4 | `01 <src> 02 00 <value u16 LE>`, value in ‰ (1–1000) |
| SteelSeries Arctis 7 | `06 18` | byte 2 percent |
| SteelSeries Arctis 1 | `06 12` | byte 3 percent, byte 2 = 1 means off |
| SteelSeries Arctis Nova 7 / 7X / 7P, Arctis 7+ | `00 B0` | byte 2 level (0–4 on older, percent on newer product ids), byte 3: 0 off, 1/2 charging (7+: byte 1 = 1 off) |
| SteelSeries Arctis Nova 3P / 5 | `00 B0` | byte 3 percent, byte 1 = 2 off, byte 4 = 1 charging |
| SteelSeries Arctis Nova Pro Wireless | `06 B0` on interface 4 | byte 6 level 0–8, byte 15: 1 off, 2 charging |
| SteelSeries Arctis 9 | `00 20` on interface 0 | byte 3 level `0x64`–`0x9A`, byte 4: `0xFF` off, 1 charging |
| SteelSeries Aerox / Prime Wireless | `00 92` (cable), `00 D2` (dongle) on interface 3 | byte 1: bit 7 charging, `(raw − 1) × 5` percent |
| SteelSeries Rival 3 Wireless, Rival 650 | `00 AA 01` | byte 0 percent, byte 2 ≠ 0 charging |
| ASUS ROG / TUF keyboards | `00 12 01` (Omni: `02 12 01` on `mi_02&col02`) | `12 01` echoed; byte 5 after the report id percent (Falchion: byte 10), byte 8 = 1 charging |
| ASUS ROG headsets | `CC 12 07` / `CC 12 08` on usage page `0xFF00` | byte 6 percent (Cetra SpeedNova: lower of bytes 6 and 7) / byte 5 = 1 charging |
| ASUS ROG / TUF mice | `00 12 07` output report on the model's interface (mostly `mi_00`); `03 12 07` on `mi_02&col03` (Omni receiver, Harpe II Ace dongle) | `12 07` echoed; byte 4 after the header pair level (percent; 0–4 quarters on Keris Wireless / EVA, Chakram, Pugio II, Gladius II, Strix Impact II Wireless; Strix Carry: quarters at byte 6), byte 9 charging; level 0 and not charging = mouse asleep; `FF AA` = error |
| ASUS ROG Omni receiver pairing | `01 A0 00 00` on `mi_02&col01` | from byte 5: 4-byte slots, product id little endian, 0 ends the list; first supported mouse and keyboard are queried |
| JBL Quantum 810 / 910P | Get Feature report `0x49` on interface 5 | `49 <percent>`; echoed id checked, 0 = headset off; no charging state |
| JBL Quantum 950 | Feature report `88 00 DD 01 00 01 00 02 00 <code>` on interface 5, usage page `0xFF12` | input `86 00 DD …`, bytes 11–12 code, byte 15 value; code `35 00` link (`0x80` connected), `0D 00` level |

A device that does not answer is not asked again for 10 minutes (2 minutes for devices
that are expected to sleep), so a misbehaving device cannot be flooded.

## JBL speakers over Bluetooth LE

The speakers of the `AA` frame generation (product ids in `jbl_ble.rs`: Charge 3/4/5/6, Flip 4/5/6/7, Xtreme 2/3/4, Pulse 3/4/5, Boombox/2/3, Go 4 (incl. Duo), Clip 5, Tuner 3) expose a vendor GATT service
`65786365-6c70-6f69-6e74-2e636f6d0000` (notify `…0001`, write `…0002`). Only while
such a speaker is paired and connected: find its LE address by an advertisement with
company id `0x0057` and the product id little endian at the start of the manufacturer
data (6 s active scan, only while the address is unknown), connect, enable
notifications, write `AA 11 00` (speaker info), collect the `AA 12 <len> 00 <token> …`
burst for up to 3 s, disconnect. Windows may answer the service lookup by UUID with an
empty list right after connecting; the complete service list is the fallback. Token `0x44`: low 7 bits percent, bit 7 charging.
Every 5 minutes, at most one speaker per round. A failure forgets the (rotating) address
and waits 10, 20, 40, then 60 minutes. Not covered: Wi-Fi and classic-only models, and
the `DD 00` generation (Boombox 4, Xtreme 5, Go 5, Grip, Essential 3 SE), for which no
battery query is documented.

## Merging

Order of precedence: G HUB, HID++, vendor HID, Bluetooth. Two entries are the same
device if their normalised names match (brand prefixes and marketing suffixes removed)
and their kinds do not contradict. A radio or cable link wins over Bluetooth, a
connected entry over a disconnected one. Logitech devices that stop answering stay
listed as idle for 10 minutes.
