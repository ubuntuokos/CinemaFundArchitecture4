// Port of FFmpeg n7.1.1 libavcodec/atrac9dec.c (Rostislav Pehlivanov, 2018),
// LGPL-2.1-or-later; see docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! **ATRAC9** (Sony, PlayStation Vita and PlayStation 4), as FFmpeg's `atrac9` decoder reads
//! it: superframes of 1, 2 or 4 frames of 64, 128 or 256 samples per channel, in mono, dual mono,
//! stereo, quad, 5.1 or 7.1 block layouts, configured by the 12-byte extradata of the WAV
//! (`.at9`) `fmt ` chunk. The output is planar `f32` in `[-1, 1]`.
//!
//! Every value from the stream is hostile: a damaged frame is an [`Error`], never a panic. In
//! strict mode ([`Decoder::set_strict`]) the decoder also refuses frames FFmpeg reads past their
//! end, so that what it accepts decodes exactly as FFmpeg's does.

use std::sync::OnceLock;

use crate::bits::Bits;
use crate::lfg::Lfg;
use crate::mdct::Imdct;
use crate::tables_atrac9 as t;
use crate::{Error, Result};

fn invalid<T>(what: &'static str) -> Result<T> {
    Err(Error::Invalid(what))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockType {
    /// One channel.
    Sce,
    /// Two coupled channels.
    Cpe,
    /// One low-frequency channel.
    Lfe,
}

/// `at9_block_layout`: the blocks of each configuration and their output planes.
struct Layout {
    channels: u16,
    blocks: &'static [(BlockType, [usize; 2])],
}

const LAYOUTS: [Layout; 6] = [
    Layout { channels: 1, blocks: &[(BlockType::Sce, [0, 0])] },
    Layout { channels: 2, blocks: &[(BlockType::Sce, [0, 0]), (BlockType::Sce, [1, 0])] },
    Layout { channels: 2, blocks: &[(BlockType::Cpe, [0, 1])] },
    Layout { channels: 6, blocks: &[(BlockType::Cpe, [0, 1]), (BlockType::Sce, [2, 0]), (BlockType::Lfe, [3, 0]), (BlockType::Cpe, [4, 5])] },
    Layout {
        channels: 8,
        blocks: &[(BlockType::Cpe, [0, 1]), (BlockType::Sce, [2, 0]), (BlockType::Lfe, [3, 0]), (BlockType::Cpe, [4, 5]), (BlockType::Cpe, [6, 7])],
    },
    Layout { channels: 4, blocks: &[(BlockType::Cpe, [0, 1]), (BlockType::Cpe, [2, 3])] },
];

/// A single-level prefix code table: `(symbol, length)` for every `bits`-bit prefix.
struct Vlc {
    bits: u32,
    table: Vec<(i16, u8)>,
}

impl Vlc {
    /// `ff_vlc_init_from_lengths`: `(symbol, length)` pairs in code order, symbols shifted by
    /// `offset`.
    fn new(pairs: &[u8], offset: i16) -> Self {
        let bits = pairs.chunks(2).filter_map(|p| p.get(1)).map(|&l| u32::from(l)).max().unwrap_or(1).clamp(1, 16);
        let mut table = vec![(-1i16, 0u8); 1 << bits];
        let mut code: u64 = 0;
        for p in pairs.chunks(2) {
            let (Some(&sym), Some(&len)) = (p.first(), p.get(1)) else { continue };
            let l = u32::from(len).clamp(1, bits);
            let start = ((code >> (32 - l)) as usize) << (bits - l);
            for e in table.iter_mut().skip(start).take(1 << (bits - l)) {
                *e = (i16::from(sym) + offset, l as u8);
            }
            code += 1u64 << (32 - l);
        }
        Self { bits, table }
    }
}

/// `HuffmanCodebook`.
#[derive(Clone, Copy, Default)]
struct Book {
    value_cnt: usize,
    value_cnt_pow: u32,
    value_bits: u32,
}

/// Tables shared by every stream, built once.
struct Tables {
    /// Unsigned scalefactor codes by length 1..=6.
    sf_unsigned: Vec<Option<Vlc>>,
    /// Signed scalefactor codes by length 2..=5.
    sf_signed: Vec<Option<Vlc>>,
    /// Coefficient codes `[codebook set][precision][codebook index]`.
    coeff: Vec<Option<(Vlc, Book)>>,
    /// `alloc_curve`.
    alloc_curve: [[u8; 48]; 48],
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(Tables::new)
}

fn coeff_index(set: usize, prec: usize, cbi: usize) -> usize {
    set * 32 + prec * 4 + cbi
}

impl Tables {
    fn new() -> Self {
        let mut pos = 0usize;
        let mut take = |src: &'static [u8], n: usize| {
            let s = src.get(pos * 2..(pos + n) * 2).unwrap_or(&[]);
            pos += n;
            s
        };
        let mut sf_unsigned: Vec<Option<Vlc>> = (0..7).map(|_| None).collect();
        for (i, slot) in sf_unsigned.iter_mut().enumerate().skip(1) {
            let size = usize::from(t::HUFFMAN_SF_UNSIGNED.get(i).map_or(0, |h| h[0]));
            *slot = Some(Vlc::new(take(&t::SFB_A_TAB, size), 0));
        }
        pos = 0;
        let mut take = |src: &'static [u8], n: usize| {
            let s = src.get(pos * 2..(pos + n) * 2).unwrap_or(&[]);
            pos += n;
            s
        };
        let mut sf_signed: Vec<Option<Vlc>> = (0..6).map(|_| None).collect();
        for (i, slot) in sf_signed.iter_mut().enumerate().skip(2) {
            let size = usize::from(t::HUFFMAN_SF_SIGNED.get(i).map_or(0, |h| h[0]));
            // symbols -16..=15 are stored offset by 16
            *slot = Some(Vlc::new(take(&t::SFB_B_TAB, size), -16));
        }
        pos = 0;
        let mut coeff: Vec<Option<(Vlc, Book)>> = (0..64).map(|_| None).collect();
        for set in 0..2 {
            for prec in 2..8 {
                for cbi in set..4 {
                    let k = coeff_index(set, prec, cbi);
                    let h = t::HUFFMAN_COEFFS.get(k).copied().unwrap_or_default();
                    let size = usize::from(h[0]);
                    let s = t::COEFFS_TAB.get(pos * 2..(pos + size) * 2).unwrap_or(&[]);
                    pos += size;
                    let book = Book { value_cnt: usize::from(h[1]), value_cnt_pow: u32::from(h[2]), value_bits: u32::from(h[3]) };
                    if let Some(slot) = coeff.get_mut(k) {
                        *slot = Some((Vlc::new(s, 0), book));
                    }
                }
            }
        }
        let mut alloc_curve = [[0u8; 48]; 48];
        let n = t::TAB_B_DIST.len();
        for i in 1..=n {
            for j in 0..i {
                alloc_curve[i - 1][j] = t::TAB_B_DIST.get(j * n / i).copied().unwrap_or(0);
            }
        }
        Self { sf_unsigned, sf_signed, coeff, alloc_curve }
    }
}

