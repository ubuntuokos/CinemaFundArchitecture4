//! HDR gain map JPEGs (Adobe Gain Map / Google Ultra HDR, ISO 21496-1) and Multi-Picture Format
//! files (`.mpo`, CIPA DC-007). A gain map JPEG holds an SDR base picture and, as a second JPEG
//! listed in the MPF index, a gain map; applying the map to the linear base gives the HDR
//! ("alternate") rendition. CFA4 opens that HDR rendition at its full headroom as scene-linear
//! Rec.709 floats (F2g decision C1), so values above 1.0 carry the highlights; an SDR sequence
//! tone-maps it like any other HDR still.
//!
//! The parameters come from the gain map image's ISO 21496-1 block ([`iso`], preferred) or its XMP
//! `hdrgm` properties ([`xmp`]). A file whose gain map cannot be read opens as its SDR base, with a
//! warning in the log. An MPF file without a gain map (a stereo `.mpo`) opens its first picture.
//!
//! Application (Adobe Gain Map Specification 1.0, "Applying the gain map"; ISO 21496-1 clause 6):
//! for a gain map sample `g` in 0..1 of channel `c`, `g' = g^(1/gamma)`,
//! `log2 boost = min*(1-g') + max*g'`, and `hdr = (base + base_offset) * 2^(log2 boost * w) - alt_offset`,
//! with the weight `w` = 1 for the full alternate rendition. The map is upsampled bilinearly to the
//! base picture's size.

use std::sync::Arc;

use cfa4_color::{ColorInfo, Transfer};
use cfa4_frame::VideoFrame;
use cfa4_time::FrameRate;

use super::{DEFAULT_STILL_DURATION, StillSource};
use crate::{MediaError, MediaInfo, MediaKind, Result, VideoStreamInfo};

pub mod iso;
pub mod jpeg;
pub mod xmp;

/// The parameters of one gain map channel. `min` and `max` are log2 boosts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Channel {
    pub min: f32,
    pub max: f32,
    pub gamma: f32,
    pub base_offset: f32,
    pub alternate_offset: f32,
}

impl Default for Channel {
    fn default() -> Self {
        Self { min: 0.0, max: 1.0, gamma: 1.0, base_offset: 1.0 / 64.0, alternate_offset: 1.0 / 64.0 }
    }
}

/// Where the parameters were read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Iso,
    Xmp,
}

/// Gain map parameters, normalised from either metadata form.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GainMapParams {
    pub channels: [Channel; 3],
    /// HDR headroom of the base and the alternate rendition (log2 of their peak over SDR white).
    pub base_headroom: f32,
    pub alternate_headroom: f32,
    /// Whether the map applies in the base picture's colour space (else the alternate's).
    pub use_base_color_space: bool,
    /// Whether the base is the HDR rendition and the map leads to SDR.
    pub backward: bool,
    pub origin: Origin,
}

/// Largest log2 boost we accept (2^16 over SDR white is far beyond any display).
const MAX_LOG2: f32 = 16.0;

impl GainMapParams {
    /// Parameters that leave the base unchanged.
    #[cfg(test)]
    pub fn identity() -> Self {
        Self {
            channels: [Channel::default(); 3],
            base_headroom: 0.0,
            alternate_headroom: 1.0,
            use_base_color_space: true,
            backward: false,
            origin: Origin::Iso,
        }
    }

    /// Refuse values that would make the arithmetic meaningless.
    pub fn validate(&self) -> std::result::Result<(), String> {
        let finite = |v: f32| v.is_finite() && v.abs() <= MAX_LOG2;
        if !finite(self.base_headroom) || !finite(self.alternate_headroom) || self.base_headroom < 0.0 || self.alternate_headroom < 0.0 {
            return Err("HDR headroom out of range".into());
        }
        for c in &self.channels {
            if !finite(c.min) || !finite(c.max) || c.max < c.min {
                return Err("gain map minimum/maximum out of range".into());
            }
            if !(c.gamma.is_finite() && c.gamma > 0.0 && c.gamma <= 64.0) {
                return Err("gain map gamma out of range".into());
            }
            if !(c.base_offset.is_finite() && c.alternate_offset.is_finite() && c.base_offset.abs() <= 1.0 && c.alternate_offset.abs() <= 1.0) {
                return Err("gain map offsets out of range".into());
            }
        }
        Ok(())
    }
}

/// The picture of a gain map file, decoded.
pub struct Decoded {
    pub frame: VideoFrame,
    pub params: GainMapParams,
}

/// What a JPEG file with an MPF index or gain map metadata turned out to be.
pub enum Probe {
    /// A gain map picture, decoded as its HDR rendition.
    GainMap(Decoded),
    /// An MPF file without a usable gain map; `stereo` when the MP index marks a disparity
    /// (stereo pair) image set.
    MultiPicture { stereo: bool },
    /// A plain JPEG.
    Plain,
}

