// After atracdenc 0.2.3 src/atrac/atrac_psy_common.{h,cpp} and src/atrac/atrac_scale.cpp
// (Copyright (c) Daniil Cherednik; LGPL-2.1-or-later). The absolute threshold of hearing table
// is the one atracdenc borrowed from Musepack (`ATHformula_Frank`, LGPL-2.1-or-later). See
// docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! Psychoacoustic helpers shared by the encoders: the threshold of hearing, the loudness curve
//! and tracker, the scale factor spread, block scaling and mantissa quantisation.

/// The absolute threshold of hearing in millibel, four steps per third from 10 Hz.
const ATH_TAB: [i16; 140] = [
    9669, 9669, 9626, 9512, 9353, 9113, 8882, 8676, 8469, 8243, 7997, 7748, 7492, 7239, 7000, 6762, 6529, 6302, 6084, 5900, 5717, 5534, 5351, 5167, 5004, 4812,
    4638, 4466, 4310, 4173, 4050, 3922, 3723, 3577, 3451, 3281, 3132, 3036, 2902, 2760, 2658, 2591, 2441, 2301, 2212, 2125, 2018, 1900, 1770, 1682, 1594, 1512,
    1430, 1341, 1260, 1198, 1136, 1057, 998, 943, 887, 846, 744, 712, 693, 668, 637, 606, 580, 555, 529, 502, 475, 448, 422, 398, 375, 351, 327, 322, 312, 301,
    291, 268, 246, 215, 182, 146, 107, 61, 13, -35, -96, -156, -179, -235, -295, -350, -401, -421, -446, -499, -532, -535, -513, -476, -431, -313, -179, 8,
    203, 403, 580, 736, 881, 1022, 1154, 1251, 1348, 1421, 1479, 1399, 1285, 1193, 1287, 1519, 1914, 2369, 3352, 4352, 5352, 6352, 7352, 8352, 9352, 9999,
    9999, 9999, 9999, 9999,
];

fn ath_frank(freq: f32) -> f32 {
    let freq = freq.clamp(10.0, 29853.0);
    let freq_log = 40.0 * (0.1 * f64::from(freq)).log10();
    let index = (freq_log as usize).min(ATH_TAB.len() - 2);
    (0.01 * (f64::from(ATH_TAB[index]) * (1.0 + index as f64 - freq_log) + f64::from(ATH_TAB[index + 1]) * (freq_log - index as f64))) as f32
}

/// The threshold of hearing (dB) of `len` lines of a spectrum of `sample_rate`.
pub(crate) fn calc_ath(len: usize, sample_rate: u32) -> Vec<f32> {
    let mf = sample_rate as f32 / 2000.0;
    (0..len)
        .map(|i| {
            let f = (i + 1) as f32 * mf / len as f32; // kHz
            let trh = ath_frank(1.0e3 * f) - 100.0;
            trh - f * f * 0.015
        })
        .collect()
}

/// A per-line weight of the spectrum's loudness.
pub(crate) fn loudness_curve(sz: usize) -> Vec<f32> {
    (0..sz)
        .map(|i| {
            let f = (i + 3) as f32 * 0.5 * 44100.0 / sz as f32;
            let t = f.log10() - 3.5;
            let t = -10.0 * t * t + 3.0 - f / 3000.0;
            10f32.powf(0.1 * t)
        })
        .collect()
}

pub(crate) fn track_loudness2(prev: f32, l0: f32, l1: f32) -> f32 {
    0.98 * prev + 0.01 * (l0 + l1)
}

pub(crate) fn track_loudness(prev: f32, l: f32) -> f32 {
    0.98 * prev + 0.02 * l
}

/// A scaled block of spectral lines: values in (-1, 1), the scale factor index and the
/// largest line energy.
#[derive(Clone, Debug, Default)]
pub(crate) struct ScaledBlock {
    pub(crate) sfi: u8,
    pub(crate) values: Vec<f32>,
    pub(crate) max_energy: f32,
}

