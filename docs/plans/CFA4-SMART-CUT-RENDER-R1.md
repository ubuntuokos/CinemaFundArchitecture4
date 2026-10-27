# CFA4 – Native Smart Cut / Smart Render
## GOPchop-alapelvek integrációs és végrehajtási terve

**Projekt:** CinemaFundArchitecture4 (CFA4)
**Cél repository:** `ubuntuokos/CinemaFundArchitecture4`
**Specifikáció:** `CFA4-SMART-CUT-RENDER-R1`
**Dátum:** 2026-10-27
**Állapot:** Implementációra előkészített terv; még nem megvalósított funkciók. 2026-10-27: a CFA4-SCE-001 v1.5 döntései szerint igazítva (0. fejezet).

> A tulajdonos (ubuntuokos) által 2026-10-27-én megadott terv változatlan szövege. A kiindulási
> audit (WP-00) eredménye és a terv és a repository szabályai közötti eltérések:
> [`docs/audit/smart-edit-baseline.md`](../audit/smart-edit-baseline.md). Az upstream ős nevét a
> repository függetlenségi szabálya miatt (AGENTS.md, `python/tests/test_independence.py`) általános
> megnevezés helyettesíti. A 2026-10-27-i, tulajdonos által véglegesített CFA4-SCE-001 v1.5 terv
> döntései alapján a szöveg az alábbi pontokon módosult: FFmpeg-utalások (§1, §3, §12, §15, §19),
> a „határ-újrakódolás” elnevezés (§5.3, §6, §8, §10, §17), az Auto mód (§14), a parancsnévtér (§11)
> és a modulhely (§4). Az eredeti szöveg a git-előzményben elérhető.

---

## 0. Igazítás a CFA4-SCE-001 v1.5 döntéseihez (2026-10-27)

A végleges, egyesített terv: `docs/plans/CFA4-SCE-001-STREAM-COPY-EXPORT-2026-10-27.md` (main). Ahol
ez a dokumentum eltér tőle, a v1.5 döntései érvényesek:

1. **GOP Smart Cut** = a Smart Render egyklipes esete; ugyanaz a tervtípus és végrehajtó. A Qt6-ban a
   forrásmonitor „Forrás vágása” művelete, nem külön exportüzemmód.
2. **Auto mód** csak javasol (Strict Copy → Compatible Copy → Smart Render → Full Render); újrakódolás
   csak kifejezett megerősítés után fut (§14).
3. **Terminológia:** „Smart Render” = idővonal-hibrid. A `BoundaryReencode` szegmens neve
   **határ-újrakódolás**; ebben a dokumentumban a korábbi „Smart Render” megnevezés, ahol a
   határszakasz-újrakódolást jelentette, erre módosult.
4. **Bitstream-adapterek** natív Rustban. A Smart Cut / Smart Render végrehajtásában az FFmpeg 9.0.2
   kizárólag CI-teszt-orákulum, futásidejű függőség nincs (lásd még `docs/audit/smart-edit-baseline.md`
   §3). Az ADR 0005 szerinti, opcionális `cfa4-ffmpeg` termékbackend (terve:
   `docs/plans/CFA4-FFMPEG-BACKEND-R1-2026-10-27.md`) ettől független: a Smart Edit nem függhet tőle,
   legfeljebb a `cfa4-route` által választott, külön igazolt kódolási/dekódolási háttér lehet.
5. **Rétegzett validáció:** a szerkezeti egyezés (`BITSTREAM_EQUIVALENT`) elég a PASS-hoz; dekódolásos
   összevetés csak a saját dekóder által támogatott profilnál fut.
6. **Atomikus írás:** közös segéd a `cfa4-export`-ban (ideiglenes fájl, fsync, átnevezés).
7. **Határ-újrakódolás kodekjei:** az első kiadásban csak H.264. MPEG-2 (nincs natív kódoló), HEVC
   (ma csak NVENC, gyártófüggő) és AV1 (nincs kódoló) forrásnál kulcskocka-kerekítés vagy teljes
   újrakódolás, amíg nincs gyártófüggetlen kódoló.
8. **Parancsnévtér:** egységes `export.analyze`, `export.plan`, `export.start`, `export.cancel`,
   `export.validate` (§11).

---

## 1. Kötelező megvalósítási keretek

A fejlesztés kizárólag a CFA4 downstream repositoryban történhet.

