//! AV1 index: OBUs of each temporal unit, the start of every frame header, sequence header changes
//! (AV1 bitstream specification §5.3, §5.5, §5.9.2, §7.5).
//!
//! A packet is one temporal unit (as in MP4 / Matroska samples): the frames it decodes and the one
//! frame it shows. A temporal unit whose first frame is a key frame that is shown is a clean access
//! point ([`RandomAccessKind::KeyFrame`]): a shown key frame refreshes every reference slot, so
//! nothing after it refers to anything before it. It needs a sequence header, from the codec
//! configuration (`av1C`) or in-band before it.
//!
//! A key frame that is not shown (a forward key frame, shown later with `show_existing_frame`) is
//! intra but not used as a cut point: the frames between it and its showing may refer to frames
//! before it. Intra-only frames do not reset the reference slots either. Reference slots are not
//! resolved, so [`FrameIndexEntry::references`] is `None` and every decoded frame counts as a
//! possible reference; a temporal unit that only shows an existing frame decodes nothing new.
//!
//! Only the base spatial layer (`spatial_id` 0) is read.

use cfa4_bitstream::BitReader;

use crate::capabilities::Codec;
use crate::media_index::{
    DependencyStatus, FrameIndexEntry, IndexError, MAX_FRAMES, Packet, PictureType, RandomAccessKind, StreamIndex, gops, presentation_from_pts,
    timestamp_problems,
};
use crate::timestamp::Rational;

const OBU_SEQUENCE_HEADER: u8 = 1;
const OBU_FRAME_HEADER: u8 = 3;
const OBU_FRAME: u8 = 6;
/// Largest sequence header kept (they are a few dozen bytes; see the H.264 index).
const MAX_SEQUENCE_HEADER_BYTES: usize = 4096;

const KEY_FRAME: u32 = 0;
const INTRA_ONLY_FRAME: u32 = 2;

/// One OBU: type, `spatial_id` (0 without an extension) and payload.
struct Obu<'a> {
    kind: u8,
    spatial_id: u8,
    payload: &'a [u8],
}

/// `leb128()` (§4.10.5): the value and its length in bytes.
fn leb128(data: &[u8]) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for (i, &b) in data.iter().take(8).enumerate() {
        value |= u64::from(b & 0x7f) << (7 * i);
        if b & 0x80 == 0 {
            return Some((value, i + 1));
        }
    }
    None
}

/// The OBUs of a temporal unit (low-overhead format, §5.2), until the data ends or an OBU is
/// malformed.
fn obus(mut data: &[u8]) -> Vec<Obu<'_>> {
    let mut out = Vec::new();
    while let Some(&header) = data.first() {
        if header & 0x80 != 0 {
            break; // obu_forbidden_bit
        }
        let kind = (header >> 3) & 0x0f;
        let extension = header & 0x04 != 0;
        let has_size = header & 0x02 != 0;
        let mut at = 1usize;
        let mut spatial_id = 0;
        if extension {
            let Some(&ext) = data.get(1) else { break };
            spatial_id = (ext >> 3) & 0x03;
            at = 2;
        }
        let size = if has_size {
            let Some((size, n)) = data.get(at..).and_then(leb128) else { break };
            at += n;
            match usize::try_from(size) {
                Ok(size) => size,
                Err(_) => break,
            }
        } else {
            data.len().saturating_sub(at)
        };
        let Some(payload) = at.checked_add(size).and_then(|end| data.get(at..end)) else { break };
        out.push(Obu { kind, spatial_id, payload });
        data = data.get(at + size..).unwrap_or_default();
    }
    out
}

/// `reduced_still_picture_header` of a sequence header (§5.5.1).
fn reduced_still_picture(sequence_header: &[u8]) -> Option<bool> {
    let mut r = BitReader::new(sequence_header);
    r.skip(3).ok()?; // seq_profile
    r.skip(1).ok()?; // still_picture
    r.read_flag().ok()
}

/// What the start of a frame header says (§5.9.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Header {
    /// `show_existing_frame`: shows a frame decoded before, decodes nothing.
    ShowExisting,
    /// A new frame: `frame_type` and `show_frame`.
    Frame { frame_type: u32, shown: bool },
}

fn frame_header(payload: &[u8], reduced_still_picture: bool) -> Option<Header> {
    if reduced_still_picture {
        return Some(Header::Frame { frame_type: KEY_FRAME, shown: true });
    }
    let mut r = BitReader::new(payload);
    if r.read_flag().ok()? {
        return Some(Header::ShowExisting);
    }
    let frame_type = r.read_bits(2).ok()?;
    let shown = r.read_flag().ok()?;
    Some(Header::Frame { frame_type, shown })
}

#[derive(Default)]
struct TemporalUnit {
    /// The frame headers of the base layer, in order (a redundant copy is not counted).
    headers: Vec<Header>,
    /// The sequence header in force for the first frame (payload, trailing zeros dropped).
    config: Option<Vec<u8>>,
}

/// The sequence header being used and whether its frame headers are reduced.
#[derive(Default)]
struct State {
    sequence_header: Option<Vec<u8>>,
    reduced: bool,
}

impl State {
    fn record(&mut self, payload: &[u8]) {
        if payload.len() > MAX_SEQUENCE_HEADER_BYTES {
            return;
        }
        let Some(reduced) = reduced_still_picture(payload) else { return };
        let mut body = payload.to_vec();
        while body.last() == Some(&0) {
            body.pop();
        }
        self.sequence_header = Some(body);
        self.reduced = reduced;
    }
}

