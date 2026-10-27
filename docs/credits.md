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
| Spine Position | Gerinc | Horizontal offset of the following blocks from the centre, pixels; `450 parallel` (`párhuzamos`) starts a column beside the previous one, from the same top |
| Page Style | Oldal | Starts a new page with that style (`Scroll`, `Card`, or your own) |
| Page Runtime | Játékidő | Page length: `00:00:12:00`, `12s`, `300f`. On a scroll page the speed is kept and the gaps between blocks stretch or shrink to fit (up to 20-fold); if they cannot, the speed changes, with a warning |
| Page Gap | Oldalköz | Empty time after the page; negative overlaps the next page |

A new head, an empty row, a new style or a new page starts a new block. `{{Blank}}` is an empty
line inside a block. Line breaks inside a cell are kept.

**Letter styles inside a cell.** `{{Style Bold}}Kiss{{Style}} Anna` sets *Kiss* in bold: a
`{{Style Name}}` tag (`{{Stílus Név}}`) switches the text after it to that inline style until the
next style tag or the end of the line, and `{{Style}}` alone switches back. Built-in: `Bold`
(`Félkövér`), `Italic` (`Dőlt`), `Caps` (`Nagybetűs`), `Underline` (`Aláhúzott`); your own are
`inlineStyles` in the styling (`font`, `fontStyle`, `size`, `color`, `tracking`, `allCaps`,
`underline`, `baselineShift`; only what a style names changes). They become the text layer's own
character styles, editable in Essential Graphics. An unknown name is a warning.

**Parallel columns.** Blocks with a spine position followed by `parallel` stand beside the
column before them, starting at the same height: e.g. *Camera* at `-450`, then *Sound* at
`450 parallel`. The column set ends at the next spine position without `parallel`; what follows
goes below the longest column. On a roll with a `Page Runtime`, the first column's gaps stretch.

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

**Videos.** `{{Video intro.mp4}}` (`{{Videó …}}`, `{{Mozgókép …}}`, with an optional height like
pictures) puts a video in the credits the same way: its own clip that moves with the roll or comes
in with the card, playing from its start; a video shorter than its page ends early. Its size is
read from the file (a file over 256 MiB is taken as 16:9).

**Portraits.** A body cell with one picture tag and a name – `{{Pic anna.jpg}} Kiss Anna` – is a
name with its portrait. A block with portraits is a grid of faces with the names under them
(`portraitSide: left`: beside them), `columns` to a row, `portraitHeight` (default 220) high, the
built-in `Portraits` content style showing four to a row. Cells without a picture are names only;
a picture that cannot be read is a warning and the name stays.

Example:
[`crates/credits/examples/credits.csv`](../crates/credits/examples/credits.csv).

## Styles

Sizes are pixels of a 1080-pixel-high frame and scale with the sequence.

- **Page styles:** `Scroll` (rolls from below to above the frame, 3 px per frame), `Card`
  (120 frames, 12-frame fades) and `Crawl` (a one-line ticker near the bottom, right to left,
  4 px per frame).
