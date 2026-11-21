//! DV encoder: 25 Mbit/s DV (IEC 61834-2) and DVCPRO 25 (SMPTE 314M) frames.
//!
//! The inverse of the decoder, built on the same tables: every DCT block is transformed (8-8,
//! or 2-4-8 when its two fields differ much more than its neighbouring lines), weighted and
//! given a class from its largest coefficient; each video segment (five macroblocks, 5 × 76
//! bytes of coded data) then takes the finest quantisation number (QNO) whose variable-length
//! codes fit the segment, refined per macroblock. The codes are laid out the way the decoder's
//! three passes read them back: each block in its own area, what does not fit in the free space
//! of its macroblock, then in the free space of the segment.
//!
//! The frame carries a DIF header, subcode blocks (SMPTE time code pack `0x13` when a time code
//! is given), VAUX source and source control packs (system, signal type, 4:3 / 16:9) and audio
//! blocks with an AAUX source pack: 16-bit PCM at 48 kHz, two channels (silence when no sound is
//! given). Written from the decoder's structure and the published DIF layout; no encoder source
//! (libdv, FFmpeg) was read.

use crate::dif::{Aspect, BLOCK_SIZE, ChromaFormat, Format, SEQUENCE_SIZE, System, sct};
use crate::layout::{self, Shape};
use crate::subcode::{TIMECODE_PACK, Timecode};
use crate::{Error, Result, idct, quant, vlc};

/// What to encode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncoderConfig {
    pub system: System,
    /// [`Format::Dv25`] (4:1:1 at 525 lines, 4:2:0 at 625) or [`Format::Dvcpro25`] (4:1:1).
    pub format: Format,
    pub aspect: Aspect,
}

impl EncoderConfig {
    /// Chroma format of the pictures the encoder takes.
    pub fn chroma(&self) -> ChromaFormat {
        if self.system == System::Lines625 && self.format == Format::Dv25 { ChromaFormat::Yuv420 } else { ChromaFormat::Yuv411 }
    }

    /// Picture size (720 × 480 or 720 × 576).
    pub fn size(&self) -> (u32, u32) {
        (720, if self.system == System::Lines525 { 480 } else { 576 })
    }

    fn sequences(&self) -> usize {
        if self.system == System::Lines525 { 10 } else { 12 }
    }

    /// Bytes per encoded frame.
    pub fn frame_size(&self) -> usize {
        self.sequences() * SEQUENCE_SIZE
    }

    fn apt(&self) -> u8 {
        u8::from(self.format == Format::Dvcpro25)
    }

    /// Audio samples per channel the frame `index` (from 0) of a stream carries at 48 kHz: 1920 at
    /// 625/50, and the five-frame 1600, 1602, 1602, 1602, 1602 cycle at 525/60 (8008 samples per
    /// 5 frames of 30000/1001 frames/s).
    pub fn audio_samples(&self, index: u64) -> usize {
        match self.system {
            System::Lines625 => 1920,
            System::Lines525 if index.is_multiple_of(5) => 1600,
            System::Lines525 => 1602,
        }
    }
}

/// One picture in the encoder's chroma format ([`EncoderConfig::chroma`]): tight 8-bit planes,
/// luma `width × height`, chroma `width / 4 × height` (4:1:1) or `width / 2 × height / 2`
/// (4:2:0).
#[derive(Clone, Copy, Debug)]
pub struct EncoderInput<'a> {
    pub y: &'a [u8],
    pub cb: &'a [u8],
    pub cr: &'a [u8],
    /// Two channels of 16-bit PCM at 48 kHz, each [`EncoderConfig::audio_samples`] long (longer
    /// slices are cut, shorter ones padded with silence); `None` writes silence.
    pub audio: Option<[&'a [i16]; 2]>,
    /// Time code of the frame, written to the subcode.
    pub timecode: Option<Timecode>,
}

/// DV frame encoder.
#[derive(Debug)]
pub struct Encoder {
    cfg: EncoderConfig,
    threads: bool,
    frames: u64,
}

/// Bits of the AC data area of each block of a macroblock (Y0-Y3: 14 bytes, Cr, Cb: 10, less
/// the 12 bits of DC, DCT mode and class).
const AREA_BITS: [usize; 6] = [100, 100, 100, 100, 68, 68];
/// First byte of each block's area in a video DIF block.
const AREA_START: [usize; 6] = [4, 18, 32, 46, 60, 70];
const SEGMENT_BITS: usize = 5 * (4 * 100 + 2 * 68);

