//! Bink 1 (`cfa4-bink`) through the media source against ffmpeg: a stream written by the
//! `cfa4-bink` test encoder (a first frame of fill, pattern and raw blocks, skipped frames, and a
//! second key frame, so seeking starts from either), with an RDFT or DCT audio track. Frames are
//! requested last to first, so every one of them is reached by seeking.

mod common;

#[path = "../../bink/tests/common/mod.rs"]
#[allow(dead_code, unused_imports, clippy::all)]
mod bink_gen;

use cfa4_frame::PixelData;
use common::*;

fn yuv(f: &cfa4_frame::VideoFrame) -> Vec<u8> {
    match &f.data {
        PixelData::Yuv8 { planes, alpha, .. } => planes.iter().flat_map(|p| p.iter().copied()).chain(alpha.iter().flat_map(|a| a.iter().copied())).collect(),
        _ => panic!("expected 8-bit YUV"),
    }
}

fn stream(ver: u8, w: u32, h: u32, dct: bool) -> Vec<u8> {
    let mut rng = bink_gen::Rng(42 + ver as u64);
    let mut video = vec![bink_gen::first_frame(&mut rng, ver, w, h, false)];
    for _ in 0..3 {
        video.push(bink_gen::skip_frame(ver, w, h));
    }
    video.push(bink_gen::first_frame(&mut rng, ver, w, h, false));
    for _ in 0..3 {
        video.push(bink_gen::skip_frame(ver, w, h));
    }
    let p = bink_gen::Params { rate: 22050, channels: 2, dct, revision: ver };
    let packets: Vec<Vec<u8>> = (0..video.len()).map(|_| bink_gen::packet(&mut rng, &p, 1)).collect();
    let flags = 0x2000 | if dct { 0x1000 } else { 0 };
    bink_gen::bik_keys(&[b'B', b'I', b'K', ver], w, h, 0, &video, &[bink_gen::Track { rate: 22050, flags, packets }], &[0, 4])
}

#[test]
fn frames_and_audio_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::ffmpeg_or_skip("bink media source oracle") else { return };
    let dir = cfa4_testkit::fixtures_dir("bink");
    for (ver, w, h, dct) in [(b'i', 40u32, 24u32, false), (b'k', 34, 18, true), (b'h', 16, 16, false)] {
        let name = format!("source-{}-{w}x{h}.bik", ver as char);
        let path = dir.join(&name);
        std::fs::write(&path, stream(ver, w, h, dct)).unwrap();
        let src = cfa4_codecs::open_bytes(&name, bytes(&path)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(src.info().container, "Bink");
        let v = src.info().video.clone().unwrap();
        assert_eq!((v.width, v.height), (w, h));
        let raw = ffmpeg_out(&ff, &["-i", path.to_str().unwrap(), "-map", "0:v:0", "-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "yuv420p", "-"]);
        let size = (w * h + 2 * w.div_ceil(2) * h.div_ceil(2)) as usize;
        let want: Vec<&[u8]> = raw.chunks(size).collect();
        assert_eq!(want.len(), 8, "{name}: ffmpeg decoded {} frames", want.len());
        for i in (0..want.len()).rev() {
            let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("{name}: frame {i}: {e}"));
            let got = yuv(&f);
            if let Some(at) = got.iter().zip(want[i]).position(|(a, b)| a != b) {
                panic!("{name}: frame {i} differs from ffmpeg's at byte {at}");
            }
        }
        // the audio: planar from ffmpeg, in two reads
        let a = src.info().audio_streams[0].clone();
        assert_eq!((a.sample_rate, a.channels), (22050, 2));
        let pcm = ffmpeg_out(&ff, &["-i", path.to_str().unwrap(), "-map", "0:a:0", "-f", "f32le", "-"]);
        let theirs: Vec<f32> = pcm.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        let n = theirs.len() / 2;
        let take = n.min(20_000);
        // the second half first (a jump back to a checkpoint), then the start
        let late = src.audio(1000, take - 1000, 22050).unwrap();
        let early = src.audio(0, take, 22050).unwrap();
        for (label, buf, off) in [("late", &late, 1000usize), ("early", &early, 0usize)] {
            let (mut sig, mut err) = (0f64, 0f64);
            for c in 0..2 {
                for (i, &s) in buf.channels[c].iter().enumerate() {
                    let t = theirs[(off + i) * 2 + c] as f64;
                    sig += t * t;
                    err += (s as f64 - t).powi(2);
                }
            }
            let snr = 10.0 * (sig / err.max(1e-30)).log10();
            assert!(snr > 100.0, "{name}: {label} audio SNR {snr:.1} dB");
        }
    }
}
