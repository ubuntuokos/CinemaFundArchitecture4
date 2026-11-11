//! HEVC / H.265 index: NAL unit types, the first slice segment header, VPS / SPS / PPS changes
//! (ITU-T H.265 §7.3.1.2, §7.3.6.1, §7.4.2.2, §8.1.3).
//!
//! The access points are the IRAP pictures. An IDR picture resets the decoder: it is a clean access
//! point. A CRA or BLA picture also starts decoding, but its RASL pictures (shown before it,
//! decoded after it) refer to the pictures before the access point: they are lost when decoding
//! starts there ([`DependencyStatus::CrossesAccessPoint`], or undecodable after a BLA or at the start
//! of the stream). A CRA or BLA picture with no RASL picture is clean. RADL pictures refer only to
//! their IRAP picture and the pictures after it, and stay within the GOP.
//!
//! Only the base layer (`nuh_layer_id` 0) is read. The picture type comes from the first slice
//! segment of the picture (an IRAP picture is always intra); reference lists are not resolved, so
//! [`FrameIndexEntry::references`] is `None`.

use std::collections::BTreeMap;

use cfa4_bitstream::{BitReader, unescape_rbsp};

use crate::capabilities::Codec;
use crate::h264_index::{NalFormat, nal_units};
use crate::media_index::{
    DependencyStatus, FrameIndexEntry, IndexError, MAX_FRAMES, Packet, PictureType, RandomAccessKind, StreamIndex, gops, presentation_from_pts,
    timestamp_problems,
};
use crate::timestamp::Rational;

const RASL_N: u8 = 8;
const RASL_R: u8 = 9;
const BLA_W_LP: u8 = 16;
const BLA_N_LP: u8 = 18;
const IDR_W_RADL: u8 = 19;
const IDR_N_LP: u8 = 20;
const CRA_NUT: u8 = 21;
/// IRAP types run 16..=23 (22 and 23 reserved).
const IRAP_LAST: u8 = 23;
const VPS_NUT: u8 = 32;
const SPS_NUT: u8 = 33;
const PPS_NUT: u8 = 34;
/// Bytes of a slice segment header read (the fields up to `slice_type`).
const SLICE_HEAD: usize = 32;
/// `vps_video_parameter_set_id` 0..=15, `sps_seq_parameter_set_id` 0..=15,
/// `pps_pic_parameter_set_id` 0..=63 (H.265 §7.4.3).
const MAX_VPS_ID: u32 = 15;
const MAX_SPS_ID: u32 = 15;
const MAX_PPS_ID: u32 = 63;
/// Largest parameter set kept (see the H.264 index).
const MAX_PARAMETER_SET_BYTES: usize = 4096;

/// The VPS, SPS and PPS payloads a picture is coded with.
type Config = (Vec<u8>, Vec<u8>, Vec<u8>);

/// Parameter sets by id: SPS with its VPS id, PPS with its SPS id and `num_extra_slice_header_bits`.
#[derive(Default)]
struct Params {
    vps: BTreeMap<u32, Vec<u8>>,
    sps: BTreeMap<u32, (u32, Vec<u8>)>,
    pps: BTreeMap<u32, (u32, u8, Vec<u8>)>,
}

/// Skip `profile_tier_level(1, max_sub_layers_minus1)` (H.265 §7.3.3).
fn skip_profile_tier_level(r: &mut BitReader<'_>, max_sub_layers_minus1: u32) -> Option<()> {
    // general profile (88 bits) and level (8 bits)
    r.skip(96).ok()?;
    let mut present = Vec::new();
    for _ in 0..max_sub_layers_minus1 {
        let profile = r.read_flag().ok()?;
        let level = r.read_flag().ok()?;
        present.push((profile, level));
    }
    if max_sub_layers_minus1 > 0 {
        for _ in max_sub_layers_minus1..8 {
            r.skip(2).ok()?;
        }
    }
    for (profile, level) in present {
        if profile {
            r.skip(88).ok()?;
        }
        if level {
            r.skip(8).ok()?;
        }
    }
    Some(())
}

