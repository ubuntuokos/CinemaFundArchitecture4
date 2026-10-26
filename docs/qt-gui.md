# CFA4 Qt 6 GUI

The Qt 6 GUI (`python/cfa4_qt/`, PySide6) is the CFA4 editor frontend that replaces the egui
editor. The Rust engine stays: the GUI starts it as a child process (`cfa4-cli serve`) and
drives it with engine commands. Decision record: `docs/adr/ADR-F8-QT6-RUST-ENGINE-2026-10-23.md`.
Point-by-point parity with egui (the condition for deleting egui): [`qt-parity.md`](qt-parity.md).

## Running it

Until the Qt GUI is merged into `main`, it lives on the `cfa4/qt6-gui` branch: switch to it
first, or the engine you build has no `serve` command and there is no `cfa4-qt`.

Ubuntu 26.04 does not let `pip` install into the system Python (PEP 668,
"externally-managed-environment"), so PySide6 goes into a virtual environment in the checkout:

```sh
git switch cfa4/qt6-gui                         # not needed once it is merged into main
cargo build --release -p cfa4-cli          # the engine (must include `serve`)
sudo apt install python3-venv                   # once, if `python3 -m venv` is missing
python3 -m venv .venv
.venv/bin/python -m pip install -e '.[qt]'      # PySide6 + cfa4-qt, inside .venv only
.venv/bin/cfa4-qt --demo --lang hu              # demo project, Hungarian UI
.venv/bin/cfa4-qt project.fcproj                # open a project
```

`source .venv/bin/activate` lets you type `cfa4-qt` without the path in that terminal.

