// Ported from FFmpeg n7.1.1 libavcodec/takdec.c, tak.c, tak.h, takdsp.c, tak_parser.c and
// libavformat/takdec.c (Copyright (c) 2012, 2015 Paul B Mahol), LGPL-2.1-or-later; see
// docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! Pure-Rust decoder for **TAK** (Tom's lossless Audio Kompressor): the mono / stereo and
//! multichannel codecs, 8-, 16- and 24-bit audio, 1 to 6 channels, LPC subframes, every channel
//! decorrelation mode and the windowed residual coding. The output is the same as ffmpeg's `tak`
//! decoder, sample for sample.
//!
//! - [`Decoder`] decodes one frame at a time.
//! - [`decode_file`] decodes a whole `.tak` file to one plane per channel.

/// Errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("not a TAK stream")]
    NotTak,
    #[error("invalid TAK stream: {0}")]
    Invalid(&'static str),
    #[error("unsupported TAK stream: {0}")]
    Unsupported(&'static str),
    #[error("no audio in the TAK stream")]
    Empty,
}

pub type Result<T> = std::result::Result<T, Error>;

/// The most channels the decoder handles (as ffmpeg's).
pub const MAX_CHANNELS: u32 = 6;
/// Cap on decoded values (samples × channels) for [`decode_file`]: 2³⁰, 4 GiB of `i32`.
pub const MAX_FILE_VALUES: u64 = 1 << 30;

const CODEC_MONO_STEREO: u32 = 2;
const CODEC_MULTICHANNEL: u32 = 4;
const FLAG_IS_LAST: u32 = 1;
const FLAG_HAS_INFO: u32 = 2;
const FLAG_HAS_METADATA: u32 = 4;
const SYNC: u32 = 0xA0FF;
const MIN_FRAME_HEADER_BYTES: usize = 8;
const CRC_INIT: u32 = 0xCE_04B7;

const MAX_SUBFRAMES: usize = 8;
const MAX_PREDICTORS: usize = 256;
const RESIDUES: usize = 544;

const CHANNEL_LAYOUTS: [u64; 19] =
    [0, 0x1, 0x2, 0x4, 0x8, 0x10, 0x20, 0x40, 0x80, 0x100, 0x200, 0x400, 0x800, 0x1000, 0x2000, 0x4000, 0x8000, 0x1_0000, 0x2_0000];
const FRAME_DURATION_QUANTS: [i32; 10] = [3, 4, 6, 8, 4096, 8192, 16384, 512, 1024, 2048];
const MC_DMODES: [u8; 4] = [1, 3, 4, 6];
const PREDICTOR_SIZES: [usize; 16] = [4, 8, 12, 16, 24, 32, 48, 64, 80, 96, 128, 160, 192, 224, 256, 0];

/// One residual coding parameter set (`init`, `escape`, `scale`, `aescape`, `bias`).
struct CParam {
    init: u32,
    escape: u32,
    scale: u32,
    aescape: u32,
    bias: u32,
}

const fn cp(init: u32, escape: u32, scale: u32, aescape: u32, bias: u32) -> CParam {
    CParam { init, escape, scale, aescape, bias }
}

const XCODES: [CParam; 50] = [
    cp(0x01, 0x0000001, 0x0000001, 0x0000003, 0x0000008),
    cp(0x02, 0x0000003, 0x0000001, 0x0000007, 0x0000006),
    cp(0x03, 0x0000005, 0x0000002, 0x000000E, 0x000000D),
    cp(0x03, 0x0000003, 0x0000003, 0x000000D, 0x0000018),
    cp(0x04, 0x000000B, 0x0000004, 0x000001C, 0x0000019),
    cp(0x04, 0x0000006, 0x0000006, 0x000001A, 0x0000030),
    cp(0x05, 0x0000016, 0x0000008, 0x0000038, 0x0000032),
    cp(0x05, 0x000000C, 0x000000C, 0x0000034, 0x0000060),
    cp(0x06, 0x000002C, 0x0000010, 0x0000070, 0x0000064),
    cp(0x06, 0x0000018, 0x0000018, 0x0000068, 0x00000C0),
    cp(0x07, 0x0000058, 0x0000020, 0x00000E0, 0x00000C8),
    cp(0x07, 0x0000030, 0x0000030, 0x00000D0, 0x0000180),
    cp(0x08, 0x00000B0, 0x0000040, 0x00001C0, 0x0000190),
    cp(0x08, 0x0000060, 0x0000060, 0x00001A0, 0x0000300),
    cp(0x09, 0x0000160, 0x0000080, 0x0000380, 0x0000320),
    cp(0x09, 0x00000C0, 0x00000C0, 0x0000340, 0x0000600),
    cp(0x0A, 0x00002C0, 0x0000100, 0x0000700, 0x0000640),
    cp(0x0A, 0x0000180, 0x0000180, 0x0000680, 0x0000C00),
    cp(0x0B, 0x0000580, 0x0000200, 0x0000E00, 0x0000C80),
    cp(0x0B, 0x0000300, 0x0000300, 0x0000D00, 0x0001800),
    cp(0x0C, 0x0000B00, 0x0000400, 0x0001C00, 0x0001900),
    cp(0x0C, 0x0000600, 0x0000600, 0x0001A00, 0x0003000),
    cp(0x0D, 0x0001600, 0x0000800, 0x0003800, 0x0003200),
    cp(0x0D, 0x0000C00, 0x0000C00, 0x0003400, 0x0006000),
    cp(0x0E, 0x0002C00, 0x0001000, 0x0007000, 0x0006400),
    cp(0x0E, 0x0001800, 0x0001800, 0x0006800, 0x000C000),
    cp(0x0F, 0x0005800, 0x0002000, 0x000E000, 0x000C800),
    cp(0x0F, 0x0003000, 0x0003000, 0x000D000, 0x0018000),
    cp(0x10, 0x000B000, 0x0004000, 0x001C000, 0x0019000),
    cp(0x10, 0x0006000, 0x0006000, 0x001A000, 0x0030000),
    cp(0x11, 0x0016000, 0x0008000, 0x0038000, 0x0032000),
    cp(0x11, 0x000C000, 0x000C000, 0x0034000, 0x0060000),
    cp(0x12, 0x002C000, 0x0010000, 0x0070000, 0x0064000),
    cp(0x12, 0x0018000, 0x0018000, 0x0068000, 0x00C0000),
    cp(0x13, 0x0058000, 0x0020000, 0x00E0000, 0x00C8000),
    cp(0x13, 0x0030000, 0x0030000, 0x00D0000, 0x0180000),
    cp(0x14, 0x00B0000, 0x0040000, 0x01C0000, 0x0190000),
    cp(0x14, 0x0060000, 0x0060000, 0x01A0000, 0x0300000),
    cp(0x15, 0x0160000, 0x0080000, 0x0380000, 0x0320000),
    cp(0x15, 0x00C0000, 0x00C0000, 0x0340000, 0x0600000),
    cp(0x16, 0x02C0000, 0x0100000, 0x0700000, 0x0640000),
    cp(0x16, 0x0180000, 0x0180000, 0x0680000, 0x0C00000),
    cp(0x17, 0x0580000, 0x0200000, 0x0E00000, 0x0C80000),
    cp(0x17, 0x0300000, 0x0300000, 0x0D00000, 0x1800000),
    cp(0x18, 0x0B00000, 0x0400000, 0x1C00000, 0x1900000),
    cp(0x18, 0x0600000, 0x0600000, 0x1A00000, 0x3000000),
    cp(0x19, 0x1600000, 0x0800000, 0x3800000, 0x3200000),
    cp(0x19, 0x0C00000, 0x0C00000, 0x3400000, 0x6000000),
    cp(0x1A, 0x2C00000, 0x1000000, 0x7000000, 0x6400000),
    cp(0x1A, 0x1800000, 0x1800000, 0x6800000, 0xC000000),
];

const CRC_TABLE: [u32; 256] = crc_table();

const fn crc_table() -> [u32; 256] {
    // libavutil's big-endian table for CRC-24 (polynomial 0x864CFB), byte-swapped as av_crc_init stores it.
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = (i as u32) << 24;
        let mut j = 0;
        while j < 8 {
            c = (c << 1) ^ ((0x0086_4CFB << 8) & (((c as i32) >> 31) as u32));
            j += 1;
        }
        t[i] = c.swap_bytes();
        i += 1;
    }
    t
}