- Operációs rendszer: Ubuntu 26.04 LTS és Ubuntu-alapú változatai.
- Architektúra: amd64.
- Grafikus felület: Qt6, elsődlegesen KDE Plasma / Wayland.
- Vezérlési és feldolgozási réteg: Rust/Cargo.
- Natív multimédia-feldolgozás: a CFA4 saját Rust crate-jei (demux, dekódolás, kódolás, mux). A Smart Cut / Smart Render végrehajtásában az FFmpeg 9.0.2 kizárólag CI-teszt-orákulum, futásidejű függőség nincs; az opcionális `cfa4-ffmpeg` backend külön terv (0. fejezet, 4. pont).
- Megengedett kiegészítő nyelvek: C++, Python 3.14.
- Terjesztés: DEB, Flatpak, AppImage.
- Conda/Miniforge alkalmazása tilos.
- A rendszernek offline környezetben is működnie kell.
- A GOPchop külön alkalmazásként, futásidejű függőségként vagy külön GUI-ként nem kerülhet beépítésre.
- Az upstream (örökölt ős) repository nem módosítható.

### Kötelező kompatibilitási elv

Az új alrendszernek a meglévő CFA4 natív média-, idővonal-, projektkezelő és exportfunkciókat kell kiegészítenie.

A jelenlegi szerkesztési és exportadatok formátuma nem változtatható inkompatibilisen. Új séma esetén verziózott migráció és visszafelé kompatibilis beolvasás szükséges.

---

## 2. Fejlesztési célok

### SC-01 – Veszteségmentes vágás

Olyan vágási folyamat kialakítása, amelynél az érintetlen videó- és hangcsomagok bitstreamtartalma változatlan marad.

### SC-02 – GOP-aware editing

A rendszer azonosítsa:

- GOP-határok;
- I/P/B képtípusok;
- véletlen hozzáférési pontok;
- zárt és nyitott GOP-ok;
- képkockák közötti referenciafüggőségek;
- dekódolási és megjelenítési sorrend;
- időbélyegek és timebase adatok;
- codec-konfigurációváltozások.

Az I-frame jelenléte önmagában nem minősül elegendő bizonyítéknak a biztonságos vágáshoz.

### SC-03 – Smart Render

A rendszer felismerje azokat a szakaszokat, amelyek:

1. változatlanul másolhatók;
2. csak a határaiknál igényelnek újrakódolást;
3. teljes újrakódolást igényelnek.

### SC-04 – Frame-accurate editing

A felhasználó által kijelölt vágási pontok képkockapontosan érvényesüljenek, amennyiben az adott exportmód ezt támogatja.

### SC-05 – Automatikus végrehajtási terv

A render előtt egy determinisztikus elemző állítson elő exporttervet, amely megmutatja a másolható és újrakódolandó szakaszokat.

### SC-06 – Biztonságos export

Minden kimenetnél ellenőrizni kell a dekódolhatóságot, az időbélyegeket, az A/V szinkront és a konténer megfelelőségét.

---

## 3. Tervezett architektúra

```text
CFA4 Qt6 GUI
    |
    +-- Timeline / Trim / Split
    +-- GOP Overlay
    +-- Smart Export Dialog
    +-- Render Plan Preview
    |
    v
CFA4 Native Control API
    |
    v
Rust Smart Edit Controller
    |
    +-- Media Capability Registry
    +-- Bitstream / GOP Indexer
    +-- Random Access Analyzer
    +-- Edit Dependency Analyzer
    +-- Smart Cut Planner
    +-- Smart Render Planner
    +-- Timestamp Coordinator
    +-- Audio Boundary Planner
    +-- Validation Controller
    |
    v
CFA4 natív médiaréteg (cfa4-export)
    |
    +-- Konténerek: isobmff, matroska, mpegts
    +-- Kodekek: saját dekóderek / kódolók
    +-- Natív bitstream-adapterek
    |
    +-- Stream Copy Executor
    +-- Boundary Re-encode Executor (határ-újrakódolás)
    +-- Full Render Executor
    +-- Muxer / Output Writer (közös atomikus író)
    |
    v
Export Validator
    |
    +-- Szerkezeti egyezés (BITSTREAM_EQUIVALENT)
    +-- Decode (saját dekóder, ha a profil támogatott)
    +-- Frame Count / Duration
    +-- PTS / DTS / Timebase
    +-- A/V Synchronization
    +-- Boundary Artifact Detection
    |
    v
Final Output
```

### Kötelező kialakítás

A Smart Cut és Smart Render ne hozzon létre párhuzamos, önálló exportszolgáltatást.

A tervező a meglévő CFA4 exportfolyamatába kapcsolódjon. A feladatütemezés, állapotkezelés, megszakítás, naplózás és hibajelentés a meglévő szolgáltatásokon keresztül működjön.

