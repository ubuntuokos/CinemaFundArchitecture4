//! H.264 / AVC index: NAL unit types, `nal_ref_idc`, slice types, SPS / PPS changes and recovery
//! point SEI (ITU-T H.264 §7.3, §7.4.1, §D.2.8).
//!
//! An IDR picture is a clean access point. A non-IDR I picture is an access point only with a
//! recovery point SEI (`recovery_frame_cnt` 0); without one it is [`RandomAccessKind::IntraOnly`]:
//! later pictures may still refer to pictures before it (the DPB is not reset). Pictures shown
//! before a non-IDR access point but decoded after it are leading pictures that refer to the
//! previous GOP.
//!
//! Reference lists are not resolved (that needs the full slice header and DPB simulation), so
//! [`FrameIndexEntry::references`] is `None`; dependencies are classified per GOP.

use std::collections::BTreeMap;

use cfa4_bitstream::{BitReader, unescape_rbsp};

use crate::capabilities::Codec;
use crate::media_index::{
    DependencyStatus, FrameIndexEntry, IndexError, MAX_FRAMES, Packet, PictureType, RandomAccessKind, StreamIndex, gops, presentation_from_pts,
    timestamp_problems,
};
use crate::timestamp::Rational;

/// How NAL units are framed in each packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NalFormat {
    /// Start codes (`00 00 01`): elementary streams, MPEG-TS.
    AnnexB,
    /// Big-endian length before each NAL unit (MP4 / Matroska, `avcC` `lengthSizeMinusOne` + 1).
    LengthPrefixed(u8),
}

const NAL_SLICE: u8 = 1;
const NAL_IDR: u8 = 5;
const NAL_SEI: u8 = 6;
const NAL_SPS: u8 = 7;
const NAL_PPS: u8 = 8;
const SEI_RECOVERY_POINT: u32 = 6;
/// Slice header bytes read for `first_mb_in_slice` and `slice_type`.
const SLICE_HEAD: usize = 32;
/// `seq_parameter_set_id` is 0..=31, `pic_parameter_set_id` 0..=255 (H.264 §7.4.2.1, §7.4.2.2).
const MAX_SPS_ID: u32 = 31;
const MAX_PPS_ID: u32 = 255;
/// Largest parameter set kept. Real ones are a few hundred bytes; with the id ranges this bounds
/// what hostile input can make the index hold to about 1 MiB.
const MAX_PARAMETER_SET_BYTES: usize = 4096;
/// SEI payloads per NAL unit looked at.
const MAX_SEI_MESSAGES: usize = 64;

/// Recovery point SEI fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Recovery {
    frame_cnt: u32,
    exact_match: bool,
    broken_link: bool,
}

/// The SPS and PPS payloads a picture is coded with.
type Config = (Vec<u8>, Vec<u8>);

#[derive(Default)]
struct Picture {
    /// Worst slice type: 0 none, 1 I, 2 P, 3 B.
    rank: u8,
    idr: bool,
    reference: bool,
    recovery: Option<Recovery>,
    slices: usize,
    /// Parameter sets selected by the first slice's `pic_parameter_set_id`; `None` when they are
    /// unknown.
    config: Option<Config>,
}

/// Parameter sets by id (PPS with the id of the SPS it refers to). Ids out of range and oversized
/// payloads are not kept: pictures using them have an unknown configuration.
#[derive(Default)]
struct Params {
    sps: BTreeMap<u32, Vec<u8>>,
    pps: BTreeMap<u32, (u32, Vec<u8>)>,
}

