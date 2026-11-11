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

/// The two codecs under test.
#[derive(Clone, Copy)]
enum Codec {
    Nb,
    Wb,
}

impl Codec {
    fn tag(self) -> &'static str {
        match self {
            Codec::Nb => "nb",
            Codec::Wb => "wb",
        }
    }

    fn rate(self) -> u32 {
        match self {
            Codec::Nb => 8000,
            Codec::Wb => 16000,
        }
    }

    fn ext(self) -> &'static str {
        match self {
            Codec::Nb => "amr",
            Codec::Wb => "awb",
        }
    }

    fn magic(self) -> &'static [u8] {
        match self {
            Codec::Nb => cfa4_amr::NB_MAGIC,
            Codec::Wb => cfa4_amr::WB_MAGIC,
        }
    }

    fn frame_size(self, toc: u8) -> usize {
        match self {
            Codec::Nb => cfa4_amr::nb::Decoder::frame_size(toc),
            Codec::Wb => cfa4_amr::wb::Decoder::frame_size(toc),
        }
    }

    fn samples(self) -> usize {
        match self {
            Codec::Nb => cfa4_amr::nb::FRAME_SAMPLES,
            Codec::Wb => cfa4_amr::wb::FRAME_SAMPLES,
        }
    }

    fn decode(self, data: &[u8]) -> Result<Vec<i16>, cfa4_amr::Error> {
        match self {
            Codec::Nb => cfa4_amr::decode_nb_file(data),
            Codec::Wb => cfa4_amr::decode_wb_file(data),
        }
    }
}

/// Encodes the test signal with opencore / vo-amrwbenc (`mode`, `dtx`) into a storage file.
fn encode(tool: &Path, c: Codec, mode: &str, dtx: bool) -> Option<PathBuf> {
    let dir = fixtures::fixtures_dir("amr");
    let raw = dir.join(format!("{}_source.s16", c.tag()));
    if !raw.exists() {
        write_raw(&raw, &signal(c.rate(), 12));
    }
    let out = dir.join(format!("{}_{mode}_{}.{}", c.tag(), u8::from(dtx), c.ext()));
    let cmd = format!("enc-{}", c.tag());
    fixtures::generate(&out, |tmp| run(tool, &[&cmd, mode, if dtx { "1" } else { "0" }], &[&raw, tmp]))
}

/// opencore's decode of `file`.
fn decode_oracle(tool: &Path, c: Codec, file: &Path) -> Vec<i16> {
    let out = file.with_extension(format!("{}.s16", c.ext()));
    let tmp = fixtures::temp_path(&out);
    let cmd = format!("dec-{}", c.tag());
    assert!(run(tool, &[&cmd], &[file, &tmp]), "oracle decode of {}", file.display());
    let pcm = read_raw(&tmp);
    let _ = std::fs::remove_file(&tmp);
    pcm
}

fn compare(name: &str, frame: usize, ours: &[i16], theirs: &[i16]) {
    assert_eq!(ours.len(), theirs.len(), "{name}: sample count");
    if let Some(i) = ours.iter().zip(theirs).position(|(a, b)| a != b) {
        let f = i / frame;
        panic!(
            "{name}: first mismatch at sample {i} (frame {f}): ours {:?} opencore {:?}",
            &ours[i..(i + 8).min(ours.len())],
            &theirs[i..(i + 8).min(theirs.len())]
        );
    }
}

fn check_file(tool: &Path, c: Codec, name: &str, file: &Path) {
    let data = std::fs::read(file).expect("read amr");
    let ours = c.decode(&data).expect("decode");
    compare(name, c.samples(), &ours, &decode_oracle(tool, c, file));
}

fn every_mode(c: Codec, modes: &[&str]) {
    let Some(tool) = tool() else { return };
    for &mode in modes {
        for dtx in [false, true] {
            let name = format!("{} mode {mode} dtx {dtx}", c.tag());
            let Some(file) = encode(&tool, c, mode, dtx) else {
                oracle::skip("amr oracle", &format!("could not encode {name}"));
                return;
            };
            check_file(&tool, c, &name, &file);
        }
    }
}

#[test]
fn nb_every_mode_matches_opencore() {
    every_mode(Codec::Nb, &["0", "1", "2", "3", "4", "5", "6", "7", "cycle"]);
}

