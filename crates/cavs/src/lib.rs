// SPDX-License-Identifier: GPL-3.0-or-later AND LGPL-2.1-or-later
//! Pure-Rust AVS1-P2 (GB/T 20090.2, "Chinese AVS", Jizhun profile) video decoder.
//!
//! A port of FFmpeg n7.1.1 `libavcodec/cavs.c`, `cavsdec.c`, `cavsdsp.c` and `cavs_parser.c`
//! (LGPL-2.1-or-later, Stefan Gehrer; owner-approved exception, `docs/licensing.md`). The decoder
//! is bit-exact with FFmpeg's `cavs` decoder, including its output order: with `low_delay` unset a
//! reference picture is output when the next reference picture is decoded, a B picture at once.
//!
//! Every input is hostile: no panics, no `unsafe`, capped picture size.

mod bits;
mod dec;
mod dsp;
#[cfg(any(test, feature = "synth"))]
#[doc(hidden)]
pub mod synth;
mod tables;

use dec::{Ctx, PIC_I_START, PIC_PB_START, Pic};

/// The sequence header start code.
const SEQ_START: u32 = 0xb0;
/// The Guangdian profile, which FFmpeg's demuxer probe accepts (its decoder does not).
const PROFILE_GUANGDIAN: u8 = 0x48;
/// The Jizhun profile.
const PROFILE_JIZHUN: u8 = 0x20;

/// Decoding errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("truncated AVS data")]
    Truncated,
    #[error("malformed AVS data: {0}")]
    Malformed(&'static str),
    #[error("unsupported AVS stream: {0}")]
    Unsupported(&'static str),
}

/// A decoded 8-bit 4:2:0 picture, cropped to the coded size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Y, Cb and Cr planes, tightly packed (chroma is `ceil(w/2) × ceil(h/2)`).
    pub planes: [Vec<u8>; 3],
    /// The tag passed with the packet that held the picture.
    pub tag: i64,
}

/// Stream parameters from the first sequence header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamInfo {
    pub profile: u32,
    pub level: u32,
    pub width: u32,
    pub height: u32,
    /// `aspect_ratio_information` (1 = square samples, 2 = 4:3, 3 = 16:9, 4 = 2.21:1).
    pub aspect_ratio: u32,
    /// Frame rate as a fraction.
    pub frame_rate: (u32, u32),
    /// Pictures are output in decoding order (no B pictures).
    pub low_delay: bool,
}

/// Iterates the start codes of `data` as FFmpeg's `avpriv_find_start_code` finds them: yields the
/// code (`0x000001xx`) and the offset just after it.
fn start_codes(data: &[u8]) -> impl Iterator<Item = (u32, usize)> + '_ {
    let mut state = u32::MAX;
    data.iter().enumerate().filter_map(move |(i, &b)| {
        state = (state << 8) | u32::from(b);
        ((state & 0xffff_ff00) == 0x100).then_some((state, i + 1))
    })
}

/// The coding type of a picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PictureType {
    I,
    P,
    B,
}

/// The type of the first picture in `packet` (`None` when it holds none or a reserved type).
pub fn picture_type(packet: &[u8]) -> Option<PictureType> {
    let (code, at) = start_codes(packet).find(|&(c, _)| matches!(c & 0xff, PIC_I_START | PIC_PB_START))?;
    if code & 0xff == PIC_I_START {
        return Some(PictureType::I);
    }
    // bbv_delay (16 bits), then picture_coding_type (2 bits)
    match packet.get(at + 2)? >> 6 {
        1 => Some(PictureType::P),
        2 => Some(PictureType::B),
        _ => None,
    }
}

/// Whether `code` (the byte after `00 00 01`) ends a packet before it ([`split_packets`]).
pub fn is_packet_start(code: u8) -> bool {
    matches!(u32::from(code), PIC_I_START | PIC_PB_START | SEQ_START)
}

/// Whether `code` starts a picture.
pub fn is_picture_start(code: u8) -> bool {
    matches!(u32::from(code), PIC_I_START | PIC_PB_START)
}

