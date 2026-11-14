//! RealMedia with RealVideo 1.0 to 4.0 (`cfa4-realmedia` + `cfa4-realvideo`) through the media
//! source against ffmpeg: streams from ffmpeg's `rv10` / `rv20` encoders, and, when present,
//! ffmpeg's FATE sample `real/G2_with_SVT_320_240.rm` (RealVideo 2.0 with B-frames, from
//! `https://fate-suite.ffmpeg.org/real/`) in `target/fixtures/real`, and the FATE samples `rv30.rm`
//! (RealVideo 3.0) and `spygames-2MB.rmvb` (RealVideo 4.0) there. Frames are requested last
//! to first, so every one of them is reached by seeking. ffmpeg runs with its simple IDCT, the one
//! the decoder implements.

mod common;

use std::path::Path;

use cfa4_frame::PixelData;
use common::*;

fn yuv(f: &cfa4_frame::VideoFrame) -> Vec<u8> {
    match &f.data {
        PixelData::Yuv8 { planes, .. } => planes.iter().flat_map(|p| p.iter().copied()).collect(),
        _ => panic!("expected 8-bit YUV"),
    }
}

fn compare(ff: &Path, name: &str, path: &Path, codec: &str) -> usize {
    let src = cfa4_codecs::open_bytes(name, bytes(path)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let v = src.info().video.clone().unwrap_or_else(|| panic!("{name}: no video"));
    assert_eq!(v.codec, codec, "{name}: codec label");
    assert_eq!(src.info().container, "RealMedia");
    let raw = ffmpeg_out(
        ff,
        &[
            "-idct",
            "simple",
            "-flags",
            "+bitexact",
            "-i",
            path.to_str().unwrap(),
            "-map",
            "0:v:0",
            "-fps_mode",
            "passthrough",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv420p",
            "-",
        ],
    );
    let (w, h) = (v.width as usize, v.height as usize);
    let size = w * h + 2 * w.div_ceil(2) * h.div_ceil(2);
    let want: Vec<&[u8]> = raw.chunks(size).collect();
    assert!(want.len() > 2, "{name}: ffmpeg decoded {} frames", want.len());
    let frames = v.frame_rate.frame_at(src.info().duration);
    // ffmpeg leaves out the last reference picture of a stream with B-frames; an audio stream
    // can last longer than the video
    let only_video = src.info().audio_streams.is_empty();
    assert!(
        frames == want.len() as i64 || frames == want.len() as i64 + 1 || (!only_video && frames > want.len() as i64),
        "{name}: {frames} frames, ffmpeg {}",
        want.len()
    );
    for i in (0..want.len()).rev() {
        let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("{name}: frame {i}: {e}"));
        let got = yuv(&f);
        assert_eq!(got.len(), want[i].len(), "{name}: frame {i} size");
        if let Some(at) = got.iter().zip(want[i]).position(|(a, b)| a != b) {
            panic!("{name}: frame {i} differs from ffmpeg's at byte {at}: ours {}, ffmpeg {}", got[at], want[i][at]);
        }
    }
    want.len()
}

#[test]
fn ffmpeg_encoded_rv10_and_rv20_match() {
    let Some(ff) = cfa4_testkit::ffmpeg_or_skip("realmedia oracle") else { return };
    for (codec, label, size) in [("rv10", "RealVideo 1.0", "176x144"), ("rv20", "RealVideo 2.0", "320x240")] {
        let name = format!("realmedia-{codec}.rm");
        let src = format!("testsrc2=size={size}:rate=25:duration=2,noise=alls=8:allf=t");
        let Some(p) = fixture(&ff, &name, &["-f", "lavfi", "-i", &src, "-c:v", codec, "-g", "12", "-q:v", "4", "-f", "rm"]) else {
            panic!("{name}: ffmpeg could not make the fixture");
        };
        assert_eq!(compare(&ff, &name, &p, label), 50);
    }
}

