# DNEG-00 – képesség- és tervütközési audit (D1–D9)

> **Állapot:** audit, tervszintű, a tulajdonos jóváhagyta (2026-11-21 22:49); **nem implementációs engedély** (coordination.md §2.10).
> **Projekt:** CFA4-DNEG-TECHNOLOGY-ADAPTATION-001 · **Dátum:** 2026-11-21
> **Alap:** CFA4 `main` @ `97d7982f` (git fetch, 2026-11-21). Kód nem módosult; csak dokumentáció.
> **Hálózati ellenőrzés:** 2026-11-21 (a tulajdonos jelezte, hogy a hálózat elérhető). A DNEG- és upstream-repók shallow klónjai, licenc- és fejlécolvasás, `git rev-parse` összevetés; az eredmények a D7/D8-ban és a jegyzékben. Az AAF SDK **forrását nem olvastam** (clean-room), csak a licencszöveget és a README-t.
> **[MÓD-5] Egyeztetés (2026-11-22, jóváhagyásra vár):** [`CFA4-DNEG-VFX-EGYEZTETES-001`](CFA4-DNEG-VFX-EGYEZTETES-001-0.1.md) – donor-újraellenőrzés, OpenVDB-licenc verziónként (v10.0.1/v11 MPL-2.0, v12.0.0-tól Apache-2.0), a 26.04-es csomagok és a CFA4-VFX-001-gyel való ütközések. A jelölt sorok a jóváhagyásig nem hatályosak.
> **A végleges terv:** [`CFA4-DNEG-TECHNOLOGY-ADAPTATION-001-1.0.md`](CFA4-DNEG-TECHNOLOGY-ADAPTATION-001-1.0.md) (D10, kapuk, elfogadási feltételek).

## 0. Módszer, és ami ebből az auditból hiányzik

Az összevetés a megvalósítottságtól független: a `main`-ben található **tervekhez** (docs, ADR-ek,
`gaps.md`, `VISION.md`) hasonlítottam, a kódot csak annak megállapítására olvastam, hogy egy
képesség már létezik-e, nem azért, hogy egy tervezett képességet „hiánynak” minősítsek.
Minden állítás mellett ott a forrás (fájl vagy parancs). Ami nem ellenőrizhető, az **ELLENŐRIZETLEN**,
és sehol nem számít teljesítettnek.

### 0.1 Kritikus megállapítás: öt hivatkozott terv nincs a `main`-ben

A megbízás ezekre a „korábbi CFA4-tervekre” hivatkozik: **CFA4 VFX Catalog**, **FX Authoring**,
**Adaptive Representation Fabric (ARF)**, **OpenUSD / 3D Scene Exchange / procedurális 3D**,
**Potree-alapú point-cloud donorvizsgálat** (és Gaffer). Ellenőrzés a `main`-en:

