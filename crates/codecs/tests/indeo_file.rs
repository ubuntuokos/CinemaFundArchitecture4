//! Indeo 2 through the media source on the FATE sample `rt21/VPAR0026.AVI` (local only, in
//! `target/fixtures/fate/`, never committed); skipped without it. The decoder itself is checked
//! against ffmpeg in `crates/indeo/tests/oracle.rs`.

#[test]
fn indeo2_avi_reads_through_the_media_source() {
    let path = cfa4_testkit::fixtures::fixtures_root().join("fate").join("rt21").join("VPAR0026.AVI");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("no FATE sample, skipping");
        return;
    };
    let src = cfa4_codecs::open_bytes("VPAR0026.AVI", bytes.into()).unwrap();
    let v = src.info().video.clone().unwrap();
    assert_eq!((v.width, v.height, v.codec.as_str()), (160, 120, "Indeo 2"));
    let mut prev = None;
    for i in [0usize, 7, 3, 20, 1] {
        let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("frame {i}: {e}"));
        let cfa4_frame::PixelData::Yuv8 { planes, chroma, .. } = &f.data else { panic!("YUV 8-bit frame") };
        assert_eq!((*chroma, planes[0].len(), planes[1].len()), (cfa4_frame::Chroma::C420, 160 * 120, 80 * 60));
        assert!(prev.as_ref() != Some(&planes[0]), "frame {i} equals the previous one");
        prev = Some(planes[0].clone());
    }
}
