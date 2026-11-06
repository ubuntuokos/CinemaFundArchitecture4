// Copyright (c) 2019, Aous Naman
// Copyright (c) 2019, Kakadu Software Pty Ltd, Australia
// Copyright (c) 2019, The University of New South Wales, Australia
// SPDX-License-Identifier: BSD-2-Clause
//
// Ported to safe Rust for CFA4 from OpenJPH's `ojph_block_decoder32.cpp` and
// `ojph_block_common.cpp` (BSD-2-Clause, see `LICENSE-OpenJPH` next to this file). The
// word-at-a-time, alignment-driven readers became byte-at-a-time readers with the same bit
// semantics; every buffer access is bounds-checked.

//! HTJ2K (ITU-T T.814 | ISO/IEC 15444-15) block decoder: the high-throughput cleanup pass (MEL,
//! VLC, UVLC and MagSgn segments) and the optional significance propagation and magnitude
//! refinement passes of one code-block.
//!
//! Samples are decoded as OpenJPH does: sign in bit 31, the magnitude MSB-aligned at bit 30 with
//! the reconstruction midpoint set below the last decoded bit-plane, then brought to the
//! one-fractional-bit magnitudes of [`crate::t1`].

use crate::ht_tables::{VLC0, VLC1};
use crate::t1::{Block, MAX_PLANES, STYLE_VSC};
use crate::{Error, Result};

/// Code-block style bit of HT code-blocks (T.814, Table A.4).
pub(crate) const STYLE_HT: u8 = 0x40;
/// Mixed HT / non-HT code-blocks: not supported.
pub(crate) const STYLE_HT_MIXED: u8 = 0x80;

/// A VLC table entry: bits 0-2 codeword length, 3 u_off, 4-7 rho, 8-11 e_1, 12-15 e_k.
const fn vlc_table(src: &[[u8; 7]]) -> [u16; 1024] {
    let mut t = [0u16; 1024];
    let mut j = 0;
    while j < src.len() {
        let [c_q, rho, u_off, e_k, e_1, cwd, len] = src[j];
        let entry = ((rho as u16) << 4) | ((u_off as u16) << 3) | ((e_k as u16) << 12) | ((e_1 as u16) << 8) | len as u16;
        // every 7-bit index whose low `len` bits are the codeword
        let mut hi = 0usize;
        while hi < (1usize << (7 - len)) {
            t[((c_q as usize) << 7) | (hi << len) | cwd as usize] = entry;
            hi += 1;
        }
        j += 1;
    }
    t
}

static VLC_TBL0: [u16; 1024] = vlc_table(&VLC0);
static VLC_TBL1: [u16; 1024] = vlc_table(&VLC1);

/// UVLC prefix decoding by the 3 low bits: prefix length (2 bits), suffix length (3), u prefix.
const UVLC_DEC: [u8; 8] =
    [3 | (5 << 2) | (5 << 5), 1 | (1 << 5), 2 | (2 << 5), 1 | (1 << 5), 3 | (1 << 2) | (3 << 5), 1 | (1 << 5), 2 | (2 << 5), 1 | (1 << 5)];

const fn pack_uvlc(total_prefix: u32, total_suffix: u32, u0_suffix_len: u32, u0: u32, u1: u32) -> u16 {
    (total_prefix | (total_suffix << 3) | (u0_suffix_len << 7) | (u0 << 10) | (u1 << 13)) as u16
}

