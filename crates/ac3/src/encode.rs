//! Clean-room AC-3 and E-AC-3 encoder (ATSC A/52:2012): the decoder's syntax and bit allocation run
//! forwards.
//!
//! - Forward MDCT as the exact transpose of the decoder's FFT-based IMDCT (512-sample long
//!   blocks, the Kaiser-Bessel derived window), scaled for perfect reconstruction.
//! - Exponents (§7.1): per block, shared across blocks while the spectrum stays alike (the set
//!   holds each bin's smallest exponent), D15 / D25 / D45 by the bits per channel, differences
//!   limited to ±2.
//! - Rematrixing for 2/0 (§7.5) per band where the sum and difference are cheaper.
//! - The parametric bit allocation (§7.2) is the decoder's own; one SNR offset per syncframe,
//!   the largest whose mantissas fit the frame (and blocks 0-1 its first 5/8, §6.1.3).
//! - Symmetric grouped (3-, 5-, 11-level) and 7- / 15-level mantissas, asymmetric ones up to 16
//!   bits (§7.3).
//! - Syncinfo, bsi, audio blocks, auxiliary data and both CRC words (§5.4, §7.10).
//!
//! E-AC-3 (Annex E): bsid 16, one independent substream, six blocks, per-block exponent
//! strategies, frame SNR offsets and the defaults for everything else.
//!
//! Not written (#120): coupling, block switching, spectral extension, 7.1, dynamic range words,
//! 1+1 mode.

use crate::{AllocParams, BITRATES, Error, FASTGAIN, Globals, NFCHANS, QNTZTAB, Result, bit_allocation, ifft, twiddles, window};

/// Samples per channel in a syncframe.
pub const FRAME_SAMPLES: usize = 1536;

/// Bit allocation parameters written in block 0 (the A/52 encoder defaults).
const GLOBALS: (usize, usize, usize, usize, usize) = (2, 1, 1, 2, 7);
/// Fast gain code of every channel (Table 7.11: 0x300).
const FGAINCOD: usize = 4;
/// Rematrixing bands (§7.5.2): bins 13-24, 25-36, 37-60, 61-252.
const REMAT: [usize; 5] = [13, 25, 37, 61, 253];

/// MDCT of 512 samples (the previous block, then this one): the transpose of the decoder's
/// long-block IMDCT (§7.9.4.1), step by step in reverse — window and output permutation, the
/// post-twiddle by the conjugate, a forward FFT (the transpose of the inverse one), the
/// pre-twiddle by the conjugate — divided by 256 so that the decoder's overlap-add (which
/// doubles) gives the input back.
fn mdct(x: &[f32; 512], out: &mut [f32; 256]) {
    const N: usize = 512;
    let w = window();
    let t = twiddles();
    let (mut yr, mut yi) = ([0f32; 128], [0f32; 128]);
    for n in 0..N / 8 {
        yi[N / 8 + n] -= x[2 * n] * w[2 * n];
        yr[N / 8 - n - 1] += x[2 * n + 1] * w[2 * n + 1];
        yr[n] -= x[N / 4 + 2 * n] * w[N / 4 + 2 * n];
        yi[N / 4 - n - 1] += x[N / 4 + 2 * n + 1] * w[N / 4 + 2 * n + 1];
        yr[N / 8 + n] -= x[N / 2 + 2 * n] * w[N / 2 - 2 * n - 1];
        yi[N / 8 - n - 1] += x[N / 2 + 2 * n + 1] * w[N / 2 - 2 * n - 2];
        yi[n] += x[3 * N / 4 + 2 * n] * w[N / 4 - 2 * n - 1];
        yr[N / 4 - n - 1] -= x[3 * N / 4 + 2 * n + 1] * w[N / 4 - 2 * n - 2];
    }
    // post-twiddle transposed, then the forward FFT as conj(IFFT(conj))
    let (mut zr, mut zi) = ([0f32; 128], [0f32; 128]);
    for k in 0..N / 4 {
        let (c, s) = (t.c1[k], t.s1[k]);
        zr[k] = yr[k] * c + yi[k] * s;
        zi[k] = -(yi[k] * c - yr[k] * s);
    }
    ifft(&mut zr, &mut zi);
    for k in 0..N / 4 {
        let (r, i) = (zr[k], -zi[k]);
        let (c, s) = (t.c1[k], t.s1[k]);
        out[N / 2 - 2 * k - 1] = (r * c + i * s) / 256.0;
        out[2 * k] = (i * c - r * s) / 256.0;
    }
}

/// MSB-first bit writer that can fill in fields later.
struct Writer {
    out: Vec<u8>,
    bits: usize,
}

impl Writer {
    fn new(bytes: usize) -> Self {
        Writer { out: vec![0; bytes], bits: 0 }
    }
    fn put(&mut self, v: u32, n: u32) {
        for k in (0..n).rev() {
            self.set(self.bits, (v >> k) & 1);
            self.bits += 1;
        }
    }
    fn set(&mut self, at: usize, b: u32) {
        if let Some(byte) = self.out.get_mut(at >> 3) {
            let mask = 0x80u8 >> (at & 7);
            if b == 1 {
                *byte |= mask;
            } else {
                *byte &= !mask;
            }
        }
    }
    fn patch(&mut self, at: usize, v: u32, n: u32) {
        for k in 0..n {
            self.set(at + k as usize, (v >> (n - 1 - k)) & 1);
        }
    }
}

