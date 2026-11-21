//! Theora (`cfa4-theora`) in Matroska (`V_THEORA`) against ffmpeg's decode of the same file: every
//! frame equal, in order, backwards and after jumps. Needs an ffmpeg with the libtheora encoder
//! (the file is skipped otherwise).

mod common;

use common::*;

#[test]
fn theora_in_matroska_matches_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    // (name, size, pixel format, extra encoder options)
    let cases: [(&str, (u32, u32), &str, &[&str]); 6] = [
        ("theora_420.mkv", (176, 144), "yuv420p", &["-g", "5", "-b:v", "300k"]),
        ("theora_420_q.mkv", (176, 144), "yuv420p", &["-g", "100", "-q:v", "10"]),
        ("theora_420_lowq.mkv", (176, 144), "yuv420p", &["-g", "7", "-q:v", "2"]),
        ("theora_422.mkv", (160, 96), "yuv422p", &["-g", "4", "-q:v", "7"]),
        ("theora_444.mkv", (160, 96), "yuv444p", &["-g", "4", "-q:v", "7"]),
        ("theora_crop.mkv", (170, 130), "yuv420p", &["-g", "6", "-q:v", "6"]),
    ];
    for (name, (w, h), pix, opts) in cases {
        let src = format!("testsrc2=size={w}x{h}:rate=25:duration=0.8");
        let mut args = vec!["-f", "lavfi", "-i", &src, "-pix_fmt", pix, "-c:v", "libtheora"];
        args.extend_from_slice(opts);
        let Some(path) = fixture(&ff, name, &args) else {
            cfa4_testkit::oracle::skip(name, "ffmpeg can't write it");
            continue;
        };
        let src = cfa4_codecs::open_bytes(name, bytes(&path)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let v = src.info().video.clone().unwrap();
        assert_eq!((v.width, v.height), (w, h), "{name}: reported size");
        let (sx, sy) = match pix {
            "yuv420p" => (1, 1),
            "yuv422p" => (1, 0),
            _ => (0, 0),
        };
        let (cw, ch) = ((w as usize).div_ceil(1 << sx), (h as usize).div_ceil(1 << sy));
        let raw = ffmpeg_frames(&ff, &path, pix);
        let frame_bytes = (w * h) as usize + 2 * cw * ch;
        let want: Vec<&[u8]> = raw.chunks(frame_bytes).collect();
        assert_eq!(want.len(), 20, "{name}");
        for i in (0..want.len()).rev().chain(0..want.len()) {
            let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("{name}: frame {i}: {e}"));
            assert_eq!((f.width, f.height), (w, h), "{name}: frame {i} size");
            assert!(planes(&f) == raw_planes(want[i], w as usize, h as usize, cw, ch, 1), "{name}: frame {i} differs from ffmpeg's");
        }
    }
}
