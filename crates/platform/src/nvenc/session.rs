//! An NVENC (H.264 or HEVC) encode session: the driver library, the device it is opened on, the
//! encoder configuration, and the input / output buffers (every call into the driver).
//!
//! FFI module (docs/adr/0001-platform-ffi.md): every `unsafe` block has a `// SAFETY:` comment, the
//! public items are safe and return `Result<_, String>`, no driver pointer leaves this module.

use std::ffi::{CStr, c_void};
use std::sync::OnceLock;

use super::device::Device;
use super::ffi::*;

/// The driver's entry points (loaded once).
struct Api {
    list: NV_ENCODE_API_FUNCTION_LIST,
    /// The API level this process speaks to the driver and its structure version tags.
    level: ApiLevel,
    ver: Versions,
    #[cfg(target_os = "linux")]
    _library: libloading::Library,
}

// SAFETY: the function table is plain function pointers, immutable after loading; NVENC's API is
// callable from any thread (calls on one encoder session are serialised by `&mut Session`).
unsafe impl Send for Api {}
// SAFETY: see above.
unsafe impl Sync for Api {}

fn api() -> Result<&'static Api, String> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    API.get_or_init(load).as_ref().map_err(Clone::clone)
}

#[cfg(target_os = "linux")]
fn load() -> Result<Api, String> {
    // SAFETY: runtime-load NVIDIA's driver and use the signatures from nvEncodeAPI.h.
    // The library lives alongside the immutable function table, for the process lifetime.
    unsafe {
        let library = libloading::Library::new("libnvidia-encode.so.1").map_err(|e| format!("no NVIDIA encoder driver: {e}"))?;
        let version: libloading::Symbol<'_, MaxSupportedVersionFn> = library.get(b"NvEncodeAPIGetMaxSupportedVersion\0").map_err(|e| e.to_string())?;
        let create: libloading::Symbol<'_, CreateInstanceFn> = library.get(b"NvEncodeAPICreateInstance\0").map_err(|e| e.to_string())?;
        let mut v = 0;
        if version(&mut v) != NV_ENC_SUCCESS {
            return Err("cannot query the NVENC version".into());
        }
        let level = choose_level(v, std::env::var("CFA4_NVENC_API").ok().as_deref())?;
        let ver = level.versions();
        let mut list: NV_ENCODE_API_FUNCTION_LIST = std::mem::zeroed();
        list.version = ver.function_list;
        if create(&mut list) != NV_ENC_SUCCESS {
            return Err("NvEncodeAPICreateInstance failed".into());
        }
        Ok(Api { list, level, ver, _library: library })
    }
}

/// The API level for a driver reporting `max` (`NvEncodeAPIGetMaxSupportedVersion`), honouring
/// `CFA4_NVENC_API` (`12.1` forces the older level on a newer driver, to test it there; `13.1` asks for
/// the newer one and fails when the driver cannot do it).
fn choose_level(max: u32, forced: Option<&str>) -> Result<ApiLevel, String> {
    let best = ApiLevel::for_driver(max).ok_or_else(|| {
        let ours = ApiLevel::V12_1.packed();
        format!("the NVIDIA driver's NVENC ({}.{}) is older than {}.{}", max >> 4, max & 0xf, ours >> 4, ours & 0xf)
    })?;
    match forced.map(str::trim) {
        None | Some("") | Some("auto") => Ok(best),
        Some("12.1") => Ok(ApiLevel::V12_1),
        Some("13.1") if best >= ApiLevel::V13_1 => Ok(ApiLevel::V13_1),
        Some("13.1") => Err(format!("CFA4_NVENC_API=13.1, but the driver's NVENC is {}.{}", max >> 4, max & 0xf)),
        Some(other) => Err(format!("CFA4_NVENC_API={other} is not auto, 12.1 or 13.1")),
    }
}

/// HDR10 static metadata the 13.1 encoder writes itself: the ST 2086 mastering display colour volume
/// and the content light level, read from the SEI payloads (H.265 D.2.28 / D.2.35: big-endian fields
/// in the order the 13.1 structures declare them). Returns them and the SEI messages left over.
fn native_hdr(sei: &[Sei]) -> (Option<MASTERING_DISPLAY_INFO>, Option<CONTENT_LIGHT_LEVEL>, Vec<Sei>) {
    let (mut md, mut cll, mut rest) = (None, None, Vec::new());
    let u16at = |b: &[u8], i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
    let u32at = |b: &[u8], i: usize| u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    for m in sei {
        let b = m.payload.as_slice();
        if m.payload_type == 137 && b.len() == 24 && md.is_none() {
            let pt = |i| CHROMA_POINTS { x: u16at(b, i), y: u16at(b, i + 2) };
            md = Some(MASTERING_DISPLAY_INFO { g: pt(0), b: pt(4), r: pt(8), whitePoint: pt(12), maxLuma: u32at(b, 16), minLuma: u32at(b, 20) });
        } else if m.payload_type == 144 && b.len() == 4 && cll.is_none() {
            cll = Some(CONTENT_LIGHT_LEVEL { maxContentLightLevel: u16at(b, 0), maxPicAverageLightLevel: u16at(b, 2) });
        } else {
            rest.push(m.clone());
        }
    }
    (md, cll, rest)
}

