// ATRAC3plus decoding after FFmpeg n7.1.1 libavcodec/atrac3plus.c, atrac3plusdsp.c,
// atrac3plusdec.c (Copyright (c) 2010-2013 Maxim Poliakovski) and atrac.c (Copyright (c)
// 2006-2008 Maxim Poliakovski, Benjamin Larsson), LGPL-2.1-or-later; see docs/licensing.md §5.
// CFA4 changes GPL-3.0-or-later.

//! ATRAC3plus (Sony's ATRAC3+, "ATRAC-X"; OpenMG `.oma`/`.aa3`, WAV `.at3`): a 16-band PQF
//! split, a 128-line MDCT per band with gain control, power compensation noise and sine-wave
//! (GHA) tones. A port of FFmpeg n7.1.1 `atrac3plus*.c`. Output: planar `f32` in `[-1, 1]`,
//! 2048 samples per channel and frame.

use std::f64::consts::PI;
use std::sync::OnceLock;

use crate::bits::Bits;
use crate::mdct::Imdct;
use crate::tables_atrac3p as t;
use crate::{Error, Result};

const SUBBANDS: usize = 16;
const SB_SAMPLES: usize = 128;
const FRAME_SAMPLES: usize = SB_SAMPLES * SUBBANDS;
const PQF_FIR_LEN: usize = 12;
const POWER_COMP_OFF: u8 = 15;
const UNIT_MONO: u32 = 0;
const UNIT_STEREO: u32 = 1;
const UNIT_EXTENSION: u32 = 2;
const UNIT_TERMINATOR: u32 = 3;
/// Largest frame accepted (OpenMG stores the size in 10 bits of 8-byte words, plus 8).
const MAX_BLOCK_ALIGN: usize = 0x3FF * 8 + 8;

fn invalid(what: &'static str) -> Error {
    Error::Invalid(what)
}

/// `sign_extend`.
fn sext(v: i32, bits: u32) -> i32 {
    let sh = 32 - bits;
    ((v as u32) << sh) as i32 >> sh
}

/// `av_log2` (0 for 0).
fn log2(v: i32) -> u32 {
    31u32.saturating_sub((v as u32 | 1).leading_zeros())
}

fn at<T: Copy + Default>(s: &[T], i: i32) -> T {
    usize::try_from(i).ok().and_then(|i| s.get(i)).copied().unwrap_or_default()
}

fn put<T>(s: &mut [T], i: i32, v: T) {
    if let Some(d) = usize::try_from(i).ok().and_then(|i| s.get_mut(i)) {
        *d = v;
    }
}

/// A single-level VLC table as `ff_vlc_init_from_lengths` builds it for `get_vlc2(.., 1)`:
/// the next `bits` bits index (symbol, length); a code outside the table reads as -1 and
/// consumes nothing.
struct Vlc {
    bits: u32,
    table: Vec<(i16, u8)>,
}

impl Vlc {
    /// `cb`: code counts for lengths 1..=12; `xlat`: the symbols in code order. Returns the
    /// table and the number of symbols used.
    fn new(cb: &[u8], xlat: &[u8]) -> (Self, usize) {
        let mut lens = Vec::new();
        for (b, &n) in cb.iter().take(12).enumerate() {
            lens.extend(std::iter::repeat_n(b as u32 + 1, usize::from(n)));
        }
        let bits = lens.last().copied().unwrap_or(1);
        let mut table = vec![(-1i16, 0u8); 1 << bits];
        let mut code: u64 = 0;
        for (i, &l) in lens.iter().enumerate() {
            let sym = i16::from(xlat.get(i).copied().unwrap_or(0));
            let start = ((code >> (32 - l)) as usize) << (bits - l);
            for e in table.iter_mut().skip(start).take(1 << (bits - l)) {
                *e = (sym, l as u8);
            }
            code += 1u64 << (32 - l);
        }
        (Self { bits, table }, lens.len())
    }
}

/// The bit reader of one frame, noting anything FFmpeg would read as an invalid code or past
/// the end (for [`Decoder::set_strict`]).
struct Rd<'a> {
    b: Bits<'a>,
    size_bits: usize,
    bad: bool,
}

impl Rd<'_> {
    fn get(&mut self, n: i32) -> i32 {
        self.b.get(n.clamp(0, 25) as u32) as i32
    }

    fn bit(&mut self) -> i32 {
        self.get(1)
    }

    fn vlc(&mut self, v: &Vlc) -> i32 {
        let (sym, len) = v.table.get(self.b.show(v.bits) as usize).copied().unwrap_or((-1, 0));
        if len == 0 {
            self.bad = true;
        }
        self.b.skip(u32::from(len));
        i32::from(sym)
    }
}

/// Tables shared by every stream, built once.
struct Tables {
    wl: Vec<Vlc>,
    ct: Vec<Vlc>,
    sf: Vec<Vlc>,
    spec: Vec<Vlc>,
    spec_map: [usize; 112],
    gain: Vec<Vlc>,
    tone: Vec<Vlc>,
    sine: Vec<f32>,
    hann: Vec<f32>,
    amp_sf: [f32; 64],
    sine128: [f32; 128],
    sine64: [f32; 64],
    gain1: [f32; 16],
    gain2: [f32; 31],
    /// The 16-point half inverse MDCT of the PQF (`AV_TX_FLOAT_MDCT`, scale 32/32768; `av_tx`'s
    /// sign is the opposite of [`Imdct`]'s formula).
    pqf_dct: Vec<[f64; 16]>,
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(Tables::new)
}

impl Tables {
    fn new() -> Self {
        let cb = |c: &[u8], i: usize| c.get(i * 12..i * 12 + 12).unwrap_or(&[]).to_vec();
        let mut wl = Vec::new();
        let mut ct = Vec::new();
        let mut x = 0;
        for i in 0..4 {
            let (v, n) = Vlc::new(&cb(&t::WL_CBS, i), t::WL_CT_XLATS.get(x..).unwrap_or(&[]));
            wl.push(v);
            x += n;
            let (v, n) = Vlc::new(&cb(&t::CT_CBS, i), t::WL_CT_XLATS.get(x..).unwrap_or(&[]));
            ct.push(v);
            x += n;
        }
        let build = |cbs: &[u8], xlats: &[u8], count: usize| {
            let mut x = 0;
            (0..count)
                .map(|i| {
                    let (v, n) = Vlc::new(&cb(cbs, i), xlats.get(x..).unwrap_or(&[]));
                    x += n;
                    v
                })
                .collect::<Vec<_>>()
        };
        let sf = build(&t::SF_CBS, &t::SF_XLATS, 8);
        let gain = build(&t::GAIN_CBS, &t::GAIN_XLATS, 11);
        let tone = build(&t::TONE_CBS, &t::TONE_XLATS, 7);
        let mut spec = Vec::new();
        let mut spec_map = [0usize; 112];
        let mut x = 0;
        for i in 0..112 {
            // int8_t in FFmpeg, read as uint8_t counts; a negative first entry reuses a table
            let row = t::SPECTRA_CBS.get(i * 12..i * 12 + 12).unwrap_or(&[]);
            let first = row.first().copied().unwrap_or(0) as i8;
            if first >= 0 {
                let (v, n) = Vlc::new(row, t::SPECTRA_XLATS.get(x..).unwrap_or(&[]));
                x += n;
                spec_map[i] = spec.len();
                spec.push(v);
            } else {
                spec_map[i] = spec_map.get(usize::from(first.unsigned_abs())).copied().unwrap_or(0);
            }
        }
        let sine = (0..2048).map(|i| (2.0 * PI * f64::from(i) / 2048.0).sin() as f32).collect();
        let hann = (0..256).map(|i| ((1.0 - (2.0 * PI * f64::from(i) / 256.0).cos()) * 0.5) as f32).collect();
        let mut amp_sf = [0f32; 64];
        for (i, v) in amp_sf.iter_mut().enumerate() {
            *v = ((i as i32 - 3) as f32 / 4.0).exp2();
        }
        let sine_window = |n: usize, w: &mut [f32]| {
            for (i, v) in w.iter_mut().enumerate() {
                *v = (((i as f64 + 0.5) * (PI / (2.0 * n as f64))) as f32).sin();
            }
        };
        let mut sine128 = [0f32; 128];
        sine_window(128, &mut sine128);
        let mut sine64 = [0f32; 64];
        sine_window(64, &mut sine64);
        // ff_atrac_init_gain_compensation(6, 2)
        let mut gain1 = [0f32; 16];
        for (i, v) in gain1.iter_mut().enumerate() {
            *v = 2f32.powf((6 - i as i32) as f32);
        }
        let mut gain2 = [0f32; 31];
        for (k, v) in gain2.iter_mut().enumerate() {
            *v = 2f32.powf(-1.0f32 / 4.0 * (k as i32 - 15) as f32);
        }
        let scale = -f64::from(32.0f32 / 32768.0);
        let pqf_dct = (0..16)
            .map(|i| {
                let mut r = [0f64; 16];
                for (k, v) in r.iter_mut().enumerate() {
                    *v = scale * (PI / 16.0 * (f64::from(i) + 16.5) * (k as f64 + 0.5)).cos();
                }
                r
            })
            .collect();
        Self { wl, ct, sf, spec, spec_map, gain, tone, sine, hann, amp_sf, sine128, sine64, gain1, gain2, pqf_dct }
    }

    fn spec_vlc(&self, i: usize) -> Option<&Vlc> {
        self.spec.get(*self.spec_map.get(i)?)
    }
}

#[derive(Clone, Copy, Default)]
struct Gain {
    num_points: i32,
    lev: [i32; 7],
    loc: [i32; 7],
}

impl Gain {
    const EMPTY: Gain = Gain { num_points: 0, lev: [0; 7], loc: [0; 7] };

