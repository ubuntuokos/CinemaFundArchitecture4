//! FLIC (`cfa4-flic`) through the media layer: files opened by content, frames asked for in a
//! shuffled order against a straight decode (restarts from checkpoints included), and other files
//! left to their own openers.

use std::sync::Arc;

use cfa4_flic::synth::{packets, stream};
use cfa4_flic::{Decoder, Header};
use cfa4_frame::PixelData;
use cfa4_media::{FrameRequest, SharedSource};

fn open(name: &str, data: Vec<u8>) -> SharedSource {
    let bytes: Arc<[u8]> = data.into();
    cfa4_codecs::openers().iter().find_map(|o| o(name, bytes.clone())).expect("recognised by content").unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn reference(file: &[u8]) -> Vec<Vec<u8>> {
    let h = Header::parse(file).expect("header");
    let mut d = Decoder::new(&h).expect("decoder");
    packets(file, &h)
        .iter()
        .map(|p| {
            d.decode(p).expect("decodes");
            d.rgba()
        })
        .collect()
}

#[test]
fn frames_in_any_order() {
    // enough frames for several checkpoints
    for (seed, w, h, frames, magic) in [(11u64, 96usize, 64usize, 150usize, 0xAF12u16), (12, 61, 33, 40, 0xAF11)] {
        let file = stream(seed, w, h, frames, magic);
        let want = reference(&file);
        assert_eq!(want.len(), frames);
        let src = open("a.flc", file);
        let v = src.info().video.clone().expect("video");
        assert_eq!((v.width, v.height), (w as u32, h as u32));
        assert_eq!(v.codec, cfa4_codecs::flic::NAME);
        let mut order: Vec<usize> = (0..frames).rev().collect();
        order.swap(0, frames / 2);
        for i in order.into_iter().chain(0..frames) {
            let f = src.video_frame(FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("frame {i}: {e}"));
            let PixelData::Rgba8(px) = &f.data else { panic!("RGBA expected") };
            assert!(**px == want[i], "seed {seed} frame {i}");
        }
    }
}

#[test]
fn other_files_are_not_flic() {
    assert!(!cfa4_codecs::flic::sniff(&[0; 4096]));
    // an AVS1 / AVS2 / AVS3 start code stream
    assert!(!cfa4_codecs::flic::sniff(&[0, 0, 1, 0xb0, 0x20, 0x42, 0x81, 0x68, 0x12, 0x02, 0, 0, 1, 0xb3, 0, 0, 0, 0]));
}
