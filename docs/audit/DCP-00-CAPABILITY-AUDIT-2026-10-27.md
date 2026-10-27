# DCP-00 – CFA4 képességaudit a DCP-validációhoz

**Dátum:** 2026-10-27  
**Státusz:** audit kész, **tulajdonosi jóváhagyásra vár** (7. pont döntései)  
**Terv:** [CFA4-DCP-VALIDATION-001](../plans/DCP-VALIDATION-2026-10-27.md), DCP-00 munkacsomag  
**Vizsgált állapot:** `main` @ `16debf92d85430d7bbf43687c35ab1b043b2a9ca` (2026-10-27 16:29 CET), és a 2026-10-27 17:00 UTC-kor nyitott PR-ok  
**Párdokumentum:** [DCP-01 forrás- és licencaudit](DCP-01-SOURCE-LICENSE-AUDIT-2026-10-27.md)

Csak olvasásos vizsgálat: kód nem változott. Minden állítás mögött a megnevezett fájl vagy PR áll;
a mérés nélküli becsléseket külön jelzem.

## 1. Összefoglaló

1. **DCP-csomagszintű kód ma nincs a CFA4-ben.** Sem ASSETMAP-, PKL- vagy CPL-olvasó, sem
   DCP-import, -export vagy -validáció. A csomagimport külön nyitott feladat: #101 (G10).
2. **Az essence-szintű alapok erősek, és újrahasznosíthatók:** `cfa4-mxf` (JPEG 2000, IAB, PCM,
   AES3 essence felismerése), `cfa4-j2k` (Part 1 + HTJ2K dekóder, fejlécelemző, DCI 2K/4K
   kódoló), `cfa4-iab` (ST 2098-2 elemző, renderelő, író), `cfa4-audio-dsp` (BS.1770 / EBU R128:
   integrált hangosság, LRA, true peak), `cfa4-time` (pontos `Tick`, racionális `FrameRate`),
   `cfa4-text` (`skrifa` betűkészlet-olvasás → glyph-lefedettség).
3. **Hiányzik a DCP-validációhoz:** titkosított MXF (ST 429-6) felismerése, MCA-alleírók
   (ST 377-4), timed-text MXF (ST 429-5), Interop DCSubtitle és SMPTE ST 428-7 feliratelemző,
   XML-aláírás (C14N, XMLDSig, X.509), SHA-1 (a lockfile-ban csak `sha2` van), XSD-validátor.
4. **Átfedés nyitott PR-okkal, amelyekhez igazodni kell:**
   - **Media QC** – #255 (terv, a tulajdonos jóváhagyta) és #302 (`cfa4-qc` crate: jelentésséma,
     verziózott TOML-profilok, verdiktszabályok, streamelt SHA-256). A DCP-jelentés és -profil
     ugyanazt a mintát kövesse (6.4 pont).
   - **#103** – JPEG 2000 MXF-import (DCP-képsávok is), `crates/codecs/src/j2k.rs`; nyitott.
   - **#101** – DCP-csomagimport (ASSETMAP/PKL/CPL → idővonal); a DCP-02 adatmodell ezt is
     kiszolgálja.
   - **#249** – `cfa4-ffmpeg` háttérmotor (ADR 0005); a natív DCP-validációnak nem kell.
