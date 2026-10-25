# ADR 0004 – Kdenlive-képességek beépítése a CFA4-be

**Dátum:** 2026-10-25 · **Állapot:** JÓVÁHAGYVA (a tulajdonos, `ubuntuokos`, 2026-10-25-én minden
nyitott kérdést eldöntött, lásd 6. pont) · **Élő változat:** a projekt Claude-dokumentuma
(„Kdenlive képességek beépítési terve a CFA4-be”); ez a fájl a `main`-en őrzött, végleges másolat
(VISION.md 2.6).

## 1. Döntés röviden

A Kdenlive-ből a **működési mintákat** vesszük át, nem a C++ kódot és nem az MLT-motort. A CFA4
Rust motorja marad a projekt, az idővonal és a render egyetlen igazsága (ADR F8). A Kdenlive ott
ad a legtöbbet, ahol a CFA4 most gyenge: Qt-felület, külön folyamatban futó renderelés, linuxos
munkafolyamatok, helyi MI-szkriptek kezelése.

14 képesség, becsült ráfordítás kb. **435–745 ügynökóra** (ebből a P1 kb. 160–270). A becslés a
repó saját módszerét követi, mérés nélkül.

## 2. Források és licencek

| Repó | Licenc | Állapot | Használható elv |
|---|---|---|---|
| kdenlive (26.11 fejlesztői ág) | GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL (néhány BSD/MIT/LGPL) | korszerű | Qt-architektúra, `kdenlive_render`, `TaskManager`, proxy, MI-szkriptek, OTIO |
| kdenlive-export-profiles | CC0-1.0 | korszerű | 72 platformprofil |
| kdenlive_ease_generator | GPL-3.0 | elavult | Penner-féle easingek |
| gyroflow-frei0r | MIT OR Apache-2.0 (a `gyroflow-core` GPL-3.0) | részben | giroszkópos stabilizálás Rust API-n |
| kdenlive-profiles | GPL-2.0, „or later” nélkül | elavult | csak tartalmi minta |
| kdenlive-clipper | MIT | elavult | kivágás jelölők szerint |
| kdenlive-network-render | GPL-3.0 | elavult | render több gépen |
| kdenlive-multirender | MIT | elavult | darabolt párhuzamos render |
| kdenlive-post-render-script | GPL-3.0-or-later | elavult | export utáni metaadat, borítókép |
| kdenlive-project-analyzer | GPL-3.0 | elavult | projektfájl-elemzés |
| kdenlive-tools | BSD | elavult | projektfájl parancssori javítása |

Átvett Kdenlive-fájl megtartja a GPL-3.0-only fejlécét, és bekerül a `docs/licensing.md` §5
nyilvántartásába. Az ilyen fájlokat tartalmazó terjesztett egész gyakorlatilag GPL-3.0 alatt megy.

## 3. Képességek

| # | Képesség | Prioritás | Ráfordítás (óra) | Előfeltétel |
|---|---|---|---|---|
| K1 | Qt6-felület Kdenlive-mintára (modell/nézet, QML-idővonal átvétele) | P1 | 40–70 | ADR F8 2. lépés |
| K2 | Külön folyamatban futó render és feladatkezelő (`cfa4-cli render`) | P1 | 25–45 | megvan |
| K3 | Platform-előbeállítások a CC0 profilokból; hardveres presetek | P1 | 15–25 | natív VA-API-kódoló (G5) |
| K4 | Kamerás proxyk felismerése (GoPro LRV, DJI LRF, Sony, Insta360) | P1 | 8–15 | megvan |
| K5 | **Kdenlive-projektek olvasása (kötelező)** | P1 | 35–60 | OTIO-olvasó megvan |
| K6 | Giroszkópos stabilizálás `gyroflow-core`-ral | P1 | 35–55 | függőség-audit |
| K7 | Párhuzamos és hálózati render, `cfa4-render-node` | P2 | 70–120 | K2 |
| K8 | Helyi MI: modellkezelő, SAM2 objektum-maszk, fordítás | P2 | 60–110 | K14, G19 |
| K9 | Export utáni lépések, fejezetek, jelölőnkénti export | P2 | 12–20 | K2 |
| K10 | Egyedi klipfeladatok, kötegelt átkódolás | P2 | 25–40 | K2 |
| K13 | Projektnyilvántartás SQLite-tal (sorozatok, sablonok, presetek) | P2 | 30–50 | ADR F8 3. lépés |
| K14 | Online források és letöltéskezelő (Hugging Face, Freesound, Pixabay) | P2 | 25–45 | – |
| K11 | További easing-típusok | P3 | 8–15 | – |
| K12 | frei0r- és LV2-host külön folyamatban, deklaratív effektleírás | P3 | 50–80 | külön ADR (G11) |

