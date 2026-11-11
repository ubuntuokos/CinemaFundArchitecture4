//! Graphics and Titles ▸ Check Text Legibility: finds titles that are hard to read and marks
//! each one on the timeline.
//!
//! | kind | what is found |
//! |---|---|
//! | `lowContrast` | text whose colour is too close to what is behind it (WCAG 2 contrast ratio) |
//! | `smallText` | text smaller than `minSize` percent of the frame height |
//! | `titleSafe` | text reaching outside the project's title safe area |
//!
//! Each graphic clip is checked at its middle. What is behind the text is the real picture: the
//! sequence is rendered at that time with the clip's text layers hidden (its shapes, the clips
//! below it and everything else stay), so a lower-third bar, a busy shot or another title all
//! count. The contrast ratio is taken on the darkest / brightest tenth of the background under
//! the text box (the 10th percentile), so a small bright spot does not fail a title and a
//! mostly bright background does. A text background box is blended over the picture with its
//! opacity; an outer stroke at least 4 % of the font size wide separates the letters from the
//! picture, so the fill is then measured against the stroke when that is better. Drop shadows
//! are not counted. Large text (at least 4 % of the frame height, like WCAG's 18 pt) needs
//! `largeContrast` (3:1), other text `minContrast` (4.5:1).
//!
//! Text is measured on the graphic's canvas (the sequence frame) before the clip's Motion, as
//! the Essential Graphics panel lays it out. With `markers` (default on) each problem gets a
//! sequence marker whose comment starts with [`CHECK_COMMENT`]; running the check again replaces
//! them, and `markers.clearLegibility` removes them.

use std::sync::Arc;

use cfa4_project::graphic::{Appearance, LayerContent, LayerSpec};
use cfa4_project::{ItemKind, Label, Marker, MarkerId, MarkerKind, TrackKind};
use cfa4_time::Tick;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, bool_p, f64_p, has_seq};
use crate::{EngineError, Result, Session};

/// The comment every legibility marker starts with.
pub const CHECK_COMMENT: &str = "Legibility check";

/// Width of the background render (keeps the check fast on 4K/8K sequences).
const PROBE_WIDTH: f32 = 480.0;
/// Text at least this fraction of the frame height counts as large.
const LARGE_TEXT: f64 = 0.04;
/// An outer stroke at least this fraction of the font size outlines the letters.
const OUTLINE: f64 = 0.04;
/// The share of the background the contrast is taken on (the worst tenth).
const PERCENTILE: f64 = 0.1;

#[derive(Clone, Debug)]
struct Issue {
    kind: &'static str,
    title: String,
    start: Tick,
    duration: Tick,
    clip: u64,
    layer: usize,
    name: String,
    contrast: Option<f64>,
    needed: Option<f64>,
}

fn is_check_marker(m: &Marker) -> bool {
    m.comment.starts_with(CHECK_COMMENT)
}

fn has_checks(s: &Session) -> std::result::Result<(), String> {
    let q = s.active_sequence().ok_or("no sequence is open")?;
    if q.markers.iter().any(is_check_marker) { Ok(()) } else { Err("the sequence has no legibility markers".into()) }
}

fn srgb_to_linear(c: f32) -> f64 {
    let c = f64::from(c).clamp(0.0, 1.0);
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// WCAG relative luminance of linear-light RGB.
fn luminance(r: f64, g: f64, b: f64) -> f64 {
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    if y.is_finite() { y.clamp(0.0, 1.0) } else { 0.0 }
}

/// Linear-light RGB of an sRGB-encoded colour.
fn linear_rgb(c: [f32; 4]) -> [f64; 3] {
    [srgb_to_linear(c[0]), srgb_to_linear(c[1]), srgb_to_linear(c[2])]
}

/// WCAG contrast ratio (1..=21) between two luminances.
pub fn contrast_ratio(a: f64, b: f64) -> f64 {
    let (hi, lo) = if a >= b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}

/// `fg` (linear RGB, `alpha` 0..1) over `bg`, blended in linear light.
fn over(fg: [f64; 3], alpha: f64, bg: [f64; 3]) -> [f64; 3] {
    let a = if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 0.0 };
    [fg[0] * a + bg[0] * (1.0 - a), fg[1] * a + bg[1] * (1.0 - a), fg[2] * a + bg[2] * (1.0 - a)]
}