impl Encoder {
    pub fn new(cfg: EncoderConfig) -> Result<Self> {
        if !matches!(cfg.format, Format::Dv25 | Format::Dvcpro25) {
            return Err(Error::Unsupported(cfg.format));
        }
        Ok(Self { cfg, threads: cfg!(feature = "threads"), frames: 0 })
    }

    pub fn config(&self) -> &EncoderConfig {
        &self.cfg
    }

    /// Encode the video segments in parallel (feature `threads`; ignored without it).
    pub fn set_threads(&mut self, on: bool) {
        self.threads = on && cfg!(feature = "threads");
    }

    /// Encode one frame; the audio sample count follows the encoder's frame counter.
    pub fn encode(&mut self, input: &EncoderInput<'_>) -> Result<Vec<u8>> {
        let cfg = self.cfg;
        let (w, h) = cfg.size();
        let (w, h) = (w as usize, h as usize);
        let (sx, sy) = cfg.chroma().shifts();
        let (cw, ch) = (w >> sx, h >> sy);
        if input.y.len() < w * h || input.cb.len() < cw * ch || input.cr.len() < cw * ch {
            return Err(Error::Input(format!("planes of {w}x{h} ({cw}x{ch} chroma) expected")));
        }
        let pic = Planes { y: input.y, cb: input.cb, cr: input.cr, w, cw, chroma: cfg.chroma() };
        let mut frame = vec![0xffu8; cfg.frame_size()];
        let seqs = cfg.sequences();
        let jobs: Vec<(usize, usize)> = (0..seqs).flat_map(|s| (0..27).map(move |k| (s, k))).collect();
        let run = |&(s, k): &(usize, usize)| encode_segment(&pic, seqs, s, k);
        #[cfg(feature = "threads")]
        let segments: Vec<[[u8; BLOCK_SIZE]; 5]> = if self.threads {
            use rayon::prelude::*;
            jobs.par_iter().map(run).collect()
        } else {
            jobs.iter().map(run).collect()
        };
        #[cfg(not(feature = "threads"))]
        let segments: Vec<[[u8; BLOCK_SIZE]; 5]> = jobs.iter().map(run).collect();
        for (&(s, k), blocks) in jobs.iter().zip(&segments) {
            for (m, b) in blocks.iter().enumerate() {
                let n = k * 5 + m;
                let at = s * SEQUENCE_SIZE + (6 + (k / 3) * 16 + 1 + (k % 3) * 5 + m) * BLOCK_SIZE;
                if let Some(dst) = frame.get_mut(at..at + BLOCK_SIZE) {
                    dst.copy_from_slice(b);
                    dst[0] = (sct::VIDEO << 5) | 0x1f;
                    dst[1] = ((s as u8) << 4) | 0x07;
                    dst[2] = n as u8;
                }
            }
        }
        let samples = cfg.audio_samples(self.frames);
        self.write_system_blocks(&mut frame, input, samples);
        self.frames += 1;
        Ok(frame)
    }

