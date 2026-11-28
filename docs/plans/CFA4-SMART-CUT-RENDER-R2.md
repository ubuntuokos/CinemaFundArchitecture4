# CFA4 – Smart Cut R2: Matroska és bitstream-hang (Atmos, DTS:X, E-AC-3)

**Specifikáció:** `CFA4-SMART-CUT-RENDER-R2`
**Dátum:** 2026-11-27
**Állapot:** Jóváhagyva 2026-11-27 (István, a projektszálban): a terv ebben a formában, sorrend: előbb a hang MP4-ben (5. fejezet, 2. döntés).
**Előzmény:** `docs/plans/CFA4-SMART-CUT-RENDER-R1.md` (WP-00…WP-10 kész, 2026-11-11), `docs/plans/CFA4-SCE-001-STREAM-COPY-EXPORT-2026-10-27.md` P6 (kodek- és konténerbővítés).

---

## 0. Miért ez a következő lépés

Az R1 program kész, a nyitott maradékok is bent vannak a `main`-ben:

- az `mpeg2v` `Config::low_delay` motorbekötése: `crates/export/src/boundary_reencode_mpeg.rs:275`;
- a #982 (i18n, 32 nyelv): beolvasztva 2026-11-11.

A projekt saját mintafájljai viszont ma **egyike sem vágható Smart Cuttal veszteségmentesen, a hangjával együtt**. A projekt mappájában öt Altitude16 minta van. Az alábbi adatok ffprobe-mérésből (2026-11-27) és a kód olvasásából származnak.

| Fájl | Videó | Hang | Kulcskocka-távolság | Mi történik ma |
|---|---|---|---|---|
| Dolby Atmos – 16-9.mkv | HEVC Main 3840×2160, 23.976 | TrueHD + Atmos 7.1 | 1,25 s | elutasítva: a Smart Cut csak MP4/MOV-t és MPEG PS/TS-t olvas (`engine/src/smart_cut.rs:149`) |
| Dolby Atmos – 2.35.mkv | HEVC Main 3840×1634 | TrueHD + Atmos 7.1 | 1,25 s | elutasítva (ugyanaz) |
| DTS-X – 2.35.mkv | HEVC Main 3840×1634 | DTS-HD MA 7.1 (DTS:X) | **8,3 s** | elutasítva (ugyanaz) |
| Auro-3D – 2.35.mkv | HEVC Main 3840×1634 | DTS-HD MA 7.1 (Auro-3D) | 1,25 s | elutasítva (ugyanaz) |
| Dolby Atmos DD – 16-9.mp4 | H.264 High 3840×2160 | E-AC-3 + Atmos (JOC) 5.1 | 6,0 s | a videó vágható, **a hang kimarad**: „E-AC-3 audio is not copied yet” (`export/src/audio_copy.rs:149`, csak AAC és PCM) |

A kapcsolódó alapelemek nagy része már megvan, csak nincs bekötve a Smart Cutba:

- Matroska demuxer és író: `cfa4-matroska`, `MkvWriter`. A registry szerint „not reachable from export”, a H.264/HEVC MKV-demux pedig Experimental.
- MP4 mintabejegyzések: `ac-3`, `ec-3`, `dtsc/dtsh/dtsl` és `mlpa` olvasása és írása (`isobmff/src/codec.rs`).
- Bitstream-elemzők: `cfa4-ac3` (E-AC-3), `cfa4-dts` (DTS-HD MA, DTS:X alap), `cfa4-truehd` (TrueHD/MLP, OAMD).
- HEVC és H.264 határ-újrakódolás.

**A terv ezt a hiányt zárja be: MKV és MP4 Smart Cut az R1 garanciáival (bitazonos másolás, ellenőrzés írás előtt, A/V szinkron), a nagy felbontású filmhang újrakódolása nélkül.**

## 1. Hatókör

**Benne van:**

1. Matroska forrás: H.264, HEVC, AV1 és MPEG-2 videó, Smart Cut (GOP-határ és képkocka-pontos).
2. Matroska cél, és MKV → MKV másolás Strict módban.
3. Bitstream-hang másolása vágással, dekódolás és újrakódolás nélkül, MP4-be és MKV-be:
   - AC-3;
   - E-AC-3, benne az Atmos JOC;
   - DTS: core, DTS-HD HRA/MA, és a DTS:X és Auro-3D kiterjesztések átlátszó bájtokként;
   - Dolby TrueHD, benne az Atmos.
4. Idővonalas Smart Render MKV-forrásokkal (a WP-06c útján). A hang itt is másolt, ha a szakasz hangja érintetlen.
5. Qt6: a forrásmonitor Smart Cut és a Smart Export elfogadja az `.mkv` fájlt. Kimeneti konténer választható. Sávonkénti hangjelentés: „másolva” vagy „kihagyva, mert…”.

