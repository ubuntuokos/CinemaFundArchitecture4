//! The parts of a JPEG file the gain map reader needs: the application segments before the first
//! scan (ITU-T T.81 B.1.1: every marker segment up to SOS carries a 16-bit big-endian length), the
//! JPEG Multi-Picture Format index (CIPA DC-007-2009, 5.2: an APP2 segment `MPF\0` holding a TIFF
//! header and the MP Index IFD), and the end of the first picture.

/// The APP1 identifier of an XMP packet (XMP Specification Part 3, 1.1.3).
const XMP_ID: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
/// The APP2 identifier of an MPF segment (CIPA DC-007, 5.2.1).
const MPF_ID: &[u8] = b"MPF\0";
/// The APP2 identifier of an ISO 21496-1 gain map metadata segment.
pub const ISO_ID: &[u8] = b"urn:iso:std:iso:ts:21496:-1\0";

/// The segments of one JPEG picture that matter here (each one's payload after its identifier).
#[derive(Default, Debug)]
pub struct Segments<'a> {
    pub xmp: Option<&'a [u8]>,
    /// The MPF payload (from its TIFF header on) and its offset in the picture.
    pub mpf: Option<(usize, &'a [u8])>,
    pub iso: Option<&'a [u8]>,
    /// Where the first scan starts (the SOS marker).
    pub sos: Option<usize>,
}

/// Read the marker segments of the JPEG picture `b` up to its first scan. Damaged or truncated
/// headers end the walk; what was read up to there is returned.
pub fn segments(b: &[u8]) -> Segments<'_> {
    let mut s = Segments::default();
    if !b.starts_with(&[0xFF, 0xD8]) {
        return s;
    }
    let mut i = 2;
    while i + 4 <= b.len() {
        if b[i] != 0xFF {
            break;
        }
        let m = b[i + 1];
        if m == 0xFF {
            i += 1; // fill byte
            continue;
        }
        if m == 0xDA {
            s.sos = Some(i);
            break;
        }
        if m == 0xD8 || (0xD0..=0xD7).contains(&m) || m == 0x01 {
            i += 2; // markers without a length
            continue;
        }
        let len = usize::from(u16::from_be_bytes([b[i + 2], b[i + 3]]));
        if len < 2 || i + 2 + len > b.len() {
            break;
        }
        let body = &b[i + 4..i + 2 + len];
        match m {
            0xE1 if body.starts_with(XMP_ID) && s.xmp.is_none() => s.xmp = Some(&body[XMP_ID.len()..]),
            0xE2 if body.starts_with(MPF_ID) && s.mpf.is_none() => s.mpf = Some((i + 4 + MPF_ID.len(), &body[MPF_ID.len()..])),
            0xE2 if body.starts_with(ISO_ID) && s.iso.is_none() => s.iso = Some(&body[ISO_ID.len()..]),
            _ => {}
        }
        i += 2 + len;
    }
    s
}

/// Where the first picture of `b` ends (just after its EOI marker). Entropy-coded data never holds
/// `FF D9` (T.81 B.1.1.5: an `FF` data byte is followed by a stuffed zero), so the first EOI after the
/// first scan ends the picture.
pub fn picture_end(b: &[u8], sos: usize) -> Option<usize> {
    let tail = b.get(sos..)?;
    tail.windows(2).position(|w| w == [0xFF, 0xD9]).map(|p| sos + p + 2)
}

/// One image of an MP Index IFD (CIPA DC-007, 5.2.3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MpEntry {
    /// The Individual Image Attribute: flags (bits 31..27), image data format (26..24, 0 = JPEG)
    /// and the MP type code (23..0).
    pub attribute: u32,
    /// Where the image starts in the file and its length in bytes.
    pub offset: usize,
    pub size: usize,
}

/// MP type code of the primary image of a multi-frame stereo or disparity file (CIPA DC-007 Table 4).
pub const MP_TYPE_DISPARITY: u32 = 0x02_0002;

impl MpEntry {
    pub fn type_code(&self) -> u32 {
        self.attribute & 0x00FF_FFFF
    }
    /// Whether the image is a JPEG (image data format 0).
    pub fn is_jpeg(&self) -> bool {
        (self.attribute >> 24) & 7 == 0
    }
}

/// Upper bound on the images an MP index lists that we look at (real files hold two to a few dozen).
const MAX_ENTRIES: usize = 256;

/// The images of an MPF segment whose TIFF header starts at `base` in the file of length `file_len`.
/// Offsets are relative to that TIFF header, the first image's offset is 0 and means the start of
/// the file (CIPA DC-007, 5.2.3.3.3). Entries outside the file are dropped; a damaged index gives
/// `None`.
pub fn mp_entries(base: usize, mpf: &[u8], file_len: usize) -> Option<Vec<MpEntry>> {
    let le = match mpf.get(..4)? {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return None,
    };
    let u16_at = |o: usize| -> Option<u16> {
        let b: [u8; 2] = mpf.get(o..o + 2)?.try_into().ok()?;
        Some(if le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    };
    let u32_at = |o: usize| -> Option<u32> {
        let b: [u8; 4] = mpf.get(o..o + 4)?.try_into().ok()?;
        Some(if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    };
    let ifd = usize::try_from(u32_at(4)?).ok()?;
    let count = usize::from(u16_at(ifd)?);
    let (mut n, mut entries_at) = (None, None);
    for k in 0..count.min(64) {
        let e = ifd + 2 + 12 * k;
        let (tag, kind, cnt) = (u16_at(e)?, u16_at(e + 2)?, u32_at(e + 4)?);
        match tag {
            0xB001 if kind == 4 => n = Some(u32_at(e + 8)? as usize),
            // MP Entry: UNDEFINED, 16 bytes per image, always stored at an offset
            0xB002 if kind == 7 && cnt >= 16 => entries_at = Some((u32_at(e + 8)? as usize, cnt as usize / 16)),
            _ => {}
        }
    }
    let (at, listed) = entries_at?;
    let n = n.unwrap_or(listed).min(listed).min(MAX_ENTRIES);
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let e = at + 16 * k;
        let (attribute, size, offset) = (u32_at(e)?, u32_at(e + 4)? as usize, u32_at(e + 8)? as usize);
        let offset = if k == 0 && offset == 0 { 0 } else { base.checked_add(offset)? };
        if size == 0 || offset.checked_add(size).is_none_or(|end| end > file_len) {
            continue;
        }
        out.push(MpEntry { attribute, offset, size });
    }
    Some(out)
}
