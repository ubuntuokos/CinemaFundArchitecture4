//! DV subcode: the time code of a frame.
//!
//! The two subcode DIF blocks of every DIF sequence hold six sync blocks (SSYB) each, after the
//! 3-byte block ID: 2 ID bytes, a reserved byte, then a 5-byte pack. The time code pack (header
//! `0x13`) carries the SMPTE 12M fields in BCD: frames (with the drop-frame flag in bit 6),
//! seconds, minutes and hours; the top bits are flags (colour frame, binary groups) and are
//! masked off. Camcorders repeat the pack in many sync blocks and some of them hold `0xFF`
//! (no data), so the first pack with valid digits wins.

use crate::dif::{BLOCK_SIZE, block_sct, sct};

/// Pack header of the time code pack.
pub const TIMECODE_PACK: u8 = 0x13;

/// SMPTE time code of a frame, as the subcode carries it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timecode {
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
    pub frames: u8,
    /// The drop-frame flag (meaningful at 525/60 only).
    pub drop_frame: bool,
}

/// Number of SSYBs in a subcode block, and their size.
const SSYBS: usize = 6;
const SSYB_SIZE: usize = 8;

/// The time code of a DIF frame (or the head of one): the first valid time code pack of its
/// subcode blocks. `None` if no subcode block holds one.
pub fn timecode(frame: &[u8]) -> Option<Timecode> {
    frame
        .chunks_exact(BLOCK_SIZE)
        .filter(|b| block_sct(b) == Some(sct::SUBCODE))
        .flat_map(|b| (0..SSYBS).filter_map(move |k| b.get(3 + k * SSYB_SIZE + 3..3 + (k + 1) * SSYB_SIZE)))
        .find_map(parse_pack)
}

/// The fields of a 5-byte time code pack, if its header and digits are valid.
pub fn parse_pack(pack: &[u8]) -> Option<Timecode> {
    let &[TIMECODE_PACK, f, s, m, h] = pack else { return None };
    let bcd = |v: u8, tens_mask: u8, max: u8| {
        let (tens, units) = ((v >> 4) & tens_mask, v & 0x0F);
        (units <= 9).then_some(tens * 10 + units).filter(|&n| n <= max)
    };
    Some(Timecode { frames: bcd(f, 0x3, 39)?, seconds: bcd(s, 0x7, 59)?, minutes: bcd(m, 0x7, 59)?, hours: bcd(h, 0x3, 23)?, drop_frame: f & 0x40 != 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subcode_block(packs: &[[u8; 5]]) -> Vec<u8> {
        let mut b = vec![0xFFu8; BLOCK_SIZE];
        b[0] = sct::SUBCODE << 5 | 0x1F;
        b[1] = 0x07;
        b[2] = 0;
        for (k, p) in packs.iter().enumerate() {
            b[3 + k * SSYB_SIZE + 3..3 + (k + 1) * SSYB_SIZE].copy_from_slice(p);
        }
        b
    }

    #[test]
    fn reads_the_first_valid_pack() {
        let mut frame = vec![0u8; BLOCK_SIZE]; // header block (sct 0)
        frame.extend(subcode_block(&[[0xFF; 5], [0x13, 0xFF, 0xFF, 0xFF, 0xFF], [0x13, 0xC4, 0x83, 0x82, 0xC1]]));
        let tc = timecode(&frame);
        assert_eq!(tc, Some(Timecode { hours: 1, minutes: 2, seconds: 3, frames: 4, drop_frame: true }));
    }

    #[test]
    fn rejects_missing_or_bad_packs() {
        assert_eq!(timecode(&[]), None);
        assert_eq!(timecode(&subcode_block(&[[0x13, 0x0A, 0, 0, 0]])), None); // units 10
        assert_eq!(timecode(&subcode_block(&[[0x13, 0, 0, 0, 0x24]])), None); // 24 h
        assert_eq!(timecode(&subcode_block(&[[0x14, 0, 0, 0, 0]])), None); // other pack
        assert_eq!(parse_pack(&[0x13, 0x29, 0x59, 0x59, 0x23]), Some(Timecode { hours: 23, minutes: 59, seconds: 59, frames: 29, drop_frame: false }));
    }
}
