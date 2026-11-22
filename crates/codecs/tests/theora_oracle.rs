//! Theora (`cfa4-theora`) in Matroska (`V_THEORA`) against ffmpeg's decode of the same stream:
//! every frame equal, in order, backwards and after jumps.
//!
//! The streams in `data/theora/*.bin` were produced once by libtheora (4:2:0, 4:2:2, 4:4:4, a
//! cropped picture, three quality settings); each file is the packet list (`u32` LE length +
//! bytes): three headers, then the frames. So the test needs ffmpeg only as a *decoder* (the
//! ffmpeg 9 build in CI has no libtheora encoder): it muxes the packets into Matroska here and
//! compares our decode with ffmpeg's.

mod common;

use std::io::Cursor;
use std::sync::Arc;

use cfa4_matroska::{MkvWriter, MuxOptions, TrackKind, TrackSpec};
use cfa4_theora::synth::laced;
use common::*;

/// Split a `.bin` fixture into its packets.
fn packets(mut d: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    while !d.is_empty() {
        let n = u32::from_le_bytes(d[..4].try_into().unwrap()) as usize;
        out.push(&d[4..4 + n]);
        d = &d[4 + n..];
    }
    out
}

#[test]
fn theora_in_matroska_matches_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    // (name, size, pixel format, stream)
    let cases: [(&str, (u32, u32), &str, &[u8]); 6] = [
        ("theora_420", (96, 64), "yuv420p", include_bytes!("data/theora/theora_420.bin")),
        ("theora_420_q", (96, 64), "yuv420p", include_bytes!("data/theora/theora_420_q.bin")),
        ("theora_420_lowq", (96, 64), "yuv420p", include_bytes!("data/theora/theora_420_lowq.bin")),
        ("theora_422", (80, 48), "yuv422p", include_bytes!("data/theora/theora_422.bin")),
        ("theora_444", (80, 48), "yuv444p", include_bytes!("data/theora/theora_444.bin")),
        ("theora_crop", (90, 66), "yuv420p", include_bytes!("data/theora/theora_crop.bin")),
    ];
    for (name, (w, h), pix, stream) in cases {
        let pk = packets(stream);
        let frames = &pk[3..];
        let mut t = TrackSpec::new(TrackKind::Video, "V_THEORA");
        t.video_size = Some((w, h));
        t.codec_private = laced([pk[0], pk[1], pk[2]]);
        t.default_duration_ns = Some(40_000_000);
        let mut mux = MkvWriter::new(Cursor::new(Vec::new()), vec![t], MuxOptions::default()).expect("writer");
        for (i, p) in frames.iter().enumerate() {
            // bit 6 of the first byte: 0 = intra frame
            mux.write_frame(0, i as i64 * 40_000_000, p[0] & 0x40 == 0, p, Some(40_000_000)).expect("frame");
        }
        let data = mux.finish().expect("finish").into_inner();
        let file = format!("{name}.mkv");
        let path = dir().join(&file);
        std::fs::create_dir_all(dir()).unwrap();
        std::fs::write(&path, &data).unwrap();

        let src = cfa4_codecs::open_bytes(&file, Arc::<[u8]>::from(data)).unwrap_or_else(|e| panic!("{name}: {e}"));
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
        assert_eq!(want.len(), frames.len(), "{name}: ffmpeg's frame count");
        for i in (0..want.len()).rev().chain(0..want.len()) {
            let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("{name}: frame {i}: {e}"));
            assert_eq!((f.width, f.height), (w, h), "{name}: frame {i} size");
            assert!(planes(&f) == raw_planes(want[i], w as usize, h as usize, cw, ch, 1), "{name}: frame {i} differs from ffmpeg's");
        }
    }
}
