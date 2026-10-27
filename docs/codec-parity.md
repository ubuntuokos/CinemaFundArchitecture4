# Codec parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-26 · **Change:** minor (CineForm decoder and encoder) · **Target:** Adobe Premiere Pro 2026 (26.5.2) and Media Encoder 2026

Video and audio codecs, decode and encode, with fidelity and how each is tested. Containers,
project and interchange files are in [file-format-parity.md](file-format-parity.md); hardware
paths per platform in [hardware-parity.md](hardware-parity.md). All our codecs are clean-room
pure Rust from public specifications ([AGENTS.md](../AGENTS.md) §2); ffmpeg is a test oracle only.

Premiere's list comes from Adobe's published supported-formats pages and the names of the
installed bundle's `MediaIO/systempresets` and `Frameworks` entries (listings only, never their
contents). Percent is ready-for-real-work for that codec: depth, profiles, speed. The plan for
Dolby, DTS, JPEG 2000 / DCP and OpenMAX is in [cinema-codecs.md](cinema-codecs.md).

## Video

| Codec | Premiere | Our decode | Our encode | Fidelity and tests | % | Est. |
|---|---|---|---|---|---|---|
| H.264 / AVC | decode all common profiles incl. High 10, 4:2:2 (XAVC S-I, AVC-Intra); encode 8/10-bit | own decoder, **8-bit 4:2:0 only** (Baseline / Main / High, CABAC / CAVLC); no High 10 / 4:2:2 / 4:4:4, interlaced field / MBAFF, lossless | own encoder (High / Main / Baseline, B-frames, CBR / VBR / 2-pass); 8-bit 4:2:0 only; hardware VideoToolbox / NVENC | decode bit-exact on 37+ conformance streams, 500–600 fps 1080p; encode ffprobe-checked, byte-identical across machines | 65% | 20–35 h |
| HEVC / H.265 | decode Main, Main 10, 4:2:2 10 (XAVC HS); encode 8/10-bit, HDR | own decoder Main / Main 10 / Main 12 / Main Still Picture, monochrome 4:0:0, 4:2:2 and 4:4:4 up to 12-bit, bit-exact on 60 fixtures, ~225 fps 1080p; RExt coding tools not yet (#167) | own encoder Main / Main 10 4:2:0 (I / P, CRF / CBR / VBR, HDR10 SEI, parallel slices; no B pictures, SAO, 2-pass, 4:2:2 yet, #171–#180), 1080p ~2.3 fps on 4 cores; hardware first where present (VideoToolbox 8-bit; NVENC 8-bit and Main 10 HDR) | own encoder: round trips through our decoder, bit-exact agreement of our decoder and ffmpeg on its output, ffprobe HDR signalling; NVENC round trip 48.9 dB (8-bit), 71.1 dB (10-bit) | 70% | 25–50 h |
| ProRes 422 family / 4444 / XQ | decode + encode all | own decode + encode, all six profiles, alpha | yes (export UI exposes 422 flavours; 4444 / XQ request #342) | ±1 LSB, 318 fps | 85% | 3–6 h |
| ProRes RAW | decode | none (#345) | — | — | 0% | 30–50 h (spec availability uncertain) |
| DNxHD / DNxHR | decode + encode, MXF / MOV | own decode (all SMPTE ST 2019-1 CIDs, 8/10/12-bit, 4:2:2 / 4:4:4, interlaced, alpha) and encode LB / SQ / HQ / HQX / 444 | yes, MOV + MXF | within IDCT precision of ffmpeg on 22 fixtures | 90% | 2–4 h |
| GoPro CineForm (CFHD) | decode + encode (GoPro CineForm in MOV / AVI) | own decode (`cfa4-cineform`): intra samples, YUV 4:2:2 10-bit, RGB 4:4:4 / RGBA 4:4:4:4 12-bit, all band codings, peak tables; MOV / AVI / MKV import. No temporal (GOP) samples (#154), Bayer, YUVA, 3D (#155) | own encode: YUV 4:2:2 10-bit, RGB / RGBA 12-bit, Film Scan 3 … Low, MOV export with three presets (width a multiple of 16) | decode bit-exact vs ffmpeg `cfhd` on 7 fixtures; ffmpeg decodes our samples exactly as we do (6 fixtures, every quality); fuzzed damaged streams; ~33 ms decode, ~45 ms encode per 1080p frame | 75% | 10–20 h (GOP, Bayer, SDK samples) |
| AV1 | decode (hardware on Windows); export through Media Encoder plug-ins / recent versions | own decoder Main 8/10-bit, all tools, film grain, bit-exact vs libdav1d on 41 vectors; hardware on Windows | none | slow in software (~4 fps 1080p single-thread) | 55% | 30–50 h (encoder + speed) |
| VP9 | not a Premiere import format on all versions | own decoder profiles 0–3, 8/10/12-bit, bit-exact on 50+ fixtures, frame threading | own encoder, profile 0 (8-bit 4:2:0), key + inter frames, constant quantizer or bit rate (#89); WebM export (#149, about 1 fps at 1080p); about twice libvpx's size at equal PSNR (#151) | conformance; ffmpeg decodes our streams sample-exactly like we do | ahead | — |
| APV (RFC 9924) | — (Samsung's pro codec; not in Premiere) | own decode | own encode | spec tests | ahead | — |
| MPEG-2 / MPEG-1 | decode incl. 4:2:2 (IMX, XDCAM); encode DVD / Blu-ray / MXF | own decoder (4:2:2, interlaced, field pictures, IEEE 1180 IDCT) | **none** | conformance fixtures | 60% | 20–35 h (encoder) |
| Motion JPEG | decode + encode | yes | yes (MOV) | — | 90% | — |
| JPEG 2000 (MXF, DCP) | decode + encode (Kakadu) | own Part 1 decoder (`cfa4-j2k`): codestreams and JP2, tiles, all progressions, layers, 5/3 and 9/7, RCT / ICT, DCI 2K / 4K X'Y'Z'; no bypass style, POC, PPM / PPT, RGN, HTJ2K; **not yet wired into MXF / DCP import**; ~1.4 s per 2K frame single-threaded | none | 5/3 bit-exact to the input, 9/7 and DCI 2K within ±1 of ffmpeg on 17 fixtures; damaged-stream fuzzing | 35% | 30–50 h (MXF / DCP wiring, speed, encoder) |
| JPEG XS | decode (`libjpegxs`) | none | none | — | 0% | 20–40 h |
| DV / DVCPRO / DV100 | decode + encode | none | none | — | 0% | 15–25 h |
| MPEG-4 Part 2 | decode | none | none | — | 0% | 10–20 h |
| WMV / VC-1 | decode, encode on Windows | none | none | — | 0% | 20–35 h |
| CinemaDNG / DNG | decode | own decoder (#1065): uncompressed and lossless JPEG raw, Bayer and LinearRaw, DNG colour model to linear Rec. 709 floats; folders of `.dng` frames as one clip with the CinemaDNG frame rate and time code; **no** lossy / JPEG XL DNG, MXF-wrapped CinemaDNG, opcode lists or Raw settings UI | — | raw samples bit-exact (and identical to LibRaw's on our synthetic files), processing against hand-computed values | 60% | 10–20 h |
| Camera RAW: R3D (incl. R3D NE), ARRIRAW, Sony RAW / X-OCN, Canon RAW, BRAW (plug-in) | decode (vendor SDKs) | none (#383) | — | — | 0% | 50–90 h for the ones with public specs; vendor-SDK-only formats need an owner decision |
| GIF (animated) | encode | decode as a clip (#402) | encode | — | 80% | — |
| WebP | none | stills: lossy (VP8, with or without alpha) and lossless (VP8L) via the pure-Rust `image-webp`; animations as clips, composed on the canvas by CFA4's own code (#87) | none | stills and animations (offsets, disposal, blending, lossy / lossless / mixed) bit-exact against libwebp's `dwebp` and `anim_dump`; damaged-file fuzzing | ahead | — |
| Uncompressed / v210 / RGB | decode + encode (AVI, MOV) | partial (MOV PCM-style video not verified) | none | — | 30% | 8–15 h |

## Audio

| Codec | Premiere | Our decode | Our encode | Fidelity and tests | % | Est. |
|---|---|---|---|---|---|---|
| PCM (WAV, BWF, AIFF, MOV, MXF, Blu-ray / DVD LPCM) | yes | yes, BWF timecode | WAV, AIFF, MOV, MXF | sample-exact | 95% | — |
| AAC (LC, HE, LATM) | decode + encode | own decoder (LC, HE core), LATM | own LC encoder | loudness oracle vs ffmpeg | 90% | 3–5 h |
| AC-3 | decode + encode (licensed) | own decoder (ATSC A/52) | own encoder (#86): 1.0–5.1, 32–640 kb/s, rematrixing, no coupling / block switching (#120); **not yet in export** (#119) | conformance; encoder: ffmpeg decodes with CRC checking, SNR equal to or above ffmpeg's encoder | 85% | 5–10 h (export wiring) |
| E-AC-3 (Dolby Digital Plus) | decode + encode | own decoder (ATSC A/52 Annex E, #647): independent substream, 1–6 blocks, spectral extension, 7.1 and other layouts from dependent substreams with custom channel maps; **no** adaptive hybrid transform or enhanced coupling | own encoder (#86): 1.0–5.1, 32–1024 kb/s, six-block frames; no 7.1 / spectral extension (#120); **not yet in export** (#119) | SNR vs ffmpeg 65–91 dB (raw, MP4, MOV, Matroska, TS); 7.1 72–86 dB | 85% | 5–10 h |
| DTS, DTS-HD MA / HRA, DTS:X | decode | `cfa4-dts`, FFmpeg's decoder via the harletty-bridge `dca` port, LGPL-2.1-or-later (#83): core up to 5.1, Master Audio lossless up to 7.1 / 192 kHz, High Resolution core + XXCH, the DTS:X bed; in Matroska, MP4, MPEG-TS / Blu-ray; **no** DTS:X heights on their own speakers (#130), Express / 14-bit / DTS-ES XCh / raw `.dts` (#132) | none | core bit-exact vs ffmpeg (raw, Matroska, MP4, TS, random access); Master Audio unverified here (#131) | 70% | — |
| MP3 / MP2 | decode + MP3 encode | MP3: symphonia (MPL-2.0); MP1 / MP2: own decoder (ISO/IEC 11172-3 / 13818-3, #809), also the bit rate / mode combinations ffmpeg writes by default (mono 384 kbit/s); in raw, MP4, MOV (#800), Matroska, TS, PS | **none** | MP1 / MP2: SNR vs ffmpeg 73–84 dB; MP2 in MP4 / MOV within 1e-4 of ffmpeg, also after a seek (#800) | 65% | 10–20 h (MP3 encoder) |
| Opus | — (not a Premiere import format) | own decoder, RFC 8251 range-exact | own CELT encoder (#99): fullband 20 ms, CBR, mono / stereo (intensity + dual stereo), Opus Audio (Ogg) and WebM export; **no** VBR (#207), psychoacoustic tuning (#208), SILK / hybrid (#209), other frame sizes or rates (#210), surround (#211), Opus in MP4 / Matroska outside WebM (#212) | conformance vectors; encoder: libopus decodes every stream (78–82 dB agreement with our decoder, matching final range), source SNR within 1–1.6 dB of libopus CBR at the same rate | ahead | 30–60 h (encoder follow-ups) |
| FLAC, ALAC, Vorbis | partial | decode (symphonia); FLAC in MP4 silent (#603) | none | — | 70% | 3–6 h |
| WMA | decode on Windows | none | none | — | 0% | 10–20 h |

## Summary

- **Ahead of Premiere:** VP9, Opus, APV decode / encode; bit-exact software decoders everywhere,
  including Linux and FreeBSD, where Premiere does not run.
- **Behind, by user impact:** 10-bit / 4:2:2 H.264 and HEVC in software ([G3](gaps.md#g3-10-bit-and-422-camera-media-needs-hardware)),
  E-AC-3 decode tools, camera RAW, AV1 software encoding, HEVC B pictures and 4:2:2 encoding, MPEG-2 / JPEG 2000 encoding for broadcast
  and DCP, DV, WMV.
- **Codec dimension: ~60%, 150–250 h** ([target-app-parity.md](target-app-parity.md#by-dimension)).

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-26 | minor | VP9 encoder (`cfa4_vp9::encode`, #89) |
| 2026-10-26 | minor | AC-3 / E-AC-3 encoder (#86) |
| 2026-10-26 | minor | HEVC software encoder (Main / Main 10, `cfa4_hevc::encode`), H.265 export without hardware |
| 2026-10-26 | minor | HEVC 4:2:2 / 4:4:4 and 12-bit decode (Main 4:2:2 10, Main 4:4:4, Main 12) |
| 2026-10-26 | minor | HEVC monochrome (4:0:0) decode |
| 2026-10-26 | minor | JPEG 2000 Part 1 decoder (`cfa4-j2k`) |
| 2026-10-25 | minor | E-AC-3 7.1 from dependent substreams |
| 2026-10-11 | minor | CinemaDNG / DNG decode (#1065) |
| 2026-10-10 | major | Created: codec-by-codec comparison with Premiere 26.5.2 from the codec crates' READMEs, the export format registry and the installed bundle's listings |
