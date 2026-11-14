# CFA4-DECKLINK-001 1.0 – CFA4-illesztés és végrehajtási terv

**Dátum:** 2026-11-14 · **Állapot:** javaslat, tulajdonosi döntésre vár (a IV. fejezet Q1–Q11 kérdései) ·
**Terv:** [`CFA4-DECKLINK-001-1.0.md`](CFA4-DECKLINK-001-1.0.md) (a tulajdonos szövege, az F8-nál csonka) ·
**Kiinduló állapot:** `main` 4be66a6e

Ez a dokumentum **nem módosítja** a tervjavaslatot. Három dolgot rögzít, az
[EVENT-FABRIC illesztési mellékletének](CFA4-EVENT-FABRIC-001-1.0-CFA4-ILLESZTES.md) mintájára:

1. **mit talál a terv a `main`-en** (I. fejezet: ami megvan, ami hiányzik, amit a terv másnak feltételez);
2. **hol ütközik** a CFA4 rögzített szabályaival és a már jóváhagyott tervekkel (II. fejezet), mert
   a `VISION.md` szerint az ellentmondást jelezni kell, nem csendben feloldani;
3. **a végrehajtási tervet** (III., V–VIII. fejezet): crate-ek és folyamatok, FFI-határ, API-szerződések,
   fázisok feladatokkal és elfogadási tesztekkel, donortriázs.

Kódot ez a PR nem tartalmaz; a megvalósítás a döntések után, fázisonként indul.

---

## 0. Összefoglaló

- **A terv megvalósítható**, de a mostani formájában négy ponton ütközik a `main`-nel. A legsúlyosabb a
  **licenc**: a DeckLink futtatókönyvtára (`libDeckLinkAPI.so`, a Desktop Video csomag része) zárt kód,
  a `docs/licensing.md` 4. pontja pedig kimondja, hogy a BRAW-adapteren kívül „más zárt könyvtár nem kap
  kivételt”. **DeckLink-támogatás csak új tulajdonosi döntéssel lehet** (Q1). A javaslat a BRAW-minta:
  GPL-3 7. pont szerinti kiegészítő engedély, külön folyamat, a Desktop Videót a felhasználó telepíti.
- **Az F4 (PTP), F5 (ST 2110) és F6 (NMOS) nagyrészt már jóváhagyott munka** a
  [LIVE-STUDIO-001 ST 2110 kiegészítésben](CFA4-LIVE-STUDIO-001-ST2110.md) (T0–T7; D8 saját Rust crate,
  D9 linuxptp csak olvasva, D10 csak VC-2, D11 NMOS a T7-ben). A `cfa4-st2110` crate T0-ja kész a `main`-en.
  Ezeket nem kell újra megtervezni, hanem össze kell kötni: a DeckLink-terv csak a hídelemeket adja (Q4).
- **Amire a terv épít, annak egy része nincs meg:** nincs „CFA4 Recording” rendszer vagy terv (a hangalámondás
  WAV-felvételén kívül), nincs „Control Surface” architektúra (`docs/hardware-parity.md`: 0%), a szerkesztő
  nem használ PostgreSQL-t (az csak a BacklotHub adattára, ADR 0006), és az export nem FFmpeg-alapú (saját
  kódolók, az FFmpeg a `cfa4-route` mögötti tartalék). Ezeket a terv F0-jában létre kell hozni vagy át kell
  fogalmazni (II. fejezet).
- **A médiapipeline nem élő bemenetre készült:** a `MediaSource` véletlen elérésű (idő szerint kér),
  a kódolók RGBA8/f32 képet kapnak, a `VideoFrame` nem visz ANC-t, időkódot és képkockánkénti
  HDR-metaadatot. A „nincs észrevétlen bitmélység-csökkenés” feltételhez **natív YUV-rögzítőút** kell
  (v210 → ProRes/DNxHR/FFV1/tömörítetlen, RGBA-kerülő nélkül).
- **Hardver:** a referenciagépben (`docs/development/reference-machine.md`) nincs DeckLink-kártya. Az F0–F1a
  hardver nélkül, mock eszközzel és CI-ben is megy; az F1b-től kell kártya (beszerzési javaslat: Q8).
- **Ajánlott sorrend:** F0 → F1a (HAL, mock, natív rögzítő) → F1b/F1c (DeckLink be/ki, külső monitor) →
  F2 és F3 párhuzamosan → F4 → F7; az F5/F6 a LIVE-STUDIO T-fázisaiban fut. Durva becslés a DeckLink-részre:
  **≈ 740–1290 óra** (VII. fejezet).

---

## I. Kiinduló helyzet a `main`-en

