# Contributing

Bug reports, device reports and pull requests are welcome.

## Reporting a device

Most protocol code was written from documentation and protocol analysis and has not
been run against every device. If a device shows a wrong level or does not appear:

1. Start `BatteryWidget.exe --verbose`, reproduce the problem, then attach
   `%LOCALAPPDATA%\BatteryWidget\app.log`.
2. Include the vendor and product id (Device Manager → Details → Hardware Ids) and how
   the device is connected (Bluetooth, receiver, cable).

## Development

```powershell
cargo test --workspace                 # unit tests (also run on Linux/macOS)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo run                              # debug build
```

Rules of the house:

- **Read-only.** Drivers send documented *query* commands only, and only to endpoints
  identified by vendor id, product id and report layout. No pairing, configuration or
  firmware commands, and no speculative probing of unknown devices.
- **Pure first.** Packet building, parsing and matching go into platform-neutral
  functions with unit tests; only the I/O lives behind `#[cfg(windows)]`.
- **No telemetry, no network** except the optional local G HUB socket on `127.0.0.1`.
- Keep dependencies minimal.

## Adding a vendor driver

Add a module under `crates/core/src/vendor/`, export a `Driver` constant and register it
in `DRIVERS` (`vendor/mod.rs`). `steelseries.rs` is a good template for table-driven
devices, `playstation.rs` for devices that are only listened to.
