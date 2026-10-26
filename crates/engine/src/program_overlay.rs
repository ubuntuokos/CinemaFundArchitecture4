//! `program.overlay`: what the Program monitor draws over the picture for on-monitor editing, in
//! sequence frame pixels and clip pixels with the matrices between them, so a GUI only maps the
//! frame onto its picture rect and sends the edits back as ordinary commands.
//!
//! * `clips`: the pictures at the playhead, top-most first (`quad`: TL, TR, BR, BL in frame
//!   pixels), for double-clicking a clip's picture to select its Motion.
//! * `transform`: the selected effect's box while it is Motion or Transform of a selected clip
//!   under the playhead (`toFrame`: clip pixels → frame; `outer`: where its Position is measured
//!   → frame), with the values a drag starts from. Drags send `effects.setParam`.
//! * `mask`: the selected mask while its clip is selected and under the playhead: its contour,
//!   expansion and feather contours, vertices and Bezier handles, the feather / expansion handles
//!   and the corner-roundness handle, in clip pixels (`toFrame` maps them). Drags send
//!   `masks.translate` / `masks.moveVertex` / `masks.set` / `masks.roundCorners`.
//! * `clipToFrame` for the `clip` asked for (the pen draws a new mask's points on it).
//!
//! Every number here comes from project data: matrices that are not invertible or not finite
//! leave their part out instead of drawing garbage.

use cfa4_geom::{Affine, Vec2};
use cfa4_project::{ClipId, EffectInstance, MaskPath, TrackItem};
use cfa4_time::Tick;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, clip_p, f64_p, has_seq};
use crate::masks::MaskSel;
use crate::{EngineError, Result, Session};

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "program.overlay",
        label: "Program Monitor Overlay",
        menu: &[],
        shortcut: None,
        params: r#"{"clip":id? (its clipToFrame, for the pen),"roundRadius":px? (the selected mask rounded that much: roundedContour)}"#,
        enabled: has_seq,
        run: overlay,
        journal: false,
    }]
}

fn matrix(m: &Affine) -> Option<Value> {
    let v = [m.a, m.b, m.c, m.d, m.e, m.f];
    v.iter().all(|x| x.is_finite()).then(|| json!(v))
}

fn pt(v: Vec2) -> Value {
    if v.x.is_finite() && v.y.is_finite() { json!([v.x, v.y]) } else { Value::Null }
}

fn points(v: &[Vec2]) -> Value {
    Value::Array(v.iter().map(|p| pt(*p)).collect())
}

/// Clip pixels → sequence frame pixels for the clip's Motion at media time `mt`, and its size.
fn clip_to_frame(s: &Session, it: &TrackItem, mt: Tick, frame: (u32, u32)) -> Option<(Affine, (u32, u32))> {
    let seq = s.active_sequence()?;
    let size = cfa4_render::source_size(&s.project, it.item).unwrap_or(frame);
    let m = cfa4_render::motion_matrix(seq, it, size, cfa4_render::source_par(&s.project, it.item), mt);
    m.inverse().map(|_| (m, size))
}

fn quad(m: &Affine, size: (u32, u32)) -> [Vec2; 4] {
    let (w, h) = (f64::from(size.0), f64::from(size.1));
    [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].map(|(x, y)| m.apply(Vec2::new(x, y)))
}

/// Outward offset of a flattened (clip-space) polygon by `d` along averaged vertex normals.
fn offset(pts: &[Vec2], d: f64) -> Vec<Vec2> {
    let n = pts.len();
    if n < 3 || d == 0.0 || !d.is_finite() {
        return pts.to_vec();
    }
    // orientation: positive shoelace area in y-down = clockwise on screen
    let area: f64 = (0..n).map(|i| pts[i].x * pts[(i + 1) % n].y - pts[(i + 1) % n].x * pts[i].y).sum();
    let s = if area > 0.0 { 1.0 } else { -1.0 };
    (0..n)
        .map(|i| {
            let a = pts[(i + n - 1) % n];
            let b = pts[(i + 1) % n];
            let t = b - a;
            let len = t.length().max(1e-9);
            // right-hand normal of a clockwise polygon points outward
            let nrm = Vec2::new(t.y / len, -t.x / len) * s;
            pts[i] + nrm * d
        })
        .collect()
}

