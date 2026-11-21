//! AVS3 (`cfa4-avs3`) through the media layer: raw streams opened by content, frames asked for in
//! a shuffled order, against the pictures the decoder gives when it runs through the stream
//! directly. The streams are encoded by uavs3e (`scripts/ci/install-avs3-oracle.sh`; the test
//! skips without it; `CFA4_AVS3_ORACLE_DIR` as in `crates/avs3/tests/uavs3d_oracle.rs`) into
//! `target/fixtures/avs3/`. AVS1 streams, which share AVS3's start codes and profile number,
//! must stay with the AVS1 decoder.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use cfa4_cavs::synth::{Params, clean_stream};
use cfa4_frame::PixelData;
use cfa4_media::{FrameRequest, SharedSource};

fn uavs3enc() -> PathBuf {
    match std::env::var_os("CFA4_AVS3_ORACLE_DIR") {
        Some(dir) => Path::new(&dir).join("uavs3enc"),
        None => PathBuf::from("uavs3enc"),
    }
}

/// The decoder's pictures in output order, each as all its samples.
fn reference(data: &[u8]) -> Vec<Vec<u16>> {
    let mut d = cfa4_avs3::Decoder::new();
    let mut out = Vec::new();
    for (i, p) in cfa4_avs3::split_packets(data).into_iter().enumerate() {
        out.extend(d.decode(p, i as i64).expect("decodes").into_iter().map(|f| f.planes.concat()));
    }
    out.extend(d.flush().into_iter().map(|f| f.planes.concat()));
    out
}

fn samples(data: &PixelData) -> Vec<u16> {
    match data {
        PixelData::Yuv8 { planes, .. } => planes.iter().flat_map(|p| p.iter().map(|&v| u16::from(v))).collect(),
        PixelData::Yuv16 { planes, .. } => planes.iter().flat_map(|p| p.iter().copied()).collect(),
        _ => panic!("not a YUV frame"),
    }
}

fn open(name: &str, data: Vec<u8>) -> SharedSource {
    let bytes: Arc<[u8]> = data.into();
    cfa4_codecs::openers().iter().find_map(|o| o(name, bytes.clone())).expect("recognised by content").unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// A moving gradient with a sliding block, 8-bit 4:2:0.
fn source(w: usize, h: usize, frames: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for f in 0..frames {
        for y in 0..h {
            for x in 0..w {
                let block = x.abs_diff(f * 5 % w) < 16 && y.abs_diff(f * 3 % h) < 12;
                out.push(((x * 3 + y * 2 + f * 4) % 220 + if block { 30 } else { 0 }) as u8);
            }
        }
        out.extend(std::iter::repeat_n(128u8, w * h / 2));
    }
    out
}

fn encode(name: &str, w: usize, h: usize, frames: usize, args: &[&str]) -> Option<Vec<u8>> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/avs3");
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join(format!("{name}.src.yuv"));
    std::fs::write(&input, source(w, h, frames)).unwrap();
    let out = dir.join(format!("{name}.avs3"));
    let status = Command::new(uavs3enc())
        .current_dir(&dir)
        .arg("-i")
        .arg(&input)
        .args(["-w", &w.to_string(), "-h", &h.to_string(), "-f", &frames.to_string()])
        .args(["-d", "8", "--fps_num", "25", "--fps_den", "1", "-v", "0"])
        .arg("-o")
        .arg(&out)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    assert!(status.success(), "{name}: uavs3enc failed");
    Some(std::fs::read(&out).unwrap())
}

/// Every frame, last to first with the middle one first, then in order.
fn check(name: &str, src: &SharedSource, want: &[Vec<u16>]) {
    let v = src.info().video.clone().unwrap();
    assert_eq!(v.codec, cfa4_codecs::avs3::NAME, "{name}");
    assert_eq!(src.info().duration, v.frame_rate.tick_of(want.len() as i64), "{name}");
    let count = want.len();
    let mut order: Vec<usize> = (0..count).rev().collect();
    order.swap(0, count / 2);
    for i in order.into_iter().chain(0..count) {
        let f = src.video_frame(FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("{name} frame {i}: {e}"));
        let s = samples(&f.data);
        assert!(s == want[i], "{name} frame {i}: equals {:?}", want.iter().position(|w| *w == s));
    }
}

#[test]
fn encoded_streams_through_the_media_layer() {
    let cases: [(&str, usize, usize, usize, &[&str]); 3] =
        [("media_ra", 176, 144, 17, &["-q", "30"]), ("media_ld", 176, 144, 12, &["-q", "30", "-p", "0"]), ("media_odd", 122, 70, 10, &["-q", "30"])];
    for (name, w, h, frames, args) in cases {
        let Some(data) = encode(name, w, h, frames, args) else {
            eprintln!("SKIPPED (AVS3 media): uavs3enc is not installed (scripts/ci/install-avs3-oracle.sh)");
            return;
        };
        let want = reference(&data);
        assert_eq!(want.len(), frames, "{name}: pictures");
        let src = open(name, data);
        let v = src.info().video.clone().unwrap();
        assert_eq!((v.width, v.height), (w as u32, h as u32), "{name}");
        check(name, &src, &want);
    }
}

#[test]
fn avs1_streams_stay_with_the_avs1_decoder() {
    for (i, p) in
        [Params { width: 96, height: 64, low_delay: false, seed: 21, pictures: 6 }, Params { width: 50, height: 34, low_delay: true, seed: 22, pictures: 4 }]
            .into_iter()
            .enumerate()
    {
        let (data, _) = clean_stream(p).expect("a stream");
        assert!(!cfa4_codecs::avs3::sniff(&data), "AVS1 stream {i} sniffed as AVS3");
        let src = open("avs1", data);
        assert_eq!(src.info().video.clone().unwrap().codec, cfa4_codecs::cavs::NAME);
    }
    // FFmpeg's FATE samples (real AVS1 streams), when present (see crates/cavs/tests/fate_oracle.rs)
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/cavs/fate");
    for name in ["bunny.avs", "cavs.avs"] {
        if let Ok(data) = std::fs::read(dir.join(name)) {
            assert!(!cfa4_codecs::avs3::sniff(&data), "{name} sniffed as AVS3");
        }
    }
}
