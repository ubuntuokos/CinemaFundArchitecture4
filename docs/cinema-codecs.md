# Cinema-grade codecs: Dolby, DTS, OpenMAX, JPEG 2000 / DCP

> **Last updated:** 2026-11-11 · **Change:** minor (the clean-room text aligned with AGENTS.md §2: porting a permitted, registered implementation is allowed)

Which public specification each cinema-grade format needs and which licence or patent constraint
applies. Every row follows [AGENTS.md](../AGENTS.md) §2: codecs are written in Rust
from public specifications by default; porting an existing implementation is allowed when its
licence is permitted by [licensing.md](licensing.md) §3 and the port is recorded in the reused-code
register (§5); FFmpeg and GStreamer
follow the Media backends rule in the AGENTS.md CFA4 section, and other system libraries are reached
only through `crates/platform` ([ADR 0001](adr/0001-platform-ffi.md)).

**This file does not track status.** What CFA4 decodes and encodes today, what is missing and the
effort estimates live in one place only: [codec-parity.md](codec-parity.md) for codecs,
[file-format-parity.md](file-format-parity.md) for DCP and image sequences. The patent review of
each format is in [patent-review.md](patent-review.md). Before 2026-11-11 this file had its own
status column; it was not updated when the JPEG 2000, DTS and TrueHD decoders landed on
2026-10-26, so it is gone.

## Specifications and licences

| Area | Public specification | Licence / patents | Status |
|---|---|---|---|
| Dolby Digital (AC-3) | ATSC A/52:2012 | AC-3 patents expired (2017) | [codec-parity.md](codec-parity.md) (AC-3) |
| Dolby Digital Plus (E-AC-3) | ATSC A/52:2012 Annex E | essential patents mostly expired; check before shipping an encoder | [codec-parity.md](codec-parity.md) (E-AC-3) |
| Dolby Atmos in E-AC-3 (JOC) | ETSI TS 103 420 | ETSI FRAND licensing; Dolby trademark may not be used | [codec-parity.md](codec-parity.md) (Dolby Atmos in E-AC-3) |
| Dolby TrueHD (MLP), Atmos in TrueHD | **no public specification** (MLP is described only in the licence-gated DVD-Audio spec); ported from the Apache-2.0 `truehd` crate ([licensing.md](licensing.md)) | Dolby trademark may not be used | [codec-parity.md](codec-parity.md) (Dolby TrueHD) |
| Dolby AC-4 | ETSI TS 103 190-1 / -2 | FRAND, patents active: owner decision before work starts | [codec backlog](plans/CODEC-BACKLOG-2026-10-27.md) (K2, ⚖️) |
| DTS (core, Coherent Acoustics) | ETSI TS 102 114 | core patents expired | [codec-parity.md](codec-parity.md) (DTS) |
| DTS-HD Master Audio (XLL) | ETSI TS 102 114 v1.4+ (extension substream) | DTS trademark may not be used | [codec-parity.md](codec-parity.md) (DTS) |
| Auro-3D (Auro-Codec in 24-bit PCM) | **no public specification**; ported from harletty-bridge `auro` (Apache-2.0) | US 8,620,465 B2 in force (owner accepted the risk, #299) | [codec-parity.md](codec-parity.md) (Auro-3D) |
| JPEG 2000 (MXF, DCP) | ITU-T T.800 / ISO/IEC 15444-1, SMPTE ST 422 (MXF), ST 429 (DCP) | royalty-free by design | [codec-parity.md](codec-parity.md) (JPEG 2000) |
| DCP packaging | SMPTE ST 429-2/-3/-7/-8/-9 | open; encryption (KDM) out of scope | [file-format-parity.md](file-format-parity.md) (DCP) |
| ProRes, DNxHD / DNxHR | — | — | [codec-parity.md](codec-parity.md) |
| OpenEXR, DPX | OpenEXR file layout docs | BSD-3-Clause | [file-format-parity.md](file-format-parity.md) (OpenEXR, DPX) |
| OpenMAX IL hardware codecs | Khronos OpenMAX IL 1.1.2 | the API is open; components are vendor drivers | probe only, by design: see below |

## OpenMAX

OpenMAX IL is a Khronos C API that hardware vendors implement as shared-library components. On
Linux x86_64 it has been replaced by VA-API and Vulkan Video: Mesa removed its OpenMAX state
trackers, and today's OpenMAX components ship mainly on embedded boards (Raspberry Pi, older
NVIDIA Tegra). CFA4 is Linux x86_64 only and vendor-neutral, so:

- the GStreamer catalogue lists the `gst-omx` elements (`omxh264dec`, `omxh265dec`, `omxh264enc`,
  `omxh265enc`), so `cfa4_gstreamer::probe()` reports when a machine has OpenMAX components;
- there is **no** OpenMAX FFI in `crates/platform`: VA-API (AMD, Intel) and Vulkan Video (any
  vendor) stay the planned hardware paths ([hardware-parity.md](hardware-parity.md)), with the
  pure-Rust decoders as the fallback.

## Before work starts

Each codec needs its source in hand before work starts: either the public specification, or an
implementation whose licence [licensing.md](licensing.md) §3 permits, ported and recorded in the
reused-code register (§5) in the same commit ([AGENTS.md](../AGENTS.md) §2). A decoder is never
written from memory. The crate README records the spec edition or the ported source and its
revision. Where neither exists, the owner decides before work starts.
