//! Inverse transforms, motion compensation and wavelet recomposition of the Indeo 4 / 5 decoders
//! (a re-expression of FFmpeg n7.1.1 `libavcodec/ivi_dsp.c`).
//!
//! All arithmetic is 32-bit and wrapping (as the C `int`), results are stored as 16-bit.

/// Inverse transforms of a block (`ivi_dsp.c`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inv {
    Haar8x8,
    RowHaar8,
    ColHaar8,
    PutPixels8x8,
    Slant8x8,
    RowSlant8,
    ColSlant8,
    Haar4x4,
    Slant4x4,
    RowHaar4,
    ColHaar4,
    RowSlant4,
    ColSlant4,
}

/// DC-only transforms (a block whose only coefficient is the DC one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dc {
    Haar2d,
    Slant2d,
    RowSlant,
    ColSlant,
    PutDcPixel8x8,
}

fn bfly_haar(s1: i32, s2: i32) -> (i32, i32) {
    let t = s1.wrapping_sub(s2) >> 1;
    (s1.wrapping_add(s2) >> 1, t)
}

/// inverse 8-point Haar transform; the arguments are in the order of the C macro
/// `INV_HAAR8(s1, s5, s3, s7, s2, s4, s6, s8)`
#[allow(clippy::too_many_arguments)]
fn inv_haar8(s1: i32, s5: i32, s3: i32, s7: i32, s2: i32, s4: i32, s6: i32, s8: i32) -> [i32; 8] {
    let t1 = s1.wrapping_mul(2);
    let t5 = s5.wrapping_mul(2);
    let (t1, t5) = bfly_haar(t1, t5);
    let (t1, t3) = bfly_haar(t1, s3);
    let (t5, t7) = bfly_haar(t5, s7);
    let (t1, t2) = bfly_haar(t1, s2);
    let (t3, t4) = bfly_haar(t3, s4);
    let (t5, t6) = bfly_haar(t5, s6);
    let (t7, t8) = bfly_haar(t7, s8);
    [t1, t2, t3, t4, t5, t6, t7, t8]
}

/// inverse 4-point Haar transform (`INV_HAAR4(s1, s3, s5, s7)`)
fn inv_haar4(s1: i32, s3: i32, s5: i32, s7: i32) -> [i32; 4] {
    let (t0, t1) = bfly_haar(s1, s3);
    let (d1, d2) = bfly_haar(t0, s5);
    let (d3, d4) = bfly_haar(t1, s7);
    [d1, d2, d3, d4]
}

fn bfly_slant(s1: i32, s2: i32) -> (i32, i32) {
    (s1.wrapping_add(s2), s1.wrapping_sub(s2))
}

/// reflection a,b = 1/2, 5/4
fn ireflect(s1: i32, s2: i32) -> (i32, i32) {
    let o1 = (s1.wrapping_add(s2.wrapping_mul(2)).wrapping_add(2) >> 2).wrapping_add(s1);
    let o2 = (s1.wrapping_mul(2).wrapping_sub(s2).wrapping_add(2) >> 2).wrapping_sub(s2);
    (o1, o2)
}

/// reflection a,b = 1/2, 7/8
fn slant_part4(s1: i32, s2: i32) -> (i32, i32) {
    let o1 = s2.wrapping_add(s1.wrapping_mul(4).wrapping_sub(s2).wrapping_add(4) >> 3);
    let o2 = s1.wrapping_add(s1.wrapping_neg().wrapping_sub(s2.wrapping_mul(4)).wrapping_add(4) >> 3);
    (o1, o2)
}

/// inverse slant8 transform, `x` in the order the C code passes them to
/// `IVI_INV_SLANT8(s1, s4, s8, s5, s2, s6, s3, s7, ...)`
fn inv_slant8(x: [i32; 8]) -> [i32; 8] {
    let (s1, s4, s8, s5, s2, s6, s3, s7) = (x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7]);
    let (t4, t5) = slant_part4(s4, s5);
    let (t1, t5) = bfly_slant(s1, t5);
    let (t2, t6) = bfly_slant(s2, s6);
    let (t7, t3) = bfly_slant(s7, s3);
    let (t4, t8) = bfly_slant(t4, s8);
    let (t1, t2) = bfly_slant(t1, t2);
    let (t4, t3) = ireflect(t4, t3);
    let (t5, t6) = bfly_slant(t5, t6);
    let (t8, t7) = ireflect(t8, t7);
    let (t1, t4) = bfly_slant(t1, t4);
    let (t2, t3) = bfly_slant(t2, t3);
    let (t5, t8) = bfly_slant(t5, t8);
    let (t6, t7) = bfly_slant(t6, t7);
    [t1, t2, t3, t4, t5, t6, t7, t8]
}

