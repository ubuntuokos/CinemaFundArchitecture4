//! DCP export (#115): a SMPTE Digital Cinema Package (ST 429-2) in the output folder.
//!
//! - Picture: JPEG 2000 (our encoder, DCI 2K or 4K profile) of 12-bit X'Y'Z' (ST 428-1), the
//!   sequence fitted into a DCI container (Flat, Scope or Full) with black bars, in a
//!   frame-wrapped MXF track file (ST 429-4).
//! - Sound: 5.1 (L, R, C, LFE, Ls, Rs), 24-bit 48 kHz PCM in a frame-wrapped MXF track file
//!   (ST 429-3), channel configuration 1.
//! - One reel; composition playlist (ST 429-7), packing list (ST 429-8), asset map (ST 429-9)
//!   and volume index, with SHA-1 hashes and random (version 4) UUIDs.
//!
//! With immersive audio (#343, [`crate::dcp_iab`]) the reel also holds an IAB track file
//! (ST 429-18, a CPL `AuxData` asset) and the main sound becomes 16 channels with the FSK
//! synchronisation signal (ST 429-19).
//!
//! Unencrypted and unsigned (no KDM, no XML signature): a DCP that servers ingest as an OV.
//! The composition edit rate is the DCI rate nearest the export's (24, 25, 30, 48, 50 or 60; 4K
//! 24, 25 or 30).

use std::io::{Read, Write};

use cfa4_mxf::{FrameInfo, MxfWriter, PackageIds, Pattern, PictureDesc, Rational, SoundDesc, StartTimecode, Timestamp, Umid, WriterConfig};
use cfa4_time::FrameRate;
use rayon::prelude::*;

use crate::dcp_iab::{DcpImmersive, ImmersiveMux};
use crate::{AudioCodec, EncodedPacket, EncoderFrame, ExportError, ExportSettings, Format, Out, Result, SampleFormat, Scaling, Stage, VideoEncoder, timed};

/// Image resolution of a DCP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DcpResolution {
    TwoK,
    FourK,
}

impl DcpResolution {
    pub const ALL: [DcpResolution; 2] = [DcpResolution::TwoK, DcpResolution::FourK];

    /// `2k` (also empty) or `4k`.
    pub fn from_name(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "2k" => Some(DcpResolution::TwoK),
            "4k" => Some(DcpResolution::FourK),
            _ => None,
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            DcpResolution::TwoK => "2k",
            DcpResolution::FourK => "4k",
        }
    }
    /// The resolution an export asks for (2K unless set).
    pub fn of(s: &ExportSettings) -> Self {
        DcpResolution::from_name(&s.dcp_resolution).unwrap_or(DcpResolution::TwoK)
    }
    /// Frame rates (frames per second) of this resolution (ST 429-2 table 1).
    pub fn rates(self) -> &'static [i64] {
        match self {
            DcpResolution::TwoK => &[24, 25, 30, 48, 50, 60],
            DcpResolution::FourK => &[24, 25, 30],
        }
    }
}

/// The DCI container the picture is fitted into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DcpAspect {
    /// 1.85:1.
    Flat,
    /// 2.39:1.
    Scope,
    /// 1.90:1, the whole DCI frame.
    Full,
}

impl DcpAspect {
    pub const ALL: [DcpAspect; 3] = [DcpAspect::Flat, DcpAspect::Scope, DcpAspect::Full];

    /// `flat` (also empty, `185`), `scope` (`239`, `cinemascope`) or `full` (`190`).
    pub fn from_name(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().replace([' ', '.', ':', '-'], "").as_str() {
            "" | "flat" | "185" | "1851" => Some(DcpAspect::Flat),
            "scope" | "239" | "2391" | "cinemascope" => Some(DcpAspect::Scope),
            "full" | "190" | "1901" | "fullcontainer" => Some(DcpAspect::Full),
            _ => None,
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            DcpAspect::Flat => "flat",
            DcpAspect::Scope => "scope",
            DcpAspect::Full => "full",
        }
    }
    /// The container an export asks for (Flat unless set).
    pub fn of(s: &ExportSettings) -> Self {
        DcpAspect::from_name(&s.dcp_aspect).unwrap_or(DcpAspect::Flat)
    }
    /// Pixel array (ST 429-2 table 1).
    pub fn size(self, res: DcpResolution) -> (u32, u32) {
        match (self, res) {
            (DcpAspect::Flat, DcpResolution::TwoK) => (1998, 1080),
            (DcpAspect::Scope, DcpResolution::TwoK) => (2048, 858),
            (DcpAspect::Full, DcpResolution::TwoK) => (2048, 1080),
            (DcpAspect::Flat, DcpResolution::FourK) => (3996, 2160),
            (DcpAspect::Scope, DcpResolution::FourK) => (4096, 1716),
            (DcpAspect::Full, DcpResolution::FourK) => (4096, 2160),
        }
    }
}

