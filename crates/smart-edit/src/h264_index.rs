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

use cfa4_bitstream::{BitReader, annexb_nals, length_prefixed_nals, unescape_rbsp};

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
/// SEI payloads per NAL unit looked at.
const MAX_SEI_MESSAGES: usize = 64;

/// Recovery point SEI fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Recovery {
    frame_cnt: u32,
    exact_match: bool,
    broken_link: bool,
}

#[derive(Default)]
struct Picture {
    /// Worst slice type: 0 none, 1 I, 2 P, 3 B.
    rank: u8,
    idr: bool,
    reference: bool,
    recovery: Option<Recovery>,
    slices: usize,
}

/// Parameter sets by id; a changed payload for a known id is a configuration change.
#[derive(Default)]
struct Params {
    sps: BTreeMap<u32, Vec<u8>>,
    pps: BTreeMap<u32, Vec<u8>>,
}

impl Params {
    /// Record a parameter set; true when it replaces a different one with the same id.
    fn record(&mut self, nal_type: u8, nal: &[u8]) -> bool {
        let rbsp = unescape_rbsp(nal.get(1..).unwrap_or(&[]));
        let mut r = BitReader::new(&rbsp);
        let id = if nal_type == NAL_SPS {
            // profile_idc, constraint flags, level_idc
            if r.skip(24).is_err() {
                return false;
            }
            r.read_ue()
        } else {
            r.read_ue()
        };
        let Ok(id) = id else { return false };
        let map = if nal_type == NAL_SPS { &mut self.sps } else { &mut self.pps };
        // trailing zero bytes do not change the parameter set
        let mut body = rbsp;
        while body.last() == Some(&0) {
            body.pop();
        }
        match map.insert(id, body.clone()) {
            Some(old) => old != body,
            None => false,
        }
    }
}

fn slice_rank(nal: &[u8]) -> Option<u8> {
    let head = nal.get(1..nal.len().min(1 + SLICE_HEAD))?;
    let rbsp = unescape_rbsp(head);
    let mut r = BitReader::new(&rbsp);
    let _first_mb = r.read_ue().ok()?;
    let slice_type = r.read_ue().ok()?;
    // 0 P, 1 B, 2 I, 3 SP, 4 SI (+5 when every slice of the picture has the same type)
    Some(match slice_type % 5 {
        2 | 4 => 1,
        0 | 3 => 2,
        _ => 3,
    })
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

fn analyse(data: &[u8], format: NalFormat, params: &mut Params) -> (Picture, bool) {
    let nals = match format {
        NalFormat::AnnexB => annexb_nals(data),
        NalFormat::LengthPrefixed(n) => length_prefixed_nals(data, usize::from(n)).unwrap_or_default(),
    };
    let mut pic = Picture::default();
    let mut changed = false;
    for nal in nals {
        let Some(&h) = nal.first() else { continue };
        let ty = h & 0x1F;
        let ref_idc = (h >> 5) & 3;
        match ty {
            NAL_SLICE | NAL_IDR => {
                pic.slices += 1;
                pic.idr |= ty == NAL_IDR;
                pic.reference |= ref_idc != 0;
                if let Some(rank) = slice_rank(nal) {
                    pic.rank = pic.rank.max(rank);
                }
            }
            NAL_SEI => {
                if pic.recovery.is_none() {
                    pic.recovery = recovery_point(nal);
                }
            }
            NAL_SPS | NAL_PPS => changed |= params.record(ty, nal),
            _ => {}
        }
    }
    (pic, changed)
}

/// Index an H.264 stream from its access units in decode order. `parameter_sets` are the SPS /
/// PPS NAL units of the container's codec configuration (`avcC`), if any.
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
    let mut params = Params::default();
    for ps in parameter_sets {
        if let Some(&h) = ps.first() {
            params.record(h & 0x1F, ps);
        }
    }
    let mut frames: Vec<FrameIndexEntry> = Vec::new();
    let mut config_id = 0u32;
    let mut config_changes = Vec::new();
    // latest access point: (pts, kind)
    let mut ap: Option<(Option<i64>, RandomAccessKind)> = None;
    for (i, p) in packets.into_iter().enumerate() {
        if i >= MAX_FRAMES {
            return Err(IndexError::TooManyFrames);
        }
        let di = i as u64;
        let size = u32::try_from(p.data.len()).map_err(|_| IndexError::PacketTooLarge { decode_index: di })?;
        let (pic, changed) = analyse(p.data, format, &mut params);
        if changed {
            config_id = config_id.saturating_add(1);
            config_changes.push(di);
        }
        let picture_type = match pic.rank {
            1 => PictureType::I,
            2 => PictureType::P,
            3 => PictureType::B,
            _ => PictureType::Unknown,
        };
        let random_access = if pic.idr {
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
                None => DependencyStatus::Undecodable,
                Some((ap_pts, kind)) => {
                    let leading = matches!((p.pts, ap_pts), (Some(a), Some(b)) if a < b);
                    match kind {
                        RandomAccessKind::RecoveryPoint { broken_link: true, .. } if leading => DependencyStatus::Undecodable,
                        RandomAccessKind::RecoveryPoint { .. } if leading => DependencyStatus::CrossesAccessPoint,
                        _ => DependencyStatus::WithinGop,
                    }
                }
            }
        };
        if random_access.is_access_point() {
            ap = Some((p.pts, random_access));
        }
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
