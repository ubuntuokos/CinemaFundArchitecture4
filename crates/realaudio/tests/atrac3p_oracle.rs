//! ATRAC3plus against FFmpeg's `atrac3plus` decoder (the external oracle): FATE samples in
//! OpenMG files when present (`target/fixtures/atrac3p`, local only), and synthetic frames.

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_realaudio::atrac3p::Decoder;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/atrac3p")
}

fn have_ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success())
}

/// ffmpeg's decode as interleaved f32.
fn ffmpeg_f32(path: &Path) -> Vec<f32> {
    let o = Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(path).args(["-f", "f32le", "-"]).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    o.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
}

/// (channels, frame size, frames) of an OpenMG ATRAC3plus file.
fn oma(b: &[u8]) -> (u16, usize, &[u8]) {
    let mut at = 0;
    if &b[..3] == b"ea3" {
        at = (usize::from(b[6] & 0x7F) << 21 | usize::from(b[7] & 0x7F) << 14 | usize::from(b[8] & 0x7F) << 7 | usize::from(b[9] & 0x7F)) + 10;
    }
    let h = &b[at..at + 96];
    assert_eq!(&h[..3], b"EA3");
    assert_eq!(h[32], 1, "not ATRAC3plus");
    let p = u32::from(h[33]) << 16 | u32::from(h[34]) << 8 | u32::from(h[35]);
    let ch = [1u16, 2, 3, 4, 6, 7, 8][((p >> 10) & 7) as usize - 1];
    (ch, (p & 0x3FF) as usize * 8 + 8, &b[at + 96..])
}

fn decode_all(d: &mut Decoder, frames: &[u8]) -> Vec<f32> {
    let ch = d.channels();
    let mut out = Vec::new();
    // a short last packet decodes too, as ffmpeg's demuxer passes it on
    for f in frames.chunks(d.block_align()) {
        if let Ok(p) = d.decode(f) {
            for i in 0..2048 {
                out.extend(p.iter().take(ch).map(|c| c[i]));
            }
        }
    }
    out
}

fn compare(name: &str, ours: &[f32], want: &[f32], tol: f32) {
    assert_eq!(ours.len(), want.len(), "{name}: sample count");
    let mut worst = 0f32;
    for (i, (a, b)) in ours.iter().zip(want).enumerate() {
        let d = (a - b).abs();
        worst = worst.max(d);
        assert!(d <= tol, "{name}: sample {i}: {a} vs {b}");
    }
    eprintln!("{name}: {} samples, worst difference {worst:e}", ours.len());
}

#[test]
fn fate_samples_match_ffmpeg() {
    if !have_ffmpeg() {
        eprintln!("skipped: no ffmpeg");
        return;
    }
    for name in ["at3p_sample1.oma", "sonateno14op27-2-cut.aa3"] {
        let path = fixtures().join(name);
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("skipped {name}: no FATE sample");
            continue;
        };
        let (ch, ba, frames) = oma(&bytes);
        let mut d = Decoder::new(ch, ba).unwrap();
        compare(name, &decode_all(&mut d, frames), &ffmpeg_f32(&path), 1e-5);
    }
}

/// An OpenMG file (EA3 header, no ID3 tag) holding `frames` of `ba` bytes.
fn write_oma(chan_id: u32, ba: usize, frames: &[Vec<u8>]) -> Vec<u8> {
    let mut h = vec![0u8; 96];
    h[..3].copy_from_slice(b"EA3");
    h[3] = 1;
    h[5] = 96;
    h[6] = 0xFF;
    h[7] = 0xFF;
    h[32] = 1;
    let p = 1 << 13 | chan_id << 10 | ((ba - 8) / 8) as u32;
    h[33..36].copy_from_slice(&p.to_be_bytes()[1..]);
    for f in frames {
        h.extend_from_slice(f);
    }
    h
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }
}

/// Random frames that decode with fully defined behaviour (strict mode), so ffmpeg must give
/// the same samples; they reach every coding mode of the bitstream with random parameters.
fn synthetic_frames(channels: u16, ba: usize, want: usize, seed: u64) -> Vec<Vec<u8>> {
    let mut rng = Rng(seed);
    let first_unit = if channels == 1 { 0 } else { 1u8 };
    let mut out = Vec::new();
    let mut tries = 0;
    while out.len() < want && tries < 400_000 {
        tries += 1;
        let mut f: Vec<u8> = (0..ba).map(|_| rng.next() as u8).collect();
        f[0] = (f[0] & 0x1F) | first_unit << 5;
        let mut d = Decoder::new(channels, ba).unwrap();
        d.set_strict(true);
        if d.decode(&f).is_ok() {
            out.push(f);
        }
    }
    eprintln!("{channels} channels: {} frames from {tries} tries", out.len());
    out
}

