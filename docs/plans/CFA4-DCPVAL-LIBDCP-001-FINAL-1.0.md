# CFA4-DCPVAL-LIBDCP-001 v1.0 – A CFA4 DCP-ellenőrző és a libdcp két eltérése: végleges terv

**Állapot:** **végleges, a tulajdonos által jóváhagyva (2026-11-14):** „A terv jóváhagyva”. A nyitott pontok (D0–D5) a javaslatok szerint dőltek el (lásd 3. pont).  
**Verzió:** 1.0, 2026-11-14  
**Kapcsolódik:** [CFA4-DCPOMATIC-001](CFA4-DCPOMATIC-001-FINAL-1.0.md) (2. fázis elfogadása), [DCP-VALIDATION](DCP-VALIDATION-2026-10-27.md)  
**Kiváltó ok:** [DCPOM-01 mérés](../audit/DCPOM-01-DCPOMATIC-SURVEY-2026-11-13.md) §3: a DCP-o-matic 2.18.51 DCP-jeit a libdcp `dcpverify` elfogadja, a `cfa4-dcp-validate` két szabályon elbuktatja (`DCP.AM.002` Interopon, `DCP.J2K.001` a mozis profilokon).
  
**Projekthatár:** a javítás a DCP-validáció projektjének (`docs/plans/DCP-VALIDATION-2026-10-27.md`, crate-ek: `cfa4-dcp`, `cfa4-dcp-validate`) és a DCP-exportnak (`crates/export/src/dcp_out.rs`) a kódja. A `docs/coordination.md` §2.8 szerint a tulajdonos dönt arról, melyik projekt végzi; a D0 szerint a DCP-validáció projektje.

## 1. Röviden

| Eltérés | Ki téved? | Bizonyosság | Javasolt javítás |
|---|---|---|---|
| 1. Interop ASSETMAP elemsorrend (`DCP.AM.002`) | **A CFA4.** | Biztos: a két séma sorrendje szövegszerűen eltér. | Interopra az Interop séma sorrendje (`… IssueDate, Issuer, Creator, AssetList`). |
| 2. J2K többkomponensű transzformáció, MCT (`DCP.J2K.001`) | **Nagy valószínűséggel a CFA4.** | Közepes-magas: a libdcp, a DCI DCSS és a gyakorlat egy irányba mutat; az ISO-tábla szövegét nem tudtam ellenőrizni (fizetős). | Az MCT = 1 legyen elfogadott. A CFA4 exportja is MCT = 1-gyel kódoljon, mert a libdcp az MCT = 0-t **hibának** veszi, így a CFA4 saját DCP-i valószínűleg ma elbuknak a `dcpverify`-on (ezt az 1. fázis méri meg). |

## 2. Kiinduló állapot és bizonyítékok

### 2.1 Interop ASSETMAP elemsorrend

- **CFA4:** `crates/dcp/src/assetmap.rs:55` egyetlen sorrendet használ mindkét szabványra: `Id, AnnotationText, Creator, VolumeCount, IssueDate, Issuer, AssetList`. A `Reader::unknown` (`crates/dcp/src/xml.rs:139`) ebből a listából jelzi a sorrendhibát.
- **SMPTE ST 429-9 séma** (`SMPTE-429-9-2007-AM.xsd`, a libdcp v1.10.64-ben): `Id, AnnotationText?, Creator, VolumeCount, IssueDate, Issuer, AssetList`. Ez egyezik a CFA4 listájával.
- **Interop séma** (`PROTO-ASDCP-AM-20040311.xsd`, „Assembled from mpeg_ii_am_spec.doc 2004-11-23”): `Id, AnnotationText?, VolumeCount, IssueDate, Issuer, Creator, AssetList`. Itt a `Creator` az `Issuer` után jön, és a `VolumeCount` megelőzi.
- **libdcp író** (`src/asset_map.cc`, `AssetMap::write_xml`): szabványonként külön ág, Interopnál pontosan a séma sorrendje. A `dcpverify` Xercesszel a fenti sémák szerint ellenőriz (`src/verify.cc`, `validate_xml`).
- **CFA4 teszt:** `crates/dcp/src/tests.rs:135` Interop-mintája SMPTE-sorrendben van, és a `schema_order_and_positive_volume_fields` teszt (`:423`) a helyes Interop-sorrendet hibának várja. A teszt tehát a hibát rögzíti.
- **A többi dokumentum rendben van:** a PKL (`pkl.rs:43`) és a CPL (`cpl.rs:149`) gyökérsorrendje mindkét szabvány sémájával egyezik; az Asset és Chunk elemek sorrendje is.
- **CFA4 export:** csak SMPTE ASSETMAP-et ír (`dcp_out.rs:925`), ezt a javítás nem érinti.

### 2.2 J2K MCT

