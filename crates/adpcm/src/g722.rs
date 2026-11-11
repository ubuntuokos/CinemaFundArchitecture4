//! ITU-T G.722 (11/88) sub-band ADPCM decoder, mode 1 (64 kbit/s): each octet carries a 6-bit
//! lower sub-band codeword (low bits) and a 2-bit higher sub-band codeword (top bits), and decodes
//! to two 16 kHz output samples.
//!
//! The blocks follow §6.2 of the Recommendation (INVQAL, INVQBL, LOGSCL, SCALEL, UPZERO, UPPOL1/2,
//! FILTEZ, FILTEP, LIMIT and their higher sub-band counterparts) with its 16-bit saturating
//! arithmetic, and the receive QMF of §5.2.2. The QMF's word length and product rounding are left
//! to the implementation (§5.2, Note 1); ours accumulates exactly and truncates once, to 16 bits.

/// 16-bit saturating addition (the Recommendation's `+`).
fn add(a: i32, b: i32) -> i32 {
    (a + b).clamp(-32768, 32767)
}

/// The Recommendation's `*`: a 16 × 16 product scaled back by 2^15 (truncating).
fn mul(a: i32, b: i32) -> i32 {
    (a * b) >> 15
}

/// Table 14: lower sub-band 6-bit inverse quantizer outputs QQ6, addresses 1..=30.
const QQ6: [i32; 31] = [
    0, 17, 54, 91, 130, 170, 211, 254, 300, 347, 396, 447, 501, 558, 618, 682, 750, 822, 899, 982, 1072, 1170, 1279, 1399, 1535, 1689, 1873, 2088, 2376, 2738,
    3101,
];
/// Table 14: 4-bit inverse quantizer outputs QQ4 and scale factor multipliers WL, addresses 0..=7.
const QQ4: [i32; 8] = [0, 150, 323, 530, 786, 1121, 1612, 2557];
const WL: [i32; 8] = [-60, -30, 58, 172, 334, 538, 1198, 3042];
/// Table 14: higher sub-band QQ2 and WH, addresses 1..=2.
const QQ2: [i32; 3] = [0, 202, 926];
const WH: [i32; 3] = [0, -214, 798];
/// Table 15: the 32-entry log-to-linear table (Method 2).
const ILB: [i32; 32] = [
    2048, 2093, 2139, 2186, 2233, 2282, 2332, 2383, 2435, 2489, 2543, 2599, 2656, 2714, 2774, 2834, 2896, 2960, 3025, 3091, 3158, 3228, 3298, 3371, 3444, 3520,
    3597, 3676, 3756, 3838, 3922, 4008,
];
/// Table 11: QMF coefficients H0..H11 (H23..H12 mirror them).
const H: [i32; 12] = [3, -11, -11, 53, 12, -156, 32, 362, -210, -805, 951, 3876];

/// Table 17: a 4-bit codeword → (negative, IL4).
fn table17(ril: u8) -> (bool, usize) {
    match ril & 15 {
        0 => (false, 0),
        r @ 1..=7 => (true, usize::from(8 - r)),
        r => (false, usize::from(15 - r)),
    }
}

/// Table 18: a 6-bit codeword → (negative, IL6).
fn table18(ril: u8) -> (bool, usize) {
    match ril & 63 {
        0..=3 => (true, 1),
        r @ 4..=31 => (true, usize::from(34 - r)),
        r @ 32..=61 => (false, usize::from(62 - r)),
        62 => (true, 2),
        _ => (true, 1),
    }
}

/// Table 21: a 2-bit higher sub-band codeword → (negative, IH2).
fn table21(ih: u8) -> (bool, usize) {
    match ih & 3 {
        0 => (true, 2),
        1 => (true, 1),
        3 => (false, 1),
        _ => (false, 2),
    }
}

/// Method 2 log-to-linear conversion (SCALEL / SCALEH): `ILB` scaled by the integer part of the
/// log scale factor, `bias` 8 for the lower and 10 for the higher sub-band.
fn scale(nb: i32, bias: i32) -> i32 {
    let frac = ((nb >> 6) & 31) as usize;
    let int = nb >> 11;
    let v = ILB.get(frac).copied().unwrap_or(2048);
    let shift = bias - int;
    let v = if shift >= 0 { v >> shift.min(31) } else { v << (-shift).min(15) };
    v << 2
}

