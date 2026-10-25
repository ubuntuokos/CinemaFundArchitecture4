# Qt 6 GUI ↔ egui parity (point by point)

**Owner decision (2026-10-24):** the egui editor (`crates/ui-egui`, `apps/filmcraft`) is deleted
**only when the Qt 6 GUI does, point by point, everything the egui editor does.** Until then:

- the egui editor stays the working editor and is **frozen**: bug fixes and security fixes only,
  no new features or panels;
- every new UI feature is built in the Qt 6 GUI (`python/cfa4_qt/`);
- a row of this table becomes ✅ only with evidence (a test in `python/tests/` or a recorded
  manual check on the reference machine), as `VISION.md` §2.4 requires;
- removing egui needs every row ✅ **and** the owner's explicit go-ahead, recorded in
  `docs/adr/ADR-F8-QT6-RUST-ENGINE-2026-10-23.md`.

Legend: ✅ done with evidence · 🟡 partly (what is missing is named) · ❌ not started.
"Generic form" means the command runs from its menu with an auto-generated parameter form
instead of egui's purpose-built dialog — usable, but **not** parity.

## 1. Foundation

| # | Point | egui | Qt 6 | Evidence / missing |
|---|---|---|---|---|
| 1.1 | Rust engine owns all project data; GUI holds views only | ✅ in-process | ✅ child process `cfa4-cli serve` | `apps/filmcraft-cli/src/serve.rs` tests; `test_qt_gui.py` |
| 1.2 | Every engine command reachable from the menus | ✅ | ✅ menus built from `engine.commands` | `test_registry_builds_menus_and_shortcuts` |
| 1.3 | Keyboard shortcuts (engine bindings, user presets) | ✅ | 🟡 | engine bindings mapped; panel-scoped keys and the shortcut editor dialog missing |
| 1.4 | Menu enablement follows the selection/state | ✅ | ✅ refreshed after every change | `test_main_window_with_fake_engine` |
| 1.5 | Auto-save and crash-recovery journal | ✅ | 🟡 | `serve --autosave`, Recover / Not Now / Discard prompt; needs a kill-the-engine recovery test |
| 1.6 | Save / Don't Save / Cancel on close | ✅ | 🟡 | implemented; no automated test yet |
| 1.7 | Engine crash does not take the GUI down | n/a | 🟡 | GUI reports it and keeps running; automatic engine restart + recovery missing |
| 1.8 | Translations (12 languages) | ✅ | 🟡 | shared TSV catalogs + full Hungarian `hu.tsv` for the Qt GUI (`test_hungarian_catalog_covers_*`: every Qt string, every engine menu label); panel-specific engine data (track names) not translated, other languages only through the shared catalogs |
| 1.9 | Wayland and X11 on every Ubuntu 26.04 flavour | 🟡 | ❌ | to be checked per flavour (VISION §3) |
| 1.10 | Packaging (.deb, AppImage, Flatpak, tar.gz) with the Qt runtime | n/a | ❌ | ADR F8 phase 7 |
| 1.11 | Agent/automation control (`--control`, `ui.*`, screenshots, MCP bridge) | ✅ | ❌ | Qt equivalent of `ui.inspect`/`ui.elements`/`ui.click`; object names are already set |
| 1.12 | Never-crash guards (panic hook, `catch_unwind`) | ✅ | 🟡 | engine requests run under `catch_unwind`; Python-side exception hook missing |

## 2. Workspace and docking

| # | Point | egui | Qt 6 | Evidence / missing |
|---|---|---|---|---|
| 2.1 | Dockable, tabbable, floating panels | ✅ | ✅ QDockWidget | layout saved per user; Qt 6.10–6.12 tab-bar bugs worked around (`test_layout_has_no_orphaned_tab_bars`, docs/qt-gui.md) |
| 2.2 | Named workspaces (save, switch, reset) | ✅ | 🟡 | only "Reset Panel Layout" |
| 2.3 | Themes (light/dark, favourites) | ✅ | 🟡 | follows the desktop's Qt/KDE theme; no in-app theme switch |
| 2.4 | Panel docking commands (`window.floatPanel`, `window.dockPanel`, …) | ✅ | ❌ | |

## 3. Panels (egui `PanelKind`)

