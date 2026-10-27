//! Clean-room Immersive Audio Bitstream (IAB) decoder, renderer and writer, written from SMPTE
//! ST 2098-2 (the public committee draft of the 2026 revision, github.com/SMPTE/st2098-2).
//!
//! IAB is the object-based sound format of SMPTE Digital Cinema Packages (ST 429-18) and IMF
//! (ST 2067-201): every IABitstream frame carries, for one picture edit unit, the mono audio
//! assets (linear PCM or the lossless DLC coding of Annex B), the Beds that route assets to
//! loudspeaker channels (with conditional replacements and remap matrices) and the Audio Objects
//! that pan assets to positions in the room, in sub-blocks of about 5 ms.
//!
//! - [`parse`]: one IABitstream frame (or a bare IAFrame element) → [`Frame`], with every audio
//!   asset decoded to 32-bit left-justified samples (DLC: lattice predictors, Rice / PCM
//!   residuals, the 96 kHz extension layer over the interpolated 48 kHz base layer).
//! - [`render`]: a [`Frame`] → loudspeaker feeds for a [`Layout`] (stereo, 5.1, 7.1, 7.1.4):
//!   UseCase selection of Beds, BedRemaps and Objects, Bed channels routed to their loudspeakers
//!   (or panned from their nominal positions), Objects panned per sub-block with gain ramps,
//!   snap, spread and zone gains.
//! - [`write`]: a [`Frame`] → an IABitstream frame, audio as PCM or DLC (adaptive lattice
//!   prediction with Rice or PCM residuals, both sample rates).
//!
//! Layer L0: depends only on `cfa4-bitstream` and `thiserror`, no `unsafe`, no panics on
//! malformed input.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

mod dlc;
mod frame;
mod render;
mod write;

pub use frame::{
    AudioData, Bed, BedChannel, Coding, Decorrelation, Description, Frame, FrameRate, Gain, Object, ObjectZones19, Pan, Remap, RemapBlock, Snap, Spread,
    UserData, ZoneGain, frame_size, parse, split,
};
pub use render::{Layout, Speaker, channel_position, render};
pub use write::write;

/// Errors. Malformed input gives an error, never a panic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("truncated IAB data")]
    Truncated,
    #[error("invalid IAB data: {0}")]
    Invalid(&'static str),
    #[error("unsupported IAB feature: {0}")]
    Unsupported(&'static str),
}

impl From<cfa4_bitstream::BitError> for Error {
    fn from(_: cfa4_bitstream::BitError) -> Self {
        Error::Truncated
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// ElementID codes (Table 15).
pub mod element {
    pub const IA_FRAME: u32 = 0x08;
    pub const BED_DEFINITION: u32 = 0x10;
    pub const BED_REMAP: u32 = 0x20;
    pub const OBJECT_DEFINITION: u32 = 0x40;
    pub const OBJECT_ZONE_DEFINITION19: u32 = 0x80;
    pub const AUTHORING_TOOL_INFO: u32 = 0x100;
    pub const USER_DATA: u32 = 0x101;
    pub const AUDIO_DATA_DLC: u32 = 0x200;
    pub const AUDIO_DATA_PCM: u32 = 0x400;
    pub const FRAME_SEQUENCE: u32 = 0x800;
}

/// UseCase codes (Table 1).
pub mod use_case {
    pub const SURROUND_5_1: u8 = 0x01;
    pub const SURROUND_7_1_DS: u8 = 0x02;
    pub const SURROUND_7_1_SDS: u8 = 0x03;
    pub const HT_11_1: u8 = 0x04;
    pub const HT_13_1: u8 = 0x05;
    pub const OH_9_1: u8 = 0x06;
    /// ITU-R BS.2051 sound system A (0+2+0).
    pub const ITU_A: u8 = 0x30;
    /// ITU-R BS.2051 sound system D (4+5+0).
    pub const ITU_D: u8 = 0x31;
    /// ITU-R BS.2051 sound system J (4+7+0).
    pub const ITU_J: u8 = 0x32;
    pub const ALWAYS: u8 = 0xFF;
}

#[cfg(test)]
mod tests;
