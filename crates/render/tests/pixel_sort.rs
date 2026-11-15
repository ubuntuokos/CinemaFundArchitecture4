//! Pixel Sort (`pixel_sort`, Video Effects ▸ Creative Looks): runs inside the threshold band are
//! sorted, every line keeps its pixels, and the angle turns the lines.

use cfa4_color::linear_to_srgb;
use cfa4_project::{EffectInstance, ParamValue, find_effect};
use cfa4_render::{
    Image,
    effects::{FxCtx, apply},
};
use cfa4_time::Tick;

fn cx() -> FxCtx<'static> {
    FxCtx {
        t: Tick::ZERO,
        px_scale: 1.0,
        seconds: 0.0,
        timecode: "00:00:00:00",
        clip_name: "",
        project: None,
        env: None,
        frame_rate: 24.0,
        working: cfa4_color::WorkingSpace::Rec709,
    }
}

fn sort(params: &[(&str, ParamValue)]) -> EffectInstance {
    let mut e = find_effect("pixel_sort").expect("pixel_sort").instance();
    for (k, v) in params {
        e.param_mut(k).unwrap_or_else(|| panic!("pixel_sort.{k}")).value = v.clone();
    }
    e
}

fn render(e: &EffectInstance, img: &Image) -> Image {
    let mut out = img.clone();
    apply(&mut out, e, &cx());
    out
}

fn fl(v: f64) -> ParamValue {
    ParamValue::Float(v)
}
fn ch(c: u32) -> ParamValue {
    ParamValue::Choice(c)
}

/// The whole band: every opaque pixel is sortable.
fn all(params: &[(&str, ParamValue)]) -> EffectInstance {
    let mut p = vec![("lower", fl(0.0)), ("upper", fl(100.0))];
    p.extend(params.iter().cloned());
    sort(&p)
}

/// Opaque greys, scrambled per row.
fn scrambled(w: usize, h: usize) -> Image {
    let mut img = Image::new(w, h);
    for (i, p) in img.px.chunks_exact_mut(4).enumerate() {
        let (x, y) = (i % w, i / w);
        let v = ((x * 7 + y * 3) % w) as f32 / w as f32;
        p.copy_from_slice(&[v, v, v, 1.0]);
    }
    img
}

fn luma(p: [f32; 4]) -> f32 {
    linear_to_srgb(p[0] / p[3])
}

fn row(img: &Image, y: usize) -> Vec<[f32; 4]> {
    (0..img.w).map(|x| img.get(x, y)).collect()
}

fn col(img: &Image, x: usize) -> Vec<[f32; 4]> {
    (0..img.h).map(|y| img.get(x, y)).collect()
}

fn multiset(mut v: Vec<[f32; 4]>) -> Vec<[f32; 4]> {
    v.sort_by(|a, b| a.iter().zip(b).map(|(x, y)| x.total_cmp(y)).find(|o| o.is_ne()).unwrap_or(std::cmp::Ordering::Equal));
    v
}

#[test]
fn rows_are_sorted_and_keep_their_pixels() {
    let img = scrambled(16, 6);
    let out = render(&all(&[]), &img);
    for y in 0..6 {
        let r = row(&out, y);
        assert!(r.windows(2).all(|w| luma(w[0]) <= luma(w[1])), "row {y}");
        assert_eq!(multiset(r), multiset(row(&img, y)), "row {y} keeps its pixels");
    }
    // descending
    let out = render(&all(&[("order", ch(1))]), &img);
    assert!(row(&out, 2).windows(2).all(|w| luma(w[0]) >= luma(w[1])));
}

#[test]
fn an_already_sorted_picture_is_unchanged() {
    let mut img = Image::new(12, 4);
    for (i, p) in img.px.chunks_exact_mut(4).enumerate() {
        let v = (i % 12) as f32 / 12.0;
        p.copy_from_slice(&[v, v * 0.5, v * 0.2, 1.0]);
    }
    assert_eq!(render(&all(&[]), &img).px, img.px);
}