fn trimmed(mut body: Vec<u8>) -> Vec<u8> {
    // trailing zero bytes do not change the parameter set
    while body.last() == Some(&0) {
        body.pop();
    }
    body
}

impl Params {
    fn record(&mut self, nal_type: u8, nal: &[u8]) {
        if nal.len() > MAX_PARAMETER_SET_BYTES {
            return;
        }
        let rbsp = unescape_rbsp(nal.get(2..).unwrap_or(&[]));
        let mut r = BitReader::new(&rbsp);
        match nal_type {
            VPS_NUT => {
                let Ok(id) = r.read_bits(4) else { return };
                if id <= MAX_VPS_ID {
                    self.vps.insert(id, trimmed(rbsp));
                }
            }
            SPS_NUT => {
                let (Ok(vps_id), Ok(max_sub_layers_minus1), Ok(_nesting)) = (r.read_bits(4), r.read_bits(3), r.read_flag()) else { return };
                if skip_profile_tier_level(&mut r, max_sub_layers_minus1).is_none() {
                    return;
                }
                let Ok(id) = r.read_ue() else { return };
                if id <= MAX_SPS_ID {
                    self.sps.insert(id, (vps_id, trimmed(rbsp)));
                }
            }
            PPS_NUT => {
                let (Ok(id), Ok(sps_id)) = (r.read_ue(), r.read_ue()) else { return };
                // dependent_slice_segments_enabled_flag, output_flag_present_flag
                let (Ok(_), Ok(_), Ok(extra)) = (r.read_flag(), r.read_flag(), r.read_bits(3)) else { return };
                if id <= MAX_PPS_ID && sps_id <= MAX_SPS_ID {
                    self.pps.insert(id, (sps_id, extra as u8, trimmed(rbsp)));
                }
            }
            _ => {}
        }
    }

    /// The configuration a slice with this PPS id is decoded with.
    fn active(&self, pps_id: u32) -> Option<Config> {
        let (sps_id, _, pps) = self.pps.get(&pps_id)?;
        let (vps_id, sps) = self.sps.get(sps_id)?;
        Some((self.vps.get(vps_id)?.clone(), sps.clone(), pps.clone()))
    }
}

/// `pps_id` and `slice_type` (0 B, 1 P, 2 I) of the first slice segment of a picture; `None` when
/// the NAL unit is not a first segment. `slice_type` is `None` when its PPS is unknown.
fn first_segment(nal: &[u8], nal_type: u8, params: &Params) -> Option<(u32, Option<u32>)> {
    let head = nal.get(2..nal.len().min(2 + SLICE_HEAD))?;
    let rbsp = unescape_rbsp(head);
    let mut r = BitReader::new(&rbsp);
    if !r.read_flag().ok()? {
        return None;
    }
    if (BLA_W_LP..=IRAP_LAST).contains(&nal_type) {
        r.skip(1).ok()?; // no_output_of_prior_pics_flag
    }
    let pps_id = r.read_ue().ok()?;
    // a first slice segment is never dependent: no dependent_slice_segment_flag or address
    let slice_type = params.pps.get(&pps_id).and_then(|&(_, extra, _)| {
        r.skip(usize::from(extra)).ok()?;
        r.read_ue().ok()
    });
    Some((pps_id, slice_type))
}

#[derive(Default)]
struct Picture {
    /// NAL unit type of the picture's first slice segment.
    nal_type: Option<u8>,
    /// `slice_type` of the first slice segment.
    slice_type: Option<u32>,
    reference: bool,
    slices: usize,
    config: Option<Config>,
}

