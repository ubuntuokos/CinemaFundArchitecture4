//! The complete stream list of a media file (Media Cutter K13): every stream the container
//! carries, video, audio, subtitle, data (timecode, chapter text, metadata) and attachment, with
//! its codec, language, channels, size and id, plus the file's chapters.
//!
//! Names follow FFmpeg's vocabulary (`codec_type`, `codec_name`, stream id), so the list can be
//! checked against `ffprobe` (`tests/streams_oracle.rs`); where FFmpeg has no codec name for a
//! stream (a QuickTime timecode track) the CFA4 name is given and `codec_tag` carries the FourCC.
//!
//! Reading the list never decodes media. For MP4 / MOV, Matroska, MXF, AVI, WAV, FLAC and Ogg
//! only container headers and codec configuration are read (Ogg: its beginning-of-stream pages).
//! MPEG TS / PS have no header that lists the streams with their properties: the demuxer
//! (`cfa4_mpegts::open`) walks the packets of the whole file to find every stream and its access
//! units, and the list then reads the first access unit of each stream for its parameters.

use std::io;

use cfa4_media::ByteReader;
use cfa4_time::Tick;
use serde_json::{Value, json};

/// What a stream carries (FFmpeg's `codec_type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamType {
    Video,
    Audio,
    Subtitle,
    Data,
    Attachment,
}

impl StreamType {
    pub fn name(self) -> &'static str {
        match self {
            StreamType::Video => "video",
            StreamType::Audio => "audio",
            StreamType::Subtitle => "subtitle",
            StreamType::Data => "data",
            StreamType::Attachment => "attachment",
        }
    }
}

/// One stream of the file.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamEntry {
    /// Position in the list (file order; FFmpeg's stream index).
    pub index: usize,
    /// The container's own id of the stream: MP4 track id and MPEG TS PID as `0x…` (as ffprobe
    /// prints them), MPEG PS start code (`0x1e0`) or private stream 1 sub-stream, Matroska track
    /// number, Ogg serial, MXF track id. `None` when the container has none (AVI, attachments).
    pub id: Option<String>,
    pub kind: StreamType,
    /// Codec name (FFmpeg's `codec_name` where FFmpeg has one). `None` when unknown.
    pub codec: Option<String>,
    /// The container's codec FourCC, when it has one (`avc1`, `tmcd`, `MJPG`).
    pub codec_tag: Option<String>,
    /// ISO 639-2 language; `None` when the file gives none or `und`.
    pub language: Option<String>,
    pub title: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub channels: Option<u32>,
    pub sample_rate: Option<u32>,
    pub default: Option<bool>,
    pub forced: Option<bool>,
    /// A timecode track's start (`HH:MM:SS:FF`).
    pub timecode: Option<String>,
    /// An attachment's file name, media type and size in bytes.
    pub filename: Option<String>,
    pub mimetype: Option<String>,
    pub size: Option<u64>,
}

impl StreamEntry {
    fn new(kind: StreamType) -> StreamEntry {
        StreamEntry {
            index: 0,
            id: None,
            kind,
            codec: None,
            codec_tag: None,
            language: None,
            title: None,
            width: None,
            height: None,
            channels: None,
            sample_rate: None,
            default: None,
            forced: None,
            timecode: None,
            filename: None,
            mimetype: None,
            size: None,
        }
    }

    fn codec(mut self, c: impl Into<String>) -> StreamEntry {
        let c = c.into();
        self.codec = (!c.is_empty()).then_some(c);
        self
    }

    fn size(mut self, w: u32, h: u32) -> StreamEntry {
        if w > 0 && h > 0 {
            self.width = Some(w);
            self.height = Some(h);
        }
        self
    }

    fn audio(mut self, channels: u32, sample_rate: u32) -> StreamEntry {
        self.channels = (channels > 0).then_some(channels);
        self.sample_rate = (sample_rate > 0).then_some(sample_rate);
        self
    }

    pub fn to_json(&self) -> Value {
        let mut v = json!({"index": self.index, "type": self.kind.name(), "codec": self.codec, "id": self.id});
        let mut put = |k: &str, x: Value| {
            if !x.is_null() {
                v[k] = x;
            }
        };
        put("codecTag", json!(self.codec_tag));
        put("language", json!(self.language));
        put("title", json!(self.title));
        put("width", json!(self.width));
        put("height", json!(self.height));
        put("channels", json!(self.channels));
        put("sampleRate", json!(self.sample_rate));
        put("default", json!(self.default));
        put("forced", json!(self.forced));
        put("timecode", json!(self.timecode));
        put("filename", json!(self.filename));
        put("mimetype", json!(self.mimetype));
        put("size", json!(self.size));
        v
    }
}

/// A chapter mark: start and (when known) end on the file's timeline, and its title.
#[derive(Clone, Debug, PartialEq)]
pub struct ChapterEntry {
    pub start: Tick,
    pub end: Option<Tick>,
    pub title: String,
}

/// The streams and chapters of a file.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct StreamList {
    /// The container (`mp4`, `mov`, `matroska`, `webm`, `mpegts`, `mpegps`, `mxf`, `ogg`, `avi`,
    /// `wav`, `flac`).
    pub container: String,
    pub streams: Vec<StreamEntry>,
    pub chapters: Vec<ChapterEntry>,
    pub warnings: Vec<String>,
}