fn err(name: &str, what: &str) -> MediaError {
    MediaError::Decode(format!("{name}: gain map: {what}"))
}

/// The gain map JPEG inside `b` and its parameters, when `b` is a gain map file. `Err` explains why
/// a file that announces a gain map has none we can use.
fn find_gain_map<'a>(b: &'a [u8], primary: &jpeg::Segments<'_>) -> std::result::Result<Option<(&'a [u8], GainMapParams)>, String> {
    let primary_xmp = primary.xmp.map(xmp::parse).transpose().unwrap_or_else(|e| {
        log::warn!("gain map: primary XMP unreadable: {e}");
        None
    });
    let announced_iso = primary.iso.is_some();
    let announced_xmp = primary_xmp.as_ref().is_some_and(|i| i.has_version);

    // candidate secondary pictures: the MPF index, else what follows the primary picture
    let mut candidates: Vec<&[u8]> = Vec::new();
    if let Some((base, mpf)) = primary.mpf
        && let Some(entries) = jpeg::mp_entries(base, mpf, b.len())
    {
        candidates.extend(entries.iter().skip(1).filter(|e| e.is_jpeg()).filter_map(|e| b.get(e.offset..e.offset + e.size)));
    }
    if candidates.is_empty()
        && (announced_iso || announced_xmp)
        && let Some(end) = primary.sos.and_then(|s| jpeg::picture_end(b, s))
        && let Some(rest) = b.get(end..)
        && let Some(start) = rest.windows(3).position(|w| w == [0xFF, 0xD8, 0xFF])
    {
        let len = primary_xmp.as_ref().and_then(|i| i.gain_map_length).unwrap_or(rest.len() - start);
        if let Some(pic) = rest.get(start..start + len) {
            candidates.push(pic);
        }
    }

    let mut why = None;
    for pic in candidates {
        let s = jpeg::segments(pic);
        // ISO 21496-1 first, then the XMP form
        if let Some(block) = s.iso {
            match iso::parse(block) {
                Ok(iso::Block::Params(p)) => return Ok(Some((pic, p))),
                Ok(iso::Block::VersionOnly) => {}
                Err(e) => why = Some(format!("ISO 21496-1 metadata: {e}")),
            }
        }
        if let Some(x) = s.xmp {
            match xmp::parse(x) {
                Ok(xmp::Info { params: Some(p), .. }) => return Ok(Some((pic, p))),
                Ok(_) => {}
                Err(e) => why = Some(format!("hdrgm metadata: {e}")),
            }
        }
    }
    match why {
        Some(w) => Err(w),
        None if announced_iso || announced_xmp => Err("the file announces a gain map but holds no readable one".into()),
        None => Ok(None),
    }
}

/// A decoded gain map: one or three channels of 8-bit samples.
struct Map {
    width: usize,
    height: usize,
    channels: usize,
    data: Vec<u8>,
}

impl Map {
    /// Bilinear sample of channel `c` at the centre of base pixel (`x`, `y`) of a `w` x `h` picture,
    /// normalised to 0..1.
    fn sample(&self, x: usize, y: usize, w: usize, h: usize, c: usize) -> f32 {
        let fx = ((x as f32 + 0.5) * self.width as f32 / w as f32 - 0.5).clamp(0.0, (self.width - 1) as f32);
        let fy = ((y as f32 + 0.5) * self.height as f32 / h as f32 - 0.5).clamp(0.0, (self.height - 1) as f32);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let c = c.min(self.channels - 1);
        let at = |x: usize, y: usize| f32::from(self.data[(y * self.width + x) * self.channels + c]);
        let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
        let bottom = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
        (top * (1.0 - ty) + bottom * ty) / 255.0
    }
}

fn decode_map(name: &str, pic: &[u8]) -> Result<Map> {
    let img = image::load_from_memory_with_format(pic, image::ImageFormat::Jpeg).map_err(|e| err(name, &format!("gain map picture: {e}")))?;
    let (width, height) = (img.width() as usize, img.height() as usize);
    if width == 0 || height == 0 {
        return Err(err(name, "empty gain map picture"));
    }
    Ok(if img.color().channel_count() == 1 {
        Map { width, height, channels: 1, data: img.into_luma8().into_raw() }
    } else {
        Map { width, height, channels: 3, data: img.into_rgb8().into_raw() }
    })
}

