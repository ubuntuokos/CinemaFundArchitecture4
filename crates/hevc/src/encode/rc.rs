//! Frame-level rate control (constant QP, constant quality, 1-pass ABR with a VBV) and the cheap
//! half-resolution complexity analysis that feeds it. The model follows the H.264 encoder's
//! (`crates/h264enc/src/ratecontrol.rs`): bits ≈ coef · complexity / qscale per frame type.

pub fn qp2qscale(qp: f64) -> f64 {
    0.85 * 2f64.powf((qp - 12.0) / 6.0)
}

pub fn qscale2qp(q: f64) -> f64 {
    12.0 + 6.0 * (q / 0.85).log2()
}

const QCOMP: f64 = 0.6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    I,
    P,
}

fn type_factor(t: Kind) -> f64 {
    match t {
        Kind::I => 1.0 / 1.4,
        Kind::P => 1.0,
    }
}

#[derive(Clone, Debug)]
pub enum RcKind {
    Qp(u8),
    Crf(f32),
    Abr { kbps: u32 },
}

pub struct RateControl {
    kind: RcKind,
    fps: f64,
    coef: [f64; 2],
    coef_n: [u32; 2],
    total_bits: f64,
    frames: u64,
    sum_s: f64,
    vbv_max_bits_per_frame: f64,
    vbv_size: f64,
    vbv_fill: f64,
    prior: Option<(f64, f64)>,
    inter_s: f64,
    inter_n: u64,
}

impl RateControl {
    pub fn new(kind: RcKind, fps: f64, vbv_maxrate_kbps: Option<u32>, vbv_buf_kbits: Option<u32>) -> Self {
        let fps = if fps.is_finite() && fps > 0.0 { fps } else { 25.0 };
        let max = vbv_maxrate_kbps.map_or(0.0, |k| k as f64 * 1000.0 / fps);
        let size = vbv_buf_kbits.map_or(0.0, |k| k as f64 * 1000.0);
        RateControl {
            kind,
            fps,
            coef: [1.6, 1.4],
            coef_n: [0; 2],
            total_bits: 0.0,
            frames: 0,
            sum_s: 0.0,
            vbv_max_bits_per_frame: max,
            vbv_size: size,
            vbv_fill: size * 0.9,
            prior: None,
            inter_s: 0.0,
            inter_n: 0,
        }
    }

    /// QP for the next frame of type `t` with complexity `cplx`.
    pub fn frame_qp(&mut self, t: Kind, cplx: f64) -> f64 {
        let ti = t as usize;
        let cplx = cplx.max(1.0);
        let qp = match &self.kind {
            RcKind::Qp(q) => return if t == Kind::I { (*q as f64 - 2.0).max(0.0) } else { *q as f64 },
            RcKind::Crf(c) => return (*c as f64 + if t == Kind::I { -2.0 } else { 0.0 }).clamp(0.0, 51.0),
            RcKind::Abr { kbps } => {
                let bpf = *kbps as f64 * 1000.0 / self.fps;
                let s_here = self.coef[ti] * cplx.powf(QCOMP) / type_factor(t);
                let fps = self.fps;
                let (pn0, c0) = *self.prior.get_or_insert_with(|| (fps.max(1.0), if t == Kind::I { 0.4 * cplx } else { cplx }));
                let per_frame = if self.inter_n > 0 { self.inter_s / self.inter_n as f64 } else { self.coef[1] * c0.powf(QCOMP) };
                let fade = (1.0 - self.frames as f64 / pn0).max(0.0);
                let (pn, ps) = (pn0 * fade, pn0 * fade * per_frame);
                let s = self.sum_s + s_here + ps;
                let wanted = bpf * (self.frames as f64 + 1.0 + pn);
                let mut rf = s / wanted;
                let abr_buffer = bpf * self.fps.max(1.0);
                let overflow = (1.0 + (self.total_bits - bpf * self.frames as f64) / abr_buffer).clamp(0.5, 2.0);
                rf *= overflow;
                qscale2qp(rf * cplx.powf(1.0 - QCOMP) * type_factor(t))
            }
        };
        let mut qp = qp.clamp(1.0, 51.0);
        if self.vbv_size > 0.0 {
            for _ in 0..60 {
                let pred = self.coef[ti] * cplx / qp2qscale(qp);
                if pred <= self.vbv_fill * 0.8 || qp >= 51.0 {
                    break;
                }
                qp += 0.5;
            }
        }
        qp
    }