    /// Header, subcode, VAUX and audio blocks of every DIF sequence.
    fn write_system_blocks(&self, frame: &mut [u8], input: &EncoderInput<'_>, samples: usize) {
        let cfg = &self.cfg;
        let seqs = cfg.sequences();
        let half = seqs / 2;
        let fifty = cfg.system == System::Lines625;
        let apt = cfg.apt();
        let id = |frame: &mut [u8], at: usize, s: usize, kind: u8, n: u8| {
            if let Some(b) = frame.get_mut(at..at + 3) {
                b.copy_from_slice(&[(kind << 5) | 0x1f, ((s as u8) << 4) | 0x07, n]);
            }
        };
        let put = |frame: &mut [u8], at: usize, bytes: &[u8]| {
            if let Some(d) = frame.get_mut(at..at + bytes.len()) {
                d.copy_from_slice(bytes);
            }
        };
        let source = [0x60, 0xff, 0xff, 0xc0 | (u8::from(fifty) << 5), 0xff];
        let disp = if cfg.aspect == Aspect::Ratio16x9 { 2 } else { 0 };
        let control = [0x61, 0x3f, 0xc8 | disp, 0xfc, 0xff];
        let tc = input.timecode.map(timecode_pack);
        // AAUX source pack: samples above the minimum, two channels, 50/60, 48 kHz 16-bit, no
        // emphasis
        let min = if fifty { 1896 } else { 1580 };
        let af = (samples.saturating_sub(min)).min(63) as u8;
        let aaux_source = [0x50, 0xc0 | af, 0x00, 0xc0 | (u8::from(fifty) << 5), 0xc0];
        for s in 0..seqs {
            let base = s * SEQUENCE_SIZE;
            // header
            id(frame, base, s, sct::HEADER, 0);
            put(frame, base + 3, &[(u8::from(fifty) << 7) | 0x3f, 0xf8 | apt, 0x78 | apt, 0x78 | apt, 0x78 | apt]);
            // subcode: six sync blocks each, time code in sync blocks 0 and 3
            for b in 0..2 {
                let at = base + (1 + b) * BLOCK_SIZE;
                id(frame, at, s, sct::SUBCODE, b as u8);
                for k in 0..6 {
                    let syb = (b * 6 + k) as u8;
                    let fr = u8::from(s >= half);
                    put(frame, at + 3 + 8 * k, &[(fr << 7) | (apt << 4) | 0x0f, 0xf0 | syb, 0xff]);
                    if let Some(p) = tc.filter(|_| k % 3 == 0) {
                        put(frame, at + 6 + 8 * k, &p);
                    }
                }
            }
            // VAUX: source and source control packs in the first and third block
            for b in 0..3 {
                let at = base + (3 + b) * BLOCK_SIZE;
                id(frame, at, s, sct::VAUX, b as u8);
                if b == 0 {
                    put(frame, at + 3, &source);
                    put(frame, at + 8, &control);
                }
                if b == 2 {
                    put(frame, at + 3 + 5 * 9, &source);
                    put(frame, at + 3 + 5 * 10, &control);
                }
            }
            // audio
            let (chan, ds) = if s < half { (0, s) } else { (1, s - half) };
            let pcm = input.audio.and_then(|a| a.get(chan).copied()).unwrap_or(&[]);
            let first_pack = if s.is_multiple_of(2) { 3 } else { 0 };
            for b in 0..9 {
                let at = base + (6 + b * 16) * BLOCK_SIZE;
                id(frame, at, s, sct::AUDIO, b as u8);
                if b == first_pack {
                    put(frame, at + 3, &aaux_source);
                }
                let Some(base_pos) = crate::audio::unshuffle(fifty, ds, b) else { continue };
                let stride = if fifty { 54 } else { 45 };
                for k in 0..36 {
                    let n = base_pos + k * stride;
                    let v = if n < samples { pcm.get(n).copied().unwrap_or(0) } else { 0 };
                    put(frame, at + 8 + 2 * k, &v.to_be_bytes());
                }
            }
        }
    }
}

/// The SMPTE time code pack of a frame (BCD; flags clear).
fn timecode_pack(tc: Timecode) -> [u8; 5] {
    let bcd = |v: u8| ((v / 10) << 4) | (v % 10);
    [TIMECODE_PACK, bcd(tc.frames.min(39)) | (u8::from(tc.drop_frame) << 6), bcd(tc.seconds.min(59)), bcd(tc.minutes.min(59)), bcd(tc.hours.min(23))]
}

struct Planes<'a> {
    y: &'a [u8],
    cb: &'a [u8],
    cr: &'a [u8],
    w: usize,
    cw: usize,
    chroma: ChromaFormat,
}

/// `w × h` pixels at (`x`, `y`) of `plane` into `dst` at column `dst_x`.
fn get(plane: &[u8], stride: usize, x: usize, y: usize, w: usize, h: usize, dst: &mut [u8; 64], dst_x: usize) {
    for r in 0..h {
        if let (Some(src), Some(d)) = (plane.get((y + r) * stride + x..(y + r) * stride + x + w), dst.get_mut(r * 8 + dst_x..r * 8 + dst_x + w)) {
            d.copy_from_slice(src);
        }
    }
}

