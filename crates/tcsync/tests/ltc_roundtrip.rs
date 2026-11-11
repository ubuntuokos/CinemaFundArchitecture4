//! LTC encoder → decoder round trips: every LTC rate, both polarities, reverse play, speed changes,
//! noise, low level, and hostile input (plan TC-1).

use cfa4_tcsync::ltc::{LtcDecoded, LtcDecoder, LtcEncoder, LtcFrame};
use cfa4_time::FrameRate;

fn encode_run(sample_rate: u32, rate: FrameRate, drop: bool, first: i64, count: i64, level: f32) -> (Vec<f32>, Vec<LtcFrame>) {
    let mut enc = LtcEncoder::new(sample_rate, rate, level).expect("encoder");
    let mut out = Vec::new();
    let mut frames = Vec::new();
    for n in first..first + count {
        let mut f = LtcFrame::from_frame_number(n, rate, drop);
        f.user_bits = (n as u32).wrapping_mul(0x9E37_79B9);
        enc.encode(&f, &mut out);
        frames.push(f);
    }
    (out, frames)
}

fn decode_all(sample_rate: u32, samples: &[f32], chunk: usize) -> Vec<LtcDecoded> {
    let mut dec = LtcDecoder::new(sample_rate).expect("decoder");
    let mut got = Vec::new();
    for c in samples.chunks(chunk.max(1)) {
        dec.decode(c, &mut got);
    }
    got
}

/// The decoder needs the first frame to find the bit timing, and a frame completes only at the
/// first edge of the next one, so the first and the last frame may be missing; every frame in
/// between must come back, in order.
fn assert_tail(got: &[LtcDecoded], want: &[LtcFrame], reverse: bool) {
    assert!(got.len() + 2 >= want.len(), "decoded {} of {} frames", got.len(), want.len());
    let Some(first) = got.first() else { return };
    let Some(skip) = want.iter().position(|w| *w == first.frame) else {
        panic!("decoded a frame that was not sent: {first:?}");
    };
    assert!(skip <= 1, "lost {skip} frames at the start");
    for (g, w) in got.iter().zip(&want[skip..]) {
        assert_eq!(&g.frame, w);
        assert_eq!(g.reverse, reverse);
    }
}

const RATES: [(FrameRate, bool); 6] = [
    (FrameRate::FPS_23_976, false),
    (FrameRate::FPS_24, false),
    (FrameRate::FPS_25, false),
    (FrameRate::FPS_29_97, true),
    (FrameRate::FPS_29_97, false),
    (FrameRate::FPS_30, false),
];

#[test]
fn round_trip_every_rate_and_sample_rate() {
    for sr in [44_100, 48_000, 96_000] {
        for (rate, drop) in RATES {
            // Start just before a minute boundary so drop-frame labels are exercised.
            let (s, want) = encode_run(sr, rate, drop, 1795, 60, -18.0);
            let got = decode_all(sr, &s, 512);
            assert_tail(&got, &want, false);
        }
    }
}

#[test]
fn frame_timing_matches_the_rate() {
    let sr = 48_000;
    let (s, _) = encode_run(sr, FrameRate::FPS_25, false, 0, 50, -18.0);
    let got = decode_all(sr, &s, 4096);
    for d in &got {
        // 25 fps at 48 kHz: 1920 samples per frame (edge detection may shift by a few samples).
        assert!((1900..=1940).contains(&d.duration()), "{}", d.duration());
    }
    for w in got.windows(2) {
        let gap = w[1].start.abs_diff(w[0].end);
        assert!(gap <= 4, "frames not contiguous: {gap}");
    }
}

#[test]
fn inverted_polarity() {
    let (s, want) = encode_run(48_000, FrameRate::FPS_30, false, 100, 30, -18.0);
    let inv: Vec<f32> = s.iter().map(|x| -x).collect();
    assert_tail(&decode_all(48_000, &inv, 333), &want, false);
}

#[test]
fn reverse_play() {
    let (mut s, mut want) = encode_run(48_000, FrameRate::FPS_25, false, 5000, 40, -18.0);
    s.reverse();
    want.reverse();
    assert_tail(&decode_all(48_000, &s, 1000), &want, true);
}

