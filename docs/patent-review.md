# Szabadalmi vizsgálat – nyilvántartás

> **Létrehozva:** 2026-10-27 · **Frissítve:** 2026-11-10 · **Szabály:** [ADR 0003](adr/ADR-0003-aswf-and-conversion-sources.md) 13. pont ·
> **Kapcsolódik:** Ubuntu-modell ([`licensing.md`](licensing.md) 7. pont, ADR 0003 12. pont)

Ebben a fájlban rögzül minden szabadalmi vizsgálat eredménye. Sorai a CFA4 által kezelt
formátumok, formátumonként és irányonként (dekódolás, kódolás, konténer írása/olvasása). A
**Komponens** oszlop minden CFA4-részt felsorol, amely az adott formátumot kezeli: saját crate,
hardveres út, FFmpeg-háttér ([ADR 0005](adr/ADR-0005-media-backends.md)), GStreamer, a
`cfa4-restricted-extras` csomag.

A szabadalmi kérdés független a szerzői jogi licenctől: egy szabad szoftver is megvalósíthat
szabadalommal védett formátumot.

## Hogyan kell kitölteni

- **Eredmény**: `függőben` (még nincs vizsgálva) · `szabad` (a terjesztést szabadalom nem
  korlátozza) · `korlátozott` (szabadalom vagy licencdíj érinti) · `tisztázatlan` (vizsgálva,
  nincs egyértelmű válasz).
- Ha az Eredmény nem `függőben`, a **Dátum** (ÉÉÉÉ-HH-NN) és a **Vizsgáló / forrás** kitöltése
  kötelező: ki vizsgálta (jogász, szervezet) és mire alapozta (jogi vélemény, szabadalmi pool
  listája, lejárati adatok, hivatalos nyilatkozat). A rövid indoklás a forrás után áll.
- **Csomag**: ahol a komponens ma van – `alap` vagy `kiegészítő` (`cfa4-restricted-extras`).
  `korlátozott` eredmény után a besorolásról a tulajdonos dönt (ADR 0003 13. pont), és a döntés
  ebbe az oszlopba kerül.
- Új kodek, konténer vagy kódolási út csak a saját sorával kerülhet be; a
  `python/tests/test_patent_register.py` ellenőrzi, hogy minden kodek- és konténer-crate szerepel.

*Ez a nyilvántartás nem jogi tanács, és nem helyettesíti a vizsgálatot.*

## Videó