/// UVLC tables, indexed by mode (u_off of the two quads, and for the first row the MEL event when
/// both are set) times 64 plus the next 6 bits: total prefix length (3 bits), total suffix length
/// (4), quad 0's suffix length (3), quad 0's u prefix (3), quad 1's (3).
const fn uvlc_table(initial: bool) -> [u16; 320] {
    let mut t = [0u16; 320];
    let mut i = 0usize;
    let modes = if initial { 5 } else { 4 };
    while i < modes * 64 {
        let mode = i >> 6;
        let vlc = i & 0x3F;
        if mode == 1 || mode == 2 {
            let d = UVLC_DEC[vlc & 7] as u32;
            let total_prefix = d & 3;
            let total_suffix = (d >> 2) & 7;
            let (u0_suffix_len, u0, u1) = if mode == 1 { (total_suffix, d >> 5, 0) } else { (0, 0, d >> 5) };
            t[i] = pack_uvlc(total_prefix, total_suffix, u0_suffix_len, u0, u1);
        } else if mode >= 3 {
            let d0 = UVLC_DEC[vlc & 7] as u32;
            let rest = vlc >> (d0 & 3);
            let d1 = UVLC_DEC[rest & 7] as u32;
            if initial && mode == 3 && d0 & 3 == 3 {
                // both u_off set, MEL event 0, quad 0 prefix "001" or longer: quad 1 has u 1 or 2
                let u0_suffix_len = (d0 >> 2) & 7;
                t[i] = pack_uvlc((d0 & 3) + 1, u0_suffix_len, u0_suffix_len, d0 >> 5, (rest as u32 & 1) + 1);
            } else {
                let add = if initial && mode == 4 { 2 } else { 0 };
                let u0_suffix_len = (d0 >> 2) & 7;
                let total_suffix = u0_suffix_len + ((d1 >> 2) & 7);
                t[i] = pack_uvlc((d0 & 3) + (d1 & 3), total_suffix, u0_suffix_len, (d0 >> 5) + add, (d1 >> 5) + add);
            }
        }
        i += 1;
    }
    t
}

static UVLC_TBL0: [u16; 320] = uvlc_table(true);
static UVLC_TBL1: [u16; 320] = uvlc_table(false);

/// MEL decoder (T.814 §7.3.3): an adaptive run-length code read forward from the start of the
/// MEL segment, MSB first, unstuffing after 0xFF; 0xFF is fed once the segment is consumed.
struct Mel<'a> {
    d: &'a [u8],
    pos: usize,
    /// Bytes of MEL + VLC left (the last one shares its low nibble with the VLC segment).
    size: usize,
    unstuff: bool,
    cur: u32,
    left: u32,
    k: usize,
}

const MEL_EXP: [u32; 13] = [0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 4, 5];

impl Mel<'_> {
    fn bit(&mut self) -> u32 {
        if self.left == 0 {
            let mut b = 0xFFu32;
            if self.size > 0 {
                b = u32::from(self.d.get(self.pos).copied().unwrap_or(0xFF));
                if self.size == 1 {
                    b |= 0xF;
                }
                self.pos += 1;
                self.size -= 1;
            }
            self.left = if self.unstuff { 7 } else { 8 };
            self.unstuff = b == 0xFF;
            self.cur = b;
        }
        self.left -= 1;
        (self.cur >> self.left) & 1
    }

    /// The next run: twice the zero events, plus one when it ends with a one event.
    fn run(&mut self) -> i32 {
        let eval = MEL_EXP.get(self.k).copied().unwrap_or(5);
        if self.bit() == 1 {
            self.k = (self.k + 1).min(12);
            ((1i32 << eval) - 1) << 1
        } else {
            let mut run = 0i32;
            for _ in 0..eval {
                run = (run << 1) | self.bit() as i32;
            }
            self.k = self.k.saturating_sub(1);
            (run << 1) + 1
        }
    }
}

/// A backward-growing segment (VLC, MagRef) read LSB first from its end, unstuffing a 0x7F after
/// a byte above 0x8F; zeros are fed once it is consumed.
struct Rev<'a> {
    d: &'a [u8],
    /// Next byte to read (counting down); `None` past the start.
    pos: Option<usize>,
    size: usize,
    tmp: u64,
    bits: u32,
    unstuff: bool,
}

impl<'a> Rev<'a> {
    /// The VLC segment: it ends at the cleanup pass's last byte but one, whose high nibble is its
    /// first data (the 12 bits there hold the length of MEL + VLC).
    fn vlc(d: &'a [u8], lcup: usize, scup: usize) -> Self {
        let first = u32::from(d.get(lcup - 2).copied().unwrap_or(0));
        let tmp = first >> 4;
        let bits = 4 - u32::from(tmp & 7 == 7);
        let unstuff = (first | 0xF) > 0x8F;
        let mut r = Rev { d, pos: (lcup - 2).checked_sub(1), size: scup - 2, tmp: u64::from(tmp), bits, unstuff };
        r.fill();
        r
    }

