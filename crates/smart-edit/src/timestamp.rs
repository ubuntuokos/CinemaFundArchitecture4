//! Exact media time: an integer count of a rational timebase (plan §5: no floating-point seconds
//! as timeline coordinates).

use serde::{Deserialize, Serialize};

use cfa4_time::{TICKS_PER_SECOND, Tick};

/// A timebase `num / den` seconds per unit (90 kHz MPEG: 1/90000).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rational {
    pub num: i64,
    pub den: i64,
}

impl Rational {
    pub const MPEG_90K: Rational = Rational { num: 1, den: 90_000 };

    pub fn new(num: i64, den: i64) -> Rational {
        Rational { num, den }
    }

    /// A usable timebase: both parts positive.
    pub fn is_valid(self) -> bool {
        self.num > 0 && self.den > 0
    }
}

/// A point in time on a stream's timebase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MediaTime {
    pub value: i64,
    pub timebase: Rational,
}

impl MediaTime {
    pub fn new(value: i64, timebase: Rational) -> MediaTime {
        MediaTime { value, timebase }
    }

    /// The time in CFA4 ticks, rounded toward negative infinity; `None` for an invalid timebase or
    /// a value outside the tick range.
    pub fn to_tick(self) -> Option<Tick> {
        if !self.timebase.is_valid() {
            return None;
        }
        let n = i128::from(self.value).checked_mul(i128::from(self.timebase.num))?.checked_mul(i128::from(TICKS_PER_SECOND))?;
        let t = n.div_euclid(i128::from(self.timebase.den));
        let t = i64::try_from(t).ok()?;
        (t.unsigned_abs() <= Tick::MAX.0.unsigned_abs()).then_some(Tick(t))
    }
}