#[test]
fn pixels_outside_the_band_stay_put_and_split_the_runs() {
    // dark / bright / dark: the bright middle pixel is outside a dark-only band
    let mut img = Image::new(7, 1);
    let vals = [0.3f32, 0.1, 0.2, 0.95, 0.25, 0.05, 0.15];
    for (p, v) in img.px.chunks_exact_mut(4).zip(vals) {
        p.copy_from_slice(&[v, v, v, 1.0]);
    }
    let out = render(&sort(&[("lower", fl(0.0)), ("upper", fl(80.0))]), &img);
    let got: Vec<f32> = out.px.chunks_exact(4).map(|p| p[0]).collect();
    assert_eq!(got, [0.1, 0.2, 0.3, 0.95, 0.05, 0.15, 0.25]);
    // an empty band (lower above upper) changes nothing
    assert_eq!(render(&sort(&[("lower", fl(90.0)), ("upper", fl(10.0))]), &img).px, img.px);
}

#[test]
fn ninety_degrees_sorts_columns_and_180_mirrors_zero() {
    let img = scrambled(10, 12);
    let up = render(&all(&[("angle", fl(90.0))]), &img);
    // 90°: lines run up the screen, so each column is sorted from the bottom
    for x in 0..10 {
        let c = col(&up, x);
        assert!(c.windows(2).all(|w| luma(w[0]) >= luma(w[1])), "column {x}");
        assert_eq!(multiset(c), multiset(col(&img, x)));
    }
    // 180° on the picture equals 0° on its mirror image, mirrored back
    let mirror = |im: &Image| {
        let mut m = im.clone();
        for y in 0..im.h {
            for x in 0..im.w {
                let p = im.get(im.w - 1 - x, y);
                m.px[(y * im.w + x) * 4..][..4].copy_from_slice(&p);
            }
        }
        m
    };
    let a = render(&all(&[("angle", fl(180.0))]), &img);
    let b = mirror(&render(&all(&[]), &mirror(&img)));
    assert_eq!(a.px, b.px);
}

#[test]
fn diagonal_lines_keep_every_pixel() {
    let img = scrambled(20, 14);
    let out = render(&all(&[("angle", fl(37.0))]), &img);
    let mut a: Vec<[f32; 4]> = out.px.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]).collect();
    let mut b: Vec<[f32; 4]> = img.px.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]).collect();
    a = multiset(a);
    b = multiset(b);
    assert_eq!(a, b);
    assert_ne!(out.px, img.px);
}

#[test]
fn hue_and_saturation_keys_and_blend() {
    let mut img = Image::new(6, 1);
    let cols = [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.5, 0.4, 0.4], [0.9, 0.1, 0.1], [0.3, 0.3, 0.9]];
    for (p, c) in img.px.chunks_exact_mut(4).zip(cols) {
        p.copy_from_slice(&[c[0], c[1], c[2], 1.0]);
    }
    for by in 1..=2 {
        let out = render(&all(&[("sort_by", ch(by))]), &img);
        let key = |p: [f32; 4]| cfa4_color::rgb_to_hsl(linear_to_srgb(p[0]), linear_to_srgb(p[1]), linear_to_srgb(p[2]))[by as usize - 1];
        assert!(row(&out, 0).windows(2).all(|w| key(w[0]) <= key(w[1]) + 1e-6), "key {by}");
    }
    // full blend gives the original back
    assert_eq!(render(&all(&[("blend", fl(100.0))]), &img).px, img.px);
}

#[test]
fn transparent_pixels_split_runs_and_stay_put() {
    let mut img = scrambled(9, 1);
    img.px[4 * 4..4 * 4 + 4].copy_from_slice(&[0.0; 4]);
    let out = render(&all(&[]), &img);
    assert_eq!(&out.px[16..20], &[0.0; 4]);
    assert!(out.px.chunks_exact(4).enumerate().all(|(i, p)| i == 4 || p[3] == 1.0));
}

#[test]
fn hostile_parameters_neither_panic_nor_produce_non_finite_values() {
    let img = scrambled(14, 9);
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1e30, 1e30, 0.0] {
        for k in ["angle", "lower", "upper", "blend"] {
            for by in [0, 1, 2, 99] {
                let out = render(&all(&[("sort_by", ch(by)), ("order", ch(by)), (k, fl(v))]), &img);
                assert!(out.px.iter().all(|x| x.is_finite()), "{k} = {v}, key {by}");
            }
        }
    }
    for (w, h) in [(1, 1), (1, 5), (3, 1)] {
        let out = render(&all(&[("angle", fl(45.0))]), &Image::filled(w, h, [0.2, 0.3, 0.4, 1.0]));
        assert!(out.px.iter().all(|x| x.is_finite()));
    }
    assert_eq!(render(&all(&[]), &Image::new(6, 6)).px, Image::new(6, 6).px);
}