/// inverse slant4 transform (`IVI_INV_SLANT4(s1, s4, s2, s3, ...)`)
fn inv_slant4(x: [i32; 4]) -> [i32; 4] {
    let (s1, s4, s2, s3) = (x[0], x[1], x[2], x[3]);
    let (t1, t2) = bfly_slant(s1, s2);
    let (t4, t3) = ireflect(s4, s3);
    let (t1, t4) = bfly_slant(t1, t4);
    let (t2, t3) = bfly_slant(t2, t3);
    [t1, t2, t3, t4]
}

fn comp(v: i32) -> i32 {
    v.wrapping_add(1) >> 1
}

fn nz8(v: &[i32]) -> bool {
    v[..8].iter().any(|&x| x != 0)
}

fn nz4(v: &[i32]) -> bool {
    v[..4].iter().any(|&x| x != 0)
}

/// Apply the inverse transform `kind` to the coefficients `inp` (8x8 or 4x4, row-major) and
/// write the block at `out[off..]` with row pitch `pitch`.
pub fn inverse(kind: Inv, inp: &[i32; 64], out: &mut [i16], off: usize, pitch: usize, flags: &[u8; 8]) {
    let mut o = |row: usize, col: usize, v: i32| out[off + row * pitch + col] = v as i16;
    match kind {
        Inv::Haar8x8 => {
            let mut tmp = [0i32; 64];
            for i in 0..8 {
                if flags[i] != 0 {
                    let shift = u32::from(i & 4 == 0);
                    let sp = |k: usize| inp[i + 8 * k].wrapping_mul(1 << shift);
                    let d = inv_haar8(sp(0), sp(1), sp(2), sp(3), inp[i + 32], inp[i + 40], inp[i + 48], inp[i + 56]);
                    for (k, v) in d.iter().enumerate() {
                        tmp[i + 8 * k] = *v;
                    }
                }
            }
            for i in 0..8 {
                let s = &tmp[i * 8..i * 8 + 8];
                if nz8(s) {
                    let d = inv_haar8(s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]);
                    for (k, v) in d.iter().enumerate() {
                        o(i, k, *v);
                    }
                } else {
                    for k in 0..8 {
                        o(i, k, 0);
                    }
                }
            }
        }
        Inv::RowHaar8 => {
            for i in 0..8 {
                let s = &inp[i * 8..i * 8 + 8];
                if nz8(s) {
                    let d = inv_haar8(s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]);
                    for (k, v) in d.iter().enumerate() {
                        o(i, k, *v);
                    }
                } else {
                    for k in 0..8 {
                        o(i, k, 0);
                    }
                }
            }
        }
        Inv::ColHaar8 => {
            for i in 0..8 {
                if flags[i] != 0 {
                    let d = inv_haar8(inp[i], inp[i + 8], inp[i + 16], inp[i + 24], inp[i + 32], inp[i + 40], inp[i + 48], inp[i + 56]);
                    for (k, v) in d.iter().enumerate() {
                        o(k, i, *v);
                    }
                } else {
                    for k in 0..8 {
                        o(k, i, 0);
                    }
                }
            }
        }
        Inv::PutPixels8x8 => {
            for y in 0..8 {
                for x in 0..8 {
                    o(y, x, inp[y * 8 + x]);
                }
            }
        }
        Inv::Slant8x8 => {
            let mut tmp = [0i32; 64];
            for i in 0..8 {
                if flags[i] != 0 {
                    let d = inv_slant8([inp[i], inp[i + 8], inp[i + 16], inp[i + 24], inp[i + 32], inp[i + 40], inp[i + 48], inp[i + 56]]);
                    for (k, v) in d.iter().enumerate() {
                        tmp[i + 8 * k] = *v;
                    }
                }
            }
            for i in 0..8 {
                let s = &tmp[i * 8..i * 8 + 8];
                if nz8(s) {
                    let d = inv_slant8([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]);
                    for (k, v) in d.iter().enumerate() {
                        o(i, k, comp(*v));
                    }
                } else {
                    for k in 0..8 {
                        o(i, k, 0);
                    }
                }
            }
        }
        Inv::RowSlant8 => {
            for i in 0..8 {
                let s = &inp[i * 8..i * 8 + 8];
                if nz8(s) {
                    let d = inv_slant8([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]);
                    for (k, v) in d.iter().enumerate() {
                        o(i, k, comp(*v));
                    }
                } else {
                    for k in 0..8 {
                        o(i, k, 0);
                    }
                }
            }
        }
        Inv::ColSlant8 => {
            for i in 0..8 {
                if flags[i] != 0 {
                    let d = inv_slant8([inp[i], inp[i + 8], inp[i + 16], inp[i + 24], inp[i + 32], inp[i + 40], inp[i + 48], inp[i + 56]]);
                    for (k, v) in d.iter().enumerate() {
                        o(k, i, comp(*v));
                    }
                } else {
                    for k in 0..8 {
                        o(k, i, 0);
                    }
                }
            }
        }
        Inv::Haar4x4 => {
            let mut tmp = [0i32; 16];
            for i in 0..4 {
                if flags[i] != 0 {
                    let shift = u32::from(i & 2 == 0);
                    let sp1 = inp[i].wrapping_mul(1 << shift);
                    let sp2 = inp[i + 4].wrapping_mul(1 << shift);
                    let d = inv_haar4(sp1, sp2, inp[i + 8], inp[i + 12]);
                    for (k, v) in d.iter().enumerate() {
                        tmp[i + 4 * k] = *v;
                    }
                }
            }
            for i in 0..4 {
                let s = &tmp[i * 4..i * 4 + 4];
                if nz4(s) {
                    let d = inv_haar4(s[0], s[1], s[2], s[3]);
                    for (k, v) in d.iter().enumerate() {
                        o(i, k, *v);
                    }
                } else {
                    for k in 0..4 {
                        o(i, k, 0);
                    }
                }
            }
        }
        Inv::RowHaar4 => {
            for i in 0..4 {
                let s = &inp[i * 4..i * 4 + 4];
                if nz4(s) {
                    let d = inv_haar4(s[0], s[1], s[2], s[3]);
                    for (k, v) in d.iter().enumerate() {
                        o(i, k, *v);
                    }
                } else {
                    for k in 0..4 {
                        o(i, k, 0);
                    }
                }
            }
        }
        Inv::ColHaar4 => {
            for i in 0..4 {
                if flags[i] != 0 {
                    let d = inv_haar4(inp[i], inp[i + 4], inp[i + 8], inp[i + 12]);
                    for (k, v) in d.iter().enumerate() {
                        o(k, i, *v);
                    }
                } else {
                    for k in 0..4 {
                        o(k, i, 0);
                    }
                }
            }
        }
        Inv::Slant4x4 => {
            let mut tmp = [0i32; 16];
            for i in 0..4 {
                if flags[i] != 0 {
                    let d = inv_slant4([inp[i], inp[i + 4], inp[i + 8], inp[i + 12]]);
                    for (k, v) in d.iter().enumerate() {
                        tmp[i + 4 * k] = *v;
                    }
                }
            }
            for i in 0..4 {
                let s = &tmp[i * 4..i * 4 + 4];
                if nz4(s) {
                    let d = inv_slant4([s[0], s[1], s[2], s[3]]);
                    for (k, v) in d.iter().enumerate() {
                        o(i, k, comp(*v));
                    }
                } else {
                    for k in 0..4 {
                        o(i, k, 0);
                    }
                }
            }
        }
        Inv::RowSlant4 => {
            for i in 0..4 {
                let s = &inp[i * 4..i * 4 + 4];
                if nz4(s) {
                    let d = inv_slant4([s[0], s[1], s[2], s[3]]);
                    for (k, v) in d.iter().enumerate() {
                        o(i, k, comp(*v));
                    }
                } else {
                    for k in 0..4 {
                        o(i, k, 0);
                    }
                }
            }
        }
        Inv::ColSlant4 => {
            for i in 0..4 {
                if flags[i] != 0 {
                    let d = inv_slant4([inp[i], inp[i + 4], inp[i + 8], inp[i + 12]]);
                    for (k, v) in d.iter().enumerate() {
                        o(k, i, comp(*v));
                    }
                } else {
                    for k in 0..4 {
                        o(k, i, 0);
                    }
                }
            }
        }
    }
}

