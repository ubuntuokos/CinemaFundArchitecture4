//! The Bink audio decoders: `binkaudio_rdft` (one interleaved stream through a real FFT) and
//! `binkaudio_dct` (channel pairs through a DCT-III). Output is 32-bit float, interleaved.

use crate::bits::Bits;
use crate::{Error, Result};

const MAX_CHANNELS: usize = 2;
const MAX_DCT_CHANNELS: usize = 6;
const CRITICAL_FREQS: [u32; 25] =
    [100, 200, 300, 400, 510, 630, 770, 920, 1080, 1270, 1480, 1720, 2000, 2320, 2700, 3150, 3700, 4400, 5300, 6400, 7700, 9500, 12000, 15500, 24500];
const RLE_LENGTH_TAB: [usize; 16] = [2, 3, 4, 5, 6, 8, 9, 10, 11, 12, 13, 14, 15, 16, 32, 64];

#[derive(Clone)]
pub struct Decoder {
    dct: bool,
    version_b: bool,
    first: bool,
    /// Channels of the stream.
    channels: usize,
    /// Channels per transform (1 for the RDFT variant, which is already interleaved).
    tx_channels: usize,
    frame_len: usize,
    overlap_len: usize,
    block_size: usize,
    num_bands: usize,
    root: f32,
    bands: [usize; 26],
    previous: Vec<Vec<f32>>,
    quant_table: [f32; 96],
}

/// In-place complex FFT (sign +1 is the inverse direction, no scaling).
fn fft(re: &mut [f64], im: &mut [f64], inverse: bool) {
    let n = re.len();
    let mut j = 0usize;
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
        let ang = 2.0 * std::f64::consts::PI / len as f64 * if inverse { 1.0 } else { -1.0 };
        let (wr, wi) = (ang.cos(), ang.sin());
        for s in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f64, 0.0f64);
            for k in 0..len / 2 {
                let (a, b) = (s + k, s + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let nr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = nr;
            }
        }
        len <<= 1;
    }
}

/// The inverse real FFT of `n` points from the bins `c[2k], c[2k+1]` (k = 0..=n/2), scaled.
fn irdft(c: &[f32], n: usize, scale: f64) -> Vec<f32> {
    let mut re = vec![0.0f64; n];
    let mut im = vec![0.0f64; n];
    for k in 0..=n / 2 {
        re[k] = c[2 * k] as f64;
        im[k] = c[2 * k + 1] as f64;
        if k > 0 && k < n / 2 {
            re[n - k] = re[k];
            im[n - k] = -im[k];
        }
    }
    fft(&mut re, &mut im, true);
    re.iter().map(|&v| (v * scale) as f32).collect()
}

/// A DCT-III of `n` points: `scale * (c0 + 2 * sum c[k] cos(pi k (2i+1) / 2n))`.
fn idct(c: &[f32], n: usize, scale: f64) -> Vec<f32> {
    let m = 2 * n;
    let mut re = vec![0.0f64; m];
    let mut im = vec![0.0f64; m];
    for k in 0..n {
        let a = std::f64::consts::PI * k as f64 / m as f64;
        re[k] = c[k] as f64 * a.cos();
        im[k] = c[k] as f64 * a.sin();
    }
    fft(&mut re, &mut im, true);
    (0..n).map(|i| (scale * (2.0 * re[i] - c[0] as f64)) as f32).collect()
}

fn get_float(gb: &mut Bits<'_>) -> f32 {
    let power = gb.get(5) as i32;
    let m = gb.get(23) as f32;
    let f = m * (2.0f32).powi(power - 23);
    if gb.bit() != 0 { -f } else { f }
}

