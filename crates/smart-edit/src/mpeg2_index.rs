//! MPEG-2 (and MPEG-1) video index: sequence headers, GOP headers (`closed_gop`, `broken_link`),
//! picture types and the anchor references of every picture (ISO/IEC 13818-2 §6.1.1.11, §7.6).
//!
//! Each [`Packet`] is one access unit as `cfa4_mpeg2v::access_units` (or a container) splits it: a
//! frame picture or a field pair, with the headers before it.
//!
//! References are exact for MPEG-2: a P picture predicts from the latest anchor (I or P picture)
//! in decode order, a B picture from the two latest ones, and a leading B picture of a closed GOP
//! (or of a broken link) only from the anchor after it.

use cfa4_mpeg2v::{GopHeader, PictureHeader, PictureType as Mpeg2Type};

use crate::capabilities::Codec;
use crate::media_index::{
    DependencyStatus, FrameIndexEntry, IndexError, MAX_FRAMES, Packet, PictureType, RandomAccessKind, StreamIndex, gops, presentation_from_pts,
    timestamp_problems,
};
use crate::timestamp::Rational;

const SEQUENCE_HEADER: u8 = 0xB3;
const PICTURE_START: u8 = 0x00;
const GROUP_START: u8 = 0xB8;

/// What one access unit holds, from the headers alone.
struct Scan {
    ptype: PictureType,
    sequence_header: Option<Vec<u8>>,
    gop: Option<(bool, bool)>,
    temporal_reference: Option<u16>,
}

/// Start codes (`00 00 01 xx`: position, code) of one packet, found lazily with the rules of
/// `cfa4_mpeg2v::start_codes`, so a packet full of start codes costs no memory per code.
struct StartCodes<'a> {
    data: &'a [u8],
    i: usize,
}

impl Iterator for StartCodes<'_> {
    type Item = (usize, u8);

    fn next(&mut self) -> Option<(usize, u8)> {
        loop {
            let i = self.i;
            let w = self.data.get(i..i.checked_add(4)?)?;
            if w.get(2).is_some_and(|&b| b > 1) {
                self.i = i.checked_add(3)?;
            } else if let [0, 0, 1, code] = *w {
                self.i = i.checked_add(4)?;
                return Some((i, code));
            } else {
                self.i = i.checked_add(1)?;
            }
        }
    }
}

/// The headers of one access unit (what `cfa4_mpeg2v::scan_access_unit` reads, without keeping
/// every picture header: only the first picture counts here).
fn scan_headers(data: &[u8]) -> (bool, Option<GopHeader>, Option<PictureHeader>) {
    let mut codes = StartCodes { data, i: 0 }.peekable();
    let mut sequence_header = false;
    let mut gop = None;
    let mut first_picture: Option<PictureHeader> = None;
    while let Some((pos, code)) = codes.next() {
        let end = codes.peek().map_or(data.len(), |c| c.0);
        let payload = data.get(pos.saturating_add(4).min(end)..end).unwrap_or_default();
        match code {
            SEQUENCE_HEADER => sequence_header = true,
            GROUP_START => gop = GopHeader::parse(payload).ok(),
            PICTURE_START if first_picture.is_none() => first_picture = PictureHeader::parse(payload).ok(),
            _ => {}
        }
    }
    (sequence_header, gop, first_picture)
}

fn scan(data: &[u8]) -> Scan {
    let (sequence_header, gop, first_picture) = scan_headers(data);
    let ptype = match first_picture.as_ref().map(|p| p.coding_type) {
        Some(Mpeg2Type::I) => PictureType::I,
        Some(Mpeg2Type::P) => PictureType::P,
        Some(Mpeg2Type::B) => PictureType::B,
        Some(Mpeg2Type::D) => PictureType::D,
        None => PictureType::Unknown,
    };
    Scan {
        ptype,
        sequence_header: sequence_header.then(|| sequence_header_bytes(data)).flatten(),
        gop: gop.map(|g| (g.closed_gop, g.broken_link)),
        temporal_reference: first_picture.map(|p| p.temporal_reference),
    }
}

