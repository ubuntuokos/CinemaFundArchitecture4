//! Lay out read credits into graphic-clip layers: one graphic clip per page.
//!
//! Every piece of text is an ordinary CFA4 text layer (`graphic_text`), so a laid-out page
//! can be edited afterwards in Essential Graphics like any title. Scroll pages use the graphic's
//! Roll (start and end off screen); card pages are centred vertically and fade with opacity
//! keyframes. Text sizes come from the caller's [`Measure`] (the engine uses the real text
//! engine), so this crate stays free of fonts and rendering.

use cfa4_geom::Vec2;
use cfa4_project::graphic::{new_shape_layer, new_text_layer};
use cfa4_project::graphic_design::{GraphicMeta, Roll, RollMode};
use cfa4_project::gtemplate::ensure_uids;
use cfa4_project::{EffectInstance, Interpolation, Keyframe, Param, ParamValue};
use cfa4_time::{FrameRate, Tick};

use crate::CreditsError;
use crate::sheet::{Block, Credits, Gap, Page, Warning};
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
}

/// A picture to show with a page, in graphic-canvas pixels where the page was laid out (before
/// any roll): the engine places it as its own clip that moves with the page.
#[derive(Clone, Debug, PartialEq)]
pub struct PicturePlace {
    pub file: String,
    pub center: (f64, f64),
    pub size: (f64, f64),
    pub row: usize,
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
    pub meta: GraphicMeta,
    pub duration: Tick,
    /// Empty time after the page; negative overlaps the next page.
    pub gap_after: Tick,
    /// Height of the content (frame pixels).
    pub content_height: f64,
    /// Pictures that go with this page.
    pub pictures: Vec<PicturePlace>,
    /// Card fades actually used (zero for scroll pages).
    pub fade_in: Tick,
    pub fade_out: Tick,
}

