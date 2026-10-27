# Cinema-grade codecs: Dolby, DTS, OpenMAX, JPEG 2000 / DCP

> **Last updated:** 2026-10-25 · **Change:** major (new file: plan for the Dolby, OpenMAX and other
> cinema-grade codecs requested for CFA4)

What CFA4 has, what is missing, which public specification each missing piece needs and which
licence or patent constraint applies. Every row follows [AGENTS.md](../AGENTS.md) §2: codecs are
clean-room Rust from public specifications, ffmpeg is a test oracle only, and system libraries are
reached only through the optional GStreamer backend
([ADR 0002](adr/ADR-0002-gstreamer-backend.md)) or `crates/platform`
([ADR 0001](adr/0001-platform-ffi.md)). Effort estimates use the same scale as
[codec-parity.md](codec-parity.md).

## Status

| Area | Have today | Missing | Public specification | Licence / patents | Est. |
|---|---|---|---|---|---|
| Dolby Digital (AC-3) | decode, all modes ([crates/ac3](../crates/ac3/README.md)) | encode | ATSC A/52:2012 | AC-3 patents expired (2017) | 20–30 h (encoder) |
| Dolby Digital Plus (E-AC-3) | decode, independent substream (7.1 plays as 5.1) | dependent substreams (7.1), adaptive hybrid transform, enhanced coupling | ATSC A/52:2012 Annex E | essential patents mostly expired; check before shipping an encoder | 10–20 h |
| Dolby Atmos in E-AC-3 (JOC) | none (plays the 5.1 bed) | object decoding, renderer to the speaker layout | ETSI TS 103 420 | ETSI FRAND licensing; Dolby trademark may not be used | 40–60 h |
| Dolby TrueHD (MLP) | none | lossless decode | **no public specification** (MLP is described only in the DVD-Audio spec, licence-gated) | clean-room needs a public spec: owner decision | — |
| Dolby AC-4 | none | decode | ETSI TS 103 190-1 / -2 | FRAND, patents active | 60–100 h |
| DTS (core, Coherent Acoustics) | none | decode | ETSI TS 102 114 | core patents expired | 25–40 h |
| DTS-HD Master Audio (XLL) | none | lossless extension | ETSI TS 102 114 v1.4+ (extension substream) | DTS trademark may not be used | 30–50 h |
| JPEG 2000 (MXF, DCP) | none | decode + encode, Part 1 and the cinema profiles | ITU-T T.800 / ISO/IEC 15444-1, SMPTE ST 422 (MXF), ST 429 (DCP) | royalty-free by design | 40–70 h |
| DCP packaging | none | Interop / SMPTE DCP write (CPL, PKL, ASSETMAP, MXF essence) | SMPTE ST 429-2/-3/-7/-8/-9 | open; encryption (KDM) out of scope | 20–30 h after JPEG 2000 |
| ProRes, DNxHD / DNxHR | decode + encode | ProRes RAW | — | — | see codec-parity.md |
| OpenEXR, DPX | read + write | EXR PIZ / B44 / DWA, tiled, multi-part | OpenEXR file layout docs | BSD-3-Clause | 15–25 h |
| OpenMAX IL hardware codecs | **probe only** (this change) | — | Khronos OpenMAX IL 1.1.2 | the API is open; components are vendor drivers | see below |

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

## Proposed order

1. **E-AC-3 7.1 dependent substreams**: extends an existing crate, the spec is already used.
2. **DTS core decoder**: the most common cinema / Blu-ray audio CFA4 cannot open at all.
3. **JPEG 2000 decoder**, then the encoder and **DCP** packaging: the cinema delivery path.
4. **Dolby Atmos (E-AC-3 JOC)** and **DTS-HD MA**: after their base codecs.
5. **TrueHD**: only if a public specification becomes available, or by the owner's decision to
   leave it to the GStreamer backend on machines that have a suitable plugin installed.

Each codec needs its specification in hand before work starts: the clean-room rule forbids
writing a decoder from memory or from another implementation's source. The spec edition used is
recorded in the crate README.
