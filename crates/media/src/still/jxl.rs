//! JPEG XL (`.jxl`) stills through the native decoder `cfa4-jxl` (ISO/IEC 18181, a safe port of
//! jxl-oxide). The first frame of the file is shown; 8-bit sRGB pictures stay RGBA8, everything
//! else (more bits, HDR, wide gamut, linear) becomes premultiplied scene-linear RGBA floats so the
//! extra precision reaches effects and colour work.

use std::sync::Arc;

use cfa4_color::{ColorInfo, Primaries, Transfer};
use cfa4_frame::VideoFrame;
use cfa4_time::FrameRate;

use super::{DEFAULT_STILL_DURATION, StillSource};
use crate::{MediaError, MediaInfo, MediaKind, Result, VideoStreamInfo};

/// Whether `b` starts like a JPEG XL codestream or container.
pub fn sniff(b: &[u8]) -> bool {
    cfa4_jxl::cfa4::sniff(b)
}

/// A decoded JPEG XL picture.
pub struct Decoded {
    /// Straight sRGB RGBA8, or premultiplied linear RGBA floats.
    pub frame: VideoFrame,
    /// Whether any pixel is not fully opaque.
    pub has_alpha: bool,
    /// For the media info, e.g. "RGBA 16-bit".
    pub pixel_format: String,
}

fn primaries(code: u8) -> Primaries {
    match code {
        5 => Primaries::Bt601_625,
        6 => Primaries::Bt601_525,
        9 => Primaries::Bt2020,
        12 => Primaries::P3D65,
        _ => Primaries::Bt709,
    }
}

/// Decode the first frame of a JPEG XL file.
pub fn decode(name: &str, b: &[u8]) -> Result<Decoded> {
    let p = cfa4_jxl::cfa4::decode(b).map_err(|e| MediaError::Decode(format!("{name}: {e}")))?;
    let (w, h) = (p.width, p.height);
    let ch = p.channels as usize;
    if !(1..=4).contains(&ch) || w == 0 || h == 0 {
        return Err(MediaError::Decode(format!("{name}: JPEG XL: unsupported picture layout")));
    }
    let pixels = (w as usize).saturating_mul(h as usize);
    if p.samples.len() < pixels.saturating_mul(ch) {
        return Err(MediaError::Decode(format!("{name}: JPEG XL: short sample buffer")));
    }
    // RGBA from 1–4 interleaved channels
    let rgba = |s: &[f32]| -> [f32; 4] {
        let g = |i: usize| s.get(i).copied().unwrap_or(0.0);
        match ch {
            1 => [g(0), g(0), g(0), 1.0],
            2 => [g(0), g(0), g(0), g(1)],
            3 => [g(0), g(1), g(2), 1.0],
            _ => [g(0), g(1), g(2), g(3)],
        }
    };
    let has_alpha = (ch == 2 || ch == 4) && p.samples.chunks_exact(ch).any(|s| rgba(s)[3] < 1.0);
    let layout = match (ch, has_alpha) {
        (1 | 2, false) => "Grey",
        (1 | 2, true) => "Grey + alpha",
        (_, false) => "RGB",
        (_, true) => "RGBA",
    };
    let pixel_format = format!("{layout} {}-bit", p.bits_per_sample);
    // no CICP (an ICC profile we cannot convert): show the samples as sRGB
    let [prim, trc, ..] = p.cicp.unwrap_or([1, 13, 0, 1]);
    let srgb = prim == 1 && trc == 13;
    let frame = if srgb && p.bits_per_sample <= 8 {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let mut out = Vec::with_capacity(pixels.saturating_mul(4));
        for s in p.samples.chunks_exact(ch).take(pixels) {
            out.extend(rgba(s).map(q));
        }
        VideoFrame::rgba8(w, h, out)
    } else {
        let transfer = Transfer::from_code(trc).unwrap_or(Transfer::Srgb);
        let mut out = Vec::with_capacity(pixels.saturating_mul(4));
        for s in p.samples.chunks_exact(ch).take(pixels) {
            let [r, g, bl, a] = rgba(s);
            let a = a.clamp(0.0, 1.0);
            out.extend([r, g, bl].map(|v| cfa4_color::to_linear(v, transfer) * a));
            out.push(a);
        }
        let mut frame = VideoFrame::rgba_f32(w, h, out);
        frame.color = ColorInfo { transfer: Transfer::Linear, primaries: primaries(prim), ..ColorInfo::SRGB_FULL };
        frame
    };
    Ok(Decoded { frame, has_alpha, pixel_format })
}

/// Open a JPEG XL file as a still.
pub fn still(name: &str, bytes: &[u8]) -> Result<StillSource> {
    let d = decode(name, bytes)?;
    let info = MediaInfo {
        name: name.to_string(),
        kind: MediaKind::Still,
        duration: DEFAULT_STILL_DURATION,
        video: Some(VideoStreamInfo {
            width: d.frame.width,
            height: d.frame.height,
            frame_rate: FrameRate::FPS_29_97,
            par: (1, 1),
            codec: "JPEG XL".into(),
            pixel_format: d.pixel_format,
            color: d.frame.color,
            has_alpha: d.has_alpha,
            bitrate: None,
            hdr: None,
        }),
        audio_streams: Vec::new(),
        container: "Image".into(),
        start_timecode: None,
        file_size: Some(bytes.len() as u64),
    };
    Ok(StillSource { info, frame: Arc::new(d.frame) })
}

#[cfg(test)]
mod tests;
