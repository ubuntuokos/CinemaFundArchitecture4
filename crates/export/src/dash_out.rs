//! MPEG-DASH manifest (MPD, ISO/IEC 23009-1) for the CMAF segments an HLS export writes: a static
//! `isoff-live` presentation, one AdaptationSet per media type, `SegmentTemplate` with a
//! `SegmentTimeline` (segments are cut at key frames, so their lengths differ).
//!
//! The manifest is text only; it points at the init segment and the numbered `.m4s` segments by
//! file name, so it sits next to them like the HLS playlists do.

use std::fmt::Write as _;

/// The DASH manifest an HLS export to `path` writes with [`crate::ExportSettings::dash_manifest`]:
/// `<stem>.mpd` next to the multivariant playlist.
pub fn dash_files(path: &str) -> String {
    std::path::Path::new(path).with_extension("mpd").to_string_lossy().to_string()
}

/// What a [`DashRepresentation`] carries.
#[derive(Clone, Debug, PartialEq)]
pub enum DashMedia {
    /// Video: coded size and frame rate (`num`/`den`).
    Video { width: u32, height: u32, frame_rate: (i64, i64) },
    /// Audio: sample rate and channel count.
    Audio { sample_rate: u32, channels: u16 },
}

/// One Representation: a track's init segment and its numbered media segments.
#[derive(Clone, Debug, PartialEq)]
pub struct DashRepresentation {
    /// `Representation@id` (unique in the manifest).
    pub id: String,
    pub media: DashMedia,
    /// RFC 6381 codecs string (`avc1.640028`, `mp4a.40.2`).
    pub codecs: String,
    /// Peak bits per second.
    pub bandwidth: u64,
    /// Media timescale of the segments (the track's `mdhd` timescale).
    pub timescale: u32,
    /// Init segment file name.
    pub init: String,
    /// Media segment `n` is `<segment_prefix><n:05>.m4s` (the [`crate::HlsFiles`] naming, from 0).
    pub segment_prefix: String,
    /// Decode time of the first segment (its `tfdt`), in `timescale`.
    pub start: u64,
    /// Segment durations in `timescale`, in order.
    pub segments: Vec<u64>,
}

/// The MPD for `reps`, lasting `duration_seconds`. Video and audio Representations go into separate
/// AdaptationSets (video first). File names are XML-escaped, and `$` (the template marker) is
/// doubled as the template syntax requires.
pub fn dash_mpd(duration_seconds: f64, reps: &[DashRepresentation]) -> String {
    let secs = if duration_seconds.is_finite() { duration_seconds.max(0.0) } else { 0.0 };
    let min_buffer = reps.iter().flat_map(|r| r.segments.iter().map(move |&d| d as f64 / f64::from(r.timescale.max(1)))).fold(0.0, f64::max).ceil().max(1.0);
    let mut m = String::new();
    m += "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";
    let _ = writeln!(
        m,
        "<MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" profiles=\"urn:mpeg:dash:profile:isoff-live:2011\" type=\"static\" mediaPresentationDuration=\"{}\" minBufferTime=\"{}\">",
        iso_duration(secs),
        iso_duration(min_buffer)
    );
    m += "  <Period id=\"0\" start=\"PT0S\">\n";
    let video: Vec<&DashRepresentation> = reps.iter().filter(|r| matches!(r.media, DashMedia::Video { .. })).collect();
    let audio: Vec<&DashRepresentation> = reps.iter().filter(|r| matches!(r.media, DashMedia::Audio { .. })).collect();
    for (id, (kind, set)) in [("video", video), ("audio", audio)].into_iter().filter(|(_, s)| !s.is_empty()).enumerate() {
        let _ = writeln!(m, "    <AdaptationSet id=\"{id}\" contentType=\"{kind}\" mimeType=\"{kind}/mp4\" segmentAlignment=\"true\" startWithSAP=\"1\">");
        for r in set {
            representation(&mut m, r);
        }
        m += "    </AdaptationSet>\n";
    }
    m += "  </Period>\n</MPD>\n";
    m
}

fn representation(m: &mut String, r: &DashRepresentation) {
    let _ = write!(m, "      <Representation id=\"{}\" codecs=\"{}\" bandwidth=\"{}\"", xml(&r.id), xml(&r.codecs), r.bandwidth.max(1));
    match &r.media {
        DashMedia::Video { width, height, frame_rate: (num, den) } => {
            let _ = write!(m, " width=\"{width}\" height=\"{height}\"");
            if *num > 0 && *den > 0 {
                let _ = write!(m, " frameRate=\"{}\"", if *den == 1 { num.to_string() } else { format!("{num}/{den}") });
            }
            m.push_str(">\n");
        }
        DashMedia::Audio { sample_rate, channels } => {
            let _ = writeln!(m, " audioSamplingRate=\"{sample_rate}\">");
            let _ =
                writeln!(m, "        <AudioChannelConfiguration schemeIdUri=\"urn:mpeg:dash:23003:3:audio_channel_configuration:2011\" value=\"{channels}\"/>");
        }
    }
    let _ = writeln!(
        m,
        "        <SegmentTemplate timescale=\"{}\" initialization=\"{}\" media=\"{}$Number%05d$.m4s\" startNumber=\"0\">",
        r.timescale.max(1),
        xml(&template(&r.init)),
        xml(&template(&r.segment_prefix))
    );
    m.push_str("          <SegmentTimeline>\n");
    // runs of equal durations: <S t d r/>, `r` counting the repeats after the first
    let mut i = 0;
    let mut first = true;
    while i < r.segments.len() {
        let d = r.segments[i];
        let n = r.segments[i..].iter().take_while(|&&x| x == d).count();
        m.push_str("            <S");
        if first {
            let _ = write!(m, " t=\"{}\"", r.start);
            first = false;
        }
        let _ = write!(m, " d=\"{d}\"");
        if n > 1 {
            let _ = write!(m, " r=\"{}\"", n - 1);
        }
        m.push_str("/>\n");
        i += n;
    }
    m.push_str("          </SegmentTimeline>\n        </SegmentTemplate>\n      </Representation>\n");
}

