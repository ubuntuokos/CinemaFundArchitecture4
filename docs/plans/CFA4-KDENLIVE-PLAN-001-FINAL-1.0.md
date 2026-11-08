# CFA4-KDENLIVE-PLAN-001 — FINAL 1.0

## Kdenlive-paritás és Kdenlive-eredetű képességek — egységes, végleges terv

| Dokumentumadat | Érték |
|---|---|
| Verzió | FINAL 1.0 |
| Állapot | **Jóváhagyva** (tulajdonos: ubuntuokos, 2026-11-03). Ez a Kdenlive egyetlen érvényes terve; minden korábbi Kdenlive-terv elavult |
| Dátum | 2026. november 3. |
| Helye | `docs/plans/CFA4-KDENLIVE-PLAN-001-FINAL-1.0.md` a CinemaFundArchitecture4 `main` ágán (tulajdonosi szabály, 2026-11-03: terv csak itt lehet) |
| Felváltja | `docs/kdenlive-parity.md` (CinemaFundArchitecture4, `c9b024f`, 2026-10-27); `docs/kdenlive-parity.md` (CFA4-AI_modules, `e241771`, 2026-10-30); CFA4-KDENLIVE-CAPABILITY-ADAPTATION-001 v1.0 (2026-11-03) |
| Végrehajtás | A dokumentum elfogadása nem indít végrehajtást; minden K-pont előtt a IV. pont kapuja kötelező |

## I. Összefoglaló

Ez a terv három, egymásnak részben ellentmondó dokumentumot fésül össze. A két `kdenlive-parity.md`
2026-10-25-én vált ketté (közös ős: `581ca0e`): a fő repóban a K31–K35, a 11.4 és a 12.1 készült el,
a CFA4-AI_modules-ban az ADR 0006 szerinti 11.3. Az ADAPTATION-001 a Kdenlive-forkokat hozta. Az
ellentmondásokat a tulajdonos szabálya szerint oldottuk fel: **ahol két jóváhagyott szöveg ütközik,
az újabb nyer** (III. pont).

A tartalmi változások a korábbi tervekhez képest:

- **Egy feladat egy helyen él.** A beszéd- és feliratpontok (K6, K9, K26–K33) a Subtitle Studio-terv
  vagy az ADR 0006 feladatai; itt csak hivatkozás marad (11.2, 11.4).
- **Az MI-pontok az ADR 0006 kapuihoz kötődnek**, nem az általános „MI-réteg után” feltételhez. A K2
  ezért most indítható (ADR 0006 F0).
- **A CFA4 nem szállít modellt**; a fordítómodellt is a felhasználó tölti le (K6).
- **Új: a Kdenlive-forkok** (11.6, K36–K41) és az adaptációs terv hiányzó függőségeinek helye (D1–D6).
- **Végrehajtási kapu** minden K-pont előtt (IV. pont).

## II. Források

