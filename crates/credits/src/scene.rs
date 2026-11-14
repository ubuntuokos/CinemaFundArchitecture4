//! Title scenes in a credits document (F10-C16): title, opening, chapter and lower-third pages.
//!
//! A page whose style has one of these behaviours ([`Behaviour::uses_template`]) is not laid out
//! as text: **each of its blocks fills a graphics template** (`.fcgt`, the same templates as
//! Essential Graphics) and becomes a card of its own. The block's head, body and tail go into the
//! template's text controls ([`PageStyle::fields`], or by the controls' ids), so one spreadsheet
//! holds the main title, the opening credits, the lower thirds of the cast, chapter titles and the
//! rolling end credits, and a refresh from the spreadsheet updates them all.
//!
//! The placed graphic stays linked to its template (Essential Graphics ▸ Edit shows the controls)
//! and to the credits document, like every other credits page.

use cfa4_project::gtemplate::{ControlKind, GraphicsTemplate, builtin_templates, ensure_uids, layer_uid};
use cfa4_project::{EffectInstance, Param, ParamValue};
use cfa4_time::{FrameRate, Tick};

use crate::CreditsError;
use crate::layout::{Frame, Measure, PageOut, block_key, card_params, piece_key};
use crate::sheet::{Block, Credits, Page};
use crate::style::{PageStyle, Styling};

/// A built-in graphics template by id (`builtin:title-centered`, also without `builtin:`) or name.
pub fn builtin_template(key: &str) -> Option<GraphicsTemplate> {
    let k = key.trim();
    builtin_templates().into_iter().find(|t| t.id == k || t.id.strip_prefix("builtin:") == Some(k) || t.name.eq_ignore_ascii_case(k))
}

/// The part of a block a template control takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Head,
    Body,
    Tail,
}

impl Part {
    /// `head` / `body` / `tail`, also in Hungarian (`fej`, `törzs`, `láb`).
    pub fn parse(s: &str) -> Option<Part> {
        Some(match s.trim().to_lowercase().as_str() {
            "head" | "fej" | "fejléc" | "szerep" => Part::Head,
            "body" | "törzs" | "név" | "nevek" => Part::Body,
            "tail" | "láb" | "farok" => Part::Tail,
            _ => return None,
        })
    }

    fn text(self, b: &Block) -> String {
        let raw = match self {
            Part::Head => b.head.clone().unwrap_or_default(),
            Part::Body => b.body.join("\n"),
            Part::Tail => b.tails.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join("\n"),
        };
        crate::markup::plain(&raw).trim().to_string()
    }
}

/// Control ids that take the body, then the head, when the page style maps none.
const BODY_IDS: [&str; 6] = ["name", "title", "heading", "text", "label", "credits"];
const HEAD_IDS: [&str; 4] = ["role", "subtitle", "job", "caption"];

/// Which text control (index into `t.controls`) takes which part. Unknown control ids or parts
/// in `fields` are reported through `bad`.
pub fn assign(t: &GraphicsTemplate, fields: &std::collections::BTreeMap<String, String>, bad: &mut dyn FnMut(&str, &str)) -> Vec<(usize, Part)> {
    let text: Vec<usize> = t.controls.iter().enumerate().filter(|(_, c)| c.kind == ControlKind::Text && c.param == "text").map(|(i, _)| i).collect();
    let mut out: Vec<(usize, Part)> = Vec::new();
    if !fields.is_empty() {
        for (id, part) in fields {
            let ctl = text.iter().copied().find(|i| t.controls.get(*i).is_some_and(|c| c.id.eq_ignore_ascii_case(id.trim())));
            match (ctl, Part::parse(part)) {
                (Some(i), Some(p)) if !out.iter().any(|(j, _)| *j == i) => out.push((i, p)),
                _ => bad(id, part),
            }
        }
        return out;
    }
    let mut free = text;
    let mut take = |ids: &[&str]| -> Option<usize> {
        let k = ids
            .iter()
            .find_map(|id| free.iter().position(|i| t.controls.get(*i).is_some_and(|c| c.id.eq_ignore_ascii_case(id))))
            .or_else(|| (!free.is_empty()).then_some(0))?;
        Some(free.remove(k))
    };
    for (ids, part) in [(&BODY_IDS[..], Part::Body), (&HEAD_IDS[..], Part::Head), (&[][..], Part::Tail)] {
        if let Some(i) = take(ids) {
            out.push((i, part));
        }
    }
    out
}

