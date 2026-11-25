//! F2-0: the CFA4 CPU compositor measured with the MQ-1 meter (`cfa4_golden::range_meter`).
//!
//! Synthetic material only (generated in `range_meter::fixture`, known `f64` answer). Run with
//! `--nocapture` to read the figures; they are the F2-0 baseline the candidates are compared with.

use cfa4_golden::range_meter::{Layer, RangeFrame, fixture, measure, truth};
use cfa4_render::{Blend, Image, blend::composite};

const W: usize = 64;
const H: usize = 32;

fn image(l: &Layer) -> Image {
    Image { w: W, h: H, px: l.rgba.clone() }
}

/// The CFA4 compositor on the fixture: layer 0 is the base, the others are composited over it.
fn cfa4_composite(layers: &[Layer], modes: [Blend; 2]) -> RangeFrame {
    let mut dst = image(&layers[0]);
    for (l, m) in layers[1..].iter().zip(modes) {
        composite(&mut dst, &image(l), l.opacity, m);
    }
    RangeFrame { width: W, height: H, rgba: dst.px }
}

#[test]
fn the_cpu_compositor_keeps_out_of_range_values_in_normal_mode() {
    let layers = fixture(W, H).unwrap();
    let frame = cfa4_composite(&layers, [Blend::Normal, Blend::Normal]);
    let r = measure(&truth(&layers), &frame).unwrap();
    println!("F2-0 baseline, CFA4 CPU, Normal over, {W}x{H}, 3 layers (synthetic): {r:?}");
    assert!(r.pass, "{r:?}");
    assert_eq!(r.clamped_samples, 0);
    assert!(r.out_of_range_samples > 1000, "{r:?}");
}

/// Observation, not a defect claim: the other blend modes are defined on display-encoded values
/// and clamp the blend result to [0, 1] on purpose (see `render::blend`). Over an opaque base of 3.0
/// an opaque Screen layer gives at most 1.0 (the highlight is gone), while a half-transparent
/// Normal layer keeps it above 1.0. So MQ-1 holds through Normal, not through the other modes.
#[test]
fn the_other_blend_modes_clamp_by_design() {
    let top = Image::filled(2, 1, [0.5, 0.5, 0.5, 1.0]);
    let mut screen = Image::filled(2, 1, [3.0, 3.0, 3.0, 1.0]);
    composite(&mut screen, &top, 1.0, Blend::Screen);
    let mut normal = Image::filled(2, 1, [3.0, 3.0, 3.0, 1.0]);
    composite(&mut normal, &top, 0.5, Blend::Normal);
    let (s, n) = (screen.px[0], normal.px[0]);
    println!("F2-0 baseline, CFA4 CPU, base 3.0 under an opaque Screen layer: {s}; under a 50% Normal layer: {n} (synthetic)");
    assert!(s <= 1.0 + 1e-6, "{s}");
    assert!(n > 1.0, "{n}");
}