| Terület | A terv feltételezése | A `main` valósága | Következmény |
|---|---|---|---|
| Médiapipeline (képkocka) | közös pipeline videóval, hanggal, PTS-sel, színnel, HDR-rel | `cfa4-frame`: `VideoFrame` (`Yuv8`, `Yuv16` 9–16 bit, `Rgb16`, `Rgba8`, `RgbaF32`; `ColorInfo`; `FrameLayout` mezőrend és chroma-helyzet; `pts: Tick`), `AudioBuffer` (síkonkénti `f32`) | a képadat jó alap (10/12 bit `Yuv16`/`Rgb16`-ben veszteségmentes); **hiányzik** a képkocka melletti adat: ANC, időkód, képkockánkénti HDR-metaadat, hardveridő, elejtés-jelzők |
| Médiapipeline (forrás) | a backend „átadja” a pipeline-nak | `cfa4-media::MediaSource` húzó modell: `video_frame(time)`, `audio(start, frames)` | élő, toló forrásra nincs trait; **új élő-forrás réteg kell** |
| Kódolás, rögzítés | „FFmpeg-alapú kódolási útvonal” és „CFA4 rögzítési rendszer” | `cfa4-export`: saját kódolók (ProRes, DNxHR, H.264, HEVC, AV1, MPEG-2, APV…) és muxerek (MOV/MP4, MXF OP1a/OP-Atom, WebM, MPEG-TS, DCP, IMF); a `VideoEncoder` `EncoderFrame { rgba, hdr: Option<&[f32]> }`-t kap; az FFmpeg (`cfa4-ffmpeg`) kiegészítő háttér a `cfa4-route` mögött | a rögzítés a saját kódolókra és muxerekre épül, nem az FFmpeg-re; **a YUV-képet ma RGBA-n át kódoljuk** – élő rögzítéshez közvetlen YUV-bemenet kell a kódolókba és tömörítetlen v210/PCM-írás |
| „CFA4 Recording” | létező tervek és rendszer (F0 1. pont) | nincs ilyen terv; van: hangalámondás WAV-ba (`engine::voiceover`), a növekvő fájlok frissítése (`media_watch`), az élő stúdió S5 „Felvétel” fázisa (libobs, a Qt6 után) | a rögzítő **ebben a tervben** készül (F1a), és ugyanezt kell használnia később az élő stúdió felvételének is (Q5) |
| FFmpeg DeckLink-eszköze | — | az FFmpeg `--enable-decklink` a `nonfree` listán van (a `configure` `EXTERNAL_LIBRARY_NONFREE_LIST`-je), a CFA4 szabálya pedig „never `--enable-nonfree`” | az FFmpeg DeckLink-útja **kizárt** |
| GStreamer | kiegészítő interoperabilitás | `cfa4-gstreamer` (L5, opcionális); az Ubuntu 26.04 `gstreamer1.0-plugins-bad` csomagja tartalmazza a `libgstdecklink.so`-t | tesztorákulumnak kiváló (a mi rögzítésünk összevetése a `decklinkvideosrc`-ével); termékútként nem kell (II. E1, E6) |
| Élő stúdió | — | LIVE-STUDIO-001: `cfa4-obs` (L5) S0 kész: `StudioBackend` trait, `cfa4-studio-host` külön folyamat felügyelettel és újraindítással, memfd/`$XDG_RUNTIME_DIR` képkocka-gyűrű (`ring.rs`, `unsafe` nélkül) | **ez a minta** a DeckLink-folyamathoz; a DeckLink-bemenet a stúdió forrása, a program a DeckLink kimenete lesz |
| ST 2110, PTP, NMOS | F4–F6 új munka | `cfa4-st2110` (L0) T0 kész: RTP, SDP, -20 GPM/BPM csomagolás; T1–T7 jóváhagyva; PTP: a rendszer `linuxptp`-je, a CFA4 csak olvas (D9); NMOS IS-04/05 a T7-ben (D11); -22 csak VC-2 (D10) | F4 PTP-része = T1, F5 = T2–T6, F6 = T7 (Q4) |
| Időkód | Timecode | `cfa4-time` (`Tick` = 1/254 016 000 000 s, NDF/DF időkód), `cfa4-tcsync` (LTC, MTC, chase), `cfa4-tcsync-io` (cpal, ALSA szekvenszer, `SyncManager`) | a DeckLink RP188/VITC-időkód és a genlock állapot ugyanabba a szinkronállapotba kerül |
| Feliratok | Caption adatok | `cfa4-captions`: CEA-608 (`cea608.rs`), MCC, SCC, TTML, STL… | az ST 334 (CEA-608/708 ANC-ban) bontás után ide kerül; az MCC a természetes oldalfájl |
| Vezérlés | „meglévő Control Surface architektúra” | nincs (`docs/hardware-parity.md`: control surfaces 0%); van: parancsregiszter (minden művelet parancs), vezérlőcsatorna, MCP, MIDI csak az MTC-hez; tervben: obs-websocket kiszolgáló (S10) | a „közös vezérlőmotor” a parancsregiszter; az adaptereknek parancsokra kell képezniük (II. E4) |
| Adatkezelés | PostgreSQL: eszközprofilok, konfiguráció, metaadat | a szerkesztőben nincs adatbázis; a beállítások a preferenciákban, a projekt `.fcproj` (serde); a PostgreSQL a BacklotHub/`cfa4_files` adattára (ADR 0006, 0009) | eszközprofil és rögzítési beállítás a preferenciákba és a projektbe; PostgreSQL csak a BacklotHub-katalógusba jelentéshez (Q9) |
| Qt6 GUI | eszköz-, jel-, profilfelületek | `python/cfa4_qt/`, a motor `cfa4-cli serve` (JSON-sorok); a migráció folyamatban; az élő stúdió UI-ja a Qt6 befejezése után (LIVE D7) | a Rust-oldal most mehet, a Qt-panelek a D7 szerint (Q7) |
| Rétegek | — | `cargo xtask layers`: L5 a hardverközeli crate-ek (`platform`, `obs`, `tcsync-io`), L6 az alkalmazások | új crate-ek elhelyezése: III.2 |
| Hardver | valós DeckLink | a referenciagépen nincs DeckLink; 2,5 GbE, két NUMA-csomópont, kernel 7.0 | hardveres elfogadás a kártya beszerzése után; a Desktop Video DKMS-modulja és a 7.0-s kernel viszonya F0-kockázat |

---

## II. Eltérések és ellentmondások

| # | Téma | A terv | A CFA4 szabálya / állapota | Javaslat |
|---|---|---|---|---|
| E1 | **Licenc (blokkoló)** | „Hivatalos Blackmagic SDK – kötelező alap” | `docs/licensing.md` 4.: zárt kód sem a folyamatban, sem külön folyamatként; egyetlen kivétel a BRAW-adapter, „no other proprietary library gets this exception”. LIVE D4: zárt SDK-t igénylő protokoll (NDI) nincs. Az SDK fejlécei megengedő licencűek, de a 2021-es fejléc szerint a Blackmagictől letöltött példányra a DeckLink SDK EULA vonatkozik; a futtatókönyvtár zárt | **Q1.** Javaslat: a BRAW-minta. A DeckLink-adapter GPL-3.0-or-later, GPL-3 7. pont szerinti kiegészítő engedéllyel a felhasználó által telepített DeckLink API futtatókönyvtár betöltésére; a CFA4 nem szállítja, nem tölti le és nem telepíti a Desktop Videót; nélküle a DeckLink „nem elérhető”, nem hiba. Külön folyamatban fut (E8). Kiadás előtt jogi átnézés (a `licensing.md` maga is ezt kéri) |
| E2 | **Átfedés az ST 2110 kiegészítéssel** | F4 PTP, F5 ST 2110, F6 NMOS saját fázisként | jóváhagyva: T0–T7, D8–D12; `cfa4-st2110` T0 kész | **Q4.** Az F5 és F6 ne legyen külön backend: a T-fázisok viszik. A DeckLink-terv adja: (a) a közös HAL-t, amelyen az ST 2110 vevő/küldő is eszközként jelenik meg, (b) SDI↔ST 2110 átjárást, (c) a Blackmagic IP-kártyák (ha Q1 engedi) hardveres ST 2110-útját mint másik eszközt ugyanazon a HAL-on. Átfedés: a koordinátornak (`docs/coordination.md` §2.8), a döntés a `coordination-log.md`-be |
| E3 | **Párhuzamos rögzítőmotor veszélye** | „nem hozhat létre párhuzamos rögzítési motort”, „FFmpeg-alapú rögzítés” | ma nincs általános rögzítő; az élő stúdió S5-je a libobs-szal rögzítene; a GStreamer és az FFmpeg nem lehet fő rögzítő (ADR 0005) | **Q5.** Egy rögzítő legyen: `cfa4-ingest` (III.2), a saját kódolókkal és muxerekkel, a `cfa4-route` választásával (FFmpeg csak tartalék). Az élő stúdió S5 programfelvétele is ezt használja (a LIVE-tervet érinti: koordináció) |
| E4 | **Nincs Control Surface architektúra** | „a meglévő Control Surface architektúrához kell illeszkednie” | nincs ilyen; a közös vezérlés ma a parancsregiszter (UI, CLI, vezérlőcsatorna, MCP ugyanazt hívja) | **Q6.** A „közös vezérlőmotor” a parancsregiszter. Új `cfa4-control-io` (L5): TSL UMD, OSC, MIDI, később HID/Mackie mind parancsra és eseményre képez, saját állapotgép nélkül. A Mackie/EUCON-típusú vezérlőfelületek ugyanezt használják majd |
| E5 | PostgreSQL mint konfigurációs tár | eszközprofilok és konfiguráció PostgreSQL-ben | a szerkesztő offline, adatbázis nélkül indul; a PostgreSQL a BacklotHub külön szolgáltatása | **Q9.** Eszközprofil: preferencia (`settings`); rögzítési előbeállítás: a projekt; klipmetaadat: a fájlban és oldalfájlban. A BacklotHub-katalógusba csak „bekerült egy felvétel” esemény megy, ha a BacklotHub fut |
| E6 | GStreamer szerepe | interoperabilitás | ADR 0002: opcionális háttér; a DeckLink-plugin ugyanúgy zárt könyvtárat tölt be a CFA4 folyamatába | termékútként nem kell (a natív út teljesebb: ANC, HDR, ütemezett kimenet, keyer, genlock); **tesztorákulum** a hardveres gépen, mint az FFmpeg a kodekeknél |
| E7 | „C++ FFI” | Rust–C++ FFI | `AGENTS.md`: Rust first, nem-Rust komponens mehet, ha nem lassít; `unsafe` csak a kötés moduljában (ADR 0001 kiegészítés, 2026-11-07) | **Q2.** Vékony C++-réteg (`extern "C"`, csak a DeckLink-folyamatba fordítva) vagy tisztán Rust vtable-kötés. Javaslat: C++-réteg (a COM-szerű C++ interfészek és visszahívások ABI-ját a fordító adja, nem kézzel átírt vtable); később Rustra portolható |
| E8 | A DeckLink kódja hol fut | a Rust backend része | never-crash (`AGENTS.md` §0), libobs-precedens: külön folyamat (LIVE D2) | **Q3.** Külön `cfa4-io-host` folyamat (nem a `cfa4-studio-host`): a gyártói könyvtár hibája csak állapotváltozás; a felvétel fájlja lezárható marad (III.4) |
| E9 | Szigorú fáziskapuk | „a következő szakasz csak az előző elfogadása után” | az F4–F6 hardver nélkül is halad (T-fázisok), az F2 és F3 egymástól független | **Q11.** Függőségi gráf (VI. fejezet) a lánc helyett; az elfogadási feltételek változatlanok |
| E10 | DNG-útvonal az F2-ben | „DNG-alapú RAW munkafolyamatok vizsgálata” | a DeckLink nem ad szenzoradatot; a DNG/CinemaDNG a PROCODEC-002 11. munkacsomagja | **Q10.** Az F2-ből kikerül; az EXR-írás (köztes, lineáris) marad |
| E11 | Key/Fill és többkimenet | F7 | a kompozitálás az élő stúdióban (S4 stúdió mód) és a CFA4 renderelőjében van | az F7 Key/Fill része az S4 után; a DeckLink belső keyere (ha a kártya tudja) önállóan is mehet a szerkesztő grafikájához |
| E12 | Hardveres teszt | „valós hardver”, „kétórás teszt” | a referenciagépen nincs kártya; a CI-ben nincs hardver | mock eszköz és visszajátszó a CI-ben; hardveres hurokteszt (a kártya saját kimenete a bemenetére) a referenciagépen; a kártya bekerül a `reference-machine.md`-be, ha megvan (Q8) |
| E13 | A terv csonka | F8 elfogadási feltételek („Telepítési együtt…”) | — | a hiányzó rész a tulajdonostól; az F8-at itt az együttélési pontokkal egészítjük ki (V. F8), a szöveg érkezéséig javaslatként |

