use super::*;
use crate::{Decoder, Mode, parse_header};

/// A test signal per channel: two tones (different per channel) and a little noise.
fn signal(rate: u32, channels: usize, n: usize) -> Vec<Vec<f32>> {
    noisy(rate, channels, n, 0.005)
}

/// [`signal`] with noise of amplitude `noise`.
fn noisy(rate: u32, channels: usize, n: usize, noise: f32) -> Vec<Vec<f32>> {
    let mut seed = 0x1234_5678u32;
    (0..channels)
        .map(|ch| {
            (0..n)
                .map(|i| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let t = i as f32 / rate as f32;
                    let f = if ch == 0 { 440.0 } else { 660.0 };
                    0.4 * (2.0 * std::f32::consts::PI * f * t).sin()
                        + 0.1 * (2.0 * std::f32::consts::PI * 3100.0 * t).sin()
                        + noise * ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5)
                })
                .collect()
        })
        .collect()
}

fn encode_all(cfg: Config, input: &[Vec<f32>]) -> Vec<Vec<u8>> {
    let mut enc = Encoder::new(cfg).unwrap();
    let refs: Vec<&[f32]> = input.iter().map(Vec::as_slice).collect();
    // in uneven pieces, to exercise the buffering
    let mut frames = Vec::new();
    let mut at = 0;
    let n = input[0].len();
    let mut piece = 1000;
    while at < n {
        let end = (at + piece).min(n);
        let parts: Vec<&[f32]> = refs.iter().map(|c| &c[at..end]).collect();
        frames.extend(enc.encode(&parts).unwrap());
        at = end;
        piece = piece * 7 % 3001 + 1;
    }
    frames.extend(enc.flush());
    frames
}

fn decode_all(frames: &[Vec<u8>], channels: usize) -> Vec<Vec<f32>> {
    let mut dec = Decoder::new();
    let mut out = vec![Vec::new(); channels];
    for f in frames {
        let d = dec.decode(f).unwrap();
        for (o, c) in out.iter_mut().zip(d.channels) {
            o.extend(c);
        }
    }
    out
}

/// SNR (dB) of `decoded` against `input` delayed by [`DELAY`], over the middle of the signal.
fn snr(input: &[f32], decoded: &[f32]) -> f64 {
    let (mut s, mut e) = (0f64, 0f64);
    for (i, &x) in input.iter().enumerate().skip(2000).take(input.len().saturating_sub(4000)) {
        let y = decoded[i + DELAY];
        s += f64::from(x) * f64::from(x);
        e += f64::from(x - y) * f64::from(x - y);
    }
    10.0 * (s / e.max(1e-30)).log10()
}

/// Analysis followed by the decoder's synthesis, unquantized: the input comes back DELAY samples
/// later (the pair is nearly a perfect reconstruction filter bank).
#[test]
fn analysis_then_synthesis_reconstructs_the_input() {
    let input = signal(48_000, 1, 48_000).remove(0);
    let mut x = [0f32; 512];
    let mut v = [0f32; 1024];
    let mut out = Vec::new();
    for chunk in input.chunks(32) {
        let mut s = [0f32; 32];
        analyse(&mut x, chunk, &mut s);
        let mut o = [0f32; 32];
        crate::synthesize(&mut v, tables::matrix(), tables::window(), &s, &mut o);
        out.extend_from_slice(&o);
    }
    out.resize(input.len() + DELAY, 0.0);
    let r = snr(&input, &out);
    assert!(r > 80.0, "filter bank SNR {r:.1} dB");
}

#[test]
fn stereo_and_mono_round_trip_through_our_decoder() {
    for (rate, channels, kbps, min_snr) in
        [(48_000, 2, 192, 40.0), (44_100, 2, 128, 32.0), (32_000, 1, 64, 35.0), (48_000, 1, 192, 52.0), (24_000, 2, 96, 33.0)]
    {
        // tones only: a noise floor costs bits the model rightly spends on it
        let input = noisy(rate, channels, rate as usize * 2, 0.0);
        let frames = encode_all(Config::new(rate, channels, kbps), &input);
        let decoded = decode_all(&frames, channels);
        for (ch, (i, d)) in input.iter().zip(&decoded).enumerate() {
            assert!(d.len() >= i.len() + DELAY);
            let r = snr(i, d);
            assert!(r > min_snr, "{rate} Hz {channels} ch {kbps} kbit/s channel {ch}: SNR {r:.1} dB");
        }
        let h = parse_header(&frames[0]).unwrap();
        assert_eq!((h.layer, h.sample_rate, h.bitrate_kbps, h.channels()), (2, rate, kbps, channels));
        assert_eq!(h.mode, if channels == 1 { Mode::Mono } else { Mode::Stereo });
    }
}

