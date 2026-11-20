# CFA4-SHOTCUT-CAPABILITY-ADAPTATION-001: F0-audit és végrehajtható terv

**Állapot:** F0-audit kész; a terv tulajdonosi átnézésre vár (SC-4 és a V1-bővítés jóváhagyva 2026-11-20; a `docs/plans/` mappába másolást és a V1 átvezetését István 2026-11-20-án engedélyezte). Ez a dokumentum **nem indít el semmit**: a megvalósítás (SC-1…SC-4) külön engedélyt igényel.
**Dátum:** 2026-11-20 · **Auditált állapot:** `ubuntuokos/CinemaFundArchitecture4` `main` @ `2140a51` (2026-11-20) · **Módszer:** csak olvasás (sekély klón egy ideiglenes mappába, kódkeresés, a `docs/` átnézése, a nyitott PR-ok és ágak címeinek átfutása).
**Nyelv:** magyar. A Shotcut forráskódját ebben az auditban **nem** vizsgáltam; a Shotcut-képességek a megbízás F1–F13 leírásából valók.

---

## 0. Röviden (az eredmény)

A 13 jelölt közül **csak 4 kis hiány maradt**; a többi vagy már megvan a CFA4-ben, vagy jóváhagyott terv fedi, vagy elvetendő duplikáció.

| Státusz | Jelöltek |
|---|---|
| **EXISTING** (kész, kóddal igazolva) | F1, F2, F7, F8 |
| **EXISTING + APPROVED-PLAN-COVERED** (a maradék is tervben van) | F3, F4, F13 |
| **APPROVED-PLAN-COVERED** | F5 (bővíthetőség), F6 |
| **EXISTING / REJECTED** (a maradék elvetendő) | F12 |
| **PARTIAL** (elkülönített hiány marad) | F10 (2 hiány), F11 (1 kis hiány) |
| **MISSING** | F9 (csatornánkénti hullámforma) |
| **BLOCKED** | nincs |

**Az ellenőrzött hiánylista (ez kerül a specifikációs szakaszba):**

| Azonosító | Hiány | Forrás-jelölt | Prioritás | Becslés* |
|---|---|---|---|---|
| **SC-1** | Vágókeret (crop) fogantyúi közvetlenül a Program monitoron | F10 | A | 10–20 ó |
| **SC-2** | Fehéregyensúly-pipetta a Lumetri Temperature/Tint mezőihez | F10 | A | 8–14 ó |
| **SC-3** | Csatornánkénti (L/R, többcsatornás) hullámforma-megjelenítés | F9 | C | 12–24 ó |
| **SC-4** | Több effekt/klip egyszerre ki-be kapcsolása a kijelölésen + „legutóbb használt effektek” lista | F11 | D (P3, jóváhagyva 2026-11-20) | 6–12 ó |

\* A becslések az enyémek, nem mértek (a CFA4 a becsléseit „Opus 5.5 ügynökórában” adja, `docs/roadmap.md`). Összesen kb. **36–70 óra**.

**Tulajdonosi döntés (István, 2026-11-20):** az **SC-4 bekerül a tervbe**, és az **F6/V1 elfogadási feltételei bővülnek** a régióelőnézettel, a szerkeszthető kijelöléssel törlés előtt és a kötegelt elemzéssel (lásd 5.2). A V1 szövegét a `docs/venturi-parity.md`-ben ugyanabban a PR-ban átvezettük (V1 5. pont).

**Nem tudtam ellenőrizni (részletek a 9. pontban):** a gitignore-olt, gép-lokális `plan/` mappa (STATUS.md, execution-plan.md) tartalmát, a CFA4-AI_modules külön repót, a CFA3 eredeti terveit (csak egy tervezet és idézetek vannak a repóban), valamint hogy a Qt6 idővonal valójában hogyan rajzolja a klipmarkereket.

---

## 1. Shotcut → CFA4 képességösszevetés (1. szállítandó)

Az „Állapot” oszlop az F0.4 szerinti besorolás. A bizonyíték fájl- vagy parancsazonosító-szintű; a részletek a 2–3. pontban.

| # | Shotcut-képesség | CFA4 állapot | Döntés |
|---|---|---|---|
| F1 | Slip & slide | **EXISTING** | kizár |
| F2 | Multicam | **EXISTING** | kizár |
| F3 | Médiakezelés, archiválás | **EXISTING + APPROVED-PLAN-COVERED** (K22, K23, K45, K46) | kizár |
| F4 | Interoperabilitás, validálás | **EXISTING + APPROVED-PLAN-COVERED** (K1, K42, K20, K25, E13) | kizár |
| F5 | Többsávos és általános átmenetek | **APPROVED-PLAN-COVERED** (ADR 0003 A4/G11) + **REJECTED** (a többsávos rész) | kizár |
| F6 | Csendfelismerés és -törlés | **APPROVED-PLAN-COVERED** (V1; `transcript.removePauses` kész) | kizár; V1-kiegészítés javasolt |
| F7 | Klipmarkerek, zászlók | **EXISTING** (a „zászló/státusz” rész **REJECTED**, duplikáció) | kizár |
| F8 | Poszterkocka | **EXISTING** | kizár |
| F9 | Csatornánkénti hullámforma | **MISSING** | **SC-3** |
| F10 | Képernyőn lévő vezérlők (VUI) | **PARTIAL** | **SC-1, SC-2** |
| F11 | Szűrő-munkafolyamat | **PARTIAL** (K55 tervben van) | **SC-4** (kicsi, jóváhagyva) |
| F12 | Beágyazott projektek | **EXISTING** (egyprojektes beágyazás) + **REJECTED** (külső élő hivatkozás) | kizár |
| F13 | RAW / CinemaDNG | **EXISTING + APPROVED-PLAN-COVERED** (PROCODEC-002 §11) | kizár |

