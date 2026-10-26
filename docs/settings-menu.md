# Settings and Hardware menu (Qt 6)

> **Last updated:** 2026-10-26 · **Change:** minor (Media & Playback ▸ Media Backends page, Edit ▸ Media Backends…, Help ▸ Restricted Codecs…)

The Settings window of the Qt 6 shell (`python/cfa4_devtools/qt_settings.py`, Edit ▸ Settings…,
Ctrl+,). It merges the CFA3 Control Center layout, the FA3 SystemSettingsPage draft and the
current CFA4 preferences. New settings UI is Qt 6 / PySide6 only
([ADR F8](adr/ADR-F8-QT6-RUST-ENGINE-2026-10-23.md)); the egui window is not extended.

Full menu plan (all groups, Kdenlive / OpenShot mapping, codec settings, AI, publishing and
providers, open questions): <https://claude.ai/artifact/7tCP6EREnCdZ43owCSJPBC>.

## How it works

- The pages are the engine's settings schema (`cfa4_engine::settings`). Each category now names
  the **group** it is listed under (`Category::group`, `settings::groups()`), and `prefs.schema`
  returns `{"groups": [...], "categories": [...]}`.
- The window talks to the engine only through short local `cfa4-cli` processes on a QThreadPool
  worker, never on the GUI thread:
  - `cfa4-cli settings schema|get|set|reset [arg]` reads and writes `<data dir>/preferences.json`
    through the same validated `prefs.*` commands as the desktop app (`--data-dir`, else
    `CFA4_DATA_DIR` / `XDG_DATA_HOME/cfa4`).
  - `cfa4-cli hardware` prints the accelerator inventory (`cfa4-accel` sysfs probe), the backend
    chain per workload and the vendor coverage.
  - `cfa4-cli media-backends` prints the media backends in the order they are tried (the CFA4
    engine, then FFmpeg and GStreamer, [ADR 0005](adr/ADR-0005-media-backends.md)), each with
    whether it is compiled in, available and switched on, its version, licence and problem, and
    the patent-restricted codecs of the Ubuntu model ([licensing.md](licensing.md) §7).
- OK saves only the changed keys; Reset Page… resets one category after a confirmation and keeps
  unsaved edits of the other pages. Settings the engine stores but does not act on yet are shown
  greyed out with "no effect yet".

## Groups (phase 1)

| Group | Pages |
|---|---|
| General | General, Auto Save, Plugins |
| Appearance | Appearance, Labels |
| Editing | Color, Graphics, Timeline, Trim |
| Media & Playback | Audio, Media, **Media Backends** (switches + live status and restricted codecs, Qt only), Media Cache, Playback |
| Hardware & Devices | **Accelerators (GPU / NPU)** (read-only, Qt only), Audio Hardware, Memory |
| Local AI | Media Analysis & Transcription |

Removed from the schema in phase 1 (keys still load from old files): `media.hardwareDecoding`
(an unwired duplicate of `playback.hardwareDecoding`) and `media.proresHardwareEncoding`
(macOS VideoToolbox only, not a Linux target).

## Menu access

| Menu item | Opens |
|---|---|
| Edit ▸ Settings… (Ctrl+,) | the Settings window at its first page |
| Edit ▸ Media Backends… | the Settings window at Media & Playback ▸ Media Backends |
| Help ▸ Restricted Codecs… | the same page: which patent-restricted codecs are installed and the command that adds them (copied, never run) |

## Later phases

Decoding & Encoding page (codec settings from [codec-parity.md](codec-parity.md)), devices
(cameras, storage, monitors, drawing tablets), Local AI models, Publishing & providers, and the
System (root) section with confirmation-gated helpers (GPU Unlock as an optional, separately
installed component). Secret storage (tokens, passwords) is designed in a separate security
project; no token value is ever written to settings files, logs or projects.

## Automation ids

`settings.window`, `settings.categories`, `settings.page`, `settings.category.<page>`,
`settings.<key>` (and `settings.<unitKey>` for a duration's unit), `settings.ok`,
`settings.cancel`, `settings.reset`, `settings.accelerators.devices`,
`settings.accelerators.plans`, `settings.accelerators.refresh`, `settings.mediaBackends.status`,
`settings.mediaBackends.list`, `settings.mediaBackends.refresh`, `settings.mediaBackends.restricted`,
`settings.mediaBackends.restricted.list`, `settings.mediaBackends.restricted.install`,
`settings.mediaBackends.restricted.copy`, `menu.edit.settings`, `menu.edit.mediaBackends`,
`menu.help.restrictedCodecs`.

## Tests

`python3 -m unittest python/tests/test_settings_backend.py` (with `PYTHONPATH=python`): the CLI
contract and the data model always; the window itself when PySide6 is installed
(`QT_QPA_PLATFORM=offscreen`); a round trip through the real engine when `target/debug/cfa4-cli`
(or `CFA4_CLI`) exists. Rust: `cargo test -p cfa4-engine settings`, `cargo test -p cfa4-cli`.
