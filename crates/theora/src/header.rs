//! The identification and setup headers (Theora specification §6; FFmpeg n7.1.1 `vp3.c`
//! `theora_decode_header` and `theora_decode_tables`, LGPL-2.1-or-later).

use crate::bits::Bits;
use crate::vlc::Vlc;
use crate::{Chroma, Error, Result};

/// The largest frame accepted, in pixels per side and in total.
const MAX_SIDE: u32 = 16384;
const MAX_AREA: u64 = 1 << 26;

/// The identification header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    /// Bitstream version, `major << 16 | minor << 8 | revision` (3.2.0 and later).
    pub version: u32,
    /// The coded frame size (a multiple of 16).
    pub width: u32,
    pub height: u32,
    /// The visible picture: size and offset of its lower left corner in the frame.
    pub pic_width: u32,
    pub pic_height: u32,
    pub pic_x: u32,
    pub pic_y: u32,
    /// Frame rate as numerator and denominator (0 when not given).
    pub fps: (u32, u32),
    /// Pixel aspect ratio (0 when not given).
    pub aspect: (u32, u32),
    /// 0 unspecified, 1 Rec. 470M, 2 Rec. 470BG.
    pub colorspace: u8,
    pub chroma: Chroma,
}

/// The 80 Huffman tables, the quantizer parameters and the loop filter limits.
pub(crate) struct Setup {
    pub(crate) filter_limit: [u8; 64],
    pub(crate) ac_scale: [u16; 64],
    pub(crate) dc_scale: [u16; 64],
    pub(crate) base_matrix: Vec<[u8; 64]>,
    pub(crate) qr_count: [[usize; 3]; 2],
    pub(crate) qr_size: [[[u8; 64]; 3]; 2],
    pub(crate) qr_base: [[[u16; 64]; 3]; 2],
    pub(crate) huffman: Vec<Vlc>,
}

fn ilog(v: u32) -> u32 {
    32 - v.leading_zeros()
}

/// Checks the `0x80 "theora"` style packet start and returns the reader after it.
fn start<'a>(packet: &'a [u8], kind: u8) -> Result<Bits<'a>> {
    if packet.len() < 7 || packet.first() != Some(&kind) || packet.get(1..7) != Some(&b"theora"[..]) {
        return Err(Error::Invalid("not a Theora header packet"));
    }
    Ok(Bits::new(packet.get(7..).unwrap_or(&[])))
}

pub(crate) fn parse_info(packet: &[u8]) -> Result<Info> {
    let mut b = start(packet, 0x80)?;
    if b.left() < 35 * 8 {
        return Err(Error::Invalid("Theora identification header too short"));
    }
    let version = b.get(24);
    if version < 0x03_02_00 {
        return Err(Error::Unsupported("Theora streams before 3.2.0"));
    }
    let width = b.get(16) << 4;
    let height = b.get(16) << 4;
    let pic_width = b.get(24);
    let pic_height = b.get(24);
    let pic_x = b.get(8);
    let pic_y = b.get(8);
    let fps = (b.get(32), b.get(32));
    let aspect = (b.get(24), b.get(24));
    let colorspace = b.get(8) as u8;
    b.skip(24 + 6 + 5); // nominal bitrate, quality hint, keyframe granule shift
    let chroma = match b.get(2) {
        0 => Chroma::C420,
        2 => Chroma::C422,
        3 => Chroma::C444,
        _ => return Err(Error::Invalid("Theora pixel format")),
    };
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE || u64::from(width) * u64::from(height) > MAX_AREA {
        return Err(Error::Invalid("Theora frame size"));
    }
    if pic_width == 0 || pic_height == 0 || pic_width + pic_x > width || pic_height + pic_y > height {
        return Err(Error::Invalid("Theora picture region"));
    }
    Ok(Info { version, width, height, pic_width, pic_height, pic_x, pic_y, fps, aspect, colorspace, chroma })
}

/// Reads one Huffman tree (leaves as `(length, token)` in left-to-right order).
fn read_tree(b: &mut Bits<'_>, depth: u32, out: &mut Vec<(u8, i16)>) -> Result<()> {
    if b.left() == 0 {
        return Err(Error::Invalid("Huffman tree runs past the header"));
    }
    if b.get1() {
        if out.len() >= 32 {
            return Err(Error::Invalid("Huffman tree has more than 32 leaves"));
        }
        out.push((depth as u8, b.get(5) as i16));
    } else {
        if depth >= 31 {
            return Err(Error::Invalid("Huffman tree too deep"));
        }
        read_tree(b, depth + 1, out)?;
        read_tree(b, depth + 1, out)?;
    }
    Ok(())
}

pub(crate) fn parse_setup(packet: &[u8]) -> Result<Setup> {
    let mut b = start(packet, 0x82)?;
    let mut s = Setup {
        filter_limit: [0; 64],
        ac_scale: [0; 64],
        dc_scale: [0; 64],
        base_matrix: Vec::new(),
        qr_count: [[0; 3]; 2],
        qr_size: [[[0; 64]; 3]; 2],
        qr_base: [[[0; 64]; 3]; 2],
        huffman: Vec::new(),
    };
    let n = b.get(3);
    if n != 0 {
        for v in s.filter_limit.iter_mut() {
            *v = b.get(n) as u8;
        }
    }
    let n = b.get(4) + 1;
    for v in s.ac_scale.iter_mut() {
        *v = b.get(n) as u16;
    }
    let n = b.get(4) + 1;
    for v in s.dc_scale.iter_mut() {
        *v = b.get(n) as u16;
    }
    let matrices = b.get(9) as usize + 1;
    if matrices > 384 {
        return Err(Error::Invalid("Theora base matrix count"));
    }
    for _ in 0..matrices {
        let mut m = [0u8; 64];
        for v in m.iter_mut() {
            *v = b.get(8) as u8;
        }
        s.base_matrix.push(m);
    }
    for inter in 0..2usize {
        for plane in 0..3usize {
            let newqr = if inter == 1 || plane > 0 { b.get1() } else { true };
            if !newqr {
                let (qtj, plj) = if inter == 1 && b.get1() { (0, plane) } else { ((3 * inter + plane - 1) / 3, (plane + 2) % 3) };
                s.qr_count[inter][plane] = s.qr_count[qtj][plj];
                s.qr_size[inter][plane] = s.qr_size[qtj][plj];
                s.qr_base[inter][plane] = s.qr_base[qtj][plj];
            } else {
                let mut qri = 0usize;
                let mut qi = 0u32;
                loop {
                    let i = b.get(ilog(matrices as u32 - 1)) as usize;
                    if i >= matrices {
                        return Err(Error::Invalid("Theora base matrix index"));
                    }
                    if let Some(v) = s.qr_base[inter][plane].get_mut(qri) {
                        *v = i as u16;
                    }
                    if qi >= 63 {
                        break;
                    }
                    let size = b.get(ilog(63 - qi)) + 1;
                    if qri >= 63 {
                        return Err(Error::Invalid("Theora quant range count"));
                    }
                    s.qr_size[inter][plane][qri] = size as u8;
                    qri += 1;
                    qi += size;
                }
                if qi > 63 {
                    return Err(Error::Invalid("Theora quant ranges"));
                }
                s.qr_count[inter][plane] = qri;
            }
        }
    }
    for _ in 0..80 {
        let mut leaves = Vec::new();
        read_tree(&mut b, 0, &mut leaves)?;
        s.huffman.push(Vlc::new(&leaves)?);
    }
    Ok(s)
}