/// TAK's CRC-24 of `data` (as ffmpeg computes it: compare with the three bytes that follow read
/// big-endian).
pub fn crc24(data: &[u8]) -> u32 {
    data.iter().fold(CRC_INIT, |crc, &b| CRC_TABLE[usize::from((crc as u8) ^ b)] ^ (crc >> 8))
}

/// `data` ends in a CRC-24 of the bytes before it.
fn check_crc(data: &[u8]) -> bool {
    let Some(n) = data.len().checked_sub(3).filter(|&n| n > 0) else { return false };
    let (body, tail) = data.split_at(n);
    let stored = tail.iter().fold(0u32, |a, &b| (a << 8) | u32::from(b));
    crc24(body) == stored
}

/// Little-endian bit reader; reads past the end return zeros.
struct Bits<'a> {
    data: &'a [u8],
    pos: u64,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn left(&self) -> i64 {
        (self.data.len() as i64).saturating_mul(8).saturating_sub(self.pos as i64)
    }

    /// `n` ≤ 32 bits, the first bit read is the lowest.
    fn bits(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let byte = (self.pos >> 3) as usize;
        let mut w = 0u64;
        for i in 0..5 {
            if let Some(&b) = byte.checked_add(i).and_then(|k| self.data.get(k)) {
                w |= u64::from(b) << (8 * i);
            }
        }
        let v = (w >> (self.pos & 7)) & ((1u64 << n.min(32)) - 1);
        self.pos = self.pos.saturating_add(u64::from(n));
        v as u32
    }

    fn bit(&mut self) -> u32 {
        self.bits(1)
    }

    /// A signed `n`-bit value, 1 ≤ `n` ≤ 32.
    fn sbits(&mut self, n: u32) -> i32 {
        let n = n.clamp(1, 32);
        let v = self.bits(n);
        ((v << (32 - n)) as i32) >> (32 - n)
    }

    fn bits64(&mut self, n: u32) -> u64 {
        let lo = u64::from(self.bits(32));
        lo | (u64::from(self.bits(n.saturating_sub(32))) << 32)
    }

    /// Zero bits before a one, at most `len` bits read.
    fn unary(&mut self, len: u32) -> u32 {
        let mut i = 0;
        while i < len && self.bit() != 1 {
            i += 1;
        }
        i
    }

    fn esc4(&mut self) -> u32 {
        if self.bit() == 1 { self.bits(4) + 1 } else { 0 }
    }

    fn skip(&mut self, n: u32) {
        self.pos = self.pos.saturating_add(u64::from(n));
    }

    fn align(&mut self) {
        self.pos = self.pos.saturating_add(7) & !7;
    }
}

/// Stream parameters from a STREAMINFO block or a frame header.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamInfo {
    /// 2: mono / stereo, 4: multichannel.
    pub codec: u32,
    pub data_type: u32,
    pub sample_rate: u32,
    pub channels: u32,
    pub bps: u32,
    /// Samples in every frame but the last.
    pub frame_samples: u32,
    /// The channel mask (ffmpeg's `AV_CH_*` bits), 0 when unspecified.
    pub ch_layout: u64,
    /// Total samples per channel, as declared.
    pub samples: u64,
}

