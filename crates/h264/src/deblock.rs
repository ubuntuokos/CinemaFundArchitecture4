//! Deblocking filter (8.7) for frames, MBAFF frames and field pictures, 4:2:0 / 4:2:2 / 4:4:4,
//! 8 to 14 bits.

use crate::picture::{MbKind, MbState, Sample};
use crate::slicedec::{PicState, SliceInfo};
use crate::tables::{ALPHA, BETA, TC0};

/// Reference picture identity + mvs of one 4x4 block, for bS = 1 decisions.
#[derive(Clone, Copy)]
struct Motion {
    ids: [u32; 2],
    mv: [[i16; 2]; 2],
    count: u8,
}

/// Referenced picture ids per 8x8 block and list (u32::MAX = list unused).
#[inline]
fn ref_ids8(st: &MbState, sl: &SliceInfo) -> [[u32; 2]; 4] {
    std::array::from_fn(|b8| {
        std::array::from_fn(|l| {
            let r = st.ref_idx[l][b8];
            if r >= 0 { sl.ref_key(st, l, r) } else { u32::MAX }
        })
    })
}

#[inline(always)]
fn block_motion(st: &MbState, ids8: &[[u32; 2]; 4], raster: usize) -> Motion {
    let b8 = (raster >> 3) * 2 + ((raster & 3) >> 1);
    let ids = ids8[b8];
    let count = (ids[0] != u32::MAX) as u8 + (ids[1] != u32::MAX) as u8;
    Motion { ids, mv: [st.mv[0][raster], st.mv[1][raster]], count }
}

#[inline(always)]
fn mv_far(a: [i16; 2], b: [i16; 2], lim: i32) -> bool {
    (a[0] as i32 - b[0] as i32).abs() >= 4 || (a[1] as i32 - b[1] as i32).abs() >= lim
}

#[inline]
/// bS 0 or 1 from the motion of two blocks; `lim`: the vertical motion vector difference that
/// counts (4 quarter frame samples, 2 in field units).
fn motion_bs(p: &Motion, q: &Motion, lim: i32) -> u8 {
    if p.ids == q.ids && p.mv == q.mv {
        return 0;
    }
    if p.count != q.count {
        return 1;
    }
    if p.count == 1 {
        let (pi, pm) = if p.ids[0] != u32::MAX { (p.ids[0], p.mv[0]) } else { (p.ids[1], p.mv[1]) };
        let (qi, qm) = if q.ids[0] != u32::MAX { (q.ids[0], q.mv[0]) } else { (q.ids[1], q.mv[1]) };
        if pi != qi {
            return 1;
        }
        return mv_far(pm, qm, lim) as u8;
    }
    if p.count == 0 {
        return 0;
    }
    // two motion vectors each
    let same_set = (p.ids[0] == q.ids[0] && p.ids[1] == q.ids[1]) || (p.ids[0] == q.ids[1] && p.ids[1] == q.ids[0]);
    if !same_set {
        return 1;
    }
    if p.ids[0] != p.ids[1] {
        // two different reference pictures: compare mvs referring to the same picture
        if p.ids[0] == q.ids[0] {
            (mv_far(p.mv[0], q.mv[0], lim) || mv_far(p.mv[1], q.mv[1], lim)) as u8
        } else {
            (mv_far(p.mv[0], q.mv[1], lim) || mv_far(p.mv[1], q.mv[0], lim)) as u8
        }
    } else {
        // both mvs refer to the same picture
        ((mv_far(p.mv[0], q.mv[0], lim) || mv_far(p.mv[1], q.mv[1], lim)) && (mv_far(p.mv[0], q.mv[1], lim) || mv_far(p.mv[1], q.mv[0], lim))) as u8
    }
}

#[cfg(test)]
fn clip3(lo: i32, hi: i32, v: i32) -> i32 {
    v.clamp(lo, hi)
}

