//! JP2 file format (T.800 Annex I): the boxes around the codestream. Only what decoding needs is
//! read: the signature box and the contiguous codestream box (`jp2c`). Colour specification and
//! other metadata boxes are skipped.

use crate::{Error, Result};

const SIGNATURE: [u8; 12] = [0, 0, 0, 12, b'j', b'P', b' ', b' ', 0x0D, 0x0A, 0x87, 0x0A];

/// The payload of the first `jp2c` box of a JP2 file.
pub(crate) fn codestream(data: &[u8]) -> Result<&[u8]> {
    if !data.starts_with(&SIGNATURE) {
        return Err(Error::Invalid("neither a JPEG 2000 codestream nor a JP2 file"));
    }
    let mut at = SIGNATURE.len();
    while at < data.len() {
        let head = data.get(at..at + 8).ok_or(Error::Truncated)?;
        let len32 = u32::from_be_bytes([head[0], head[1], head[2], head[3]]);
        let kind = &head[4..8];
        let (header, len) = match len32 {
            // to the end of the file
            0 => (8, data.len() - at),
            1 => {
                let x = data.get(at + 8..at + 16).ok_or(Error::Truncated)?;
                let x = u64::from_be_bytes([x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7]]);
                (16, usize::try_from(x).map_err(|_| Error::TooLarge)?)
            }
            n => (8, n as usize),
        };
        if len < header {
            return Err(Error::Invalid("JP2 box shorter than its header"));
        }
        let end = at.checked_add(len).ok_or(Error::Truncated)?;
        if kind == b"jp2c" {
            // a truncated last box still gives what it has; the codestream parser reports damage
            return Ok(data.get(at + header..end.min(data.len())).unwrap_or_default());
        }
        at = end;
    }
    Err(Error::Invalid("JP2 file without a codestream box"))
}
