# CFA4-OPENUSD-3D-001 – 3D-jelenetcsere (glTF/OBJ/STL, OpenUSD), projektfüggő 3D, 1.0

> **Állapot:** JÓVÁHAGYVA (István, 2026-11-22 18:01, Europe/Budapest); a §11 döntéseit István 16:55-kor zárta le. **Fázisengedélyek:** István 2026-11-22 18:03-kor megadta az U1, U2 és U3 fázisengedélyeket (`docs/coordination.md` §2.10). Ez **nem** kerüli meg a fázisok saját kapuit (§7): az U1 a tervnek a `main`-be kerülése után, friss `main`-ből indulhat; az U2-höz kell a render C0 és a G-USD-1 (most: nincs natív USD, §11 5. pont); az U3 elfogadási feltételeit az indítása előtt meg kell írni. Terv-változtatás (a fázisok lépéseinek, sorrendjének vagy feltételeinek módosítása) újra jóváhagyást kér. A 0.1 változat helyébe lép.
> **Verzió:** 1.0 · **Dátum:** 2026-11-22 · **Kérte:** István (ubuntuokos). A 0.1 helyébe lép (a fájl átnevezve: `…-0.1.md` → `…-1.0.md`).
> **Beépítve:** a `[MÓD-6]` (két USD-út) és `[MÓD-8]` (3D projektfüggő) javaslatok a [`CFA4-DNEG-VFX-EGYEZTETES-001`](CFA4-DNEG-VFX-EGYEZTETES-001-0.1.md)-ből (K-3, K-4, K-5, D-4, D-12) és a `CFA4-VFX-001` v1.1 (§3.1, §8a; VFX-8, X-4, X-5).
> **Alap:** CFA4 `main` @ `2ff5b412`; ez a dokumentum csak dokumentáció, a `VISION.md`-t nem módosítja.

## 1. Cél

3D-jelenetek (hierarchia, transzform, kamera, háló, animáció, anyag-hivatkozás) **bekötése a CFA4 videó-projektjébe**, a CFA4 saját projektmodelljének megtartásával. Az USD **csereformátum**, nem a CFA4 projektformátuma, nem új backend, nem új motor.

## 2. Döntések összefoglalva

| # | Döntés | Forrás |
|---|---|---|
| **Dönt-1** | **A 3D videó-projekt-függő**, nem globális szint: attól függ, mit importált a felhasználó az adott projektbe. A 3D-állapot a projektben tárolódik; 3D-tartalom nélküli projektet nem érint. | István, 2026-11-22 15:36 (VISION §8, K-5, D-12) |
| **Dönt-2** | **Két út** minden 3D-tartalomhoz, **projektenként/tartalmanként választható, alapértelmezés A:** **A)** külső EXR-csere (a külső backend maga olvassa a fájlt, EXR/AOV-t ad; a CFA4-nek nincs 3D-értelmezője); **B)** natív import (U1/U2). | István 15:36; VFX-001 VFX-8/X-5; K-4 |
| **Dönt-3** | **Egy worker/job-szerződés:** a render-terv C0 szerződése. Az USD-worker és a 3D-render ennek kliense, nincs saját protokoll. | K-3, D-3 |
| **Dönt-4** | **Nincs DNEG USD-fork**; USD csak upstream OpenUSD lehet. | DNEG-audit D7 |
| **Dönt-6** | **Import és export egyaránt cél** (István, 2026-11-22 16:55): az U3 export nem marad ki, de külön fázisengedély kell hozzá. | §11 3. pont |
| **Javaslat-5** | **Megvalósítási sorrend:** glTF/OBJ/STL (U1, tiszta Rust) → USD (U2, külön folyamat). | 0.1 §3, változatlan |

## 3. Igazolt tények (dátumozva)

