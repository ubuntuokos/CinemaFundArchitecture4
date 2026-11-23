//! SVQ1 through the media source: the FATE sample `svq1/marymary-shackles.mov` (local only, in
//! `target/fixtures/fate/`, never committed) remuxed to AVI with ffmpeg; skipped without either.
//! The decoder itself is checked against ffmpeg in `crates/svq1/tests/oracle.rs`.

#[test]
fn svq1_avi_reads_through_the_media_source() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("svq1 file") else { return };
    let root = cfa4_testkit::fixtures::fixtures_root();
    let src = root.join("fate/svq1/marymary-shackles.mov");
    if !src.exists() {
        eprintln!("no FATE sample svq1/marymary-shackles.mov, skipping");
        return;
    }
    let dir = root.join("svq1-file");
    std::fs::create_dir_all(&dir).unwrap();
    let avi = dir.join("marymary.avi");
    let ok = std::process::Command::new(ff)
        .args(["-v", "error", "-y", "-i"])
        .arg(&src)
        .args(["-an", "-c:v", "copy", "-tag:v", "SVQ1", "-r", "15"])
        .arg(&avi)
        .status()
        .unwrap()
        .success();
    assert!(ok, "remux");
    let src = cfa4_codecs::open_bytes("marymary.avi", std::fs::read(&avi).unwrap().into()).unwrap();
    let v = src.info().video.clone().unwrap();
    assert_eq!(v.codec.as_str(), "Sorenson Video 1");
    let mut seen = Vec::new();
    for i in [0usize, 20, 7, 100, 3, 464] {
        let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("frame {i}: {e}"));
        let cfa4_frame::PixelData::Yuv8 { planes, chroma, .. } = &f.data else { panic!("YUV 8-bit frame") };
        let (w, h) = (v.width as usize, v.height as usize);
        assert_eq!((*chroma, planes[0].len(), planes[1].len()), (cfa4_frame::Chroma::C420, w * h, w.div_ceil(2) * h.div_ceil(2)));
        seen.push(planes[0].clone());
    }
    assert!(seen.iter().any(|p| p != &seen[0]), "every frame is the same");
}
