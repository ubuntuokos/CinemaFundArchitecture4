# Attribution

Every non-code asset in CFA4, with its author, source and licence. Each file listed here also has
a `<file>.attribution` sidecar with full details. The rules are in [AGENTS.md](AGENTS.md) §1: no Adobe
iconography, images or other Adobe assets; open licences only; every asset attributed.
`cargo xtask assets` checks that this index and the sidecars cover every asset in the repository.

## Asset files

| Asset | Author | Source | Licence |
|---|---|---|---|
| `assets/cfa4/cfa4-official-logo.png` | CFA4 project owner (István) | Original CFA3 emblem supplied by the project owner, adapted and approved as the official CFA4 logo on 2026-10-23 | GPL-3.0-or-later |
| `assets/cfa4/hicolor/16x16/apps/io.github.ubuntuokos.cfa4.png` | CFA4 project owner (István) | CFA4 application icon, emblem of the official CFA4 logo | GPL-3.0-or-later |
| `assets/cfa4/hicolor/24x24/apps/io.github.ubuntuokos.cfa4.png` | CFA4 project owner (István) | CFA4 application icon, emblem of the official CFA4 logo | GPL-3.0-or-later |
| `assets/cfa4/hicolor/32x32/apps/io.github.ubuntuokos.cfa4.png` | CFA4 project owner (István) | CFA4 application icon, emblem of the official CFA4 logo | GPL-3.0-or-later |
| `assets/cfa4/hicolor/48x48/apps/io.github.ubuntuokos.cfa4.png` | CFA4 project owner (István) | CFA4 application icon, emblem of the official CFA4 logo | GPL-3.0-or-later |
| `assets/cfa4/hicolor/64x64/apps/io.github.ubuntuokos.cfa4.png` | CFA4 project owner (István) | CFA4 application icon, emblem of the official CFA4 logo | GPL-3.0-or-later |
| `assets/cfa4/hicolor/128x128/apps/io.github.ubuntuokos.cfa4.png` | CFA4 project owner (István) | CFA4 application icon, emblem of the official CFA4 logo | GPL-3.0-or-later |
| `assets/cfa4/hicolor/256x256/apps/io.github.ubuntuokos.cfa4.png` | CFA4 project owner (István) | CFA4 application icon, emblem of the official CFA4 logo | GPL-3.0-or-later |
| `assets/cfa4/hicolor/512x512/apps/io.github.ubuntuokos.cfa4.png` | CFA4 project owner (István) | CFA4 application icon, emblem of the official CFA4 logo | GPL-3.0-or-later |
| `assets/cfa4/hicolor/scalable/apps/io.github.ubuntuokos.cfa4.svg` | CFA4 project owner (István) | CFA4 application icon, emblem of the official CFA4 logo | GPL-3.0-or-later |
| `apps/cfa4-web/web/favicon.svg` | CFA4 project | Copy of the CFA4 development geometric icon (web favicon) | MIT OR Apache-2.0 |
| `crates/ui-egui/src/i18n/it.tsv` | Upstream contributors (see NOTICE) | Original work: clean-room Italian interface translations | MIT OR Apache-2.0 |
| `crates/ui-egui/src/i18n/id.tsv` | Upstream contributors (see NOTICE) | Original work: clean-room Indonesian interface translations | MIT OR Apache-2.0 |
| `crates/ui-egui/src/i18n/ko.tsv` | Upstream contributors (see NOTICE) | Original work: clean-room Korean interface translations | MIT OR Apache-2.0 |
| `crates/ui-egui/src/i18n/fr.tsv` | Upstream contributors (see NOTICE) | Original work: clean-room French interface translations | MIT OR Apache-2.0 |
| `crates/ui-egui/src/i18n/pt-br.tsv` | Upstream contributors (see NOTICE) | Original work: clean-room Brazilian Portuguese interface translations | MIT OR Apache-2.0 |
| `crates/ui-egui/src/i18n/ru.tsv` | Upstream contributors (see NOTICE) | Original work: clean-room Russian interface translations | MIT OR Apache-2.0 |
| `crates/ui-egui/src/i18n/uk.tsv` | Upstream contributors (see NOTICE) | Original work: clean-room Ukrainian interface translations | MIT OR Apache-2.0 |
| `assets/fonts/Inter-Regular.ttf` | The Inter Project Authors | https://github.com/rsms/inter | OFL-1.1 (`assets/fonts/OFL-Inter.txt`) |
| `assets/fonts/Inter-Medium.ttf` | The Inter Project Authors | https://github.com/rsms/inter | OFL-1.1 (`assets/fonts/OFL-Inter.txt`) |
| `assets/fonts/Inter-SemiBold.ttf` | The Inter Project Authors | https://github.com/rsms/inter | OFL-1.1 (`assets/fonts/OFL-Inter.txt`) |
| `assets/fonts/Inter-Bold.ttf` | The Inter Project Authors | https://github.com/rsms/inter | OFL-1.1 (`assets/fonts/OFL-Inter.txt`) |
| `assets/fonts/Inter-Italic.ttf` | The Inter Project Authors | https://github.com/rsms/inter | OFL-1.1 (`assets/fonts/OFL-Inter.txt`) |
| `assets/fonts/NotoSerif-Regular.ttf` | The Noto Project Authors | https://github.com/notofonts/latin-greek-cyrillic | OFL-1.1 (`assets/fonts/OFL-NotoSerif.txt`) |
| `assets/fonts/JetBrainsMono-Regular.ttf` | The JetBrains Mono Project Authors | https://github.com/JetBrains/JetBrainsMono | OFL-1.1 (`assets/fonts/OFL-JetBrainsMono.txt`) |

