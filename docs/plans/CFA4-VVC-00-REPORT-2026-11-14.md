# CFA4-VVC-00 – Technológiai és jogi elővizsgálat (jelentés)

**Terv:** [CFA4-VVC-PLAN-001](CFA4-VVC-PLAN-001-2026-11-07.md), VVC-00 munkacsomag, G0 kapu
**Dátum:** 2026-11-12 (mérések), 2026-11-14 (tulajdonosi döntések)
**Hol futott:** felhős fejlesztői környezet (tulajdonosi döntés, 2026-11-12), **nem** a referenciagép
**Állapot:** G0 feltételesen elfogadva (tulajdonos, 2026-11-14)

## 1. Összefoglaló

- **Minden komponens lefordul és működik** Ubuntu 24.04 / GCC 13.3 / CMake 3.28 alatt: VVdeC v3.2.1, VVenC v1.14.0, uvg266 (master), FFmpeg 9.0.2 `libvvenc`-kel. Az FFmpeg-hez a VVenC-et **megosztott könyvtárként** kell fordítani; statikusan a link elbukott (C++ futásidejű könyvtár).
- **Dekódolás:** a VVdeC 1,7–2,8× gyorsabb az FFmpeg natív `vvc` dekóderénél, és a kettő kimenete **bitre egyezik** mindhárom tesztfolyamon. Ez alátámasztja a „VVdeC elsődleges, FFmpeg tartalék” döntést.
- **Kódolás:** az uvg266 a mérés szerint sem gyorsabb, sem hatékonyabb a VVenC `faster` presetjénél; hasonló sebességnél kb. 2–3× akkora bitrátát igényel. **Döntés:** az uvg266 kikerül az exportprofilokból, a Gyors profil VVenC `faster`.
- **Konténerek – hiba az FFmpeg 9.0.2-ben:** a `libvvenc` kódolás MKV-be és MPEG-TS-be hibás DTS-eket ír (64 képből 34 dekódolható). MP4-be közvetlen kódolással hibátlan. A CFA4-nek a konténer-időbélyegeket a saját muxereivel kell írnia; addig VVC-export csak MP4.
- **Hardver:** az Intel szerint a Battlemage (Arc B, így a Pro B50 is) **nem** dekódol VVC-t, csak a Lunar Lake (Xe2-LPG) iGPU és az Xe3. NVIDIA-n nincs VVC. Az FFmpeg 9.0.2-ben van VA-API és QSV VVC-út.
- **Nem sikerült (hálózati tiltás):** VTM, JVET konformancia-bitfolyamok, természetes tesztvideók (Xiph). Ezek a G2 belépő feltételei.

## 2. Verziók és licencek (Dependency Manifest vázlat)

| Komponens | Verzió / commit | Dátum | Licenc | Megjegyzés |
|---|---|---|---|---|
| VVdeC | v3.2.1 / `cb1a3ef` | 2026-10-07 | BSD-3-Clause-Clear | VVC Main10 profil (README) |
| VVenC | v1.14.0 / `9428ea8` | 2026-01-21 | BSD-3-Clause-Clear | SIMD: AVX2 |
| uvg266 | master `48dfc49` | 2026-04-24 | BSD-3-Clause | 10 bithez `-DUVG_BIT_DEPTH=10` kell |
| uvg266 | v0.8.1 / `d008a00` | 2024-07-01 | BSD-3-Clause | 10 bites build nem indult („crc32c strategy” hiba) |
| FFmpeg | n9.0.2 / `946fcce` | 2026-09-18 | GPL-3 build (`--enable-gpl --enable-version3`, nonfree nélkül) | `libvvenc >= 1.6.1` |
| VTM (JVET) | – | – | BSD | Nem elérhető: a hálózat tiltja |

A BSD-3-Clause-Clear kifejezetten nem ad szabadalmi engedélyt. Mindegyik licenc GPL-3-kompatibilis.

## 3. FFmpeg 9.0.2 VVC-képességei

| Képesség | Állapot |
|---|---|
| Natív `vvc` dekóder | 8/10/12 bit, 4:2:0/4:2:2/4:4:4; többrétegű bitfolyam nincs; x86 SIMD (AVX2/AVX-512) |
| `libvvenc` kódoló | Van (`--enable-libvvenc`) |
| `libvvdec`, `libuvg266` | Nincs, saját binding kell |
| Hardveres | `vvc_vaapi` hwaccel, `vvc_qsv` dekóder |
| MP4 | `vvc1`/`vvi1`, `vvcC` olvasás/írás |
| MPEG-TS | stream type `0x33` olvasás/írás |
| Matroska | `V_MPEGI/ISO/VVC` olvasás/írás |
| Nyers | `vvc` demuxer/muxer (Annex B) |

## 4. Mérések

**Környezet:** felhős konténer, 4 vCPU Intel Xeon @ 2,1 GHz (AVX2 + AVX-512), 15 GB RAM, Ubuntu 24.04, GPU nincs; minden futtatás 4 szálon. Nem a referenciagép: az abszolút számok tájékoztatók, az arányok a fontosak.
**Tesztanyag:** szintetikus (mandelbrot + testsrc2, időbeli zajjal), 10 bit 4:2:0, 1080p30 64 kép, 2160p30 32 kép. Természetes tesztvideó nem volt elérhető, a BD-rate számok irányt mutatnak.

### 4.1. Kódolás (1080p, QP 32)

