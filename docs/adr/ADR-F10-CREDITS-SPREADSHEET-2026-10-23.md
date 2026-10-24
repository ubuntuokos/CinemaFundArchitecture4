# CFA4 ADR F10 – Credits sequences from a spreadsheet (Cinecred-like capability)

**Date:** 2026-10-23  
**Status:** ACCEPTED; ENGINE + egui MENU IMPLEMENTED (CSV, xlsx, ods, pictures); Qt6 PANEL PENDING (F8 bridge)  
**Owner request:** “a Cinecred feliratozó beépítése a CFA4-be” (owner: ubuntuokos)

## Context

The owner asked to build [Cinecred](https://github.com/LoadingByte/cinecred) into CFA4. Three
facts rule out taking the program itself:

1. **Licence.** Cinecred is GPL-3.0-or-later; CFA4 is MIT OR Apache-2.0. `AGENTS.md` §2 forbids
   copying GPL code, and `VISION.md` §4 says CFA4 takes over other programs' *capabilities*, not
   their code, names or trademarks.
2. **Language.** Cinecred is Kotlin on the JVM; CFA4's product code is pure Rust with a Qt6
   (PySide6) front end (`CLAUDE.md`, ADR F8).
3. **Kind.** Cinecred makes **film credits** (rolling and card credits from a spreadsheet), not
   subtitles. Subtitles (Subtitle Edit capabilities) remain a separate VISION area.

## Decision

The owner chose a **clean-room CFA4 credits module** (2026-10-23):

- New crate `crates/credits` (`cfa4-credits`, layer **L2**, no I/O, wasm-clean): CSV / TSV
  reader, spreadsheet semantics, page / content / letter styles (JSON), layout into ordinary
  graphic-clip text layers with exact `Tick` durations.
- Engine module `crates/engine/src/credits.rs` (L4): commands `graphics.credits.import`,
  `graphics.credits.preview`, `graphics.credits.styles`; measurement with the real text engine.
- egui: Graphics and Titles ▸ Import Credits Spreadsheet… (file dialog).
- Written only from Cinecred's **public user guide** (cinecred.com/guide: what the spreadsheet
  columns and styles do). No Cinecred source file was opened. Built-in styles are original.
- Output is **native CFA4 graphics**: each page is a graphic clip that can be edited in
  Essential Graphics, saved as a `.fcgt` template, exported and rendered by the existing CPU /
  GPU pipeline. Scroll pages reuse the graphic Roll; cards use opacity keyframes.

## Scope of the first version

| Capability | Status |
|---|---|
| CSV / TSV / `;` sheets, BOM, quoted cells with line breaks | done |
| Head / Body / Tail blocks, `{{Blank}}`, empty-row gaps, Vertical Gap (units / px) | done |
| Content styles: Gutter (head beside body), Centered, Heading, Title, Grid (n columns), Bullets (flow), Paragraphs; custom styles in JSON | done |
| Page styles: Scroll (px per frame), Card (runtime, fades); Page Runtime, Page Gap (also negative) | done |
| Spine position as a horizontal pixel offset | done |
| Scaling to any frame size (designed at 1080 p) | done |
| English and Hungarian column names | done |
| `.xlsx` (ECMA-376) and `.ods` (OASIS ODF) from their specifications, sheet choice; own ZIP reader on CFA4's RFC 1951 inflater | done (2026-10-24) |
| Online sheets, Apple Numbers | not yet: download / export as `.xlsx` (clear error) |
| Pictures and logos (`{{Pic file [height]}}`): own clips that follow the roll exactly / fade with the card, placeholder layer in the page | done (2026-10-24) |
| Inline letter styles and videos (`{{…}}`) | not yet: tags are removed with a warning |
| Parallel / hooked spines, head/tail leaders, width harmonisation | not yet |
| Scroll runtime by stretching gaps (instead of changing speed) | not yet: a runtime changes the speed |
| Qt6 panel | pending: the F8 Qt bridge allowlists read-only commands; `graphics.credits.preview` can be allowlisted first, `…import` once mutating commands are allowed |

## Consequences

- No GPL code, no JVM, no new third-party dependency (only workspace `serde`, `serde_json`,
  `roxmltree`; the inflater is CFA4's own, now exported as `cfa4_media::inflate`).
- Never-crash: input limits (16 MiB, 100 000 rows, 256 columns, 20 000 chars per cell, 1 000
  pages, 50 000 blocks, 5 000 layers per page; ZIP: 10 000 entries, 64 MiB per entry, no ZIP64 or
  encryption; XML: 8 million nodes, no DTD), no panicking calls, mutation-fuzz tests for sheets,
  stylings and ZIP packages.
- A whole import is one undo step.
