# CFA4-AX-INTERPRETER-001 – saját, korlátozott kifejezés-interpreter (N3, G4), 0.1

> **Állapot:** JÓVÁHAGYVA (István, 2026-11-22 18:56, Europe/Budapest), a §9 javaslatai (Q-X1…Q-X5) elfogadva. A jóváhagyás **nem implementációs engedély**:
> X0 (nyelvleírás, kód nélkül) elkészült és elfogadva (István, 2026-11-22 18:57); X1-től minden fázishoz külön engedélyt kérek (coordination.md §2.10). A Q-X2 futásidő-határ számát a tulajdonos az X2 mérése előtt adja meg.
> **Dátum:** 2026-11-22 · **Kérte:** István (ubuntuokos) · **Szülő terv:** [`CFA4-DNEG-TECHNOLOGY-ADAPTATION-001-1.0.md`](CFA4-DNEG-TECHNOLOGY-ADAPTATION-001-1.0.md) (§4 N3, G4, Q4)
> **Platform:** Linux amd64, Ubuntu 26.04; nincs automatikus letöltés, nincs Conda; nincs LLVM-függőség.
> **Tulajdonosi döntés (2026-11-21 22:49, Q4):** az AX-szerű térbeli kifejezést **saját, korlátozott interpreter** futtatja; az LLVM-JIT-worker nem scope.
> A szülő terv G4 sora ezt a tervet várja („az interpreter tervét a G4 külön, jóváhagyandó tervben rögzíti”).

## 1. Eredmény röviden

1. **Mi ez:** egy kicsi, típusos, determinisztikus kifejezésnyelv (a továbbiakban **CFX**), amely egy *pontra* vagy egy *voxelre* ad értéket, és erre ír vissza attribútumot. Az ötlet az OpenVDB AX-ból származik, de **nem AX-kompatibilis** (nem cél, hogy AX-szkriptek fussanak).
2. **Tiszta Rust, `unsafe` nélkül**, új crate-ben (`cfa4-expr`, L0, csak `cfa4-time`/std függőséggel). Bájtkódra fordít, és fuel-számlálós virtuális gépen fut. Nincs JIT, nincs gépikód-generálás, nincs LLVM, nincs rendszerhívás-felület: a nyelvnek **nincs** I/O-művelete, ezért a „kiszökés” a nyelv szintjén lehetetlen, nem csak sandboxszal tiltott.
3. **Védelem két rétegben:** (a) a nyelv szerkezete (nincs I/O, nincs rekurzió, korlátos ciklus, korlátos tömb, fuel); (b) a voxeladatot továbbra is a már meglévő, sandboxolt `workers/vdb` worker tartja és írja (N1); az interpreter csak blokkokat kap és ad vissza.
4. **Hatókör szándékosan szűk:** v1 = szám (float/int/bool), vec3, matematikai függvények, determinisztikus zaj, csak olvasás/írás a saját ponton/voxelen és a megnevezett bemeneti attribútumokon. Nincs szomszéd-hozzáférés v1-ben (v2: korlátos szomszédság), nincs felhasználói függvény, nincs string.
5. **Mit nem ígér:** AX-kompatibilitás, GPU-futtatás, valós pontfelhő-forrás (a Points terv nincs scope; a pont-kontextust szintetikus attribútumoszlopokon bizonyítjuk, a valós forrás a [Pontfelhő terv](CFA4-POINTCLOUD-001-0.1.md) dolga).

## 2. Donor- és licenckérdés

- Az AX forrása (Apache-2.0, ASWF OpenVDB 14.0.0) a szülő terv szerint **szóba jöhet donorként**, de itt **nem használjuk**: a CFX szintaxisát és szemantikáját magunk tervezzük a szükségletből, a forrást nem olvassuk. Ez a clean-room szabály (AGENTS §2) legbiztonságosabb olvasata; nincs származékmű, nincs attribúciós kötelezettség.
- Az AX nyelv dokumentációjának licence **ELLENŐRIZETLEN** (nem néztem meg); ezért a dokumentációból sem másolunk szöveget. A függvénynevek általános matematikai nevek (`sin`, `lerp`, `clamp`…), ez nem védhető.
- Új külső függőség nincs (a zajhoz saját, kis hash-alapú gradiens-zaj; a licencelt algoritmus-szabadalmakat a megvalósítás előtt át kell nézni, lásd Q-X3).