/// Content kinds of a composition playlist (ST 429-7).
pub const DCP_CONTENT_KINDS: [&str; 10] = ["feature", "trailer", "teaser", "short", "advertisement", "test", "transitional", "rating", "policy", "psa"];

/// The content kind an export writes (`feature` unless set), if it is one of [`DCP_CONTENT_KINDS`].
pub fn dcp_content_kind(s: &ExportSettings) -> Option<&'static str> {
    let k = s.dcp_content_kind.trim().to_ascii_lowercase();
    if k.is_empty() {
        return Some("feature");
    }
    DCP_CONTENT_KINDS.into_iter().find(|c| *c == k)
}

/// The DCP frame rate of `res` nearest `want` (the lower one on a tie).
pub fn dcp_rate(res: DcpResolution, want: FrameRate) -> FrameRate {
    let fps = want.sane().as_f64();
    let best = res.rates().iter().copied().min_by(|a, b| (*a as f64 - fps).abs().total_cmp(&(*b as f64 - fps).abs()).then(a.cmp(b))).unwrap_or(24);
    FrameRate::new(best, 1)
}

/// The settings a DCP export runs with: the DCI container size (the picture scaled to fit, black
/// bars), square pixels, the nearest DCI rate, 5.1 24-bit 48 kHz PCM, SDR.
pub(crate) fn normalize(s: &ExportSettings, seq_rate: FrameRate) -> ExportSettings {
    let mut s = s.clone();
    let res = DcpResolution::of(&s);
    s.frame_size = Some(DcpAspect::of(&s).size(res));
    s.scale = 1.0;
    s.scaling = Scaling::ScaleToFit;
    s.pixel_aspect = None;
    s.frame_rate = Some(dcp_rate(res, s.frame_rate.unwrap_or(seq_rate)));
    s.audio.sample_rate = Some(48_000);
    s.audio.channels = 6;
    s.audio.codec = AudioCodec::Pcm;
    s.audio.set_sample_format(SampleFormat::Int24);
    s.sdr = true;
    s.alpha = false;
    s
}

/// Whether `s` already is what [`normalize`] makes of it.
pub(crate) fn is_normalized(s: &ExportSettings) -> bool {
    let res = DcpResolution::of(s);
    s.frame_size == Some(DcpAspect::of(s).size(res))
        && s.frame_rate.is_some_and(|r| r.den == 1 && res.rates().contains(&r.num))
        && s.audio.sample_rate == Some(48_000)
        && s.audio.channels == 6
        && s.audio.sample_format() == SampleFormat::Int24
        && s.pixel_aspect.is_none()
        && s.scaling == Scaling::ScaleToFit
}

/// Errors in the DCP settings (resolution, container, content kind names).
pub(crate) fn validate(s: &ExportSettings) -> Result<()> {
    if DcpResolution::from_name(&s.dcp_resolution).is_none() {
        return Err(ExportError::Unsupported(format!("unknown DCP resolution {:?}: choose 2k or 4k", s.dcp_resolution)));
    }
    if DcpAspect::from_name(&s.dcp_aspect).is_none() {
        return Err(ExportError::Unsupported(format!("unknown DCP container {:?}: choose flat, scope or full", s.dcp_aspect)));
    }
    if dcp_content_kind(s).is_none() {
        return Err(ExportError::Unsupported(format!("unknown DCP content kind {:?}: choose one of {}", s.dcp_content_kind, DCP_CONTENT_KINDS.join(", "))));
    }
    crate::dcp_iab::validate(s)
}

// ---------------------------------------------------------------------------------------------
// X'Y'Z' (ST 428-1)

/// Reference white luminance (cd/m²) over the 52.37 cd/m² of code value 4095.
const DCDM_SCALE: f32 = 48.0 / 52.37;

