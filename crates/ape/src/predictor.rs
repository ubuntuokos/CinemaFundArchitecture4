//! The version 3.95+ prediction stages (`NewPredictor.cpp`, `NNFilter*.cpp`, `Interim.cpp`,
//! `ScaledFirstOrderFilter.h`, `RollBuffer.h` of the Monkey's Audio SDK).
//!
//! The reference decoder instantiates these for `int` (8- to 24-bit audio, with `short` filter
//! data) and for `int64` (32-bit audio, with `int` filter data); [`Int`] models both, with the
//! reference's two's-complement wrap-around. "Interim" mode decodes the 24-bit files written by
//! Monkey's Audio 5.01–8.51, whose encoder summed some products in 64 bits.

/// The integer type a predictor works in (`INTTYPE`).
pub(crate) trait Int: Copy + PartialOrd + Default {
    /// `int64` (32-bit audio) rather than `int`.
    const WIDE: bool;
    const ZERO: Self;
    fn from_i64(v: i64) -> Self;
    fn to_i64(self) -> i64;
    fn add(self, o: Self) -> Self;
    fn sub(self, o: Self) -> Self;
    fn mul(self, o: Self) -> Self;
    fn div(self, o: Self) -> Self;
    fn shr(self, n: u32) -> Self;
    fn and(self, o: Self) -> Self;
    fn abs(self) -> Self;
}

macro_rules! int_impl {
    ($t:ty, $wide:expr) => {
        impl Int for $t {
            const WIDE: bool = $wide;
            const ZERO: Self = 0;
            fn from_i64(v: i64) -> Self {
                v as $t
            }
            fn to_i64(self) -> i64 {
                i64::from(self)
            }
            fn add(self, o: Self) -> Self {
                self.wrapping_add(o)
            }
            fn sub(self, o: Self) -> Self {
                self.wrapping_sub(o)
            }
            fn mul(self, o: Self) -> Self {
                self.wrapping_mul(o)
            }
            fn div(self, o: Self) -> Self {
                self.checked_div(o).unwrap_or(0)
            }
            fn shr(self, n: u32) -> Self {
                self >> n
            }
            fn and(self, o: Self) -> Self {
                self & o
            }
            fn abs(self) -> Self {
                self.wrapping_abs()
            }
        }
    };
}

int_impl!(i32, false);
int_impl!(i64, true);

fn c<T: Int>(v: i64) -> T {
    T::from_i64(v)
}

/// `GetSaturatedShortFromInt`.
fn saturate_short(v: i64) -> i32 {
    v.clamp(-32768, 32767) as i32
}

/// The filter data type (`DATATYPE`): `short` for narrow predictors, `int` for wide ones.
fn data<T: Int>(v: i64) -> i32 {
    if T::WIDE { v as i32 } else { i32::from(v as i16) }
}

/// `CRollBuffer`: a window of `window` values over a history of `history`, indexed relative to
/// the current position.
#[derive(Debug, Clone)]
struct Roll<V> {
    data: Vec<V>,
    history: usize,
    cur: usize,
}

impl<V: Copy + Default> Roll<V> {
    fn new(window: usize, history: usize) -> Self {
        Self { data: vec![V::default(); window + history], history, cur: history }
    }
    fn flush(&mut self) {
        for v in self.data.iter_mut().take(self.history + 1) {
            *v = V::default();
        }
        self.cur = self.history;
    }
    fn roll(&mut self) {
        let start = self.cur.saturating_sub(self.history);
        if self.cur <= self.data.len() {
            self.data.copy_within(start..self.cur, 0);
        }
        self.cur = self.history;
    }
    /// `IncrementSafe`.
    fn advance_safe(&mut self) {
        self.cur += 1;
        if self.cur == self.data.len() {
            self.roll();
        }
    }
    /// `IncrementFast` (the owner rolls before the window ends).
    fn advance(&mut self) {
        self.cur += 1;
    }
    /// The value `back` positions before the current one (0 is the current).
    fn at(&self, back: usize) -> V {
        self.cur.checked_sub(back).and_then(|i| self.data.get(i)).copied().unwrap_or_default()
    }
    fn set(&mut self, back: usize, v: V) {
        if let Some(slot) = self.cur.checked_sub(back).and_then(|i| self.data.get_mut(i)) {
            *slot = v;
        }
    }
    /// The `n` values before the current one, oldest first.
    fn before(&self, n: usize) -> &[V] {
        self.cur.checked_sub(n).and_then(|start| self.data.get(start..self.cur)).unwrap_or(&[])
    }
}

const NN_WINDOW: usize = 512;

/// `CNNFilter` (decoder side).
#[derive(Debug, Clone)]
struct NnFilter<T: Int> {
    order: usize,
    shift: u32,
    round: i64,
    interim: bool,
    m: Vec<i32>,
    input: Roll<i32>,
    delta: Roll<i32>,
    running_average: T,
}

impl<T: Int> NnFilter<T> {
    fn new(order: usize, shift: u32) -> Self {
        Self {
            order,
            shift,
            round: 1 << (shift - 1),
            interim: false,
            m: vec![0; order],
            input: Roll::new(NN_WINDOW, order),
            delta: Roll::new(NN_WINDOW, order),
            running_average: T::ZERO,
        }
    }

