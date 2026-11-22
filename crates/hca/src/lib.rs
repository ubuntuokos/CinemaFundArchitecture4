// Ported from FFmpeg n9.0.2 libavcodec/hcadec.c (Copyright the FFmpeg developers) and
// libavformat/hca.c (Copyright (c) 2020 Paul B Mahol), with the tables of libavcodec/hca_data.h,
// LGPL-2.1-or-later; see docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! Pure-Rust decoder for **CRI HCA** (High Compression Audio, `.hca`), the MDCT audio codec of
//! CRIWARE game middleware: header chunks (`fmt`, `comp` / `dec`, `ath`, `ciph`, …), 1 to 16
//! channels in tracks, the adaptive threshold of hearing, intensity stereo, high-frequency
//! reconstruction and the 56-bit key cipher. A frame is 1024 samples of 128-sample MDCT blocks.
//!
//! The output is the same as ffmpeg's `hca` decoder up to float rounding (both compute the
//! inverse MDCT in single precision; this port uses a double-precision DCT-IV).
//!
//! [`decode_file`] decodes a whole file to one plane of `f32` samples per channel;
//! [`Decoder`] decodes block by block.

mod tables;

use tables::{
    ATH_BASE_CURVE, DEQUANTIZER_SCALING, INTENSITY_RATIO, MAX_BITS, QUANT_SPECTRUM_BITS, QUANT_SPECTRUM_VALUE, QUANT_STEP_SIZE, SCALE_CONVERSION, SCALE_TABLE,
    WINDOW,
};

/// The most samples (all channels together) one [`decode_file`] may produce: 2 GiB of `f32`.
pub const MAX_SAMPLES: usize = 1 << 29;
/// Samples per channel in one HCA block.
pub const BLOCK_SAMPLES: usize = 1024;
/// The most channels a stream can have.
pub const MAX_CHANNELS: usize = 16;
/// ffmpeg's `scale_conv_bias`.
const SCALE_CONV_BIAS: i32 = 64;
/// Chunk tags have bit 7 of every byte masked (an encrypted header sets them).
const MASK: u32 = 0x7f7f_7f7f;

/// Errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("not an HCA file (no 'HCA' signature)")]
    NotHca,
    #[error("invalid HCA header: {0}")]
    Invalid(&'static str),
    #[error("unsupported HCA stream: {0}")]
    Unsupported(&'static str),
    #[error("the HCA stream is encrypted and needs its key")]
    KeyRequired,
    #[error("invalid HCA block: {0}")]
    Block(&'static str),
    #[error("no audio in the HCA file")]
    Empty,
    #[error("the decoded audio would exceed {MAX_SAMPLES} samples")]
    TooLarge,
}

/// The cipher key of an encrypted HCA stream (CRIWARE "key code" and the optional AWB subkey).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Key {
    pub key: u64,
    pub subkey: u16,
}

/// The stream parameters of the HCA header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// Format version (`0x0101`, `0x0102`, `0x0103`, `0x0200`, `0x0300`).
    pub version: u16,
    /// Offset of the first block (the header size).
    pub data_offset: usize,
    pub channels: u8,
    pub sample_rate: u32,
    pub block_count: u32,
    /// Bytes per block.
    pub block_size: u16,
    /// Samples of encoder delay and padding the format records (informational: like ffmpeg, the
    /// decoder does not trim them).
    pub encoder_delay: u16,
    pub encoder_padding: u16,
    pub track_count: u8,
    pub channel_config: u8,
    pub total_band_count: u8,
    pub base_band_count: u8,
    pub stereo_band_count: u8,
    pub bands_per_hfr_group: u8,
    /// Cipher type of the `ciph` chunk: 0 none, 1 fixed table (unsupported), 56 keyed.
    pub cipher_type: u16,
    ath_type: u16,
    hfr_group_count: u32,
}

