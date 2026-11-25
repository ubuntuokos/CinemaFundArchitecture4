//! The decoder state and the frame synthesis (FFmpeg `truespeech.c`; its C integer arithmetic,
//! including the narrowing to 16 bits on assignment, is kept as is).

use crate::tables::{CODEBOOK, DECAY_3_4, DECAY_35_64, DECAY_994_1000, ORDER2_COEFFS, PULSE_SCALES, PULSE_VALUES};
use crate::{Error, FRAME_BYTES, FRAME_SAMPLES, Result};

/// The fields of one frame.
#[derive(Default)]
struct Frame {
    vector: [i16; 8],
    offset1: [i32; 2],
    offset2: [i32; 4],
    pulseoff: [i32; 4],
    pulsepos: [i32; 4],
    pulseval: [i32; 4],
    flag: bool,
}

/// MSB-first bit reader over the frame with each 32-bit word byte-swapped, as FFmpeg reads it.
struct Bits {
    buf: [u8; FRAME_BYTES],
    pos: usize,
}

impl Bits {
    fn new(input: &[u8]) -> Self {
        let mut buf = [0u8; FRAME_BYTES];
        for (w, chunk) in buf.chunks_exact_mut(4).enumerate() {
            for (k, b) in chunk.iter_mut().enumerate() {
                *b = input.get(w * 4 + 3 - k).copied().unwrap_or(0);
            }
        }
        Self { buf, pos: 0 }
    }

    fn get(&mut self, n: usize) -> i32 {
        let mut v = 0i32;
        for _ in 0..n {
            let byte = self.buf.get(self.pos >> 3).copied().unwrap_or(0);
            v = (v << 1) | i32::from((byte >> (7 - (self.pos & 7))) & 1);
            self.pos += 1;
        }
        v
    }
}

/// A TrueSpeech decoder; the state carries from frame to frame.
#[derive(Clone)]
pub struct Decoder {
    filtbuf: [i32; 146],
    prevfilt: [i32; 8],
    tmp1: [i16; 8],
    tmp2: [i16; 8],
    tmp3: [i16; 8],
    cvector: [i16; 8],
    filtval: i32,
    newvec: [i16; 60],
    filters: [i16; 32],
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

fn clip(v: i32) -> i16 {
    v.clamp(-0x7FFE, 0x7FFE) as i16
}

impl Decoder {
    /// A decoder in its initial state.
    pub fn new() -> Self {
        Self { filtbuf: [0; 146], prevfilt: [0; 8], tmp1: [0; 8], tmp2: [0; 8], tmp3: [0; 8], cvector: [0; 8], filtval: 0, newvec: [0; 60], filters: [0; 32] }
    }

    /// Back to the initial state.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Decode every whole frame of `pkt` (a trailing partial frame is ignored): 240 samples each.
    pub fn decode(&mut self, pkt: &[u8]) -> Result<Vec<i16>> {
        let n = pkt.len() / FRAME_BYTES;
        if n == 0 {
            return Err(Error::TooSmall(pkt.len()));
        }
        let mut out = vec![0i16; n * FRAME_SAMPLES];
        for (frame, samples) in pkt.chunks_exact(FRAME_BYTES).zip(out.chunks_exact_mut(FRAME_SAMPLES)) {
            self.decode_frame(frame, samples);
        }
        Ok(out)
    }

    /// Decode one frame of [`FRAME_BYTES`] bytes into [`FRAME_SAMPLES`] samples.
    pub fn decode_frame(&mut self, frame: &[u8], out: &mut [i16]) {
        let mut f = read_frame(frame);
        self.correlate_filter(&f);
        self.filters_merge(&f);
        for (quart, samples) in out.chunks_exact_mut(60).take(4).enumerate() {
            self.twopoint_filter(&f, quart);
            place_pulses(&mut f, samples, quart);
            self.update_filters(samples);
            self.synth(samples, quart);
        }
        for (p, &c) in self.prevfilt.iter_mut().zip(&self.cvector) {
            *p = i32::from(c);
        }
    }