/// The colours the letters are drawn in (solid fill, or both ends of a gradient), with alpha.
fn text_colors(app: &Appearance) -> Vec<[f32; 4]> {
    match (&app.gradient, app.fill) {
        (Some(g), _) => vec![g.start, g.end],
        (None, Some(f)) => vec![f],
        // no fill: the letters are their outline
        (None, None) => app.strokes.last().map(|s| vec![s.0]).unwrap_or_default(),
    }
}

/// The contrast of a text layer against background pixels (`(r, g, b)` linear light, already
/// over black). `None` when the layer draws nothing measurable.
fn layer_contrast(spec: &LayerSpec, size_px: f64, bg: &[[f64; 3]]) -> Option<f64> {
    let opacity = f64::from(spec.transform.opacity).clamp(0.0, 1.0);
    let colors = text_colors(&spec.appearance);
    if colors.is_empty() || bg.is_empty() || opacity <= 0.0 {
        return None;
    }
    // a background box sits between the picture and the letters
    let boxed: Vec<[f64; 3]> = match spec.appearance.background {
        Some((c, _, _)) => bg.iter().map(|p| over(linear_rgb(c), f64::from(c[3]) * opacity, *p)).collect(),
        None => bg.to_vec(),
    };
    let mut worst = f64::INFINITY;
    for c in &colors {
        let lin = linear_rgb(*c);
        let alpha = f64::from(c[3]) * opacity;
        let mut ratios: Vec<f64> = boxed
            .iter()
            .map(|p| {
                let t = over(lin, alpha, *p);
                contrast_ratio(luminance(t[0], t[1], t[2]), luminance(p[0], p[1], p[2]))
            })
            .collect();
        ratios.sort_by(f64::total_cmp);
        let k = ((ratios.len() as f64 * PERCENTILE).floor() as usize).min(ratios.len().saturating_sub(1));
        let mut ratio = ratios.get(k).copied().unwrap_or(1.0);
        // an outline wide enough separates the letters from the picture
        if spec.appearance.fill.is_some() || spec.appearance.gradient.is_some() {
            let outline = spec.appearance.strokes.iter().rfind(|s| s.2 == 0 && f64::from(s.1) >= (size_px * OUTLINE).max(1.5));
            if let Some((sc, _, _)) = outline {
                let s = over(linear_rgb(*sc), f64::from(sc[3]) * opacity, [0.0; 3]);
                let t = over(lin, alpha, s);
                let ring = if sc[3] >= 0.9 { contrast_ratio(luminance(t[0], t[1], t[2]), luminance(s[0], s[1], s[2])) } else { 1.0 };
                ratio = ratio.max(ring);
            }
        }
        worst = worst.min(ratio);
    }
    worst.is_finite().then_some(worst)
}

/// The font size of a text layer in canvas pixels (its Scale applied).
fn text_size_px(spec: &LayerSpec, text_size: f32) -> f64 {
    let sy = spec.transform.scale.y.abs();
    let s = f64::from(text_size) * if sy.is_finite() { sy } else { 1.0 };
    if s.is_finite() { s.max(0.0) } else { 0.0 }
}

fn inside(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] >= b[0] - 0.5 && a[1] >= b[1] - 0.5 && a[2] <= b[2] + 0.5 && a[3] <= b[3] + 0.5
}

/// One text layer to measure: its effect index, layer number, name, spec and bounds.
struct TextLayer {
    effect: usize,
    number: usize,
    spec: LayerSpec,
    size: f64,
    bounds: [f64; 4],
}

