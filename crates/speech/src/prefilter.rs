//! Speech pre-filter for the recogniser (plan CFA4-V1-SILENCE-REMOVAL-001, V1 point 3).
//!
//! Whisper "hallucinates" text in silence and room noise and spends time on it. The pre-filter
//! cuts the audio to the stretches a voice-activity detector calls speech, hands only those to the
//! [`Transcriber`], and puts the words back at their place in media time. It works with any
//! detector: it takes one voiced/unvoiced flag per frame, as [`crate::vad::silence_regions_from_voiced`]
//! does, so the energy detector and the Silero model share it.
//!
//! Pure functions and no model: the tests use a fixed transcriber.

use cfa4_project::Transcript;
use cfa4_time::Tick;

use crate::{Options, ProgressFn, SpeechError, TICKS_PER_SAMPLE, Transcriber, diarize, sample_tick, seconds_tick};

/// How the speech stretches are formed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Audio kept on both sides of every speech stretch, so a word's edges are not clipped.
    pub padding: Tick,
    /// Two speech stretches whose gap is at most this long are one stretch: the recogniser keeps
    /// the context of a sentence, and a short pause is not worth a second call.
    pub merge_gap: Tick,
    /// A stretch shorter than this is noise, not speech, and is dropped (before padding).
    pub min_speech: Tick,
}

impl Default for Params {
    fn default() -> Self {
        Self { padding: seconds_tick(0.2), merge_gap: seconds_tick(0.5), min_speech: seconds_tick(0.1) }
    }
}

/// A duration in ticks as whole samples (negative counts as zero).
fn samples_of(t: Tick) -> usize {
    usize::try_from(t.0.max(0) / TICKS_PER_SAMPLE).unwrap_or(usize::MAX)
}