impl StreamList {
    pub fn to_json(&self) -> Value {
        json!({
            "container": self.container,
            "streams": self.streams.iter().map(StreamEntry::to_json).collect::<Vec<_>>(),
            "chapters": self.chapters.iter().map(|c| json!({"start": c.start.0, "end": c.end.map(|t| t.0), "title": c.title})).collect::<Vec<_>>(),
            "warnings": self.warnings,
        })
    }

    fn push(&mut self, mut s: StreamEntry) {
        s.index = self.streams.len();
        self.streams.push(s);
    }
}

/// The list of a stream-less file described only by its [`cfa4_media::MediaInfo`] (an elementary
/// audio or video file, a still): its video stream and every audio stream.
pub fn from_media_info(info: &cfa4_media::MediaInfo) -> StreamList {
    let mut l = StreamList { container: info.container.clone(), ..StreamList::default() };
    if let Some(v) = &info.video {
        l.push(StreamEntry::new(StreamType::Video).codec(v.codec.to_ascii_lowercase()).size(v.width, v.height));
    }
    for a in &info.audio_streams {
        l.push(StreamEntry::new(StreamType::Audio).codec(a.codec.to_ascii_lowercase()).audio(a.channels, a.sample_rate));
    }
    l
}

/// Every container crate has its own `ByteSource`; one adapter serves them all.
struct Src<'a>(&'a dyn ByteReader);

macro_rules! byte_source {
    ($($t:path),*) => {$(
        impl $t for Src<'_> {
            fn len(&self) -> u64 {
                self.0.len()
            }
            fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
                self.0.read_at(offset, buf)
            }
        }
    )*};
}
byte_source!(cfa4_isobmff::ByteSource, cfa4_matroska::ByteSource, cfa4_mpegts::ByteSource, cfa4_mxf::ByteSource, cfa4_avi::ByteSource);

/// Bytes read from the start of the file to recognise its container.
const HEAD: u64 = 70_000;

/// The stream list of the file `src` holds, or `None` when its container is not one listed here
/// (callers fall back to [`from_media_info`]). An error is a recognised container that fails to
/// parse.
pub fn stream_list(src: &dyn ByteReader) -> Option<Result<StreamList, String>> {
    let mut head = vec![0; src.len().min(HEAD) as usize];
    if let Err(e) = src.read_at(0, &mut head) {
        return Some(Err(e.to_string()));
    }
    let s = Src(src);
    if cfa4_mxf::sniff(&head) {
        return Some(cfa4_mxf::open(&s).map(|f| mxf(&f)).map_err(|e| e.to_string()));
    }
    if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some(cfa4_matroska::open(&s).map(|f| matroska(&f)).map_err(|e| e.to_string()));
    }
    if is_isobmff(&head) {
        return Some(cfa4_isobmff::open(&s).map(|f| mp4(&f)).map_err(|e| e.to_string()));
    }
    if cfa4_avi::sniff(&head) {
        return Some(cfa4_avi::open(&s).map(|f| avi(&f)).map_err(|e| e.to_string()));
    }
    if cfa4_ogg::sniff(&head) {
        return Some(ogg(src));
    }
    if cfa4_mpegts::sniff(&head).is_some() {
        return Some(cfa4_mpegts::open(&s).map(|f| mpeg(&f, &s)).map_err(|e| e.to_string()));
    }
    if head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WAVE") {
        return wav(src);
    }
    if head.starts_with(b"fLaC") {
        return Some(flac(&head));
    }
    None
}

fn is_isobmff(head: &[u8]) -> bool {
    matches!(head.get(4..8), Some(b"ftyp" | b"moov" | b"mdat" | b"free" | b"wide" | b"skip" | b"pnot"))
}