/// FFmpeg's raw `cavsvideo` probe: true when `data` looks like an AVS1 video elementary stream.
pub fn probe(data: &[u8]) -> bool {
    let (mut pic, mut seq, mut slice_pos) = (0u32, 0u32, 0u32);
    for (code, at) in start_codes(data) {
        let c = code & 0xff;
        if c < SEQ_START {
            // slices have to be consecutive
            if c < slice_pos {
                return false;
            }
            slice_pos = c;
        } else {
            slice_pos = 0;
        }
        if c == SEQ_START {
            seq += 1;
            let profile = data.get(at).copied().unwrap_or(0);
            if profile != PROFILE_JIZHUN && profile != PROFILE_GUANGDIAN {
                return false;
            }
        } else if c == PIC_I_START || c == PIC_PB_START {
            pic += 1;
        } else if c == 0xb4 || c > 0xb7 {
            return false;
        }
    }
    seq > 0 && u64::from(seq) * 9 <= u64::from(pic) * 10
}

/// Reads the first sequence header of a stream.
pub fn stream_info(data: &[u8]) -> Result<StreamInfo, Error> {
    let (_, at) = start_codes(data).find(|&(c, _)| c & 0xff == SEQ_START).ok_or(Error::Malformed("no sequence header"))?;
    let s = dec::parse_seq(data.get(at..).unwrap_or_default())?;
    let frame_rate = tables::FRAME_RATES.get(s.frame_rate_code as usize).copied().unwrap_or((25, 1));
    Ok(StreamInfo { profile: s.profile, level: s.level, width: s.width, height: s.height, aspect_ratio: s.aspect_ratio, frame_rate, low_delay: s.low_delay })
}

/// Splits an elementary stream into packets as FFmpeg's `cavs` parser does: a packet runs from a
/// picture start code to the next picture or sequence header start code (user data, extension and
/// sequence end codes stay in the packet before them). Leading data before the first picture
/// belongs to the first packet.
pub fn split_packets(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut pic_found = false;
    for (code, at) in start_codes(data) {
        let c = code & 0xff;
        if !pic_found {
            if c == PIC_I_START || c == PIC_PB_START {
                pic_found = true;
            }
        } else if c == PIC_I_START || c == PIC_PB_START || c == SEQ_START {
            let end = at.saturating_sub(4);
            if let Some(p) = data.get(start..end) {
                out.push(p);
            }
            start = end;
            pic_found = c == PIC_I_START || c == PIC_PB_START;
        }
    }
    if let Some(p) = data.get(start..)
        && !p.is_empty()
    {
        out.push(p);
    }
    out
}

/// An AVS1 decoder fed packets (as [`split_packets`] makes them).
#[derive(Default)]
pub struct Decoder {
    ctx: Ctx,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// The current sequence parameters (after the first sequence header).
    pub fn info(&self) -> Option<StreamInfo> {
        let s = self.ctx.seq.as_ref()?;
        let frame_rate = tables::FRAME_RATES.get(s.frame_rate_code as usize).copied().unwrap_or((25, 1));
        Some(StreamInfo {
            profile: s.profile,
            level: s.level,
            width: s.width,
            height: s.height,
            aspect_ratio: s.aspect_ratio,
            frame_rate,
            low_delay: s.low_delay,
        })
    }

