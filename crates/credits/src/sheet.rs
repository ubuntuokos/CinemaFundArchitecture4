//! The credits spreadsheet: from a table of cells to pages of blocks.
//!
//! A header row names the columns (case and a leading `@` are ignored; English and Hungarian
//! names are accepted, see [`Column::from_header`]). Rows above the header are ignored, so a
//! sheet can start with a title or notes.
//!
//! - **Head / Body / Tail** form a block. Consecutive body cells belong to one block; a new head,
//!   an empty row, a new content style or a new page starts the next one.
//!   A tail may stand on any row of its block. `{{Blank}}` in a cell is an empty line. A cell's own line breaks are kept.
//! - **Vertical Gap**: an empty row is one gap unit; a number in this column between two blocks
//!   replaces that gap (`2` = two units, `40px` = 40 pixels).
//! - **Content Style** applies to this and every following block until another is named.
//! - **Spine Position**: horizontal offset (pixels from the frame centre) of the following blocks.
//! - **Page Style** starts a new page with that style.
//! - **Page Runtime** sets the current page's duration; **Page Gap** the empty time after it.
//!   Durations are timecodes (`00:00:12:00`), seconds (`12s`, `1.5s`) or frames (`300f`).
//!
//! Anything not understood becomes a [`Warning`] with its row number; reading never fails on
//! content, only on limits ([`crate::table::Limits`]).

use cfa4_time::{FrameRate, Tick, parse_timecode};

use crate::CreditsError;

/// Something in the spreadsheet that was ignored or adjusted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    /// 1-based spreadsheet row (0 = not tied to a row).
    pub row: usize,
    pub message: String,
}

/// A spreadsheet column the reader understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Head,
    Body,
    Tail,
    Gap,
    Style,
    Spine,
    Page,
    Runtime,
    PageGap,
}

impl Column {
    /// Recognise a header cell.
    pub fn from_header(cell: &str) -> Option<Column> {
        let n: String = cell.trim().trim_start_matches('@').to_lowercase().chars().filter(|c| !matches!(c, ' ' | '-' | '_')).collect();
        Some(match n.as_str() {
            "head" | "fej" | "fejléc" | "szerep" => Column::Head,
            "body" | "törzs" | "nevek" | "név" => Column::Body,
            "tail" | "láb" | "farok" => Column::Tail,
            "verticalgap" | "gap" | "térköz" | "függőlegestérköz" => Column::Gap,
            "contentstyle" | "style" | "stílus" | "tartalomstílus" => Column::Style,
            "spineposition" | "spine" | "gerinc" | "gerincpozíció" => Column::Spine,
            "pagestyle" | "page" | "oldal" | "oldalstílus" => Column::Page,
            "pageruntime" | "runtime" | "játékidő" | "oldalhossz" => Column::Runtime,
            "pagegap" | "oldalköz" => Column::PageGap,
            _ => return None,
        })
    }
}

/// A duration as written in the spreadsheet, resolved against the sequence frame rate.
#[derive(Clone, Debug, PartialEq)]
pub enum TimeSpec {
    Ticks(Tick),
    Frames(i64),
    Timecode(String),
}

/// Longest accepted duration (24 hours).
const MAX_SECONDS: f64 = 86_400.0;

impl TimeSpec {
    pub fn parse(s: &str) -> Option<TimeSpec> {
        let t = s.trim();
        if t.is_empty() {
            return None;
        }
        let lower = t.to_lowercase();
        if let Some(n) = lower.strip_suffix('s').or_else(|| lower.strip_suffix("sec")) {
            let v: f64 = n.trim().replace(',', ".").parse().ok()?;
            return (v.is_finite() && v.abs() <= MAX_SECONDS).then(|| TimeSpec::Ticks(Tick::from_seconds_f64(v)));
        }
        if let Some(n) = lower.strip_suffix('f').or_else(|| lower.strip_suffix("fr")) {
            let v: i64 = n.trim().parse().ok()?;
            return (v.unsigned_abs() <= 86_400 * 240).then_some(TimeSpec::Frames(v));
        }
        Some(TimeSpec::Timecode(t.to_string()))
    }

