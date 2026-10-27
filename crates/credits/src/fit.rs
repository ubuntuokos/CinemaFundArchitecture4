//! Fit credits to a length – usually the length of their music.
//!
//! The credits' total length is the sum of the pages and the gaps between them. Fitting keeps
//! what has a fixed length – cards, pages with their own `Page Runtime`, page gaps – and shares
//! the rest among the scroll pages in proportion to their natural length. Each scroll page then
//! reaches its share by stretching or squeezing the gaps between its blocks at the page style's
//! speed (see the layout); only if that is impossible does its speed change, with a warning.
//!
//! With `card_beat`, cards without their own runtime last a whole number of beats (at least
//! one), and the scroll page before a card is lengthened or shortened by up to half a beat so the
//! card starts on a beat (`beat_offset` says where the beats fall); the last scroll page takes
//! back the difference. Cards after the last scroll page are on the beat when the length is (the
//! last bar line, for example).
//!
//! The setting is part of the credits document ([`Fit`]), so a refresh from the spreadsheet keeps
//! the credits fitted.

use serde::{Deserialize, Serialize};

use cfa4_time::Tick;

use crate::layout::{Frame, Laid, Measure, layout};
use crate::sheet::{TimeSpec, Warning};
use crate::{Behaviour, Credits, CreditsError, Styling};

/// How credits are fitted (stored in the credits document).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Fit {
    /// Total length, page gaps included (ticks).
    pub total: i64,
    /// Cards last a whole number of these (ticks): the music's beat. 0 = off.
    #[serde(skip_serializing_if = "is_zero")]
    pub card_beat: i64,
    /// The music it was fitted to, for the user's information (clip name).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub music: String,
    /// Where the beats fall: the time from the credits' start to a beat (ticks, less than
    /// `card_beat`). Cards then start and end on beats, not just last whole beats.
    #[serde(skip_serializing_if = "is_zero")]
    pub beat_offset: i64,
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

/// What fitting did.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FitReport {
    /// Length without fitting.
    pub natural: i64,
    /// Asked length, to the nearest frame.
    pub target: i64,
    /// Length reached.
    pub result: i64,
    /// Pages that were stretched or squeezed.
    pub scroll_pages: usize,
}

/// Total length of laid-out pages, page gaps included (the last page's gap does not count).
pub fn total_length(l: &Laid) -> Tick {
    let n = l.pages.len();
    let mut t = 0i64;
    for (i, p) in l.pages.iter().enumerate() {
        t = t.saturating_add(p.duration.0);
        if i + 1 < n {
            t = t.saturating_add(p.gap_after.0);
        }
    }
    Tick(t.max(0))
}

/// Lay out `credits` fitted to `fit`. Returns the credits as fitted (scroll pages and cards get
/// explicit runtimes), their layout and a report.
pub fn fit_layout(credits: &Credits, styling: &Styling, frame: Frame, measure: &dyn Measure, fit: &Fit) -> Result<(Credits, Laid, FitReport), CreditsError> {
    let natural = layout(credits, styling, frame, measure)?;
    let natural_total = total_length(&natural);
    let fd = frame.rate.frame_duration().0.max(1);
    let mut fitted = credits.clone();
    let mut warnings: Vec<Warning> = Vec::new();

    // cards on the beat
    if fit.card_beat > 0 {
        for (p, out) in fitted.pages.iter_mut().zip(&natural.pages) {
            if out.behaviour == Behaviour::Card && p.runtime.is_none() {
                let beats = ((out.duration.0 as f64 / fit.card_beat as f64).round() as i64).max(1);
                let t = frame.rate.snap_nearest(Tick(fit.card_beat.saturating_mul(beats)));
                p.runtime = Some(TimeSpec::Ticks(t.max(Tick(fd))));
            }
        }
    }
    let staged = layout(&fitted, styling, frame, measure)?;

    // what the scroll pages without their own runtime have to fill
    let flexible: Vec<usize> = staged
        .pages
        .iter()
        .enumerate()
        .filter(|(i, p)| p.behaviour == Behaviour::Scroll && credits.pages.get(*i).is_some_and(|c| c.runtime.is_none()))
        .map(|(i, _)| i)
        .collect();
    let fixed: i64 = total_length(&staged).0 - flexible.iter().filter_map(|i| staged.pages.get(*i)).map(|p| p.duration.0).sum::<i64>();
    // lengths are whole frames
    let target = frame.rate.snap_nearest(Tick(fit.total.max(0))).0;
    let share = target.saturating_sub(fixed);
    let natural_flex: i64 = flexible.iter().filter_map(|i| staged.pages.get(*i)).map(|p| p.duration.0).sum();
    if flexible.is_empty() {
        warnings.push(Warning { row: 0, message: "there is no scroll page without its own runtime to stretch; the length cannot change".into() });
    } else if share < fd.saturating_mul(flexible.len() as i64) {
        warnings.push(Warning {
            row: 0,
            message: format!(
                "the cards and gaps alone are {:.1} s; the asked length is shorter, so the scroll pages keep their natural length",
                fixed as f64 / cfa4_time::TICKS_PER_SECOND as f64
            ),
        });
    } else if natural_flex > 0 {
        // shares in whole frames; every page keeps at least one, the last takes the rounding
        let frames_total = share / fd;
        let n = flexible.len() as i64;
        let mut left = frames_total;
        let mut alloc: Vec<(usize, i64)> = Vec::with_capacity(flexible.len());
        for (k, i) in flexible.iter().enumerate() {
            let Some(p) = staged.pages.get(*i) else { continue };
            let after = n - k as i64 - 1;
            let want = if after == 0 { left } else { ((p.duration.0 as f64 / natural_flex as f64) * frames_total as f64).round() as i64 };
            // leave a frame for each page still to come
            let frames = want.min(left - after).max(1);
            left -= frames;
            alloc.push((*i, frames));
        }
        if fit.card_beat > 0 {
            cards_on_beats(&staged, &flexible, &mut alloc, fit, fd);
        }
        for (i, frames) in alloc {
            if let Some(page) = fitted.pages.get_mut(i) {
                page.runtime = Some(TimeSpec::Frames(frames));
            }
        }
    }
    let mut out = layout(&fitted, styling, frame, measure)?;
    out.warnings.extend(warnings);
    let result = total_length(&out);
    let report = FitReport { natural: natural_total.0, target, result: result.0, scroll_pages: flexible.len() };
    Ok((fitted, out, report))
}

