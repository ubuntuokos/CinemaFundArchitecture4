//! VC-2 (`cfa4-dirac`) through the containers against ffmpeg: QuickTime, Matroska and AVI files
//! that ffmpeg's `vc2` encoder writes decode to ffmpeg's pictures, frame by frame, also in
//! reverse; the frame size, codec label and colour are reported from the stream.

mod common;

use common::*;

#[test]
fn vc2_in_quicktime_matroska_and_avi_matches_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let cases: [(&str, &str, u32, u32, &[&str]); 4] = [
        ("vc2_420.mov", "yuv420p", 352, 288, &[]),
        ("vc2_422p10.mkv", "yuv422p10le", 320, 180, &["-wavelet_type", "5_3"]),
        ("vc2_444.avi", "yuv444p", 176, 144, &["-wavelet_type", "haar", "-slice_width", "64"]),
        ("vc2_1080p.mov", "yuv422p10le", 1920, 1080, &["-b:v", "200M"]),
    ];
    for (name, pix_fmt, w, h, opts) in cases {
        let src = format!("testsrc2=size={w}x{h}:rate=25:duration=0.24");
        let mut args = vec!["-f", "lavfi", "-i", &src, "-pix_fmt", pix_fmt, "-c:v", "vc2"];
        args.extend_from_slice(opts);
        let Some(path) = fixture(&ff, name, &args) else {
            eprintln!("SKIPPED ({name}): this ffmpeg can't write it");
            continue;
        };
        let src = cfa4_codecs::open_bytes(name, bytes(&path)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let v = src.info().video.clone().unwrap();
        assert_eq!((v.width, v.height), (w, h), "{name}: reported size");
        assert_eq!(v.codec, "VC-2 / Dirac", "{name}: codec label");
        let (cw, ch, bps) = match pix_fmt {
            "yuv420p" => (w / 2, h / 2, 1),
            "yuv444p" => (w, h, 1),
            _ => (w / 2, h, 2),
        };
        let raw = ffmpeg_frames(&ff, &path, pix_fmt);
        let frame_bytes = ((w * h + 2 * cw * ch) * bps) as usize;
        let want: Vec<&[u8]> = raw.chunks(frame_bytes).collect();
        assert_eq!(want.len(), 6, "{name}");
        for i in (0..want.len()).rev().chain(0..want.len()) {
            let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("{name}: frame {i}: {e}"));
            assert_eq!((f.width, f.height), (w, h), "{name}: frame {i} size");
            assert!(
                planes(&f) == raw_planes(want[i], w as usize, h as usize, cw as usize, ch as usize, bps as usize),
                "{name}: frame {i} differs from ffmpeg's"
            );
            // ffmpeg writes 8-bit and 10-bit video range, BT.709 for HD sizes
            assert_eq!(f.color.range, cfa4_color::Range::Limited, "{name}: range");
        }
    }
}
