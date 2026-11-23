//! The H.264 intra predictors SVQ3 uses (`h264pred.c`, `h264pred_template.c`, 8-bit), with
//! SVQ3's own "diagonal down left" 4x4 and "plane" 16x16 variants. `pos` is the index of the
//! block's top-left sample in `buf`; neighbours at `pos - 1` and `pos - stride` exist (the planes
//! have a margin), predictors only read those they are meant to.

fn clip_u8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// A 4x4 block predictor, `mode` as `h264pred.h` numbers them (0..=11); `topright` is the index of
/// the four samples right of the row above the block (for the modes that use them).
pub fn pred4x4(mode: usize, buf: &mut [u8], pos: usize, stride: usize, topright: Option<usize>) {
    let g = |buf: &[u8], dx: isize, dy: isize| i32::from(buf[(pos as isize + dx + dy * stride as isize) as usize]);
    let t: [i32; 4] = std::array::from_fn(|i| g(buf, i as isize, -1));
    let l: [i32; 4] = std::array::from_fn(|i| g(buf, -1, i as isize));
    let lt = g(buf, -1, -1);
    let tr: [i32; 4] = match topright {
        Some(p) => std::array::from_fn(|i| i32::from(buf[p + i])),
        None => [0; 4],
    };
    let mut out = [[0i32; 4]; 4]; // out[y][x]
    match mode {
        0 => {
            out.fill(t);
        }
        1 => {
            for (y, row) in out.iter_mut().enumerate() {
                *row = [l[y]; 4];
            }
        }
        2 => out = [[(t.iter().sum::<i32>() + l.iter().sum::<i32>() + 4) >> 3; 4]; 4],
        3 => {
            // SVQ3's variant: uses the left column too
            out[0][0] = (l[1] + t[1]) >> 1;
            out[0][1] = (l[2] + t[2]) >> 1;
            out[1][0] = out[0][1];
            let v = (l[3] + t[3]) >> 1;
            for (y, row) in out.iter_mut().enumerate() {
                for (x, o) in row.iter_mut().enumerate() {
                    if x + y >= 2 {
                        *o = v;
                    }
                }
            }
        }
        4 => {
            let (t0, t1, t2, t3) = (t[0], t[1], t[2], t[3]);
            let (l0, l1, l2, l3) = (l[0], l[1], l[2], l[3]);
            out[3][0] = (l3 + 2 * l2 + l1 + 2) >> 2;
            out[2][0] = (l2 + 2 * l1 + l0 + 2) >> 2;
            out[3][1] = out[2][0];
            let v = (l1 + 2 * l0 + lt + 2) >> 2;
            out[1][0] = v;
            out[2][1] = v;
            out[3][2] = v;
            let v = (l0 + 2 * lt + t0 + 2) >> 2;
            for i in 0..4 {
                out[i][i] = v;
            }
            let v = (lt + 2 * t0 + t1 + 2) >> 2;
            out[0][1] = v;
            out[1][2] = v;
            out[2][3] = v;
            let v = (t0 + 2 * t1 + t2 + 2) >> 2;
            out[0][2] = v;
            out[1][3] = v;
            out[0][3] = (t1 + 2 * t2 + t3 + 2) >> 2;
        }
        5 => {
            let (t0, t1, t2, t3) = (t[0], t[1], t[2], t[3]);
            let (l0, l1, l2) = (l[0], l[1], l[2]);
            out[0][0] = (lt + t0 + 1) >> 1;
            out[2][1] = out[0][0];
            out[0][1] = (t0 + t1 + 1) >> 1;
            out[2][2] = out[0][1];
            out[0][2] = (t1 + t2 + 1) >> 1;
            out[2][3] = out[0][2];
            out[0][3] = (t2 + t3 + 1) >> 1;
            out[1][0] = (l0 + 2 * lt + t0 + 2) >> 2;
            out[3][1] = out[1][0];
            out[1][1] = (lt + 2 * t0 + t1 + 2) >> 2;
            out[3][2] = out[1][1];
            out[1][2] = (t0 + 2 * t1 + t2 + 2) >> 2;
            out[3][3] = out[1][2];
            out[1][3] = (t1 + 2 * t2 + t3 + 2) >> 2;
            out[2][0] = (lt + 2 * l0 + l1 + 2) >> 2;
            out[3][0] = (l0 + 2 * l1 + l2 + 2) >> 2;
        }
        6 => {
            let (t0, t1, t2) = (t[0], t[1], t[2]);
            let (l0, l1, l2, l3) = (l[0], l[1], l[2], l[3]);
            out[0][0] = (lt + l0 + 1) >> 1;
            out[1][2] = out[0][0];
            out[0][1] = (l0 + 2 * lt + t0 + 2) >> 2;
            out[1][3] = out[0][1];
            out[0][2] = (lt + 2 * t0 + t1 + 2) >> 2;
            out[0][3] = (t0 + 2 * t1 + t2 + 2) >> 2;
            out[1][0] = (l0 + l1 + 1) >> 1;
            out[2][2] = out[1][0];
            out[1][1] = (lt + 2 * l0 + l1 + 2) >> 2;
            out[2][3] = out[1][1];
            out[2][0] = (l1 + l2 + 1) >> 1;
            out[3][2] = out[2][0];
            out[2][1] = (l0 + 2 * l1 + l2 + 2) >> 2;
            out[3][3] = out[2][1];
            out[3][0] = (l2 + l3 + 1) >> 1;
            out[3][1] = (l1 + 2 * l2 + l3 + 2) >> 2;
        }
        7 => {
            let (t0, t1, t2, t3) = (t[0], t[1], t[2], t[3]);
            let (t4, t5, t6) = (tr[0], tr[1], tr[2]);
            out[0][0] = (t0 + t1 + 1) >> 1;
            out[0][1] = (t1 + t2 + 1) >> 1;
            out[2][0] = out[0][1];
            out[0][2] = (t2 + t3 + 1) >> 1;
            out[2][1] = out[0][2];
            out[0][3] = (t3 + t4 + 1) >> 1;
            out[2][2] = out[0][3];
            out[2][3] = (t4 + t5 + 1) >> 1;
            out[1][0] = (t0 + 2 * t1 + t2 + 2) >> 2;
            out[1][1] = (t1 + 2 * t2 + t3 + 2) >> 2;
            out[3][0] = out[1][1];
            out[1][2] = (t2 + 2 * t3 + t4 + 2) >> 2;
            out[3][1] = out[1][2];
            out[1][3] = (t3 + 2 * t4 + t5 + 2) >> 2;
            out[3][2] = out[1][3];
            out[3][3] = (t4 + 2 * t5 + t6 + 2) >> 2;
        }
        8 => {
            let (l0, l1, l2, l3) = (l[0], l[1], l[2], l[3]);
            out[0][0] = (l0 + l1 + 1) >> 1;
            out[0][1] = (l0 + 2 * l1 + l2 + 2) >> 2;
            out[0][2] = (l1 + l2 + 1) >> 1;
            out[1][0] = out[0][2];
            out[0][3] = (l1 + 2 * l2 + l3 + 2) >> 2;
            out[1][1] = out[0][3];
            out[1][2] = (l2 + l3 + 1) >> 1;
            out[2][0] = out[1][2];
            out[1][3] = (l2 + 2 * l3 + l3 + 2) >> 2;
            out[2][1] = out[1][3];
            for (y, x) in [(2, 2), (2, 3), (3, 0), (3, 1), (3, 2), (3, 3)] {
                out[y][x] = l3;
            }
        }
        9 => out = [[(l.iter().sum::<i32>() + 2) >> 2; 4]; 4],
        10 => out = [[(t.iter().sum::<i32>() + 2) >> 2; 4]; 4],
        _ => out = [[128; 4]; 4],
    }
    for (y, row) in out.iter().enumerate() {
        for (x, &v) in row.iter().enumerate() {
            buf[pos + x + y * stride] = v as u8;
        }
    }
}