## 3. A nyelv (CFX v1)

### 3.1 Típusok és kifejezések
`float` (f32), `int` (i32, ellenőrzött aritmetika: túlcsordulás = futási hiba, nem pánik), `bool`, `vec3` (három f32). Operátorok: `+ - * / %`, összehasonlítások, `&& || !`, `? :`. Konverzió kifejezett (`float(x)`, `int(x)`).

### 3.2 Utasítások
Értékadás, `if/else`, `for i in 0..N` **konstans, fordításkor ismert határral** (N ≤ 1024), blokk. **Nincs** `while`, nincs rekurzió, nincs felhasználói függvény, nincs dinamikus tömb.

### 3.3 Kontextusok és attribútumok
- **Volume-kontextus:** beépített `@density`-szerű attribútumok névvel (`@name:float|vec3`), `P` (a voxel világpozíciója), `ijk` (index), `voxelsize`. Olvasás és írás **csak az aktuális voxelre**.
- **Pont-kontextus:** `P`, `id`, és megnevezett attribútumoszlopok; olvasás/írás az aktuális pontra.
- A kifejezés **fejlécben** deklarálja, mely attribútumokat olvassa és írja (`reads @a, @b; writes @c;`). A fordító ellenőrzi; a deklarálatlan attribútum fordítási hiba. Ebből következik a függőségi gráf (cache-érvénytelenítés) és a determinizmus.

### 3.4 Függvények
Kis, zárt lista: `abs min max clamp lerp smoothstep step floor ceil round sqrt pow exp log sin cos tan atan2 length dot cross normalize fit`, `noise(vec3)` (determinisztikus gradiens-zaj), `hash(int…)`, `rand(seed, id)` (kriptográfiailag nem erős, determinisztikus). Minden függvény teljes (nincs „nem definiált”): `sqrt(-1)`, `log(0)` stb. definiált értéket (NaN-t vagy 0-t) ad, **dokumentálva**; a NaN a kimeneten jelölt (nem terjed csendben).

## 4. Architektúra

| Elem | Hely | Feladat |
|---|---|---|
| **cfa4-expr** (új, L0) | `crates/expr` | lexer, parser, típusellenőrző, bájtkód-fordító, VM; tiszta, `unsafe` nélkül; hiba = `Result`, soha pánik |
| **Volume-híd** | `crates/vdb` bővítés + `workers/vdb` új parancsok (`read-tiles`, `write-tiles`) | a worker blokkokat (leaf-tile, 8³) ír nyers fájlba / olvas vissza; a VM a Rust oldalon fut, a worker a sandboxban marad |
| **Engine parancsok** | `crates/engine` | `expr.check`, `expr.run_preview`, `expr.bake`, `expr.cancel`, `expr.approve` (id-vel, mint minden parancs) |
| **Projekt** | `crates/project` | a kifejezés forrásszövege + hash + paraméterek + „jóváhagyva ezen a gépen” jelző (nem a projektfájlban) |
| **Qt6 szerkesztő** | `python/cfa4_qt/panels/` | szintaxiskiemelés, függvénylista, hibák helyükön, compile-állapot, előnézet; automation-id |

Blokkos végrehajtás: egy voxel kiértékelése tiszta függvény a bemeneti blokkokból, ezért a szálszám **nem** befolyásolja az eredményt (a N2 render determinizmus-elvével azonos; teszt: 1 és N szál bitazonos).

## 5. Biztonsági modell