---

## 2. A CFA4 meglévő megvalósítása (2. szállítandó)

### F1 Slip & slide: EXISTING
- `crates/edit/src/lib.rs`: `slip` (1008. sor), `slide` (1022.), `slide_items` (1084.); a tesztje `crates/edit/src/tests.rs` `roll_slip_slide` (a slip a média elején lefog; a slide a szomszédot igazítja), valamint `slide_keeps_reversed_neighbours_content`.
- Parancsok: `tool.slip`, `tool.slide`, `timeline.slip`, `timeline.slide`, `timeline.slipLeft/Right(5)`, `timeline.slideLeft/Right(5)` (`crates/engine/src/commands.rs`, `keyboard.rs`).
- Idő: `Tick`-alapú, tehát képkocka-pontos. Az undo a parancsrendszeren át megy. A GUI-oldal: `docs/qt-parity.md` 3.14 (Tools) ✅. A Kdenlive-terv 2. táblája is ✅-nek jelöli.
- **Nem ellenőriztem:** külön mentés/újratöltés tesztet a slip/slide eredményére (a modell a klip In/Out-ja, így a szokásos projektmentés viszi, de dedikált teszt létezését nem találtam).

### F2 Multicam: EXISTING
- `crates/engine/src/multicam.rs` (fejléc: `clip.createMulticam`, `multicam.switchAngle/cut/recordStart/recordStop`, `multicam.cutToCamera1..9`, `multicam.editCameras`, `multicam.gridLayout`, `clip.multicamFlatten` …) és `crates/engine/src/sync.rs`.
- Szinkron: `in`, `out`, `timecode`, `marker` és `audio` (hullámforma-keresztkorreláció, `cfa4_audio_dsp::sync`), plusz kézi `offset` (frame). Tesztek: `multicam_tests.rs` (`clip.synchronize` audio és timecode módban).
- Forrásfájl nem módosul (a multicam forrás-szekvencia); a szerkesztés utólag átírható (`multicam.switchAngle`). Qt6: `docs/qt-parity.md` 4.12 ✅.
- Ismert kisebb hiány, **más terv már fedi:** `multicam.transmitView` csak tárol, kimenet nincs (**K60**).

### F3 Médiakezelés és archiválás: EXISTING
- `crates/engine/src/project_manager.rs`: **Collect Files** és **Consolidate and Transcode**, száraz futás lemezhely-jelentéssel, a projektfájl a végén, a kimenetek ujjlenyomatával; nested szekvenciákat követi.
- `crates/engine/src/relink.rs`: `MediaIdentity` (méret + első/utolsó MiB hash), hiányzó média felismerése nyitáskor (`media.findMissing`), `media.relink`, `media.autoRelink` (mappaleképezés), keresés. Tesztek: `relink_tests.rs`, `project_manager_tests.rs`.
- Nem fedett maradék → **mind jóváhagyott tervben van** (Kdenlive-terv, `docs/plans/CFA4-KDENLIVE-PLAN-001-FINAL-1.0.md`): zip/tar archív **K45**; hash a médiahivatkozásban és forráscsere **K22**; projektellenőrző (hiányzó források, integritás, függőségi jelentés) **K23**; távoli erőforrás figyelmeztetés **K46**.

### F4 Interoperabilitás és validálás: EXISTING + tervben
- OTIO: `file.exportOtio`, `crates/interchange/src/otio.rs`, a Kdenlive OTIO-tesztkészlete fut (`tests/kdenlive_otio.rs`); terv: `docs/plans/OTIO-INTEGRATION-2026-10-24.md`.
- CMX3600 EDL import **és** export: `crates/interchange/src/edl.rs` (`looks_like_edl`), `file.exportEdl`, `file.import` felismeri. További: FCP7 XML, FCPXML, AAF, OMF.
- Importjelentés: az `interchange` `Report` típusa (átalakított/nem támogatott elemek).
- MLT XML olvasás → **K1** (`.kdenlive`), **K42** (MLT-szolgáltatás → parancs leképezés, nem támogatott = figyelmeztetés), **K20**, **K25**. **A tulajdonos döntése (2026-11-03, E10/E11): MLT-be/Kdenlive-be exportálás nem lesz, az MLT csak tesztorákulum.** Az F4 „MLT XML kompatibilitás/validálás” ezért az import oldalon tervben van, az export oldalon elvetett.
- Szerkezeti validálás: **E13** (`cfa4-cli project schema/validate`, `docs/open-editors-parity.md`) és **K23**. Helyreállítás: `file.recover`, `autoSave.*` kész.

