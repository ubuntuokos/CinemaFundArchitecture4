//! HLS output of the stepped exporter (RFC 8216): H.264 + AAC in CMAF fragmented-MP4 segments.
//! The output path is the multivariant playlist; next to it go the media playlist, the init
//! segment (`ftyp` + `moov`) and numbered `moof` + `mdat` segments ([`hls_files`]). A segment
//! starts at a video key frame once the previous one holds [`SEGMENT_SECONDS`], so every segment
//! decodes on its own (`EXT-X-INDEPENDENT-SEGMENTS`). The sound of a segment is the AAC access
//! units starting before its end; a segment is cut only once the sound has caught up with it.

use std::collections::VecDeque;
use std::fmt::Write as _;

use cfa4_isobmff::{Brand, CodecConfig, FragmentedWriter, SampleEntry, TrackConfig, WriteSample, WriterOptions};
use cfa4_time::FrameRate;

use crate::{EncodedPacket, ExportError, ExportSettings, Result, write_output};

/// Target segment length (Apple's HLS authoring guidance).
pub const SEGMENT_SECONDS: u64 = 6;

/// The files of an HLS export to `path` with a `height`-line rendition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HlsFiles {
    /// The multivariant playlist: `path` itself.
    pub multivariant: String,
    /// `<stem>_<height>p.m3u8`.
    pub media: String,
    /// `<stem>_<height>p_init.mp4`.
    pub init: String,
    /// `<stem>_<height>p_`: segment `n` is `<prefix><n:05>.m4s`.
    pub segment_prefix: String,
}

impl HlsFiles {
    pub fn segment(&self, n: usize) -> String {
        format!("{}{n:05}.m4s", self.segment_prefix)
    }
}

/// Where an HLS export to `path` writes its files.
pub fn hls_files(path: &str, height: u32) -> HlsFiles {
    let p = std::path::Path::new(path);
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "export".into());
    let at = |name: String| match p.parent().filter(|d| !d.as_os_str().is_empty()) {
        Some(d) => d.join(name).to_string_lossy().to_string(),
        None => name,
    };
    HlsFiles {
        multivariant: path.to_string(),
        media: at(format!("{stem}_{height}p.m3u8")),
        init: at(format!("{stem}_{height}p_init.mp4")),
        segment_prefix: at(format!("{stem}_{height}p_")),
    }
}

/// The file name of `path` (playlists refer to files next to them).
fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

/// The RFC 6381 `CODECS` entry of a video sample entry (`avc1.PPCCLL`).
pub(crate) fn video_codecs(entry: &SampleEntry) -> String {
    match &entry.codec {
        CodecConfig::Avc(c) => format!("avc1.{:02x}{:02x}{:02x}", c.profile, c.compatibility, c.level),
        _ => "avc1.640028".into(),
    }
}

fn mux_err(e: cfa4_isobmff::Error) -> ExportError {
    ExportError::Io(e.to_string())
}

/// Everything the HLS muxer needs from the export.
pub(crate) struct HlsSetup<'a> {
    pub settings: &'a ExportSettings,
    pub width: u32,
    pub height: u32,
    pub rate: FrameRate,
    pub video: SampleEntry,
    pub video_timescale: u32,
    /// The B-frame delay in the video timescale: subtracted from composition offsets so that the
    /// first picture is presented at 0.
    pub video_delay: i64,
    /// The audio sample entry, sample rate and encoder delay (priming) in samples.
    pub audio: Option<(SampleEntry, u32, u32)>,
}

/// One written segment.
struct Segment {
    name: String,
    seconds: f64,
    bytes: u64,
}

/// An HLS export in progress.
pub(crate) struct HlsMux {
    settings: ExportSettings,
    files: HlsFiles,
    frag: FragmentedWriter<Vec<u8>>,
    vt: usize,
    at: Option<usize>,
    v_ts: u64,
    a_ts: u64,
    video_delay: i64,
    /// Video packets not in a segment yet, with their decode times.
    vq: VecDeque<(u64, EncodedPacket)>,
    v_end: u64,
    /// AAC access units not in a segment yet: decode time, data, duration.
    aq: VecDeque<(u64, Vec<u8>, u32)>,
    a_end: u64,
    /// Decode times of the key frames that start the next segments.
    cuts: VecDeque<u64>,
    last_cut: u64,
    seg_start: u64,
    segments: Vec<Segment>,
    /// Bytes written (playlists included at the end).
    bytes: u64,
    codecs: String,
    width: u32,
    height: u32,
    rate: FrameRate,
}