## Golden test images

Reference frames for the renderer golden tests (`crates/golden`), rendered by the project's own renderer from
procedural demo footage (`crates/media/src/generators.rs`); no external media. Regenerate with
`CFA4_BLESS=1 cargo test -p cfa4-golden`.

| Asset | Author | Source | Licence |
|---|---|---|---|
| `crates/golden/goldens/blend_modes.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `blend_modes`) from procedurally generated media | MIT OR Apache-2.0 |
| `crates/golden/goldens/crop.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `crop`) from procedurally generated media | MIT OR Apache-2.0 |
| `crates/golden/goldens/gaussian_blur.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `gaussian_blur`) from procedurally generated media | MIT OR Apache-2.0 |
| `crates/golden/goldens/graphic_shapes.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `graphic_shapes`) from procedurally generated media and bundled OFL fonts | MIT OR Apache-2.0 |
| `crates/golden/goldens/graphic_template.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `graphic_template`; a built-in CFA4t graphics template) from procedurally generated media and bundled OFL fonts | MIT OR Apache-2.0 |
| `crates/golden/goldens/graphic_rich_text.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `graphic_rich_text`) from procedurally generated media and bundled OFL fonts | MIT OR Apache-2.0 |
| `crates/golden/goldens/log_to_rec709.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `log_to_rec709`) from procedurally generated media re-encoded as S-Log3 | MIT OR Apache-2.0 |
| `crates/golden/goldens/hdr_tone_map.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `hdr_tone_map`) from procedurally generated media re-encoded as Rec. 2100 PQ | MIT OR Apache-2.0 |
| `crates/golden/goldens/graphic_title.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `graphic_title`) from procedurally generated media and bundled OFL fonts | MIT OR Apache-2.0 |
| `crates/golden/goldens/lumetri_basic.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `lumetri_basic`) from procedurally generated media | MIT OR Apache-2.0 |
| `crates/golden/goldens/lumetri_hdr_pq.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `lumetri_hdr_pq`) from procedurally generated media re-encoded as Rec. 2100 PQ | MIT OR Apache-2.0 |
| `crates/golden/goldens/lumetri_presets_grid.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, `lumetri_presets_grid`): the project's own Lumetri presets on its procedural preview picture | MIT OR Apache-2.0 |
| `crates/golden/goldens/text_burnin.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `text_burnin`) from procedurally generated media | MIT OR Apache-2.0 |
| `crates/golden/goldens/transform_opacity.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `transform_opacity`) from procedurally generated media | MIT OR Apache-2.0 |
| `crates/golden/goldens/transition_cross_dissolve.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `transition_cross_dissolve`) from procedurally generated media | MIT OR Apache-2.0 |
| `crates/golden/goldens/transition_dip_to_black.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `transition_dip_to_black`) from procedurally generated media | MIT OR Apache-2.0 |
| `crates/golden/goldens/transition_wipe.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `transition_wipe`) from procedurally generated media | MIT OR Apache-2.0 |
| `crates/golden/goldens/mask_blur.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `mask_blur`) from procedurally generated media | MIT OR Apache-2.0 |
| `crates/golden/goldens/mask_color.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `mask_color`) from procedurally generated media | MIT OR Apache-2.0 |
| `crates/golden/goldens/mask_inverted.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `mask_inverted`) from procedurally generated media | MIT OR Apache-2.0 |
| `crates/golden/goldens/mask_opacity.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `mask_opacity`) from procedurally generated media | MIT OR Apache-2.0 |
| `crates/golden/goldens/vfx_distort.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `vfx_distort`) from procedurally generated media | MIT OR Apache-2.0 |
| `crates/golden/goldens/vfx_ultra_key.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `vfx_ultra_key`) from procedurally generated media | MIT OR Apache-2.0 |
| `crates/golden/goldens/vfx_glows.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `vfx_glows`) from procedurally generated media | MIT OR Apache-2.0 |
| `crates/golden/goldens/vfx_spacer_stroke_shadow.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `vfx_spacer_stroke_shadow`) from procedurally generated media | MIT OR Apache-2.0 |
| `crates/golden/goldens/vfx_vr_rotate.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `vfx_vr_rotate`) from procedurally generated media | MIT OR Apache-2.0 |
| `crates/golden/goldens/vfx_stylize.png` | Upstream contributors (see NOTICE) | Original work: golden reference rendered by the project renderer (crates/golden/tests/golden.rs, scene `vfx_stylize`) from procedurally generated media | MIT OR Apache-2.0 |

