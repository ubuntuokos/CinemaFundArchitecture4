//! EBCOT tier-1 encoding (T.800 Annex D): the coding passes of one code-block with the MQ
//! encoder, mirroring [`crate::t1`]'s decoder decision for decision, and per pass the
//! truncation length and the distortion it removes (for rate control).

use crate::mq::{CONTEXTS, CTX_RL, CTX_UNIFORM, Ctx, MqEncoder, initial_contexts, truncation_length};
use crate::t1::{NEG, NEG_E, NEG_N, NEG_S, NEG_W, NEIGHBOURS, REFINED, SC_LUT, SIG, SIG_E, SIG_N, SIG_NE, SIG_NW, SIG_S, SIG_SE, SIG_SW, SIG_W, VISIT, ZC_LUT};

/// A truncation point: the code-block's first `len` bytes decode the passes up to it, which
/// remove `dist` squared error (in quantization-index units) from the all-zero block.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Pass {
    pub len: usize,
    pub dist: f64,
}

/// An encoded code-block: the terminated codeword, its passes and the coded bit-planes.
#[derive(Clone, Debug, Default)]
pub(crate) struct Encoded {
    pub data: Vec<u8>,
    pub passes: Vec<Pass>,
    pub planes: u32,
}

struct State<'a> {
    w: usize,
    h: usize,
    stride: usize,
    flags: Vec<u16>,
    /// Magnitudes as the decoder reconstructs them, with one fractional bit (see `crate::t1`).
    rec: Vec<u32>,
    /// Quantization indices and the exact values (index units), by bordered position.
    q: Vec<u32>,
    neg: Vec<bool>,
    exact: Vec<f32>,
    zc: &'a [u8; 256],
    mq: MqEncoder,
    cx: [Ctx; CONTEXTS],
    /// Distortion removed so far.
    dist: f64,
}