| Formátum | Irány | Komponens | Csomag | Eredmény | Dátum | Vizsgáló / forrás |
|---|---|---|---|---|---|---|
| H.264 / AVC (High 10-ig) | dekódolás | `crates/h264`; `crates/platform` (VA-API, NVDEC); FFmpeg-háttér | alap | függőben | | |
| H.264 / AVC | kódolás | `crates/h264enc`; `crates/platform` (NVENC); x264 (FFmpeg-háttér) | alap; x264: kiegészítő | függőben | | |
| H.265 / HEVC (Main, Main 10, RExt 4:2:2 / 4:4:4, 12 bit) | dekódolás | `crates/hevc`; `crates/platform` (VA-API, NVDEC); FFmpeg-háttér | alap | függőben | | |
| H.265 / HEVC | kódolás | `crates/hevc` (szoftveres, Main / Main 10); `crates/platform` (NVENC); x265 (FFmpeg-háttér) | alap; x265: kiegészítő | függőben | | |
| LCEVC (MPEG-5 Part 2, ISO/IEC 23094-2) | csak felismerés (H.264 / HEVC SEI, MP4 `lvc1`, TS 0x36; a javítóréteget nem dekódolja) | `crates/lcevc` | alap | függőben | | |
| MPEG-1 / MPEG-2 videó (H.262) | dekódolás | `crates/mpeg2v`; FFmpeg-háttér | alap | függőben | | |
| MPEG-4 Part 2 (Visual) | dekódolás, kódolás | FFmpeg-háttér | alap | függőben | | |
| VC-1 / WMV | dekódolás | FFmpeg-háttér; GStreamer | alap | függőben | | |
| DV / DVCPRO 25 / 50, DVCPRO HD (IEC 61834-2, SMPTE 314M, SMPTE 370M) | dekódolás | `crates/dv`; FFmpeg-háttér | alap | függőben | | |
| AV1 | dekódolás | `crates/av1`; FFmpeg-háttér (dav1d) | alap | függőben | | |
| AV1 | kódolás | tervezett (rav1e) | – | függőben | | |
| AV2 | bitfolyam-elemzés (OBU, sequence header); dekódolás csak fejlesztői kapcsolóval (AV2-LAB) | `crates/av2`; `cfa4-av2-decode` segédprogram (dav2d, külön folyamat, ADR 0010) | alap | függőben | | |
| VP8 | dekódolás | `crates/vp8`; FFmpeg-háttér (libvpx) | alap | függőben | | |
| VP8 | kódolás | FFmpeg-háttér (libvpx) | alap | függőben | | |
| VP9 | dekódolás | `crates/vp9`; FFmpeg-háttér (libvpx) | alap | függőben | | |
| VP9 | kódolás | `crates/vp9` (0. profil); FFmpeg-háttér (libvpx) | alap | függőben | | |
| APV | dekódolás, kódolás | `crates/apv` | alap | függőben | | |
| Apple ProRes | dekódolás, kódolás | `crates/prores` | alap | függőben | | |
| DNxHD / DNxHR (VC-3) | dekódolás, kódolás | `crates/dnx` | alap | függőben | | |
| JPEG 2000 (Part 1) | dekódolás, kódolás (DCI 2K / 4K profilok) | `crates/j2k` | alap | függőben | | |
| HTJ2K (JPEG 2000 Part 15) | dekódolás, kódolás | `crates/j2k` (az OpenJPH blokkdekódolójának és blokk-kódolójának portja) | alap | függőben | | |
| GoPro CineForm (CFHD) | dekódolás, kódolás | `crates/cineform` (a CineForm SDK portja) | alap | függőben | | |
| JPEG XL | dekódolás | `crates/jxl` (a jxl-oxide portja) | alap | függőben | | |
| HEIF / HEIC állókép | olvasás (a képelemek HEVC-dekódolása a `crates/hevc` sorában) | `crates/heif` | alap | függőben | | |
| Animált WebP | dekódolás (VP8 / VP8L) | `crates/media` (a VP8-dekódolás a `crates/vp8` sorában) | alap | függőben | | |
| DNG / CinemaDNG (lossless JPEG) | dekódolás | `crates/dng` | alap | függőben | | |
| FFV1 (RFC 9043) | dekódolás | `crates/ffv1` | alap | függőben | | |
| SMPTE VC-2 (ST 2042-1) | dekódolás; QuickTime / MP4, AVI és Matroska olvasása | `crates/dirac` (a BBC VC-2 referencia-pszeudokódjának portja) | alap | függőben | | |
| Huffyuv (HFYU) | dekódolás | `crates/huffyuv` | alap | függőben | | |
| Ut Video | dekódolás | `crates/utvideo` | alap | függőben | | |
| Canopus HQ, HQA, HQX | dekódolás | `crates/canopus` (az FFmpeg portja) | alap | függőben | | |
| Hap (Hap, Hap Alpha, Hap Q, Hap Q Alpha, Hap Alpha-Only) | dekódolás | `crates/hap` | alap | függőben | | |

## Hang

