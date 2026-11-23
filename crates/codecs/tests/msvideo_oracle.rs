//! Microsoft Video 1 and Microsoft RLE (`cfa4-msvideo`) through the containers against ffmpeg:
//! files whose pictures decode to ffmpeg's, frame by frame, also out of order (a seek decodes
//! from the nearest key frame and must give the same picture). The decoders themselves are
//! compared with ffmpeg in `crates/msvideo/tests/oracle.rs`.

mod common;

use common::*;

/// A minimal AVI with one video stream; only the first picture is a key frame.
fn avi(w: u32, h: u32, bits: u16, compression: [u8; 4], palette: &[u8], frames: &[Vec<u8>]) -> Vec<u8> {
    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(body);
        if body.len() % 2 == 1 {
            v.push(0);
        }
        v
    }
    fn list(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        chunk(b"LIST", &[kind.as_slice(), body].concat())
    }
    let words = |v: &[u32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let n = frames.len() as u32;
    let avih = words(&[40_000, 0, 0, 0x10, n, 0, 1, 0, w, h, 0, 0, 0, 0]);
    let strh = [b"vids".as_slice(), &compression, &words(&[0, 0, 0, 1, 25, 0, n, 0, u32::MAX, 0]), &[0u8; 8]].concat();
    let strf = [words(&[40, w, h]), [1u16.to_le_bytes(), bits.to_le_bytes()].concat(), compression.to_vec(), words(&[0; 5]), palette.to_vec()].concat();
    let hdrl = list(b"hdrl", &[chunk(b"avih", &avih), list(b"strl", &[chunk(b"strh", &strh), chunk(b"strf", &strf)].concat())].concat());
    let (mut movi, mut idx) = (Vec::new(), Vec::new());
    for (i, f) in frames.iter().enumerate() {
        idx.extend_from_slice(&[b"00dc".as_slice(), &words(&[if i == 0 { 0x10 } else { 0 }, 4 + movi.len() as u32, f.len() as u32])].concat());
        movi.extend_from_slice(&chunk(b"00dc", f));
    }
    chunk(b"RIFF", &[b"AVI ".to_vec(), hdrl, list(b"movi", &movi), chunk(b"idx1", &idx)].concat())
}

fn check(ff: &std::path::Path, name: &str, path: &std::path::Path, label: &str, w: u32, h: u32, frames: usize) {
    let src = cfa4_codecs::open_bytes(name, bytes(path)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let v = src.info().video.clone().unwrap();
    assert_eq!((v.width, v.height, v.codec.as_str()), (w, h, label), "{name}: stream info");
    let raw = ffmpeg_frames(ff, path, "rgba");
    let want: Vec<&[u8]> = raw.chunks((w * h * 4) as usize).collect();
    assert_eq!(want.len(), frames, "{name}: ffmpeg's picture count");
    for i in (0..frames).rev().chain(0..frames) {
        let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("{name}: frame {i}: {e}"));
        let cfa4_frame::PixelData::Rgba8(px) = &f.data else { panic!("{name}: RGBA8 frame") };
        assert!(px.as_slice() == want[i], "{name}: frame {i} differs from ffmpeg's");
    }
}

#[test]
fn video1_in_avi_and_matroska_matches_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let (w, h) = (176u32, 144u32);
    for name in ["msvideo1.avi", "msvideo1.mkv"] {
        let Some(path) = fixture(
            &ff,
            name,
            &["-f", "lavfi", "-i", "testsrc2=size=176x144:rate=25:duration=1", "-pix_fmt", "rgb555le", "-c:v", "msvideo1", "-g", "6", "-q:v", "2"],
        ) else {
            eprintln!("SKIPPED ({name}): this ffmpeg can't write it");
            continue;
        };
        check(&ff, name, &path, "Microsoft Video 1", w, h, 25);
    }
}

/// Run-length lines of random 8-bit pixels covering `w`, bottom row first.
fn rle8_picture(rng: &mut Rng, w: usize, h: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for y in 0..h {
        let mut x = 0;
        while x < w {
            let n = (1 + rng.below(12) as usize).min(w - x);
            if n >= 3 && rng.below(2) == 0 {
                out.extend_from_slice(&[0, n as u8]);
                out.extend((0..n).map(|_| rng.below(256) as u8));
                if n % 2 == 1 {
                    out.push(0);
                }
            } else {
                out.extend_from_slice(&[n as u8, rng.below(256) as u8]);
            }
            x += n;
        }
        out.extend_from_slice(&[0, u8::from(y + 1 == h)]);
    }
    out
}

#[test]
fn rle_in_avi_matches_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let (w, h) = (48u32, 20u32);
    let mut rng = Rng(11);
    let palette: Vec<u8> = (0..256).flat_map(|_| [rng.below(256) as u8, rng.below(256) as u8, rng.below(256) as u8, 0]).collect();
    let frames: Vec<Vec<u8>> = (0..8).map(|_| rle8_picture(&mut rng, w as usize, h as usize)).collect();
    let path = dir().join("msrle.avi");
    std::fs::write(&path, avi(w, h, 8, [1, 0, 0, 0], &palette, &frames)).unwrap();
    check(&ff, "msrle.avi", &path, "Microsoft RLE", w, h, frames.len());
}
