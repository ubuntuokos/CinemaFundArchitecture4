//! Ogg Speex files read through the media source: every sample equal to the reference `speexdec`
//! output (16-bit, scaled by 1/32768), front to back and from the middle (a seek). The files come
//! from the reference `speexenc`; the test skips when the Speex tools are not installed
//! (`speexenc` / `speexdec` on the PATH, or `CFA4_SPEEXENC` / `CFA4_SPEEXDEC`).

mod common;

use std::process::{Command, Stdio};

use common::*;

/// (file, rate, channels, `speexenc` options).
const CASES: &[(&str, u32, u32, &[&str])] = &[
    ("nb.spx", 8000, 1, &["-n", "--quality", "5"]),
    ("wb_vbr.spx", 16000, 1, &["-w", "--vbr", "--nframes", "2"]),
    ("uwb.spx", 32000, 1, &["-u", "--quality", "8"]),
    ("wb_stereo.spx", 16000, 2, &["-w", "--quality", "6"]),
];

fn run(cmd: &mut Command) -> bool {
    cmd.stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

#[test]
fn speex_files_read_through_the_media_source() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let (Some(enc), Some(dec)) = (cfa4_testkit::oracle::find_tool("speexenc"), cfa4_testkit::oracle::find_tool("speexdec")) else {
        eprintln!("SKIPPED (speex oracle): speexenc / speexdec are not installed");
        return;
    };
    for &(name, rate, ch, args) in CASES {
        let src = "aevalsrc=exprs='0.4*sin(2*PI*(150+300*t)*t)*(1+0.5*sin(2*PI*5*t))+0.05*(random(0)-0.5)|0.3*sin(2*PI*700*t)':s=RATE:d=3"
            .replace("RATE", &rate.to_string());
        let pan = if ch == 2 { "pan=stereo|c0=c0|c1=c1" } else { "pan=mono|c0=c0" };
        let Some(wav) = fixture(&ff, &format!("speex_{name}.wav"), &["-f", "lavfi", "-i", &src, "-af", pan, "-c:a", "pcm_s16le", "-f", "wav"]) else {
            cfa4_testkit::oracle::skip(name, "ffmpeg can't write the source");
            return;
        };
        let path = wav.with_file_name(format!("speex_{name}"));
        let path = cfa4_testkit::fixtures::generate(&path, |tmp| run(Command::new(&enc).args(args).arg(&wav).arg(tmp)))
            .unwrap_or_else(|| panic!("{name}: speexenc failed"));
        let raw = path.with_extension("raw");
        let raw = cfa4_testkit::fixtures::generate(&raw, |tmp| {
            let t = tmp.with_extension("raw");
            run(Command::new(&dec).arg(&path).arg(&t)) && std::fs::rename(&t, tmp).is_ok()
        })
        .unwrap_or_else(|| panic!("{name}: speexdec failed"));
        let want: Vec<f32> = std::fs::read(&raw).unwrap().as_chunks::<2>().0.iter().map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0).collect();

        let c = ch as usize;
        let s = cfa4_codecs::open_bytes(name, bytes(&path)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let info = s.info().audio().unwrap_or_else(|| panic!("{name}: no audio"));
        assert_eq!((info.sample_rate, info.channels, info.codec.as_str()), (rate, ch, "Speex"), "{name}");
        let frames = want.len() / c;
        assert!(frames > rate as usize, "{name}: {frames} frames");
        let read = |from: usize, n: usize| -> Vec<f32> {
            let b = s.audio(from as i64, n, rate).unwrap_or_else(|e| panic!("{name}: {e}"));
            (0..n).flat_map(|i| b.channels.iter().map(move |p| p[i])).collect()
        };
        let compare = |from: usize, ours: &[f32]| {
            let theirs = &want[from * c..from * c + ours.len()];
            if let Some(i) = ours.iter().zip(theirs).position(|(a, b)| a.to_bits() != b.to_bits()) {
                panic!("{name}: frame {} channel {} differs: ours {}, speexdec {}", from + i / c, i % c, ours[i], theirs[i]);
            }
        };
        compare(0, &read(0, frames));
        let mid = frames / 2 + 333;
        compare(mid, &read(mid, frames / 4));
        let tail = s.audio(frames as i64, 64, rate).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(tail.channels.iter().all(|p| p.iter().all(|v| *v == 0.0)), "{name}: audio past the end");
    }
}