| Formátum | Irány | Komponens | Csomag | Eredmény | Dátum | Vizsgáló / forrás |
|---|---|---|---|---|---|---|
| AAC | dekódolás, kódolás | `crates/aac`; FFmpeg-háttér | alap | függőben | | |
| AC-3 / E-AC-3 (Dolby Digital, Dolby Digital Plus) | dekódolás, kódolás | `crates/ac3`; FFmpeg-háttér | alap | függőben | | |
| Dolby TrueHD / MLP | dekódolás | `crates/truehd` (a truehd crate portja); Blu-ray MPEG-TS (AC-3 maggal) a `crates/mpegts` sorában | alap | függőben | | |
| Dolby Atmos (TrueHD-objektumok) | dekódolás, renderelés 7.1.4-re, DCP-exportban IAB-objektumként | `crates/truehd` | alap | függőben | | |
| DTS Coherent Acoustics, DTS-HD MA / HRA, DTS:X alap | dekódolás | `crates/dts` (az FFmpeg dca-dekódolójának portja) | alap | függőben | | |
| Auro-3D (Auro-Codec) | felismerés, magassági csatornák dekódolása 24 bites PCM-hordozóból | `crates/auro` | alap | függőben | | |
| IAB – Immersive Audio Bitstream (SMPTE ST 2098-2) | dekódolás, renderelés, írás; IAB MXF-ben | `crates/iab`; `crates/mxf` | alap | függőben | | |
| MPEG-1/2 Audio Layer I / II | dekódolás | `crates/mpa`; FFmpeg-háttér | alap | függőben | | |
| MP3 (MPEG Audio Layer III) | dekódolás | FFmpeg-háttér | alap | függőben | | |
| MP3 (MPEG Audio Layer III) | kódolás | FFmpeg-háttér (LAME) | alap | függőben | | |
| Opus | dekódolás | `crates/opus`; FFmpeg-háttér | alap | függőben | | |
| FLAC | kódolás | `crates/flac` | alap | függőben | | |
| Vorbis | dekódolás | FFmpeg-háttér; GStreamer | alap | függőben | | |
| WMA | dekódolás | FFmpeg-háttér; GStreamer | alap | függőben | | |
| Speex | dekódolás; Ogg Speex (`.spx`) olvasása | `crates/speex` (a libspeex portja) | alap | függőben | | |
| SMPTE LTC időkód (ST 12-1) és MIDI Timecode (MTC) | dekódolás, kódolás (hangjel, MIDI-üzenetek) | `crates/tcsync` | alap | függőben | | |
| G.711 (µ-law, A-law), G.722, IMA ADPCM, Microsoft ADPCM | dekódolás | `crates/adpcm` | alap | függőben | | |
| G.729, G.729 Annex B | dekódolás; nyers `.g729` fájlok olvasása | `crates/g729` (a bcg729 portja) | alap | függőben | | |
| TTA (True Audio) | dekódolás; `.tta` és Matroska olvasása | `crates/tta` (a libtta portja) | alap | függőben | | |
| AMR-NB, AMR-WB | dekódolás | `crates/amr` (az opencore-amr portja) | alap | függőben | | |

## Konténerek

| Formátum | Irány | Komponens | Csomag | Eredmény | Dátum | Vizsgáló / forrás |
|---|---|---|---|---|---|---|
| MP4 / MOV (ISO BMFF, QuickTime) | olvasás, írás | `crates/isobmff` | alap | függőben | | |
| Matroska / WebM | olvasás, írás | `crates/matroska` | alap | függőben | | |
| MXF | olvasás, írás | `crates/mxf` | alap | függőben | | |
| MPEG-2 TS / PS (AVCHD, VOB) | olvasás | `crates/mpegts` | alap | függőben | | |
| SMPTE ST 2110-20 (tömörítetlen kép RTP-n, RFC 4175), RTP, SDP | csomagolás, bontás | `crates/st2110` | alap | függőben | | |
| AVI / OpenDML | olvasás | `crates/avi`; FFmpeg-háttér | alap | függőben | | |
| Ogg | olvasás | `crates/ogg` | alap | függőben | | |
| ASF | olvasás | FFmpeg-háttér; GStreamer | alap | függőben | | |
| AAF (Compound File Binary) | olvasás, írás | `crates/cfb` | alap | függőben | | |
| Konténer- és kodekközpont | olvasás | `crates/codecs` (a fenti sorok formátumai) | alap | függőben | | |

## Hátterek és exportutak

| Formátum | Irány | Komponens | Csomag | Eredmény | Dátum | Vizsgáló / forrás |
|---|---|---|---|---|---|---|
| FFmpeg-háttér: a fenti sorok „FFmpeg-háttér” bejegyzései és a csak FFmpeg-gel megnyitható formátumok | dekódolás (tartalékút a saját motor mögött, ADR 0005) | `crates/ffmpeg` (FFmpeg 9.0.2 alap-build); x264 / x265: `--restricted` build | alap; `--restricted` build: kiegészítő | függőben | | |
| Smart Cut / Smart Render: H.264, HEVC, AV1, MPEG-2 videó | bitfolyam-elemzés (GOP-index), csomagmásolás, határ-újrakódolás (`crates/h264enc`) | `crates/smart-edit` | alap | függőben | | |