/// The six 8x8 blocks (Y0-Y3, Cr, Cb) of a macroblock, as the decoder places them.
fn fetch(p: &Planes<'_>, x: usize, y: usize, shape: Shape) -> [[u8; 64]; 6] {
    let mut b = [[128u8; 64]; 6];
    match shape {
        Shape::Wide411 => {
            for (i, blk) in b.iter_mut().take(4).enumerate() {
                get(p.y, p.w, x + 8 * i, y, 8, 8, blk, 0);
            }
            get(p.cr, p.cw, x / 4, y, 8, 8, &mut b[4], 0);
            get(p.cb, p.cw, x / 4, y, 8, 8, &mut b[5], 0);
        }
        Shape::Right411 | Shape::Square420 => {
            get(p.y, p.w, x, y, 8, 8, &mut b[0], 0);
            get(p.y, p.w, x + 8, y, 8, 8, &mut b[1], 0);
            get(p.y, p.w, x, y + 8, 8, 8, &mut b[2], 0);
            get(p.y, p.w, x + 8, y + 8, 8, 8, &mut b[3], 0);
            if shape == Shape::Right411 {
                for (i, plane) in [p.cr, p.cb].into_iter().enumerate() {
                    get(plane, p.cw, x / 4, y, 4, 8, &mut b[4 + i], 0);
                    get(plane, p.cw, x / 4, y + 8, 4, 8, &mut b[4 + i], 4);
                }
            } else {
                get(p.cr, p.cw, x / 2, y / 2, 8, 8, &mut b[4], 0);
                get(p.cb, p.cw, x / 2, y / 2, 8, 8, &mut b[5], 0);
            }
        }
    }
    b
}

/// 2-4-8 when the lines of one field differ much less from each other than from the lines of
/// the other field (motion between the fields).
fn interlaced(p: &[u8; 64]) -> bool {
    let d = |a: usize, b: usize| -> u32 { (0..8).map(|x| u32::from(p[a * 8 + x].abs_diff(p[b * 8 + x]))).sum() };
    let adjacent: u32 = (0..7).map(|r| d(r, r + 1)).sum();
    let same_field: u32 = (0..6).map(|r| d(r, r + 2)).sum();
    adjacent > 256 && adjacent * 3 > same_field * 7
}

/// One transformed block, ready to quantise.
struct Coded {
    dc: i32,
    mode248: bool,
    class: u8,
    /// Coefficients in zigzag order (index 0 unused).
    zz: [f32; 64],
}

fn transform(p: &[u8; 64]) -> Coded {
    let mode248 = interlaced(p);
    let c = if mode248 { idct::fdct_248(p) } else { idct::fdct_88(p) };
    let scan = if mode248 { &quant::SCAN_248 } else { &quant::SCAN_88 };
    let mut zz = [0f32; 64];
    for (i, z) in zz.iter_mut().enumerate().skip(1) {
        *z = c[usize::from(scan[i] & 63)];
    }
    // the largest amplitude at the finest step of class 2 picks the class: flat blocks get finer
    // steps, very large coefficients need class 3's doubled step to stay within 255
    let (f, _) = quant::factors(mode248, 2, 15);
    let max = (1..64).map(|i| (zz[i] / f[usize::from(scan[i] & 63)]).abs()).fold(0f32, f32::max);
    let class = match max {
        m if m > 255.0 => 3,
        m if m > 48.0 => 2,
        m if m > 16.0 => 1,
        _ => 0,
    };
    let dc = ((c[0] / 4.0).round() as i32).clamp(-256, 255);
    Coded { dc, mode248, class, zz }
}

/// Quantised amplitudes (zigzag order) of a block at `qno`.
fn quantise(b: &Coded, qno: u8) -> [i16; 64] {
    let (f, extra) = quant::factors(b.mode248, b.class, qno);
    let scan = if b.mode248 { &quant::SCAN_248 } else { &quant::SCAN_88 };
    let mut a = [0i16; 64];
    for i in 1..64 {
        let v = b.zz[i] / (f[usize::from(scan[i] & 63)] * extra);
        // rounding with a small dead zone
        let m = (v.abs() + 0.375).floor().min(255.0) as i16;
        a[i] = if v < 0.0 { -m } else { m };
    }
    a
}