/// Apply the DC-only transform `kind` to the coefficient `dc`.
pub fn dc_transform(kind: Dc, dc: i32, out: &mut [i16], off: usize, pitch: usize, blk: usize) {
    match kind {
        Dc::Haar2d => {
            let v = (dc >> 3) as i16;
            for y in 0..blk {
                out[off + y * pitch..off + y * pitch + blk].fill(v);
            }
        }
        Dc::Slant2d => {
            let v = (dc.wrapping_add(1) >> 1) as i16;
            for y in 0..blk {
                out[off + y * pitch..off + y * pitch + blk].fill(v);
            }
        }
        Dc::RowSlant => {
            let v = (dc.wrapping_add(1) >> 1) as i16;
            out[off..off + blk].fill(v);
            for y in 1..blk {
                out[off + y * pitch..off + y * pitch + blk].fill(0);
            }
        }
        Dc::ColSlant => {
            let v = (dc.wrapping_add(1) >> 1) as i16;
            for y in 0..blk {
                let r = off + y * pitch;
                out[r] = v;
                out[r + 1..r + blk].fill(0);
            }
        }
        Dc::PutDcPixel8x8 => {
            out[off] = dc as i16;
            out[off + 1..off + 8].fill(0);
            for y in 1..8 {
                out[off + y * pitch..off + y * pitch + 8].fill(0);
            }
        }
    }
}

