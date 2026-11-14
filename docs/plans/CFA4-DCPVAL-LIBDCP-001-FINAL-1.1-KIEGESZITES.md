# CFA4-DCPVAL-LIBDCP-001 v1.1 – kiegészítés: a mérés két új eltérése

**Állapot:** jóváhagyva (István, 2026-11-14). A D6–D9 döntések a javaslat szerint.
**Verzió:** 1.1, 2026-11-14
**Alap:** [CFA4-DCPVAL-LIBDCP-001 v1.0](CFA4-DCPVAL-LIBDCP-001-FINAL-1.0.md) (jóváhagyva 2026-11-14). A v1.0 minden pontja változatlan marad, ez csak hozzáad.
**Kiváltó ok:** a DCPVAL-LIBDCP-01 mérés (PR #1222) §2 és §3.1: a CFA4-export az MCT-javítás után is két okból kap hibát a libdcp `dcpverify`-on. A tulajdonos 2026-11-14-én döntött: a terv 1.1-es kiegészítést kap.

## 1. Röviden

| Eltérés | Ki téved? | Bizonyosság | Javasolt javítás |
|---|---|---|---|
| 3. `MainMarkers` `EntryPoint`-tal és `Duration`-nel | Egyik sem szabálytalan. A libdcp szigorúbb a sémánál. | Biztos, a séma alapján | A CFA4-export hagyja el a két elemet (a séma szerint opcionálisak). A CFA4 ellenőrzője WARNING-ot adjon rájuk. |
| 4. 2K kódfolyam 2 guard bittel | **A CFA4 kódolója.** | Magas: a CFA4 saját ellenőrzője, a libdcp és a libdcp kódolója is 1-et vár 2K-n | A `cfa4-j2k` 2K mozis profilban 1 guard bitet írjon, és telítse a ritka túlcsorduló indexeket. |

## 2. Bizonyítékok

### 2.1 MainMarkers

- **SMPTE ST 429-7 séma** (`SMPTE-429-7-2006-CPL.xsd`, a libdcp v1.10.64-ben): a `MainMarkers` típusa `MarkerAssetType`, ez a `GenericAssetType` kiterjesztése. Abban az `EntryPoint` és a `Duration` is `minOccurs="0"`. A séma tehát megengedi, de nem kéri őket.
- **libdcp:** `UNEXPECTED_ENTRY_POINT` és `UNEXPECTED_DURATION`, típusuk ERROR (`src/verify.cc`). Commit `56fcbad9` (2022-03-24): „Add verify test to check for erroneous <EntryPoint> and <Duration> tags inside <MainMarkers>”. Szabványpontot nem nevez meg.
- **CFA4 export:** `crates/export/src/dcp_out.rs:818` mindkettőt kiírja. A CFA4 saját ellenőrzőjének tesztmintája viszont már nélkülük írja a jelölőket (`crates/dcp-validate/src/tests.rs`: „the markers have no Duration”).
- **Következtetés:** az elhagyás mindkét ellenőrzőnek megfelel, és semmit nem veszít, mert a jelölő-assetnek nincs saját lejátszandó tartalma.

### 2.2 2K guard bitek

- **CFA4 ellenőrző:** `DCP.J2K.001` FAIL, ha 2K-n nem 1 (4K-n nem 2) a guard bitek száma (`crates/dcp-validate/src/codestream.rs`). A teszt rögzíti, hogy a `cfa4-j2k` kimenete ezen elbukik („as cfa4-j2k writes it: 2 guard bits”).
- **libdcp ellenőrző:** `INVALID_JPEG2000_GUARD_BITS_FOR_2K` (Bv2.1 error), 4K-n 2-t vár (`src/verify_j2k.cc:275-280`).
- **libdcp kódoló:** `parameters.numgbits = fourk ? 2 : 1` (`src/j2k_transcode.cc:276`).
- **CFA4 kódoló:** `crates/j2k/src/encode/mod.rs`: „guard bits: enough for the largest index of every band”, legalább 2. Így a 2K kép soha nem felel meg a profilnak.
- **Miért nem elég nagyobb lépésköz:** expounded kvantálásnál a bitsíkok száma `Mb = G + e − 1`. Ha a lépésközt kétszerezzük, az index is és az `e` is eggyel csökken, tehát a guard bit-igény nem változik. 1 guard bitnél a ritka, a névleges tartományon túli indexeket telíteni kell (vágás a legnagyobb ábrázolható értékre). A libdcp kódolója (OpenJPEG) is ezt teszi.

## 3. Tulajdonosi döntések

| # | Kérdés | Döntés |
|---|---|---|
| D6 | MainMarkers a CFA4-exportban | Az `EntryPoint` és a `Duration` elmarad. |
| D7 | MainMarkers a CFA4 ellenőrzőjében | WARNING („a séma megengedi, a libdcp hibának jelzi”), nem FAIL. |
| D8 | 2K guard bitek | A 2K mozis profil 1 guard bitet ír, a túlcsorduló indexek telítődnek; a 4K marad 2. A többi profil (IMF, Part 1) nem változik. |
| D9 | A v1.0 4. fázisának elfogadása | Változatlan: a CFA4-export hiba nélkül megy át a `dcpverify`-on. Ezt az 5. és 6. fázis után a 7. fázis méri. A 4. fázis PR-je az MCT-t javítja, és nem állítja, hogy egyedül teljesíti az elfogadást. |

## 4. Új fázisok (mindegyik külön PR; a beolvasztás sorrendje a v1.0 fázisai után)

### 5. fázis – MainMarkers (D6, D7)

- `crates/export/src/dcp_out.rs`: a `MainMarkers` `EntryPoint` és `Duration` nélkül.
- `crates/dcp-validate`: új WARNING, ha a `MainMarkers`-ben van `EntryPoint` vagy `Duration`. Szabálykód: a meglévő `DCP.MARK.001` („Composition markers are present and in range”).
- Tesztek: az export CPL-jében nincs `EntryPoint`/`Duration` a jelölőkben; az ellenőrző WARNING-ot ad, ha van.

**Elfogadás:** a `dcpverify` a CFA4-exporton nem ad `UNEXPECTED_ENTRY_POINT` / `UNEXPECTED_DURATION` jelzést; a CFA4 ellenőrzője a régi exportra WARNING-ot ad, az újra nem.

### 6. fázis – 2K guard bitek (D8)

- `crates/j2k/src/encode/mod.rs`: a `Profile::Cinema2k` 1 guard bitet ír. A kvantált indexek a `G + e − 1` bitre telítődnek; a telített együtthatók számát a kódoló statisztikája jelzi.
- Tesztek: a 2K mozis kódfolyam QCD-je 1 guard bit; szürke, színes és szélsőséges (0 és 4095 közötti ugrás) 12 bites X'Y'Z' képen a PSNR legalább 50 dB (a mostani teszt küszöbe); a 4K, IMF és Part 1 kimenet bájtra változatlan.
- A `crates/dcp-validate` `encoded_2k()` tesztsegédje már nem kell, hogy átírja a guard biteket.

**Elfogadás:** a CFA4 2K-exportja a CFA4 `DCP.J2K.001`-én és a `dcpverify`-on is átmegy guard bit-jelzés nélkül.

### 7. fázis – Ellenőrző mérés (kódváltozás nélkül)

- A DCPVAL-LIBDCP-01 mérés megismétlése 1 másodpercnél hosszabb, nem Feature tartalomtípusú mintával, hogy a teszt-mellékhatások ne zavarjanak.
- Kimenet: `docs/audit/DCPVAL-LIBDCP-02-…md`.

**Elfogadás:** a `dcpverify` ERROR és Bv2.1 error nélkül fut le a CFA4 2K és 4K exportján (a v1.0 4. fázisának elfogadása, D9).

## 5. Kockázatok

- **Telítés (6. fázis):** a névleges tartományon túli együtthatók vágása kis, helyi hibát okoz. A PSNR-teszt és a szélsőséges minta ezt méri. Ha a küszöb nem teljesül, a fázis megáll és visszajelzek.
- **Kodek-crate:** a 6. fázis a `crates/j2k`-t érinti, ezért CodeRabbit-ellenőrzés kell (`docs/coordination.md` §2.9).
- **MainMarkers és más eszközök:** a régi, `EntryPoint`-os CFA4 DCP-k a CFA4 ellenőrzőjén WARNING-ot kapnak, nem FAIL-t, ezért nem buknak el visszamenőleg.
