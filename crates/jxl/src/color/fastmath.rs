// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-color 0.11.0.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

//! Fast math routines, ported from libjxl.

mod powf;
pub mod rational_poly;

pub use powf::*;
