# Credits sequences (stáblista)

CFA4 builds film credits from a spreadsheet: write the credits in LibreOffice Calc, Excel or
any spreadsheet, save it (`.ods`, `.xlsx` or CSV), and **Graphics and Titles ▸ Import Credits
Spreadsheet…** opens the Credits window (Qt6): choose the sheet, see the pages, warnings and a
picture of each page, then place them at the playhead: one graphic clip per page. Each page is an ordinary graphic: edit it in Essential
Graphics, trim it, export it. Design decision: [ADR F10](adr/ADR-F10-CREDITS-SPREADSHEET-2026-10-23.md).
Code: [`crates/credits`](../crates/credits/README.md), `crates/engine/src/credits.rs`.

## The spreadsheet

A header row names the columns; rows above it are ignored. Names are case-insensitive, a
leading `@` is allowed, and Hungarian names work too.

| Column | Hungarian | Meaning |
|---|---|---|
| Head | Fej, Szerep | Role / heading of a block |
| Body | Törzs, Nevek | Names; consecutive cells form one block. **Required.** |
| Tail | Láb | Text after the body (e.g. a character name) |
| Vertical Gap | Térköz | Space before the next block: `2` (units) or `40px`. An empty row = 1 unit |
| Content Style | Stílus | Style of this and the following blocks |
| Spine Position | Gerinc | Horizontal offset of the following blocks from the centre, pixels |
| Page Style | Oldal | Starts a new page with that style (`Scroll`, `Card`, or your own) |
| Page Runtime | Játékidő | Page length: `00:00:12:00`, `12s`, `300f`. On a scroll page the speed is kept and the gaps between blocks stretch or shrink to fit (up to 20-fold); if they cannot, the speed changes, with a warning |
| Page Gap | Oldalköz | Empty time after the page; negative overlaps the next page |

A new head, an empty row, a new style or a new page starts a new block. `{{Blank}}` is an empty
line inside a block. Line breaks inside a cell are kept.

**Files.** `.ods` (LibreOffice), `.xlsx` (Excel, LibreOffice, Google Sheets download) and CSV /
TSV (comma, semicolon or tab). A workbook's first sheet is used unless `sheet` names another (by
name or number). Stored values are read, not display formats: write durations as text (`12s`,
`00:00:12:00`). Apple Numbers: export as `.xlsx` or CSV.

**Pictures and logos.** A body cell holding only picture tags shows pictures instead of text:
`{{Pic logo.png}}`, with an optional height in pixels of a 1080-high frame
(`{{Pic logo.png 120}}`, default 150); `{{Kép …}}` and `{{Logo …}}` work too. Several tags in one
cell stand side by side. Names are relative to the spreadsheet's folder (or `pictureDir`), or
absolute. Each picture becomes its own clip above the page: on a scroll page it moves exactly
with the roll, on a card it fades with the card; the page keeps a transparent placeholder layer
(“Picture 1: logo.png”) so its height and roll include the picture. A missing picture is a
warning and is left out.

Example:
[`crates/credits/examples/credits.csv`](../crates/credits/examples/credits.csv).

## Styles

Sizes are pixels of a 1080-pixel-high frame and scale with the sequence.

- **Page styles:** `Scroll` (rolls from below to above the frame, 3 px per frame) and `Card`
  (120 frames, 12-frame fades).
- **Content styles:** `Gutter` (role left of the centre, names right; the default), `Centered`,
  `Heading`, `Title`, `Grid` (3 columns), `Bullets` (names run on with •), `Paragraphs`.

Your own styles are JSON (`styling` parameter, or a `.json` path). `graphics.credits.styles`
returns every built-in style as a starter file to copy and edit:

```json
{
  "unitPx": 32,
  "defaultPageStyle": "Scroll",
  "defaultContentStyle": "Gutter",
  "pageStyles": [{ "name": "Slow", "behaviour": "scroll", "scrollPxPerFrame": 2 }],
  "contentStyles": [{
    "name": "Cast", "orientation": "horizontal", "layout": "grid", "columns": 1,
    "head": { "font": "Inter", "fontStyle": "Regular", "size": 34, "color": "#c8c8c8" },
    "body": { "font": "Inter", "fontStyle": "SemiBold", "size": 40, "color": "#ffffff" },
    "headGap": 40
  }]
}
```

Content style fields: `orientation` (`horizontal` = head beside the body, `vertical` = above),
`layout` (`grid`, `flow`, `paragraphs`), `columns`, `align` (`left`, `center`, `right`), `body` /
`head` / `tail` letter styles (`font`, `fontStyle`, `size`, `color`, `tracking`, `allCaps`,
`leading`), `headGap`, `tailGap`, `columnGap`, `paragraphGap`, `wrapWidth`, `flowSeparator`,
`margin`. Page style fields: `behaviour` (`scroll`, `card`), `scrollPxPerFrame`, `cardFrames`,
`fadeInFrames`, `fadeOutFrames`, `gapAfterFrames`.

## Checks

The Credits window lists the engine's checks under *Checks and warnings*, with an error, warning
or information icon and the spreadsheet row or page (`graphics.credits.validate`, also part of
`graphics.credits.preview`). They are advice: nothing stops the import.