/// Filter one set of eight samples p3 p2 p1 p0 | q0 q1 q2 q3 across an edge (8.7.2.3 / 8.7.2.4):
/// the per-line reference for the lane filter.
#[cfg(test)]
fn filter8(v: &mut [u8; 8], bs: u8, alpha: i32, beta: i32, tc0: i32, chroma: bool) {
    let p0 = v[3] as i32;
    let q0 = v[4] as i32;
    let p1 = v[2] as i32;
    let q1 = v[5] as i32;
    if (p0 - q0).abs() >= alpha || (p1 - p0).abs() >= beta || (q1 - q0).abs() >= beta {
        return;
    }
    if chroma {
        if bs < 4 {
            let tc = tc0 + 1;
            let delta = clip3(-tc, tc, (((q0 - p0) << 2) + (p1 - q1) + 4) >> 3);
            v[3] = (p0 + delta).clamp(0, 255) as u8;
            v[4] = (q0 - delta).clamp(0, 255) as u8;
        } else {
            v[3] = ((2 * p1 + p0 + q1 + 2) >> 2) as u8;
            v[4] = ((2 * q1 + q0 + p1 + 2) >> 2) as u8;
        }
        return;
    }
    let p2 = v[1] as i32;
    let q2 = v[6] as i32;
    let ap = (p2 - p0).abs();
    let aq = (q2 - q0).abs();
    if bs < 4 {
        let tc = tc0 + (ap < beta) as i32 + (aq < beta) as i32;
        let delta = clip3(-tc, tc, (((q0 - p0) << 2) + (p1 - q1) + 4) >> 3);
        v[3] = (p0 + delta).clamp(0, 255) as u8;
        v[4] = (q0 - delta).clamp(0, 255) as u8;
        if ap < beta {
            v[2] = (p1 + clip3(-tc0, tc0, (p2 + ((p0 + q0 + 1) >> 1) - (p1 << 1)) >> 1)) as u8;
        }
        if aq < beta {
            v[5] = (q1 + clip3(-tc0, tc0, (q2 + ((p0 + q0 + 1) >> 1) - (q1 << 1)) >> 1)) as u8;
        }
    } else {
        let strong = (p0 - q0).abs() < ((alpha >> 2) + 2);
        if ap < beta && strong {
            let p3 = v[0] as i32;
            v[3] = ((p2 + 2 * p1 + 2 * p0 + 2 * q0 + q1 + 4) >> 3) as u8;
            v[2] = ((p2 + p1 + p0 + q0 + 2) >> 2) as u8;
            v[1] = ((2 * p3 + 3 * p2 + p1 + p0 + q0 + 4) >> 3) as u8;
        } else {
            v[3] = ((2 * p1 + p0 + q1 + 2) >> 2) as u8;
        }
        if aq < beta && strong {
            let q3 = v[7] as i32;
            v[4] = ((p1 + 2 * p0 + 2 * q0 + 2 * q1 + q2 + 4) >> 3) as u8;
            v[5] = ((p0 + q0 + q1 + q2 + 2) >> 2) as u8;
            v[6] = ((2 * q3 + 3 * q2 + q1 + q0 + p0 + 4) >> 3) as u8;
        } else {
            v[4] = ((2 * q1 + q0 + p1 + 2) >> 2) as u8;
        }
    }
}

/// Edge filter parameters for `n` lines; `bs[i]` / `tc0[i]` apply to lines `i * n / 4 .. (i + 1) * n / 4`.
pub struct EdgeParams {
    bs: [u8; 4],
    tc0: [i32; 4],
    alpha: i32,
    beta: i32,
}

/// Lane arithmetic of the edge filter: `i16` lanes are enough for 8-bit samples (and twice as many
/// fit a SIMD register), 9 to 14 bits overflow them in the strong filter's weighted sums and need
/// `i32`. The two instantiations below are generated from one body by [`lane_filter`].
pub trait Lanes: Sample {
    /// Lane accumulator type.
    type L: Copy + Default;
    fn to_lane(self) -> Self::L;
    fn from_lane(l: Self::L) -> Self;
    /// Filter one edge of `N` lines (see [`lane_filter`]).
    fn filter_edge<const N: usize>(s: &mut [[Self::L; N]; 8], ep: &EdgeParams, chroma: bool, max: i32);
}

