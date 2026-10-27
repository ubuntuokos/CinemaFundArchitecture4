//! Lay out read credits into graphic-clip layers: one graphic clip per page.
//!
//! Every piece of text is an ordinary CFA4 text layer (`graphic_text`), so a laid-out page
//! can be edited afterwards in Essential Graphics like any title. Scroll pages use the graphic's
//! Roll (start and end off screen); crawl pages put their blocks side by side on one line and
//! use Crawl Left; card pages are centred vertically and fade with opacity keyframes. Text sizes come from the caller's [`Measure`] (the engine uses the real text
//! engine), so this crate stays free of fonts and rendering.

use cfa4_geom::Vec2;
use cfa4_project::graphic::{new_shape_layer, new_text_layer};
use cfa4_project::graphic_design::{CharStyle, GraphicMeta, Roll, RollMode};
use cfa4_project::gtemplate::ensure_uids;
use cfa4_project::{EffectInstance, Interpolation, Keyframe, Param, ParamValue};
use cfa4_time::{FrameRate, Tick};

use crate::CreditsError;
use crate::sheet::{Block, Credits, Gap, Page, Picture, Warning};
use crate::style::{Align, Behaviour, BodyLayout, ContentStyle, LetterStyle, Orientation, Styling, parse_color};

/// Measures text layers.
pub trait Measure {
    /// Line-box bounds `[x0, y0, x1, y1]` of a text layer placed at the origin, in layer pixels
    /// (point text: the origin is the alignment point on the first baseline; paragraph text: the
    /// top-left corner of its box). `None` if it cannot be measured.
    fn bounds(&self, layer: &EffectInstance) -> Option<[f64; 4]>;

    /// Pixel size of a picture named in the sheet (`{{Pic file}}`), or `None` if it cannot be
    /// found or read.
    fn picture_size(&self, _file: &str) -> Option<(f64, f64)> {
        None
    }

    /// Whether a background's picture or video file can be found (default: assume it can).
    fn file_exists(&self, _file: &str) -> bool {
        true
    }
}

/// A picture to show with a page, in graphic-canvas pixels where the page was laid out (before
/// any roll): the engine places it as its own clip that moves with the page.
#[derive(Clone, Debug, PartialEq)]
pub struct PicturePlace {
    pub file: String,
    pub center: (f64, f64),
    pub size: (f64, f64),
    pub row: usize,
    /// A video, playing from its start.
    pub video: bool,
    /// When it comes in on a card with a staggered entrance (zero otherwise).
    pub delay: Tick,
}

/// A picture placeholder's file and size on screen.
#[derive(Clone, Debug)]
struct PicInfo {
    file: String,
    w: f64,
    h: f64,
    row: usize,
    video: bool,
}

/// Default picture height (1080-high pixels).
pub const DEFAULT_PICTURE_HEIGHT: f64 = 150.0;

/// The frame the credits are laid out for.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rate: FrameRate,
}

/// One laid-out page: the layers and settings of one graphic clip.
#[derive(Clone, Debug)]
pub struct PageOut {
    pub name: String,
    pub behaviour: Behaviour,
    pub layers: Vec<EffectInstance>,
    /// `(uid, key)` of every layer: the key stays the same across refreshes while the layer's
    /// block is unchanged, so manual edits can be carried over.
    pub keys: Vec<(u64, String)>,
    /// Spreadsheet row (1-based) of each layer's block, in the order of `layers`.
    pub layer_rows: Vec<usize>,
    pub meta: GraphicMeta,
    pub duration: Tick,
    /// Empty time after the page; negative overlaps the next page.
    pub gap_after: Tick,
    /// Height of the content (frame pixels).
    pub content_height: f64,
    /// Pictures that go with this page.
    pub pictures: Vec<PicturePlace>,
    /// Card fades actually used (zero for scroll pages, and at a cut with a transition).
    pub fade_in: Tick,
    pub fade_out: Tick,
    /// Crawl: length of the line (frame pixels; zero for other pages).
    pub content_width: f64,
    /// Where the content's top is on the frame before any roll (cards: centred).
    pub content_top: f64,
    /// The spines of the page's blocks (frame x, left to right; none on a crawl).
    pub spines: Vec<f64>,
    /// What is behind the page, if anything.
    pub background: Option<crate::style::Background>,
    /// The transition into the next page (video transition id and length), when it has one
    /// and the pages touch.
    pub transition: Option<(String, Tick)>,
    /// Length of the transition from the previous page into this one (zero for none).
    pub transition_in: Tick,
    /// A card's entrance: where its content starts relative to where it ends (frame pixels) and
    /// how long the move takes; pictures move the same way from their own `delay`.
    pub entrance: Option<(f64, f64, Tick)>,
}

/// Laid-out credits.
#[derive(Clone, Debug)]
pub struct Laid {
    pub pages: Vec<PageOut>,
    pub warnings: Vec<Warning>,
}

/// The most a runtime may stretch a page's gaps.
const MAX_GAP_SCALE: f64 = 20.0;

/// Most text layers on one page.
pub const MAX_LAYERS_PER_PAGE: usize = 5_000;

struct Piece {
    layer: EffectInstance,
    /// Local bounds of the layer.
    b: [f64; 4],
    /// Origin x in frame pixels.
    x: f64,
    /// Top of the line box in content pixels.
    top: f64,
    /// A picture's placeholder: the file and its size on screen.
    pic: Option<Box<PicInfo>>,
    /// Stable key of the layer: its block's content and its own name (see [`block_key`]).
    key: String,
    /// Spreadsheet row (1-based) of the layer's block.
    row: usize,
}

impl Piece {
    fn height(&self) -> f64 {
        (self.b[3] - self.b[1]).max(0.0)
    }
    fn right(&self) -> f64 {
        self.x + self.b[2]
    }
}

fn set(e: &mut EffectInstance, id: &str, v: ParamValue) {
    e.params.insert(id.to_string(), Param::new(v));
}

struct Ctx<'a> {
    s: f64,
    /// Frame width.
    w: f64,
    missing_pics: Vec<String>,
    measure: &'a dyn Measure,
    warnings: Vec<Warning>,
    bad_colors: Vec<String>,
    /// For each page, for each block: how many identical blocks come before it in the credits
    /// (repeated blocks get keys of their own).
    occurrences: Vec<Vec<usize>>,
    /// Inline letter styles by lower-case name (Hungarian built-in names too), scaled.
    inline: Vec<(String, CharStyle)>,
    unknown_inline: Vec<String>,
}

