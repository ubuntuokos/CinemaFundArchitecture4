// Copyright Rainbaby, from the `truehd` crate 0.7.2 (Apache-2.0, crates/truehd/LICENSE).
// Modified for CFA4: see crates/truehd/README.md for the list of changes.
//! Utility functions and supporting infrastructure.
//!
//! Provides bitstream I/O, CRC validation, Huffman decoding, error handling,
//! buffer management, and other supporting functionality for processing.

pub mod bitstream_io;
pub mod buffer_pool;
pub mod crc;
pub mod diagnostic;
pub mod dither;
pub mod errors;
pub mod fifo;
pub mod perf;
pub mod timing;
