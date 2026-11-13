// Copyright (c) 2019, Aous Naman
// Copyright (c) 2019, Kakadu Software Pty Ltd, Australia
// Copyright (c) 2019, The University of New South Wales, Australia
// SPDX-License-Identifier: BSD-2-Clause
//
// Ported to safe Rust for CFA4 from OpenJPH 0.28.1's `ojph_block_encoder.cpp`
// (`ojph_encode_codeblock32`, the MEL, VLC and MagSgn writers and the table set-up; BSD-2-Clause,
// see `crates/j2k/src/LICENSE-OpenJPH`). The fixed-size stack buffers became growable vectors,
// the backward VLC buffer a vector reversed at the end, and the pointer walks indices.

//! HTJ2K (ITU-T T.814 | ISO/IEC 15444-15) block encoder: the HT cleanup pass of one code-block,
//! with every bit-plane of its quantization indices (no SigProp / MagRef passes), the inverse of
//! [`crate::ht`]'s decoder.

use std::sync::LazyLock;

use crate::ht_tables::{VLC0, VLC1};
use crate::{Error, Result};

/// The VLC encoder table of one row kind, indexed by context (3 bits), rho (4) and the
/// u_off-relevant EMB pattern (4): codeword in bits 8-15, its length in 4-6, e_k in 0-3.
fn vlc_encoder_table(src: &[[u8; 7]]) -> Vec<u16> {
    let mut t = vec![0u16; 2048];
    for (i, slot) in t.iter_mut().enumerate() {
        let (c_q, rho, emb) = ((i >> 8) as u8, ((i >> 4) & 0xF) as u8, (i & 0xF) as u8);
        if emb & rho != emb || (rho == 0 && c_q == 0) {
            continue;
        }
        let mut best: Option<&[u8; 7]> = None;
        if emb != 0 {
            // u_off = 1: of the codewords whose known EMB bits match, the one that tells the most
            let mut best_e_k = -1;
            for e in src {
                if e[0] == c_q && e[1] == rho && e[2] == 1 && emb & e[3] == e[4] {
                    let ones = e[3].count_ones() as i32;
                    if ones >= best_e_k {
                        best = Some(e);
                        best_e_k = ones;
                    }
                }
            }
        } else {
            best = src.iter().find(|e| e[0] == c_q && e[1] == rho && e[2] == 0);
        }
        if let Some(e) = best {
            *slot = (u16::from(e[5]) << 8) | (u16::from(e[6]) << 4) | u16::from(e[3]);
        }
    }
    t
}

static VLC_ENC: LazyLock<[Vec<u16>; 2]> = LazyLock::new(|| [vlc_encoder_table(&VLC0), vlc_encoder_table(&VLC1)]);

/// UVLC code of an unsigned residual u (T.814 Table 3): prefix, its length, suffix, its length.
/// The encoder never needs the extension (u <= 30 with at most 30 bit-planes).
fn uvlc(u: i32) -> (u32, u32, u32, u32) {
    match u {
        0 => (0, 0, 0, 0),
        1 => (1, 1, 0, 0),
        2 => (2, 2, 0, 0),
        3 => (4, 3, 0, 1),
        4 => (4, 3, 1, 1),
        _ => (0, 3, (u - 5) as u32, 5),
    }
}

/// The MEL (adaptive run-length) writer.
struct Mel {
    buf: Vec<u8>,
    remaining_bits: u32,
    tmp: u32,
    run: u32,
    k: usize,
    threshold: u32,
}

const MEL_EXP: [u32; 13] = [0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 4, 5];

impl Mel {
    fn new() -> Self {
        Mel { buf: Vec::new(), remaining_bits: 8, tmp: 0, run: 0, k: 0, threshold: 1 }
    }

    fn emit_bit(&mut self, v: u32) {
        self.tmp = (self.tmp << 1) + v;
        self.remaining_bits -= 1;
        if self.remaining_bits == 0 {
            self.buf.push(self.tmp as u8);
            self.remaining_bits = if self.tmp == 0xFF { 7 } else { 8 };
            self.tmp = 0;
        }
    }