/// What the scanner reads from one packet (tests compare it with `cfa4_mpeg2v::scan_access_unit`).
#[cfg(test)]
pub(crate) fn scan_summary(data: &[u8]) -> (bool, Option<GopHeader>, Option<PictureHeader>) {
    scan_headers(data)
}

/// The bytes of the first sequence header (start code to the next start code), which together
/// with its extensions define the codec configuration.
fn sequence_header_bytes(data: &[u8]) -> Option<Vec<u8>> {
    let start = data.windows(4).position(|w| w == [0, 0, 1, SEQUENCE_HEADER])?;
    let body = data.get(start + 4..)?;
    let end = body.windows(3).position(|w| w == [0, 0, 1]).map_or(body.len(), |e| e);
    // the sequence extension (and display extension) follow as their own start codes; take them
    // with the header: they change the configuration as much as the header does
    let mut stop = start + 4 + end;
    while let Some(rest) = data.get(stop..) {
        if rest.len() >= 4 && rest.get(..3) == Some(&[0, 0, 1]) && rest.get(3) == Some(&0xB5) {
            let next = rest.get(4..).and_then(|r| r.windows(3).position(|w| w == [0, 0, 1])).map_or(rest.len(), |e| e + 4);
            stop += next;
        } else {
            break;
        }
    }
    data.get(start..stop).map(<[u8]>::to_vec)
}

