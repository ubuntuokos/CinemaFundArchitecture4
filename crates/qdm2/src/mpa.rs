//! FFmpeg n7.1.1 `dct32_template.c` (float) and the synthesis window of `mpegaudiodsp_data.c`,
//! LGPL-2.1-or-later.

#![allow(clippy::approx_constant, clippy::excessive_precision, clippy::unreadable_literal, clippy::identity_op)]

use std::f64::consts::FRAC_1_SQRT_2;

const C0_0: f32 = (0.50060299823519630134_f64 / 2.0) as f32;
const C0_1: f32 = (0.50547095989754365998_f64 / 2.0) as f32;
const C0_2: f32 = (0.51544730992262454697_f64 / 2.0) as f32;
const C0_3: f32 = (0.53104259108978417447_f64 / 2.0) as f32;
const C0_4: f32 = (0.55310389603444452782_f64 / 2.0) as f32;
const C0_5: f32 = (0.58293496820613387367_f64 / 2.0) as f32;
const C0_6: f32 = (0.62250412303566481615_f64 / 2.0) as f32;
const C0_7: f32 = (0.67480834145500574602_f64 / 2.0) as f32;
const C0_8: f32 = (0.74453627100229844977_f64 / 2.0) as f32;
const C0_9: f32 = (0.83934964541552703873_f64 / 2.0) as f32;
const C0_10: f32 = (0.97256823786196069369_f64 / 2.0) as f32;
const C0_11: f32 = (1.16943993343288495515_f64 / 4.0) as f32;
const C0_12: f32 = (1.48416461631416627724_f64 / 4.0) as f32;
const C0_13: f32 = (2.05778100995341155085_f64 / 8.0) as f32;
const C0_14: f32 = (3.40760841846871878570_f64 / 8.0) as f32;
const C0_15: f32 = (10.19000812354805681150_f64 / 32.0) as f32;
const C1_0: f32 = (0.50241928618815570551_f64 / 2.0) as f32;
const C1_1: f32 = (0.52249861493968888062_f64 / 2.0) as f32;
const C1_2: f32 = (0.56694403481635770368_f64 / 2.0) as f32;
const C1_3: f32 = (0.64682178335999012954_f64 / 2.0) as f32;
const C1_4: f32 = (0.78815462345125022473_f64 / 2.0) as f32;
const C1_5: f32 = (1.06067768599034747134_f64 / 4.0) as f32;
const C1_6: f32 = (1.72244709823833392782_f64 / 4.0) as f32;
const C1_7: f32 = (5.10114861868916385802_f64 / 16.0) as f32;
const C2_0: f32 = (0.50979557910415916894_f64 / 2.0) as f32;
const C2_1: f32 = (0.60134488693504528054_f64 / 2.0) as f32;
const C2_2: f32 = (0.89997622313641570463_f64 / 2.0) as f32;
const C2_3: f32 = (2.56291544774150617881_f64 / 8.0) as f32;
const C3_0: f32 = (0.54119610014619698439_f64 / 2.0) as f32;
const C3_1: f32 = (1.30656296487637652785_f64 / 4.0) as f32;
const C4_0: f32 = (FRAC_1_SQRT_2 / 2.0) as f32;

/// `MULH3(x, y, s)`: `(s) * (y) * (x)` in float.
fn mulh3(x: f32, y: f32, s: u32) -> f32 {
    (s as f32) * y * x
}

fn bf0(v: &mut [f32; 32], tab: &[f32; 32], a: usize, b: usize, c: f32, s: u32) {
    let (t0, t1) = (tab[a] + tab[b], tab[a] - tab[b]);
    v[a] = t0;
    v[b] = mulh3(t1, c, 1 << s);
}

fn bf(v: &mut [f32; 32], a: usize, b: usize, c: f32, s: u32) {
    let (t0, t1) = (v[a] + v[b], v[a] - v[b]);
    v[a] = t0;
    v[b] = mulh3(t1, c, 1 << s);
}