fn fill(buf: &mut [u8], pos: usize, stride: usize, w: usize, h: usize, v: i32) {
    for y in 0..h {
        buf[pos + y * stride..pos + y * stride + w].fill(v as u8);
    }
}

/// A chroma 8x8 predictor: 0 DC, 1 horizontal, 2 vertical, 3 plane, 4 left DC, 5 top DC, 6 128.
pub fn pred8x8(mode: i32, buf: &mut [u8], pos: usize, stride: usize) {
    let s = stride;
    let px = |buf: &[u8], x: isize, y: isize| i32::from(buf[(pos as isize + x + y * s as isize) as usize]);
    match mode {
        0 => {
            let (mut dc0, mut dc1, mut dc2) = (0, 0, 0);
            for i in 0..4isize {
                dc0 += px(buf, -1, i) + px(buf, i, -1);
                dc1 += px(buf, 4 + i, -1);
                dc2 += px(buf, -1, i + 4);
            }
            fill(buf, pos, s, 4, 4, (dc0 + 4) >> 3);
            fill(buf, pos + 4, s, 4, 4, (dc1 + 2) >> 2);
            fill(buf, pos + 4 * s, s, 4, 4, (dc2 + 2) >> 2);
            fill(buf, pos + 4 * s + 4, s, 4, 4, (dc1 + dc2 + 4) >> 3);
        }
        1 => {
            for y in 0..8 {
                let v = px(buf, -1, y as isize);
                fill(buf, pos + y * s, s, 8, 1, v);
            }
        }
        2 => {
            for y in 0..8 {
                for x in 0..8 {
                    buf[pos + y * s + x] = buf[pos + x - s];
                }
            }
        }
        3 => {
            let (mut h, mut v) = (px(buf, 4, -1) - px(buf, 2, -1), px(buf, -1, 4) - px(buf, -1, 2));
            for k in 2..=4isize {
                h += k as i32 * (px(buf, 3 + k, -1) - px(buf, 3 - k, -1));
                v += k as i32 * (px(buf, -1, 3 + k) - px(buf, -1, 3 - k));
            }
            h = (17 * h + 16) >> 5;
            v = (17 * v + 16) >> 5;
            let a = 16 * (px(buf, -1, 7) + px(buf, 7, -1) + 1) - 3 * (v + h);
            for y in 0..8 {
                for x in 0..8 {
                    buf[pos + y * s + x] = clip_u8((a + y as i32 * v + x as i32 * h) >> 5);
                }
            }
        }
        4 => {
            let (mut dc0, mut dc2) = (0, 0);
            for i in 0..4isize {
                dc0 += px(buf, -1, i);
                dc2 += px(buf, -1, i + 4);
            }
            fill(buf, pos, s, 8, 4, (dc0 + 2) >> 2);
            fill(buf, pos + 4 * s, s, 8, 4, (dc2 + 2) >> 2);
        }
        5 => {
            let (mut dc0, mut dc1) = (0, 0);
            for i in 0..4isize {
                dc0 += px(buf, i, -1);
                dc1 += px(buf, 4 + i, -1);
            }
            fill(buf, pos, s, 4, 8, (dc0 + 2) >> 2);
            fill(buf, pos + 4, s, 4, 8, (dc1 + 2) >> 2);
        }
        _ => fill(buf, pos, s, 8, 8, 128),
    }
}

