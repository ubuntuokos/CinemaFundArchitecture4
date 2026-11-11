//! The video buffering verifier (ITU-T H.262 Annex C) as the encoder keeps it: the occupancy of
//! the decoder's buffer just before each picture is removed, so that no picture is larger than
//! what has arrived (underflow) and, at a constant bitrate, the buffer never overflows (stuffing).
//!
//! Constant bitrate: bits enter at R; picture n is removed `vbv_delay` 90 kHz periods after its
//! picture start code arrived, one picture period after picture n - 1. The buffer is full when
//! the first picture is removed.
//!
//! Variable bitrate (`vbv_delay` 0xFFFF, Annex C.3): bits enter at the maximum rate until the
//! buffer is full, and the first picture is removed when it is.
//!
//! Occupancies are kept in bits × fps_num, so a picture period adds exactly R × fps_den.

/// The longest `vbv_delay` (just below the 0xFFFF that means variable bitrate), in seconds.
const MAX_DELAY: f64 = 65_000.0 / 90_000.0;

pub(super) struct Vbv {
    /// Bitrate into the buffer (R, or the maximum for variable bitrate).
    rate: u64,
    cbr: bool,
    /// The buffer size the encoder fills, in bits × fps_num: the signalled size, at a constant
    /// bitrate no more than `vbv_delay` can express.
    size: u128,
    /// Bits entering in one picture period, × fps_num.
    period: u128,
    num: u128,
    /// Occupancy just before the next picture is removed, × fps_num.
    full: u128,
    /// Pictures larger than the buffer held, however coarsely quantised.
    underflows: u64,
}

impl Vbv {
    /// `size`: the signalled vbv_buffer_size in bits.
    pub fn new(rate: u64, size: u64, cbr: bool, fps_num: u32, fps_den: u32) -> Vbv {
        let num = u128::from(fps_num.max(1));
        let usable = if cbr { size.min((rate as f64 * MAX_DELAY) as u64) } else { size };
        let size = u128::from(usable) * num;
        Vbv { rate: rate.max(1), cbr, size, period: u128::from(rate) * u128::from(fps_den), num, full: size, underflows: 0 }
    }

    /// The occupancy (bits) just before the next picture is removed: the largest picture that
    /// fits.
    pub fn occupancy(&self) -> u64 {
        u64::try_from(self.full / self.num).unwrap_or(u64::MAX)
    }

    /// The buffer size the encoder fills (bits).
    pub fn size(&self) -> u64 {
        u64::try_from(self.size / self.num).unwrap_or(u64::MAX)
    }

    /// Bits entering in one picture period.
    pub fn per_picture(&self) -> u64 {
        u64::try_from(self.period / self.num).unwrap_or(u64::MAX)
    }

    pub fn cbr(&self) -> bool {
        self.cbr
    }

    /// The vbv_delay of the next picture whose `head` bits (headers up to and including its
    /// picture start code) come before the end of its picture start code.
    pub fn delay(&self, head: u64) -> u16 {
        if !self.cbr {
            return 0xFFFF;
        }
        // the bits that arrive after the start code until the picture is removed
        let after = (self.full / self.num).saturating_sub(u128::from(head));
        u16::try_from(after * 90_000 / u128::from(self.rate)).unwrap_or(0xFFFE).min(0xFFFE)
    }

    /// Remove a picture of `bits`; returns the zero bytes of stuffing to append to it so that
    /// the buffer does not overflow before the next picture (constant bitrate).
    pub fn remove(&mut self, bits: u64) -> u64 {
        let b = u128::from(bits) * self.num;
        if b > self.full {
            self.underflows += 1;
            self.full = 0;
        } else {
            self.full -= b;
        }
        self.full += self.period;
        if self.full <= self.size {
            return 0;
        }
        if !self.cbr {
            // variable bitrate: the input stops while the buffer is full
            self.full = self.size;
            return 0;
        }
        let bytes = (self.full - self.size).div_ceil(8 * self.num);
        self.full = self.full.saturating_sub(bytes * 8 * self.num);
        u64::try_from(bytes).unwrap_or(0)
    }

    pub fn underflows(&self) -> u64 {
        self.underflows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_bitrate_stuffs_and_flags_underflow() {
        // 1 Mbit/s at 25 fps: 40 000 bits a picture into a 200 000-bit buffer
        let mut v = Vbv::new(1_000_000, 200_000, true, 25, 1);
        assert_eq!((v.occupancy(), v.per_picture()), (200_000, 40_000));
        // the first picture waits for a full buffer: 0.2 s after its start code
        assert_eq!(v.delay(0), 18_000);
        assert_eq!(v.remove(150_000), 0);
        assert_eq!(v.occupancy(), 90_000);
        // small pictures: the buffer would overflow, so the picture is stuffed
        assert_eq!(v.remove(10_000), 0);
        assert_eq!(v.occupancy(), 120_000);
        let mut stuffed = 0;
        for _ in 0..5 {
            stuffed += v.remove(1_000);
        }
        assert!(stuffed > 0 && v.occupancy() <= 200_000);
        assert_eq!(v.underflows(), 0);
        assert_eq!(v.remove(250_000), 0);
        assert_eq!(v.underflows(), 1);
        // vbv_delay cannot exceed 0xFFFE periods: a huge buffer is used only partly
        assert!(Vbv::new(100_000, 1_835_008, true, 25, 1).size() <= 72_223);
    }

    #[test]
    fn variable_bitrate_stops_when_full() {
        let mut v = Vbv::new(2_000_000, 300_000, false, 30000, 1001);
        assert_eq!(v.delay(0), 0xFFFF);
        assert_eq!(v.remove(1_000), 0);
        assert_eq!(v.occupancy(), 300_000);
        assert_eq!(v.underflows(), 0);
    }
}
