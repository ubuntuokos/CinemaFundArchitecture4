//! Inverse transform, deblocking filters, intra prediction and motion compensation of AVS1-P2
//! (FFmpeg `cavsdsp.c`, `cavs.c` and the H.264 chroma interpolation it shares).

fn clip8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// The 8×8 inverse transform, added to `dst` (`cavs_idct8_add_c`). The first pass keeps
/// 16-bit intermediates, as the C code does.
pub(crate) fn idct8_add(dst: &mut [u8], off: usize, stride: usize, block: &mut [i16; 64]) {
    block[0] = block[0].wrapping_add(8);
    for i in 0..8 {
        let s = |k: usize| i32::from(block[i * 8 + k]);
        let a0 = 3 * s(1) - 2 * s(7);
        let a1 = 3 * s(3) + 2 * s(5);
        let a2 = 2 * s(3) - 3 * s(5);
        let a3 = 2 * s(1) + 3 * s(7);
        let b4 = 2 * (a0 + a1 + a3) + a1;
        let b5 = 2 * (a0 - a1 + a2) + a0;
        let b6 = 2 * (a3 - a2 - a1) + a3;
        let b7 = 2 * (a0 - a2 - a3) - a2;
        let a7 = 4 * s(2) - 10 * s(6);
        let a6 = 4 * s(6) + 10 * s(2);
        let a5 = 8 * (s(0) - s(4)) + 4;
        let a4 = 8 * (s(0) + s(4)) + 4;
        let (b0, b1, b2, b3) = (a4 + a6, a5 + a7, a5 - a7, a4 - a6);
        let r = [b0 + b4, b1 + b5, b2 + b6, b3 + b7, b3 - b7, b2 - b6, b1 - b5, b0 - b4];
        for (k, v) in r.into_iter().enumerate() {
            block[i * 8 + k] = (v >> 3) as i16;
        }
    }
    for i in 0..8 {
        let s = |k: usize| i32::from(block[k * 8 + i]);
        let a0 = 3 * s(1) - 2 * s(7);
        let a1 = 3 * s(3) + 2 * s(5);
        let a2 = 2 * s(3) - 3 * s(5);
        let a3 = 2 * s(1) + 3 * s(7);
        let b4 = 2 * (a0 + a1 + a3) + a1;
        let b5 = 2 * (a0 - a1 + a2) + a0;
        let b6 = 2 * (a3 - a2 - a1) + a3;
        let b7 = 2 * (a0 - a2 - a3) - a2;
        let a7 = 4 * s(2) - 10 * s(6);
        let a6 = 4 * s(6) + 10 * s(2);
        let a5 = 8 * (s(0) - s(4));
        let a4 = 8 * (s(0) + s(4));
        let (b0, b1, b2, b3) = (a4 + a6, a5 + a7, a5 - a7, a4 - a6);
        let r = [b0 + b4, b1 + b5, b2 + b6, b3 + b7, b3 - b7, b2 - b6, b1 - b5, b0 - b4];
        for (k, v) in r.into_iter().enumerate() {
            if let Some(p) = dst.get_mut(off + k * stride + i) {
                *p = clip8(i32::from(*p) + (v >> 7));
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// deblocking

/// Reads the six samples across an edge: `q` is the first sample after the edge, `s` the step
/// across it. `None` when the edge is not inside the plane.
fn edge(d: &[u8], q: usize, s: usize) -> Option<[i32; 6]> {
    let p2 = q.checked_sub(3 * s)?;
    let last = q.checked_add(2 * s)?;
    if last >= d.len() {
        return None;
    }
    Some([0, 1, 2, 3, 4, 5].map(|k| i32::from(d[p2 + k * s])))
}

fn put(d: &mut [u8], q: usize, s: usize, k: isize, v: i32) {
    let i = q as isize + k * s as isize;
    if let Some(p) = usize::try_from(i).ok().and_then(|i| d.get_mut(i)) {
        *p = v as u8;
    }
}

fn filter_l2(d: &mut [u8], q: usize, s: usize, alpha: i32, beta: i32) {
    let Some([p2, p1, p0, q0, q1, q2]) = edge(d, q, s) else { return };
    if (p0 - q0).abs() < alpha && (p1 - p0).abs() < beta && (q1 - q0).abs() < beta {
        let sum = p0 + q0 + 2;
        let alpha = (alpha >> 2) + 2;
        if (p2 - p0).abs() < beta && (p0 - q0).abs() < alpha {
            put(d, q, s, -1, (p1 + p0 + sum) >> 2);
            put(d, q, s, -2, (2 * p1 + sum) >> 2);
        } else {
            put(d, q, s, -1, (2 * p1 + sum) >> 2);
        }
        if (q2 - q0).abs() < beta && (q0 - p0).abs() < alpha {
            put(d, q, s, 0, (q1 + q0 + sum) >> 2);
            put(d, q, s, 1, (2 * q1 + sum) >> 2);
        } else {
            put(d, q, s, 0, (2 * q1 + sum) >> 2);
        }
    }
}

fn filter_l1(d: &mut [u8], q: usize, s: usize, alpha: i32, beta: i32, tc: i32) {
    let Some([p2, p1, p0, q0, q1, q2]) = edge(d, q, s) else { return };
    if (p0 - q0).abs() < alpha && (p1 - p0).abs() < beta && (q1 - q0).abs() < beta {
        let delta = (((q0 - p0) * 3 + p1 - q1 + 4) >> 3).clamp(-tc, tc);
        let np0 = i32::from(clip8(p0 + delta));
        let nq0 = i32::from(clip8(q0 - delta));
        put(d, q, s, -1, np0);
        put(d, q, s, 0, nq0);
        if (p2 - p0).abs() < beta {
            let delta = (((np0 - p1) * 3 + p2 - nq0 + 4) >> 3).clamp(-tc, tc);
            put(d, q, s, -2, i32::from(clip8(p1 + delta)));
        }
        if (q2 - q0).abs() < beta {
            let delta = (((q1 - nq0) * 3 + np0 - q2 + 4) >> 3).clamp(-tc, tc);
            put(d, q, s, 1, i32::from(clip8(q1 - delta)));
        }
    }
}

fn filter_c2(d: &mut [u8], q: usize, s: usize, alpha: i32, beta: i32) {
    let Some([p2, p1, p0, q0, q1, q2]) = edge(d, q, s) else { return };
    if (p0 - q0).abs() < alpha && (p1 - p0).abs() < beta && (q1 - q0).abs() < beta {
        let sum = p0 + q0 + 2;
        let alpha = (alpha >> 2) + 2;
        if (p2 - p0).abs() < beta && (p0 - q0).abs() < alpha {
            put(d, q, s, -1, (p1 + p0 + sum) >> 2);
        } else {
            put(d, q, s, -1, (2 * p1 + sum) >> 2);
        }
        if (q2 - q0).abs() < beta && (q0 - p0).abs() < alpha {
            put(d, q, s, 0, (q1 + q0 + sum) >> 2);
        } else {
            put(d, q, s, 0, (2 * q1 + sum) >> 2);
        }
    }
}

fn filter_c1(d: &mut [u8], q: usize, s: usize, alpha: i32, beta: i32, tc: i32) {
    let Some([_, p1, p0, q0, q1, _]) = edge(d, q, s) else { return };
    if (p0 - q0).abs() < alpha && (p1 - p0).abs() < beta && (q1 - q0).abs() < beta {
        let delta = (((q0 - p0) * 3 + p1 - q1 + 4) >> 3).clamp(-tc, tc);
        put(d, q, s, -1, i32::from(clip8(p0 + delta)));
        put(d, q, s, 0, i32::from(clip8(q0 - delta)));
    }
}

/// Filter strength parameters of an edge.
#[derive(Clone, Copy)]
pub(crate) struct Edge {
    pub alpha: i32,
    pub beta: i32,
    pub tc: i32,
}

/// A luma edge of 16 samples starting at `off`; `vertical` edges run down a column (the
/// filter works across x). `bs1` covers the first 8 samples, `bs2` the last 8.
pub(crate) fn filter_luma(d: &mut [u8], off: usize, stride: usize, vertical: bool, e: Edge, bs1: u8, bs2: u8) {
    let (along, across) = if vertical { (stride, 1) } else { (1, stride) };
    if bs1 == 2 {
        for i in 0..16 {
            filter_l2(d, off + i * along, across, e.alpha, e.beta);
        }
    } else {
        if bs1 != 0 {
            for i in 0..8 {
                filter_l1(d, off + i * along, across, e.alpha, e.beta, e.tc);
            }
        }
        if bs2 != 0 {
            for i in 8..16 {
                filter_l1(d, off + i * along, across, e.alpha, e.beta, e.tc);
            }
        }
    }
}

/// A chroma edge of 8 samples.
pub(crate) fn filter_chroma(d: &mut [u8], off: usize, stride: usize, vertical: bool, e: Edge, bs1: u8, bs2: u8) {
    let (along, across) = if vertical { (stride, 1) } else { (1, stride) };
    if bs1 == 2 {
        for i in 0..8 {
            filter_c2(d, off + i * along, across, e.alpha, e.beta);
        }
    } else {
        if bs1 != 0 {
            for i in 0..4 {
                filter_c1(d, off + i * along, across, e.alpha, e.beta, e.tc);
            }
        }
        if bs2 != 0 {
            for i in 4..8 {
                filter_c1(d, off + i * along, across, e.alpha, e.beta, e.tc);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// intra prediction (8×8 blocks; `top[0]` is the top-left sample, `left[0]` too)

pub(crate) const INTRA_L_LP: i32 = 2;

fn lowpass(a: &[u8; 18], i: usize) -> i32 {
    (i32::from(a[i - 1]) + 2 * i32::from(a[i]) + i32::from(a[i + 1]) + 2) >> 2
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pred {
    Vert,
    Horiz,
    Lp,
    DownLeft,
    DownRight,
    LpLeft,
    LpTop,
    Dc128,
    Plane,
}

/// Luma modes 0–7 (`intra_pred_l`).
const LUMA: [Pred; 8] = [Pred::Vert, Pred::Horiz, Pred::Lp, Pred::DownLeft, Pred::DownRight, Pred::LpLeft, Pred::LpTop, Pred::Dc128];
/// Chroma modes 0–6 (`intra_pred_c`).
const CHROMA: [Pred; 7] = [Pred::Lp, Pred::Horiz, Pred::Vert, Pred::Plane, Pred::LpLeft, Pred::LpTop, Pred::Dc128];

pub(crate) fn intra_luma(mode: i32, d: &mut [u8], off: usize, stride: usize, top: &[u8; 18], left: &[u8; 18]) {
    let p = LUMA.get(usize::try_from(mode).unwrap_or(0)).copied().unwrap_or(Pred::Vert);
    intra(p, d, off, stride, top, left);
}

pub(crate) fn intra_chroma(mode: u32, d: &mut [u8], off: usize, stride: usize, top: &[u8; 18], left: &[u8; 18]) {
    let p = CHROMA.get(mode as usize).copied().unwrap_or(Pred::Lp);
    intra(p, d, off, stride, top, left);
}

fn intra(p: Pred, d: &mut [u8], off: usize, stride: usize, top: &[u8; 18], left: &[u8; 18]) {
    let (mut ih, mut iv) = (0, 0);
    if p == Pred::Plane {
        for x in 0..4 {
            ih += (x as i32 + 1) * (i32::from(top[5 + x]) - i32::from(top[3 - x]));
            iv += (x as i32 + 1) * (i32::from(left[5 + x]) - i32::from(left[3 - x]));
        }
    }
    let ia = (i32::from(top[8]) + i32::from(left[8])) << 4;
    let ih = (17 * ih + 16) >> 5;
    let iv = (17 * iv + 16) >> 5;
    for y in 0..8 {
        for x in 0..8 {
            let v = match p {
                Pred::Vert => top[x + 1],
                Pred::Horiz => left[y + 1],
                Pred::Dc128 => 128,
                Pred::Plane => clip8((ia + (x as i32 - 3) * ih + (y as i32 - 3) * iv + 16) >> 5),
                Pred::Lp => ((lowpass(top, x + 1) + lowpass(left, y + 1)) >> 1) as u8,
                Pred::DownLeft => ((lowpass(top, x + y + 2) + lowpass(left, x + y + 2)) >> 1) as u8,
                Pred::DownRight => {
                    if x == y {
                        ((i32::from(left[1]) + 2 * i32::from(top[0]) + i32::from(top[1]) + 2) >> 2) as u8
                    } else if x > y {
                        lowpass(top, x - y) as u8
                    } else {
                        lowpass(left, y - x) as u8
                    }
                }
                Pred::LpLeft => lowpass(left, y + 1) as u8,
                Pred::LpTop => lowpass(top, x + 1) as u8,
            };
            if let Some(s) = d.get_mut(off + y * stride + x) {
                *s = v;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// motion compensation

/// A plane to predict from, read with coordinates clamped into it (FFmpeg reads the picture
/// directly inside and through `emulated_edge_mc` near the edges: the same samples).
pub(crate) struct RefPlane<'a> {
    pub data: &'a [u8],
    pub stride: usize,
    pub width: i32,
    pub height: i32,
}

impl RefPlane<'_> {
    fn at(&self, x: i32, y: i32) -> i32 {
        let x = x.clamp(0, self.width - 1) as usize;
        let y = y.clamp(0, self.height - 1) as usize;
        i32::from(self.data.get(y * self.stride + x).copied().unwrap_or(0))
    }
}

const HPEL: [i32; 6] = [0, -1, 5, 5, -1, 0];
const QPEL_L: [i32; 6] = [-1, -2, 96, 42, -7, 0];
const QPEL_R: [i32; 6] = [0, -7, 42, 96, -2, -1];

fn round_shift(v: i32, shift: u32) -> u8 {
    clip8((v + (1 << (shift - 1))) >> shift)
}

/// Quarter-sample luma prediction of a `size`×`size` block at full-sample position (`fx`,
/// `fy`) with sub-sample phase `xy` (`(mx & 3) + 4 * (my & 3)`); `avg` averages into `dst`.
pub(crate) fn luma_mc(src: &RefPlane<'_>, fx: i32, fy: i32, xy: u32, size: usize, dst: &mut [u8], off: usize, stride: usize, avg: bool) {
    // the block and 2 / 3 samples around it
    let w = size + 5;
    let mut s = vec![0i32; w * w];
    for (i, v) in s.iter_mut().enumerate() {
        *v = src.at(fx + (i % w) as i32 - 2, fy + (i / w) as i32 - 2);
    }
    let g = |i: usize| s.get(i).copied().unwrap_or(0);
    let px = |x: usize, y: usize| g((y + 2) * w + x + 2);
    let h = |t: &[i32; 6], x: usize, y: usize| hrow(&s, w, t, x, y + 2);
    let v = |t: &[i32; 6], x: usize, y: usize| (0..6).map(|k| t[k] * g((y + k) * w + x + 2)).sum::<i32>();
    // horizontal pass on source rows -2..=3 around the output row, kept as 16-bit values
    // (`int16_t temp[]`), then the vertical pass
    let hv = |th: &[i32; 6], tv: &[i32; 6], x: usize, y: usize| (0..6).map(|k| tv[k] * i32::from(hrow(&s, w, th, x, y + k) as i16)).sum::<i32>();
    for y in 0..size {
        for x in 0..size {
            let p = match xy {
                0 => px(x, y) as u8,
                1 => round_shift(h(&QPEL_L, x, y), 7),
                2 => round_shift(h(&HPEL, x, y), 3),
                3 => round_shift(h(&QPEL_R, x, y), 7),
                4 => round_shift(v(&QPEL_L, x, y), 7),
                8 => round_shift(v(&HPEL, x, y), 3),
                12 => round_shift(v(&QPEL_R, x, y), 7),
                10 => round_shift(hv(&HPEL, &HPEL, x, y), 6),
                6 => round_shift(hv(&HPEL, &QPEL_L, x, y), 10),
                9 => round_shift(hv(&QPEL_L, &HPEL, x, y), 10),
                11 => round_shift(hv(&QPEL_R, &HPEL, x, y), 10),
                14 => round_shift(hv(&HPEL, &QPEL_R, x, y), 10),
                5 => round_shift(hv(&HPEL, &HPEL, x, y) + 64 * px(x, y), 7),
                13 => round_shift(hv(&HPEL, &HPEL, x, y) + 64 * px(x, y + 1), 7),
                7 => round_shift(hv(&HPEL, &HPEL, x, y) + 64 * px(x + 1, y), 7),
                _ => round_shift(hv(&HPEL, &HPEL, x, y) + 64 * px(x + 1, y + 1), 7),
            };
            if let Some(d) = dst.get_mut(off + y * stride + x) {
                *d = if avg { ((u16::from(*d) + u16::from(p) + 1) >> 1) as u8 } else { p };
            }
        }
    }
}

/// The horizontal filter `t` at block column `x` on source row `row` (row 0 is two rows above
/// the block).
fn hrow(s: &[i32], w: usize, t: &[i32; 6], x: usize, row: usize) -> i32 {
    (0..6).map(|k| t[k] * s.get(row * w + x + k).copied().unwrap_or(0)).sum()
}

/// Eighth-sample chroma prediction (`put/avg_h264_chroma_mc`) of a `w`×`h` block.
pub(crate) fn chroma_mc(src: &RefPlane<'_>, cx: i32, cy: i32, fx: i32, fy: i32, w: usize, h: usize, dst: &mut [u8], off: usize, stride: usize, avg: bool) {
    let a = (8 - fx) * (8 - fy);
    let b = fx * (8 - fy);
    let c = (8 - fx) * fy;
    let d = fx * fy;
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = (cx + x as i32, cy + y as i32);
            let v = (a * src.at(sx, sy) + b * src.at(sx + 1, sy) + c * src.at(sx, sy + 1) + d * src.at(sx + 1, sy + 1) + 32) >> 6;
            if let Some(p) = dst.get_mut(off + y * stride + x) {
                *p = if avg { ((i32::from(*p) + v + 1) >> 1) as u8 } else { v as u8 };
            }
        }
    }
}
