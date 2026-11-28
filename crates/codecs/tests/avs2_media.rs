//! AVS2 (`cfa4-avs2`) through the media layer: raw streams opened by content, frames asked for in
//! a shuffled order, against the pictures the decoder gives when it runs through the stream
//! directly. The streams are encoded by xavs2 (`scripts/ci/install-avs2-oracle.sh`; the test
//! skips without it; `CFA4_AVS2_ORACLE_DIR` as in `crates/avs2/tests/davs2_oracle.rs`) into
//! `target/fixtures/avs2/`. AVS1 streams, which share AVS2's start codes and profile number,
//! must stay with the AVS1 decoder.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use cfa4_cavs::synth::{Params, clean_stream};
use cfa4_frame::PixelData;
use cfa4_media::{FrameRequest, SharedSource};

fn xavs2() -> PathBuf {
    match std::env::var_os("CFA4_AVS2_ORACLE_DIR") {
        Some(dir) => Path::new(&dir).join("xavs2"),
        None => PathBuf::from("xavs2"),
    }
}

/// The decoder's pictures in output order, each as all its samples.
fn reference(data: &[u8]) -> Vec<Vec<u16>> {
    let mut d = cfa4_avs2::Decoder::new();
    let mut out = Vec::new();
    for (i, p) in cfa4_avs2::split_packets(data).into_iter().enumerate() {
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
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/avs2");
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join(format!("{name}.src.yuv"));
    std::fs::write(&input, source(w, h, frames)).unwrap();
    let out = dir.join(format!("{name}.avs2"));
    let status = Command::new(xavs2())
        .arg("-p")
        .arg(format!("InputFile={}", input.display()))
        .arg(format!("--SourceWidth={w}"))
        .arg(format!("--SourceHeight={h}"))
        .arg(format!("--FramesToBeEncoded={frames}"))
        .args(["--fps=25", "--InputSampleBitDepth=8", "--thread_frames=1", "--thread_rows=1"])
        .arg(format!("--OutputFile={}", out.display()))
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    assert!(status.success(), "{name}: xavs2 failed");
    Some(std::fs::read(&out).unwrap())
}

/// Every frame, last to first with the middle one first, then in order.
fn check(name: &str, src: &SharedSource, want: &[Vec<u16>]) {
    let v = src.info().video.clone().unwrap();
    assert_eq!(v.codec, cfa4_codecs::avs2::NAME, "{name}");
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
    let cases: [(&str, usize, usize, usize, &[&str]); 3] = [
        ("media_ra", 176, 144, 25, &["--preset=3", "--IntraPeriodMax=8", "--IntraPeriodMin=8", "--OpenGOP=0"]),
        ("media_ld", 176, 144, 12, &["--preset=3", "--bframes=0", "--FFRAMEEnable=1", "--IntraPeriodMax=5", "--IntraPeriodMin=5"]),
        ("media_odd", 122, 70, 10, &["--preset=3", "--MaxSizeInBit=5"]),
    ];
    for (name, w, h, frames, args) in cases {
        let Some(data) = encode(name, w, h, frames, args) else {
            eprintln!("SKIPPED (AVS2 media): xavs2 is not installed (scripts/ci/install-avs2-oracle.sh)");
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
        assert!(!cfa4_codecs::avs2::sniff(&data), "AVS1 stream {i} sniffed as AVS2");
        let src = open("avs1", data);
        assert_eq!(src.info().video.clone().unwrap().codec, cfa4_codecs::cavs::NAME);
    }
    // FFmpeg's FATE samples (real AVS1 streams), when present (see crates/cavs/tests/fate_oracle.rs)
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/cavs/fate");
    for name in ["bunny.avs", "cavs.avs"] {
        if let Ok(data) = std::fs::read(dir.join(name)) {
            assert!(!cfa4_codecs::avs2::sniff(&data), "{name} sniffed as AVS2");
        }
    }
}