/// The bit reader of one packet, noting reads past its end (for [`Decoder::set_strict`]).
struct Rd<'a> {
    b: Bits<'a>,
    size_bits: usize,
}

impl Rd<'_> {
    fn get(&mut self, n: u32) -> i32 {
        self.b.get(n.min(25)) as i32
    }

    fn bit(&mut self) -> i32 {
        self.get(1)
    }

    fn vlc(&mut self, v: &Vlc) -> i32 {
        let (sym, len) = v.table.get(self.b.show(v.bits) as usize).copied().unwrap_or((-1, 0));
        self.b.skip(u32::from(len));
        i32::from(sym)
    }

    fn align(&mut self) {
        let r = self.b.count() % 8;
        if r != 0 {
            self.b.skip(8 - r as u32);
        }
    }

    fn overread(&self) -> bool {
        self.b.count() > self.size_bits
    }
}

fn sign_extend(v: i32, bits: u32) -> i32 {
    if bits == 0 || bits >= 32 {
        return v;
    }
    let s = 32 - bits;
    (v << s) >> s
}

fn coeff_idx(i: usize) -> usize {
    usize::from(t::Q_UNIT_TO_COEFF_IDX.get(i).copied().unwrap_or(256)).min(256)
}

fn at<T: Copy + Default>(a: &[T], i: usize) -> T {
    a.get(i).copied().unwrap_or_default()
}

fn atx<T: Copy + Default>(a: &[T], i: i32) -> T {
    usize::try_from(i).ok().and_then(|i| a.get(i)).copied().unwrap_or_default()
}

fn put<T>(a: &mut [T], i: usize, v: T) {
    if let Some(x) = a.get_mut(i) {
        *x = v;
    }
}

#[derive(Clone)]
struct Chan {
    band_ext: i32,
    q_unit_cnt: usize,
    band_ext_data: [i32; 4],
    scalefactors: [i32; 31],
    scalefactors_prev: [i32; 31],
    precision_coarse: [i32; 30],
    precision_fine: [i32; 30],
    precision_mask: [i32; 30],
    codebookset: [i32; 30],
    q_coeffs_coarse: [i32; 256],
    q_coeffs_fine: [i32; 256],
    coeffs: [f32; 256],
    prev_win: [f32; 128],
}

impl Default for Chan {
    fn default() -> Self {
        Self {
            band_ext: 0,
            q_unit_cnt: 0,
            band_ext_data: [0; 4],
            scalefactors: [0; 31],
            scalefactors_prev: [0; 31],
            precision_coarse: [0; 30],
            precision_fine: [0; 30],
            precision_mask: [0; 30],
            codebookset: [0; 30],
            q_coeffs_coarse: [0; 256],
            q_coeffs_fine: [0; 256],
            coeffs: [0.0; 256],
            prev_win: [0.0; 128],
        }
    }
}

#[derive(Clone, Default)]
struct Block {
    ch: [Chan; 2],
    band_count: usize,
    q_unit_cnt: usize,
    q_unit_cnt_prev: usize,
    stereo_q_unit: usize,
    has_band_ext: bool,
    has_band_ext_data: bool,
    band_ext_q_unit: usize,
    grad_mode: i32,
    grad_boundary: usize,
    gradient: [i32; 31],
    cpe_base_channel: usize,
    is_signs: [i32; 30],
    reuseable: bool,
}

/// An ATRAC9 decoder for one stream.
#[derive(Clone)]
pub struct Decoder {
    layout: usize,
    samplerate_idx: usize,
    frame_log2: u32,
    avg_frame_size: usize,
    frame_count: usize,
    block_align: usize,
    blocks: Vec<Block>,
    lfg: Lfg,
    imdct: Imdct,
    imdct_win: Vec<f32>,
    strict: bool,
    bits_used: usize,
    blocks_decoded: usize,
}