/// Move each card (laid out in `staged`) that a scroll page comes before onto a beat: the closest
/// scroll page before it gets up to half a beat more or less; the last scroll page gives it back.
fn cards_on_beats(staged: &Laid, flexible: &[usize], alloc: &mut [(usize, i64)], fit: &Fit, fd: i64) {
    let period = fit.card_beat;
    let Some(&last_flex) = flexible.last() else { return };
    let dur = |i: usize, alloc: &[(usize, i64)]| -> i64 {
        alloc.iter().find(|(k, _)| *k == i).map(|(_, f)| f.saturating_mul(fd)).unwrap_or_else(|| staged.pages.get(i).map(|p| p.duration.0).unwrap_or(0))
    };
    let mut t = 0i64;
    for i in 0..staged.pages.len() {
        let Some(p) = staged.pages.get(i) else { break };
        if p.behaviour == Behaviour::Card && i < last_flex {
            // the closest scroll page before the card
            if let Some(&j) = flexible.iter().rev().find(|j| **j < i) {
                let phase = (t - fit.beat_offset).rem_euclid(period);
                let delta = if phase * 2 < period { -phase } else { period - phase };
                let frames = (delta as f64 / fd as f64).round() as i64;
                let (Some(aj), Some(al)) = (alloc.iter().position(|(k, _)| *k == j), alloc.iter().position(|(k, _)| *k == last_flex)) else { continue };
                let fj = alloc.get(aj).map(|x| x.1).unwrap_or(1);
                let fl = alloc.get(al).map(|x| x.1).unwrap_or(1);
                // both pages keep at least a frame
                let frames = frames.clamp(1 - fj, fl - 1);
                if let Some(x) = alloc.get_mut(aj) {
                    x.1 += frames;
                }
                if let Some(x) = alloc.get_mut(al) {
                    x.1 -= frames;
                }
                t = t.saturating_add(frames.saturating_mul(fd));
            }
        }
        t = t.saturating_add(dur(i, alloc));
        if i + 1 < staged.pages.len() {
            t = t.saturating_add(p.gap_after.0);
        }
    }
}

/// Candidate lengths offered for a piece of music. Nothing is applied: the user picks one.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    /// `musicLength`, `soundEnd` or `lastBar`.
    pub id: &'static str,
    pub total: i64,
}

/// What is known about the music.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Music {
    /// Clip length.
    pub duration: Tick,
    /// The last moment that is not silence (clip time).
    pub sound_end: Option<Tick>,
    /// Beats (clip time), increasing.
    pub beats: Vec<Tick>,
}

/// Bars are assumed to be four beats.
pub const BEATS_PER_BAR: usize = 4;

/// Lengths to offer: the whole clip; until the sound ends (when it ends more than half a second
/// before the clip); the last bar line before the sound ends (when the beat is known).
pub fn suggestions(m: &Music) -> Vec<Suggestion> {
    let mut v = vec![Suggestion { id: "musicLength", total: m.duration.0 }];
    let end = m.sound_end.unwrap_or(m.duration).min(m.duration);
    if m.duration.0 - end.0 > cfa4_time::TICKS_PER_SECOND / 2 {
        v.push(Suggestion { id: "soundEnd", total: end.0 });
    }
    if m.beats.len() > BEATS_PER_BAR {
        let first = m.beats.first().copied().unwrap_or(Tick::ZERO);
        let bar = m.beats.iter().enumerate().filter(|(i, b)| i % BEATS_PER_BAR == 0 && **b <= end && **b > first).map(|(_, b)| *b).next_back();
        if let Some(b) = bar.filter(|b| b.0 > 0 && b.0 != end.0) {
            v.push(Suggestion { id: "lastBar", total: b.0 });
        }
    }
    v
}

/// The music's beat period, from its beats (median spacing); `None` with too few beats.
pub fn beat_period(beats: &[Tick]) -> Option<Tick> {
    if beats.len() < 3 {
        return None;
    }
    let mut d: Vec<i64> = beats.windows(2).filter_map(|w| Some(w.get(1)?.0 - w.first()?.0)).filter(|d| *d > 0).collect();
    d.sort_unstable();
    d.get(d.len() / 2).map(|x| Tick(*x))
}
