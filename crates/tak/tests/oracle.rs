//! TAK streams from the test's own encoder against ffmpeg's `tak` decoder (the only TAK oracle:
//! ffmpeg has no TAK encoder), and ffmpeg's FATE sample `luckynight-partial.tak` (optional local
//! fixture, `$TAK_SAMPLES` or `target/fixtures/tak`).
//!
//! The encoder writes two kinds of streams: real audio coded losslessly with simple choices
//! (checked against the source and ffmpeg), and frames whose every syntax element is drawn at
//! random within the bitstream's rules (LPC subframes, windowed residuals with escape codes,
//! sample shifts, every decorrelation mode, multichannel decorrelation), checked against ffmpeg.

use cfa4_tak::{Decoder, crc24, decode_file};
use std::path::{Path, PathBuf};
use std::process::Command;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
}

/// Little-endian bit writer (the first bit written is the lowest of a value).
#[derive(Default)]
struct Bw {
    bytes: Vec<u8>,
    nbits: usize,
}

impl Bw {
    fn put(&mut self, v: u64, n: u32) {
        for i in 0..n {
            if self.nbits.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if (v >> i) & 1 == 1 {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 1 << (self.nbits % 8);
            }
            self.nbits += 1;
        }
    }
    fn sput(&mut self, v: i64, n: u32) {
        self.put(v as u64 & ((1u64 << n) - 1), n);
    }
    fn align(&mut self) {
        self.nbits = self.bytes.len() * 8;
    }
    fn bytes(&mut self, b: &[u8]) {
        self.align();
        self.bytes.extend_from_slice(b);
        self.nbits = self.bytes.len() * 8;
    }
    /// The CRC of the bytes from `from`, as three bytes.
    fn crc_from(&mut self, from: usize) {
        self.align();
        let c = crc24(&self.bytes[from..]);
        self.bytes(&[(c >> 16) as u8, (c >> 8) as u8, c as u8]);
    }
}

