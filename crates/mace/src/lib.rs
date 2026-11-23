//! Apple MACE (Macintosh Audio Compression/Expansion) 3:1 and 6:1, the 8-bit sound codecs of
//! classic Mac OS and QuickTime (`MAC3`, `MAC6`). Ported from FFmpeg n9.0.2's
//! `libavcodec/mace.c` (LGPL-2.1-or-later, `COPYING.LGPL-2.1`); CFA4 changes are
//! GPL-3.0-or-later. Mono or stereo; the state of each channel carries from packet to packet.

mod tables;

use tables::{TAB1, TAB2, TAB3, TAB4};

/// Which MACE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    /// 3:1: one byte holds three samples.
    Mace3,
    /// 6:1: one byte holds six samples.
    Mace6,
}

impl Version {
    /// The QuickTime sample-description FourCC (`MAC3`, `MAC6`).
    pub fn from_fourcc(f: &[u8; 4]) -> Option<Self> {
        match f {
            b"MAC3" => Some(Self::Mace3),
            b"MAC6" => Some(Self::Mace6),
            _ => None,
        }
    }

    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Mace3 => "MACE 3:1",
            Self::Mace6 => "MACE 6:1",
        }
    }
}

/// A MACE decoding error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// MACE is mono or stereo.
    #[error("MACE: {0} channels")]
    Channels(usize),
    /// The packet holds no whole frame (one byte per channel for 6:1, two for 3:1).
    #[error("MACE: packet of {0} bytes holds no whole frame")]
    TooSmall(usize),
}

/// Result alias.
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, Default)]
struct Channel {
    index: i16,
    factor: i16,
    prev2: i16,
    previous: i16,
    level: i16,
}

/// FFmpeg's `mace_broken_clip_int16`: the low end clips to -32767, as the original does.
fn broken_clip(n: i32) -> i16 {
    n.clamp(-32767, 32767) as i16
}

/// `QT_8S_2_16S`.
fn qt(x: i32) -> i16 {
    ((x & 0xFF00) | ((x >> 8) & 0xFF)) as i16
}

impl Channel {
    fn read_table(&mut self, val: u8, tab: usize) -> i16 {
        let (stride, tab1): (usize, &[i16]) = if tab == 1 { (2, &TAB3) } else { (4, &TAB1) };
        let row = usize::try_from((self.index & 0x7f0) >> 4).unwrap_or(0).min(127);
        let v = usize::from(val);
        let at = |i: usize| -> i16 { if tab == 1 { TAB4[row].get(i).copied().unwrap_or(0) } else { TAB2[row].get(i).copied().unwrap_or(0) } };
        let current = if v < stride { at(v) } else { (-1i16).wrapping_sub(at((2 * stride).wrapping_sub(v + 1))) };
        let n = i32::from(self.index) + i32::from(tab1.get(v).copied().unwrap_or(0)) - i32::from(self.index >> 5);
        let n = n as i16;
        self.index = n.max(0);
        current
    }

    fn chomp3(&mut self, val: u8, tab: usize) -> i16 {
        let current = self.read_table(val, tab);
        let current = broken_clip(i32::from(current) + i32::from(self.level));
        self.level = current - (current >> 3);
        qt(i32::from(current))
    }

    fn chomp6(&mut self, val: u8, tab: usize) -> [i16; 2] {
        let mut current = self.read_table(val, tab);
        if (self.previous ^ current) >= 0 {
            self.factor = (i32::from(self.factor) + 506).min(32767) as i16;
        } else if i32::from(self.factor) - 314 < -32768 {
            self.factor = -32767;
        } else {
            self.factor -= 314;
        }
        current = broken_clip(i32::from(current) + i32::from(self.level));
        self.level = ((i32::from(current) * i32::from(self.factor)) >> 15) as i16;
        current >>= 1;
        let (p, p2, c) = (i32::from(self.previous), i32::from(self.prev2), i32::from(current));
        let out = [qt(p + p2 - ((p2 - c) >> 2)), qt(p + c + ((p2 - c) >> 2))];
        self.prev2 = self.previous;
        self.previous = current;
        out
    }
}

/// A MACE decoder for one stream.
#[derive(Debug, Clone)]
pub struct Decoder {
    version: Version,
    channels: usize,
    chd: [Channel; 2],
}

