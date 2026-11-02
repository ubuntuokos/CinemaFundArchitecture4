# Kdenlive-paritás: mit tud a Kdenlive, és mit tud már a CFA4

> **Állapot:** a tulajdonos 2026-10-25-én jóváhagyta (K1–K35); a 2026-10-27-i újraelemzést is jóváhagyta. · **Utolsó átnézés:** 2026-10-27 · **Változás:** major (3. változat: a kiegészítő Kdenlive-repók
> elemzése, K10–K19 új teendők, a tulajdonos 2026-10-25-i döntései, 12. pont; K20–K25 az FA3 elődtervből, 11.1; K26–K30 felirat- és átiratlánc, 11.2; K31–K35 Whisper, 11.3; MI-döntések, 12.1; átfedések és szállítás, 11.4) · **Cél:** Kdenlive 26.08.2 (2026-10-13) · **CFA4:** `main` @ `bbbefc3`

Ez a lista a Kdenlive képességeit veti össze a CFA4 jelenlegi állapotával, és rangsorolja a
hiányokat. A `docs/gaps.md` a Premiere-hez mér; ha egy hiány ott is szerepel, a G-számra
hivatkozik. A végrehajtási szakaszok az [ADR F8](adr/ADR-F8-QT6-RUST-ENGINE-2026-10-23.md)
(Qt6) és az [ADR 0003](adr/ADR-0003-aswf-and-conversion-sources.md) (források) lépéseihez kötődnek.

## 0. Licenc: mit lehet átvenni a Kdenlive-ból