/// ffmpeg's xcodes rows: (init, escape, scale, aescape, bias).
fn xcode(mode: u32) -> (u32, u32, u32, u32, u32) {
    const ROWS: [(u32, u32, u32, u32, u32); 50] = [
        (0x01, 0x0000001, 0x0000001, 0x0000003, 0x0000008),
        (0x02, 0x0000003, 0x0000001, 0x0000007, 0x0000006),
        (0x03, 0x0000005, 0x0000002, 0x000000E, 0x000000D),
        (0x03, 0x0000003, 0x0000003, 0x000000D, 0x0000018),
        (0x04, 0x000000B, 0x0000004, 0x000001C, 0x0000019),
        (0x04, 0x0000006, 0x0000006, 0x000001A, 0x0000030),
        (0x05, 0x0000016, 0x0000008, 0x0000038, 0x0000032),
        (0x05, 0x000000C, 0x000000C, 0x0000034, 0x0000060),
        (0x06, 0x000002C, 0x0000010, 0x0000070, 0x0000064),
        (0x06, 0x0000018, 0x0000018, 0x0000068, 0x00000C0),
        (0x07, 0x0000058, 0x0000020, 0x00000E0, 0x00000C8),
        (0x07, 0x0000030, 0x0000030, 0x00000D0, 0x0000180),
        (0x08, 0x00000B0, 0x0000040, 0x00001C0, 0x0000190),
        (0x08, 0x0000060, 0x0000060, 0x00001A0, 0x0000300),
        (0x09, 0x0000160, 0x0000080, 0x0000380, 0x0000320),
        (0x09, 0x00000C0, 0x00000C0, 0x0000340, 0x0000600),
        (0x0A, 0x00002C0, 0x0000100, 0x0000700, 0x0000640),
        (0x0A, 0x0000180, 0x0000180, 0x0000680, 0x0000C00),
        (0x0B, 0x0000580, 0x0000200, 0x0000E00, 0x0000C80),
        (0x0B, 0x0000300, 0x0000300, 0x0000D00, 0x0001800),
        (0x0C, 0x0000B00, 0x0000400, 0x0001C00, 0x0001900),
        (0x0C, 0x0000600, 0x0000600, 0x0001A00, 0x0003000),
        (0x0D, 0x0001600, 0x0000800, 0x0003800, 0x0003200),
        (0x0D, 0x0000C00, 0x0000C00, 0x0003400, 0x0006000),
        (0x0E, 0x0002C00, 0x0001000, 0x0007000, 0x0006400),
        (0x0E, 0x0001800, 0x0001800, 0x0006800, 0x000C000),
        (0x0F, 0x0005800, 0x0002000, 0x000E000, 0x000C800),
        (0x0F, 0x0003000, 0x0003000, 0x000D000, 0x0018000),
        (0x10, 0x000B000, 0x0004000, 0x001C000, 0x0019000),
        (0x10, 0x0006000, 0x0006000, 0x001A000, 0x0030000),
        (0x11, 0x0016000, 0x0008000, 0x0038000, 0x0032000),
        (0x11, 0x000C000, 0x000C000, 0x0034000, 0x0060000),
        (0x12, 0x002C000, 0x0010000, 0x0070000, 0x0064000),
        (0x12, 0x0018000, 0x0018000, 0x0068000, 0x00C0000),
        (0x13, 0x0058000, 0x0020000, 0x00E0000, 0x00C8000),
        (0x13, 0x0030000, 0x0030000, 0x00D0000, 0x0180000),
        (0x14, 0x00B0000, 0x0040000, 0x01C0000, 0x0190000),
        (0x14, 0x0060000, 0x0060000, 0x01A0000, 0x0300000),
        (0x15, 0x0160000, 0x0080000, 0x0380000, 0x0320000),
        (0x15, 0x00C0000, 0x00C0000, 0x0340000, 0x0600000),
        (0x16, 0x02C0000, 0x0100000, 0x0700000, 0x0640000),
        (0x16, 0x0180000, 0x0180000, 0x0680000, 0x0C00000),
        (0x17, 0x0580000, 0x0200000, 0x0E00000, 0x0C80000),
        (0x17, 0x0300000, 0x0300000, 0x0D00000, 0x1800000),
        (0x18, 0x0B00000, 0x0400000, 0x1C00000, 0x1900000),
        (0x18, 0x0600000, 0x0600000, 0x1A00000, 0x3000000),
        (0x19, 0x1600000, 0x0800000, 0x3800000, 0x3200000),
        (0x19, 0x0C00000, 0x0C00000, 0x3400000, 0x6000000),
        (0x1A, 0x2C00000, 0x1000000, 0x7000000, 0x6400000),
        (0x1A, 0x1800000, 0x1800000, 0x6800000, 0xC000000),
    ];
    ROWS[mode as usize - 1]
}

const PREDICTOR_SIZES: [usize; 16] = [4, 8, 12, 16, 24, 32, 48, 64, 80, 96, 128, 160, 192, 224, 256, 0];
const FRAME_QUANTS: [u32; 10] = [3, 4, 6, 8, 4096, 8192, 16384, 512, 1024, 2048];

#[derive(Clone, Copy, Debug)]
struct Config {
    codec: u32,
    channels: usize,
    bps: u32,
    rate: u32,
    frame_type: u32,
}

impl Config {
    fn frame_samples(&self) -> usize {
        let q = FRAME_QUANTS[self.frame_type as usize];
        (if self.frame_type <= 3 { (self.rate * q) >> 5 } else { q }) as usize
    }
    fn align4(&self) -> usize {
        (((self.rate + 511) >> 9) as usize + 3) & !3
    }
    fn uval(&self) -> usize {
        let shift = if self.rate < 11025 {
            3
        } else if self.rate < 22050 {
            2
        } else if self.rate < 44100 {
            1
        } else {
            0
        };
        self.align4() << shift
    }
    fn subframe_scale(&self) -> usize {
        self.align4() << 1
    }
    fn streaminfo(&self, w: &mut Bw, samples: u64, layout: bool) {
        w.put(u64::from(self.codec), 6);
        w.put(0, 4);
        w.put(u64::from(self.frame_type), 4);
        w.put(samples, 35);
        w.put(0, 3);
        w.put(u64::from(self.rate - 6000), 18);
        w.put(u64::from(self.bps - 8), 5);
        w.put(self.channels as u64 - 1, 4);
        w.put(u64::from(layout), 1);
        if layout {
            w.put(0, 5);
            w.put(1, 1);
            for c in 0..self.channels {
                w.put(c as u64 + 1, 6); // FL, FR, FC, LFE, BL, BR
            }
        }
    }
}

