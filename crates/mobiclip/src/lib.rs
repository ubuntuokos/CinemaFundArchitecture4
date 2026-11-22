// Ported from FFmpeg n7.1.1: libavformat/moflex.c (Copyright (c) 2015-2016 Florian Nouwt, (c) 2017
// Adib Surani, (c) 2020 Paul B Mahol), libavcodec/mobiclip.c (Copyright (c) 2022 Paul B Mahol),
// libavcodec/fastaudio.c (Copyright (c) 2020 Paul B Mahol) and libavcodec/adpcm.c (the MOFLEX
// part, Copyright (c) 2001-2003 The FFmpeg project); all LGPL-2.1-or-later (`COPYING.LGPL-2.1`);
// see docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! Pure-Rust support for Nintendo **MobiClip** movies (`.moflex`, codec backlog CB-28.2).
//!
//! - [`probe`] and [`parse`] read the MOFLEX container the way ffmpeg's `moflex` demuxer does.
//! - [`video::Decoder`] decodes the MobiClip pictures, bit for bit.
//! - [`FastAudio`], [`decode_ima`] and [`decode_pcm`] decode the three sound formats of the
//!   container.

mod tables;
pub mod video;

pub use video::{Decoder, Frame};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid MobiClip data: {0}")]
    Invalid(&'static str),
    #[error("unsupported MobiClip feature: {0}")]
    Unsupported(&'static str),
    #[error("not a MOFLEX file")]
    NotMoflex,
}

pub type Result<T> = std::result::Result<T, Error>;

/// Most pixels of one picture.
pub const MAX_PIXELS: u64 = 1 << 26;

/// The sound format of a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioCodec {
    FastAudio,
    ImaMoflex,
    PcmS16le,
}

/// One video packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoPacket {
    pub data: Vec<u8>,
    /// The decoder's first bit is set: an intra picture (a point to start decoding at).
    pub key: bool,
}

/// The first sound stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioTrack {
    pub codec: AudioCodec,
    pub channels: u32,
    pub sample_rate: u32,
    pub packets: Vec<Vec<u8>>,
}

/// What the container holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Movie {
    pub width: u32,
    pub height: u32,
    /// Frames per second: `fps_num / fps_den` (the stream's time base is its inverse).
    pub fps_num: u32,
    pub fps_den: u32,
    pub video: Vec<VideoPacket>,
    pub audio: Option<AudioTrack>,
    /// The packet chain stopped making sense before the end of the file.
    pub damaged: bool,
}

const MAGIC: u16 = 0x4C32;

/// ffmpeg's `moflex_probe`: the magic, a non-zero block size and a plausible stream list.
pub fn probe(head: &[u8]) -> bool {
    if head.len() < 14 || u16::from_be_bytes([head[0], head[1]]) != MAGIC || u16::from_be_bytes([head[12], head[13]]) == 0 {
        return false;
    }
    let mut score = 15;
    let mut p = 14;
    while p + 2 <= head.len() {
        let (ty, size) = (head[p], usize::from(head[p + 1]));
        p += 2;
        if ty == 0 {
            score += if size == 0 { 5 } else { 0 };
            break;
        }
        if matches!((ty, size), (1, 12) | (2, 6) | (3, 13) | (4, 2)) {
            score += 20;
        }
        p += size;
    }
    // ffmpeg accepts a probe score of at least 25 (AVPROBE_SCORE_RETRY)
    score >= 25
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stop {
    Eof,
    Bad,
    Unsupported,
}

struct Reader<'a> {
    d: &'a [u8],
    pos: usize,
    eof: bool,
}

