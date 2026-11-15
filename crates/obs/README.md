# cfa4-obs

Live studio core for CFA4 (layer L5), phase S0 of
[CFA4-LIVE-STUDIO-001](../../docs/plans/CFA4-LIVE-STUDIO-001.md): the OBS Studio core (**libobs**)
does the real-time work now, behind a Rust trait, in its own supervised process; CFA4's own Rust
code replaces it part by part later (plan S12+).

## What it contains

| Module | What |
|---|---|
| `StudioBackend` (lib.rs) | the compositor trait: video settings, empty scene, frames out. Every backend implements it; callers see only it |
| `software` | the pure-Rust backend: always available; the fallback when libobs cannot start. S0: the empty scene (opaque black frames, paced) |
| `libobs` | the binding of the pinned CFA4 libobs 33.0.1 build, loaded at run time from `$CFA4_LIBOBS_DIR/lib/libobs.so.30` (default `/opt/cfa4/libobs-33.0.1`); any other version is refused. `ffi` holds the transcribed declarations (with layout tests against the installed headers), `sys` is the only module with `unsafe` |
| `ring` | the frame ring: program frames in a private (0600) shared-memory file in `$XDG_RUNTIME_DIR`; torn frames are dropped, never shown. Layout in the module docs |
| `host` | `Supervisor`: starts `cfa4-studio-host`, watches it, restarts it after a crash (100 ms back-off, doubling to 5 s; gives up after 5 crashes a minute), stops it (quit, then kill after 2 s) |
| `ptp` | ST 2110 addendum T1: the PTP state `ptp4l` reports, read with `pmc -u -b 0 -d $CFA4_PTP_DOMAIN` (default 127, ST 2059-2; 2 s timeout; linuxptp runs PTP, CFA4 only reads, plan D9), the TAI time (system UTC + the grandmaster's `currentUtcOffset`) and the `ts-refclk` / `mediaclk` an ST 2110 sender signals now (the grandmaster and `direct=0` when locked, else `localmac` of `$CFA4_PTP_INTERFACE` or the first interface, and `sender`). Shown as `ptp` in `live.studio.host.status` |

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
