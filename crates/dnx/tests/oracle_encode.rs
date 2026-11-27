//! Encode DNxHR with our encoder, decode with ffmpeg (`-xerror`) and with our decoder.

mod common;
use cfa4_dnx::{ChromaFormat, DecodeOptions, Encoder, EncoderConfig, Profile, decode_frame_with};
use common::*;

const COMPAT: DecodeOptions = DecodeOptions { threads: true, ffmpeg_compat: true };

struct Case {
    name: &'static str,
    profile: Profile,
    depth: u8,
    lavfi: &'static str,
    min_psnr: f64,
}

#[test]
fn encode_decodes_in_ffmpeg() {
    let Some(ff) = ffmpeg() else { return };
    let (w, h) = (1920u32, 1080u32);
    let cases = [
        Case { name: "enc_sq_mandel", profile: Profile::Sq, depth: 8, lavfi: "mandelbrot=s=1920x1080:r=25", min_psnr: 40.0 },
        Case { name: "enc_hq_mandel", profile: Profile::Hq, depth: 8, lavfi: "mandelbrot=s=1920x1080:r=25", min_psnr: 45.0 },
        Case { name: "enc_hq_testsrc", profile: Profile::Hq, depth: 8, lavfi: "testsrc2=s=1920x1080:r=25", min_psnr: 45.0 },
        Case { name: "enc_hqx10_mandel", profile: Profile::Hqx, depth: 10, lavfi: "mandelbrot=s=1920x1080:r=25", min_psnr: 45.0 },
        Case { name: "enc_hqx12_testsrc", profile: Profile::Hqx, depth: 12, lavfi: "testsrc2=s=1920x1080:r=25", min_psnr: 45.0 },
        Case { name: "enc_lb_testsrc", profile: Profile::Lb, depth: 8, lavfi: "testsrc2=s=1920x1080:r=25", min_psnr: 30.0 },
        Case { name: "enc_444_testsrc", profile: Profile::R444, depth: 10, lavfi: "testsrc2=s=1920x1080:r=25", min_psnr: 45.0 },
    ];
    for c in cases {
        let mut cfg = EncoderConfig::new(c.profile, w, h);
        cfg.bit_depth = c.depth;
        // FFmpeg is the oracle: write what its decoder expects (CFA4-DNX-SPEC-001 R3)
        cfg.ffmpeg_compat = true;
        let chroma = cfg.chroma();
        let src = source_frames(&ff, c.name, c.lavfi, 2, w, h, chroma, c.depth);
        let mut enc = Encoder::with_config(cfg).unwrap();
        let t = std::time::Instant::now();
        let coded: Vec<Vec<u8>> = src.iter().map(|f| enc.encode(f).unwrap()).collect();
        let ms = t.elapsed().as_secs_f64() * 1000.0 / src.len() as f64;
        for p in &coded {
            assert_eq!(p.len() as u32, enc.frame_size(), "{}: CBR frame size", c.name);
        }
        let mov = fixture_dir().join(format!("{}.out.mov", c.name));
        write_mov(&mov, b"AVdh", w as u16, h as u16, 24, &coded);
        let pix = match (chroma, c.depth) {
            (ChromaFormat::Yuv444, 10) => "yuv444p10le",
            (_, 8) => "yuv422p",
            (_, 10) => "yuv422p10le",
            _ => "yuv422p12le",
        };
        // ffmpeg reads 4:4:4 Y'CbCr DNxHR as gbrp unless every macroblock is flagged
        let ff_raw = ffmpeg_decode(&ff, &mov, pix);
        let mut agree = Stats::default();
        let mut psnr_min = f64::MAX;
        let per = frame_samples(&src[0]);
        assert_eq!(ff_raw.len(), per * coded.len(), "{}: ffmpeg frame count", c.name);
        for (i, p) in coded.iter().enumerate() {
            let f = decode_frame_with(p, &COMPAT).unwrap();
            let ours = planes(&f, pix);
            agree.add(&ours, &ff_raw[i * per..(i + 1) * per]);
            let peak = ((1u32 << c.depth) - 1) as f64;
            let srcp = planes(&src[i], pix);
            psnr_min = psnr_min.min(psnr(&srcp[..f.y.len()], &ours[..f.y.len()], peak));
        }
        println!(
            "{:<18} {} {}-bit: {} bytes/frame, {:.1} ms/frame, Y PSNR {:.2} dB, vs ffmpeg max {} exact {:.2}%",
            c.name,
            c.profile.name(),
            c.depth,
            enc.frame_size(),
            ms,
            psnr_min,
            agree.max,
            agree.exact_pct()
        );
        assert!(agree.max <= 2, "{}: ffmpeg disagrees by {}", c.name, agree.max);
        assert!(psnr_min >= c.min_psnr, "{}: PSNR {psnr_min:.2}", c.name);
    }
}