impl Reader<'_> {
    fn r8(&mut self) -> u32 {
        match self.d.get(self.pos) {
            Some(&b) => {
                self.pos += 1;
                u32::from(b)
            }
            None => {
                self.eof = true;
                0
            }
        }
    }
    fn rb16(&mut self) -> u32 {
        let a = self.r8();
        (a << 8) | self.r8()
    }
    fn rb24(&mut self) -> u32 {
        let a = self.rb16();
        (a << 8) | self.r8()
    }
    fn rb64(&mut self) {
        for _ in 0..8 {
            self.r8();
        }
    }
    fn var_byte(&mut self) -> u32 {
        let mut value = 0u32;
        for i in 0..4 {
            let data = self.r8();
            if i == 3 {
                return value | data;
            }
            if data & 0x80 == 0 {
                return value | data;
            }
            value = (value | (data & 0x7F)) << 7;
        }
        value
    }
}

/// ffmpeg's `BitReader` of the demuxer: MSB first, one byte at a time.
struct Pop {
    last: u32,
    pos: u32,
}

impl Pop {
    fn pop(&mut self, r: &mut Reader<'_>) -> std::result::Result<u32, Stop> {
        if r.eof {
            return Err(Stop::Eof);
        }
        if self.pos & 7 == 0 {
            self.last = r.r8() << 24;
        } else {
            self.last <<= 1;
        }
        self.pos += 1;
        Ok(self.last >> 31)
    }
    fn int(&mut self, r: &mut Reader<'_>, n: u32) -> std::result::Result<u32, Stop> {
        let mut value = 0u32;
        for _ in 0..n {
            let b = self.pop(r)?;
            if b > i32::MAX as u32 - value - value {
                return Err(Stop::Bad);
            }
            value = 2 * value + b;
        }
        Ok(value)
    }
    fn length(&mut self, r: &mut Reader<'_>) -> std::result::Result<u32, Stop> {
        let mut n = 1;
        while self.pop(r)? == 0 {
            n += 1;
        }
        Ok(n)
    }
}

enum Kind {
    Video,
    Audio,
    Data,
}

#[derive(Default)]
struct State {
    kinds: Vec<Kind>,
    first_video: Option<usize>,
    first_audio: Option<usize>,
    width: u32,
    height: u32,
    tb_num: u32,
    tb_den: u32,
    audio: Option<(AudioCodec, u32, u32)>,
    size: usize,
}

/// `moflex_read_sync`: `Ok(true)` after a block header, `Ok(false)` when there is none.
fn read_sync(r: &mut Reader<'_>, st: &mut State) -> std::result::Result<bool, Stop> {
    if r.rb16() != u32::from(MAGIC) {
        if r.eof {
            return Err(Stop::Eof);
        }
        r.pos -= 2;
        return Ok(false);
    }
    r.pos += 2;
    r.rb64();
    st.size = r.rb16() as usize + 1;
    while !r.eof {
        let ty = r.var_byte();
        let ssize = r.var_byte();
        let mut index = usize::MAX;
        let (mut kind, mut codec, mut w, mut h, mut rate, mut ch, mut num, mut den) = (Kind::Data, 0, 0, 0, 0, 0, 0, 0);
        match ty {
            0 => {
                r.pos = r.pos.saturating_add(ssize as usize);
                return Ok(true);
            }
            2 => {
                kind = Kind::Audio;
                index = r.r8() as usize;
                codec = r.r8();
                if codec > 2 {
                    return Err(Stop::Unsupported);
                }
                rate = r.rb24() + 1;
                ch = r.r8() + 1;
            }
            1 | 3 => {
                kind = Kind::Video;
                index = r.r8() as usize;
                codec = r.r8();
                if codec != 0 {
                    return Err(Stop::Unsupported);
                }
                den = r.rb16();
                num = r.rb16();
                w = r.rb16();
                h = r.rb16();
                r.pos += if ty == 3 { 3 } else { 2 };
            }
            4 => {
                index = r.r8() as usize;
                r.pos += 1;
            }
            _ => {}
        }
        if index == st.kinds.len() {
            match kind {
                Kind::Video if st.first_video.is_none() => {
                    st.first_video = Some(index);
                    (st.width, st.height, st.tb_num, st.tb_den) = (w, h, num, den);
                }
                Kind::Audio if st.first_audio.is_none() => {
                    st.first_audio = Some(index);
                    let c = match codec {
                        0 => AudioCodec::FastAudio,
                        1 => AudioCodec::ImaMoflex,
                        _ => AudioCodec::PcmS16le,
                    };
                    st.audio = Some((c, ch, rate));
                }
                _ => {}
            }
            st.kinds.push(kind);
        }
    }
    Ok(true)
}