---

## III. Célarchitektúra a CFA4-ben

### III.1 Folyamatok és adatút

```
 Qt6 felület (Python)        CFA4 motor (cfa4-cli serve)                 cfa4-io-host (új folyamat)
  I/O panelek  ──JSON──►  io.* parancsok, felügyelet  ──vezérlő-socket──►  DeckLink-szolgáltató
  kép, állapot ◄──────────  cfa4-ingest rögzítő      ◄── I/O-gyűrű (be) ──  cfa4-decklink (C++-réteg)
                            lejátszó (Program)       ──► I/O-gyűrű (ki) ──►  ütemezett kimenet
                                 │                                            │
                                 ▼                                       libDeckLinkAPI.so
                     fájl (MOV/MXF) + oldalfájlok                         (a felhasználó telepíti)
                                 │
                     projekt (.fcproj), opcionálisan BacklotHub-esemény

 cfa4-studio-host (élő stúdió) ugyanazt az I/O-gyűrűt olvassa forrásként, és oda írja a programot
 ST 2110 (cfa4-st2110, T-fázisok) ugyanazon a HAL-on jelenik meg eszközként
```

Az adatút a terv 3.2 pontja szerint, a CFA4-re igazítva:
DeckLink hardver → DeckLink API → C++-réteg → `cfa4-decklink` (a `cfa4-io-host`-ban) → I/O-gyűrű →
`cfa4-capture` élő forrás → `cfa4-ingest` (saját kódolók és muxerek, a `cfa4-route` választásával) →
fájl és projekt. A kimenet fordítva: a lejátszó vagy az élő stúdió → I/O-gyűrű → ütemezett DeckLink-kimenet.

### III.2 Crate-ek és rétegek

| Crate / alkalmazás | Réteg | Felelősség | Függ |
|---|---|---|---|
| `cfa4-anc` (új) | L0 | SMPTE ST 291-1 ANC-csomag: DID/SDID, DBN, DC, UDW, ellenőrzőösszeg, paritás; v210 VANC-sor bontása és építése; payloadok: ST 334-1/-2 (CEA-608/708 CDP), ST 2016-3 AFD és bar data, ST 12-2 ATC időkód, ST 2108 HDR-metaadat, ST 2038 (MPEG-TS-be), KLV (ST 336) átlátszóan; RFC 8331 (ST 2110-40) közös kódja | `cfa4-bitstream` |
| `cfa4-capture` (új) | L1 | eszközfüggetlen HAL: típusok és traitek (III.3), pixelcsomagolók (v210, r210, R12L, 2vuy, 8/10 bit BGRA/ARGB ⇄ `Yuv16`/`Rgb16`, bitazonos), I/O-gyűrű v2 és vezérlőprotokoll, mock eszköz és fájlból visszajátszó eszköz a tesztekhez | `cfa4-time`, `cfa4-frame`, `cfa4-color`, `cfa4-anc`, `serde` |
| `cfa4-ingest` (új) | L3 | élő rögzítő: puffer, kódolás (közvetlen YUV-bemenet), mux MOV/MXF (időkódsáv, többcsatornás PCM, AES3 átlátszóan), növekvő fájl, szegmentálás, elejtés- és veszteségnapló, metaadat-megőrzési jelentés | `cfa4-capture`, `cfa4-export`, `cfa4-codecs`, `cfa4-mxf`, `cfa4-isobmff`, `cfa4-captions`, `cfa4-route` |
| `cfa4-decklink` (új) | L5 | a DeckLink API kötése (`feature = "decklink"`), `unsafe` csak az FFI-modulban (ADR 0001 szabályai), C++-réteg `cc`-vel fordítva; `cfa4_capture::DeviceProvider` megvalósítása; futásidejű felismerés | `cfa4-capture` |
| `cfa4-io` (új) | L5 | a `cfa4-io-host` felügyelete (indítás, újraindítás, állapot), a vezérlő-socket kliense, a gyűrű olvasója; ST 2110- és mock szolgáltató regisztrációja | `cfa4-capture` |
| `cfa4-control-io` (új, F7) | L5 | TSL UMD v3.1/v4/v5, OSC, MIDI (a meglévő ALSA-szekvenszer kódra építve) → parancs és esemény | `cfa4-capture` (tally-állapot), `alsa`, OSC-crate |
| `cfa4-io-host` (új alkalmazás) | L6 | a DeckLink-folyamat: szolgáltatók betöltése, gyűrű írása/olvasása, minden visszahívás `catch_unwind` alatt | `cfa4-decklink`, `cfa4-capture` |
| `apps/cfa4-cli` | L6 | `serve_io.rs`: `io.*` parancsok (a `serve_live.rs`, `serve_tcsync.rs` mintájára) | `cfa4-io`, `cfa4-ingest` |
| `cfa4-st2110` | L0 | változatlan; a -40 ANC (T5) a `cfa4-anc`-ot használja | `cfa4-anc` (új él; az L0 kodek- és konténercrate-ek ma csak a `cfa4-bitstream`-től függhetnek, ezért az F0.5 dönti el, hogy a `cfa4-anc` alapcrate lesz-e, mint a `time` és a `color`, vagy az -40 a `cfa4-capture`-ben köt össze) |