impl Ctx<'_> {
    fn warn(&mut self, row: usize, m: String) {
        if self.warnings.len() < 1_000 {
            self.warnings.push(Warning { row, message: m });
        }
    }

    fn text(
        &mut self,
        row: usize,
        name: &str,
        text: &str,
        ls: &LetterStyle,
        align: Align,
        wrap: Option<f64>,
    ) -> Result<(EffectInstance, [f64; 4]), CreditsError> {
        let s = self.s;
        let size = finite_or(ls.size, 40.0).clamp(1.0, 2_000.0) * s;
        let st = crate::markup::styled(text, |n| {
            let n = n.trim().to_lowercase();
            self.inline.iter().find(|(k, _)| *k == n).map(|(_, c)| c.clone())
        });
        for u in st.unknown {
            if !self.unknown_inline.contains(&u) {
                self.warn(row, format!("letter style `{u}` does not exist; the text keeps the block's style"));
                self.unknown_inline.push(u);
            }
        }
        let mut e = new_text_layer(&st.text, Vec2::new(0.0, 0.0), size);
        if !st.runs.is_empty() {
            e.layer.get_or_insert_with(Default::default).runs = st.runs;
        }
        set(&mut e, "name", ParamValue::Text(name.into()));
        set(&mut e, "font", ParamValue::Text(ls.font.clone()));
        set(&mut e, "font_style", ParamValue::Text(ls.font_style.clone()));
        let color = match parse_color(&ls.color) {
            Some(c) => c,
            None => {
                if !self.bad_colors.contains(&ls.color) {
                    self.bad_colors.push(ls.color.clone());
                    self.warn(row, format!("colour `{}` is not #rrggbb; white is used", ls.color));
                }
                [1.0, 1.0, 1.0, 1.0]
            }
        };
        set(&mut e, "fill_color", ParamValue::Color(color));
        set(&mut e, "tracking", ParamValue::Float(finite_or(ls.tracking, 0.0).clamp(-1_000.0, 10_000.0)));
        set(&mut e, "leading", ParamValue::Float(finite_or(ls.leading, 0.0).clamp(-1_000.0, 10_000.0) * s));
        set(&mut e, "caps", ParamValue::Choice(u32::from(ls.all_caps)));
        set(&mut e, "align", ParamValue::Choice(align.choice()));
        if let Some(w) = wrap {
            set(&mut e, "box_width", ParamValue::Float(w.max(1.0)));
        }
        let b = self.measure.bounds(&e).filter(|b| b.iter().all(|v| v.is_finite()) && b[2] >= b[0] && b[3] >= b[1]);
        let Some(b) = b else {
            return Err(CreditsError::Layout(format!("row {row}: the text could not be measured (font `{}` {})", ls.font, ls.font_style)));
        };
        Ok((e, b))
    }
}

fn finite_or(v: f64, d: f64) -> f64 {
    if v.is_finite() { v } else { d }
}

/// Where a point-text origin sits in a box of width `w` starting at `left`.
fn origin_in_box(left: f64, w: f64, a: Align) -> f64 {
    match a {
        Align::Left => left,
        Align::Center => left + w / 2.0,
        Align::Right => left + w,
    }
}

/// Left edge of something `w` wide anchored at `x` with alignment `a`.
fn left_of(x: f64, w: f64, a: Align) -> f64 {
    match a {
        Align::Left => x,
        Align::Center => x - w / 2.0,
        Align::Right => x - w,
    }
}

/// Lay out the body cells with their top at `top`, anchored at `x`. Returns the pieces.
fn lay_body(ctx: &mut Ctx, b: &Block, cs: &ContentStyle, align: Align, x: f64, top: f64) -> Result<Vec<Piece>, CreditsError> {
    let s = ctx.s;
    let mut out = Vec::new();
    if b.body.is_empty() {
        return Ok(out);
    }
    if b.portraits.iter().any(Option::is_some) {
        return lay_portraits(ctx, b, cs, align, x, top);
    }
    match cs.layout {
        BodyLayout::Grid => {
            let n = (cs.columns.clamp(1, 16) as usize).min(b.body.len()).max(1);
            if n == 1 {
                let (e, bb) = ctx.text(b.row, "Body", &b.body.join("\n"), &cs.body, align, None)?;
                out.push(Piece { row: 0, key: String::new(), layer: e, b: bb, x, top, pic: None });
                return Ok(out);
            }
            // pad every cell of a row to the row's line count so the columns stay in step
            let rows = b.body.chunks(n);
            let mut cols: Vec<Vec<String>> = vec![Vec::new(); n];
            for row in rows {
                let lines = row.iter().map(|c| c.split('\n').count()).max().unwrap_or(1);
                for j in 0..n {
                    let cell = row.get(j).map(String::as_str).unwrap_or("");
                    let have = cell.split('\n').count();
                    let mut c = cell.to_string();
                    for _ in have..lines {
                        c.push('\n');
                    }
                    if let Some(col) = cols.get_mut(j) {
                        col.push(c);
                    }
                }
            }
            let mut measured = Vec::with_capacity(n);
            for (j, col) in cols.iter().enumerate() {
                let (e, bb) = ctx.text(b.row, &format!("Body {}", j + 1), &col.join("\n"), &cs.body, align, None)?;
                measured.push((e, bb));
            }
            let gap = finite_or(cs.column_gap, 0.0).max(0.0) * s;
            // equal column widths look tidier than ragged ones
            let colw = measured.iter().map(|(_, bb)| bb[2] - bb[0]).fold(0.0f64, f64::max);
            let total = colw * n as f64 + gap * (n as f64 - 1.0);
            let mut left = left_of(x, total, align);
            for (e, bb) in measured {
                out.push(Piece { row: 0, key: String::new(), layer: e, b: bb, x: origin_in_box(left, colw, align), top, pic: None });
                left += colw + gap;
            }
        }
        BodyLayout::Flow => {
            let wrap = finite_or(cs.wrap_width, 1200.0).clamp(10.0, 100_000.0) * s;
            let text = b.body.iter().map(|c| c.replace('\n', " ")).collect::<Vec<_>>().join(&cs.flow_separator);
            let (e, bb) = ctx.text(b.row, "Body", &text, &cs.body, align, Some(wrap))?;
            out.push(Piece { row: 0, key: String::new(), layer: e, b: bb, x: left_of(x, wrap, align), top, pic: None });
        }
        BodyLayout::Paragraphs => {
            let wrap = finite_or(cs.wrap_width, 1200.0).clamp(10.0, 100_000.0) * s;
            let gap = finite_or(cs.paragraph_gap, 0.0).max(0.0) * s;
            let mut y = top;
            for (k, cell) in b.body.iter().enumerate() {
                let (e, bb) = ctx.text(b.row, &format!("Paragraph {}", k + 1), cell, &cs.body, align, Some(wrap))?;
                let h = bb[3] - bb[1];
                out.push(Piece { row: 0, key: String::new(), layer: e, b: bb, x: left_of(x, wrap, align), top: y, pic: None });
                y += h + gap;
            }
        }
    }
    Ok(out)
}