fn nb_samples(sample_rate: u32, frame_type: u32) -> Result<u32> {
    let sr = sample_rate as i32;
    let quant = |t: usize| FRAME_DURATION_QUANTS.get(t).copied().unwrap_or(0);
    let (nb, max) = if frame_type <= 3 {
        (sr.wrapping_mul(quant(frame_type as usize)) >> 5, 16384)
    } else if frame_type < 10 {
        (quant(frame_type as usize), sr.wrapping_mul(8) >> 5)
    } else {
        return Err(Error::Invalid("frame size type"));
    };
    if nb <= 0 || nb > max {
        return Err(Error::Invalid("frame size"));
    }
    Ok(nb as u32)
}

/// Reads the fields into `s` as they come (as ffmpeg does), then the frame size.
fn parse_streaminfo(s: &mut StreamInfo, gb: &mut Bits<'_>) -> Result<()> {
    s.codec = gb.bits(6);
    gb.skip(4);
    let frame_type = gb.bits(4);
    s.samples = gb.bits64(35);
    s.data_type = gb.bits(3);
    s.sample_rate = gb.bits(18) + 6000;
    s.bps = gb.bits(5) + 8;
    s.channels = gb.bits(4) + 1;
    let mut mask = 0u64;
    if gb.bit() == 1 {
        gb.skip(5);
        if gb.bit() == 1 {
            for _ in 0..s.channels {
                let v = gb.bits(6) as usize;
                mask |= CHANNEL_LAYOUTS.get(v).copied().unwrap_or(0);
            }
        }
    }
    s.ch_layout = mask;
    s.frame_samples = nb_samples(s.sample_rate, frame_type)?;
    Ok(())
}

/// Parses a STREAMINFO metadata block (without its CRC).
pub fn parse_streaminfo_block(data: &[u8]) -> Result<StreamInfo> {
    let mut s = StreamInfo::default();
    parse_streaminfo(&mut s, &mut Bits::new(data))?;
    Ok(s)
}

#[derive(Clone, Copy, Debug)]
struct FrameHeader {
    flags: u32,
    last_frame_samples: u32,
}

/// Validates and decodes a frame header; a STREAMINFO in it updates `ti`.
fn decode_frame_header(gb: &mut Bits<'_>, ti: &mut StreamInfo) -> Result<FrameHeader> {
    if gb.bits(16) != SYNC {
        return Err(Error::Invalid("missing sync id"));
    }
    let flags = gb.bits(3);
    gb.skip(21); // frame number
    let mut last_frame_samples = 0;
    if flags & FLAG_IS_LAST != 0 {
        last_frame_samples = gb.bits(14) + 1;
        gb.skip(2);
    }
    if flags & FLAG_HAS_INFO != 0 {
        parse_streaminfo(ti, gb)?;
        if gb.bits(6) != 0 {
            gb.skip(25);
        }
        gb.align();
    }
    if flags & FLAG_HAS_METADATA != 0 {
        return Err(Error::Unsupported("frame metadata"));
    }
    if gb.left() < 24 {
        return Err(Error::Invalid("truncated frame header"));
    }
    gb.skip(24);
    Ok(FrameHeader { flags, last_frame_samples })
}

/// The frame header at the start of `data` is valid and its CRC matches (as ffmpeg's parser
/// checks when it looks for frames).
fn valid_header(data: &[u8], ti: &mut StreamInfo) -> bool {
    let mut gb = Bits::new(data);
    if decode_frame_header(&mut gb, ti).is_err() {
        return false;
    }
    data.get(..(gb.pos / 8) as usize).is_some_and(check_crc)
}

#[derive(Clone, Copy, Debug, Default)]
struct McdParam {
    present: bool,
    index: u8,
    chan1: usize,
    chan2: usize,
}

/// A frame decoder. It keeps the stream parameters of the last frame that carried them.
pub struct Decoder {
    ti: StreamInfo,
    uval: i32,
    subframe_scale: i32,
    nb_samples: usize,
    bps: u32,
    decoded: Vec<Vec<i32>>,
    lpc_mode: [u8; 16],
    sample_shift: [u32; 16],
    predictors: [i16; MAX_PREDICTORS],
    filter: [i16; MAX_PREDICTORS],
    residues: [i16; RESIDUES],
    coding_mode: [i8; 128],
    dmode: u8,
    mcd: [McdParam; 16],
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new(None)
    }
}

/// `av_clip_intp2(a, 13)`.
fn clip13(a: i32) -> i32 {
    a.clamp(-(1 << 13), (1 << 13) - 1)
}

/// The prediction filter's dot product, modulo 2³² as ffmpeg's.
fn dot(res: &[i16], filter: &[i16]) -> u32 {
    res.iter().zip(filter).fold(0u32, |a, (&r, &f)| a.wrapping_add((i32::from(r) * i32::from(f)) as u32))
}

