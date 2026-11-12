//! Musepack files against reference decoders.
//!
//! ffmpeg's FATE samples (`inside-mp7.mpc`, `inside-mp8.mpc` from
//! `https://fate-suite.ffmpeg.org/musepack/`) in `$MUSEPACK_SAMPLES` or
//! `target/fixtures/musepack/fate`, against ffmpeg's decoder: its fixed-point synthesis rounds to
//! 16 bits, so every sample must be within 1 LSB of ours rounded; ffmpeg keeps the synthesis
//! delay of 481 samples that libmpcdec (and this decoder) drops. Also damaged copies of them,
//! which must decode without a panic.

use std::path::PathBuf;
use std::process::Command;

fn samples() -> Vec<(String, Vec<u8>)> {
    let dir = std::env::var_os("MUSEPACK_SAMPLES").map(PathBuf::from).unwrap_or_else(|| cfa4_testkit::fixtures::fixtures_root().join("musepack/fate"));
    let mut out = Vec::new();
    for name in ["inside-mp7.mpc", "inside-mp8.mpc"] {
        match std::fs::read(dir.join(name)) {
            Ok(d) => out.push((name.to_string(), d)),
            Err(_) => cfa4_testkit::oracle::skip(name, "FATE sample not present"),
        }
    }
    out
}

#[test]
fn fate_samples_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg() else {
        cfa4_testkit::oracle::skip("musepack", "no ffmpeg");
        return;
    };
    let dir = std::env::var_os("MUSEPACK_SAMPLES").map(PathBuf::from).unwrap_or_else(|| cfa4_testkit::fixtures::fixtures_root().join("musepack/fate"));
    for (name, data) in samples() {
        let (info, planes) = cfa4_musepack::decode_file(&data).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((info.sample_rate, info.channels, info.samples), (44100, 2, 524_277), "{name}");
        assert!(planes.iter().all(|p| p.len() as u64 == info.samples), "{name}: length");
        let out = Command::new(&ff)
            .args(["-v", "error", "-i"])
            .arg(dir.join(&name))
            .args(["-f", "s16le", "-c:a", "pcm_s16le", "-"])
            .output()
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(out.status.success(), "{name}: ffmpeg failed");
        let reference: Vec<i16> = out.stdout.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        let delay = 481 * 2;
        assert!(reference.len() >= delay + planes[0].len() * 2, "{name}: ffmpeg output too short");
        for (i, (l, r)) in planes[0].iter().zip(&planes[1]).enumerate() {
            for (c, v) in [l, r].into_iter().enumerate() {
                let ours = (f64::from(*v) * 32768.0).round().clamp(-32768.0, 32767.0) as i32;
                let theirs = i32::from(reference[delay + i * 2 + c]);
                assert!((ours - theirs).abs() <= 1, "{name}: sample {i} channel {c}: ours {ours}, ffmpeg {theirs}");
            }
        }
    }
}

#[test]
fn damaged_samples_never_panic() {
    let mut seed = 0x1234_5678_9abc_def0u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for (name, data) in samples() {
        // the first 20 kB keep the decode short
        let data = &data[..data.len().min(20_000)];
        for round in 0..1000 {
            let mut d = data.to_vec();
            match round % 5 {
                0 => d.truncate((next() as usize) % data.len()),
                1 => {
                    for _ in 0..1 + next() % 8 {
                        let i = (next() as usize) % d.len();
                        d[i] ^= 1 << (next() % 8);
                    }
                }
                2 => {
                    // keep the header, scramble a run of the stream
                    let start = 40 + (next() as usize) % (d.len() - 40);
                    let end = (start + 1 + (next() as usize) % 64).min(d.len());
                    for b in &mut d[start..end] {
                        *b = next() as u8;
                    }
                }
                3 => {
                    // keep the headers, everything after them random
                    for b in &mut d[200..] {
                        *b = next() as u8;
                    }
                }
                _ => {
                    let i = 4 + (next() as usize) % 40;
                    d[i] = next() as u8;
                }
            }
            if let Ok((info, planes)) = cfa4_musepack::decode_file(&d) {
                assert!(planes.len() == usize::from(info.channels), "{name} round {round}");
                assert!(planes.iter().flatten().all(|v| v.is_finite()), "{name} round {round}: not finite");
            }
        }
    }
}