## Third-party media shown in screenshots

The README screenshots show frames from these works. The media files themselves are not in the
repository; each screenshot's sidecar lists exactly which appear.

| Work | Author | Status | Source |
|---|---|---|---|
| Night of the Living Dead (1968) | George A. Romero / Image Ten | US public domain (no copyright notice) | https://archive.org/details/night-of-the-living-dead-1968-english |
| Carnival of Souls (1962) | Herk Harvey / Harcourt Productions | US public domain | https://archive.org/details/CarnivalOfSoulsVideoQualityUpgrade |
| Charade (1963) | Stanley Donen / Universal | US public domain (defective notice) | https://archive.org/details/charade-1963-cary-grant-audrey-hepburn-comedy-mystery-romance-thriller-full-movie |
| Earth Views from the ISS (NHQ_2020_1221) | NASA | US Government work, public domain (no NASA endorsement implied) | https://images.nasa.gov/details/NHQ_2020_1221_Earth%20Views |
| Chopin: Nocturne Op. 48 No. 1, Ballade No. 1 | Musopen | CC0-1.0 | https://archive.org/details/musopen-chopin |

## Assets drawn or generated in code

These are original work by the upstream and CFA4 contributors under the project licence (MIT OR Apache-2.0). None
is traced or derived from Adobe artwork.

