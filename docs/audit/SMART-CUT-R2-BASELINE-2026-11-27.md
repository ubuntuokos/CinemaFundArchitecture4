# Smart Cut R2 – R2-00 kiindulási audit (2026-11-27)

Terv: [`CFA4-SMART-CUT-RENDER-R2`](../plans/CFA4-SMART-CUT-RENDER-R2.md), R2-00. A mérés a `main`
`9f38bfe` állapotán készült. A motor a `cargo build --release -p cfa4-cli` paranccsal készült,
és a `cfa4-cli run` jsonl-szkriptjeivel futott, ugyanúgy, mint a WP-10 csomagpróbája.

A minták a projekt saját fizikai mintái (Altitude16, öt fájl). Nincsenek a repóban, és nem is
kerülnek bele (`docs/testing.md`). A futás felhős konténerben volt (4 mag), nem a referenciagépen.
Az idők ezért csak tájékoztatók. A referenciagépes mérés az R2-07 része.

## 1. A minták

ffprobe (FFmpeg 6.1) mérése alapján.

| Fájl | Videó | Kulcskocka-távolság | Hang | Hangcsomag |
|---|---|---|---|---|
| Dolby Atmos – 16-9.mkv | HEVC Main 8 bit, 3840×2160, 24000/1001, BT.709 | 1,251 s | TrueHD + Atmos, 7.1, 48 kHz | 1 access unit (~0,83 ms); major sync ~106 ms-onként (29 a 3 600-ból) |
| Dolby Atmos – 2.35.mkv | HEVC Main, 3840×1634 | 1,251 s | TrueHD + Atmos, 7.1 | ugyanaz |
| DTS-X – 2.35.mkv | HEVC Main, 3840×1634 | **8,342 s** | DTS-HD MA 7.1 (DTS:X) | 10,7 ms, mind kulcskocka |
| Auro-3D – 2.35.mkv | HEVC Main, 3840×1634 | 1,251 s | DTS-HD MA 7.1 (Auro-3D) | 10,7 ms |
| Dolby Atmos DD – 16-9.mp4 | H.264 High, 3840×2160 | 6,006 s | E-AC-3 + Atmos (JOC), 5.1(side) | 32 ms, mind kulcskocka |

Dolby Vision vagy HDR-metaadat egyik videóban sincs (`color_transfer` BT.709 vagy ismeretlen,
nincs DOVI side data).

## 2. Mit csinál ma a Smart Cut

Mind az öt fájlon ugyanaz a szkript futott:

1. `export.analyze`;
2. `export.plan` és `export.start` GOP-határon (`in` 144, `out` 432);
3. a kimenet `export.analyze`-a;
4. az MP4-en ezen felül képkocka-pontos vágás is (`in` 150, `out` 300, `confirm`).

| Fájl | Eredmény |
|---|---|
| a négy MKV | **elutasítva az első lépésben:** „not an MP4 / MOV file Smart Cut can read (no moov box found)”. Az `engine/src/smart_cut.rs` `open_source` csak ISOBMFF-et nyit, az MPEG PS/TS-t külön út kezeli. |
| MP4, GOP-határ | ✅ 288 kép másolva, `verified: true`, 66 008 945 bájt, a kimenet újraolvasva 288 kép. **A hang kimarad:** `dropped: [{trackId: 2, reason: "This audio is not copied yet"}]`. A kimenet csak videó (ffprobe: egy h264 folyam). |
| MP4, képkocka-pontos | ✅ 9 kép másolva, 141 újrakódolva (4K H.264), `verified: true`, `minPsnr` 50,4 dB, `boundaryReencode.ready: true`. A hang itt is kimarad. |

Az MP4-szkript a négy MP4-lépéssel és a kimenetek újraolvasásával együtt 49 s alatt futott le.

## 3. Hiányok, a terv munkacsomagjai szerint

| # | Hiány | Hol | WP |
|---|---|---|---|
| 1 | AC-3, E-AC-3, DTS és TrueHD hang nem másolható. Az `audio_copy::plan_track` csak AAC-t és PCM-et ismer. | `crates/export/src/audio_copy.rs` | R2-01, R2-02 |
| 2 | A kihagyás oka nem nevezi meg a kodeket („This audio…”). A `codec_name` csak MP3-at, PCM-et és AAC-t nevez meg. Ez megtévesztő a felhasználónak, ezért az R2-02 javítja. | `audio_copy.rs` `codec_name` | R2-02 |
| 3 | Nincs hang-hozzáférésipont index. A TrueHD csak major syncnél kezdhető (a mintákon ~106 ms-onként), és az AC-3, az E-AC-3 és a DTS átfedő transzformációja egy előző keretet kér. | `crates/smart-edit` | R2-01 |
| 4 | Matroska forrás nem nyitható. | `engine/src/smart_cut.rs` `open_source` | R2-03 |
| 5 | Matroska kimenet nincs bekötve. A registry szerint az `MkvWriter` „not reachable from export”. | `smart-edit/src/capabilities.rs` | R2-04 |
| 6 | A H.264/HEVC MKV-demux csak Experimental, orákulumteszt nélkül. | `capabilities.rs` | R2-03 |
| 7 | Hosszú GOP: a DTS-X mintán 8,3 s (200 kép). Képkocka-pontos vágásnál ennyi 4K HEVC kép újrakódolása kell. | – | R2-04, R2-07 (mérés) |

## 4. Ami már megvan és felhasználható

- MP4 mintabejegyzések olvasása és írása: `ac-3`/`dac3`, `ec-3`/`dec3`, `dtsc/dtsh/dtsl`/`ddts`, `mlpa`/`dmlp` (`crates/isobmff/src/codec.rs`).
- Matroska kodekazonosítók: `A_AC3`, `A_EAC3`, `A_DTS`, `A_TRUEHD` (`crates/matroska/src/codec.rs`). Matroska író: `MkvWriter`.
- Dekóderek a bitpontos ellenőrzéshez (R2-07): `cfa4-ac3`, `cfa4-dts`, `cfa4-truehd`.
- Az R1 AAC-útja (előtekercselő csomag + edit list + `verify_audio_tracks`), amire az R2-02 épül.
