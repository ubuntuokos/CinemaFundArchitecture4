# cfa4-credits

Film credits from a spreadsheet for CFA4 (layer L2, no I/O, wasm-clean).

- `table`: CSV / TSV / semicolon reader (BOM, quotes, line breaks in cells), with limits.
- `zip`, `office`: `.xlsx` (ECMA-376) and `.ods` (OASIS ODF) packages, from the specifications.
- `sheet`: Head / Body / Tail blocks, gaps, content and page styles, spine offsets, page
  runtimes and gaps → `Credits`, plus row-numbered warnings.
- `style`: `Styling` (JSON) with original built-in page and content styles; page backgrounds
  and transitions.
- `layout`: one graphic clip per page (text layers; Roll for scroll pages, Crawl Left for crawl
  pages, opacity fades for cards), sizes from a caller-supplied `Measure`; pictures as
  transparent placeholders plus placements the engine turns into clips; the page's background
  and transition into the next page for the engine to place; spines for the preview guides.
- `doc`, `diff`, `validate`, `fit`: credits files, row-by-row refresh, checks, fit to music.

Clean-room: written from the public user guide of Cinecred (what its spreadsheet does), never
from its GPL source. User documentation: [`docs/credits.md`](../../docs/credits.md). Decision:
[ADR F10](../../docs/adr/ADR-F10-CREDITS-SPREADSHEET-2026-10-23.md).

Tests: `cargo test -p cfa4-credits` (semantics, layout with a deterministic measurer, and
a mutation-fuzz test that hostile sheets and stylings never panic); engine tests in
`crates/engine/src/credits_tests.rs` use the real text engine.
