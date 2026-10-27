//! Property and golden tests for every video transition.
//!
//! Golden fingerprints (`goldens.txt`): a 4×4 grid of mean 8-bit RGB values at p = 0.25 / 0.5 /
//! 0.75 on two procedural test frames. Re-bless after an intended change with
//! `CFA4_BLESS=1 cargo test -p cfa4-render transitions::tests::golden_fingerprints`
//! and review the diff.

use cfa4_geom::Vec2;
use cfa4_project::{EffectDef, EffectKind, ParamKind, ParamValue, effect_defs};

use super::*;

const W: usize = 128;
const H: usize = 72;

/// Outgoing test frame: warm horizontal ramp with a checker overlay.
fn frame_a() -> Image {
    paint(W, H, |x, y| {
        let u = x / W as f32;
        let cell = W as f32 / 8.0;
        let chk = if ((x / cell).floor() as i32 + (y / cell).floor() as i32) % 2 == 0 { 0.15 } else { 0.0 };
        [0.2 + 0.7 * u + chk, 0.1 + 0.3 * (y / H as f32), 0.05 + chk, 1.0]
    })
}
/// Incoming test frame: cool vertical ramp with a bright disc.
fn frame_b() -> Image {
    paint(W, H, |x, y| {
        let v = y / H as f32;
        let d = ((x - W as f32 * 0.62).powi(2) + (y - H as f32 * 0.4).powi(2)).sqrt();
        let disc = if d < H as f32 * 0.22 { 0.6 } else { 0.0 };
        [0.05 + disc, 0.2 + 0.2 * v + disc, 0.4 + 0.5 * v, 1.0]
    })
}

fn transitions() -> impl Iterator<Item = &'static EffectDef> {
    effect_defs().iter().filter(|d| d.kind == EffectKind::VideoTransition)
}

fn mean_abs(a: &Image, b: &Image) -> f32 {
    a.px.iter().zip(&b.px).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.px.len() as f32
}

#[test]
fn endpoints_are_exact_outgoing_and_incoming() {
    let (a, b) = (frame_a(), frame_b());
    for d in transitions() {
        let e = d.instance();
        assert_eq!(apply(&e, &a, &b, 0.0), a, "{} p=0 is A", d.id);
        assert_eq!(apply(&e, &a, &b, 1.0), b, "{} p=1 is B", d.id);
    }
}

#[test]
fn continuous_near_both_ends() {
    let (a, b) = (frame_a(), frame_b());
    for d in transitions() {
        let e = d.instance();
        let near_a = mean_abs(&apply(&e, &a, &b, 0.01), &a);
        let near_b = mean_abs(&apply(&e, &a, &b, 0.99), &b);
        assert!(near_a < 0.06, "{}: p=0.01 jumps away from A by {near_a}", d.id);
        assert!(near_b < 0.06, "{}: p=0.99 jumps away from B by {near_b}", d.id);
    }
}

#[test]
fn midpoint_is_a_real_transition_and_sane() {
    let (a, b) = (frame_a(), frame_b());
    for d in transitions() {
        let e = d.instance();
        for p in [0.3, 0.5, 0.7] {
            let m = apply(&e, &a, &b, p);
            assert_eq!((m.w, m.h), (W, H));
            assert!(m.px.iter().all(|v| v.is_finite()), "{} p={p}: non-finite", d.id);
            assert!(m.px.chunks(4).all(|c| (-1e-4..=1.0 + 1e-4).contains(&c[3])), "{} p={p}: alpha out of range", d.id);
            assert!(m.px.iter().all(|v| *v > -0.3 && *v < 50.0), "{} p={p}: wild values", d.id);
        }
        let m = apply(&e, &a, &b, 0.5);
        let (da, db) = (mean_abs(&m, &a), mean_abs(&m, &b));
        assert!(da > 0.005 && db > 0.005, "{}: midpoint equals one side (ΔA {da}, ΔB {db})", d.id);
    }
}

#[test]
fn deterministic() {
    let (a, b) = (frame_a(), frame_b());
    for d in transitions() {
        let e = d.instance();
        assert_eq!(apply(&e, &a, &b, 0.37), apply(&e, &a, &b, 0.37), "{}", d.id);
    }
}

