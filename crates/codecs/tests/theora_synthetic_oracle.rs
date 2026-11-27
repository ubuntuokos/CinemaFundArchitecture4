//! Theora in Matroska from synthetic streams, against ffmpeg's decoder. The headers are written
//! here (flat Huffman tables, random quantizers) and the frame payloads are random bytes, which
//! parse as valid frames: every coding flag, mode, vector and token is a bit field. So the whole
//! decoding path (coding flags, modes, vectors, tokens, dequantization, IDCT, motion
//! compensation, loop filter) is compared without a Theora encoder (ffmpeg 9 only decodes it).

mod common;

use std::io::Cursor;
use std::sync::Arc;

use cfa4_matroska::{MkvWriter, MuxOptions, TrackKind, TrackSpec};
use cfa4_theora::synth::{Rng, comment, ident, laced, packet, setup};
use common::*;

#[test]
fn synthetic_theora_in_matroska_matches_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    // (size, chroma format 0 / 2 / 3 as in the identification header, pixel format, seed)
    let sizes = [(64, 48), (96, 64), (48, 32), (80, 48)];
    let cases: Vec<((u32, u32), u64, &str, u64)> = (1..=36u64)
        .map(|seed| {
            let (pf, pix) = [(0, "yuv420p"), (2, "yuv422p"), (3, "yuv444p")][(seed % 3) as usize];
            (sizes[(seed / 3 % 4) as usize], pf, pix, seed)
        })
        .collect();
    for ((w, h), pf, pix, seed) in cases {
        let name = format!("theora_synth_{seed}.mkv");
        let mut rng = Rng::new(seed);
        let (id, cm, st) = (ident(w, h, pf), comment(), setup(&mut rng));
        let mut t = TrackSpec::new(TrackKind::Video, "V_THEORA");
        t.video_size = Some((w, h));
        t.codec_private = laced([&id, &cm, &st]);
        t.default_duration_ns = Some(40_000_000);
        let mut mux = MkvWriter::new(Cursor::new(Vec::new()), vec![t], MuxOptions::default()).expect("writer");
        let keys = [true, false, false, false, false, true, false, false, false, false];
        // A random inter frame often breaks a run-length rule (the runs must cover every
        // superblock exactly): draw until our decoder takes the frame
        let mut ours = cfa4_theora::Decoder::new(&id, &cm, &st).expect("headers");
        for (i, &key) in keys.iter().enumerate() {
            let p = (0..20_000).map(|_| packet(&mut rng, key)).find(|p| ours.decode(p).is_ok()).expect("a decodable random frame");
            mux.write_frame(0, i as i64 * 40_000_000, key, &p, Some(40_000_000)).expect("frame");
        }
        let data = mux.finish().expect("finish").into_inner();
        let path = dir().join(&name);
        std::fs::create_dir_all(dir()).unwrap();
        std::fs::write(&path, &data).unwrap();

        let src = cfa4_codecs::open_bytes(&name, Arc::<[u8]>::from(data)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let v = src.info().video.clone().unwrap();
        assert_eq!((v.width, v.height), (w, h), "{name}: reported size");
        let (sx, sy) = match pf {
            0 => (1, 1),
            2 => (1, 0),
            _ => (0, 0),
        };
        let (cw, ch) = ((w as usize).div_ceil(1 << sx), (h as usize).div_ceil(1 << sy));
        let raw = ffmpeg_frames(&ff, &path, pix);
        let frame_bytes = (w * h) as usize + 2 * cw * ch;
        let want: Vec<&[u8]> = raw.chunks(frame_bytes).collect();
        assert_eq!(want.len(), keys.len(), "{name}: ffmpeg's frame count");
        for i in (0..want.len()).chain((0..want.len()).rev()) {
            let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("{name}: frame {i}: {e}"));
            let (ours, theirs) = (planes(&f), raw_planes(want[i], w as usize, h as usize, cw, ch, 1));
            for (pl, (a, b)) in ours.iter().zip(&theirs).enumerate() {
                let width = if pl == 0 { w as usize } else { cw };
                let bad: Vec<usize> = (0..a.len()).filter(|&k| a[k] != b[k]).collect();
                assert!(
                    bad.is_empty(),
                    "{name}: frame {i} plane {pl} differs from ffmpeg's in {} samples, first (x, y, ours, ffmpeg): {:?}",
                    bad.len(),
                    bad.iter().take(100).map(|&k| (k % width, k / width, a[k], b[k])).collect::<Vec<_>>()
                );
            }
        }
    }
}
