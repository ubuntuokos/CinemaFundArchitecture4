//! Huffyuv MT (`HYMT`) in AVI through the media source: sliced packets built from ffmpeg-encoded
//! Huffyuv slices (see the huffyuv crate's `hymt_oracle`), compared with ffmpeg's `hymt` decode.

#[path = "../../huffyuv/tests/support/hymt.rs"]
mod hymt;

#[test]
fn hymt_avi_opens_and_decodes_like_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let _ = ff;
    let (w, slice_h, nb) = (64u32, 16u32, 3usize);
    let height = slice_h * nb as u32;
    let parts: Vec<_> = (0..nb).map(|k| hymt::slice_packets(&format!("media_slice{k}"), "left", w, slice_h, 11 * k as u32 + 5, 2)).collect();
    let extra = parts[0].0.clone();
    let packets: Vec<Vec<u8>> = (0..2).map(|f| hymt::hymt_packet(&parts.iter().map(|p| p.1[f].as_slice()).collect::<Vec<_>>(), slice_h)).collect();
    let file = hymt::dir().join("media_hymt.avi");
    std::fs::write(&file, hymt::avi(w, height, b"HYMT", &extra, &packets)).expect("write");
    let raw = hymt::dir().join("media_hymt.yuv");
    hymt::ffmpeg(&["-i", &file.to_string_lossy(), "-f", "rawvideo", "-pix_fmt", "yuv422p", &raw.to_string_lossy()]);
    let want = std::fs::read(&raw).expect("raw");
    let src = cfa4_codecs::open_bytes("media_hymt.avi", std::fs::read(&file).expect("read").into()).expect("open");
    let v = src.info().video.clone().expect("video");
    assert_eq!((v.width, v.height, v.codec.as_str()), (w, height, "Huffyuv MT"));
    let size = (w * height * 2) as usize;
    for i in [1usize, 0] {
        let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).expect("frame");
        let cfa4_frame::PixelData::Yuv8 { planes, .. } = &f.data else { panic!("YUV 8-bit frame") };
        let got: Vec<u8> = planes.iter().flat_map(|p| p.iter().copied()).collect();
        assert!(got == want[i * size..(i + 1) * size], "frame {i} differs from ffmpeg's");
    }
}
