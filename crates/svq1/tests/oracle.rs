//! The SVQ1 decoder against ffmpeg (the external oracle) on FATE samples. The samples are never
//! committed: fetch them into `target/fixtures/fate/` (see README); tests skip without them. The
//! QuickTime files are remuxed to AVI with `-c copy` at their own frame rate (no extradata, as ffmpeg's decoder sees it
//! in both runs).

use std::path::{Path, PathBuf};
use std::process::Command;

fn fate(name: &str) -> Option<PathBuf> {
    let p = cfa4_testkit::fixtures::fixtures_root().join("fate").join(name);
    p.exists().then_some(p)
}

/// The `NNdc` / `NNdb` video chunks of an AVI, in order (empty ones are dropped).
fn video_chunks(file: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 12;
    while i + 8 <= file.len() {
        let size = u32::from_le_bytes(file[i + 4..i + 8].try_into().unwrap()) as usize;
        if &file[i..i + 4] == b"LIST" {
            i += 12;
            continue;
        }
        if file[i..i + 2].iter().all(u8::is_ascii_digit) && matches!(&file[i + 2..i + 4], b"dc" | b"db") && size > 0 && i + 8 + size <= file.len() {
            out.push(file[i + 8..i + 8 + size].to_vec());
        }
        i += 8 + size + (size & 1);
    }
    out
}

fn ffmpeg_raw(ff: &Path, file: &Path) -> Vec<u8> {
    Command::new(ff)
        .args(["-v", "error", "-threads", "1", "-i"])
        .arg(file)
        .args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "yuv410p", "-"])
        .output()
        .expect("run ffmpeg")
        .stdout
}

/// Remux `name` into an AVI in the target directory.
fn avi_of(ff: &Path, name: &str, fps: &str) -> Option<PathBuf> {
    let src = fate(name)?;
    let out = cfa4_testkit::fixtures::fixtures_root().join("svq1-oracle").join(format!("{}.avi", name.replace('/', "_")));
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let ok = Command::new(ff)
        .args(["-v", "error", "-y", "-i"])
        .arg(&src)
        .args(["-an", "-c:v", "copy", "-tag:v", "SVQ1", "-r", fps])
        .arg(&out)
        .status()
        .ok()?
        .success();
    ok.then_some(out)
}

fn raw(p: &cfa4_svq1::Picture) -> Vec<u8> {
    p.planes.iter().flatten().copied().collect()
}

fn check(name: &str, fps: &str) {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("svq1 oracle") else { return };
    let Some(path) = avi_of(&ff, name, fps) else {
        eprintln!("no FATE sample {name} (or no remux), skipping");
        return;
    };
    let file = std::fs::read(&path).unwrap();
    let mut dec = cfa4_svq1::Decoder::new(0, 0, false);
    let mut ours = Vec::new();
    let (mut frames, mut errors) = (0, 0);
    for (n, c) in video_chunks(&file).iter().enumerate() {
        match dec.decode(c) {
            Ok(p) => {
                ours.extend(raw(&p));
                frames += 1;
            }
            Err(e) => {
                eprintln!("chunk {n}: {e}");
                errors += 1;
            }
        }
    }
    let theirs = ffmpeg_raw(&ff, &path);
    eprintln!("{name}: {frames} frames, {errors} errors, {} bytes (ffmpeg {})", ours.len(), theirs.len());
    assert!(frames > 5, "only {frames} frames");
    assert_eq!(theirs.len(), ours.len(), "length");
    assert!(theirs == ours, "differs from ffmpeg");
}

#[test]
fn svq1_ct_ending_matches_ffmpeg() {
    check("svq1/ct_ending_cut.mov", "10");
}

#[test]
fn svq1_marymary_matches_ffmpeg() {
    check("svq1/marymary-shackles.mov", "15");
}

#[test]
fn svq1_damaged_frames_never_panic() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("svq1 damage") else { return };
    let Some(path) = avi_of(&ff, "svq1/marymary-shackles.mov", "15") else { return };
    let chunks = video_chunks(&std::fs::read(path).unwrap());
    let mut x = 0x1234_5678u64;
    let mut rnd = |n: usize| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x >> 16) as usize % n.max(1)
    };
    let key = chunks.iter().find(|c| cfa4_svq1::Decoder::new(0, 0, false).decode(c).is_ok_and(|p| p.key)).cloned();
    for c in chunks.iter().filter(|c| c.len() > 40).take(8) {
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
            let mut dec = cfa4_svq1::Decoder::new(0, 0, false);
            if let Some(k) = &key {
                let _ = dec.decode(k);
            }
            let _ = dec.decode(&bad);
            let _ = dec.decode(c);
        }
    }
}

/// `frame_type` agrees with the decoder about which frames are key frames.
#[test]
fn frame_type_matches_the_decoder() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("svq1 frame types") else { return };
    let Some(path) = avi_of(&ff, "svq1/marymary-shackles.mov", "15") else { return };
    let mut dec = cfa4_svq1::Decoder::new(0, 0, false);
    let mut keys = 0;
    for c in video_chunks(&std::fs::read(path).unwrap()) {
        let t = cfa4_svq1::frame_type(&c);
        let p = dec.decode(&c).unwrap();
        assert_eq!(t == Some(0), p.key);
        keys += usize::from(p.key);
    }
    assert!(keys > 5);
}
