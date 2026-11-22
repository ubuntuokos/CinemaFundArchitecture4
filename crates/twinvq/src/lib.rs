// Ported from FFmpeg n7.1.1: libavformat/vqf.c (Copyright (c) 2009 Vitor Sessak),
// libavcodec/twinvq.c, twinvqdec.c, twinvq.h (Copyright (c) 2009 Vitor Sessak), the tables of
// twinvq_data.h and metasound_twinvq_data.h, and the helpers of lsp.c, sinewin.c and float_dsp.c;
// all LGPL-2.1-or-later (`COPYING.LGPL-2.1`); see docs/licensing.md. CFA4 changes
// GPL-3.0-or-later.

//! Pure-Rust decoder for **NTT TwinVQ** audio in VQF files (`.vqf`, `.vql`, `.vqe`), codec backlog
//! CB-31.2.
//!
//! - [`probe`] and [`parse`] read the header like ffmpeg's `vqf` demuxer.
//! - [`decode`] cuts the bit-packed frames and returns every channel's samples as floats.
//!
//! The decoder follows ffmpeg's float arithmetic, so the samples agree with ffmpeg's to a few
//! ulps (the inverse MDCT is computed in double precision).

mod tables;

use tables as t;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid VQF data: {0}")]
    Invalid(&'static str),
    #[error("unsupported VQF mode: {0}")]
    Unsupported(&'static str),
    #[error("not a VQF file")]
    NotVqf,
}

pub type Result<T> = std::result::Result<T, Error>;

/// `TWIN` followed by the version string.
pub fn probe(head: &[u8]) -> bool {
    head.get(..4) == Some(b"TWIN")
}

/// What the header says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stream {
    pub channels: usize,
    pub sample_rate: u32,
    /// Bits per second (all channels).
    pub bit_rate: u32,
    /// Samples per channel in a frame.
    pub frame_samples: usize,
    /// Bits of one frame.
    pub frame_bits: usize,
    /// Byte offset of the first frame.
    pub data_start: usize,
    mode: usize,
}

/// The decoded sound.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    pub channels: Vec<Vec<f32>>,
}