```
grep -rliE "FX Authoring|Adaptive Representation|VFX Catalog|Gaffer|Potree|NanoVDB|glTF" \
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

4. **Frissítés (2026-11-21, tulajdonosi döntés):** a hiányzó tervekhez *új terv készül*, a `docs/plans/` alatt (javaslatok, jóváhagyásra várnak): [`CFA4-VFX-CATALOG-001`](CFA4-VFX-CATALOG-001-0.1.md), [`CFA4-FX-AUTHORING-001`](CFA4-FX-AUTHORING-001-0.1.md), [`CFA4-ARF-001`](CFA4-ARF-001-0.1.md), [`CFA4-OPENUSD-3D-001`](CFA4-OPENUSD-3D-001-1.0.md), [`CFA4-POINTCLOUD-001`](CFA4-POINTCLOUD-001-0.1.md). Amíg ezek nincsenek jóváhagyva, a D1 `?` sorai nem zárulnak (a besorolás nem változik).

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
| T1 | **DNEG OpenVDB fork** (`dneg/openvdb`) | **D** | ADR-0003 §2 (OpenVDB = ASWF-projekt, A7) | nincs külön: az ADR az **ASWF upstreamet** nevezi meg | – | magas (fork-függés) | A fork nem kerül be; csak az ASWF upstream OpenVDB (14.0.0, Apache-2.0) a jelölt. A fork 2020-12-i, MPL-2.0 pillanatkép (igazolt, D8). |
| T2 | **ASWF OpenVDB core** (ritka voxelrács, I/O, transzform, grid típusok) | **B** (a donor az A7-ben megnevezett, integrációs terv nincs) | ADR-0003 A7 | Volumetrikus adat tárolása/olvasása (`.vdb`): CFA4-ben nincs volume-adat sem terv, sem kód | külön folyamatú worker + média-eszköz típus (D10 N1) | közepes: ne legyen második render/timeline | **Scope** (N1), csak adat+inspekció+renderelt kimenet; a 0.1 szerint ha az ARF-terv már fedi, a mapping csak annak kiegészítése. |
| T3 | **OpenVDB AX** (JIT kifejezésnyelv) | **C** | – (CFA4 kulcskocka-/kifejezésrendszere képkocka-paraméter, nem térbeli; `grep expression` nem ad térbeli kifejezésnyelvet) | Pozíciófüggő, voxel/pont-szintű programozható módosítás | N1 workerben; GUI: a meglévő effekt-szerkesztő panel (Qt6) | alacsony, **de** biztonsági kockázat (LLVM JIT) | **Feltételes scope** (N3): csak sandbox-PoC kapu (G4) után. Alternatíva: Rust-oldali korlátozott interpreter. |
| T4 | **DNEG openvdb_points_dev** (archivált, béta) | **D** | – | A koncepció az upstream OpenVDB `openvdb/points/` moduljában él (igazolt: a klónban megvan; a DNEG-repó README-je deprecated-nek jelöli) | – | magas | Nem kerül be; csak az upstream points modul a jelölt. |
| T5 | **OpenVDB Points** (attribútum, group, iteráció, régiószűrés) | **B?** | ARF (hivatkozott, nincs a `main`-ben) | Típusos attribútumok, point group, régió-lekérdezés, tömörítés | N1 worker; ARF-ra képezve | magas, ha az ARF létezik | **Nincs önálló scope**; D4 szerint csak a *leképezési specifikáció* készül, ha az ARF-terv elérhető (G0). |
| T6 | **NanoVDB** (GPU/CPU csak-olvasható VDB) | **B?** | wgpu-s GPU út (`crates/gpu`), ADR-0004 | gyors, csak-olvasható mintavétel | opcionális gyorsító a workerben | alacsony | **Nincs scope**: a CUDA-út csak vendor-párral lenne engedélyezett (AGENTS); CPU-útvonal az alap. Későbbi, mérésen alapuló döntés. |
| T7 | **DNEG AAF SDK** (`dneg/aaf`) | **D** | OTIO-INTEGRATION; `cfa4-interchange` AAF (clean-room) | nincs: az AAF import/export már megvan | – | magas | **F4 döntés = 3.** (lásd D6). Az SDK forrása nem olvasható (clean-room, interchange README); licence: AAF SDK PSL 2.0, GPL-3-kompatibilitás nem igazolt (D8). |
| T8 | **DNEG USD fork** (`dneg/USD`) | **D** | nincs USD-terv a `main`-ben | – | – | magas | Nem kerül be; a fork-diff elkészült: a fork = upstream v23.05, delta üres (D7). |
| T9 | **OpenUSD upstream** | **C?/nem besorolható** | „OpenUSD / 3D Scene Exchange” – hivatkozott, nincs a `main`-ben | 3D jelenetcsere | – | – | **Nincs scope** G0-ig. Nem a DNEG-terv tárgya (a DNEG-fork nem kell). |

### D1.2 – A–E területek (F0 §1.2) képességei

| # | Képesség | Besorolás | Kapcsolódó terv / tény | Új érték (DNEG) | Hely | Dup. | Döntés |
|---|---|---|---|---|---|---|---|
| A1 | VFX-effektkatalógus, effektláncok | **A** | M5.11 katalógus (`crates/render/src/vfx`), G18 | – | – | – | Nem fejlesztési feladat. A „CFA4 VFX Catalog” elnevezésű terv nincs a `main`-ben (0.1). |
| A2 | FX Authoring | **A?** | nincs a `main`-ben | – | – | – | Kimarad (G0). |
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
| LLVM-függőség (Ubuntu 26.04) | LLVM ≥ 18 igazolt (OpenVDB `dependencies.txt`); az Ubuntu 26.04-es csomag ELLENŐRIZETLEN | D8: telepítési felmérés a G4 előtt |
| AX állapota/licence | az upstream ASWF-projektben él; a DNEG-repó karbantartási módban (a megbízás állítása); licenc igazolt: Apache-2.0 (upstream `openvdb_ax/`), a DNEG-fork MPL-2.0 | D8 |
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

**Licenc/SDK-audit:** az AAF SDK licence az *AAF SDK Public Source License 2.0* (igazolt a szövegből, D8); a GPL-3-kompatibilitás jogilag **nem minősített**,
ezért nem tekintem GPL-kompatibilisnek. **Clean-room-védelem:** az SDK-forrás olvasása a `cfa4-interchange`
README-jével ütközne, ezért **nem olvasható**. Megengedett: a kész SDK/`pyaaf2` jellegű eszköz
**külső tesztorákulumként** (futtatható program, nem forrás), ha a licencét előbb ellenőrizzük
(a hivatalos OTIO-hoz hasonló, ADR-0003 §2.1 szerinti mód). Ezt az OTIO-terv A1-fázisának mintájára,
külön, kicsi PR-ban lehet bevezetni; ebben a tervben **nincs** vállalva.

**Tesztstratégia (ha az orákulum jóváhagyást kap):** meglévő `aaf_omf` tesztek + oda-vissza egy külső
olvasóval + hostile-fuzz (`aaf/hostile_tests.rs`). Új AAF-kód nem készül.

## D7 – USD differenciális audit (hálózati ellenőrzéssel, 2026-11-21)

**Módszer:** `dneg/USD` és `PixarAnimationStudios/OpenUSD` shallow klón; a DNEG-fork `HEAD` commitját és az upstream `v23.05` tag commitját `git rev-parse`-szal, majd `git diff --stat`-tal összevetve.

| Vizsgálat | Eredmény | Állapot |
|---|---|---|
| Fork állapot | `dneg/USD` `HEAD` = `b53573ea2a6b29bc4a6b129f604bbb342c35df5c`, „Merge release v23.05”, 2023-04-18; egyetlen ág (`release`) | **IGAZOLT** |
| Upstream-verzió | az upstream `v23.05` tag commitja **azonos** (`b53573ea…`); az upstream `release` ága 2026-11-18-ig halad („Add sparse mung buffer locking debug output…”) | **IGAZOLT** |
| Commit-különbségek | a fork `HEAD`-je és az upstream `v23.05` közt `git diff --stat`: **üres** (0 fájl) | **IGAZOLT** |
| DNEG-specifikus módosítás | **nincs** a fa szintjén (azonos a v23.05-tel). A „dneg” szó csak upstream-dokumentációban és teszt-baseline fájlokban fordul elő | **IGAZOLT** (a fa; a fork 2023-ig tartó előtörténete nem vizsgált: a shallow klón nem tartalmazza) |
| DNEG-egyedi feature | **nincs** | **IGAZOLT** a fára |
| Függőség, build | az upstream `build_scripts/build_usd.py`-t használja (letöltő script: a CFA4 szabálya szerint **tilos** automatikusan futtatni) | részben (fájl olvasva, build nem futtatva) |
| Linux/Ubuntu 26.04 build | nem próbáltam | **ELLENŐRIZETLEN** |
| Licenc | `LICENSE.txt`: a fork „Modified Apache 2.0”, az upstream HEAD „Tomorrow Open Source Technology License 1.0” (az Apache-2.0-tól csak a 6. szakasz, a védjegyek, tér el) | **IGAZOLT a szövegből**; a GPL-3 kompatibilitás jogi minősítése: Apache-2.0-szerű, a CFA4 szabálya engedi az Apache-2.0-t; a TOST-1.0 különbségét (védjegy) a tulajdonos hagyja jóvá → **nyitott** |
| Karbantartás | a fork 2023 óta áll; az upstream aktív | **IGAZOLT** |

**Eredmény a D7-táblára:** `DNEG delta` = **üres** (a fork a v23.05 tükre); `upstreamben már elérhető` = minden; `egyedi DNEG-funkció` = **nincs**; `adaptálható` = nincs mit; `nem adaptálható` = a fork mint hordozó (elavult, nincs értéke). **Döntés:** a DNEG USD-fork **nem kerül be és nem is vizsgálandó tovább**; ha a CFA4 USD-t használ, az az upstream OpenUSD (lásd [`CFA4-OPENUSD-3D-001`](CFA4-OPENUSD-3D-001-1.0.md)).

## D8 – Licenc és függőségek (hálózati ellenőrzéssel, 2026-11-21)

Az ellenőrzés a repók `LICENSE`-fájljaiból és forrásfejlécek `SPDX-License-Identifier` sorából történt (a fájlok olvasása; az AAF SDK forrásának olvasása nélkül).

| Donor | Licenc (igazolt, forrás) | Állapot | Következmény |
|---|---|---|---|
| **ASWF OpenVDB** (core, `points`, `nanovdb`, `openvdb_ax`) | **`[MÓD-5]` verziófüggő: v10.0.1 és v11.0.0 = MPL-2.0, v12.0.0-tól Apache-2.0** (a tag-ek `LICENSE`/`SPDX` fejlécei, 2026-11-22; az Ubuntu 26.04-es csomag 10.0.1 → MPL-2.0). A lenti sor a 14.0.0/HEAD állapotot írja le: **Apache-2.0** (`LICENSE`; fejlécek: `Grid.h`, `AST.h`: `SPDX-License-Identifier: Apache-2.0`; `RE-LICENSE_NOTE.txt`: a közreműködők az MPL-2.0-ról Apache-2.0-ra relicenszelték, köztük a „Double Negative”). Verzió a klónban: CMake `14.0.0`; a klón HEAD 2026-11-20 | **IGAZOLT** | GPL-3-kompatibilis (licensing.md §3). Az előző állításom (MPL-2.0) **téves** volt: az MPL a relicenc előtti állapot. A megbízás állítása helyes. ADR-0003 táblasora („BSD-3 / Apache-2.0”) pontosítandó: **Apache-2.0**. |
| `dneg/openvdb` fork | **MPL-2.0** (`LICENSE`; fejléc: `SPDX … MPL-2.0`); utolsó commit 2020-12-24, tehát az upstream relicencelése *előtti* pillanatkép | **IGAZOLT** | MPL-2.0 megengedett, de a fork elavult; **nem használjuk** |
| `dneg/openvdb_ax` | **MPL-2.0** (`LICENSE`); a README: „as of OpenVDB AX 1.0.0 and OpenVDB 8.0.0 this repository has been merged into the OpenVDB ASWF repository”; utolsó commit 2021-01-18 | **IGAZOLT** | AX elsődleges forrása = az ASWF-repó `openvdb_ax/` (Apache-2.0 fejlécek) |
| `dneg/openvdb_points_dev` | a repóban **csak `README.md`** van, licencfájl nincs; a README: „This repository is deprecated … integrated into the Dreamworks Animation OpenVDB repository”; utolsó commit 2017-01-20 | **IGAZOLT** | licenc a repóban **nem megállapítható**; **nem használható**; a Points = az upstream `openvdb/points/` (a klónban megvan: AttributeArray/Set/Group, PointGroup, PointCount, PointDelete, PointMove, PointScatter, StreamCompression…) |
| **DNEG AAF SDK** (`dneg/aaf`) | **AAF SDK Public Source License 2.0** (`LEGAL/AAFSDKPSL.TXT`, 2008): saját, MPL-szerű szöveg: szerzői jogi és szabadalmi licenc, **kölcsönös szabadalmi licenc** (2.2), védekező felfüggesztés (2.3), az „AAF” név használata csak „Standard Version” esetén (2.1.5); a licenc maga jelzi, hogy nem ad szabadalmi jogot az AAF-szabványra. Utolsó commit 2012-12-14, SDK 1.1.5 | **IGAZOLT a szöveg**; **GPL-3-kompatibilitás NEM igazolt** | az SDK **nem** GPL-kompatibilisnek tekintendő jogi vélemény nélkül; amúgy is clean-room-szabály tiltja a forrás olvasását. |
| **DNEG USD** | „Modified Apache 2.0” (a fork); az upstream: Tomorrow Open Source Technology License 1.0 | **IGAZOLT** | lásd D7 |

**Függőségek (OpenVDB `doc/dependencies.txt`, upstream HEAD – igazolt a szövegből):** core: CMake, C++17, **oneTBB** (min. 2021, aj. ≥ 2022), zlib (≥ 1.2.7), Zstd (opcionális), Imath (3.2; mellékelve), VCL (mellékelve); render: OpenEXR, libpng; **AX: LLVM ≥ 18**, Bison, Flex; **NanoVDB: CUDA/TBB/zlib – a GPU-ág CUDA-s** (vendor-semlegességi szabály: csak CPU-párral). A dokumentum Ubuntu-példát is ad (`apt-get install libtbb-dev … llvm-18-dev`).

**Még ELLENŐRIZETLEN (nem tudtam ellenőrizni):**
- Ubuntu **26.04** csomagok és verziók (libopenvdb, libtbb, LLVM) – a munkamenet nem 26.04-es csomagforrást lát.
- Az AX LLVM-ismert hibái (a dokumentum „known issues”) a 26.04-es LLVM-verzióval.
- A transzitív függőségek licence (oneTBB: Apache-2.0, zlib, Zstd: BSD/GPL kettős, LLVM: Apache-2.0 + LLVM-kivétel) – ismereten alapul, fájlból nem olvastam.
- Redistributió/`.deb` `Depends` kialakítása és AppImage/Flatpak-csomagolás.
- A DNEG-forkok 2020–2021-es közreműködői előtörténete (shallow klónok).

**CFA4-szabályok az integrációra (rögzítve):**
- Függőség csak az Ubuntu-tárolóból (rendszercsomag, `Depends`/`Recommends` a `.deb`-ben; AppImage/Flatpak: külön audit), **pinelve** és `cargo deny`-vel/SBOM-mal auditálva (`docs/audit/F1-01-SBOM-*`).
- A build nem tölthet le semmit; a `build_usd.py` jellegű letöltő scriptek nem futnak; nincs Conda/Miniforge; nincs Windows/macOS ág.
- Kód átvétele **nincs** a tervben: a worker rendszer-OpenVDB-t **dinamikusan linkel**, Rust-interfész mögött; átvétel esetén reused-code register sor (`licensing.md` §5).
- Védjegy: „OpenVDB”, „DNEG” nevek csak megnevezésként; a licenc a közreműködők védjegyhasználatát tiltja a projekt társításában.

**Karbantarthatóság (igazolt):** ASWF OpenVDB aktív (HEAD 2026-11-20; 14.0.0); DNEG-forkok állnak (2012–2023), a points-repó deprecated; ezért **csak az upstream** jöhet szóba.

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

## Nem teljesített / ellenőrizetlen tételek jegyzéke (frissítve 2026-11-21, hálózati ellenőrzés után)

| ID | Tétel | Állapot | Mi oldja fel |
|---|---|---|---|
| U1 | VFX Catalog, FX Authoring, ARF, OpenUSD/3D, Potree/pontfelhő terv | **jóváhagyva 2026-11-21** (`docs/plans/…-0.1.md`); a `CFA4-VFX-001`-gyel való egyeztetés `[MÓD-5]` jóváhagyásra vár | az egyeztetés jóváhagyása |
| U2 | OpenVDB / AX / Points / NanoVDB licenc | **IGAZOLT: Apache-2.0** (upstream); DNEG-forkok MPL-2.0 (nem használjuk) | – |
| U3 | DNEG-fork állapotok | **IGAZOLT** (stagnáló/deprecated; D8) | – |
| U4 | USD fork-diff | **IGAZOLT: delta üres** (fork = upstream v23.05); Ubuntu 26.04-es build nincs próbálva | – / U6 |
| U5 | AAF SDK licence | **IGAZOLT a szöveg** (PSL 2.0); GPL-3-kompatibilitás **jogilag nem minősített** | jogi vélemény, ha egyáltalán kell |
| U6 | Ubuntu 26.04 csomagok (libopenvdb, TBB, LLVM ≥ 18) | **részben igazolt `[MÓD-5]`**: `openvdb` 10.0.1-3 (Launchpad, G1, 2026-11-22; upstream MPL-2.0); `openusd`/`usd-core` forráscsomag nincs; LLVM tárgytalan (Q4); a bináris csomagok tartalma és a tényleges `copyright` fájl ELLENŐRIZETLEN | 26.04-es gépen/konténerben `apt-cache policy`, `apt-file` |
| U7 | AX sandbox-PoC | nincs implementáció | G4 |
| U8 | Teljesítmény- és memóriamérések | nincs implementáció | G2–G5 |
| U9 | Transzitív licencek (TBB, zlib, Zstd, LLVM) | ismeret, fájlból nem olvasva | G1 |
| U10 | OpenUSD upstream HEAD verziószáma | **lezárva `[MÓD-5]`**: legutóbbi kiadás v26.11.1 (2026-10-21), `dev` HEAD 2026-11-18 | – |