/// The exponent smallest enough for `x`: |x|·2^e in [0.5, 1), 0 for |x| ≥ 0.5 and 24 at most.
fn exponent(x: f32) -> u8 {
    let a = x.abs();
    if a < 2f32.powi(-24) {
        return 24;
    }
    let mut e = 0u8;
    let mut v = a;
    while v < 0.5 && e < 24 {
        v *= 2.0;
        e += 1;
    }
    e
}

/// An exponent set as coded: the absolute exponent, the 7-bit groups, and the exponents the
/// decoder reconstructs (never above `raw`, so every mantissa still fits).
struct Exponents {
    abs: u8,
    groups: Vec<u8>,
    exps: [u8; 256],
}

/// Codes `raw[..end]` with group size `g` (1, 2, 4) the way §7.1.3 decodes it.
fn code_exponents(raw: &[u8; 256], end: usize, g: usize) -> Exponents {
    let ngrps = (end.max(1) - 1 + 3 * g - 3) / (3 * g);
    let n = 1 + 3 * ngrps;
    let mut v = vec![24u8; n];
    v[0] = raw[0].min(15);
    for (j, slot) in v.iter_mut().enumerate().skip(1) {
        let first = 1 + (j - 1) * g;
        *slot = (first..first + g).filter(|&b| b < end).map(|b| raw[b]).min().unwrap_or(24);
    }
    // differences within ±2: lower a value until its successor is close enough, then its
    // predecessor (lowering only adds headroom)
    for i in (0..n - 1).rev() {
        v[i] = v[i].min(v[i + 1] + 2);
    }
    for i in 1..n {
        v[i] = v[i].min(v[i - 1] + 2);
    }
    let mut groups = Vec::with_capacity(ngrps);
    for j in 0..ngrps {
        let d = |i: usize| (i32::from(v[i]) - i32::from(v[i - 1]) + 2) as u8;
        groups.push(25 * d(3 * j + 1) + 5 * d(3 * j + 2) + d(3 * j + 3));
    }
    let mut exps = [24u8; 256];
    exps[0] = v[0];
    for (j, &e) in v.iter().enumerate().skip(1) {
        let first = 1 + (j - 1) * g;
        for b in first..(first + g).min(256) {
            exps[b] = e;
        }
    }
    Exponents { abs: v[0], groups, exps }
}

/// A mantissa code for `m` in (-1, 1) at allocation `bap` (the decoder's `mantissa` inverted).
fn quantize(m: f32, bap: u8) -> u32 {
    let sym = |levels: i32| ((m * levels as f32 + (levels - 1) as f32) / 2.0).round().clamp(0.0, (levels - 1) as f32) as u32;
    match bap {
        1 => sym(3),
        2 => sym(5),
        3 => sym(7),
        4 => sym(11),
        5 => sym(15),
        _ => {
            let bits = QNTZTAB[bap as usize & 15];
            let half = 1i32 << (bits - 1);
            let v = (m * half as f32).round().clamp(-half as f32, (half - 1) as f32) as i32;
            (v as u32) & ((1u32 << bits) - 1)
        }
    }
}

/// Grouped mantissas of one block: the position of the open group and the codes so far.
#[derive(Default)]
struct Open {
    at: usize,
    codes: Vec<u32>,
}

/// Writes mantissa code `q` at allocation `bap`, grouping as §7.3.5 reads them.
fn put_mantissa(w: &mut Writer, groups: &mut [Open; 3], bap: u8, q: u32) {
    let (slot, size, bits, base) = match bap {
        1 => (0, 3, 5, 3),
        2 => (1, 3, 7, 5),
        4 => (2, 2, 7, 11),
        3 => return w.put(q, 3),
        5 => return w.put(q, 4),
        _ => return w.put(q, QNTZTAB[bap as usize & 15]),
    };
    let g = &mut groups[slot];
    if g.codes.is_empty() {
        g.at = w.bits;
        w.put(0, bits);
    }
    g.codes.push(q);
    let mut v = 0;
    for k in 0..size {
        v = v * base + g.codes.get(k).copied().unwrap_or(0);
    }
    w.patch(g.at, v, bits);
    if g.codes.len() == size {
        g.codes.clear();
    }
}

/// CRC-16 with x^16 + x^15 + x^2 + 1 (§7.10.1), MSB first, from `crc`.
fn crc16(mut crc: u16, data: &[u8]) -> u16 {
    for &b in data {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x8005 } else { crc << 1 };
        }
    }
    crc
}

