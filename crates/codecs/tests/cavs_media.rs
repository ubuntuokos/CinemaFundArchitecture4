//! AVS1 (`cfa4-cavs`) through the media layer: raw streams opened by content, frames asked for
//! in a shuffled order, against the pictures the decoder gives when it runs through the stream
//! directly. Synthetic streams (the crate's generator) always; FFmpeg's FATE samples when they
//! are in `target/fixtures/cavs/fate/` (see `crates/cavs/tests/fate_oracle.rs`).

use std::path::Path;
use std::sync::Arc;

use cfa4_cavs::synth::{Params, clean_stream};
use cfa4_frame::PixelData;
use cfa4_media::{FrameRequest, SharedSource};

/// The decoder's pictures in output order, each as all its samples.
fn reference(data: &[u8]) -> Vec<Vec<u8>> {
    let mut d = cfa4_cavs::Decoder::new();
    let mut out = Vec::new();
    for p in cfa4_cavs::split_packets(data) {
        if let Some(f) = d.decode(p).expect("decodes") {
            out.push(f.planes.concat());
        }
    }
    out.extend(d.flush().map(|f| f.planes.concat()));
    out
}

fn samples(data: &PixelData) -> Vec<u8> {
    match data {
        PixelData::Yuv8 { planes, .. } => planes.iter().flat_map(|p| p.iter().copied()).collect(),
        _ => panic!("not an 8-bit YUV frame"),
    }
}

fn open(name: &str, data: Vec<u8>) -> SharedSource {
    let bytes: Arc<[u8]> = data.into();
    cfa4_codecs::openers().iter().find_map(|o| o(name, bytes.clone())).expect("recognised by content").unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// Every frame, last to first with the middle one first, then in order. `slack` frames at the
/// end are left out (a cut sample's last picture does not decode, which the index cannot know).
fn check(name: &str, src: &SharedSource, want: &[Vec<u8>], slack: usize) {
    let v = src.info().video.clone().unwrap();
    assert_eq!(v.codec, cfa4_codecs::cavs::NAME);
    if slack == 0 {
        assert_eq!(src.info().duration, v.frame_rate.tick_of(want.len() as i64), "{name}");
    }
    let count = want.len().saturating_sub(slack);
    let mut order: Vec<usize> = (0..count).rev().collect();
    order.swap(0, count / 2);
    for i in order.into_iter().chain(0..count) {
        let f = src.video_frame(FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("{name} frame {i}: {e}"));
        let s = samples(&f.data);
        assert!(s == want[i], "{name} frame {i}: equals {:?}", want.iter().position(|w| *w == s));
    }
}

#[test]
fn synthetic_streams_through_the_media_layer() {
    let cases = [
        Params { width: 96, height: 64, low_delay: false, seed: 11, pictures: 16 },
        Params { width: 50, height: 34, low_delay: true, seed: 12, pictures: 9 },
        Params { width: 64, height: 48, low_delay: false, seed: 13, pictures: 23 },
    ];
    for p in cases {
        let (data, seed) = clean_stream(p).expect("a stream");
        let name = format!("synth_{seed}.avs");
        let want = reference(&data);
        let pictures = cfa4_cavs::split_packets(&data).into_iter().filter(|p| cfa4_cavs::picture_type(p).is_some()).count();
        assert!(pictures >= p.pictures);
        assert_eq!(want.len(), pictures, "{name}: every picture decodes");
        let src = open(&name, data);
        let v = src.info().video.clone().unwrap();
        assert_eq!((v.width, v.height), (p.width, p.height));
        check(&name, &src, &want, 0);
    }
}

#[test]
fn damaged_streams_do_not_panic() {
    let (data, _) = clean_stream(Params { width: 48, height: 32, low_delay: false, seed: 21, pictures: 10 }).expect("a stream");
    for cut in [data.len() / 3, data.len() / 2, data.len() - 3] {
        let bytes: Arc<[u8]> = data[..cut].to_vec().into();
        if let Some(Ok(src)) = cfa4_codecs::openers().iter().find_map(|o| o("cut.avs", bytes.clone())) {
            let v = src.info().video.clone().unwrap();
            for i in 0..12 {
                let _ = src.video_frame(FrameRequest::full(v.frame_rate.tick_of(i)));
            }
        }
    }
    let mut flipped = data.clone();
    for k in (40..flipped.len()).step_by(7) {
        flipped[k] ^= 0x5a;
    }
    let bytes: Arc<[u8]> = flipped.into();
    if let Some(Ok(src)) = cfa4_codecs::openers().iter().find_map(|o| o("flipped.avs", bytes.clone())) {
        let v = src.info().video.clone().unwrap();
        for i in (0..12).rev() {
            let _ = src.video_frame(FrameRequest::full(v.frame_rate.tick_of(i)));
        }
    }
}

#[test]
fn fate_samples_through_the_media_layer() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/cavs/fate");
    for name in ["bunny.avs", "cavs.avs"] {
        let Ok(data) = std::fs::read(dir.join(name)) else {
            eprintln!("SKIPPED (AVS1 FATE media): {name} is missing");
            continue;
        };
        let want = reference(&data);
        let src = open(name, data);
        check(name, &src, &want, 2);
    }
}
