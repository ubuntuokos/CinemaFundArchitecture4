# CFA4 ADR F8 – Qt6 editor / Rust engine boundary

**Date:** 2026-10-23  
**Status:** ACCEPTED_ARCHITECTURE_FOR_IMPLEMENTATION; QT6_GUI_IN_PROGRESS (see `docs/qt-parity.md`)  
**Amended:** 2026-10-24 (engine server boundary, egui freeze and deletion rule); 2026-11-08 (phase status, finish plan)  
**Release:** HOLD

## Decision

The CFA4 target editor GUI is **Qt6 Widgets + PySide6** on Ubuntu
26.04 AMD64, KDE Plasma/Wayland first, with GNOME/Cinnamon via standard XDG.
The existing Rust/Cargo media, project, timeline, playback, render,
codec, NVDEC/NVENC and export engine is **retained**. The inherited
Rust/egui NLE remains the working application until a Qt6 port has
demonstrated real feature parity; it is not called a complete Qt6 editor.

The upstream parent remains read-only. A new upstream feature or fix can
enter CFA4 only after independent review and tests. No cloud AI or
runtime network dependency is introduced.

## Boundary

```text
CFA4 Qt6 Widgets GUI (future production NLE)
  -> Qt controller / signals / asynchronous request worker
     -> bounded local CFA4 CLI --bridge on 127.0.0.1
        -> retained Rust project/media/render/export engine
           -> GPU/CPU codecs (NVDEC/NVENC, software fallback)
```

The first source contract is `python/cfa4_devtools/qt_control_bridge.py`.
It issues **read-only** `perf.stats` and `sequence.inspect` queries
against an already-running local Rust editor. It validates command
allowlists, local loopback port, short timeouts, JSON envelopes, errors
and CLI executable. It does **not** spawn arbitrary commands or open an
outbound connection. The current Qt6 development shell is intentionally
**not wired into this bridge** until the controller lifecycle, worker
thread, project isolation and error handling have been tested.

The CLI search prefers the installed `cfa4-cli` binary, then the
inherited `cfa4-cli` while transition binaries still carry that
internal name. No CFA4 release server is consulted.

## Amendment 2026-10-24: engine server, egui freeze, deletion rule

**Boundary changed.** The Qt GUI no longer reaches the engine through the running egui app's
control port (`--bridge`). It starts the engine itself as a child process, `cfa4-cli serve`,
and talks to it over the child's stdin/stdout (JSON lines with binary frame/audio tails;
protocol in `docs/qt-gui.md`). No egui window, no socket, no network:

```text
CFA4 Qt6 GUI (python/cfa4_qt, PySide6)
  -> EngineClient: pipe to the child process, reader thread, Qt signals
     -> cfa4-cli serve: headless Rust Session (all engine commands, frame.render, audio.render,
        auto-save + crash-recovery journal)
        -> retained Rust project/media/render/export engine
```

`qt_control_bridge.py` and the old `qt_app.py` shell stay for their existing tests; the GUI
does not use them.

**Owner decision (ubuntuokos, 2026-10-24).** egui is deleted only when the Qt 6 GUI does,
point by point, everything egui does. Until then egui is **frozen**: bug and security fixes
only; every new UI feature goes into the Qt 6 GUI. The point-by-point list and its evidence live
in `docs/qt-parity.md`; deletion needs every row ✅ and the owner's explicit go-ahead recorded
here.

## Rejected alternatives

- Reimplement the media engine in Python/Qt: duplicates tested Rust
  codecs/render/export, loses source and hardware test provenance.
- Replace egui before a functioning Qt6 timeline/import/export exists:
  destroys the already working editor during migration.
- Use a cloud/HTTP service to connect the Qt GUI to the engine:
  violates the fully local baseline and is unnecessary.
- Invoke blocking subprocesses on the Qt event thread: risks UI
  freezes, timeouts, and lost project state.

## Phased, independently testable acceptance

Status as of 2026-11-08 (`main`, `docs/qt-parity.md`: 59 of 63 rows ✅, 4 🟡, none ❌):

1. **Controller safety:** read-only, bounded local JSON bridge; source
   regressions. DONE (source contract; superseded for the GUI by the
   engine server of the 2026-10-24 amendment, kept for its tests).
2. **Worker ownership:** request/response signals, subprocess lifecycle,
   error handling. DONE (`EngineClient`, `cfa4-cli serve`; a dead engine is
   restarted and offers its unsaved changes: parity 1.5, 1.7, 1.12).
3. **Project/media browser:** local file import, project save/open,
   permission/error path, no hidden network. DONE (parity 3.1, 3.2, 4.3, 4.10).
4. **Timeline:** multi-track edits, selection, undo/redo and clip
   operations against Rust project model, no duplicated truth. DONE
   (parity 3.13, 3.14, 5.6; tested against the real engine).
5. **Preview, audio, GPU:** correct frame clock, audio/video precision,
   CPU fallback and physical RTX A1000 behavior. IN PROGRESS: frame clock,
   A/V sync (the picture follows the sound, measured offset under a frame),
   shuttle audio, draft decoding and dropped-frame statistics are done
   (parity 5.1–5.4); frames reach the monitors through shared memory, GPU
   upload without the RGBA8 frame (dmabuf / GL texture) is open (parity 5.5).
6. **Export + reimport:** H264/HEVC with real audio, FFprobe metadata,
   progress/cancel, integrated Qt6 GUI and CFA4 project round-trip.
   IN PROGRESS: the Qt Export window → engine queue → H.264 + AAC checked
   with ffprobe → re-imported runs automated with the real engine; parity 4.2
   is still 🟡. Earlier egui success does NOT transfer to Qt6.
7. **Packaging:** correctly package tested Qt6 runtime, licenses,
   XDG desktop and all four Ubuntu 26.04 AMD64 formats. IN PROGRESS: the
   .deb and the tar.gz ship the Qt GUI with Ubuntu's PySide6 as a dependency;
   the AppImage and the Flatpak do not carry it, and the desktop entry still
   starts egui (parity 1.10).

The remaining work (parity 1.9, 1.10, 4.2, 5.5) and the egui removal
steps are planned in `docs/plans/CFA4-QT6-FINISH-001.md`. That plan does
not decide anything recorded here: it lists the open decisions for the owner.

**Result:** Qt6 is the chosen future UI, Rust owns media and project data.
Do not mark F8 runtime/UI acceptance until phases 2–7 have their own
recorded tests. The earlier egui/NVENC/NVDEC hardware and GUI evidence
remains accepted without repetition.