    fn correlate_filter(&mut self, f: &Frame) {
        for i in 0..8 {
            if i > 0 {
                let tmp = self.cvector;
                for j in 0..i {
                    let v = (i32::from(tmp[i - j - 1]) * i32::from(f.vector[i]) + 0x4000) >> 15;
                    self.cvector[j] = (i32::from(self.cvector[j]) + v) as i16;
                }
            }
            self.cvector[i] = ((8 - i32::from(f.vector[i])) >> 3) as i16;
        }
        for (c, &d) in self.cvector.iter_mut().zip(&DECAY_994_1000) {
            *c = ((i32::from(*c) * i32::from(d)) >> 15) as i16;
        }
        self.filtval = i32::from(f.vector[0]);
    }

    fn filters_merge(&mut self, f: &Frame) {
        for i in 0..8 {
            let (c, p) = (i32::from(self.cvector[i]), self.prevfilt[i]);
            if f.flag {
                self.filters[i] = ((c * 21846 + p * 10923 + 16384) >> 15) as i16;
                self.filters[i + 8] = ((c * 10923 + p * 21846 + 16384) >> 15) as i16;
            } else {
                self.filters[i] = p as i16;
                self.filters[i + 8] = p as i16;
            }
            self.filters[i + 16] = self.cvector[i];
            self.filters[i + 24] = self.cvector[i];
        }
    }

    fn twopoint_filter(&mut self, f: &Frame, quart: usize) {
        let t = f.offset2[quart];
        if t == 127 {
            self.newvec = [0; 60];
            return;
        }
        let mut tmp = [0i16; 146 + 60];
        for (d, &s) in tmp.iter_mut().zip(&self.filtbuf) {
            *d = s as i16;
        }
        let off = (t / 25 + f.offset1[quart >> 1] + 18).clamp(0, 145) as usize;
        let at = (t % 25) as usize * 2;
        let (c0, c1) = (i32::from(ORDER2_COEFFS[at]), i32::from(ORDER2_COEFFS[at + 1]));
        let p0 = 145 - off;
        for i in 0..60 {
            let v = ((i32::from(tmp[p0 + i]) * c0 + i32::from(tmp[p0 + i + 1]) * c1 + 0x2000) >> 14) as i16;
            self.newvec[i] = v;
            tmp[146 + i] = v;
        }
    }

    fn update_filters(&mut self, out: &mut [i16]) {
        self.filtbuf.copy_within(60..146, 0);
        for (i, o) in out.iter_mut().enumerate().take(60) {
            let n = i32::from(self.newvec[i]);
            self.filtbuf[i + 86] = i32::from(*o) + n - (n >> 3);
            *o = (i32::from(*o) + n) as i16;
        }
    }