impl Header {
    /// Whether the stream is encrypted with a key.
    pub fn encrypted(&self) -> bool {
        self.cipher_type == 56
    }

    /// Length of the stream in samples per channel (the format stores whole blocks).
    pub fn total_samples(&self) -> u64 {
        u64::from(self.block_count) * BLOCK_SAMPLES as u64
    }
}

/// A byte cursor with FFmpeg's `bytestream2` rules: a read past the end returns 0 and moves to
/// the end.
struct Cur<'a> {
    d: &'a [u8],
    p: usize,
}

impl Cur<'_> {
    fn left(&self) -> usize {
        self.d.len() - self.p
    }

    fn skip(&mut self, n: usize) {
        self.p += n.min(self.left());
    }

    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        if self.left() < N {
            self.p = self.d.len();
            return None;
        }
        let mut a = [0u8; N];
        a.copy_from_slice(&self.d[self.p..self.p + N]);
        self.p += N;
        Some(a)
    }

    fn u8(&mut self) -> u8 {
        self.take::<1>().map_or(0, |a| a[0])
    }

    fn peek_u8(&self) -> u8 {
        self.d.get(self.p).copied().unwrap_or(0)
    }

    fn be16(&mut self) -> u16 {
        self.take::<2>().map_or(0, u16::from_be_bytes)
    }

    fn be24(&mut self) -> u32 {
        self.take::<3>().map_or(0, |a| u32::from_be_bytes([0, a[0], a[1], a[2]]))
    }

    fn be32(&mut self) -> u32 {
        self.take::<4>().map_or(0, u32::from_be_bytes)
    }
}

const fn tag(a: u8, b: u8, c: u8) -> u32 {
    u32::from_be_bytes([a, b, c, 0])
}

/// ffmpeg's `ceil2`.
fn ceil2(a: u32, b: u32) -> u32 {
    a.checked_div(b).map_or(0, |q| q + u32::from(!a.is_multiple_of(b)))
}

/// The number of high-frequency groups of a band layout; the layout must fit the 128 bands.
fn hfr_groups(total: u8, base: u8, stereo: u8, bands_per_hfr: u8) -> Result<u32, Error> {
    if total > 128 || u16::from(base) + u16::from(stereo) > u16::from(total) {
        return Err(Error::Invalid("band count"));
    }
    let a = (i32::from(total) - (i32::from(base) + i32::from(stereo))) as u32;
    let groups = ceil2(a, u32::from(bands_per_hfr));
    if u64::from(base) + u64::from(stereo) + u64::from(groups) > 128 {
        return Err(Error::Invalid("band layout"));
    }
    Ok(groups)
}