| Check | Severity | When |
|---|---|---|
| A name twice in one block | warning | the same name (ignoring case, accents, spacing) appears twice in a block |
| One person written two ways | warning | e.g. `Kiss Anna` and `Kiss Ánna`; names the other row |
| Font not installed | error | another font is used instead |
| Letters missing from a font | warning | e.g. a font without `ő` / `ű`: another font draws them |
| Outside the title-safe area | warning | text beyond 5 % from the frame edges (rolls: the sides only) |
| Low contrast | warning | text colour against black below 4.5:1 (WCAG 2.1) |
| Small text | warning | below 24 px on a 1080-line frame |
| Shimmering roll | warning | not a whole number of pixels per frame |
| Fast roll | warning | a line crosses the frame in under 6 seconds |
| Short card | warning | less than 2 seconds on screen after its fades |
| Overlapping pages | info | a negative page gap |
| Soft picture | warning | a picture shown larger than its pixels |

More than 50 of one kind are counted in one line. Codes, severities and values are stable
(`crates/credits/src/validate.rs`), so the Qt GUI words them in Hungarian.

## Credits files and refreshing (linked credits)

**Credits file (`.cfa4credits`).** *Save as Credits File…* in the Credits window (or
`graphics.credits.save`) writes one versioned JSON file with the spreadsheet's path, file name
and worksheet, the picture folder, the styling, and **the rows of the sheet embedded**: it opens
on any machine, without the spreadsheet. Open it like a spreadsheet (the Credits window, or
`path` of `graphics.credits.import` / `.preview`). Newer versions are refused; unknown fields
are ignored.

| Field | Meaning |
|---|---|
| `format`, `version` | `"cfa4.credits"`, `1` |
| `source` | `path` (absolute), `name` (tried next to the project when the path is gone), `sheet` |
| `pictureDir` | folder of `{{Pic …}}` pictures |
| `styling` | the styling (see above) |
| `table` | the accepted rows of the sheet |

**Linked pages.** Every placed page keeps the credits document as accepted, its pictures' clips
and a fingerprint of every layer it made (`GraphicMeta.credits` in the project file). When the
spreadsheet changes, **Graphics and Titles ▸ Refresh Credits from Spreadsheet…** (with a credits
page selected) lists what changed since, **row by row** – added, removed and changed rows,
before and after – with a tick for each, and the layers you edited by hand. *Apply* replaces the
pages in place with the ticked changes, as one undo step:

- unticked changes are not applied, and are offered again next time;
- your hand edits are kept where their block is unchanged (*Keep my edits…*); the window says
  which edits would be replaced because their block changed;
- if the spreadsheet moved, *Locate Spreadsheet…* gives its new place.

Trailing empty rows are not changes; empty rows inside the sheet are (they are gaps).

Hand edits are carried over as what you changed: a layer whose text or colour you edited keeps
your text or colour, but moves to where the new layout puts it, unless you moved it yourself. A
block that appears twice (e.g. the same heading on two pages) is two blocks: editing one does
not change the other. Copied or pasted credits pages are credits of their own: refreshing the
copy leaves the original alone. Import and refresh are each one undo step, and a refresh that
fails leaves the old pages as they were.

## Commands

| Command | Menu | Params |
|---|---|---|
| `graphics.credits.import` | Graphics and Titles ▸ Import Credits Spreadsheet… | `path` (.csv/.tsv/.xlsx/.ods/.cfa4credits) or `csv` (text), `sheet`, `pictureDir`, `styling`, `time`, `track` → `{clips: [{clip, start, duration, pictures}], group, end, sheet, warnings}`; one undo step |
| `graphics.credits.preview` | (query) | as above → `{pages: [{name, behaviour, duration, seconds, layers, contentHeight}], duration, sheets, sheet, warnings, issues}` |
| `graphics.credits.thumbnail` | (query) | as preview, plus `page`, `at` (0–1, moment in the page), `width` → `{width, height, page, pages, time, pngBase64}`: the page's text with the roll or fades applied, pictures outlined |
| `graphics.credits.validate` | (query; shown in the Credits window) | as preview, or `clip` for placed credits → `{errors, warnings, infos, issues: [{code, severity, page?, row?, values, message}]}` |
| `graphics.credits.save` | Credits window: *Save as Credits File…* | `to` (.cfa4credits), and `clip` (placed credits; default the selection) or `from` {import parameters} → `{path, rows}` |
| `graphics.credits.diff` | (query) | `clip`?, `source`? (the spreadsheet's new place) → `{group, clip (a page of the group), source, changes: [{id, kind: added/removed/changed, oldRows, newRows (1-based), before, after}], manual: [{key, page, layer, text, survivesAll}]}` |
| `graphics.credits.refresh` | Graphics and Titles ▸ Refresh Credits from Spreadsheet… | `clip`?, `source`?, `accept` ("all" / "none" / [ids], default all), `keepManual` (default true), `styling`? → `{clips, accepted, changes, keptEdits, lostEdits, warnings}`; one undo step |
| `graphics.credits.styles` | (query) | `styling` → style names, column names and a starter styling |

Problems in the sheet (unknown styles, bad numbers, unsupported `{{…}}` tags) are warnings with
the row number; the import still succeeds. Wrong files, missing header, too-large input and
broken JSON are errors.

## Not yet

Online sheets (download as `.xlsx`), inline letter styles, videos in the credits, parallel and
hooked spines, leaders, head-width harmonisation.
