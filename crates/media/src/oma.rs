//! Sony OpenMG audio (`.oma`, `.omg`, `.aa3`): optional `ea3` ID3v2 tags, a 96-byte `EA3`
//! header, then fixed-size ATRAC3 or ATRAC3plus frames (as FFmpeg's `omadec.c` reads them),
//! decoded through `cfa4-realaudio` and read in place like a WAV's ([`WavSource`]). Encrypted
//! (DRM) files and the MP3, LPCM and ATRAC Advanced Lossless variants are refused.

use crate::reader::{SharedReader, read_range};
use crate::wav::{Atrac, WavSource};
use crate::{MediaError, Result};

const EA3_HEADER: usize = 96;
/// `ea3` tags skipped before the header (FFmpeg reads one after another).
const MAX_TAGS: usize = 16;
/// OMA sample rates in units of 100 Hz (`ff_oma_srate_tab`).
const RATES: [u32; 8] = [320, 441, 480, 882, 960, 0, 0, 0];
/// Channel counts of the ATRAC3plus channel ids 1..=7.
const CHANNELS: [u16; 7] = [1, 2, 3, 4, 6, 7, 8];

/// Bytes of an `ea3` ID3v2 tag starting at `b`, if it is one (`ff_id3v2_match`).
fn tag_len(b: &[u8]) -> Option<u64> {
    let h = b.get(..10)?;
    if &h[..3] != b"ea3" || h[3] == 0xFF || h[4] == 0xFF || h[6..10].iter().any(|&x| x & 0x80 != 0) {
        return None;
    }
    let size = h[6..10].iter().fold(0u64, |a, &x| a << 7 | u64::from(x));
    Some(size + 10 + if h[5] & 0x10 != 0 { 10 } else { 0 })
}

fn is_header(h: &[u8]) -> bool {
    h.len() >= 6 && &h[..3] == b"EA3" && h[4] == 0 && usize::from(h[5]) == EA3_HEADER
}

pub fn sniff(b: &[u8]) -> bool {
    tag_len(b).is_some() || is_header(b)
}