    fn encode(&mut self, bit: bool) {
        if !bit {
            self.run += 1;
            if self.run >= self.threshold {
                self.emit_bit(1);
                self.run = 0;
                self.k = (self.k + 1).min(12);
                self.threshold = 1 << MEL_EXP[self.k];
            }
        } else {
            self.emit_bit(0);
            let mut t = MEL_EXP[self.k];
            while t > 0 {
                t -= 1;
                self.emit_bit((self.run >> t) & 1);
            }
            self.run = 0;
            self.k = self.k.saturating_sub(1);
            self.threshold = 1 << MEL_EXP[self.k];
        }
    }
}

/// The VLC writer; its bytes run backwards from the end of the cleanup segment, so they are
/// collected in writing order and reversed at the end.
struct Vlc {
    buf: Vec<u8>,
    used_bits: u32,
    tmp: u32,
    last_greater_than_8f: bool,
}

impl Vlc {
    fn new() -> Self {
        // the first byte and the low nibble of the second hold Scup at the end
        Vlc { buf: vec![0xFF], used_bits: 4, tmp: 0xF, last_greater_than_8f: true }
    }

    fn encode(&mut self, mut cwd: u32, mut len: u32) {
        while len > 0 {
            let mut avail = 8 - u32::from(self.last_greater_than_8f) - self.used_bits;
            let t = avail.min(len);
            self.tmp |= (cwd & ((1 << t) - 1)) << self.used_bits;
            self.used_bits += t;
            avail -= t;
            len -= t;
            cwd >>= t;
            if avail == 0 {
                if self.last_greater_than_8f && self.tmp != 0x7F {
                    // no stuffing needed: one more bit fits
                    self.last_greater_than_8f = false;
                    continue;
                }
                self.buf.push(self.tmp as u8);
                self.last_greater_than_8f = self.tmp > 0x8F;
                self.tmp = 0;
                self.used_bits = 0;
            }
        }
    }
}

/// Ends the MEL and VLC segments, fusing their last bytes when the bits allow it.
fn terminate_mel_vlc(mel: &mut Mel, vlc: &mut Vlc) {
    if mel.run > 0 {
        mel.emit_bit(1);
    }
    mel.tmp <<= mel.remaining_bits;
    let mel_mask = (0xFFu32 << mel.remaining_bits) & 0xFF;
    let vlc_mask = 0xFFu32 >> (8 - vlc.used_bits);
    if mel_mask | vlc_mask == 0 {
        return;
    }
    let fuse = mel.tmp | vlc.tmp;
    if (((fuse ^ mel.tmp) & mel_mask) | ((fuse ^ vlc.tmp) & vlc_mask)) == 0 && fuse != 0xFF && vlc.buf.len() > 1 {
        mel.buf.push(fuse as u8);
    } else {
        mel.buf.push(mel.tmp as u8);
        vlc.buf.push(vlc.tmp as u8);
    }
}

/// The MagSgn writer (forward, bit-stuffed after 0xFF).
struct MagSgn {
    buf: Vec<u8>,
    max_bits: u32,
    used_bits: u32,
    tmp: u32,
}

impl MagSgn {
    fn new() -> Self {
        MagSgn { buf: Vec::new(), max_bits: 8, used_bits: 0, tmp: 0 }
    }

    fn encode(&mut self, mut cwd: u64, mut len: u32) {
        while len > 0 {
            let t = (self.max_bits - self.used_bits).min(len);
            self.tmp |= ((cwd & ((1u64 << t) - 1)) as u32) << self.used_bits;
            self.used_bits += t;
            cwd >>= t;
            len -= t;
            if self.used_bits >= self.max_bits {
                self.buf.push(self.tmp as u8);
                self.max_bits = if self.tmp == 0xFF { 7 } else { 8 };
                self.tmp = 0;
                self.used_bits = 0;
            }
        }
    }

    fn terminate(&mut self) {
        if self.used_bits > 0 {
            let t = self.max_bits - self.used_bits;
            self.tmp |= (0xFF & ((1u32 << t) - 1)) << self.used_bits;
            if self.tmp != 0xFF {
                self.buf.push(self.tmp as u8);
            }
        } else if self.max_bits == 7 {
            self.buf.pop();
        }
    }
}

/// One quad's samples (the 2 × 2 block in column order: top left, bottom left, top right, bottom
/// right): rho, the exponents e_q, their maximum and the MagSgn values 2(mu - 1) + sign.
#[derive(Default)]
struct Quad {
    rho: usize,
    e: [i32; 4],
    e_max: i32,
    s: [u64; 4],
}