/// The codec a session encodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    H264,
    Hevc,
}

impl Codec {
    fn guid(self) -> Guid {
        match self {
            Codec::H264 => NV_ENC_CODEC_H264_GUID,
            Codec::Hevc => NV_ENC_CODEC_HEVC_GUID,
        }
    }
}

/// One SEI message the encoder writes in front of every IDR picture (HEVC): its payload type and bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sei {
    pub payload_type: u32,
    pub payload: Vec<u8>,
}

/// The colour description of a stream (ITU-T H.273 code points, limited range) and the SEI messages
/// that go with it. The default is BT.709 without SEI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signal {
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
    pub sei: Vec<Sei>,
}

impl Default for Signal {
    fn default() -> Self {
        Signal { primaries: 1, transfer: 1, matrix: 1, sei: Vec::new() }
    }
}

/// What the encoder says it can do for a codec.
#[derive(Clone, Copy, Debug)]
pub struct Caps {
    /// 10-bit encoding (Main 10) is supported.
    pub ten_bit: bool,
    pub max_bframes: u32,
    pub min_size: (u32, u32),
    pub max_size: (u32, u32),
}

/// Encoder settings (already validated by the caller).
#[derive(Clone, Debug)]
pub struct Params {
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    pub fps: (u32, u32),
    pub bitrate: u32,
    pub max_bitrate: u32,
    pub cbr: bool,
    pub gop: u32,
    /// H.264 only: 0 baseline, 1 main, 2 high (HEVC is Main, or Main 10 with `ten_bit`).
    pub profile: u8,
    /// HEVC Main 10: 10-bit 4:2:0 pictures (P010), otherwise 8-bit (NV12).
    pub ten_bit: bool,
    /// 8-bit only: pictures go in as packed RGBA (ABGR) and NVENC converts them to 4:2:0 on the GPU.
    pub rgba_input: bool,
    /// HEVC: the VUI colour description and the SEI messages of every IDR picture (H.264 keeps BT.709).
    pub signal: Signal,
    /// The codec's own level code: `level × 10` for H.264, `level × 30` for HEVC.
    pub level: Option<u8>,
    pub sar: Option<(u32, u32)>,
    /// B-frames between references (0 or 1).
    pub bframes: u32,
}

/// An input buffer locked for writing: NV12 (or P010) rows go to `data` at `pitch` bytes per row, the
/// chroma plane after `pitch * height` bytes; packed RGBA (ABGR) has `height` rows and no chroma plane.
pub struct Locked<'a> {
    pub data: &'a mut [u8],
    pub pitch: usize,
}

/// A finished picture's bitstream.
pub struct Output {
    /// Annex B bytes.
    pub data: Vec<u8>,
    pub pts: u64,
    /// `NV_ENC_PIC_TYPE_*`.
    pub pic_type: u32,
}

/// The result of submitting a picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Submitted {
    /// The pictures submitted so far have their bitstreams ready.
    Ready,
    /// The encoder wants more input (B-frame reordering) before it has output.
    NeedMoreInput,
}

struct Slot {
    input: *mut c_void,
    output: *mut c_void,
}

/// An initialised encoder with its buffers.
pub struct Session {
    api: &'static Api,
    enc: Handle,
    // Keeps the retained CUDA context alive through session destruction.
    _device: Device,
    slots: Vec<Slot>,
    size: (u32, u32),
    out_size: u32,
    /// `NV_ENC_BUFFER_FORMAT_*` of the input buffers and pictures, and its bytes per sample.
    fmt: u32,
    bytes_per_sample: usize,
    /// The SEI messages of every IDR picture. The payload bytes and the array of descriptors live on
    /// the heap, owned by the session for its whole life, so the pointers handed to the driver stay
    /// valid however the `Session` value is moved and however long the driver keeps a picture queued.
    sei_payloads: Vec<Box<[u8]>>,
    sei_array: Box<[NV_ENC_SEI_PAYLOAD]>,
    /// API 13.1: the HDR10 metadata the encoder writes itself (boxed for the same pointer-stability reason).
    hdr_md: Option<Box<MASTERING_DISPLAY_INFO>>,
    hdr_cll: Option<Box<CONTENT_LIGHT_LEVEL>>,
    /// Pictures submitted, and the distance between IDR pictures (to know which pictures get the SEI).
    submitted: u64,
    gop: u64,
}

