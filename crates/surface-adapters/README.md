# cfa4-surface-adapters

Hardware control surface adapter processes (layer L5) for plan CFA4-HW-CONTROL-SURFACES-001
(integration annex E2, E5, E13, `docs/plans/CFA4-HW-CONTROL-SURFACES-001-FINAL-1.1-CFA4-ILLESZTES.md`,
added with the M0 audit).

An adapter family that brings its own runtime or C code runs in its own process, so a crash or a
hang in a device library ends that family's process, never CFA4 (plan §5.4):

```
cfa4-cli surfaces --adapter mirabox-family          # the child process; JSON lines on stdin/stdout
cfa4-cli surfaces --adapter mirabox-family --probe  # lists the family's HID devices and exits
```

## Wire protocol (`cfa4-surface-adapter`, version 1)

One JSON object per line ([`src/wire.rs`](src/wire.rs)). The child writes events on stdout:
`hello` (first), `connected` (model, USB id, firmware, `serialHash` (never the serial), whether the
serial is unique, the control groups), `disconnected`, `input` (`control` `keys` or `buttons`,
0-based `index`, `press` / `release`), `ack` (for a command with a `seq`) and `error` (a machine
code: `permission_denied`, `device_busy`, `firmware_unknown`, `protocol_mismatch`, `io_timeout`,
`io_error`, `bad_request`, `device_not_found`, `unsupported`, `internal`).

It reads commands on stdin: `setImage` (RGBA, base64, at most 1024 px a side; the adapter scales,
rotates and encodes it for the device), `clearImage` (one key or all), `setBrightness`, `sleep`
and `quit`. End of input is `quit`.

The supervisor (`cfa4-surface` in `cfa4-cli serve`, annex Q1) turns events into `SurfaceEvent`s
of `cfa4-surface-model`; that part waits for the shared base of CFA4-CONTROL-SURFACE-001.

## Adapter loop

[`run`](src/lib.rs) is generic over a `Backend`. It scans for devices every second, opens a new
one once and keeps that connection for as long as the device is present, and runs each device in
its own tokio task: a panic inside the device library becomes an `internal` error and a
`disconnected` for that device only, and the device is opened again after a hold-off. An open
failure (for example `permission_denied` without the udev `uaccess` rule) is reported once per
hold-off, not on every scan. The `testing` feature has a backend without hardware for tests.

## mirabox-family

The real backend ([`src/mirabox.rs`](src/mirabox.rs), feature `mirabox`, on by default) uses
[mirajazz](https://crates.io/crates/mirajazz) 0.16 (MPL-2.0) on async-hid's hidraw backend (MIT),
unchanged. Its known panic paths are kept out of reach: the protocol version is checked before
`Device::connect` asserts on it, key number 0 is ignored instead of subtracted from, and the
`DeviceWatcher` (which unwraps) is not used.

Device profiles are in [`profiles/mirabox.toml`](profiles/mirabox.toml): USB id, HID usage,
protocol version, image format and the map from display position to device key index. The two
AJAZZ AKP153E profiles (`0300:1010`, rev. 2 `0300:3010`) are `confirmed = false`: their values
come from references (sources in the file), so the catalogue keeps the features UNTESTED until the
hardware test wizard writes a report from a real unit. The first step on the owner's unit is
`cfa4-cli surfaces --adapter mirabox-family --probe`.

Open questions for the real unit: whether the AKP153E reports key release on protocol 1 (mirajazz
reports each press as press + release there), and whether it needs a periodic keep-alive.
