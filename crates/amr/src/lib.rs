//! Pure-Rust AMR speech decoders.
//!
//! - [`nb`]: AMR narrowband (3GPP TS 26.071, 8 kHz), bit-exact with opencore-amrnb, the
//!   Apache-2.0 fork of the 3GPP fixed-point reference that ffmpeg and most players use.
//! - [`wb`]: AMR wideband (3GPP TS 26.171, G.722.2, 16 kHz), bit-exact with opencore-amrwb.
//!
//! Frames come as in RFC 4867 storage files (`#!AMR\n`, `#!AMR-WB\n`) and in the `samr` /
//! `sawb` (3GPP / MP4) and Matroska `A_AMR/NB` / `A_AMR/WB` tracks: a table-of-contents byte
//! followed by the class-ordered payload.

mod basic;
pub mod nb;
pub mod wb;

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

/// The RFC 4867 storage-file magic of AMR wideband.
pub const WB_MAGIC: &[u8] = b"#!AMR-WB\n";

/// An AMR-NB or AMR-WB decoder for container packets: each packet holds one or more frames in
/// storage format (a table-of-contents byte, then the payload), as in 3GP / MP4 `samr` / `sawb`
/// samples and Matroska tracks.
#[derive(Clone, Debug)]
pub enum Decoder {
    Nb(Box<nb::Decoder>),
    Wb(Box<wb::Decoder>),
}

impl Decoder {
    /// A narrowband (`wide == false`) or wideband decoder.
    pub fn new(wide: bool) -> Self {
        if wide { Decoder::Wb(Box::default()) } else { Decoder::Nb(Box::default()) }
    }

    /// Whether this is the wideband decoder.
    pub fn is_wide(&self) -> bool {
        matches!(self, Decoder::Wb(_))
    }

    /// Output sample rate: 8000 or 16000 Hz.
    pub fn sample_rate(&self) -> u32 {
        if self.is_wide() { wb::SAMPLE_RATE } else { nb::SAMPLE_RATE }
    }

    /// Samples per frame: 160 or 320.
    pub fn frame_samples(&self) -> usize {
        if self.is_wide() { wb::FRAME_SAMPLES } else { nb::FRAME_SAMPLES }
    }

    /// Bytes of the frame whose table-of-contents byte is `toc`, that byte included.
    pub fn frame_size(&self, toc: u8) -> usize {
        if self.is_wide() { wb::Decoder::frame_size(toc) } else { nb::Decoder::frame_size(toc) }
    }

    /// Back to the initial state (after a seek).
    pub fn reset(&mut self) {
        *self = Decoder::new(self.is_wide());
    }

    /// Decodes the frames of one packet, appending their samples to `out`. A truncated last
    /// frame is an error after the whole frames before it are decoded.
    pub fn decode_packet(&mut self, data: &[u8], out: &mut Vec<i16>) -> Result<(), Error> {
        let mut pos = 0usize;
        while let Some(&toc) = data.get(pos) {
            let end = pos.saturating_add(self.frame_size(toc));
            let f = data.get(pos..end).ok_or(Error::Truncated("AMR frame"))?;
            match self {
                Decoder::Nb(d) => {
                    let mut frame = [0i16; nb::FRAME_SAMPLES];
                    d.decode_frame(f, &mut frame)?;
                    out.extend_from_slice(&frame);
                }
                Decoder::Wb(d) => {
                    let mut frame = [0i16; wb::FRAME_SAMPLES];
                    d.decode_frame(f, &mut frame)?;
                    out.extend_from_slice(&frame);
                }
            }
            pos = end;
        }
        Ok(())
    }
}

/// Decodes a whole AMR-NB storage file (`#!AMR\n`, single channel) to 8 kHz samples.
pub fn decode_nb_file(data: &[u8]) -> Result<Vec<i16>, Error> {
    let body = data.strip_prefix(NB_MAGIC).ok_or(Error::Invalid("AMR-NB storage magic"))?;
    let mut out = Vec::new();
    Decoder::new(false).decode_packet(body, &mut out)?;
    Ok(out)
}

/// Decodes a whole AMR-WB storage file (`#!AMR-WB\n`, single channel) to 16 kHz samples.
pub fn decode_wb_file(data: &[u8]) -> Result<Vec<i16>, Error> {
    let body = data.strip_prefix(WB_MAGIC).ok_or(Error::Invalid("AMR-WB storage magic"))?;
    let mut out = Vec::new();
    Decoder::new(true).decode_packet(body, &mut out)?;
    Ok(out)
}

/// Decodes a whole storage file of either kind (told apart by the magic): the samples and the
/// sample rate.
pub fn decode_file(data: &[u8]) -> Result<(Vec<i16>, u32), Error> {
    if data.starts_with(WB_MAGIC) { Ok((decode_wb_file(data)?, wb::SAMPLE_RATE)) } else { Ok((decode_nb_file(data)?, nb::SAMPLE_RATE)) }
}
