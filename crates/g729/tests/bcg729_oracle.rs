//! Bit-exactness against bcg729 1.1.2, the decoder this crate is ported from. The external
//! helper `cfa4-g729-oracle` (scripts/ci/install-g729-oracle.sh) encodes a synthetic test
//! signal with bcg729, with and without Annex B VAD / DTX, and decodes it and damaged streams;
//! our decoder must give the same samples. Skipped without the helper unless
//! `CFA4_REQUIRE_ORACLES=1`.

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_testkit::oracle;

const TOOL: &str = "cfa4-g729-oracle";

fn tool() -> Option<PathBuf> {
    let t = oracle::find_tool(TOOL);
    if t.is_none() {
        oracle::skip("g729 oracle", "cfa4-g729-oracle not found (scripts/ci/install-g729-oracle.sh)");
    }
    t
}

/// A deterministic speech-like 8 kHz signal: voiced segments with a gliding pitch, unvoiced
/// noise, silence and low-level noise (so the VAD sends SID and untransmitted frames), and a
/// clipped full-scale burst.
fn signal(seconds: u32) -> Vec<i16> {
    let rate = 8000.0;
    let n = 8000 * seconds as usize;
    let mut seed: u32 = 0x1234_5678;
    let mut noise = move || {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        ((seed >> 16) as i32 - 32768) as f64 / 32768.0
    };
    let mut phase = 0.0f64;
    (0..n)
        .map(|i| {
            let t = i as f64 / rate;
            let f0 = 110.0 + 80.0 * (t * 1.7).sin() + 40.0 * (t * 0.3).cos();
            phase += f0 / rate;
            let voiced: f64 = (1..=12)
                .map(|k| {
                    let k = f64::from(k);
                    let formant = (-((k * f0 - 700.0) / 400.0).powi(2)).exp() + 0.6 * (-((k * f0 - 1800.0) / 500.0).powi(2)).exp();
                    (formant + 0.05) * (std::f64::consts::TAU * k * phase).sin() / k.sqrt()
                })
                .sum();
            let v = match (t * 2.0) as u32 % 6 {
                0 => 9000.0 * voiced,
                1 => 6000.0 * noise(),
                2 => 0.0,
                3 => 40.0 * noise(),
                4 => 14000.0 * voiced + 2000.0 * noise(),
                _ => 60000.0 * (std::f64::consts::TAU * 900.0 * t).sin(),
            };
            v.clamp(-32768.0, 32767.0) as i16
        })
        .collect()
}

fn run(tool: &Path, args: &[&str], files: &[&Path]) -> bool {
    Command::new(tool).args(args).args(files).status().is_ok_and(|s| s.success())
}

fn read_raw(path: &Path) -> Vec<i16> {
    std::fs::read(path).unwrap_or_default().chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()
}

/// bcg729's decoding of an ITU-T `.bit` stream.
fn bcg729_decode(tool: &Path, dir: &Path, bit: &[u8], tag: &str) -> Vec<i16> {
    let input = dir.join(format!("{tag}.bit"));
    let output = dir.join(format!("{tag}.raw"));
    std::fs::write(&input, bit).expect("write bit");
    assert!(run(tool, &["dec"], &[&input, &output]), "{tag}: bcg729 decode failed");
    read_raw(&output)
}

fn assert_same(ours: &[i16], theirs: &[i16], tag: &str) {
    assert_eq!(ours.len(), theirs.len(), "{tag}: length");
    let diff = ours.iter().zip(theirs).filter(|(a, b)| a != b).count();
    let first = ours.iter().zip(theirs).position(|(a, b)| a != b);
    assert_eq!(diff, 0, "{tag}: {diff}/{} samples differ, first at {first:?}", ours.len());
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cfa4-g729-oracle-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// One `.bit` frame: sync word, bit count and the bits of `bytes`.
fn bit_frame(out: &mut Vec<u8>, sync: u16, nbits: usize, bytes: &[u8]) {
    out.extend_from_slice(&sync.to_le_bytes());
    out.extend_from_slice(&(nbits as u16).to_le_bytes());
    for k in 0..nbits {
        let one = bytes.get(k / 8).is_some_and(|b| b >> (7 - k % 8) & 1 == 1);
        out.extend_from_slice(&(if one { 0x0081u16 } else { 0x007f }).to_le_bytes());
    }
}

#[test]
fn bcg729_encoded_speech_matches_bit_exactly() {
    let Some(tool) = tool() else { return };
    let dir = temp_dir("enc");
    let pcm = signal(12);
    let raw = dir.join("in.raw");
    std::fs::write(&raw, pcm.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<u8>>()).expect("write raw");
    for vad in ["0", "1"] {
        let bit = dir.join(format!("vad{vad}.bit"));
        assert!(run(&tool, &["enc", vad], &[&raw, &bit]), "bcg729 encode failed");
        let data = std::fs::read(&bit).expect("read bit");
        let theirs = bcg729_decode(&tool, &dir, &data, &format!("vad{vad}"));
        let ours = cfa4_g729::decode_file(&data).expect("decode");
        assert_same(&ours, &theirs, &format!("vad {vad}"));
        if vad == "1" {
            // the stream really exercises Annex B: SID and untransmitted frames
            let lens: Vec<u16> = data.chunks_exact(2).map(|w| u16::from_le_bytes([w[0], w[1]])).collect();
            let (mut i, mut sid, mut untx) = (0usize, 0, 0);
            while let Some(&n) = lens.get(i + 1) {
                sid += usize::from(n == 16);
                untx += usize::from(n == 0);
                i += 2 + usize::from(n);
            }
            assert!(sid > 3 && untx > 10, "{sid} SID and {untx} untransmitted frames");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn random_and_damaged_streams_match_bit_exactly() {
    let Some(tool) = tool() else { return };
    let dir = temp_dir("rnd");
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    let mut rng = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for seed in 0..6 {
        let mut data = Vec::new();
        for _ in 0..400 {
            let bytes = rng().to_le_bytes();
            let more = rng().to_le_bytes();
            let frame = [bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], more[0], more[1]];
            match rng() % 16 {
                0 => bit_frame(&mut data, 0x6b20, 80, &frame),          // erased
                1 | 2 => bit_frame(&mut data, 0x6b21, 16, &frame[..2]), // SID
                3 => bit_frame(&mut data, 0x6b21, 0, &[]),              // not sent
                _ => bit_frame(&mut data, 0x6b21, 80, &frame),
            }
        }
        let tag = format!("seed{seed}");
        let theirs = bcg729_decode(&tool, &dir, &data, &tag);
        let ours = cfa4_g729::decode_file(&data).expect("decode");
        assert_same(&ours, &theirs, &tag);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
