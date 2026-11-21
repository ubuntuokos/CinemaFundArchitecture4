# CFA4-DNEG-TECHNOLOGY-ADAPTATION-001 – végleges integrációs terv (D10), 1.0

> **Állapot:** JAVASOLT VÉGLEGES 1.0 – tulajdonosi jóváhagyásra vár (az öt kapcsolódó terv 0.1 jóváhagyva 2026-11-21). A terv elfogadása **nem
> implementációs engedély**: minden fázis külön tulajdonosi engedélyt igényel (coordination.md §2.10).
> **Dátum:** 2026-11-21 · **Kérte:** István (ubuntuokos) · **Alap:** CFA4 `main` @ `97d7982f`
> **Bizonyíték (D1–D9):** [`CFA4-DNEG-TECHNOLOGY-ADAPTATION-001-AUDIT-D1-D9.md`](CFA4-DNEG-TECHNOLOGY-ADAPTATION-001-AUDIT-D1-D9.md)
> **Platform:** Linux amd64, Ubuntu 26.04; Conda/Miniforge kizárva; automatikus letöltés nincs; az eredeti DNEG-projektek nem módosulnak.
> Ez a dokumentum csak dokumentáció: **kód nem módosult**.
> **Frissítve 2026-11-21 (hálózati ellenőrzés után):** a donor-licencek és a USD-fork diff igazoltak (audit D7, D8); a hiányzó tervekhez új, külön tervek készültek (§1 pont 6).

## 1. Eredmény röviden

1. **A DNEG-forkok egyike sem kerül be** (openvdb, openvdb_ax, openvdb_points_dev, USD, aaf). Technológiai donorként az **ASWF upstream** OpenVDB (core, AX, points) jöhet szóba – az ADR-0003 A7 ezt már megnevezi („3D/VFX-szakasz, külön tervezendő”), ez a terv annak első, szűk szeletét részletezi.
2. **AAF: nincs új fejlesztés.** A CFA4-nek már van saját, clean-room AAF import+export. F4 döntés: **3. lehetőség** (a meglévő fedi). A DNEG AAF SDK forrása a clean-room szabály miatt nem is olvasható.
3. **USD: a DNEG-fork nem kell.** Igazolt: a `dneg/USD` `HEAD`-je azonos az upstream `v23.05` tag commitjával (`b53573ea…`), a delta üres, a fork 2023-ban megállt. USD csak upstream OpenUSD lehet, a [`CFA4-OPENUSD-3D-001`](CFA4-OPENUSD-3D-001-0.1.md) terv keretében.
4. **Valóban új** (C) csak: `.vdb` volume-adat kezelése (N1), annak renderelt megjelenítése a meglévő médiaútvonalon (N2), és – feltételesen, biztonsági kapu mögött – térbeli kifejezés (AX, N3).
5. **Erőforrás-kezelés (D9): nincs új rendszer**; a worker az ACS/TASKSTORE keretbe regisztrál.
6. **Öt hivatkozott terv nem volt a `main`-ben** (audit §0.1). A tulajdonos döntése szerint mindegyikhez új terv készült (javaslat, jóváhagyásra vár): [VFX Catalog](CFA4-VFX-CATALOG-001-0.1.md), [EffectCraft](CFA4-EFFECTCRAFT-001-0.1.md), [ARF](CFA4-ARF-001-0.1.md), [OpenUSD/3D](CFA4-OPENUSD-3D-001-0.1.md), [Pontfelhő/Potree](CFA4-POINTCLOUD-001-0.1.md). Az általuk lefedett képességek **nem kerülnek e terv scope-jába**; az N1–N3 a G0 kapu után indulhat.
7. **Licenc (igazolt a repókból, 2026-11-21):** az ASWF OpenVDB (core, points, NanoVDB, AX; 14.0.0) **Apache-2.0** (a közreműködők az MPL-2.0-ról relicenszeltek, `RE-LICENSE_NOTE.txt`); a DNEG-forkok MPL-2.0-s, 2017–2023-ban megállt pillanatképek (a points-repó deprecated, licencfájl nélkül); a DNEG AAF SDK saját *AAF SDK PSL 2.0*, GPL-3-kompatibilitása nem igazolt. **Ubuntu 26.04 csomagverziók továbbra is ELLENŐRIZETLENEK** (G1).

## 2. Alapelvek (a megbízás 0. pontja és a CFA4-szabályok)

