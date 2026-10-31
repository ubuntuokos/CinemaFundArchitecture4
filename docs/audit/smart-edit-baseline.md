# CFA4 Smart Cut / Smart Render – kiindulási audit (WP-00)

**Specifikáció:** [`CFA4-SMART-CUT-RENDER-R1`](../plans/CFA4-SMART-CUT-RENDER-R1.md) · **Munkacsomag:** WP-00
**Dátum:** 2026-10-27 · **Bázis:** `main` @ `f49a5def4d16d04d217b604d388c8eece834d48a`
(„Merge pull request #205 from ubuntuokos/qt/f8-timeline-port”, 2026-10-27 03:43 +0100)
**Fejlesztési ág:** `feature/native-smart-cut-render`
**Módszer:** forráskód- és dokumentációolvasás. Ez az audit nem fordít, nem futtat tesztet és nem
mér; minden állítás mellett ott a forrás, ahol ellenőrizhető.

## 0. Összefoglalás

- A CFA4-ben **ma nincs Smart Cut, Smart Render vagy stream-copy export.** Az exportút mindig
  renderel és újrakódol (`crates/export`). A `docs/roadmap.md` M6-sora és a `docs/gaps.md` a
  „smart render”-t kifejezetten hiányzó funkcióként sorolja fel.
- Az építőkövek nagy része megvan, saját, tiszta Rust-implementációban: demuxerek (MP4/MOV,
  Matroska/WebM, MPEG TS/PS, MXF), dekóderek (MPEG-2, H.264, HEVC, AV1), MP4/MOV és Matroska
  muxer, amely kész (előre kódolt) mintákat is elfogad, saját H.264- és AAC-encoder, GOP-tudatos
  keresés nyitott GOP-kezeléssel.
- Hiányzik: tartós, képkockaszintű GOP/hozzáférésipont-index, referenciafüggőség-gráf,
  packet-copy végrehajtás az exportban, MPEG-2 encoder, MPEG TS/PS muxer, MPEG-2 videó MP4-mintabejegyzés,
  szoftveres HEVC/AV1 encoder, a Qt6 idővonal GOP-overlay-e és az export módválasztója.
- **FFmpeg, eldöntve:** a terv „Native FFmpeg 9 Adapter”-t (libavformat, libavcodec, …) ír elő.
  **Tulajdonosi döntés (2026-10-27): a Smart Cut / Smart Render saját Rust végrehajtással készül**,
  az FFmpeg 9 itt csak validációs orákulum (§3). Ez a funkció tervezési döntése, nem tárolószintű
  tiltás: a repository szabályai (`AGENTS.md`, [ADR 0005](../adr/ADR-0005-media-backends.md))
  az FFmpeg ≥ 9.0.2-t kiegészítő háttérként engedik a `cfa4-route` útválasztón át (§3, utóirat).
- MLT-integráció nincs a kódban, így a terv MLT-re vonatkozó kikötése ma nem érint semmit (§8).

## 1. Natív média- és exportmodulok

Rétegek: `docs/architecture.md` §1, ellenőrzi a `cargo xtask layers` (`xtask/src/main.rs`, `LAYERS`).

| Terület | Crate (réteg) | Smart Edit szempontjából fontos tartalom |
|---|---|---|
| Időábrázolás | `time` (L0) | `Tick` = 1/254 016 000 000 s egész szám, `FrameRate { num, den }` racionális, `TimeRange`. Az f64 másodperc tiltott a szerkesztési matekban (CLAUDE.md). |
| MPEG-2 videó | `mpeg2v` (L0) | Sequence header (`probe`, `SequenceInfo`), GOP header `closed_gop` / `broken_link` (`headers.rs:300`), képtípus a `PictureHeader`-ben, `scan_access_unit` → `AccessUnitInfo { sequence_header, gop, pictures }`, `is_intra()`, `is_disposable()`, `access_units(es)`. A dekóder a B-képeket zárt GOP nélkül eldobja, ha hiányzik a referencia (`lib.rs:637`). |
| H.264 | `h264` (L0) | NAL-típusok (`slice::nal_type`, IDR = 5), `params` (SPS/PPS), slice header `idr`, `idr_pic_id`, POC-számítás, `dpb` (IDR-kezelés, `no_output_of_prior_pics`). Recovery point SEI felhasználása nem látszik. |
| HEVC | `hevc` (L0) | `params` (VPS/SPS/PPS), `slice`, `dpb`, Main/Main 10 dekódolás. |
| AV1 | `av1` (L0) | `SequenceHeader`, `FrameHeader`, `is_key_frame_unit(data)` (`lib.rs:82`), bit-exakt dekóder. |
| H.264 encoder | `h264enc` (L0) | Baseline/Main/High, 4:2:0 8 bit, B-képek 0–3, `keyint`, CRF/CBR/VBR/QP, Annex B vagy hosszelőtagos csomag, `avcc()`; csomagonként `pts`, `dts`, `keyframe`, `frame_type`. |
| MPEG rendszer | `mpegts` (L0) | TS (188/192/204 bájt) és PS (MPEG-1/2 PS, VOB, MOD) **demuxer**, PES, PTS/PCR, hozzáférésiegység-index megnyitáskor. Muxer nincs. |
| MP4/MOV | `isobmff` (L0) | Demux és mux: `Mp4Writer::write_sample(track, WriteSample { data, duration, composition_offset, is_sync })`, fragmentált író, edit list. Mintabejegyzések: AVC, HEVC, AV1, VP9, ProRes, MJPEG, DNx, APV, AAC, MP3/MP2, PCM, ALAC… **MPEG-2 videó nincs** (`codec.rs:164`). |
| Matroska | `matroska` (L0) | Demux és `MkvWriter` mux; a mux az exportból **nem** érhető el (`Multiplexer` csak `Mp4`/`Mov`, `export/src/settings.rs:124`). |
| MXF | `mxf` (L0) | OP1a / OP-Atom demux és mux, AVC / VC-3 / ProRes / MPEG-2 azonosítás. |
| Konténer- és codec-hub | `codecs` (L2) | `VideoSamples` trait (`gop.rs:114`: `count`, `pts`, `sync_before`, `sample_at`, `read`, `make_decoder`), `decode_start` nyitott GOP vezető képeire (`gop.rs:127`, pl. HEVC CRA + RASL), `GopCache` dekódolt-képkocka cache, dekóder-regiszter, hardveres dekódolás (`hw.rs`). |
| Médiaforrások | `media` (L1) | `MediaSource` trait, probe/openerek, frame cache. |
| Projekt és formátum | `project` (L1), `format` (L3) | Dokumentummodell; `.fcproj` séma `SCHEMA_VERSION = MIGRATIONS.len() + 1`, migrációs lánc, `TooNew`/`TooOld` hibák, atomikus mentés (`format/src/lib.rs:66`). Új Smart Export beállítás itt kap migrációt. |
| Szerkesztési algebra | `edit` (L2) | insert, overwrite, razor, ripple, roll, slip, slide, rate stretch, lift, extract. |
| Export | `export` (L3) | `Format` enum (`lib.rs:64`; H.264, HEVC, AV1, ProRes, DNxHR, APV, MJPEG, képsorozatok, GIF, WAV, AIFF, AAC, FLAC, MXF), `VideoEncoder` trait (`lib.rs:737`), `register_encoder`, `register_format_probe` + `available(format)` (futásidejű elérhetőség, `lib.rs:1009–1046`), `hdr_available`, `Exporter` / `Step` léptetett job (`job.rs`), progress és cancel atomikus jelzőkkel, `ExportError::{Unsupported, Cancelled, …}`. |
| Motor | `engine` (L4) | `Session`, parancsregiszter, `jobs` (háttérfeladatok), `export_tools.rs`: `export.resolve`, `export.presets.*`, `export.queue.*`, `export.formats`. |
| Hardver | `platform` (L5) | NVENC H.264/H.265, VA-API dekódolás; HEVC exportot csak hardveres encoder regisztrál. AV1 encodert semmi nem regisztrál (`export/src/settings_tests.rs`: `Format::Av1` nem elérhető). |
| Útválasztó | `route` (L1) | ADR 0004 számítási útválasztó; az `ffmpeg` háttér a táblában `NotRegistered`. |
| Qt6 felület | `python/cfa4_qt` | PySide6 kliens; a `cfa4-cli serve` Rust motorral stdin/stdout JSON-sorokon beszél (`engine.py`, protokoll: `docs/qt-gui.md`). |

**Következmény a Smart Edit helyére:** a tervező (index, terv, döntések) új, tiszta Rust crate-be
kerüljön, amely nem függ a rendertől és a felülettől. Javasolt név és réteg: `crates/smart-edit`
(`cfa4-smart-edit`), **L3**, mert a WP-02 indexelője a `codecs` (L2) mintatábláira épül, és így nem
kell új azonos-rétegű élet engedélyezni. A végrehajtás (WP-04/06) a meglévő `export` jobba és az
`engine` parancsaiba kapcsolódik, nem külön exportszolgáltatásként (terv §3). A crate-et fel kell
venni a `LAYERS` táblába és a `docs/architecture.md` §1-be, és wasm-hordozhatónak kell maradnia
(`cargo xtask wasm` az L0–L4 crate-eket ellenőrzi).

## 2. Git-állapot

| Adat | Érték |
|---|---|
| Alap | `main` @ `f49a5def4d16d04d217b604d388c8eece834d48a` |
| Fejlesztési ág | `feature/native-smart-cut-render` (ebből a commitból) |
| Toolchain | Rust 1.97.1 (`rust-toolchain.toml`) |
| Kötelező PR-kapu | `.github/workflows/pr-check.yml`: `cargo metadata --locked`, `cargo check --workspace` (csak `main` célú PR-on) |
| Teljes validáció | kézi `workflow_dispatch` vagy `cfa4-full-validation` címke (AGENTS.md) |

## 3. FFmpeg 9 runtime-linkage

**Eredmény: a CFA4 termék ma nem linkel FFmpeg-et.** Az alábbi felsorolás az audit idején
olvasott szabályszöveget rögzíti; a jelenlegi szabályt lásd a szakasz végén, az utóiratban.

- `AGENTS.md` §2: „ffmpeg/ffprobe may be used only as external test oracles and fixture
  generators. They are never linked, bundled or shipped.” Ugyanez a `CLAUDE.md` „Clean-room”
  pontjában.
- `docs/adr/ADR-0003` (157., 223. sor): az FFmpeg a termékbe csak saját ADR-rel kerülhet; addig
  külső tesztorákulum.
- `docs/adr/ADR-0004-compute-routing.md` (165–169. sor) már rögzíti ezt az ütközést: az `ffmpeg`
  háttér a táblában szerepel, de `NotRegistered`; előbb a szabályt kell módosítani, és dönteni kell
  a kapcsolódás módjáról (dinamikus linkelés vagy külső folyamat) és az FFmpeg-build licencéről.
- `docs/licensing.md` §7: az FFmpeg-háttér nem lehet kötelező linkfüggőség; az x264/x265 a külön
  `cfa4-restricted-extras` csomagba tartozik.
- Az `unsafe` csak a `crates/platform`-ban engedett (ADR 0001); libav FFI ehhez kötődne.
- CI-orákulum: `scripts/ci/install-ffmpeg-9.0.2.sh` külön, statikus, orákulumcélú FFmpeg 9.0.2.
  Az auditkörnyezetben (felhőkonténer) ffmpeg 6.1.1 és `libavcodec.so.60` van, ez nem a célplatform.

**Ütközés a tervvel:** a terv §1 („Natív multimédia-feldolgozás: FFmpeg 9 … libav API-k”), §3
(„Native FFmpeg 9 Adapter”) és WP-04 („FFmpeg 9 packet-copy”) a mai szabályokkal nem
valósítható meg. A VISION.md szerint az ütközést a tulajdonosnak kell eldöntenie. Két út:

1. **Saját Rust végrehajtás (a mai szabályokkal összhangban):** demux, packet-copy, mux és
   határszakasz-újrakódolás a meglévő crate-ekkel; az FFmpeg 9 csak validációs orákulum
   (ffprobe, dekódolhatóság, payload-összevetés). Hiányzik hozzá: MPEG TS/PS muxer, MPEG-2
   encoder, MP4 MPEG-2 mintabejegyzés, szoftveres HEVC/AV1 encoder.
2. **FFmpeg 9 háttér új ADR-rel:** opcionális, rendszerből dinamikusan linkelt libav (vagy külső
   folyamat), a saját motor tartalékként; szabálymódosítás az AGENTS.md-ben és a CLAUDE.md-ben.

A WP-01 és WP-02 mindkét úton azonos (háttérfüggetlen nyilvántartás és saját bitstream-elemzés).

**Döntés (ubuntuokos, 2026-10-27): 1. út, saját Rust végrehajtás.** A packet-copy, a mux és a
határszakasz-újrakódolás a CFA4 saját crate-jeivel készül; az FFmpeg 9 kizárólag validációs
orákulum (ffprobe, dekódolhatóság, payload-összevetés). Ehhez szabálymódosítás nem kell; a terv §3 „Native FFmpeg 9 Adapter” rétegét a meglévő `export` pipeline és a saját
muxerek töltik be. Ehhez hiányzik és külön munkacsomagban pótlandó: MPEG TS/PS muxer, MPEG-2
encoder (az MPEG-2 Smart Renderhez), MP4 MPEG-2 mintabejegyzés, szoftveres HEVC encoder (a
HEVC Smart Render addig csak hardveres encoderrel, képességvizsgálat után), és AV1 encoder
(se szoftveres, se hardveres integráció nincs; az AV1 Smart Render addig nem lehetséges).

**Utóirat (2026-10-31): a jelenlegi tárolószabály.** A `main` mostani `AGENTS.md`-je és az
elfogadott [ADR 0005](../adr/ADR-0005-media-backends.md) szerint az FFmpeg ≥ 9.0.2 (GPL-3-as build,
`nonfree` nélkül) a tervezett `cfa4-ffmpeg` crate-en át **engedélyezett kiegészítő háttér**. A
hívó kód nem éri el közvetlenül, csak a `cfa4-route` útválasztón keresztül, és a saját motor mindig
tartalék. A fenti, „tiltja” értelmű pontok (AGENTS.md §2, ADR 0003/0004 korábbi szövege) ezért ma
nem tárolószintű akadályok, és új ADR sem kell hozzájuk. A Smart Cut / Smart Render végrehajtására
ettől függetlenül a tulajdonos 2026-10-27-i döntése érvényes: saját Rust út, az FFmpeg 9 validációs
orákulum. Az FFmpeg-háttér a capability registrybe (`Backend::Ffmpeg`) csak a tulajdonos újabb
döntésével, a `cfa4-ffmpeg` elkészülte után, az ADR 0005 szerint kerülhet be; a registry ezt
modellváltás nélkül befogadja.

## 4. Meglévő stream-copy képességek

| Képesség | Állapot | Bizonyíték |
|---|---|---|
| Stream-copy / packet-copy export | **nincs** | `crates/export` minden formátumot renderel és kódol; `Format`-ban nincs copy mód |
| Remux kész mintákból MP4/MOV-ba | építőkő megvan, csak tesztben | `crates/isobmff/tests/oracle_mux.rs` (H.264+AAC, HEVC, AV1, VP9, ProRes, edit list…; ffprobe-bal ellenőrizve) |
| Codec-konfiguráció átvitele remuxkor | megvan (MP4) | `crates/isobmff/README.md`: „Remuxing copies codec config, colr, pasp, clap, fiel, gama and btrt” |
| Matroska mux | megvan, exportból nem elérhető | `crates/matroska/src/mux.rs`, `codecs/src/mkv.rs` tesztjei |
| MPEG-2 PS/TS mux | **nincs** | `crates/mpegts` csak demuxer |
| MPEG-2 videó MP4-ben | **nincs** mintabejegyzés | `crates/isobmff/src/codec.rs:164` |
| MXF mux | megvan (AVC, VC-3, ProRes, PCM) | `crates/mxf`, `export/src/mxf_out.rs` |

## 5. Keyframe-indexelés és navigáció

- Megvan: konténerenkénti mintatábla `sync_before` (szinkronminta), `sample_at`, pts, és a
  `decode_start` nyitott GOP vezető képeire (`codecs/src/gop.rs:127`). A `GopCache` a lejátszáshoz
  és exporthoz szinkronmintától dekódol előre, és cache-el.
- MPEG-2: GOP header `closed_gop` / `broken_link` elemzés megvan; a TS/PS demuxer
  hozzáférésiegység-indexet épít megnyitáskor.
- **Hiányzik:** tartós, verziózott, forrásazonosítóhoz kötött index; képkockánkénti I/P/B és
  referenciaállapot a szerkesztő számára; H.264 recovery point; HEVC IDR/CRA/BLA és RASL/RADL
  megkülönböztetése index szinten (a dekóder belül kezeli); AV1 show-existing-frame / order-hint
  index; cache-érvénytelenítés (méret, mtime, tartalomazonosító).
- A szinkronminta-jelző (konténer „sync sample”) a terv szerint önmagában nem elég bizonyíték
  (SC-02); a WP-02-nek a bitstreamből kell igazolnia.

## 6. Qt6 idővonal és exportpanel

- Idővonal: `python/cfa4_qt/panels/timeline.py`, `timeline_media.py` (F8 port, #205). Kulcskép-,
  GOP- vagy biztonságosvágáspont-jelölés nincs.
- Export: `python/cfa4_qt/panels/export.py`. Mindent a motortól kérdez (`export.presets.list`,
  `export.formats`, `export.resolve`, `export.queue.*`), maga nem számol beállítást. Smart
  Export módválasztó és renderterv-előnézet nincs. Ez jó illesztési pont: az új módok és a terv
  a motor `export.*` parancsain keresztül érkezhetnek.
- Az egui-felület be van fagyasztva (VISION.md 2.1): új funkció csak a Qt6-ba kerülhet.

## 7. Export- és reimport-tesztek

| Teszt | Mit igazol |
|---|---|
| `crates/export/src/tests.rs` | MJPEG/ProRes/ProRes 4444/DNxHR export → saját dekóderrel visszaolvasva; H.264 feliratbeégetés ffmpeg-orákulummal; HDR PQ/HLG jelzés; `colr` |
| `crates/export/src/determinism_tests.rs` | bájtazonos export (#19) |
| `crates/export/src/surround_tests.rs` | 5.1 WAV/MOV/AAC, ffmpeg mintapontos dekódolással |
| `crates/isobmff/tests/oracle_mux.rs` | remux ffprobe-validációval |
| `crates/codecs/tests/*oracle*.rs` | demux/dekódolás ffmpeg ellen (MP4, MPEG, MXF, Matroska…) |
| `apps/cfa4-cli/tests/workflows.rs` | import → szerkesztés → export munkafolyamatok |

Az orákulumtesztek ffmpeg nélkül kihagyódnak (`cfa4_testkit::require_ffmpeg!`), CI-ben
`CFA4_REQUIRE_ORACLES=1` teszi kötelezővé (`docs/testing.md` §2). Fixture-ök ffmpeggel készülnek
a `target/fixtures/` alá, médiafájl nem kerül commitba. **Hiányzik:** payload-összevető
(copy-integritás), képkockapontos vágáshatár-teszt, A/V-szinkron mérés
(`docs/audit/f7-media-capability-status.json`: `av_sync_precision: NOT_MEASURED`).

## 8. MLT-adatfolyam

A Rust- és Python-kódban nincs MLT-integráció (csak dokumentációs említés: `docs/kdenlive-parity.md`,
ADR 0003, és az OTIO-csere a `crates/interchange`-ben). A terv §3 MLT-kikötése ezért ma nem érint
kódot; az egységes edit-leírás forrása a CFA4 saját projektmodellje (`project`, `edit`).

## 9. Meglévő és hiányzó képességek – codecenként (terv §6)

Jelölés: ✅ megvan, 🟡 részben / csak építőkő, ❌ nincs.

| Képesség | MPEG-2 | H.264 | HEVC | AV1 |
|---|---|---|---|---|
| Demux (konténerek) | ✅ PS/TS, MXF, MP4/MOV/MKV | ✅ MP4/MOV/MKV/TS/MXF | ✅ MP4/MOV/MKV/TS | ✅ MP4/MKV/WebM |
| Dekódolás (saját) | ✅ | ✅ | ✅ Main/Main 10 | ✅ Main |
| Paraméter-/fejlécelemzés | ✅ sequence/GOP header | ✅ SPS/PPS, IDR | ✅ VPS/SPS/PPS | ✅ sequence/frame header |
| Nyitott/zárt GOP, RA-típus index | 🟡 GOP header | 🟡 IDR, recovery point ❌ | 🟡 dekóderben (CRA/RASL) | 🟡 `is_key_frame_unit` |
| Referenciafüggőség-gráf | ❌ | ❌ | ❌ | ❌ |
| Packet-copy export | ❌ (muxer sincs) | 🟡 MP4/MOV muxer kész | 🟡 MP4/MOV muxer kész | 🟡 MP4 muxer kész |
| Encoder határszakaszhoz | ❌ | ✅ `h264enc` (4:2:0 8 bit) | 🟡 csak hardveres | ❌ nincs (hardveres sem) |
| Smart Render | ❌ | ❌ | ❌ | ❌ |

## 10. Kapufeltétel és következő lépések

A WP-00 kapufeltétele (egyértelműen dokumentált meglévő és hiányzó képességek) a fenti 1–9.
pontokkal teljesül. A végrehajtás módjáról a tulajdonos döntött: saját Rust végrehajtás (§3).

1. **WP-01 – Capability Registry** (`crates/smart-edit`, `capabilities`): háttérfüggetlen,
   determinisztikus nyilvántartás a fenti táblával mint kezdőállapottal; futásidejű probe-ok
   (`export::available` mintájára); FFmpeg-háttérsor nincs: a Smart Cut saját Rust úton fut
   (tulajdonosi döntés, §3). Az ADR 0005 szerinti `cfa4-ffmpeg` elkészülte után, a tulajdonos
   döntésével, sorként felvehető.
2. **WP-02 – Media/GOP Indexer:** MPEG-2 és H.264 első körben, a meglévő `mpeg2v` és `h264`
   parserekre építve, a `codecs::VideoSamples` mintatábláiból.
3. Projektséma-bővítés (Smart Export beállítás) csak `format` migrációval, visszafelé kompatibilisen.
