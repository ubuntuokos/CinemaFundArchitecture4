# CFA4-OBJECT-MASK-PLAN-001 — Objektumfelismerés, -követés és -maszkolás

| Dokumentumadat | Érték |
|---|---|
| Állapot | **Jóváhagyva** a tulajdonos által, 2026-11-03, **elmentett terv**: a végrehajtásról (az A szakasz indításáról is) a tulajdonos később dönt |
| Dátum | 2026-11-03 |
| Tulajdonos, jóváhagyó | ubuntuokos (István) |
| Forrás | OpenShot-fork-audit (hat repository, 2026-11-03); a munkaváltozat: [Claude Doc](https://claude.ai/artifact/68ssSH9L61i5JfDzyztRC4) |
| Vizsgált állapot | CFA4 `main` 942d046; CFA4-AI_modules e241771; [OpenShot/libopenshot](https://github.com/OpenShot/libopenshot) `develop` 528ebb1 (2026-11-01) |
| Kapcsolódó | [ADR 0006](https://github.com/ubuntuokos/CFA4-AI_modules/blob/main/docs/adr/ADR-0006-ai-processing-layer.md) (AI-réteg), [Kdenlive-terv](../kdenlive-parity.md) K4, [gaps](../gaps.md) G12 és G18, [ADR 0004](../adr/ADR-0004-compute-routing.md) |

## 1. Összefoglaló

A Premiere 26 Object Mask képességét (gaps G12) a libopenshot objektumfelismerési, -követési és
-maszkolási lánca alapján valósítjuk meg, hat lépésre bontva. Ebből három (Tracking, Tracked Object
Data, Effect Binding) tiszta Rust, AI nélkül, és az F0 kapu nélkül is elvégezhető, amint a tulajdonos a végrehajtásról dönt. A másik három (Detection,
Segmentation, Mask Propagation) az ADR 0006 AI-rétegét igényli, ezért az F0 kapu után jön.

Döntések:

- **Rust, ahol számít; C++ ott, ahol a port túl drága** (tulajdonosi szabály, 2026-11-03, `AGENTS.md`).
  A szerkesztő folyamatába OpenCV és protobuf nem kerül. A libopenshot EfficientSAM/Cutie lánca
  C++-ban maradhat, külön worker-folyamatban (5. pont).
- **A modellek az ADR 0006 worker-protokollja mögött futnak**, a modellkezelő és az erőforrás-kezelő
  szabályai szerint. A libopenshot automatikus CUDA → OpenCL → CPU visszalépését nem vesszük át.
- **Ultralytics YOLO (v5/v8/11) nem használható**: kód és súly is AGPL-3.0. Apache-2.0 detektor kell
  (RT-DETR, D-FINE vagy YOLOX).
- **Az Effect Binding a meglévő maszkmodell bővítése**: `cfa4_project::Mask` új `source` mezőt kap,
  így minden effekt objektummal maszkolható.
- A stabilizálás és a klasszikus követők (CSRT stb.) külön terv tárgya; a CFA4 Warp Stabilizere és
  `masks.track` követője ennél a tervnél kiindulópont.

## 2. Meglévő CFA4-elemek

| Terület | Elem | Szerep a tervben |
|---|---|---|
| Maszkmodell | `crates/project/src/mask.rs`: minden effektpéldányon `masks: Vec<Mask>` | Az Effect Binding alapja |
| Maszk-renderelés | `crates/render/src/mask.rs` (CPU-referencia), `cfa4_gpu::GpuMask` (WGSL) | Raszteres lefedési ág kell |
| Maszkkövetés | `masks.track`, `crates/render/src/track.rs` (Shi–Tomasi, piramisos LK, RANSAC) | Nem AI-s követés és propagálás alapja |
| Háttérjobok | `crate::Job`, `guard_job` (`catch_unwind`) | A nem AI-s lépések jobjai |
| AI-réteg | ADR 0006 (elfogadva, kód még nincs), F0–F6 kapuk | A modellek futtatása |
| Útválasztás | `cfa4-route`, `routing.toml` | `ai.vision.*` sorok kellenek (ONNX Runtime) |

## 3. Licenc

| Elem | Kód | Súlyok | Tanítóadat | Döntés |
|---|---|---|---|---|
| libopenshot CV-fájlok | LGPL-3.0-or-later (fejlécből) | – | – | Átvehető, fejléc marad, sor a `docs/licensing.md` §5 regiszterbe |
| SORT Hungarian-rész | BSD-3-Clause (Cong Ma) | – | – | Rustban újraírjuk |
| Ultralytics YOLO | AGPL-3.0 | AGPL-3.0 | COCO | **Tilos** |
| RT-DETR, D-FINE, YOLOX | Apache-2.0 | Apache-2.0 | COCO | Detektorjelöltek |
| SAM2 | Apache-2.0 | Apache-2.0 | SA-V, SA-1B | A K4 célmodellje |
| EfficientSAM | Apache-2.0 | Apache-2.0 | SA-1B | Használható, ha a SA-1B feltételei megfelelnek |
| Cutie | MIT | MIT (repó szerint) | többek közt MOSE | Tisztázandó: a MOSE nem kereskedelmi |
| ONNX Runtime, `ort` | MIT; MIT OR Apache-2.0 | – | – | Rendben |

A modellsorokat a manifest felvétele előtt a modell saját oldaláról, rögzített revízióval igazolni
kell (ADR 0006 §2 és §8: négy licencmező, ismeretlen mező blokkol).

## 4. A hat lépés

| # | Lépés | Hely | Parancsok | AI-réteg kell? |
|---|---|---|---|---|
| 1 | Detection | worker képesség `cv.detect` | `ai.objects.detect` | Igen (F4) |
| 2 | Tracking | új `cfa4-objtrack` crate (L2): IoU, Kalman, Hungarian, SORT | `objects.track` | Nem |
| 3 | Segmentation | worker képesség `cv.segment`; promptmodell a `cfa4-project`-ben (L1) | `ai.objects.segment`, `objects.setPrompt` | A modellhez igen |
| 4 | Mask Propagation | worker képesség `cv.propagate`; AI nélküli tartalék: vetemítés a `track_step` transzformációjával | `ai.objects.propagate`, `objects.propagate` | A modellhez igen |
| 5 | Tracked Object Data | `cfa4-project` (L1) `ObjectTrack`; RLE-maszkok tartalomcímzett gyorsítótárban | `objects.list`, `objects.rename`, `objects.remove` | Nem |
| 6 | Effect Binding | `Mask.source`; raszteres lefedés a `mask.rs`-ben és a `GpuMask`-ban | `masks.add` `{"source":{"object":id}}` | Nem |

Kötelező szabályok:

- **Adatmodell:** `ObjectTrack { id, label, class, source: Manual | Detected | Segmented,
  samples: BTreeMap<Tick, ObjectSample>, prompts, provenance }`, `ObjectSample { bbox, score,
  mask: Option<MaskRef> }`. Az idő médiaidő `Tick`-ben; a `provenance` a modell azonosítója és SHA-256-ja.
- **RLE ellenséges bemenet** (AGENTS.md §0.4): a futáshosszak összege pontosan `width × height`,
  méretkorlát, `checked_*` aritmetika, mutációs fuzzteszt.
- **Visszafelé kompatibilitás:** `Mask.source` alapértéke `Path`, `#[serde(default)]`; régi projekt teszttel.
- **CPU-referencia, GPU gyors út:** előbb a CPU-lefedés arany képpel, utána a `GpuMask` textúrás változata, a CPU-hoz mérve.
- **Munkafolyamat** (ADR 0006 §4): az AI eredménye javasolt `ObjectTrack`; Előnézet után egy
  visszavonható Alkalmazás lépés teszi a klipre, projektverzió-ellenőrzéssel.
- **Hardver:** az eszközt az erőforrás-kezelő választja; CPU-ra csak engedéllyel lép vissza.

## 5. Mi marad C++-ban

| Rész | Nyelv | Indok |
|---|---|---|
| Tracked Object Data, Effect Binding, RLE | Rust | A szerkesztő folyamatában, renderelés közben fut |
| SORT-követés | Rust | Kis, olcsó port |
| EfficientSAM-prompt és Cutie-terjesztés | C++, a libopenshot kódjából | A legdrágább port (`CVObjectMask.cpp` 1247 sor, négy ONNX-gráf); külön folyamatban nem lassít |
| Detektálás | C++ vagy ONNX Runtime worker | AGPL-es YOLO-modell nem |

A C++-worker feltételei: külön folyamat az ADR 0006 protokolljával; az LGPL-fejlécek megmaradnak és
regisztersor készül; az eszközt a `load` üzenet adja meg (nincs automatikus visszalépés); a
referenciagépen mérve a lejátszás és a renderelés képkocka-ideje nem romlik, a worker a számítási
kártyát használja; az OpenCV a rendszercsomagból jön, a `cargo` munkaterületet nem érinti.

## 6. Ütemezés

| Szakasz | Tartalom | Kapu |
|---|---|---|
| A — AI nélkül | 5, 6, 2 lépés; 4 AI nélküli tartaléka | a tulajdonos végrehajtási döntése (nem felület, nem AI-crate, az F0 nem köti) |
| B — AI-réteg alapja | ADR 0006 F1–F3: modellkezelő, jegyzék, jobtár, ONNX/C++ worker, `routing.toml` sorok | F0: Qt6 kész, egui ki |
| C — AI-képességek | 3, 4, 1 lépés; Qt6 objektumpanel | F3: első AI-képesség |

Az A szakasz feladatai (egy feladat egy commit, `OM-A1` … `OM-A7`):

1. **OM-A1** `cfa4-project`: `ObjectTrack`, `ObjectSample`, `MaskRef`, ellenőrzött RLE-kódoló és -dekódoló, fuzzteszttel.
2. **OM-A2** `cfa4-project`: `Mask.source` (`Path` | `Object`), régi projektes teszttel.
3. **OM-A3** `crates/render/src/mask.rs`: raszteres lefedés (átméretezés, tágítás, toll), arany képpel.
4. **OM-A4** `cfa4-objtrack` (L2): IoU, Kalman, Hungarian, SORT, determinisztikus tesztekkel; `cargo xtask layers` szabály.
5. **OM-A5** `crates/engine`: `objects.createFromBox`, `objects.track`, `objects.list`, `objects.remove`, ellenséges paraméteres tesztekkel.
6. **OM-A6** `cfa4_gpu::GpuMask`: textúrás objektummaszk, a CPU-hoz mérve.
7. **OM-A7** `docs/licensing.md` §5 regisztersor (ha port kerül be), `docs/gaps.md` G12 frissítése.

Becslés (mérés nélkül): A szakasz 60–100 óra; C szakasz modellenként 30–60 óra; a B szakasz az ADR 0006 saját költsége.

## 7. Nyitott kérdések

- SAM2 egyedül, vagy EfficientSAM + Cutie (a libopenshot útja)? Javaslat: SAM2 a K4 szerint; a C++-worker indulhat EfficientSAM + Cutie párral, ha a Cutie tanítóadata rendben van.
- Melyik Apache-2.0 detektor legyen az első? 2K-s sebesség mérése a referenciagép RTX A4000 kártyáján.
- Az RLE-maszkok helye: javaslat a tartalomcímzett gyorsítótár, hash-hivatkozással a projektben.
- A `routing.toml` ai családja és az ADR 0006 ONNX Runtime döntése eltér; ezt az ADR 0006 gazdája húzza össze.

## Változások

| Dátum | Változás |
|---|---|
| 2026-11-03 | Első, jóváhagyott változat; elmentve, a végrehajtás külön tulajdonosi döntésre vár |
