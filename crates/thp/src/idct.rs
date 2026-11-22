// Ported from FFmpeg n7.1.1 libavcodec/simple_idct_template.c (8-bit), LGPL-2.1-or-later,
// "Based upon some commented-out C code from mpeg2dec (idct_mmx.c written by Aaron Holtzman)".

//! FFmpeg's 8-bit `simple_idct`: the IDCT the JPEG decoder puts its blocks through.

const W1: i32 = 22725;
const W2: i32 = 21407;
const W3: i32 = 19266;
const W4: i32 = 16383;
const W5: i32 = 12873;
const W6: i32 = 8867;
const W7: i32 = 4520;
const ROW_SHIFT: u32 = 11;
const COL_SHIFT: u32 = 20;
const DC_SHIFT: u32 = 3;

fn m(a: i32, b: i32) -> i32 {
    a.wrapping_mul(b)
}

fn row(r: &mut [i16]) {
    // a row with nothing but its first coefficient
    if r[1..].iter().all(|&v| v == 0) {
        let t = (i32::from(r[0]) << DC_SHIFT) as i16;
        r.fill(t);
        return;
    }
    let x: [i32; 8] = std::array::from_fn(|i| i32::from(r[i]));
    let mut a0 = m(W4, x[0]).wrapping_add(1 << (ROW_SHIFT - 1));
    let mut a1 = a0;
    let mut a2 = a0;
    let mut a3 = a0;
    a0 = a0.wrapping_add(m(W2, x[2]));
    a1 = a1.wrapping_add(m(W6, x[2]));
    a2 = a2.wrapping_sub(m(W6, x[2]));
    a3 = a3.wrapping_sub(m(W2, x[2]));
    let mut b0 = m(W1, x[1]).wrapping_add(m(W3, x[3]));
    let mut b1 = m(W3, x[1]).wrapping_sub(m(W7, x[3]));
    let mut b2 = m(W5, x[1]).wrapping_sub(m(W1, x[3]));
    let mut b3 = m(W7, x[1]).wrapping_sub(m(W5, x[3]));
    a0 = a0.wrapping_add(m(W4, x[4])).wrapping_add(m(W6, x[6]));
    a1 = a1.wrapping_sub(m(W4, x[4])).wrapping_sub(m(W2, x[6]));
    a2 = a2.wrapping_sub(m(W4, x[4])).wrapping_add(m(W2, x[6]));
    a3 = a3.wrapping_add(m(W4, x[4])).wrapping_sub(m(W6, x[6]));
    b0 = b0.wrapping_add(m(W5, x[5])).wrapping_add(m(W7, x[7]));
    b1 = b1.wrapping_sub(m(W1, x[5])).wrapping_sub(m(W5, x[7]));
    b2 = b2.wrapping_add(m(W7, x[5])).wrapping_add(m(W3, x[7]));
    b3 = b3.wrapping_add(m(W3, x[5])).wrapping_sub(m(W1, x[7]));
    r[0] = (a0.wrapping_add(b0) >> ROW_SHIFT) as i16;
    r[7] = (a0.wrapping_sub(b0) >> ROW_SHIFT) as i16;
    r[1] = (a1.wrapping_add(b1) >> ROW_SHIFT) as i16;
    r[6] = (a1.wrapping_sub(b1) >> ROW_SHIFT) as i16;
    r[2] = (a2.wrapping_add(b2) >> ROW_SHIFT) as i16;
    r[5] = (a2.wrapping_sub(b2) >> ROW_SHIFT) as i16;
    r[3] = (a3.wrapping_add(b3) >> ROW_SHIFT) as i16;
    r[4] = (a3.wrapping_sub(b3) >> ROW_SHIFT) as i16;
}

fn clip(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

fn col(block: &[i16; 64], c: usize, out: &mut [u8; 8]) {
    let x: [i32; 8] = std::array::from_fn(|i| i32::from(block[8 * i + c]));
    let mut a0 = m(W4, x[0].wrapping_add((1 << (COL_SHIFT - 1)) / W4));
    let mut a1 = a0;
    let mut a2 = a0;
    let mut a3 = a0;
    a0 = a0.wrapping_add(m(W2, x[2]));
    a1 = a1.wrapping_add(m(W6, x[2]));
    a2 = a2.wrapping_sub(m(W6, x[2]));
    a3 = a3.wrapping_sub(m(W2, x[2]));
    let mut b0 = m(W1, x[1]).wrapping_add(m(W3, x[3]));
    let mut b1 = m(W3, x[1]).wrapping_sub(m(W7, x[3]));
    let mut b2 = m(W5, x[1]).wrapping_sub(m(W1, x[3]));
    let mut b3 = m(W7, x[1]).wrapping_sub(m(W5, x[3]));
    a0 = a0.wrapping_add(m(W4, x[4]));
    a1 = a1.wrapping_sub(m(W4, x[4]));
    a2 = a2.wrapping_sub(m(W4, x[4]));
    a3 = a3.wrapping_add(m(W4, x[4]));
    b0 = b0.wrapping_add(m(W5, x[5]));
    b1 = b1.wrapping_sub(m(W1, x[5]));
    b2 = b2.wrapping_add(m(W7, x[5]));
    b3 = b3.wrapping_add(m(W3, x[5]));
    a0 = a0.wrapping_add(m(W6, x[6]));
    a1 = a1.wrapping_sub(m(W2, x[6]));
    a2 = a2.wrapping_add(m(W2, x[6]));
    a3 = a3.wrapping_sub(m(W6, x[6]));
    b0 = b0.wrapping_add(m(W7, x[7]));
    b1 = b1.wrapping_sub(m(W5, x[7]));
    b2 = b2.wrapping_add(m(W3, x[7]));
    b3 = b3.wrapping_sub(m(W1, x[7]));
    out[0] = clip(a0.wrapping_add(b0) >> COL_SHIFT);
    out[1] = clip(a1.wrapping_add(b1) >> COL_SHIFT);
    out[2] = clip(a2.wrapping_add(b2) >> COL_SHIFT);
    out[3] = clip(a3.wrapping_add(b3) >> COL_SHIFT);
    out[4] = clip(a3.wrapping_sub(b3) >> COL_SHIFT);
    out[5] = clip(a2.wrapping_sub(b2) >> COL_SHIFT);
    out[6] = clip(a1.wrapping_sub(b1) >> COL_SHIFT);
    out[7] = clip(a0.wrapping_sub(b0) >> COL_SHIFT);
}

/// Transform `block` (row-major) and write the 8 × 8 result to `dest` at `off` with `stride`.
pub fn put(block: &mut [i16; 64], dest: &mut [u8], off: usize, stride: usize) {
    for r in block.chunks_exact_mut(8) {
        row(r);
    }
    let mut o = [0u8; 8];
    for c in 0..8 {
        col(block, c, &mut o);
        for (y, &v) in o.iter().enumerate() {
            dest[off + y * stride + c] = v;
        }
    }
}