// SAFETY: an encoder session and its buffers are driver objects used from one thread at a time (the
// session is `&mut`-driven), and NVENC takes an explicit CUDA context.
unsafe impl Send for Session {}

/// A readable form of a driver status.
fn status(what: &str, s: NvStatus, enc: Handle, api: &Api) -> String {
    let detail = (!enc.is_null())
        .then(|| {
            // SAFETY: a valid session handle; the driver returns a NUL-terminated string it owns.
            api.list.nvEncGetLastErrorString.and_then(|f| unsafe {
                let p = f(enc);
                (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned())
            })
        })
        .flatten()
        // the driver's text is sometimes not text at all (HEVC parameter errors): keep it only when readable
        .filter(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_graphic() || c == ' '));
    match detail {
        Some(d) => format!("{what} failed ({s}): {d}"),
        None => format!("{what} failed ({s})"),
    }
}

impl Session {
    /// Open a session on the system's NVIDIA GPU, or say why not.
    pub fn open() -> Result<Session, String> {
        let api = api()?;
        let device = Device::new()?;
        // SAFETY: a zeroed parameter block with its version and the matching device pointer; the
        // device is kept in the session for as long as the encoder lives. On failure a non-null
        // handle is the driver's and is destroyed exactly once, here, before returning.
        let enc = unsafe {
            let mut p: NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS = std::mem::zeroed();
            p.version = api.ver.open_session;
            p.deviceType = Device::KIND;
            p.device = device.as_raw();
            p.apiVersion = api.level.api_version();
            let open = api.list.nvEncOpenEncodeSessionEx.ok_or("the driver has no nvEncOpenEncodeSessionEx")?;
            let mut enc: Handle = std::ptr::null_mut();
            let s = open(&mut p, &mut enc);
            if s != NV_ENC_SUCCESS || enc.is_null() {
                let e = status("opening the NVENC session", s, enc, api);
                // a failed open may still hand back a session; destroy it so the GPU's encoder
                // session limit is not used up by sessions nobody owns
                if !enc.is_null()
                    && let Some(d) = api.list.nvEncDestroyEncoder
                {
                    d(enc);
                }
                return Err(e);
            }
            enc
        };
        Ok(Session {
            api,
            enc,
            _device: device,
            slots: Vec::new(),
            size: (0, 0),
            out_size: 0,
            fmt: NV_ENC_BUFFER_FORMAT_NV12,
            bytes_per_sample: 1,
            sei_payloads: Vec::new(),
            sei_array: Box::new([]),
            hdr_md: None,
            hdr_cll: None,
            submitted: 0,
            gop: 1,
        })
    }

    /// The encoder's limits for `codec` (an error when the GPU cannot encode it).
    pub fn caps(&self, codec: Codec) -> Result<Caps, String> {
        let get = |cap: u32| -> Result<u32, String> {
            let f = self.api.list.nvEncGetEncodeCaps.ok_or("no nvEncGetEncodeCaps")?;
            // SAFETY: a valid session, a zeroed parameter block with its version, one int out.
            unsafe {
                let mut p: NV_ENC_CAPS_PARAM = std::mem::zeroed();
                p.version = self.api.ver.caps_param;
                p.capsToQuery = cap;
                let mut v = 0i32;
                let s = f(self.enc, codec.guid(), &mut p, &mut v);
                if s != NV_ENC_SUCCESS {
                    return Err(status(if codec == Codec::Hevc { "querying the HEVC encoder" } else { "querying the encoder" }, s, self.enc, self.api));
                }
                Ok(v.max(0) as u32)
            }
        };
        Ok(Caps {
            // a failing query means no 10-bit encoder
            ten_bit: get(NV_ENC_CAPS_SUPPORT_10BIT_ENCODE).unwrap_or(0) != 0,
            max_bframes: get(NV_ENC_CAPS_NUM_MAX_BFRAMES)?,
            min_size: (get(NV_ENC_CAPS_WIDTH_MIN)?, get(NV_ENC_CAPS_HEIGHT_MIN)?),
            max_size: (get(NV_ENC_CAPS_WIDTH_MAX)?, get(NV_ENC_CAPS_HEIGHT_MAX)?),
        })
    }

