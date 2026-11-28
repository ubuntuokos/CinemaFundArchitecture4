// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the CFA4 contributors

//! B-01 tests (CFA4-BINAURAL-MONITOR-001 §7): the built-in set loads, malformed files are errors,
//! the renderer is symmetric, lateral sources give the expected level and time differences, a
//! moving source does not click, the FIR equals a direct convolution, resampled sets keep their
//! response, and the virtual loudspeakers place channels and the LFE.

use std::f64::consts::PI;
use std::sync::Arc;

use cfa4_hrtf::{Binaural, BinauralConfig, Error, HrtfSet, SpeakerLayout, VirtualSpeakers, VirtualSpeakersConfig, near_field, position_of};

const SR: u32 = 48_000;

fn set() -> Arc<HrtfSet> {
    Arc::new(HrtfSet::builtin(SR).unwrap())
}

fn sine(freq: f64, n: usize, rate: u32) -> Vec<f32> {
    (0..n).map(|i| (2.0 * PI * freq * i as f64 / f64::from(rate)).sin() as f32 * 0.5).collect()
}

fn noise(n: usize) -> Vec<f32> {
    let mut s = 0x1234_5678u64;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            ((s >> 40) as f32 / (1u64 << 24) as f32) - 0.5
        })
        .collect()
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|&v| f64::from(v) * f64::from(v)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-30).log10()
}

/// Amplitude of `freq` in `x` (single-bin DFT).
fn tone(x: &[f32], freq: f64, rate: u32) -> f64 {
    let (mut re, mut im) = (0.0, 0.0);
    for (i, &v) in x.iter().enumerate() {
        let w = 2.0 * PI * freq * i as f64 / f64::from(rate);
        re += f64::from(v) * w.cos();
        im -= f64::from(v) * w.sin();
    }
    2.0 * (re * re + im * im).sqrt() / x.len() as f64
}

fn render(b: &mut Binaural, pos: [f32; 3], input: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0; input.len()];
    let mut r = vec![0.0; input.len()];
    b.process(pos, 1.0, input, &mut l, &mut r);
    (l, r)
}

fn at(az: f32, el: f32) -> [f32; 3] {
    position_of(az, el, 2.0)
}

#[test]
fn builtin_set_loads_with_its_licence() {
    let s = HrtfSet::builtin(SR).unwrap();
    assert_eq!(s.sample_rate(), 48_000);
    assert_eq!(s.taps(), 128);
    assert!(s.direction_count() > 1000, "{}", s.direction_count());
    assert!(s.license().contains("Apache License"), "{}", s.license());
    assert!(s.name().contains("SADIE"), "{}", s.name());
    assert!((0.5..=3.0).contains(&s.distance()), "{}", s.distance());
}

#[test]
fn malformed_files_are_errors_never_panics() {
    let good = include_bytes!("../../../assets/hrtf/sadie2-ku100-48k.maudhrtf");
    assert!(HrtfSet::from_maudhrtf(&good[..], None).is_ok());
    assert!(matches!(HrtfSet::from_maudhrtf(b"", None), Err(Error::Format(_))));
    assert!(matches!(HrtfSet::from_maudhrtf(b"MAUDHRTF", None), Err(Error::Format(_))));
    let mut v = good.to_vec();
    v[8] = 9;
    assert_eq!(HrtfSet::from_maudhrtf(&v, None).unwrap_err(), Error::Version);
    // a flipped payload byte fails the checksum; truncations fail somewhere
    let mut rng = 7u64;
    for _ in 0..200 {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let mut v = good.to_vec();
        let i = 16 + (rng >> 33) as usize % (v.len() - 16);
        v[i] ^= 1 << (rng % 8);
        assert!(HrtfSet::from_maudhrtf(&v, None).is_err());
        let cut = (rng >> 20) as usize % good.len();
        assert!(HrtfSet::from_maudhrtf(&good[..cut], None).is_err());
    }
    assert!(matches!(HrtfSet::builtin(1000), Err(Error::Invalid(_))));
}

/// Broadband level of an ear's blended response for a direction (white noise in → this ratio out).
fn response_level(s: &HrtfSet, az: f32, el: f32, ear: usize) -> f64 {
    let mut h = vec![0.0; s.taps()];
    s.blend(&s.neighbours(az, el), ear, &mut h);
    h.iter().map(|&v| f64::from(v) * f64::from(v)).sum::<f64>().sqrt()
}