impl Quad {
    fn read(q: &[i32], w: usize, h: usize, x: usize, y: usize) -> Self {
        let mut quad = Quad::default();
        for (n, (dx, dy)) in [(0, 0), (0, 1), (1, 0), (1, 1)].into_iter().enumerate() {
            let (sx, sy) = (x + dx, y + dy);
            if sx >= w || sy >= h {
                continue;
            }
            let v = q.get(sy * w + sx).copied().unwrap_or(0);
            let mu = u64::from(v.unsigned_abs());
            if mu == 0 {
                continue;
            }
            quad.rho |= 1 << n;
            quad.e[n] = (64 - (2 * mu - 1).leading_zeros()) as i32;
            quad.e_max = quad.e_max.max(quad.e[n]);
            quad.s[n] = 2 * (mu - 1) + u64::from(v < 0);
        }
        quad
    }

    /// The EMB pattern: which samples reach the quad's exponent bound (only when u_q > 0).
    fn eps(&self, u: i32) -> usize {
        if u <= 0 {
            return 0;
        }
        (0..4).filter(|&n| self.e[n] == self.e_max).fold(0, |a, n| a | 1 << n)
    }

    /// The VLC codeword for context `c_q` and the quad's MagSgn bits.
    fn code(&self, table: &[u16], c_q: usize, u: i32, big_u: i32, vlc: &mut Vlc, mel: &mut Mel, ms: &mut MagSgn) -> Result<()> {
        let tuple = table.get((c_q << 8) + (self.rho << 4) + self.eps(u)).copied().unwrap_or(0);
        let len = u32::from((tuple >> 4) & 7);
        // an empty quad in context 0 has no codeword, only its MEL event
        if len == 0 && (self.rho != 0 || c_q != 0) {
            return Err(Error::Invalid("HT encoder: no VLC codeword"));
        }
        vlc.encode(u32::from(tuple >> 8), len);
        if c_q == 0 {
            mel.encode(self.rho != 0);
        }
        for n in 0..4 {
            if self.rho & (1 << n) != 0 {
                let m = (big_u - i32::from((tuple >> n) & 1)) as u32;
                ms.encode(self.s[n] & ((1u64 << m) - 1), m);
            }
        }
        Ok(())
    }
}

/// Largest exponent of the neighbours above a quad, from the previous quad row's bottom row.
fn max_e(e_val: &[u8], i: usize) -> i32 {
    i32::from(e_val.get(i).copied().unwrap_or(0).max(e_val.get(i + 1).copied().unwrap_or(0))) - 1
}

