//! Shared media I/O core (layer L1, CFA4-DECKLINK-READY-001 decision R1).
//!
//! What every media input/output process of CFA4 needs, independent of how it makes pictures:
//! the live studio host (`cfa4-obs`, libobs or the software compositor) uses it today, a capture
//! and playout host can use it later without depending on libobs.
//!
//! - [`ptp`]: the PTP state `ptp4l` reports (read through `pmc`), the TAI time and the SDP clock
//!   signalling of an ST 2110 sender. Moved here unchanged from `cfa4-obs` (READY-V1), which
//!   re-exports it as `cfa4_obs::ptp`.
//! - [`frame`]: the picture ([`frame::Frame`], [`frame::PixelFormat`]) and sound ([`frame::Audio`]) formats.
//! - [`ring`]: the versioned shared-memory frame ring (layout version 2); moved here from
//!   `cfa4-obs` (READY-V2.1), which re-exports it as `cfa4_obs::ring`.
//!
//! Only `std`, `serde`, `thiserror`, `cfa4-frame` (L1) and L0 crates; no `unsafe`, no threads of its own.

pub mod frame;
pub mod ptp;
pub mod ring;