#[test]
fn fate_rv20_with_b_frames_matches() {
    let Some(ff) = cfa4_testkit::ffmpeg_or_skip("realmedia FATE") else { return };
    let p = cfa4_testkit::fixtures_dir("real").join("G2_with_SVT_320_240.rm");
    if !p.exists() {
        eprintln!("skipping: {} not present", p.display());
        return;
    }
    compare(&ff, "G2_with_SVT_320_240.rm", &p, "RealVideo 2.0");
}

#[test]
fn fate_rv30_and_rv40_match() {
    let Some(ff) = cfa4_testkit::ffmpeg_or_skip("realmedia FATE") else { return };
    for (file, label) in [("rv30.rm", "RealVideo 3.0"), ("spygames-2MB.rmvb", "RealVideo 4.0")] {
        let p = cfa4_testkit::fixtures_dir("real").join(file);
        if !p.exists() {
            eprintln!("skipping: {} not present", p.display());
            continue;
        }
        compare(&ff, file, &p, label);
    }
}

/// ffmpeg's Cook samples of the first audio stream, planar.
fn ffmpeg_audio(ff: &Path, path: &Path, ch: usize) -> Vec<Vec<f32>> {
    let raw = ffmpeg_out(ff, &["-flags", "+bitexact", "-i", path.to_str().unwrap(), "-vn", "-map", "0:a:0", "-f", "f32le", "-c:a", "pcm_f32le", "-"]);
    let v: Vec<f32> = raw.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    (0..ch).map(|c| v.iter().skip(c).step_by(ch).copied().collect()).collect()
}

/// Cook through the media source: the timeline starts with the two frames the decoder needs to
/// start up (silence, ffmpeg outputs nothing for them); after that every sample equals ffmpeg's
/// within rounding, read in order and with jumps backwards.
fn compare_cook(ff: &Path, file: &str, channels: u32) {
    let p = cfa4_testkit::fixtures_dir("real").join(file);
    if !p.exists() {
        eprintln!("skipping: {} not present", p.display());
        return;
    }
    let src = cfa4_codecs::open_bytes(file, bytes(&p)).unwrap_or_else(|e| panic!("{file}: {e}"));
    let a = src.info().audio_streams.first().cloned().unwrap_or_else(|| panic!("{file}: no audio"));
    assert_eq!((a.codec.as_str(), a.channels, a.sample_rate), ("Cook", channels, 44_100), "{file}");
    let want = ffmpeg_audio(ff, &p, channels as usize);
    let n = want[0].len();
    assert!(n > 10_000, "{file}: ffmpeg decoded {n} samples");
    // two frames of 256, 512 or 1024 samples: the lead that lines the first sound up
    let at = want[0].iter().position(|s| s.abs() > 1e-3).unwrap_or(0);
    let first = src.audio(0, at + 4096, 44_100).unwrap();
    let lead = [512usize, 1024, 2048]
        .into_iter()
        .min_by(|&x, &y| {
            let err = |l: usize| first.channels[0][at + l..at + l + 2000].iter().zip(&want[0][at..]).map(|(a, b)| f64::from((a - b).abs())).sum::<f64>();
            err(x).total_cmp(&err(y))
        })
        .unwrap_or(0);
    assert!(first.channels.iter().all(|c| c[..lead].iter().all(|&s| s == 0.0)), "{file}: the start-up frames are not silent");
    let check = |start: usize, len: usize| {
        let got = src.audio((lead + start) as i64, len, 44_100).unwrap_or_else(|e| panic!("{file}: {e}"));
        for (c, (g, w)) in got.channels.iter().zip(&want).enumerate() {
            for (i, (x, y)) in g.iter().zip(&w[start..(start + len).min(n)]).enumerate() {
                if (x - y).abs() >= 1e-6 {
                    let k = start + i;
                    panic!(
                        "{file}: lead {lead} channel {c} sample {k} ours {:?} ffmpeg {:?}",
                        &g[i.saturating_sub(3)..(i + 3).min(g.len())],
                        &w[k.saturating_sub(3)..k + 3]
                    );
                }
            }
        }
    };
    for s in (0..n).step_by(8192) {
        check(s, 8192.min(n - s));
    }
    for s in (0..n).step_by(30_000).rev() {
        check(s, 4000.min(n - s));
    }
}

