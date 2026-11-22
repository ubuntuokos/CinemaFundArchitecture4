//! The `.bik` container (`libavformat/bink.c`): the header, the frame index and the packets of
//! a frame (one per audio track with data, then the video packet).

use crate::{Error, Result};

const AUD_STEREO: u16 = 0x2000;
const AUD_USEDCT: u16 = 0x1000;
const MAX_AUDIO_TRACKS: u32 = 256;
const MAX_WIDTH: u32 = 7680;
const MAX_HEIGHT: u32 = 4800;
const SMUSH_BLOCK_SIZE: usize = 512;

/// One audio track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioTrack {
    pub sample_rate: u32,
    pub channels: u32,
    /// The DCT variant (otherwise RDFT).
    pub dct: bool,
    pub id: u32,
}

/// Where the packets of a frame lie in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub keyframe: bool,
    /// (track number, byte range of the packet), for the tracks with at least 4 bytes of data.
    pub audio: Vec<(usize, std::ops::Range<usize>)>,
    pub video: std::ops::Range<usize>,
}

/// One frame's packets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame<'a> {
    pub keyframe: bool,
    /// (track number, packet), for the tracks with at least 4 bytes of data.
    pub audio: Vec<(usize, &'a [u8])>,
    pub video: &'a [u8],
}

/// A parsed file.
#[derive(Clone, Debug)]
pub struct Bink<'a> {
    data: &'a [u8],
    pub tag: [u8; 4],
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    /// The 32-bit extradata word (flags, alpha).
    pub flags: u32,
    pub tracks: Vec<AudioTrack>,
    smush_size: usize,
    /// (offset, size, keyframe)
    index: Vec<(u32, u32, bool)>,
}

/// Whether `data` starts like a Bink 1 file.
pub fn probe(data: &[u8]) -> bool {
    let rl = |o: usize| data.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let smush = data.starts_with(b"SMUS");
    let mut b = 0usize;
    loop {
        let ok = data.len() >= b + 36
            && &data[b..b + 3] == b"BIK"
            && matches!(data[b + 3], b'b' | b'f' | b'g' | b'h' | b'i' | b'k')
            && rl(b + 8).is_some_and(|v| v > 0)
            && rl(b + 20).is_some_and(|v| v > 0 && v <= MAX_WIDTH)
            && rl(b + 24).is_some_and(|v| v > 0 && v <= MAX_HEIGHT)
            && rl(b + 28).is_some_and(|v| v > 0)
            && rl(b + 32).is_some_and(|v| v > 0);
        if ok {
            return true;
        }
        b += SMUSH_BLOCK_SIZE;
        if !(smush && b + 32 < data.len()) {
            return false;
        }
    }
}

struct Rd<'a> {
    d: &'a [u8],
    p: usize,
}

impl Rd<'_> {
    fn u32(&mut self) -> Result<u32> {
        let b = self.d.get(self.p..self.p + 4).ok_or(Error::Invalid("header truncated"))?;
        self.p += 4;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u16(&mut self) -> Result<u16> {
        let b = self.d.get(self.p..self.p + 2).ok_or(Error::Invalid("header truncated"))?;
        self.p += 2;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
}

impl<'a> Bink<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Bink<'a>> {
        let mut rd = Rd { d: data, p: 0 };
        let mut smush_size = 0usize;
        let mut tag = rd.u32()?.to_le_bytes();
        if &tag == b"SMUS" {
            loop {
                smush_size += SMUSH_BLOCK_SIZE;
                rd.p += SMUSH_BLOCK_SIZE - 4;
                tag = rd.u32().map_err(|_| Error::Invalid("SMUSH header: BIK not found"))?.to_le_bytes();
                if &tag[..3] == b"BIK" {
                    break;
                }
            }
        }
        let file_size = rd.u32()? as u64 + 8;
        let frames = rd.u32()?;
        if frames > 1_000_000 {
            return Err(Error::Invalid("more than 1000000 frames"));
        }
        if rd.u32()? as u64 > file_size {
            return Err(Error::Invalid("largest frame size greater than file size"));
        }
        rd.p += 4;
        let width = rd.u32()?;
        let height = rd.u32()?;
        let fps_num = rd.u32()?;
        let fps_den = rd.u32()?;
        if fps_num == 0 || fps_den == 0 {
            return Err(Error::Invalid("invalid fps"));
        }
        if &tag[..3] == b"KB2" {
            return Err(Error::Unsupported("Bink 2"));
        }
        if &tag[..3] != b"BIK" {
            return Err(Error::Invalid("not a Bink file"));
        }
        if width == 0 || height == 0 || width > MAX_WIDTH || height > MAX_HEIGHT {
            return Err(Error::Invalid("frame size"));
        }
        let flags = rd.u32()?;
        let n_audio = rd.u32()?;
        if n_audio > MAX_AUDIO_TRACKS {
            return Err(Error::Invalid("more than 256 audio tracks"));
        }
        if tag[3] == b'k' {
            rd.p += 4;
        }
        let mut tracks = Vec::new();
        if n_audio > 0 {
            rd.p += 4 * n_audio as usize;
            for _ in 0..n_audio {
                let sample_rate = rd.u16()? as u32;
                let f = rd.u16()?;
                tracks.push(AudioTrack { sample_rate, channels: if f & AUD_STEREO != 0 { 2 } else { 1 }, dct: f & AUD_USEDCT != 0, id: 0 });
            }
            for t in tracks.iter_mut() {
                t.id = rd.u32()?;
            }
        }
        let mut index = Vec::with_capacity(frames as usize);
        let mut next_pos = if frames > 0 { rd.u32()? } else { 0 };
        let mut next_key = true;
        for i in 0..frames {
            let (mut pos, key) = (next_pos, next_key);
            if i == frames - 1 {
                next_pos = file_size as u32;
                next_key = false;
            } else {
                next_pos = rd.u32()?;
                next_key = next_pos & 1 != 0;
            }
            pos &= !1;
            next_pos &= !1;
            if next_pos <= pos {
                return Err(Error::Invalid("invalid frame index table"));
            }
            index.push((pos, next_pos - pos, key));
        }
        Ok(Bink { data, tag, width, height, fps_num, fps_den, flags, tracks, smush_size, index })
    }

    pub fn frame_count(&self) -> usize {
        self.index.len()
    }

    /// Where the packets of frame `i` lie in the file.
    pub fn layout(&self, i: usize) -> Result<Layout> {
        let &(pos, size, keyframe) = self.index.get(i).ok_or(Error::Invalid("no such frame"))?;
        let start = pos as usize + self.smush_size;
        let end = start + size as usize;
        if end > self.data.len() {
            return Err(Error::Invalid("frame beyond the end of the file"));
        }
        let mut at = start;
        let mut audio = Vec::new();
        for t in 0..self.tracks.len() {
            let head = self.data.get(at..at + 4).filter(|_| at + 4 <= end).ok_or(Error::Invalid("audio size"))?;
            let n = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
            if n > end - at - 4 {
                return Err(Error::Invalid("audio size larger than the packet"));
            }
            if n >= 4 {
                audio.push((t, at + 4..at + 4 + n));
            }
            at += 4 + n;
        }
        Ok(Layout { keyframe, audio, video: at..end })
    }

    /// The packets of frame `i`.
    pub fn frame(&self, i: usize) -> Result<Frame<'a>> {
        let l = self.layout(i)?;
        Ok(Frame { keyframe: l.keyframe, audio: l.audio.into_iter().map(|(t, r)| (t, &self.data[r])).collect(), video: &self.data[l.video] })
    }
}