A `cfa4-capture` gyűrűje a `cfa4-obs::ring` mintája (pozicionált írás/olvasás, `unsafe` nélkül, szakadt
képkocka eldobva), de bővített rekesszel (kép, hang, oldaladat). A két gyűrű később egyesíthető;
ez nem feltétele egyik fázisnak sem.

### III.3 API-szerződések (vázlat, a F0 véglegesíti)

```rust
// cfa4-capture (L1) – eszközfüggetlen; minden pub fn Result-ot ad, panic nincs.

pub struct DeviceId { pub provider: ProviderKind, pub persistent: String } // DeckLink: persistent ID / topológiai ID
pub enum ProviderKind { DeckLink, St2110, Mock, Replay }

pub struct DeviceInfo {
    pub id: DeviceId,
    pub model: String,
    pub display_name: String,
    pub caps: Capabilities,
}

/// Csak az szerepel, amit a hardver, a gyártói API ÉS a CFA4-út is tud (a terv 5.4 hármas feltétele):
/// minden tétel `Support { hardware, api, pipeline }`, a felület csak a teljesen támogatottat kínálja.
pub struct Capabilities {
    pub inputs: Vec<Connector>,           // SDI 1..n, HDMI, optikai, referencia
    pub outputs: Vec<Connector>,
    pub modes: Vec<ModeSupport>,          // VideoMode × irány × PixelFormat × Support
    pub audio: AudioCaps,                 // csatornaszám, 16/32 bites egész, 48 kHz
    pub anc: AncCaps,                     // VANC be/ki, ANC-csomag API
    pub hdr: HdrCaps,                     // EOTF-ek, ST 2086 / CLL be és ki
    pub sync: SyncCaps,                   // genlock bemenet, időzítési eltolás, PTP (IP-kártya)
    pub keyer: Option<KeyerCaps>,         // belső / külső
    pub duplex: DuplexCaps,               // fél/teljes duplex, profilok
}

pub struct VideoMode { pub width: u32, pub height: u32, pub rate: cfa4_time::FrameRate,
                       pub field_order: cfa4_frame::FieldOrder, pub psf: bool }
pub enum PixelFormat { Yuv8_2vuy, Yuv10_v210, Rgb10_r210, Rgb10_r10l, Rgb12_r12l, Bgra8, Argb8 }

pub struct CapturedFrame {
    pub seq: u64,                          // a szolgáltató sorszáma; rés = elejtés
    pub mode: VideoMode,
    pub video: Option<cfa4_frame::VideoFrame>, // None: jel nélküli képkocka (NoSignal)
    pub audio: Option<PcmBlock>,           // egész PCM, nem f32: bitátlátszó (ST 337 is)
    pub timecode: Option<FrameTimecode>,   // RP188 (LTC/VITC1/VITC2), VITC, forrás megjelölve
    pub anc: Vec<cfa4_anc::AncPacket>,
    pub hdr: Option<HdrMetadata>,          // ST 2086 primerek, fehérpont, lum., MaxCLL, MaxFALL, EOTF
    pub clocks: FrameClocks,
    pub flags: FrameFlags,                 // NoSignal, FormatChanged, Dropped(n), Late
}

pub struct PcmBlock { pub sample_rate: u32, pub channels: u16, pub bits: u8, // 16 vagy 32 (24 hasznos)
                      pub interleaved: Vec<i32> }

pub struct FrameClocks {
    pub stream: cfa4_time::Tick,           // a DeckLink stream time-ja közvetlenül Tick-időskálán kérve
    pub hw_reference_ns: Option<u64>,      // hardveres érkezési idő (CLOCK_MONOTONIC tartomány)
    pub host_monotonic_ns: u64,
    pub tai_ns: Option<u64>,               // ha van PTP (ST 2110 T1 állapotából)
}

pub enum DeviceEvent {
    Added(DeviceInfo), Removed(DeviceId),
    SignalLocked { id: DeviceId, mode: VideoMode }, SignalLost(DeviceId),
    FormatChanged { id: DeviceId, mode: VideoMode, format: PixelFormat },
    ReferenceLocked(DeviceId, bool), Dropped { id: DeviceId, frames: u64, reason: DropReason },
    Underrun(DeviceId), Error { id: DeviceId, message: String },
}

pub trait DeviceProvider: Send {
    fn kind(&self) -> ProviderKind;
    fn available(&self) -> Availability;   // NotInstalled { hint } | Ready { api_version } | Failed { reason }
    fn devices(&mut self) -> Result<Vec<DeviceInfo>>;
    fn open_input(&mut self, id: &DeviceId, cfg: &InputConfig) -> Result<Box<dyn CaptureStream>>;
    fn open_output(&mut self, id: &DeviceId, cfg: &OutputConfig) -> Result<Box<dyn PlayoutStream>>;
    fn poll_events(&mut self) -> Vec<DeviceEvent>;
}

pub trait CaptureStream: Send {
    fn start(&mut self) -> Result<()>;
    fn next(&mut self, timeout: std::time::Duration) -> Result<Option<CapturedFrame>>;
    fn stats(&self) -> CaptureStats;       // érkezett, elejtett, késett, puffermélység
    fn stop(&mut self) -> Result<()>;
}

pub trait PlayoutStream: Send {
    fn preroll(&mut self, frames: &[OutFrame]) -> Result<()>;
    fn schedule(&mut self, frame: OutFrame) -> Result<()>; // OutFrame: kép + PCM + TC + ANC + HDR, Tick-időponttal
    fn start_at(&mut self, at: cfa4_time::Tick) -> Result<()>;
    fn stats(&self) -> PlayoutStats;       // ütemezett, késve kiadott, elejtett, alulcsordulás
    fn stop(&mut self) -> Result<()>;
}
```

```rust
// cfa4-ingest (L3) – a közös rögzítő; a hardveres szolgáltató nem ír fájlt (terv 3.3).
pub struct RecordJob { pub target: RecordTarget, pub video: VideoCodecChoice, pub audio: AudioChoice,
                       pub segment: Option<Segmenting>, pub metadata: MetadataPolicy }
pub enum VideoCodecChoice { UncompressedV210, UncompressedRgb12, ProRes(ProResProfile),
                            DnxHr(DnxProfile), Ffv1, Routed(cfa4_route::Task) }
pub struct MetadataPolicy { pub on_unsupported: OnUnsupported } // Warn | Sidecar | Refuse
pub trait Recorder { fn push(&mut self, f: &CapturedFrame) -> Result<RecordStatus>;
                     fn finish(self) -> Result<RecordReport>; }
pub struct RecordReport { pub frames: u64, pub audio_samples: u64, pub drops: Vec<DropRecord>,
                          pub preserved: Vec<MetaKind>, pub lost: Vec<(MetaKind, String)>, /* … */ }
```

A szerződés lényege: **a szolgáltató csak ad és vesz** (kép, hang, időkód, ANC, HDR, órák, események),
minden puffer, kódolás, fájl és projekt a `cfa4-ingest`-é és a motoré (terv 3.3). A `RecordReport.lost`
teljesíti a 3.2 pont „nem veszhet el észrevétlenül” szabályát: ha a célformátum nem visz egy metaadatot,
a jelentés és a felület megnevezi, oldalfájlba menti (`MetadataPolicy::Sidecar`), vagy a rögzítés el sem
indul (`Refuse`).

**Parancsok (minden parancs, ügynökkel vezérelhető):** `io.providers` (telepítve-e a Desktop Video,
API-verzió), `io.devices.list`, `io.device.status`, `io.input.open`/`close`, `io.capture.start`/`stop`
(rögzítési feladat, haladás, megszakítás), `io.output.open`/`close`, `io.monitor.set` (a Program monitor
képe a külső kimenetre, a Premiere „Mercury Transmit” megfelelője), `io.sync.status`. A felület állapota
serde, automation id-kkal.