MLT integráció esetén a Smart Render végrehajtás natív exportútja maradjon elkülönítve az MLT-s szerkesztési gráftól. A kép- és hangeffektusok leképezése egységes edit-leírásból történjen.

---

## 4. Javasolt Rust-modulok

A végleges elérési útvonalat a repository auditja határozza meg. Az alábbiak logikai modulnevek.

*v1.5:* a tervezés (capability, index, függőség, planner, `render_plan.rs` = közös `ExportExecutionPlan`)
a `crates/smart-edit` (L3) crate-ben, a végrehajtás (`execution.rs`, copy- és határ-újrakódoló
végrehajtó, validátor) a `crates/export`-ban (`copy/`, `smart_render/`) készül.

```text
smart_edit/
    mod.rs
    capabilities.rs
    media_index.rs
    frame_dependency.rs
    access_points.rs
    edit_decision.rs
    smart_cut.rs
    smart_render.rs
    render_plan.rs
    execution.rs
    audio_sync.rs
    timestamp.rs
    validator.rs
    error.rs
```

### Modulok feladatai

**capabilities.rs** – A codec-, konténer-, encoder- és muxer-képességek nyilvántartása. A tényleges támogatást futásidejű és tesztelt capability-adatok alapján határozza meg.

**media_index.rs** – Videócsomagok, képkockák, időbélyegek, streamazonosítók és hozzáférési pontok indexelése.

**frame_dependency.rs** – Képkockafüggőségek elemzése, beleértve a referencia-képkockákat és a GOP-határok közötti függőségeket.

**access_points.rs** – Biztonságos, önállóan dekódolható vágási kezdőpontok felismerése.

**edit_decision.rs** – Az idővonalon végrehajtott vágások és szerkesztések egységes reprezentációja.

**smart_cut.rs** – Packet-copy lehetőségek meghatározása.

**smart_render.rs** – Idővonal-hibrid szakaszterv; a határ-újrakódolás (határ-GOP) terve és kompatibilitási feltételei.

**render_plan.rs** – A teljes export végrehajtási gráfjának előállítása.

**execution.rs** – A meglévő natív exportmotorhoz kapcsolódó végrehajtás.

**audio_sync.rs** – Hangcsomag-határok, encoder-delay, priming, padding és A/V illesztés.

**timestamp.rs** – PTS/DTS, timebase, edit-list és timestamp-diszkontinuitások kezelése.

**validator.rs** – Exportvalidáció, ellenőrzési eredmények és diagnosztikai jelentés.

---

## 5. Kötelező adatmodellek

A Rust-adatmodelleknek az alábbi információkat kell kezelniük.

### 5.1 MediaIndex

```rust
pub struct MediaIndex {
    pub source_id: String,
    pub streams: Vec<StreamIndex>,
    pub duration: Option<MediaTime>,
    pub index_version: u32,
}
```

### 5.2 FrameIndexEntry

```rust
pub struct FrameIndexEntry {
    pub stream_id: u32,
    pub decode_index: u64,
    pub presentation_index: Option<u64>,
    pub pts: Option<i64>,
    pub dts: Option<i64>,
    pub keyframe: bool,
    pub random_access: RandomAccessKind,
    pub dependency_status: DependencyStatus,
}
```

A példák interfészvázlatok. A kapcsolódó típusokat az implementáció során kell definiálni.

### 5.3 RenderMode

```rust
pub enum RenderMode {
    StreamCopy,
    BoundaryReencode, // határ-újrakódolás
    FullReencode,
}
```

*v1.5:* a megvalósításban ez a közös `ExportExecutionPlan` szegmensmódja: `StrictCopy`,
`CompatibleCopy`, `BoundaryReencode` (határ-újrakódolás), `Render`. A `StreamCopy` a két külön
választható copy-garanciára bomlik.

### 5.4 RenderSegment

```rust
pub struct RenderSegment {
    pub source_id: String,
    pub source_in: MediaTime,
    pub source_out: MediaTime,
    pub output_in: MediaTime,
    pub mode: RenderMode,
    pub reason: RenderReason,
}
```

### 5.5 RenderPlan

```rust
pub struct RenderPlan {
    pub plan_version: u32,
    pub segments: Vec<RenderSegment>,
    pub expected_duration: MediaTime,
    pub requires_validation: bool,
}
```

### Adatmodell-követelmények

