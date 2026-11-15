//! Shorten streams against ffmpeg's `shorten` decoder (the code this crate is ported from) and
//! against the PCM they encode (Shorten is lossless).
//!
//! No Shorten encoder ships with ffmpeg, so `Encoder` below writes the streams: every version
//! (0–3), the fixed predictors, quantised LPC, zero blocks, running means, bit shifts, a smaller
//! last block, 8-bit and 16-bit PCM behind WAVE and AIFF headers. ffmpeg's FATE sample
//! `luckynight-partial.shn` (optional local fixture, `$SHORTEN_SAMPLES` or
//! `target/fixtures/shorten`) is compared too.

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_shorten::{SampleFormat, decode_file};

/// An MSB-first bit writer with the Shorten Rice codes.
#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
    bits: u64,
}

impl Writer {
    fn bit(&mut self, b: u32) {
        if self.bits.is_multiple_of(8) {
            self.bytes.push(0);
        }
        if b != 0 {
            let last = self.bytes.len() - 1;
            self.bytes[last] |= 0x80 >> (self.bits % 8);
        }
        self.bits += 1;
    }

    fn put(&mut self, v: u32, n: u32) {
        for i in (0..n).rev() {
            self.bit((v >> i) & 1);
        }
    }

    fn ur(&mut self, k: u32, v: u32) {
        for _ in 0..(v >> k) {
            self.bit(0);
        }
        self.bit(1);
        self.put(v & ((1u64 << k) - 1) as u32, k);
    }

    fn sr(&mut self, k: u32, v: i32) {
        self.ur(k + 1, ((v << 1) ^ (v >> 31)) as u32);
    }

    fn uint(&mut self, version: u8, k: u32, v: u32) {
        if version == 0 {
            self.ur(k, v);
        } else {
            let k = 32 - v.leading_zeros();
            let k = k.saturating_sub(2);
            self.ur(2, k);
            self.ur(k, v);
        }
    }
}

/// How one block of one channel is coded.
#[derive(Clone, Copy)]
enum Pred {
    Diff(u32),
    Qlpc(&'static [i32]),
    Zero,
}

struct Config {
    version: u8,
    channels: usize,
    u8_pcm: bool,
    aiff: bool,
    blocksize: usize,
    nmean: usize,
    maxnlpc: usize,
}

/// A Shorten encoder that mirrors the decoder's state so every residual is exact.
struct Encoder {
    cfg: Config,
    w: Writer,
    nwrap: usize,
    hist: Vec<Vec<i32>>,
    offset: Vec<Vec<i32>>,
    bitshift: u32,
}

impl Encoder {
    fn new(cfg: Config, rate: u32) -> Self {
        let mut w = Writer::default();
        for &b in b"ajkg" {
            w.put(u32::from(b), 8);
        }
        let v = cfg.version;
        w.put(u32::from(v), 8);
        w.uint(v, 4, if cfg.u8_pcm { 2 } else { 5 });
        w.uint(v, 0, cfg.channels as u32);
        if v > 0 {
            w.uint(v, 8, cfg.blocksize as u32);
            w.uint(v, 2, cfg.maxnlpc as u32);
            w.uint(v, 0, cfg.nmean as u32);
            w.uint(v, 1, 0);
        }
        let header = if cfg.aiff { aiff_header(cfg.channels as u16, rate, cfg.u8_pcm) } else { wave_header(cfg.channels as u16, rate, cfg.u8_pcm) };
        w.ur(2, 9);
        w.ur(5, header.len() as u32);
        for b in header {
            w.ur(8, u32::from(b));
        }
        let nwrap = 3.max(cfg.maxnlpc);
        let mean = if cfg.u8_pcm { 0x80 } else { 0 };
        let nm = if v == 0 { 1 } else { cfg.nmean.max(1) };
        Self { hist: vec![vec![0; nwrap]; cfg.channels], offset: vec![vec![mean; nm]; cfg.channels], nwrap, w, bitshift: 0, cfg }
    }

    fn coffset(&self, ch: usize) -> i32 {
        let v = self.cfg.version;
        if v == 0 {
            return 0;
        }
        let n = self.cfg.nmean;
        if n == 0 {
            return self.offset[ch][0];
        }
        let mut sum: i32 = if v < 2 { 0 } else { (n / 2) as i32 };
        for &o in &self.offset[ch][..n] {
            sum = sum.wrapping_add(o);
        }
        let c = sum / n as i32;
        if v >= 2 && self.bitshift != 0 { c >> (self.bitshift - 1) >> 1 } else { c }
    }