fn be32(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

/// Mode index by (rate in kHz, kbit/s per channel), as the decoder's `switch` and the demuxer's
/// frame size table.
fn mode_of(rate_flag: u32, ibps: u32) -> Option<usize> {
    Some(match (rate_flag, ibps) {
        (8, 8) => 0,
        (11, 8) => 1,
        (11, 10) => 2,
        (16, 16) => 3,
        (22, 20) => 4,
        (22, 24) => 5,
        (22, 32) => 6,
        (44, 40) => 7,
        (44, 48) => 8,
        _ => return None,
    })
}

/// Read the header like `vqf_read_header` does.
pub fn parse(data: &[u8]) -> Result<Stream> {
    if !probe(data) {
        return Err(Error::NotVqf);
    }
    let mut pos = 12usize;
    let mut header_size = i64::from(be32(data, pos).ok_or(Error::Invalid("short header"))? as i32);
    pos += 4;
    if header_size < 0 {
        return Err(Error::Invalid("negative header size"));
    }
    let mut comm: Option<[u8; 12]> = None;
    while let Some(tag) = data.get(pos..pos + 4) {
        pos += 4;
        if tag == b"DATA" {
            break;
        }
        let len = be32(data, pos).ok_or(Error::Invalid("short chunk"))?;
        pos += 4;
        if len > (i32::MAX / 2) as u32 || header_size < 8 {
            return Err(Error::Invalid("malformed header"));
        }
        header_size -= 8;
        match tag {
            b"COMM" => {
                if len < 12 {
                    return Err(Error::Invalid("short COMM chunk"));
                }
                let b = data.get(pos..pos + 12).ok_or(Error::Invalid("short COMM chunk"))?;
                comm = Some(b.try_into().map_err(|_| Error::Invalid("short COMM chunk"))?);
                pos = pos.saturating_add(len as usize);
            }
            // ffmpeg reads only the first four bytes of DSIZ
            b"DSIZ" => pos += 4,
            b"YEAR" | b"ENCD" | b"EXTR" | b"_YMH" | b"_NTT" | b"_ID3" => {
                pos = pos.saturating_add(i64::from(len).min(header_size).max(0) as usize);
            }
            _ => {
                // metadata: the bytes up to the rest of the header
                let n = i64::from(len).min(header_size).max(0) as usize;
                if pos.saturating_add(n) > data.len() {
                    return Err(Error::Invalid("short metadata"));
                }
                pos += n;
            }
        }
        header_size -= i64::from(len);
        if header_size < 0 || pos >= data.len() {
            break;
        }
    }
    let comm = comm.ok_or(Error::Invalid("COMM chunk not found"))?;
    let channels = be32(&comm, 0).unwrap_or(0).wrapping_add(1);
    let read_bitrate = be32(&comm, 4).unwrap_or(0);
    let rate_flag = be32(&comm, 8).unwrap_or(0);
    if channels == 0 || channels > 2 {
        return Err(Error::Unsupported("channel count"));
    }
    let sample_rate = match rate_flag {
        44 => 44_100,
        22 => 22_050,
        11 => 11_025,
        8..=43 => rate_flag * 1000,
        _ => return Err(Error::Invalid("rate flag")),
    };
    let ibps = read_bitrate / channels;
    if !(8..=48).contains(&ibps) {
        return Err(Error::Invalid("bit rate per channel"));
    }
    let mode = mode_of(sample_rate / 1000, ibps).ok_or(Error::Unsupported("rate and bit rate combination"))?;
    let frame_samples = MODES[mode].size;
    let bit_rate = read_bitrate * 1000;
    let frame_bits = (u64::from(bit_rate) * frame_samples as u64 / u64::from(sample_rate)) as usize;
    Ok(Stream { channels: channels as usize, sample_rate, bit_rate, frame_samples, frame_bits, data_start: pos, mode })
}

/// Decode every whole frame of the file (the first two frames only prime the overlap and give no
/// sound, as in ffmpeg).
pub fn decode(data: &[u8], s: &Stream) -> Decoded {
    let mut dec = Decoder::new(s);
    let mut out = vec![Vec::new(); s.channels];
    let body = data.get(s.data_start..).unwrap_or(&[]);
    let mut pos = 0usize;
    let mut remaining: i32 = 0;
    let mut last_bits = 0u8;
    loop {
        // the demuxer's packet: bits to skip, the shared byte of the last packet, whole bytes
        let size = ((s.frame_bits as i64 - i64::from(remaining) + 7) >> 3).max(0) as usize;
        let Some(bytes) = body.get(pos..pos + size) else { break };
        let mut pkt = Vec::with_capacity(size + 2);
        pkt.push((8 - remaining) as u8);
        pkt.push(last_bits);
        pkt.extend_from_slice(bytes);
        pos += size;
        last_bits = bytes.last().copied().unwrap_or(0);
        remaining += (size as i32 * 8) - s.frame_bits as i32;
        dec.packet(&pkt, &mut out);
    }
    Decoded { channels: out }
}

// ---------------------------------------------------------------------------------------------
// mode tables

#[derive(Clone, Copy)]
enum Ft {
    Short = 0,
    Medium = 1,
    Long = 2,
    Ppc = 3,
}

struct FrameMode {
    sub: usize,
    bark_tab: &'static [u16],
    bark_env_size: usize,
    bark_cb: &'static [i16],
    bark_n_coef: usize,
    bark_n_bit: u32,
    cb0: &'static [i16],
    cb1: &'static [i16],
    cb_len_read: usize,
}

struct Mode {
    fmode: [FrameMode; 3],
    size: usize,
    n_lsp: usize,
    lsp: &'static [f32],
    lsp_bit0: u32,
    lsp_bit1: u32,
    lsp_bit2: u32,
    lsp_split: usize,
    ppc_shape_cb: &'static [i16],
    ppc_period_bit: u32,
    ppc_shape_bit: usize,
    ppc_shape_len: usize,
    pgain_bit: u32,
    peak_per2wid: i32,
}

macro_rules! fm {
    ($sub:expr, $bark:expr, $env:expr, $cb:expr, $nc:expr, $nb:expr, $c0:expr, $c1:expr, $len:expr) => {
        FrameMode {
            sub: $sub,
            bark_tab: &$bark,
            bark_env_size: $env,
            bark_cb: &$cb,
            bark_n_coef: $nc,
            bark_n_bit: $nb,
            cb0: &$c0,
            cb1: &$c1,
            cb_len_read: $len,
        }
    };
}

#[rustfmt::skip]
static MODES: [Mode; 9] = [
    // 8 kHz, 8 kbit/s
    Mode { fmode: [
        fm!(8, t::BARK_TAB_S08_64, 10, t::FCB08S, 1, 5, t::CB0808S0, t::CB0808S1, 18),
        fm!(2, t::BARK_TAB_M08_256, 20, t::FCB08M, 2, 5, t::CB0808M0, t::CB0808M1, 16),
        fm!(1, t::BARK_TAB_L08_512, 30, t::FCB08L, 3, 6, t::CB0808L0, t::CB0808L1, 17)],
      size: 512, n_lsp: 12, lsp: &t::METASOUND_LSP8, lsp_bit0: 1, lsp_bit1: 5, lsp_bit2: 3, lsp_split: 3,
      ppc_shape_cb: &t::SHAPE08, ppc_period_bit: 8, ppc_shape_bit: 28, ppc_shape_len: 20, pgain_bit: 6, peak_per2wid: 40 },
    // 11 kHz, 8 kbit/s
    Mode { fmode: [
        fm!(8, t::BARK_TAB_S11_64, 10, t::FCB11S, 1, 5, t::CB1108S0, t::CB1108S1, 29),
        fm!(2, t::BARK_TAB_M11_256, 20, t::FCB11M, 2, 5, t::CB1108M0, t::CB1108M1, 24),
        fm!(1, t::BARK_TAB_L11_512, 30, t::FCB11L, 3, 6, t::CB1108L0, t::CB1108L1, 27)],
      size: 512, n_lsp: 16, lsp: &t::METASOUND_LSP11, lsp_bit0: 1, lsp_bit1: 6, lsp_bit2: 4, lsp_split: 3,
      ppc_shape_cb: &t::SHAPE11, ppc_period_bit: 9, ppc_shape_bit: 36, ppc_shape_len: 30, pgain_bit: 7, peak_per2wid: 90 },
    // 11 kHz, 10 kbit/s
    Mode { fmode: [
        fm!(8, t::BARK_TAB_S11_64, 10, t::FCB11S, 1, 5, t::CB1110S0, t::CB1110S1, 21),
        fm!(2, t::BARK_TAB_M11_256, 20, t::FCB11M, 2, 5, t::CB1110M0, t::CB1110M1, 18),
        fm!(1, t::BARK_TAB_L11_512, 30, t::FCB11L, 3, 6, t::CB1110L0, t::CB1110L1, 20)],
      size: 512, n_lsp: 16, lsp: &t::METASOUND_LSP11, lsp_bit0: 1, lsp_bit1: 6, lsp_bit2: 4, lsp_split: 3,
      ppc_shape_cb: &t::SHAPE11, ppc_period_bit: 9, ppc_shape_bit: 36, ppc_shape_len: 30, pgain_bit: 7, peak_per2wid: 90 },
    // 16 kHz, 16 kbit/s
    Mode { fmode: [
        fm!(8, t::BARK_TAB_S16_128, 10, t::FCB16S, 1, 5, t::CB1616S0, t::CB1616S1, 16),
        fm!(2, t::BARK_TAB_M16_512, 20, t::FCB16M, 2, 5, t::CB1616M0, t::CB1616M1, 15),
        fm!(1, t::BARK_TAB_L16_1024, 30, t::FCB16L, 3, 6, t::CB1616L0, t::CB1616L1, 16)],
      size: 1024, n_lsp: 16, lsp: &t::METASOUND_LSP16, lsp_bit0: 1, lsp_bit1: 6, lsp_bit2: 4, lsp_split: 3,
      ppc_shape_cb: &t::SHAPE16, ppc_period_bit: 9, ppc_shape_bit: 56, ppc_shape_len: 60, pgain_bit: 7, peak_per2wid: 180 },
    // 22 kHz, 20 kbit/s
    Mode { fmode: [
        fm!(8, t::BARK_TAB_S22_128, 10, t::FCB22S_1, 1, 6, t::CB2220S0, t::CB2220S1, 18),
        fm!(2, t::BARK_TAB_M22_512, 20, t::FCB22M_1, 2, 6, t::CB2220M0, t::CB2220M1, 17),
        fm!(1, t::BARK_TAB_L22_1024, 32, t::FCB22L_1, 4, 6, t::CB2220L0, t::CB2220L1, 18)],
      size: 1024, n_lsp: 16, lsp: &t::METASOUND_LSP22, lsp_bit0: 1, lsp_bit1: 6, lsp_bit2: 4, lsp_split: 3,
      ppc_shape_cb: &t::SHAPE22_1, ppc_period_bit: 9, ppc_shape_bit: 56, ppc_shape_len: 36, pgain_bit: 7, peak_per2wid: 144 },
    // 22 kHz, 24 kbit/s
    Mode { fmode: [
        fm!(8, t::BARK_TAB_S22_128, 10, t::FCB22S_1, 1, 6, t::CB2224S0, t::CB2224S1, 15),
        fm!(2, t::BARK_TAB_M22_512, 20, t::FCB22M_1, 2, 6, t::CB2224M0, t::CB2224M1, 14),
        fm!(1, t::BARK_TAB_L22_1024, 32, t::FCB22L_1, 4, 6, t::CB2224L0, t::CB2224L1, 15)],
      size: 1024, n_lsp: 16, lsp: &t::METASOUND_LSP22, lsp_bit0: 1, lsp_bit1: 6, lsp_bit2: 4, lsp_split: 3,
      ppc_shape_cb: &t::SHAPE22_1, ppc_period_bit: 9, ppc_shape_bit: 56, ppc_shape_len: 36, pgain_bit: 7, peak_per2wid: 144 },
    // 22 kHz, 32 kbit/s
    Mode { fmode: [
        fm!(4, t::BARK_TAB_S22_128, 10, t::FCB22S_2, 1, 6, t::CB2232S0, t::CB2232S1, 11),
        fm!(2, t::BARK_TAB_M22_256, 20, t::FCB22M_2, 2, 6, t::CB2232M0, t::CB2232M1, 11),
        fm!(1, t::BARK_TAB_L22_512, 32, t::FCB22L_2, 4, 6, t::CB2232L0, t::CB2232L1, 12)],
      size: 512, n_lsp: 16, lsp: &t::LSP22_2, lsp_bit0: 1, lsp_bit1: 6, lsp_bit2: 4, lsp_split: 4,
      ppc_shape_cb: &t::SHAPE22_2, ppc_period_bit: 9, ppc_shape_bit: 56, ppc_shape_len: 36, pgain_bit: 7, peak_per2wid: 72 },
    // 44 kHz, 40 kbit/s
    Mode { fmode: [
        fm!(16, t::BARK_TAB_S44_128, 10, t::FCB44S, 1, 6, t::CB4440S0, t::CB4440S1, 18),
        fm!(4, t::BARK_TAB_M44_512, 20, t::FCB44M, 2, 6, t::CB4440M0, t::CB4440M1, 17),
        fm!(1, t::BARK_TAB_L44_2048, 40, t::FCB44L, 4, 6, t::CB4440L0, t::CB4440L1, 17)],
      size: 2048, n_lsp: 20, lsp: &t::METASOUND_LSP44, lsp_bit0: 1, lsp_bit1: 6, lsp_bit2: 4, lsp_split: 4,
      ppc_shape_cb: &t::SHAPE44, ppc_period_bit: 9, ppc_shape_bit: 84, ppc_shape_len: 54, pgain_bit: 7, peak_per2wid: 432 },
    // 44 kHz, 48 kbit/s
    Mode { fmode: [
        fm!(16, t::BARK_TAB_S44_128, 10, t::FCB44S, 1, 6, t::CB4448S0, t::CB4448S1, 15),
        fm!(4, t::BARK_TAB_M44_512, 20, t::FCB44M, 2, 6, t::CB4448M0, t::CB4448M1, 14),
        fm!(1, t::BARK_TAB_L44_2048, 40, t::FCB44L, 4, 6, t::CB4448L0, t::CB4448L1, 14)],
      size: 2048, n_lsp: 20, lsp: &t::METASOUND_LSP44, lsp_bit0: 1, lsp_bit1: 6, lsp_bit2: 4, lsp_split: 4,
      ppc_shape_cb: &t::SHAPE44, ppc_period_bit: 9, ppc_shape_bit: 84, ppc_shape_len: 54, pgain_bit: 7, peak_per2wid: 432 },
];

const GAIN_BITS: u32 = 8;
const SUB_GAIN_BITS: u32 = 5;
const WINDOW_TYPE_BITS: u32 = 4;
const AMP_MAX: f32 = 13_000.0;
const SUB_AMP_MAX: f32 = 4500.0;
const MULAW_MU: f32 = 100.0;
const PGAIN_MU: f32 = 200.0;
const PPC_SHAPE_CB_SIZE: usize = 64;

const WTYPE_TO_FTYPE: [Ft; 9] = [Ft::Long, Ft::Long, Ft::Short, Ft::Long, Ft::Medium, Ft::Long, Ft::Long, Ft::Medium, Ft::Medium];
const WTYPE_TO_WSIZE: [usize; 9] = [0, 0, 2, 2, 2, 1, 0, 1, 1];

fn rounded_div(a: i32, b: i32) -> i32 {
    (if a >= 0 { a + (b >> 1) } else { a - (b >> 1) }) / b
}

fn av_log2(v: u32) -> u32 {
    if v == 0 { 0 } else { 31 - v.leading_zeros() }
}

fn mulawinv(y: f32, clip: f32, mu: f32) -> f32 {
    let y = (y / clip).clamp(-1.0, 1.0);
    let sign = if y > 0.0 { 1.0f32 } else { -1.0 };
    let v = f64::from(clip * sign) * (((1.0 + mu).ln_f64() * f64::from(y.abs())).exp() - 1.0) / f64::from(mu);
    v as f32
}

trait LnF64 {
    fn ln_f64(self) -> f64;
}
impl LnF64 for f32 {
    fn ln_f64(self) -> f64 {
        f64::from(self).ln()
    }
}

/// An MSB-first bit reader: reads past the end give zeros (ffmpeg's buffers are zero padded).
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
    fn skip(&mut self, n: usize) {
        self.pos = self.pos.saturating_add(n);
    }
}