5. **A tervvel ütköző vagy pontosítandó pontok** (a `VISION.md` szerint jelezni kell, nem
   csendben feloldani):
   - **Qt6-felület = PySide6 (Python).** A CFA4 Qt6 GUI-ja `python/cfa4_qt/`, a Rust-motorral a
     `cfa4-cli serve` JSON-protokollon beszél (`docs/qt-gui.md`). A DCP Validation panel ezért
     Python/PySide6 panel lesz, a logika a Rust-motorban; a terv „Rust primary” elve így teljesül.
   - **FFmpeg a loudnesshez.** A natív CFA4-útnak nem kell FFmpeg: a `cfa4-audio-dsp` R128-mérője
     megvan. Az FFmpeg 9.0.2 ADR 0005 szerint csak tesztorákulum és opcionális háttér; a
     dcp_inspect referenciamotor saját `ffmpeg ebur128` hívásai a CFA4 rögzített 9.0.2-es
     orákulumára mutassanak (lásd [`docs/development/dcp-validation.md`](../development/dcp-validation.md)).
   - **Licenc.** A CFA4 GPL-3.0-or-later, ezért a dcp_inspect GPL-3.0-or-later kódja
     **kompatibilis**; a terv 15. pontjában jelzett GPL-kockázat nagyrészt megszűnik (DCP-01).
   - **Crate-nevek.** A tervben szereplő `dcp-model`, `dcp-validation`, `dcp-inspect-adapter`,
     `dcp-report` helyett a workspace-konvenció szerinti nevek és rétegek (6. pont).
   - **CI.** Az `AGENTS.md` szerint az automatikus PR-ellenőrzés gyors és nem fordít; a teljes
     Rust/FFmpeg-teszt csak `workflow_dispatch`-csel vagy a `cfa4-full-validation` címkével fut.
     A terv 13. pontjának külön DCP-workflow-ja is így indulhat.
   - **Elavult dokumentum:** a [`docs/cinema-codecs.md`](../cinema-codecs.md) még „none”-t ír a
     JPEG 2000-re, pedig a `cfa4-j2k` dekódol és kódol. A javítás nem ennek a PR-nak a témája.

## 2. Modulonkénti leltár