| Tény | Forrás / ellenőrzés |
|---|---|
| A `main`-ben 3D-jelenet, USD, glTF, OBJ, STL: se terv, se kód (a `git grep` csak egy ikonfájlt talál) | `git grep -il "gltf\|openusd\|\.usd" origin/main` (nem `docs/plans`), 2026-11-22 |
| `dneg/USD` HEAD = upstream `v23.05` (`b53573ea…`), delta üres, 2023 óta áll → a fork nem kell | DNEG-audit D7; egyeztetés §5 |
| Upstream OpenUSD: legutóbbi kiadás **v26.11.1** (2026-10-21; commit `96c1989167d2…`), `dev` HEAD 2026-11-18 | egyeztetés §5 (`git ls-remote`) |
| Licenc: *Tomorrow Open Source Technology License 1.0* (TOST-1.0): az Apache-2.0-tól csak a 6. szakasz (védjegyek) tér el | `LICENSE.txt` olvasva; a PyPI is `LicenseRef-TOST-1.0`-t jelöl |
| **Az Ubuntu 26.04-ben (`resolute`) nincs `openusd`/`usd-core` forráscsomag** | Launchpad API, 2026-11-22 |
| **Új:** a PyPI-n van előre épített `usd-core` 26.11 kerék (`manylinux_2_28_x86_64`, cp311–cp314, 2026-10-21). Binárisan töltendő le és a belőle bundle-ölt könyvtárak licence **nincs auditálva** | PyPI JSON, 2026-11-22; nem töltöttem le, nem telepítettem |
| Az upstream `build_usd.py` letöltő jellegű → a CFA4-ben nem futtatható (nincs automatikus letöltés) | CFA4-szabály |
| Rust olvasók (crates.io, 2026-11-22): `gltf` 1.4.1 (`MIT OR Apache-2.0`, utolsó kiadás 2024-05-10), `tobj` 4.0.5 (`MIT`, 2026-08-02), `stl_io` 0.11.0 (`MIT`, 2026-03-15), `obj-rs` 0.7.4 (`Apache-2.0 OR MIT`, 2024-09-28) | crates.io API; a kód nincs olvasva, `cargo deny` nincs futtatva |
| A `crates/sandbox` (`cfa4-sandbox`) a `main`-ben megvan (Landlock-profilok, CFA4-SECURITY-001 B1) | `ls crates`, `docs/plans/CFA4-SECURITY-001.md` |
| 3D-render a `main`-ben nincs; a render-terv C0 (worker protokoll, job API) még nem kész | `CFA4-RENDER-INTEGRATION-PLAN-001` |

## 4. A projektfüggő 3D-modell (U0 kidolgozva)

**Elv:** nincs globális „3D-mód”. A 3D-képesség akkor és csak ott kapcsol be, ahol a projekt 3D-tartalmat hivatkozik.

1. **Tartalomtípusok és a bekapcsolás:**

| Tartalom (import) | Bekapcsolt képesség | Alapút | B-út (natív) |
|---|---|---|---|
| glTF 2.0, OBJ, STL | 3D-asset hivatkozás a projektben, előnézet | A (külső EXR) | **U1** (Rust olvasó) |
| USD (`.usd/.usda/.usdc/.usdz`) | 3D-asset hivatkozás, előnézet | A (külső EXR) | **U2** (USD-worker), feltételes |
| Pontfelhő, `.vdb` | nem e terv tárgya | – | [`POINTCLOUD-001`](CFA4-POINTCLOUD-001-0.1.md), DNEG N1/N2 (ugyanez az elv) |

2. **Projektben tárolt állapot:** a projekt additív, verziózott `scene3d` szakaszt kap, csak ha van 3D-asset: hivatkozás (**út + SHA-256**, nem beágyazás), importer-verzió, a kiválasztott út (`A`/`B`), az **unsupported-jelentés**, a tartalomtípusonként bekapcsolt képességek. Hiányzik a szakasz, ha nincs 3D-tartalom.
3. **Régi projekt változatlan:** 3D-tartalom nélküli projekt ugyanúgy nyílik, renderel és exportál (render-terv AC-15); a séma additív, az ismeretlen szakaszt a régi verzió megőrzi/figyelmen kívül hagyja a projekt szabálya szerint.
4. **Út projektenként/tartalmanként:** egy 3D-asset útját a felhasználó a projektben váltja (A↔B); alapértelmezés A. Az út váltása nem veszíthet adatot: a hivatkozás és a jelentés megmarad.
5. **Felület:** nincs globális 3D-beállítás. A 3D-panel és az importpanel akkor jelenik meg, ha a projekt tartalmaz 3D-assetet, vagy a felhasználó éppen importál ilyet. Automation-id minden elemen.
6. **Mélység tartalomtípusonként** fokozódik; a felső határ (egyszerű 3D-cím vs. teljes jelenetszerkesztés) **még nem dőlt el**, az U1 tapasztalata után kérendő (§11 2. pont). Saját 3D-szerkesztő és render-motor e tervben nincs.
7. **Az A út mit kíván e tervtől:** csak a közös projekt-hivatkozást (2. pont) és a jelentést; a végrehajtás (backend-indítás, EXR/AOV, hibaágak) a `CFA4-VFX-001` VFX-8-é (X-5 szerződés, Blender, majd Gaffer, a render C0 szerint). Ez a terv **nem** ír backend-szerződést.