impl Params {
    fn record(&mut self, nal_type: u8, nal: &[u8]) {
        if nal.len() > MAX_PARAMETER_SET_BYTES {
            return;
        }
        let rbsp = unescape_rbsp(nal.get(1..).unwrap_or(&[]));
        let mut r = BitReader::new(&rbsp);
        // trailing zero bytes do not change the parameter set
        let trimmed = |mut body: Vec<u8>| {
            while body.last() == Some(&0) {
                body.pop();
            }
            body
        };
        if nal_type == NAL_SPS {
            // profile_idc, constraint flags, level_idc
            if r.skip(24).is_err() {
                return;
            }
            let Ok(id) = r.read_ue() else { return };
            if id <= MAX_SPS_ID {
                self.sps.insert(id, trimmed(rbsp));
            }
        } else {
            let (Ok(id), Ok(sps_id)) = (r.read_ue(), r.read_ue()) else { return };
            if id <= MAX_PPS_ID && sps_id <= MAX_SPS_ID {
                self.pps.insert(id, (sps_id, trimmed(rbsp)));
            }
        }
    }

    /// The configuration a slice with this PPS id is decoded with.
    fn active(&self, pps_id: u32) -> Option<Config> {
        let (sps_id, pps) = self.pps.get(&pps_id)?;
        Some((self.sps.get(sps_id)?.clone(), pps.clone()))
    }
}

/// Slice rank (see [`Picture::rank`]) and `pic_parameter_set_id` of a slice header.
fn slice_header(nal: &[u8]) -> Option<(u8, Option<u32>)> {
    let head = nal.get(1..nal.len().min(1 + SLICE_HEAD))?;
    let rbsp = unescape_rbsp(head);
    let mut r = BitReader::new(&rbsp);
    let _first_mb = r.read_ue().ok()?;
    let slice_type = r.read_ue().ok()?;
    let pps_id = r.read_ue().ok();
    // 0 P, 1 B, 2 I, 3 SP, 4 SI (+5 when every slice of the picture has the same type)
    let rank = match slice_type % 5 {
        2 | 4 => 1,
        0 | 3 => 2,
        _ => 3,
    };
    Some((rank, pps_id))
}

fn recovery_point(nal: &[u8]) -> Option<Recovery> {
    let rbsp = unescape_rbsp(nal.get(1..)?);
    let mut pos = 0usize;
    for _ in 0..MAX_SEI_MESSAGES {
        // rbsp_trailing_bits
        if rbsp.len().saturating_sub(pos) < 2 {
            return None;
        }
        let read = |pos: &mut usize| -> Option<u32> {
            let mut v = 0u32;
            loop {
                let b = *rbsp.get(*pos)?;
                *pos += 1;
                v = v.checked_add(u32::from(b))?;
                if b != 0xFF {
                    return Some(v);
                }
            }
        };
        let ty = read(&mut pos)?;
        let size = usize::try_from(read(&mut pos)?).ok()?;
        let end = pos.checked_add(size)?;
        let payload = rbsp.get(pos..end)?;
        if ty == SEI_RECOVERY_POINT {
            let mut r = BitReader::new(payload);
            let frame_cnt = r.read_ue().ok()?;
            let exact_match = r.read_flag().ok()?;
            let broken_link = r.read_flag().ok()?;
            return Some(Recovery { frame_cnt, exact_match, broken_link });
        }
        pos = end;
    }
    None
}

/// Annex B NAL units of one packet, found lazily (no allocation per NAL unit, so a hostile packet
/// full of start codes cannot amplify memory). Trailing zero bytes (a 4-byte start code or
/// `trailing_zero_8bits`) are trimmed and empty units skipped.
struct AnnexBNals<'a> {
    data: &'a [u8],
    /// Start of the current NAL unit payload, `None` before the first start code is found.
    pos: Option<usize>,
}

/// Position just after the next `00 00 01` at or after `from`.
fn next_start_code(data: &[u8], from: usize) -> Option<usize> {
    let rest = data.get(from..)?;
    rest.windows(3).position(|w| w == [0, 0, 1]).and_then(|i| from.checked_add(i)?.checked_add(3))
}

impl<'a> Iterator for AnnexBNals<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        loop {
            let start = match self.pos {
                Some(p) => p,
                None => next_start_code(self.data, 0)?,
            };
            let next = next_start_code(self.data, start);
            let end = next.map_or(self.data.len(), |n| n.saturating_sub(3));
            self.pos = Some(next.unwrap_or(self.data.len()));
            let mut nal = self.data.get(start..end).unwrap_or_default();
            while let [head @ .., 0] = nal {
                nal = head;
            }
            if next.is_none() && nal.is_empty() {
                return None;
            }
            if !nal.is_empty() {
                return Some(nal);
            }
        }
    }
}

