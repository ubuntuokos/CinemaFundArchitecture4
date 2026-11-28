# Settings and Hardware menu (Qt 6)

> **Last updated:** 2026-11-28 · **Change:** minor (the experimental shell's Settings window was removed; the Qt editor is the only Settings window)

The Settings window of the Qt 6 editor, `cfa4-qt` (`python/cfa4_qt/panels/settings.py` and
`settings_hardware.py`, Edit ▸ Preferences ▸ <page>, Ctrl+,). Phases 1 and 2 were first built in
the experimental Qt shell (`cfa4-qt-shell`); that copy was removed (CFA4-SETTINGS-HW-003 R5), so the
editor is the only Settings window ([plans/CFA4-SETTINGS-HW-002.md](plans/CFA4-SETTINGS-HW-002.md)). It merges the CFA3 Control Center layout, the FA3 SystemSettingsPage draft and the
current CFA4 preferences. New settings UI is Qt 6 / PySide6 only
([ADR F8](adr/ADR-F8-QT6-RUST-ENGINE-2026-10-23.md)); the egui window is not extended.

Full menu plan (all groups, Kdenlive / OpenShot mapping, codec settings, AI, publishing and
providers, open questions): [plans/CFA4-SETTINGS-MENU-001.md](plans/CFA4-SETTINGS-MENU-001.md).
Next step (approved 2026-11-27): [plans/CFA4-SETTINGS-HW-002.md](plans/CFA4-SETTINGS-HW-002.md).

## How it works

- The pages are the engine's settings schema (`cfa4_engine::settings`). Each category now names
  the **group** it is listed under (`Category::group`, `settings::groups()`), and `prefs.schema`
  returns `{"groups": [...], "categories": [...]}`.
- The window talks to the engine only through short local `cfa4-cli` processes on a QThreadPool
  worker, never on the GUI thread:
  - `cfa4-cli settings schema|get|set|reset [arg]` reads and writes `<data dir>/preferences.json`
    through the same validated `prefs.*` commands as the desktop app (`--data-dir`, else
    `CFA4_DATA_DIR` / `XDG_DATA_HOME/cfa4`).
  - `cfa4-cli hardware` prints the accelerator inventory (`cfa4-route` sysfs probe), the backend
    chain per family from the routing table and the vendor coverage.
  - `cfa4-cli codecs` prints what this machine decodes and encodes: CFA4's own codecs
    ([codec-parity.md](codec-parity.md)), the hardware decoder `cfa4_platform::register` found
    (NVDEC or VA-API, for H.264 and HEVC), the export format that encodes each codec (own
    software encoder, or a registered hardware one where CFA4 has none) and the GStreamer backend's state.
  - `cfa4-cli media-backends` prints the media backends in the order they are tried (the CFA4
    engine, then FFmpeg and GStreamer, [ADR 0005](adr/ADR-0005-media-backends.md)), each with
    whether it is compiled in, available and switched on, its version, licence and problem, and
    the patent-restricted codecs of the Ubuntu model ([licensing.md](licensing.md) §7).
- OK saves only the changed keys; Reset Page… resets one category after a confirmation and keeps
  unsaved edits of the other pages. Settings the engine stores but does not act on yet are shown
  greyed out with "no effect yet".

## In the editor (`cfa4-qt`, CFA4-SETTINGS-HW-002)

- The category list is grouped under the schema's groups (headings `settings.group.<id>`); an
  engine without groups keeps the flat list.
- Hardware & Devices ▸ **Accelerators (GPU / NPU)** and Media & Playback ▸ **Decoding & Encoding**
  are Qt-only pages. Their data comes over the editor's engine channel (`cfa4-cli serve`):
  `hardware.inventory` and `codecs.capabilities` give the same JSON as `cfa4-cli hardware` and
  `cfa4-cli codecs` ([qt-gui.md](qt-gui.md)), probed once per session and again on Detect Again.
- Decoding & Encoding shows `playback.hardwareDecoding` and `playback.draftDecode`; they stay on
  the Playback page too, and Reset… is off on both Qt-only pages.
- Edit ▸ Preferences ▸ Hardware Overview…, Accelerators (GPU / NPU)…, Decoding & Encoding… and System Settings (root)… open them.
- Tests: `python/tests/test_qt_settings_hardware.py` (stub engine; the built engine when there is
  one), `cargo test -p cfa4-cli --bin cfa4-cli serve::`.

## Overview and the device page (`cfa4-qt`, CFA4-SETTINGS-HW-003 R4)

- Hardware & Devices ▸ **Overview** is the group's first page (`settings_overview.py`): one
  read-only row each for the accelerators, decoding, the system components, the control surfaces,
  the audio devices and the memory, with warnings (for example "a group membership is missing")
  and an Open… button to the row's own page. It asks nothing new of the engine: it reads
  `hardware.inventory`, `codecs.capabilities`, `system.components`, `surface.devices`, Qt's audio
  devices and the window's picture memory. Rows of pages the schema lacks are left out.
- **The device page** (`device_page.py`): a device list with each device's status on the left, tabs
  on the right (Details, Capabilities, Drivers, Diagnostics). Accelerators is built on it: the
  status and the Diagnostics tab come from the system helper's GPU-access check, the Capabilities
  tab keeps the device and per-job tables. Later device pages (displays, storage, cameras, tablets)
  reuse it under their own plans.
- Object names: `settings.overview.<page>`, `.summary`, `.open`; `settings.accelerators.page.list`,
  `.tabs`, `.tab.<details|capabilities|drivers|diagnostics>`.

## System Settings (root) (`cfa4-qt`, CFA4-SETTINGS-HW-003)

- Hardware & Devices ▸ **System Settings (root)** is the group's last, Qt-only page
  (`python/cfa4_qt/panels/settings_system.py`). It shows the components of `cfa4-system-helper`
  ([ADR 0017](adr/ADR-0017-rendszerszintu-komponensek.md), [README](../apps/cfa4-system-helper/README.md)):
  GPU access (`render` / `video` group), VA-API drivers and the NPU check, each with its status
  (*Not needed*, *OK*, *Missing*, *Waiting for log out*, *Not checked*) and the facts behind it.
- One way for every change: **Check** (no root) → **Plan…** (no root; the exact changes and how they
  are undone) → **Apply…** (only the plan the user looked at; a confirmation, then the system's
  password dialog through `pkexec`) → the page checks again. **Undo…** undoes only what the helper
  did. The Log tab lists what happened in this window.
- Engine side: `system.components`, `system.plan`, `system.apply`, `system.disable`,
  `system.uninstall` ([qt-gui.md](qt-gui.md)); the changes answer later, from a worker, so the
  editor keeps working under the password dialog.
- Without the helper (tarball, AppImage, Flatpak, or no `pkexec`) the page says so and only shows
  what is missing. Accelerators and Decoding & Encoding link here when GPU access or a driver is
  missing (`settings.system.open`).
- Tests: `python/tests/test_qt_settings_hardware.py`, `cargo test -p cfa4-system-helper`,
  `cargo test -p cfa4-cli --bin cfa4-cli serve_system::`.

## Groups (phase 1)

| Group | Pages |
|---|---|
| General | General, Auto Save, Plugins |
| Appearance | Appearance, Labels |
| Editing | Color, Graphics, Timeline, Trim |
| Media & Playback | Audio, Media, **Decoding & Encoding** (Qt only), **Media Backends** (switches + live status and restricted codecs), Media Cache, Playback |
| Hardware & Devices | **Accelerators (GPU / NPU)** (read-only, Qt only), Audio Hardware, Memory |
| Local AI | Media Analysis & Transcription |

Removed from the schema in phase 1 (keys still load from old files): `media.hardwareDecoding`
(an unwired duplicate of `playback.hardwareDecoding`) and `media.proresHardwareEncoding`
(a removed macOS-only option).

## Menu access

| Menu item | Opens |
|---|---|
| Edit ▸ Settings… (Ctrl+,) | the Settings window at its first page |
| Edit ▸ Media Backends… | the Settings window at Media & Playback ▸ Media Backends |
| Help ▸ Restricted Codecs… | the same page: which patent-restricted codecs are installed and the command that adds them (copied, never run) |

## Decoding & Encoding (phase 2)

The page holds the decoding settings that work today, moved here from Playback in the Qt window
(same keys, `playback.hardwareDecoding` and `playback.draftDecode`), and a read-only "On this
machine" table with the hardware decoder, the GStreamer backend and one row per codec. When no
hardware decoder is found the page says which vendor driver to install; CFA4 never installs
drivers itself. Reset Page is off on the Qt-only pages: their keys belong to engine categories
(Playback) and reset with them.

Planned for this page once the engine acts on them: decode / encode / proxy threads, a backend
per codec where several exist (own software, VA-API, NVDEC, GStreamer), decoder and encoder GPU,
maximum hardware resolution, and the default GPU rendering of exports.

## Later phases

Devices
(cameras, storage, monitors, drawing tablets), Local AI models, Publishing & providers, and the
System (root) section with confirmation-gated helpers (GPU Unlock as an optional, separately
installed component). Secret storage (tokens, passwords) is designed in a separate security
project; no token value is ever written to settings files, logs or projects.

## Automation ids

`settings` (the window), `settings.categories`, `settings.page`, `settings.category.<page>`,
`settings.<key>` (and `settings.<unitKey>` for a duration's unit), `settings.ok`,
`settings.cancel`, `settings.reset`, `settings.accelerators.devices`,
`settings.accelerators.plans`, `settings.accelerators.refresh`, `settings.codecs.table`,
`settings.codecs.hardwareDecoder`, `settings.codecs.gstreamer`, `settings.codecs.refresh`. The
Media Backends dialog (`python/cfa4_qt/media_backends.py`) uses `dialog.mediaBackends` and
`mediaBackends.*`.

## Tests

`python/tests/test_qt_settings_hardware.py` (stub engine; the built engine when there is one). Rust: `cargo test -p cfa4-engine settings`, `cargo test -p cfa4-cli`.
