# Localization parity

> **Nem cél / Non-goal (2026-10-30):** a Windows-, macOS- és Android-paritás nem cél. A CFA4 kizárólag Ubuntu/Kubuntu 26.04 AMD64-re készül; a célalkalmazás más platformokon elérhető funkciói csak a Linuxra átültethető részükben számítanak. Az Apple-formátumok (ProRes, Apple Log, QuickTime/MOV) támogatása formátum, nem platform, ezért marad. / Windows, macOS and Android parity is not a goal; CFA4 targets Ubuntu 26.04 AMD64 only. Apple formats stay supported.

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-11-10 · **Change:** minor (all catalogs in the top-level `i18n/`; Hungarian complete in both frontends) · **Target:** Adobe Premiere Pro 2026 (26.5.2)

Per-language status of CFA4's interface. Premiere 26.5.2 ships 10 interface languages (the
installed bundle's `.lproj` folders): German, English, Spanish, French, Italian, Japanese, Korean,
Portuguese (Brazil), Russian, Simplified Chinese.

## How it works

Edit ▸ Language (also Settings ▸ General, `general.interfaceLanguage`) switches the interface and
persists across restarts; System Language (the default) follows the OS's or browser's preferred
languages and falls back to English. Commands `app.language.<english|japanese|spanish|portuguese|ukrainian|chinese|german|russian|french|indonesian|italian|korean|hungarian|british|australian>`
are reachable over the control channel. One catalog per language,
the top-level `i18n/<code>.tsv`, shared by egui and the Qt GUI (format in the header of `es.tsv`; owner decision 2026-11-09: every CFA4 translation lives there); UI code wraps strings in
`tl!("…")` / `tlf!("…{name}", name)`, and registry names (effects and their parameters, settings,
commands, panels, workspaces, presets) are translated where they are drawn. Searches match the
displayed labels as well as English sources. Tests fail when a `tl!` literal, a menu label or a
registry name has no Spanish entry; `zh-cn.tsv` must also cover every `tl!` literal and menu
label; `ja.tsv`, `fr.tsv`, `pt-br.tsv` and `de.tsv` have the same entries but are not yet required by tests
(French and German must cover the core menus, Brazilian Portuguese every menu label, and neither may keep an
entry without a Spanish one). Engine error messages, the
control channel, the CLI and MCP stay English.

Fonts come from an external font directory (`CFA4_FONTS_DIR`, all
official releases) or an installed system face; without one, switching to Japanese or Chinese is
refused with a message (Simplified and Traditional Chinese alike). Simplified Chinese is picked for
`zh`, `zh-CN`, `zh-SG`, `zh-Hans`; Traditional Chinese for `zh-TW`, `zh-HK`, `zh-MO`, `zh-Hant`. Ukrainian uses the bundled Cyrillic interface fonts. Vertical text in titles
is supported. The interface (egui) shapes each font run with harfrust and has Arabic and Hebrew
fallback fonts (`crates/ui-egui/src/rtl.rs`), but it has no bidi pass: only captions (timeline
blocks, the Text panel list when not being edited) are laid out in reading order, and the interface
itself is never mirrored right to left. The title engine (`crates/text`, harfrust + bidi) lays out
right to left, but Arabic titles are reported broken (#395).

## Status

Measured 2026-10-10 by counting entries in each catalog. The denominator is the union of all
catalogs' sources (**3,498 strings**); strings not yet wrapped in `tl!` and engine messages are
not in it, so real coverage is somewhat lower than shown. Estimates calibrated from #449 (Japanese,
whole interface, one PR) at ~4–8 h per language for the catalog plus tests.