    fn synth(&mut self, out: &mut [i16], quart: usize) {
        let ptr1: [i16; 8] = std::array::from_fn(|k| self.filters[quart * 8 + k]);
        for o in out.iter_mut().take(60) {
            let mut sum = 0u32;
            for k in 0..8 {
                sum = sum.wrapping_add((i32::from(self.tmp1[k]) as u32).wrapping_mul(i32::from(ptr1[k]) as u32));
            }
            let s = i32::from(*o) + ((sum.wrapping_add(0x800) as i32) >> 12);
            *o = clip(s);
            self.tmp1.copy_within(0..7, 1);
            self.tmp1[0] = *o;
        }

        let mut t: [i32; 8] = std::array::from_fn(|i| (i32::from(DECAY_35_64[i]) * i32::from(ptr1[i])) >> 15);
        for o in out.iter_mut().take(60) {
            let mut sum = 0i32;
            for k in 0..8 {
                sum = sum.wrapping_add(i32::from(self.tmp2[k]).wrapping_mul(t[k]));
            }
            self.tmp2.copy_within(0..7, 1);
            self.tmp2[0] = *o;
            *o = (i32::from(*o) + (sum.wrapping_neg() >> 12)) as i16;
        }

        t = std::array::from_fn(|i| (i32::from(DECAY_3_4[i]) * i32::from(ptr1[i])) >> 15);
        for o in out.iter_mut().take(60) {
            let mut sum = i32::from(*o).wrapping_mul(1 << 12);
            for k in 0..8 {
                sum = sum.wrapping_add(i32::from(self.tmp3[k]).wrapping_mul(t[k]));
            }
            self.tmp3.copy_within(0..7, 1);
            self.tmp3[0] = clip(sum.wrapping_add(0x800) >> 12);
            let fv = self.filtval - (self.filtval >> 2);
            sum = (i32::from(self.tmp3[1]).wrapping_mul(fv) >> 4).wrapping_add(sum);
            sum -= sum >> 3;
            *o = clip(sum.wrapping_add(0x800) >> 12);
        }
    }
}

fn read_frame(input: &[u8]) -> Frame {
    let mut g = Bits::new(input);
    let mut f = Frame::default();
    for (i, bits) in [(7, 3), (6, 3), (5, 3), (4, 4), (3, 4), (2, 4), (1, 5), (0, 5)] {
        let idx = g.get(bits) as usize;
        f.vector[i] = CODEBOOK[i].get(idx).copied().unwrap_or(0);
    }
    f.flag = g.get(1) != 0;

    f.offset1[0] = g.get(4) << 4;
    for i in (0..4).rev() {
        f.offset2[i] = g.get(7);
    }

    f.offset1[1] = g.get(4);
    f.pulseval[1] = g.get(14);
    f.pulseval[0] = g.get(14);

    f.offset1[1] |= g.get(4) << 4;
    f.pulseval[3] = g.get(14);
    f.pulseval[2] = g.get(14);

    for q in 0..4 {
        f.offset1[0] |= g.get(1) << q;
        f.pulsepos[q] = g.get(27);
        f.pulseoff[q] = g.get(4);
    }
    f
}

fn place_pulses(f: &mut Frame, out: &mut [i16], quart: usize) {
    out.iter_mut().take(60).for_each(|o| *o = 0);
    let mut tmp = [0i16; 7];
    for i in 0..7 {
        let t = (f.pulseval[quart] & 3) as usize;
        f.pulseval[quart] >>= 2;
        tmp[6 - i] = PULSE_SCALES.get(f.pulseoff[quart] as usize * 4 + t).copied().unwrap_or(0);
    }
    let pv = |at: usize| i32::from(PULSE_VALUES.get(at).copied().unwrap_or(0));

    let mut coef = f.pulsepos[quart] >> 15;
    let (mut p1, mut p2, mut j) = (30usize, 0usize, 3);
    let mut i = 0;
    while i < 30 && j > 0 {
        let t = pv(p1);
        p1 += 1;
        if coef >= t {
            coef -= t;
        } else {
            out[i] = tmp.get(p2).copied().unwrap_or(0);
            p2 += 1;
            p1 += 30;
            j -= 1;
        }
        i += 1;
    }
    coef = f.pulsepos[quart] & 0x7FFF;
    p1 = 0;
    j = 4;
    i = 30;
    while i < 60 && j > 0 {
        let t = pv(p1);
        p1 += 1;
        if coef >= t {
            coef -= t;
        } else {
            out[i] = tmp.get(p2).copied().unwrap_or(0);
            p2 += 1;
            p1 += 30;
            j -= 1;
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_packets_are_errors_and_silence_stays_quiet() {
        let mut d = Decoder::new();
        assert!(d.decode(&[0; 31]).is_err());
        let pcm = d.decode(&[0; 64]).expect("two frames");
        assert_eq!(pcm.len(), 480);
    }

    #[test]
    fn damaged_packets_do_not_panic() {
        let mut d = Decoder::new();
        let mut x = 5u32;
        for len in 0..400 {
            let pkt: Vec<u8> = (0..len)
                .map(|_| {
                    x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    (x >> 16) as u8
                })
                .collect();
            let _ = d.decode(&pkt);
        }
        for pat in [0x00u8, 0xFF, 0x55, 0xAA] {
            let _ = d.decode(&[pat; 320]);
        }
    }

    #[test]
    fn reset_restores_the_initial_state() {
        let pkt: Vec<u8> = (0..96u8).map(|i| i.wrapping_mul(37)).collect();
        let mut d = Decoder::new();
        let a = d.decode(&pkt).expect("decode");
        let _ = d.decode(&pkt);
        d.reset();
        assert_eq!(d.decode(&pkt).expect("decode"), a);
    }
}