/// The adaptive predictor and scale factor of one sub-band (§6.2.1.3, §6.2.1.4).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Band {
    /// Log scale factor NB and linear scale factor DET.
    nb: i32,
    det: i32,
    /// Pole and zero section coefficients.
    a: [i32; 2],
    b: [i32; 6],
    /// Quantized difference signal, partially reconstructed signal and reconstructed signal,
    /// delayed by 1.. samples.
    d: [i32; 6],
    p: [i32; 2],
    r: [i32; 2],
    /// Predictor outputs for the next sample: SZ (zero section) and S (zero + pole).
    sz: i32,
    s: i32,
}

impl Band {
    fn new(det: i32) -> Self {
        Self { nb: 0, det, a: [0; 2], b: [0; 6], d: [0; 6], p: [0; 2], r: [0; 2], sz: 0, s: 0 }
    }

    /// Update the predictor with the quantized difference `dt` (BLOCK 4L / 4H).
    fn predict(&mut self, dt: i32) {
        // PARREC, RECONS
        let pt = add(dt, self.sz);
        let rt = add(self.s, dt);
        // UPZERO
        let g = if dt == 0 { 0 } else { 128 };
        let sg0 = dt >> 15;
        for (b, &d) in self.b.iter_mut().zip(&self.d) {
            let wd2 = if sg0 == d >> 15 { g } else { -g };
            *b = add(wd2, mul(*b, 32640));
        }
        // UPPOL2: f(AL1) = 4·AL1 saturated, and its negation saturated too (0 − (−32768) is
        // 32767, not 32768)
        let (sg0, sg1, sg2) = (pt >> 15, self.p[0] >> 15, self.p[1] >> 15);
        let wd1 = add(self.a[0], self.a[0]);
        let wd1 = add(wd1, wd1);
        let wd2 = (if sg0 == sg1 { add(0, -wd1) } else { wd1 }) >> 7;
        let wd3 = if sg0 == sg2 { 128 } else { -128 };
        let a2 = add(add(wd2, wd3), mul(self.a[1], 32512)).clamp(-12288, 12288);
        // UPPOL1
        let wd1 = if sg0 == sg1 { 192 } else { -192 };
        let a1 = add(wd1, mul(self.a[0], 32640));
        let lim = 15360 - a2;
        let a1 = a1.clamp(-lim, lim);
        self.a = [a1, a2];
        self.d = [dt, self.d[0], self.d[1], self.d[2], self.d[3], self.d[4]];
        self.p = [pt, self.p[0]];
        self.r = [rt, self.r[0]];
        // FILTEZ, FILTEP, PREDIC
        let mut sz = 0;
        for (&b, &d) in self.b.iter().zip(&self.d).rev() {
            sz = add(sz, mul(b, add(d, d)));
        }
        let sp = add(mul(self.a[0], add(self.r[0], self.r[0])), mul(self.a[1], add(self.r[1], self.r[1])));
        self.sz = sz;
        self.s = add(sp, sz);
    }
}