fn analyse(data: &[u8], state: &mut State) -> TemporalUnit {
    let mut tu = TemporalUnit::default();
    for obu in obus(data) {
        if obu.spatial_id != 0 {
            continue;
        }
        match obu.kind {
            OBU_SEQUENCE_HEADER => state.record(obu.payload),
            OBU_FRAME_HEADER | OBU_FRAME => {
                if let Some(h) = frame_header(obu.payload, state.reduced) {
                    if tu.headers.is_empty() {
                        tu.config = state.sequence_header.clone();
                    }
                    tu.headers.push(h);
                }
            }
            _ => {}
        }
    }
    tu
}

impl TemporalUnit {
    /// Whether the unit is a clean access point, an intra unit that is no cut point, or neither.
    fn random_access(&self) -> RandomAccessKind {
        let first = self.headers.first().copied();
        if first == Some(Header::Frame { frame_type: KEY_FRAME, shown: true }) && self.config.is_some() {
            RandomAccessKind::KeyFrame
        } else if self.headers.iter().any(|h| matches!(h, Header::Frame { frame_type: KEY_FRAME | INTRA_ONLY_FRAME, .. })) {
            RandomAccessKind::IntraOnly
        } else {
            RandomAccessKind::None
        }
    }
}

/// Classifies AV1 temporal units one by one, in decode order, as [`index_av1`] does: a check that
/// a copied run of packets still has the access points of the index (Smart Cut executor).
pub struct Av1AccessPoints {
    state: State,
}

impl Av1AccessPoints {
    /// Start with the sequence header of the codec configuration (`configOBUs` of `av1C`), if any.
    pub fn new(config_obus: &[u8]) -> Av1AccessPoints {
        let mut state = State::default();
        for obu in obus(config_obus) {
            if obu.kind == OBU_SEQUENCE_HEADER {
                state.record(obu.payload);
            }
        }
        Av1AccessPoints { state }
    }

    /// The random-access kind of the next temporal unit (`KeyFrame`, `IntraOnly` or `None`).
    pub fn next(&mut self, data: &[u8]) -> RandomAccessKind {
        analyse(data, &mut self.state).random_access()
    }
}

/// Index an AV1 stream from its temporal units in decode order. `config_obus` are the OBUs of the
/// container's codec configuration (the `configOBUs` of `av1C`: a sequence header), if any.
pub fn index_av1<'a>(stream_id: u32, timebase: Rational, config_obus: &[u8], packets: impl IntoIterator<Item = Packet<'a>>) -> Result<StreamIndex, IndexError> {
    if !timebase.is_valid() {
        return Err(IndexError::InvalidTimebase(timebase));
    }
    let mut state = Av1AccessPoints::new(config_obus).state;
    let mut frames: Vec<FrameIndexEntry> = Vec::new();
    let mut config_id = 0u32;
    let mut config_changes = Vec::new();
    let mut config: Option<Vec<u8>> = None;
    let mut keyed = false;
    for (i, p) in packets.into_iter().enumerate() {
        if i >= MAX_FRAMES {
            return Err(IndexError::TooManyFrames);
        }
        let di = i as u64;
        let size = u32::try_from(p.data.len()).map_err(|_| IndexError::PacketTooLarge { decode_index: di })?;
        let tu = analyse(p.data, &mut state);
        if let Some(active) = tu.config.clone() {
            if config.as_ref().is_some_and(|c| *c != active) {
                config_id = config_id.saturating_add(1);
                config_changes.push(di);
            }
            config = Some(active);
        }
        let first = tu.headers.first().copied();
        let random_access = tu.random_access();
        let clean = random_access == RandomAccessKind::KeyFrame;
        let decodes = tu.headers.iter().any(|h| matches!(h, Header::Frame { .. }));
        let picture_type = match first {
            None => PictureType::Unknown,
            Some(_) if clean => PictureType::I,
            // the shown frame decides: the last frame header of the unit
            Some(_) => match tu.headers.last() {
                Some(Header::Frame { frame_type: KEY_FRAME | INTRA_ONLY_FRAME, .. }) => PictureType::I,
                Some(Header::Frame { .. }) => PictureType::P,
                _ => PictureType::Unknown,
            },
        };
        let dependency_status = match (first, clean, keyed) {
            (None, _, _) => DependencyStatus::Unknown,
            (_, true, _) => DependencyStatus::Independent,
            (_, false, true) => DependencyStatus::WithinGop,
            (_, false, false) => DependencyStatus::Undecodable,
        };
        keyed |= clean;
        frames.push(FrameIndexEntry {
            stream_id,
            decode_index: di,
            presentation_index: None,
            pts: p.pts,
            dts: p.dts,
            keyframe: p.keyframe,
            picture_type,
            reference: decodes,
            random_access,
            dependency_status,
            references: None,
            config_id,
            size,
        });
    }
    let pts: Vec<Option<i64>> = frames.iter().map(|f| f.pts).collect();
    if let Some(order) = presentation_from_pts(&pts) {
        for (f, p) in frames.iter_mut().zip(order) {
            f.presentation_index = Some(p);
        }
    }
    let gops = gops(&frames);
    let timestamp_problems = timestamp_problems(&frames);
    Ok(StreamIndex { stream_id, codec: Codec::Av1, timebase, frames, gops, config_changes, timestamp_problems })
}
