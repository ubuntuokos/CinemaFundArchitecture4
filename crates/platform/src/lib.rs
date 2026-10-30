//! OS media integration (layer L5): hardware video decoding and encoding through the operating
//! system's codecs.
//!
//! [`register`] puts the platform's hardware decoder factory in front of CFA4's own decoders
//! (`cfa4_codecs::register_video_decoder`): NVDEC ([`nvdec`], NVIDIA's driver) and VA-API
//! ([`vaapi`]) for H.264 and HEVC; without either, registration does nothing and reports [`Availability::Unavailable`]. It also registers a
//! hardware H.264 encoder factory (`cfa4_export::register_encoder`) that only acts when an
//! export asks for it (`ExportSettings::hardware_encoding` = `Auto`), see [`hardware_encode`]; with
//! NVENC ([`nvenc`]) it also makes the H.265 export format available and (Main 10) lets HDR
//! sequences export as HDR H.265.
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

// Used by the Linux decoders (NVDEC, VA-API); compiled everywhere so their tests run.
#[cfg_attr(not(all(target_os = "linux", target_pointer_width = "64")), allow(dead_code))]
mod annexb;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod biplanar;
pub mod cursor;
pub mod hybrid;
#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
pub mod nvdec;
#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
pub mod nvenc;
#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
pub mod routes;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub mod vaapi;

pub use hybrid::HybridDecoder;

/// What [`register`] made available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    /// A hardware decoder factory was registered (its name).
    Available(&'static str),
    /// Nothing to register on this system (why).
    Unavailable(&'static str),
}

/// Register the platform's hardware video decoders (call once at startup; repeated calls are
/// harmless). Streams they do not take, and every stream while hardware decoding is Off, keep
/// using CFA4's own decoders.
pub fn register() -> Availability {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        // The H.264 factory is registered on every machine, as before; it asks the compute router
        // (`routes::nvenc_allowed`) before it opens the driver, and an export with hardware
        // encoding Auto that the router sends to the software encoder counts as declined.
        cfa4_export::register_encoder(nvenc::export::factory);
        // Linux uses the same CUDA NVENC HEVC implementation as H.264. A successful hardware
        // probe is required before the HEVC format is offered; the probe is only registered (and
        // warmed) where the router routes H.265 encoding (`video.encode.hevc`, its own task) to
        // NVENC, so other machines never load the CUDA driver for it (the format was unavailable
        // there anyway).
        if routes::current().is_none_or(|r| r.hevc_encoder() == "nvenc") {
            cfa4_export::register_format_probe(cfa4_export::Format::Hevc, nvenc::hevc_available);
            cfa4_export::register_hdr_probe(cfa4_export::Format::Hevc, nvenc::hevc_hdr_available);
            log::info!("startup stage: scheduling NVENC HEVC probe");
            nvenc::warm_hevc_probe();
            log::info!("startup stage: NVENC HEVC probe scheduled");
        }
    }
    #[cfg(target_os = "linux")]
    {
        static VAAPI: std::sync::Once = std::sync::Once::new();
        static NVDEC: std::sync::Once = std::sync::Once::new();
        let mut available = Availability::Unavailable("no VA-API driver (libva and a DRM render node) and no NVIDIA driver (NVDEC)");
        #[cfg(target_pointer_width = "64")]
        {
            // The compute router decides (ADR 0004): the shipped order is NVDEC, VA-API, then the
            // software decoders. NVIDIA comes first because some NVIDIA DRM nodes expose a libva
            // implementation whose vaInitialize can block indefinitely; the router asks VA-API only
            // when NVDEC declined, and unsupported streams still use CFA4's software decoders.
            match routes::current().map(|r| (r.decoder(), &r.decode)) {
                Some(("nvdec", decision)) => {
                    NVDEC.call_once(|| log::info!("hardware decoding through NVDEC on {}", decision.device()));
                    cfa4_codecs::register_video_decoder(nvdec_factory);
                    cfa4_codecs::hw::set_hw_backend("NVDEC");
                    available = Availability::Available("NVDEC");
                }
                Some(("vaapi", decision)) => {
                    VAAPI.call_once(|| log::info!("hardware decoding through VA-API on {}", decision.device()));
                    cfa4_codecs::register_video_decoder(vaapi_factory);
                    cfa4_codecs::hw::set_hw_backend("VA-API");
                    available = Availability::Available("VA-API");
                }
                Some((other, decision)) => {
                    log::info!("no hardware decoding: the compute router chose {other} ({} routes passed over)", decision.chain.declined.len());
                }
                None => log::warn!("no hardware decoding: the compute router could not route the decoder"),
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

    #[cfg(not(target_os = "linux"))]
    {
        Availability::Unavailable("no hardware video decoder for this system yet")
    }
}

/// Whether [`register`] has put a hardware decoder factory in front of our decoders.
pub fn registered() -> bool {
    #[cfg(target_os = "linux")]
    {
        #[cfg(target_pointer_width = "64")]
        if cfa4_codecs::video_decoder_registered(nvdec_factory) {
            return true;
        }
        cfa4_codecs::video_decoder_registered(vaapi_factory)
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Whether this system's hardware decoder takes the stream of `entry` (whatever the Hardware
/// decoding setting says): diagnostics and tests.
pub fn hardware_decoder_for(entry: &cfa4_isobmff::SampleEntry) -> bool {
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
    #[cfg(not(target_os = "linux"))]
    {
        let _ = entry;
        false
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