/// 1 for tone-like, 0 for noise-like spectra.
pub(crate) fn scale_factor_spread(blocks: &[ScaledBlock]) -> f32 {
    if blocks.is_empty() {
        return 0.0;
    }
    let n = blocks.len() as f32;
    let s = blocks.iter().map(|b| f32::from(b.sfi)).sum::<f32>() / n;
    let sigma = (blocks.iter().map(|b| (f32::from(b.sfi) - s).powi(2)).sum::<f32>() / n).sqrt();
    sigma.min(14.0) / 14.0
}

/// Scales `input` by the smallest entry of `table` (increasing, 64 entries, last 1.0) that is at
/// least its peak.
pub(crate) fn scale(table: &[f32; 64], input: &[f32]) -> ScaledBlock {
    let peak = input.iter().fold(0f32, |m, v| m.max(v.abs())).min(1.0);
    let sfi = table.iter().position(|&t| t >= peak).unwrap_or(63);
    let sf = table[sfi];
    let mut max_energy = 0f32;
    let values = input
        .iter()
        .map(|&v| {
            max_energy = max_energy.max(v * v);
            let s = v / sf;
            if s.abs() >= 1.0 { if s > 0.0 { 0.99999 } else { -0.99999 } } else { s }
        })
        .collect();
    ScaledBlock { sfi: sfi as u8, values, max_energy }
}

/// atracdenc's `ToInt` (`lrint`, round half to even).
pub(crate) fn to_int(x: f32) -> i32 {
    x.round_ties_even() as i32
}

/// Quantises `input` with `mul` into `out`; with `ea`, rounds the lines nearest a half step the
/// other way where that brings the block's energy closer to the original. Returns the energy
/// ratio (original / quantised).
pub(crate) fn quant_mantissas(input: &[f32], mul: f32, ea: bool, out: &mut [i32]) -> f32 {
    let mut e1 = 0f32;
    let mut e2 = 0f32;
    let inv2 = 1.0 / (mul * mul);
    let mut candidates: Vec<(f32, usize)> = Vec::new();
    for (j, &v) in input.iter().enumerate() {
        let t = v * mul;
        e1 += v * v;
        out[j] = to_int(t);
        e2 += (out[j] * out[j]) as f32 * inv2;
        if ea {
            let delta = t - (t.trunc() + 0.5);
            if delta.abs() < 0.25 {
                candidates.push((delta, j));
            }
        }
    }
    if candidates.is_empty() {
        return e1 / e2;
    }
    candidates.sort_by(|a, b| a.0.abs().total_cmp(&b.0.abs()));
    if e2 < e1 {
        for &(_, j) in &candidates {
            let t = input[j] * mul;
            let a = out[j].abs() as f32;
            if a < t.abs() && a < mul - 1.0 {
                let mut m = out[j];
                if m > 0 {
                    m += 1;
                } else if m < 0 {
                    m -= 1;
                } else {
                    m = if t > 0.0 { 1 } else { -1 };
                }
                let ex = e2 - (out[j] * out[j]) as f32 * inv2 + (m * m) as f32 * inv2;
                if (ex - e1).abs() < (e2 - e1).abs() {
                    out[j] = m;
                    e2 = ex;
                }
            }
        }
    } else if e2 > e1 {
        for &(_, j) in &candidates {
            let t = input[j] * mul;
            if (out[j].abs() as f32) > t.abs() {
                let m = out[j] - out[j].signum();
                let ex = e2 - (out[j] * out[j]) as f32 * inv2 + (m * m) as f32 * inv2;
                if (ex - e1).abs() < (e2 - e1).abs() {
                    out[j] = m;
                    e2 = ex;
                }
            }
        }
    }
    e1 / e2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_is_half_even() {
        assert_eq!(to_int(0.5), 0);
        assert_eq!(to_int(1.5), 2);
        assert_eq!(to_int(-2.5), -2);
    }

    #[test]
    fn ath_is_lowest_near_3_khz() {
        let a = calc_ath(1024, 44100);
        let low = a[..1024].iter().enumerate().min_by(|x, y| x.1.total_cmp(y.1)).map(|(i, _)| i).unwrap_or(0);
        let f = (low + 1) as f32 * 22050.0 / 1024.0;
        assert!((2500.0..4500.0).contains(&f), "{f}");
    }
}
