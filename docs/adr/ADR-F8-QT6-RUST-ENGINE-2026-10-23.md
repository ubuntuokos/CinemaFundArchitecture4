# CFA4 ADR F8 – Qt6 editor / Rust engine boundary

**Date:** 2026-10-23  
**Status:** ACCEPTED_ARCHITECTURE_FOR_IMPLEMENTATION; FULL_QT6_NLE_NOT_IMPLEMENTED  
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

\`\`\`text
CFA4 Qt6 Widgets GUI (future production NLE)
  -> Qt controller / signals / asynchronous request worker
     -> bounded local CFA4 CLI --bridge on 127.0.0.1
        -> retained Rust project/media/render/export engine
           -> GPU/CPU codecs (NVDEC/NVENC, software fallback)
\`\`\`

The first source contract is \`python/cfa4_devtools/qt_control_bridge.py\`.
It issues **read-only** \`perf.stats\` and \`sequence.inspect\` queries
against an already-running local Rust editor. It validates command
allowlists, local loopback port, short timeouts, JSON envelopes, errors
and CLI executable. It does **not** spawn arbitrary commands or open an
outbound connection. The current Qt6 development shell is intentionally
**not wired into this bridge** until the controller lifecycle, worker
thread, project isolation and error handling have been tested.

The CLI search prefers the installed \`cfa4-cli\` binary, then the
inherited \`cfa4-cli\` while transition binaries still carry that
internal name. No CFA4 release server is consulted.

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

1. **Controller safety:** read-only, bounded local JSON bridge; source
   regressions. This PR implements the source contract only.
2. **Worker ownership:** QThread/QThreadPool request/response signals,
   cancellation, subprocess lifecycle, error/timeout handling. OPEN.
3. **Project/media browser:** local file import, project save/open,
   permission/error path, no hidden network. OPEN.
4. **Timeline:** multi-track edits, selection, undo/redo and clip
   operations against Rust project model, no duplicated truth. OPEN.
5. **Preview, audio, GPU:** correct frame clock, audio/video precision,
   CPU fallback and physical RTX A1000 behavior. OPEN.
6. **Export + reimport:** H264/HEVC with real audio, FFprobe metadata,
   progress/cancel, integrated Qt6 GUI and CFA4 project round-trip.
   OPEN; earlier egui success does NOT transfer to Qt6.
7. **Packaging:** correctly package tested Qt6 runtime, licenses,
   XDG desktop and all four Ubuntu 26.04 AMD64 formats. OPEN.

**Result:** Qt6 is the chosen future UI, Rust owns media and project data.
Do not mark F8 runtime/UI acceptance until phases 2–7 have their own
recorded tests. The earlier egui/NVENC/NVDEC hardware and GUI evidence
remains accepted without repetition.