- **Content styles:** `Gutter` (role left of the centre, names right; the default), `Centered`,
  `Heading`, `Title`, `Grid` (3 columns), `Bullets` (names run on with •), `Paragraphs`,
  `Leaders` (roles in one even left column, dots to the names).

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
`margin`, and for the horizontal orientation `headAlign` (`right` by default; `left` makes a
column), `harmonizeHeads` (every head of the style on a page gets the widest one's width),
`leader` (e.g. `" ."`: repeated from the head to the body, on the head's line, in its letter
style) and `leaderGap` (space on either side of the dots, default 16); for portraits
`portraitHeight`, `portraitSide` (`above`, `left`) and `portraitGap`.

**Card entrances.** A card page style can bring its content in: `entrance` (`slideLeft`,
`slideRight`, `slideUp`, `slideDown`) moves it `entranceDistance` pixels (default 120) over
`entranceFrames` (default 18, easing out) while it fades in, and `staggerFrames` brings each block
in that many frames after the one before. Pictures and videos come in with their block. They are
ordinary keyframes on the layers, editable afterwards. Page style fields: `behaviour` (`scroll`, `card`, `crawl`), `scrollPxPerFrame` (rolls
and crawls), `cardFrames`, `fadeInFrames`, `fadeOutFrames`, `gapAfterFrames`, `crawlLine`,
`crawlGap`, `background`, `transition`, `transitionFrames` (see below).

## Backgrounds, transitions and crawls

**Background.** A page style can put something behind its pages:

```json
{ "name": "Night", "behaviour": "card",
  "background": { "kind": "gradient", "color": "#1d2b53", "color2": "#000000", "angle": 90,
                  "opacity": 100, "blur": 0 } }
```

`kind` is `none` (the default: whatever is on the tracks below shows through, black on an empty
sequence), `color`, `gradient` (from `color` to `color2` along `angle` degrees), `image` or
`video` (`file`, relative to the picture folder or absolute; `fit`: `fill` covers the frame and
cuts the overhang, `fit` shows it whole). `opacity` is 0–100 %, `blur` a radius in pixels of a
1080-high frame. The background is **its own clip just below the page**, as long as the page: it
does not roll, and its opacity, blur (Gaussian Blur), size and position stay editable like any
clip's. A video shorter than the page ends early (it is not looped). A missing file is a warning
and the background is left out. A refresh replaces the backgrounds with the pages.

**Transition.** `transition` names the transition into the next page – `dissolve` (or
`áttűnés`), `push` (`eltolás`), `slide`, `wipe`, `dip to black`, or any video transition's id
(`iris_round`, …) – and `transitionFrames` its length (default 12). It is an ordinary transition
on the cut, centred, from the transition library, so it can be changed or removed on the
timeline; when both pages have a background, their backgrounds get the same transition. It needs
the pages to touch (page gap 0; otherwise a warning), the last page has none, and on cards it
takes the place of the fade at that cut. All pages of the credits go on one track (the asked
`track`, else the first above everything the credits cover), their backgrounds on the track
below, so every transition finds its two pages. Pictures (`{{Pic …}}`) fade out and in across
half of each transition.

**Crawl.** A `crawl` page puts its blocks side by side on one line – each block as it would look
on a roll (head above or beside the body), `crawlGap` pixels (default 120) plus the block's own
vertical gap apart – centred at `crawlLine` (fraction of the frame height from the top, default
0.85) and runs it from the right edge to the left at `scrollPxPerFrame` (the graphic's Crawl
Left). `Page Runtime` sets its length; the speed changes to fit. Pictures crawl with it.

**Guides.** Below the page picture of the Credits window, *Safe areas and spines* draws the
action-safe (93 %) and title-safe (90 %) areas, the centre lines and the spines of the page's
blocks; *Baseline grid* draws lines one gap unit (`unitPx`) apart that move with the page. They
are only in the picture, never in the credits.

## Cinecred projects

A [Cinecred](https://cinecred.com) project folder opens in the Credits window like a spreadsheet
(*Cinecred Project…*, or `path` naming the folder or its `Styling.toml`): its credits
spreadsheet (the one called *Credits…*, else the first; `credits` picks another) with its
`Styling.toml` turned into CFA4 styles. Pictures are found in the project folder, also without
their extension (`{{Pic logo}}`). Cinecred's column and tag names in its languages work in any
CFA4 sheet (`@Kopf`, `{{Bild …}}`, `{{Stil …}}`, `400 Parallel`, …).

What carries over: page styles (card / scroll, card length and fades, scroll speed, page gap),
content styles (orientation, grid / flow / paragraphs, columns, gaps, wrap widths, head
alignment, head-width harmonisation, head leaders), letter styles (font and weight, size and
leading, tracking, capitals, the colour of the first coloured layer), the unit gap; sizes are
scaled from the project's resolution to a 1080-line frame. What does not – each is listed among
the warnings: further letter layers (outlines, shadows, stripes), transition, picture and video
(tape) styles, sorting, and the whole-credits runtime (use *Fit Credits to Music*). The
spreadsheet stays the source: *Refresh Credits from Spreadsheet* works as for any sheet.

## Templates

A credits template is a ready look: the styling (page and content styles, backgrounds,
transitions, entrances, inline styles), an optional fitting length and a sample sheet showing
the pages it is made for. In the Credits window, *Template* picks one (a styling file, if given,
wins); *New Spreadsheet from Template…* writes its sample as a CSV to start from; *Save as
Template…* keeps the current look (styling or template, and the sheet's first rows as the sample)
as your own (a background picture or video is then named by its full path, so the template finds
it from any spreadsheet). **Graphics and Titles ▸ Apply Credits Template…** gives placed credits
another look, keeping their rows and hand edits (even when the template changes the blocks'
default style), as one undo step. Template files over 8 MiB are skipped.

| Kind | Where | Licence |
|---|---|---|
| built in | in CFA4: *Classic Roll*, *Cards and Roll*, *News Crawl*, *Cast Portraits* (original designs) | MIT OR Apache-2.0 |
| user | `<data dir>/Credits Templates/*.cfa4credtpl` | what its `license` says |
| project | kept in the project file | what its `license` says |

A template file is versioned JSON: `format` (`"cfa4.credits-template"`), `version` (1), `id`,
`name`, `description`, `category`, `source` (who made it), `license`, `styling`, `sample` (rows),
`fit`. Newer versions are refused; unknown fields are ignored.

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
| Outside the title-safe area | warning | text beyond 5 % from the frame edges (rolls: the sides only; crawls: top and bottom only) |
| Low contrast | warning | text against the page's background colour (both ends of a gradient; black without a background; a background's opacity mixed with black in linear light, as it is rendered) below 4.5:1 (WCAG 2.1) |
| Contrast not measured | info | a picture or video is behind the text: check it by eye |
| Small text | warning | below 24 px on a 1080-line frame |
| Shimmering roll or crawl | warning | not a whole number of pixels per frame |
| Fast roll or crawl | warning | a line crosses the frame (a crawl: its width) in under 6 seconds |
| Short card | warning | less than 2 seconds on screen after its fades |
| Overlapping pages | info | a negative page gap |
| Soft picture | warning | a picture shown larger than its pixels |

More than 50 of one kind are counted in one line. Codes, severities and values are stable
(`crates/credits/src/validate.rs`), so the Qt GUI words them in Hungarian.

## Fit to music

With a credits page selected, **Graphics and Titles ▸ Fit Credits to Music…** picks the music –
the selected audio clip, else the audio clip that overlaps the credits most – and shows what the
engine found in it: its length, where the sound ends, its tempo, and the credits' length without
fitting. The music is heard as the timeline plays it: with its speed, reverse, Remix edits and
clip effects. Without usable music (none under the credits, or offline) the window still fits to
a typed length. It offers lengths, and changes nothing until you choose:

- **the whole music**; **until the sound ends** (when it ends more than half a second before the
  clip); **to the last bar line** before the sound ends (bars of four beats; the beats come from
  the same analysis as Clip ▸ Remix); or a length you type;
- **Cards last whole beats** – cards without their own `Page Runtime` last a whole number of
  beats and start on a beat (the scroll page before a card gives or takes up to half a beat; the
  last scroll page evens it out), so their cuts fall on the beat; cards after the last scroll page
  are on the beat when the length is (the last bar line, for example);
- **Start the credits where the music starts**.

*Fit to This Length* replaces the pages in place, as one undo step, keeping hand edits: cards,
page gaps and pages with their own runtime keep their length, and the scroll pages share the rest
in proportion to their natural length, each reaching its share by stretching or squeezing the
gaps between its blocks at its normal speed (if a page cannot, its speed changes, with a
warning). The result is exact to the frame. The fitting is stored in the credits document
(`fit`), so *Refresh Credits from Spreadsheet* keeps it; *Remove Fitting* undoes it.

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
| `graphics.credits.import` | Graphics and Titles ▸ Import Credits Spreadsheet… | `path` (.csv/.tsv/.xlsx/.ods/.cfa4credits, or a Cinecred project folder / its `Styling.toml`, with `credits` to pick its spreadsheet) or `csv` (text), `sheet`, `pictureDir`, `styling`, `template` (its styling and fitting; `styling` wins), `time`, `track` (of each page's lowest clip: its background, else the page) → `{clips: [{clip, start, duration, behaviour, pictures, background, transition}], group, end, sheet, warnings}`; one undo step |
| `graphics.credits.preview` | (query) | as above → `{pages: [{name, behaviour (scroll/card/crawl), duration, seconds, layers, contentHeight, contentWidth, background, transition: {effect, duration}}], duration, sheets, sheet, warnings, issues}` |
| `graphics.credits.thumbnail` | (query) | as preview, plus `page`, `at` (0–1, moment in the page), `width`, `guides`, `grid` → `{width, height, page, pages, time, pngBase64}`: the page's background (a picture or video as grey) and text with the roll, crawl or fades applied, pictures outlined, guides drawn |
| `graphics.credits.validate` | (query; shown in the Credits window) | as preview, or `clip` for placed credits → `{errors, warnings, infos, issues: [{code, severity, page?, row?, values, message}]}` |
| `graphics.credits.music` | (query; the Fit window) | `audio`?, `clip`? → `{audio, name, start, duration, soundEnd, bpm, beat, beats, suggestions: [{id: musicLength/soundEnd/lastBar, total}], credits, natural, fit}`; changes nothing |
| `graphics.credits.fit` | Graphics and Titles ▸ Fit Credits to Music… | `clip`?, `total` / `seconds` / `use` (a suggestion id, default `musicLength`), `audio`?, `cardsOnBeat`, `alignToAudio`, `off` → `{clips, end, fit, report: {natural, target, result, scrollPages}, warnings}`; one undo step |
| `graphics.credits.save` | Credits window: *Save as Credits File…* | `to` (.cfa4credits), and `clip` (placed credits; default the selection) or `from` {import parameters} → `{path, rows}` |
| `graphics.credits.diff` | (query) | `clip`?, `source`? (the spreadsheet's new place) → `{group, clip (a page of the group), source, changes: [{id, kind: added/removed/changed, oldRows, newRows (1-based), before, after}], manual: [{key, page, layer, text, survivesAll}]}` |
| `graphics.credits.refresh` | Graphics and Titles ▸ Refresh Credits from Spreadsheet… | `clip`?, `source`?, `accept` ("all" / "none" / [ids], default all), `keepManual` (default true), `styling`? → `{clips, accepted, changes, keptEdits, lostEdits, warnings}`; one undo step |
| `graphics.credits.styles` | (query) | `styling` → style names, inline style names, column names and a starter styling |
| `graphics.credits.templates` | (query) | → `{templates: [{id, name, description, category, source, license, path, pages, fit}]}` |
| `graphics.credits.template.get` | (query) | `id` (id, name or `.cfa4credtpl` path) → `{template, sampleCsv}` |
| `graphics.credits.template.save` | Credits window: *Save as Template…* | `name`, `description`?, `license`?, `source`?, `where` (`user`, `project`, `file` with `to`), and `clip` (placed credits) or `from` {import parameters} or `styling` (+ `sample`) → `{id, category, path?}`; to the project: one undo step |
| `graphics.credits.template.delete` | (query) | `id` (user or project templates) |
| `graphics.credits.template.apply` | Graphics and Titles ▸ Apply Credits Template… | `id`, `clip`? → `{clips, end, template, warnings}`; one undo step |
| `graphics.credits.template.newSheet` | Credits window: *New Spreadsheet from Template…* | `id`, `to` (.csv) → `{path, rows}` |

Problems in the sheet (unknown styles, bad numbers, unsupported `{{…}}` tags) are warnings with
the row number; the import still succeeds. Wrong files, missing header, too-large input and
broken JSON are errors.

## Not yet

Online sheets (download as `.xlsx`), hooked and fused spines, body-width harmonisation, looping
a short background or credits video, writing Cinecred projects (only on request, per the plan).
