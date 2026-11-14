# Codec parity

> **Nem cél / Non-goal (2026-10-30):** a Windows-, macOS- és Android-paritás nem cél. A CFA4 kizárólag Ubuntu/Kubuntu 26.04 AMD64-re készül; a célalkalmazás más platformokon elérhető funkciói csak a Linuxra átültethető részükben számítanak. Az Apple-formátumok (ProRes, Apple Log, QuickTime/MOV) támogatása formátum, nem platform, ezért marad. / Windows, macOS and Android parity is not a goal; CFA4 targets Ubuntu 26.04 AMD64 only. Apple formats stay supported.

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-11-11 · **Change:** minor (Dolby Atmos in E-AC-3 and Auro-3D rows; the only status for cinema codecs) · **Target:** Adobe Premiere Pro 2026 (26.5.2) and Media Encoder 2026

Video and audio codecs, decode and encode, with fidelity and how each is tested. Containers,
project and interchange files are in [file-format-parity.md](file-format-parity.md); hardware
paths per platform in [hardware-parity.md](hardware-parity.md). Our codecs are pure Rust, written
from public specifications or ported from licence-compatible projects (the catalog's Origin column,
[AGENTS.md](../AGENTS.md) §2); ffmpeg is a test oracle only.

Premiere's list comes from Adobe's published supported-formats pages and the names of the
installed bundle's `MediaIO/systempresets` and `Frameworks` entries (listings only, never their
contents). Percent is ready-for-real-work for that codec: depth, profiles, speed. The plan for
Dolby, DTS, JPEG 2000 / DCP and OpenMAX specifications, licences and patents are in
[cinema-codecs.md](cinema-codecs.md); their status is tracked only here and in
[file-format-parity.md](file-format-parity.md).

## Capability catalog (generated)