impl Decoder {
    /// A decoder for the 12-byte extradata of a WAV `fmt ` chunk (a little-endian version, then
    /// the configuration: `0xFE`, sample rate index, block layout, frame size, superframe size)
    /// and packets of `block_align` bytes.
    pub fn new(extradata: &[u8], block_align: usize) -> Result<Self> {
        if block_align == 0 || block_align > 1 << 20 {
            return Err(Error::Unsupported("ATRAC9 block size"));
        }
        if extradata.len() != 12 {
            return invalid("ATRAC9 extradata length");
        }
        let version = u32::from_le_bytes([extradata[0], extradata[1], extradata[2], extradata[3]]);
        if version > 2 {
            return Err(Error::Unsupported("ATRAC9 version"));
        }
        let cfg = extradata.get(4..).unwrap_or(&[]);
        let mut b = Bits::new(cfg, cfg.len() * 8);
        if b.get(8) != 0xFE {
            return invalid("ATRAC9 configuration magic");
        }
        let samplerate_idx = b.get(4) as usize;
        let layout = b.get(3) as usize;
        if layout > 5 {
            return Err(Error::Unsupported("ATRAC9 block layout"));
        }
        if b.get1() {
            return invalid("ATRAC9 configuration bit");
        }
        let avg_frame_size = b.get(11) as usize + 1;
        let superframe_idx = b.get(2);
        if superframe_idx & 1 != 0 {
            return invalid("ATRAC9 superframe index");
        }
        let frame_count = 1usize << superframe_idx;
        let frame_log2 = u32::from(at(&t::TAB_SRI_FRAME_LOG2, samplerate_idx));
        let len = 1usize << frame_log2;
        // the window: s_c / (s_c² + e_c²) of the raised sine, in f32 as FFmpeg computes it
        let imdct_win = (0..len)
            .map(|i| {
                let sidx = (i as f32 + 0.5) / len as f32;
                let eidx = (len as f32 - i as f32 - 0.5) / len as f32;
                let s_c = ((f64::from(sidx) * std::f64::consts::PI - std::f64::consts::FRAC_PI_2) as f32).sin() * 0.5 + 0.5;
                let e_c = ((f64::from(eidx) * std::f64::consts::PI - std::f64::consts::FRAC_PI_2) as f32).sin() * 0.5 + 0.5;
                s_c / ((s_c * s_c) + (e_c * e_c))
            })
            .collect();
        let count = LAYOUTS.get(layout).map_or(0, |l| l.blocks.len());
        Ok(Self {
            layout,
            samplerate_idx,
            frame_log2,
            avg_frame_size,
            frame_count,
            block_align,
            blocks: vec![Block::default(); count],
            lfg: Lfg::new(t::LFG_SEED_FBADF00D),
            // `av_tx` with scale 1/32768 is this transform with scale -1/32768
            imdct: Imdct::new(len, -1.0 / 32768.0),
            imdct_win,
            strict: false,
            bits_used: 0,
            blocks_decoded: 0,
        })
    }

    /// Output channels.
    pub fn channels(&self) -> usize {
        LAYOUTS.get(self.layout).map_or(1, |l| usize::from(l.channels))
    }

    /// The sample rate the configuration names.
    pub fn sample_rate(&self) -> u32 {
        at(&t::TAB_SAMPLERATES, self.samplerate_idx)
    }

    /// Bytes of a packet (a superframe).
    pub fn block_align(&self) -> usize {
        self.block_align
    }

    /// Samples per channel of a whole packet.
    pub fn samples_per_channel(&self) -> usize {
        self.frame_count << self.frame_log2
    }

    /// Samples per channel of the first part of a packet of `len` bytes (a short one holds
    /// fewer frames; see [`Decoder::decode`] for what follows them).
    pub fn samples_in(&self, len: usize) -> usize {
        (len / self.avg_frame_size).min(self.frame_count) << self.frame_log2
    }

    /// Samples per channel that [`Decoder::decode`] gives for `packet` from this state (a short
    /// packet's count depends on its content), 0 for a damaged one.
    pub fn samples_in_packet(&self, packet: &[u8]) -> usize {
        let mut d = self.clone();
        d.decode(packet).ok().and_then(|o| o.first().map(Vec::len)).unwrap_or(0)
    }

    /// Refuse frames whose decoding FFmpeg leaves undefined (reads past the packet's end).
    pub fn set_strict(&mut self, on: bool) {
        self.strict = on;
    }

    /// Bits the last [`Decoder::decode`] read, to the end of the last block it decoded whole
    /// (all of them when it succeeded).
    pub fn bits_used(&self) -> usize {
        self.bits_used
    }

    /// Blocks (of all frames) the last [`Decoder::decode`] decoded whole.
    pub fn blocks_decoded(&self) -> usize {
        self.blocks_decoded
    }

