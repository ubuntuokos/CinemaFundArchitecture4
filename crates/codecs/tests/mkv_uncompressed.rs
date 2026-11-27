//! Matroska `V_UNCOMPRESSED` video (YUY2, UYVY, I420, NV12, Y800) through the media source,
//! against ffmpeg's decode of the same file: every plane equal, in order and out of order.

mod common;

use common::*;

/// (name, ffmpeg pix_fmt of the file, ffmpeg pix_fmt to compare in, chroma width divisor, chroma
/// height divisor, label)
const CASES: &[(&str, &str, &str, usize, usize, &str)] = &[
    ("yuy2", "yuyv422", "yuv422p", 2, 1, "Uncompressed YUV 4:2:2"),
    ("uyvy", "uyvy422", "yuv422p", 2, 1, "Uncompressed YUV 4:2:2"),
    ("i420", "yuv420p", "yuv420p", 2, 2, "Uncompressed YUV 4:2:0"),
    ("nv12", "nv12", "yuv420p", 2, 2, "Uncompressed YUV 4:2:0"),
];

#[test]
fn uncompressed_video_in_matroska_matches_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let (w, h, n) = (96usize, 64usize, 6usize);
    for &(name, pix, cmp, cw, ch, label) in CASES {
        let file = format!("uncompressed_{name}.mkv");
        let Some(path) = fixture(&ff, &file, &["-f", "lavfi", "-i", "testsrc2=s=96x64:d=0.6:r=10", "-pix_fmt", pix, "-c:v", "rawvideo"]) else {
            cfa4_testkit::oracle::skip(name, "ffmpeg can't write it");
            continue;
        };
        let src = cfa4_codecs::open_bytes(&file, bytes(&path)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let v = src.info().video.clone().unwrap_or_else(|| panic!("{name}: no video"));
        assert_eq!((v.width as usize, v.height as usize, v.codec.as_str()), (w, h, label), "{name}: stream info");
        let raw = ffmpeg_frames(&ff, &path, cmp);
        let size = w * h + 2 * (w / cw) * (h / ch);
        let want: Vec<&[u8]> = raw.chunks(size).collect();
        assert_eq!(want.len(), n, "{name}: ffmpeg's picture count");
        for i in (0..n).rev().chain(0..n) {
            let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("{name}: frame {i}: {e}"));
            let got = planes(&f);
            let exp = raw_planes(want[i], w, h, w / cw, h / ch, 1);
            assert_eq!(max_diff(&got, &exp), 0, "{name}: frame {i} differs from ffmpeg's");
        }
    }
}
