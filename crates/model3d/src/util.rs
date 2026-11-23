//! Shared helpers: the memory budget, URI checks, percent- and base64 decoding.

use crate::{Error, Result, invalid};

/// Memory budget (U1-A3): charged before every allocation whose size comes from the input.
pub(crate) struct Budget {
    left: usize,
}

impl Budget {
    pub(crate) fn new(bytes: usize) -> Budget {
        Budget { left: bytes }
    }

    /// Reserve `count * elem` bytes, or fail without allocating.
    pub(crate) fn take(&mut self, count: usize, elem: usize) -> Result<()> {
        let bytes = count.checked_mul(elem).ok_or_else(|| Error::Limit("allocation size overflows".into()))?;
        if bytes > self.left {
            return Err(Error::Limit(format!("memory budget exhausted (need {bytes} bytes, {} left)", self.left)));
        }
        self.left -= bytes;
        Ok(())
    }

    /// Return bytes of a transient buffer that has been freed.
    pub(crate) fn give(&mut self, count: usize, elem: usize) {
        self.left = self.left.saturating_add(count.saturating_mul(elem));
    }

    /// A string kept in the scene.
    pub(crate) fn string(&mut self, s: &str) -> Result<String> {
        self.take(s.len().saturating_add(std::mem::size_of::<String>()), 1)?;
        Ok(s.to_string())
    }
}

/// Whether `uri` is a plain relative path the reader may hand to a resolver: no scheme, no absolute
/// path, no `..` component, no NUL or backslash.
pub fn safe_relative_uri(uri: &str) -> bool {
    if uri.is_empty() || uri.starts_with('/') || uri.contains(['\0', '\\']) {
        return false;
    }
    let first = uri.split('/').next().unwrap_or("");
    if first.contains(':') {
        return false;
    }
    !uri.split('/').any(|c| c == "..")
}

/// Decode `%XX` escapes; a malformed escape is an error.
pub(crate) fn percent_decode(s: &str) -> Result<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = b.get(i + 1..i + 3).ok_or_else(|| Error::Invalid("truncated %-escape in URI".into()))?;
            let hi = hex_val(hex[0]);
            let lo = hex_val(hex[1]);
            let (Some(hi), Some(lo)) = (hi, lo) else {
                return invalid("bad %-escape in URI");
            };
            out.push(hi << 4 | lo);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| Error::Invalid("URI is not UTF-8".into()))
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Decode standard base64 (padding required to be consistent). The size is charged to `budget` first.
pub(crate) fn base64_decode(s: &str, budget: &mut Budget) -> Result<Vec<u8>> {
    let b = s.as_bytes();
    budget.take(b.len() / 4 * 3 + 3, 1)?;
    let mut out = Vec::with_capacity(b.len() / 4 * 3 + 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    let mut pad = 0usize;
    for &c in b {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                pad += 1;
                continue;
            }
            _ => return invalid("bad base64 character"),
        };
        if pad > 0 {
            return invalid("base64 data after padding");
        }
        acc = acc << 6 | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    if pad > 2 || (!b.len().is_multiple_of(4) && pad > 0) {
        return invalid("bad base64 padding");
    }
    Ok(out)
}

/// Parse a finite `f32`; NaN and infinities are errors.
pub(crate) fn finite_f32(s: &str) -> Result<f32> {
    match s.parse::<f32>() {
        Ok(v) if v.is_finite() => Ok(v),
        _ => invalid(format!("bad number {s:?}")),
    }
}
