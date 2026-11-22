# CFA4-ARF-001 – Adaptive Representation Fabric, 0.1

> **[MÓD-3] MÓDOSÍTÁS JÓVÁHAGYÁSRA VÁR** (egyeztetés: [`CFA4-DNEG-VFX-EGYEZTETES-001`](CFA4-DNEG-VFX-EGYEZTETES-001-0.1.md) K-3, K-7, K-9, D-3). Viszony a `CFA4-VFX-001`-hez, a jóváhagyás után:
> - Az ARF-reprezentációk a `G-xx` mátrixban sorok (`DNEG-M4`); külön lista nincs.
> - **Egy részecskemotor van:** a VFX-6 (`particles` bővítése). Az ARF *Points* (és az OpenVDB Points, pontfelhő) **bemenet** (forrás, emitter-eloszlás), nem szimuláció.
> - A worker/job-szerződés a render-terv C0 szerződése (worker protokoll, job API); az ARF-3 ezt használja, nem új protokollt.
> - Az ARF nem `VfxGraph`: a leíró csak hivatkozik a külső adatra (út + SHA-256), a kompozíciós szerkesztési adat a VFX-2 dolga.
> **[MÓD-8] 3D videó-projekt-függő (István, 2026-11-22 15:36; egyeztetés: [`CFA4-DNEG-VFX-EGYEZTETES-001`](CFA4-DNEG-VFX-EGYEZTETES-001-0.1.md) K-5, D-12).** A VISION §8 kérdése így dőlt el: a 3D nem globális szint, hanem attól függ, mit importált a felhasználó az adott projektbe; a 3D-állapot a projektben tárolódik, 3D-tartalom nélküli projektet nem érint. Az ARF-leírók a projektbe importált forrásból keletkeznek (nincs globális 3D-mód); az „ARF-2 kódja a VISION §8-tól függ” előfeltétel ennek megfelelően szűkül: az ARF-0/1 (leírás, séma) indulhat, a kód külön fázisengedéllyel. A jóváhagyásig a 0.1 szöveg érvényes.

> **Állapot:** JÓVÁHAGYVA (István, 2026-11-21 22:45, Europe/Budapest). **Kódolási engedélyt nem ad:** minden fázis külön tulajdonosi engedélyt igényel (coordination.md §2.10). **Dátum:** 2026-11-21 · **Kérte:** István (ubuntuokos).
> **Alap:** CFA4 `main` @ `97d7982f`; ez a dokumentum csak dokumentáció.
> **Forrás a tartalomhoz:** a tulajdonos DNEG-megbízásának 4.2. pontja nevezi meg az ARF elemeit: *Mesh, Points, Volume, Curves, Instances, Impostors, LOD, point-cloud processing, streaming, spatial indexing*. A `main`-ben az ARF-ről semmi nem volt; a terv ezekből és a CFA4 meglévő szabályaiból (ACS, ADR-0004) épül. Ahol találgatnom kellene, **nyitott döntésként** jelölöm.
> **Előfeltétel a megvalósításhoz:** VISION §8 nyitott döntése: a 3D/VFX mélysége. Az ARF 0–1. fázisa (leírás, séma) ettől független; kódot csak e döntés után lehet indítani.

## 1. Cél

Egyetlen **reprezentáció-réteg** (adatmodell + kiválasztási szabály), amely a térbeli adatot – háló, pontok, voxel, görbék, példányok, impostor – egységesen írja le, és az erőforrás-keretnek (ACS) megfelelően választ részletességet (LOD), betöltési régiót és gyorsítótárat. **Nem renderer, nem szerkesztő, nem új idővonal.**

## 2. Kiinduló állapot

| Tény | Forrás |
|---|---|
| 3D, pontfelhő, volumetria: sem terv, sem kód | `VISION.md` §4, §8; DNEG-audit §0.2 |
| Erőforrás-keret: feladattípusok, memóriakeret, prioritás, ellenőrzőpont, jóváhagyás minőségcsökkentéshez | ACS-összevetés, TASKSTORE-PG-001, ADR-0004 |
| 2D `particles` effekt (nem térbeli) | `crates/project/src/effect/vfx.rs:509` |
| Külső formátumok jelöltjei: OpenVDB (Apache-2.0, igazolt), USD (upstream), pontfelhő (Potree/LAS/…) | DNEG-audit D8; [OPENUSD-3D](CFA4-OPENUSD-3D-001-1.0.md), [POINTCLOUD](CFA4-POINTCLOUD-001-0.1.md) |

## 3. A reprezentációk (a tulajdonos listája szerint)