What CFA4 itself implements, one row per codec: the crates, where the code comes from (`own`:
written from the public specification; `ported`: taken over from a licence-compatible project, see
the [reused code register](licensing.md#5-reused-code-register); `dependency`: a third-party Rust
crate), whether it is opened on import and whether it can be exported. The single source is the
catalog of `cfa4-cli codecs` (`apps/cfa4-cli/src/codecs_cmd.rs`); this table is generated from it
(`cfa4-cli codecs --markdown`), and the CLI tests fail when the table, the crate list or the
[patent register](patent-review.md) drifts. Hardware decoders and encoders found on a machine are
in the JSON output of `cfa4-cli codecs`, not here. The rows below the catalog add Premiere's side,
fidelity and the remaining work.

<!-- BEGIN GENERATED: cfa4-cli codecs --markdown -->
| Codec | Kind | Crates | Origin | Decode (import) | Encode (export) |
|---|---|---|---|---|---|
| H.264 / AVC | video | `crates/h264`, `crates/h264enc` | own | yes: 4:2:0 / 4:2:2 / 4:4:4, 8-14 bit, MBAFF / PAFF, lossless; encoder 8-bit 4:2:0 | yes: H.264 |
| HEVC / H.265 | video | `crates/hevc` | own | yes: up to 12-bit 4:4:4; no RExt coding tools; encoder Main / Main 10 4:2:0 | yes: H.265 (HEVC) |
| AV1 | video | `crates/av1` | own | yes: Main 8/10-bit | yes: AV1 |
| AV2 | video | `crates/av2` | own | detect only: bitstream parsing only | no |
| VP8 | video | `crates/vp8` | own | yes: RFC 6386 | no |
| VP9 | video | `crates/vp9` | own | yes: profiles 0-3, 8/10/12-bit | yes: WebM |
| Apple ProRes | video | `crates/prores` | own | yes: all six profiles, alpha | yes: Apple ProRes |
| Avid DNxHD / DNxHR | video | `crates/dnx` | own | yes: all CIDs, 8/10/12-bit, interlaced, alpha | yes: Avid DNxHR |
| GoPro CineForm | video | `crates/cineform` | ported | yes: intra samples | yes: GoPro CineForm |
| APV | video | `crates/apv` | own | yes | yes: APV |
| MPEG-2 / MPEG-1 | video | `crates/mpeg2v` | own | yes: 4:2:2, interlaced | yes: MPEG-2 |
| Motion JPEG | video | — | dependency | yes | yes: QuickTime (Motion JPEG) |
| JPEG 2000 | video | `crates/j2k` | ported | yes: Part 1 and HTJ2K; DCP and IMF App 2E packages | yes: DCP (Digital Cinema Package) |
| SMPTE VC-2, Dirac | video | `crates/dirac` | ported | yes: VC-2 (ST 2042-1) and the Dirac core syntax | no |
| FFV1 | video | `crates/ffv1` | own | yes: versions 0, 1, 3, 8-16 bit | no |
| Hap | video | `crates/hap` | own | yes | no |
| Huffyuv | video | `crates/huffyuv` | own | yes | no |
| Canopus HQ / HQA / HQX | video | `crates/canopus` | ported | yes | no |
| Ut Video | video | `crates/utvideo` | own | yes | no |
| DV / DVCPRO | video | `crates/dv` | ported | yes: DV, DVCAM, DVCPRO 25 / 50, DVCPRO HD | no |
| Animated GIF | video | — | dependency | yes | yes: Animated GIF |
| CinemaDNG / DNG | video | `crates/dng` | own | yes: uncompressed and lossless JPEG raw | no |
| LCEVC | video | `crates/lcevc` | own | detect only: enhancement layer detected, not decoded | no |
| MPEG-4 Part 2 | video | — | none | no | no |
| WMV / VC-1 | video | — | none | no | no |
| Camera RAW (R3D, ARRIRAW, BRAW…) | video | — | none | no | no |
| JPEG XL | image | `crates/jxl` | ported | yes | no |
| HEIF / HEIC | image | `crates/heif` | own | yes: HEVC image items | no |
| PCM (WAV, AIFF, BWF) | audio | — | own | yes | yes: Waveform Audio |
| AAC | audio | `crates/aac` | ported | yes: LC, HE core, LATM, USAC | yes: AAC Audio |
| AC-3 / E-AC-3 | audio | `crates/ac3` | own | yes | yes: AC-3 audio in video exports |
| DTS, DTS-HD MA | audio | `crates/dts` | ported | yes: core and lossless up to 7.1 | no |
| Dolby TrueHD | audio | `crates/truehd` | ported | yes: up to 7.1 | no |
| MPEG-1 Layer I / II (MP1 / MP2) | audio | `crates/mpa` | own | yes | yes: MP2 audio in MPEG-2 program and transport streams |
| MP3 | audio | — | dependency | yes | no |
| Opus | audio | `crates/opus` | ported | yes | yes: Opus Audio |
| FLAC | audio | `crates/flac` | own | yes | yes: FLAC |
| ALAC, Vorbis | audio | — | dependency | yes | no |
| AMR-NB / AMR-WB | audio | `crates/amr` | ported | yes | no |
| Speex | audio | `crates/speex` | ported | yes: Ogg Speex | no |
| TTA (True Audio) | audio | `crates/tta` | ported | yes | no |
| G.729 / G.729 Annex B | audio | `crates/g729` | ported | yes | no |
| iLBC | audio | `crates/ilbc` | ported | yes | no |
| Musepack SV7 / SV8 | audio | `crates/musepack` | ported | yes | no |
| ADPCM, G.711, G.722 | audio | `crates/adpcm` | own | yes | no |
| IAB (SMPTE ST 2098-2) | audio | `crates/iab` | own | yes: decode and render | yes: IAB track in DCP exports |
| Auro-3D | audio | `crates/auro` | ported | yes: side channel in 24-bit PCM | no |
| WMA | audio | — | none | no | no |
<!-- END GENERATED: cfa4-cli codecs --markdown -->

## Video

| Codec | Premiere | Our decode | Our encode | Fidelity and tests | % | Est. |
|---|---|---|---|---|---|---|
| HEVC / H.265 | decode Main, Main 10, 4:2:2 10 (XAVC HS); encode 8/10-bit, HDR | own decoder Main / Main 10 / Main 12 / Main Still Picture, monochrome 4:0:0, 4:2:2 and 4:4:4 up to 12-bit, bit-exact on 60 fixtures, ~225 fps 1080p; RExt coding tools not yet (#167) | own encoder Main / Main 10 4:2:0 (I / P, CRF / CBR / VBR, HDR10 SEI, parallel slices; no B pictures, SAO, 2-pass, 4:2:2 yet, #171–#180), 1080p ~2.3 fps on 4 cores; hardware first where present (NVENC 8-bit and Main 10 HDR) | own encoder: round trips through our decoder, bit-exact agreement of our decoder and ffmpeg on its output, ffprobe HDR signalling; NVENC round trip 48.9 dB (8-bit), 71.1 dB (10-bit) | 70% | 25–50 h |
| H.264 / AVC | decode all common profiles incl. High 10, 4:2:2 (XAVC S-I, AVC-Intra); encode 8/10-bit | own decoder, **4:2:0 / 4:2:2 / 4:4:4, 8 to 14 bits** (Baseline / Main / High / High 10 / High 4:2:2 / High 4:4:4 Predictive, CABAC / CAVLC; bit-exact with ffmpeg, incl. 16 4:2:2 / 4:4:4 fixtures); no interlaced field / MBAFF (#162), lossless (#163); monochrome 4:0:0 decodes (#279) | own encoder (High / Main / Baseline, B-frames, CBR / VBR / 2-pass); 8-bit 4:2:0 only; hardware NVENC | decode bit-exact on 37+ conformance streams, 500–600 fps 1080p; encode ffprobe-checked, byte-identical across machines | 65% | 20–35 h |
| ProRes 422 family / 4444 / XQ | decode + encode all | own decode + encode, all six profiles, alpha | yes (export UI exposes 422 flavours; 4444 / XQ request #342) | ±1 LSB, 318 fps | 85% | 3–6 h |
| ProRes RAW | decode | none (#345) | — | — | 0% | 30–50 h (spec availability uncertain) |
| DNxHD / DNxHR | decode + encode, MXF / MOV | own decode (all SMPTE ST 2019-1 CIDs, 8/10/12-bit, 4:2:2 / 4:4:4, interlaced, alpha) and encode LB / SQ / HQ / HQX / 444 | yes, MOV + MXF | within IDCT precision of ffmpeg on 22 fixtures | 90% | 2–4 h |
| GoPro CineForm (CFHD) | decode + encode (GoPro CineForm in MOV / AVI) | own decode (`cfa4-cineform`): intra samples, YUV 4:2:2 10-bit, RGB 4:4:4 / RGBA 4:4:4:4 12-bit, all band codings, peak tables; MOV / AVI / MKV import. No temporal (GOP) samples (#154), Bayer, YUVA, 3D (#155) | own encode: YUV 4:2:2 10-bit, RGB / RGBA 12-bit, Film Scan 3 … Low, MOV export with three presets (width a multiple of 16) | decode bit-exact vs ffmpeg `cfhd` on 7 fixtures; ffmpeg decodes our samples exactly as we do (6 fixtures, every quality); fuzzed damaged streams; ~33 ms decode, ~45 ms encode per 1080p frame | 75% | 10–20 h (GOP, Bayer, SDK samples) |
| VP9 | not a Premiere import format on all versions | own decoder profiles 0–3, 8/10/12-bit, bit-exact on 50+ fixtures, frame threading | own encoder, profile 0 (8-bit 4:2:0), key + inter frames, constant quantizer or bit rate (#89); WebM export (#149, about 1 fps at 1080p); about twice libvpx's size at equal PSNR (#151) | conformance; ffmpeg decodes our streams sample-exactly like we do | ahead | — |
| AV1 | decode (hardware on Windows); export through Media Encoder plug-ins / recent versions | own decoder Main 8/10-bit, all tools, film grain, bit-exact vs libdav1d on 41 vectors | own software encoder Main 8/10-bit (HDR), CRF / CBR / VBR, ~10% smaller than x265 medium (P-frames) | decode ~4 fps 1080p single-thread; encode ~0.5 fps 1080p per core | 75% | 20–40 h (encoder tools, references, speed) |
| AV2 (AOM v1.0.0) | — (not in Premiere) | **AV2-LAB only**: recognised (Annex B `.obu`, IVF `AV02`, MP4 `av02`) and reported as unsupported by default; with the `av2-external` feature and `CFA4_AV2_DECODER`, Annex B decodes through the out-of-process dav2d helper ([ADR 0010](adr/ADR-0010-av2-external-decoder.md)); router registration after G1 ([AV2 plan](plans/AV2-CODEC-PLAN-2026-10-29.md) X/7) | — | identification tests; external decoder vs the AVM v1.0.0 oracle: 10/13 corpus vectors bit-exact, 9/10 `Main_420_10_IP0` vectors (RAS frames fail to parse in dav2d `18641bb`, a G1 blocker; 4:4:4 and 4:2:2 8-bit inter differ, outside that scope) | 0% (not enabled by default) | see the AV2 plan |
| APV (RFC 9924) | — (Samsung's pro codec; not in Premiere) | own decode | own encode | spec tests | ahead | — |
| MPEG-2 / MPEG-1 | decode incl. 4:2:2 (IMX, XDCAM); encode DVD / Blu-ray / MXF | own decoder (4:2:2, interlaced, field pictures, IEEE 1180 IDCT); also Matrox MPEG-2 I-frame in AVI (`MMES`, `M701`–`M705`, backlog CB-7.1) | own encoder (`cfa4-mpeg2v`: TM5 rate control, VBV; export format MPEG-2) | conformance fixtures; Matrox AVI within the IDCT tolerance of ffmpeg's decode, also read back to front | 60% | 20–35 h (encoder) |
| Motion JPEG | decode + encode | yes | yes (MOV) | — | 90% | — |
| JPEG 2000 (MXF, DCP) | decode + encode (Kakadu) | own Part 1 decoder (`cfa4-j2k`): codestreams and JP2, tiles, all progressions, layers, 5/3 and 9/7, RCT / ICT, DCI 2K / 4K X'Y'Z'; bypass style, POC, PPM / PPT, Maxshift ROI (#79); HTJ2K (Part 15) decode, HT block decoder ported from OpenJPH (#80); MXF import (OP1a / OP-Atom, DCP picture tracks; Y'CbCr, R'G'B', gray and X'Y'Z' → P3-D65) since #77, DCP composition (CPL) import #101, interlaced / stereoscopic #102; ~150 ms per 2K frame on 4 cores (~360 ms single-threaded, #78) | own Part 1 encoder (#81): lossless 5/3, rate-controlled 9/7, DCI 2K / 4K profiles (CPRL, tile-part per component, POC, TLM, DCI byte limits), ~0.37 s per 2K and ~0.8 s per 4K frame on 4 cores (grainy synthetic frame; code-blocks stop below the predicted rate-control threshold, #116); HTJ2K encode (lossless and fixed-step lossy, HT block encoder ported from OpenJPH, #117; byte limits by whole bit-planes per code-block, CFA4-PROCODEC-002 WP4, within ~1.7 dB of the Part 1 coder at the same size); wired into SMPTE DCP export (#115) and IMF App 2E package export (10-bit R'G'B' 4:4:4, the 2K / 4K / 8K IMF profiles, #1011), **not yet into the OP1a / OP-Atom MXF export** | 5/3 bit-exact to the input, 9/7 and DCI 2K within ±1 of ffmpeg on 17 fixtures; HTJ2K: 10 Grok streams bit-exact / ±1 of OpenJPEG, refinement passes on a local sample; damaged-stream fuzzing; encoder output read by OpenJPEG and ffmpeg (lossless exact, lossy / DCI ±1), HTJ2K output by OpenJPEG and OpenJPH | 45% | 25–45 h (DCP composition #101, tier-1 speed #401, layers #117) |
| JPEG XS | decode (`libjpegxs`) | none | none | — | 0% | 20–40 h |
| DV / DVCPRO / DV100 | decode + encode | own decoder `cfa4-dv` (DV-1): DV25 / DVCAM 525 4:1:1 and 625 4:2:0, DVCPRO 25 4:1:1, 8-8 and 2-4-8 DCT, 4:3 / 16:9; tables ported from libdv (LGPL-2.1-or-later); ~10 ms per frame single-threaded; imported from raw `.dv` / `.dif`, AVI (type 2), MOV / MP4 (`dvc `, `dvcp`, `dvpp`, …) and MXF, 4:1:1 as 4:2:2, aspect and BT.601 colour from the frame (DV-2); DVCPRO 50 (4:2:2) and DVCPRO HD 1080i (1280 / 1440 wide) and 720p (960 wide), BT.709, ported from FFmpeg (`pro.rs`, owner-approved exception, DV-3); DV audio (16-bit, 12-bit expanded, 2 / 4 channels, 48 / 44.1 / 32 kHz) from raw `.dv` and type-1 AVI (DV-4); type-1 AVI video; **not yet** encoder | none | within ±3 of ffmpeg (PSNR ≥ 57 dB) on 12 fixtures incl. 2-4-8, bit spilling, DVCPRO 50 and HD 1080i / 720p; 14 container fixtures (raw, AVI, MOV, MXF) within ±3; audio bit-exact against ffmpeg (4 fixtures), type-1 AVI audio equal to the raw file's; damaged-frame fuzzing | 60% | 4–8 h (encoder) |
| LCEVC (MPEG-5 Part 2) | no | **detection only** (`cfa4-lcevc`, LCEVC-1): T.35 SEI in H.264 / HEVC (MP4, MOV, Matroska, MPEG-TS / PS), `lvc1` enhancement tracks, TS stream type 0x36; the base layer plays and the codec label says the enhancement is not decoded; decoding waits on the patent question (LCEVC-2 / LCEVC-3) | none | ffmpeg-encoded H.264 / HEVC with inserted LCEVC SEI in three containers; unit tests and damaged-input fuzzing | 10% | 40–75 h (decoder, after the patent decision) |
| MPEG-4 Part 2 | decode | none | none | — | 0% | 10–20 h |
| WMV / VC-1 | decode, encode on Windows | none | none | — | 0% | 20–35 h |
| CinemaDNG / DNG | decode | own decoder (#1065): uncompressed and lossless JPEG raw, Bayer and LinearRaw, DNG colour model to linear Rec. 709 floats; folders of `.dng` frames as one clip with the CinemaDNG frame rate and time code; **no** lossy / JPEG XL DNG, MXF-wrapped CinemaDNG, opcode lists or Raw settings UI | — | raw samples bit-exact (and identical to LibRaw's on our synthetic files), processing against hand-computed values | 60% | 10–20 h |
| Camera RAW: R3D (incl. R3D NE), ARRIRAW, Sony RAW / X-OCN, Canon RAW, BRAW (plug-in) | decode (vendor SDKs) | none (#383) | — | — | 0% | 50–90 h for the ones with public specs; vendor-SDK-only formats need an owner decision |
| GIF (animated) | encode | decode as a clip (#402) | encode | — | 80% | — |
| WebP | none | stills: lossy (VP8, with or without alpha) and lossless (VP8L) via the pure-Rust `image-webp`; animations as clips, composed on the canvas by CFA4's own code (#87) | none | stills and animations (offsets, disposal, blending, lossy / lossless / mixed) bit-exact against libwebp's `dwebp` and `anim_dump`; damaged-file fuzzing | ahead | — |
| Uncompressed / v210 / RGB | decode + encode (AVI, MOV) | partial (MOV PCM-style video not verified) | none | — | 30% | 8–15 h |
| Hap (Hap, Hap Alpha, Hap Q, Hap Q Alpha, Hap Alpha-Only) | — | own decoder ([`cfa4-hap`](../crates/hap/README.md), backlog CB-3.2): DXT1 / DXT5 / scaled YCoCg / RGTC1, Snappy, chunks; in QuickTime / MP4 and AVI, to RGBA. **No** Hap R (BC7), Hap HDR (BC6H), Hap in Matroska | none | bit-exact vs ffmpeg on 7 ffmpeg-written files and 5 with random or rewritten textures | 85% | — |
| SMPTE VC-2 (ST 2042-1) | not a Premiere import format | own decoder ([`cfa4-dirac`](../crates/dirac/README.md), backlog CB-7.2): low-delay and high-quality pictures, all seven wavelet filters, symmetric and asymmetric (horizontal-only) transforms, default and custom quantisation matrices, 4:4:4 / 4:2:2 / 4:2:0 at 8–16 bits, frame and field coding (fields woven into frames); in QuickTime / MP4 and AVI (`drac`) and Matroska (`V_DIRAC`). Picture fragments (VC-2 v3) and Dirac core syntax (CB-8.1) are not decoded yet | none | bit-exact vs ffmpeg on 12 encoder fixtures, 19 streams with random coefficients (low delay, every wavelet, custom matrices, prefix bytes) and field-coded pictures; MOV / Matroska / AVI files frame by frame, also backwards | ahead | — |
| FFV1 (RFC 9043) | not a Premiere import format | own decoder ([`cfa4-ffv1`](../crates/ffv1/README.md), backlog CB-1.1): versions 0, 1, 3, Golomb Rice and range coding (default and custom state tables), YCbCr 4:2:0 / 4:2:2 / 4:4:4 / 4:1:0 (subsamplings without a frame type are shown upsampled to 4:4:4), gray, alpha, RGB (RCT) with alpha, 8–16 bits, slices, CRCs, non-key frames; in AVI and Matroska. RGB above 8 bits is shown rounded to 8 bits | none | bit-exact vs ffmpeg on 26 codec fixtures and 4 AVI / Matroska files (all frames, also seeking backwards) | ahead | — |
| Huffyuv | — | own decoder ([`cfa4-huffyuv`](../crates/huffyuv/README.md), backlog CB-1.2): YUY2, RGB24, RGBA, left / gradient / median prediction, interlaced; in AVI and Matroska. **No** 1.x files with the built-in classic tables, no FFmpeg `FFVH` variant | none | bit-exact vs ffmpeg on 12 codec fixtures and 4 AVI / Matroska files | 85% | — |
| Ut Video | — | own decoder ([`cfa4-utvideo`](../crates/utvideo/README.md), backlog CB-2.1): 8-bit `ULY0` / `ULH0` / `ULY2` / `ULH2` / `ULY4` / `ULH4` / `ULRG` / `ULRA`, no / left / median prediction, slices; in AVI and Matroska. **No** interlaced, 10-bit `UQ..` or `UM..` streams | none | bit-exact vs ffmpeg on 14 codec fixtures and 4 AVI / Matroska files | 80% | — |
| Canopus HQ, HQA, HQX | — | own decoder ([`cfa4-canopus`](../crates/canopus/README.md), backlog CB-8.2, ported from FFmpeg): HQ (every profile) and HQA (with alpha) 8-bit 4:2:2, HQX 4:2:2 / 4:4:4 with or without alpha at 12 bits, progressive and interlaced; in AVI (`CUVC`, `CHQX`) and Matroska | none | bit-exact vs ffmpeg on 22 synthetic frames (random codes of every table, every HQX format and DC precision) and on ffmpeg's 5 FATE samples (local) | parity | — |

## Audio

| Codec | Premiere | Our decode | Our encode | Fidelity and tests | % | Est. |
|---|---|---|---|---|---|---|
| PCM (WAV, BWF, AIFF, MOV, MXF, Blu-ray / DVD LPCM) | yes | yes, BWF timecode | WAV, AIFF, MOV, MXF | sample-exact | 95% | — |
| AAC (LC, HE, LATM) | decode + encode | own decoder (LC, HE core), LATM | own LC encoder | loudness oracle vs ffmpeg | 90% | 3–5 h |
| AC-3 | decode + encode (licensed) | own decoder (ATSC A/52) | own encoder (#86): 1.0–5.1, 32–640 kb/s, rematrixing, no coupling / block switching (#120); **not yet in export** (#119) | conformance; encoder: ffmpeg decodes with CRC checking, SNR equal to or above ffmpeg's encoder | 85% | 5–10 h (export wiring) |
| E-AC-3 (Dolby Digital Plus) | decode + encode | own decoder (ATSC A/52 Annex E, #647): independent substream, 1–6 blocks, spectral extension, 7.1 and other layouts from dependent substreams with custom channel maps; **no** adaptive hybrid transform or enhanced coupling | own encoder (#86): 1.0–5.1, 32–1024 kb/s, six-block frames; no 7.1 / spectral extension (#120); **not yet in export** (#119) | SNR vs ffmpeg 65–91 dB (raw, MP4, MOV, Matroska, TS); 7.1 72–86 dB | 85% | 5–10 h |
| DTS, DTS-HD MA / HRA, DTS:X | decode | `cfa4-dts`, FFmpeg's decoder via the harletty-bridge `dca` port, LGPL-2.1-or-later (#83): core up to 5.1, Master Audio lossless up to 7.1 / 192 kHz, High Resolution core + XXCH, the DTS:X bed; in Matroska, MP4, MPEG-TS / Blu-ray; **no** DTS:X heights on their own speakers (#130), Express / 14-bit / DTS-ES XCh / raw `.dts` (#132) | none | core bit-exact vs ffmpeg (raw, Matroska, MP4, TS, random access); Master Audio unverified here (#131) | 70% | — |
| Dolby TrueHD, MLP | decode | `cfa4-truehd`, ported from the Apache-2.0 `truehd` crate (G10): 2-, 6- and 8-channel presentations (7.1 / 5.1 bed of Atmos streams), 44.1–192 kHz, raw `.thd` / `.mlp`, Matroska, MP4, Blu-ray MPEG-TS / m2ts (the interleaved AC-3 core left out, #85); Atmos objects rendered to 2.0 / 5.1 / 7.1 / 7.1.4 via `Decoder::with_objects` (#84; synthetic tests only, real streams #124; not yet wired into the app, #123); **no** E-AC-3 JOC (#126) | none | bit-exact vs ffmpeg and the encoder input (mono to 5.1, 48 / 96 kHz; 5.1 in BDAV with an AC-3 core), also after a seek; 7.1 untested (no free encoder, #111) | 80% | — |
| Dolby Atmos in E-AC-3 (JOC) | not checked | none: the 5.1 / 7.1 bed plays (#126); Atmos in TrueHD is under Dolby TrueHD | none | — | 0% | 40–60 h |
| Auro-3D (Auro-Codec in 24-bit PCM) | not checked | `cfa4-auro` (#299), ported from the Apache-2.0 `auro` crate: detection and height decoding (8.0 … 13.1) from a bit-exact carrier (DTS-HD MA, FLAC, WAV, PCM in Matroska / MP4 / MXF); **no** Auro-CX (#322) | none | ported unit tests, synthetic carriers, fuzzing; real DTS-HD MA, FLAC and PCM carriers with `CFA4_AURO_DIR` | ahead | — |
| MP3 / MP2 | decode + MP3 encode | MP3: symphonia (MPL-2.0); MP1 / MP2: own decoder (ISO/IEC 11172-3 / 13818-3, #809) and MP2 encoder (mono / stereo / dual channel, every sampling frequency and allowed bit rate; #267, joint stereo #878; MP2 audio in the MPEG-2 program / transport stream export), also the bit rate / mode combinations ffmpeg writes by default (mono 384 kbit/s); in raw, MP4, MOV (#800), Matroska, TS, PS | **none** | MP1 / MP2: SNR vs ffmpeg 73–84 dB; our MP2 streams decode in ffmpeg with CRCs checked; MP2 in MP4 / MOV within 1e-4 of ffmpeg, also after a seek (#800) | 65% | 10–20 h (MP3 encoder) |
| IAB (SMPTE ST 2098-2, DCP / IMF immersive audio) | — | own decoder (`cfa4-iab`, #82): PCM and DLC assets at 48 / 96 kHz, every frame rate incl. the 1001 cadence, Beds, conditional Beds, BedRemap, Objects; rendered to 2.0 / 5.1 / 7.1 / 7.1.4 (MXF playback: 7.1); in MXF frame- or clip-wrapped | own writer (PCM, DLC) | DLC lossless round trips; no third-party IAB stream tested yet | 60% | 10–20 h (real streams #107, layouts and decorrelation #108, IMF export #109) |
| Opus | — (not a Premiere import format) | own decoder, RFC 8251 range-exact | own CELT encoder (#99): fullband 20 ms, CBR, mono / stereo (intensity + dual stereo), Opus Audio (Ogg) and WebM export; **no** VBR (#207), psychoacoustic tuning (#208), SILK / hybrid (#209), other frame sizes or rates (#210), surround (#211), Opus in MP4 / Matroska outside WebM (#212) | conformance vectors; encoder: libopus decodes every stream (78–82 dB agreement with our decoder, matching final range), source SNR within 1–1.6 dB of libopus CBR at the same rate | ahead | 30–60 h (encoder follow-ups) |
| FLAC, ALAC, Vorbis | partial | decode (symphonia); FLAC in MP4 silent (#603) | none | — | 70% | 3–6 h |
| WMA | decode on Windows | none | none | — | 0% | 10–20 h |
| Speex | not checked | own decoder (`cfa4-speex`, CB-6.1): narrowband, wideband and ultra-wideband, every submode, VBR / ABR / DTX, several frames per packet, in-band stereo, in Ogg (`.spx`) | none | bit-exact vs the reference `speexdec` (31 files: every narrowband quality, wideband and ultra-wideband qualities, VBR, DTX, stereo), through the media source also after a seek | — | — |
| G.711 μ-law / A-law, G.722 | not checked | own decoders (`cfa4-adpcm`, CB-4.1): G.711 in WAV, AVI, MOV (`ulaw`, `alaw`), Matroska (`A_MS/ACM`); G.722 at 64 kbit/s (ITU-T G.722 11/88) in WAV, AVI, Matroska | none | bit-exact vs ffmpeg in every container, also after a seek; G.722 also on random codewords | 90% | — |
| IMA ADPCM, Microsoft ADPCM | not checked | own decoders (`cfa4-adpcm`, CB-4.2): IMA in WAV / AVI / Matroska blocks and QuickTime `ima4`, Microsoft ADPCM with custom coefficient sets; WAV read in place block by block | none | bit-exact vs ffmpeg in every container, also after a seek, and on random blocks | 90% | — |
| TTA (True Audio) | not checked | own decoder ([`cfa4-tta`](../crates/tta/README.md), backlog CB-9.1, ported from libtta): 8 / 16 / 24 bits, 1–8 channels, password-protected streams; `.tta` and Matroska (`A_TTA1`) | none | bit-exact vs ffmpeg on 10 encoder fixtures; `.tta` and Matroska files sample for sample, also read from the middle | parity | — |
| G.729, G.729 Annex B | not checked | own decoder ([`cfa4-g729`](../crates/g729/README.md), backlog CB-11.2, ported from bcg729 1.1.2): 8 kbit/s speech, Annex B SID frames and comfort noise, lost-frame concealment; raw `.g729` streams and ITU-T `.bit` bitstreams | none | bit-exact vs bcg729 on its decoder test patterns (the ITU-T G.729 and Annex B vectors, local); within 15 dB SNR of ffmpeg's separate decoder on synthetic streams; read through the media source also from the middle | — | — |
| iLBC | not checked | own decoder ([`cfa4-ilbc`](../crates/ilbc/README.md), backlog CB-12.1, ported from WebRTC libilbc 3.0.4 without the enhancer, as ffmpeg decodes): 20 ms and 30 ms frames, lost-frame concealment; RFC 3952 `.lbc` files | none | bit-exact vs ffmpeg's `ilbc` decoder on synthetic 20 ms and 30 ms streams (from ffmpeg's initial state; ffmpeg zeroes the LSF history where RFC 3951 starts from the mean); read through the media source also from the middle | — | — |
| AMR-NB, AMR-WB | not checked | own decoders (`cfa4-amr`, CB-6.2), ported from opencore-amr: every mode, DTX comfort noise, lost frames; `.amr` / `.awb`, 3GP / MP4 / MOV (`samr`, `sawb`), Matroska | none | bit-exact vs opencore-amr in every container, also after a seek (decoder checkpoints) | 90% | — |
| Musepack SV7, SV8 | not checked | own decoder (`cfa4-musepack`, CB-10.2), ported from libmpcdec r475 (float build): SV7 and SV8 streams, mid/side, noise substitution, gapless length, ID3v2-tagged files (`.mpc`) | none | exactly libmpcdec's float samples (reference r475 built from source in CI) on SV8 streams from the reference encoder (qualities 0–10, 32–48 kHz, mono and stereo) and on the 2 FATE samples (SV7 and SV8, local); within 1 LSB of ffmpeg (fixed-point) at 16 bits on the FATE samples; through the media source also from the middle; 2000 damaged files without a panic | — | — |

## Summary

- **Ahead of Premiere:** VP9, Opus, APV decode / encode; bit-exact software decoders everywhere,
  including Linux and FreeBSD, where Premiere does not run.
- **Behind, by user impact:** 10-bit / 4:2:2 H.264 and HEVC in software ([G3](gaps.md#g3-10-bit-and-422-camera-media-needs-hardware)),
  E-AC-3 decode tools, camera RAW, AV1 encoding speed, HEVC B pictures and 4:2:2 encoding, MPEG-2 encoding for broadcast,
  JPEG 2000 MXF export (the JPEG 2000 encoder exists; DCP export is done, #115), DV, WMV.
- **Low-priority backlog** (the owner's 2026-10-27 list minus what is done or planned; own decoders, after everything above): [CODEC-BACKLOG-2026-10-27.md](plans/CODEC-BACKLOG-2026-10-27.md).
- **Codec dimension: ~60%, 150–250 h** ([target-app-parity.md](target-app-parity.md#by-dimension)).

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-13 | minor | G.729 / Annex B decode (`cfa4-g729`, codec backlog CB-11.2) |
| 2026-11-13 | minor | iLBC decode (`cfa4-ilbc`, codec backlog CB-12.1) |
| 2026-11-12 | minor | Generated capability catalog from `cfa4-cli codecs` (CFA4-PROCODEC-002 work package 0) |
| 2026-11-12 | minor | Canopus HQ / HQA / HQX decode (`cfa4-canopus`, codec backlog CB-8.2) |
| 2026-11-12 | minor | AV2 early decoder behind a feature flag: out-of-process dav2d helper and Rust adapter, bit-exact with the AVM oracle on the `Main_420_10_IP0` corpus (AV2 plan X/6) |
| 2026-11-12 | minor | TTA decode (`cfa4-tta`, codec backlog CB-9.1) |
| 2026-11-11 | minor | AV2 identified (Annex B, IVF, MP4 `av02`) and reported as unsupported (AV2 plan X/4) |
| 2026-11-11 | minor | Matrox MPEG-2 I-frame in AVI (`MMES`, `M701`–`M705`; codec backlog CB-7.1) |
| 2026-11-11 | minor | Speex decode (`cfa4-speex`, codec backlog CB-6.1) |
| 2026-11-11 | minor | SMPTE VC-2 decode (`cfa4-dirac`, codec backlog CB-7.2) |
| 2026-11-11 | minor | AMR-NB and AMR-WB decode (`cfa4-amr`, codec backlog CB-6.2) |
| 2026-11-11 | minor | Dolby Atmos in E-AC-3 (JOC) and Auro-3D rows, moved from [cinema-codecs.md](cinema-codecs.md), which no longer keeps its own status |
| 2026-11-10 | minor | MP2 (MPEG-1/2 Layer II) encoder in `cfa4-mpa` (#267) |
| 2026-11-09 | minor | H.264 monochrome (4:0:0) decode, 8 and 10 bits (#279) |
| 2026-11-09 | minor | G.711, G.722, IMA and Microsoft ADPCM decode (`cfa4-adpcm`, codec backlog CB-4.1, CB-4.2) |
| 2026-11-08 | minor | Link to the low-priority codec backlog ([plans](plans/CODEC-BACKLOG-2026-10-27.md)) |
| 2026-10-27 | minor | Ubuntu 26.04 only: VideoToolbox H.264 / HEVC and Windows AV1 hardware decoding left the Ours column |
| 2026-10-26 | minor | JPEG 2000 in MXF import, DCP picture tracks included (#77) |
| 2026-10-26 | minor | Dolby TrueHD / MLP decode (`cfa4-truehd`) |
| 2026-10-26 | minor | JPEG 2000 encoder with the DCI 2K / 4K profiles (#81) |
| 2026-10-26 | minor | HTJ2K (JPEG 2000 Part 15) decode (#80) |
| 2026-10-26 | minor | JPEG 2000 decoder about 3× faster single-threaded, multi-threaded (#78) |
| 2026-10-26 | minor | VP9 encoder (`cfa4_vp9::encode`, #89) |
| 2026-10-26 | minor | AC-3 / E-AC-3 encoder (#86) |
| 2026-10-26 | minor | HEVC software encoder (Main / Main 10, `cfa4_hevc::encode`), H.265 export without hardware |
| 2026-10-26 | minor | HEVC 4:2:2 / 4:4:4 and 12-bit decode (Main 4:2:2 10, Main 4:4:4, Main 12) |
| 2026-10-26 | minor | HEVC monochrome (4:0:0) decode |
| 2026-10-26 | minor | JPEG 2000 Part 1 decoder (`cfa4-j2k`) |
| 2026-10-25 | minor | E-AC-3 7.1 from dependent substreams |
| 2026-10-11 | minor | CinemaDNG / DNG decode (#1065) |
| 2026-10-10 | major | Created: codec-by-codec comparison with Premiere 26.5.2 from the codec crates' READMEs, the export format registry and the installed bundle's listings |
