//! FFV1 (`cfa4-ffv1`, RFC 9043) through the containers against ffmpeg: AVI and Matroska files
//! that ffmpeg writes (version 3 with its configuration record, and version 1 with non-key frames)
//! decode to ffmpeg's pictures, frame by frame, also out of order.

mod common;

use common::*;

#[test]
fn ffv1_in_avi_and_matroska_matches_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let (w, h) = (176u32, 144u32);
    let cases: [(&str, &[&str]); 4] = [
        ("ffv1_v3.avi", &["-level", "3", "-slices", "4", "-slicecrc", "1"]),
        ("ffv1_v3.mkv", &["-level", "3", "-coder", "1", "-context", "1"]),
        ("ffv1_v1_gop.avi", &["-level", "1", "-g", "5"]),
        ("ffv1_v1_gop.mkv", &["-level", "1", "-g", "5", "-coder", "1"]),
    ];
    for (name, opts) in cases {
        let mut args = vec!["-f", "lavfi", "-i", "testsrc2=size=176x144:rate=25:duration=0.48", "-pix_fmt", "yuv420p", "-c:v", "ffv1"];
        args.extend_from_slice(opts);
        let Some(path) = fixture(&ff, name, &args) else {
            eprintln!("SKIPPED ({name}): this ffmpeg can't write it");
            continue;
        };
        let src = cfa4_codecs::open_bytes(name, bytes(&path)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let v = src.info().video.clone().unwrap();
        assert_eq!((v.width, v.height), (w, h), "{name}: reported size");
        let (cw, ch) = (w.div_ceil(2) as usize, h.div_ceil(2) as usize);
        let raw = ffmpeg_frames(&ff, &path, "yuv420p");
        let frame_bytes = (w * h) as usize + 2 * cw * ch;
        let want: Vec<&[u8]> = raw.chunks(frame_bytes).collect();
        assert_eq!(want.len(), 12, "{name}");
        // backwards first (every frame from its key frame), then forwards
        for i in (0..want.len()).rev().chain(0..want.len()) {
            let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("{name}: frame {i}: {e}"));
            assert_eq!((f.width, f.height), (w, h), "{name}: frame {i} size");
            assert!(planes(&f) == raw_planes(want[i], w as usize, h as usize, cw, ch, 1), "{name}: frame {i} differs from ffmpeg's");
        }
    }
}
