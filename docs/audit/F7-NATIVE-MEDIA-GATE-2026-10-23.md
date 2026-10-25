# CFA4 F7 – Native media capability truth and A/V timestamp oracle

Target: Ubuntu 26.04 AMD64. The upstream parent is read-only; CFA4 is the product.

## Closed evidence (do not repeat)
- 2026-10-22 RTX A1000 true native Rust/CFA4 H.264/HEVC NVDEC/NVENC,
  application-side export/reimport: prior physical PASS with counters.
- FFmpeg 9.0.2 CUDA/Vulkan/OpenCL CLI audit: prior physical PASS.
- Tarball manual picture, sound, playback, scrubbing, export/reimport:
  prior user-acknowledged smoke PASS, **not** a sync timing measurement.
- Integration PR #23 GitHub full CI all eight jobs PASS on its merge commit.

## Newly prepared timestamp audit

`python -m cfa4_devtools.media_timestamp_audit /absolute/path/to/a-real-audio-video.mp4
--seconds 10 --output target/audit/f7-timestamps.json`

The input must contain **both** an audio and video stream and provide
actual FFprobe packet PTS, DTS, and duration evidence. The 10-second default
probe is capped at 30 seconds and 15,000 packets. Decode timestamps must
increase monotonically; missing packet data or audio is a failure, not a
silent fallback. By default the first audio/video packet PTS discrepancy
must be at most 250 ms; end-of-window discrepancy at most 500 ms.
Tolerances are explicit in each output and may need adjustment for media
priming, edit lists, and non-zero start times.

**Scope:** even `PASS_CONTAINER_TIMESTAMPS_ONLY` does not prove that
audio/visual content (e.g. lip movements vs waveform) is synchronized.
It does not compare source-vs-export content, codec paths, long-term drift,
or Qt6 GUI playback. Perceptual and end-to-end A/V precision are still OPEN
until independent real-source measurements are recorded.

The CI test runs synthetic fixtures only and is not GPU/hardware evidence.

## Capabilities deliberately unresolved

- CFA4 native Rust AV1 NVDEC: **NOT_IMPLEMENTED**. FFmpeg CLI's AV1
  NVDEC support is not a substitute.
- HEIF ICC/EXIF/10-bit HDR/multi-picture round-trip: **NOT_IMPLEMENTED**.
  Current libheif path converts the first picture to RGBA8/sRGB.
- Full Qt6 nonlinear editor: **NOT_IMPLEMENTED**. Current native editor is egui.
- HEVC/Qt/HEIF patent and licensing, full package dependencies, distribution:
  **RELEASE HOLD**.

Machine-readable status is kept in
`docs/audit/f7-media-capability-status.json`. Its CI validation ensures
the unresolved feature claims cannot silently turn into release PASS.

No packages are installed and no prior user media/GUI test is rerun.