/// sRGB / BT.709 primaries, D65 white, to CIE XYZ (IEC 61966-2-1).
const RGB_TO_XYZ: [[f32; 3]; 3] = [[0.412_456_4, 0.357_576_1, 0.180_437_5], [0.212_672_9, 0.715_152_2, 0.072_175], [0.019_333_9, 0.119_192, 0.950_304_1]];

/// Linear light of an 8-bit sRGB code value (the sRGB EOTF).
fn srgb_linear(v: u8) -> f32 {
    let c = f32::from(v) / 255.0;
    if c <= 0.040_45 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// The 12-bit code value of a tristimulus value relative to the reference white (Y = 1 at
/// 48 cd/m²): `INT(4095 × (48 X / 52.37)^(1/2.6))`, clamped to 0..=4095.
pub fn dcdm_code(t: f32) -> i32 {
    let v = (t.max(0.0) * DCDM_SCALE).powf(1.0 / 2.6) * 4095.0;
    if v.is_finite() { (v + 0.5).floor().clamp(0.0, 4095.0) as i32 } else { 0 }
}

/// X', Y' and Z' planes (12-bit code values) of straight sRGB RGBA8 pictures `w` x `h` (alpha
/// ignored: the exporter flattens over black). The white of the picture stays D65 (no
/// chromatic adaptation), reference white at 48 cd/m².
pub fn rgba_to_xyz12(rgba: &[u8], w: usize, h: usize) -> [Vec<i32>; 3] {
    let lut: Vec<f32> = (0..=255u8).map(srgb_linear).collect();
    let n = w.saturating_mul(h);
    let mut planes = [vec![0i32; n], vec![0i32; n], vec![0i32; n]];
    let [px, py, pz] = &mut planes;
    let rows = rgba.par_chunks(w.max(1) * 4).zip(px.par_chunks_mut(w.max(1))).zip(py.par_chunks_mut(w.max(1))).zip(pz.par_chunks_mut(w.max(1)));
    rows.for_each(|(((src, x), y), z)| {
        for (k, p) in src.chunks_exact(4).enumerate() {
            let lin = |i: usize| p.get(i).and_then(|v| lut.get(usize::from(*v))).copied().unwrap_or(0.0);
            let (r, g, b) = (lin(0), lin(1), lin(2));
            let t = |m: [f32; 3]| m[0] * r + m[1] * g + m[2] * b;
            if let (Some(x), Some(y), Some(z)) = (x.get_mut(k), y.get_mut(k), z.get_mut(k)) {
                *x = dcdm_code(t(RGB_TO_XYZ[0]));
                *y = dcdm_code(t(RGB_TO_XYZ[1]));
                *z = dcdm_code(t(RGB_TO_XYZ[2]));
            }
        }
    });
    planes
}

/// JPEG 2000 for DCPs: X'Y'Z' in the DCI 2K or 4K profile. Frames are collected and coded
/// [`DcpEncoder::batch`] at a time in parallel (each one is independent; the coder alone keeps a
/// core busy), so packets come out up to a batch late and in order.
struct DcpEncoder {
    opts: cfa4_j2k::EncodeOptions,
    rate: FrameRate,
    /// RGBA8 frames (width, height, pixels) waiting for their batch.
    pending: Vec<(u32, u32, Vec<u8>)>,
    batch: usize,
}

impl DcpEncoder {
    fn code(&self, w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>> {
        let [x, y, z] = timed(Stage::Convert, || rgba_to_xyz12(rgba, w as usize, h as usize));
        let plane = |data: Vec<i32>| cfa4_j2k::Plane { width: w, height: h, precision: 12, signed: false, dx: 1, dy: 1, data };
        let image = cfa4_j2k::Image { width: w, height: h, components: vec![plane(x), plane(y), plane(z)] };
        cfa4_j2k::encode(&image, &self.opts).map_err(|e| ExportError::Encode(format!("JPEG 2000: {e}")))
    }

    /// Code every pending frame, in parallel, in order.
    fn drain(&mut self) -> Result<Vec<EncodedPacket>> {
        let frames = std::mem::take(&mut self.pending);
        let coded: Vec<Result<Vec<u8>>> = frames.par_iter().map(|(w, h, rgba)| self.code(*w, *h, rgba)).collect();
        coded.into_iter().map(|data| Ok(EncodedPacket { data: data?, key: true, duration: 1, composition_offset: 0 })).collect()
    }
}

impl VideoEncoder for DcpEncoder {
    fn sample_entry(&self) -> cfa4_isobmff::SampleEntry {
        // never written: DCP pictures go to MXF track files
        let fourcc = cfa4_isobmff::FourCc(*b"mjp2");
        cfa4_isobmff::SampleEntry::video(fourcc, cfa4_isobmff::CodecConfig::Unknown { fourcc, raw: Vec::new() }, 0, 0)
    }
    fn timescale(&self) -> u32 {
        u32::try_from(self.rate.num).unwrap_or(24)
    }
    fn encode(&mut self, f: &EncoderFrame) -> Result<Vec<EncodedPacket>> {
        self.pending.push((f.width, f.height, f.rgba.to_vec()));
        if self.pending.len() >= self.batch { self.drain() } else { Ok(Vec::new()) }
    }
    fn flush(&mut self) -> Result<Vec<EncodedPacket>> {
        self.drain()
    }
}

pub(crate) fn dcp_factory(format: Format, w: u32, h: u32, rate: FrameRate, s: &ExportSettings) -> Option<Result<Box<dyn VideoEncoder>>> {
    if format != Format::Dcp {
        return None;
    }
    let res = DcpResolution::of(s);
    Some(if (w, h) != DcpAspect::of(s).size(res) || rate.den != 1 || !res.rates().contains(&rate.num) {
        Err(ExportError::Unsupported(format!("a DCP has no {w}x{h} pictures at {} fps", rate.label())))
    } else {
        let fps = u32::try_from(rate.num).unwrap_or(24);
        let opts = match res {
            DcpResolution::TwoK => cfa4_j2k::EncodeOptions::cinema_2k(fps),
            DcpResolution::FourK => cfa4_j2k::EncodeOptions::cinema_4k(fps),
        };
        // a batch per core, fewer for 4K (each 4K frame holds ~110 MB while it is coded)
        let batch = rayon::current_num_threads().clamp(1, if res == DcpResolution::FourK { 4 } else { 8 });
        Ok(Box::new(DcpEncoder { opts, rate, pending: Vec::new(), batch }) as Box<dyn VideoEncoder>)
    })
}

// ---------------------------------------------------------------------------------------------
// SHA-1 (FIPS 180-4), the asset hash of ST 429-8

/// Streaming SHA-1.
pub(crate) struct Sha1 {
    h: [u32; 5],
    block: [u8; 64],
    fill: usize,
    len: u64,
}

impl Sha1 {
    pub fn new() -> Self {
        Sha1 { h: [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0], block: [0; 64], fill: 0, len: 0 }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.len = self.len.wrapping_add(data.len() as u64);
        while !data.is_empty() {
            let take = (64 - self.fill).min(data.len());
            let (head, rest) = data.split_at(take);
            if let Some(dst) = self.block.get_mut(self.fill..self.fill + take) {
                dst.copy_from_slice(head);
            }
            self.fill += take;
            data = rest;
            if self.fill == 64 {
                let block = self.block;
                self.compress(&block);
                self.fill = 0;
            }
        }
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 80];
        for (i, c) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = self.h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            (e, d, c, b, a) = (d, c, b.rotate_left(30), a, t);
        }
        for (h, v) in self.h.iter_mut().zip([a, b, c, d, e]) {
            *h = h.wrapping_add(v);
        }
    }

    pub fn finish(mut self) -> [u8; 20] {
        let bits = self.len.wrapping_mul(8);
        self.update(&[0x80]);
        while self.fill != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_be_bytes());
        let mut out = [0u8; 20];
        for (o, h) in out.chunks_exact_mut(4).zip(self.h) {
            o.copy_from_slice(&h.to_be_bytes());
        }
        out
    }
}