#[test]
fn the_renderer_adds_no_asymmetry_of_its_own() {
    // The KU100 measurement itself is not exactly symmetric straight ahead (about -1.1 dB and two
    // samples of onset difference: the head sat slightly turned). The renderer must reproduce the
    // set's own left/right ratio, adding none of its own.
    let s = set();
    let x = noise(48_000);
    for (az, el) in [(0.0, 0.0), (180.0, 0.0), (0.0, 60.0), (45.0, -20.0)] {
        let mut b = Binaural::new(s.clone(), BinauralConfig { near_field: false, ..BinauralConfig::default() }).unwrap();
        let (l, r) = render(&mut b, at(az, el), &x);
        let got = db(rms(&l[4800..]) / rms(&r[4800..]));
        let want = db(response_level(&s, az, el, 0) / response_level(&s, az, el, 1));
        assert!((got - want).abs() < 0.5, "{az}/{el}: rendered L/R {got:.2} dB, set {want:.2} dB");
        if (az, el) == (0.0, 0.0) || (az, el) == (0.0, 60.0) {
            assert!(got.abs() < 2.0, "{az}/{el}: L/R {got:.2} dB");
        }
    }
}

#[test]
fn lateral_source_level_and_time_differences() {
    // ILD at 4 kHz for a source on the left
    let x = sine(4000.0, 48_000, SR);
    let mut b = Binaural::new(set(), BinauralConfig::default()).unwrap();
    let (l, r) = render(&mut b, at(90.0, 0.0), &x);
    let ild = db(tone(&l[4800..], 4000.0, SR) / tone(&r[4800..], 4000.0, SR));
    assert!(ild > 10.0, "ILD at 4 kHz {ild:.1} dB");
    // the right side mirrors it
    let mut b = Binaural::new(set(), BinauralConfig::default()).unwrap();
    let (l, r) = render(&mut b, at(-90.0, 0.0), &x);
    let ild_r = db(tone(&r[4800..], 4000.0, SR) / tone(&l[4800..], 4000.0, SR));
    // the measured head is not perfectly symmetric (15.5 against 19.7 dB at 4 kHz on this set)
    assert!(ild_r > 10.0 && (ild - ild_r).abs() < 6.0, "left {ild:.1} dB, right {ild_r:.1} dB");

    // ITD: cross-correlation of low-passed ears for an impulse-like noise burst
    let x = noise(9600);
    let mut b = Binaural::new(set(), BinauralConfig::default()).unwrap();
    let (l, r) = render(&mut b, at(90.0, 0.0), &x);
    let lp = |v: &[f32]| -> Vec<f32> {
        let mut y = 0.0f32;
        v.iter()
            .map(|&s| {
                y += 0.1 * (s - y);
                y
            })
            .collect()
    };
    let (l, r) = (lp(&l), lp(&r));
    let (mut best, mut lag) = (f64::MIN, 0i32);
    for k in -60i32..=60 {
        let mut c = 0.0;
        for n in 200..9000usize {
            let m = n as i32 + k;
            c += f64::from(l[n]) * f64::from(r[m as usize]);
        }
        if c > best {
            best = c;
            lag = k;
        }
    }
    let itd_ms = f64::from(lag) / f64::from(SR) * 1000.0;
    // The KU100's interaural delay at 90 degrees is about 0.8 ms (the set's onset delays differ by
    // 37.8 samples there; the minimum-phase responses add a little): physiologically plausible
    // (0.6 to 0.95 ms) and following the set's own onset difference.
    let s = set();
    let nb = s.neighbours(90.0, 0.0);
    let mut h = vec![0.0; s.taps()];
    let onset = f64::from(s.blend(&nb, 1, &mut h) - s.blend(&nb, 0, &mut h));
    assert!((0.60..=0.95).contains(&itd_ms), "ITD {itd_ms:.3} ms (right ear later by {lag} samples)");
    assert!((f64::from(lag) - onset).abs() < 6.0, "ITD {lag} samples, the set's onset difference {onset:.1}");
}

#[test]
fn moving_source_does_not_click() {
    let n = 96_000;
    let x = sine(500.0, n, SR);
    // the steepest step of the still source anywhere on the horizon
    let mut still = 0.0f32;
    for az in (0..360).step_by(15) {
        let mut b = Binaural::new(set(), BinauralConfig::default()).unwrap();
        let (l, r) = render(&mut b, at(az as f32, 0.0), &x[..9600]);
        for o in [&l, &r] {
            for w in o[2000..].windows(2) {
                still = still.max((w[1] - w[0]).abs());
            }
        }
    }
    // one turn a second, in 64-frame blocks
    let mut b = Binaural::new(set(), BinauralConfig::default()).unwrap();
    let mut l = vec![0.0; n];
    let mut r = vec![0.0; n];
    for (i, chunk) in x.chunks(64).enumerate() {
        let az = 360.0 * (i * 64) as f32 / SR as f32;
        let s = i * 64;
        b.process(at(az, 10.0), 1.0, chunk, &mut l[s..s + chunk.len()], &mut r[s..s + chunk.len()]);
    }
    let mut moving = 0.0f32;
    for o in [&l, &r] {
        for w in o[2000..].windows(2) {
            moving = moving.max((w[1] - w[0]).abs());
        }
    }
    assert!(moving <= 1.5 * still, "moving source steps {moving:.5}, still at most {still:.5}");
}