/// Read the stream list and collect every packet, as ffmpeg's `moflex` demuxer does.
pub fn parse(data: &[u8]) -> Result<Movie> {
    if !probe(data) {
        return Err(Error::NotMoflex);
    }
    let mut r = Reader { d: data, pos: 0, eof: false };
    let mut st = State::default();
    if let Err(e) = read_sync(&mut r, &mut st) {
        return Err(match e {
            Stop::Unsupported => Error::Unsupported("codec of a stream"),
            _ => Error::Invalid("stream list"),
        });
    }
    let Some(vi) = st.first_video else {
        return Err(Error::Invalid("no video stream"));
    };
    if st.tb_num == 0 || st.tb_den == 0 {
        return Err(Error::Invalid("time base"));
    }
    if st.width == 0 || st.height == 0 || u64::from(st.width) * u64::from(st.height) > MAX_PIXELS {
        return Err(Error::Invalid("picture size"));
    }
    r.pos = 0;
    r.eof = false;
    let mut pending: Vec<Vec<u8>> = Vec::new();
    let mut video = Vec::new();
    let mut audio_packets = Vec::new();
    let mut damaged = false;
    let mut guard = data.len().saturating_mul(2).saturating_add(64);
    'outer: while !r.eof {
        guard = match guard.checked_sub(1) {
            Some(g) => g,
            None => {
                damaged = true;
                break;
            }
        };
        let block = r.pos;
        match read_sync(&mut r, &mut st) {
            Ok(_) => {}
            Err(Stop::Eof) => break,
            Err(_) => {
                damaged = true;
                break;
            }
        }
        let flags = r.r8();
        if flags & 2 != 0 {
            r.pos += 2;
        }
        while r.pos < block + st.size && !r.eof && r.r8() != 0 {
            r.pos -= 1;
            let mut br = Pop { last: 0, pos: 0 };
            let res: std::result::Result<(), Stop> = (|| {
                let bits = br.length(&mut r)?;
                let index = br.int(&mut r, bits)? as usize;
                if index >= st.kinds.len() {
                    return Err(Stop::Bad);
                }
                let end = br.pop(&mut r)? != 0;
                if end {
                    let bits = br.length(&mut r)?;
                    let _ = br.int(&mut r, bits);
                    let _ = br.pop(&mut r);
                    let bits = br.length(&mut r)?;
                    let _ = br.int(&mut r, bits.saturating_mul(2).saturating_add(26));
                }
                let size = br.int(&mut r, 13)? as usize + 1;
                if size > st.size {
                    return Err(Stop::Bad);
                }
                if pending.len() < st.kinds.len() {
                    pending.resize(st.kinds.len(), Vec::new());
                }
                let avail = size.min(data.len().saturating_sub(r.pos));
                if avail == 0 {
                    r.eof = true;
                    return Err(Stop::Eof);
                }
                pending[index].extend_from_slice(&data[r.pos..r.pos + avail]);
                r.pos += avail;
                if avail < size {
                    r.eof = true;
                }
                if end && !pending[index].is_empty() {
                    let pkt = std::mem::take(&mut pending[index]);
                    match st.kinds[index] {
                        Kind::Video if index == vi => {
                            // the decoder reads 16-bit pairs byte-swapped, so its first bit is bit 7 of byte 1
                            // (ffmpeg's demuxer flags packets by byte 0, which is not the decoder's bit)
                            let key = pkt.get(1).is_some_and(|b| b & 0x80 != 0);
                            video.push(VideoPacket { data: pkt, key });
                        }
                        Kind::Audio if Some(index) == st.first_audio => audio_packets.push(pkt),
                        _ => {}
                    }
                }
                Ok(())
            })();
            match res {
                Ok(()) => {}
                Err(Stop::Eof) => break 'outer,
                Err(_) => {
                    damaged = true;
                    break 'outer;
                }
            }
        }
        if flags.is_multiple_of(2) {
            r.pos = block + st.size;
            r.eof = false;
        }
    }
    let audio = st.audio.map(|(codec, channels, sample_rate)| AudioTrack { codec, channels, sample_rate, packets: audio_packets });
    Ok(Movie { width: st.width, height: st.height, fps_num: st.tb_den, fps_den: st.tb_num, video, audio, damaged })
}