- Az időadatokhoz racionális timebase-reprezentáció szükséges.
- Lebegőpontos másodpercek nem használhatók elsődleges idővonal-koordinátaként.
- VFR esetén a valódi PTS-adatok elsődlegesek.
- A hiányzó DTS/PTS értékeket külön állapotként kell kezelni.
- Az indexelt adatokhoz forrásazonosító és érvényességi ellenőrzés tartozzon.
- Cache-találatnál ellenőrizni kell a forrásméretet, módosítási állapotot és szükség szerint a tartalomazonosítót.
- Az eredeti mediafájlok soha nem módosíthatók közvetlenül.

---

## 6. Codec-támogatás

### Első fejlesztési szint

| Codec | GOP elemzés | Packet Copy | Határ-újrakódolás |
|---|---|---|---|
| MPEG-2 | Kötelező | Kötelező | Nincs natív kódoló: kulcskocka-kerekítés vagy teljes újrakódolás (v1.5) |
| H.264/AVC | Kötelező | Kötelező | Kötelező, validált konfigurációkra |
| HEVC/H.265 | Kötelező | Kötelező | Csak gyártófüggetlen kódoló után (ma csak NVENC) |
| AV1 | Kötelező | Kötelező | Nincs kódoló; külön kompatibilitási kapu |

A táblázat célkövetelmény, nem jelenlegi támogatottsági állapot.

### MPEG-2

- Sequence header felismerése.
- GOP-header elemzése.
- Closed/open GOP megkülönböztetése.
- B-frame referenciafüggőségek kezelése.
- MPEG-2 Program Stream támogatás.
- Remux kompatibilitás ellenőrzése.

### H.264

- SPS/PPS elemzés.
- IDR és nem IDR képek megkülönböztetése.
- Recovery point információk vizsgálata.
- Referenciaképek és DPB-függőségek kezelése.
- Annex B / AVCC kompatibilitás.

### HEVC

- VPS/SPS/PPS feldolgozás.
- IDR, CRA és BLA hozzáférési pontok megkülönböztetése.
- RASL/RADL függőségek kezelése.
- Decoder configuration összeegyeztethetőség ellenőrzése.

### AV1

- OBU-struktúra elemzése.
- Sequence header vizsgálata.
- Keyframe és egyéb hozzáférési lehetőségek értékelése.
- Reference frame kezelés.
- Show-existing-frame és order-hint kezelés.
- Decoder/encoder/muxer képességek külön ellenőrzése.

A négy codec sajátosságait nem szabad egyetlen egyszerű „keyframe = biztonságos vágás” szabályra redukálni.

---

## 7. Smart Cut algoritmus

### Bemenet

- Médiaforrás
- Kijelölt kezdőpont
- Kijelölt végpont
- Kimeneti konténer
- Streamlista
- Vágási pontossági követelmény

### Feldolgozás

1. A médiaindex betöltése vagy elkészítése.
2. A vágáshoz tartozó megjelenítési képkockák feloldása.
3. Biztonságos hozzáférési pont keresése.
4. Referenciafüggőségek elemzése.
5. A hangsávok és az időbélyegek ellenőrzése.
6. A kimeneti muxer alkalmasságának vizsgálata.
7. Packet-copy terv készítése.
8. Ha a terv nem teljesíti a pontossági követelményt, határ-újrakódolás vagy Full Render javaslata; újrakódolás csak kifejezett megerősítés után (v1.5).
9. Export végrehajtása.
10. Eredményvalidáció.

### Kötelező biztonsági szabály

A vágási pont nem változtatható meg hallgatólagosan.

Ha a felhasználó veszteségmentes stream-copy módot kényszerít ki, és a kiválasztott vágás nem hajtható végre pontosan, a rendszer jelenítse meg a lehetséges biztonságos pontokat, és kérjen új választást.

---

## 8. Határ-újrakódolás algoritmusa

*v1.5:* ez a fejezet a Smart Render (idővonal-hibrid) határ-újrakódolási szegmenstípusát írja le; a
teljes idővonal-hibridet a `docs/plans/SMART-RENDER-2026-10-27.md` részletezi.

A határ-újrakódolás célja a lehető legkisebb szükséges újrakódolási tartomány.

### Folyamat

1. Pontos szerkesztési határok meghatározása.
2. Az érintett dekódolási referenciahalmaz kiszámítása.
3. Újrakódolandó határszakaszok kijelölése.
4. Az érintetlen köztes szakaszok packet-copy alkalmasságának ellenőrzése.
5. Encoderkompatibilitási terv elkészítése.
6. A határszakaszok dekódolása és újrakódolása.
7. Az újrakódolt és másolt szakaszok konténerkompatibilitásának ellenőrzése.
8. Kimeneti csomagsorrend és időbélyegek rendezése.
9. Az eredmény remuxolása.
10. Teljes kimeneti validáció.

