//! Huffyuv (`cfa4-huffyuv`) through the containers against ffmpeg: AVI and Matroska files that
//! ffmpeg writes (YUY2 and RGBA) decode to ffmpeg's pictures, frame by frame, also out of order.

mod common;

use common::*;

#[test]
fn huffyuv_in_avi_and_matroska_matches_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let (w, h) = (176u32, 144u32);
    let src = "testsrc2=size=176x144:rate=25:duration=0.4";
    for name in ["huffyuv_median.avi", "huffyuv_median.mkv"] {
        let Some(path) = fixture(&ff, name, &["-f", "lavfi", "-i", src, "-pix_fmt", "yuv422p", "-c:v", "huffyuv", "-pred", "median"]) else {
            eprintln!("SKIPPED ({name}): this ffmpeg can't write it");
            continue;
        };
        let src = cfa4_codecs::open_bytes(name, bytes(&path)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let v = src.info().video.clone().unwrap();
        assert_eq!((v.width, v.height), (w, h), "{name}: reported size");
        let cw = (w / 2) as usize;
        let raw = ffmpeg_frames(&ff, &path, "yuv422p");
        let frame_bytes = (w * h) as usize + 2 * cw * h as usize;
        let want: Vec<&[u8]> = raw.chunks(frame_bytes).collect();
        assert_eq!(want.len(), 10, "{name}");
        for i in (0..want.len()).rev().chain(0..want.len()) {
            let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("{name}: frame {i}: {e}"));
            assert!(planes(&f) == raw_planes(want[i], w as usize, h as usize, cw, h as usize, 1), "{name}: frame {i} differs from ffmpeg's");
        }
    }
    for name in ["huffyuv_rgba.avi", "huffyuv_rgba.mkv"] {
        let Some(path) = fixture(&ff, name, &["-f", "lavfi", "-i", src, "-pix_fmt", "bgra", "-c:v", "huffyuv", "-pred", "plane"]) else {
            eprintln!("SKIPPED ({name}): this ffmpeg can't write it");
            continue;
        };
        let src = cfa4_codecs::open_bytes(name, bytes(&path)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let v = src.info().video.clone().unwrap();
        let raw = ffmpeg_frames(&ff, &path, "rgba");
        let frame_bytes = (w * h * 4) as usize;
        let want: Vec<&[u8]> = raw.chunks(frame_bytes).collect();
        assert_eq!(want.len(), 10, "{name}");
        for (i, want) in want.iter().enumerate() {
            let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("{name}: frame {i}: {e}"));
            let cfa4_frame::PixelData::Rgba8(px) = &f.data else { panic!("{name}: RGBA8 frame") };
            assert!(px.as_slice() == *want, "{name}: frame {i} differs from ffmpeg's");
        }
    }
}
