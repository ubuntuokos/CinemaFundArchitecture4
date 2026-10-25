# cfa4-gstreamer

Optional GStreamer backend for CFA4 (layer L5). The Rust media engine stays the primary path;
GStreamer supplements it where the system's media stack can do what the engine does not do yet.
Decision and rules: [ADR 0002](../../docs/adr/ADR-0002-gstreamer-backend.md).

## What it does today (steps 1 and 2)

- `probe()`: GStreamer version and whether each element of `CATALOGUE` is installed, with the
  plugin, package and licence the plugin declares. The catalogue covers the planned areas:
  vendor-neutral hardware decoding and encoding (VA-API, Vulkan Video), Qt6 preview
  (`qml6glsink`), capture (PipeWire, V4L2) and general pipeline elements.
- `discover(path, timeout)`: container, duration (exact `Tick`s), seekability and the video,
  audio and subtitle streams of a file, through GStreamer's `Discoverer`.
- `open(path)`: an engine `MediaSource` decoded by GStreamer (RGBA 8-bit video, 32-bit float
  audio at the requested rate, accurate seeks, sequential reads without seeking).
- `register()`: makes GStreamer the engine's fallback backend (`cfa4_media::fallback`). The
  built-in openers stay first; GStreamer takes files they reject as unsupported, or open without
  the video they carry (Ogg Theora + Vorbis opened as audio only), and adds `.ogv`, `.flv`, `.wmv`,
  `.wma` and similar extensions to import. The apps call it at startup when built with their
  `gstreamer` feature (`cargo run -p cfa4 --features gstreamer`).

## Building

The `gstreamer` feature is **off by default**, so the workspace builds and tests without
GStreamer. Without the feature, every entry point returns `Error::Unavailable`.

```sh
sudo apt install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev gstreamer1.0-tools
cargo test -p cfa4-gstreamer --features gstreamer
cargo run  -p cfa4-gstreamer --features gstreamer --example gst_probe -- some-file.mkv
```

Useful runtime packages for the catalogue on Ubuntu: `gstreamer1.0-plugins-bad` (VA-API and Vulkan
decoders), `gstreamer1.0-qt6` (`qml6glsink`), `gstreamer1.0-pipewire`, `gstreamer1.0-gl`.

## Rules

- No `unsafe`: only the safe gstreamer-rs API is used (workspace lints apply unchanged).
- Never crash: every failure is an `Error`; panics inside the bindings are caught and reported as
  `Error::Internal`; paths are checked before GStreamer sees them and timeouts are capped.
- Licences: GStreamer is LGPL-2.1-or-later and dynamically linked from the system. The catalogue
  avoids the GPL / patent-encumbered `ugly` and `libav` plugin sets, and a unit test keeps it so.