### Kötelező technikai feltételek

A határszakasz újrakódolása csak akkor egyesíthető másolt bitstreammel, ha a codec-paraméterek és a kimeneti formátum kompatibilisek.

Vizsgálni kell:

- profil és level;
- felbontás;
- pixel formátum;
- bitmélység;
- chroma subsampling;
- color primaries, transfer és matrix;
- HDR metadata;
- codec extradata;
- referenciakép-konfiguráció;
- stream-header kompatibilitás;
- encoder által létrehozott hozzáférési pontok.

Ha a kompatibilitás nem bizonyítható, a rendszer ne alkalmazzon veszélyes bitstream-összefűzést.

Ilyenkor Full Renderre kell váltani, vagy felhasználói beállítás szerint le kell állítani az exportot.

---

## 9. Időbélyeg- és hangkezelés

A Smart Render és a határ-újrakódolás önálló A/V koordináció nélkül nem tekinthető késznek.

### Kötelező feladatok

- PTS/DTS normalizálás.
- Decode order és presentation order helyes kezelése.
- B-frame reorder kezelés.
- Időbélyegek megfelelő monotonitásának biztosítása.
- VFR támogatása.
- Kezdő időeltolások kezelése.
- Hiányzó és hibás timestamp felismerése.
- Audio encoder delay kezelése.
- AAC priming/padding ellenőrzése.
- Hangcsomag-határok összehangolása.
- Több hangsáv és hangsávcsatorna kezelése.
- Felirat- és metadata-stream kompatibilitási vizsgálat.

### A/V szinkron

A vágási ponton a hang és a videó időbeli illeszkedése nem térhet el a specifikált tolerancián túl.

Frame-accurate exportnál a validáció elsődleges követelménye a kijelölt tartalom és a tényleges kimeneti megjelenítési idővonal egyezése.

---

## 10. Qt6 grafikus integráció

Az új funkciók a meglévő CFA4 GUI-ba kerüljenek.

### 10.1 Idővonal

Új vizuális megjelenítési lehetőségek:

- Kulcsképkocka-jelölések.
- GOP-határok.
- Biztonságos vágási pontok.
- Újrakódolást igénylő vágási pontok.
- Nyitott GOP vagy problémás referenciafüggőség jelölése.

### 10.2 Smart Export panel

Választható módok:

- Automatikus (csak javasol; újrakódolás előtt megerősítést kér).
- Stream Copy – Bitazonos.
- Stream Copy – Kompatibilis.
- Smart Render.
- Full Render.

*v1.5:* a GOP Smart Cut nem exportmód, hanem a forrásmonitor „Forrás vágása” művelete.

### 10.3 Render Plan előnézet

A felhasználó export előtt láthassa:

- A másolható szakaszokat.
- Az újrakódolandó határszakaszokat.
- A teljes renderelést igénylő szakaszokat.
- Az elutasított vagy nem támogatott műveletek indokát.

### 10.4 Hibajelzés

A GUI emberileg érthető hibaüzeneteket jelenítsen meg. A részletes diagnosztika strukturált naplóba kerüljön.

Példák:

- „A kijelölt ponton a stream-copy nem biztonságos.”
- „A vágási pont nyitott GOP-on belül található.”
- „Az export határszakaszainak encoder-paraméterei nem kompatibilisek.”
- „A teljes újrakódolás szükséges a kiválasztott effektus miatt.”

---

## 11. Natív vezérlő API

A Smart Edit Controllernek a következő logikai műveleteket kell nyújtania:

```text
export.analyze     # forrás, GOP-index, biztonságos hozzáférési pontok, képességek
export.plan        # egyetlen ExportExecutionPlan; mód és hatókör (idővonal / forrástartomány) paraméter
export.start       # végrehajtás, render-job azonosítóval és állapottal
export.cancel
export.validate    # terv és kimenet validációja
```

*v1.5 (8. döntés):* egységes `export.*` névtér. A korábbi javaslatok megfeleltetése:
`media.analyze_cut_points`, `media.get_gop_index`, `media.get_safe_access_points` → `export.analyze`;
`edit.plan_smart_cut`, `edit.plan_smart_render` → `export.plan`; `edit.validate_render_plan`,
`export.validate_output` → `export.validate`; `export.execute_render_plan` → `export.start`;
`export.cancel_render_plan` → `export.cancel`; `export.get_render_status` → az `export.queue.*`
állapota. A meglévő `export.formats`, `export.resolve` és `export.queue.*` megmarad.

