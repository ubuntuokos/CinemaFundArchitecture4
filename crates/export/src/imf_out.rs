//! IMF export (#1011): an Interoperable Master Package of SMPTE ST 2067-21 (Application #2E) in
//! the output folder.
//!
//! - Picture: JPEG 2000 (our encoder, the 2K / 4K / 8K IMF profile the frame size needs, lossy or
//!   reversible) of 10-bit full-range R'G'B' 4:4:4 (BT.709, COLOR.3 with QE.2), frame-wrapped in
//!   an IMF image track file (ST 2067-5).
//! - Sound: 24-bit 48 kHz PCM, mono, stereo or 5.1, clip-wrapped in an IMF audio track file with
//!   the MCA labels of ST 2067-8 / ST 428-12 and the soundfield group titles of ST 2067-2.
//! - One segment; composition playlist (ST 2067-3, with each track file's essence descriptor as
//!   RegXML, [`cfa4_mxf::regxml`]), IMF packing list (ST 2067-2 Annex J), asset map (ST 429-9) and
//!   volume index, with SHA-1 hashes and random (version 4) UUIDs.
//!
//! Unencrypted and unsigned. The composition edit rate is the App 2E rate nearest the export's.

use std::io::{Read, Write};

use cfa4_mxf::{
    FrameInfo, J2kSubDescriptor, McaLabel, McaTitles, MxfWriter, PackageIds, Pattern, PictureCoding, PictureDesc, Rational, SoundDesc, StartTimecode,
    Timestamp, Umid, WriterConfig,
};
use cfa4_time::FrameRate;
use rayon::prelude::*;

use crate::dcp_out::{Sha1, base64, io, join, sha1, uuid, uuid_text, xml_text};
use crate::{AudioCodec, EncodedPacket, EncoderFrame, ExportError, ExportSettings, Format, Out, Result, SampleFormat, Stage, VideoEncoder, timed};

/// Content kinds of a composition playlist (ST 2067-3 Table 3, the default scope).
pub const IMF_CONTENT_KINDS: [&str; 12] =
    ["feature", "trailer", "teaser", "short", "advertisement", "test", "transitional", "rating", "psa", "episode", "highlights", "event"];

/// The App 2E progressive frame rates (ST 2067-21 Table 2), as (numerator, denominator).
pub const IMF_RATES: [(i64, i64); 8] = [(24, 1), (24000, 1001), (25, 1), (30, 1), (30000, 1001), (50, 1), (60, 1), (60000, 1001)];

/// The largest App 2E picture (the 8K IMF profiles).
pub const IMF_MAX_SIZE: (u32, u32) = (8192, 6224);

/// ST 2067-21 8.1, the 5th edition.
const APPLICATION_ID: &str = "http://www.smpte-ra.org/ns/2067-21/5ED";

/// The content kind an export writes (`feature` unless set), if it is one of [`IMF_CONTENT_KINDS`].
pub fn imf_content_kind(s: &ExportSettings) -> Option<&'static str> {
    let k = s.imf_content_kind.trim().to_ascii_lowercase();
    if k.is_empty() {
        return Some("feature");
    }
    IMF_CONTENT_KINDS.into_iter().find(|c| *c == k)
}

/// The App 2E rate nearest `want` (exact rates kept; the lower one on a tie).
pub fn imf_rate(want: FrameRate) -> FrameRate {
    let want = want.sane();
    if IMF_RATES.iter().any(|&(n, d)| FrameRate::new(n, d) == want) {
        return want;
    }
    let fps = want.as_f64();
    let fps_of = |(n, d): (i64, i64)| n as f64 / d as f64;
    let best = IMF_RATES
        .iter()
        .copied()
        .min_by(|a, b| (fps_of(*a) - fps).abs().total_cmp(&(fps_of(*b) - fps).abs()).then(fps_of(*a).total_cmp(&fps_of(*b))))
        .unwrap_or((24, 1));
    FrameRate::new(best.0, best.1)
}

