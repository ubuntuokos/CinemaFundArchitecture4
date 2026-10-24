# CFA4 Qt 6 GUI

The Qt 6 GUI (`python/cfa4_qt/`, PySide6) is the CFA4 editor frontend that replaces the egui
editor. The Rust engine stays: the GUI starts it as a child process (`cfa4-cli serve`) and
drives it with engine commands. Decision record: `docs/adr/ADR-F8-QT6-RUST-ENGINE-2026-10-23.md`.
Point-by-point parity with egui (the condition for deleting egui): [`qt-parity.md`](qt-parity.md).

## Running it

```sh
cargo build --release -p filmcraft-cli          # the engine
python -m pip install -e '.[qt]'               # PySide6
cfa4-qt                                         # or: python -m cfa4_qt
cfa4-qt project.fcproj                          # open a project
cfa4-qt --demo --lang hu                        # demo project, Hungarian UI
```

The engine is found as `--engine PATH`, `$CFA4_ENGINE`, `cfa4-cli` / `filmcraft-cli` on `PATH`,
then `target/release` and `target/debug` of the checkout. `--smoke [--screenshot out.png]`
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
| `frame.render` | `{time \| seconds \| frame, scale?, maxWidth?, maxHeight?}` | `{w, h, format: "rgba8", time, offline}` + `w·h·4` bytes (sRGB over black) |
| `audio.render` | `{start (samples) \| time \| seconds, frames}` | `{sampleRate, channels, frames, start, format: "f32le"}` + interleaved samples |
| `session.state` | – | `{dirty, revision, path, canUndo, canRedo, playhead}` |
| `app.quit` | – | `{quit: true}`, then the session ends (as stdin EOF does) |

`frame.render` renders the active sequence's Program picture without moving the playhead; the
frame fits `maxWidth × maxHeight` (16–8192) and is never upscaled. `audio.render` mixes at most
1 920 000 sample frames per call. Times are engine ticks (254 016 000 000 per second).

## Code map

| File | Role |
|---|---|
| `apps/filmcraft-cli/src/serve.rs` | engine server (protocol, frame/audio rendering, upkeep loop) |
| `python/cfa4_qt/engine.py` | process, framing, request/reply/event plumbing |
| `python/cfa4_qt/commands.py` | registry → menus, actions, shortcuts; parameter specs |
| `python/cfa4_qt/main_window.py` | docks, refresh cycle, command routing, close/recovery prompts |
| `python/cfa4_qt/panels/` | Project, Timeline, Program monitor + playback, Effects, Inspector, History, Events |
| `python/cfa4_qt/i18n.py`, `i18n/*.tsv` | translations (shared catalogs + Qt/Hungarian) |
| `python/tests/test_qt_gui.py` | framing, registry, i18n, window against a fake and the real engine |