/// MSB-first bit writer.
struct BitWriter(Vec<u8>, usize);

impl BitWriter {
    fn put(&mut self, bit: bool) {
        if self.1.is_multiple_of(8) {
            self.0.push(0);
        }
        if bit {
            let n = self.0.len();
            self.0[n - 1] |= 0x80 >> (self.1 % 8);
        }
        self.1 += 1;
    }

    fn bits(&mut self, v: u32, n: u32) {
        for i in (0..n).rev() {
            self.put(v >> i & 1 != 0);
        }
    }

    fn copy(&mut self, src: &[u8], from: usize, to: usize) {
        for p in from..to {
            self.put(src[p / 8] >> (7 - p % 8) & 1 != 0);
        }
    }
}

/// The channel unit bits (after the unit id) of random valid one-unit frames.
fn unit_pool(stereo: bool, ba: usize, want: usize, seed: u64) -> Vec<(Vec<u8>, usize)> {
    let channels = if stereo { 2 } else { 1 };
    synthetic_frames(channels, ba, want, seed)
        .into_iter()
        .map(|f| {
            let mut d = Decoder::new(channels, ba).unwrap();
            d.decode(&f).unwrap();
            // start bit, unit id, ..., terminator
            let end = d.bits_used() - 2;
            let mut w = BitWriter(Vec::new(), 0);
            w.copy(&f, 3, end);
            (w.0, end - 3)
        })
        .collect()
}

/// Multichannel frames assembled from valid mono and stereo units, in the unit order of
/// the layout (stereo, mono, …).
fn multichannel_frames(layout: &[bool], ba: usize, want: usize, seed: u64) -> Vec<Vec<u8>> {
    let mono = unit_pool(false, 300, 24, seed);
    let stereo = unit_pool(true, 500, 24, seed + 100);
    let mut rng = Rng(seed);
    let mut out = Vec::new();
    while out.len() < want {
        let mut w = BitWriter(Vec::new(), 0);
        w.put(false);
        for &s in layout {
            let pool = if s { &stereo } else { &mono };
            let (bits, n) = &pool[rng.next() as usize % pool.len()];
            w.bits(u32::from(s), 2);
            w.copy(bits, 0, *n);
        }
        w.bits(3, 2);
        if w.1 > ba * 8 {
            continue;
        }
        while w.0.len() < ba {
            w.0.push(rng.next() as u8);
        }
        let mut d = Decoder::new(layout.iter().map(|&s| if s { 2 } else { 1 }).sum(), ba).unwrap();
        d.set_strict(true);
        d.decode(&w.0).unwrap();
        out.push(w.0);
    }
    out
}

#[test]
fn synthetic_frames_match_ffmpeg() {
    let n1 = 60;
    if !have_ffmpeg() {
        eprintln!("skipped: no ffmpeg");
        return;
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("atrac3p");
    std::fs::create_dir_all(&dir).unwrap();
    let (s, m) = (true, false);
    let layouts: [&[bool]; 5] = [&[s, m], &[s, m, m], &[s, m, s, m], &[s, m, s, m, m], &[s, m, s, s, m]];
    for (chan_id, channels, ba, seed) in
        [(1u32, 1u16, 376usize, 1u64), (2, 2, 1488, 2), (2, 2, 280, 3), (3, 3, 1000, 4), (4, 4, 1200, 5), (5, 6, 1600, 6), (6, 7, 2000, 7), (7, 8, 2400, 8)]
    {
        let frames =
            if channels <= 2 { synthetic_frames(channels, ba, n1, seed) } else { multichannel_frames(layouts[chan_id as usize - 3], ba, n1 * 3 / 4, seed) };
        assert!(frames.len() >= 10, "{channels} channels: too few valid frames");
        let path = dir.join(format!("synthetic_{channels}_{ba}.oma"));
        std::fs::write(&path, write_oma(chan_id, ba, &frames)).unwrap();
        let mut d = Decoder::new(channels, ba).unwrap();
        let ours = decode_all(&mut d, &frames.concat());
        let want = ffmpeg_f32(&path);
        assert_eq!(ours.len(), want.len(), "{channels} channels: sample count");
        // random spectra are loud: compare within each frame relative to its peak
        let per = 2048 * usize::from(channels);
        let mut worst = 0f32;
        for (k, (a, b)) in ours.chunks(per).zip(want.chunks(per)).enumerate() {
            let peak = b.iter().fold(1f32, |m, v| m.max(v.abs()));
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                let e = (x - y).abs() / peak;
                worst = worst.max(e);
                assert!(e <= 1e-5, "{channels} channels, frame {k}, sample {i}: {x} vs {y}");
            }
        }
        eprintln!("{channels} channels at {ba} bytes: {} frames, worst relative difference {worst:e}", frames.len());
    }
}