    fn n(&self) -> i32 {
        self.num_points.clamp(0, 7)
    }
}

#[derive(Clone, Copy, Default)]
struct Env {
    has_start: i32,
    has_stop: i32,
    start_pos: i32,
    stop_pos: i32,
}

#[derive(Clone, Copy, Default)]
struct Waves {
    pend: Env,
    curr: Env,
    num_wavs: i32,
    start_index: i32,
}

impl Waves {
    const EMPTY: Waves = Waves { pend: Env::EMPTY, curr: Env::EMPTY, num_wavs: 0, start_index: 0 };
}

impl Env {
    const EMPTY: Env = Env { has_start: 0, has_stop: 0, start_pos: 0, stop_pos: 0 };
}

#[derive(Clone, Copy, Default)]
struct Wave {
    freq: i32,
    amp_sf: i32,
    amp_index: i32,
    phase: i32,
}

#[derive(Clone)]
struct Synth {
    tones_present: bool,
    amplitude_mode: bool,
    num_tone_bands: i32,
    tone_sharing: [u8; SUBBANDS],
    tone_master: [u8; SUBBANDS],
    invert_phase: [u8; SUBBANDS],
    tones_index: i32,
    waves: [Wave; 48],
}

impl Synth {
    fn new() -> Self {
        Self {
            tones_present: false,
            amplitude_mode: false,
            num_tone_bands: 0,
            tone_sharing: [0; SUBBANDS],
            tone_master: [0; SUBBANDS],
            invert_phase: [0; SUBBANDS],
            tones_index: 0,
            waves: [Wave::default(); 48],
        }
    }

    fn bands(&self) -> usize {
        self.num_tone_bands.clamp(0, SUBBANDS as i32) as usize
    }
}

/// `Atrac3pChanParams`; the two-frame histories are indexed by `cur` (this frame) and `cur ^ 1`.
#[derive(Clone)]
struct Chan {
    ch_num: i32,
    num_coded_vals: i32,
    fill_mode: i32,
    split_point: i32,
    table_type: i32,
    wl: [i32; 32],
    sf: [i32; 32],
    tab: [i32; 32],
    spectrum: [i16; FRAME_SAMPLES],
    power_levs: [u8; 5],
    wnd: [[u8; SUBBANDS]; 2],
    gain: [[Gain; SUBBANDS]; 2],
    tones: [[Waves; SUBBANDS]; 2],
    cur: usize,
}

impl Chan {
    const fn new(ch_num: i32) -> Self {
        Self {
            ch_num,
            num_coded_vals: 0,
            fill_mode: 0,
            split_point: 0,
            table_type: 0,
            wl: [0; 32],
            sf: [0; 32],
            tab: [0; 32],
            spectrum: [0; FRAME_SAMPLES],
            power_levs: [0; 5],
            wnd: [[0; SUBBANDS]; 2],
            gain: [[Gain::EMPTY; SUBBANDS]; 2],
            tones: [[Waves::EMPTY; SUBBANDS]; 2],
            cur: 0,
        }
    }
}

/// The reference channel of channel 0 (never read: every use is for channel 1).
static NO_REF: Chan = Chan::new(0);

/// Channel `ch` and its reference channel 0.
fn pair(chans: &mut [Chan; 2], ch: usize) -> (&mut Chan, &Chan) {
    let (a, b) = chans.split_at_mut(1);
    if ch == 0 { (&mut a[0], &NO_REF) } else { (&mut b[0], &a[0]) }
}

#[derive(Clone)]
struct Ipqf {
    buf1: [[f32; 8]; PQF_FIR_LEN * 2],
    buf2: [[f32; 8]; PQF_FIR_LEN * 2],
    pos: usize,
}

/// `Atrac3pChanUnitCtx`.
#[derive(Clone)]
struct Unit {
    unit_type: u32,
    num_quant_units: i32,
    num_subbands: i32,
    used_quant_units: i32,
    num_coded_subbands: i32,
    mute: bool,
    use_full_table: i32,
    swap: [u8; SUBBANDS],
    negate: [u8; SUBBANDS],
    ch: [Chan; 2],
    synth: [Synth; 2],
    wcur: usize,
    ipqf: [Ipqf; 2],
    prev_buf: [[f32; FRAME_SAMPLES]; 2],
}

impl Unit {
    fn new() -> Self {
        let ipqf = Ipqf { buf1: [[0.0; 8]; PQF_FIR_LEN * 2], buf2: [[0.0; 8]; PQF_FIR_LEN * 2], pos: 0 };
        Self {
            unit_type: 0,
            num_quant_units: 0,
            num_subbands: 0,
            used_quant_units: 0,
            num_coded_subbands: 0,
            mute: false,
            use_full_table: 0,
            swap: [0; SUBBANDS],
            negate: [0; SUBBANDS],
            ch: [Chan::new(0), Chan::new(1)],
            synth: [Synth::new(), Synth::new()],
            wcur: 0,
            ipqf: [ipqf.clone(), ipqf],
            prev_buf: [[0.0; FRAME_SAMPLES]; 2],
        }
    }
}

/// An ATRAC3plus decoder.
#[derive(Clone)]
pub struct Decoder {
    channels: usize,
    block_align: usize,
    blocks: Vec<u32>,
    map: &'static [usize],
    units: Vec<Unit>,
    imdct: Imdct,
    strict: bool,
    bits_used: usize,
}

impl Decoder {
    /// A decoder for `channels` (1–4, 6, 7 or 8) and frames of `block_align` bytes.
    pub fn new(channels: u16, block_align: usize) -> Result<Self> {
        const MAP: [&[usize]; 8] = [&[0], &[0, 1], &[0, 1, 2], &[0, 1, 2, 3], &[0], &[0, 1, 2, 4, 5, 3], &[0, 1, 2, 4, 5, 6, 3], &[0, 1, 2, 4, 5, 6, 7, 3]];
        let (m, s) = (UNIT_MONO, UNIT_STEREO);
        let blocks: Vec<u32> = match channels {
            1 => vec![m],
            2 => vec![s],
            3 => vec![s, m],
            4 => vec![s, m, m],
            6 => vec![s, m, s, m],
            7 => vec![s, m, s, m, m],
            8 => vec![s, m, s, s, m],
            _ => return Err(Error::Unsupported("ATRAC3plus channel count")),
        };
        if block_align == 0 || block_align > MAX_BLOCK_ALIGN {
            return Err(invalid("ATRAC3plus frame size"));
        }
        let map = MAP.get(usize::from(channels) - 1).copied().unwrap_or(&[]);
        Ok(Self {
            channels: usize::from(channels),
            block_align,
            units: vec![Unit::new(); blocks.len()],
            blocks,
            map,
            // FFmpeg's scale -1 in `av_tx` terms
            imdct: Imdct::new(128, 1.0),
            strict: false,
            bits_used: 0,
        })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn block_align(&self) -> usize {
        self.block_align
    }

    pub fn samples_per_channel(&self) -> usize {
        FRAME_SAMPLES
    }

    /// Strict mode refuses a frame that holds a code outside the Huffman tables, an index past
    /// a table or reads past its end, where FFmpeg would go on with undefined values (used to
    /// pick test frames whose decoding is fully defined).
    pub fn set_strict(&mut self, on: bool) {
        self.strict = on;
    }

    /// Bits the last decoded frame used, its terminator included.
    pub fn bits_used(&self) -> usize {
        self.bits_used
    }

    /// Decodes one frame (the first [`Self::block_align`] bytes of `frame`) into planar
    /// samples. A damaged frame is an error; the decoder keeps going with the next one, as
    /// FFmpeg's does.
    pub fn decode(&mut self, frame: &[u8]) -> Result<Vec<Vec<f32>>> {
        let t = tables();
        let data = frame.get(..self.block_align.min(frame.len())).unwrap_or(&[]);
        if data.is_empty() {
            return Err(invalid("ATRAC3plus empty frame"));
        }
        let mut rd = Rd { b: Bits::new(data, data.len() * 8), size_bits: data.len() * 8, bad: false };
        if rd.bit() != 0 {
            return Err(invalid("ATRAC3plus start bit"));
        }
        let mut out = vec![vec![0f32; FRAME_SAMPLES]; self.channels];
        let mut block = 0;
        let mut out_ch = 0;
        let mut samples = Box::new([[0f32; FRAME_SAMPLES]; 2]);
        let mut outp = Box::new([[0f32; FRAME_SAMPLES]; 2]);
        while rd.b.left() >= 2 {
            let id = rd.get(2) as u32;
            if id == UNIT_TERMINATOR {
                break;
            }
            if id == UNIT_EXTENSION {
                return Err(Error::Unsupported("ATRAC3plus channel unit extension"));
            }
            if self.blocks.get(block) != Some(&id) {
                return Err(invalid("ATRAC3plus frame does not match the channel layout"));
            }
            let n = id as usize + 1;
            let strict = self.strict;
            let u = self.units.get_mut(block).ok_or(invalid("ATRAC3plus channel block"))?;
            u.unit_type = id;
            decode_channel_unit(&mut rd, u, n, t)?;
            if strict && (rd.bad || rd.b.count() > rd.size_bits) {
                return Err(invalid("ATRAC3plus undefined coding"));
            }
            residual(u, n, &mut samples);
            reconstruct(u, t, &self.imdct, n, &mut samples, &mut outp);
            for (i, o) in outp.iter().take(n).enumerate() {
                let c = self.map.get(out_ch + i).copied().unwrap_or(usize::MAX);
                if let Some(d) = out.get_mut(c) {
                    d.copy_from_slice(o);
                }
            }
            block += 1;
            out_ch += n;
        }
        // FFmpeg leaves the channels of missing units as whatever its buffer held
        if self.strict && block < self.blocks.len() {
            return Err(invalid("ATRAC3plus frame without every channel unit"));
        }
        self.bits_used = rd.b.count();
        Ok(out)
    }
}

// ---- bitstream (atrac3plus.c) ----

fn num_coded_units(rd: &mut Rd<'_>, chan: &mut Chan, nqu: i32) -> Result<()> {
    chan.fill_mode = rd.get(2);
    if chan.fill_mode == 0 {
        chan.num_coded_vals = nqu;
    } else {
        chan.num_coded_vals = rd.get(5);
        if chan.num_coded_vals > nqu {
            return Err(invalid("ATRAC3plus number of transmitted units"));
        }
        if chan.fill_mode == 3 {
            chan.split_point = rd.get(2) + (chan.ch_num << 1) + 1;
        }
    }
    Ok(())
}

/// `unpack_vq_shape`; `shape` holds 9 values.
fn unpack_vq_shape(start: i32, shape: &[i8], dst: &mut [i32; 32], n: i32) {
    if n != 0 {
        dst[0] = start;
        dst[1] = start;
        dst[2] = start;
        for i in 3..n.clamp(0, 32) {
            let seg = i32::from(at(&t::QU_NUM_TO_SEG, i)) - 1;
            put(dst, i, start - i32::from(at(shape, seg)));
        }
    }
}

fn wl_shape(start: i32, k: i32) -> &'static [i8] {
    let i = usize::try_from((start * 16 + k) * 9).unwrap_or(usize::MAX);
    t::WL_SHAPES.get(i..i + 9).unwrap_or(&[])
}