- A CFA4 projektmodellje, Rust-motorja, Qt6 GUI-ja, idővonala, render- és VFX-architektúrája **változatlan marad**; a donorok nem írják felül.
- **Nincs párhuzamos rendszer:** nincs új editor, timeline, projektmodell, render engine, particle/volume motor, kompozitor.
- **Futtatási modell** (AGENTS „Port or keep the original language”, ADR 0001 / worker-elv): az OpenVDB C++ marad, **külön felügyelt worker-folyamatban** fut, Rust-interfész mögött; a CFA4-folyamatban nincs `unsafe`, nincs in-process VDB. Összeomlása hiba, nem a CFA4 összeomlása. „Nem lassít”: a lejátszás és render képkockaidői a worker futása alatt változatlanok (referenciagépen mérve). Az eredet-megőrzés indoka: nincs érett, GPL-kompatibilis Rust-megfelelő (**ELLENŐRIZETLEN**: nem kerestem Rust-portot); a port ellenjavallt, mert aránytalanul hosszú.
- **Never crash:** hibás/csonka/hostile `.vdb` → `Result::Err`, mutation-fuzz teszt.
- **Gyártófüggetlenség:** CPU az alap és kötelező; GPU opcionális; CUDA/NanoVDB-CUDA csak vendorfüggetlen párral.
- **Qt6:** minden felületi elem a `python/cfa4_qt`-ban; minden művelet engine-parancs automation-id-vel (CLAUDE.md).
- Fejlesztés **csak jóváhagyott terv és fázisengedély alapján**; terv-módosítás megállítja a munkát.

## 3. Fázisrend és kapuk

A megbízás F10 sorrendje szerinti munkacsomagok (F0…F9) megmaradnak; a kapuk a CFA4 saját fázisaira vannak leképezve. **Sikertelen vagy bizonyítatlan fázis nem engedi automatikusan a következőt.** Minden fázis külön tulajdonosi engedélyt kér.

