//! WavPack in `.wv` files and Matroska (`A_WAVPACK4`), read through the media sources, against
//! ffmpeg's decode of the same file: every sample equal, read front to back and from the middle
//! (a seek). Hybrid and float files from the reference `wavpack` encoder are remuxed into Matroska
//! when that CLI is installed.

mod common;

use std::process::Command;

use common::*;

/// Tones, a sweep, noise, silence and clipping, different on every channel.
const SOURCE: &str = "aevalsrc=exprs='if(lt(t,1),0.5*sin(2*PI*440*t),if(lt(t,2),0.9*sin(2*PI*(100+3000*t)*t),if(lt(t,3),(random(0)-0.5)*1.6,1.2*sin(2*PI*1500*t))))|0.3*sin(2*PI*220*t)|0.2*sin(2*PI*3*t)|(random(1)-0.5)*0.1|0.6*sin(2*PI*90*t)|0.4*sin(2*PI*5000*t)':s=RATE:d=3.6";

/// (file, sample format, rate, channels, extra encoder arguments).
const CASES: &[(&str, &str, u32, u32, &[&str])] = &[
    ("s16.wv", "s16p", 44100, 2, &[]),
    ("u8_mono.wv", "u8p", 22050, 1, &[]),
    ("s32_l6.wv", "s32p", 48000, 2, &["-compression_level", "6"]),
    ("flt.wv", "fltp", 48000, 2, &[]),
    ("s16.mkv", "s16p", 48000, 2, &[]),
    ("s16_51.mkv", "s16p", 48000, 6, &["-compression_level", "3"]),
    ("s32_3ch.mkv", "s32p", 96000, 3, &[]),
    ("flt_mono.mkv", "fltp", 44100, 1, &[]),
];

fn compare_source(name: &str, path: &std::path::Path, rate: u32, ch: u32) {
    let ff = cfa4_testkit::require_ffmpeg!();
    let decoded: Vec<f32> = ffmpeg_out(&ff, &["-i", path.to_str().unwrap(), "-map", "0:a:0", "-f", "f32le", "-c:a", "pcm_f32le", "-"])
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    let c = ch as usize;
    let s = cfa4_codecs::open_bytes(name, bytes(path)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let info = s.info().audio().unwrap_or_else(|| panic!("{name}: no audio"));
    assert_eq!((info.sample_rate, info.channels, info.codec.as_str()), (rate, ch, "WavPack"), "{name}");
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
    assert!(frames > rate as usize, "{name}: ffmpeg decoded {frames} frames");
    let compare = |from: usize, ours: &[f32]| {
        let theirs = &decoded[from * c..from * c + ours.len()];
        if let Some(i) = ours.iter().zip(theirs).position(|(a, b)| a.to_bits() != b.to_bits()) {
            panic!("{name}: frame {} channel {} differs: ours {}, ffmpeg {}", from + i / c, i % c, ours[i], theirs[i]);
        }
    };
    compare(0, &read(0, frames));
    let mid = frames / 2 + 777;
    compare(mid, &read(mid, frames / 4));
    // nothing after the end
    let tail = s.audio(frames as i64, 64, rate).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(tail.channels.iter().all(|p| p.iter().all(|v| *v == 0.0)), "{name}: audio past the end");
}

#[test]
fn wavpack_in_containers() {
    let ff = cfa4_testkit::require_ffmpeg!();
    for &(name, fmt, rate, ch, extra) in CASES {
        let src = SOURCE.replace("RATE", &rate.to_string());
        let pan = format!("pan={ch}c|{}", (0..ch).map(|i| format!("c{i}=c{i}")).collect::<Vec<_>>().join("|"));
        let file = format!("wavpack_{name}");
        let mut args = vec!["-f", "lavfi", "-i", &src, "-af", &pan, "-c:a", "wavpack", "-sample_fmt", fmt];
        args.extend_from_slice(extra);
        let Some(path) = fixture(&ff, &file, &args) else {
            cfa4_testkit::oracle::skip(name, "ffmpeg can't write it");
            return;
        };
        compare_source(&file, &path, rate, ch);
    }
}

/// Hybrid (lossy) and float streams of the reference encoder, remuxed into Matroska by ffmpeg.
#[test]
fn reference_hybrid_streams_in_matroska() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let Some(wavpack) = cfa4_testkit::oracle::find_tool("wavpack") else {
        eprintln!("SKIPPED (wavpack in Matroska): the reference wavpack CLI is not installed");
        return;
    };
    let cases: &[(&str, u32, u32, &str, &[&str])] = &[
        ("hyb.mkv", 44100, 2, "pcm_s16le", &["-b3"]),
        ("hyb_6ch.mkv", 48000, 6, "pcm_s24le", &["-b3.5", "-h"]),
        ("hyb_flt.mkv", 48000, 2, "pcm_f32le", &["-b4", "-x2"]),
    ];
    for &(name, rate, ch, pcm, opts) in cases {
        let src = SOURCE.replace("RATE", &rate.to_string());
        let pan = format!("pan={ch}c|{}", (0..ch).map(|i| format!("c{i}=c{i}")).collect::<Vec<_>>().join("|"));
        let Some(wav) = fixture(&ff, &format!("wavpack_{name}.wav"), &["-f", "lavfi", "-i", &src, "-af", &pan, "-c:a", pcm]) else { return };
        let wv = wav.with_extension("wv");
        let ok = Command::new(&wavpack).args(["-q", "-y"]).args(opts).arg(&wav).arg("-o").arg(&wv).status().is_ok_and(|s| s.success());
        assert!(ok, "{name}: wavpack failed");
        let file = format!("wavpack_{name}");
        let Some(path) = fixture(&ff, &file, &["-i", wv.to_str().unwrap(), "-c:a", "copy"]) else {
            cfa4_testkit::oracle::skip(name, "ffmpeg can't remux it");
            return;
        };
        compare_source(&file, &path, rate, ch);
    }
}