/// `tBaK`, a STREAMINFO block, the END block and the frames.
fn file(cfg: &Config, samples: u64, frames: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"tBaK".to_vec();
    let mut si = Bw::default();
    cfg.streaminfo(&mut si, samples, cfg.channels > 2);
    si.align();
    let body = si.bytes;
    let size = body.len() + 3;
    out.extend_from_slice(&[1, size as u8, (size >> 8) as u8, (size >> 16) as u8]);
    out.extend_from_slice(&body);
    let c = crc24(&body);
    out.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8]);
    out.extend_from_slice(&[0, 0, 0, 0]);
    for f in frames {
        out.extend_from_slice(f);
    }
    out
}

fn frame_header(w: &mut Bw, cfg: &Config, num: u32, last: Option<usize>, info: bool, samples: u64, rng: &mut Rng) {
    w.put(0xA0FF, 16);
    w.put(u64::from(last.is_some()) | (u64::from(info) << 1), 3);
    w.put(u64::from(num), 21);
    if let Some(n) = last {
        w.put(n as u64 - 1, 14);
        w.put(0, 2);
    }
    if info {
        cfg.streaminfo(w, samples, cfg.channels > 2); // the same layout as the file: ffmpeg would remix on a change
        if rng.chance(30) {
            w.put(1 + rng.below(63), 6);
            w.put(rng.below(1 << 25), 25);
        } else {
            w.put(0, 6);
        }
    }
    w.crc_from(0);
}

/// The residual code for `zigzag` values below 2^init, with `mode`.
fn code_plain(w: &mut Bw, mode: u32, x: u32) {
    let (init, escape, ..) = xcode(mode);
    w.put(u64::from(x), init);
    if x >= escape {
        w.put(0, 1);
    }
}

fn zigzag(v: i32) -> u32 {
    ((v << 1) ^ (v >> 31)) as u32
}

/// Random residual syntax for one segment.
fn random_segment(w: &mut Bw, mode: u32, len: usize, rng: &mut Rng) {
    if mode == 0 {
        return;
    }
    let (init, escape, _, aescape, _) = xcode(mode);
    for _ in 0..len {
        let x = if rng.chance(85) { rng.below(u64::from(escape)) } else { rng.below(1 << init) } as u32;
        w.put(u64::from(x), init);
        if x >= escape {
            let flag = rng.chance(60);
            w.put(u64::from(flag), 1);
            if flag && (x | 1 << init) >= aescape {
                let s = rng.below(10) as u32;
                w.put(1 << s, s + 1 - u32::from(s == 9)); // s zeros, then a one unless s == 9
                if s == 9 {
                    let sb = if rng.chance(80) { rng.below(7) as u32 } else { 7 + rng.below(4) as u32 };
                    if sb >= 7 {
                        w.put(7, 3);
                        w.put(u64::from(sb - 7), 5);
                    } else {
                        w.put(u64::from(sb), 3);
                    }
                    if sb > 0 {
                        w.put(rng.below(1 << sb), sb);
                    }
                }
            }
        }
    }
}

