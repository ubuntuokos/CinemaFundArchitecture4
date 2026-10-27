#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]
// SPDX-License-Identifier: LGPL-2.1-or-later
// Ported from FFmpeg's DTS decoder (libavcodec/dca*.c, Copyright (C) 2016 foo86 and the FFmpeg
// developers, LGPL-2.1-or-later) by the `dca` crate 0.9.0 of harletty-bridge
// (https://github.com/harletty/harletty-bridge, commit 74cf6ed), which published it as
// Apache-2.0. As FFmpeg-derived code it is used here under LGPL-2.1-or-later (docs/licensing.md
// §5). Modified for CFA4: see crates/dts/README.md. The harletty-bridge header follows.
//
//! Independent DTS Coherent Acoustics (DCA) bitstream parser and decoder.
//!
//! A native Rust port of ffmpeg's `dca` decoder, structured like the sibling
//! `eac3` crate. Phase 1 targets the DTS **core** (5.1 lossy bed); the EXSS +
//! XLL lossless extension (7.1) is added in a later phase.
//!
//! Reference C sources: FFmpeg's `libavcodec/dca*.c` (LGPL-2.1-or-later).

mod bitstream;
#[cfg(test)]
mod bitstream_writer;
pub mod cfa4;

mod dcadec;
pub mod extract;
pub mod hd;
pub mod parser;
mod pcm;
pub mod spatial;
mod types;

pub use dcadec::xmeta::{
    BedFold, FoldEstimator, FoldPlan, MAX_SOURCES, REFERENCE_CHANNELS, REFERENCE_MASK_5_1, REFERENCE_MASK_7_1, REFERENCE_SPEAKERS, SourceMetadata, SourceRole,
    SphericalPosition, XMetadata, XMetadataError, gain_code_linear,
};
pub use dcadec::xrender::FoldRenderer;
pub use extract::{ExtractError, Extractor, Frame};
pub use hd::{ExssKind, HdDecoder, HdError, HdFrame, exss_declared_size, exss_has_xll, exss_kind, exss_substream_size};
pub use parser::{AudioMode, FrameInfo, ParseError as HeaderParseError, SYNCWORD_CORE_BE, SYNCWORD_SUBSTREAM, parse_header};
pub use pcm::{CorePcmFrame, DecodeError, PcmDecoder, PcmPushResult, bed_layout};
pub use spatial::{SpatialChannel, XPresentation};
pub use types::BedChannel;