/// `PT<s>S` with millisecond precision, trailing zeros dropped.
fn iso_duration(secs: f64) -> String {
    let s = format!("{secs:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    format!("PT{s}S")
}

/// A literal file name inside a template: `$` is written `$$`.
fn template(name: &str) -> String {
    name.replace('$', "$$")
}

fn xml(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c => o.push(c),
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(segments: Vec<u64>) -> DashRepresentation {
        DashRepresentation {
            id: "v".into(),
            media: DashMedia::Video { width: 1280, height: 720, frame_rate: (24000, 1001) },
            codecs: "avc1.640028".into(),
            bandwidth: 2_500_000,
            timescale: 24000,
            init: "Demo_720p_init.mp4".into(),
            segment_prefix: "Demo_720p_".into(),
            start: 0,
            segments,
        }
    }

    #[test]
    fn video_and_audio_sets_with_a_timeline() {
        let a = DashRepresentation {
            id: "a".into(),
            media: DashMedia::Audio { sample_rate: 48000, channels: 2 },
            codecs: "mp4a.40.2".into(),
            bandwidth: 0,
            timescale: 48000,
            init: "Demo_audio_init.mp4".into(),
            segment_prefix: "Demo_audio_".into(),
            start: 1024,
            segments: vec![288_000, 288_000, 48_000],
        };
        let mpd = dash_mpd(13.0, &[a, video(vec![144_144, 144_144, 144_144, 24_024])]);
        let want = r#"<?xml version="1.0" encoding="UTF-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" profiles="urn:mpeg:dash:profile:isoff-live:2011" type="static" mediaPresentationDuration="PT13S" minBufferTime="PT7S">
  <Period id="0" start="PT0S">
    <AdaptationSet id="0" contentType="video" mimeType="video/mp4" segmentAlignment="true" startWithSAP="1">
      <Representation id="v" codecs="avc1.640028" bandwidth="2500000" width="1280" height="720" frameRate="24000/1001">
        <SegmentTemplate timescale="24000" initialization="Demo_720p_init.mp4" media="Demo_720p_$Number%05d$.m4s" startNumber="0">
          <SegmentTimeline>
            <S t="0" d="144144" r="2"/>
            <S d="24024"/>
          </SegmentTimeline>
        </SegmentTemplate>
      </Representation>
    </AdaptationSet>
    <AdaptationSet id="1" contentType="audio" mimeType="audio/mp4" segmentAlignment="true" startWithSAP="1">
      <Representation id="a" codecs="mp4a.40.2" bandwidth="1" audioSamplingRate="48000">
        <AudioChannelConfiguration schemeIdUri="urn:mpeg:dash:23003:3:audio_channel_configuration:2011" value="2"/>
        <SegmentTemplate timescale="48000" initialization="Demo_audio_init.mp4" media="Demo_audio_$Number%05d$.m4s" startNumber="0">
          <SegmentTimeline>
            <S t="1024" d="288000" r="1"/>
            <S d="48000"/>
          </SegmentTimeline>
        </SegmentTemplate>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>
"#;
        assert_eq!(mpd, want);
    }

    #[test]
    fn names_are_escaped_and_odd_input_does_not_panic() {
        let mut v = video(vec![]);
        v.init = "A&B <\"$1\">_init.mp4".into();
        v.segment_prefix = "A&B $x_".into();
        v.media = DashMedia::Video { width: 0, height: 0, frame_rate: (0, 0) };
        v.timescale = 0;
        let mpd = dash_mpd(f64::NAN, &[v]);
        assert!(mpd.contains("initialization=\"A&amp;B &lt;&quot;$$1&quot;&gt;_init.mp4\""), "{mpd}");
        assert!(mpd.contains("media=\"A&amp;B $$x_$Number%05d$.m4s\""), "{mpd}");
        assert!(mpd.contains("mediaPresentationDuration=\"PT0S\" minBufferTime=\"PT1S\""), "{mpd}");
        assert!(mpd.contains("timescale=\"1\"") && !mpd.contains("frameRate"), "{mpd}");
        assert!(mpd.contains("<SegmentTimeline>\n          </SegmentTimeline>"), "{mpd}");
        // integer frame rates are written without a denominator; no sets at all is still an MPD
        let mut v = video(vec![1]);
        v.media = DashMedia::Video { width: 64, height: 36, frame_rate: (25, 1) };
        assert!(dash_mpd(1.5, &[v]).contains("frameRate=\"25\""));
        let empty = dash_mpd(1.5, &[]);
        assert!(empty.contains("mediaPresentationDuration=\"PT1.5S\"") && !empty.contains("AdaptationSet") && empty.ends_with("</MPD>\n"), "{empty}");
    }
}