    fn flush(&mut self) {
        self.m.iter_mut().for_each(|v| *v = 0);
        self.input.flush();
        self.delta.flush();
        self.running_average = T::ZERO;
    }

    fn dot(&self) -> T {
        let pairs = self.input.before(self.order).iter().zip(&self.m);
        if T::WIDE {
            // int products wrap at 32 bits, the sum is int64
            c(pairs.fold(0i64, |acc, (&a, &b)| acc.wrapping_add(i64::from(a.wrapping_mul(b)))))
        } else {
            // short products summed in int
            c(i64::from(pairs.fold(0i32, |acc, (&a, &b)| acc.wrapping_add(a.wrapping_mul(b)))))
        }
    }

    fn decompress(&mut self, input: T) -> T {
        let dot = self.dot();
        let output: T = if self.interim {
            c(input.to_i64().wrapping_add(dot.to_i64().wrapping_add(self.round) >> self.shift))
        } else {
            input.add(dot.add(c(self.round)).shr(self.shift))
        };
        // Adapt: M += deltaM for a negative input, M -= deltaM for a positive one.
        let zero = T::ZERO;
        if input != zero {
            let negative = input < zero;
            let deltas = self.delta.before(self.order);
            for (m, &d) in self.m.iter_mut().zip(deltas) {
                let v = if negative { i64::from(*m) + i64::from(d) } else { i64::from(*m) - i64::from(d) };
                *m = data::<T>(v);
            }
        }
        // APE_UPDATE_DELTA_NEW (every 3.98+ stream)
        let abs = output.abs();
        let avg = self.running_average;
        let d: T = if abs > avg.mul(c(3)) {
            output.shr(25).and(c(64)).sub(c(32))
        } else if abs > avg.mul(c(4)).div(c(3)) {
            output.shr(26).and(c(32)).sub(c(16))
        } else if abs > zero {
            output.shr(27).and(c(16)).sub(c(8))
        } else {
            zero
        };
        self.delta.set(0, data::<T>(d.to_i64()));
        self.running_average = avg.add(abs.sub(avg).div(c(16)));
        for back in [1, 2, 8] {
            let v = self.delta.at(back);
            self.delta.set(back, data::<T>(i64::from(v >> 1)));
        }
        self.input.set(0, saturate_short(output.to_i64()));
        self.input.advance_safe();
        self.delta.advance_safe();
        output
    }
}

/// `CScaledFirstOrderFilter<INTTYPE, 31, 5>`.
#[derive(Debug, Clone, Copy, Default)]
struct FirstOrder {
    last: i32,
}

impl FirstOrder {
    fn compress<T: Int>(&mut self, input: i32) -> T {
        let r = c::<T>(i64::from(input)).sub(c::<T>(i64::from(self.last)).mul(c(31)).shr(5));
        self.last = input;
        r
    }
    fn decompress<T: Int>(&mut self, input: T) -> i32 {
        self.last = input.add(c::<T>(i64::from(self.last)).mul(c(31)).shr(5)).to_i64() as i32;
        self.last
    }
}

const WINDOW_BLOCKS: usize = 256;
const HISTORY: usize = 8;

/// `CPredictorDecompress3950toCurrent` (and `CPredictorDecompressInterim`).
#[derive(Debug, Clone)]
pub(crate) struct Predictor<T: Int> {
    filters: Vec<NnFilter<T>>,
    bits: u32,
    interim: bool,
    pred_a: Roll<T>,
    pred_b: Roll<T>,
    adapt_a: Roll<T>,
    adapt_b: Roll<T>,
    stage1_a: FirstOrder,
    stage1_b: FirstOrder,
    m_a: [T; 4],
    m_b: [T; 5],
    last_a: T,
    index: usize,
}

/// The NN filter cascade of a compression level, as (order, shift) from the first applied to
/// the last; `None` for an unknown level.
pub(crate) fn level_filters(level: u16) -> Option<&'static [(usize, u32)]> {
    Some(match level {
        1000 => &[],
        2000 => &[(16, 11)],
        3000 => &[(64, 11)],
        4000 => &[(256, 13), (32, 10)],
        5000 => &[(1024 + 256, 15), (256, 13), (16, 11)],
        _ => return None,
    })
}

impl<T: Int> Predictor<T> {
    pub(crate) fn new(filters: &[(usize, u32)], bits: u32) -> Self {
        // DecompressValue applies the smallest (last created) filter first.
        let filters = filters.iter().rev().map(|&(order, shift)| NnFilter::new(order, shift)).collect();
        let mut p = Self {
            filters,
            bits,
            interim: false,
            pred_a: Roll::new(WINDOW_BLOCKS, HISTORY),
            pred_b: Roll::new(WINDOW_BLOCKS, HISTORY),
            adapt_a: Roll::new(WINDOW_BLOCKS, HISTORY),
            adapt_b: Roll::new(WINDOW_BLOCKS, HISTORY),
            stage1_a: FirstOrder::default(),
            stage1_b: FirstOrder::default(),
            m_a: [T::ZERO; 4],
            m_b: [T::ZERO; 5],
            last_a: T::ZERO,
            index: 0,
        };
        p.flush();
        p
    }