/// Template pages hold one row each: every row with a name (body cell) of a template page
/// becomes a page of its own (a lower third per cast member), with the page's style, runtime and
/// gap. The head goes with the block's first row, a tail with the row it stands on.
pub fn split_pages(credits: &mut Credits, styling: &Styling) {
    let template = |p: &Page| styling.page_style(&p.style).is_some_and(|s| s.behaviour.uses_template());
    let several = |p: &Page| p.blocks.len() > 1 || p.blocks.iter().any(|b| b.body.len() > 1);
    if !credits.pages.iter().any(|p| several(p) && template(p)) {
        return;
    }
    let mut out = Vec::with_capacity(credits.pages.len());
    for p in std::mem::take(&mut credits.pages) {
        if !(several(&p) && template(&p)) {
            out.push(p);
            continue;
        }
        for b in p.blocks {
            for one in split_rows(b) {
                out.push(Page { style: p.style.clone(), runtime: p.runtime.clone(), gap_after: p.gap_after.clone(), row: one.row, blocks: vec![one] });
            }
        }
    }
    credits.pages = out;
}

/// A block of several body cells as one block per cell.
fn split_rows(b: Block) -> Vec<Block> {
    if b.body.len() <= 1 {
        return vec![b];
    }
    let n = b.body.len();
    (0..n)
        .map(|k| Block {
            style: b.style.clone(),
            spine: b.spine,
            gap_before: if k == 0 { b.gap_before } else { crate::sheet::Gap::Units(0.0) },
            head: if k == 0 { b.head.clone() } else { None },
            body: b.body.get(k).cloned().into_iter().collect(),
            tails: b.tails.iter().filter(|(i, _)| *i == k || (k + 1 == n && *i >= n)).map(|(_, t)| (0, t.clone())).collect(),
            pictures: if k == 0 { b.pictures.clone() } else { Vec::new() },
            portraits: b.portraits.get(k).cloned().map(|p| vec![p]).unwrap_or_default(),
            body_rows: b.body_rows.get(k).copied().into_iter().collect(),
            parallel: false,
            row: if k == 0 { b.row } else { b.body_rows.get(k).copied().unwrap_or(b.row) },
        })
        .collect()
}

fn frames(rate: FrameRate, n: i64) -> Tick {
    Tick(rate.frame_duration().0.saturating_mul(n))
}

/// The static opacity of a layer (percent), or `None` when it is animated.
fn static_opacity(e: &EffectInstance) -> Option<f64> {
    match e.params.get("opacity") {
        None => Some(100.0),
        Some(p) if p.keyframes.is_empty() => match p.value {
            ParamValue::Float(v) => Some(v),
            _ => Some(100.0),
        },
        Some(_) => None,
    }
}

