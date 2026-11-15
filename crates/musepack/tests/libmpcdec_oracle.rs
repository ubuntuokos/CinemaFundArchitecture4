//! Exactness against libmpcdec itself (musepack_src_r475, the reference this crate is ported
//! from). The external tools from scripts/ci/install-musepack-oracle.sh encode a synthetic
//! signal with the reference SV8 encoder `mpcenc` at several qualities, sample rates and channel
//! counts, and `cfa4-mpc-oracle` decodes it with libmpcdec's floating-point build; our samples
//! must be the same floats. ffmpeg's FATE samples (SV7 and SV8, optional local fixtures) are
//! compared the same way when present. Skipped without the tools unless
//! `CFA4_REQUIRE_ORACLES=1`.

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_testkit::oracle;

fn tools() -> Option<(PathBuf, PathBuf)> {
    let dec = oracle::find_tool("cfa4-mpc-oracle");
    let enc = oracle::find_tool("mpcenc");
    match (dec, enc) {
        (Some(d), Some(e)) => Some((d, e)),
        _ => {
            oracle::skip("musepack oracle", "cfa4-mpc-oracle / mpcenc not found (scripts/ci/install-musepack-oracle.sh)");
            None
        }
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cfa4-mpc-oracle-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// A 16-bit PCM WAV file of a deterministic music-like test signal: chords with vibrato,
/// noise bursts, a silent gap and a clipped burst, the channels different.
fn wav(rate: u32, channels: u16, seconds: u32) -> Vec<u8> {
    let n = (rate * seconds) as usize;
    let mut seed: u32 = 0x2468_ace1;
    let mut noise = move || {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        ((seed >> 16) as i32 - 32768) as f64 / 32768.0
    };
    let mut pcm = Vec::with_capacity(n * usize::from(channels) * 2);
    for i in 0..n {
        let t = i as f64 / f64::from(rate);
        let seg = (t * 3.0) as u32 % 5;
        for c in 0..channels {
            let detune = 1.0 + 0.003 * f64::from(c);
            let chord: f64 = [220.0, 277.18, 329.63, 440.0, 1760.0]
                .iter()
                .enumerate()
                .map(|(k, f)| (std::f64::consts::TAU * f * detune * t + 0.4 * (t * 5.0).sin()).sin() / (k as f64 + 1.0))
                .sum();
            let v = match seg {
                0 => 9000.0 * chord,
                1 => 7000.0 * chord + 3000.0 * noise(),
                2 => 0.0,
                3 => 300.0 * noise(),
                _ => 50000.0 * (std::f64::consts::TAU * 3000.0 * t).sin(),
            };
            pcm.extend_from_slice(&(v.clamp(-32768.0, 32767.0) as i16).to_le_bytes());
        }
    }
    let mut out = Vec::new();
    let block = channels * 2;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
    out.extend_from_slice(&block.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(&pcm);
    out
}

/// libmpcdec's samples of `mpc`, interleaved.
fn reference(dec: &Path, mpc: &Path) -> Vec<f32> {
    let out = mpc.with_extension("f32");
    let ok = Command::new(dec).arg(mpc).arg(&out).status().is_ok_and(|s| s.success());
    assert!(ok, "{}: cfa4-mpc-oracle failed", mpc.display());
    std::fs::read(&out).expect("read f32").chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
}

fn assert_same(tag: &str, data: &[u8], theirs: &[f32]) {
    let (info, planes) = cfa4_musepack::decode_file(data).unwrap_or_else(|e| panic!("{tag}: {e}"));
    let ch = usize::from(info.channels);
    assert_eq!(planes.len(), ch, "{tag}: channels");
    let frames = planes.first().map_or(0, Vec::len);
    assert_eq!(frames * ch, theirs.len(), "{tag}: length");
    let mut diff = 0usize;
    let mut first = None;
    for (i, want) in theirs.iter().enumerate() {
        let got = planes[i % ch][i / ch];
        if got.to_bits() != want.to_bits() {
            diff += 1;
            first.get_or_insert((i / ch, i % ch, got, *want));
        }
    }
    assert_eq!(diff, 0, "{tag}: {diff}/{} samples differ, first (sample, channel, ours, libmpcdec) {first:?}", theirs.len());
}

#[test]
fn mpcenc_streams_match_libmpcdec_exactly() {
    let Some((dec, enc)) = tools() else { return };
    let dir = temp_dir("enc");
    for (rate, channels, quality) in [(44100, 2, "5"), (44100, 2, "0"), (44100, 2, "10"), (48000, 2, "3"), (32000, 1, "7"), (44100, 1, "5")] {
        let tag = format!("{rate}-{channels}ch-q{quality}");
        let input = dir.join(format!("{tag}.wav"));
        let mpc = dir.join(format!("{tag}.mpc"));
        std::fs::write(&input, wav(rate, channels, 5)).expect("write wav");
        let ok = Command::new(&enc).args(["--silent", "--overwrite", "--quality", quality]).arg(&input).arg(&mpc).status().is_ok_and(|s| s.success());
        assert!(ok, "{tag}: mpcenc failed");
        let data = std::fs::read(&mpc).expect("read mpc");
        assert_same(&tag, &data, &reference(&dec, &mpc));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fate_samples_match_libmpcdec_exactly() {
    let Some((dec, _)) = tools() else { return };
    let dir = std::env::var_os("MUSEPACK_SAMPLES").map(PathBuf::from).unwrap_or_else(|| cfa4_testkit::fixtures::fixtures_root().join("musepack/fate"));
    for name in ["inside-mp7.mpc", "inside-mp8.mpc"] {
        let path = dir.join(name);
        let Ok(data) = std::fs::read(&path) else {
            eprintln!("{name}: optional FATE sample not present");
            continue;
        };
        let work = temp_dir(name);
        let copy = work.join(name);
        std::fs::write(&copy, &data).expect("copy");
        assert_same(name, &data, &reference(&dec, &copy));
        let _ = std::fs::remove_dir_all(&work);
    }
}