/// FFmpeg's `ff_dct32_float` (without the 1/sqrt(2) scaling of coefficient 0).
pub(crate) fn dct32(out: &mut [f32; 32], tab: &[f32; 32]) {
    let mut v = [0.0f32; 32];
    bf0(&mut v, tab, 0, 31, C0_0, 1);
    bf0(&mut v, tab, 15, 16, C0_15, 5);
    bf(&mut v, 0, 15, C1_0, 1);
    bf(&mut v, 16, 31, -C1_0, 1);
    bf0(&mut v, tab, 7, 24, C0_7, 1);
    bf0(&mut v, tab, 8, 23, C0_8, 1);
    bf(&mut v, 7, 8, C1_7, 4);
    bf(&mut v, 23, 24, -C1_7, 4);
    bf(&mut v, 0, 7, C2_0, 1);
    bf(&mut v, 8, 15, -C2_0, 1);
    bf(&mut v, 16, 23, C2_0, 1);
    bf(&mut v, 24, 31, -C2_0, 1);
    bf0(&mut v, tab, 3, 28, C0_3, 1);
    bf0(&mut v, tab, 12, 19, C0_12, 2);
    bf(&mut v, 3, 12, C1_3, 1);
    bf(&mut v, 19, 28, -C1_3, 1);
    bf0(&mut v, tab, 4, 27, C0_4, 1);
    bf0(&mut v, tab, 11, 20, C0_11, 2);
    bf(&mut v, 4, 11, C1_4, 1);
    bf(&mut v, 20, 27, -C1_4, 1);
    bf(&mut v, 3, 4, C2_3, 3);
    bf(&mut v, 11, 12, -C2_3, 3);
    bf(&mut v, 19, 20, C2_3, 3);
    bf(&mut v, 27, 28, -C2_3, 3);
    bf(&mut v, 0, 3, C3_0, 1);
    bf(&mut v, 4, 7, -C3_0, 1);
    bf(&mut v, 8, 11, C3_0, 1);
    bf(&mut v, 12, 15, -C3_0, 1);
    bf(&mut v, 16, 19, C3_0, 1);
    bf(&mut v, 20, 23, -C3_0, 1);
    bf(&mut v, 24, 27, C3_0, 1);
    bf(&mut v, 28, 31, -C3_0, 1);
    bf0(&mut v, tab, 1, 30, C0_1, 1);
    bf0(&mut v, tab, 14, 17, C0_14, 3);
    bf(&mut v, 1, 14, C1_1, 1);
    bf(&mut v, 17, 30, -C1_1, 1);
    bf0(&mut v, tab, 6, 25, C0_6, 1);
    bf0(&mut v, tab, 9, 22, C0_9, 1);
    bf(&mut v, 6, 9, C1_6, 2);
    bf(&mut v, 22, 25, -C1_6, 2);
    bf(&mut v, 1, 6, C2_1, 1);
    bf(&mut v, 9, 14, -C2_1, 1);
    bf(&mut v, 17, 22, C2_1, 1);
    bf(&mut v, 25, 30, -C2_1, 1);
    bf0(&mut v, tab, 2, 29, C0_2, 1);
    bf0(&mut v, tab, 13, 18, C0_13, 3);
    bf(&mut v, 2, 13, C1_2, 1);
    bf(&mut v, 18, 29, -C1_2, 1);
    bf0(&mut v, tab, 5, 26, C0_5, 1);
    bf0(&mut v, tab, 10, 21, C0_10, 1);
    bf(&mut v, 5, 10, C1_5, 2);
    bf(&mut v, 21, 26, -C1_5, 2);
    bf(&mut v, 2, 5, C2_2, 1);
    bf(&mut v, 10, 13, -C2_2, 1);
    bf(&mut v, 18, 21, C2_2, 1);
    bf(&mut v, 26, 29, -C2_2, 1);
    bf(&mut v, 1, 2, C3_1, 2);
    bf(&mut v, 5, 6, -C3_1, 2);
    bf(&mut v, 9, 10, C3_1, 2);
    bf(&mut v, 13, 14, -C3_1, 2);
    bf(&mut v, 17, 18, C3_1, 2);
    bf(&mut v, 21, 22, -C3_1, 2);
    bf(&mut v, 25, 26, C3_1, 2);
    bf(&mut v, 29, 30, -C3_1, 2);
    bf(&mut v, 0, 1, C4_0, 1);
    bf(&mut v, 2, 3, -C4_0, 1);
    v[2] += v[3];
    bf(&mut v, 4, 5, C4_0, 1);
    bf(&mut v, 6, 7, -C4_0, 1);
    v[6] += v[7];
    v[4] += v[6];
    v[6] += v[5];
    v[5] += v[7];
    bf(&mut v, 8, 9, C4_0, 1);
    bf(&mut v, 10, 11, -C4_0, 1);
    v[10] += v[11];
    bf(&mut v, 12, 13, C4_0, 1);
    bf(&mut v, 14, 15, -C4_0, 1);
    v[14] += v[15];
    v[12] += v[14];
    v[14] += v[13];
    v[13] += v[15];
    bf(&mut v, 16, 17, C4_0, 1);
    bf(&mut v, 18, 19, -C4_0, 1);
    v[18] += v[19];
    bf(&mut v, 20, 21, C4_0, 1);
    bf(&mut v, 22, 23, -C4_0, 1);
    v[22] += v[23];
    v[20] += v[22];
    v[22] += v[21];
    v[21] += v[23];
    bf(&mut v, 24, 25, C4_0, 1);
    bf(&mut v, 26, 27, -C4_0, 1);
    v[26] += v[27];
    bf(&mut v, 28, 29, C4_0, 1);
    bf(&mut v, 30, 31, -C4_0, 1);
    v[30] += v[31];
    v[28] += v[30];
    v[30] += v[29];
    v[29] += v[31];
    v[8] += v[12];
    v[12] += v[10];
    v[10] += v[14];
    v[14] += v[9];
    v[9] += v[13];
    v[13] += v[11];
    v[11] += v[15];
    out[0] = v[0];
    out[16] = v[1];
    out[8] = v[2];
    out[24] = v[3];
    out[4] = v[4];
    out[20] = v[5];
    out[12] = v[6];
    out[28] = v[7];
    out[2] = v[8];
    out[18] = v[9];
    out[10] = v[10];
    out[26] = v[11];
    out[6] = v[12];
    out[22] = v[13];
    out[14] = v[14];
    out[30] = v[15];
    v[24] += v[28];
    v[28] += v[26];
    v[26] += v[30];
    v[30] += v[25];
    v[25] += v[29];
    v[29] += v[27];
    v[27] += v[31];
    out[1] = v[16] + v[24];
    out[17] = v[17] + v[25];
    out[9] = v[18] + v[26];
    out[25] = v[19] + v[27];
    out[5] = v[20] + v[28];
    out[21] = v[21] + v[29];
    out[13] = v[22] + v[30];
    out[29] = v[23] + v[31];
    out[3] = v[24] + v[20];
    out[19] = v[25] + v[21];
    out[11] = v[26] + v[22];
    out[27] = v[27] + v[23];
    out[7] = v[28] + v[18];
    out[23] = v[29] + v[19];
    out[15] = v[30] + v[17];
    out[31] = v[31];
}

