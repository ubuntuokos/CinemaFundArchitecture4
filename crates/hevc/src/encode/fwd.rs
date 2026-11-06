//! Forward transforms (the transposes of 8.6.4.2), quantization and block distortion measures.

use crate::tables::{DST_MATRIX, TRANS_MATRIX};

/// Forward DCT (or DST-VII for intra 4x4 luma) of the n x n residual `r` into `out` (row-major,
/// `out[v * n + u]` = horizontal frequency u, vertical v), scaled like the reference encoder so that
/// the decoder's scaling and inverse transform invert it.
pub fn forward(r: &[i32], out: &mut [i32], n: usize, dst: bool, bit_depth: u32) {
    match n {
        4 => fwd::<4>(r, out, dst, bit_depth),
        8 => fwd::<8>(r, out, false, bit_depth),
        16 => fwd::<16>(r, out, false, bit_depth),
        _ => fwd::<32>(r, out, false, bit_depth),
    }
}

fn fwd<const N: usize>(r: &[i32], out: &mut [i32], dst: bool, bit_depth: u32) {
    let log2 = N.trailing_zeros();
    let shift1 = (log2 + bit_depth).saturating_sub(9);
    let add1 = if shift1 > 0 { 1i32 << (shift1 - 1) } else { 0 };
    let shift2 = log2 + 6;
    let add2 = 1i32 << (shift2 - 1);
    let t1 = |x: &[i32], o: &mut [i32]| if dst { dst4(x, o) } else { dct_1d(x, N, o) };
    // horizontal: tmp[k][y] = sum_x M[k][x] r[y][x] (stored transposed)
    let mut tmp = [[0i32; N]; N];
    let mut line = [0i32; N];
    for y in 0..N {
        t1(&r[y * N..y * N + N], &mut line);
        for k in 0..N {
            tmp[k][y] = (line[k] + add1) >> shift1;
        }
    }
    // vertical: out[v][u] = sum_y M[v][y] tmp[u][y]
    for u in 0..N {
        t1(&tmp[u], &mut line);
        for v in 0..N {
            out[v * N + u] = (line[v] + add2) >> shift2;
        }
    }
}

/// `out[k] = sum_j M[k][j] x[j]` for the n-point DCT (n = 4..32) by even / odd decomposition: the
/// even rows of the n-point matrix are the (n/2)-point matrix applied to x[j] + x[n-1-j], the odd
/// rows are antisymmetric. Sums stay within i32 for residuals of up to 16 bits.
fn dct_1d(x: &[i32], n: usize, out: &mut [i32]) {
    let step = 32 / n;
    if n == 4 {
        for (k, o) in out.iter_mut().enumerate().take(4) {
            let m = &TRANS_MATRIX[k * step];
            *o = m[0] as i32 * x[0] + m[1] as i32 * x[1] + m[2] as i32 * x[2] + m[3] as i32 * x[3];
        }
        return;
    }
    let h = n / 2;
    let mut e = [0i32; 16];
    let mut od = [0i32; 16];
    for k in 0..h {
        e[k] = x[k] + x[n - 1 - k];
        od[k] = x[k] - x[n - 1 - k];
    }
    let mut ee = [0i32; 16];
    dct_1d(&e[..h], h, &mut ee[..h]);
    for m in 0..h {
        out[2 * m] = ee[m];
        let row = &TRANS_MATRIX[(2 * m + 1) * step];
        let mut s = 0i32;
        for j in 0..h {
            s += row[j] as i32 * od[j];
        }
        out[2 * m + 1] = s;
    }
}

fn dst4(x: &[i32], out: &mut [i32]) {
    for (k, o) in out.iter_mut().enumerate().take(4) {
        let m = &DST_MATRIX[k];
        *o = m[0] as i32 * x[0] + m[1] as i32 * x[1] + m[2] as i32 * x[2] + m[3] as i32 * x[3];
    }
}

const QUANT_SCALES: [i64; 6] = [26214, 23302, 20560, 18396, 16384, 14564];

/// Dead-zone scalar quantization of `c` in place (levels replace coefficients); `qp` includes
/// QpBdOffset. Returns the number of non-zero levels.
pub fn quantize(c: &mut [i32], n: usize, qp: i32, bit_depth: u32, intra: bool) -> usize {
    let log2 = n.trailing_zeros() as i32;
    let transform_shift = 15 - bit_depth as i32 - log2;
    let q_bits = 14 + qp / 6 + transform_shift;
    let scale = QUANT_SCALES[qp.rem_euclid(6) as usize];
    // rounding offset: 171/512 (intra) or 85/512 (inter) of a step
    let offset = (if intra { 171i64 } else { 85 }) << (q_bits - 9).max(0);
    let mut nz = 0;
    for v in c[..n * n].iter_mut() {
        let a = (*v as i64).abs();
        let level = ((a * scale + offset) >> q_bits).min(32767) as i32;
        if level != 0 {
            nz += 1;
        }
        *v = if *v < 0 { -level } else { level };
    }
    nz
}

/// Sum of squared differences of an n x n block (`a` from a picture plane, `b` the source).
#[inline]
pub fn sse<S: crate::picture::Sample>(a: &[S], astride: usize, b: &[u16], bstride: usize, w: usize, h: usize) -> u64 {
    let mut s = 0u64;
    for y in 0..h {
        let ra = &a[y * astride..y * astride + w];
        let rb = &b[y * bstride..y * bstride + w];
        for (&p, &q) in ra.iter().zip(rb) {
            let d = p.to_i32() - q as i32;
            s += (d * d) as u64;
        }
    }
    s
}

