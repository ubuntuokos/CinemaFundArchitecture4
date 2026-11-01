//! Media / GOP index (WP-02): one entry per coded picture, in decode order, with its timestamps,
//! picture type, random-access kind and reference dependencies, plus the GOPs and codec
//! configuration changes of the stream.
//!
//! The index is built from the bitstream, not from the container's sync flags: a keyframe flag (or
//! an I picture) alone does not prove a clean cut point (plan SC-02). [`RandomAccessKind::is_clean`]
//! says whether decoding can start at a picture with every later picture (in presentation order)
//! decodable; [`DependencyStatus`] says what each picture needs.
//!
//! Codec analysers: [`crate::mpeg2_index`] and [`crate::h264_index`].

use serde::{Deserialize, Serialize};

use crate::capabilities::Codec;
use crate::timestamp::{MediaTime, Rational};

/// Version of the index data model; a cached index of another version is rebuilt.
pub const INDEX_VERSION: u32 = 1;

/// Most pictures one stream index holds (about 11.5 days at 50 fps); longer inputs are refused
/// rather than allocating without bound.
pub const MAX_FRAMES: usize = 50_000_000;

/// The coded picture type. For H.264 the "worst" slice type of the picture (B over P over I).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PictureType {
    I,
    P,
    B,
    /// MPEG-1 DC-coded picture (intra).
    D,
    /// The picture header could not be read.
    Unknown,
}

impl PictureType {
    pub fn is_intra(self) -> bool {
        matches!(self, PictureType::I | PictureType::D)
    }
}

/// Whether, and how cleanly, decoding can start at a picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RandomAccessKind {
    /// Decoding cannot start here.
    None,
    /// H.264 IDR picture: the decoder is reset; nothing after it refers to anything before it.
    Idr,
    /// H.264 I picture with a recovery point SEI (`recovery_frame_cnt` 0, `exact_match`): later
    /// pictures in output order decode exactly; leading pictures may not.
    RecoveryPoint { exact_match: bool, broken_link: bool },
    /// MPEG-2 I picture after a sequence header in a closed GOP (or a GOP with no leading B
    /// pictures): a clean start.
    ClosedGop,
    /// MPEG-2 I picture after a sequence header in an open GOP: its leading B pictures need the
    /// previous GOP.
    OpenGop { broken_link: bool },
    /// An intra picture without the signalling that makes it an access point (an H.264 non-IDR I
    /// picture without recovery point SEI, an MPEG-2 I picture without a sequence header before
    /// it). Not a safe cut point on its own.
    IntraOnly,
}

impl RandomAccessKind {
    /// Decoding may start here.
    pub fn is_access_point(self) -> bool {
        !matches!(self, RandomAccessKind::None | RandomAccessKind::IntraOnly)
    }

    /// Starting here, every picture at or after this one in presentation order decodes
    /// exactly, and nothing at or after it refers to anything before it in decode order.
    pub fn is_clean(self) -> bool {
        matches!(self, RandomAccessKind::Idr | RandomAccessKind::ClosedGop)
    }
}

/// What a picture needs to decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyStatus {
    /// Intra-coded: needs no other picture.
    Independent,
    /// Refers only to pictures of its own GOP (since the last access point in decode order).
    WithinGop,
    /// A leading picture of an open GOP (shown before its access point): refers to the previous
    /// GOP, so it is lost when decoding starts at that access point.
    CrossesAccessPoint,
    /// Cannot be decoded correctly: a reference is missing (stream starts mid-GOP, broken link).
    Undecodable,
    /// The picture could not be analysed.
    Unknown,
}

