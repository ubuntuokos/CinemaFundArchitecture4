//! Audio ABI additions: the channel layout and the format the buffer does not carry itself.
//! `cfa4_frame::AudioBuffer` stays planar f32; this describes what its channels *are*.

use cfa4_frame::AudioBuffer;
use serde::{Deserialize, Serialize};

/// Channel order is not guessed: a count that is not clearly one of the named layouts is
/// `Unknown`, and the caller has to say what it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ChannelLayout {
    Mono,
    Stereo,
    /// L R C LFE Ls Rs
    Surround51,
    /// L R C LFE Lss Rss Lsr Rsr
    Surround71,
    Unknown {
        channels: u16,
    },
}

impl ChannelLayout {
    pub fn channels(self) -> u16 {
        match self {
            ChannelLayout::Mono => 1,
            ChannelLayout::Stereo => 2,
            ChannelLayout::Surround51 => 6,
            ChannelLayout::Surround71 => 8,
            ChannelLayout::Unknown { channels } => channels,
        }
    }

    /// Only 1 and 2 channels are unambiguous; 6 and 8 could be 5.1/7.1 or something else.
    pub fn from_count(channels: u16) -> ChannelLayout {
        match channels {
            1 => ChannelLayout::Mono,
            2 => ChannelLayout::Stereo,
            n => ChannelLayout::Unknown { channels: n },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AudioFormatError {
    #[error("sample rate {0} Hz is outside 8000..=384000")]
    SampleRate(u32),
    #[error("source bit depth {0} is not between 8 and 64")]
    BitDepth(u8),
    #[error("no channels")]
    NoChannels,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub layout: ChannelLayout,
    /// Bit depth of the source before it became f32 (MQ-7: 24 bit); `None` when not known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_bit_depth: Option<u8>,
}

impl AudioFormat {
    pub fn validate(&self) -> Result<(), AudioFormatError> {
        if !(8_000..=384_000).contains(&self.sample_rate) {
            return Err(AudioFormatError::SampleRate(self.sample_rate));
        }
        if self.layout.channels() == 0 {
            return Err(AudioFormatError::NoChannels);
        }
        match self.source_bit_depth {
            Some(b) if !(8..=64).contains(&b) => Err(AudioFormatError::BitDepth(b)),
            _ => Ok(()),
        }
    }

    /// The cinema rates of MQ-7.
    pub fn is_cinema_rate(&self) -> bool {
        matches!(self.sample_rate, 48_000 | 96_000)
    }

    /// Whether `buffer` has this rate and this many channels.
    pub fn describes(&self, buffer: &AudioBuffer) -> bool {
        buffer.sample_rate == self.sample_rate && buffer.channels.len() == usize::from(self.layout.channels())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_channels_are_not_assumed_to_be_5_1() {
        assert_eq!(ChannelLayout::from_count(6), ChannelLayout::Unknown { channels: 6 });
        assert_eq!(ChannelLayout::from_count(2), ChannelLayout::Stereo);
        assert_eq!(ChannelLayout::Surround71.channels(), 8);
    }

    #[test]
    fn hostile_formats_are_rejected() {
        let ok = AudioFormat { sample_rate: 48_000, layout: ChannelLayout::Surround51, source_bit_depth: Some(24) };
        assert_eq!(ok.validate(), Ok(()));
        assert!(ok.is_cinema_rate());
        assert_eq!(AudioFormat { sample_rate: 0, ..ok }.validate(), Err(AudioFormatError::SampleRate(0)));
        assert_eq!(AudioFormat { sample_rate: u32::MAX, ..ok }.validate(), Err(AudioFormatError::SampleRate(u32::MAX)));
        assert_eq!(AudioFormat { layout: ChannelLayout::Unknown { channels: 0 }, ..ok }.validate(), Err(AudioFormatError::NoChannels));
        assert_eq!(AudioFormat { source_bit_depth: Some(0), ..ok }.validate(), Err(AudioFormatError::BitDepth(0)));
    }

    #[test]
    fn a_format_describes_only_a_matching_buffer() {
        let f = AudioFormat { sample_rate: 48_000, layout: ChannelLayout::Stereo, source_bit_depth: None };
        assert!(f.describes(&AudioBuffer::silence(48_000, 2, 10)));
        assert!(!f.describes(&AudioBuffer::silence(44_100, 2, 10)));
        assert!(!f.describes(&AudioBuffer::silence(48_000, 6, 10)));
    }
}