- **CFA4 ellenőrző:** `crates/dcp-validate/src/codestream.rs:152`: ha a fő COD `SGcod` MCT bájtja 1, akkor `Fail`: „the profile codes X'Y'Z' without one”. Forrásként az ISO/IEC 15444-1 Amd 1-et és az ST 429-4-et nevezi meg, de konkrét táblát vagy sort nem.
- **CFA4 export:** `crates/export/src/dcp_out.rs:336` `mct: false`-szal kódol. (Az IMF-export, `imf_out.rs:130`, `mct: true`.) A dekóder (`crates/j2k/src/decode.rs:588`) az inverz ICT-t tudja.
- **libdcp ellenőrző** (`src/verify_j2k.cc:246`): `require_8(1, "invalid multi-component transform flag %1")`. Az MCT ≠ 1 `INVALID_JPEG2000_CODESTREAM`, ennek típusa **ERROR** (`verify.cc:2328`). Alapból fut (`check_picture_details = true`).
- **libdcp kódoló:** 2015-ös commit `3afe04f2` „Missing specification of tcp_mct=1.” A szerző hiányzó előírásnak tekintette, nem választásnak. A DCP-o-matic ezt a kódolót használja.
- **DCI DCSS 1.4.2, §4.3.2:** „All decoders shall implement the inverse Irreversible Color Transform (ICT) using at least 16 bit fixed point precision.” A §4.3.3 kódfolyam-előírásai (Rsiz, csempe, szintek, kódblokk, precinct, CPRL, POC, csempe-részek) az MCT-t nem tiltják. Ha a profil MCT nélküli volna, a kötelező dekóder-ICT-nek nem lenne értelme.
- **Nem ellenőrzött:** az ISO/IEC 15444-1 (ITU-T T.800) Annex A 2K/4K profiltáblájának MCT-sora. A T.800 hatályos kiadása fizetős, a 2019-es ingyenes PDF nem volt elérhető. Emiatt a 2. eltérésnél „nagy valószínűséggel” a minősítés, nem „biztos”.

**Következtetés:** az MCT = 1-es kódfolyam a libdcp, a DCP-o-matic és a DCSS dekóder-előírása szerint is érvényes, ezért a CFA4 `Fail`-je téves. Hogy az MCT = 0 *tiltott-e*, azt csak az ISO-tábla döntheti el. A libdcp hibának veszi, tehát a gyakorlatban kockázatos.

## 3. Tulajdonosi döntések

| # | Kérdés | Javaslat | Döntés |
|---|---|---|---|
| D0 | Melyik projekt végzi a javítást (§2.8)? | A DCP-validáció projektje, mert a két szabály az övé; az export MCT-váltása ugyanabban a tervben, külön PR-ban. A tulajdonos a gazdával közösen dönt. | A javaslat szerint (István, 2026-11-14, a terv jóváhagyásával; a gazda aznap nem volt elérhető). |
| D1 | Interop ASSETMAP: javítsuk a sorrendet? | Igen, szabványonkénti sorrendlista. | A javaslat szerint (István, 2026-11-14, a terv jóváhagyásával). |
| D2 | MCT a CFA4 ellenőrzőjében | MCT = 1: Pass. MCT = 0: **WARNING** („a libdcp hibának jelzi; az ISO-tábla nincs ellenőrizve”), amíg D4 el nem dől. | A javaslat szerint (István, 2026-11-14, a terv jóváhagyásával). |
| D3 | A CFA4 DCP-exportja MCT = 1-gyel kódoljon? | Igen, ha az 1. fázis mérése megerősíti, hogy a mai export elbukik a `dcpverify`-on. | A javaslat szerint (István, 2026-11-14, a terv jóváhagyásával). |
| D4 | Beszerezzük az ISO/IEC 15444-1 (T.800) szövegét a végleges döntéshez? | Igen, ha van rá keret. Ha az MCT = 0 tiltott, a D2 WARNING-ja FAIL lesz. Ha nincs keret, a D2 marad. | A javaslat szerint (István, 2026-11-14, a terv jóváhagyásával). |
| D5 | DCPOM-2 elfogadási feltétele | Lásd 5. pont. | A javaslat szerint (István, 2026-11-14, a terv jóváhagyásával). |

## 4. Fázisok (mindegyik külön PR, a tulajdonos olvaszt be a gazdán keresztül)

### 1. fázis – Mérés (kódváltozás nélkül)

- Egy CFA4-exportált 2K SMPTE DCP-t (`target/fixtures/`) lefuttatni a libdcp v1.10.64 `dcpverify`-án (a DCPOM-01-ben fordított eszközzel).
- Rögzíteni: van-e `INVALID_JPEG2000_CODESTREAM … multi-component transform flag 0`, és van-e más eltérés.
- Kimenet: `docs/audit/DCPVAL-LIBDCP-01-…md`, nyers kimenettel.

**Elfogadás:** a jelentés megmondja, hány eltérés van a CFA4-export és a `dcpverify` között, és melyek azok.

### 2. fázis – Interop ASSETMAP sorrend (D1)