/// Parse the header at the start of `bytes`.
pub fn parse_header(bytes: &[u8]) -> Result<Header, Error> {
    if bytes.len() < 12 {
        return Err(Error::NotHca);
    }
    if u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) & MASK != tag(b'H', b'C', b'A') {
        return Err(Error::NotHca);
    }
    let version = u16::from_be_bytes([bytes[4], bytes[5]]);
    let data_offset = usize::from(u16::from_be_bytes([bytes[6], bytes[7]]));
    if data_offset <= 8 {
        return Err(Error::Invalid("header size"));
    }
    // FFmpeg's decoder works on the header plus 10 key bytes (extradata of `data_offset + 10`)
    // and refuses less than 36 bytes.
    if data_offset + 10 < 36 {
        return Err(Error::Invalid("header too short"));
    }
    let header = bytes.get(..data_offset).ok_or(Error::Invalid("file ends inside the header"))?;
    let mut g = Cur { d: header, p: 8 };
    if g.be32() & MASK != tag(b'f', b'm', b't') {
        return Err(Error::Invalid("no fmt chunk"));
    }
    let channels = g.u8();
    let sample_rate = g.be24();
    let block_count = g.be32();
    let encoder_delay = g.be16();
    let encoder_padding = g.be16();
    let chunk = g.be32() & MASK;
    let (block_size, track_count, channel_config, total, base, stereo, bands_per_hfr);
    if chunk == u32::from_be_bytes(*b"comp") {
        block_size = g.be16();
        g.skip(2);
        track_count = g.u8();
        channel_config = g.u8();
        total = g.u8();
        base = g.u8();
        stereo = g.u8();
        bands_per_hfr = g.u8();
        g.skip(2);
    } else if chunk == tag(b'd', b'e', b'c') {
        block_size = g.be16();
        g.skip(2);
        total = g.u8().wrapping_add(1);
        let mut b = g.u8().wrapping_add(1);
        track_count = g.peek_u8() >> 4;
        channel_config = g.u8() & 0xF;
        if g.u8() == 0 {
            b = total;
        }
        base = b;
        stereo = total.wrapping_sub(base);
        bands_per_hfr = 0;
    } else {
        return Err(Error::Invalid("no comp or dec chunk"));
    }
    if block_size < 8 {
        return Err(Error::Invalid("block size"));
    }
    if channels == 0 || usize::from(channels) > MAX_CHANNELS {
        return Err(Error::Invalid("channel count"));
    }
    if sample_rate == 0 {
        return Err(Error::Invalid("sample rate"));
    }
    if total > 128 {
        return Err(Error::Invalid("band count"));
    }
    // The optional chunks.
    let mut ath_type = if version >= 0x200 { 0 } else { 1 };
    let mut cipher_type = 0;
    while g.left() >= 4 {
        let chunk = g.be32() & MASK;
        if chunk == tag(b'v', b'b', b'r') {
            g.skip(4);
        } else if chunk == tag(b'a', b't', b'h') {
            ath_type = g.be16();
        } else if chunk == tag(b'r', b'v', b'a') {
            g.skip(4);
        } else if chunk == u32::from_be_bytes(*b"comm") {
            let n = usize::from(g.u8());
            g.skip(n * 8);
        } else if chunk == u32::from_be_bytes(*b"ciph") {
            cipher_type = g.be16();
        } else if chunk == u32::from_be_bytes(*b"loop") {
            g.skip(12);
        } else {
            break; // "pad" or anything unknown ends the chunks
        }
    }
    let hfr_group_count = hfr_groups(total, base, stereo, bands_per_hfr)?;
    Ok(Header {
        version,
        data_offset,
        channels,
        sample_rate,
        block_count,
        block_size,
        encoder_delay,
        encoder_padding,
        track_count,
        channel_config,
        total_band_count: total,
        base_band_count: base,
        stereo_band_count: stereo,
        bands_per_hfr_group: bands_per_hfr,
        cipher_type,
        ath_type,
        hfr_group_count,
    })
}

// ---------------------------------------------------------------------------------------------
// Cipher

fn cipher56_create_table(r: &mut [u8], key: u8) {
    let mul = u32::from(((key & 1) << 3) | 5);
    let add = u32::from((key & 0xE) | 1);
    let mut key = u32::from(key >> 4);
    for v in r.iter_mut().take(16) {
        key = (key * mul + add) & 0xF;
        *v = key as u8;
    }
}

