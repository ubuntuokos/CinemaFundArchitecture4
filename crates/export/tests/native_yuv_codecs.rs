//! READY-V5.1 (CFA4-DECKLINK-READY-001): a 10-bit 4:2:2 `VideoFrame` reaches the ProRes and
//! DNxHR encoders as planar Y'CbCr, with no RGBA round trip, and decodes back within 50 dB luma
//! PSNR.
//!
//! The property this guards: the ProRes and DNx encoder inputs stay planar 16-bit Y'CbCr
//! (`cfa4_prores::Frame`, `cfa4_dnx::Frame`), so a capture source that delivers native YUV
//! (`PixelData::Yuv16`) can be encoded without converting through RGBA. An RGBA input may be
//! added later only next to this path, never in its place.

use std::sync::Arc;

use cfa4_color::ColorInfo;
use cfa4_frame::{Chroma, FrameLayout, PixelData, VideoFrame};
use cfa4_time::Tick;

const W: u32 = 256;
const H: u32 = 144;

/// A 10-bit 4:2:2 test picture: smooth gradients with a gentle texture, limited range.
fn yuv422_10() -> VideoFrame {
    let (w, h) = (W as usize, H as usize);
    let cw = w / 2;
    let mut y = vec![0u16; w * h];
    let mut cb = vec![0u16; cw * h];
    let mut cr = vec![0u16; cw * h];
    for row in 0..h {
        for x in 0..w {
            let ramp = 64.0 + 876.0 * (x as f64 / (w - 1) as f64) * 0.8 + 60.0 * (row as f64 / (h - 1) as f64);
            let texture = 12.0 * ((x as f64 * 0.21).sin() * (row as f64 * 0.17).cos());
            y[row * w + x] = (ramp + texture).round().clamp(64.0, 940.0) as u16;
        }
        for x in 0..cw {
            cb[row * cw + x] = (512.0 + 200.0 * ((x as f64 / cw as f64) - 0.5)).round() as u16;
            cr[row * cw + x] = (512.0 - 150.0 * ((row as f64 / h as f64) - 0.5)).round() as u16;
        }
    }
    VideoFrame {
        width: W,
        height: H,
        data: PixelData::Yuv16 { planes: [Arc::new(y), Arc::new(cb), Arc::new(cr)], chroma: Chroma::C422, bits: 10, alpha: None },
        color: ColorInfo::default(),
        par: (1, 1),
        pts: Tick(0),
        layout: FrameLayout::default(),
    }
}

/// The planes of a 10-bit 4:2:2 frame, borrowed as they are: no conversion of any kind.
fn planes(f: &VideoFrame) -> (&[u16], &[u16], &[u16]) {
    match &f.data {
        PixelData::Yuv16 { planes, chroma: Chroma::C422, bits: 10, alpha: None } => (&planes[0], &planes[1], &planes[2]),
        other => panic!("expected 10-bit 4:2:2 Y'CbCr, got {}", other_label(other)),
    }
}

fn other_label(d: &PixelData) -> &'static str {
    match d {
        PixelData::Rgba8(_) => "RGBA8",
        PixelData::RgbaF32(_) => "RGBA f32",
        PixelData::Yuv8 { .. } => "8-bit Y'CbCr",
        PixelData::Yuv16 { .. } => "Y'CbCr of another layout",
        PixelData::Rgb16 { .. } => "RGB16",
    }
}

/// Luma PSNR in dB for 10-bit samples.
fn psnr10(a: &[u16], b: &[u16]) -> f64 {
    assert_eq!(a.len(), b.len());
    let mse = a.iter().zip(b).map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2)).sum::<f64>() / a.len() as f64;
    if mse == 0.0 { f64::INFINITY } else { 10.0 * (1023.0f64 * 1023.0 / mse).log10() }
}

#[test]
fn yuv16_422_10bit_encodes_to_prores_422_hq_without_rgba() {
    let src = yuv422_10();
    let (y, cb, cr) = planes(&src);
    let mut fr = cfa4_prores::Frame::new(W, H, cfa4_prores::ChromaFormat::Yuv422, 10, false);
    fr.y.copy_from_slice(y);
    fr.cb.copy_from_slice(cb);
    fr.cr.copy_from_slice(cr);
    let mut enc = cfa4_prores::Encoder::new(cfa4_prores::Profile::Hq, W, H);
    let packet = enc.encode(&fr).expect("ProRes 422 HQ encodes planar 10-bit input");
    let out = cfa4_prores::decode_frame(&packet).expect("decodes");
    assert_eq!((out.width, out.height, out.chroma), (W, H, cfa4_prores::ChromaFormat::Yuv422));
    let luma = if out.bit_depth == 10 { out.y.clone() } else { out.y.iter().map(|&v| v >> (out.bit_depth - 10)).collect() };
    let p = psnr10(y, &luma);
    assert!(p >= 50.0, "ProRes 422 HQ luma PSNR {p:.2} dB < 50 dB");
}

#[test]
fn yuv16_422_10bit_encodes_to_dnxhr_hqx_without_rgba() {
    let src = yuv422_10();
    let (y, cb, cr) = planes(&src);
    let mut fr = cfa4_dnx::Frame::new(W, H, cfa4_dnx::ChromaFormat::Yuv422, 10, false);
    fr.y.copy_from_slice(y);
    fr.cb.copy_from_slice(cb);
    fr.cr.copy_from_slice(cr);
    let mut enc = cfa4_dnx::Encoder::new(cfa4_dnx::Profile::Hqx, W, H).expect("DNxHR HQX takes this raster");
    let packet = enc.encode(&fr).expect("DNxHR HQX encodes planar 10-bit input");
    let out = cfa4_dnx::decode_frame(&packet).expect("decodes");
    assert_eq!((out.width, out.height, out.chroma), (W, H, cfa4_dnx::ChromaFormat::Yuv422));
    let luma: Vec<u16> = match out.bit_depth {
        10 => out.y.clone(),
        d if d > 10 => out.y.iter().map(|&v| v >> (d - 10)).collect(),
        d => out.y.iter().map(|&v| v << (10 - d)).collect(),
    };
    let p = psnr10(y, &luma);
    assert!(p >= 50.0, "DNxHR HQX luma PSNR {p:.2} dB < 50 dB");
}