| # | Panel | Qt 6 | Evidence / missing |
|---|---|---|---|
| 3.1 | Project (bins, list) | 🟡 | tree, search, double-click opens, drag to timeline, import/new bin/new sequence; icon view, columns, view presets, rename, bin tabs missing |
| 3.2 | Media Browser | ❌ | |
| 3.3 | Libraries | ❌ | |
| 3.4 | Info | ❌ | |
| 3.5 | Effects | 🟡 | browse, search, apply effect / default-transition; favourites, badges, presets missing |
| 3.6 | Markers | ❌ | markers drawn on the ruler only |
| 3.7 | History | 🟡 | list shown; click-to-step missing |
| 3.8 | Source monitor | 🟡 | picture and sound of the opened clip from the engine (`target: "source"`), time bar with In/Out, step/start/end, Mark In/Out, Insert/Overwrite; keys with a `target` follow the focused monitor (`test_real_engine_source_monitor`); drag to the timeline, zoom, safe margins, J/K/L, audio-only waveform view, recent-clips menu missing |
| 3.9 | Effect Controls (keyframes, masks, transform handles) | 🟡 | panel next to Source: every effect of the selected clip with on/off, reset, remove; number, angle, point, checkbox, choice, colour and text parameters editable (a drag is one undo step); stopwatch, keyframe add/remove/previous/next, keyframe strip with click-to-go and interpolation menu; values at the playhead from the engine (`effects.inspect`; `test_real_engine_effect_controls`, engine `effects_inspect_tests`); curves/wheels/masks, value graphs, Bézier handles, keyframe dragging and copy/paste, on-monitor transform handles missing |
| 3.10 | Audio Clip Mixer | ❌ | |
| 3.11 | Metadata | ❌ | |
| 3.12 | Program monitor | 🟡 | frames from the engine, transport, timecode, time bar with In/Out, Mark In/Out; zoom, safe margins, guides, rulers, comparison view, playback resolution menu missing |
| 3.13 | Timeline | 🟡 | tracks, clips, ruler, markers, in/out, playhead, select, move (snap, insert, copy), trim (regular/ripple), razor, drop from Project, zoom, horizontal and vertical scrolling (`test_every_track_is_reachable_in_a_short_timeline`), whole-timecode ruler; track header buttons, waveforms, thumbnails, keyframe lanes, transitions drawing, captions track, roll/slip/slide tools, sequence tabs missing |
| 3.14 | Tools | 🟡 | Selection and Razor only |
| 3.15 | Audio Meters | ❌ | |
| 3.16 | Audio Track Mixer | ❌ | |
| 3.17 | Lumetri Color | ❌ | |
| 3.18 | Lumetri Scopes | ❌ | |
| 3.19 | Essential Graphics | ❌ | |
| 3.20 | Essential Sound | ❌ | |
| 3.21 | Text to Speech | ❌ | |
| 3.22 | Properties | 🟡 | Inspector shows the selected clip's timing, speed, gain |
| 3.23 | Text (transcript, captions) | ❌ | |
| 3.24 | Events | ✅ | engine toasts and errors logged |
| 3.25 | Progress (export/render jobs) | 🟡 | export queue with status, progress bar and remaining time in the Export window; a dockable Progress panel and render-job progress missing |
| 3.26 | Reference Monitor | ❌ | |
| 3.27 | Timecode | ❌ | |
| 3.28 | Project Notes | ❌ | |

## 4. Modes and dialogs

| # | Point | Qt 6 | Evidence / missing |
|---|---|---|---|
| 4.1 | Import mode | ❌ | File ▸ Import (native file dialog) works |
| 4.2 | Export mode (presets, settings, queue, progress) | 🟡 | Export window: presets, formats, output file, range (entire, In/Out, work area, custom), frame size, bitrate mode/target, hardware encoding, caption burn-in, sound and audio codec, resolved summary, queue with progress/cancel/retry (`test_real_engine_export_through_the_dialog_and_reimport`: H.264 + AAC checked with ffprobe, re-imported); preset manager (save/delete/favourites/import), per-format panels (ProRes/DNx/APV profiles, EXR, loudness, captions sidecar), preview pane, Quick Export missing |
| 4.3 | Open / Save / Save As / Save a Copy | ✅ | native dialogs |
| 4.4 | Sequence settings, new sequence | 🟡 | generic form |
| 4.5 | Preferences (`settings.rs`) | ❌ | |
| 4.6 | Keyboard shortcuts editor | ❌ | |
| 4.7 | Clip dialogs (speed/duration, gain, frame hold, …) | 🟡 | generic form |
| 4.8 | Menu dialogs (Find, Scene Edit Detection, …) | 🟡 | generic form |
| 4.9 | Colour dialogs (interpret footage, working space) | 🟡 | generic form |
| 4.10 | Media dialogs (relink, proxies, ingest, Project Manager) | 🟡 | generic form |
| 4.11 | Frame export, interchange export (AAF/XML/EDL) | 🟡 | generic form |
| 4.12 | Multicam, Remix, Voice-over recording | ❌ | |

## 5. Playback

| # | Point | Qt 6 | Evidence / missing |
|---|---|---|---|
| 5.1 | Play/stop, step, start/end | ✅ | transport + Space |
| 5.2 | Sound during playback | 🟡 | `audio.render` → QAudioSink; A/V sync not yet measured |
| 5.3 | J/K/L shuttle, play around, loop in/out | ❌ | |
| 5.4 | Playback resolution, draft decoding, dropped-frame stats | ❌ | |
| 5.5 | GPU frame path (no RGBA copy per frame) | ❌ | frames travel as RGBA8 over a pipe; shared memory/dmabuf later |
| 5.6 | Trim monitor, dynamic trimming | ❌ | |
