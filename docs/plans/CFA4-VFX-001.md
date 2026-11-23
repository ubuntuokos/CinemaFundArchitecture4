# CFA4-VFX-001 – VFX és motion graphics: tartalmi réteg a CFA4 render-infrastruktúráján

**Verzió:** 1.1 (végleges) · **Dátum:** 2026-11-22 · a változások: 11. fejezet
**Előzmények:** `CFA4_VFX_EGYESITETT_TERV.md` v0.9.8 (a master- és az After Effects-terv egyesítése) és `CFA4_VFX_OSSZEVETES_A_REPO_TERVEKKEL.md` (összevetés a `main` `3d1615e` állapotával). A bemeneti tervek és a v0.9.8 lépésről lépésre megőrzik a részleteket; ahol ez a terv eltér tőlük, **ez az irányadó**.
**Állapot:** `JÓVÁHAGYOTT` a tulajdonos által, 2026-11-22; a fejlesztés a 5. fejezet sorrendje szerint indul (VFX-0a → 0b → 1; a VFX-2…6 a render-terv függőségeinek készültéig vár; a VFX-8 az X-4/X-5 döntésig áll). A repóba vitel PR-ral történik (`coordination.md` §2.11).
**Névhasználat:** a repó márkaszűrője (`python/tests/test_independence.py`) miatt a szövegben „EC-donor (#11)” és „upstream” szerepel; a márkanevek és a donor-URL-ek nincsenek kiírva. A fájl így közvetlenül a `docs/plans/`-ba vihető.

## 1. Cél és hely a CFA4-ben

A VFX-projekt **nem külön motor, kiértékelő, sor vagy formátum**. A CFA4 meglévő és jóváhagyott infrastruktúrájára épülő **tartalmi réteg**: új szerkesztési képességek, effektek és referencialeltár. Ami már létezik vagy más projekté (4. fejezet), azt nem építi újra (`coordination.md` §2.8).

Kiindulás: `VISION.md` §4 (Resolve: „Fusion-szerű csomópontos kompozitálás és 2D/3D VFX”; Krita: 2D animáció; OpenShot: animált címek) és §8 (a 3D/VFX mélysége még nyitott tulajdonosi döntés).

## 2. Alapszabályok

| Terület | Szabály | Forrás |
|---|---|---|
| Platform, GUI | Ubuntu 26.04 LTS, amd64, Wayland és X11; új felület csak Qt6/PySide6; az egui fagyasztott | `VISION.md` §2–3, `AGENTS.md` |
| **Csak Linux-hatókör** | A donorokból **nem** veszünk át macOS-, Windows-, FreeBSD- és böngésző/web-specifikus részt: sem forráskódot (`cfg(windows)`, `cfg(target_os = "macos")` ágak, platformmodulok), sem csomagolást (`packaging/windows`, `packaging/macos`, `packaging/freebsd`), sem UI-hidat (a Flutter `windows/`, `macos/` részei), sem CI-t, fordítást vagy tesztet ezekre. Portoláskor a platformágakat elhagyjuk, nem portoljuk át „későbbre”. A CFA4 CI-jébe nem kerül macOS- vagy Windows-job; a donor-tesztek csak Ubuntu 26.04 amd64-en futnak | `VISION.md` §2, `.github/workflows/ci-linux.yml` (`platform-scope`), István 2026-11-22 |
| Port vagy marad (`AGENTS.md`, tulajdonosi döntés 2026-11-03) | Az átvett kód alapból Rustra portolódik (EC-donor, Lumit-donor). A C++/Python kód (Gaffer-donor) nem kerül a szerkesztő folyamatába: csak külső, felügyelt workerként, Rust-interfész mögött, a render C0 worker-szerződésén át futhat, és csak ha a referenciagépen mérve nem lassítja a lejátszást és a renderelést. A port-vagy-marad indoklást minden átvételi PR rögzíti | `AGENTS.md` CFA4-szakasz |
| Motor | Rust; minden parancs `Session::execute(id, params)`; idő mindig `cfa4_time::Tick` (254 016 000 000/s) | `CLAUDE.md` |
| Egy projektmodell | Nincs második projektmodell vagy kiértékelő; a kompozíció adat marad | render-terv A1 |
| Kép | Premultiplikált, lineáris fényű RGBA f32 a referencia; CPU-referencia, GPU a CPU-hoz mérve | `docs/architecture.md`, render-terv MQ |
| Hiba | Nincs `unwrap`/`panic`/`unsafe`; minden bemenet ellenséges; mutációs fuzz | `AGENTS.md` §0 |
| Licenc | GPL-3.0-or-later; kód csak GPL-kompatibilis forrásból, fájlszintű fejléc-ellenőrzéssel és `docs/licensing.md` §5 regisztersorral; zárt, GPL-2.0-only és AGPL kód nem; nincs fizetős rész | `AGENTS.md` |
| Adobe | After Effects csak viselkedési referencia (megfigyelés használattal). Nincs Adobe-eszköz, -SDK/UXP-kompatibilitás vagy zárt bővítmény | `AGENTS.md` §1–2, ADR 0003 §4 |
| Donorok | Az upstream ős, az EC-donor, a Gaffer-fork, a **Lumit-donor** (`CFA4_Lumit`, Rust+wgpu, `GPL-3.0-only`) és a Kdenlive-fork **csak olvasott** forrás; a kód a CFA4-be portolódik. A donorok a 2026-11-22-i állapoton befagyasztva maradnak a megvalósítás végéig | István döntése, 2026-10-27 és 2026-11-22 |
| MI | Helyi, opt-in; a modell- és erőforrásszabályok az AI-terv hatásköre. Önálló GPU nélküli AI-út nincs | `VISION.md` §5 |
| Fordítás, közös fájlok | Csak `i18n/*.tsv`; közös fájlokba (effektlisták) csak a saját sor kerül | `AGENTS.md`, `coordination.md` §2.5 |
| Folyamat | Egy PR = egy téma; a `main`-be csak a gazda olvaszt; kész PR „Ready for review”, „Gazda: …” issue; kockázatos PR-on CodeRabbit; `Munkamenet:` sor | `coordination.md` |
| Bizonyítás | `PASS` csak mért eredménnyel; küszöb a mért baseline-ból (*Küszöb@F0*), nem előre választott szám | render-terv F0V |

## 3. Viszony a repó terveihez (X-1…X-8, István utasítására az ajánlások szerint beállítva)

| # | Döntés | Beállított viszony |
|---|---|---|
| **X-1** | Viszony a `CFA4-RENDER-INTEGRATION-PLAN-001`-hez | **A VFX-projekt a render-terv F3–F5 infrastruktúrájára épülő tartalmi réteg.** A `VfxGraph` csak szerkesztési adat az L1-ben (adatot tárol, mint a `LineArt` és az `ObjectTrack`), nincs saját kiértékelő; a fordítást és végrehajtást a render-terv gráfja végzi. Az eredeti D-01 ezzel felváltva. |
| **X-2** | Hogyan kerül a terv a repóba | Ez a fájl `docs/plans/CFA4-VFX-001.md` néven, saját ágon, PR-rel, tulajdonosi jóváhagyás után. Addig vázlat. |
| **X-3** | Katalógus | Egy katalógus: a `docs/audit/render-f0v/F0-capability-gap-matrix.csv` (`G-xx`) kiterjesztése (6. fejezet). Az eredeti D-11 (külön YAML-katalógus) felváltva. |
| **X-4** | A 3D/VFX mélysége (`VISION.md` §8) | A VFX-8 munkacsomag **felfüggesztve a tulajdonos §8-as döntéséig**. Az ajánlott első lépés a render-terv szerinti külső EXR-csere, a natív 3D-szerkesztés később. A `VISION.md`-t ez a terv nem módosítja. |
| **X-5** | Külső 3D-backend | Egy közös EXR/USD-job-szerződés; az első implementáció a render-terv G-26 szerint PASS-minősítésű Blender (külső folyamat), a Gaffer ugyanazon szerződés másik backendje, a render-terv A6/A7/A11 szabályai alatt (külön nyilvántartott Python, host coexistence). |
| **X-6** | `VISION.md` §4 bővítése | Javaslat a tulajdonosnak: az After Effects (viselkedési referencia) és a VFX-donorok felvétele. Amíg nem történik meg, a terv nem hivatkozik a VISION felhatalmazására ezekre. |
| **X-7** | Lottie | Egyetlen gazda: a `CFA4-TITLE-CREDITS-001` főcímgenerátor (Kdenlive-terv K43). A VFX-projekt nem készít Lottie-értelmezőt. |
| **X-8** | WASM-plugin-host | Nem része a tervnek. Ha később szükséges: külön ADR a `cfa4-sandbox` (`CFA4-SECURITY-001`) szabályai szerint. |

### 3.1 Projekthatár-táblázat (mit nem építünk újra)

| Terület | Gazdaprojekt | A VFX-projekt feladata |
|---|---|---|
| Render-DAG, scheduler, cache, CPU/GPU backend, job API | `CFA4-RENDER-INTEGRATION-PLAN-001` (F3, F4, C0) | Igényt ad át; a `VfxGraph`-ot az F3 fordítja |
| OCIO, OpenImageIO, OpenEXR, képszekvencia-I/O, AOV | ADR 0003 A2/A3, render-terv F5 | Függőség; csatorna-igények (depth, motion, cryptomatte) átadása |
| OpenFX-host | ADR 0003 A4 (cél: 2026-11-21) | Nincs; a Natron csak képességminta, `openfx-misc` plugin |
| Objektumkövetés, maszkforrás, objektummaszk, SAM2 | `OBJECT-MASK-PLAN-001`, Kdenlive K4, AI-terv | Csak hiánylista (planar/camera track, finomítás, paint) |
| Szöveg, alakzat, text animator, címek, stáblista, vonalrajz, Lottie | `TITLE-CREDITS-001`, `LINEART-001`, K43, `docs/graphics.md` | Csak a nem lefedett igények (repeater, trim path), egyeztetve |
| Feladatsor, ütemezés, erőforrás-kezelés, render queue | `TASKSTORE-PG-001`, ACS, ADR 0007, Event Fabric, render C0 | Nincs új sor |
| Sandbox | `CFA4-SECURITY-001` (`cfa4-sandbox`) | Szabályokat követ |
| OTIO, EDL, FCPXML, AAF | `OTIO-INTEGRATION` | Nincs; megvan |
| Validációs alap, golden, metrikák | render-terv F0V (`cfa4-golden`, `cfa4-metrics`, `cfa4-qc`, `cfa4-testkit`) | Ezen fut; küszöb: *Küszöb@F0* |
| Smart Render, render bar | `SMART-CUT-RENDER-R1`, `SMART-RENDER` | Nincs |
| Élő grafika | `LIVE-STUDIO-001` (C10) | A közös grafikus réteget nem törheti |
| Kdenlive/MLT | Külön projekt (K1, K20, K43) | Nincs; a K20 táblát a katalógus hivatkozza |
| AI-modellek, model manager | `CFA4-AI_modules` ADR 0008, `AI-MODEL-TRAINING-001` | Csak interfész |
| OpenVDB-adat, volume-előnézet, térbeli kifejezés (kontextus a közös kifejezésmagon) | `CFA4-DNEG-TECHNOLOGY-ADAPTATION-001` (N1–N3) | A kifejezésmagot (VFX-4) adja; a DNEG-munka a mátrixban `DNEG-M1…M3` |
| Reprezentáció-réteg (mesh, pont, volume, görbe, példány, impostor) | `CFA4-ARF-001` | Csak bemenet a VFX-6-nak; nincs második részecskemotor |
| Effektlánc-preset, macro | `CFA4-EFFECTCHAIN-001` | A preset a `VfxGraph` részgráfjává migrálható; a kifejezés a VFX-4 magja |
| Jelenetcsere (glTF/OBJ/STL, natív USD-import) | `CFA4-OPENUSD-3D-001` | VFX-8 külső EXR-csere az alap; a natív import az X-4 döntés után |
| Pontfelhő | `CFA4-POINTCLOUD-001` | Effektek (scatter) csak a VFX-6 sorain |
| EC-donor besorolása (`PASS`/`BLOCKED`/`REFERENCE_ONLY`) | render-terv F1 (EC-jelölt #11 audit) | A VFX-1 csak leltárat és átvételi sorrendet ad, a besorolást nem ismétli |

## 4. Mit épít a VFX-projekt (jogosult hatókör)

1. **Kifejezésmotor** (valueAtTime, wiggle, loop, property-hivatkozás), determinisztikus seeddel, a `cfa4-sandbox` korlátai szerint (nincs fájl- és hálózathozzáférés, idő- és memóriakorlát). Ez a repóban nem létezik.
2. **Szerkesztési adatmodell és csomópont-szerkesztő** (`VfxGraph` L1-adat, Qt6-panel a meglévő munkaterület-rendszerben), a render-terv F3 szerződésére.
3. **EC-donor (#11) és Lumit-donor effektek és modulok átvétele** a meglévő effektregiszterbe és render-pipeline-ra, a render-terv F1/F2 keretében (a korábbi BLOCKED ok, a hiányzó repo-hivatkozás, elhárult).
4. **Részecskék kiterjesztése** a meglévő `particles` effektből (`crates/project/src/effect/vfx.rs`): emitter, erők, ütközés, sprite, checkpoint, seed+Tick determinisztika.
5. **Referencialeltár** az After Effects 26.5 (stabil) és a béta képességeiről, `reference_status = STABLE/BETA`, a `G-xx` mátrix bővítéseként.
6. **3D és külső 3D-backend**: csak a `VISION.md` §8 tulajdonosi döntése után (X-4, X-5).

## 5. Munkacsomagok

Jelölés: a **render-terv fázisai (F3, F4, F5, C0, F9)**, az **OBJECT-MASK** és az **ADR 0003** külön jóváhagyott projektek; a VFX-csomag csak akkor indulhat, ha a megnevezett függősége kész és a tulajdonos jóváhagyta (`coordination.md` §2.10). Minden PR egy téma.

| WP | Munkacsomag | Kimenet | Függőség | Átvételi kapu |
|---|---|---|---|---|
| **VFX-0a** | Donor-pillanatkép rögzítése (csak a Linux-hatókörű részekre; a macOS/Windows részek ki vannak zárva, 2. fejezet): az EC-donor (#11) és a Gaffer-donor a 2026-11-22-i állapoton marad a terv megvalósításának végéig (tulajdonosi döntés; nem frissülnek, a frissítést a tulajdonos jelzi a fejlesztő szálnak). A fork leválasztása nem előfeltétel | rögzítve: donor-SHA, elágazási pont SHA, szülő `LICENSE`/NOTICE | – | az eredet a `docs/licensing.md` §5 regiszterében |
| **VFX-0b** | Donor-licencregiszter és fájlszintű eredet | regiszterfrissítés (PR) | VFX-0a | minden átvett fájlhoz fejléc-ellenőrzés |
| **VFX-1** | Katalógusbővítés (csak Linux-hatókör; macOS/Windows-specifikus effekt-, modul- és tesztrészek nem kerülnek a katalógusba átvételi jelöltként): `G-xx` mátrix + EC-donor és Lumit-donor F1/F2 (modulonként: kifejezés/script, csomópontgráf, roto, követés, Lottie, OFX, 3D-jelenet, AE-import, effektek; a Flutter UI nem jelölt) + AE-referencialeltár | bővített `F0-capability-gap-matrix.csv`, `F1-…` audit | VFX-0b; render F1/F2 | minden sorhoz forrás; a számok (307/306/280/3032/2984) a donor rögzített SHA-ján mérve; nincs duplikátum a `G-xx` sorokkal |
| **VFX-2** | `VfxGraph` L1-adatmodell + parancsok (a névtérről a gazdával egyeztetve) | `crates/project` adat, `crates/engine` parancsok, additív sémaverzió | render C0 (szerződés), F3 (gráf) | régi projekt változatlanul nyílik és renderel (AC-15); ellenséges bemenet nem pánik |
| **VFX-3** | Qt6 csomópont-szerkesztő | panel a meglévő munkaterületben, undo/redo, automatizálási azonosítók, `i18n/*.tsv` | VFX-2 | Qt-teszt, Wayland és X11; stale async válasz nem állít vissza kijelölést |
| **VFX-4** | Kifejezésmotor | determinisztikus motor, sandbox-profil | VFX-2; `cfa4-sandbox` | seedelt `wiggle` azonos képkockát ad lineáris és véletlen elérésnél; fuel/idő/memória korlát; fuzz |
| **VFX-5** | EC-donor effektek átvétele (P0 → P1 → P2) | effektek a regiszterben (csak hozzáfűzés), CPU-referencia, majd GPU | VFX-1, render F3/F4 | golden a F0V-harnessen; `ec.*`-alias, a CFA4-azonosító nem íródik felül; MQ-1…8 |
| **VFX-6** | Részecskék kiterjesztése | bővített `particles` | VFX-1, render F3/F4 | 1000+ frame véletlen elérés, azonos hash; CPU/GPU a F0V-küszöbön |
| **VFX-7** | Hiánylisták átadása | dokumentum a gazdaprojekteknek: OBJECT-MASK (planar/camera track, paint), grafikai tervek (repeater, trim path), K43 (Lottie-export) | VFX-1 | a gazdaprojekt visszaigazolta; nincs párhuzamos kód |
| **VFX-8** | 3D és külső 3D-backend | EXR/USD-job-szerződés, Blender-, majd Gaffer-backend | **tulajdonosi döntés (X-4, X-5)**; render F5, C0 | USD→EXR AOV→CFA4 export; timeout/crash tartalék; A6/A7/A11 |
| **VFX-9** | E2E és kiadási igazolás | a 7. fejezet tesztjei a render-terv F9/AC-kapukon | VFX-2…VFX-6 | `PASS` csak mért eredménnyel |
| **VFX-X1** | Folyamatos: forrás- és referencia-drift | donor-SHA-frissítés, Adobe stabil/béta delta | VFX-1 | nem módosítja automatikusan a stabil baseline-t |

**Sorrend:** VFX-0a → 0b → 1 → (render C0/F3 után) 2 → 3 és 4 párhuzamosan → 5 és 6 (render F3/F4 után) → 7 folyamatosan → 9. A VFX-8 külön, döntésre várva.

## 6. Egységes katalógus (a `G-xx` mátrix kiterjesztése)

**Alap:** `docs/audit/render-f0v/F0-capability-gap-matrix.csv`; mezői: `id, capability, plan_phase, existing_cfa4, evidence, status, measure_or_target, ac, candidate_if_gap`. A VFX-projekt a következő mezőkkel bővíti, a meglévő sorokat nem másolja és nem írja át:

| Új mező | Tartalom |
|---|---|
| `family` | taxonómia-család (T1–T16, 6.1) |
| `source_project`, `source_ref` | donor vagy referencia; SHA vagy `reference_version`, útvonal, licenc-SPDX, `reference_status` (`STABLE`/`BETA`/`NOT_APPLICABLE`) |
| `parity_level` | `NAME_MATCH_UNVERIFIED`, `API_PARITY`, `PARAM_PARITY`, `VISUAL_PARITY_TESTED`, `DIFFERENT`, `UNKNOWABLE` |
| `integration_decision` | a repó szótára (6.2) |
| `owner_project` | melyik projekt gazdája a képességnek (3.1 szerint) |
| `aliases` | régi azonosítók: `ec.*`, AE-neve, K20-azonosító |
| `license_status` | `PENDING`, `APPROVED`, `APPROVED_WITH_NOTICE`, `REJECTED`, `NOT_APPLICABLE` |
| `verification` | CPU, GPU, Qt6, export, E2E: `UNTESTED`/`PASS`/`FAIL`/`NOT_APPLICABLE` + bizonyíték |

Szabály: egy képességnek egy elsődleges megvalósítása van; a név nem kulcs; az azonos nevű eszköz nem ekvivalens, csak render- és paramétertesztet fogadunk el. Az effektszámok csak forrásonként, „bejegyzésszám” néven szerepelhetnek, összeadni nem szabad őket. A 307/306 effekt, a 280 GPU-jelölés és a 3032/2984 paraméter a donor **rögzített SHA-ján futtatott regiszterexportból** mérendő; a bemeneti tervek számai egy másik repóból származtak. A 27 keverési mód a CFA4 GPU-ja (`G-07`); a „38” forrása még ismeretlen.

### 6.1 Taxonómia

T1 VFX alaprendszer · T2 kép és matte · T3 szín és finishing · T4 2D motion graphics · T5 animáció és vezérlés (itt van az expression) · T6 tracking, stabilizálás, roto · T7 warp, deform, paint · T8 részecskék, szimuláció · T9 2.5D/3D · T10 3D lookdev és render · T11 immersive és sztereó · T12 plugin és interchange · T13 lokális AI VFX · T14 legacy és kapcsolódó (audio) · **T15 render, cache, ütemezés** (gazda: render- és TASKSTORE-terv) · **T16 munkaterület, automatizálás, scripting** (gazda: meglévő Qt6/parancsrendszer).

Az AE-01…AE-19 hozzárendelése (elsődleges család → hely): AE-01 T1 → meglévő `Sequence`/`AdjustmentLayer`; AE-02 T5 → VFX-4; AE-03 T1 → meglévő Effect Controls; AE-04 T2 → meglévő kompozitor; AE-05 T2 → VFX-5, MI-rész OBJECT-MASK; AE-06/07 T6 → OBJECT-MASK; AE-08 T4 → grafikai tervek; AE-09 T5 → VFX-4; AE-10 T9 → VFX-8; AE-11 T8 → VFX-6; AE-12 T3 → ADR 0003 A2; AE-13 T15 → render/TASKSTORE; AE-14 T12 → OTIO-terv, K43; AE-15 T12 → ADR 0003 A4; AE-16 T16 → meglévő MCP/vezérlőcsatorna; AE-17 T16 → VFX-3; AE-18 T13 → AI-terv; AE-19 béta → `reference_status = BETA`, P3, VFX-X1.

### 6.2 Státuszszótár (a repó szótárával összehangolva)

| Dimenzió | Értékek |
|---|---|
| Létezés (a repó F0-mátrixa) | `EXISTING`, `TARGET`, `GAP`, `GAP_STATIC` |
| Döntés (a repó D1-je) | `INTEGRATE_COMPONENT`, `ADAPT_PATTERN_ONLY`, `EXTERNAL_ADAPTER`, `RESEARCH_ONLY`, `REJECT`; VFX-kiegészítés: `KEEP`, `FIX`, `PORT_CODE`, `CLEAN_ROOM`, `LEGALLY_EXCLUDED`, `OUT_OF_SCOPE`, `AUDIT_PENDING` |
| Jelölt-besorolás (a repó F1-je) | `PASS`, `BLOCKED`, `REFERENCE_ONLY` |
| Megvalósítás | `NOT_STARTED` → `IN_PROGRESS` → `CI_PASS` → `HW_E2E_PASS` → `RELEASED` (csak mért bizonyítékkal) |
| Forrásaudit | `AUDIT_DONE`, `PRELIMINARY`, `NOT_AUDITED` |

A korábbi AE-szótár (`SAME_VERIFIED`, `MISSING_ADAPTABLE` stb.) a v0.9.8 3.2 leképezése szerint származtatott nézet, nem tárolt mező.

## 7. Elfogadási tesztek

A tesztek a render-terv harnessén és kapuin (F0V, MQ-1…8, AC-01…18) futnak; a VFX-projekt új AC-t nem vezet be, csak forgatókönyveket ad.

| ID | Forgatókönyv | Kimenet | WP |
|---|---|---|---|
| V-E1 | Import → `VfxGraph` egy kliphez (key → matte → glow → grade) → kulcskocka és maszk → mentés, újraindítás → export | CPU/GPU a F0V-küszöbön; ffprobe; projekt-roundtrip | 2, 3, 5 |
| V-E2 | Régi projektek nyitása; késői async válasz; topológia- és paraméter-undo; drag közbeni Esc | változatlan render (AC-15); nincs kijelölés-visszaállítás; pontos `editTick` | 2, 3 |
| V-E3 | Seedelt `wiggle` kifejezés, undo/redo, újranyitás | determinisztikus paraméter és időpont; lineáris és véletlen elérés azonos | 4 |
| V-E4 | Részecskék 300 frame, seek 200→1→300, újraindítás | azonos frame-hash; CPU/GPU napló; memóriaplafon | 6 |
| V-E5 | EC-donor effekt átvétele: ID- és paramétermegőrzés | régi azonosító nem íródik felül; golden; MQ-1…8 | 5 |
| V-E6 | Külső 3D-job: glTF/USD → backend → EXR AOV → CFA4 export; backend hiány, timeout, hibás EXR | minden hibaág kezelt; verziók és SHA-k naplózva (csak X-4/X-5 után) | 8 |
| V-E7 | Több Ubuntu-változat, telepített másik CFA4-komponensek mellett | nincs fájl-, csomag-, port-, desktop-entry ütközés (AC-10) | 9 |

## 8. Kockázatok

| Kockázat | Kezelés |
|---|---|
| Második modell vagy kiértékelő csúszik be | X-1: a `VfxGraph` L1-adat; a `cargo xtask layers` és a gazda-átnézés ellenőrzi |
| A gazdaprojektek menet közben változnak | A VFX-csomag indulása előtt újraellenőrzés a `main`-ből; a WP-k a függőség kész állapotára várnak |
| Azonos név, eltérő viselkedés | `ec.*`-alias, golden, paraméterösszevetés; nincs vak felülírás |
| Licenc/eredet (fork a `storytold`-ból, külső függőségek) | VFX-0a/0b; fájlszintű fejléc; regisztersor minden átvételhez |
| **Lumit-kód licence: `GPL-3.0-only`** a CFA4 `GPL-3.0-or-later` kikötésével szemben | A tulajdonos elfogadta (2026-11-22): a Lumit-ból átvett fájl `GPL-3.0-only` marad, fájlszintű SPDX-fejléccel és regisztersorral; a `GPL-3.0-or-later` kikötés ezekre nem terjed ki. A `docs/licensing.md` §5 regiszterét és a `cargo deny` konfigurációt a VFX-0b frissíti, a csomagolási jogi megjegyzés (`packaging/linux/copyright`) a gazdával egyeztetve |
| A Lumit modulai kiváltják a saját hatókör egyes elemeit | A VFX-1 audit dönt elemenként (átvétel / tiszta szobás / saját építés); a 4. fejezet addig nem szűkül |
| Márkaszűrő | Semleges nevek a repóban; a tesztet a PR előtt le kell futtatni |
| A 3D-döntés késik | VFX-8 külön, a többit nem blokkolja |
| 30-nál több nyitott PR (`coordination.md` §2.6) | Új munka csak a tulajdonos kérésére |

## 8a. Elfogadott egyeztetési döntések (2026-11-22, a `CFA4-DNEG-VFX-EGYEZTETES-001` D-1…D-9 szerint)

D-1 egy katalógus (a `G-xx` mátrix; a VFX-CATALOG-001 szűkül); D-2 egy kifejezésmag, gazdája a VFX-4; D-3 egy worker/job-szerződés (render C0), az N1/N2 saját indítója átmeneti; D-4 USD: külső EXR-csere az alap, natív import feltételes; D-5…D-8 pontosítások (N2-3, „G” névkonvenció, OpenVDB-licenc, DNEG-donorok pillanatképe); D-9 az EFFECTCHAIN név marad. Az X-4 (3D-mélység) **nem** dőlt el: az OPENUSD, ARF, POINTCLOUD és a VFX-8 addig áll.

## 9. Nyitott pontok

**Tulajdonosi döntésre vár:**
1. **X-4:** a 3D/VFX mélysége (`VISION.md` §8). Ettől függ: VFX-8, valamint a `CFA4-OPENUSD-3D-001`, `CFA4-ARF-001` és `CFA4-POINTCLOUD-001` kódot érintő része. Addig ezek állnak.
2. **X-6:** a `VISION.md` §4 bővítése az After Effects-szel és a VFX-donorokkal.

**Egyéb nyitott:**
3. Az elágazási pont SHA az EC- és a Gaffer-donornál (a szülő története nem ellenőrzött; a VFX-0a és VFX-0b dokumentum jelzi).
4. A WP-k indítása a render-terv megfelelő fázisai után, külön engedéllyel (`coordination.md` §2.10).

**Lezárt:** a terv repóba vitele és jóváhagyása (X-2, PR #1531), a donor-pillanatkép (VFX-0a, PR #1538), az egyeztetési döntések D-1…D-9 (8a. fejezet).

## 10. Mit nem állít ez a terv

Egyik szám (307, 306, 280, 3032, 2984, 38) sincs újramérve. A nyitott PR-ok, az AI-modulok repója, a donor-repók tartalma és az elágazási pontok nincsenek ellenőrizve. A tervezett útvonalak és parancsnevek tervek, nem létező kód. A hivatkozott `main`-állapot: `3d1615e`.

## 11. Változásnapló

| Verzió | Dátum | Változás |
|---|---|---|
| 1.0 | 2026-11-22 | Az összevont VFX-terv repóra szabott változata: X-1…X-8 az ajánlások szerint, WP-k VFX-0a…X1, a `G-xx` mátrix kiterjesztése. A tulajdonos jóváhagyta. |
| 1.0a | 2026-11-22 | A Lumit-donor felvétele (csak olvasott; `GPL-3.0-only` fájlok így maradnak); a donorok befagyasztva, a fork leválasztása nem előfeltétel. |
| 1.0b | 2026-11-22 | Csak Linux-hatókör: macOS-, Windows-, FreeBSD- és web-specifikus rész nem kerül át. |
| 1.1 | 2026-11-22 | Egyeztetés a nyitott projektekkel (`CFA4-DNEG-VFX-EGYEZTETES-001`, D-1…D-9 elfogadva): új sorok a projekthatár-táblázatban (3.1), port-vagy-marad szabály (2. fejezet), 8a. fejezet, a 9. fejezet nyitott pontjai (X-4, X-6) frissítve. A donorokhoz csak olvasással nyúlunk. |