fn analyse(data: &[u8], format: NalFormat, params: &mut Params) -> Picture {
    let mut pic = Picture::default();
    for nal in nal_units(data, format) {
        let [h0, h1, ..] = *nal else { continue };
        let ty = (h0 >> 1) & 0x3F;
        let layer = (u16::from(h0 & 1) << 5) | u16::from(h1 >> 3);
        if layer != 0 {
            continue;
        }
        match ty {
            0..=IRAP_LAST if ty <= RASL_R || (BLA_W_LP..=CRA_NUT).contains(&ty) => {
                pic.slices += 1;
                // sub-layer non-reference pictures have even types up to 14
                pic.reference |= !(ty <= 14 && ty % 2 == 0);
                if let Some((pps_id, slice_type)) = first_segment(nal, ty, params)
                    && pic.nal_type.is_none()
                {
                    pic.nal_type = Some(ty);
                    pic.slice_type = slice_type;
                    pic.config = params.active(pps_id);
                }
            }
            VPS_NUT | SPS_NUT | PPS_NUT => params.record(ty, nal),
            _ => {}
        }
    }
    pic
}

/// Index an HEVC stream from its access units in decode order. `parameter_sets` are the VPS / SPS /
/// PPS NAL units of the container's codec configuration (`hvcC`), if any.
pub fn index_hevc<'a>(
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
            params.record((h >> 1) & 0x3F, ps);
        }
    }
    let mut frames: Vec<FrameIndexEntry> = Vec::new();
    let mut config_id = 0u32;
    let mut config_changes = Vec::new();
    let mut config: Option<Config> = None;
    // the latest IRAP picture: (frame position, whether pictures before it give its RASL pictures
    // their references)
    let mut irap: Option<(usize, bool)> = None;
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
        let ty = pic.nal_type;
        let is_irap = ty.is_some_and(|t| (BLA_W_LP..=IRAP_LAST).contains(&t));
        let picture_type = match (is_irap, pic.slice_type) {
            (true, _) | (false, Some(2)) => PictureType::I,
            (false, Some(1)) => PictureType::P,
            (false, Some(0)) => PictureType::B,
            _ => PictureType::Unknown,
        };
        // the RASL pictures that follow decide whether a CRA / BLA is clean (set below)
        let random_access = match ty {
            Some(IDR_W_RADL | IDR_N_LP) => RandomAccessKind::Idr,
            Some(CRA_NUT) => RandomAccessKind::Cra { rasl: false },
            Some(BLA_W_LP..=BLA_N_LP) => RandomAccessKind::Bla { rasl: false },
            Some(_) if picture_type == PictureType::I => RandomAccessKind::IntraOnly,
            _ => RandomAccessKind::None,
        };
        let dependency_status = if pic.slices == 0 || ty.is_none() {
            DependencyStatus::Unknown
        } else if is_irap {
            DependencyStatus::Independent
        } else {
            match (ty, irap) {
                (_, None) => DependencyStatus::Undecodable,
                (Some(RASL_N | RASL_R), Some((at, history))) => {
                    // the IRAP picture this RASL picture leads: it is not clean
                    let mut bla = false;
                    if let Some(head) = frames.get_mut(at) {
                        bla = matches!(head.random_access, RandomAccessKind::Bla { .. });
                        head.random_access = match head.random_access {
                            RandomAccessKind::Cra { .. } => RandomAccessKind::Cra { rasl: true },
                            RandomAccessKind::Bla { .. } => RandomAccessKind::Bla { rasl: true },
                            other => other,
                        };
                    }
                    if bla || !history { DependencyStatus::Undecodable } else { DependencyStatus::CrossesAccessPoint }
                }
                // RADL and trailing pictures refer to their IRAP picture and what follows it
                (Some(_), Some(_)) => DependencyStatus::WithinGop,
                (None, Some(_)) => DependencyStatus::Unknown,
            }
        };
        if is_irap {
            // a CRA's RASL pictures need the pictures decoded before it
            irap = Some((frames.len(), irap.is_some()));
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
    Ok(StreamIndex { stream_id, codec: Codec::Hevc, timebase, frames, gops, config_changes, timestamp_problems })
}
