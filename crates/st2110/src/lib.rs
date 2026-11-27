//! Clean-room SMPTE ST 2110 essence formats for the CFA4 live studio (CFA4-LIVE-STUDIO-001 ST 2110
//! addendum, phases T0 and T1).
//!
//! - [`rtp`]: the RTP fixed header (RFC 3550) with ST 2110's constraints.
//! - [`sdp`]: session descriptions (RFC 4566 / 8866) with `ts-refclk`, `mediaclk` and `fmtp`.
//! - [`video`], [`mod@format`]: the ST 2110-20 video description (sampling, depth, pgroups, the
//!   `fmtp` parameters with the -21 and -10:2022 additions).
//! - [`clock`]: PTP (TAI) time, the media clock and RTP timestamps aligned to the SMPTE Epoch,
//!   frame and field timestamps (59.94 Hz steps 1501/1502 exactly), `ptp4l` state from `pmc`, and
//!   the sender's `ts-refclk` / `mediaclk` with and without a PTP lock.
//! - [`pack`]: ST 2110-20 packetization (General and Block Packing Mode) and reassembly, and
//!   conversion between sample planes and pgroup rows.
//!
//! No sockets, clocks or threads: those come with the network phases (T1+). Every input from the
//! network is checked; malformed data is an [`Error`], never a panic.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod clock;
pub mod format;
pub mod pack;
pub mod rtp;
pub mod sdp;
pub mod video;

pub use format::VideoFormat;
pub use pack::{Depacketizer, Image, Packetizer, Planes, planes_to_rows, rows_to_planes};

/// ST 2110-10 Standard UDP Size Limit: the largest UDP datagram, header included.
pub const STANDARD_UDP_LIMIT: usize = 1460;
/// ST 2110-10 Extended UDP Size Limit (signalled with `MAXUDP`).
pub const EXTENDED_UDP_LIMIT: usize = 8960;
/// The UDP header inside the datagram size limit.
pub const UDP_HEADER_LEN: usize = 8;

/// Errors from parsing or building ST 2110 data.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("truncated {0}")]
    Truncated(&'static str),
    #[error("invalid: {0}")]
    Invalid(&'static str),
    #[error("missing {0}")]
    Missing(&'static str),
    #[error("unsupported: {0}")]
    Unsupported(String),
}