| Jel | Forrás | Mit ad |
|---|---|---|
| S1 | CinemaFundArchitecture4 `docs/kdenlive-parity.md` @ `c9b024f` (2026-10-27) | K1–K35, 11.4, 12, 12.1 |
| S2 | CFA4-AI_modules `docs/kdenlive-parity.md` @ `e241771` (2026-10-30) | 11.3 (MI-részek az ADR 0006 szerint), K2/K4/K6 új előfeltételei, K6-döntés |
| S3 | CFA4-KDENLIVE-CAPABILITY-ADAPTATION-001 v1.0 (2026-11-03) | A forkok listája, átvételi módszertan (A–E), függőségi igények |
| S4 | [Subtitle Studio FINAL 1.2](SUBTITLE-STUDIO-PLAN-001-FINAL-1.2.md) (2026-10-30) és [1.1](SUBTITLE-STUDIO-PLAN-001-FINAL-1.1.md) | A feliratozás egész területe |
| S5 | CFA4-AI_modules [ADR 0006](https://github.com/ubuntuokos/CFA4-AI_modules/blob/main/docs/adr/ADR-0006-ai-processing-layer.md) (2026-10-30) és `docs/ai/hang-besorolas.csv` | MI-réteg, kapuk F0–F6, motorcsaládok |
| S6 | [`open-editors-parity.md`](../open-editors-parity.md) (E1), [`venturi-parity.md`](../venturi-parity.md) (V3), [`control-protocol.md`](../control-protocol.md), [`agents.md`](../agents.md) | Ügynöki szerkesztés, vezérlőcsatorna |

## III. Feloldott ellentmondások

| # | Téma | Régebbi szöveg | Újabb szöveg | Döntés ebben a tervben |
|---|---|---|---|---|
| E1 | Két `kdenlive-parity.md` | S1 (2026-10-27) | S2 (2026-10-30) | Mindkettőt ez a terv váltja fel; a S1-ben meglévő, S2-ből hiányzó részek (K31–K35, 11.4, 12.1) átkerültek |
| E2 | K2 indítása | S1, 10-27: „csak az MI-réteg üzembe helyezése után” | S5, 10-30: ADR 0006 F0 kifejezetten engedi, mert kész kód | **Most indítható** (ADR 0006 F0) |
| E3 | MI-pontok előfeltétele | S1: „MI-réteg” | S2, S5: konkrét ADR 0006 kapu (K4 → F4, K2/K6 → F5) | Az ADR 0006 kapuja (11.3) |
| E4 | Fordítómodell (K6) | S1: a csomagba egy modell kerül | S2: a CFA4 nem szállít modellt, a felhasználó tölti le (K18) | **Nem szállítunk modellt** |
| E5 | Két 11.3 pont | S1: Whisper-bővítések | S2: MI-részek az ADR 0006 szerint | 11.3 = MI-részek (S2), 11.4 = Whisper-bővítések, 11.5 = átfedések és szállítás |
| E6 | Felhős MI, külső fordítás | S1 12.1: nyitott (VISION.md 8.) | S4 D3 (10-30): külső fordítás csak kapcsolható, külön folyamatként, alapból ki | A fordításra a D3 érvényes; egyéb felhős MI továbbra is nyitott |
| E7 | Feliratpontok helye | S1: K6, K9, K26–K33 itt | S4 (10-30) ugyanezeket lefedi | A feladat a Subtitle Studióé vagy az ADR 0006-é; itt hivatkozás (11.2, 11.4) |
| E8 | K29 alapértékei | S1: 17 karakter/mp, 42 karakter/sor | S4 (1.1 §10): nincs egyetemes CPS-érték | A K29 értékei csak egy előre megadott preset lehetnek a Subtitle Studio QC-profiljában |
| E9 | MI-futtatók (K32) | S1 12.1 és ADR 0004: Burn (Vulkan, CUDA) a Whisperhez | S5: futtatósorrend ONNX → candle → PyTorch, a Whisper candle-on (F5) | Az ADR 0006 sorrendje; a Burn sorsáról az ADR 0004 és 0006 összehangolása dönt (13. pont) |
| E10 | Kdenlive-export, MI-ügynök P0-ként, GPL-3 tiltás, saját Editing API/Core | S1, S2 tulajdonosi döntései (12. pont) | S3 (11-03) | **Nem vesszük át.** Az S3 időben újabb, de a tulajdonos 2026-11-03-án kimondta, hogy az S3 csak a forkokból származó új képességeket tartalmazhatja; ezek a pontok nem forkból jönnek, hanem a meglévő döntéseket írnák felül. **Megerősítve** a terv jóváhagyásával (2026-11-03) |
| E11 | MLT mint motor a termékben | S3 (Backend Adapter Layer), CFA4-SMART-CUT-RENDER-R1 175. és 562. sor | S1, S2 (MLT csak tesztorákulum), ADR 0005 4. pont | **Eldöntve (tulajdonos, 2026-11-03):** a saját Rust-motor marad az alap, az MLT most csak tesztorákulum. Később opcionális háttérként bekerülhet a `cfa4-route` alá az ADR 0005 4. pontja szerint, ha mérés mutatja, hogy szükség van rá; ehhez az ADR 0005 kiegészítése kell. A meglévő motor- és rendertervek nem változnak |

## IV. Végrehajtási kapu (tulajdonosi szabály, 2026-11-03)

Minden K-pont végrehajtása előtt kötelező:

1. a CFA4 (`main`, nyitott PR-ok) és a K-pontban felhasznált minden tároló aktualizálása és újraelemzése;
2. az így felmerült új kérdések és függőségek összegyűjtése és rendezése, szükség esetén tulajdonosi döntéssel;
3. ha egy K-pont közben elkészült, átfed vagy fölösleges lett, a sor frissítése ebben a tervben.

A végrehajtás csak ezek után indulhat. A tervnek csak a `docs/plans/` mappában lévő példánya
érvényes; minden máshol talált változat vázlat.

## 0. Licenc: mit lehet átvenni a Kdenlive-ból

A CFA4 licence **GPL-3.0-or-later** (#37, [`docs/licensing.md`](../licensing.md)). A Kdenlive
GPL-licencű, ezért **a kódja átvehető**, de csak fájlonként ellenőrizve:

- **Átvehető:** GPL-3.0, GPL-2.0-or-later, LGPL fejlécű fájl. A KDE-ben gyakori
  „`GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL`” fejléc a GPL-3.0 opcióval
  átvehető.
- **Nem vehető át:** a csak GPL-2.0-only fejlécű fájl. Ennek a képességét saját kóddal kell
  megírni.
- **Kötelező minden átvételnél:** az eredeti szerzői jogi fejléc megtartása, és egy sor a
  [`docs/licensing.md` §5](../licensing.md) nyilvántartásban, ugyanabban a commitban.
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
[`qt-parity.md`](../qt-parity.md)-ben van (2026-10-27: 40 pont ✅, 19 🟡, 9 ❌; ADR F8 2–6. lépés
folyamatban, 7. nyitott).

## 1. Projekt, fájlok, médiakezelés

| Kdenlive-képesség | CFA4 | Megjegyzés, bizonyíték |
|---|:-:|---|
| `.kdenlive` projekt megnyitása (MLT XML) | ❌ | **K1**, lásd lent |
| OpenTimelineIO import / export | ✅ | `file.exportOtio`, `crates/interchange/src/otio.rs`; orákulum-teszt: #44 |
| Proxy klipek | ✅ | `media.createProxies`, `media.toggleProxies`, `media.proxyPresets` |
| Projekt archiválása mappába | ✅ | `file.projectManager` (collect / consolidate) |
| Tömörített archív (zip) | ❌ | **K45** (11.7) |
| Automatikus mentés, helyreállítás | ✅ | `autoSave.*`, `file.recover` |
| Klip szerkesztése külső programban (pl. Gyroflow) | 🟡 | `edit.editOriginal`; a visszatöltés: **K48** (11.7) |
| Fájlok megnyitása fájlkezelőből | ✅ | XDG-asztali fájl és MIME-társítás (F6-audit) |
| Képsorozat, diavetítés, WebP | ✅ | `file.importImageSequence` |
| Beágyazott feliratfolyamok felismerése | 🟡 | `media.includeCaptionsOnImport` |
| Figyelmeztetés távoli erőforrásra | ❌ | **K46** (11.7) |
| Renderelt fájl a Project Binbe | ❌ | **K47** (11.7) |

## 2. Idővonal és vágás

| Kdenlive-képesség | CFA4 | Megjegyzés, bizonyíték |
|---|:-:|---|
| Korlátlan sáv, több szekvencia, beágyazás | ✅ | `sequence.addTracks`, `file.newSequence`, `clip.nest` |
| Kijelölés másolása új szekvenciába | ✅ | `sequence.makeSubsequence` |
| Hangsáv automatikus létrehozása | ✅ | `timeline.addTracksAutomatically` |
| Sávok átrendezése | ❌ | Nincs sávmozgató parancs; **K49** (11.7) |
| Marker a klipek közti résekbe | ❌ | Résre ugrás van (`sequence.goToNextGap`); **K50** (11.7) |
| Lejátszófej illesztése | ✅ | `timeline.snapPlayhead` |
| Ripple, roll, slip, slide, razor, csoportok | ✅ | `tool.*`, `trim.*`, `clip.group` |
| Áttűnés vágásponton („mix”) | ✅ | `sequence.applyVideoTransition` |
| Áttűnés automatikus hossza, animált előnézet | 🟡 | Alapértelmezett hossz van (`timeline.videoTransitionDuration`); animált előnézet: **K51** (11.7) |
| Sebesség, visszafelé, time remap, kimerevítés | ✅ | `clip.speedDuration`, `clip.frameHold` |
| Több klip sebessége egyszerre | ❌ | **K52** (11.7) |
| Multicam, szinkron időkód alapján | ✅ | `clip.createMulticam`, `multicam.*`, `clip.synchronize` |
| Rögzített lejátszófej, középső egérgombos görgetés | ❌ | **K53** (11.7) |
| Klipszínek típusonként | ✅ | `labels.defaults.*` |
| Undo-előzmények törlése | ❌ | `history.list` van; **K54** (11.7) |

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
| Rajzolás a monitoron (Drawbox, Delogo) | 🟡 | Maszkok, alakzatok vannak; Delogo: **K56** (11.7) |
| Heatmap, Color Enhance (frei0r) | ❌ | frei0r-hosttal együtt (fent) |
| Effektek kikapcsolása az egész idővonalon | ❌ | Klipenként van (`effects.toggleEnabled`); **K55** (11.7) |

## 4. Feliratok, beszéd, szöveg

| Kdenlive-képesség | CFA4 | Megjegyzés, bizonyíték |
|---|:-:|---|
| Feliratszerkesztő, több sáv, SRT, VTT | ✅ | `captions.*`, `crates/captions` |
| ASS / SSA feliratok | ❌ | **K9**, a Subtitle Studio-terv feladata (F2). Kdenlive- vagy Subtitle Edit-kódból is átvehető, fájlonként ellenőrizve |
| Beszédfelismerés (Whisper) | 🟡 | Megvan (`transcript.generate`), a kiadott buildekben ki van kapcsolva (G19). **K2** |
| Szöveg alapú vágás | ✅ | `transcript.*`, `textPanel.*` |
| Felirat fordítása | ❌ | **K6**. A Kdenlive által használt SeamlessM4T nem kereskedelmi licencű; kereskedelmi használatot engedő, a felhasználó által letöltött modell kell (11.3) |
| Címszerkesztő | ✅ | `graphics.*` |
| Címszerkesztő-illesztés, lekerekített téglalap | 🟡 | `view.snapInProgramMonitor`; a többi: **K57** (11.7) |
| Görgetett stáblista | ✅ | `crates/credits`, táblázatból is (F10, #36) |
| (CFA4-többlet) Szövegfelolvasás | ✅ | `tts.*`, Kokoro-82M |

## 5. Hang

| Kdenlive-képesség | CFA4 | Megjegyzés, bizonyíték |
|---|:-:|---|
| Hangkeverő, szóló, némítás | ✅ | `mixer.*` |
| Szóló hozzáadása Shift+kattintással | ❌ | **K58** (11.7) |
| Hangfelvétel (voice-over) | ✅ | `audio.voiceover.*` |
| Normalizálás, EQ, zengetés, zajcsökkentés, ducking | ✅ | Saját DSP, `repair.*`, `essentialSound.generateDucking` |
| Időnyújtás és hangmagasság jobb minőségben | 🟡 | Saját pitch shift; Rubber Band: ADR 0003 A6 |
| Hanghullám szekvenciákon | ❌ | Klipeken van; **K59** (11.7) |

## 6. Monitorok, mérőeszközök

| Kdenlive-képesség | CFA4 | Megjegyzés, bizonyíték |
|---|:-:|---|
| Waveform, vektorszkóp, hisztogram, RGB parade | ✅ | `crates/scopes` |
| Hangspektrum, spektrogram panel | 🟡 | Spektrumrajz a hangeffekt-szerkesztőben; külön panel nincs. **K8** |
| Teljes képernyős monitor | ✅ | `window.toggleFullScreen` |
| Monitor tükrözése második kijelzőre | ❌ | `multicam.transmitView` van, eszköz nélkül; **K60** (11.7) |

## 7. Renderelés, export

| Kdenlive-képesség | CFA4 | Megjegyzés, bizonyíték |
|---|:-:|---|
| Render-sablonok, render-sor, zóna-render, előnézet-render | ✅ | `export.presets.*`, `export.queue.*`, `sequence.renderInToOut` |
| Szüneteltetés és folytatás | 🟡 | Megszakítás van; szüneteltetés: **K61** (11.7) |
| Alfa export | ✅ | ProRes 4444, PNG-sorozat |
| Hardveres kódolás AMD-n és Intelen (VA-API) | 🟡 | Az NVENC export és a Linux HW-dekódolás már a `cfa4-route`-on át megy (ADR 0004 1. fázis, részben kész). VA-API-kódolás a saját motorban vagy kiegészítő háttérből (FFmpeg `*_vaapi`, GStreamer `va*enc`); a kiadott csomagok GStreamerrel épülnek (ADR 0002, ADR 0005); gyártófüggetlen terv (#39) |
| Szoftveres HEVC, jobb H.264, MP3 export | ❌ | x265, x264, LAME: **a CFA4 nem szállítja**; a felhasználó az Ubuntu tárolójából telepíti (tulajdonosi döntés, 2026-11-03). A CFA4 futáskor felismeri őket a rendszer GStreamerén át (`x264enc`, `x265enc`, `lamemp3enc`), a `cfa4-route` alatt; ha hiányoznak, a funkció látható, de inaktív, és mellette ott a telepítési útmutató. Teendő: **K62** (11.7), igazítás: T10 |
| Széles formátumlefedettség | 🟡 | A saját Rust-motor az alap; kiegészítő hátterek: FFmpeg ≥ 9.0.2 (tervezett `cfa4-ffmpeg`) és a GStreamer (alapból az Ubuntué, újabb verzióhoz elkülönített CFA4-futtatókörnyezet; `crates/gstreamer`, #40), köztük az automatikus útválasztó (`cfa4-route`) dönt (ADR 0005, elfogadva). Az FFmpeg ≥ 9.0.2 (csak GPL-3-as build) a `/opt/cfa4`-ben szállítva; a `cfa4-ffmpeg` háttér a #249-ben, a sorrendező a #263-ban készül |

## 8. Platform és felület

| Kdenlive-képesség | CFA4 | Megjegyzés, bizonyíték |
|---|:-:|---|
| Linux, deb, AppImage, Flatpak | 🟡 | Csak Ubuntu 26.04 AMD64; mind a négy formátum tervezett, a kiadás HOLD (F9) |
| Qt-felület | 🟡 | Qt6 Widgets + PySide6; az egui befagyasztva, a pontos paritásig marad (#48); állapot: [`qt-parity.md`](../qt-parity.md) |
| Wayland és X11 | ❓ | VISION.md 3. pont: minden 26.04-es változaton tesztelendő |
| Munkaterületek, billentyűparancsok, témák | ✅ | Motorban és egui-ban; Qt6-ban 🟡 (dokkolás kész, billentyűparancs-szerkesztő a #214-ben) |

## 9. Teendők rangsora

A sorrend a VISION.md 6. pontját követi: a Qt6-felület az első, ezért minden felületi munka
közvetlenül Qt6-ban készül, nem egui-ban.

| # | Feladat | Kapcsolódó lépés | Átvehető forrás |
|---|---|---|---|
| **K2** | Whisper bekapcsolása a kiadott buildekben | most indítható: ADR 0006 F0 (G19; III. pont E2); a CFA4 candle-Whisperje, Vosk nem kell (11.3) | — (kész kód) |
| **K1** | `.kdenlive` (MLT XML) projektimport | motor most; Qt6-menü az ADR F8 3. lépésénél | Kdenlive és MLT olvasó logikája, fájlonként |
| **K3** | Linux-stabilitás, Wayland és X11 minden változaton | ADR F8 5–7. lépés, VISION.md 3. pont | — |
| **K7** | Matematikai kifejezés a számmezőkben | ADR F8 4. lépés (közös Qt6-spinbox) | Kdenlive spinbox-logikája, ha a fejléce engedi |
| **K8** | Hangspektrum- és spektrogram-panel | ADR F8 5. lépés | Kdenlive vagy Audacity-kód, fájlonként |
| **K9** | ASS / SSA felirat | a Subtitle Studio F2 fázisában (`crates/captions`) | Subtitle Edit (GPL-3.0) vagy Kdenlive |
| **K4** | AI háttéreltávolítás (SAM2-szerű) | ADR 0006 F4 kapu, ONNX-worker (11.3) | SAM2 (Apache-2.0), helyben, nem csak NVIDIA-n |
| **K5** | frei0r és LV2 effektek külön folyamatban | ADR 0003 A6–A7 | frei0r, lilv |
| **K6** | Feliratfordítás kereskedelmi használatot engedő modellel | Subtitle Studio F5 és ADR 0006 M10 (11.3) | modellválasztás kell (licenc és helyi futás) |

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

| # | Feladat | Prioritás | Óra | Előfeltétel | Minta | Hol él (2026-11-03) |
|---|---|---|---|---|---|---|
| **K26** | Hangleválasztás (beszéd és zene szétválasztása) opcionális előfeldolgozásként a felismerés előtt | P2 | 25–40 | ADR 0006 F6, K2, K18 | Demucs (MIT) | **ADR 0006**, M15 (`hang-besorolas.csv` 23. sor) |
| **K27** | Szóidők ellenőrzése és kényszerített igazítás (forced alignment), ha a natív időzítés pontatlan | P2 | 25–40 | ADR 0006, K2 | CTC-alapú igazítás (modell licencellenőrzéssel) | **Subtitle Studio** §7 szóillesztés; modell: ADR 0006 M10 (csv 37. sor) |
| **K28** | Fordítás nyomkövetéssel: forrás- és célnyelv, modell, verzió a feliratsáv mellett | P2 (a K6 része) | 6–10 | K6 | FA3 `TranslationArtifact` | **Subtitle Studio** §5 (sávszerep, revízió), AC-19 |
| **K29** | Feliratminőség-ellenőrzés az idővonalra kerülés előtt | P1 | 15–25 | – | FA3 `CaptionQualityEvidence` | **Subtitle Studio** §10 QC-profil (III. pont E8) |
| **K30** | SBV (YouTube) felirat import és export | P3 | 3–5 | – | Kdenlive feliratimport | **Subtitle Studio**: a formátumlistába felveendő (13. pont) |

Ezek a pontok itt csak hivatkozásként maradnak, a K-számuk nem változik. Végrehajtásuk és
elfogadásuk a megjelölt terv szerint történik. Az eredeti részletek (S1 11.2) a Subtitle Studio-terv
következő változatába viendők át, ahol még hiányoznak.

### 11.3 MI-részek az ADR 0006 szerint (2026-10-30)

A Kdenlive AI-funkciói nem saját futtatókörnyezettel jönnek át, hanem a CFA4 AI-rétegén
([ADR 0006](https://github.com/ubuntuokos/CFA4-AI_modules/blob/main/docs/adr/ADR-0006-ai-processing-layer.md)) futnak. A workerek a
[CFA4-sdk](https://github.com/ubuntuokos/CFA4-sdk) protokollját beszélik.

- **Sorrend:** az AI-réteg crate-jei csak a Qt6-átállás befejezése és az egui kivezetése után
  indulnak (ADR 0006 F0). Kivétel a K2, mert az kész kód bekapcsolása. Ez felváltja a 2026-10-27-i
  „minden MI-funkció csak az MI-réteg után” szabályt a K2-re (III. pont E2); a többi MI-pontra a
  szabály az ADR 0006 kapuin keresztül érvényes marad.
- **K2 (beszédfelismerés, M10, F5 kapu):** a CFA4 saját, candle-alapú Whisperje
  (`transcript.generate`). A Kdenlive Vosk-ága és Python-alapú Whisper-hívása nem kell. A
  Speech Editor szövegalapú vágási logikája Qt6-ban átvehető, fájlonkénti licencellenőrzéssel.
- **K4 (háttéreltávolítás, M6, F4 kapu):** SAM2 ONNX Runtime-workerben, gyártófüggetlen
  futtatókkal (CUDA/TensorRT, MIGraphX, OpenVINO, CPU). A Kdenlive SAM2-szkriptje
  (Apache-2.0 OR GPL-3.0) csak referencia, a CFA4 nem futtatja.
- **K6 és K28 (fordítás, M10):** csak kereskedelmi használatot engedő modell. A SeamlessM4T és
  az NLLB (CC-BY-NC) kizárva.
- **K26 (hangleválasztás):** Demucs (MIT) a közös PyTorch-workerben, vagy ONNX-exporttal.
  **K27 (kényszerített igazítás):** CTC-modell candle-on vagy ONNX-en, licencellenőrzés után.
- **Futtatókörnyezet:** a Kdenlive CUDA-verziónkénti venv- és requirements-rendszerét nem
  vesszük át. Egy közös, uv-vel rögzített PyTorch-környezet van egy CUDA 12.x-szel, a sorrend
  ONNX → candle → PyTorch (ADR 0006, 2026-10-30).
- **Modellek:** a CFA4 nem szállít modellt. A felhasználó a K18 letöltővel tölti le őket,
  jóváhagyással, a manifestben rögzített ellenőrzőösszeggel. A képesség csak a telepített
  modellel jelenik meg.
- **Működés:** az `ai.*` parancsok a motor parancsregiszterében vannak, és előnézettel,
  visszavonhatóan, naplózva futnak (11.1). Az idővonalra csak jóváhagyás után kerül eredmény.
- **Minimumteszt:** minden AI-funkciót a `scripts/dev/run-minspec.sh` alatt mérünk (32 GB RAM,
  8 CPU, egy 12 GB-os referencia-GPU).

### 11.4 Whisper-bővítések egy korábbi Kdenlive + Whisper beszélgetésből (2026-10-25)

A beszélgetés Python-szkriptjei (faster-whisper, FFmpeg `shell=True`-val, `sudo`, szöveges
parancs-tiltólista) a CFA4-be nem kerülnek át: a CFA4-ben a Whisper Rustban, a programba építve fut
(`crates/speech`, candle), van beszédszakasz-felismerés (`vad.rs`) és beszélőfelismerés, és az MI csak
regisztrált parancsokat hívhat, shellt nem. Az ötletek közül ezek hiányoznak:


| # | Feladat | Prioritás | Óra | Előfeltétel | Minta | Hol él (2026-11-03) |
|---|---|---|---|---|---|---|
| **K31** | Nagyobb Whisper-modellek a katalógusban: `medium`, `large-v3`, `large-v3-turbo` (MIT) | P1 | 6–10 | ADR 0006 F1 (modellkezelő), K2, K18 | OpenAI Whisper (Hugging Face) | **ADR 0006** modellkezelő; a konkrét modelleket a hangtáblába fel kell venni (13. pont) |
| **K32** | Whisper (és Kokoro) GPU-n, gyártófüggetlenül, CPU-tartalékkal | P2 | 30–50 | ADR 0006 F5, ADR 0004 | a futtatósorrend az ADR 0006 szerint (III. pont E9) | **ADR 0006** és ADR 0004 |
| **K33** | Magyar pontosságmérés (WER) modellenként, mérési jegyzőkönyvvel | P1 | 4–8 | K2 | `crates/speech/tests/whisper_model.rs` | **Subtitle Studio** AC-08 |
| **K34** | Figyelt mappa: a bedobott médiából átirat és felirat automatikusan, jóváhagyott beállításokkal | P2 (a K16 része) | 8–12 | ADR 0006 F5, K10, K16 | – | **Itt marad**, a K16 része (más terv nem fedi) |
| **K35** | Élő felirat mikrofonról vagy élő bemenetről | P3 | 30–50 | ADR 0006, K32, élő adás (VISION, #60) | – | Az **élő adás terve** (még nincs); addig itt marad |

- **K31.** A modellek nincsenek a csomagban; a K18 letöltője tölti le őket jóváhagyással,
  SHA-256-tal rögzítve, mint a mai `whisper-tiny/base/small`. A `large-v3-turbo` a gyors
  alapértelmezés jelöltje, ha a K33 mérése igazolja.
- **K34.** A K16 feladatkezelőjén fut; egy fájl csak akkor indul, ha már nem íródik.
- **K35.** Az ADR 0004 szerint a valós idejű MI-előnézet az egyetlen kivétel a CPU-tartalék alól:
  GPU nélkül kikapcsol. Csak az élő adás ütemezési döntése után. A motor: `hang-besorolas.csv`
  36. sor (valós idejű átirat).

### 11.5 Átfedések más tervekkel és a szállítás módja (2026-10-27)

A 2026-10-27-i újraelemzés szerint a K-lista nem változik, de néhány pontot azóta egy jóváhagyott
terv vagy nyitott PR részben lefed. Ezeket nem írjuk meg újra:

| K-pont | Átfedés | Teendő |
|---|---|---|
| K10 | A Qt6 motorja külön folyamat, van exportsor és Progress panel | Csak a render saját folyamata marad (lásd a K10 sort) |
| K14 | [Smart Render](SMART-RENDER-2026-10-27.md) és [GOP Smart Cut](GOP-SMART-CUT-2026-10-27.md) terv; nyitott Smart Edit PR-ok (#215, #216, #219, #228, #229) | A szakaszolás és a vegyes copy/render összefűzés a Smart Renderből jön; a K14 erre épül |
| K15, K23, K29 | Nyitott QC-sorozat (`cfa4-qc`, `cfa4-cli qc`, Qt6 Quality Check ablak: #255, #302, #305, #306, #317, #319) | A K23 és a K15 ellenőrző lépése új `cfa4-qc`-szabály; a K29 feliratszabályai QC-profil |
| K25 | [OTIO-integrációs terv](OTIO-INTEGRATION-2026-10-24.md) 4. fázisa (interop-mátrix), nyitott #44 (OTIO tesztorákulum) | A Kdenlive OTIO-része oda tartozik; a `melt`-összevetés csak a `.kdenlive` részre kell |
| K19, K11, K16 | Nyitott #182 (easing presetek), #188 (közösségimédia-export, kötegelt változatok) | Bekerülésük után a K19 valószínűleg kész, a K11 és a K16 kisebb |

**Szállítás** ([`coordination.md`](../coordination.md), tulajdonosi döntés 2026-10-27): ág mindig
friss `main`-ből, egy PR egy K-pont (vagy annak egy lépése); csak a kijelölt beolvasztási gazda
olvaszt be; ebben a közös fájlban csak a saját sor módosítható; ha 20-nál több PR nyitott, új
K-pont csak a tulajdonos kérésére indul.

**MI-pontok** (tulajdonosi szabály, 2026-10-27, az ADR 0006 2026-10-30-i kapuival pontosítva): a
K4, K6, K26–K28, K31, K32, K34, K35 és K41 csak a 11.3 szerinti ADR 0006 kapu után kezdhető; a K2
most indítható (III. pont E2). A K29 (feliratminőség-ellenőrzés) szabályalapú, nem MI.

### 11.6 Kdenlive-forkok és a hiányzó függőségek (az ADAPTATION-001 alapján, 2026-11-03)

A CFA4-KDENLIVE-CAPABILITY-ADAPTATION-001 v1.0 terv (jóváhagyva 2026-11-03) a Kdenlive
**forkjait** és az azokra épülő automatizálási és MI-projekteket vizsgálja. Ezek a fenti pontokban
nem szerepelnek, mert a 11. pont a hivatalos Kdenlive-ot és tíz segédrepót elemezte. Ebből a
tervből csak az új rész kerül ide; amit a CFA4 már tud, vagy amit a 12. pont döntései kizárnak,
az nem.

**Nem vesszük át a tervből:** az MLT-t mint termékbeli háttért (12. pont: csak teszt-orákulum), a
Kdenlive-exportot (12. pont), az MI-ügynököt P0-ként (12.1: minden MI-funkció csak az MI-réteg
után), a GPL-3.0-kód átvételének tiltását (0. pont: a CFA4 GPL-3.0-or-later), a saját
„Universal Editing API”, „Editing Core” és headless motor építését (megvan: parancsregiszter,
[`control-protocol.md`](../control-protocol.md), `cfa4-cli mcp` fejetlen és híd módban,
[`agents.md`](../agents.md)), valamint az F0–F10 fázisszámokat (az F-jelölés az ADR-eké).

**Vizsgálandó forkok:** [D-Ogi/kdenlive](https://github.com/D-Ogi/kdenlive),
[D-Ogi/kdenlive-api](https://github.com/D-Ogi/kdenlive-api),
[D-Ogi/mcp-kdenlive](https://github.com/D-Ogi/mcp-kdenlive),
[DanEscher98/kdenlive](https://github.com/DanEscher98/kdenlive) (WebSocket, JSON-RPC),
[IO-AtelierTech/kdenlive-automation](https://github.com/IO-AtelierTech/kdenlive-automation) (Python),
[0xmeowmeow/vibecut](https://github.com/0xmeowmeow/vibecut),
[hive-agi/hive-kdenlive](https://github.com/hive-agi/hive-kdenlive) és az Arynwood Cutroom
(repója még azonosítandó). Létezésüket, licencüket és állapotukat a K36 ellenőrzi.

#### Kapu: újraelemzés a kivitelezés előtt (tulajdonosi döntés, 2026-11-03)

A K37–K41 egyike sem indulhat, amíg a **K36 le nem zárult** és a tulajdonos el nem fogadta.
A K36 egyszerre elemzi újra a forkokat és a **CFA4 akkori állapotát** (`main` commit, nyitott
PR-ok, az ADR 0006 és a V3, E1 állása). Ha a K36 szerint egy pont már kész, átfed vagy fölösleges,
a pont kikerül vagy szűkül, és ez a táblázat a K36 eredményével frissül. A K36 a IV. pont
kapujának alkalmazása a forkokra.

#### Hiányzó függőségek (D1–D6) és hol valósulnak meg

Az adaptációs terv hat olyan elemre hivatkozik, amely a CFA4 fő repójában nincs, vagy más néven
van. Új infrastruktúrát egyikhez sem építünk; mindegyik egy meglévő tervhez vagy egy K-ponthoz
kerül.

| # | Függőség a tervben | CFA4 ma (2026-11-03) | Hol valósul meg | Teendő |
|---|---|---|---|---|
| **D1** | MI-réteg | ADR 0006 elfogadva (2026-10-30) a [CFA4-AI_modules](https://github.com/ubuntuokos/CFA4-AI_modules/blob/main/docs/adr/ADR-0006-ai-processing-layer.md) repóban, kód még nincs; a fő repóban nincs ADR 0006. Az F0 kapuja a teljes Qt6-átállás | ADR 0006 F0–F6 | Az MI-t használó K41 csak az ADR 0006 F3 (első képesség, teljes Előnézet → Alkalmazás lánc) után indul |
| **D2** | Model Router | Az ADR 0006 1. eleme: `cfa4-ai-registry` (a CFA3 Model Router utódja) | ADR 0006 F1 | Nincs külön teendő; a „Model Router” név helyett a `cfa4-ai-registry` |
| **D3** | HRB (Host Resource Broker) | Az ADR 0006 4. eleme: erőforrás-kezelő a `cfa4-ai-jobs`-ban. A CFA3 HRB-jétől a CFA4 nem függhet ([Subtitle Studio-terv](SUBTITLE-STUDIO-PLAN-001-FINAL-1.2.md)) | ADR 0006 F2 | Nincs külön teendő |
| **D4** | Event Fabric (CFA4-EVENT-FABRIC-001) | Ilyen dokumentum vagy kód egyik CFA4-repóban sincs. A vezérlőcsatorna csak kérés–válasz; esemény ma csak az MCP `notifications/progress` és `notifications/cancelled` | **K39** | Eseményfeliratkozás a meglévő csatornákon; külön eseményinfrastruktúra nincs. Ha a CFA4-EVENT-FABRIC-001 máshol létezik, a K36 ahhoz igazítja |
| **D5** | API-hitelesítés és jogosultsági szintek | A vezérlőcsatorna TCP-n, hitelesítés nélkül megy (`control-protocol.md`). A Unix socketet (0600) és a mozdulat alatti visszatartást a V3 tervezi | V3, utána **K38** | A V3 adja a szállítást; a K38 csak a jogosultsági szinteket |
| **D6** | AI-jóváhagyás, egy lépésben visszavonható terv | Nincs; az [E1](../open-editors-parity.md) tervezi (`plan.prepare` / `plan.commit`, revízióhoz kötve), MI-réteg nélkül indítható | E1 | Nincs külön teendő; a K41 előfeltétele |

#### Teendők

| # | Feladat | Prioritás | Óra | Előfeltétel | Minta |
|---|---|---|---|---|---|
| **K36** | **Kapu.** A forkok és a CFA4 aktuális állapotának újraelemzése: forkonként licenc (fájlszinten is), utolsó commit, ténylegesen működő funkciók, A–E átvételi kategória; a CFA4 `main`, a nyitott PR-ok és a D1–D6 állása; delta-tábla (megvan / részben / hiányzik) | P1 | 12–20 | – | a 11. pont elemzése |
| **K37** | Hiányzó MCP-eszközök és parancsok a forkok eszközkészletéhez mérve | P1 | 10–20 | K36 | D-Ogi `kdenlive-api`, `mcp-kdenlive` |
| **K38** | Jogosultsági szintek a vezérlőcsatornán és az MCP-n: olvasó, szerkesztő, romboló; az MCP-annotációkra (`readOnlyHint`, `destructiveHint`) építve | P1 | 10–20 | K36, V3 | DanEscher98 (JSON-RPC) |
| **K39** | Eseményfeliratkozás (D4): projektrevízió, jobállapot, export-előrehaladás a vezérlőcsatornán és MCP-értesítésként | P2 | 15–30 | K36, V3 | DanEscher98 (WebSocket) |
| **K40** | Python-automatizálási minták összevetése a CFA4 Python-klienseivel (`agents.md`, `python/`) | P2 | 6–12 | K36 | IO-AtelierTech |
| **K41** | MI-szerkesztési munkafolyamatok (például csendvágás, előzetes, legjobb felvétel) a forkok mintái alapján | P2 | a K36 után | K36, E1, **MI-réteg** (ADR 0006 F3) | vibecut, hive-kdenlive, Cutroom |

A K36–K40 együtt kb. 53–102 óra; a K41 becslése a K36 eredményéből készül. A K41 MI-pont
(12.1), ezért a 11.5 MI-szabálya rá is vonatkozik.

- **K36.** A README nem elég: belépési pontok, API és RPC, adatmodell, tesztek. A kísérleti
  állapot önmagában nem kizáró ok. Kimenet: forrásonkénti jelentés és a K37–K41 frissített sorai
  ebben a pontban, egy PR-ban. Kész, ha minden vizsgált funkcióhoz van forráshely, licenc és
  döntés, és a tulajdonos elfogadta.
- **K37.** A hiányzó eszköz új parancs a `crates/engine` regiszterében, így a felület, a CLI és az
  MCP egyszerre kapja meg; külön MCP-logika nincs. Kész, ha a K36 listájának minden pontja
  parancs, vagy indokolt elutasítás.
- **K38.** A szint az eszköz annotációjából jön, nem külön listából. Kész, ha olvasó szinten egy
  módosító hívás hibát ad, és ezt regressziós teszt igazolja (`crates/automation` vagy `python/tests/`).
- **K39.** Az esemény csak jelez; a projektet továbbra is csak parancs módosíthatja. Kész, ha egy
  feliratkozott kliens a felhasználó szerkesztése után megkapja az új revíziót.
- **K41.** Az MI csak az E1 tervén át szerkeszt (előnézet, egy visszavonási lépés), a modellt a
  `cfa4-ai-registry` választja, az erőforrást az ADR 0006 erőforrás-kezelője foglalja.

### 11.7 Amit MLT nélkül a saját motorban kell pótolni (2026-11-03)

A tulajdonos 2026-11-03-i döntése (III. pont E11): a saját Rust-motor az alap, az MLT csak
tesztorákulum. Ezért ami a Kdenlive-ban az MLT-ből jön, azt a CFA4-nek magának kell tudnia, vagy
egy kiegészítő háttérből a `cfa4-route` alatt (ADR 0005). Ez a pont két listát ad.

- **A lista:** az MLT-szolgáltatások, amelyekre a Kdenlive épít.
- **B lista:** a fenti 1–8. pont ❓ és ❌ sorai, amelyeknek eddig nem volt K-száma.

Az ellenőrzés a main 942d046 parancsazonosítóin alapul. A K36-os újraelemzés (IV. pont) ezt
megerősíti vagy javítja, és csak utána indulhat a kivitelezés.

**A. MLT-szolgáltatások és a CFA4 megfelelője**

| MLT-szolgáltatás (Kdenlive használja) | CFA4 most | Teendő |
|---|---|---|
| `avformat` producer és consumer (dekódolás, kódolás) | ✅ saját kodekek + FFmpeg/GStreamer a routeren át | — |
| `timewarp` (sebesség, visszafelé) | ✅ `clip.speedDuration` | — |
| `color`, `qimage`, `pixbuf` képsorozat producerek | ✅ `file.importImageSequence`, színmatt | — |
| `kdenlivetitle` / `qtext` (címek) | ✅ `graphics.*` | a K1-import leképezése (K42) |
| `glaxnimate` / Lottie-animáció producer | ❌ | **K43**, a főcímgenerátor eleme |
| `composite`, `affine`, `qtblend`, `luma`, `mix` átmenetek és keverések | ✅ saját áttűnések, `motion.*` | a K1-import leképezése (K42) |
| `movit` GPU-szűrők | ✅ `crates/gpu`, gyártófüggetlen | — |
| `avfilter.*` (az FFmpeg összes szűrője) | ❌ | **K44**, opcionális háttérként |
| `frei0r.*`, `ladspa`, `lv2` | ❌ | már a **K5** (ADR 0003 A6–A7) |
| `rubberband` (időnyújtás) | 🟡 | már az ADR 0003 A6 |
| Előnézet-render darabokban (timeline preview) | ✅ `sequence.renderInToOut` | — |
| `multi` consumer, második monitor | ❌ | **K60**: elsősorban a gép további monitorjai; SDI/Blackmagic-kártya csak később, opcionálisan |

**B. Eddig számozatlan hiányok az 1–8. pontból**

| # | Feladat | Honnan (pont) | Kapcsolódó lépés | Becslés (óra) |
|---|---|---|---|---|
| **K42** | MLT-szolgáltatás → CFA4-parancs leképezési tábla a K1-importhoz; a le nem képezett szolgáltatás figyelmeztetést ad az import-jelentésben, nem crash-el | 1. (K1) | a K1-gyel együtt, `crates/interchange` | 12–20 |
| **K43** | Lottie-animáció (Glaxnimate-kimenet) lejátszása és szerkesztése **a főcímgenerátor elemeként** (tulajdonosi döntés, 2026-11-03), saját Rust-értelmezővel; a részletes feladat a főcímgenerátor tervébe kerül, itt csak hivatkozás marad | A lista | a főcímgenerátor terve szerint; ilyen terv még nincs a main `docs/plans/` alatt | 30–60 (a főcímgenerátor tervében) |
| **K44** | `cfa4-ffmpeg` szűrők elérése effektként a routeren át, ahol a saját motor nem tud valamit; a saját effekt marad az első | A lista | ADR 0005 2. lépés után | 16–30 |
| **K45** | Projekt archiválása tömörített fájlba (zip vagy tar) | 1. | `file.projectManager` bővítése | 4–8 |
| **K46** | Figyelmeztetés távoli (hálózati) erőforrásra importkor | 1. | motor | 2–4 |
| **K47** | A renderelt fájl automatikus felvétele a Project Binbe | 1. | `export.queue.*` | 2–4 |
| **K48** | Külső programban szerkesztett klip automatikus visszatöltése | 1. | `edit.editOriginal` bővítése | 4–8 |
| **K49** | Sávok átrendezése (fel, le, húzással) | 2. | motor most, Qt6-felület az ADR F8 5. lépésénél | 6–10 |
| **K50** | Marker a klipek közti résekbe | 2. | motor | 2–4 |
| **K51** | Áttűnés animált előnézete (az alapértelmezett hossz már megvan: `timeline.videoTransitionDuration`) | 2. | Qt6, ADR F8 5. lépés | 6–12 |
| **K52** | Több kijelölt klip sebességének egyszerre állítása | 2. | `clip.speedDuration` bővítése | 3–6 |
| **K53** | Rögzített lejátszófej és középső egérgombos görgetés az idővonalon | 2. | Qt6, ADR F8 5. lépés | 4–8 |
| **K54** | Undo-előzmények törlése | 2. | motor (`history.*`) | 1–2 |
| **K55** | Minden effekt ki- és bekapcsolása az egész idővonalon | 3. | motor (`effects.*`) | 2–4 |
| **K56** | Logóeltávolító (Delogo) effekt | 3. | saját effekt | 8–16 |
| **K57** | Lekerekített téglalap és igazítóvonalak a címszerkesztőben | 4. | `graphics.*`, Qt6 | 4–8 |
| **K58** | Szóló hozzáadása Shift+kattintással a keverőben | 5. | Qt6, ADR F8 5. lépés | 1–2 |
| **K59** | Hanghullám beágyazott szekvenciákon | 5. | motor és Qt6 | 6–12 |
| **K60** | **Többmonitoros munka** (a tulajdonos szerint a gépek többmonitorosak, 2026-11-03): a Program és a Source monitor teljes képernyőn egy választott további kijelzőn (Wayland és X11), panelek és lebegő ablakok áthelyezése más monitorra, a kijelzőhozzárendelés mentése a munkaterületben, és visszaállás, ha a monitor nincs bekapcsolva. A `multicam.transmitView` erre épül. SDI/Blackmagic-kimenet csak később, opcionális háttérként | 6. és A lista | motor (kijelzőválasztás, állapot serde-ben), Qt6 `QScreen` az ADR F8 5. lépésénél; K3-mal együtt tesztelve | 16–30 |
| **K61** | Render szüneteltetése és folytatása | 7. | `export.queue.*`, `jobs.*` | 6–12 |
| **K62** | x264, x265, LAME használata a felhasználó által az Ubuntu tárolójából telepített csomagokból: felismerés (`probe()`), exportbeállítás, inaktív állapot telepítési útmutatóval, ha hiányzik; a CFA4 nem szállítja és nem linkeli statikusan | 7. | ADR 0005 2. lépés, ADR F8 6. lépés (export) | 10–20 |

Összesen kb. **146–280 óra**. A Wayland és X11 tesztelése már a **K3**, a beágyazott
feliratfolyamok a Subtitle Studio-terv feladata, a pontkövetés és a 3-D stabilizálás az
ADR 0003 A4-ben és a G18-ban van.

**Sorrend.** A tisztán motoros pontok (K42, K45–K47, K50, K52, K54, K55) bármikor indulhatnak
a végrehajtási kapu után, mert nem kell hozzájuk felület. A felületi pontok (K49, K51, K53, K57,
K58) az ADR F8 Qt6-lépéseivel mennek. A K44, K56, K59, K60, K61 és K62 a K36 újraelemzés eredménye szerint sorolódik; a K43 a
főcímgenerátor tervével megy.

**Ütemezés (tulajdonosi döntés, 2026-11-03):** a 11.7 pontjai később jönnek; a terv
jóváhagyása nem ütemezi őket előre.

**Kész, ha** minden pontnál van parancs a `crates/engine` regiszterében, automatizálási
azonosító, teszt, és az 1–8. pont táblájában a sor ✅-re vált.

## 12. Tulajdonosi döntések (2026-10-25, frissítve 2026-11-03)

| Téma | Döntés |
|---|---|
| `.kdenlive`-import (K1) | kötelező; Kdenlive-be exportálás nem lesz |
| MLT (`melt`) | csak teszt-orákulum, a termékbe nem kerül; az FFmpeg 9 + MLT 7 motorjavaslat elvetve |
| FFmpeg | ADR 0005 (elfogadva, a `main`-ben): FFmpeg ≥ 9.0.2 szállított kiegészítő háttér (`cfa4-ffmpeg`, csak GPL-3-as build) a `cfa4-route` mögött; a CI FFmpeg 9.0.2-je külön orákulum |
| GStreamer | a kiadott csomagok GStreamerrel épülnek (alapból az Ubuntu rendszercsomagja); elkülönített `cfa4-gstreamer-runtime` csak akkor, ha egy funkció újabb verziót kér (ADR 0002, ADR 0005) |
| `gyroflow-core` (K13) | kell |
| Hálózati render (K14) | több gép és több GPU engedélyezett; csak renderelő kliens is készül |
| Fordítómodell (K6) | nyitott, kereskedelmi használatot is engedő modell (jelölt: Opus-MT/Marian magyar–angol, licencellenőrzés után), amelyet a felhasználó a K18 letöltővel tölt le; a CFA4 nem szállít modellt (S2, 2026-10-30; III. pont E4). A felhasználó cserélheti. SeamlessM4T és NLLB nem (CC-BY-NC). Külső fordítószolgáltatás: Subtitle Studio D3 |
| Hardveres kódolás (K11) | a gyártói meghajtót a felhasználó telepíti, hiány esetén telepítési link |
| SQLite-nyilvántartás (K17) | kell |
| Online források (K18) | fontos |
| Qt6-idővonal | **Qt Widgets marad** (#48, #63); a Kdenlive QML-idővonala csak referencia, átvenni a `.ui` fájlokat és a modell/nézet mintát érdemes |

### 12.1 A CFA4 MI-döntései és a terv (ellenőrizve 2026-10-25, frissítve 2026-11-03)

| Döntés | Forrás | Hol van a tervben |
|---|---|---|
| **Minden MI-funkció csak az MI-réteg megfelelő kapuja után kerül be**; a réteg terve az ADR 0006 (CFA4-AI_modules, 2026-10-30). Kivétel: K2 (ADR 0006 F0) | tulajdonos, 2026-10-27; ADR 0006 | 11.3, 11.5 |
| Az MI alapvetően helyi, a felhasználó adatai nem hagyják el a gépet | VISION.md 2.3 | minden MI-teendő (K2, K4, K6, K26–K35) helyben fut |
| Modell CPU-n, bármely gyártó GPU-ján vagy NPU-n; NVIDIA-út csak gyártófüggetlen párral | VISION.md 2.4, 5. | K4, K13, K32 |
| Minden modell licence és forrása rögzítve; letöltés csak a felhasználó tudtával | VISION.md 5. | K18 (jóváhagyás, licenc, hash), K31 |
| Kötelező CPU-tartalék; kivétel csak a valós idejű MI-előnézet | ADR 0004 (#55), 1. döntés | K32, K35 |
| MI-futtatók sorrendje: ONNX Runtime (F2–F4) → candle (F5) → közös PyTorch (F6); NPU: OpenVINO. A Burn szerepe (ADR 0004 2. fázis) az ADR 0004 és 0006 összehangolásán múlik (13. pont) | ADR 0006 6. pont; ADR 0004 | K4, K32 |
| A CFA4 nem szállít modellt; a fordítómodellt is a felhasználó tölti le, a betöltött modell licence látszik, nem kereskedelmi licencre figyelmeztet | S2 (2026-10-30); a 2026-10-25-i „csomagban egy modell” döntést felváltja | K6, K28 |
| SeamlessM4T, NLLB, MMS-súlyok kizárva (CC-BY-NC) | licencellenőrzés, 2026-10-25 | K6, K27 |
| Az MI ugyanazokat a parancsokat hívja, mint az ember, előnézettel és visszavonhatóan; idővonal-módosítás csak emberi jóváhagyással | 11.1, 11.2 (FA3-tervekből) | K2, K4, K6, K26–K35 |
| Előbb stabil kézi szerkesztés, csak utána MI | VISION.md 6.; FA3-terv | sorrend: K1, K3 és a nem MI-s pontok az MI-réteg és az MI-pontok előtt |
| Whisper a kiadott buildekben (ma ki van kapcsolva) | `docs/gaps.md` G19 | K2 |

A VISION.md 5. pontjának MI-irányai közül ezek **nem Kdenlive-eredetűek**, ezért ebben a tervben
nincs saját teendőjük; a `docs/gaps.md` G12 és a VISION.md követi őket: szinkron (fordított szöveg
helyi gépi hangon, szájmozgáshoz időzítve), beszédtisztítás, objektumkövetés, médiakeresés tartalom
szerint, automatikus képkivágás (Auto Reframe), hangtípusok címkézése. Meglévők: jelenetfelismerés,
színillesztés, szövegalapú vágás, Kokoro-82M felolvasás.

**Eldöntve azóta:** a csomagba nem kerül MI-modell (S2, ADR 0006); a külső fordítás csak
kapcsolható, külön folyamatként (Subtitle Studio D3). **Nyitott (VISION.md 8.):** szabad-e
kivételesen, kifejezett engedéllyel egyéb felhős MI-szolgáltatást használni.

## 13. Más terveket érintő teendők

Ezek nem ennek a tervnek a feladatai, de az ellentmondások csak velük együtt szűnnek meg. Mindegyik
a saját tervében, külön tulajdonosi döntéssel készül.

| # | Hol | Teendő |
|---|---|---|
| T1 | `docs/kdenlive-parity.md` (mindkét repó) | A terv elfogadása után egy sorra cserélendő, amely erre a tervre mutat, vagy törlendő |
| T2 | Subtitle Studio-terv | Az SBV (K30) felvétele a formátumlistába; a K27–K29 részleteinek átvétele, ahol hiányoznak |
| T3 | ADR 0006 (CFA4-AI_modules) | A „0004 (FFI-kivétel elve) és 0005 (`cfa4-route`)” hivatkozás ütközik a fő repó ADR 0004 (útvonalválasztó) és ADR 0005 (médiahátterek) számozásával; javítandó. A Burn (ADR 0004 2. fázis) és az ADR 0006 futtatósorrendje összehangolandó (E9) |
| T4 | ADR 0006 helye | A tulajdonosi szabály szerint terv csak a fő repó `docs/plans/` mappájában lehet; az ADR 0006-nak is ott kell lennie, vagy a szabályt az ADR-ekre pontosítani kell |
| T5 | `docs/ai/hang-besorolas.csv` | A K31 modelljeinek (`medium`, `large-v3`, `large-v3-turbo`) felvétele |
| T6 | CFA4-AI_modules régi másolatai | A fő repóból 2026-10-25-én másolt 18 elavult fájl törlése vagy hivatkozásra cserélése |
| T7 | CFA4-EVENT-FABRIC-001 | Egyik tárolóban sincs; ha létezik, a `docs/plans/`-ba kell tenni, különben a hivatkozás megszűnik (D4) |
| T8 | ADAPTATION-001 | Ez a terv felváltja; a .docx vázlatként archiválható |
| T9 | CFA4-SMART-CUT-RENDER-R1 | A 175. sor („MLT integráció esetén…”) és az 562. sor („MLT-adatfolyam illesztési pontjainak vizsgálata”) átírása: az MLT csak tesztorákulum; esetleges későbbi háttérként az ADR 0005 4. pontja szerint, a `cfa4-route` alatt (E11) |
| T10 | ADR 0003 (A5 és a `cfa4-restricted-extras` szakasz), ADR 0002 7. pont, ADR 0005 FFmpeg-szakasz, AGENTS.md | Az x264, x265 és LAME a tulajdonos 2026-11-03-i döntése szerint nem a CFA4 csomagjából jön, hanem az Ubuntu tárolójából, a felhasználó telepíti. A `cfa4-restricted-extras` saját csomag helyett az Ubuntu csomagjai (pl. `gstreamer1.0-plugins-ugly`, `libx264`, `libx265`, `libmp3lame`) kerülnek a leírásba. Az ADR 0002 7. pontja pontosítandó: az `ugly` elemeit a CFA4 nem szállítja és nem követeli meg, de ha a felhasználó telepítette, a router használhatja. A `/opt/cfa4` FFmpeg-build x264/x265 nélkül marad, ezért ezek az útja a rendszer GStreamere (K62) |

## 14. Hogyan frissítsd ezt a fájlt

- Ha egy ❓ sort kipróbáltál, cseréld ✅, 🟡 vagy ❌ jelre, és írd mellé a parancsot vagy issue-t.
- Ha a Qt6-felületen elkészül egy terület, jelöld külön (pl. „✅ motor, ✅ Qt6”).
- Kdenlive-kód átvételekor a sor mellé írd a `docs/licensing.md` §5 bejegyzését.
- Új Kdenlive-kiadásnál olvasd át a kiadási jegyzeteket.
- Másik tervre áthelyezett pontot itt ne bővíts; a változás a célterv dolga, itt csak a hivatkozás frissül.
- Minden K-pont előtt alkalmazd a IV. pont kapuját.

## Források

- Kdenlive kiadási jegyzetek: 26.04.0 (2026-04-27), 26.08.0 (2026-08-31) — kdenlive.org
- Kdenlive háttéreltávolítás (SAM2), 25.04 — kdenlive.org, 2024-12
- CFA4: `VISION.md`, `AGENTS.md`, `docs/licensing.md`, ADR F8, ADR 0003, ADR 0004, ADR 0005, `docs/gaps.md`,
  `crates/engine/src/*` parancs-azonosítók; S1–S6 (II. pont)

## Változásnapló

| Dátum | Változás | Összefoglaló |
|---|---|---|
| 2026-10-24 – 2026-10-30 | — | Az előzmények a két `docs/kdenlive-parity.md` változásnaplójában (S1, S2) |
| 2026-11-03 | major | FINAL 1.0: S1, S2 és S3 összefésülése; ellentmondások feloldása (E1–E11); feliratpontok hivatkozássá alakítása; MI-pontok az ADR 0006 kapuihoz; K36–K41 és D1–D6 a forkokból; végrehajtási kapu |
| 2026-11-03 | minor | E11: MLT-döntés rögzítve (saját motor, MLT később opcionális háttér az ADR 0005 4. pontja szerint); T9: Smart Cut terv két MLT-sorának javítása |
| 2026-11-03 | minor | 11.7: MLT nélkül pótlandó elemek (A lista: MLT-szolgáltatások) és az eddig számozatlan ❓/❌ sorok (B lista), K42–K61, kb. 136–260 óra |
| 2026-11-03 | minor | x264, x265, LAME: a CFA4 nem szállítja, a felhasználó az Ubuntu tárolójából telepíti (tulajdonosi döntés); K62, T10 |
| 2026-11-03 | minor | K43 (Lottie/Glaxnimate) a főcímgenerátor eleme (tulajdonosi döntés) |
| 2026-11-03 | minor | K60 átírva: többmonitoros munka az első, SDI/Blackmagic később opcionális |
| 2026-11-03 | major | Jóváhagyva a tulajdonos által; E10 megerősítve; a 11.7 pontjai később jönnek; a korábbi Kdenlive-tervek elavultak (T1, T8) |
