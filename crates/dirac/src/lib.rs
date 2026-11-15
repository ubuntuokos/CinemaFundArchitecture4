// Ported from the SMPTE ST 2042-1 pseudocode of the BBC VC-2 conformance software
// (vc2_conformance 2.0.0, Copyright British Broadcasting Corporation, GPL-3.0-only); see
// docs/licensing.md §5.

//! Pure-Rust decoder for the Dirac video codec and SMPTE ST 2042-1 VC-2, its intra-only,
//! wavelet-based professional subset.
//!
//! - VC-2: low-delay and high-quality pictures, every wavelet filter of the standard, symmetric
//!   and asymmetric transforms, custom and default quantisation matrices, 4:4:4 / 4:2:2 / 4:2:0
//!   at up to 16 bits, frame and field coding. Ported from the normative pseudocode of ST 2042-1
//!   as published in the BBC VC-2 conformance software (`vc2_conformance` 2.0.0 and
//!   `vc2_data_tables`, GPL-3.0-only).
//! - Dirac core syntax: intra and inter pictures with arithmetic or exp-Golomb coded
//!   coefficients, one or two references, overlapped block motion compensation at every
//!   motion vector precision, global motion, reference weights. Ported from FFmpeg n7.1.1
//!   (`dirac_arith.c`, `diracdec.c`, `diracdsp.c`, LGPL-2.1-or-later, `COPYING.LGPL-2.1`).
//!   Pictures come out in decode order with their picture numbers.
//!
//! Rejected with [`Error::Unsupported`]: VC-2 picture fragments, Dirac low-delay pictures,
//! Dirac core field pictures, and inter pictures deeper than 8 bits or without arithmetic
//! coding (as in FFmpeg).

mod arith;
mod bits;
mod core;
mod header;
mod picture;
mod tables;

pub use header::{ChromaFormat, Sequence, VideoParameters};

/// The prefix of every parse info header (`BBCD`).
const PREFIX: &[u8; 4] = b"BBCD";
const PARSE_INFO_BYTES: usize = 13;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("VC-2 stream truncated")]
    Truncated,
    #[error("invalid VC-2 stream: {0}")]
    Invalid(&'static str),
    #[error("unsupported VC-2 / Dirac feature: {0}")]
    Unsupported(&'static str),
    #[error("VC-2 picture before any sequence header")]
    NoSequenceHeader,
}

pub type Result<T> = std::result::Result<T, Error>;

/// A decoded frame: luma and two colour difference planes (`C1` = Cb, `C2` = Cr), unsigned with
/// the coded signal offsets (e.g. 16–235 luma for 8-bit video range).
#[derive(Clone, Debug)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub chroma: ChromaFormat,
    pub luma_depth: u32,
    pub color_diff_depth: u32,
    /// The picture number of the frame (of its first field when fields are coded).
    pub picture_number: u32,
    /// Y (`width` x `height`), then C1 and C2 at the chroma format's size.
    pub planes: [Vec<u16>; 3],
}

/// One data unit of a stream.
struct Unit<'a> {
    parse_code: u8,
    next: usize,
    body: &'a [u8],
}

fn unit(data: &[u8], at: usize) -> Option<Unit<'_>> {
    let h = data.get(at..at.checked_add(PARSE_INFO_BYTES)?)?;
    if h.get(..4)? != PREFIX {
        return None;
    }
    let next = u32::from_be_bytes(h.get(5..9)?.try_into().ok()?) as usize;
    Some(Unit { parse_code: *h.get(4)?, next, body: data.get(at + PARSE_INFO_BYTES..)? })
}

fn is_picture(parse_code: u8) -> bool {
    parse_code & 0x8C == 0x88
}

/// The first sequence header in `data` (a sample or the start of a stream).
pub fn sequence_header(data: &[u8]) -> Option<Sequence> {
    let mut at = 0;
    for _ in 0..64 {
        let u = unit(data, at)?;
        if u.parse_code == 0x00 {
            return header::parse(u.body).ok();
        }
        if u.next < PARSE_INFO_BYTES {
            return None;
        }
        at = at.checked_add(u.next)?;
    }
    None
}

/// Does `data` start with a VC-2 / Dirac parse info header?
pub fn is_vc2(data: &[u8]) -> bool {
    data.starts_with(PREFIX)
}

