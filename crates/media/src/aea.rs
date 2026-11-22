//! Sony MiniDisc Studio audio (`.aea`): a 2048-byte header (magic `00 08 00 00`, a 256-byte
//! title, the block count, the channel count at byte 264), then 212-byte ATRAC1 sound units, one
//! per channel in each 512-sample packet at 44.1 kHz (as FFmpeg's `aeadec.c` reads them),
//! decoded through `cfa4-realaudio` and read in place like a WAV's ([`WavSource`]).

use crate::reader::{SharedReader, read_range};
use crate::wav::{Atrac, WavSource};
use crate::{MediaError, Result};

const HEADER: u64 = 2048;
const RATE: u32 = 44_100;

pub fn sniff(b: &[u8]) -> bool {
    b.get(..4) == Some(&[0, 8, 0, 0]) && matches!(b.get(264), Some(1 | 2)) && b.len() > HEADER as usize
}

/// Opens an AEA file through a random-access reader.
pub fn open(name: &str, reader: SharedReader) -> Result<WavSource> {
    let io = |e: std::io::Error| MediaError::Io(format!("{name}: {e}"));
    let len = reader.len();
    if len < HEADER {
        return Err(MediaError::Decode(format!("{name}: truncated AEA header")));
    }
    let h = read_range(&*reader, 0, 265).map_err(io)?;
    if h.get(..4) != Some(&[0, 8, 0, 0]) {
        return Err(MediaError::Decode(format!("{name}: no AEA header")));
    }
    let channels = usize::from(h.get(264).copied().unwrap_or(0));
    if channels != 1 && channels != 2 {
        return Err(MediaError::Unsupported(format!("{name}: {channels} AEA channels")));
    }
    let d = cfa4_realaudio::atrac1::Decoder::new(channels, cfa4_realaudio::atrac1::SU_SIZE * channels)
        .map_err(|e| MediaError::Unsupported(format!("{name}: {e}")))?;
    Ok(WavSource::atrac_stream(name, reader, (HEADER, len - HEADER), RATE, Atrac::One(Box::new(d)), "AEA"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::MediaSource;
    use crate::reader::MemReader;

    fn file(channels: u8, units: usize) -> Vec<u8> {
        let mut f = vec![0u8; 2048];
        f[..4].copy_from_slice(&[0, 8, 0, 0]);
        f[264] = channels;
        f.extend(std::iter::repeat_n(0u8, 212 * usize::from(channels) * units + 100));
        f
    }

    #[test]
    fn header_and_length_are_read() {
        let f = file(2, 5);
        assert!(sniff(&f));
        let src = open("t.aea", Arc::new(MemReader(f.into()))).unwrap();
        let a = src.info().audio().unwrap();
        assert_eq!((a.codec.as_str(), a.channels, a.sample_rate), ("ATRAC1", 2, 44_100));
        assert_eq!(src.info().container, "AEA");
        // five whole packets: the short rest is dropped, as FFmpeg's decoder does
        assert_eq!(src.info().duration, cfa4_time::Tick::from_units(5 * 512, 44_100));
        assert!(src.audio(0, 3000, 44_100).unwrap().channels.iter().all(|c| c.iter().all(|v| v.is_finite())));
    }

    #[test]
    fn hostile_headers_are_errors() {
        let mut f = file(1, 2);
        f[264] = 3;
        assert!(!sniff(&f));
        assert!(open("t.aea", Arc::new(MemReader(f.into()))).is_err());
        assert!(open("t.aea", Arc::new(MemReader(vec![0, 8, 0, 0, 1].into()))).is_err());
        let mut g = file(1, 2);
        g[1] = 9;
        assert!(!sniff(&g));
        assert!(open("t.aea", Arc::new(MemReader(g.into()))).is_err());
    }
}
