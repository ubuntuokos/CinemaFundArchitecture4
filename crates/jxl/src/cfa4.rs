//! The interface the rest of CFA4 uses: decode a JPEG XL file's first frame to interleaved
//! floating-point samples. CFA4's own code (GPL-3.0-or-later).

/// A decoded JPEG XL picture.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    /// 1 (grey), 2 (grey + alpha), 3 (RGB) or 4 (RGBA).
    pub channels: u32,
    /// Interleaved samples, nominally 0.0–1.0 (HDR and wide-gamut content may exceed it), in the
    /// colour space [`Picture::cicp`] names, rows top first, already oriented.
    pub samples: Vec<f32>,
    /// Bits per sample the file declares (8 for most images).
    pub bits_per_sample: u32,
    /// The output colour as ITU-T H.273 code points (primaries, transfer, matrix, full range),
    /// when it has one.
    pub cicp: Option<[u8; 4]>,
}

/// Why a file did not decode.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("JPEG XL: {0}")]
    Decode(String),
    #[error("JPEG XL image of {0}×{1} is too large")]
    TooLarge(u32, u32),
}

/// Largest decoded picture, in samples (all channels).
pub const MAX_SAMPLES: u64 = 1 << 28;

/// Whether `bytes` starts like a JPEG XL codestream or container.
pub fn sniff(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0x0A]) || bytes.starts_with(&[0, 0, 0, 0x0C, b'J', b'X', b'L', b' ', 0x0D, 0x0A, 0x87, 0x0A])
}

/// Decode the first frame of a JPEG XL file.
///
/// The ported decoder returns errors instead of panicking; as a last-resort guard (a slice index
/// on a corrupt file the port did not anticipate) a panic is caught and reported as an error.
pub fn decode(bytes: &[u8]) -> Result<Picture, Error> {
    std::panic::catch_unwind(|| decode_unguarded(bytes)).unwrap_or_else(|_| Err(Error::Decode("internal decoder error".into())))
}

/// [`decode`] without the panic guard, for the fuzz tests that check the decoder itself.
#[doc(hidden)]
pub fn decode_unguarded(bytes: &[u8]) -> Result<Picture, Error> {
    let err = |e: Box<dyn std::error::Error + Send + Sync>| Error::Decode(e.to_string());
    #[cfg(feature = "threads")]
    let pool = crate::JxlThreadPool::rayon_global();
    #[cfg(not(feature = "threads"))]
    let pool = crate::JxlThreadPool::none();
    let image = crate::JxlImage::builder().pool(pool).read(bytes).map_err(err)?;
    let (w, h) = (image.width(), image.height());
    let pixels = u64::from(w).saturating_mul(u64::from(h));
    let channels = image.pixel_format().channels() as u64;
    if pixels.saturating_mul(channels) > MAX_SAMPLES {
        return Err(Error::TooLarge(w, h));
    }
    let bits_per_sample = image.image_header().metadata.bit_depth.bits_per_sample();
    let cicp = image.rendered_cicp();
    let render = image.render_frame(0).map_err(err)?;
    let mut stream = render.stream();
    let (width, height, channels) = (stream.width(), stream.height(), stream.channels());
    let len = u64::from(width).saturating_mul(u64::from(height)).saturating_mul(u64::from(channels));
    if len > MAX_SAMPLES {
        return Err(Error::TooLarge(width, height));
    }
    let len = usize::try_from(len).map_err(|_| Error::TooLarge(width, height))?;
    let mut samples = vec![0f32; len];
    stream.write_to_buffer(&mut samples);
    Ok(Picture { width, height, channels, samples, bits_per_sample, cicp })
}