fn decode_lpc(c: &mut [i32], mode: u8, length: usize) {
    let Some(c) = c.get_mut(..length) else { return };
    if length < 2 {
        return;
    }
    let g = |c: &[i32], i: usize| c.get(i).copied().unwrap_or(0) as u32;
    let set = |c: &mut [i32], i: usize, v: u32| {
        if let Some(x) = c.get_mut(i) {
            *x = v as i32;
        }
    };
    match mode {
        1 => {
            let mut a1 = g(c, 0);
            let mut k = 1;
            for _ in 0..(length - 1) >> 1 {
                let v0 = g(c, k).wrapping_add(a1);
                set(c, k, v0);
                let v1 = g(c, k + 1).wrapping_add(v0);
                set(c, k + 1, v1);
                a1 = v1;
                k += 2;
            }
            if (length - 1) & 1 == 1 {
                let v = g(c, k).wrapping_add(a1);
                set(c, k, v);
            }
        }
        2 => {
            let mut a1 = g(c, 1);
            let mut a2 = a1.wrapping_add(g(c, 0));
            set(c, 1, a2);
            if length > 2 {
                let mut k = 2;
                for _ in 0..(length - 2) >> 1 {
                    let a3 = g(c, k).wrapping_add(a1);
                    let a4 = a3.wrapping_add(a2);
                    set(c, k, a4);
                    a1 = g(c, k + 1).wrapping_add(a3);
                    a2 = a1.wrapping_add(a4);
                    set(c, k + 1, a2);
                    k += 2;
                }
                if length & 1 == 1 {
                    let v = g(c, k).wrapping_add(a1.wrapping_add(a2));
                    set(c, k, v);
                }
            }
        }
        3 => {
            let a1 = g(c, 1);
            let a2 = a1.wrapping_add(g(c, 0));
            set(c, 1, a2);
            if length > 2 {
                let mut a3 = g(c, 2);
                let mut a4 = a3.wrapping_add(a1);
                let mut a5 = a4.wrapping_add(a2);
                set(c, 2, a5);
                for k in 3..length {
                    a3 = a3.wrapping_add(g(c, k));
                    a4 = a4.wrapping_add(a3);
                    a5 = a5.wrapping_add(a4);
                    set(c, k, a5);
                }
            }
        }
        _ => {}
    }
}

fn decode_segment(gb: &mut Bits<'_>, mode: i8, out: &mut [i32]) -> Result<()> {
    if mode == 0 {
        out.fill(0);
        return Ok(());
    }
    let code = usize::try_from(mode).ok().and_then(|m| XCODES.get(m - 1)).ok_or(Error::Invalid("residual coding mode"))?;
    for v in out {
        let mut x = gb.bits(code.init);
        if x >= code.escape && gb.bit() == 1 {
            x |= 1 << code.init;
            if x >= code.aescape {
                let mut scale = gb.unary(9);
                if scale == 9 {
                    let mut scale_bits = gb.bits(3);
                    if scale_bits > 0 {
                        if scale_bits == 7 {
                            scale_bits += gb.bits(5);
                            if scale_bits > 29 {
                                return Err(Error::Invalid("residual scale"));
                            }
                        }
                        scale = gb.bits(scale_bits).wrapping_add(1);
                        x = x.wrapping_add(code.scale.wrapping_mul(scale));
                    }
                    x = x.wrapping_add(code.bias);
                } else {
                    x = x.wrapping_add(code.scale.wrapping_mul(scale).wrapping_sub(code.escape));
                }
            } else {
                x = x.wrapping_sub(code.escape);
            }
        }
        *v = ((x >> 1) ^ (x & 1).wrapping_neg()) as i32;
    }
    Ok(())
}

/// Two distinct channels, mutably.
fn two(decoded: &mut [Vec<i32>], a: usize, b: usize) -> Result<(&mut Vec<i32>, &mut Vec<i32>)> {
    if a == b || a >= decoded.len() || b >= decoded.len() {
        return Err(Error::Invalid("decorrelation channels"));
    }
    if a < b {
        let (l, r) = decoded.split_at_mut(b);
        Ok((&mut l[a], &mut r[0]))
    } else {
        let (l, r) = decoded.split_at_mut(a);
        Ok((&mut r[0], &mut l[b]))
    }
}

impl Decoder {
    /// A decoder; `info` (from the file's STREAMINFO block) is used until a frame carries its own.
    pub fn new(info: Option<StreamInfo>) -> Self {
        Self {
            ti: info.unwrap_or_default(),
            uval: 0,
            subframe_scale: 0,
            nb_samples: 0,
            bps: 0,
            decoded: Vec::new(),
            lpc_mode: [0; 16],
            sample_shift: [0; 16],
            predictors: [0; MAX_PREDICTORS],
            filter: [0; MAX_PREDICTORS],
            residues: [0; RESIDUES],
            coding_mode: [0; 128],
            dmode: 0,
            mcd: [McdParam::default(); 16],
        }
    }

    /// The stream parameters in effect (after the last decoded frame header).
    pub fn info(&self) -> &StreamInfo {
        &self.ti
    }

    fn set_sample_rate_params(&mut self) {
        let sr = i64::from(self.ti.sample_rate);
        let shift = if sr < 11025 {
            3
        } else if sr < 22050 {
            2
        } else if sr < 44100 {
            1
        } else {
            0
        };
        let a = ((sr + 511) >> 9).wrapping_add(3) & !3;
        self.uval = (a << shift) as i32;
        self.subframe_scale = (a << 1) as i32;
    }

    fn decode_residues(&mut self, gb: &mut Bits<'_>, out: &mut [i32]) -> Result<()> {
        let length = out.len();
        if length > self.nb_samples {
            return Err(Error::Invalid("residual length"));
        }
        if gb.bit() == 1 {
            let uval = usize::try_from(self.uval).ok().filter(|&u| u > 0).ok_or(Error::Invalid("sample rate"))?;
            let mut wlength = length / uval;
            let mut rval = length - wlength * uval;
            if rval < uval / 2 {
                rval += uval;
            } else {
                wlength += 1;
            }
            if wlength <= 1 || wlength > 128 {
                return Err(Error::Invalid("residual windows"));
            }
            let mut mode = gb.bits(6) as i32;
            self.coding_mode[0] = mode as i8;
            for i in 1..wlength {
                match gb.unary(6) {
                    6 => mode = gb.bits(6) as i32,
                    c @ 3..=5 => {
                        let sign = gb.bit() as i32;
                        mode += (-sign ^ (c as i32 - 1)) + sign;
                    }
                    2 => mode += 1,
                    1 => mode -= 1,
                    _ => {}
                }
                self.coding_mode[i] = mode as i8;
            }
            let mut i = 0;
            let mut off = 0usize;
            while i < wlength {
                let mut len = 0;
                let m = self.coding_mode[i];
                loop {
                    len += if i >= wlength - 1 { rval } else { uval };
                    i += 1;
                    if i == wlength || self.coding_mode[i] != m {
                        break;
                    }
                }
                let seg = out.get_mut(off..off + len).ok_or(Error::Invalid("residual windows"))?;
                decode_segment(gb, m, seg)?;
                off += len;
            }
        } else {
            let mode = gb.bits(6) as i8;
            decode_segment(gb, mode, out)?;
        }
        Ok(())
    }