/// The settings an IMF export runs with: an App 2E rate, 24-bit 48 kHz PCM, SDR, no alpha.
pub(crate) fn normalize(s: &ExportSettings, seq_rate: FrameRate) -> ExportSettings {
    let mut s = s.clone();
    s.frame_rate = Some(imf_rate(s.frame_rate.unwrap_or(seq_rate)));
    s.audio.sample_rate = Some(48_000);
    s.audio.codec = AudioCodec::Pcm;
    s.audio.set_sample_format(SampleFormat::Int24);
    s.sdr = true;
    s.alpha = false;
    s
}

/// Whether `s` already is what [`normalize`] makes of it.
pub(crate) fn is_normalized(s: &ExportSettings) -> bool {
    s.frame_rate.is_some_and(|r| IMF_RATES.iter().any(|&(n, d)| FrameRate::new(n, d) == r))
        && s.audio.sample_rate == Some(48_000)
        && s.audio.sample_format() == SampleFormat::Int24
}

/// Errors in the IMF settings (content kind, language).
pub(crate) fn validate(s: &ExportSettings) -> Result<()> {
    if imf_content_kind(s).is_none() {
        return Err(ExportError::Unsupported(format!("unknown IMF content kind {:?}: choose one of {}", s.imf_content_kind, IMF_CONTENT_KINDS.join(", "))));
    }
    if crate::dcp_language(s).is_none() {
        return Err(ExportError::Unsupported(format!("IMF language {:?} is not an RFC 5646 tag such as en-US or hu-HU", s.dcp_language)));
    }
    Ok(())
}

/// 8-bit RGBA as 10-bit full-range (QE.2) R', G' and B' planes.
pub fn rgba_to_rgb10(rgba: &[u8], w: usize, h: usize) -> [Vec<i32>; 3] {
    let n = w.saturating_mul(h);
    let mut planes = [vec![0i32; n], vec![0i32; n], vec![0i32; n]];
    let ten = |v: u8| (i32::from(v) * 1023 + 127) / 255;
    let [r, g, b] = &mut planes;
    for (((px, r), g), b) in rgba.chunks_exact(4).zip(r.iter_mut()).zip(g.iter_mut()).zip(b.iter_mut()) {
        (*r, *g, *b) = (ten(px[0]), ten(px[1]), ten(px[2]));
    }
    planes
}

/// The JPEG 2000 IMF profile of an export: the size class of the picture, reversible when
/// lossless is asked for, else the sublevel whose bit rate holds an eighth of the raw 10-bit
/// R'G'B' rate (at least 100 Mbit/s).
pub fn imf_profile(w: u32, h: u32, rate: FrameRate, lossless: bool) -> Option<cfa4_j2k::ImfProfile> {
    let (num, den) = (u32::try_from(rate.num).ok()?, u32::try_from(rate.den).ok()?);
    let raw = u128::from(w) * u128::from(h) * 30 * u128::from(num) / u128::from(den.max(1));
    let bitrate = u64::try_from(raw / 8).unwrap_or(u64::MAX).max(100_000_000);
    cfa4_j2k::ImfProfile::select(w, h, 3, num, den, lossless, bitrate)
}

/// JPEG 2000 for IMF App 2E: 10-bit R'G'B' 4:4:4 in an IMF profile, coded a batch at a time in
/// parallel like the DCP encoder.
struct ImfEncoder {
    opts: cfa4_j2k::EncodeOptions,
    rate: FrameRate,
    pending: Vec<(u32, u32, Vec<u8>)>,
    batch: usize,
}