#[test]
fn no_nan_on_degenerate_inputs() {
    let clear = Image::new(W, H);
    let (a, b) = (frame_a(), frame_b());
    let tiny_a = Image::filled(1, 1, [1.0, 0.0, 0.0, 1.0]);
    let tiny_b = Image::filled(1, 1, [0.0, 0.0, 1.0, 1.0]);
    let odd_a = Image::filled(3, 2, [0.5, 0.5, 0.0, 1.0]);
    let odd_b = Image::filled(3, 2, [0.0, 0.5, 0.5, 1.0]);
    for d in transitions() {
        let e = d.instance();
        for p in [f32::NAN, -1.0, 1e-6, 0.25, 0.5, 0.999_999, 2.0] {
            for (x, y) in [(&a, &clear), (&clear, &b), (&tiny_a, &tiny_b), (&odd_a, &odd_b)] {
                let m = apply(&e, x, y, p);
                assert!(m.px.iter().all(|v| v.is_finite()), "{} p={p} {}x{}", d.id, x.w, x.h);
            }
        }
        // a single-sided fade from nothing ends exactly on B
        assert_eq!(apply(&e, &clear, &b, 1.0), b);
    }
}

/// A non-default value for a parameter (to check every parameter is wired).
fn changed(def: &cfa4_project::ParamDef) -> Option<ParamValue> {
    Some(match (&def.kind, &def.default) {
        (ParamKind::Float { min, max, .. }, ParamValue::Float(v)) => {
            let hi = if *max > 1e5 { v * 2.0 + 10.0 } else { *max };
            let target = if (hi - v).abs() > (v - min).abs() { v + (hi - v) * 0.5 } else { v - (v - min) * 0.5 };
            // pixel sizes are authored for 1080p; keep them meaningful on the small test frame
            ParamValue::Float(if def.id == "border_width" {
                6.0
            } else if matches!(&def.kind, ParamKind::Float { unit: "px", .. }) && *v > 20.0 {
                v * 0.25
            } else {
                target
            })
        }
        (ParamKind::Choice(opts), ParamValue::Choice(c)) => ParamValue::Choice((c + 1) % opts.len() as u32),
        (_, ParamValue::Bool(v)) => ParamValue::Bool(!v),
        (_, ParamValue::Color(c)) => ParamValue::Color([1.0 - c[0], 0.5, c[2] * 0.3, 1.0]),
        (ParamKind::Point, _) => ParamValue::Vec2(Vec2::new(12.0, 9.0)),
        (ParamKind::Angle, ParamValue::Float(v)) => ParamValue::Float(v + 70.0),
        _ => return None,
    })
}

#[test]
fn every_parameter_changes_the_picture() {
    let (a, b) = (frame_a(), frame_b());
    let mut failures = Vec::new();
    for d in transitions() {
        let mut base = d.instance();
        // make shape/border parameters observable on the small test frame
        let mut set = |id: &str, v: ParamValue| {
            if let Some(p) = base.params.get_mut(id) {
                p.value = v;
            }
        };
        if d.id == "shape_dissolve" {
            set("size", ParamValue::Float(16.0));
            set("shape", ParamValue::Choice(3));
        }
        for pd in &d.params {
            let Some(v) = changed(pd) else { continue };
            let mut base = base.clone();
            if pd.id == "border_color"
                && let Some(bw) = base.params.get_mut("border_width")
            {
                bw.value = ParamValue::Float(6.0);
            }
            let base = base;
            let mut e = base.clone();
            e.params.get_mut(pd.id).unwrap().value = v;
            if pd.id == "border_width" {
                // a border only shows if it differs from the content
                if let Some(c) = e.params.get_mut("border_color") {
                    c.value = ParamValue::Color([0.0, 1.0, 0.0, 1.0]);
                }
            }
            let differs = [0.2, 0.35, 0.5, 0.65, 0.8].iter().any(|&p| mean_abs(&apply(&base, &a, &b, p), &apply(&e, &a, &b, p)) > 1e-4);
            if !differs {
                failures.push(format!("{}.{}", d.id, pd.id));
            }
        }
    }
    assert!(failures.is_empty(), "parameters with no visible effect: {failures:?}");
}