/// Motion compensation of a `size` x `size` block (`IVI_MC_TEMPLATE`): `add` accumulates into
/// the destination (the `_delta` variants), otherwise the prediction replaces it. An unknown
/// `mc_type` (the C `switch` has no default) leaves the block untouched.
#[allow(clippy::too_many_arguments)]
pub fn mc(dst: &mut [i16], doff: usize, dpitch: usize, refb: &[i16], roff: usize, pitch: usize, mc_type: i32, size: usize, add: bool) {
    let put = |d: &mut i16, v: i32| {
        if add {
            *d = (i32::from(*d)).wrapping_add(v) as i16;
        } else {
            *d = v as i16;
        }
    };
    let r = |i: usize, j: usize| i32::from(refb[roff + i * pitch + j]);
    for i in 0..size {
        for j in 0..size {
            let v = match mc_type {
                0 => r(i, j),
                1 => (r(i, j) + r(i, j + 1)) >> 1,
                2 => (r(i, j) + r(i + 1, j)) >> 1,
                3 => (r(i, j) + r(i, j + 1) + r(i + 1, j) + r(i + 1, j + 1)) >> 2,
                _ => return,
            };
            put(&mut dst[doff + i * dpitch + j], v);
        }
    }
}

/// Bidirectional motion compensation (`IVI_MC_AVG_TEMPLATE`): the average of two predictions.
#[allow(clippy::too_many_arguments)]
pub fn mc_avg(
    dst: &mut [i16],
    doff: usize,
    pitch: usize,
    ref1: &[i16],
    roff1: usize,
    ref2: &[i16],
    roff2: usize,
    mc_type: i32,
    mc_type2: i32,
    size: usize,
    add: bool,
) {
    let mut tmp = [0i16; 64];
    mc(&mut tmp, 0, size, ref1, roff1, pitch, mc_type, size, false);
    mc(&mut tmp, 0, size, ref2, roff2, pitch, mc_type2, size, true);
    for i in 0..size {
        for j in 0..size {
            let v = i32::from(tmp[i * size + j]) >> 1;
            let d = &mut dst[doff + i * pitch + j];
            if add {
                *d = i32::from(*d).wrapping_add(v) as i16;
            } else {
                *d = v as i16;
            }
        }
    }
}

fn clip_u8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

