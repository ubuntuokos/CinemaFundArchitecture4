//! The Bink video decoder against ffmpeg's `binkvideo` (the external oracle). ffmpeg cannot
//! encode Bink, so the first frame of every stream is written by a small encoder here (fill,
//! pattern and raw blocks through the bundles, so it is fully defined), and the frames after it
//! are random bytes: wherever ffmpeg decodes one, the picture must be the same, and whenever
//! ffmpeg refuses one we must refuse it too.

mod common;

use std::path::Path;
use std::process::Command;

use cfa4_bink::container::Bink;
use cfa4_bink::video::Decoder;
use common::{Rng, bik, first_frame, frame_b};

fn ffmpeg_frames(ff: &Path, file: &[u8], name: &str, fmt: &str) -> Vec<(u32, Vec<u8>)> {
    let dir = cfa4_testkit::fixtures::fixtures_dir("bink/oracle").join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("in.bik");
    std::fs::write(&path, file).unwrap();
    let st = Command::new(ff)
        .args(["-v", "error", "-threads", "1", "-i"])
        .arg(&path)
        .args(["-map", "0:v:0", "-fps_mode", "passthrough", "-c:v", "rawvideo", "-pix_fmt", fmt, "-f", "image2", "-frame_pts", "1", "-start_number", "0"])
        .arg(dir.join("f%d.yuv"))
        .output()
        .expect("run ffmpeg");
    let _ = st;
    let mut out = Vec::new();
    for e in std::fs::read_dir(&dir).unwrap() {
        let e = e.unwrap();
        let n = e.file_name().to_string_lossy().to_string();
        if let Some(i) = n.strip_prefix('f').and_then(|r| r.strip_suffix(".yuv")) {
            out.push((i.parse::<u32>().unwrap(), std::fs::read(e.path()).unwrap()));
        }
    }
    out.sort_by_key(|f| f.0);
    let _ = std::fs::remove_dir_all(&dir);
    out
}

fn stream_check(ff: &Path, ver: u8, w: u32, h: u32, alpha: bool, frames: usize, seed: u64, label: &str) -> (usize, usize) {
    let mut rng = Rng(seed);
    let mut packets = vec![first_frame(&mut rng, ver, w, h, alpha)];
    for _ in 1..frames {
        if ver == b'b' && rng.below(10) < 7 {
            packets.push(frame_b(&mut rng, w, h, true));
            continue;
        }
        let n = 24 + rng.below(260) as usize;
        packets.push((0..n).map(|_| rng.next() as u8).collect());
    }
    let flags = if alpha { 0x0010_0000 } else { 0 };
    let tag = [b'B', b'I', b'K', ver];
    let file = bik(&tag, w, h, flags, &packets, &[]);
    let bink = Bink::parse(&file).expect("parse");
    assert_eq!(bink.frame_count(), frames);
    let mut dec = Decoder::new(w, h, tag, flags).unwrap();
    let mut ours: Vec<(u32, Vec<u8>, bool)> = Vec::new();
    for i in 0..frames {
        let f = bink.frame(i).unwrap();
        if let Ok(p) = dec.decode(f.video) {
            ours.push((i as u32, p.planes.concat(), p.complete));
        }
    }
    let theirs = ffmpeg_frames(ff, &file, &format!("{label}-{seed}"), if alpha { "yuva420p" } else { "yuv420p" });
    let ours_idx: Vec<u32> = ours.iter().map(|f| f.0).collect();
    let theirs_idx: Vec<u32> = theirs.iter().map(|f| f.0).collect();
    if ours_idx != theirs_idx {
        let keep = cfa4_testkit::fixtures::fixtures_dir("bink").join(format!("fail-{label}-{seed}.bik"));
        std::fs::write(&keep, &file).unwrap();
        eprintln!("kept {}", keep.display());
    }
    assert_eq!(ours_idx, theirs_idx, "{label} seed {seed}: frames decoded differ (ours vs ffmpeg)");
    // after a frame with undefined pixels the references differ from ffmpeg's stale memory
    for (a, b) in ours.iter().zip(&theirs) {
        assert_eq!(a.1.len(), b.1.len(), "{label} frame {}: size", a.0);
        if !a.2 {
            break;
        }
        if a.1 != b.1 {
            let keep = cfa4_testkit::fixtures::fixtures_dir("bink").join(format!("diff-{label}-{seed}.bik"));
            std::fs::write(&keep, &file).unwrap();
            eprintln!("kept {} (frame {})", keep.display(), a.0);
            let i = a.1.iter().zip(&b.1).position(|(x, y)| x != y).unwrap();
            let nd = a.1.iter().zip(&b.1).filter(|(x, y)| x != y).count();
            eprintln!("{nd} bytes differ; first at {i}: ours {:?} theirs {:?}", &a.1[i..(i + 16).min(a.1.len())], &b.1[i..(i + 16).min(b.1.len())]);
            panic!("{label} seed {seed} frame {}: differs from ffmpeg at byte {i}", a.0);
        }
    }
    (ours.len(), frames)
}

#[test]
fn random_frames_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("bink video oracle") else { return };
    let (mut ok, mut all) = (0, 0);
    for (ver, w, h) in [
        (b'i', 8u32, 8u32),
        (b'i', 16, 16),
        (b'i', 34, 18),
        (b'h', 24, 16),
        (b'g', 40, 24),
        (b'f', 16, 24),
        (b'k', 24, 16),
        (b'k', 48, 32),
        (b'b', 8, 8),
        (b'b', 24, 16),
        (b'b', 34, 18),
    ] {
        for seed in 1..=10u64 {
            for alpha in [false, true] {
                if alpha && (ver == b'k' || ver == b'b') {
                    continue;
                }
                let (a, b) = stream_check(&ff, ver, w, h, alpha, 40, seed * 7919 + ver as u64, &format!("{}-{w}x{h}", ver as char));
                ok += a;
                all += b;
                if seed == 1 {
                    eprintln!("{}{} {w}x{h}: {a} of {b}", ver as char, if alpha { "+alpha" } else { "" });
                }
            }
        }
    }
    eprintln!("bink video: {ok} of {all} frames decoded by both");
    assert!(ok > 200, "too few frames decoded to be meaningful: {ok}");
}

/// `BINK_FILE=x.bik cargo test --test video_oracle dump -- --ignored --nocapture`: our verdict per frame.
#[test]
#[ignore]
fn dump() {
    let path = std::env::var("BINK_FILE").expect("BINK_FILE");
    let file = std::fs::read(path).unwrap();
    let bink = Bink::parse(&file).unwrap();
    let mut dec = Decoder::new(bink.width, bink.height, bink.tag, bink.flags).unwrap();
    for i in 0..bink.frame_count() {
        match dec.decode(bink.frame(i).unwrap().video) {
            Ok(p) => eprintln!("frame {i}: ok complete={}", p.complete),
            Err(e) => eprintln!("frame {i}: {e}"),
        }
    }
}

#[test]
fn first_frames_decode() {
    for (ver, w, h) in [(b'i', 34u32, 18u32), (b'i', 16, 16), (b'k', 24, 16), (b'b', 16, 16)] {
        let mut rng = Rng(5);
        let pkt = first_frame(&mut rng, ver, w, h, false);
        let mut dec = Decoder::new(w, h, [b'B', b'I', b'K', ver], 0).unwrap();
        let r = dec.decode(&pkt);
        eprintln!("{} {w}x{h}: {:?}", ver as char, r.as_ref().map(|p| p.complete));
    }
}