/// Opens an OpenMG file through a random-access reader.
pub fn open(name: &str, reader: SharedReader) -> Result<WavSource> {
    let io = |e: std::io::Error| MediaError::Io(format!("{name}: {e}"));
    let bad = |what: &str| MediaError::Decode(format!("{name}: {what}"));
    let len = reader.len();
    let mut pos = 0u64;
    for _ in 0..MAX_TAGS {
        if pos.saturating_add(10) > len {
            break;
        }
        let h = read_range(&*reader, pos, 10).map_err(io)?;
        match tag_len(&h) {
            Some(n) => pos = pos.saturating_add(n),
            None => break,
        }
    }
    if pos.saturating_add(EA3_HEADER as u64) > len {
        return Err(bad("no OpenMG header"));
    }
    let h = read_range(&*reader, pos, EA3_HEADER).map_err(io)?;
    if !is_header(&h) {
        return Err(bad("no OpenMG header"));
    }
    let eid = u16::from_be_bytes([h[6], h[7]]);
    if eid != 0xFFFF && eid != 0xFF80 {
        return Err(MediaError::Unsupported(format!("{name}: encrypted OpenMG file")));
    }
    let params = u32::from(h[33]) << 16 | u32::from(h[34]) << 8 | u32::from(h[35]);
    let rate = RATES[((params >> 13) & 7) as usize] * 100;
    if rate == 0 {
        return Err(MediaError::Unsupported(format!("{name}: OpenMG sample rate")));
    }
    let unsupported = |e: cfa4_realaudio::Error| MediaError::Unsupported(format!("{name}: {e}"));
    let d = match h[32] {
        0 => {
            // FFmpeg fakes WAV extradata: joint stereo flag as the coding mode, frame factor 1
            let js = u8::from(params >> 17 & 1 != 0);
            let mut extradata = vec![0u8; 14];
            extradata[0] = 1;
            extradata[2..6].copy_from_slice(&rate.to_le_bytes());
            extradata[6] = js;
            extradata[8] = js;
            extradata[10] = 1;
            let p = cfa4_realaudio::atrac3::Params { channels: 2, block_align: (params & 0x3FF) as usize * 8, extradata };
            Atrac::Three(Box::new(cfa4_realaudio::atrac3::Decoder::new(&p).map_err(unsupported)?))
        }
        1 => {
            let id = (params >> 10) & 7;
            let ch = *CHANNELS.get((id as usize).wrapping_sub(1)).ok_or_else(|| bad("ATRAC3plus channel id"))?;
            Atrac::Plus(Box::new(cfa4_realaudio::atrac3p::Decoder::new(ch, (params & 0x3FF) as usize * 8 + 8).map_err(unsupported)?))
        }
        c => return Err(MediaError::Unsupported(format!("{name}: OpenMG codec {c}"))),
    };
    let start = pos + EA3_HEADER as u64;
    Ok(WavSource::atrac_stream(name, reader, (start, len - start), rate, d, "OpenMG"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::MediaSource;
    use crate::reader::MemReader;

    fn header(codec: u8, params: u32) -> Vec<u8> {
        let mut h = vec![0u8; 96];
        h[..3].copy_from_slice(b"EA3");
        h[3] = 1;
        h[5] = 96;
        h[6] = 0xFF;
        h[7] = 0xFF;
        h[32] = codec;
        h[33..36].copy_from_slice(&params.to_be_bytes()[1..]);
        h
    }

    fn open_bytes(b: Vec<u8>) -> Result<WavSource> {
        open("t.oma", Arc::new(MemReader(b.into())))
    }

    #[test]
    fn headers_are_checked() {
        // ATRAC3plus stereo 44.1 kHz, 280-byte frames
        let p = 1 << 13 | 2 << 10 | 34;
        let mut f = header(1, p);
        f.extend(std::iter::repeat_n(0u8, 280 * 3 + 10));
        let src = open_bytes(f.clone()).unwrap();
        let a = src.info().audio().unwrap();
        assert_eq!((a.codec.as_str(), a.channels, a.sample_rate), ("ATRAC3plus", 2, 44_100));
        assert_eq!(src.info().container, "OpenMG");
        // four frames: the short last one counts, as in FFmpeg
        assert_eq!(src.info().duration, cfa4_time::Tick::from_units(4 * 2048, 44_100));
        assert!(src.audio(0, 9000, 44_100).unwrap().channels.iter().all(|c| c.iter().all(|v| v.is_finite())));
        // behind an ea3 tag
        let mut tagged = b"ea3\x03\x00\x00\x00\x00\x00\x04".to_vec();
        tagged.extend_from_slice(&[0; 4]);
        tagged.extend_from_slice(&f);
        assert!(sniff(&tagged));
        assert!(open_bytes(tagged).is_ok());
        // ATRAC3 joint stereo, 384-byte frames
        let mut f3 = header(0, 1 << 17 | 1 << 13 | 48);
        f3.extend(std::iter::repeat_n(0u8, 384 * 2));
        assert_eq!(open_bytes(f3).unwrap().info().audio().unwrap().codec, "ATRAC3");
        // encrypted, unknown codec, rate or channel id, truncated
        let mut enc = header(1, p);
        enc[6] = 0;
        assert!(matches!(open_bytes(enc), Err(MediaError::Unsupported(_))));
        assert!(open_bytes(header(3, p)).is_err());
        assert!(open_bytes(header(1, 5 << 13 | 2 << 10 | 34)).is_err());
        assert!(open_bytes(header(1, 1 << 13 | 34)).is_err());
        assert!(open_bytes(header(1, p)[..50].to_vec()).is_err());
        assert!(open_bytes(b"ea3\x03\x00\x00\x7f\x7f\x7f\x7f".to_vec()).is_err());
    }
}