## 5. Hatókör

**Benne:** (1) a 4. fejezet modellje; (2) **U1:** glTF/OBJ/STL olvasó (hierarchia, transzform, kamera, háló, kulcskocka-animáció, anyag-hivatkozás); (3) **U2:** USD-import worker (B-út, feltételes); (4) **unsupported-jelentés** (nincs csendes veszteség, mint az AAF-nál); (5) **U3 export** (cél, külön fázisengedéllyel; az export-kapuk U3-A1… az U3 indítása előtt íródnak).

**Nincs benne:** DNEG USD-fork; az upstream OpenUSD lecserélése forkra; második worker-protokoll; saját 3D-szerkesztő; render-motor; a 3D→kép render (VFX-8/C0 tárgya; előnézet-minta a DNEG N2: worker → EXR/AOV vagy PNG → sima média); pontfelhő és VDB (saját tervük); második részecskemotor (VFX-6).

**ARF-kapcsolat:** a betöltött prim-ek az [ARF](CFA4-ARF-001-0.1.md) leíróival írandók le (Mesh, Curves, Instances); az ARF-leírók a projektbe importált forrásból keletkeznek (ARF `[MÓD-8]`).

## 6. Futtatási modell (B-út)

- **U1:** a glTF/OBJ/STL olvasók Rust-ban, folyamaton belül futhatnak, **ha** a crate licence (GPL-3 kompatibilis, `cargo deny check licenses`), karbantartása és a fuzz-kapu (U1-A2) megfelel. A `gltf` utolsó kiadása 2024-05 (karbantartási kockázat; §11 4. pont: in-process fuzz-kapuval); ha egy crate a kaput nem teljesíti, az olvasó külön folyamatba (az USD-workerrel azonos modellbe) kerül. Nincs `unsafe` a CFA4-folyamatban.
- **U2:** az USD C++ marad („keep original language”, `AGENTS.md`), **külön felügyelt worker-folyamat**, Rust-interfész mögött, a render-terv **C0 szerződésén** és az ACS/ADR-0004 útján; `cfa4-sandbox` Landlock-profil (nincs hálózat, csak a megnyitott fájl és a kimeneti könyvtár), CPU-only. Összeomlása hiba, nem a CFA4 összeomlása. A port-vagy-marad indoklást (nem lassítja a lejátszást/renderelést, referenciagépen mérve) az átvételi PR rögzíti.

## 7. Fázisok és kapuk

| Fázis | Tartalom | Kapu / előfeltétel |
|---|---|---|
| **U0** | Ez a terv: formátum-, modell- és útdöntés | tulajdonosi jóváhagyás (ez a PR). A `VISION.md` §8 formális átvezetése a tulajdonos/gazda dolga |
| **U1** | glTF/OBJ/STL olvasó + `scene3d` projektszakasz + unsupported-jelentés | külön fázisengedély; a projektséma-bővítés egyeztetése a VFX-2 (`VfxGraph`) és az ARF sémával (két független additív szakasz, közös verziózási szabály); U1-A1…A5 |
| **U2** | USD-import worker (B-út) | külön fázisengedély; **G-USD-1** (lent); render C0 kész; U2-A1…A5 |
| **U3** | Export (glTF/USD kiírás, import mellett cél: István, 2026-11-22 16:55) | külön fázisengedély; az U1 (glTF/OBJ/STL export) illetve U2 (USD export) után; elfogadási feltételek az U3 indítása előtt, a tulajdonos jóváhagyásával kiegészítendők (§8) |

