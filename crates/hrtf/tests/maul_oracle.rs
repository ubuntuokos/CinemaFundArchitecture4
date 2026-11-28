// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the CFA4 contributors

//! The Rust port against Maul Audio's C library (CFA4-BINAURAL-MONITOR-001 §7 2a): the same
//! moving source rendered by `tests/oracle/maul_harness.c` (fixtures in `tests/data/`, the first
//! 4096 stereo frames). On the machine that made the fixtures the full 48000-frame renders were
//! bit-identical (2026-11-27, clang 18 -O2 against this crate in release). The tests allow 1e-6
//! (1e-5 for the resampled set, whose kernel is thousands of f64 `sin`/`cos` calls) so that another
//! machine's libm rounding cannot fail them: the CI runner's resampled render differed by 1.5e-6,
//! about -110 dB below the signal.

use std::sync::Arc;

use cfa4_hrtf::{Binaural, BinauralConfig, HrtfSet};

fn scene(rate: u32, near_field: bool, distance: f32) -> Vec<f32> {
    let s = Arc::new(HrtfSet::builtin(rate).unwrap());
    let mut b = Binaural::new(s, BinauralConfig { near_field, ..BinauralConfig::default() }).unwrap();
    let n = 4096usize;
    let mut st = 0x1234_5678u64;
    let x: Vec<f32> = (0..n)
        .map(|_| {
            st ^= st << 13;
            st ^= st >> 7;
            st ^= st << 17;
            (st >> 40) as f32 / (1u64 << 24) as f32 - 0.5
        })
        .collect();
    let mut l = vec![0.0f32; n];
    let mut r = vec![0.0f32; n];
    for i in (0..n).step_by(64) {
        // the harness's 3.14159265f rounds to the same f32 as PI
        let az = 2.0f32 * std::f32::consts::PI * (i as f32) / 24000.0;
        let el = 0.3f32 * ((i as f32) / 5000.0).sin();
        let p = [-az.sin() * el.cos() * distance, el.sin() * distance, -az.cos() * el.cos() * distance];
        b.process(p, 0.8, &x[i..i + 64], &mut l[i..i + 64], &mut r[i..i + 64]);
    }
    l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]).collect()
}

fn fixture(bytes: &[u8]) -> Vec<f32> {
    bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn check(name: &str, want: &[u8], got: Vec<f32>, tolerance: f32) {
    let want = fixture(want);
    assert_eq!(want.len(), got.len(), "{name}");
    let worst = want.iter().zip(&got).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(worst < tolerance, "{name}: largest difference from Maul Audio {worst:e}");
}

#[test]
fn moving_near_source_matches_maul_audio() {
    check("48 kHz, near field, 0.4 m", include_bytes!("data/maul-1_0.4_48000.f32"), scene(48_000, true, 0.4), 1e-6);
}

#[test]
fn resampled_set_matches_maul_audio() {
    check("44.1 kHz, near field, 0.4 m", include_bytes!("data/maul-1_0.4_44100.f32"), scene(44_100, true, 0.4), 1e-5);
}

#[test]
fn far_source_without_near_field_matches_maul_audio() {
    check("48 kHz, no near field, 2 m", include_bytes!("data/maul-0_2.0_48000.f32"), scene(48_000, false, 2.0), 1e-6);
}
