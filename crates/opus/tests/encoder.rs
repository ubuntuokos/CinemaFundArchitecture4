//! Opus encoder round trips through the crate's own decoder.

use cfa4_opus::{Encoder, StreamDecoder};

fn test_signal(channels: usize, frames: usize, seed: u32) -> Vec<f32> {
    let n = frames * Encoder::FRAME_SIZE;
    let mut s = seed;
    let mut out = Vec::with_capacity(n * channels);
    for i in 0..n {
        let t = i as f32 / 48_000.0;
        for ch in 0..channels {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = ((s >> 9) as f32 / (1u32 << 23) as f32 - 0.5) * 0.02;
            let f = 440.0 * (ch + 1) as f32;
            // A tone plus a click every 250 ms (transients).
            let click = if i % 12_000 < 48 { 0.4 * ((i % 48) as f32 * 0.7).sin() } else { 0.0 };
            out.push(0.3 * (2.0 * std::f32::consts::PI * f * t).sin() + 0.1 * (2.0 * std::f32::consts::PI * 3_150.0 * t).sin() + noise + click);
        }
    }
    out
}

/// SNR in dB of `dec` against `src` after removing the codec delay.
fn snr(src: &[f32], dec: &[f32], channels: usize) -> f64 {
    let d = Encoder::LOOKAHEAD * channels;
    let (mut e, mut s) = (0f64, 0f64);
    for (i, &x) in src.iter().enumerate().skip(4800 * channels) {
        let Some(&y) = dec.get(i + d) else { break };
        e += ((x - y) as f64).powi(2);
        s += (x as f64).powi(2);
    }
    10.0 * (s / e.max(1e-20)).log10()
}

fn round_trip(channels: usize, bitrate: u32) -> f64 {
    let frames = 50;
    let src = test_signal(channels, frames, 7);
    let mut enc = Encoder::new(channels, bitrate).unwrap();
    let mut dec = StreamDecoder::new(48_000, channels).unwrap();
    let mut out = Vec::new();
    for f in 0..frames {
        let fs = Encoder::FRAME_SIZE * channels;
        let pkt = enc.encode(&src[f * fs..(f + 1) * fs]);
        assert_eq!(pkt.len(), enc.packet_bytes());
        let mut pcm = Vec::new();
        let n = dec.decode_interleaved(Some(&pkt), &mut pcm).unwrap();
        assert_eq!(n, Encoder::FRAME_SIZE);
        assert_eq!(dec.final_range(), enc.final_range(), "frame {f}: range coder state differs");
        out.extend_from_slice(&pcm[..n * channels]);
    }
    snr(&src, &out, channels)
}

#[test]
fn mono_round_trip() {
    for (rate, min) in [(32_000, 8.0), (64_000, 14.0), (128_000, 20.0)] {
        let s = round_trip(1, rate);
        eprintln!("mono {rate}: {s:.1} dB");
        assert!(s > min, "mono {rate}: SNR {s:.1} dB");
    }
}

#[test]
fn stereo_round_trip() {
    for (rate, min) in [(64_000, 8.0), (128_000, 14.0), (256_000, 20.0)] {
        let s = round_trip(2, rate);
        eprintln!("stereo {rate}: {s:.1} dB");
        assert!(s > min, "stereo {rate}: SNR {s:.1} dB");
    }
}

/// Silence, hostile samples, short input and every bit rate edge decode cleanly.
#[test]
fn edge_inputs_and_rates() {
    let fs = Encoder::FRAME_SIZE;
    for channels in [1usize, 2] {
        for rate in [6_000u32, 12_000, 24_000, 510_000] {
            let mut enc = Encoder::new(channels, rate).unwrap();
            let mut dec = StreamDecoder::new(48_000, channels).unwrap();
            let loud: Vec<f32> = (0..fs * channels).map(|i| if i % 3 == 0 { 8.0 } else { -8.0 }).collect();
            let hostile: Vec<f32> = (0..fs * channels).map(|i| [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30, -0.5][i % 5]).collect();
            let tone: Vec<f32> = (0..fs * channels).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
            let inputs: [&[f32]; 6] = [&vec![0.0; fs * channels], &tone, &loud, &hostile, &tone[..17], &[]];
            for (k, pcm) in inputs.iter().enumerate() {
                let pkt = enc.encode(pcm);
                assert_eq!(pkt.len(), enc.packet_bytes());
                let mut out = Vec::new();
                assert_eq!(dec.decode_interleaved(Some(&pkt), &mut out).unwrap(), fs, "input {k}");
                assert!(out.iter().all(|v| v.is_finite()), "input {k}");
                if k == 0 {
                    assert!(out.iter().all(|v| v.abs() < 1e-3), "silence decodes as silence");
                }
            }
        }
    }
    assert!(Encoder::new(3, 64_000).is_err());
    assert!(Encoder::new(2, 1_000).is_err());
}

/// Regression: an all-zero frame after sound was coded as a silence frame, skipping the MDCT that
/// completes the previous frame's overlap, so its last samples were lost.
#[test]
fn zero_frame_after_sound_finishes_the_overlap() {
    let fs = Encoder::FRAME_SIZE;
    let mut enc = Encoder::new(1, 128_000).unwrap();
    let mut dec = StreamDecoder::new(48_000, 1).unwrap();
    let tone: Vec<f32> = (0..fs).map(|i| 0.4 * (i as f32 * 0.031).sin()).collect();
    let mut out = Vec::new();
    for frame in [&tone[..], &vec![0.0; fs], &vec![0.0; fs]] {
        let pkt = enc.encode(frame);
        dec.decode_interleaved(Some(&pkt), &mut out).unwrap();
    }
    // The tone's last 120 samples come out in the second packet.
    let (mut e, mut s) = (0f64, 0f64);
    for i in fs - 120..fs {
        e += ((tone[i] - out[i + Encoder::LOOKAHEAD]) as f64).powi(2);
        s += (tone[i] as f64).powi(2);
    }
    let snr = 10.0 * (s / e.max(1e-20)).log10();
    assert!(snr > 15.0, "tail SNR {snr:.1} dB");
}
