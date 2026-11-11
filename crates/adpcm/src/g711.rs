//! ITU-T G.711 logarithmic PCM: μ-law and A-law expansion to 16-bit linear PCM.
//!
//! The 8-bit code is a sign, a 3-bit segment and a 4-bit step inside the segment (G.711 Tables 1a /
//! 2a). μ-law codes are transmitted inverted, A-law codes with the even bits inverted (XOR 0x55).
//! The decoded value is the middle of the step's quantization interval, in 16-bit units (14-bit
//! μ-law and 13-bit A-law magnitudes shifted to the top of a 16-bit word).

/// μ-law: bias 33 (in 14-bit units) folded into the segment base, removed after the shift.
pub fn ulaw_to_i16(code: u8) -> i16 {
    let u = !code;
    let segment = u32::from((u >> 4) & 7);
    let step = u32::from(u & 0x0F);
    // ((2·step + 33) << segment) − 33, in 14-bit units, times 4 for 16 bits
    let magnitude = (((step << 3) + 0x84) << segment) - 0x84;
    let m = magnitude as i32;
    (if u & 0x80 != 0 { -m } else { m }) as i16
}

/// A-law: segment 0 is linear; segment `s` ≥ 1 doubles the step `s − 1` times.
pub fn alaw_to_i16(code: u8) -> i16 {
    let a = code ^ 0x55;
    let segment = u32::from((a >> 4) & 7);
    let step = u32::from(a & 0x0F);
    let mut magnitude = (step << 4) + 8;
    if segment > 0 {
        magnitude = (magnitude + 0x100) << (segment - 1);
    }
    let m = magnitude as i32;
    (if a & 0x80 != 0 { m } else { -m }) as i16
}

/// μ-law or A-law expansion table, built once per decoder.
pub(crate) fn table(alaw: bool) -> [i16; 256] {
    let mut t = [0i16; 256];
    for (i, v) in t.iter_mut().enumerate() {
        let c = i as u8;
        *v = if alaw { alaw_to_i16(c) } else { ulaw_to_i16(c) };
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values() {
        // G.711 Table 2a: code 0xFF / 0x7F are the smallest magnitudes, 0x80 / 0x00 the largest
        assert_eq!(ulaw_to_i16(0xFF), 0);
        assert_eq!(ulaw_to_i16(0x7F), 0);
        assert_eq!(ulaw_to_i16(0x80), 32124);
        assert_eq!(ulaw_to_i16(0x00), -32124);
        // A-law: 0xD5 / 0x55 are +8 / −8, 0xAA / 0x2A the extremes
        assert_eq!(alaw_to_i16(0xD5), 8);
        assert_eq!(alaw_to_i16(0x55), -8);
        assert_eq!(alaw_to_i16(0xAA), 32256);
        assert_eq!(alaw_to_i16(0x2A), -32256);
    }

    #[test]
    fn monotonic_in_each_half() {
        // positive μ-law codes 0xFF..=0x80 grow, A-law codes are monotonic after the XOR
        let u: Vec<i16> = (0x80..=0xFFu8).rev().map(ulaw_to_i16).collect();
        assert!(u.windows(2).all(|w| w[0] < w[1]));
        let a: Vec<i16> = (0..=0x7Fu8).map(|v| alaw_to_i16((v | 0x80) ^ 0x55)).collect();
        assert!(a.windows(2).all(|w| w[0] < w[1]));
    }
}