    /// Switch to the Monkey's Audio 5.01–8.51 arithmetic for 24-bit streams.
    pub(crate) fn set_interim(&mut self) {
        self.interim = true;
        for f in &mut self.filters {
            f.interim = true;
        }
    }

    /// `Flush`, at the start of every frame.
    pub(crate) fn flush(&mut self) {
        for f in &mut self.filters {
            f.flush();
        }
        self.m_a = [c(360), c(317), c(-109), c(98)];
        self.m_b = [T::ZERO; 5];
        self.pred_a.flush();
        self.pred_b.flush();
        self.adapt_a.flush();
        self.adapt_b.flush();
        self.stage1_a = FirstOrder::default();
        self.stage1_b = FirstOrder::default();
        self.last_a = T::ZERO;
        self.index = 0;
    }

    fn sign_adapt(v: T) -> T {
        if v == T::ZERO { T::ZERO } else { v.shr(30).and(c(2)).sub(c(1)) }
    }

    /// `DecompressValue(nA, nB)`.
    pub(crate) fn decompress(&mut self, a: i64, b: i64) -> i32 {
        if self.index == WINDOW_BLOCKS {
            self.pred_a.roll();
            self.pred_b.roll();
            self.adapt_a.roll();
            self.adapt_b.roll();
            self.index = 0;
        }
        let mut na: T = c(a);
        let nb: T = c(b);
        for f in &mut self.filters {
            na = f.decompress(na);
        }

        self.pred_a.set(0, self.last_a);
        self.pred_a.set(1, self.pred_a.at(0).sub(self.pred_a.at(1)));
        let b0: T = self.stage1_b.compress(nb.to_i64() as i32);
        self.pred_b.set(0, b0);
        self.pred_b.set(1, self.pred_b.at(0).sub(self.pred_b.at(1)));

        let current_a: T = if self.bits <= 16 || T::WIDE {
            let pa = (0..4).fold(T::ZERO, |s, i| s.add(self.pred_a.at(i).mul(self.m_a[i])));
            let pb = (0..5).fold(T::ZERO, |s, i| s.add(self.pred_b.at(i).mul(self.m_b[i])));
            na.add(pa.add(pb.shr(1)).shr(10))
        } else {
            let pa = (0..4).fold(0i64, |s, i| s.wrapping_add(self.pred_a.at(i).to_i64().wrapping_mul(self.m_a[i].to_i64())));
            let pb = (0..5).fold(0i64, |s, i| s.wrapping_add(self.pred_b.at(i).to_i64().wrapping_mul(self.m_b[i].to_i64())));
            if self.interim { na.add(c(pa.wrapping_add(pb >> 1) >> 10)) } else { na.add(c(i64::from((pa as i32).wrapping_add((pb as i32) >> 1) >> 10))) }
        };

        self.adapt_a.set(0, Self::sign_adapt(self.pred_a.at(0)));
        self.adapt_a.set(1, Self::sign_adapt(self.pred_a.at(1)));
        self.adapt_b.set(0, Self::sign_adapt(self.pred_b.at(0)));
        self.adapt_b.set(1, Self::sign_adapt(self.pred_b.at(1)));

        let direction: T = c(i64::from(na < T::ZERO) - i64::from(na > T::ZERO));
        for i in 0..4 {
            self.m_a[i] = self.m_a[i].add(self.adapt_a.at(i).mul(direction));
        }
        for i in 0..5 {
            self.m_b[i] = self.m_b[i].add(self.adapt_b.at(i).mul(direction));
        }

        let result = self.stage1_a.decompress(current_a);
        self.last_a = current_a;
        self.pred_a.advance();
        self.pred_b.advance();
        self.adapt_a.advance();
        self.adapt_b.advance();
        self.index += 1;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roll_buffer_keeps_its_history_across_a_roll() {
        let mut r: Roll<i32> = Roll::new(4, 2);
        for v in 1..=10 {
            r.set(0, v);
            r.advance_safe();
        }
        assert_eq!(r.before(2), &[9, 10]);
        assert_eq!(r.at(1), 10);
    }

    #[test]
    fn fast_level_is_the_two_stage_predictor_only() {
        // A zero residual stream decodes to zero.
        let mut p: Predictor<i32> = Predictor::new(&[], 16);
        for _ in 0..1000 {
            assert_eq!(p.decompress(0, 0), 0);
        }
        let mut w: Predictor<i64> = Predictor::new(level_filters(5000).unwrap_or(&[]), 32);
        for _ in 0..1000 {
            assert_eq!(w.decompress(0, 0), 0);
        }
    }

    #[test]
    fn saturation_and_short_wrap() {
        assert_eq!(saturate_short(40000), 32767);
        assert_eq!(saturate_short(-40000), -32768);
        assert_eq!(data::<i32>(32768), -32768);
        assert_eq!(data::<i64>(32768), 32768);
    }
}
