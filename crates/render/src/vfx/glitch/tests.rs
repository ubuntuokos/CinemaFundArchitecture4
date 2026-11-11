//! Known-value and hostile-parameter tests for the Glitch folder.

use cfa4_project::{EffectInstance, ParamValue, find_effect};

use super::*;

fn cx(seconds: f64) -> FxCtx<'static> {
    FxCtx {
        t: cfa4_time::Tick::ZERO,
        px_scale: 1.0,
        seconds,
        timecode: "01:00:00:00",
        clip_name: "clip",
        project: None,
        env: None,
        frame_rate: 24.0,
        working: cfa4_color::WorkingSpace::Rec709,
    }
}

fn inst(id: &str) -> EffectInstance {
    find_effect(id).unwrap_or_else(|| panic!("no effect {id}")).instance()
}

fn set(e: &mut EffectInstance, k: &str, v: ParamValue) {
    e.params.get_mut(k).unwrap_or_else(|| panic!("{}: no param {k}", e.effect)).value = v;
}

fn setf(e: &mut EffectInstance, k: &str, v: f64) {
    set(e, k, ParamValue::Float(v));
}

/// Every pixel a different opaque colour, so moved pixels can be traced back.
fn picture(w: usize, h: usize) -> Image {
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let c = [(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32, ((x * 7 + y * 13) % 17) as f32 / 17.0];
            img.px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 1.0]);
        }
    }
    img
}

fn run(e: &EffectInstance, img: &Image, seconds: f64) -> Image {
    let mut o = img.clone();
    assert!(crate::vfx::apply(&mut o, e, &cx(seconds)), "{} not dispatched", e.effect);
    o
}

fn row(img: &Image, y: usize) -> Vec<[f32; 4]> {
    (0..img.w).map(|x| img.get(x, y)).collect()
}

const IDS: &[&str] = &["bit_crush", "block_glitch", "databend", "vhs", "scan_line_jitter", "tile_jitter"];

#[test]
fn glitch_effects_sit_in_the_glitch_folder() {
    for id in IDS {
        let d = find_effect(id).unwrap_or_else(|| panic!("{id}"));
        assert_eq!(d.category, cfa4_project::effect::GLITCH, "{id}");
        assert!(cfa4_project::effect::CFA4_OWN.contains(id), "{id}");
    }
}

#[test]
fn bit_crush_quantizes_and_dithers() {
    // one bit, no dither: every encoded channel is 0 or 1
    let img = picture(16, 8);
    let mut e = inst("bit_crush");
    setf(&mut e, "bits", 1.0);
    set(&mut e, "dither", ParamValue::Bool(false));
    let o = run(&e, &img, 0.0);
    for p in o.px.as_chunks::<4>().0 {
        for v in enc([p[0], p[1], p[2]]) {
            assert!(v.abs() < 1e-4 || (v - 1.0).abs() < 1e-4, "{v}");
        }
        assert_eq!(p[3], 1.0);
    }
    // ordered dither on a flat mid grey: half the pixels of each 4×4 cell light up
    let grey = dec([0.5; 3]);
    let flat = Image::filled(8, 8, [grey[0], grey[1], grey[2], 1.0]);
    set(&mut e, "dither", ParamValue::Bool(true));
    let o = run(&e, &flat, 0.0);
    let lit = o.px.as_chunks::<4>().0.iter().filter(|p| p[0] > 0.5).count();
    assert_eq!(lit, 32, "a 50% grey dithers to half on");
    // larger dither cells repeat the pattern at that size
    setf(&mut e, "cell", 2.0);
    let o2 = run(&e, &flat, 0.0);
    assert_eq!(o2.get(0, 0), o2.get(1, 1));
    assert_eq!(o2.get(2, 0), o2.get(3, 1));
}

