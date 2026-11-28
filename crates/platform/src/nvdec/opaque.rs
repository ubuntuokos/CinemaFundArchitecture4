//! NVDEC output into CUDA arrays: the opaque (block-linear) layout of Video Codec SDK 13.1
//! (CFA4-NVDEC-CUARRAY-001). The decoder writes each picture into an array the application
//! allocated and registered, instead of its own surface pool that `cuvidMapVideoFrame` copies out
//! of. Sources: NVIDIA's NVDEC programming guide (13.1, "decoding to opaque output"), the CUDA
//! driver API reference (`cuArray3DCreate`, `cuArrayGetPlane`, `cuMemcpy2D`) and the MIT-licensed
//! headers transcribed in [`super::ffi`]. This module holds the safe part (the switch, the
//! driver and stream checks, the counters); the driver calls are in [`super::cuvid`], the NVDEC
//! FFI module.
//!
//! Off unless switched on ([`set_enabled`] or `CFA4_NVDEC_CUDA_ARRAY=1`), and only on a driver of
//! [`MIN_DRIVER`] or newer whose decoder reports the opaque output format and exports both
//! SDK 13.1 functions. Anything else, or any failure while setting it up, keeps the pitch-linear
//! path: an older driver never loses NVDEC.
use std::ffi::{c_int, c_uint};

use super::ffi::*;

/// The NVIDIA driver Video Codec SDK 13.1 requires.
pub const MIN_DRIVER: u32 = 610;

/// Switch CUDA-array output on or off for decoders created afterwards (process-wide; the same
/// switch as Settings ▸ Playback, `cfa4_codecs::hw::set_nvdec_cuda_array`). It overrides
/// `CFA4_NVDEC_CUDA_ARRAY`.
pub fn set_enabled(on: bool) {
    cfa4_codecs::hw::set_nvdec_cuda_array(on);
}

/// Whether new decoders try CUDA-array output: the switch ([`set_enabled`], the setting), else
/// `CFA4_NVDEC_CUDA_ARRAY=1` (for measurements), else off.
pub fn enabled() -> bool {
    cfa4_codecs::hw::nvdec_cuda_array().unwrap_or_else(|| std::env::var("CFA4_NVDEC_CUDA_ARRAY").is_ok_and(|v| v == "1"))
}

/// Pictures this process's decoders have read out of CUDA arrays.
pub fn array_frames() -> u64 {
    cfa4_codecs::hw::nvdec_cuda_array_frames()
}

pub(super) fn note_array_frame() {
    cfa4_codecs::hw::note_nvdec_cuda_array_frame();
}

/// The kernel module version from `/proc/driver/nvidia/version`'s first line: major and minor
/// (`… Kernel Module  610.43.02  …`, or `… Open Kernel Module for x86_64  610.43.02  …`).
pub fn parse_driver_version(text: &str) -> Option<(u32, u32)> {
    let line = text.lines().next()?;
    let rest = line.split("Kernel Module").nth(1)?;
    let version = rest.split_whitespace().find(|t| t.starts_with(|c: char| c.is_ascii_digit()))?;
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map_or(Some(0), |m| m.parse().ok())?;
    Some((major, minor))
}

/// The loaded NVIDIA kernel module's version, if any.
pub fn driver_version() -> Option<(u32, u32)> {
    parse_driver_version(&std::fs::read_to_string("/proc/driver/nvidia/version").ok()?)
}

/// The opaque decoder output for a bit depth: NV12 (8-bit) or P016 (10-bit).
pub fn surface_format(bits: u32) -> c_int {
    if bits == 10 { cudaVideoSurfaceFormat_P016_Opaque } else { cudaVideoSurfaceFormat_NV12_Opaque }
}

pub(super) fn array_format(bits: u32) -> c_uint {
    if bits == 10 { CU_AD_FORMAT_P016 } else { CU_AD_FORMAT_NV12 }
}

