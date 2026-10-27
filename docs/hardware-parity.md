# Hardware parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-27 · **Change:** minor (Ubuntu 26.04 only: the macOS and Windows columns and backends removed) · **Target:** Adobe Premiere Pro 2026 (26.5.2)

Each hardware feature Premiere uses against ours on Ubuntu 26.04. Implementation detail lives in
[`crates/platform/README.md`](../crates/platform/README.md) (decode / encode backends, the one
crate allowed `unsafe`, [ADR 0001](adr/0001-platform-ffi.md)), the GPU compositor in
[architecture.md](architecture.md), and timings in [performance.md](performance.md).
Hardware paths are always optional: our software codecs are the tested fallback, and a hardware
decoder that fails mid-stream hands over to software (`HybridDecoder`).

Premiere's side is from Adobe's published GPU and hardware-acceleration documentation and
release notes (Premiere does not run on Linux).

## Hardware decode

CFA4 runs on Ubuntu 26.04 only; Premiere runs on macOS and Windows, so its column lists what it
does there.

| Codec / profile | Premiere (macOS / Windows) | **Ours** (Ubuntu 26.04: NVDEC, VA-API) |
|---|---|---|
| H.264 8-bit 4:2:0 | yes (Intel, NVIDIA, AMD, Apple silicon) | **yes** (NVDEC; VA-API Intel / AMD / Mesa) |
| H.264 10-bit / 4:2:2 | Apple silicon; Intel; NVIDIA Blackwell (MXF too) | no (declined; no software fallback, [G3](gaps.md#g3-10-bit-and-422-camera-media-needs-hardware)) |
| HEVC Main / Main 10 4:2:0 | yes | **yes** (NVDEC 8/10; VA-API Main / Main 10) |
| HEVC 4:2:2 10-bit | Apple silicon; Intel; NVIDIA Blackwell | no |
| VP9 profile 0 / 2 | — | software (VA-API VP9 not yet) |
| AV1 main 8 / 10 | Windows | software |
| ProRes | Apple silicon | software |
| Field-coded H.264 | yes | no |

Measured (`cargo xtask bench --hw off|auto`): Intel Iris Xe VA-API H.264 2160p 417 → 11.4 ms,
HEVC Main 10 2160p 212 → 18.4 ms CPU per frame. Every decoded picture is read back to the CPU
and re-uploaded to wgpu (**no zero-copy** yet), which Premiere avoids.

## Hardware encode

| Codec | Premiere (macOS / Windows) | **Ours** (Ubuntu 26.04) |
|---|---|---|
| H.264 8-bit | yes (Apple silicon, Intel QSV, NVENC, AMD) | **NVENC only** (opt-in) |
| H.264 10-bit / 4:2:2 | Apple silicon, Intel | no |
| HEVC Main 8-bit | yes (Apple silicon, QSV, NVENC, AMD) | **NVENC** (hardware-only format, offered where the GPU's HEVC encoder answers the probe) |
| HEVC Main 10 / HDR (PQ, HLG) | yes | **NVENC** where the GPU has 10-bit HEVC (HDR10 SEI, `mdcv` / `clli`) |
| ProRes | Apple silicon | software |
| AV1 | via plug-ins / newer GPUs | no |

Measured: NVENC HEVC round trip 48.9 dB luma PSNR (8-bit), 71.1 dB (10-bit) on an RTX 5060.

## GPU rendering

| Feature | Premiere (Mercury Playback Engine: Metal, CUDA, OpenCL) | Ours (wgpu: Vulkan, OpenGL ES) | % |
|---|---|---|---|
| Compositing, blend modes, transforms | GPU | **GPU** (#32), CPU compositor is the oracle | 90% |
| Accelerated video effects | ~82 of ~92 effects marked Accelerated (reference list `plan/premiere/10-effects-list.md`) | **34 effect ids** (`render::gpufx::GPU_EFFECTS`): colour, levels, blurs, sharpen, crop, transform, flips, vignette, video limiter, ASC CDL, channel mix, Lumetri basic / creative / vignette | ~40% |
| Lumetri curves, wheels, HSL secondary | GPU | CPU | 0% |
| Keys (Ultra Key), masks, distortions, transitions | GPU | CPU (masks have WGSL parity tests, not in the live path) | 10% |
| Export rendering on the GPU | yes | **yes** (#421; Export ▸ GPU Rendering, Auto): 2.4× (1080p) to 3.2× (4K) faster on effects-heavy timelines | 80% |
| Thumbnails on the GPU (26.0) | yes | CPU | 0% |
| Software adapter fallback | warns | silently renders everything on the CPU (#632) | — |
| Intel integrated graphics | supported | start-up crash on Intel UHD (#512) | — |

## Devices and I/O

| Hardware | Premiere | Ours | % |
|---|---|---|---|
| Audio interfaces | CoreAudio, ASIO, WASAPI; multichannel mapping | cpal (ALSA, PipeWire / PulseAudio through ALSA); device reports #605, #602 | 60% |
| Voice-over microphone | yes | yes (punch-in) | 80% |
| External video monitoring (Mercury Transmit, Blackmagic DeckLink, AJA) | yes | no | 0% |
| Second computer monitor full-screen playback | yes | full screen inside the app window; no separate-display output or floating windows on other screens (#466) | 40% |
| HDR display | macOS EDR, Windows HDR | no (SDR display of HDR sequences; Wayland HDR not used yet) | 0% |
| Control surfaces (EUCON, Mackie HUI / MCU) | yes | no | 0% |
| LTC timecode input | yes | no | 0% |
| Tape / deck control | legacy | no | n/a |

## Summary

**Hardware dimension: ~45%, 100–170 h.** Ahead of Premiere: it runs on Linux at all, with
hardware decode through VA-API and NVDEC. Behind, by user impact: zero-copy upload; GPU effects
beyond 34 (Lumetri curves / wheels / HSL, keys, masks, transitions); Intel and AMD encoders
(VA-API encode); VP9 / AV1 hardware decode; 10-bit / 4:2:2 decode; HDR display; external video
I/O and control surfaces (need hardware we don't own). Calibration: one decode backend ~3–5 h,
one encoder ~4–8 h ([target-app-parity.md](target-app-parity.md#calibration)).

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-10 | major | Created from ROADMAP.md's performance-and-hardware row and `crates/platform/README.md`; added Linux NVDEC (#549), GPU export and Lumetri on the GPU (#421), Premiere's per-platform hardware list, devices and I/O |
| 2026-10-27 | minor | Ubuntu 26.04 is the only platform: macOS and Windows columns, VideoToolbox / Media Foundation backends and their measurements removed; HEVC NVENC listed for Linux; 120–200 → 100–170 h |
