//! ATRAC3 encoder round trips: our AT3 files decoded by our ATRAC3 decoder (`cfa4-realaudio`)
//! and, where ffmpeg is present, by ffmpeg's `atrac3` decoder (the CI oracle): the files parse,
//! every frame decodes, and the decoded audio matches the source within the codec's precision.

use std::process::Command;

use cfa4_atracenc::atrac3::{MODES, Mode};
use cfa4_atracenc::riff::{ATRAC3_DELAY, encode_at3};
use cfa4_realaudio::atrac3::{Decoder, Params};

/// (channels, block_align, fmt extra data, data chunk) of a WAV file.
fn wav(d: &[u8]) -> (u16, usize, Vec<u8>, Vec<u8>) {
    let mut pos = 12;
    let (mut ch, mut ba, mut extra, mut data) = (0, 0, Vec::new(), Vec::new());
    while pos + 8 <= d.len() {
        let id = &d[pos..pos + 4];
        let len = u32::from_le_bytes([d[pos + 4], d[pos + 5], d[pos + 6], d[pos + 7]]) as usize;
        let body = &d[pos + 8..(pos + 8 + len).min(d.len())];
        if id == b"fmt " {
            ch = u16::from_le_bytes([body[2], body[3]]);
            ba = usize::from(u16::from_le_bytes([body[12], body[13]]));
            let cb = usize::from(u16::from_le_bytes([body[16], body[17]]));
            extra = body[18..18 + cb].to_vec();
        } else if id == b"data" {
            data = body.to_vec();
        }
        pos += 8 + len + (len & 1);
    }
    (ch, ba, extra, data)
}

/// Interleaved test signal: a tone sweep with a few hard attacks (castanet-like clicks) and
/// some noise, different per channel.
fn signal(channels: usize, seconds: f32) -> Vec<f32> {
    let n = (44100.0 * seconds) as usize;
    let mut seed = 12345u32;
    let mut out = Vec::with_capacity(n * channels);
    for i in 0..n {
        let t = i as f32 / 44100.0;
        for c in 0..channels {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (seed >> 8) as f32 / (1 << 24) as f32 - 0.5;
            let f = 220.0 * (1.0 + c as f32 * 0.5) * (1.0 + t);
            let tone = (2.0 * std::f32::consts::PI * f * t).sin() * 0.4;
            let click = if i % 11025 < 200 && t > 0.2 { (1.0 - (i % 11025) as f32 / 200.0) * 0.5 * ((i as f32) * 0.9).sin() } else { 0.0 };
            out.push(tone + click + noise * 0.02);
        }
    }
    out
}

fn decode_ours(file: &[u8]) -> (usize, Vec<f32>) {
    let (ch, ba, extra, data) = wav(file);
    let mut dec = Decoder::new(&Params { channels: ch, block_align: ba, extradata: extra }).expect("decoder accepts our header");
    let mut out = Vec::new();
    for fr in data.chunks_exact(ba) {
        let p = dec.decode(fr).expect("every frame decodes");
        for i in 0..p[0].len() {
            for chan in &p {
                out.push(chan[i]);
            }
        }
    }
    (usize::from(ch), out)
}

/// Signal-to-noise ratio (dB) of `decoded` (delayed by `delay` samples per channel) against `src`.
fn snr(src: &[f32], decoded: &[f32], channels: usize, delay: usize) -> f64 {
    let (mut s, mut e) = (0f64, 0f64);
    for (i, &v) in src.iter().enumerate() {
        let Some(&d) = decoded.get(i + delay * channels) else { break };
        s += f64::from(v) * f64::from(v);
        e += f64::from(v - d) * f64::from(v - d);
    }
    10.0 * (s / e.max(1e-20)).log10()
}