pub(crate) static ENWINDOW: [i32; 257] = [
    0, -1, -1, -1, -1, -1, -1, -2, -2, -2, -2, -3, -3, -4, -4, -5, -5, -6, -7, -7, -8, -9, -10, -11, -13, -14, -16, -17, -19, -21, -24, -26, -29, -31, -35,
    -38, -41, -45, -49, -53, -58, -63, -68, -73, -79, -85, -91, -97, -104, -111, -117, -125, -132, -139, -147, -154, -161, -169, -176, -183, -190, -196, -202,
    -208, 213, 218, 222, 225, 227, 228, 228, 227, 224, 221, 215, 208, 200, 189, 177, 163, 146, 127, 106, 83, 57, 29, -2, -36, -72, -111, -153, -197, -244,
    -294, -347, -401, -459, -519, -581, -645, -711, -779, -848, -919, -991, -1064, -1137, -1210, -1283, -1356, -1428, -1498, -1567, -1634, -1698, -1759, -1817,
    -1870, -1919, -1962, -2001, -2032, -2057, -2075, -2085, -2087, -2080, -2063, 2037, 2000, 1952, 1893, 1822, 1739, 1644, 1535, 1414, 1280, 1131, 970, 794,
    605, 402, 185, -45, -288, -545, -814, -1095, -1388, -1692, -2006, -2330, -2663, -3004, -3351, -3705, -4063, -4425, -4788, -5153, -5517, -5879, -6237,
    -6589, -6935, -7271, -7597, -7910, -8209, -8491, -8755, -8998, -9219, -9416, -9585, -9727, -9838, -9916, -9959, -9966, -9935, -9863, -9750, -9592, -9389,
    -9139, -8840, -8492, -8092, -7640, -7134, 6574, 5959, 5288, 4561, 3776, 2935, 2037, 1082, 70, -998, -2122, -3300, -4533, -5818, -7154, -8540, -9975,
    -11455, -12980, -14548, -16155, -17799, -19478, -21189, -22929, -24694, -26482, -28289, -30112, -31947, -33791, -35640, -37489, -39336, -41176, -43006,
    -44821, -46617, -48390, -50137, -51853, -53534, -55178, -56778, -58333, -59838, -61289, -62684, -64019, -65290, -66494, -67629, -68692, -69679, -70590,
    -71420, -72169, -72835, -73415, -73908, -74313, -74630, -74856, -74992, 75038,
];