| Language | Code | UI strings | Dialogs / tooltips / help | Script support | Native review | Status | To `full` |
|---|---|---|---|---|---|---|---|
| English (US) | en | 3,498 (100%, source) | yes; no help docs in-app | Latin | n/a | **full** | — |
| English (UK) | en-gb | the source with British spelling where it differs (`en-gb.tsv`: colour, centre, favourite, -ise…; ~200 entries); System Language picks it for `en-GB` and `en-IE` | as English | Latin | not recorded | **full** | — |
| English (Australia) | en-au | the British catalog plus `en-au.tsv` (Australian differences; none yet); System Language picks it for `en-AU` and `en-NZ` | as English | Latin | not recorded | **full** | — |
| Simplified Chinese | zh-cn | 3,485 (99.6%) | dialogs, Settings, effects, shortcuts, status; engine errors English | CJK font fallback; system IME via winit | not recorded | partial | 6–10 h + review |
| Spanish | es | 3,496 (99.9%) | as above | Latin | not recorded | partial | 4–8 h + review |
| Hindi | hi | 0 (0%) | — | **no Devanagari shaping in the interface** | — | none | 30–50 h (shaping in UI + catalog + fonts) |
| Arabic | ar | 0 (0%) | — | **no RTL interface layout**; fallback fonts and shaping in the interface, captions in reading order, other text in egui's left-to-right run order; titles broken (#395) | — | none | 40–70 h |
| French | fr | 3,526 (same entries as Spanish) | as Spanish | Latin (bundled fonts) | not recorded; welcome | partial (#498) | review |
| Portuguese (Brazil) | pt-br | 3,541 (same entries as Spanish) | as Spanish | Latin (bundled fonts) | not recorded; welcome | partial (#859) | review |
| Indonesian | id | 3,541 (same entries as Spanish); must cover the core menus, no entry without a Spanish one | as Spanish | Latin (bundled fonts); System Language also maps the legacy `in` code | not recorded; welcome | partial (#867) | review |
| Japanese | ja | 3,487 (99.7%) | as zh-cn; system-font kanji may pick the Chinese fallback (#547) | CJK fonts, vertical title text | not recorded | partial | 4–8 h + review |
| German | de | 3,590 (same entries as Spanish); must cover the core menus, no entry without a Spanish one | as Spanish | Latin (bundled fonts) | menus yes (#630); the rest not recorded, welcome | partial | review |
| Korean | ko | 3,573 (same entries as Spanish) | as Spanish | installed system Hangul font (Noto Sans CJK KR, NanumGothic…); refused without one | not recorded; welcome | partial (#508) | review; Hangul in extra-fonts for the web |
| Vietnamese | vi | 0 (0%) | — | Latin with stacked diacritics (font coverage to check) | — | none | 6–12 h + review |
| *Other shipped:* Italian | it | 3,573 (same entries as Spanish); must cover the core menus, no entry without a Spanish one | as Spanish | Latin (bundled fonts) | not recorded; welcome | partial | review |
| *Other shipped:* Ukrainian | uk | 351 (10.0%) | menus only | Cyrillic (bundled) | not recorded | menus only | 5–9 h |
| *Other shipped:* Hungarian | hu | 4,072 (every entry of the Spanish catalog, egui and Qt GUI strings); the tests require it to stay complete (`hungarian_covers_the_whole_interface`) | as Spanish | Latin with ő/ű (bundled fonts, `hungarian_renders_with_bundled_fonts`) | not recorded; welcome | **full** (catalog) | review |
| *Other shipped:* Polish | pl | 4,072 (every entry of the Spanish catalog, egui and Qt GUI strings; 570 from craft-i18n); the tests require it to stay complete (`complete_languages_cover_the_whole_interface`) | as Spanish | Latin with ą ę ł ś ź ż (bundled fonts, `complete_languages_render_with_bundled_fonts`) | not recorded; welcome | **full** (catalog) | review |
| *Other shipped:* Turkish | tr | 4,072 (every entry of the Spanish catalog, egui and Qt GUI strings; 500 from craft-i18n); the tests require it to stay complete (`complete_languages_cover_the_whole_interface`) | as Spanish | Latin with ç ğ ı İ ö ş ü (bundled fonts, `complete_languages_render_with_bundled_fonts`) | not recorded; welcome | **full** (catalog) | review |
| *Other shipped:* Dutch | nl | 4,072 (every entry of the Spanish catalog, egui and Qt GUI strings; 503 from craft-i18n); the tests require it to stay complete (`complete_languages_cover_the_whole_interface`) | as Spanish | Latin with é ë ï ö (bundled fonts, `complete_languages_render_with_bundled_fonts`) | not recorded; welcome | **full** (catalog) | review |
| *Other shipped:* Czech | cs | 4,072 (every entry of the Spanish catalog, egui and Qt GUI strings; 566 from craft-i18n); the tests require it to stay complete (`complete_languages_cover_the_whole_interface`) | as Spanish | Latin with č ř š ž ů (bundled fonts, `complete_languages_render_with_bundled_fonts`) | not recorded; welcome | **full** (catalog) | review |
| *Other shipped:* Vietnamese | vi | 4,072 (every entry of the Spanish catalog, egui and Qt GUI strings; 561 from craft-i18n); the tests require it to stay complete (`complete_languages_cover_the_whole_interface`) | as Spanish | Latin with tone marks ơ ư ạ ế (bundled fonts, `complete_languages_render_with_bundled_fonts`) | not recorded; welcome | **full** (catalog) | review |
| *Other shipped:* Traditional Chinese | zh-hant | 4,072 (every entry of the Spanish catalog, egui and Qt GUI strings; 520 from craft-i18n); the tests require it to stay complete (`complete_languages_cover_the_whole_interface`) | as Spanish | Han, needs a Chinese face like Simplified (`cjk.rs`; refused without one) | not recorded; welcome | **full** (catalog) | review |
| *Other shipped:* Greek | el | 4,072 (every entry of the Spanish catalog, egui and Qt GUI strings; 523 from craft-i18n); the tests require it to stay complete (`complete_languages_cover_the_whole_interface`) | as Spanish | Greek script (bundled fonts) | not recorded; welcome | **full** (catalog) | review |
| *Other shipped:* Swedish | sv | 4,072 (every entry of the Spanish catalog, egui and Qt GUI strings; 424 from craft-i18n); the tests require it to stay complete (`complete_languages_cover_the_whole_interface`) | as Spanish | Latin (bundled fonts) | not recorded; welcome | **full** (catalog) | review |
| *Other shipped:* Serbian (Latin) | sr-latn | 4,072 (every entry of the Spanish catalog, egui and Qt GUI strings; 482 from craft-i18n); the tests require it to stay complete (`complete_languages_cover_the_whole_interface`) | as Spanish | Latin script, ekavian; every Serbian locale until a Cyrillic catalog exists | not recorded; welcome | **full** (catalog) | review |
| *Other shipped:* Bulgarian | bg | 4,072 (every entry of the Spanish catalog, egui and Qt GUI strings; 451 from craft-i18n); the tests require it to stay complete (`complete_languages_cover_the_whole_interface`) | as Spanish | Cyrillic; verbal-noun commands | not recorded; welcome | **full** (catalog) | review |
| *Other shipped:* Georgian | ka | 4,073 (every entry of the Spanish catalog, egui and Qt GUI strings; 407 from craft-i18n); the tests require it to stay complete (`complete_languages_cover_the_whole_interface`) | as Spanish | Georgian script; needs an extra-fonts or installed Georgian face | not recorded; welcome | **full** (catalog) | review |

UI strings are catalog entries (the non-comment lines of `i18n/<code>.tsv`; English counts the
source strings in the code), so the rows were measured at different times: at this change the
Spanish and German catalogs both have 3,590 entries, German every Spanish one. Strings added
later may stay English in German until translated; only the core menus are required.

Premiere also ships Italian, which we now cover like Spanish, and Russian (6–10 h).

**Localization dimension:** 9 of the 12 key languages near-complete (English, Spanish, Japanese,
Simplified Chinese, French, Portuguese (Brazil), Indonesian, Korean, German), 3 none (Hindi, Arabic,
Vietnamese); against Premiere's 10 languages we cover 9 well and Russian partly. **~35%, 150–230 h** for all twelve to `full` plus Italian and Russian, of which the script
work (RTL and complex shaping in the egui interface) is the largest and least parallel part.
Every language needs native-speaker review (human).

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-10 | minor | Polish (pl) added, complete: 4,072 entries (570 imported from craft-i18n), in egui's Edit ▸ Language, Settings, System Language and the Qt GUI |
| 2026-11-10 | minor | Turkish (tr) added, complete: 4,072 entries (500 imported from craft-i18n), in egui's Edit ▸ Language, Settings, System Language and the Qt GUI |
| 2026-11-10 | minor | Dutch (nl) added, complete: 4,072 entries (503 imported from craft-i18n), in egui's Edit ▸ Language, Settings, System Language and the Qt GUI |
| 2026-11-10 | minor | Czech (cs) added, complete: 4,072 entries (566 imported from craft-i18n), in egui's Edit ▸ Language, Settings, System Language and the Qt GUI |
| 2026-11-10 | minor | Vietnamese (vi) added, complete: 4,072 entries (561 imported from craft-i18n), in egui's Edit ▸ Language, Settings, System Language and the Qt GUI |
| 2026-11-10 | minor | Traditional Chinese (zh-hant) added, complete: 4,072 entries (520 imported from craft-i18n), in egui's Edit ▸ Language, Settings, System Language and the Qt GUI; zh-TW, zh-HK, zh-MO and zh-Hant locales now pick it |
| 2026-11-10 | minor | Greek (el) added, complete: 4,072 entries (523 imported from craft-i18n), in egui's Edit ▸ Language, Settings, System Language and the Qt GUI |
| 2026-11-10 | minor | Swedish (sv) added, complete: 4,072 entries (424 imported from craft-i18n), in egui's Edit ▸ Language, Settings, System Language and the Qt GUI |
| 2026-11-10 | minor | Serbian, Latin script (sr-latn) added, complete: 4,072 entries (482 imported from craft-i18n), in egui's Edit ▸ Language, Settings, System Language and the Qt GUI; every Serbian locale uses it until a Cyrillic catalog exists |
| 2026-11-11 | minor | Bulgarian (bg) added, complete: 4,072 entries (451 imported from craft-i18n), in egui's Edit ▸ Language, Settings, System Language and the Qt GUI |
| 2026-11-11 | minor | Georgian (ka) added, complete: 4,073 entries (407 imported from craft-i18n), in egui's Edit ▸ Language, Settings, System Language and the Qt GUI; egui adds an extra-fonts or installed Georgian face (none is bundled) and otherwise keeps English, like Korean; one new string in every complete catalog |
| 2026-11-10 | minor | English (UK) and English (Australia) added (US English is the source, shown as English (US)); de, es, fr, it, ja, ko, zh-cn, pt-br and id completed (718 entries) |
| 2026-11-10 | minor | Every catalog moved to the top-level `i18n/` (owner decision 2026-11-09), one per language for egui and the Qt GUI (the Qt-only strings in a "Qt GUI" section; where the two had translated the same string differently, the Qt GUI's wording was kept); Hungarian (hu) completed (1,572 new entries) and added to egui's Edit ▸ Language, Settings and System Language |
| 2026-10-27 | minor | Ubuntu 26.04 only: the macOS app-menu entries (Settings, Hide CFA4, Hide Others) and macOS hints left the catalogs; Korean font list names Linux fonts |
| 2026-10-13 | minor | German (de) covers the whole interface: every entry of the Spanish catalog (3,590); the tests require the core menus and the macOS app menu and reject entries without a Spanish one |
| 2026-10-11 | minor | German (de) ships the menus (#630): 371 of 3,498 strings (10.6%), native-reviewed; dimension now 2 menus-only, 5 none |
| 2026-10-10 | major | Created from ROADMAP.md's "Interface localization" section; catalog coverage measured (es 3,496, ja 3,487, zh-cn 3,485, pt-br 348, uk 351 of 3,498); the 12-language table and script-support gaps added |
