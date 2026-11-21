# F1: forrás-, licenc- és függőségaudit (CFA4-RENDER-INTEGRATION-PLAN-001)

**Dátum:** 2026-11-21 · **Jelleg:** csak olvasás; sem build, sem telepítés, sem kódátvétel.
**Szabály (tulajdonos, 2026-11-21):** ami a tervben már máshol létezik (másik projekt, crate, ADR, PR),
azt nem építjük újra; **meglévőként** jelöljük, az audit csak rögzítéseket, függőségeket és kockázatokat ad hozzá.

## 1. Eredmény jelöltenként

| Jelölt | Döntés | Licenc | Rögzített állapot | Egy mondatban |
|---|---|---|---|---|
| OpenImageIO | **PASS** (meglévő: ADR 0003 A3) | Apache-2.0 | 3.3.0.0, `8a21cac9fbab` (2026-11-20, `main`; a beépítéskor kiadási címkére kell rögzíteni) | A beépítést az ADR 0003 már eldöntötte (külön ADR kell hozzá); az audit feltétele, hogy az FFmpeg-bővítménye kikapcsolva marad. |
| VapourSynth | **PASS**, csak külön folyamatban futó worker (POC) | LGPL-2.1 (mag); a bővítmények külön lásd 3. pont | R83, `3b2c049876f6` (2026-11-15) | A retusáló szűrők (zajszűrés, deband, deinterlace, IVTC) POC-jelöltje; a Python-kérdés eldöntve (4. pont). |
| ReelForge | **REFERENCE_ONLY** (legfeljebb ADAPT_PATTERN_ONLY) | MIT | 0.2.1, `ac85a94cfbf8` (2026-10-20), 1 kiadási címke | Egyetlen karbantartó: az életképességi szabály (≥2) miatt csak minta; a DAG/dirty-region ötletei használhatók, kód nem. |
| avio / `ff-*` | **REFERENCE_ONLY** | MIT OR Apache-2.0 | 0.18.4, `453c67b992c4` (2026-11-03), 34 címke | Egyetlen karbantartó; FFmpeg 7.x/8.x-ra épül, a CFA4 FFmpeg 9.0.2; saját libav-kötés (A7 sérülne), a CFA4-nek már van `crates/ffmpeg`. |
| EC-jelölt (#11) | **BLOCKED** | nem vizsgált | – | A repo-hivatkozása nincs a CFA4 repóban (márkaszűrő), ez a munkamenet nem látta; a blokkoló: az upstream link. |
| rust-motion | **REFERENCE_ONLY** | MIT OR Apache-2.0 | 1.0.0, `b1873581ac02` (2026-11-20), 1 kiadási címke | Egyetlen karbantartó; a CFA4-gyel közös upstream-eredetű kódot tartalmaz (5. pont), az új rész (vektor, útvonal, alakzat, diagram, `tiny-skia`/`resvg` raszterizáló) minta. |
| Blender | **PASS** csak külső folyamatként | GPL (a pontos változat a rögzítéskor ellenőrzendő) | verzió PENDING (a rögzítés az F5-höz) | Nem linkelünk hozzá; EXR-szekvencia cserével, külön folyamatban fut. |
| Natron | **REFERENCE_ONLY** (tulajdonos, 2026-11-21) | GPL-2.0 (a `openfx-misc` GPL-2.0-or-later, `docs/gaps.md`) | – | Csak képességi minta, kódátvétel nincs; az ADR 0003 A4 változatlan, amíg a tulajdonos nem válaszol. |
| MLT, MovieLite, Gausian, Dioxuscut | **REFERENCE_ONLY** (terv szerint) | – | – | Ebben a körben kódot nem olvastam; az átfedés-döntéshez a Dioxuscut a `rust-motion` 5. pontjával együtt nézendő. |

PASS = beépítési/futtatási szempontból nincs jogi vagy technikai akadály a megadott feltétellel. REFERENCE_ONLY = csak
képességi vagy módszertani minta, kód és függőség nem kerül be. BLOCKED = a megnevezett akadály elhárulásáig nem dönthető.

## 2. Életképesség és MSRV (a terv F1 4–5. pontja)

| Jelölt | Kiadás 12 hónapon belül | Karbantartók a git-történetben | MSRV | CFA4 toolchain (1.97.1) |
|---|---|---|---|---|
| ReelForge | igen (0.2.1, 2026-10) | 1 | 1.97 | megfelel |
| avio | igen (34 címke) | 1 (két git-azonosító: `itsakeyfut`, `itsakey`) | 1.93.0 | megfelel |
| rust-motion | igen (1.0.0) | 1 (két azonosító: `김성주`, `Sung Joo Kim`) | nincs megadva (edition 2021) | PENDING |

Mindhárom a 300 commitos lekért történet alapján egyszemélyes, ezért a terv szabálya szerint legfeljebb ADAPT_PATTERN_ONLY.
(A bot-fiókokat nem számoltam.) A „karbantartó” itt a commit-szerző, nem a GitHub-jogosultság.

## 3. VapourSynth: bővítmény- és szkriptkészlet licencei (A11 nyilvántartás kezdete)

| Komponens | Szerep | Licenc | Rögzített állapot | GPL-3.0-or-later szempont |
|---|---|---|---|---|
| VapourSynth mag | szűrőgráf | LGPL-2.1 | R83 | linkelhető; a CFA4-ben úgysem linkeljük, külön worker |
| HAvsFunc | QTGMC-család szkriptjei | Unlicense (közkincs) | v34, `9a0683a` | rendben; **függ a vs-jetpack csomagtól** |
| vs-jetpack | a HAvsFunc és más szkriptek alapja | MIT | `af85c5f4781b` (2026-11-16), Python ≥ 3.13 | rendben; függőségei: `numpy`, `rich`, `psutil`, `jetpytools` |
| VapourSynth-BM3D | zajszűrés | MIT | `f85173a` | rendben |
| neo_f3kdb | deband | GPL-3.0 | `939e6fa` | GPL-3.0-or-later-rel kompatibilis, de csak külön folyamatban; nem linkelhető Apache/LGPL komponenshez |
| fmtconv | formátum- és színátalakítás | WTFPL | `77f26b9` | rendben |
| mvtools, nnedi3/znedi3, eedi3, fft3dfilter, dfttest | a QTGMC motorjai | **PENDING** | – | nem ellenőriztem; a rögzítéskor kötelező, GPL-2.0-only kizáró ok (AGENTS.md) |
| IVTC | VapourSynth `vivtc` | a mag része (LGPL-2.1) | R83 | rendben, a pontos modulút a POC-ban |

Következtetés: a QTGMC-lánc minden tagját jogilag a rögzítés előtt egyenként kell lezárni; a három ellenőrizetlen
motor-licenc a F1 nyitott tétele (a kockázatnyilvántartásban R-07).

## 4. A VapourSynth Python-kérdés (kilépési feltétel)

- A CFA4 Python-csomagja `requires-python >= 3.12` (`pyproject.toml`); az ellenőrző konténer rendszer-Pythonja 3.13.18.
  A referenciagép rendszer-Pythonja a `render_environment.py` futtatásáig PENDING.
- A vs-jetpack (a QTGMC-szkriptek alapja) `>= 3.13`-at kér; a VapourSynth R80 óta támogatja a szabadon szálazott Pythont.
- **Döntés (javaslat, a tulajdonos hagyja jóvá):** a worker nem hoz saját Pythont; ugyanazt a rendszer-Python 3.13+ ágat
  használja, mint a Qt-környezet, külön virtuális környezetben. Így az A11 szerinti Python-verziók száma egy (3.13+), és a
  CFA4 `requires-python` alsó határát nem kell emelni. Ha a referenciagép rendszer-Pythonja 3.13-nál régebbi, a döntés megbukik,
  és a worker külön Pythont kap (ez külön jóváhagyás).

## 5. rust-motion: kódátfedés a CFA4-gyel

A `rust-motion` workspace sok crate-je a CFA4 közös upstream-jének leszármazottja: `bitstream`, `apv`, `geom`, `color`,
`audio-dsp`, `mxf`, `captions`, `psd`, `dng`, `hevc` stb. Fájlszám- és tartalomeltérés minden vizsgált crate-ben van
(például `bitstream`: 2 eltérő bejegyzés, `mxf`: 28), tehát ezek **nem** új képességek: a CFA4-ben már megvannak (meglévő).
Az egyedi rész: `vector`, `paths`, `shapes`, `charts`, `animation`, `rasterizer` (`tiny-skia` 0.11, `resvg` 0.45),
`renderer` (FFmpeg-csővezeték). Következmény: kódátvétel nem indokolt, a CFA4 saját `crates/text`, `crates/captions` és
`crates/render` bővítése előtt a minta használható; a Dioxuscut átfedése külön, kódolvasás nélkül nem eldönthető.

## 6. Az FFmpeg 9.0.2 és az A7 (második libav tilalma)

- **OpenImageIO:** az FFmpeg-bővítmény opcionális (`checked_find_package (FFmpeg VERSION_MIN 4.0)`), tehát bekapcsolva
  második libav-kötés lenne. Feltétel: a beépítés `-DUSE_FFMPEG=OFF`-fal épül (a CFA4-ben a képfájl-I/O-hoz nem kell).
  Opcionális, tiltandó/ellenőrzendő még: R3DSDK (zárt), Nuke, OpenCV, libheif, LibRaw, OpenJPEG, JXL.
- **avio:** a README FFmpeg 7.x vagy 8.x fejlesztői csomagokat kér, a build-szkript `ffmpeg8` cfg-t ismer; 9.0.2-vel való
  fordulás **nem kipróbált (PENDING)**. Saját libav-kötést hoz (`ff-sys`, `unsafe_code = "warn"`), a CFA4-ben `unsafe` csak
  a `crates/platform`-ban lehet. Ezért REFERENCE_ONLY.
- **rust-motion:** FFmpeg csővezetéken át, külön folyamat; nincs libav-linkelés, ez megfelel az A7-nek.
- **ReelForge:** a kimenet a gazda FFmpeg-jén át; libav-linkelés nincs.

## 7. Állítások ellenőrzése (claim test, mintavétel)

| Állítás | Eredmény |
|---|---|
| ReelForge: „parallel raster, fixed-point color, bicubic resize” | a README ezt állítja; a képút 8 bites fixpontos, a CFA4 16/32 bites útjának (MQ-1) nem helyettesítője |
| rust-motion: „determinisztikus CPU-rasterizáló (`tiny-skia`)” | a README ezt állítja; a bit-azonosságot nem mértem (POC) |
| avio: „FFmpeg 7.x vagy 8.x” | egyezik a build-szkripttel; 9.0.2 nem szerepel |

Mérés nem történt: az F1 csak olvas. A minőségi kapu (MQ-1..8) előszűrése: egyik REFERENCE_ONLY jelölt sem kerül POC-ba,
ezért MQ-mérésre csak az OpenImageIO (EXR-pontosság) és a VapourSynth (szűrőminőség) kerül.

## 8. Nyitott tételek

1. EC-jelölt (#11): az upstream link megadása után pótolandó (BLOCKED).
2. A QTGMC motorok licencei (mvtools, nnedi3/znedi3, eedi3, fft3dfilter, dfttest).
3. Blender: licencváltozat és a rögzítendő verzió.
4. A referenciagép Python-verziója és `render_environment.py` futtatása.
5. avio fordulása FFmpeg 9.0.2-vel (csak ha a REFERENCE_ONLY döntés felülvizsgálandó).
6. A tulajdonos válasza az ADR 0003 A4 (`openfx-misc` mint első csomag) és a Natron „csak képesség” elv összhangjára.