    fn decode_subframe(&mut self, gb: &mut Bits<'_>, d: &mut [i32], pos: usize, subframe_size: usize, prev: usize) -> Result<()> {
        if gb.bit() == 0 {
            let out = d.get_mut(pos..pos + subframe_size).ok_or(Error::Invalid("subframe"))?;
            return self.decode_residues(gb, out);
        }
        let filter_order = PREDICTOR_SIZES[gb.bits(4) as usize & 15];
        let (start, size);
        if prev > 0 && gb.bit() == 1 {
            if filter_order > prev {
                return Err(Error::Invalid("filter order"));
            }
            start = pos.checked_sub(filter_order).ok_or(Error::Invalid("filter order"))?;
            size = subframe_size + filter_order;
        } else {
            if filter_order > subframe_size {
                return Err(Error::Invalid("filter order"));
            }
            let lpc_mode = gb.bits(2);
            if lpc_mode > 2 {
                return Err(Error::Invalid("subframe LPC mode"));
            }
            let head = d.get_mut(pos..pos + filter_order).ok_or(Error::Invalid("subframe"))?;
            self.decode_residues(gb, head)?;
            if lpc_mode != 0 {
                decode_lpc(head, lpc_mode as u8, filter_order);
            }
            start = pos;
            size = subframe_size;
        }
        let dshift = gb.esc4();
        let psize = gb.bit() + 6;
        let mut filter_quant = 10u32;
        if gb.bit() == 1 {
            filter_quant = filter_quant.wrapping_sub(gb.bits(3) + 1);
            if !(3..=10).contains(&filter_quant) {
                return Err(Error::Invalid("filter quantisation"));
            }
        }
        let pscale = 1i32 << (10 - psize);
        self.predictors[0] = gb.sbits(10) as i16;
        self.predictors[1] = gb.sbits(10) as i16;
        self.predictors[2] = (gb.sbits(psize) * pscale) as i16;
        self.predictors[3] = (gb.sbits(psize) * pscale) as i16;
        if filter_order > 4 {
            let tmp = psize - gb.bit();
            let mut x = 0;
            for i in 4..filter_order {
                if i & 3 == 0 {
                    x = tmp - gb.bits(2);
                }
                self.predictors[i] = (gb.sbits(x) * pscale) as i16;
            }
        }

        let mut tfilter = [0u32; MAX_PREDICTORS];
        tfilter[0] = (i32::from(self.predictors[0]) * 64) as u32;
        for i in 1..filter_order {
            let p = i32::from(self.predictors[i]) as u32;
            let mut p2 = i - 1;
            for p1 in 0..i.div_ceil(2) {
                let a = tfilter[p1];
                let b = tfilter[p2];
                let x = a.wrapping_add((p.wrapping_mul(b).wrapping_add(256) as i32 >> 9) as u32);
                tfilter[p2] = b.wrapping_add((p.wrapping_mul(a).wrapping_add(256) as i32 >> 9) as u32);
                tfilter[p1] = x;
                p2 = p2.wrapping_sub(1);
            }
            tfilter[i] = (i32::from(self.predictors[i]) * 64) as u32;
        }
        let qs = 15 - filter_quant;
        let x = 1i32 << (32 - qs);
        let y = 1i32 << (qs - 1);
        for i in 0..filter_order / 2 {
            let j = filter_order - 1 - i;
            self.filter[j] = x.wrapping_sub((tfilter[i] as i32).wrapping_add(y) >> qs) as i16;
            self.filter[i] = x.wrapping_sub((tfilter[j] as i32).wrapping_add(y) >> qs) as i16;
        }

        let rest = d.get_mut(start + filter_order..start + size).ok_or(Error::Invalid("subframe"))?;
        self.decode_residues(gb, rest)?;

        let seg = d.get_mut(start..start + size).ok_or(Error::Invalid("subframe"))?;
        for i in 0..filter_order {
            self.residues[i] = (seg[i] >> dshift) as i16;
        }
        let y = RESIDUES - filter_order;
        let mut x = size - filter_order;
        let mut k = filter_order;
        let filter = &self.filter[..filter_order];
        while x > 0 {
            let tmp = y.min(x);
            for i in 0..tmp {
                let v = (1u32 << (filter_quant - 1)).wrapping_add(dot(&self.residues[i..i + filter_order], filter)) as i32;
                let v = ((clip13(v >> filter_quant) << dshift) as u32).wrapping_sub(seg[k] as u32) as i32;
                seg[k] = v;
                k += 1;
                self.residues[filter_order + i] = (v >> dshift) as i16;
            }
            x -= tmp;
            if x > 0 {
                self.residues.copy_within(y..y + filter_order, 0);
            }
        }
        Ok(())
    }

    fn decode_channel(&mut self, gb: &mut Bits<'_>, chan: usize) -> Result<()> {
        let mut d = std::mem::take(self.decoded.get_mut(chan).ok_or(Error::Invalid("channel"))?);
        let r = self.decode_channel_into(gb, chan, &mut d);
        if let Some(slot) = self.decoded.get_mut(chan) {
            *slot = d;
        }
        r
    }