fn bottom(pieces: &[Piece]) -> Option<f64> {
    pieces.iter().map(|p| p.top + p.height()).reduce(f64::max)
}

/// A row of pictures with its top at `top`: placeholder pieces (transparent rectangles, so the
/// page's size and roll include them). `anchor_x` and `align` place the row like text.
/// A picture's size on screen (`default_h` high unless the tag gives a height, never wider than
/// the frame), or `None` with a warning when it cannot be read.
fn size_picture(ctx: &mut Ctx, row: usize, p: &Picture, default_h: f64) -> Option<(f64, f64)> {
    let s = ctx.s;
    let Some((pw, ph)) = ctx.measure.picture_size(&p.file).filter(|(w, h)| w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0) else {
        if !ctx.missing_pics.contains(&p.file) {
            ctx.missing_pics.push(p.file.clone());
            let what = if p.video { "video" } else { "picture" };
            ctx.warn(row, format!("{what} `{}` was not found or cannot be read; it is left out", p.file));
        }
        return None;
    };
    let mut h = finite_or(p.height.unwrap_or(default_h), default_h).clamp(1.0, 10_000.0) * s;
    let mut w = h * pw / ph;
    let max_w = ctx.w * 0.95;
    if w > max_w {
        h *= max_w / w;
        w = max_w;
    }
    Some((w, h))
}

/// A transparent placeholder piece for a picture centred at `cx` with its top at `top`.
fn picture_piece(p: &Picture, k: usize, w: f64, h: f64, cx: f64, top: f64, row: usize) -> Piece {
    let mut e = new_shape_layer(0, Vec2::new(0.0, 0.0), Vec2::new(w, h), vec![]);
    let what = if p.video { "Video" } else { "Picture" };
    set(&mut e, "name", ParamValue::Text(format!("{what} {}: {}", k + 1, p.file)));
    set(&mut e, "fill", ParamValue::Bool(false));
    Piece {
        row,
        key: String::new(),
        layer: e,
        b: [-w / 2.0, -h / 2.0, w / 2.0, h / 2.0],
        x: cx,
        top,
        pic: Some(Box::new(PicInfo { file: p.file.clone(), w, h, row, video: p.video })),
    }
}

/// A row of pictures with its top at `top`: placeholder pieces (transparent rectangles, so the
/// page's size and roll include them). `anchor_x` and `align` place the row like text.
fn lay_pictures(ctx: &mut Ctx, b: &Block, cs: &ContentStyle, anchor_x: f64, align: Align, top: f64) -> Vec<Piece> {
    let s = ctx.s;
    let mut sized = Vec::new();
    for p in &b.pictures {
        if let Some((w, h)) = size_picture(ctx, b.row, p, DEFAULT_PICTURE_HEIGHT) {
            sized.push((p, w, h));
        }
    }
    let gap = finite_or(cs.column_gap, 0.0).max(0.0) * s;
    let total: f64 = sized.iter().map(|(_, w, _)| *w).sum::<f64>() + gap * (sized.len().saturating_sub(1)) as f64;
    let row_h = sized.iter().map(|(_, _, h)| *h).fold(0.0f64, f64::max);
    let mut left = left_of(anchor_x, total, align);
    let mut out = Vec::new();
    for (k, (p, w, h)) in sized.into_iter().enumerate() {
        out.push(picture_piece(p, k, w, h, left + w / 2.0, top + (row_h - h) / 2.0, b.row));
        left += w + gap;
    }
    out
}

/// Names with their portraits, `columns` to a row: each portrait above its name (or left of
/// it, `portraitSide: left`), the grid placed like text at `x` with `align`.
fn lay_portraits(ctx: &mut Ctx, b: &Block, cs: &ContentStyle, align: Align, x: f64, top: f64) -> Result<Vec<Piece>, CreditsError> {
    let s = ctx.s;
    let left_side = cs.portrait_side == crate::style::PortraitSide::Left;
    let pgap = finite_or(cs.portrait_gap, 0.0).clamp(0.0, 10_000.0) * s;
    let ph_default = finite_or(cs.portrait_height, 220.0).clamp(1.0, 10_000.0);
    // each cell: its name, its portrait (if readable)
    let mut cells = Vec::with_capacity(b.body.len());
    for (i, cell) in b.body.iter().enumerate() {
        let row = b.body_rows.get(i).copied().unwrap_or(b.row);
        let (e, bb) = ctx.text(row, &format!("Name {}", i + 1), cell, &cs.body, if left_side { Align::Left } else { Align::Center }, None)?;
        let pic = b.portraits.get(i).cloned().flatten().and_then(|p| size_picture(ctx, row, &p, ph_default).map(|(w, h)| (p, w, h)));
        cells.push((e, bb, pic, row));
    }
    let n = (cs.columns.clamp(1, 16) as usize).min(cells.len()).max(1);
    let gap = finite_or(cs.column_gap, 0.0).max(0.0) * s;
    let row_gap = finite_or(cs.paragraph_gap, 0.0).max(0.0) * s;
    let pic_w = cells.iter().filter_map(|c| c.2.as_ref().map(|p| p.1)).fold(0.0f64, f64::max);
    let text_w = cells.iter().map(|c| (c.1[2] - c.1[0]).max(0.0)).fold(0.0f64, f64::max);
    let col_w = if left_side { pic_w + if pic_w > 0.0 { pgap } else { 0.0 } + text_w } else { pic_w.max(text_w) };
    let total = col_w * n as f64 + gap * (n as f64 - 1.0);
    let left = left_of(x, total, align);
    let mut out = Vec::new();
    let mut y = top;
    let mut k = 0usize;
    for chunk in cells.chunks(n) {
        let row_ph = chunk.iter().filter_map(|c| c.2.as_ref().map(|p| p.2)).fold(0.0f64, f64::max);
        let row_th = chunk.iter().map(|c| (c.1[3] - c.1[1]).max(0.0)).fold(0.0f64, f64::max);
        let row_h = if left_side { row_ph.max(row_th) } else { row_ph + if row_ph > 0.0 { pgap } else { 0.0 } + row_th };
        for (j, (e, bb, pic, row)) in chunk.iter().enumerate() {
            let col_left = left + j as f64 * (col_w + gap);
            let th = (bb[3] - bb[1]).max(0.0);
            if left_side {
                if let Some((p, w, h)) = pic {
                    out.push(picture_piece(p, k, *w, *h, col_left + pic_w / 2.0, y + (row_h - h) / 2.0, *row));
                    k += 1;
                }
                let tx = col_left + if pic_w > 0.0 { pic_w + pgap } else { 0.0 };
                out.push(Piece { row: *row, key: String::new(), layer: e.clone(), b: *bb, x: tx, top: y + (row_h - th) / 2.0, pic: None });
            } else {
                let cx = col_left + col_w / 2.0;
                if let Some((p, w, h)) = pic {
                    // portraits stand on one line at the bottom of the row's picture band
                    out.push(picture_piece(p, k, *w, *h, cx, y + row_ph - h, *row));
                    k += 1;
                }
                let ty = y + if row_ph > 0.0 { row_ph + pgap } else { 0.0 };
                out.push(Piece { row: *row, key: String::new(), layer: e.clone(), b: *bb, x: cx, top: ty, pic: None });
            }
        }
        y += row_h + row_gap;
    }
    Ok(out)
}