/// Length-prefixed NAL units of one packet, read lazily. Empty units are skipped.
struct LengthPrefixedNals<'a> {
    data: &'a [u8],
    size: usize,
}

impl<'a> Iterator for LengthPrefixedNals<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        loop {
            let (len, rest) = split_length(self.data, self.size)?;
            let nal = rest.get(..len)?;
            self.data = rest.get(len..)?;
            if !nal.is_empty() {
                return Some(nal);
            }
        }
    }
}

fn split_length(data: &[u8], size: usize) -> Option<(usize, &[u8])> {
    let (prefix, rest) = (data.get(..size)?, data.get(size..)?);
    let len = prefix.iter().try_fold(0usize, |acc, &b| acc.checked_mul(256)?.checked_add(usize::from(b)))?;
    Some((len, rest))
}

/// Whether a length-prefixed packet is framed exactly (every length fits, nothing left over). A
/// packet that is not is skipped whole, as a truncated one would be.
fn length_prefixed_is_well_formed(mut data: &[u8], size: usize) -> bool {
    while !data.is_empty() {
        let Some((len, rest)) = split_length(data, size) else { return false };
        let Some(tail) = rest.get(len..) else { return false };
        data = tail;
    }
    true
}

fn nal_units(data: &[u8], format: NalFormat) -> Box<dyn Iterator<Item = &[u8]> + '_> {
    match format {
        NalFormat::AnnexB => Box::new(AnnexBNals { data, pos: None }),
        NalFormat::LengthPrefixed(n) => {
            let size = usize::from(n);
            if size == 0 || !length_prefixed_is_well_formed(data, size) { Box::new(std::iter::empty()) } else { Box::new(LengthPrefixedNals { data, size }) }
        }
    }
}

/// The NAL units the analyser sees in one packet (tests compare them with `cfa4-bitstream`).
#[cfg(test)]
pub(crate) fn split_nal_units(data: &[u8], format: NalFormat) -> Vec<&[u8]> {
    nal_units(data, format).collect()
}

fn analyse(data: &[u8], format: NalFormat, params: &mut Params) -> Picture {
    let nals = nal_units(data, format);
    let mut pic = Picture::default();
    for nal in nals {
        let Some(&h) = nal.first() else { continue };
        let ty = h & 0x1F;
        let ref_idc = (h >> 5) & 3;
        match ty {
            NAL_SLICE | NAL_IDR => {
                pic.slices += 1;
                pic.idr |= ty == NAL_IDR;
                pic.reference |= ref_idc != 0;
                if let Some((rank, pps_id)) = slice_header(nal) {
                    pic.rank = pic.rank.max(rank);
                    if pic.slices == 1 {
                        pic.config = pps_id.and_then(|id| params.active(id));
                    }
                }
            }
            NAL_SEI => {
                if pic.recovery.is_none() {
                    pic.recovery = recovery_point(nal);
                }
            }
            NAL_SPS | NAL_PPS => params.record(ty, nal),
            _ => {}
        }
    }
    pic
}

