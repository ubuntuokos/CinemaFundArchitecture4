//! Single-frame formats and true per-channel depth, separate from sequence export settings.
use crate::{ExportError, Format, Result};
use cfa4_render::Image;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StillFormat {
    Png,
    Tiff,
    Bmp,
    Jpeg,
    Heic,
    Heif,
    /// Lossless WebP (CFA4-LOSSLESS-MEDIA-TOOLS-001 K7).
    WebP,
}
impl StillFormat {
    pub const ALL: [Self; 7] = [Self::Png, Self::Jpeg, Self::Tiff, Self::Bmp, Self::Heic, Self::Heif, Self::WebP];
    pub fn parse(name: &str) -> Result<Self> {
        match name.to_ascii_lowercase().as_str() {
            "png" => Ok(Self::Png),
            "tif" | "tiff" => Ok(Self::Tiff),
            "bmp" => Ok(Self::Bmp),
            "jpg" | "jpeg" => Ok(Self::Jpeg),
            "heic" => Ok(Self::Heic),
            "heif" | "hif" => Ok(Self::Heif),
            "webp" => Ok(Self::WebP),
            _ => Err(ExportError::Unsupported("still format must be png, jpeg, tiff, bmp, heic, heif or webp".into())),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Tiff => "TIFF",
            Self::Bmp => "BMP",
            Self::Jpeg => "JPEG",
            Self::Heic => "HEIC",
            Self::Heif => "HEIF",
            Self::WebP => "WebP",
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Tiff => "tiff",
            Self::Bmp => "bmp",
            Self::Jpeg => "jpg",
            Self::Heic => "heic",
            Self::Heif => "heif",
            Self::WebP => "webp",
        }
    }
    pub fn supports_16(self) -> bool {
        matches!(self, Self::Png | Self::Tiff | Self::Heic | Self::Heif)
    }
    /// Whether `quality` changes the output (JPEG; the others are lossless).
    pub fn has_quality(self) -> bool {
        self == Self::Jpeg
    }
}

/// JPEG quality when none is asked for.
pub const DEFAULT_QUALITY: u8 = 95;

/// How a still is written besides its format and depth (K7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StillOptions {
    /// JPEG quality, 1–100.
    pub quality: u8,
    /// Output size; `None` keeps the frame's own.
    pub size: Option<(u32, u32)>,
}

impl Default for StillOptions {
    fn default() -> Self {
        Self { quality: DEFAULT_QUALITY, size: None }
    }
}

/// The output size for a requested `width` and/or `height`: one side alone keeps the aspect ratio
/// (rounded, at least 1 pixel); both set it exactly.
pub fn fit_size(w: u32, h: u32, width: Option<u32>, height: Option<u32>) -> Option<(u32, u32)> {
    let scaled =
        |a: u32, b: u32, to: u32| -> u32 { ((u64::from(b) * u64::from(to) + u64::from(a) / 2) / u64::from(a.max(1))).clamp(1, u64::from(u32::MAX)) as u32 };
    match (width, height) {
        (None, None) => None,
        (Some(x), Some(y)) => Some((x, y)),
        (Some(x), None) => Some((x, scaled(w, h, x))),
        (None, Some(y)) => Some((scaled(h, w, y), y)),
    }
}

/// The image at another size (Lanczos on the premultiplied linear values, so edges keep no halo).
fn resized(image: &Image, (w, h): (u32, u32)) -> Result<Image> {
    let bad = || ExportError::Encode("invalid still image dimensions or pixel buffer".into());
    cfa4_project::validate_frame_size(w, h).map_err(|e| ExportError::Unsupported(format!("output size {w}×{h}: {e}")))?;
    let src: image::ImageBuffer<image::Rgba<f32>, Vec<f32>> =
        image::ImageBuffer::from_raw(u32::try_from(image.w).map_err(|_| bad())?, u32::try_from(image.h).map_err(|_| bad())?, image.px.clone())
            .ok_or_else(bad)?;
    let out = image::imageops::resize(&src, w, h, image::imageops::FilterType::Lanczos3);
    Ok(Image { w: w as usize, h: h as usize, px: out.into_raw() })
}

/// [`encode`] with a JPEG quality and an output size.
pub fn encode_with(image: &Image, format: StillFormat, depth: u8, opts: StillOptions) -> Result<Vec<u8>> {
    if !(1..=100).contains(&opts.quality) {
        return Err(ExportError::Unsupported("quality must be 1 to 100".into()));
    }
    match opts.size {
        Some(size) if size != (image.w as u32, image.h as u32) => encode_at(&resized(image, size)?, format, depth, opts.quality),
        _ => encode_at(image, format, depth, opts.quality),
    }
}

