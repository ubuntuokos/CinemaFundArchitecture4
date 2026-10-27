//! The MQ arithmetic decoder (T.800 Annex C), in the software conventions of C.3: the code
//! register `c` holds the arriving bytes above bit 16, `a` the interval.

/// Contexts of EBCOT tier-1 (D.3): zero coding 0-8, sign coding 9-13, magnitude refinement
/// 14-16, run-length 17, uniform 18.
pub(crate) const CONTEXTS: usize = 19;
pub(crate) const CTX_RL: usize = 17;
pub(crate) const CTX_UNIFORM: usize = 18;

/// Qe, next state after an MPS, next state after an LPS, MPS switch (Table C.2).
const STATES: [(u32, u8, u8, bool); 47] = [
    (0x5601, 1, 1, true),
    (0x3401, 2, 6, false),
    (0x1801, 3, 9, false),
    (0x0AC1, 4, 12, false),
    (0x0521, 5, 29, false),
    (0x0221, 38, 33, false),
    (0x5601, 7, 6, true),
    (0x5401, 8, 14, false),
    (0x4801, 9, 14, false),
    (0x3801, 10, 14, false),
    (0x3001, 11, 17, false),
    (0x2401, 12, 18, false),
    (0x1C01, 13, 20, false),
    (0x1601, 29, 21, false),
    (0x5601, 15, 14, true),
    (0x5401, 16, 14, false),
    (0x5101, 17, 15, false),
    (0x4801, 18, 16, false),
    (0x3801, 19, 17, false),
    (0x3401, 20, 18, false),
    (0x3001, 21, 19, false),
    (0x2801, 22, 19, false),
    (0x2401, 23, 20, false),
    (0x2201, 24, 21, false),
    (0x1C01, 25, 22, false),
    (0x1801, 26, 23, false),
    (0x1601, 27, 24, false),
    (0x1401, 28, 25, false),
    (0x1201, 29, 26, false),
    (0x1101, 30, 27, false),
    (0x0AC1, 31, 28, false),
    (0x09C1, 32, 29, false),
    (0x08A1, 33, 30, false),
    (0x0521, 34, 31, false),
    (0x0441, 35, 32, false),
    (0x02A1, 36, 33, false),
    (0x0221, 37, 34, false),
    (0x0141, 38, 35, false),
    (0x0111, 39, 36, false),
    (0x0085, 40, 37, false),
    (0x0049, 41, 38, false),
    (0x0025, 42, 39, false),
    (0x0015, 43, 40, false),
    (0x0009, 44, 41, false),
    (0x0005, 45, 42, false),
    (0x0001, 45, 43, false),
    (0x5601, 46, 46, false),
];

/// A context: its state index and MPS, as `state << 1 | mps` (an index into [`TABLE`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Ctx(u8);

/// One entry per state and MPS: Qe, the MPS, and the contexts after an MPS and after an LPS (the
/// MPS switch applied).
#[derive(Clone, Copy)]
struct Entry {
    qe: u32,
    mps: u8,
    nmps: Ctx,
    nlps: Ctx,
}

const TABLE: [Entry; 94] = {
    let mut t = [Entry { qe: 0, mps: 0, nmps: Ctx(0), nlps: Ctx(0) }; 94];
    let mut i = 0;
    while i < 94 {
        let (qe, nmps, nlps, switch) = STATES[i / 2];
        let mps = (i % 2) as u8;
        let lps_mps = if switch { 1 - mps } else { mps };
        t[i] = Entry { qe, mps, nmps: Ctx(nmps << 1 | mps), nlps: Ctx(nlps << 1 | lps_mps) };
        i += 1;
    }
    t
};

/// The initial states of the contexts (Table D.7): all 0 except zero coding context 0 (state 4),
/// run-length (3) and uniform (46).
pub(crate) fn initial_contexts() -> [Ctx; CONTEXTS] {
    let mut c = [Ctx::default(); CONTEXTS];
    c[0] = Ctx(4 << 1);
    c[CTX_RL] = Ctx(3 << 1);
    c[CTX_UNIFORM] = Ctx(46 << 1);
    c
}