impl HlsMux {
    pub fn new(s: HlsSetup) -> Result<Self> {
        let files = hls_files(&s.settings.path, s.height);
        let mut opts = WriterOptions::new(Brand::Mp4);
        opts.metadata = s.settings.metadata.udta();
        let mut frag = FragmentedWriter::new(Vec::new(), opts);
        let mut codecs = video_codecs(&s.video);
        let vt = frag.add_track(TrackConfig::new(s.video, s.video_timescale)).map_err(mux_err)?;
        let (at, a_ts) = match s.audio {
            Some((entry, sr, priming)) => {
                let mut c = TrackConfig::new(entry, sr);
                c.media_start = Some(i64::from(priming));
                codecs += ",mp4a.40.2";
                (Some(frag.add_track(c).map_err(mux_err)?), u64::from(sr.max(1)))
            }
            None => (None, 1),
        };
        frag.write_init_segment().map_err(mux_err)?;
        let init = std::mem::take(frag.get_mut());
        let bytes = write_output(s.settings, &files.init, init)?;
        Ok(HlsMux {
            settings: s.settings.clone(),
            files,
            frag,
            vt,
            at,
            v_ts: u64::from(s.video_timescale.max(1)),
            a_ts,
            video_delay: s.video_delay,
            vq: VecDeque::new(),
            v_end: 0,
            aq: VecDeque::new(),
            a_end: 0,
            cuts: VecDeque::new(),
            last_cut: 0,
            seg_start: 0,
            segments: Vec::new(),
            bytes,
            codecs,
            width: s.width,
            height: s.height,
            rate: s.rate,
        })
    }

    pub fn write_video(&mut self, packets: Vec<EncodedPacket>) -> Result<()> {
        let target = SEGMENT_SECONDS.saturating_mul(self.v_ts);
        for p in packets {
            if p.key && self.v_end > self.last_cut && self.v_end - self.last_cut >= target {
                self.cuts.push_back(self.v_end);
                self.last_cut = self.v_end;
            }
            let dts = self.v_end;
            self.v_end = self.v_end.saturating_add(u64::from(p.duration));
            self.vq.push_back((dts, p));
        }
        self.pump()
    }

    /// AAC access units with their durations (samples).
    pub fn write_audio(&mut self, units: Vec<(Vec<u8>, u32)>) -> Result<()> {
        for (au, d) in units {
            self.aq.push_back((self.a_end, au, d));
            self.a_end = self.a_end.saturating_add(u64::from(d));
        }
        self.pump()
    }

    /// Whether the sound has reached video time `t`.
    fn audio_reached(&self, t: u64) -> bool {
        self.at.is_none() || u128::from(self.a_end) * u128::from(self.v_ts) >= u128::from(t) * u128::from(self.a_ts)
    }

    fn pump(&mut self) -> Result<()> {
        while let Some(&t) = self.cuts.front() {
            if !self.audio_reached(t) {
                break;
            }
            self.cuts.pop_front();
            self.segment(t)?;
        }
        Ok(())
    }

    /// Write the samples before video time `end` as the next segment.
    fn segment(&mut self, end: u64) -> Result<()> {
        let mut last = self.seg_start;
        while self.vq.front().is_some_and(|(dts, _)| *dts < end) {
            let Some((dts, p)) = self.vq.pop_front() else { break };
            last = dts.saturating_add(u64::from(p.duration));
            let cto = i64::from(p.composition_offset).saturating_sub(self.video_delay);
            let cto = i32::try_from(cto).unwrap_or(0);
            self.frag.write_sample(self.vt, WriteSample { data: &p.data, duration: p.duration, composition_offset: cto, is_sync: p.key }).map_err(mux_err)?;
        }
        if let Some(at) = self.at {
            let (v_ts, a_ts) = (u128::from(self.v_ts), u128::from(self.a_ts));
            while self.aq.front().is_some_and(|(dts, ..)| u128::from(*dts) * v_ts < u128::from(end) * a_ts) {
                let Some((_, au, d)) = self.aq.pop_front() else { break };
                self.frag.write_sample(at, WriteSample { data: &au, duration: d, composition_offset: 0, is_sync: true }).map_err(mux_err)?;
            }
        }
        self.frag.flush_fragment().map_err(mux_err)?;
        let data = std::mem::take(self.frag.get_mut());
        if data.is_empty() {
            return Ok(());
        }
        let path = self.files.segment(self.segments.len());
        let n = write_output(&self.settings, &path, data)?;
        self.bytes = self.bytes.saturating_add(n);
        let seconds = last.saturating_sub(self.seg_start) as f64 / self.v_ts as f64;
        self.segments.push(Segment { name: file_name(&path), seconds, bytes: n });
        self.seg_start = last;
        Ok(())
    }