| Kapu | Mi | Belépési feltétel | Kilépési (átvételi) feltétel |
|---|---|---|---|
| **G0** | **Terv-alap**: a VFX Catalog / EffectCraft / ARF / OpenUSD-3D / Pontfelhő tervek (`docs/plans/*-0.1.md`, a tulajdonos 2026-11-21-én jóváhagyta) a `main`-ben | ez a terv jóváhagyva | a D1 `?` sorai lezárva (A vagy B/C), a D4 leképezési táblázat (ARF ↔ OpenVDB Points) elkészült |
| **G1** | **OpenVDB csomagfelmérés (F8 második fele)** – a licenc már igazolt (Apache-2.0) | G0; 26.04-es csomagforrás | libopenvdb/oneTBB/zlib/Zstd (+LLVM ≥ 18 AX-hez) **Ubuntu 26.04-es** verziója és transzitív licence dokumentálva; ADR-0003 táblasora javítva („Apache-2.0”); `cargo deny`/SBOM-terv. **Ha a 26.04-es csomag nem teljesíti a minimumot (TBB ≥ 2021, LLVM ≥ 18) → a fázisok megállnak.** |
| **G2** | **N1 – VDB adatréteg (F1)** | G0, G1, ACS-6 (PR #1471) beolvasva | N1 átvételi feltételek (§4) mind zöldek |
| **G3** | **N2 – Volume megjelenítés (F6 része)** | G2 | N2 feltételek zöldek |
| **G4** | **N3 – AX (F2)** – *csak biztonsági PoC után* | G2, CFA4-SECURITY-001 B3/B4 (a folyamat-sandbox alkalmazva) | sandbox-PoC zöld (§4 N3-S) + a döntés: AX-worker vagy saját interpreter; N3 feltételek zöldek |
| **G5** | **Volumetrikus/procedurális VFX-bővítés (F6 többi)** | G3 és (G4, ha AX-et használ) | külön, e terv *módosításaként* jóváhagyott lista (VFX-katalógus tényleges hiányai G0 után ismertek); jelenleg **nincs scope** |
| **G6** | **Erőforrás-illesztés (F7)** | G2 | N4 feltételek zöldek |
| **G7** | **USD differenciális audit (F5)** | – | **lezárva (2026-11-21): a delta üres**, a DNEG-fork nem kell; az upstream OpenUSD integrációja a [`CFA4-OPENUSD-3D-001`](CFA4-OPENUSD-3D-001-0.1.md) terv dolga |
| **G8** | **AAF (F4)** | – | **lezárva: 3. döntés**; nincs további fázis; az opcionális külső AAF-orákulum külön, kicsi PR (licenc előbb) |
| **G9** | **Validációs keret (F9)** | G2-től minden fázissal együtt | §6 regressziókapu minden fázis PR-ján zöld |

Az időzítés **tulajdonosi döntés**: VISION §6 szerint a 3D/VFX az utolsó új terület, a Qt6-felület az első prioritás.

## 4. Végső scope és konkrét elfogadási feltételek

**Csak ez kerül a scope-ba;** minden más a D1-ben „A/B?/D/E”, és nem része a fejlesztésnek. A feltételek „bizonyíték, nem állítás” szerint teszttel vagy mérési jegyzőkönyvvel igazolandók; egy nem futtatott teszt **nem** teljesített.

### N1 – VDB adatréteg (C; G2)

Egy `cfa4-vdb` Rust-crate (L1/L2 típusok, `Result`, nincs `unsafe`) + külön folyamatú `cfa4-vdb-worker` (C++, rendszer-OpenVDB dinamikus linkkel) a `cfa4-sandbox`-profilban. A projekt csak hivatkozik a `.vdb`-re (útvonal + SHA-256); a VDB nem ír a projektstruktúrába.

| ID | Feltétel | Bizonyíték |
|---|---|---|
| N1-1 | `vdb.inspect` kiolvassa: grid-lista, típus (float/double/vec3/int/bool/points), voxel-méret, transzform, aktív voxel-szám, bbox, metaadat | teszt szintetikus VDB-n (a worker tesztkészlete generálja futáskor; bináris fixture nincs a tárban) |
| N1-2 | Kis és nagy (≥ 1 GiB aktív adat) fájl; ritka és sűrű | memóriamérés `cgroup`-korláttal; csúcs-RSS a keretben |
| N1-3 | Hibás/csonka/bitroncsolt fájl → hiba, sem pánik, sem worker-összeomlás a CFA4-ben | mutation-fuzz `catch_unwind` alatt (AGENTS §0.8); a worker halála → `Err`, a CFA4 él |
| N1-4 | Round-trip: olvasás → írás → olvasás, grid- és metaadat-azonosság | bitazonos érték a támogatott típusokra |
| N1-5 | Képsorozat (`név.####.vdb`) a meglévő sorozat-importon át; **nincs új idővonal** | a kiegészítés csak a média-eszköz típusa |
| N1-6 | CPU-only, GPU nélkül | mindkét NVIDIA-kártya elrejtve, `--hw off` |
| N1-7 | Mentés/visszaállítás a projektben (hivatkozás, relink hiányzó fájlnál jelentéssel) | projekt-round-trip teszt |
| N1-8 | A worker nem kap hálózatot és csak a megengedett útvonalat látja | sandbox-profil teszt |
| N1-9 | Lejátszás/render képkockaidő változatlan, amíg a worker fut | referencia-mérés előtte/utána |
| N1-10 | Qt6: a médiaböngészőben megjelenik, a „Voxel-attribútumok” panel olvasható; minden elem automation-id-vel | Qt6 GUI-teszt |

### N2 – Volume megjelenítés renderelt médiaként (C; G3)

A worker CPU-n **sűrűség-/mélységelőnézetet és AOV-t renderel** EXR-be (PNG előnézet), a projekt gyorsítótárába; a CFA4 azt ismert médiaként használja (kompozitor, render változatlan). Függőség: a többrészes EXR olvasás (ADR-0003 A3) – ha az nincs meg, az egyrészes EXR/PNG elég az első körben.

| ID | Feltétel | Bizonyíték |
|---|---|---|
| N2-1 | Előnézeti minőségi szintek (felbontás, voxel-lépés, mintaszám); minőségcsökkentés **csak** felhasználói jóváhagyással | teszt: automatikus lépés nem változtatja az eredményt |
| N2-2 | Determinisztikus eredmény azonos bemenetre és szálszámra (ahol garantálható: dokumentált tűrés a szálszám-függéshez) | két futás összevetése, tűrés a jegyzőkönyvben |
| N2-3 | Az eredmény sima klipként kerül az idővonalra és a kompozitba; **nincs** új render-útvonal a motorban | `git diff` a `crates/render`, `crates/gpu` fájlokra: 0 |
| N2-4 | Megszakítható; időkeret; OOM → hiba | cancel-teszt, memóriakorlátos teszt |
| N2-5 | Háttérben fut, a GUI reszponzív marad | UI-késleltetés mérés |

### N3 – AX térbeli kifejezés (C, feltételes; G4)

Csak a G4 után. Nincs külön DNEG-szerkesztő: a kifejezés az Effect Controls egyik effekt-szerű bejegyzése, paraméterekkel és preset-könyvtárral; a Qt6-os szerkesztő a meglévő szerkesztőfelület része.

**N3-S – sandbox-PoC (kötelező előfeltétel):**

| ID | Feltétel |
|---|---|
| N3-S1 | A JIT külön folyamatban fut Landlock+seccomp profillal; nincs shell/exec, nincs fájl-írás a megengedett kimeneten kívül, nincs hálózat (próba: a kifejezések `system`-hívási kísérlete elbukik) |
| N3-S2 | Kemény időkeret és memóriakeret; a túllépés a worker megölése, hiba a felhasználónak |
| N3-S3 | Hostile kifejezés-fuzz (parser/compile) 0 összeomlás a CFA4-ben |
| N3-S4 | LLVM mint függőség: verzió, licenc és Ubuntu 26.04 csomag igazolva (G1 kiterjesztése); ha nem teljesül, a döntés a saját, korlátozott interpreter |

**N3 funkcionális feltételek:**

| ID | Feltétel |
|---|---|
| N3-1 | Pont-kontextus és volume-kontextus kifejezés; több attribútum |
| N3-2 | Compile-hiba és futás közbeni hiba pontos helymegjelöléssel, nem összeomlás |
| N3-3 | Megszakítás; nagy adathalmaz; CPU-only |
| N3-4 | Determinisztikus eredmény, ahol garantálható (dokumentálva, hol nem) |
| N3-5 | Kifejezés mentése, újrahasználata, preset; projekt-round-trip |
| N3-6 | Szintaxiskiemelés, függvénylista, validálás, compile-állapot, előnézet a Qt6 GUI-ban (automation-id, parancsok) |
| N3-7 | A projektbetöltés nem futtat kifejezést jóváhagyás nélkül (biztonságos betöltés) |

### N4 – Erőforrás-illesztés (G6)

Feladattípusok (`vdb.inspect`, `vdb.render`, `vdb.ax`) regisztrálása az **ACS TaskHost-ban**; új ütemező nincs.

| ID | Feltétel |
|---|---|
| N4-1 | A worker-manifest: útvonalak (CPU kötelező, mérten validált), memóriaszükséglet, `cancellable`, `resume` (szakaszhatár) |
| N4-2 | `tasks.check`/`decide` helyesen dönt: CPU-s validált út van → elérhető; nincs → indokolt „nem futtatható” |
| N4-3 | Minimumgép-mérés (16 GB, 8 mag) cgroup-korláttal: lefut, lassabban is, összeomlás és lefagyás nélkül |
| N4-4 | Nagy volume és nagy VDB-pontrács: partial/régió feldolgozás, a memóriakeret nem sérül |
| N4-5 | GPU opcionális: ha van gyártófüggetlen útvonal, ugyanaz az eredmény a tűrésen belül; ha nincs, CPU |

### Kimarad (nem scope, D1 szerint A/B?/D/E)

2D effektek, effektlánc, kulcskocka, kompozit, GPU-motor, plugin-hosting (G11/A4), idővonal, interchange (AAF/OTIO/EDL/…), médiaformátumok, glTF/OBJ/STL/3D-jelenet, pontfelhő-streaming/octree/Potree/ARF, USD, OpenFX, Media Intelligence-térbeli metaadat, NanoVDB-CUDA, minden DNEG-fork.

## 5. Licenc, függőség, build (D8 szabályai)

- Függőség csak Ubuntu-tárolóból, pinelve, `Depends/Recommends` a `.deb`-ben, SBOM-mal; AppImage/Flatpak külön packaging-audit; **nincs build-idejű vagy futásidejű letöltés.**
- Kód-átvétel nincs; ha a G1 szerint mégis kell, a `docs/licensing.md` §5 sor és a fejlécek ugyanabban a commitban.
- Az OpenVDB licence **igazolt: Apache-2.0** (megengedett, licensing.md §3). A 26.04-es csomagverziók és a transzitív licencek (oneTBB, zlib, Zstd, LLVM) a G1-ig **ellenőrizetlenek**; GPL-2.0-only/AGPL/ismeretlen licencű függőség esetén a fázisok megállnak. A DNEG AAF SDK (PSL 2.0) nem GPL-kompatibilisnek tekintendő jogi vélemény nélkül.
- Python csak a meglévő CFA4-környezetben (`.venv`, `--require-hashes`); a worker nem igényel Python-t.

## 6. Regressziókapu (minden fázis PR-jára)

A következők nem romolhatnak: idővonal, média-import, render, export, kompozit, VFX, kulcskocka, projektmentés/-betöltés, Qt6 GUI, meglévő kodek-munkafolyamat. Bizonyíték: a teljes validáció (`cfa4-full-validation`) zöld **azon a fejen**; `cargo xtask layers`, `cargo xtask assets`, `cargo deny check licenses`; a worker nélküli futás (OpenVDB nincs telepítve) a CFA4 többi részét nem érinti (opcionális `Recommends`, a funkció „nem elérhető”, nem hiba).

## 7. Tesztkeret (F9 összefoglaló)

| Terület | Teszt |
|---|---|
| VDB | import, grid-típusok, transzform, round-trip, hostile-fuzz, nagy fájl, memória |
| AX (feltételes) | parser, compile, futtatás, pont-/volume-kontextus, hibás kifejezés, timeout/cancel, erőforráskorlát, sandbox-kiszökés kísérletek |
| Points | csak olvasó inspekció (G0 után bővíthető) |
| AAF | meglévő `aaf_omf` + hostile; új: nincs |
| USD | G7 |
| GUI | Qt6 panelek, automation-id, reszponzivitás, háttérfuttatás |
| Erőforrás | CPU-only, kevés RAM, nagy adat, cancel, cache |

A nehéz mérések a `docs/development/reference-machine.md` szabályai szerint (minimumgép: cgroup-korlát; CPU-only: GPU-k elrejtve).

## 8. Kockázatok és nyitott pontok a tulajdonosnak

| # | Pont | Javaslat |
|---|---|---|
| Q1 | Az öt új terv (VFX Catalog, EffectCraft, ARF, OpenUSD/3D, Pontfelhő) jóváhagyása | Tartalmuk a te szövegedből és a repó állapotából következtetett javaslat; kérlek, nézd át, mielőtt a `?` sorok lezárulnak. |
| Q2 | OpenVDB-licenc | **Lezárva:** Apache-2.0 (igazolt). Az ADR-0003 táblasorát („BSD-3 / Apache-2.0”) „Apache-2.0”-ra javítani külön kis PR-ban. |
| Q3 | Az N1 külön folyamatú C++ worker (AGENTS „keep original language”) – elfogadod? | Javasolt: igen; az alternatíva (Rust-port) aránytalan. |
| Q4 | AX: LLVM-JIT worker vagy saját korlátozott interpreter? | G4 PoC-ból döntsön; addig egyik sem engedélyezett. |
| Q5 | A 3D/VFX-mélység (VISION §8) nyitott döntés; ez a terv ezt nem dönti el | Az N1–N2 csak adat+megjelenítés; 3D-jelenet nincs scope. |
| Q6 | Opcionális külső AAF-orákulum (licenc után)? | Nem sürgős; külön PR. |
| Q7 | A megbízás sorrendje szerint az F8 (licenc) a 9. lépés; a CFA4-szabály szerint integráció előtt kötelező | Megoldás: a G1 belépési kapu **minden** fázis előtt – ez nem tervmódosítás a sorrenden, csak előfeltétel. |

## 9. Összefüggés a megbízás elfogadási kritériumaival (14. pont)

| Kritérium | Állapot |
|---|---|
| Minden releváns korábbi CFA4-terv összevetése | **Részben**: a `main`-ben lévő tervekkel megtörtént; az öt hiányzó terv külön, draft PR-okban javaslatként készült el, de jóváhagyásig a sorok `?`-ek. Nem jelölöm teljesítettnek. |
| Megvalósítástól független összevetés | igen (D1 a tervekhez, nem a kódhoz) |
| Nincs elfogadott funkció újrafelvéve | igen (D1 A-sorai a „Kimarad”-ban) |
| OpenVDB / AX / Points scope egyértelmű | igen: N1/N2; N3 feltételes; Points nincs scope |
| AAF gap audit kész | igen, döntés: 3. |
| USD diff audit kész | **igen**: a delta üres (fork = upstream v23.05); Ubuntu 26.04-es build nincs próbálva |
| VFX-integráció a meglévő rendszerekbe | igen (D5) |
| Dependency- és licenc-audit dokumentált | **igen a licencekre** (forrásfájlból); a 26.04-es csomagverziók és transzitív licencek **ELLENŐRIZETLENEK** (G1) |
| Ubuntu 26.04 amd64 kompatibilitás | **nem igazolt** (U6); tervszinten csak rendszercsomag + külön folyamat |
| CPU-only tervezett; GPU opcionális | igen |
| Erőforrás-adaptáció dokumentált | igen (D9, N4) |
| Regressziókapuk meghatározottak | igen (§6) |
| Minden scope-nak van elfogadási feltétele | igen (N1–N4) |
| Repó-módosítás nem történt | **a kód nem módosult**; két új dokumentum került a `docs/plans/` alá (D1–D10 terv + audit); az öt új terv külön PR-okban, jóváhagyásig draftként készül (coordination.md §2.2) |
| Implementáció külön engedélyhez kötött | igen |