fn fourcc(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn language(l: &str) -> Option<String> {
    let l = l.trim_matches(char::from(0)).trim();
    (!l.is_empty() && l != "und").then(|| l.to_string())
}

fn some_text(t: &str) -> Option<String> {
    let t = t.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn ticks(value: u64, per_second: u64) -> Tick {
    let t = i128::from(value) * i128::from(cfa4_time::TICKS_PER_SECOND) / i128::from(per_second.max(1));
    Tick(i64::try_from(t).unwrap_or(i64::MAX))
}

/// Chapter ends: each chapter ends where the next begins, the last at `end`.
fn close_chapters(chapters: &mut [ChapterEntry], end: Option<Tick>) {
    for i in 0..chapters.len() {
        if chapters[i].end.is_none() {
            chapters[i].end = chapters.get(i + 1).map(|c| c.start).or(end).filter(|&e| e >= chapters[i].start);
        }
    }
}

// ---------------------------------------------------------------- MP4 / MOV

fn mp4(f: &cfa4_isobmff::Mp4File) -> StreamList {
    use cfa4_isobmff::{CodecConfig, TrackKind};
    let mut l = StreamList { container: if f.is_quicktime { "mov" } else { "mp4" }.into(), ..StreamList::default() };
    for t in &f.tracks {
        let entry = t.entries.first();
        let tag = entry.map(|e| fourcc(&e.format.0));
        let codec = t.codec();
        let kind = match t.kind {
            TrackKind::Video => StreamType::Video,
            TrackKind::Audio => StreamType::Audio,
            TrackKind::Subtitle => StreamType::Subtitle,
            TrackKind::Timecode | TrackKind::Other => StreamType::Data,
        };
        let name = match codec {
            Some(CodecConfig::Pcm(p)) => pcm_name(p.bits, p.float, p.big_endian, p.signed),
            Some(CodecConfig::Unknown { fourcc: fc, .. }) => mp4_fourcc_codec(kind, &fc.0).to_string(),
            Some(CodecConfig::Timecode(_)) => "timecode".into(),
            Some(c) => c.name().to_string(),
            None => String::new(),
        };
        let mut s = StreamEntry::new(kind).codec(name);
        s.id = Some(format!("0x{:x}", t.id));
        s.codec_tag = tag;
        s.language = language(&t.language);
        if let Some(v) = t.video() {
            s = s.size(u32::from(v.width), u32::from(v.height));
        }
        if let Some(a) = t.audio() {
            let (mut ch, mut rate) = (a.channels, a.sample_rate as u32);
            match codec {
                Some(CodecConfig::Aac(c)) if c.channel_config > 0 && c.channel_config < 8 => {
                    ch = [0, 1, 2, 3, 4, 5, 6, 8][usize::from(c.channel_config)];
                }
                Some(CodecConfig::Pcm(p)) => (ch, rate) = (p.channels, p.sample_rate as u32),
                Some(CodecConfig::Flac(c)) => (ch, rate) = (u32::from(c.channels), c.sample_rate),
                Some(CodecConfig::Opus(c)) => (ch, rate) = (u32::from(c.output_channels), 48_000),
                _ => {}
            }
            s = s.audio(ch, rate);
        }
        if let Some(CodecConfig::Timecode(tc)) = codec {
            s.timecode = tc.start_frame.map(|f| tc.format_frame(f));
        }
        s.default = Some(t.enabled);
        l.push(s);
    }
    let end = (f.timescale > 0).then(|| ticks(f.duration, u64::from(f.timescale)));
    l.chapters = f.metadata.chapters.iter().map(|c| ChapterEntry { start: ticks(c.start_100ns, 10_000_000), end: None, title: c.title.clone() }).collect();
    close_chapters(&mut l.chapters, end);
    l
}

/// FFmpeg's codec of an MP4 sample entry CFA4 keeps opaque.
fn mp4_fourcc_codec(kind: StreamType, fc: &[u8; 4]) -> &'static str {
    match fc {
        b"tx3g" | b"text" => {
            if kind == StreamType::Subtitle {
                "mov_text"
            } else {
                "bin_data"
            }
        }
        b"wvtt" => "webvtt",
        b"stpp" => "ttml",
        b"c608" => "eia_608",
        b"mp4s" => "dvd_subtitle",
        b"mp4v" => "mpeg4",
        b"mp2v" | b"xdvc" | b"xdv2" | b"xd5c" | b"mx5p" | b"mx5n" => "mpeg2video",
        b"vp08" => "vp8",
        b"hev1" | b"hvc1" => "hevc",
        b"ac-4" => "ac4",
        b"ec+3" => "eac3",
        b"samr" => "amr_nb",
        b"sawb" => "amr_wb",
        b"ima4" => "adpcm_ima_qt",
        b"ulaw" => "pcm_mulaw",
        b"alaw" => "pcm_alaw",
        _ => "",
    }
}

fn pcm_name(bits: u16, float: bool, big_endian: bool, signed: bool) -> String {
    let e = if big_endian { "be" } else { "le" };
    match (float, bits, signed) {
        (true, b, _) => format!("pcm_f{b}{e}"),
        (false, 8, false) => "pcm_u8".into(),
        (false, 8, true) => "pcm_s8".into(),
        (false, b, true) => format!("pcm_s{b}{e}"),
        (false, b, false) => format!("pcm_u{b}{e}"),
    }
}

// ---------------------------------------------------------------- Matroska / WebM

fn matroska(f: &cfa4_matroska::MkvFile) -> StreamList {
    use cfa4_matroska::TrackKind;
    let mut l = StreamList { container: if f.doc_type == "webm" { "webm" } else { "matroska" }.into(), ..StreamList::default() };
    for t in &f.tracks {
        let kind = match t.kind {
            TrackKind::Video => StreamType::Video,
            TrackKind::Audio => StreamType::Audio,
            TrackKind::Subtitle => StreamType::Subtitle,
            _ => StreamType::Data,
        };
        let mut s = StreamEntry::new(kind).codec(t.codec.name());
        s.id = Some(t.number.to_string());
        s.codec_tag = some_text(&t.codec_id);
        s.language = language(&t.language);
        s.title = some_text(&t.name);
        if let Some(v) = &t.video {
            s = s.size(v.pixel_width, v.pixel_height);
        }
        if let Some(a) = &t.audio {
            let rate = a.output_sampling_frequency.unwrap_or(a.sampling_frequency);
            s = s.audio(u32::try_from(a.channels).unwrap_or(0), rate as u32);
        }
        s.default = Some(t.default);
        s.forced = Some(t.forced);
        l.push(s);
    }
    for a in &f.attachments {
        let mut s = StreamEntry::new(StreamType::Attachment).codec(attachment_codec(&a.media_type, &a.name));
        s.filename = some_text(&a.name);
        s.mimetype = some_text(&a.media_type);
        s.title = a.description.as_deref().and_then(some_text);
        s.size = Some(a.data_size);
        l.push(s);
    }
    // the default edition (else the first): its top-level chapters, as players show them
    let edition = f.chapters.iter().find(|e| e.default).or_else(|| f.chapters.first());
    if let Some(e) = edition {
        l.chapters = e
            .chapters
            .iter()
            .map(|c| ChapterEntry {
                start: ticks(c.start_ns, 1_000_000_000),
                end: c.end_ns.map(|e| ticks(e, 1_000_000_000)),
                title: c.displays.first().map(|d| d.string.clone()).unwrap_or_default(),
            })
            .collect();
    }
    // the segment duration is a float in timestamp units (RFC 9559 §5.1.2.3); whole nanoseconds are exact enough
    let end = f.info.duration.filter(|d| d.is_finite() && *d >= 0.0).map(|d| ticks((d * f.info.timestamp_scale as f64) as u64, 1_000_000_000));
    close_chapters(&mut l.chapters, end);
    l
}

/// The codec FFmpeg gives a Matroska attachment, by its media type (fonts), or by its extension.
fn attachment_codec(media_type: &str, name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match media_type.to_ascii_lowercase().as_str() {
        "application/x-truetype-font" | "application/x-font" | "application/font-sfnt" | "font/ttf" | "font/sfnt" => "ttf",
        "application/vnd.ms-opentype" | "font/otf" => "otf",
        "application/x-font-otf" => "otf",
        "binary" => "bin_data",
        "text/plain" => "text",
        _ => match ext.as_str() {
            "ttf" => "ttf",
            "otf" => "otf",
            _ => "",
        },
    }
}

// ---------------------------------------------------------------- MPEG TS / PS

fn mpeg<S: cfa4_mpegts::ByteSource + ?Sized>(f: &cfa4_mpegts::File, src: &S) -> StreamList {
    use cfa4_mpegts::{Codec, Kind, StreamId};
    let ps = matches!(f.streams.first().map(|s| s.id), Some(StreamId::Ps { .. }));
    let mut l = StreamList { container: if ps { "mpegps" } else { "mpegts" }.into(), warnings: f.warnings.clone(), ..StreamList::default() };
    for (i, st) in f.streams.iter().enumerate() {
        let kind = match st.kind() {
            Kind::Video => StreamType::Video,
            Kind::Audio => StreamType::Audio,
            Kind::Subtitle => StreamType::Subtitle,
            Kind::Other => StreamType::Data,
        };
        let first = (!st.units.is_empty()).then(|| f.read_unit(src, i, 0).ok()).flatten();
        let info = first.as_deref().and_then(|b| cfa4_mpegts::frame_info(&st.codec, b));
        let codec = match &st.codec {
            Codec::Mpeg1Video => "mpeg1video",
            Codec::Mpeg2Video => "mpeg2video",
            Codec::H264 => "h264",
            Codec::Hevc => "hevc",
            Codec::MpegAudio => match info.map(|i| i.variant) {
                Some(1) => "mp1",
                Some(3) => "mp3",
                _ => "mp2",
            },
            Codec::AacAdts => "aac",
            Codec::AacLatm => "aac_latm",
            Codec::Ac3 => "ac3",
            Codec::Eac3 => "eac3",
            Codec::Dts => "dts",
            Codec::TrueHd => "truehd",
            Codec::LpcmBluray => "pcm_bluray",
            Codec::LpcmDvd => "pcm_dvd",
            Codec::Subtitle("DVB subtitles") => "dvb_subtitle",
            Codec::Subtitle("Teletext") => "dvb_teletext",
            Codec::Subtitle("PGS subtitles") => "hdmv_pgs_subtitle",
            Codec::Subtitle("Text subtitles") => "hdmv_text_subtitle",
            Codec::Subtitle("DVD sub-pictures") => "dvd_subtitle",
            Codec::Subtitle(_) | Codec::Unknown(_) => "",
        };
        let mut s = StreamEntry::new(kind).codec(codec);
        s.id = Some(match st.id {
            StreamId::Pid(pid) => format!("0x{pid:x}"),
            StreamId::Ps { sub_id: Some(sub), .. } => format!("0x{sub:x}"),
            StreamId::Ps { stream_id, sub_id: None } => format!("0x1{stream_id:02x}"),
        });
        s.codec_tag = Some(format!("0x{:02x}", st.stream_type));
        s.language = st.language.as_deref().and_then(language);
        if let Some(i) = info {
            s = s.audio(i.channels, i.sample_rate);
        }
        if let Some(p) = st.lpcm {
            s = s.audio(p.channels, p.sample_rate);
        }
        if let Some(b) = first.as_deref()
            && let Some((w, h)) = picture_size(&st.codec, b)
        {
            s = s.size(w, h);
        }
        l.push(s);
    }
    l
}

/// The picture size of a video stream from its first access unit (Annex B / MPEG start codes).
fn picture_size(codec: &cfa4_mpegts::Codec, unit: &[u8]) -> Option<(u32, u32)> {
    use cfa4_mpegts::Codec;
    match codec {
        Codec::Mpeg1Video | Codec::Mpeg2Video => cfa4_mpeg2v::probe(unit).map(|i| (i.width, i.height)),
        Codec::H264 => nal_units(unit).find(|n| n.first().is_some_and(|b| b & 0x1F == 7)).and_then(|n| {
            let sps = cfa4_h264::params::Sps::parse(&cfa4_bitstream::unescape_rbsp(n.get(1..)?)).ok()?;
            let (_, _, w, h) = sps.crop_rect();
            Some((w, h))
        }),
        Codec::Hevc => nal_units(unit).find(|n| n.first().is_some_and(|b| (b >> 1) & 0x3F == 33)).and_then(|n| {
            let sps = cfa4_hevc::params::Sps::parse(&cfa4_bitstream::unescape_rbsp(n.get(2..)?)).ok()?;
            let (_, _, w, h) = sps.crop_rect();
            Some((w, h))
        }),
        _ => None,
    }
}

/// The NAL units of an Annex B byte stream (start codes removed).
fn nal_units(b: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= b.len() {
        if b[i] == 0 && b[i + 1] == 0 && b[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let ends: Vec<usize> = starts.iter().skip(1).map(|&s| s - 3).chain(std::iter::once(b.len())).collect();
    starts.into_iter().zip(ends).map(move |(s, e)| {
        // a 4-byte start code leaves a zero byte at the end of the previous unit
        let mut e = e.max(s);
        while e > s && b[e - 1] == 0 {
            e -= 1;
        }
        &b[s..e]
    })
}

// ---------------------------------------------------------------- MXF

fn mxf(f: &cfa4_mxf::MxfFile) -> StreamList {
    use cfa4_mxf::{Codec, TrackKind};
    let mut l = StreamList { container: "mxf".into(), warnings: f.warnings.clone(), ..StreamList::default() };
    for t in &f.tracks {
        let kind = match t.kind {
            TrackKind::Picture => StreamType::Video,
            TrackKind::Sound => StreamType::Audio,
            TrackKind::Data => StreamType::Data,
        };
        let bits = t.sound.as_ref().map(|s| s.bits).unwrap_or(16);
        let codec = match t.codec {
            Codec::Avc { .. } => "h264".to_string(),
            Codec::Vc3 => "dnxhd".into(),
            Codec::ProRes { .. } => "prores".into(),
            Codec::Mpeg2 => "mpeg2video".into(),
            Codec::Mpeg4Visual => "mpeg4".into(),
            Codec::Jpeg2000 => "jpeg2000".into(),
            Codec::Dv => "dvvideo".into(),
            Codec::Uncompressed => "rawvideo".into(),
            Codec::Pcm | Codec::Aes3Element => format!("pcm_s{bits}le"),
            Codec::Iab => "iab".into(),
            Codec::Unknown => String::new(),
        };
        let mut s = StreamEntry::new(kind).codec(codec);
        s.id = Some(t.track_id.to_string());
        s.title = t.name.as_deref().and_then(some_text);
        if let Some(p) = &t.picture {
            let (w, h) = if p.display_width > 0 && p.display_height > 0 { (p.display_width, p.display_height) } else { (p.stored_width, p.stored_height) };
            match mxf_frame_height(h, p.frame_layout) {
                Some(h) => s = s.size(w, h),
                None => l.warnings.push(format!("track {}: picture height {h} with separated fields overflows; size not listed", t.track_id)),
            }
        }
        if let Some(a) = &t.sound {
            let rate = if a.sample_rate.den > 0 { (a.sample_rate.num / a.sample_rate.den) as u32 } else { 0 };
            s = s.audio(a.channels, rate);
        }
        l.push(s);
    }
    l
}

/// The frame height of an MXF picture: separated fields (frame layout 1) describe one field, so
/// the frame is twice as tall. `None` when that overflows (a corrupt descriptor).
fn mxf_frame_height(height: u32, frame_layout: u8) -> Option<u32> {
    if frame_layout == 1 { height.checked_mul(2) } else { Some(height) }
}

// ---------------------------------------------------------------- Ogg

/// Pages read at most while looking for the first page of every logical stream.
const OGG_MAX_HEADER_PAGES: usize = 1024;

/// The first packet of each logical stream of an Ogg file, by serial, in file order. Every stream
/// begins with a beginning-of-stream page, and all of them precede the first data page (RFC 3533
/// §4), so only the pages at the start of the file are read, never the whole file. A chained
/// file (another link after the first one ends) is described by its first link, as ffprobe does.
fn ogg_first_packets(src: &dyn ByteReader) -> Result<Vec<(u32, Vec<u8>)>, String> {
    let len = src.len();
    let mut firsts: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut pos = 0u64;
    let mut hdr = [0u8; 27 + 255];
    for _ in 0..OGG_MAX_HEADER_PAGES {
        if pos + 27 > len {
            break;
        }
        src.read_at(pos, &mut hdr[..27]).map_err(|e| e.to_string())?;
        if &hdr[..4] != b"OggS" {
            if firsts.is_empty() {
                return Err(format!("no Ogg page at byte {pos}"));
            }
            break;
        }
        let bos = hdr[5] & 2 != 0;
        if !bos {
            break;
        }
        let serial = u32::from_le_bytes([hdr[14], hdr[15], hdr[16], hdr[17]]);
        let nseg = usize::from(hdr[26]);
        if pos + 27 + nseg as u64 > len {
            return Err(format!("truncated Ogg page header at byte {pos}"));
        }
        src.read_at(pos + 27, &mut hdr[27..27 + nseg]).map_err(|e| e.to_string())?;
        let lacing = &hdr[27..27 + nseg];
        let body_len: u64 = lacing.iter().map(|&b| u64::from(b)).sum();
        // the first packet: segments up to and including the first shorter than 255 bytes
        let first_len: u64 = match lacing.iter().position(|&b| b < 255) {
            Some(i) => lacing[..=i].iter().map(|&b| u64::from(b)).sum(),
            None => body_len,
        };
        let body_at = pos + 27 + nseg as u64;
        // the whole page must be there, not only its first packet (a page cut after it is damaged)
        if body_at.checked_add(body_len).is_none_or(|end| end > len) {
            return Err(format!("truncated Ogg page at byte {pos}"));
        }
        let mut packet = vec![0; first_len as usize];
        src.read_at(body_at, &mut packet).map_err(|e| e.to_string())?;
        if !firsts.iter().any(|(s, _)| *s == serial) {
            firsts.push((serial, packet));
        }
        pos = body_at + body_len;
    }
    if firsts.is_empty() {
        return Err("no Ogg beginning-of-stream page".into());
    }
    Ok(firsts)
}

fn ogg(src: &dyn ByteReader) -> Result<StreamList, String> {
    let mut l = StreamList { container: "ogg".into(), ..StreamList::default() };
    for (serial, h) in ogg_first_packets(src)? {
        let h = h.as_slice();
        let le32 = |o: usize| h.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0);
        let be24 = |o: usize| h.get(o..o + 3).map(|b| u32::from_be_bytes([0, b[0], b[1], b[2]])).unwrap_or(0);
        let mut s = if h.starts_with(b"OpusHead") {
            StreamEntry::new(StreamType::Audio).codec("opus").audio(h.get(9).copied().map(u32::from).unwrap_or(0), 48_000)
        } else if h.starts_with(b"\x01vorbis") {
            StreamEntry::new(StreamType::Audio).codec("vorbis").audio(h.get(11).copied().map(u32::from).unwrap_or(0), le32(12))
        } else if h.starts_with(b"Speex   ") {
            StreamEntry::new(StreamType::Audio).codec("speex").audio(le32(48), le32(36))
        } else if h.starts_with(b"\x7fFLAC") {
            // 0x7F "FLAC", version (2 bytes), header count (2 bytes), then "fLaC" + STREAMINFO
            let (rate, ch) = h.get(9..).and_then(flac_streaminfo).unwrap_or((0, 0));
            StreamEntry::new(StreamType::Audio).codec("flac").audio(ch, rate)
        } else if h.starts_with(b"\x80theora") {
            StreamEntry::new(StreamType::Video).codec("theora").size(be24(14), be24(17))
        } else {
            StreamEntry::new(StreamType::Data)
        };
        s.id = Some(serial.to_string());
        l.push(s);
    }
    Ok(l)
}

/// (sample rate, channels) of a FLAC `fLaC` + STREAMINFO block.
fn flac_streaminfo(b: &[u8]) -> Option<(u32, u32)> {
    flac_streaminfo_checked(b).ok()
}

/// (sample rate, channels) of a FLAC `fLaC` + STREAMINFO block, checking that the first metadata
/// block is a STREAMINFO (type 0) of its 34 bytes and that all of them are there.
fn flac_streaminfo_checked(b: &[u8]) -> Result<(u32, u32), String> {
    let rest = b.strip_prefix(b"fLaC").ok_or("no fLaC marker")?;
    let header = rest.get(..4).ok_or("truncated FLAC metadata block header")?;
    if header[0] & 0x7F != 0 {
        return Err(format!("the first FLAC metadata block is type {}, not STREAMINFO", header[0] & 0x7F));
    }
    let size = u32::from_be_bytes([0, header[1], header[2], header[3]]);
    if size != 34 {
        return Err(format!("FLAC STREAMINFO is {size} bytes, not 34"));
    }
    let si = rest.get(4..4 + 34).ok_or("truncated FLAC STREAMINFO")?;
    let rate = (u32::from(si[10]) << 12) | (u32::from(si[11]) << 4) | (u32::from(si[12]) >> 4);
    let ch = ((u32::from(si[12]) >> 1) & 7) + 1;
    Ok((rate, ch))
}

// ---------------------------------------------------------------- AVI / WAV / FLAC

fn avi(f: &cfa4_avi::AviFile) -> StreamList {
    use cfa4_avi::StreamKind;
    let mut l = StreamList { container: "avi".into(), ..StreamList::default() };
    for st in &f.streams {
        let mut s = match st.kind {
            StreamKind::Video => {
                let (codec, tag, w, h) = match &st.video {
                    Some(v) => (avi_video_codec(v.compression), fourcc_tag(v.compression), v.width.unsigned_abs(), v.height.unsigned_abs()),
                    None => (String::new(), fourcc_tag(st.handler), 0, 0),
                };
                let mut s = StreamEntry::new(StreamType::Video).codec(codec).size(w, h);
                s.codec_tag = tag;
                s
            }
            StreamKind::Audio => match &st.audio {
                Some(a) => wave_format_entry(a.format_tag, a.bits_per_sample, &a.extra).audio(u32::from(a.channels), a.sample_rate),
                None => StreamEntry::new(StreamType::Audio),
            },
            StreamKind::Other(t) => {
                let mut s = StreamEntry::new(if &t == b"txts" { StreamType::Subtitle } else { StreamType::Data });
                s.codec_tag = fourcc_tag(t);
                s
            }
        };
        s.title = st.name.as_deref().and_then(some_text);
        l.push(s);
    }
    l
}

fn fourcc_tag(fc: [u8; 4]) -> Option<String> {
    if u32::from_le_bytes(fc) < 0x100 {
        return None;
    }
    some_text(&String::from_utf8_lossy(&fc))
}

/// FFmpeg's codec of an AVI video FourCC (a common subset; others are named by the FourCC).
fn avi_video_codec(fc: [u8; 4]) -> String {
    let up: [u8; 4] = fc.map(|b| b.to_ascii_uppercase());
    let c = match &up {
        [0, 0, 0, 0] | [3, 0, 0, 0] => "rawvideo",
        b"MJPG" | b"AVRN" | b"LJPG" | b"JPGL" | b"AVDJ" => "mjpeg",
        b"H264" | b"X264" | b"AVC1" | b"DAVC" => "h264",
        b"HEVC" | b"H265" | b"HVC1" | b"HEV1" => "hevc",
        b"XVID" | b"DIVX" | b"DX50" | b"FMP4" | b"MP4V" | b"3IV2" => "mpeg4",
        b"MPG2" | b"MPEG" | b"MPG1" | b"MMES" => "mpeg2video",
        b"DVSD" | b"DV25" | b"DV50" | b"DVHD" | b"DVSL" | b"CDVC" => "dvvideo",
        b"FFV1" => "ffv1",
        b"HFYU" => "huffyuv",
        b"FFVH" => "ffvhuff",
        b"ULRG" | b"ULRA" | b"ULY0" | b"ULY2" | b"ULY4" | b"ULH0" | b"ULH2" | b"ULH4" => "utvideo",
        b"CFHD" => "cfhd",
        b"AVDN" | b"AVDH" => "dnxhd",
        b"VP80" => "vp8",
        b"VP90" => "vp9",
        b"AV01" => "av1",
        b"DRAC" => "dirac",
        b"CUVC" | b"CHQX" | b"CHQ2" => "hq_hqa",
        _ => return String::from_utf8_lossy(&fc).trim().to_ascii_lowercase(),
    };
    c.into()
}

/// An audio stream described by a `WAVEFORMATEX` (`wFormatTag`, bits, extra bytes).
fn wave_format_entry(format_tag: u16, bits: u16, extra: &[u8]) -> StreamEntry {
    // WAVE_FORMAT_EXTENSIBLE: the sub-format GUID's first two bytes are the real tag
    let tag = if format_tag == 0xFFFE { extra.get(6..8).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(format_tag) } else { format_tag };
    let codec = match tag {
        0x0001 => pcm_name(bits, false, false, bits != 8),
        0x0003 => pcm_name(bits, true, false, true),
        0x0002 => "adpcm_ms".into(),
        0x0006 => "pcm_alaw".into(),
        0x0007 => "pcm_mulaw".into(),
        0x0011 => "adpcm_ima_wav".into(),
        0x0050 => "mp2".into(),
        0x0055 => "mp3".into(),
        0x00FF | 0x1610 | 0x706D => "aac".into(),
        0x0160 => "wmav1".into(),
        0x0161 => "wmav2".into(),
        0x2000 => "ac3".into(),
        0x2001 => "dts".into(),
        0xF1AC => "flac".into(),
        _ => String::new(),
    };
    let mut s = StreamEntry::new(StreamType::Audio).codec(codec);
    s.codec_tag = Some(format!("0x{format_tag:04x}"));
    s
}

/// Chunks walked at most while looking for the WAV `fmt ` chunk.
const WAV_MAX_CHUNKS: usize = 256;
/// The largest `fmt ` chunk read (a WAVEFORMATEXTENSIBLE is 40 bytes).
const WAV_MAX_FMT: u64 = 64 << 10;

/// A WAV file: its `fmt ` chunk, found by walking the chunk headers from the source (a large
/// `JUNK` or `bext` chunk before it is skipped, not read). `None` when there is none (the caller
/// falls back to the opened source); an error when it is truncated or too short to describe the
/// audio.
fn wav(src: &dyn ByteReader) -> Option<Result<StreamList, String>> {
    let len = src.len();
    let mut o = 12u64;
    for _ in 0..WAV_MAX_CHUNKS {
        if o.checked_add(8).is_none_or(|e| e > len) {
            return None;
        }
        let mut h = [0u8; 8];
        if let Err(e) = src.read_at(o, &mut h) {
            return Some(Err(e.to_string()));
        }
        let size = u64::from(u32::from_le_bytes([h[4], h[5], h[6], h[7]]));
        if &h[..4] == b"fmt " {
            // WAVEFORMAT is 16 bytes (tag, channels, rate, byte rate, block align, bits)
            if !(16..=WAV_MAX_FMT).contains(&size) || o + 8 + size > len {
                return Some(Err(format!("truncated WAV fmt chunk at byte {o} ({size} bytes)")));
            }
            let mut b = vec![0u8; size as usize];
            if let Err(e) = src.read_at(o + 8, &mut b) {
                return Some(Err(e.to_string()));
            }
            let le16 = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
            let rate = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
            let extra = b.get(18..).unwrap_or_default();
            let s = wave_format_entry(le16(0), le16(14), extra).audio(u32::from(le16(2)), rate);
            let mut l = StreamList { container: "wav".into(), ..StreamList::default() };
            l.push(s);
            return Some(Ok(l));
        }
        o = o.checked_add(8)?.checked_add(size)?.checked_add(size & 1)?;
    }
    None
}

/// A FLAC file (it starts with `fLaC`): its STREAMINFO, or an error when that is damaged.
fn flac(head: &[u8]) -> Result<StreamList, String> {
    let (rate, ch) = flac_streaminfo_checked(head)?;
    let mut l = StreamList { container: "flac".into(), ..StreamList::default() };
    l.push(StreamEntry::new(StreamType::Audio).codec("flac").audio(ch, rate));
    Ok(l)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav_head(fmt_size: u32, fmt: &[u8]) -> Vec<u8> {
        let mut b = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        b.extend(fmt_size.to_le_bytes());
        b.extend(fmt);
        b
    }

    fn mem(b: Vec<u8>) -> cfa4_media::reader::MemReader {
        cfa4_media::reader::MemReader(b.into())
    }

    fn pcm_fmt() -> Vec<u8> {
        let mut fmt = vec![1, 0, 2, 0];
        fmt.extend(48_000u32.to_le_bytes());
        fmt.extend(192_000u32.to_le_bytes());
        fmt.extend([4, 0, 16, 0]);
        fmt
    }

    #[test]
    fn a_wav_fmt_chunk_is_read() {
        let l = wav(&mem(wav_head(16, &pcm_fmt()))).unwrap().unwrap();
        assert_eq!((l.streams[0].codec.as_deref(), l.streams[0].channels, l.streams[0].sample_rate), (Some("pcm_s16le"), Some(2), Some(48_000)));
    }

    #[test]
    fn a_wav_fmt_chunk_after_a_large_junk_chunk_is_read() {
        // the fmt chunk starts past the bytes read to recognise the container
        let junk = HEAD as usize + 1001;
        let mut b = b"RIFF\0\0\0\0WAVEJUNK".to_vec();
        b.extend((junk as u32).to_le_bytes());
        b.extend(vec![0; junk + 1]); // odd size: one pad byte
        b.extend(b"fmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(pcm_fmt());
        let l = stream_list(&mem(b)).unwrap().unwrap();
        assert_eq!((l.container.as_str(), l.streams[0].channels, l.streams[0].sample_rate), ("wav", Some(2), Some(48_000)));
    }

    #[test]
    fn a_truncated_wav_fmt_chunk_is_an_error() {
        // the chunk says 16 bytes but the file ends after 6; a chunk that says 6 is too short too
        assert!(wav(&mem(wav_head(16, &[1, 0, 2, 0, 0x80, 0xBB]))).unwrap().is_err());
        assert!(wav(&mem(wav_head(6, &[1, 0, 2, 0, 0x80, 0xBB]))).unwrap().is_err());
        // no fmt chunk: not recognised, the caller falls back
        assert!(wav(&mem(b"RIFF\0\0\0\0WAVE".to_vec())).is_none());
    }

    #[test]
    fn an_overflowing_mxf_field_height_is_not_listed() {
        assert_eq!(mxf_frame_height(288, 1), Some(576));
        assert_eq!(mxf_frame_height(1080, 0), Some(1080));
        assert_eq!(mxf_frame_height(0x8000_0001, 1), None);
        assert_eq!(mxf_frame_height(0x8000_0001, 0), Some(0x8000_0001));
    }

    #[test]
    fn ogg_first_packets_stop_at_the_first_data_page() {
        fn page(flags: u8, serial: u32, body: &[u8]) -> Vec<u8> {
            let mut p = b"OggS\0".to_vec();
            p.push(flags);
            p.extend([0; 8]);
            p.extend(serial.to_le_bytes());
            p.extend([0; 8]);
            p.push(1);
            p.push(body.len() as u8);
            p.extend(body);
            p
        }
        let mut f = page(2, 7, b"OpusHead\x01\x02");
        f.extend(page(0, 7, b"data"));
        // a chained file: the first link ends (EOS) and a new one begins (BOS). Like ffprobe, the
        // list describes the first link; reading the next one would mean walking every page.
        f.extend(page(4, 7, b"last"));
        f.extend(page(2, 9, b"late bos is not read"));
        let r = cfa4_media::reader::MemReader(f.into());
        let firsts = ogg_first_packets(&r).unwrap();
        assert_eq!(firsts, [(7, b"OpusHead\x01\x02".to_vec())]);
        let l = ogg(&r).unwrap();
        assert_eq!((l.streams[0].codec.as_deref(), l.streams[0].channels), (Some("opus"), Some(2)));
    }
}