/// The parse codes of the pictures in `data` (a sample), up to 64 data units.
fn picture_codes(data: &[u8]) -> Vec<u8> {
    let mut codes = Vec::new();
    let mut at = 0usize;
    for _ in 0..64 {
        let Some(u) = unit(data, at) else { break };
        if u.parse_code & 0x08 == 0x08 {
            codes.push(u.parse_code);
        }
        if u.next < PARSE_INFO_BYTES {
            break;
        }
        at = at.saturating_add(u.next);
    }
    codes
}

/// Can decoding start at `data` (a sample)? Its first picture is a VC-2 picture or a Dirac intra
/// reference picture.
pub fn is_random_access(data: &[u8]) -> bool {
    picture_codes(data).first().is_some_and(|&c| is_picture(c) || c & 0x8F == 0x0C)
}

/// Does `data` (a sample) hold pictures, none of them a reference for other pictures?
pub fn is_non_reference(data: &[u8]) -> bool {
    let codes = picture_codes(data);
    !codes.is_empty() && codes.iter().all(|&c| c & 0x04 == 0 || is_picture(c))
}

/// Does `data` (a sample) hold a Dirac core syntax picture (coded in decode order, which may
/// differ from display order)? VC-2 pictures come in display order.
pub fn is_core_syntax(data: &[u8]) -> bool {
    picture_codes(data).iter().any(|&c| c & 0x88 == 0x08)
}

/// Does `data` (a sample) hold a Dirac inter picture (one predicted from other pictures)?
pub fn has_inter_picture(data: &[u8]) -> bool {
    picture_codes(data).iter().any(|&c| c & 0x88 == 0x08 && c & 0x03 != 0)
}

/// A first field waiting for its second.
struct Pending {
    picture_number: u32,
    planes: [Vec<u16>; 3],
}

/// Reference pictures held at most (FFmpeg's `MAX_REFERENCE_FRAMES`).
const MAX_REFERENCES: usize = 8;

#[derive(Default)]
pub struct Decoder {
    seq: Option<Sequence>,
    pending: Option<Pending>,
    refs: Vec<core::Reference>,
    old_delta_quant: bool,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// The current sequence header.
    pub fn sequence(&self) -> Option<&Sequence> {
        self.seq.as_ref()
    }

    /// Forget a first field waiting for its second and the reference pictures (after a seek).
    pub fn reset(&mut self) {
        self.pending = None;
        self.refs.clear();
    }

