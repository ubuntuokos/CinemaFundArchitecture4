# cfa4-ffmpeg

Optional FFmpeg backend for CFA4 (layer L5). CFA4's own Rust media engine stays the base;
FFmpeg (at least `MIN_VERSION`, GPL-3 build) supplements it for files the engine does not decode yet.
Decision and rules: [ADR 0005](../../docs/adr/ADR-0005-media-backends.md).

## What it does

- `probe()`: the FFmpeg this process linked: version, licence, configuration, hardware
  acceleration methods, and whether the patent-restricted encoders of the Ubuntu model
  (`libx264`, `libx265`; [`docs/licensing.md`](../../docs/licensing.md) §7) are present. A build
  older than `MIN_VERSION` (`CFA4_FFMPEG_MIN_VERSION` in `packaging/ffmpeg-version.env`), without GPL-3, or with `--enable-nonfree` is refused.
- `discover(path)`: container, duration and streams.
- `open(path)`: an engine `MediaSource` decoded by FFmpeg at the stream's **native** layout and
  bit depth (8-bit and 9–16-bit planar Y'CbCr go to the engine as `Yuv8` / `Yuv16`, other layouts
  as RGBA 8-bit), audio as 32-bit float at the rate the engine asks for.
- Panasonic RW2 stills (`open` routes a file that starts with `IIU\0` to `src/rw2.rs`): FFmpeg's
  `rw2` decoder gives the 16-bit Bayer mosaic with the RAW colour parameters and EXIF side data;
  `cfa4-dng` develops it (`develop_mosaic`) with the user's DNG camera profile when one is
  installed ([`docs/camera-raw.md`](../../docs/camera-raw.md)). Tests: the mosaic is bit-exact
  with the ffmpeg CLI's, the levels and white balance agree with the file's own tags, and the
  developed picture is compared with the embedded JPEG preview (a printed ΔE00 table, no
  threshold), against the CC0 samples of `scripts/dev/fetch-rw2-samples.sh`.
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
- **No `unsafe`** in this crate. Side data that rsmedia hands over as bytes (`AVRawColorParams`)
  is read field by field at the bound struct's `offset_of!` offsets.
- **Still images are declined**, except Panasonic RW2 (the engine has no decoder for it).

## Building

The `ffmpeg` feature is **off by default**: without it the workspace builds and tests without
FFmpeg and every entry point returns `Error::Unavailable`.

```sh
sudo apt install build-essential nasm pkg-config libva-dev libdrm-dev libdav1d-dev libvpx-dev \
  libopus-dev libwebp-dev libmp3lame-dev libvulkan-dev libffmpeg-nvenc-dev libvpl-dev glslc git
scripts/dev/build-ffmpeg.sh                 # → /opt/cfa4/ffmpeg-$CFA4_FFMPEG_VERSION (base; on Ubuntu 24.04/26.04 it downloads the prebuilt one, scripts/dev/fetch-ffmpeg.sh)
. packaging/ffmpeg-version.env                # CFA4_FFMPEG_VERSION, the pinned version
export PKG_CONFIG_PATH=/opt/cfa4/ffmpeg-$CFA4_FFMPEG_VERSION/lib/pkgconfig
scripts/dev/fetch-rw2-samples.sh             # RW2 test samples → target/fixtures/rw2 (CC0, SHA-256 pinned)
cargo test -p cfa4-ffmpeg --features ffmpeg
cargo build --release -p cfa4 -p cfa4-cli --features cfa4/ffmpeg,cfa4-cli/ffmpeg
```

The apps link FFmpeg dynamically and look for it at run time in
`/opt/cfa4/ffmpeg-$CFA4_FFMPEG_VERSION-restricted/lib` first, `/opt/cfa4/ffmpeg-$CFA4_FFMPEG_VERSION/lib` second and
`$ORIGIN/../lib/cfa4/ffmpeg-$CFA4_FFMPEG_VERSION/lib` third, the copy the tarball, AppImage and Flatpak carry
next to the binary (`apps/*/build.rs`, `CFA4_FFMPEG_PREFIX` moves the first two). The release
packages ship this build (ADR 0005 step 4, `packaging/linux/stage-ffmpeg.sh`): the `.deb` installs
it into `/opt/cfa4/ffmpeg-$CFA4_FFMPEG_VERSION`. The restricted build, the same FFmpeg with
x264 and x265, is what the `cfa4-restricted-extras` package installs:

```sh
sudo apt install libx264-dev libx265-dev
scripts/dev/build-ffmpeg.sh --restricted   # → /opt/cfa4/ffmpeg-$CFA4_FFMPEG_VERSION-restricted
```

## Not measured yet

VA-API and Vulkan Video encoding on a non-NVIDIA GPU, and NVENC / NVDEC on the RTX A1000
reference machine: the trial container has no GPU.