pub(crate) fn sha1(data: &[u8]) -> [u8; 20] {
    let mut s = Sha1::new();
    s.update(data);
    s.finish()
}

// ---------------------------------------------------------------------------------------------
// The package

/// A version 4 (random) UUID made from a hash of `seed` (callers make the seed unique: output
/// path, time, role).
fn uuid(seed: &str) -> [u8; 16] {
    let h = sha1(seed.as_bytes());
    let mut u = [0u8; 16];
    u.copy_from_slice(&h[..16]);
    u[6] = (u[6] & 0x0F) | 0x40;
    u[8] = (u[8] & 0x3F) | 0x80;
    u
}

fn uuid_text(u: &[u8; 16]) -> String {
    let hex: String = u.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

/// Text for XML element content.
fn xml_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

fn base64(data: &[u8]) -> String {
    cfa4_project::gtemplate::base64_encode(data)
}

/// `name` inside the folder `dir`.
fn join(dir: &str, name: &str) -> String {
    std::path::Path::new(dir).join(name).to_string_lossy().into_owned()
}

fn io(e: impl std::fmt::Display) -> ExportError {
    ExportError::Io(e.to_string())
}

/// Finish an output file and hash it: (size, SHA-1).
fn finish_hashed(out: Out, settings: &ExportSettings, path: &str) -> Result<(u64, [u8; 20])> {
    match out {
        Out::Mem(c) => {
            let data = c.into_inner();
            let hash = sha1(&data);
            Ok((crate::write_output(settings, path, data)?, hash))
        }
        Out::File(mut w) => {
            w.flush().map_err(io)?;
            drop(w);
            let mut f = std::fs::File::open(path).map_err(|e| io(format!("{path}: {e}")))?;
            let mut s = Sha1::new();
            let mut buf = vec![0u8; 1 << 20];
            let mut n = 0u64;
            loop {
                let k = f.read(&mut buf).map_err(io)?;
                if k == 0 {
                    break;
                }
                s.update(buf.get(..k).unwrap_or_default());
                n += k as u64;
            }
            Ok((n, s.finish()))
        }
    }
}

/// One file of the package.
struct Asset {
    id: [u8; 16],
    /// File name inside the package folder.
    name: String,
    size: u64,
    hash: [u8; 20],
    mime: &'static str,
    /// Edit units of a track file when they may differ from the picture's (the IAB track).
    units: Option<u64>,
}

/// Everything the DCP muxer needs from the export.
pub(crate) struct DcpSetup<'a> {
    pub settings: &'a ExportSettings,
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub rate: FrameRate,
    /// Whether the export has sound (always 5.1 24-bit 48 kHz).
    pub audio: bool,
}