#[test]
fn fate_cook_audio_matches() {
    let Some(ff) = cfa4_testkit::ffmpeg_or_skip("realmedia Cook") else { return };
    compare_cook(&ff, "ra_cook.rm", 2);
    compare_cook(&ff, "rv30.rm", 1);
    compare_cook(&ff, "spygames-2MB.rmvb", 2);
}

/// A mono RealAudio stream with no start-up delay through the media source: every sample equals
/// ffmpeg's, read in order and with jumps backwards.
fn compare_mono(ff: &Path, p: &Path, codec: &str, rate: u32) {
    let file = p.file_name().and_then(|f| f.to_str()).unwrap_or("?");
    let src = cfa4_codecs::open_bytes(file, bytes(p)).unwrap_or_else(|e| panic!("{file}: {e}"));
    let a = src.info().audio_streams.first().cloned().unwrap_or_else(|| panic!("{file}: no audio"));
    assert_eq!((a.codec.as_str(), a.channels, a.sample_rate), (codec, 1, rate), "{file}");
    let want = ffmpeg_audio(ff, p, 1).remove(0);
    let n = want.len();
    assert!(n > 5000, "{file}: ffmpeg decoded {n} samples");
    let check = |start: usize, len: usize| {
        let got = src.audio(start as i64, len, rate).unwrap_or_else(|e| panic!("{file}: {e}"));
        for (i, (x, y)) in got.channels[0].iter().zip(&want[start..start + len]).enumerate() {
            assert!(x.to_bits() == y.to_bits(), "{file}: sample {} ours {x} ffmpeg {y}", start + i);
        }
    };
    for s in (0..n).step_by(7000) {
        check(s, 7000.min(n - s));
    }
    for s in (0..n).step_by(25_000).rev() {
        check(s, 3000.min(n - s));
    }
}

#[test]
fn realaudio_1_encoded_matches() {
    let Some(ff) = cfa4_testkit::ffmpeg_or_skip("realmedia RealAudio 1.0") else { return };
    let p = cfa4_testkit::fixtures_dir("realmedia").join("ra144.rm");
    let st = std::process::Command::new(&ff)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "aevalsrc=0.6*sin(2*PI*(200+300*t)*t)+0.1*sin(2*PI*1900*t):s=8000:d=6"])
        .args(["-ac", "1", "-ar", "8000", "-c:a", "real_144", "-f", "rm"])
        .arg(&p)
        .status()
        .unwrap();
    assert!(st.success());
    compare_mono(&ff, &p, "RealAudio 1.0 (14.4)", 8000);
}

#[test]
fn fate_realaudio_lpc_matches() {
    let Some(ff) = cfa4_testkit::ffmpeg_or_skip("realmedia RealAudio FATE") else { return };
    for (dir, file, codec, rate) in [
        ("real", "ra3_in_rm_file.rm", "RealAudio 1.0 (14.4)", 8000),
        ("real", "ra_288.rm", "RealAudio 2.0 (28.8)", 8000),
        ("sipr", "sipr_5k0.rm", "RealAudio SIPR", 8000),
        ("sipr", "sipr_6k5.rm", "RealAudio SIPR", 8000),
        ("sipr", "sipr_8k5.rm", "RealAudio SIPR", 8000),
        ("sipr", "sipr_16k.rm", "RealAudio SIPR", 16_000),
    ] {
        let p = cfa4_testkit::fixtures_dir(dir).join(file);
        if !p.exists() {
            eprintln!("skipping: {} not present", p.display());
            continue;
        }
        compare_mono(&ff, &p, codec, rate);
    }
}
