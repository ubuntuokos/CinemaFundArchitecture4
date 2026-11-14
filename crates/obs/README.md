# cfa4-obs

Live studio core for CFA4 (layer L5), phases S0–S1 of
[CFA4-LIVE-STUDIO-001](../../docs/plans/CFA4-LIVE-STUDIO-001.md): the OBS Studio core (**libobs**)
does the real-time work now, behind a Rust trait, in its own supervised process; CFA4's own Rust
code replaces it part by part later (plan S12+).

## What it contains

| Module | What |
|---|---|
| `StudioBackend` (lib.rs) | the compositor trait: video settings, a scene, frames out. Every backend implements it; callers see only it |
| `scene` | S1: the scene the engine sends (`HostScene`: layers bottom first, each a picture and OBS's scene-item geometry: crop, scale, rotation about the top-left corner, position), checked and loaded (`HostScene::load`), and `compose`, the software compositor of it |
| `software` | the pure-Rust backend: always available; the fallback when libobs cannot start. Composites the scene once per change (opaque black under it) and paces the frames |
| `libobs` | the binding of the pinned CFA4 libobs 33.0.1 build, loaded at run time from `$CFA4_LIBOBS_DIR/lib/libobs.so.30` (default `/opt/cfa4/libobs-33.0.1`); any other version is refused. A scene is a private libobs scene of `cfa4_picture` sources (an asynchronous video source type registered from Rust, plan V.3; no libobs plugin needed), placed with OBS's own scene-item transform. `ffi` holds the transcribed declarations (with layout tests against the installed headers), `sys` is the only module with `unsafe` |
| `ring` | the frame ring: program frames in a private (0600) shared-memory file in `$XDG_RUNTIME_DIR`; torn frames are dropped, never shown. Layout in the module docs |
| `host` | `Supervisor`: starts `cfa4-studio-host`, watches it, restarts it after a crash (100 ms back-off, doubling to 5 s; gives up after 5 crashes a minute), stops it (quit, then kill after 2 s) |

## How the pieces fit (plan V)

```text
cfa4-cli serve (engine)                         cfa4-studio-host (apps/cfa4-studio-host)
 live.studio.host.start ── Supervisor ──spawn──► libobs 33.0.1 (or software backend)
 live.studio.host.status   (restart on crash)    empty scene ──raw video──► RingWriter
 live.studio.frame ◄──── RingReader ◄──── frame ring (shared memory, survives restarts)
```

The engine's methods are in `apps/cfa4-cli/src/serve_live.rs`; `live.studio.frame` answers like
`frame.render` (RGBA bytes, or the `shm` ring), so the Qt6 monitors can show it unchanged once the
Qt6 side asks for it (after the Qt6 GUI is finished, plan D7).

## Scenes (S1)

The scenes live in the project (`cfa4_project::studio`, schema v16) and are edited by the engine's
undoable `live.studio.scene.*` / `live.studio.source.*` commands (`crates/engine/src/studio.rs`).
After every request that changed the current scene, `cfa4-cli serve` renders its visible sources
with CFA4's own code (colour; picture files through CFA4's decoders; text through the title
renderer), writes the pictures as raw RGBA files into a private (0700) directory next to the frame
ring, and sends the host `{"cmd":"scene","scene":{…}}`. The host loads and checks the scene on its
stdin thread and shows it from the next frame (`{"event":"sceneShown","revision":…}`); a scene that
cannot be loaded is an `error` event and the shown scene stays. The supervisor keeps the last scene
and gives it to every restarted host.

```text
project (studio scenes) ── live.studio.source.set … ──► engine (undo, save)
cfa4-cli serve upkeep ── render pictures (raw RGBA, 0700 dir) ── {"cmd":"scene"} ──► cfa4-studio-host
                                                                      software: compose · libobs: cfa4_picture items
```

## Never crash

- libobs is C code: it runs only in `cfa4-studio-host`. A crash there (segfault, abort) ends that
  process; the supervisor records it (`crashes`, `lastExit`) and restarts it, the engine and the
  editor carry on. Tested: `apps/cfa4-studio-host/tests/supervisor.rs` kills the host with
  SIGKILL and makes it abort repeatedly.
- `unsafe` only in `libobs::sys` (ADR 0001 addendum, 2026-11-07), `// SAFETY:` on every block, the
  raw-video callback under `catch_unwind`, no raw pointers in the public API.
- Every parameter (video size, frame rate, ring headers written by another process) is checked;
  errors are `StudioError`, never panics.

## Building libobs

```sh
scripts/dev/build-ffmpeg-9.0.2.sh      # libobs 33 needs FFmpeg >= 8.0: CFA4's own 9.0.2
scripts/dev/build-libobs-33.0.1.sh     # → /opt/cfa4/libobs-33.0.1 (licence, source SHA-256, checksums in share/doc/libobs)
```

Ubuntu's own package is not used: 26.04 ships OBS 32.1.0 (`libobs0t64`, `libobs-dev`) without the
browser source; 24.04 ships 30.0.2. S0 builds libobs and libobs-opengl only (no frontend, no
scripting, no core plugins); later phases add the plugins they need.

## Tests

```sh
cargo test -p cfa4-obs -p cfa4-studio-host
# with the libobs build and a display (CI: Xvfb), libobs itself runs in the host:
Xvfb :99 & DISPLAY=:99 cargo test -p cfa4-obs -p cfa4-studio-host
```

The libobs tests skip (with a note) when the build or a display is missing; the software-backend
and supervisor tests always run.