/// The longest leader text on a line.
const MAX_LEADER: usize = 2_000;

/// Lay out one block with its top at `top`. `head_w`: the width the head column has (heads of a
/// harmonised style); `None` = the head's own.
fn lay_block(ctx: &mut Ctx, b: &Block, cs: &ContentStyle, spine_x: f64, top: f64, head_w: Option<f64>) -> Result<(Vec<Piece>, f64), CreditsError> {
    let s = ctx.s;
    let mut pieces = Vec::new();
    let head_gap = finite_or(cs.head_gap, 0.0).max(0.0) * s;
    let tail_gap = finite_or(cs.tail_gap, 0.0).max(0.0) * s;
    match cs.orientation {
        Orientation::Vertical => {
            let mut y = top;
            if let Some(h) = &b.head {
                let (e, bb) = ctx.text(b.row, "Head", h, &cs.head, cs.align, None)?;
                let p = Piece { row: 0, key: String::new(), layer: e, b: bb, x: spine_x, top: y, pic: None };
                y += p.height() + head_gap;
                pieces.push(p);
            }
            let body = if b.pictures.is_empty() { lay_body(ctx, b, cs, cs.align, spine_x, y)? } else { lay_pictures(ctx, b, cs, spine_x, cs.align, y) };
            if let Some(bt) = bottom(&body) {
                y = bt;
            }
            pieces.extend(body);
            if !b.tails.is_empty() {
                let text = b.tails.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join("\n");
                let (e, bb) = ctx.text(b.row, "Tail", &text, &cs.tail, cs.align, None)?;
                pieces.push(Piece { row: 0, key: String::new(), layer: e, b: bb, x: spine_x, top: y + tail_gap, pic: None });
            }
        }
        Orientation::Horizontal => {
            let body = if b.pictures.is_empty() {
                lay_body(ctx, b, cs, Align::Left, spine_x + head_gap / 2.0, top)?
            } else {
                lay_pictures(ctx, b, cs, spine_x + head_gap / 2.0, Align::Left, top)
            };
            // head and tails stand on the body's baselines (beside pictures: on their middle)
            let first_baseline = if b.pictures.is_empty() { body.first().map(|p| p.top - p.b[1]) } else { bottom(&body).map(|bt| (top + bt) / 2.0) };
            let at_baseline = |bb: [f64; 4], baseline: Option<f64>| baseline.map(|bl| bl + bb[1]).unwrap_or(top);
            if let Some(h) = &b.head {
                let (e, bb) = ctx.text(b.row, "Head", h, &cs.head, cs.head_align, None)?;
                let own = (bb[2] - bb[0]).max(0.0);
                let col = head_w.unwrap_or(own).max(own);
                // the column ends at the gap before the body
                let col_right = spine_x - head_gap / 2.0;
                let x = origin_in_box(col_right - col, col, cs.head_align);
                let head_top = at_baseline(bb, first_baseline);
                let head_right = x + bb[2];
                pieces.push(Piece { row: 0, key: String::new(), layer: e, b: bb, x, top: head_top, pic: None });
                // dots from the head to the body, on the head's first baseline
                let unit = cs.leader.as_str();
                if !unit.trim().is_empty() {
                    let lg = finite_or(cs.leader_gap, 0.0).clamp(0.0, 10_000.0) * s;
                    let (x0, x1) = (head_right + lg, spine_x + head_gap / 2.0 - lg);
                    let (_, ub) = ctx.text(b.row, "Leader", &unit.repeat(10), &cs.head, Align::Left, None)?;
                    let uw = (ub[2] - ub[0]) / 10.0;
                    if uw > 0.0 && x1 > x0 + uw {
                        let n = (((x1 - x0) / uw).floor() as usize).min(MAX_LEADER);
                        let (le, lb) = ctx.text(b.row, "Leader", &unit.repeat(n), &cs.head, Align::Right, None)?;
                        let baseline = head_top - bb[1];
                        pieces.push(Piece { row: 0, key: String::new(), layer: le, b: lb, x: x1, top: baseline + lb[1], pic: None });
                    }
                }
            }
            let right = body.iter().map(Piece::right).reduce(f64::max).unwrap_or(spine_x + head_gap / 2.0);
            let single_column = cs.layout == BodyLayout::Grid && body.len() == 1 && b.pictures.is_empty();
            pieces.extend(body);
            if !b.tails.is_empty() {
                let x = right + tail_gap;
                if single_column {
                    // line advance of the body: one line versus two
                    let (_, one) = ctx.text(b.row, "Body", "X", &cs.body, Align::Left, None)?;
                    let (_, two) = ctx.text(b.row, "Body", "X\nX", &cs.body, Align::Left, None)?;
                    let advance = (two[3] - two[1]) - (one[3] - one[1]);
                    for (k, (at, t)) in b.tails.iter().enumerate() {
                        let line: usize = b.body.iter().take(*at).map(|c| c.split('\n').count()).sum();
                        let bl = first_baseline.map(|b0| b0 + advance * line as f64);
                        let (e, bb) = ctx.text(b.row, &format!("Tail {}", k + 1), t, &cs.tail, Align::Left, None)?;
                        pieces.push(Piece { row: 0, key: String::new(), layer: e, b: bb, x, top: at_baseline(bb, bl), pic: None });
                    }
                } else {
                    let text = b.tails.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join("\n");
                    let (e, bb) = ctx.text(b.row, "Tail", &text, &cs.tail, Align::Left, None)?;
                    pieces.push(Piece { row: 0, key: String::new(), layer: e, b: bb, x, top: at_baseline(bb, first_baseline), pic: None });
                }
            }
        }
    }
    // a head or tail placed on a baseline may rise above the block's top
    if let Some(min_top) = pieces.iter().map(|p| p.top).reduce(f64::min).filter(|m| *m < top) {
        let shift = top - min_top;
        for p in &mut pieces {
            p.top += shift;
        }
    }
    let h = bottom(&pieces).map(|bt| (bt - top).max(0.0)).unwrap_or(0.0);
    Ok((pieces, h))
}