#[derive(Clone, Default)]
struct FrameData {
    window_type: usize,
    ftype: usize,
    main_coeffs: Vec<u8>,
    ppc_coeffs: Vec<u8>,
    gain_bits: [u8; 2],
    sub_gain_bits: [u8; 32],
    bark1: [[[u8; 4]; 16]; 2],
    bark_use_hist: [[u8; 16]; 2],
    lpc_idx1: [u8; 2],
    lpc_idx2: [[u8; 4]; 2],
    lpc_hist_idx: [u8; 2],
    p_coef: [u32; 2],
    g_coef: [u32; 2],
}

fn ftype_of(i: usize) -> Ft {
    match i {
        0 => Ft::Short,
        1 => Ft::Medium,
        2 => Ft::Long,
        _ => Ft::Ppc,
    }
}

struct Decoder {
    mode: &'static Mode,
    channels: usize,
    sample_rate: i32,
    bit_rate: i32,
    discarded: u32,
    lsp_hist: [[f32; 20]; 2],
    bark_hist: [[[f32; 40]; 2]; 3],
    permut: [Vec<i16>; 4],
    length: [[usize; 2]; 4],
    length_change: [usize; 4],
    bits_main_spec: [[[u32; 2]; 4]; 2],
    bits_main_spec_change: [usize; 4],
    n_div: [usize; 4],
    spectrum: Vec<f32>,
    curr_frame: Vec<f32>,
    prev_frame: Vec<f32>,
    last_block_pos: [usize; 2],
    cos_tabs: [Vec<f32>; 3],
    tmp_buf: Vec<f32>,
    scale: [f64; 3],
}