**Taskbar name and logo.** KDE and GNOME take a window's name and icon from the installed
`.desktop` file of its app id (`io.github.ubuntuokos.cfa4`). Packages install it; a checkout
run has none, so the panel shows "python3". Install a user-level entry once (it is never done
without asking, and an installed package's entry is never overridden):

```sh
.venv/bin/cfa4-qt --install-desktop-entry      # then start CFA4 again
```

The engine is `--engine PATH` or `$CFA4_ENGINE` when given; otherwise the build of this checkout
(`target/release` or `target/debug`, the newer first), then `cfa4-cli` on
`PATH`. A candidate without the `serve` subcommand (an engine built from a branch without the
Qt GUI, or an older installed CLI) is skipped, and if none is usable the GUI lists every path
it checked and why. `--smoke [--screenshot out.png]`
starts, waits for the engine, and quits (CI). `--no-autosave` turns off the auto-save and
crash-recovery journal, `--data-dir` moves it.

## Architecture

```text
Qt 6 GUI process (Python, PySide6)               Engine process (Rust, cfa4-cli serve)
  MainWindow, panels, menus  ──JSON lines──▶  stdin   Session: project, history, commands,
  EngineClient (reader thread) ◀──JSON+binary── stdout  media pool, render, audio mix, export,
                                                       auto-save journal
```

- **One source of truth.** Panels hold the last `project.inspect` / `sequence.inspect` replies
  and redraw when the engine reports `projectChanged`. Every edit is an engine command, so undo,
  the journal, auto-save, CLI and MCP see the same edits as the GUI.
- **Menus and shortcuts come from the engine** (`engine.commands`: id, label, menu path,
  shortcut, params, enabled). A command whose parameters the user must give gets a parameter
  form; commands taking paths get native file dialogs. Purpose-built dialogs replace the
  generic form one by one (see the parity list).
- **Never block the event loop.** Requests are written from the GUI thread and return at once;
  a reader thread delivers replies and events through Qt signals. The monitor keeps at most one
  frame and one audio request in flight and drops stale frames instead of queueing them.
- **A crash in one command does not end the session**: the engine runs each request under
  `catch_unwind` and answers with an error. If the engine process itself dies, the GUI says so;
  unsaved changes are in the recovery journal and are offered on the next start.
- **No network.** The engine talks only over its stdin/stdout pipes, which only the GUI holds.

## Protocol (`cfa4-cli serve`, version 1)

One request per line on stdin, a JSON object:

```json
{"id": 7, "method": "engine.execute", "params": {"command": "timeline.razor", "params": {"time": 508032000000}}}
```

Replies on stdout, one per request, in order: `{"id": 7, "ok": true, "result": …}` or
`{"id": 7, "ok": false, "error": "…"}`. A reply with `"binary": N` is followed by exactly `N` raw
bytes. Lines with an `event` key and no `id` are notifications:
`projectChanged {revision}`, `toast {message, error}`, `openSequence {item}`, `openSource {item}`,
`revealInProject {item}`. A line that is not a request gets an error reply with `id: null`; a
line longer than 4 MiB is skipped with an error reply.

| Method | Params | Result |
|---|---|---|
| `hello` | – | `{protocol, engine, pid, autosave, recovery: [candidate…]}` |
| `engine.execute` | `{command, params}` | the command's result (any engine command id) |
| `engine.commands` | – | the registry (`command.list`: id, label, menu, shortcut(s), params, enabled) |
| `frame.render` | `{target?: "program" \| "source", item?, time \| seconds \| frame, scale?, maxWidth?, maxHeight?}` | `{w, h, fullW, fullH, pixelAspect, format: "rgba8", time, target, offline \| item}` + `w·h·4` bytes (sRGB over black); `fullW × fullH` is the frame's own size, `pixelAspect` its pixel aspect ratio (the monitors draw it at its display aspect) |
| `audio.render` | `{target?, item?, start (samples) \| time \| seconds, frames, sampleRate? (source only), meter?}` | `{sampleRate, channels, frames, start, format: "f32le", target, meter?}` + interleaved samples; with `meter: true`, `meter` is `{block, peaks, loudness, reset}`: per `block` samples (1/100 s) the sample peak of each channel (linear) and `[M, S, I LUFS, TP dBTP]` so far (`null` below −70); loudness keeps integrating over consecutive requests of the same target and starts over (`reset: true`) on a jump; for the Program `meter.strips` is `[{id, peaks}]`, each mixer strip's peak over the block (linear, post-fader, per channel; the Mix is id 2⁶⁴−1). The Program is mixed through the live mixer state, so a fader held in the Audio Track Mixer is heard. The Audio Meters and the Audio Track Mixer show these |
| `scopes.image` | `{kind, time?, waveformType?, paradeType?, colorSpace?, brightness?, scale?, clamp?, rows?}` | `{kind, w, h, format: "rgba8premultiplied", lo, hi, nits, colorSpace, parades, nitsTicks?, extent?, skinToneDeg?, targets?, hls?}` + `w·h·4` bytes: one Lumetri scope's trace image of the active sequence's frame; the GUI draws the graticule |
| `mediaBrowser.inspect` | `{path?, probe?}` | `{dir, canBack, canForward, canUp, selection, favorite, fileTypes, view, columns, importAsImageSequence, hoverScrub, thumbnailSize, recent, createFolderForImportedProjects, allowDuplicateMedia, ingest, fileTypeChoices, extensions, allColumns, entries: [{name, path, isDir, kind, numbered, inProject, cells}], pending, error?}`: the Media Browser; `cells` are the list columns' texts, probing at most `probe` (8) new files per call (`pending` are left) |
| `mediaBrowser.thumbnail` | `{path, fraction?, width?}` | `{w, h, format: "rgba8", path, time}` + RGBA8: a file's thumbnail `fraction` (0.3) into it, in the project or not |
| `lumetri.inspect` | `{clip?, time?}` | `{clip, name, hdr, effect, enabled, mediaTime, start, end, params: {id: {type, value, label, animated, …}}, luts: {input, look}}`: the Lumetri Color panel; `clip: null` without a selected video clip, `effect: null` without a Lumetri effect; floats carry `min`/`max`/`softMin`/`softMax`/`decimals`/`default`, curves their points and the drawn `line` (128 y values) |
| `text.inspect` | `{time?}` | `{sequence, graphics: [{clip, layer, start, timecode, track, name, text, at}], graphicSelection: {clips, layers}, captionFormats, speech}`: the Text panel's Graphics tab (every text layer of the sequence, by start, track, layer; `at` is the time its text is read and set at), the caption formats and whether speech-to-text is available |
| `clipMixer.inspect` | `{time?}` | `{time, recording, strips: [{track, ref, name, muted, solo, channels, mode, clip: {id, name, volumeDb, pan} \| null}]}`: the Audio Clip Mixer's strips |
| `time.readout` | `{rows: [{source: program\|source, mode: current\|media\|duration\|inOut\|remaining, display?}], time?, sourceTime?}` | `{rows: [{value, mode, source, name}]}`: the Timecode panel's rows (`time`/`sourceTime` stand in for the playheads while playing) |
| `time.parse` | `{text, target?}` | `{time, frame}`: a timecode typed into a monitor, read as Premiere does (`01:02:03:04`, `1.2.3.4`, bare digits as right-aligned `HHMMSSFF`, `+15` / `-1.00` relative to the monitor's frame) at the sequence's or the Source clip's rate |
| `media.peaks` | `{item, start, end (media ticks), buckets ≤ 20000}` | `{item, buckets, start, end, hasAudio}` + `buckets` (min, max) pairs, f32le, channels mixed to mono (≤ 600 s per call) |
| `session.state` | – | `{dirty, revision, path, canUndo, canRedo, playhead}` |
| `app.quit` | – | `{quit: true}`, then the session ends (as stdin EOF does) |

`frame.render` renders the active sequence's Program picture without moving the playhead; the
frame fits `maxWidth × maxHeight` (16–8192) and is never upscaled. `audio.render` mixes at most
1 920 000 sample frames per call. Times are engine ticks (254 016 000 000 per second).

With `target: "source"` both render the Source monitor instead: the clip open there (or `item`),
at the source playhead unless a time is given, in media time. A source picture comes from the
decoder by whole-number decimation, so it is at least the asked size, not exactly it; the
source sound is the media's own stream as stereo at `sampleRate` (default 48 000).

## Known Qt issues

Checked 2026-10-24 with PySide6 6.10.2, 6.10.3, 6.11.2 and 6.12.0 (offscreen and minimal
platforms), each with a minimal reproduction that contains no CFA4 code:

- **Tabifying docks in two dock areas before `show()`** leaves an orphaned copy of a tab bar
  drawn over the panels (6.10.3, 6.11.2, 6.12.0).
- **`restoreState()` on a window that is on screen** makes new tab bars and leaves the old ones
  visible at the same place, one more set per call (6.10.3, 6.11.2, 6.12.0). A stale bar on top
  of the live one swallows the clicks on the tabs.
- **Tabifying docks after `show()`** can crash in `tabifyDockWidget` (6.10.2: segmentation fault
  in 8 of 12 runs).
- **Restoring a tabbed state into a window whose docks are not tabbed yet** is clean on every
  version above, before or after `show()`.

So the main window never tabifies its own docks. The default layout is arranged in a hidden
stand-in window with empty docks of the same names (`MainWindow._default_layout_state`), and the
real window only restores states: the saved layout, or that default. View ▸ Reset Panel Layout
restores the default on the live window and hides every tab bar that existed before the restore
when a new bar shows the same tabs. `test_layout_has_no_orphaned_tab_bars` checks first start,
start with a saved layout and reset; it passes on all four versions, so `PySide6>=6.10,<7` stays.

## Code map

| File | Role |
|---|---|
| `apps/cfa4-cli/src/serve.rs` | engine server (protocol, frame/audio rendering, upkeep loop) |
| `python/cfa4_qt/panels/effect_controls.py` | Effect Controls: parameters, stopwatch, keyframes (`effects.inspect` + `effects.*` commands) |
| `python/cfa4_qt/panels/export.py` | Export window: presets, formats, range, overrides, queue with progress |
| `python/cfa4_qt/engine.py` | process, framing, request/reply/event plumbing |
| `python/cfa4_qt/commands.py` | registry → menus, actions, shortcuts; parameter specs |
| `python/cfa4_qt/main_window.py` | docks, refresh cycle, command routing, close/recovery prompts |
| `python/cfa4_qt/panels/` | Project, Timeline, Program monitor + playback, Effects, Inspector, History, Events |
| `python/cfa4_qt/i18n.py`, `i18n/*.tsv` | translations (shared catalogs + Qt/Hungarian) |
| `python/tests/test_qt_gui.py` | framing, registry, i18n, window against a fake and the real engine |

## Export window

File ▸ Export ▸ Media… (and `export.queue.add`, Ctrl+M) opens a non-modal Export window. It
lists the engine's presets (`export.presets.list`) and formats (`export.formats`), shows what
the current choice resolves to (`export.resolve`: size, rate, bitrate, sound, frames) and sends
the export to the engine's queue (`export.queue.add`), which the engine server encodes between
requests; the queue table polls `export.queue.list` for status, progress and remaining time,
with start/stop, cancel, retry, remove and clear. **Only the settings the user changed override
the preset** — bitrate mode, audio codec, sound on/off, hardware encoding and caption burn-in
are not sent until touched, so a preset's own choices are never silently replaced. Frame size
is always sent when set (the fields show the preset's size; 0 × 0 is the preset's or the
sequence's). `test_real_engine_export_through_the_dialog_and_reimport` exports through the
window, checks the file with ffprobe (H.264 + AAC, the asked size) and imports it back.
