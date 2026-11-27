//! A little-endian bit reader and FFmpeg's `ff_vlc_init_from_lengths` tables (`VLC_INIT_LE`).

/// Bits read least significant first (FFmpeg's `BITSTREAM_READER_LE`); past the end they are zero.
pub(crate) struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    pub(crate) fn new(d: &'a [u8]) -> Self {
        Self { d, pos: 0 }
    }

    pub(crate) fn count(&self) -> usize {
        self.pos
    }

    /// Bits left (negative once read past the end).
    pub(crate) fn left(&self) -> i64 {
        (self.d.len() * 8) as i64 - self.pos as i64
    }

    pub(crate) fn skip(&mut self, n: usize) {
        self.pos += n;
    }

    pub(crate) fn get1(&mut self) -> u32 {
        let b = self.peek_bit(0);
        self.pos += 1;
        b
    }

    fn peek_bit(&self, k: usize) -> u32 {
        let p = self.pos + k;
        self.d.get(p >> 3).map_or(0, |&x| u32::from(x >> (p & 7)) & 1)
    }

    pub(crate) fn get(&mut self, n: u32) -> u32 {
        let mut v = 0u32;
        for i in 0..n.min(32) {
            v |= self.peek_bit(i as usize) << i;
        }
        self.pos += n as usize;
        v
    }
}

/// A prefix code: `(length, code, symbol)`, codes read most significant bit first.
pub(crate) struct Vlc {
    codes: Vec<(u32, u32, i32)>,
    max_len: u32,
}

impl Vlc {
    /// From `[symbol, length]` pairs as FFmpeg builds them (codes follow in table order, the
    /// symbol is the first entry minus 1; -1 is the escape).
    pub(crate) fn new(tab: &[[u8; 2]]) -> Self {
        let mut codes = Vec::new();
        let mut code: u64 = 0;
        let mut max_len = 0;
        for e in tab {
            let len = u32::from(e[1]);
            if len == 0 {
                continue;
            }
            codes.push((len, (code >> (32 - len)) as u32, i32::from(e[0]) - 1));
            max_len = max_len.max(len);
            code += 1u64 << (32 - len);
        }
        Self { codes, max_len }
    }

    /// The symbol of the next code (consuming it), or -1 when no code matches (nothing consumed).
    pub(crate) fn get(&self, gb: &mut Bits<'_>) -> i32 {
        let mut code = 0u32;
        for len in 1..=self.max_len {
            code = code << 1 | gb.peek_bit(len as usize - 1);
            if let Some(&(_, _, sym)) = self.codes.iter().find(|&&(l, c, _)| l == len && c == code) {
                gb.pos += len as usize;
                return sym;
            }
        }
        -1
    }
}
