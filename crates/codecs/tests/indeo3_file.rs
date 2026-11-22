//! Indeo 3 through the media source on the FATE sample `iv32/OPENINGH.avi` (local only, in
//! `target/fixtures/fate/`, never committed); skipped without it. The decoder itself is checked
//! against ffmpeg in `crates/indeo3/tests/oracle.rs`.

#[test]
fn indeo3_avi_reads_through_the_media_source() {
    let path = cfa4_testkit::fixtures::fixtures_root().join("fate").join("iv32").join("OPENINGH.avi");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("no FATE sample, skipping");
        return;
    };
    let src = cfa4_codecs::open_bytes("OPENINGH.avi", bytes.into()).unwrap();
    let v = src.info().video.clone().unwrap();
    assert_eq!(v.codec.as_str(), "Indeo 3");
    let mut seen = Vec::new();
    for i in [40usize, 55, 45, 60, 41] {
        let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("frame {i}: {e}"));
        let cfa4_frame::PixelData::Yuv8 { planes, chroma, .. } = &f.data else { panic!("YUV 8-bit frame") };
        let (w, h) = (v.width as usize, v.height as usize);
        assert_eq!((*chroma, planes[0].len(), planes[1].len()), (cfa4_frame::Chroma::C420, w * h, w.div_ceil(2) * h.div_ceil(2)));
        seen.push(planes[0].clone());
    }
    // null frames repeat a picture, but a moving clip is not one picture
    assert!(seen.iter().any(|p| p != &seen[0]), "every frame is the same");
}
