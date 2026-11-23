//! The FLIC decoder against ffmpeg's `flic` decoder (the external oracle), on streams written by
//! a small encoder here: every chunk type (256- and 64-level palettes, byte run, line compressed,
//! delta, black, copy, mini), odd sizes, Magic Carpet files, plus damaged streams that must never
//! panic.

use std::path::Path;
use std::process::Command;

use cfa4_flic::synth::{Rng, packets, stream};
use cfa4_flic::{Decoder, Header};

fn ours(file: &[u8]) -> Result<(u32, u32, Vec<u8>, usize), String> {
    let h = Header::parse(file).map_err(|e| e.to_string())?;
    let mut d = Decoder::new(&h).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let mut n = 0;
    for p in packets(file, &h) {
        d.decode(&p).map_err(|e| e.to_string())?;
        for px in d.rgba().chunks(4) {
            out.extend_from_slice(&px[..3]);
        }
        n += 1;
    }
    Ok((h.width, h.height, out, n))
}

fn ffmpeg_decode(ff: &Path, file: &[u8], tag: &str) -> Vec<u8> {
    let dir = cfa4_testkit::fixtures::fixtures_dir("flic");
    let path = dir.join(format!("{tag}.flc"));
    std::fs::write(&path, file).expect("write flc");
    let out = Command::new(ff)
        .args(["-v", "error", "-threads", "1", "-i"])
        .arg(&path)
        .args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .expect("run ffmpeg");
    out.stdout
}

#[test]
fn random_streams_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("flic oracle") else { return };
    let cases: [(u64, usize, usize, usize, u16); 10] = [
        (1, 64, 48, 12, 0xAF12),
        (2, 320, 200, 10, 0xAF11),
        (3, 97, 61, 12, 0xAF12),
        (4, 33, 17, 15, 0xAF44),
        (5, 160, 100, 20, 0xAF11),
        (6, 18, 90, 12, 0xAF12),
        (7, 255, 4, 12, 0xAF12),
        (8, 130, 120, 12, 0xAF11),
        (9, 64, 40, 20, 0xAF12),
        (10, 128, 50, 20, 0xAF11),
    ];
    for (seed, w, h, frames, magic) in cases {
        let file = stream(seed, w, h, frames, magic);
        let tag = format!("random{seed}");
        let (_, _, mine, n) = ours(&file).unwrap_or_else(|e| panic!("{tag}: {e}"));
        assert_eq!(n, frames, "{tag}");
        let theirs = ffmpeg_decode(&ff, &file, &tag);
        assert_eq!(theirs.len(), mine.len(), "{tag}: ffmpeg {} bytes, we {}", theirs.len(), mine.len());
        if let Some(i) = theirs.iter().zip(&mine).position(|(a, b)| a != b) {
            panic!("{tag}: differs from ffmpeg at byte {i} (frame {})", i / (w * h * 3));
        }
    }
}

#[test]
fn damaged_streams_do_not_panic() {
    let mut rng = Rng(99);
    for round in 0..400u64 {
        let w = 8 + rng.below(100) as usize;
        let h = 4 + rng.below(60) as usize;
        let mut file = stream(round, w, h, 6, 0xAF12);
        for _ in 0..1 + rng.below(8) {
            if file.len() <= 130 {
                break;
            }
            let i = 128 + rng.below((file.len() - 128) as u32) as usize;
            match rng.below(3) {
                0 => file[i] ^= 1 << rng.below(8),
                1 => file[i] = rng.below(256) as u8,
                _ => file.truncate(i),
            }
        }
        if file.len() < 134 {
            continue;
        }
        let _ = ours(&file);
        // a header that claims other sizes
        file[8] = rng.below(256) as u8;
        file[10] = rng.below(256) as u8;
        let _ = ours(&file);
    }
}

#[test]
fn probe_rules() {
    let mut f = stream(1, 64, 48, 3, 0xAF12);
    assert!(cfa4_flic::probe(&f));
    f[4] = 0;
    assert!(!cfa4_flic::probe(&f));
    assert!(!cfa4_flic::probe(&[0; 64]));
}
