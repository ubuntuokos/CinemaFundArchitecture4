# DNEG-00 – képesség- és tervütközési audit (D1–D9)

> **Állapot:** audit, tervszintű; **nem implementációs engedély** (coordination.md §2.10).
> **Projekt:** CFA4-DNEG-TECHNOLOGY-ADAPTATION-001 · **Dátum:** 2026-11-21
> **Alap:** CFA4 `main` @ `97d7982f` (git fetch, 2026-11-21). Kód nem módosult; csak dokumentáció.
> **A végleges terv:** [`docs/plans/CFA4-DNEG-TECHNOLOGY-ADAPTATION-001-1.0.md`](../plans/CFA4-DNEG-TECHNOLOGY-ADAPTATION-001-1.0.md) (D10, kapuk, elfogadási feltételek).

## 0. Módszer, és ami ebből az auditból hiányzik

Az összevetés a megvalósítottságtól független: a `main`-ben található **tervekhez** (docs, ADR-ek,
`gaps.md`, `VISION.md`) hasonlítottam, a kódot csak annak megállapítására olvastam, hogy egy
képesség már létezik-e, nem azért, hogy egy tervezett képességet „hiánynak” minősítsek.
Minden állítás mellett ott a forrás (fájl vagy parancs). Ami nem ellenőrizhető, az **ELLENŐRIZETLEN**,
és sehol nem számít teljesítettnek.

### 0.1 Kritikus megállapítás: öt hivatkozott terv nincs a `main`-ben

A megbízás ezekre a „korábbi CFA4-tervekre” hivatkozik: **CFA4 VFX Catalog**, **EffectCraft**,
**Adaptive Representation Fabric (ARF)**, **OpenUSD / 3D Scene Exchange / procedurális 3D**,
**Potree-alapú point-cloud donorvizsgálat** (és Gaffer). Ellenőrzés a `main`-en:

```
grep -rliE "EffectCraft|Adaptive Representation|VFX Catalog|Gaffer|Potree|NanoVDB|glTF" \
     --include=*.md --include=*.rs --include=*.py --include=*.toml .   # (vendor/ nélkül)  → 0 találat
grep -rliE "OpenVDB" …                                                  → csak ADR-0003
grep -rliE "OpenUSD|\bUSD\b" …                                          → csak spec-kit/superpowers (nem média-USD)
```

A klónban csak a `main` ág látszik; a nyitott PR-ok (31 db a lekérdezéskor, 2026-11-21) címei között nincs ilyen
(VFX, 3D, volumetrikus, USD, pontfelhő). Következmények:

1. VISION.md §2.6: „minden terv a `main` ágon él”; ami nincs a `main`-ben, azt **nem tekinthetem
   elfogadott CFA4-tervnek**, és a tartalmát sem ismerem. Nem találgatok a tartalmukról.
2. Ahol egy képesség besorolása ezektől a tervektől függne, a besorolás **feltételes**
   (`A?` / `B?`): *ha* a hivatkozott terv létezik és fedi, akkor a képesség A vagy B, és **nem kerül
   a végső scope-ba**. Amíg a tulajdonos nem adja át a terveket (vagy nem kerülnek a `main`-be),
   az érintett képességek **nem kerülnek a végső scope-ba** (D10 §4, kapu G0).
3. A tervek valószínűleg más tárolóban (CFA4-AI_modules, cfa4-sdk, korábbi CFA3/FA3 anyag) vagy
   a tulajdonos gépén vannak. Ezt nem tudtam ellenőrizni (a munkamenethez csak ez a tároló van
   csatolva). **Tulajdonosi teendő:** a hiányzó tervek `main`-be emelése vagy elérhetővé tétele (G0).

### 0.2 Ami a `main`-ben ténylegesen van (a viszonyítási alap)

