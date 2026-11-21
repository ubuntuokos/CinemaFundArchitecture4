//! Energy-based tightening of word bounds.
//!
//! Attention alignment places every word boundary on the next word's start, so the words come
//! out contiguous: silence between words is absorbed into the word before it (and leading silence
//! into the first word), and a start that the alignment puts late lands the pause **and the
//! beginning of the next word** inside the previous word's span. Pause detection and text-based
//! editing need the silences, so every word is measured against 10 ms frames whose level is
//! compared with a threshold set between the clip's noise floor (10th percentile of the levels of
//! frames above −100 dBFS) and its speech level (95th percentile): 30 % of the way up, and at
//! least 6 dB above the floor. Two passes:
//!
//! 1. **A pause inside a word's span ends the word.** A run of at least 200 ms of frames below
//!    the threshold with voice on both sides of it is a pause between two words, never part of
//!    one. The word keeps the voice before the pause and the next word, when it was contiguous,
//!    starts at the voice after it. When the voice before the pause is too short to be the word
//!    at all (under half of 50 ms per letter, the sound of a word's last syllable mis-assigned to
//!    the next word) the word starts after the pause instead; the previous word takes the scrap
//!    when it was contiguous and no pause separates its voice from the scrap (otherwise the scrap
//!    is a breath or a click that no word owns). A span that holds several pauses is cut at the
//!    first; the rest cascade through the following words.
//! 2. **Silence at the edges is trimmed.** Each word's start and end are moved inward past
//!    frames below the threshold. A word keeps at least 60 ms.

use cfa4_project::Word;
use cfa4_time::Tick;

use crate::{SAMPLE_RATE, TICKS_PER_SAMPLE};

const FRAME: usize = (SAMPLE_RATE / 100) as usize;
const MIN_WORD_FRAMES: usize = 6;
/// A run of silence this long (10 ms frames) inside a word's span is a pause between words.
const MIN_PAUSE_FRAMES: usize = 20;
/// How long a word is expected to sound, per letter (10 ms frames): the rule of pass 1.
const FRAMES_PER_LETTER: usize = 5;
/// Letters counted towards the expected length (a longer word is no longer than this).
const MAX_LETTERS: usize = 40;

/// Frame levels in dBFS (10 ms RMS).
pub fn frame_db(audio: &[f32]) -> Vec<f32> {
    audio
        .chunks(FRAME)
        .map(|c| {
            let e = c.iter().map(|v| v * v).sum::<f32>() / c.len().max(1) as f32;
            10.0 * (e + 1e-12).log10()
        })
        .collect()
}

/// The speech/silence threshold in dBFS.
pub fn threshold(db: &[f32]) -> f32 {
    if db.is_empty() {
        return -60.0;
    }
    // digital silence (< −100 dBFS) would drag the floor far below the recording's own noise
    let mut s: Vec<f32> = db.iter().copied().filter(|v| *v > -100.0).collect();
    if s.is_empty() {
        return -60.0;
    }
    s.sort_by(f32::total_cmp);
    let q = |p: f32| s.get(((s.len() - 1) as f32 * p) as usize).copied().unwrap_or(-60.0);
    let (floor, speech) = (q(0.10), q(0.95));
    (floor + 0.3 * (speech - floor)).max(floor + 6.0)
}

/// The frame range `[a, b)` of a word, clamped to the `frames` of the clip.
fn frames_of(w: &Word, frame_ticks: i64, frames: usize) -> (usize, usize) {
    let a = (w.start.0 / frame_ticks).max(0) as usize;
    let b = ((w.end.0.saturating_add(frame_ticks - 1)) / frame_ticks).max(0) as usize;
    (a.min(frames), b.min(frames).max(a.min(frames)))
}

/// The first pause (see [`MIN_PAUSE_FRAMES`]) in `[a, b)` with voice before and after it, as the
/// range of its silent frames.
fn first_pause(voiced: &[bool], a: usize, b: usize) -> Option<(usize, usize)> {
    let frames = voiced.get(a..b)?;
    let first_voice = frames.iter().position(|&v| v)?;
    let last_voice = frames.iter().rposition(|&v| v)?;
    let mut run_start = None;
    // up to and including the last voiced frame: a pause followed by a single frame of voice
    // (the next word's onset, where the alignment put the boundary) is still a pause
    for (i, &v) in frames.iter().enumerate().take(last_voice + 1).skip(first_voice) {
        match (v, run_start) {
            (false, None) => run_start = Some(i),
            (true, Some(s)) => {
                if i - s >= MIN_PAUSE_FRAMES {
                    return Some((a + s, a + i));
                }
                run_start = None;
            }
            _ => {}
        }
    }
    None
}