/// Quantize the original float image directly, never pad an 8-bit image to 16 bits.
pub fn encode(image: &Image, format: StillFormat, depth: u8) -> Result<Vec<u8>> {
    encode_at(image, format, depth, DEFAULT_QUALITY)
}

fn encode_at(image: &Image, format: StillFormat, depth: u8, quality: u8) -> Result<Vec<u8>> {
    if !matches!(depth, 8 | 16) || (depth == 16 && !format.supports_16()) {
        return Err(ExportError::Unsupported("16-bit stills require PNG, TIFF, HEIC or HEIF; JPEG, BMP and WebP support 8 bits per channel".into()));
    }
    let bad = || ExportError::Encode("invalid still image dimensions or pixel buffer".into());
    let w = u32::try_from(image.w).map_err(|_| bad())?;
    let h = u32::try_from(image.h).map_err(|_| bad())?;
    cfa4_project::validate_frame_size(w, h).map_err(|_| bad())?;
    let samples = image.w.checked_mul(image.h).and_then(|n| n.checked_mul(4)).ok_or_else(bad)?;
    if image.px.len() != samples {
        return Err(bad());
    }
    let err = |e: image::ImageError| ExportError::Encode(e.to_string());
    // HEIC/HEIF Export Frame uses the local libheif HEVC encoder through
    // an intermediate PNG. This does not add a video/sequence preset.
    if matches!(format, StillFormat::Heic | StillFormat::Heif) {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let png = encode_at(image, StillFormat::Png, depth, quality)?;
            return cfa4_media::still::heif::encode_png(&png).map_err(|e| ExportError::Encode(e.to_string()));
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        return Err(ExportError::Unsupported("HEIC/HEIF export is supported on Linux AMD64 only".into()));
    }
    if depth == 8 {
        let rgba = image.to_rgba8();
        if format == StillFormat::Jpeg {
            let rgb: Vec<u8> = rgba
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| {
                    let [r, g, b, _] = *p;
                    [r, g, b]
                })
                .collect();
            let mut out = Vec::new();
            image::ImageEncoder::write_image(
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100)),
                &rgb,
                w,
                h,
                image::ExtendedColorType::Rgb8,
            )
            .map_err(err)?;
            return Ok(out);
        }
        if format == StillFormat::WebP {
            let mut out = Vec::new();
            image::ImageEncoder::write_image(image::codecs::webp::WebPEncoder::new_lossless(&mut out), &rgba, w, h, image::ExtendedColorType::Rgba8)
                .map_err(err)?;
            return Ok(out);
        }
        let f = match format {
            StillFormat::Png => Format::PngSequence,
            StillFormat::Tiff => Format::TiffSequence,
            _ => Format::BmpSequence,
        };
        // 8-bit TIFF and BMP stay RGB, as Export Frame wrote them before; PNG and 16-bit output keep alpha.
        return crate::encode_still(f, rgba, w, h, format == StillFormat::Png);
    }
    let capacity = samples.checked_mul(2).ok_or_else(bad)?;
    let mut bytes = Vec::with_capacity(capacity);
    for pixel in image.px.as_chunks::<4>().0 {
        let [r, g, b, a] = *pixel;
        let a = if a.is_finite() { a.clamp(0.0, 1.0) } else { 0.0 };
        let encode_channel = |v: f32| {
            let v = if a > 0.0 && v.is_finite() { cfa4_color::linear_to_srgb(v / a).clamp(0.0, 1.0) } else { 0.0 };
            (v * 65535.0 + 0.5) as u16
        };
        for v in [encode_channel(r), encode_channel(g), encode_channel(b), (a * 65535.0 + 0.5) as u16] {
            bytes.extend_from_slice(&v.to_ne_bytes());
        }
    }
    let mut out = std::io::Cursor::new(Vec::new());
    match format {
        StillFormat::Png => {
            image::ImageEncoder::write_image(image::codecs::png::PngEncoder::new(&mut out), &bytes, w, h, image::ExtendedColorType::Rgba16).map_err(err)?
        }
        StillFormat::Tiff => {
            image::ImageEncoder::write_image(image::codecs::tiff::TiffEncoder::new(&mut out), &bytes, w, h, image::ExtendedColorType::Rgba16).map_err(err)?
        }
        _ => return Err(ExportError::Unsupported("16-bit still format".into())),
    }
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sixteen_bit_stills_preserve_sub_eight_bit_precision_and_alpha() {
        let img = Image { w: 2, h: 1, px: vec![0.101, 0.203, 0.307, 1.0, 0.101 / 2.0, 0.203 / 2.0, 0.307 / 2.0, 0.5] };
        for format in [StillFormat::Png, StillFormat::Tiff] {
            let bytes = encode(&img, format, 16).unwrap();
            let decoded = image::load_from_memory(&bytes).unwrap();
            assert_eq!(decoded.color(), image::ColorType::Rgba16);
            let px = decoded.to_rgba16();
            let a = px.get_pixel(0, 0).0;
            let b = px.get_pixel(1, 0).0;
            assert_ne!(a[0] % 257, 0, "must not be padded 8-bit values");
            assert_eq!(&a[..3], &b[..3]);
            assert_eq!(b[3], 32768);
        }
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn heic_heif_single_frame_exports_round_trip() {
        if !cfa4_testkit::oracle::on_path_or_skip(
            "heic_heif_single_frame_exports_round_trip",
            &["heif-enc", "heif-dec"],
            "Ubuntu packages libheif-examples, libheif-plugin-x265, libheif-plugin-libde265",
        ) {
            return;
        }
        let source = Image::filled(64, 64, [0.25, 0.4, 0.6, 1.0]);
        for format in [StillFormat::Heic, StillFormat::Heif] {
            assert_eq!(StillFormat::parse(format.extension()).unwrap(), format);
            for depth in [8, 16] {
                let encoded = encode(&source, format, depth).expect("local heif-enc and x265 plugin required");
                assert!(cfa4_media::still::heif::sniff(&encoded));
                let decoded = cfa4_media::still::heif::decode("export.heic", &encoded).expect("libheif HEIC output must decode");
                assert_eq!((decoded.width, decoded.height), (64, 64));
            }
        }
    }

    #[test]
    fn webp_is_lossless_jpeg_quality_and_the_size_are_chosen() {
        let mut image = Image::filled(8, 4, [0.2, 0.3, 0.4, 1.0]);
        image.px[0] = 0.9;
        let webp = encode(&image, StillFormat::WebP, 8).unwrap();
        let back = image::load_from_memory(&webp).unwrap().to_rgba8();
        assert_eq!(back.into_raw(), image.to_rgba8(), "WebP is written without loss");
        assert!(encode(&image, StillFormat::WebP, 16).is_err());
        let big = Image::filled(64, 64, [0.5, 0.2, 0.7, 1.0]);
        let q = |quality| encode_with(&big, StillFormat::Jpeg, 8, StillOptions { quality, size: None }).unwrap().len();
        assert!(q(20) < q(100), "a lower quality writes a smaller file");
        let small = encode_with(&big, StillFormat::Png, 8, StillOptions { size: Some((16, 8)), ..Default::default() }).unwrap();
        assert_eq!(image::load_from_memory(&small).unwrap().dimensions_tuple(), (16, 8));
        assert!(encode_with(&big, StillFormat::Jpeg, 8, StillOptions { quality: 0, size: None }).is_err());
        assert!(encode_with(&big, StillFormat::Png, 8, StillOptions { size: Some((0, 8)), ..Default::default() }).is_err());
        assert_eq!(fit_size(1920, 1080, Some(640), None), Some((640, 360)));
        assert_eq!(fit_size(1920, 1080, None, Some(1)), Some((2, 1)));
        assert_eq!(fit_size(1920, 1080, None, None), None);
        assert_eq!(StillFormat::parse("WEBP").unwrap(), StillFormat::WebP);
    }

    trait Dims {
        fn dimensions_tuple(&self) -> (u32, u32);
    }
    impl Dims for image::DynamicImage {
        fn dimensions_tuple(&self) -> (u32, u32) {
            (self.width(), self.height())
        }
    }

    #[test]
    fn jpeg_and_bmp_are_eight_bit_and_invalid_buffers_are_errors() {
        let image = Image::filled(2, 2, [0.2, 0.3, 0.4, 1.0]);
        for format in [StillFormat::Jpeg, StillFormat::Bmp] {
            let bytes = encode(&image, format, 8).unwrap();
            assert_eq!(image::load_from_memory(&bytes).unwrap().color(), image::ColorType::Rgb8);
            assert!(encode(&image, format, 16).is_err());
        }
        assert!(encode(&Image { w: 2, h: 2, px: vec![] }, StillFormat::Png, 16).is_err());
        assert!(encode(&image, StillFormat::Png, 12).is_err());
    }
}