### III.4 FFI-határ és hibahatár

- **Betöltés:** a `cfa4-decklink` a `libDeckLinkAPI.so`-t futásidőben tölti be (a SDK `DeckLinkAPIDispatch.cpp`
  mintájára, `dlopen`), így a CFA4 fordítása és indulása nem függ tőle. Hiányában `Availability::NotInstalled`
  telepítési útmutatóval, nem hiba (a `licensing.md` 7. pontjának felhasználói mintája).
- **C++-réteg (Q2):** `crates/decklink/shim/` néhány száz–ezer sor, csak `extern "C"` függvények és POD-struktúrák;
  a COM-szerű referenciaszámlálás, a `IDeckLinkInputCallback` / `IDeckLinkVideoOutputCallback` /
  `IDeckLinkAudioOutputCallback` / `IDeckLinkDeviceNotificationCallback` megvalósítása itt van, és a Rust felé
  csak sorba tett eseményeket és képkockapuffereket ad. A visszahívásból semmi nem hív vissza Rustba
  szinkronban: a C++ oldal egy korlátos sorba tesz, a Rust szál olvas – így a C++-kivétel és a Rust-pánik
  nem keresztezi a határt.
- **Fejlécek:** az SDK `include/` fájljai a fordításhoz kellenek. A vendorálás (a GStreamer is így teszi) csak
  a Q1 és az EULA-átnézés után; addig a `CFA4_DECKLINK_SDK_DIR` mutat a helyi SDK-ra, és a feature nélkül a
  workspace a fejlécek nélkül is fordul.
- **`unsafe`:** csak a `cfa4-decklink::ffi` modulban, `// SAFETY:` megjegyzéssel, ABI-tesztekkel (méretek,
  eltolások), mint a `vaapi` és a `libobs` kötésnél (ADR 0001).
- **Folyamathatár (Q3):** a `cfa4-io-host` összeomlása a motorban `SignalLost`/`Error` esemény; a felügyelő
  újraindítja; a futó felvétel a `cfa4-ingest`-ben a motor folyamatában van, így a fájl szabályosan lezárul
  (az utolsó teljes képkockáig), és a folytatás új szegmensbe megy.
- **Valós idő:** a gyűrű rekeszei előre lefoglaltak; a host szálai a kártya NUMA-csomópontjához rögzíthetők
  (a referenciagép két csomópontos); túlcsordulás = számlált elejtés, soha nem blokkol a DeckLink-visszahívás.

---

## IV. Tulajdonosi döntések (javaslattal)

| # | Kérdés | Javaslat | Döntés |
|---|---|---|---|
| Q1 | Kaphat-e a DeckLink API futtatókönyvtára a BRAW-hoz hasonló GPL-3 7. pont szerinti kivételt? | Igen, a BRAW-mintával: GPL-3.0-or-later adapter + kiegészítő engedély, a felhasználó telepíti a Desktop Videót, külön folyamat, kiadás előtt jogi átnézés. Másik út: nincs DeckLink, csak ST 2110 (IP) és a GStreamer-orákulum fejlesztői gépen | nyitott |
| Q2 | C++-réteg vagy tisztán Rust vtable-kötés? | Vékony C++-réteg csak a `cfa4-io-host`-ba fordítva; Rust-port később | nyitott |
| Q3 | Külön `cfa4-io-host` vagy a `cfa4-studio-host` része? | Külön folyamat; a stúdió a gyűrűn kapja a képet | nyitott |
| Q4 | Az F4 PTP-része, az F5 és az F6 beolvad-e a LIVE-STUDIO-001 ST 2110 T1–T7 fázisaiba? | Igen; a DeckLink-terv csak a HAL-t és a hídat adja; koordinátori bejegyzés | nyitott |
| Q5 | Egy közös rögzítő (`cfa4-ingest`) legyen-e a DeckLink-felvételhez és az élő stúdió S5 felvételéhez is? | Igen (a terv „nincs párhuzamos rögzítőmotor” szabálya); a LIVE-terv S5-jét érinti | nyitott |
| Q6 | A „közös vezérlőmotor” a parancsregiszter, és az F7 adapterei egy új `cfa4-control-io`-ban készüljenek? | Igen; a későbbi vezérlőfelület-munka (Mackie, EUCON) ugyanerre épül | nyitott |
| Q7 | Mikor jönnek a Qt6-panelek? | A Rust-oldal (F0–F4) most; a Qt-panelek a Qt6 befejezése után (LIVE D7 mintája); addig vezérlőcsatorna és MCP | nyitott |
| Q8 | Melyik DeckLink-kártya legyen a referenciagépben? | Egy kétirányú 12G-SDI-s, referencia-bemenetes, 12 bites RGB-t és HDR-metaadatot tudó kártya (pl. DeckLink 8K Pro osztály), mellé egy olcsó egybemenetes a többeszközös teszthez; a pontos képességeket az F0 a gyártói adatlapból ellenőrzi | nyitott |
| Q9 | PostgreSQL a DeckLink-részben? | Nem: preferencia és projekt; a BacklotHub-katalógus csak eseményt kap | nyitott |
| Q10 | DNG kikerül-e az F2-ből? | Igen, a PROCODEC-002 11. csomagjába tartozik | nyitott |
| Q11 | Szigorú fázislánc vagy függőségi gráf? | Gráf (VI. fejezet), az elfogadási feltételek változatlanok | nyitott |

---

## V. Fázisok, feladatok, elfogadási tesztek

A fázisnevek a tervéi; az F1 három részre bomlik, hogy hardver nélkül is legyen beolvasztható eredmény.
Minden fázis végén zöld kapuk (`cargo xtask ci`, tesztek, Python-ellenőrzések), never-crash tesztek
(hibás bemenet, `catch_unwind`), és a feladatonkénti commit-szabály (`DL-F1a.3: …`).

### F0 – Előkészítés és döntések (hardver nélkül)

| # | Feladat | Eredmény |
|---|---|---|
| F0.1 | A Q1–Q11 döntések | e dokumentum IV. fejezete kitöltve; Q1-től függően a `licensing.md` 4. pont és a regiszter módosítása külön PR-ban |
| F0.2 | Blackmagic SDK és Desktop Video: aktuális verzió, EULA (újraterjesztés, fejlécek, futtatókönyvtár), Ubuntu 26.04 / 7.0-s kernel DKMS-kompatibilitás | `docs/decklink.md` „Licenc és telepítés” fejezete, forrásmegjelöléssel |
| F0.3 | Donortriázs (VIII. fejezet) | `docs/decklink-donors.md`, mind a 26 sor kitöltve |
| F0.4 | ADR 0012: DeckLink I/O (folyamatmodell, FFI, licenckivétel, betöltés, gyűrű) | `docs/adr/ADR-0012-decklink-io.md` |
| F0.5 | API-szerződés véglegesítése (III.3), a `cfa4-capture` és `cfa4-anc` váza üres megvalósítással, a `cargo xtask layers` táblájába felvéve | fordul, rétegellenőrzés zöld |
| F0.6 | Koordináció: átfedés a LIVE-STUDIO-001-gyel (E2, E3) | sor a `docs/coordination-log.md`-ben |
| F0.7 | Hardveres tesztkörnyezet terve: hurokteszt, jelgenerátor, referencia-szinkronjel | `docs/decklink.md` „Tesztkörnyezet” |