// ---------------------------------------------------------------------------------------------
// sound

/// The sound of FastAudio packets (`fastaudio.c`): 256 samples per 40 bytes and channel.
#[derive(Clone)]
pub struct FastAudio {
    table: [[f32; 64]; 8],
    ch: Vec<([f32; 8], f32)>,
}

impl FastAudio {
    pub fn new(channels: usize) -> Self {
        let mut t = [[0f32; 64]; 8];
        for i in 0..8 {
            t[0][i] = (i as f32 - 159.5) / 160.0;
        }
        for i in 0..11 {
            t[0][i + 8] = (i as f32 - 37.5) / 40.0;
        }
        for i in 0..27 {
            t[0][i + 19] = (i as f32 - 13.0) / 20.0;
        }
        for i in 0..11 {
            t[0][i + 46] = (i as f32 + 27.5) / 40.0;
        }
        for i in 0..7 {
            t[0][i + 57] = (i as f32 + 152.5) / 160.0;
        }
        t[1] = t[0];
        for i in 0..7 {
            t[2][i] = (i as f32 - 33.5) / 40.0;
        }
        for i in 0..25 {
            t[2][i + 7] = (i as f32 - 13.0) / 20.0;
        }
        for i in 0..32 {
            t[3][i] = -t[2][31 - i];
        }
        for i in 0..16 {
            t[4][i] = i as f32 * 0.22 / 3.0 - 0.6;
        }
        for i in 0..16 {
            t[5][i] = i as f32 * 0.20 / 3.0 - 0.3;
        }
        for i in 0..8 {
            t[6][i] = i as f32 * 0.36 / 3.0 - 0.4;
        }
        for i in 0..8 {
            t[7][i] = i as f32 * 0.34 / 3.0 - 0.2;
        }
        Self { table: t, ch: vec![([0.0; 8], 0.0); channels] }
    }

