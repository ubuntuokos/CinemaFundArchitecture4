//! MXF output of the stepped exporter: OP1a (one file, frame-wrapped picture + one PCM track) or
//! OP-Atom (Avid style: the picture in `path`, one mono PCM file per audio channel next to it,
//! `<stem>_A1.mxf`, `<stem>_A2.mxf`, …), written with [`cfa4_mxf::MxfWriter`].

use cfa4_mxf::{
    CodedKind, ColorSpace, ComponentSpec, FrameInfo, MxfWriter, PackageIds, Pattern, PictureCoding, PictureDesc, Rational, SoundDesc, StartTimecode, Timestamp,
    VersionClipSpec, VersionConfig, VersionTrackKind, VersionTrackSpec, WriterConfig, write_version,
};
use cfa4_time::FrameRate;

use crate::{BitrateMode, ColorSignal, EncodedPacket, ExportError, ExportSettings, FieldOrder, Format, H264Profile, MpegVideoInfo, MxfVideoCodec, Out, Result};

/// Paths of the OP-Atom audio files for an export to `path` with `channels` channels.
pub fn opatom_audio_paths(path: &str, channels: usize) -> Vec<String> {
    let p = std::path::Path::new(path);
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "export".into());
    (1..=channels)
        .map(|k| {
            let name = format!("{stem}_A{k}.mxf");
            match p.parent().filter(|d| !d.as_os_str().is_empty()) {
                Some(d) => d.join(name).to_string_lossy().to_string(),
                None => name,
            }
        })
        .collect()
}

fn io(e: std::io::Error) -> ExportError {
    ExportError::Io(e.to_string())
}

/// The picture essence description for an export (codec from the MXF video codec setting; MPEG-2
/// from what its encoder reports).
fn picture(settings: &ExportSettings, w: u32, h: u32, rate: FrameRate, mpeg: Option<MpegVideoInfo>) -> Result<PictureDesc> {
    if settings.mxf_video_codec == MxfVideoCodec::D10 {
        let (mbps, lines) = crate::d10_params(settings, w, h, rate)?;
        let mut d = PictureDesc::new(PictureCoding::D10 { mbps, lines }, 720, if lines == 625 { 608 } else { 512 });
        // the display aspect ratio of the active picture: 16:9 when the pixel aspect says so
        let wide = settings.pixel_aspect.is_some_and(|(n, dn)| dn > 0 && u64::from(w) * u64::from(n) * 9 >= u64::from(h) * u64::from(dn) * 15);
        d.aspect = if wide { Rational::new(16, 9) } else { Rational::new(4, 3) };
        d.field_dominance = (settings.field_order != FieldOrder::Progressive).then_some(1);
        return Ok(d);
    }
    let coding = match settings.video_format() {
        _ if let Some(layout) = settings.mxf_video_codec.uncompressed_layout() => PictureCoding::Uncompressed { layout },
        Format::Mpeg2 => {
            let m = mpeg.ok_or_else(|| ExportError::Encode("internal: MPEG-2 encoder without stream parameters".into()))?;
            if !matches!(m.profile_and_level, 0x44 | 0x46 | 0x82) {
                return Err(ExportError::Unsupported(format!(
                    "MPEG-2 in MXF is written for HD pictures (High and High-1440 levels); {w}x{h} is standard definition (D-10 / IMX)"
                )));
            }
            if settings.mxf_video_codec == MxfVideoCodec::XdcamHd422 && (m.profile_and_level != 0x82 || !matches!((w, h), (1920, 1080) | (1280, 720))) {
                return Err(ExportError::Unsupported(format!("XDCAM HD422 (RDD 9) pictures are 1920x1080 or 1280x720 in the 4:2:2 Profile; not {w}x{h}")));
            }
            if settings.field_order != FieldOrder::Progressive {
                // ST 274 interlaced: 1080 lines; RDD 9 has 1080i at 25 and 29.97 frames per second
                if h != 1080 {
                    return Err(ExportError::Unsupported(format!("interlaced MPEG-2 in MXF is 1080 lines (1920x1080 or 1440x1080), not {w}x{h}")));
                }
                if settings.mxf_video_codec == MxfVideoCodec::XdcamHd422 && !matches!((rate.num, rate.den), (25, 1) | (30000, 1001)) {
                    return Err(ExportError::Unsupported(format!(
                        "XDCAM HD422 (RDD 9) 1080i runs at 25 or 29.97 fps (50i / 59.94i), not {} fps",
                        rate.label()
                    )));
                }
            }
            PictureCoding::Mpeg2 {
                profile_and_level: m.profile_and_level,
                max_gop: u16::try_from(m.gop.max(1)).unwrap_or(u16::MAX),
                b_pictures: u8::try_from(m.b_frames).unwrap_or(u8::MAX),
                closed_gop: m.closed_gop,
                bit_rate: u32::try_from(m.bit_rate).unwrap_or(u32::MAX),
            }
        }
        Format::ProRes => {
            use cfa4_prores::Profile;
            PictureCoding::ProRes {
                profile: match crate::prores_profile(&settings.prores_profile) {
                    Profile::Proxy => 1,
                    Profile::Lt => 2,
                    Profile::Standard => 3,
                    Profile::Hq => 4,
                    Profile::P4444 => 5,
                    Profile::P4444Xq => 6,
                },
            }
        }
        Format::H264 => PictureCoding::Avc {
            profile_idc: match settings.h264_profile {
                H264Profile::Baseline => 66,
                H264Profile::Main => 77,
                H264Profile::High => 100,
            },
            intra: false,
        },
        _ => PictureCoding::Vc3 { cid: crate::dnx_profile(&settings.dnx_profile).cid() },
    };
    let mut d = PictureDesc::new(coding, w, h);
    d.field_dominance = match settings.field_order {
        FieldOrder::Progressive => None,
        FieldOrder::UpperFirst => Some(1),
        FieldOrder::LowerFirst => Some(2),
    };
    d.color = match settings.signal {
        s if s == ColorSignal::PQ => ColorSpace::Rec2020Pq,
        s if s == ColorSignal::HLG => ColorSpace::Rec2020Hlg,
        _ => ColorSpace::Rec709,
    };
    if let Some((n, dn)) = settings.pixel_aspect.filter(|(n, d)| *n > 0 && *d > 0 && n != d) {
        let (a, b) = (w as u64 * n as u64, h as u64 * dn as u64);
        let g = gcd(a, b);
        d.aspect = Rational::new((a / g) as i32, (b / g) as i32);
    }
    Ok(d)
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a.max(1) } else { gcd(b, a % b) }
}