#[test]
fn varying_speed() {
    // Encoding at another sample rate than the decoder assumes is play at another speed.
    for (enc_sr, speed) in [(96_000, "0.5x"), (24_000, "2x"), (16_000, "3x"), (192_000, "0.25x")] {
        let (s, want) = encode_run(enc_sr, FrameRate::FPS_25, false, 0, 30, -18.0);
        let got = decode_all(48_000, &s, 777);
        assert!(!got.is_empty(), "{speed}");
        assert_tail(&got, &want, false);
    }
}

#[test]
fn speed_change_mid_stream_recovers() {
    let (a, _) = encode_run(48_000, FrameRate::FPS_25, false, 0, 20, -18.0);
    let (b, want_b) = encode_run(36_000, FrameRate::FPS_25, false, 20, 20, -18.0); // 1.33×
    let mut s = a;
    s.extend_from_slice(&b);
    let got = decode_all(48_000, &s, 640);
    let tail: Vec<_> = got.iter().filter(|d| d.frame.frame_number(FrameRate::FPS_25) >= 22).collect();
    // Frames 22..=38: the last frame completes only with a following edge.
    assert_eq!(tail.len(), 17, "frames after the speed change");
    for d in tail {
        assert_eq!(d.frame, want_b[(d.frame.frame_number(FrameRate::FPS_25) - 20) as usize]);
    }
}

/// Deterministic noise (xorshift).
fn noise(seed: &mut u32) -> f32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    (*seed as f32 / u32::MAX as f32) * 2.0 - 1.0
}

#[test]
fn noise_low_level_and_dc_offset() {
    let (s, want) = encode_run(48_000, FrameRate::FPS_29_97, true, 0, 60, -40.0);
    let amp = 10f32.powf(-40.0 / 20.0);
    let mut seed = 0x1234_5678;
    let noisy: Vec<f32> = s.iter().map(|x| x + 0.1 * amp * noise(&mut seed) + 0.05 * amp).collect();
    assert_tail(&decode_all(48_000, &noisy, 256), &want, false);
}

#[test]
fn silence_gap_resyncs() {
    let (a, _) = encode_run(48_000, FrameRate::FPS_24, false, 0, 10, -18.0);
    let (b, want_b) = encode_run(48_000, FrameRate::FPS_24, false, 1000, 10, -18.0);
    let mut s = a;
    s.extend(std::iter::repeat_n(0.0, 48_000));
    s.extend_from_slice(&b);
    let got = decode_all(48_000, &s, 4800);
    let after: Vec<_> = got.iter().filter(|d| d.frame.frame_number(FrameRate::FPS_24) >= 1000).map(|d| d.frame).collect();
    assert!(after.len() >= 9);
    assert_eq!(after.last(), want_b.get(want_b.len() - 2));
}

#[test]
fn hostile_input_never_panics() {
    let (clean, _) = encode_run(48_000, FrameRate::FPS_25, false, 0, 8, -18.0);
    let mut seed = 0xC0FF_EE11u32;
    for round in 0..200 {
        let mut s = clean.clone();
        // Bit flips in the sample bits, truncation, non-finite values, huge values.
        for _ in 0..(round % 50) {
            let i = (noise(&mut seed).abs() * (s.len() - 1) as f32) as usize;
            if let Some(x) = s.get_mut(i) {
                let bit = seed % 32;
                *x = f32::from_bits(x.to_bits() ^ (1 << bit));
            }
        }
        let cut = (noise(&mut seed).abs() * s.len() as f32) as usize;
        s.truncate(cut.max(1));
        if round % 7 == 0 {
            s.push(f32::NAN);
            s.push(f32::INFINITY);
            s.push(f32::MAX);
        }
        let r = std::panic::catch_unwind(|| decode_all(48_000, &s, 97));
        assert!(r.is_ok(), "round {round} panicked");
    }
    // Pure noise.
    let s: Vec<f32> = (0..200_000).map(|_| noise(&mut seed)).collect();
    let r = std::panic::catch_unwind(|| decode_all(48_000, &s, 1024));
    assert!(r.is_ok());
}