    /// The MagRef segment: from the end of the refinement segment backwards.
    fn mrp(d: &'a [u8], lcup: usize, len2: usize) -> Self {
        let mut r = Rev { d, pos: (lcup + len2).checked_sub(1), size: len2, tmp: 0, bits: 0, unstuff: true };
        r.fill();
        r
    }

    fn fill(&mut self) {
        while self.bits <= 56 {
            let mut b = 0u64;
            if self.size > 0 {
                b = self.pos.and_then(|p| self.d.get(p)).copied().map_or(0, u64::from);
                self.pos = self.pos.and_then(|p| p.checked_sub(1));
                self.size -= 1;
            }
            let n = if self.unstuff && b & 0x7F == 0x7F { 7 } else { 8 };
            self.tmp |= b << self.bits;
            self.bits += n;
            self.unstuff = b > 0x8F;
        }
    }

    fn fetch(&mut self) -> u32 {
        if self.bits < 32 {
            self.fill();
        }
        self.tmp as u32
    }

    fn advance(&mut self, n: u32) -> u32 {
        let n = n.min(self.bits);
        self.tmp = self.tmp.checked_shr(n).unwrap_or(0);
        self.bits -= n;
        self.tmp as u32
    }
}

/// A forward-growing segment (MagSgn, SigProp) read LSB first, unstuffing after 0xFF; `fill` is
/// fed once it is consumed (0xFF for MagSgn, 0 for SigProp).
struct Fwd<'a> {
    d: &'a [u8],
    pos: usize,
    end: usize,
    tmp: u64,
    bits: u32,
    unstuff: bool,
    feed: u8,
}

impl<'a> Fwd<'a> {
    fn new(d: &'a [u8], start: usize, len: usize, feed: u8) -> Self {
        let mut f = Fwd { d, pos: start, end: start.saturating_add(len), tmp: 0, bits: 0, unstuff: false, feed };
        f.fill();
        f
    }

    fn fill(&mut self) {
        while self.bits <= 56 {
            let b = if self.pos < self.end { self.d.get(self.pos).copied().unwrap_or(self.feed) } else { self.feed };
            self.pos = self.pos.saturating_add(1);
            self.tmp |= u64::from(b) << self.bits;
            self.bits += if self.unstuff { 7 } else { 8 };
            self.unstuff = b == 0xFF;
        }
    }

    fn fetch(&mut self) -> u32 {
        if self.bits < 32 {
            self.fill();
        }
        self.tmp as u32
    }

    fn advance(&mut self, n: u32) {
        let n = n.min(self.bits);
        self.tmp = self.tmp.checked_shr(n).unwrap_or(0);
        self.bits -= n;
    }
}

/// One step of the MEL event bookkeeping: the run counter counts events twice, its low bit
/// telling whether the run ends in a one event. Returns the event.
fn mel_event(run: &mut i32, mel: &mut Mel) -> bool {
    *run -= 2;
    let one = *run == -1;
    if *run < 0 {
        *run = mel.run();
    }
    one
}

/// Decodes the significant sample `bit` (0-3) of a quad from MagSgn: sign in bit 31, magnitude
/// with the bin centre, shifted up to plane `p`. Also returns v_n (for the exponent of the next
/// row's quads).
fn mag_sgn(ms: &mut Fwd, inf: u32, bit: u32, u_q: u32, p: u32) -> (u32, u32) {
    if inf & (1 << (4 + bit)) == 0 {
        return (0, 0);
    }
    let ms_val = ms.fetch();
    let m_n = u_q.saturating_sub((inf >> (12 + bit)) & 1);
    ms.advance(m_n);
    let sign = ms_val << 31;
    let mut v_n = ms_val & 1u32.checked_shl(m_n).map_or(u32::MAX, |m| m - 1);
    v_n |= ((inf >> (8 + bit)) & 1).checked_shl(m_n).unwrap_or(0);
    v_n |= 1;
    // v_n is 2(mu - 1) + 1: add 2 for 2 mu + 1, then up to the plane
    let mag = (v_n.wrapping_add(2)).checked_shl(p - 1).unwrap_or(0) & 0x7FFF_FFFF;
    (sign | mag, v_n)
}