| Terület | Mi van a `main`-ben | Forrás |
|---|---|---|
| Videóeffekt-katalógus | A „Premiere 26 effektkatalógus” (M5.11) CPU-s megvalósítása: blur, color, distort, glitch, stylize, temporal, lights, denoise, pixelate, text; paraméterek kulcskockázhatók; egy generatív `particles` effekt (magozott részecske-szökőkút, 2D) | `crates/render/src/vfx/*`, `crates/project/src/effect/vfx.rs` (1543 sor), `vfx.rs:509-524` |
| GPU | wgpu 30 kompozitor és effekt-WGSL (gyártófüggetlen) | `crates/gpu/Cargo.toml`, `fx.wgsl`, `composite.wgsl` |
| Plugin-hosting | **Nincs** (G11); OpenFX-host terve: ADR-0003 A4, külön folyamatban, jelölt: Natron openfx-misc; „jelölt ≠ vállalás, külön ADR” | `docs/gaps.md` G11, ADR-0003 §6, §9 |
| ASWF-sor | OpenColorIO A1/A2, OpenImageIO+OpenEXR A3, OpenFX A4; **MaterialX, OSL, OpenVDB, openqmc: A7, „3D/VFX-szakasz, külön tervezendő”**; saját EXR olvasó/író van | ADR-0003 §2, §7 |
| Interchange | EDL, FCP7 XML, FCPXML, OTIO, **AAF import+export (saját, clean-room)**, OMF, ALE; `cfa4-cfb` (MS-CFB), `cfa4-mxf` | `crates/interchange/README.md`, `crates/interchange/src/aaf/mod.rs` |
| OTIO terv | jóváhagyva 2026-10-24; hivatalos OTIO csak tesztorákulum | `docs/plans/OTIO-INTEGRATION-2026-10-24.md` |
| Erőforrás-kezelés | CFA4-ADAPTIVE-CAPABILITY-001 (ACS-0…7) összevetése lezárva; feladatsor/jobtár: CFA4-TASKSTORE-PG-001; útválasztó: ADR-0004 (`cfa4-route`); az ACS-6 (engine → `cfa4-tasks` TaskHost) PR #1471 nyitott | `docs/plans/CFA4-ADAPTIVE-CAPABILITY-001-OSSZEVETES.md`, `CFA4-TASKSTORE-PG-001.md`, ADR-0004 |
| Biztonság | `cfa4-sandbox` (Landlock; B1 kész, folyamatok még nem alkalmazzák) | `crates/sandbox/README.md`, CFA4-SECURITY-001 |
| Adatréteg | PostgreSQL/pgvector terv (Media Intelligence is), v1.0 VÉGLEGES | `docs/plans/CFA4-DATA-PG-001-1.0.md` |
| 3D, pontfelhő, volumetria | **Nincs** terv sem kód. VISION §8: „a 3D és VFX mélysége” nyitott tulajdonosi döntés; §6: 3D/VFX az utolsó új terület | `VISION.md` §4, §6, §8 |

### 0.3 Kötelező kereteket adó szabályok (AGENTS.md / VISION.md)

- **Licenc:** GPL-3.0-or-later; MPL-2.0, Apache-2.0, BSD, MIT beolvasztható, fejlécekkel és
  nyilvántartási sorral (`docs/licensing.md` §3, §5); GPL-2.0-only, AGPL, ismeretlen nem.
- **Port vagy eredeti nyelv** (2026-11-03): C++ marad, ha nem lassítja a CFA4-et, és folyamaton
  kívül, felügyelt workerként fut Rust-interfész mögött; a döntést a bevezető terv rögzíti.
- **Nincs `unsafe`** a termékkódban (kivétel: `crates/platform` és a 2026-11-07-i kivétel ADR-rel).
- **Never crash** (AGENTS §0): hibás VDB/AAF/kifejezés → hiba, nem összeomlás.
- **Gyártófüggetlenség:** CUDA/NanoVDB-CUDA csak AMD/Intel/CPU párral.
- **Fejlesztés csak jóváhagyott terv alapján** (coordination.md §2.10).
- **Projekthatár** (§2.8): ez a terv nem nyúl másik projekt tárgyához (ACS, DATA-PG, SECURITY,
  LIVE-STUDIO, OTIO); átfedést jelez (lásd D1, D9).
- **Nincs automatikus letöltés**; Conda/Miniforge kizárva; csak Linux amd64 / Ubuntu 26.04.

---

## D1 – Átfedési mátrix

Jelölés: **A** már elfogadott CFA4-tervben; **B** meglévő terv valódi bővítése; **C** új, nem tervezett;
**D** donor/referencia, nem kerül be; **E** nem releváns/kizárt. A `?` = feltételes (0.1).
„Hely” = hova illeszkedne; „Dup.” = duplikációs kockázat.

### D1.1 – Technológiák (donorok)

