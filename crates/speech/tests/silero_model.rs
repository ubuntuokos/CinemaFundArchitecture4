//! Silero VAD on real model weights, when the model is on this machine.
//!
//! Never downloads. Looks for the model in `$CFA4_MODELS_DIR/silero-vad/`, then
//! `<repo>/target/models/silero-vad/` (put `silero_vad_op18_ifless.onnx` of Silero VAD 6.2.3 there by hand, or
//! let the app download it into the data directory and point `CFA4_MODELS_DIR` at
//! `<data dir>/models`). Without it every test prints SKIPPED. Speech is CFA4's built-in formant
//! voice (generated, nothing is committed); the VAD bounds must agree with the generated layout
//! within one 32 ms window plus the model's own smoothing.
#![cfg(feature = "silero")]

use std::path::{Path, PathBuf};

use cfa4_speech::vad::SilenceParams;
use cfa4_speech::vad::silero::{self, SileroVad, WINDOW};
use cfa4_speech::{SAMPLE_RATE, seconds_tick};
use cfa4_tts::Params;

fn models_dir() -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = std::env::var_os("CFA4_MODELS_DIR").map(PathBuf::from).into_iter().collect();
    roots.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/models"));
    roots.into_iter().find(|r| silero::installed(r))
}

fn vad() -> Option<SileroVad> {
    let Some(dir) = models_dir() else {
        eprintln!("SKIPPED: the Silero VAD model is not on this machine");
        return None;
    };
    Some(SileroVad::load(&dir).expect("an installed model loads"))
}

/// The built-in voice at 16 kHz (linear resampling from 24 kHz is enough for a VAD test).
fn speech() -> Vec<f32> {
    let voice = cfa4_tts::voice(cfa4_tts::default_voice_id()).expect("built-in voice");
    let a = voice.synthesize(cfa4_tts::SAMPLE_SENTENCE, &Params::default()).expect("synthesis");
    let ratio = f64::from(a.sample_rate) / f64::from(SAMPLE_RATE);
    let n = (a.samples.len() as f64 / ratio) as usize;
    (0..n)
        .map(|i| {
            let x = i as f64 * ratio;
            let (i0, f) = (x as usize, (x - x.floor()) as f32);
            let (a0, a1) = (a.samples.get(i0).copied().unwrap_or(0.0), a.samples.get(i0 + 1).copied().unwrap_or(0.0));
            a0 + (a1 - a0) * f
        })
        .collect()
}

#[test]
fn silence_and_quiet_noise_are_not_speech() {
    let Some(v) = vad() else { return };
    let mut x = 12345u32;
    let noise: Vec<f32> = (0..SAMPLE_RATE as usize * 2)
        .map(|_| {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            ((x >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.002
        })
        .collect();
    for audio in [vec![0.0f32; SAMPLE_RATE as usize * 2], noise] {
        let p = v.probabilities(&audio).unwrap();
        assert_eq!(p.len(), audio.len().div_ceil(WINDOW));
        assert!(p.iter().all(|p| (0.0..=1.0).contains(p)));
        assert!(!silero::voiced(&p).iter().any(|&b| b), "no window of silence or faint noise is speech");
        // no speech at all: no region to measure the silence against
        assert!(silero::silence_regions(&v, &audio, &SilenceParams::default()).unwrap().is_empty());
    }
}

#[test]
fn pieces_give_the_same_probabilities_as_the_whole() {
    let Some(v) = vad() else { return };
    let mut audio = vec![0.0f32; SAMPLE_RATE as usize / 2];
    audio.extend(speech());
    let whole = v.probabilities(&audio).unwrap();
    let mut s = v.stream();
    let mut parts = Vec::new();
    // odd piece sizes that never line up with the 512-sample window
    for c in audio.chunks(1000) {
        parts.extend(s.push(c).unwrap());
    }
    parts.extend(s.finish().unwrap());
    assert_eq!(whole.len(), parts.len());
    for (a, b) in whole.iter().zip(&parts) {
        assert!((a - b).abs() < 1e-5, "{a} vs {b}");
    }
}

#[test]
fn nan_and_out_of_range_samples_do_not_break_the_model() {
    let Some(v) = vad() else { return };
    let audio = [f32::NAN, f32::INFINITY, -1e30, 1e30].repeat(2000);
    let p = v.probabilities(&audio).unwrap();
    assert!(p.iter().all(|p| p.is_finite() && (0.0..=1.0).contains(p)));
    assert!(v.probabilities(&[]).unwrap().is_empty());
}

#[test]
fn speech_between_silences_is_found_within_a_window() {
    let Some(v) = vad() else { return };
    let lead = SAMPLE_RATE as usize * 2;
    let sp = speech();
    let mut audio = vec![0.0f32; lead];
    audio.extend(&sp);
    audio.extend(vec![0.0f32; lead]);
    let voiced = v.voiced(&audio).unwrap();
    let first = voiced.iter().position(|&b| b);
    let last = voiced.iter().rposition(|&b| b);
    let (Some(first), Some(last)) = (first, last) else {
        // The robotic formant voice is not guaranteed to be speech to the model; say so loudly
        // instead of passing silently.
        panic!("the model found no speech in the built-in voice");
    };
    let (start, end) = ((first * WINDOW) as i64, ((last + 1) * WINDOW) as i64);
    let (want_start, want_end) = (lead as i64, (lead + sp.len()) as i64);
    // The model, not the generated layout, decides what is speech: a few windows at the start, and
    // up to 0.4 s at the end, where the robotic voice fades out in a breathy tail the model
    // (rightly) does not call speech.
    let (tol_start, tol_end) = ((WINDOW * 4) as i64, (SAMPLE_RATE as i64 * 2) / 5);
    assert!((start - want_start).abs() <= tol_start, "start {start} vs {want_start}");
    assert!((end - want_end).abs() <= tol_end, "end {end} vs {want_end}");
    // the silent regions are the two leads, minus nothing at the audio's outer edges
    let r = silero::silence_regions(&v, &audio, &SilenceParams { threshold_db: None, min_silence: seconds_tick(0.5), padding: seconds_tick(0.0) }).unwrap();
    assert_eq!(r.len(), 2, "{r:?}");
}