impl Decoder {
    fn new(s: &Stream) -> Self {
        let mode = &MODES[s.mode];
        let channels = s.channels;
        let table_size = 2 * mode.size * channels;
        let norm = if channels == 1 { 2.0f64 } else { 1.0 };
        let mut scale = [0.0; 3];
        let mut cos_tabs: [Vec<f32>; 3] = Default::default();
        for i in 0..3 {
            let bsize = mode.size / mode.fmode[i].sub;
            scale[i] = f64::from((-(norm / bsize as f64).sqrt() / 32768.0) as f32);
            let m = 4 * mode.size / mode.fmode[i].sub;
            let freq = 2.0 * std::f64::consts::PI / m as f64;
            let mut c = vec![0.0f32; m / 4];
            for j in 0..=m / 8 {
                c[j] = ((2 * j + 1) as f64 * freq).cos() as f32;
            }
            for j in 1..m / 8 {
                c[m / 4 - j] = c[j];
            }
            cos_tabs[i] = c;
        }
        let mut d = Self {
            mode,
            channels,
            sample_rate: s.sample_rate as i32,
            bit_rate: s.bit_rate as i32,
            discarded: 0,
            lsp_hist: [[0.0; 20]; 2],
            // ffmpeg's `FF_ARRAY_ELEMS(bark_hist)` is 3, so only three floats start at 0.1
            bark_hist: [[[0.0; 40]; 2]; 3],
            permut: Default::default(),
            length: [[0; 2]; 4],
            length_change: [0; 4],
            bits_main_spec: [[[0; 2]; 4]; 2],
            bits_main_spec_change: [0; 4],
            n_div: [0; 4],
            spectrum: vec![0.0; table_size],
            curr_frame: vec![0.0; table_size],
            prev_frame: vec![0.0; table_size],
            last_block_pos: [0; 2],
            cos_tabs,
            tmp_buf: vec![0.0; mode.size],
            scale,
        };
        d.bark_hist[0][0][..3].fill(0.1);
        d.init_bitstream_params(s.bit_rate as i32, s.sample_rate as i32);
        d
    }

    fn init_bitstream_params(&mut self, bit_rate: i32, sample_rate: i32) {
        let mtab = self.mode;
        let n_ch = self.channels as i32;
        let size = mtab.size as i32;
        let total_fr_bits = (i64::from(bit_rate) * i64::from(size) / i64::from(sample_rate)) as i32;
        let lsp_bits_per_block = n_ch * (mtab.lsp_bit0 + mtab.lsp_bit1 + mtab.lsp_split as u32 * mtab.lsp_bit2) as i32;
        let ppc_bits = n_ch * (mtab.pgain_bit as i32 + mtab.ppc_shape_bit as i32 + mtab.ppc_period_bit as i32);
        let mut bse_bits = [0i32; 3];
        for (i, b) in bse_bits.iter_mut().enumerate() {
            *b = n_ch * (mtab.fmode[i].bark_n_coef as i32 * mtab.fmode[i].bark_n_bit as i32 + 1);
        }
        let mut bsize_no_main_cb = [0i32; 3];
        bsize_no_main_cb[2] = bse_bits[2] + lsp_bits_per_block + ppc_bits + WINDOW_TYPE_BITS as i32 + n_ch * GAIN_BITS as i32;
        for i in 0..2 {
            bsize_no_main_cb[i] =
                lsp_bits_per_block + n_ch * GAIN_BITS as i32 + WINDOW_TYPE_BITS as i32 + mtab.fmode[i].sub as i32 * (bse_bits[i] + n_ch * SUB_GAIN_BITS as i32);
        }
        for i in 0..4 {
            let (bit_size, vect_size) =
                if i == 3 { (n_ch * mtab.ppc_shape_bit as i32, n_ch * mtab.ppc_shape_len as i32) } else { (total_fr_bits - bsize_no_main_cb[i], n_ch * size) };
            let n_div = ((bit_size + 13) / 14).max(1);
            self.n_div[i] = n_div as usize;
            let rounded_up = (bit_size + n_div - 1) / n_div;
            let rounded_down = bit_size / n_div;
            let num_rounded_down = rounded_up * n_div - bit_size;
            let num_rounded_up = n_div - num_rounded_down;
            self.bits_main_spec[0][i][0] = ((rounded_up + 1) / 2).max(0) as u32;
            self.bits_main_spec[1][i][0] = (rounded_up / 2).max(0) as u32;
            self.bits_main_spec[0][i][1] = ((rounded_down + 1) / 2).max(0) as u32;
            self.bits_main_spec[1][i][1] = (rounded_down / 2).max(0) as u32;
            self.bits_main_spec_change[i] = num_rounded_up.max(0) as usize;
            let rounded_up = (vect_size + n_div - 1) / n_div;
            let rounded_down = vect_size / n_div;
            let num_rounded_down = rounded_up * n_div - vect_size;
            let num_rounded_up = n_div - num_rounded_down;
            self.length[i] = [rounded_up.max(0) as usize, rounded_down.max(0) as usize];
            self.length_change[i] = num_rounded_up.max(0) as usize;
        }
        for ft in 0..4 {
            self.construct_perm_table(ftype_of(ft));
        }
    }

    fn construct_perm_table(&mut self, ftype: Ft) {
        let mtab = self.mode;
        let f = ftype as usize;
        let (size, block_size) = match ftype {
            Ft::Ppc => (self.channels, mtab.ppc_shape_len),
            _ => (self.channels * mtab.fmode[f].sub, mtab.size / mtab.fmode[f].sub),
        };
        let n_div = self.n_div[f];
        let len = self.length[f];
        let lc = self.length_change[f];
        // permutate_in_line
        let mut tmp = vec![0i16; 8192];
        for i in 0..len[0] {
            let shift =
                if size == 1 || (matches!(ftype, Ft::Long) && !n_div.is_multiple_of(size)) || (!matches!(ftype, Ft::Long) && n_div & 1 != 0) || i == len[1] {
                    0
                } else if matches!(ftype, Ft::Long) {
                    i
                } else {
                    i * i
                };
            let mut j = 0;
            while j < n_div && j + n_div * i < block_size * size {
                if let Some(e) = tmp.get_mut(i * n_div + j) {
                    *e = (i * n_div + (j + shift) % n_div) as i16;
                }
                j += 1;
            }
        }
        // transpose_perm
        let mut out = vec![0i16; 4096];
        let mut cont = 0;
        for i in 0..n_div {
            for j in 0..len[usize::from(i >= lc)] {
                if let (Some(o), Some(&v)) = (out.get_mut(cont), tmp.get(j * n_div + i)) {
                    *o = v;
                }
                cont += 1;
            }
        }
        // linear_perm
        let total = size * block_size;
        let bs = (total / size) as i32;
        for v in out.iter_mut().take(total) {
            let x = i32::from(*v);
            *v = (bs * (x % size as i32) + x / size as i32) as i16;
        }
        self.permut[f] = out;
    }

    /// One demuxed packet; `out` receives the samples once the first two packets are past.
    fn packet(&mut self, pkt: &[u8], out: &mut [Vec<f32>]) {
        let Some(bits) = self.read_bitstream(pkt) else { return };
        self.read_and_decode_spectrum(&bits);
        let mut frame = if self.discarded >= 2 { Some(vec![vec![0.0f32; self.mode.size]; self.channels]) } else { None };
        self.imdct_output(&bits, frame.as_mut());
        std::mem::swap(&mut self.curr_frame, &mut self.prev_frame);
        if let Some(f) = frame {
            for (o, c) in out.iter_mut().zip(f) {
                o.extend(c);
            }
        } else {
            self.discarded += 1;
        }
    }

    fn read_cb_data(&self, gb: &mut Bits<'_>, dst: &mut Vec<u8>, ftype: usize) {
        dst.clear();
        for i in 0..self.n_div[ftype] {
            let second = usize::from(i >= self.bits_main_spec_change[ftype]);
            dst.push(gb.get(self.bits_main_spec[0][ftype][second]) as u8);
            dst.push(gb.get(self.bits_main_spec[1][ftype][second]) as u8);
        }
    }

