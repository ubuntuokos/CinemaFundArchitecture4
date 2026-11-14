# CFA4-VVC-00 – Technológiai és jogi elővizsgálat (jelentés)

**Terv:** [CFA4-VVC-PLAN-001](CFA4-VVC-PLAN-001-2026-11-07.md), VVC-00 munkacsomag, G0 kapu
**Dátum:** 2026-11-12 (mérések), 2026-11-14 (tulajdonosi döntések; VTM-, JVET- és természetes anyagos pótlás, 4.6–4.7)
**Hol futott:** felhős fejlesztői környezet (tulajdonosi döntés, 2026-11-12), **nem** a referenciagép
**Állapot:** G0 feltételesen elfogadva (tulajdonos, 2026-11-14)

## 1. Összefoglaló

- **Minden komponens lefordul és működik** Ubuntu 24.04 / GCC 13.3 / CMake 3.28 alatt: VVdeC v3.2.1, VVenC v1.14.0, uvg266 (master), FFmpeg 9.0.2 `libvvenc`-kel. Az FFmpeg-hez a VVenC-et **megosztott könyvtárként** kell fordítani; statikusan a link elbukott (C++ futásidejű könyvtár).
- **Dekódolás:** a VVdeC 1,7–2,8× gyorsabb az FFmpeg natív `vvc` dekóderénél, és a kettő kimenete **bitre egyezik** mindhárom tesztfolyamon. Ez alátámasztja a „VVdeC elsődleges, FFmpeg tartalék” döntést.
- **Kódolás:** az uvg266 a mérés szerint sem gyorsabb, sem hatékonyabb a VVenC `faster` presetjénél; hasonló sebességnél kb. 2–3× akkora bitrátát igényel. **Döntés:** az uvg266 kikerül az exportprofilokból, a Gyors profil VVenC `faster`.
- **Konténerek – hiba az FFmpeg 9.0.2-ben:** a `libvvenc` kódolás MKV-be és MPEG-TS-be hibás DTS-eket ír (64 képből 34 dekódolható). MP4-be közvetlen kódolással hibátlan. A CFA4-nek a konténer-időbélyegeket a saját muxereivel kell írnia; addig VVC-export csak MP4.
- **Hardver:** az Intel szerint a Battlemage (Arc B, így a Pro B50 is) **nem** dekódol VVC-t, csak a Lunar Lake (Xe2-LPG) iGPU és az Xe3. NVIDIA-n nincs VVC. Az FFmpeg 9.0.2-ben van VA-API és QSV VVC-út.
- **Pótolva 2026-11-14-én (a hálózati engedély után):** VTM-24.0 orákulum és a JVET v1 konformanciakészlet (283 folyam). A Main 10 profilon a **VVdeC minden folyamon bitpontos**; a palettás Main 4:4:4 10 folyamokat (21) és a többrétegű folyamokat (10) nem dekódolja. Az FFmpeg natív dekódere 273 egyrétegű folyamból 269-en bitpontos (4.6).
- **Természetes anyagon (Xiph, 4 × 1080p)** a VVenC `faster` BD-rate-je +22% a `medium`-hoz képest, az uvg266-é +132% (medium) és +172% (ultrafast); a VVenC `faster` közben gyorsabb is (3,8 fps, uvg266 ultrafast 2,9 fps, 2 szálon). Az uvg266 kizárását ez megerősíti. **Javítás:** az uvg266 kimenete VTM-mel ellenőrizve bitpontos; a 4.3-ban leírt eltérés nem reprodukálható (4.7).

## 2. Verziók és licencek (Dependency Manifest vázlat)

| Komponens | Verzió / commit | Dátum | Licenc | Megjegyzés |
|---|---|---|---|---|
| VVdeC | v3.2.1 / `cb1a3ef` | 2026-10-07 | BSD-3-Clause-Clear | VVC Main10 profil (README) |
| VVenC | v1.14.0 / `9428ea8` | 2026-01-21 | BSD-3-Clause-Clear | SIMD: AVX2 |
| uvg266 | master `48dfc49` | 2026-04-24 | BSD-3-Clause | 10 bithez `-DUVG_BIT_DEPTH=10` kell |
| uvg266 | v0.8.1 / `d008a00` | 2024-07-01 | BSD-3-Clause | 10 bites build nem indult („crc32c strategy” hiba) |
| FFmpeg | n9.0.2 / `946fcce` | 2026-09-18 | GPL-3 build (`--enable-gpl --enable-version3`, nonfree nélkül) | `libvvenc >= 1.6.1` |
| VTM (JVET) | VTM-24.0 / `ca331de` | 2026-04-08 | BSD-3-Clause | Orákulum (4.6); második orákulumként VTM-12.0 |

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
- ~~Az uvg266 `--debug` rekonstrukciója nem bitpontos a dekódolt kimenettel (93–102 dB PSNR).~~ **Nem reprodukálható** (2026-11-14, 4.7): 35 uvg266-folyamon (természetes és szintetikus anyag) a rekonstrukció, a VTM, a VVdeC és az FFmpeg kimenete bitre egyezik.