/// One coded picture, in decode order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameIndexEntry {
    pub stream_id: u32,
    /// Position in decode (bitstream) order, from 0.
    pub decode_index: u64,
    /// Position in presentation order, from 0; `None` when it cannot be derived (missing
    /// timestamps and no codec ordering information).
    pub presentation_index: Option<u64>,
    /// Container timestamps on the stream timebase. `None`: the container gave none (kept apart
    /// from zero).
    pub pts: Option<i64>,
    pub dts: Option<i64>,
    /// The container marked the packet as a sync sample / keyframe.
    pub keyframe: bool,
    pub picture_type: PictureType,
    /// Other pictures may refer to this one.
    pub reference: bool,
    pub random_access: RandomAccessKind,
    pub dependency_status: DependencyStatus,
    /// Decode indices of the pictures this one refers to, when the codec analyser knows them
    /// exactly (MPEG-2); `None` when it does not (H.264 reference lists are not resolved).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub references: Option<Vec<u64>>,
    /// Codec configuration in effect (bumped at every sequence header / parameter-set change).
    pub config_id: u32,
    /// Coded size in bytes.
    pub size: u32,
}

/// A group of pictures: from one access point up to (not including) the next one, in decode
/// order. Pictures before the first access point form no GOP.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GopInfo {
    /// Decode index of the access point.
    pub start: u64,
    /// Pictures in the GOP.
    pub count: u64,
    pub kind: RandomAccessKind,
}

/// The index of one video stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamIndex {
    pub stream_id: u32,
    pub codec: Codec,
    pub timebase: Rational,
    pub frames: Vec<FrameIndexEntry>,
    pub gops: Vec<GopInfo>,
    /// Decode indices where the codec configuration changes (after the first one).
    pub config_changes: Vec<u64>,
    /// Pictures whose timestamps go backwards in decode order (DTS) or repeat (PTS).
    pub timestamp_problems: Vec<u64>,
}

impl StreamIndex {
    /// Decode indices of the clean access points (safe lossless cut-in points).
    pub fn clean_access_points(&self) -> Vec<u64> {
        self.frames.iter().filter(|f| f.random_access.is_clean()).map(|f| f.decode_index).collect()
    }

    /// Decode indices of every access point, clean or not.
    pub fn access_points(&self) -> Vec<u64> {
        self.frames.iter().filter(|f| f.random_access.is_access_point()).map(|f| f.decode_index).collect()
    }

    /// The picture at presentation position `p`.
    pub fn by_presentation(&self, p: u64) -> Option<&FrameIndexEntry> {
        self.frames.iter().find(|f| f.presentation_index == Some(p))
    }

    /// Stream duration from the presentation timestamps: last PTS + the last frame interval
    /// minus the first PTS (`None` without timestamps).
    pub fn duration(&self) -> Option<MediaTime> {
        let mut pts: Vec<i64> = self.frames.iter().filter_map(|f| f.pts).collect();
        pts.sort_unstable();
        let first = *pts.first()?;
        let last = *pts.last()?;
        let step = match pts.len() {
            n if n >= 2 => pts.get(n - 1)?.checked_sub(*pts.get(n - 2)?)?,
            _ => 0,
        };
        Some(MediaTime::new(last.checked_sub(first)?.checked_add(step)?, self.timebase))
    }
}

/// What identifies the source file an index was built from (plan §5: size, modification state
/// and, where needed, a content identifier).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFingerprint {
    pub size: u64,
    /// Modification time, nanoseconds since the Unix epoch, when the file system gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_ns: Option<i128>,
    /// A content identifier (a hash of the file, or of its head and tail), when one was computed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_id: Option<String>,
}

/// Why a cached index cannot be used.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "reason")]
pub enum Staleness {
    OtherVersion {
        found: u32,
    },
    SizeChanged {
        indexed: u64,
        now: u64,
    },
    Modified,
    ContentChanged,
    /// The index has a content id and the caller could not provide one to compare.
    ContentUnknown,
}

/// The index of one source file. The original media is only read, never modified.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaIndex {
    pub source_id: String,
    pub fingerprint: SourceFingerprint,
    pub streams: Vec<StreamIndex>,
    pub duration: Option<MediaTime>,
    pub index_version: u32,
}

impl MediaIndex {
    pub fn new(source_id: impl Into<String>, fingerprint: SourceFingerprint, streams: Vec<StreamIndex>) -> MediaIndex {
        // the longest stream, measured on its own timebase converted to ticks
        let duration = streams.iter().filter_map(|s| s.duration()).max_by_key(|d| d.to_tick().map_or(i64::MIN, |t| t.0));
        MediaIndex { source_id: source_id.into(), fingerprint, streams, duration, index_version: INDEX_VERSION }
    }