/// A card layer's keyframes: it comes in at `delay` (fading in over `fin`, and with `motion`
/// sliding from `(dx, dy)` away over its frames to `at`) and fades out over `fout` at the end.
pub fn card_keys(e: &mut EffectInstance, d: Tick, delay: Tick, fin: Tick, fout: Tick, motion: Option<(f64, f64, Tick)>, at: (f64, f64)) {
    let (opacity, position) = card_params(d, delay, fin, fout, motion, at);
    if let Some(o) = opacity {
        e.params.insert("opacity".into(), o);
    }
    if let Some(p) = position {
        e.params.insert("position".into(), p);
    }
}

/// The opacity (`None`: always fully shown) and position (`None`: standing still) of something
/// on a card, as [`card_keys`] gives a text layer; pictures use the same.
pub fn card_params(d: Tick, delay: Tick, fin: Tick, fout: Tick, motion: Option<(f64, f64, Tick)>, at: (f64, f64)) -> (Option<Param>, Option<Param>) {
    let kf = |t: Tick, v: ParamValue, interp: Interpolation| Keyframe { time: t, value: v, interp, out_influence: 1.0 / 3.0, in_influence: 1.0 / 3.0 };
    let opacity = if delay.0 <= 0 && motion.is_none() {
        fade_param(d, fin, fout)
    } else {
        // hidden until `delay`, then in over `fin` (at least a tick)
        let fin = Tick(fin.0.max(1));
        let out_start = Tick(d.0.saturating_sub(fout.0));
        let mut ok = vec![kf(Tick::ZERO, ParamValue::Float(0.0), if delay.0 > 0 { Interpolation::Hold } else { Interpolation::Linear })];
        if delay.0 > 0 {
            ok.push(kf(delay, ParamValue::Float(0.0), Interpolation::Linear));
        }
        let shown = Tick((delay.0.saturating_add(fin.0)).min(out_start.0).max(delay.0));
        ok.push(kf(shown, ParamValue::Float(100.0), Interpolation::Linear));
        if fout.0 > 0 && out_start > shown {
            ok.push(kf(out_start, ParamValue::Float(100.0), Interpolation::Linear));
            ok.push(kf(d, ParamValue::Float(0.0), Interpolation::Linear));
        }
        ok.dedup_by_key(|k| k.time);
        Some(Param { value: ParamValue::Float(100.0), keyframes: ok })
    };
    let position = motion.map(|(dx, dy, len)| {
        let from = ParamValue::Vec2(Vec2::new(at.0 + dx, at.1 + dy));
        let to = ParamValue::Vec2(Vec2::new(at.0, at.1));
        let mut pk = vec![kf(Tick::ZERO, from.clone(), if delay.0 > 0 { Interpolation::Hold } else { Interpolation::EaseOut })];
        if delay.0 > 0 {
            pk.push(kf(delay, from, Interpolation::EaseOut));
        }
        pk.push(kf(Tick(delay.0.saturating_add(len.0).min(d.0)), to.clone(), Interpolation::Linear));
        pk.dedup_by_key(|k| k.time);
        Param { value: to, keyframes: pk }
    });
    (opacity, position)
}

/// Opacity fading in over `fin` and out over `fout` of `d`; `None` without fades.
fn fade_param(d: Tick, fin: Tick, fout: Tick) -> Option<Param> {
    if fin.0 <= 0 && fout.0 <= 0 {
        return None;
    }
    let kf =
        |t: Tick, v: f64| Keyframe { time: t, value: ParamValue::Float(v), interp: Interpolation::Linear, out_influence: 1.0 / 3.0, in_influence: 1.0 / 3.0 };
    let mut kfs = Vec::new();
    if fin.0 > 0 {
        kfs.push(kf(Tick::ZERO, 0.0));
        kfs.push(kf(fin, 100.0));
    } else {
        kfs.push(kf(Tick::ZERO, 100.0));
    }
    if fout.0 > 0 {
        let start = Tick(d.0.saturating_sub(fout.0));
        if start.0 > kfs.last().map(|k| k.time.0).unwrap_or(0) {
            kfs.push(kf(start, 100.0));
        }
        kfs.push(kf(d, 0.0));
    }
    kfs.dedup_by_key(|k| k.time);
    Some(Param { value: ParamValue::Float(100.0), keyframes: kfs })
}

fn frames(rate: FrameRate, n: i64) -> Tick {
    Tick(rate.frame_duration().0.saturating_mul(n))
}

/// A block's identity: its style, spine and content. Unchanged blocks keep their key when rows
/// around them are added or removed.
pub fn block_key(b: &Block) -> u64 {
    let mut parts: Vec<Vec<u8>> = vec![b.style.as_bytes().to_vec(), b.spine.to_bits().to_le_bytes().to_vec()];
    parts.push(b.head.clone().unwrap_or_default().into_bytes());
    for x in &b.body {
        parts.push(x.clone().into_bytes());
    }
    for (i, t) in &b.tails {
        parts.push(format!("{i}:{t}").into_bytes());
    }
    for p in &b.pictures {
        parts.push(format!("{}@{:?}{}", p.file, p.height, if p.video { "v" } else { "" }).into_bytes());
    }
    for p in b.portraits.iter().flatten() {
        parts.push(format!("portrait {}@{:?}", p.file, p.height).into_bytes());
    }
    if b.parallel {
        parts.push(b"parallel".to_vec());
    }
    let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
    crate::doc::fnv1a(&refs)
}

/// A layer's key: its block, which repetition of that block it is (the first has none, so keys
/// stay as they were before repetitions were counted), its place in the block and its name.
fn piece_key(bk: u64, nth: usize, k: usize, name: &str) -> String {
    if nth == 0 { format!("{bk:016x}/{k}/{name}") } else { format!("{bk:016x}#{nth}/{k}/{name}") }
}

/// For each page and block: how many identical blocks come before it.
fn occurrences(credits: &Credits) -> Vec<Vec<usize>> {
    let mut seen: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();
    credits
        .pages
        .iter()
        .map(|p| {
            p.blocks
                .iter()
                .map(|b| {
                    let n = seen.entry(block_key(b)).or_insert(0);
                    let nth = *n;
                    *n += 1;
                    nth
                })
                .collect()
        })
        .collect()
}