    /// Decodes one packet into planar samples ([`Decoder::samples_per_channel`] per channel).
    ///
    /// A short packet (less than a superframe) decodes as FFmpeg 9 does: the whole frames it
    /// has room for, then the same again from the first byte after them, until fewer than a
    /// frame's bytes are left.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<Vec<f32>>> {
        let mut out = self.decode_part(packet)?;
        let full = out.first().map_or(0, Vec::len) >= self.samples_per_channel();
        if full {
            return Ok(out);
        }
        let mut rest = packet;
        let (blocks, mut bits) = (self.blocks_decoded, self.bits_used);
        loop {
            // FFmpeg's decoder reports the bytes it read; the rest is decoded again
            let used = self.bits_used / 8;
            rest = rest.get(used..).unwrap_or(&[]);
            if used == 0 || rest.len() < self.avg_frame_size {
                break;
            }
            let start = packet.len() - rest.len();
            match self.decode_part(rest) {
                Ok(more) => {
                    for (o, m) in out.iter_mut().zip(more) {
                        o.extend(m);
                    }
                    bits = start * 8 + self.bits_used;
                }
                // FFmpeg drops the rest of a packet after an error
                Err(e) if self.strict => return Err(e),
                Err(_) => break,
            }
        }
        self.blocks_decoded = blocks;
        self.bits_used = bits;
        Ok(out)
    }

    fn decode_part(&mut self, packet: &[u8]) -> Result<Vec<Vec<f32>>> {
        let frames = (packet.len() / self.avg_frame_size).min(self.frame_count);
        if frames == 0 {
            return invalid("ATRAC9 packet too short");
        }
        let layout = LAYOUTS.get(self.layout).ok_or(Error::Unsupported("ATRAC9 block layout"))?;
        let wsize = 1usize << self.frame_log2;
        let mut out = vec![vec![0f32; wsize * frames]; usize::from(layout.channels)];
        let mut rd = Rd { b: Bits::new(packet, packet.len() * 8), size_bits: packet.len() * 8 };
        let mut temp = vec![0f32; 2 * wsize];
        self.bits_used = 0;
        self.blocks_decoded = 0;
        for f in 0..frames {
            for (j, &(ty, planes)) in layout.blocks.iter().enumerate() {
                self.decode_block(&mut rd, j, ty)?;
                let stereo = ty == BlockType::Cpe;
                let Some(b) = self.blocks.get_mut(j) else { continue };
                for (i, &plane) in planes.iter().enumerate().take(if stereo { 2 } else { 1 }) {
                    let c = &mut b.ch[i];
                    self.imdct.inverse(&c.coeffs[..wsize], &mut temp);
                    // the half inverse MDCT: the middle `wsize` samples of the full one
                    let half = &temp[wsize / 2..wsize / 2 + wsize];
                    let Some(dst) = out.get_mut(plane).and_then(|p| p.get_mut(f * wsize..(f + 1) * wsize)) else { continue };
                    fmul_window(dst, &c.prev_win[..wsize / 2], half, &self.imdct_win, wsize / 2);
                    c.prev_win[..wsize / 2].copy_from_slice(&half[wsize / 2..]);
                }
                rd.align();
                if self.strict && rd.overread() {
                    return invalid("ATRAC9 packet overread");
                }
                self.bits_used = rd.b.count();
                self.blocks_decoded += 1;
            }
        }
        Ok(out)
    }

    fn decode_block(&mut self, rd: &mut Rd<'_>, j: usize, ty: BlockType) -> Result<()> {
        let tb = tables();
        let samplerate_idx = self.samplerate_idx;
        let Some(b) = self.blocks.get_mut(j) else { return invalid("ATRAC9 block") };
        let first_in_pkt = rd.bit() == 0;
        let reuse_params = rd.bit() != 0;
        let stereo = ty == BlockType::Cpe;

        if ty == BlockType::Lfe {
            let c = &mut b.ch[0];
            let precision = if reuse_params { 8 } else { 4 };
            b.q_unit_cnt = 2;
            c.q_unit_cnt = 2;
            c.scalefactors = [0; 31];
            c.q_coeffs_fine = [0; 256];
            c.q_coeffs_coarse = [0; 256];
            for i in 0..2 {
                c.scalefactors[i] = rd.get(5);
                c.precision_coarse[i] = precision;
                c.precision_fine[i] = 0;
            }
            for i in 0..2 {
                for k in coeff_idx(i)..coeff_idx(i + 1) {
                    c.q_coeffs_coarse[k] = rd.get(precision as u32 + 1);
                }
            }
            dequantize(c);
            apply_scalefactors(b, false);
            return Ok(());
        }

        if first_in_pkt && reuse_params {
            return invalid("ATRAC9 block flags");
        }

        if !reuse_params {
            let min_band_count = if samplerate_idx > 7 { 1 } else { 3 };
            b.reuseable = false;
            b.band_count = rd.get(4) as usize + min_band_count;
            b.q_unit_cnt = usize::from(at(&t::TAB_BAND_Q_UNIT_MAP, b.band_count));
            b.band_ext_q_unit = b.q_unit_cnt;
            b.stereo_q_unit = b.q_unit_cnt;
            if b.band_count > usize::from(at(&t::TAB_SRI_MAX_BANDS, samplerate_idx)) {
                return invalid("ATRAC9 band count");
            }
            if stereo {
                let stereo_band = rd.get(4) as usize + min_band_count;
                if stereo_band > b.band_count {
                    return invalid("ATRAC9 stereo band");
                }
                b.stereo_q_unit = usize::from(at(&t::TAB_BAND_Q_UNIT_MAP, stereo_band));
            }
            b.has_band_ext = rd.bit() != 0;
            if b.has_band_ext {
                let ext_band = rd.get(4) as usize + min_band_count;
                if ext_band < b.band_count {
                    return invalid("ATRAC9 extension band");
                }
                b.band_ext_q_unit = usize::from(at(&t::TAB_BAND_Q_UNIT_MAP, ext_band));
            }
            b.reuseable = true;
        }
        if !b.reuseable {
            return invalid("ATRAC9 block reused");
        }

        parse_gradient(tb, b, rd)?;

        b.cpe_base_channel = 0;
        if stereo {
            b.cpe_base_channel = rd.bit() as usize;
            if rd.bit() != 0 {
                for i in b.stereo_q_unit..b.q_unit_cnt {
                    let s = 1 - 2 * rd.bit();
                    put(&mut b.is_signs, i, s);
                }
            } else {
                b.is_signs = [1; 30];
            }
        }

        parse_band_ext(b, rd, stereo)?;

        for i in 0..=usize::from(stereo) {
            let q = if i == b.cpe_base_channel { b.q_unit_cnt } else { b.stereo_q_unit };
            b.ch[i].q_unit_cnt = q;
            read_scalefactors(tb, b, i, rd, first_in_pkt)?;
            let (grad_mode, grad_boundary, q_unit_cnt, gradient) = (b.grad_mode, b.grad_boundary, b.q_unit_cnt, b.gradient);
            let c = &mut b.ch[i];
            calc_precision(c, grad_mode, grad_boundary, q_unit_cnt, &gradient);
            calc_codebook_idx(c, samplerate_idx);
            read_coeffs_coarse(tb, c, rd, samplerate_idx);
            read_coeffs_fine(c, rd);
            dequantize(c);
        }

        b.q_unit_cnt_prev = if b.has_band_ext { b.band_ext_q_unit } else { b.q_unit_cnt };

        apply_intensity_stereo(b, stereo);
        apply_scalefactors(b, stereo);

        if b.has_band_ext && b.has_band_ext_data {
            apply_band_extension(b, &mut self.lfg, stereo);
        }
        Ok(())
    }
}

