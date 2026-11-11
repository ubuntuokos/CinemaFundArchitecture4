//! LCEVC enhancement layers (MPEG-5 Part 2) in imported files: found by `cfa4-lcevc`, not
//! decoded (step LCEVC-1 of the DV/DVCPRO + LCEVC plan; decoding waits on the patent question).
//! A file that carries one plays its base layer; the stream's codec label and the log say so.

/// Samples scanned at open for an LCEVC T.35 SEI (the enhancement rides in every picture, so
/// the first few decide).
pub(crate) const SCAN_SAMPLES: usize = 8;

/// What the codec label gains when the file carries LCEVC.
pub const LABEL_SUFFIX: &str = " + LCEVC (enhancement not decoded)";

/// Mark a stream as carrying LCEVC: label and a warning in the log.
pub(crate) fn mark(name: &str, codec: &mut String, how: &str) {
    if !codec.ends_with(LABEL_SUFFIX) {
        codec.push_str(LABEL_SUFFIX);
    }
    log::warn!("{name}: LCEVC enhancement layer ({how}) found; CFA4 does not decode LCEVC yet, the base layer plays");
}

/// The base codec and NAL framing of an MP4 / Matroska sample entry, for the SEI scan.
pub(crate) fn base_of(codec: &cfa4_isobmff::CodecConfig) -> Option<(cfa4_lcevc::BaseCodec, cfa4_lcevc::Framing)> {
    match codec {
        cfa4_isobmff::CodecConfig::Avc(c) => Some((cfa4_lcevc::BaseCodec::H264, cfa4_lcevc::Framing::LengthPrefixed(c.length_size))),
        cfa4_isobmff::CodecConfig::Hevc(c) => Some((cfa4_lcevc::BaseCodec::Hevc, cfa4_lcevc::Framing::LengthPrefixed(c.length_size))),
        _ => None,
    }
}