    fn read_bitstream(&self, buf: &[u8]) -> Option<FrameData> {
        let mtab = self.mode;
        let mut gb = Bits { d: buf, pos: 0 };
        let skip = gb.get(8) as usize;
        gb.skip(skip);
        let mut bits = FrameData { window_type: gb.get(WINDOW_TYPE_BITS) as usize, ..FrameData::default() };
        if bits.window_type > 8 {
            return None;
        }
        let ft = WTYPE_TO_FTYPE[bits.window_type];
        bits.ftype = ft as usize;
        let fm = &mtab.fmode[bits.ftype];
        let sub = fm.sub;
        let mut main = Vec::new();
        self.read_cb_data(&mut gb, &mut main, bits.ftype);
        bits.main_coeffs = main;
        for i in 0..self.channels {
            for j in 0..sub {
                for k in 0..fm.bark_n_coef {
                    bits.bark1[i][j][k] = gb.get(fm.bark_n_bit) as u8;
                }
            }
        }
        for i in 0..self.channels {
            for j in 0..sub {
                bits.bark_use_hist[i][j] = gb.get(1) as u8;
            }
        }
        if matches!(ft, Ft::Long) {
            for i in 0..self.channels {
                bits.gain_bits[i] = gb.get(GAIN_BITS) as u8;
            }
        } else {
            for i in 0..self.channels {
                bits.gain_bits[i] = gb.get(GAIN_BITS) as u8;
                for j in 0..sub {
                    bits.sub_gain_bits[i * sub + j] = gb.get(SUB_GAIN_BITS) as u8;
                }
            }
        }
        for i in 0..self.channels {
            bits.lpc_hist_idx[i] = gb.get(mtab.lsp_bit0) as u8;
            bits.lpc_idx1[i] = gb.get(mtab.lsp_bit1) as u8;
            for j in 0..mtab.lsp_split {
                bits.lpc_idx2[i][j] = gb.get(mtab.lsp_bit2) as u8;
            }
        }
        if matches!(ft, Ft::Long) {
            let mut ppc = Vec::new();
            self.read_cb_data(&mut gb, &mut ppc, 3);
            bits.ppc_coeffs = ppc;
            for i in 0..self.channels {
                bits.p_coef[i] = gb.get(mtab.ppc_period_bit);
                bits.g_coef[i] = gb.get(mtab.pgain_bit);
            }
        }
        Some(bits)
    }

    // -----------------------------------------------------------------------------------------
    // spectrum

    fn dequant(&self, cb_bits: &[u8], out: &mut [f32], ftype: usize, cb0: &[i16], cb1: &[i16], cb_len: usize) {
        let mut pos = 0usize;
        let mut it = cb_bits.iter();
        for i in 0..self.n_div[ftype] {
            let (mut sign0, mut sign1) = (1i32, 1i32);
            let length = self.length[ftype][usize::from(i >= self.length_change[ftype])];
            let second = usize::from(i >= self.bits_main_spec_change[ftype]);
            let mut tmp0 = usize::from(*it.next().unwrap_or(&0));
            if self.bits_main_spec[0][ftype][second] == 7 {
                if tmp0 & 0x40 != 0 {
                    sign0 = -1;
                }
                tmp0 &= 0x3F;
            }
            let mut tmp1 = usize::from(*it.next().unwrap_or(&0));
            if self.bits_main_spec[1][ftype][second] == 7 {
                if tmp1 & 0x40 != 0 {
                    sign1 = -1;
                }
                tmp1 &= 0x3F;
            }
            let (b0, b1) = (tmp0 * cb_len, tmp1 * cb_len);
            for j in 0..length {
                let a = i32::from(cb0.get(b0 + j).copied().unwrap_or(0));
                let b = i32::from(cb1.get(b1 + j).copied().unwrap_or(0));
                let idx = self.permut[ftype].get(pos + j).copied().unwrap_or(0) as usize;
                if let Some(o) = out.get_mut(idx) {
                    *o = (sign0 * a + sign1 * b) as f32;
                }
            }
            pos += length;
        }
    }

    fn dec_gain(&self, bits: &FrameData, ftype: usize, out: &mut [f32]) {
        let mtab = self.mode;
        let sub = mtab.fmode[ftype].sub;
        let step = (f64::from(AMP_MAX) / f64::from((1 << GAIN_BITS) - 1)) as f32;
        let sub_step = (f64::from(SUB_AMP_MAX) / f64::from((1 << SUB_GAIN_BITS) - 1)) as f32;
        if ftype == Ft::Long as usize {
            for i in 0..self.channels {
                let y = (f64::from(step) * 0.5 + f64::from(step) * f64::from(bits.gain_bits[i])) as f32;
                out[i] = ((1.0 / f64::from(1u32 << 13)) * f64::from(mulawinv(y, AMP_MAX, MULAW_MU))) as f32;
            }
        } else {
            for i in 0..self.channels {
                let y = (f64::from(step) * 0.5 + f64::from(step) * f64::from(bits.gain_bits[i])) as f32;
                let val = ((1.0 / f64::from(1u32 << 23)) * f64::from(mulawinv(y, AMP_MAX, MULAW_MU))) as f32;
                for j in 0..sub {
                    let ys = (f64::from(sub_step) * 0.5 + f64::from(sub_step) * f64::from(bits.sub_gain_bits[i * sub + j])) as f32;
                    out[i * sub + j] = val * mulawinv(ys, SUB_AMP_MAX, MULAW_MU);
                }
            }
        }
    }

    fn dec_bark_env(&mut self, bits: &FrameData, i: usize, j: usize, ftype: usize, gain: f32, out: &mut [f32]) {
        let mtab = self.mode;
        let fm = &mtab.fmode[ftype];
        let val = [0.4f32, 0.35, 0.28][ftype];
        let bark_n_coef = fm.bark_n_coef;
        let fw_cb_len = fm.bark_env_size / bark_n_coef;
        let use_hist = bits.bark_use_hist[i][j] != 0;
        let input = &bits.bark1[i][j];
        let hist = &mut self.bark_hist[ftype][i];
        let mut idx = 0usize;
        let mut o = 0usize;
        for k in 0..fw_cb_len {
            for jj in 0..bark_n_coef {
                let cbv = fm.bark_cb.get(fw_cb_len * usize::from(input[jj]) + k).copied().unwrap_or(0);
                let tmp2 = (f64::from(cbv) * (1.0 / 4096.0)) as f32;
                let mut st = if use_hist {
                    ((1.0 - f64::from(val)) * f64::from(tmp2) + f64::from(val) * f64::from(hist[idx]) + 1.0) as f32
                } else {
                    (f64::from(tmp2) + 1.0) as f32
                };
                hist[idx] = tmp2;
                if st < -1.0 {
                    st = 1.0;
                }
                let n = usize::from(fm.bark_tab[idx]);
                let v = st * gain;
                for e in out.iter_mut().skip(o).take(n) {
                    *e = v;
                }
                o += n;
                idx += 1;
            }
        }
    }