    /// Decodes one packet with tag 0.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Option<Frame>, Error> {
        self.decode_tagged(packet, 0)
    }

    /// Decodes one packet (`cavs_decode_frame`). Returns the picture FFmpeg outputs for it, if
    /// any; an output reference picture carries the tag of the packet it was decoded from.
    pub fn decode_tagged(&mut self, packet: &[u8], tag: i64) -> Result<Option<Frame>, Error> {
        if packet.is_empty() {
            return Ok(self.flush());
        }
        let mut out: Option<Frame> = None;
        let mut frame_start = 0u32;
        for (code, at) in start_codes(packet) {
            if at >= packet.len() {
                break;
            }
            let rest = packet.get(at..).unwrap_or_default();
            let c = code & 0xff;
            match c {
                SEQ_START => {
                    // FFmpeg ignores a sequence header it cannot use
                    if let Ok(s) = dec::parse_seq(rest) {
                        let _ = self.ctx.sequence(s);
                    }
                }
                PIC_I_START | PIC_PB_START => {
                    if c == PIC_I_START && !self.ctx.got_keyframe {
                        self.ctx.dpb = [None, None];
                        self.ctx.got_keyframe = true;
                    }
                    if frame_start > 1 {
                        return Err(Error::Malformed("more than two pictures in a packet"));
                    }
                    frame_start += 1;
                    out = None;
                    if !self.ctx.got_keyframe {
                        continue;
                    }
                    let Some(seq) = self.ctx.seq.clone() else { continue };
                    match self.ctx.decode_pic(rest, c, tag) {
                        Ok(Some(b)) => out = Some(self.frame(&b)),
                        Ok(None) => {
                            let idx = usize::from(!seq.low_delay);
                            out = self.ctx.dpb.get(idx).and_then(Option::as_ref).map(|p| self.frame(p));
                        }
                        Err(_) => {}
                    }
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// Drains the decoder at the end of the stream: outputs the newest reference picture that was
    /// not output yet (only when `low_delay` is unset).
    pub fn flush(&mut self) -> Option<Frame> {
        let low_delay = self.ctx.seq.as_ref().is_some_and(|s| s.low_delay);
        if low_delay {
            return None;
        }
        let pic = self.ctx.dpb.get_mut(0).and_then(Option::take)?;
        Some(self.frame(&pic))
    }

    /// Forgets the reference pictures before a seek (`cavs_flush`): decoding resumes at the next
    /// I picture.
    pub fn reset(&mut self) {
        self.ctx.got_keyframe = false;
    }

    fn frame(&self, pic: &Pic) -> Frame {
        let Some(s) = self.ctx.seq.as_ref() else {
            return Frame { width: 0, height: 0, planes: [Vec::new(), Vec::new(), Vec::new()], tag: pic.tag };
        };
        let (w, h) = (s.width as usize, s.height as usize);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let crop = |src: &[u8], stride: usize, w: usize, h: usize| -> Vec<u8> {
            let mut v = Vec::with_capacity(w * h);
            for row in src.chunks(stride.max(1)).take(h) {
                v.extend_from_slice(row.get(..w).unwrap_or(row));
            }
            v
        };
        let p = &pic.planes;
        Frame {
            width: s.width,
            height: s.height,
            planes: [crop(&p.y, s.mb_w * 16, w, h), crop(&p.u, s.mb_w * 8, cw, ch), crop(&p.v, s.mb_w * 8, cw, ch)],
            tag: pic.tag,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_inputs_do_not_panic() {
        let mut d = Decoder::new();
        for data in [&[][..], &[0, 0, 1], &[0, 0, 1, 0xb3], &[0, 0, 1, 0xb0, 0x20], &[0, 0, 1, 0xb6, 0xff, 0xff, 0xff]] {
            let _ = d.decode(data);
            let _ = split_packets(data);
            let _ = probe(data);
            let _ = stream_info(data);
        }
    }

    #[test]
    fn picture_types() {
        assert_eq!(picture_type(&[0, 0, 1, 0xb0, 0x20, 0, 0, 1, 0xb3, 0]), Some(PictureType::I));
        assert_eq!(picture_type(&[0, 0, 1, 0xb6, 0xff, 0xff, 0x40]), Some(PictureType::P));
        assert_eq!(picture_type(&[0, 0, 1, 0xb6, 0xff, 0xff, 0x80]), Some(PictureType::B));
        assert_eq!(picture_type(&[0, 0, 1, 0xb6, 0xff, 0xff, 0xc0]), None);
        assert_eq!(picture_type(&[0, 0, 1, 0xb6, 0xff]), None);
        assert_eq!(picture_type(&[0, 0, 1, 0xb0]), None);
    }

    #[test]
    fn splits_before_the_next_picture() {
        // seq, I, slice 0, PB, user data, PB, sequence end
        let data = [0, 0, 1, 0xb0, 0x20, 0, 0, 1, 0xb3, 1, 2, 0, 0, 1, 0x00, 3, 0, 0, 1, 0xb6, 4, 0, 0, 1, 0xb2, 5, 0, 0, 1, 0xb6, 6, 0, 0, 1, 0xb1];
        let p = split_packets(&data);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0], &data[..16]);
        assert_eq!(p[1], &data[16..26]);
        assert_eq!(p[2], &data[26..]);
    }
}
