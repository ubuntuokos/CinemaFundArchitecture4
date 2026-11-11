//! Bit-exactness against opencore-amr, the Apache-2.0 decoder ffmpeg and most players use. The
//! external helper `cfa4-amr-oracle` (scripts/ci/install-amr-oracle.sh) encodes our synthetic
//! test signal in every mode and decodes it; our decoder must give the same samples. Skipped
//! without the helper unless `CFA4_REQUIRE_ORACLES=1`.

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_testkit::{fixtures, oracle};

const TOOL: &str = "cfa4-amr-oracle";

fn tool() -> Option<PathBuf> {
    let t = oracle::find_tool(TOOL);
    if t.is_none() {
        oracle::skip("amr oracle", "cfa4-amr-oracle not found (scripts/ci/install-amr-oracle.sh)");
    }
    t
}

/// A deterministic speech-like test signal at `rate`: voiced segments with a gliding pitch and
/// formant-like harmonics, unvoiced noise, silence (to trigger DTX), low-level noise, and a
/// clipped full-scale burst (saturation paths).
fn signal(rate: u32, seconds: u32) -> Vec<i16> {
    let n = (rate * seconds) as usize;
    let mut seed: u32 = 0x1234_5678;
    let mut noise = move || {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        ((seed >> 16) as i32 - 32768) as f64 / 32768.0
    };
    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0f64;
    for i in 0..n {
        let t = i as f64 / f64::from(rate);
        let seg = (t * 2.0) as u32 % 6;
        let f0 = 110.0 + 80.0 * (t * 1.7).sin() + 40.0 * (t * 0.3).cos();
        phase += f0 / f64::from(rate);
        let voiced: f64 = (1..=12)
            .map(|k| {
                let k = f64::from(k);
                let formant = (-((k * f0 - 700.0) / 400.0).powi(2)).exp() + 0.6 * (-((k * f0 - 1800.0) / 500.0).powi(2)).exp();
                (formant + 0.05) * (std::f64::consts::TAU * k * phase).sin() / k.sqrt()
            })
            .sum();
        let v = match seg {
            0 => 9000.0 * voiced,
            1 => 6000.0 * noise(),
            2 => 0.0,
            3 => 40.0 * noise(),
            4 => 14000.0 * voiced + 2000.0 * noise(),
            _ => 60000.0 * (std::f64::consts::TAU * 900.0 * t).sin(),
        };
        out.push(v.clamp(-32768.0, 32767.0) as i16);
    }
    out
}

fn write_raw(path: &Path, pcm: &[i16]) {
    let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
    std::fs::write(path, bytes).expect("write raw");
}

fn read_raw(path: &Path) -> Vec<i16> {
    let b = std::fs::read(path).expect("read raw");
    b.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()
}

fn run(tool: &Path, args: &[&str], files: &[&Path]) -> bool {
    Command::new(tool).args(args).args(files).status().is_ok_and(|s| s.success())
}

/// Encodes the test signal with opencore (`mode`, `dtx`) into an AMR-NB storage file.
fn encode_nb(tool: &Path, mode: &str, dtx: bool) -> Option<PathBuf> {
    let dir = fixtures::fixtures_dir("amr");
    let raw = dir.join("nb_source.s16");
    if !raw.exists() {
        write_raw(&raw, &signal(8000, 12));
    }
    let out = dir.join(format!("nb_{mode}_{}.amr", u8::from(dtx)));
    fixtures::generate(&out, |tmp| run(tool, &["enc-nb", mode, if dtx { "1" } else { "0" }], &[&raw, tmp]))
}

/// opencore's decode of `amr`.
fn decode_nb_oracle(tool: &Path, amr: &Path) -> Vec<i16> {
    let out = amr.with_extension("amr.s16");
    let tmp = fixtures::temp_path(&out);
    assert!(run(tool, &["dec-nb"], &[amr, &tmp]), "oracle decode of {}", amr.display());
    let pcm = read_raw(&tmp);
    let _ = std::fs::remove_file(&tmp);
    pcm
}