| # | Capability (donor) | Besorolás | Kapcsolódó CFA4-terv | Új érték | Integrációs hely | Dup. | Döntés és indok |
|---|---|---|---|---|---|---|---|
| T1 | **DNEG OpenVDB fork** (`dneg/openvdb`) | **D** | ADR-0003 §2 (OpenVDB = ASWF-projekt, A7) | nincs külön: az ADR az **ASWF upstreamet** nevezi meg | – | magas (fork-függés) | A fork nem kerül be; csak az ASWF upstream OpenVDB a jelölt. A fork→upstream állapot ELLENŐRIZETLEN (D7/D8 elv). |
| T2 | **ASWF OpenVDB core** (ritka voxelrács, I/O, transzform, grid típusok) | **B** (a donor az A7-ben megnevezett, integrációs terv nincs) | ADR-0003 A7 | Volumetrikus adat tárolása/olvasása (`.vdb`): CFA4-ben nincs volume-adat sem terv, sem kód | külön folyamatú worker + média-eszköz típus (D10 N1) | közepes: ne legyen második render/timeline | **Scope** (N1), csak adat+inspekció+renderelt kimenet; a 0.1 szerint ha az ARF-terv már fedi, a mapping csak annak kiegészítése. |
| T3 | **OpenVDB AX** (JIT kifejezésnyelv) | **C** | – (CFA4 kulcskocka-/kifejezésrendszere képkocka-paraméter, nem térbeli; `grep expression` nem ad térbeli kifejezésnyelvet) | Pozíciófüggő, voxel/pont-szintű programozható módosítás | N1 workerben; GUI: a meglévő effekt-szerkesztő panel (Qt6) | alacsony, **de** biztonsági kockázat (LLVM JIT) | **Feltételes scope** (N3): csak sandbox-PoC kapu (G4) után. Alternatíva: Rust-oldali korlátozott interpreter. |
| T4 | **DNEG openvdb_points_dev** (archivált, béta) | **D** | – | A koncepció az upstream OpenVDB `points` moduljába ment át (ELLENŐRIZETLEN, ismeret alapján) | – | magas | Nem kerül be; csak az upstream points modul a jelölt. |
| T5 | **OpenVDB Points** (attribútum, group, iteráció, régiószűrés) | **B?** | ARF (hivatkozott, nincs a `main`-ben) | Típusos attribútumok, point group, régió-lekérdezés, tömörítés | N1 worker; ARF-ra képezve | magas, ha az ARF létezik | **Nincs önálló scope**; D4 szerint csak a *leképezési specifikáció* készül, ha az ARF-terv elérhető (G0). |
| T6 | **NanoVDB** (GPU/CPU csak-olvasható VDB) | **B?** | wgpu-s GPU út (`crates/gpu`), ADR-0004 | gyors, csak-olvasható mintavétel | opcionális gyorsító a workerben | alacsony | **Nincs scope**: a CUDA-út csak vendor-párral lenne engedélyezett (AGENTS); CPU-útvonal az alap. Későbbi, mérésen alapuló döntés. |
| T7 | **DNEG AAF SDK** (`dneg/aaf`) | **D** | OTIO-INTEGRATION; `cfa4-interchange` AAF (clean-room) | nincs: az AAF import/export már megvan | – | magas | **F4 döntés = 3.** (lásd D6). Az SDK forrása nem olvasható (clean-room, interchange README). |
| T8 | **DNEG USD fork** (`dneg/USD`) | **D** | nincs USD-terv a `main`-ben | – | – | magas | Nem kerül be; fork-diff csak a USD-integráció tervének `main`-be kerülése után értelmezhető (D7). |
| T9 | **OpenUSD upstream** | **C?/nem besorolható** | „OpenUSD / 3D Scene Exchange” – hivatkozott, nincs a `main`-ben | 3D jelenetcsere | – | – | **Nincs scope** G0-ig. Nem a DNEG-terv tárgya (a DNEG-fork nem kell). |

### D1.2 – A–E területek (F0 §1.2) képességei