### F5 Átmenetek: tervben + elvetett rész
- Saját átmenetmotor (`crates/render/src/transitions/`, ~30 átmenet, `transitions.inspect`, `sequence.applyVideoTransition`); a bővíthetőség (OpenFX-host, Natron openfx-misc első csomagnak) **ADR 0003 A4 / G11** terve (`docs/adr/ADR-0003-aswf-and-conversion-sources.md`).
- A „többsávos átmenet”: a CFA4 Premiere-modellje a sávok egymásra rétegezésével, átlátszósággal és keveréssel komponál; külön „sávok közötti átmenet” objektumot **nem találtam**, és hozzáadása párhuzamos átmenetmotort jelentene. **Besorolás: REJECTED** (nem ellenőrzött Shotcut-oldali részletek miatt újranyitható, ha a tulajdonos kéri).

### F6 Csend: tervben
- Kész: `transcript.removePauses` (átirat alapján), QC-ellenőrzés `audio.silence` (`crates/qc`, csak elemzés), `crates/speech/src/vad.rs` (energiaalapú).
- **Jóváhagyott terv:** `docs/venturi-parity.md` **V1** (jóváhagyva 2026-10-27): Silero VAD, `audio.detectSpeech` és `clip.removeSilence` (küszöb, minimális szünet, megtartott ráhagyás `Tick`-ben, átirat nélkül). Kapcsolódó: Kdenlive-terv **K41** (MI-szerkesztési munkafolyamatok, pl. csendvágás, E1-en át).

### F7 Klipmarkerek: EXISTING
- Modell: `crates/project/src/lib.rs` `Marker { id, start, duration, name, comment, kind, color }`; `markers: Vec<Marker>` a `MediaClip`-en és az idővonali klipen is (a szekvencia-markertől külön).
- Parancsok: `markers.markClip`, `markers.edit`, `markers.filterColors`, `markers.list`; klipcímkék: `edit.label.*`, `labels.*`. A sync `marker` módja is klipmarkert használ.
- A „zászló/státusz-kategória” külön objektum **duplikáció** (színcímke + markerszín + megjegyzés + szűrés már lefedi) → REJECTED.
- **Nem ellenőriztem:** hogy a Qt6 idővonal kirajzolja-e a klipmarkereket (`docs/qt-parity.md` 3.6 a szekvencia-markerekről szól).

### F8 Poszterkocka: EXISTING
- `clip.setPosterFrame` (Cmd+P) és `clip.clearPosterFrame` (Alt+P), `crates/engine/src/keyboard.rs` (`set_poster`): képkockára igazít, a klip metaadatában tárolja (`POSTER_FRAME_KEY`), egy visszavonási lépés, a forrásmédia érintetlen; belső kulcsként rejtett (`project_panel.rs` `INTERNAL_KEYS`).
- Qt6: `python/cfa4_qt/panels/project.py` (kártya-miniatűr poszterkockával, „Set Poster Frame” a helyi menüben).

### F9 Hullámforma csatornánként: MISSING
- A Qt6 idővonal a `media.peaks` lekérdezést hívja: `{item, start, end, buckets, stream}`; a válasz **egy (min,max) pár vödrönként** egy streamre (`python/cfa4_qt/panels/timeline_media.py`, `_peaks_done`). Csatornaválasztó vagy csatornánkénti paraméter nincs. `clip.audioChannels` csatornaleképezés, nem megjelenítés.
- **Nem tudtam ellenőrizni**, hogy az engine a csatornákat összekeveri-e: a `media.peaks` kezelőjét nem találtam a parancsregiszter szövegei között, ezért az SC-3 első lépése ennek azonosítása.
- Közeli, **de más** tervek: **K59** (hullámforma beágyazott szekvenciákon), **K8** (spektrogram-panel); a CFA3-tervezet *Hang-2* (Audacity-szerű munkaterület, lásd 4. pont) csak tervezet.

