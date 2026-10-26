//! `tts.previewAudio`: the last `tts.preview` result as samples for the Qt GUI to play, as egui's
//! `play_tts_preview` reads `Session::tts_preview`: resampled (linear, as egui) to the output
//! device's rate and copied to every channel, as little-endian f32 frames.

use cfa4_engine::Session;
use serde_json::{Value, json};

/// `tts.previewAudio {sampleRate?, channels?}` → `{seconds, sampleRate, channels, frames}` + f32 frames.
pub fn preview_audio(s: &Session, p: &Value) -> Result<(Value, Vec<u8>), String> {
    let audio = s.tts_preview.clone().ok_or("nothing to preview")?;
    let rate = match p.get("sampleRate") {
        Some(v) => v.as_u64().and_then(|x| u32::try_from(x).ok()).filter(|x| (8_000..=192_000).contains(x)).ok_or("`sampleRate` must be 8000..=192000")?,
        None => 48_000,
    };
    let channels = match p.get("channels") {
        Some(v) => v.as_u64().filter(|c| (1..=8).contains(c)).ok_or("`channels` must be 1..=8")? as usize,
        None => 2,
    };
    let step = f64::from(audio.sample_rate.max(1)) / f64::from(rate);
    let frames = (audio.samples.len() as f64 / step).floor() as usize;
    let mut out = Vec::with_capacity(frames * channels * 4);
    for n in 0..frames {
        let pos = n as f64 * step;
        let i = pos as usize;
        let f = (pos - i as f64) as f32;
        let x0 = audio.samples.get(i).copied().unwrap_or(0.0);
        let x1 = audio.samples.get(i + 1).copied().unwrap_or(x0);
        let x = x0 + (x1 - x0) * f;
        for _ in 0..channels {
            out.extend_from_slice(&x.to_le_bytes());
        }
    }
    Ok((json!({"seconds": audio.seconds(), "sampleRate": rate, "channels": channels, "frames": frames}), out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn resamples_the_preview_for_the_device() {
        let mut s = Session::default();
        assert!(preview_audio(&s, &json!({})).is_err(), "nothing previewed yet");
        s.tts_preview = Some(Arc::new(cfa4_tts::Audio { samples: vec![0.0, 1.0, 0.0, -1.0], sample_rate: 24_000 }));
        let (meta, bytes) = preview_audio(&s, &json!({"sampleRate": 48_000, "channels": 2})).expect("audio");
        assert_eq!(meta["frames"], json!(8));
        assert_eq!(bytes.len(), 8 * 2 * 4);
        let v: Vec<f32> = bytes.chunks(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        assert_eq!(&v[..6], &[0.0, 0.0, 0.5, 0.5, 1.0, 1.0]);
        assert!(preview_audio(&s, &json!({"channels": 0})).is_err());
    }
}
