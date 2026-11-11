//! Snappy decompression, from the public "Snappy compressed format description".

use crate::error::{Error, Result, ensure};

/// Decompress one Snappy block. The declared length must not exceed `max`.
pub fn decompress(src: &[u8], max: usize) -> Result<Vec<u8>> {
    let mut pos = 0usize;
    // preamble: the uncompressed length as a little-endian varint (at most 5 bytes)
    let mut len: u64 = 0;
    let mut shift = 0u32;
    loop {
        let b = *src.get(pos).ok_or_else(|| Error::Invalid("snappy: truncated length".into()))?;
        pos += 1;
        len |= u64::from(b & 0x7F) << shift;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
        ensure!(shift < 35, "snappy: length varint too long");
    }
    let len = usize::try_from(len).map_err(|_| Error::Invalid("snappy: length too large".into()))?;
    ensure!(len <= max, "snappy: {len} bytes declared, at most {max} expected");
    let mut out: Vec<u8> = Vec::with_capacity(len);
    let read_le = |pos: usize, n: usize| -> Result<usize> {
        let b = src.get(pos..pos + n).ok_or_else(|| Error::Invalid("snappy: truncated element".into()))?;
        Ok(b.iter().rev().fold(0usize, |acc, &x| (acc << 8) | usize::from(x)))
    };
    while out.len() < len {
        let tag = *src.get(pos).ok_or_else(|| Error::Invalid("snappy: truncated stream".into()))?;
        pos += 1;
        let upper = usize::from(tag >> 2);
        let (offset, n) = match tag & 3 {
            0 => {
                let n = if upper < 60 {
                    upper + 1
                } else {
                    let bytes = upper - 59;
                    let v = read_le(pos, bytes)?;
                    pos += bytes;
                    v.checked_add(1).ok_or_else(|| Error::Invalid("snappy: literal length".into()))?
                };
                let end = pos.checked_add(n).ok_or_else(|| Error::Invalid("snappy: literal length".into()))?;
                let lit = src.get(pos..end).ok_or_else(|| Error::Invalid("snappy: truncated literal".into()))?;
                ensure!(out.len() + n <= len, "snappy: literal past the declared length");
                out.extend_from_slice(lit);
                pos = end;
                continue;
            }
            1 => {
                let lo = read_le(pos, 1)?;
                pos += 1;
                (((upper >> 3) << 8) | lo, (upper & 7) + 4)
            }
            2 => {
                let o = read_le(pos, 2)?;
                pos += 2;
                (o, upper + 1)
            }
            _ => {
                let o = read_le(pos, 4)?;
                pos += 4;
                (o, upper + 1)
            }
        };
        ensure!(offset != 0 && offset <= out.len(), "snappy: copy offset {offset} out of range");
        ensure!(out.len() + n <= len, "snappy: copy past the declared length");
        let start = out.len() - offset;
        for i in 0..n {
            let b = out.get(start + i).copied().unwrap_or(0);
            out.push(b);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_and_overlapping_copy() {
        // "xababab": literal "xab", copy offset 2 length 4 (2-byte offset form)
        let s = [7, 2 << 2, b'x', b'a', b'b', (3 << 2) | 2, 2, 0];
        assert_eq!(decompress(&s, 100).unwrap(), b"xababab");
        // the same copy with a 1-byte offset: len 4 → bits 2..4 = 0, offset 2
        let s = [7, 2 << 2, b'x', b'a', b'b', 1, 2];
        assert_eq!(decompress(&s, 100).unwrap(), b"xababab");
    }

    #[test]
    fn long_literal_and_errors() {
        let mut s = vec![70, 60 << 2, 69];
        s.extend(0..70u8);
        assert_eq!(decompress(&s, 70).unwrap(), (0..70u8).collect::<Vec<_>>());
        assert!(decompress(&s, 69).is_err());
        assert!(decompress(&s[..20], 70).is_err());
        assert!(decompress(&[4, 1, 0], 10).is_err()); // copy before any data
        assert!(decompress(&[0xFF; 6], 10).is_err());
    }
}