### F10 VUI: PARTIAL
- Megvan (Qt6, `python/cfa4_qt/panels/program_overlay.py`, engine `program.overlay` lekérdezés): transzformdoboz (pozíció, méretezés fogantyúkkal, **Uniform Scale** = képarány-zár, forgatás, horgonypont), maszkfogantyúk és toll, mind egy visszavonási lépés/húzás (`effects.setParam` `merge` kulccsal). Pipetta színparaméterre: `effects.eyedropper` (`crates/engine/src/eyedropper.rs`; az Ultra Key kulcsszín-választást lefedi). Qt6: `docs/qt-parity.md` 3.9, 3.12 ✅.
- **Hiányzik** (nem találtam sem a Qt6-ban, sem az egui `transform_handles.rs`-ben, sem terven): (a) vágókeret-fogantyúk a monitoron (a Crop csak számmező a Properties panelben, 3.22); (b) fehéregyensúly-pipetta (a Lumetri `temperature`/`tint` csúszkák léteznek, `lumetri.py`; pipetta nincs).
- Nem hiány: a G8 (`docs/gaps.md`) „on-screen transform handles (#639)” a Qt6-ban elkészült; ez az audit a maradékot veszi.

### F11 Szűrők: PARTIAL
- Megvan: `effects.apply` több klipre (`clips: [..]`), `effects.toggleEnabled` (egy klip, egy index), presetek, Paste Attributes, `effects.presetGrid`.
- Tervben van: az effektek ki-be kapcsolása az **egész idővonalon** → **K55**.
- Nincs és nincs terven: több effekt/klip egy lépésben kapcsolása a kijelölésen, „legutóbb használt effektek”. Jelentéktelenebb → SC-4, P3, **tulajdonosi döntés**, hogy kell-e.

### F12 Beágyazott projekt: EXISTING + REJECTED
- A CFA4 beágyazása **szekvenciákkal** működik egy projekten belül: `clip.nest`, `sequence.nestSequences`, `sequence.makeSubsequence`, `sequence.revealNested`, `sequence.open`; körkörös beágyazás ellen védelem (`crates/project/src/lib.rs` `nest_cycle`, teszt `nest_cycle_finds_sequences_that_contain_themselves`); a project manager követi a beágyazott szekvenciákat; hullámforma/render beágyazottakon fut.
- Külső `.fcproj`-ra hivatkozó **élő** beágyazott projekt (változások terjedése, gyorsítótár, hiányzó projekt felismerése, újrakötés): nem található, és bevezetése második hivatkozási/frissítési rendszert hozna a szekvencia-modell mellett; a Premiere-célmodellbe sem illik. **REJECTED.**
- **Nem ellenőriztem:** létezik-e „szekvenciák importálása másik projektből” (pillanatkép); ha nincs, ez lehet egy jövőbeli, kicsi jelölt (nem része ennek a tervnek).

### F13 RAW / CinemaDNG: EXISTING + tervben
- `crates/dng` (tiszta Rust, DNG/CinemaDNG dekóder, `develop()` fehéregyensúly/színmátrix paraméterekkel, libraw-orákulum teszt), `crates/media/src/still/dng.rs`: számozott `.dng` mappa **egy klipként** importálódik, a CinemaDNG FrameRate és TimeCodes címkékből képkockasebesség és kezdő időkód (`image_sequence_tests.rs` `import_cinemadng_folder_as_one_clip`).
- Tervben: `docs/plans/CFA4-PROCODEC-002-2026-11-11.md` §11 (11.3 beállítások: debayer, fehéregyensúly, expozíció, kameralog, proxy/teljes felbontás; 11.4 natív út) és `docs/camera-raw.md`. A „Source Settings” parancs ma még „nincs forrásbeállítás” üzenetet ad nyers formátumokra (`project_tools.rs`), ezt az 11.3 oldja fel.

---

## 3. Jóváhagyott tervek fedési mátrixa (3. szállítandó)

Átnézett jóváhagyott dokumentumok: `docs/plans/CFA4-KDENLIVE-PLAN-001-FINAL-1.0.md` (jóváhagyva 2026-11-03), `docs/venturi-parity.md` (V1–V10, 2026-10-27), `docs/open-editors-parity.md` (E1–E15, 2026-10-27), `docs/plans/OTIO-INTEGRATION-2026-10-24.md`, `docs/plans/CFA4-PROCODEC-002-2026-11-11.md`, `docs/gaps.md` (G1–G20), `docs/adr/ADR-0003/0005`, `docs/camera-raw.md`, `docs/plans/CFA4-PARITY-G7-2026-11-20.md`, a többi `docs/plans/*` címei. A Kdenlive-terv IV. pontja szerint csak a `docs/plans/` mappában lévő példány érvényes.

| Jelölt | Fedő terv(ek) | Mit fed |
|---|---|---|
| F1 | – (kész) | – |
| F2 | K60 | második monitoros/átviteli kimenet |
| F3 | K22, K23, K45, K46, K48 | relink+hash, ellenőrző/függőségi jelentés, zip, távoli figyelmeztetés |
| F4 | K1, K42, K20, K25, E13, K23, OTIO-INTEGRATION | MLT XML import + jelentés, séma/validálás, OTIO |
| F5 | ADR 0003 A4 / G11 | OpenFX-host: bővíthető átmenetek |
| F6 | **V1**, K41 | Silero VAD, `clip.removeSilence`, MI-csendvágás |
| F7 | – (kész) | – |
| F8 | – (kész) | – |
| F9 | K59, K8 (csak rokon) | nem fedik a csatornánkénti megjelenítést |
| F10 | G8 | transzformfogantyúk (kész); vágókeret/WB nincs benne |
| F11 | K55 | globális effekt-kapcsolás |
| F12 | – | – |
| F13 | PROCODEC-002 §11, camera-raw.md | RAW-beállítások, adapterek, natív dekóderek |

**Nyitott PR-ok átfutása** (40 db címe): egyik sem fed F1–F13-at. Rokon, de eltérő: #1124 LOSSLESS-MEDIA-TOOLS-001 (veszteségmentes médiaeszközök, tartalmát nem néztem), #886 OSA-PROFILES (szekvenciaprofilok, F4 „profilkompatibilitás” szomszédja), #1404/#1408 (objektumkövetés), #1406/#1414/#1415 (stabilizátor).

---

## 4. CFA3 történeti terv felülvizsgálata (4. szállítandó)

- A **CFA3 eredeti tervei nincsenek a repóban**; csak idézetek: a Kdenlive-terv 11.1 (FA3-VIDEO-EDITOR-001 átvett pontjai: K20–K25, mind K-számot kapott) és hivatkozások a Subtitle Studio-terv/AUDIT/FILE-SERVICES dokumentumokban. Az eredeti szöveget így **nem tudtam lekérni** (F0.3 első pont).
- Az `claude/cfa3-creative-studio-sources` ágon van egy **tervezet**: `docs/plans/CFA4-CFA3-CREATIVE-001.md` (1.2, 2026-11-11, „tulajdonosi jóváhagyásra vár”): **nem jóváhagyott, ezért nem számít fedésnek**, és nincs a `main`-en. Érintés: *Videó-6 „Képességfelmérés: mit tud még két nyílt vágóprogram”* (e terv célja rokon; egyeztetendő), *Hang-2 „Hangszerkesztő”* (hullámforma, spektrum, sávok; az F9-cel részben átfed, 150–280 ó, Qt6-ra vár).
- Következtetés: az F0.3 alapján **egyetlen CFA3-eredetű jelölt sem marad** az F1–F13-ból; az SC-3 és a Hang-2 átfedését a tulajdonosnak/a koordinátornak kell eldöntenie (`docs/coordination.md` §2.8).

---

## 5. Elvetett és duplikált elemek (6. szállítandó)

| Elem | Ok |
|---|---|
| F1 slip/slide | már kész (`crates/edit`, tesztelt) |
| F2 multicam (szinkron, váltás, rács, mentés) | már kész; csak `transmitView` hiányzik → K60 |
| F3 teljes csomag | kész + K22/K23/K45/K46 fedi a maradékot |
| F4 OTIO/EDL import-export | kész; MLT/Kdenlive-**export** a tulajdonos döntése szerint **nem lesz** (E10/E11) |
| F4 MLT import/validálás | K1/K42/K25 fedi |
| F5 általános átmenet-bővíthetőség | ADR 0003 A4/G11 (OpenFX-host) fedi |
| F5 többsávos átmenet | duplikált párhuzamos átmenetmotor lenne; a sávrétegezés ezt már megoldja |
| F6 csend | V1 fedi (**kiegészítés-javaslat lent**) |
| F7 klipmarkerek | kész; a zászló/státusz külön objektum duplikáció |
| F8 poszterkocka | kész (engine + Qt6) |
| F10 transzform/maszk/pipetta/képarány-zár | kész (`program_overlay.py`, `effects.eyedropper`) |
| F11 globális kapcsolás | K55 fedi |
| F11 effektelőzmények/használati statisztika | alacsony érték, nincs terv; csak az SC-4 „legutóbbi” része az SC-4-ben marad (jóváhagyva) |
| F12 külső élő beágyazott projekt | második hivatkozási rendszer a szekvenciamodell mellett |
| F13 teljes RAW-lánc | kész (CinemaDNG) + PROCODEC-002 §11 |

### 5.2 A V1 elfogadási feltételeinek bővítése (jóváhagyva és átvezetve)
A V1 `audio.detectSpeech`/`clip.removeSilence` specifikációja nem írja elő: (1) az észlelt régiók **előnézetét és markerként való megjelenítését**, (2) a régiók **szerkeszthető kijelölését törlés előtt** („nincs automatikus romboló törlés”), (3) opcionális **ripple** és **nem romboló** módot, (4) **kötegelt elemzést**, (5) a Shotcut-szerű elfogadási tesztet (hangszinkron megmarad, undo/redo, render-ellenőrzés). **Döntés (2026-11-20):** az (1) régióelőnézet, (2) szerkeszthető kijelölés törlés előtt és (4) kötegelt elemzés a V1 elfogadási feltétele lesz. A (3) opcionális ripple-törlés és nem romboló mód szintén a V1 elfogadási feltétele lesz (István, 2026-11-20). Az (5) elfogadási teszt (hangszinkron megmarad, undo/redo, render-ellenőrzés) név szerint nem szerepelt a döntésben; a V1 átvezetésekor tisztázni kell. Az átvezetés a `docs/venturi-parity.md` V1 szakaszának új 5. pontjaként megtörtént (István engedélye, 2026-11-20).

---

## 6. Végleges prioritási mátrix (7. szállítandó)

**Elv:** a meglévő CFA4-ütemezés nem változik (`docs/roadmap.md` „Current focus” 1–7: G4, G2, G3, G1/G7, G6, G5, G8). Az SC-tételek **nem kerülnek be az ütemezésbe**; csak a *kapcsolódó* meglévő tétel után indulhatnak, külön engedéllyel.

| Prioritás | Tétel | Mihez kapcsolódik / mi után | Megjegyzés |
|---|---|---|---|
| **A** | SC-1 vágókeret-fogantyúk | G8 (UI-hűség) és az ADR F8 Qt6-lépései után | a transzformdoboz mintáját követi |
| **A** | SC-2 WB-pipetta | SC-1-gyel azonos kapu; a Lumetri panel és az `effects.eyedropper` megléte kell | előfeltétel: ellenőrizni, hogy a Lumetri nem kap-e pipettát a G8/G18 alatt |
| **C** | SC-3 csatornánkénti hullámforma | Qt6 idővonal (ADR F8 5. lépés); egyeztetni K59-cel, K8-cal és a CFA3 Hang-2 tervezettel | |
| **D (P3)** | SC-4 effekt-tömegkapcsolás, legutóbbi effektek | K55 után | jóváhagyva 2026-11-20; indításhoz külön engedély |
| – | V1 elfogadási feltételek bővítése | V1 | jóváhagyva; átvezetés külön engedéllyel |

Az eredeti A–E sorrendből: a Priority B és E (F3, F4, F12, F13) teljesen kiesik, mert tervben/kódban vannak; az F1, F2, F7, F8 (A/D) kész.

---

## 7. Architektúra és függőségi térkép (8. szállítandó)

Minden SC-tétel a meglévő alrendszereken fut; **új motor vagy párhuzamos modul nincs**.

```
Qt6 (python/cfa4_qt)  ──parancsok/lekérdezések──►  crates/engine (parancsregiszter) ──► crates/{project,edit,render,gpu}
        │                                               │
        └ automation id-k (overlay.hit, objectName)     └ cfa4-cli serve / MCP / vezérlőcsatorna ugyanazt éri el
```

| Tétel | GUI | Engine/parancs | Modell/perzisztencia | Feldolgozás | Függ |
|---|---|---|---|---|---|
| SC-1 | `program_overlay.py`: új `crop.*` fogantyúk | `program.overlay` kiegészítés (crop-mátrix); a vágás `effects.setParam`-mal a **meglévő Crop effektre** | nincs új mező (a Crop effekt paraméterei) | a meglévő Crop renderelés | G8 állapota, Qt6 Program monitor |
| SC-2 | Lumetri panel: pipettagomb | új `effects.pickWhiteBalance` (az `eyedropper.rs` mintavételét használja, hőmérséklet/árnyalat számítás), eredmény `effects.setParam`-on át | nincs új mező | a meglévő mintavétel (5×5 átlag) | Lumetri `temp`/`tint` paraméterek |
| SC-3 | `timeline_media.py` peak-cache + klip-rajzolás csatornánként | a `media.peaks` kiegészítése `channels` paraméterrel (a kezelő helyének azonosítása az első lépés) | nincs projektmező; megjelenítési beállítás a munkaterületben/preferenciában | a meglévő dekódolás, csatornánkénti csúcsszámítás (`cfa4-audio-dsp`) | Qt6 idővonal, K59 |
| SC-4 | Effect Controls többszörös kijelölés, legutóbbi lista | `effects.toggleEnabled` kibővítése `clips`/`indices` listával; „recent” a felhasználói preferenciákban | preferencia, projektformátum-változás nélkül | – | K55 |

**Erőforrások és tartalék (6. pont 6–8.):** mind a négy CPU-n fut (nincs GPU-igény); SC-3 megnövelt csúcs-gyorsítótárat igényel (csatornák × vödrök), ezért csatornaszám-korlát és lusta számítás kell; kis erőforrású gépen a csatornánkénti nézet kikapcsolható marad. Hibakezelés: ismeretlen/hiányzó csatorna esetén a mostani egyesített nézet marad (nincs panic, `AGENTS.md` §0).
**Visszafelé kompatibilitás (9.):** egyik tétel sem módosítja a `.fcproj` sémát, így nincs migráció. **Együttélés a Shotcuttal (10.):** a tételek nem hoznak létre fájlt, portot, MIME-t vagy konfigurációt; az XDG-elkülönítést a meglévő `docs/audit/F6-XDG-SOURCE-2026-10-23.md` auditálja (ebben az auditban újra nem ellenőriztem).

---

## 8. Licenc- és donorértékelés (9. szállítandó)

- A CFA4 licence **GPL-3.0-or-later** (`LICENSE`, `docs/licensing.md`). Az `AGENTS.md` CFA4-szakasza a **Shotcutot** is nevesíti mint átvehető GPL-kompatibilis forrást, **fájlonkénti fejlécellenőrzéssel**, az eredeti szerzői jogi fejléc megtartásával és a `docs/licensing.md` §5 átvett-kód-nyilvántartás sorával, ugyanabban a commitban. GPL-2.0-only, AGPL, jogtisztázatlan és zárt kód **soha** nem vehető át.
- A Shotcut maga (megbízás szerint) GPLv3: elvben a CFA4-be kód átvehető, **de nem javaslom**: a Shotcut UI-ja Qt Quick/QML + MLT-szűrőtulajdonságokhoz kötött (C++/QML/MLT), a CFA4 pedig Rust-motor + Python/PySide6 Qt Widgets felület, saját parancs- és időmodellel (`Tick`). A logika átírása kisebb kockázat, mint egy QML/MLT-réteg adaptálása.
- **Javasolt döntés minden SC-tételre: CFA4-natív megvalósítás**, Shotcut csak viselkedési referencia. Donorkód átvétele csak külön F3 (megbízás-szerinti) felülvizsgálat után: pontos fájl, fejléc, SPDX, függőségek licence (`cargo deny check licenses`), karbantartottság, Ubuntu 26.04/amd64, Qt6-illeszkedés, nyelvi átjárhatóság, biztonság.
- Új függőség egyik tételhez sem kell; automatikus telepítés tilos marad.
- **Nem ellenőrzött:** a Shotcut adott fájljainak tényleges fejléce (ehhez nem klónoztam/olvastam a Shotcut forrását).

---

## 9. Mit nem tudtam ellenőrizni (kötelező nyilvánosság)

1. **`plan/` mappa** (STATUS.md, execution-plan.md, architecture.md): a `CLAUDE.md` szerint gép-lokális és gitignore-olt, a repóban nincs. Ha ott van jóváhagyott terv, amely az SC-1…SC-4-et fedi, azt nem láthattam.
2. **CFA4-AI_modules** (ADR 0006/0008) és más külön repók: nincs hozzáférésem (a hatókörön kívül), a K41/V1 MI-függő részeit ezért csak a CFA4-ben lévő hivatkozásokból ismerem.
3. **CFA3 eredeti tervei** (FA3-VIDEO-EDITOR-001 stb.): nincsenek a repóban; csak a Kdenlive-terv 11.1 idézete és egy tervezet az ágon.
4. **Futó viselkedés**: nem építettem és nem futtattam a CFA4-et; minden állítás kód-/dokumentumolvasáson alapul. A GUI-viselkedés (pl. klipmarker-megjelenítés idővonalon, a `media.peaks` csatornakezelése) ezért ellenőrzendő az F1 előtt.
5. **Nyitott PR-ok tartalma**: csak a 40 cím; mélyen nem néztem át.
6. **Shotcut forrása és licencfejlécei**: nem vizsgáltam.
7. A becslések sajátok, nem mértek.

---

## 10. Végrehajtható terv fázisonként (10. szállítandó: implementációs specifikációk)

Minden fázis **külön tulajdonosi engedélyt** igényel. Az ütemezést nem érinti. Minden tételre előzetes kapu: a Kdenlive-terv IV. pontja (a `main` és a nyitott PR-ok újraelemzése, átfedés/elkészültség-ellenőrzés), és csak a `docs/plans/` mappában lévő, jóváhagyott terv érvényes.

### Közös előfeltételek (a Kdenlive-terv és az `AGENTS.md` szerint)
- Tulajdonosi jóváhagyás a tervre, majd a terv bekerülése `docs/plans/CFA4-SHOTCUT-CAPABILITY-ADAPTATION-001.md` néven a `main`-re (**ez repo-módosítás, itt nem engedélyezett**; a tulajdonos engedélye kell).
- Parancsok a `crates/engine` regiszterében, automatizálási azonosító minden új vezérlőn, `Tick`-idő, nincs `f64` másodperc, nincs pánik (`AGENTS.md` §0).
- Kapuk: `cargo xtask ci`, `cargo test --workspace --exclude cfa4-ui --release`, Python: `ruff`, `mypy` (új fájl átmegy), `pytest`; a Qt GUI-tesztek (`test_qt_gui.py`) futnak, de a Qt6-befejezésig nem blokkolnak.

### SC-1 Vágókeret a Program monitoron (A)
- **Követelmény:** a kijelölt klip Crop effektjéhez 8 fogantyú + húzás a keret belsejében; képarány-zár modifier (Shift) és középre húzás (Alt); undo-lépés húzásonként; a keret a `program.overlay` mátrixait használja.
- **Elfogadás:** a húzás ugyanazt a paraméterváltozást adja, mint a Properties Crop számmezői (azonos `effects.setParam` utak); képkocka-pontos állapot-szinkron; egy húzás = egy undo; mentés/újratöltés megőrzi; a renderelt kimenet megegyezik a számmezős beállítással (golden képkocka-összevetés).
- **Tesztek:** engine: `program.overlay` crop-mátrix egységtest; Python: `overlay.hit("crop.handle.<n>")` automatizált húzás (a transzform tesztek mintájára, `python/tests/`); regresszió: transzformdoboz változatlan; CPU-only (a feldolgozás CPU-n is fut); hibakezelés: nem vágható klip/adjustment layer → nincs doboz.
- **Fázisok:** (1) overlay-lekérdezés bővítése, (2) Qt fogantyúk, (3) undo/merge, (4) tesztek+golden. **Becslés:** 10–20 ó.

### SC-2 Fehéregyensúly-pipetta (A)
- **Követelmény:** a Lumetri Basic Correction „WB Selector” gombjával a Program kép egy semleges pontja kijelölhető; az engine kiszámolja a Temperature/Tint értéket és `effects.setParam`-mal beírja.
- **Elfogadás:** semleges szürke mintán a számított Temperature/Tint a vártra hoz (tűréssel); egy undo; az eredmény nem függ a médiától a naplóban (ahogy az `effects.eyedropper` is csak a végeredményt rögzíti); a forráskép mintavétele a Lumetri nélküli képkockáról.
- **Tesztek:** golden szürkekártya-fixture (`target/fixtures/`, generált, nem commitolt); a számítás egységtesztje; GUI: gomb élesítése/Esc; CPU-only.
- **Előfeltétel/ellenőrzés:** megerősíteni, hogy a Lumetri nem kap pipettát más terv alatt. **Becslés:** 8–14 ó.

### SC-3 Csatornánkénti hullámforma (C)
- **Követelmény:** az idővonal-klip és a Source monitor hullámformája választhatóan csatornánként (L/R, többcsatornás), csatornacímkével, állítható magassággal és csatornánkénti nagyítással; az alapértelmezett a mostani egyesített nézet.
- **Elfogadás:** helyes csatornaleképezés (a `clip.audioChannels`/sáv-formátum szerint); stabil rajz különböző nagyításon; a hang nem módosul; a gyorsítótár memóriája korlátozott.
- **Lépések:** (0) a `media.peaks` kezelő és a csatornakezelés azonosítása (nyitott pont, 9.4); (1) engine: `channels` paraméter; (2) Qt: cache és rajz; (3) beállítás; (4) tesztek.
- **Tesztek:** generált sztereó fixture eltérő L/R tartalommal; 5.1 fixture; zoom-regresszió; teljesítménytesztek hosszú klipen; CPU-only.
- **Egyeztetés:** K59, K8, CFA3 Hang-2 tervezet. **Becslés:** 12–24 ó.

### SC-4 Effekt-tömegkapcsolás és legutóbbi effektek (D, jóváhagyva)
- **Követelmény:** `effects.toggleEnabled` több klipen/indexen egy undo-lépésben; legutóbb használt effektek listája a felhasználói preferenciában.
- **Elfogadás:** egy undo; a meglévő effektviselkedés változatlan; K55-tel nem ütközik (a K55 az egész idővonalat kapcsolja). **Becslés:** 6–12 ó. Tervbe vétel jóváhagyva (2026-11-20); indításhoz külön engedély kell.

---

## 11. Teszt- és elfogadási kritériumok (11. szállítandó)

A 8. pont minden kategóriája minden SC-tételre kötelező, kivéve ahol nem alkalmazható (indokkal):

| Kategória | SC-1 | SC-2 | SC-3 | SC-4 |
|---|---|---|---|---|
| Funkcionális | igen | igen | igen | igen |
| GUI (automatizálási azonosítókkal) | igen | igen | igen | igen |
| Idővonal-integritás | igen (vágás nem mozgat klipet) | – | – | igen |
| Projektperzisztencia | igen | igen | beállításként | preferencia |
| Undo/redo | igen | igen | – | igen |
| Médiaintegritás (forrás érintetlen) | igen | igen | igen | – |
| Render/kimenet | igen (golden) | igen (golden) | – | – |
| Regresszió | transzformdoboz, Crop számmező | `effects.eyedropper` | meglévő hullámforma | K55 |
| Hardver / CPU-only | CPU-only | CPU-only | CPU-only | – |
| Teljesítmény/erőforrás | – | – | cache-korlát, hosszú klip | – |
| Hibakezelés, helyreállítás | igen | igen | igen | igen |

**Nincs kész státusz** puszta fordulás vagy a GUI megjelenése alapján: kell a tesztkimenet (CI-napló) és a kimeneti viselkedés ellenőrzése (`AGENTS.md`/`CLAUDE.md`: „bizonyíték, nem állítás”).

---

## 12. Fázis-lezárási bizonyíték (12. szállítandó)

- **F0 (ez a dokumentum):** teljesült: CFA4-megvalósítási audit, jóváhagyott tervek átnézése, duplikátumok kivétele, függőségek és licenckockázat dokumentálva, prioritás az ütemezés érintése nélkül. **Nyitott maradt** a 9. pont tételei, ezért a 11. „Elfogadási feltételek” első három pontja **feltételesen** teljesül: a `plan/` és a külön repók áttekintése nélkül nem állítható, hogy *minden* jóváhagyott terv ellenőrzött.
- **F1–F7:** nem indultak; a végrehajtási kapu (tulajdonosi engedély) hiányzik. Minden fázishoz kötelező lezárási bizonyíték: CI-napló (`cargo xtask ci` + tesztek), a golden összevetés kimenete, a GUI-automatizálási napló, undo/redo és mentés-újratöltés teszt kimenete, regressziós összevetés.

### Reprodukálás (az F0 bizonyítékai)
Forrás: `main` @ `2140a51f987f57de771b7095df79dc5b496358b6`. Parancsazonosítók: `grep -rhoE '"[a-z][A-Za-z0-9]*\.[a-zA-Z0-9.]+"' crates/*/src` (klip: `timeline.slip*`, `timeline.slide*`, `tool.slip`, `tool.slide`, `clip.setPosterFrame`, `markers.markClip`, `media.findMissing`, `media.relink`, `file.exportEdl`, `file.exportOtio`, `clip.synchronize`, `effects.eyedropper`). Kulcsfájlok: `crates/edit/src/lib.rs`, `crates/engine/src/{multicam,sync,project_manager,relink,keyboard,eyedropper,commands}.rs`, `crates/interchange/src/{edl,otio}.rs`, `crates/project/src/lib.rs`, `crates/dng/`, `python/cfa4_qt/panels/{program_overlay,project,timeline_media,lumetri}.py`. Tervek: lásd 3. pont.