**Elfogadás:** a terv F0-feltételei; ezen felül az ADR elfogadva, a Q1 eldöntve. Ha a Q1 „nem”, a terv az
F1a-val és az IP-úttal folytatódik, DeckLink nélkül.

### F1a – HAL, mock eszköz, natív rögzítő (hardver nélkül, CI-ben)

Feladatok: `cfa4-capture` típusai és gyűrűje; pixelcsomagolók (v210, r210, R12L, 2vuy, BGRA) bitazonos
oda-vissza; mock szolgáltató (tesztábra, futó RP188-időkód, 1–16 csatorna 24 bites PCM a 29,97 Hz-es
1602/1601-es mintaszám-ütemmel, ANC-csomagok, beállítható elejtés és formátumváltás); fájlból visszajátszó
szolgáltató; `cfa4-ingest`: közvetlen YUV-bemenet a ProRes- és DNxHR-kódolóba (az RGBA-kerülő nélkül),
tömörítetlen v210 MOV-ba és MXF-be, többcsatornás egész PCM, időkódsáv, növekvő fájl, szegmentálás,
veszteségnapló, metaadat-jelentés; `io.*` parancsok a mock eszközzel.

Elfogadási tesztek:
- `cfa4-capture`: minden csomagoló bitazonos oda-vissza minden szélességre (a v210 6 pixeles csoportja,
  páratlan szélesség, sorvégi kitöltés); mutációs fuzz a gyűrű és a csomagolók bemenetén.
- `cfa4-ingest`: 10 perc mock 1080p50 v210 + 16 csatorna + RP188 → MOV és MXF; visszaolvasva a CFA4
  `MediaSource`-szal és az ffprobe-orákulummal: képkockaszám, kezdő időkód, hangminták száma
  (= képkockák × 960), képkockánkénti hash bitazonos.
- Elejtés-injektálás (1, 10, 100 képkocka): a jelentés pontosan ennyit és ott jelez; a fájl lejátszható,
  az idővonal-pozíció nem csúszik (pótolt vagy jelölt rés, beállítás szerint).
- Formátumváltás felvétel közben: új szegmens, a régi fájl lezárva.
- A motor megölése felvétel közben: a fájl a `media_watch` szerint újranyitható, legfeljebb az utolsó
  szegmens/partíció hiányzik (dokumentált határ).
- Becsült munka: 100–160 óra.

### F1b – DeckLink-bemenet (Q1 után; a kód hardver nélkül fordul, a teszt kártyát kér)

Feladatok: `cfa4-decklink` C++-réteg és kötés; `cfa4-io-host` és `cfa4-io` felügyelő; eszközfelderítés és
hot-plug (`IDeckLinkDiscovery`), képességek (`IDeckLinkProfileAttributes`, `DoesSupportVideoMode`), profilok;
bemenet 8/10 bites YUV-ben, automatikus formátumfelismeréssel; hang 2–16 (a kártya szerint 32/64)
csatorna 32 bites egészként; időkód (RP188 VITC1/VITC2/LTC, VITC), stream time Tick-időskálán,
hardveres érkezési idő; jelvesztés, formátumváltás, elejtés események; `io.devices.list` stb. valós
eszközzel.

Elfogadási tesztek (referenciagép, kártyával):
- **Hurokteszt:** a kártya saját kimenete (vagy egy második kártya) a bemenetre kötve; ismert tesztábra +
  sorszám a képben és ANC-ban, ismert hang (csatornánként más frekvencia és impulzus) → a rögzített adat
  bitazonos, a hang–kép eltolás mérve és állandó (a jegyzőkönyv rögzíti), az időkód folytonos.
- Kábelkihúzás, rossz formátum, a host megölése: esemény, nincs összeomlás, a felvétel lezárul.
- 2 órás rögzítés 1080p50 10 bit + 16 csatorna, majd 2160p50 10 bit (ha a kártya és a lemez bírja):
  0 megmagyarázatlan elejtés; a CPU-terhelés és a szerkesztő lejátszása mérve (a „nem lassít” szabály).
- Összevetés a GStreamer `decklinkvideosrc`-vel ugyanazon a jelen (orákulum).
- Becsült munka: 120–200 óra.

### F1c – DeckLink-kimenet és külső monitor

Feladatok: ütemezett kimenet (`ScheduleVideoFrame`, előtöltés, hangütemezés), kimeneti időkód és ANC;
alulcsordulás és késés jelzése; `io.monitor.set`: a Program (és Source) monitor képe a lejátszóból a kártyára,
A/V-késleltetés-kiegyenlítéssel; a kimeneti képkocka-időzítés a kártya órájához zárva.

Elfogadás: hurokteszt kimenet → bemenet bitazonos; 2 órás lejátszás alulcsordulás nélkül;
a szerkesztő lejátszása közben a külső monitor képe és hangja szinkronban (mérés villanás + sípszóval);
a `docs/hardware-parity.md` „External video monitoring” sora frissítve. Becsült munka: 80–140 óra.

### F2 – HDR és nagy bitmélység (az F1b után, az F3-mal párhuzamosan)

Feladatok: 10/12 bites RGB (r210, R12L) és YUV be/ki; HDR-metaadat be/ki (EOTF: SDR, PQ, HLG; ST 2086,
MaxCLL/MaxFALL) a kártya metaadat-interfészén és az ST 2108 ANC-n; leképezés a `cfa4-color`
színtereire és a `ColorInfo`-ra; rögzítés ProRes 4444 XQ 12 bit, DNxHR 444, FFV1 (ha a kódoló kész,
PROCODEC-002 2. csomag), tömörítetlen, EXR-sorozat; metaadat-megőrzési mátrix konténerenként
(MOV `mdcv`/`clli`, MXF ST 2067-21 mezők, EXR fejléc).

Elfogadás: hurokteszt 12 bites RGB-vel bitazonos; nincs észrevétlen bitmélység-csökkenés (ha a célformátum
kevesebbet tud, a rögzítés előtt figyelmeztet); a HDR-metaadat oda-vissza azonos, vagy a jelentés megnevezi;
numerikus összevetés (PSNR = ∞ a veszteségmentes utakon, a ProRes-en küszöb); EXR-út külön teszttel.
Becsült munka: 80–140 óra.

### F3 – ANC/VANC (az F1b után)

Feladatok: `cfa4-anc` teljes; DeckLink ANC-csomag API be és ki; ST 334 → `cfa4-captions` (CEA-608/708) →
feliratsáv a projektben és MCC-oldalfájl; AFD és bar data a klipmetaadatba; ATC időkód; ST 2038
MPEG-TS-be (`cfa4-mpegts`); ismeretlen csomagok átlátszóan oldalfájlba (MCC vagy saját ANC-napló);
kimeneten visszaírás; a `cfa4-st2110` -40 ugyanerre a kódra.

Elfogadás: ismert tesztvektorok (saját, a szabály szerint épített; a klvanc-tools csak összevetésre)
bitazonosan beolvasva; sérült csomag (rossz ellenőrzőösszeg, paritás, hossz) jelölve, nem összeomlás
(fuzz); rögzítés → export → visszaolvasás: a feliratok és az AFD megmaradnak; caption- és AFD-út külön
teszttel. Becsült munka: 80–140 óra.

### F4 – Genlock, időkód, többeszközös szinkron (PTP: a LIVE T1)

Feladatok: referenciajel állapota és kimeneti időzítési eltolás; `io.sync.status`: genlock, PTP (a T1
`CLOCK_TAI`/`ptp4l` olvasása), LTC/MTC (`cfa4-tcsync-io`), RP188 egy nézetben, külön-külön (a terv 5.6:
a PTP és a genlock nem helyettesítik egymást); szinkronvesztés-esemény; több kártya bemenetének
egymáshoz mért eltérése hardveres időbélyegből, naplózva; többkamerás rögzítés közös időkóddal
(a meglévő `multicam` szinkronjához).

