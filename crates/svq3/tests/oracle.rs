//! The SVQ3 decoder against ffmpeg (the external oracle) on FATE samples. The samples are never
//! committed: fetch them into `target/fixtures/fate/svq3/` (see README); tests skip without them.
//! The QuickTime files are remuxed to AVI with `-c copy` (the decoder's extradata travels in the
//! `strf` chunk), so both decoders see the same packets and the same extradata.

use std::path::{Path, PathBuf};
use std::process::Command;

fn fate(name: &str) -> Option<PathBuf> {
    let p = cfa4_testkit::fixtures::fixtures_root().join("fate").join(name);
    p.exists().then_some(p)
}

/// The video chunks of an AVI (empty ones dropped) and the extradata after its `BITMAPINFOHEADER`.
fn avi_video(file: &[u8]) -> (Vec<u8>, Vec<Vec<u8>>) {
    let (mut chunks, mut extra) = (Vec::new(), Vec::new());
    let mut i = 12;
    while i + 8 <= file.len() {
        let size = u32::from_le_bytes(file[i + 4..i + 8].try_into().unwrap()) as usize;
        if &file[i..i + 4] == b"LIST" {
            i += 12;
            continue;
        }
        if &file[i..i + 4] == b"strf" && size >= 40 && &file[i + 8 + 16..i + 8 + 20] == b"SVQ3" && extra.is_empty() {
            extra = file[i + 8 + 40..i + 8 + size].to_vec();
        }
        if file[i..i + 2].iter().all(u8::is_ascii_digit) && matches!(&file[i + 2..i + 4], b"dc" | b"db") && size > 0 && i + 8 + size <= file.len() {
            chunks.push(file[i + 8..i + 8 + size].to_vec());
        }
        i += 8 + size + (size & 1);
    }
    (extra, chunks)
}

fn ffmpeg_raw(ff: &Path, file: &Path) -> Vec<u8> {
    Command::new(ff)
        .args(["-v", "error", "-threads", "1", "-i"])
        .arg(file)
        .args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "yuvj420p", "-"])
        .output()
        .expect("run ffmpeg")
        .stdout
}

/// Remux `name` into an AVI in the target directory (video only; once, tests run in parallel).
fn avi_of(ff: &Path, name: &str) -> Option<PathBuf> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let src = fate(name)?;
    let out = cfa4_testkit::fixtures::fixtures_root().join("svq3-oracle").join(format!("{}.avi", name.replace('/', "_")));
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    if out.metadata().is_ok_and(|m| m.len() > 0 && m.modified().ok() > src.metadata().and_then(|s| s.modified()).ok()) {
        return Some(out);
    }
    let ok = Command::new(ff)
        .args(["-v", "error", "-y", "-i"])
        .arg(&src)
        .args(["-an", "-map", "0:v:0", "-c:v", "copy", "-tag:v", "SVQ3", "-r", "25"])
        .arg(&out)
        .status()
        .ok()?
        .success();
    ok.then_some(out)
}

fn raw(p: &cfa4_svq3::Picture) -> Vec<u8> {
    p.planes.iter().flatten().copied().collect()
}

fn decode_all(extra: &[u8], chunks: &[Vec<u8>]) -> (Vec<u8>, usize, usize) {
    let mut dec = cfa4_svq3::Decoder::new(0, 0, extra).expect("decoder");
    let (mut ours, mut frames, mut errors) = (Vec::new(), 0, 0);
    for (n, c) in chunks.iter().enumerate() {
        match dec.decode(c, n as i64) {
            Ok(Some(o)) => {
                ours.extend(raw(&o.picture));
                frames += 1;
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("chunk {n}: {e}");
                errors += 1;
            }
        }
    }
    while let Some(o) = dec.flush() {
        ours.extend(raw(&o.picture));
        frames += 1;
    }
    (ours, frames, errors)
}