pub(crate) struct Mq<'a> {
    data: &'a [u8],
    /// Position of the byte `B` (C.3.4).
    bp: usize,
    c: u32,
    a: u32,
    ct: u32,
}

impl<'a> Mq<'a> {
    /// INITDEC. Bytes past the end of `data` read as 0xFF (the terminating marker the decoder
    /// may run into).
    pub fn new(data: &'a [u8]) -> Self {
        let mut m = Mq { data, bp: 0, c: 0, a: 0x8000, ct: 0 };
        m.c = m.byte(0) << 16;
        m.byte_in();
        m.c <<= 7;
        m.ct = m.ct.saturating_sub(7);
        m.a = 0x8000;
        m
    }

    #[inline]
    fn byte(&self, at: usize) -> u32 {
        u32::from(self.data.get(at).copied().unwrap_or(0xFF))
    }

    /// BYTEIN.
    fn byte_in(&mut self) {
        let next = self.byte(self.bp + 1);
        if self.byte(self.bp) == 0xFF {
            if next > 0x8F {
                self.c = self.c.wrapping_add(0xFF00);
                self.ct = 8;
            } else {
                self.bp += 1;
                self.c = self.c.wrapping_add(next << 9);
                self.ct = 7;
            }
        } else {
            self.bp += 1;
            self.c = self.c.wrapping_add(next << 8);
            self.ct = 8;
        }
    }

    fn renorm(&mut self) {
        loop {
            if self.ct == 0 {
                self.byte_in();
            }
            self.a <<= 1;
            self.c <<= 1;
            self.ct = self.ct.saturating_sub(1);
            if self.a >= 0x8000 {
                break;
            }
        }
    }

    /// DECODE: one decision in context `cx`.
    #[inline]
    pub fn decode(&mut self, cx: &mut Ctx) -> u8 {
        let e = TABLE.get(usize::from(cx.0)).copied().unwrap_or(TABLE[0]);
        let qe = e.qe;
        self.a = self.a.wrapping_sub(qe);
        if (self.c >> 16) < qe {
            // LPS exchange
            let d = if self.a < qe {
                *cx = e.nmps;
                e.mps
            } else {
                *cx = e.nlps;
                1 - e.mps
            };
            self.a = qe;
            self.renorm();
            d
        } else {
            self.c = self.c.wrapping_sub(qe << 16);
            if self.a & 0x8000 != 0 {
                return e.mps;
            }
            // MPS exchange
            let d = if self.a < qe {
                *cx = e.nlps;
                1 - e.mps
            } else {
                *cx = e.nmps;
                e.mps
            };
            self.renorm();
            d
        }
    }
}

/// The raw (bypass) decoder of the selective arithmetic coding bypass (D.6): bits straight from
/// the bytes, most significant first; after a 0xFF byte the next byte's top bit is a stuffed 0.
/// Past the end of `data` it reads 1 bits, as from a terminating 0xFF.
pub(crate) struct Raw<'a> {
    data: &'a [u8],
    pos: usize,
    c: u32,
    ct: u32,
}

impl<'a> Raw<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Raw { data, pos: 0, c: 0, ct: 0 }
    }

    #[inline]
    pub fn bit(&mut self) -> u8 {
        if self.ct == 0 {
            let stuffed = self.c == 0xFF;
            let next = self.data.get(self.pos).copied();
            match next {
                // a marker (0xFF followed by more than 0x8F) ends the segment
                Some(b) if !(stuffed && b > 0x8F) => {
                    self.c = u32::from(b);
                    self.pos += 1;
                    self.ct = if stuffed { 7 } else { 8 };
                }
                _ => {
                    self.c = 0xFF;
                    self.ct = 8;
                }
            }
        }
        self.ct -= 1;
        ((self.c >> self.ct) & 1) as u8
    }
}

