# ADR 0005 – Kiegészítő médiahátterek: FFmpeg 9 és a GStreamer (alapból a rendszeré), automatikus útválasztással

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
3. **A kiegészítő hátterek alapsorrendje** a kodek-családokban: saját utak (hardveres és
   szoftveres) → **FFmpeg 9** → **GStreamer** (ahogy a `crates/route/routing.toml` rögzíti;
   tulajdonosi döntés, 2026-10-27). A saját motor minden fájlt elsőként nyit meg; a kiegészítő
   hátterek csak azt kapják meg, amit a motor nem tud megnyitni vagy dekódolni. Az útválasztó a
   `media.fallback` feladaton dönti el a sorrendjüket és azt, melyik léphet működésbe (Beállítások ▸
   Médiahátterek kapcsolói, a felhasználói tábla), és a láncot a `perf.stats` és a `media.backends`
   jelenti, minden kihagyott háttérrel és okkal. Ahol egy
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
  használunk.
- **Elkülönített futtatókörnyezet csak szükség esetén** (#51, ADR 0002 „GStreamer-verzió és
  frissítés”): ha egy funkcióhoz újabb GStreamer kell, mint amit az Ubuntu ad, a CFA4 saját
  `cfa4-gstreamer-runtime` csomagja (`/opt/cfa4/gstreamer`) csak arra a funkcióra lép be; tartalék a
  rendszer GStreamere, majd a saját motor. Szabadalmilag korlátozott elemet nem tartalmaz (azok az
  Ubuntu-modell szerint az Ubuntutól jönnek), biztonsági frissítéséért a CFA4 felel, és minden
  CFA4-kiadással frissül. A rendszer GStreamere alap marad, sosem cseréljük le.
- A többi szabály az ADR 0002-ben marad (nincs `unsafe`, `ugly`/`libav` elemek nélkül, opcionális
  Cargo-funkció).

## Közös szabályok

1. **Egy útválasztó, egy felület** (fent, 2. pont).
2. **Mérés dönti el a sorrend változását** (VISION.md: bizonyíték, nem állítás).
3. **A saját dekóderek ellenőrként is maradnak:** a kiegészítő hátterek kimenetét a tesztek a
   saját, bitpontos dekóderekhez mérik.
4. **Mindig van tartalék:** ha egy kiegészítő háttér hiányzik vagy hibázik, a következő út jön
   (`Router::next_after`); a saját szoftveres kodekek a kiegészítő hátterek előtt állnak, ezért ha
   egy kiegészítő háttér hibázik, a motor eredeti hibaüzenete marad.
5. **Nem crash-el** (AGENTS.md 0. pont): minden háttérhiba `Result`, sérült fájlra hibaüzenet jön.

## Lépések

| Lépés | Tartalom | Állapot |
|---|---|---|
| 1 | `cfa4-ffmpeg` crate: kötés (vendorozott rsmedia → rsmpeg → rusty_ffmpeg, `vendor/ffmpeg-bindings/`), `probe()` (verzió ≥ 9.0.2, GPL-3-as build, nonfree tiltva, korlátozott kodekek), `discover()`, `open()` natív bitmélységgel, első tartalék a GStreamer előtt; FFmpeg nélkül is fordul | kész (a buildpróba 2026-10-26: bitpontos a G3- és G10-esetekre) |
| 1a | Beállítások ▸ Média és lejátszás ▸ Médiahátterek (kapcsolók, állapot, korlátozott kodekek), `media.backends`, `cfa4-cli media-backends`; menü: Szerkesztés ▸ Médiahátterek…, Súgó ▸ Korlátozott kodekek… | kész |
| 2 | FFmpeg- és GStreamer-háttér regisztrálása a `cfa4-route`-ban (ADR 0004 1. fázisa után): a `media.fallback` feladat lánca (`crates/engine/src/media_route.rs`) adja a `cfa4_media::set_fallback_order` sorrendjét; a felhasználói `[compute.task."media.fallback"]` sor felcserélheti vagy kihagyhatja őket | kész (2026-10-27) |
| 3 | Feladatsorok a táblában (pl. felvétel, konvertálás) mérés alapján | tervezett |
| 4 | Csomagolás: FFmpeg 9.0.2 elkülönítve `/opt/cfa4/ffmpeg-9.0.2` alatt (`scripts/dev/build-ffmpeg-9.0.2.sh`); a `cfa4-restricted-extras` ugyanazt az FFmpeg-et x264-gyel és x265-tel `/opt/cfa4/ffmpeg-9.0.2-restricted` alá teszi, és a programok futásidejű keresési útja ezt nézi először (`apps/*/build.rs`); GStreamer `Depends`/`Recommends` az Ubuntu archívumából | FFmpeg-build kész; `.deb`-csomagok a Qt6-átállás után |

## Számozás

Az ADR 0003 eredetileg az FFI-kivétel elvének szánta a 0004-es számot. A 0004 az útvonalválasztóé
(#55), a 0005 ezé az ADR-é; az FFI-elv ADR-je elkészültekor a következő szabad számot kapja
(várhatóan 0006). Az ADR 0003 hivatkozásai ennek megfelelően módosultak.