    fn bitshift(&mut self, b: u32) {
        self.w.ur(2, 6);
        self.w.ur(2, b);
        self.bitshift = b;
    }

    fn blocksize(&mut self, n: usize) {
        self.w.ur(2, 5);
        let k = 31 - (self.cfg.blocksize as u32 | 1).leading_zeros();
        self.w.uint(self.cfg.version, k, n as u32);
        self.cfg.blocksize = n;
    }

    /// Codes one channel's block; `block` holds the samples after the bit shift is undone.
    fn block(&mut self, ch: usize, pred: Pred, block: &[i32]) {
        let v = self.cfg.version;
        let shifted: Vec<i32> = block.iter().map(|&s| s >> self.bitshift).collect();
        let coffset = self.coffset(ch);
        let nwrap = self.nwrap;
        let mut h = self.hist[ch].clone();
        h.extend_from_slice(&shifted);
        if matches!(pred, Pred::Zero) {
            assert!(block.iter().all(|&s| s == 0));
            self.w.ur(2, 8);
        }
        if !matches!(pred, Pred::Zero) {
            let (cmd, order, coeffs, qshift): (u32, usize, Vec<i32>, u32) = match pred {
                Pred::Diff(o) => (o, o as usize, [[0, 0, 0], [1, 0, 0], [2, -1, 0], [3, -3, 1]][o as usize][..o as usize].to_vec(), 0),
                Pred::Qlpc(c) => (7, c.len(), c.to_vec(), 5),
                Pred::Zero => unreachable!(),
            };
            let qlpc = cmd == 7;
            let mut hist: Vec<i32> = h.clone();
            if qlpc && coffset != 0 {
                for x in &mut hist[nwrap - order..nwrap] {
                    *x = x.wrapping_sub(coffset);
                }
                for x in &mut hist[nwrap..] {
                    *x = x.wrapping_sub(coffset);
                }
            }
            let init = if order == 0 {
                coffset
            } else if qlpc {
                if v > 1 { 32 } else { 0 }
            } else {
                0
            };
            let res: Vec<i32> = (0..shifted.len())
                .map(|i| {
                    let at = nwrap + i;
                    let mut sum = init;
                    for (j, c) in coeffs.iter().enumerate() {
                        sum = sum.wrapping_add(c.wrapping_mul(hist[at - j - 1]));
                    }
                    hist[at].wrapping_sub(sum >> qshift)
                })
                .collect();
            let mean = res.iter().map(|r| i64::from(r.unsigned_abs())).sum::<i64>() / res.len().max(1) as i64;
            let rs = (64 - (mean as u64).leading_zeros()).min(20);
            self.w.ur(2, cmd);
            self.w.ur(3, if v == 0 { rs + 1 } else { rs });
            if qlpc {
                self.w.ur(2, order as u32);
                for &c in &coeffs {
                    self.w.sr(5, c);
                }
            }
            for r in res {
                self.w.sr(rs, r);
            }
        }
        // the decoder's state after the block
        let bs = shifted.len();
        if v > 0 && self.cfg.nmean > 0 {
            let n = self.cfg.nmean;
            let mut sum: i64 = if v < 2 { 0 } else { (bs / 2) as i64 };
            sum += shifted.iter().map(|&s| i64::from(s)).sum::<i64>();
            let off = &mut self.offset[ch];
            off.copy_within(1..n, 0);
            let m = sum / bs as i64;
            off[n - 1] = if v < 2 {
                m as i32
            } else if self.bitshift == 32 {
                0
            } else {
                (m * (1i64 << self.bitshift)) as i32
            };
        }
        self.hist[ch] = h[bs..bs + nwrap].to_vec();
    }

