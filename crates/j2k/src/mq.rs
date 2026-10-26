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

/// A context: its state index and MPS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Ctx {
    state: u8,
    mps: u8,
}

/// The initial states of the contexts (Table D.7): all 0 except zero coding context 0 (state 4),
/// run-length (3) and uniform (46).
pub(crate) fn initial_contexts() -> [Ctx; CONTEXTS] {
    let mut c = [Ctx::default(); CONTEXTS];
    c[0].state = 4;
    c[CTX_RL].state = 3;
    c[CTX_UNIFORM].state = 46;
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
    pub fn decode(&mut self, cx: &mut Ctx) -> u8 {
        let (qe, nmps, nlps, switch) = STATES[usize::from(cx.state).min(46)];
        self.a = self.a.wrapping_sub(qe);
        let d;
        if (self.c >> 16) < qe {
            // LPS exchange
            if self.a < qe {
                d = cx.mps;
                cx.state = nmps;
            } else {
                d = 1 - cx.mps;
                if switch {
                    cx.mps = 1 - cx.mps;
                }
                cx.state = nlps;
            }
            self.a = qe;
            self.renorm();
        } else {
            self.c = self.c.wrapping_sub(qe << 16);
            if self.a & 0x8000 == 0 {
                // MPS exchange
                if self.a < qe {
                    d = 1 - cx.mps;
                    if switch {
                        cx.mps = 1 - cx.mps;
                    }
                    cx.state = nlps;
                } else {
                    d = cx.mps;
                    cx.state = nmps;
                }
                self.renorm();
            } else {
                d = cx.mps;
            }
        }
        d
    }
}