/// `ff_mpa_synth_init_float`: the synthesis window (512 + 256 floats).
pub(crate) fn synth_window() -> Vec<f32> {
    let mut window = vec![0.0f32; 512 + 256];
    for i in 0..257usize {
        let mut v = (f64::from(ENWINDOW[i] as f32) * (1.0 / (1u64 << (16 + 23)) as f64)) as f32;
        window[i] = v;
        if i & 63 != 0 {
            v = -v;
        }
        if i != 0 {
            window[512 - i] = v;
        }
    }
    for i in 0..8 {
        for j in 0..16 {
            window[512 + 16 * i + j] = window[64 * i + 32 - j];
        }
    }
    for i in 0..8 {
        for j in 0..16 {
            window[512 + 128 + 16 * i + j] = window[64 * i + 48 - j];
        }
    }
    window
}

/// `ff_mpa_synth_filter_float` for one block of 32 subband samples: `samples[at + i * incr]`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn synth_filter(
    synth_buf: &mut [f32; 1024],
    offset: &mut usize,
    window: &[f32],
    dither_state: &mut i32,
    samples: &mut [f32],
    at: usize,
    incr: usize,
    sb_samples: &[f32; 32],
) {
    let o = *offset;
    let mut tmp = [0.0f32; 32];
    dct32(&mut tmp, sb_samples);
    synth_buf[o..o + 32].copy_from_slice(&tmp);
    apply_window(synth_buf, o, window, dither_state, samples, at, incr);
    *offset = o.wrapping_sub(32) & 511;
}

