//! A MobiClip file (the FFmpeg FATE sample, when it is in `target/fixtures/mobiclip/fate`) read
//! through the media source: every picture equals what `cfa4_mobiclip::Decoder` gives in file
//! order, whichever order the pictures are asked for in, and the sound reads from the middle.

use cfa4_frame::PixelData;
use cfa4_media::FrameRequest;
use cfa4_time::Tick;
use std::sync::Arc;

#[test]
fn mobiclip_file_reads_in_any_order() {
    let path = cfa4_testkit::fixtures::fixtures_dir("mobiclip").join("fate/mobiclip-reference-rect-partitions.moflex");
    let Ok(data) = std::fs::read(&path) else {
        eprintln!("skipping: FATE sample is not in target/fixtures/mobiclip/fate");
        return;
    };
    let m = cfa4_mobiclip::parse(&data).unwrap();
    let mut dec = cfa4_mobiclip::Decoder::new(m.width, m.height).unwrap();
    let want: Vec<_> = m.video.iter().map(|p| dec.decode(&p.data).unwrap()).collect();

    let src = cfa4_codecs::open_bytes("clip.moflex", Arc::from(data.clone())).unwrap();
    let info = src.info();
    assert_eq!(info.container, "Nintendo MobiClip MOFLEX");
    let v = info.video.as_ref().unwrap();
    assert_eq!((v.width, v.height), (320, 240));
    let a = info.audio().unwrap();
    assert_eq!((a.sample_rate, a.channels), (32000, 2));

    let rate = v.frame_rate;
    let at = |k: usize| FrameRequest::full(rate.tick_of(k as i64) + Tick::from_rational(1, 1, 1000));
    for k in [1, 0, 1, 1, 0] {
        let f = src.video_frame(at(k)).unwrap();
        let PixelData::Yuv8 { planes, .. } = &f.data else { panic!("not 8-bit YUV") };
        for p in 0..3 {
            assert!(*planes[p] == want[k].planes[p], "frame {k}, plane {p}");
        }
    }

    let mut fast = cfa4_mobiclip::FastAudio::new(2);
    let mut left: Vec<f32> = Vec::new();
    for p in &m.audio.as_ref().unwrap().packets {
        left.extend(fast.decode(p)[0].iter());
    }
    let got = src.audio(100, 500, 32000).unwrap();
    assert_eq!(got.channels[0], left[100..600]);
}