### 4.4. Konténerek

| Út | MP4 | MPEG-TS | MKV |
|---|---|---|---|
| Nyers `.266` → `-c copy` | 8/64 kép elvész | Hiba: „first pts and dts value must be set” | Hiba: „unknown timestamp” |
| `libvvenc` közvetlenül | 64/64, rendben | Hibás DTS, 34 kép dekódolható | Ugyanígy, 34 kép |

Az MKV-ból kinyert bitfolyamból a VVdeC mind a 64 képet dekódolja: a hiba az időbélyegekben van, nem a bitfolyamban. **Következmény:** a VVC-03-ban a konténerbe írás a CFA4 muxereivel (`crates/isobmff`, `crates/matroska`, `crates/mpegts`) történik; addig VVC-export csak MP4. A VVC-TEST-004/005 erre regressziós esetet kap.

### 4.5. Seek

1080p, 10 s MP4 (VVenC faster), `ffmpeg -ss` öt pozíción: 0,58–0,71 s (folyamatindítással). Felső becslés; a szerkesztői seeket a VVC-04 méri.

### 4.6. Pótlás (2026-11-14): VTM-orákulum és JVET konformancia

A hálózati engedély után pótolva, ugyanabban a felhős környezetben (2 vCPU Intel Xeon @ 2,3 GHz,
AVX2, AVX-512 nélkül; Ubuntu 24.04, GCC 13.3). Ugyanazok a VVdeC-, VVenC-, uvg266- és FFmpeg-commitok,
mint a 2. fejezetben, újrafordítva.

**Orákulum:** VTM-24.0 (`ca331de`, 2026-04-08, a legutóbbi kiadás), második VTM-ként VTM-12.0 (azokra
a folyamokra, ahol a VTM-24.0 hibázott vagy eltért; a régi kód GCC 13-mal csak `-include cstdint` és
`-Werror` nélkül fordul). **Korpusz:** a JVET H.266 v1 konformanciakészlete (ITU `bitstream_exchange/VVC/FDIS_r1`,
283 folyam, 285 MB), ebből 273 egyrétegű és 10 többrétegű. **Összevetés:** a dekódolt YUV-fájl MD5-je
a csomag `*.yuv.md5` értékével. Az FFmpeg `-fps_mode passthrough -noautoscale` kapcsolókkal fut
(enélkül a CLI a képkocka-időzítés és a felbontásváltás miatt képeket ismétel, dob vagy átméretez: hat
folyamon ez hamis eltérést okozott, lásd lent).

| Dekóder | Egyezik (273 egyrétegű) | Eltér | Nincs kimenet |
|---|---|---|---|
| VTM-24.0 | 272 | 0 | 1 (`HRD_B_2`: „Exceeded FIFO size”; a VTM-12.0 helyesen dekódolja) |
| VVdeC v3.2.1 | 252 | 0 | 21 (mind palettás SPS: „palette mode is not yet supported”) |
| FFmpeg 9.0.2 natív `vvc` | 269 | 4 | 0 |

- A `STILL_B_1` és `STILL444_B_1` referenciája a szabvány szerint csak az első kép (Main 10 Still
  Picture); mindhárom dekóder első képe egyezik vele.
- **VVdeC:** a 21 hiányzó folyam (`8b422_G…L`, `10b422_G…L`, `8b444_A/B`, `ACT_A/B`, `PALETTE_A…E`) mind
  a Main 4:4:4 10 profilhoz tartozik, és az SPS-ben engedélyezett palettát a VVdeC elutasítja. A palettát
  nem használó 4:2:2-es folyamokat (`8b422_A…F`, `10b422_A…F`) és a `10b444_A/B`-t bitpontosan dekódolja.
  **A Main 10 profilon a VVdeC minden folyamon bitpontos.**
- **FFmpeg, valódi eltérés:** `FIELD_A_4`, `FIELD_B_2` (mezőkódolás: azonos képszám, más tartalom),
  `GDR_D_1` (hiányzó referenciákra mutató GDR-rel induló folyam: dekódolási hiba), `MNUT_B_3` (vegyes
  NAL-típusok: dekódolási hiba, kevesebb kép). A `BOUNDARY_A_3`, `GDR_B_2` és `RPR_A…D` eltérése az FFmpeg
  CLI időzítéséből és automatikus átméretezéséből jött, a fenti kapcsolókkal bitpontos.
- **Többrétegű folyamok** (`OLS_A…C`, `VPS_A…C`, `ILRPL_A`, `OPI_B`, `SPATSCAL_A`, `SPATSCAL444_A`): a VVdeC
  nem ad kimenetet, az FFmpeg kimenete eltér (többrétegű dekódolás nincs, `PATCHWELCOME`). A VTM-24.0
  az `OLS_A_5` alaprétegét (`-p 0`) helyesen adja, az 1-es OLS-en hibával áll le; a többrétegű
  VTM-orákulumot a VVC-02 tisztázza.