fn check(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "graphics.checkLegibility";
    let seq_id = crate::export_tools::sequence_param(s, p, CMD)?;
    let q = s.project.sequence(seq_id).ok_or(EngineError::NoSequence)?.clone();
    let (w, h) = (f64::from(q.settings.width.max(1)), f64::from(q.settings.height.max(1)));
    let min_contrast = f64_p(p, "minContrast").unwrap_or(4.5);
    let large_contrast = f64_p(p, "largeContrast").unwrap_or(3.0);
    for (k, v) in [("minContrast", min_contrast), ("largeContrast", large_contrast)] {
        if !v.is_finite() || !(1.0..=21.0).contains(&v) {
            return Err(bad(CMD, format!("{k} must be a contrast ratio from 1 to 21")));
        }
    }
    let min_size = f64_p(p, "minSize").unwrap_or(2.5);
    if !min_size.is_finite() || !(0.0..=100.0).contains(&min_size) {
        return Err(bad(CMD, "minSize is a percent of the frame height (0 to 100)"));
    }
    let title_safe = bool_p(p, "titleSafe").unwrap_or(true);
    let ts = s.project.settings.title_safe;
    let pct = |v: f64| if v.is_finite() { v.clamp(0.0, 100.0) / 200.0 } else { 0.1 };
    let safe = [w * pct(ts.0), h * pct(ts.1), w * (1.0 - pct(ts.0)), h * (1.0 - pct(ts.1))];
    let frame_rect = [0.0, 0.0, w, h];
    let scale = (PROBE_WIDTH / w as f32).min(1.0);
    let (rw, rh) = cfa4_render::output_size(&q, scale);
    let size_ok = u32::try_from(rw).ok().zip(u32::try_from(rh).ok()).is_some_and(|(a, b)| cfa4_project::validate_frame_size(a, b).is_ok());
    if !size_ok {
        return Err(bad(CMD, "the sequence frame is too large to check"));
    }
    let rate = q.settings.frame_rate;
    let mut issues = Vec::new();
    let mut checked = 0usize;
    for t in q.video_tracks.iter().filter(|t| t.kind == TrackKind::Video && t.enabled) {
        for it in t.items.iter().filter(|it| it.enabled && it.duration > Tick::ZERO) {
            let Some(ItemKind::Graphic { width, height, .. }) = s.project.item(it.item).map(|x| &x.kind) else { continue };
            let canvas = (*width, *height);
            let (cw, ch) = (f64::from(canvas.0.max(1)), f64::from(canvas.1.max(1)));
            let half = Tick(it.duration.0 / 2);
            let mt = it.source_in + half;
            let at = rate.snap(it.start + half);
            let mut layers = Vec::new();
            for (n, (ei, spec)) in cfa4_render::graphic_clip::item_layer_specs(it, mt, canvas).into_iter().enumerate() {
                let number = n + 1;
                let LayerContent::Text(tp) = &spec.content else { continue };
                if !spec.enabled || tp.text.trim().is_empty() || spec.transform.opacity <= 0.0 {
                    continue;
                }
                let b = cfa4_render::graphic_clip::layer_bounds(&spec);
                if !b.iter().all(|v| v.is_finite()) || b[2] <= b[0] || b[3] <= b[1] {
                    continue;
                }
                // canvas → sequence frame
                let bounds = [b[0] * w / cw, b[1] * h / ch, b[2] * w / cw, b[3] * h / ch];
                let size = text_size_px(&spec, tp.size) * h / ch;
                layers.push(TextLayer { effect: ei, number, spec, size, bounds });
            }
            if layers.is_empty() {
                continue;
            }
            checked += layers.len();
            // the picture behind the text: this clip's text layers hidden, all else as shown
            let mut project = (*s.project).clone();
            if let Some((_, m)) = project.sequence_mut(seq_id).and_then(|sq| sq.find_item_mut(it.id)) {
                for l in &layers {
                    if let Some(e) = m.effects.get_mut(l.effect) {
                        e.enabled = false;
                    }
                }
            }
            let project = Arc::new(project);
            let provider = s.media.provider(project.clone(), s.services.clone());
            let opts = cfa4_render::RenderOptions { scale, ..Default::default() };
            let img = cfa4_render::render_sequence(&project, seq_id, at, opts, &provider);
            for l in &layers {
                let issue = |kind: &'static str, title: String, contrast: Option<f64>, needed: Option<f64>| Issue {
                    kind,
                    title,
                    start: it.start,
                    duration: it.duration,
                    clip: it.id.0,
                    layer: l.number,
                    name: l.spec.name.clone(),
                    contrast,
                    needed,
                };
                if title_safe && l.bounds[0] < w && l.bounds[2] > 0.0 && l.bounds[1] < h && l.bounds[3] > 0.0 && !inside(l.bounds, safe) {
                    issues.push(issue("titleSafe", "Text outside title safe".into(), None, None));
                }
                if l.size < h * min_size / 100.0 {
                    issues.push(issue("smallText", format!("Text too small ({:.0} px)", l.size), None, None));
                }
                // the background under the text box, in render pixels
                let b = l.bounds;
                let (sx, sy) = (img.w as f64 / w, img.h as f64 / h);
                let x0 = (b[0].max(0.0) * sx).floor() as usize;
                let y0 = (b[1].max(0.0) * sy).floor() as usize;
                let x1 = ((b[2].min(frame_rect[2]) * sx).ceil() as usize).min(img.w);
                let y1 = ((b[3].min(frame_rect[3]) * sy).ceil() as usize).min(img.h);
                let mut bg = Vec::new();
                for y in y0..y1 {
                    for x in x0..x1 {
                        let i = y.saturating_mul(img.w).saturating_add(x).saturating_mul(4);
                        if let Some(px) = img.px.get(i..i.saturating_add(3)) {
                            let c = |v: f32| if v.is_finite() { f64::from(v).clamp(0.0, 1.0) } else { 0.0 };
                            bg.push([c(px[0]), c(px[1]), c(px[2])]);
                        }
                    }
                }
                let needed = if l.size >= h * LARGE_TEXT { large_contrast } else { min_contrast };
                if let Some(c) = layer_contrast(&l.spec, l.size, &bg)
                    && c < needed
                {
                    issues.push(issue("lowContrast", format!("Low text contrast ({c:.1}:1, needs {needed:.1}:1)"), Some(c), Some(needed)));
                }
            }
        }
    }
    issues.sort_by_key(|i| (i.start, i.clip, i.layer, i.kind));
    let markers = bool_p(p, "markers").unwrap_or(true);
    let active = s.state.active_sequence == Some(seq_id);
    if markers && active {
        let list = issues.clone();
        s.edit_sequence("Check Text Legibility", |q, ctx, _| {
            q.markers.retain(|m| !is_check_marker(m));
            for i in &list {
                q.markers.push(Marker {
                    id: MarkerId(ctx.alloc()),
                    start: i.start,
                    duration: i.duration,
                    name: i.title.clone(),
                    comment: format!("{CHECK_COMMENT}: {} (layer {} \"{}\")", i.kind, i.layer, i.name),
                    kind: MarkerKind::Comment,
                    color: Label::Mango,
                });
            }
            q.markers.sort_by_key(|m| m.start);
            Ok(())
        })?;
    }
    let mut counts = serde_json::Map::new();
    for i in &issues {
        let n = counts.get(i.kind).and_then(Value::as_u64).unwrap_or(0);
        counts.insert(i.kind.into(), json!(n + 1));
    }
    let round = |v: f64| (v * 100.0).round() / 100.0;
    Ok(json!({
        "ok": issues.is_empty(),
        "checked": checked,
        "count": issues.len(),
        "counts": counts,
        "issues": issues.iter().map(|i| json!({
            "kind": i.kind, "title": i.title, "time": i.start.0, "seconds": i.start.seconds(),
            "duration": i.duration.0, "clip": i.clip, "layer": i.layer, "name": i.name,
            "contrast": i.contrast.map(round), "needed": i.needed,
        })).collect::<Vec<_>>(),
        "markers": markers && active,
    }))
}

fn clear(s: &mut Session, _: &Value) -> Result<Value> {
    let n = s.edit_sequence("Clear Legibility Markers", |q, _, _| {
        let before = q.markers.len();
        q.markers.retain(|m| !is_check_marker(m));
        Ok(before - q.markers.len())
    })?;
    Ok(json!({"removed": n}))
}

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "graphics.checkLegibility",
            label: "Check Text Legibility",
            menu: &["Graphics and Titles"],
            shortcut: None,
            params: r#"{"sequence":id?,"minContrast":f64=4.5,"largeContrast":f64=3,"minSize":percent=2.5,"titleSafe":bool=true,"markers":bool=true}"#,
            enabled: has_seq,
            run: check,
            journal: true,
        },
        CommandSpec {
            id: "markers.clearLegibility",
            label: "Clear Legibility Markers",
            menu: &["Markers"],
            shortcut: None,
            params: "{}",
            enabled: has_checks,
            run: clear,
            journal: true,
        },
    ]
}