fn at(b: &[i16], i: isize) -> i32 {
    usize::try_from(i).ok().and_then(|i| b.get(i)).map_or(0, |&v| i32::from(v))
}

/// Haar recomposition of the four luma bands into a plane (`ff_ivi_recompose_haar`). `dst` must
/// hold `(height rounded up to even) * dst_pitch` bytes, `dst_pitch >= width rounded up to even`.
pub fn recompose_haar(bands: [&[i16]; 4], pitch: usize, width: usize, height: usize, dst: &mut [u8], dst_pitch: usize) {
    let mut base = 0usize;
    let mut y = 0;
    while y < height {
        let mut x = 0;
        let mut indx = 0;
        while x < width {
            let g = |b: usize| at(bands[b], (base + indx) as isize);
            let (b0, b1, b2, b3) = (g(0), g(1), g(2), g(3));
            let p0 = (b0 + b1 + b2 + b3 + 2) >> 2;
            let p1 = (b0 + b1 - b2 - b3 + 2) >> 2;
            let p2 = (b0 - b1 + b2 - b3 + 2) >> 2;
            let p3 = (b0 - b1 - b2 + b3 + 2) >> 2;
            dst[y * dst_pitch + x] = clip_u8(p0 + 128);
            dst[y * dst_pitch + x + 1] = clip_u8(p1 + 128);
            dst[(y + 1) * dst_pitch + x] = clip_u8(p2 + 128);
            dst[(y + 1) * dst_pitch + x + 1] = clip_u8(p3 + 128);
            x += 2;
            indx += 1;
        }
        y += 2;
        base += pitch;
    }
}