/// Index an H.264 stream from its access units in decode order. `parameter_sets` are the SPS /
/// PPS NAL units of the container's codec configuration (`avcC`), if any.
///
/// A configuration change is a picture coded with other SPS / PPS payloads than the picture
/// before it, whether a parameter set was replaced or the slices switched to another id.
pub fn index_h264<'a>(
    stream_id: u32,
    timebase: Rational,
    format: NalFormat,
    parameter_sets: &[&[u8]],
    packets: impl IntoIterator<Item = Packet<'a>>,
) -> Result<StreamIndex, IndexError> {
    if !timebase.is_valid() {
        return Err(IndexError::InvalidTimebase(timebase));
    }
    if let NalFormat::LengthPrefixed(n) = format
        && !(1..=4).contains(&n)
    {
        return Err(IndexError::InvalidNalLengthSize(n));
    }
    let mut params = Params::default();
    for ps in parameter_sets {
        if let Some(&h) = ps.first() {
            params.record(h & 0x1F, ps);
        }
    }
    let mut frames: Vec<FrameIndexEntry> = Vec::new();
    let mut config_id = 0u32;
    let mut config_changes = Vec::new();
    let mut config: Option<Config> = None;
    // latest access point: (pts, kind, whether a decodable picture came before it, so that its
    // leading pictures have the references they predict from)
    let mut ap: Option<(Option<i64>, RandomAccessKind, bool)> = None;
    // an intra picture was seen (an access point or not): pictures after it may have their
    // references even without a signalled access point
    let mut seen_intra = false;
    for (i, p) in packets.into_iter().enumerate() {
        if i >= MAX_FRAMES {
            return Err(IndexError::TooManyFrames);
        }
        let di = i as u64;
        let size = u32::try_from(p.data.len()).map_err(|_| IndexError::PacketTooLarge { decode_index: di })?;
        let pic = analyse(p.data, format, &mut params);
        if let Some(active) = pic.config {
            if config.as_ref().is_some_and(|c| *c != active) {
                config_id = config_id.saturating_add(1);
                config_changes.push(di);
            }
            config = Some(active);
        }
        let picture_type = match pic.rank {
            1 => PictureType::I,
            2 => PictureType::P,
            3 => PictureType::B,
            _ => PictureType::Unknown,
        };
        // an IDR picture whose slice header cannot be read is no proven cut point
        let random_access = if pic.idr && picture_type == PictureType::I {
            RandomAccessKind::Idr
        } else if picture_type == PictureType::I {
            match pic.recovery {
                Some(r) if r.frame_cnt == 0 => RandomAccessKind::RecoveryPoint { exact_match: r.exact_match, broken_link: r.broken_link },
                _ => RandomAccessKind::IntraOnly,
            }
        } else {
            RandomAccessKind::None
        };
        let dependency_status = if pic.slices == 0 || picture_type == PictureType::Unknown {
            DependencyStatus::Unknown
        } else if picture_type == PictureType::I {
            DependencyStatus::Independent
        } else {
            match ap {
                // reference lists are not resolved: after an unsignalled intra picture it stays
                // open whether a picture has its references; before any intra picture it has none
                None if seen_intra => DependencyStatus::Unknown,
                None => DependencyStatus::Undecodable,
                Some((ap_pts, kind, history)) => {
                    let leading = matches!((p.pts, ap_pts), (Some(a), Some(b)) if a < b);
                    let unordered = p.pts.is_none() || ap_pts.is_none();
                    match kind {
                        // leading pictures are found from the PTS (picture order counts are not
                        // derived): without them it stays open whether a picture crosses
                        RandomAccessKind::RecoveryPoint { .. } if unordered => DependencyStatus::Unknown,
                        RandomAccessKind::RecoveryPoint { broken_link: true, .. } if leading => DependencyStatus::Undecodable,
                        // the stream starts at the recovery point: the pictures before it are absent
                        RandomAccessKind::RecoveryPoint { .. } if leading && !history => DependencyStatus::Undecodable,
                        RandomAccessKind::RecoveryPoint { .. } if leading => DependencyStatus::CrossesAccessPoint,
                        _ => DependencyStatus::WithinGop,
                    }
                }
            }
        };
        if random_access.is_access_point() {
            ap = Some((p.pts, random_access, ap.is_some()));
        }
        seen_intra |= picture_type == PictureType::I;
        frames.push(FrameIndexEntry {
            stream_id,
            decode_index: di,
            presentation_index: None,
            pts: p.pts,
            dts: p.dts,
            keyframe: p.keyframe,
            picture_type,
            reference: pic.reference,
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
    Ok(StreamIndex { stream_id, codec: Codec::H264, timebase, frames, gops, config_changes, timestamp_problems })
}
