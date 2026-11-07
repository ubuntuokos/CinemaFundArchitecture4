//! Test support (feature `test-util`): no free encoder writes E-AC-3 dependent substreams, so tests
//! here and in `cfa4-codecs` make them from ffmpeg's independent syncframes.

use std::cell::RefCell;
use std::ops::Range;

use crate::{Decoder, Error, Result};

thread_local! {
    /// Bit ranges of the fields only independent substreams carry, recorded while one is decoded;
    /// an empty range marks where a dependent substream has chanmape.
    static INDEPENDENT_ONLY: RefCell<Vec<Range<usize>>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn independent_only(bits: Range<usize>) {
    INDEPENDENT_ONLY.with(|m| m.borrow_mut().push(bits));
}

fn take_independent_only() -> Vec<Range<usize>> {
    INDEPENDENT_ONLY.with(|m| std::mem::take(&mut *m.borrow_mut()))
}

/// Rewrite an independent E-AC-3 syncframe into a dependent substream (strmtyp 1) with custom
/// channel map `chanmap` (Table E2.5, bit 0 the MSB). Decoding the syncframe records where the bsi
/// takes chanmape and the bit ranges of the fields only independent substreams carry (the
/// program-level mixing metadata, convsync, convexpstr, convsnroffst; §E2.2.2-4); the chanmap is
/// inserted, those fields are removed, zero padding before auxdatae / crcrsv / crc2 keeps the
/// frame size, and crc2 is recomputed (§E2.2.6).
pub fn make_dependent_substream(frame: &[u8], chanmap: u16) -> Result<Vec<u8>> {
    take_independent_only();
    let h = Decoder::new().decode(frame)?.header;
    let marks = take_independent_only();
    if !h.is_eac3() || h.strmtyp != 0 {
        return Err(Error::Unsupported("not an independent E-AC-3 syncframe".into()));
    }
    let insert_at = marks.iter().find(|m| m.is_empty()).map(|m| m.start).ok_or(Error::Invalid("no bsi"))?;
    let frame = frame.get(..h.frame_bytes).ok_or(Error::Invalid("truncated syncframe"))?;
    let total = frame.len() * 8;
    let tail = total.checked_sub(18).ok_or(Error::Invalid("syncframe too short"))?;
    let bit = |i: usize| frame.get(i / 8).is_some_and(|b| (b >> (7 - i % 8)) & 1 == 1);
    let mut out: Vec<bool> = Vec::with_capacity(total);
    for i in 0..tail {
        if i == insert_at {
            out.push(true); // chanmape
            out.extend((0..16).map(|k| chanmap & (0x8000 >> k) != 0));
        }
        if !marks.iter().any(|m| m.contains(&i)) {
            out.push(bit(i));
        }
    }
    if out.len() > tail {
        return Err(Error::Unsupported("the dependent substream does not fit the syncframe".into()));
    }
    out.resize(tail, false);
    out.extend((tail..total).map(bit));
    let mut f: Vec<u8> = out.chunks(8).map(|c| c.iter().fold(0u8, |a, &b| (a << 1) | b as u8)).collect();
    let n = f.len();
    if let Some(b) = f.get_mut(2) {
        *b = (*b & 0x3F) | 0x40; // strmtyp 1
    }
    // crc2 over the syncframe after the sync word: x^16 + x^15 + x^2 + 1
    let mut crc = 0u16;
    for &byte in f.get(2..n - 2).unwrap_or_default() {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x8005 } else { crc << 1 };
        }
    }
    if let Some(c) = f.get_mut(n - 2..) {
        c.copy_from_slice(&crc.to_be_bytes());
    }
    Ok(f)
}