    fn decode_channel_into(&mut self, gb: &mut Bits<'_>, chan: usize, d: &mut [i32]) -> Result<()> {
        let shift = gb.esc4();
        self.sample_shift[chan & 15] = shift;
        if shift >= self.bps {
            return Err(Error::Invalid("sample shift"));
        }
        let first = gb.sbits(self.bps - shift);
        *d.first_mut().ok_or(Error::Invalid("frame size"))? = first;
        self.lpc_mode[chan & 15] = gb.bits(2) as u8;
        let nb_subframes = gb.bits(3) as usize + 1;
        let mut subframe_len = [0i32; MAX_SUBFRAMES];
        let mut left = self.nb_samples as i32 - 1;
        let mut i = 0;
        if nb_subframes > 1 {
            if gb.left() < ((nb_subframes - 1) * 6) as i64 {
                return Err(Error::Invalid("subframes"));
            }
            let mut prev = 0i32;
            while i < nb_subframes - 1 {
                let v = gb.bits(6) as i32;
                let len = i32::from(((v - prev).wrapping_mul(self.subframe_scale)) as i16);
                if len <= 0 {
                    return Err(Error::Invalid("subframe length"));
                }
                subframe_len[i] = len;
                left -= len;
                prev = v;
                i += 1;
            }
            if left <= 0 {
                return Err(Error::Invalid("subframe lengths"));
            }
        }
        subframe_len[i] = i32::from(left as i16);
        let mut pos = 1usize;
        let mut prev = 0usize;
        for &len in &subframe_len[..nb_subframes] {
            let len = usize::try_from(len).map_err(|_| Error::Invalid("subframe length"))?;
            self.decode_subframe(gb, d, pos, len, prev)?;
            pos += len;
            prev = len;
        }
        Ok(())
    }

    fn decorrelate(&mut self, gb: &mut Bits<'_>, c1: usize, c2: usize, length: usize) -> Result<()> {
        let dmode = self.dmode;
        let off = usize::from(dmode > 5);
        let (ch1, ch2) = two(&mut self.decoded, c1, c2)?;
        let mut p1: &mut [i32] = ch1.get_mut(off..).ok_or(Error::Invalid("frame size"))?;
        let mut p2: &mut [i32] = ch2.get_mut(off..).ok_or(Error::Invalid("frame size"))?;
        let length = length + usize::from(dmode < 6);
        if p1.len() < length || p2.len() < length {
            return Err(Error::Invalid("frame size"));
        }
        let (mut bp1, mut bp2) = (p1[0], p2[0]);
        match dmode {
            1 => {
                for i in 0..length {
                    p2[i] = p1[i].wrapping_add(p2[i]);
                }
            }
            2 => {
                for i in 0..length {
                    p1[i] = p2[i].wrapping_sub(p1[i]);
                }
            }
            3 => {
                for i in 0..length {
                    let b = p2[i];
                    let a = (p1[i] as u32).wrapping_sub((b >> 1) as u32);
                    p1[i] = a as i32;
                    p2[i] = a.wrapping_add(b as u32) as i32;
                }
            }
            4 | 5 => {
                if dmode == 4 {
                    std::mem::swap(&mut p1, &mut p2);
                    std::mem::swap(&mut bp1, &mut bp2);
                }
                let dshift = gb.esc4();
                let dfactor = gb.sbits(10);
                for i in 0..length {
                    let a = p1[i] as u32;
                    let b = p2[i];
                    let t = ((dfactor as u32).wrapping_mul((b >> dshift) as u32).wrapping_add(128) as i32) >> 8;
                    p1[i] = ((t as u32) << dshift).wrapping_sub(a) as i32;
                }
            }
            6 | 7 => {
                if dmode == 6 {
                    std::mem::swap(&mut p1, &mut p2);
                }
                if length < 256 {
                    return Err(Error::Invalid("decorrelation length"));
                }
                let dshift = gb.esc4();
                let filter_order = 8usize << gb.bit();
                let dval1 = gb.bit() == 1;
                let dval2 = gb.bit() == 1;
                let mut code_size = 0;
                for i in 0..filter_order {
                    if i & 3 == 0 {
                        code_size = 14 - gb.bits(3);
                    }
                    self.filter[i] = gb.sbits(code_size) as i16;
                }
                let order_half = filter_order / 2;
                let mut length2 = length - (filter_order - 1);
                if dval1 {
                    for i in 0..order_half {
                        p1[i] = p1[i].wrapping_add(p2[i]);
                    }
                }
                if dval2 {
                    for i in length2 + order_half..length {
                        p1[i] = p1[i].wrapping_add(p2[i]);
                    }
                }
                for i in 0..filter_order {
                    self.residues[i] = (p2[i] >> dshift) as i16;
                }
                let mut q2 = filter_order;
                let mut q1 = order_half;
                let x = RESIDUES - filter_order;
                let filter = &self.filter[..filter_order];
                while length2 > 0 {
                    let tmp = length2.min(x);
                    for i in 0..tmp - usize::from(tmp == length2) {
                        self.residues[filter_order + i] = (p2[q2] >> dshift) as i16;
                        q2 += 1;
                    }
                    for i in 0..tmp {
                        let v = 512u32.wrapping_add(dot(&self.residues[i..i + filter_order], filter)) as i32;
                        let v = ((clip13(v >> 10) as u32).wrapping_mul(1 << dshift)).wrapping_sub(p1[q1] as u32);
                        p1[q1] = v as i32;
                        q1 += 1;
                    }
                    self.residues.copy_within(tmp..tmp + filter_order, 0);
                    length2 -= tmp;
                }
            }
            _ => {}
        }
        if dmode > 0 && dmode < 6 {
            p1[0] = bp1;
            p2[0] = bp2;
        }
        Ok(())
    }