/// `vector_fmul_window`: `2 · len` samples overlapping `src0` and `src1` under `win`.
fn fmul_window(dst: &mut [f32], src0: &[f32], src1: &[f32], win: &[f32], len: usize) {
    for k in 0..len {
        // i = -len + k (dst[len + i] = dst[k]), j = len - 1 - k
        let j = len - 1 - k;
        let s0 = at(src0, k);
        let s1 = at(src1, j);
        let wi = at(win, k);
        let wj = at(win, len + j);
        put(dst, k, s0 * wj - s1 * wi);
        put(dst, len + j, s0 * wi + s1 * wj);
    }
}

fn parse_gradient(tb: &Tables, b: &mut Block, rd: &mut Rd<'_>) -> Result<()> {
    let mut grad_range = [0i32; 2];
    let mut grad_value = [0i32; 2];
    b.grad_mode = rd.get(2);
    if b.grad_mode != 0 {
        grad_range = [rd.get(5), 31];
        grad_value = [rd.get(5), 31];
    } else {
        grad_range[0] = rd.get(6);
        grad_range[1] = rd.get(6) + 1;
        grad_value[0] = rd.get(5);
        grad_value[1] = rd.get(5);
    }
    b.grad_boundary = rd.get(4) as usize;
    if grad_range[0] >= grad_range[1] || grad_range[1] > 31 {
        return invalid("ATRAC9 gradient range");
    }
    if b.grad_boundary > b.q_unit_cnt {
        return invalid("ATRAC9 gradient boundary");
    }
    let values = grad_value[1] - grad_value[0];
    let sign = 1 - 2 * i32::from(values < 0);
    let base = grad_value[0] + sign;
    let scale = (values.abs() - 1) as f32 / 31.0;
    let curve = tb.alloc_curve.get((grad_range[1] - grad_range[0] - 1) as usize).copied().unwrap_or([0; 48]);
    for i in 0..=b.q_unit_cnt {
        let v = grad_value[usize::from(i as i32 >= grad_range[0])];
        put(&mut b.gradient, i, v);
    }
    for i in grad_range[0]..grad_range[1] {
        let v = base + sign * ((scale * f32::from(atx(&curve, i - grad_range[0]))) as i32);
        put(&mut b.gradient, i as usize, v);
    }
    Ok(())
}

fn calc_precision(c: &mut Chan, grad_mode: i32, grad_boundary: usize, q_unit_cnt: usize, gradient: &[i32; 31]) {
    c.precision_mask = [0; 30];
    for i in 1..q_unit_cnt {
        let (cur, prev) = (at(&c.scalefactors, i), at(&c.scalefactors, i - 1));
        let delta = (cur - prev).abs() - 1;
        if delta > 0 {
            let neg = usize::from(prev > cur);
            let k = i - neg;
            let v = at(&c.precision_mask, k) + delta.min(5);
            put(&mut c.precision_mask, k, v);
        }
    }
    for i in 0..q_unit_cnt.min(30) {
        let mut p;
        if grad_mode != 0 {
            p = c.scalefactors[i] + c.precision_mask[i] - gradient[i];
            if p >= 0 {
                p = match grad_mode {
                    1 => p >> 1,
                    2 => (3 * p) >> 3,
                    3 => p >> 2,
                    _ => p,
                };
            }
        } else {
            p = c.scalefactors[i] - gradient[i];
        }
        c.precision_coarse[i] = p.max(1);
    }
    for i in 0..grad_boundary.min(30) {
        c.precision_coarse[i] += 1;
    }
    for i in 0..q_unit_cnt.min(30) {
        c.precision_fine[i] = 0;
        if c.precision_coarse[i] > 15 {
            c.precision_fine[i] = c.precision_coarse[i].min(30) - 15;
            c.precision_coarse[i] = 15;
        }
    }
}

fn parse_band_ext(b: &mut Block, rd: &mut Rd<'_>, stereo: bool) -> Result<()> {
    let mut ext_band = 0usize;
    if b.has_band_ext {
        if !(13..=20).contains(&b.q_unit_cnt) {
            return invalid("ATRAC9 band extension units");
        }
        ext_band = usize::from(at(&t::TAB_BAND_EXT_GROUP, (b.q_unit_cnt - 13) * 3 + 2));
        if stereo {
            let v = rd.get(2);
            b.ch[1].band_ext = if ext_band > 2 { v } else { 4 };
        } else {
            rd.get(1);
        }
    }
    b.has_band_ext_data = rd.bit() != 0;
    if !b.has_band_ext_data {
        return Ok(());
    }
    if !b.has_band_ext {
        rd.get(2);
        let n = rd.get(5);
        rd.b.skip(n as u32);
        return Ok(());
    }
    let v = rd.get(2);
    b.ch[0].band_ext = if ext_band > 2 { v } else { 4 };
    let count = |c: &Chan| usize::from(at(&t::TAB_BAND_EXT_CNT, c.band_ext as usize * 6 + ext_band));
    let length = |c: &Chan, j: usize| u32::from(at(&t::TAB_BAND_EXT_LENGTHS, (c.band_ext as usize * 6 + ext_band) * 4 + j));
    if rd.get(5) == 0 {
        for c in b.ch.iter_mut().take(1 + usize::from(stereo)) {
            for j in 0..count(c).min(4) {
                let len = length(c, j);
                c.band_ext_data[j] = clip_uintp2(c.band_ext_data[j], len);
            }
        }
        return Ok(());
    }
    for c in b.ch.iter_mut().take(1 + usize::from(stereo)) {
        for j in 0..count(c).min(4) {
            let len = length(c, j);
            c.band_ext_data[j] = rd.get(len);
        }
    }
    Ok(())
}