/// Little-endian pair of u16 entries at `i` and `i + 1` (zero outside).
fn pair(a: &[u16], i: usize) -> u32 {
    u32::from(a.get(i).copied().unwrap_or(0)) | (u32::from(a.get(i + 1).copied().unwrap_or(0)) << 16)
}

/// Decodes an HT code-block: its first segment is the HT cleanup segment (which may follow
/// placeholder passes), the optional second the refinement (SigProp, MagRef) segment.
pub(crate) fn decode(b: &Block) -> Result<Vec<i32>> {
    let n = b.w.checked_mul(b.h).ok_or(Error::TooLarge)?;
    let Some((cup, cup_passes)) = b.segments.first() else { return Ok(vec![0; n]) };
    if b.mb > MAX_PLANES {
        return Err(Error::Unsupported("code-block with more than 30 bit-planes"));
    }
    if b.w > 1024 || b.h > 1024 || n > 4096 {
        return Err(Error::Invalid("HT code-block size"));
    }
    // placeholder passes before the cleanup pass skip whole bit-planes
    let missing = b.zero + cup_passes.saturating_sub(1) / 3;
    if n == 0 || cup.len() < 2 || missing >= b.mb {
        return Ok(vec![0; n]);
    }
    let refine = b.segments.get(1);
    let mut passes = 1 + refine.map_or(0, |s| s.1.min(2));
    // the cleanup pass's plane, 30 being the MSB (missing < mb <= 30, so p >= 1)
    let p = 30 - missing;
    if p < 2 {
        passes = 1;
    }
    // cleanup and refinement bytes, one buffer: the MagRef reader runs back from its end
    let mut data = cup.clone();
    let lcup = data.len();
    let len2 = refine.map_or(0, |s| s.0.len());
    if let Some(s) = refine {
        data.extend_from_slice(&s.0);
    }
    if passes > 1 && len2 == 0 {
        passes = 1;
    }
    let mut out = vec![0u32; b.w * (b.h + 1)];
    cleanup(&data, lcup, b, p, &mut out)?;
    if passes > 1 {
        refinement(&data, lcup, len2, b, p, passes, &mut out);
    }
    let shift = 30 - b.mb;
    let mut res = Vec::with_capacity(n);
    for &v in out.iter().take(n) {
        let m = ((v & 0x7FFF_FFFF) >> shift) as i32;
        res.push(if v & 0x8000_0000 != 0 { -m } else { m });
    }
    Ok(res)
}