/// Frames a word of `text` is expected to sound for.
fn expected_frames(text: &str) -> usize {
    let letters = text.chars().filter(|c| c.is_alphanumeric()).take(MAX_LETTERS).count();
    letters.saturating_mul(FRAMES_PER_LETTER).max(MIN_WORD_FRAMES)
}

/// Pass 1 of the module docs: cut every word at the first pause inside its span.
fn split_at_pauses(voiced: &[bool], words: &mut [Word], frame_ticks: i64) {
    let tick = |frame: usize| Tick((frame as i64).saturating_mul(frame_ticks));
    // a span can hold several pauses; a word moved past one is measured again (every move
    // skips a pause, so this many is more than there can be)
    let moves = voiced.len() / MIN_PAUSE_FRAMES + 1;
    for i in 0..words.len() {
        for _ in 0..moves {
            let Some(w) = words.get(i) else { return };
            let (a, b) = frames_of(w, frame_ticks, voiced.len());
            let Some((ps, pe)) = first_pause(voiced, a, b) else { break };
            let voice_before = voiced.get(a..ps).map(|f| f.iter().filter(|&&v| v).count()).unwrap_or(0);
            if voice_before.saturating_mul(2) < expected_frames(&w.text) {
                // too short to be this word: the word is after the pause. The scrap is the
                // previous word's tail when that word was contiguous and no pause separates its
                // voice from the scrap; otherwise (a breath or a click after a silence) no word
                // takes it
                let start = tick(pe);
                let scrap = voiced.get(a..ps).and_then(|f| f.iter().position(|&v| v)).map(|p| a + p);
                if let (Some(scrap), Some(prev)) = (scrap, i.checked_sub(1).and_then(|p| words.get_mut(p)))
                    && prev.end >= tick(a)
                {
                    let (pa, pb) = frames_of(prev, frame_ticks, voiced.len());
                    let prev_voice = voiced.get(pa..pb.min(scrap)).and_then(|f| f.iter().rposition(|&v| v)).map(|p| pa + p);
                    if prev_voice.is_some_and(|v| scrap.saturating_sub(v) <= MIN_PAUSE_FRAMES) {
                        prev.end = tick(ps).max(prev.start);
                    }
                }
                if let Some(w) = words.get_mut(i) {
                    w.start = start.min(w.end);
                }
                continue;
            }
            let old_end = words.get(i).map(|w| w.end).unwrap_or_default();
            if let Some(w) = words.get_mut(i) {
                w.end = tick(ps).max(w.start);
            }
            if let Some(next) = words.get_mut(i + 1)
                && next.start <= old_end
            {
                next.start = tick(pe).min(next.end);
            }
            break;
        }
    }
}

/// Pass 2 of the module docs: move word bounds inward past silent frames.
fn trim_edges(voiced: &[bool], words: &mut [Word], frame_ticks: i64) {
    for w in words {
        let (a, b) = frames_of(w, frame_ticks, voiced.len());
        if b <= a + MIN_WORD_FRAMES {
            continue;
        }
        let mut s = a;
        while s + MIN_WORD_FRAMES < b && !voiced.get(s).copied().unwrap_or(true) {
            s += 1;
        }
        let mut e = b;
        while e > s + MIN_WORD_FRAMES && !voiced.get(e - 1).copied().unwrap_or(true) {
            e -= 1;
        }
        // nothing voiced at all: leave the word alone
        if voiced.get(s..e).is_none_or(|f| !f.iter().any(|&v| v)) {
            continue;
        }
        if s > a {
            w.start = Tick((s as i64).saturating_mul(frame_ticks));
        }
        if e < b {
            w.end = Tick((e as i64).saturating_mul(frame_ticks)).max(w.start);
        }
    }
}

/// Tighten word bounds against the audio (see the module docs): pauses inside a word's span end
/// it, then silence at the edges is trimmed. `words` are in time order and non-overlapping
/// (`Transcript::normalize`).
pub fn tighten_words(audio: &[f32], words: &mut [Word]) {
    let db = frame_db(audio);
    if db.is_empty() {
        return;
    }
    let th = threshold(&db);
    let voiced: Vec<bool> = db.iter().map(|&v| v >= th).collect();
    let frame_ticks = TICKS_PER_SAMPLE * FRAME as i64;
    split_at_pauses(&voiced, words, frame_ticks);
    trim_edges(&voiced, words, frame_ticks);
}

