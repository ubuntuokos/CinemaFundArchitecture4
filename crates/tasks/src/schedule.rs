//! Time zones for timed starts (ACS-4): a task started "at" a local time keeps that local time
//! across daylight-saving changes, and a missed start moves to its next slot in the task's own
//! IANA time zone (`Europe/Budapest`), not in UTC.
//!
//! A slot is the same local wall-clock time as the start that was missed, on the first day it is
//! still ahead. When that local time does not exist that day (the clocks jump forward over it) the
//! slot is the first moment after the gap; when it exists twice (the clocks go back), the first
//! one. Calendar time goes through chrono (owner decision 2026-11-03); the time zone rules come
//! from the IANA database bundled with `jiff` (only the UTC offset of a zone at an instant is read
//! from it; `chrono-tz` is unmaintained, RUSTSEC-2026-0555). An unknown zone is an error, never a
//! guess.

use chrono::{DateTime, NaiveDateTime, TimeDelta, Utc};
use jiff::tz::TimeZone as Zone;

use crate::queue::UtcMs;

/// Longest accepted time zone name.
pub const MAX_ZONE: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScheduleError {
    UnknownZone(String),
    /// A time outside what the calendar can represent.
    OutOfRange,
}

fn zone(name: &str) -> Result<Zone, ScheduleError> {
    if name.is_empty() || name.len() > MAX_ZONE {
        return Err(ScheduleError::UnknownZone(name.chars().take(MAX_ZONE).collect()));
    }
    jiff::tz::db().get(name).map_err(|_| ScheduleError::UnknownZone(name.to_owned()))
}

fn ts(ms: UtcMs) -> Result<DateTime<Utc>, ScheduleError> {
    let ms = i64::try_from(ms).map_err(|_| ScheduleError::OutOfRange)?;
    DateTime::from_timestamp_millis(ms).ok_or(ScheduleError::OutOfRange)
}

fn ms(t: DateTime<Utc>) -> Result<UtcMs, ScheduleError> {
    UtcMs::try_from(t.timestamp_millis()).map_err(|_| ScheduleError::OutOfRange)
}

/// Check that `name` is a time zone this build knows.
pub fn check_zone(name: &str) -> Result<(), ScheduleError> {
    zone(name).map(|_| ())
}

/// The UTC offset of `tz` at instant `t`, in seconds (local time minus UTC).
fn offset_at(tz: &Zone, t: DateTime<Utc>) -> Result<i64, ScheduleError> {
    let at = jiff::Timestamp::from_second(t.timestamp()).map_err(|_| ScheduleError::OutOfRange)?;
    Ok(i64::from(tz.to_offset(at).seconds()))
}

/// `local` read with `offset`: the instant whose local time it would be.
fn with_offset(local: NaiveDateTime, offset: i64) -> Result<DateTime<Utc>, ScheduleError> {
    let offset = TimeDelta::try_seconds(offset).ok_or(ScheduleError::OutOfRange)?;
    local.checked_sub_signed(offset).map(|t| t.and_utc()).ok_or(ScheduleError::OutOfRange)
}

/// The instant of a local date and time: the first one when it exists twice, and when it does
/// not exist (the clocks jump over it) the local time read with the offset from before the jump,
/// which lands as far after the gap's start as the gap is long (02:30 in a one-hour gap is
/// 03:30 local).
fn resolve(tz: &Zone, local: NaiveDateTime) -> Result<DateTime<Utc>, ScheduleError> {
    // a zone's offset changes at most once within a day either side of a local time, so the
    // offsets in force a day before and a day after are the only ones that can apply to it
    let day = TimeDelta::try_days(1).ok_or(ScheduleError::OutOfRange)?;
    let as_utc = local.and_utc();
    let before = offset_at(tz, as_utc.checked_sub_signed(day).ok_or(ScheduleError::OutOfRange)?)?;
    let after = offset_at(tz, as_utc.checked_add_signed(day).ok_or(ScheduleError::OutOfRange)?)?;
    let mut first: Option<DateTime<Utc>> = None;
    for offset in [before, after] {
        let t = with_offset(local, offset)?;
        if offset_at(tz, t)? == offset && first.is_none_or(|f| t < f) {
            first = Some(t);
        }
    }
    match first {
        Some(t) => Ok(t),
        // in a gap: read with the offset the clocks had before the jump
        None => with_offset(local, before),
    }
}

