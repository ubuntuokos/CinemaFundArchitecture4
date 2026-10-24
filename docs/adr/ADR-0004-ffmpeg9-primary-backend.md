# ADR 0004 – FFmpeg 9 az elsődleges, a GStreamer a másodlagos médiaháttér

**Dátum:** 2026-10-24 · **Állapot:** elfogadva (a tulajdonos, `ubuntuokos`, 2026-10-24-i döntése) ·
**Módosítja:** [ADR 0002](ADR-0002-gstreamer-backend.md) (a GStreamer szerepe),
[ADR 0003](ADR-0003-aswf-and-conversion-sources.md) 3. pont (az FFmpeg helye),
az `AGENTS.md` és a `CLAUDE.md` „ffmpeg csak tesztorákulum” szabálya

## Döntés

A CFA4 médiaháttereinek sorrendje:

| Sorrend | Háttér | Szerep |
|---|---|---|
| 1. | **FFmpeg 9** (libavformat, libavcodec, libavfilter, libswscale, libswresample) | **elsődleges** médiaháttér: konténerek olvasása és írása, dekódolás, kódolás, szűrők, hardveres gyorsítás |
| 2. | **GStreamer** (`crates/gstreamer`, ADR 0002) | **másodlagos** médiaháttér: amit az FFmpeg 9 nem tud, vagy ahol mérés szerint jobb (pl. PipeWire-felvétel, Qt6-előnézet) |
| 3. | **Saját Rust-kodekek és konténerek** (`crates/h264`, `hevc`, `isobmff` …) | **tartalék**: mindig elérhető, ha az FFmpeg 9 és a GStreamer hiányzik vagy hibázik; továbbá a kimenetek bitpontos ellenőrzése |

Ami **nem** változik:

- A **Rust-motor** marad a projekt, az idővonal, a lejátszás vezérlése, a render, az export
  vezérlése és a parancsrendszer gazdája ([ADR F8](ADR-F8-QT6-RUST-ENGINE-2026-10-23.md)). A
  Qt6-felület továbbra is ezzel beszél. Az FFmpeg 9 és a GStreamer a motor `MediaSource` és
  `VideoEncoder` felületei mögé kerül, nem helyette.
- A natív VA-API és NVDEC/NVENC útvonal (`crates/platform`, [ADR 0001](0001-platform-ffi.md))
  megmarad; hogy egy feladatot ez vagy az FFmpeg 9 lát el, mérés dönti el.
- A **gyártófüggetlenség** (AGENTS.md): egy funkció sem lehet csak NVIDIA-n elérhető.
- A FilmCraft-projektek (`.fcproj`) továbbra is megnyithatók.

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
4. **Tartaléklánc.** Ha az FFmpeg 9 nincs telepítve, egy formátumot nem ismer, vagy hibázik, a
   fájlt a GStreamer kapja; ha az is elutasítja, a saját Rust-kodekek. Egy addig működő fájl a
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
| 1 | `cfa4-ffmpeg` crate: kötés kiválasztása, `probe()` (verzió, buildopciók, licenc: GPL-3-as build ellenőrzése, kodekek, hardveres gyorsítók), `discover()`; tesztek FFmpeg nélkül is fordulnak | tervezett |
| 2 | Importálás: az FFmpeg 9 lesz az első megnyitó a `filmcraft_media` regisztrációjában, utána a GStreamer, végül a beépített megnyitók | tervezett |
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

- Három médiaútvonal: FFmpeg 9, GStreamer, saját kodekek. A sorrend és a tartaléklánc rögzített,
  a választást futásidejű felmérés (`probe()`) és mérés dönti el.
- Az `AGENTS.md` és a `CLAUDE.md` „ffmpeg soha nem linkelhető és nem szállítható” szabálya ezzel
  az ADR-rel módosul: a termékben az FFmpeg 9 a `cfa4-ffmpeg` crate-en át, a fenti szabályok
  szerint használható.
- Fejlesztői függőség, ha a funkció be van kapcsolva: FFmpeg 9 fejlesztői könyvtárak
  (`libavformat`, `libavcodec`, `libavfilter`, `libswscale`, `libswresample`, `libavutil`).