    fn finish(mut self) -> Vec<u8> {
        self.w.ur(2, 4);
        self.w.bytes
    }
}

fn wave_header(channels: u16, rate: u32, u8_pcm: bool) -> Vec<u8> {
    let bps: u16 = if u8_pcm { 8 } else { 16 };
    let block = channels * bps / 8;
    let mut h = Vec::new();
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&0u32.to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&channels.to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
    h.extend_from_slice(&block.to_le_bytes());
    h.extend_from_slice(&bps.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&0u32.to_le_bytes());
    h
}

fn aiff_header(channels: u16, rate: u32, u8_pcm: bool) -> Vec<u8> {
    let bps: u16 = if u8_pcm { 8 } else { 16 };
    let mut h = Vec::new();
    h.extend_from_slice(b"FORM");
    h.extend_from_slice(&0u32.to_be_bytes());
    h.extend_from_slice(b"AIFFCOMM");
    h.extend_from_slice(&18u32.to_be_bytes());
    h.extend_from_slice(&channels.to_be_bytes());
    h.extend_from_slice(&0u32.to_be_bytes());
    h.extend_from_slice(&bps.to_be_bytes());
    // 80-bit extended rate
    let e = 31 - rate.leading_zeros();
    h.extend_from_slice(&(16383 + e as u16).to_be_bytes());
    h.extend_from_slice(&(u64::from(rate) << (63 - e)).to_be_bytes());
    h.extend_from_slice(b"SSND");
    h.extend_from_slice(&8u32.to_be_bytes());
    h.extend_from_slice(&[0; 8]);
    h
}

/// A test signal for `ch`: tones, noise and silence; 8-bit values are unsigned.
fn signal(n: usize, ch: usize, u8_pcm: bool, seed: u32) -> Vec<i32> {
    let mut x = seed.wrapping_add(ch as u32 * 7919) | 1;
    (0..n)
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let t = i as f64 / 8000.0;
            let seg = (i / 700) % 4;
            let v = match seg {
                0 => 9000.0 * (std::f64::consts::TAU * (300.0 + 50.0 * ch as f64) * t).sin(),
                1 => f64::from((x >> 16) as i16) / 3.0,
                2 => 0.0,
                _ => 30000.0 * (std::f64::consts::TAU * 2500.0 * t).sin() + f64::from((x >> 20) as i16 as i32 % 300),
            };
            if u8_pcm { (128.0 + v / 256.0).clamp(0.0, 255.0) as i32 } else { v.clamp(-32768.0, 32767.0) as i32 }
        })
        .collect()
}

const QLPC2: &[i32] = &[50, -20];
const QLPC4: &[i32] = &[60, -40, 20, -8];

/// Encodes `pcm` (one plane per channel) cycling through the predictors, with a bit-shifted
/// stretch where the samples allow it, a zero block where they are zero, and a smaller last block.
fn encode(cfg: Config, rate: u32, pcm: &[Vec<i32>]) -> Vec<u8> {
    let bs = cfg.blocksize;
    let channels = cfg.channels;
    let qlpc_ok = cfg.maxnlpc >= 4;
    let u8_pcm = cfg.u8_pcm;
    let mut e = Encoder::new(cfg, rate);
    let n = pcm[0].len();
    let mut start = 0;
    let mut k = 0usize;
    while start < n {
        let len = (n - start).min(bs);
        if len < e.cfg.blocksize {
            e.blocksize(len);
        }
        // a bit shift where every sample of the block is even (and not for 8-bit unsigned)
        let even = !u8_pcm && pcm.iter().all(|p| p[start..start + len].iter().all(|v| v % 2 == 0));
        let want = if even { 1 } else { 0 };
        if want != e.bitshift {
            e.bitshift(want);
        }
        for (ch, plane) in pcm.iter().enumerate().take(channels) {
            let block = &plane[start..start + len];
            let pred = if block.iter().all(|&v| v == 0) && k.is_multiple_of(2) {
                Pred::Zero
            } else {
                match (k + ch) % 6 {
                    0 => Pred::Diff(0),
                    1 => Pred::Diff(1),
                    2 => Pred::Diff(2),
                    3 => Pred::Diff(3),
                    4 if qlpc_ok => Pred::Qlpc(QLPC4),
                    _ if qlpc_ok => Pred::Qlpc(QLPC2),
                    _ => Pred::Diff(2),
                }
            };
            e.block(ch, pred, block);
        }
        start += len;
        k += 1;
    }
    e.finish()
}

