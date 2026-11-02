//! FD filter bank of USAC: IMDCT, windowing and overlap-add for the window sequences of
//! ISO/IEC 23003-3 §7.9 (including `STOP_START_SEQUENCE`), for 1024- and 768-sample frames.

use crate::mdct::{Mdct, kbd_half, sine_half};

/// USAC `window_sequence` after `win_seq_select()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WinSeq {
    #[default]
    OnlyLong,
    LongStart,
    EightShort,
    LongStop,
    StopStart,
}

impl WinSeq {
    /// Map the coded 2-bit value; a `LONG_START` after a sequence that ended short becomes
    /// `STOP_START` (`ixheaacd_win_seq_select`).
    pub fn select(coded: u32, last: WinSeq) -> WinSeq {
        match coded & 3 {
            0 => WinSeq::OnlyLong,
            1 => {
                if matches!(last, WinSeq::LongStart | WinSeq::EightShort | WinSeq::StopStart) {
                    WinSeq::StopStart
                } else {
                    WinSeq::LongStart
                }
            }
            2 => WinSeq::EightShort,
            _ => WinSeq::LongStop,
        }
    }
    pub fn is_short(self) -> bool {
        self == WinSeq::EightShort
    }
}

/// Transforms and rising window halves for one frame length.
pub struct FilterBank {
    pub n: usize,
    long: Mdct,
    short: Mdct,
    /// [sine, kbd] rising halves of length n and n/8.
    win_long: [Vec<f32>; 2],
    win_short: [Vec<f32>; 2],
}

impl FilterBank {
    pub fn new(n: usize) -> FilterBank {
        FilterBank {
            n,
            long: Mdct::new(n),
            short: Mdct::new(n / 8),
            win_long: [sine_half(n), kbd_half(n, 4.0)],
            win_short: [sine_half(n / 8), kbd_half(n / 8, 6.0)],
        }
    }

    fn wl(&self, kbd: bool) -> &[f32] {
        &self.win_long[kbd as usize]
    }
    fn ws(&self, kbd: bool) -> &[f32] {
        &self.win_short[kbd as usize]
    }

    /// Synthesise one FD frame. `spec` holds n lines (short windows at stride n/8); `overlap`
    /// (length n) is updated. Output is scaled to ±1.0 full scale.
    pub fn synth(&self, spec: &[f32], seq: WinSeq, kbd: bool, prev_kbd: bool, overlap: &mut [f32], out: &mut [f32]) {
        let n = self.n;
        let ns = n / 8;
        let flat = (n - ns) / 2;
        let mut buf = vec![0f32; 2 * n];
        let norm = 1.0 / 32768.0;
        if seq == WinSeq::EightShort {
            let mut y = vec![0f32; 2 * ns];
            let wc = self.ws(kbd);
            let s = norm / ns as f32;
            for w in 0..8 {
                self.short.inverse(&spec[w * ns..(w + 1) * ns], &mut y);
                let wl = self.ws(if w == 0 { prev_kbd } else { kbd });
                let off = flat + ns * w;
                for k in 0..ns {
                    buf[off + k] += y[k] * wl[k] * s;
                    buf[off + ns + k] += y[ns + k] * wc[ns - 1 - k] * s;
                }
            }
        } else {
            let mut y = vec![0f32; 2 * n];
            self.long.inverse(&spec[..n], &mut y);
            let s = norm / n as f32;
            if matches!(seq, WinSeq::OnlyLong | WinSeq::LongStart) {
                let w = self.wl(prev_kbd);
                for k in 0..n {
                    buf[k] = y[k] * w[k] * s;
                }
            } else {
                let w = self.ws(prev_kbd);
                for k in 0..ns {
                    buf[flat + k] = y[flat + k] * w[k] * s;
                }
                for k in flat + ns..n {
                    buf[k] = y[k] * s;
                }
            }
            if matches!(seq, WinSeq::OnlyLong | WinSeq::LongStop) {
                let w = self.wl(kbd);
                for k in 0..n {
                    buf[n + k] = y[n + k] * w[n - 1 - k] * s;
                }
            } else {
                let w = self.ws(kbd);
                for k in 0..flat {
                    buf[n + k] = y[n + k] * s;
                }
                for k in 0..ns {
                    buf[n + flat + k] = y[n + flat + k] * w[ns - 1 - k] * s;
                }
            }
        }
        for k in 0..n {
            out[k] = overlap[k] + buf[k];
        }
        overlap.copy_from_slice(&buf[n..]);
    }
}