    /// Decode one packet into one vector of samples per channel.
    pub fn decode(&mut self, pkt: &[u8]) -> Vec<Vec<f32>> {
        const BITS: [u32; 8] = [6, 6, 5, 5, 4, 0, 3, 3];
        let channels = self.ch.len();
        let mut out = vec![Vec::new(); channels];
        if channels == 0 {
            return out;
        }
        let subframes = pkt.len() / (40 * channels);
        let mut p = 0;
        for _ in 0..subframes {
            for (c, o) in out.iter_mut().enumerate() {
                let mut src = [0u32; 10];
                for s in &mut src {
                    *s = u32::from_le_bytes([pkt[p], pkt[p + 1], pkt[p + 2], pkt[p + 3]]);
                    p += 4;
                }
                let mut pos = 0i32;
                let read = |bits: u32, pos: &mut i32| -> i32 {
                    *pos += bits as i32;
                    let r = src[((*pos - 1) / 32).max(0) as usize] >> ((-*pos) & 31);
                    (r & ((1u32 << bits) - 1)) as i32
                };
                let mut m = [0f32; 8];
                for i in 0..8 {
                    m[7 - i] = self.table[i][read(BITS[i], &mut pos) as usize];
                }
                let mut inds = [0i32; 4];
                let mut pads = [0usize; 4];
                for i in 0..4 {
                    inds[3 - i] = read(6, &mut pos);
                }
                for i in 0..4 {
                    pads[3 - i] = read(2, &mut pos) as usize;
                }
                let mut result = [0f32; 256];
                let mut index5 = 0i32;
                for i in 0..4 {
                    let value = f32::from_bits(((inds[i] + 1) << 20) as u32) * 2f32.powi(116);
                    let mut tmp = 0i32;
                    for j in 0..21usize {
                        let v = if j == 20 { tmp / 2 } else { read(3, &mut pos) };
                        result[i * 64 + pads[i] + j * 3] = value * (2 * v - 7) as f32;
                        if j % 10 == 9 {
                            tmp = 4 * tmp + read(2, &mut pos);
                        }
                        if j == 20 {
                            index5 = (2 * index5 + tmp % 2).min(63);
                        }
                    }
                    m[2] = self.table[5][index5 as usize];
                }
                let (f, last) = &mut self.ch[c];
                for r in &mut result {
                    let mut x = *r;
                    for j in 0..8 {
                        x -= m[j] * f[j];
                        f[j] += m[j] * x;
                    }
                    f.copy_within(1..8, 0);
                    f[7] = x;
                    *last = x + *last * 0.86;
                    *r = *last * 2.0;
                }
                o.extend_from_slice(&result);
            }
        }
        out
    }
}

const STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230,
    253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327,
    3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794,
    32767,
];
const INDEX: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

/// Decode an IMA MOFLEX packet: per channel a step index and a predictor (16 bits each), then
/// blocks of 256 samples per channel. Samples that do not fill a block are dropped.
pub fn decode_ima(pkt: &[u8], channels: usize) -> Result<Vec<Vec<i16>>> {
    if channels == 0 || pkt.len() < 4 * channels {
        return Err(Error::Invalid("IMA MOFLEX packet"));
    }
    let mut state = Vec::with_capacity(channels);
    for c in 0..channels {
        let idx = i32::from(i16::from_le_bytes([pkt[4 * c], pkt[4 * c + 1]]));
        let pred = i32::from(i16::from_le_bytes([pkt[4 * c + 2], pkt[4 * c + 3]]));
        if !(0..=88).contains(&idx) {
            return Err(Error::Invalid("IMA step index"));
        }
        state.push((idx, pred));
    }
    let blocks = (pkt.len() - 4 * channels) * 2 / channels / 256;
    let mut out = vec![Vec::with_capacity(blocks * 256); channels];
    let mut p = 4 * channels;
    for _ in 0..blocks {
        for (c, o) in out.iter_mut().enumerate() {
            let (idx, pred) = &mut state[c];
            let mut nibble = |code: u8| {
                let step = STEPS[*idx as usize];
                *idx = (*idx + INDEX[usize::from(code)]).clamp(0, 88);
                let diff = ((2 * i32::from(code & 7) + 1) * step) >> 3;
                *pred = (*pred + if code & 8 != 0 { -diff } else { diff }).clamp(-32768, 32767);
                *pred as i16
            };
            for _ in 0..128 {
                let v = pkt[p];
                p += 1;
                o.push(nibble(v & 15));
                o.push(nibble(v >> 4));
            }
        }
    }
    Ok(out)
}

/// Decode a PCM packet (signed 16 bits, little endian, interleaved).
pub fn decode_pcm(pkt: &[u8], channels: usize) -> Vec<Vec<i16>> {
    let mut out = vec![Vec::new(); channels];
    if channels > 0 {
        for frame in pkt.chunks_exact(2 * channels) {
            for (c, o) in out.iter_mut().enumerate() {
                o.push(i16::from_le_bytes([frame[2 * c], frame[2 * c + 1]]));
            }
        }
    }
    out
}
