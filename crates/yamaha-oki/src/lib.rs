// Ported from FFmpeg n7.1.1: libavcodec/adpcm.c (`adpcm_yamaha_expand_nibble`,
// `adpcm_ima_oki_expand_nibble` and their decode loops, Copyright (c) 2001-2003 The FFmpeg
// project) and libavcodec/adpcm_data.c (the Yamaha tables) and libavformat/riff.c (the WAV
// format tags); all LGPL-2.1-or-later (`COPYING.LGPL-2.1`); see docs/licensing.md §5. CFA4 changes
// GPL-3.0-or-later.

//! Pure-Rust decoders for **Yamaha ADPCM** (WAV format tag 0x0020, the 4-bit codec of the YM2608
//! family) and **Dialogic OKI ADPCM** (WAV tag 0x0010, the MSM6295 / VOX family) (codec backlog
//! CB-31.1).
//!
//! Both are plain streams of 4-bit codes with no block headers: the adaptation state starts at a
//! fixed value and runs on from byte to byte, so a [`State`] is cloned to checkpoint a stream.
//! One or two channels (the codes of a stereo stream alternate between the channels).

/// The codec of a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Yamaha ADPCM: the low nibble of a byte first.
    Yamaha,
    /// Dialogic OKI ADPCM: the high nibble of a byte first, 12-bit output scaled by 16.
    Oki,
}

/// `WAVE_FORMAT_OKI_ADPCM`.
pub const WAVE_FORMAT_OKI_ADPCM: u16 = 0x0010;
/// `WAVE_FORMAT_YAMAHA_ADPCM`.
pub const WAVE_FORMAT_YAMAHA_ADPCM: u16 = 0x0020;

impl Kind {
    /// The codec a WAV `fmt ` tag names.
    pub fn from_wave_format(tag: u16) -> Option<Self> {
        match tag {
            WAVE_FORMAT_YAMAHA_ADPCM => Some(Self::Yamaha),
            WAVE_FORMAT_OKI_ADPCM => Some(Self::Oki),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Yamaha => "Yamaha ADPCM",
            Self::Oki => "Dialogic OKI ADPCM",
        }
    }
}

/// Most channels (ffmpeg's ADPCM decoders allow two).
pub const MAX_CHANNELS: usize = 2;

/// Sample frames that `bytes` of data decode to.
pub fn frames_in(bytes: usize, channels: usize) -> usize {
    bytes.saturating_mul(2) / channels.max(1)
}

const YAMAHA_INDEXSCALE: [i32; 16] = [230, 230, 230, 230, 307, 409, 512, 614, 230, 230, 230, 230, 307, 409, 512, 614];
const YAMAHA_DIFFLOOKUP: [i32; 16] = [1, 3, 5, 7, 9, 11, 13, 15, -1, -3, -5, -7, -9, -11, -13, -15];

const OKI_STEPS: [i32; 49] = [
    16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408,
    449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552,
];
const OKI_INDEX: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

/// One channel's adaptation state (ffmpeg's `ADPCMChannelStatus`: predictor, and step or index).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Channel {
    predictor: i32,
    /// Yamaha: the step size. OKI: the index into the step table.
    step: i32,
}

impl Channel {
    fn yamaha(&mut self, nibble: u8) -> i16 {
        let n = usize::from(nibble & 15);
        if self.step == 0 {
            self.predictor = 0;
            self.step = 127;
        }
        self.predictor += (self.step * YAMAHA_DIFFLOOKUP[n]) / 8;
        self.predictor = self.predictor.clamp(-32768, 32767);
        self.step = ((self.step * YAMAHA_INDEXSCALE[n]) >> 8).clamp(127, 24576);
        self.predictor as i16
    }

    fn oki(&mut self, nibble: u8) -> i16 {
        let n = usize::from(nibble & 15);
        let step = OKI_STEPS[self.step as usize];
        self.step = (self.step + OKI_INDEX[n]).clamp(0, 48);
        let diff = ((2 * (n as i32 & 7) + 1) * step) >> 3;
        let p = if n & 8 != 0 { self.predictor - diff } else { self.predictor + diff };
        self.predictor = p.clamp(-2048, 2047);
        (self.predictor * 16) as i16
    }
}

/// A decoder, cloneable to checkpoint the stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    kind: Kind,
    channels: usize,
    ch: [Channel; MAX_CHANNELS],
}

impl State {
    /// A decoder at the start of a stream; `None` for a channel count ffmpeg does not decode.
    pub fn new(kind: Kind, channels: usize) -> Option<Self> {
        (1..=MAX_CHANNELS).contains(&channels).then_some(Self { kind, channels, ch: [Channel::default(); MAX_CHANNELS] })
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Decode `data`, appending each channel's samples to `out` (one vector per channel).
    pub fn decode(&mut self, data: &[u8], out: &mut [Vec<i16>]) {
        let st = self.channels - 1;
        for &v in data {
            let (first, second) = match self.kind {
                Kind::Yamaha => (v & 15, v >> 4),
                Kind::Oki => (v >> 4, v & 15),
            };
            let a = self.step(0, first);
            let b = self.step(st, second);
            if let Some(o) = out.get_mut(0) {
                o.push(a);
            }
            if let Some(o) = out.get_mut(st) {
                o.push(b);
            }
        }
    }

    fn step(&mut self, c: usize, nibble: u8) -> i16 {
        match self.kind {
            Kind::Yamaha => self.ch[c].yamaha(nibble),
            Kind::Oki => self.ch[c].oki(nibble),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_alternates_nibbles_of_one_state() {
        let mut s = State::new(Kind::Yamaha, 1).unwrap();
        let mut out = vec![Vec::new()];
        s.decode(&[0x10, 0x00], &mut out);
        assert_eq!(out[0].len(), 4);
        // the first code (0) adds step 127 * 1 / 8 = 15
        assert_eq!(out[0][0], 15);
    }

    #[test]
    fn stereo_splits_channels() {
        let mut s = State::new(Kind::Oki, 2).unwrap();
        let mut out = vec![Vec::new(), Vec::new()];
        s.decode(&[0x70, 0x07, 0x70], &mut out);
        assert_eq!((out[0].len(), out[1].len()), (3, 3));
        assert_eq!(frames_in(3, 2), 3);
        assert_eq!(frames_in(3, 1), 6);
    }

    #[test]
    fn tags_and_channels() {
        assert_eq!(Kind::from_wave_format(0x20), Some(Kind::Yamaha));
        assert_eq!(Kind::from_wave_format(0x10), Some(Kind::Oki));
        assert_eq!(Kind::from_wave_format(0x11), None);
        assert!(State::new(Kind::Oki, 0).is_none());
        assert!(State::new(Kind::Oki, 3).is_none());
    }

    #[test]
    fn state_runs_on_between_calls() {
        let data: Vec<u8> = (0..=255u8).collect();
        for kind in [Kind::Yamaha, Kind::Oki] {
            let mut whole = vec![Vec::new()];
            State::new(kind, 1).unwrap().decode(&data, &mut whole);
            let mut s = State::new(kind, 1).unwrap();
            let mut parts = vec![Vec::new()];
            s.decode(&data[..100], &mut parts);
            let mut s2 = s.clone();
            s2.decode(&data[100..], &mut parts);
            assert_eq!(whole, parts);
        }
    }
}