    /// Configure the encoder (preset P5, high quality tuning, the settings' rate control) and
    /// create `ring` pairs of input / output buffers.
    pub fn initialize(&mut self, p: &Params, ring: usize) -> Result<(), String> {
        if p.ten_bit && p.codec != Codec::Hevc {
            return Err("10-bit encoding is HEVC Main 10 here".into());
        }
        if !p.signal.sei.is_empty() && p.codec != Codec::Hevc {
            return Err("SEI messages are written for HEVC only".into());
        }
        if p.ten_bit && p.rgba_input {
            return Err("RGBA input is 8-bit only".into());
        }
        (self.fmt, self.bytes_per_sample) = match (p.ten_bit, p.rgba_input) {
            (true, _) => (NV_ENC_BUFFER_FORMAT_YUV420_10BIT, 2),
            (false, true) => (NV_ENC_BUFFER_FORMAT_ABGR, 4),
            (false, false) => (NV_ENC_BUFFER_FORMAT_NV12, 1),
        };
        self.gop = u64::from(p.gop.max(1));
        self.submitted = 0;
        // the SEI payloads: heap blocks that outlive every picture (see the field documentation)
        // at API 13.1 the encoder writes the HDR10 mastering display and content light level SEI itself
        let (md, cll, sei) = if self.api.level >= ApiLevel::V13_1 { native_hdr(&p.signal.sei) } else { (None, None, p.signal.sei.clone()) };
        self.hdr_md = md.map(Box::new);
        self.hdr_cll = cll.map(Box::new);
        self.sei_payloads = sei.iter().map(|m| m.payload.clone().into_boxed_slice()).collect();
        let mut array = Vec::with_capacity(self.sei_payloads.len());
        for (m, bytes) in sei.iter().zip(self.sei_payloads.iter_mut()) {
            let size = u32::try_from(bytes.len()).map_err(|_| "an SEI payload longer than 4 GiB".to_string())?;
            array.push(NV_ENC_SEI_PAYLOAD { payloadSize: size, payloadType: m.payload_type, payload: bytes.as_mut_ptr() });
        }
        self.sei_array = array.into_boxed_slice();
        let preset_fn = self.api.list.nvEncGetEncodePresetConfigEx.ok_or("no nvEncGetEncodePresetConfigEx")?;
        let init_fn = self.api.list.nvEncInitializeEncoder.ok_or("no nvEncInitializeEncoder")?;
        // SAFETY: zeroed parameter blocks with their versions; the preset call fills `preset`; the
        // configuration lives in a box that outlives `nvEncInitializeEncoder`, which copies it.
        unsafe {
            let mut preset: Box<NV_ENC_PRESET_CONFIG> = Box::new(std::mem::zeroed());
            preset.version = self.api.ver.preset_config;
            preset.presetCfg.version = self.api.ver.config;
            let s = preset_fn(self.enc, p.codec.guid(), NV_ENC_PRESET_P5_GUID, NV_ENC_TUNING_INFO_HIGH_QUALITY, &mut *preset);
            if s != NV_ENC_SUCCESS {
                return Err(status("reading the encoder preset", s, self.enc, self.api));
            }
            let mut cfg: Box<NV_ENC_CONFIG> = Box::new(preset.presetCfg);
            cfg.version = self.api.ver.config;
            cfg.profileGUID = match (p.codec, p.profile) {
                (Codec::Hevc, _) if p.ten_bit => NV_ENC_HEVC_PROFILE_MAIN10_GUID,
                (Codec::Hevc, _) => NV_ENC_HEVC_PROFILE_MAIN_GUID,
                (Codec::H264, 0) => NV_ENC_H264_PROFILE_BASELINE_GUID,
                (Codec::H264, 1) => NV_ENC_H264_PROFILE_MAIN_GUID,
                (Codec::H264, _) => NV_ENC_H264_PROFILE_HIGH_GUID,
            };
            cfg.gopLength = p.gop.max(1);
            cfg.frameIntervalP = 1 + p.bframes as i32;
            let rc = &mut cfg.rcParams;
            rc.version = self.api.ver.rc_params;
            rc.rateControlMode = if p.cbr { NV_ENC_PARAMS_RC_CBR } else { NV_ENC_PARAMS_RC_VBR };
            rc.averageBitRate = p.bitrate.saturating_mul(1000);
            rc.maxBitRate = if p.cbr { rc.averageBitRate } else { p.max_bitrate.max(p.bitrate).saturating_mul(1000) };
            // a one-second buffer, the usual for streaming-style rate control
            rc.vbvBufferSize = rc.maxBitRate;
            rc.vbvInitialDelay = rc.vbvBufferSize;
            match p.codec {
                Codec::H264 => {
                    let h = &mut cfg.encodeCodecConfig.h264Config;
                    h.idrPeriod = p.gop.max(1);
                    h.level = p.level.map_or(NV_ENC_LEVEL_AUTOSELECT, u32::from);
                    h.flags &= !(H264_OUTPUT_AUD | H264_REPEAT_SPSPPS);
                    h.entropyCodingMode = if p.profile == 0 { 2 } else { NV_ENC_H264_ENTROPY_CODING_MODE_CABAC };
                    h.maxNumRefFrames = 0;
                    // BT.709 limited range, like our own encoder's default signalling
                    let vui = &mut h.h264VUIParameters;
                    vui.videoSignalTypePresentFlag = 1;
                    vui.videoFormat = NV_ENC_VUI_VIDEO_FORMAT_UNSPECIFIED;
                    vui.videoFullRangeFlag = 0;
                    vui.colourDescriptionPresentFlag = 1;
                    vui.colourPrimaries = NV_ENC_VUI_COLOR_PRIMARIES_BT709;
                    vui.transferCharacteristics = NV_ENC_VUI_TRANSFER_CHARACTERISTIC_BT709;
                    vui.colourMatrix = NV_ENC_VUI_MATRIX_COEFFS_BT709;
                    vui.timingInfoPresentFlag = 1;
                    vui.numUnitInTicks = p.fps.1;
                    vui.timeScale = p.fps.0.saturating_mul(2);
                }
                Codec::Hevc => {
                    let h = &mut cfg.encodeCodecConfig.hevcConfig;
                    h.level = p.level.map_or(NV_ENC_LEVEL_AUTOSELECT, u32::from);
                    h.tier = NV_ENC_TIER_HEVC_MAIN;
                    h.minCUSize = NV_ENC_HEVC_CUSIZE_AUTOSELECT;
                    h.maxCUSize = NV_ENC_HEVC_CUSIZE_AUTOSELECT;
                    h.idrPeriod = p.gop.max(1);
                    // parameter sets only in the first IDR (and the hvcC), no access unit delimiters, 4:2:0 at 8 or 10 bits
                    h.flags &= !(HEVC_OUTPUT_AUD
                        | HEVC_DISABLE_SPSPPS
                        | HEVC_REPEAT_SPSPPS
                        | HEVC_ENABLE_INTRA_REFRESH
                        | HEVC_CHROMA_FORMAT_MASK
                        | HEVC_PIXEL_BIT_DEPTH_MASK);
                    h.flags |= 1 << HEVC_CHROMA_FORMAT_SHIFT;
                    if self.api.level >= ApiLevel::V13_1 {
                        // 13.1 replaced the `pixelBitDepthMinus8` flag bits by these fields
                        let depth = if p.ten_bit { NV_ENC_BIT_DEPTH_10 } else { NV_ENC_BIT_DEPTH_8 };
                        h.outputBitDepth = depth;
                        h.inputBitDepth = depth;
                        if self.hdr_md.is_some() {
                            h.flags |= HEVC_OUTPUT_MASTERING_DISPLAY;
                        }
                        if self.hdr_cll.is_some() {
                            h.flags |= HEVC_OUTPUT_MAX_CLL;
                        }
                    } else if p.ten_bit {
                        h.flags |= 2 << HEVC_PIXEL_BIT_DEPTH_SHIFT;
                    }
                    h.maxNumRefFramesInDPB = 0;
                    // the stream's colour description (BT.709 unless the signal says otherwise), limited
                    // range; an HEVC VUI counts frames, not fields: frame rate = time_scale / num_units_in_tick
                    let vui = &mut h.hevcVUIParameters;
                    vui.videoSignalTypePresentFlag = 1;
                    vui.videoFormat = NV_ENC_VUI_VIDEO_FORMAT_UNSPECIFIED;
                    vui.videoFullRangeFlag = 0;
                    vui.colourDescriptionPresentFlag = 1;
                    vui.colourPrimaries = u32::from(p.signal.primaries);
                    vui.transferCharacteristics = u32::from(p.signal.transfer);
                    vui.colourMatrix = u32::from(p.signal.matrix);
                    // Let NVENC derive timing from frameRateNum/frameRateDen. Some drivers
                    // reverse the explicitly supplied HEVC VUI timing pair.
                    vui.timingInfoPresentFlag = 0;
                    vui.numUnitInTicks = 0;
                    vui.timeScale = 0;
                }
            }

            let mut init: Box<NV_ENC_INITIALIZE_PARAMS> = Box::new(std::mem::zeroed());
            init.version = self.api.ver.initialize_params;
            init.encodeGUID = p.codec.guid();
            init.presetGUID = NV_ENC_PRESET_P5_GUID;
            init.encodeWidth = p.width;
            init.encodeHeight = p.height;
            let (sn, sd) = p.sar.unwrap_or((1, 1));
            init.darWidth = p.width.saturating_mul(sn.max(1));
            init.darHeight = p.height.saturating_mul(sd.max(1));
            init.frameRateNum = p.fps.0;
            init.frameRateDen = p.fps.1;
            init.enablePTD = 1;
            init.encodeConfig = &mut *cfg;
            init.tuningInfo = NV_ENC_TUNING_INFO_HIGH_QUALITY;
            init.bufferFormat = self.fmt;
            let s = init_fn(self.enc, &mut *init);
            if s != NV_ENC_SUCCESS {
                return Err(status("initialising the encoder", s, self.enc, self.api));
            }
        }
        self.size = (p.width, p.height);
        self.out_size = (p.width.saturating_mul(p.height).saturating_mul(2)).max(1 << 20);
        for _ in 0..ring.max(1) {
            self.add_slot()?;
        }
        Ok(())
    }