/// The next slot after `now` for a start planned at `scheduled_at`, in time zone `name`.
pub fn next_slot(scheduled_at: UtcMs, name: &str, now: UtcMs) -> Result<UtcMs, ScheduleError> {
    let tz = zone(name)?;
    let local_time = checked_local(ts(scheduled_at)?, &tz)?.time();
    let now_ts = ts(now)?;
    let mut day = checked_local(now_ts, &tz)?.date();
    // today, or tomorrow; one more day covers a slot that falls into a gap pushed past `now`
    for _ in 0..3 {
        let candidate = resolve(&tz, day.and_time(local_time))?;
        if candidate > now_ts {
            return ms(candidate);
        }
        day = day.succ_opt().ok_or(ScheduleError::OutOfRange)?;
    }
    Err(ScheduleError::OutOfRange)
}

/// The local date and time of an instant; a time too close to the calendar's ends is out of range.
fn checked_local(t: DateTime<Utc>, tz: &Zone) -> Result<NaiveDateTime, ScheduleError> {
    let offset = TimeDelta::try_seconds(offset_at(tz, t)?).ok_or(ScheduleError::OutOfRange)?;
    t.naive_utc().checked_add_signed(offset).ok_or(ScheduleError::OutOfRange)
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: u64 = 3_600_000;

    fn at(s: &str) -> UtcMs {
        u64::try_from(s.parse::<DateTime<Utc>>().unwrap().timestamp_millis()).unwrap()
    }

    #[test]
    fn a_missed_start_moves_to_the_same_local_time_on_the_next_day() {
        // planned 02:00 Budapest (01:00 UTC in winter), the machine woke at 09:00 local
        let planned = at("2026-11-12T01:00:00Z");
        assert_eq!(next_slot(planned, "Europe/Budapest", at("2026-11-12T08:00:00Z")), Ok(at("2026-11-13T01:00:00Z")));
        // woke before the slot on the same day: today
        assert_eq!(next_slot(planned, "Europe/Budapest", at("2026-11-12T00:30:00Z")), Ok(planned));
    }

    #[test]
    fn local_time_survives_daylight_saving_changes() {
        // 22:00 Budapest in summer (20:00 UTC) missed just before the October change: the next
        // slot is 22:00 winter time, 21:00 UTC, not 20:00 UTC
        let planned = at("2026-10-24T20:00:00Z");
        assert_eq!(next_slot(planned, "Europe/Budapest", at("2026-10-25T06:00:00Z")), Ok(at("2026-10-25T21:00:00Z")));
        // 02:30 does not exist on 2027-03-28 in Budapest: the first moment after the gap
        let planned = at("2027-03-27T01:30:00Z");
        let next = next_slot(planned, "Europe/Budapest", at("2027-03-27T12:00:00Z")).unwrap();
        assert_eq!(next, at("2027-03-28T01:30:00Z"), "03:30 summer time, one hour after the gap began");
        // 02:30 exists twice on 2026-10-25 in Budapest: the first one (summer time, 00:30 UTC)
        let planned = at("2026-10-24T00:30:00Z");
        assert_eq!(next_slot(planned, "Europe/Budapest", at("2026-10-24T12:00:00Z")), Ok(at("2026-10-25T00:30:00Z")));
        // a zone west of UTC, across its own (March) change: 08:00 New York stays 08:00
        let planned = at("2027-03-13T13:00:00Z");
        assert_eq!(next_slot(planned, "America/New_York", at("2027-03-13T14:00:00Z")), Ok(at("2027-03-14T12:00:00Z")));
        // UTC itself has no changes
        assert_eq!(next_slot(5 * H, "UTC", 30 * H), Ok(53 * H));
    }

    #[test]
    fn unknown_zones_and_hostile_times_are_errors() {
        assert!(matches!(next_slot(0, "Mars/Olympus_Mons", 1), Err(ScheduleError::UnknownZone(_))));
        assert!(matches!(next_slot(0, "", 1), Err(ScheduleError::UnknownZone(_))));
        assert!(matches!(next_slot(0, &"x".repeat(10_000), 1), Err(ScheduleError::UnknownZone(z)) if z.len() == MAX_ZONE));
        assert_eq!(next_slot(u64::MAX, "UTC", 0), Err(ScheduleError::OutOfRange));
        assert_eq!(next_slot(0, "UTC", u64::MAX), Err(ScheduleError::OutOfRange));
        assert_eq!(next_slot(i64::MAX as u64, "Europe/Budapest", 0), Err(ScheduleError::OutOfRange));
        assert!(check_zone("Europe/Budapest").is_ok());
    }
}
