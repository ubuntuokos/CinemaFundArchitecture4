# ADR 0005 – Kiegészítő médiahátterek: FFmpeg 9 és a rendszer GStreamere, automatikus útválasztással

**Dátum:** 2026-10-25 · **Állapot:** elfogadva (a tulajdonos, `ubuntuokos`, 2026-10-25-i döntése) ·
**Kapcsolódik:** [ADR 0002](ADR-0002-gstreamer-backend.md) (GStreamer),
[ADR 0003](ADR-0003-aswf-and-conversion-sources.md) 3. pont (az FFmpeg helye),
[ADR 0004](ADR-0004-compute-routing.md) – feladatonkénti útvonalválasztó (`crates/route`, #55, #72)

## Döntés

1. **A saját Rust-motor az alap.** A konténerek, kodekek, a lejátszás, a render és az export
   elsődleges útja a CFA4 saját motorja (VISION.md 2.1, ADR F8). Ez nem változik.
2. **Automatikus útválasztó dönt.** Minden további médiaháttér az ADR 0004 útvonalválasztóján
   (`cfa4-route`) keresztül kerül sorra, háttérként regisztrálva. A hívó kód soha nem hív
   közvetlenül FFmpeg-et vagy GStreamert, csak a motor felületeit (`MediaSource`,
   `VideoEncoder`, szűrők). A választást a `routing.toml` táblája, a futásidejű felmérés és a mérés
   dönti el; a felhasználó felülbírálhatja (`~/.config/cfa4/routing.toml`).
3. **A kiegészítő hátterek alapsorrendje** a kodek-családokban: saját utak → **FFmpeg 9** →
   **GStreamer** → saját szoftveres kodekek (ahogy a `crates/route/routing.toml` rögzíti). Ahol egy
   feladatra a saját motor nem képes (például képernyő- és kamerafelvétel PipeWire-ből), ott a
   kiegészítő háttér kerül előre; ezt a tábla feladatsora rögzíti, mérés alapján.
4. **Újabb motorok is jöhetnek.** Az útválasztó nyitott: új motor új háttérként és új táblasorként
   kerül be, a hívó kód módosítása nélkül. Tervezett: a tulajdonos korábbi, a „H3-” kiváltására
   fejlesztett motorja, amely az MI-részek fejlesztésekor kerül be. Mindegyikre ugyanezek a
   szabályok vonatkoznak (licenc, nem crash-el, CPU-tartalék, mérés).

## FFmpeg

- **Minimális verzió: FFmpeg 9.0.2.** Régebbi FFmpeg-et a háttér nem fogad el; a `probe()` ezt
  futáskor ellenőrzi és jelenti.
- **Licenc:** csak GPL-3-mal kompatibilis build (`--enable-gpl --enable-version3`), **soha**
  `--enable-nonfree`, és nem kerülhet bele GPL-3-mal össze nem férő külső könyvtár (pl.
  `libfdk_aac`). FFmpeg-forráskódot nem másolunk a repóba.
- **Bekötés:** opcionális crate (munkanév: `cfa4-ffmpeg`, L5 réteg), a `crates/gstreamer` mintájára.
  A tulajdonos 2026-10-25-i döntése szerint az FFmpeg-háttér (rsmedia) a kiadott csomagokba is
  bekerül. Ha a kötés biztonságos API-t ad, a crate-ben nincs `unsafe`; ha saját FFI-réteg kell, az
  [ADR 0001](0001-platform-ffi.md) feltételei érvényesek.
- **Szabadalmilag korlátozott részek** (pl. x264, x265): az Ubuntu-modell szerint, külön, kérésre
  telepíthető csomagban (#57); az alapcsomag nélkülük is működik.
- **Szállítás (tulajdonosi döntés, 2026-10-25):** az Ubuntu 26.04 tárolóiban 8.x van, ezért a CFA4
  saját FFmpeg 9.0.2-buildet szállít, megosztott könyvtárakként **`/opt/cfa4` alá** telepítve (a
  rendszer FFmpeg-jét nem cseréli): GPL v3, `nonfree` nélkül, x264 és x265 nélkül (azok a #57
  szerinti külön csomagban), VA-API-val. Az rsmedia erre a buildre fordul.
- **A tesztorákulum külön marad:** a CI rögzített FFmpeg 9.0.2 CLI-je
  (`scripts/ci/install-ffmpeg-9.0.2.sh`) a saját kodekek független ellenőrzője, nem a termék része.

## GStreamer

- **Az Ubuntu által a telepítő tárolóiban terjesztett GStreamer** a háttér: a rendszer csomagjaiból,
  dinamikusan töltődik be (LGPL-2.1-or-later). **A kiadott csomag GStreamerrel fordul** (tulajdonosi
  döntés, 2026-10-25, a korábbi „GStreamer nélkül” helyett): az Ubuntu archívumából függőségként
  települ (`Depends` az alapkönyvtárakra, `Recommends` a bővítményekre), nem a csomagba ágyazva. A rendszer GStreamerét lecserélő tárolót nem
  használunk; ha egyszer újabb kellene, csak elkülönített, CFA4-es futtatókörnyezet jöhet szóba
  (ADR 0002, #51).
- A többi szabály az ADR 0002-ben marad (nincs `unsafe`, `ugly`/`libav` elemek nélkül, opcionális
  Cargo-funkció).

## Közös szabályok

1. **Egy útválasztó, egy felület** (fent, 2. pont).
2. **Mérés dönti el a sorrend változását** (VISION.md: bizonyíték, nem állítás).
3. **A saját dekóderek ellenőrként is maradnak:** a kiegészítő hátterek kimenetét a tesztek a
   saját, bitpontos dekóderekhez mérik.
4. **Mindig van tartalék:** ha egy kiegészítő háttér hiányzik vagy hibázik, a következő út jön
   (`Router::next_after`), végül a saját szoftveres kodekek.
5. **Nem crash-el** (AGENTS.md 0. pont): minden háttérhiba `Result`, sérült fájlra hibaüzenet jön.

## Lépések

| Lépés | Tartalom | Állapot |
|---|---|---|
| 1 | `cfa4-ffmpeg` crate: kötés, `probe()` (verzió ≥ 9.0.2, GPL-3-as build ellenőrzése, kodekek, gyorsítók), `discover()`; FFmpeg nélkül is fordul | tervezett |
| 2 | FFmpeg- és GStreamer-háttér regisztrálása a `cfa4-route`-ban (ADR 0004 1. fázisa után) | tervezett |
| 3 | Feladatsorok a táblában (pl. felvétel, konvertálás) mérés alapján | tervezett |
| 4 | Csomagolás: FFmpeg 9.0.2 `/opt/cfa4` alatt; GStreamer `Depends`/`Recommends` az Ubuntu archívumából (a kiadási munka a Qt6-átállás után jön) | eldöntve, tervezett |

## Számozás

Az ADR 0003 eredetileg az FFI-kivétel elvének szánta a 0004-es számot. A 0004 az útvonalválasztóé
(#55), a 0005 ezé az ADR-é; az FFI-elv ADR-je elkészültekor a következő szabad számot kapja
(várhatóan 0006). Az ADR 0003 hivatkozásai ennek megfelelően módosultak.