    /// Write the last segment and the playlists; returns (total bytes, files besides the path).
    pub fn finish(mut self) -> Result<(u64, Vec<String>)> {
        // every cut, then everything left
        while let Some(t) = self.cuts.pop_front() {
            self.segment(t)?;
        }
        self.segment(u64::MAX)?;
        let media = self.media_playlist();
        let multivariant = self.multivariant_playlist();
        self.bytes = self.bytes.saturating_add(write_output(&self.settings, &self.files.media, media.into_bytes())?);
        self.bytes = self.bytes.saturating_add(write_output(&self.settings, &self.files.multivariant, multivariant.into_bytes())?);
        let mut extra = vec![self.files.media.clone(), self.files.init.clone()];
        extra.extend((0..self.segments.len()).map(|n| self.files.segment(n)));
        Ok((self.bytes, extra))
    }

    fn media_playlist(&self) -> String {
        // EXTINF durations rounded to the nearest second never exceed the target (RFC 8216 §4.3.3.1)
        let target = self.segments.iter().map(|s| s.seconds.round() as u64).max().unwrap_or(0).max(1);
        let mut m = format!(
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{target}\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-INDEPENDENT-SEGMENTS\n#EXT-X-MAP:URI=\"{}\"\n",
            file_name(&self.files.init)
        );
        for s in &self.segments {
            let _ = write!(m, "#EXTINF:{:.5},\n{}\n", s.seconds, s.name);
        }
        m += "#EXT-X-ENDLIST\n";
        m
    }

    fn multivariant_playlist(&self) -> String {
        let bits = |s: &Segment| if s.seconds > 0.0 { s.bytes as f64 * 8.0 / s.seconds } else { 0.0 };
        let peak = self.segments.iter().map(bits).fold(0.0, f64::max).ceil() as u64;
        let seconds: f64 = self.segments.iter().map(|s| s.seconds).sum();
        let bytes: u64 = self.segments.iter().map(|s| s.bytes).sum();
        let average = if seconds > 0.0 { (bytes as f64 * 8.0 / seconds).ceil() as u64 } else { 0 };
        let fps = self.rate.as_f64();
        format!(
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-INDEPENDENT-SEGMENTS\n#EXT-X-STREAM-INF:BANDWIDTH={},AVERAGE-BANDWIDTH={},CODECS=\"{}\",RESOLUTION={}x{},FRAME-RATE={:.3}\n{}\n",
            peak.max(1),
            average.max(1),
            self.codecs,
            self.width,
            self.height,
            if fps.is_finite() { fps } else { 0.0 },
            file_name(&self.files.media)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_next_to_the_playlist() {
        let f = hls_files("/out/Demo.m3u8", 720);
        assert_eq!(f.multivariant, "/out/Demo.m3u8");
        assert_eq!(f.media, "/out/Demo_720p.m3u8");
        assert_eq!(f.init, "/out/Demo_720p_init.mp4");
        assert_eq!(f.segment(3), "/out/Demo_720p_00003.m4s");
        assert_eq!(hls_files("Demo.m3u8", 1080).media, "Demo_1080p.m3u8");
    }

    #[test]
    fn codecs_of_an_avc_entry() {
        let mut c = cfa4_isobmff::AvcConfig::new(vec![vec![0x67, 0x64, 0, 0x28]], vec![vec![0x68]], 4);
        (c.profile, c.compatibility, c.level) = (100, 0, 40);
        assert_eq!(video_codecs(&SampleEntry::avc(c, 64, 36)), "avc1.640028");
    }
}
