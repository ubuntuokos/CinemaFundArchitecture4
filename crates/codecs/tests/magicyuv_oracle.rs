//! MagicYUV (`cfa4-magicyuv`) through the containers against ffmpeg: AVI and Matroska files that
//! ffmpeg writes (4:2:0 with slices and median prediction, and RGBA) decode to ffmpeg's pictures, frame by frame, also out of order.

mod common;

use common::*;

#[test]
fn magicyuv_in_avi_and_matroska_matches_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let (w, h) = (176u32, 144u32);
    let src = "testsrc2=size=176x144:rate=25:duration=0.4";
    for name in ["magicyuv_median.avi", "magicyuv_median.mkv"] {
        let Some(path) = fixture(&ff, name, &["-f", "lavfi", "-i", src, "-pix_fmt", "yuv420p", "-c:v", "magicyuv", "-pred", "3", "-slices", "3"]) else {
            eprintln!("SKIPPED ({name}): this ffmpeg can't write it");
            continue;
        };
        let src = cfa4_codecs::open_bytes(name, bytes(&path)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let v = src.info().video.clone().unwrap();
        assert_eq!((v.width, v.height), (w, h), "{name}: reported size");
        let (cw, ch) = ((w / 2) as usize, (h / 2) as usize);
        let raw = ffmpeg_frames(&ff, &path, "yuv420p");
        let frame_bytes = (w * h) as usize + 2 * cw * ch;
        let want: Vec<&[u8]> = raw.chunks(frame_bytes).collect();
        assert_eq!(want.len(), 10, "{name}");
        for i in (0..want.len()).rev().chain(0..want.len()) {
            let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("{name}: frame {i}: {e}"));
            assert!(planes(&f) == raw_planes(want[i], w as usize, h as usize, cw, ch, 1), "{name}: frame {i} differs from ffmpeg's");
        }
    }
    for name in ["magicyuv_rgba.avi", "magicyuv_rgba.mkv"] {
        let Some(path) = fixture(&ff, name, &["-f", "lavfi", "-i", src, "-pix_fmt", "gbrap", "-c:v", "magicyuv", "-pred", "2"]) else {
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
