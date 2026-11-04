# Szabadalmi vizsgálat – nyilvántartás

> **Létrehozva:** 2026-10-27 · **Szabály:** [ADR 0003](adr/ADR-0003-aswf-and-conversion-sources.md) 13. pont ·
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
| H.264 / AVC | dekódolás | `crates/h264`; `crates/platform` (VA-API, NVDEC); FFmpeg-háttér | alap | függőben | | |
| H.264 / AVC | kódolás | `crates/h264enc`; `crates/platform` (NVENC); x264 (FFmpeg-háttér) | alap; x264: kiegészítő | függőben | | |
| H.265 / HEVC | dekódolás | `crates/hevc`; `crates/platform` (VA-API, NVDEC); FFmpeg-háttér | alap | függőben | | |
| H.265 / HEVC | kódolás | `crates/platform` (NVENC); x265 (FFmpeg-háttér) | alap; x265: kiegészítő | függőben | | |
| MPEG-1 / MPEG-2 videó (H.262) | dekódolás | `crates/mpeg2v`; FFmpeg-háttér | alap | függőben | | |
| MPEG-4 Part 2 (Visual) | dekódolás, kódolás | FFmpeg-háttér | alap | függőben | | |
| VC-1 / WMV | dekódolás | FFmpeg-háttér; GStreamer | alap | függőben | | |
| DV / DVCPRO | dekódolás | FFmpeg-háttér | alap | függőben | | |
| AV1 | dekódolás | `crates/av1`; FFmpeg-háttér (dav1d) | alap | függőben | | |
| AV1 | kódolás | tervezett (rav1e) | – | függőben | | |
| VP8 | dekódolás | `crates/vp8`; FFmpeg-háttér (libvpx) | alap | függőben | | |
| VP9 | dekódolás | `crates/vp9`; FFmpeg-háttér (libvpx) | alap | függőben | | |
| VP8 / VP9 | kódolás | FFmpeg-háttér (libvpx) | alap | függőben | | |
| APV | dekódolás, kódolás | `crates/apv` | alap | függőben | | |
| Apple ProRes | dekódolás, kódolás | `crates/prores` | alap | függőben | | |
| DNxHD / DNxHR (VC-3) | dekódolás, kódolás | `crates/dnx` | alap | függőben | | |
| JPEG 2000 | dekódolás | `crates/j2k` | alap | függőben | | |
| DNG / CinemaDNG (lossless JPEG) | dekódolás | `crates/dng` | alap | függőben | | |

## Hang

| Formátum | Irány | Komponens | Csomag | Eredmény | Dátum | Vizsgáló / forrás |
|---|---|---|---|---|---|---|
| AAC | dekódolás, kódolás | `crates/aac`; FFmpeg-háttér | alap | függőben | | |
| AC-3 / E-AC-3 (Dolby Digital, Dolby Digital Plus) | dekódolás | `crates/ac3`; FFmpeg-háttér | alap | függőben | | |
| MPEG-1/2 Audio Layer I / II | dekódolás | `crates/mpa`; FFmpeg-háttér | alap | függőben | | |
| MP3 (MPEG Audio Layer III) | dekódolás | FFmpeg-háttér | alap | függőben | | |
| MP3 (MPEG Audio Layer III) | kódolás | FFmpeg-háttér (LAME) | alap | függőben | | |
| Opus | dekódolás | `crates/opus`; FFmpeg-háttér | alap | függőben | | |
| FLAC | kódolás | `crates/flac` | alap | függőben | | |
| Vorbis | dekódolás | FFmpeg-háttér; GStreamer | alap | függőben | | |
| WMA | dekódolás | FFmpeg-háttér; GStreamer | alap | függőben | | |

## Konténerek

| Formátum | Irány | Komponens | Csomag | Eredmény | Dátum | Vizsgáló / forrás |
|---|---|---|---|---|---|---|
| MP4 / MOV (ISO BMFF, QuickTime) | olvasás, írás | `crates/isobmff` | alap | függőben | | |
| Matroska / WebM | olvasás, írás | `crates/matroska` | alap | függőben | | |
| MXF | olvasás, írás | `crates/mxf` | alap | függőben | | |
| MPEG-2 TS / PS (AVCHD, VOB) | olvasás | `crates/mpegts` | alap | függőben | | |
| AVI / OpenDML | olvasás | `crates/avi`; FFmpeg-háttér | alap | függőben | | |
| Ogg | olvasás | `crates/ogg` | alap | függőben | | |
| ASF | olvasás | FFmpeg-háttér; GStreamer | alap | függőben | | |
| AAF (Compound File Binary) | olvasás, írás | `crates/cfb` | alap | függőben | | |
| Konténer- és kodekközpont | olvasás | `crates/codecs` (a fenti sorok formátumai) | alap | függőben | | |

## Nyitott PR-ok, amelyek összevonásuk előtt felveszik a sorukat

TrueHD / MLP (#75, #112), DTS és DTS-HD (#134), Dolby Atmos objektumok (#128), AC-3 / E-AC-3
kódoló (#121), HEVC szoftveres kódoló (#181), H.264 High 10 (#165), HEVC RExt (#168), VP9-kódoló
(#152), JPEG 2000 kódoló (#118), HTJ2K (#114), GoPro CineForm (#158, #160), JPEG XL (#143),
HEIF / HEIC (#137), animált WebP (#135), IAB (#110).
