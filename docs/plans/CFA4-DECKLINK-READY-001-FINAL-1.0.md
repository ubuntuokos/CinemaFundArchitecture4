# CFA4-DECKLINK-READY-001 – A DeckLink beilleszthetőségének előkészítése (V1–V11), FINAL 1.0

> Forrás: ez a terv eredetileg online Claude-dokumentumként készült (https://claude.ai/artifact/VK2j3jZ5zW27KHKdVVUd2j); a CFA4 szabálya szerint („minden tervnek a repóban a helye”) 2026-11-15-én átkerült ide, tartalma változatlan. Ettől kezdve ez a fájl az irányadó.

2026-11-14 · Horváth H. István

**Állapot:** FINAL 1.0, István 2026-11-15-én jóváhagyta, az R1–R4 döntéseket a javaslatok szerint. A jóváhagyás a 3. fejezet C típusú tervkiegészítéseit is magában foglalja (V1, V3, V4.1–V4.2, V7, V9): az érintett projekt munkamenete a saját tervébe átvezeti a szöveget, és a hullámok szerint indulhat. A [`coordination.md`](https://github.com/ubuntuokos/CinemaFundArchitecture4/blob/main/docs/coordination.md) §2.10 szerint ez a jóváhagyás a fejlesztés feltétele; maga a DeckLink-fejlesztés továbbra sem indul. **Szülőterv:** [CFA4-DECKLINK-001 FINAL 1.0](CFA4-DECKLINK-001-FINAL-1.0.md), 9. fejezet (V1–V11). **Alap:** `main` 0d598d11 és a 2026-11-14-én nyitott PR-ok.

## 1. Összefoglaló

A DeckLink-fejlesztés nem indul. Ez a terv megmondja a **most futó projektek fejlesztőinek**, mit kell tenniük, hogy a DeckLink-terület később átírás és duplázás nélkül beilleszthető legyen. Minden munkacsomag a saját projektjében fut, annak munkamenete végzi, a saját PR-jában; a DeckLink-terület nem ír kódot más projektbe.

A kiegészítő kódfelmérés két helyen pontosítja a szülőterv 9. fejezetét:

- **V5 (PROCODEC) könnyebb, mint gondoltuk.** A `cfa4-frame` már ismeri a 10–16 bites YUV-t és RGB-t (`PixelData::Yuv16`, `Rgb16`, `crates/frame/src/lib.rs`), a ProRes- és DNx-kódoló pedig síkokra bontott 16 bites YUV-t kap (`cfa4_prores::Frame`, `cfa4_dnx::Frame`); az RGBA→YUV átalakítás csak az export-csőben van. Tervkiegészítés nem kell, csak egy megőrzendő tulajdonság és egy teszt.
- **V9 (File Services) konkrét hiánya:** a terv írási útja mindig másol (staging → átnevezés). Egy 10 Gbit/s-os felvételt nem lehet utólag átmásolni, ezért kell egy „helyben véglegesítés” művelet. Ez C típusú kiegészítés.

**Becslés összesen:** 84–133 óra, hét projektben. Hardver nem kell hozzá, minden CI-ben tesztelhető.

### 1.1 Döntések (jóváhagyva 2026-11-15, a javaslat szerint)

| # | Kérdés | Javaslat | Ha nem |
| --- | --- | --- | --- |
| R1 | Hová kerüljön a PTP-olvasó (V1) és a képkockagyűrű (V2), hogy a stúdióhost és a DeckLink io-host is használhassa? | Új, közös L1 crate: `cfa4-io-core` (`ptp` és `ring` modul, csak std és `cfa4-st2110`); a `cfa4-obs` újraexportálja, a stúdió viselkedése nem változik | az io-host a `cfa4-obs`-tól függne, és vele a libobs-betöltőtől; vagy a kód kétszer készülne |
| R2 | Az S5 felvétele és a DeckLink-felvétel: egy rögzítő (D5) hogyan teljesül, ha a stúdió a libobs kódolóit használja? | Egy közös **rögzítési szerződés** (`RecordJob`, `RecordReport`, fájlnév, véglegesítés a File Servicesben), két háttérrel: libobs (stúdióprogram, S5) és natív (DeckLink). A szerződés az S5 indulásakor készül | ha a szigorú olvasat kell (csak natív rögzítő), az S5 csak a `cfa4-ingest` után indulhat, ami a LIVE-tervet hónapokkal késleltetné |
| R3 | Ki építi a TSL UMD-forrást? | A DeckLink-terület F7.2-je, de a vezérlőfelület-motor L5-forrásaként a `cfa4-surface`-ben, az F-OSC mintájára. Most (V4) csak a modell bővíthetősége és a tally-visszajelzés készül | a vezérlőfelület-projekt készíti; akkor az ő tervébe kerül egy új fázis |
| R4 | Hogyan véglegesít egy felvételt a File Services másolás nélkül? | `files.stage.begin` / `commit` / `abort`: a rögzítő a gyökér `staging` könyvtárába ír, végén `fsync`, kivonat, atomikus átnevezés, katalógusrekord. A BLAKE3-at a rögzítő írás közben számolja, a File Services mintavételesen ellenőrzi | a felvétel a katalóguson kívül marad, és csak utólagos `files.import`-tal (dupla lemezterület, dupla idő) kerül be |

## 2. Közös szabályok a fejlesztőknek

1. **Csak jóváhagyás után.** Egy munkacsomag akkor indul, ha István ezt a tervet jóváhagyta, és a C típusú csomagnál az érintett projekt terve is módosult (§2.10). A tervmódosítást az érintett projekt munkamenete írja meg, ennek a tervnek a szövegével.
2. **Saját projekt, saját PR.** Minden csomag a 3. fejezetben megnevezett projekt munkamenetéé. Egy PR egy feladatazonosítót visz (pl. `READY-V7.2: ioHost sandbox profile`), a projekt saját azonosítója mellett.
3. **Csak additív változás.** Meglévő viselkedés, protokoll és fájlformátum nem törik. Ahol verziószám van (gyűrű, adapterprotokoll), ott a verzió nő, és a régit a teszt bizonyítja vagy kezelhető hibával utasítja el.
4. **Nincs DeckLink-kód.** Semmi nem hivatkozik a DeckLink SDK-ra vagy a `libDeckLinkAPI.so`-ra, és nem készül „előre” üres DeckLink-változat. Csak általános bővítési pont készül, amelyet egy valódi, már létező használó (stúdió, vezérlőfelület, sandbox, File Services) is használ, vagy teszt fed le.
5. **CFA4-szabályok:** never crash (`AGENTS.md` §0), rétegek (`cargo xtask layers`), `unsafe` csak FFI-modulban, minden idő `Tick` vagy monoton ns, minden művelet parancs. Minőségi kapuk a PR előtt: `cargo xtask ci` és `cargo test --workspace --exclude cfa4-ui --release` (a `cfa4-ci-runner` által).
6. **Átvétel.** Minden csomag átvételi tesztjei a 3. fejezetben vannak. A csomag akkor kész, ha ezek zöldek a CI-ben, és a PR leírása hivatkozik erre a tervre.
7. **Átfedés.** Ha egy csomag más projekt kódjához érne, a munkamenet megáll, és jelzi a koordinátornak (§2.8).

## 3. Munkacsomagok

Minden csomagnál: ki végzi, kiinduló állapot (fájl és PR), feladatok, interfész, átvételi tesztek, órabecslés. A típus a szülőterv 9.1 pontja szerinti: **A** = nincs teendő, **B** = megőrzendő tulajdonság a projekt saját tervén belül, **C** = tervkiegészítés István jóváhagyásával.

### V1 – Közös PTP-állapot (C, 8–12 óra)

**Végzi:** LIVE-STUDIO-001 ST 2110 kiegészítés, T1 munkamenete (#1128, ág `cfa4/live-studio-st2110-t1`).

**Kiinduló állapot:** a #1128 a `crates/obs/src/ptp.rs`-ben olvassa a `ptp4l` állapotát a `pmc` futtatásával (`PtpReport`, `report_from`, `find_pmc`, `run_pmc`, `local_mac`, `query`), a `cfa4_st2110::clock` típusaira (`PtpStatus`, `PtpTime`) építve. A `serve_live.rs` legfeljebb másodpercenként kérdezi le (`PTP_REFRESH`). A kód nem függ a libobs-tól, csak rossz helyen van: a `cfa4-obs` (L5) a libobs-betöltőt is hozza.

| ID | Feladat | Eredmény | Óra |
| --- | --- | --- | --- |
| V1.1 | Új crate: `crates/io-core` (`cfa4-io-core`, L1, függőség: `cfa4-st2110`, `serde`, `log`, `thiserror`); `cargo xtask layers` és `docs/architecture.md` sor (R1) | üres crate a rétegtáblában | 1–2 |
| V1.2 | A `ptp.rs` áthelyezése `cfa4_io_core::ptp` néven, változatlan API-val; a `cfa4-obs` `pub use cfa4_io_core::ptp;` sorral újraexportálja | a stúdió és a `serve_live.rs` változatlanul fordít | 2–3 |
| V1.3 | A `PtpReport` stúdiófüggetlen: nincs benne host- vagy jelenetmező; a `query()` paraméterezhető domainnel és időkorláttal (`query_with(domain, timeout)`), a régi `query()` az ST 2059 alapértékével hívja | két hívó ugyanazt a kódot használja | 2–3 |
| V1.4 | A tesztek átköltöztetése és egy új teszt: `pmc` hiánya, időtúllépés és sérült kimenet kezelhető `PtpReport` (nem pánik) | zöld tesztek | 2–3 |
| V1.5 | README a crate-ben; a #1128 leírása hivatkozik erre a csomagra | dokumentáció | 1 |

**Interfész (változatlan, csak új helyen):**

```rust
// cfa4_io_core::ptp
pub const ST2059_DEFAULT_DOMAIN: u8 = 127;
pub fn query() -> PtpReport;                                   // ST 2059 domain, 2 s
pub fn query_with(domain: u8, timeout: Duration) -> PtpReport;  // új
pub fn report_from(pmc: Result<String, String>, now: SystemTime, mac: [u8; 6]) -> PtpReport;
```

**Átvétel:** (1) `cargo xtask layers` zöld, a `cfa4-io-core` nem függ L1 feletti crate-től; (2) a `live.studio.host.status` kimenete bájtra azonos a változtatás előttiével ugyanarra a `pmc`-kimenetre (meglévő teszt); (3) `pmc` nélküli gépen a `query()` hibát tartalmazó jelentést ad, nem pánikol.

**Függőség:** R1. Ha a #1128 a csomag előtt beolvad, a V1 egy külön, kis PR a T1 munkamenettől.

### V2 – Bővíthető képkockagyűrű és forrástípus (B, 22–35 óra)

**Végzi:** LIVE-STUDIO-001 S0/S1 munkamenete (#1129, #1189 gazdája).

**Kiinduló állapot:**

- A gyűrű (`crates/obs/src/ring.rs`) már formátumkódot hordoz a slotfejlécben, de csak egy formátum létezik (`PixelFormat::Bgra8`, kód 1, `crates/obs/src/lib.rs`), hang és időkód nincs benne. Fejléc: `CFA4LIV1`, `VERSION = 1`, 3 slot, legfeljebb 1 GiB.
- A `SourceKind` (`crates/project/src/studio.rs`, #1189) `tag = "type"` szerinti enum (`color`, `image`, `text`); ismeretlen típusra a serde hibát ad.

| ID | Feladat | Eredmény | Óra |
| --- | --- | --- | --- |
| V2.1 | A gyűrű áthelyezése `cfa4_io_core::ring` néven (R1); a `PixelFormat` és a `Frame` vele megy, a `cfa4-obs` újraexportálja | a stúdió változatlanul működik | 3–5 |
| V2.2 | Gyűrű v2: magic `CFA4RNG2`, `VERSION = 2`. A slotfejléc fenntartott bájtjaiban: `timecode u32` (SMPTE BCD, 0 = nincs), `audio_len u32`, `audio_channels u16`, `audio_format u16`, `flags u32` (pl. mezősorrend). A képadat után opcionális hangblokk. A slotméret a formátumból és a hangkeretből számolódik | verziózott, bővíthető fejléc | 6–9 |
| V2.3 | Új képformátumok a `PixelFormat`-ban: `Yuv422P10` (síkokra bontott 16 bites tárolás, a `cfa4-frame` `Yuv16`-jával egyező), `Rgb12P` (16 bites tárolás), `Uyvy8`. Hozzájuk `bytes_per_frame(w, h)` és átalakítás `cfa4_frame::VideoFrame`-be | a gyűrű 10 és 12 bites képet is visz | 5–8 |
| V2.4 | Hangformátum: 32 bites előjeles egész PCM, csatornasorrendben összefésülve (a szülőterv `CapturedFrame` integer PCM-jével egyező); a stúdió egyelőre nem ír hangot (`audio_len = 0`) | a mező létezik és tesztelt | 2–3 |
| V2.5 | Olvasó: v1-es fájlt kezelhető hibával utasít el (`StudioError::Ring("ring version 1")`), ismeretlen formátumkódnál a képkockát eldobja és számolja, nem pánikol | never crash | 2–3 |
| V2.6 | `SourceKind`: teszt, hogy ismeretlen `type` (pl. `"capture"`) a projekt betöltésénél olvasható hibát ad a forrás nevével, a szerkesztő nem omlik össze, a projekt többi része betölthető marad, ha a projektformátum ezt engedi; ha nem, a hibaüzenet megnevezi a forrást | dokumentált viselkedés | 2–4 |
| V2.7 | `crates/obs/README.md` és az új crate README-je: a gyűrű v2 elrendezése táblázatban | dokumentáció | 2–3 |

**Interfész:**

```rust
// cfa4_io_core::ring (v2)
pub const MAGIC: &[u8; 8] = b"CFA4RNG2";
pub const VERSION: u32 = 2;
#[non_exhaustive]
pub enum PixelFormat { Bgra8 = 1, Uyvy8 = 2, Yuv422P10 = 3, Rgb12P = 4 }
pub struct Frame {
    pub width: u32, pub height: u32, pub stride: u32,
    pub format: PixelFormat, pub pts_ns: u64,
    pub timecode: Option<u32>,         // SMPTE BCD
    pub field_order: FieldOrder,       // cfa4_frame
    pub data: Vec<u8>,
    pub audio: Option<RingAudio>,      // i32 PCM, interleaved
}
pub struct RingAudio { pub channels: u16, pub rate: u32, pub samples: Vec<i32> }
```

**Átvétel:** (1) a stúdió meglévő tesztjei zöldek, az előnézet változatlan (`--smoke --screenshot` képe egyezik); (2) új körteszt minden formátumra: írás → olvasás → bájtazonos kép és hang; (3) fordított sorrendű, túlírt slotot az olvasó eldobja (meglévő teszt v2-re); (4) a `fuzz` cél a slotfejléc-olvasóra (a SECURITY-001 mintájára) 10 percig hiba nélkül fut; (5) 2160p50 `Yuv422P10` képkocka írása és olvasása a referenciagépen 4 ms alatt (mérés, nem kapu).

**Függőség:** R1, V1.1 (a crate).

### V3 – Közös rögzítési szerződés (C, 4–6 óra most, a kód az S5-ben)

**Végzi:** LIVE-STUDIO-001 munkamenete, a terv módosításaként; a kód az S5 része lesz.

**Kiinduló állapot:** a LIVE-terv V. fejezete szerint a `cfa4-studio-host` végzi a „kódolás, felvétel, stream” feladatot libobs-szal; az S5 (MKV/MP4/fMP4/MOV, szétválasztás, fejezetjelek) még nem indult. A szülőterv D5-je egy rögzítőt kér.

| ID | Feladat | Eredmény | Óra |
| --- | --- | --- | --- |
| V3.1 | A LIVE-terv S5 sorának kiegészítése: „a felvétel a közös rögzítési szerződésen át történik (R2); a libobs ennek egyik háttere”, és egy új átvételi feltétel: a felvételről `RecordReport` készül, a fájl a File Servicesben véglegesítődik (V9) | tervmódosítás István jóváhagyására | 2–3 |
| V3.2 | A szerződés leírása a LIVE-terv mellékletében (lent), hogy az S5 és a későbbi `cfa4-ingest` ugyanazt valósítsa meg | szerződésvázlat | 2–3 |

**A szerződés (vázlat, a kód az S5-ben készül, L3):**

```rust
pub struct RecordJob {
    pub target: StagedFile,          // V9: files.stage.begin eredménye
    pub container: Container,        // Mkv, Mp4, FragmentedMp4, Mov, Mxf
    pub video: Vec<VideoTrackSpec>,  // kódoló, felbontás, képkocka-sebesség (Tick)
    pub audio: Vec<AudioTrackSpec>,
    pub split: Option<SplitRule>,    // méret vagy idő szerint
    pub start_timecode: Option<Timecode>,
}
pub trait Recorder: Send {
    fn start(&mut self, job: RecordJob) -> Result<RecordHandle, RecordError>;
    fn marker(&mut self, h: &RecordHandle, at: Tick, label: &str) -> Result<(), RecordError>;
    fn stop(&mut self, h: RecordHandle) -> Result<RecordReport, RecordError>;
}
pub struct RecordReport {
    pub files: Vec<FileId>,          // File Services azonosítók
    pub frames: u64, pub dropped: u64,
    pub preserved: Vec<MetadataKind>, // pl. időkód, HDR, feliratok
    pub lost: Vec<(MetadataKind, String)>,
}
```

**Átvétel:** István jóváhagyja a LIVE-terv módosítását; az S5 átvételi táblájában szerepel a `RecordReport` és a File Services-véglegesítés.

**Függőség:** R2, V9 (a `StagedFile` típus).

### V4 – Tally-visszajelzés és bővíthető vezérlőfelület-modell (B és C, 16–26 óra)

**Végzi:** CFA4-CONTROL-SURFACE-001 munkamenete (CS-*, ADR 0012) a V4.1–V4.3 és V4.5 feladatot; CFA4-HW-CONTROL-SURFACES-001 munkamenete (#1287) a V4.4-et.

**Kiinduló állapot (#1231, `crates/surface-model/src/lib.rs`):**

- `SurfaceProtocol { Hid, Midi, Osc }`, `SurfaceInput` (`Button`, `Absolute`, `Relative`…), `SurfaceEvent` eszközönkénti `seq`-kel.
- `FeedbackTarget { Led, Fader, Ring, Display, Timecode, Meter, DeviceState }`, `FeedbackValue { Bool, U14, Text, Level, Tick }`. Tally-állapotot (program/előnézet) nem tud kifejezni, csak egy LED be/ki állapotát.
- `DeviceId(String)`, csak HID-konstruktorral (`DeviceId::hid`).
- Az adapterfolyamat-protokoll (`cfa4-surface-adapter` v1, `crates/surface-adapters/src/wire.rs`, #1287) JSON-sorokat használ: `hello`, `connected`, `input`, `error` gépi kóddal; a felügyelő a `cfa4-cli serve`-ben lesz.

| ID | Feladat | Típus | Eredmény | Óra |
| --- | --- | --- | --- | --- |
| V4.1 | Tally a modellben: `FeedbackTarget::Tally { index: u16 }` és `FeedbackValue::Tally(TallyState)`; `TallyState { program: bool, preview: bool, aux: bool }`, serde camelCase; a `FeedbackPriority` szerint a program előzi az előnézetet | C | új típusok, tesztek | 4–6 |
| V4.2 | L4: a tally-visszajelzés leképezése eszközre a profil szerint (színes LED: piros/zöld/sárga; csak be/ki LED: program = be). Tesztparancs: `surface.feedback.test` (`target`, `value`), hogy hardver nélkül is kipróbálható legyen | C | leképezés, parancs | 4–6 |
| V4.3 | Profiltűrés: ismeretlen `protocol` vagy bemenettípus egy profilban nem teszi tönkre a profilt; az a kötés letiltva betöltődik, figyelmeztetéssel (`surface.profile.warnings`) | B | előre-kompatibilis profilok | 3–5 |
| V4.4 | Az adapterprotokoll általánosítása: a `hello` hordozza a `protocolVersion`, `family` és `kind` (`surface` vagy `io`) mezőt; eltérő verziónál a felügyelő `protocol_mismatch` hibával leállítja a gyermeket, nem omlik össze. A leírás közös dokumentumba kerül (`docs/adapter-process-protocol.md`), mert a DeckLink io-host is ezt követi | B | verziózott, dokumentált protokoll | 4–7 |
| V4.5 | `DeviceId::net(protocol, addr)` hálózati forrásokhoz (OSC most, TSL később); a `file_stem` erre is biztonságos fájlnevet ad | B | konstruktor, teszt | 1–2 |

**Interfész:**

```rust
// cfa4_surface_model
pub enum FeedbackTarget { /* …meglévők… */ Tally { index: u16 } }
pub enum FeedbackValue  { /* …meglévők… */ Tally(TallyState) }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TallyState { pub program: bool, pub preview: bool, pub aux: bool }
impl DeviceId { pub fn net(protocol: SurfaceProtocol, addr: &str) -> Self; }
```

```json
{"type":"hello","protocolVersion":2,"family":"mirabox-family","kind":"surface"}
```

**Átvétel:** (1) a `surface-model` serde-körtesztje az új típusokra; (2) a ShuttlePRO- és MIDI-profilok változatlanul betöltődnek (meglévő tesztek); (3) egy ismeretlen protokollt tartalmazó profil betöltődik, a kötés letiltva, egy figyelmeztetéssel; (4) a `surface.feedback.test` a tesztháttéren (`testing` feature) a várt LED-állapotot adja programra, előnézetre és mindkettőre; (5) eltérő `protocolVersion`-ú gyermeknél `protocol_mismatch` esemény, a `serve` fut tovább.

**Függőség:** R3. A V4.1–V4.2 a vezérlőfelület-terv módosítása (C).

### V5 – Natív YUV a kódolókig (A, B tulajdonsággal, 2–4 óra)

**Végzi:** CFA4-PROCODEC-002 munkamenete.

**Kiinduló állapot:** a `cfa4-frame` `PixelData` már ismeri a `Yuv16 { planes, chroma, bits, alpha }` és `Rgb16` változatot; a `FrameLayout` a mezősorrendet és a chroma-helyet (#1204). A `cfa4_prores::Frame` és a `cfa4_dnx::Frame` síkokra bontott 16 bites YUV-t (`y`, `cb`, `cr`) kap. Az RGBA→YUV átalakítás csak az export-csőben van (`crates/export/src/lib.rs`).

**Megőrzendő tulajdonság (B):** a ProRes- és DNx-kódoló bemenete síkokra bontott YUV marad; ha később RGBA-bemenet is jön, az csak kiegészítés lehet, nem csere.

| ID | Feladat | Eredmény | Óra |
| --- | --- | --- | --- |
| V5.1 | Teszt: egy `VideoFrame` `Yuv16` (4:2:2, 10 bit) képkocka RGBA-kör nélkül ProRes 422 HQ-ra és DNxHR HQX-re kódolva, majd dekódolva; a luma PSNR legalább 50 dB, a kódoló nem hív RGBA-átalakítót | átvételi teszt | 2–4 |

**Átvétel:** a V5.1 teszt zöld a CI-ben. Tervkiegészítés nem kell.

### V6 – Forrásfüggetlen időkód-követés (B, 6–10 óra)

**Végzi:** a TIMECODE-SYNC LTC/MTC terv munkamenete (TC-*, #1285).

**Kiinduló állapot:** a `cfa4-tcsync` chase-motorja már forrásfüggetlen: `Observation { at: Tick, position: Tick }` megfigyeléseket kap (`crates/tcsync/src/chase.rs`). A `cfa4-tcsync-io` viszont zárt: `Source { Ltc, Mtc }`, és a `Backend` trait csak `ltc_input`, `ltc_output`, `mtc` metódust ismer (`crates/tcsync-io/src/manager.rs`). Egy SDI-időkód (RP188/VITC) ma csak a manager átírásával tudna chase-forrás lenni.

| ID | Feladat | Eredmény | Óra |
| --- | --- | --- | --- |
| V6.1 | `SyncManager::external_feed(name: &str) -> Sender<Input>`: külső forrás, amely kész `Observation`-öket küld; a `Source` kiegészül egy `External` változattal, a `SyncStatus` a forrás nevét is mutatja. Az LTC- és MTC-út változatlan | bővítési pont | 3–5 |
| V6.2 | Teszt: szintetikus, 50p képkockapontos forrás a `external_feed`-en; zárás, kimaradás utáni freewheel, újrazárás, mint az LTC-teszteknél | átvételi teszt | 2–3 |
| V6.3 | A crate README-je: hogyan köt be egy új időkód-forrás | dokumentáció | 1–2 |

**Átvétel:** (1) a meglévő LTC- és MTC-tesztek változatlanul zöldek; (2) a V6.2 teszt zöld; (3) a `timecodeSync.*` beállítások formátuma nem változik (az `External` forrás nem választható a beállításokban, amíg nincs használója).

### V7 – `ioHost` folyamattípus a sandboxban (C, 12–18 óra)

**Végzi:** CFA4-SECURITY-001 munkamenete (#1020, `cfa4-sandbox`, B1).

**Kiinduló állapot (`crates/sandbox/src/profile.rs`, #1020):** nyolc folyamattípus (`ProcessKind::ALL`), a profil adatként: `read`, `write` (könyvtárak), `outputs` (egyes fájlok, csak maga a fájl), `exec`, `network`, `gpu`. Eszközcsomópontot csak a `SYSTEM_DEVICES` (`/dev/null`…) és a `GPU_DEVICES` lista enged. Egy rögzítő folyamat, amely kártyát nyit és több fájlra bontva ír, ma nem írható le.

| ID | Feladat | Eredmény | Óra |
| --- | --- | --- | --- |
| V7.1 | `ProcessKind::IoHost` (`id = "ioHost"`, címke „Capture and playout host”), a `ALL` tömbben a terv IV.2 táblázatának sorrendjében | új típus | 2–3 |
| V7.2 | Általános eszközengedély: `Profile::devices: Vec<PathBuf>` (karaktereszközök olvasásra-írásra, eszköz-ioctl-lel, a `gpu` mintájára) és `Context::capture_devices`, `Context::recording_dir`. Nem DeckLink-specifikus: V4L2-eszközre is jó | bővítési pont | 4–6 |
| V7.3 | Az `ioHost` profilja: olvasás = `SYSTEM_READ` (a Desktop Video a `/usr` alatt van); eszköz = `capture_devices`; írás = `recording_dir` (a File Services staging könyvtára, V9); hálózat nincs; programindítás nincs; GPU nincs | profil | 2–3 |
| V7.4 | Tesztek: a profil nem ad hálózatot, `execve`-t és `$HOME`-ot; korlátozási teszt egy ideiglenes `recording_dir`-rel és a `/dev/null`-lal mint eszközzel: a könyvtáron belül ír, kívül `EPERM`, socket nyitása `EPERM` | zöld tesztek | 3–5 |
| V7.5 | `docs/security/threat-model.md`: az `ioHost` sora; a `security.status` listázza | dokumentáció | 1 |

**Interfész:**

```rust
pub enum ProcessKind { /* …meglévő nyolc… */ IoHost }
pub struct Profile { /* … */ pub devices: Vec<PathBuf> }
pub struct Context { /* … */ pub capture_devices: Vec<PathBuf>, pub recording_dir: Option<PathBuf> }
```

**Átvétel:** (1) a meglévő nyolc profil bájtra azonos (pillanatkép-teszt); (2) a V7.4 tesztjei zöldek Landlock ABI ≥ 4 mellett, alacsonyabb ABI-n a `missing()` jelzi a hiányt (meglévő minta); (3) `security.status` az `ioHost`-ot mutatja.

**Függőség:** a SECURITY-terv módosítása (új folyamattípus a IV.2 táblázatban); a `recording_dir` jelentése V9-től függ.

### V8 – Event Fabric (A, 0 óra)

Nincs teendő az Event Fabric projektben. Az E10 értelmezése szerint az io-host és a szerkesztő pont-pont adapterprotokollja (V4.4) nem eseménybusz; a folyamatok közti eseményeket (pl. „új felvétel” a BacklotHubnak) a DeckLink-terület az Event Fabricen küldi. Ezt a DeckLink F0.6 átfedés-ellenőrzése nézi meg.

### V9 – Felvétel véglegesítése másolás nélkül (C, 14–22 óra)

**Végzi:** CFA4-FILE-SERVICES-001 munkamenete, az FS-1 fázis részeként.

**Kiinduló állapot:** a File Services terve (jóváhagyva 2026-11-08, ADR 0009) szerint az írás mindig: másolás a gyökér `staging` könyvtárába → `fsync` → atomikus átnevezés → katalógusrekord (`committed`). Parancsok: `files.import`, `files.seal`, `files.verify`…; a katalógus Pythonban, az adatút Rustban (#45). Kód még nincs a `main`-en (`crates/` alatt nincs files crate). Egy órás, 10 Gbit/s-os felvétel utólagos átmásolása kétszeres lemezhelyet és órákat igényelne.

| ID | Feladat | Eredmény | Óra |
| --- | --- | --- | --- |
| V9.1 | A File Services-terv kiegészítése a 6. és 10. fejezetben: „helyben írt fájl véglegesítése”, három új paranccsal (R4) | tervmódosítás István jóváhagyására | 1–2 |
| V9.2 | `stage_begin(root, rel_path) -> StagedFile`: a gyökér `staging` könyvtárában kizárólagosan (`O_EXCL`, 0600) létrehozott fájl; az út a gyökéren belül marad (meglévő szabály) | Rust API | 3–4 |
| V9.3 | `stage_commit(handle, digest)`: `fsync`, a végleges név ütközésekor számozott név (felülírás soha), atomikus átnevezés, katalógusadat a Python-katalógusnak. Ha az író BLAKE3-at ad (írás közben számolta), a rekord `digestSource: "writer"` jelölést kap, és a teljes ellenőrzést a háttérben futó `files.verify` végzi; ha nem ad, a commit számolja | Rust API | 4–6 |
| V9.4 | `stage_abort(handle)` és induláskori takarítás: a gazdátlan staging-fájlok listája, törlés csak jóváhagyással (meglévő elv) | Rust API | 2–3 |
| V9.5 | Parancsok: `files.stage.begin`, `files.stage.commit`, `files.stage.abort`, audittál (CFA4-AUDIT-001) | parancsok | 1–2 |
| V9.6 | Tesztek: írás közben megölt író (a katalógus nem mutat félkész fájlra); teli lemez commit közben (`FILES_DISK_FULL`, nem pánik); `..`-os út; két egyidejű `begin` ugyanarra a névre; hamis `digest` → a háttér-ellenőrzés `FILES_HASH_MISMATCH`-et ad | zöld tesztek | 3–5 |

**Interfész:**

```rust
pub struct StagedFile { pub handle: StageId, pub path: PathBuf }   // UUIDv7
pub fn stage_begin(root: RootId, rel_path: &RelPath) -> Result<StagedFile, FilesError>;
pub fn stage_commit(h: StageId, digest: Option<Blake3Digest>) -> Result<FileObject, FilesError>;
pub fn stage_abort(h: StageId) -> Result<(), FilesError>;
```

**Átvétel:** a V9.6 tesztjei zöldek; a `files.stage.*` parancsok a vezérlőcsatornán és az MCP-n elérhetők; az FS-1 átvételi táblájában szerepelnek.

**Függőség:** R4; az FS-1 indulása (BacklotHub 5. és 8. lépcső). A V3 és a V7 a `StagedFile`-ra és a staging könyvtárra hivatkozik.

### V10 – Qt6 GUI befejezése (A, 0 óra)

Nincs teendő. A szülőterv D7-je szerint az io-panelek a Qt6 befejezése után jönnek, a panelkeretnek nem kell előre semmit tudnia. A V4.2 tesztparancsa vezérlőcsatornán elérhető, Qt-panel nem kell hozzá.

### V11 – Alkalmazásindító (A, 0 óra)

Nincs teendő. Az io-hostot a szerkesztő felügyelője indítja (a stúdióhost mintájára), nem az ADR 0014 szerinti alkalmazásindító; nincs átfedés.

## 4. Sorrend, becslés, kockázatok

### 4.1 Sorrend (három hullám)

1. **Első hullám, e terv jóváhagyása után azonnal** (csak B és A tételek, tervmódosítás nem kell): V5.1, V6.1–V6.3, V4.3–V4.5, V2.6. Ezek egymástól függetlenek, párhuzamosan futhatnak.
2. **Második hullám, az R1 döntés után:** V1 (a crate-tel), majd V2.1–V2.5 és V2.7. A V2 a V1.1-ben létrejövő crate-re épül.
3. **Harmadik hullám, a tervmódosítások jóváhagyása után:** V4.1–V4.2 (R3, vezérlőfelület-terv), V9 (R4, az FS-1-gyel együtt), V7 (SECURITY-terv; a `recording_dir` a V9 staging könyvtára), V3 (R2, LIVE-terv; a kód az S5-ben).

A DeckLink F0 indítási kapuja (szülőterv 9.3) akkor nyílik meg, ha mindhárom hullám kész, vagy a hátralévő tételekről István döntött.

### 4.2 Becslés projektenként

| Csomag | Projekt (munkamenet) | Típus | Hullám | Óra |
| --- | --- | --- | --- | --- |
| V1 | LIVE-STUDIO ST 2110 T1 | C | 2 | 8–12 |
| V2 | LIVE-STUDIO S0/S1 | B | 1 (V2.6), 2 | 22–35 |
| V3 | LIVE-STUDIO (terv) | C | 3 | 4–6 |
| V4 | CONTROL-SURFACE, HW-CONTROL-SURFACES | B, C | 1, 3 | 16–26 |
| V5 | PROCODEC-002 | A (B) | 1 | 2–4 |
| V6 | TIMECODE-SYNC | B | 1 | 6–10 |
| V7 | SECURITY-001 | C | 3 | 12–18 |
| V8 | EVENT-FABRIC | A | – | 0 |
| V9 | FILE-SERVICES (FS-1) | C | 3 | 14–22 |
| V10 | QT6-FINISH | A | – | 0 |
| V11 | APP-LAUNCHER | A | – | 0 |
| **Összesen** |  |  |  | **84–133** |

Az órák a fejlesztő munkamenet munkáját jelentik a PR-ig, a CI-futtatásokkal együtt. A DeckLink-terület becslését (674–1166 óra) nem csökkentik, mert azokat a részeket a szülőterv eddig is közös rétegként számolta.

### 4.3 Kockázatok

| Kockázat | Hatás | Kezelés |
| --- | --- | --- |
| A #1128 a V1 előtt beolvad | a PTP-olvasó a `cfa4-obs`-ben marad | a V1 külön, kis PR a T1 munkamenettől; az API változatlan, így törés nincs |
| A tervmódosítások a §2.10 szerint megállítják az érintett projektet | a LIVE, a vezérlőfelület, a SECURITY és a File Services munkája feltartózik | minden C tétel kicsi, külön jóváhagyható kiegészítés; csak az érintett lépés áll meg, nem az egész projekt |
| Az FS-1 még nem indult | a V9, és rajta keresztül a V3 és a V7 `recording_dir`-je vár | a V7 a `recording_dir`-t általános könyvtárként veszi fel; a staging-jelentés az FS-1-gyel érkezik |
| A gyűrű v2 lassítja a stúdió előnézetét | képkocka-kiesés az előnézetben | a V2 átvételében mérés a referenciagépen; a BGRA-út kódja nem változik |
| Landlock nem engedi a karaktereszköz ioctl-jét a referenciagép kernelén | az `ioHost` nem nyithatja a kártyát korlátozva | a V7.4 teszt ezt kimutatja; a `missing()` jelzi, és a meglévő módszer szerint (figyelmeztető mód) dönt a felhasználó |
| Egy B tétel PR-ellenőrzéskor elsikkad | későbbi átírás | a koordinátor ezt a tervet és a szülőterv 9. fejezetét ellenőrző listáként használja |

