//! G.729 against ffmpeg's own G.729 decoder on synthetic bitstreams.
//!
//! ffmpeg's decoder is a separate implementation that is not bit-exact with the ITU-T reference
//! (on the ITU speech test vector it differs in most samples), so this checks agreement within a
//! signal-to-noise ratio; bit-exactness is checked against bcg729's test patterns (`patterns.rs`).

use std::process::Command;

fn rng(x: &mut u64) -> u64 {
    *x ^= *x << 13;
    *x ^= *x >> 7;
    *x ^= *x << 17;
    *x
}

/// A raw stream of `frames` random 10-byte frames.
fn stream(seed: u64, frames: usize) -> Vec<u8> {
    let mut x = seed;
    (0..frames * 10).map(|_| rng(&mut x) as u8).collect()
}

fn ffmpeg_decode(ff: &std::path::Path, data: &[u8], tag: &str) -> Option<Vec<i16>> {
    let dir = std::env::temp_dir().join(format!("cfa4-g729-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let input = dir.join("in.g729");
    let output = dir.join("out.raw");
    std::fs::write(&input, data).ok()?;
    let ok =
        Command::new(ff).args(["-v", "error", "-y", "-f", "g729", "-i"]).arg(&input).args(["-f", "s16le", "-ac", "1"]).arg(&output).status().ok()?.success();
    let raw = std::fs::read(&output).ok();
    let _ = std::fs::remove_dir_all(&dir);
    if !ok {
        return None;
    }
    Some(raw?.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect())
}

#[test]
fn synthetic_streams_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("G.729 oracle") else { return };
    for seed in 1..=8u64 {
        let data = stream(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15), 300);
        let ours = cfa4_g729::decode_file(&data).expect("decode");
        let Some(theirs) = ffmpeg_decode(&ff, &data, &seed.to_string()) else { panic!("ffmpeg failed") };
        assert_eq!(ours.len(), theirs.len());
        let mut max = 0i32;
        let mut diff = 0usize;
        let mut first = None;
        for (i, (a, b)) in ours.iter().zip(&theirs).enumerate() {
            let d = (i32::from(*a) - i32::from(*b)).abs();
            if d > 0 {
                diff += 1;
                first.get_or_insert(i);
            }
            max = max.max(d);
        }
        let sig: f64 = theirs.iter().map(|&v| f64::from(v).powi(2)).sum();
        let err: f64 = ours.iter().zip(&theirs).map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2)).sum();
        let snr = 10.0 * (sig / err.max(1e-9)).log10();
        eprintln!("seed {seed}: {diff}/{} differ, max {max}, first {first:?}, snr {snr:.1} dB", ours.len());
        assert!(snr > 15.0, "seed {seed}: {snr:.1} dB against ffmpeg");
    }
}