/// The value of the first two bytes of `data` (now zero) that makes its CRC zero: the CRC is
/// linear in them, so the 16 unit images are solved for the CRC of the rest.
fn crc_prefix(data: &mut [u8]) -> u16 {
    let target = crc16(0, data);
    let mut basis = [(0u16, 0u16); 16];
    let mut probe = vec![0u8; data.len()];
    for (i, b) in basis.iter_mut().enumerate() {
        let v = 1u16 << i;
        if let Some(p) = probe.get_mut(..2) {
            p.copy_from_slice(&v.to_be_bytes());
        }
        *b = (crc16(0, &probe), v);
    }
    // Gaussian elimination over GF(2): rows (image, preimage)
    let mut rows: Vec<(u16, u16)> = basis.to_vec();
    let mut solution = 0u16;
    let mut want = target;
    for bit in (0..16).rev() {
        let mask = 1u16 << bit;
        let Some(p) = rows.iter().position(|r| r.0 & mask != 0) else { continue };
        let pivot = rows.swap_remove(p);
        for r in &mut rows {
            if r.0 & mask != 0 {
                r.0 ^= pivot.0;
                r.1 ^= pivot.1;
            }
        }
        if want & mask != 0 {
            want ^= pivot.0;
            solution ^= pivot.1;
        }
    }
    solution
}

/// The plan of one syncframe: transmitted coefficients and exponent sets (all but the bit
/// allocation, which depends on the SNR offset searched for).
struct Plan {
    /// Per coded channel (fbw, then LFE), per block.
    coefs: Vec<[[f32; 256]; 6]>,
    /// Exponent strategy per channel per block (0 reuse, 1-3 D15 / D25 / D45; LFE 0 / 1).
    strategy: Vec<[u8; 6]>,
    sets: Vec<[Option<Exponents>; 6]>,
    /// The exponents in force in each block.
    exps: Vec<[[u8; 256]; 6]>,
    rematflg: [[bool; 4]; 6],
}

/// An AC-3 encoder: 1536 samples per channel in, one syncframe out. The output lags the input
/// by 256 samples (the transform overlap).
pub struct Encoder {
    rate: u32,
    /// E-AC-3 (bsid 16) rather than AC-3 (bsid 8).
    eac3: bool,
    fscod: u8,
    frmsizecod: u8,
    frame_bytes: usize,
    acmod: usize,
    lfe: bool,
    /// Coded channel `c` takes input channel `map[c]` (WAV order in).
    map: Vec<usize>,
    /// The last 256 input samples of each coded channel.
    tail: Vec<[f32; 256]>,
    /// Input waiting for a whole syncframe, per input channel.
    pending: Vec<Vec<f32>>,
    endmant: usize,
}

impl Encoder {
    /// An encoder for `channels` (1 mono, 2 stereo, 3 L R C, 4 L R Ls Rs, 5 L R C Ls Rs, 6 5.1:
    /// L R C LFE Ls Rs) at `sample_rate` (48 000, 44 100 or 32 000) and `bitrate_kbps` (32-640, a
    /// value of Table 5.18).
    pub fn new(sample_rate: u32, channels: usize, bitrate_kbps: u32) -> Result<Encoder> {
        Self::with(false, sample_rate, channels, bitrate_kbps)
    }

    /// An E-AC-3 (Dolby Digital Plus) encoder: six-block syncframes of one independent
    /// substream, at any bit rate from 32 to 1024 kb/s (the syncframe holds at most 2048 words).
    /// Channels and rates as [`Encoder::new`].
    pub fn new_eac3(sample_rate: u32, channels: usize, bitrate_kbps: u32) -> Result<Encoder> {
        Self::with(true, sample_rate, channels, bitrate_kbps)
    }