#[test]
fn directions_differ() {
    let (a, b) = (frame_a(), frame_b());
    for d in transitions().filter(|d| d.param("direction").is_some()) {
        let mut renders = Vec::new();
        for dir in 0..4 {
            let mut e = d.instance();
            e.params.get_mut("direction").unwrap().value = ParamValue::Choice(dir);
            renders.push(apply(&e, &a, &b, 0.4));
        }
        assert!(mean_abs(&renders[1], &renders[3]) > 1e-3, "{}: East vs West identical", d.id);
    }
}

#[test]
fn wipe_border_and_centre() {
    let (a, b) = (frame_a(), frame_b());
    let def = cfa4_project::find_effect("iris_round").unwrap();
    let mut e = def.instance();
    e.params.get_mut("border_width").unwrap().value = ParamValue::Float(3.0);
    e.params.get_mut("border_color").unwrap().value = ParamValue::Color([0.0, 1.0, 0.0, 1.0]);
    let m = apply(&e, &a, &b, 0.4);
    let green = m.px.chunks(4).filter(|c| c[1] > 0.9 && c[0] < 0.1 && c[2] < 0.1).count();
    assert!(green > 20, "border drawn ({green} px)");
    // centre at the top-left: the incoming clip shows there first
    let mut e = def.instance();
    e.params.get_mut("center").unwrap().value = ParamValue::Vec2(Vec2::new(0.0, 0.0));
    let m = apply(&e, &a, &b, 0.2);
    assert_eq!(m.get(1, 1), b.get(1, 1));
    assert_eq!(m.get(W - 2, H - 2), a.get(W - 2, H - 2));
    // half-resolution parameters scale with the image
    let half = apply_scaled(&def.instance(), &a, &b, 0.5, 0.5);
    assert!(half.px.iter().all(|v| v.is_finite()));
}

#[test]
fn keyhole_opens_from_the_centre_in_a_keyhole_shape() {
    use crate::transitions::wipe::keyhole_dist;
    // the head is wider than the slot: at the same distance, sideways in the head is inside a
    // smaller keyhole than sideways in the slot
    assert!(keyhole_dist(0.4, 0.3) < keyhole_dist(0.4, -0.5));
    // scale-invariant metric: doubling the point doubles the value
    for (x, y) in [(0.3, 0.2), (-0.1, -0.6), (0.0, 0.7)] {
        assert!((keyhole_dist(2.0 * x, 2.0 * y) - 2.0 * keyhole_dist(x, y)).abs() < 1e-4);
    }
    assert_eq!(keyhole_dist(0.0, 0.0), 0.0);
    assert!(keyhole_dist(f32::NAN, 1.0).is_finite());
    // early on, the centre shows B and the corners still show A
    let (a, b) = (frame_a(), frame_b());
    let e = cfa4_project::find_effect("keyhole").unwrap().instance();
    let o = apply(&e, &a, &b, 0.3);
    let (cx, cy) = (o.w / 2, o.h / 2);
    assert_eq!(o.get(cx, cy), b.get(cx, cy));
    assert_eq!(o.get(0, 0), a.get(0, 0));
}