/// Why CUDA-array output cannot be used for this stream, or `Ok`.
pub fn eligible(driver: Option<(u32, u32)>, output_mask: u16, bits: u32) -> Result<(), String> {
    match driver {
        Some((major, _)) if major >= MIN_DRIVER => {}
        Some((major, minor)) => return Err(format!("NVIDIA driver {major}.{minor} is older than {MIN_DRIVER}")),
        None => return Err("the NVIDIA driver version is unknown".into()),
    }
    if !matches!(bits, 8 | 10) {
        return Err(format!("{bits}-bit output has no CUDA-array format here"));
    }
    let bit = 1u16.checked_shl(surface_format(bits) as u32).ok_or("NVDEC output format overflows")?;
    if output_mask & bit == 0 {
        return Err("the decoder does not offer CUDA-array output for this stream".into());
    }
    Ok(())
}

/// The bytes of one picture row in either plane (NV12 and P016 chroma rows are as wide as luma).
pub fn row_bytes(width: u32, bits: u32) -> Result<usize, String> {
    let bytes = width.checked_mul(if bits == 10 { 2 } else { 1 }).ok_or("NVDEC row size overflows")?;
    usize::try_from(bytes).map_err(|_| "NVDEC row size overflows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_versions_are_read_from_the_kernel_module_line() {
        let line = "NVRM version: NVIDIA UNIX x86_64 Kernel Module  610.43.02  Thu Oct  1 00:00:00 UTC 2026\nGCC version: gcc 15";
        assert_eq!(parse_driver_version(line), Some((610, 43)));
        assert_eq!(parse_driver_version("NVRM version: NVIDIA UNIX Open Kernel Module for x86_64  595.12  Release Build"), Some((595, 12)));
        assert_eq!(parse_driver_version("NVRM version: NVIDIA UNIX x86_64 Kernel Module  612  x"), Some((612, 0)));
        for bad in ["", "garbage", "NVRM version: NVIDIA UNIX x86_64 Kernel Module", "NVRM version: NVIDIA UNIX x86_64 Kernel Module  abc.1"] {
            assert_eq!(parse_driver_version(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn eligibility_needs_the_driver_and_the_output_format() {
        let nv12 = 1u16 << cudaVideoSurfaceFormat_NV12_Opaque;
        let p016 = 1u16 << cudaVideoSurfaceFormat_P016_Opaque;
        assert!(eligible(Some((610, 0)), nv12, 8).is_ok());
        assert!(eligible(Some((615, 71)), nv12 | p016, 10).is_ok());
        assert!(eligible(Some((609, 99)), nv12, 8).unwrap_err().contains("older than 610"));
        assert!(eligible(None, nv12, 8).is_err());
        assert!(eligible(Some((610, 0)), 1 << cudaVideoSurfaceFormat_NV12, 8).is_err(), "pitch-linear NV12 only");
        assert!(eligible(Some((610, 0)), nv12, 10).is_err(), "10-bit needs P016");
        assert!(eligible(Some((610, 0)), u16::MAX, 12).is_err());
    }

    #[test]
    fn formats_and_rows() {
        assert_eq!((surface_format(8), array_format(8)), (cudaVideoSurfaceFormat_NV12_Opaque, CU_AD_FORMAT_NV12));
        assert_eq!((surface_format(10), array_format(10)), (cudaVideoSurfaceFormat_P016_Opaque, CU_AD_FORMAT_P016));
        assert_eq!(row_bytes(1920, 8), Ok(1920));
        assert_eq!(row_bytes(1920, 10), Ok(3840));
        assert!(row_bytes(u32::MAX, 10).is_err());
    }

    #[test]
    fn the_switch_overrides_the_environment() {
        // the only test of this binary that touches the switch (it stays set afterwards)
        let from_env = std::env::var("CFA4_NVDEC_CUDA_ARRAY").is_ok_and(|v| v == "1");
        assert_eq!(enabled(), from_env, "unset: the environment decides (off without it)");
        set_enabled(true);
        assert!(enabled());
        set_enabled(false);
        assert!(!enabled(), "off wins over CFA4_NVDEC_CUDA_ARRAY=1 too");
    }
}
