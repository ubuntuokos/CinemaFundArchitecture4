//! Canopus HQ / HQA / HQX (`cfa4-canopus`) through the containers against ffmpeg: a hand-made
//! HQA frame that ffmpeg muxes into AVI (`CUVC`) and Matroska, and, when present, ffmpeg's FATE
//! samples (`canopus/hq.avi`, `hq25i.avi`, `hqa.avi`, `hqx422.avi`, `hqx422a.avi`, from
//! `https://fate-suite.ffmpeg.org/canopus/`) in `$CANOPUS_SAMPLES` or
//! `target/fixtures/canopus/fate`: every plane of every frame equal, alpha included. The decoder's
//! own bit-exactness tests (synthetic HQ, HQA and HQX frames) are in `crates/canopus`.

mod common;

use std::path::Path;

use cfa4_frame::PixelData;
use common::*;

/// Every plane of `f` as ffmpeg's native raw layout (12-bit HQX as ffmpeg's 16-bit LE).
fn native(f: &cfa4_frame::VideoFrame) -> (Vec<u8>, &'static str) {
    match &f.data {
        PixelData::Yuv8 { planes, alpha, .. } => {
            let mut v: Vec<u8> = planes.iter().flat_map(|p| p.iter().copied()).collect();
            if let Some(a) = alpha {
                v.extend(a.iter());
            }
            (v, if alpha.is_some() { "yuva422p" } else { "yuv422p" })
        }
        PixelData::Yuv16 { planes, alpha, chroma, .. } => {
            let mut v: Vec<u8> = Vec::new();
            for p in planes.iter().map(|p| p.as_slice()).chain(alpha.as_deref().map(|a| a.as_slice())) {
                v.extend(p.iter().flat_map(|&s| ((s << 4) | (s >> 8)).to_le_bytes()));
            }
            let fmt = match (chroma, alpha.is_some()) {
                (cfa4_frame::Chroma::C444, false) => "yuv444p16le",
                (cfa4_frame::Chroma::C444, true) => "yuva444p16le",
                (_, false) => "yuv422p16le",
                (_, true) => "yuva422p16le",
            };
            (v, fmt)
        }
        _ => panic!("expected a YUV frame"),
    }
}

/// Every frame of `path` through our media source against ffmpeg's decode.
fn compare(ff: &Path, name: &str, path: &Path, codec: &str) -> usize {
    let src = cfa4_codecs::open_bytes(name, bytes(path)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let v = src.info().video.clone().unwrap_or_else(|| panic!("{name}: no video"));
    assert_eq!(v.codec, codec, "{name}: codec label");
    let first = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(0))).unwrap_or_else(|e| panic!("{name}: {e}"));
    let (_, fmt) = native(&first);
    let raw = ffmpeg_frames(ff, path, fmt);
    let size = native(&first).0.len();
    let want: Vec<&[u8]> = raw.chunks(size).collect();
    assert!(!want.is_empty(), "{name}: ffmpeg decoded nothing");
    for i in (0..want.len()).rev() {
        let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("{name}: frame {i}: {e}"));
        let (got, _) = native(&f);
        assert_eq!(got.len(), want[i].len(), "{name}: frame {i} size");
        if let Some(at) = got.iter().zip(want[i]).position(|(a, b)| a != b) {
            panic!("{name}: frame {i} differs from ffmpeg's at byte {at}: ours {}, ffmpeg {}", got[at], want[i][at]);
        }
    }
    want.len()
}

/// A 32×16 HQA frame of two macroblocks without coded blocks (mid grey, transparent).
fn hqa_frame() -> Vec<u8> {
    let mut f = b"HQA1".to_vec();
    f.extend(32u16.to_be_bytes());
    f.extend(16u16.to_be_bytes());
    f.extend([0, 0, 0, 0]);
    for i in 0..9u32 {
        f.extend((48 + i).to_be_bytes());
    }
    // cbp 0 is `0100`
    f.extend([0x40; 8]);
    f
}

#[test]
fn hqa_in_avi_and_matroska_matches_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let frame = dir().join("canopus_hqa.bin");
    std::fs::write(&frame, hqa_frame()).expect("write frame");
    for name in ["canopus_hqa.avi", "canopus_hqa.mkv"] {
        let Some(path) = fixture(&ff, name, &["-f", "image2", "-c:v", "hq_hqa", "-framerate", "25", "-i", frame.to_str().unwrap(), "-c:v", "copy"]) else {
            cfa4_testkit::oracle::skip(name, "ffmpeg can't write it");
            continue;
        };
        assert_eq!(compare(&ff, name, &path, "Canopus HQ"), 1, "{name}");
    }
}

#[test]
fn fate_samples_match_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let dir = std::env::var_os("CANOPUS_SAMPLES").map(std::path::PathBuf::from).unwrap_or_else(|| cfa4_testkit::fixtures::fixtures_root().join("canopus/fate"));
    let mut tested = 0;
    for (file, codec) in
        [("hq.avi", "Canopus HQ"), ("hq25i.avi", "Canopus HQ"), ("hqa.avi", "Canopus HQ"), ("hqx422.avi", "Canopus HQX"), ("hqx422a.avi", "Canopus HQX")]
    {
        let path = dir.join(file);
        if !path.exists() {
            continue;
        }
        let n = compare(&ff, file, &path, codec);
        eprintln!("{file}: {n} frames bit-exact");
        tested += 1;
    }
    if tested == 0 {
        eprintln!("SKIP: no Canopus samples in {}", dir.display());
    }
}