fn cipher56_init(mut keycode: u64) -> [u8; 256] {
    let mut base = [0u8; 256];
    let (mut base_r, mut base_c) = ([0u8; 16], [0u8; 16]);
    let mut kc = [0u8; 8];
    // 56-bit keycode (the upper 8 bits are unused): keycode - 1
    keycode = keycode.saturating_sub(1);
    for k in kc.iter_mut().take(7) {
        *k = (keycode & 0xFF) as u8;
        keycode >>= 8;
    }
    let seed = [
        kc[1],
        kc[1] ^ kc[6],
        kc[2] ^ kc[3],
        kc[2],
        kc[2] ^ kc[1],
        kc[3] ^ kc[4],
        kc[3],
        kc[3] ^ kc[2],
        kc[4] ^ kc[5],
        kc[4],
        kc[4] ^ kc[3],
        kc[5] ^ kc[6],
        kc[5],
        kc[5] ^ kc[4],
        kc[6] ^ kc[1],
        kc[6],
    ];
    cipher56_create_table(&mut base_r, kc[0]);
    for (r, s) in seed.iter().enumerate() {
        cipher56_create_table(&mut base_c, *s);
        let nb = base_r[r] << 4;
        for c in 0..16 {
            base[r * 16 + c] = nb | base_c[c];
        }
    }
    let mut cipher = [0u8; 256];
    let (mut x, mut pos) = (0usize, 1usize);
    for _ in 0..256 {
        x = (x + 17) & 0xFF;
        if base[x] != 0 && base[x] != 0xFF && pos < 256 {
            cipher[pos] = base[x];
            pos += 1;
        }
    }
    cipher[0] = 0;
    cipher[0xFF] = 0xFF;
    cipher
}

/// The byte substitution table of the keyed cipher (type 56): decrypting maps every encrypted
/// byte `b` to `table[b]`.
pub fn cipher_table(key: Key) -> [u8; 256] {
    let mut keycode = key.key;
    if keycode == 0 {
        return core::array::from_fn(|i| i as u8);
    }
    if key.subkey != 0 {
        let sub = u64::from(key.subkey);
        keycode = keycode.wrapping_mul((sub << 16) | (u64::from(!key.subkey) + 2));
    }
    cipher56_init(keycode)
}

// ---------------------------------------------------------------------------------------------
// Bit reader and the inverse MDCT

/// An MSB-first bit reader; reads past the end give zeros, and the position can step back.
struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn get(&mut self, n: u32) -> u32 {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = self.d.get(self.pos >> 3).copied().unwrap_or(0);
            v = (v << 1) | u32::from((byte >> (7 - (self.pos & 7))) & 1);
            self.pos += 1;
        }
        v
    }

    fn skip(&mut self, n: i32) {
        let end = self.d.len() * 8 + 8;
        self.pos = (self.pos as i64 + i64::from(n)).clamp(0, end as i64) as usize;
    }
}

/// DCT-IV of length 128 through a 64-point complex FFT (double precision).
struct Dct4 {
    pre: [(f64, f64); 64],
    post: [(f64, f64); 64],
    fft_w: [(f64, f64); 32],
    rev: [u8; 64],
}

impl Dct4 {
    const N: usize = 128;

    fn new() -> Self {
        let n = Self::N as f64;
        let pre = core::array::from_fn(|i| {
            let a = -std::f64::consts::PI * (4 * i + 1) as f64 / (4.0 * n);
            (a.cos(), a.sin())
        });
        let post = core::array::from_fn(|i| {
            let a = -std::f64::consts::PI * i as f64 / n;
            (a.cos(), a.sin())
        });
        let fft_w = core::array::from_fn(|i| {
            let a = -2.0 * std::f64::consts::PI * i as f64 / 64.0;
            (a.cos(), a.sin())
        });
        let rev = core::array::from_fn(|i| (i as u8).reverse_bits() >> 2);
        Self { pre, post, fft_w, rev }
    }

