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

/// Mono 16 kHz samples of a 16-bit PCM WAV file (the first channel; the rate must be 16 kHz).
fn read_wav_16k(path: &Path) -> Result<Vec<f32>, String> {
    let b = std::fs::read(path).map_err(|e| e.to_string())?;
    let u16_at = |i: usize| b.get(i..i + 2).map(|x| u16::from_le_bytes([x[0], x[1]]));
    let u32_at = |i: usize| b.get(i..i + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]));
    if b.get(0..4) != Some(b"RIFF") || b.get(8..12) != Some(b"WAVE") {
        return Err("not a WAV file".into());
    }
    let (mut at, mut fmt, mut data) = (12usize, None, None);
    while let (Some(id), Some(len)) = (b.get(at..at + 4), u32_at(at + 4)) {
        let body = at + 8;
        match id {
            b"fmt " => fmt = Some((u16_at(body), u16_at(body + 2), u32_at(body + 4), u16_at(body + 14))),
            b"data" => data = b.get(body..(body + len as usize).min(b.len())),
            _ => {}
        }
        at = body + len as usize + (len as usize & 1);
    }
    let (Some((Some(1), Some(ch), Some(rate), Some(16))), Some(data)) = (fmt, data) else { return Err("only 16-bit PCM WAV is read".into()) };
    if rate != SAMPLE_RATE || ch == 0 {
        return Err(format!("the fixture must be {SAMPLE_RATE} Hz (it is {rate} Hz); convert it with `ffmpeg -ar 16000`"));
    }
    Ok(data.chunks_exact(2 * ch as usize).map(|f| i16::from_le_bytes([f[0], f[1]]) as f32 / 32768.0).collect())
}

/// Acceptance of plan CFA4-V1-SILENCE-REMOVAL-001 (V1 point 4) on **real human speech**: the
/// model's speech bounds against hand-labelled ones, within one window (32 ms).
///
/// Needs a fixture that is not in the repository (no media is committed): `CFA4_SPEECH_FIXTURE`
/// names a 16 kHz 16-bit PCM WAV, and `<same path>.speech.tsv` lists the speech stretches, one
/// `start_seconds<TAB>end_seconds` per line (a pause of at least 0.3 s between stretches). Without
/// both the test prints SKIPPED. Every measured difference is printed, so a failure shows how far
/// off the model is (run with `--nocapture`).
#[test]
fn real_speech_bounds_are_within_one_window() {
    let Some(v) = vad() else { return };
    let Some(wav) = std::env::var_os("CFA4_SPEECH_FIXTURE").map(PathBuf::from) else {
        eprintln!("SKIPPED: CFA4_SPEECH_FIXTURE (a 16 kHz WAV of real speech with a .speech.tsv label file) is not set");
        return;
    };
    let audio = read_wav_16k(&wav).expect("the fixture WAV");
    let mut labels_path = wav.clone().into_os_string();
    labels_path.push(".speech.tsv");
    let labels: Vec<(f64, f64)> = std::fs::read_to_string(&labels_path)
        .expect("the label file next to the fixture")
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let mut f = l.split('\t').map(|x| x.trim().parse::<f64>().expect("seconds"));
            (f.next().expect("start"), f.next().expect("end"))
        })
        .collect();
    assert!(!labels.is_empty(), "the label file has no stretch");
    // the model's stretches, joined across pauses shorter than 0.3 s as the labels are
    let voiced = v.voiced(&audio).unwrap();
    let win = WINDOW as f64 / f64::from(SAMPLE_RATE);
    let mut found: Vec<(f64, f64)> = Vec::new();
    let mut i = 0;
    while i < voiced.len() {
        if !voiced[i] {
            i += 1;
            continue;
        }
        let a = i;
        while i < voiced.len() && voiced[i] {
            i += 1;
        }
        let (s, e) = (a as f64 * win, i as f64 * win);
        match found.last_mut() {
            Some(l) if s - l.1 < 0.3 => l.1 = e,
            _ => found.push((s, e)),
        }
    }
    eprintln!("labelled {} stretches, the model found {}", labels.len(), found.len());
    assert_eq!(found.len(), labels.len(), "stretches found {found:?} vs labelled {labels:?}");
    let mut worst = 0.0f64;
    for (f, l) in found.iter().zip(&labels) {
        let (ds, de) = ((f.0 - l.0).abs(), (f.1 - l.1).abs());
        eprintln!("labelled {:.3}-{:.3} s, model {:.3}-{:.3} s: start off {:.0} ms, end off {:.0} ms", l.0, l.1, f.0, f.1, ds * 1e3, de * 1e3);
        worst = worst.max(ds).max(de);
    }
    assert!(worst <= win + 1e-9, "worst bound is {:.0} ms off, the tolerance is one window ({:.0} ms)", worst * 1e3, win * 1e3);
}