### API-követelmények

- Verziózott üzenetséma.
- Aszinkron indexelési feladat.
- Megszakítható végrehajtás.
- Strukturált hibakódok.
- Render-job azonosító.
- Determinisztikus tervezési kimenet.
- Az idővonal módosításakor a régi tervek érvénytelenítése.
- Elavult aszinkron válasz nem írhatja felül az új szerkesztési állapotot.
- Az exporttervnek a létrehozáskor érvényes projekt- és idővonal-verzióhoz kell kötődnie.

---

## 12. Fejlesztési munkacsomagok

### WP-00 – CFA4 kiindulási audit

**Feladatok**

1. Natív média- és exportmodulok feltérképezése.
2. Aktuális Git commit és fejlesztési ág rögzítése.
3. FFmpeg 9 runtime-linkage ellenőrzése (eredmény: nincs; az FFmpeg 9.0.2 csak CI-orákulum, lásd az audit §3-at).
4. Meglévő stream-copy képességek felmérése.
5. Keyframe-indexelés és navigáció ellenőrzése.
6. Qt6 idővonal és exportpanel vizsgálata.
7. Export és reimport tesztek felmérése.
8. MLT-adatfolyam illesztési pontjainak vizsgálata.

**Kimenet:** `smart-edit-baseline.md`

**Kapufeltétel:** egyértelműen dokumentált meglévő és hiányzó képességek.

### WP-01 – Capability Registry

A codec-, decoder-, encoder-, demuxer- és muxer-támogatás egységes nyilvántartása.

**Kimenet:** capability-adatmodell és automatizált tesztek.

**Kapufeltétel:** a támogatási döntések reprodukálhatók.

### WP-02 – Media/GOP Indexer

Csomag- és képkockaszintű indexelés, hozzáférési pontokkal és referenciafüggőségekkel.

**Kimenet:** médiaindex-szolgáltatás.

**Kapufeltétel:** az MPEG-2 és H.264 referenciaanyagok helyes indexelése.

### WP-03 – Smart Cut Planner

Stream-copy alkalmasság és pontos vágási pontok kezelése.

**Kimenet:** determinisztikus packet-copy terv.

**Kapufeltétel:** nem biztonságos vágás nem minősülhet automatikusan veszteségmentesnek.

### WP-04 – Smart Cut Executor

Natív Rust packet-copy végrehajtás a meglévő exportszolgáltatásban (`cfa4-export`), a közös atomikus íróval.

**Kimenet:** validált stream-copy export.

**Kapufeltétel:** sikeres export és reimport, változatlan másolt payloadokkal.

### WP-05 – Smart Render Planner

Idővonal-hibrid szakaszterv: újrakódolandó határszakaszok (határ-újrakódolás) és változatlanul másolható tartományok meghatározása.

**Kimenet:** vegyes végrehajtási terv.

**Kapufeltétel:** referenciafüggőségek miatt szükséges további tartományok helyes kijelölése.

### WP-06 – Smart Render Executor

Határszakaszok célzott újrakódolása és kompatibilis kimenet előállítása.

**Kimenet:** H.264 határ-újrakódolás és Smart Render végrehajtó; MPEG-2-nél kódoló hiányában kulcskocka-kerekítés vagy teljes újrakódolás (v1.5).

**Kapufeltétel:** pontos vágás, dekódolható összefűzés és ellenőrzött időbélyegek.

### WP-07 – HEVC / AV1 bővítés

A két codec saját referenciafüggőségeinek és hozzáférési pontjainak támogatása.

**Kimenet:** képességvizsgálattal vezérelt HEVC/AV1 integráció.

**Kapufeltétel:** sikeres codec-specifikus tesztkészlet; támogatás csak validált konfigurációkra engedélyezhető.

### WP-08 – Qt6 GUI

GOP-overlay, exportmódok és renderterv-előnézet beépítése.

**Kimenet:** CFA4 natív Qt6 felület.

**Kapufeltétel:** Wayland alatt használható teljes szerkesztési és exportfolyamat.

### WP-09 – Validáció és regresszió

Teljes automatizált tesztcsomag.

**Kimenet:** tesztjelentések és CI-ellenőrzések.

**Kapufeltétel:** blokkoló regresszió nélkül teljesülnek az elfogadási feltételek.

### WP-10 – Csomagolás

A kész megoldás átvezetése a támogatott Linux-csomagokba.

