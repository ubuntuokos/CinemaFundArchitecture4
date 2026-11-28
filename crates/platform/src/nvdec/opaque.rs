//! NVDEC output into CUDA arrays: the opaque (block-linear) layout of Video Codec SDK 13.1
//! (CFA4-NVDEC-CUARRAY-001). The decoder writes each picture into an array the application
//! allocated and registered, instead of its own surface pool that `cuvidMapVideoFrame` copies out
//! of. Sources: NVIDIA's NVDEC programming guide (13.1, "decoding to opaque output"), the CUDA
//! driver API reference (`cuArray3DCreate`, `cuArrayGetPlane`, `cuMemcpy2D`) and the MIT-licensed
//! headers transcribed in [`super::ffi`].
//!
//! Off unless switched on ([`set_enabled`] or `CFA4_NVDEC_CUDA_ARRAY=1`), and only on a driver of
//! [`MIN_DRIVER`] or newer whose decoder reports the opaque output format and exports both
//! SDK 13.1 functions. Anything else, or any failure while setting it up, keeps the pitch-linear
//! path: an older driver never loses NVDEC.
use std::ffi::{c_int, c_uint, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use super::ffi::*;

/// The NVIDIA driver Video Codec SDK 13.1 requires.
pub const MIN_DRIVER: u32 = 610;

static ENABLED: AtomicBool = AtomicBool::new(false);
static USED: AtomicBool = AtomicBool::new(false);

/// Whether a decoder of this process has decoded into CUDA arrays.
pub fn used() -> bool {
    USED.load(Ordering::Relaxed)
}

pub(super) fn note_used() {
    USED.store(true, Ordering::Relaxed);
}

/// Switch CUDA-array output on or off for decoders created afterwards (process-wide).
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// Whether new decoders try CUDA-array output: [`set_enabled`], or `CFA4_NVDEC_CUDA_ARRAY=1` for
/// measurements.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed) || std::env::var("CFA4_NVDEC_CUDA_ARRAY").is_ok_and(|v| v == "1")
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

fn array_format(bits: u32) -> c_uint {
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

/// The SDK 13.1 NVDEC functions and the CUDA array functions the opaque path calls.
#[derive(Clone, Copy)]
pub struct OpaqueApi {
    register: RegisterDecodeSurfaces,
    pub(super) decode_async: DecodePictureAsync,
    create: Array3DCreate,
    destroy: ArrayDestroy,
    plane: ArrayGetPlane,
    copy: Memcpy2D,
}

impl OpaqueApi {
    /// The functions from the already loaded `libnvcuvid` and from `libcuda`, which the caller keeps
    /// loaded (the returned library) as long as any of them may be called.
    pub fn load(nvcuvid: &libloading::Library) -> Result<(Self, libloading::Library), String> {
        // SAFETY: the symbols have the signatures of NVIDIA's public headers (`super::ffi`). The
        // caller retains both libraries until every handle using these pointers is destroyed.
        unsafe {
            let cuda = libloading::Library::new("libcuda.so.1").map_err(|e| format!("no CUDA driver library: {e}"))?;
            let missing = |e: libloading::Error| format!("the driver lacks a Video Codec SDK 13.1 function: {e}");
            let api = Self {
                register: *nvcuvid.get(b"cuvidRegisterDecodeSurfaces\0").map_err(missing)?,
                decode_async: *nvcuvid.get(b"cuvidDecodePictureAsync\0").map_err(missing)?,
                create: *cuda.get(b"cuArray3DCreate_v2\0").map_err(missing)?,
                destroy: *cuda.get(b"cuArrayDestroy\0").map_err(missing)?,
                plane: *cuda.get(b"cuArrayGetPlane\0").map_err(missing)?,
                copy: *cuda.get(b"cuMemcpy2D_v2\0").map_err(missing)?,
            };
            Ok((api, cuda))
        }
    }
}

fn status(code: i32, operation: &str) -> Result<(), String> {
    if code == 0 { Ok(()) } else { Err(format!("{operation} failed ({code})")) }
}

/// The bytes of one picture row in either plane (NV12 and P016 chroma rows are as wide as luma).
pub fn row_bytes(width: u32, bits: u32) -> Result<usize, String> {
    let bytes = width.checked_mul(if bits == 10 { 2 } else { 1 }).ok_or("NVDEC row size overflows")?;
    usize::try_from(bytes).map_err(|_| "NVDEC row size overflows".into())
}

/// Decode surfaces as CUDA arrays, one per decoder picture index. Created, registered, read and
/// destroyed with the CUDA context current on the calling thread.
pub(super) struct Arrays {
    pub(super) api: OpaqueApi,
    list: Vec<*mut c_void>,
    coded: (u32, u32),
    bits: u32,
}

impl Arrays {
    /// `count` arrays for `coded`-sized pictures of `bits` depth.
    pub(super) fn allocate(api: OpaqueApi, coded: (u32, u32), bits: u32, count: u32) -> Result<Self, String> {
        if count == 0 || count > MAX_NUM_REGISTERED_DECODE_SURFACES {
            return Err(format!("{count} decode surfaces cannot be registered (at most {MAX_NUM_REGISTERED_DECODE_SURFACES})"));
        }
        let mut arrays = Self { api, list: Vec::new(), coded, bits };
        arrays.list.try_reserve_exact(count as usize).map_err(|e| format!("NVDEC array list allocation failed: {e}"))?;
        let desc = CUDA_ARRAY3D_DESCRIPTOR {
            Width: coded.0 as usize,
            Height: coded.1 as usize,
            Depth: 0,
            Format: array_format(bits),
            NumChannels: 3,
            Flags: CUDA_ARRAY3D_SURFACE_LDST | CUDA_ARRAY3D_VIDEO_ENCODE_DECODE,
        };
        for _ in 0..count {
            let mut array = ptr::null_mut();
            // SAFETY: desc and the out-pointer are live; the context is current (caller). An array
            // created here is owned by `arrays` and destroyed in its Drop, also on an early return.
            status(unsafe { (api.create)(&mut array, &desc) }, "cuArray3DCreate")?;
            if array.is_null() {
                return Err("cuArray3DCreate returned a null array".into());
            }
            arrays.list.push(array);
        }
        Ok(arrays)
    }

    /// Hand the arrays to `decoder` as its decode surfaces.
    pub(super) fn register(&mut self, decoder: *mut c_void) -> Result<(), String> {
        // SAFETY: plain C data (integers and nullable pointers) permits an all-zero value.
        let mut info: CUVIDREGISTERDECODESURFACESINFO = unsafe { std::mem::zeroed() };
        info.ulNumDecodeSurfaces = u32::try_from(self.list.len()).map_err(|_| "NVDEC surface count overflows")?;
        info.pDecodeSurfaces = self.list.as_mut_ptr();
        // SAFETY: the live decoder and the array list (owned by self, unchanged while registered)
        // outlive the call; the context is current (caller).
        status(unsafe { (self.api.register)(decoder, &mut info) }, "cuvidRegisterDecodeSurfaces")
    }

    /// Copy picture `index` into `host`: the luma rows, then the interleaved chroma rows, each
    /// [`row_bytes`] long with no padding, the layout `Biplanar` reads with that stride.
    pub(super) fn read(&self, index: i32, host: &mut [u8]) -> Result<(), String> {
        let array = usize::try_from(index).ok().and_then(|i| self.list.get(i)).copied().ok_or("NVDEC picture index has no CUDA array")?;
        let row = row_bytes(self.coded.0, self.bits)?;
        let rows = self.coded.1 as usize;
        let luma = row.checked_mul(rows).ok_or("NVDEC luma size overflows")?;
        let chroma = row.checked_mul(rows / 2).ok_or("NVDEC chroma size overflows")?;
        if luma.checked_add(chroma) != Some(host.len()) {
            return Err("NVDEC host buffer does not fit the picture".into());
        }
        let (luma_host, chroma_host) = host.split_at_mut(luma);
        for (plane, (dst, rows)) in [(luma_host, rows), (chroma_host, rows / 2)].into_iter().enumerate() {
            let mut source = ptr::null_mut();
            // SAFETY: `array` is a live multi-planar array owned by self; the out-pointer is live.
            status(unsafe { (self.api.plane)(&mut source, array, plane as c_uint) }, "cuArrayGetPlane")?;
            // SAFETY: plain C data (integers and nullable pointers) permits an all-zero value.
            let mut copy: CUDA_MEMCPY2D = unsafe { std::mem::zeroed() };
            copy.srcMemoryType = CU_MEMORYTYPE_ARRAY;
            copy.srcArray = source;
            copy.dstMemoryType = CU_MEMORYTYPE_HOST;
            copy.dstHost = dst.as_mut_ptr().cast();
            copy.dstPitch = row;
            copy.WidthInBytes = row;
            copy.Height = rows;
            // SAFETY: the synchronous copy writes `row * rows` bytes, exactly `dst`'s length, into
            // the live slice; it is ordered after the asynchronous decode on the legacy default
            // stream. The context is current (caller).
            status(unsafe { (self.api.copy)(&copy) }, "cuMemcpy2D")?;
        }
        Ok(())
    }
}

impl Drop for Arrays {
    fn drop(&mut self) {
        for array in self.list.drain(..) {
            // SAFETY: each array was created by this object and is no longer used by a decoder: the
            // owner drops the arrays only after destroying the decoder, with the context current.
            if let Err(e) = status(unsafe { (self.api.destroy)(array) }, "cuArrayDestroy") {
                log::warn!("{e}");
            }
        }
    }
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
    fn the_switch_is_off_by_default() {
        // other tests in this binary do not switch it
        if std::env::var("CFA4_NVDEC_CUDA_ARRAY").is_err() {
            assert!(!enabled());
            set_enabled(true);
            assert!(enabled());
            set_enabled(false);
            assert!(!enabled());
        }
    }
}
