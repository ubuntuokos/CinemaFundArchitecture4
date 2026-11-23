# CFA4-POINTCLOUD-001 – pontfelhő (Potree-donorvizsgálattal), 0.1

> **[MÓD-7] MÓDOSÍTÁS JÓVÁHAGYÁSRA VÁR** (egyeztetés: [`CFA4-DNEG-VFX-EGYEZTETES-001`](CFA4-DNEG-VFX-EGYEZTETES-001-0.1.md) K-3, K-7, D-3). A jóváhagyásig a 0.1 szöveg érvényes; utána:
> - A PC-4 render-kimenet a render-terv C0 job-szerződésén és a VFX-001 X-5 EXR-szerződésén megy (a DNEG N2 mintája), nem külön protokollon.
> - A pontfelhő-effektek (scatter, térbeli maszk) a VFX-6 mátrixsorain jelennek meg; második részecskemotor nincs. A pontfelhő mátrixsora: `DNEG-M6`.
> **[MÓD-8] (jóváhagyva 2026-11-23 07:37) 3D videó-projekt-függő (István, 2026-11-22 15:36; egyeztetés: [`CFA4-DNEG-VFX-EGYEZTETES-001`](CFA4-DNEG-VFX-EGYEZTETES-001-0.1.md) K-5, D-12).** A VISION §8 kérdése így dőlt el: a 3D nem globális szint, hanem attól függ, mit importált a felhasználó az adott projektbe; a 3D-állapot a projektben tárolódik, 3D-tartalom nélküli projektet nem érint. A pontfelhő-képesség csak olyan projektben aktív, amely pontfelhőt importált; a PC-0 előfeltétele („VISION §8”) ennek megfelelően szűkül, a „kell-e egyáltalán” (§8 2. pont) kérdés projektfüggő bekapcsolással kezelhető. A jóváhagyásig a 0.1 szöveg érvényes.
> - **Donor-tények újraolvasva (2026-11-22):** `potree/potree` HEAD 2026-11-20 (BSD-2, © 2011–2020 Markus Schütz); `potree/PotreeConverter` HEAD 2026-11-02, legutóbbi címke 2.1.5 (BSD-2, © 2020).

> **Állapot:** JÓVÁHAGYVA (István, 2026-11-21 22:45, Europe/Budapest). **Kódolási engedélyt nem ad:** minden fázis külön tulajdonosi engedélyt igényel (coordination.md §2.10). **Dátum:** 2026-11-21 · **Kérte:** István (ubuntuokos).
> **Alap:** CFA4 `main` @ `97d7982f`; ez a dokumentum csak dokumentáció.
> **Előfeltétel kódhoz:** VISION §8 (3D/VFX mélysége) és az [ARF](CFA4-ARF-001-0.1.md) jóváhagyása.

## 1. Cél

Pontfelhő-adat (LiDAR/fotogrammetria) **beolvasása, nagy adatnál is memóriahatékony kezelése és megjelenítése** a CFA4-ben az ARF reprezentáció-rétegére építve. Nem önálló particle engine, nem új szerkesztő.

## 2. Kiinduló állapot és igazolt tények

| Tény | Ellenőrzés |
|---|---|
| A `main`-ben pontfelhő-terv/kód nincs; a 2D `particles` effekt nem pontfelhő | DNEG-audit §0.1 |
| **Potree** (webes, WebGL-es megjelenítő): BSD-2-Clause jellegű licenc (Markus Schütz, 2011–2020), aktív (HEAD 2026-11-20) | `potree/potree` `LICENSE`, 2026-11-21 |
| **PotreeConverter** (C++; pontfelhő → Potree-oktafa): BSD-2-Clause (2020), HEAD 2026-11-02 | `potree/PotreeConverter` `LICENSE`, 2026-11-21 |
| A Potree viewer JavaScript/WebGL → **közvetlenül nem használható** a Qt6/Rust CFA4-ben | természetéből; a CFA4 nem webes |
| OpenVDB Points (Apache-2.0) mint alternatív/kiegészítő pont-tároló: igazolt, `openvdb/points/` | DNEG-audit D8 |

## 3. Donor-besorolás (a Potree-vizsgálat eredménye)

| Elem | Besorolás | Indok |
|---|---|---|
| Potree viewer (JS/WebGL) | **D** – csak referencia | nem illeszthető a Qt6/Rust-világba; az oktafa-LOD és a láthatósági ötletek referenciák |
| Potree oktafa-formátum (a „Potree 2.0” elrendezés) | **referencia/formátum** | olvasása elegendő lehet; a pontos specifikációt a G-PC-1 audit olvassa |
| PotreeConverter | **külső eszköz (opcionális)** | C++ konverter, BSD-2; futtatható *külön folyamatban* felhasználó által kezdeményezve; a CFA4 nem tölt le semmit |
| OpenVDB Points | **B** | pont+attribútum+csoport egy VDB-fában; a DNEG-terv N1 olvasója már felvehet ilyet |

