//! G.711, G.722, IMA ADPCM and Microsoft ADPCM in WAV, AVI, Matroska (`A_MS/ACM`) and QuickTime,
//! read through the media sources, against ffmpeg's decode of the same file: every sample equal,
//! read front to back and from the middle (a seek).

mod common;

use common::*;

/// Tones, a sweep, noise and full-scale clipping.
const SOURCE: &str = "aevalsrc=exprs='if(lt(t,1),0.5*sin(2*PI*440*t),if(lt(t,2),0.9*sin(2*PI*(100+3000*t)*t),if(lt(t,3),(random(0)-0.5)*1.6,1.2*sin(2*PI*1500*t))))|0.3*sin(2*PI*220*t)':s=RATE:d=3.6";

/// (file, ffmpeg encoder, rate, channels, frames trimmed): ffmpeg's G.722 encoder declares 22
/// samples of delay, which Matroska keeps as `CodecDelay` and we discard, as the format says.
const CASES: &[(&str, &str, u32, u32, usize)] = &[
    ("mulaw.wav", "pcm_mulaw", 8000, 2, 0),
    ("alaw.wav", "pcm_alaw", 8000, 1, 0),
    ("g722.wav", "g722", 16000, 1, 0),
    ("ima_wav.wav", "adpcm_ima_wav", 22050, 2, 0),
    ("ms.wav", "adpcm_ms", 22050, 2, 0),
    ("mulaw.avi", "pcm_mulaw", 8000, 1, 0),
    ("alaw.avi", "pcm_alaw", 8000, 2, 0),
    ("ima_wav.avi", "adpcm_ima_wav", 44100, 2, 0),
    ("ms.avi", "adpcm_ms", 22050, 1, 0),
    ("g722.avi", "g722", 16000, 1, 0),
    ("ima_wav.mkv", "adpcm_ima_wav", 22050, 1, 0),
    ("ms.mkv", "adpcm_ms", 44100, 2, 0),
    ("mulaw.mkv", "pcm_mulaw", 8000, 1, 0),
    ("g722.mkv", "g722", 16000, 1, 22),
    ("mulaw.mov", "pcm_mulaw", 8000, 2, 0),
    ("alaw.mov", "pcm_alaw", 8000, 1, 0),
    ("ima4.mov", "adpcm_ima_qt", 44100, 2, 0),
];

fn check(name: &str, codec: &str, rate: u32, ch: u32, trim: usize) {
    let ff = cfa4_testkit::require_ffmpeg!();
    let src = SOURCE.replace("RATE", &rate.to_string());
    let layout = if ch == 1 { "pan=mono|c0=c0" } else { "anull" };
    let file = format!("adpcm_{name}");
    let Some(path) = fixture(&ff, &file, &["-f", "lavfi", "-i", &src, "-af", layout, "-c:a", codec]) else {
        cfa4_testkit::oracle::skip(name, "ffmpeg can't write it");
        return;
    };
    let decoded: Vec<f32> = ffmpeg_out(&ff, &["-i", path.to_str().unwrap(), "-map", "0:a:0", "-f", "s16le", "-c:a", "pcm_s16le", "-"])
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0)
        .collect();
    let c = ch as usize;
    let reference = &decoded[trim * c..];
    let frames = reference.len() / c;
    let s = cfa4_codecs::open_bytes(&file, bytes(&path)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let info = s.info().audio().unwrap_or_else(|| panic!("{name}: no audio"));
    assert_eq!((info.sample_rate, info.channels), (rate, ch), "{name}");
    // front to back in playback-sized reads
    let read = |from: usize, n: usize| -> Vec<f32> {
        let mut out = Vec::with_capacity(n * c);
        let mut at = from;
        while at < from + n {
            let k = 1000.min(from + n - at);
            let b = s.audio(at as i64, k, rate).unwrap_or_else(|e| panic!("{name}: {e}"));
            for i in 0..k {
                out.extend(b.channels.iter().map(|p| p[i]));
            }
            at += k;
        }
        out
    };
    let compare = |from: usize, ours: &[f32]| {
        let theirs = &reference[from * c..from * c + ours.len()];
        if let Some(i) = ours.iter().zip(theirs).position(|(a, b)| a != b) {
            panic!("{name}: frame {} channel {} differs: ours {}, ffmpeg {}", from + i / c, i % c, ours[i], theirs[i]);
        }
    };
    compare(0, &read(0, frames));
    // a jump into the middle, then on
    let mid = frames / 2 + 777;
    compare(mid, &read(mid, frames / 4));
}

#[test]
fn adpcm_in_containers() {
    for &(name, codec, rate, ch, trim) in CASES {
        check(name, codec, rate, ch, trim);
    }
}
