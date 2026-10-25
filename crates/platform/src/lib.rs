//! OS media integration (layer L5): hardware video decoding and encoding through the operating
//! system's codecs.
//!
//! [`register`] puts the platform's hardware decoder factory in front of CFA4's own decoders
//! (`cfa4_codecs::register_video_decoder`). Today that is VideoToolbox on macOS for H.264
//! (`avcC`) and HEVC (`hvcC`) streams, 8- and 10-bit, 4:2:0 and 4:2:2; Media Foundation / DXVA on
//! Windows; NVDEC ([`nvdec`], NVIDIA's driver) and VA-API ([`vaapi`]) on Linux for H.264 and HEVC; on other systems (and on
//! Linux with neither) registration does nothing and reports [`Availability::Unavailable`]. It also registers a
//! hardware H.264 encoder factory (`cfa4_export::register_encoder`) that only acts when an
//! export asks for it (`ExportSettings::hardware_encoding` = `Auto`), see [`hardware_encode`]; on
//! macOS and on Windows (NVENC, [`nvenc`]) it also makes the H.265 export format available, and on Windows
//! (Main 10) lets HDR sequences export as HDR H.265.
//!
//! Hardware decoding never makes a file undecodable:
//!
//! - the factory declines (falls through to the software decoder) when Settings ▸ Playback ▸
//!   Hardware decoding is Off (`cfa4_codecs::hw::set_hardware_decoding`), when the stream's
//!   format is one the hardware path does not take, or when the OS cannot create a hardware
//!   session for it (profile, size, no hardware decoder);
//! - a decoder that fails mid-stream switches to the software decoder transparently
//!   ([`HybridDecoder`]) and logs it.
//!
//! Pictures are the software decoder's: same planes (bit-exact on the parity fixtures), colour,
//! pixel aspect, pts and presentation order, so the two are interchangeable.
//!
//! This is the one crate allowed to use `unsafe` (OS FFI), and only in its FFI modules
//! (docs/adr/0001-platform-ffi.md).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

// Used by the Windows (and, for `biplanar`, Linux) decoders; compiled everywhere so their tests run
// on every system.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod annexb;
#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
mod biplanar;
pub mod cursor;
#[cfg(target_os = "macos")]
pub mod hardware_encode;
pub mod hybrid;
#[cfg(target_os = "windows")]
pub mod media_foundation;
#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
pub mod nvdec;
#[cfg(any(target_os = "windows", all(target_os = "linux", target_pointer_width = "64")))]
pub mod nvenc;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub mod vaapi;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub mod videotoolbox;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub mod videotoolbox_encode;

pub use hybrid::HybridDecoder;

