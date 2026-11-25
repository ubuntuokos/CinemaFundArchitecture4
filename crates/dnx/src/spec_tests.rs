//! SMPTE ST 2019-1:2016 conformance of the default (standard) mode against the FFmpeg-compatible
//! mode (CFA4-DNX-SPEC-001 R1–R3): inverse quantisation (§8.2.7, equation (8.1)) and the 4:4:4
//! signalling (§6.3, §7.2 CLF, §7.3.1.1 ACF, §8.1.1 ConvertMacroblockColor).

use crate::dct::fdct8x8;
use crate::ffmpeg_probe::stream_cid;
use crate::header::SCAN_INDEX_OFFSET;
use crate::tables::{ZIGZAG, cid_info};
use crate::{ChromaFormat, ColorVolume, DecodeOptions, Encoder, EncoderConfig, Frame, Profile, decode_frame_with, probe};

const STD: DecodeOptions = DecodeOptions { threads: false, ffmpeg_compat: false };
const FFMPEG: DecodeOptions = DecodeOptions { threads: false, ffmpeg_compat: true };

/// The DCT coefficient at zig-zag position `pos` of the top-left luma block, measured with a
/// forward DCT of the decoded samples.
fn measured(f: &Frame, pos: usize) -> f32 {
    let mid = (1u32 << (f.bit_depth - 1)) as f32;
    let mut b = [0f32; 64];
    for j in 0..8 {
        for i in 0..8 {
            b[j * 8 + i] = f.y[j * f.width as usize + i] as f32 - mid;
        }
    }
    fdct8x8(&mut b);
    b[ZIGZAG[pos] as usize]
}

/// Equation (8.1) for an AC amplitude `a` at quantiser `qsf`.
fn eq_8_1(a: u32, w: u32, qsf: u32, p: u32) -> f32 {
    let add = if w != p { p / 2 } else { 0 };
    ((a * w * qsf + (w * qsf) / 2 + add) / p) as f32
}

#[test]
fn inverse_quantisation_follows_equation_8_1_by_default() {
    // one AC coefficient in the first luma block; every CID family with an 8- or 10-bit raster
    for (cid, w, h, depth) in [(1235u32, 1920u32, 1080u32, 10u8), (1237, 1920, 1080, 8), (1252, 1280, 720, 8), (1271, 256, 128, 10), (1273, 256, 128, 8)] {
        let info = cid_info(cid).unwrap();
        // coarse steps, so that the two reconstructions are far apart (a quarter step each)
        for (pos, amp, qsf) in [(1usize, 3u32, 40u32), (2, 2, 48), (9, 2, 60)] {
            let z = ZIGZAG[pos] as usize;
            let wt = info.weights[0][z - 1] as u32;
            let data = stream_cid(cid, w, h, depth, qsf, pos, amp, false, 0);
            let std = measured(&decode_frame_with(&data, &STD).unwrap(), pos);
            let ff = measured(&decode_frame_with(&data, &FFMPEG).unwrap(), pos);
            let want_std = eq_8_1(amp, wt, qsf, info.p);
            let want_ff = ((4 * amp + 3) * wt * qsf) as f32 / (4 * info.p) as f32;
            assert!((want_ff - want_std).abs() > 6.0, "CID {cid} pos {pos}: the test cannot tell the modes apart");
            // the samples are rounded to integers: the measured coefficient is within ±2.5
            assert!((std - want_std).abs() <= 2.5, "CID {cid} pos {pos}: (8.1) gives {want_std}, decoded {std}");
            assert!((ff - want_ff).abs() <= 2.5, "CID {cid} pos {pos}: (A + 3/4) gives {want_ff}, decoded {ff}");
        }
    }
}

/// A 4:4:4 test picture: smooth gradients in each plane, at `depth` bits.
fn picture(rgb: bool, depth: u8, w: u32, h: u32) -> Frame {
    let mut f = Frame::new(w, h, ChromaFormat::Yuv444, depth, false);
    f.rgb = rgb;
    let max = (1u32 << depth) - 1;
    for y in 0..h as usize {
        for x in 0..w as usize {
            let i = y * w as usize + x;
            f.y[i] = (max * x as u32 / w) as u16;
            f.cb[i] = (max * y as u32 / h) as u16;
            f.cr[i] = (max * ((x + y) as u32 % w) / w) as u16;
        }
    }
    f
}

/// The ACF flag of the first macroblock: the bit after its 11-bit qsf (§7.3.1.1).
fn first_acf(data: &[u8]) -> bool {
    let hs = probe(data).unwrap().header_size as usize;
    let first = u32::from_be_bytes(data[SCAN_INDEX_OFFSET..SCAN_INDEX_OFFSET + 4].try_into().unwrap()) as usize;
    (data[hs + first + 1] >> 4) & 1 == 1
}

fn encode(f: &Frame, compat: bool) -> Vec<u8> {
    let mut cfg = EncoderConfig::new(Profile::R444, f.width, f.height);
    cfg.bit_depth = f.bit_depth;
    cfg.ffmpeg_compat = compat;
    Encoder::with_config(cfg).unwrap().encode(f).unwrap()
}

