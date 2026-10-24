# Kdenlive-paritás: mit tud a Kdenlive, és mit tud már a CFA4

> **Utolsó átnézés:** 2026-10-24 · **Változás:** major (2. változat: GPL-3.0-or-later, Qt6/PySide6,
> FFmpeg 9 + GStreamer médiaháttér, márkaeltávolítás) · **Cél:** Kdenlive 26.08.2 (2026-10-13) · **CFA4:** `main` @ `bbbefc3`

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
- **Márkák:** a Kdenlive és a KDE neve, logója nem kerülhet a CFA4-be, ahogy az ArtCraft és a
  *Craft márkák is kikerülnek (#41).

### Mit érdemes valóban átvenni

A Kdenlive kódja nem illeszthető be változtatás nélkül: C++/Qt, és az MLT-keretrendszerre épül,
a CFA4 viszont Rust-motor + PySide6-felület. A haszna ezért részenként eltér:

| Kdenlive-rész | Haszna a CFA4-ben | Hogyan |
|---|---|---|
| Qt Designer `.ui` fájlok (párbeszédablakok, beállítások) | **nagy** | A PySide6 közvetlenül betölti vagy Pythonná alakítja (`pyside6-uic`). Fájlonként licencellenőrzés |
| `.kdenlive` / MLT XML olvasó logika | **nagy** (K1) | A logika és a mezőleképezés átírható Rustra (`crates/interchange`). Az MLT maga LGPL-2.1-or-later |
| Algoritmusok (pl. hullámforma, audio-szinkron korrelációval, jelenetfelismerés) | közepes | Átírás Rustra, forrásmegjelöléssel |
| Feliratkezelés, Speech Editor logikája | közepes | Python- vagy Rust-átírás |
| Titler sablonok, render-profilok, luma-fájlok | közepes | Tartalom, nem kód: fájlonként ellenőrizendő licenc |
| Idővonal-felület (QML) | kicsi | A CFA4 Qt Widgetset használ, nem QML-t |

## Jelmagyarázat

| Jel | Jelentés |
|-----|----------|
| ✅ | Megvan a Rust-motorban (parancs-azonosítóval igazolva), és az örökölt egui-felületen |
| 🟡 | Részben van, vagy másképp működik |
| ❌ | Hiányzik |
| ❓ | Nincs ellenőrizve, kipróbálandó |

**Fontos:** a ✅ a Rust-motorra és az egui-felületre vonatkozik. A **Qt6-felület** még csak
fejlesztési prototípus (ADR F8 1. lépés kész, 2–7. nyitott), így a Qt6-ban ma ezek közül
szinte egyik sem érhető el. A Qt6-paritás külön követendő.

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
| Hardveres kódolás AMD-n és Intelen (VA-API) | ❌ | Ma csak NVENC. Elsődlegesen FFmpeg 9 `*_vaapi` ([ADR 0004](adr/ADR-0004-ffmpeg9-primary-backend.md), #53), másodlagosan GStreamer `va*enc` (ADR 0002 3. lépés), gyártófüggetlen terv (#39) |
| Szoftveres HEVC, jobb H.264, MP3 export | ❌ | x265, x264, LAME: ADR 0003 A5, előtte szabadalmi vizsgálat |
| Széles formátumlefedettség | 🟡 | Médiaháttér-alapsorrend: FFmpeg 9 elsődleges (ADR 0004, #53, tervezett `cfa4-ffmpeg`), GStreamer másodlagos (`crates/gstreamer`, #40), saját Rust-kodekek tartalék; feladatonkénti útválasztási táblával (az igazolt formátumok idővonal-lejátszása a saját motorral kezd). A kiadott csomagok GStreamer nélkül épülnek; az FFmpeg 9 szállítása nyitott döntés |

## 8. Platform és felület

| Kdenlive-képesség | CFA4 | Megjegyzés, bizonyíték |
|---|:-:|---|
| Linux, deb, AppImage, Flatpak | 🟡 | Csak Ubuntu 26.04 AMD64; mind a négy formátum tervezett, a kiadás HOLD (F9) |
| Qt-felület | 🟡 | Qt6 Widgets + PySide6 prototípus; egui a működő felület a pontos paritásig (#48) |
| Wayland és X11 | ❓ | VISION.md 3. pont: minden 26.04-es változaton tesztelendő |
| Munkaterületek, billentyűparancsok, témák | ✅ | Motorban és egui-ban; Qt6-ban ❌ |

## 9. Teendők rangsora

A sorrend a VISION.md 6. pontját követi: a Qt6-felület az első, ezért minden felületi munka
közvetlenül Qt6-ban készül, nem egui-ban.

| # | Feladat | Kapcsolódó lépés | Átvehető forrás |
|---|---|---|---|
| **K2** | Whisper bekapcsolása a kiadott buildekben | független, most indítható (G19) | — (kész kód) |
| **K1** | `.kdenlive` (MLT XML) projektimport | motor most; Qt6-menü az ADR F8 3. lépésénél | Kdenlive és MLT olvasó logikája, fájlonként |
| **K3** | Linux-stabilitás, Wayland és X11 minden változaton | ADR F8 5–7. lépés, VISION.md 3. pont | — |
| **K7** | Matematikai kifejezés a számmezőkben | ADR F8 4. lépés (közös Qt6-spinbox) | Kdenlive spinbox-logikája, ha a fejléce engedi |
| **K8** | Hangspektrum- és spektrogram-panel | ADR F8 5. lépés | Kdenlive vagy Audacity-kód, fájlonként |
| **K9** | ASS / SSA felirat | bármikor (`crates/captions`) | Subtitle Edit (GPL-3.0) vagy Kdenlive |
| **K4** | AI háttéreltávolítás (SAM2-szerű) | MI-szakasz | SAM2 (Apache-2.0), helyben, nem csak NVIDIA-n |
| **K5** | frei0r és LV2 effektek külön folyamatban | ADR 0003 A6–A7 | frei0r, lilv |
| **K6** | Feliratfordítás szállítható licencű modellel | MI-szakasz | modellválasztás kell (licenc és helyi futás) |

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

## 10. Hogyan frissítsd ezt a fájlt

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
| 2026-10-24 | minor | Médiaháttér-sorrend az ADR 0004 szerint (#53): FFmpeg 9 elsődleges, GStreamer másodlagos, feladatonkénti útválasztással |