impl ImfEncoder {
    fn code(&self, w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>> {
        let [r, g, b] = timed(Stage::Convert, || rgba_to_rgb10(rgba, w as usize, h as usize));
        let plane = |data: Vec<i32>| cfa4_j2k::Plane { width: w, height: h, precision: 10, signed: false, dx: 1, dy: 1, data };
        let image = cfa4_j2k::Image { width: w, height: h, components: vec![plane(r), plane(g), plane(b)], mct: true };
        cfa4_j2k::encode(&image, &self.opts).map_err(|e| ExportError::Encode(format!("JPEG 2000: {e}")))
    }

    fn drain(&mut self) -> Result<Vec<EncodedPacket>> {
        let frames = std::mem::take(&mut self.pending);
        let coded: Vec<Result<Vec<u8>>> = frames.par_iter().map(|(w, h, rgba)| self.code(*w, *h, rgba)).collect();
        coded.into_iter().map(|data| Ok(EncodedPacket { data: data?, key: true, duration: 1, composition_offset: 0 })).collect()
    }
}

impl VideoEncoder for ImfEncoder {
    fn sample_entry(&self) -> cfa4_isobmff::SampleEntry {
        // never written: IMF pictures go to MXF track files
        let fourcc = cfa4_isobmff::FourCc(*b"mjp2");
        cfa4_isobmff::SampleEntry::video(fourcc, cfa4_isobmff::CodecConfig::Unknown { fourcc, raw: Vec::new() }, 0, 0)
    }
    fn timescale(&self) -> u32 {
        u32::try_from(self.rate.num).unwrap_or(24)
    }
    fn encode(&mut self, f: &EncoderFrame<'_>) -> Result<Vec<EncodedPacket>> {
        self.pending.push((f.width, f.height, f.rgba.to_vec()));
        if self.pending.len() >= self.batch { self.drain() } else { Ok(Vec::new()) }
    }
    fn flush(&mut self) -> Result<Vec<EncodedPacket>> {
        self.drain()
    }
}

pub(crate) fn imf_factory(format: Format, w: u32, h: u32, rate: FrameRate, s: &ExportSettings) -> Option<Result<Box<dyn VideoEncoder>>> {
    if format != Format::Imf {
        return None;
    }
    let Some(profile) = imf_profile(w, h, rate, s.imf_lossless) else {
        return Some(Err(ExportError::Unsupported(format!(
            "an IMF App 2E package has no {w}x{h} pictures at {} fps (at most {}x{})",
            rate.label(),
            IMF_MAX_SIZE.0,
            IMF_MAX_SIZE.1
        ))));
    };
    let (num, den) = (u32::try_from(rate.num).unwrap_or(24), u32::try_from(rate.den).unwrap_or(1));
    let opts = cfa4_j2k::EncodeOptions::imf(profile, w, h, num, den);
    // a batch per core, fewer for big pictures (each frame holds its coefficients while coded)
    let cap = if u64::from(w) * u64::from(h) > 2048 * 1556 { 4 } else { 8 };
    let batch = rayon::current_num_threads().clamp(1, cap);
    Some(Ok(Box::new(ImfEncoder { opts, rate, pending: Vec::new(), batch }) as Box<dyn VideoEncoder>))
}

// ---------------------------------------------------------------------------------------------
// MCA labels (ST 2067-8, ST 428-12)

const fn mca_dict(facet: u8, designator: u8, item: u8, number: u8) -> [u8; 16] {
    [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x0D, 0x03, 0x02, facet, designator, item, number, 0, 0]
}

/// The MCA labels of an IMF audio track file of `channels` channels: the soundfield group (with
/// its ST 2067-2 titles) and one label per channel. Mono is a Discrete Numbered Sources group of
/// one numbered source channel (ST 2067-8); stereo the Standard Stereo group (ST 2067-8); 5.1 the
/// ST 428-12 group. An empty `language` is tagged "und". `None` for other layouts.
pub(crate) fn imf_mca(channels: u32, title: &str, language: &str, id: &dyn Fn(&str) -> [u8; 16]) -> Option<Vec<McaLabel>> {
    type Channel = ([u8; 16], &'static str, &'static str);
    let (group, (sym, name), chans): ([u8; 16], (&str, &str), Vec<Channel>) = match channels {
        1 => (
            mca_dict(0x02, 0x20, 0x03, 0),
            ("sgDNS", "Discrete Numbered Sources"),
            vec![(mca_dict(0x01, 0x20, 0x08, 0x01), "chNSC001", "Numbered Source Channel 001")],
        ),
        2 => (
            mca_dict(0x02, 0x20, 0x01, 0),
            ("sgST", "Standard Stereo"),
            vec![(mca_dict(0x01, 0x01, 0, 0), "chL", "Left"), (mca_dict(0x01, 0x02, 0, 0), "chR", "Right")],
        ),
        6 => (
            mca_dict(0x02, 0x01, 0, 0),
            ("sg51", "5.1"),
            vec![
                (mca_dict(0x01, 0x01, 0, 0), "chL", "Left"),
                (mca_dict(0x01, 0x02, 0, 0), "chR", "Right"),
                (mca_dict(0x01, 0x03, 0, 0), "chC", "Center"),
                (mca_dict(0x01, 0x04, 0, 0), "chLFE", "LFE"),
                (mca_dict(0x01, 0x05, 0, 0), "chLs", "Left Surround"),
                (mca_dict(0x01, 0x06, 0, 0), "chRs", "Right Surround"),
            ],
        ),
        _ => return None,
    };
    // ST 2067-2 5.3.6.5: the soundfield group carries the primary spoken language; when the user
    // set none it is RFC 5646 "und" (undetermined), so validators find a tag
    let lang = Some(if language.is_empty() { "und".to_string() } else { language.to_string() });
    let link = id(sym);
    let titles = McaTitles {
        title: if title.trim().is_empty() { "Untitled".into() } else { title.to_string() },
        title_version: "1".into(),
        // ST 377-41: the primary programme, a full composite mix
        content_kind: "PRM".into(),
        element_kind: "FCMP".into(),
    };
    let mut v = vec![McaLabel {
        dictionary: group,
        link,
        symbol: sym.into(),
        name: name.into(),
        channel: None,
        language: lang.clone(),
        group: None,
        titles: Some(titles),
    }];
    for (k, (dict, symbol, name)) in chans.into_iter().enumerate() {
        v.push(McaLabel {
            dictionary: dict,
            link: id(symbol),
            symbol: symbol.into(),
            name: name.into(),
            channel: Some(k as u32 + 1),
            language: lang.clone(),
            group: Some(link),
            titles: None,
        });
    }
    Some(v)
}

// ---------------------------------------------------------------------------------------------
// The package

/// Bytes read from the start of a track file for its essence descriptor (the header partition).
const HEADER_PREFIX: usize = 4 << 20;

/// One file of the package.
struct Asset {
    id: [u8; 16],
    name: String,
    size: u64,
    hash: [u8; 20],
    mime: &'static str,
}

/// A finished track file: the asset, its essence descriptor (RegXML) and edit units.
struct Track {
    asset: Asset,
    descriptor: String,
    units: u64,
}

/// Finish a track file: (size, SHA-1, the first [`HEADER_PREFIX`] bytes).
fn finish_track(out: Out, settings: &ExportSettings, path: &str) -> Result<(u64, [u8; 20], Vec<u8>)> {
    match out {
        Out::Mem(c) => {
            let data = c.into_inner();
            let hash = sha1(&data);
            let prefix = data.get(..data.len().min(HEADER_PREFIX)).unwrap_or_default().to_vec();
            Ok((crate::write_output(settings, path, data)?, hash, prefix))
        }
        Out::File(mut w) => {
            w.flush().map_err(io)?;
            drop(w);
            let mut f = std::fs::File::open(path).map_err(|e| io(format!("{path}: {e}")))?;
            let mut s = Sha1::new();
            let mut buf = vec![0u8; 1 << 20];
            let mut prefix = Vec::new();
            let mut n = 0u64;
            loop {
                let k = f.read(&mut buf).map_err(io)?;
                if k == 0 {
                    break;
                }
                let chunk = buf.get(..k).unwrap_or_default();
                s.update(chunk);
                if prefix.len() < HEADER_PREFIX {
                    prefix.extend_from_slice(chunk.get(..k.min(HEADER_PREFIX - prefix.len())).unwrap_or_default());
                }
                n += k as u64;
            }
            Ok((n, s.finish(), prefix))
        }
    }
}

/// Everything the IMF muxer needs from the export.
pub(crate) struct ImfSetup<'a> {
    pub settings: &'a ExportSettings,
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub rate: FrameRate,
    /// Audio channels (1, 2 or 6), 0 without sound.
    pub channels: u32,
}