#[test]
fn burn_away_leaves_an_ember_edge_between_the_clips() {
    let (a, b) = (frame_a(), frame_b());
    let e = cfa4_project::find_effect("burn_away").unwrap().instance();
    let o = apply(&e, &a, &b, 0.5);
    // pixels that are neither clip: the glowing rim (red well above blue) and the dark char
    let near = |p: [f32; 4], q: [f32; 4]| (0..3).all(|k| (p[k] - q[k]).abs() < 0.02);
    let between: Vec<[f32; 4]> = (0..o.h)
        .flat_map(|y| (0..o.w).map(move |x| (x, y)))
        .filter(|&(x, y)| !near(o.get(x, y), a.get(x, y)) && !near(o.get(x, y), b.get(x, y)))
        .map(|(x, y)| o.get(x, y))
        .collect();
    let ember = between.iter().filter(|p| p[0] > 0.5 && p[0] > p[2] * 3.0).count();
    let char_px = between.iter().filter(|p| p[0] + p[1] + p[2] < 0.15).count();
    assert!(char_px > 0, "a charred band shows mid-transition");
    assert!(ember > 0, "an orange ember edge shows mid-transition");
    let from_a = (0..o.h).flat_map(|y| (0..o.w).map(move |x| (x, y))).filter(|&(x, y)| near(o.get(x, y), a.get(x, y))).count();
    let from_b = (0..o.h).flat_map(|y| (0..o.w).map(move |x| (x, y))).filter(|&(x, y)| near(o.get(x, y), b.get(x, y))).count();
    assert!(from_a > o.w * o.h / 20 && from_b > o.w * o.h / 20, "both clips show at the midpoint: {from_a} / {from_b}");
}

#[test]
fn linear_wipe_reveals_from_the_trailing_side() {
    let (a, b) = (frame_a(), frame_b());
    let e = cfa4_project::find_effect("linear_wipe").unwrap().instance(); // 90° = left → right
    let m = apply(&e, &a, &b, 0.5);
    assert_eq!(m.get(2, 10), b.get(2, 10));
    assert_eq!(m.get(W - 3, 10), a.get(W - 3, 10));
}

#[test]
fn push_moves_frames_by_the_eased_distance() {
    let (a, b) = (frame_a(), frame_b());
    let mut e = cfa4_project::find_effect("push").unwrap().instance();
    e.params.get_mut("motion_blur").unwrap().value = ParamValue::Float(0.0);
    let m = apply(&e, &a, &b, 0.5); // eased 0.5 → half a frame, from the west
    for x in [4usize, 20, 28] {
        let want = b.get(x + W / 2, 12);
        let got = m.get(x, 12);
        assert!(want.iter().zip(got).all(|(p, q)| (p - q).abs() < 1e-3), "x={x}: {got:?} vs {want:?}");
    }
}

fn close(got: [f32; 4], want: [f32; 4], tol: f32) -> bool {
    want.iter().zip(got).all(|(p, q)| (p - q).abs() < tol)
}

fn tx<'a>(e: &'a cfa4_project::EffectInstance, a: &'a Image, b: &'a Image) -> Tx<'a> {
    Tx { e, a, b, p: 0.5, w: a.w, h: a.h, wf: a.w as f32, hf: a.h as f32, scale: 1.0 }
}

#[test]
fn slide_moves_diagonally_and_keeps_the_frame_covered() {
    let (a, b) = (frame_a(), frame_b());
    let (w, h) = (W as i64, H as i64);
    // direction → B's offset at the eased half-way point (half a frame from its corner)
    for (dir, (bx, by)) in [(4u32, (-w / 2, -h / 2)), (5, (w / 2, -h / 2)), (6, (w / 2, h / 2)), (7, (-w / 2, h / 2))] {
        let mut e = cfa4_project::find_effect("slide").unwrap().instance();
        e.params.get_mut("motion_blur").unwrap().value = ParamValue::Float(0.0);
        e.params.get_mut("direction").unwrap().value = ParamValue::Choice(dir);
        let m = apply(&e, &a, &b, 0.5);
        // B fills its quarter; the other three quarters are A, still in place
        for (qx, qy) in [(w / 4, h / 4), (3 * w / 4, h / 4), (w / 4, 3 * h / 4), (3 * w / 4, 3 * h / 4)] {
            let in_b = (qx - bx) >= 0 && (qx - bx) < w && (qy - by) >= 0 && (qy - by) < h;
            let want = if in_b { b.get((qx - bx) as usize, (qy - by) as usize) } else { a.get(qx as usize, qy as usize) };
            assert!(close(m.get(qx as usize, qy as usize), want, 1e-3), "dir {dir}: ({qx}, {qy}) in B {in_b}");
        }
        assert!(m.px.chunks(4).all(|c| (c[3] - 1.0).abs() < 1e-3), "dir {dir}: every pixel opaque");
    }
}

