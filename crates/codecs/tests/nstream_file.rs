//! A Nintendo stream (the FFmpeg FATE samples, when they are in `target/fixtures/nstream/fate`)
//! read through the media source: the sound equals what `cfa4_nstream::decode` gives, from the
//! middle too.

use std::sync::Arc;

fn check(file: &str, container: &str, channels: u32) {
    let path = cfa4_testkit::fixtures::fixtures_dir("nstream").join("fate").join(file);
    let Ok(data) = std::fs::read(&path) else {
        eprintln!("skipping: FATE sample {file} is not in target/fixtures/nstream/fate");
        return;
    };
    let s = cfa4_nstream::parse(&data).unwrap();
    let d = cfa4_nstream::decode(&data, &s);
    let src = cfa4_codecs::open_bytes(file, Arc::from(data.clone())).unwrap();
    let info = src.info();
    assert_eq!(info.container, container);
    let a = info.audio().unwrap();
    assert_eq!((a.sample_rate, a.channels), (s.sample_rate, channels));
    assert!(info.video.is_none());
    let got = src.audio(1_500, 2_000, s.sample_rate).unwrap();
    for c in 0..channels as usize {
        let want: Vec<f32> = d.channels[c][1_500..3_500].iter().map(|&v| f32::from(v) / 32768.0).collect();
        assert_eq!(got.channels[c], want, "{file}: channel {c}");
    }
}

#[test]
fn nintendo_streams_read_through_the_media_source() {
    check("lozswd_partial.brstm", "Nintendo BRSTM", 6);
    check("spl-forest-day.bfstm", "Nintendo BFSTM", 1);
    check("loz-mm-mikau.bcstm", "Nintendo BCSTM", 2);
}