fn apply_window(b: &mut [f32; 1024], o: usize, w: &[f32], dither_state: &mut i32, samples: &mut [f32], at: usize, incr: usize) {
    // copy to avoid wrap
    for i in 0..32 {
        b[o + 512 + i] = b[o + i];
    }
    let mut s_lo = at;
    let mut s_hi = at + 31 * incr;
    let mut wi = 0usize;
    let mut w2 = 31usize;
    let mut sum = *dither_state as f32;
    let mut p = o + 16;
    for k in 0..8 {
        sum += w[wi + k * 64] * b[p + k * 64];
    }
    p = o + 48;
    for k in 0..8 {
        sum -= w[wi + 32 + k * 64] * b[p + k * 64];
    }
    samples[s_lo] = sum;
    sum = 0.0;
    s_lo += incr;
    wi += 1;
    for j in 1..16 {
        let mut sum2 = 0.0f32;
        p = o + 16 + j;
        for k in 0..8 {
            let t = b[p + k * 64];
            sum += w[wi + k * 64] * t;
            sum2 -= w[w2 + k * 64] * t;
        }
        p = o + 48 - j;
        for k in 0..8 {
            let t = b[p + k * 64];
            sum -= w[wi + 32 + k * 64] * t;
            sum2 -= w[w2 + 32 + k * 64] * t;
        }
        samples[s_lo] = sum;
        sum = 0.0;
        s_lo += incr;
        sum += sum2;
        samples[s_hi] = sum;
        sum = 0.0;
        s_hi -= incr;
        wi += 1;
        w2 -= 1;
    }
    p = o + 32;
    for k in 0..8 {
        sum -= w[wi + 32 + k * 64] * b[p + k * 64];
    }
    samples[s_lo] = sum;
    *dither_state = sum as i32;
}

/// The inverse real DFT of `av_tx` (`AV_TX_FLOAT_RDFT`, inverse): `x` holds `m + 1` complex values
/// (the imaginary parts of the first and last are ignored), the result `2 m` real samples
/// scaled by `scale`.
pub(crate) fn inverse_rdft(x: &[(f32, f32)], scale: f32) -> Vec<f32> {
    let m = x.len() - 1;
    let n = 2 * m;
    // a half-size complex inverse FFT after the usual even/odd split
    let mut z: Vec<(f64, f64)> = vec![(0.0, 0.0); m];
    for k in 0..m {
        let (xr, xi) = (f64::from(x[k].0), f64::from(x[k].1));
        let (yr, yi) = (f64::from(x[m - k].0), -f64::from(x[m - k].1));
        let (xi, yi) = (if k == 0 { 0.0 } else { xi }, if k == 0 { 0.0 } else { yi });
        let (er, ei) = (xr + yr, xi + yi);
        let (dr, di) = (xr - yr, xi - yi);
        let ang = std::f64::consts::PI * k as f64 / m as f64;
        let (c, s) = (ang.cos(), ang.sin());
        // odd part rotated by e^{+i pi k / m}
        let (orr, oi) = (dr * c - di * s, dr * s + di * c);
        z[k] = (er - oi, ei + orr);
    }
    let z = fft_inverse(z);
    let sc = f64::from(scale);
    let mut out = vec![0.0f32; n];
    for (i, v) in z.iter().enumerate() {
        out[2 * i] = (v.0 * sc) as f32;
        out[2 * i + 1] = (v.1 * sc) as f32;
    }
    out
}

/// Unnormalised inverse FFT of a power-of-two length.
fn fft_inverse(mut a: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    let n = a.len();
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            a.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = 2.0 * std::f64::consts::PI / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (c, s) = ((ang * k as f64).cos(), (ang * k as f64).sin());
                let (ur, ui) = a[start + k];
                let (vr0, vi0) = a[start + k + len / 2];
                let (vr, vi) = (vr0 * c - vi0 * s, vr0 * s + vi0 * c);
                a[start + k] = (ur + vr, ui + vi);
                a[start + k + len / 2] = (ur - vr, ui - vi);
            }
        }
        len <<= 1;
    }
    a
}
