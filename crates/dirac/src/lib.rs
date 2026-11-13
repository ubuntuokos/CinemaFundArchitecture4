// Ported from the SMPTE ST 2042-1 pseudocode of the BBC VC-2 conformance software
// (vc2_conformance 2.0.0, Copyright British Broadcasting Corporation, GPL-3.0-only); see
// docs/licensing.md §5.

//! Pure-Rust decoder for SMPTE ST 2042-1 VC-2, the intra-only, wavelet-based professional subset
//! of the Dirac video codec: low-delay and high-quality pictures, every wavelet filter of the
//! standard, symmetric and asymmetric transforms, custom and default quantisation matrices, 4:4:4
//! / 4:2:2 / 4:2:0 at up to 16 bits, frame and field coding.
//!
//! Ported from the normative pseudocode of ST 2042-1 as published in the BBC VC-2 conformance
//! software (`vc2_conformance` 2.0.0 and `vc2_data_tables`, GPL-3.0-only). Dirac core syntax
//! (arithmetic coding, motion compensation) and VC-2 picture fragments are rejected with
//! [`Error::Unsupported`].

mod bits;
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

/// A first field waiting for its second.
struct Pending {
    picture_number: u32,
    planes: [Vec<u16>; 3],
}

#[derive(Default)]
pub struct Decoder {
    seq: Option<Sequence>,
    pending: Option<Pending>,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// The current sequence header.
    pub fn sequence(&self) -> Option<&Sequence> {
        self.seq.as_ref()
    }

    /// Forget a first field waiting for its second (after a seek).
    pub fn reset(&mut self) {
        self.pending = None;
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
                0x10 => step = PARSE_INFO_BYTES,
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
                c if is_picture(c) || c & 0x08 == 0x08 => return Err(Error::Unsupported("Dirac core syntax pictures")),
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
        // Dirac core syntax
        let mut core = b"BBCD\x0C\0\0\0\0\0\0\0\0".to_vec();
        core.extend_from_slice(&[0; 16]);
        assert!(matches!(d.decode(&core), Err(Error::Unsupported(_))));
    }
}