/// Encodes the HT cleanup pass of a `w` × `h` code-block (raster order) of quantization indices
/// with magnitudes below 2^30, coding every bit-plane: the decoder takes the block as having
/// Mb - 1 missing MSBs. `None` for an all-zero block (no passes); otherwise the cleanup segment
/// (MagSgn, MEL, VLC and the Scup interface locator in its last two bytes).
pub(crate) fn encode(q: &[i32], w: usize, h: usize) -> Result<Option<Vec<u8>>> {
    if w == 0 || h == 0 || w > 1024 || h > 1024 || w * h > 4096 || q.len() < w * h {
        return Err(Error::Invalid("HT code-block size"));
    }
    if q.iter().all(|&v| v == 0) {
        return Ok(None);
    }
    if q.iter().any(|v| v.unsigned_abs() >= 1 << 30) {
        return Err(Error::Unsupported("HT code-block with more than 30 bit-planes"));
    }
    let [tbl0, tbl1] = &*VLC_ENC;
    let mut mel = Mel::new();
    let mut vlc = Vlc::new();
    let mut ms = MagSgn::new();
    let quads = w.div_ceil(2);
    // per quad column pair: the largest exponent and the significance of the row above
    let mut e_val = vec![0u8; quads + 3];
    let mut cx_val = vec![0u8; quads + 3];

    // the first quad row: context from the left neighbour only
    let (mut lep, mut lcx) = (0usize, 0usize);
    let mut c_q0 = 0usize;
    for x in (0..w).step_by(4) {
        let q0 = Quad::read(q, w, h, x, 0);
        let big_u0 = q0.e_max.max(1);
        let u_q0 = big_u0 - 1;
        let mut u_q1 = 0;
        e_val[lep] = e_val[lep].max(q0.e[1] as u8);
        lep += 1;
        e_val[lep] = q0.e[3] as u8;
        cx_val[lcx] |= ((q0.rho & 2) >> 1) as u8;
        lcx += 1;
        cx_val[lcx] = ((q0.rho & 8) >> 3) as u8;
        q0.code(tbl0, c_q0, u_q0, big_u0, &mut vlc, &mut mel, &mut ms)?;
        let mut rho1 = 0;
        if x + 2 < w {
            let q1 = Quad::read(q, w, h, x + 2, 0);
            let c_q1 = (q0.rho >> 1) | (q0.rho & 1);
            let big_u1 = q1.e_max.max(1);
            u_q1 = big_u1 - 1;
            e_val[lep] = e_val[lep].max(q1.e[1] as u8);
            lep += 1;
            e_val[lep] = q1.e[3] as u8;
            cx_val[lcx] |= ((q1.rho & 2) >> 1) as u8;
            lcx += 1;
            cx_val[lcx] = ((q1.rho & 8) >> 3) as u8;
            q1.code(tbl0, c_q1, u_q1, big_u1, &mut vlc, &mut mel, &mut ms)?;
            rho1 = q1.rho;
        }
        if u_q0 > 0 && u_q1 > 0 {
            mel.encode(u_q0.min(u_q1) > 2);
        }
        if u_q0 > 2 && u_q1 > 2 {
            let (a, b) = (uvlc(u_q0 - 2), uvlc(u_q1 - 2));
            vlc.encode(a.0, a.1);
            vlc.encode(b.0, b.1);
            vlc.encode(a.2, a.3);
            vlc.encode(b.2, b.3);
        } else if u_q0 > 2 && u_q1 > 0 {
            let a = uvlc(u_q0);
            vlc.encode(a.0, a.1);
            vlc.encode((u_q1 - 1) as u32, 1);
            vlc.encode(a.2, a.3);
        } else {
            let (a, b) = (uvlc(u_q0), uvlc(u_q1));
            vlc.encode(a.0, a.1);
            vlc.encode(b.0, b.1);
            vlc.encode(a.2, a.3);
            vlc.encode(b.2, b.3);
        }
        c_q0 = (rho1 >> 1) | (rho1 & 1);
    }
    if let Some(e) = e_val.get_mut(lep + 1) {
        *e = 0;
    }

    // the other quad rows: context and exponent bound from the row above too
    for y in (2..h).step_by(2) {
        lep = 0;
        let mut me = max_e(&e_val, 0);
        e_val[0] = 0;
        lcx = 0;
        c_q0 = usize::from(cx_val[0]) + (usize::from(cx_val[1]) << 2);
        cx_val[0] = 0;
        for x in (0..w).step_by(4) {
            let q0 = Quad::read(q, w, h, x, y);
            let kappa = if q0.rho & q0.rho.wrapping_sub(1) != 0 { me.max(1) } else { 1 };
            let big_u0 = q0.e_max.max(kappa);
            let u_q0 = big_u0 - kappa;
            let mut u_q1 = 0;
            e_val[lep] = e_val[lep].max(q0.e[1] as u8);
            lep += 1;
            me = max_e(&e_val, lep);
            e_val[lep] = q0.e[3] as u8;
            cx_val[lcx] |= ((q0.rho & 2) >> 1) as u8;
            lcx += 1;
            let mut c_q1 = usize::from(cx_val[lcx]) + (usize::from(cx_val[lcx + 1]) << 2);
            cx_val[lcx] = ((q0.rho & 8) >> 3) as u8;
            q0.code(tbl1, c_q0, u_q0, big_u0, &mut vlc, &mut mel, &mut ms)?;
            let mut rho1 = 0;
            if x + 2 < w {
                let q1 = Quad::read(q, w, h, x + 2, y);
                let kappa = if q1.rho & q1.rho.wrapping_sub(1) != 0 { me.max(1) } else { 1 };
                c_q1 |= ((q0.rho & 4) >> 1) | ((q0.rho & 8) >> 2);
                let big_u1 = q1.e_max.max(kappa);
                u_q1 = big_u1 - kappa;
                e_val[lep] = e_val[lep].max(q1.e[1] as u8);
                lep += 1;
                me = max_e(&e_val, lep);
                e_val[lep] = q1.e[3] as u8;
                cx_val[lcx] |= ((q1.rho & 2) >> 1) as u8;
                lcx += 1;
                c_q0 = usize::from(cx_val[lcx]) + (usize::from(cx_val[lcx + 1]) << 2);
                cx_val[lcx] = ((q1.rho & 8) >> 3) as u8;
                q1.code(tbl1, c_q1, u_q1, big_u1, &mut vlc, &mut mel, &mut ms)?;
                rho1 = q1.rho;
            }
            let (a, b) = (uvlc(u_q0), uvlc(u_q1));
            vlc.encode(a.0, a.1);
            vlc.encode(b.0, b.1);
            vlc.encode(a.2, a.3);
            vlc.encode(b.2, b.3);
            c_q0 |= ((rho1 & 4) >> 1) | ((rho1 & 8) >> 2);
        }
    }

    terminate_mel_vlc(&mut mel, &mut vlc);
    ms.terminate();
    let scup = mel.buf.len() + vlc.buf.len();
    if scup > 4079 {
        return Err(Error::Invalid("HT encoder: MEL and VLC segments over 4079 bytes"));
    }
    let mut out = ms.buf;
    out.extend_from_slice(&mel.buf);
    out.extend(vlc.buf.iter().rev());
    let n = out.len();
    // Scup: its high 8 bits in the last byte, its low 4 in the low nibble of the one before
    out[n - 1] = (scup >> 4) as u8;
    out[n - 2] = (out[n - 2] & 0xF0) | (scup & 0xF) as u8;
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ht::{STYLE_HT, decode};
    use crate::t1::Block;

    /// Encodes and decodes one block; the decoder's one-fractional-bit magnitudes are 2|q| + 1.
    fn roundtrip(q: &[i32], w: usize, h: usize) {
        let mb = 32 - q.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0).leading_zeros();
        let Some(data) = encode(q, w, h).expect("encodes") else {
            assert!(q.iter().all(|&v| v == 0));
            return;
        };
        let mb = mb.max(1);
        let segments = [(data, 1)];
        let b = Block { w, h, orient: 0, style: STYLE_HT, mb, zero: mb - 1, segments: &segments };
        let got = decode(&b).expect("decodes");
        for (i, (&g, &v)) in got.iter().zip(q).enumerate() {
            let want = if v == 0 { 0 } else { v.signum() * (2 * v.abs() + 1) };
            assert_eq!(g, want, "{w}x{h} sample {i}");
        }
    }

    fn noise(seed: u32) -> impl FnMut() -> u32 {
        let mut x = seed;
        move || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        }
    }

    #[test]
    fn every_reachable_vlc_index_has_a_codeword() {
        for t in VLC_ENC.iter() {
            for (i, &e) in t.iter().enumerate() {
                let (c_q, rho, emb) = (i >> 8, (i >> 4) & 0xF, i & 0xF);
                if emb & rho == emb && !(rho == 0 && c_q == 0) {
                    assert_ne!((e >> 4) & 7, 0, "index {i:#x}");
                }
            }
        }
    }

    #[test]
    fn blocks_round_trip_through_the_decoder() {
        let mut rnd = noise(0x9E37_79B9);
        for (w, h) in [(1, 1), (2, 2), (3, 5), (4, 4), (5, 3), (7, 9), (16, 16), (32, 32), (64, 64), (1024, 4), (4, 1024), (128, 32)] {
            for bits in [1u32, 2, 3, 5, 8, 12, 17, 24, 30] {
                for density in [1u32, 4, 16, 100] {
                    let q: Vec<i32> = (0..w * h)
                        .map(|_| {
                            if rnd() % 100 >= density {
                                return 0;
                            }
                            let m = (rnd() & ((1u32 << bits) - 1)) as i32;
                            if rnd() & 1 == 1 { -m } else { m }
                        })
                        .collect();
                    roundtrip(&q, w, h);
                }
            }
        }
    }

    #[test]
    fn extreme_blocks_round_trip() {
        let max = (1 << 30) - 1;
        roundtrip(&vec![max; 4096], 64, 64);
        roundtrip(&vec![-max; 4096], 64, 64);
        roundtrip(&(0..4096).map(|i| if i % 2 == 0 { max } else { -1 }).collect::<Vec<_>>(), 64, 64);
        roundtrip(&(0..4096).map(|i| (i % 7) - 3).collect::<Vec<_>>(), 64, 64);
        assert!(encode(&[0; 16], 4, 4).expect("zero block").is_none());
        assert!(encode(&[1 << 30], 1, 1).is_err());
        assert!(encode(&[1; 8192], 128, 64).is_err());
    }
}