    /// Decode the data units in `data` (one or more, e.g. a container sample: a sequence header
    /// and a picture). Returns the frames completed.
    pub fn decode(&mut self, data: &[u8]) -> Result<Vec<Picture>> {
        let mut out = Vec::new();
        let mut at = 0usize;
        let mut units = 0;
        while at < data.len() {
            let Some(u) = unit(data, at) else {
                if units == 0 {
                    return Err(Error::Invalid("no parse info header"));
                }
                break;
            };
            units += 1;
            if u.next != 0 && u.next < PARSE_INFO_BYTES {
                return Err(Error::Invalid("next parse offset"));
            }
            let mut step = u.next;
            match u.parse_code {
                0x00 => self.seq = Some(header::parse(u.body)?),
                0x10 => {
                    step = PARSE_INFO_BYTES;
                    self.refs.clear();
                }
                0x20 => {
                    // encoder name: Schrödinger before 1.0.8 codes quantiser deltas differently
                    if let Some(rest) = u.body.strip_prefix(&[1u8][..]).and_then(|r| r.strip_prefix(b"Schroedinger 1.0.")) {
                        let digits: Vec<u8> = rest.iter().copied().take_while(u8::is_ascii_digit).collect();
                        let minor = std::str::from_utf8(&digits).ok().and_then(|d| d.parse::<u32>().ok());
                        self.old_delta_quant = minor.is_some_and(|m| m <= 7);
                    }
                }
                c if c & 0xF8 == 0x20 || c & 0xF8 == 0x30 => {}
                0xC8 | 0xE8 => {
                    let seq = self.seq.as_ref().ok_or(Error::NoSequenceHeader)?;
                    let (pic, used) = picture::decode(seq, u.parse_code == 0xC8, u.body)?;
                    if step == 0 {
                        step = PARSE_INFO_BYTES + used;
                    }
                    if let Some(frame) = self.frame(pic) {
                        out.push(frame);
                    }
                }
                0xCC | 0xEC => return Err(Error::Unsupported("VC-2 picture fragments")),
                c if c & 0x88 == 0x08 => {
                    let seq = self.seq.as_ref().ok_or(Error::NoSequenceHeader)?;
                    if seq.fields {
                        return Err(Error::Unsupported("Dirac core syntax field pictures"));
                    }
                    let kind = core::Kind::new(c, self.old_delta_quant);
                    let (hdr, planes, used) = core::decode(seq, kind, u.body, &mut self.refs)?;
                    if step == 0 {
                        step = PARSE_INFO_BYTES + used;
                    }
                    if kind.reference {
                        if let Some(r) = hdr.retire.filter(|&r| r != hdr.number) {
                            self.refs.retain(|p| p.number != r);
                        }
                        if self.refs.len() >= MAX_REFERENCES {
                            self.refs.remove(0);
                        }
                        self.refs.push(core::Reference::new(hdr.number, &planes));
                    }
                    let v = &seq.video;
                    out.push(Picture {
                        width: v.frame_width,
                        height: v.frame_height,
                        chroma: v.chroma,
                        luma_depth: seq.luma_depth,
                        color_diff_depth: seq.color_diff_depth,
                        picture_number: hdr.number,
                        planes,
                    });
                }
                c if is_picture(c) || c & 0x08 == 0x08 => return Err(Error::Unsupported("Dirac low-delay pictures")),
                _ => return Err(Error::Invalid("unknown parse code")),
            }
            if step == 0 {
                break;
            }
            at = at.saturating_add(step);
        }
        Ok(out)
    }

    /// A frame from a decoded picture: the picture itself, or two fields woven together.
    fn frame(&mut self, pic: picture::Components) -> Option<Picture> {
        let seq = self.seq.as_ref()?;
        let v = &seq.video;
        let make = |planes, picture_number| Picture {
            width: v.frame_width,
            height: v.frame_height,
            chroma: v.chroma,
            luma_depth: seq.luma_depth,
            color_diff_depth: seq.color_diff_depth,
            picture_number,
            planes,
        };
        if !seq.fields {
            return Some(make(pic.planes, pic.picture_number));
        }
        // The earlier field of a frame has an even picture number (§12.2).
        if pic.picture_number.is_multiple_of(2) {
            self.pending = Some(Pending { picture_number: pic.picture_number, planes: pic.planes });
            return None;
        }
        let first = self.pending.take().filter(|p| p.picture_number.wrapping_add(1) == pic.picture_number)?;
        let (top, bottom) = if v.top_field_first { (first.planes, pic.planes) } else { (pic.planes, first.planes) };
        let widths = [seq.luma_width, seq.color_diff_width, seq.color_diff_width];
        let mut planes: [Vec<u16>; 3] = Default::default();
        for (c, plane) in planes.iter_mut().enumerate() {
            let w = widths[c] as usize;
            let (t, b) = (&top[c], &bottom[c]);
            plane.reserve(t.len() + b.len());
            for (rt, rb) in t.chunks_exact(w.max(1)).zip(b.chunks_exact(w.max(1))) {
                plane.extend_from_slice(rt);
                plane.extend_from_slice(rb);
            }
        }
        Some(make(planes, first.picture_number))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garbage_is_an_error() {
        let mut d = Decoder::new();
        assert!(d.decode(b"not a stream").is_err());
        assert!(d.decode(&[]).unwrap().is_empty());
        // a picture before any sequence header
        let mut pic = b"BBCD\xE8\0\0\0\0\0\0\0\0".to_vec();
        pic.extend_from_slice(&[0; 16]);
        assert_eq!(d.decode(&pic).unwrap_err(), Error::NoSequenceHeader);
        // a Dirac core syntax picture, likewise
        let mut core = b"BBCD\x0C\0\0\0\0\0\0\0\0".to_vec();
        core.extend_from_slice(&[0; 16]);
        assert_eq!(d.decode(&core).unwrap_err(), Error::NoSequenceHeader);
    }
}