/// The cleanup pass: VLC and MEL into per-quad records, then MagSgn into `out` (stride `b.w`).
fn cleanup(d: &[u8], lcup: usize, b: &Block, p: u32, out: &mut [u32]) -> Result<()> {
    let (width, height, stride) = (b.w, b.h, b.w);
    let scup = (usize::from(d.get(lcup - 1).copied().unwrap_or(0)) << 4) + usize::from(d.get(lcup - 2).copied().unwrap_or(0) & 0xF);
    if scup < 2 || scup > lcup || scup > 4079 {
        return Err(Error::Invalid("HT cleanup segment lengths"));
    }
    let mmsbp2 = 30 - p + 2;
    // per quad: (inf, u_q) with inf = e_k << 12 | e_1 << 8 | rho << 4 | u_off << 3 | length
    let sstr = ((width + 2) + 7) & !7;
    let mut scratch = vec![0u16; sstr * (height.div_ceil(2) + 2)];
    {
        let mut mel = Mel { d, pos: lcup - scup, size: scup - 1, unstuff: false, cur: 0, left: 0, k: 0 };
        let mut vlc = Rev::vlc(d, lcup, scup);
        let mut run = mel.run();
        for y in (0..height).step_by(2) {
            let initial = y == 0;
            let row = (y >> 1) * sstr;
            let above = row.wrapping_sub(sstr);
            let at = |s: &[u16], i: usize| u32::from(s.get(i).copied().unwrap_or(0));
            let mut c_q = 0u32;
            let mut sp = row;
            let mut x = 0;
            while x < width {
                if !initial {
                    c_q |= (at(&scratch, above.wrapping_add(sp - row)) & 0xA0) << 2;
                    c_q |= (at(&scratch, above.wrapping_add(sp - row + 2)) & 0x20) << 4;
                }
                let table = if initial { &VLC_TBL0 } else { &VLC_TBL1 };
                // first quad
                let mut vlc_val = vlc.fetch();
                let mut t0 = u32::from(table[((c_q + (vlc_val & 0x7F)) & 0x3FF) as usize]);
                if c_q == 0 && !mel_event(&mut run, &mut mel) {
                    t0 = 0;
                }
                if let Some(s) = scratch.get_mut(sp) {
                    *s = t0 as u16;
                }
                x += 2;
                if initial {
                    c_q = ((t0 & 0x10) << 3) | ((t0 & 0xE0) << 2);
                } else {
                    let up = above.wrapping_add(sp - row);
                    c_q = ((t0 & 0x40) << 2) | ((t0 & 0x80) << 1);
                    c_q |= at(&scratch, up) & 0x80;
                    c_q |= (at(&scratch, up.wrapping_add(2)) & 0xA0) << 2;
                    c_q |= (at(&scratch, up.wrapping_add(4)) & 0x20) << 4;
                }
                vlc_val = vlc.advance(t0 & 7);
                // second quad
                let mut t1 = u32::from(table[((c_q + (vlc_val & 0x7F)) & 0x3FF) as usize]);
                if c_q == 0 && x < width && !mel_event(&mut run, &mut mel) {
                    t1 = 0;
                }
                if x >= width {
                    t1 = 0;
                }
                if let Some(s) = scratch.get_mut(sp + 2) {
                    *s = t1 as u16;
                }
                x += 2;
                if initial {
                    c_q = ((t1 & 0x10) << 3) | ((t1 & 0xE0) << 2);
                } else {
                    c_q = ((t1 & 0x40) << 2) | ((t1 & 0x80) << 1);
                    c_q |= at(&scratch, above.wrapping_add(sp - row + 2)) & 0x80;
                }
                vlc_val = vlc.advance(t1 & 7);
                // u of both quads
                let mut mode = ((t0 & 0x8) << 3) | ((t1 & 0x8) << 4);
                if initial && mode == 0xC0 && mel_event(&mut run, &mut mel) {
                    mode += 0x40;
                }
                let utab = if initial { &UVLC_TBL0 } else { &UVLC_TBL1 };
                let mut entry = u32::from(utab.get((mode + (vlc_val & 0x3F)) as usize).copied().unwrap_or(0));
                vlc_val = vlc.advance(entry & 7);
                entry >>= 3;
                let len = entry & 0xF;
                let tmp = vlc_val & ((1u32 << len) - 1);
                vlc.advance(len);
                entry >>= 4;
                let len0 = entry & 7;
                entry >>= 3;
                // the first row's u carries kappa = 1
                let k = u32::from(initial);
                let u0 = k + (entry & 7) + (tmp & !(0xFFu32 << len0));
                let u1 = k + (entry >> 3) + (tmp >> len0);
                if let Some(s) = scratch.get_mut(sp + 1) {
                    *s = u0 as u16;
                }
                if let Some(s) = scratch.get_mut(sp + 3) {
                    *s = u1 as u16;
                }
                sp += 4;
            }
            if let Some(s) = scratch.get_mut(sp..sp + 2) {
                s.fill(0);
            }
        }
    }
    // MagSgn
    let mut ms = Fwd::new(d, 0, lcup - scup, 0xFF);
    let mut v_n_row = vec![0u32; width.div_ceil(2) + 4];
    for y in (0..height).step_by(2) {
        let mut sp = (y >> 1) * sstr;
        let mut vp = 0usize;
        let mut dp = y * stride;
        let mut prev_v_n = 0u32;
        let mut x = 0;
        while x < width {
            let inf = u32::from(scratch.get(sp).copied().unwrap_or(0));
            let u_q = u32::from(scratch.get(sp + 1).copied().unwrap_or(0));
            let big_u = if y == 0 {
                u_q
            } else {
                // kappa: the largest exponent of the quads above when more than one sample is
                // significant
                let mut gamma = inf & 0xF0;
                gamma &= gamma.wrapping_sub(0x10);
                let emax = v_n_row.get(vp).copied().unwrap_or(0) | v_n_row.get(vp + 1).copied().unwrap_or(0);
                let emax = 31 - (emax | 2).leading_zeros();
                u_q + if gamma != 0 { emax } else { 1 }
            };
            if big_u > mmsbp2 {
                return Err(Error::Invalid("HT exponent exceeds the code-block's bit-planes"));
            }
            let (v0, _) = mag_sgn(&mut ms, inf, 0, big_u, p);
            let (v1, n1) = mag_sgn(&mut ms, inf, 1, big_u, p);
            set(out, dp, v0);
            set(out, dp + stride, v1);
            if let Some(v) = v_n_row.get_mut(vp) {
                *v = prev_v_n | n1;
            }
            prev_v_n = 0;
            dp += 1;
            x += 1;
            if x >= width {
                vp += 1;
                break;
            }
            let (v2, _) = mag_sgn(&mut ms, inf, 2, big_u, p);
            let (v3, n3) = mag_sgn(&mut ms, inf, 3, big_u, p);
            set(out, dp, v2);
            set(out, dp + stride, v3);
            prev_v_n = n3;
            dp += 1;
            x += 1;
            sp += 2;
            vp += 1;
        }
        if let Some(v) = v_n_row.get_mut(vp) {
            *v = prev_v_n;
        }
    }
    Ok(())
}