/// Random residuals of `len` samples (simple or windowed coding).
fn random_residues(w: &mut Bw, cfg: &Config, len: usize, rng: &mut Rng) {
    let maxmode = if cfg.bps > 16 { 50 } else { 30 };
    let uval = cfg.uval();
    let mut wlength = len / uval;
    let mut rval = len - wlength * uval;
    if rval < uval / 2 {
        rval += uval;
    } else {
        wlength += 1;
    }
    if (2..=128).contains(&wlength) && rng.chance(60) {
        w.put(1, 1);
        let mut mode = 1 + rng.below(maxmode) as i64;
        let mut modes = vec![mode as u32];
        w.put(mode as u64, 6);
        for _ in 1..wlength {
            let c = rng.below(7) as i64;
            match c {
                6 => {
                    mode = rng.below(maxmode + 1) as i64;
                    w.put(1 << 6, 6); // six zeros, no stop bit
                    w.put(mode as u64, 6);
                }
                _ => {
                    let delta = match c {
                        1 => -1,
                        2 => 1,
                        3..=5 => c - 1,
                        _ => 0,
                    };
                    let sign = c >= 3 && rng.chance(50);
                    let delta = if sign { -delta } else { delta };
                    if mode + delta < 0 || mode + delta > maxmode as i64 {
                        w.put(1, 1); // c = 0: unchanged
                    } else {
                        w.put(1 << c, c as u32 + 1);
                        if c >= 3 {
                            w.put(u64::from(sign), 1);
                        }
                        mode += delta;
                    }
                }
            }
            modes.push(mode as u32);
        }
        let mut i = 0;
        while i < wlength {
            let m = modes[i];
            let mut n = 0;
            loop {
                n += if i >= wlength - 1 { rval } else { uval };
                i += 1;
                if i == wlength || modes[i] != m {
                    break;
                }
            }
            random_segment(w, m, n, rng);
        }
    } else {
        w.put(0, 1);
        let mode = if rng.chance(5) { 0 } else { 1 + rng.below(maxmode) as u32 };
        w.put(u64::from(mode), 6);
        random_segment(w, mode, len, rng);
    }
}

fn esc4(w: &mut Bw, v: u32) {
    if v == 0 {
        w.put(0, 1);
    } else {
        w.put(1, 1);
        w.put(u64::from(v - 1), 4);
    }
}

fn random_subframe(w: &mut Bw, cfg: &Config, len: usize, prev: usize, rng: &mut Rng) {
    if rng.chance(35) {
        w.put(0, 1);
        random_residues(w, cfg, len, rng);
        return;
    }
    w.put(1, 1);
    let use_prev = prev > 0 && rng.chance(50);
    let limit = if use_prev { prev } else { len };
    let mut choices: Vec<usize> = (0..15).filter(|&i| PREDICTOR_SIZES[i] <= limit).collect();
    if choices.is_empty() || rng.chance(5) {
        choices.push(15); // order 0
    }
    let idx = choices[rng.below(choices.len() as u64) as usize];
    let fo = PREDICTOR_SIZES[idx];
    w.put(idx as u64, 4);
    let rest = if use_prev {
        w.put(1, 1);
        len
    } else {
        if prev > 0 {
            w.put(0, 1);
        }
        w.put(rng.below(3), 2);
        random_residues(w, cfg, fo, rng);
        len - fo
    };
    esc4(w, if rng.chance(60) { 0 } else { 1 + rng.below(8) as u32 });
    let size = 6 + rng.below(2) as u32;
    w.put(u64::from(size - 6), 1);
    if rng.chance(50) {
        w.put(1, 1);
        w.put(rng.below(7), 3);
    } else {
        w.put(0, 1);
    }
    w.sput(rng.below(1024) as i64 - 512, 10);
    w.sput(rng.below(1024) as i64 - 512, 10);
    w.sput(rng.below(1 << size) as i64, size);
    w.sput(rng.below(1 << size) as i64, size);
    if fo > 4 {
        let b = rng.below(2) as u32;
        w.put(u64::from(b), 1);
        let tmp = size - b;
        let mut x = 0;
        for i in 4..fo {
            if i & 3 == 0 {
                let d = rng.below(4) as u32;
                w.put(u64::from(d), 2);
                x = tmp - d;
            }
            w.put(rng.below(1 << x), x);
        }
    }
    random_residues(w, cfg, rest, rng);
}