fn overlay(s: &mut Session, p: &Value) -> Result<Value> {
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let frame = (seq.settings.width.max(1), seq.settings.height.max(1));
    let ph = s.playhead();
    let mut out = json!({"frame": [frame.0, frame.1], "clips": [], "transform": null, "mask": null});
    // pictures at the playhead, top-most first
    let mut clips = Vec::new();
    for tr in seq.video_tracks.iter().rev().filter(|t| t.enabled) {
        let Some(it) = tr.item_at(ph).filter(|it| it.enabled) else { continue };
        if let Some((m, size)) = clip_to_frame(s, it, it.effect_time_at(ph), frame) {
            clips.push(json!({"clip": it.id.0, "quad": points(&quad(&m, size))}));
        }
    }
    out["clips"] = Value::Array(clips);
    if let Some(c) = clip_p(p, "clip") {
        let (_, it) = seq.find_item(c).ok_or_else(|| bad("program.overlay", "no such clip"))?;
        let mt = it.effect_time_at(ph.clamp(it.start, (it.end() - Tick(1)).max(it.start)));
        if let Some((m, _)) = clip_to_frame(s, it, mt, frame) {
            out["clipToFrame"] = matrix(&m).unwrap_or(Value::Null);
        }
    }
    if let Some(t) = transform(s, frame) {
        out["transform"] = t;
    }
    if let Some(m) = mask(s, frame, f64_p(p, "roundRadius")) {
        out["mask"] = m;
    }
    Ok(out)
}

/// Scale's parameter: Motion's `scale` is the height when Uniform Scale is off.
fn height_param(effect: &str) -> &'static str {
    if effect == "transform" { "scale_height" } else { "scale" }
}

/// The selected effect's box while it is Motion or Transform of a selected clip under the playhead.
fn transform(s: &Session, frame: (u32, u32)) -> Option<Value> {
    let sel = s.state.selected_effect.as_ref()?;
    if !s.state.selection.contains(&sel.clip) {
        return None;
    }
    let seq = s.active_sequence()?;
    let ph = s.playhead();
    let (_, it) = seq.find_item(sel.clip)?;
    if !it.range().contains(ph) {
        return None;
    }
    let index = sel.index(&it.effects)?;
    let e = it.effects.get(index).filter(|e| e.enabled)?;
    let def = e.def().filter(|d| d.id == "motion" || d.id == "transform")?;
    let mt = it.effect_time_at(ph);
    let (motion, size) = clip_to_frame(s, it, mt, frame)?;
    // "auto" points (NaN) are where the render puts them
    let point = |e: &EffectInstance, id: &str, auto: Vec2| {
        let v = e.param(id).map_or(auto, |p| p.vec2_at(mt));
        Vec2::new(if v.x.is_nan() { auto.x } else { v.x }, if v.y.is_nan() { auto.y } else { v.y })
    };
    let centre = Vec2::new(f64::from(size.0) / 2.0, f64::from(size.1) / 2.0);
    let uniform = e.param("uniform_scale").and_then(|p| p.value.as_bool()).unwrap_or(true);
    let (height, width, rotation) = (e.f64_at(height_param(&e.effect), mt), e.f64_at("scale_width", mt), e.f64_at("rotation", mt));
    let (outer, to_frame, position, anchor) = if e.effect == "transform" {
        let (anchor, position) = (point(e, "anchor", centre), point(e, "position", centre));
        let (sh, sw) = (height / 100.0, if uniform { height } else { width } / 100.0);
        // as the render places it (`cfa4_render::gpufx`, "transform")
        let axis = e.f64_at("skew_axis", mt);
        let shear = Affine { c: e.f64_at("skew", mt).to_radians().tan(), ..Affine::IDENTITY };
        let skew = Affine::rotate_deg(axis).then_apply(&shear).then_apply(&Affine::rotate_deg(-axis));
        let own = Affine::translate(position.x, position.y)
            .then_apply(&Affine::rotate_deg(rotation))
            .then_apply(&skew)
            .then_apply(&Affine::scale(sw, sh))
            .then_apply(&Affine::translate(-anchor.x, -anchor.y));
        (motion, motion.then_apply(&own), position, anchor)
    } else {
        let frame_centre = Vec2::new(f64::from(seq.settings.width) / 2.0, f64::from(seq.settings.height) / 2.0);
        (Affine::IDENTITY, motion, point(e, "position", frame_centre), point(e, "anchor", centre))
    };
    to_frame.inverse()?;
    outer.inverse()?;
    Some(json!({
        "clip": sel.clip.0,
        "effect": index,
        "effectId": e.effect,
        "name": def.name,
        "size": [size.0, size.1],
        "quad": points(&quad(&to_frame, size)),
        "anchorAt": pt(to_frame.apply(anchor)),
        "outer": matrix(&outer)?,
        "toFrame": matrix(&to_frame)?,
        "position": pt(position),
        "anchor": pt(anchor),
        "height": height,
        "width": width,
        "rotation": rotation,
        "uniform": uniform,
    }))
}

/// A sharp corner `masks.roundCorners` can fillet: its point, the unit bisector pointing to the
/// fillet's centre, `sin(θ/2)` of its opening angle θ, and the largest radius it takes (the fillet
/// never uses more than half of either edge).
struct RoundableCorner {
    p: Vec2,
    bisector: Vec2,
    sin_half: f64,
    max_radius: f64,
}