### Képességenként

- **K1.** PySide6-modellek a Rust motor `project.inspect` / `sequence.inspect` válaszaiból,
  parancsazonosítókkal írnak vissza. A Kdenlive QML-idővonala (`src/timeline2/view/qml`)
  átvehető. Kész, ha egy 3 sávos, 50 klipes szekvencia szerkeszthető és visszavonható Waylanden és
  X11-en.
- **K2.** A Qt6-héj `QProcess`-szel indítja a `cfa4-cli render`-t, az előrehaladás JSON-sorokban
  jön. Kész, ha a Qt-héj leállítása után a render befejeződik, és a megszakítás nem hagy félkész fájlt.
- **K3.** A `kdenlive-export-profiles` tartalma (felbontás, bitráta, GOP, hang) a
  `crates/export/src/presets.rs` formátumára átírva, forrásmegjelöléssel. Hardveres presetek a
  kiadott csomagban is, GStreamer nélkül: NVIDIA-n NVENC, Intelen és AMD-n natív VA-API
  (`crates/platform`, #39 `VaApiEncode`). A gyártói meghajtót a felhasználó telepíti; ha hiányzik,
  a CFA4 megmutatja az Ubuntu-csomagot (`apt`, NVIDIA-n „További illesztőprogramok”) és a gyártó
  telepítési oldalának linkjét. Semmit nem tölt le magától; a linkek adatfájlban vannak, kiadás
  előtt ellenőrizzük őket. Addig a szoftveres kódoló a tartalék.
- **K4.** `externalproxies.rc`-mintára adatfájlban leírt névpárok; a meglévő `ProxySource`-ként
  csatolva, újrakódolás nélkül.
- **K5.** 1) A Kdenlive OTIO-exportja a meglévő `otio.rs`-sel, valódi fájlokon tesztelve.
  2) Saját `.kdenlive` (MLT XML) olvasó a `crates/interchange`-ben, MLT-könyvtár nélkül: vágások,
  sávok, jelölők, feliratok, alap effektek; minden elem „támogatott / átalakított / nem támogatott”
  osztályba sorolva, import után kompatibilitási jelentéssel. Kész, ha 5 valódi Kdenlive 24–26-os
  projekt képkockára egyezik a `melt`-renderrel, és hibás XML-re nincs pánik (fuzz-teszt).
  Kdenlive-be exportálás nem lesz.
- **K6.** `gyroflow-core` opcionális Cargo-funkcióként, frei0r-felület nélkül, `catch_unwind`
  alatt. Kész, ha GoPro-mintán mérhetően jobb a 2D Warp Stabilizernél.
- **K7.** GOP-határon darabolt render, a saját muxer újrakódolás nélkül fűz. Több gép és gépenként
  több GPU; a vezérlő a csomópontok mért sebessége szerint oszt, kieső csomópont szakaszát újra
  kiosztja. `cfa4-render-node`: szerkesztőfelület nélküli renderelő kliens gyenge irodai gépre,
  alacsony prioritással vagy tétlen időben, a gép használója szüneteltetheti. Helyi hálózat,
  párosítás, titkosított kapcsolat; média közös tárolóról vagy ujjlenyomattal ellenőrzött
  másolással; azonos CFA4-verzió. Kész, ha két gépen (egyik csak renderelő kliens) ugyanazt a
  fájlt adja, mint az egyben renderelés, és egy csomópont kihúzása után is befejeződik.
- **K8.** Egységes modellkezelő (licenc és forrás rögzítve, letöltés csak engedéllyel); SAM2
  objektum-maszk külön folyamatban (G12). A csomagba egy nyitott, kereskedelmi használatot is
  engedő fordítómodell kerül (jelölt: Opus-MT/Marian magyar–angol, licencellenőrzés után); a
  felhasználó opcionálisan cserélheti, nem kereskedelmi licencnél figyelmeztetést kap.
  SeamlessM4T és NLLB nem használható (CC-BY-NC).