/// Laid-out credits.
#[derive(Clone, Debug)]
pub struct Laid {
    pub pages: Vec<PageOut>,
    pub warnings: Vec<Warning>,
}

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
    pic: Option<Box<(String, f64, f64, usize)>>,
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
        let mut e = new_text_layer(text, Vec2::new(0.0, 0.0), size);
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
    match cs.layout {
        BodyLayout::Grid => {
            let n = (cs.columns.clamp(1, 16) as usize).min(b.body.len()).max(1);
            if n == 1 {
                let (e, bb) = ctx.text(b.row, "Body", &b.body.join("\n"), &cs.body, align, None)?;
                out.push(Piece { layer: e, b: bb, x, top, pic: None });
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
                out.push(Piece { layer: e, b: bb, x: origin_in_box(left, colw, align), top, pic: None });
                left += colw + gap;
            }
        }
        BodyLayout::Flow => {
            let wrap = finite_or(cs.wrap_width, 1200.0).clamp(10.0, 100_000.0) * s;
            let text = b.body.iter().map(|c| c.replace('\n', " ")).collect::<Vec<_>>().join(&cs.flow_separator);
            let (e, bb) = ctx.text(b.row, "Body", &text, &cs.body, align, Some(wrap))?;
            out.push(Piece { layer: e, b: bb, x: left_of(x, wrap, align), top, pic: None });
        }
        BodyLayout::Paragraphs => {
            let wrap = finite_or(cs.wrap_width, 1200.0).clamp(10.0, 100_000.0) * s;
            let gap = finite_or(cs.paragraph_gap, 0.0).max(0.0) * s;
            let mut y = top;
            for (k, cell) in b.body.iter().enumerate() {
                let (e, bb) = ctx.text(b.row, &format!("Paragraph {}", k + 1), cell, &cs.body, align, Some(wrap))?;
                let h = bb[3] - bb[1];
                out.push(Piece { layer: e, b: bb, x: left_of(x, wrap, align), top: y, pic: None });
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
fn lay_pictures(ctx: &mut Ctx, b: &Block, cs: &ContentStyle, anchor_x: f64, align: Align, top: f64) -> Vec<Piece> {
    let s = ctx.s;
    let mut sized = Vec::new();
    for p in &b.pictures {
        let Some((pw, ph)) = ctx.measure.picture_size(&p.file).filter(|(w, h)| w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0) else {
            if !ctx.missing_pics.contains(&p.file) {
                ctx.missing_pics.push(p.file.clone());
                ctx.warn(b.row, format!("picture `{}` was not found or cannot be read; it is left out", p.file));
            }
            continue;
        };
        let mut h = finite_or(p.height.unwrap_or(DEFAULT_PICTURE_HEIGHT), DEFAULT_PICTURE_HEIGHT).clamp(1.0, 10_000.0) * s;
        let mut w = h * pw / ph;
        // never wider than the frame
        let max_w = ctx.w * 0.95;
        if w > max_w {
            h *= max_w / w;
            w = max_w;
        }
        sized.push((p.file.clone(), w, h));
    }
    let gap = finite_or(cs.column_gap, 0.0).max(0.0) * s;
    let total: f64 = sized.iter().map(|(_, w, _)| *w).sum::<f64>() + gap * (sized.len().saturating_sub(1)) as f64;
    let row_h = sized.iter().map(|(_, _, h)| *h).fold(0.0f64, f64::max);
    let mut left = left_of(anchor_x, total, align);
    let mut out = Vec::new();
    for (k, (file, w, h)) in sized.into_iter().enumerate() {
        let cx = left + w / 2.0;
        let mut e = new_shape_layer(0, Vec2::new(0.0, 0.0), Vec2::new(w, h), vec![]);
        set(&mut e, "name", ParamValue::Text(format!("Picture {}: {file}", k + 1)));
        set(&mut e, "fill", ParamValue::Bool(false));
        let top_k = top + (row_h - h) / 2.0;
        out.push(Piece { layer: e, b: [-w / 2.0, -h / 2.0, w / 2.0, h / 2.0], x: cx, top: top_k, pic: Some(Box::new((file, w, h, b.row))) });
        left += w + gap;
    }
    out
}

/// Lay out one block with its top at `top`. Returns its pieces and height.
fn lay_block(ctx: &mut Ctx, b: &Block, cs: &ContentStyle, spine_x: f64, top: f64) -> Result<(Vec<Piece>, f64), CreditsError> {
    let s = ctx.s;
    let mut pieces = Vec::new();
    let head_gap = finite_or(cs.head_gap, 0.0).max(0.0) * s;
    let tail_gap = finite_or(cs.tail_gap, 0.0).max(0.0) * s;
    match cs.orientation {
        Orientation::Vertical => {
            let mut y = top;
            if let Some(h) = &b.head {
                let (e, bb) = ctx.text(b.row, "Head", h, &cs.head, cs.align, None)?;
                let p = Piece { layer: e, b: bb, x: spine_x, top: y, pic: None };
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
                pieces.push(Piece { layer: e, b: bb, x: spine_x, top: y + tail_gap, pic: None });
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
                let (e, bb) = ctx.text(b.row, "Head", h, &cs.head, Align::Right, None)?;
                pieces.push(Piece { layer: e, b: bb, x: spine_x - head_gap / 2.0, top: at_baseline(bb, first_baseline), pic: None });
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
                        pieces.push(Piece { layer: e, b: bb, x, top: at_baseline(bb, bl), pic: None });
                    }
                } else {
                    let text = b.tails.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join("\n");
                    let (e, bb) = ctx.text(b.row, "Tail", &text, &cs.tail, Align::Left, None)?;
                    pieces.push(Piece { layer: e, b: bb, x, top: at_baseline(bb, first_baseline), pic: None });
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

fn opacity_fade(e: &mut EffectInstance, d: Tick, fin: Tick, fout: Tick) {
    if fin.0 <= 0 && fout.0 <= 0 {
        return;
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
    e.params.insert("opacity".into(), Param { value: ParamValue::Float(100.0), keyframes: kfs });
}

fn frames(rate: FrameRate, n: i64) -> Tick {
    Tick(rate.frame_duration().0.saturating_mul(n))
}

fn lay_page(ctx: &mut Ctx, styling: &Styling, page: &Page, frame: Frame, index: usize) -> Result<PageOut, CreditsError> {
    let s = ctx.s;
    let (w, h) = (f64::from(frame.width), f64::from(frame.height));
    let ps = match styling.page_style(&page.style) {
        Some(p) => p,
        None => {
            ctx.warn(page.row, format!("page style `{}` does not exist; `Scroll` is used", page.style));
            crate::style::PageStyle::default()
        }
    };
    let unit = finite_or(styling.unit_px, 32.0).clamp(0.0, 10_000.0) * s;
    let mut pieces: Vec<Piece> = Vec::new();
    let mut y = 0.0f64;
    let mut prev_margin = 0.0f64;
    let fallback = styling.content_style(&styling.default_content_style).unwrap_or_default();
    let mut missing: Vec<String> = Vec::new();
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
        if k > 0 {
            let gap = match b.gap_before {
                Gap::Units(u) => finite_or(u, 0.0).max(0.0) * unit,
                Gap::Px(p) => finite_or(p, 0.0).max(0.0) * s,
            };
            y += gap.max(margin).max(prev_margin);
        }
        prev_margin = margin;
        let spine_x = w / 2.0 + finite_or(b.spine, 0.0) * s;
        let (ps_, bh) = lay_block(ctx, b, &cs, spine_x, y)?;
        y += bh;
        pieces.extend(ps_);
        if pieces.len() > MAX_LAYERS_PER_PAGE {
            return Err(CreditsError::TooLarge(format!("page {} needs more than {MAX_LAYERS_PER_PAGE} text layers", index + 1)));
        }
    }
    let content_h = y.max(0.0);
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
            let travel = h + content_h;
            let n = (travel / speed).ceil().clamp(1.0, 86_400.0 * 240.0) as i64;
            meta.roll = Roll { mode: RollMode::Roll, start_off_screen: true, end_off_screen: true, ..Default::default() };
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
    let (fade_in, fade_out) = if ps.behaviour == Behaviour::Card {
        let (mut fin, mut fout) = (frames(rate, i64::from(ps.fade_in_frames)), frames(rate, i64::from(ps.fade_out_frames)));
        let both = fin.0.saturating_add(fout.0);
        if both > duration.0 && both > 0 {
            fin = Tick((fin.0 as i128 * duration.0 as i128 / both as i128) as i64);
            fout = Tick(duration.0 - fin.0);
        }
        (fin, fout)
    } else {
        (Tick::ZERO, Tick::ZERO)
    };
    let mut layers = Vec::with_capacity(pieces.len());
    let mut pictures = Vec::new();
    for p in pieces {
        let mut e = p.layer;
        let y = p.top + dy - p.b[1];
        set(&mut e, "position", ParamValue::Vec2(Vec2::new(p.x, y)));
        if let Some(pic) = p.pic {
            let (file, w, h, row) = *pic;
            pictures.push(PicturePlace { file, center: (p.x, y), size: (w, h), row });
        } else if ps.behaviour == Behaviour::Card {
            opacity_fade(&mut e, duration, fade_in, fade_out);
        }
        layers.push(e);
    }
    ensure_uids(&mut layers);
    let name = page.blocks.iter().find_map(|b| b.head.clone().or_else(|| b.body.first().cloned())).unwrap_or_default();
    let name = name.lines().next().unwrap_or("").chars().take(40).collect::<String>();
    let name = if name.trim().is_empty() { format!("Credits {}", index + 1) } else { format!("Credits {}: {}", index + 1, name.trim()) };
    Ok(PageOut { name, behaviour: ps.behaviour, layers, meta, duration, gap_after, content_height: content_h, pictures, fade_in, fade_out })
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
    };
    let mut pages = Vec::with_capacity(credits.pages.len());
    for (i, p) in credits.pages.iter().enumerate() {
        pages.push(lay_page(&mut ctx, styling, p, frame, i)?);
    }
    Ok(Laid { pages, warnings: ctx.warnings })
}
