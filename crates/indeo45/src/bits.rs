//! Little-endian bit reader (FFmpeg's `BITSTREAM_READER_LE` `GetBitContext`): bits are taken from
//! the least significant end of each byte. Reading past the end yields zero bits.

pub struct Bits<'a> {
    data: &'a [u8],
    /// position in bits
    pos: usize,
}

impl<'a> Bits<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// The next `n` (at most 32) bits without consuming them.
    pub fn peek(&self, n: u32) -> u32 {
        debug_assert!(n <= 32);
        if n == 0 {
            return 0;
        }
        let byte = self.pos >> 3;
        let shift = (self.pos & 7) as u32;
        let mut v = 0u64;
        for k in 0..5 {
            v |= u64::from(self.data.get(byte + k).copied().unwrap_or(0)) << (8 * k);
        }
        ((v >> shift) & ((1u64 << n) - 1)) as u32
    }

    pub fn get(&mut self, n: u32) -> u32 {
        let v = self.peek(n);
        self.pos += n as usize;
        v
    }

    pub fn get1(&mut self) -> bool {
        self.get(1) != 0
    }

    pub fn skip(&mut self, n: usize) {
        self.pos += n;
    }

    /// Skip to the next byte boundary.
    pub fn align(&mut self) {
        self.pos = (self.pos + 7) & !7;
    }

    /// Bits consumed so far.
    pub fn count(&self) -> usize {
        self.pos
    }

    /// Bits left (negative once the reader ran past the end).
    pub fn left(&self) -> isize {
        (self.data.len() * 8) as isize - self.pos as isize
    }
}

/// A prefix code of at most [`VLC_BITS`] bits, read MSB-first from the little-endian stream.
pub const VLC_BITS: u32 = 13;

#[derive(Clone, Default)]
pub struct Vlc {
    /// Indexed by the next 13 stream bits; `sym << 8 | len`, `len == 0` for an unused code.
    table: Vec<u32>,
}

impl Vlc {
    /// Build from code lengths and code words (the first bit read is the most significant one).
    pub fn new(bits: &[u8], codes: &[u16]) -> Option<Vlc> {
        let mut table = vec![0u32; 1 << VLC_BITS];
        for (sym, (&len, &code)) in bits.iter().zip(codes).enumerate() {
            let len = u32::from(len);
            if len == 0 || len > VLC_BITS {
                return None;
            }
            let rev = u32::from(code).reverse_bits() >> (32 - len);
            let mut idx = rev;
            while idx < (1 << VLC_BITS) {
                if table[idx as usize] != 0 {
                    return None;
                }
                table[idx as usize] = ((sym as u32) << 8) | len;
                idx += 1 << len;
            }
        }
        Some(Vlc { table })
    }

    /// Decode one symbol, `None` for an unused code.
    pub fn get(&self, b: &mut Bits<'_>) -> Option<u32> {
        let e = *self.table.get(b.peek(VLC_BITS) as usize)?;
        let len = e & 0xff;
        if len == 0 {
            return None;
        }
        b.skip(len as usize);
        Some(e >> 8)
    }
}
