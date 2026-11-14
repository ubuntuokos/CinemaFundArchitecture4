// After atracdenc 0.2.3 src/qmf/qmf.h and src/atrac/atrac3_qmf.h (Copyright (c) Daniil
// Cherednik; LGPL-2.1-or-later); see docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! The 48-tap two-band QMF analysis and ATRAC3's four-band tree of them.

#[allow(clippy::excessive_precision)] // atracdenc's literals, kept as published
const TAP_HALF: [f32; 24] = [
    -0.000_014_619_07,
    -0.000_092_054_79,
    -0.000_056_157_569,
    0.000_301_172_69,
    0.000_242_251_9,
    -0.000_852_938_97,
    -0.000_520_557_4,
    0.002_034_016_9,
    0.000_783_338_91,
    -0.004_215_386_2,
    -0.000_756_149_88,
    0.007_840_294_4,
    -0.000_061_169_922,
    -0.013_441_62,
    0.002_462_682_1,
    0.021_736_089,
    -0.007_801_671,
    -0.034_090_221,
    0.018_809_49,
    0.054_326_009,
    -0.043_596_379,
    -0.099_384_367,
    0.132_079_09,
    0.464_241_59,
];

/// One two-band analysis stage for blocks of `n` input samples.
#[derive(Clone)]
pub(crate) struct Qmf {
    n: usize,
    window: [f32; 48],
    pcm: Vec<f32>,
}

impl Qmf {
    pub(crate) fn new(n: usize) -> Self {
        let mut window = [0f32; 48];
        for i in 0..24 {
            window[i] = TAP_HALF[i] * 2.0;
            window[47 - i] = TAP_HALF[i] * 2.0;
        }
        Self { n, window, pcm: vec![0.0; n + 46] }
    }

    /// `input` (n samples) → `lower`, `upper` (n/2 each).
    pub(crate) fn analysis(&mut self, input: &[f32], lower: &mut [f32], upper: &mut [f32]) {
        let n = self.n;
        self.pcm.copy_within(n..n + 46, 0);
        self.pcm[46..46 + n].copy_from_slice(&input[..n]);
        for j in (0..n).step_by(2) {
            let (mut lo, mut up) = (0f32, 0f32);
            for i in 0..24 {
                lo += self.window[2 * i] * self.pcm[48 - 1 + j - 2 * i];
                up += self.window[2 * i + 1] * self.pcm[48 - 1 + j - 2 * i - 1];
            }
            lower[j / 2] = lo + up;
            upper[j / 2] = lo - up;
        }
    }
}

/// ATRAC3's four bands of 256 samples from 1024: (0, 1) from the low half, (3, 2) from the high.
#[derive(Clone)]
pub(crate) struct Atrac3Analysis {
    q1: Qmf,
    q2: Qmf,
    q3: Qmf,
    buf1: Vec<f32>,
    buf2: Vec<f32>,
}

impl Atrac3Analysis {
    pub(crate) fn new() -> Self {
        Self { q1: Qmf::new(1024), q2: Qmf::new(512), q3: Qmf::new(512), buf1: vec![0.0; 512], buf2: vec![0.0; 512] }
    }

    pub(crate) fn analysis(&mut self, pcm: &[f32], bands: &mut [[f32; 256]; 4]) {
        self.q1.analysis(pcm, &mut self.buf1, &mut self.buf2);
        let [b0, b1, b2, b3] = bands;
        self.q2.analysis(&self.buf1, b0, b1);
        self.q3.analysis(&self.buf2, b3, b2);
    }
}
