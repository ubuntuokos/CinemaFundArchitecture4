# File format parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-26 · **Change:** minor (JPEG XL stills) · **Target:** Adobe Premiere Pro 2026 (26.5.2) and Media Encoder 2026

Containers, stills, audio files, project and interchange files, captions and presets. Codecs inside
the containers are in [codec-parity.md](codec-parity.md). Our import list is
`cfa4_media::{VIDEO,AUDIO,STILL}_EXTENSIONS` plus the openers in `cfa4_codecs::openers()`;
our export list is `cfa4_export::Format`. Premiere's list is from Adobe's supported-formats
documentation and the exporter folders in the installed bundle's `MediaIO/systempresets` (names
only). "Tested" names the evidence.

## Project and interchange

| Format | Premiere | Read | Write | Tested | % |
|---|---|---|---|---|---|
| **Premiere project `.prproj`** | native | **no** | **no** | — | **0%** ([G2](gaps.md#g2-premieres-prproj-cannot-be-opened)) |
| Our project `.fcproj` | — | yes, schema migrations | yes, atomic, auto-save, recovery journal | migration tests | — |
| FCP7 XML (`xmeml`) | read + write | yes | yes | round-trip tests; Premiere-exported XML slow (#461) | 75% |
| FCPXML | read (via XtoCC) | yes | yes | round-trip tests | 80% |
| AAF (Edit Protocol) | read + write | yes | yes (embedded / linked / consolidated) | own CFB; **never opened in Media Composer / Pro Tools** | 55% |
| OMF 2.0 | write | no | yes | never opened in Pro Tools | 50% |
| EDL (CMX 3600) | read + write | yes | yes | tests | 85% |
| OpenTimelineIO | read + write (26.0) | yes | yes | tests | 80% |
| Avid Log Exchange (ALE) | write | no | yes | — | 70% |
| Marker CSV / text | write | — | yes (#454) | — | 80% |
| Selection as project | Premiere project | — | `.fcproj` only | — | 50% |
| Templates (`.prproj` templates) | yes | — | our templates | — | 70% |
| Motion Graphics templates `.mogrt` | read + write | **refused by design** (clean-room, AGENTS.md §1); our `.fcgt` instead | `.fcgt` | — | n/a |
| Effect presets `.prfpset` | read + write | no | our JSON presets | — | 40% |
| Keyboard shortcuts `.kys` | read + write | no | our presets incl. a Premiere layout | — | 60% |
| Export presets `.epr` | read + write | no | our presets (import / export) | — | 50% |
| LUTs `.cube`, `.3dl` | read | yes (1D / 3D / shaper) | — | tetrahedral CPU = WGSL | 95% |

## Video containers

| Container | Premiere | Read | Write | Tested | % |
|---|---|---|---|---|---|
| MP4 / M4V / 3GP | read + write | yes | yes (H.264, HEVC + AAC) | ffprobe, round trips | 90% |
| QuickTime MOV | read + write | yes | yes (H.264, HEVC, ProRes, DNxHR, APV, MJPEG, PCM) | ffprobe | 90% |
| MXF OP1a / OP-Atom | read + write (XDCAM, AVC-Intra, P2, AS-10 / AS-11, JPEG 2000, DNx) | yes (AVC, DNx, ProRes, MPEG-2, PCM / AES3, IAB immersive audio, timecode) | OP1a (DNxHR, ProRes, H.264), OP-Atom | index-table seeking tests | 70% |
| HLS (RFC 8216) / MPEG-DASH | no (Adobe Media Encoder only) | no (#235) | HLS: H.264 + AAC in CMAF fMP4 segments, media + multivariant playlists (#100); ABR ladder #230, TS segments #231, I-frame playlist #232, WebVTT / alternate audio #233; DASH #234 | ffprobe / ffmpeg (`hls` demuxer) | 40% |
| MPEG TS / PS (AVCHD `.mts`, `.m2ts`, `.ts`, `.mpg`, `.vob`, `.mod`) | read; write MPEG-2 / Blu-ray / DVD | yes | **no** | fixtures | 65% |
| Matroska / WebM | read (limited) | yes (H.264, HEVC, VP9, AV1, ProRes, MJPEG + AAC, Opus, FLAC, MP3, Vorbis, PCM); WebM black report (#432), MKV multi-audio (#601) | WebM: VP9 (own encoder) + Opus, CodecDelay / DiscardPadding (#149) | fixtures; audio sample-aligned with the encoded signal (#790) | 75% |
| **AVI** | read + write | **no** (extension listed, no demuxer, #598) | no | — | 0% |
| WMV / ASF | read; write on Windows | no | no | — | 0% |
| FLV / F4V | read + write | no | no | — | 0% |
| DCP | write (Wraptor) | no | SMPTE DCP (#115): JPEG 2000 X'Y'Z' 2K / 4K in Flat, Scope or Full, 5.1 24-bit PCM, one reel, CPL / PKL / ASSETMAP with SHA-1; immersive audio (#343): an IAB track file (ST 429-18, CPL AuxData) from the 5.1 mix as a 7.1 DS bed or re-wrapped from an IAB file, 16-channel main sound with the ST 430-12 FSK sync; no Interop, encryption, MCA labels, subtitles or 3D yet | our MXF reader and J2K decoder, PKL hashes, ffprobe / ffmpeg | 45% |
| APV raw / Y4M | — | yes | APV in MOV | — | ahead |
| Ogg (Opus / Vorbis) | — | yes | no | granule-seek tests | ahead |
| Camera folder structures (XDCAM, P2, AVCHD, Canon XF, RED) | Media Browser reads them | AVCHD files yes; spanned clips and metadata no | — | — | 30% |

## Stills and image sequences

| Format | Premiere | Read | Write (frame / sequence) | % |
|---|---|---|---|---|
| PNG, JPEG, TIFF, BMP, GIF | read + write | yes | PNG / TIFF / BMP / JPEG sequences, GIF | 90% |
| WebP | — | yes: lossy, lossless, alpha; animated WebP as a clip (#87); bit-exact against libwebp | no | ahead |
| JPEG XL | — | stills and sequences, own decoder (`cfa4-jxl`, safe port of jxl-oxide): VarDCT and Modular, lossless and lossy, grey / RGB, alpha; deeper than 8 bits, non-sRGB transfers (PQ / HLG / linear) and wide gamut kept as linear float; first frame only (#138), no ICC-profile conversion or CMYK (#139) | no | ahead |
| **PSD** (layers as sequence) | read (layered) | no | — | 0% |
| **OpenEXR** | read + write | stills and sequences: scan-line, none / RLE / ZIPS / ZIP / PXR24, half / float / uint, RGB(A) or Y, kept as linear float (#828); no PIZ, B44, DWA, tiled, multi-part or deep | sequences (half float, linear RGB / premultiplied RGBA; uncompressed, RLE, ZIPS or ZIP; #806, #874) | 70% |
| **DPX** | read + write | yes: luma / RGB / RGBA, 8–16-bit, filled 10/12-bit, either byte order (#821); packed 10/12-bit, Y'CbCr 4:2:2 / 4:4:4 with or without alpha (#841); no RLE | sequences (10-bit RGB, BT.709; #779) | 90% |
| Targa | read + write | yes: every image type, RLE, colour maps, 15–32-bit (#821) | sequences (24-bit, 32-bit with alpha; #779) | 95% |
| HEIF / HEIC | read | yes (#88): CFA4's own container reader and HEVC decoder, Main / Main 10 4:2:0, grids, `iden`, clean aperture, rotation, mirroring, `colr` / VUI colour, alpha planes (monochrome HEVC, #136); bit-exact against libheif. Non-HEVC items (AVIF, JPEG) go to the libheif tools when installed | HEIC frame export through `heif-enc` | 80% |
| **Radiance HDR** | read | yes: RGBE, flat, old and new run-length scan lines, every axis order, kept as linear float (#1407); no XYZE, EXPOSURE / PRIMARIES lines ignored | — | 70% |
| AI / EPS, ICO, PTL | read | no | — | 0% |

## Audio files

| Format | Premiere | Read | Write | % |
|---|---|---|---|---|
| WAV / BWF (timecode, iXML partial) | read + write | yes | yes | 90% |
| AIFF / AIFC | read + write | yes | yes | 95% |
| MP3 | read + write | read | **no** | 60% |
| AAC / M4A | read + write | yes | yes (in MP4 / MOV, and AAC Audio: `.m4a` or ADTS `.aac`, #814) | 95% |
| FLAC, Ogg, Opus | partial | yes | FLAC; Opus Audio (`.opus`, Ogg Opus, own CELT encoder, 48 kHz mono / stereo, #99) | ahead |
| WMA | read (Windows) | no | no | 0% |

## Captions and subtitles

| Format | Premiere | Read | Write | Tested | % |
|---|---|---|---|---|---|
| SRT, WebVTT, SCC | read + write | yes | yes | frame-exact, property-tested | 95% |
| MCC (608 + 708), EBU STL, TTML / IMSC1, DFXP | read + write | yes | yes | frame-exact | 90% |
| 608 / 708 embedded in the video stream | write (MXF, H.264 SEI) | no | **no** | — | 0% |
| Burn-in | yes | — | yes | golden tests | 95% |

## Summary

**File formats dimension: ~60%, 140–240 h** (codecs counted separately). The beta blocker is
`.prproj`. After it, by user impact: AVI and WMV import, EXR / PSD stills, AAF / OMF
validation in Avid and Pro Tools, DVD / Blu-ray / DCP / broadcast MXF delivery.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-26 | minor | JPEG XL stills and sequences (`cfa4-jxl`) |
| 2026-10-26 | minor | Native HEIF / HEIC decode incl. alpha planes (#88, #136) |
| 2026-10-10 | major | Created: format-by-format comparison with Premiere 26.5.2 from the import / export registries, crate READMEs, the installed bundle's exporter listings and open issues |
