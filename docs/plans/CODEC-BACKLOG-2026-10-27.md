# Kodek-backlog (alacsony prioritás)

> **Utolsó frissítés:** 2026-11-14 · **Változás:** minor (az OptimFROG, a Sony SR Codec és a Bink 2 kimarad, tulajdonosi döntés) · **Hely a tervben:** a kodekfejlesztés legvégén, az [M9](../roadmap.md#milestones-history-and-remaining) és a [B3](../roadmap.md#beta-milestones) után

A tulajdonos 2026-10-27-én két kodeklistát adott át (általánosan használt, valamint ritkább és
ázsiai videó-, audio- és képkodekek). Ebből a fájlból **kimaradt minden, ami ebben a tárolóban
már meg van oldva vagy már tervben van** (lásd lent), a többi ide került. **Nem sürgős:** ezek a
tételek a kodekfejlesztés végére kerülnek, a meglévő kodekmunka (M9, B3, a
[codec-parity.md](../codec-parity.md) „Behind” sorai) után. A sávokon belül a sorrend sem kötelező.
Heti ütemterv (legalább 2 tétel hetente): [CODEC-BACKLOG-PLAN-2026-10-27.md](CODEC-BACKLOG-PLAN-2026-10-27.md).

**Saját dekóder a cél** (tulajdonosi döntés, 2026-11-08): az, hogy egy kodeket az FFmpeg 9
tartalék háttér ([ADR 0005](../adr/ADR-0005-media-backends.md)) már megnyit, nem számít
megoldottnak; a backlog tétele a saját, pure-Rust dekóder.

Minden tételre érvényesek a szokásos szabályok: clean-room, nyilvános leírásból, pure Rust
([AGENTS.md](../../AGENTS.md) §2); ffmpeg csak tesztorákulum. Ahol nincs nyilvános specifikáció, vagy
szabadalmi / licencdöntés kell, ott **tulajdonosi döntés** szükséges, mielőtt bárki hozzányúl.

## Már megoldva vagy tervezve (ezért kimaradt)

A crate-ek, a [codec-parity.md](../codec-parity.md), a [file-format-parity.md](../file-format-parity.md),
a [cinema-codecs.md](../cinema-codecs.md) és a [gaps.md](../gaps.md) alapján, 2026-11-08-án. Ami a
tulajdonos listáján szerepelt, de itt már van:

| Kodek / formátum | Hol van |
|---|---|
| CineForm | saját dekóder és enkóder: `crates/cineform` |
| DTS, DTS-HD Master Audio (és DTS:X) | saját dekóder: `crates/dts` |
| Dolby TrueHD (Atmosszal) | saját dekóder: `crates/truehd` |
| JPEG XL | saját dekóder: `crates/jxl` |
| AVIF | tervben: [G10](../gaps.md#g10-container-and-codec-gaps-on-import) (AVIF elemek a HEIF-ben) |
| JPEG 2000 | saját dekóder és enkóder: `crates/j2k` |
| H.264 High 10 / 4:2:2 / 4:4:4, HEVC 4:2:2 / 4:4:4, HEVC és AV1 enkóder, E-AC-3 7.1 | saját kodekek, lásd [codec-parity.md](../codec-parity.md) |

Ami a listán volt, és már máshol tervben van (ott kell követni, nem itt):

| Kodek / formátum | Hol van a tervben |
|---|---|
| HEVC RExt kódolóeszközök | [codec-parity.md](../codec-parity.md), #167 |
| E-AC-3 hiányzó részei (AHT, enhanced coupling) | [G10](../gaps.md#g10-container-and-codec-gaps-on-import), #647 |
| MPEG-4 Part 2 (DivX / Xvid), JPEG XS, FLV | [G10](../gaps.md#g10-container-and-codec-gaps-on-import), [codec-parity.md](../codec-parity.md), [file-format-parity.md](../file-format-parity.md) |
| Kamera-RAW (Canon Cinema RAW Light és a többi) | #383, [codec-parity.md](../codec-parity.md) |
| MP3 és MPEG-2 enkóder | [G13](../gaps.md#g13-delivery-formats), [codec-parity.md](../codec-parity.md) |
| H.266 / VVC | CFA4-VVC-PLAN-001 (#721, jóváhagyva 2026-11-07) |

## K1 – elterjedt vagy professzionális (a backlogon belül először)

| Kodek / formátum | Típus | Megjegyzés |
|---|---|---|
| FFV1 | videó | |
| HAP | videó | |
| Huffyuv, Ut Video, MagicYUV, Lagarith | videó | Veszteségmentes köztes kodekek |
| ADPCM (IMA, Microsoft) | audio | |
| G.711 / G.722 | audio | Az MP4 felismeri az `ulaw` / `alaw` bejegyzést; a dekódolást ellenőrizni kell |
| AMR-NB / AMR-WB | audio | |
| WavPack | audio | |
| Monkey's Audio (APE) | audio | |
| Speex | audio | Az Ogg már felismeri, dekóder nincs |

## K2 – regionális, új generációs, szűkebb szakmai

| Kodek / formátum | Típus | Megjegyzés |
|---|---|---|
| MPEG-5 EVC | videó | |
| LCEVC | videó | Kiegészítő réteg más kodekekhez |
| AVS1 / AVS-P2, AVS+ | videó | Kínai |
| AVS2 (IEEE 1857.4), AVS3 (IEEE 1857.10) | videó | Kínai UHD / broadcast |
| AVS Audio, AVS2 Audio, AVS3 Audio | audio | Kínai |
| Audio Vivid | audio | Kínai, immerszív; az AVS3 Audiótól külön ellenőrizni |
| DRA (Digital Rise Audio) | audio | Kínai |
| MPEG-H 3D Audio | audio | |
| AC-4 | audio | Licenc: tulajdonosi döntés |
| VC-2, Dirac | videó | |
| Canopus HQ / HQX | videó | |
| Matrox MPEG-2 I-frame | videó | Valószínűleg már a meglévő MPEG-2 dekóderrel megy, csak ellenőrizni kell |
| ~~Sony SR Codec~~ | videó | Kimarad (tulajdonosi döntés, 2026-11-14): nincs nyilvános leírás, sem nyílt megvalósítás |
| RealVideo RV10–RV40 | videó | RealMedia konténer is kell |
| RealAudio Cook, SIPR, 14.4 / 28.8 | audio | |
| Sony ATRAC3, ATRAC3plus, ATRAC9 | audio | |
| G.729, EVS, iLBC | audio | Beszédkodekek |
| Musepack SV7 / SV8, TAK, TTA, Shorten | audio | Ritka veszteségmentes / zenei. Az OptimFROG kimarad (tulajdonosi döntés, 2026-11-14): csak zárt gyártói SDK |

## K3 – örökölt, játék és egyéb ritka (a legvégén)

Ezek többségéhez nincs nyilvános specifikáció, ezért a clean-room szabály miatt mindegyikhez
külön tulajdonosi döntés kell.

| Kodek / formátum | Típus |
|---|---|
| Bink, Bink Audio (a Bink 2 kimarad, tulajdonosi döntés, 2026-11-14: nincs nyílt dekóder) | játék |
| Smacker, Smacker Audio | játék |
| id RoQ, Interplay MVE, Interplay ACM | játék |
| CRIWARE Sofdec.Prime, Sofdec2, CRI ADX, CRI HCA | japán játék |
| Nintendo THP, Mobiclip, HVQM / HVQM4 | japán játék |
| Nintendo DSP ADPCM, BRSTM, BCSTM / BFSTM | japán játék |
| Sony PSP MPEG-4 AVC profilok | japán játék (valószínűleg a H.264-dekóder elég, csak a konténer kell) |
| Yamaha ADPCM, OKI ADPCM, NTT TwinVQ | régi audio |
| AMV Video | régi kínai lejátszók |
| Cinepak, Intel Indeo 2 / 3 / 4 / 5, Sorenson Video 1 / 3 | régi videó |
| Microsoft RLE / Video 1, Autodesk FLIC / FLC | régi videó |
| HuffYUV MT | régi videó |
| QDesign Music 2, Voxware MetaSound, TrueSpeech, Apple MACE 3 / 6, QCELP | régi audio |
| AVS2 3D / Multiview, AVS3 panoráma / immerszív profilok, AVS3 RTC Speech | kínai speciális |

## K4 – legutolsó: AVS-kapcsolódó technológiák

A K2-es AVS-munkához (AVS2, AVS3, AVS-audiók) kötődnek, és csak utána, a backlog legvégén jönnek.

| Technológia | Típus |
|---|---|
| TPG | AVS-kapcsolódó képformátum / konténer |
| SMT (Smart Media Transport) | multimédia-átviteli technológia |
| AVS3 Immersive Media | immerszív média szabványcsalád |

## Nem kodek, nincs külön feladat

- **ARIB STD-B32** (japán videó / audio / multiplex szabvány): a benne használt kodekek (MPEG-2,
  H.264, HEVC, AAC) és az MPEG-TS már megoldottak; legfeljebb egy ellenőrző teszt kell.
- **ISDB-T, ISDB-S / S3**: rádiós átviteli rendszerek, videószerkesztőben nincs teendő.

## Revíziótörténet

| Dátum | Változás | Összefoglaló |
|---|---|---|
| 2026-11-14 | minor | Tulajdonosi döntés: az OptimFROG, a Sony SR Codec és a Bink 2 kimarad (nincs nyilvános leírás és nyílt dekóder sem). A forráskeresésre váró tételek az ütemtervben (🔎) |
| 2026-11-09 | minor | A H.266 / VVC átkerült a „máshol tervben” táblába: saját, jóváhagyott terve van (CFA4-VVC-PLAN-001, #721) |
| 2026-11-08 | major | Áthelyezve a CFA4-AI_modules tárolóból (ott: PR #9) a CinemaFundArchitecture4-be, a tulajdonos döntésére. Összevetve az itteni kodekekkel: kimaradt a CineForm, a DTS / DTS-HD MA, a TrueHD és a JPEG XL (saját dekóder van), az AVIF (G10). A „máshol tervben” tábla ehhez a tárolóhoz igazítva. Saját dekóder a cél, az FFmpeg-tartalék nem számít megoldásnak |
| 2026-10-27 | major | Létrehozva a tulajdonos 2026-10-27-i kodeklistájából (a CFA4-AI_modules tárolóban) |