/// Everything the MXF muxer needs from the export.
pub(crate) struct MxfSetup<'a> {
    pub settings: &'a ExportSettings,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub rate: FrameRate,
    /// (sample rate, channels) of the audio, if any.
    pub audio: Option<(u32, usize)>,
    /// Start timecode in frames of `rate`, and drop-frame.
    pub timecode: (i64, bool),
    /// MPEG-2 video: the encoder's stream parameters.
    pub mpeg: Option<MpegVideoInfo>,
    /// Closed captions for the ANC data track (`ExportSettings::mxf_anc_captions`), timed from
    /// the first exported frame.
    pub captions: Option<cfa4_captions::Document>,
    /// AS-11 programme parts from segmentation markers, (start, duration) in frames from the first
    /// exported frame; used when `ExportSettings::mxf_as11_segments` is empty.
    pub as11_parts: Vec<(i64, i64)>,
}

/// AS-11 segments of the marker parts: numbered in order, each of the total.
fn as11_marker_parts(parts: &[(i64, i64)]) -> std::result::Result<Vec<cfa4_mxf::as11::Segment>, String> {
    let total = u16::try_from(parts.len()).map_err(|_| format!("{} segmentation markers, more than an AS-11 part number can count", parts.len()))?;
    Ok(parts.iter().zip(1u16..).map(|(&(start, duration), part_number)| cfa4_mxf::as11::Segment { start, duration, part_number, part_total: total }).collect())
}

/// What an AS-02 bundle export needs to write the version file and the manifest at the end.
struct As02Bundle {
    /// The bundle folder and its name (the folder's name, also the version file's stem).
    dir: String,
    name: String,
    /// The bundle's own name (the folder's), which differs from `name` for versions 2 and up.
    bundle: String,
    /// 0 for the first version, 2 and up for a version appended to an existing bundle.
    version: u32,
    rate: Rational,
    modified: Timestamp,
    seed: String,
    /// File package UMID and bundle-relative path of the picture component and of every sound one.
    picture: (cfa4_mxf::Umid, String),
    sound: Vec<(cfa4_mxf::Umid, String)>,
}