#[test]
fn slide_cardinal_directions_and_push_are_unchanged() {
    let (a, b) = (frame_a(), frame_b());
    let def = cfa4_project::find_effect("slide").unwrap();
    for dir in 0..4u32 {
        let mut e = def.instance();
        e.params.get_mut("direction").unwrap().value = ParamValue::Choice(dir);
        let t = tx(&e, &a, &b);
        let (dx, dy) = t.dir();
        assert_eq!(t.shift8(), (dx * t.travel(), dy * t.travel()), "dir {dir}");
    }
    // out-of-range choices from a project file fall back to From West, as everywhere; Push has
    // no diagonals (two cards moving diagonally would leave two quarters of the frame empty)
    for (id, bad) in [("slide", u32::MAX), ("push", u32::MAX), ("push", 4)] {
        let def = cfa4_project::find_effect(id).unwrap();
        let mut e = def.instance();
        e.params.get_mut("direction").unwrap().value = ParamValue::Choice(bad);
        assert_eq!(apply(&e, &a, &b, 0.4), apply(&def.instance(), &a, &b, 0.4), "{id} {bad}");
    }
}

#[test]
fn motion_camera_negative_zoom_pulls_back_without_smeared_edges() {
    let (a, b) = (frame_a(), frame_b());
    let mut e = cfa4_project::find_effect("motion_camera").unwrap().instance();
    for (k, v) in [("zoom", -30.0), ("rotation", 0.0), ("motion_blur", 0.0)] {
        e.params.get_mut(k).unwrap().value = ParamValue::Float(v);
    }
    assert_eq!(apply(&e, &a, &b, 0.0), a);
    assert_eq!(apply(&e, &a, &b, 1.0), b);
    // before the cut A has shrunk: the corner shows the (magnified) blue B, not A's smeared edge
    let m = apply(&e, &a, &b, 0.3);
    let corner = m.get(1, 1);
    assert!(corner[2] > 0.3 && (corner[3] - 1.0).abs() < 1e-3, "corner {corner:?}");
    // while the centre is still the warm A
    let centre = m.get(W / 2, H / 2);
    assert!(centre[0] > 0.3, "centre {centre:?}");
    // a hostile zoom (−1000 %) is clamped, never a division by zero
    e.params.get_mut("zoom").unwrap().value = ParamValue::Float(-1000.0);
    for p in [0.2, 0.5, 0.8] {
        assert!(apply(&e, &a, &b, p).px.iter().all(|v| v.is_finite()), "p={p}");
    }
}

#[test]
fn grunge_feather_softens_the_torn_edge() {
    let (a, b) = (frame_a(), frame_b());
    let def = cfa4_project::find_effect("grunge").unwrap();
    assert_eq!(def.instance().f64_at("feather", cfa4_time::Tick::ZERO), 0.0, "hard edge by default");
    let mut soft = def.instance();
    soft.params.get_mut("edge_width").unwrap().value = ParamValue::Float(0.0);
    soft.params.get_mut("feather").unwrap().value = ParamValue::Float(40.0);
    let mut sharp = soft.clone();
    sharp.params.get_mut("feather").unwrap().value = ParamValue::Float(0.0);
    // pixels that are a real blend of A and B (neither side within 2 %)
    let blended = |m: &Image| {
        (0..H)
            .flat_map(|y| (0..W).map(move |x| (x, y)))
            .filter(|&(x, y)| !close(m.get(x, y), a.get(x, y), 0.02) && !close(m.get(x, y), b.get(x, y), 0.02))
            .count()
    };
    let (ns, nh) = (blended(&apply(&soft, &a, &b, 0.5)), blended(&apply(&sharp, &a, &b, 0.5)));
    assert!(ns > nh * 2, "feather widens the blended edge ({ns} vs {nh} px)");
    assert_eq!(apply(&soft, &a, &b, 0.0), a);
    assert_eq!(apply(&soft, &a, &b, 1.0), b);
    soft.params.get_mut("feather").unwrap().value = ParamValue::Float(1e30);
    assert!(apply(&soft, &a, &b, 0.5).px.iter().all(|v| v.is_finite()));
}