    fn decode_lsp(&mut self, bits: &FrameData, ch: usize, lsp: &mut [f32; 20]) {
        let mtab = self.mode;
        let n = mtab.n_lsp;
        let cb = mtab.lsp;
        let cb2_off = (1usize << mtab.lsp_bit1) * n;
        let cb3_off = cb2_off + (1usize << mtab.lsp_bit2) * n;
        let at = |i: usize| cb.get(i).copied().unwrap_or(0.0);
        let funny: [i32; 4] = [-2, if mtab.lsp_split == 4 { -2 } else { 1 }, if mtab.lsp_split == 4 { -2 } else { 1 }, 0];
        let idx1 = usize::from(bits.lpc_idx1[ch]);
        let mut j = 0usize;
        for i in 0..mtab.lsp_split {
            let chunk_end = (((i + 1) * n) as i32 + funny[i]) as usize / mtab.lsp_split;
            while j < chunk_end {
                let idx2 = usize::from(bits.lpc_idx2[ch][i]);
                lsp[j] = at(idx1 * n + j) + at(cb2_off + idx2 * n + j);
                j += 1;
            }
        }
        rearrange_lsp(n, lsp, 0.0001);
        let hidx = usize::from(bits.lpc_hist_idx[ch]);
        let hist = &mut self.lsp_hist[ch];
        for i in 0..n {
            let c3 = at(cb3_off + hidx * n + i);
            let tmp1 = (1.0 - f64::from(c3)) as f32;
            let tmp2 = hist[i] * c3;
            hist[i] = lsp[i];
            lsp[i] = lsp[i] * tmp1 + tmp2;
        }
        rearrange_lsp(n, lsp, 0.0001);
        rearrange_lsp(n, lsp, 0.000095);
        // ff_sort_nearly_sorted_floats
        for i in 0..n.saturating_sub(1) {
            let mut j = i as isize;
            while j >= 0 && lsp[j as usize] > lsp[j as usize + 1] {
                lsp.swap(j as usize, j as usize + 1);
                j -= 1;
            }
        }
    }

    fn dec_lpc_spectrum_inv(&self, lsp: &mut [f32; 20], ftype: usize, lpc: &mut [f32]) {
        let mtab = self.mode;
        let size = mtab.size / mtab.fmode[ftype].sub;
        for v in lsp.iter_mut().take(mtab.n_lsp) {
            *v = (2.0 * f64::from(*v).cos()) as f32;
        }
        match ftype {
            2 => self.eval_lpcenv_2parts(ftype, lsp, lpc, size, 8),
            1 => self.eval_lpcenv_2parts(ftype, lsp, lpc, size, 2),
            _ => self.eval_lpcenv(lsp, lpc),
        }
    }

    fn eval_lpcenv(&self, cos_vals: &[f32], lpc: &mut [f32]) {
        let mtab = self.mode;
        let size_s = mtab.size / mtab.fmode[Ft::Short as usize].sub;
        for i in 0..size_s / 2 {
            let cos_i = self.cos_tabs[0][i];
            lpc[i] = eval_lpc_spectrum(cos_vals, cos_i, mtab.n_lsp);
            lpc[size_s - i - 1] = eval_lpc_spectrum(cos_vals, -cos_i, mtab.n_lsp);
        }
    }

    fn eval_lpcenv_or_interp(&self, ftype: usize, out: &mut [f32], input: &[f32], size: usize, step: usize, part: bool) {
        let n_lsp = self.mode.n_lsp;
        let cos_tab = &self.cos_tabs[ftype];
        let get_cos = |idx: usize| -> f32 {
            if part { -cos_tab.get(size.wrapping_sub(idx).wrapping_sub(1)).copied().unwrap_or(0.0) } else { cos_tab.get(idx).copied().unwrap_or(0.0) }
        };
        let mut i = 0;
        while i < size {
            out[i] = eval_lpc_spectrum(input, get_cos(i), n_lsp);
            i += step;
        }
        let mut i = step;
        while i + 2 * step <= size {
            if f64::from(out[i + step] + out[i - step]) > 1.95 * f64::from(out[i]) || out[i + step] >= out[i - step] {
                interpolate(out, i - step + 1, out[i], out[i - step], step - 1);
            } else {
                out[i - step / 2] = eval_lpc_spectrum(input, get_cos(i - step / 2), n_lsp);
                interpolate(out, i - step + 1, out[i - step / 2], out[i - step], step / 2 - 1);
                interpolate(out, i - step / 2 + 1, out[i], out[i - step / 2], step / 2 - 1);
            }
            i += step;
        }
        interpolate(out, size - 2 * step + 1, out[size - step], out[size - 2 * step], step - 1);
    }

    fn eval_lpcenv_2parts(&self, ftype: usize, buf: &[f32], lpc: &mut [f32], size: usize, step: usize) {
        let h = size / 2;
        self.eval_lpcenv_or_interp(ftype, &mut lpc[..h], buf, h, step, false);
        self.eval_lpcenv_or_interp(ftype, &mut lpc[h..], buf, h, 2 * step, true);
        interpolate(lpc, h - step + 1, lpc[h], lpc[h - step], step);
        let v = lpc[size - 2 * step];
        for e in lpc.iter_mut().skip(size - 2 * step + 1).take(2 * step - 1) {
            *e = v;
        }
    }

    fn decode_ppc(&self, period_coef: u32, g_coef: u32, shape: &[f32], speech: &mut [f32]) {
        let mtab = self.mode;
        let isampf = self.sample_rate / 1000;
        let ibps = self.bit_rate / (1000 * self.channels as i32);
        let size = mtab.size as i32;
        let min_period = rounded_div(40 * 2 * size, isampf);
        let max_period = rounded_div(40 * 2 * size * 6, isampf);
        let period_range = max_period - min_period;
        let pgain_step = (25_000.0f64 / f64::from((1u32 << mtab.pgain_bit) - 1)) as f32;
        let ppc_gain = ((1.0f64 / 8192.0)
            * f64::from(mulawinv((f64::from(pgain_step) * f64::from(g_coef) + f64::from(pgain_step) / 2.0) as f32, 25_000.0, PGAIN_MU)))
            as f32;
        let period = min_period + rounded_div(period_coef as i32 * period_range, (1 << mtab.ppc_period_bit) - 1);
        let width =
            if isampf == 22 && ibps == 32 { rounded_div((period + 800) * mtab.peak_per2wid, 400 * size) } else { period * mtab.peak_per2wid / (400 * size) };
        add_peak(period, width, shape, ppc_gain, speech, mtab.ppc_shape_len as i32);
    }

