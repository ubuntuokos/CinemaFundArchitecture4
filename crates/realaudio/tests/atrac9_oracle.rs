//! ATRAC9 against FFmpeg's `atrac9` decoder (the external oracle). There is no public ATRAC9
//! sample and no free encoder, so the streams are synthetic: packets whose every block the
//! decoder's strict mode accepts, built block by block from random bits, in every block layout,
//! written as WAV (`.at9`, `WAVE_FORMAT_EXTENSIBLE` with the ATRAC9 sub-format).

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_realaudio::atrac9::Decoder;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/atrac9")
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

/// The 12-byte `fmt ` extradata: version 2, then the configuration word.
fn extradata(sri: u32, layout: u32, frame_size: usize, superframe_idx: u32) -> [u8; 12] {
    let cfg = 0xFEu32 << 24 | sri << 20 | layout << 17 | (frame_size as u32 - 1) << 5 | superframe_idx << 3;
    let mut e = [0u8; 12];
    e[0] = 2;
    e[4..8].copy_from_slice(&cfg.to_be_bytes());
    e
}

/// A WAV with `WAVE_FORMAT_EXTENSIBLE` and the ATRAC9 sub-format, as Sony's `.at9` files.
fn at9(channels: u16, rate: u32, extra: &[u8; 12], ba: usize, packets: &[Vec<u8>]) -> Vec<u8> {
    let data = packets.concat();
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&0xFFFEu16.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(ba as u32 * 100).to_le_bytes());
    fmt.extend_from_slice(&(ba as u16).to_le_bytes());
    fmt.extend_from_slice(&0u16.to_le_bytes());
    fmt.extend_from_slice(&34u16.to_le_bytes());
    fmt.extend_from_slice(&256u16.to_le_bytes());
    fmt.extend_from_slice(&0u32.to_le_bytes());
    fmt.extend_from_slice(&[0xD2, 0x42, 0xE1, 0x47, 0xBA, 0x36, 0x8D, 0x4D, 0x88, 0xFC, 0x61, 0x65, 0x4F, 0x8C, 0x83, 0x6C]);
    fmt.extend_from_slice(extra);
    let mut w = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
    w.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    w.extend_from_slice(&fmt);
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(&data);
    let riff = (w.len() - 8) as u32;
    w[4..8].copy_from_slice(&riff.to_le_bytes());
    w
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

/// A packet of `size` bytes that `d` (left unchanged) decodes in strict mode, grown one block at
/// a time from random bits.
fn valid_packet(d: &Decoder, size: usize, blocks: usize, rng: &mut Rng) -> Option<Vec<u8>> {
    let mut prefix = Vec::new();
    for k in 0..blocks {
        let mut found = false;
        for _ in 0..4000 {
            let mut cand = prefix.clone();
            let fill = size.checked_sub(cand.len())?;
            cand.extend(rng.bytes(fill));
            let mut c = d.clone();
            c.set_strict(true);
            let _ = c.decode(&cand);
            if c.blocks_decoded() > k {
                cand.truncate(c.bits_used() / 8);
                prefix = cand;
                found = true;
                break;
            }
        }
        if !found {
            return None;
        }
    }
    let fill = size - prefix.len();
    prefix.extend(rng.bytes(fill));
    Some(prefix)
}

#[test]
fn synthetic_streams_match_ffmpeg() {
    if !have_ffmpeg() {
        eprintln!("skipped: no ffmpeg");
        return;
    }
    std::fs::create_dir_all(fixtures()).unwrap();
    // (block layout, sample rate index, frame bytes, superframe index, block count per frame)
    let cases = [
        (0u32, 7u32, 256usize, 2u32, 1usize),
        (1, 4, 300, 0, 2),
        (2, 7, 384, 2, 1),
        (2, 0, 200, 0, 1),
        (3, 6, 700, 0, 4),
        (4, 7, 900, 0, 5),
        (5, 12, 600, 2, 2),
        (2, 13, 500, 0, 1),
    ];
    for (n, &(layout, sri, fs, sfi, blocks)) in cases.iter().enumerate() {
        let extra = extradata(sri, layout, fs, sfi);
        let ba = fs << sfi;
        let mut d = Decoder::new(&extra, ba).unwrap();
        d.set_strict(true);
        let frames = 1usize << sfi;
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ ((n as u64 + 1) * 0x1234_5678_9ABC));
        let mut packets = Vec::new();
        let mut ours = Vec::new();
        let ch = d.channels();
        let spf = d.samples_per_channel();
        let mut peaks = Vec::new();
        while packets.len() < 60 {
            let Some(p) = valid_packet(&d, ba, blocks * frames, &mut rng) else { continue };
            let out = d.decode(&p).unwrap();
            for i in 0..spf {
                ours.extend(out.iter().map(|c| c[i]));
            }
            peaks.push(out.iter().flatten().fold(1f32, |m, v| m.max(v.abs())));
            packets.push(p);
        }
        let name = format!("random_{layout}_{sri}_{sfi}.at9");
        let path = fixtures().join(&name);
        std::fs::write(&path, at9(d.channels() as u16, d.sample_rate(), &extra, ba, &packets)).unwrap();
        let want = ffmpeg_f32(&path);
        assert_eq!(ours.len(), want.len(), "{name}: sample count");
        let mut worst = 0f32;
        for (i, (a, b)) in ours.iter().zip(&want).enumerate() {
            let rel = (a - b).abs() / peaks[i / (spf * ch)];
            worst = worst.max(rel);
            assert!(rel <= 1e-5, "{name}: sample {i}: ours {a} ffmpeg {b}");
        }
        eprintln!("{name}: {} samples, worst relative difference {worst:e}", ours.len());
    }
}
