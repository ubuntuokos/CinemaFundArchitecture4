//! `develop_mosaic` (a mosaic decoded by another decoder, Panasonic RW2 through FFmpeg) develops
//! exactly like a DNG carrying the same samples and colour tags, with or without a camera profile.

use cfa4_dng::color::Calibration;
use cfa4_dng::demosaic::Pattern;
use cfa4_dng::tiff::tags;
use cfa4_dng::writer::{Options, Value, write, write_profile};
use cfa4_dng::{Dng, Error, MAX_MOSAIC_PIXELS, Mosaic, dcp, develop_mosaic};

fn picture(w: usize, h: usize, seed: u32) -> Vec<u16> {
    let mut s = seed;
    (0..w * h)
        .map(|i| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            // a smooth ramp plus noise between the black and white points
            (2048 + (i % w) as u32 * 40 + (s >> 22)).min(60_000) as u16
        })
        .collect()
}

const CM1: [f64; 9] = [0.68, -0.23, -0.05, -0.39, 1.15, 0.27, -0.06, 0.16, 0.62];
const CM2: [f64; 9] = [0.72, -0.21, -0.07, -0.33, 1.08, 0.28, -0.04, 0.13, 0.66];
const FM1: [f64; 9] = [0.65, 0.22, 0.10, 0.27, 0.80, -0.07, 0.03, 0.0, 0.79];

fn colour_tags(neutral: [f64; 3], forward: bool) -> Vec<(u16, Value)> {
    let mut t = vec![
        (tags::COLOR_MATRIX_1, Value::srationals(&CM1)),
        (tags::COLOR_MATRIX_2, Value::srationals(&CM2)),
        (tags::CALIBRATION_ILLUMINANT_1, Value::Short(vec![17])),
        (tags::CALIBRATION_ILLUMINANT_2, Value::Short(vec![21])),
        (tags::AS_SHOT_NEUTRAL, Value::rationals(&neutral)),
        (tags::UNIQUE_CAMERA_MODEL, Value::Ascii("Panasonic DC-TEST".into())),
    ];
    if forward {
        t.push((tags::FORWARD_MATRIX_1, Value::srationals(&FM1)));
    }
    t
}

#[test]
fn develops_like_the_same_dng() {
    let (w, h) = (24, 16);
    let samples = picture(w, h, 7);
    // white balance multipliers 2.25 (red) and 1.75 (blue): neutral = 1 / multiplier
    let neutral = [1.0 / 2.25, 1.0, 1.0 / 1.75];
    // the neutral goes through the writer's 1/10000 rationals: use what the DNG will carry
    let neutral = neutral.map(|v: f64| (v * 10_000.0).round() / 10_000.0);
    for (colors, forward) in [(vec![0, 1, 1, 2], false), (vec![1, 2, 0, 1], true), (vec![2, 1, 1, 0], false)] {
        let o = Options {
            width: w,
            height: h,
            bits: 16,
            cfa: Some((2, 2, colors.clone())),
            main_tags: colour_tags(neutral, forward),
            raw_tags: vec![(tags::BLACK_LEVEL, Value::Long(vec![2048])), (tags::WHITE_LEVEL, Value::Long(vec![60_000]))],
            layout: cfa4_dng::writer::Layout::Strips { rows: h },
            ..Options::default()
        };
        let dng = write(&samples, &o);
        let want = Dng::parse(&dng).unwrap().develop().unwrap();

        // the same colour tags as a camera profile
        let mut profile_tags = colour_tags(neutral, forward);
        profile_tags.retain(|(t, _)| *t != tags::AS_SHOT_NEUTRAL);
        let profile = dcp::parse(&write_profile(&profile_tags, false)).unwrap();
        assert_eq!(profile.camera_model.as_deref(), Some("Panasonic DC-TEST"));
        let m = Mosaic { width: w, height: h, data: &samples, bits: 16, pattern: Pattern { rows: 2, cols: 2, colors }, black: 2048.0, white: 60_000.0 };
        let got = develop_mosaic(&m, neutral, Some(&profile.calibration)).unwrap();
        assert_eq!((got.width, got.height), (want.width, want.height));
        assert_eq!(got.rgba, want.rgba, "mosaic and DNG developments differ");
    }
}

#[test]
fn without_a_profile_the_camera_values_are_white_balanced_only() {
    // a flat grey patch: every site 50 % above black; neutral (0.5, 1, 0.5)
    let (w, h) = (8, 8);
    let data = vec![1024u16; w * h];
    let m = Mosaic { width: w, height: h, data: &data, bits: 12, pattern: Pattern { rows: 2, cols: 2, colors: vec![0, 1, 1, 2] }, black: 0.0, white: 2048.0 };
    let d = develop_mosaic(&m, [0.5, 1.0, 0.5], None).unwrap();
    // red and blue: 0.5 / 0.5 = 1, green 0.5
    for px in d.rgba.chunks_exact(4) {
        assert!((px[0] - 1.0).abs() < 1e-6 && (px[1] - 0.5).abs() < 1e-6 && (px[2] - 1.0).abs() < 1e-6 && px[3] == 1.0, "{px:?}");
    }
    // an empty profile behaves the same
    let e = develop_mosaic(&m, [0.5, 1.0, 0.5], Some(&Calibration::default())).unwrap();
    assert_eq!(e.rgba, d.rgba);
}

#[test]
fn hostile_mosaics_are_errors() {
    let data = vec![0u16; 16];
    let pat = || Pattern { rows: 2, cols: 2, colors: vec![0, 1, 1, 2] };
    let ok = Mosaic { width: 4, height: 4, data: &data, bits: 16, pattern: pat(), black: 0.0, white: 65_535.0 };
    assert!(develop_mosaic(&ok, [1.0; 3], None).is_ok());
    let bad = [
        Mosaic { width: 5, ..Mosaic { pattern: pat(), ..ok } },
        Mosaic { width: 0, height: 0, data: &[], pattern: pat(), ..ok },
        Mosaic { width: MAX_MOSAIC_PIXELS, height: 2, pattern: pat(), ..ok },
        Mosaic { width: usize::MAX, height: 3, pattern: pat(), ..ok },
        Mosaic { bits: 0, pattern: pat(), ..ok },
        Mosaic { black: 70_000.0, pattern: pat(), ..ok },
        Mosaic { white: f64::NAN, pattern: pat(), ..ok },
        Mosaic { pattern: Pattern { rows: 2, cols: 2, colors: vec![0] }, ..ok },
        Mosaic { pattern: Pattern { rows: 0, cols: 2, colors: vec![] }, ..ok },
    ];
    for m in &bad {
        assert!(develop_mosaic(m, [1.0; 3], None).is_err());
    }
    assert!(matches!(develop_mosaic(&ok, [0.0, 1.0, 1.0], None), Err(Error::Malformed(_))));
    assert!(develop_mosaic(&ok, [f64::NAN, 1.0, 1.0], None).is_err());
}
