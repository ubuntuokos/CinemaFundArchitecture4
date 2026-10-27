# cfa4-ffmpeg

Optional FFmpeg backend for CFA4 (layer L5). CFA4's own Rust media engine stays the base;
FFmpeg (at least 9.0.2, GPL-3 build) supplements it for files the engine does not decode yet.
Decision and rules: [ADR 0005](../../docs/adr/ADR-0005-media-backends.md).

## What it does

- `probe()`: the FFmpeg this process linked: version, licence, configuration, hardware
  acceleration methods, and whether the patent-restricted encoders of the Ubuntu model
  (`libx264`, `libx265`; [`docs/licensing.md`](../../docs/licensing.md) §7) are present. A build
  older than 9.0.2, without GPL-3, or with `--enable-nonfree` is refused.
- `discover(path)`: container, duration and streams.
- `open(path)`: an engine `MediaSource` decoded by FFmpeg at the stream's **native** layout and
  bit depth (8-bit and 9–16-bit planar Y'CbCr go to the engine as `Yuv8` / `Yuv16`, other layouts
  as RGBA 8-bit), audio as 32-bit float at the rate the engine asks for.
- `register()`: makes FFmpeg the engine's first fallback backend, before GStreamer. The built-in
  openers stay first; FFmpeg takes only what they reject. `set_enabled()` switches it at run time.
- `register_status()` / `report()`: the entry of Settings ▸ Media & Playback ▸ Media Backends
  (`media.backends`, `cfa4-cli media-backends`).
- `route_backend()`: the `ffmpeg` backend of the compute router (`cfa4-route`, ADR 0004).

## Rules this crate keeps (measured in the rsmedia build trial, 2026-10-26)

- **Native pixel format.** rsmedia converts every frame to one target format (8-bit 4:2:0 by
  default) and has no pass-through. The source asks for the stream's own format, so 10-bit and
  4:2:2 media keep their precision. Tested bit-exact against `ffmpeg -f framemd5` for H.264 High
  4:2:2 10-bit, HEVC 4:2:2 10-bit, H.264 High 10, DV, DVCPRO HD, MPEG-4 Part 2 in AVI and WMV2.
- **No crash from FFmpeg.** rsmedia calls `CString::new(..).unwrap()` on paths and URLs, so a NUL
  byte panics inside it. Every path is checked first (existing regular file, absolute, UTF-8, no
  NUL byte, so no `concat:` or `http:` protocol either), and every call into rsmedia runs under
  `catch_unwind`. A panic becomes an `Error::Internal`.
- **Pinned sources.** rsmedia is not on crates.io: it is pinned to a reviewed commit, its rsmpeg
  fork by `Cargo.lock`; `deny.toml` allows exactly these two git sources.
- **No `unsafe`** in this crate.

## Building

The `ffmpeg` feature is **off by default**: without it the workspace builds and tests without
FFmpeg and every entry point returns `Error::Unavailable`.

```sh
sudo apt install build-essential nasm pkg-config libva-dev libdrm-dev libdav1d-dev libvpx-dev \
  libopus-dev libwebp-dev libmp3lame-dev libvulkan-dev libffmpeg-nvenc-dev libvpl-dev
scripts/dev/build-ffmpeg-9.0.2.sh                 # → /opt/cfa4/ffmpeg-9.0.2 (base)
export PKG_CONFIG_PATH=/opt/cfa4/ffmpeg-9.0.2/lib/pkgconfig
cargo test -p cfa4-ffmpeg --features ffmpeg
cargo build --release -p cfa4 -p cfa4-cli --features cfa4/ffmpeg,cfa4-cli/ffmpeg
```

The apps link FFmpeg dynamically and look for it at run time in
`/opt/cfa4/ffmpeg-9.0.2-restricted/lib` first and `/opt/cfa4/ffmpeg-9.0.2/lib` second
(`apps/*/build.rs`, `CFA4_FFMPEG_PREFIX` moves both). The restricted build, the same FFmpeg with
x264 and x265, is what the `cfa4-restricted-extras` package installs:

```sh
sudo apt install libx264-dev libx265-dev
scripts/dev/build-ffmpeg-9.0.2.sh --restricted   # → /opt/cfa4/ffmpeg-9.0.2-restricted
```

## Not measured yet

VA-API and Vulkan Video encoding on a non-NVIDIA GPU, and NVENC / NVDEC on the RTX A1000
reference machine: the trial container has no GPU.