| Terület | CFA4-modul | Ami megvan (bizonyíték) | Ami hiányzik a DCP-validációhoz | Újrahasznosítás |
|---|---|---|---|---|
| MXF | `crates/mxf` (L0, `cfa4-mxf`) | ST 377-1 demux: partíciók, primer, header metadata, indextáblák, OP1a / OP-Atom; `Codec::Jpeg2000`, `Codec::Iab`, `SoundFormat::{Pcm, Aes3Element, Iab}`; RGBA- és JPEG 2000-alleíró (`meta.rs` 0x29, 0x5A), IAB essence descriptor (0x7B); `EssenceTrack` mintatáblával, `Rational`, `Ul`, kezdő timecode (`src/lib.rs`, `essence.rs`, `meta.rs`) | ST 429-6 titkosított essence (Cryptographic Framework / Context) felismerése és `KeyId`; ST 377-4 MCA-alleírók (csatornacímkék); ST 429-5 timed-text track file; a fájl- és package-UUID-k és a CPL-hez kötendő `AssetUUID` kiolvasása publikus API-n; JPEG 2000-író (a writer VC-3 / ProRes / AVC / PCM) | Közvetlenül: DCP-F06 a `cfa4_mxf::open` eredményére épül. A hiányzó elemek a `cfa4-mxf` bővítései, nem új MXF-elemző |
| JPEG 2000 | `crates/j2k` (L0) | Part 1 + HTJ2K dekóder; `parse_header` (SIZ, komponensek, dekódolás nélkül); `EncodeOptions::cinema_2k/4k`; a #103 hozzáadja a `capabilities` (Rsiz), `mct`, `digital_cinema()` mezőket | Cinema-profil szabályellenőrző (ST 429-4 / ISO 15444-1 Annex A.10 korlátai), tile-part / TLM / PLT-hosszak szerkezeti bejárása teljes dekódolás nélkül | DCP-F07 a `parse_header`-re és a codestream-markerek bejárására épül; a pixel-dekódolás külön (terv F07 megjegyzése) |
| IAB | `crates/iab` (L0) | ST 2098-2 `parse` (IAFrame, BedDefinition, ObjectDefinition, AudioData PCM/DLC), `render`, `write`, `split` | ST 429-18 alkalmazásprofil-szabályok; jelentés a nem ellenőrzött részekről | DCP-F11 közvetlenül |
| Hang / loudness | `crates/audio-dsp` (L1) | `LoudnessMeter`: K-szűrés, integrált LUFS, rövid távú és pillanatnyi, LRA (EBU Tech 3342), sample peak és true peak (`loudness.rs`); csatornasúlyok | Mozis csatornakiosztás szerinti súlyozás (LFE, HI/VI kizárása), reel-sorrendű streamelt mérés | DCP-F10 natívan, FFmpeg nélkül |
| PCM | `cfa4-mxf::decode_pcm`, `crates/codecs/src/audio.rs` | 24-bit LE PCM és AES3 dekódolás | Csatornaszám / MCA egyeztetése a CPL-lel | DCP-F09 |
| Feliratok | `crates/captions` (L2) | SRT, WebVTT, SCC, MCC, EBU STL, TTML/IMSC1/DFXP (`roxmltree`), elrendezés, beégetés | **Interop DCSubtitle** és **SMPTE ST 428-7 DCST** elemző nincs; PNG-alapú feliratok; fade-időzítés DCP-szemantikával | Az XML-alapozás (`roxmltree`) és a beégető (renderelt ellenőrzéshez) újrahasznosítható; az elemző új modul |
| Betűkészlet | `crates/text` (L1) | `skrifa` 0.44 font-olvasás | Glyph-lefedettségi lekérdezés a felirat szövegére | DCP-F12 „szükséges glyph-ek” |
| Idő | `crates/time` (L0) | `Tick` (254 016 000 000/s), `FrameRate`, `Tick::from_rational`, `mul_ratio` | – | Reel- és kompozícióidő pontos számítása (DCP-F04, F16) |
| Idővonal / markerek | `crates/project` (`Marker`), `crates/engine/src/marker_export.rs` | Projektmarkerek | CPL `MarkerList` ↔ CFA4 marker megfeleltetés | DCP-F16 |
| Export | `crates/export` (L3) | OP1a / OP-Atom MXF-export (VC-3, ProRes, AVC, PCM), feladatkezelés, haladás, megszakítás | **DCP-export nincs** (J2K-MXF író, CPL/PKL/ASSETMAP-író, aláírás) | DCP-F20 csak integrációs pont; a DCP-export külön képesség (a terv 1. pontja szerint is) |
| Kodek-hub | `crates/codecs` (L2) | `MxfSource`; a #103 bekötné a J2K-képsávot | DCP-mappa mint forrás (#101) | A DCP-02 modell a #101-et is kiszolgálja |
| Médiaböngésző | `crates/engine/src/media_browser.rs`, `python/cfa4_qt/panels/media_browser.py` | Mappák böngészése, metaadat | DCP-mappa felismerése | DCP-F01 kijelzése a Qt-ben |
| Háttérfeladatok | `crates/engine` (`jobs.list`, `jobs.cancel`), `crates/media/src/cancel.rs` | Megszakítható háttérfeladatok, haladás | – | `ValidationControl` erre épül; a Qt-szál nem blokkol |
| Útválasztó | `crates/route` (L1, ADR 0004) | Backend-katalógus és -választás | – | Nem kell: a DCP-validátor-backendek választása a profil dolga, nem számítási útvonal. Ha a tulajdonos egységességet akar, később regisztrálható |
| XML | `roxmltree` 0.21 (workspace), `quick-xml` 0.41 (lockfile) | Olvasás a feliratokban és az interchange-ben | Kanonizálás (C14N 1.0) az aláírásellenőrzéshez; XSD | Az XML-modell `roxmltree`-re épüljön (entitás- és mélységkorlátos, külső entitást nem tölt) |
| Hash | `sha2` 0.10.9 (lockfile); `crates/qc/src/hash.rs` (#302, streamelt, megszakítható SHA-256) | SHA-256 | **SHA-1** (a PKL-hash SHA-1, base64) | A #302 hash-mintája SHA-1-re is |
| Kriptográfia | – | – | RSA PKCS#1 v1.5 ellenőrzés (SHA-1 / SHA-256), X.509-elemzés, tanúsítványlánc | Új függőség, DCP-10 (6.5 pont, 7. pont D4) |
| CLI | `apps/cfa4-cli` | `serve`, export, codecs | `cfa4-cli dcp …` | DCP-F18/F19 CLI-paritás |
| Qt6 | `python/cfa4_qt/` (PySide6), motor: `cfa4-cli serve` | Panelek, motorkliens (`engine.py`), nem blokkoló olvasószál | DCP Validation panel | DCP-12 |
| QC | `crates/qc` (#302, még nincs a `main`-ben) | `cfa4-qc-report/1`, TOML-profil SHA-256-tal, `Pass/Warn/Fail/NotRun`, verdikt | DCP-specifikus állapotok (7 állapot) | Séma- és profilminta (6.4 pont) |

## 3. A funkciókatalógus (DCP-F01…F21) lefedettsége ma

Jelölés: **Alap** = a mérő/elemző megvan, csak DCP-szabály kell; **Részleges** = egy része
megvan; **Nincs** = új kód kell.

| ID | Terület | Állapot | Mire épül / mi hiányzik |
|---|---|---|---|
| F01 | Felfedezés | Nincs | `walkdir` a lockfile-ban; symlink- és útvonalvédelem új |
| F02 | ASSETMAP | Nincs | DCP-02 modell + `roxmltree` |
| F03 | PKL | Nincs | DCP-02 + SHA-1 (új) + streamelt hash (#302 minta) |
| F04 | CPL | Nincs | DCP-02 + `cfa4-time` |
| F05 | SMPTE/Interop profil | Nincs | névtér alapján (DCP-02) + profil-TOML |
| F06 | MXF | Részleges | `cfa4-mxf`; hiányzik ST 429-6, MCA, ST 429-5, AssetUUID |
| F07 | JPEG 2000 | Részleges | `cfa4-j2k::parse_header`; hiányzik a cinema-profil szabálykészlet és a tile-part bejárás |
| F08 | Képformátum, frame rate | Részleges | `PictureInfo` + `FrameRate`; szabálytábla új |
| F09 | Hang | Részleges | PCM-dekódolás megvan; MCA hiányzik |
| F10 | Loudness | Alap | `cfa4-audio-dsp::LoudnessMeter` |
| F11 | IAB | Alap | `cfa4-iab::parse` |
| F12 | Feliratok | Részleges | `roxmltree`, `skrifa`, PNG (`image`/`png` a `cfa4-media`-ban); DCSubtitle / ST 428-7 elemző új |
| F13 | ISDCF | Nincs | regiszterforrás tisztázandó (DCP-01 4. pont) |
| F14 | Aláírások | Nincs | C14N + RSA + X.509 új |
| F15 | KDM (1. fázis) | Nincs | ST 429-6 felismerés a `cfa4-mxf`-ben + CPL `KeyId` |
| F16 | Markerek | Részleges | `Marker` a projektben; CPL-oldal új |
| F17 | Integritás | Nincs | F02–F04 + hash |
| F18 | Jelentés | Részleges | `cfa4-qc` séma-minta (#302) |
| F19 | Qt6 GUI | Nincs | PySide6 panel + motorparancsok |
| F20 | Exportintegráció | Nincs | nincs DCP-export; csak az integrációs pont készülhet el |
| F21 | Javítási műveletek | Nincs | `engine` parancsok; csak projektadat / jóváhagyott munkapéldány |

## 4. Workspace-konvenciók, amelyeket az új crate-eknek követniük kell

- **Név és hely:** `crates/<dir>`, csomagnév `cfa4-<dir>` (`docs/architecture.md` §1).
- **Réteg:** minden crate szerepeljen az `xtask/src/main.rs` `LAYERS` táblájában és a
  `docs/architecture.md` §1 táblájában; a `cargo xtask layers` ezt ellenőrzi. L0 kodek/konténer
  crate belső függősége csak `cfa4-bitstream` lehet; azonos rétegű élhez `SAME_LAYER`-sor kell.
- **L0–L4 crate-nek `wasm32-unknown-unknown`-ra is fordulnia kell** (`cargo xtask wasm`).
- **Soha ne omoljon össze** (`AGENTS.md` §0): `#![cfg_attr(not(test), deny(clippy::unwrap_used, …))]`,
  `unsafe_code = "forbid"`, minden bemeneti szám ellenséges, korlátos allokáció és rekurzió,
  feladat `catch_unwind` alatt; elemzőkhöz mutációs fuzz-teszt.
- **Specifikáció:** a crate README-je rögzíti a használt szabványkiadást (SMPTE ST 429-x stb.).
- **Átvett kód:** eredeti fejléc + sor a `docs/licensing.md` §5 regiszterébe, ugyanabban a commitban.
- **Függőség:** `cargo deny check licenses` (`deny.toml`); közös fájlban (`Cargo.toml`,
  `Cargo.lock`) csak saját sor (`docs/coordination.md` §2.5).
- **Fixture:** nem kerül a repóba; generálva `target/fixtures/`-be, vagy rögzített commit +
  sha256 alapján letöltve.
- **Minden parancs:** az `engine` parancsazonosítóin át érhető el UI-ból, CLI-ből, control
  channelről és MCP-ről.

## 5. Mit **nem** szabad duplikálni

- MXF-elemzést: a `cfa4-mxf` bővül, nincs második MXF-olvasó.
- JPEG 2000-fejlécelemzést: a `cfa4-j2k` API-ja bővül.
- Loudness-mérést: a `cfa4-audio-dsp` mérője; FFmpeg csak a referenciamotorban és orákulumként.
- IAB-elemzést: `cfa4-iab`.
- Hash- és jelentésmintát: igazodás a `cfa4-qc`-hez (#302), ha az bekerül.

## 6. Crate-elrendezési javaslat a DCP-02 és DCP-03 munkacsomaghoz

### 6.1. Megfeleltetés a terv neveivel

| Terv (3.2) | Javasolt crate | Réteg | Munkacsomag | Indok |
|---|---|---|---|---|
| `crates/dcp-model/` | `crates/dcp` → **`cfa4-dcp`** | **L0** (konténer, mint az `mxf`) | DCP-02 | A DCP-csomag konténerformátum. Tisztán adat + XML-olvasás, wasm-kompatibilis; a #101 (import) is használja |
| `crates/dcp-validation/` | `crates/dcp-validate` → **`cfa4-dcp-validate`** | **L3** (mint `export`, `qc`) | DCP-03, majd DCP-05…11 | Több L0–L1 crate-et használ (`mxf`, `j2k`, `iab`, `audio-dsp`, `time`, `text`) |
| `crates/dcp-report/` | a `cfa4-dcp-validate` `report` modulja | L3 | DCP-15 | A jelentésséma maga az API; külön crate csak akkor, ha egy fogyasztó validáció nélkül akarja (D3) |
| `crates/dcp-inspect-adapter/` | `crates/dcp-inspect` → **`cfa4-dcp-inspect`** | L3 (azonos rétegű él → `dcp-validate`) | DCP-04 | Csak `std::process`; nincs Ruby-függőség a Rustban vagy a GUI-ban; ha a motor nincs telepítve, a backend „nem elérhető” |

Ehhez az `xtask` `LAYERS`-be: `("dcp", 0)`, `("dcp-validate", 3)`, `("dcp-inspect", 3)`;
`SAME_LAYER`-be: `("dcp-inspect", "dcp-validate")`, és ha a QC-sémát közvetlenül használja:
`("dcp-validate", "qc")`. Mindhárom bekerül a `docs/architecture.md` §1 táblájába.

### 6.2. `cfa4-dcp` (DCP-02) tartalma

```text
crates/dcp/
  Cargo.toml        # deps: roxmltree, serde, thiserror (mind workspace)
  README.md         # SMPTE ST 429-7/-8/-9, ST 429-10, Interop (PROTO-ASDCP) kiadásokkal
  src/lib.rs        # Error, Standard { Smpte, Interop }, Uuid, Hash(Sha1Base64), Rational
  src/assetmap.rs   # AssetMap, Asset, Chunk (VolumeIndex/Offset/Length is beolvasva)
  src/pkl.rs        # PackingList, PklAsset { id, hash, size, type, original_file_name }
  src/cpl.rs        # Composition, Reel, MainPicture/MainSound/MainSubtitle/AuxData, Marker, KeyId
  src/signature.rs  # csak beolvasás: Signer, ds:Signature nyers csomópontok (ellenőrzés: DCP-10)
  src/xml.rs        # közös: névtér-azonosítás, méret- és mélységkorlát, forráspozíció (sor:oszlop)
  src/tests.rs      # szintetikus XML, csonkítás / bitflip fuzz `catch_unwind` alatt
```

- Saját egzakt `Rational` (i64/i64), mert L0 crate nem függhet a `cfa4-time`-tól; a `Tick`-re
  váltás az L3-ban.
- Minden elem megőrzi a forráshelyét (fájl, sor, oszlop), mert a terv szerint minden hibánál
  meg kell jelennie a pontos helynek.
- Ismeretlen elemek és kiterjesztések nem vesznek el: `unknown: Vec<…>` mező, hogy a validátor
  „nem támogatott kiterjesztés” állapotot adhasson (F05).
- Csak olvas; DCP-írás (export) később ugyanebbe a crate-be kerülhet.

### 6.3. `cfa4-dcp-validate` (DCP-03) API-ja

A terv 3.3-as interfésze, CFA4-hez igazítva:

```rust
pub trait DcpValidator: Send + Sync {
    fn id(&self) -> &'static str;                 // "native", "dcp-inspect"
    fn capabilities(&self) -> ValidatorCapabilities; // támogatott szabályazonosítók, verzió
    fn validate(&self, request: &ValidationRequest, control: &ValidationControl)
        -> Result<ValidationReport, ValidationError>;
}

pub struct ValidationRequest { pub root: PathBuf, pub profile: Profile, pub limits: Limits }
pub struct ValidationControl { cancel: Arc<AtomicBool>, progress: Box<dyn Fn(Progress) + Send + Sync> }

pub enum Status { Pass, Fail, Warning, Skipped, Unchecked, NotApplicable, Interrupted }
pub struct Finding { pub rule: RuleId, pub status: Status, pub location: Location,
                     pub evidence: Evidence, pub message: String, pub remedy: Option<Remedy> }
```

- A `validate` szinkron; az `engine` háttérfeladatként, `catch_unwind` alatt futtatja
  (`jobs.list` / `jobs.cancel`), így a Qt-szál sosem blokkol.
- **Szabályazonosító:** `DCP.<terület>.<szám>` (pl. `DCP.PKL.004`), profilonként verziózott.
- **Összesítés** külön mező: `PASS` csak ha a profil minden kötelező szabálya `Pass` vagy
  `NotApplicable`; bármely kötelező `Fail` → `FAIL`; kötelező `Skipped`/`Unchecked` → `INCOMPLETE`;
  megszakítás → `INTERRUPTED`. A `Quick Scan` ezért sosem ad „teljes integritás” eredményt.
- **Jelentés:** `cfa4-dcp-report/1` JSON (+ Markdown, HTML renderelő), benne profil és annak
  SHA-256-ja, backend és verzió, szabványkiadások, ASSETMAP/PKL/CPL-azonosítók, időpont.
- **Profilok:** `profiles/*.toml` (`cfa4-dcp-profile/1`), a terv 5. pontjának 8 profiljával.
- A natív szabálymodulok (`rules/assetmap.rs`, `pkl.rs`, `cpl.rs`, `mxf.rs`, …) a DCP-05-től.

### 6.4. Igazodás a Media QC-hez (#255, #302)

A `cfa4-qc` ugyanazt a feladatot oldja meg általános médiára: verziózott TOML-profil, a profil
SHA-256-ja a jelentésben, `NotRun` sosem összesít `Pass`-ra, kézzel átírt jelentés elutasítása.
Javaslat: a DCP-jelentés **saját sémát** kap (a 7 állapot és a csomag/kompozíció/reel-helyek
miatt), de a mezőnevek, a profilformátum és a verdiktszabály a `cfa4-qc`-t követi, és a DCP-
eredmény leképezhető QC-eredményre (`Pass→Pass`, `Warning→Warn`, `Fail→Fail`,
`Skipped/Unchecked/Interrupted→NotRun` okkal, `NotApplicable` kimarad). Így az export utáni
QC-állapot (F20) egy helyen látszik.

### 6.5. Rust-függőségek (javaslat; a `Cargo.toml`-ba csak az első felhasználó crate-tel kerülnek)

| Csomag | Verzió | Licenc | Állapot a CFA4-ben | Mire | Munkacsomag |
|---|---|---|---|---|---|
| `roxmltree` | 0.21 | MIT OR Apache-2.0 | workspace-függőség | XML-olvasás | DCP-02 |
| `serde`, `serde_json` | 1 | MIT OR Apache-2.0 | workspace | modell és jelentés | DCP-02/03 |
| `thiserror` | 2 | MIT OR Apache-2.0 | workspace | hibatípusok | DCP-02 |
| `toml` | 1 | MIT OR Apache-2.0 | `cfa4-route` használja | profilok | DCP-03 |
| `sha2` | 0.10 | MIT OR Apache-2.0 | lockfile | profil- és jelentés-hash | DCP-03 |
| `sha1` | 0.10.x (a `sha2` 0.10-zel közös `digest` 0.10 miatt) | MIT OR Apache-2.0 | **új** | PKL-hash | DCP-05 |
| `base64` | 0.22 | MIT OR Apache-2.0 | lockfile | PKL-hash, aláírásérték | DCP-05 |
| `rsa` | 0.9 (vagy a 0.10 stabil, ha megjelenik) | MIT OR Apache-2.0 | **új** | RSA PKCS#1 v1.5 ellenőrzés | DCP-10, D4 |
| `x509-cert` | 0.2 (`der`, `spki`) | Apache-2.0 OR MIT | **új** | tanúsítványok | DCP-10, D4 |

- Az `rsa` 0.9-re a RUSTSEC-2023-0071 (Marvin, időzítéses oldalcsatorna) vonatkozik. Ez a
  privát kulcsos műveleteket érinti; a CFA4 csak nyilvános kulccsal **ellenőriz**, de a
  `cargo deny` jelezni fogja, ezért indokolt `deny.toml`-kivétel kell (D4).
- A `ring` 0.17 már a lockfile-ban van (rustls útján), de C/assembly kódot fordít, és a
  `wasm32-unknown-unknown` fordítást (`cargo xtask wasm`, L0–L4) megnehezíti; ezért nem javasolt.
- **C14N 1.0** (a DCP-aláírás kanonizálása): nincs karbantartott, tisztán Rust crate; saját,
  W3C-specifikáció szerinti megvalósítás a `roxmltree`-fán (DCP-10).
- **XSD:** nincs érett, tisztán Rust XSD 1.0-validátor. Javaslat: a natív motor a szabványok
  szerkezeti szabályait Rust-kódként ellenőrzi, és amit nem modellez, azt `Unchecked`-nek jelenti;
  teljes XSD-validáció a referenciamotorban (Nokogiri/libxml2) marad (D5).

## 7. Döntést igénylő pontok (javaslattal)

| # | Kérdés | Javaslat |
|---|---|---|
| D1 | Crate-elrendezés | A 6.1 szerinti három crate (`cfa4-dcp` L0, `cfa4-dcp-validate` L3, `cfa4-dcp-inspect` L3) |
| D2 | A DCP-02 modell az importot (#101) is kiszolgálja | Igen: egy DCP-modell az importhoz és a validációhoz |
| D3 | Külön `dcp-report` crate | Nem, a `cfa4-dcp-validate` modulja; később kiemelhető |
| D4 | Kriptográfia | RustCrypto (`rsa`, `x509-cert`, `sha1`) + indokolt `deny.toml`-kivétel; saját C14N |
| D5 | XSD a natív motorban | Szerkezeti szabályok Rustban + `Unchecked`; XSD-validáció csak a referenciamotorban |
| D6 | Igazodás a `cfa4-qc`-hez | Saját DCP-séma, QC-kompatibilis mezőkkel és leképezéssel (6.4) |
| D7 | dcp_inspect logikájának Rustba portolása | Szabványból írt natív kód; a dcp_inspect fekete dobozos orákulum. Konkrét átvétel csak regiszterbejegyzéssel (DCP-01 5. pont) |

## 8. Következő lépés

A D1–D7 jóváhagyása után: DCP-02 (`cfa4-dcp`) PR, utána DCP-03 (`cfa4-dcp-validate` váz: API,
állapotok, profil- és jelentésséma), majd DCP-04 (`cfa4-dcp-inspect`). A tesztadatok (DCP-16)
már a DCP-02-vel indulnak: szintetikus XML és a meglévő J2K/PCM MXF-íróval gyártott csomagok
`target/fixtures/`-be, a repóba nem kerül médium.
