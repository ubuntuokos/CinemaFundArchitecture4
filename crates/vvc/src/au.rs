//! Access units (H.266 § 7.4.2.4.3) and the picture-level facts an editor needs from them:
//! where decoding can start and which pictures nothing else refers to.

use std::ops::Range;

use cfa4_bitstream::annexb_nals;

use crate::nal::{HEADER_LEN, NalHeader, NalType};

/// What one access unit holds, from its NAL unit headers and picture headers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AccessUnitInfo {
    /// Coded pictures (one per layer) whose first VCL NAL unit was seen.
    pub pictures: usize,
    /// Every picture is IRAP (IDR or CRA): decoding can start here.
    pub irap: bool,
    /// Every picture is IDR: no picture before this access unit is referenced (closed GOP).
    pub idr: bool,
    /// A picture is a GDR picture (gradual decoding refresh: clean only after its recovery point).
    pub gdr: bool,
    /// Every picture has `ph_non_ref_pic_flag`: dropping the access unit changes no other picture.
    pub non_ref: bool,
    /// The access unit carries a sequence parameter set.
    pub has_sps: bool,
}

/// The first two bits of `picture_header_structure()` (§ 7.3.2.8):
/// `(ph_gdr_or_irap_pic_flag, ph_non_ref_pic_flag)`.
fn picture_header_flags(byte: u8) -> (bool, bool) {
    (byte & 0x80 != 0, byte & 0x40 != 0)
}

/// For a VCL NAL unit: whether it is the first slice of a picture that carries its picture
/// header in the slice header (`sh_picture_header_in_slice_header_flag`, § 7.3.7.1), and then the
/// picture header flags. A picture with that flag has exactly one slice.
fn slice_picture_header(nal: &[u8]) -> Option<(bool, bool)> {
    // The flags sit in the first payload byte. No emulation prevention byte can precede it: the
    // second header byte is never zero (nuh_temporal_id_plus1 > 0).
    let b = *nal.get(HEADER_LEN)?;
    if b & 0x80 == 0 {
        return None;
    }
    Some(picture_header_flags(b << 1))
}

/// Splits a sequence of NAL units (decoding order, without start codes or lengths) into access
/// units: ranges of indices into `nals`. NAL units with a malformed header stay in the current
/// access unit. Multi-layer access units are kept together (a picture of a higher layer does not
/// start a new access unit).
pub fn access_units(nals: &[&[u8]]) -> Vec<Range<usize>> {
    let mut units: Vec<Range<usize>> = Vec::new();
    let mut start = 0usize;
    // Layer of the last VCL NAL unit in the current access unit (None: no VCL yet).
    let mut last_vcl_layer: Option<u8> = None;
    for (i, nal) in nals.iter().enumerate() {
        let Ok(h) = NalHeader::parse(nal) else { continue };
        let begins = match last_vcl_layer {
            None => false,
            Some(layer) => {
                if h.nal_type.is_vcl() {
                    // A new picture with its own picture header in a layer not above the last one.
                    slice_picture_header(nal).is_some() && h.layer_id <= layer
                } else if h.nal_type == NalType::Ph {
                    h.layer_id <= layer
                } else {
                    h.nal_type.starts_access_unit()
                }
            }
        };
        if begins && i > start {
            units.push(start..i);
            start = i;
            last_vcl_layer = None;
        }
        if h.nal_type.is_vcl() {
            last_vcl_layer = Some(h.layer_id);
        }
    }
    if start < nals.len() {
        units.push(start..nals.len());
    }
    units
}

/// Splits an Annex B byte stream into access units: `(offset, length)` byte ranges that cover
/// the whole input (leading bytes go to the first unit). An input without NAL units gives none.
pub fn annexb_access_units(data: &[u8]) -> Vec<(usize, usize)> {
    let nals = annexb_nals(data);
    let base = data.as_ptr() as usize;
    // Where the start code of NAL unit `k` begins (the zero_byte of a four-byte start code too).
    let code_start = |k: usize| -> usize {
        let off = nals[k].as_ptr() as usize - base;
        let mut s = off.saturating_sub(3);
        if s > 0 && data[s - 1] == 0 {
            s -= 1;
        }
        s
    };
    let ranges = access_units(&nals);
    let mut out = Vec::with_capacity(ranges.len());
    for (k, r) in ranges.iter().enumerate() {
        let begin = if k == 0 { 0 } else { code_start(r.start) };
        let end = match ranges.get(k + 1) {
            Some(next) => code_start(next.start),
            None => data.len(),
        };
        out.push((begin, end.saturating_sub(begin)));
    }
    out
}