/// `av_clip_uintp2_c`.
fn clip_uintp2(a: i32, p: u32) -> i32 {
    let m = ((1i64 << p.min(31)) - 1) as i32;
    if a & !m != 0 { (!a >> 31) & m } else { a }
}

fn read_scalefactors(tb: &Tables, b: &mut Block, channel_idx: usize, rd: &mut Rd<'_>, first_in_pkt: bool) -> Result<()> {
    const MODE_MAP: [[u8; 4]; 2] = [[0, 1, 2, 3], [0, 2, 3, 4]];
    let mode = MODE_MAP[channel_idx.min(1)][rd.get(2) as usize & 3];
    let n = b.band_ext_q_unit.min(31);
    let ch0 = b.ch[0].scalefactors;
    let (q_unit_cnt_prev, band_ext_q_unit) = (b.q_unit_cnt_prev, b.band_ext_q_unit);
    let c = &mut b.ch[channel_idx.min(1)];
    c.scalefactors = [0; 31];
    if first_in_pkt && (mode == 4 || (mode == 3 && channel_idx == 0)) {
        return invalid("ATRAC9 scalefactor coding mode");
    }
    match mode {
        0 => {
            let w = rd.get(3) as usize;
            let base = rd.get(5);
            let len = rd.get(2) as u32 + 3;
            let tab = tb.sf_unsigned.get(len as usize).and_then(Option::as_ref);
            c.scalefactors[0] = rd.get(len);
            for i in 1..n {
                let d = tab.map_or(0, |v| rd.vlc(v));
                let val = c.scalefactors[i - 1] + d;
                c.scalefactors[i] = val & ((1 << len) - 1);
            }
            for i in 0..n {
                c.scalefactors[i] += base - i32::from(at(&t::TAB_SF_WEIGHTS, w * 32 + i));
            }
        }
        1 => {
            let len = rd.get(2) as u32 + 2;
            let base = if len < 5 { rd.get(5) } else { 0 };
            for i in 0..n {
                c.scalefactors[i] = base + rd.get(len);
            }
        }
        2 | 4 => {
            let (baseline, baseline_len) = if mode == 4 {
                (c.scalefactors_prev, q_unit_cnt_prev)
            } else if channel_idx != 0 {
                (ch0, band_ext_q_unit)
            } else {
                (c.scalefactors_prev, q_unit_cnt_prev)
            };
            let len = rd.get(2) as usize + 2;
            let unit_cnt = n.min(baseline_len);
            let tab = tb.sf_signed.get(len).and_then(Option::as_ref);
            for i in 0..unit_cnt {
                let dist = tab.map_or(0, |v| rd.vlc(v));
                c.scalefactors[i] = baseline[i] + dist;
            }
            for i in unit_cnt..n {
                c.scalefactors[i] = rd.get(5);
            }
        }
        _ => {
            let (baseline, baseline_len) = if channel_idx != 0 { (ch0, band_ext_q_unit) } else { (c.scalefactors_prev, q_unit_cnt_prev) };
            let base = rd.get(5) - 16;
            let len = rd.get(2) as u32 + 1;
            let unit_cnt = n.min(baseline_len);
            let tab = tb.sf_unsigned.get(len as usize).and_then(Option::as_ref);
            c.scalefactors[0] = rd.get(len);
            for i in 1..unit_cnt {
                let d = tab.map_or(0, |v| rd.vlc(v));
                let val = c.scalefactors[i - 1] + d;
                c.scalefactors[i] = val & ((1 << len) - 1);
            }
            for i in 0..unit_cnt {
                c.scalefactors[i] += base + baseline[i];
            }
            for i in unit_cnt..n {
                c.scalefactors[i] = rd.get(5);
            }
        }
    }
    if c.scalefactors.iter().take(n).any(|&s| !(0..=31).contains(&s)) {
        return invalid("ATRAC9 scalefactor");
    }
    c.scalefactors_prev = c.scalefactors;
    Ok(())
}

fn calc_codebook_idx(c: &mut Chan, samplerate_idx: usize) {
    c.codebookset = [0; 30];
    let q = c.q_unit_cnt.min(30);
    if q <= 1 || samplerate_idx > 7 {
        return;
    }
    let last_sf = c.scalefactors[q];
    c.scalefactors[q] = c.scalefactors[q - 1];
    let mut avg = 0;
    if q > 12 {
        avg = (c.scalefactors[..12].iter().sum::<i32>() + 6) / 12;
    }
    for i in 8..q {
        let (prev, cur, next) = (c.scalefactors[i - 1], c.scalefactors[i], c.scalefactors[i + 1]);
        let min = prev.min(next);
        if cur - min >= 3 || 2 * cur - prev - next >= 3 {
            c.codebookset[i] = 1;
        }
    }
    for i in 12..q {
        let cur = c.scalefactors[i];
        let cnd = i32::from(at(&t::Q_UNIT_TO_COEFF_CNT, i) == 16);
        let min = c.scalefactors[i + 1].min(c.scalefactors[i - 1]);
        if c.codebookset[i] != 0 {
            continue;
        }
        c.codebookset[i] = i32::from(cur - min >= 2 && cur >= avg - cnd);
    }
    c.scalefactors[q] = last_sf;
}