fn random_channel(w: &mut Bw, cfg: &Config, nb: usize, rng: &mut Rng) {
    let shift = if rng.chance(70) { 0 } else { 1 + rng.below(u64::from(cfg.bps.min(17) - 1)) as u32 };
    esc4(w, shift);
    let n = cfg.bps - shift;
    w.put(rng.below(1 << n), n);
    w.put(rng.below(4), 2);
    let scale = cfg.subframe_scale();
    // subframe boundaries in units of `scale`
    let max_v = ((nb - 2) / scale).min(63);
    let mut bounds: Vec<usize> = Vec::new();
    if max_v >= 1 {
        let want = rng.below(8) as usize;
        let mut vs: Vec<usize> = (0..want).map(|_| 1 + rng.below(max_v as u64) as usize).collect();
        vs.sort_unstable();
        vs.dedup();
        bounds = vs;
    }
    w.put(bounds.len() as u64, 3);
    let mut lens = Vec::new();
    let mut prev = 0;
    for &v in &bounds {
        w.put(v as u64, 6);
        lens.push((v - prev) * scale);
        prev = v;
    }
    lens.push(nb - 1 - prev * scale);
    let mut p = 0;
    for len in lens {
        random_subframe(w, cfg, len, p, rng);
        p = len;
    }
}

fn random_decorrelation(w: &mut Bw, dmode: u32, rng: &mut Rng) {
    match dmode {
        4 | 5 => {
            esc4(w, rng.below(9) as u32);
            w.sput(rng.below(1024) as i64 - 512, 10);
        }
        6 | 7 => {
            esc4(w, rng.below(9) as u32);
            let order = 8 << rng.below(2);
            w.put((order == 16) as u64, 1);
            w.put(rng.below(2), 1);
            w.put(rng.below(2), 1);
            let mut cs = 0;
            for i in 0..order {
                if i & 3 == 0 {
                    let d = rng.below(8) as u32;
                    w.put(u64::from(d), 3);
                    cs = 14 - d;
                }
                w.put(rng.below(1 << cs), cs);
            }
        }
        _ => {}
    }
}

/// A frame whose syntax elements are random but valid.
fn random_frame(cfg: &Config, num: u32, nb: usize, last: bool, total: u64, rng: &mut Rng) -> Vec<u8> {
    let mut w = Bw::default();
    frame_header(&mut w, cfg, num, last.then_some(nb), num == 0 || rng.chance(30), total, rng);
    let hsize = w.bytes.len();
    if nb < 16 {
        for _ in 0..cfg.channels * nb {
            w.put(rng.below(1 << cfg.bps), cfg.bps);
        }
    } else if cfg.codec == 2 {
        for _ in 0..cfg.channels {
            random_channel(&mut w, cfg, nb, rng);
        }
        if cfg.channels == 2 {
            let b = rng.below(2);
            w.put(b, 1);
            if b == 1 {
                w.put(rng.below(64), 6);
            }
            let dmode = if nb > 256 { rng.below(8) } else { rng.below(6) } as u32;
            w.put(u64::from(dmode), 3);
            random_decorrelation(&mut w, dmode, rng);
        }
    } else if rng.chance(25) {
        w.put(0, 1);
        for _ in 0..cfg.channels {
            random_channel(&mut w, cfg, nb, rng);
        }
    } else {
        // (chan1, Some((index, chan2)))
        let mut left: Vec<usize> = (0..cfg.channels).collect();
        for i in (1..left.len()).rev() {
            left.swap(i, rng.below(i as u64 + 1) as usize);
        }
        let mut done: Vec<usize> = Vec::new();
        let mut entries: Vec<(usize, Option<(u32, usize)>)> = Vec::new();
        while let Some(c1) = left.pop() {
            let mut opts: Vec<u32> = Vec::new();
            if !done.is_empty() {
                opts.extend([0, 2]);
                if nb > 256 {
                    opts.push(3);
                }
            }
            if !left.is_empty() {
                opts.push(1);
            }
            let e = if !opts.is_empty() && rng.chance(75) {
                let index = opts[rng.below(opts.len() as u64) as usize];
                let c2 = if index == 1 {
                    let c2 = left.pop().unwrap_or(0);
                    done.push(c2);
                    c2
                } else {
                    done[rng.below(done.len() as u64) as usize]
                };
                Some((index, c2))
            } else {
                None
            };
            done.push(c1);
            entries.push((c1, e));
        }
        w.put(1, 1);
        w.put(entries.len() as u64 - 1, 4);
        for &(c1, e) in &entries {
            w.put(c1 as u64, 4);
            w.put(u64::from(e.is_some()), 1);
            if let Some((index, c2)) = e {
                w.put(u64::from(index), 2);
                w.put(c2 as u64, 4);
            }
        }
        for &(_, e) in &entries {
            if let Some((1, _)) = e {
                random_channel(&mut w, cfg, nb, rng);
            }
            random_channel(&mut w, cfg, nb, rng);
            if let Some((index, _)) = e {
                random_decorrelation(&mut w, [1, 3, 4, 6][index as usize], rng);
            }
        }
    }
    w.crc_from(hsize);
    w.bytes
}