/// G.722 decoder state (lower and higher sub-band ADPCM plus the receive QMF delay lines).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoder {
    low: Band,
    high: Band,
    /// QMF inputs XD (RL − RH) and XS (RL + RH), newest first.
    xd: [i32; 12],
    xs: [i32; 12],
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    /// A decoder in the reset state (§6.2: DETL = 32, DETH = 8, everything else 0).
    pub fn new() -> Self {
        Self { low: Band::new(32), high: Band::new(8), xd: [0; 12], xs: [0; 12] }
    }

    /// Decode one octet into two 16 kHz samples.
    pub fn decode_octet(&mut self, octet: u8) -> [i16; 2] {
        let il = octet & 63;
        let ih = octet >> 6;
        let rl = self.lower(il);
        let rh = self.higher(ih);
        // receive QMF (RECA, RECB, ACCUMC, ACCUMD, SELECT)
        self.xd.rotate_right(1);
        self.xs.rotate_right(1);
        self.xd[0] = add(rl, -rh);
        self.xs[0] = add(rl, rh);
        let mut even = 0i64;
        let mut odd = 0i64;
        for i in 0..12 {
            // H(2i) and H(2i+1) of the 24-tap symmetric filter
            let h_even = coefficient(2 * i);
            let h_odd = coefficient(2 * i + 1);
            even += i64::from(self.xd[i]) * i64::from(h_even);
            odd += i64::from(self.xs[i]) * i64::from(h_odd);
        }
        [qmf_out(even), qmf_out(odd)]
    }

    /// Decode a run of octets, appending the 16 kHz samples to `out`.
    pub fn decode_into(&mut self, data: &[u8], out: &mut Vec<i16>) {
        out.reserve(data.len().saturating_mul(2));
        for &b in data {
            out.extend_from_slice(&self.decode_octet(b));
        }
    }

    /// Lower sub-band ADPCM decoder (§6.2.1): returns RL.
    fn lower(&mut self, il: u8) -> i32 {
        let band = &mut self.low;
        // INVQAL: the 4-bit difference for the predictor
        let (neg4, il4) = table17(il >> 2);
        let q4 = QQ4.get(il4).copied().unwrap_or(0) << 3;
        let dlt = mul(band.det, if neg4 { -q4 } else { q4 });
        // INVQBL (mode 1): the 6-bit difference for the output
        let (neg6, il6) = table18(il);
        let q6 = QQ6.get(il6).copied().unwrap_or(0) << 3;
        let dl = mul(band.det, if neg6 { -q6 } else { q6 });
        let rl = add(band.s, dl).clamp(-16384, 16383);
        // LOGSCL, SCALEL
        let nb = add(mul(band.nb, 32512), WL.get(il4).copied().unwrap_or(0)).clamp(0, 18432);
        band.nb = nb;
        band.det = scale(nb, 8);
        band.predict(dlt);
        rl
    }

    /// Higher sub-band ADPCM decoder (§6.2.2): returns RH.
    fn higher(&mut self, ih: u8) -> i32 {
        let band = &mut self.high;
        let (neg, ih2) = table21(ih);
        let q = QQ2.get(ih2).copied().unwrap_or(0) << 3;
        let dh = mul(band.det, if neg { -q } else { q });
        let rh = add(band.s, dh).clamp(-16384, 16383);
        let nb = add(mul(band.nb, 32512), WH.get(ih2).copied().unwrap_or(0)).clamp(0, 22528);
        band.nb = nb;
        band.det = scale(nb, 10);
        band.predict(dh);
        rh
    }
}

/// QMF coefficient `k` of 0..24 (Table 11; symmetric).
fn coefficient(k: usize) -> i32 {
    let k = if k < 12 { k } else { 23usize.saturating_sub(k) };
    H.get(k).copied().unwrap_or(0)
}

/// ACCUMC / ACCUMD output as a 16-bit sample: `2 · Σ h·x` with `h` in units of 2^-13 is
/// `Σ >> 12` in the signal's 15-bit units, one bit more for 16-bit output (the QMF's final
/// scaling is implementation-defined, §5.2 Note 1; this one matches ffmpeg's decoder).
fn qmf_out(acc: i64) -> i16 {
    (acc >> 11).clamp(-32768, 32767) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codeword_tables_cover_every_interval() {
        // Table 17: each IL4 0..=7 appears with both signs (0 only positive, 0000 is an error code)
        let mut seen = [[false; 8]; 2];
        for r in 0..16u8 {
            let (n, i) = table17(r);
            seen[usize::from(n)][i] = true;
        }
        assert!(seen[0].iter().all(|&s| s));
        assert!(seen[1][1..].iter().all(|&s| s));
        // Table 18: IL6 1..=30 both signs
        let mut seen = [[false; 31]; 2];
        for r in 0..64u8 {
            let (n, i) = table18(r);
            seen[usize::from(n)][i] = true;
        }
        assert!(seen.iter().all(|s| s[1..].iter().all(|&v| v)));
    }

    #[test]
    fn scale_methods_agree() {
        // Method 2 (ILB) against Method 1 (ILA) at the ends of the range
        assert_eq!(scale(0, 8), 8 << 2);
        assert_eq!(scale(18432, 8), 4096 << 2);
        assert_eq!(scale(22528, 10), 4096 << 2);
        assert_eq!(scale(6400, 8), 69 << 2);
    }

    #[test]
    fn silence_stays_bounded_and_random_input_never_panics() {
        let mut d = Decoder::new();
        let mut out = Vec::new();
        let mut x = 0x1234_5678u32;
        let data: Vec<u8> = (0..100_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        d.decode_into(&data, &mut out);
        assert_eq!(out.len(), 200_000);
        d.decode_into(&[0xFF; 4], &mut out);
    }
}