/// The MQ arithmetic encoder (C.2, software conventions): `c` holds the code register, `b` the
/// byte that a carry may still change, `out` the bytes before it.
pub(crate) struct MqEncoder {
    a: u32,
    c: u32,
    ct: u32,
    b: u8,
    /// `b` is a real byte (the first BYTEOUT has happened).
    started: bool,
    out: Vec<u8>,
}

impl MqEncoder {
    /// INITENC.
    pub fn new() -> Self {
        MqEncoder { a: 0x8000, c: 0, ct: 12, b: 0, started: false, out: Vec::new() }
    }

    /// ENCODE: decision `d` in context `cx`.
    #[inline]
    pub fn encode(&mut self, d: u8, cx: &mut Ctx) {
        let e = TABLE.get(usize::from(cx.0)).copied().unwrap_or(TABLE[0]);
        let qe = e.qe;
        self.a -= qe;
        if d == e.mps {
            // CODEMPS
            if self.a & 0x8000 == 0 {
                if self.a < qe {
                    self.a = qe;
                } else {
                    self.c += qe;
                }
                *cx = e.nmps;
                self.renorm();
            } else {
                self.c += qe;
            }
        } else {
            // CODELPS
            if self.a < qe {
                self.c += qe;
            } else {
                self.a = qe;
            }
            *cx = e.nlps;
            self.renorm();
        }
    }

    fn renorm(&mut self) {
        loop {
            self.a <<= 1;
            self.c <<= 1;
            self.ct -= 1;
            if self.ct == 0 {
                self.byte_out();
            }
            if self.a & 0x8000 != 0 {
                break;
            }
        }
    }

    fn push(&mut self, b: u8) {
        if self.started {
            self.out.push(self.b);
        }
        self.started = true;
        self.b = b;
    }

    /// BYTEOUT, with bit stuffing after 0xFF and the carry into `b`.
    fn byte_out(&mut self) {
        if self.b == 0xFF && self.started {
            self.push((self.c >> 20) as u8);
            self.c &= 0xF_FFFF;
            self.ct = 7;
        } else if self.c < 0x800_0000 {
            self.push((self.c >> 19) as u8);
            self.c &= 0x7_FFFF;
            self.ct = 8;
        } else {
            self.b = self.b.wrapping_add(1);
            if self.b == 0xFF {
                self.c &= 0x7FF_FFFF;
                self.push((self.c >> 20) as u8);
                self.c &= 0xF_FFFF;
                self.ct = 7;
            } else {
                self.push((self.c >> 19) as u8);
                self.c &= 0x7_FFFF;
                self.ct = 8;
            }
        }
    }

    /// Where a truncation after the decisions so far must end, for [`truncation_length`]: the
    /// bytes written and the pending byte, and the code register's bits still to come down to
    /// the interval's precision (its bit 0). The decoder pads with 1 bits (0xFF), which keeps the
    /// code value inside the interval when every bit above that precision is present.
    pub fn truncation_point(&self) -> (usize, u32) {
        (self.out.len() + usize::from(self.started), 27u32.saturating_sub(self.ct))
    }

    /// FLUSH (C.2.9): the terminated codeword, without a final 0xFF.
    pub fn finish(mut self) -> Vec<u8> {
        // SETBITS
        let temp = self.c + self.a;
        self.c |= 0xFFFF;
        if self.c >= temp {
            self.c -= 0x8000;
        }
        self.c <<= self.ct;
        self.byte_out();
        self.c <<= self.ct;
        self.byte_out();
        if self.started {
            self.out.push(self.b);
        }
        while self.out.last() == Some(&0xFF) {
            self.out.pop();
        }
        self.out
    }
}

/// The length of the terminated codeword `data` a decoder needs for the decisions up to a
/// [`MqEncoder::truncation_point`]: the bytes there, then bytes until `bits` more code bits are
/// covered (7 per byte after 0xFF, which carries a stuffed bit).
pub(crate) fn truncation_length(data: &[u8], (base, bits): (usize, u32)) -> usize {
    let mut len = base.min(data.len());
    let mut got = 0;
    while got < bits && len < data.len() {
        got += if len > 0 && data[len - 1] == 0xFF { 7 } else { 8 };
        len += 1;
    }
    len
}