fn read_coeffs_coarse(tb: &Tables, c: &mut Chan, rd: &mut Rd<'_>, samplerate_idx: usize) {
    let max_prec = if samplerate_idx > 7 { 1 } else { 7 };
    c.q_coeffs_coarse = [0; 256];
    for i in 0..c.q_unit_cnt.min(30) {
        let mut k = coeff_idx(i);
        let bands = usize::from(at(&t::Q_UNIT_TO_COEFF_CNT, i));
        let prec = c.precision_coarse[i] + 1;
        if prec <= max_prec {
            let cb = c.codebookset[i] as usize;
            let cbi = usize::from(at(&t::Q_UNIT_TO_CODEBOOKIDX, i));
            let Some(Some((tab, huff))) = tb.coeff.get(coeff_index(cb, prec as usize, cbi)) else { continue };
            let groups = bands >> huff.value_cnt_pow;
            for _ in 0..groups {
                let mut val = rd.vlc(tab) as u16;
                for _ in 0..huff.value_cnt {
                    put(&mut c.q_coeffs_coarse, k, sign_extend(i32::from(val), huff.value_bits));
                    val >>= huff.value_bits;
                    k += 1;
                }
            }
        } else {
            let p = prec.clamp(0, 31) as u32;
            for _ in 0..bands {
                let v = sign_extend(rd.get(p), p);
                put(&mut c.q_coeffs_coarse, k, v);
                k += 1;
            }
        }
    }
}

fn read_coeffs_fine(c: &mut Chan, rd: &mut Rd<'_>) {
    c.q_coeffs_fine = [0; 256];
    for i in 0..c.q_unit_cnt.min(30) {
        if c.precision_fine[i] <= 0 {
            continue;
        }
        let len = (c.precision_fine[i] + 1) as u32;
        for j in coeff_idx(i)..coeff_idx(i + 1) {
            c.q_coeffs_fine[j] = sign_extend(rd.get(len), len);
        }
    }
}

fn dequantize(c: &mut Chan) {
    c.coeffs = [0.0; 256];
    for i in 0..c.q_unit_cnt.min(30) {
        let coarse = atx(&t::QUANT_STEP_COARSE, c.precision_coarse[i]);
        let fine = atx(&t::QUANT_STEP_FINE, c.precision_fine[i]);
        for j in coeff_idx(i)..coeff_idx(i + 1) {
            let vc = c.q_coeffs_coarse[j] as f32 * coarse;
            let vf = c.q_coeffs_fine[j] as f32 * fine;
            c.coeffs[j] = vc + vf;
        }
    }
}

fn apply_intensity_stereo(b: &mut Block, stereo: bool) {
    if !stereo || b.q_unit_cnt <= b.stereo_q_unit {
        return;
    }
    let base = b.cpe_base_channel.min(1);
    let src = b.ch[base].coeffs;
    let dst = &mut b.ch[base ^ 1].coeffs;
    for i in b.stereo_q_unit..b.q_unit_cnt.min(30) {
        let sign = b.is_signs[i] as f32;
        for j in coeff_idx(i)..coeff_idx(i + 1) {
            dst[j] = sign * src[j];
        }
    }
}

fn apply_scalefactors(b: &mut Block, stereo: bool) {
    let q = b.q_unit_cnt.min(30);
    for c in b.ch.iter_mut().take(1 + usize::from(stereo)) {
        for j in 0..q {
            let scale = atx(&t::SCALEFACTOR_C, c.scalefactors[j]);
            for k in coeff_idx(j)..coeff_idx(j + 1) {
                c.coeffs[k] *= scale;
            }
        }
    }
}

fn fill_with_noise(lfg: &mut Lfg, c: &mut Chan, start: usize, count: usize) {
    let mut maxval = 0f32;
    for i in (0..count).step_by(2) {
        let tmp = lfg.bmg();
        put(&mut c.coeffs, start + i, tmp[0] as f32);
        put(&mut c.coeffs, start + i + 1, tmp[1] as f32);
        maxval = tmp[0].abs().max(tmp[1].abs()).max(f64::from(maxval)) as f32;
    }
    for v in c.coeffs.iter_mut().skip(start).take(count) {
        *v /= maxval;
    }
}

fn scale_band_ext_coeffs(c: &mut Chan, sf: &[f32; 6], s_unit: usize, e_unit: usize) {
    for i in s_unit..e_unit {
        let f = at(sf, i - s_unit);
        for j in coeff_idx(i)..coeff_idx(i + 1) {
            c.coeffs[j] *= f;
        }
    }
}