impl Decoder {
    /// A decoder for `channels` (1 or 2) channels.
    pub fn new(version: Version, channels: usize) -> Result<Self> {
        if !(1..=2).contains(&channels) {
            return Err(Error::Channels(channels));
        }
        Ok(Self { version, channels, chd: [Channel::default(); 2] })
    }

    /// The version.
    pub fn version(&self) -> Version {
        self.version
    }

    /// The channel count.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Back to the start-of-stream state.
    pub fn reset(&mut self) {
        self.chd = [Channel::default(); 2];
    }

    /// Samples per channel that a packet of `bytes` decodes to (a short tail is dropped).
    pub fn samples_in(&self, bytes: usize) -> usize {
        let unit = self.channels << usize::from(self.version == Version::Mace3);
        let whole = bytes - bytes % unit;
        match self.version {
            Version::Mace3 => 3 * whole / self.channels,
            Version::Mace6 => 6 * whole / self.channels,
        }
    }

    /// Decode one packet into planar 16-bit channels. A trailing partial frame is ignored; a
    /// packet under one frame is an error.
    pub fn decode(&mut self, pkt: &[u8]) -> Result<Vec<Vec<i16>>> {
        let m3 = self.version == Version::Mace3;
        let ch = self.channels;
        let unit = ch << usize::from(m3);
        let size = pkt.len() - pkt.len() % unit;
        if size == 0 {
            return Err(Error::TooSmall(pkt.len()));
        }
        let mut out = Vec::with_capacity(ch);
        for i in 0..ch {
            let mut samples = Vec::with_capacity(self.samples_in(size));
            for j in 0..size / unit {
                for k in 0..(1usize << usize::from(m3)) {
                    let byte = pkt.get((i << usize::from(m3)) + ((j * ch) << usize::from(m3)) + k).copied().unwrap_or(0);
                    let (hi, mid, lo) = (byte >> 5, (byte >> 3) & 3, byte & 7);
                    if m3 {
                        for (l, v) in [lo, mid, hi].into_iter().enumerate() {
                            samples.push(self.chd[i].chomp3(v, l));
                        }
                    } else {
                        for (l, v) in [hi, mid, lo].into_iter().enumerate() {
                            samples.extend(self.chd[i].chomp6(v, l));
                        }
                    }
                }
            }
            out.push(samples);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_errors() {
        let mut d = Decoder::new(Version::Mace3, 2).expect("decoder");
        let p = d.decode(&[0; 4]).expect("one frame");
        assert_eq!((p.len(), p[0].len(), p[1].len()), (2, 6, 6));
        // a partial frame is dropped, less than one is an error
        assert_eq!(d.decode(&[0; 7]).expect("one frame and a tail")[0].len(), 6);
        assert!(matches!(d.decode(&[0; 3]), Err(Error::TooSmall(3))));
        let mut d6 = Decoder::new(Version::Mace6, 1).expect("decoder");
        assert_eq!(d6.decode(&[0; 5]).expect("five bytes")[0].len(), 30);
        assert!(Decoder::new(Version::Mace6, 0).is_err());
        assert!(Decoder::new(Version::Mace6, 3).is_err());
    }

    #[test]
    fn damaged_packets_do_not_panic() {
        let mut x = 9u32;
        for v in [Version::Mace3, Version::Mace6] {
            for ch in [1, 2] {
                let mut d = Decoder::new(v, ch).expect("decoder");
                for len in 0..300 {
                    let pkt: Vec<u8> = (0..len)
                        .map(|_| {
                            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                            (x >> 16) as u8
                        })
                        .collect();
                    let _ = d.decode(&pkt);
                }
                for pat in [0x00u8, 0xFF, 0x55, 0xAA] {
                    let _ = d.decode(&[pat; 600]);
                }
            }
        }
    }

    #[test]
    fn reset_restores_the_initial_state() {
        let pkt: Vec<u8> = (0..64u8).map(|i| i.wrapping_mul(53)).collect();
        let mut d = Decoder::new(Version::Mace6, 2).expect("decoder");
        let a = d.decode(&pkt).expect("decode");
        let _ = d.decode(&pkt);
        d.reset();
        assert_eq!(d.decode(&pkt).expect("decode"), a);
    }

    #[test]
    fn fourcc_mapping() {
        assert_eq!(Version::from_fourcc(b"MAC3"), Some(Version::Mace3));
        assert_eq!(Version::from_fourcc(b"MAC6"), Some(Version::Mace6));
        assert_eq!(Version::from_fourcc(b"ima4"), None);
    }
}