    fn add_slot(&mut self) -> Result<(), String> {
        let (create_in, create_out) = (
            self.api.list.nvEncCreateInputBuffer.ok_or("no nvEncCreateInputBuffer")?,
            self.api.list.nvEncCreateBitstreamBuffer.ok_or("no nvEncCreateBitstreamBuffer")?,
        );
        // SAFETY: zeroed parameter blocks with their versions; the driver fills in the buffer
        // handles, which `Drop` destroys.
        unsafe {
            let mut i: NV_ENC_CREATE_INPUT_BUFFER = std::mem::zeroed();
            i.version = self.api.ver.create_input_buffer;
            i.width = self.size.0;
            i.height = self.size.1;
            i.memoryHeap = NV_ENC_MEMORY_HEAP_SYSMEM_CACHED;
            i.bufferFmt = self.fmt;
            let s = create_in(self.enc, &mut i);
            if s != NV_ENC_SUCCESS {
                return Err(status("creating an input buffer", s, self.enc, self.api));
            }
            let mut o: NV_ENC_CREATE_BITSTREAM_BUFFER = std::mem::zeroed();
            o.version = self.api.ver.create_bitstream_buffer;
            o.size = self.out_size;
            o.memoryHeap = NV_ENC_MEMORY_HEAP_SYSMEM_CACHED;
            let s = create_out(self.enc, &mut o);
            if s != NV_ENC_SUCCESS {
                if let Some(d) = self.api.list.nvEncDestroyInputBuffer {
                    d(self.enc, i.inputBuffer);
                }
                return Err(status("creating an output buffer", s, self.enc, self.api));
            }
            self.slots.push(Slot { input: i.inputBuffer, output: o.bitstreamBuffer });
        }
        Ok(())
    }