/// A UUID URN from a seed (version 4 layout).
fn uuid_urn(seed: &str) -> String {
    let mut b = cfa4_mxf::Umid::from_seed(seed.as_bytes()).material_number();
    b[6] = (b[6] & 0x0F) | 0x40;
    b[8] = (b[8] & 0x3F) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("urn:uuid:{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

/// An `xs:dateTime` in UTC.
fn xs_date_time(t: Timestamp) -> String {
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", t.year, t.month, t.day, t.hour, t.minute, t.second)
}

/// An MXF export in progress.
pub(crate) struct MxfMux {
    video: MxfWriter<Out>,
    /// OP-Atom: one writer per audio channel.
    audio: Vec<(String, MxfWriter<Out>)>,
    atom: bool,
    /// AS-02: the files of the bundle are separate mono OP1a files (`audio` holds the sound ones).
    as02: Option<As02Bundle>,
    bits: u32,
    channels: usize,
    /// Stored pictures so far, and the composition offset of the first (H.264 reorder delay).
    stored: i64,
    first_offset: Option<i64>,
    max_display: i64,
    frame_den: i64,
    /// Closed captions: one caption distribution packet per content package.
    cdp: Option<cfa4_captions::mcc::CdpStream>,
}

impl MxfMux {
    pub fn new(s: MxfSetup<'_>) -> Result<Self> {
        let settings = s.settings;
        let atom = settings.format == Format::MxfOpAtom;
        let as02_on = settings.mxf_as02;
        let rate = Rational::new(s.rate.num as i32, s.rate.den as i32);
        let now = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let seed = format!("{}|{now}", settings.path);
        let ids = PackageIds::from_seed(&seed, &s.name);
        // integers only: `ExportSettings::validate` refuses float PCM in MXF
        let bits = u32::from(settings.audio.sample_format().bits());
        let mut cfg = WriterConfig::new(if atom { Pattern::OpAtom } else { Pattern::Op1a }, rate, ids.clone());
        cfg.picture = Some(picture(settings, s.width, s.height, s.rate, s.mpeg)?);
        cfg.modified = Timestamp::from_unix((now / 1_000_000_000) as i64);
        cfg.timecode = Some(StartTimecode { frames: s.timecode.0, rate, drop_frame: s.timecode.1 && s.rate.supports_drop_frame() });
        if !atom
            && !as02_on
            && let Some((sr, ch)) = s.audio
        {
            cfg.sound = vec![SoundDesc { sample_rate: sr, channels: ch as u32, bits }];
        }
        cfg.rdd9 = settings.mxf_video_codec == MxfVideoCodec::XdcamHd422;
        if !atom && !settings.mxf_as11_core.trim().is_empty() {
            use cfa4_mxf::as11;
            let bad = |e: String| ExportError::Unsupported(format!("AS-11: {e}"));
            let base = cfg.timecode.map_or(25, |t| t.rounded_base());
            let clock = as11::Clock { start: s.timecode.0, base };
            let ukdpp =
                if settings.mxf_as11_ukdpp.trim().is_empty() { Vec::new() } else { as11::parse_framework_text(&settings.mxf_as11_ukdpp).map_err(bad)? };
            cfg.as11 = Some(as11::As11 {
                core: as11::parse_framework_text(&settings.mxf_as11_core).map_err(bad)?,
                segments: if settings.mxf_as11_segments.trim().is_empty() {
                    as11_marker_parts(&s.as11_parts).map_err(bad)?
                } else {
                    as11::parse_segments_text(&settings.mxf_as11_segments, clock).map_err(bad)?
                },
                ukdpp,
                as10: Vec::new(),
                as03: Vec::new(),
                as12: Vec::new(),
            });
        }
        if !atom && !settings.mxf_as10.trim().is_empty() {
            use cfa4_mxf::as10;
            let bad = |e: String| ExportError::Unsupported(format!("AS-10: {e}"));
            let values = cfa4_mxf::as11::parse_framework_text(&settings.mxf_as10).map_err(bad)?;
            let (profile_and_level, chroma_422) = match &cfg.picture {
                Some(d) => match d.coding {
                    PictureCoding::Mpeg2 { profile_and_level, .. } => (profile_and_level, profile_and_level == 0x82),
                    _ => (0, false),
                },
                None => (0, false),
            };
            let facts = as10::Facts {
                profile_and_level,
                chroma_422,
                width: s.width,
                height: s.height,
                audio_channels: s.audio.map_or(0, |(_, ch)| ch as u32),
                audio_bits: bits,
                audio_sample_rate: s.audio.map_or(0, |(sr, _)| sr),
                cbr: Some(settings.mpeg2_quantizer.is_none() && settings.bitrate_mode == BitrateMode::Cbr),
                bit_rate: match cfg.picture.as_ref().map(|d| d.coding) {
                    Some(PictureCoding::Mpeg2 { bit_rate, .. }) => bit_rate,
                    _ => 0,
                },
            };
            if let Some(name) = values.iter().find(|(k, _)| k.eq_ignore_ascii_case("ShimName")).map(|(_, v)| v.as_str()) {
                as10::check_shim(name, &facts).map_err(bad)?;
            }
            cfg.as11.get_or_insert_with(Default::default).as10 = values;
        }
        if !atom && !settings.mxf_as03.trim().is_empty() {
            use cfa4_mxf::as03;
            let bad = |e: String| ExportError::Unsupported(format!("AS-03: {e}"));
            let values = cfa4_mxf::as11::parse_framework_text(&settings.mxf_as03).map_err(bad)?;
            let (video, bit_rate) = match cfg.picture.as_ref().map(|d| d.coding) {
                Some(PictureCoding::Mpeg2 { bit_rate, .. }) => (as03::Video::Mpeg2, bit_rate),
                Some(PictureCoding::Avc { .. }) => (as03::Video::H264, settings.bitrate_kbps.saturating_mul(1000)),
                _ => (as03::Video::Other, 0),
            };
            let facts = as03::Facts {
                video,
                width: s.width,
                height: s.height,
                bit_rate,
                audio_channels: s.audio.map_or(0, |(_, ch)| ch as u32),
                audio_bits: bits,
                audio_sample_rate: s.audio.map_or(0, |(sr, _)| sr),
            };
            let name = values.iter().find(|(k, _)| k.eq_ignore_ascii_case("ShimName")).map(|(_, v)| v.as_str()).unwrap_or_default();
            as03::check_shim(name, &facts).map_err(bad)?;
            cfg.as11.get_or_insert_with(Default::default).as03 = values;
        }
        if !atom && !settings.mxf_as12.trim().is_empty() {
            let bad = |e: String| ExportError::Unsupported(format!("AS-12: {e}"));
            cfg.as11.get_or_insert_with(Default::default).as12 = cfa4_mxf::as11::parse_framework_text(&settings.mxf_as12).map_err(bad)?;
        }
        let cdp = match s.captions.as_ref().filter(|_| !atom) {
            Some(doc) => {
                let Some(st) = cfa4_captions::mcc::CdpStream::new_at(doc, s.rate) else {
                    return Err(ExportError::Unsupported(format!(
                        "closed captions in MXF (ANC) need 23.976, 24, 25, 29.97, 30, 50, 59.94 or 60 fps (SMPTE ST 334-2), not {}",
                        s.rate.label()
                    )));
                };
                cfg.anc = true;
                Some(st)
            }
            None => None,
        };
        let mut audio = Vec::new();
        let mut as02 = None;
        let mut video_path = settings.path.clone();
        if as02_on {
            // AS-02 6.2: every essence component is a mono OP1a file under media/, with a file
            // package of its own; the version file ties them together (written in `finish`)
            let dir = settings.path.trim_end_matches(['/', '\\']).to_string();
            let bundle =
                std::path::Path::new(&dir).file_name().map(|n| n.to_string_lossy().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| "bundle".into());
            let version = settings.mxf_as02_version;
            let name = if version >= 2 { format!("{bundle}_v{version}") } else { bundle.clone() };
            std::fs::create_dir_all(format!("{dir}/media")).map_err(io)?;
            let component = |tag: &str| {
                let rel = format!("media/{name}_{tag}.mxf");
                let ids = PackageIds {
                    material: cfa4_mxf::Umid::from_seed(format!("material:{seed}:{tag}").as_bytes()),
                    file: cfa4_mxf::Umid::from_seed(format!("file:{seed}:{tag}").as_bytes()),
                    material_name: format!("{} {tag}", s.name),
                    file_name: format!("{} {tag}", s.name),
                };
                (rel, ids)
            };
            let (rel, vids) = component("V");
            cfg.ids = vids.clone();
            video_path = format!("{dir}/{rel}");
            let picture = (vids.file, rel);
            let mut sound = Vec::new();
            if let Some((sr, ch)) = s.audio {
                for k in 1..=ch {
                    let (rel, ids) = component(&format!("A{k}"));
                    let mut a = WriterConfig::new(Pattern::Op1a, rate, ids.clone());
                    a.sound = vec![SoundDesc { sample_rate: sr, channels: 1, bits }];
                    a.timecode = cfg.timecode;
                    a.modified = cfg.modified;
                    let path = format!("{dir}/{rel}");
                    audio.push((path.clone(), MxfWriter::new(Out::create_path(settings, &path)?, a).map_err(io)?));
                    sound.push((ids.file, rel));
                }
            }
            as02 = Some(As02Bundle { dir, name, bundle, version, rate, modified: cfg.modified, seed: seed.clone(), picture, sound });
        } else if atom && let Some((sr, ch)) = s.audio {
            for (k, path) in opatom_audio_paths(&settings.path, ch).into_iter().enumerate() {
                // Avid style: the files share the material package; each has its own file package.
                let mut a = WriterConfig::new(
                    Pattern::OpAtom,
                    rate,
                    PackageIds {
                        material: ids.material,
                        file: cfa4_mxf::Umid::from_seed(format!("file:{seed}:A{}", k + 1).as_bytes()),
                        material_name: ids.material_name.clone(),
                        file_name: format!("{} A{}", s.name, k + 1),
                    },
                );
                a.first_track_id = 3 + k as u32;
                a.sound = vec![SoundDesc { sample_rate: sr, channels: 1, bits }];
                a.timecode = cfg.timecode;
                a.modified = cfg.modified;
                let out = Out::create_path(settings, &path)?;
                audio.push((path, MxfWriter::new(out, a).map_err(io)?));
            }
        }
        let video = MxfWriter::new(Out::create_path(settings, &video_path)?, cfg).map_err(io)?;
        Ok(MxfMux {
            video,
            audio,
            atom,
            as02,
            bits,
            channels: s.audio.map_or(0, |a| a.1),
            stored: 0,
            first_offset: None,
            max_display: -1,
            frame_den: s.rate.den.max(1),
            cdp,
        })
    }

    pub fn write_video(&mut self, packets: Vec<EncodedPacket>) -> Result<()> {
        for p in packets {
            let intra = !matches!(
                self.video.config().picture.as_ref().map(|p| p.coding),
                Some(PictureCoding::Avc { .. }) | Some(PictureCoding::Mpeg2 { max_gop: 2.., .. })
            );
            let info = if intra {
                FrameInfo::intra()
            } else {
                // display position from the composition offset (pts − dts) relative to the first
                let off = p.composition_offset as i64;
                let first = *self.first_offset.get_or_insert(off);
                let display = self.stored + (off - first) / self.frame_den;
                let kind = if p.key {
                    CodedKind::Intra
                } else if display < self.max_display {
                    CodedKind::Bidirectional
                } else {
                    CodedKind::Predicted
                };
                self.max_display = self.max_display.max(display);
                FrameInfo { key: p.key, kind, display: Some(display) }
            };
            if let Some(c) = self.cdp.as_mut() {
                // ST 436-1 5.2: the ANC elements are in display order, one per content package;
                // the CDP rides on VANC line 9 (ST 334-1), 8-bit luma samples
                let words = c.frame(self.stored);
                let packet = cfa4_mxf::AncPacket { line: 9, wrapping: 0x01, coding: 4, words };
                self.video.push_anc(&cfa4_mxf::anc_element(&[packet])).map_err(io)?;
            }
            self.video.push_picture(&p.data, info).map_err(io)?;
            self.stored += 1;
        }
        Ok(())
    }

    pub fn write_audio(&mut self, planar: &[Vec<f32>]) -> Result<()> {
        if self.channels == 0 || planar.first().is_none_or(Vec::is_empty) {
            return Ok(());
        }
        if self.atom || self.as02.is_some() {
            for (k, (_, w)) in self.audio.iter_mut().enumerate() {
                if let Some(c) = planar.get(k) {
                    w.push_sound(0, &cfa4_mxf::encode_pcm(std::slice::from_ref(c), self.bits)).map_err(io)?;
                }
            }
            Ok(())
        } else {
            self.video.push_sound(0, &cfa4_mxf::encode_pcm(planar, self.bits)).map_err(io)
        }
    }

    /// Finish every file; returns (total bytes, extra files written besides `settings.path`).
    pub fn finish(self, settings: &ExportSettings) -> Result<(u64, Vec<String>)> {
        if let Some(b) = self.as02 {
            return finish_bundle(b, self.video, self.audio, settings);
        }
        let mut total = self.video.finish().map_err(io)?.finish_path(settings, &settings.path)?;
        let mut extra = Vec::new();
        for (path, w) in self.audio {
            total += w.finish().map_err(io)?.finish_path(settings, &path)?;
            extra.push(path);
        }
        Ok((total, extra))
    }
}

/// The AS-02 media integrity check (9.2.5) of a whole file: MD5 over every byte, in hex.
fn md5_mic(path: &str) -> Result<cfa4_mxf::as02::Mic> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| ExportError::Io(format!("{path}: {e}")))?;
    let mut h = cfa4_flac::Md5::default();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| ExportError::Io(format!("{path}: {e}")))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    let value = h.digest().iter().map(|b| format!("{b:02x}")).collect();
    Ok(cfa4_mxf::as02::Mic { kind: "md5".into(), scope: "entire_file".into(), value })
}