    /// The duration at `rate`; `None` if it is not a valid timecode or out of range.
    pub fn resolve(&self, rate: FrameRate) -> Option<Tick> {
        let fd = rate.frame_duration().0;
        if fd <= 0 {
            return None;
        }
        let frames = match self {
            TimeSpec::Ticks(t) => return Some(rate.snap_nearest(*t)),
            TimeSpec::Frames(f) => *f,
            TimeSpec::Timecode(s) => parse_timecode(s, rate, false, 0).ok()?,
        };
        let t = fd.checked_mul(frames)?;
        (t.unsigned_abs() as f64 / cfa4_time::TICKS_PER_SECOND as f64 <= MAX_SECONDS).then_some(Tick(t))
    }
}

/// Space before a block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gap {
    Units(f64),
    Px(f64),
}

/// A head–body–tail block.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub style: String,
    /// Horizontal spine offset from the frame centre (1080-high pixels).
    pub spine: f64,
    pub gap_before: Gap,
    pub head: Option<String>,
    pub body: Vec<String>,
    /// Tails with the index of the body cell they stand beside (row by row).
    pub tails: Vec<(usize, String)>,
    /// Pictures shown in place of a body (a body cell holding only `{{Pic …}}` tags).
    pub pictures: Vec<Picture>,
    /// 1-based row of the block's first cell.
    pub row: usize,
}

/// A picture tag: `{{Pic file}}` or `{{Pic file 120}}` (height in pixels of a 1080-high frame;
/// also `120px`). `Kép` and `Logo` work as the tag name too.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    /// As written: a file name relative to the spreadsheet's folder, or an absolute path.
    pub file: String,
    pub height: Option<f64>,
}

const PIC_TAGS: [&str; 4] = ["pic", "picture", "kép", "logo"];

/// Take the picture tags out of a cell: the rest of the cell and the pictures, in order.
pub fn take_pictures(cell: &str) -> (String, Vec<Picture>) {
    let mut out = String::with_capacity(cell.len());
    let mut pics = Vec::new();
    let mut rest = cell;
    while let Some(i) = rest.find("{{") {
        let after = rest.get(i + 2..).unwrap_or("");
        let Some(j) = after.find("}}") else { break };
        let tag = after.get(..j).unwrap_or("").trim();
        let (word, arg) = tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
        let lower = word.to_lowercase();
        if PIC_TAGS.contains(&lower.as_str()) && !arg.trim().is_empty() && pics.len() < 64 {
            let arg = arg.trim();
            let (file, height) = match arg.rsplit_once(char::is_whitespace) {
                Some((f, h)) => {
                    let n = h.trim().to_lowercase();
                    match n.strip_suffix("px").unwrap_or(&n).replace(',', ".").parse::<f64>() {
                        Ok(v) if v.is_finite() && v > 0.0 && v <= 10_000.0 => (f.trim(), Some(v)),
                        _ => (arg, None),
                    }
                }
                None => (arg, None),
            };
            out.push_str(rest.get(..i).unwrap_or(""));
            pics.push(Picture { file: file.to_string(), height });
        } else {
            out.push_str(rest.get(..i + 2 + j + 2).unwrap_or(""));
        }
        rest = after.get(j + 2..).unwrap_or("");
    }
    out.push_str(rest);
    (out, pics)
}

/// A page of blocks.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub style: String,
    pub runtime: Option<TimeSpec>,
    pub gap_after: Option<TimeSpec>,
    pub blocks: Vec<Block>,
    pub row: usize,
}

/// A read credits spreadsheet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Credits {
    pub pages: Vec<Page>,
    pub warnings: Vec<Warning>,
}

/// Most pages and blocks a sheet may hold.
pub const MAX_PAGES: usize = 1_000;
pub const MAX_BLOCKS: usize = 50_000;

