# Upstream-szinkron nyilvántartás

A CFA4-UPSTREAM-SYNC-001 terv (`docs/plans/CFA4-UPSTREAM-SYNC-001.md`) nyilvántartása. Az upstream a
`NOTICE`-ban megnevezett szülőprojekt; a PR-számok az upstream tárolóra vonatkoznak. Az upstream
márkanevét a `test_independence.py` miatt `[upstream]` helyettesíti.

| Mező | Érték |
| --- | --- |
| Leválási pont | upstream `v0.10.0` = `cdf43fd`, 2026-10-20 |
| Utolsó átvett upstream címke | `v0.10.0` (az U0 végén `v0.15.0` lesz) |
| Utolsó osztályozott upstream címke | `v0.15.0` (U0-0, 2026-11-01) |
| CFA4 `main`, amelyhez az alkalmazhatóságot mértük | `b93bf62` (2026-11-01) |

## Hogyan készült (U0-0, 2026-11-01)

- **PR-lista:** a `v0.10.0..v0.15.0` git-történetből, a „tesztelt köteg” merge-ök (#3240, #3288)
  belső tételeivel együtt. 254 upstream PR.
- **Kategória:** a terv 4. fejezete szerint, szabályalapú első kör után kézzel javítva. „C+D” = a motorrész
  C, a felületi rész D (a Qt6 GUI-ba).
- **Alkalmazhatóság:** minden PR változását a CFA4 neveire írtuk át, és `git apply --check`-kel
  mértük a CFA4 `main`-en: „tisztán” = változtatás nélkül alkalmazható, „kézi port” = adaptálni kell,
  „már megvan” = visszafelé alkalmazható, vagyis a CFA4-ben már benne van. A tisztán alkalmazható tétel
  is portolandó: a végrehajtó PR a CFA4 szabályai szerint ellenőrzi (teszt, `cargo xtask ci`).
- **„Már megvan” ellenőrzés tartalmi szinten:** a kézi port tételeknél a végrehajtó köteg PR-ja
  nézi meg, hogy a CFA4 a saját fejlesztésével nem oldotta-e már meg ugyanazt; ha igen, a sor
  „már megvan” lesz.
- **U0-kötegek:** U0-A1 videodekóderek, U0-A2 konténerek és bitfolyam, U0-A3 feliratok, interchange,
  beszéd, szöveg, vezérlés, U0-A4 a motor többi része (engine, render, export, projekt, GPU stb., a
  0.15 robusztussági tételeivel), U0-B hibajavítás és teljesítmény, U0-C új motorképesség, U0-D
  Qt6-felület, U0-E fordítások.

## Összesítés

| Kategória | PR |
| --- | --- |
| A | 100 |
| B | 62 |
| C | 8 |
| C+D | 18 |
| D | 13 |
| E | 2 |
| F | 49 |
| köteg | 2 |
| **Összesen** | **254** |

| U0-köteg | PR |
| --- | --- |
| U0-A1 | 20 |
| U0-A2 | 17 |
| U0-A3 | 29 |
| U0-A4 | 34 |
| U0-B | 61 |
| U0-C | 8 |
| U0-C, U0-D | 18 |
| U0-D | 13 |
| U0-E | 2 |

Átveendő tételek alkalmazhatósága a CFA4 `main`-re: tisztán alkalmazható 101, kézi port 101, már megvan 1.

## Tételek

Sorrend: kiadás (legújabb elöl), azon belül kategória, majd PR-szám. A „Döntés” és „Állapot” mezők
értékei a terv 6. fejezetében.

| Upstream kiadás | Upstream PR | Cím (upstream) | Kat. | Döntés | Alkalmazhatóság | U0-köteg | CFA4 PR | Állapot |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 0.15.0 | #3338 | #2798: contain Symphonia audio decoder panics | A | átvesz | kézi port | U0-A2 | – | tervezett |
| 0.15.0 | #3344 | Reject non-PCM WAVE_FORMAT_EXTENSIBLE files in parse_wav | A | átvesz | kézi port | U0-A3 | – | tervezett |
| 0.15.0 | #3417 | Refuse non-finite colour components in effects.setParam | A | átvesz | kézi port | U0-A4 | – | tervezett |
| 0.15.0 | #3548 | gpu: create wgpu instances without OpenGL so concurrent device setup cannot deadlock (#3418) | A | átvesz | kézi port | U0-A4 | – | tervezett |
| 0.15.0 | #3594 | File safety: Make Offline with deleteFiles only removes the media file the item was linked to | A | átvesz | kézi port | U0-A4 | – | tervezett |
| 0.15.0 | #3659 | Reject out-of-range SCC timecode fields | A | átvesz | kézi port | U0-A3 | – | tervezett |
| 0.15.0 | #3347 | speech: a pause inside a word's span ends the word (#3346) | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.15.0 | #3405 | engine: Duplicate puts the copy in the bin of its source (#3368) | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.15.0 | #3409 | engine: effects.addKeyframe keeps an explicit time at the out point and says what it did (#3386) | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.15.0 | #3416 | Shorten atomic_write temp names so long project names still save | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.15.0 | #3423 | Export: preserve optional binary transparency in animated GIFs (#3396) | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.15.0 | #3432 | Strip UTF-8 byte order mark when parsing .cube LUT files | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.15.0 | #3440 | Fix trim_caption moving edges of captions shorter than one frame | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.15.0 | #3452 | fix(codecs): frame lookup inside a frame with one timestamp unit per frame | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.15.0 | #3470 | Export: write fast-start MP4 files for upload compatibility (#3459) | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.15.0 | #3471 | frame: move rotated chroma buffers instead of cloning (#3468) | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.15.0 | #3474 | Media cache: a folder of its own inside a custom location (fixes #2772) | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.15.0 | #3565 | Fix graphics template width scaling across canvas sizes (#3415) | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.15.0 | #3590 | Audio Track Mixer: a fader moved during playback in Read mode keeps its level | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.15.0 | #3591 | Synchronize: Clip > Synchronize (Audio) works on long stereo clips | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.15.0 | #3607 | render: convert the CPU monitor frame to sRGB bytes in vectorised batches | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.15.0 | #3627 | fix(engine): accept time objects for timeline.place sourceIn and duration | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.15.0 | #3637 | Free decoder frame caches and plane pools when an export ends | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.15.0 | #3535 | control: say why the control server is not serving (#3436) | C | átvesz | kézi port | U0-C | – | tervezett |
| 0.15.0 | #3631 | Add timeline.delete alias and document headless clip removal | C | átvesz | kézi port | U0-C | – | tervezett |
| 0.15.0 | #3402 | #3364: warn about one-frame In/Out exports | C+D | átvesz (motor + Qt6) | kézi port | U0-C, U0-D | – | tervezett |
| 0.15.0 | #3433 | feat(text): strikethrough for graphic text | C+D | átvesz (motor + Qt6) | kézi port | U0-C, U0-D | – | tervezett |
| 0.15.0 | #3522 | Timeline: grouped clips select together | C+D | átvesz (motor + Qt6) | kézi port | U0-C, U0-D | – | tervezett |
| 0.15.0 | #3587 | Effect Controls: an effect dropped on the panel is applied to the clip it shows | C+D | átvesz (motor + Qt6) | kézi port | U0-C, U0-D | – | tervezett |
| 0.15.0 | #3608 | render: a monitor frame that is one opaque video picture goes straight to sRGB bytes | C+D | átvesz (motor + Qt6) | kézi port | U0-C, U0-D | – | tervezett |
| 0.15.0 | #3327 | Project panel search finds names holding every typed word | D | átvesz (Qt6) | kézi port | U0-D | – | tervezett |
| 0.15.0 | #3523 | Captions: File ▸ Export ▸ Captions… asks for the file format (#3365) | D | átvesz (Qt6) | kézi port | U0-D | – | tervezett |
| 0.15.0 | #3584 | Program Monitor: Snap in Program Monitor applies to video clip Motion drags | D | átvesz (Qt6) | kézi port | U0-D | – | tervezett |
| 0.15.0 | #3588 | Effect Controls: Font and Font Style of a text layer are picked from drop-downs | D | átvesz (Qt6) | kézi port | U0-D | – | tervezett |
| 0.15.0 | #3373 | Declare the UI languages to AppKit so the open and save panels are translated | F | kihagy: nem Linux x86_64 cél vagy más csomagolás | kézi port | – | – | lezárva |
| 0.15.0 | #3422 | web: Keep WebCodecs enabled after media read errors | F | kihagy: nem Linux x86_64 cél vagy más csomagolás | kézi port | – | – | lezárva |
| 0.15.0 | #3527 | Grant network access so update checks can reach GitHub | F | kihagy: önfrissítő (nem kell) | kézi port | – | – | lezárva |
| 0.15.0 | #3534 | Honor CARGO_BUILD_TARGET in FreeBSD packaging for aarch64 release builds | F | kihagy: nem Linux x86_64 cél vagy más csomagolás | kézi port | – | – | lezárva |
| 0.15.0 | #3596 | contributors: add names for marshiyar | F | kihagy: upstream dokumentáció / közösség | kézi port | – | – | lezárva |
| 0.15.0 | #3645 | CI: workflows only call [upstream] at @main | F | kihagy: upstream CI és kiadási folyamat | kézi port | – | – | lezárva |
| 0.15.0 | #3663 | Update prompt shows the app icon and accent color instead of a letter | F | kihagy: önfrissítő (nem kell) | kézi port | – | – | lezárva |
| 0.15.0 | #3668 | UPD.1: honor app pretend versions in update checks | F | kihagy: önfrissítő (nem kell) | kézi port | – | – | lezárva |
| 0.15.0 | #3726 | fix(flatpak): allow reading launcher version pins | F | kihagy: nem Linux x86_64 cél vagy más csomagolás | kézi port | – | – | lezárva |
| 0.14.0 | #1915 | edit: reject repeated clip identities in move plans | A | átvesz | kézi port | U0-A4 | – | tervezett |
| 0.14.0 | #2287 | fix(interchange): OTIO export stops writing nested-sequence copies past a limit | A | átvesz | kézi port | U0-A3 | – | tervezett |
| 0.14.0 | #2394 | Fix TTML/DFXP export overflow for cue times near i64::MAX | A | átvesz | tisztán | U0-A3 | – | tervezett |
| 0.14.0 | #2888 | security(av1): reject non-increasing grain points | A | átvesz | tisztán | U0-A1 | – | tervezett |
| 0.14.0 | #3080 | security(media): bound generator frames, imported audio dimensions, and render images | A | átvesz | kézi port | U0-A4 | – | tervezett |
| 0.14.0 | #3307 | captions: bound MCC export work to scheduled data (#2811) | A | átvesz | kézi port | U0-A3 | – | tervezett |
| 0.14.0 | #3349 | Audio Track Mixer: mixer.meterInputOnly refuses parameters that are not an object | A | átvesz | kézi port | U0-A4 | – | tervezett |
| 0.14.0 | #1103 | fix(engine): Unlink a selection that also holds unlinked clips | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.14.0 | #1845 | fix(engine): clip.audioChannels no longer resets channels on a channels-less call | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.14.0 | #1869 | PNG Sequence writes RGB files when Include Alpha Channel is off | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.14.0 | #1950 | fix(media): time Counting Leader beeps by source frame rate | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.14.0 | #1986 | fix(engine): Close Gap with no gap selected closes the sequence's gaps | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.14.0 | #2008 | fix(engine): Close Gap with no gap selected closes the sequence's gaps (#1986) | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.14.0 | #2011 | fix(engine): Unlink a selection that also holds unlinked clips (#1103) | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.14.0 | #2387 | #2343: saving a project creates its previews folder only when there are previews | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.14.0 | #2427 | Keep '#' inside a quoted .cube TITLE when parsing | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.14.0 | #2483 | fix(engine): clip.audioChannels no longer resets channels on a channels-less call (#1845) | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.14.0 | #2588 | Lay out Motion's auto points in the sequence frame when applied | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.14.0 | #2597 | Centre Transform's auto points in the clip's picture on apply and reset (#2588) | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.14.0 | #2638 | Stop writing an empty ref for FCPXML titles and adjustment layers | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.14.0 | #2653 | Stop writing an empty ref for FCPXML titles and adjustment layers (#2638) | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.14.0 | #3190 | Stop reading on-the-hour times with a spurious "zero" | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.14.0 | #3216 | frame: 10-bit Y'CbCr normalisation through per-code tables | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.14.0 | #3229 | Fit typewriter transition grid and caret to the layer's content | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.14.0 | #3235 | Enable only the Alt-selected side of a linked clip | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.14.0 | #3239 | UI tests: wait up to 120 seconds for background work instead of a fixed number of frames | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.14.0 | #3241 | codecs: the rayon deadlock test allows 120 s between two answered frame requests | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.14.0 | #3244 | Frame workers: a replaced frame server's workers end and release its cache and media pool | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.14.0 | #3252 | text, ui: font coverage tests probe faces without keeping every font file loaded | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.14.0 | #3286 | Keep draft-decoder counter assertions in one test | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.14.0 | #3321 | fix(av1): drop the duplicate strictly_increasing and its test | B | már megvan | már megvan | – | – | lezárva |
| 0.14.0 | #3456 | Speed up H.264 export by a fifth with byte-identical output | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.14.0 | #1841 | #1545: effects.apply answers with the applied instances (clip, index, effect) | C | átvesz | tisztán | U0-C | – | tervezett |
| 0.14.0 | #1870 | #1216: a cap on the CPU threads an export uses (maxThreads) | C | átvesz | kézi port | U0-C | – | tervezett |
| 0.14.0 | #2415 | #2340: uncompressed QuickTime video (raw 24-bit RGB, 2vuy, v210) decodes | C | átvesz | kézi port | U0-C | – | tervezett |
| 0.14.0 | #3281 | Report parameter and accepted value from effects.setParam | C | átvesz | kézi port | U0-C | – | tervezett |
| 0.14.0 | #3209 | Program: drag a marquee or Shift-click to select several mask vertices, and move them together | C+D | átvesz (motor + Qt6) | kézi port | U0-C, U0-D | – | tervezett |
| 0.14.0 | #3211 | Lift and Extract put the removed range on the clipboard | C+D | átvesz (motor + Qt6) | kézi port | U0-C, U0-D | – | tervezett |
| 0.14.0 | #3224 | engine: keep linked edit points together in Trim Mode (#3085) | C+D | átvesz (motor + Qt6) | kézi port | U0-C, U0-D | – | tervezett |
| 0.14.0 | #3301 | Alt-drag of a clip edge trims only that side of a link | C+D | átvesz (motor + Qt6) | kézi port | U0-C, U0-D | – | tervezett |
| 0.14.0 | #2445 | ime: an edit that ends while composing commits the marked text | D | átvesz (Qt6) | kézi port | U0-D | – | tervezett |
| 0.14.0 | #3226 | i18n(fr): translate the 42 strings French still lacked | E | átvesz | kézi port | U0-E | – | tervezett |
| 0.14.0 | #634 | Linux: default to XWayland for file drag-and-drop on Wayland desktops | F | kihagy: winit/egui-specifikus | kézi port | – | – | lezárva |
| 0.14.0 | #2054 | docs: add structured issue intake and security reporting guidance | F | kihagy: upstream dokumentáció / közösség | tisztán | – | – | lezárva |
| 0.14.0 | #2549 | Panels: move onto [upstream]'s shared docking | F | kihagy: közös UI-könyvtár / egui-migráció | kézi port | – | – | lezárva |
| 0.14.0 | #3249 | Migrate [upstream] to egui 0.37 with current editing changes preserved | F | kihagy: közös UI-könyvtár / egui-migráció | kézi port | – | – | lezárva |
| 0.14.0 | #3298 | feat: startup update prompt modal and direct self-updating | F | kihagy: önfrissítő (nem kell) | kézi port | – | – | lezárva |
| 0.14.0 | #3322 | build: pin craft-update to [upstream] main (495e597) | F | kihagy: önfrissítő (nem kell) | kézi port | – | – | lezárva |
| 0.14.0 | #3325 | chore(deps): refresh safe dependency bumps | F | kihagy: függőségek a CFA4 saját ütemében (cargo deny) | kézi port | – | – | lezárva |
| 0.14.0 | #3329 | chore: resolve SonarCloud maintainability issues | F | kihagy: csak SonarCloud-refaktor | kézi port | – | – | lezárva |
| 0.14.0 | #3331 | Captions: the font picker test allows glyphs the interface fonts lack | F | kihagy: upstream betűkészlet-tároló | kézi port | – | – | lezárva |
| 0.14.0 | #3335 | refactor: use method paths instead of closures (S1612) | F | kihagy: nem illik a CFA4 irányához | kézi port | – | – | lezárva |
| 0.14.0 | #3336 | refactor: clear S9045 and S107 new-code issues | F | kihagy: nem illik a CFA4 irányához | kézi port | – | – | lezárva |
| 0.14.0 | #3337 | test: split fuzz sweeps into helpers (S3776) | F | kihagy: csak upstream egui-teszt | kézi port | – | – | lezárva |
| 0.14.0 | #3340 | Text tool: the IME preedit and its active clause show on the video, through [upstream]'s composer | F | kihagy: közös UI-könyvtár / egui-migráció | kézi port | – | – | lezárva |
| 0.14.0 | #3345 | Menus: the hover test draws with the app's fonts, so it passes on macOS | F | kihagy: nem Linux x86_64 cél vagy más csomagolás | kézi port | – | – | lezárva |
| 0.14.0 | #3410 | refactor: shared module layout (craft-contrib#20) | F | kihagy: nem illik a CFA4 irányához | kézi port | – | – | lezárva |
| 0.14.0 | #3412 | ci: require nextest 0.9.150 | F | kihagy: csak upstream egui-teszt | kézi port | – | – | lezárva |
| 0.14.0 | #3419 | Windows installation policy: [upstream] | F | kihagy: nem Linux x86_64 cél vagy más csomagolás | kézi port | – | – | lezárva |
| 0.14.0 | #3426 | Enforce Rust coding standards via Clippy | F | kihagy: a CFA4 saját lint-szabályai | kézi port | – | – | lezárva |
| 0.14.0 | #3445 | refactor(ui): use [upstream]'s ComboBox for every dropdown | F | kihagy: közös UI-könyvtár / egui-migráció | kézi port | – | – | lezárva |
| 0.14.0 | #3288 | Merge 15 open PRs verified on a packaged build (tested batch) | köteg | gyűjtő merge: tételei külön sorban | kézi port | – | – | lezárva |
| 0.13.0 | #2600 | Reject non-hex characters in colour hex parsing | A | átvesz | tisztán | U0-A4 | – | tervezett |
| 0.13.0 | #2630 | Reject signed fields in SCC timecode parser | A | átvesz | tisztán | U0-A3 | – | tervezett |
| 0.13.0 | #2636 | Reject signed fields inside multi-field timecodes | A | átvesz | tisztán | U0-A4 | – | tervezett |
| 0.13.0 | #2709 | #2679: effects.setParam refuses values outside the parameter's range | A | átvesz | tisztán | U0-A4 | – | tervezett |
| 0.13.0 | #2764 | control: Require session-token authentication before dispatching requests | A | átvesz | kézi port | U0-A3 | – | tervezett |
| 0.13.0 | #2866 | fix(speech): bound Whisper model metadata allocations | A | átvesz | tisztán | U0-A3 | – | tervezett |
| 0.13.0 | #2867 | fix(speech): contain and verify downloaded model artifacts | A | átvesz | kézi port | U0-A3 | – | tervezett |
| 0.13.0 | #2868 | fix(control): bound synthetic pointer events | A | átvesz | kézi port | U0-A4 | – | tervezett |
| 0.13.0 | #2869 | security(engine): confine stored media filesystem access | A | átvesz | tisztán | U0-A4 | – | tervezett |
| 0.13.0 | #2870 | security(text): bound text and phonemizer work | A | átvesz | kézi port | U0-A3 | – | tervezett |
| 0.13.0 | #2871 | security(tts): make malformed-token scanning linear | A | átvesz | tisztán | U0-A3 | – | tervezett |
| 0.13.0 | #2876 | security(text): keep font scans within configured folders | A | átvesz | tisztán | U0-A3 | – | tervezett |
| 0.13.0 | #2878 | security(frame): bound decimation sampling work | A | átvesz | tisztán | U0-A4 | – | tervezett |
| 0.13.0 | #2879 | security(captions): bound malformed text processing | A | átvesz | tisztán | U0-A3 | – | tervezett |
| 0.13.0 | #2880 | security(bitstream): cache RBSP stop-bit lookup | A | átvesz | tisztán | U0-A2 | – | tervezett |
| 0.13.0 | #2881 | security(format): preserve permissions on atomic replacement | A | átvesz | kézi port | U0-A4 | – | tervezett |
| 0.13.0 | #2883 | security(codecs): avoid quadratic MPEG timestamp lookup | A | átvesz | tisztán | U0-A2 | – | tervezett |
| 0.13.0 | #2884 | security(hevc): bound specification equation parsing | A | átvesz | tisztán | U0-A1 | – | tervezett |
| 0.13.0 | #2885 | security(project): scan malformed caption markup once | A | átvesz | tisztán | U0-A4 | – | tervezett |
| 0.13.0 | #2886 | security(project): validate graphic templates in linear time | A | átvesz | tisztán | U0-A4 | – | tervezett |
| 0.13.0 | #2887 | security(engine): bound graphic template thumbnails | A | átvesz | tisztán | U0-A4 | – | tervezett |
| 0.13.0 | #2890 | security(codecs): deduplicate hardware parameter-set checks | A | átvesz | tisztán | U0-A2 | – | tervezett |
| 0.13.0 | #2891 | security(speech): bound diarization clustering work | A | átvesz | tisztán | U0-A3 | – | tervezett |
| 0.13.0 | #2892 | security(engine): validate imported sequence settings | A | átvesz | tisztán | U0-A4 | – | tervezett |
| 0.13.0 | #2894 | security(h264): enforce long-term DPB limits | A | átvesz | kézi port | U0-A4 | – | tervezett |
| 0.13.0 | #2896 | security(captions): bound TTML inline span output | A | átvesz | tisztán | U0-A3 | – | tervezett |
| 0.13.0 | #2897 | security(captions): bound TTML speaker labels | A | átvesz | tisztán | U0-A3 | – | tervezett |
| 0.13.0 | #2898 | security(mpegts): avoid repeated audio buffer drains | A | átvesz | tisztán | U0-A2 | – | tervezett |
| 0.13.0 | #2899 | security(codecs): cap GOP cache frame retention | A | átvesz | tisztán | U0-A2 | – | tervezett |
| 0.13.0 | #2900 | security(text): bound transformed glyph masks | A | átvesz | tisztán | U0-A3 | – | tervezett |
| 0.13.0 | #2901 | security(render): bound embedded LUT resolution | A | átvesz | tisztán | U0-A4 | – | tervezett |
| 0.13.0 | #2902 | security(ui): bound export folder elision work | A | átvesz | tisztán | U0-A4 | – | tervezett |
| 0.13.0 | #2903 | security(export): confine metadata-derived names | A | átvesz | tisztán | U0-A4 | – | tervezett |
| 0.13.0 | #2904 | security(mpeg2v): index picture extensions linearly | A | átvesz | tisztán | U0-A1 | – | tervezett |
| 0.13.0 | #2905 | security(opus): validate direct multistream headers | A | átvesz | tisztán | U0-A2 | – | tervezett |
| 0.13.0 | #2906 | security(h264enc): cache second-pass plan sums | A | átvesz | tisztán | U0-A1 | – | tervezett |
| 0.13.0 | #2908 | security(videotoolbox): bound reorder buffering | A | átvesz | kézi port | U0-A4 | – | tervezett |
| 0.13.0 | #2910 | security(interchange): bound imported track indices | A | átvesz | tisztán | U0-A3 | – | tervezett |
| 0.13.0 | #2911 | security(matroska): compose stripped private data once | A | átvesz | tisztán | U0-A2 | – | tervezett |
| 0.13.0 | #2913 | security(exr): validate chunks before frame allocation | A | átvesz | tisztán | U0-A1 | – | tervezett |
| 0.13.0 | #2914 | security(dnx): validate fields before frame allocation | A | átvesz | tisztán | U0-A1 | – | tervezett |
| 0.13.0 | #2915 | security(h264): bound hostile picture allocation | A | átvesz | kézi port | U0-A1 | – | tervezett |
| 0.13.0 | #2916 | security(apv): validate tile payload before allocation | A | átvesz | tisztán | U0-A1 | – | tervezett |
| 0.13.0 | #2917 | security(av1): require unique complete tile coverage | A | átvesz | tisztán | U0-A1 | – | tervezett |
| 0.13.0 | #2918 | security(tts): verify Kokoro checkpoint before parsing | A | átvesz | tisztán | U0-A3 | – | tervezett |
| 0.13.0 | #2919 | security(automation): isolate bridge screenshots | A | átvesz | kézi port | U0-A3 | – | tervezett |
| 0.13.0 | #2920 | codecs: walk open-GOP leading pictures back in a loop, not by recursion | A | átvesz | tisztán | U0-A2 | – | tervezett |
| 0.13.0 | #2922 | security(proxies): reserve outputs atomically | A | átvesz | tisztán | U0-A4 | – | tervezett |
| 0.13.0 | #2925 | security(avi): bound OpenDML index work | A | átvesz | tisztán | U0-A2 | – | tervezett |
| 0.13.0 | #2926 | security(remix): bound analysis resources | A | átvesz | kézi port | U0-A4 | – | tervezett |
| 0.13.0 | #2928 | security(bench): isolate temporary export outputs | A | átvesz | kézi port | U0-A4 | – | tervezett |
| 0.13.0 | #2929 | security(vp8): bound frame allocation | A | átvesz | tisztán | U0-A1 | – | tervezett |
| 0.13.0 | #2930 | security(vp9): bound frame allocation | A | átvesz | tisztán | U0-A1 | – | tervezett |
| 0.13.0 | #2931 | security(prores): bound decoded plane storage | A | átvesz | tisztán | U0-A1 | – | tervezett |
| 0.13.0 | #2932 | security(avi): bound framed audio scans | A | átvesz | kézi port | U0-A2 | – | tervezett |
| 0.13.0 | #2933 | security(isobmff): bound metadata expansion | A | átvesz | tisztán | U0-A2 | – | tervezett |
| 0.13.0 | #2934 | security(aaf): bound materialized streams | A | átvesz | tisztán | U0-A3 | – | tervezett |
| 0.13.0 | #2935 | security(mpeg2v): bound access-unit output | A | átvesz | tisztán | U0-A1 | – | tervezett |
| 0.13.0 | #2938 | security(platform): bound hybrid replay output | A | átvesz | tisztán | U0-A4 | – | tervezett |
| 0.13.0 | #2939 | security(hevc): validate slice payload before allocation | A | átvesz | kézi port | U0-A1 | – | tervezett |
| 0.13.0 | #2941 | security(captions): bound burn-in overlays | A | átvesz | tisztán | U0-A3 | – | tervezett |
| 0.13.0 | #2942 | security(render): bound expanded frame plans | A | átvesz | tisztán | U0-A4 | – | tervezett |
| 0.13.0 | #2943 | security(render): bound burn-in text rasters | A | átvesz | kézi port | U0-A4 | – | tervezett |
| 0.13.0 | #2944 | security(export): bound image overlay decoding | A | átvesz | tisztán | U0-A4 | – | tervezett |
| 0.13.0 | #2946 | security(omf): bound metadata expansion and traversal | A | átvesz | tisztán | U0-A3 | – | tervezett |
| 0.13.0 | #2948 | security(mxf): bound recovery and PCM edit units | A | átvesz | kézi port | U0-A2 | – | tervezett |
| 0.13.0 | #2949 | security(mxf): share resolved descriptor metadata | A | átvesz | kézi port | U0-A2 | – | tervezett |
| 0.13.0 | #2950 | security(ogg): bound recovery scanning | A | átvesz | tisztán | U0-A2 | – | tervezett |
| 0.13.0 | #2952 | security(dng): bound tile decoding and IFD traversal | A | átvesz | tisztán | U0-A1 | – | tervezett |
| 0.13.0 | #2953 | fix(bitstream): reject invalid length-prefixed NAL widths | A | átvesz | tisztán | U0-A2 | – | tervezett |
| 0.13.0 | #2955 | fix(captions): bound TTML XML nesting | A | átvesz | tisztán | U0-A3 | – | tervezett |
| 0.13.0 | #2956 | fix(interchange): confine embedded audio output names | A | átvesz | tisztán | U0-A3 | – | tervezett |
| 0.13.0 | #2957 | fix(media): validate HDR scanlines before frame allocation | A | átvesz | tisztán | U0-A1 | – | tervezett |
| 0.13.0 | #2983 | fix: harden saves, CLI file I/O and AV1 tooling (SonarCloud security/reliability) | A | átvesz | kézi port | U0-A4 | – | tervezett |
| 0.13.0 | #2989 | CTL2807: release idle control connections after bounded socket waits | A | átvesz | kézi port | U0-A3 | – | tervezett |
| 0.13.0 | #3007 | tools: quit the super-linear regex scans in the spec table extractors (SonarCloud S8786) | A | átvesz | tisztán | U0-A1 | – | tervezett |
| 0.13.0 | #3078 | security: harden developer tools, scripts, and testkit against hostile input | A | átvesz | kézi port | U0-A4 | – | tervezett |
| 0.13.0 | #3079 | security(interchange): harden FCPXML cycle detection, XML DTD entities, and imported formats | A | átvesz | kézi port | U0-A3 | – | tervezett |
| 0.13.0 | #3081 | security(codecs): validate VP9 empty tiles and AV1 film grain scaling points | A | átvesz | tisztán | U0-A1 | – | tervezett |
| 0.13.0 | #3082 | security(audio): bound sequence sample rates and AAF/OMF export channel buffers | A | átvesz | tisztán | U0-A3 | – | tervezett |
| 0.13.0 | #3083 | security(engine): require local trust for ingest destinations | A | átvesz | tisztán | U0-A4 | – | tervezett |
| 0.13.0 | #2586 | Escape tab/newline/CR in XML attribute values | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.13.0 | #2601 | Fix false overlaps for empty time ranges | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.13.0 | #2629 | Keep WebVTT cues whose identifier starts with NOTE | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.13.0 | #2662 | Use chrono instead of hand-rolled UTC conversion | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.13.0 | #2861 | Regression tests for #130, #703, #1084 and #1309 | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.13.0 | #2864 | Raise a loaded project's next_id past the ids already in it | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.13.0 | #2991 | [upstream] #293: finish ProRes alpha runs | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.13.0 | #2992 | HEVC2797: share immutable gray rows across unavailable references | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.13.0 | #2993 | REC2792: retain recovery candidates when discard fails | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.13.0 | #2995 | Only materialise kept frames when filling an H.264 frame_num gap | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.13.0 | #3004 | control: preserve MCP pointer input defaults (#2784) | B | átvesz | kézi port | U0-B | – | tervezett |
| 0.13.0 | #3072 | Don't treat pieces with different settings as through edits | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.13.0 | #3088 | Keep WebVTT cues whose identifier is STYLE or REGION | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.13.0 | #3236 | Timeline: a transition stays within its clips after a regular trim | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.13.0 | #2819 | M8: Fujifilm F-Log, F-Log2 and F-Log2 C colour spaces (F-Gamut / F-Gamut C) | C | átvesz | kézi port | U0-C | – | tervezett |
| 0.13.0 | #3038 | media: import animated WebP as a clip | C | átvesz | kézi port | U0-C | – | tervezett |
| 0.13.0 | #3040 | Timeline: Option-drag duplicates captions | C+D | átvesz (motor + Qt6) | kézi port | U0-C, U0-D | – | tervezett |
| 0.13.0 | #3041 | Edit: Copy, Cut and Paste work on captions selected in the Timeline | C+D | átvesz (motor + Qt6) | kézi port | U0-C, U0-D | – | tervezett |
| 0.13.0 | #3075 | History: the panel menu deletes redoable actions and clears the history | C+D | átvesz (motor + Qt6) | kézi port | U0-C, U0-D | – | tervezett |
| 0.13.0 | #3087 | Timeline: the work area bar, with Set Work Area Bar In and Out Point | C+D | átvesz (motor + Qt6) | kézi port | U0-C, U0-D | – | tervezett |
| 0.13.0 | #3115 | effect controls: drag a transition's cut to move it with the transition | C+D | átvesz (motor + Qt6) | kézi port | U0-C, U0-D | – | tervezett |
| 0.13.0 | #3155 | Timeline: a Rolling Edit drag stops at the roll's limits | C+D | átvesz (motor + Qt6) | kézi port | U0-C, U0-D | – | tervezett |
| 0.13.0 | #3212 | Timeline: Show Duplicate Frame Markers marks media frames used more than once | C+D | átvesz (motor + Qt6) | kézi port | U0-C, U0-D | – | tervezett |
| 0.13.0 | #3227 | Audio Track Mixer: Show/Hide Tracks and Meter Input(s) Only are commands with Premiere's keys | C+D | átvesz (motor + Qt6) | kézi port | U0-C, U0-D | – | tervezett |
| 0.13.0 | #3233 | Monitors: Transparency Grid shows the picture's transparent areas | C+D | átvesz (motor + Qt6) | kézi port | U0-C, U0-D | – | tervezett |
| 0.13.0 | #2862 | Scroll the timeline clip menu when it is taller than the screen (#2852) | D | átvesz (Qt6) | kézi port | U0-D | – | tervezett |
| 0.13.0 | #2940 | Project panel: a clip dropped on New Item makes a sequence from it (fixes #2855) | D | átvesz (Qt6) | kézi port | U0-D | – | tervezett |
| 0.13.0 | #3011 | meters: drop to silence right after Stop | D | átvesz (Qt6) | kézi port | U0-D | – | tervezett |
| 0.13.0 | #3039 | Timeline: the Razor tool cuts captions | D | átvesz (Qt6) | kézi port | U0-D | – | tervezett |
| 0.13.0 | #3042 | Text panel: the Captions list follows the playhead | D | átvesz (Qt6) | kézi port | U0-D | – | tervezett |
| 0.13.0 | #3062 | Say why H.265 and AV1 are unavailable without a hardware encoder | D | átvesz (Qt6) | kézi port | U0-D | – | tervezett |
| 0.13.0 | #3243 | Follow-ups to #3240: alpha upload table, duplicate markers off the UI thread, cut drag over the clips | D | átvesz (Qt6) | kézi port | U0-D | – | tervezett |
| 0.13.0 | #3012 | i18n: missing translations never fail; xtask ci prints the missing list | E | átvesz | kézi port | U0-E | – | tervezett |
| 0.13.0 | #2435 | Build: use craft-media for bitstream, isobmff and matroska | F | kihagy: külső könyvtárra átállás (D6: a javításait figyeljük) | kézi port | – | – | lezárva |
| 0.13.0 | #2563 | CI: run cargo xtask ci on pull requests and pushes to main | F | kihagy: upstream CI és kiadási folyamat | tisztán | – | – | lezárva |
| 0.13.0 | #2576 | i18n test: leave installed right-to-left faces out of the Japanese check | F | kihagy: csak upstream egui-teszt | kézi port | – | – | lezárva |
| 0.13.0 | #2609 | fix(engine): resolve stored media paths with [upstream]'s resolve_absolute | F | kihagy: upstream CI és kiadási folyamat | kézi port | – | – | lezárva |
| 0.13.0 | #2610 | Packaging: pin the signing client and appimagetool, and check their hashes | F | kihagy: nem Linux x86_64 cél vagy más csomagolás | kézi port | – | – | lezárva |
| 0.13.0 | #2646 | Fonts: pin [upstream] bfcbf71; [upstream]_REQUIRED fails without [upstream]_DIR | F | kihagy: upstream betűkészlet-tároló | kézi port | – | – | lezárva |
| 0.13.0 | #2860 | CI: xtask ci in the dev profile, one test binary per crate, nextest | F | kihagy: csak upstream egui-teszt | kézi port | – | – | lezárva |
| 0.13.0 | #2937 | CI: run the gate through [upstream]' shared gate.yml; FreeBSD nightly and on release | F | kihagy: nem Linux x86_64 cél vagy más csomagolás | kézi port | – | – | lezárva |
| 0.13.0 | #2971 | ci(deps): bump Swatinem/rust-cache | F | kihagy: függőségek a CFA4 saját ütemében (cargo deny) | kézi port | – | – | lezárva |
| 0.13.0 | #2988 | Import: numbered embedded-audio names keep the path's separators on Windows | F | kihagy: nem Linux x86_64 cél vagy más csomagolás | kézi port | – | – | lezárva |
| 0.13.0 | #3003 | packaging: use [[ for bash conditional tests (SonarCloud S7688) | F | kihagy: csak SonarCloud-refaktor | kézi port | – | – | lezárva |
| 0.13.0 | #3014 | release: the git-sources gate moves into [upstream] (storytold/[upstream]#78) | F | kihagy: upstream CI és kiadási folyamat | kézi port | – | – | lezárva |
| 0.13.0 | #3026 | ci(sonar): don't cancel superseded runs on main, so coverage is published | F | kihagy: csak SonarCloud-refaktor | kézi port | – | – | lezárva |
| 0.13.0 | #3113 | relink tests: no unused index_folder import on Windows | F | kihagy: nem Linux x86_64 cél vagy más csomagolás | kézi port | – | – | lezárva |
| 0.13.0 | #3240 | Merge 6 open PRs (tested batch) | köteg | gyűjtő merge: tételei külön sorban | kézi port | – | – | lezárva |
| 0.12.0 | #2753 | Reject TTML offset times that have no integer digits | A | átvesz | tisztán | U0-A3 | – | tervezett |
| 0.12.0 | #2756 | Speak the minus sign of negative currency amounts in TTS normalizer | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.12.0 | #2757 | Keep file URL hosts that only start with "localhost" | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.12.0 | #2832 | Read the a of a.m. and attached am as the letter name A | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.12.0 | #2572 | dev_icon: write the MimeType line the packaged .desktop file has | F | kihagy: a CFA4 saját .desktop azonosítója | kézi port | – | – | lezárva |
| 0.11.0 | #1795 | Hostile-input hardening for video decoders (AV1 panic + oversize allocation, MPEG-2 allocation) and fuzz tests | A | átvesz | kézi port | U0-A1 | – | tervezett |
| 0.11.0 | #1987 | codecs: avoid duration-sized silence allocation for damaged MP4 audio | A | átvesz | tisztán | U0-A2 | – | tervezett |
| 0.11.0 | #2023 | engine: preserve existing files when AAF/OMF imports embedded audio | A | átvesz | kézi port | U0-A4 | – | tervezett |
| 0.11.0 | #2720 | security: enforce HTTPS on curl downloads (SonarCloud S6506) | A | átvesz | kézi port | U0-A4 | – | tervezett |
| 0.11.0 | #2723 | security: validate CLI paths/args in dev tools (SonarCloud S2083/S8707/S8705) | A | átvesz | kézi port | U0-A1 | – | tervezett |
| 0.11.0 | #1955 | Order auto-saves by real time so rotation keeps the newest | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.11.0 | #2388 | #2342: CLI file.exportMedia waits for the export before the process exits | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.11.0 | #2724 | fix: use async file I/O in async MCP screenshot handler (SonarCloud rust:S7493) | B | átvesz | tisztán | U0-B | – | tervezett |
| 0.11.0 | #2555 | Re-open a failed audio output while playing instead of going silent for good | D | átvesz (Qt6) | kézi port | U0-D | – | tervezett |
| 0.11.0 | #1943 | fix(macos): ask to save on Cmd-Q instead of quitting outright | F | kihagy: nem Linux x86_64 cél vagy más csomagolás | kézi port | – | – | lezárva |
| 0.11.0 | #2006 | fix(macos): ask to save on Cmd-Q instead of quitting outright (#1943) | F | kihagy: nem Linux x86_64 cél vagy más csomagolás | kézi port | – | – | lezárva |
| 0.11.0 | #2157 | ci: switch to shared nix workflow from [upstream] | F | kihagy: nem Linux x86_64 cél vagy más csomagolás | kézi port | – | – | lezárva |
| 0.11.0 | #2368 | docs: add SonarCloud and CodeRabbit badges to README | F | kihagy: csak SonarCloud-refaktor | kézi port | – | – | lezárva |
| 0.11.0 | #2594 | Skip CodeRabbit reviews on Dependabot PRs | F | kihagy: upstream CI és kiadási folyamat | tisztán | – | – | lezárva |
| 0.11.0 | #2763 | README: the app icon above the title | F | kihagy: upstream dokumentáció / közösség | kézi port | – | – | lezárva |
