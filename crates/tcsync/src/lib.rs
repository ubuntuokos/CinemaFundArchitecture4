//! External timecode sync for CFA4 (plan: `docs/plans/TIMECODE-SYNC-LTC-MTC-2026-11-11.md`).
//!
//! Pure logic, no device I/O: the devices (cpal for LTC audio, the ALSA sequencer for MIDI
//! Timecode) live in a higher layer and feed samples and bytes in here.
//!
//! - [`ltc`]: SMPTE linear timecode (LTC): the 80-bit frame, a biphase-mark encoder producing
//!   audio samples, and a decoder that reads frames back from audio in either direction and at
//!   varying speed.
//! - [`mtc`]: MIDI Timecode: quarter-frame and full-frame messages, a generator and a byte-stream
//!   parser.
//!
//! Clean-room implementation from public descriptions of the LTC format (bit layout, biphase-mark
//! coding, sync word) and of MIDI Timecode in the MIDI 1.0 specification; no code is taken from
//! other timecode libraries. Every input is treated as hostile (`AGENTS.md` §0): no panics,
//! bounded state, non-finite samples are read as silence.

#![forbid(unsafe_code)]

pub mod ltc;
pub mod mtc;