/// What the access unit made of `nals` holds.
pub fn au_info(nals: &[&[u8]]) -> AccessUnitInfo {
    let mut info = AccessUnitInfo { irap: true, idr: true, non_ref: true, ..AccessUnitInfo::default() };
    // Picture header flags from a PH NAL unit, waiting for the picture's first slice.
    let mut pending_ph: Option<(bool, bool)> = None;
    let mut in_picture = false;
    for nal in nals {
        let Ok(h) = NalHeader::parse(nal) else { continue };
        let t = h.nal_type;
        if t == NalType::Sps {
            info.has_sps = true;
        }
        if t == NalType::Ph {
            pending_ph = nal.get(HEADER_LEN).map(|&b| picture_header_flags(b));
            in_picture = false;
            continue;
        }
        if !t.is_vcl() {
            continue;
        }
        let flags = match slice_picture_header(nal) {
            Some(f) => Some(f),
            None if !in_picture => pending_ph.take(),
            None => continue,
        };
        in_picture = true;
        let Some((_gdr_or_irap, non_ref)) = flags else {
            // A slice without a picture header we saw: count the picture, know nothing more.
            info.pictures += 1;
            info.non_ref = false;
            continue;
        };
        info.pictures += 1;
        info.irap &= t.is_irap();
        info.idr &= t.is_idr();
        info.gdr |= t == NalType::Gdr;
        info.non_ref &= non_ref;
    }
    if info.pictures == 0 {
        info.irap = false;
        info.idr = false;
        info.non_ref = false;
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nal(t: u8, layer: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![layer & 0x3f, (t << 3) | 1];
        v.extend_from_slice(payload);
        v
    }
    /// A slice with its picture header in the slice header.
    fn slice(t: u8, non_ref: bool) -> Vec<u8> {
        let gdr_or_irap = matches!(t, 7..=10);
        nal(t, 0, &[0x80 | (u8::from(gdr_or_irap) << 6) | (u8::from(non_ref) << 5), 0x80])
    }

    #[test]
    fn units_split_at_parameter_sets_and_new_pictures() {
        let sps = nal(15, 0, &[0x00]);
        let pps = nal(16, 0, &[0x00]);
        let idr = slice(7, false);
        let suffix = nal(24, 0, &[0x00]);
        let trail = slice(0, true);
        let aps = nal(17, 0, &[0x00]);
        let list: Vec<&[u8]> = vec![&sps, &pps, &idr, &suffix, &trail, &aps, &trail];
        assert_eq!(access_units(&list), vec![0..4, 4..5, 5..7]);

        let first = au_info(&list[0..4]);
        assert!(first.irap && first.idr && first.has_sps && !first.non_ref && first.pictures == 1);
        let second = au_info(&list[4..5]);
        assert!(!second.irap && second.non_ref);
    }

    #[test]
    fn picture_header_nal_and_multi_slice_pictures() {
        // PH NAL unit (gdr_or_irap = 1, non_ref = 0), then two CRA slices without picture header.
        let ph = nal(19, 0, &[0x80]);
        let s = nal(9, 0, &[0x00, 0x80]);
        let ph2 = nal(19, 0, &[0x40]);
        let t = nal(0, 0, &[0x00, 0x80]);
        let list: Vec<&[u8]> = vec![&ph, &s, &s, &ph2, &t, &t];
        assert_eq!(access_units(&list), vec![0..3, 3..6]);
        let a = au_info(&list[0..3]);
        assert!(a.irap && !a.idr && a.pictures == 1 && !a.non_ref);
        let b = au_info(&list[3..6]);
        assert!(!b.irap && b.non_ref && b.pictures == 1);
    }

    #[test]
    fn higher_layer_picture_stays_in_the_unit() {
        let base = slice(7, false);
        let mut enh = slice(7, false);
        enh[0] = 1;
        let next = slice(0, false);
        let list: Vec<&[u8]> = vec![&base, &enh, &next];
        assert_eq!(access_units(&list), vec![0..2, 2..3]);
        assert_eq!(au_info(&list[0..2]).pictures, 2);
    }

    #[test]
    fn annexb_ranges_cover_the_input() {
        let mut data = vec![0u8, 0, 0, 1];
        data.extend(nal(15, 0, &[0x11]));
        data.extend([0, 0, 1]);
        data.extend(slice(8, false));
        data.extend([0, 0, 0, 1]);
        data.extend(slice(0, false));
        let units = annexb_access_units(&data);
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].0, 0);
        assert_eq!(units[0].0 + units[0].1, units[1].0);
        assert_eq!(units[1].0 + units[1].1, data.len());
        assert_eq!(&data[units[1].0..units[1].0 + 4], &[0, 0, 0, 1]);
        assert!(annexb_access_units(&[]).is_empty());
        assert!(annexb_access_units(&[0, 0, 0]).is_empty());
    }

    #[test]
    fn units_without_pictures_are_not_random_access() {
        let sps = nal(15, 0, &[0x00]);
        let info = au_info(&[&sps]);
        assert!(!info.irap && !info.non_ref && info.has_sps && info.pictures == 0);
    }
}