/// Filter one edge for `N` lines at once (lane `i` = line `i` across the edge). `s[k][i]` holds
/// sample p3 p2 p1 p0 q0 q1 q2 q3 (k = 0..8) of line `i`. Branch-free per lane, so the loops
/// vectorise; bit-exact with [`filter8`] applied to every line (8.7.2.3 / 8.7.2.4).
macro_rules! lane_filter {
    ($name:ident, $t:ty) => {
        #[inline(always)]
        fn $name<const N: usize>(s: &mut [[$t; N]; 8], ep: &EdgeParams, chroma: bool, max: i32) {
            let per = N / 4;
            let (alpha, beta, max) = (ep.alpha as $t, ep.beta as $t, max as $t);
            let mut bs_on = [false; N];
            let mut tc0 = [0 as $t; N];
            for k in 0..4 {
                for i in k * per..(k + 1) * per {
                    bs_on[i] = ep.bs[k] != 0;
                    tc0[i] = ep.tc0[k] as $t;
                }
            }
            let [p3, p2, p1, p0, q0, q1, q2, q3] = *s;
            // Masks are 0 / -1 per lane, selects are bitwise, and clipping to per-lane bounds uses
            // min / max (`clamp` would assert its bounds per lane), so every loop below vectorises.
            let lt = |x: $t, y: $t| -((x < y) as $t);
            let sel = |m: $t, a: $t, b: $t| (a & m) | (b & !m);
            let mut on = [0 as $t; N];
            let mut any = 0 as $t;
            for i in 0..N {
                on[i] = -(bs_on[i] as $t) & lt((p0[i] - q0[i]).abs(), alpha) & lt((p1[i] - p0[i]).abs(), beta) & lt((q1[i] - q0[i]).abs(), beta);
                any |= on[i];
            }
            if any == 0 {
                return;
            }
            // bS 4 is only ever assigned to all four segments of an edge
            let strong_edge = ep.bs[0] >= 4;
            if chroma {
                for i in 0..N {
                    let (np0, nq0) = if strong_edge {
                        ((2 * p1[i] + p0[i] + q1[i] + 2) >> 2, (2 * q1[i] + q0[i] + p1[i] + 2) >> 2)
                    } else {
                        let tc = tc0[i] + 1;
                        let delta = ((((q0[i] - p0[i]) << 2) + (p1[i] - q1[i]) + 4) >> 3).max(-tc).min(tc);
                        ((p0[i] + delta).max(0).min(max), (q0[i] - delta).max(0).min(max))
                    };
                    s[3][i] = sel(on[i], np0, p0[i]);
                    s[4][i] = sel(on[i], nq0, q0[i]);
                }
                return;
            }
            if strong_edge {
                let lim = (alpha >> 2) + 2;
                for i in 0..N {
                    let (a, b, c, d, e, f, g, h) = (p3[i], p2[i], p1[i], p0[i], q0[i], q1[i], q2[i], q3[i]);
                    let strong = lt((d - e).abs(), lim);
                    let sp = on[i] & lt((b - d).abs(), beta) & strong;
                    let sq = on[i] & lt((g - e).abs(), beta) & strong;
                    let wp0 = (2 * c + d + f + 2) >> 2;
                    let wq0 = (2 * f + e + c + 2) >> 2;
                    let sp0 = (b + 2 * c + 2 * d + 2 * e + f + 4) >> 3;
                    let sq0 = (c + 2 * d + 2 * e + 2 * f + g + 4) >> 3;
                    s[3][i] = sel(sp, sp0, sel(on[i], wp0, d));
                    s[2][i] = sel(sp, (b + c + d + e + 2) >> 2, c);
                    s[1][i] = sel(sp, (2 * a + 3 * b + c + d + e + 4) >> 3, b);
                    s[4][i] = sel(sq, sq0, sel(on[i], wq0, e));
                    s[5][i] = sel(sq, (d + e + f + g + 2) >> 2, f);
                    s[6][i] = sel(sq, (2 * h + 3 * g + f + e + d + 4) >> 3, g);
                }
            } else {
                for i in 0..N {
                    let ap = lt((p2[i] - p0[i]).abs(), beta);
                    let aq = lt((q2[i] - q0[i]).abs(), beta);
                    let t0 = tc0[i];
                    let tc = t0 - ap - aq;
                    let delta = ((((q0[i] - p0[i]) << 2) + (p1[i] - q1[i]) + 4) >> 3).max(-tc).min(tc);
                    let avg = (p0[i] + q0[i] + 1) >> 1;
                    let np1 = p1[i] + ((p2[i] + avg - (p1[i] << 1)) >> 1).max(-t0).min(t0);
                    let nq1 = q1[i] + ((q2[i] + avg - (q1[i] << 1)) >> 1).max(-t0).min(t0);
                    s[3][i] = sel(on[i], (p0[i] + delta).max(0).min(max), p0[i]);
                    s[4][i] = sel(on[i], (q0[i] - delta).max(0).min(max), q0[i]);
                    s[2][i] = sel(on[i] & ap, np1, p1[i]);
                    s[5][i] = sel(on[i] & aq, nq1, q1[i]);
                }
            }
        }
    };
}

lane_filter!(filter_lanes_i16, i16);
lane_filter!(filter_lanes_i32, i32);

impl Lanes for u8 {
    type L = i16;
    #[inline(always)]
    fn to_lane(self) -> i16 {
        self as i16
    }
    #[inline(always)]
    fn from_lane(l: i16) -> u8 {
        l as u8
    }
    #[inline(always)]
    fn filter_edge<const N: usize>(s: &mut [[i16; N]; 8], ep: &EdgeParams, chroma: bool, max: i32) {
        filter_lanes_i16(s, ep, chroma, max);
    }
}

impl Lanes for u16 {
    type L = i32;
    #[inline(always)]
    fn to_lane(self) -> i32 {
        self as i32
    }
    #[inline(always)]
    fn from_lane(l: i32) -> u16 {
        l as u16
    }
    #[inline(always)]
    fn filter_edge<const N: usize>(s: &mut [[i32; N]; 8], ep: &EdgeParams, chroma: bool, max: i32) {
        filter_lanes_i32(s, ep, chroma, max);
    }
}

/// Filter a vertical edge whose q0 column is at `x`, for `N` lines starting at row `y`.
#[inline(always)]
fn filter_vertical<S: Lanes, const N: usize>(pix: &mut [S], stride: usize, x: usize, y: usize, ep: &EdgeParams, chroma: bool, max: i32) {
    // The eight samples of each line are contiguous: a fixed-size chunk loads them without a
    // bounds check per sample.
    let mut s = [[S::L::default(); N]; 8];
    for i in 0..N {
        let o = (y + i) * stride + x - 4;
        let Some(line) = pix.get(o..).and_then(|p| p.first_chunk::<8>()) else {
            return;
        };
        for k in 0..8 {
            s[k][i] = line[k].to_lane();
        }
    }
    S::filter_edge(&mut s, ep, chroma, max);
    for i in 0..N {
        let o = (y + i) * stride + x - 4;
        let Some(line) = pix.get_mut(o..).and_then(|p| p.first_chunk_mut::<8>()) else {
            return;
        };
        for k in 1..7 {
            line[k] = S::from_lane(s[k][i]);
        }
    }
}

