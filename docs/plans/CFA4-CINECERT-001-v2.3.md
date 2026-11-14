# CFA4 – CineCert Digital Cinema & IMF Mastering (v2, elemzéssel kiegészítve)

**Azonosító:** CFA4-PLAN-CINECERT-001 · **Verzió:** v2.3 (elemzés + kiegészítések, a v2.1/v2.2 kiegészítések jóváhagyva)  
**Dátum:** 2026-10-27 · utolsó módosítás: 2026-11-09  
**Állapot:** ✅ jóváhagyott implementációs terv (István, 2026-11-13); a fejlesztés a 15. fejezet PR-sorrendjével indul  
**Célrepository:** https://github.com/ubuntuokos/CinemaFundArchitecture4  
**Ellenőrzött forrásállapot:** `main` @ `16debf92d85430d7bbf43687c35ab1b043b2a9ca` (2026-10-27 16:29 +0100), csak olvasással

> **Repó-megjegyzés (2026-11-13):** a tulajdonos által jóváhagyott v2.3 szöveg változatlanul, hogy a
> `VISION.md` 2.6. pontja szerint a `main` ágon éljen (a munkapéldány a projekt megosztott mappájában
> van; az eltéréseket ide kell visszavezetni). Az upstream projekt nevét az `AGENTS.md` márkaszabálya
> miatt (`python/tests/test_independence.py`) az „upstream” szó helyettesíti.
> **A terv a 2026-10-27-i `main`-re épül; azóta a DCP/IMF munka nagy része más projektekben
> elkészült vagy folyamatban van.** Az aktuális állapot, az átfedések és a PR-sorrend hatása:
> [CINECERT-WP00 gap matrix](../audit/CINECERT-WP00-GAP-MATRIX-2026-11-13.md). Ahol a kettő eltér,
> a gap matrix a tényállás; a munka tovább csak az ott „CineCert” gazdájúként jelölt részeken
> folyik, a többiről a tulajdonos és a gazda dönt (`docs/coordination.md` §2.8).

> **Jelölések ebben a változatban**
> - 🆕 **[ÚJ]** – új tartalom, az eredeti tervben nem szerepelt
> - ✏️ **[MÓD]** – az eredeti állítás vagy döntés javítva/módosítva, indoklással
> - ⚠️ **[ELLENTMONDÁS]** – a terv ütközik a repóban rögzített szabállyal vagy ténnyel
> - Jelölés nélküli szöveg: az eredeti terv változatlanul
>
> A szabványkiadások (évszámok) és a külső projektek licencei a végrehajtás előtt forrásból újraellenőrizendők; ahol ez a dokumentumban „ellenőrizendő” jelzéssel áll, ott nem bizonyított tényről van szó.

---

## 0. 🆕 [ÚJ] Elemzési összefoglaló

### 0.1 Ami a repóban már megvan (és a terv nem tud róla)

Az eredeti terv 2. fejezete szerint „nem jelent meg DCP/IMF/MXF eredmény”. A `main` tényleges tartalma ennél sokkal több, és ez a teljes tervet átrendezi:

