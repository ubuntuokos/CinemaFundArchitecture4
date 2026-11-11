# CFA4-SMART-CUT-RENDER-R1 – WP-09 validációs jelentés (2026-11-10)

A [terv](../plans/CFA4-SMART-CUT-RENDER-R1.md) 13. fejezetének tesztmátrixa és 24 kötelező
szélső esete, összevetve a `main` ág (WP-00 … WP-08, #215 … #851) automatizált tesztjeivel.
Az állapot a jelentés napjára vonatkozik.

**Jelölések:** ✅ teljesül, automatizált teszttel · 🟡 részben (a megjegyzés mondja meg, mi hiányzik)
· ⛔ nincs megvalósítva (blokkoló a 19. fejezet Definition of Done-ja szerint) · ⚪ capability-gated:
a registry szabályosan elutasítja, és ezt teszt ellenőrzi.

**Orákulum:** minden „ffmpeg” jelű teszt az FFmpeg-et csak ellenőrzésre használja
(`cfa4_testkit::ffmpeg_or_skip`, framemd5 / ffprobe); a CI „Rust workspace tests with FFmpeg
oracles” jobja futtatja őket. A végrehajtás mindenhol saját Rust kód (tulajdonosi döntés, 2026-10-27).

## 1. Codec-vizsgálatok

| Vizsgálat | MPEG-2 | H.264 | HEVC | AV1 |
|---|---|---|---|---|
| Indexelés | ✅ `index_tests::mpeg2_*`, `index_oracle::mpeg2_{open,closed}_gop_reference` | ✅ `index_tests::h264_*`, `index_oracle::h264_*` | ✅ `hevc_index_tests`, `index_oracle::hevc_*` | ✅ `av1_index_tests`, `index_oracle::av1_{libaom,svt}_reference` |
| Biztonságos stream copy | ⛔ nincs MPEG-2 packet copy (PS/TS muxer és MPEG-2 kódoló már van) | ✅ `export::smart_cut_tests` (payload-egyezés, ffmpeg) | ✅ `smart_cut_hevc_tests` | ✅ `smart_cut_av1_tests` |
| Nyitott GOP | 🟡 indexelés ✅ (`mpeg2_open_gops`, `mpeg2_closed_gop_and_broken_link`); vágás nincs | ✅ `an_open_gop_is_no_copy_start`, `h264_recovery_point_*` | ✅ `cra_pictures_with_rasl_leading_pictures_are_not_clean` | ✅ `hidden_key_frames_and_missing_sequence_headers_are_not_cut_points` |
| Pontos vágás | ⛔ | ✅ `boundary_reencode_tests`, `copy_planner_tests::a_frame_accurate_*` | ✅ `boundary_reencode_hevc_tests` | ⚪ GOP-határon pontos; képkocka-pontos vágáshoz nincs AV1 kódoló (`av1_sources_are_analyzed_and_copied_but_not_re_encoded`) |
| Boundary re-encode | ⛔ (a terv szerint kötelező) | ✅ `ffmpeg_decodes_the_spliced_stream` | ✅ `ffmpeg_decodes_spliced_hevc` | ⚪ `baseline_av1_has_no_encoder_and_mpeg2_demuxes_from_iso_and_matroska` |
| Export / reimport | ⛔ | ✅ `a_gop_boundary_cut_copies_the_planned_samples_unchanged` (újraindexelés) | ✅ `hevc_gops_are_copied_unchanged` | ✅ `av1_temporal_units_are_copied_unchanged` |

Idővonal Smart Render (WP-06c): ✅ H.264 MP4/MOV-ba (`timeline_render_tests`, engine
`smart_render_tests::ffmpeg_decodes_the_sequence_export`). HEVC/AV1 klipek az idővonalon teljes
renderelést kapnak (`hevc_clips_on_a_sequence_are_rendered`), ez szabályos fallback.

## 2. Kötelező szélső esetek

| # | Eset | Állapot | Bizonyíték / hiány |
|---|---|---|---|
| 1 | Vágás az első képkockán | ✅ | `copy_planner_tests::first_and_last_picture_cuts` |
| 2 | Vágás az utolsó képkockán | ✅ | ugyanaz |
| 3 | Vágás GOP-határon | ✅ | `a_whole_gop_is_copied_unchanged`, `a_gop_boundary_plan_runs_as_a_pure_copy` |
| 4 | Vágás GOP közepén | ✅ | `a_mid_gop_cut_is_refused_in_gop_boundary_mode_with_the_nearest_safe_points`, `a_frame_accurate_cut_reencodes_head_and_tail` |
| 5 | B-frame függőségek a határon | ✅ | `pictures_crossing_an_access_point_end_a_copy_span`, `smart_render_tests::boundaries_decode_the_pictures_they_reference` |
| 6 | Nyitott GOP | ✅ | lásd 1. fejezet (MPEG-2 csak indexelés) |
| 7 | Több vágási pont | ✅ | `timeline_render_tests::two_sources_a_gap_and_boundaries_make_one_stream` |
| 8 | Több összefűzött klip | ✅ | ugyanaz, `random_timelines_cover_the_output_and_decode_every_reference` |
| 9 | Eltérő időalapok | 🟡 | eltérő időalap → elavult index elutasítva (`an_index_of_another_file_is_refused`), eltérő képfrekvencia → renderelés (`another_codec_or_frame_rate_is_rendered`); nincs teszt két, eltérő timescale-ű, de azonos képfrekvenciájú forrás másolására |
| 10 | VFR bemenet | 🟡 | a tervező a forrás PTS-eit használja; nincs VFR-forrással végzett vágásteszt |
| 11 | Hibás / hiányzó PTS/DTS | ✅ | `missing_or_repeated_timestamps_are_refused`, `timestamp_problems_are_reported`, `h264_recovery_point_without_timestamps_leaves_dependencies_unknown` |
| 12 | AAC encoder delay | ⛔ | a Smart Cut / Smart Render kimenete csak videó; nincs hangkezelés |
| 13 | Több hangsáv | ⛔ | ugyanaz |
| 14 | Eltérő codec extradata | ✅ | `a_configuration_change_ends_a_copy_span`, `a_stream_with_another_configuration_is_re_encoded`, `in_band_parameter_sets_are_refused` |
| 15 | HDR metadata | 🟡 | határ-újrakódolásnál az SEI HDR-metaadat átkerül (`boundary_reencode.rs` `check_sei`), packet copy-nál bitazonos; nincs külön HDR-forrás teszt a Smart Cutra |
| 16 | Kimeneti muxer inkompatibilitása | ✅ | `the_copy_guarantee_decides_whether_a_framing_change_is_allowed`, `start_refuses_before_writing_anything` (.mkv), registry `Mux Mpeg2Video Mp4` Unsupported |
| 17 | Sérült médiafájl | ✅ | `hostile_packets_do_not_panic`, `random_indexes_and_requests_never_panic_and_plans_cover_the_range`, `hostile_report_json_does_not_panic` |
| 18 | Megszakított export | ✅ | a kimenet memóriában készül, ellenőrzés után íródik ki egyben (`engine::smart_cut::start`); hiba esetén nincs részleges fájl (`renderer_failures_stop_the_export`, `start_refuses_before_writing_anything`). Felhasználói megszakítás gomb nincs |
| 19 | Export közben módosított projekt | 🟡 | a motor parancsai sorosan futnak, az export a parancs elején vett állapotból dolgozik; nincs rá külön teszt |
| 20 | Sikertelen validáció utáni fallback | ✅ | `verification_catches_a_changed_payload`, `verification_catches_a_poor_or_damaged_re_encode`, `verification_catches_a_changed_or_poor_output`; ilyenkor nem íródik fájl |
| 21 | Forrásfájl időközbeni módosítása | ✅ | `a_stale_hevc_index_is_refused`, `a_stale_av1_index_is_refused`, **új:** `repeated_exports_of_one_source_are_identical_and_follow_a_changed_source` |
| 22 | Többszöri export azonos bemenetből | ✅ | **új:** ugyanaz (GOP-határ és pontos vágás, bájtazonos kimenet), `plans_are_deterministic_and_round_trip_as_json` |
| 23 | GOP-index cache érvénytelenítése | ✅ | `cached_index_validity`; a motor minden parancsnál újraolvassa a forrást (**új** teszt fent) |
| 24 | HW és SW encoder eltérő paraméterei | 🟡 | `smart_render_picks_the_encoder_that_fits_the_stream`, `a_boundary_reencode_needs_a_known_stream_format_that_the_encoder_fits`, `hardware_encoders_wait_for_a_probe`; a CI-ban nincs hardveres kódoló, így HW-kimenet nincs mérve |

## 3. Elfogadási metrikák

- **Packet Copy:** ✅ payload-egyezés (`verify_mp4_copy`), dekódolhatóság (ffmpeg framemd5), konténer
  (újraindexelés, ffprobe) H.264 / HEVC / AV1 esetén. MPEG-2: ⛔.
- **Smart Render:** ✅ kezdő/záró képkocka, referenciák, változatlan belső szakaszok, csak a tervezett
  tartomány újrakódolva (`verify_mp4_boundary_reencode`, `verify_mp4_timeline`).
- **A/V:** ⛔ nincs hangsáv a kimenetben, így A/V szinkron sincs mérve.
- **Regresszió:** ✅ a teljes `cargo test --workspace`, a Python- és Qt-tesztek a CI-ban zöldek; az
  új funkciók mind automatizált teszttel kerültek be (WP-01 … WP-08).

## 4. Kapufeltétel és következő lépések

A WP-09 kapufeltétele („blokkoló regresszió nélkül teljesülnek az elfogadási feltételek”) **még
nem teljesül**. Regresszió nincs, de a terv kötelező követelményei közül hiányzik:

1. **Hang a Smart Cut / Smart Render kimenetben** (9. fejezet, DoD 5; szélső esetek 12–13):
   AAC és PCM hangcsomagok másolása a videóval együtt, encoder delay / priming kezelése, több
   hangsáv, A/V szinkron mérése. → WP-09b.
2. **MPEG-2 Smart Cut** (tesztmátrix): packet copy MPEG-PS / MPEG-TS-be a meglévő `cfa4-mpegts`
   muxerrel, majd határ-újrakódolás a `cfa4-mpeg2v` kódolóval. → WP-09c.
3. **Pótló tesztek** a 🟡 esetekre (9, 10, 15, 19). → WP-09d.

A WP-10 (csomagolás) ezek után következik.

## 5. Frissítés (2026-11-11): WP-09b, WP-09c, WP-09d

A 4. fejezet hiányai közül a hang, az MPEG-2 csomagmásolás és a pótló tesztek elkészültek
(#853, #854, #909, és a jelen WP-09d). A korábbi sorok helyett az alábbi állapot érvényes.

### Codec-vizsgálatok (MPEG-2 oszlop)

| Vizsgálat | MPEG-2 |
|---|---|
| Biztonságos stream copy | ✅ `export::smart_cut_mpeg_tests` (PS: LPCM + AC-3, TS: AAC + MP2; payload-egyezés, ffmpeg framemd5), motor: `smart_cut_mpeg_tests` |
| Nyitott GOP | ✅ indexelés; másolás csak tiszta hozzáférési ponttól (zárt GOP / I-kép), a GOP-index szerint |
| Pontos vágás | ⛔ a terv képpontos vágásnál `boundaryReencode.ready = false` indoklással; futtatáskor írás előtt elutasítva (`mpeg_cuts_the_executor_cannot_run_are_refused_before_writing`) → WP-09e |
| Boundary re-encode | ⛔ → WP-09e (`cfa4-mpeg2v` kódolóval) |
| Export / reimport | ✅ `verify_mpeg_copy` (újra-demux, képek, hangkeretek, időbélyegek, A/V eltolás), `a_gop_cut_of_an_mpeg_source_is_copied_with_its_audio_and_verified` |

### Szélső esetek

| # | Eset | Állapot | Bizonyíték |
|---|---|---|---|
| 9 | Eltérő időalapok | ✅ | `smart_cut_edge_tests::one_stream_in_other_timescales_is_cut_to_the_same_samples` (25, 90 000, 12 800 Hz); idővonalon: `smart_render_tests::clips_in_other_timescales_at_one_frame_rate_are_copied`. **A teszt hibát talált:** a terv másolást ígért, a végrehajtó viszont elutasította az eltérő timescale-ű másolt szakaszokat. Javítva: a kompozíciós eltolások pontosan átszámolódnak a kimeneti timescale-re, kerekítés nélkül (ha nem egész, elutasítás; `copied_timestamps_are_rescaled_only_when_exact`) |
| 10 | VFR bemenet | ✅ | `a_variable_frame_rate_source_keeps_its_durations`: GOP-másolás képenkénti időtartammal; képpontos vágás csak egyforma időtartamú határképekkel, különben írás előtt elutasítva; ffmpeg: `ffmpeg_decodes_the_timescale_and_vfr_copies` |
| 12 | AAC encoder delay | ✅ | `audio_copy_tests::aac_and_pcm_tracks_are_copied_unchanged_and_in_sync` (pre-roll csomag + edit list), `ffmpeg_plays_the_cut_audio_from_the_cut_point`; idővonal: `the_sequence_audio_is_mixed_encoded_and_presented_with_the_video` (priming az edit listben) |
| 13 | Több hangsáv | ✅ | ugyanaz (AAC + PCM sáv egyszerre), `the_audio_tracks_are_copied_with_the_cut_and_in_sync`; MPEG: két hangfolyam PS-ben és TS-ben |
| 15 | HDR metadata | ✅ | `hdr_metadata_survives_copies_and_re_encoded_boundaries`: HEVC Main 10, BT.2020/PQ, ST 2086 + CLL; másolásnál `colr`/`mdcv`/`clli` és SEI bájtra azonos, a határ-újrakódolt IDR ugyanazt az SEI-t hordozza; ffprobe: `ffprobe_reads_the_hdr_copy_as_bt2020_pq` |
| 19 | Export közben módosított projekt | ✅ | `an_edit_after_the_plan_is_what_the_export_writes`: a korábbi terv nem használódik újra, a szerkesztés utáni állapot íródik ki, a megerősítést újra kéri |
| 24 | HW és SW encoder | 🟡 | változatlan: a CI-ban nincs hardveres kódoló |

### Elfogadási metrikák

- **Packet Copy:** ✅ MPEG-2 PS / TS is (`verify_mpeg_copy`, ffmpeg framemd5).
- **A/V:** ✅ az A/V eltérés mérve és korlátozva (`verify_audio_tracks`: legfeljebb fél videóképkocka; MPEG: az első hangkeret eltolása az első képhez képest bitpontosan megmarad).

### Kapufeltétel

Regresszió továbbra sincs. A terv kötelező elemei közül egy hiányzik még: az **MPEG-2
határ-újrakódolás** (képpontos vágás) → WP-09e. A 24. eset hardverfüggő, CI-ban nem mérhető;
ez nem blokkoló, a szoftveres út teljesen tesztelt. WP-09e után következik a WP-10 (csomagolás).

## 6. Frissítés (2026-11-11): WP-09e és WP-10

Az MPEG-2 határ-újrakódolás elkészült: #911 (végrehajtó és ellenőrző) és a jelen PR (motor). A
csomagolás-validáció (WP-10) is kész: #912,
[`SMART-CUT-RENDER-WP10-PACKAGING-2026-11-11.md`](SMART-CUT-RENDER-WP10-PACKAGING-2026-11-11.md).

### Codec-vizsgálatok (MPEG-2 oszlop)

| Vizsgálat | MPEG-2 |
|---|---|
| Pontos vágás | ✅ képkocka-pontos vágás PS-ben és TS-ben. `export.plan`: `boundaryReencode.ready = true`. `export.start` csak `"confirm": true`-val fut (`a_frame_accurate_mpeg_cut_re_encodes_its_boundaries_once_confirmed`) |
| Boundary re-encode | ✅ `boundary_reencode_mpeg_tests` |

A `boundary_reencode_mpeg_tests` esetei:

- fej/másolás/fej szakaszok;
- vágás egy GOP-on belül;
- nyitott GOP-ok;
- elutasítás írás előtt: VFR a határon, ellentmondó képfrekvencia, MPEG-1, interlaced, 4:4:4;
- az ellenőrző elkapja a megváltozott képet és a gyenge újrakódolást;
- FFmpeg framemd5: a másolt képek azonosak.

Az FFmpeg-összevetés hibát talált: az újrakódolt szakasz `low_delay` jelzője eltért a forrásétól, ezért a dekóder a vágásnál egy képet megismételt, egy másikat eldobott. Javítva: `cfa4-mpeg2v` `Config::low_delay`, az újrakódolt szakasz a forrás jelzőjét kapja.

### Kapufeltétel

A WP-09 kapufeltétele teljesül:

- a terv kötelező MPEG-2 / H.264 elemei mind tesztelve vannak;
- regresszió nincs;
- a 24. eset (hardveres kódolók) CI-ban nem mérhető, és nem blokkoló.

A WP-10 csomagpróbája egyelőre GOP-határon vág. A képkocka-pontos lépés a jelen PR beolvasztása után kerül bele.
