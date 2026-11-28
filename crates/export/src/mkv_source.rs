//! Matroska / WebM sources for Smart Cut (R2-03 of `CFA4-SMART-CUT-RENDER-R2`).
//!
//! The video track of a Matroska file is indexed the way WP-02 indexes an MP4 track. The
//! `CodecPrivate` of H.264, HEVC and AV1 is the ISOBMFF configuration record (`avcC`, `hvcC`,
//! `av1C`), so it is parsed with `cfa4-isobmff`. MPEG-2 video (`V_MPEG2`) carries its sequence
//! headers in the frames. Matroska stores presentation times only. The frames come in decode
//! order, so the index needs no decode times.

use std::fmt;

use cfa4_isobmff::{Av1Config, AvcConfig, CodecConfig, FourCc, HevcConfig};
use cfa4_matroska::{ByteSource, Codec as MkvCodec, MkvFile, TrackKind};
use cfa4_smart_edit::av1_index::index_av1;
use cfa4_smart_edit::capabilities::{Codec, Container};
use cfa4_smart_edit::h264_index::{NalFormat, index_h264};
use cfa4_smart_edit::hevc_index::index_hevc;
use cfa4_smart_edit::media_index::{Packet, StreamIndex};
use cfa4_smart_edit::mpeg2_index::index_mpeg2;
use cfa4_smart_edit::timestamp::Rational;

/// The indexed video track of a Matroska source.
#[derive(Clone, Debug)]
pub struct MkvVideo {
    /// Index into [`MkvFile::tracks`].
    pub track: usize,
    pub codec: Codec,
    /// The codec configuration as an ISOBMFF sample entry would carry it. MPEG-2 has no record
    /// and keeps its `CodecPrivate` (the sequence header).
    pub config: CodecConfig,
    pub index: StreamIndex,
}

/// Why a Matroska source cannot be indexed for Smart Cut.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MkvSourceError {
    /// The file was opened without its sample tables.
    NotIndexed,
    NoVideo,
    /// The video codec is not one Smart Cut copies.
    Unsupported {
        codec: String,
    },
    /// The frames are compressed or encrypted, so they cannot be read or copied as stored.
    Encoded,
    /// The `CodecPrivate` configuration record does not parse.
    Config {
        detail: String,
    },
    /// A frame cannot be read.
    Read {
        sample: usize,
        detail: String,
    },
    /// The bitstream does not index.
    Index {
        detail: String,
    },
}

impl fmt::Display for MkvSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MkvSourceError::NotIndexed => write!(f, "the Matroska file was opened without its frame tables"),
            MkvSourceError::NoVideo => write!(f, "the Matroska file has no video track"),
            MkvSourceError::Unsupported { codec } => write!(f, "Smart Cut copies H.264, HEVC, AV1 and MPEG-2 video from Matroska, not {codec}"),
            MkvSourceError::Encoded => write!(f, "the video frames are compressed or encrypted in the file and cannot be copied"),
            MkvSourceError::Config { detail } => write!(f, "the codec configuration (CodecPrivate) does not parse: {detail}"),
            MkvSourceError::Read { sample, detail } => write!(f, "frame {sample} cannot be read: {detail}"),
            MkvSourceError::Index { detail } => write!(f, "indexing failed: {detail}"),
        }
    }
}

impl std::error::Error for MkvSourceError {}

/// The Smart Cut container of a Matroska file: WebM by its `DocType`, Matroska otherwise.
pub fn container(file: &MkvFile) -> Container {
    if file.is_webm() { Container::WebM } else { Container::Matroska }
}

/// Index the first video track of `file` (opened with its sample tables) out of `bytes`.
pub fn index_mkv_video<S: ByteSource + ?Sized>(file: &MkvFile, bytes: &S) -> Result<MkvVideo, MkvSourceError> {
    if !file.indexed {
        return Err(MkvSourceError::NotIndexed);
    }
    let track = file.tracks.iter().position(|t| t.kind == TrackKind::Video).ok_or(MkvSourceError::NoVideo)?;
    let t = &file.tracks[track];
    let config_err = |e: &dyn fmt::Debug| MkvSourceError::Config { detail: format!("{e:?}") };
    let (codec, config) = match &t.codec {
        MkvCodec::Avc { avcc } => (Codec::H264, CodecConfig::Avc(AvcConfig::parse(avcc).map_err(|e| config_err(&e))?)),
        MkvCodec::Hevc { hvcc } => (Codec::Hevc, CodecConfig::Hevc(HevcConfig::parse(hvcc).map_err(|e| config_err(&e))?)),
        MkvCodec::Av1 { av1c } => (Codec::Av1, CodecConfig::Av1(Av1Config::parse(av1c).map_err(|e| config_err(&e))?)),
        MkvCodec::Other(id) if id == "V_MPEG2" => (Codec::Mpeg2Video, CodecConfig::Unknown { fourcc: FourCc(*b"mp2v"), raw: t.codec_private.clone() }),
        _ => return Err(MkvSourceError::Unsupported { codec: t.codec_id.clone() }),
    };
    if !t.frames_readable() {
        return Err(MkvSourceError::Encoded);
    }
    let mut data = Vec::with_capacity(t.samples.len());
    for i in 0..t.samples.len() {
        data.push(file.read_sample(bytes, track, i).map_err(|e| MkvSourceError::Read { sample: i, detail: e.to_string() })?);
    }
    let packets = t.samples.iter().zip(&data).map(|(s, d)| Packet { data: d, pts: Some(s.pts), dts: None, keyframe: s.keyframe });
    let id = u32::try_from(t.number).map_err(|_| MkvSourceError::Index { detail: format!("track number {} is too large", t.number) })?;
    let (num, den) = (i64::try_from(t.timebase.0), i64::try_from(t.timebase.1));
    let (Ok(num), Ok(den)) = (num, den) else { return Err(MkvSourceError::Index { detail: format!("timebase {:?}", t.timebase) }) };
    let tb = Rational::new(num, den);
    let index = match &config {
        CodecConfig::Avc(avc) => {
            let ps: Vec<&[u8]> = avc.sps.iter().chain(&avc.pps).map(Vec::as_slice).collect();
            index_h264(id, tb, NalFormat::LengthPrefixed(avc.length_size), &ps, packets)
        }
        CodecConfig::Hevc(hvcc) => {
            let ps: Vec<&[u8]> = hvcc.arrays.iter().flat_map(|a| a.nalus.iter().map(Vec::as_slice)).collect();
            index_hevc(id, tb, NalFormat::LengthPrefixed(hvcc.length_size), &ps, packets)
        }
        CodecConfig::Av1(av1c) => index_av1(id, tb, &av1c.config_obus, packets),
        _ => index_mpeg2(id, tb, packets),
    }
    .map_err(|e| MkvSourceError::Index { detail: format!("{e:?}") })?;
    Ok(MkvVideo { track, codec, config, index })
}