| Komponens (valós crate) | Mit tud ma | Bizonyíték |
|---|---|---|
| `crates/mxf` (`cfa4-mxf`, L0) | clean-room MXF demux (ST 377-1, OP1a, OP-Atom, generic container, index táblák, RIP) és **író** (OP1a/OP-Atom: VC-3, ProRes, AVC, PCM, timecode, streaming `Write + Seek`); IAB track file olvasás (ST 2067-201) | `crates/mxf/src/lib.rs`, `crates/mxf/README.md` |
| `crates/mxf` – hiányok | JPEG 2000 MXF írás (ST 422), titkosított essence (ST 429-6) **kifejezetten nem támogatott**, OP1b/OP2x+, deszkriptív metaadat | `crates/mxf/README.md` 76–78. sor |
| `crates/j2k` (`cfa4-j2k`) | clean-room JPEG 2000 Part 1 dekóder + **HTJ2K (Part 15) dekóder** (OpenJPH-portból, regisztrálva), Part 1 enkóder **DCI 2K/4K cinema profillal** (`EncodeOptions::cinema_2k/4k`, Rsiz 3/4, 12-bit X'Y'Z', képkocka-bájtlimit) | `crates/j2k/README.md`, `docs/licensing.md` 138. sor |
| `crates/j2k` – hiányok | több minőségi réteg, tile-ok, **HTJ2K kódolás**, SOP/EPH/PLT, **IMF és broadcast profilok** (#117), DCP/MXF export bekötése (#115), sebesség (#116) | `crates/j2k/README.md` 88–89. sor |
| `crates/iab` (`cfa4-iab`, L0) | IAB (ST 2098-2) parser, DLC dekóder/enkóder, renderer (stereo…7.1.4), író – a DCP (ST 429-18) és IMF (ST 2067-201) objektumhang-formátuma | `crates/iab/src/lib.rs` |
| `crates/captions` | TTML **IMSC1 Text** profil olvasás/írás, SRT, VTT, SCC, MCC, EBU STL; égetett felirat | `crates/captions/src/lib.rs` |
| `crates/color` | színtér/transzfer/gamut/LUT; ACES/OCIO csak tesztorákulumként tervezett (ADR-0003 A1/A2) | `crates/color`, `docs/adr/ADR-0003-…md` |
| `crates/export` | export pipeline, `mxf_out.rs`, preset-rendszer, determinisztikus teszt | `crates/export/src` |
| Workspace függőségek | `sha2`, `uuid`, `quick-xml`, `roxmltree`, `ring`, `rustls` már a lockfile-ban | `Cargo.lock` |
| Meglévő tervdokumentum | `docs/cinema-codecs.md`: JPEG 2000 → DCP csomagolás 20–30 óra a J2K után, **KDM/titkosítás kifejezetten hatókörön kívül** | `docs/cinema-codecs.md` |
| Hiánylista | `docs/gaps.md`: JPEG 2000 MXF/DCP import („a dekóder megvan, nincs bekötve”), DCP és J2K MXF export (#81, #115) | `docs/gaps.md` 202–245. sor |
| `docs/file-format-parity.md` | DCP: 0% | 47. sor |

**Következmény:** a terv „C++ könyvtár + C ABI + Rust wrapper” gerince a CFA4-ben a meglévő tiszta Rust MXF/J2K/IAB/IMSC1 rétegre épülő **bővítésként** valósítható meg, nem új natív alrendszerként. Az eredeti architektúra egy részét (WP01–WP02) ezért át kell gondolni, lásd 0.3.

### 0.2 ⚠️ [ELLENTMONDÁS] Ütközések a repó kötelező szabályaival

| # | Terv állítása | Repó szabálya | Hatás |
|---|---|---|---|
| E1 | `vendor/asdcplib` C++ library + C ABI + Rust FFI (WP01–02) | `AGENTS.md` §0.3: `unsafe_code = "forbid"` a workspace-re, egyetlen kivétel a `crates/platform` (ADR 0001); `CLAUDE.md`: „Pure Rust in the product”, kivétel csak FFmpeg/GStreamer (ADR 0005) | Új FFI-crate **csak új ADR-rel** lehetséges. Az ADR-0003 A2 már előirányoz egy „ADR 0006: FFI-kivétel elve” döntést – ehhez kell csatlakozni, nem megkerülni. |
| E2 | OpenJPEG beépítése DCP J2K-hoz | `AGENTS.md` §2: kodekek clean-room Rust-ban, publikus specifikációból; `cfa4-j2k` már tud DCI 2K/4K kódolást; az OpenJPEG/Grok/OpenJPH a repóban **csak tesztorákulum** | Az OpenJPEG termékbe emelése duplikálná a meglévő kodeket és szabályt sértene. Javaslat: OpenJPEG marad orákulum. |
| E3 | „Qt6 GUI / PySide6 shell → Rust application service API” | ADR F8: Qt6 → **lokális `cfa4-cli --bridge` (127.0.0.1)** → Rust engine; az egui a működő alkalmazás, a Qt shell prototípus | A GUI-integráció a bridge parancskészletén keresztül történik; nincs külön „service API” réteg. Minden funkció parancs + agent/MCP-vezérelhető (`CLAUDE.md`). |
| E4 | Új crate-ek (`cfa4-dcp`, `cfa4-imf`, `cfa4-crypto`…) szabad elhelyezéssel | `cargo xtask layers`: minden új crate-et fel kell venni a `LAYERS` táblába; L0 crate csak `cfa4-bitstream`-től függhet és wasm-ra is fordul | FFI-t tartalmazó crate nem lehet L0; az XML/csomag-logika L1–L2. Lásd 4.1. |
| E5 | Referenciaminták CI-ben | `AGENTS.md` §2 vége: nagy bináris fixture tilos a repóban; pinelt commit + sha256 (`cargo xtask corpus` minta) | DCP/IMF fixture-ök külső korpuszban, lásd 13. fejezet. |
| E6 | Egy nagy CineCert-munka | `docs/coordination.md` + `AGENTS.md` fejléc: egy téma / PR, friss `main`-ről, nem építhet másik nyitott PR-re, csak a merge-koordinátor merge-el; teljes CI csak `cfa4-full-validation` címkével | A munkacsomagokat kis, független PR-ekre kell bontani (15. fejezet). |
| E7 | WP10 KDM/titkosítás P-szintű cél | `docs/cinema-codecs.md`: „encryption (KDM) out of scope” | Tulajdonosi döntés kell a hatókör bővítéséről (16. fejezet, D3). |
| E8 | WP14: Ubuntu 26.04 amd64 csomagok | `docs/releasing.md` még macOS/Windows/aarch64/FreeBSD célokat és `ubuntu-22.04` runnert ír, miközben `AGENTS.md` szerint egyetlen cél a Linux x86_64 | A release-dokumentáció elavult; a WP14-et az `AGENTS.md`-hez kell igazítani, és a `releasing.md` frissítését külön PR-ben jelezni. |

### 0.3 ✏️ [MÓD] Javasolt stratégiai irány

**Ajánlott (A változat):** clean-room Rust bővítés a meglévő crate-ekre, a CineCert-eszközök **külső tesztorákulumként**.
- `cfa4-mxf` bővítése: ST 422 JPEG 2000 mapping (írás), ST 429-3/-4 DCP track file-ok, ST 429-5 timed text track, ST 429-10 sztereó, AS-02 / ST 2067-5 IMF essence; később ST 429-6 titkosított essence.
- Új L1/L2 crate a csomag-XML-hez (CPL/PKL/ASSETMAP/VOLINDEX, IMF CPL/OPL) – a meglévő `quick-xml`/`roxmltree` és `uuid`/`sha2` függőségekkel.
- `asdcplib` (`asdcp-test`, `asdcp-info`, `as-02-unwrap` stb.), OpenJPEG, `mxf-digest`, `imf-vtfp` referencia-implementációk: **külső folyamatként futó orákulumok** a tesztekben, ahogy ma az ffmpeg/OpenJPEG/Grok.
- Előny: nincs új `unsafe`, nincs ADR-kivétel, wasm/layers gate marad, egységes „never crash” garancia.
- Hátrány: több saját kód; a megfelelőséget független validátorokkal kell bizonyítani (ez a terv szerint egyébként is kötelező).

**B változat (csak tulajdonosi döntéssel):** `asdcplib` FFI egy izolált `crates/cinema-ffi` crate-ben az ADR 0001 feltételeivel, az ADR-0003 A2 által előirányzott „ADR 0006 – FFI-kivétel elve” keretében. Ekkor is: opcionális Cargo feature, Rust útvonal a tartalék, nincs FFI-típus a publikus API-ban, `// SAFETY:` minden blokkon.

✅ **D1 eldöntve (2026-10-27): A változat.** A dokumentum az A változatot tekinti alapnak, és jelzi, hol változna B esetén.

### 0.4 Legfontosabb hiányok röviden

1. **Szabványmátrix hiányos** – nincs felsorolva többek közt ST 428-7 (DCP felirat), ST 429-12 (caption), ST 429-16 (CPL metadata), ST 429-18 (IAB DCP), SMPTE RDD 52 (DCP Bv2.1 alkalmazási profil), ST 2067-2/-3/-8/-9/-100/-101/-102, IMF App 2E (ST 2067-21), App 4 (ST 2067-40), App 5 ACES (ST 2067-50). → 12. fejezet
2. **Feliratok és akadálymentesítés** nincs külön munkacsomagban: Interop CineCanvas vs SMPTE ST 428-7, font-subset, PNG-feliratok, zárt felirat (CCAP), HI/VI-N hangsávok, ISDCF jelnyelvi videó, IMF IMSC1 Text/Image. → 🆕 WP15
3. **Hang/MCA**: ST 377-4 MCA címkék, csatornakiosztás (5.1, 7.1 DS/SDDS, HI/VI-N a 7–8. csatornán), 48/96 kHz, Leq(m) trailer-szint; az IAB már megvan, de a terv nem említi; a Dolby Atmos saját bitfolyama (aux data, ST 429-14) licenchez kötött. → WP04/WP06 kiegészítés
4. **IMF App 2E / HDR**: HTJ2K kódolás (a dekóder már kész), Rec.2020, PQ/HLG, ST 2086 / MaxCLL/MaxFALL metaadat, Dolby Vision sidecar (SCM, ST 2067-9). → 🆕 WP16
5. **DCP-specifikus kriptográfia** pontatlan: a PKL hash **SHA-1 (base64)**, az XML-aláírás XML-DSig + C14N, a KDM RSA-OAEP + AES-128, a titkosított MXF AES-128-CBC + HMAC-SHA1 (MIC). A terv csak SHA-256/512-ről beszél. → WP08/WP10 kiegészítés
6. **Külső validátorok nincsenek megnevezve**: Clairmeta (DCP/IMF), Netflix Photon (IMF), DCP-o-matic `dcpverify` (libdcp), `asdcp-info`. → 13. fejezet
7. **Fixture-források**, **ráfordítás-becslés**, **felelősök/PR-bontás**, **issue-hozzárendelés** (#81, #115, #116, #117) hiányoznak. → 13–15. fejezet
8. **Átadási adathordozó**: ISDCF szerinti ext2/ext3 (128 bájtos inode) DCP-meghajtó, VOLINDEX több kötetre. → WP13 kiegészítés
9. **Determinizmus vs. szabvány**: IssueDate, UUID v4 – csak injektált óra/UUID-forrással lehet bitazonos teszt. → WP04/WP11 kiegészítés
10. Apró hibák: WP03-ban idegen karakterek („fájlok解析”), WP03 gate „import/export”-ot ír ingest-csomagnál, 8.6. elfogadási teszt („4 azonos virtuális sáv”) pontosítandó, a CTL upstream licence (AMPAS-licenc) GPL-kompatibilitása nincs ellenőrizve, a `mxf-digest` és `imf-vtfp` licence nincs rögzítve.

---

## 1. Cél és határok

A CFA4 upstreamből származó Rust médiamotorját professzionális DCP- és IMF-mastering, csomagellenőrzés és átadási képességekkel egészítjük ki. A CineCert forrásaiból csak a megfelelőségi, licencelési, karbantarthatósági és integrációs auditon átment komponensek kerülnek be. A hiányzó funkciókat saját CFA4 Rust implementáció pótolja. Nem épül párhuzamos videoszerkesztő vagy FFmpeg/GStreamer router. Csak Ubuntu 26.04 és variánsai, amd64, Qt6, Rust/Cargo, Python venv, C/C++ natív függőségek; nincs conda/mamba. A munka kizárólag a CFA4 repositoryban történhet; az upstream változatlan. A társrendszerekkel való ütközésmentesség kötelező.

✏️ **[MÓD]** Pontosítások:
- „C/C++ natív függőségek” → az A változatban a termékben **nincs új C/C++ függőség**; C/C++ eszközök csak tesztorákulumként, külső folyamatként futnak. B változatban kizárólag ADR 0006 alapján.
- „Nem épül párhuzamos … router” → az új munka is a `cfa4-route` (ADR 0004) és az ADR 0005 szabályait követi; a J2K kódolási útvonalak (CPU, később GPU) a routerben regisztrálódnak.
- 🆕 **Hatókörön kívül (explicit):** DCI tanúsítás (a DCI CTP lejátszó-/szerver-eszközökre vonatkozik, nem tartalomkészítő szoftverre), Dolby Atmos saját bitfolyam írása, DRM-megkerülés, harmadik fél KDM-jeinek feltörése, valós mozis tanúsítványok kezelése a tulajdonos D3 döntése nélkül.

## 2. Bizonyított kiindulási tények és ismeretlenek

A CFA4 aktuális README (`main`, 2026-10-27) a következőket jelzi: GPL-3.0-or-later; Rust 1.97.1 pin; örökölt Rust engine; kísérleti PySide6/Qt6 shell; FFmpeg >=9.0.2 és GStreamer mint backend; Linux x86_64 release még nem kész. ~~A GitHub szöveges keresésben nem jelent meg DCP/IMF/MXF eredmény~~, de ez nem helyettesíti a teljes repo- és ágfelmérést. A végrehajtás előtt az összes érintett ág és nyitott PR, Cargo workspace, média-export, Qt6 modell, backend-router, csomagoló és teszt-gate tényleges ellenőrzése kötelező. A forrásállapothoz commit SHA-t kell rögzíteni.

✏️ **[MÓD]** A keresési eredmény téves volt: a repóban jelentős MXF/J2K/IAB/IMSC1 alap van (0.1). Rögzített forrásállapot: `16debf92d85430d7bbf43687c35ab1b043b2a9ca`.

🆕 **[ÚJ] Még ismeretlen (ebben az elemzésben nem volt ellenőrizhető):**
- a nyitott PR-ek és ágak listája (a GitHub API ebből a munkamenetből nem volt elérhető) – a WP00 első lépése;
- a #81, #115, #116, #117 issue-k aktuális állapota;
- a `cfa4-j2k` cinema enkóder mért sebessége (a `j2k_encode_bench` példa létezik, eredményt nem rögzít a README);
- `docs/coordination-log.md` szerinti folyamatban lévő párhuzamos munkák, amelyek a `crates/mxf`-et vagy `crates/export`-ot érintik.

## 3. Források, befoglalási döntések

| Forrás | Használat | Stratégia | Státusz |
|---|---|---|---|
| https://github.com/cinecert/asdcplib | AS-DCP ST 429 / IMF AS-02 MXF és essence | ✏️ **A:** külső tesztorákulum (`asdcp-test`, `asdcp-info`, `as-02-*` CLI) és viselkedési referencia; **B:** verziópinelt, izolált C++ library + C ABI/Rust wrapper ADR 0006 alapján | ✏️ P0 orákulum (A) / P0 jelölt (B) |
| https://github.com/cinecert/openjpeg | JPEG 2000, DCP profilok | ✏️ csak tesztorákulum (a `cfa4-j2k` már tud DCI 2K/4K-t); upstream https://github.com/uclouvain/openjpeg, CineCert fork csak diff-audit után, orákulumként | ✏️ P1 orákulum |
| https://github.com/cinecert/mxf-digest | KLV-alapú SHA-512 digest | interoperábilis saját Rust implementáció tesztvektorokkal; 🆕 licenc rögzítendő | P1 jelölt |
| https://github.com/cinecert/imf-vtfp | IMF virtual-track fingerprint | dokumentum/tesztek alapján saját Rust implementáció; 🆕 licenc és a specifikáció stabilitása rögzítendő | P1 jelölt |
| https://github.com/cinecert/registries | ISDCF metaadat JSON | preferált hivatalos https://github.com/ISDCF/registries forrás; verziózott offline snapshot; 🆕 adatlicenc rögzítendő | P1 jelölt |
| https://github.com/cinecert/CTL | színtranszformáció | preferált https://github.com/aces-aswf/CTL upstream, opcionális adapter meglévő OCIO/ACES mellett; 🆕 ⚠️ az AMPAS-licenc GPL-3 kompatibilitása és `deny.toml` szerinti elfogadhatósága ellenőrizendő; illeszteni kell az ADR-0003 A1/A2 ütemezéshez | P2 jelölt |
| https://github.com/cinecert/fltk | FLTK GUI | nem használjuk, Qt6 marad | kizárt |
| asdcplib libphdr | elavult/experimental PHDR | alaptelepítésből kizárva; későbbi elkülönített kutatási döntés | nem alapkövetelmény |
| 🆕 Clairmeta (Eclair) | DCP/IMF ellenőrző (Python) | külső validátor (külön venv, külső folyamat) | P0 validátor |
| 🆕 Netflix Photon | IMF ellenőrző (Java, Apache-2.0) | külső validátor, IMF App 2/2E | P0 validátor (IMF) |
| 🆕 DCP-o-matic `dcpverify` / libdcp | DCP ellenőrző | csak külső folyamat; kód nem vehető át, amíg a fájlfejlécek licence nincs ellenőrizve (GPL-2.0-only tiltott) | P1 validátor |
| 🆕 SMPTE nyilvános XSD-k / regiszterek | CPL/PKL/AssetMap sémák, UL-ek (RP 210/224) | séma-ellenőrzés tesztben; terjesztési feltétel ellenőrizendő | P0 |
| 🆕 ISDCF dokumentumok (Doc 2 – Digital Cinema Naming, DCP-meghajtó ajánlás, jelnyelvi videó) | névadás, átadás, akadálymentesség | szabályforrás a QC-hez (nem kód) | P1 |
| 🆕 https://github.com/cinema-ONE/auro3d-detector (+ https://github.com/almirus/Orua-D3) | Auro-3D jelenlét és layout felismerése PCM LSB-ből (Python + ffmpeg, MIT) | **csak tesztorákulum és módszerleírás** (WP17); kód nem kerül a termékbe; ⚠️ a layout-táblák eredete (visszafejtett gyári dekóder) és az US8620465 szabadalom státusza jogilag ellenőrizendő | P2 orákulum (opcionális) |
| 🆕 https://github.com/tmeiczin/opendcp | DCP-készítő (C, Qt4 GUI, CLI: J2K, MXF, XML, XML-DSig), GPL-3.0-or-later | **csak legacy-fixture generátor** (WP03 Interop/régi SMPTE ingest-tesztekhez, D2); 2016 óta gyakorlatilag nem fejlesztik, elavult asdcplib 2.2.7 / OpenSSL 0.9.8 / OpenJPEG 1.x / Qt4 a forrásfában, Bv2.1/MCA nincs → validátornak és viselkedési referenciának **nem** alkalmas; kód nem vehető át (clean-room) | P3 fixture-forrás (opcionális) |

Megőrzendő a fájlonkénti jogi értesítés; külön ellenőrizendő asdcplib AS-02 attribution, OpenJPEG/CTL és az összes transzitív C/C++ library. Nincs automatikus hardveres vagy Dolby/DCP megfelelőségi tanúsítás.

🆕 Minden átvett vagy portolt kódrészlet a `docs/licensing.md` §5 reused-code regiszterébe kerül ugyanabban a commitban; minden Rust függőségnek át kell mennie a `cargo deny check licenses` ellenőrzésen.

## 4. Tervezett rendszerarchitektúra

```text
Qt6 GUI / PySide6 shell --> Rust application service API <-- cfa4-cli
                               |
                    cfa4-cinema orchestration
                               |
           +-------------------+-------------------+
           |                   |                   |
     cfa4-dcp             cfa4-imf           cfa4-cinema-qc
           |                   |                   |
     cfa4-mxf / cfa4-crypto / cfa4-cinema-metadata / cfa4-isdcf
           |                   |                   |
     isolated C ABI       Rust parsers        rules+reports
           |
  vendor/asdcplib + OpenJPEG upstream (+ optional CTL adapter)
           |
 existing CFA4 media-router / FFmpeg 9.x / GStreamer / color pipeline
```

Ezek logikai komponensnevek, nem ellenőrzötten létező mappák. Valós crate-, könyvtár- és UI-névtér kijelölése a meglévő struktúra auditja után, azzal kompatibilisen történik.

A C++ API-t ne közvetlenül exponáljuk a GUI vagy Python felé: szűk, verziózott C ABI; Rust safe wrapper; ellenőrzött ownership/élettartam; UTF-8, hibakód, hibaüzenet; pánik- és kivételhatár; streamelt I/O. Érzékeny titkos kulcs sosem kerül Qt-modellekbe vagy logba.

### 4.1 🆕 [ÚJ] A valós repóstruktúrához igazított architektúra (A változat)

```text
egui (működő app)  /  Qt6 Widgets + PySide6 (prototípus, ADR F8)
        |                         |
        |              cfa4-cli --bridge (127.0.0.1, parancsok)
        +------------+------------+
                     |
       cfa4-engine / parancsok / MCP (minden funkció parancs)
                     |
   L2  cfa4-cinema (új): DCP/IMF composer, ingest, QC-szabályok, riportok
                     |
   L1  cfa4-cinema-pkg (új): CPL/PKL/ASSETMAP/VOLINDEX, IMF CPL/OPL/SCM XML,
       UUID/IssueDate injektálás, hash (SHA-1 PKL, SHA-512 mxf-digest), ISDCF registry snapshot
       cfa4-cinema-crypto (új, D3 után): AES/HMAC/RSA/X.509/XML-DSig – RustCrypto, pure Rust
                     |
   L0  cfa4-mxf (bővítés: ST 422 J2K, ST 429-x, AS-02/2067-5)  cfa4-j2k (bővítés: profilok, HTJ2K enc)
       cfa4-iab (meglévő)   cfa4-captions (bővítés: ST 428-7, Interop, IMSC1 Image)
                     |
   cfa4-route (ADR 0004) → saját CPU-útvonal → (később) GPU J2K → FFmpeg/GStreamer csak dekódolásra
   cfa4-color (XYZ, DCI-P3, gamma 2.6; OCIO csak ADR-0003 szerint)

Tesztorákulumok (nem termék): asdcplib CLI, OpenJPEG, Clairmeta, Photon, dcpverify, mxf-digest, imf-vtfp
```

- A crate-nevek javaslatok; a `xtask/src/main.rs` `LAYERS` és `SAME_LAYER` táblájába felveendők.
- B változatban egy további `crates/cinema-ffi` (L5, `unsafe_code = "deny"`, ADR 0001 feltételei) kerülne a `cfa4-cinema` alá, opcionális feature-rel; a Rust útvonal tartalék marad.
- A titkos kulcsok kezelése a `cfa4-cinema-crypto`-ban marad; a bridge és az MCP soha nem ad vissza kulcsanyagot.

## 5. Részletes munkacsomagok

### WP00 – Repo-, dokumentáció- és hiány-audit
- main és összes releváns fejlesztési ág, nyitott PR, tervezett Qt6 átállás és media-router felmérése.
- Existing/Partial/Missing/Conflict státusz minden, lent felsorolt komponensre; pontos kódhely és tesztbizonyíték.
- A használt SMPTE/ISDCF/DCI szabványok kiadásának és hozzáférhetőségének rögzítése (ST 429, ST 430, ST 2067, ST 377-1, ST 336, releváns JPEG 2000/DCP profilok és alkalmazási kötöttségek).
- Verziópin és SBOM, licenceszerződés- és függőséglista; aktuális release-kompatibilitás.
- 🆕 Kiindulási gap matrix a 0.1 táblázatból; összevetés a `docs/cinema-codecs.md`, `docs/gaps.md`, `docs/file-format-parity.md` sorokkal, és ezek frissítése a terv elfogadásakor (a repó szabálya szerint minden doksi „Last updated” sort kap).
- 🆕 Issue-hozzárendelés: #81 (J2K enkóder), #115 (DCP/MXF export), #116 (J2K sebesség), #117 (J2K profilok); új issue-k a hiányzó WP-khez.
- 🆕 A teljes szabványmátrix (12. fejezet) kiadásévvel és beszerzési móddal (SMPTE előfizetés / nyilvános).
- 🆕 Tulajdonosi döntések rögzítése: D1–D5 (16. fejezet).
**Gate:** jóváhagyható gap matrix, nem módosítunk ismeretlen meglévő funkciót.

### WP01 – Natív dependency build és beágyazás
✏️ **[MÓD] A változatban átnevezve: „WP01 – Tesztorákulum-környezet”.**
- asdcplib külön forrásverzió pin, CMake/autotools Ubuntu 26.04 amd64 reprodukálható build – 🆕 A változatban csak a teszteszközök (`asdcp-test`, `asdcp-info`, `as-02-wrap`, `as-02-unwrap`) számára, `target/`-on belüli vagy `/opt/cfa4-test/` prefixbe, a termékcsomagba nem kerül.
- AS-02 külön build-kapcsoló; a szükségtelen libphdr ki.
- OpenJPEG upstream és CineCert fork diff; csak egy provider legyen aktív, diszkrét build-prefix.
- Nem írjuk felül `/usr/lib`, `/usr/include`, rendszer-OpenJPEG, rendszer-FFmpeg vagy más alkalmazás könyvtárait.
- ASAN/UBSAN ahol lehetséges, ABI smoke, static/dynamic linking jogi ellenőrzés, SBOM és NOTICE. (B változat)
- 🆕 Clairmeta külön Python venv-ben, Photon pinelt JAR-ként (OpenJDK az Ubuntu tárolóból); mindegyik verziója a riportba kerül.
- 🆕 Hiányzó orákulumnál a teszt **kihagyódik és ezt jelzi**, nem bukik csendben és nem lesz PASS (a repó meglévő orákulum-mintája szerint).
**Gate:** clean-room build, runtime linkage, release packaging függőségek PASS. 🆕 (A: orákulumok egy paranccsal telepíthetők és verziójuk rögzített; termékcsomagban nincs új natív függőség.)

### WP02 – C ABI és Rust FFI
✏️ **[MÓD] Csak B változatban.** A változatban helyette: **WP02-A – MXF bővítés a `cfa4-mxf`-ben:**
- 🆕 ST 422 JPEG 2000 frame-wrapped mapping (olvasás bekötése a `cfa4-codecs`-be, írás), RGBA/CDCI picture descriptor, J2K sub-descriptor.
- 🆕 ST 429-3 (DCP track file-ok, OP-Atom kötöttségek), ST 429-4 (J2K DCP), ST 429-5 (timed text track file), ST 429-10 (sztereó, „left/right” frame-ek), ST 429-18 (IAB DCP).
- 🆕 AS-02 / ST 2067-5 IMF essence component (clip-wrapped PCM, frame-wrapped J2K/HTJ2K), ST 2067-201 IAB (olvasás megvan, írás bővítendő).
- 🆕 ST 377-4 MCA címkék írása/olvasása (hangcsatorna-azonosítás).
- 🆕 Fuzz/mutáció a meglévő mintára (`catch_unwind`, csonkítás, bitflip, hibás méretek).

Eredeti B-tartalom:
- cfa4-cinema-ffi adapter: fájlinformáció, MXF read/write, JPEG 2000 stream, PCM, timed text támogatott részhalmaz.
- Safe Rust RAII, error enum, async job/cancel/progress, streaming nagy fájlokra és file descriptor kontroll.
- Fuzz az MXF/KLV/XML bináris határán, hibás és ellenséges bemenetek kezelésére.
**Gate:** round-trip és memory-safety teszt, nincsenek transzitív C++ objektumok a publikus ABI-ban. 🆕 (A: a `cfa4-mxf` által írt DCP/IMF track file-okat az `asdcp-info`/`asdcp-test -v` és a Clairmeta hiba nélkül olvassa; a `cargo xtask wasm` és `layers` zöld.)

### WP03 – DCP ingest/inspect
- Interop és SMPTE DCP import, AssetMap/PKL/CPL fájlok ✏️ **elemzése** (az eredetiben hibás karakterek), track feloldás és összefűzés.
- JPEG 2000 2K/4K, DCP frame rate, képarány, color XYZ kontroll; mono/stereo kép, PCM többcsatornás audio, timed-text (támogatott profilok szerint).
- Hash- és aláírás-státusz; sérült/hiányzó assets, eltérő UUID, idővonalhibák diagnosztikája.
- A titkosított tartalomhoz csak jogosult és támogatott dekódolási út használható.
- 🆕 A meglévő `cfa4-j2k` dekóder bekötése (gaps.md: „exists, not yet wired in”); proxy/preview a `cfa4-route`-on keresztül, mert 4K J2K valós idejű szoftveres dekódolása CPU-n várhatóan nem éri el a 24 fps-t – mérendő.
- 🆕 IAB hangsáv (meglévő `cfa4-iab`) és ST 428-7/Interop felirat beolvasása az idővonalra.
- 🆕 VF csomag ingest: hiányzó OV-assetek felismerése és OV-mappa hozzárendelése.
- 🆕 Multi-reel CPL → egyetlen idővonal, reel-határok jelölőként.
**Gate:** ✏️ valid DCP **import** reference fixtures (export a WP04-ben) és hibás csomagok explicit rejection.

### WP04 – DCP komponálás/mastering
- Profilvezérelt SMPTE/Interop composer; DCDM->XYZ, képkockánkénti J2K, WAV/PCM mapping, timed-text/font assets.
- CPL/PKL/AssetMap/VOLINDEX szerkesztő és determinisztikus XML-kimenet; UUID-k és referencia-integritás.
- DCP OV/VF és több CPL; 2D/3D a ténylegesen tesztelt szabványprofilok szerint.
- Asset-hash, opcionális XML aláírás; QC gate után atomi output finalizálás, részleges export helyreállítása.
- 🆕 **Alapprofil: SMPTE DCP az RDD 52 (Bv2.1) szerint**; Interop írás csak „legacy” opció, figyelmeztetéssel.
- 🆕 Konténerek és képarányok: 2K Flat 1998×1080, Scope 2048×858, Full 2048×1080; 4K: 3996×2160, 4096×1716, 4096×2160. Frame rate-ek: 24, 25, 30, 48, 50, 60 (HFR és 4K kombinációk profil szerint korlátozva); 3D csak 2K-ban és tesztelt rate-eken.
- 🆕 Színkonverzió: Rec.709 / P3-D65 / P3-DCI forrás → X'Y'Z' 12-bit, gamma 2.6, 48 cd/m² referenciafehér; a konverzió a `cfa4-color`-ban, egyetlen helyen (nincs kettős transzformáció).
- 🆕 Hang: 24-bit PCM, 48 vagy 96 kHz; 5.1 / 7.1 (DS és SDDS) csatornakiosztás, **ST 377-4 MCA címkék** (Bv2.1 kötelező), HI a 7., VI-N a 8. csatornán; IAB (ST 429-18) opcionális immerzív sávként a meglévő `cfa4-iab`-bal.
- 🆕 CPL metaadat (ST 429-16): ContentVersion, MainSoundConfiguration, MainPictureStoredArea/ActiveArea, ExtensionMetadata, Luminance; ISDCF Doc 2 szerinti ContentTitleText javaslat (WP09).
- 🆕 PKL hash: SHA-1, base64 – a szabvány így írja elő; mellette a belső provenance SHA-256/512 (WP08).
- 🆕 Determinizmus: IssueDate és UUID-forrás injektálható (teszt: rögzített óra + seedelt UUID → bitazonos XML); éles futáskor UUID v4 és valós idő.
- 🆕 Reel-felosztás: reel-hosszak, minimum 1 s / ajánlott korlátok, reel-határon frame-pontos hang.
- 🆕 Trailer/rövid tartalom: Leq(m) mérés figyelmeztetésként (ISDCF/SMPTE ajánlás), nem automatikus korrekció.
**Gate:** külső független DCP validátorral elfogadott, tesztanyaggal összevetett csomag. 🆕 Konkrétan: Clairmeta `check` és `dcpverify` hiba nélkül, `asdcp-info` minden track file-t olvas; legalább egy valós lejátszón (pl. szabad szoftveres DCP-lejátszó vagy partnermozi szervere) lejátszási próba dokumentálva.

### WP05 – IMF ingest és struktúramodell
- AS-02 és ST 2067-5 komponensek beolvasása; IMF AssetMap, PKL, CPL, szerializáció, cross-reference.
- Essence-, erőforrás-, virtuális track-, szegmens- és szerkesztési sebességmodell.
- CPL timeline megjelenítés, differenciális verzió-összehasonlítás; XML namespace/schema ellenőrzés.
- Profil-/Application-képességek külön whitelist szerint, nem általános minden IMF-profil támogatást állítunk.
- 🆕 Javasolt whitelist sorrend: (1) **App 2E** (ST 2067-21; J2K és HTJ2K, Rec.709/Rec.2020, SDR/PQ/HLG), (2) **App 4** cinema mezzanine (ST 2067-40), (3) App 5 ACES (ST 2067-50) csak ACES-döntés után; ProRes-alapú alkalmazás (RDD 45 / App ProRes) a meglévő ProRes kodekkel később.
- 🆕 OPL (ST 2067-100) és Output Profile List; Sidecar Composition Map (ST 2067-9) olvasása.
- 🆕 Racionális időzítés a meglévő `cfa4_time::Tick` modellre (23.976 = 24000/1001 IMF-ben megengedett, DCP-ben nem).
- 🆕 Timed text: IMSC1 Text (meglévő) és **IMSC1 Image** profil; ISXD (ST 2067-202) és MGA S-ADM (ST 2067-203) csak felismerés/átadás.
**Gate:** az engedélyezett IMF-profiloknál reference round-trip, ismeretlen profilnál korrekten visszautasít. 🆕 Photon ellenőrzés hiba nélkül az App 2E whitelistre.

### WP06 – IMF mastering/versioning
- IMP csomaggyártás, új CPL, meglévő essence újrafelhasználása, verziócsomag és területi/nyelvi variánsok.
- Timeline resource trim/repeat/continuation kezelés, audio és felirat sávok cseréje.
- VTFP alapján szemantikusan egyenértékű virtuális sávok beazonosítása, állományhash-től elkülönítve.
- Opcionális output profile/app policy, determinisztikus hiba- és validációs jelentés.
- 🆕 Supplemental IMP (csak az új/változott essence + új CPL), hivatkozás az eredeti IMP-re; hiányzó hivatkozott asset explicit hiba.
- 🆕 Hangsávok: ST 2067-8 common audio labels (soundfield group + MCA), nyelvi címkék RFC 5646 szerint.
- 🆕 HDR metaadat (App 2E): mastering display (ST 2086), MaxCLL/MaxFALL; Dolby Vision metaadat csak sidecar-átvételként (SCM), generálás nem.
- 🆕 Szállítói (pl. streaming-szolgáltatói) delivery spec-ek mint QC-profilok, ha a tulajdonos megnevezi őket (D5).
**Gate:** két független IMF tesztcsomag validálva és reprodukálható CPL különbségvizsgálat.

### WP07 – JPEG 2000 és color pipeline
- DCI/DCP kompatibilis JPEG 2000 encode/decode profil, bitmélység, frame/frame-size constraint, 2K/4K sebességprofilok.
- FFmpeg/GStreamer és OpenJPEG közös routing, nincs kettős színtranszformáció.
- XYZ kimenet, gamma és gamut, ICC/OCIO/ACES összhang; színskálák, mérhető ΔE/referenciaképek.
- CTL kizárólag opcionális ACES/legacy feladatra, annak upstream projektjéből.
- ✏️ **[MÓD]** Az enkóder alapja a meglévő `cfa4-j2k` (DCI 2K/4K kész). Hiányzik (#117): tile-ok, több minőségi réteg, SOP/EPH/PLT, IMF/broadcast profilok, **HTJ2K kódolás** (App 2E-hez). OpenJPEG csak orákulum (bitstream-validitás, PSNR-összevetés).
- 🆕 **Sebességcél (#116):** egy 2 órás 24 fps film = 172 800 képkocka; 1 fps-nél 48 óra, 8 fps-nél 6 óra. Javasolt elfogadási cél: 2K DCI ≥ 8 fps és 4K ≥ 2 fps egy 16 magos CPU-n (mérendő, a cél a tulajdonossal egyeztetendő). Párhuzamosítás képkockánként (`rayon`), később gyártósemleges GPU-útvonal (Vulkan compute) a `cfa4-route`-on keresztül, NVIDIA-only út nem lehet egyedüli.
- 🆕 Bitráta-korlátok: DCI max. 250 Mbit/s (1 302 083 bájt/képkocka 24 fps-nél), komponensenkénti korlát; HFR esetén profilfüggő.
- 🆕 Színreferencia: ΔE2000 ≤ 1,0 a CPU-referenciához képest (ADR-0003 A2 mércéjével összhangban).
**Gate:** vizuális és numerikus színparitás, bitstream és konténer QC, CPU-only út kötelező.

### WP08 – MXF digest és IMF-VTFP
- KLV packet boundary és MXF run-in kihagyásának megfelelő SHA-512 feldolgozó (mxf-digest), determinisztikus reference vektorokkal.
- VTFP resource normalizálás és kanonizálás a cinecert dokumentációhoz illeszkedve; az abban meghatározott interoperábilis fingerprint csak azonosításra.
- Emellett külön SHA-256 vagy SHA-512 file-level cryptographic digest, és nem keverjük a különböző algoritmusok jelentését.
- 🆕 Algoritmus-jegyzék a riportban: **SHA-1** (DCP PKL, szabványkötelező), SHA-256 (IMF PKL lehetőség szerint, ST 2067-2 kiadástól függően – ellenőrizendő), SHA-512 mxf-digest, VTFP; a felhasználói felületen mindegyik a nevével jelenik meg.
- 🆕 A `sha2` már a lockfile-ban van; SHA-1-hez pure-Rust `sha1` crate (RustCrypto, MIT/Apache) – `cargo deny` ellenőrzéssel.
**Gate:** CineCert reference fixture egyezés, randomizált property tests, nagy file streaming.

### WP09 – Metaadatok és ISDCF
- Hivatalos registries JSON verziózott offline snapshot, forrásmezők és auditnapló; opcionális online frissítés explicit engedéllyel.
- DCP/IMF nevekkel, területtel, ratinggel, nyelvvel, track ID-val, UUID-val kapcsolatos, regiszter által ténylegesen fedett mezők ellenőrzése.
- Registry validation és névadási szabály külön válik, nem minden konvenciót deklarálunk registry-eredetűnek.
- 🆕 ISDCF Doc 2 (Digital Cinema Naming Convention) generátor és ellenőrző: cím_típus-verzió_képarány_nyelv-felirat_terület-korhatár_hang_felbontás_stúdió_dátum_létesítmény_szabvány_csomagtípus – figyelmeztetésként, nem blokkolóként.
- 🆕 A snapshot asset-szabály alá esik (`AGENTS.md` §1): `.attribution` sidecar + `ATTRIBUTION.md` sor, `cargo xtask assets`.
- 🆕 Az online frissítés megfelel a „nincs runtime hálózati függőség” elvnek (ADR F8): csak kézi, explicit parancsra.
**Gate:** offline és online-snapshot paritás, hibás kódok megfogható hibákat adnak.

### WP10 – Titkosítás, aláírás, KDM
⚠️ **[ELLENTMONDÁS]** `docs/cinema-codecs.md` szerint jelenleg hatókörön kívül; tulajdonosi döntés (D3) szükséges.
- Encryption/decryption opciók csak megfelelő jogosultságú assetekhez; kulcsok életciklus-kezelése és biztonságos törlés.
- DCP essence encryption, CPL/PKL aláírás és KDM/X.509 kapcsolat a vonatkozó ST 430 profilok szerint.
- KDM generálás/import, validitási intervallum, tanúsítványlánc, eszköz-célzás és az engedélyezett terjesztés kontrollja.
- OS keyring/HSM lehetőség; titkok naplózása tilos, privát kulcs soha nem kerül projektfájlba.
- Szabályzat: nem kerülhet be DRM-feltörés, kulcskinyerés vagy jogosulatlan hozzáférés funkció.
- 🆕 Pontos algoritmusok: essence AES-128-CBC (ST 429-6), MIC HMAC-SHA1; KDM: RSA-2048 OAEP a CipherValue-hoz (ST 430-1), X.509 lánc (ST 430-2), XML-DSig RSA-SHA256 + **Canonical XML (C14N)** a CPL/PKL/KDM aláíráshoz.
- 🆕 Megvalósítás pure Rustban (RustCrypto: `aes`, `cbc`, `hmac`, `sha1`, `rsa`, `x509-cert`), nincs OpenSSL FFI. ⚠️ A Rustban nincs érett XML-DSig/C14N könyvtár – saját, szűk C14N implementáció kell, tesztvektorokkal (ez a WP legnagyobb kockázata).
- 🆕 DKDM (distribution KDM) a saját mastering-tanúsítványra; target-tanúsítványok importja `.pem`/`.crt` fájlból, lánc- és lejárat-ellenőrzés; ETM (ST 430-3) csak ha a célrendszer igényli.
- 🆕 Kulcstárolás: Linuxon Secret Service (libsecret / KWallet a Plasmán) egy pure-Rust D-Bus klienssel vagy jelszóval védett PKCS#8 fájl; a bridge és az MCP soha nem ad ki kulcsanyagot.
**Gate:** érvényes és lejárt/revokált/hibás cert/KDM teszt, titokkezelési review; titkosítatlan DCP export ettől függetlenül működik. 🆕 Titkosított DCP + KDM legalább egy független eszközzel (pl. DCP-o-matic KDM-ellenőrzés, `asdcp-test` kulccsal) visszafejthető.

### WP11 – Szabvány- és minőségellenőrző motor
- Profile-aware rules: schema/namespace, UUID, AssetMap/PKL/CPL referenciák, hash, signature, reel/szegmens, frame rate, image/audio constraints, timed-text.
- Report severity = BLOCKER/ERROR/WARNING/INFO; minden hibához file, UUID, timecode, szabvány és javítási javaslat.
- CLI JSON és Qt6 interaktív QC; automatikus javítás csak safe whitelisttel és explicit jóváhagyással.
- Független validátorral cross-check; egyetlen saját validator PASS nem egyenlő szabványtanúsítással.
- 🆕 Szabálykészletek: RDD 52 (Bv2.1) ellenőrzőlista, ISDCF Doc 2, ST 428-7 felirat-korlátok (fontméret, sorhossz, font-fájlméret), IMF App 2E korlátok, Photon-kompatibilis IMF ellenőrzések.
- 🆕 Szabályonként azonosító (pl. `DCP-CPL-017`) és forráshivatkozás (szabvány + szakasz), hogy a külső validátorok eredményével gépileg párosítható legyen.
- 🆕 Tartalmi QC (nem csak struktúra): fekete/néma képkockák, csatorna-fázis, clipping, felirat-időzítés átfedés, felirat a képterületen kívül.
**Gate:** negatív fixture lefedés és false-positive/false-negative jelentés, szabványprofilonként QC-tábla.

### WP12 – Qt6 GUI és CLI integráció
- Qt6 panelek: Cinema Project/Profiles, DCP Import/Inspector, DCP Composer, IMF Explorer, IMF Version Manager, JPEG 2000 Export, KDM Manager, Registry Browser, QC/Delivery.
- A GUI a Rust application service API-t használja; nem indul független alrendszer külön projektadatbázissal.
- Long-running jobs progress, cancel, resume (ahol biztonságos), message log, file picker, validációs részletek, magyar nyelv.
- `cfa4-cli cinema ...` logikai parancsnevek: inspect, create-dcp, create-imf, validate, digest, fingerprint, kdm (pontos CLI szerkezet repository-kompatibilitási audit után).
- ✏️ **[MÓD]** „Rust application service API” → a meglévő **parancsrendszer + `cfa4-cli --bridge`** (ADR F8); minden művelet MCP-n/agent-vezérléssel is elérhető (CLAUDE.md: „everything is a command”).
- 🆕 Az egui az ADR F8 szerint „befagyasztott”: új nagy panel elsődlegesen Qt6-ban készül; tisztázandó (D4), hogy az M2-ig kell-e egui-felület is, vagy elég a CLI + Qt prototípus.
- 🆕 Az export-módban DCP/IMF presetek (`crates/export/src/presets.rs`), hogy a meglévő export-sor (queue) használható legyen.
- 🆕 Lokalizáció a `docs/localization-parity.md` szerinti folyamattal; a szakkifejezések (CPL, PKL, KDM, reel) nem fordítandók.
**Gate:** CLI/Qt6 ugyanazon core outputot állít elő; Wayland/Plasma és GNOME/Cinnamon smoke.

### WP13 – Delivery, archiválás és provenance
- Hibabiztos, ellenőrzött átadás local disk/network/removable media felé; tárolási cél és jogosultság ellenőrzése.
- Determinisztikus manifest, teljes fájlhash, másolás utáni verify, atomic completion marker, export report.
- Resume és rollback, megszakadt export jelölése INCOMPLETE; soha ne látszódjon késznek hibás csomag.
- Provenance: source asset hash, build/profile version, transcoder id, QC results, timestamps, külső validátor verzió.
- 🆕 **DCP-meghajtó** (ISDCF ajánlás): ext2 vagy ext3, 128 bájtos inode-méret, MBR partíciós tábla; a formázás csak külön megerősítéssel és csak a kiválasztott eszközön (adatvesztés-veszély) – vagy csak ellenőrzés és útmutató, formázás nélkül (javasolt első lépésben).
- 🆕 Több kötetes DCP (VOLINDEX, ASSETMAP VolumeCount) nagy csomagokhoz.
- 🆕 Fájlnév-korlátok (ASCII, hosszkorlát, nincs szóköz) ellenőrzése másolás előtt.
- 🆕 Archiválás: IMF mint archív mesterformátum; opcionális BagIt csomagolás a provenance-riporttal.
**Gate:** megszakított másolás és sérült asset tesztek, kész átadás után bitazonosság.

### WP14 – Kiadási csomagok, CI és regresszió
- Ubuntu 26.04 amd64: tar fejlesztői, .deb, Flatpak, AppImage; mindegyiknél natív függőségek és licenc manifest.
- CPU-only teszt, NVIDIA támogatás csak opcionális képességként; VRAM hiány esetén biztonságos CPU fallback.
- CI: clang/gcc build, cargo fmt/clippy/test, native sanitizers, FFI ABI, J2K, DCP/IMF fixtures, GUI offscreen, packaging, Q&A report.
- Nagyméretű referenciatesztek CI külön, letöltési jogok és minták SHA-val rögzítve; release csak teljes zöld gate-tel.
- ✏️ **[MÓD]** A CI a repó szabálya szerint: automatikus PR-ellenőrzés gyors és nem fordít; teljes Rust/Qt6/orákulum-tesztek csak `workflow_dispatch` vagy `cfa4-full-validation` címke esetén. A gate-ek: `cargo xtask ci` (fmt, clippy, test, layers, assets, wasm). A clang/gcc/sanitizer/FFI ABI lépés csak B változatban kell.
- 🆕 A `docs/releasing.md` eltérése az `AGENTS.md`-től (0.2 / E8) külön PR-ben jelzendő.
**Gate:** tiszta Ubuntu 26.04 install / uninstall / parallel-install PASS, csomag és GUI ugyanazzal a média-engine-nel.

### 🆕 WP15 – [ÚJ] Feliratok, zárt feliratok és akadálymentesítés
- DCP felirat: **SMPTE ST 428-7** timed text (XML + font), Interop **CineCanvas** XML (olvasás, legacy írás), PNG-alapú képfeliratok; font beágyazás és subset, fontméret-korlát (ellenőrizendő érték), ST 429-5 track file csomagolás.
- Zárt feliratok (CCAP) és nyitott feliratok külön sávtípusként, ST 429-12 szerint; több nyelv, CPL-enkénti nyelvválasztás.
- Akadálymentes hangsávok: HI (hallássérült, dialógus-kiemelt) és VI-N (narrált audiokommentár) a 7–8. csatornán MCA-címkével; ISDCF jelnyelvi videó (a DCP-ben hangcsatornán hordozott videó – ISDCF dokumentum alapján, ellenőrizendő).
- IMF: IMSC1 Text (meglévő) és IMSC1 Image profil, ST 2067-2 szerinti timed text resource; forced narrative jelölés.
- Konverziók a meglévő `cfa4-captions` formátumokból (SRT, VTT, STL, SCC, TTML) DCP/IMF feliratokba, a sorhossz/időtartam szabályok QC-jével.
- Magyar és más ékezetes karakterek: Unicode-normalizálás, font-lefedettség ellenőrzése (hiányzó glifa = ERROR).
**Gate:** a generált feliratokat Clairmeta és `dcpverify` elfogadja; IMSC1 a Photonon és egy W3C IMSC-ellenőrzőn átmegy; minden akadálymentes sáv MCA-címkéje helyes.
**Becsült ráfordítás:** 50–80 óra.

### 🆕 WP16 – [ÚJ] IMF App 2E / HDR és HTJ2K kódolás
- HTJ2K kódolás a `cfa4-j2k`-ban (a dekóder kész; ITU-T T.814 | ISO/IEC 15444-15 publikus).
- App 2E profilok: Rec.709 / Rec.2020, SDR / PQ / HLG, 10/12/16 bit, 4:4:4 és 4:2:2, J2K és HTJ2K bitstream-korlátok.
- HDR metaadat: mastering display (ST 2086), MaxCLL/MaxFALL számítás a renderelt képkockákból; a `cfa4-color` PQ/HLG transzferjeire építve.
- Dolby Vision: csak a meglévő metaadat-XML átvétele sidecarként (SCM), saját generálás nincs (licenc).
**Gate:** Photon App 2E PASS, HTJ2K bitstream OpenJPEG/Grok orákulummal dekódolható és a reverzibilis út bitazonos.
**Becsült ráfordítás:** 70–120 óra.

### 🆕 WP17 – [ÚJ, 2026-10-27] Auro-3D felismerés és PCM-bitpontosság-őr (opcionális, P2) – ✅ jóváhagyva (István, 2026-11-09)
*Forrás: https://github.com/cinema-ONE/auro3d-detector értékelése.* Az Auro-3D (mozis Auro 11.1/13.1 is) a magassági réteget a normál 24-bit PCM 5.1/7.1 hordozó legalsó bitjeiben viszi; semmilyen metaadat (CPL, MXF, MCA) nem jelzi. A DCP-ben így egy „sima 5.1” sáv lehet Auro-kódolt – és bármilyen hangfeldolgozás csendben tönkreteszi.
- **Felismerés (ingest/QC, WP03/WP11):** a 0. bitben 16 egyes szinkron, blokkhossz a fejlécből, CRC16-CCITT a 24-bites minták mindhárom bájtján; layout az ADOL bytecode `0x1E` (ChannelInputConfig) operandusából. Saját, `unsafe` nélküli Rust implementáció (pl. `cfa4-audio` vagy a QC-crate-ben), a nyilvános leírás (README, Auro white paper, US8620465) alapján; a Python-kód nem portolható be (clean-room szabály, `docs/licensing.md` §5).
- **Bitpontosság-őr (WP04/WP06, ez a fő érték):** ha a forrás PCM Auro-t hordoz, minden olyan lépés BLOCKER/WARNING-ot kap, ami a mintákat módosítja: mintavételi-frekvencia konverzió (48↔96 kHz), 24→25 fps PAL speed-up / pitch-korrekció, gain/normalizálás, dither, bitmélység-csökkentés, csatorna-átrendezés. Export után ellenőrzés: a kimeneti MXF PCM-ből újra kimutatható az Auro, és a CRC-érvényes blokkok aránya ≈ a forrásé.
- **Layout-riport:** csak a hardveresen megerősített azonosítók (20, 30, 40, 50, 54, 62) kapnak nevet; ismeretlen azonosító „ismeretlen + nyers id”, nem találgatunk. A CPL-be semmi nem íródik automatikusan (nincs szabványos jelölés).
- **Orákulum:** `auro3d.py` (külső folyamat, pinelt commit) ugyanarra a fixture-re ugyanazt a jelenlét/id-t adja; hiányzó orákulumnál a teszt kihagyódik és jelzi.
- **Nyitott pontok:** ⚠️ a projekt korpusza otthoni Blu-ray (DTS-HD MA/LPCM), **mozis Auro DCP-n nincs igazolva** – valódi Auro DCP mintára van szükség (D6); ⚠️ jogi ellenőrzés (szabadalom, visszafejtett táblák); egyszemélyes, AI-segítséggel írt, ~2 hónapos repó, tesztek nincsenek benne – karbantartására nem építünk.
**Gate:** szintetikus negatív korpuszon 0 téves pozitív (Atmos/IAB, sima 5.1/7.1, zaj, LFE); legalább egy valódi Auro DCP-n pozitív és az orákulummal egyező id; a bitpontosság-őr minden mintamódosító lépést elkap.
**Becsült ráfordítás:** 16–30 óra (felismerés + őr + tesztek), fixture-beszerzés nélkül.

## 6. DCP/IMF használati folyamatok – definíciók

**DCP ingest:** csomag kiválasztása -> AssetMap -> PKL/CPL -> track metaadat -> hash/signature -> dekódolható proxy/preview -> timeline link -> QC. Titkosított assetnél csak jogosult decryption.

**DCP create:** CFA4 timeline freeze -> render profile -> DCDM/XYZ+PCM+timed-text -> JPEG 2000 -> AS-DCP MXF -> CPL/PKL/AssetMap/volindex -> optional crypto/KDM -> QC -> external validator -> atomic delivery.

**IMF ingest:** IMP scan -> CPL/AssetMap/PKL -> essence resource map -> virtual track -> rational timing -> fingerprint -> timeline view -> QC.

**IMF create/version:** source/render resources -> AS-02 essence -> CPL track resources -> shared assets -> PKL/AssetMap -> VTFP + digests -> QC -> version export + provenance.

🆕 **DCP VF (version file) create:** meglévő OV ingest -> csak az eltérő reel-ek/sávok renderelése (pl. lokalizált hang, felirat, inzert) -> új CPL, amely az OV assetekre hivatkozik -> PKL csak az új assetekkel -> QC az OV-vel együtt.

🆕 **KDM kiadás (D3 után):** DKDM import/saját mastering-kulcs -> célszerver tanúsítvány import + láncellenőrzés -> érvényességi ablak -> KDM generálás -> ellenőrzés -> átadás (fájl), napló kulcsanyag nélkül.

## 7. Ütemezés és függőségek

| Szakasz | Függőség | Eredmény |
|---|---|---|
| G0 WP00 | nincs | teljes hiánytérkép + specifikációmátrix |
| G1 WP01–02 | G0 | ✏️ A: orákulum-környezet + `cfa4-mxf` DCP/IMF bővítés; B: izolált lib és Rust FFI |
| G2 WP03, WP05, WP08, WP09 | G1 (WP08 részben független) | megbízható DCP/IMF read+inspect |
| G3 WP07, WP04, 🆕 WP15 (DCP-felirat rész) | G2 és media-router | DCP write+render |
| G4 WP06, 🆕 WP16 | WP05, WP08 | IMF version/mastering |
| G5 WP10–11 | G2–G4 releváns elemei; 🆕 WP10 csak D3 után | biztonságos crypto + QC |
| G6 WP12–13 | stabil Rust API, QC | végfelhasználói UI/CLI + delivery |
| G7 WP14 | valamennyi célkomponens | támogatott Linux-csomagok |

🆕 **Megjegyzés:** a WP11 QC-motor alapjait (séma, referenciák, hash) már G2-ben érdemes elkezdeni, mert a WP03 ingest és a WP04 gate is erre épül; különben a G3 gate csak külső validátorra támaszkodhat.

⏳ **M2 utáni döntési kapu:** az M2 lezárásakor a D1 (A → B váltás) újra a tulajdonos elé kerül (16. fejezet).

Kiadási inkrementumok: M1 inspect-only (nem állítható masteringnek); M2 validált titkosítatlan SMPTE DCP mastering; M3 IMF ingest és támogatott profilú mastering/versioning; M4 KDM és aláírt/encrypted delivery; M5 teljes GUI, csomagolás, QC. Minden inkrementum önállóan és pontos néven jelentendő; részfunkció nem nevezhető kész egésznek.

🆕 Inkrementumok pontosítása: M2 = SMPTE DCP Bv2.1, 2K/4K 2D, 24/25 fps, 5.1/7.1 PCM + MCA, ST 428-7 felirat, OV; M2.1 = VF, 3D, HFR, IAB; M3 = IMF App 2E (J2K), M3.1 = HTJ2K + HDR.

## 8. Kiadás előtti kötelező elfogadási tesztek

1. Clean Ubuntu 26.04 amd64 install (tar/.deb/Flatpak/AppImage), konfliktusmentes párhuzamos működés más OpenJPEG/FFmpeg/GStreamer alkalmazások mellett.
2. Media-router regression: korábbi CFA4 import/export, encode, decode, Qt6 funkciók és idővonalváltozatok nem romlanak.
3. DCP referencia 2K/4K (támogatott képkockasebességek), mono/stereo, PCM channel mapping, timed text, OV/VF, Interop/SMPTE típus szerint explicit támogatási mátrix.
4. IMF referencia AS-02/ST 2067 engedélyezett alkalmazásprofilok, több CPL és verziócsomag.
5. JPEG 2000 compliance, XYZ/ACES/OCIO számszerű színtesztek, validált bitstream.
6. MXF digest reference, ✏️ VTFP: **ugyanazt a tartalmat különböző módon (más fájlfelosztással/trim-mel) leíró virtuális sávokra azonos fingerprint, tartalmilag eltérő (pl. más sztereó erőforrás) sávra eltérő** – az eredeti „4 azonos virtuális sáv” megfogalmazás pontosítva.
7. Hash, aláírás, KDM, kulcsjogosultság, érvényesség, negatív esetek.
8. Truncation/corruption, XML external entity, path traversal, oversized input, UUID duplikáció, race/cancel/resume, out-of-disk-space.
9. CLI/GUI egyezőség, gyors megszakítás, hungarian localization, Qt6 Wayland smoke.
10. Reprodukálható package, SBOM, notice/licences, külső validátorral megerősített PASS; bizonyíték issue/PR/CI-artifact szinten.
11. 🆕 Akadálymentesség: HI/VI-N sávok MCA-címkéje, zárt felirat, IMSC1 Text/Image validáció (WP15).
12. 🆕 Teljesítmény: J2K kódolási sebesség a WP07 céljai szerint, mérés rögzítve a `docs/performance.md`-ben.
13. 🆕 Hosszú tartalom: legalább 2 órás, több reeles DCP és IMF generálása memória-plafon alatt (streaming, nincs teljes fájl a memóriában).
14. 🆕 Determinizmus: rögzített óra + UUID-seed mellett két futás bitazonos XML-t és MXF-et ad.
15. 🆕 Never-crash: minden új parser mutációs fuzz-tesztje `catch_unwind` alatt (AGENTS.md §0.8).

## 9. Kockázatok és kezelési szabályok

- **Profilfüggő szabványmegfelelőség:** DCP/IMF általános elnevezése nem bizonyít minden alkalmazási profil támogatását. Profilonként acceptance matrix.
- **J2K teljesítmény:** rétegzett CPU GPU fallback, profilhelyes output prioritás a sebesség előtt.
- **C++/Rust ABI:** minimális C interface, izolált build, sanitizer és fuzz. 🆕 (A változatban nem áll fenn.)
- **Csomagolás és OS-coexistence:** saját runtime névterek, nincs globális install.
- **Licenc és attribution:** GPLv3 és upstream licencek jogi ellenőrzése, fájlszintű notices.
- **Kulcsbiztonság:** rejtett titkok, RBAC/OS keyring, KDM jogosultságteszt, semmi DRM megkerülés.
- **Qt6 átállás:** API-first; UI csak stabil szolgáltatásokra épül, fennálló Qt6 PR regressziótesztek kötelezők.
- **Legacy PHDR:** funkcióígéret nélkül karantén/kutatási ág.
- 🆕 **Szabványhozzáférés:** a SMPTE dokumentumok többsége fizetős; a clean-room szabály szerint specifikáció nélkül nem kezdhető el munka. Kezelés: WP00-ban beszerzési lista és költség, a használt kiadás a crate README-ben.
- 🆕 **XML-DSig / C14N Rustban:** nincs érett könyvtár; saját implementáció hibája érvénytelen aláírást okoz. Kezelés: szűk C14N részhalmaz, tesztvektorok, külső ellenőrzés (`xmlsec1` mint orákulum).
- 🆕 **Fixture-hiány:** kevés szabadon terjeszthető, valódi DCP/IMF csomag létezik. Kezelés: szintetikus generálás + orákulummal készített csomagok + licencelt nyílt tartalom (13. fejezet).
- 🆕 **Párhuzamos munka ütközése:** a `crates/mxf` és `crates/export` más munkamenetekben is változhat. Kezelés: kis PR-ek, friss `main`, `docs/coordination-log.md` bejegyzés.
- 🆕 **Felelősség valós mozis lejátszásért:** egy hibás DCP nyilvános vetítésen hibát okozhat. Kezelés: a riport mindig jelzi, hogy saját PASS ≠ tanúsítás; éles átadás előtt tesztvetítés ajánlott.
- 🆕 **Védjegyek:** „Dolby”, „Atmos”, „DCI” nevek csak leíró módon, logó nélkül (AGENTS.md márkaszabály szellemében).
- 🆕 **Rejtett LSB-hangformátum (Auro-3D):** egy metaadat nélküli PCM-sáv hordozhat Auro réteget, amit resample/gain/dither csendben megsemmisít. Kezelés: WP17 felismerés + bitpontosság-őr; „Auro-3D” név csak leíró módon.

## 10. Kész definíciója és döntési kapu

Csak az a munkacsomag DONE, amelynek kódja, dokumentációja, licencellenőrzése, futó tesztje, valós referenciamintája, negatív tesztje, CI bizonyítéka és integrációs review-ja megtörtént. 'PASS_PARTIAL' nem konvertálható 'PASS_FULL'-lá. Módosítást kizárólag a CFA4 repóban, izolált ágban/PR-ben szabad végezni. Az érintett párhuzamos ágak és nyitott PR-ek változásait merge előtt újra ellenőrizni kell. Az upstreamhez nem nyúlunk.

🆕 Kiegészítés: DONE feltétele továbbá a `cargo xtask ci` zöld futása, a `docs/cinema-codecs.md` / `docs/gaps.md` / `docs/file-format-parity.md` érintett sorainak frissítése, és – új crate esetén – a `LAYERS` bejegyzés. Merge-et kizárólag a kijelölt merge-koordinátor végez (`docs/coordination.md`).

## 11. Források

- CFA4: https://github.com/ubuntuokos/CinemaFundArchitecture4
- asdcplib: https://github.com/cinecert/asdcplib
- mxf-digest: https://github.com/cinecert/mxf-digest
- imf-vtfp: https://github.com/cinecert/imf-vtfp
- OpenJPEG upstream: https://github.com/uclouvain/openjpeg
- CTL upstream: https://github.com/aces-aswf/CTL
- ISDCF registries upstream: https://github.com/ISDCF/registries
- CineCert organization: https://github.com/cinecert
- 🆕 Netflix Photon: https://github.com/Netflix/photon
- 🆕 Clairmeta: https://github.com/Ymagis/ClairMeta
- 🆕 DCP-o-matic / libdcp (csak külső eszközként): https://dcpomatic.com
- 🆕 ISDCF dokumentumok: https://isdcf.com
- 🆕 Repó belső hivatkozások: `AGENTS.md`, `CLAUDE.md`, `docs/cinema-codecs.md`, `docs/gaps.md`, `docs/adr/0001-platform-ffi.md`, `docs/adr/ADR-0003-aswf-and-conversion-sources.md`, `docs/adr/ADR-0005-media-backends.md`, `docs/adr/ADR-F8-QT6-RUST-ENGINE-2026-10-23.md`, `crates/mxf/README.md`, `crates/j2k/README.md`

## 12. 🆕 [ÚJ] Szabványmátrix (kiindulás a WP00-hoz)

A kiadásévek a WP00-ban rögzítendők; „Prioritás” = melyik inkrementumhoz kell.

| Szabvány | Tárgy | Érintett WP | Prioritás |
|---|---|---|---|
| SMPTE ST 428-1 | DCDM képkarakterisztika (X'Y'Z', gamma 2.6) | WP07 | M2 |
| SMPTE ST 428-7 | D-Cinema felirat (timed text XML) | WP15 | M2 |
| SMPTE ST 429-2 | DCP működési korlátok | WP04, WP11 | M2 |
| SMPTE ST 429-3 | Hang- és képtrack file | WP02-A | M2 |
| SMPTE ST 429-4 | JPEG 2000 MXF DCP-hez | WP02-A | M2 |
| SMPTE ST 429-5 | Timed text track file | WP02-A, WP15 | M2 |
| SMPTE ST 429-6 | Essence-titkosítás MXF-ben | WP10 | M4 |
| SMPTE ST 429-7 | Composition Playlist (CPL) | WP03, WP04 | M1/M2 |
| SMPTE ST 429-8 | Packing List (PKL) | WP03, WP04 | M1/M2 |
| SMPTE ST 429-9 | Asset Map | WP03, WP04 | M1/M2 |
| SMPTE ST 429-10 | Sztereoszkópikus kép track file | WP02-A | M2.1 |
| SMPTE ST 429-12 | Caption és zárt felirat | WP15 | M2.1 |
| SMPTE ST 429-14 | Auxiliary data track file | WP04 (csak átvétel) | később |
| SMPTE ST 429-16 | CPL metaadat és aláírás-kiegészítések | WP04 | M2 |
| SMPTE ST 429-18 | IAB track file DCP-ben | WP04 | M2.1 |
| SMPTE RDD 52 | SMPTE DCP Bv2.1 alkalmazási profil | WP04, WP11 | M2 |
| SMPTE ST 430-1 / -2 / -3 | KDM / tanúsítványok / ETM | WP10 | M4 |
| SMPTE ST 433 | XML adattípusok | WP03–06 | M1 |
| SMPTE ST 377-1, ST 377-4, ST 379-1/2, ST 422 | MXF, MCA címkék, generic container, J2K mapping | WP02-A | M1/M2 |
| SMPTE ST 336 | KLV kódolás | WP02-A, WP08 | M1 |
| SMPTE ST 2067-2 / -3 / -5 | IMF core / CPL / essence component | WP05, WP06 | M3 |
| SMPTE ST 2067-8 | Common audio labels | WP05, WP06 | M3 |
| SMPTE ST 2067-9 | Sidecar Composition Map | WP05, WP16 | M3.1 |
| SMPTE ST 2067-21 | Application #2E | WP05, WP16 | M3 |
| SMPTE ST 2067-40 | Application #4 cinema mezzanine | WP05 | később |
| SMPTE ST 2067-50 | Application #5 ACES | WP05 | később (ACES-döntéstől függ) |
| SMPTE ST 2067-100 / -101 / -102 | OPL / output profile / pixel color schemes | WP05, WP06 | M3 |
| SMPTE ST 2067-201 / -202 / -203 | IAB / ISXD / MGA S-ADM IMF-ben | WP05 | M3 (IAB), később (többi) |
| SMPTE ST 2098-2 | IAB bitfolyam | meglévő `cfa4-iab` | megvan |
| SMPTE ST 2086 | Mastering display metaadat | WP16 | M3.1 |
| ITU-T T.800 / ISO/IEC 15444-1 | JPEG 2000 Part 1 | meglévő `cfa4-j2k` | megvan |
| ITU-T T.814 / ISO/IEC 15444-15 | HTJ2K | WP16 (kódolás) | M3.1 |
| W3C IMSC 1 / 1.1 | IMF timed text | WP15 | M3 |
| ISDCF Doc 2 és ajánlások | névadás, meghajtó, jelnyelv | WP09, WP13, WP15 | M2 |
| RFC 5646 | nyelvi címkék | WP05, WP15 | M2 |
| W3C XML-DSig, Canonical XML 1.0 | aláírás | WP10 | M4 |

## 13. 🆕 [ÚJ] Külső validátorok és tesztanyag-források

**Validátorok (mindegyik külső folyamat, verzió a riportban):**

| Eszköz | Mire | Licenc (ellenőrizendő a pinelt verziónál) | Megjegyzés |
|---|---|---|---|
| Clairmeta | DCP és IMF struktúra/tartalom | BSD-típus (ellenőrizendő) | Python venv |
| Netflix Photon | IMF (App 2/2E) | Apache-2.0 | Java |
| `dcpverify` (DCP-o-matic/libdcp) | DCP Bv2.1 ellenőrzések | GPL | csak futtatás, kódátvétel nincs |
| asdcplib CLI (`asdcp-info`, `asdcp-test`, `as-02-*`) | MXF track file olvashatóság, kulcsos visszafejtés | BSD-3-Clause (ellenőrizendő) | orákulum |
| OpenJPEG / Grok | J2K bitstream dekódolhatóság | BSD-2 / AGPL (Grok: csak futtatás!) | Grok AGPL – kód nem vehető át |
| `xmlsec1` | XML-DSig ellenőrzés | MIT | WP10 |
| 🆕 `auro3d.py` (cinema-ONE/auro3d-detector) | Auro-3D jelenlét és ChannelInputConfig id | MIT | WP17, opcionális; Python 3.9+ és ffmpeg |
| mxf-digest / imf-vtfp referencia | digest és fingerprint vektorok | ellenőrizendő | WP08 |

**Fixture-források (a repóba nem kerülnek; pinelt URL + sha256, `cargo xtask corpus` mintára):**
1. **Szintetikus, saját generálású** csomagok (színsávok, hangtesztjelek, időkód-burn-in) – elsődleges, licencproblémától mentes, `.attribution` sidecarral.
2. **Orákulummal készített** csomagok (asdcplib `asdcp-test -c`, DCP-o-matic) ugyanabból a szintetikus anyagból – kereszt-összehasonlításhoz.
3. **Negatív korpusz:** a fenti csomagok szándékosan rontott változatai (hiányzó asset, hibás hash, rossz UUID, csonkított MXF, XXE-t tartalmazó XML, path traversal az ASSETMAP-ben).
4. **Nyílt licencű valódi tartalom** (pl. a Netflix Open Content projekt CC BY 4.0 alatt közzétett anyagai, IMF-csomagokkal) – licenc és letöltési feltételek ellenőrzése után, csak külső korpuszban.
5. A Photon és a Clairmeta saját tesztkészletei (ha licencük engedi) a negatív esetekhez.
6. 🆕 **Legacy csomagok OpenDCP-vel** (konténerben, izolált régi toolchainnel buildelve): Interop és Bv2.1 előtti SMPTE DCP-k az ingest/QC teszteléséhez – az elvárt QC-eredmény itt „hiányzó MCA / nem Bv2.1” figyelmeztetés, nem PASS.

## 14. 🆕 [ÚJ] Ráfordítás-becslés

Durva becslés (±50%), a `docs/codec-parity.md` órás skáláján, a meglévő crate-ek figyelembevételével. A szabványok beszerzési ideje és költsége nincs benne.

| WP | A változat (óra) | B változat eltérése | Megjegyzés |
|---|---|---|---|
| WP00 | 12–20 | – | gap matrix, szabványlista, döntések |
| WP01 | 8–16 (orákulum-környezet) | 30–50 | B: natív build, SBOM, sanitizer |
| WP02 | 40–70 (WP02-A, MXF bővítés) | 50–80 (FFI) | |
| WP03 | 30–50 | −10 | J2K dekóder kész |
| WP04 | 50–80 | −10 | `cinema-codecs.md` 20–30 órát ír csak a csomagolásra |
| WP05 | 40–60 | −10 | |
| WP06 | 50–80 | – | |
| WP07 | 40–80 | – | ebből sebesség (#116) 20–40 |
| WP08 | 15–25 | – | |
| WP09 | 10–20 | – | |
| WP10 | 100–160 | −20 | C14N/XML-DSig a fő tétel; csak D3 után |
| WP11 | 60–100 | – | |
| WP12 | 60–100 | – | Qt6 + bridge parancsok |
| WP13 | 25–40 | – | |
| WP14 | 20–40 | +20 | |
| WP15 🆕 | 50–80 | – | |
| WP16 🆕 | 70–120 | – | |
| **Összesen** | **~680–1 120** | | M2-ig (WP00–04, 07–09, 11 alap, 15 DCP-rész): ~330–520 |

## 15. 🆕 [ÚJ] Felelősség és PR-bontás

A repó koordinációs szabálya (egy téma / PR, friss `main`, csak a merge-koordinátor merge-el) miatt javasolt PR-sorrend M2-ig, mindegyik önállóan zöld és visszavonható:

1. `docs`: terv elfogadása, `cinema-codecs.md` / `gaps.md` frissítés, szabványmátrix (WP00).
2. `xtask`: orákulum-telepítő és -felismerő, korpusz-letöltő sha256-tal (WP01-A).
3. `mxf`: ST 422 J2K olvasás + bekötés a `cfa4-codecs`-be (WP02-A, a #115 olvasási fele).
4. `mxf`: J2K és PCM DCP track file írás, MCA címkék (WP02-A).
5. `cinema-pkg` (új crate, `LAYERS` bejegyzéssel): CPL/PKL/ASSETMAP olvasás + séma-ellenőrzés (WP03).
6. `cinema-pkg`: írás, determinisztikus UUID/óra, SHA-1 PKL hash (WP04).
7. `cinema`: DCP inspect parancs + CLI/MCP (WP03, M1).
8. `j2k`: sebesség (#116), tile/rétegek ahol a profil kéri (#117).
9. `cinema`: DCP composer + export preset (WP04, #115).
10. `captions`: ST 428-7 írás/olvasás, font subset (WP15).
11. `cinema`: QC-szabályok alapkészlete + JSON riport (WP11).
12. Qt6 bridge-parancsok és panelek (WP12) – a Qt-munkával egyeztetve.

**Felelősök (kitöltendő a tulajdonos által):** termékdöntések és D1–D5 – István; merge – kijelölt merge-koordinátor; munkacsomagonként egy felelős munkamenet/fejlesztő, akit a `docs/coordination-log.md` rögzít.

## 16. 🆕 [ÚJ] Nyitott tulajdonosi döntések

| ID | Kérdés | Javaslat |
|---|---|---|
| D1 | asdcplib termékbe emelése FFI-vel (B) vagy clean-room Rust + asdcplib orákulumként (A)? | ✅ **ELDÖNTVE (2026-10-27, István): A** – clean-room Rust, asdcplib/OpenJPEG csak tesztorákulum. Később B-re bővíthető új ADR-rel. ⏳ **Újradöntési kapu (István, 2026-10-27):** az M2 (validált titkosítatlan SMPTE DCP) elkészülte után a B változatra (asdcplib FFI, ADR 0006) való váltást újra kell vizsgálni és a tulajdonos elé kell vinni. |
| D2 | Interop DCP írás kell-e, vagy csak olvasás? | csak olvasás + legacy írás figyelmeztetéssel, alapértelmezés SMPTE Bv2.1 |
| D3 | KDM/titkosítás hatókörbe kerül-e (a `cinema-codecs.md` jelenleg kizárja)? | igen, de M4-ben, M2/M3 után |
| D4 | M2-höz kell-e egui-felület, vagy elég a CLI + Qt6 prototípus? | CLI + MCP elsőként, Qt6 panel a Qt-ütemezéssel; egui csak export-preset szinten |
| D5 | Van-e konkrét szállítói (fesztivál, forgalmazó, streaming) delivery spec, amelyre QC-profil kell? | a tulajdonos nevezze meg; addig általános RDD 52 / App 2E profil |
| 🆕 D6 | Kell-e Auro-3D támogatás (WP17), és van-e hozzáférés valódi Auro DCP mintához? | ✅ **ELDÖNTVE (2026-11-09, István):** igen, csak felismerés + bitpontosság-őr, M2 után, P2. ⏳ Indítási feltétel: valódi Auro DCP minta és jogi ellenőrzés (szabadalom, visszafejtett táblák) – ennek hiányában nem indul. |

---

## Változásnapló

- **v2.3 jóváhagyás (2026-11-13):** István a tervet egészében jóváhagyta, és elrendelte a fejlesztés indítását a 15. fejezet szerinti PR-sorrendben (1. PR: WP00 `docs`).
- **v2.3 (2026-11-09):** a v2.1 (WP17, D6) és v2.2 (OpenDCP mint legacy fixture-forrás) kiegészítéseket István jóváhagyta; D6 eldöntve.
- **v2.2 (2026-11-08):** 🆕 tmeiczin/opendcp értékelése: forrássor (3) és 6. fixture-forrás (13) – csak legacy DCP fixture-generátorként.
- **v2.1 (2026-10-27):** 🆕 cinema-ONE/auro3d-detector értékelése: új WP17 (Auro-3D felismerés és PCM-bitpontosság-őr), forrássor (3), orákulumsor (13), kockázat (9), D6 döntés (16).
- **v2 (2026-10-27):** repóellenőrzés (`main` @ 16debf9) alapján elemzési fejezet (0), ellentmondások (E1–E8), A/B stratégiai változat, 4.1 valós architektúra, WP-kiegészítések, új WP15 (feliratok, akadálymentesség) és WP16 (IMF App 2E / HDR / HTJ2K), szabványmátrix (12), validátorok és fixture-ök (13), ráfordítás-becslés (14), PR-bontás (15), nyitott döntések (16); javítva a WP03 karakterhibája, a WP03 gate és a 8.6. teszt megfogalmazása.
- **v1 (2026-10-27):** eredeti terv.

**Állapot:** a terv jóváhagyva (2026-11-13); a fejlesztés külön munkaszálban indul.