fn check(name: &str, min_frames: usize) {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("svq3 oracle") else { return };
    let Some(path) = avi_of(&ff, name) else {
        eprintln!("no FATE sample {name} (or no remux), skipping");
        return;
    };
    let (extra, chunks) = avi_video(&std::fs::read(&path).unwrap());
    let (ours, frames, errors) = decode_all(&extra, &chunks);
    let theirs = ffmpeg_raw(&ff, &path);
    if let Ok(dir) = std::env::var("SVQ3_DUMP") {
        std::fs::write(format!("{dir}/ours.yuv"), &ours).unwrap();
        std::fs::write(format!("{dir}/theirs.yuv"), &theirs).unwrap();
    }
    eprintln!("{name}: {frames} frames, {errors} errors, {} bytes (ffmpeg {})", ours.len(), theirs.len());
    assert!(frames >= min_frames, "only {frames} frames");
    assert_eq!(theirs.len(), ours.len(), "length");
    if theirs != ours {
        // where do they first differ?
        let (w, h) = cfa4_svq3::Decoder::new(0, 0, &extra).unwrap().size();
        let (w, h) = (w as usize, h as usize);
        let fsize = w * h + 2 * w.div_ceil(2) * h.div_ceil(2);
        for (n, (a, b)) in theirs.chunks(fsize).zip(ours.chunks(fsize)).enumerate() {
            if let Some(i) = a.iter().zip(b).position(|(x, y)| x != y) {
                let (plane, off, pw) = if i < w * h {
                    ("Y", i, w)
                } else if i < w * h + (fsize - w * h) / 2 {
                    ("U", i - w * h, w.div_ceil(2))
                } else {
                    ("V", i - w * h - (fsize - w * h) / 2, w.div_ceil(2))
                };
                let n_diff = a.iter().zip(b).filter(|(x, y)| x != y).count();
                panic!("{name}: frame {n} differs from ffmpeg first at {plane} ({}, {}), {n_diff} samples differ", off % pw, off / pw);
            }
        }
        panic!("differs from ffmpeg");
    }
}

#[test]
fn svq3_vertical_matches_ffmpeg() {
    check("svq3/Vertical400kbit.sorenson3.mov", 100);
}

#[test]
fn svq3_decoding_regression_matches_ffmpeg() {
    check("svq3/svq3_decoding_regression.mov", 5);
}

#[test]
fn svq3_watermark_matches_ffmpeg() {
    check("svq3/svq3_watermark.mov", 5);
}

/// Truncated and corrupted packets (and sequence headers) are errors, never panics.
#[test]
fn svq3_damaged_input_never_panics() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("svq3 damage") else { return };
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    let mut rnd = |n: usize| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x >> 16) as usize % n.max(1)
    };
    for name in ["svq3/svq3_decoding_regression.mov", "svq3/svq3_watermark.mov", "svq3/Vertical400kbit.sorenson3.mov"] {
        let Some(path) = avi_of(&ff, name) else { continue };
        let (extra, chunks) = avi_video(&std::fs::read(path).unwrap());
        // damaged sequence headers
        for _ in 0..200 {
            let mut e = extra.clone();
            for _ in 0..3 {
                if !e.is_empty() {
                    let i = rnd(e.len());
                    e[i] = rnd(256) as u8;
                }
            }
            e.truncate(e.len() - rnd(e.len().min(8) + 1));
            let _ = cfa4_svq3::Decoder::new(64, 64, &e);
        }
        // damaged packets, after an intact start so that references exist
        for c in chunks.iter().skip(1).take(6) {
            for _ in 0..60 {
                let mut bad = c.clone();
                if rnd(2) == 0 {
                    bad.truncate(rnd(bad.len()));
                } else {
                    for _ in 0..4 {
                        let i = rnd(bad.len());
                        bad[i] = rnd(256) as u8;
                    }
                }
                let mut dec = cfa4_svq3::Decoder::new(0, 0, &extra).unwrap();
                for k in &chunks[..2] {
                    let _ = dec.decode(k, 0);
                }
                let _ = dec.decode(&bad, 1);
                let _ = dec.decode(c, 2);
                let _ = dec.flush();
            }
        }
    }
}

/// With B pictures the output lags by one reference picture; `reset` forgets the pictures.
#[test]
fn svq3_output_delay_and_reset() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("svq3 delay") else { return };
    let Some(path) = avi_of(&ff, "svq3/svq3_watermark.mov") else { return };
    let (extra, chunks) = avi_video(&std::fs::read(path).unwrap());
    let mut dec = cfa4_svq3::Decoder::new(0, 0, &extra).unwrap();
    let outs: Vec<_> = chunks
        .iter()
        .enumerate()
        .map(|(n, c)| {
            dec.decode(c, n as i64).unwrap().map(|o| {
                // the picture's type is what `frame_type` says of its packet
                assert_eq!(o.picture.key, dec.frame_type(&chunks[o.tag as usize]) == Some(cfa4_svq3::PicType::I));
                o.tag
            })
        })
        .collect();
    for t in [cfa4_svq3::PicType::I, cfa4_svq3::PicType::P, cfa4_svq3::PicType::B] {
        assert!(chunks.iter().any(|c| dec.frame_type(c) == Some(t)), "no {t:?} picture");
    }
    // every packet's picture comes out exactly once, the last ones from `flush`
    let mut tags: Vec<i64> = outs.iter().flatten().copied().collect();
    while let Some(o) = dec.flush() {
        tags.push(o.tag);
    }
    tags.sort_unstable();
    assert_eq!(tags, (0..chunks.len() as i64).collect::<Vec<_>>(), "outs {outs:?}");
    dec.reset();
    assert!(dec.flush().is_none());
}
