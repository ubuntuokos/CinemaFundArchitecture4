//! AMV files (`.amv`): the RIFF-like container of cheap MP4 players, its JPEG-variant video and its
//! IMA ADPCM audio, codec backlog CB-32.1. Ported from FFmpeg n7.1.1 (`avidec.c`'s AMV branch,
//! `sp5xdec.c`, `mjpegdec.c`, `simple_idct.c`, `adpcm.c`); LGPL-2.1-or-later, see `COPYING.LGPL-2.1`.

mod jpeg;
mod tables;

pub use jpeg::{Decoder, Picture};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("AMV: {0}")]
    Invalid(String),
}

/// Whether `head` starts an AMV file (`RIFF`, any size, `AMV `).
pub fn probe(head: &[u8]) -> bool {
    head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"AMV "
}

fn le16(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?))
}

fn le32(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

/// Where a chunk's data is in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub offset: usize,
    pub len: usize,
}

/// A parsed AMV file.
#[derive(Clone, Debug)]
pub struct Amv {
    pub width: u32,
    pub height: u32,
    /// Frames per second as a fraction.
    pub frame_rate: (u32, u32),
    pub video: Vec<Chunk>,
    pub audio: Vec<Chunk>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl Amv {
    /// Reads the headers and the chunk table. The sizes of the `RIFF` and `LIST` chunks in these
    /// files are zero, so the lists are walked into, not skipped.
    pub fn parse(d: &[u8]) -> Result<Self, Error> {
        if !probe(d) {
            return Err(Error::Invalid("not an AMV file".into()));
        }
        let mut p = 12;
        let (mut width, mut height, mut period, mut fps) = (0u32, 0u32, 0u32, 0u32);
        let (mut sample_rate, mut channels) = (0u32, 0u16);
        let mut strh = 0;
        let (mut video, mut audio) = (Vec::new(), Vec::new());
        let mut in_movi = false;
        while p + 8 <= d.len() {
            let id = &d[p..p + 4];
            let size = le32(d, p + 4).unwrap_or(0) as usize;
            if id == b"LIST" {
                // LIST, size (unreliable), type
                in_movi = d.get(p + 8..p + 12) == Some(b"movi");
                p += 12;
                continue;
            }
            let body = p + 8;
            let end = body.saturating_add(size);
            match id {
                b"amvh" => {
                    period = le32(d, body).unwrap_or(0);
                    width = le32(d, body + 32).unwrap_or(0);
                    height = le32(d, body + 36).unwrap_or(0);
                    fps = le32(d, body + 40).unwrap_or(0);
                }
                b"strh" => strh += 1,
                b"strf" if strh == 2 => {
                    // the audio stream's WAVEFORMATEX
                    channels = le16(d, body + 2).unwrap_or(0);
                    sample_rate = le32(d, body + 4).unwrap_or(0);
                }
                b"00dc" if in_movi => video.push(Chunk { offset: body, len: size.min(d.len().saturating_sub(body)) }),
                b"01wb" if in_movi => audio.push(Chunk { offset: body, len: size.min(d.len().saturating_sub(body)) }),
                _ => {}
            }
            // chunks are not padded to even sizes in AMV files
            p = end;
        }
        if video.is_empty() {
            return Err(Error::Invalid("no video frames".into()));
        }
        // the header's frame period in microseconds is exact; the whole-number rate is its fallback
        let frame_rate = if period > 0 {
            (1_000_000, period)
        } else if fps > 0 {
            (fps, 1)
        } else {
            return Err(Error::Invalid("no frame rate".into()));
        };
        if width == 0 || height == 0 {
            return Err(Error::Invalid("no picture size".into()));
        }
        Ok(Self { width, height, frame_rate, video, audio, sample_rate, channels: channels.max(1) })
    }
}

/// Samples a `01wb` chunk holds (FFmpeg's `get_nb_samples` for ADPCM IMA AMV).
pub fn audio_samples(chunk: &[u8]) -> usize {
    let coded = le32(chunk, 4).unwrap_or(0) as usize;
    chunk.len().saturating_sub(8).saturating_mul(2).min(coded)
}

/// Decodes a `01wb` chunk (header: predictor, step index, 6 bytes; then the nibbles, high
/// first). Every chunk stands alone.
pub fn decode_audio(chunk: &[u8]) -> Result<Vec<i16>, Error> {
    let (Some(pred), Some(&step)) = (le16(chunk, 0), chunk.get(2)) else {
        return Err(Error::Invalid("short audio chunk".into()));
    };
    if step > 88 || chunk.len() < 8 {
        return Err(Error::Invalid(format!("step index {step}")));
    }
    let n = audio_samples(chunk);
    let mut predictor = i32::from(pred as i16);
    let mut index = i32::from(step);
    let mut out = Vec::with_capacity(n);
    let mut nibble = |nib: u8| {
        let st = i32::from(tables::STEP[index as usize]);
        index = (index + i32::from(tables::INDEX[usize::from(nib)])).clamp(0, 88);
        let diff = ((2 * i32::from(nib & 7) + 1) * st) >> 3;
        predictor = if nib & 8 != 0 { predictor - diff } else { predictor + diff }.clamp(-32768, 32767);
        predictor as i16
    };
    for k in 0..n {
        let b = chunk[8 + k / 2];
        out.push(nibble(if k % 2 == 0 { b >> 4 } else { b & 15 }));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_and_short_files() {
        assert!(probe(b"RIFF\0\0\0\0AMV "));
        assert!(!probe(b"RIFF\0\0\0\0AVI "));
        assert!(!probe(b""));
        assert!(Amv::parse(b"RIFF\0\0\0\0AMV ").is_err());
        assert!(Amv::parse(b"RIFF\0\0\0\0AMV LIST\0\0\0\0hdrlamvh").is_err());
    }

    #[test]
    fn audio_chunks() {
        assert!(decode_audio(&[]).is_err());
        assert!(decode_audio(&[0, 0, 89, 0, 0, 0, 0, 0]).is_err());
        // two samples from one byte, then the coded count caps it
        let c = [0, 0, 0, 0, 4, 0, 0, 0, 0x7f, 0x77];
        assert_eq!(audio_samples(&c), 4);
        assert_eq!(decode_audio(&c).unwrap().len(), 4);
        assert_eq!(audio_samples(&[0, 0, 0, 0, 100, 0, 0, 0, 1]), 2);
    }

    #[test]
    fn damaged_video_never_panics() {
        let d = Decoder::new(40, 24).unwrap();
        assert!(Decoder::new(0, 8).is_err());
        let mut x = 0x2545_f491u32;
        for n in 0..400 {
            let chunk: Vec<u8> = (0..n % 120)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    (x >> 8) as u8
                })
                .collect();
            let _ = d.decode(&chunk);
        }
    }
}