    /// Whether this (cached) index still describes the file whose fingerprint is `now`.
    pub fn check_valid(&self, now: &SourceFingerprint) -> Result<(), Staleness> {
        if self.index_version != INDEX_VERSION {
            return Err(Staleness::OtherVersion { found: self.index_version });
        }
        if self.fingerprint.size != now.size {
            return Err(Staleness::SizeChanged { indexed: self.fingerprint.size, now: now.size });
        }
        if let (Some(a), Some(b)) = (self.fingerprint.modified_ns, now.modified_ns)
            && a != b
        {
            return Err(Staleness::Modified);
        }
        if self.fingerprint.modified_ns.is_some() != now.modified_ns.is_some() {
            return Err(Staleness::Modified);
        }
        match (&self.fingerprint.content_id, &now.content_id) {
            (Some(a), Some(b)) if a != b => Err(Staleness::ContentChanged),
            (Some(_), None) => Err(Staleness::ContentUnknown),
            _ => Ok(()),
        }
    }
}

/// One coded picture as the container delivers it, in decode order.
#[derive(Clone, Copy, Debug)]
pub struct Packet<'a> {
    pub data: &'a [u8],
    pub pts: Option<i64>,
    pub dts: Option<i64>,
    /// The container marked it as a sync sample.
    pub keyframe: bool,
}

/// A stream that cannot be indexed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexError {
    InvalidTimebase(Rational),
    TooManyFrames,
    /// The packet is larger than `u32::MAX` bytes.
    PacketTooLarge {
        decode_index: u64,
    },
    /// NAL length prefixes are 1, 2 or 4 bytes (`avcC` `lengthSizeMinusOne`); 3 is accepted too.
    InvalidNalLengthSize(u8),
}

impl std::fmt::Display for IndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IndexError::InvalidTimebase(r) => write!(f, "invalid timebase {}/{}", r.num, r.den),
            IndexError::TooManyFrames => write!(f, "more than {MAX_FRAMES} pictures in one stream"),
            IndexError::PacketTooLarge { decode_index } => write!(f, "packet {decode_index} is larger than 4 GiB"),
            IndexError::InvalidNalLengthSize(n) => write!(f, "NAL length prefix of {n} bytes"),
        }
    }
}

impl std::error::Error for IndexError {}

/// Presentation positions from the timestamps (ties keep decode order); `None` for every picture
/// when any picture lacks a PTS.
pub(crate) fn presentation_from_pts(pts: &[Option<i64>]) -> Option<Vec<u64>> {
    let mut order: Vec<(i64, usize)> = Vec::with_capacity(pts.len());
    for (i, p) in pts.iter().enumerate() {
        order.push(((*p)?, i));
    }
    order.sort_unstable();
    let mut out = vec![0u64; pts.len()];
    for (rank, &(_, i)) in order.iter().enumerate() {
        if let Some(slot) = out.get_mut(i) {
            *slot = rank as u64;
        }
    }
    Some(out)
}

/// Decode indices whose DTS goes backwards or whose PTS repeats an earlier one.
pub(crate) fn timestamp_problems(frames: &[FrameIndexEntry]) -> Vec<u64> {
    let mut out = Vec::new();
    let mut last_dts: Option<i64> = None;
    let mut seen = std::collections::BTreeSet::new();
    for f in frames {
        let mut bad = false;
        if let Some(d) = f.dts {
            if last_dts.is_some_and(|l| d <= l) {
                bad = true;
            }
            last_dts = Some(d);
        }
        if let Some(p) = f.pts
            && !seen.insert(p)
        {
            bad = true;
        }
        if bad {
            out.push(f.decode_index);
        }
    }
    out
}

/// GOPs from the access points of decode-ordered pictures.
pub(crate) fn gops(frames: &[FrameIndexEntry]) -> Vec<GopInfo> {
    let mut out: Vec<GopInfo> = Vec::new();
    for f in frames {
        if f.random_access.is_access_point() {
            out.push(GopInfo { start: f.decode_index, count: 1, kind: f.random_access });
        } else if let Some(g) = out.last_mut() {
            g.count = g.count.saturating_add(1);
        }
    }
    out
}