    /// Update the model with the size of the frame just coded.
    pub fn update(&mut self, t: Kind, qp: f64, cplx: f64, bits: u64) {
        let ti = t as usize;
        let cplx = cplx.max(1.0);
        let c = bits as f64 * qp2qscale(qp) / cplx;
        let n = self.coef_n[ti];
        let w = if n == 0 { 1.0 } else { 0.35f64.max(1.0 / (n + 1) as f64) };
        self.coef[ti] = self.coef[ti] * (1.0 - w) + c * w;
        self.coef_n[ti] += 1;
        let other = 1 - ti;
        if self.coef_n[other] == 0 {
            self.coef[other] = self.coef[ti] * [1.2, 1.0][other] / [1.2, 1.0][ti];
        }
        let s_frame = self.coef[ti] * cplx.powf(QCOMP) / type_factor(t);
        self.sum_s += s_frame;
        if t != Kind::I {
            self.inter_s += s_frame;
            self.inter_n += 1;
        }
        self.total_bits += bits as f64;
        self.frames += 1;
        if self.vbv_size > 0.0 {
            self.vbv_fill = (self.vbv_fill - bits as f64 + self.vbv_max_bits_per_frame).clamp(0.0, self.vbv_size);
        }
    }
}

/// Half-resolution luma at 8-bit scale.
pub struct LowRes {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

impl LowRes {
    pub fn new(y: &[u16], w: usize, h: usize, bit_depth: u32) -> Self {
        let (lw, lh) = (w / 2, h / 2);
        let sh = bit_depth.saturating_sub(8) + 2;
        let mut data = vec![0u8; lw * lh];
        for yy in 0..lh {
            for xx in 0..lw {
                let (a, b) = (y[2 * yy * w + 2 * xx] as u32, y[2 * yy * w + 2 * xx + 1] as u32);
                let (c, d) = (y[(2 * yy + 1) * w + 2 * xx] as u32, y[(2 * yy + 1) * w + 2 * xx + 1] as u32);
                data[yy * lw + xx] = ((a + b + c + d + (1 << (sh - 1))) >> sh).min(255) as u8;
            }
        }
        LowRes { w: lw, h: lh, data }
    }
}

fn sad8(a: &[u8], astride: usize, b: &[u8], bstride: usize) -> u32 {
    let mut s = 0;
    for y in 0..8 {
        for x in 0..8 {
            s += (a[y * astride + x] as i32 - b[y * bstride + x] as i32).unsigned_abs();
        }
    }
    s
}

/// Frame complexity: the sum over 8x8 low-res blocks of the cheaper of a DC/H/V intra prediction
/// error and (with a previous frame) a small-diamond motion search error.
pub fn complexity(cur: &LowRes, prev: Option<&LowRes>, intra_only: bool) -> f64 {
    let (bw, bh) = (cur.w / 8, cur.h / 8);
    let mut total = 0u64;
    let mut pred = [0u8; 64];
    for by in 0..bh {
        for bx in 0..bw {
            let (x0, y0) = (bx * 8, by * 8);
            let base = y0 * cur.w + x0;
            let mut best = u32::MAX;
            for mode in 0..3 {
                if (mode == 1 && x0 == 0) || (mode == 2 && y0 == 0) {
                    continue;
                }
                for y in 0..8 {
                    for x in 0..8 {
                        pred[y * 8 + x] = match mode {
                            0 if x0 > 0 && y0 > 0 => ((cur.data[(y0 - 1) * cur.w + x0 + x] as u32 + cur.data[(y0 + y) * cur.w + x0 - 1] as u32) / 2) as u8,
                            0 => 128,
                            1 => cur.data[(y0 + y) * cur.w + x0 - 1],
                            _ => cur.data[(y0 - 1) * cur.w + x0 + x],
                        };
                    }
                }
                best = best.min(sad8(&cur.data[base..], cur.w, &pred, 8));
            }
            let mut cost = best as u64 + 16;
            if let (Some(p), false) = (prev, intra_only) {
                let at = |mx: i32, my: i32| -> u32 {
                    let (x, y) = (x0 as i32 + mx, y0 as i32 + my);
                    if x < 0 || y < 0 || x + 8 > p.w as i32 || y + 8 > p.h as i32 {
                        return u32::MAX;
                    }
                    sad8(&cur.data[base..], cur.w, &p.data[y as usize * p.w + x as usize..], p.w)
                };
                let mut bmv = (0, 0);
                let mut bc = at(0, 0);
                for _ in 0..8 {
                    let c = bmv;
                    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-2, 0), (2, 0), (0, -2), (0, 2)] {
                        let k = at(c.0 + dx, c.1 + dy);
                        if k < bc {
                            bc = k;
                            bmv = (c.0 + dx, c.1 + dy);
                        }
                    }
                    if bmv == c {
                        break;
                    }
                }
                cost = cost.min(bc as u64 + 8);
            }
            total += cost;
        }
    }
    total as f64
}