#[test]
fn fir_equals_direct_convolution_for_a_still_source() {
    let s = set();
    let x = noise(4096);
    let mut b = Binaural::new(s.clone(), BinauralConfig { near_field: false, ..BinauralConfig::default() }).unwrap();
    let pos = at(40.0, 20.0);
    let (l, _) = render(&mut b, pos, &x);
    // the same filter by hand: blended response, integer + fractional delay (cubic), convolution
    let look = [pos[0] / 2.0, pos[1] / 2.0, pos[2] / 2.0];
    let az = (-look[0]).atan2(-look[2]).to_degrees();
    let el = look[1].asin().to_degrees();
    let nb = s.neighbours(az, el);
    let mut h = vec![0.0; s.taps()];
    let delay = s.blend(&nb, 0, &mut h) + 1.0;
    let whole = delay as usize;
    let f = delay - whole as f32;
    let c = [-f * (f - 1.0) * (f - 2.0) / 6.0, (f + 1.0) * (f - 1.0) * (f - 2.0) / 2.0, -(f + 1.0) * f * (f - 2.0) / 2.0, (f + 1.0) * f * (f - 1.0) / 6.0];
    let xi = |i: isize| if i < 0 { 0.0 } else { x.get(i as usize).copied().unwrap_or(0.0) };
    let d: Vec<f32> = (0..x.len() as isize)
        .map(|n| {
            let p = n - whole as isize;
            c[0] * xi(p + 1) + c[1] * xi(p) + c[2] * xi(p - 1) + c[3] * xi(p - 2)
        })
        .collect();
    let mut worst = 0.0f32;
    for n in 0..x.len() {
        let mut y = 0.0f32;
        for (k, hk) in h.iter().enumerate() {
            if n >= k {
                y += hk * d[n - k];
            }
        }
        worst = worst.max((y - l[n]).abs());
    }
    assert!(worst < 1e-5, "largest difference {worst:e}");
}

#[test]
fn block_size_does_not_change_the_output() {
    let x = noise(10_000);
    let pos = at(-70.0, 15.0);
    let mut a = Binaural::new(set(), BinauralConfig::default()).unwrap();
    let (la, ra) = render(&mut a, pos, &x);
    let mut b = Binaural::new(set(), BinauralConfig { max_frames: 37, ..BinauralConfig::default() }).unwrap();
    let (lb, rb) = render(&mut b, pos, &x);
    assert_eq!(la, lb);
    assert_eq!(ra, rb);
}

#[test]
fn resampled_sets_keep_their_response() {
    let base = HrtfSet::builtin(48_000).unwrap();
    for rate in [44_100u32, 96_000] {
        let s = HrtfSet::builtin(rate).unwrap();
        assert_eq!(s.sample_rate(), rate);
        // the 1 kHz level for a source ahead stays within 0.5 dB
        let x48 = sine(1000.0, 48_000, 48_000);
        let xr = sine(1000.0, rate as usize, rate);
        let mut b48 = Binaural::new(Arc::new(base.clone()), BinauralConfig::default()).unwrap();
        let mut br = Binaural::new(Arc::new(s), BinauralConfig::default()).unwrap();
        let (l48, _) = render(&mut b48, at(30.0, 0.0), &x48);
        let (lr, _) = render(&mut br, at(30.0, 0.0), &xr);
        let d = db(tone(&lr[rate as usize / 10..], 1000.0, rate) / tone(&l48[4800..], 1000.0, 48_000));
        assert!(d.abs() < 0.5, "{rate} Hz: 1 kHz differs by {d:.2} dB");
    }
}

#[test]
fn near_field_is_identity_at_the_set_distance_and_boosts_a_close_ear() {
    let f = near_field(60.0, 0.0875 / 1.2, 0.0875 / 1.2, 0.0875, 48_000.0);
    assert!((f.b0 - 1.0).abs() < 1e-6 && (f.b1 - f.a1).abs() < 1e-6, "{f:?}");
    // a source 0.2 m to the left: the left ear's lows rise against the same direction far away
    let x = sine(300.0, 48_000, SR);
    let mut near = Binaural::new(set(), BinauralConfig::default()).unwrap();
    let mut far = Binaural::new(set(), BinauralConfig::default()).unwrap();
    let (ln, rn) = render(&mut near, position_of(90.0, 0.0, 0.2), &x);
    let (lf, rf) = render(&mut far, position_of(90.0, 0.0, 2.0), &x);
    let ild_near = db(tone(&ln[4800..], 300.0, SR) / tone(&rn[4800..], 300.0, SR));
    let ild_far = db(tone(&lf[4800..], 300.0, SR) / tone(&rf[4800..], 300.0, SR));
    assert!(ild_near > ild_far + 3.0, "300 Hz ILD near {ild_near:.1} dB, far {ild_far:.1} dB");
}

