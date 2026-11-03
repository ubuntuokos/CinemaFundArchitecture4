//! Decode speed (ignored; run with `cargo test --release -p cfa4-mpeg2v --test perf --
//! --ignored --nocapture`). Uses the 1080i fixture of `tests/oracle.rs` (ffmpeg-encoded, 25 Mb/s).

mod common;

use common::*;

#[test]
#[ignore]
fn perf_1080i_decode_fps() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let f = make(&ff, &HD1080I).expect("fixture");
    let es = std::fs::read(&f).unwrap();
    let aus = cfa4_mpeg2v::access_units(&es);
    for threads in [false, true] {
        let mut best = f64::MAX;
        let mut frames = 0;
        for _ in 0..5 {
            let mut d = cfa4_mpeg2v::Decoder::new();
            d.set_threads(threads);
            let t0 = std::time::Instant::now();
            let mut n = 0;
            for (i, r) in aus.iter().enumerate() {
                n += d.decode(&es[r.clone()], i as i64).unwrap().len();
            }
            n += d.flush().len();
            best = best.min(t0.elapsed().as_secs_f64());
            frames = n;
        }
        println!(
            "1080i25 MPEG-2 (4:2:0, 25 Mb/s, IBBP): {frames} frames in {:.1} ms = {:.1} fps ({})",
            best * 1e3,
            frames as f64 / best,
            if threads { "slice rows on rayon" } else { "single thread" }
        );
    }
}

/// Encode speed and quality against ffmpeg's MPEG-2 encoder at the same bitrate and GOP (I and
/// P pictures), both measured by decoding with our decoder.
#[test]
#[ignore]
fn perf_1080p_encode_fps_and_psnr() {
    use cfa4_mpeg2v::encode::{Config, Encoder, Input, Rate};
    let ff = cfa4_testkit::require_ffmpeg!();
    let (w, h, frames, rate) = (1920usize, 1080usize, 50usize, 15_000_000u64);
    let raw_path = cfa4_testkit::fixtures_dir("mpeg2v").join("enc_src_1080p.yuv");
    let ok = std::process::Command::new(&ff)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc2=size=1920x1080:rate=25"])
        .args(["-frames:v", &frames.to_string(), "-pix_fmt", "yuv420p", "-f", "rawvideo"])
        .arg(&raw_path)
        .status()
        .unwrap()
        .success();
    assert!(ok);
    let raw = std::fs::read(&raw_path).unwrap();
    let fs = w * h * 3 / 2;
    let psnr_of = |es: &[u8]| {
        let mut d = cfa4_mpeg2v::Decoder::new();
        let mut pics = Vec::new();
        for (i, r) in cfa4_mpeg2v::access_units(es).into_iter().enumerate() {
            pics.extend(d.decode(&es[r], i as i64).unwrap());
        }
        pics.extend(d.flush());
        assert_eq!(pics.len(), frames);
        let mut sse = 0f64;
        for (p, f) in pics.iter().zip(raw.chunks(fs)) {
            sse += p.y.iter().zip(&f[..w * h]).map(|(&a, &b)| (a as f64 - b as f64).powi(2)).sum::<f64>();
        }
        10.0 * (255f64 * 255.0 * (w * h * frames) as f64 / sse).log10()
    };
    let mut cfg = Config::new(w as u32, h as u32, 25, 1);
    cfg.rate = Rate::Bitrate { bits_per_second: rate };
    let mut enc = Encoder::new(cfg).unwrap();
    let t0 = std::time::Instant::now();
    let mut es = Vec::new();
    for f in raw.chunks(fs) {
        let (y, c) = f.split_at(w * h);
        let (cb, cr) = c.split_at(w * h / 4);
        es.extend(enc.encode(&Input { y, cb, cr, y_stride: w, c_stride: w / 2 }).unwrap().into_iter().flat_map(|p| p.data));
    }
    let secs = t0.elapsed().as_secs_f64();
    let ours = (es.len() as f64 * 8.0 * 25.0 / frames as f64, psnr_of(&es));
    let ff_path = cfa4_testkit::fixtures_dir("mpeg2v").join("enc_ffmpeg_1080p.m2v");
    let ok = std::process::Command::new(&ff)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "yuv420p", "-s", "1920x1080", "-r", "25", "-i"])
        .arg(&raw_path)
        .args(["-c:v", "mpeg2video", "-b:v", "15M", "-g", "15", "-bf", "0", "-f", "mpeg2video"])
        .arg(&ff_path)
        .status()
        .unwrap()
        .success();
    assert!(ok);
    let ff_es = std::fs::read(&ff_path).unwrap();
    let theirs = (ff_es.len() as f64 * 8.0 * 25.0 / frames as f64, psnr_of(&ff_es));
    println!("1080p25 MPEG-2 encode (I/P, GOP 15, 15 Mb/s target): {:.1} fps", frames as f64 / secs);
    println!("  ours:   {:.2} Mb/s, luma PSNR {:.2} dB", ours.0 / 1e6, ours.1);
    println!("  ffmpeg: {:.2} Mb/s, luma PSNR {:.2} dB", theirs.0 / 1e6, theirs.1);
}
