//! Highpass band entropy decoding: run/value codewords, companding and dequantization.

use std::sync::OnceLock;

use cfa4_bitstream::BitReader;

use crate::tables::{CODESET9, CODESET9_BAND_END, CODESET17, CODESET17_BAND_END, Rlv};
use crate::{Error, Result};

/// Codes up to this many bits resolve with one table lookup.
const LUT_BITS: u32 = 12;
/// Longest codeword of any codebook.
const MAX_LEN: u32 = 26;

/// A decoded symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sym {
    /// `count` copies of the signed (companded, unquantized) magnitude `value`.
    Run { count: u16, value: u16 },
    /// The band end codeword.
    End,
}

struct Codebook {
    /// Indexed by the next `LUT_BITS` bits: `(len, symbol)`; `len == 0` = longer code.
    lut: Vec<(u8, Sym)>,
    /// Codes longer than `LUT_BITS`, per length (index `len - LUT_BITS - 1`), sorted by code.
    long: Vec<Vec<(u32, Sym)>>,
}

impl Codebook {
    fn new(entries: &[Rlv], end: (u8, u32)) -> Codebook {
        let mut lut = vec![(0u8, Sym::End); 1 << LUT_BITS];
        let mut long = vec![Vec::new(); (MAX_LEN - LUT_BITS) as usize];
        let all = entries.iter().map(|e| (e.len, e.code, Sym::Run { count: e.count, value: e.value })).chain(std::iter::once((end.0, end.1, Sym::End)));
        for (len, code, sym) in all {
            let len32 = u32::from(len);
            if len32 == 0 || len32 > MAX_LEN {
                continue;
            }
            if len32 <= LUT_BITS {
                let shift = LUT_BITS - len32;
                let first = (code as usize) << shift;
                for slot in lut.iter_mut().skip(first).take(1 << shift) {
                    *slot = (len, sym);
                }
            } else if let Some(v) = long.get_mut((len32 - LUT_BITS - 1) as usize) {
                v.push((code, sym));
            }
        }
        for v in &mut long {
            v.sort_unstable_by_key(|&(c, _)| c);
        }
        Codebook { lut, long }
    }

    fn read(&self, br: &mut BitReader) -> Result<Sym> {
        let (len, sym) = self.lut.get(br.peek(LUT_BITS) as usize).copied().unwrap_or((0, Sym::End));
        if len > 0 {
            br.skip(usize::from(len)).map_err(|_| Error::Truncated)?;
            return Ok(sym);
        }
        for (i, codes) in self.long.iter().enumerate() {
            let len = LUT_BITS + 1 + i as u32;
            if codes.is_empty() {
                continue;
            }
            let bits = br.peek(len);
            if let Ok(k) = codes.binary_search_by_key(&bits, |&(c, _)| c) {
                br.skip(len as usize).map_err(|_| Error::Truncated)?;
                return codes.get(k).map(|&(_, s)| s).ok_or(Error::Invalid("bad codeword"));
            }
        }
        Err(Error::Invalid("bad highpass codeword"))
    }
}

fn codebooks() -> &'static [Codebook; 2] {
    static BOOKS: OnceLock<[Codebook; 2]> = OnceLock::new();
    BOOKS.get_or_init(|| [Codebook::new(&CODESET9, CODESET9_BAND_END), Codebook::new(&CODESET17, CODESET17_BAND_END)])
}

/// Expand a coded magnitude with the companding curve of codebook set `set`
/// (0: piecewise linear, 1: cubic, 2: none).
fn decompand(set: u8, m: u16) -> i32 {
    let m = i32::from(m);
    match set {
        0 => {
            let mut v = m;
            if v >= 54 {
                v = (v - 54) * 4 + 54;
            }
            if v >= 40 {
                v = (v - 40) * 4 + 40;
            }
            v
        }
        1 => {
            let c = i64::from(m) * i64::from(m) * i64::from(m) * 768 / (1 << 24);
            m + c as i32
        }
        _ => m,
    }
}

/// Optional table of exact values for coefficients whose dequantized magnitude exceeds `level`.
pub(crate) struct Peaks<'a> {
    pub level: i32,
    /// Little-endian 16-bit values, in coefficient order.
    pub data: &'a [u8],
}