/// A DCP export in progress: the two track files are written as the export runs; the XML at the
/// end.
pub(crate) struct DcpMux {
    dir: String,
    seed: String,
    title: String,
    kind: &'static str,
    rate: FrameRate,
    size: (u32, u32),
    issued: String,
    picture: Option<([u8; 16], String, MxfWriter<Out>)>,
    sound: Option<([u8; 16], String, MxfWriter<Out>)>,
    /// The IAB track and the 16-channel main sound of an immersive audio DCP.
    immersive: Option<ImmersiveMux>,
    frames: u64,
}

/// ST 429-2 channel configuration 1: 5.1 with HI / VI-N (`L R C LFE Ls Rs`, 6 or 8 channels).
const CHANNEL_CONFIG_1: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x0B, 0x04, 0x02, 0x02, 0x10, 0x03, 0x01, 0x01, 0x00];

impl DcpMux {
    pub fn new(s: DcpSetup) -> Result<Self> {
        let settings = s.settings;
        let dir = settings.path.clone();
        if settings.sink.is_none() {
            std::fs::create_dir_all(&dir).map_err(|e| io(format!("{dir}: {e}")))?;
        }
        let now = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let seed = format!("{dir}|{now}");
        let secs = i64::try_from(now / 1_000_000_000).unwrap_or(0);
        let t = Timestamp::from_unix(secs);
        let issued = format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}+00:00", t.year, t.month, t.day, t.hour, t.minute, t.second);
        let rate = Rational::new(i32::try_from(s.rate.num).unwrap_or(24), i32::try_from(s.rate.den).unwrap_or(1));
        let res = DcpResolution::of(settings);
        let material = Umid::dcinema(uuid(&format!("{seed}|material")));
        let config = |role: &str, id: [u8; 16]| {
            let ids = PackageIds { material, file: Umid::dcinema(id), material_name: s.title.clone(), file_name: format!("{} {role}", s.title) };
            let mut c = WriterConfig::new(Pattern::DCinema, rate, ids);
            c.timecode = Some(StartTimecode { frames: 0, rate, drop_frame: false });
            c.modified = t;
            c
        };
        let pic_id = uuid(&format!("{seed}|picture"));
        let mut pc = config("picture", pic_id);
        let rsiz = if res == DcpResolution::FourK { 4 } else { 3 };
        pc.picture = Some(PictureDesc::dcinema(rsiz, s.width, s.height));
        let pic_name = format!("j2c_{}.mxf", uuid_text(&pic_id));
        let picture = Some((pic_id, pic_name.clone(), MxfWriter::new(Out::create_path(settings, &join(&dir, &pic_name))?, pc).map_err(io)?));
        let immersive = DcpImmersive::of(settings) != DcpImmersive::Off;
        // immersive audio: the main sound is always there, 16 channels, Configuration 4
        let sound = if s.audio || immersive {
            let id = uuid(&format!("{seed}|sound"));
            let mut sc = config("sound", id);
            let channels = if immersive { crate::dcp_iab::MAIN_SOUND_CHANNELS } else { 6 };
            sc.sound = vec![SoundDesc { sample_rate: 48_000, channels, bits: 24 }];
            sc.channel_assignment = Some(if immersive { crate::dcp_iab::CHANNEL_CONFIG_4 } else { CHANNEL_CONFIG_1 });
            let name = format!("pcm_{}.mxf", uuid_text(&id));
            Some((id, name.clone(), MxfWriter::new(Out::create_path(settings, &join(&dir, &name))?, sc).map_err(io)?))
        } else {
            None
        };
        let immersive = match (&sound, immersive) {
            (Some((sound_id, _, _)), true) => {
                let id = uuid(&format!("{seed}|iab"));
                let mut ic = config("immersive audio", id);
                ic.iab = Some(crate::dcp_iab::initial_desc());
                let name = format!("iab_{}.mxf", uuid_text(&id));
                let w = MxfWriter::new(Out::create_path(settings, &join(&dir, &name))?, ic).map_err(io)?;
                let fps = u32::try_from(s.rate.num).unwrap_or(24);
                Some(ImmersiveMux::new(settings, fps, id, name, w, *sound_id)?)
            }
            _ => None,
        };
        Ok(DcpMux {
            dir,
            seed,
            title: s.title,
            kind: dcp_content_kind(settings).unwrap_or("feature"),
            rate: s.rate,
            size: (s.width, s.height),
            issued,
            picture,
            sound,
            immersive,
            frames: 0,
        })
    }

    /// The picture length, for tests that drive the sound tracks alone.
    #[cfg(test)]
    pub(crate) fn set_frames(&mut self, frames: u64) {
        self.frames = frames;
    }

    pub fn write_video(&mut self, packets: Vec<EncodedPacket>) -> Result<()> {
        let (_, _, w) = self.picture.as_mut().ok_or_else(|| ExportError::Encode("internal: the DCP picture file is closed".into()))?;
        for p in packets {
            w.push_picture(&p.data, FrameInfo::intra()).map_err(io)?;
            self.frames += 1;
        }
        Ok(())
    }

    pub fn write_audio(&mut self, planar: &[Vec<f32>]) -> Result<()> {
        let Some((_, _, w)) = self.sound.as_mut() else { return Ok(()) };
        if planar.first().is_none_or(Vec::is_empty) {
            return Ok(());
        }
        if let Some(im) = self.immersive.as_mut() {
            return im.write_audio(planar, w);
        }
        w.push_sound(0, &cfa4_mxf::encode_pcm(planar, 24)).map_err(io)
    }

    /// Finish the track files and write the composition playlist, packing list, asset map and
    /// volume index; returns (total bytes, every file written). `cancelled` is polled while an
    /// immersive source is copied.
    pub fn finish(mut self, settings: &ExportSettings, cancelled: &dyn Fn() -> bool) -> Result<(u64, Vec<String>)> {
        let (pic, mut snd) = (self.picture.take(), self.sound.take());
        // the IAB and the main sound get their last edit units first
        let iab = match (self.immersive.take(), snd.as_mut()) {
            (Some(im), Some((_, _, sw))) => {
                let (id, name) = (im.id, im.name.clone());
                let (w, units) = im.finish(self.frames, sw, cancelled)?;
                Some((id, name, w, units))
            }
            _ => None,
        };
        let dir = self.dir.clone();
        let mut files = Vec::new();
        let mut total = 0u64;
        let mut track = |id: [u8; 16], name: String, w: MxfWriter<Out>| -> Result<Asset> {
            let path = join(&dir, &name);
            let (size, hash) = finish_hashed(w.finish().map_err(io)?, settings, &path)?;
            total += size;
            files.push(path);
            Ok(Asset { id, name, size, hash, mime: "application/mxf", units: None })
        };
        let (pid, pname, pw) = pic.ok_or_else(|| ExportError::Encode("internal: the DCP picture file is closed".into()))?;
        let picture = track(pid, pname, pw)?;
        let sound = match snd {
            Some((id, name, w)) => Some(track(id, name, w)?),
            None => None,
        };
        let aux = match iab {
            Some((id, name, w, units)) => Some(Asset { units: Some(units), ..track(id, name, w)? }),
            None => None,
        };
        let mut write = |name: String, xml: String, id: [u8; 16]| -> Result<Asset> {
            let path = join(&dir, &name);
            let data = xml.into_bytes();
            let hash = sha1(&data);
            let size = crate::write_output(settings, &path, data)?;
            total += size;
            files.push(path);
            Ok(Asset { id, name, size, hash, mime: "text/xml", units: None })
        };
        let cpl_id = uuid(&format!("{}|cpl", self.seed));
        let cpl = write(format!("cpl_{}.xml", uuid_text(&cpl_id)), self.cpl(cpl_id, &picture, sound.as_ref(), aux.as_ref()), cpl_id)?;
        let pkl_id = uuid(&format!("{}|pkl", self.seed));
        let mut assets = vec![&cpl, &picture];
        assets.extend(sound.as_ref());
        assets.extend(aux.as_ref());
        let pkl = write(format!("pkl_{}.xml", uuid_text(&pkl_id)), self.pkl(pkl_id, &assets), pkl_id)?;
        assets.insert(0, &pkl);
        let am = self.assetmap(&assets, &pkl);
        let am_id = uuid(&format!("{}|assetmap", self.seed));
        write("ASSETMAP.xml".into(), am, am_id)?;
        let vol = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<VolumeIndex xmlns=\"http://www.smpte-ra.org/schemas/429-9/2007/AM\">\n  <Index>1</Index>\n</VolumeIndex>\n";
        write("VOLINDEX.xml".into(), vol.into(), am_id)?;
        Ok((total, files))
    }

    fn header(&self, out: &mut String, id: [u8; 16]) {
        out.push_str(&format!("  <Id>urn:uuid:{}</Id>\n", uuid_text(&id)));
        out.push_str(&format!("  <AnnotationText>{}</AnnotationText>\n", xml_text(&self.title)));
        out.push_str(&format!("  <IssueDate>{}</IssueDate>\n", self.issued));
        out.push_str("  <Issuer>CFA4</Issuer>\n");
        out.push_str(&format!("  <Creator>CFA4 {}</Creator>\n", env!("CARGO_PKG_VERSION")));
    }

    /// Composition playlist (ST 429-7): one reel, main picture, main sound and the immersive
    /// audio `AuxData` of ST 429-18 clause 14.
    fn cpl(&self, id: [u8; 16], picture: &Asset, sound: Option<&Asset>, aux: Option<&Asset>) -> String {
        let rate = format!("{} {}", self.rate.num, self.rate.den);
        let mut x =
            String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<CompositionPlaylist xmlns=\"http://www.smpte-ra.org/schemas/429-7/2006/CPL\">\n");
        self.header(&mut x, id);
        x.push_str(&format!("  <ContentTitleText>{}</ContentTitleText>\n", xml_text(&self.title)));
        x.push_str(&format!("  <ContentKind>{}</ContentKind>\n", self.kind));
        let version = uuid(&format!("{}|version", self.seed));
        x.push_str(&format!(
            "  <ContentVersion>\n    <Id>urn:uuid:{}</Id>\n    <LabelText>{}</LabelText>\n  </ContentVersion>\n",
            uuid_text(&version),
            xml_text(&self.title)
        ));
        x.push_str("  <RatingList/>\n  <ReelList>\n    <Reel>\n");
        x.push_str(&format!("      <Id>urn:uuid:{}</Id>\n      <AssetList>\n", uuid_text(&uuid(&format!("{}|reel1", self.seed)))));
        let track = |x: &mut String, a: &Asset| {
            x.push_str(&format!("          <Id>urn:uuid:{}</Id>\n", uuid_text(&a.id)));
            x.push_str(&format!("          <EditRate>{rate}</EditRate>\n"));
            x.push_str(&format!("          <IntrinsicDuration>{}</IntrinsicDuration>\n", a.units.unwrap_or(self.frames).max(self.frames)));
            x.push_str("          <EntryPoint>0</EntryPoint>\n");
            x.push_str(&format!("          <Duration>{}</Duration>\n", self.frames));
            x.push_str(&format!("          <Hash>{}</Hash>\n", base64(&a.hash)));
        };
        x.push_str("        <MainPicture>\n");
        track(&mut x, picture);
        x.push_str(&format!("          <FrameRate>{rate}</FrameRate>\n"));
        x.push_str(&format!("          <ScreenAspectRatio>{} {}</ScreenAspectRatio>\n", self.size.0, self.size.1));
        x.push_str("        </MainPicture>\n");
        if let Some(a) = sound {
            x.push_str("        <MainSound>\n");
            track(&mut x, a);
            x.push_str("        </MainSound>\n");
        }
        if let Some(a) = aux {
            x.push_str("        <axd:AuxData xmlns:axd=\"http://www.dolby.com/schemas/2012/AD\">\n");
            track(&mut x, a);
            x.push_str("          <axd:DataType>urn:smpte:ul:060e2b34.04010105.0e090604.00000000</axd:DataType>\n");
            x.push_str("        </axd:AuxData>\n");
        }
        x.push_str("      </AssetList>\n    </Reel>\n  </ReelList>\n</CompositionPlaylist>\n");
        x
    }

    /// Packing list (ST 429-8).
    fn pkl(&self, id: [u8; 16], assets: &[&Asset]) -> String {
        let mut x = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<PackingList xmlns=\"http://www.smpte-ra.org/schemas/429-8/2007/PKL\">\n");
        self.header(&mut x, id);
        x.push_str("  <AssetList>\n");
        for a in assets {
            x.push_str("    <Asset>\n");
            x.push_str(&format!("      <Id>urn:uuid:{}</Id>\n", uuid_text(&a.id)));
            x.push_str(&format!("      <AnnotationText>{}</AnnotationText>\n", xml_text(&a.name)));
            x.push_str(&format!("      <Hash>{}</Hash>\n", base64(&a.hash)));
            x.push_str(&format!("      <Size>{}</Size>\n", a.size));
            x.push_str(&format!("      <Type>{}</Type>\n", a.mime));
            x.push_str(&format!("      <OriginalFileName>{}</OriginalFileName>\n", xml_text(&a.name)));
            x.push_str("    </Asset>\n");
        }
        x.push_str("  </AssetList>\n</PackingList>\n");
        x
    }

    /// Asset map (ST 429-9): every asset, the packing list first.
    fn assetmap(&self, assets: &[&Asset], pkl: &Asset) -> String {
        let id = uuid(&format!("{}|assetmap", self.seed));
        let mut x = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<AssetMap xmlns=\"http://www.smpte-ra.org/schemas/429-9/2007/AM\">\n");
        x.push_str(&format!("  <Id>urn:uuid:{}</Id>\n", uuid_text(&id)));
        x.push_str(&format!("  <AnnotationText>{}</AnnotationText>\n", xml_text(&self.title)));
        x.push_str(&format!("  <Creator>CFA4 {}</Creator>\n", env!("CARGO_PKG_VERSION")));
        x.push_str("  <VolumeCount>1</VolumeCount>\n");
        x.push_str(&format!("  <IssueDate>{}</IssueDate>\n", self.issued));
        x.push_str("  <Issuer>CFA4</Issuer>\n  <AssetList>\n");
        for a in assets {
            x.push_str("    <Asset>\n");
            x.push_str(&format!("      <Id>urn:uuid:{}</Id>\n", uuid_text(&a.id)));
            if a.id == pkl.id {
                x.push_str("      <PackingList>true</PackingList>\n");
            }
            x.push_str("      <ChunkList>\n        <Chunk>\n");
            x.push_str(&format!("          <Path>{}</Path>\n", xml_text(&a.name)));
            x.push_str(&format!("          <VolumeIndex>1</VolumeIndex>\n          <Offset>0</Offset>\n          <Length>{}</Length>\n", a.size));
            x.push_str("        </Chunk>\n      </ChunkList>\n    </Asset>\n");
        }
        x.push_str("  </AssetList>\n</AssetMap>\n");
        x
    }
}