/// Every sampling frequency, channel count and allowed bit rate: frames parse with the right
/// length, decode, and the padding keeps the average bit rate exact.
#[test]
fn every_rate_and_bit_rate_makes_valid_frames() {
    for rate in [32_000, 44_100, 48_000, 16_000, 22_050, 24_000] {
        for channels in [1, 2] {
            let rates = bitrates(rate, channels);
            assert!(!rates.is_empty());
            for kbps in rates {
                let input = signal(rate, channels, FRAME_SAMPLES * 20);
                let frames = encode_all(Config::new(rate, channels, kbps), &input);
                let mut dec = Decoder::new();
                let mut bytes = 0usize;
                for f in &frames {
                    let h = parse_header(f).unwrap();
                    assert_eq!(h.frame_bytes, f.len(), "{rate} {channels} {kbps}");
                    dec.decode(f).unwrap();
                    bytes += f.len();
                }
                let want = frames.len() as f64 * FRAME_SAMPLES as f64 / rate as f64 * kbps as f64 * 1000.0 / 8.0;
                assert!((bytes as f64 - want).abs() <= 1.0, "{rate} {channels} {kbps}: {bytes} bytes, want {want:.1}");
            }
        }
    }
}

#[test]
fn rejects_what_layer_two_cannot_carry() {
    assert!(Encoder::new(Config::new(96_000, 2, 192)).is_err());
    assert!(Encoder::new(Config::new(48_000, 3, 192)).is_err());
    assert!(Encoder::new(Config::new(48_000, 0, 192)).is_err());
    assert!(Encoder::new(Config::new(48_000, 2, 100)).is_err());
    // 11172-3: no mono above 192 kbit/s, no stereo below 64 or at 80 kbit/s
    assert!(Encoder::new(Config::new(48_000, 1, 256)).is_err());
    assert!(Encoder::new(Config::new(48_000, 2, 48)).is_err());
    assert!(Encoder::new(Config::new(48_000, 2, 80)).is_err());
    assert!(Encoder::new(Config::new(24_000, 2, 8)).is_ok());
    let mut enc = Encoder::new(Config::new(48_000, 2, 192)).unwrap();
    assert!(enc.encode(&[&[0.0; 10]]).is_err());
    assert!(enc.encode(&[&[0.0; 10], &[0.0; 9]]).is_err());
}

/// Silence, full-scale square waves and non-finite samples encode to decodable frames.
#[test]
fn extreme_input_never_breaks_the_frames() {
    let n = FRAME_SAMPLES * 4;
    let square: Vec<f32> = (0..n).map(|i| if i / 20 % 2 == 0 { 1.0 } else { -1.0 }).collect();
    let wild: Vec<f32> = (0..n).map(|i| [f32::NAN, f32::INFINITY, -f32::INFINITY, 1e30, -1e30, 0.5][i % 6]).collect();
    for input in [vec![vec![0.0; n], vec![0.0; n]], vec![square.clone(), square], vec![wild.clone(), wild]] {
        let mut cfg = Config::new(44_100, 2, 384);
        cfg.crc = true;
        for f in encode_all(cfg, &input) {
            let d = Decoder::new().decode(&f).unwrap();
            assert!(d.channels.iter().flatten().all(|v| v.is_finite()));
        }
    }
}

#[test]
fn dual_channel_and_crc_are_signalled() {
    let mut cfg = Config::new(48_000, 2, 256);
    cfg.stereo = StereoMode::DualChannel;
    cfg.crc = true;
    let input = signal(48_000, 2, FRAME_SAMPLES * 3);
    let frames = encode_all(cfg, &input);
    let h = parse_header(&frames[0]).unwrap();
    assert_eq!(h.mode, Mode::DualChannel);
    assert!(h.crc);
}

/// The CRC-16 of the standard (generator 0x8005, all-ones start, no reflection, no final XOR:
/// the catalogued "CRC-16/CMS") has the check value 0xAEE7 for "123456789".
#[test]
fn crc16_matches_the_reference_check_value() {
    assert_eq!(crc16(b"123456789", 72), 0xAEE7);
}

#[test]
fn scalefactor_selection_merges_close_thirds() {
    assert_eq!(select_scalefactors([10, 11, 12]), (2, [10, 10, 10]));
    assert_eq!(select_scalefactors([10, 11, 30]), (1, [10, 10, 30]));
    assert_eq!(select_scalefactors([30, 10, 11]), (3, [30, 10, 10]));
    assert_eq!(select_scalefactors([0, 20, 40]), (0, [0, 20, 40]));
    assert_eq!(scalefactor_index(2.5), 0);
    assert_eq!(scalefactor_index(0.0), 62);
    assert!(tables::scalefactor(scalefactor_index(0.3)) >= 0.3);
    assert!(tables::scalefactor(scalefactor_index(0.3) + 1) < 0.3);
}