## 4. Hatókör

1. **Formátumok (beolvasás):** LAS/LAZ, E57, PLY, (COPC és Potree-oktafa a G-PC-1 audit után). A konkrét Rust-crate-ek vagy C++ könyvtárak (LAZ-dekompresszió, E57) **verziója és licence még ellenőrizendő**; ismeretlen/nem GPL-kompatibilis nem megy be.
2. **Memóriahatékonyság:** régió-alapú betöltés, oktafa-LOD, részleges hozzáférés, gyorsítótár – az ARF LOD- és régiószabálya, az ACS memóriakerete alatt.
3. **Attribútumok és szűrés:** szín, intenzitás, osztály, GPS-idő; szűrés régió/attribútum szerint; pontcsoport (a CFA4-ben az ARF-leképezés szerint).
4. **Megjelenítés:** előnézet a Qt6-ban (CPU-s alap; GPU opcionális, gyártófüggetlen wgpu); **render a kompozitba** a DNEG-terv N2 mintájával: worker/modul → EXR/PNG sorozat → sima média. A render-motorhoz nem nyúlunk.
5. **Effektek (scatter, térbeli maszk, procedurális eloszlás):** nincs e tervben; a [VFX-katalógus](CFA4-VFX-CATALOG-001-0.1.md)/DNEG G5 kapu keretében külön.

**Nincs benne:** önálló particle engine, webes viewer, felhő-függőség, automatikus letöltés, külön projektmodell.

## 5. Fázisok és kapuk

| Fázis | Tartalom | Kapu |
|---|---|---|
| PC-0 | Jóváhagyás + ARF-0 + VISION §8 | tulajdonosi döntés |
| PC-1 (G-PC-1) | Donor- és formátumaudit: Potree-oktafa spec, LAZ/E57/COPC könyvtárak licence és karbantartása, Ubuntu 26.04 | licencek igazolva; ismeretlen = nem megy be |
| PC-2 | Olvasó (LAS/LAZ, PLY) ARF-leíróval, régióbetöltéssel | PC-A1…A6 |
| PC-3 | LOD/streaming, gyorsítótár, ACS-integráció | PC-A7…A9 |
| PC-4 | Előnézet, render-kimenet (EXR/AOV) | PC-A10…A12 |

## 6. Elfogadási feltételek

- **PC-A1:** kis és nagy (≥ 100 millió pont) fájl beolvasása a memóriakereten belül (cgroup-korláttal mérve).
- **PC-A2:** attribútumok megőrzése (szín/intenzitás/osztály/idő) beolvasás→leíró→kiírás körben; a nem támogatott attribútum jelentve.
- **PC-A3:** régió-lekérdezés csak a kért pontokat tölti be (mért betöltött adatmennyiség ≤ régió + küszöb).
- **PC-A4:** hibás/csonka/hostile fájl → `Err`, nincs összeomlás (mutation-fuzz, AGENTS §0.8).
- **PC-A5:** CPU-only működés.
- **PC-A6:** projektmentés/-betöltés: a projekt csak hivatkozik a fájlra (út + SHA-256).
- **PC-A7:** LOD-váltás determinisztikus; minőségcsökkentés csak jóváhagyással, ha az eredményt változtatja.
- **PC-A8:** OOM-védelem: túl kicsi keret → indokolt „nem futtatható”, nem összeomlás.
- **PC-A9:** háttérfuttatás, megszakítás; a GUI reszponzív.
- **PC-A10:** előnézet Qt6-ban, automation-id-vel; **PC-A11:** render-kimenet sima médiaként a kompozitba; **PC-A12:** regressziókapu (idővonal, render, export, projekt, Qt6).

## 7. Licenc, függőség

Potree és PotreeConverter BSD-2 (igazolt a `LICENSE` fájlokból) – megengedett (licensing.md §3). A tényleges függőségek (LAZ/E57 dekódolók) licence **a PC-1 auditig ellenőrizetlen**. Nincs automatikus letöltés; külső konverter csak felhasználói indításra.

## 8. Nyitott döntések

1. **VISION §8:** a 3D/VFX mélysége.
2. A pontfelhő kell-e egyáltalán a CFA4-be (a DNEG-terv a „spatial” képességet vizsgálta, de VISION nem említi) – a javaslat a tulajdonos igenje után indul.
3. Potree-oktafa olvasása vs. saját oktafa-formátum.