fn roundtrip(mode: Mode, channels: u16) -> Vec<u8> {
    let c = usize::from(channels);
    let src = signal(c, 2.0);
    let file = encode_at3(&src, channels, mode, &mut |_| true).unwrap().unwrap();
    let (ch, out) = decode_ours(&file);
    assert_eq!(ch, c);
    assert!(out.len() >= src.len() + ATRAC3_DELAY as usize * c, "every source sample decodes");
    // the delay: the best-matching shift near the nominal one
    let best = (ATRAC3_DELAY as usize - 200..ATRAC3_DELAY as usize + 200).max_by(|&a, &b| snr(&src, &out, c, a).total_cmp(&snr(&src, &out, c, b))).unwrap();
    let q = snr(&src, &out, c, ATRAC3_DELAY as usize);
    eprintln!("{} kbit/s, {c} ch: SNR {q:.1} dB at the nominal delay {ATRAC3_DELAY}, best shift {best} ({:.1} dB)", mode.kbps(), snr(&src, &out, c, best));
    assert_eq!(best, ATRAC3_DELAY as usize, "the delay written to the fact chunk");
    let floor = 18.0;
    assert!(q > floor, "{} kbit/s {c} ch: SNR {q:.1} dB", mode.kbps());
    file
}

#[test]
fn all_modes_mono_and_stereo() {
    for m in MODES {
        for ch in 1..=2 {
            roundtrip(m, ch);
        }
    }
}

#[test]
fn silence_and_extremes() {
    for m in MODES {
        let silent = vec![0f32; 44100];
        let file = encode_at3(&silent, 2, m, &mut |_| true).unwrap().unwrap();
        let (_, out) = decode_ours(&file);
        assert!(out.iter().all(|v| v.abs() < 1e-4));
        // full-scale square wave and non-finite samples
        let mut wild: Vec<f32> = (0..44100).map(|i| if (i / 50) % 2 == 0 { 1.0 } else { -1.0 }).collect();
        wild[100] = f32::NAN;
        wild[200] = f32::INFINITY;
        let file = encode_at3(&wild, 1, m, &mut |_| true).unwrap().unwrap();
        let (_, out) = decode_ours(&file);
        assert!(out.iter().all(|v| v.is_finite()));
    }
    // no samples at all: one frame
    let file = encode_at3(&[], 2, MODES[2], &mut |_| true).unwrap().unwrap();
    assert!(!wav(&file).3.is_empty());
}

#[test]
fn cancel() {
    let src = signal(2, 3.0);
    assert!(encode_at3(&src, 2, MODES[2], &mut |_| false).unwrap().is_none());
}

#[test]
fn ffmpeg_decodes_our_files() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("ATRAC3 encoder") else { return };
    let dir = cfa4_testkit::fixtures::fixtures_dir("atracenc");
    std::fs::create_dir_all(&dir).unwrap();
    for m in MODES {
        for ch in 1..=2u16 {
            let file = roundtrip(m, ch);
            let path = dir.join(format!("enc_{}_{ch}.at3", m.kbps()));
            std::fs::write(&path, &file).unwrap();
            let out = Command::new(&ff)
                .args(["-v", "error", "-flags", "+bitexact", "-i"])
                .arg(&path)
                .args(["-f", "f32le", "-c:a", "pcm_f32le", "-"])
                .output()
                .unwrap();
            assert!(out.status.success(), "ffmpeg: {}", String::from_utf8_lossy(&out.stderr));
            assert!(out.stderr.is_empty(), "ffmpeg reports: {}", String::from_utf8_lossy(&out.stderr));
            let theirs: Vec<f32> = out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
            let (_, ours) = decode_ours(&file);
            assert_eq!(ours.len(), theirs.len(), "{} kbit/s {ch} ch: sample count", m.kbps());
            let worst = ours.iter().zip(&theirs).fold(0f32, |w, (a, b)| w.max((a - b).abs()));
            assert!(worst < 1e-5, "{} kbit/s {ch} ch: ffmpeg and our decoder differ by {worst}", m.kbps());
        }
    }
}