/// Replace inline markup: `{{Blank}}` becomes an empty line; other `{{…}}` tags (letter styles,
/// videos, and pictures not standing alone in a body cell) are removed. Returns the text and whether anything was removed.
pub fn clean_markup(cell: &str) -> (String, bool) {
    let mut out = String::with_capacity(cell.len());
    let mut rest = cell;
    let mut dropped = false;
    while let Some(i) = rest.find("{{") {
        out.push_str(rest.get(..i).unwrap_or(""));
        let after = rest.get(i + 2..).unwrap_or("");
        let Some(j) = after.find("}}") else {
            out.push_str(rest.get(i..).unwrap_or(""));
            rest = "";
            break;
        };
        let tag = after.get(..j).unwrap_or("").trim();
        if !tag.eq_ignore_ascii_case("blank") {
            dropped = true;
        }
        rest = after.get(j + 2..).unwrap_or("");
    }
    out.push_str(rest);
    (out, dropped)
}

fn parse_gap(s: &str) -> Option<Gap> {
    let t = s.trim().to_lowercase().replace(',', ".");
    let (num, px) = match t.strip_suffix("px") {
        Some(n) => (n.trim().to_string(), true),
        None => (t, false),
    };
    let v: f64 = num.parse().ok()?;
    if !v.is_finite() || !(0.0..=100_000.0).contains(&v) {
        return None;
    }
    Some(if px { Gap::Px(v) } else { Gap::Units(v) })
}

fn parse_spine(s: &str) -> Option<f64> {
    let t = s.trim().to_lowercase().replace(',', ".");
    let n = t.strip_suffix("px").unwrap_or(&t).trim();
    let v: f64 = n.parse().ok()?;
    (v.is_finite() && v.abs() <= 100_000.0).then_some(v)
}