| Asset | Where |
|---|---|
| UI icons (tools, transport, panels, header) | `crates/ui-egui/src/icons.rs`: vector paths on a 16×16 grid |
| Source and Program range braces and trim hover cues | `crates/ui-egui/src/panels/monitor.rs` and `source_range.rs`: original generic vector shapes by upstream contributors, MIT OR Apache-2.0; no third-party artwork |
| Align / distribute / paragraph-alignment glyphs in the graphics panel | `crates/ui-egui/src/panels/graphics.rs` (`align_glyph`): bars and lines drawn with the egui painter |
| Demo footage (ocean sunset, aurora, city night, dunes, forest, plasma), bars and tone, counting leader, colour matte | `crates/media/src/generators.rs` |
| Built-in graphics templates (Lower Third – Slab / Rule, Ticker – Crawl, Title – Centered / Boxed, End Credits – Roll, Callout – Pointer / Tag) and their placeholder text | `crates/project/src/gtemplate.rs` (`builtin_templates`): original designs defined in code with the bundled Inter font; thumbnails are rendered at runtime, no template, image or font files |
| Lumetri "Look" presets (Teal & Orange, Warm Film, …) | `crates/render/src/effects.rs` (`apply_look`): procedural colour transforms, not LUT files |
| Lumetri Presets (Effects panel: Cinematic, Film Emulation, Monochrome, Technical) and their preview picture | `crates/render/src/lumetri_presets.rs`: Lumetri parameter values chosen by contributors, descriptive names, and a procedural preview picture (dusk sky, ridges, lake, grey ramp, colour chips) |
| Colour palette and layout metrics | `crates/ui-egui/src/theme.rs`: colour values and sizes only; no artwork |

## Optional build input: extra-fonts

These fonts are **not files in this repository**. Builds made with the optional build input
`CFA4_FONTS_DIR` (see [docs/contributing.md](docs/contributing.md#building-with-extra-fonts))
embed them from an external font directory, where each file,
its author, pinned upstream source and licence text are listed in that directory's
`ATTRIBUTION.md`. Release packages
carry each font's licence as `OFL-<family>.txt`.

| Font | Use in CFA4 | Licence |
|---|---|---|
| BIZ UDPGothic Regular, Bold (Morisawa) | Japanese UI text; Japanese fallback for sans titles (the web build embeds only Regular) | OFL-1.1 |
| Shippori Mincho Regular (FONTDASU) | Japanese fallback for serif titles (native builds) | OFL-1.1 |
| BIZ UDMincho Regular (Morisawa) | Japanese fallback for serif titles (native builds) | OFL-1.1 |

## Third-party source code

Source files that are transcribed from third-party code rather than written from scratch. Each keeps
the original copyright and permission notice in its header.

| File | Author | Source | Licence |
|---|---|---|---|
| `crates/platform/src/nvenc/ffi.rs` | NVIDIA Corporation (`nvEncodeAPI.h`); upstream contributors (Rust transcription) | Structures (including `NV_ENC_CONFIG_HEVC` and its bit-field masks, `NV_ENC_PIC_PARAMS_HEVC`, `NV_ENC_SEI_PAYLOAD`, `NV_ENC_TIME_CODE`), function table, codec / profile GUIDs (H.264, HEVC Main, HEVC Main 10) and constants (`NV_ENC_LEVEL_HEVC_*`, tier, CU size, `NV_ENC_BUFFER_FORMAT_YUV420_10BIT`, `NV_ENC_CAPS_SUPPORT_10BIT_ENCODE`, BT.2020 / PQ / HLG VUI codes) transcribed from `nvEncodeAPI.h`, NVIDIA Video Codec SDK, API 12.1 (https://developer.nvidia.com/video-codec-sdk) | MIT (Copyright (c) 2010-2023 NVIDIA Corporation; notice kept in the file header) |

## Downloaded at runtime

These files are never in the repository or the app bundle. CFA4 downloads them into the
per-user data directory (`<data dir>/models/<id>/`) only after the user confirms a dialog that shows
the size, the source URL and the licence. Every file is pinned to a revision and checked against its
SHA-256 (`crates/speech/src/models.rs`, `docs/transcripts.md`).

| Asset | Author | Source | Licence |
|---|---|---|---|
| Whisper tiny / base / small speech-recognition weights (`model.safetensors`, `config.json`, `generation_config.json`, `tokenizer.json`) | OpenAI | `https://huggingface.co/openai/whisper-{tiny,base,small}` (revisions `169d4a43…`, `e37978b9…`, `973afd24…`); weights released with `https://github.com/openai/whisper` | MIT (weights and code, github.com/openai/whisper); the Hugging Face conversion is labelled Apache-2.0 |