#[test]
fn block_glitch_only_moves_blocks() {
    let img = picture(48, 32);
    let mut e = inst("block_glitch");
    setf(&mut e, "size", 6.0);
    setf(&mut e, "distance", 20.0);
    let a = run(&e, &img, 0.0);
    assert_ne!(a.px, img.px, "blocks moved");
    // every output pixel is a source pixel (copies, nothing invented)
    let src: Vec<[f32; 4]> = img.px.as_chunks::<4>().0.to_vec();
    for p in a.px.as_chunks::<4>().0 {
        assert!(src.contains(p), "{p:?}");
    }
    // the same frame step renders the same; a later step re-rolls
    assert_eq!(run(&e, &img, 0.05).px, a.px, "8 changes a second: 0.05 s is the same step");
    assert_ne!(run(&e, &img, 0.5).px, a.px);
    // Max Distance 0 leaves the picture alone even with Snap to Grid
    setf(&mut e, "distance", 0.0);
    assert_eq!(run(&e, &img, 0.0).px, img.px);
    setf(&mut e, "distance", 20.0);
    // 0 changes per second freezes it, and the seed picks another pattern
    setf(&mut e, "rate", 0.0);
    assert_eq!(run(&e, &img, 7.0).px, run(&e, &img, 0.0).px);
    let base = run(&e, &img, 0.0);
    setf(&mut e, "seed", 5.0);
    assert_ne!(run(&e, &img, 0.0).px, base.px);
}

#[test]
fn databend_shifts_rows_within_themselves() {
    let img = picture(64, 48);
    let mut e = inst("databend");
    setf(&mut e, "color", 0.0);
    setf(&mut e, "shift", 100.0);
    setf(&mut e, "breaks", 30.0);
    setf(&mut e, "size", 8.0);
    let o = run(&e, &img, 0.0);
    assert_ne!(o.px, img.px, "something was damaged");
    // with no colour damage, every pixel comes from its own row
    for y in 0..img.h {
        let src = row(&img, y);
        for p in row(&o, y) {
            assert!(src.contains(&p), "row {y}: {p:?}");
        }
    }
    // colour damage changes colours but never alpha
    setf(&mut e, "color", 100.0);
    let o = run(&e, &img, 0.0);
    assert!(o.px.as_chunks::<4>().0.iter().all(|p| p[3] == 1.0));
}

#[test]
fn vhs_bleeds_colour_and_darkens_alternate_rows() {
    // left half black, right half white
    let mut img = Image::new(40, 6);
    for y in 0..6 {
        for x in 20..40 {
            let i = (y * 40 + x) * 4;
            img.px[i..i + 4].copy_from_slice(&[1.0; 4]);
        }
        for x in 0..20 {
            img.px[(y * 40 + x) * 4 + 3] = 1.0;
        }
    }
    let mut e = inst("vhs");
    for k in ["wobble", "snow", "tracking", "head", "scanlines"] {
        setf(&mut e, k, 0.0);
    }
    setf(&mut e, "bleed", 10.0);
    let o = run(&e, &img, 0.0);
    // green keeps the sharp edge, red and blue smear to the right
    assert_eq!(o.get(20, 0)[1], 1.0);
    assert_eq!(o.get(19, 0)[1], 0.0);
    let r: Vec<f32> = (20..40).map(|x| o.get(x, 0)[0]).collect();
    assert!(r[0] < 0.5 && r.windows(2).all(|p| p[1] >= p[0]) && r[19] > 0.9, "{r:?}");
    assert_eq!(o.get(25, 0)[0], o.get(25, 0)[2]);
    // Scan Lines alone: odd rows lose a quarter at 100%, even rows are untouched
    setf(&mut e, "bleed", 0.0);
    setf(&mut e, "scanlines", 100.0);
    let o = run(&e, &img, 0.0);
    assert_eq!(o.get(30, 0), [1.0; 4]);
    assert_eq!(o.get(30, 1), [0.75, 0.75, 0.75, 1.0]);
    // a translucent pixel after an opaque white one keeps colour within its coverage
    let mut edge = Image::new(4, 1);
    edge.px[..4].copy_from_slice(&[1.0; 4]);
    edge.px[4..8].copy_from_slice(&[0.0, 0.0, 0.0, 0.1]);
    let mut e2 = inst("vhs");
    for k in ["wobble", "snow", "tracking", "head", "scanlines"] {
        setf(&mut e2, k, 0.0);
    }
    setf(&mut e2, "bleed", 10.0);
    let p = run(&e2, &edge, 0.0).get(1, 0);
    assert!(p[0] <= p[3] && p[2] <= p[3] && p[3] == 0.1, "{p:?}");
    // the full look moves from frame to frame
    let e = inst("vhs");
    let img = picture(72, 48);
    assert_ne!(run(&e, &img, 0.0).px, run(&e, &img, 1.0 / 24.0).px);
}

