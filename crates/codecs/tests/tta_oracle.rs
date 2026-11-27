//! TTA in `.tta` files and Matroska (`A_TTA1`), read through the media sources, against ffmpeg's
//! decode of the same file: every sample equal (as f32 of the s32 decode), read front to back and
//! from the middle (a seek).

mod common;

use common::*;

/// Tones, a sweep, noise and full-scale clipping.
const SOURCE: &str = "aevalsrc=exprs='if(lt(t,1),0.5*sin(2*PI*440*t),if(lt(t,2),0.9*sin(2*PI*(100+3000*t)*t),if(lt(t,3),(random(0)-0.5)*1.6,1.2*sin(2*PI*1500*t))))|0.3*sin(2*PI*220*t)':s=RATE:d=3.6";

/// (file, sample format, rate, channels).
const CASES: &[(&str, &str, u32, u32)] = &[
    ("s16.tta", "s16", 44100, 2),
    ("s24.tta", "s32", 48000, 2),
    ("u8.tta", "u8", 22050, 1),
    ("s16.mka", "s16", 44100, 2),
    ("s24.mka", "s32", 96000, 1),
    ("u8.mka", "u8", 32000, 2),
];

fn check(name: &str, fmt: &str, rate: u32, ch: u32) {
    let ff = cfa4_testkit::require_ffmpeg!();
    let src = SOURCE.replace("RATE", &rate.to_string());
    let layout = if ch == 1 { format!("pan=mono|c0=c0,aformat=sample_fmts={fmt}") } else { format!("aformat=sample_fmts={fmt}") };
    let file = format!("tta_{name}");
    let Some(path) = fixture(&ff, &file, &["-f", "lavfi", "-i", &src, "-af", &layout, "-c:a", "tta"]) else {
        cfa4_testkit::oracle::skip(name, "ffmpeg can't write it");
        return;
    };
    let decoded: Vec<f32> = ffmpeg_out(&ff, &["-i", path.to_str().unwrap(), "-map", "0:a:0", "-f", "s32le", "-c:a", "pcm_s32le", "-"])
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| (i32::from_le_bytes(*b) >> 8) as f32 / 8_388_608.0)
        .collect();
    let c = ch as usize;
    let s = cfa4_codecs::open_bytes(&file, bytes(&path)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let info = s.info().audio().unwrap_or_else(|| panic!("{name}: no audio"));
    assert_eq!((info.sample_rate, info.channels), (rate, ch), "{name}");
    assert_eq!(info.codec, "TTA", "{name}: codec label");
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
    let frames = decoded.len() / c;
    let all = read(0, frames);
    if let Some(i) = all.iter().zip(&decoded).position(|(a, b)| a != b) {
        panic!("{name}: sample {i} differs: ours {}, ffmpeg {}", all[i], decoded[i]);
    }
    // a jump into the middle
    let mid = frames / 2 + 123;
    let part = read(mid, 3000);
    assert!(part == decoded[mid * c..(mid + 3000) * c], "{name}: read from the middle differs");
}

#[test]
fn tta_in_files_and_matroska_matches_ffmpeg() {
    for &(name, fmt, rate, ch) in CASES {
        check(name, fmt, rate, ch);
    }
}