/// Stack a page's blocks from the top; the gaps between blocks are multiplied by `gap_scale`.
/// Returns the pieces, the content height and the total of those (elastic) gaps.
fn stack(ctx: &mut Ctx, styling: &Styling, page: &Page, w: f64, index: usize, gap_scale: f64) -> Result<(Vec<Piece>, f64, f64), CreditsError> {
    let s = ctx.s;
    let unit = finite_or(styling.unit_px, 32.0).clamp(0.0, 10_000.0) * s;
    let mut pieces: Vec<Piece> = Vec::new();
    let mut y = 0.0f64;
    let mut elastic = 0.0f64;
    let mut prev_margin = 0.0f64;
    let fallback = styling.content_style(&styling.default_content_style).unwrap_or_default();
    let mut missing: Vec<String> = Vec::new();
    // parallel columns: where the current set of columns starts, how far down the finished
    // columns reach, and whether the block is in a column after the first
    let (mut group_top, mut group_bottom, mut in_parallel) = (0.0f64, 0.0f64, false);
    // the widest head of each harmonised style on the page
    let mut head_widths: Vec<(String, f64)> = Vec::new();
    for b in &page.blocks {
        let (Some(h), Some(cs)) = (&b.head, styling.content_style(&b.style)) else { continue };
        if !(cs.harmonize_heads && cs.orientation == Orientation::Horizontal) {
            continue;
        }
        let (_, bb) = ctx.text(b.row, "Head", h, &cs.head, Align::Left, None)?;
        let w = (bb[2] - bb[0]).max(0.0);
        match head_widths.iter_mut().find(|(n, _)| *n == cs.name) {
            Some((_, m)) => *m = m.max(w),
            None => head_widths.push((cs.name.clone(), w)),
        }
    }
    for (k, b) in page.blocks.iter().enumerate() {
        let cs = match styling.content_style(&b.style) {
            Some(c) => c,
            None => {
                if !missing.contains(&b.style) {
                    missing.push(b.style.clone());
                    ctx.warn(b.row, format!("content style `{}` does not exist; `{}` is used", b.style, fallback.name));
                }
                fallback.clone()
            }
        };
        let margin = finite_or(cs.margin, 0.0).max(0.0) * s;
        let spine_changed = k > 0 && page.blocks.get(k - 1).is_some_and(|p| p.spine.to_bits() != b.spine.to_bits());
        if b.parallel && k > 0 {
            // a parallel column starts beside the previous one, at the same top
            group_bottom = group_bottom.max(y);
            y = group_top;
            in_parallel = true;
        } else {
            if spine_changed {
                // the columns end: continue below the longest
                y = y.max(group_bottom);
                in_parallel = false;
            }
            if k > 0 {
                let gap = match b.gap_before {
                    Gap::Units(u) => finite_or(u, 0.0).max(0.0) * unit,
                    Gap::Px(p) => finite_or(p, 0.0).max(0.0) * s,
                };
                let g = gap.max(margin).max(prev_margin);
                // only the first column's gaps decide the page's length
                if !in_parallel {
                    elastic += g;
                }
                y += g * gap_scale;
            }
            if spine_changed || k == 0 {
                group_top = y;
                group_bottom = y;
            }
        }
        prev_margin = margin;
        let spine_x = w / 2.0 + finite_or(b.spine, 0.0) * s;
        let head_w = head_widths.iter().find(|(n, _)| *n == cs.name).map(|(_, w)| *w);
        let (mut ps_, bh) = lay_block(ctx, b, &cs, spine_x, y, head_w)?;
        let nth = ctx.occurrences.get(index).and_then(|v| v.get(k)).copied().unwrap_or(0);
        key_pieces(b, nth, &mut ps_);
        y += bh;
        pieces.extend(ps_);
        if pieces.len() > MAX_LAYERS_PER_PAGE {
            return Err(CreditsError::TooLarge(format!("page {} needs more than {MAX_LAYERS_PER_PAGE} text layers", index + 1)));
        }
    }
    Ok((pieces, y.max(group_bottom).max(0.0), elastic))
}

/// Lay a crawl page's blocks side by side on one line centred at `line_y` (frame pixels), from
/// x = 0. Returns the pieces (tops in frame pixels) and the line's length.
fn line(ctx: &mut Ctx, styling: &Styling, page: &Page, index: usize, line_y: f64, crawl_gap: f64) -> Result<(Vec<Piece>, f64), CreditsError> {
    let s = ctx.s;
    let unit = finite_or(styling.unit_px, 32.0).clamp(0.0, 10_000.0) * s;
    let fallback = styling.content_style(&styling.default_content_style).unwrap_or_default();
    let mut missing: Vec<String> = Vec::new();
    let mut pieces: Vec<Piece> = Vec::new();
    let mut x = 0.0f64;
    for (k, b) in page.blocks.iter().enumerate() {
        let cs = match styling.content_style(&b.style) {
            Some(c) => c,
            None => {
                if !missing.contains(&b.style) {
                    missing.push(b.style.clone());
                    ctx.warn(b.row, format!("content style `{}` does not exist; `{}` is used", b.style, fallback.name));
                }
                fallback.clone()
            }
        };
        let (mut ps_, bh) = lay_block(ctx, b, &cs, 0.0, 0.0, None)?;
        if ps_.is_empty() {
            continue;
        }
        let x0 = ps_.iter().map(|p| p.x + p.b[0]).fold(f64::INFINITY, f64::min);
        let x1 = ps_.iter().map(|p| p.right()).fold(f64::NEG_INFINITY, f64::max);
        if !(x0.is_finite() && x1.is_finite()) {
            continue;
        }
        if k > 0 && !pieces.is_empty() {
            let gap = match b.gap_before {
                Gap::Units(u) => finite_or(u, 0.0).max(0.0) * unit,
                Gap::Px(p) => finite_or(p, 0.0).max(0.0) * s,
            };
            x += gap + finite_or(crawl_gap, 0.0).clamp(0.0, 100_000.0) * s;
        }
        let (dx, dy) = (x - x0, line_y - bh / 2.0);
        let nth = ctx.occurrences.get(index).and_then(|v| v.get(k)).copied().unwrap_or(0);
        key_pieces(b, nth, &mut ps_);
        for p in &mut ps_ {
            p.x += dx;
            p.top += dy;
        }
        x += x1 - x0;
        pieces.extend(ps_);
        if pieces.len() > MAX_LAYERS_PER_PAGE {
            return Err(CreditsError::TooLarge(format!("page {} needs more than {MAX_LAYERS_PER_PAGE} text layers", index + 1)));
        }
    }
    Ok((pieces, x.max(0.0)))
}