fn ffmpeg_decode(ff: &Path, data: &[u8], tag: &str, u8_pcm: bool) -> Option<Vec<i32>> {
    let dir = std::env::temp_dir().join(format!("cfa4-shorten-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let input = dir.join("in.shn");
    std::fs::write(&input, data).ok()?;
    let out = Command::new(ff)
        .args(["-v", "error", "-f", "shn", "-i"])
        .arg(&input)
        .args(if u8_pcm { ["-f", "u8", "-c:a", "pcm_u8", "-"] } else { ["-f", "s16le", "-c:a", "pcm_s16le", "-"] })
        .output()
        .ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    if !out.status.success() {
        return None;
    }
    Some(if u8_pcm {
        out.stdout.iter().map(|&b| i32::from(b)).collect()
    } else {
        out.stdout.chunks_exact(2).map(|c| i32::from(i16::from_le_bytes([c[0], c[1]]))).collect()
    })
}

fn interleave(planes: &[Vec<i32>]) -> Vec<i32> {
    let n = planes.first().map_or(0, Vec::len);
    (0..n).flat_map(|i| planes.iter().map(move |p| p[i])).collect()
}

#[test]
fn encoded_streams_match_ffmpeg_and_the_source() {
    let ff = cfa4_testkit::oracle::ffmpeg_or_skip("shorten oracle");
    let cases = [
        // (version, channels, u8, aiff, blocksize, nmean, maxnlpc)
        (2u8, 2usize, false, false, 256usize, 4usize, 4usize),
        (3, 2, false, false, 512, 4, 8),
        (2, 1, false, true, 256, 0, 4),
        (1, 2, false, false, 256, 4, 4),
        (1, 1, true, false, 128, 4, 0),
        (0, 2, false, false, 256, 0, 0),
        (0, 1, true, true, 256, 0, 0),
        (3, 6, false, false, 300, 2, 4),
    ];
    for (i, &(version, channels, u8_pcm, aiff, blocksize, nmean, maxnlpc)) in cases.iter().enumerate() {
        let tag =
            format!("v{version}-{channels}ch-{}-{}-bs{blocksize}-m{nmean}-lpc{maxnlpc}", if u8_pcm { "u8" } else { "s16" }, if aiff { "aiff" } else { "wav" });
        let n = 5000 + 37 * i;
        let pcm: Vec<Vec<i32>> = (0..channels).map(|c| signal(n, c, u8_pcm, 0x5eed + i as u32)).collect();
        // version 0 has no block-size header; it keeps the default 256
        let bs = if version == 0 { 256 } else { blocksize };
        let cfg = Config {
            version,
            channels,
            u8_pcm,
            aiff,
            blocksize: bs,
            nmean: if version == 0 { 0 } else { nmean },
            maxnlpc: if version == 0 { 0 } else { maxnlpc },
        };
        let data = encode(cfg, 44100, &pcm);
        let (info, planes) = decode_file(&data).unwrap_or_else(|e| panic!("{tag}: {e}"));
        assert_eq!(usize::from(info.channels), channels, "{tag}");
        assert_eq!(info.sample_rate, 44100, "{tag}");
        assert_eq!(info.format, if u8_pcm { SampleFormat::U8 } else { SampleFormat::S16 }, "{tag}");
        assert_eq!(planes, pcm, "{tag}: lossless round trip");
        if let Some(ff) = &ff {
            let theirs = ffmpeg_decode(ff, &data, &tag, u8_pcm).unwrap_or_else(|| panic!("{tag}: ffmpeg failed"));
            assert_eq!(interleave(&planes), theirs, "{tag}: against ffmpeg");
        }
    }
}

#[test]
fn fate_sample_matches_ffmpeg() {
    let dir = std::env::var_os("SHORTEN_SAMPLES").map(PathBuf::from).unwrap_or_else(|| cfa4_testkit::fixtures::fixtures_root().join("shorten"));
    let Ok(data) = std::fs::read(dir.join("luckynight-partial.shn")) else {
        eprintln!("luckynight-partial.shn: optional FATE sample not present");
        return;
    };
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("shorten FATE") else { return };
    let (info, planes) = decode_file(&data).expect("decode");
    assert_eq!((info.channels, info.sample_rate), (2, 44100));
    let theirs = ffmpeg_decode(&ff, &data, "fate", false).expect("ffmpeg");
    let ours = interleave(&planes);
    assert_eq!(ours.len(), theirs.len());
    let first = ours.iter().zip(&theirs).position(|(a, b)| a != b);
    assert_eq!(first, None, "first difference");
}

#[test]
fn damaged_streams_never_panic() {
    let pcm = vec![signal(3000, 0, false, 7), signal(3000, 1, false, 8)];
    let data = encode(Config { version: 2, channels: 2, u8_pcm: false, aiff: false, blocksize: 256, nmean: 4, maxnlpc: 4 }, 8000, &pcm);
    let mut x = 0x1234_5678_9abc_def0u64;
    for round in 0..3000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let mut d = data.clone();
        match round % 3 {
            0 => d.truncate((x as usize) % d.len()),
            1 => {
                for _ in 0..1 + x % 8 {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    let p = (x as usize) % d.len();
                    d[p] ^= (x >> 32) as u8 | 1;
                }
            }
            _ => {
                let p = 5 + (x as usize) % (d.len() - 5);
                d[p..].iter_mut().for_each(|b| *b = (x >> 24) as u8);
            }
        }
        let _ = decode_file(&d);
    }
    for d in [&b""[..], b"ajkg", b"ajkg\x02", &[0xff; 64]] {
        assert!(decode_file(d).is_err());
    }
}
