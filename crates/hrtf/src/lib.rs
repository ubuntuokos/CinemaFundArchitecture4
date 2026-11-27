// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the CFA4 contributors
//
// Parts of this crate are ported from Maul Audio (MIT, Copyright (c) 2026 Sirac Ozmen,
// https://github.com/Rawframe-Project/maul-audio, commit aeb383458250759f92a3e724a5bd006505e45b4d);
// each such file keeps the MIT notice in its header. See README.md and docs/licensing.md §5.

//! HRTF sets and binaural (headphone) rendering for the CFA4 monitor
//! (`docs/plans/CFA4-BINAURAL-MONITOR-001.md`, work package B-01).
//!
//! - [`HrtfSet`]: a head-related transfer function set in the `.maudhrtf` format: minimum-phase
//!   responses with each ear's onset delay kept apart, on elevation rings of evenly spaced
//!   azimuths, optionally resampled to the output rate. [`HrtfSet::builtin`] is the SADIE II
//!   KU100 set (Apache-2.0, `assets/hrtf/`).
//! - [`Binaural`]: one mono source rendered to two ears from a position that may move: four
//!   measured responses blended around the direction, the interaural delay read at a fraction of a
//!   sample, a near-field filter for sources close to the head, and a crossfade between the old
//!   and new responses when the direction changes.
//! - [`VirtualSpeakers`]: a channel layout (2.0 … 7.1.4) heard through headphones, each channel a
//!   [`Binaural`] at its loudspeaker's nominal direction, the LFE low-passed into both ears.
//! - [`SceneRenderer`] (B-02): a scene of beds and moving objects with timed updates, with size,
//!   Dolby's headphone render modes and a room; [`atmos`] feeds it from Dolby TrueHD Atmos and
//!   IAB.
//!
//! Positions are in metres in the listener's frame: x to the right, y up, -z straight ahead.
//! Nothing here allocates once a renderer is made (a scene may when an update is scheduled), and
//! no input makes it panic.

pub mod atmos;
mod binaural;
mod dsp;
mod file;
mod near_field;
mod near_field_table;
mod resample;
pub mod scene;
mod set;
mod speakers;

pub use binaural::{Binaural, BinauralConfig};
pub use near_field::{NearFieldFilter, ear_direction, near_field};
pub use scene::{HeadphoneMode, Placement, SceneConfig, SceneRenderer, SceneState};
pub use set::{HrtfSet, Neighbours};
pub use speakers::{Speaker, SpeakerLayout, VirtualSpeakers, VirtualSpeakersConfig, position_of};

/// Errors from loading an HRTF set or making a renderer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The bytes are not a well-formed `.maudhrtf` file.
    #[error("not a valid HRTF set: {0}")]
    Format(&'static str),
    /// A `.maudhrtf` file of another format version, or with a field only a later version writes.
    #[error("HRTF set of an unsupported format version")]
    Version,
    /// A setting out of its range.
    #[error("invalid setting: {0}")]
    Invalid(&'static str),
}

/// A `Result` with this crate's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