1. **Betöltés:** a projektbetöltés **nem futtat** kifejezést. A kifejezés csak felhasználói művelettel (előnézet, bake) fut; a „bake” és a mentett eredmény felhasználása **jóváhagyást** kér, ha a kifejezés forrása nem ebből a gépről/sessionből jön (hash alapján). (Az interpreter szerkezetileg ártalmatlan, de az erőforrás-felhasználás nem: lásd 6. pont.)
2. **Nincs I/O a nyelvben:** nincs fájl, hálózat, folyamat, óra, környezeti változó. A „nem determinisztikus” forrás sincs (nincs idő, nincs OS-véletlen).
3. **Fuel:** minden bájtkód-utasítás fuelt fogyaszt; voxelenként és futásonként is korlátos (alap: 10 000 utasítás/voxel, a felhasználó csökkentheti, növelheti egy kemény felső határig). Kimerülés = hiba a hely megjelölésével, nem lefagyás.
4. **Memória:** a VM verem- és regiszterszáma a fordításkor kiszámolt, rögzített; nincs dinamikus allokáció a voxel-kiértékelésben. A blokk-puffer méretét a híd korlátozza.
5. **Megszakítás:** blokkonként ellenőriz egy `cancel` jelzőt; egy blokk legfeljebb a fuel-korlát ideig tart.
6. **A voxeladat útja:** a worker sandbox (Landlock + seccomp, N1) változatlan; az interpreter ebben nem fut, tehát a sandbox profilja **nem tágul**.
7. **Fuzz:** parser/típusellenőrző/VM coverage-vezérelt fuzz-célja (`fuzz/fuzz_targets/expr.rs`); a „0 összeomlás” kapu a CI `fuzz-targets` jobjában fut.

## 6. Mérföldkövek és elfogadási feltételek

| Fázis | Tartalom | Elfogadás |
|---|---|---|
| **X0** | Nyelvleírás ([`docs/expr-language.md`](../expr-language.md)), nyelvtan, hiba-katalógus, példák | a tulajdonos átnézte a nyelvet; a 3. szakasz minden eleme példával; nincs kód |
| **X1** | `cfa4-expr`: lexer, parser, típusellenőrzés, bájtkód, VM, fuel, determinisztikus zaj | lásd N3-1′…N3-4′ lent; `cargo xtask ci` + `cargo test -p cfa4-expr` zöld; fuzz 0 összeomlás |
| **X2** | Volume-híd (worker tile-parancsok + `cfa4-vdb` API) és `expr.run_preview` | a N2 előnézeti útvonalon látható eredmény; 1 és N szál bitazonos; megszakítás ≤ 1 blokk idő |
| **X3** | Engine-parancsok, projektmentés, jóváhagyási jelző, `expr.bake` | N3-5′, N3-7′ |
| **X4** | Qt6 szerkesztő | N3-6′; `--smoke --screenshot` ellenőrzés |
| **X5** | Kiszökési- és erőforrás-tesztcsomag (hostile kifejezések), mérés a referenciagépen | N3-S1′…S3′ |

**Az N3 sorok pontosítása az interpreterre** (a szülő terv N3-S és N3-1…7 sorai helyett/mellé; a szülő terv szövegének módosítása tulajdonosi jóváhagyást kér):

| ID | Feltétel |
|---|---|
| N3-S1′ | Nincs a nyelvben I/O: hostile kifejezés-korpusz (fájl-, hálózat-, folyamat-, idő-kísérlet szintaxisa) mind fordítási hibát ad; a worker sandbox profilja a baseline-hoz képest változatlan (diff = üres) |
| N3-S2′ | Fuel- és memóriakorlát: végtelen/óriási ciklus, mély kifejezésfa, nagy konstans-határ mind hibával áll meg a határon belül; a gazdafolyamat RSS-növekedése korlátos (mérve) |
| N3-S3′ | Fuzz (parser + típusellenőrző + VM) 0 összeomlás, 0 pánik, 0 timeout |
| N3-S4′ | Új külső függőség: nincs; a `cargo deny` zöld |
| N3-1′ | Volume-kontextus (több attribútum, vec3 is); pont-kontextus **szintetikus oszlopokon** |
| N3-2′ | Fordítási és futási hiba pontos hely- (sor:oszlop) megjelöléssel, soha pánik |
| N3-3′ | Megszakítás; nagy adathalmaz (≈1 GiB aktív voxeladat, csúcs-RSS mérve, mint N1-2); CPU-only |
| N3-4′ | Determinizmus: azonos bemenet + azonos paraméterek → bitazonos kimenet, bármely szálszámmal; a NaN/definiálatlan viselkedés dokumentálva |
| N3-5′ | Mentés, preset, újrahasználat; projekt-round-trip (forrás + hash) |
| N3-6′ | Szintaxiskiemelés, függvénylista, validálás, compile-állapot, előnézet a Qt6-ban (automation-id, parancsok) |
| N3-7′ | A projektbetöltés nem futtat kifejezést jóváhagyás nélkül; a jóváhagyás nem kerül a projektfájlba |

