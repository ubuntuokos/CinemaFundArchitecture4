# Settings and Hardware menu (Qt 6)

> **Last updated:** 2026-11-27 · **Change:** minor (the groups and the Accelerators and Decoding & Encoding pages are in the Qt editor's Preferences dialog, `cfa4-qt`; the menu plan moved into the repo)

The Settings window of the Qt 6 editor, `cfa4-qt` (`python/cfa4_qt/panels/settings.py` and
`settings_hardware.py`, Edit ▸ Preferences ▸ <page>, Ctrl+,). Phases 1 and 2 were first built in
the experimental Qt shell (`python/cfa4_devtools/qt_settings.py`, `cfa4-qt-shell`), which keeps its
own copy for now; the editor is the one users see ([plans/CFA4-SETTINGS-HW-002.md](plans/CFA4-SETTINGS-HW-002.md)). It merges the CFA3 Control Center layout, the FA3 SystemSettingsPage draft and the
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
- Edit ▸ Preferences ▸ Accelerators (GPU / NPU)… and Decoding & Encoding… open them.
- Tests: `python/tests/test_qt_settings_hardware.py` (stub engine; the built engine when there is
  one), `cargo test -p cfa4-cli --bin cfa4-cli serve::`.

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

`settings.window`, `settings.categories`, `settings.page`, `settings.category.<page>`,
`settings.<key>` (and `settings.<unitKey>` for a duration's unit), `settings.ok`,
`settings.cancel`, `settings.reset`, `settings.accelerators.devices`,
`settings.accelerators.plans`, `settings.accelerators.refresh`, `settings.codecs.table`,
`settings.codecs.hardwareDecoder`, `settings.codecs.gstreamer`, `settings.codecs.refresh`,
`settings.mediaBackends.status`, `settings.mediaBackends.list`, `settings.mediaBackends.refresh`,
`settings.mediaBackends.restricted`, `settings.mediaBackends.restricted.list`,
`settings.mediaBackends.restricted.install`, `settings.mediaBackends.restricted.copy`,
`menu.edit.settings`, `menu.edit.mediaBackends`, `menu.help.restrictedCodecs`.

## Tests

`python3 -m unittest python/tests/test_settings_backend.py` (with `PYTHONPATH=python`): the CLI
contract and the data model always; the window itself when PySide6 is installed
(`QT_QPA_PLATFORM=offscreen`); a round trip through the real engine when `target/debug/cfa4-cli`
(or `CFA4_CLI`) exists. Rust: `cargo test -p cfa4-engine settings`, `cargo test -p cfa4-cli`.