/// 5/3 wavelet recomposition of the four luma bands (`ff_ivi_recompose53`).
pub fn recompose53(bands: [&[i16]; 4], pitch: usize, width: usize, height: usize, dst: &mut [u8], dst_pitch: usize) {
    let mut pitch = pitch as isize;
    let mut back_pitch = 0isize;
    // pointers into the four bands
    let mut ptr = [0isize; 4];
    let (mut b0_1, mut b0_2): (i32, i32);
    let (mut b1_1, mut b1_2, mut b1_3): (i32, i32, i32);
    let (mut b2_1, mut b2_2, mut b2_3, mut b2_4, mut b2_5, mut b2_6): (i32, i32, i32, i32, i32, i32);
    let (mut b3_1, mut b3_2, mut b3_3, mut b3_4, mut b3_5, mut b3_6, mut b3_7, mut b3_8, mut b3_9): (i32, i32, i32, i32, i32, i32, i32, i32, i32);
    let mut y = 0usize;
    let mut dst_row = 0usize;
    while y < height {
        if y + 2 >= height {
            pitch = 0;
        }
        let g = |b: usize, p: &[isize; 4], off: isize| at(bands[b], p[b] + off);
        b0_1 = g(0, &ptr, 0);
        b0_2 = g(0, &ptr, pitch);

        b1_1 = g(1, &ptr, back_pitch);
        b1_2 = g(1, &ptr, 0);
        b1_3 = b1_1.wrapping_sub(b1_2.wrapping_mul(6)).wrapping_add(g(1, &ptr, pitch));

        b2_2 = g(2, &ptr, 0);
        b2_3 = b2_2;
        b2_5 = g(2, &ptr, pitch);
        b2_6 = b2_5;

        b3_2 = g(3, &ptr, back_pitch);
        b3_3 = b3_2;
        b3_5 = g(3, &ptr, 0);
        b3_6 = b3_5;
        b3_8 = b3_2.wrapping_sub(b3_5.wrapping_mul(6)).wrapping_add(g(3, &ptr, pitch));
        b3_9 = b3_8;

        let mut x = 0usize;
        let mut indx = 0isize;
        while x < width {
            if x + 2 >= width {
                for p in ptr.iter_mut() {
                    *p -= 1;
                }
            }
            b2_1 = b2_2;
            b2_2 = b2_3;
            b2_4 = b2_5;
            b2_5 = b2_6;
            b3_1 = b3_2;
            b3_2 = b3_3;
            b3_4 = b3_5;
            b3_5 = b3_6;
            b3_7 = b3_8;
            b3_8 = b3_9;

            let mut p0;
            let mut p1;
            let mut p2;
            let mut p3;

            // LL band: LPF both ways
            {
                let tmp0 = b0_1;
                let tmp2 = b0_2;
                b0_1 = g(0, &ptr, indx + 1);
                b0_2 = g(0, &ptr, pitch + indx + 1);
                let tmp1 = tmp0.wrapping_add(b0_1);
                p0 = tmp0.wrapping_mul(16);
                p1 = tmp1.wrapping_mul(8);
                p2 = tmp0.wrapping_add(tmp2).wrapping_mul(8);
                p3 = tmp1.wrapping_add(tmp2).wrapping_add(b0_2).wrapping_mul(4);
            }
            // HL band: HPF vertically, LPF horizontally
            {
                let tmp0 = b1_2;
                let tmp1 = b1_1;
                b1_2 = g(1, &ptr, indx + 1);
                b1_1 = g(1, &ptr, back_pitch + indx + 1);
                let tmp2 = tmp1.wrapping_sub(tmp0.wrapping_mul(6)).wrapping_add(b1_3);
                b1_3 = b1_1.wrapping_sub(b1_2.wrapping_mul(6)).wrapping_add(g(1, &ptr, pitch + indx + 1));
                p0 = p0.wrapping_add(tmp0.wrapping_add(tmp1).wrapping_mul(8));
                p1 = p1.wrapping_add(tmp0.wrapping_add(tmp1).wrapping_add(b1_1).wrapping_add(b1_2).wrapping_mul(4));
                p2 = p2.wrapping_add(tmp2.wrapping_mul(4));
                p3 = p3.wrapping_add(tmp2.wrapping_add(b1_3).wrapping_mul(2));
            }
            // LH band: LPF vertically, HPF horizontally
            {
                b2_3 = g(2, &ptr, indx + 1);
                b2_6 = g(2, &ptr, pitch + indx + 1);
                let tmp0 = b2_1.wrapping_add(b2_2);
                let tmp1 = b2_1.wrapping_sub(b2_2.wrapping_mul(6)).wrapping_add(b2_3);
                p0 = p0.wrapping_add(tmp0.wrapping_mul(8));
                p1 = p1.wrapping_add(tmp1.wrapping_mul(4));
                p2 = p2.wrapping_add(tmp0.wrapping_add(b2_4).wrapping_add(b2_5).wrapping_mul(4));
                p3 = p3.wrapping_add(tmp1.wrapping_add(b2_4).wrapping_sub(b2_5.wrapping_mul(6)).wrapping_add(b2_6).wrapping_mul(2));
            }
            // HH band: HPF both ways
            {
                b3_6 = g(3, &ptr, indx + 1);
                b3_3 = g(3, &ptr, back_pitch + indx + 1);
                let tmp0 = b3_1.wrapping_add(b3_4);
                let tmp1 = b3_2.wrapping_add(b3_5);
                let tmp2 = b3_3.wrapping_add(b3_6);
                b3_9 = b3_3.wrapping_sub(b3_6.wrapping_mul(6)).wrapping_add(g(3, &ptr, pitch + indx + 1));
                p0 = p0.wrapping_add(tmp0.wrapping_add(tmp1).wrapping_mul(4));
                p1 = p1.wrapping_add(tmp0.wrapping_sub(tmp1.wrapping_mul(6)).wrapping_add(tmp2).wrapping_mul(2));
                p2 = p2.wrapping_add(b3_7.wrapping_add(b3_8).wrapping_mul(2));
                p3 = p3.wrapping_add(b3_7.wrapping_sub(b3_8.wrapping_mul(6)).wrapping_add(b3_9));
            }

            dst[dst_row + x] = clip_u8((p0 >> 6) + 128);
            dst[dst_row + x + 1] = clip_u8((p1 >> 6) + 128);
            dst[dst_row + dst_pitch + x] = clip_u8((p2 >> 6) + 128);
            dst[dst_row + dst_pitch + x + 1] = clip_u8((p3 >> 6) + 128);
            x += 2;
            indx += 1;
        }
        dst_row += dst_pitch << 1;
        back_pitch = -pitch;
        for p in ptr.iter_mut() {
            *p += pitch + 1;
        }
        y += 2;
    }
}