## 7. Teljesítmény (becslés, nem mérés)

A bájtkód-VM voxelenként jellemzően tízszer–százszor lassabb a JIT-nél; ez ellenőrizetlen becslés, **mérendő X1/X2-ben** (referenciagép, AGENTS owner decision 2026-11-02). Ellenszer: blokk szintű párhuzamosítás, aktív voxelek kizárólagos feldolgozása (a háttérértékű blokkokat kihagyjuk), konstans-hajtogatás, a háttérértékre kiszámolt eredmény újrahasználata. Ha az X2 mérése szerint a 1 GiB aktív adat feldolgozása a felhasználónak elfogadhatatlan (ennek határát a tulajdonos adja meg, Q-X2), a döntés visszakerül a tulajdonoshoz: (a) marad így, háttérfeladatként (ACS-6 sor, #1471), (b) gépikód-generálású gyors útvonal egy későbbi tervben.

## 8. Összefüggések

- **N1/N2:** a worker és a sandbox újrahasznosul; az N1-6/N1-9 és N2-5 nyitott sorai (referenciagép, ACS-6) ezt a tervet is érintik (háttérsor).
- **Effektlánc ([CFA4-EFFECTCHAIN-001](CFA4-EFFECTCHAIN-001-0.1.md)):** a kifejezés effekt-szerű bejegyzés az Effect Controlsban; a paraméterek, preset és kulcskockák az effektláncé.
- **OpenUSD/3D, Pontfelhő:** a pont-kontextus valós forrása onnan jön, nem innen.
- **CFA4-SECURITY-001 B3/B4:** nem előfeltétel (a nyelv nem ír/olvas semmit magától); a worker sandboxa B3-tól függetlenül már él (N1).

## 9. Döntések (a javaslatok elfogadva, István 2026-11-22 18:56)

| ID | Kérdés | Javaslat |
|---|---|---|
| Q-X1 | Elegendő-e a v1 nyelv szomszéd-hozzáférés nélkül? (pl. blur/erózió nem írható meg) | Igen v1-re; v2-ben korlátos szomszédság (`neighbor(ijk+d)`, ≤ 3 sugár), külön elfogadási sorokkal |
| Q-X2 | Mi az elfogadható futásidő 1 GiB aktív voxeladaton CPU-n? | Háttérfeladat, perc nagyságrend; a számot Te adod meg az X2 mérése előtt |
| Q-X3 | Zaj: saját egyszerű gradiens-zaj vagy kell Perlin/Simplex-szerű, ismert mintázat? (szabadalmi átnézés kell) | Saját hash-alapú gradiens-zaj, ismert algoritmus megnevezése nélkül; szabadalmi átnézés X1 előtt |
| Q-X4 | Kell-e AX-szkriptek részleges importja (konverter)? | Nem; csak ha később valós szükség és a licenc rendben |
| Q-X5 | A pont-kontextus mehet-e szintetikus oszlopokon, a valós forrás nélkül? | Igen; a bizonyítás szintetikus, a valós forrás a Pontfelhő terv |

A Q-X2 számszerű határa még nem ismert (a javaslat: háttérfeladat, perc nagyságrend); X2 mérése előtt kérem.

## 10. Ellenőrizetlen / nyitott

- A CFX teljesítménye (mérés kell, §7).
- Az AX dokumentációjának licence (nem ellenőrizve; nem használjuk).
- A gradiens-zaj szabadalmi helyzete (átnézés X1 előtt).
- A worker `read-tiles`/`write-tiles` parancsok memória- és sebességprofilja nagy adaton (X2).
- Az ACS-6 háttérsor (#1471) állapota X2 idején.

## 11. Jóváhagyási lánc

1. Ez a terv (0.1) jóváhagyása → X0 indulhat (nyelvleírás).
2. Külön tulajdonosi engedély kell X1-re (kód); minden további fázisra a coordination §2.10 szerint.
3. A szülő terv G4 sora és az N3-S/N3 sorok módosítása (§6 táblázat) külön jóváhagyás tárgya.