#[test]
fn non_finite_parameters_keep_the_last_good_ones() {
    let x = noise(2048);
    let mut b = Binaural::new(set(), BinauralConfig::default()).unwrap();
    let mut l = vec![0.0; 2048];
    let mut r = vec![0.0; 2048];
    b.process(at(10.0, 0.0), 1.0, &x[..1024], &mut l[..1024], &mut r[..1024]);
    b.process([f32::NAN, 0.0, 0.0], f32::INFINITY, &x[1024..], &mut l[1024..], &mut r[1024..]);
    assert!(l.iter().chain(&r).all(|v| v.is_finite()));
    assert!(rms(&l[1024..]) > 0.01);
}

#[test]
fn virtual_speakers_place_channels_and_the_lfe() {
    let s = set();
    let n = 24_000;
    let x = noise(n);
    let silent = vec![0.0f32; n];
    // centre only: equal ears
    let mut v = VirtualSpeakers::new(s.clone(), SpeakerLayout::surround_5_1(), VirtualSpeakersConfig::default()).unwrap();
    let mut l = vec![0.0; n];
    let mut r = vec![0.0; n];
    v.process(&[&silent, &silent, &x, &silent, &silent, &silent], &mut l, &mut r);
    let d = db(rms(&l[2400..]) / rms(&r[2400..]));
    let want = db(response_level(&s, 0.0, 0.0, 0) / response_level(&s, 0.0, 0.0, 1));
    assert!((d - want).abs() < 0.5, "centre L/R {d:.2} dB, the set's own {want:.2} dB");
    // left surround: left ear louder
    let mut v = VirtualSpeakers::new(s.clone(), SpeakerLayout::surround_5_1(), VirtualSpeakersConfig::default()).unwrap();
    v.process(&[&silent, &silent, &silent, &silent, &x, &silent], &mut l, &mut r);
    assert!(db(rms(&l[2400..]) / rms(&r[2400..])) > 3.0);
    // LFE: identical ears, low-passed, at -10 dB
    let lo = sine(40.0, n, SR);
    let hi = sine(2000.0, n, SR);
    let mut v = VirtualSpeakers::new(s.clone(), SpeakerLayout::surround_5_1(), VirtualSpeakersConfig::default()).unwrap();
    v.process(&[&silent, &silent, &silent, &lo, &silent, &silent], &mut l, &mut r);
    assert_eq!(l, r);
    let g = db(tone(&l[4800..], 40.0, SR) / 0.5);
    assert!((g + 10.0).abs() < 0.5, "LFE gain {g:.2} dB");
    let mut v = VirtualSpeakers::new(s.clone(), SpeakerLayout::surround_5_1(), VirtualSpeakersConfig::default()).unwrap();
    v.process(&[&silent, &silent, &silent, &hi, &silent, &silent], &mut l, &mut r);
    assert!(db(tone(&l[4800..], 2000.0, SR) / 0.5) < -50.0);
    // silence in, silence out; missing channels count as silent
    let mut v = VirtualSpeakers::new(s.clone(), SpeakerLayout::surround_7_1_4(), VirtualSpeakersConfig::default()).unwrap();
    v.process(&[], &mut l, &mut r);
    assert!(l.iter().chain(&r).all(|&v| v == 0.0));
    assert!(SpeakerLayout::for_channels(12).unwrap().channels() == 12);
    assert!(SpeakerLayout::for_channels(7).is_none());
}

#[test]
fn realtime_budget_7_1_4() {
    // 12 channels, one second of 48 kHz audio: must be far below real time even in debug-opt-3
    let s = set();
    let mut v = VirtualSpeakers::new(s, SpeakerLayout::surround_7_1_4(), VirtualSpeakersConfig::default()).unwrap();
    let x = noise(48_000);
    let inputs: Vec<&[f32]> = (0..12).map(|_| &x[..]).collect();
    let mut l = vec![0.0; 48_000];
    let mut r = vec![0.0; 48_000];
    let t = std::time::Instant::now();
    for block in 0..(48_000 / 512) {
        let a = block * 512;
        let ins: Vec<&[f32]> = inputs.iter().map(|i| &i[a..a + 512]).collect();
        v.process(&ins, &mut l[a..a + 512], &mut r[a..a + 512]);
    }
    let secs = t.elapsed().as_secs_f64();
    eprintln!("7.1.4 virtual speakers: 1 s of audio in {:.1} ms", secs * 1000.0);
    assert!(secs < 0.5, "7.1.4 took {secs:.3} s for 1 s of audio");
}