**Kimenet:** DEB, Flatpak és AppImage csomagolási validáció.

**Kapufeltétel:** a funkció a csomagolt alkalmazásban, külön GOPchop telepítése nélkül működik.

---

## 13. Tesztmátrix

### Codec-vizsgálatok

| Teszt | MPEG-2 | H.264 | HEVC | AV1 |
|---|---|---|---|---|
| Indexelés | Kötelező | Kötelező | Kötelező | Kötelező |
| Biztonságos stream copy | Kötelező | Kötelező | Kötelező | Kötelező |
| Nyitott GOP kezelése | Kötelező | Kötelező | Releváns megfelelője | Releváns megfelelője |
| Pontos vágás | Kötelező | Kötelező | Kötelező | Kötelező |
| Boundary re-encode | Kötelező | Kötelező | Capability-gated | Capability-gated |
| Export/reimport | Kötelező | Kötelező | Kötelező | Kötelező |

### Kötelező szélső esetek

1. Vágás az első képkockán.
2. Vágás az utolsó képkockán.
3. Vágás GOP-határon.
4. Vágás GOP közepén.
5. B-frame függőségek a határon.
6. Nyitott GOP.
7. Több vágási pont.
8. Több összefűzött klip.
9. Eltérő időalapok.
10. VFR bemenet.
11. Hibás vagy hiányzó PTS/DTS.
12. AAC encoder delay.
13. Több hangsáv.
14. Eltérő codec extradata.
15. HDR metadata.
16. Kimeneti muxer inkompatibilitása.
17. Sérült médiafájl.
18. Megszakított export.
19. Export közben módosított projekt.
20. Sikertelen validáció utáni fallback.
21. Forrásfájl időközbeni módosítása.
22. Többszöri export azonos bemenetből.
23. GOP-index cache érvénytelenítése.
24. Hardveres encoder és szoftveres encoder eltérő kimeneti paraméterei.

### Elfogadási metrikák

**Packet Copy**

- A ténylegesen másolt tömörített payloadok egyezzenek a forrás megfelelő payloadjaival.
- A kimenet minden hozzáférési ponttól megfelelően dekódolható legyen.
- A kimeneti konténer megfeleljen az időbélyeg- és streamszabályoknak.

**Smart Render**

- A kijelölt kezdő és végső megjelenítési képkockák egyezzenek a szerkesztési tervvel.
- A vágási határokon ne legyen dekódolási hiba, referenciahiány vagy nem tervezett képkockakiesés.
- Az eredeti, másolt belső szakaszok payloadjai maradjanak változatlanok.
- Az újrakódolás kizárólag a validált terv által szükségesnek jelölt tartományokon történjen.

**A/V**

- A szinkroneltérés maradjon a dokumentált, formátumfüggő tolerancián belül.
- A validáció külön értékelje a hangsávok tényleges mintapontosságát és a codec-frame határokat.

**Regresszió**

- A korábban működő CFA4 import/export folyamatok ne romoljanak.
- Minden új funkció rendelkezzen automatizált teszttel.
- A meglévő projektfájlok maradjanak olvashatók.

---

## 14. Hibakezelés és fallback-stratégia

A rendertervező a következő döntési sorrendet kövesse:

```text
Requested Edit
     |
     v
Can safely copy?
     |
     +-- YES --> Packet Copy
     |
     +-- NO
          |
          v
Can safely re-encode boundaries?
          |
          +-- YES --> Smart Render
          |
          +-- NO
               |
               v
Can fully render?
               |
               +-- YES --> Full Render
               |
               +-- NO --> Error + explanation
```

Automatikus mód esetén a döntési lánc csak javaslatot ad (v1.5): a felület megmutatja a választott módot, az indokot és szegmensenként a másolt és az újrakódolt részt. Újrakódolást tartalmazó javaslat csak a felhasználó kifejezett megerősítése után fut, és csak akkor, ha a kimenet megfelel a felhasználó minőségi és codec-korlátozásainak. A „Smart Render” ág itt a határ-újrakódolást jelenti.

Kifejezetten kikényszerített Stream Copy módban a rendszer nem válthat észrevétlenül újrakódolásra.

---

## 15. CI/CD követelmények

A módosítások külön fejlesztési ágon készüljenek.

**Javasolt ág:** `feature/native-smart-cut-render`

A CI-kapuk tartalmazzák:

- `cargo fmt --check`
- `cargo clippy`
- Rust unit tesztek
- Rust integration tesztek
- FFmpeg 9.0.2 orákulum-összevetés (csak CI; ffprobe, dekódolhatóság, payload-összevetés)
- Codec-fixture alapú vágási tesztek
- Export/reimport E2E
- Qt6 GUI smoke és állapotkonzisztencia tesztek
- Wayland kompatibilitási ellenőrzés
- DEB / Flatpak / AppImage ellenőrzés
- Licenc- és függőségi audit

A tesztfixture-öket reprodukálhatóan kell előállítani, vagy ellenőrzött licencű tesztanyagokként kell tárolni.

A regressziós eredményeket géppel olvasható formában is menteni kell.

---

## 16. Kötelező megvalósítási sorrend

| Sorrend | Munkacsomag | Függőség |
|---|---|---|
| 1 | WP-00 – Kiindulási audit | Nincs |
| 2 | WP-01 – Capability Registry | WP-00 |
| 3 | WP-02 – Media/GOP Indexer | WP-00, WP-01 |
| 4 | WP-03 – Smart Cut Planner | WP-02 |
| 5 | WP-04 – Smart Cut Executor | WP-03 |
| 6 | WP-05 – Smart Render Planner | WP-02, WP-03 |
| 7 | WP-06 – Smart Render Executor | WP-04, WP-05 |
| 8 | WP-07 – HEVC / AV1 támogatás | WP-02–06 |
| 9 | WP-08 – Qt6 GUI integráció | WP-03–07 |
| 10 | WP-09 – Teljes regressziós validáció | WP-04–08 |
| 11 | WP-10 – Csomagolás | WP-09 |

A minimális Qt6-integráció korábban is elkezdhető, amennyiben a vezérlő API szerződése már stabil.

---

## 17. Kiadási kapuk

### R1 – Smart Cut alap

MPEG-2 és H.264 GOP/hozzáférésipont-indexelés, biztonságos packet-copy, export/reimport.

### R2 – Smart Render alap

Határ-újrakódolás H.264-re, kompatibilitási és időbélyeg-validációval; MPEG-2-nél natív kódoló hiányában kulcskocka-kerekítés vagy teljes újrakódolás (v1.5).

### R3 – Codec-bővítés

HEVC és AV1 támogatás ellenőrzött konfigurációkra; nem támogatott esetekben biztonságos fallback.

### R4 – Teljes GUI és csomagolás

Qt6 GOP-megjelenítés, renderterv-előnézet, javasló Auto mód megerősítéssel és támogatott Linux-csomagformátumok.

---

## 18. Kifejezett kizárások

Az implementáció nem tartalmazhatja:

- A GOPchop önálló GUI-ját.
- GTK-alapú párhuzamos felület kialakítását.
- GOPchop kötelező telepítését.
- Az upstream (örökölt ős) repository módosítását.
- Új, független, a CFA4 exportmotorját megkerülő renderarchitektúrát.
- Conda-alapú függőségkezelést.
- Cloudszolgáltatást.
- Automatikusan letöltött AI-modelleket.
- Nem igazolt encoder/codec-kompatibilitásra épülő bitstream-összefűzést.

A GOPchop eredeti forrásából történő kódátvétel csak külön licencvizsgálat után engedhető meg. A tervezett alapmegoldás saját CFA4-implementáció.

---

## 19. Végső Definition of Done

A funkció akkor tekinthető késznek, ha:

1. A CFA4 natív idővonalán GOP-határok és biztonságos vágási pontok megjeleníthetők.
2. Az alkalmazás felismeri, mely szakaszok másolhatók újrakódolás nélkül.
3. A rendszer képes tényleges veszteségmentes packet-copy exportálásra.
4. Pontos vágásnál a támogatott codec-konfigurációkhoz határ-újrakódolás alkalmazható.
5. A kimenet A/V szinkronja és timestampjei megfelelnek a validációs feltételeknek.
6. A nem támogatott szerkesztési helyzetek biztonságos fallbacket kapnak.
7. A funkció a CFA4 meglévő natív exportfolyamatában működik.
8. A Qt6 GUI alatt végrehajtható a teljes felhasználói munkafolyamat.
9. A célzott codec-tesztmátrix és regressziós tesztek sikeresek.
10. A funkció nem igényli a GOPchop alkalmazás külön telepítését.
11. A DEB, Flatpak és AppImage csomagokban működik.
12. Az upstream (örökölt ős) repository változatlan marad.

**Végleges architekturális döntés:** GOPchop működési elve → natív Rust GOP/bitstream elemzés → natív Rust Smart Cut / Smart Render végrehajtás → CFA4 Qt6 GUI → validált export (FFmpeg 9.0.2 csak CI-orákulum).