/// A frame coding `pcm` losslessly: no LPC subframes, first differences (channel LPC mode 1),
/// plain residual codes, no decorrelation.
fn lossless_frame(cfg: &Config, num: u32, pcm: &[Vec<i32>], start: usize, nb: usize, last: bool, total: u64, rng: &mut Rng) -> Vec<u8> {
    let mut w = Bw::default();
    frame_header(&mut w, cfg, num, last.then_some(nb), num == 0, total, rng);
    let hsize = w.bytes.len();
    let chans: Vec<&[i32]> = pcm.iter().map(|c| &c[start..start + nb]).collect();
    if nb < 16 {
        for c in &chans {
            for &v in *c {
                w.sput(i64::from(v), cfg.bps);
            }
        }
    } else {
        if cfg.codec == 4 {
            w.put(0, 1);
        }
        for c in &chans {
            esc4(&mut w, 0);
            w.sput(i64::from(c[0]), cfg.bps);
            w.put(1, 2); // channel LPC mode 1: running sum
            w.put(0, 3); // one subframe
            w.put(0, 1); // no LPC
            w.put(0, 1); // simple residual coding
            let zz: Vec<u32> = c.windows(2).map(|p| zigzag(p[1] - p[0])).collect();
            let need = 32 - zz.iter().max().copied().unwrap_or(0).leading_zeros();
            let mode = (1..=50).find(|&m| xcode(m).0 >= need.max(1)).unwrap_or(50);
            w.put(u64::from(mode), 6);
            for x in zz {
                code_plain(&mut w, mode, x);
            }
        }
        if cfg.codec == 2 && cfg.channels == 2 {
            w.put(0, 1);
            w.put(0, 3);
        }
    }
    w.crc_from(hsize);
    w.bytes
}

fn signal(n: usize, ch: usize, bps: u32, seed: u64) -> Vec<i32> {
    let amp = f64::from((1u32 << (bps - 1)) - 1) * 0.7;
    let mut rng = Rng(seed | 1);
    (0..n)
        .map(|i| {
            let t = i as f64;
            let v = amp * ((t * (0.01 + 0.003 * ch as f64)).sin() * 0.8 + (t * 0.17).sin() * 0.15) + (rng.below(32) as f64 - 16.0);
            v.clamp(-amp, amp) as i32
        })
        .collect()
}

fn ffmpeg_decode(ff: &Path, data: &[u8], tag: &str, bps: u32) -> Option<Vec<i32>> {
    let dir = std::env::temp_dir().join(format!("cfa4-tak-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let input = dir.join("in.tak");
    std::fs::write(&input, data).ok()?;
    let fmt = match bps {
        8 => ["-f", "u8", "-c:a", "pcm_u8", "-"],
        16 => ["-f", "s16le", "-c:a", "pcm_s16le", "-"],
        _ => ["-f", "s32le", "-c:a", "pcm_s32le", "-"],
    };
    let out = Command::new(ff).args(["-v", "error", "-f", "tak", "-i"]).arg(&input).args(fmt).output().ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    if !out.status.success() {
        return None;
    }
    Some(match bps {
        8 => out.stdout.iter().map(|&b| i32::from(b) - 128).collect(),
        16 => out.stdout.chunks_exact(2).map(|c| i32::from(i16::from_le_bytes([c[0], c[1]]))).collect(),
        _ => out.stdout.chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]) >> 8).collect(),
    })
}