Elfogadás: a terv F4-feltételei, a többeszközös teszt két kártyával és közös referenciával dokumentálva.
Becsült munka (a DeckLink-rész): 40–80 óra.

### F5 – ST 2110 (a LIVE T2–T6 fázisai; itt csak a híd)

Feladatok: az ST 2110 vevő és küldő `DeviceProvider`-ként a HAL-on (így az `io.*` parancsok és a rögzítő
ugyanúgy kezeli); SDI → ST 2110 és ST 2110 → SDI átjárás; ha a Q1 engedi, a Blackmagic IP-kártyák hardveres
ST 2110-útja mint másik szolgáltató (külön vizsgálat).

Elfogadás: a LIVE T2–T6 feltételei, valamint egy DeckLink-bemenet ST 2110-kimenetként egy külső vevőben.
Becsült munka (a híd): 30–60 óra.

### F6 – NMOS (a LIVE T7)

Feladatok: IS-04/IS-05 a T7 szerint; a DeckLink-portok NMOS-küldőként/-vevőként (SDI-portok mint
„devices”); IS-07 (tally és GPI események) és IS-08 csak igény esetén. Megvalósítás: saját Rust (a D8
mintájára) vagy a nmos-cpp külön folyamatként – a T7 dönt.
Elfogadás: AMWA nmos-testing eredmények a PR-ban (fejlesztői eszközként futtatva, nem függőség).
Becsült munka (a DeckLink-rész): 20–40 óra.

### F7 – Vezérlés, TSL UMD, tally, Key/Fill, többkimenet

Feladatok: `cfa4-control-io`: TSL UMD v3.1 (UDP), v4, v5 (UDP/TCP) be és ki, tally → a forrás/kimenet
állapota a felületen és a monitorokon; OSC és MIDI → parancs (beállítható leképezés, a parancsregiszteren
át); DeckLink belső/külső keyer; Key/Fill két szinkron kimeneten; több kimenet (program, előnézet) a stúdió
módhoz (LIVE S4 után), cue-vezérlés a műsortervvel (S4a).

Elfogadás: TSL UMD-teszteszközzel és egy valódi tally-forrással; a vezérlés csak parancsokat hív (teszt:
az adapter nem ér el állapotot parancs nélkül); Key/Fill fázishelyesség mérve a hurokteszten; két kimenet
egymáshoz mért eltérése 0 képkocka. Becsült munka: 100–180 óra.

### F8 – Rendszerszintű validáció és csomagolás

Feladatok: a terv F8-feladatai; csomagolás: `cfa4-io-host` a `.deb`-ben (a Desktop Video nem függőség,
csak a leírásban említve), AppImage/Flatpak: a rendszer `libDeckLinkAPI.so`-ja (Flatpakban a kernelmodul
miatt csak korlátozva, F0-ban vizsgálva); diagnosztika: `help.systemReport` DeckLink-szakasz (API-verzió,
eszközök, jelállapot), a host naplója a `Logs` könyvtárban.

**Együttélés (a terv 2.3 pontja, a csonka F8 kiegészítéseként):**
- a CFA4 nem ír a Desktop Video fájljaiba (`/usr/lib/libDeckLinkAPI.so`, `/usr/lib/blackmagic/`,
  `/etc/blackmagic/`, a kernelmodul), és nem módosítja a kártya globális beállításait kérés nélkül
  (a csatlakozóválasztás és a profil az eszközön globális: a CFA4 kiírja, ha más program is használja);
- binárisnév `cfa4-io-host`, beállítás a `~/.config/cfa4/` alatt, nincs D-Bus-név, nincs saját MIME;
- hálózati portok (TSL, OSC, NMOS) beállíthatók, alapból a gyakori ütköző programokéitól eltérők, foglaltság
  esetén kezelhető hiba;
- egyidejű futás az OBS-szel, a Blackmagic Media Expresszel és a GStreamerrel ugyanazon a gépen: a kártya
  foglaltsága (`bmdErrorDeviceBusy`-féle hiba) érthető üzenet, nem összeomlás;
- a CFA4 libobs-fordítása a DeckLink-plugin nélkül készül (a kártyához csak a `cfa4-io-host` nyúl).

Elfogadás: a terv F8-feltételei, a fentiekkel; Ubuntu 26.04-en, KDE-n (referenciagép) és egy GNOME-os
telepítésen. Becsült munka: 60–100 óra.

---

## VI. Függőségek és sorrend

```
F0 ──► F1a ──► F1b ──► F1c ──► F7 (Key/Fill: + LIVE S4)
              │  ▲      │
              │  Q1     ├──► F2
              │         └──► F3 ──► (LIVE T5: ST 2110-40 a cfa4-anc-on)
              └──► F4 (genlock: F1b után; PTP: LIVE T1)
LIVE T1–T6 ──► F5 (híd) ; LIVE T7 ──► F6
minden Qt6-panel: a Qt6 befejezése után (Q7)
```

Az F1a nem függ a Q1-től és a hardvertől, ezért a Q1 eldöntése alatt is haladhat: az eredménye (élő forrás,
natív rögzítő) az ST 2110-vevőnek és az élő stúdiónak is kell.

---

## VII. Becslés

| Fázis | Óra (nagyságrend) |
|---|---|
| F0 | 30–50 |
| F1a | 100–160 |
| F1b | 120–200 |
| F1c | 80–140 |
| F2 | 80–140 |
| F3 | 80–140 |
| F4 (DeckLink-rész) | 40–80 |
| F5, F6 (híd; a T-fázisok a LIVE-tervben) | 50–100 |
| F7 | 100–180 |
| F8 | 60–100 |
| **Összesen** | **≈ 740–1290** |

A hardveres mérések emberi jelenlétet kérnek (kábelezés, jelgenerátor); a becslés ezt nem tartalmazza.

---

## VIII. Donortriázs

### VIII.1 Módszer (F0.3)

Minden donorhoz egy sor a `docs/decklink-donors.md`-ben, rögzített adatokkal:

| Mező | Tartalom |
|---|---|
| Azonosság | pontos URL, alapértelmezett ág, vizsgált commit SHA, utolsó commit dátuma, archivált-e, a tervben szereplő szerep valóban megvan-e (pl. a „blackmagic-io” név a Desktop Video kernelmoduljáé is) |
| Licenc | a tároló licence **és fájlonként** a fejlécek (`licensing.md` 3.), almodulok és vendorált kód, a beágyazott SDK-másolat licence |
| Kategória | 1 alap / 2 kód- vagy komponensdonor / 3 algoritmus- és protokollreferencia / 4 tesztdonor (a terv 4.5) – vagy **5 kizárva** |
| Mit veszünk | konkrét fájl vagy csak a megoldás leírása; ha kód: a regiszter sora (`licensing.md` 5.) ugyanabban a commitban |
| Biztonság, karbantartás | CVE-k, utolsó kiadás, CI, tesztek |
| Együttélés | binárisnevek, portok, konfigurációs könyvtárak (ha futtatható eszköz) |
| Döntés | dátum, ki döntött |

Szabályok a sorokhoz:
- **SDK-másolatot tartalmazó tároló** (hhool, a DeckLinkCapture és mások, ha beágyazták az SDK-t) csak
  referenciaként; SDK-fájlt onnan nem veszünk át, csak a Blackmagictől letöltött, rögzített verzióból (Q1).