/// Decode one run-length coded band starting at the beginning of `data` into `out`: rows of
/// `stride` coefficients of which the first `width` belong to the band (the coder runs over the
/// padding of rows rounded up to 8 coefficients, the SDK's 16-byte aligned band pitch).
/// Returns the number of bytes used, rounded up to the next 32-bit boundary (where the band
/// trailer tag sits).
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_band(
    data: &[u8],
    out: &mut [i16],
    stride: usize,
    width: usize,
    set: u8,
    quant: i32,
    difference: bool,
    peaks: Option<Peaks>,
) -> Result<usize> {
    let book_index = match set {
        0 => 0,
        1 | 2 => 1,
        _ => return Err(Error::Invalid("unknown codebook")),
    };
    let book = codebooks().get(book_index).ok_or(Error::Invalid("unknown codebook"))?;
    // value -> dequantized coefficient, with the 16-bit wrap of the reference
    let mut table = [0i32; 256];
    for (m, t) in (0u16..).zip(table.iter_mut()) {
        *t = decompand(set, m).saturating_mul(quant);
    }
    let mut br = BitReader::new(data);
    let mut pos = 0usize;
    let mut peak_pos = 0usize;
    loop {
        match book.read(&mut br)? {
            Sym::End => break,
            Sym::Run { count, value } => {
                if value == 0 {
                    pos = pos.saturating_add(usize::from(count)).min(out.len());
                    continue;
                }
                let negative = br.read_bit().map_err(|_| Error::Truncated)?;
                let mag = table.get(usize::from(value)).copied().ok_or(Error::Invalid("coefficient out of range"))?;
                let mut v = if negative { -mag } else { mag } as i16;
                if let Some(p) = &peaks
                    && i32::from(v).abs() > p.level
                {
                    let b = p.data.get(peak_pos..peak_pos + 2).and_then(|b| <[u8; 2]>::try_from(b).ok()).ok_or(Error::Invalid("peak table too short"))?;
                    v = i16::from_le_bytes(b);
                    peak_pos += 2;
                }
                for _ in 0..count {
                    *out.get_mut(pos).ok_or(Error::Invalid("band overflow"))? = v;
                    pos += 1;
                }
            }
        }
    }
    if difference && stride > 0 {
        for row in out.chunks_mut(stride) {
            let mut prev = 0i16;
            for v in row.iter_mut().take(width) {
                *v = v.wrapping_add(prev);
                prev = *v;
            }
        }
    }
    Ok(br.position().div_ceil(32) * 4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfa4_bitstream::BitWriter;

    #[test]
    fn codebooks_are_prefix_free() {
        for (entries, end) in [(&CODESET9[..], CODESET9_BAND_END), (&CODESET17[..], CODESET17_BAND_END)] {
            let all: Vec<(u8, u32)> = entries.iter().map(|e| (e.len, e.code)).chain(std::iter::once(end)).collect();
            for (i, &(la, ca)) in all.iter().enumerate() {
                for &(lb, cb) in &all[i + 1..] {
                    let l = la.min(lb);
                    assert_ne!(ca >> (la - l), cb >> (lb - l), "codes {la}:{ca:x} and {lb}:{cb:x} share a prefix");
                }
            }
        }
    }

    #[test]
    fn decodes_every_codeword() {
        for (set, entries, end) in [(0u8, &CODESET9[..], CODESET9_BAND_END), (1, &CODESET17[..], CODESET17_BAND_END)] {
            let mut w = BitWriter::new();
            let mut expect = Vec::new();
            for e in entries {
                w.write_bits(e.code, u32::from(e.len));
                if e.value != 0 {
                    w.write_bit(e.value % 2 == 1);
                    let m = decompand(set, e.value) as i16;
                    expect.extend(std::iter::repeat_n(if e.value % 2 == 1 { -m } else { m }, usize::from(e.count)));
                } else {
                    expect.extend(std::iter::repeat_n(0, usize::from(e.count)));
                }
            }
            w.write_bits(end.1, u32::from(end.0));
            let data = w.finish();
            let mut out = vec![0i16; expect.len()];
            let n = out.len();
            let used = decode_band(&data, &mut out, n, n, set, 1, false, None).expect("decode");
            assert_eq!(out, expect);
            assert_eq!(used % 4, 0);
        }
    }

    #[test]
    fn peaks_replace_large_coefficients_and_difference_coding_accumulates() {
        let code = |v: u16| CODESET17.iter().find(|e| e.value == v && e.count == 1).map(|e| (e.code, u32::from(e.len))).expect("code");
        let mut w = BitWriter::new();
        for (v, neg) in [(2u16, false), (5, true), (1, false), (5, false)] {
            let (c, l) = code(v);
            w.write_bits(c, l);
            w.write_bit(neg);
        }
        w.write_bits(CODESET17_BAND_END.1, u32::from(CODESET17_BAND_END.0));
        let data = w.finish();
        let peaks = [1234i16.to_le_bytes(), (-77i16).to_le_bytes()].concat();
        let mut out = vec![0i16; 4];
        decode_band(&data, &mut out, 4, 4, 2, 1, false, Some(Peaks { level: 3, data: &peaks })).expect("decode");
        assert_eq!(out, vec![2, 1234, 1, -77]);
        let mut out = vec![0i16; 4];
        decode_band(&data, &mut out, 4, 3, 2, 2, true, None).expect("decode");
        // quantization 2, then a running sum over the first 3 (of 4) coefficients of the row
        assert_eq!(out, vec![4, -6, -4, 10]);
    }

    #[test]
    fn cubic_and_linear_companding() {
        assert_eq!(decompand(1, 0), 0);
        assert_eq!(decompand(1, 100), 100 + 1_000_000 * 768 / 16_777_216);
        assert_eq!(decompand(0, 39), 39);
        assert_eq!(decompand(0, 41), 44);
        assert_eq!(decompand(0, 55), (58 - 40) * 4 + 40);
        assert_eq!(decompand(2, 200), 200);
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        let mut out = vec![0i16; 16];
        assert!(decode_band(&[0xff; 8], &mut out, 8, 4, 1, 3, true, None).is_err());
        assert!(decode_band(&[], &mut out, 8, 4, 0, 3, false, None).is_err());
        assert!(decode_band(&[0; 4], &mut out, 8, 4, 7, 3, false, None).is_err());
        assert!(decode_band(&[0; 4], &mut out, 0, 0, 1, 3, true, None).is_err());
    }
}