| Reprezentáció | Leírás | Tipikus forrás | Külső munka, amelyre épül |
|---|---|---|---|
| **Mesh** | háromszög-/soklapháló, anyag-hivatkozás | USD, glTF, OBJ, STL | OPENUSD-3D |
| **Points** | pont + típusos attribútumok + csoportok | LAS/LAZ/E57/PLY, OpenVDB Points | POINTCLOUD; DNEG-terv N1 (olvasó) |
| **Volume** | ritka voxelrács, több grid | `.vdb` | DNEG-terv N1/N2 (OpenVDB, Apache-2.0) |
| **Curves** | spline/szál (haj, pálya) | USD BasisCurves | OPENUSD-3D |
| **Instances** | egy forrás sok transzformmal | USD PointInstancer, glTF EXT_mesh_gpu_instancing | OPENUSD-3D |
| **Impostor** | előrenderelt síkbeli helyettesítő távoli részletességre | generált (worker) | ARF-saját |

## 4. Mit ad az ARF (és mit nem)

**Ad:**
1. **Leíró séma** (Rust-típusok, L1/L2): reprezentáció-azonosító, forrás-hivatkozás (út + SHA-256, a projekt csak hivatkozik), bbox, transzform, attribútum-lista (név, típus, tömörítés), LOD-szintek listája (mindegyiknek becsült memória/idő).
2. **LOD- és betöltési szabály:** egy **tiszta függvény** `(leíró, keret, nézet) → terv`, determinisztikus, tesztelhető; a keretet az ACS adja (memória-, idő-, minőségi szint). Minőségcsökkentés **csak jóváhagyással**, ha az eredményt változtatja (ACS ütközés-3 szabálya).
3. **Régió-lekérdezés szerződés:** `bbox/frustum → rész-leíró`; a konkrét I/O a forrás-munkáké (worker).
4. **Leképezési táblázat** ARF-attribútumok ↔ OpenVDB Points, ARF ↔ USD-prim-típusok (a DNEG-audit D4-hez).

**Nem ad:** rendert, GPU-motort, új szerkesztőt, adattárolást (a PostgreSQL/pgvector DATA-PG-001 dolga), konverziót formátumok közt (a worker-munkák).

## 5. Fázisok és kapuk

| Fázis | Tartalom | Kapu / kilépés |
|---|---|---|
| ARF-0 | Ez a terv + séma-vázlat jóváhagyása | tulajdonosi jóváhagyás; **VISION §8 döntés** a kód előtt |
| ARF-1 | Leíró séma és LOD-szabály specifikációja, leképezési táblázatok | ARF-A1…A3 (dokumentum- és tesztterv szinten) |
| ARF-2 | `cfa4-arf` crate (L1, nincs `unsafe`, nincs I/O): típusok + szabály-függvény | ARF-A4…A8 zöld |
| ARF-3 | Integráció az ACS-tervvel (a worker-manifest a leírót használja) | ARF-A9 zöld |

## 6. Elfogadási feltételek

- **ARF-A1:** a séma lefedi mind a hat reprezentációt, és minden mezőnek van típusa, egysége és érvényességi tartománya (specifikáció).
- **ARF-A2:** a leképezési táblázat OpenVDB Points ↔ ARF minden sorához van „megőrzött / elveszett / jelentett” besorolás (nincs csendes veszteség).
- **ARF-A3:** minden LOD-döntés indoklást ad (melyik keret miatt), az ACS `route_decision` elvével.
- **ARF-A4:** a szabály-függvény determinisztikus: azonos bemenet → azonos terv (property-teszt).
- **ARF-A5:** hibás/hostile leíró (negatív méret, NaN, túl nagy szám, ciklikus hivatkozás) → `Err` (AGENTS §0.4–0.5, mutation-fuzz).
- **ARF-A6:** memóriakeret alatt a terv sosem lépi túl a keretet; túl kicsi keret → „nem futtatható” indoklással, nem összeomlás.
- **ARF-A7:** minőségcsökkentés jóváhagyás nélkül nem történik, ha az eredmény változik (teszt).
- **ARF-A8:** `cargo xtask layers` zöld: az ARF nem függ UI-tól, I/O-tól, külső C++ könyvtártól.
- **ARF-A9:** CPU-only; GPU nem feltétel.

## 7. Licenc, függőség, erőforrás

Új külső függőség az ARF-ben **nincs** (tiszta Rust, GPL-3.0-or-later). A külső formátumok licenceit a megfelelő tervek kezelik (OpenVDB Apache-2.0 igazolt; OpenUSD: TOST-1.0; Potree BSD-2). Erőforrás: az ACS-hez kapcsolódik, nem új rendszer.

## 8. Nyitott döntések (tulajdonosnak)

1. **VISION §8:** a 3D/VFX mélysége (egyszerű 3D-címek vs teljes jelenetszerkesztés) – az ARF-2 kódja ettől függ.
2. Az „Impostor” pontos jelentése a CFA4-ben (a javaslat: generált sík-helyettesítő nagy távolságra).
3. Az ARF külön crate legyen-e, vagy a `cfa4-project` része (javaslat: külön L1 crate).