/// Apply `p` with `map` to the sRGB base picture `rgba` (`w` x `h`), giving linear RGBA floats.
fn apply(rgba: &[u8], w: usize, h: usize, map: &Map, p: &GainMapParams) -> Vec<f32> {
    let lut: Vec<f32> = (0..256).map(|v| cfa4_color::to_linear(v as f32 / 255.0, Transfer::Srgb)).collect();
    let weight = if p.alternate_headroom >= p.base_headroom { 1.0 } else { -1.0 } * if p.backward { 0.0 } else { 1.0 };
    let mut out = vec![0f32; w * h * 4];
    out.chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let i = y * w + x;
            for (c, ch) in p.channels.iter().enumerate() {
                let base = lut[usize::from(rgba[i * 4 + c])];
                let mut g = map.sample(x, y, w, h, c);
                if ch.gamma != 1.0 {
                    g = g.powf(1.0 / ch.gamma);
                }
                let boost = ch.min * (1.0 - g) + ch.max * g;
                row[x * 4 + c] = (base + ch.base_offset) * (boost * weight).exp2() - ch.alternate_offset;
            }
            row[x * 4 + 3] = 1.0;
        }
    });
    out
}

/// Look at a JPEG file: decode its HDR rendition when it is a gain map picture.
pub fn probe(name: &str, b: &[u8]) -> Result<Probe> {
    let s = jpeg::segments(b);
    if s.mpf.is_none() && s.iso.is_none() && s.xmp.is_none() {
        return Ok(Probe::Plain);
    }
    let found = match find_gain_map(b, &s) {
        Ok(f) => f,
        Err(why) => {
            log::warn!("{name}: {why}; opening the SDR base picture");
            None
        }
    };
    let Some((pic, params)) = found else {
        let stereo = s
            .mpf
            .and_then(|(base, mpf)| jpeg::mp_entries(base, mpf, b.len()))
            .is_some_and(|e| e.len() >= 2 && e.iter().any(|e| e.type_code() == jpeg::MP_TYPE_DISPARITY));
        return Ok(if s.mpf.is_some() { Probe::MultiPicture { stereo } } else { Probe::Plain });
    };
    if params.backward {
        log::warn!("{name}: the base picture is the HDR rendition; opening it as stored");
        return Ok(Probe::Plain);
    }
    if !params.use_base_color_space {
        log::info!("{name}: the gain map applies in the alternate colour space; applying it in the base's (Rec.709)");
    }
    let base = image::load_from_memory_with_format(b, image::ImageFormat::Jpeg).map_err(|e| err(name, &e.to_string()))?.into_rgba8();
    let (w, h) = (base.width() as usize, base.height() as usize);
    let map = match decode_map(name, pic) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("{e}; opening the SDR base picture");
            return Ok(Probe::Plain);
        }
    };
    let px = apply(base.as_raw(), w, h, &map, &params);
    let mut frame = VideoFrame::rgba_f32(w as u32, h as u32, px);
    frame.color = LINEAR_709;
    Ok(Probe::GainMap(Decoded { frame, params }))
}

/// The colour of decoded gain map pictures: scene-linear light, Rec.709 primaries.
const LINEAR_709: ColorInfo = ColorInfo { transfer: Transfer::Linear, ..ColorInfo::SRGB_FULL };

/// Open a JPEG as a still when it is a gain map or MPF file (`None`: a plain JPEG for the usual path).
pub fn still(name: &str, b: &[u8]) -> Result<Option<StillSource>> {
    let (frame, codec, pixel_format, color) = match probe(name, b)? {
        Probe::Plain => return Ok(None),
        Probe::MultiPicture { stereo } => {
            let img = image::load_from_memory_with_format(b, image::ImageFormat::Jpeg).map_err(|e| err(name, &e.to_string()))?.into_rgba8();
            let (w, h) = img.dimensions();
            let what = if stereo { "RGB 8-bit (left picture of a stereo pair)" } else { "RGB 8-bit (first picture)" };
            (VideoFrame::rgba8(w, h, img.into_raw()), "MPO", what.to_string(), ColorInfo::SRGB_FULL)
        }
        Probe::GainMap(d) => {
            let stops = d.params.alternate_headroom - d.params.base_headroom;
            let form = match d.params.origin {
                Origin::Iso => "ISO 21496-1",
                Origin::Xmp => "Ultra HDR",
            };
            (d.frame, "JPEG", format!("RGB 32-bit float ({form} gain map, headroom {stops:.1} stops)"), LINEAR_709)
        }
    };
    let info = MediaInfo {
        name: name.to_string(),
        kind: MediaKind::Still,
        duration: DEFAULT_STILL_DURATION,
        video: Some(VideoStreamInfo {
            width: frame.width,
            height: frame.height,
            frame_rate: FrameRate::FPS_29_97,
            par: (1, 1),
            codec: codec.into(),
            pixel_format,
            color,
            has_alpha: false,
            bitrate: None,
            hdr: None,
        }),
        audio_streams: Vec::new(),
        container: "Image".into(),
        start_timecode: None,
        file_size: Some(b.len() as u64),
    };
    Ok(Some(StillSource { info, frame: Arc::new(frame) }))
}

#[cfg(test)]
mod tests;
