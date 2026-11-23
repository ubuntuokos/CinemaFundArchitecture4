# CFA4-OPENUSD-3D-EXPORT-001 – 3D-export (U3): glTF/OBJ/STL és USD kiírás, 1.0

> **Állapot:** JÓVÁHAGYVA (István, 2026-11-22 18:07): az E-1…E-5 javaslatok elfogadva (§7), az elfogadási feltételek érvényesek. **Dokumentum-szintű terv; kódot nem indít.** Az U3 fázisengedélyt István 2026-11-22 18:03-kor megadta; az elfogadási feltételeket ez a dokumentum írja meg ([`CFA4-OPENUSD-3D-001-1.0.md`](CFA4-OPENUSD-3D-001-1.0.md) §7). A fázis kapui érvényesek: az U3a az U1 után, a terv `main`-be kerülése után indulhat; az U3b az U2-től és a G-USD-1 (a)/(b) kiválasztásától függ (`docs/coordination.md` §2.10).
> **Dátum:** 2026-11-22 · **Kérte:** István (ubuntuokos) · **Döntés, amelyre épül:** import és export egyaránt cél (István, 2026-11-22 16:55).
> **Alap:** CFA4 `main` @ `2ff5b412`; a `main`-ben 3D-kód nincs.

## 1. Cél és határ

A projektbe importált 3D-asset (`scene3d`, lásd a fő terv 4. fejezete) **kiírása fájlba**, a CFA4 saját projektmodelljének megtartásával. Az export is projektfüggő: csak olyan projektben érhető el, amely 3D-assetet tartalmaz; 3D-tartalom nélküli projekt nem lát exportfelületet.

**Értelmezés (jóváhagyandó, E-1):** az export a hivatkozott 3D-asset **hű kiírása**, beleértve a CFA4-ben végzett módosításokat is, **ha és amennyire** a CFA4 a jelenetet szerkeszti (a szerkesztés mélysége a fő terv §11 2. pontja szerint az U1 után dől el). Szerkesztés nélkül az export formátumok közötti átírás (például glTF → STL) veszteség-jelentéssel.

**Nincs benne:** az idővonal vagy a videó-projekt kiírása 3D-jelenetként (például klipek textúrás síkként); render-kép kiírása (az a render-terv/VFX-8); saját 3D-szerkesztő; a külső backend (VFX-8) exportja.

## 2. Függőségek és sorrend

| Rész | Tartalom | Előfeltétel |
|---|---|---|
| **U3a** | glTF 2.0, OBJ, STL export (tiszta Rust, folyamaton belül) | U1 kész (olvasó, `scene3d`, unsupported-jelentés); a terv a `main`-ben |
| **U3b** | USD export | U2 kész **és** G-USD-1 nem „(c)”: a fő terv §11 5. pontja szerint most nincs natív USD, így a CFA4-ben nincs USD-író sem. Az USD-írást csak az U2 USD-worker tudná végezni, amely nincs engedélyezve a beszerzési út (a) vagy (b) kiválasztásáig |

Következmény, amelyet a tulajdonosnak látnia kell: **az „export” döntése az USD-re addig nem teljesíthető, amíg az U2 beszerzési útja nem dől el.** Az U3a ettől független.

## 3. Formátum-mátrix

| Formátum | Író | Licenc / forrás (2026-11-22, crates.io) | Megjegyzés |
|---|---|---|---|
| glTF 2.0 (`.glb`, `.gltf`+`.bin`) | `gltf-json` 1.4.1 szerkezetei és szerializálása (`MIT OR Apache-2.0`, utolsó kiadás 2024-05-10) | a kód nincs olvasva; a `gltf` olvasó crate-hez tartozik (karbantartási kockázat, fő terv §6) | a `gltf` olvasó fő célja az olvasás; az írást a `gltf-json` típusaiból kell összeállítani |
| STL (bináris, opció: ASCII) | `stl_io` 0.11.0 (`MIT`, 2026-03-15; „STL input and output”) | a kód nincs olvasva | csak háló: nincs anyag, hierarchia, kamera |
| OBJ (+MTL) | saját író (egyszerű szöveges formátum) | nincs külső függőség; az olvasók: `tobj` 4.0.5 (`MIT`), `obj-rs` 0.7.4 | nincs hierarchia, kamera, animáció |
| USD | USD-worker (U2) | OpenUSD TOST-1.0 (fő terv §9) | blokkolt, lásd 2. fejezet |

A felsorolt licencek (MIT / Apache-2.0) GPL-3 kompatibilisek; `cargo deny check licenses` nem futott, a kódot nem olvastam.

## 4. Szabályok

1. **Nincs csendes veszteség.** Kiírás előtt a CFA4 veszteség-jelentést ad: mi nem ábrázolható a célformátumban (például OBJ-ban nincs hierarchia, kamera és animáció; STL-ben csak a háló). A felhasználó a jelentés ismeretében hagyja jóvá a kiírást. Ugyanaz az elv, mint az import unsupported-jelentése.
2. **Biztonságos írás.** Csak a felhasználó által kijelölt kimeneti mappába ír; a forrásfájlt nem írja felül; meglévő célfájlnál rákérdez; atomi írás (ideiglenes fájl, majd átnevezés), hiba esetén nem marad félkész fájl.
3. **Eredet és szerzői jog megmarad.** A forrás szerzői jogi és licenc-mezői (például a glTF `asset.copyright`) átkerülnek; a `generator` mezőbe a CFA4 neve és verziója kerül. Nincs telemetria és nincs hálózat.
4. **Determinisztikus kimenet.** Azonos bemenet és beállítás bájtazonos fájlt ad; nincs időbélyeg a fájlban.
5. **Egység és tengely.** A glTF Y-fel felfelé, méter egységű; az OBJ/STL egységet nem hordoz: a konverzió és az egység a jelentésben szerepel.
6. **Projektfüggő felület.** Exportparancs és -panel csak 3D-assetet tartalmazó projektben jelenik meg. Parancs-azonosító (javaslat): `scene3d.export`; minden elem automation-id-t kap (`CLAUDE.md`: minden parancs).
7. **Erőforrás.** Nagy hálónál a memóriakeret az ACS szerint; a kiírás megszakítható és a háttérszálon fut (a Qt6 főszálon nincs hosszú művelet).
8. **Glb alapértelmezés (javaslat, E-3):** `.glb` az alap, `.gltf`+külső fájlok opció; a textúra az import hivatkozása szerint (nem beágyazott), `.glb`-nél kérésre beágyazható (E-5).