/// Index an MPEG-1/2 video stream from its access units in decode order.
pub fn index_mpeg2<'a>(stream_id: u32, timebase: Rational, packets: impl IntoIterator<Item = Packet<'a>>) -> Result<StreamIndex, IndexError> {
    if !timebase.is_valid() {
        return Err(IndexError::InvalidTimebase(timebase));
    }
    let mut scans = Vec::new();
    let mut meta = Vec::new();
    for (i, p) in packets.into_iter().enumerate() {
        if i >= MAX_FRAMES {
            return Err(IndexError::TooManyFrames);
        }
        let size = u32::try_from(p.data.len()).map_err(|_| IndexError::PacketTooLarge { decode_index: i as u64 })?;
        scans.push(scan(p.data));
        meta.push((p.pts, p.dts, p.keyframe, size));
    }
    let n = scans.len();

    // leading B pictures: the B pictures right after an intra picture, before the next anchor
    let has_leading: Vec<bool> =
        (0..n).map(|i| scans.get(i).is_some_and(|s| s.ptype.is_intra()) && scans.get(i + 1).is_some_and(|s| s.ptype == PictureType::B)).collect();

    let mut frames: Vec<FrameIndexEntry> = Vec::with_capacity(n);
    let mut config: Option<Vec<u8>> = None;
    let mut config_id = 0u32;
    let mut config_changes = Vec::new();
    // (decode index, status) of the two latest anchors
    let mut last: Option<(u64, DependencyStatus)> = None;
    let mut prev: Option<(u64, DependencyStatus)> = None;
    // the latest access point, or GOP header with closed_gop / broken_link: (decode index, closed
    // or no leading pictures, broken link)
    let mut ap: Option<(u64, bool, bool)> = None;
    let mut since_ap_anchor = false;

    for (i, (s, &(pts, dts, keyframe, size))) in scans.iter().zip(meta.iter()).enumerate() {
        let di = i as u64;
        if let Some(sh) = &s.sequence_header {
            match &config {
                Some(c) if c != sh => {
                    config_id = config_id.saturating_add(1);
                    config_changes.push(di);
                    config = Some(sh.clone());
                }
                None => config = Some(sh.clone()),
                _ => {}
            }
        }
        let (closed, broken) = s.gop.unwrap_or((false, false));
        let random_access = if s.ptype.is_intra() {
            if s.sequence_header.is_some() {
                if closed || !has_leading.get(i).copied().unwrap_or(false) {
                    RandomAccessKind::ClosedGop
                } else {
                    RandomAccessKind::OpenGop { broken_link: broken }
                }
            } else {
                RandomAccessKind::IntraOnly
            }
        } else {
            RandomAccessKind::None
        };
        let worst = |a: DependencyStatus, b: DependencyStatus| {
            if a == DependencyStatus::Undecodable || b == DependencyStatus::Undecodable { DependencyStatus::Undecodable } else { DependencyStatus::WithinGop }
        };

        let (references, dependency_status) = match s.ptype {
            PictureType::I | PictureType::D => (Vec::new(), DependencyStatus::Independent),
            PictureType::P => match last {
                Some((r, st)) => (vec![r], worst(st, DependencyStatus::WithinGop)),
                None => (Vec::new(), DependencyStatus::Undecodable),
            },
            PictureType::B => {
                let leading = ap.is_some_and(|(a, _, _)| !since_ap_anchor && last.is_some_and(|(l, _)| l == a));
                match (prev, last) {
                    // leading picture of a closed GOP: backward prediction only
                    (_, Some((l, st))) if leading && ap.is_some_and(|(_, c, _)| c) => (vec![l], worst(st, DependencyStatus::WithinGop)),
                    (_, Some((l, _))) if leading && ap.is_some_and(|(_, _, b)| b) => (vec![l], DependencyStatus::Undecodable),
                    (Some((p, ps)), Some((l, ls))) => {
                        let st = if leading {
                            if ps == DependencyStatus::Undecodable { DependencyStatus::Undecodable } else { DependencyStatus::CrossesAccessPoint }
                        } else {
                            worst(ps, ls)
                        };
                        (vec![p, l], st)
                    }
                    (None, Some((l, _))) => (vec![l], DependencyStatus::Undecodable),
                    _ => (Vec::new(), DependencyStatus::Undecodable),
                }
            }
            PictureType::Unknown => (Vec::new(), DependencyStatus::Unknown),
        };

        let reference = matches!(s.ptype, PictureType::I | PictureType::P);
        if random_access.is_access_point() {
            ap = Some((di, random_access.is_clean(), matches!(random_access, RandomAccessKind::OpenGop { broken_link: true })));
            since_ap_anchor = false;
        } else if s.ptype.is_intra() && (closed || broken) {
            // a GOP header without a repeated sequence header: no access point, but its flags
            // still say what the leading B pictures predict from
            ap = Some((di, closed, broken));
            since_ap_anchor = false;
        } else if reference {
            since_ap_anchor = true;
        }
        if reference {
            // an intra picture restarts the chain: its successors are decodable from it
            let st = if s.ptype.is_intra() { DependencyStatus::Independent } else { dependency_status };
            prev = last;
            last = Some((di, st));
        }
        frames.push(FrameIndexEntry {
            stream_id,
            decode_index: di,
            presentation_index: None,
            pts,
            dts,
            keyframe,
            picture_type: s.ptype,
            reference,
            random_access,
            dependency_status,
            references: Some(references),
            config_id,
            size,
        });
    }

    let pts: Vec<Option<i64>> = frames.iter().map(|f| f.pts).collect();
    let order = presentation_from_pts(&pts).or_else(|| presentation_from_temporal_reference(&scans));
    if let Some(order) = order {
        for (f, p) in frames.iter_mut().zip(order) {
            f.presentation_index = Some(p);
        }
    }
    let gops = gops(&frames);
    let timestamp_problems = timestamp_problems(&frames);
    Ok(StreamIndex { stream_id, codec: Codec::Mpeg2Video, timebase, frames, gops, config_changes, timestamp_problems })
}

/// Presentation order without timestamps: `temporal_reference` counts pictures in display order
/// from each GOP header. `None` unless the result is a permutation of 0..n.
fn presentation_from_temporal_reference(scans: &[Scan]) -> Option<Vec<u64>> {
    let mut out = Vec::with_capacity(scans.len());
    let mut base = 0u64;
    let mut group = 0u64;
    for (i, s) in scans.iter().enumerate() {
        if i > 0 && s.gop.is_some() {
            base = base.checked_add(group)?;
            group = 0;
        }
        let tr = u64::from(s.temporal_reference?);
        out.push(base.checked_add(tr)?);
        group = group.checked_add(1)?;
    }
    let mut sorted = out.clone();
    sorted.sort_unstable();
    sorted.iter().enumerate().all(|(i, &p)| p == i as u64).then_some(out)
}