A CFA4 licence **GPL-3.0-or-later** (#37, [`docs/licensing.md`](licensing.md)). A Kdenlive
GPL-licencű, ezért **a kódja átvehető**, de csak fájlonként ellenőrizve:

- **Átvehető:** GPL-3.0, GPL-2.0-or-later, LGPL fejlécű fájl. A KDE-ben gyakori
  „`GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL`” fejléc a GPL-3.0 opcióval
  átvehető.
- **Nem vehető át:** a csak GPL-2.0-only fejlécű fájl. Ennek a képességét saját kóddal kell
  megírni.
- **Kötelező minden átvételnél:** az eredeti szerzői jogi fejléc megtartása, és egy sor a
  [`docs/licensing.md` §5](licensing.md) nyilvántartásban, ugyanabban a commitban.
- **Clean-room marad** a kodekekre és konténerekre (AGENTS.md §2). Ez a Kdenlive-ot nem érinti,
  mert a Kdenlive nem tartalmaz saját kodeket.
- **Márkák:** a Kdenlive és a KDE neve, logója nem kerülhet a CFA4-be, ahogy az örökölt upstream
  márkák is kikerültek (#41).

### Mit érdemes valóban átvenni

A Kdenlive kódja nem illeszthető be változtatás nélkül: C++/Qt, és az MLT-keretrendszerre épül,
a CFA4 viszont Rust-motor + PySide6-felület. A haszna ezért részenként eltér:

| Kdenlive-rész | Haszna a CFA4-ben | Hogyan |
|---|---|---|
| Qt Designer `.ui` fájlok (párbeszédablakok, beállítások) | **nagy** | A PySide6 közvetlenül betölti vagy Pythonná alakítja (`pyside6-uic`). Fájlonként licencellenőrzés |
| `.kdenlive` / MLT XML olvasó logika | **nagy** (K1) | A logika és a mezőleképezés átírható Rustra (`crates/interchange`). Az MLT maga LGPL-2.1-or-later |
| Algoritmusok (pl. hullámforma, audio-szinkron korrelációval, jelenetfelismerés) | közepes | Átírás Rustra, forrásmegjelöléssel |
| Feliratkezelés, Speech Editor logikája | közepes | Python- vagy Rust-átírás |
| Titler sablonok, render-profilok, luma-fájlok | közepes | Tartalom, nem kód. A `REUSE.toml` szerint CC0-1.0: `data/effects`, `data/transitions`, `data/generators` XML, átmenet-előnézetek, `data/lumas/PAL` és `HD`, `data/luts`, `data/layouts` (K20, K21); minden másnál fájlonként ellenőrizendő |
| Idővonal-felület (QML) | kicsi | A CFA4 Qt Widgetset használ, nem QML-t |

## Jelmagyarázat

| Jel | Jelentés |
|-----|----------|
| ✅ | Megvan a Rust-motorban (parancs-azonosítóval igazolva), és az örökölt egui-felületen |
| 🟡 | Részben van, vagy másképp működik |
| ❌ | Hiányzik |
| ❓ | Nincs ellenőrizve, kipróbálandó |

**Fontos:** a ✅ a Rust-motorra és az egui-felületre vonatkozik. Az egui be van fagyasztva, minden
új felületi munka a **Qt6-felületen** készül. A Qt6 pontról pontra követett állapota a
[`qt-parity.md`](qt-parity.md)-ben van (2026-10-27: 40 pont ✅, 19 🟡, 9 ❌; ADR F8 2–6. lépés
folyamatban, 7. nyitott).

## 1. Projekt, fájlok, médiakezelés

| Kdenlive-képesség | CFA4 | Megjegyzés, bizonyíték |
|---|:-:|---|
| `.kdenlive` projekt megnyitása (MLT XML) | ❌ | **K1**, lásd lent |
| OpenTimelineIO import / export | ✅ | `file.exportOtio`, `crates/interchange/src/otio.rs`; orákulum-teszt: #44 |
| Proxy klipek | ✅ | `media.createProxies`, `media.toggleProxies`, `media.proxyPresets` |
| Projekt archiválása mappába | ✅ | `file.projectManager` (collect / consolidate) |
| Tömörített archív (zip) | ❓ | |
| Automatikus mentés, helyreállítás | ✅ | `autoSave.*`, `file.recover` |
| Klip szerkesztése külső programban (pl. Gyroflow) | 🟡 | `edit.editOriginal`; a visszatöltés nincs külön kezelve |
| Fájlok megnyitása fájlkezelőből | ✅ | XDG-asztali fájl és MIME-társítás (F6-audit) |
| Képsorozat, diavetítés, WebP | ✅ | `file.importImageSequence` |
| Beágyazott feliratfolyamok felismerése | 🟡 | `media.includeCaptionsOnImport` |
| Figyelmeztetés távoli erőforrásra | ❓ | |
| Renderelt fájl a Project Binbe | ❓ | |

## 2. Idővonal és vágás

| Kdenlive-képesség | CFA4 | Megjegyzés, bizonyíték |
|---|:-:|---|
| Korlátlan sáv, több szekvencia, beágyazás | ✅ | `sequence.addTracks`, `file.newSequence`, `clip.nest` |
| Kijelölés másolása új szekvenciába | ✅ | `sequence.makeSubsequence` |
| Hangsáv automatikus létrehozása | ✅ | `timeline.addTracksAutomatically` |
| Sávok átrendezése | ❓ | Nem találtam sávmozgató parancsot |
| Marker a klipek közti résekbe | ❓ | Résre ugrás van (`sequence.goToNextGap`) |
| Lejátszófej illesztése | ✅ | `timeline.snapPlayhead` |
| Ripple, roll, slip, slide, razor, csoportok | ✅ | `tool.*`, `trim.*`, `clip.group` |
| Áttűnés vágásponton („mix”) | ✅ | `sequence.applyVideoTransition` |
| Áttűnés automatikus hossza, animált előnézet | ❓ | |
| Sebesség, visszafelé, time remap, kimerevítés | ✅ | `clip.speedDuration`, `clip.frameHold` |
| Több klip sebessége egyszerre | ❓ | |
| Multicam, szinkron időkód alapján | ✅ | `clip.createMulticam`, `multicam.*`, `clip.synchronize` |
| Rögzített lejátszófej, középső egérgombos görgetés | ❓ | |
| Klipszínek típusonként | ✅ | `labels.defaults.*` |
| Undo-előzmények törlése | ❓ | `history.list` van |

## 3. Effektek, színkezelés, maszkok

| Kdenlive-képesség | CFA4 | Megjegyzés, bizonyíték |
|---|:-:|---|
| Saját effektkönyvtár | ✅ | 90+ videóeffekt, ~30 áttűnés, saját DSP |
| frei0r-effektek | ❌ | Mostantól licencben megengedett (GPL-2.0-or-later). ADR 0003 A7, **külön folyamatban** (ADR 0003 9. pont) |
| LADSPA / LV2 hangeffektek | ❌ | lilv-alapú LV2-host külön folyamatban, ADR 0003 A6 |
| Chroma key, LUT, görbék, színkerekek | ✅ | Ultra Key, `lut.import`, Lumetri |
| Maszkok, kulcskockázható alfa-maszk | ✅ | `masks.*` |
| Mozgáskövetés | 🟡 | `masks.track` maszkot követ; pontkövetés effektparaméterre a Natron openfx-misc-ből jöhet (ADR 0003 A4) |
| Képstabilizálás | 🟡 | Warp Stabilizer, csak 2-D (G18) |
| AI háttéreltávolítás (SAM2) | ❌ | **K4**. Helyben, gyártófüggetlenül (CPU, bármely GPU, NPU) |
| Transform forgatási középpont | ✅ | `motion.anchor` |
| Matematikai kifejezés a számmezőkben | ❌ | **K7**. Qt6-ban egy közös spinbox-osztállyal megoldható |
| Rajzolás a monitoron (Drawbox, Delogo) | 🟡 | Maszkok, alakzatok vannak; külön Delogo ❓ |
| Heatmap, Color Enhance (frei0r) | ❌ | frei0r-hosttal együtt (fent) |
| Effektek kikapcsolása az egész idővonalon | ❓ | Klipenként van (`effects.toggleEnabled`) |

## 4. Feliratok, beszéd, szöveg

| Kdenlive-képesség | CFA4 | Megjegyzés, bizonyíték |
|---|:-:|---|
| Feliratszerkesztő, több sáv, SRT, VTT | ✅ | `captions.*`, `crates/captions` |
| ASS / SSA feliratok | ❌ | **K9**. Kdenlive- vagy Subtitle Edit-kódból is átvehető, fájlonként ellenőrizve |
| Beszédfelismerés (Whisper) | 🟡 | Megvan (`transcript.generate`), a kiadott buildekben ki van kapcsolva (G19). **K2** |
| Szöveg alapú vágás | ✅ | `transcript.*`, `textPanel.*` |
| Felirat fordítása | ❌ | **K6**. A Kdenlive által használt SeamlessM4T nem kereskedelmi licencű; szállítható modell kell |
| Címszerkesztő | ✅ | `graphics.*` |
| Címszerkesztő-illesztés, lekerekített téglalap | 🟡 | `view.snapInProgramMonitor`; a többi ❓ |
| Görgetett stáblista | ✅ | `crates/credits`, táblázatból is (F10, #36) |
| (CFA4-többlet) Szövegfelolvasás | ✅ | `tts.*`, Kokoro-82M |

## 5. Hang

| Kdenlive-képesség | CFA4 | Megjegyzés, bizonyíték |
|---|:-:|---|
| Hangkeverő, szóló, némítás | ✅ | `mixer.*` |
| Szóló hozzáadása Shift+kattintással | ❓ | |
| Hangfelvétel (voice-over) | ✅ | `audio.voiceover.*` |
| Normalizálás, EQ, zengetés, zajcsökkentés, ducking | ✅ | Saját DSP, `repair.*`, `essentialSound.generateDucking` |
| Időnyújtás és hangmagasság jobb minőségben | 🟡 | Saját pitch shift; Rubber Band: ADR 0003 A6 |
| Hanghullám szekvenciákon | ❓ | Klipeken van |

## 6. Monitorok, mérőeszközök

| Kdenlive-képesség | CFA4 | Megjegyzés, bizonyíték |
|---|:-:|---|
| Waveform, vektorszkóp, hisztogram, RGB parade | ✅ | `crates/scopes` |
| Hangspektrum, spektrogram panel | 🟡 | Spektrumrajz a hangeffekt-szerkesztőben; külön panel nincs. **K8** |
| Teljes képernyős monitor | ✅ | `window.toggleFullScreen` |
| Monitor tükrözése második kijelzőre | ❌ | `multicam.transmitView` van, eszköz nélkül |

## 7. Renderelés, export

| Kdenlive-képesség | CFA4 | Megjegyzés, bizonyíték |
|---|:-:|---|
| Render-sablonok, render-sor, zóna-render, előnézet-render | ✅ | `export.presets.*`, `export.queue.*`, `sequence.renderInToOut` |
| Szüneteltetés és folytatás | 🟡 | Megszakítás van; szüneteltetés ❓ |
| Alfa export | ✅ | ProRes 4444, PNG-sorozat |
| Hardveres kódolás AMD-n és Intelen (VA-API) | 🟡 | Az NVENC export és a Linux HW-dekódolás már a `cfa4-route`-on át megy (ADR 0004 1. fázis, részben kész). VA-API-kódolás a saját motorban vagy kiegészítő háttérből (FFmpeg `*_vaapi`, GStreamer `va*enc`); a kiadott csomagok GStreamerrel épülnek (ADR 0002, ADR 0005); gyártófüggetlen terv (#39) |
| Szoftveres HEVC, jobb H.264, MP3 export | ❌ | x265, x264, LAME: ADR 0003 A5, előtte szabadalmi vizsgálat |
| Széles formátumlefedettség | 🟡 | A saját Rust-motor az alap; kiegészítő hátterek: FFmpeg ≥ 9.0.2 (tervezett `cfa4-ffmpeg`) és a GStreamer (alapból az Ubuntué, újabb verzióhoz elkülönített CFA4-futtatókörnyezet; `crates/gstreamer`, #40), köztük az automatikus útválasztó (`cfa4-route`) dönt (ADR 0005, elfogadva). Az FFmpeg ≥ 9.0.2 (csak GPL-3-as build) a `/opt/cfa4`-ben szállítva; a `cfa4-ffmpeg` háttér a #249-ben, a sorrendező a #263-ban készül |

## 8. Platform és felület

| Kdenlive-képesség | CFA4 | Megjegyzés, bizonyíték |
|---|:-:|---|
| Linux, deb, AppImage, Flatpak | 🟡 | Csak Ubuntu 26.04 AMD64; mind a négy formátum tervezett, a kiadás HOLD (F9) |
| Qt-felület | 🟡 | Qt6 Widgets + PySide6; az egui befagyasztva, a pontos paritásig marad (#48); állapot: [`qt-parity.md`](qt-parity.md) |
| Wayland és X11 | ❓ | VISION.md 3. pont: minden 26.04-es változaton tesztelendő |
| Munkaterületek, billentyűparancsok, témák | ✅ | Motorban és egui-ban; Qt6-ban 🟡 (dokkolás kész, billentyűparancs-szerkesztő a #214-ben) |

## 9. Teendők rangsora

A sorrend a VISION.md 6. pontját követi: a Qt6-felület az első, ezért minden felületi munka
közvetlenül Qt6-ban készül, nem egui-ban.

| # | Feladat | Kapcsolódó lépés | Átvehető forrás |
|---|---|---|---|
| **K2** | Whisper bekapcsolása a kiadott buildekben | az MI-réteg üzembe helyezése után (G19; 12.1) | — (kész kód) |
| **K1** | `.kdenlive` (MLT XML) projektimport | motor most; Qt6-menü az ADR F8 3. lépésénél | Kdenlive és MLT olvasó logikája, fájlonként |
| **K3** | Linux-stabilitás, Wayland és X11 minden változaton | ADR F8 5–7. lépés, VISION.md 3. pont | — |
| **K7** | Matematikai kifejezés a számmezőkben | ADR F8 4. lépés (közös Qt6-spinbox) | Kdenlive spinbox-logikája, ha a fejléce engedi |
| **K8** | Hangspektrum- és spektrogram-panel | ADR F8 5. lépés | Kdenlive vagy Audacity-kód, fájlonként |
| **K9** | ASS / SSA felirat | bármikor (`crates/captions`) | Subtitle Edit (GPL-3.0) vagy Kdenlive |
| **K4** | AI háttéreltávolítás (SAM2-szerű) | az MI-réteg üzembe helyezése után | SAM2 (Apache-2.0), helyben, nem csak NVIDIA-n |
| **K5** | frei0r és LV2 effektek külön folyamatban | ADR 0003 A6–A7 | frei0r, lilv |
| **K6** | Feliratfordítás szállítható licencű modellel | az MI-réteg üzembe helyezése után | modellválasztás kell (licenc és helyi futás) |

### K1 részletesen: `.kdenlive` import

- **Formátum:** MLT XML (`<mlt>`, `<producer>`, `<chain>`, `<playlist>`, `<tractor>`,
  `<transition>`, `<filter>`), Kdenlive-specifikus `kdenlive:` tulajdonságokkal.
- **Források:** a Kdenlive projektbetöltő kódja és az MLT XML-dokumentáció. A Kdenlive-kódból
  csak a GPL-3-mal kompatibilis fejlécű fájlok vehetők át; a logikát Rustra kell írni.
- **Tesztminták:** Kdenlive 26.08-ban saját magunk által készített projektek, a
  `crates/interchange` tesztkészletében.
- **Sorrend:** profil → sávok → klipek, vágópontok → csoportok → sebesség → áttűnések →
  markerek, guide-ok → szekvenciák → feliratok → gyakori effektek (transform, volume, LUT).
- **Kész, ha:** a mintaprojektek megnyílnak, a vágópontok képkocka-pontosan egyeznek, és az
  import jelenti, mit nem tudott áthozni (VISION.md 2.4: bizonyíték, nem állítás).
- **Hová:** `crates/interchange` új modulja; a `file.import` felismeri a `.kdenlive`-ot.
- **Kötelező** (tulajdonosi döntés, 2026-10-25). MLT-könyvtár nem kell hozzá: a `.kdenlive` XML-t
  a Rust olvasó közvetlenül értelmezi.
- **Első lépés:** a Kdenlive OTIO-exportja a meglévő `otio.rs`-sel, valódi fájlokon tesztelve
  (az OTIO-terv, #43, OTIO-4.2 interop-mátrixával együtt). **Kész (2026-10-25):** a Kdenlive saját
  CC0 OTIO-tesztkészlete (8 fájl, `crates/interchange/tests/data/kdenlive/`) a
  `tests/kdenlive_otio.rs`-ben fut: sávok, klipek, vágópontok, guide-ok, klipmarkerek, színklipek,
  áttűnések, hiányzó média. Javítva: a Kdenlive `kdenlive:SolidColor` színe (`0xRRGGBBAA`) helyesen
  jön át; a „Transition” nevű áttűnés a `transition_type` szerint képződik le; az üres `target_url`
  offline médiát ad (korábban a projektmappára mutatott).
- **Kompatibilitási jelentés:** minden elem „támogatott / átalakított / nem támogatott” osztályba
  kerül; a jelentés az import után megjelenik.
- **Orákulum:** a `melt` (MLT) csak a tesztekben fut, a Kdenlive-renderrel való képkockás
  összevetéshez. **Kdenlive-be exportálás nem lesz**, mert csak veszteséggel menne.

## 11. Kiegészítő teendők a Kdenlive segédrepóiból (2026-10-25)

A Kdenlive 26.11 fejlesztői ágán kívül tíz kiegészítő repó került át: `kdenlive-export-profiles`
(CC0), `kdenlive_ease_generator` (GPL-3.0), `gyroflow-frei0r` (MIT/Apache-2.0, a `gyroflow-core`
GPL-3.0), `kdenlive-profiles` (GPL-2.0, „or later” nélkül: csak tartalmi minta),
`kdenlive-clipper` (MIT), `kdenlive-network-render` (GPL-3.0), `kdenlive-multirender` (MIT),
`kdenlive-post-render-script` (GPL-3.0-or-later), `kdenlive-project-analyzer` (GPL-3.0),
`kdenlive-tools` (BSD). A legtöbb elavult; a működési elvüket vesszük át. Becslés: ügynökóra,
mérés nélkül.

| # | Feladat | Prioritás | Óra | Előfeltétel | Minta |
|---|---|---|---|---|---|
| **K10** | Render saját folyamatban, hogy egy kódoló összeomlása ne vigye magával a szerkesztőmotort (`cfa4-cli render`, JSON-sorok). A Qt6 motorja már külön folyamat (`cfa4-cli serve`), exportsor és Progress panel van (`qt-parity.md` 3.25, 4.2) | P1 | 10–20 | ADR F8 2. lépés | `kdenlive_render` + `QLocalSocket`, `TaskManager` |
| **K11** | Platform-előbeállítások (YouTube, Instagram, TikTok, AV1, VP9, archív, hardveres) | P1 | 15–25 | `cfa4-route` (#55) a hardveres sorokhoz | `kdenlive-export-profiles` (CC0) |
| **K12** | Kamerák saját proxyfájljainak felismerése (GoPro LRV, DJI LRF, Sony, Insta360) | P1 | 8–15 | megvan (`media.createProxies`) | `data/externalproxies.rc`, `camcorderfilters.rc` |
| **K13** | Giroszkópos stabilizálás `gyroflow-core`-ral, opcionális funkcióként | P1 | 35–55 | függőség-audit | `gyroflow-frei0r` |
| **K14** | Párhuzamos és hálózati render, `cfa4-render-node` csak renderelő kliens | P2 | 70–120 | K10 | `kdenlive-multirender`, `kdenlive-network-render` |
| **K15** | Export utáni lépések: borítókép, MP4-fejezetek, YouTube-fejezetszöveg, export jelölőnként | P2 | 12–20 | K10 | `post-render-script`, `kdenlive-clipper`, `exportguidesdialog` |
| **K16** | Egyedi klipfeladatok, kötegelt átkódolás | P2 | 25–40 | K10, `cfa4-route` | `customjobtask`, `kdenlivetranscodingrc` |
| **K17** | Projektnyilvántartás SQLite-tal: sorozatok, epizódok, feliratsablonok, presetek, proxyk, modellek | P2 | 30–50 | ADR F8 3. lépés | Kdenlive Library |
| **K18** | Online források és letöltéskezelő: Hugging Face-modellek, Freesound, Pixabay, egyéb linkek | P2 | 25–45 | K4 és K6 előtt | `src/onlineresources`, `data/resourceproviders` |
| **K19** | További easing-típusok (elastic, bounce, back, expo …) | P3 | 8–15 | – | `kdenlive_ease_generator` |

A K1–K19 együtt kb. 400–700 óra; a K10–K19 kb. 255–430 óra. A 11.1 pont K20–K25 teendőivel
együtt (kb. 66–107 óra) a K1–K25 kb. 470–810 óra; a 11.2 pont K26–K30 teendőivel (kb. 74–120 óra)
a K1–K30 kb. 545–930 óra; a 11.3 pont K31–K35 teendőivel (kb. 78–130 óra) a teljes terv
kb. 625–1060 óra volt. A K10 szűkítése (2026-10-27, 11.4) után kb. 610–1035 óra.

### Részletek

- **K10.** A Qt6-héj `QProcess`-szel, munkaszálon indítja a `cfa4-cli render`-t; az előrehaladás
  JSON-sorokban jön. Kész, ha a Qt-héj leállítása után a render befejeződik, és a megszakítás nem
  hagy félkész fájlt. Ugyanez a minta szolgálja a proxy- és átkódolási feladatokat.
- **K11.** A profilok tartalma (felbontás, bitráta, GOP, hang) a `crates/export/src/presets.rs`
  formátumára átírva, `ATTRIBUTION.md`-bejegyzéssel. A hardveres presetek a `cfa4-route` választásával
  futnak (saját NVENC / VA-API, FFmpeg `*_vaapi`, GStreamer `va*enc`, ADR 0005). Ha a gyártói
  meghajtó hiányzik, a CFA4 megmutatja, honnan telepíthető: előbb az Ubuntu-csomag (`apt`, NVIDIA-n
  „További illesztőprogramok”), utána a gyártó telepítési oldalának linkje; magától semmit nem tölt
  le. A linkek adatfájlban vannak, kiadás előtt ellenőrizzük őket.
- **K12.** Adatfájlban leírt névpárok; a kameraproxy a meglévő `ProxySource`-ként csatolódik,
  újrakódolás nélkül.
- **K13.** A `gyroflow-core` Rust crate opcionális Cargo-funkcióként, frei0r-felület nélkül,
  `catch_unwind` alatt. Kész, ha GoPro-mintán mérhetően jobb a 2D Warp Stabilizernél.
- **K14.** GOP-határon darabolt render, a saját muxer újrakódolás nélkül fűz. Több gép, gépenként
  több GPU; a vezérlő a csomópontok mért sebessége szerint oszt, a kieső csomópont szakaszát újra
  kiosztja. A `cfa4-render-node` szerkesztőfelület nélkül, gyenge irodai gépen is fut, alacsony
  prioritással vagy tétlen időben, a gép használója szüneteltetheti. Helyi hálózat, párosítás,
  titkosított kapcsolat, ujjlenyomattal ellenőrzött média, azonos CFA4-verzió. Kész, ha két gépen
  (az egyik csak renderelő kliens) ugyanazt a fájlt adja, mint az egyben renderelés.
- **K15.** A CFA4 már ír `udta`-metaadatot és ismer fejezetjelölőt (`MarkerKind::Chapter`); ehhez
  jön a borítókép, az MP4-fejezet, a YouTube-szöveg és a felhasználó által megadott export utáni parancs.
- **K16.** Kötegelt átkódolás a K10 feladatkezelőjén, a `cfa4-route` által választott háttérrel
  (saját motor, FFmpeg 9, GStreamer; ADR 0005).
- **K17.** Helyi SQLite (`rusqlite`) a felhasználói adatkönyvtárban. A projekt igazsága a `.fcproj`
  marad; az adatbázis a projektfájlokból újraépíthető. Kész, ha egy sorozat új epizódja egy lépésben
  megkapja a sorozat feliratsablonját és presetjeit.
- **K18.** Közös letöltő, adatfájlban leírt szolgáltatókkal. Letöltés előtt látszik a forrás, a méret
  és a licenc; csak jóváhagyással indul (VISION.md 5.); hash-ellenőrzés, folytatható letöltés,
  bejegyzés a K17-be. Hálózat nélkül minden más működik.
- **K19.** Penner-függvények új `Interpolation` változatként vagy Bezier-kulcskockákká sütve.

### 11.1 Átvétel az FA3 elődtervből (FA3-VIDEO-EDITOR-001, 2026-10-25)

A CFA4 elődjének videoszerkesztő-terve saját szerkesztőmagot, Kdenlive-kompatibilitási réteget és
Kdenlive-alkalmazást mint teszt-orákulumot írt elő; ez egyezik a CFA4 irányával. **Nem vesszük át**
belőle: az MLT- és FFmpeg-alapmotort (12. pont, ADR 0005), a `.kdenlive`-exportot, a C++
`EditorialService` réteget (a CFA4 parancsrendszere ezt adja, `Tick`-idővel), a QSV- és AMF-kódolót
(ADR 0003 §5), és a régi PyQt-, BeautifulSoup- és XSLT-felületeket.

| # | Feladat | Prioritás | Óra | Előfeltétel | Minta |
|---|---|---|---|---|---|
| **K20** | Kdenlive effekt-, átmenet- és generátorkatalógus: leképezési tábla Kdenlive-azonosító → CFA4-effekt a K1 importhoz | P1 | 15–25 | K1 | `data/effects`, `transitions`, `generators` (CC0) |
| **K21** | CC0 lumák (átmenetmaszkok), LUT-ok és átmenet-előnézetek eszközként | P2 | 6–10 | – | `data/lumas`, `data/luts`, `data/transitions/previews` (CC0) |
| **K22** | Hiányzó média újrakapcsolása, forrásfájl-csere; hash a médiahivatkozásban; az eredeti `.kdenlive` megőrzése import után | P1 | 10–15 | K1 | `kdenlive-tools` (BSD-2-Clause) |
| **K23** | Projekt-ellenőrző: hiányzó források, proxyk, effektek, integritás, teljesítmény | P2 | 12–20 | – | `kdenlive-project-analyzer` (GPL-3.0) információs modellje |
| **K24** | Diavetítés-automatizálás: Ken Burns kezdő- és végkeret, képsorrend, ütemhez igazított időtartam | P3 | 15–25 | licencellenőrzés | `kdenlive_slideshow_editor` (MIT az FA3-terv szerint, nem ellenőrzött) |
| **K25** | Upstream-kompatibilitási CI: friss Kdenlive mintaprojektek → CFA4 import → render → összevetés a `melt` kimenetével | P1 (a K1 része) | 8–12 | K1 | FA3-terv 18. pont |

- **K20.** Az XML-ek MLT- és frei0r-szűrőket írnak le (például `audiobalance.xml` → MLT `panner`),
  ezért leírásként használjuk őket: paraméternevek, tartományok, alapértékek, UI-szövegek. Minden
  effekt saját Rust-implementációval fut; ami nem feleltethető meg, a K1 kompatibilitási
  jelentésében „nem támogatott” sorba kerül. Kész, ha a K1 mintaprojektjeinek effektjei mind
  szerepelnek a táblában.
- **K21.** Minden átvett fájlhoz `.attribution` kísérőfájl és `ATTRIBUTION.md`-bejegyzés
  (forrás, upstream commit, eredeti útvonal, SPDX, helyi módosítás); a `cargo xtask assets` ellenőrzi.
- **K22.** A régi Python CLI nem kerül át, csak a működés: forráslista, útvonal-leképezés,
  forráscsere. Kész, ha egy áthelyezett médiakönyvtárú `.kdenlive` projekt egy lépésben újrakapcsolható.
- **K23.** A megnyitott projekt modelljéből dolgozik, XML-elemzés nélkül; a régi GUI és XSLT nem kell.
  Eredménye a vezérlőcsatornán is lekérdezhető.
- **K24.** Csak a licenc ellenőrzése után; az algoritmusok kellenek (geometria, Ken Burns, ütem),
  a PyQt-felület nem.
- **K25.** A Kdenlive alkalmazás és a `melt` csak a CI-ben fut, teszt-orákulumként.

Tervezési elvek a meglévő pontokhoz:

- **K11:** a *szállítási profil* (felbontás, fps, kodek, minőség, színtér, konténer) külön van az
  *enkóder-szolgáltatótól* (szoftver, VA-API, NVENC); a `cfa4-route` választ alá, így egy preset
  nem kötődik GPU-gyártóhoz.
- **K14:** RenderRequest → tervező → szakaszoló → erőforrás-elosztó → workerek → minőségellenőrzés →
  összefűzés → nyugta; a helyi és a hálózati render ugyanaz a kód. A `kdenlive-network-render`
  megoldásait (nyers socket, SSHFS, interaktív `input()`, alfolyamatnak átadott SSH-parancs, amely
  a saját README-je szerint kódinjekcióra ad lehetőséget) nem vesszük át.
- **K15:** általános „render utáni művelet” felület (ellenőrzés, metaadat, borító, felirat-mux,
  bélyegkép, hangosság, ellenőrzőösszeg, archiválás, közzététel).
- **AI-funkciók (K2, K4, K6):** ugyanazokat a parancsokat hívják, mint az ember, előnézettel,
  visszavonhatóan, naplózva. Előbb stabil kézi szerkesztés, csak utána AI.

### 11.2 Felirat- és átiratlánc az FA3 Kdenlive-konszolidációjából (2026-10-25)

Az FA3 konszolidációja a Kdenlive-ot tette szerkesztővé és az OTIO-t a projekt igazságává; ezt a
CFA4-ben nem vesszük át (a CFA4 maga a szerkesztő, a `.fcproj` az igazság, a Kdenlive importforrás).
Egyező szabály: külső `.kdenlive` írás nincs, és az MI által javasolt idővonal-módosítás csak emberi
jóváhagyással kerül be. A CFA4-ben ma megvan a Whisper (`transcript.generate`), a szószintű időzítés
és a beszélőfelismerés (`docs/transcripts.md`); az alábbiak hiányoznak.

| # | Feladat | Prioritás | Óra | Előfeltétel | Minta |
|---|---|---|---|---|---|
| **K26** | Hangleválasztás (beszéd és zene szétválasztása) opcionális előfeldolgozásként a felismerés előtt | P2 | 25–40 | MI-réteg, K2, K18 | Demucs (MIT) |
| **K27** | Szóidők ellenőrzése és kényszerített igazítás (forced alignment), ha a natív időzítés pontatlan | P2 | 25–40 | MI-réteg, K2 | CTC-alapú igazítás (modell licencellenőrzéssel) |
| **K28** | Fordítás nyomkövetéssel: forrás- és célnyelv, modell, verzió a feliratsáv mellett | P2 (a K6 része) | 6–10 | MI-réteg, K6 | FA3 `TranslationArtifact` |
| **K29** | Feliratminőség-ellenőrzés az idővonalra kerülés előtt: olvasási sebesség, sorhossz, sorok száma, legrövidebb és leghosszabb időtartam, átfedés, rés | P1 | 15–25 | – | FA3 `CaptionQualityEvidence` |
| **K30** | SBV (YouTube) felirat import és export | P3 | 3–5 | – | Kdenlive feliratimport |

- **K26.** A felismerés a leválasztott beszédsávon fut, de a felirat szövegének és időzítésének
  forrása a felismerő marad; a leválasztás csak javítja a bemenetet. Saját Rust-futtatás (candle),
  a modell a K18 letöltőjével, jóváhagyással érkezik. Kész, ha zenés mintán mérhetően csökken a WER.
- **K27.** Az ellenőrzés jelzi a gyanús szóidőket (nulla hossz, átfedés, ugrás); igazítás csak
  ezeknél fut. Az eredmény külön, visszavonható lépés. Modell csak kereskedelmi használatot engedő
  licenccel (a CC-BY-NC súlyok, például az MMS, kizárva).
- **K28.** A lefordított sáv metaadatban hordozza a forrássávot, a nyelvpárt és a modellt; a forrás
  módosulása után jelzi, hogy a fordítás elavult.
- **K29.** Állítható szabálykészlet (alapértelmezés: legfeljebb 17 karakter/másodperc, 42 karakter/sor,
  2 sor, 0,83–7 másodperc), presetként mentve (K17: sorozatonkénti feliratsablon). A hibák listája a
  vezérlőcsatornán is lekérdezhető; export előtt figyelmeztet.
- **K30.** A `crates/captions` meglévő SRT- és VTT-kezelése mellé.

### 11.3 Whisper-bővítések egy korábbi Kdenlive + Whisper beszélgetésből (2026-10-25)

A beszélgetés Python-szkriptjei (faster-whisper, FFmpeg `shell=True`-val, `sudo`, szöveges
parancs-tiltólista) a CFA4-be nem kerülnek át: a CFA4-ben a Whisper Rustban, a programba építve fut
(`crates/speech`, candle), van beszédszakasz-felismerés (`vad.rs`) és beszélőfelismerés, és az MI csak
regisztrált parancsokat hívhat, shellt nem. Az ötletek közül ezek hiányoznak:

| # | Feladat | Prioritás | Óra | Előfeltétel | Minta |
|---|---|---|---|---|---|
| **K31** | Nagyobb Whisper-modellek a katalógusban: `medium`, `large-v3`, `large-v3-turbo` (MIT) | P1 | 6–10 | MI-réteg, K2, K18 | OpenAI Whisper (Hugging Face) |
| **K32** | Whisper (és Kokoro) GPU-n, gyártófüggetlenül, CPU-tartalékkal | P2 | 30–50 | MI-réteg, ADR 0004 (#55) 2. fázis | Burn: Vulkan, CUDA, ROCm; Intel NPU: OpenVINO (4. fázis) |
| **K33** | Magyar pontosságmérés (WER) modellenként, mérési jegyzőkönyvvel | P1 | 4–8 | MI-réteg, K2 | `crates/speech/tests/whisper_model.rs` |
| **K34** | Figyelt mappa: a bedobott médiából átirat és felirat automatikusan, jóváhagyott beállításokkal | P2 (a K16 része) | 8–12 | MI-réteg, K10, K16 | – |
| **K35** | Élő felirat mikrofonról vagy élő bemenetről | P3 | 30–50 | MI-réteg, K32, élő adás (VISION, #60) | – |

- **K31.** A modellek nincsenek a csomagban; a K18 letöltője tölti le őket jóváhagyással,
  SHA-256-tal rögzítve, mint a mai `whisper-tiny/base/small`. A `large-v3-turbo` a gyors
  alapértelmezés jelöltje, ha a K33 mérése igazolja.
- **K32.** Nem saját CUDA-út: az ADR 0004 szerint a `cfa4-route` választ, NVIDIA-specifikus út csak
  gyártófüggetlen párja mellett. Kész, ha az ADR 0004 2. fázisának MI-küszöbei teljesülnek.
- **K33.** Közös, nyílt licencű magyar beszédmintákon (a minták nem kerülnek a repóba), a mostani
  angol mérés (`docs/transcripts.md`) mintájára.
- **K34.** A K16 feladatkezelőjén fut; egy fájl csak akkor indul, ha már nem íródik.
- **K35.** Az ADR 0004 szerint a valós idejű MI-előnézet az egyetlen kivétel a CPU-tartalék alól:
  GPU nélkül kikapcsol. Csak az élő adás ütemezési döntése után.

### 11.4 Átfedések más tervekkel és a szállítás módja (2026-10-27)

A 2026-10-27-i újraelemzés szerint a K-lista nem változik, de néhány pontot azóta egy jóváhagyott
terv vagy nyitott PR részben lefed. Ezeket nem írjuk meg újra:

| K-pont | Átfedés | Teendő |
|---|---|---|
| K10 | A Qt6 motorja külön folyamat, van exportsor és Progress panel | Csak a render saját folyamata marad (lásd a K10 sort) |
| K14 | [Smart Render](plans/SMART-RENDER-2026-10-27.md) és [GOP Smart Cut](plans/GOP-SMART-CUT-2026-10-27.md) terv; nyitott Smart Edit PR-ok (#215, #216, #219, #228, #229) | A szakaszolás és a vegyes copy/render összefűzés a Smart Renderből jön; a K14 erre épül |
| K15, K23, K29 | Nyitott QC-sorozat (`cfa4-qc`, `cfa4-cli qc`, Qt6 Quality Check ablak: #255, #302, #305, #306, #317, #319) | A K23 és a K15 ellenőrző lépése új `cfa4-qc`-szabály; a K29 feliratszabályai QC-profil |
| K25 | [OTIO-integrációs terv](plans/OTIO-INTEGRATION-2026-10-24.md) 4. fázisa (interop-mátrix), nyitott #44 (OTIO tesztorákulum) | A Kdenlive OTIO-része oda tartozik; a `melt`-összevetés csak a `.kdenlive` részre kell |
| K19, K11, K16 | Nyitott #182 (easing presetek), #188 (közösségimédia-export, kötegelt változatok) | Bekerülésük után a K19 valószínűleg kész, a K11 és a K16 kisebb |

**Szállítás** ([`coordination.md`](coordination.md), tulajdonosi döntés 2026-10-27): ág mindig
friss `main`-ből, egy PR egy K-pont (vagy annak egy lépése); csak a kijelölt beolvasztási gazda
olvaszt be; ebben a közös fájlban csak a saját sor módosítható; ha 20-nál több PR nyitott, új
K-pont csak a tulajdonos kérésére indul.

**MI-pontok** (tulajdonosi szabály, 2026-10-27; 12.1): a K2, K4, K6, K26–K28 és K31–K35 csak az
MI-réteg üzembe helyezése után kezdhető. A K29 (feliratminőség-ellenőrzés) szabályalapú, nem MI,
így marad.

## 12. Tulajdonosi döntések (2026-10-25)

| Téma | Döntés |
|---|---|
| `.kdenlive`-import (K1) | kötelező; Kdenlive-be exportálás nem lesz |
| MLT (`melt`) | csak teszt-orákulum, a termékbe nem kerül; az FFmpeg 9 + MLT 7 motorjavaslat elvetve |
| FFmpeg | ADR 0005 (elfogadva, a `main`-ben): FFmpeg ≥ 9.0.2 szállított kiegészítő háttér (`cfa4-ffmpeg`, csak GPL-3-as build) a `cfa4-route` mögött; a CI FFmpeg 9.0.2-je külön orákulum |
| GStreamer | a kiadott csomagok GStreamerrel épülnek (alapból az Ubuntu rendszercsomagja); elkülönített `cfa4-gstreamer-runtime` csak akkor, ha egy funkció újabb verziót kér (ADR 0002, ADR 0005) |
| `gyroflow-core` (K13) | kell |
| Hálózati render (K14) | több gép és több GPU engedélyezett; csak renderelő kliens is készül |
| Fordítómodell (K6) | a csomagba egy nyitott, kereskedelmi használatot is engedő modell (jelölt: Opus-MT/Marian magyar–angol, licencellenőrzés után); a felhasználó opcionálisan cserélheti. SeamlessM4T és NLLB nem (CC-BY-NC) |
| Hardveres kódolás (K11) | a gyártói meghajtót a felhasználó telepíti, hiány esetén telepítési link |
| SQLite-nyilvántartás (K17) | kell |
| Online források (K18) | fontos |
| Qt6-idővonal | **Qt Widgets marad** (#48, #63); a Kdenlive QML-idővonala csak referencia, átvenni a `.ui` fájlokat és a modell/nézet mintát érdemes |

### 12.1 A CFA4 MI-döntései és a terv (ellenőrizve 2026-10-25)

| Döntés | Forrás | Hol van a tervben |
|---|---|---|
| **Minden MI-funkció, minden alkalmazásban, csak az MI-réteg üzembe helyezése után kerül be**; az MI-rétegről külön terv készül | tulajdonos, 2026-10-27 | K2, K4, K6, K26–K28, K31–K35 függősége: „MI-réteg” |
| Az MI alapvetően helyi, a felhasználó adatai nem hagyják el a gépet | VISION.md 2.3 | minden MI-teendő (K2, K4, K6, K26–K35) helyben fut |
| Modell CPU-n, bármely gyártó GPU-ján vagy NPU-n; NVIDIA-út csak gyártófüggetlen párral | VISION.md 2.4, 5. | K4, K13, K32 |
| Minden modell licence és forrása rögzítve; letöltés csak a felhasználó tudtával | VISION.md 5. | K18 (jóváhagyás, licenc, hash), K31 |
| Kötelező CPU-tartalék; kivétel csak a valós idejű MI-előnézet | ADR 0004 (#55), 1. döntés | K32, K35 |
| MI-futtatókörnyezet a candle mellé: Burn (Vulkan, CUDA, ROCm, CPU); NPU: előbb Intel, OpenVINO | ADR 0004 (#55), 3–4. döntés | K32 |
| A csomagban egy nyílt, kereskedelmi használatot engedő fordítómodell; a felhasználó cserélheti, a betöltött modell licence látszik, nem kereskedelmi licencre figyelmeztet | tulajdonos, 2026-10-25 | K6, K28 |
| SeamlessM4T, NLLB, MMS-súlyok kizárva (CC-BY-NC) | licencellenőrzés, 2026-10-25 | K6, K27 |
| Az MI ugyanazokat a parancsokat hívja, mint az ember, előnézettel és visszavonhatóan; idővonal-módosítás csak emberi jóváhagyással | 11.1, 11.2 (FA3-tervekből) | K2, K4, K6, K26–K35 |
| Előbb stabil kézi szerkesztés, csak utána MI | VISION.md 6.; FA3-terv | sorrend: K1, K3 és a nem MI-s pontok az MI-réteg és az MI-pontok előtt |
| Whisper a kiadott buildekben (ma ki van kapcsolva) | `docs/gaps.md` G19 | K2 |

A VISION.md 5. pontjának MI-irányai közül ezek **nem Kdenlive-eredetűek**, ezért ebben a tervben
nincs saját teendőjük; a `docs/gaps.md` G12 és a VISION.md követi őket: szinkron (fordított szöveg
helyi gépi hangon, szájmozgáshoz időzítve), beszédtisztítás, objektumkövetés, médiakeresés tartalom
szerint, automatikus képkivágás (Auto Reframe), hangtípusok címkézése. Meglévők: jelenetfelismerés,
színillesztés, szövegalapú vágás, Kokoro-82M felolvasás.

**Nyitott (VISION.md 8.):** hány MI-modell kerüljön a csomagba a fordítómodellen túl, és szabad-e
kivételesen, kifejezett engedéllyel felhős MI-szolgáltatást használni. A K31 és a K18 addig a
„csomagban nincs, jóváhagyással letölthető” szabályt követi.

## 13. Hogyan frissítsd ezt a fájlt

- Ha egy ❓ sort kipróbáltál, cseréld ✅, 🟡 vagy ❌ jelre, és írd mellé a parancsot vagy issue-t.
- Ha a Qt6-felületen elkészül egy terület, jelöld külön (pl. „✅ motor, ✅ Qt6”).
- Kdenlive-kód átvételekor a sor mellé írd a `docs/licensing.md` §5 bejegyzését.
- Új Kdenlive-kiadásnál olvasd át a kiadási jegyzeteket.

## Források

- Kdenlive kiadási jegyzetek: 26.04.0 (2026-04-27), 26.08.0 (2026-08-31) — kdenlive.org
- Kdenlive háttéreltávolítás (SAM2), 25.04 — kdenlive.org, 2024-12
- CFA4: `VISION.md`, `AGENTS.md`, `docs/licensing.md`, ADR F8, ADR 0003, `docs/gaps.md`,
  `crates/engine/src/*` parancs-azonosítók (`main` @ `bbbefc3`), PR-ek: #39, #40 (beolvasztva), #41, #44, #48

## Változásnapló

| Dátum | Változás | Összefoglaló |
|---|---|---|
| 2026-10-24 | major | Első változat (MIT/Apache-feltételezéssel) |
| 2026-10-24 | major | 2. változat: GPL-3.0-or-later (Kdenlive-kód átvehető), Qt6/PySide6-felület, GStreamer, gyártófüggetlen gyorsítás, Linux AMD64-only, márkaeltávolítás; rangsor az ADR F8 és ADR 0003 lépéseihez kötve |
| 2026-10-25 | minor | Médiahátterek az ADR 0005 (#62) szerint: saját motor az alap, FFmpeg ≥ 9.0.2 és Ubuntu GStreamer kiegészítő, automatikus útválasztóval (a visszavont #53 helyett) |
| 2026-10-25 | major | 3. változat: a tíz kiegészítő Kdenlive-repó elemzése, K10–K19, K1 kötelező (OTIO-teszt, kompatibilitási jelentés, `melt`-orákulum, export nélkül), tulajdonosi döntések (12. pont) |
| 2026-10-25 | minor | 11.1: K20–K25 és tervezési elvek az FA3 elődtervből (FA3-VIDEO-EDITOR-001); a Kdenlive `REUSE.toml` CC0-részei rögzítve |
| 2026-10-25 | minor | 11.2: K26–K30 felirat- és átiratlánc az FA3 Kdenlive-konszolidációjából (hangleválasztás, kényszerített igazítás, fordítás nyomkövetéssel, feliratminőség-ellenőrzés, SBV) |
| 2026-10-25 | minor | A tulajdonos jóváhagyta a tervet (K1–K30) |
| 2026-10-25 | minor | K1 első lépése kész: a Kdenlive CC0 OTIO-tesztkészlete a CFA4 OTIO-importjával tesztelve, három javítással |
| 2026-10-25 | minor | 11.3: K31–K35 Whisper-bővítések; 12.1: a CFA4 MI-döntéseinek ellenőrzése (VISION.md, ADR 0004, tulajdonosi döntések) |
| 2026-10-27 | minor | Újraelemzés a `main` (16debf92) és a nyitott PR-ok alapján, a tulajdonos jóváhagyásával: ADR 0005 elfogadva (FFmpeg-feltétel törölve), kiadás GStreamerrel, Qt6-állapot a `qt-parity.md`-ből, K10 szűkítve, 11.4 átfedések és szállítási szabályok |
| 2026-10-27 | minor | Tulajdonosi szabály: minden MI-funkció csak az MI-réteg üzembe helyezése után; a K2, K4, K6, K26–K28, K31–K35 függősége „MI-réteg” (12.1, 11.4) |