fn set(out: &mut [u32], i: usize, v: u32) {
    if let Some(o) = out.get_mut(i) {
        *o = v;
    }
}

/// The significance propagation pass and, with three passes, the magnitude refinement pass.
fn refinement(d: &[u8], lcup: usize, len2: usize, b: &Block, p: u32, passes: u32, out: &mut [u32]) {
    let (width, height, stride) = (b.w, b.h, b.w);
    let stripe_causal = b.style & STYLE_VSC != 0;
    // column significance: 4 bits per column of 4 rows, from the decoded samples (a sample is
    // significant after the cleanup pass exactly when it is non-zero)
    let mstr = ((((width + 3) >> 2) + 2) + 7) & !7;
    let rows = height.div_ceil(4) + 1;
    let mut sigma = vec![0u16; mstr * rows + 2];
    for y in 0..height {
        for x in 0..width {
            if out.get(y * stride + x).is_some_and(|&v| v != 0)
                && let Some(s) = sigma.get_mut((y >> 2) * mstr + (x >> 2))
            {
                *s |= 1 << (((x & 3) << 2) | (y & 3));
            }
        }
    }
    // significance propagation
    let mut prev_row_sig = vec![0u16; (width >> 2) + 10];
    let mut sp = Fwd::new(d, lcup, len2, 0);
    for y in (0..height).step_by(4) {
        let mut pattern: u32 = match height - y {
            1 => 0x1111,
            2 => 0x3333,
            3 => 0x7777,
            _ => 0xFFFF,
        };
        let mut prev = 0u32;
        let cur_row = (y >> 2) * mstr;
        let dpp = y * stride;
        for (g, x) in (0..width).step_by(4).enumerate() {
            let s = (x + 4).saturating_sub(width) as u32;
            pattern >>= s * 4;
            let ps = pair(&prev_row_sig, g);
            let ns = pair(&sigma, cur_row + mstr + g);
            let mut u = (ps & 0x8888_8888) >> 3;
            if !stripe_causal {
                u |= (ns & 0x1111_1111) << 3;
            }
            let cs = pair(&sigma, cur_row + g);
            let mut mbr = cs;
            mbr |= (cs & 0x7777_7777) << 1;
            mbr |= (cs & 0xEEEE_EEEE) >> 1;
            mbr |= u;
            let t = mbr;
            mbr |= t << 4;
            mbr |= t >> 4;
            mbr |= prev >> 12;
            mbr &= pattern;
            mbr &= !cs;
            let mut new_sig = mbr;
            if new_sig != 0 {
                let mut cwd = sp.fetch();
                let mut cnt = 0;
                let inv_sig = !cs & pattern;
                for i in (0..16).step_by(4) {
                    let col_mask = 0xFu32 << i;
                    if col_mask & new_sig == 0 {
                        continue;
                    }
                    let mut sample_mask = 0x1111u32 & col_mask;
                    for spread in [0x33u32, 0x76, 0xEC, 0xC8] {
                        if new_sig & sample_mask != 0 {
                            new_sig &= !sample_mask;
                            if cwd & 1 != 0 {
                                new_sig |= (spread << i) & inv_sig;
                            }
                            cwd >>= 1;
                            cnt += 1;
                        }
                        sample_mask <<= 1;
                    }
                }
                if new_sig != 0 {
                    // signs of the samples that became significant
                    let val = 3u32 << (p - 2);
                    for i in 0..4 {
                        let col_mask = 0xFu32 << (4 * i);
                        if col_mask & new_sig == 0 {
                            continue;
                        }
                        let mut sample_mask = 0x1111u32 & col_mask;
                        for r in 0..4 {
                            if new_sig & sample_mask != 0 {
                                set(out, dpp + x + i + r * stride, (cwd << 31) | val);
                                cwd >>= 1;
                                cnt += 1;
                            }
                            sample_mask <<= 1;
                        }
                    }
                }
                sp.advance(cnt);
            }
            new_sig |= cs;
            if let Some(s) = prev_row_sig.get_mut(g) {
                *s = new_sig as u16;
            }
            let t = new_sig;
            new_sig |= (t & 0x7777) << 1;
            new_sig |= (t & 0xEEEE) >> 1;
            prev = (new_sig | u) & 0xF000;
        }
    }
    if passes < 3 {
        return;
    }
    // magnitude refinement of the samples significant after the cleanup pass
    let mut mrp = Rev::mrp(d, lcup, len2);
    let half = 1u32 << (p - 2);
    for y in (0..height).step_by(4) {
        let row = (y >> 2) * mstr;
        let dpp = y * stride;
        for (k, i) in (0..width).step_by(8).enumerate() {
            let mut cwd = mrp.fetch();
            let sig = pair(&sigma, row + 2 * k);
            if sig != 0 {
                for j in 0..8 {
                    let col_mask = 0xFu32 << (4 * j);
                    if sig & col_mask == 0 {
                        continue;
                    }
                    let mut sample_mask = 0x1111_1111u32 & col_mask;
                    for r in 0..4 {
                        if sig & sample_mask != 0 {
                            let at = dpp + i + j + r * stride;
                            let sym = ((1 - (cwd & 1)) << (p - 1)) | half;
                            if let Some(o) = out.get_mut(at) {
                                *o ^= sym;
                            }
                            cwd >>= 1;
                        }
                        sample_mask <<= 1;
                    }
                }
            }
            mrp.advance(sig.count_ones());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vlc_tables_cover_every_codeword_of_their_contexts() {
        // each context's codewords form a complete prefix code over 7 bits
        for (src, table) in [(&VLC0[..], &VLC_TBL0), (&VLC1[..], &VLC_TBL1)] {
            for c_q in 0..8usize {
                if src.iter().any(|e| usize::from(e[0]) == c_q) {
                    assert!((0..128).all(|i| table[(c_q << 7) | i] & 7 != 0), "context {c_q}");
                }
            }
        }
    }

    #[test]
    fn random_segments_never_panic() {
        let mut x = 0x1234_5678u32;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        for (w, h) in [(1, 1), (3, 5), (4, 4), (17, 9), (64, 64), (1024, 4), (4, 1024)] {
            for _ in 0..40 {
                let len = 2 + rnd() as usize % 300;
                let mut cup: Vec<u8> = (0..len).map(|_| rnd() as u8).collect();
                // a valid MEL + VLC length, so the passes run on the noise
                let scup = 2 + rnd() as usize % (len - 1).min(4078);
                cup[len - 1] = (scup >> 4) as u8;
                cup[len - 2] = (cup[len - 2] & 0xF0) | (scup & 0xF) as u8;
                let refine: Vec<u8> = (0..rnd() as usize % 40).map(|_| rnd() as u8).collect();
                let segments = [(cup, 1 + 3 * (rnd() % 3)), (refine, 1 + rnd() % 2)];
                let mb = 1 + rnd() % 30;
                let b = Block { w, h, orient: 0, style: STYLE_HT | (rnd() as u8 & STYLE_VSC), mb, zero: rnd() % mb, segments: &segments };
                if let Ok(v) = decode(&b) {
                    assert_eq!(v.len(), w * h);
                }
            }
        }
    }
}