#[test]
fn scan_line_jitter_wraps_whole_lines() {
    let img = picture(32, 24);
    let mut e = inst("scan_line_jitter");
    setf(&mut e, "amount", 100.0);
    let sums = |img: &Image, by_row: bool| -> Vec<f32> {
        let (n, m) = if by_row { (img.h, img.w) } else { (img.w, img.h) };
        (0..n).map(|i| (0..m).map(|j| if by_row { img.get(j, i)[0] } else { img.get(i, j)[1] }).sum()).collect()
    };
    let o = run(&e, &img, 0.0);
    assert_ne!(o.px, img.px);
    // a wrapped shift keeps what is on each row
    for (a, b) in sums(&o, true).iter().zip(sums(&img, true)) {
        assert!((a - b).abs() < 1e-3, "{a} {b}");
    }
    set(&mut e, "direction", ParamValue::Choice(1));
    let o = run(&e, &img, 0.0);
    assert_ne!(o.px, img.px);
    for (a, b) in sums(&o, false).iter().zip(sums(&img, false)) {
        assert!((a - b).abs() < 1e-3, "{a} {b}");
    }
    // Periodic, half a period in: the pulse is off
    set(&mut e, "interval", ParamValue::Choice(1));
    setf(&mut e, "frequency", std::f64::consts::PI);
    setf(&mut e, "rate", 0.0);
    assert_eq!(run(&e, &img, 1.0).px, img.px);
}

#[test]
fn tile_jitter_slides_every_other_band() {
    let img = picture(20, 8);
    let mut e = inst("tile_jitter");
    setf(&mut e, "tiles", 2.0);
    setf(&mut e, "amount", 3.0);
    // at time 0 the slide is the full Amount; the top band moves, the bottom one stays
    let o = run(&e, &img, 0.0);
    for x in 0..17 {
        assert_eq!(o.get(x, 1), img.get(x + 3, 1), "x {x}");
    }
    assert_eq!(o.get(19, 1), img.get(19, 1), "clamped at the edge");
    assert_eq!(row(&o, 6), row(&img, 6));
    // vertical bands, vertical slide
    set(&mut e, "bands", ParamValue::Choice(1));
    set(&mut e, "direction", ParamValue::Choice(1));
    let o = run(&e, &img, 0.0);
    assert_eq!(o.get(2, 0), img.get(2, 3));
    assert_eq!(o.get(15, 0), img.get(15, 0));
}

#[test]
fn hostile_parameters_and_tiny_frames_stay_finite() {
    // 1e30 is finite as f32 (past every integer range); 1e300 is not
    let bad = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0e300, 1.0e300, 1.0e30, -1.0e30, 3.0e9, 0.0];
    for id in IDS {
        let d = find_effect(id).unwrap_or_else(|| panic!("{id}"));
        for (w, h) in [(1, 1), (1, 5), (5, 1), (3, 2), (33, 17)] {
            let img = picture(w, h);
            for v in bad {
                let mut e = d.instance();
                for p in &d.params {
                    if matches!(p.default, ParamValue::Float(_)) {
                        setf(&mut e, p.id, v);
                    }
                }
                for t in [0.0, 1.0e12, f64::NAN] {
                    let o = run(&e, &img, t);
                    assert_eq!((o.w, o.h), (w, h));
                    assert!(o.px.iter().all(|v| v.is_finite()), "{id} {w}x{h} {v} {t}");
                }
            }
        }
    }
}
