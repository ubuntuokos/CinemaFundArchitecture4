# CFA4-LOSSLESS-MEDIA-TOOLS-001 – F4 stabilizálás: tesztjelentés

**Dátum:** 2026-11-13 · **Terv:** CFA4-LOSSLESS-MEDIA-TOOLS-001 1.1, 10. pont (kötelező tesztelési
területek) és 11. pont (elfogadási kritériumok) · **Ág:** `claude/project-thread-pzk4nu` (PR #1124)

Ez a jelentés a terv 10. pontjának minden kötelező területéhez megnevezi az automatikus teszteket és
az eredményüket. Az F4 során négy új teszt készült ott, ahol az F1–F3 még nem fedett le egy területet
(`crates/engine/src/lossless_f4_tests.rs`). A tesztek fixture-jei futás közben készülnek (memóriában
vagy a `target/fixtures/` alatt), médiát nem commitolunk; az FFmpeg csak orákulum
(`cfa4_testkit::ffmpeg_or_skip`).

## 1. Eredmény röviden

| Terület | Eredmény |
|---|---|
| Veszteségmentes vágás | PASS (F4-ben kiegészítve: összefűzött kimenet, FFmpeg-orákulummal) |
| Kulcsképkockák | PASS |
| Újrakódolás | PASS |
| Hang | PASS |
| Szegmensek | PASS (F4-ben kiegészítve: mentés és visszatöltés) |
| Elemzések | PASS |
| CSV/EDL | PASS |
| Hibatűrés | PASS (F4-ben kiegészítve: sérült forrás, teli lemez) |
| Adatvédelem | PASS (F4-ben kiegészítve: az eredeti fájl bájtra azonos; létező célfájl csak megerősítéssel) |
| Teljesítmény | **Nem mérhető itt** – a referenciagépen kell mérni (5. pont) |

## 2. Területek és tesztek

### 2.1 Veszteségmentes vágás – payload-egyezés, időbélyegek, vágási határok

- `crates/engine/src/smart_cut_tests.rs`: `start_copies_verifies_and_writes_the_cut`,
  `repeated_exports_of_one_source_are_identical_and_follow_a_changed_source`,
  `hevc_sources_are_analyzed_copied_and_re_encoded_at_the_boundaries`,
  `av1_sources_are_analyzed_and_copied_but_not_re_encoded`,
  `a_project_media_item_is_cut_on_its_marked_range`.
- `crates/engine/src/segments_tests.rs`: `segments_are_cut_one_file_each_in_the_chosen_order`
  (több tartomány, egy fájl tartományonként).
- Összefűzött kimenet (K1, „egyenkénti és összefűzött kimenettel”): `export.start` /
  `segments.export` `"join": true` paraméterrel; a tartományok a memóriában vágódnak és
  ellenőrződnek, a `cfa4_isobmff::concat` újrakódolás nélkül fűzi őket egy fájlba. Sávonként
  **egyetlen** edit-bejegyzés van (a CFA4 saját olvasója és sok lejátszó csak az elsőt alkalmazza): a
  képeknek pontosan folytatniuk kell az előző részt (különben a kérés elutasítva, „külön fájlokba”), a
  hang minden részt a rész elejéhez legközelebbi mintával kezd és a rész végén zár (PCM képkockára
  pontosan), így a priming és a farok kimarad, a szinkronhiba legfeljebb egy kép
  (`maxSyncError100ns`). Végül visszaolvassa a fájlt, és minden kiírt mintát összevet a részekkel. Tesztek:
  `joined_ranges_are_one_verified_file_with_every_sample`,
  `a_join_of_re_encoded_ranges_needs_the_confirmation_and_stays_verified`,
  `bad_join_requests_are_refused_before_writing`; FFmpeg-orákulum (`crates/isobmff/tests/oracle_mux.rs`):
  `joined_parts_ffprobe` (H.264 + AAC primingdal, valamint MOV + PCM: a dekódolt képek képkockára
  azonosak a részek egymás utáni dekódolásával, a hossz a részek összege, a CFA4 saját olvasója minden
  rész első képét a rész kezdetén mutatja; az FFmpeg saját edit listás vágása, amely nem folytatja az
  előző részt, elutasítva), `parts_of_other_streams_are_refused`.
- FFmpeg-orákulum (`crates/export`): `ffmpeg_decodes_the_copy_to_the_source_pictures`,
  `ffmpeg_decodes_hevc_cuts_to_the_source_pictures`, `ffmpeg_decodes_av1_cuts_to_the_source_pictures`.
- WP-09 kiegészítők: `one_stream_in_other_timescales_is_cut_to_the_same_samples`,
  `clips_in_other_timescales_at_one_frame_rate_are_copied` (eltérő timescale),
  `a_variable_frame_rate_source_keeps_its_durations`, `ffmpeg_decodes_the_timescale_and_vfr_copies`
  (VFR), `hdr_metadata_survives_copies_and_re_encoded_boundaries` (HDR), valamint az MPEG-2 esetek
  (`a_gop_cut_of_an_mpeg_source_is_copied_with_its_audio_and_verified`,
  `mpeg_sources_are_analyzed_with_their_gops`).

Minden írás előtt a motor visszaolvassa és ellenőrzi a kimenetet (`verified: true`); a fejezetek
(K14) a vágás után is a helyükön vannak: `the_chapters_inside_the_cut_are_kept_and_moved`,
`replacing_chapters_keeps_the_samples_and_refuses_odd_files`, és az ffprobe is ugyanígy olvassa
őket (`replaced_chapters_ffprobe`: kezdőidő és cím egyezik, minden csomag változatlan).

### 2.2 Kulcsképkockák – kért és tényleges határ, csendes igazítás nélkül

- `analyze_reports_the_access_points_and_the_registry_decision`,
  `plan_names_the_safe_points_or_the_boundaries`: a terv a kért és a tényleges határt is megadja, és
  ha a kettő eltér, megnevezi a biztonságos pontokat.
- `start_refuses_before_writing_anything`: ha a vágás nem esik kulcsképkockára és nincs megerősítés,
  semmi nem íródik ki, a hibaüzenet megnevezi az okot.

### 2.3 Újrakódolás – csak megerősítéssel, csak a határokon

- `start_re_encodes_the_boundaries_of_a_confirmed_frame_accurate_cut` (H.264),
  `hevc_sources_are_analyzed_copied_and_re_encoded_at_the_boundaries` (HEVC),
  `a_frame_accurate_mpeg_cut_re_encodes_its_boundaries_once_confirmed`,
  `a_frame_accurate_mpeg2_cut_re_encodes_only_the_boundaries` (MPEG-2).
- `av1_sources_are_analyzed_and_copied_but_not_re_encoded`: ahol nincs engedélyezett kódoló, ott a
  rendszer elutasít, nem kódol csendben.
- `mpeg_cuts_the_executor_cannot_run_are_refused_before_writing`.

### 2.4 Hang – szinkron, csatornák, eldobott sávok jelzése

- `the_audio_tracks_are_copied_with_the_cut_and_in_sync`: a hangsávok a vágással együtt, szinkronban
  másolódnak; az eredmény `audio.verified` mezője sávonként mutatja a mintaszámot és a szinkronhibát.
- `ffmpeg_plays_the_cut_audio_from_the_cut_point` (FFmpeg-orákulum).
- `audio_shorter_or_later_than_the_cut_is_trimmed_or_dropped`: a vágáson kívül eső sáv eldobását az
  eredmény `audio.dropped` listája okkal együtt jelzi.
- Nem AAC/PCM hang (WP-09): `program_and_transport_streams_carry_ac3`, `program_stream_carries_mpeg2_video_and_lpcm`.

### 2.5 Szegmensek – mentés, visszatöltés, átadás az idővonalra

- **Új (F4):** `segments_are_saved_with_the_project_and_come_back` – a szegmensek `file.saveAs` után
  `file.open`-nel változatlanul visszajönnek, és a visszatöltött projektből a teszt ténylegesen ki
  is exportálja őket (`segments.export`, mindkét fájl kiíródik és ellenőrzött).
- `listed_segments_are_removed_renamed_and_labelled_in_undo_steps` (`segments.remove`,
  `markers.edit` a forráson); Qt: a Jelölők panel „A forrásklip szegmensei” nézete
  (`test_markers_panel_shows_and_edits_the_source_segments`).
- `range_markers_are_listed_in_time_order_and_point_markers_are_not_segments`,
  `segments_become_named_subclips_in_one_undo_step`,
  `segments_are_placed_back_to_back_on_the_timeline` (átadás az idővonalra, egy visszavonási lépés),
  `bad_segment_requests_are_refused`, `hostile_marker_times_never_panic`.

### 2.6 Elemzések – jelenet-, fekete- és csendtalálatok, küszöbök

`crates/engine/src/segment_detect_tests.rs`, generált ProRes-mozival (24 kép napnyugta hanggal,
24 kép fekete csenddel, 24 kép erdő hanggal):

- `scenes_black_and_silence_are_proposed_without_changing_the_project`: a jelenetváltás, a fekete és
  a csend képpontosan a várt helyen, a projekt változatlan.
- `a_shorter_minimum_or_a_narrower_span_changes_the_proposal`: a küszöbök és a minimális hossz
  hatása.
- `an_accepted_proposal_becomes_segments_in_one_undo_step`: kézi javítás után elfogadás, egy
  visszavonási lépés.
- `bad_requests_are_refused`, `hostile_values_never_panic`,
  `a_running_analysis_reports_its_state_and_can_be_stopped`.
- `detects_the_cuts_of_an_ffmpeg_concat_and_cuts_linked_audio` (meglévő jelenetfelismerés, orákulum).

### 2.7 CSV/EDL – import-export és időalap

`crates/engine/src/marker_import_tests.rs` és `crates/engine/src/marker_export.rs`:

- `segments_go_out_as_csv_and_come_back_unchanged`, `segments_go_out_as_an_edl_and_come_back_unchanged`,
  `sequence_markers_round_trip_through_csv`.
- Időalap: `a_csv_with_timecodes_only_is_read_at_the_targets_rate`,
  `a_different_frame_rate_is_refused_unless_allowed`,
  `an_edl_of_another_rate_or_outside_the_clip_is_refused`,
  `drop_frame_and_sequence_start_timecode_are_used_for_both_bounds`.
- `bad_files_and_hostile_values_are_refused_without_panicking`,
  `spreadsheet_formulas_are_escaped_in_all_user_text_columns`.

### 2.8 Hibatűrés – sérült fájlok, megszakítás, teli lemez

- **Új (F4):** `a_damaged_source_is_refused_and_never_panics` – félbevágott, üres, bitenként sérült
  és hiányzó forrás: az `export.analyze`, az `export.plan` és az `export.start` sem omlik össze;
  elutasításkor a teszt azt is ellenőrzi, hogy az `o.mp4` nem jött létre, sikeres vágásnál pedig
  hogy az eredmény ellenőrzött.
- **Új (F4):** `a_full_disk_in_the_middle_of_a_batch_names_what_was_written` – ha a második
  tartománynál megtelik a lemez, a hiba megnevezi a tartományt, az okot és a már kiírt fájlt.
- Megszakítás: `a_cancelled_copy_leaves_nothing_behind`, `a_dropped_output_leaves_nothing_and_keeps_the_old_file`,
  `queue_cancels_a_running_export_and_retries_a_failed_one`,
  `a_running_analysis_reports_its_state_and_can_be_stopped`.

### 2.9 Adatvédelem – az eredeti média érintetlen

- **Új (F4):** `the_original_media_is_never_changed` – egytartományos, többtartományos és megerősített
  képpontos vágás után a forrás bájtra azonos; a forrásfájl nem lehet a kimenet.
- A többi tesztben is: `a_full_disk_in_the_middle_of_a_batch_names_what_was_written` és a
  szegmenselemzés (`…without_changing_the_project`).
- Létező célfájl csak megerősítéssel íródik felül (7. pont): a köteg terve `existing` listában
  jelzi a már létező kimeneteket, és az `export.start` `"overwrite": true` nélkül semmit nem ír
  (`existing_output_files_are_replaced_only_when_asked`); a Qt-ablak rákérdez.
- A CFA4 hálózatot nem használ ezekhez a műveletekhez, adatot nem küld ki.

## 3. Grafikus felület (Qt6)

`python/tests/test_qt_gui.py` (a koordinációs szabály szerint fut, de még nem blokkol): a Smart Cut
panel, a szegmenskereső párbeszédablak, a jelölők CSV/EDL importja és exportja, valamint a
képkockamentés minősége és mérete, a szegmensek összefűzése (ffprobe-bal ellenőrizve: 75 kép, a
szegmensnevek fejezetként) és a Jelölők panel szegmensnézete. Eredmény: zöld. A projekt 52 új
felületi szövege a magyar mellett mind a 31 további nyelvi katalógusba bekerült
(`test_qt_translations.py`: zöld).

## 4. Minőségkapuk

`cargo xtask fmt-check`, `cargo clippy --workspace --exclude cfa4-ui --all-targets --release -- -D warnings`,
`cargo xtask layers`, `cargo xtask assets`, `cargo test --release` (engine, export, isobmff,
interchange), `ruff check python` és `pytest -q python/tests --ignore=python/tests/test_qt_gui.py`:
zöld. A Qt GUI-tesztekből (`test_qt_gui.py`) **csak a projekt saját tesztjei futottak** (Smart Cut,
szegmensek, Jelölők panel, Szegmenskereső, jelölő-CSV: zöld); a teljes fájl nem futott le a felhős
környezet időkorlátján belül, és a koordinációs szabály szerint (§3.4) nem blokkoló. A pontos számok
a PR #1124 leírásában vannak.

## 5. Teljesítmény – a referenciagépen mérendő

A 10. pont szerint a CPU-, RAM-, GPU- és I/O-terhelést a referenciagépen kell mérni
([`docs/development/reference-machine.md`](../development/reference-machine.md)). Ez a felhős
fejlesztőkörnyezetben nem végezhető el hitelesen, ezért itt **nincs** mérési eredmény. Az eljárás:

1. Fixture: 10 perces 4K H.264 és HEVC forrás a `target/fixtures/` alá (`ffmpeg`, 2 s GOP, AAC hang).
2. Mérendő parancsok `cfa4-cli`-vel: `export.analyze`, `export.start` GOP-határon (egy és tíz
   tartomány), megerősített képpontos vágás, `segments.detectScenes|detectBlack|detectSilence`.
3. Mérés: `/usr/bin/time -v` (CPU-idő, maximális RSS), `iostat -x 1` (I/O), `nvidia-smi dmon` (GPU,
   csak a határ-újrakódolásnál várható terhelés).
4. Elvárás: a GOP-határos vágás I/O-korlátos (közel a lemez olvasási sebességéhez), a RAM nem nő a
   fájlmérettel, a GPU a tiszta másolásnál tétlen.

Az eredményt a referenciagépen futtatva ehhez a jelentéshez kell hozzáfűzni.

## 6. Licencellenőrzés (11. pont, 9. kritérium)

- **Új külső függőség nincs:** a `Cargo.lock` a `main`-hez képest változatlan. Az egyetlen
  függőségváltozás az `image` crate `webp` funkciójának bekapcsolása (`crates/export/Cargo.toml`,
  K7); az általa használt `image-webp` 0.2.4 (MIT OR Apache-2.0) már korábban is a
  `Cargo.lock`-ban volt. A `cargo deny check licenses` a CI-ban fut (`ci-linux.yml`
  és `cfa4-gate.yml`).
- **FFmpeg:** az új képességek nem használják; csak a tesztekben orákulum (`ffmpeg_or_skip`). Új
  FFmpeg-opció nincs.
- **Clean-room (AGENTS.md §2):** a LosslessCut (GPL-2.0-only) kódja, felületi szövegei, ikonjai
  és gyorsbillentyű-táblái nem kerültek át; a felületi szövegek saját megfogalmazások, a magyar
  katalógusban (`i18n/hu.tsv`). A LosslessCut neve a termékben nem jelenik meg.
- **Nyitott:** a terv 8. pontja szerint a végleges licencmegfelelőséget jogi ellenőrzés
  támasztja alá; ez a tulajdonos döntése, fejlesztői feladat nincs benne.
