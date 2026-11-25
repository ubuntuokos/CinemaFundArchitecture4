// After FFmpeg n7.1.1 libavutil/lfg.c and lfg.h (av_lfg_get, av_bmg_get), LGPL-2.1-or-later;
// see docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! FFmpeg's lagged Fibonacci generator (`AVLFG`), started from a precomputed `av_lfg_init`
//! state.

#[derive(Clone)]
pub(crate) struct Lfg {
    pub(crate) state: [u32; 64],
    pub(crate) index: u32,
}

impl Lfg {
    pub(crate) fn new(state: [u32; 64]) -> Self {
        Self { state, index: 0 }
    }

    /// `av_lfg_get`.
    pub(crate) fn get(&mut self) -> u32 {
        let i = self.index;
        let a = self.state[(i.wrapping_sub(24) & 63) as usize].wrapping_add(self.state[(i.wrapping_sub(55) & 63) as usize]);
        self.state[(i & 63) as usize] = a;
        self.index = i.wrapping_add(1);
        a
    }

    /// `av_bmg_get`: two normally distributed values (Box-Muller, polar form).
    pub(crate) fn bmg(&mut self) -> [f64; 2] {
        loop {
            let x1 = 2.0 / f64::from(u32::MAX) * f64::from(self.get()) - 1.0;
            let x2 = 2.0 / f64::from(u32::MAX) * f64::from(self.get()) - 1.0;
            let w = x1 * x1 + x2 * x2;
            if w < 1.0 {
                let w = ((-2.0 * w.ln()) / w).sqrt();
                return [x1 * w, x2 * w];
            }
        }
    }
}