    fn with(eac3: bool, sample_rate: u32, channels: usize, bitrate_kbps: u32) -> Result<Encoder> {
        let fscod: u8 = match sample_rate {
            48_000 => 0,
            44_100 => 1,
            32_000 => 2,
            _ => return Err(Error::Unsupported(format!("AC-3 sample rate {sample_rate}"))),
        };
        let idx = match BITRATES.iter().position(|&b| b == bitrate_kbps) {
            Some(i) => i,
            None if eac3 => 0,
            None => return Err(Error::Unsupported(format!("AC-3 bit rate {bitrate_kbps} kb/s"))),
        };
        let (acmod, lfe, map): (usize, bool, Vec<usize>) = match channels {
            1 => (1, false, vec![0]),
            2 => (2, false, vec![0, 1]),
            3 => (3, false, vec![0, 2, 1]),
            4 => (6, false, vec![0, 1, 2, 3]),
            5 => (7, false, vec![0, 2, 1, 3, 4]),
            6 => (7, true, vec![0, 2, 1, 4, 5, 3]),
            _ => return Err(Error::Unsupported(format!("{channels} channels for AC-3"))),
        };
        let words = if eac3 {
            // 1536 samples per syncframe, 16 bits per word
            let w = u64::from(bitrate_kbps) * 1000 * 1536 / u64::from(sample_rate) / 16;
            if !(32..=1024).contains(&bitrate_kbps) || w > 2048 {
                return Err(Error::Unsupported(format!("E-AC-3 bit rate {bitrate_kbps} kb/s")));
            }
            w as u32
        } else {
            match fscod {
                0 => bitrate_kbps * 2,
                1 => bitrate_kbps * 96_000 / 44_100,
                _ => bitrate_kbps * 3,
            }
        };
        let nf = NFCHANS[acmod];
        // bits per full-bandwidth channel and syncframe decide bandwidth and exponent precision
        let per = (words as usize * 16) / nf.max(1);
        let bw = match per {
            0..2200 => 30,
            2200..3500 => 42,
            3500..5000 => 52,
            _ => 60,
        };
        Ok(Encoder {
            rate: sample_rate,
            eac3,
            fscod,
            frmsizecod: (idx * 2) as u8,
            frame_bytes: words as usize * 2,
            acmod,
            lfe,
            tail: vec![[0.0; 256]; map.len()],
            map,
            pending: vec![Vec::new(); channels],
            endmant: (bw + 12) * 3 + 37,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.rate
    }

    /// Bytes of every syncframe.
    pub fn frame_bytes(&self) -> usize {
        self.frame_bytes
    }

    /// Encodes planar samples (full scale ±1, one slice per channel, equal lengths): the
    /// syncframes completed.
    pub fn encode(&mut self, pcm: &[&[f32]]) -> Result<Vec<Vec<u8>>> {
        if pcm.len() != self.pending.len() {
            return Err(Error::Invalid("channel count differs from the encoder's"));
        }
        for (p, s) in self.pending.iter_mut().zip(pcm) {
            // NaN and infinities would poison the transform's overlap
            p.extend(s.iter().map(|&v| if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 }));
        }
        self.drain()
    }

    /// Pads what is pending with silence and encodes it, plus the transform's 256-sample tail.
    pub fn flush(&mut self) -> Result<Vec<Vec<u8>>> {
        let n = self.pending.iter().map(Vec::len).max().unwrap_or(0) + 256;
        let pad = n.div_ceil(FRAME_SAMPLES) * FRAME_SAMPLES;
        for p in &mut self.pending {
            p.resize(pad, 0.0);
        }
        self.drain()
    }

    /// Encodes every whole syncframe pending, then drops those samples in one move.
    fn drain(&mut self) -> Result<Vec<Vec<u8>>> {
        let n = self.pending.iter().map(Vec::len).min().unwrap_or(0) / FRAME_SAMPLES;
        let mut out = Vec::with_capacity(n);
        for k in 0..n {
            let frame: Vec<Vec<f32>> = self.pending.iter().map(|p| p.get(k * FRAME_SAMPLES..(k + 1) * FRAME_SAMPLES).unwrap_or_default().to_vec()).collect();
            out.push(self.frame(&frame)?);
        }
        for p in &mut self.pending {
            p.drain(..(n * FRAME_SAMPLES).min(p.len()));
        }
        Ok(out)
    }

    fn nf(&self) -> usize {
        NFCHANS[self.acmod]
    }

    /// One syncframe from 1536 samples of each input channel.
    fn frame(&mut self, pcm: &[Vec<f32>]) -> Result<Vec<u8>> {
        let (mut plan, tail) = self.plan(pcm, false);
        if self.write(&plan, 0, true).is_none() {
            // exponents too costly for the frame: one coarse set per channel
            plan = self.plan(pcm, true).0;
        }
        self.tail = tail;
        // the largest SNR offset that fits (bits grow with it)
        let (mut lo, mut hi) = (0u32, 1024u32);
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if self.write(&plan, mid, true).is_some() {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let mut out = self.write(&plan, lo, false).ok_or(Error::Invalid("syncframe too small for its side information"))?;
        // CRC words: crc1 zeroes the first 5/8 (after the sync word), crc2 the rest; E-AC-3 has
        // only crc2, over the whole syncframe
        let n = self.frame_bytes;
        let f58 = if self.eac3 { 2 } else { ((n >> 2) + (n >> 4)) << 1 };
        if !self.eac3 {
            let c1 = out.get_mut(2..f58).map(crc_prefix).unwrap_or(0);
            if let Some(b) = out.get_mut(2..4) {
                b.copy_from_slice(&c1.to_be_bytes());
            }
        }
        let c2 = out.get(f58..n - 2).map_or(0, |d| crc16(0, d));
        if let Some(b) = out.get_mut(n - 2..n) {
            b.copy_from_slice(&c2.to_be_bytes());
        }
        Ok(out)
    }

    /// Transforms, rematrixes and chooses exponents (with `coarse`, one D45 set per channel);
    /// also the last 256 samples of each channel, the next frame's overlap.
    fn plan(&self, pcm: &[Vec<f32>], coarse: bool) -> (Plan, Vec<[f32; 256]>) {
        let nf = self.nf();
        let nch = self.map.len();
        let mut coefs = vec![[[0f32; 256]; 6]; nch];
        let mut tail = Vec::with_capacity(nch);
        for (c, &input) in self.map.iter().enumerate() {
            let src = pcm.get(input).map_or(&[][..], Vec::as_slice);
            let mut win = [0f32; 512];
            let mut prev = self.tail[c];
            for blk in 0..6 {
                win[..256].copy_from_slice(&prev);
                for (n, w) in win[256..].iter_mut().enumerate() {
                    *w = src.get(blk * 256 + n).copied().unwrap_or(0.0);
                }
                mdct(&win, &mut coefs[c][blk]);
                prev.copy_from_slice(&win[256..]);
            }
            tail.push(prev);
        }
        let end = |c: usize| if c < nf { self.endmant } else { 7 };
        // rematrixing: sum and difference in the bands where they carry less energy, decided
        // for the whole frame so that the exponents can be shared across its blocks
        let mut rematflg = [[false; 4]; 6];
        if self.acmod == 2 {
            for k in 0..4 {
                let range = REMAT[k]..REMAT[k + 1].min(self.endmant);
                let (mut el, mut er, mut em, mut es) = (0f32, 0f32, 0f32, 0f32);
                for blk in 0..6 {
                    for b in range.clone() {
                        let (l, r) = (coefs[0][blk][b], coefs[1][blk][b]);
                        el += l * l;
                        er += r * r;
                        em += (l + r) * (l + r) / 4.0;
                        es += (l - r) * (l - r) / 4.0;
                    }
                }
                if em.min(es) < el.min(er) {
                    for (blk, flags) in rematflg.iter_mut().enumerate() {
                        flags[k] = true;
                        for b in range.clone() {
                            let (l, r) = (coefs[0][blk][b], coefs[1][blk][b]);
                            coefs[0][blk][b] = (l + r) / 2.0;
                            coefs[1][blk][b] = (l - r) / 2.0;
                        }
                    }
                }
            }
        }
        // exponents: a new set when the spectrum moves, holding the smallest exponent of the
        // blocks it covers
        let mut strategy = vec![[0u8; 6]; nch];
        let mut sets: Vec<[Option<Exponents>; 6]> = (0..nch).map(|_| Default::default()).collect();
        let mut exps = vec![[[24u8; 256]; 6]; nch];
        for c in 0..nch {
            let e = end(c);
            let raw: Vec<[u8; 256]> = (0..6)
                .map(|blk| {
                    let mut r = [24u8; 256];
                    for (b, slot) in r.iter_mut().enumerate().take(e) {
                        *slot = exponent(coefs[c][blk][b]);
                    }
                    r
                })
                .collect();
            let mut start = 0;
            while start < 6 {
                let mut stop = start + 1;
                while stop < 6 && coarse {
                    stop += 1;
                }
                while stop < 6 {
                    // how far the audible part of the spectrum moved (bins below about -90 dB
                    // get no bits either way)
                    let diff: u32 = (0..e).filter(|&b| raw[stop][b].min(raw[start][b]) < 15).map(|b| u32::from(raw[stop][b].abs_diff(raw[start][b]))).sum();
                    if diff as usize > e {
                        break;
                    }
                    stop += 1;
                }
                let mut merged = [24u8; 256];
                for r in &raw[start..stop] {
                    for (m, &v) in merged.iter_mut().zip(r.iter()) {
                        *m = (*m).min(v);
                    }
                }
                // finer exponents for sets that last: D45 for one block, D25 for two or three,
                // D15 beyond
                let strat: u8 = match (c < nf, stop - start) {
                    (false, _) => 1,
                    (true, _) if coarse => 3,
                    (true, 1) => 3,
                    (true, 2 | 3) => 2,
                    _ => 1,
                };
                let coded = code_exponents(&merged, e, 1usize << (strat - 1));
                for blk in start..stop {
                    exps[c][blk] = coded.exps;
                }
                strategy[c][start] = strat;
                sets[c][start] = Some(coded);
                start = stop;
            }
        }
        (Plan { coefs, strategy, sets, exps, rematflg }, tail)
    }

    /// AC-3 syncinfo and bsi (§5.3.1, §5.3.2), the CRC word zero.
    fn ac3_header(&self, w: &mut Writer) {
        w.put(0x0B77, 16);
        w.put(0, 16);
        w.put(u32::from(self.fscod), 2);
        w.put(u32::from(self.frmsizecod), 6);
        w.put(8, 5); // bsid
        w.put(0, 3); // bsmod: complete main
        w.put(self.acmod as u32, 3);
        if self.acmod & 1 != 0 && self.acmod != 1 {
            w.put(0, 2); // cmixlev -3 dB
        }
        if self.acmod & 4 != 0 {
            w.put(0, 2); // surmixlev -3 dB
        }
        if self.acmod == 2 {
            w.put(0, 2); // dsurmod
        }
        w.put(u32::from(self.lfe), 1);
        w.put(31, 5); // dialnorm -31 dB: no attenuation
        w.put(0, 3); // compre, langcode, audprodie
        w.put(0b01, 2); // copyrightb, origbs
        w.put(0, 3); // timecod1e, timecod2e, addbsie
    }

    /// E-AC-3 syncinfo, bsi and audfrm (§E2.2.1-E2.2.3): independent substream 0, six blocks,
    /// per-block exponent strategies, frame SNR offsets, everything else at its default.
    fn eac3_header(&self, w: &mut Writer, plan: &Plan, csnr: u32, fsnr: u32) {
        let nf = self.nf();
        w.put(0x0B77, 16);
        w.put(0, 2); // strmtyp: independent
        w.put(0, 3); // substreamid
        w.put((self.frame_bytes / 2 - 1) as u32, 11); // frmsiz
        w.put(u32::from(self.fscod), 2);
        w.put(3, 2); // numblkscod: six blocks
        w.put(self.acmod as u32, 3);
        w.put(u32::from(self.lfe), 1);
        w.put(16, 5); // bsid
        w.put(31, 5); // dialnorm
        w.put(0, 1); // compre
        w.put(0, 3); // mixmdate, infomdate, addbsie
        // audfrm
        w.put(0b10, 2); // expstre, ahte
        w.put(0, 2); // snroffststr: frame offsets
        w.put(0, 7); // transproce, blkswe, dithflage, bamode, frmfgaincode, dbaflde, skipflde
        w.put(0, 1); // spxattene
        if self.acmod > 1 {
            w.put(0, 6); // cplinu in block 0, cplstre in blocks 1-5
        }
        for blk in 0..6 {
            for c in 0..nf {
                w.put(u32::from(plan.strategy[c][blk]), 2);
            }
        }
        if self.lfe {
            for blk in 0..6 {
                w.put(u32::from(plan.strategy[nf][blk]), 1);
            }
        }
        w.put(0, 5 * nf as u32); // convexpstr
        w.put(csnr, 6);
        w.put(fsnr, 4);
        w.put(0, 1); // blkstrtinfoe
    }

    /// The syncframe at SNR offset `snr` (csnroffst · 16 + fsnroffst), CRC words zero; `None`
    /// when it does not fit. With `count`, the mantissas are only counted (for the search).
    fn write(&self, plan: &Plan, snr: u32, count: bool) -> Option<Vec<u8>> {
        let nf = self.nf();
        let nch = self.map.len();
        let n = self.frame_bytes;
        let mut w = Writer::new(n);
        let (csnr, fsnr) = (snr >> 4, snr & 15);
        if self.eac3 {
            self.eac3_header(&mut w, plan, csnr, fsnr);
        } else {
            self.ac3_header(&mut w);
        }
        let g =
            Globals { fscod: usize::from(self.fscod), sdcycod: GLOBALS.0, fdcycod: GLOBALS.1, sgaincod: GLOBALS.2, dbpbcod: GLOBALS.3, floorcod: GLOBALS.4 };
        let offset = ((csnr as i32 - 15) << 4) + fsnr as i32;
        let f58 = ((n >> 2) + (n >> 4)) << 4;
        let eac3 = self.eac3;
        for blk in 0..6 {
            if !eac3 {
                (0..nf).for_each(|_| w.put(0, 1)); // blksw
                (0..nf).for_each(|_| w.put(1, 1)); // dithflag
            }
            w.put(0, 1); // dynrnge
            if eac3 {
                // spectral extension off (spxinu in block 0, then spxstre 0)
                w.put(0, 1);
            } else if blk == 0 {
                w.put(0b10, 2); // cplstre, cplinu off
            } else {
                w.put(0, 1);
            }
            if self.acmod == 2 {
                // E-AC-3 sends the flags in block 0 without rematstr
                if !(eac3 && blk == 0) {
                    w.put(1, 1); // rematstr
                }
                plan.rematflg[blk].iter().for_each(|&f| w.put(u32::from(f), 1));
            }
            if !eac3 {
                for c in 0..nf {
                    w.put(u32::from(plan.strategy[c][blk]), 2);
                }
                if self.lfe {
                    w.put(u32::from(plan.strategy[nf][blk]), 1);
                }
            }
            for c in 0..nf {
                if plan.strategy[c][blk] != 0 {
                    w.put(((self.endmant - 37) / 3 - 12) as u32, 6); // chbwcod
                }
            }
            for c in 0..nch {
                if let Some(set) = &plan.sets[c][blk] {
                    w.put(u32::from(set.abs), 4);
                    set.groups.iter().for_each(|&gr| w.put(u32::from(gr), 7));
                    if c < nf {
                        w.put(0, 2); // gainrng
                    }
                }
            }
            if eac3 {
                // bit allocation defaults, frame SNR offsets, fast gain 4; convsnroffste
                w.put(0, 1);
            } else if blk == 0 {
                w.put(1, 1); // baie
                w.put(GLOBALS.0 as u32, 2);
                w.put(GLOBALS.1 as u32, 2);
                w.put(GLOBALS.2 as u32, 2);
                w.put(GLOBALS.3 as u32, 2);
                w.put(GLOBALS.4 as u32, 3);
                w.put(1, 1); // snroffste
                w.put(csnr, 6);
                for _ in 0..nch {
                    w.put(fsnr, 4);
                    w.put(FGAINCOD as u32, 3);
                }
            } else {
                w.put(0, 2); // baie, snroffste
            }
            if !eac3 {
                w.put(0, 2); // deltbaie, skiple
            }
            // mantissas
            let mut groups: [Open; 3] = Default::default();
            // counted mantissas of bap 1, 2 and 4
            let mut grouped = [0usize; 3];
            for c in 0..nch {
                let end = if c < nf { self.endmant } else { 7 };
                let exps = &plan.exps[c][blk];
                let mut bap = [0u8; 256];
                if snr > 0 {
                    let p = AllocParams { start: 0, end, fgain: FASTGAIN[FGAINCOD], snroffset: offset << 2, leak: None, dba: None };
                    bit_allocation(exps, &p, &g, &mut bap);
                }
                for b in 0..end {
                    match (bap[b], count) {
                        (0, _) => {}
                        (1 | 2 | 4, true) => grouped[usize::from(bap[b] >> 1)] += 1,
                        (k, true) => w.bits += QNTZTAB[usize::from(k & 15)] as usize,
                        (k, false) => {
                            let m = plan.coefs[c][blk][b] * 2f32.powi(i32::from(exps[b]));
                            put_mantissa(&mut w, &mut groups, k, quantize(m, k));
                        }
                    }
                }
            }
            // counted groups: three 3-level codes in 5 bits, three 5-level in 7, two 11-level in 7
            w.bits += grouped[0].div_ceil(3) * 5 + grouped[1].div_ceil(3) * 7 + grouped[2].div_ceil(2) * 7;
            if !eac3 && blk == 1 && w.bits > f58 {
                return None;
            }
            if w.bits + 18 > n * 8 {
                return None;
            }
        }
        // auxdata: none (auxdatae 0), crcrsv 0, crc2 later; the gap stays zero
        Some(w.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Decoder;

    fn tone(rate: u32, n: usize, f: f32, amp: f32, phase: f32) -> Vec<f32> {
        (0..n).map(|i| amp * (2.0 * std::f32::consts::PI * f * i as f32 / rate as f32 + phase).sin()).collect()
    }

    /// Encodes and decodes `input` (WAV order): the decode, aligned with the input.
    fn round_trip(rate: u32, kbps: u32, input: &[Vec<f32>]) -> Vec<Vec<f32>> {
        round_trip_with(Encoder::new(rate, input.len(), kbps).unwrap(), input)
    }

    fn round_trip_with(mut enc: Encoder, input: &[Vec<f32>]) -> Vec<Vec<f32>> {
        let (rate, eac3) = (enc.rate, enc.eac3);
        let refs: Vec<&[f32]> = input.iter().map(Vec::as_slice).collect();
        let mut frames = enc.encode(&refs).unwrap();
        frames.extend(enc.flush().unwrap());
        let mut dec = Decoder::new();
        let mut out = vec![Vec::new(); input.len()];
        for f in &frames {
            assert_eq!(f.len(), enc.frame_bytes());
            let h = crate::parse_header(f).unwrap();
            assert_eq!((h.sample_rate, h.channels(), h.is_eac3(), h.blocks), (rate, input.len(), eac3, 6));
            let d = dec.decode(f).unwrap();
            for (o, c) in out.iter_mut().zip(d.channels) {
                o.extend(c);
            }
        }
        out.iter().map(|c| c.get(256..256 + input[0].len()).unwrap().to_vec()).collect()
    }

    fn snr(a: &[f32], b: &[f32]) -> f64 {
        let s: f64 = a.iter().map(|&x| f64::from(x).powi(2)).sum();
        let e: f64 = a.iter().zip(b).map(|(&x, &y)| f64::from(x - y).powi(2)).sum();
        10.0 * (s / e.max(1e-30)).log10()
    }

    /// The fast MDCT is the transpose of the decoder's IMDCT: ⟨mdct(x), y⟩ = ⟨x, imdct(y)⟩ / 256.
    #[test]
    fn mdct_is_the_imdct_transposed() {
        let mut seed = 5u32;
        let mut r = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        for k in 0..256 {
            let x: [f32; 512] = std::array::from_fn(|_| r());
            let mut e = [0f32; 256];
            e[k] = 1.0;
            let mut col = [0f32; 512];
            crate::imdct(&e, false, &mut col);
            let want: f32 = col.iter().zip(&x).map(|(a, b)| a * b).sum::<f32>() / 256.0;
            let mut got = [0f32; 256];
            mdct(&x, &mut got);
            assert!((got[k] - want).abs() < 1e-5, "bin {k}: {} vs {want}", got[k]);
        }
    }

    /// Counting the mantissas decides fit exactly as writing them.
    #[test]
    fn counted_and_written_frames_agree() {
        for (eac3, kbps) in [(false, 96), (false, 448), (true, 100)] {
            let mut enc = if eac3 { Encoder::new_eac3(48_000, 2, kbps) } else { Encoder::new(48_000, 2, kbps) }.unwrap();
            let pcm = vec![tone(48_000, 1536, 440.0, 0.5, 0.0), tone(48_000, 1536, 5000.0, 0.2, 1.0)];
            let (plan, _) = enc.plan(&pcm, false);
            for snr in (0..1024).step_by(7) {
                assert_eq!(enc.write(&plan, snr, true).is_some(), enc.write(&plan, snr, false).is_some(), "{kbps} kb/s, offset {snr}");
            }
            let _ = &mut enc;
        }
    }

    #[test]
    fn eac3_frames_decode() {
        for (rate, kbps, channels) in [(48_000, 192, 2), (44_100, 100, 1), (32_000, 64, 2), (48_000, 640, 6), (48_000, 1024, 6)] {
            let n = rate as usize / 2;
            let input: Vec<Vec<f32>> =
                (0..channels).map(|c| tone(rate, n, if channels == 6 && c == 3 { 60.0 } else { 300.0 + 170.0 * c as f32 }, 0.4, c as f32)).collect();
            let enc = Encoder::new_eac3(rate, channels, kbps).unwrap();
            let bytes = enc.frame_bytes();
            assert!(bytes as u64 * 8 * u64::from(rate) / 1536 <= u64::from(kbps) * 1000);
            let out = round_trip_with(enc, &input);
            for (c, (a, b)) in input.iter().zip(&out).enumerate() {
                let s = snr(a, b);
                assert!(s > 35.0, "E-AC-3 {rate} Hz {kbps} kb/s {channels} ch, channel {c}: {s:.1} dB");
            }
        }
        let mut e = Encoder::new_eac3(48_000, 2, 96).unwrap();
        let x = tone(48_000, 1536, 440.0, 0.5, 0.0);
        let f = &e.encode(&[&x, &x]).unwrap()[0];
        assert_eq!(crc16(0, &f[2..]), 0);
        assert!(Encoder::new_eac3(48_000, 2, 2000).is_err());
        assert!(Encoder::new_eac3(48_000, 2, 16).is_err());
    }

    #[test]
    fn crc_words_check_both_parts() {
        let mut enc = Encoder::new(48_000, 2, 192).unwrap();
        let x = tone(48_000, 1536, 440.0, 0.5, 0.0);
        let f = &enc.encode(&[&x, &x]).unwrap()[0];
        let n = f.len();
        let f58 = ((n >> 2) + (n >> 4)) << 1;
        assert_eq!(crc16(0, &f[2..f58]), 0);
        assert_eq!(crc16(0, &f[f58..]), 0);
        assert_eq!(crc16(0, &f[2..]), 0);
    }

    #[test]
    fn tones_survive_every_channel_mode_and_rate() {
        for (rate, kbps, channels, min) in [
            (48_000, 192, 2, 45.0),
            (44_100, 96, 1, 40.0),
            (32_000, 128, 2, 40.0),
            (48_000, 256, 3, 40.0),
            (48_000, 320, 4, 40.0),
            (48_000, 384, 5, 40.0),
            (48_000, 448, 6, 40.0),
        ] {
            let n = rate as usize / 2;
            let input: Vec<Vec<f32>> = (0..channels)
                .map(|c| if channels == 6 && c == 3 { tone(rate, n, 60.0, 0.4, 0.0) } else { tone(rate, n, 300.0 + 170.0 * c as f32, 0.4, c as f32) })
                .collect();
            let out = round_trip(rate, kbps, &input);
            for (c, (a, b)) in input.iter().zip(&out).enumerate() {
                let s = snr(a, b);
                assert!(s > min, "{rate} Hz {kbps} kb/s {channels} ch, channel {c}: {s:.1} dB");
            }
        }
    }

    #[test]
    fn stereo_correlated_and_wideband() {
        let rate = 48_000;
        let n = 24_000;
        let mut seed = 1u32;
        let noise: Vec<f32> = (0..n)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            })
            .collect();
        // nearly identical channels: rematrixing
        let l: Vec<f32> = tone(rate, n, 1000.0, 0.5, 0.0).iter().zip(&noise).map(|(a, b)| a + 0.05 * b).collect();
        let r: Vec<f32> = l.iter().map(|v| v * 0.95).collect();
        let out = round_trip(rate, 192, &[l.clone(), r.clone()]);
        assert!(snr(&l, &out[0]) > 25.0 && snr(&r, &out[1]) > 25.0, "{} {}", snr(&l, &out[0]), snr(&r, &out[1]));
        // white noise at a low rate still decodes, with some SNR
        let out = round_trip(rate, 96, &[noise.clone(), noise.iter().map(|v| -v).collect()]);
        assert!(snr(&noise, &out[0]) > 1.0, "{}", snr(&noise, &out[0]));
        // full scale and beyond is clamped, not wrapped
        let loud: Vec<f32> = tone(rate, n, 50.0, 1.5, 0.0).iter().map(|v| v.clamp(-1.0, 1.0)).collect();
        let out = round_trip(rate, 192, &[loud.clone(), loud.clone()]);
        assert!(snr(&loud, &out[0]) > 30.0, "{}", snr(&loud, &out[0]));
    }

    #[test]
    fn silence_and_bad_settings() {
        let out = round_trip(48_000, 64, &[vec![0.0; 5000]]);
        assert!(out[0].iter().all(|v| v.abs() < 1e-4));
        assert!(Encoder::new(22_050, 2, 192).is_err());
        assert!(Encoder::new(48_000, 2, 100).is_err());
        assert!(Encoder::new(48_000, 7, 448).is_err());
        let mut e = Encoder::new(48_000, 2, 192).unwrap();
        assert!(e.encode(&[&[0.0; 10]]).is_err());
        // NaN and infinities do not panic
        let bad = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30];
        let x: Vec<f32> = (0..3000).map(|i| bad[i % 4]).collect();
        let frames = e.encode(&[&x, &x]).unwrap();
        assert!(frames.iter().all(|f| crate::Decoder::new().decode(f).is_ok()));
    }
}