/// Filter a horizontal edge whose q0 row is `y`, for `N` columns starting at `x`.
#[inline(always)]
fn filter_horizontal<S: Lanes, const N: usize>(pix: &mut [S], stride: usize, x: usize, y: usize, ep: &EdgeParams, chroma: bool, max: i32) {
    let base = (y - 4) * stride + x;
    let mut s = [[S::L::default(); N]; 8];
    for (k, row) in s.iter_mut().enumerate() {
        let Some(src) = pix.get(base + k * stride..).and_then(|p| p.first_chunk::<N>()) else {
            return;
        };
        for i in 0..N {
            row[i] = src[i].to_lane();
        }
    }
    S::filter_edge(&mut s, ep, chroma, max);
    let (k0, k1) = if chroma { (3, 5) } else { (1, 7) };
    for (k, row) in s.iter().enumerate().take(k1).skip(k0) {
        let Some(dst) = pix.get_mut(base + k * stride..).and_then(|p| p.first_chunk_mut::<N>()) else {
            return;
        };
        for i in 0..N {
            dst[i] = S::from_lane(row[i]);
        }
    }
}

/// Where the samples of a macroblock are in a plane: `plane[off + (y0 + y) * stride + x0 + x]`
/// (field macroblocks of MBAFF frames address their field's lines).
#[derive(Clone, Copy)]
struct View {
    off: usize,
    stride: usize,
    x0: usize,
    y0: usize,
}

/// The macroblock(s) across the left or top macroblock edge.
#[derive(Clone, Copy)]
enum Across {
    None,
    /// One macroblock; `mixed`: one of the two is a field and the other a frame macroblock
    /// (mixedModeEdgeFlag).
    One(usize, bool),
    /// MBAFF left edge between a frame and a field macroblock pair (whose top macroblock is
    /// given): each line has its own neighbour.
    MixedLeft(usize),
    /// MBAFF top edge of a frame macroblock under a field pair (whose top macroblock is given):
    /// filtered once per field (8.7.1).
    FieldPairAbove(usize),
}

/// Edge filter parameters (alpha, beta, tC0 per segment) of an edge between macroblocks with
/// QPs `qpp` and `qpq` (8.7.2.2), scaled to the bit depth.
#[inline]
fn edge_params(qpp: i32, qpq: i32, b: [u8; 4], alpha_off: i32, beta_off: i32, scale: i32) -> EdgeParams {
    let qpav = (qpp + qpq + 1) >> 1;
    let index_a = (qpav + alpha_off).clamp(0, 51) as usize;
    let index_b = (qpav + beta_off).clamp(0, 51) as usize;
    let tc = |b: u8| if (1..4).contains(&b) { TC0[index_a][b as usize - 1] as i32 * scale } else { 0 };
    EdgeParams { bs: b, tc0: [tc(b[0]), tc(b[1]), tc(b[2]), tc(b[3])], alpha: ALPHA[index_a] as i32 * scale, beta: BETA[index_b] as i32 * scale }
}

#[inline]
fn qp_of(st: &MbState) -> i32 {
    if st.kind == MbKind::IPcm { 0 } else { st.qp as i32 }
}

