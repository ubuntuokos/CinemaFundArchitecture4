//! Image fidelity metrics for comparing our renders with a reference: CIEDE2000 colour difference
//! (ΔE₀₀) and SSIM. These are the building blocks of the output-comparison harness of
//! docs/gaps.md G1 (per-effect PSNR / ΔE scores).
//!
//! Provenance: the CIEDE2000 formula and the Gaussian SSIM window follow
//! `oximedia-quality` (`ciede2000.rs`, `ssim.rs`) of OxiMedia 0.2.1 (Apache-2.0), rewritten for
//! CFA4 (no panics, separable SSIM filtering, sRGB → Lab through `cfa4-color`); see
//! docs/licensing.md §5. Both are checked against independent references: the CIEDE2000 test
//! data of Sharma, Wu and Dalal (2005) and scikit-image's `structural_similarity`.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

mod ciede2000;
mod lab;
mod ssim;

pub use ciede2000::{DeltaEParams, DeltaEStats, ciede2000, ciede2000_with, delta_e_srgb8};
pub use lab::{Lab, linear_rgb_to_lab, srgb8_to_lab, xyz_to_lab};
pub use ssim::{SSIM_WINDOW, ssim, ssim_u8, ssim_u16};

/// Why a metric could not be computed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetricError {
    /// The two images or sample buffers differ in size.
    SizeMismatch { reference: usize, distorted: usize },
    /// A buffer does not hold `width × height × channels` samples.
    BadLength { expected: usize, got: usize },
    /// The image is smaller than the metric's window, or empty.
    TooSmall { width: usize, height: usize },
    /// Unsupported channel count (ΔE takes 3 = RGB or 4 = RGBA).
    Channels(usize),
    /// `width × height` overflows or exceeds [`MAX_PIXELS`].
    TooLarge,
    /// `data_range` is not a positive finite number.
    DataRange,
    /// A CIEDE2000 weighting factor (kL, kC, kH) is not a positive finite number.
    Weights,
}

impl std::fmt::Display for MetricError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MetricError::SizeMismatch { reference, distorted } => {
                write!(f, "reference has {reference} samples, distorted has {distorted}")
            }
            MetricError::BadLength { expected, got } => write!(f, "expected {expected} samples, got {got}"),
            MetricError::TooSmall { width, height } => write!(f, "{width}x{height} is too small for this metric"),
            MetricError::Channels(c) => write!(f, "{c} channels: expected 3 (RGB) or 4 (RGBA)"),
            MetricError::TooLarge => write!(f, "image too large"),
            MetricError::DataRange => write!(f, "data range must be a positive finite number"),
            MetricError::Weights => write!(f, "CIEDE2000 weighting factors must be positive finite numbers"),
        }
    }
}

impl std::error::Error for MetricError {}

/// Largest image the metrics accept (16K × 16K), so a hostile size cannot force a huge allocation.
pub const MAX_PIXELS: usize = 16_384 * 16_384;

/// `width × height`, checked against overflow and [`MAX_PIXELS`].
fn pixel_count(width: usize, height: usize) -> Result<usize, MetricError> {
    match width.checked_mul(height) {
        Some(n) if n <= MAX_PIXELS => Ok(n),
        _ => Err(MetricError::TooLarge),
    }
}
