# CFA4-OPENUSD-3D-001 – OpenUSD és 3D-jelenetcsere, 0.1

> **[MÓD-6] MÓDOSÍTÁS JÓVÁHAGYÁSRA VÁR** (egyeztetés: [`CFA4-DNEG-VFX-EGYEZTETES-001`](CFA4-DNEG-VFX-EGYEZTETES-001-0.1.md) K-3, K-4, K-5, D-4). A jóváhagyásig a 0.1 szöveg érvényes; utána:
> - **Két út az USD-hez.** **A) Külső EXR-csere (alap):** a `CFA4-VFX-001` VFX-8/X-5 szerint a külső backend (Blender, majd Gaffer) maga olvassa az USD/glTF fájlt és EXR AOV-t ad; a CFA4-nek nincs USD-értelmezője. **B) Natív import (U2):** csak akkor indul, ha a tulajdonos VISION §8 / X-4 döntése natív 3D-szerkesztést kér. Az U1 (glTF/OBJ/STL) marad.
> - A 3D → kép útvonal (§3 4. pont) a render-terv C0 job-szerződésén és a VFX-001 X-5 EXR/USD-szerződésén megy, nem külön worker-protokollon.
> - **Tények (2026-11-22, hálózaton):** az upstream OpenUSD legutóbbi kiadása **v26.11.1** (2026-10-21), a `dev` HEAD 2026-11-18; **az Ubuntu 26.04-ben nincs `openusd`/`usd-core` forráscsomag** (Launchpad). A G-USD-1 kapu ezért „külön audit / forrásbuild”-kérdés; a letöltő `build_usd.py` tilos, a forrásbuild külön tulajdonosi döntés.

> **Állapot:** JÓVÁHAGYVA (István, 2026-11-21 22:45, Europe/Budapest). **Kódolási engedélyt nem ad:** minden fázis külön tulajdonosi engedélyt igényel (coordination.md §2.10). **Dátum:** 2026-11-21 · **Kérte:** István (ubuntuokos).
> **Alap:** CFA4 `main` @ `97d7982f`; ez a dokumentum csak dokumentáció.
> **Előfeltétel kódhoz:** VISION §8: a 3D/VFX mélysége tulajdonosi döntés; e nélkül csak az U0–U1 (döntés, specifikáció) indulhat.

## 1. Cél

3D-jelenetek (háló, kamera, hierarchia, animáció, anyag-hivatkozás) **beolvasása és kiírása** a CFA4 projektjéhez, a CFA4 saját projektmodelljének megtartásával. A USD **csereformátum**, nem a CFA4 projektformátuma és nem új backend.

## 2. Kiinduló állapot és igazolt tények

| Tény | Forrás/ellenőrzés |
|---|---|
| A `main`-ben 3D-jelenet, USD, glTF, OBJ, STL: se terv, se kód | DNEG-audit §0.1; `grep` 0 találat |
| `dneg/USD` fork = upstream `v23.05` (b53573ea…), delta üres, 2023-ban megállt → **a fork nem kell** | DNEG-audit D7 (git rev-parse + diff) |
| Upstream OpenUSD aktív (HEAD 2026-11-18); licenc: *Tomorrow Open Source Technology License 1.0*, az Apache-2.0-tól csak a 6. szakasz (védjegyek) tér el | `LICENSE.txt` olvasva, 2026-11-21 |
| Az upstream build-szkriptje (`build_usd.py`) letöltő jellegű → a CFA4-ben tilos automatikusan futtatni | CFA4-szabály: nincs automatikus letöltés |
| 3D-effekt/render a `main`-ben nincs; render-integrációs terv más projekt tárgya | `CFA4-RENDER-INTEGRATION-PLAN-001` |
| ADR-0003: az MaterialX/OSL/OpenVDB az A7 „3D/VFX-szakasz”, külön tervezendő; az OpenUSD nincs a listán | ADR-0003 §2 |

## 3. Hatókör