fn compare(name: &str, ours: &[i16], theirs: &[i16]) {
    assert_eq!(ours.len(), theirs.len(), "{name}: sample count");
    if let Some(i) = ours.iter().zip(theirs).position(|(a, b)| a != b) {
        let f = i / 160;
        panic!(
            "{name}: first mismatch at sample {i} (frame {f}): ours {:?} opencore {:?}",
            &ours[i..(i + 8).min(ours.len())],
            &theirs[i..(i + 8).min(theirs.len())]
        );
    }
}

fn check_file(tool: &Path, name: &str, amr: &Path) {
    let data = std::fs::read(amr).expect("read amr");
    let ours = cfa4_amr::decode_nb_file(&data).expect("decode");
    compare(name, &ours, &decode_nb_oracle(tool, amr));
}

#[test]
fn nb_every_mode_matches_opencore() {
    let Some(tool) = tool() else { return };
    for mode in ["0", "1", "2", "3", "4", "5", "6", "7", "cycle"] {
        for dtx in [false, true] {
            let name = format!("nb mode {mode} dtx {dtx}");
            let Some(amr) = encode_nb(&tool, mode, dtx) else {
                oracle::skip("amr oracle", &format!("could not encode {name}"));
                return;
            };
            check_file(&tool, &name, &amr);
        }
    }
}

/// Frames of `amr` (after the magic) as `(offset, size)`.
fn frames(data: &[u8]) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    let mut pos = cfa4_amr::NB_MAGIC.len();
    while let Some(&toc) = data.get(pos) {
        let size = cfa4_amr::nb::Decoder::frame_size(toc);
        if pos + size > data.len() {
            break;
        }
        v.push((pos, size));
        pos += size;
    }
    v
}

/// Lost frames (`NO_DATA`), frame types the decoder skips, and corrupted payloads, in speech and
/// in DTX: concealment, comfort noise and hostile bits must all match opencore.
#[test]
fn nb_lost_and_damaged_frames_match_opencore() {
    let Some(tool) = tool() else { return };
    let dir = fixtures::fixtures_dir("amr");
    for (mode, dtx) in [("7", false), ("0", true), ("cycle", true), ("5", false)] {
        let Some(amr) = encode_nb(&tool, mode, dtx) else { return };
        let data = std::fs::read(&amr).expect("read");
        let mut out = cfa4_amr::NB_MAGIC.to_vec();
        let mut seed: u32 = 99;
        for (k, (pos, size)) in frames(&data).into_iter().enumerate() {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let r = seed >> 24;
            let frame = &data[pos..pos + size];
            match r % 16 {
                // Lose a frame, or a run of frames.
                0 | 1 => out.push(0x7c),
                2 if k % 3 == 0 => out.extend([0x7c; 8]),
                // A frame type the decoder skips (9-14).
                3 => out.push(((9 + (r >> 4) % 6) << 3) as u8 | 4),
                // Flip payload bits, keeping the frame type.
                4 => {
                    let mut f = frame.to_vec();
                    for b in f.iter_mut().skip(1) {
                        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                        *b ^= (seed >> 24) as u8;
                    }
                    out.extend_from_slice(&f);
                }
                _ => out.extend_from_slice(frame),
            }
        }
        let damaged = dir.join(format!("nb_damaged_{mode}_{}.amr", u8::from(dtx)));
        std::fs::write(&damaged, &out).expect("write");
        check_file(&tool, &format!("damaged nb mode {mode} dtx {dtx}"), &damaged);
    }
}

/// Random bytes after the magic: never a panic.
#[test]
fn nb_random_input_never_panics() {
    let mut seed: u32 = 7;
    for len in [0usize, 1, 5, 13, 100, 1000, 5000] {
        let mut data = cfa4_amr::NB_MAGIC.to_vec();
        for _ in 0..len {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            data.push((seed >> 24) as u8);
        }
        let _ = cfa4_amr::decode_nb_file(&data);
    }
}