| # | Képesség | Besorolás | Kapcsolódó terv / tény | Új érték (DNEG) | Hely | Dup. | Döntés |
|---|---|---|---|---|---|---|---|
| A1 | VFX-effektkatalógus, effektláncok | **A** | M5.11 katalógus (`crates/render/src/vfx`), G18 | – | – | – | Nem fejlesztési feladat. A „CFA4 VFX Catalog” elnevezésű terv nincs a `main`-ben (0.1). |
| A2 | EffectCraft | **A?** | nincs a `main`-ben | – | – | – | Kimarad (G0). |
| A3 | Procedurális effektek (2D) | **A** | `particles`, glitch, stylize | – | – | – | Kimarad. |
| A4 | GPU/CPU végrehajtás | **A** | wgpu, ADR-0004, ACS | – | – | – | Kimarad (D9). |
| A5 | Kulcskocka (kifejezés: képkocka-paraméter) | **A** | Effect Controls kulcskockák | – | – | – | Kimarad. |
| A6 | **Térbeli** kifejezés | **C** | – | AX (T3) | N3 | alacsony | Feltételes scope. |
| A7 | Kompozitálás, track matte, blend módok | **A** | `crates/render`, `crates/gpu/blend_tests.rs` | – | – | – | Kimarad. |
| A8 | Követés, rotó | **A** | maszkok, tracking, openfx-misc jelölt (ADR-0003) | – | – | – | Kimarad. |
| A9 | Plugin/extension architektúra | **A** | ADR-0003 A4 (OFX-host), G11 | – | – | – | Nem DNEG-kérdés; a VDB-workernek **nem** plugin-host az útja. |
| A10 | 3D modellek, PBR | **A?** | VISION §8 nyitott döntés; nincs terv | – | – | – | Nincs scope (G0, tulajdonosi döntés VISION §8). |
| B1 | OpenVDB / NanoVDB / volume reprezentáció | **B**/**B?** | ADR-0003 A7 (megnevezve); ARF? | VDB-adat | N1 | közepes | T2/T6. |
| B2 | Volume LOD, ARF | **B?** | ARF hivatkozott | – | – | – | Nem veszem fel; D2 §4. |
| B3 | Volumetrikus effektek (füst, köd, felhő, tűz) | **C** (ha VFX-katalógus nem fedi) | `particles` nem volumetrikus | VDB-alapú sűrűségmezők | N2 (renderelt passz → média) | közepes | Csak N1 után, külön terv (kapu G5): nincs scope-ban, mert render-útvonal nincs. |
| B4 | Volume render, volume compositing | **C** | render-integráció: `CFA4-RENDER-INTEGRATION-PLAN-001` (más projekt, nem volumetrikus) | Raymarch/AOV | N2 | magas (render-motor) | A volume-render **workerben** készít EXR/AOV-t, amit a kompozitor *sima médiaként* olvas; a render motorhoz nem nyúl. |
| B5 | Voxel alapú feldolgozás | **C** | – | VDB csomóponti műveletek (CSG, szűrők) | N1/N3 | alacsony | Az N3-hoz kötve. |
| C1 | OpenUSD | **C?** | nincs a `main`-ben | – | – | – | G0. |
| C2 | glTF/OBJ/STL | **E/C?** | nincs terv; a DNEG-tervnek nem tárgya | – | – | – | Nem DNEG-képesség: kimarad. |
| C3 | Kamerák, hierarchia, animáció, proxy | **A?/C?** | nincs | – | – | – | Kimarad. |
| C4 | Render pass / AOV | **B** | EXR olvasás van (saját), többrészes/mély EXR: ADR-0003 A3 | EXR-AOV kimenet a workerből | N2 | alacsony | A kimenet az A3-ra támaszkodik: **függőség**, nem új képesség. |
| D1 | Pontfelhő, spatial index, octree, streaming | **A?/D** | Potree-donorvizsgálat hivatkozott, nincs a `main`-ben | – | – | – | Nem veszem fel. |
| D2 | Particle rendszer | **A** (2D `particles`), **A?** (ARF) | `vfx.rs:509` | – | – | – | Új, párhuzamos particle engine nem készül (12. kizárás). |
| D3 | Point attribútum/group/iteráció | **B?** | ARF? | OpenVDB Points | T5 | magas | Csak specifikáció (D4). |
| D4 | Region loading, partial access | **B?** | ARF? | – | – | – | Nem veszem fel. |
| E1 | Timeline, trackek, médiareferencia, metaadat | **A** | M3 szerkesztőmag | – | – | – | Kimarad. |
| E2 | Interchange (EDL/FCP7/FCPXML/OTIO/AAF/OMF/ALE) | **A** | `cfa4-interchange` | – | – | – | Kimarad. |
| E3 | AAF import/export | **A** | `crates/interchange/src/aaf` | – | – | – | D6. |
| E4 | Médiaformátumok | **A** | G10, codec-parity | – | – | – | Kimarad. |
| E5 | Erőforrás-alkalmazkodás (CPU/GPU/RAM, cache, LOD) | **A** | ACS-0…7, TASKSTORE-PG-001, ADR-0004 | – | – | – | **Nincs új rendszer**; D9: a worker feladattípusként regisztrál. |
| E6 | Media Intelligence térbeli metaadat | **B?** | DATA-PG-001 (pgvector) | VDB metaadat-kulcsok | – | alacsony | Későbbi, a DATA-PG-001 projektjében; nem vállalom. |

**Átfedés más projekttel (jelzés a §2.8 szerint, a döntés a tulajdonosé/gazdáé):** (1) ACS/TASKSTORE –
a worker-feladattípusok regisztrálása az ő felületüket használja; (2) CFA4-SECURITY-001 – az AX-hez a
`cfa4-sandbox` profil szükséges, amelyet a folyamatok még nem alkalmaznak (B3/B4 függőség); (3)
ADR-0003 A3/A7 – az EXR-AOV és a 3D/VFX-szakasz tervét az A7 külön tervezésére bízta; ez a terv az A7
első, szűk szeletét részletezi, nem írja felül.

---

## D2 – OpenVDB core: tényleges új képességek

Amit a `main` tervei nem fednek (feltéve a 0.1 szerinti hiányzó terveket nem pótolja ARF):

1. `.vdb` fájl **beolvasása és validálása** (grid-lista, típus, voxel-méret, transzform, metaadat, aktív voxel-szám, bbox) – *inspekció*, hibás fájl → hiba.
2. **Grid → renderelt kép/AOV** (sűrűség-előnézet, rétegek) CPU-n, workerben; kimenet EXR/PNG sorozat, amit a CFA4 ismert médiaként használ.
3. **VDB-írás** (round-trip) és **egyszerű műveletek** (transzform, szűrés, kombinálás – csak ha az N3-mal együtt jóváhagyott).
4. **Időbeli sorozat** (`name.####.vdb`) leképezése a meglévő képsorozat-importra: a CFA4 *nem kap* új idővonalat.

**Nem új (elvetve):** teljes volumen-motor, a VFX-rendszer VDB-re építése, minden média VDB-vé alakítása,
kötelező GPU/CUDA, a render motor cseréje (DNEG-terv 2.5, 12. kizárás).

**Leképezés (CFA4 Volume ↔ VDB Grid):** a CFA4 projektmodell az elsődleges; a VDB csak *külső fájl*,
amelyre a projekt hivatkozik (út + hash), és a worker olvas. A CFA4 `Volume` objektum a projektben
nem VDB-struktúra. A mezők (spacing, transform, grid type, density/temperature/velocity/color,
custom attribútum) a worker JSON-jelentésében szerepelnek, és read-only metaadatok. (Konkrét séma:
D10 N1.)

## D3 – AX: hiányzó funkciók

| Hiány | Tény | Következmény |
|---|---|---|
| Térbeli, pozíciófüggő kifejezés | a CFA4-ben csak idő/képkocka-függő paraméter és kulcskocka van | AX valódi új képesség (C) |
| Voxel/pont attribútum-programozás | nincs voxel/pont adatmodell (D2, D4) | AX csak N1 **után** értelmezhető |
| JIT-biztonság | LLVM-JIT kódgenerálás; CFA4 §0 + SECURITY-001 | Kötelező: külön folyamat + Landlock/seccomp-profil + idő- és memóriakorlát (G4 PoC); a `cfa4-sandbox` jelenleg még nincs folyamatra alkalmazva |
| LLVM-függőség (Ubuntu 26.04) | ELLENŐRIZETLEN (csomagnév/verzió hálózat nélkül nem állapítható meg) | D8: telepítési felmérés a G4 előtt |
| AX állapota/licence | az upstream ASWF-projektben él; a DNEG-repó karbantartási módban (a megbízás állítása); licenc: ELLENŐRIZETLEN | D8 |
| Alternatíva | Rust-oldali, korlátozott (nem JIT) kifejezés-interpreter a közös adatkörnyezetre | G4 döntési pont: AX-worker vs. saját interpreter; nincs döntés ebben a tervben |

## D4 – Points: attribútum, group, iteráció, régió, memória

A DNEG-megbízás az ARF/point/Potree terveket „már lefedettnek” mondja, de azok nincsenek a `main`-ben.
Ezért **nem fogalmazok hiánylistát**; a következőt rögzítem:

- A jelöltképességek (típusos attribútum, group, szűrt iteráció, régió-betöltés, delay-load, tömörítés)
  mind **OpenVDB Points**-eredetűek, és **B?** besorolásúak.
- Teendő G0 után: *leképezési táblázat* ARF-attribútumok ↔ OpenVDB Points; a hiányzó sorok adják a D4-et.
- Amíg ez nincs, a Points-képesség **nincs a végső scope-ban**. A VDB-workerben a `points` modul
  csak *olvasható/inspekciós* (az `.vdb`-ban lévő pontrács jelentése) lehet, mert az `.vdb` fájl
  tartalmazhat ilyet, és a worker nem hibázhat rajta (never crash).

## D5 – VFX integrációs térkép

| Új képesség | CFA4-rendszer, amelyhez kapcsolódik | Módszer (nincs új szerkesztő/timeline/projektmodell) |
|---|---|---|
| VDB-megjelenítés (N2) | Effect/Media import, kompozitor, EXR AOV | A worker képsorozatot/EXR-AOV-t ír a projekt gyorsítótárába; a CFA4 azt **ismert médiaként** importálja (`crates/render` változatlan) |
| AX-effekt (N3) | Effect Controls, kulcskocka, effekt-preset | Egy effekt-szerű bejegyzés paraméterekkel (kifejezés-szöveg, kötött attribútumok); a kulcskockázható numerikus paraméterek a meglévő keyframe-rendszerben maradnak |
| Volume/point kimenet kompozitba | track matte, blend, alpha | Az EXR/PNG sorozat sima videó-/kép-klip |
| Háttérfeldolgozás, preview-minőség | ACS TaskHost (D9) | Worker-feladattípus; nincs új ütemező |
| Qt6 | `python/cfa4_qt` | Minden UI-elem a Qt6 GUI-ban (AGENTS), automation-id-vel, parancson át (engine command) |

Modulhatárok (DNEG-terv 7.6): nincs önálló editor, timeline, projektmodell; nem duplikálja az
effektkatalógust vagy a render-interfészt. **Teszt:** `cargo xtask layers` az új crate-ekre.

## D6 – AAF-rés audit

**Tény (a `main`-ből):** a CFA4-nek saját, clean-room AAF import és export van (`cfa4-interchange::aaf`,
`cfa4-cfb`), a következő szabványokból: AMWA AAF Object Spec v1.1, Low-Level Container Spec v1.0.1,
Edit Protocol v1.0, MS-CFB. Az README rögzíti, hogy **az AAF SDK nem volt felhasználva**.

| Terület (DNEG-terv §5.2) | CFA4-állapot | Rés? |
|---|---|---|
| Projekt-metaadat, composition, sequence, track, clip, timecode (A, C) | export+import (timecode slot, source clip, filler, átmenet, marker, nested sequence) | nincs rés |
| Média-referencia, fájl-SourceMob, tape-SourceMob, relink (B) | MasterMob/SourceMob, network locator, beágyazott PCM | relink-UX: más (importer) rétegben, nem AAF-rés; ELLENŐRIZETLEN, külön nem vizsgáltam |
| Import: felismerés, metaadat, unsupported-jelentés (D) | `Report` minden irányban; hostile-tesztek (`aaf/hostile_tests.rs`) | nincs |
| Export: unsupported-jelentés (E) | `Report`; dokumentált nem-képviselt elemek | **ismert, dokumentált korlátok:** sebességváltás és fagyasztás (100%-ként + jelentés), grafika és szintetikus média (gap), az átmeneteken kívüli videóeffekt, OMF-videó és -marker |

**Döntés (F4 kapu): 3. lehetőség** – a CFA4 korábbi tervei és kódja már fedik az AAF-igényt; **nem
készül új AAF-integráció, és a DNEG AAF SDK nem kerül be.** A felsorolt korlátok csendes veszteségei
a jelentésben látszanak; ha valamelyik (pl. sebességváltás AAF-ben) bővítendő, az Premiere-paritás
(`gaps.md`), nem DNEG-munka.

**Licenc/SDK-audit:** a DNEG AAF-repó és az AAF SDK licence **ELLENŐRIZETLEN** (nincs hálózat);
nem tekintem GPL-kompatibilisnek. **Clean-room-védelem:** az SDK-forrás olvasása a `cfa4-interchange`
README-jével ütközne, ezért **nem olvasható**. Megengedett: a kész SDK/`pyaaf2` jellegű eszköz
**külső tesztorákulumként** (futtatható program, nem forrás), ha a licencét előbb ellenőrizzük
(a hivatalos OTIO-hoz hasonló, ADR-0003 §2.1 szerinti mód). Ezt az OTIO-terv A1-fázisának mintájára,
külön, kicsi PR-ban lehet bevezetni; ebben a tervben **nincs** vállalva.

**Tesztstratégia (ha az orákulum jóváhagyást kap):** meglévő `aaf_omf` tesztek + oda-vissza egy külső
olvasóval + hostile-fuzz (`aaf/hostile_tests.rs`). Új AAF-kód nem készül.

## D7 – USD differenciális audit

**Hálózat nélkül ellenőrizhető:**
- A `main`-ben **nincs USD-terv és USD-kód** (grep, 0.1) → a „CFA4 korábbi OpenUSD-terve” nem alap.
- ADR-0003 nem sorolja a USD-t az ASWF-projektek közé (az OpenUSD nem ASWF-projekt a listában).
- Az `upstream OpenUSD` lecserélése DNEG-forkra a megbízás kizárása (12.), ezt átveszem.

**Nem ellenőrizhető (ELLENŐRIZETLEN – nem teljesített):**
fork-állapot, upstream-verzió, commit-különbségek, DNEG-specifikus feature-ök, függőségek,
build-mód, Linux-kompatibilitás, licenc, karbantartás, downstream-kompatibilitás.
Ezek mind a `dneg/USD` és `PixarAnimationStudios/OpenUSD` repók elérését igénylik; a munkamenethez
ezek nincsenek csatolva, és az API 403-at adott.

**Döntés:** a USD-ből **nincs scope**. A differenciális audit akkor fut le, ha (a) a USD/3D-jelenetcsere
terve a `main`-be került (G0), és (b) hálózati hozzáférés van; a „Gate G7” addig zárva. A D7 „diff
report” táblázata (`DNEG delta | upstreamben van | CFA4-ben tervezett | egyedi | adaptálható`) ezért
**kitöltetlen**, nem „üres = nincs eltérés”.

## D8 – Licenc és függőségek

Hálózat nélkül **semelyik donor licence nem ellenőrizhető a forrásfájlból**, ezért a következő
táblázat *nem* teljesített audit; minden sor **ELLENŐRIZETLEN**, a megadott érték a megbízás/ADR állítása.

| Donor | Állított licenc (forrás) | Ellenőrzés állapota | CFA4-szabály szerinti következmény |
|---|---|---|---|
| ASWF OpenVDB | megbízás: Apache-2.0; ADR-0003: „BSD-3 / Apache-2.0”; **saját emlékem szerint MPL-2.0** | **ELLENŐRIZETLEN és ELLENTMONDÁS** a két állítás és az emlékem között | A `LICENSE` fájl olvasása kötelező (G1 belépési kapu). MPL-2.0 megengedett (licensing.md §3: „MPL files stay MPL; a módosítások közzététele”). Apache-2.0 is megengedett. Az ADR-0003 táblázatsora javítandó, ha eltér (külön, kis PR). |
| OpenVDB AX | megbízás: upstream ASWF; DNEG-repó MPL-2.0 | ELLENŐRIZETLEN | fájlonkénti fejléc-ellenőrzés a felhasznált fájlokon; LLVM licenc (Apache-2.0 WITH LLVM-exception, ismeret) is ellenőrizendő |
| OpenVDB Points | megbízás: DNEG-repó MPL-2.0, archivált | ELLENŐRIZETLEN | a DNEG-repó nem használandó; upstream modul |
| DNEG AAF SDK | – | ELLENŐRIZETLEN | nem GPL-kompatibilisnek tekintendő, míg nincs igazolva; forrás nem olvasható |
| DNEG USD fork | – | ELLENŐRIZETLEN | nincs scope |
| NanoVDB | az OpenVDB része (ismeret) | ELLENŐRIZETLEN | – |

**Transzitív függőségek (OpenVDB, ismeret alapján, ELLENŐRIZETLEN):** Boost, oneTBB, Blosc, zlib,
Imath, (AX:) LLVM; Ubuntu 26.04 csomagnevei/verziói **nem állapíthatók meg** hálózat nélkül.

**CFA4-szabályok az integrációra (rögzítve):**
- Függőség csak az Ubuntu-tárolóból (rendszercsomag, `Depends`/`Recommends` a `.deb`-ben; AppImage/Flatpak: külön audit, packaging), **pinelve** és `cargo deny`-vel/SBOM-mal auditálva (`docs/audit/F1-01-SBOM-*`).
- A build nem tölthet le semmit; nincs Conda/Miniforge; nincs Windows/macOS ág.
- Reused-code register sor (`licensing.md` §5) minden átvett fájlra; kód átvétele **nincs** a tervben: a worker rendszer-OpenVDB-t **dinamikusan linkel**, Rust-interfész mögött.
- Védjegy: „DNEG” és „OpenVDB” nevek a UI-ban nem védjegyhasználatként (a VISION: „a nevüket nem veszi át”).

**Karbantarthatóság:** DNEG-forkok (openvdb, openvdb_ax, openvdb_points_dev, USD, aaf) – a megbízás szerint
maintenance/archivált; ez **nem** ellenőrzött. Ezért egyikre sem épül a CFA4.

## D9 – Erőforrás-alkalmazkodás

**Nincs új erőforrás-rendszer.** A meglévő keret (ACS-0…7, TASKSTORE-PG-001, ADR-0004,
`cfa4-tasks` TaskHost [ACS-6: PR #1471 nyitott]) fedi: CPU-only, többszálúság, memóriakeret,
prioritás, háttérfeladat, megszakítás, ütemezés, becslés, ellenőrzőpont.

A DNEG-eredetű képességek ehhez **csak** a következőket adják hozzá, a worker-manifestben:

| Mező | Tartalom |
|---|---|
| feladattípusok | `vdb.inspect`, `vdb.render`, (feltételesen) `vdb.ax` |
| útvonalak | CPU (kötelező, mérten validált); GPU csak ha a használt technológia támogatja, gyártófüggetlen párral; **CUDA nem kötelező** |
| memóriakeret | a worker a kapott keretet tartja (régió/csempe szerinti feldolgozás, OOM-védelem); a túllépés `oom` hiba, nem összeomlás |
| minőségi szintek | előnézeti felbontás, voxel-felbontás, cache-méret – **csak az eredményt nem változtató automatikus lépés megengedett; minőségcsökkentéshez jóváhagyás** (ACS-összevetés, 3. ütközés) |
| megszakítás | szakaszhatáron; AX-nél időkeret + kemény kill |
| mérés | a referenciagép 512 GiB RAM-ja elfedné a memórianyomást: minimum-gép mérés `cgroup`-korláttal (`reference-machine.md`), CPU-only: mindkét NVIDIA-kártya elrejtve |

Átfedés jelzése: a feladatregisztráció az ACS/TASKSTORE-projektek felületére épül; **nem nyúlok** az
ő kódjukhoz (§2.8). Az ACS-6 (PR #1471) beolvasztása **függőség** az N1 implementációjához (G3).

---

## Nem teljesített / ellenőrizetlen tételek jegyzéke

| ID | Tétel | Miért nem teljesített | Mi oldja fel |
|---|---|---|---|
| U1 | VFX Catalog, EffectCraft, ARF, OpenUSD/3D-csere, Potree-terv tartalma | nincs a `main`-ben | G0: a tulajdonos beemeli/elérhetővé teszi |
| U2 | OpenVDB / AX / Points / NanoVDB licenc és verzió | nincs hálózat; az állítások ellentmondanak | G1: `LICENSE`-fájl és csomaginfo olvasása a G1 előtt |
| U3 | DNEG-fork állapotok (karbantartás, archiválás, delta az upstreamhez) | nincs hálózat | opcionális, mert a forkok nem kerülnek be |
| U4 | USD fork-diff (D7 minden eleme) | nincs hálózat; nincs USD-terv | G7 |
| U5 | AAF SDK licence/dependency | nincs hálózat | külön PR (orákulum), ha kell |
| U6 | Ubuntu 26.04 csomagok (libopenvdb, LLVM, TBB, Blosc) | nincs hálózat a csomagforráshoz | G1 felmérés egy 26.04 gépen/konténerben |
| U7 | AX sandbox-PoC | nincs implementáció | G4 |
| U8 | Teljesítmény- és memóriamérések | nincs implementáció | G2–G5 |