    fn read_and_decode_spectrum(&mut self, bits: &FrameData) {
        let mtab = self.mode;
        let ftype = bits.ftype;
        let sub = mtab.fmode[ftype].sub;
        let size = mtab.size;
        let block_size = size / sub;
        let channels = self.channels;
        let mut out = std::mem::take(&mut self.spectrum);
        out.iter_mut().for_each(|v| *v = 0.0);
        let fm = &mtab.fmode[ftype];
        self.dequant(&bits.main_coeffs, &mut out, ftype, fm.cb0, fm.cb1, fm.cb_len_read);
        let mut gain = [0.0f32; 32];
        self.dec_gain(bits, ftype, &mut gain);
        let mut ppc_shape = vec![0.0f32; 60 * 2 * 4];
        if ftype == Ft::Long as usize {
            let cb_len_p = (self.n_div[3] + mtab.ppc_shape_len * channels - 1) / self.n_div[3];
            let cb1 = mtab.ppc_shape_cb.get(cb_len_p * PPC_SHAPE_CB_SIZE..).unwrap_or(&[]);
            self.dequant(&bits.ppc_coeffs, &mut ppc_shape, Ft::Ppc as usize, mtab.ppc_shape_cb, cb1, cb_len_p);
        }
        let mut tmp = std::mem::take(&mut self.tmp_buf);
        for i in 0..channels {
            let chunk = &mut out[size * i..size * (i + 1)];
            for j in 0..sub {
                self.dec_bark_env(bits, i, j, ftype, gain[sub * i + j], &mut tmp);
                for (c, t) in chunk[block_size * j..block_size * (j + 1)].iter_mut().zip(&tmp) {
                    *c *= *t;
                }
            }
            if ftype == Ft::Long as usize {
                let shape = ppc_shape.get(i * mtab.ppc_shape_len..).unwrap_or(&[]);
                self.decode_ppc(bits.p_coef[i], bits.g_coef[i], shape, chunk);
            }
            let mut lsp = [0.0f32; 20];
            self.decode_lsp(bits, i, &mut lsp);
            self.dec_lpc_spectrum_inv(&mut lsp, ftype, &mut tmp);
            for j in 0..sub {
                for (c, t) in chunk[block_size * j..block_size * (j + 1)].iter_mut().zip(&tmp) {
                    *c *= *t;
                }
            }
        }
        self.tmp_buf = tmp;
        self.spectrum = out;
    }

    // -----------------------------------------------------------------------------------------
    // synthesis

    /// The inverse MDCT of channel `ch`'s spectrum and the windowed overlap with the previous
    /// frame (`prev_base`: where the previous frame's channel data starts in `prev_frame`).
    fn imdct_and_window(&mut self, ftype: usize, wtype: usize, ch: usize, prev_base: usize) {
        let mtab = self.mode;
        let sub = mtab.fmode[ftype].sub;
        let bsize = mtab.size / sub;
        let size = mtab.size;
        let types_sizes = [
            mtab.size / mtab.fmode[Ft::Long as usize].sub,
            mtab.size / mtab.fmode[Ft::Medium as usize].sub,
            mtab.size / (mtab.fmode[Ft::Short as usize].sub * 2),
        ];
        let first_wsize = types_sizes[WTYPE_TO_WSIZE[wtype]];
        let mut buf1 = vec![0.0f32; size];
        // where the overlap is read from: the previous frame, then the block just made
        let mut from_prev = true;
        let mut prev_pos = prev_base + (size - bsize) / 2;
        let mut out2 = 2 * ch * size;
        for j in 0..sub {
            let sub_wtype = if j == 0 && wtype == 4 {
                4
            } else if j == sub - 1 && wtype == 7 {
                7
            } else if ftype == Ft::Medium as usize {
                8
            } else {
                wtype
            };
            let wsize = types_sizes[WTYPE_TO_WSIZE[sub_wtype]];
            let input = &self.spectrum[ch * size + bsize * j..ch * size + bsize * (j + 1)];
            imdct(input, &mut buf1[bsize * j..bsize * (j + 1)], self.scale[ftype]);
            let win = sine_window(wsize);
            let half = wsize / 2;
            let src0 = prev_pos + (bsize - wsize) / 2;
            for k in 0..half {
                let s0 = if from_prev { self.prev_frame.get(src0 + k).copied() } else { buf1.get(src0 + k).copied() }.unwrap_or(0.0);
                let s1 = buf1[bsize * j + half - 1 - k];
                let wi = win[k];
                let wj = win[2 * half - 1 - k];
                if let Some(e) = self.curr_frame.get_mut(out2 + k) {
                    *e = s0 * wj - s1 * wi;
                }
                if let Some(e) = self.curr_frame.get_mut(out2 + 2 * half - 1 - k) {
                    *e = s0 * wi + s1 * wj;
                }
            }
            out2 += wsize;
            for k in 0..bsize - half {
                let v = buf1[bsize * j + half + k];
                if let Some(e) = self.curr_frame.get_mut(out2 + k) {
                    *e = v;
                }
            }
            out2 += if ftype == Ft::Medium as usize { (bsize - wsize) / 2 } else { bsize - wsize };
            from_prev = false;
            prev_pos = bsize * j + bsize / 2;
        }
        self.last_block_pos[ch] = (size + first_wsize) / 2;
    }

    fn imdct_output(&mut self, bits: &FrameData, out: Option<&mut Vec<Vec<f32>>>) {
        let mtab = self.mode;
        let size = mtab.size;
        let prev_base = self.last_block_pos[0];
        for i in 0..self.channels {
            self.imdct_and_window(bits.ftype, bits.window_type, i, prev_base + 2 * i * size);
        }
        let Some(out) = out else { return };
        let size2 = self.last_block_pos[0];
        let size1 = size - size2;
        for c in 0..self.channels {
            let o = &mut out[c];
            for k in 0..size1 {
                o[k] = self.prev_frame.get(prev_base + 2 * c * size + k).copied().unwrap_or(0.0);
            }
            for k in 0..size2 {
                o[size1 + k] = self.curr_frame.get(2 * c * size + k).copied().unwrap_or(0.0);
            }
        }
        if self.channels == 2 {
            let (a, b) = out.split_at_mut(1);
            for (x, y) in a[0].iter_mut().zip(b[0].iter_mut()) {
                let t = *x - *y;
                *x += *y;
                *y = t;
            }
        }
    }
}

fn eval_lpc_spectrum(lsp: &[f32], cos_val: f32, order: usize) -> f32 {
    let mut p = 0.5f32;
    let mut q = 0.5f32;
    let two_cos_w = 2.0 * cos_val;
    let mut j = 0;
    while j + 1 < order {
        q *= lsp[j] - two_cos_w;
        p *= lsp[j + 1] - two_cos_w;
        q *= lsp[j + 2] - two_cos_w;
        p *= lsp[j + 3] - two_cos_w;
        j += 4;
    }
    p = p * (p * (2.0 - two_cos_w));
    q = q * (q * (2.0 + two_cos_w));
    (0.5 / f64::from(p + q)) as f32
}

/// Fill `out[at..at + size]` stepping from `v2` to `v1`.
fn interpolate(out: &mut [f32], at: usize, v1: f32, mut v2: f32, size: usize) {
    let step = (v1 - v2) / (size + 1) as f32;
    for i in 0..size {
        v2 += step;
        if let Some(e) = out.get_mut(at + i) {
            *e = v2;
        }
    }
}