/// Deblock one macroblock (macroblocks must be processed in order of increasing addresses).
#[allow(clippy::needless_range_loop)]
pub fn deblock_mb<S: Lanes>(pic: &mut PicState<S>, addr: usize) {
    let mb_w = pic.mb_w.max(1);
    let Some(q) = pic.mbs.get(addr) else { return };
    if q.slice_num == u32::MAX {
        return;
    }
    let Some(sq) = pic.slices.get(q.slice_num as usize) else { return };
    if sq.disable_deblocking_filter_idc == 1 {
        return;
    }
    let usable = |n: usize| -> Option<usize> {
        let st = pic.mbs.get(n)?;
        if st.slice_num == u32::MAX || (sq.disable_deblocking_filter_idc == 2 && st.slice_num != q.slice_num) {
            return None;
        }
        Some(n)
    };
    let (mbaff, field_pic) = (pic.mbaff, pic.field.is_some());
    let (width, cwidth) = (pic.planes.width, pic.planes.cwidth);
    let chroma = pic.planes.chroma;
    let (cw, ch) = (chroma.mb_w(), chroma.mb_h());
    let one = |n: Option<usize>, mixed: bool| n.map_or(Across::None, |n| Across::One(n, mixed));
    let (yv, cv, left, top) = if mbaff {
        let (pair, b) = (addr / 2, addr & 1);
        let (mx, py) = (pair % mb_w, pair / mb_w);
        let (yv, cv) = if q.field {
            (View { off: b * width, stride: 2 * width, x0: mx * 16, y0: py * 16 }, View { off: b * cwidth, stride: 2 * cwidth, x0: mx * cw, y0: py * ch })
        } else {
            let r = 2 * py + b;
            (View { off: 0, stride: width, x0: mx * 16, y0: r * 16 }, View { off: 0, stride: cwidth, x0: mx * cw, y0: r * ch })
        };
        let left = match (mx > 0).then(|| usable(2 * (pair - 1))).flatten() {
            None => Across::None,
            Some(lt) if pic.mbs[lt].field != q.field => Across::MixedLeft(lt),
            Some(lt) => one(usable(lt + b), false),
        };
        let top = if !q.field && b == 1 {
            one(usable(addr - 1), false)
        } else if py == 0 {
            Across::None
        } else {
            match usable(2 * (pair - mb_w)) {
                None => Across::None,
                Some(at) => match (q.field, pic.mbs[at].field) {
                    (false, true) => Across::FieldPairAbove(at),
                    (false, false) => one(usable(at + 1), false),
                    (true, true) => one(usable(at + b), false),
                    (true, false) => one(usable(at + 1), true),
                },
            }
        };
        (yv, cv, left, top)
    } else {
        let (mx, my) = (addr % mb_w, addr / mb_w);
        (
            View { off: 0, stride: width, x0: mx * 16, y0: my * 16 },
            View { off: 0, stride: cwidth, x0: mx * cw, y0: my * ch },
            one((mx > 0).then(|| usable(addr - 1)).flatten(), false),
            one((my > 0).then(|| usable(addr - mb_w)).flatten(), false),
        )
    };
    let alpha_off = sq.alpha_offset;
    let beta_off = sq.beta_offset;
    let t8 = q.transform_8x8;
    // vertical motion vector differences of field macroblocks count in field units (8.7.2.1)
    let lim = if q.field { 2 } else { 4 };
    // bS[dir][edge][segment]; dir 0 = vertical edges (x), 1 = horizontal edges (y)
    let mut bs = [[[0u8; 4]; 4]; 2];
    let qids = ref_ids8(q, sq);
    let q_uniform = !q.kind.is_intra()
        && (0..2).all(|l| {
            let r = q.ref_idx[l];
            r[1] == r[0] && r[2] == r[0] && r[3] == r[0] && q.mv[l].iter().all(|m| *m == q.mv[l][0])
        });
    let qm: [Motion; 16] = std::array::from_fn(|r| block_motion(q, &qids, r));
    for dir in 0..2 {
        if let Across::One(n, mixed) = if dir == 0 { left } else { top } {
            let p = &pic.mbs[n];
            if p.kind.is_intra() || q.kind.is_intra() {
                // bS 4 across macroblock edges between frame macroblocks, and across vertical
                // edges of MBAFF frames and field pictures
                let strong = (!p.field && !q.field) || ((mbaff || field_pic) && dir == 0);
                bs[dir][0] = [if strong { 4 } else { 3 }; 4];
            } else {
                let pids = pic.slices.get(p.slice_num as usize).map_or([[u32::MAX - 1; 2]; 4], |s| ref_ids8(p, s));
                for k in 0..4 {
                    let (rq, rp) = if dir == 0 { (k * 4, k * 4 + 3) } else { (k, 12 + k) };
                    bs[dir][0][k] = if (p.nz_mask >> rp) & 1 != 0 || (q.nz_mask >> rq) & 1 != 0 {
                        2
                    } else if mixed {
                        1
                    } else {
                        motion_bs(&block_motion(p, &pids, rp), &qm[rq], lim)
                    };
                }
            }
        }
        if q.kind.is_intra() {
            for e in 1..4 {
                if !(t8 && e % 2 == 1) || chroma.0 == 2 {
                    bs[dir][e] = [3; 4];
                }
            }
            continue;
        }
        if q_uniform && q.nz_mask == 0 {
            // one motion for the whole MB and no coefficients: all internal edges have bS 0
            continue;
        }
        for e in 1..4 {
            // edges inside an 8x8 transform block are only needed for 4:2:2 chroma
            if t8 && e % 2 == 1 && chroma.0 != 2 {
                continue;
            }
            for k in 0..4 {
                let (rq, rp) = if dir == 0 { (k * 4 + e, k * 4 + e - 1) } else { (e * 4 + k, e * 4 + k - 4) };
                bs[dir][e][k] = if (q.nz_mask >> rp) & 1 != 0 || (q.nz_mask >> rq) & 1 != 0 { 2 } else { motion_bs(&qm[rp], &qm[rq], lim) };
            }
        }
    }
    let q_qp = qp_of(q);
    let q_qpc = q.qpc;
    let across_qp = |a: Across| if let Across::One(n, _) = a { Some((qp_of(&pic.mbs[n]), pic.mbs[n].qpc)) } else { None };
    let p_qp = [across_qp(left), across_qp(top)];
    // alpha, beta and tC0 are the table values scaled by the bit depth (8.7.2.2)
    let scale = 1i32 << pic.bit_depth.saturating_sub(8);
    let max = (1i32 << pic.bit_depth) - 1;
    let params = |qpp: i32, qpq: i32, b: [u8; 4]| edge_params(qpp, qpq, b, alpha_off, beta_off, scale);
    let q_copy = *q;
    // the special macroblock edges of MBAFF frames first (8.7: left edge, then top edge)
    if let Across::MixedLeft(lt) = left {
        mixed_left_edge(pic, &q_copy, lt, addr & 1, yv, cv, (alpha_off, beta_off, scale, max));
    }
    // luma: the 8x8 transform leaves the edges inside its blocks unfiltered (the bS of those edges
    // is still derived above: 4:2:2 chroma uses it)
    for dir in 0..2 {
        if dir == 1
            && let Across::FieldPairAbove(at) = top
        {
            field_pair_top_edge(pic, &q_copy, at, 0, yv, (alpha_off, beta_off, scale, max));
        }
        for e in 0..4 {
            if bs[dir][e] == [0; 4] || (t8 && e % 2 == 1) {
                continue;
            }
            let qpp = if e == 0 { p_qp[dir].map(|p| p.0).unwrap_or(q_qp) } else { q_qp };
            let ep = params(qpp, q_qp, bs[dir][e]);
            if ep.alpha == 0 || ep.beta == 0 {
                // filterSamplesFlag needs |p0 - q0| < alpha and |p1 - p0| < beta (8.7.2.2): no
                // sample of the edge changes (low QPs)
                continue;
            }
            let Some(y) = pic.planes.y.get_mut(yv.off..) else { return };
            if dir == 0 {
                filter_vertical::<S, 16>(y, yv.stride, yv.x0 + e * 4, yv.y0, &ep, false, max);
            } else {
                filter_horizontal::<S, 16>(y, yv.stride, yv.x0, yv.y0 + e * 4, &ep, false, max);
            }
        }
    }
    // chroma: the bS of the luma edge through the luma sample at (SubWidthC * x, SubHeightC * y)
    // (8.7.2). 4:4:4 filters the luma edges with the luma filter; 4:2:0 / 4:2:2 filter the edges
    // every four chroma samples with the chroma filter.
    if !chroma.coded() {
        return;
    }
    let (sx, sy) = (chroma.sx(), chroma.sy());
    for c in 0..2 {
        for dir in 0..2 {
            if dir == 1
                && let Across::FieldPairAbove(at) = top
            {
                field_pair_top_edge(pic, &q_copy, at, 1 + c, cv, (alpha_off, beta_off, scale, max));
            }
            let (shift, n) = if dir == 0 { (sx, cw / 4) } else { (sy, ch / 4) };
            for ce in 0..n {
                // luma edge index of chroma edge ce
                let e = (ce << shift) & 3;
                if bs[dir][e] == [0; 4] || (chroma.0 == 3 && t8 && e % 2 == 1) {
                    continue;
                }
                let qpp = if ce == 0 { p_qp[dir].map(|p| p.1[c]).unwrap_or(q_qpc[c]) } else { q_qpc[c] } as i32;
                let ep = params(qpp, q_qpc[c] as i32, bs[dir][e]);
                if ep.alpha == 0 || ep.beta == 0 {
                    continue;
                }
                let plane = if c == 0 { &mut pic.planes.cb } else { &mut pic.planes.cr };
                let Some(plane) = plane.get_mut(cv.off..) else { return };
                let (cs, x0, y0) = (cv.stride, cv.x0, cv.y0);
                let luma_style = chroma.0 == 3;
                // the segments of a chroma edge follow the luma segments it spans
                match (dir, luma_style, ch) {
                    (_, true, _) => {
                        if dir == 0 {
                            filter_vertical::<S, 16>(plane, cs, x0 + ce * 4, y0, &ep, false, max);
                        } else {
                            filter_horizontal::<S, 16>(plane, cs, x0, y0 + ce * 4, &ep, false, max);
                        }
                    }
                    (0, false, 16) => filter_vertical::<S, 16>(plane, cs, x0 + ce * 4, y0, &ep, true, max),
                    (0, false, _) => filter_vertical::<S, 8>(plane, cs, x0 + ce * 4, y0, &ep, true, max),
                    _ => filter_horizontal::<S, 8>(plane, cs, x0, y0 + ce * 4, &ep, true, max),
                }
            }
        }
    }
}

