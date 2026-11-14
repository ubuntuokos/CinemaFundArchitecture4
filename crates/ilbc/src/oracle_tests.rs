//! iLBC against ffmpeg's `ilbc` decoder, which is derived from the same WebRTC code without the
//! enhancer, so the output must match sample for sample.
//!
//! Two differences are set aside here. ffmpeg starts with a zeroed LSF history where WebRTC
//! (and RFC 3951) start from the mean LSF vector, so the comparison starts our decoder from
//! ffmpeg's state; that only changes the first frames of a stream. And ffmpeg neither encodes
//! iLBC nor conceals bad frames, so the streams are random frames re-rolled until each one
//! decodes as a normal (valid) frame.

use std::process::Command;

use crate::{Decoder, Mode};

fn rng(x: &mut u64) -> u64 {
    *x ^= *x << 13;
    *x ^= *x >> 7;
    *x ^= *x << 17;
    *x
}

/// Our decoder in ffmpeg's initial state.
fn ffmpeg_like(mode: Mode) -> Decoder {
    let mut d = Decoder::new(mode);
    d.lsfdeqold = [0; 10];
    d
}

/// An RFC 3952 file of `frames` random valid frames, and our decoding of it.
fn stream(seed: u64, mode: Mode, frames: usize) -> (Vec<u8>, Vec<i16>) {
    let mut x = seed;
    let mut d = ffmpeg_like(mode);
    let mut file = match mode {
        Mode::Ms20 => b"#!iLBC20\n".to_vec(),
        Mode::Ms30 => b"#!iLBC30\n".to_vec(),
    };
    let mut pcm = Vec::new();
    for _ in 0..frames {
        loop {
            let f: Vec<u8> = (0..mode.frame_bytes()).map(|_| rng(&mut x) as u8).collect();
            let mut probe = d.clone();
            let mut out = Vec::new();
            if probe.decode(Some(&f), &mut out) {
                d = probe;
                file.extend_from_slice(&f);
                pcm.extend_from_slice(&out);
                break;
            }
        }
    }
    (file, pcm)
}

fn ffmpeg_decode(ff: &std::path::Path, data: &[u8], tag: &str) -> Option<Vec<i16>> {
    let dir = std::env::temp_dir().join(format!("cfa4-ilbc-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let input = dir.join("in.lbc");
    let output = dir.join("out.raw");
    std::fs::write(&input, data).ok()?;
    let ok =
        Command::new(ff).args(["-v", "error", "-y", "-f", "ilbc", "-i"]).arg(&input).args(["-f", "s16le", "-ac", "1"]).arg(&output).status().ok()?.success();
    let raw = std::fs::read(&output).ok();
    let _ = std::fs::remove_dir_all(&dir);
    if !ok {
        return None;
    }
    Some(raw?.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect())
}

#[test]
fn synthetic_streams_match_ffmpeg_bit_exactly() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("iLBC oracle") else { return };
    for mode in [Mode::Ms20, Mode::Ms30] {
        for seed in 1..=6u64 {
            let (file, ours) = stream(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15), mode, 200);
            let tag = format!("{mode:?}-{seed}");
            let Some(theirs) = ffmpeg_decode(&ff, &file, &tag) else { panic!("{tag}: ffmpeg failed") };
            assert_eq!(ours.len(), theirs.len(), "{tag}");
            let diff = ours.iter().zip(&theirs).filter(|(a, b)| a != b).count();
            let first = ours.iter().zip(&theirs).position(|(a, b)| a != b);
            assert_eq!(diff, 0, "{tag}: {diff}/{} samples differ, first at {first:?}", ours.len());
            // decode_file (from the RFC 3951 initial state) gives the same length and converges
            let file_pcm = crate::decode_file(&file).expect("decode");
            assert_eq!(file_pcm.len(), ours.len(), "{tag}");
            let tail = ours.len() / 2;
            assert_eq!(file_pcm[tail..], ours[tail..], "{tag}");
        }
    }
}