/// The progressive DNxHD IDs, decoded by ffmpeg from a MOV (`AVdn`).
#[test]
fn dnxhd_encode_decodes_in_ffmpeg() {
    let Some(ff) = ffmpeg() else { return };
    for profile in Profile::ALL.into_iter().filter(|p| p.hd_raster().is_some()) {
        let (w, h, depth) = profile.hd_raster().unwrap();
        let mut cfg = EncoderConfig::new(profile, w, h);
        cfg.ffmpeg_compat = true;
        let chroma = cfg.chroma();
        let name = format!("enc_dnxhd_{}", profile.cid());
        let lavfi = format!("testsrc2=s={w}x{h}:r=25");
        let src = source_frames(&ff, &name, &lavfi, 2, w, h, chroma, depth);
        let mut enc = Encoder::with_config(cfg).unwrap();
        let coded: Vec<Vec<u8>> = src.iter().map(|f| enc.encode(f).unwrap()).collect();
        let mov = fixture_dir().join(format!("{name}.out.mov"));
        write_mov(&mov, b"AVdn", w as u16, h as u16, 24, &coded);
        let pix = match (chroma, depth) {
            (ChromaFormat::Yuv444, _) => "yuv444p10le",
            (_, 8) => "yuv422p",
            _ => "yuv422p10le",
        };
        let ff_raw = ffmpeg_decode(&ff, &mov, pix);
        let per = frame_samples(&src[0]);
        assert_eq!(ff_raw.len(), per * coded.len(), "{name}: ffmpeg frame count");
        let mut agree = Stats::default();
        let mut psnr_min = f64::MAX;
        for (i, p) in coded.iter().enumerate() {
            let f = decode_frame_with(p, &COMPAT).unwrap();
            let ours = planes(&f, pix);
            agree.add(&ours, &ff_raw[i * per..(i + 1) * per]);
            let srcp = planes(&src[i], pix);
            psnr_min = psnr_min.min(psnr(&srcp[..f.y.len()], &ours[..f.y.len()], ((1u32 << depth) - 1) as f64));
        }
        println!("{name}: {} bytes/frame, Y PSNR {psnr_min:.2} dB, vs ffmpeg max {} exact {:.2}%", enc.frame_size(), agree.max, agree.exact_pct());
        assert!(agree.max <= 2, "{name}: ffmpeg disagrees by {}", agree.max);
        assert!(psnr_min >= if profile == Profile::Hd1253 { 30.0 } else { 40.0 }, "{name}: PSNR {psnr_min:.2}");
    }
}

/// CFA4-DNX-SPEC-001 R4: 4:4:4 streams written by the standard, read by the pinned FFmpeg.
///
/// Y'CbCr (CLF = 0) decodes without any error. RGB (CLF = 1, ACF = 0, R, G, B) is not supported
/// by FFmpeg: it reports "Unsupported: variable ACT flag" and decodes the channels as Y'CbCr, so
/// FFmpeg-based tools show wrong colours. The owner's decision (2026-11-24): standard RGB stays
/// the default and the export panel warns, pointing to the FFmpeg-compatible setting. This test
/// pins that FFmpeg behaviour, so a change in FFmpeg is noticed.
#[test]
fn standard_444_streams_in_ffmpeg() {
    let Some(ff) = ffmpeg() else { return };
    let (w, h) = (1280u32, 720u32);
    for rgb in [false, true] {
        let name = format!("enc_444_standard_{}", if rgb { "rgb" } else { "ycbcr" });
        let mut src = source_frames(&ff, &name, "testsrc2=s=1280x720:r=25", 2, w, h, ChromaFormat::Yuv444, 10);
        for f in &mut src {
            f.rgb = rgb;
        }
        let mut cfg = EncoderConfig::new(Profile::R444, w, h);
        cfg.bit_depth = 10;
        let mut enc = Encoder::with_config(cfg).unwrap();
        let coded: Vec<Vec<u8>> = src.iter().map(|f| enc.encode(f).unwrap()).collect();
        let mov = fixture_dir().join(format!("{name}.out.mov"));
        write_mov(&mov, b"AVdh", w as u16, h as u16, 24, &coded);
        let per = frame_samples(&src[0]);
        if !rgb {
            let raw = ffmpeg_decode(&ff, &mov, "yuv444p10le");
            assert_eq!(raw.len(), per * coded.len(), "{name}: FFmpeg frame count");
            continue;
        }
        let out = std::process::Command::new(&ff)
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(&mov)
            .args(["-f", "rawvideo", "-pix_fmt", "gbrp10le", "-"])
            .output()
            .expect("run ffmpeg");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{name}: FFmpeg failed: {err}");
        assert_eq!(out.stdout.len(), per * coded.len() * 2, "{name}: FFmpeg frame count");
        assert!(err.contains("variable ACT flag"), "{name}: FFmpeg now reads standard RGB without complaint ({err:?}); revisit CFA4-DNX-SPEC-001 R4");
    }
}