#[test]
fn modern_and_legacy_ids_are_distinct_renders() {
    let (a, b) = (frame_a(), frame_b());
    for (modern, legacy) in [("push", "push_legacy"), ("clock_wipe", "clock_wipe_legacy"), ("cross_zoom", "cross_zoom_legacy"), ("whip", "whip_legacy")] {
        let m = apply(&cfa4_project::find_effect(modern).unwrap().instance(), &a, &b, 0.4);
        let l = apply(&cfa4_project::find_effect(legacy).unwrap().instance(), &a, &b, 0.4);
        assert!(mean_abs(&m, &l) > 1e-4, "{modern} vs {legacy}");
    }
    // the legacy dissolves keep their exact maths
    let e = cfa4_project::find_effect("cross_dissolve_legacy").unwrap().instance();
    let m = apply(&e, &a, &b, 0.25);
    let want = a.clone().lerp(&b, 0.25);
    assert!(mean_abs(&m, &want) < 1e-6);
}

/// A jump cut (256×144): a static background with a 64×72 subject whose left edge is at `x0`;
/// flat colours, or a smooth texture on both (the subject's own texture moves with it).
fn jump_cut_frame(x0: f32, textured: bool) -> Image {
    let tex = |u: f32, v: f32, k: f32| {
        0.35 + 0.15 * (u * 0.21 * k).sin() * (v * 0.17).cos() + 0.1 * ((u + v) * 0.11 * k).sin() + 0.05 * (u * 0.53 - v * 0.31 * k).cos()
    };
    paint(256, 144, |x, y| {
        let inside = (x0..x0 + 64.0).contains(&x) && (36.0..108.0).contains(&y);
        let l = match (textured, inside) {
            (false, true) => 0.95,
            (false, false) => 0.3,
            (true, true) => 0.4 + tex(x - x0, y, 0.6),
            (true, false) => 0.6 * tex(x, y, 1.0),
        };
        [l, l * 0.9, l * 0.8, 1.0]
    })
}

#[test]
fn morph_cut_moves_the_subject_instead_of_double_exposing() {
    // the subject jumps 12 px between the shots; at the midpoint Morph Cut shows it once, 6 px
    // along, where a cross dissolve shows two half-bright copies
    let (a, b) = (jump_cut_frame(96.0, false), jump_cut_frame(108.0, false));
    let want = jump_cut_frame(102.0, false);
    let morph = apply(&cfa4_project::find_effect("morph_cut").unwrap().instance(), &a, &b, 0.5);
    let dissolve = apply(&cfa4_project::find_effect("cross_dissolve").unwrap().instance(), &a, &b, 0.5);
    // mean red error down a column through the subject's rows
    let column_err = |img: &Image, x: usize| (40..104).map(|y| (img.get(x, y)[0] - want.get(x, y)[0]).abs()).sum::<f32>() / 64.0;
    // uncovered background, both edges, the middle, background still to be covered
    for x in [98usize, 103, 134, 165, 170] {
        let (m, d) = (column_err(&morph, x), column_err(&dissolve, x));
        assert!(m < 0.12, "x={x}: morph off by {m} (dissolve {d})");
    }
    for x in [98usize, 170] {
        assert!(column_err(&dissolve, x) > 0.3, "x={x}: the dissolve ghosts here");
    }
    let (e_morph, e_dissolve) = (mean_abs(&morph, &want), mean_abs(&dissolve, &want));
    assert!(e_morph * 10.0 < e_dissolve, "morph {e_morph} vs dissolve {e_dissolve}");
    // the background away from the subject is left alone
    for (x, y) in [(8, 8), (40, 70), (220, 130)] {
        assert!(morph.get(x, y).iter().zip(a.get(x, y)).all(|(p, q)| (p - q).abs() < 1e-4), "({x}, {y})");
    }
}