    /// `out[k] = sum_n x[n] cos(pi/128 (n + 1/2)(k + 1/2))`.
    fn run(&self, x: &[f32; 128], out: &mut [f64; 128]) {
        let mut z = [(0f64, 0f64); 64];
        for n in 0..64 {
            let (re, im) = (f64::from(x[2 * n]), f64::from(x[127 - 2 * n]));
            let (wr, wi) = self.pre[n];
            z[self.rev[n] as usize] = (re * wr - im * wi, re * wi + im * wr);
        }
        let mut len = 2;
        while len <= 64 {
            let step = 64 / len;
            for start in (0..64).step_by(len) {
                for k in 0..len / 2 {
                    let (wr, wi) = self.fft_w[k * step];
                    let (ar, ai) = z[start + k];
                    let (br, bi) = z[start + k + len / 2];
                    let (tr, ti) = (br * wr - bi * wi, br * wi + bi * wr);
                    z[start + k] = (ar + tr, ai + ti);
                    z[start + k + len / 2] = (ar - tr, ai - ti);
                }
            }
            len *= 2;
        }
        for k in 0..64 {
            let (wr, wi) = self.post[k];
            let (zr, zi) = z[k];
            out[2 * k] = zr * wr - zi * wi;
            out[127 - 2 * k] = -(zr * wi + zi * wr);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Decoder

#[derive(Clone)]
struct Channel {
    base: [f32; 128],
    factors: [f32; 128],
    imdct_in: [f32; 128],
    imdct_prev: [f32; 128],
    scale_factors: [i8; 128],
    scale: [u8; 128],
    intensity: [i8; 8],
    count: usize,
    chan_type: u8,
}

impl Channel {
    fn new() -> Self {
        Self {
            base: [0.0; 128],
            factors: [0.0; 128],
            imdct_in: [0.0; 128],
            imdct_prev: [0.0; 128],
            scale_factors: [0; 128],
            scale: [0; 128],
            intensity: [0; 8],
            count: 0,
            chan_type: 0,
        }
    }
}

/// Block-by-block HCA decoder.
pub struct Decoder {
    header: Header,
    ch: Vec<Channel>,
    ath: [u8; 128],
    cipher: Option<[u8; 256]>,
    hfr_start: usize,
    dct: Dct4,
}

fn ath_init(ath_type: u16, sample_rate: u32) -> Result<[u8; 128], Error> {
    let mut ath = [0u8; 128];
    match ath_type {
        0 => {}
        1 => {
            let mut acc = 0u32;
            for i in 0..128 {
                acc = acc.wrapping_add(sample_rate);
                let index = (acc >> 13) as usize;
                if index >= 654 {
                    ath[i..].fill(0xFF);
                    break;
                }
                ath[i] = ATH_BASE_CURVE[index];
            }
        }
        _ => return Err(Error::Invalid("ath type")),
    }
    Ok(ath)
}

impl Decoder {
    /// A decoder for the stream whose header is `header` (from [`parse_header`]); `key` is needed
    /// for an encrypted stream.
    pub fn new(mut header: Header, key: Option<Key>) -> Result<Self, Error> {
        // The fields are public: check again what `parse_header` checked, and derive the group
        // count from the values in use.
        if header.channels == 0 || usize::from(header.channels) > MAX_CHANNELS {
            return Err(Error::Invalid("channel count"));
        }
        if header.sample_rate == 0 {
            return Err(Error::Invalid("sample rate"));
        }
        header.hfr_group_count = hfr_groups(header.total_band_count, header.base_band_count, header.stereo_band_count, header.bands_per_hfr_group)?;
        let cipher = match header.cipher_type {
            0 => None,
            56 => match key {
                Some(k) if k.key != 0 => Some(cipher_table(k)),
                _ => return Err(Error::KeyRequired),
            },
            _ => return Err(Error::Unsupported("cipher type")),
        };
        let ath = ath_init(header.ath_type, header.sample_rate)?;
        let channels = usize::from(header.channels);
        let tracks = usize::from(header.track_count.max(1));
        // The channel types of the stereo tracks: 1 is the left, 2 the right channel of a pair.
        let mut r = [0u8; 16];
        let b = channels / tracks;
        if header.stereo_band_count != 0 && b > 1 {
            let cfg = header.channel_config;
            for t in 0..tracks {
                let x = &mut r[t * b..];
                match b {
                    2 | 3 => {
                        x[0] = 1;
                        x[1] = 2;
                    }
                    4 => {
                        x[0] = 1;
                        x[1] = 2;
                        if cfg == 0 {
                            x[2] = 1;
                            x[3] = 2;
                        }
                    }
                    5 => {
                        x[0] = 1;
                        x[1] = 2;
                        if cfg <= 2 {
                            x[3] = 1;
                            x[4] = 2;
                        }
                    }
                    6 | 7 => {
                        x[0] = 1;
                        x[1] = 2;
                        x[4] = 1;
                        x[5] = 2;
                    }
                    8 => {
                        x[0] = 1;
                        x[1] = 2;
                        x[4] = 1;
                        x[5] = 2;
                        x[6] = 1;
                        x[7] = 2;
                    }
                    _ => {}
                }
            }
        }
        let hfr_start = usize::from(header.base_band_count) + usize::from(header.stereo_band_count);
        let mut ch = Vec::with_capacity(channels);
        for &t in r.iter().take(channels) {
            let mut c = Channel::new();
            c.chan_type = t;
            c.count = usize::from(header.base_band_count) + if t != 2 { usize::from(header.stereo_band_count) } else { 0 };
            if c.count > 128 {
                return Err(Error::Invalid("band count"));
            }
            ch.push(c);
        }
        Ok(Self { header, ch, ath, cipher, hfr_start, dct: Dct4::new() })
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Forget the overlap of the previous block (after a seek).
    pub fn flush(&mut self) {
        for c in &mut self.ch {
            c.imdct_prev = [0.0; 128];
        }
    }

    /// Decode one block of [`Header::block_size`] bytes, appending [`BLOCK_SAMPLES`] samples to
    /// each plane of `out`. On an error nothing is appended and the state is unchanged.
    pub fn decode_block(&mut self, block: &[u8], out: &mut [Vec<f32>]) -> Result<(), Error> {
        if block.len() <= 8 {
            return Err(Error::Block("too short"));
        }
        if block[0] != 0xFF || block[1] != 0xFF {
            return Err(Error::Block("no sync word"));
        }
        let deciphered;
        let data: &[u8] = if let Some(table) = &self.cipher {
            deciphered = block.iter().map(|&b| table[usize::from(b)]).collect::<Vec<u8>>();
            &deciphered
        } else {
            block
        };
        let mut gb = Bits { d: data, pos: 0 };
        if gb.get(16) != 0xFFFF {
            return Err(Error::Block("no sync word"));
        }
        let nch = self.ch.len();
        // The 9-bit noise level is read before the 7-bit one.
        let hi = gb.get(9) as i32;
        let lo = gb.get(7) as i32;
        let packed_noise_level = (hi << 8) - lo;
        let hfr_groups = self.header.hfr_group_count as usize;
        let hfr_start = self.hfr_start;
        for c in &mut self.ch {
            unpack(c, &mut gb, hfr_groups, hfr_start, packed_noise_level, &self.ath);
        }
        let total = usize::from(self.header.total_band_count);
        let base = usize::from(self.header.base_band_count);
        let stereo = usize::from(self.header.stereo_band_count);
        let bands_per_hfr = usize::from(self.header.bands_per_hfr_group);
        let mut planes = vec![[0f32; BLOCK_SAMPLES]; nch];
        for i in 0..8 {
            for c in &mut self.ch {
                dequantize(c, &mut gb);
            }
            for c in &mut self.ch {
                reconstruct_hfr(c, hfr_groups, bands_per_hfr, hfr_start, total);
            }
            for k in 0..nch.saturating_sub(1) {
                let (a, b) = self.ch.split_at_mut(k + 1);
                apply_intensity_stereo(&mut a[k], &mut b[0], i, total - base, base, stereo);
            }
            for (c, plane) in self.ch.iter_mut().zip(planes.iter_mut()) {
                run_imdct(&self.dct, c, &mut plane[i * 128..(i + 1) * 128]);
            }
        }
        for (o, p) in out.iter_mut().zip(planes.iter()) {
            o.extend_from_slice(p);
        }
        Ok(())
    }
}

fn unpack(c: &mut Channel, gb: &mut Bits<'_>, hfr_groups: usize, hfr_start: usize, packed_noise_level: i32, ath: &[u8; 128]) {
    let delta_bits = gb.get(3);
    if delta_bits > 5 {
        for i in 0..c.count {
            c.scale_factors[i] = gb.get(6) as i8;
        }
    } else if delta_bits != 0 {
        let mut factor = gb.get(6) as i32;
        let max_value = (1i32 << delta_bits) - 1;
        let half_max = max_value >> 1;
        c.scale_factors[0] = factor as i8;
        for i in 1..c.count {
            let delta = gb.get(delta_bits) as i32;
            if delta == max_value {
                factor = gb.get(6) as i32;
            } else {
                factor += delta - half_max;
            }
            factor = factor.clamp(0, 63);
            c.scale_factors[i] = factor as i8;
        }
    } else {
        c.scale_factors = [0; 128];
    }
    if c.chan_type == 2 {
        c.intensity[0] = gb.get(4) as i8;
        if c.intensity[0] < 15 {
            for i in 1..8 {
                c.intensity[i] = gb.get(4) as i8;
            }
        }
    } else {
        for i in 0..hfr_groups {
            c.scale_factors[hfr_start + i] = gb.get(6) as i8;
        }
    }
    for i in 0..c.count {
        let mut scale = i32::from(c.scale_factors[i]);
        if scale != 0 {
            scale = i32::from(ath[i]) + ((packed_noise_level + i as i32) >> 8) - ((scale * 5) >> 1) + 2;
            scale = i32::from(SCALE_TABLE[scale.clamp(0, 58) as usize]);
        }
        c.scale[i] = scale as u8;
    }
    c.scale[c.count..].fill(0);
    for i in 0..c.count {
        c.base[i] = DEQUANTIZER_SCALING[c.scale_factors[i] as usize] * QUANT_STEP_SIZE[usize::from(c.scale[i])];
    }
}

fn dequantize(c: &mut Channel, gb: &mut Bits<'_>) {
    for i in 0..c.count {
        let scale = usize::from(c.scale[i]);
        let nb_bits = u32::from(MAX_BITS[scale]);
        let mut value = gb.get(nb_bits) as i32;
        let factor;
        if scale > 7 {
            value = (1 - ((value & 1) << 1)) * (value >> 1);
            if value == 0 {
                gb.skip(-1);
            }
            factor = value as f32;
        } else {
            let idx = value as usize + (scale << 4);
            gb.skip(i32::from(QUANT_SPECTRUM_BITS[idx]) - nb_bits as i32);
            factor = f32::from(QUANT_SPECTRUM_VALUE[idx]);
        }
        c.factors[i] = factor;
    }
    c.factors[c.count..].fill(0.0);
    for i in 0..128 {
        c.imdct_in[i] = c.factors[i] * c.base[i];
    }
}

fn reconstruct_hfr(c: &mut Channel, hfr_groups: usize, bands_per_hfr: usize, start_band: usize, total: usize) {
    if c.chan_type == 2 || bands_per_hfr == 0 {
        return;
    }
    let (mut k, mut l) = (start_band, start_band as i32 - 1);
    for i in 0..hfr_groups {
        let mut j = 0;
        while j < bands_per_hfr && k < total && l >= 0 {
            let d = (i32::from(c.scale_factors[start_band + i]) - i32::from(c.scale_factors[l as usize])).clamp(-64, 63);
            c.imdct_in[k] = SCALE_CONVERSION[(SCALE_CONV_BIAS + d) as usize] * c.imdct_in[l as usize];
            j += 1;
            k += 1;
            l -= 1;
        }
    }
    c.imdct_in[127] = 0.0;
}

fn apply_intensity_stereo(c1: &mut Channel, c2: &mut Channel, index: usize, band_count: usize, base: usize, stereo: usize) {
    if c1.chan_type != 1 || stereo == 0 {
        return;
    }
    let ratio_l = INTENSITY_RATIO[c2.intensity[index] as usize & 15];
    let ratio_r = ratio_l - 2.0;
    for i in 0..band_count {
        c2.imdct_in[base + i] = c1.imdct_in[base + i] * ratio_r;
        c1.imdct_in[base + i] *= ratio_l;
    }
}

fn run_imdct(dct: &Dct4, c: &mut Channel, out: &mut [f32]) {
    let mut u = [0f64; 128];
    dct.run(&c.imdct_in, &mut u);
    // ffmpeg's float MDCT, inverse, length 128, scale 1/8: the middle half of the IMDCT.
    let mut cur = [0f32; 128];
    for n in 0..128 {
        cur[n] = (0.125 * u[127 - n]) as f32;
    }
    for a in 0..64 {
        let s0 = c.imdct_prev[64 + a];
        let s1 = cur[63 - a];
        let wi = WINDOW[a];
        let wj = WINDOW[127 - a];
        out[a] = s0 * wj - s1 * wi;
        out[127 - a] = s0 * wi + s1 * wj;
    }
    c.imdct_prev = cur;
}

/// Decode a whole `.hca` file: the header and one plane of samples per channel. A block that
/// cannot be decoded (no sync word) is replaced by silence and counted in the second value.
pub fn decode_file(bytes: &[u8], key: Option<Key>) -> Result<(Header, Vec<Vec<f32>>, usize), Error> {
    let header = parse_header(bytes)?;
    let mut dec = Decoder::new(header.clone(), key)?;
    let size = usize::from(header.block_size);
    let data = &bytes[header.data_offset..];
    let blocks = (data.len() / size).min(header.block_count as usize);
    let channels = usize::from(header.channels);
    match blocks.checked_mul(BLOCK_SAMPLES).and_then(|n| n.checked_mul(channels)) {
        Some(n) if n <= MAX_SAMPLES => {}
        _ => return Err(Error::TooLarge),
    }
    let mut planes: Vec<Vec<f32>> = Vec::with_capacity(channels);
    for _ in 0..channels {
        let mut p = Vec::new();
        p.try_reserve_exact(blocks * BLOCK_SAMPLES).map_err(|_| Error::TooLarge)?;
        planes.push(p);
    }
    let mut damaged = 0;
    for b in 0..blocks {
        if dec.decode_block(&data[b * size..(b + 1) * size], &mut planes).is_err() {
            damaged += 1;
            for p in &mut planes {
                p.resize(p.len() + BLOCK_SAMPLES, 0.0);
            }
        }
    }
    if blocks == 0 {
        return Err(Error::Empty);
    }
    Ok((header, planes, damaged))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dct4_matches_the_definition() {
        let dct = Dct4::new();
        let x: [f32; 128] = core::array::from_fn(|i| ((i * 37 % 101) as f32 - 50.0) / 7.0);
        let mut got = [0f64; 128];
        dct.run(&x, &mut got);
        for (k, g) in got.iter().enumerate() {
            let want: f64 = (0..128).map(|n| f64::from(x[n]) * (std::f64::consts::PI / 128.0 * (n as f64 + 0.5) * (k as f64 + 0.5)).cos()).sum();
            assert!((g - want).abs() < 1e-9, "k={k}: {g} vs {want}");
        }
    }

    #[test]
    fn cipher_table_is_a_permutation() {
        for key in [Key { key: 1, subkey: 0 }, Key { key: 0xCF222F1FE0748978, subkey: 0 }, Key { key: 0x1234_5678_9ABC, subkey: 77 }] {
            let t = cipher_table(key);
            let mut seen = [false; 256];
            for b in t {
                assert!(!seen[usize::from(b)]);
                seen[usize::from(b)] = true;
            }
            assert_eq!((t[0], t[255]), (0, 255));
        }
    }
}