**Nincs benne (kizárás):**

- Atmos, DTS:X vagy Auro-3D kódolás vagy újrakódolás. Nincs rá gyártófüggetlen kódoló, és az R1 elve szerint nem keverünk. Ahol a hang nem vágható veszteségmentesen, ott a sáv kimarad, konkrét okkal, csendes átkódolás nélkül.
- Dolby Vision RPU újragenerálása. Ha DV-réteget észlelünk, a képkocka-pontos vágás elutasítva, a GOP-határ másolás megengedett.
- Blu-ray (M2TS/BDMV) authoring, AACS (#1875).
- Upstream módosítás, GOPchop-kód, libav-linkelés (az R1 döntései érvényben maradnak).

## 2. Szakmai alapszabályok (a mérések alapján)

### 2.1 Hang vágási pontjai

| Kodek | Független vágási egység | Mért keretidő | Belépési pont |
|---|---|---|---|
| AC-3 / E-AC-3 | syncframe (E-AC-3: független alfolyam + függők együtt) | 32 ms | minden keret |
| DTS (core + XLL/X) | core frame a kiterjesztéseivel | 10,7 ms | minden keret, ha az XLL-nek is van ott sync. Az elemző ellenőrzi. |
| TrueHD | access unit (40 minta) | 0,83 ms | **csak a major sync**. Mért távolság: **~106 ms** (128 AU). Atmosnál az OAMD is itt van. |

**A/V szabály.** A hangcsomagok eredeti időbélyege a videóhoz képest változatlan marad, így a szinkron bitpontosan megmarad. Csak a hang kezdete és vége kerekedik a legközelebbi belépési pontra, és mindig a vágáson kívül.

- **MP4:** a többletet az edit list rejti el, ahogy az R1 AAC-útja (`audio_copy_tests`).
- **MKV:** nincs edit list. A hang legfeljebb egy egységgel (TrueHD: ~106 ms) a kép előtt indulhat, és ezt a jelentés kiírja. A vég `DiscardPadding`-gel pontosítható, ha a kodek engedi, egyébként a keret egészben marad.

A TrueHD-nél a major sync előtti, félbevágott szakasz nem kerül ki, mert az Atmos-dekóder az első OAMD-ig nem tud renderelni. Ez illeszkedik a #1883-hoz (TrueHD Atmos: nincs csendes kezdet az első OAMD előtt).

### 2.2 Videó

- MKV-ben a H.264/HEVC hosszmezős. A copy planner `Framing::LengthPrefixed` útja már kezeli (`copy_planner.rs:445`).
- A `CodecPrivate` (avcC/hvcC) változása szakaszt zár, ahogy MP4-ben.
- A GOP-távolság 8,3 s is lehet (DTS-X minta). A képkocka-pontos vágás ott akár 200 képkockányi 4K HEVC határ-újrakódolás. Ennek idejét a referenciagépen mérni kell (WP R2-07), és a `export.plan` előre megmutatja a becsült időt.

### 2.3 Konténerváltás

- **MKV → MKV és MP4 → MP4:** Strict.
- **MKV → MP4 és MP4 → MKV:** Compatible, a SCE-001 allowlist szerint. A `CodecPrivate` és a mintabejegyzés átírása (`dec3`, `ddts`, `dmlp` előállítása a bitfolyamból) naplózott átalakítás.
- **TrueHD MP4-be:** csak Compatible, és a jelentés figyelmeztet, hogy sok lejátszó nem játssza le.

## 3. Munkacsomagok

A sorrend kötelező, a következő csomag az előző beolvasztása után indul.

| WP | Tartalom | Kilépési feltétel |
|---|---|---|
| **R2-00** | Kiindulási audit. A mai `cfa4-cli` lefuttatása az öt mintán, rögzítve, mi történik. A TrueHD major sync, a DTS XLL sync és az E-AC-3 alfolyam-szerkezet mérése. `docs/audit/SMART-CUT-R2-BASELINE-…md`. | jelentés `main`-ben |
| **R2-01** | Hang-hozzáférésipont index a `cfa4-smart-edit`-ben: `AudioIndex` (egységek, időtartam, belépési pont, konfiguráció-azonosító) AC-3-ra, E-AC-3-ra, DTS-re és TrueHD-re. Hostile-input és fuzz cél. | ffmpeg-orákulum a keretszámokra, fuzz zöld |
| **R2-02** | Bitstream-hang másolás MP4-ben (`audio_copy.rs` bővítése), edit list és szinkronellenőrzés (`verify_audio_tracks`). **Az első látható eredmény: a .mp4 minta E-AC-3 Atmos hangja megmarad.** | szintetikus AC-3/E-AC-3/DTS/TrueHD (ffmpeg-kódolók) bitazonos, ffmpeg lejátssza a vágásponttól |
| **R2-03** | Matroska forrás: `open_source` MKV-ág, `MediaIndex` H.264/HEVC/AV1/MPEG-2-re. A registry demux-sorai Experimental-ról Supported-re, orákulumteszttel. | `smart_cut_mkv_tests` index + terv |
| **R2-04** | Matroska kimenet a Smart Cut végrehajtóban (`MkvWriter`, Cues, a közös atomikus író). GOP-határ és képkocka-pontos (H.264/HEVC határ-újrakódolás), hanggal. Újra-demuxos ellenőrzés írás előtt. | MKV → MKV bitazonos payload, ffmpeg framemd5 |
| **R2-05** | Konténerváltás Compatible módban (MKV ↔ MP4), mintabejegyzés- és `CodecPrivate`-átírás, napló. | `cross_container_tests` |
| **R2-06** | Motor és Qt6: `export.analyze/plan/start` MKV-vel, kimeneti konténer választó, hangsáv-jelentés, idővonalas Smart Render MKV-forrással. i18n a közös katalógusokba. | Qt offscreen teszt + képernyőkép |
| **R2-07** | Validáció: szintetikus CI-mátrix. Fizikai próba az öt Altitude16 mintán: a másolt hangszakasz dekódolt PCM-je bitazonos a forráséval (`cfa4-truehd`, `cfa4-dts`, `cfa4-ac3`), és a 4K HEVC határ-újrakódolás ideje mérve a referenciagépen. A csomagpróba (WP-10 workflow) egy MKV-esettel bővül. Opcionális kézi lépés: a vágott fájlok bitstream-továbbítása eARC-n erősítőre (pl. LG G5 + `dakenf/lg-g5-dts`, vagy Linuxon `mpv --audio-spdif=dts-hd,truehd,eac3`), hallható megszakítás nélkül. | `docs/audit/SMART-CUT-R2-VALIDATION-…md`, regresszió nincs |

Becslés: 8 PR (R2-00…R2-07), mindegyik a saját „Gazda: …” issue-jával. A saját PR-okat a szabály szerint mi olvasztjuk be.

## 4. Tesztelés és bizonyíték

- **CI:** csak szintetikus minták, `target/fixtures/`, médiafájl nincs commitolva. Az ffmpeg kódolói: `ac3`, `eac3`, `dca` (experimental), `truehd` (experimental), `mlp`. Az Atmos, a DTS:X és az Auro-3D nem szintetizálható. Ezekre a bitfolyam átlátszóságát a meglévő szintetikus kiterjesztés-blokkok és a fizikai próba igazolja.
- **Fizikai próba:** a projekt öt mintáján, a `docs/development/deferred-physical-tests.md` szerint felvett lépésként. A minták nem kerülnek a repóba.
- **FFmpeg 9:** csak orákulum (framemd5, keretszám, lejátszhatóság).

## 5. Döntések Istvántól

*Döntve 2026-11-27:* 1. jóváhagyva; 2. előbb a hang MP4-ben (R2-01–R2-02), utána a Matroska; 3. és 4. a javaslat szerint (István, 2026-11-27, 22:08).

1. **Indítás.** Jóváhagyod-e az R2 programot ebben a formában?
2. **Sorrend.** Javaslat: először a hang MP4-ben (R2-01–R2-02), mert ez a leggyorsabb kézzelfogható eredmény (a .mp4 minta Atmos hangja), utána a Matroska (R2-03–R2-04). Alternatíva: előbb a Matroska.
3. **MKV hangkezdet.** Javaslat: a hang legfeljebb egy belépési egységgel (TrueHD ~106 ms) a kép előtt indulhat, pontos szinkronnal. Alternatíva: a hang a kép utáni első belépési pontnál indul, ilyenkor az eleje néma marad.
4. **Kimeneti konténer alapértéke.** Javaslat: ugyanaz, mint a forrásé, és a váltás Compatible módot igényel.

## 6. Kockázatok

- A TrueHD és a DTS:X belső szerkezete nincs nyilvánosan specifikálva. Csak keret- és szinkronszintű elemzést végzünk, a payloadot nem módosítjuk. A licencelt portok (`cfa4-truehd` Apache-2.0, `cfa4-dts` LGPL-2.1+) csak olvasásra kellenek.
- 4K HEVC határ-újrakódolás hosszú GOP-nál lassú lehet. Ezt mérjük, és az előnézet előre jelzi.
- Az MKV H.264/HEVC demux ma Experimental. Az R2-03 orákulumtesztje nélkül nem kapcsoljuk Supported-re.