/// Encoding of `(run, amp)` codes: the table entries (without sign) and the escapes.
struct Vlc {
    /// `direct[run][amp]` = (bits, length) without the sign bit.
    direct: Vec<[Option<(u32, u8)>; 256]>,
    eob: (u32, u8),
}

fn vlc_table() -> &'static Vlc {
    static T: std::sync::OnceLock<Vlc> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut direct = vec![[None; 256]; 64];
        for (bits, len, run, amp) in vlc::codes() {
            if let Some(slot) = direct.get_mut(usize::from(run)).and_then(|r| r.get_mut(usize::from(amp))) {
                *slot = Some((bits, len));
            }
        }
        // run escape: 1111110 + 6-bit run, amplitude 0 (runs 6..=61)
        for run in 6..=61usize {
            if let Some(slot) = direct.get_mut(run).and_then(|r| r.get_mut(0)) {
                *slot = Some(((0b111_1110 << 6) | run as u32, 13));
            }
        }
        // amplitude escape: 1111111 + 8-bit amplitude, run 0 (amplitudes 23..=255)
        if let Some(r) = direct.get_mut(0) {
            for (amp, slot) in r.iter_mut().enumerate().skip(23) {
                *slot = Some(((0b111_1111 << 8) | amp as u32, 15));
            }
        }
        Vlc { direct, eob: vlc::eob() }
    })
}

impl Vlc {
    fn code(&self, run: usize, amp: usize) -> Option<(u32, u8)> {
        self.direct.get(run).and_then(|r| r.get(amp)).copied().flatten()
    }

    /// The codes for `run` zeros followed by `amp` (> 0, without its sign): either one table
    /// code, or the zeros as a zero-amplitude code (`n` zeros = run `n - 1`, amplitude 0) and
    /// then a shorter-run code, whichever is shortest.
    fn pair(&self, run: usize, amp: usize) -> [(u32, u8); 2] {
        let mut best: Option<([(u32, u8); 2], u32)> = self.code(run, amp).map(|c| ([c, (0, 0)], u32::from(c.1)));
        for tail in 0..run {
            let (Some(z), Some(c)) = (self.code(run - tail - 1, 0), self.code(tail, amp)) else { continue };
            let n = u32::from(z.1) + u32::from(c.1);
            if best.is_none_or(|(_, b)| n < b) {
                best = Some(([z, c], n));
            }
        }
        best.map_or([(0, 0); 2], |(c, _)| c)
    }

    /// The bits of a block's AC data: codes with sign bits and the end of block.
    fn block(&self, amps: &[i16; 64], out: &mut Bits) {
        let mut run = 0usize;
        for &a in amps.iter().skip(1) {
            if a == 0 {
                run += 1;
                continue;
            }
            let [first, second] = self.pair(run, usize::from(a.unsigned_abs()));
            out.push(first.0, first.1);
            out.push(second.0, second.1);
            out.push(u32::from(a < 0), 1);
            run = 0;
        }
        out.push(self.eob.0, self.eob.1);
    }
}

/// A bit string, one byte per bit.
#[derive(Default, Clone)]
struct Bits(Vec<u8>);

impl Bits {
    fn push(&mut self, v: u32, n: u8) {
        for i in (0..u32::from(n)).rev() {
            self.0.push(((v >> i) & 1) as u8);
        }
    }
}

/// The coded bits of a macroblock's six blocks at `qno`.
fn code_mb(blocks: &[Coded; 6], qno: u8) -> [Bits; 6] {
    let t = vlc_table();
    std::array::from_fn(|i| {
        let mut b = Bits::default();
        t.block(&quantise(&blocks[i], qno), &mut b);
        b
    })
}

fn total(mbs: &[[Bits; 6]]) -> usize {
    mbs.iter().flat_map(|m| m.iter()).map(|b| b.0.len()).sum()
}