impl Decoder {
    /// A decoder for a track of `sample_rate` Hz and `channels` channels. `revision` is the
    /// fourth byte of the file signature (`b` selects the old float format).
    pub fn new(sample_rate: u32, channels: u32, dct: bool, revision: u8) -> Result<Decoder> {
        let max = if dct { MAX_DCT_CHANNELS } else { MAX_CHANNELS };
        let channels = channels as usize;
        if channels < 1 || channels > max || sample_rate == 0 {
            return Err(Error::Unsupported("audio channels or rate"));
        }
        let mut frame_len_bits: u32 = if sample_rate < 22050 {
            9
        } else if sample_rate < 44100 {
            10
        } else {
            11
        };
        let version_b = revision == b'b';
        let mut rate = sample_rate as u64;
        let tx_channels;
        if !dct {
            rate *= channels as u64;
            tx_channels = 1;
            if !version_b {
                frame_len_bits += channels.ilog2();
            }
        } else {
            tx_channels = channels;
        }
        let frame_len = 1usize << frame_len_bits;
        let overlap_len = frame_len / 16;
        let block_size = (frame_len - overlap_len) * MAX_CHANNELS.min(tx_channels);
        let rate_half = rate.div_ceil(2) as u32;
        let root = if !dct { 2.0 / ((frame_len as f64).sqrt() * 32768.0) } else { frame_len as f64 / ((frame_len as f64).sqrt() * 32768.0) } as f32;
        let mut quant_table = [0.0f32; 96];
        for (i, q) in quant_table.iter_mut().enumerate() {
            *q = (i as f32 * 0.152_891_65_f32).exp() * root;
        }
        let mut num_bands = 1;
        while num_bands < 25 {
            if rate_half <= CRITICAL_FREQS[num_bands - 1] {
                break;
            }
            num_bands += 1;
        }
        let mut bands = [0usize; 26];
        bands[0] = 2;
        for i in 1..num_bands {
            bands[i] = (CRITICAL_FREQS[i - 1] as usize * frame_len / rate_half as usize) & !1;
        }
        bands[num_bands] = frame_len;
        Ok(Decoder {
            dct,
            version_b,
            first: true,
            channels,
            tx_channels,
            frame_len,
            overlap_len,
            block_size,
            num_bands,
            root,
            bands,
            previous: vec![vec![0.0; overlap_len]; MAX_DCT_CHANNELS],
            quant_table,
        })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Forget the overlap (a seek).
    pub fn flush(&mut self) {
        self.first = true;
    }

    /// One block of up to two channels into `out[ch_offset..]`, each `frame_len` long.
    fn decode_block(&mut self, gb: &mut Bits<'_>, out: &mut [Vec<f32>], channels: usize, ch_offset: usize) -> Result<()> {
        let fl = self.frame_len;
        if self.dct {
            gb.skip(2);
        }
        let bad = || Error::Invalid("audio block");
        for ch in 0..channels {
            let mut coeffs = vec![0.0f32; fl + 2];
            if self.version_b {
                if gb.left() < 64 {
                    return Err(bad());
                }
                coeffs[0] = f32::from_bits(gb.get(32)) * self.root;
                coeffs[1] = f32::from_bits(gb.get(32)) * self.root;
            } else {
                if gb.left() < 58 {
                    return Err(bad());
                }
                coeffs[0] = get_float(gb) * self.root;
                coeffs[1] = get_float(gb) * self.root;
            }
            if gb.left() < (self.num_bands * 8) as isize {
                return Err(bad());
            }
            let mut quant = [0.0f32; 25];
            for q in quant.iter_mut().take(self.num_bands) {
                *q = self.quant_table[(gb.get(8) as usize).min(95)];
            }
            let mut k = 0usize;
            let mut q = quant[0];
            let mut i = 2usize;
            while i < fl {
                let mut j = if self.version_b {
                    i + 16
                } else if gb.bit() != 0 {
                    i + RLE_LENGTH_TAB[gb.get(4) as usize] * 8
                } else {
                    i + 8
                };
                j = j.min(fl);
                let width = gb.get(4);
                if width == 0 {
                    coeffs[i..j].fill(0.0);
                    i = j;
                    while self.bands[k] < i {
                        q = quant[k];
                        k += 1;
                    }
                } else {
                    while i < j {
                        if self.bands[k] == i {
                            q = quant[k];
                            k += 1;
                        }
                        let coeff = gb.get(width);
                        coeffs[i] = if coeff != 0 {
                            let v = q * coeff as f32;
                            if gb.bit() != 0 { -v } else { v }
                        } else {
                            0.0
                        };
                        i += 1;
                    }
                }
            }
            out[ch + ch_offset] = if self.dct {
                coeffs[0] /= 0.5;
                idct(&coeffs, fl, 1.0 / fl as f64)
            } else {
                let mut i = 2;
                while i < fl {
                    coeffs[i + 1] *= -1.0;
                    i += 2;
                }
                coeffs[fl] = coeffs[1];
                coeffs[fl + 1] = 0.0;
                coeffs[1] = 0.0;
                irdft(&coeffs, fl, 0.5)
            };
        }
        for ch in 0..channels {
            let count = (self.overlap_len * channels) as f32;
            let o = &mut out[ch + ch_offset];
            if !self.first {
                let mut j = ch;
                for i in 0..self.overlap_len {
                    o[i] = (self.previous[ch + ch_offset][i] * (count - j as f32) + o[i] * j as f32) / count;
                    j += channels;
                }
            }
            self.previous[ch + ch_offset].copy_from_slice(&o[fl - self.overlap_len..]);
        }
        self.first = false;
        Ok(())
    }

    /// Decode one audio packet (with its 32-bit size word) to interleaved samples.
    pub fn decode(&mut self, pkt: &[u8]) -> Result<Vec<f32>> {
        if pkt.len() < 4 {
            return Err(Error::Invalid("audio packet too small"));
        }
        let mut gb = Bits::new(pkt);
        gb.skip(32);
        let mut result: Vec<f32> = Vec::new();
        while gb.left() > 0 {
            let mut out: Vec<Vec<f32>> = vec![Vec::new(); MAX_DCT_CHANNELS];
            let mut ch_offset = 0;
            loop {
                let n = MAX_CHANNELS.min(self.tx_channels - ch_offset);
                self.decode_block(&mut gb, &mut out, n, ch_offset)?;
                ch_offset += MAX_CHANNELS;
                gb.align32();
                if ch_offset >= self.tx_channels {
                    break;
                }
                if gb.left() <= 0 {
                    return Err(Error::Invalid("incomplete audio packet"));
                }
            }
            let n = self.block_size / MAX_CHANNELS.min(self.channels);
            if self.dct {
                for i in 0..n {
                    for ch in 0..self.channels {
                        result.push(out[ch][i]);
                    }
                }
            } else {
                result.extend_from_slice(&out[0][..self.block_size]);
            }
        }
        Ok(result)
    }
}