/// (alpha offset, beta offset, bit depth scale, largest sample).
type Tune = (i32, i32, i32, i32);

/// The left macroblock edge of an MBAFF macroblock whose left pair (top macroblock `lt`) is of
/// the other kind (frame / field). The edge is filtered in the current macroblock's lines; each
/// line's left samples belong to one macroblock of the left pair, so the lines are filtered in
/// two sets of eight, one per left macroblock, with its QP and per two lines its bS (vertical
/// MBAFF edges: 4 for intra, else 2 with coefficients, else 1 as the edge is mixed).
fn mixed_left_edge<S: Lanes>(pic: &mut PicState<S>, q: &MbState, lt: usize, bottom: usize, yv: View, cv: View, tune: Tune) {
    let (alpha_off, beta_off, scale, max) = tune;
    let chroma = pic.planes.chroma;
    // the left macroblock and its line of current line y (in the current macroblock's lines)
    let p_of = |y: usize| -> (usize, usize) {
        if q.field {
            let row = 2 * y + bottom;
            (lt + (row >= 16) as usize, row % 16)
        } else {
            let row = 16 * bottom + y;
            (lt + (row & 1), row >> 1)
        }
    };
    let bs_line = |y: usize| -> u8 {
        let (m, yw) = p_of(y);
        let Some(p) = pic.mbs.get(m) else { return 0 };
        if p.kind.is_intra() || q.kind.is_intra() {
            4
        } else if (p.nz_mask >> ((yw / 4) * 4 + 3)) & 1 != 0 || (q.nz_mask >> ((y / 4) * 4)) & 1 != 0 {
            2
        } else {
            1
        }
    };
    // set j: the current lines next to left macroblock lt + j
    let line = |j: usize, k: usize| if q.field { 8 * j + k } else { j + 2 * k };
    let set_stride = if q.field { 1 } else { 2 };
    for j in 0..2 {
        let Some(p) = pic.mbs.get(lt + j).copied() else { continue };
        let b: [u8; 4] = std::array::from_fn(|i| bs_line(line(j, 2 * i)));
        let ep = edge_params(qp_of(&p), qp_of(q), b, alpha_off, beta_off, scale);
        if ep.alpha == 0 || ep.beta == 0 {
            continue;
        }
        if let Some(y) = pic.planes.y.get_mut(yv.off + (yv.y0 + line(j, 0)) * yv.stride..) {
            filter_vertical::<S, 8>(y, yv.stride * set_stride, yv.x0, 0, &ep, false, max);
        }
    }
    if !chroma.coded() {
        return;
    }
    let ch = chroma.mb_h();
    let sub_h = 1 << chroma.sy();
    // the luma line of chroma line yc in the same field
    let luma_line = |yc: usize| if q.field { yc * sub_h } else { 2 * (yc >> 1) * sub_h + (yc & 1) };
    let n = ch / 2;
    let cline = |j: usize, k: usize| if q.field { n * j + k } else { j + 2 * k };
    for c in 0..2 {
        for j in 0..2 {
            let Some(p) = pic.mbs.get(lt + j).copied() else { continue };
            let per = (n / 4).max(1);
            let b: [u8; 4] = std::array::from_fn(|i| bs_line(luma_line(cline(j, (i * per).min(n - 1)))));
            let ep = edge_params(p.qpc[c] as i32, q.qpc[c] as i32, b, alpha_off, beta_off, scale);
            if ep.alpha == 0 || ep.beta == 0 {
                continue;
            }
            let plane = if c == 0 { &mut pic.planes.cb } else { &mut pic.planes.cr };
            let Some(pl) = plane.get_mut(cv.off + (cv.y0 + cline(j, 0)) * cv.stride..) else { continue };
            let stride = cv.stride * set_stride;
            match (chroma.0, n) {
                (3, _) => filter_vertical::<S, 8>(pl, stride, cv.x0, 0, &ep, false, max),
                (_, 8) => filter_vertical::<S, 8>(pl, stride, cv.x0, 0, &ep, true, max),
                _ => filter_vertical::<S, 4>(pl, stride, cv.x0, 0, &ep, true, max),
            }
        }
    }
}