fn encode_segment(pic: &Planes<'_>, seqs: usize, seq: usize, k: usize) -> [[u8; BLOCK_SIZE]; 5] {
    let mut mbs: Vec<[Coded; 6]> = (0..5)
        .map(|m| {
            let (x, y, shape) = layout::place(pic.chroma, seqs, seq, k, m);
            let px = fetch(pic, x, y, shape);
            std::array::from_fn(|i| transform(&px[i]))
        })
        .collect();
    // the finest common QNO that fits, then finer QNOs for single macroblocks while it still fits
    let mut qno = [0u8; 5];
    let mut coded: Vec<[Bits; 6]> = Vec::new();
    for q in (0..=15u8).rev() {
        let c: Vec<[Bits; 6]> = mbs.iter().map(|b| code_mb(b, q)).collect();
        if total(&c) <= SEGMENT_BITS || q == 0 {
            qno = [q; 5];
            coded = c;
            break;
        }
    }
    for m in 0..5 {
        for q in (qno[m] + 1..=15).rev() {
            let Some(mb) = mbs.get(m) else { break };
            let c = code_mb(mb, q);
            let before: usize = coded.get(m).map_or(0, |b| b.iter().map(|x| x.0.len()).sum());
            let after: usize = c.iter().map(|x| x.0.len()).sum();
            if total(&coded) - before + after <= SEGMENT_BITS {
                if let Some(slot) = coded.get_mut(m) {
                    *slot = c;
                }
                qno[m] = q;
                break;
            }
        }
    }
    // still too long at QNO 0: drop the last codes of the longest blocks
    while total(&coded) > SEGMENT_BITS {
        if !drop_last_coefficient(&mut mbs, &qno, &mut coded) {
            break;
        }
    }
    layout_segment(&mbs, &qno, &coded)
}

/// Zero the last non-zero coefficient of the longest block and re-code it; false when nothing is
/// left to drop.
fn drop_last_coefficient(mbs: &mut [[Coded; 6]], qno: &[u8; 5], coded: &mut [[Bits; 6]]) -> bool {
    let mut longest: Option<(usize, usize, usize)> = None;
    for (m, c) in coded.iter().enumerate() {
        for (b, bits) in c.iter().enumerate() {
            if longest.is_none_or(|(_, _, n)| bits.0.len() > n) {
                longest = Some((m, b, bits.0.len()));
            }
        }
    }
    let Some((m, b, _)) = longest else { return false };
    let q = qno.get(m).copied().unwrap_or(0);
    let Some(blk) = mbs.get_mut(m).and_then(|mb| mb.get_mut(b)) else { return false };
    let amps = quantise(blk, q);
    let Some(last) = (1..64).rev().find(|&i| amps[i] != 0) else { return false };
    blk.zz[last] = 0.0;
    let mut bits = Bits::default();
    vlc_table().block(&quantise(blk, q), &mut bits);
    if let Some(slot) = coded.get_mut(m).and_then(|c| c.get_mut(b)) {
        *slot = bits;
    }
    true
}

/// Lay the coded bits out in the five DIF video blocks (IDs filled in by the caller): each block
/// in its own area, overflow in the free space of its macroblock, then of the segment.
fn layout_segment(mbs: &[[Coded; 6]], qno: &[u8; 5], coded: &[[Bits; 6]]) -> [[u8; BLOCK_SIZE]; 5] {
    // bit buffers of the 30 areas, all ones (unused space)
    let mut areas: Vec<Vec<u8>> = (0..5).flat_map(|_| AREA_BITS.iter().map(|&n| vec![1u8; n])).collect();
    // pass 1
    let mut left: Vec<&[u8]> = Vec::with_capacity(30);
    let mut used = vec![0usize; 30];
    for m in 0..5 {
        for b in 0..6 {
            let i = m * 6 + b;
            let bits: &[u8] = coded.get(m).and_then(|c| c.get(b)).map_or(&[], |x| &x.0);
            let n = bits.len().min(AREA_BITS[b]);
            if let Some(a) = areas.get_mut(i) {
                a[..n].copy_from_slice(&bits[..n]);
            }
            used[i] = n;
            left.push(&bits[n..]);
        }
    }
    // free space of the blocks that ended, as (area, from, to)
    let free = |used: &[usize], left: &[&[u8]], i: usize| -> Option<(usize, usize, usize)> {
        let to = AREA_BITS[i % 6];
        (left[i].is_empty() && used[i] < to).then_some((i, used[i], to))
    };
    // pass 2: within each macroblock
    let mut seg_free: Vec<(usize, usize, usize)> = Vec::new();
    let mut pending: Vec<usize> = Vec::new();
    for m in 0..5 {
        let mut pool: Vec<(usize, usize, usize)> = (m * 6..m * 6 + 6).filter_map(|i| free(&used, &left, i)).collect();
        for i in m * 6..m * 6 + 6 {
            if left[i].is_empty() {
                continue;
            }
            let rest = fill(&mut areas, &mut pool, left[i]);
            left[i] = rest;
            if !left[i].is_empty() {
                pending.push(i);
            }
        }
        seg_free.extend(pool);
    }
    // pass 3: across the segment
    for i in pending {
        let rest = fill(&mut areas, &mut seg_free, left[i]);
        left[i] = rest;
    }
    let mut out = [[0xffu8; BLOCK_SIZE]; 5];
    for (m, blk) in out.iter_mut().enumerate() {
        blk[3] = qno.get(m).copied().unwrap_or(0) & 0x0f;
        for b in 0..6 {
            let Some(c) = mbs.get(m).and_then(|mb| mb.get(b)) else { continue };
            let start = AREA_START[b] * 8;
            let mut bits = Bits::default();
            bits.push((c.dc & 0x1ff) as u32, 9);
            bits.push(u32::from(c.mode248), 1);
            bits.push(u32::from(c.class), 2);
            bits.0.extend_from_slice(areas.get(m * 6 + b).map_or(&[][..], |a| a.as_slice()));
            for (j, &bit) in bits.0.iter().enumerate() {
                let (byte, shift) = ((start + j) / 8, 7 - (start + j) % 8);
                if let Some(v) = blk.get_mut(byte) {
                    *v = (*v & !(1 << shift)) | (bit << shift);
                }
            }
        }
    }
    out
}