fn rearrange_lsp(order: usize, lsp: &mut [f32], min_dist: f32) {
    let min_dist2 = (f64::from(min_dist) * 0.5) as f32;
    for i in 1..order {
        if lsp[i] - lsp[i - 1] < min_dist {
            let avg = (f64::from(lsp[i] + lsp[i - 1]) * 0.5) as f32;
            lsp[i - 1] = avg - min_dist2;
            lsp[i] = avg + min_dist2;
        }
    }
}

fn very_broken_op(a: i32, b: i32) -> i32 {
    let x = a * b + 200;
    if x % 400 != 0 || b % 5 != 0 {
        return x / 400;
    }
    let x = x / 400;
    let Some((size, tab)) = broken_table((b / 5) as usize) else { return x };
    let idx = size as i64 * i64::from(av_log2((2 * (x - 1) / size as i32).max(0) as u32)) + i64::from((x - 1) % size as i32);
    x - tab.get(idx.max(0) as usize).map_or(0, |&v| i32::from(v))
}

fn broken_table(i: usize) -> Option<(usize, &'static [u8])> {
    let flat = |s: usize, t: &'static [u8]| Some((s, t));
    match i {
        1 | 2 | 4 | 8 => flat(5, &t::TAB8),
        3 | 6 | 12 => flat(15, &t::TAB12),
        5 | 10 => flat(25, &t::TAB10),
        7 => flat(35, &t::TAB7),
        9 => flat(45, &t::TAB9),
        11 => flat(55, &t::TAB11),
        _ => None,
    }
}

fn add_peak(period: i32, width: i32, shape: &[f32], ppc_gain: f32, speech: &mut [f32], len: i32) {
    let sp = std::cell::Cell::new(0usize);
    let add = |speech: &mut [f32], idx: i64| {
        let v = shape.get(sp.get()).copied().unwrap_or(0.0);
        sp.set(sp.get() + 1);
        if let Some(e) = usize::try_from(idx).ok().and_then(|i| speech.get_mut(i)) {
            *e += ppc_gain * v;
        }
    };
    if width <= 0 {
        return;
    }
    for i in 0..width / 2 {
        add(speech, i64::from(i));
    }
    let mut i = 1;
    while i < rounded_div(len, width) {
        let center = very_broken_op(period, i);
        for j in -width / 2..(width + 1) / 2 {
            add(speech, i64::from(j + center));
        }
        i += 1;
    }
    let center = very_broken_op(period, i);
    let mut j = -width / 2;
    while j < (width + 1) / 2 && (sp.get() as i32) < len {
        add(speech, i64::from(j + center));
        j += 1;
    }
}

/// `ff_sine_windows[log2(n)]`: `sin((i + 0.5) / (2 n) · π)` for `i < n`.
fn sine_window(n: usize) -> Vec<f32> {
    (0..n).map(|i| ((i as f64 + 0.5) / (2.0 * n as f64) * std::f64::consts::PI).sin() as f32).collect()
}

/// ffmpeg's inverse MDCT (`AV_TX_FLOAT_MDCT`, inverse): `n` coefficients to `n` samples,
/// `out[i] = scale · DCT-IV(input)[n − 1 − i]`.
fn imdct(input: &[f32], out: &mut [f32], scale: f64) {
    let n = input.len();
    let c = dct4(&input.iter().map(|&v| f64::from(v)).collect::<Vec<_>>());
    for i in 0..n {
        out[i] = (c[n - 1 - i] * scale) as f32;
    }
}

/// `c[k] = Σ x[m] cos(π/n (m + ½)(k + ½))` through an `n/2`-point complex FFT.
fn dct4(x: &[f64]) -> Vec<f64> {
    let n = x.len();
    let h = n / 2;
    let pi = std::f64::consts::PI;
    let mut re = vec![0.0; h];
    let mut im = vec![0.0; h];
    for k in 0..h {
        let (a, b) = (x[2 * k], x[n - 1 - 2 * k]);
        let ang = -pi * (4 * k + 1) as f64 / (4 * n) as f64;
        let (s, c) = ang.sin_cos();
        re[k] = a * c - b * s;
        im[k] = a * s + b * c;
    }
    fft(&mut re, &mut im);
    let mut out = vec![0.0; n];
    for k in 0..h {
        let ang = -pi * k as f64 / n as f64;
        let (s, c) = ang.sin_cos();
        let yr = re[k] * c - im[k] * s;
        let yi = re[k] * s + im[k] * c;
        out[2 * k] = yr;
        out[n - 1 - 2 * k] = -yi;
    }
    out
}

/// In-place radix-2 FFT (`e^{-i...}`), length a power of two.
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (ang * k as f64).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * c - im[b] * s;
                let ti = re[b] * s + im[b] * c;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dct4_matches_the_definition() {
        let n = 16;
        let x: Vec<f64> = (0..n).map(|i| ((i * 7 + 3) % 11) as f64 - 5.0).collect();
        let c = dct4(&x);
        for k in 0..n {
            let d: f64 = (0..n).map(|m| x[m] * (std::f64::consts::PI / n as f64 * (m as f64 + 0.5) * (k as f64 + 0.5)).cos()).sum();
            assert!((c[k] - d).abs() < 1e-9, "{k}: {} vs {d}", c[k]);
        }
    }

    #[test]
    fn imdct_matches_ffmpegs_first_basis_function() {
        // av_tx inverse MDCT of the unit vector k = 0, n = 16: sin((2 i + 1) π / 64)
        let mut input = [0.0f32; 16];
        input[0] = 1.0;
        let mut out = [0.0f32; 16];
        imdct(&input, &mut out, 1.0);
        for (i, v) in out.iter().enumerate() {
            let want = (((2 * i + 1) as f64) * std::f64::consts::PI / 64.0).sin() as f32;
            assert!((v - want).abs() < 1e-6, "{i}");
        }
    }

    #[test]
    fn rejects_garbage() {
        assert!(!probe(b"RIFF"));
        assert_eq!(parse(b"TWIN97012000"), Err(Error::Invalid("short header")));
        assert!(parse(&[]).is_err());
    }

    #[test]
    fn permutation_tables_build_for_every_mode_and_layout() {
        for mode in 0..MODES.len() {
            for channels in 1..=2usize {
                let m = &MODES[mode];
                let rate = [8000, 11025, 11025, 16000, 22050, 22050, 22050, 44100, 44100][mode];
                let kbps = [8, 8, 10, 16, 20, 24, 32, 40, 48][mode];
                let s =
                    Stream { channels, sample_rate: rate, bit_rate: kbps * 1000 * channels as u32, frame_samples: m.size, frame_bits: 0, data_start: 0, mode };
                let d = Decoder::new(&s);
                for f in 0..4 {
                    assert!(d.n_div[f] > 0);
                }
                // every junk frame decodes without panicking
                let mut dec = Decoder::new(&s);
                let mut out = vec![Vec::new(); channels];
                for seed in 0..6u32 {
                    let pkt: Vec<u8> = (0..2000u32).map(|i| (i.wrapping_mul(2_654_435_761).wrapping_add(seed * 977) >> 13) as u8).collect();
                    let mut p = pkt.clone();
                    p[0] = 8;
                    dec.packet(&p, &mut out);
                }
            }
        }
    }
}