#[test]
fn wb_every_mode_matches_opencore() {
    every_mode(Codec::Wb, &["0", "1", "2", "3", "4", "5", "6", "7", "8", "cycle"]);
}

/// Frames of `data` (after the magic) as `(offset, size)`.
fn frames(c: Codec, data: &[u8]) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    let mut pos = c.magic().len();
    while let Some(&toc) = data.get(pos) {
        let size = c.frame_size(toc);
        if pos + size > data.len() {
            break;
        }
        v.push((pos, size));
        pos += size;
    }
    v
}

/// Lost frames (`NO_DATA`), other frame types, and corrupted payloads, in speech and in DTX:
/// concealment, comfort noise and hostile bits must all match opencore.
fn lost_and_damaged(c: Codec, cases: &[(&str, bool)]) {
    let Some(tool) = tool() else { return };
    let dir = fixtures::fixtures_dir("amr");
    for &(mode, dtx) in cases {
        let Some(file) = encode(&tool, c, mode, dtx) else { return };
        let data = std::fs::read(&file).expect("read");
        let mut out = c.magic().to_vec();
        let mut seed: u32 = 99;
        for (k, (pos, size)) in frames(c, &data).into_iter().enumerate() {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let r = seed >> 24;
            let frame = &data[pos..pos + size];
            match r % 16 {
                // Lose a frame, or a run of frames.
                0 | 1 => out.push(0x7c),
                2 if k % 3 == 0 => out.extend([0x7c; 8]),
                // Another frame type without payload (9-14 for NB, 10-14 for WB).
                3 => out.push(((10 + (r >> 4) % 5) << 3) as u8 | 4),
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
        let damaged = dir.join(format!("{}_damaged_{mode}_{}.{}", c.tag(), u8::from(dtx), c.ext()));
        std::fs::write(&damaged, &out).expect("write");
        check_file(&tool, c, &format!("damaged {} mode {mode} dtx {dtx}", c.tag()), &damaged);
    }
}

#[test]
fn nb_lost_and_damaged_frames_match_opencore() {
    lost_and_damaged(Codec::Nb, &[("7", false), ("0", true), ("cycle", true), ("5", false)]);
}

#[test]
fn wb_lost_and_damaged_frames_match_opencore() {
    lost_and_damaged(Codec::Wb, &[("8", false), ("0", true), ("cycle", true), ("2", false), ("1", true)]);
}

/// Encoder homing frames (every sample 0x0008) in the input make the encoder send decoder
/// homing frames; the decoder must reset and output the homing pattern exactly as opencore.
fn homing(c: Codec, modes: &[&str]) {
    let Some(tool) = tool() else { return };
    let dir = fixtures::fixtures_dir("amr");
    let n = c.samples();
    let mut pcm = signal(c.rate(), 6);
    for f in [0usize, 1, 2, 10, 11, 40, 100, 101, 102, 103, 200] {
        if let Some(fr) = pcm.get_mut(f * n..(f + 1) * n) {
            fr.fill(8);
        }
    }
    let raw = dir.join(format!("{}_homing.s16", c.tag()));
    write_raw(&raw, &pcm);
    for &mode in modes {
        let out = dir.join(format!("{}_homing_{mode}.{}", c.tag(), c.ext()));
        let cmd = format!("enc-{}", c.tag());
        assert!(run(&tool, &[&cmd, mode, "0"], &[&raw, &out]), "oracle encode");
        check_file(&tool, c, &format!("{} homing mode {mode}", c.tag()), &out);
    }
}

#[test]
fn nb_homing_frames_match_opencore() {
    homing(Codec::Nb, &["0", "7", "cycle"]);
}

#[test]
fn wb_homing_frames_match_opencore() {
    homing(Codec::Wb, &["0", "1", "2", "8", "cycle"]);
}

/// Random bytes after the magic: never a panic.
#[test]
fn random_input_never_panics() {
    for c in [Codec::Nb, Codec::Wb] {
        let mut seed: u32 = 7;
        for len in [0usize, 1, 5, 13, 100, 1000, 5000, 20000] {
            let mut data = c.magic().to_vec();
            for _ in 0..len {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                data.push((seed >> 24) as u8);
            }
            let _ = c.decode(&data);
        }
    }
}