impl State<'_> {
    fn at(&self, x: usize, y: usize) -> usize {
        (y + 1) * self.stride + x + 1
    }

    fn flags(&self, i: usize) -> u16 {
        self.flags.get(i).copied().unwrap_or(0)
    }

    fn or(&mut self, i: usize, bits: u16) {
        if let Some(f) = self.flags.get_mut(i) {
            *f |= bits;
        }
    }

    fn zc(&self, f: u16) -> usize {
        usize::from(self.zc[usize::from((f >> 4) as u8)])
    }

    fn bit(&self, i: usize, plane: u32) -> u8 {
        (self.q.get(i).copied().unwrap_or(0) >> plane) as u8 & 1
    }

    /// The reconstruction at `i` becomes `rec`: the distortion it removes is counted.
    fn reconstruct(&mut self, i: usize, rec: u32) {
        let v = f64::from(self.exact.get(i).copied().unwrap_or(0.0).abs());
        let old = f64::from(self.rec.get(i).copied().unwrap_or(0)) / 2.0;
        let new = f64::from(rec) / 2.0;
        self.dist += (v - old).powi(2) - (v - new).powi(2);
        if let Some(r) = self.rec.get_mut(i) {
            *r = rec;
        }
    }

    /// Codes the sign of the sample at `i`, which becomes significant in `plane`.
    fn become_significant(&mut self, i: usize, plane: u32) {
        let f = self.flags(i);
        let (c, xor) = SC_LUT[usize::from((f >> 4) & 0xF) | usize::from((f >> 12) << 4)];
        let neg = self.neg.get(i).copied().unwrap_or(false);
        let mut ctx = self.cx[usize::from(c)];
        self.mq.encode(u8::from(neg) ^ xor, &mut ctx);
        self.cx[usize::from(c)] = ctx;
        let s = self.stride;
        self.or(i, SIG | if neg { NEG } else { 0 });
        self.or(i - s, SIG_S | if neg { NEG_S } else { 0 });
        self.or(i + s, SIG_N | if neg { NEG_N } else { 0 });
        self.or(i - 1, SIG_E | if neg { NEG_E } else { 0 });
        self.or(i + 1, SIG_W | if neg { NEG_W } else { 0 });
        self.or(i - s - 1, SIG_SE);
        self.or(i - s + 1, SIG_SW);
        self.or(i + s - 1, SIG_NE);
        self.or(i + s + 1, SIG_NW);
        self.reconstruct(i, 3 << plane);
    }

    fn code(&mut self, d: u8, c: usize) {
        let mut ctx = self.cx[c];
        self.mq.encode(d, &mut ctx);
        self.cx[c] = ctx;
    }

    /// Significance propagation pass (D.3.1).
    fn spp(&mut self, plane: u32) {
        let s = self.stride;
        for y0 in (0..self.h).step_by(4) {
            let rows = (self.h - y0).min(4);
            for x in 0..self.w {
                for k in 0..rows {
                    let i = self.at(x, y0) + k * s;
                    let f = self.flags(i);
                    if f & SIG != 0 || f & NEIGHBOURS == 0 {
                        continue;
                    }
                    let b = self.bit(i, plane);
                    self.code(b, self.zc(f));
                    if b == 1 {
                        self.become_significant(i, plane);
                    }
                    self.or(i, VISIT);
                }
            }
        }
    }

    /// Magnitude refinement pass (D.3.3).
    fn mrp(&mut self, plane: u32) {
        let s = self.stride;
        for y0 in (0..self.h).step_by(4) {
            let rows = (self.h - y0).min(4);
            for x in 0..self.w {
                for k in 0..rows {
                    let i = self.at(x, y0) + k * s;
                    let f = self.flags(i);
                    if f & (SIG | VISIT) != SIG {
                        continue;
                    }
                    let c = if f & REFINED != 0 {
                        16
                    } else if f & NEIGHBOURS != 0 {
                        15
                    } else {
                        14
                    };
                    let b = self.bit(i, plane);
                    self.code(b, c);
                    let r = self.rec.get(i).copied().unwrap_or(0);
                    self.reconstruct(i, if b == 1 { r.wrapping_add(1 << plane) } else { r.wrapping_sub(1 << plane) });
                    self.or(i, REFINED);
                }
            }
        }
    }

    /// Cleanup pass (D.3.4) with run-length coding.
    fn cup(&mut self, plane: u32) {
        let s = self.stride;
        for y0 in (0..self.h).step_by(4) {
            let rows = (self.h - y0).min(4);
            for x in 0..self.w {
                let col = self.at(x, y0);
                let mut k = 0;
                let quiet = rows == 4 && (0..4).all(|k| self.flags(col + k * s) & (SIG | VISIT | NEIGHBOURS) == 0);
                if quiet {
                    let first = (0..4).find(|&k| self.bit(col + k * s, plane) == 1);
                    let Some(first) = first else {
                        self.code(0, CTX_RL);
                        continue;
                    };
                    self.code(1, CTX_RL);
                    self.code((first >> 1) as u8, CTX_UNIFORM);
                    self.code((first & 1) as u8, CTX_UNIFORM);
                    self.become_significant(col + first * s, plane);
                    k = first + 1;
                }
                while k < rows {
                    let i = col + k * s;
                    let f = self.flags(i);
                    if f & (SIG | VISIT) == 0 {
                        let b = self.bit(i, plane);
                        self.code(b, self.zc(f));
                        if b == 1 {
                            self.become_significant(i, plane);
                        }
                    }
                    k += 1;
                }
            }
        }
        for f in &mut self.flags {
            *f &= !VISIT;
        }
    }

    fn end_pass(&self, passes: &mut Vec<Pass>, points: &mut Vec<(usize, u32)>) {
        passes.push(Pass { len: 0, dist: self.dist });
        points.push(self.mq.truncation_point());
    }
}