    /// Number of buffer slots.
    pub fn slots(&self) -> usize {
        self.slots.len()
    }

    /// The parameter sets as one Annex B byte string (SPS and PPS; VPS, SPS and PPS for HEVC).
    pub fn sequence_params(&self) -> Result<Vec<u8>, String> {
        let f = self.api.list.nvEncGetSequenceParams.ok_or("no nvEncGetSequenceParams")?;
        let mut buf = vec![0u8; 1024];
        let mut len = 0u32;
        // SAFETY: a zeroed parameter block with its version; `buf` and `len` outlive the call and
        // the driver writes at most `inBufferSize` bytes into `buf`.
        unsafe {
            let mut p: NV_ENC_SEQUENCE_PARAM_PAYLOAD = std::mem::zeroed();
            p.version = self.api.ver.sequence_param_payload;
            p.inBufferSize = buf.len() as u32;
            p.spsppsBuffer = buf.as_mut_ptr().cast();
            p.outSPSPPSPayloadSize = &mut len;
            let s = f(self.enc, &mut p);
            if s != NV_ENC_SUCCESS {
                return Err(status("reading the parameter sets", s, self.enc, self.api));
            }
        }
        buf.truncate(len as usize);
        Ok(buf)
    }

    /// Write a picture into slot `slot`'s input buffer through `fill` (which gets the locked
    /// buffer), then submit it with timestamp `pts`; `output_slot` receives the bitstream.
    pub fn submit(&mut self, slot: usize, pts: u64, fill: impl FnOnce(Locked<'_>)) -> Result<Submitted, String> {
        let s = self.slots.get(slot).ok_or("no such buffer slot")?;
        let (input, output) = (s.input, s.output);
        let (lock, unlock, encode) = (
            self.api.list.nvEncLockInputBuffer.ok_or("no nvEncLockInputBuffer")?,
            self.api.list.nvEncUnlockInputBuffer.ok_or("no nvEncUnlockInputBuffer")?,
            self.api.list.nvEncEncodePicture.ok_or("no nvEncEncodePicture")?,
        );
        let (w, h) = self.size;
        // SAFETY: the buffer handle is one this session created. `LockInputBuffer` returns a
        // writable block of `pitch * h * 3 / 2` bytes (NV12 or P010: luma rows then chroma rows, the pitch
        // in bytes), or `pitch * h` bytes (ABGR: packed rows only), that stays valid until
        // `UnlockInputBuffer`; the slice borrows it only inside `fill`.
        let pitch = unsafe {
            let mut l: NV_ENC_LOCK_INPUT_BUFFER = std::mem::zeroed();
            l.version = self.api.ver.lock_input_buffer;
            l.inputBuffer = input;
            let st = lock(self.enc, &mut l);
            if st != NV_ENC_SUCCESS || l.bufferDataPtr.is_null() || l.pitch == 0 {
                return Err(status("locking an input buffer", st, self.enc, self.api));
            }
            // a driver pitch narrower than a row (in bytes: P010 samples are two) would make `fill` write past each row
            if (l.pitch as usize) < (w as usize).saturating_mul(self.bytes_per_sample) {
                // the lock succeeded, so release it before giving up (its status no longer matters)
                let _ = unlock(self.enc, input);
                return Err(format!("the driver returned an input pitch of {} bytes for {w}-pixel rows of {}-byte samples", l.pitch, self.bytes_per_sample));
            }
            let rows = (l.pitch as usize).saturating_mul(h as usize);
            let len = if self.fmt == NV_ENC_BUFFER_FORMAT_ABGR { rows } else { rows.saturating_mul(3) / 2 };
            fill(Locked { data: std::slice::from_raw_parts_mut(l.bufferDataPtr.cast::<u8>(), len), pitch: l.pitch as usize });
            let st = unlock(self.enc, input);
            if st != NV_ENC_SUCCESS {
                return Err(status("unlocking an input buffer", st, self.enc, self.api));
            }
            l.pitch
        };
        // IDR pictures (every `gop` pictures from the first, in submission = display order) carry the SEI messages
        let has_sei = !self.sei_array.is_empty() || self.hdr_md.is_some() || self.hdr_cll.is_some();
        let idr = has_sei && self.submitted.is_multiple_of(self.gop);
        self.submitted = self.submitted.saturating_add(1);
        // SAFETY: a zeroed parameter block with its version; the buffers are this session's. The SEI
        // descriptor array and the payload bytes it points to are heap blocks owned by `self`, which
        // stay allocated (and unmoved) until the session is dropped, so they outlive this call and
        // whatever time the driver keeps the picture queued.
        let st = unsafe {
            let mut e: NV_ENC_PIC_PARAMS = std::mem::zeroed();
            if idr {
                let h = &mut e.codecPicParams.hevcPicParams;
                h.seiPayloadArrayCnt = u32::try_from(self.sei_array.len()).unwrap_or(0);
                h.seiPayloadArray = self.sei_array.as_mut_ptr();
                if let Some(md) = self.hdr_md.as_mut() {
                    h.pMasteringDisplay = std::ptr::from_mut(&mut **md);
                }
                if let Some(cll) = self.hdr_cll.as_mut() {
                    h.pMaxCll = std::ptr::from_mut(&mut **cll);
                }
            }
            e.version = self.api.ver.pic_params;
            e.inputWidth = w;
            e.inputHeight = h;
            e.inputPitch = pitch;
            e.inputBuffer = input;
            e.outputBitstream = output;
            e.bufferFmt = self.fmt;
            e.pictureStruct = NV_ENC_PIC_STRUCT_FRAME;
            e.inputTimeStamp = pts;
            e.inputDuration = 1;
            encode(self.enc, &mut e)
        };
        match st {
            NV_ENC_SUCCESS => Ok(Submitted::Ready),
            NV_ENC_ERR_NEED_MORE_INPUT => Ok(Submitted::NeedMoreInput),
            s => Err(status("encoding a picture", s, self.enc, self.api)),
        }
    }

    /// Tell the encoder the stream ends: every picture it still holds becomes ready.
    pub fn end_of_stream(&mut self) -> Result<(), String> {
        let encode = self.api.list.nvEncEncodePicture.ok_or("no nvEncEncodePicture")?;
        // SAFETY: a zeroed parameter block with its version and the EOS flag (no input buffer).
        let st = unsafe {
            let mut e: NV_ENC_PIC_PARAMS = std::mem::zeroed();
            e.version = self.api.ver.pic_params;
            e.encodePicFlags = NV_ENC_PIC_FLAG_EOS;
            encode(self.enc, &mut e)
        };
        if st != NV_ENC_SUCCESS {
            return Err(status("ending the stream", st, self.enc, self.api));
        }
        Ok(())
    }

    /// Read the finished bitstream of slot `slot` (blocks until the encoder has it).
    pub fn read(&mut self, slot: usize) -> Result<Output, String> {
        let output = self.slots.get(slot).ok_or("no such buffer slot")?.output;
        let (lock, unlock) =
            (self.api.list.nvEncLockBitstream.ok_or("no nvEncLockBitstream")?, self.api.list.nvEncUnlockBitstream.ok_or("no nvEncUnlockBitstream")?);
        // SAFETY: a zeroed parameter block with its version and a buffer of this session; after a
        // successful lock `bitstreamBufferPtr` addresses `bitstreamSizeInBytes` readable bytes until
        // `UnlockBitstream`, and they are copied before it.
        unsafe {
            let mut l: NV_ENC_LOCK_BITSTREAM = std::mem::zeroed();
            l.version = self.api.ver.lock_bitstream;
            l.outputBitstream = output;
            let st = lock(self.enc, &mut l);
            if st != NV_ENC_SUCCESS {
                return Err(status("reading a bitstream", st, self.enc, self.api));
            }
            let n = l.bitstreamSizeInBytes as usize;
            let data =
                if l.bitstreamBufferPtr.is_null() || n == 0 { Vec::new() } else { std::slice::from_raw_parts(l.bitstreamBufferPtr.cast::<u8>(), n).to_vec() };
            let out = Output { data, pts: l.outputTimeStamp, pic_type: l.pictureType };
            let st = unlock(self.enc, output);
            if st != NV_ENC_SUCCESS {
                return Err(status("releasing a bitstream", st, self.enc, self.api));
            }
            Ok(out)
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: every handle was created by this session and is destroyed once, buffers first.
        unsafe {
            for s in self.slots.drain(..) {
                if let Some(d) = self.api.list.nvEncDestroyInputBuffer {
                    d(self.enc, s.input);
                }
                if let Some(d) = self.api.list.nvEncDestroyBitstreamBuffer {
                    d(self.enc, s.output);
                }
            }
            if let Some(d) = self.api.list.nvEncDestroyEncoder {
                d(self.enc);
            }
        }
    }
}

#[cfg(test)]
mod level_tests {
    use super::*;

    #[test]
    fn level_follows_the_driver_and_the_override() {
        assert_eq!(choose_level(0xd1, None), Ok(ApiLevel::V13_1));
        assert_eq!(choose_level(0xd1, Some("auto")), Ok(ApiLevel::V13_1));
        assert_eq!(choose_level(0xd1, Some("12.1")), Ok(ApiLevel::V12_1));
        assert_eq!(choose_level(0xc1, None), Ok(ApiLevel::V12_1));
        assert!(choose_level(0xc1, Some("13.1")).is_err());
        assert!(choose_level(0xc1, Some("14.0")).is_err());
        assert!(choose_level(0xb1, None).unwrap_err().contains("older than 12.1"));
    }

    #[test]
    fn hdr_sei_payloads_become_the_native_structures() {
        let mut md = Vec::new();
        for v in [34000u16, 16000, 13250, 34500, 7500, 3000, 15635, 16450] {
            md.extend(v.to_be_bytes());
        }
        md.extend(10_000_000u32.to_be_bytes());
        md.extend(50u32.to_be_bytes());
        let other = Sei { payload_type: 5, payload: vec![1, 2, 3] };
        let sei = [Sei { payload_type: 137, payload: md }, Sei { payload_type: 144, payload: vec![0x03, 0xe8, 0x00, 0xc8] }, other.clone()];
        let (m, c, rest) = native_hdr(&sei);
        let m = m.unwrap();
        assert_eq!((m.g.x, m.g.y, m.b.x, m.b.y, m.r.x, m.r.y), (34000, 16000, 13250, 34500, 7500, 3000));
        assert_eq!((m.whitePoint.x, m.whitePoint.y, m.maxLuma, m.minLuma), (15635, 16450, 10_000_000, 50));
        let c = c.unwrap();
        assert_eq!((c.maxContentLightLevel, c.maxPicAverageLightLevel), (1000, 200));
        assert_eq!(rest, vec![other]);
        // a malformed payload is left to the SEI array instead of being guessed at
        let (m, c, rest) = native_hdr(&[Sei { payload_type: 137, payload: vec![0; 5] }, Sei { payload_type: 144, payload: vec![0; 3] }]);
        assert!(m.is_none() && c.is_none() && rest.len() == 2);
    }
}
