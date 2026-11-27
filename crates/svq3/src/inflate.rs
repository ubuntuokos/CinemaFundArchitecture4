//! A small zlib (RFC 1950) / DEFLATE (RFC 1951) decompressor, after `puff.c` by Mark Adler. The
//! SVQ3 sequence header can carry a zlib-compressed watermark logo whose CRC is the key that
//! scrambles every slice; nothing else in the codec needs it.

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    bit_buf: u32,
    bit_cnt: u32,
}

impl Reader<'_> {
    fn bits(&mut self, need: u32) -> Option<u32> {
        let mut val = self.bit_buf;
        while self.bit_cnt < need {
            let b = *self.data.get(self.pos)?;
            self.pos += 1;
            val |= u32::from(b) << self.bit_cnt;
            self.bit_cnt += 8;
        }
        self.bit_buf = if need >= 32 { 0 } else { val >> need };
        self.bit_cnt -= need;
        Some(if need >= 32 { val } else { val & ((1u32 << need) - 1) })
    }
}

struct Huffman {
    count: [u16; 16],
    symbol: Vec<u16>,
}

impl Huffman {
    /// Builds the canonical code of `lengths`; `None` for an over-subscribed set.
    fn new(lengths: &[u8]) -> Option<Huffman> {
        let mut count = [0u16; 16];
        for &l in lengths {
            count[usize::from(l)] += 1;
        }
        let mut left: i32 = 1;
        for &c in &count[1..] {
            left <<= 1;
            left -= i32::from(c);
            if left < 0 {
                return None;
            }
        }
        let mut offs = [0u16; 16];
        for len in 1..15 {
            offs[len + 1] = offs[len] + count[len];
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbol[usize::from(offs[usize::from(l)])] = sym as u16;
                offs[usize::from(l)] += 1;
            }
        }
        Some(Huffman { count, symbol })
    }

    fn decode(&self, r: &mut Reader<'_>) -> Option<u16> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= r.bits(1)? as i32;
            let count = i32::from(self.count[len]);
            if code - count < first {
                return self.symbol.get((index + (code - first)) as usize).copied();
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        None
    }
}

const LBASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEXT: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DBASE: [u16; 30] =
    [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DEXT: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

fn codes(r: &mut Reader<'_>, out: &mut Vec<u8>, limit: usize, lit: &Huffman, dist: &Huffman) -> Option<()> {
    loop {
        let sym = usize::from(lit.decode(r)?);
        match sym {
            0..=255 => {
                if out.len() >= limit {
                    return None;
                }
                out.push(sym as u8);
            }
            256 => return Some(()),
            _ => {
                let s = sym - 257;
                if s >= 29 {
                    return None;
                }
                let len = usize::from(LBASE[s]) + r.bits(u32::from(LEXT[s]))? as usize;
                let ds = usize::from(dist.decode(r)?);
                if ds >= 30 {
                    return None;
                }
                let d = usize::from(DBASE[ds]) + r.bits(u32::from(DEXT[ds]))? as usize;
                if d > out.len() || out.len() + len > limit {
                    return None;
                }
                for _ in 0..len {
                    out.push(out[out.len() - d]);
                }
            }
        }
    }
}

fn fixed_tables() -> Option<(Huffman, Huffman)> {
    let mut l = [0u8; 288];
    for (i, v) in l.iter_mut().enumerate() {
        *v = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    Some((Huffman::new(&l)?, Huffman::new(&[5u8; 30])?))
}

fn dynamic_tables(r: &mut Reader<'_>) -> Option<(Huffman, Huffman)> {
    const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
    let nlen = r.bits(5)? as usize + 257;
    let ndist = r.bits(5)? as usize + 1;
    let ncode = r.bits(4)? as usize + 4;
    if nlen > 286 || ndist > 30 {
        return None;
    }
    let mut lengths = [0u8; 320];
    for &o in ORDER.iter().take(ncode) {
        lengths[o] = r.bits(3)? as u8;
    }
    let lencode = Huffman::new(&lengths[..19])?;
    let mut index = 0;
    let mut lengths = [0u8; 320];
    while index < nlen + ndist {
        let sym = lencode.decode(r)?;
        if sym < 16 {
            lengths[index] = sym as u8;
            index += 1;
        } else {
            let (prev, rep) = match sym {
                16 => {
                    if index == 0 {
                        return None;
                    }
                    (lengths[index - 1], 3 + r.bits(2)? as usize)
                }
                17 => (0, 3 + r.bits(3)? as usize),
                _ => (0, 11 + r.bits(7)? as usize),
            };
            if index + rep > nlen + ndist {
                return None;
            }
            for _ in 0..rep {
                lengths[index] = prev;
                index += 1;
            }
        }
    }
    if lengths[256] == 0 {
        return None;
    }
    Some((Huffman::new(&lengths[..nlen])?, Huffman::new(&lengths[nlen..nlen + ndist])?))
}

fn adler32(d: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in d {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// Decompresses a zlib stream into at most `limit` bytes; `None` if it is damaged, truncated,
/// longer than `limit` or fails its Adler-32 check (what zlib's `uncompress` calls an error).
pub fn zlib_decompress(input: &[u8], limit: usize) -> Option<Vec<u8>> {
    if input.len() < 6 {
        return None;
    }
    let (cmf, flg) = (input[0], input[1]);
    if cmf & 0x0F != 8 || cmf >> 4 > 7 || ((u32::from(cmf) << 8) | u32::from(flg)) % 31 != 0 || flg & 0x20 != 0 {
        return None;
    }
    let mut r = Reader { data: &input[2..], pos: 0, bit_buf: 0, bit_cnt: 0 };
    let mut out = Vec::new();
    loop {
        let last = r.bits(1)?;
        match r.bits(2)? {
            0 => {
                r.bit_buf = 0;
                r.bit_cnt = 0;
                let d = r.data.get(r.pos..r.pos + 4)?;
                let len = usize::from(u16::from_le_bytes([d[0], d[1]]));
                if len != usize::from(!u16::from_le_bytes([d[2], d[3]])) {
                    return None;
                }
                r.pos += 4;
                if out.len() + len > limit {
                    return None;
                }
                out.extend_from_slice(r.data.get(r.pos..r.pos + len)?);
                r.pos += len;
            }
            1 => {
                let (l, d) = fixed_tables()?;
                codes(&mut r, &mut out, limit, &l, &d)?;
            }
            2 => {
                let (l, d) = dynamic_tables(&mut r)?;
                codes(&mut r, &mut out, limit, &l, &d)?;
            }
            _ => return None,
        }
        if last == 1 {
            break;
        }
    }
    // the trailer is byte aligned
    let tail = r.data.get(r.pos..r.pos + 4)?;
    (u32::from_be_bytes([tail[0], tail[1], tail[2], tail[3]]) == adler32(&out)).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_fixed_and_dynamic_blocks() {
        // zlib.compress(b"hello hello hello hello", 9): a fixed-code block
        let fixed = [0x78, 0xDA, 0xCB, 0x48, 0xCD, 0xC9, 0xC9, 0x57, 0xC8, 0x40, 0x27, 0x01, 0x68, 0x03, 0x08, 0xB1];
        assert_eq!(zlib_decompress(&fixed, 100).as_deref(), Some(&b"hello hello hello hello"[..]));
        assert!(zlib_decompress(&fixed, 5).is_none());
        // a stored block
        let stored = [0x78, 0x01, 0x01, 0x03, 0x00, 0xFC, 0xFF, b'a', b'b', b'c', 0x02, 0x4D, 0x01, 0x27];
        assert_eq!(zlib_decompress(&stored, 10).as_deref(), Some(&b"abc"[..]));
        let mut bad = stored;
        bad[13] ^= 1;
        assert!(zlib_decompress(&bad, 10).is_none());
        assert!(zlib_decompress(&stored[..9], 10).is_none());
        // a dynamic-code block
        const TEXT: &str = "ebrsqccr  ubmh ndrasgratvysahahj ocnkp yzawpocavxavama fpyxmafreaanamafvpajnt  tx aarmpbgdbsdacae ldaapekawa e quai yrahzaaaaaqxaag zaaahpg auuvmaafuzaaxaaesaa plraaaxa  iqsrexo a aeaaayapdkqzaagyoyakqgoz qaaraodkhgzywl o  vananklrwkjc aai aeaa eqwiw  ca iaka ewdafjqaqiaaaagha v aaga aalaauafvaajwoi";
        const HEX: &str = "78da1d8f591685200c43b792ad4540269549405cfdabaf5f4d9b9e9b9aadb6a25405fa763a5cbab2d9ca7bac46471790d41533d6cb9993e2783878127b5ecfc9bd1af212bd8fcc70ddc0fd80ac67deacde9aa6a2c1a1c96c222761503a3d56a57bf95579488baf77d982bd8f93dcbb0c64611a897c547e0af0a555f32410142a17b38e459c76a5c5586c7a5184cda4a3b3ef9a07123024de158f3a635092ccff6f61caf413508467143935f75058fc97c93a6288d70a8807d9e53b32cce47fd67e723c";
        let z: Vec<u8> = (0..HEX.len() / 2).map(|i| u8::from_str_radix(&HEX[2 * i..2 * i + 2], 16).unwrap()).collect();
        assert_eq!((z[2] >> 1) & 3, 2);
        assert_eq!(zlib_decompress(&z, 1000).as_deref(), Some(TEXT.as_bytes()));
        for cut in 0..z.len() {
            assert!(zlib_decompress(&z[..cut], 1000).is_none() || cut == z.len());
        }
    }
}
