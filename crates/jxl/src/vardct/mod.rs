// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-vardct 0.11.1.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

//! This crate provides types related to representation of VarDCT frames, such as
//! [varblock transform types][TransformType], [LF images][LfCoeff],
//! [dequantization matrices][DequantMatrixSet] and [HF coefficients][write_hf_coeff].
//!
//! Actual decoding (dequantization and rendering) of such frames is not done in this crate.
mod dct_select;
mod dequant;
mod error;
mod hf_coeff;
mod hf_metadata;
mod hf_pass;
mod lf;

pub use dct_select::TransformType;
pub use dequant::*;
pub use error::{Error, Result};
pub use hf_coeff::*;
pub use hf_metadata::*;
pub use hf_pass::*;
pub use lf::*;