fn sf_shape(k: i32) -> &'static [i8] {
    let i = usize::try_from(k * 9).unwrap_or(usize::MAX);
    t::SF_SHAPES.get(i..i + 9).unwrap_or(&[])
}

fn vlc_of(v: &[Vlc], i: i32) -> Result<&Vlc> {
    usize::try_from(i).ok().and_then(|i| v.get(i)).ok_or(invalid("ATRAC3plus table index"))
}

fn decode_channel_wordlen(rd: &mut Rd<'_>, u: &mut Unit, ch: usize, t: &Tables) -> Result<()> {
    let nqu = u.num_quant_units;
    let (chan, rf) = pair(&mut u.ch, ch);
    let mut weight_idx = 0;
    chan.fill_mode = 0;
    match rd.get(2) {
        0 => {
            for i in 0..nqu {
                let v = rd.get(3);
                put(&mut chan.wl, i, v);
            }
        }
        1 => {
            if ch != 0 {
                num_coded_units(rd, chan, nqu)?;
                if chan.num_coded_vals != 0 {
                    let v = vlc_of(&t.wl, rd.get(2))?;
                    for i in 0..chan.num_coded_vals {
                        let d = rd.vlc(v);
                        put(&mut chan.wl, i, (at(&rf.wl, i) + d) & 7);
                    }
                }
            } else {
                weight_idx = rd.get(2);
                num_coded_units(rd, chan, nqu)?;
                if chan.num_coded_vals != 0 {
                    let pos = rd.get(5);
                    if pos > chan.num_coded_vals {
                        return Err(invalid("ATRAC3plus word length position"));
                    }
                    let delta_bits = rd.get(2);
                    let min_val = rd.get(3);
                    for i in 0..pos {
                        let v = rd.get(3);
                        put(&mut chan.wl, i, v);
                    }
                    for i in pos..chan.num_coded_vals {
                        let v = (min_val + rd.get(delta_bits)) & 7;
                        put(&mut chan.wl, i, v);
                    }
                }
            }
        }
        2 => {
            num_coded_units(rd, chan, nqu)?;
            let ncv = chan.num_coded_vals;
            if ch != 0 && ncv != 0 {
                let v = vlc_of(&t.wl, rd.get(2))?;
                let d = rd.vlc(v);
                chan.wl[0] = (rf.wl[0] + d) & 7;
                for i in 1..ncv {
                    let diff = at(&rf.wl, i) - at(&rf.wl, i - 1);
                    let d = rd.vlc(v);
                    let nv = (at(&chan.wl, i - 1) + diff + d) & 7;
                    put(&mut chan.wl, i, nv);
                }
            } else if ncv != 0 {
                let flag = rd.bit();
                let v = vlc_of(&t.wl, rd.bit())?;
                let start = rd.get(3);
                let k = rd.get(4);
                unpack_vq_shape(start, wl_shape(start, k), &mut chan.wl, ncv);
                if flag == 0 {
                    for i in 0..ncv {
                        let d = rd.vlc(v);
                        let nv = (at(&chan.wl, i) + d) & 7;
                        put(&mut chan.wl, i, nv);
                    }
                } else {
                    let mut i = 0;
                    while i < (ncv & -2) {
                        if rd.bit() == 0 {
                            let d = rd.vlc(v);
                            let nv = (at(&chan.wl, i) + d) & 7;
                            put(&mut chan.wl, i, nv);
                            let d = rd.vlc(v);
                            let nv = (at(&chan.wl, i + 1) + d) & 7;
                            put(&mut chan.wl, i + 1, nv);
                        }
                        i += 2;
                    }
                    if ncv & 1 != 0 {
                        let d = rd.vlc(v);
                        let nv = (at(&chan.wl, i) + d) & 7;
                        put(&mut chan.wl, i, nv);
                    }
                }
            }
        }
        _ => {
            weight_idx = rd.get(2);
            num_coded_units(rd, chan, nqu)?;
            if chan.num_coded_vals != 0 {
                let v = vlc_of(&t.wl, rd.get(2))?;
                chan.wl[0] = rd.get(3);
                for i in 1..chan.num_coded_vals {
                    let d = rd.vlc(v);
                    let nv = (at(&chan.wl, i - 1) + d) & 7;
                    put(&mut chan.wl, i, nv);
                }
            }
        }
    }
    if chan.fill_mode == 2 {
        for i in chan.num_coded_vals..nqu {
            let v = if ch != 0 { rd.bit() } else { 1 };
            put(&mut chan.wl, i, v);
        }
    } else if chan.fill_mode == 3 {
        let pos = if ch != 0 { chan.num_coded_vals + chan.split_point } else { nqu - chan.split_point };
        // FFmpeg compares with the unsigned array size, so a negative position becomes 32 too
        let pos = if (0..=32).contains(&pos) { pos } else { 32 };
        for i in chan.num_coded_vals..pos {
            put(&mut chan.wl, i, 1);
        }
    }
    if weight_idx != 0 {
        let row = usize::try_from(chan.ch_num * 3 + weight_idx - 1).unwrap_or(usize::MAX);
        let w = t::WL_WEIGHTS.get(row * 32..row * 32 + 32).ok_or(invalid("ATRAC3plus word length weights"))?;
        for i in 0..nqu {
            let v = at(&chan.wl, i) + i32::from(at(w, i));
            put(&mut chan.wl, i, v);
            if !(0..=7).contains(&v) {
                return Err(invalid("ATRAC3plus word length out of range"));
            }
        }
    }
    Ok(())
}

fn decode_channel_sf_idx(rd: &mut Rd<'_>, u: &mut Unit, ch: usize, t: &Tables) -> Result<()> {
    let used = u.used_quant_units;
    let (chan, rf) = pair(&mut u.ch, ch);
    let mut weight_idx = 0;
    let sf = &mut chan.sf;
    match rd.get(2) {
        0 => {
            for i in 0..used {
                let v = rd.get(6);
                put(sf, i, v);
            }
        }
        1 => {
            if ch != 0 {
                let v = vlc_of(&t.sf, rd.get(2))?;
                for i in 0..used {
                    let d = rd.vlc(v);
                    let nv = (at(&rf.sf, i) + d) & 0x3F;
                    put(sf, i, nv);
                }
            } else {
                weight_idx = rd.get(2);
                if weight_idx == 3 {
                    let start = rd.get(6);
                    let k = rd.get(6);
                    unpack_vq_shape(start, sf_shape(k), sf, used);
                    let num_long = rd.get(5);
                    let delta_bits = rd.get(2);
                    let min_val = rd.get(4) - 7;
                    for i in 0..num_long {
                        let v = (at(sf, i) + rd.get(4) - 7) & 0x3F;
                        put(sf, i, v);
                    }
                    for i in num_long..used {
                        let v = (at(sf, i) + min_val + rd.get(delta_bits)) & 0x3F;
                        put(sf, i, v);
                    }
                } else {
                    let num_long = rd.get(5);
                    let delta_bits = rd.get(3);
                    let min_val = rd.get(6);
                    if num_long > used || delta_bits == 7 {
                        return Err(invalid("ATRAC3plus scale factor parameters"));
                    }
                    for i in 0..num_long {
                        let v = rd.get(6);
                        put(sf, i, v);
                    }
                    for i in num_long..used {
                        let v = (min_val + rd.get(delta_bits)) & 0x3F;
                        put(sf, i, v);
                    }
                }
            }
        }
        2 => {
            if ch != 0 {
                let v = vlc_of(&t.sf, rd.get(2))?;
                let d = rd.vlc(v);
                sf[0] = (rf.sf[0] + d) & 0x3F;
                for i in 1..used {
                    let diff = at(&rf.sf, i) - at(&rf.sf, i - 1);
                    let d = rd.vlc(v);
                    let nv = (at(sf, i - 1) + diff + d) & 0x3F;
                    put(sf, i, nv);
                }
            } else {
                let v = vlc_of(&t.sf, rd.get(2) + 4)?;
                let start = rd.get(6);
                let k = rd.get(6);
                unpack_vq_shape(start, sf_shape(k), sf, used);
                for i in 0..used {
                    let d = rd.vlc(v);
                    let nv = (at(sf, i) + sext(d, 4)) & 0x3F;
                    put(sf, i, nv);
                }
            }
        }
        _ => {
            if ch != 0 {
                for i in 0..used {
                    let nv = at(&rf.sf, i);
                    put(sf, i, nv);
                }
            } else {
                weight_idx = rd.get(2);
                let sel = rd.get(2);
                if weight_idx == 3 {
                    let v = vlc_of(&t.sf, sel + 4)?;
                    let start = rd.get(6);
                    let k = rd.get(6);
                    unpack_vq_shape(start, sf_shape(k), sf, used);
                    let mut diff = (rd.get(4) + 56) & 0x3F;
                    sf[0] = (sf[0] + diff) & 0x3F;
                    for i in 1..used {
                        let d = rd.vlc(v);
                        diff = (diff + sext(d, 4)) & 0x3F;
                        let nv = (diff + at(sf, i)) & 0x3F;
                        put(sf, i, nv);
                    }
                } else {
                    let v = vlc_of(&t.sf, sel)?;
                    sf[0] = rd.get(6);
                    for i in 1..used {
                        let d = rd.vlc(v);
                        let nv = (at(sf, i - 1) + d) & 0x3F;
                        put(sf, i, nv);
                    }
                }
            }
        }
    }
    if weight_idx != 0 && weight_idx < 3 {
        let row = usize::try_from(weight_idx - 1).unwrap_or(usize::MAX);
        let w = t::SF_WEIGHTS.get(row * 32..row * 32 + 32).ok_or(invalid("ATRAC3plus scale factor weights"))?;
        for i in 0..used {
            let v = at(sf, i) - i32::from(at(w, i));
            put(sf, i, v);
            if !(0..=63).contains(&v) {
                return Err(invalid("ATRAC3plus scale factor out of range"));
            }
        }
    }
    Ok(())
}