| Kódoló | fps | kbit/s | PSNR-Y (dB) |
|---|---|---|---|
| VVenC medium | 2,17 | 526 | 35,21 |
| VVenC faster | 12,53 | 536 | 34,82 |
| uvg266 medium | 3,47 | 2 235 | 35,37 |
| uvg266 ultrafast | 11,02 | 1 191 | 34,87 |

2160p, QP 32: VVenC faster 3,42 fps / 1 128 kbit/s; uvg266 ultrafast 3,08 fps / 3 740 kbit/s.

BD-rate a VVenC medium-hoz képest (QP 22/27/32/37, PSNR-Y): VVenC faster +63%, uvg266 medium +197%, uvg266 ultrafast +214%.

### 4.2. Dekódolás (4 szál, kiírás nélkül)

| Folyam | VVdeC fps | FFmpeg `vvc` fps | Arány |
|---|---|---|---|
| 1080p, VVenC medium QP32 | 150,6 | 90,0 | 1,7× |
| 1080p, uvg266 medium QP32 | 155,3 | 55,9 | 2,8× |
| 2160p, VVenC faster QP32 | 41,9 | 18,0 | 2,3× |

Az FFmpeg AVX-512 nélkül (`-cpuflags -avx512`) a mérési szóráson belül ugyanannyi. 4K lejátszáshoz 4 szálon egyik dekóder sem elég, ezért a proxy (VVC-FR-4) kell; a referenciagépen mérni kell.

### 4.3. Bitpontosság

- VVdeC = FFmpeg mindhárom folyamon (azonos MD5). Ez nem helyettesíti a VTM-összevetést.
- Az uvg266 `--debug` rekonstrukciója nem bitpontos a dekódolt kimenettel (93–102 dB PSNR).

### 4.4. Konténerek

| Út | MP4 | MPEG-TS | MKV |
|---|---|---|---|
| Nyers `.266` → `-c copy` | 8/64 kép elvész | Hiba: „first pts and dts value must be set” | Hiba: „unknown timestamp” |
| `libvvenc` közvetlenül | 64/64, rendben | Hibás DTS, 34 kép dekódolható | Ugyanígy, 34 kép |

Az MKV-ból kinyert bitfolyamból a VVdeC mind a 64 képet dekódolja: a hiba az időbélyegekben van, nem a bitfolyamban. **Következmény:** a VVC-03-ban a konténerbe írás a CFA4 muxereivel (`crates/isobmff`, `crates/matroska`, `crates/mpegts`) történik; addig VVC-export csak MP4. A VVC-TEST-004/005 erre regressziós esetet kap.

### 4.5. Seek

1080p, 10 s MP4 (VVenC faster), `ffmpeg -ss` öt pozíción: 0,58–0,71 s (folyamatindítással). Felső becslés; a szerkesztői seeket a VVC-04 méri.

## 5. G0 kapu

| G0 feltétel | Állapot |
|---|---|
| Forrásverziók rögzítve | VERIFIED (VTM kivételével) |
| Build Ubuntu-n | VERIFIED 24.04-en; 26.04-en nem |
| FFmpeg 9.0.2 képességek | VERIFIED |
| Sebesség- és minőségmérés | RÉSZBEN: felhős gépen, szintetikus anyaggal |
| VTM-orákulum, JVET konformancia | BLOCKED (hálózati tiltás) → G2 belépő feltétele |
| Konténerjelölések | VERIFIED; hibát találtunk (4.4) |
| Licencjegyzőkönyv | RÉSZBEN |
| Kódoló–profil hozzárendelés | ELDÖNTVE: uvg266 ki, Gyors = VVenC `faster` |

**Döntés (tulajdonos, 2026-11-14):** a G0 feltételesen elfogadva.

## 6. Licenc- és szabadalmi jegyzőkönyv (részleges)

- **Programok:** Access Advance **VVC Advance** (készülékpool) és **VCL Advance**; a Wikipedia szerint az Access Advance 2025 decemberében átvette a Via LA HEVC/VVC poolját (ellenőrizendő). Streamingre: Access Advance VDP és Avanci Video. A Sisvelnek nincs VVC-programja.
- **Nyílt forrású terjesztés:** egyik forrás sem mondja ki, hogy az ingyenes szoftveres terjesztés jogdíjmentes; a `cfa4-restricted-extras` besorolás (licensing.md §7) marad. A díjtáblák és szerződések a G7 jogi kapuban.
- Ez nem jogi vélemény.

Források: [Intel – Arc B-series VVC](https://community.intel.com/t5/Graphics/Arc-B-series-support-VVC-H-266-decoding/td-p/1699558) · [Access Advance VVC Advance](https://accessadvance.com/vvc-advance-patent-pool-general-pool-terms/) · [Lumenci – HEVC vs AV1 vs VVC licensing](https://lumenci.com/blogs/hevc-vs-av1-vs-vvc-royalties-licensing-risk/) · [Wikipedia – VVC](https://en.wikipedia.org/wiki/Versatile_Video_Coding)

## 7. Tulajdonosi döntések (2026-11-14)

1. Az uvg266 kikerül az exportprofilokból; a Gyors profil VVenC `faster`. Az uvg266 későbbi alacsony késleltetésű / streaming jelölt (az uvgRTP-vel együtt).
2. A G0 feltételesen elfogadva; a VTM/JVET konformancia a G2 belépő feltétele.
3. A felhős környezet hálózati engedélye jóváhagyva (`vcgit.hhi.fraunhofer.de`, `www.itu.int`, `media.xiph.org`).