/// Give a block's pieces their stable keys and row; `nth`: which repetition of an identical
/// block it is.
fn key_pieces(b: &Block, nth: usize, pieces: &mut [Piece]) {
    let bk = block_key(b);
    for (k, p) in pieces.iter_mut().enumerate() {
        let name = match p.layer.params.get("name").map(|x| &x.value) {
            Some(ParamValue::Text(t)) => t.clone(),
            _ => String::new(),
        };
        p.key = piece_key(bk, nth, k, &name);
        p.row = b.row;
    }
}

/// `last`: no page follows; `cut_in`: the previous page has a transition into this one.
fn lay_page(ctx: &mut Ctx, styling: &Styling, page: &Page, frame: Frame, index: usize, last: bool, cut_in: bool) -> Result<PageOut, CreditsError> {
    let s = ctx.s;
    let (w, h) = (f64::from(frame.width), f64::from(frame.height));
    let ps = match styling.page_style(&page.style) {
        Some(p) => p,
        None => {
            ctx.warn(page.row, format!("page style `{}` does not exist; `Scroll` is used", page.style));
            crate::style::PageStyle::default()
        }
    };
    let crawl = ps.behaviour == Behaviour::Crawl;
    let line_y = finite_or(ps.crawl_line, 0.85).clamp(0.0, 1.0) * h;
    let (mut pieces, mut content_h, elastic, content_w) = if crawl {
        let (p, cw) = line(ctx, styling, page, index, line_y, ps.crawl_gap)?;
        let top = p.iter().map(|x| x.top).fold(f64::INFINITY, f64::min);
        let bot = bottom(&p).unwrap_or(0.0);
        (p, if top.is_finite() { (bot - top).max(0.0) } else { 0.0 }, 0.0, cw)
    } else {
        let (p, ch, el) = stack(ctx, styling, page, w, index, 1.0)?;
        (p, ch, el, 0.0)
    };
    let rate = frame.rate;
    let fd = rate.frame_duration();
    let runtime = match &page.runtime {
        Some(t) => match t.resolve(rate) {
            Some(d) if d.0 > 0 => Some(d),
            _ => {
                ctx.warn(page.row, "the page runtime is not a positive duration; the page style's is used".into());
                None
            }
        },
        None => None,
    };
    let gap_after = match &page.gap_after {
        Some(t) => t.resolve(rate).unwrap_or_else(|| {
            ctx.warn(page.row, "the page gap is not a duration; the page style's is used".into());
            frames(rate, i64::from(ps.gap_after_frames))
        }),
        None => frames(rate, i64::from(ps.gap_after_frames)),
    };
    let mut meta = GraphicMeta::default();
    let (dy, duration) = match ps.behaviour {
        Behaviour::Scroll => {
            let speed = finite_or(ps.scroll_px_per_frame, 3.0).clamp(0.1, 10_000.0) * s;
            meta.roll = Roll { mode: RollMode::Roll, start_off_screen: true, end_off_screen: true, ..Default::default() };
            match runtime {
                Some(rt) => {
                    // keep the speed and fit the runtime by stretching or squeezing the gaps
                    // between blocks; if the gaps cannot absorb it, the speed changes instead
                    let n = rt.0 as f64 / fd.0.max(1) as f64;
                    let want_content = speed * n - h;
                    let rigid = content_h - elastic;
                    let scale = (want_content - rigid) / elastic;
                    if elastic > 0.0 && (0.0..=MAX_GAP_SCALE).contains(&scale) {
                        let kept = ctx.warnings.len();
                        let (p2, c2, _) = stack(ctx, styling, page, w, index, scale)?;
                        ctx.warnings.truncate(kept);
                        pieces = p2;
                        content_h = c2;
                    } else {
                        let why = if elastic <= 0.0 {
                            "the page has no gaps between blocks to stretch"
                        } else if scale < 0.0 {
                            "the credits are too long even without gaps"
                        } else {
                            "the gaps would have to grow more than 20-fold"
                        };
                        ctx.warn(page.row, format!("page {}: the runtime is reached by changing the scroll speed ({why})", index + 1));
                    }
                    (0.0, rt)
                }
                None => {
                    let n = ((h + content_h) / speed).ceil().clamp(1.0, 86_400.0 * 240.0) as i64;
                    (0.0, frames(rate, n))
                }
            }
        }
        Behaviour::Crawl => {
            let speed = finite_or(ps.scroll_px_per_frame, 4.0).clamp(0.1, 10_000.0) * s;
            meta.roll = Roll { mode: RollMode::CrawlLeft, start_off_screen: true, end_off_screen: true, ..Default::default() };
            // a runtime changes the speed: a ticker has no gaps worth stretching
            let n = ((w + content_w) / speed).ceil().clamp(1.0, 86_400.0 * 240.0) as i64;
            (0.0, runtime.unwrap_or_else(|| frames(rate, n)))
        }
        Behaviour::Card => {
            if content_h > h {
                ctx.warn(page.row, format!("card page {} is taller than the frame; use a Scroll page", index + 1));
            }
            (((h - content_h) / 2.0), runtime.unwrap_or_else(|| frames(rate, i64::from(ps.card_frames.max(1)))))
        }
    };
    let duration = rate.snap_nearest(duration).max(fd);
    let known = |id: &str| cfa4_project::vtransition::find_transition(id, cfa4_project::EffectKind::VideoTransition).map(|d| d.id.to_string());
    let transition = match crate::style::transition_id(&ps.transition) {
        Some(id) if known(&id).is_none() => {
            ctx.warn(page.row, format!("page {}: there is no transition `{}`; the pages cut", index + 1, ps.transition.trim()));
            None
        }
        Some(id) if !last && gap_after.0 == 0 => {
            let id = known(&id).unwrap_or(id);
            let d = frames(rate, i64::from(ps.transition_frames.max(1))).min(duration);
            Some((id, d))
        }
        Some(_) if !last => {
            ctx.warn(page.row, format!("page {}: the transition needs the pages to touch (page gap 0); it is left out", index + 1));
            None
        }
        _ => None,
    };
    let (fade_in, fade_out) = if ps.behaviour == Behaviour::Card {
        let (mut fin, mut fout) = (frames(rate, i64::from(ps.fade_in_frames)), frames(rate, i64::from(ps.fade_out_frames)));
        let both = fin.0.saturating_add(fout.0);
        if both > duration.0 && both > 0 {
            fin = Tick((fin.0 as i128 * duration.0 as i128 / both as i128) as i64);
            fout = Tick(duration.0 - fin.0);
        }
        // a transition takes the place of the fade at its cut
        (if cut_in { Tick::ZERO } else { fin }, if transition.is_some() { Tick::ZERO } else { fout })
    } else {
        (Tick::ZERO, Tick::ZERO)
    };
    let mut layers = Vec::with_capacity(pieces.len());
    let mut keys = Vec::with_capacity(pieces.len());
    let mut layer_rows = Vec::with_capacity(pieces.len());
    let mut pictures = Vec::new();
    // a card's entrance: how far its content travels, how long, and each block's delay
    let card = ps.behaviour == Behaviour::Card;
    let (ux, uy) = ps.entrance.from();
    let dist = finite_or(ps.entrance_distance, 0.0).clamp(0.0, 100_000.0) * s;
    let motion = (card && ps.entrance != crate::style::Entrance::None && dist > 0.0)
        .then(|| (ux * dist, uy * dist, frames(rate, i64::from(ps.entrance_frames.clamp(1, 100_000))).min(duration)));
    let stagger = if card { frames(rate, i64::from(ps.stagger_frames.min(100_000))) } else { Tick::ZERO };
    let mut block_rows: Vec<usize> = pieces.iter().map(|p| p.row).collect();
    block_rows.sort_unstable();
    block_rows.dedup();
    let latest = Tick(duration.0.saturating_sub(fade_out.0).saturating_sub(motion.map_or(0, |m| m.2.0)).max(0));
    let delay_of = |row: usize| -> Tick {
        let k = block_rows.iter().position(|r| *r == row).unwrap_or(0) as i64;
        Tick(stagger.0.saturating_mul(k)).min(latest)
    };
    for p in pieces {
        let p_key = p.key;
        layer_rows.push(p.row);
        let mut e = p.layer;
        let y = p.top + dy - p.b[1];
        set(&mut e, "position", ParamValue::Vec2(Vec2::new(p.x, y)));
        let delay = delay_of(p.row);
        if let Some(pic) = p.pic {
            let PicInfo { file, w, h, row, video } = *pic;
            pictures.push(PicturePlace { file, center: (p.x, y), size: (w, h), row, video, delay });
        } else if card {
            card_keys(&mut e, duration, delay, fade_in, fade_out, motion, (p.x, y));
        }
        keys.push(p_key);
        layers.push(e);
    }
    ensure_uids(&mut layers);
    let keys = layers.iter().zip(keys).map(|(e, k)| (cfa4_project::gtemplate::layer_uid(e), k)).collect();
    let name = page.blocks.iter().find_map(|b| b.head.clone().or_else(|| b.body.first().cloned())).unwrap_or_default();
    let name = crate::markup::plain(&name);
    let name = name.lines().next().unwrap_or("").chars().take(40).collect::<String>();
    let name = if name.trim().is_empty() { format!("Credits {}", index + 1) } else { format!("Credits {}: {}", index + 1, name.trim()) };
    let mut spines: Vec<f64> = Vec::new();
    if !crawl {
        for b in &page.blocks {
            let x = w / 2.0 + finite_or(b.spine, 0.0) * s;
            if !spines.iter().any(|v| (v - x).abs() < 0.5) {
                spines.push(x);
            }
        }
        spines.sort_by(f64::total_cmp);
    }
    let background = (!ps.background.is_none()).then(|| ps.background.clone());
    if let Some(bg) = &background {
        use crate::style::BackgroundKind as K;
        let colours: &[&str] = match bg.kind {
            K::Color => &[&bg.color],
            K::Gradient => &[&bg.color, &bg.color2],
            _ => &[],
        };
        for c in colours {
            if parse_color(c).is_none() && !ctx.bad_colors.iter().any(|b| b == c) {
                ctx.bad_colors.push((*c).to_string());
                ctx.warn(page.row, format!("colour `{c}` is not #rrggbb; black is used"));
            }
        }
        if matches!(bg.kind, K::Image | K::Video) {
            if bg.file.trim().is_empty() {
                ctx.warn(page.row, format!("page {}: the background names no file; it is left out", index + 1));
            } else if !ctx.measure.file_exists(&bg.file) && !ctx.missing_pics.contains(&bg.file) {
                ctx.missing_pics.push(bg.file.clone());
                ctx.warn(page.row, format!("background `{}` was not found; it is left out", bg.file));
            }
        }
    }
    Ok(PageOut {
        name,
        behaviour: ps.behaviour,
        layers,
        keys,
        layer_rows,
        meta,
        duration,
        gap_after,
        content_height: content_h,
        pictures,
        fade_in,
        fade_out,
        content_width: content_w,
        content_top: dy,
        spines,
        background,
        transition,
        transition_in: Tick::ZERO,
        entrance: motion,
    })
}