    /// Decodes one frame (from its sync code to its end) to one plane per channel: signed values
    /// of the stream's bit depth (8-bit audio centred on 0).
    pub fn decode(&mut self, pkt: &[u8]) -> Result<Vec<Vec<i32>>> {
        if pkt.len() < MIN_FRAME_HEADER_BYTES {
            return Err(Error::Invalid("frame too short"));
        }
        let mut gb = Bits::new(pkt);
        let hdr = decode_frame_header(&mut gb, &mut self.ti)?;
        let ti = &self.ti;
        if ti.codec != CODEC_MONO_STEREO && ti.codec != CODEC_MULTICHANNEL {
            return Err(Error::Unsupported("codec type"));
        }
        if ti.data_type != 0 {
            return Err(Error::Unsupported("data type"));
        }
        if ti.codec == CODEC_MONO_STEREO && ti.channels > 2 {
            return Err(Error::Invalid("channel count"));
        }
        if ti.channels > MAX_CHANNELS {
            return Err(Error::Unsupported("more than 6 channels"));
        }
        if ti.frame_samples == 0 {
            return Err(Error::Invalid("frame size"));
        }
        if !matches!(ti.bps, 8 | 16 | 24) {
            return Err(Error::Unsupported("bits per sample"));
        }
        let nch = ti.channels as usize;
        let codec = ti.codec;
        let nb = if hdr.last_frame_samples != 0 { hdr.last_frame_samples } else { ti.frame_samples } as usize;
        self.bps = ti.bps;
        self.set_sample_rate_params();
        self.nb_samples = nb;
        self.decoded = vec![vec![0i32; nb]; nch];

        if nb < 16 {
            for ch in &mut self.decoded {
                for v in ch.iter_mut() {
                    *v = gb.sbits(self.bps);
                }
            }
        } else {
            if codec == CODEC_MONO_STEREO {
                for chan in 0..nch {
                    self.decode_channel(&mut gb, chan)?;
                }
                if nch == 2 {
                    if gb.bit() == 1 {
                        gb.skip(6);
                    }
                    self.dmode = gb.bits(3) as u8;
                    self.decorrelate(&mut gb, 0, 1, nb - 1)?;
                }
            } else {
                let chan;
                if gb.bit() == 1 {
                    let mut mask = 0u32;
                    chan = gb.bits(4) as usize + 1;
                    if chan > nch {
                        return Err(Error::Invalid("decorrelation channel count"));
                    }
                    for i in 0..chan {
                        let nbit = gb.bits(4) as usize;
                        if nbit >= nch || mask & (1 << nbit) != 0 {
                            return Err(Error::Invalid("decorrelation channel"));
                        }
                        let m = &mut self.mcd[i];
                        m.present = gb.bit() == 1;
                        if m.present {
                            m.index = gb.bits(2) as u8;
                            m.chan2 = gb.bits(4) as usize;
                            if m.chan2 >= nch {
                                return Err(Error::Invalid("decorrelation channel"));
                            }
                            if m.index == 1 {
                                if nbit == m.chan2 || mask & (1 << m.chan2) != 0 {
                                    return Err(Error::Invalid("decorrelation channel"));
                                }
                                mask |= 1 << m.chan2;
                            } else if mask & (1 << m.chan2) == 0 {
                                return Err(Error::Invalid("decorrelation channel"));
                            }
                        }
                        m.chan1 = nbit;
                        mask |= 1 << nbit;
                    }
                } else {
                    chan = nch;
                    for (i, m) in self.mcd.iter_mut().enumerate().take(chan) {
                        m.present = false;
                        m.chan1 = i;
                    }
                }
                for i in 0..chan {
                    let m = self.mcd[i];
                    if m.present && m.index == 1 {
                        self.decode_channel(&mut gb, m.chan2)?;
                    }
                    self.decode_channel(&mut gb, m.chan1)?;
                    if m.present {
                        self.dmode = MC_DMODES[usize::from(m.index & 3)];
                        self.decorrelate(&mut gb, m.chan2, m.chan1, nb - 1)?;
                    }
                }
            }
            for (chan, d) in self.decoded.iter_mut().enumerate() {
                let mode = self.lpc_mode[chan & 15];
                if mode != 0 {
                    decode_lpc(d, mode, nb);
                }
                let shift = self.sample_shift[chan & 15];
                if shift > 0 {
                    for v in d.iter_mut() {
                        *v = (*v as u32).wrapping_shl(shift) as i32;
                    }
                }
            }
        }
        // ffmpeg's output formats (u8, s16, s32 << 8) keep only the stream's bit depth.
        let bps = self.bps;
        let mut out = std::mem::take(&mut self.decoded);
        for ch in &mut out {
            for v in ch.iter_mut() {
                *v = (*v << (32 - bps)) >> (32 - bps);
            }
        }
        Ok(out)
    }
}

/// The file's parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Info {
    pub sample_rate: u32,
    pub channels: u16,
    /// 8, 16 or 24.
    pub bits_per_sample: u16,
    /// ffmpeg's `AV_CH_*` mask, 0 when the file does not say.
    pub channel_mask: u64,
    /// Total samples per channel, as declared in the file.
    pub samples: u64,
}

impl Info {
    /// A decoded sample as `f32` in −1…1.
    pub fn to_f32(&self, v: i32) -> f32 {
        v as f32 / (1u32 << (self.bits_per_sample.clamp(8, 32) - 1)) as f32
    }
}

/// The parts of a `.tak` file.
struct Layout {
    streaminfo: Option<StreamInfo>,
    start: usize,
    end: usize,
}

fn u24le(d: &[u8], at: usize) -> Option<u32> {
    let b = d.get(at..at.checked_add(3)?)?;
    Some(u32::from(b[0]) | (u32::from(b[1]) << 8) | (u32::from(b[2]) << 16))
}

