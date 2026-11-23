//! A VQF file (the FFmpeg FATE sample, when it is in `target/fixtures/vqf/fate`) read through the
//! media source: the sound equals what `cfa4_twinvq::decode` gives, from the middle too.

use std::sync::Arc;

#[test]
fn vqf_reads_through_the_media_source() {
    let path = cfa4_testkit::fixtures::fixtures_dir("vqf").join("fate").join("achterba.vqf");
    let Ok(data) = std::fs::read(&path) else {
        eprintln!("skipping: FATE sample achterba.vqf is not in target/fixtures/vqf/fate");
        return;
    };
    let s = cfa4_twinvq::parse(&data).unwrap();
    let d = cfa4_twinvq::decode(&data, &s);
    let src = cfa4_codecs::open_bytes("achterba.vqf", Arc::from(data.clone())).unwrap();
    let info = src.info();
    assert_eq!(info.container, "NTT VQF");
    let a = info.audio().unwrap();
    assert_eq!((a.sample_rate, a.channels as usize), (s.sample_rate, s.channels));
    assert!(info.video.is_none());
    let got = src.audio(100_000, 2_000, s.sample_rate).unwrap();
    for c in 0..s.channels {
        assert_eq!(got.channels[c], d.channels[c][100_000..102_000], "channel {c}");
    }
}