fn interleave(planes: &[Vec<i32>]) -> Vec<i32> {
    let n = planes.first().map_or(0, Vec::len);
    (0..n).flat_map(|i| planes.iter().map(move |p| p[i])).collect()
}

const CASES: [Config; 8] = [
    Config { codec: 2, channels: 1, bps: 16, rate: 44100, frame_type: 4 },
    Config { codec: 2, channels: 2, bps: 16, rate: 48000, frame_type: 0 },
    Config { codec: 2, channels: 2, bps: 8, rate: 22050, frame_type: 7 },
    Config { codec: 2, channels: 2, bps: 24, rate: 96000, frame_type: 5 },
    Config { codec: 4, channels: 6, bps: 24, rate: 48000, frame_type: 8 },
    Config { codec: 4, channels: 3, bps: 16, rate: 8000, frame_type: 7 },
    Config { codec: 2, channels: 1, bps: 8, rate: 6000, frame_type: 3 },
    Config { codec: 4, channels: 4, bps: 8, rate: 32000, frame_type: 9 },
];

fn tag(cfg: &Config) -> String {
    format!("c{}-{}ch-{}bit-{}hz-t{}", cfg.codec, cfg.channels, cfg.bps, cfg.rate, cfg.frame_type)
}

/// The frame lengths of a stream of `n` samples.
fn frame_lens(cfg: &Config, n: usize) -> Vec<usize> {
    let fs = cfg.frame_samples();
    let mut v = vec![fs; n / fs];
    if !n.is_multiple_of(fs) {
        v.push(n % fs);
    }
    v
}

#[test]
fn lossless_streams_match_the_source_and_ffmpeg() {
    let ff = cfa4_testkit::oracle::ffmpeg_or_skip("tak oracle");
    for (i, cfg) in CASES.iter().enumerate() {
        let tag = tag(cfg);
        let fs = cfg.frame_samples();
        // whole frames plus a short (< 16) or a long last frame
        let n = 3 * fs + if i % 2 == 0 { 7 } else { fs / 2 + 3 };
        let pcm: Vec<Vec<i32>> = (0..cfg.channels).map(|c| signal(n, c, cfg.bps, 0x7a4 + i as u64 * 31 + c as u64)).collect();
        let mut rng = Rng(99 + i as u64);
        let lens = frame_lens(cfg, n);
        let mut frames = Vec::new();
        let mut start = 0;
        for (k, &len) in lens.iter().enumerate() {
            frames.push(lossless_frame(cfg, k as u32, &pcm, start, len, k + 1 == lens.len(), n as u64, &mut rng));
            start += len;
        }
        let data = file(cfg, n as u64, &frames);
        let (info, planes) = decode_file(&data).unwrap_or_else(|e| panic!("{tag}: {e}"));
        assert_eq!((info.sample_rate, usize::from(info.channels), u32::from(info.bits_per_sample)), (cfg.rate, cfg.channels, cfg.bps), "{tag}");
        assert_eq!(planes, pcm, "{tag}: lossless");
        if let Some(ff) = &ff {
            let theirs = ffmpeg_decode(ff, &data, &tag, cfg.bps).unwrap_or_else(|| panic!("{tag}: ffmpeg failed"));
            assert_eq!(interleave(&planes), theirs, "{tag}: against ffmpeg");
        }
    }
}