/// Encodes a `w` × `h` code-block of subband orientation `orient` from its quantization indices
/// `q` (magnitudes below 2^30) and the exact values they were quantized from (index units, for
/// the distortion), in raster order.
pub(crate) fn encode(q: &[i32], exact: &[f32], w: usize, h: usize, orient: u8) -> Encoded {
    let max = q.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0).min((1 << 30) - 1);
    let planes = 32 - max.leading_zeros();
    if planes == 0 || w == 0 || h == 0 {
        return Encoded::default();
    }
    let stride = w + 2;
    let padded = stride * (h + 2);
    let mut st = State {
        w,
        h,
        stride,
        flags: vec![0; padded],
        rec: vec![0; padded],
        q: vec![0; padded],
        neg: vec![false; padded],
        exact: vec![0.0; padded],
        zc: &ZC_LUT[usize::from(orient & 3)],
        mq: MqEncoder::new(),
        cx: initial_contexts(),
        dist: 0.0,
    };
    for y in 0..h {
        for x in 0..w {
            let i = st.at(x, y);
            let k = y * w + x;
            let v = q.get(k).copied().unwrap_or(0);
            st.q[i] = v.unsigned_abs().min(max);
            st.neg[i] = v < 0;
            st.exact[i] = exact.get(k).copied().unwrap_or(0.0);
        }
    }
    let mut passes = Vec::with_capacity(3 * planes as usize);
    let mut points = Vec::with_capacity(3 * planes as usize);
    st.cup(planes - 1);
    st.end_pass(&mut passes, &mut points);
    for plane in (0..planes - 1).rev() {
        st.spp(plane);
        st.end_pass(&mut passes, &mut points);
        st.mrp(plane);
        st.end_pass(&mut passes, &mut points);
        st.cup(plane);
        st.end_pass(&mut passes, &mut points);
    }
    let data = st.mq.finish();
    // the lengths can only grow; a truncation never ends on 0xFF (the decoder pads with 1 bits
    // anyway, and a packet body must not end in what could start a marker)
    let mut prev = 0;
    for (p, &point) in passes.iter_mut().zip(&points) {
        let mut len = truncation_length(&data, point);
        while len > 0 && data.get(len - 1) == Some(&0xFF) {
            len -= 1;
        }
        p.len = len.max(prev);
        prev = p.len;
    }
    if let Some(last) = passes.last_mut() {
        last.len = data.len();
    }
    Encoded { data, passes, planes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::t1::{Block, decode};

    fn rnd(seed: &mut u32) -> u32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;
        *seed
    }

    /// Every truncation point decodes to the reconstruction the encoder counted, and the whole
    /// codeword to the indices.
    #[test]
    fn truncated_codewords_decode_to_their_passes() {
        let mut seed = 0xACE1u32;
        for &(w, h, spread) in CASES.iter().chain(CASES) {
            for orient in 0..4u8 {
                let q: Vec<i32> = (0..w * h)
                    .map(|_| {
                        let r = rnd(&mut seed);
                        // mostly small values, some zero runs
                        if r.is_multiple_of(5) { 0 } else { (r >> 8) as i32 % spread * if r & 1 == 0 { 1 } else { -1 } }
                    })
                    .collect();
                let exact: Vec<f32> = q.iter().map(|&v| v as f32).collect();
                let e = encode(&q, &exact, w, h, orient);
                let mb = e.planes + 2;
                let full = decode(&Block { w, h, orient, style: 0, mb, zero: 2, segments: &[(e.data.clone(), e.passes.len() as u32)] }).unwrap();
                let want: Vec<i32> = q.iter().map(|&v| v * 2).collect();
                assert_eq!(full.iter().map(|&v| if v < 0 { -((-v) >> 1) * 2 } else { (v >> 1) * 2 }).collect::<Vec<_>>(), want, "{w}x{h} orient {orient}");
                for (k, p) in e.passes.iter().enumerate() {
                    assert_ne!(e.data[..p.len].last(), Some(&0xFF));
                    let part = decode(&Block { w, h, orient, style: 0, mb, zero: 2, segments: &[(e.data[..p.len].to_vec(), k as u32 + 1)] }).unwrap();
                    // the distortion counted for this truncation is what the decoder gives
                    let d: f64 = q.iter().zip(&part).map(|(&v, &r)| f64::from(v).powi(2) - (f64::from(v.abs()) - f64::from(r.abs()) / 2.0).powi(2)).sum();
                    assert!((d - p.dist).abs() < 1e-6 * d.abs().max(1.0), "{w}x{h} orient {orient} pass {k}: {d} vs {}", p.dist);
                }
            }
        }
    }

    /// (width, height, magnitude spread) of the random blocks.
    const CASES: &[(usize, usize, i32)] = &[(4, 4, 7), (32, 32, 3000), (17, 9, 40), (64, 64, 100_000), (5, 31, 1), (64, 1, 255)];
}