**Következmény a VVC-02-re:** a VVdeC-út vállalt köre a Main 10 profil (egyrétegű), ezen a VTM-mel
bitpontos. A palettás Main 4:4:4 10 folyamokra a VVdeC-út „nem támogatott” hibát ad (never-crash), és
az FFmpeg-tartalékra vált, amely ezeket bitpontosan dekódolja. A mezőkódolt, a GDR-rel hiányzó
referenciákkal induló és a vegyes NAL-típusú folyamokon az FFmpeg nem bitpontos, ezért ott nem tartalék.
A többrétegű folyamokat egyik út sem támogatja. A CFA4 tesztharnesze az FFmpeg CLI-t
`-fps_mode passthrough -noautoscale` kapcsolókkal hívja, vagy `framemd5`-öt használ.

### 4.7. Pótlás (2026-11-14): kódolás természetes anyagon, uvg266-bitpontosság

**Tesztanyag:** Xiph `derf` 1080p50 (SVT): `crowd_run`, `ducks_take_off`, `old_town_cross`, `park_joy`, az
első 32 kép, 10 bites 4:2:0-ra alakítva; 2160p50 `crowd_run`, 16 kép. **Kódolás:** 2 szálon, QP 22/27/32/37
(random access, a kódolók alapbeállításával). A PSNR-Y a VVdeC kimenetéből. **Környezet:** a 4.6-os, 2 vCPU,
AVX-512 nélkül, ezért az fps-ek nem vethetők össze a 4.1-gyel.

BD-rate a VVenC `medium`-hoz képest (PSNR-Y):

| Folyam | VVenC faster | uvg266 medium | uvg266 ultrafast |
|---|---|---|---|
| crowd_run | +21,4% | +65,6% | +109,6% |
| ducks_take_off | +20,8% | +115,2% | +109,4% |
| old_town_cross | +33,1% | +295,7% | +384,9% |
| park_joy | +13,9% | +51,9% | +82,1% |
| **Átlag** | **+22,3%** | **+132,1%** | **+171,5%** |

Átlagos kódolási sebesség (1080p, 2 szál): VVenC medium 0,51 fps, VVenC faster 3,76 fps, uvg266 medium
0,83 fps, uvg266 ultrafast 2,93 fps. 2160p, QP 32: VVenC faster 1,06 fps / 13 733 kbit/s / 31,15 dB;
uvg266 ultrafast 0,80 fps / 27 023 kbit/s / 31,94 dB.

A szintetikus anyaghoz képest (4.1) a sorrend ugyanaz, a VVenC `faster` vesztesége kisebb (+22% a +63%
helyett). **A VVenC `faster` természetes anyagon gyorsabb és sokkal hatékonyabb az uvg266 ultrafastnál,
ez megerősíti az uvg266 kizárását az exportprofilokból.**

**Bitpontosság:** mind a 66 kódolt folyamon (VVenC és uvg266, 1080p és 2160p) a VTM-24.0, a VVdeC és az
FFmpeg kimenete bitre egyezik. Az uvg266 `--debug` rekonstrukciója mind a 33 uvg266-folyamon, valamint két
új szintetikus (mandelbrot + testsrc2, időbeli zajjal) folyamon is bitre egyezik a dekódolt kimenettel.
A 4.3-ban leírt 93–102 dB-es eltérés tehát nem reprodukálható; az eredeti mérés nyers adatai nem maradtak
meg, ezért az ok nem deríthető ki, valószínűleg mérési hiba volt.

Nyers adatok és szkriptek: [`CFA4-VVC-00-data/`](CFA4-VVC-00-data/) (CSV-k, szkriptek; médiafájl nincs).

## 5. G0 kapu

| G0 feltétel | Állapot |
|---|---|
| Forrásverziók rögzítve | VERIFIED (a VTM-24.0 is, 2026-11-14) |
| Build Ubuntu-n | VERIFIED 24.04-en; 26.04-en nem |
| FFmpeg 9.0.2 képességek | VERIFIED |
| Sebesség- és minőségmérés | RÉSZBEN: felhős gépen; természetes anyaggal pótolva (4.7), a referenciagépen a VVC-02 méri |
| VTM-orákulum, JVET konformancia | VERIFIED 2026-11-14 (4.6): Main 10 profilon a VVdeC bitpontos; a teljes G2-konformancia a VVC-02-ben |
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
4. A jelentés és a v1.2 terv jóváhagyva, az ADR 0013 (VVC-01) elfogadva; a VTM/JVET és a természetes anyagos mérés pótlását a tulajdonos ugyanekkor elrendelte (eredmény: 4.6–4.7).