fn decode_channel_code_tab(rd: &mut Rd<'_>, u: &mut Unit, ch: usize, t: &Tables) -> Result<()> {
    let full = u.use_full_table != 0;
    let mask = if full { 7 } else { 3 };
    let used = u.used_quant_units;
    let (chan, rf) = pair(&mut u.ch, ch);
    let num_vals = |rd: &mut Rd<'_>| -> Result<i32> {
        if rd.bit() != 0 {
            let n = rd.get(5);
            if n > used {
                return Err(invalid("ATRAC3plus number of code table indexes"));
            }
            Ok(n)
        } else {
            Ok(used)
        }
    };
    chan.table_type = rd.bit();
    let ct = &t.ct;
    let (vt, dt) = if full { (&ct[1], &ct[2]) } else { (&ct[0], &ct[0]) };
    let mode = rd.get(2);
    if mode == 3 && ch == 0 {
        return Ok(());
    }
    let n = num_vals(rd)?;
    let mut pred = 0;
    for i in 0..n {
        if at(&chan.wl, i) != 0 {
            let v = match mode {
                0 => rd.get(u.use_full_table + 2),
                1 => rd.vlc(vt),
                2 => {
                    let v = if i == 0 { rd.vlc(vt) } else { (pred + rd.vlc(dt)) & mask };
                    pred = v;
                    v
                }
                _ => (at(&rf.tab, i) + rd.vlc(if full { &ct[3] } else { &ct[0] })) & mask,
            };
            put(&mut chan.tab, i, v);
        } else if ch != 0 && at(&rf.wl, i) != 0 {
            let v = rd.bit();
            put(&mut chan.tab, i, v);
        }
    }
    Ok(())
}

fn decode_qu_spectra(rd: &mut Rd<'_>, tab: &[u8], vlc: &Vlc, out: &mut [i16], num_specs: usize) {
    let (group, nc, bits, signed) = (usize::from(at(tab, 0)), usize::from(at(tab, 1)), u32::from(at(tab, 2)), at(tab, 3) != 0);
    let mask = (1u32 << bits) - 1;
    let mut pos = 0;
    while pos < num_specs {
        if group == 1 || rd.bit() != 0 {
            for _ in 0..group {
                let mut val = rd.vlc(vlc) as u32;
                for _ in 0..nc {
                    let mut cf = (val & mask) as i32;
                    if signed {
                        cf = sext(cf, bits);
                    } else if cf != 0 && rd.bit() != 0 {
                        cf = -cf;
                    }
                    if let Some(o) = out.get_mut(pos) {
                        *o = cf as i16;
                    }
                    pos += 1;
                    val >>= bits;
                }
            }
        } else {
            pos += group * nc;
        }
        if group * nc == 0 {
            break;
        }
    }
}

fn qu_pos(qu: i32) -> usize {
    usize::from(at(&t::QU_TO_SPEC_POS, qu))
}

fn decode_spectrum(rd: &mut Rd<'_>, u: &mut Unit, n: usize, t: &Tables) -> Result<()> {
    let full = u.use_full_table != 0;
    let used = u.used_quant_units;
    let ncs = u.num_coded_subbands;
    for ch in 0..n {
        let (chan, rf) = pair(&mut u.ch, ch);
        chan.spectrum = [0; FRAME_SAMPLES];
        chan.power_levs = [POWER_COMP_OFF; 5];
        for qu in 0..used {
            let (p0, p1) = (qu_pos(qu), qu_pos(qu + 1));
            let num_specs = p1.saturating_sub(p0);
            let wl = at(&chan.wl, qu);
            let mut codetab = at(&chan.tab, qu);
            if wl != 0 {
                if !full {
                    if !(0..4).contains(&codetab) {
                        rd.bad = true;
                        return Err(invalid("ATRAC3plus code table index"));
                    }
                    codetab = i32::from(at(&t::CT_RESTRICTED_TO_FULL, (chan.table_type * 7 + wl - 1) * 4 + codetab));
                }
                if !(0..8).contains(&codetab) {
                    rd.bad = true;
                    return Err(invalid("ATRAC3plus code table index"));
                }
                let ti = (chan.table_type * 8 + codetab) * 7 + wl - 1;
                let tu = usize::try_from(ti).unwrap_or(usize::MAX);
                let tab = t::SPECTRA_TABS.get(tu * 4..tu * 4 + 4).ok_or(invalid("ATRAC3plus spectrum table"))?;
                let vlc = t.spec_vlc(tu).ok_or(invalid("ATRAC3plus spectrum table"))?;
                decode_qu_spectra(rd, tab, vlc, chan.spectrum.get_mut(p0..).unwrap_or(&mut []), num_specs);
            } else if ch != 0 && at(&rf.wl, qu) != 0 && codetab == 0 {
                if let (Some(d), Some(s)) = (chan.spectrum.get_mut(p0..p1), rf.spectrum.get(p0..p1)) {
                    d.copy_from_slice(s);
                }
                put(&mut chan.wl, qu, at(&rf.wl, qu));
            }
        }
        if used > 2 {
            let groups = at(&t::SUBBAND_TO_NUM_POWGRPS, ncs - 1);
            for i in 0..i32::from(groups) {
                let v = rd.get(4) as u8;
                put(&mut chan.power_levs, i, v);
            }
        }
    }
    Ok(())
}

fn subband_flags(rd: &mut Rd<'_>, out: &mut [u8; SUBBANDS], num: i32) -> bool {
    let n = num.clamp(0, SUBBANDS as i32) as usize;
    out[..n].fill(0);
    let result = rd.bit() != 0;
    if result {
        if rd.bit() != 0 {
            for o in out.iter_mut().take(n) {
                *o = rd.bit() as u8;
            }
        } else {
            out[..n].fill(1);
        }
    }
    result
}

fn decode_gainc_npoints(rd: &mut Rd<'_>, u: &mut Unit, ch: usize, coded: usize, t: &Tables) -> Result<()> {
    let (chan, rf) = pair(&mut u.ch, ch);
    let g = &mut chan.gain[chan.cur];
    let r = &rf.gain[rf.cur];
    match rd.get(2) {
        0 => {
            for d in g.iter_mut().take(coded) {
                d.num_points = rd.get(3);
            }
        }
        1 => {
            for d in g.iter_mut().take(coded) {
                d.num_points = rd.vlc(&t.gain[0]);
            }
        }
        2 => {
            if ch != 0 {
                for (d, r) in g.iter_mut().zip(r).take(coded) {
                    let delta = rd.vlc(&t.gain[1]);
                    d.num_points = (r.num_points + delta) & 7;
                }
            } else {
                g[0].num_points = rd.vlc(&t.gain[0]);
                for i in 1..coded.min(SUBBANDS) {
                    let delta = rd.vlc(&t.gain[1]);
                    g[i].num_points = (g[i - 1].num_points + delta) & 7;
                }
            }
        }
        _ => {
            if ch != 0 {
                for (d, r) in g.iter_mut().zip(r).take(coded) {
                    d.num_points = r.num_points;
                }
            } else {
                let delta_bits = rd.get(2);
                let min_val = rd.get(3);
                for d in g.iter_mut().take(coded) {
                    d.num_points = min_val + rd.get(delta_bits);
                    if d.num_points > 7 {
                        return Err(invalid("ATRAC3plus gain point count"));
                    }
                }
            }
        }
    }
    Ok(())
}

fn level_mode3s(dst: &mut Gain, r: &Gain) {
    for i in 0..dst.n() {
        put(&mut dst.lev, i, if i >= r.num_points { 7 } else { at(&r.lev, i) });
    }
}

fn level_mode1m(rd: &mut Rd<'_>, t: &Tables, dst: &mut Gain) {
    if dst.num_points > 0 {
        dst.lev[0] = rd.vlc(&t.gain[2]);
    }
    for i in 1..dst.n() {
        let delta = rd.vlc(&t.gain[3]);
        let nv = (at(&dst.lev, i - 1) + delta) & 0xF;
        put(&mut dst.lev, i, nv);
    }
}