#[test]
fn random_syntax_streams_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("tak oracle") else { return };
    for (i, cfg) in CASES.iter().enumerate() {
        for round in 0..8u64 {
            let tag = format!("{}-r{round}", tag(cfg));
            let mut rng = Rng(0x51ab_0000 + i as u64 * 1000 + round);
            let fs = cfg.frame_samples();
            let nframes = 2 + rng.below(4) as usize;
            let last = if rng.chance(30) { 1 + rng.below(15) as usize } else { 1 + rng.below(fs as u64) as usize };
            let total = ((nframes - 1) * fs + last) as u64;
            let frames: Vec<Vec<u8>> =
                (0..nframes).map(|k| random_frame(cfg, k as u32, if k + 1 == nframes { last } else { fs }, k + 1 == nframes, total, &mut rng)).collect();
            let data = file(cfg, total, &frames);
            let (_, planes) = decode_file(&data).unwrap_or_else(|e| panic!("{tag}: {e}"));
            assert_eq!(planes[0].len() as u64, total, "{tag}");
            let theirs = ffmpeg_decode(&ff, &data, &tag, cfg.bps).unwrap_or_else(|| panic!("{tag}: ffmpeg failed"));
            let ours = interleave(&planes);
            assert_eq!(ours.len(), theirs.len(), "{tag}");
            let first = ours.iter().zip(&theirs).position(|(a, b)| a != b);
            if let Some(f) = first {
                let diffs: Vec<_> = ours.iter().zip(&theirs).enumerate().filter(|(_, (a, b))| a != b).map(|(k, (a, b))| (k, *a, *b)).take(12).collect();
                let n = ours.iter().zip(&theirs).filter(|(a, b)| a != b).count();
                if let Some(d) = std::env::var_os("TAK_DUMP") {
                    std::fs::write(Path::new(&d).join(format!("{tag}.tak")), &data).ok();
                }
                panic!("{tag}: first difference at {f}, {n} differ: {diffs:?}");
            }
        }
    }
}

#[test]
fn fate_sample_matches_ffmpeg() {
    let dir = std::env::var_os("TAK_SAMPLES").map(PathBuf::from).unwrap_or_else(|| cfa4_testkit::fixtures::fixtures_root().join("tak"));
    let Ok(data) = std::fs::read(dir.join("luckynight-partial.tak")) else {
        eprintln!("luckynight-partial.tak: optional FATE sample not present");
        return;
    };
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("tak FATE") else { return };
    let (info, planes) = decode_file(&data).expect("decode");
    assert_eq!((info.channels, info.sample_rate, info.bits_per_sample), (2, 44100, 16));
    let theirs = ffmpeg_decode(&ff, &data, "fate", 16).expect("ffmpeg");
    let ours = interleave(&planes);
    assert_eq!(ours.len(), theirs.len());
    let first = ours.iter().zip(&theirs).position(|(a, b)| a != b);
    assert_eq!(first, None, "first difference");
}

#[test]
fn damaged_streams_never_panic() {
    let mut inputs = Vec::new();
    for (i, cfg) in CASES.iter().enumerate() {
        let mut rng = Rng(77 + i as u64);
        let fs = cfg.frame_samples();
        let frames: Vec<Vec<u8>> = (0..3).map(|k| random_frame(cfg, k, fs, k == 2, 3 * fs as u64, &mut rng)).collect();
        inputs.push(file(cfg, 3 * fs as u64, &frames));
    }
    let mut x = Rng(0x1234_5678_9abc_def0);
    for round in 0..2400 {
        let mut d = inputs[round % inputs.len()].clone();
        match round % 3 {
            0 => d.truncate(x.below(d.len() as u64) as usize),
            1 => {
                for _ in 0..1 + x.below(8) {
                    let p = x.below(d.len() as u64) as usize;
                    d[p] ^= (x.next() >> 32) as u8 | 1;
                }
            }
            _ => {
                let p = 5 + x.below(d.len() as u64 - 5) as usize;
                let b = x.next() as u8;
                d[p..].iter_mut().for_each(|v| *v = b);
            }
        }
        let _ = decode_file(&d);
        let mut dec = Decoder::default();
        let _ = dec.decode(&d[d.len().min(30)..]);
    }
    for d in [&b""[..], b"tBaK", b"tBaK\x01\x00\x00", &[0xff; 64]] {
        assert!(decode_file(d).is_err());
    }
}