/// Parameters of [`silence_regions`]. Durations are `Tick`s (AGENTS.md: exact time is `Tick`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SilenceParams {
    /// Fixed threshold in dBFS; `None` is the clip's own automatic one ([`threshold`]).
    pub threshold_db: Option<f32>,
    /// A silence shorter than this is not a region.
    pub min_silence: Tick,
    /// Silence kept on both sides of the speech: taken off both ends of every region.
    pub padding: Tick,
}

impl Default for SilenceParams {
    /// 0.5 s minimum silence, 0.1 s padding (plan CFA4-V1-SILENCE-REMOVAL-001, D5).
    fn default() -> Self {
        Self { threshold_db: None, min_silence: crate::seconds_tick(0.5), padding: crate::seconds_tick(0.1) }
    }
}

/// The silent regions of `audio` (16 kHz mono) as `(start, duration)` in ticks from the start of
/// the audio, in time order and non-overlapping, on the 10 ms frame grid.
///
/// A run of frames below the threshold that lasts at least `min_silence` is a region; `padding` is
/// then taken off both ends, so a region never touches speech by less than the padding. Silence at
/// the very start or end of the audio has no speech on its outer side, so no padding is taken
/// there. A region that padding shrinks to nothing is dropped. A threshold or duration that is
/// NaN, infinite or negative falls back to the automatic threshold, zero minimum and zero padding.
/// Audio with no voiced frame at all (digital silence, a flat tone below the floor) gives no region:
/// there is no speech to measure the silence against.
pub fn silence_regions(audio: &[f32], p: &SilenceParams) -> Vec<(Tick, Tick)> {
    let db = frame_db(audio);
    if db.is_empty() {
        return Vec::new();
    }
    let th = match p.threshold_db {
        Some(t) if t.is_finite() => t,
        _ => threshold(&db),
    };
    let voiced: Vec<bool> = db.iter().map(|&v| v >= th).collect();
    if !voiced.iter().any(|&v| v) {
        return Vec::new();
    }
    let frame_ticks = TICKS_PER_SAMPLE * FRAME as i64;
    let frames = |t: Tick| (t.0.max(0) / frame_ticks) as usize;
    let min_frames = frames(p.min_silence).max(1);
    let pad = frames(p.padding);
    let n = voiced.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        if voiced.get(i).copied().unwrap_or(true) {
            i += 1;
            continue;
        }
        let a = i;
        while i < n && !voiced.get(i).copied().unwrap_or(true) {
            i += 1;
        }
        let b = i;
        if b - a < min_frames {
            continue;
        }
        let from = if a == 0 { a } else { a.saturating_add(pad) };
        let to = if b == n { b } else { b.saturating_sub(pad) };
        if to > from {
            let start = Tick((from as i64).saturating_mul(frame_ticks));
            // the last frame can be partial: the region ends with the audio
            let end_ticks = (audio.len() as i64).saturating_mul(TICKS_PER_SAMPLE);
            let end = Tick((to as i64).saturating_mul(frame_ticks)).min(Tick(end_ticks));
            if end > start {
                out.push((start, Tick(end.0 - start.0)));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seconds_tick;

    /// `seconds` of audio: a faint noise floor, with a tone over each `(from, to)` range.
    fn audio(seconds: f64, tones: &[(f64, f64)]) -> Vec<f32> {
        let n = (seconds * SAMPLE_RATE as f64) as usize;
        let mut a: Vec<f32> = (0..n).map(|i| if i % 2 == 0 { 0.0001 } else { 0.0 }).collect();
        for &(from, to) in tones {
            for i in (from * SAMPLE_RATE as f64) as usize..((to * SAMPLE_RATE as f64) as usize).min(n) {
                a[i] = 0.3 * (i as f32 * 0.2).sin();
            }
        }
        a
    }

    fn word(text: &str, from: f64, to: f64) -> Word {
        Word::new(text, seconds_tick(from), seconds_tick(to))
    }

    fn bounds(w: &[Word]) -> Vec<(f64, f64)> {
        let s = |t: Tick| (t.0 as f64 / cfa4_time::TICKS_PER_SECOND as f64 * 100.0).round() / 100.0;
        w.iter().map(|w| (s(w.start), s(w.end))).collect()
    }

    #[test]
    fn silence_is_trimmed_from_words() {
        // 0.0–0.5 silence, 0.5–1.0 tone, 1.0–2.0 silence, 2.0–2.5 tone
        let a = audio(2.5, &[(0.5, 1.0), (2.0, 2.5)]);
        let mut w = vec![word("a", 0.0, 2.0), word("b", 2.0, 2.5)];
        tighten_words(&a, &mut w);
        assert_eq!(w[0].start, seconds_tick(0.5));
        assert_eq!(w[0].end, seconds_tick(1.0));
        assert_eq!((w[1].start, w[1].end), (seconds_tick(2.0), seconds_tick(2.5)));
    }

    #[test]
    fn a_pause_inside_a_word_ends_it_and_the_next_word_starts_after_the_pause() {
        // "hello" sounds 0.0–0.5, a 0.5 s pause, "world" sounds 1.0–1.5; the alignment put the
        // start of "world" 0.5 s late, so "hello" spans the pause and half of "world"
        let a = audio(2.0, &[(0.0, 0.5), (1.0, 1.5)]);
        let mut w = vec![word("hello", 0.0, 1.25), word("world", 1.25, 1.5)];
        tighten_words(&a, &mut w);
        assert_eq!(bounds(&w), [(0.0, 0.5), (1.0, 1.5)]);
    }

    #[test]
    fn a_scrap_before_the_pause_goes_to_the_previous_word() {
        // "one" sounds 0.0–0.54 (its last 40 ms assigned to "cute"), a pause, "cute" sounds
        // 0.9–1.3: 40 ms cannot be "cute", so it starts after the pause and "one" takes the scrap
        let a = audio(1.5, &[(0.0, 0.54), (0.9, 1.3)]);
        let mut w = vec![word("one", 0.0, 0.5), word("cute", 0.5, 1.3)];
        tighten_words(&a, &mut w);
        assert_eq!(bounds(&w), [(0.0, 0.54), (0.9, 1.3)]);
    }

    #[test]
    fn a_click_after_a_long_silence_is_nobody_s_tail() {
        // "there?" sounds 0.0–0.4, 1.6 s of silence with a 30 ms click at 2.0, "Keep" sounds
        // 2.5–2.8; the alignment ran "there?" up to the click and "Keep" from it: the click is too
        // short to be "Keep", but it is not the tail of "there?" either, so "there?" must not
        // be stretched over the silence to take it
        let a = audio(3.0, &[(0.0, 0.4), (2.0, 2.03), (2.5, 2.8)]);
        let mut w = vec![word("there?", 0.0, 2.0), word("Keep", 2.0, 2.8)];
        tighten_words(&a, &mut w);
        assert_eq!(bounds(&w), [(0.0, 0.4), (2.5, 2.8)]);
    }

    #[test]
    fn pauses_cascade_through_late_words() {
        // three words, each 0.3 s of sound with 0.3 s pauses between them; every start but the
        // first is late by a whole pause plus a word
        let a = audio(2.0, &[(0.0, 0.3), (0.6, 0.9), (1.2, 1.5)]);
        let mut w = vec![word("bi", 0.0, 0.9), word("or", 0.9, 1.5), word("something", 1.5, 1.6)];
        tighten_words(&a, &mut w);
        assert_eq!(bounds(&w), [(0.0, 0.3), (0.6, 0.9), (1.2, 1.5)]);
    }

    #[test]
    fn a_gap_already_between_words_is_kept() {
        // "x" spans a pause and the first half of the sound 0.6–0.9, but "y" was not contiguous:
        // "x" ends at the pause and "y" is left where it was
        let a = audio(1.5, &[(0.0, 0.3), (0.6, 0.9)]);
        let mut w = vec![word("x", 0.0, 0.75), word("y", 0.8, 0.9)];
        tighten_words(&a, &mut w);
        assert_eq!(bounds(&w), [(0.0, 0.3), (0.8, 0.9)]);
    }

    #[test]
    fn short_breaks_inside_a_word_are_not_pauses() {
        // a 150 ms dip (a stop consonant) inside "paper" stays inside it
        let a = audio(1.0, &[(0.0, 0.3), (0.45, 0.8)]);
        let mut w = vec![word("paper", 0.0, 0.8), word("cut", 0.8, 1.0)];
        tighten_words(&a, &mut w);
        assert_eq!(bounds(&w), [(0.0, 0.8), (0.8, 1.0)]);
    }

    #[test]
    fn a_pause_followed_by_one_frame_of_voice_is_still_a_pause() {
        // "yeah." sounds 0.0–0.4, a 1.0 s pause, then the alignment ends it on the first 10 ms of
        // the next word's sound
        let a = audio(2.0, &[(0.0, 0.4), (1.4, 1.8)]);
        let mut w = vec![word("yeah.", 0.0, 1.41), word("so", 1.41, 1.8)];
        tighten_words(&a, &mut w);
        assert_eq!(bounds(&w), [(0.0, 0.4), (1.4, 1.8)]);
    }

    #[test]
    fn hostile_words_never_panic() {
        let a = audio(1.0, &[(0.0, 0.3), (0.6, 0.9)]);
        let mut w = vec![
            Word::new("", Tick(i64::MIN), Tick(i64::MIN)),
            word("past", 5.0, 9.0),
            Word::new("x", Tick(i64::MAX - 1), Tick(i64::MAX)),
            word("zero", 0.2, 0.2),
            word("wide", 0.0, 1.0),
        ];
        tighten_words(&a, &mut w);
        tighten_words(&[], &mut w);
        tighten_words(&[0.0; 16], &mut w);
        assert!(w.iter().all(|w| w.end >= w.start));
    }

    #[test]
    fn silence_regions_are_the_gaps_between_speech_minus_the_padding() {
        // tone 0.5–1.0 and 2.0–2.5 in 3.0 s: gaps 0–0.5, 1.0–2.0, 2.5–3.0
        let a = audio(3.0, &[(0.5, 1.0), (2.0, 2.5)]);
        let p = SilenceParams { threshold_db: None, min_silence: seconds_tick(0.4), padding: seconds_tick(0.1) };
        let r: Vec<(f64, f64)> = silence_regions(&a, &p)
            .iter()
            .map(|(s, d)| (s.0 as f64 / cfa4_time::TICKS_PER_SECOND as f64, d.0 as f64 / cfa4_time::TICKS_PER_SECOND as f64))
            .collect();
        assert_eq!(r.len(), 3);
        let near = |x: f64, y: f64| (x - y).abs() <= 0.032;
        // outer edges get no padding on the audio's side
        assert!(near(r[0].0, 0.0) && near(r[0].0 + r[0].1, 0.4), "{r:?}");
        assert!(near(r[1].0, 1.1) && near(r[1].0 + r[1].1, 1.9), "{r:?}");
        assert!(near(r[2].0, 2.6) && near(r[2].0 + r[2].1, 3.0), "{r:?}");
    }

    #[test]
    fn short_gaps_and_padding_that_eats_the_gap_give_no_region() {
        let a = audio(2.0, &[(0.0, 0.5), (0.8, 2.0)]);
        // 0.3 s gap below the minimum
        let p = SilenceParams { threshold_db: None, min_silence: seconds_tick(0.5), padding: Tick(0) };
        assert!(silence_regions(&a, &p).is_empty());
        // 0.3 s gap, padding 0.2 s on each side: nothing left
        let p = SilenceParams { threshold_db: None, min_silence: seconds_tick(0.2), padding: seconds_tick(0.2) };
        assert!(silence_regions(&a, &p).is_empty());
    }

    #[test]
    fn all_speech_digital_silence_and_empty_audio_give_no_region() {
        let p = SilenceParams::default();
        assert!(silence_regions(&audio(1.0, &[(0.0, 1.0)]), &p).is_empty());
        assert!(silence_regions(&[0.0; 16_000], &p).is_empty());
        assert!(silence_regions(&[], &p).is_empty());
    }

    #[test]
    fn hostile_silence_parameters_never_panic() {
        let a = audio(2.0, &[(0.0, 0.5), (1.5, 2.0)]);
        let p = SilenceParams { threshold_db: Some(f32::NAN), min_silence: Tick(i64::MIN), padding: Tick(i64::MAX) };
        let _ = silence_regions(&a, &p);
        let p = SilenceParams { threshold_db: Some(f32::INFINITY), min_silence: Tick(i64::MAX), padding: Tick(-5) };
        assert!(silence_regions(&a, &p).is_empty());
        let p = SilenceParams { threshold_db: Some(-40.0), min_silence: Tick(0), padding: Tick(0) };
        assert!(!silence_regions(&a, &p).is_empty());
    }
}