fn max_diff(a: &[u16], b: &[u16]) -> u32 {
    a.iter().zip(b).map(|(&x, &y)| (x as i32 - y as i32).unsigned_abs()).max().unwrap_or(0)
}

#[test]
fn rgb_444_is_clf_1_in_r_g_b_order() {
    let src = picture(true, 10, 256, 128);
    let data = encode(&src, false);
    assert!(probe(&data).unwrap().rgb, "RGB must be signalled with CLF = 1 (§7.2)");
    assert!(!first_acf(&data), "RGB macroblocks are coded with ACF = 0");
    let f = decode_frame_with(&data, &STD).unwrap();
    assert!(f.rgb);
    for (name, a, b) in [("G", &f.y, &src.y), ("B", &f.cb, &src.cb), ("R", &f.cr, &src.cr)] {
        assert!(max_diff(a, b) <= 8, "{name} plane differs by {}", max_diff(a, b));
    }
    // Ch1 is red (§6.3): FFmpeg's reading (Ch1 = G) swaps the planes
    let ff = decode_frame_with(&data, &FFMPEG).unwrap();
    assert!(ff.rgb && max_diff(&ff.y, &src.cr) <= 8, "FFmpeg's order reads Ch1 (R) as G");
}

#[test]
fn ycbcr_444_is_clf_0() {
    let src = picture(false, 10, 256, 128);
    let data = encode(&src, false);
    assert!(!probe(&data).unwrap().rgb, "Y'CbCr must be signalled with CLF = 0 (§7.2)");
    assert!(!first_acf(&data), "ACF = 1 is only permitted with CLF = 1 (§7.3.1.1)");
    let f = decode_frame_with(&data, &STD).unwrap();
    assert!(!f.rgb);
    assert!(max_diff(&f.y, &src.y) <= 8 && max_diff(&f.cb, &src.cb) <= 8 && max_diff(&f.cr, &src.cr) <= 8);
    // FFmpeg reads every 4:4:4 stream with CLF = 0 as RGB
    assert!(decode_frame_with(&data, &FFMPEG).unwrap().rgb);
}

#[test]
fn ffmpeg_compat_round_trips_its_own_signalling() {
    for rgb in [true, false] {
        let src = picture(rgb, 10, 256, 128);
        let data = encode(&src, true);
        // FFmpeg: RGB with CLF = 0, Y'CbCr with CLF = 1 and ACF = 1
        assert_eq!(probe(&data).unwrap().rgb, !rgb);
        assert_eq!(first_acf(&data), !rgb);
        let f = decode_frame_with(&data, &FFMPEG).unwrap();
        assert_eq!(f.rgb, rgb);
        assert!(max_diff(&f.y, &src.y) <= 8 && max_diff(&f.cb, &src.cb) <= 8 && max_diff(&f.cr, &src.cr) <= 8);
    }
}

#[test]
fn acf_macroblocks_are_converted_to_rgb_by_the_standard() {
    // a flat Y'CbCr picture (DC only, so both reconstructions agree) coded with CLF = 1 and
    // ACF = 1 everywhere: FFmpeg returns Y'CbCr, the standard converts every macroblock (§8.1.1)
    let mut src = Frame::new(64, 32, ChromaFormat::Yuv444, 10, false);
    let (yv, cbv, crv) = (500u16, 400u16, 640u16);
    src.y.fill(yv);
    src.cb.fill(cbv);
    src.cr.fill(crv);
    let data = encode(&src, true);
    let ff = decode_frame_with(&data, &FFMPEG).unwrap();
    assert!(!ff.rgb);
    let std = decode_frame_with(&data, &STD).unwrap();
    assert!(std.rgb, "ACF = 1 macroblocks decode to RGB");
    // BT.709 on the quantised 10-bit signals (Rec. 709 §3.5)
    let (kr, kb) = (0.2126f32, 0.0722f32);
    let c = 219.0 / 224.0;
    let (y, cb, cr) = (ff.y[0] as f32, ff.cb[0] as f32 - 512.0, ff.cr[0] as f32 - 512.0);
    let r = y + 2.0 * (1.0 - kr) * c * cr;
    let b = y + 2.0 * (1.0 - kb) * c * cb;
    let g = (y - kr * r - kb * b) / (1.0 - kr - kb);
    for (name, got, want) in [("G", std.y[0], g), ("B", std.cb[0], b), ("R", std.cr[0], r)] {
        assert!((got as f32 - want).abs() <= 1.0, "{name}: {got}, BT.709 gives {want}");
    }
}

#[test]
fn acf_with_an_out_of_band_colour_volume_is_refused() {
    let mut src = Frame::new(64, 32, ChromaFormat::Yuv444, 10, false);
    src.y.fill(500);
    let mut cfg = EncoderConfig::new(Profile::R444, 64, 32);
    cfg.ffmpeg_compat = true;
    cfg.color_volume = ColorVolume::OutOfBand;
    let data = Encoder::with_config(cfg).unwrap().encode(&src).unwrap();
    assert!(decode_frame_with(&data, &STD).is_err(), "§7.3.1.1: ACF = 1 needs CLV != 11b");
    assert!(decode_frame_with(&data, &FFMPEG).is_ok());
}