/// Write `bits` into the free ranges of `pool` from the front, consuming them; returns what did
/// not fit.
fn fill<'b>(areas: &mut [Vec<u8>], pool: &mut Vec<(usize, usize, usize)>, mut bits: &'b [u8]) -> &'b [u8] {
    while !bits.is_empty() {
        let Some(&(i, from, to)) = pool.first() else { break };
        let n = bits.len().min(to - from);
        if let Some(dst) = areas.get_mut(i).and_then(|a| a.get_mut(from..from + n)) {
            dst.copy_from_slice(&bits[..n]);
        }
        bits = &bits[n..];
        if from + n == to {
            pool.remove(0);
        } else if let Some(first) = pool.first_mut() {
            first.1 = from + n;
        }
    }
    bits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Decoder, decode_audio, timecode};

    /// A picture with gradients, edges and fine texture.
    fn picture(cfg: &EncoderConfig, seed: u32) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let (w, h) = cfg.size();
        let (w, h) = (w as usize, h as usize);
        let (sx, sy) = cfg.chroma().shifts();
        let (cw, ch) = (w >> sx, h >> sy);
        let mut r = seed;
        let mut noise = move || {
            r = r.wrapping_mul(1_103_515_245).wrapping_add(12345);
            ((r >> 16) % 9) as i32 - 4
        };
        let y = (0..w * h)
            .map(|i| {
                let (x, yy) = ((i % w) as i32, (i / w) as i32);
                let base = 40 + (x * 160 / w as i32) + if (x / 90 + yy / 60) % 2 == 0 { 20 } else { 0 };
                (base + noise()).clamp(16, 235) as u8
            })
            .collect();
        let cb = (0..cw * ch).map(|i| (128 + ((i % cw) as i32 * 60 / cw as i32) - 30) as u8).collect();
        let cr = (0..cw * ch).map(|i| (100 + ((i / cw) as i32 * 50 / ch as i32)) as u8).collect();
        (y, cb, cr)
    }

    fn psnr(a: &[u8], b: &[u8]) -> f64 {
        let mse = a.iter().zip(b).map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2)).sum::<f64>() / a.len() as f64;
        10.0 * (255.0f64 * 255.0 / mse.max(1e-9)).log10()
    }

    #[test]
    fn every_code_decodes_back() {
        let t = vlc_table();
        let dec = vlc::table();
        for run in 0..63usize {
            for amp in 1..=255usize {
                let mut amps = [0i16; 64];
                if run + 1 > 63 {
                    continue;
                }
                amps[run + 1] = -(amp as i16);
                let mut bits = Bits::default();
                t.block(&amps, &mut bits);
                let mut blk = crate::segment::test_block();
                assert!(crate::segment::test_run(&mut blk, dec, &bits.0), "run {run} amp {amp}");
                assert_eq!(blk.amps, amps, "run {run} amp {amp}");
            }
        }
    }

    #[test]
    fn round_trip_through_the_decoder() {
        for (system, format) in [(System::Lines525, Format::Dv25), (System::Lines625, Format::Dv25), (System::Lines625, Format::Dvcpro25)] {
            let cfg = EncoderConfig { system, format, aspect: Aspect::Ratio16x9 };
            let (y, cb, cr) = picture(&cfg, 7);
            let tc = Timecode { hours: 1, minutes: 2, seconds: 3, frames: 4, drop_frame: system == System::Lines525 };
            let left: Vec<i16> = (0..1920).map(|i| ((i as f32 * 0.05).sin() * 12000.0) as i16).collect();
            let right: Vec<i16> = left.iter().map(|v| -v / 2).collect();
            let mut enc = Encoder::new(cfg).unwrap();
            let frame = enc.encode(&EncoderInput { y: &y, cb: &cb, cr: &cr, audio: Some([&left, &right]), timecode: Some(tc) }).unwrap();
            assert_eq!(frame.len(), cfg.frame_size());
            let pic = Decoder::new().decode(&frame).unwrap();
            assert_eq!((pic.profile.format, pic.profile.chroma, pic.profile.aspect), (format, cfg.chroma(), Aspect::Ratio16x9), "{cfg:?}");
            assert_eq!((pic.errors, pic.concealed), (0, 0), "{cfg:?}");
            let (py, pcb, pcr) = (psnr(&y, &pic.y), psnr(&cb, &pic.cb), psnr(&cr, &pic.cr));
            assert!(py > 38.0 && pcb > 40.0 && pcr > 40.0, "{cfg:?}: PSNR {py:.1} / {pcb:.1} / {pcr:.1}");
            assert_eq!(timecode(&frame), Some(tc));
            let a = decode_audio(&frame).unwrap();
            let n = cfg.audio_samples(0);
            assert_eq!((a.format.sample_rate, a.format.bits, a.format.samples, a.errors), (48_000, 16, n, 0));
            assert_eq!(a.channels[0], left[..n]);
            assert_eq!(a.channels[1], right[..n]);
        }
    }

    #[test]
    fn ntsc_audio_follows_the_five_frame_cycle() {
        let cfg = EncoderConfig { system: System::Lines525, format: Format::Dv25, aspect: Aspect::Ratio4x3 };
        let (y, cb, cr) = picture(&cfg, 1);
        let mut enc = Encoder::new(cfg).unwrap();
        let counts: Vec<usize> = (0..6)
            .map(|_| {
                let f = enc.encode(&EncoderInput { y: &y, cb: &cb, cr: &cr, audio: None, timecode: None }).unwrap();
                assert_eq!(timecode(&f), None);
                decode_audio(&f).unwrap().format.samples
            })
            .collect();
        assert_eq!(counts, [1600, 1602, 1602, 1602, 1602, 1600]);
    }

    #[test]
    fn noise_still_fits_and_decodes_cleanly() {
        let cfg = EncoderConfig { system: System::Lines625, format: Format::Dv25, aspect: Aspect::Ratio4x3 };
        let mut r = 99u32;
        let mut next = move || {
            r = r.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (r >> 24) as u8
        };
        let y: Vec<u8> = (0..720 * 576).map(|_| next()).collect();
        let c: Vec<u8> = (0..360 * 288).map(|_| next()).collect();
        let f = Encoder::new(cfg).unwrap().encode(&EncoderInput { y: &y, cb: &c, cr: &c, audio: None, timecode: None }).unwrap();
        let pic = Decoder::new().decode(&f).unwrap();
        assert_eq!((pic.errors, pic.concealed), (0, 0));
    }

    #[test]
    fn short_planes_are_refused() {
        let cfg = EncoderConfig { system: System::Lines525, format: Format::Dvcpro25, aspect: Aspect::Ratio4x3 };
        let mut enc = Encoder::new(cfg).unwrap();
        assert!(enc.encode(&EncoderInput { y: &[], cb: &[], cr: &[], audio: None, timecode: None }).is_err());
        assert!(Encoder::new(EncoderConfig { format: Format::Dvcpro50, ..cfg }).is_err());
    }
}