#[test]
fn morph_cut_tracks_textured_subjects_and_follows_the_progress() {
    // detail on the subject and behind it: the morph lands much closer than the dissolve, at
    // the midpoint and a quarter of the way through
    for (p, from, to, at) in [(0.5f32, 96.0f32, 108.0f32, 102.0f32), (0.25, 96.0, 112.0, 100.0)] {
        let (a, b) = (jump_cut_frame(from, true), jump_cut_frame(to, true));
        let want = jump_cut_frame(at, true);
        let morph = apply(&cfa4_project::find_effect("morph_cut").unwrap().instance(), &a, &b, p);
        let dissolve = a.clone().lerp(&b, p);
        let (e_morph, e_dissolve) = (mean_abs(&morph, &want), mean_abs(&dissolve, &want));
        assert!(e_morph * 2.0 < e_dissolve, "p={p}: morph {e_morph} vs dissolve {e_dissolve}");
    }
}

// ---------------------------------------------------------------------------------------------
// Golden fingerprints

fn fingerprint(img: &Image) -> String {
    let mut s = String::new();
    for by in 0..4 {
        for bx in 0..4 {
            let mut acc = [0.0f32; 3];
            let (x0, x1, y0, y1) = (bx * W / 4, (bx + 1) * W / 4, by * H / 4, (by + 1) * H / 4);
            for y in y0..y1 {
                for x in x0..x1 {
                    let c = img.get(x, y);
                    for k in 0..3 {
                        acc[k] += c[k];
                    }
                }
            }
            let n = ((x1 - x0) * (y1 - y0)) as f32;
            for v in acc {
                s.push_str(&format!("{:02x}", ((v / n).clamp(0.0, 1.0) * 255.0).round() as u8));
            }
        }
    }
    s
}

fn golden_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/transitions/goldens.txt")
}

#[test]
fn golden_fingerprints() {
    let (a, b) = (frame_a(), frame_b());
    let mut lines = Vec::new();
    for d in transitions() {
        let e = d.instance();
        for p in [0.25f32, 0.5, 0.75] {
            lines.push(format!("{} {p:.2} {}", d.id, fingerprint(&apply(&e, &a, &b, p))));
        }
    }
    let path = golden_path();
    if std::env::var_os("CFA4_BLESS").is_some() {
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).expect("goldens.txt (bless with CFA4_BLESS=1)");
    let want: std::collections::HashMap<(String, String), String> = want
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some(((it.next()?.to_string(), it.next()?.to_string()), it.next()?.to_string()))
        })
        .collect();
    let mut bad = Vec::new();
    for l in &lines {
        let mut it = l.split_whitespace();
        let (id, p, got) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
        let Some(exp) = want.get(&(id.to_string(), p.to_string())) else {
            bad.push(format!("{id} {p}: no golden"));
            continue;
        };
        let bytes = |s: &str| (0..s.len() / 2).map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap_or(0) as i32).collect::<Vec<_>>();
        let (g, e) = (bytes(got), bytes(exp));
        let worst = g.iter().zip(&e).map(|(x, y)| (x - y).abs()).max().unwrap_or(255);
        if g.len() != e.len() || worst > 4 {
            bad.push(format!("{id} {p}: max Δ {worst}"));
        }
    }
    assert!(bad.is_empty(), "transition goldens differ: {bad:?}");
}