fn decode_gainc_levels(rd: &mut Rd<'_>, u: &mut Unit, ch: usize, coded: usize, t: &Tables) -> Result<()> {
    let (chan, rf) = pair(&mut u.ch, ch);
    let g = &mut chan.gain[chan.cur];
    let r = &rf.gain[rf.cur];
    match rd.get(2) {
        0 => {
            for d in g.iter_mut().take(coded) {
                for i in 0..d.n() {
                    let v = rd.get(4);
                    put(&mut d.lev, i, v);
                }
            }
        }
        1 => {
            if ch != 0 {
                for (d, r) in g.iter_mut().zip(r).take(coded) {
                    for i in 0..d.n() {
                        let delta = rd.vlc(&t.gain[5]);
                        let pred = if i >= r.num_points { 7 } else { at(&r.lev, i) };
                        put(&mut d.lev, i, (pred + delta) & 0xF);
                    }
                }
            } else {
                for d in g.iter_mut().take(coded) {
                    level_mode1m(rd, t, d);
                }
            }
        }
        2 => {
            if ch != 0 {
                for (d, r) in g.iter_mut().zip(r).take(coded) {
                    if d.num_points > 0 {
                        if rd.bit() != 0 {
                            level_mode1m(rd, t, d);
                        } else {
                            level_mode3s(d, r);
                        }
                    }
                }
            } else {
                if g[0].num_points > 0 {
                    level_mode1m(rd, t, &mut g[0]);
                }
                for sb in 1..coded.min(SUBBANDS) {
                    let prev = g[sb - 1];
                    let d = &mut g[sb];
                    for i in 0..d.n() {
                        let delta = rd.vlc(&t.gain[4]);
                        let pred = if i >= prev.num_points { 7 } else { at(&prev.lev, i) };
                        put(&mut d.lev, i, (pred + delta) & 0xF);
                    }
                }
            }
        }
        _ => {
            if ch != 0 {
                for (d, r) in g.iter_mut().zip(r).take(coded) {
                    level_mode3s(d, r);
                }
            } else {
                let delta_bits = rd.get(2);
                let min_val = rd.get(4);
                for d in g.iter_mut().take(coded) {
                    for i in 0..d.n() {
                        let v = min_val + rd.get(delta_bits);
                        put(&mut d.lev, i, v);
                        if v > 15 {
                            return Err(invalid("ATRAC3plus gain level"));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn loc_mode0(rd: &mut Rd<'_>, dst: &mut Gain, pos: i32) {
    let prev = at(&dst.loc, pos - 1);
    let v = if pos == 0 || prev < 15 {
        rd.get(5)
    } else if prev >= 30 {
        31
    } else {
        let bits = log2(30 - prev) as i32 + 1;
        prev + rd.get(bits) + 1
    };
    put(&mut dst.loc, pos, v);
}

fn loc_mode1(rd: &mut Rd<'_>, t: &Tables, dst: &mut Gain) {
    if dst.num_points > 0 {
        dst.loc[0] = rd.get(5);
        for i in 1..dst.n() {
            let tab = if at(&dst.lev, i) <= at(&dst.lev, i - 1) { &t.gain[7] } else { &t.gain[9] };
            let v = at(&dst.loc, i - 1) + rd.vlc(tab);
            put(&mut dst.loc, i, v);
        }
    }
}

fn decode_gainc_loc_codes(rd: &mut Rd<'_>, u: &mut Unit, ch: usize, coded: usize, t: &Tables) -> Result<()> {
    let (chan, rf) = pair(&mut u.ch, ch);
    let g = &mut chan.gain[chan.cur];
    let r = &rf.gain[rf.cur];
    match rd.get(2) {
        0 => {
            for d in g.iter_mut().take(coded) {
                for i in 0..d.n() {
                    loc_mode0(rd, d, i);
                }
            }
        }
        1 => {
            if ch != 0 {
                for (d, r) in g.iter_mut().zip(r).take(coded) {
                    if d.num_points <= 0 {
                        continue;
                    }
                    let delta = rd.vlc(&t.gain[10]);
                    let pred = if r.num_points > 0 { r.loc[0] } else { 0 };
                    d.loc[0] = (pred + delta) & 0x1F;
                    for i in 1..d.n() {
                        let more_than_ref = i >= r.num_points;
                        if at(&d.lev, i) > at(&d.lev, i - 1) {
                            if more_than_ref {
                                let delta = rd.vlc(&t.gain[9]);
                                let nv = at(&d.loc, i - 1) + delta;
                                put(&mut d.loc, i, nv);
                            } else if rd.bit() != 0 {
                                loc_mode0(rd, d, i);
                            } else {
                                put(&mut d.loc, i, at(&r.loc, i));
                            }
                        } else {
                            let delta = rd.vlc(if more_than_ref { &t.gain[7] } else { &t.gain[10] });
                            let v = if more_than_ref { at(&d.loc, i - 1) + delta } else { (at(&r.loc, i) + delta) & 0x1F };
                            put(&mut d.loc, i, v);
                        }
                    }
                }
            } else {
                for d in g.iter_mut().take(coded) {
                    loc_mode1(rd, t, d);
                }
            }
        }
        2 => {
            if ch != 0 {
                for (d, r) in g.iter_mut().zip(r).take(coded) {
                    if d.num_points <= 0 {
                        continue;
                    }
                    if d.num_points > r.num_points || rd.bit() != 0 {
                        loc_mode1(rd, t, d);
                    } else {
                        for i in 0..d.n() {
                            put(&mut d.loc, i, at(&r.loc, i));
                        }
                    }
                }
            } else {
                for i in 0..g[0].n() {
                    loc_mode0(rd, &mut g[0], i);
                }
                for sb in 1..coded.min(SUBBANDS) {
                    let prev = g[sb - 1];
                    let d = &mut g[sb];
                    if d.num_points <= 0 {
                        continue;
                    }
                    let delta = rd.vlc(&t.gain[6]);
                    let pred = if prev.num_points > 0 { prev.loc[0] } else { 0 };
                    d.loc[0] = (pred + delta) & 0x1F;
                    for i in 1..d.n() {
                        let more_than_ref = i >= prev.num_points;
                        let k = i32::from(at(&d.lev, i) > at(&d.lev, i - 1)) * 2 + i32::from(more_than_ref) + 6;
                        let delta = rd.vlc(vlc_of(&t.gain, k)?);
                        let v = if more_than_ref { at(&d.loc, i - 1) + delta } else { (at(&prev.loc, i) + delta) & 0x1F };
                        put(&mut d.loc, i, v);
                    }
                }
            }
        }
        _ => {
            if ch != 0 {
                for (d, r) in g.iter_mut().zip(r).take(coded) {
                    for i in 0..d.n() {
                        if i >= r.num_points {
                            loc_mode0(rd, d, i);
                        } else {
                            put(&mut d.loc, i, at(&r.loc, i));
                        }
                    }
                }
            } else {
                let delta_bits = rd.get(2) + 1;
                let min_val = rd.get(5);
                for d in g.iter_mut().take(coded) {
                    for i in 0..d.n() {
                        let v = min_val + i + rd.get(delta_bits);
                        put(&mut d.loc, i, v);
                    }
                }
            }
        }
    }
    for d in g.iter().take(coded) {
        for i in 0..d.n() {
            let v = at(&d.loc, i);
            if !(0..=31).contains(&v) || (i > 0 && v <= at(&d.loc, i - 1)) {
                return Err(invalid("ATRAC3plus gain location"));
            }
        }
    }
    Ok(())
}

fn decode_gainc_data(rd: &mut Rd<'_>, u: &mut Unit, n: usize, t: &Tables) -> Result<()> {
    for ch in 0..n {
        let cur = u.ch[ch].cur;
        u.ch[ch].gain[cur] = [Gain::EMPTY; SUBBANDS];
        if rd.bit() != 0 {
            let coded = rd.get(4) as usize + 1;
            let num_gain_subbands = if rd.bit() != 0 { rd.get(4) as usize + 1 } else { coded };
            decode_gainc_npoints(rd, u, ch, coded, t)?;
            decode_gainc_levels(rd, u, ch, coded, t)?;
            decode_gainc_loc_codes(rd, u, ch, coded, t)?;
            let g = &mut u.ch[ch].gain[cur];
            for sb in coded..num_gain_subbands.min(SUBBANDS) {
                g[sb] = g[sb - 1];
            }
        }
    }
    Ok(())
}

fn wave_mut(s: &mut Synth, i: i32) -> Option<&mut Wave> {
    usize::try_from(i).ok().and_then(|i| s.waves.get_mut(i))
}

fn wave(s: &Synth, i: i32) -> Wave {
    at(&s.waves, i)
}

fn decode_tones_envelope(rd: &mut Rd<'_>, s: &Synth, chan: &mut Chan, rf: &Chan, ch: usize, bht: &[bool; SUBBANDS]) {
    let dst = &mut chan.tones[chan.cur];
    let r = &rf.tones[rf.cur];
    if ch == 0 || rd.bit() == 0 {
        for sb in 0..s.bands() {
            if !bht[sb] {
                continue;
            }
            let p = &mut dst[sb].pend;
            p.has_start = rd.bit();
            p.start_pos = if p.has_start != 0 { rd.get(5) } else { -1 };
            p.has_stop = rd.bit();
            p.stop_pos = if p.has_stop != 0 { rd.get(5) } else { 32 };
        }
    } else {
        for sb in 0..s.bands() {
            if bht[sb] {
                dst[sb].pend = r[sb].pend;
            }
        }
    }
}

fn decode_band_numwavs(rd: &mut Rd<'_>, s: &mut Synth, chan: &mut Chan, rf: &Chan, ch: usize, bht: &[bool; SUBBANDS], t: &Tables) -> Result<()> {
    let dst = &mut chan.tones[chan.cur];
    let r = &rf.tones[rf.cur];
    let mode = rd.get(ch as i32 + 1);
    for sb in 0..s.bands() {
        if !bht[sb] {
            continue;
        }
        match mode {
            0 => dst[sb].num_wavs = rd.get(4),
            1 => dst[sb].num_wavs = rd.vlc(&t.tone[1]),
            2 => {
                let delta = sext(rd.vlc(&t.tone[2]), 3);
                dst[sb].num_wavs = (r[sb].num_wavs + delta) & 0xF;
            }
            _ => dst[sb].num_wavs = r[sb].num_wavs,
        }
    }
    for sb in 0..s.bands() {
        if bht[sb] {
            if s.tones_index + dst[sb].num_wavs > 48 {
                return Err(invalid("ATRAC3plus too many tones"));
            }
            dst[sb].start_index = s.tones_index;
            s.tones_index += dst[sb].num_wavs;
        }
    }
    Ok(())
}

fn decode_tones_frequency(rd: &mut Rd<'_>, s: &mut Synth, chan: &Chan, rf: &Chan, ch: usize, bht: &[bool; SUBBANDS], t: &Tables) {
    let dst = &chan.tones[chan.cur];
    let r = &rf.tones[rf.cur];
    if ch == 0 || rd.bit() == 0 {
        for sb in 0..s.bands() {
            let (base, nw) = (dst[sb].start_index, dst[sb].num_wavs);
            if !bht[sb] || nw == 0 {
                continue;
            }
            let direction = if nw > 1 { rd.bit() } else { 0 };
            if direction != 0 {
                let v = rd.get(10);
                if let Some(w) = wave_mut(s, base + nw - 1) {
                    w.freq = v;
                }
                for i in (0..=nw - 2).rev() {
                    let nbits = log2(wave(s, base + i + 1).freq) as i32 + 1;
                    let v = rd.get(nbits);
                    if let Some(w) = wave_mut(s, base + i) {
                        w.freq = v;
                    }
                }
            } else {
                for i in 0..nw {
                    let prev = wave(s, base + i - 1).freq;
                    let v = if i == 0 || prev < 512 {
                        rd.get(10)
                    } else {
                        let nbits = log2(1023 - prev) as i32 + 1;
                        rd.get(nbits) + 1024 - (1 << nbits)
                    };
                    if let Some(w) = wave_mut(s, base + i) {
                        w.freq = v;
                    }
                }
            }
        }
    } else {
        for sb in 0..s.bands() {
            let nw = dst[sb].num_wavs;
            if !bht[sb] || nw == 0 {
                continue;
            }
            let (ib, ob, rn) = (r[sb].start_index, dst[sb].start_index, r[sb].num_wavs);
            for i in 0..nw {
                let delta = sext(rd.vlc(&t.tone[6]), 8);
                let pred = if i < rn {
                    wave(s, ib + i).freq
                } else if rn != 0 {
                    wave(s, ib + rn - 1).freq
                } else {
                    0
                };
                if let Some(w) = wave_mut(s, ob + i) {
                    w.freq = (pred + delta) & 0x3FF;
                }
            }
        }
    }
}

fn decode_tones_amplitude(rd: &mut Rd<'_>, s: &mut Synth, chan: &Chan, rf: &Chan, ch: usize, bht: &[bool; SUBBANDS], t: &Tables) {
    let dst = &chan.tones[chan.cur];
    let r = &rf.tones[rf.cur];
    let mut refwaves = [0i32; 48];
    if ch != 0 {
        for sb in 0..s.bands() {
            if !bht[sb] || dst[sb].num_wavs == 0 {
                continue;
            }
            let (ds, rs, rn) = (dst[sb].start_index, r[sb].start_index, r[sb].num_wavs);
            for j in 0..dst[sb].num_wavs {
                let (mut fi, mut maxdiff) = (0, 1024);
                for i in 0..rn {
                    let diff = (wave(s, ds + j).freq - wave(s, rs + i).freq).abs();
                    if diff < maxdiff {
                        maxdiff = diff;
                        fi = i;
                    }
                }
                let v = if maxdiff < 8 {
                    fi + rs
                } else if j < rn {
                    j + rs
                } else {
                    -1
                };
                put(&mut refwaves, ds + j, v);
            }
        }
    }
    let mode = rd.get(ch as i32 + 1);
    for sb in 0..s.bands() {
        let (start, nw) = (dst[sb].start_index, dst[sb].num_wavs);
        if !bht[sb] || (nw == 0 && mode != 3) {
            continue;
        }
        match mode {
            0 | 1 => {
                let count = if s.amplitude_mode { nw } else { 1 };
                for i in 0..count {
                    let v = match (mode, s.amplitude_mode) {
                        (0, _) => rd.get(6),
                        (_, true) => rd.vlc(&t.tone[3]) + 20,
                        (_, false) => rd.vlc(&t.tone[4]) + 24,
                    };
                    if let Some(w) = wave_mut(s, start + i) {
                        w.amp_sf = v;
                    }
                }
            }
            2 => {
                for i in 0..nw {
                    let delta = sext(rd.vlc(&t.tone[5]), 5);
                    let rw = at(&refwaves, start + i);
                    let pred = if rw >= 0 { wave(s, rw).amp_sf } else { 34 };
                    if let Some(w) = wave_mut(s, start + i) {
                        w.amp_sf = (pred + delta) & 0x3F;
                    }
                }
            }
            _ => {
                for i in 0..nw {
                    let rw = at(&refwaves, start + i);
                    let v = if rw >= 0 { wave(s, rw).amp_sf } else { 32 };
                    if let Some(w) = wave_mut(s, start + i) {
                        w.amp_sf = v;
                    }
                }
            }
        }
    }
}

fn decode_tones_phase(rd: &mut Rd<'_>, s: &mut Synth, chan: &Chan, bht: &[bool; SUBBANDS]) {
    let dst = &chan.tones[chan.cur];
    for sb in 0..s.bands() {
        if !bht[sb] {
            continue;
        }
        for i in 0..dst[sb].num_wavs {
            let v = rd.get(5);
            if let Some(w) = wave_mut(s, dst[sb].start_index + i) {
                w.phase = v;
            }
        }
    }
}

fn decode_tones_info(rd: &mut Rd<'_>, u: &mut Unit, n: usize, t: &Tables) -> Result<()> {
    for c in u.ch.iter_mut().take(n) {
        c.tones[c.cur] = [Waves::EMPTY; SUBBANDS];
    }
    let s = &mut u.synth[u.wcur];
    s.tones_present = rd.bit() != 0;
    if !s.tones_present {
        return Ok(());
    }
    s.waves = [Wave::default(); 48];
    s.amplitude_mode = rd.bit() != 0;
    if !s.amplitude_mode {
        return Err(Error::Unsupported("ATRAC3plus GHA amplitude mode 0"));
    }
    s.num_tone_bands = rd.vlc(&t.tone[0]) + 1;
    if n == 2 {
        let nb = s.num_tone_bands;
        subband_flags(rd, &mut s.tone_sharing, nb);
        subband_flags(rd, &mut s.tone_master, nb);
        subband_flags(rd, &mut s.invert_phase, nb);
    }
    s.tones_index = 0;
    for ch in 0..n {
        let mut bht = [false; SUBBANDS];
        for (i, b) in bht.iter_mut().enumerate().take(s.bands()) {
            *b = ch == 0 || s.tone_sharing[i] == 0;
        }
        let (chan, rf) = pair(&mut u.ch, ch);
        decode_tones_envelope(rd, s, chan, rf, ch, &bht);
        decode_band_numwavs(rd, s, chan, rf, ch, &bht, t)?;
        decode_tones_frequency(rd, s, chan, rf, ch, &bht, t);
        decode_tones_amplitude(rd, s, chan, rf, ch, &bht, t);
        decode_tones_phase(rd, s, chan, &bht);
    }
    if n == 2 {
        let (a, b) = u.ch.split_at_mut(1);
        let (c0, c1) = (&mut a[0], &mut b[0]);
        let (t0, t1) = (&mut c0.tones[c0.cur], &mut c1.tones[c1.cur]);
        for i in 0..s.bands() {
            if s.tone_sharing[i] != 0 {
                t1[i] = t0[i];
            }
            if s.tone_master[i] != 0 {
                std::mem::swap(&mut t0[i], &mut t1[i]);
            }
        }
    }
    Ok(())
}

fn decode_channel_unit(rd: &mut Rd<'_>, u: &mut Unit, n: usize, t: &Tables) -> Result<()> {
    u.num_quant_units = rd.get(5) + 1;
    if u.num_quant_units > 28 && u.num_quant_units < 32 {
        return Err(invalid("ATRAC3plus number of quantization units"));
    }
    u.mute = rd.bit() != 0;
    for ch in 0..n {
        u.ch[ch].wl = [0; 32];
        decode_channel_wordlen(rd, u, ch, t)?;
    }
    let mut i = u.num_quant_units - 1;
    while i >= 0 {
        if at(&u.ch[0].wl, i) != 0 || (n == 2 && at(&u.ch[1].wl, i) != 0) {
            break;
        }
        i -= 1;
    }
    u.used_quant_units = i + 1;
    u.num_subbands = i32::from(at(&t::QU_TO_SUBBAND, u.num_quant_units - 1)) + 1;
    u.num_coded_subbands = if u.used_quant_units > 0 { i32::from(at(&t::QU_TO_SUBBAND, u.used_quant_units - 1)) + 1 } else { 0 };
    if u.used_quant_units > 0 {
        for ch in 0..n {
            u.ch[ch].sf = [0; 32];
            decode_channel_sf_idx(rd, u, ch, t)?;
        }
        u.use_full_table = rd.bit();
        for ch in 0..n {
            u.ch[ch].tab = [0; 32];
            decode_channel_code_tab(rd, u, ch, t)?;
        }
    }
    decode_spectrum(rd, u, n, t)?;
    if n == 2 {
        subband_flags(rd, &mut u.swap, u.num_coded_subbands);
        subband_flags(rd, &mut u.negate, u.num_coded_subbands);
    }
    for c in u.ch.iter_mut().take(n) {
        let cur = c.cur;
        subband_flags(rd, &mut c.wnd[cur], u.num_subbands);
    }
    decode_gainc_data(rd, u, n, t)?;
    decode_tones_info(rd, u, n, t)?;
    // global noise info: level and table index, unused by the decoder
    if rd.bit() != 0 {
        rd.get(8);
    }
    Ok(())
}

// ---- reconstruction (atrac3plusdec.c, atrac3plusdsp.c, atrac.c) ----

fn residual(u: &Unit, n: usize, out: &mut [[f32; FRAME_SAMPLES]; 2]) {
    if u.mute {
        for o in out.iter_mut().take(n) {
            o.fill(0.0);
        }
        return;
    }
    let mut rng = 0i32;
    for qu in 0..u.used_quant_units {
        rng = rng.wrapping_add(at(&u.ch[0].sf, qu) + at(&u.ch[1].sf, qu));
    }
    let mut sb_rng = [0i32; SUBBANDS];
    for r in sb_rng.iter_mut().take(u.num_coded_subbands.clamp(0, 16) as usize) {
        *r = rng & 0x3FC;
        rng = rng.wrapping_add(128);
    }
    for (ch, o) in out.iter_mut().enumerate().take(n) {
        o.fill(0.0);
        let c = &u.ch[ch];
        for qu in 0..u.used_quant_units {
            let wl = at(&c.wl, qu);
            if wl > 0 {
                let q = at(&t::SF_TAB, at(&c.sf, qu)) * at(&t::MANT_TAB, wl);
                let (p0, p1) = (qu_pos(qu), qu_pos(qu + 1));
                if let (Some(d), Some(s)) = (o.get_mut(p0..p1), c.spectrum.get(p0..p1)) {
                    for (d, &s) in d.iter_mut().zip(s) {
                        *d = f32::from(s) * q;
                    }
                }
            }
        }
        for (sb, &r) in sb_rng.iter().enumerate().take(u.num_coded_subbands.clamp(0, 16) as usize) {
            power_compensation(u, ch, o, r, sb);
        }
    }
    if u.unit_type == UNIT_STEREO {
        let (a, b) = out.split_at_mut(1);
        for sb in 0..u.num_coded_subbands.clamp(0, 16) as usize {
            let r = sb * SB_SAMPLES..(sb + 1) * SB_SAMPLES;
            if u.swap[sb] != 0 {
                a[0][r.clone()].swap_with_slice(&mut b[0][r.clone()]);
            }
            if u.negate[sb] != 0 {
                for v in &mut b[0][r] {
                    *v = -*v;
                }
            }
        }
    }
}

fn power_compensation(u: &Unit, ch: usize, sp: &mut [f32], rng: i32, sb: usize) {
    let swap_ch = usize::from(u.unit_type == UNIT_STEREO && u.swap[sb] != 0);
    let Some(pc) = u.ch.get(ch ^ swap_ch) else { return };
    let grp = at(&t::SUBBAND_TO_POWGRP, sb as i32);
    let lev = at(&pc.power_levs, i32::from(grp));
    if lev == POWER_COMP_OFF {
        return;
    }
    let mut pwcsp = [0f32; SB_SAMPLES];
    for (i, p) in pwcsp.iter_mut().enumerate() {
        *p = at(&t::NOISE_TAB, (rng.wrapping_add(i as i32)) & 0x3FF);
    }
    let g1 = pc.gain[pc.cur][sb];
    let g2 = pc.gain[pc.cur ^ 1][sb];
    let gain_lev = if g1.num_points > 0 { 6 - g1.lev[0] } else { 0 };
    let mut gcv = 0;
    for i in 0..g2.n() {
        gcv = gcv.max(gain_lev - (at(&g2.lev, i) - 6));
    }
    for i in 0..g1.n() {
        gcv = gcv.max(6 - at(&g1.lev, i));
    }
    let grp_lev = at(&t::PWC_LEVS, i32::from(lev)) / (1i32 << gcv.clamp(0, 30)) as f32;
    let c = &u.ch[ch];
    let first = i32::from(at(&t::SUBBAND_TO_QU, sb as i32)) + if sb == 0 { 2 } else { 0 };
    for qu in first..i32::from(at(&t::SUBBAND_TO_QU, sb as i32 + 1)) {
        let wl = at(&c.wl, qu);
        if wl <= 0 {
            continue;
        }
        let qu_lev = at(&t::SF_TAB, at(&c.sf, qu)) * at(&t::MANT_TAB, wl) / (1i32 << wl.min(30)) as f32 * grp_lev;
        if let Some(d) = sp.get_mut(qu_pos(qu)..qu_pos(qu + 1)) {
            for (d, &p) in d.iter_mut().zip(&pwcsp) {
                *d += p * qu_lev;
            }
        }
    }
}

fn imdct_window(t: &Tables, imdct: &Imdct, input: &mut [f32], out: &mut [f32; 256], wind_id: u8, sb: usize) {
    if sb & 1 == 1 {
        input.reverse();
    }
    imdct.inverse(input, out);
    if wind_id & 2 != 0 {
        out[..32].fill(0.0);
        for (o, w) in out[32..96].iter_mut().zip(&t.sine64) {
            *o *= w;
        }
    } else {
        for (o, w) in out[..128].iter_mut().zip(&t.sine128) {
            *o *= w;
        }
    }
    if wind_id & 1 != 0 {
        for (o, w) in out[160..224].iter_mut().zip(t.sine64.iter().rev()) {
            *o *= w;
        }
        out[224..].fill(0.0);
    } else {
        for (o, w) in out[128..].iter_mut().zip(t.sine128.iter().rev()) {
            *o *= w;
        }
    }
}

/// `ff_atrac_gain_compensation` (128 samples, location scale 2, level offset 6).
fn gain_compensation(t: &Tables, input: &[f32; 256], prev: &mut [f32], now: &Gain, next: &Gain, out: &mut [f32]) {
    let n = SB_SAMPLES;
    let gc_scale = if next.num_points != 0 { at(&t.gain1, next.lev[0]) } else { 1.0 };
    let mut pos = 0;
    let mut put_at = |pos: usize, lev: f32| {
        if let (Some(o), Some(&p)) = (out.get_mut(pos), prev.get(pos)) {
            *o = (input[pos] * gc_scale + p) * lev;
        }
    };
    if now.num_points != 0 {
        for i in 0..now.n() {
            let lastpos = (at(&now.loc, i).max(0) as usize) << 2;
            let mut lev = at(&t.gain1, at(&now.lev, i));
            let next_lev = if i + 1 < now.num_points { at(&now.lev, i + 1) } else { 6 };
            let gain_inc = at(&t.gain2, next_lev - at(&now.lev, i) + 15);
            while pos < lastpos.min(n) {
                put_at(pos, lev);
                pos += 1;
            }
            while pos < (lastpos + 4).min(n) {
                put_at(pos, lev);
                lev *= gain_inc;
                pos += 1;
            }
        }
    }
    while pos < n {
        if let (Some(o), Some(&p)) = (out.get_mut(pos), prev.get(pos)) {
            *o = input[pos] * gc_scale + p;
        }
        pos += 1;
    }
    if let Some(p) = prev.get_mut(..n) {
        p.copy_from_slice(&input[n..]);
    }
}

fn waves_synth(t: &Tables, s: &Synth, wd: &Waves, env: &Env, invert: bool, reg_offset: i32, out: &mut [f32; SB_SAMPLES]) {
    for wn in 0..wd.num_wavs.max(0) {
        let w = wave(s, wd.start_index + wn);
        let scale = if s.amplitude_mode { 1.0 } else { (w.amp_index + 1) as f32 / 15.13 };
        let amp = f64::from(at(&t.amp_sf, w.amp_sf) * scale);
        let inc = w.freq;
        let mut pos = (((w.phase & 0x1F) << 6).wrapping_sub((reg_offset ^ 128).wrapping_mul(inc))) & 2047;
        for o in out.iter_mut() {
            *o = (f64::from(*o) + f64::from(at(&t.sine, pos)) * amp) as f32;
            pos = (pos.wrapping_add(inc)) & 2047;
        }
    }
    if invert {
        for o in out.iter_mut() {
            *o = -*o;
        }
    }
    let h = |i: usize| t.hann.get(i).copied().unwrap_or(0.0);
    if env.has_start != 0 {
        let pos = (env.start_pos << 2) - reg_offset;
        if pos > 0 && pos <= 128 {
            let p = pos as usize;
            out[..p].fill(0.0);
            if env.has_stop == 0 || env.start_pos != env.stop_pos {
                for (k, hi) in [0, 32, 64, 96].into_iter().enumerate() {
                    if let Some(o) = out.get_mut(p + k) {
                        *o *= h(hi);
                    }
                }
            }
        }
    }
    if env.has_stop != 0 {
        let pos = ((env.stop_pos + 1) << 2) - reg_offset;
        if pos > 0 && pos <= 128 {
            let p = pos as usize;
            for (k, hi) in [96, 64, 32, 0].into_iter().enumerate() {
                if let Some(o) = (p + k).checked_sub(4).and_then(|i| out.get_mut(i)) {
                    *o *= h(hi);
                }
            }
            out[p..].fill(0.0);
        }
    }
}

fn generate_tones(t: &Tables, c: &mut Chan, synth: &[Synth; 2], wcur: usize, ch_num: usize, sb: usize, out: &mut [f32]) {
    let cur = c.cur;
    let now = c.tones[cur ^ 1][sb];
    let next = &mut c.tones[cur][sb];
    if next.pend.has_start != 0 && next.pend.start_pos < next.pend.stop_pos {
        next.curr.has_start = 1;
        next.curr.start_pos = next.pend.start_pos + 32;
    } else if now.pend.has_start != 0 {
        next.curr.has_start = 1;
        next.curr.start_pos = now.pend.start_pos;
    } else {
        next.curr.has_start = 0;
        next.curr.start_pos = 0;
    }
    if now.pend.has_stop != 0 && now.pend.stop_pos >= next.curr.start_pos {
        next.curr.has_stop = 1;
        next.curr.stop_pos = now.pend.stop_pos;
    } else if next.pend.has_stop != 0 {
        next.curr.has_stop = 1;
        next.curr.stop_pos = next.pend.stop_pos + 32;
    } else {
        next.curr.has_stop = 0;
        next.curr.stop_pos = 64;
    }
    let reg1 = now.curr.stop_pos >= 32;
    let reg2 = next.curr.start_pos < 32;
    let mut w1 = [0f32; SB_SAMPLES];
    let mut w2 = [0f32; SB_SAMPLES];
    let (sp, sc) = (&synth[wcur ^ 1], &synth[wcur]);
    if now.num_wavs != 0 && reg1 {
        waves_synth(t, sp, &now, &now.curr, sp.invert_phase[sb] as usize & ch_num != 0, 128, &mut w1);
    }
    let next = *next;
    if next.num_wavs != 0 && reg2 {
        waves_synth(t, sc, &next, &next.curr, sc.invert_phase[sb] as usize & ch_num != 0, 0, &mut w2);
    }
    let (h_lo, h_hi) = t.hann.split_at(128);
    let fmul = |w: &mut [f32; SB_SAMPLES], h: &[f32]| {
        for (v, &k) in w.iter_mut().zip(h) {
            *v *= k;
        }
    };
    if now.num_wavs != 0 && next.num_wavs != 0 && reg1 && reg2 {
        fmul(&mut w1, h_hi);
        fmul(&mut w2, h_lo);
    } else {
        if now.num_wavs != 0 && now.curr.has_stop == 0 {
            fmul(&mut w1, h_hi);
        }
        if next.num_wavs != 0 && next.curr.has_start == 0 {
            fmul(&mut w2, h_lo);
        }
    }
    for ((o, a), b) in out.iter_mut().zip(w1).zip(w2) {
        *o += a + b;
    }
}

fn ipqf(t: &Tables, h: &mut Ipqf, input: &[f32; FRAME_SAMPLES], out: &mut [f32; FRAME_SAMPLES]) {
    out.fill(0.0);
    let lut = |i: usize| usize::from(t::MOD23_LUT.get(i).copied().unwrap_or(0));
    for s in 0..SB_SAMPLES {
        let mut x = [0f32; SUBBANDS];
        for (sb, v) in x.iter_mut().enumerate() {
            *v = input[sb * SB_SAMPLES + s];
        }
        let mut y = [0f32; SUBBANDS];
        for (yi, row) in y.iter_mut().zip(&t.pqf_dct) {
            *yi = row.iter().zip(&x).map(|(&m, &v)| m * f64::from(v)).sum::<f64>() as f32;
        }
        let p = h.pos.min(PQF_FIR_LEN * 2 - 1);
        for i in 0..8 {
            h.buf1[p][i] = y[i + 8];
            h.buf2[p][i] = y[7 - i];
        }
        let mut pos_now = p;
        let mut pos_next = lut(pos_now + 2);
        let o = &mut out[s * 16..s * 16 + 16];
        for tt in 0..PQF_FIR_LEN {
            let c1 = t::IPQF_COEFFS1.get(tt * 16..tt * 16 + 16).unwrap_or(&[0.0; 16]);
            let c2 = t::IPQF_COEFFS2.get(tt * 16..tt * 16 + 16).unwrap_or(&[0.0; 16]);
            let (b1, b2) = (h.buf1.get(pos_now).copied().unwrap_or_default(), h.buf2.get(pos_next).copied().unwrap_or_default());
            for i in 0..8 {
                o[i] += b1[i] * c1[i] + b2[i] * c2[i];
                o[i + 8] += b1[7 - i] * c1[i + 8] + b2[7 - i] * c2[i + 8];
            }
            pos_now = lut(pos_next + 2);
            pos_next = lut(pos_now + 2);
        }
        h.pos = lut(p);
    }
}

fn reconstruct(u: &mut Unit, t: &Tables, imdct: &Imdct, n: usize, samples: &mut [[f32; FRAME_SAMPLES]; 2], outp: &mut [[f32; FRAME_SAMPLES]; 2]) {
    let nsb = u.num_subbands.clamp(0, 16) as usize;
    let mut time = [0f32; FRAME_SAMPLES];
    for ch in 0..n {
        let c = &u.ch[ch];
        let (cur, prev) = (c.cur, c.cur ^ 1);
        for sb in 0..nsb {
            let r = sb * SB_SAMPLES..(sb + 1) * SB_SAMPLES;
            let mut mdct = [0f32; 256];
            let wind_id = (c.wnd[prev][sb] << 1) + c.wnd[cur][sb];
            imdct_window(t, imdct, &mut samples[ch][r.clone()], &mut mdct, wind_id, sb);
            gain_compensation(t, &mdct, &mut u.prev_buf[ch][r.clone()], &c.gain[prev][sb], &c.gain[cur][sb], &mut time[r]);
        }
        u.prev_buf[ch][nsb * SB_SAMPLES..].fill(0.0);
        time[nsb * SB_SAMPLES..].fill(0.0);
        if u.synth[u.wcur].tones_present || u.synth[u.wcur ^ 1].tones_present {
            let c = &mut u.ch[ch];
            for sb in 0..nsb {
                if c.tones[c.cur][sb].num_wavs != 0 || c.tones[c.cur ^ 1][sb].num_wavs != 0 {
                    generate_tones(t, c, &u.synth, u.wcur, ch, sb, &mut time[sb * SB_SAMPLES..(sb + 1) * SB_SAMPLES]);
                }
            }
        }
        ipqf(t, &mut u.ipqf[ch], &time, &mut outp[ch]);
    }
    for c in u.ch.iter_mut().take(n) {
        c.cur ^= 1;
    }
    u.wcur ^= 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every Huffman code set is a complete prefix code and its symbols fit their use.
    #[test]
    fn code_tables_are_complete() {
        let t = tables();
        let kraft = |v: &Vlc| {
            let mut seen = std::collections::BTreeMap::new();
            for &(s, l) in &v.table {
                assert!(l > 0, "incomplete code");
                seen.insert(s, l);
            }
            seen.values().map(|&l| 1.0 / f64::from(1u32 << l)).sum::<f64>()
        };
        for v in t.wl.iter().chain(&t.ct).chain(&t.sf).chain(&t.spec).chain(&t.gain).chain(&t.tone) {
            let _ = kraft(v);
        }
        // the xlat tables are used up exactly
        let used = |cbs: &[u8], n: usize| -> usize { cbs.iter().take(n * 12).map(|&c| usize::from(c)).sum() };
        assert_eq!(used(&t::WL_CBS, 4) + used(&t::CT_CBS, 4), t::WL_CT_XLATS.len());
        assert_eq!(used(&t::SF_CBS, 8), t::SF_XLATS.len());
        assert_eq!(used(&t::GAIN_CBS, 11), t::GAIN_XLATS.len());
        assert_eq!(used(&t::TONE_CBS, 7), t::TONE_XLATS.len());
        let spec: usize = t::SPECTRA_CBS.chunks(12).filter(|r| r[0] < 128).flat_map(|r| r.iter().map(|&c| usize::from(c))).sum();
        // FFmpeg's spectrum symbol list is longer than its codes use
        assert!(spec <= t::SPECTRA_XLATS.len());
        let max = |v: &[Vlc]| v.iter().flat_map(|v| v.table.iter().map(|e| e.0)).max().unwrap();
        assert!(max(&t.ct) < 8);
        assert!(max(&t.gain[..1]) <= 7);
        assert!(max(&t.gain[2..3]) <= 15);
        assert!(max(&t.tone[..1]) < 16);
        assert!(max(&t.tone[3..4]) + 20 < 64 && max(&t.tone[4..5]) + 24 < 64);
    }

    #[test]
    fn bad_parameters_are_refused() {
        for ch in [0u16, 5, 9] {
            assert!(Decoder::new(ch, 1024).is_err());
        }
        assert!(Decoder::new(2, 0).is_err());
        assert!(Decoder::new(2, MAX_BLOCK_ALIGN + 1).is_err());
        let mut d = Decoder::new(2, 1024).unwrap();
        assert!(d.decode(&[]).is_err());
        assert!(d.decode(&[0x80; 1024]).is_err());
    }

    #[test]
    fn random_frames_do_not_panic() {
        let mut x = 0x2468_ACE1u32;
        let mut rnd = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        for (ch, ba) in [(1u16, 192usize), (2, 1488), (2, 280), (3, 600), (4, 800), (6, 1200), (7, 1400), (8, 1600)] {
            let mut d = Decoder::new(ch, ba).unwrap();
            let mut ok = 0;
            for i in 0..1500 {
                let mut f: Vec<u8> = (0..ba).map(|_| rnd() as u8).collect();
                // a valid start bit and first unit id lets decoding go deeper
                if i % 4 != 0 {
                    f[0] = (f[0] & 0x1F) | if ch == 1 { 0 } else { 0x20 };
                }
                if let Ok(p) = d.decode(&f) {
                    ok += 1;
                    assert_eq!(p.len(), usize::from(ch));
                    assert!(p.iter().all(|c| c.len() == FRAME_SAMPLES && c.iter().all(|v| v.is_finite())));
                }
            }
            assert!(ok > 0, "{ch} channels: no frame decoded");
        }
    }
}