/// The speech stretches of `samples` samples of audio as `(start, end)` sample ranges, in time
/// order, non-overlapping and inside the audio. `voiced` holds one flag per frame of
/// `frame_samples` samples. No voiced frame, a zero frame size or no audio gives no stretch.
pub fn speech_segments(voiced: &[bool], frame_samples: usize, samples: usize, p: &Params) -> Vec<(usize, usize)> {
    if frame_samples == 0 || samples == 0 {
        return Vec::new();
    }
    let at = |frame: usize| frame.saturating_mul(frame_samples).min(samples);
    // the voiced runs
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < voiced.len() {
        if !voiced.get(i).copied().unwrap_or(false) {
            i += 1;
            continue;
        }
        let a = i;
        while i < voiced.len() && voiced.get(i).copied().unwrap_or(false) {
            i += 1;
        }
        runs.push((at(a), at(i)));
    }
    let min = samples_of(p.min_speech);
    runs.retain(|(a, b)| b.saturating_sub(*a) >= min && b > a);
    // merge across short gaps
    let gap = samples_of(p.merge_gap);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (a, b) in runs {
        match merged.last_mut() {
            Some(last) if a.saturating_sub(last.1) <= gap => last.1 = b,
            _ => merged.push((a, b)),
        }
    }
    // padding, then merge what the padding made touch
    let pad = samples_of(p.padding);
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (a, b) in merged {
        let (a, b) = (a.saturating_sub(pad), b.saturating_add(pad).min(samples));
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// Transcribe only `segments` of `audio` (mono 16 kHz) and return one transcript on the whole
/// audio's time base. The language found in the first stretch with words is used for the rest, so
/// a stretch of a few words is not guessed again. Speakers are labelled once on the whole audio
/// (labels per stretch would not agree with each other). `progress` is shared over the stretches.
pub fn transcribe_segments(
    t: &dyn Transcriber,
    audio: &[f32],
    segments: &[(usize, usize)],
    opts: &Options,
    progress: ProgressFn<'_>,
) -> Result<Transcript, SpeechError> {
    let mut out = Transcript { language: opts.language.clone().unwrap_or_default(), source: t.id(), speakers: Vec::new(), words: Vec::new() };
    let mut inner = Options { diarize: false, ..opts.clone() };
    let total: usize = segments.iter().map(|(a, b)| b.saturating_sub(*a)).sum::<usize>().max(1);
    let mut done = 0usize;
    for &(a, b) in segments {
        let Some(part) = audio.get(a..b.min(audio.len())).filter(|p| !p.is_empty()) else { continue };
        let (base, span) = (done as f32 / total as f32, part.len() as f32 / total as f32);
        let mut cancelled = false;
        let mut report = |f: f32, status: &str| {
            let keep = progress(0.95 * (base + span * f.clamp(0.0, 1.0)), status);
            cancelled |= !keep;
            keep
        };
        let tr = t.transcribe(part, &inner, &mut report)?;
        if cancelled {
            return Err(SpeechError::Cancelled);
        }
        done += part.len();
        if inner.language.is_none() && !tr.words.is_empty() && !tr.language.is_empty() {
            inner.language = Some(tr.language.clone());
            out.language = tr.language.clone();
        }
        let shift = sample_tick(a as i64);
        out.words.extend(tr.words.into_iter().map(|mut w| {
            w.start = Tick(w.start.0.saturating_add(shift.0));
            w.end = Tick(w.end.0.saturating_add(shift.0));
            w
        }));
    }
    if out.language.is_empty() {
        out.language = "en".into();
    }
    out.normalize();
    if opts.diarize && !out.words.is_empty() {
        if !progress(0.96, "Labelling speakers") {
            return Err(SpeechError::Cancelled);
        }
        let p = diarize::Params { max_speakers: opts.max_speakers, ..Default::default() };
        diarize::diarize(audio, &mut out, &p);
    }
    progress(1.0, "Done");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SAMPLE_RATE;
    use cfa4_project::Word;

    fn secs(s: f64) -> usize {
        (s * f64::from(SAMPLE_RATE)).round() as usize
    }

    /// One frame of 10 ms; speech at 1.0-2.0 s and 2.3-3.0 s, and 6.0-6.05 s (a click).
    fn voiced() -> Vec<bool> {
        (0..800).map(|f| (100..200).contains(&f) || (230..300).contains(&f) || (600..605).contains(&f)).collect()
    }

    #[test]
    fn stretches_are_merged_padded_and_clicks_dropped() {
        let seg = speech_segments(&voiced(), 160, secs(8.0), &Params::default());
        // 1.0-2.0 and 2.3-3.0 are one stretch (gap 0.3 s <= 0.5 s), padded by 0.2 s; the click is gone
        assert_eq!(seg, vec![(secs(0.8), secs(3.2))]);
    }

    #[test]
    fn padding_stays_inside_the_audio_and_touching_stretches_become_one() {
        let v: Vec<bool> = (0..300).map(|f| f < 10 || (290..300).contains(&f) || (150..160).contains(&f)).collect();
        let p = Params { padding: seconds_tick(0.2), merge_gap: seconds_tick(0.0), min_speech: seconds_tick(0.05) };
        let seg = speech_segments(&v, 160, secs(3.0), &p);
        assert_eq!(seg, vec![(0, secs(0.3)), (secs(1.3), secs(1.8)), (secs(2.7), secs(3.0))]);
        let wide = Params { padding: seconds_tick(1.0), ..p };
        assert_eq!(speech_segments(&v, 160, secs(3.0), &wide), vec![(0, secs(3.0))]);
    }

    #[test]
    fn nothing_voiced_or_nothing_to_split_gives_no_stretch() {
        let p = Params::default();
        assert!(speech_segments(&[false; 50], 160, secs(0.5), &p).is_empty());
        assert!(speech_segments(&[], 160, 0, &p).is_empty());
        assert!(speech_segments(&[true; 5], 0, 800, &p).is_empty());
        // flags past the end of the audio are clamped to it
        let seg = speech_segments(&[true; 100], 160, secs(0.5), &Params { min_speech: Tick(0), ..p });
        assert_eq!(seg, vec![(0, secs(0.5))]);
    }

    /// Says one word per stretch: "w<seconds the stretch starts at>" at the stretch's own 0.1 s.
    struct Echo;
    impl Transcriber for Echo {
        fn id(&self) -> String {
            "echo".into()
        }
        fn transcribe(&self, audio: &[f32], opts: &Options, progress: ProgressFn<'_>) -> Result<Transcript, SpeechError> {
            assert!(!opts.diarize, "speakers are labelled once, on the whole audio");
            if !progress(0.5, "x") {
                return Err(SpeechError::Cancelled);
            }
            let first = audio.first().copied().unwrap_or(0.0);
            let w = Word { text: format!("w{first}"), start: seconds_tick(0.1), end: seconds_tick(0.2), speaker: None, confidence: 1.0 };
            Ok(Transcript { language: opts.language.clone().unwrap_or_else(|| "hu".into()), source: "echo".into(), speakers: vec![], words: vec![w] })
        }
    }

    #[test]
    fn words_return_to_their_place_in_media_time() {
        // sample value = its own index in tenths of a second, so each stretch is recognisable
        let audio: Vec<f32> = (0..secs(10.0)).map(|i| (i / 1600) as f32).collect();
        let seg = [(secs(1.0), secs(2.0)), (secs(5.0), secs(6.0))];
        let mut last = 0.0;
        let t = transcribe_segments(&Echo, &audio, &seg, &Options::default(), &mut |f, _| {
            assert!(f >= last, "progress never goes back");
            last = f;
            true
        })
        .unwrap();
        assert_eq!(last, 1.0);
        assert_eq!(t.words.len(), 2);
        assert_eq!((t.words[0].text.as_str(), t.words[0].start, t.words[0].end), ("w10", seconds_tick(1.1), seconds_tick(1.2)));
        assert_eq!((t.words[1].text.as_str(), t.words[1].start), ("w50", seconds_tick(5.1)));
        assert_eq!((t.language.as_str(), t.source.as_str()), ("hu", "echo"));
        assert_eq!(t.speakers.len(), 1, "diarization ran once on the whole audio");
    }

    #[test]
    fn no_stretch_gives_an_empty_transcript_and_cancel_is_honoured() {
        let audio = vec![0.0f32; secs(2.0)];
        let t = transcribe_segments(&Echo, &audio, &[], &Options { diarize: false, ..Options::default() }, &mut |_, _| true).unwrap();
        assert!(t.words.is_empty());
        assert_eq!(t.language, "en");
        let e = transcribe_segments(&Echo, &audio, &[(0, secs(1.0))], &Options { diarize: false, ..Options::default() }, &mut |_, _| false);
        assert_eq!(e, Err(SpeechError::Cancelled));
    }
}