/// Contact sheets for eyeballing: `CFA4_TRANSITION_SHEETS=<dir> cargo test -p cfa4-render
/// contact_sheets -- --ignored` writes one PNG per folder (rows = transitions, columns = p).
#[test]
#[ignore]
fn contact_sheets() {
    let Some(dir) = std::env::var_os("CFA4_TRANSITION_SHEETS") else { return };
    let (cw, chh) = (192usize, 108usize);
    let a = paint(cw, chh, |x, y| {
        let u = x / cw as f32;
        let cell = cw as f32 / 8.0;
        let chk = if ((x / cell).floor() as i32 + (y / cell).floor() as i32) % 2 == 0 { 0.15 } else { 0.0 };
        [0.2 + 0.7 * u + chk, 0.1 + 0.3 * (y / chh as f32), 0.05 + chk, 1.0]
    });
    let b = paint(cw, chh, |x, y| {
        let v = y / chh as f32;
        let d = ((x - cw as f32 * 0.62).powi(2) + (y - chh as f32 * 0.4).powi(2)).sqrt();
        let disc = if d < chh as f32 * 0.22 { 0.6 } else { 0.0 };
        [0.05 + disc, 0.2 + 0.2 * v + disc, 0.4 + 0.5 * v, 1.0]
    });
    let ps = [0.1f32, 0.25, 0.4, 0.5, 0.6, 0.75, 0.9];
    let key = |d: &EffectDef| d.category.join("-").replace([' ', '&'], "");
    let mut folders: Vec<String> = transitions().map(key).collect();
    folders.dedup();
    for f in folders {
        let defs: Vec<_> = transitions().filter(|d| key(d) == f).collect();
        let (sw, sh) = (cw * ps.len(), chh * defs.len());
        let mut sheet = ::image::RgbaImage::new(sw as u32, sh as u32);
        for (r, d) in defs.iter().enumerate() {
            for (c, &p) in ps.iter().enumerate() {
                let m = apply(&d.instance(), &a, &b, p);
                let px = m.over_black_rgba8();
                for y in 0..chh {
                    for x in 0..cw {
                        let i = (y * cw + x) * 4;
                        sheet.put_pixel((c * cw + x) as u32, (r * chh + y) as u32, ::image::Rgba([px[i], px[i + 1], px[i + 2], 255]));
                    }
                }
            }
            eprintln!("{f}: row {r} = {}", d.id);
        }
        sheet.save(std::path::Path::new(&dir).join(format!("{f}.png"))).unwrap();
    }
}

/// The disc blur as it was before #1871: offsets worked out for every pixel.
fn disc_blur_per_pixel(img: &Image, x: f32, y: f32, r: f32, n: usize) -> [f32; 4] {
    if r < 0.5 || n <= 1 {
        return img.sample_bilinear_clamped(x, y);
    }
    let mut acc = [0.0; 4];
    for i in 0..n {
        let rr = r * ((i as f32 + 0.5) / n as f32).sqrt();
        let a = i as f32 * 2.399_963;
        let p = img.sample_bilinear_clamped(x + rr * a.cos(), y + rr * a.sin());
        for k in 0..4 {
            acc[k] += p[k];
        }
    }
    scale4(acc, 1.0 / n as f32)
}

/// #1871: offsets worked out once per frame give exactly the pixels the per-pixel version gave.
#[test]
fn disc_blur_with_shared_offsets_is_bit_identical() {
    let img = frame_b();
    for (r, n) in [(0.0, 24), (0.4, 24), (3.0, 8), (30.0, 24), (75.5, 48), (200.0, 48), (12.0, 1)] {
        let offsets = disc_offsets(r, n);
        for y in (0..H).step_by(7) {
            for x in (0..W).step_by(5) {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let a = disc_blur_per_pixel(&img, fx, fy, r, n);
                let b = disc_blur_with(&img, fx, fy, &offsets);
                assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits), "r {r}, n {n}, at ({x}, {y})");
            }
        }
    }
}

/// `cargo test --release -p cfa4-render --lib lens_blur_frame_time -- --ignored --nocapture`
#[test]
#[ignore]
fn lens_blur_frame_time() {
    let (w, h) = (1920, 1080);
    let img = paint(w, h, |x, y| [x / w as f32, y / h as f32, 0.5, 1.0]);
    let (r, n) = (30.0, 24);
    let time = |f: &dyn Fn() -> Image| {
        let t = std::time::Instant::now();
        let mut out = f();
        for _ in 0..4 {
            out = f();
        }
        (t.elapsed().as_secs_f64() * 1000.0 / 5.0, out)
    };
    let (before, a) = time(&|| paint(w, h, |x, y| disc_blur_per_pixel(&img, x, y, r, n)));
    let offsets = disc_offsets(r, n);
    let (after, b) = time(&|| paint(w, h, |x, y| disc_blur_with(&img, x, y, &offsets)));
    assert_eq!(a.px, b.px);
    eprintln!(
        "1080p disc blur of one picture, radius {r}, {n} samples: {before:.1} ms per pixel offsets, {after:.1} ms shared offsets ({:.2}x)",
        before / after
    );
}