fn apply_band_extension(b: &mut Block, lfg: &mut Lfg, stereo: bool) {
    let q = b.q_unit_cnt;
    let Some(grp) = q.checked_sub(13).and_then(|g| t::TAB_BAND_EXT_GROUP.get(g * 3..g * 3 + 3)) else { return };
    let g2 = usize::from(grp[1]);
    let g_units = [q, usize::from(grp[0]), g2, g2.max(22)];
    let g_bins = g_units.map(coeff_idx);
    let ext = usize::from(grp[2]);
    for c in b.ch.iter_mut().take(1 + usize::from(stereo)) {
        // mirror the spectrum
        for i in 0..3 {
            for j in 0..g_bins[i + 1].saturating_sub(g_bins[i]) {
                let v = g_bins[i].checked_sub(j + 1).map_or(0.0, |k| at(&c.coeffs, k));
                put(&mut c.coeffs, g_bins[i] + j, v);
            }
        }
        match c.band_ext {
            0 => {
                let mut sf = [0f32; 6];
                let l = g_units[3] - g_units[0] - 1;
                let n_start = coeff_idx(g_units[3] - 1);
                let n_cnt = usize::from(at(&t::Q_UNIT_TO_COEFF_CNT, g_units[3] - 1));
                let d = c.band_ext_data.map(|x| x.clamp(0, 31) as usize);
                let m0 = |m: usize, k: usize, v: usize| at(&t::BAND_EXT_SCALES_M0, (m * 5 + k) * 32 + v);
                match ext {
                    3 | 4 => {
                        let m = ext - 3;
                        sf[0] = m0(m, 0, d[0]);
                        sf[1] = m0(m, 1, d[0]);
                        sf[2] = m0(m, 2, d[1]);
                        sf[3] = m0(m, 3, d[2]);
                        sf[4] = m0(m, 4, d[3]);
                    }
                    5 => {
                        sf[0] = m0(2, 0, d[0]);
                        sf[1] = m0(2, 1, d[1]);
                        sf[2] = m0(2, 2, d[1]);
                    }
                    _ => {}
                }
                put(&mut sf, l, atx(&t::SCALEFACTOR_C, at(&c.scalefactors, g_units[0])));
                fill_with_noise(lfg, c, n_start, n_cnt);
                scale_band_ext_coeffs(c, &sf, g_units[0], g_units[3]);
            }
            1 => {
                let mut sf = [0f32; 6];
                for i in g_units[0]..g_units[3] {
                    put(&mut sf, i - g_units[0], atx(&t::SCALEFACTOR_C, at(&c.scalefactors, i)));
                }
                fill_with_noise(lfg, c, g_bins[0], g_bins[3] - g_bins[0]);
                scale_band_ext_coeffs(c, &sf, g_units[0], g_units[3]);
            }
            2 => {
                let g_sf = [atx(&t::BAND_EXT_SCALES_M2, c.band_ext_data[0]), atx(&t::BAND_EXT_SCALES_M2, c.band_ext_data[1])];
                for (i, &f) in g_sf.iter().enumerate() {
                    for j in g_bins[i]..g_bins[i + 1] {
                        c.coeffs[j] *= f;
                    }
                }
            }
            3 => {
                let mut scale = usize::try_from(c.band_ext_data[0]).ok().and_then(|k| t::BAND_EXT_SCALES_M3.get(k * 2)).copied().unwrap_or(0.0);
                let rate = usize::try_from(c.band_ext_data[1]).ok().and_then(|k| t::BAND_EXT_SCALES_M3.get(k * 2 + 1)).copied().unwrap_or(0.0);
                let rate = 2f64.powf(f64::from(rate)) as f32;
                for j in g_bins[0]..g_bins[3] {
                    scale *= rate;
                    c.coeffs[j] *= scale;
                }
            }
            4 => {
                let m = atx(&t::BAND_EXT_SCALES_M4, c.band_ext_data[0]);
                let g_sf = [0.707_946_8_f32 * m, 0.501_190_2_f32 * m, 0.354_827_9_f32 * m];
                for (i, &f) in g_sf.iter().enumerate() {
                    for j in g_bins[i]..g_bins[i + 1] {
                        c.coeffs[j] *= f;
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extradata(sri: u32, layout: u32, frame_size: usize, superframe_idx: u32) -> [u8; 12] {
        let cfg = 0xFEu32 << 24 | sri << 20 | layout << 17 | (frame_size as u32 - 1) << 5 | superframe_idx << 3;
        let mut e = [0u8; 12];
        e[0] = 2;
        e[4..8].copy_from_slice(&cfg.to_be_bytes());
        e
    }

    #[test]
    fn code_tables_are_complete() {
        let tb = tables();
        for v in tb.sf_unsigned.iter().chain(&tb.sf_signed).flatten() {
            assert!(v.table.iter().all(|&(_, l)| l > 0));
        }
        let mut n = 0;
        for (v, _) in tb.coeff.iter().flatten() {
            assert!(v.table.iter().all(|&(_, l)| l > 0));
            n += 1;
        }
        assert_eq!(n, 42);
    }

    #[test]
    fn configurations_are_checked() {
        let d = Decoder::new(&extradata(7, 2, 256, 2), 1024).unwrap();
        assert_eq!((d.channels(), d.sample_rate(), d.samples_per_channel()), (2, 48_000, 1024));
        assert_eq!(d.samples_in(600), 512);
        assert_eq!(Decoder::new(&extradata(4, 4, 100, 0), 100).unwrap().channels(), 8);
        assert!(Decoder::new(&extradata(7, 6, 256, 2), 1024).is_err());
        assert!(Decoder::new(&extradata(7, 2, 256, 1), 1024).is_err());
        assert!(Decoder::new(&extradata(7, 2, 256, 2)[..11], 1024).is_err());
        assert!(Decoder::new(&extradata(7, 2, 256, 2), 0).is_err());
        let mut e = extradata(7, 2, 256, 2);
        e[4] = 0;
        assert!(Decoder::new(&e, 1024).is_err());
    }

    #[test]
    fn random_packets_do_not_panic() {
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        for layout in 0..6 {
            for sri in [0, 5, 7, 8, 12, 15] {
                let mut d = Decoder::new(&extradata(sri, layout, 64, 2), 256).unwrap();
                for _ in 0..200 {
                    let p: Vec<u8> = (0..256)
                        .map(|_| {
                            x ^= x << 13;
                            x ^= x >> 7;
                            x ^= x << 17;
                            x as u8
                        })
                        .collect();
                    if let Ok(s) = d.decode(&p) {
                        assert_eq!(s.len(), d.channels());
                    }
                }
            }
        }
    }
}