## 5. Fázisok

| Fázis | Tartalom | Kapu |
|---|---|---|
| **U3-0** | Ez a dokumentum | tulajdonosi jóváhagyás (az elfogadási feltételeké és az E-1…E-5 döntéseké) |
| **U3a** | glTF/OBJ/STL export | az U1 kész; U3a-A1…A9 |
| **U3b** | USD export | az U2 kész, G-USD-1 ≠ (c); U3b-A1…A3 |

Minden fázis külön PR-ban, egy témával; a fázis lépéseinek, sorrendjének vagy feltételeinek módosítása újra jóváhagyást kér (`docs/coordination.md` §2.10).

## 6. Elfogadási feltételek

**U3a (glTF/OBJ/STL):**
- **U3a-A1 (round-trip):** import → export → import után a hierarchia, a transzformok, a kamera és a háló geometriája (csúcs- és indexadat) azonos; az f32 értékek bájtra azonosak (az OBJ szöveges kiírása oda-vissza egyező, legrövidebb alakban).
- **U3a-A2 (jelentés teljessége):** a forrás-fixture minden olyan eleme, amely a célformátumban nem ábrázolható, szerepel a veszteség-jelentésben; a jelentés nélkül nincs kiírás.
- **U3a-A3 (hibaágak):** teli lemez, jogosultsághiány, megszakítás, hibás belső állapot → hiba, nem összeomlás; nem marad félkész fájl.
- **U3a-A4 (biztonság):** a forrás nem íródik felül; a kimeneti mappából nem lehet kilépni (útvonal-bejárási teszt); meglévő fájlnál rákérdez.
- **U3a-A5 (determinizmus):** két egymás utáni export bájtazonos.
- **U3a-A6 (külső ellenőrzés):** a kimenet átmegy egy, a CFA4-től független ellenőrzőn: glTF-nél egy külső validátor mint tesztorákulum (nem függőség; a választás és a licenc az U3a első PR-jában ellenőrzendő, most nincs ellenőrizve); STL/OBJ-nál független olvasó.
- **U3a-A7 (projektfüggés):** 3D-tartalom nélküli projektben nincs exportfelület; régi projekt változatlanul nyílik (a fő terv P-A1, P-A4 mintájára).
- **U3a-A8 (Qt6):** exportpárbeszéd (formátum, kimeneti mappa, `.glb`/`.gltf`), veszteség-lista, automation-id; Wayland és X11.
- **U3a-A9 (erőforrás):** nagy hálónál a memóriakeret betartva, a főszál szünete nem nő (mérés a referenciagépen).

**U3b (USD):**
- **U3b-A1:** a kiírt USD az U2 importerrel visszaolvasható, a hierarchia, transzform, kamera és háló azonos; a nem ábrázolható elem jelentve.
- **U3b-A2:** a worker összeomlása hiba, nem a CFA4 összeomlása; hálózat nélkül fut; idő- és memóriakeret; a render C0 szerződésén megy.
- **U3b-A3:** csak az az USD-tartalom íródik, amelyet az importer ismer (réteg, variáns, referencia: a támogatott részhalmaz a jelentésben).

## 7. Döntések (István, 2026-11-22 18:07: mind a javaslat szerint elfogadva)

| # | Kérdés | Döntés |
|---|---|---|
| E-1 | Mit jelent az „export” (1. fejezet értelmezése)? | hű kiírás a CFA4-beli módosításokkal; az idővonal 3D-jelenetként nincs benne |
| E-2 | Az USD-export a G-USD-1 (a)/(b) kiválasztásáig blokkolt | az U3b várja az U2-t; az U3a ettől független indulhat |
| E-3 | glTF alakja | `.glb` alap, `.gltf` opció |
| E-4 | Az OBJ és az STL export az első körben kell-e | igen: glTF, bináris STL és OBJ az U3a-ban |
| E-5 | Textúra beágyazása | hivatkozás az alap; `.glb`-ben kérésre beágyazható |

## 8. Mit nem állít ez a terv

A Rust-crate-ek kódját nem olvastam, `cargo deny` nem futott; a külső glTF-validátor (A6) nincs kiválasztva és a licence nincs ellenőrizve. Az `scene3d` és a `scene3d.export` név javaslat, nem létező kód. A 3D-szerkesztés mélysége nyitott, ezért az „a CFA4-ben végzett módosítások” kiírása addig csak a hivatkozott állapotot fedi.

## 9. Változásnapló

| Verzió | Dátum | Változás |
|---|---|---|
| 1.0 | 2026-11-22 | Jóváhagyva (István, 18:07): E-1…E-5 a javaslat szerint; fájl átnevezve (`…-0.1.md` → `…-1.0.md`). |
| 0.1 | 2026-11-22 | Első javaslat az U3 exportra: formátumok, szabályok, fázisok, elfogadási feltételek, döntések. |
