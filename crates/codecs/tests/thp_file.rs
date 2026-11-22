//! A Nintendo THP file read through the media source: every picture equals what
//! `cfa4_thp::jpeg::decode` gives, whichever order the pictures are asked for in, and the sound
//! reads from the middle.

#[path = "../../thp/tests/support/mod.rs"]
mod support;

use cfa4_frame::PixelData;
use cfa4_media::FrameRequest;
use cfa4_time::Tick;
use std::sync::Arc;
use support::*;

#[test]
fn thp_file_reads_in_any_order() {
    let mut rng = Rng(0x5eed_f00d);
    let (w, h, frames) = (48usize, 40usize, 30usize);
    let pics: Vec<Vec<u8>> = (0..frames).map(|_| picture(w, h, Some((2, 2)), 2, &mut rng)).collect();
    let audio: Vec<Vec<u8>> = (0..frames).map(|_| audio_packet(2, 1000, &mut rng)).collect();
    let data = thp_file(&pics, Some((2, audio)), w, h);
    let m = cfa4_thp::parse(&data).unwrap();
    assert_eq!((m.video.len(), m.audio_packets.len(), m.width, m.height), (frames, frames, 48, 40));
    let want: Vec<cfa4_thp::Picture> = m.video.iter().map(|s| cfa4_thp::jpeg::decode(s.of(&data)).unwrap()).collect();

    let src = cfa4_codecs::open_bytes("clip.thp", Arc::from(data.clone())).unwrap();
    let info = src.info();
    assert_eq!(info.container, "Nintendo THP");
    let v = info.video.as_ref().unwrap();
    assert_eq!((v.width, v.height), (48, 40));
    let a = info.audio().unwrap();
    assert_eq!((a.sample_rate, a.channels, a.codec.as_str()), (32000, 2, "THP ADPCM"));

    let rate = v.frame_rate;
    let at = |k: usize| FrameRequest::full(rate.tick_of(k as i64) + Tick::from_rational(1, 1, 1000));
    for k in [0, 1, 2, 29, 5, 17, 16, 0, 28, 3, 3, 12] {
        let f = src.video_frame(at(k)).unwrap();
        let PixelData::Yuv8 { planes, .. } = &f.data else { panic!("not 8-bit YUV") };
        for p in 0..3 {
            assert!(*planes[p] == want[k].planes[p], "frame {k}, plane {p}");
        }
    }

    // the sound: packet k holds 1000 samples per channel
    let mut dec = cfa4_thp::AudioDecoder::new();
    let mut pcm = Vec::new();
    for s in &m.audio_packets {
        pcm.extend(dec.decode(s.of(&data), 2).unwrap());
    }
    let got = src.audio(1_500, 2_000, 32000).unwrap();
    let left: Vec<f32> = pcm.iter().skip(2 * 1_500).step_by(2).take(2_000).map(|&v| f32::from(v) / 32768.0).collect();
    assert_eq!(got.channels[0], left);
}
