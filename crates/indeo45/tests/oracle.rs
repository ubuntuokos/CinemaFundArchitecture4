//! The Indeo 4 and 5 decoders against ffmpeg (the external oracle) on FATE samples. The samples are
//! never committed: fetch them into `target/fixtures/fate/` (see README); tests skip without them.

use std::path::{Path, PathBuf};
use std::process::Command;

fn fate(name: &str) -> Option<PathBuf> {
    let p = cfa4_testkit::fixtures::fixtures_root().join("fate").join(name);
    p.exists().then_some(p)
}

/// The `NNdc` / `NNdb` video chunks of an AVI, in order.
fn video_chunks(file: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 12;
    while i + 8 <= file.len() {
        let size = u32::from_le_bytes(file[i + 4..i + 8].try_into().unwrap()) as usize;
        if &file[i..i + 4] == b"LIST" {
            i += 12;
            continue;
        }
        if file[i..i + 2].iter().all(u8::is_ascii_digit) && (&file[i + 2..i + 4] != b"wb" && &file[i + 2..i + 4] != b"pc") && i + 8 + size <= file.len() {
            out.push(file[i + 8..i + 8 + size].to_vec());
        }
        i += 8 + size + (size & 1);
    }
    out
}

fn ffmpeg_raw(ff: &Path, file: &Path, fmt: &str) -> Vec<u8> {
    Command::new(ff)
        .args(["-v", "error", "-threads", "1", "-i"])
        .arg(file)
        .args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", fmt, "-"])
        .output()
        .expect("run ffmpeg")
        .stdout
}

fn raw(p: &cfa4_indeo45::Picture) -> Vec<u8> {
    p.planes.iter().flatten().copied().collect()
}

/// Decode every chunk (damaged ones give no picture, as in ffmpeg) and compare with ffmpeg.
fn check(name: &str, codec: cfa4_indeo45::Codec) {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("indeo45 oracle") else { return };
    let Some(path) = fate(name) else {
        eprintln!("no FATE sample {name}, skipping");
        return;
    };
    let file = std::fs::read(&path).unwrap();
    let mut dec = cfa4_indeo45::Decoder::new(codec);
    let mut ours = Vec::new();
    let (mut frames, mut errors) = (0, 0);
    for (n, c) in video_chunks(&file).iter().enumerate() {
        match dec.decode(c) {
            Ok(Some(p)) => {
                ours.extend(raw(&p));
                frames += 1;
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("chunk {n}: {e}");
                errors += 1;
            }
        }
    }
    let theirs = ffmpeg_raw(&ff, &path, "yuv410p");
    eprintln!("{name}: {frames} frames, {errors} errors, {} bytes (ffmpeg {})", ours.len(), theirs.len());
    assert!(frames > 5, "only {frames} frames");
    assert_eq!(theirs.len(), ours.len(), "length");
    assert!(theirs == ours, "differs from ffmpeg");
}

#[test]
fn indeo4_matches_ffmpeg_on_fate() {
    check("iv41/indeo41-partial.avi", cfa4_indeo45::Codec::Indeo4);
}

#[test]
fn indeo5_matches_ffmpeg_on_fate() {
    check("iv50/Educ_Movie_DeadlyForce.avi", cfa4_indeo45::Codec::Indeo5);
}

fn damaged(name: &str, codec: cfa4_indeo45::Codec) {
    let Some(path) = fate(name) else { return };
    let chunks = video_chunks(&std::fs::read(path).unwrap());
    let mut x = 0x1234_5678u64;
    let mut rnd = |n: usize| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x >> 16) as usize % n.max(1)
    };
    // a key frame first so that the damaged ones have something to refer to
    let key = chunks.iter().find(|c| cfa4_indeo45::Decoder::new(codec).decode(c).is_ok_and(|p| p.is_some())).cloned();
    for c in chunks.iter().filter(|c| c.len() > 16).take(6) {
        for _ in 0..100 {
            let mut bad = c.clone();
            if rnd(2) == 0 {
                bad.truncate(rnd(bad.len()));
            } else {
                for _ in 0..4 {
                    let i = rnd(bad.len());
                    bad[i] = rnd(256) as u8;
                }
            }
            let mut dec = cfa4_indeo45::Decoder::new(codec);
            if let Some(k) = &key {
                let _ = dec.decode(k);
            }
            let _ = dec.decode(&bad);
            let _ = dec.decode(c);
        }
    }
}

#[test]
fn indeo4_damaged_frames_never_panic() {
    damaged("iv41/indeo41-partial.avi", cfa4_indeo45::Codec::Indeo4);
}

#[test]
fn indeo5_damaged_frames_never_panic() {
    damaged("iv50/Educ_Movie_DeadlyForce.avi", cfa4_indeo45::Codec::Indeo5);
}
