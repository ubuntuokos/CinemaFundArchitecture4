//! G.711, G.722, IMA ADPCM and Microsoft ADPCM in WAV, AVI, Matroska (`A_MS/ACM`) and QuickTime,
//! read through the media sources, against ffmpeg's decode of the same file: every sample equal,
//! read front to back and from the middle (a seek).

mod common;

use common::*;

/// Tones, a sweep, noise and full-scale clipping.
const SOURCE: &str = "aevalsrc=exprs='if(lt(t,1),0.5*sin(2*PI*440*t),if(lt(t,2),0.9*sin(2*PI*(100+3000*t)*t),if(lt(t,3),(random(0)-0.5)*1.6,1.2*sin(2*PI*1500*t))))|0.3*sin(2*PI*220*t)':s=RATE:d=3.6";

/// (file, ffmpeg encoder, rate, channels, alignment found by search). ffmpeg's G.722 encoder
/// declares 22 samples of delay, which Matroska keeps as `CodecDelay` (we discard it, as the
/// format says) next to a −1 ms first timestamp; ffmpeg releases differ in how much of that their
/// raw decode drops, so for that file only the alignment is searched (within ±64 frames) and the
/// decode must then match sample for sample.
const CASES: &[(&str, &str, u32, u32, bool)] = &[
    ("mulaw.wav", "pcm_mulaw", 8000, 2, false),
    ("alaw.wav", "pcm_alaw", 8000, 1, false),
    ("g722.wav", "g722", 16000, 1, false),
    ("ima_wav.wav", "adpcm_ima_wav", 22050, 2, false),
    ("ms.wav", "adpcm_ms", 22050, 2, false),
    ("mulaw.avi", "pcm_mulaw", 8000, 1, false),
    ("alaw.avi", "pcm_alaw", 8000, 2, false),
    ("ima_wav.avi", "adpcm_ima_wav", 44100, 2, false),
    ("ms.avi", "adpcm_ms", 22050, 1, false),
    ("g722.avi", "g722", 16000, 1, false),
    ("ima_wav.mkv", "adpcm_ima_wav", 22050, 1, false),
    ("ms.mkv", "adpcm_ms", 44100, 2, false),
    ("mulaw.mkv", "pcm_mulaw", 8000, 1, false),
    ("g722.mkv", "g722", 16000, 1, true),
    ("mulaw.mov", "pcm_mulaw", 8000, 2, false),
    ("alaw.mov", "pcm_alaw", 8000, 1, false),
    ("ima4.mov", "adpcm_ima_qt", 44100, 2, false),
];

fn check(name: &str, codec: &str, rate: u32, ch: u32, search: bool) {
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
    // ours[i] = ffmpeg[i + shift]; a negative shift drops our first frames instead
    let shift: i64 = if search {
        let head = read(0, 4000);
        (-64..=64i64)
            .find(|&k| {
                let (o, r) = if k >= 0 { (&head[..], decoded.get(k as usize * c..)) } else { (&head[(-k) as usize * c..], Some(&decoded[..])) };
                r.is_some_and(|r| o.iter().zip(r).all(|(a, b)| a == b) && r.len() >= o.len())
            })
            .unwrap_or_else(|| panic!("{name}: no alignment within ±64 frames matches ffmpeg"))
    } else {
        0
    };
    let skip = shift.unsigned_abs() as usize;
    let (ours_from, reference) = if shift >= 0 { (0, &decoded[skip * c..]) } else { (skip, &decoded[..]) };
    let frames = reference.len() / c;
    let compare = |from: usize, ours: &[f32]| {
        let theirs = &reference[from * c..from * c + ours.len()];
        if let Some(i) = ours.iter().zip(theirs).position(|(a, b)| a != b) {
            panic!("{name}: frame {} channel {} differs: ours {}, ffmpeg {}", from + i / c, i % c, ours[i], theirs[i]);
        }
    };
    compare(0, &read(ours_from, frames));
    // a jump into the middle, then on
    let mid = frames / 2 + 777;
    compare(mid, &read(mid + ours_from, frames / 4));
}

#[test]
fn adpcm_in_containers() {
    for &(name, codec, rate, ch, search) in CASES {
        check(name, codec, rate, ch, search);
    }
}