/// Sum of absolute differences.
#[inline]
pub fn sad<S: crate::picture::Sample>(a: &[S], astride: usize, b: &[u16], bstride: usize, w: usize, h: usize) -> u32 {
    let mut s = 0u32;
    for y in 0..h {
        let ra = &a[y * astride..y * astride + w];
        let rb = &b[y * bstride..y * bstride + w];
        for (&p, &q) in ra.iter().zip(rb) {
            s += (p.to_i32() - q as i32).unsigned_abs();
        }
    }
    s
}

/// Sum of absolute 4x4 (w or h of 4) or 8x8 Hadamard-transformed differences of a w x h block of
/// differences `d` (row stride `w`).
pub fn satd(d: &[i32], w: usize, h: usize) -> u32 {
    let mut total = 0u32;
    if w.is_multiple_of(8) && h.is_multiple_of(8) {
        for by in (0..h).step_by(8) {
            for bx in (0..w).step_by(8) {
                let mut m = [[0i32; 8]; 8];
                for (y, row) in m.iter_mut().enumerate() {
                    row.copy_from_slice(&d[(by + y) * w + bx..(by + y) * w + bx + 8]);
                }
                total += (hadamard8(&mut m) + 2) >> 2;
            }
        }
    } else {
        for by in (0..h).step_by(4) {
            for bx in (0..w).step_by(4) {
                let mut m = [[0i32; 4]; 4];
                for (y, row) in m.iter_mut().enumerate() {
                    row.copy_from_slice(&d[(by + y) * w + bx..(by + y) * w + bx + 4]);
                }
                total += (hadamard4(&mut m) + 1) >> 1;
            }
        }
    }
    total
}

fn hadamard4(m: &mut [[i32; 4]; 4]) -> u32 {
    for row in m.iter_mut() {
        let (a, b, c, d) = (row[0] + row[1], row[0] - row[1], row[2] + row[3], row[2] - row[3]);
        *row = [a + c, b + d, a - c, b - d];
    }
    let mut s = 0u32;
    for x in 0..4 {
        let (a, b, c, d) = (m[0][x] + m[1][x], m[0][x] - m[1][x], m[2][x] + m[3][x], m[2][x] - m[3][x]);
        s += ((a + c).unsigned_abs()) + (b + d).unsigned_abs() + (a - c).unsigned_abs() + (b - d).unsigned_abs();
    }
    s
}

fn hadamard8(m: &mut [[i32; 8]; 8]) -> u32 {
    fn h8(v: &mut [i32; 8]) {
        let mut step = 1;
        while step < 8 {
            for i in (0..8).step_by(step * 2) {
                for j in i..i + step {
                    let (a, b) = (v[j], v[j + step]);
                    v[j] = a + b;
                    v[j + step] = a - b;
                }
            }
            step *= 2;
        }
    }
    for row in m.iter_mut() {
        h8(row);
    }
    let mut s = 0u32;
    for x in 0..8 {
        let mut col = [0i32; 8];
        for y in 0..8 {
            col[y] = m[y][x];
        }
        h8(&mut col);
        s += col.iter().map(|v| v.unsigned_abs()).sum::<u32>();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transform::inverse_transform;

    /// Forward transform + flat quantization at a fine QP, then the decoder's scaling and inverse
    /// transform give back the residual closely.
    #[test]
    fn forward_inverts() {
        for &(n, dst) in &[(4usize, true), (4, false), (8, false), (16, false), (32, false)] {
            for bd in [8u32, 10] {
                let mut r = vec![0i32; n * n];
                let mut seed = 7u32;
                for v in r.iter_mut() {
                    seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    *v = ((seed >> 16) % 61) as i32 - 30;
                }
                let mut c = vec![0i32; n * n];
                forward(&r, &mut c, n, dst, bd);
                let qp = 4 + 6 * (bd as i32 - 8);
                quantize(&mut c, n, qp, bd, true);
                // decoder scaling (flat, m = 16)
                let log2 = n.trailing_zeros();
                let bd_shift = bd + log2 - 5;
                let scale = (crate::tables::LEVEL_SCALE[(qp % 6) as usize] as i64) << (qp / 6);
                for v in c.iter_mut() {
                    *v = ((*v as i64 * 16 * scale + (1 << (bd_shift - 1))) >> bd_shift) as i32;
                }
                inverse_transform(&mut c, n, dst, n - 1, n - 1, 20 - bd);
                let err: i32 = r.iter().zip(&c).map(|(a, b)| (a - b).abs()).max().unwrap_or(0);
                assert!(err <= 4, "n {n} dst {dst} bd {bd}: max error {err}");
            }
        }
    }

    /// The butterfly equals the direct matrix product.
    #[test]
    fn butterfly_matches_matrix() {
        for n in [4usize, 8, 16, 32] {
            let x: Vec<i32> = (0..n).map(|i| ((i * 37 + 11) % 97) as i32 - 48).collect();
            let mut out = vec![0i32; n];
            dct_1d(&x, n, &mut out);
            for (k, &o) in out.iter().enumerate() {
                let direct: i32 = (0..n).map(|j| TRANS_MATRIX[k * (32 / n)][j] as i32 * x[j]).sum();
                assert_eq!(o, direct, "n {n} k {k}");
            }
        }
    }

    #[test]
    fn satd_of_flat_block() {
        let d = vec![3i32; 64];
        // the DC term only: 64 * 3 / 4 (8x8 normalisation)
        assert_eq!(satd(&d, 8, 8), 48);
        let d4 = vec![3i32; 16];
        assert_eq!(satd(&d4, 4, 4), 24);
    }
}