fn roundable_corners(path: &MaskPath) -> Vec<RoundableCorner> {
    let n = path.vertices.len();
    path.roundable_corners()
        .into_iter()
        .filter_map(|i| {
            let v = path.vertices.get(i)?.p;
            let prev = path.vertices.get((i + n - 1) % n)?.p;
            let next = path.vertices.get((i + 1) % n)?.p;
            let (a, b) = (prev - v, next - v);
            let (la, lb) = (a.length(), b.length());
            if !(la > 1e-6 && lb > 1e-6) {
                return None;
            }
            let (u, w) = (a * (1.0 / la), b * (1.0 / lb));
            let theta = (u.x * w.x + u.y * w.y).clamp(-1.0, 1.0).acos();
            if !(1e-6..std::f64::consts::PI - 1e-6).contains(&theta) {
                return None;
            }
            let s = u + w;
            let sl = s.length();
            if !sl.is_finite() || sl <= 1e-9 {
                return None;
            }
            let half = theta / 2.0;
            let max_radius = la.min(lb) / 2.0 * half.tan();
            max_radius.is_finite().then_some(RoundableCorner { p: v, bisector: s * (1.0 / sl), sin_half: half.sin(), max_radius })
        })
        .collect()
}

/// The selected mask, while its clip is selected and under the playhead.
fn mask(s: &Session, frame: (u32, u32), round: Option<f64>) -> Option<Value> {
    let sel: MaskSel = s.state.selected_mask?;
    if !s.state.selection.contains(&sel.clip) {
        return None;
    }
    let seq = s.active_sequence()?;
    let ph = s.playhead();
    let (_, it) = seq.find_item(ClipId(sel.clip.0))?;
    if ph < it.start || ph >= it.end() {
        return None;
    }
    let e = it.effects.get(sel.effect)?;
    let m = e.masks.get(sel.mask)?;
    let mt = it.effect_time_at(ph);
    let (to_frame, _) = clip_to_frame(s, it, mt, frame)?;
    let path = m.path_at(mt);
    let feather = m.feather.f64_at(mt).max(0.0);
    let expansion = m.expansion.f64_at(mt);
    let flat = path.flatten(0.5);
    if flat.is_empty() {
        return None;
    }
    // the feather handle at the contour's top, the expansion handle at its bottom (on screen)
    let on_frame: Vec<Vec2> = flat.iter().map(|v| to_frame.apply(*v)).collect();
    let top = (0..flat.len()).min_by(|a, b| on_frame[*a].y.total_cmp(&on_frame[*b].y));
    let bottom = (0..flat.len()).max_by(|a, b| on_frame[*a].y.total_cmp(&on_frame[*b].y));
    let unit = offset(&flat, 1.0);
    let normal = |i: usize| {
        let o = unit.get(i).copied().unwrap_or_default() - flat.get(i).copied().unwrap_or_default();
        o * (1.0 / o.length().max(1e-9))
    };
    let handle =
        |i: Option<usize>, dist: f64| i.and_then(|i| flat.get(i).map(|p| json!({"at": pt(*p), "normal": pt(normal(i)), "dist": dist}))).unwrap_or(Value::Null);
    let corners = roundable_corners(&path);
    let roundness = corners
        .iter()
        .min_by(|a, b| {
            let (pa, pb) = (to_frame.apply(a.p), to_frame.apply(b.p));
            (pa.x + pa.y).total_cmp(&(pb.x + pb.y))
        })
        .map(|c| {
            json!({
                "corner": pt(c.p),
                "bisector": pt(c.bisector),
                "sinHalf": c.sin_half,
                "maxRadius": corners.iter().map(|c| c.max_radius).fold(0.0, f64::max),
            })
        })
        .unwrap_or(Value::Null);
    let rounded = round.filter(|r| r.is_finite() && *r > 0.0).map(|r| points(&path.with_rounded_corners(r, &path.roundable_corners()).flatten(0.5)));
    Some(json!({
        "clip": sel.clip.0,
        "effect": sel.effect,
        "mask": sel.mask,
        "name": m.name,
        "toFrame": matrix(&to_frame)?,
        "contour": points(&flat),
        "expansionContour": if expansion.abs() > 1e-6 { points(&offset(&flat, expansion)) } else { Value::Null },
        "featherContour": if feather > 1e-6 { points(&offset(&flat, expansion + feather / 2.0)) } else { Value::Null },
        "feather": feather,
        "expansion": expansion,
        "vertices": path.vertices.iter().map(|v| json!({"p": pt(v.p), "in": pt(v.p + v.t_in), "out": pt(v.p + v.t_out), "corner": v.is_corner()})).collect::<Vec<_>>(),
        "featherHandle": handle(top, expansion + feather / 2.0),
        "expansionHandle": handle(bottom, expansion),
        "roundness": roundness,
        "roundedContour": rounded,
    }))
}
