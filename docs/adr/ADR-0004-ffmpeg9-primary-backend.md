# ADR 0004 – FFmpeg 9 az elsődleges, a GStreamer a másodlagos médiaháttér

**Dátum:** 2026-10-24 · **Állapot:** elfogadva (a tulajdonos, `ubuntuokos`, 2026-10-24-i döntése) ·
**Módosítja:** [ADR 0002](ADR-0002-gstreamer-backend.md) (a GStreamer szerepe),
[ADR 0003](ADR-0003-aswf-and-conversion-sources.md) 3. pont (az FFmpeg helye),
az `AGENTS.md` és a `CLAUDE.md` „ffmpeg csak tesztorákulum” szabálya

## Döntés

A CFA4 médiaháttereinek **alapsorrendje**:

| Sorrend | Háttér | Szerep |
|---|---|---|
| 1. | **FFmpeg 9** (libavformat, libavcodec, libavfilter, libswscale, libswresample) | **elsődleges** médiaháttér: konténerek olvasása és írása, dekódolás, kódolás, szűrők, hardveres gyorsítás |
| 2. | **GStreamer** (`crates/gstreamer`, ADR 0002) | **másodlagos** médiaháttér: amit az FFmpeg 9 nem tud, vagy ahol mérés szerint jobb (pl. PipeWire-felvétel, Qt6-előnézet) |
| 3. | **Saját Rust-kodekek és konténerek** (`crates/h264`, `hevc`, `isobmff` …) | **tartalék**: mindig elérhető, ha az FFmpeg 9 és a GStreamer hiányzik vagy hibázik; továbbá a kimenetek bitpontos ellenőrzése |

Ez az alapsorrend minden feladatra érvényes, **kivéve**, ahol az alábbi útválasztási tábla mást
rögzít. A legfontosabb kivétel: az idővonal lejátszása és léptetése azokban a formátumokban,
amelyeket a saját motor már igazoltan, bitpontosan kezel, a saját motorral kezd (az örökölt
upstream motor eredeti, valódi gépen mért erőssége).

### Útválasztási tábla (feladatonkénti sorrend)

| Feladat (melyik alkalmazás képessége) | 1. | 2. | 3. |
|---|---|---|---|
| Importálás, fájlinfó, ritka formátumok (minden terület) | FFmpeg 9 | GStreamer | saját |
| Konvertálás, kötegelt átkódolás, szűrők (Shutter Encoder, Videomass mintájára) | FFmpeg 9 | GStreamer | saját |
| Export, kiadási formátumok (Resolve „Deliver”-szerű, publikálás) | FFmpeg 9 | GStreamer | saját |
| Idővonal lejátszása és léptetése a saját motor által igazoltan kezelt formátumokban (Premiere, Kdenlive, Resolve „Cut”-szerű vágás) | saját (+ natív hardveres dekóder, ADR 0001) | FFmpeg 9 | GStreamer |
| Idővonal lejátszása és léptetése a többi formátumban | FFmpeg 9 | GStreamer | — |
| Hang importálása és exportálása (Audacity-szerű) | FFmpeg 9 | GStreamer | saját |
| Hangfeldolgozás, effektek (Audacity-, Fairlight-szerű) | saját DSP (+ LV2-host külön folyamatban, ADR 0003 A6) | — | — |
| Feliratozás: hullámforma és videó (Subtitle Edit-szerű) | FFmpeg 9 | saját | — |
| Felvétel: képernyő, kamera, hang (PipeWire, V4L2) | GStreamer | FFmpeg 9 | — |
| Qt6-előnézet kimenete | GStreamer vagy saját (mérés dönti el, ADR F8 5. lépés) | — | — |
| Állóképek, rajz és festés (Krita-szerű) | OpenImageIO (ADR 0003 A3) | saját | — |

### Az útválasztás szabályai

1. **Egy útválasztó, egy felület.** Minden képesség csak a motor `MediaSource`, `VideoEncoder` és
   szűrőfelületét hívja, soha nem közvetlenül az FFmpeg 9-et vagy a GStreamert. A tábla a kódban
   egyetlen helyen él (egy útválasztó modulban), és felhasználói beállítással felülbírálható.
   Új alkalmazásképesség beépítésekor a táblába kerül egy sor, nem új közvetlen hívás.
2. **Mérés dönti el a változást** (VISION.md 2.4: bizonyíték, nem állítás). Egy sor sorrendje
   csak teszttel és valódi gépes méréssel változhat: ha például az FFmpeg 9 egy formátum
   léptetésében mérhetően ugyanolyan képkocka-pontos és gyors, mint a saját motor, előre
   kerülhet. A változás és a mérés ebben az ADR-ben vagy utódjában rögzítendő.
3. **A saját dekóderek ellenőrként is maradnak.** Az FFmpeg 9 kimenetét a tesztek a saját,
   bitpontos dekóderekhez mérik (és fordítva), így a saját kodekek az FFmpeg-útvonal minőségét
   is garantálják.

Ami **nem** változik:

- A **Rust-motor** marad a projekt, az idővonal, a lejátszás vezérlése, a render, az export
  vezérlése és a parancsrendszer gazdája ([ADR F8](ADR-F8-QT6-RUST-ENGINE-2026-10-23.md)). A
  Qt6-felület továbbra is ezzel beszél. Az FFmpeg 9 és a GStreamer a motor `MediaSource` és
  `VideoEncoder` felületei mögé kerül, nem helyette.
- A natív VA-API és NVDEC/NVENC útvonal (`crates/platform`, [ADR 0001](0001-platform-ffi.md))
  megmarad; hogy egy feladatot ez vagy az FFmpeg 9 lát el, mérés dönti el.