**G-USD-1 (átfogalmazva, 26.04-es csomag nincs):** az U2 csak akkor indulhat, ha a tulajdonos kiválaszt egy beszerzési utat, és az auditált: **(a)** a PyPI `usd-core` kerék rögzítve (verzió + SHA-256, `--require-hashes`, a repó Python-szabálya szerint), a bundle-ölt könyvtárak licence- és eredetaudittal; **(b)** forrásbuild a rögzített `v26.11.1` címkéből (commit-SHA), a letöltő `build_usd.py` nélkül, a függőségek (Boost/oneTBB/OpenSubdiv/MaterialX…, verziófüggő) külön auditjával és a csomagolási hatás (deb/AppImage/Flatpak) felmérésével; **(c)** nincs natív USD (csak A út). A javaslat: **(c) most, (a) mérlegelendő az U2 indításakor**, mert az A út USD-értelmező nélkül is ad értéket.

## 8. Elfogadási feltételek

**Projektmodell (új):**
- **P-A1:** 3D-tartalom nélküli projekt bájtazonos mentést/újranyitást ad a változás előtti viselkedéssel; nincs `scene3d` szakasz, nincs 3D-felület.
- **P-A2:** a 3D-képesség bekapcsolása kizárólag a projektbe importált tartalomtól függ; nincs globális kapcsoló.
- **P-A3:** az A/B út asset-enként választható és váltható adatvesztés nélkül; alapértelmezés A.
- **P-A4:** régi projekt (séma a változás előtt) változatlanul nyílik és renderel (AC-15).

**U1:**
- **U1-A1:** minden támogatott fájl a hierarchiát, transzformot, kamerát, hálót megőrizve beolvasódik; a nem támogatott elem a jelentésben szerepel.
- **U1-A2:** hibás/csonka/hostile fájl → `Err`, nincs összeomlás (mutációs fuzz; `AGENTS.md` §0.8).
- **U1-A3:** nagy háló: memóriakeret betartva (ACS), régió/LOD az ARF-szabállyal.
- **U1-A4:** a projekt csak hivatkozik a forrásfájlra (út + SHA-256); relink hiányzó fájlnál jelentéssel.
- **U1-A5:** projektmentés/-betöltés round-trip; a CFA4 projektmodell a `scene3d` szakaszon kívül változatlan.

**U2:**
- **U2-A1:** USD-fájl (több réteg, referenciák, variánsok) beolvasása; a feloldhatatlan referenciák jelentve.
- **U2-A2:** a worker összeomlása hiba, nem a CFA4 összeomlása; hálózat nélkül fut; idő- és memóriakeret; a C0 szerződésen megy (nincs saját protokoll).
- **U2-A3:** CPU-only; nincs CUDA-függés.
- **U2-A4:** Qt6: importpanel, unsupported-lista, út-választó; automation-id; Wayland és X11.
- **U2-A5:** regressziókapu (idővonal, render, export, projekt, Qt6).

## 9. Licenc és függőség

- **OpenUSD (TOST-1.0):** Apache-2.0-szerű; a CFA4 szabálya szerint elfogadhatónak *látszik*, de a védjegy-szakasz (6.) eltérését és a GPL-3.0-or-later kompatibilitást **a tulajdonos hagyja jóvá** (§11 1. pont: elfogadva az U2 indításáig). Dinamikus/folyamat-szintű használat, kódátvétel nincs.
- **Függőségek:** OpenUSD függőségei és a kerék bundle-jei **nincsenek auditálva** (G-USD-1). Letöltő build-szkript nem futtatható, modell/bináris nem töltődik le a felhasználó tudta nélkül.
- **Rust olvasók:** a 3. fejezet táblája szerint a felsorolt licencek (MIT / Apache-2.0) GPL-3 kompatibilisek; a `cargo deny check licenses` és a kódátvizsgálás az U1 első PR-ja.
- A donor-pillanatkép (`v26.11.1`, commit `96c1989167d2…`) csak referencia; a VFX-001 donorai külön, befagyasztva.

## 10. Viszony más tervekhez (nincs duplikáció)

