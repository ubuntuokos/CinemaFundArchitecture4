//! Indeo 4 and 5 through the media source on the FATE samples `iv41/indeo41-partial.avi` and
//! `iv50/Educ_Movie_DeadlyForce.avi` (local only, in `target/fixtures/fate/`, never committed);
//! skipped without them. The decoders themselves are checked against ffmpeg in
//! `crates/indeo45/tests/oracle.rs`.

fn check(name: &str, label: &str) {
    let path = cfa4_testkit::fixtures::fixtures_root().join("fate").join(name);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("no FATE sample {name}, skipping");
        return;
    };
    let src = cfa4_codecs::open_bytes(name, bytes.into()).unwrap();
    let v = src.info().video.clone().unwrap();
    assert_eq!(v.codec.as_str(), label);
    let mut seen = Vec::new();
    for i in [0usize, 20, 7, 40, 3] {
        let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("frame {i}: {e}"));
        let cfa4_frame::PixelData::Yuv8 { planes, chroma, .. } = &f.data else { panic!("YUV 8-bit frame") };
        let (w, h) = (v.width as usize, v.height as usize);
        assert_eq!((*chroma, planes[0].len(), planes[1].len()), (cfa4_frame::Chroma::C420, w * h, w.div_ceil(2) * h.div_ceil(2)));
        seen.push(planes[0].clone());
    }
    assert!(seen.iter().any(|p| p != &seen[0]), "every frame is the same");
}

#[test]
fn indeo4_avi_reads_through_the_media_source() {
    check("iv41/indeo41-partial.avi", "Indeo 4");
}

#[test]
fn indeo5_avi_reads_through_the_media_source() {
    check("iv50/Educ_Movie_DeadlyForce.avi", "Indeo 5");
}