/// Lay out `credits` for `frame` with `styling`.
pub fn layout(credits: &Credits, styling: &Styling, frame: Frame, measure: &dyn Measure) -> Result<Laid, CreditsError> {
    if frame.width == 0 || frame.height == 0 || frame.width > 32_768 || frame.height > 32_768 {
        return Err(CreditsError::Layout(format!("bad frame size {}×{}", frame.width, frame.height)));
    }
    if frame.rate.frame_duration().0 <= 0 {
        return Err(CreditsError::Layout("bad frame rate".into()));
    }
    let mut ctx = Ctx {
        s: f64::from(frame.height) / 1080.0,
        w: f64::from(frame.width),
        missing_pics: vec![],
        measure,
        warnings: credits.warnings.clone(),
        bad_colors: vec![],
        occurrences: occurrences(credits),
        inline: Vec::new(),
        unknown_inline: vec![],
    };
    let s = ctx.s;
    let mut inline: Vec<(String, CharStyle)> = Vec::new();
    for st in &styling.inline_styles {
        let (c, bad) = st.char_style(s);
        if let Some(b) = bad {
            ctx.warn(0, format!("colour `{b}` of letter style `{}` is not #rrggbb; it is left out", st.name));
        }
        inline.push((st.name.trim().to_lowercase(), c));
    }
    for (st, hu) in crate::markup::builtin_inline_styles() {
        let (c, _) = st.char_style(s);
        for k in [st.name.to_lowercase(), hu.to_string()] {
            if !inline.iter().any(|(n, _)| *n == k) {
                inline.push((k, c.clone()));
            }
        }
    }
    ctx.inline = inline;
    let n = credits.pages.len();
    let mut pages: Vec<PageOut> = Vec::with_capacity(n);
    for (i, p) in credits.pages.iter().enumerate() {
        let incoming = pages.last().and_then(|x| x.transition.as_ref().map(|(_, d)| *d));
        let mut page = lay_page(&mut ctx, styling, p, frame, i, i + 1 == n, incoming.is_some())?;
        page.transition_in = incoming.unwrap_or(Tick::ZERO);
        pages.push(page);
    }
    Ok(Laid { pages, warnings: ctx.warnings })
}