/// An IMF export in progress: the track files are written as the export runs; the XML at the end.
pub(crate) struct ImfMux {
    dir: String,
    seed: String,
    title: String,
    kind: &'static str,
    rate: FrameRate,
    size: (u32, u32),
    language: String,
    issued: String,
    modified: Timestamp,
    picture_id: [u8; 16],
    /// Created with the first codestream (the JPEG 2000 sub-descriptor repeats its SIZ / COD).
    picture: Option<(String, MxfWriter<Out>)>,
    sound: Option<([u8; 16], String, MxfWriter<Out>, SoundDesc)>,
    frames: u64,
    samples: u64,
}

impl ImfMux {
    pub fn new(s: ImfSetup<'_>) -> Result<Self> {
        let settings = s.settings;
        let dir = settings.path.clone();
        if settings.sink.is_none() {
            std::fs::create_dir_all(&dir).map_err(|e| io(format!("{dir}: {e}")))?;
        }
        let now = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let seed = format!("{dir}|{now}|imf");
        let t = Timestamp::from_unix(i64::try_from(now / 1_000_000_000).unwrap_or(0));
        let issued = format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}+00:00", t.year, t.month, t.day, t.hour, t.minute, t.second);
        let language = crate::dcp_language(settings).unwrap_or_default().to_string();
        let mut me = ImfMux {
            dir,
            seed,
            title: s.title,
            kind: imf_content_kind(settings).unwrap_or("feature"),
            rate: s.rate,
            size: (s.width, s.height),
            language,
            issued,
            modified: t,
            picture_id: [0; 16],
            picture: None,
            sound: None,
            frames: 0,
            samples: 0,
        };
        me.picture_id = uuid(&format!("{}|picture", me.seed));
        if s.channels > 0 {
            let id = uuid(&format!("{}|sound", me.seed));
            let mut c = me.config("sound", id, Rational::new(48_000, 1));
            let desc = SoundDesc { sample_rate: 48_000, channels: s.channels, bits: 24 };
            c.sound = vec![desc];
            c.reference_image_edit_rate = Some(me.edit_rate());
            let seed = me.seed.clone();
            let label_id = |sym: &str| uuid(&format!("{seed}|mca|{sym}"));
            c.mca = imf_mca(s.channels, &me.title, &me.language, &label_id)
                .ok_or_else(|| ExportError::Unsupported(format!("an IMF audio track has no {}-channel layout here: choose mono, stereo or 5.1", s.channels)))?;
            let name = format!("AUDIO_{}.mxf", uuid_text(&id));
            let w = MxfWriter::new(Out::create_path(settings, &join(&me.dir, &name))?, c).map_err(io)?;
            me.sound = Some((id, name, w, desc));
        }
        Ok(me)
    }

    fn edit_rate(&self) -> Rational {
        Rational::new(i32::try_from(self.rate.num).unwrap_or(24), i32::try_from(self.rate.den).unwrap_or(1))
    }

    fn config(&self, role: &str, id: [u8; 16], rate: Rational) -> WriterConfig {
        let material = Umid::dcinema(uuid(&format!("{}|material|{role}", self.seed)));
        let ids = PackageIds { material, file: Umid::dcinema(id), material_name: self.title.clone(), file_name: format!("{} {role}", self.title) };
        let mut c = WriterConfig::new(Pattern::Imf, rate, ids);
        let tc = self.edit_rate();
        c.timecode = Some(StartTimecode { frames: 0, rate: tc, drop_frame: false });
        c.modified = self.modified;
        c
    }

    fn open_picture(&mut self, settings: &ExportSettings, first: &[u8]) -> Result<()> {
        let j2k = J2kSubDescriptor::from_codestream(first).ok_or_else(|| ExportError::Encode("JPEG 2000: the codestream has no SIZ / COD".into()))?;
        let mut d = PictureDesc::new(PictureCoding::J2k { rsiz: j2k.rsiz }, self.size.0, self.size.1);
        d.depth = 10;
        d.rgb = true;
        d.full_range = true;
        d.j2k = Some(j2k);
        let mut c = self.config("picture", self.picture_id, self.edit_rate());
        c.picture = Some(d);
        let name = format!("VIDEO_{}.mxf", uuid_text(&self.picture_id));
        let w = MxfWriter::new(Out::create_path(settings, &join(&self.dir, &name))?, c).map_err(io)?;
        self.picture = Some((name, w));
        Ok(())
    }

    pub fn write_video(&mut self, settings: &ExportSettings, packets: Vec<EncodedPacket>) -> Result<()> {
        for p in packets {
            if self.picture.is_none() {
                self.open_picture(settings, &p.data)?;
            }
            let (_, w) = self.picture.as_mut().ok_or_else(|| ExportError::Encode("internal: the IMF picture file is closed".into()))?;
            w.push_picture(&p.data, FrameInfo::intra()).map_err(io)?;
            self.frames += 1;
        }
        Ok(())
    }

    pub fn write_audio(&mut self, planar: &[Vec<f32>]) -> Result<()> {
        let Some((_, _, w, _)) = self.sound.as_mut() else { return Ok(()) };
        let n = planar.first().map_or(0, Vec::len);
        if n == 0 {
            return Ok(());
        }
        w.push_sound(0, &cfa4_mxf::encode_pcm(planar, 24)).map_err(io)?;
        self.samples += n as u64;
        Ok(())
    }

    /// Picture frames the composition plays: all of them, or (ST 2067-21 8.3.2) a multiple of 5
    /// when a frame does not hold a whole number of 48 kHz samples.
    fn played_frames(&self) -> u64 {
        let (num, den) = (self.rate.num.max(1) as u64, self.rate.den.max(1) as u64);
        if (48_000 * den) % num == 0 { self.frames } else { self.frames - self.frames % 5 }
    }

    /// 48 kHz samples of `frames` picture frames.
    fn samples_of(&self, frames: u64) -> u64 {
        let (num, den) = (self.rate.num.max(1) as u128, self.rate.den.max(1) as u128);
        u64::try_from(u128::from(frames) * 48_000 * den / num).unwrap_or(u64::MAX)
    }

    /// Finish the track files and write the composition playlist, packing list, asset map and
    /// volume index; returns (total bytes, every file written).
    pub fn finish(mut self, settings: &ExportSettings) -> Result<(u64, Vec<String>)> {
        let played = self.played_frames();
        if played == 0 {
            return Err(ExportError::Unsupported(format!("an IMF composition at {} fps needs at least 5 frames", self.rate.label())));
        }
        // the sound covers the played pictures: silence after a short mix
        let need = self.samples_of(played);
        if let Some((_, _, w, desc)) = self.sound.as_mut()
            && self.samples < need
        {
            let block = desc.block_align() as usize;
            let mut left = need - self.samples;
            while left > 0 {
                let n = left.min(48_000);
                w.push_sound(0, &vec![0u8; n as usize * block]).map_err(io)?;
                left -= n;
            }
            self.samples = need;
        }
        let dir = self.dir.clone();
        let mut files = Vec::new();
        let mut total = 0u64;
        let mut track = |id: [u8; 16], name: String, w: MxfWriter<Out>, units: u64| -> Result<Track> {
            let path = join(&dir, &name);
            let (size, hash, prefix) = finish_track(w.finish().map_err(io)?, settings, &path)?;
            total += size;
            files.push(path);
            let descriptor = cfa4_mxf::regxml::essence_descriptor(&prefix).map_err(|e| ExportError::Encode(format!("{name}: essence descriptor: {e}")))?;
            Ok(Track { asset: Asset { id, name, size, hash, mime: "application/mxf" }, descriptor, units })
        };
        let (pname, pw) = self.picture.take().ok_or_else(|| ExportError::Encode("internal: the IMF picture file was never opened".into()))?;
        let picture = track(self.picture_id, pname, pw, self.frames)?;
        let sound = match self.sound.take() {
            Some((id, name, w, _)) => Some(track(id, name, w, self.samples)?),
            None => None,
        };
        let mut write = |name: String, xml: String, id: [u8; 16]| -> Result<Asset> {
            let path = join(&dir, &name);
            let data = xml.into_bytes();
            let hash = sha1(&data);
            let size = crate::write_output(settings, &path, data)?;
            total += size;
            files.push(path);
            Ok(Asset { id, name, size, hash, mime: "text/xml" })
        };
        let cpl_id = uuid(&format!("{}|cpl", self.seed));
        let cpl = write(format!("CPL_{}.xml", uuid_text(&cpl_id)), self.cpl(cpl_id, &picture, sound.as_ref(), played), cpl_id)?;
        let pkl_id = uuid(&format!("{}|pkl", self.seed));
        let mut assets = vec![&cpl, &picture.asset];
        assets.extend(sound.as_ref().map(|t| &t.asset));
        let pkl = write(format!("PKL_{}.xml", uuid_text(&pkl_id)), self.pkl(pkl_id, &assets), pkl_id)?;
        assets.insert(0, &pkl);
        let am_id = uuid(&format!("{}|assetmap", self.seed));
        let am = self.assetmap(am_id, &assets, &pkl);
        write("ASSETMAP.xml".into(), am, am_id)?;
        let vol = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<VolumeIndex xmlns=\"http://www.smpte-ra.org/schemas/429-9/2007/AM\">\n  <Index>1</Index>\n</VolumeIndex>\n";
        write("VOLINDEX.xml".into(), vol.into(), am_id)?;
        Ok((total, files))
    }

    fn header(&self, x: &mut String, id: [u8; 16], annotation_tag: &str) {
        x.push_str(&format!("  <Id>urn:uuid:{}</Id>\n", uuid_text(&id)));
        x.push_str(&format!("  <{annotation_tag}>{}</{annotation_tag}>\n", xml_text(&self.title)));
        x.push_str(&format!("  <IssueDate>{}</IssueDate>\n", self.issued));
        x.push_str("  <Issuer>CFA4</Issuer>\n");
        x.push_str(&format!("  <Creator>CFA4 {}</Creator>\n", env!("CARGO_PKG_VERSION")));
    }

    /// Composition playlist (ST 2067-3:2016 schema, ST 2067-2:2020 core constraints): one
    /// segment with the main image and main audio sequences.
    fn cpl(&self, id: [u8; 16], picture: &Track, sound: Option<&Track>, played: u64) -> String {
        let rate = format!("{} {}", self.rate.num, self.rate.den);
        let mut x = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<CompositionPlaylist xmlns=\"http://www.smpte-ra.org/schemas/2067-3/2016\" xmlns:cc=\"http://www.smpte-ra.org/ns/2067-2/2020\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n",
        );
        self.header(&mut x, id, "Annotation");
        x.push_str(&format!("  <ContentTitle>{}</ContentTitle>\n", xml_text(&self.title)));
        x.push_str(&format!("  <ContentKind>{}</ContentKind>\n", self.kind));
        x.push_str(&format!(
            "  <ContentVersionList>\n    <ContentVersion>\n      <Id>urn:uuid:{}</Id>\n      <LabelText>{}</LabelText>\n    </ContentVersion>\n  </ContentVersionList>\n",
            uuid_text(&uuid(&format!("{}|version", self.seed))),
            xml_text(&self.title)
        ));
        let mut tracks = vec![picture];
        tracks.extend(sound);
        let encoding = |t: &Track| uuid(&format!("{}|encoding|{}", self.seed, uuid_text(&t.asset.id)));
        x.push_str("  <EssenceDescriptorList>\n");
        for t in &tracks {
            x.push_str(&format!("    <EssenceDescriptor>\n      <Id>urn:uuid:{}</Id>\n      ", uuid_text(&encoding(t))));
            x.push_str(t.descriptor.trim_end());
            x.push_str("\n    </EssenceDescriptor>\n");
        }
        x.push_str("  </EssenceDescriptorList>\n");
        let tc_rate = (self.rate.num.max(1) + self.rate.den.max(1) - 1) / self.rate.den.max(1);
        x.push_str(&format!(
            "  <CompositionTimecode>\n    <TimecodeDropFrame>false</TimecodeDropFrame>\n    <TimecodeRate>{tc_rate}</TimecodeRate>\n    <TimecodeStartAddress>00:00:00:00</TimecodeStartAddress>\n  </CompositionTimecode>\n"
        ));
        x.push_str(&format!("  <EditRate>{rate}</EditRate>\n"));
        let secs = u128::from(played) * self.rate.den.max(1) as u128 / self.rate.num.max(1) as u128;
        x.push_str(&format!("  <TotalRunningTime>{:02}:{:02}:{:02}</TotalRunningTime>\n", (secs / 3600).min(99), secs / 60 % 60, secs % 60));
        if !self.language.is_empty() {
            x.push_str(&format!(
                "  <LocaleList>\n    <Locale>\n      <LanguageList>\n        <Language>{}</Language>\n      </LanguageList>\n    </Locale>\n  </LocaleList>\n",
                xml_text(&self.language)
            ));
        }
        x.push_str(&format!(
            "  <ExtensionProperties>\n    <cc:ApplicationIdentification>{APPLICATION_ID}</cc:ApplicationIdentification>\n  </ExtensionProperties>\n"
        ));
        x.push_str("  <SegmentList>\n    <Segment>\n");
        x.push_str(&format!("      <Id>urn:uuid:{}</Id>\n      <SequenceList>\n", uuid_text(&uuid(&format!("{}|segment1", self.seed)))));
        let sequence = |x: &mut String, kind: &str, t: &Track, edit_rate: String, duration: u64| {
            let role = format!("{}|{kind}", self.seed);
            x.push_str(&format!("        <cc:{kind}>\n"));
            x.push_str(&format!("          <Id>urn:uuid:{}</Id>\n", uuid_text(&uuid(&format!("{role}|sequence")))));
            x.push_str(&format!("          <TrackId>urn:uuid:{}</TrackId>\n", uuid_text(&uuid(&format!("{role}|track")))));
            x.push_str("          <ResourceList>\n            <Resource xsi:type=\"TrackFileResourceType\">\n");
            x.push_str(&format!("              <Id>urn:uuid:{}</Id>\n", uuid_text(&uuid(&format!("{role}|resource")))));
            x.push_str(&format!("              <EditRate>{edit_rate}</EditRate>\n"));
            x.push_str(&format!("              <IntrinsicDuration>{}</IntrinsicDuration>\n", t.units));
            x.push_str("              <EntryPoint>0</EntryPoint>\n");
            x.push_str(&format!("              <SourceDuration>{duration}</SourceDuration>\n"));
            x.push_str(&format!("              <SourceEncoding>urn:uuid:{}</SourceEncoding>\n", uuid_text(&encoding(t))));
            x.push_str(&format!("              <TrackFileId>urn:uuid:{}</TrackFileId>\n", uuid_text(&t.asset.id)));
            x.push_str(&format!("              <Hash>{}</Hash>\n", base64(&t.asset.hash)));
            x.push_str("              <HashAlgorithm Algorithm=\"http://www.w3.org/2000/09/xmldsig#sha1\"/>\n");
            x.push_str(&format!("            </Resource>\n          </ResourceList>\n        </cc:{kind}>\n"));
        };
        sequence(&mut x, "MainImageSequence", picture, rate.clone(), played);
        if let Some(t) = sound {
            sequence(&mut x, "MainAudioSequence", t, "48000 1".into(), self.samples_of(played));
        }
        x.push_str("      </SequenceList>\n    </Segment>\n  </SegmentList>\n</CompositionPlaylist>\n");
        x
    }

    /// Packing list (ST 2067-2 Annex J).
    fn pkl(&self, id: [u8; 16], assets: &[&Asset]) -> String {
        let mut x = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<PackingList xmlns=\"http://www.smpte-ra.org/schemas/2067-2/2016/PKL\">\n");
        self.header(&mut x, id, "AnnotationText");
        x.push_str("  <AssetList>\n");
        for a in assets {
            x.push_str("    <Asset>\n");
            x.push_str(&format!("      <Id>urn:uuid:{}</Id>\n", uuid_text(&a.id)));
            x.push_str(&format!("      <AnnotationText>{}</AnnotationText>\n", xml_text(&a.name)));
            x.push_str(&format!("      <Hash>{}</Hash>\n", base64(&a.hash)));
            x.push_str(&format!("      <Size>{}</Size>\n", a.size));
            x.push_str(&format!("      <Type>{}</Type>\n", a.mime));
            x.push_str(&format!("      <OriginalFileName>{}</OriginalFileName>\n", xml_text(&a.name)));
            x.push_str("      <HashAlgorithm Algorithm=\"http://www.w3.org/2000/09/xmldsig#sha1\"/>\n");
            x.push_str("    </Asset>\n");
        }
        x.push_str("  </AssetList>\n</PackingList>\n");
        x
    }

    /// Asset map (ST 429-9): every asset, the packing list first.
    fn assetmap(&self, id: [u8; 16], assets: &[&Asset], pkl: &Asset) -> String {
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
