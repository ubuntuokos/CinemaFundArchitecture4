# Credits sequences (stáblista)

CFA4 builds film credits from a spreadsheet: write the credits in LibreOffice Calc (or any
spreadsheet), save as CSV, and **Graphics and Titles ▸ Import Credits Spreadsheet…** places one
graphic clip per page from the playhead. Each page is an ordinary graphic: edit it in Essential
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
| Page Runtime | Játékidő | Page length: `00:00:12:00`, `12s`, `300f` |
| Page Gap | Oldalköz | Empty time after the page; negative overlaps the next page |

A new head, an empty row, a new style or a new page starts a new block. `{{Blank}}` is an empty
line inside a block. Line breaks inside a cell are kept. Example:
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

## Commands

| Command | Menu | Params |
|---|---|---|
| `graphics.credits.import` | Graphics and Titles ▸ Import Credits Spreadsheet… | `path` (.csv/.tsv) or `csv` (text), `styling`, `time`, `track` → `{clips, end, warnings}`; one undo step |
| `graphics.credits.preview` | (query) | as above → `{pages: [{name, behaviour, duration, seconds, layers, contentHeight}], duration, warnings}` |
| `graphics.credits.styles` | (query) | `styling` → style names, column names and a starter styling |

Problems in the sheet (unknown styles, bad numbers, unsupported `{{…}}` tags) are warnings with
the row number; the import still succeeds. Wrong files, missing header, too-large input and
broken JSON are errors.

## Not yet

Office files (`.xlsx`, `.ods`) and online sheets (save as CSV), inline letter styles, pictures
and videos, parallel and hooked spines, leaders, head-width harmonisation, and stretching gaps
to hit a scroll runtime (a runtime currently changes the speed).
