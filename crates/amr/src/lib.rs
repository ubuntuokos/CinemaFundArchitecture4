//! Pure-Rust AMR speech decoders.
//!
//! - [`nb`]: AMR narrowband (3GPP TS 26.071, 8 kHz), bit-exact with opencore-amrnb, the
//!   Apache-2.0 fork of the 3GPP fixed-point reference that ffmpeg and most players use.
//!
//! Frames come as in RFC 4867 storage files (`#!AMR\n`) and in the `samr` (3GPP / MP4) and
//! Matroska `A_AMR/NB` tracks: a table-of-contents byte followed by the class-ordered payload.

mod basic;
pub mod nb;

/// Decoding errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("truncated {0}")]
    Truncated(&'static str),
    #[error("invalid {0}")]
    Invalid(&'static str),
}

/// The RFC 4867 storage-file magic of AMR narrowband.
pub const NB_MAGIC: &[u8] = b"#!AMR\n";

/// Decodes a whole AMR-NB storage file (`#!AMR\n`, single channel) to 8 kHz samples.
pub fn decode_nb_file(data: &[u8]) -> Result<Vec<i16>, Error> {
    let body = data.strip_prefix(NB_MAGIC).ok_or(Error::Invalid("AMR-NB storage magic"))?;
    let mut dec = nb::Decoder::new();
    let mut out = Vec::new();
    let mut pos = 0usize;
    let mut frame = [0i16; nb::FRAME_SAMPLES];
    while let Some(&toc) = body.get(pos) {
        let size = nb::Decoder::frame_size(toc);
        let end = pos.saturating_add(size);
        let f = body.get(pos..end).ok_or(Error::Truncated("AMR-NB frame"))?;
        dec.decode_frame(f, &mut frame)?;
        out.extend_from_slice(&frame);
        pos = end;
    }
    Ok(out)
}
