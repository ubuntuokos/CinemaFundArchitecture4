//! RealMedia with RealVideo 1.0 to 4.0 (`cfa4-realmedia` + `cfa4-realvideo`) through the media
//! source against ffmpeg: streams from ffmpeg's `rv10` / `rv20` encoders, and, when present,
//! ffmpeg's FATE sample `real/G2_with_SVT_320_240.rm` (RealVideo 2.0 with B-frames, from
//! `https://fate-suite.ffmpeg.org/real/`) in `target/fixtures/real`, and the FATE samples `rv30.rm`
//! (RealVideo 3.0) and `spygames-2MB.rmvb` (RealVideo 4.0) there. Frames are requested last
//! to first, so every one of them is reached by seeking. ffmpeg runs with its simple IDCT, the one
//! the decoder implements.

mod common;

use std::path::Path;

use cfa4_frame::PixelData;
use common::*;

fn yuv(f: &cfa4_frame::VideoFrame) -> Vec<u8> {
    match &f.data {
        PixelData::Yuv8 { planes, .. } => planes.iter().flat_map(|p| p.iter().copied()).collect(),
        _ => panic!("expected 8-bit YUV"),
    }
}

fn compare(ff: &Path, name: &str, path: &Path, codec: &str) -> usize {
    let src = cfa4_codecs::open_bytes(name, bytes(path)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let v = src.info().video.clone().unwrap_or_else(|| panic!("{name}: no video"));
    assert_eq!(v.codec, codec, "{name}: codec label");
    assert_eq!(src.info().container, "RealMedia");
    let raw = ffmpeg_out(
        ff,
        &[
            "-idct",
            "simple",
            "-flags",
            "+bitexact",
            "-i",
            path.to_str().unwrap(),
            "-map",
            "0:v:0",
            "-fps_mode",
            "passthrough",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv420p",
            "-",
        ],
    );
    let (w, h) = (v.width as usize, v.height as usize);
    let size = w * h + 2 * w.div_ceil(2) * h.div_ceil(2);
    let want: Vec<&[u8]> = raw.chunks(size).collect();
    assert!(want.len() > 2, "{name}: ffmpeg decoded {} frames", want.len());
    let frames = v.frame_rate.frame_at(src.info().duration);
    // ffmpeg leaves out the last reference picture of a stream with B-frames
    assert!(frames == want.len() as i64 || frames == want.len() as i64 + 1, "{name}: {frames} frames, ffmpeg {}", want.len());
    for i in (0..want.len()).rev() {
        let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("{name}: frame {i}: {e}"));
        let got = yuv(&f);
        assert_eq!(got.len(), want[i].len(), "{name}: frame {i} size");
        if let Some(at) = got.iter().zip(want[i]).position(|(a, b)| a != b) {
            panic!("{name}: frame {i} differs from ffmpeg's at byte {at}: ours {}, ffmpeg {}", got[at], want[i][at]);
        }
    }
    want.len()
}

#[test]
fn ffmpeg_encoded_rv10_and_rv20_match() {
    let Some(ff) = cfa4_testkit::ffmpeg_or_skip("realmedia oracle") else { return };
    for (codec, label, size) in [("rv10", "RealVideo 1.0", "176x144"), ("rv20", "RealVideo 2.0", "320x240")] {
        let name = format!("realmedia-{codec}.rm");
        let src = format!("testsrc2=size={size}:rate=25:duration=2,noise=alls=8:allf=t");
        let Some(p) = fixture(&ff, &name, &["-f", "lavfi", "-i", &src, "-c:v", codec, "-g", "12", "-q:v", "4", "-f", "rm"]) else {
            panic!("{name}: ffmpeg could not make the fixture");
        };
        assert_eq!(compare(&ff, &name, &p, label), 50);
    }
}

#[test]
fn fate_rv20_with_b_frames_matches() {
    let Some(ff) = cfa4_testkit::ffmpeg_or_skip("realmedia FATE") else { return };
    let p = cfa4_testkit::fixtures_dir("real").join("G2_with_SVT_320_240.rm");
    if !p.exists() {
        eprintln!("skipping: {} not present", p.display());
        return;
    }
    compare(&ff, "G2_with_SVT_320_240.rm", &p, "RealVideo 2.0");
}

#[test]
fn fate_rv30_and_rv40_match() {
    let Some(ff) = cfa4_testkit::ffmpeg_or_skip("realmedia FATE") else { return };
    for (file, label) in [("rv30.rm", "RealVideo 3.0"), ("spygames-2MB.rmvb", "RealVideo 4.0")] {
        let p = cfa4_testkit::fixtures_dir("real").join(file);
        if !p.exists() {
            eprintln!("skipping: {} not present", p.display());
            continue;
        }
        compare(&ff, file, &p, label);
    }
}