- **GPL-2.0-only, AGPL vagy licenc nélküli** donor: csak külső programként (teszteszköz) vagy egyáltalán nem;
  a forrását a megvalósítók nem olvassák kódátvétel céljából (`AGENTS.md` §2).
- **Windows-központú** részek (DirectShow, Media Foundation): kizárva (a terv 2.1).
- **Tesztdonor** (nmos-testing, SDPoker, TSL-UMD-Tester, klvanc-tools, smpte2110-tester): fejlesztői gépen,
  külső folyamatként fut, nem kerül a csomagba; a licence csak akkor számít, ha tesztvektort veszünk át belőle.
- A tényleges repository-adatok (licenc, commit) **ebből a munkamenetből nem voltak lekérdezhetők**
  (a GitHub API a munkamenet hozzáférési szabálya miatt nem érte el őket): ezek kitöltése az F0.3 feladata.

### VIII.2 Előzetes besorolás (az F0.3 igazolja vagy javítja)

| # | Donor | Előzetes kategória | Indok, teendő |
|---|---|---|---|
| 1 | Blackmagic DeckLink SDK | 1 (Q1-től függően) | a fejléc megengedő, de EULA-hoz kötött; a futtatókönyvtár zárt (E1) |
| 2 | DeckLinkCapture | 3 | callback- és felderítési minta; Qt5 GUI és kódoló nem kell |
| 3 | decklink-rs | 3, esetleg 2 | Rust-oldali erőforrás- és képkocka-minta; licenc és a C++-shim szerkezete az F0-ban; a bemenet hiányos |
| 4 | hhool SDK-másolat | 3 (csak dokumentáció) | régi SDK; fájlt nem veszünk át |
| 5 | manoreken2/projects | 3 | HDR10/PQ, r210/v210 minták; a Windows-részek kizárva; a csomagolók saját, bitazonos tesztekkel |
| 6 | bmdtools | 3 | azonosság, licenc, működés F0 |
| 7 | GStreamer DeckLink-plugin | 3 és 4 | LGPL: referenciának és **tesztorákulumnak** (E6); termékútként nem |
| 8 | blackmagic-io | F0 | a név ütközik a Desktop Video kernelmodul nevével: az azonosság az első kérdés |
| 9 | OBS DeckLink-plugin | 3 | GPL-2.0-or-later (a LIVE-terv szerint az OBS fő kódja az); hibakezelési és formátumfelismerési minta |
| 10 | pydecklink | 3 | prototípusminta; Python a médiaútban nem lehet (`AGENTS.md`) |
| 11 | pi-nmos-st2110 | 3 | a LIVE T-fázisaihoz |
| 12 | Deckboy | F0 | funkció és licenc nem igazolt |
| 13 | Frame Ferret | F0 | azonosság, licenc |
| 14 | libklvanc | 3, esetleg 2 | ANC/KLV referencia; ha LGPL, port megengedett a regiszterrel; a javaslat saját `cfa4-anc` a szabványokból |
| 15 | rust-broadcast | F0 | modulok és érettség |
| 16 | EBU LIST | 4 | elemző és mérőeszköz a T3/T4-hez; licenc F0 |
| 17 | statime | 3 | a D9 szerint nincs saját PTP-szolga; referencia |
| 18 | linuxptp | rendszerkomponens | a D9 szerint a rendszer csomagja, a CFA4 csak olvas |
| 19 | nmos-cpp | 2 vagy 3 | a T7 dönti el: külön folyamat vagy saját Rust |
| 20 | AMWA nmos-testing | 4 | megfelelőségi teszt, nem függőség |
| 21 | tslumd | 3 | protokollreferencia; a megvalósítás a nyilvános TSL-leírásból |
| 22 | tsl-umd-protocol | 3 | mint fent |
| 23 | TSL-UMD-Tester | 4 | interoperabilitási teszteszköz |
| 24 | SDPoker | 4 | SDP-ellenőrzés a T-fázisokhoz |
| 25 | smpte2110-tester | 4 | F0: tesztérték és licenc |
| 26 | klvanc-tools | 4 | ANC-tesztvektorok és -diagnosztika összevetésre |

---

## IX. Kockázatok

- **Licenc (E1):** ha a Q1 „nem”, a DeckLink kimarad; az F1a, az ST 2110 és a vezérlés ettől függetlenül értékes.
- **Kernel és driver:** a Desktop Video DKMS-modulja és a 7.0-s kernel; egy kernelfrissítés után a kártya eltűnhet.
  A CFA4 ezt felismeri és megnevezi, de nem javítja (nem nyúl a driverhez).
- **Valós idő Broadwell-magokon:** 2160p50 12 bites RGB rögzítése tömörítetlenül ~10 Gbit/s lemezre; a lemez
  és a NUMA-elhelyezés mérendő; tömörítéshez a ProRes-kódoló sebessége mérendő a referenciagépen.
- **Globális kártyabeállítások:** a csatlakozó, a duplex-profil és a referencia a kártyán globális; más program
  átállíthatja felvétel közben → esemény, nem csendes hiba.
- **Két rögzítő a gyakorlatban (E3):** ha a LIVE S5 a libobs felvételével indul, a „nincs párhuzamos motor”
  szabály sérül; a Q5 ezért a LIVE-tervvel együtt döntendő.
- **Hardveres teszt hiánya a CI-ben:** a hardveres elfogadás a referenciagépen, kézzel indított, rögzített
  jegyzőkönyvvel (a `docs/performance.md` mérési szabályai szerint); a CI a mock és a visszajátszó szolgáltatón fut.
- **Csonka terv (E13):** a hiányzó F8-szöveg módosíthatja az elfogadási feltételeket.

---

## X. Források

- A tulajdonos tervjavaslata: [`CFA4-DECKLINK-001-1.0.md`](CFA4-DECKLINK-001-1.0.md) (2026-11-14).
- CFA4: `AGENTS.md` (CFA4-szakasz, §0, §2), [`docs/licensing.md`](../licensing.md) 3., 4., 7. pont,
  [ADR 0001](../adr/0001-platform-ffi.md) (2026-11-07 és 2026-11-09 kiegészítés),
  [ADR 0002](../adr/ADR-0002-gstreamer-backend.md), [ADR 0005](../adr/ADR-0005-media-backends.md),
  [ADR 0006](../adr/ADR-0006-backlothub-postgresql.md), [CFA4-LIVE-STUDIO-001](CFA4-LIVE-STUDIO-001.md),
  [ST 2110 kiegészítés](CFA4-LIVE-STUDIO-001-ST2110.md), [CFA4-PROCODEC-002](CFA4-PROCODEC-002-2026-11-11.md)
  11. munkacsomag (BRAW-precedens), [TIMECODE-SYNC](TIMECODE-SYNC-LTC-MTC-2026-11-11.md),
  [`docs/hardware-parity.md`](../hardware-parity.md), [`docs/architecture.md`](../architecture.md) 1.,
  [`docs/development/reference-machine.md`](../development/reference-machine.md).
- FFmpeg `configure`: a `decklink` az `EXTERNAL_LIBRARY_NONFREE_LIST`-ben (ellenőrizve 2026-11-14, `master`).
- GStreamer: `subprojects/gst-plugins-bad/sys/decklink/linux/DeckLinkAPI.h` fejléce (Blackmagic Design, 2021:
  megengedő engedély, a Blackmagictől szerzett példányra a DeckLink SDK EULA-ra hivatkozva), ellenőrizve 2026-11-14.
- Ubuntu 26.04 (resolute) `gstreamer1.0-plugins-bad` fájllistája: `libgstdecklink.so` (ellenőrizve 2026-11-14).
