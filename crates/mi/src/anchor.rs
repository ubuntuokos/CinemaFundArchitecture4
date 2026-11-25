//! Time anchors: a half-open range `[start, end)` of exact [`Tick`]s inside one media item.
//!
//! Never floating-point seconds (CLAUDE.md: exact time). The operators are the time relations the
//! query planner needs (`BEFORE`, `AFTER`, `OVERLAPS`, `WITHIN`).

use cfa4_time::Tick;
use serde::{Deserialize, Serialize};

use crate::error::MiError;

/// A non-empty, non-negative range of media time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TimeAnchor {
    start: Tick,
    end: Tick,
}

impl TimeAnchor {
    /// `[start, end)`; `start` must be at least zero and strictly before `end`.
    pub fn new(start: Tick, end: Tick) -> Result<Self, MiError> {
        if start.0 < 0 {
            return Err(MiError::InvalidAnchor("start is negative"));
        }
        if end.0 <= start.0 {
            return Err(MiError::InvalidAnchor("end is not after start"));
        }
        Ok(Self { start, end })
    }

    pub fn start(&self) -> Tick {
        self.start
    }

    pub fn end(&self) -> Tick {
        self.end
    }

    /// Length in ticks (always positive; cannot overflow because both bounds are non-negative).
    pub fn duration(&self) -> i64 {
        self.end.0 - self.start.0
    }

    /// The two ranges share at least one tick.
    pub fn overlaps(&self, other: &Self) -> bool {
        self.start.0 < other.end.0 && other.start.0 < self.end.0
    }

    /// `self` lies completely inside `other`.
    pub fn within(&self, other: &Self) -> bool {
        other.start.0 <= self.start.0 && self.end.0 <= other.end.0
    }

    /// `self` ends no later than `other` starts.
    pub fn before(&self, other: &Self) -> bool {
        self.end.0 <= other.start.0
    }

    /// `self` starts no earlier than `other` ends.
    pub fn after(&self, other: &Self) -> bool {
        other.end.0 <= self.start.0
    }

    /// `self` ends at most `gap` ticks before `other` starts (a `BEFORE ... WITHIN gap` relation).
    /// A negative `gap` is never satisfied. No overflow for any input.
    pub fn before_within(&self, other: &Self, gap: i64) -> bool {
        self.before(other) && gap >= 0 && other.start.0.saturating_sub(self.end.0) <= gap
    }

    /// The common part of two ranges, if any.
    pub fn intersection(&self, other: &Self) -> Option<Self> {
        let start = self.start.0.max(other.start.0);
        let end = self.end.0.min(other.end.0);
        (start < end).then_some(Self { start: Tick(start), end: Tick(end) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: i64, e: i64) -> TimeAnchor {
        TimeAnchor::new(Tick(s), Tick(e)).unwrap()
    }

    #[test]
    fn rejects_empty_reversed_and_negative() {
        assert!(TimeAnchor::new(Tick(5), Tick(5)).is_err());
        assert!(TimeAnchor::new(Tick(6), Tick(5)).is_err());
        assert!(TimeAnchor::new(Tick(-1), Tick(5)).is_err());
        assert!(TimeAnchor::new(Tick(i64::MIN), Tick(i64::MAX)).is_err());
        assert!(TimeAnchor::new(Tick(0), Tick(i64::MAX)).is_ok());
    }

    #[test]
    fn half_open_ranges_touching_do_not_overlap() {
        assert!(!a(0, 10).overlaps(&a(10, 20)));
        assert!(a(0, 11).overlaps(&a(10, 20)));
        assert!(a(0, 10).before(&a(10, 20)));
        assert!(a(10, 20).after(&a(0, 10)));
    }

    #[test]
    fn within_and_intersection() {
        assert!(a(2, 5).within(&a(0, 10)));
        assert!(!a(2, 11).within(&a(0, 10)));
        assert_eq!(a(0, 10).intersection(&a(5, 20)), Some(a(5, 10)));
        assert_eq!(a(0, 10).intersection(&a(10, 20)), None);
    }

    #[test]
    fn before_within_gap() {
        assert!(a(0, 10).before_within(&a(15, 20), 5));
        assert!(!a(0, 10).before_within(&a(16, 20), 5));
        assert!(!a(0, 10).before_within(&a(15, 20), -1));
        assert!(!a(5, 15).before_within(&a(10, 20), 100));
        assert!(a(0, 10).before_within(&TimeAnchor::new(Tick(i64::MAX - 1), Tick(i64::MAX)).unwrap(), i64::MAX));
    }

    #[test]
    fn duration_is_exact_at_the_extremes() {
        assert_eq!(a(0, i64::MAX).duration(), i64::MAX);
    }
}
