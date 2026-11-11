//! S3TC (DXT1 / BC1, DXT5 / BC3) and RGTC1 (BC4) block decoding, from the OpenGL
//! `EXT_texture_compression_s3tc` and `EXT_texture_compression_rgtc` specifications.
//!
//! The specifications give the interpolation as exact fractions; the integer rounding used here
//! (a fixed-point 5:6:5 expansion; the colour and alpha interpolations truncate) was chosen to match the output of ffmpeg, the test oracle.

/// Expand a 5:6:5 colour to 8 bits per channel. The fixed-point factors (about 255 / 31 and
/// 255 / 63) reproduce the oracle's output for every one of the 32 and 64 input values.
fn rgb565(c: u16) -> [u8; 3] {
    let five = |v: u16| ((u32::from(v) * 263 + 16) >> 5) as u8;
    let six = |v: u16| ((u32::from(v) * 259 + 32) >> 6) as u8;
    [five((c >> 11) & 0x1F), six((c >> 5) & 0x3F), five(c & 0x1F)]
}

/// The four palette colours (RGBA) of a BC1 colour block. With `opaque_only` (DXT5 colour
/// blocks) the block is always decoded as if `color0 > color1`.
fn palette(block: &[u8; 8], opaque_only: bool) -> [[u8; 4]; 4] {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let (a, b) = (rgb565(c0), rgb565(c1));
    let mix = |wa: u32, wb: u32, div: u32| -> [u8; 4] {
        let ch = |i: usize| ((u32::from(a[i]) * wa + u32::from(b[i]) * wb) / div) as u8;
        [ch(0), ch(1), ch(2), 255]
    };
    let p0 = [a[0], a[1], a[2], 255];
    let p1 = [b[0], b[1], b[2], 255];
    if c0 > c1 || opaque_only { [p0, p1, mix(2, 1, 3), mix(1, 2, 3)] } else { [p0, p1, mix(1, 1, 2), [0, 0, 0, 0]] }
}

/// Decode a BC1 colour block into 16 RGBA texels (row-major).
pub fn bc1(block: &[u8; 8], opaque_only: bool) -> [[u8; 4]; 16] {
    let pal = palette(block, opaque_only);
    let bits = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    let mut out = [[0u8; 4]; 16];
    for (i, px) in out.iter_mut().enumerate() {
        *px = pal[((bits >> (2 * i)) & 3) as usize];
    }
    out
}

/// Decode a DXT5 alpha / RGTC1 (unsigned) block into 16 values (row-major).
pub fn bc4(block: &[u8; 8]) -> [u8; 16] {
    let (a0, a1) = (u32::from(block[0]), u32::from(block[1]));
    let mut pal = [0u8; 8];
    pal[0] = block[0];
    pal[1] = block[1];
    if a0 > a1 {
        for k in 1..7u32 {
            pal[(k + 1) as usize] = (((7 - k) * a0 + k * a1) / 7) as u8;
        }
    } else {
        for k in 1..5u32 {
            pal[(k + 1) as usize] = (((5 - k) * a0 + k * a1) / 5) as u8;
        }
        pal[6] = 0;
        pal[7] = 255;
    }
    let bits = block[2..8].iter().rev().fold(0u64, |acc, &b| (acc << 8) | u64::from(b));
    let mut out = [0u8; 16];
    for (i, v) in out.iter_mut().enumerate() {
        *v = pal[((bits >> (3 * i)) & 7) as usize];
    }
    out
}

/// Decode a DXT5 / BC3 block (alpha block, then colour block) into 16 RGBA texels.
pub fn bc3(block: &[u8; 16]) -> [[u8; 4]; 16] {
    let mut a = [0u8; 8];
    a.copy_from_slice(&block[..8]);
    let mut c = [0u8; 8];
    c.copy_from_slice(&block[8..]);
    let alpha = bc4(&a);
    let mut out = bc1(&c, true);
    for (px, a) in out.iter_mut().zip(alpha) {
        px[3] = a;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bc1_modes() {
        // color0 = white > color1 = black: thirds
        let b = [0xFF, 0xFF, 0, 0, 0b1110_0100, 0, 0, 0];
        let t = bc1(&b, false);
        assert_eq!(t[0], [255, 255, 255, 255]);
        assert_eq!(t[1], [0, 0, 0, 255]);
        assert_eq!(t[2], [170, 170, 170, 255]);
        assert_eq!(t[3], [85, 85, 85, 255]);
        assert_eq!(rgb565(0x18E7), [25, 28, 58]);
        // every 5- and 6-bit value spans 0..=255
        assert_eq!(rgb565(0xFFFF), [255, 255, 255]);
        assert_eq!(rgb565((20 << 11) | (53 << 5) | 29), [164, 214, 238]);
        // color0 <= color1: half and transparent black
        let b = [0, 0, 0xFF, 0xFF, 0b1110_0100, 0, 0, 0];
        let t = bc1(&b, false);
        assert_eq!(t[2], [127, 127, 127, 255]);
        assert_eq!(t[3], [0, 0, 0, 0]);
        assert_eq!(bc1(&b, true)[3], [170, 170, 170, 255]);
    }

    #[test]
    fn bc4_modes() {
        // a0 > a1: eight values; codes 0..7 in the first 8 texels
        let codes: u64 = (0..8u64).fold(0, |acc, k| acc | (k << (3 * k)));
        let mut b = [255u8, 0, 0, 0, 0, 0, 0, 0];
        b[2..8].copy_from_slice(&codes.to_le_bytes()[..6]);
        let v = bc4(&b);
        assert_eq!(&v[..8], &[255, 0, 218, 182, 145, 109, 72, 36]);
        // a0 <= a1: six values plus 0 and 255
        b[0] = 0;
        b[1] = 255;
        let v = bc4(&b);
        assert_eq!(&v[..8], &[0, 255, 51, 102, 153, 204, 0, 255]);
    }
}