/// The file descriptor of a finished component file, for the version file's copy of its package.
fn descriptor_of(path: &str, umid: &cfa4_mxf::Umid) -> Result<Vec<cfa4_mxf::as02_version::CopiedSet>> {
    let bytes = std::fs::read(path).map_err(|e| ExportError::Io(format!("{path}: {e}")))?;
    cfa4_mxf::as02_version::copy_descriptor(&bytes, Some(umid.0)).map_err(|e| ExportError::Encode(format!("AS-02 component {path}: {e}")))
}

/// Finish the component files of an AS-02 bundle, then write the version file (the material
/// package that cuts them together, OP1b with external essence) and the manifest.
fn finish_bundle(b: As02Bundle, video: MxfWriter<Out>, audio: Vec<(String, MxfWriter<Out>)>, settings: &ExportSettings) -> Result<(u64, Vec<String>)> {
    use cfa4_mxf::as02::{File, Manifest, Role, file_id_of_umid};
    let mut files: Vec<File> = vec![File { id: String::new(), role: Role::Folder, size: 0, path: "media".into(), mic: None, annotations: Vec::new() }];
    let mut extra = Vec::new();
    let mut total = 0u64;
    let mut components = Vec::new();
    let mut tracks: Vec<VersionTrackSpec> = Vec::new();
    let mut add = |kind: VersionTrackKind,
                   umid: cfa4_mxf::Umid,
                   rel: &str,
                   units: u64,
                   size: u64,
                   mic: cfa4_mxf::as02::Mic,
                   descriptor: Vec<cfa4_mxf::as02_version::CopiedSet>| {
        let duration = i64::try_from(units).unwrap_or(i64::MAX);
        files.push(File { id: file_id_of_umid(&umid.0), role: Role::EssenceComponent, size, path: rel.to_string(), mic: Some(mic), annotations: Vec::new() });
        components.push(ComponentSpec { umid, kind, edit_rate: b.rate, duration, track_id: 2, locator: rel.to_string(), descriptor });
        let n = tracks.len() as u32;
        tracks.push(VersionTrackSpec {
            kind,
            track_id: 10 + n,
            track_number: 1 + n,
            name: match kind {
                VersionTrackKind::Picture => "V1".to_string(),
                VersionTrackKind::Sound => format!("A{n}"),
            },
            edit_rate: b.rate,
            clips: vec![VersionClipSpec { package: Some(umid), source_track: 2, start: 0, duration }],
        });
    };
    let units = video.edit_units();
    let vpath = format!("{}/{}", b.dir, b.picture.1);
    let size = video.finish().map_err(io)?.finish_path(settings, &vpath)?;
    total += size;
    add(VersionTrackKind::Picture, b.picture.0, &b.picture.1, units, size, md5_mic(&vpath)?, descriptor_of(&vpath, &b.picture.0)?);
    extra.push(vpath);
    for ((path, w), (umid, rel)) in audio.into_iter().zip(&b.sound) {
        let units = w.edit_units();
        let size = w.finish().map_err(io)?.finish_path(settings, &path)?;
        total += size;
        add(VersionTrackKind::Sound, *umid, rel, units, size, md5_mic(&path)?, descriptor_of(&path, umid)?);
        extra.push(path);
    }
    let material = cfa4_mxf::Umid::from_seed(format!("version:{}", b.seed).as_bytes());
    let mut v = VersionConfig::new(material, &b.name);
    v.tracks = tracks;
    v.components = components;
    v.modified = b.modified;
    let bytes = write_version(&v).map_err(io)?;
    let version_rel = format!("{}.mxf", b.name);
    let version_path = format!("{}/{version_rel}", b.dir);
    total += bytes.len() as u64;
    std::fs::write(&version_path, &bytes).map_err(|e| ExportError::Io(format!("{version_path}: {e}")))?;
    files.insert(
        0,
        File {
            id: file_id_of_umid(&material.0),
            role: Role::Version,
            size: bytes.len() as u64,
            path: version_rel,
            mic: Some(md5_mic(&version_path)?),
            annotations: Vec::new(),
        },
    );
    if b.version < 2 {
        // the shim file (section 10): the bundle follows the general provisions, mono essence OP1a
        // components and a version file, with no named shim
        let shim = cfa4_mxf::as02::Shim {
            name: "CFA4 general AS-02 bundle".into(),
            id: None,
            annotations: vec!["Mono-essence OP1a components with an OP1b version file; no further constraints.".into()],
        }
        .to_xml();
        let shim_path = format!("{}/shim.xml", b.dir);
        std::fs::write(&shim_path, shim.as_bytes()).map_err(|e| ExportError::Io(format!("{shim_path}: {e}")))?;
        total += shim.len() as u64;
        files.insert(
            1,
            File {
                id: uuid_urn(&format!("shim:{}", b.seed)),
                role: Role::Shim,
                size: shim.len() as u64,
                path: "shim.xml".into(),
                mic: Some(md5_mic(&shim_path)?),
                annotations: Vec::new(),
            },
        );
        extra.push(shim_path);
    }
    let manifest_path = format!("{}/manifest.xml", b.dir);
    let manifest = if b.version >= 2 {
        // a further version: its files join the manifest of the bundle written before
        let old = std::fs::read_to_string(&manifest_path).map_err(|e| ExportError::Io(format!("{manifest_path}: {e}")))?;
        let mut m = Manifest::parse(&old).map_err(|e| ExportError::Encode(format!("AS-02 bundle: {manifest_path}: {e}")))?;
        m.files.extend(files.into_iter().filter(|f| f.role != Role::Folder));
        m
    } else {
        Manifest {
            bundle_name: b.bundle.clone(),
            bundle_id: uuid_urn(&format!("bundle:{}", b.seed)),
            creator: "CFA4".into(),
            creation_date: xs_date_time(b.modified),
            annotations: Vec::new(),
            files,
        }
    };
    // one version is a simple bundle (4.3.8); with more versions only the general rules apply
    let problems = if b.version >= 2 { manifest.check() } else { manifest.check_simple() };
    if !problems.is_empty() {
        return Err(ExportError::Encode(format!("AS-02 bundle: {}", problems.join("; "))));
    }
    let xml = manifest.to_xml();
    std::fs::write(&manifest_path, xml.as_bytes()).map_err(|e| ExportError::Io(format!("{manifest_path}: {e}")))?;
    total += xml.len() as u64;
    extra.push(version_path);
    extra.push(manifest_path);
    Ok((total, extra))
}
