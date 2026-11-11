//! Monkey's Audio (`.ape`) files read through the media source: every sample equal to the
//! source PCM (the codec is lossless), front to back and from the middle (a seek). The files come
//! from the reference `mac` encoder; the test skips when it is not installed (`mac` on the PATH
//! or `CFA4_MAC`).

mod common;

use std::process::{Command, Stdio};

use common::*;

/// Tones, a sweep, noise, silence and clipping, different on every channel.
const SOURCE: &str = "aevalsrc=exprs='if(lt(t,1),0.5*sin(2*PI*440*t),if(lt(t,2),0.9*sin(2*PI*(100+3000*t)*t),if(lt(t,3),(random(0)-0.5)*1.6,1.2*sin(2*PI*1500*t))))|0.3*sin(2*PI*220*t)|0.2*sin(2*PI*3*t)|(random(1)-0.5)*0.1|0.6*sin(2*PI*90*t)|0.4*sin(2*PI*5000*t)':s=RATE:d=3.6";

/// (file, PCM codec of the source, rate, channels, `mac` level).
const CASES: &[(&str, &str, u32, u32, u32)] = &[
    ("s16.ape", "pcm_s16le", 44100, 2, 2000),
    ("u8_mono.ape", "pcm_u8", 22050, 1, 3000),
    ("s24_insane.ape", "pcm_s24le", 48000, 2, 5000),
    ("s32.ape", "pcm_s32le", 48000, 2, 4000),
    ("flt.ape", "pcm_f32le", 48000, 2, 2000),
    ("s16_51.ape", "pcm_s16le", 48000, 6, 1000),
];

#[test]
fn ape_files_read_through_the_media_source() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let Some(mac) = cfa4_testkit::oracle::find_tool("mac") else {
        eprintln!("SKIPPED (ape oracle): the reference Monkey's Audio `mac` CLI is not installed");
        return;
    };
    for &(name, pcm, rate, ch, level) in CASES {
        let src = SOURCE.replace("RATE", &rate.to_string());
        let pan = format!("pan={ch}c|{}", (0..ch).map(|i| format!("c{i}=c{i}")).collect::<Vec<_>>().join("|"));
        let Some(wav) = fixture(&ff, &format!("ape_{name}.wav"), &["-f", "lavfi", "-i", &src, "-af", &pan, "-c:a", pcm, "-f", "wav"]) else {
            cfa4_testkit::oracle::skip(name, "ffmpeg can't write the source");
            return;
        };
        let path = wav.with_file_name(format!("ape_{name}"));
        let made = cfa4_testkit::fixtures::generate(&path, |tmp| {
            let tmp_ape = tmp.with_extension("ape");
            let ok = Command::new(&mac).arg(&wav).arg(&tmp_ape).arg(format!("-c{level}")).stdout(Stdio::null()).status().is_ok_and(|s| s.success());
            ok && std::fs::rename(&tmp_ape, tmp).is_ok()
        });
        let Some(path) = made else { panic!("{name}: mac could not encode") };

        // The source PCM as ffmpeg's floats: the same scaling as the media source.
        let want: Vec<f32> = ffmpeg_out(&ff, &["-i", wav.to_str().unwrap(), "-f", "f32le", "-c:a", "pcm_f32le", "-"])
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        let c = ch as usize;
        let s = cfa4_codecs::open_bytes(name, bytes(&path)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let info = s.info().audio().unwrap_or_else(|| panic!("{name}: no audio"));
        assert_eq!((info.sample_rate, info.channels, info.codec.as_str()), (rate, ch, "Monkey's Audio"), "{name}");
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
        let frames = want.len() / c;
        assert!(frames > rate as usize, "{name}: {frames} frames");
        let compare = |from: usize, ours: &[f32]| {
            let theirs = &want[from * c..from * c + ours.len()];
            if let Some(i) = ours.iter().zip(theirs).position(|(a, b)| a.to_bits() != b.to_bits()) {
                panic!("{name}: frame {} channel {} differs: ours {}, source {}", from + i / c, i % c, ours[i], theirs[i]);
            }
        };
        compare(0, &read(0, frames));
        let mid = frames / 2 + 777;
        compare(mid, &read(mid, frames / 4));
        let tail = s.audio(frames as i64, 64, rate).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(tail.channels.iter().all(|p| p.iter().all(|v| *v == 0.0)), "{name}: audio past the end");
    }
}