/// Interpret a table (rows of cells) as credits.
pub fn read_sheet(rows: &[Vec<String>], default_page: &str, default_content: &str) -> Result<Credits, CreditsError> {
    let header = rows.iter().take(50).position(|r| r.iter().any(|c| Column::from_header(c) == Some(Column::Body))).ok_or(CreditsError::NoHeader)?;
    let cols: Vec<Option<Column>> = rows.get(header).map(|r| r.iter().map(|c| Column::from_header(c)).collect()).unwrap_or_default();

    let mut out = Credits::default();
    let warn = |out: &mut Credits, row: usize, m: String| {
        if out.warnings.len() < 1_000 {
            out.warnings.push(Warning { row, message: m });
        }
    };
    let mut warned_markup = false;
    let mut page = Page { style: default_page.to_string(), runtime: None, gap_after: None, blocks: vec![], row: header + 2 };
    let mut style = default_content.to_string();
    let mut spine = 0.0;
    let mut pending_units = 0.0f64;
    let mut explicit_gap: Option<Gap> = None;
    let mut open = false; // the last block still takes body cells
    let mut nblocks = 0usize;

    for (i, r) in rows.iter().enumerate().skip(header + 1) {
        let rowno = i + 1;
        let raw_body = cols.iter().position(|k| *k == Some(Column::Body)).and_then(|j| r.get(j)).map(String::as_str).unwrap_or("");
        // a body cell of only picture tags is a picture block
        let (pic_rest, pics) = take_pictures(raw_body);
        let pics = if pic_rest.trim().is_empty() { pics } else { Vec::new() };
        let pic_only = !pics.is_empty();
        let mut cell = |c: Column| -> String {
            if c == Column::Body && pic_only {
                return String::new();
            }
            let raw = cols.iter().position(|k| *k == Some(c)).and_then(|j| r.get(j)).map(String::as_str).unwrap_or("");
            let (t, dropped) = clean_markup(raw);
            if dropped && !warned_markup {
                warned_markup = true;
                warn(
                    &mut out,
                    rowno,
                    "letter styles and videos in `{{…}}` are not supported yet, and a picture needs a body cell of its own; the tags were removed".into(),
                );
            }
            t
        };
        let (head, body, tail) = (cell(Column::Head), cell(Column::Body), cell(Column::Tail));
        let (gap, cstyle, spine_s) = (cell(Column::Gap), cell(Column::Style), cell(Column::Spine));
        let (pstyle, runtime, pgap) = (cell(Column::Page), cell(Column::Runtime), cell(Column::PageGap));
        let body_blank = raw_body.trim().eq_ignore_ascii_case("{{blank}}");

        // a new page
        if !pstyle.trim().is_empty() {
            if !page.blocks.is_empty() {
                if out.pages.len() >= MAX_PAGES {
                    return Err(CreditsError::TooLarge(format!("more than {MAX_PAGES} pages")));
                }
                let next = Page { style: pstyle.trim().to_string(), runtime: None, gap_after: None, blocks: vec![], row: rowno };
                out.pages.push(std::mem::replace(&mut page, next));
            } else {
                page.style = pstyle.trim().to_string();
                page.row = rowno;
            }
            open = false;
            pending_units = 0.0;
            explicit_gap = None;
        }
        if !runtime.trim().is_empty() {
            match TimeSpec::parse(&runtime) {
                Some(t) => page.runtime = Some(t),
                None => warn(&mut out, rowno, format!("page runtime `{}` is not a duration", runtime.trim())),
            }
        }
        if !pgap.trim().is_empty() {
            match TimeSpec::parse(&pgap) {
                Some(t) => page.gap_after = Some(t),
                None => warn(&mut out, rowno, format!("page gap `{}` is not a duration", pgap.trim())),
            }
        }
        if !cstyle.trim().is_empty() {
            style = cstyle.trim().to_string();
            open = false;
        }
        if !spine_s.trim().is_empty() {
            match parse_spine(&spine_s) {
                Some(v) => spine = v,
                None => warn(&mut out, rowno, format!("spine position `{}` is not supported (a pixel offset is)", spine_s.trim())),
            }
            open = false;
        }
        if !gap.trim().is_empty() {
            match parse_gap(&gap) {
                Some(g) => explicit_gap = Some(g),
                None => warn(&mut out, rowno, format!("vertical gap `{}` is not a number", gap.trim())),
            }
            open = false;
        }

        let has_head = !head.trim().is_empty();
        let has_pics = !pics.is_empty();
        let has_body = (!body.trim().is_empty() || body_blank) && !has_pics;
        let has_tail = !tail.trim().is_empty();
        if !has_head && !has_body && !has_tail && !has_pics {
            // an empty row (or one with only settings): ends the block; empty rows add a unit
            let settings_only = [&gap, &cstyle, &spine_s, &pstyle, &runtime, &pgap].iter().any(|v| !v.trim().is_empty());
            if !settings_only {
                pending_units += 1.0;
            }
            open = false;
            continue;
        }
        let body_text = if body_blank { String::new() } else { body.trim_end_matches(['\r', '\n']).to_string() };
        if has_head || has_pics || !open || page.blocks.is_empty() {
            nblocks = nblocks.saturating_add(1);
            if nblocks > MAX_BLOCKS {
                return Err(CreditsError::TooLarge(format!("more than {MAX_BLOCKS} blocks")));
            }
            let gap_before = explicit_gap.take().unwrap_or(Gap::Units(pending_units));
            page.blocks.push(Block {
                style: style.clone(),
                spine,
                gap_before,
                head: has_head.then(|| head.trim().to_string()),
                body: if has_body { vec![body_text] } else { vec![] },
                tails: vec![],
                pictures: pics,
                row: rowno,
            });
            pending_units = 0.0;
            // a picture block takes no further body cells
            open = !has_pics;
        } else if has_body && let Some(b) = page.blocks.last_mut() {
            b.body.push(body_text);
        }
        if has_tail && let Some(b) = page.blocks.last_mut() {
            let at = b.body.len().saturating_sub(1);
            b.tails.push((at, tail.trim().to_string()));
        }
    }
    if !page.blocks.is_empty() || out.pages.is_empty() {
        out.pages.push(page);
    }
    out.pages.retain(|p| !p.blocks.is_empty());
    if out.pages.is_empty() {
        return Err(CreditsError::Empty);
    }
    Ok(out)
}