/// What [`register`] made available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    /// A hardware decoder factory was registered (its name).
    Available(&'static str),
    /// Nothing to register on this system (why).
    Unavailable(&'static str),
}

#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
#[derive(Debug, PartialEq, Eq)]
enum LinuxDecoderProbe {
    Nvdec(String),
    Vaapi { driver: String, nvdec_error: String },
    Unavailable { nvdec_error: String, vaapi_error: String },
}

#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
fn probe_linux_decoder(nvdec_probe: impl FnOnce() -> Result<String, String>, vaapi_probe: impl FnOnce() -> Result<String, String>) -> LinuxDecoderProbe {
    let nvdec_error = match nvdec_probe() {
        Ok(driver) => return LinuxDecoderProbe::Nvdec(driver),
        Err(error) => error,
    };
    match vaapi_probe() {
        Ok(driver) => LinuxDecoderProbe::Vaapi { driver, nvdec_error },
        Err(vaapi_error) => LinuxDecoderProbe::Unavailable { nvdec_error, vaapi_error },
    }
}

/// Register the platform's hardware video decoders (call once at startup; repeated calls are
/// harmless). Streams they do not take, and every stream while hardware decoding is Off, keep
/// using CFA4's own decoders.
pub fn register() -> Availability {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        cfa4_export::register_encoder(nvenc::export::factory);
        // Linux uses the same CUDA NVENC HEVC implementation as H.264. A
        // successful hardware probe is required before the HEVC format is
        // offered, instead of the former Windows-only Format::Hevc gate.
        cfa4_export::register_format_probe(cfa4_export::Format::Hevc, nvenc::hevc_available);
        cfa4_export::register_hdr_probe(cfa4_export::Format::Hevc, nvenc::hevc_hdr_available);
        log::info!("startup stage: scheduling NVENC HEVC probe");
        nvenc::warm_hevc_probe();
        log::info!("startup stage: NVENC HEVC probe scheduled");
    }
    #[cfg(target_os = "macos")]
    {
        cfa4_codecs::register_video_decoder(videotoolbox_factory);
        cfa4_export::register_encoder(hardware_encode::videotoolbox_encoder_factory);
        cfa4_export::register_format_probe(cfa4_export::Format::Hevc, hardware_encode::hevc_available);
        // the probe creates a hardware session (about 0.1 s): answer it now, off the UI thread,
        // before the first draw of the format list asks
        hardware_encode::warm_hevc_probe();
        cfa4_codecs::hw::set_hw_backend("VideoToolbox");
        Availability::Available("VideoToolbox")
    }
    // Export ▸ H.265: NVENC is CFA4's only HEVC encoder on Windows, so the format is available
    // when a small HEVC session opens. That takes about half a second, so it is asked on a thread of
    // its own now rather than by the first draw of the format list.
    #[cfg(target_os = "windows")]
    {
        cfa4_export::register_format_probe(cfa4_export::Format::Hevc, nvenc::hevc_available);
        // HDR sequences export as HEVC Main 10 (PQ / HLG) only where NVENC has a 10-bit encoder; elsewhere
        // (and on macOS, whose VideoToolbox path is 8-bit) they are tone-mapped to SDR, as before
        cfa4_export::register_hdr_probe(cfa4_export::Format::Hevc, nvenc::hevc_hdr_available);
        nvenc::warm_hevc_probe();
    }
    #[cfg(target_os = "windows")]
    {
        static ENCODERS: std::sync::Once = std::sync::Once::new();
        // Export ▸ Hardware encoding (NVENC H.264): in front of the software encoder, taking an
        // export only when asked for and when NVENC can do it
        ENCODERS.call_once(|| cfa4_export::register_encoder(nvenc::export::factory));
        cfa4_codecs::register_video_decoder(media_foundation_factory);
        cfa4_codecs::hw::set_hw_backend("Media Foundation");
        Availability::Available("Media Foundation")
    }
    #[cfg(target_os = "linux")]
    {
        static VAAPI: std::sync::Once = std::sync::Once::new();
        static NVDEC: std::sync::Once = std::sync::Once::new();
        let mut available = Availability::Unavailable("no VA-API driver (libva and a DRM render node) and no NVIDIA driver (NVDEC)");
        #[cfg(target_pointer_width = "64")]
        {
            // Probe NVIDIA first. Some NVIDIA DRM nodes expose a libva implementation whose
            // vaInitialize call can block indefinitely; a working NVDEC path makes that fallback
            // unnecessary, since unsupported streams still use CFA4's software decoders.
            match probe_linux_decoder(
                || {
                    log::info!("startup stage: probing NVDEC CUDA device");
                    let result = nvdec::cuvid::probe();
                    log::info!("startup stage: NVDEC CUDA device probe returned");
                    result
                },
                || {
                    log::info!("startup stage: probing VA-API display");
                    let result = (*vaapi::va::probe()).clone();
                    log::info!("startup stage: VA-API display probe returned");
                    result
                },
            ) {
                LinuxDecoderProbe::Nvdec(driver) => {
                    NVDEC.call_once(|| log::info!("hardware decoding through NVDEC: {driver}"));
                    cfa4_codecs::register_video_decoder(nvdec_factory);
                    cfa4_codecs::hw::set_hw_backend("NVDEC");
                    available = Availability::Available("NVDEC");
                }
                LinuxDecoderProbe::Vaapi { driver, nvdec_error } => {
                    log::info!("no NVDEC hardware decoding: {nvdec_error}");
                    VAAPI.call_once(|| log::info!("hardware decoding through VA-API: {driver}"));
                    cfa4_codecs::register_video_decoder(vaapi_factory);
                    cfa4_codecs::hw::set_hw_backend("VA-API");
                    available = Availability::Available("VA-API");
                }
                LinuxDecoderProbe::Unavailable { nvdec_error, vaapi_error } => {
                    log::info!("no NVDEC hardware decoding: {nvdec_error}");
                    log::info!("no VA-API hardware decoding: {vaapi_error}");
                }
            }
        }
        #[cfg(not(target_pointer_width = "64"))]
        {
            log::info!("startup stage: probing VA-API display");
            match vaapi::va::probe() {
                Ok(driver) => {
                    VAAPI.call_once(|| log::info!("hardware decoding through VA-API: {driver}"));
                    cfa4_codecs::register_video_decoder(vaapi_factory);
                    cfa4_codecs::hw::set_hw_backend("VA-API");
                    available = Availability::Available("VA-API");
                }
                Err(why) => log::info!("no VA-API hardware decoding: {why}"),
            }
        }
        available
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Availability::Unavailable("no hardware video decoder for this system yet")
    }
}