- **K9.** Borítókép, MP4-fejezetek, YouTube-fejezetszöveg, export jelölőnként, a felhasználó által
  megadott export utáni parancs.
- **K10.** Kötegelt átkódolás a K2 feladatkezelőjén, a saját motorral (később GStreamerrel).
- **K11.** Penner-függvények új `Interpolation` változatként vagy Bezier-kulcskockákká sütve.
- **K12.** A G11 szerint, külön ADR-rel, külön folyamatban; frei0r 8 bites RGBA, csak CPU.
- **K13.** Helyi SQLite-adatbázis (`rusqlite`) a felhasználói adatkönyvtárban: projektek,
  sorozatok és epizódok, sorozatonkénti felirat- és címsablonok, presetek, proxy- és cache-fájlok,
  letöltött modellek. A projekt igazsága a `.fcproj` marad; az adatbázis a projektfájlokból
  újraépíthető.
- **K14.** Közös letöltéskezelő modellekhez és médiához, adatfájlban leírt szolgáltatókkal.
  Letöltés előtt látszik a forrás, a méret és a licenc; csak jóváhagyással indul; hash-ellenőrzés,
  folytatható letöltés, bejegyzés a K13-ba. Hálózat nélkül minden más működik.

## 4. Ütemezés

| Fázis | Kapcsolódás | Tartalom | Kapu a következőhöz |
|---|---|---|---|
| A – Alapok, most | ADR F8 2–3. lépés mellett | K2, K4, K3 (meglévő formátumok), K5 OTIO-teszt, K11 | a render túléli a Qt-héj leállását |
| B – Idővonal és kép | ADR F8 4–5. lépés | K1, K5 saját olvasó, K6, K13 | a Qt6-idővonal szerkeszt és visszavon Waylanden és X11-en |
| C – Export és konvertálás | ADR F8 6. lépés | K7, K9, K10, K3 hardveres presetek | Whisper a kiadásban (G19), bővítményhost-ADR (G11) |
| D – MI és bővítmények | VISION 6. pont 4–5. | K14, K8, K12 | – |

## 5. Nem vesszük át

MLT renderelőmotorként (két igazság, képkocka-alapú idő, `unsafe` C-kód, FFmpeg alatta) · Kdenlive-
be exportálás · nem kereskedelmi licencű modellek a csomagban · CUDA-only MI-utak · KDE Frameworks-
függő részek · VOSK (a Whisper megvan) · a `kdenlive-profiles` szövege · FFmpeg-alapú segédszkriptek.

## 6. Tulajdonosi döntések (2026-10-25)

| Téma | Döntés |
|---|---|
| Kdenlive-projektek olvasása | kötelező (K5) |
| Kdenlive-be exportálás | nem lesz, csak veszteséggel menne |
| FFmpeg és MLT (`melt`) | csak teszt-orákulum; az FFmpeg 9 + MLT 7 motorjavaslat elvetve. Az FFmpeg-szabály (AGENTS.md §2, ADR 0003 §3) változatlan |
| Kdenlive QML-idővonal | átvehető (K1) |
| `gyroflow-core` | kell (K6) |
| Hálózati render | több gép, több GPU; `cfa4-render-node` (K7) |
| Fordítómodell | egy nyitott, kereskedelmileg is használható modell a csomagban; csere opcionális (K8) |
| Hardveres kódolás | NVENC, illetve natív VA-API Intelen és AMD-n, GStreamer nélkül is; meghajtó-telepítési link (K3). Ez a kiadott csomagra pontosítja az ADR 0003 §5-öt, amely az AMD/Intel kódolást GStreamer `va*enc` elemekre bízta |
| SQLite-nyilvántartás | kell (K13) |
| Online források | fontos (K14) |

## 7. Viszony a korábbi tervekhez

A 2026-10-17-i (közös Qt/C++ alkalmazás Rust–C ABI-val, Qt/MLT hibrid, Kdenlive Rust-migráció) és
a 2026-10-19-i (upstream egui-szerkesztő + FFmpeg/MLT-adapterek) javaslatból a Rust-központú „A” irány és a
Kdenlive-kompatibilitási réteg (kompatibilitási jelentés, effekt-osztályozás) él tovább a K5-ben;
az SQLite-nyilvántartás ötlete a K13-ban. A többit a 2026-10-23/24-i döntések (Qt6 PySide6-tal,
GStreamer, GPL-3.0-or-later, FFmpeg csak orákulum) és a fenti döntések írták felül.