/// The top macroblock edge of an MBAFF frame macroblock (top of its pair) under a field
/// macroblock pair (top macroblock `at`) in colour component `comp`: filtered in field mode, the
/// current macroblock's lines of each parity against the field macroblock of that parity (8.7.1).
/// The edge is mixed and horizontal: bS 3 for intra, else 2 with coefficients, else 1.
fn field_pair_top_edge<S: Lanes>(pic: &mut PicState<S>, q: &MbState, at: usize, comp: usize, v: View, tune: Tune) {
    let (alpha_off, beta_off, scale, max) = tune;
    let chroma = pic.planes.chroma;
    for f in 0..2 {
        let Some(p) = pic.mbs.get(at + f).copied() else { continue };
        if p.slice_num == u32::MAX {
            continue;
        }
        let b: [u8; 4] = std::array::from_fn(|k| {
            if p.kind.is_intra() || q.kind.is_intra() {
                3
            } else if (p.nz_mask >> (12 + k)) & 1 != 0 || (q.nz_mask >> k) & 1 != 0 {
                2
            } else {
                1
            }
        });
        let ep = if comp == 0 {
            edge_params(qp_of(&p), qp_of(q), b, alpha_off, beta_off, scale)
        } else {
            edge_params(p.qpc[comp - 1] as i32, q.qpc[comp - 1] as i32, b, alpha_off, beta_off, scale)
        };
        if ep.alpha == 0 || ep.beta == 0 {
            continue;
        }
        let (plane, w) = match comp {
            0 => (&mut pic.planes.y, pic.planes.width),
            1 => (&mut pic.planes.cb, pic.planes.cwidth),
            _ => (&mut pic.planes.cr, pic.planes.cwidth),
        };
        // the field's lines: the current macroblock's first line of parity f is field line y0 / 2
        let Some(pl) = plane.get_mut(f * w..) else { continue };
        if comp == 0 || chroma.0 == 3 {
            filter_horizontal::<S, 16>(pl, 2 * w, v.x0, v.y0 / 2, &ep, false, max);
        } else {
            filter_horizontal::<S, 8>(pl, 2 * w, v.x0, v.y0 / 2, &ep, true, max);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vectorised edge filter equals the per-line reference [`filter8`] on random edges
    /// (small differences across the edge so that every filter branch is taken).
    #[test]
    fn lane_filter_matches_per_line_reference() {
        let mut rng = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for iter in 0..20_000 {
            let chroma = iter % 3 == 0;
            let strong = iter % 4 == 1;
            let bs: [u8; 4] = if strong { [4; 4] } else { std::array::from_fn(|_| (next() % 4) as u8) };
            let index_a = (next() % 52) as usize;
            let index_b = (next() % 52) as usize;
            let tc = |b: u8| if (1..4).contains(&b) { TC0[index_a][b as usize - 1] as i32 } else { 0 };
            let ep = EdgeParams { bs, tc0: bs.map(tc), alpha: ALPHA[index_a] as i32, beta: BETA[index_b] as i32 };
            let base = (next() % 256) as i32;
            let spread = 1 + (next() % 24) as i32;
            let mut lines = [[0u8; 8]; 16];
            for l in lines.iter_mut() {
                for v in l.iter_mut() {
                    *v = (base + (next() % (2 * spread as u64 + 1)) as i32 - spread).clamp(0, 255) as u8;
                }
            }
            let n = if chroma { 8 } else { 16 };
            let mut want = lines;
            for (i, l) in want.iter_mut().enumerate().take(n) {
                let k = i / (n / 4);
                if bs[k] != 0 {
                    filter8(l, bs[k], ep.alpha, ep.beta, ep.tc0[k], chroma);
                }
            }
            let got: Vec<[u8; 8]> = if chroma {
                let mut s = [[0i16; 8]; 8];
                for i in 0..8 {
                    for k in 0..8 {
                        s[k][i] = lines[i][k] as i16;
                    }
                }
                filter_lanes_i16(&mut s, &ep, true, 255);
                (0..8).map(|i| std::array::from_fn(|k| s[k][i] as u8)).collect()
            } else {
                let mut s = [[0i16; 16]; 8];
                for i in 0..16 {
                    for k in 0..8 {
                        s[k][i] = lines[i][k] as i16;
                    }
                }
                filter_lanes_i16(&mut s, &ep, false, 255);
                (0..16).map(|i| std::array::from_fn(|k| s[k][i] as u8)).collect()
            };
            assert_eq!(&got[..], &want[..n], "iteration {iter}: bs {bs:?} indexA {index_a} indexB {index_b}");
        }
    }
}

#[cfg(test)]
mod speed {
    use super::*;

    /// Rough timing of the edge filters (`cargo test --release -p cfa4-h264 --lib edge_filter_speed -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn edge_filter_speed() {
        let (w, h) = (1024usize, 256usize);
        let mut pix: Vec<u8> = (0..w * h).map(|i| (128 + ((i * 7919) % 13) as i32 - 6) as u8).collect();
        let eps = [
            EdgeParams { bs: [4; 4], tc0: [0; 4], alpha: 40, beta: 9 },
            EdgeParams { bs: [3; 4], tc0: [2; 4], alpha: 40, beta: 9 },
            EdgeParams { bs: [2, 1, 0, 2], tc0: [1, 1, 0, 1], alpha: 40, beta: 9 },
        ];
        let t0 = std::time::Instant::now();
        let mut n = 0;
        for _ in 0..20 {
            for my in 1..h / 16 {
                for mx in 1..w / 16 {
                    for (j, ep) in eps.iter().enumerate() {
                        filter_horizontal::<u8, 16>(&mut pix, w, mx * 16, my * 16 + j * 4, ep, false, 255);
                        n += 1;
                    }
                }
            }
        }
        let dt = t0.elapsed();
        let tv = std::time::Instant::now();
        for _ in 0..20 {
            for my in 1..h / 16 {
                for mx in 1..w / 16 {
                    for (j, ep) in eps.iter().enumerate() {
                        filter_vertical::<u8, 16>(&mut pix, w, mx * 16 + j * 4, my * 16, ep, false, 255);
                    }
                }
            }
        }
        println!("vertical: {:.1} ns/edge", tv.elapsed().as_nanos() as f64 / n as f64);
        let t1 = std::time::Instant::now();
        for _ in 0..20 {
            for my in 1..h / 16 {
                for mx in 1..w / 16 {
                    for ep in eps.iter() {
                        for (vert, x, y) in [(true, mx * 16, my * 16), (false, mx * 16, my * 16)] {
                            for i in 0..16 {
                                let b = ep.bs[i / 4];
                                if b == 0 {
                                    continue;
                                }
                                let mut v = [0u8; 8];
                                for k in 0..8 {
                                    v[k] = if vert { pix[(y + i) * w + x - 4 + k] } else { pix[(y - 4 + k) * w + x + i] };
                                }
                                filter8(&mut v, b, ep.alpha, ep.beta, ep.tc0[i / 4], false);
                                for k in 1..7 {
                                    if vert {
                                        pix[(y + i) * w + x - 4 + k] = v[k];
                                    } else {
                                        pix[(y - 4 + k) * w + x + i] = v[k];
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        println!("scalar reference (both): {:.1} ns/edge", t1.elapsed().as_nanos() as f64 / (2 * n) as f64);
        println!(
            "horizontal: {n} luma edges in {dt:?}: {:.1} ns/edge (checksum {})",
            dt.as_nanos() as f64 / n as f64,
            pix.iter().map(|&v| v as u64).sum::<u64>()
        );
    }
}