/// The header (`tBaK` and the metadata blocks, after an optional ID3v2 tag), as ffmpeg's demuxer
/// reads it; a file without `tBaK` is read as bare frames.
fn layout(data: &[u8]) -> Result<Layout> {
    let mut pos = 0usize;
    if data.len() >= 10 && data.starts_with(b"ID3") {
        let size = data[6..10].iter().fold(0usize, |a, &b| (a << 7) | usize::from(b & 0x7f));
        pos = 10 + size + if data[5] & 0x10 != 0 { 10 } else { 0 };
    }
    let rest = data.get(pos..).ok_or(Error::NotTak)?;
    if !rest.starts_with(b"tBaK") {
        return Ok(Layout { streaminfo: None, start: pos, end: data.len() });
    }
    pos += 4;
    let mut streaminfo = None;
    let mut last_frame: Option<u64> = None;
    loop {
        let ty = data.get(pos).ok_or(Error::Invalid("no end of metadata"))? & 0x7f;
        let size = u24le(data, pos + 1).ok_or(Error::Invalid("no end of metadata"))? as usize;
        pos += 4;
        match ty {
            0 => {
                let end = match last_frame {
                    Some(e) => usize::try_from(e).ok().and_then(|e| e.checked_add(pos)).map_or(data.len(), |e| e.min(data.len())),
                    None => data.len(),
                };
                return Ok(Layout { streaminfo, start: pos, end: end.max(pos) });
            }
            1 | 4 | 7 => {
                if size <= 3 {
                    return Err(Error::Invalid("metadata block size"));
                }
                if ty == 1 && streaminfo.is_some() {
                    return Err(Error::Invalid("two STREAMINFO blocks"));
                }
                let body = data.get(pos..pos + size - 3).ok_or(Error::Invalid("truncated metadata"))?;
                if ty == 1 {
                    streaminfo = Some(parse_streaminfo_block(body)?);
                } else if ty == 7 {
                    if size != 11 {
                        return Err(Error::Invalid("LAST_FRAME block size"));
                    }
                    let mut gb = Bits::new(body);
                    let at = gb.bits64(40);
                    last_frame = Some(at + u64::from(gb.bits(24)));
                }
                pos += size;
            }
            6 => {
                if size != 19 {
                    return Err(Error::Invalid("MD5 block size"));
                }
                pos += size;
            }
            _ => pos += size,
        }
        if pos >= data.len() {
            return Err(Error::Invalid("no end of metadata"));
        }
    }
}

/// The start of every frame in `data`, found as ffmpeg's parser finds them: a sync code whose
/// header decodes and whose header CRC matches.
fn frame_starts(data: &[u8]) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut ti = StreamInfo::default();
    for p in 0..data.len().saturating_sub(1) {
        if data[p] == 0xFF && data[p + 1] == 0xA0 && valid_header(&data[p..], &mut ti) {
            starts.push(p);
        }
    }
    starts
}

/// The file's parameters without decoding the audio.
pub fn probe(data: &[u8]) -> Result<Info> {
    let l = layout(data)?;
    let si = match l.streaminfo {
        Some(si) => si,
        None => {
            let body = data.get(l.start..l.end).unwrap_or(&[]);
            let mut ti = StreamInfo::default();
            frame_starts(body)
                .into_iter()
                .find_map(|p| {
                    let mut gb = Bits::new(&body[p..]);
                    decode_frame_header(&mut gb, &mut ti).ok().filter(|h| h.flags & FLAG_HAS_INFO != 0).map(|_| ti.clone())
                })
                .ok_or(Error::NotTak)?
        }
    };
    if !matches!(si.bps, 8 | 16 | 24) {
        return Err(Error::Unsupported("bits per sample"));
    }
    if si.channels == 0 || si.channels > MAX_CHANNELS {
        return Err(Error::Unsupported("more than 6 channels"));
    }
    Ok(Info { sample_rate: si.sample_rate, channels: si.channels as u16, bits_per_sample: si.bps as u16, channel_mask: si.ch_layout, samples: si.samples })
}

/// Decodes a whole `.tak` file to one plane per channel (signed values of the file's bit depth).
/// A frame that does not decode becomes silence of its length, so the timing holds.
pub fn decode_file(data: &[u8]) -> Result<(Info, Vec<Vec<i32>>)> {
    let info = probe(data)?;
    let l = layout(data)?;
    let body = data.get(l.start..l.end).unwrap_or(&[]);
    let starts = frame_starts(body);
    let nch = usize::from(info.channels);
    let mut planes: Vec<Vec<i32>> = vec![Vec::new(); nch];
    let mut dec = Decoder::new(l.streaminfo);
    let mut total = 0u64;
    for (k, &s) in starts.iter().enumerate() {
        let e = starts.get(k + 1).copied().unwrap_or(body.len());
        let pkt = &body[s..e];
        let decoded = dec.decode(pkt);
        let frame = match decoded {
            Ok(f) if f.len() == nch => f,
            _ => {
                // The frame's length from its header, as silence.
                let mut ti = dec.info().clone();
                let n = Bits::new(pkt);
                let mut gb = n;
                let Ok(h) = decode_frame_header(&mut gb, &mut ti) else { continue };
                let n = if h.last_frame_samples != 0 { h.last_frame_samples } else { ti.frame_samples } as usize;
                vec![vec![0; n]; nch]
            }
        };
        let n = frame.first().map_or(0, Vec::len) as u64;
        total = total.saturating_add(n.saturating_mul(nch as u64));
        if total > MAX_FILE_VALUES {
            return Err(Error::Unsupported("file too long"));
        }
        for (p, f) in planes.iter_mut().zip(frame) {
            p.extend_from_slice(&f);
        }
    }
    if planes.first().is_none_or(Vec::is_empty) {
        return Err(Error::Empty);
    }
    Ok((info, planes))
}