- `crates/dcp/src/assetmap.rs`: két lista (`ROOT_SMPTE`, `ROOT_INTEROP`), a `standard` szerint választva.
- Tesztek: a `tests.rs` Interop-mintája Interop-sorrendre; a `schema_order…` teszt fordítva (Interopon a SMPTE-sorrend a hiba); új teszt: SMPTE-sorrend SMPTE-n Pass, Interop-sorrend SMPTE-n Fail.
- `crates/dcp-validate/README.md` és `crates/dcp/README.md`: a két séma neve és sorrendje.

**Elfogadás:** a DCPOM-01 Interop DCP-je `full-integrity` profilon Pass; a teljes kapu zöld.

### 3. fázis – MCT az ellenőrzőben (D2)

- `codestream.rs`: MCT = 1 elfogadva; MCT = 0 a D2 szerinti súlyossággal; a modul fejkommentje és a README forrásmegjelöléssel (DCSS §4.3.2, libdcp viselkedése, és hogy az ISO-tábla még nincs ellenőrizve).
- Tesztek: MCT = 1 és MCT = 0 kódfolyam a `cfa4-j2k` kódolóval.

**Elfogadás:** a DCPOM-01 SMPTE és Interop DCP-je `smpte-cinema` / `interop-cinema` profilon nem bukik `DCP.J2K.001`-en.

### 4. fázis – A CFA4 DCP-exportja MCT = 1-gyel (D3)

- `dcp_out.rs:336` `mct: true`. A bitráta-korlát komponensenként (200 Mbit/s) újramérendő, mert az ICT átrendezi a komponensek közti bájtokat.
- Tesztek: az export utáni saját ellenőrzés (`DCP.J2K.001`) és a meglévő golden/visszaolvasási tesztek; a dekódolt X'Y'Z' eltérése a mostani tűréshatáron belül.

**Elfogadás:** egy CFA4-export hiba nélkül megy át a `dcpverify`-on (az 1. fázis módszerével) és a CFA4 saját ellenőrzésén.

## 5. Javaslat a DCPOM-2 elfogadási feltételére (D5)

- **Most, változtatás nélkül:** a DCPOM-2 feltétele a mostani szövegben maradhat („`full-integrity` SMPTE és Interop szerint”), de az Interop rész csak a 2. fázis beolvasztása után teljesülhet. Javaslat: a DCPOM-2 PR-ja **a 2. fázisra várjon**, vagy addig az Interop `full-integrity` futás `DCP.AM.002`-n kívül legyen hibamentes, név szerint kivételezve, és a kivétel a 2. fázissal megszűnjön.
- **A 3. fázis után:** a mozis profilok (`smpte-cinema`, `interop-cinema`) bekerülhetnek a feltételbe, „FAIL nélkül” formában (a még meg nem írt szabályok `Unchecked`-je és a markerfigyelmeztetések nem buktatnak).
- **Mindkét irány:** a D10 CI-jobban a DCP-o-matic DCP-jén a `dcpverify` is fusson (a libdcp úgyis része a szállított példánynak), ERROR nélkül. Így a két ellenőrző eltérése a jövőben azonnal kiderül.

## 6. Kockázatok

- **ISO-szöveg nélkül** a D2 ideiglenes. Ha a profil az MCT = 0-t is megengedi, a WARNING túl szigorú, de nem buktat.
- **Export-változás (4. fázis):** a CFA4-exportok bájtra mások lesznek; a golden tesztek frissülnek. A színhelyesség a dekódolt X'Y'Z' összevetésével ellenőrizhető.
- **libdcp mint referencia:** a libdcp nem szabvány, csak a legelterjedtebb nyílt implementáció. A terv ezért a forrásokat (sémák, DCSS) helyezi előre, és a libdcp-t csak megerősítésnek használja.
- **Ütközés más projekttel:** a 2–4. fázis a DCP-validáció projektjének kódja (D0). A D0 a javaslat szerint dőlt el (2026-11-14).

## 7. Források

- libdcp v1.10.64 (`891facc`, git.carlh.net): `xsd/PROTO-ASDCP-AM-20040311.xsd`, `xsd/SMPTE-429-9-2007-AM.xsd`, `src/asset_map.cc`, `src/verify.cc`, `src/verify_j2k.cc`; commit `3afe04f2` (2015-12-11).
- DCI Digital Cinema System Specification 1.4.2, §4.3.2–4.3.3: https://dcss.dcimovies.com/0c0cff34d231b516cb89ae3fad352d5cf37a9515/dcss.pdf
- ITU-T T.800 oldala (a 2005-ös Amendment 1 címe „Profiles for digital cinema applications”; a szöveg fizetős): https://www.itu.int/rec/T-REC-T.800/en
- CFA4 `main` @ `5baa2feb` (a sorszámok `89107cf8`-on is érvényesek): `crates/dcp/src/assetmap.rs`, `crates/dcp/src/xml.rs`, `crates/dcp/src/tests.rs`, `crates/dcp-validate/src/codestream.rs`, `crates/export/src/dcp_out.rs`.