/// Lay out a template page: its block fills the page style's graphics template. `at` is the
/// page's index and which repetition of its block it is; `ends` whether no page follows and
/// whether the previous page has a transition into this one.
pub(crate) fn lay_scene(
    page: &Page,
    ps: &PageStyle,
    frame: Frame,
    (index, nth): (usize, usize),
    (last, cut_in): (bool, bool),
    measure: &dyn Measure,
    warn: &mut dyn FnMut(usize, String),
) -> Result<PageOut, CreditsError> {
    let (w, h) = (f64::from(frame.width), f64::from(frame.height));
    let Some(block) = page.blocks.first() else {
        return Err(CreditsError::Layout(format!("page {} has no block", index + 1)));
    };
    let fallback = ps.behaviour.default_template().unwrap_or("builtin:title-centered");
    let wanted = if ps.template.trim().is_empty() { fallback } else { ps.template.trim() };
    let t = match measure.template(wanted) {
        Some(t) => t,
        None => {
            let t = builtin_template(fallback).ok_or_else(|| CreditsError::Layout(format!("the built-in graphics template `{fallback}` is missing")))?;
            warn(page.row, format!("page {}: graphics template `{wanted}` was not found; `{}` is used", index + 1, t.name));
            t
        }
    };
    if !block.pictures.is_empty() || block.portraits.iter().any(Option::is_some) {
        warn(block.row, format!("page {}: pictures are left out on title, opening, chapter and lower-third pages", index + 1));
    }
    let (mut layers, meta) = t.instantiate((frame.width, frame.height));
    ensure_uids(&mut layers);
    let mut unused: Vec<(String, String)> = Vec::new();
    let assigned = assign(&t, &ps.fields, &mut |c, p| unused.push((c.to_string(), p.to_string())));
    for (c, p) in unused {
        warn(page.row, format!("page {}: template field `{c}` → `{p}` is not used (no such text control, or not head, body or tail)", index + 1));
    }
    let filled: Vec<Part> = assigned.iter().map(|(_, p)| *p).collect();
    if [Part::Head, Part::Body, Part::Tail].iter().any(|p| !filled.contains(p) && !p.text(block).is_empty()) {
        warn(block.row, format!("page {}: graphics template `{}` has no field for all of the block's text; the rest is left out", index + 1, t.name));
    }
    for (ci, part) in &assigned {
        let Some(ctl) = t.controls.get(*ci) else { continue };
        let value = part.text(block);
        if let Some(e) = layers.iter_mut().find(|e| layer_uid(e) == ctl.layer) {
            // an empty part hides its layer, so a title without a subtitle shows no blank line
            e.enabled = !value.is_empty();
            e.params.insert(ctl.param.clone(), Param::new(ParamValue::Text(value)));
            if let Some(x) = e.layer.as_mut() {
                // the template's per-letter styles belong to its sample text
                x.runs.clear();
            }
        }
    }

    // timing as on a card page
    let rate = frame.rate;
    let fd = rate.frame_duration();
    let runtime = match &page.runtime {
        Some(t) => match t.resolve(rate) {
            Some(d) if d.0 > 0 => Some(d),
            _ => {
                warn(page.row, "the page runtime is not a positive duration; the page style's is used".into());
                None
            }
        },
        None => None,
    };
    let gap_after = match &page.gap_after {
        Some(t) => t.resolve(rate).unwrap_or_else(|| {
            warn(page.row, "the page gap is not a duration; the page style's is used".into());
            frames(rate, i64::from(ps.gap_after_frames))
        }),
        None => frames(rate, i64::from(ps.gap_after_frames)),
    };
    let duration = rate.snap_nearest(runtime.unwrap_or_else(|| frames(rate, i64::from(ps.card_frames.max(1))))).max(fd);
    let known = |id: &str| cfa4_project::vtransition::find_transition(id, cfa4_project::EffectKind::VideoTransition).map(|d| d.id.to_string());
    let transition = match crate::style::transition_id(&ps.transition) {
        Some(id) if known(&id).is_none() => {
            warn(page.row, format!("page {}: there is no transition `{}`; the pages cut", index + 1, ps.transition.trim()));
            None
        }
        Some(id) if !last && gap_after.0 == 0 => {
            let id = known(&id).unwrap_or(id);
            Some((id, frames(rate, i64::from(ps.transition_frames.max(1))).min(duration)))
        }
        Some(_) if !last => {
            warn(page.row, format!("page {}: the transition needs the pages to touch (page gap 0); it is left out", index + 1));
            None
        }
        _ => None,
    };
    let (mut fin, mut fout) = (frames(rate, i64::from(ps.fade_in_frames)), frames(rate, i64::from(ps.fade_out_frames)));
    let both = fin.0.saturating_add(fout.0);
    if both > duration.0 && both > 0 {
        fin = Tick((fin.0 as i128 * duration.0 as i128 / both as i128) as i64);
        fout = Tick(duration.0 - fin.0);
    }
    let (fade_in, fade_out) = (if cut_in { Tick::ZERO } else { fin }, if transition.is_some() { Tick::ZERO } else { fout });
    if let (Some(fade), _) = card_params(duration, Tick::ZERO, fade_in, fade_out, None, (0.0, 0.0)) {
        for e in &mut layers {
            // layers the template animates itself keep their own opacity
            let Some(base) = static_opacity(e) else { continue };
            let mut p = fade.clone();
            for k in &mut p.keyframes {
                if let ParamValue::Float(v) = &mut k.value {
                    *v *= base / 100.0;
                }
            }
            p.value = ParamValue::Float(base);
            e.params.insert("opacity".into(), p);
        }
    }

    let bk = block_key(block);
    let keys = layers
        .iter()
        .map(|e| {
            let uid = layer_uid(e);
            let name = match e.params.get("name").map(|p| &p.value) {
                Some(ParamValue::Text(n)) => n.clone(),
                _ => String::new(),
            };
            (uid, piece_key(bk, nth, uid as usize, &name))
        })
        .collect();
    let layer_rows = vec![block.row; layers.len()];
    let main = Part::Body.text(block);
    let main = if main.is_empty() { Part::Head.text(block) } else { main };
    let main: String = main.lines().next().unwrap_or("").chars().take(40).collect();
    let name = if main.trim().is_empty() { format!("{} {}", t.name, index + 1) } else { format!("{} {}: {}", t.name, index + 1, main.trim()) };
    let background = (!ps.background.is_none()).then(|| ps.background.clone());
    if let Some(bg) = &background
        && matches!(bg.kind, crate::style::BackgroundKind::Image | crate::style::BackgroundKind::Video)
        && !bg.file.trim().is_empty()
        && !measure.file_exists(&bg.file)
    {
        warn(page.row, format!("background `{}` was not found; it is left out", bg.file));
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
        content_height: h,
        pictures: Vec::new(),
        fade_in,
        fade_out,
        content_width: 0.0,
        content_top: 0.0,
        spines: vec![w / 2.0],
        background,
        transition,
        transition_in: Tick::ZERO,
        entrance: None,
        block_tops: vec![(block.row, 0.0)],
    })
}
