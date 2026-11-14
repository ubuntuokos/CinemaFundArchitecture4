# CINECERT-WP00 – DCP/IMF gap matrix az aktuális `main`-en

**Projekt:** CFA4-PLAN-CINECERT-001 v2.3 ([terv](../plans/CFA4-CINECERT-001-v2.3.md), jóváhagyva: István, 2026-11-13)  
**Munkacsomag:** WP00 (1. PR a terv 15. fejezete szerint)  
**Ellenőrzött forrásállapot:** `main` @ `92e29e4b967480a933a9a8bd297038a140955574` (2026-11-13 03:46 +0100), csak olvasással; nyitott PR-ok a GitHub REST API-ból ugyanakkor  
**Állapot:** tényfelmérés; az átfedésekről a tulajdonos és a gazda dönt ([`docs/coordination.md`](../coordination.md) §2.8)

## 1. Röviden

A terv a 2026-10-27-i `main`-re (`16debf9`) épült. Azóta más projektek a terv M2-ig tartó részének
nagy részét megcsinálták: SMPTE DCP export (#115, RDD 52, MCA, ST 429-16, IAB), JPEG 2000 MXF import
(#77), DCP XML olvasó (`cfa4-dcp`, SMPTE és Interop), DCP-validáció (`cfa4-dcp-validate`,
`cfa4-dcp-inspect`), KDM-nyitás és titkosított track file olvasás (ST 430-1/-3, ST 429-6), IMF App 2E
csomagexport (#1011, `imf_out.rs`), IMF J2K profilok (#931), és nyitott PR-ban a HTJ2K kódolás (#1113).

Ezek gazdái:

- **CFA4-PROCODEC-002** ([terv](../plans/CFA4-PROCODEC-002-2026-11-11.md), jóváhagyva 2026-11-11):
  kimondja, hogy „semmi nem kerül külön projektbe”, a DCP-csomagolás (5a), az IMF (5b), a J2K/HTJ2K
  (4) és a DCP/IMF feliratok is az ő részei.
- **CFA4-DCP-VALIDATION-001** ([terv](../plans/DCP-VALIDATION-2026-10-27.md)): DCP ingest és
  validáció (DCP-02 … DCP-06+).
- **G2 KDM-munka** (#1028, #1053, #1075; nyitott PR-ok #1076, #1084, #1086): KDM-aláírás, Qt KDM-párbeszéd.

Ezért a terv 15. fejezetének PR-sorrendje így áll:

| # | Terv szerinti PR | Állapot a `main`-en | Teendő |
|---|---|---|---|
| 1 | `docs` WP00 | ez a dokumentum | – |
| 2 | `xtask` orákulum-telepítő, korpusz-letöltő | részben: `setup-dcp-inspect.sh` (DCP-VALIDATION), `CFA4_PHOTON_CP` a tesztekben, telepítő nincs | csak a hiányzó rész (asdcplib, Photon, Clairmeta, korpusz); asdcplib: átfedés a DCP-01 L4-gyel |
| 3 | `mxf` ST 422 J2K olvasás + `cfa4-codecs` | **kész** (#77) | elmarad |
| 4 | `mxf` J2K és PCM DCP track file írás, MCA | **kész** (#115, #309 MCA-része) | elmarad |
| 5 | `cinema-pkg` CPL/PKL/ASSETMAP olvasás | **kész** más crate-ben: `cfa4-dcp` | elmarad; új crate nem kell |
| 6 | `cinema-pkg` írás, determinisztikus UUID/óra, SHA-1 | írás kész (`dcp_out.rs`, `imf_out.rs`); injektálható UUID/óra **hiányzik** | átfedés (PROCODEC-002 5a/5b) |
| 7 | DCP inspect parancs + CLI/MCP | validáció kész; CLI/MCP-parancs a DCP-VALIDATION terv része | átfedés |
| 8 | `j2k` sebesség (#116), tile/rétegek (#117) | #116 lezárva; #117 nyitott; HTJ2K kódolás PR #1113 | átfedés (PROCODEC-002 4) |
| 9 | DCP composer + export preset | **kész** (#115, #343) | elmarad |
| 10 | `captions` ST 428-7 írás/olvasás | olvasás kész (`dcst.rs`); írás hiányzik | átfedés (PROCODEC-002) |
| 11 | QC-szabályok + JSON riport | DCP: kész (`cfa4-dcp-report/1`); IMF: hiányzik | DCP: átfedés; IMF: PROCODEC-002 5b elfogadási feltétele |
| 12 | Qt6 bridge-parancsok és panelek | részben (#343 Qt IAB, KDM-párbeszéd PR-ok) | átfedés, D4 |

**Kizárólag a CineCert-tervhez tartozó, más projektben nem szereplő részek** (a „CineCert” gazdájú sorok a 3. pontban):

1. WP01-A: orákulum-környezet (asdcplib CLI, Netflix Photon, Clairmeta) telepítője és a DCP/IMF
   tesztkorpusz pinelt, sha256-ellenőrzött letöltése (terv 13. fejezet);
2. WP08: `mxf-digest` (KLV-alapú SHA-512) és IMF virtuális sáv ujjlenyomat (VTFP);
3. WP09: ISDCF registry offline snapshot és ISDCF Doc 2 névépítő/-ellenőrző;
4. WP13: átadás és archiválás (ISDCF meghajtó ext2/ext3, több kötetes VOLINDEX, provenance);
5. a terv 8. fejezetének kiadás előtti elfogadási tesztjei külső validátorokkal (kereszt-ellenőrzés).

## 2. Döntések, amelyeket egy másik jóváhagyott terv már érint

| Terv-döntés (16. fejezet) | CineCert-javaslat | Amit a `main` mutat |
|---|---|---|
| D2 Interop írás | csak olvasás, legacy írás figyelmeztetéssel | PROCODEC-002 „Célállapot”: DCP „SMPTE és Interop” írás, 5a: „Interop DCP export a meglévő SMPTE mellé” |
| D3 KDM/titkosítás | igen, M4-ben | PROCODEC-002 célállapot: „titkosítás/KDM nem”; közben a KDM **olvasása** (nyitás, aláírás-ellenőrzés PR-ban) a G2-ben elkészült; [`cinema-codecs.md`](../cinema-codecs.md) eddig „out of scope”-ot írt (javítva ebben a PR-ban) |
| D4 felület M2-höz | CLI + MCP, Qt6 panel később | Qt6 IAB DCP export (#343) és Qt KDM-párbeszéd már készül |
| D1 újradöntési kapu (M2 után) | M2 = validált titkosítatlan SMPTE DCP | az export és a natív + `dcp_inspect` validáció megvan; hogy az M2 „kész”-e, az a terv 10. fejezetének DONE-feltételétől függ (valós referenciaminta, külső validátor), ez nincs igazolva |

## 3. Munkacsomagonkénti gap matrix

Állapot: **E** = Existing (megvan), **P** = Partial, **M** = Missing, **C** = Conflict.
Gazda: melyik jóváhagyott terv vagy issue viszi ma.

| WP | Tárgy | Állapot | Bizonyíték a `main`-en | Hiányzik | Gazda |
|---|---|---|---|---|---|
| WP00 | audit | E | ez a dokumentum | – | CineCert |
| WP01-A | orákulum-környezet | P | [`scripts/dev/setup-dcp-inspect.sh`](../../scripts/dev/setup-dcp-inspect.sh); `CFA4_ASDCPLIB_PREFIX` ([`dcp-validation.md`](../development/dcp-validation.md)); `CFA4_PHOTON_CP` (`crates/export/src/imf_tests.rs`) | asdcplib CLI build (DCP-01 L4 nyitott), Photon és Clairmeta telepítő, DCP/IMF korpusz-letöltő | CineCert (asdcplib: DCP-VALIDATION-tel egyeztetni) |
| WP02-A | MXF bővítés | P | `crates/mxf/README.md`: ST 422 (2022) író és olvasó, ST 429-3/-4 (2023) író, ST 429-5 olvasó, ST 429-6 olvasó (AES-128-CBC, HMAC-SHA1 MIC), ST 429-18, ST 2067-2/-5 író, ST 377-4 MCA író | ST 429-10 sztereó és ST 422 mezőpárok (#102), ST 429-5 timed text **írás**, ST 429-6 titkosított **írás** | PROCODEC-002 (G10 #102) |
| WP03 | DCP ingest/inspect | P | `crates/dcp` (ST 429-7/-8/-9/-10/-16, Interop PROTO-ASDCP), `crates/dcp-validate`, `crates/dcp-inspect`; nyitott: #1109 (DCP-06) | DCP → idővonal import (#101) | DCP-VALIDATION; #101: G10 |
| WP04 | DCP mastering | P | `crates/export/src/dcp_out.rs`, `dcp_iab.rs`, `dcp_atmos.rs`: SMPTE, 2K/4K Flat/Scope/Full, 5.1 24 bit, IAB (ST 429-18), RDD 52, MCA, ST 429-16, DCNC mappanév; [`file-format-parity.md`](../file-format-parity.md) DCP sor | Interop írás, több reel, 3D, felirat a DCP-ben, 7.1 főhang, injektálható UUID/IssueDate | PROCODEC-002 5a |
| WP05 | IMF ingest | M | – | CPL → idővonal, supplemental package | PROCODEC-002 5b |
| WP06 | IMF mastering | P | `crates/export/src/imf_out.rs` (#1011): App 2E, egy szegmens, CPL/PKL/ASSETMAP/VOLINDEX, RegXML descriptorok | OPL (ST 2067-100), supplemental/verziózás, IAB az IMF-ben, aláírás | PROCODEC-002 5b |
| WP07 | J2K és szín | P | `crates/j2k`: DCI 2K/4K, IMF profilok (`ImfProfile::select`, #931); #116 lezárva; nyitott: #1113 (HTJ2K kódolás), #1126 | #117 (rétegek, csempék), mért valós idejű 2K dekódolás | PROCODEC-002 4 |
| WP08 | MXF digest, IMF-VTFP | M | nincs találat (`vtfp`, `mxf-digest`) | teljes | **CineCert** |
| WP09 | metaadat, ISDCF | P | DCNC mappanév az exportban; `dcp_inspect` `naming` → `DCP.NAME.001` | ISDCF registry snapshot, natív névépítő/-ellenőrző | **CineCert** (a natív szabály: DCP-VALIDATION-nel egyeztetni) |
| WP10 | titkosítás, aláírás, KDM | P / C | `crates/dcp/src/kdm.rs` (ST 430-1/-3 nyitás, RSA-OAEP); `crates/mxf/src/crypt.rs` (visszafejtés); nyitott: #1076 (KDM-aláírás), #1084, #1086 | titkosított írás, KDM-kiadás, XML-aláírás készítése | G2 (olvasás); írás: senki, D3 és PROCODEC-002 ellentétes (2. pont) |
| WP11 | QC-motor | P | DCP: `cfa4-dcp-validate` 8 profil, `cfa4-dcp-report/1`; IMF: Photon-orákulum a tesztekben | natív IMF QC | DCP: DCP-VALIDATION; IMF: PROCODEC-002 5b |
| WP12 | Qt6 / CLI | P | Qt IAB DCP export (#343); KDM-párbeszéd PR-ok | DCP/IMF inspect és QC panel | több projekt; D4 |
| WP13 | átadás, archiválás | M | `VOLINDEX` egy kötetre íródik (`dcp_out.rs`, `imf_out.rs`) | ISDCF meghajtó, több kötet, provenance | **CineCert** |
| WP14 | kiadás, CI | – | a repó egészének csomagolása és CI-je (`AGENTS.md`) | nincs külön CineCert-teendő | repó |
| WP15 | feliratok | P | `crates/captions/src/dcst.rs` (ST 428-7 2007/2010/2014 olvasás), TTML/IMSC1, ST 429-5 olvasás | ST 428-7 írás, Interop CineCanvas, CCAP (ST 429-12), IMSC1 Image | PROCODEC-002 |
| WP16 | IMF App 2E / HDR / HTJ2K | P | HTJ2K dekódolás; App 2E 10 bit BT.709 export; #1113 | PQ/HLG, ST 2086, MaxCLL/MaxFALL, SCM (ST 2067-9) | PROCODEC-002 4/5b |
| WP17 | Auro-3D őr | kapu | `crates/auro` létezik (dekódolás, #299) | indítási feltétel: valódi Auro DCP minta + jogi ellenőrzés | CineCert (M2 után) |

## 4. Issue-hozzárendelés

| Issue | Állapot (2026-11-13) | WP |
|---|---|---|
| #77 JPEG 2000 MXF/DCP import | lezárva | WP02-A, WP03 |
| #81 JPEG 2000 kódoló (DCP-export) | nyitott (a kódoló kész; a lezárás a gazdáé) | WP07 |
| #101 DCP-csomag importja idővonalra | nyitott | WP03 |
| #102 ST 422 mezőpárok, sztereó DCP (ST 429-10) | nyitott | WP02-A, WP04 |
| #115 J2K bekötése DCP / MXF exportba | lezárva | WP04 |
| #116 J2K kódoló gyorsítása | lezárva | WP07 |
| #117 rétegek, csempék, HTJ2K (IMF) | nyitott | WP07, WP16 |
| #309 MCA-címkék, ST 429-16, RDD 52 | nyitott (a `main` már írja őket) | WP04 |
| #1011 IMF App 2E, IMP csomag | nyitott (`imf_out.rs` a `main`-en) | WP06 |

Új issue csak a CineCert gazdájú részekhez nyílik, a tulajdonos döntése után.

## 5. Szabványmátrix: a használt kiadások

A terv 12. fejezete szerint a kiadásévet a WP00 rögzíti. Ahol a `main` kódja már egy kiadásra épül,
az a crate README-jéből származik; a többi a megvalósító PR-ban rögzítendő.

| Szabvány | Kiadás a `main`-en | Forrás |
|---|---|---|
| ST 336 | 2017 | `crates/mxf/README.md` |
| ST 377-1 | 2011 (+Amd 1:2012), 2019 (G. melléklet) | `crates/mxf/README.md` |
| ST 379-1 / -2 | 2009 / 2010 | `crates/mxf/README.md` |
| ST 422 | 2022 | `crates/mxf/README.md` |
| ST 429-3 / -4 / -5 / -6 / -18 | 2023 | `crates/mxf/README.md` |
| ST 429-7 | 2006 | `crates/dcp/README.md` |
| ST 429-8 | 2007 | `crates/dcp/README.md` |
| ST 429-9 | 2014 | `crates/dcp/README.md` |
| ST 429-10 | 2008 (csak CPL-olvasás) | `crates/dcp/README.md` |
| ST 429-16 | 2014 | `crates/dcp/README.md` |
| ST 430-1 | 2023 | `crates/dcp/README.md` |
| ST 2067-2 / -5 | 2020 | `crates/mxf/README.md` |
| ST 2067-21 | 2026 (5. kiadás) | `crates/mxf/README.md`, `imf_out.rs` |
| ST 2067-201 | 2019 | `crates/mxf/README.md` |
| ITU-T T.800 / T.814 | szabadon elérhető ITU-T szöveg | `crates/j2k/README.md` |
| ST 377-4 | nincs kiadás rögzítve (asdcplib szótár és DCP-o-matic fájlok alapján) | `crates/mxf/README.md`: rögzítendő |
| ST 428-7 | 2007 / 2010 / 2014 névterek | `crates/captions/src/dcst.rs` |
| ST 2067-3 / -8 / -100 / -101 / -102, ST 2086, ST 2067-9, ST 429-12, RDD 52, ST 430-2 / -3 | nincs rögzítve | a megvalósító PR-ban |

## 6. Javasolt következő lépés

A tulajdonos és a gazda dönt (§2.8). A CineCert-munkamenet javaslata: a CineCert-terv a 3. pontban
„CineCert” gazdájú részeken folytatódik (WP01-A, WP08, WP09, WP13, elfogadási tesztek), a többi
munkacsomagnál a terv a gazda projektjének ellenőrzőlistája marad, kód nem készül rájuk ebből a
projektből. A D2 és D3 eltérést (2. pont) a tulajdonos egy helyen rögzíti.