1. **Formátum-döntés (U0):** melyik formátum mire (javaslat a megvalósítási sorrendre): **glTF 2.0 / OBJ / STL** (egyszerű, kis függőség, tisztán Rust lehet) → **USD** (komplex jelenet, külön folyamatú worker, upstream OpenUSD). A konkrét Rust-crate-ek (glTF, OBJ, STL olvasók) **verziója és licence még ellenőrizendő** (nem néztem meg).
2. **Import (U1–U2):** hierarchia, transzform, kamera, animáció, háló, anyag-hivatkozás; az **unsupported elemekről jelentés** (nincs csendes veszteség, mint az AAF-nál).
3. **Export (U3, feltételes):** csak ha az import bevált és kell.
4. **Render/megjelenítés:** NEM e terv. A 3D → kép útvonal (előnézet, render pass/AOV EXR) külön döntés: a DNEG-terv N2 mintája (worker → EXR/AOV → sima médiaként a kompozitba) követhető, de a render-motorhoz **nem nyúlunk**.
5. **ARF-kapcsolat:** a betöltött prim-ek [ARF](CFA4-ARF-001-0.1.md) reprezentációként (Mesh, Curves, Instances) írandók le.

**Nincs benne:** DNEG USD-fork; az upstream lecserélése forkra; duplikált USD-backend; saját 3D-szerkesztő; render-motor.

## 4. Futtatási modell

A USD C++ marad (AGENTS „keep original language”): **külön felügyelt worker-folyamat** (ACS/ADR-0004), Rust-interfész mögött, Landlock-profillal (`cfa4-sandbox`), CPU-only működéssel. Nincs `unsafe` a CFA4-folyamatban. A glTF/OBJ/STL olvasók Rust-ban in-process is lehetnek, ha a crate-ek licence és karbantartása megfelel.

## 5. Fázisok és kapuk

| Fázis | Tartalom | Kapu |
|---|---|---|
| U0 | Formátum- és modell-döntés; VISION §8 | tulajdonosi döntés |
| U1 | glTF/OBJ/STL olvasó, hierarchia+kamera+háló | U1-A1…A5 |
| U2 | USD-import worker (upstream OpenUSD, Ubuntu-csomag vagy külön audit) | G-USD-1: 26.04-es csomag/build igazolva; U2-A1…A5 |
| U3 | Export (feltételes) | külön engedély |

## 6. Elfogadási feltételek

- **U1-A1:** minden támogatott fájl a hierarchiát, transzformot, kamerát, hálót megőrizve beolvasódik; a nem támogatott elem a jelentésben szerepel.
- **U1-A2:** hibás/csonka/hostile fájl → `Err`, nincs összeomlás (mutation-fuzz; AGENTS §0.8).
- **U1-A3:** nagy háló: memóriakeret betartva (ACS), régió/LOD az ARF-szabállyal.
- **U1-A4:** a projekt csak hivatkozik a forrásfájlra (út + SHA-256); relink hiányzó fájlnál jelentéssel.
- **U1-A5:** projektmentés/-betöltés round-trip; a CFA4 projektmodell változatlan.
- **U2-A1:** USD-fájl (több réteg, referenciák, variánsok) beolvasása; a feloldhatatlan referenciák jelentve.
- **U2-A2:** a worker összeomlása hiba, nem a CFA4 összeomlása; hálózat nélkül fut; időkeret és memóriakeret.
- **U2-A3:** CPU-only; nincs CUDA-függés.
- **U2-A4:** Qt6: importpanel, unsupported-lista; automation-id.
- **U2-A5:** regressziókapu (idővonal, render, export, projekt, Qt6).

## 7. Licenc és függőség

Az upstream OpenUSD licence (TOST-1.0, Apache-2.0-szerű) a CFA4 szabálya szerint elfogadhatónak *látszik*, de a védjegy-szakasz eltérését a tulajdonos hagyja jóvá. A függőségei (Boost/oneTBB/OpenSubdiv/MaterialX… verziófüggő) **még nincsenek auditálva** (G-USD-1). Letöltő build-szkript nem futtatható.

## 8. Nyitott döntések

1. **VISION §8:** a 3D mélysége.
2. glTF-first vagy USD-first sorrend (javaslat: glTF/OBJ/STL először).
3. A TOST-1.0 védjegy-eltérés elfogadása.
4. Kell-e USD-export, vagy elég az import.