| Terv | Viszony |
|---|---|
| `CFA4-VFX-001` VFX-8 / X-4 / X-5 | A (külső EXR-csere) gazdája; ez a terv adja a projekt-hivatkozást és a B utat. Az X-4 (3D-mélység) a Dönt-1 szerint projektfüggő; a mélység felső határa nyitott |
| Render-terv C0 / F3 / F5 | Egyetlen worker/job-szerződés; az EXR/AOV-t az F5 adja |
| [`ARF-001`](CFA4-ARF-001-0.1.md) | A betöltött prim-ek leírója; nincs második leírórendszer |
| [`POINTCLOUD-001`](CFA4-POINTCLOUD-001-0.1.md), DNEG N1/N2 | Ugyanaz a projektfüggő elv; nem 3D-jelenet, nem várja meg ezt a tervet |
| `CFA4-FX-AUTHORING-001` | Nincs átfedés; a preset a `VfxGraph` részgráfja |
| `G-xx` mátrix | Egy sor: `DNEG-M5` (ideiglenes azonosító; a végleges számot a mátrix gazdája adja, a CSV-hez e PR nem nyúl). Állapot-javaslat: `GAP_STATIC` → `GAP` (U1 indítható dokumentum-döntés után) |
| `CFA4-RENDER-INTEGRATION-PLAN-001` | A render-motorhoz nem nyúlunk |

## 11. Döntések (István, 2026-11-22 16:55)

| # | Döntés | Eredmény |
|---|---|---|
| 1 | TOST-1.0 elfogadása | **Igen, az U2 indításáig kell** (javaslat szerint). A GPL-3.0-or-later kompatibilitás ellenőrzése az U2 kapuja. |
| 2 | A mélység felső határa tartalomtípusonként | **Az U1 után döntendő** (javaslat szerint). |
| 3 | Import és/vagy export | **Import és export egyaránt cél** (a javaslattól eltérően): az U3 export bekerül a tervbe, külön fázisengedéllyel. |
| 4 | U1 Rust-crate-ek | **In-process, fuzz-kapuval; kudarc esetén worker** (javaslat szerint). |
| 5 | G-USD-1 | **„Most”: most nincs natív USD** (c út; az A út adja az USD-támogatást). Értelmezés: „most” = a javaslatom „(c) most”-ja; ha a PyPI-kerék (a) vagy a forrásbuild (b) mérlegelését kérted, jelezd. A (a)/(b) az U2 indításakor újra előkerül. |
| 6 | Projektfüggő értelmezés és a `VISION.md` §8 | **Megerősítve** (javaslat szerint): a 4. fejezet 1–7. pontja érvényes. A `VISION.md` §8 formális átvezetése a tulajdonos/gazda dolga, ez a PR nem módosítja. |

**Nyitott maradt:** a 2. pont (az U1 után), az U2-hez a (a)/(b) beszerzési út, és az U3 elfogadási feltételei.

## 12. Mit nem állít ez a terv

Nem néztem meg a Rust-crate-ek forráskódját, nem futtattam `cargo deny`-t; a `usd-core` kereket nem töltöttem le és nem vizsgáltam; az OpenUSD függőségeit nem auditáltam; a Launchpad „nincs csomag” és a PyPI-adat a lekérdezés napján igaz. A 3D-render (VFX-8, C0) még nem létezik, e terv nem állítja, hogy elkészül. A tervezett útvonalak (`scene3d`) nevei javaslatok, nem létező kód. A `main` hivatkozott állapota: `2ff5b412`.

## 13. Változásnapló

| Verzió | Dátum | Változás |
|---|---|---|
| 0.1 | 2026-11-21 | Első javaslat; jóváhagyva dokumentumként (22:45), kódolási engedély nélkül. |
| 1.0b | 2026-11-22 | Fázisengedélyek rögzítve (István, 18:03): U1, U2, U3; a fázis-kapuk érvényesek. Tartalmi változás nincs. |
| 1.0 (jóváhagyva) | 2026-11-22 | István jóváhagyta (18:01); további tartalmi változás nincs, a 1.0a döntései érvényesek. |
| 1.0a | 2026-11-22 | A §11 döntései rögzítve (István, 16:55): import és export egyaránt cél (U3 bekerül), a többi a javaslat szerint; G-USD-1: most nincs natív USD. |
| 1.0 | 2026-11-22 | Végleges javaslat: `[MÓD-6]` (két út) és `[MÓD-8]` (projektfüggő 3D) beépítve; projektfüggő modell (4. fejezet) és új `P-A1…A4` kapuk; a tények frissítve (OpenUSD v26.11.1, nincs 26.04-es csomag, `usd-core` kerék, Rust-crate-ek licence); G-USD-1 átfogalmazva; egy szerződés (C0); nyitott döntések szűkítve. |