/// A 16x16 luma predictor: 0 DC, 1 horizontal, 2 vertical, 3 plane (SVQ3's), 4 left DC, 5 top DC,
/// 6 128.
pub fn pred16x16(mode: i32, buf: &mut [u8], pos: usize, stride: usize) {
    let s = stride;
    let px = |buf: &[u8], x: isize, y: isize| i32::from(buf[(pos as isize + x + y * s as isize) as usize]);
    match mode {
        0 => {
            let dc: i32 = (0..16isize).map(|i| px(buf, -1, i) + px(buf, i, -1)).sum();
            fill(buf, pos, s, 16, 16, (dc + 16) >> 5);
        }
        1 => {
            for y in 0..16 {
                let v = px(buf, -1, y as isize);
                fill(buf, pos + y * s, s, 16, 1, v);
            }
        }
        2 => {
            for y in 0..16 {
                for x in 0..16 {
                    buf[pos + y * s + x] = buf[pos + x - s];
                }
            }
        }
        3 => {
            let (mut h, mut v) = (px(buf, 8, -1) - px(buf, 6, -1), px(buf, -1, 8) - px(buf, -1, 6));
            for k in 2..=8isize {
                h += k as i32 * (px(buf, 7 + k, -1) - px(buf, 7 - k, -1));
                v += k as i32 * (px(buf, -1, 7 + k) - px(buf, -1, 7 - k));
            }
            // SVQ3 variant, "required for 100% accuracy": H and V swapped
            h = (5 * (h / 4)) / 16;
            v = (5 * (v / 4)) / 16;
            std::mem::swap(&mut h, &mut v);
            let a = 16 * (px(buf, -1, 15) + px(buf, 15, -1) + 1) - 7 * (v + h);
            for y in 0..16 {
                for x in 0..16 {
                    buf[pos + y * s + x] = clip_u8((a + y as i32 * v + x as i32 * h) >> 5);
                }
            }
        }
        4 => {
            let dc: i32 = (0..16isize).map(|i| px(buf, -1, i)).sum();
            fill(buf, pos, s, 16, 16, (dc + 8) >> 4);
        }
        5 => {
            let dc: i32 = (0..16isize).map(|i| px(buf, i, -1)).sum();
            fill(buf, pos, s, 16, 16, (dc + 8) >> 4);
        }
        _ => fill(buf, pos, s, 16, 16, 128),
    }
}