/// Whether [`register`] has put a hardware decoder factory in front of our decoders.
pub fn registered() -> bool {
    #[cfg(target_os = "macos")]
    {
        cfa4_codecs::video_decoder_registered(videotoolbox_factory)
    }
    #[cfg(target_os = "windows")]
    {
        cfa4_codecs::video_decoder_registered(media_foundation_factory)
    }
    #[cfg(target_os = "linux")]
    {
        #[cfg(target_pointer_width = "64")]
        if cfa4_codecs::video_decoder_registered(nvdec_factory) {
            return true;
        }
        cfa4_codecs::video_decoder_registered(vaapi_factory)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        false
    }
}

/// Whether this system's hardware decoder takes the stream of `entry` (whatever the Hardware
/// decoding setting says): diagnostics and tests.
pub fn hardware_decoder_for(entry: &cfa4_isobmff::SampleEntry) -> bool {
    #[cfg(target_os = "macos")]
    {
        cfa4_codecs::hw::NalStreamInfo::from_entry(entry).and_then(|r| r.ok()).is_some_and(|info| videotoolbox::VtDecoder::new(info).is_ok())
    }
    #[cfg(target_os = "windows")]
    {
        media_foundation::stream_info(entry).is_some_and(|info| media_foundation::MfDecoder::new(info).is_ok())
    }
    #[cfg(target_os = "linux")]
    {
        cfa4_codecs::hw::NalStreamInfo::from_entry(entry).and_then(|r| r.ok()).is_some_and(|info| {
            #[cfg(target_pointer_width = "64")]
            if nvdec::NvDecoder::new(info.clone()).is_ok() {
                return true;
            }
            vaapi::VaDecoder::new(info).is_ok()
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = entry;
        false
    }
}

/// The VideoToolbox factory: a [`HybridDecoder`] around [`videotoolbox::VtDecoder`] for `avcC` /
/// `hvcC` streams VideoToolbox can decode in hardware, `None` otherwise.
#[cfg(target_os = "macos")]
pub fn videotoolbox_factory(entry: &cfa4_isobmff::SampleEntry) -> Option<cfa4_codecs::Result<Box<dyn cfa4_codecs::VideoDecoder>>> {
    if !cfa4_codecs::hw::hardware_decoding() {
        return None;
    }
    let info = cfa4_codecs::hw::NalStreamInfo::from_entry(entry)?.ok()?;
    match videotoolbox::VtDecoder::new(info.clone()) {
        Ok(vt) => Some(Ok(Box::new(HybridDecoder::new(Box::new(vt), entry.clone(), info)))),
        Err(why) => {
            log::info!("hardware decoding declined for {} video: {why}", entry.codec.name());
            cfa4_codecs::hw::note_hw_declined();
            None
        }
    }
}

/// The Media Foundation factory: a [`HybridDecoder`] around [`media_foundation::MfDecoder`] for
/// H.264 / HEVC / VP9 / AV1 streams a Direct3D-aware decoder MFT can decode with DXVA on this
/// system's GPU, `None` otherwise.
#[cfg(target_os = "windows")]
pub fn media_foundation_factory(entry: &cfa4_isobmff::SampleEntry) -> Option<cfa4_codecs::Result<Box<dyn cfa4_codecs::VideoDecoder>>> {
    if !cfa4_codecs::hw::hardware_decoding() {
        return None;
    }
    let info = media_foundation::stream_info(entry)?;
    match media_foundation::MfDecoder::new(info.clone()) {
        Ok(mf) => Some(Ok(Box::new(HybridDecoder::new(Box::new(mf), entry.clone(), info)))),
        Err(why) => {
            log::info!("hardware decoding declined for {} video: {why}", entry.codec.name());
            cfa4_codecs::hw::note_hw_declined();
            None
        }
    }
}

/// The VA-API factory: a [`HybridDecoder`] around [`vaapi::VaDecoder`] for H.264 and HEVC
/// streams this system's VA-API driver decodes, `None` otherwise (other codecs keep the software
/// decoders).
#[cfg(target_os = "linux")]
pub fn vaapi_factory(entry: &cfa4_isobmff::SampleEntry) -> Option<cfa4_codecs::Result<Box<dyn cfa4_codecs::VideoDecoder>>> {
    if !cfa4_codecs::hw::hardware_decoding() {
        return None;
    }
    let info = cfa4_codecs::hw::NalStreamInfo::from_entry(entry)?.ok()?;
    match vaapi::VaDecoder::new(info.clone()) {
        Ok(va) => Some(Ok(Box::new(HybridDecoder::new(Box::new(va), entry.clone(), info)))),
        Err(why) => {
            log::info!("hardware decoding declined for {} video: {why}", entry.codec.name());
            cfa4_codecs::hw::note_hw_declined();
            None
        }
    }
}

/// The NVDEC factory: a [`HybridDecoder`] around [`nvdec::NvDecoder`] for H.264 and HEVC streams
/// this system's NVIDIA GPU decodes, `None` otherwise (VA-API and the software decoders keep those).
#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
pub fn nvdec_factory(entry: &cfa4_isobmff::SampleEntry) -> Option<cfa4_codecs::Result<Box<dyn cfa4_codecs::VideoDecoder>>> {
    if !cfa4_codecs::hw::hardware_decoding() {
        return None;
    }
    let info = cfa4_codecs::hw::NalStreamInfo::from_entry(entry)?.ok()?;
    match nvdec::NvDecoder::new(info.clone()) {
        Ok(nv) => Some(Ok(Box::new(HybridDecoder::new(Box::new(nv), entry.clone(), info)))),
        Err(why) => {
            log::info!("NVDEC declined {} video: {why}", entry.codec.name());
            // with VA-API behind this factory, that one counts the decline (or takes the stream)
            if !cfa4_codecs::video_decoder_registered(vaapi_factory) {
                cfa4_codecs::hw::note_hw_declined();
            }
            None
        }
    }
}

#[cfg(all(test, target_os = "linux", target_pointer_width = "64"))]
mod linux_probe_tests {
    use std::cell::Cell;

    use super::{LinuxDecoderProbe, probe_linux_decoder};

    #[test]
    fn successful_nvdec_probe_skips_blocking_vaapi_probe() {
        let vaapi_called = Cell::new(false);

        let selected = probe_linux_decoder(
            || Ok("NVDEC test device".to_string()),
            || {
                vaapi_called.set(true);
                Ok("VA-API test device".to_string())
            },
        );

        assert_eq!(selected, LinuxDecoderProbe::Nvdec("NVDEC test device".to_string()));
        assert!(!vaapi_called.get(), "VA-API probe must not run after NVDEC succeeds");
    }
}