- A **gyártófüggetlenség** (AGENTS.md): egy funkció sem lehet csak NVIDIA-n elérhető.
- Az upstream alkalmazásban mentett projektek (`.fcproj`) továbbra is megnyithatók (A1, `crates/project/src/legacy.rs`).

## Szabályok

1. **Licenc.** Az FFmpeg 9-et csak GPL-3-mal kompatibilis buildként használjuk:
   `--enable-gpl --enable-version3`, **soha** `--enable-nonfree`. Nem szabad (`nonfree`) vagy
   GPL-3-mal össze nem férő külső könyvtár (pl. `libfdk_aac`) nem kerülhet a buildbe. Az
   FFmpeg-forráskódot nem másoljuk a repóba; az FFmpeg dinamikusan töltődik be.
2. **Bekötés.** Az FFmpeg 9 egy új, opcionális crate mögé kerül (munkanév: `crates/ffmpeg`,
   `cfa4-ffmpeg`, L5 réteg), a `crates/gstreamer` mintájára. Ha a választott Rust-kötés biztonságos
   API-t ad, a crate-ben nincs `unsafe`; ha saját FFI-réteg kell, az ADR 0001 feltételei érvényesek
   (`// SAFETY:` minden `unsafe` blokknál, biztonságos `Result` API, opcionális Cargo-funkció).
   A kötés kiválasztása az 1. lépés része, FFmpeg 9-támogatás és licenc alapján.
3. **Nem crash-el** (AGENTS.md 0. pont): minden FFmpeg-hiba `Result`; sérült vagy rosszindulatú
   fájl hibaüzenetet ad, nem összeomlást. Mutációs fuzz-teszt kötelező a demuxer- és
   dekóderútvonalra.
4. **Tartaléklánc.** A tábla adott sorában ha az első háttér nincs telepítve, egy formátumot nem
   ismer, vagy hibázik, a feladatot a következő kapja (alapsorrendben: FFmpeg 9 → GStreamer →
   saját Rust-kodekek). Egy addig működő fájl a
   regisztráció miatt soha nem romolhat el.
5. **Tesztorákulum külön marad.** A CI-ben használt, rögzített FFmpeg 9.0.2 CLI
   (`scripts/ci/install-ffmpeg-9.0.2.sh`) továbbra is független orákulum a saját kodekekhez. A
   termékbe kerülő FFmpeg-kimenetet a saját dekóderek és az ffprobe is ellenőrzik.
6. **Szabadalmak.** A H.264, HEVC, AAC stb. szabadalmi kérdése független a szerzői jogi
   licenctől; kiadás előtt külön vizsgálandó (ADR 0003 6. pont).
7. **Bizonyíték, nem állítás** (VISION.md 2.4): hogy egy formátumot melyik háttér kezel, teszt és
   valódi gépes mérés dönti el.

## Lépések

| Lépés | Tartalom | Állapot |
|---|---|---|
| 0 | Útválasztó modul a motorban (a fenti tábla, felhasználói felülbírálás, `probe()`-alapú elérhetőség), kezdetben a mai viselkedéssel | tervezett |
| 1 | `cfa4-ffmpeg` crate: kötés kiválasztása, `probe()` (verzió, buildopciók, licenc: GPL-3-as build ellenőrzése, kodekek, hardveres gyorsítók), `discover()`; tesztek FFmpeg nélkül is fordulnak | tervezett |
| 2 | Importálás és lejátszás a tábla szerint: importnál és a nem igazolt formátumoknál az FFmpeg 9 az első, utána a GStreamer, végül a beépített megnyitók; az igazolt formátumok idővonal-lejátszása a saját motorral kezd | tervezett |
| 3 | Export: FFmpeg-kódolók a `VideoEncoder` mögött, gyártónként (NVENC, VA-API, Vulkan Video, szoftveres) | tervezett |
| 4 | Szűrők és konvertálás (libavfilter): a VISION.md konvertálási képességei (Shutter Encoder, Videomass mintájára) | tervezett |
| 5 | Csomagolás (lásd nyitott döntések) | tervezett |

## Nyitott döntések (tulajdonos)

- **Szállítás.** Az Ubuntu 26.04 saját FFmpeg-csomagja 8.x. A kiadott CFA4-csomag FFmpeg 9-cel
  fordul-e, és ha igen, hogyan érkezik: elkülönített, csak CFA4-es futtatókörnyezetként
  (`/opt/cfa4`, a CFA4 csomagtárolójából, a rendszer FFmpeg-jét nem cserélve), vagy a csomagba
  ágyazva. A kiadott csomag a 2026-10-24-i döntés szerint GStreamer nélkül fordul (ADR 0002 6. lépés).
- A Rust-kötés kiválasztása (1. lépés).

## Következmények

- Három médiaútvonal: FFmpeg 9, GStreamer, saját kodekek. A feladatonkénti sorrendet az
  útválasztási tábla rögzíti; a tényleges választást futásidejű felmérés (`probe()`) és mérés dönti el.
- A további alkalmazások (Audacity, Subtitle Edit, Krita, Resolve stb.) képességei a tábla egy-egy
  sorát használják, így beépítésükkor nem kell a háttereket újrarendezni.
- Az `AGENTS.md` és a `CLAUDE.md` „ffmpeg soha nem linkelhető és nem szállítható” szabálya ezzel
  az ADR-rel módosul: a termékben az FFmpeg 9 a `cfa4-ffmpeg` crate-en át, a fenti szabályok
  szerint használható.
- Fejlesztői függőség, ha a funkció be van kapcsolva: FFmpeg 9 fejlesztői könyvtárak
  (`libavformat`, `libavcodec`, `libavfilter`, `libswscale`, `libswresample`, `libavutil`).
