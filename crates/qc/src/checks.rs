//! Turn [`Facts`] into check results: input (4.1), integrity (4.2), audio (4.3) and picture
//! (4.4).
//!
//! Every check the profile lists gets a result. A check that cannot judge (no render plan, a
//! container without sample-level analysis, no decoder) is NOT_RUN with the reason: never PASS on
//! missing evidence.

use serde_json::json;

use cfa4_color::Transfer;
use cfa4_time::FrameRate;

use crate::audio::{self, AudioFacts};
use crate::expect::{Expectation, label_matches};
use crate::inspect::{Facts, Findings, Found, StreamDecode};
use crate::picture::{self, PictureFacts};
use crate::profile::{CheckSpec, QcProfile};
use crate::report::{CheckResult, Event, Position, Status};

/// Why the reference comparison does not run yet: the preview references come from the engine.
const NO_REFERENCES: &str = "no preview reference frames given (the engine supplies them with the export, QC stage E)";

fn position(f: &Found, rate: Option<FrameRate>, frame_index: bool) -> Position {
    let tick = f.ms.map(|ms| cfa4_time::Tick::from_seconds_f64(ms / 1000.0).0);
    let mut p = Position { frame: frame_index.then_some(f.index), tick, timecode: None };
    if let Some(r) = rate.filter(|r| r.num > 0 && r.den > 0) {
        let frame = if frame_index { i64::try_from(f.index).ok() } else { f.ms.map(|ms| (ms / 1000.0 * r.num as f64 / r.den as f64).floor() as i64) };
        p.timecode = frame.map(|fr| cfa4_time::format_timecode_frames(fr, r, false));
    }
    p
}

fn events(list: &Findings, rate: Option<FrameRate>, frame_index: bool, prefix: &str) -> (Vec<Event>, u64) {
    let ev = list
        .kept
        .iter()
        .map(|f| Event {
            at: position(f, rate, frame_index),
            end: f.end_ms.map(|ms| position(&Found { index: 0, ms: Some(ms), end_ms: None, message: String::new() }, rate, false)),
            message: format!("{prefix}{}", f.message),
        })
        .collect();
    (ev, list.total.saturating_sub(list.kept.len() as u64))
}

/// A findings result: PASS without events, `status_if_any` with them (also when only dropped
/// events remain).
fn judged(id: &str, spec: &CheckSpec, status_if_any: Status, ev: Vec<Event>, dropped: u64) -> CheckResult {
    let mut r = CheckResult::findings(id, spec, status_if_any, ev);
    if dropped > 0 {
        r.status = status_if_any;
        r.events_dropped = r.events_dropped.saturating_add(dropped);
    }
    r
}

fn finding(msg: impl Into<String>) -> Event {
    Event { at: Position::default(), end: None, message: msg.into() }
}

struct Ctx<'a> {
    facts: &'a Facts,
    expect: &'a Expectation,
    rate: Option<FrameRate>,
}

impl Ctx<'_> {
    fn not_open(&self) -> Option<String> {
        self.facts.open_error.as_ref().map(|e| format!("the file could not be opened: {e}"))
    }

    /// The reason sample-level checks cannot run, if any.
    fn no_samples(&self) -> Option<String> {
        if let Some(e) = self.not_open() {
            return Some(e);
        }
        if let Some(e) = &self.facts.mp4_error {
            return Some(format!("the MP4 / MOV index could not be read: {e}"));
        }
        if self.facts.mp4.is_none() {
            let container = self.facts.info.as_ref().map_or("unknown", |i| i.container.as_str());
            return Some(format!("sample-level analysis covers MP4 / MOV only (this file: {container})"));
        }
        None
    }

    fn no_decode(&self) -> Option<String> {
        if let Some(r) = self.no_samples() {
            return Some(r);
        }
        if self.facts.decode.is_none() {
            return Some("the full decode did not run".into());
        }
        None
    }

    fn expects_video(&self) -> bool {
        self.expect.video.unwrap_or_else(|| self.facts.info.as_ref().is_some_and(|i| i.video.is_some()))
    }
}

fn probe_open(c: &Ctx, id: &str, spec: &CheckSpec) -> CheckResult {
    if let Some(e) = &c.facts.open_error {
        return CheckResult::findings(id, spec, Status::Fail, vec![finding(format!("cannot open: {e}"))]);
    }
    let Some(info) = &c.facts.info else { return CheckResult::not_run(id, spec, "no media information") };
    let mut r = CheckResult::findings(id, spec, Status::Fail, vec![]);
    r.detail.insert("container".into(), json!(info.container));
    r.detail.insert("kind".into(), json!(info.kind));
    r.detail.insert("videoStreams".into(), json!(usize::from(info.video.is_some())));
    r.detail.insert("audioStreams".into(), json!(info.audio_streams.len()));
    if let Some(m) = &c.facts.mp4 {
        r.detail.insert("subtitleStreams".into(), json!(m.subtitles));
    }
    r
}

fn probe_video(c: &Ctx, id: &str, spec: &CheckSpec) -> CheckResult {
    if let Some(e) = c.not_open() {
        return CheckResult::not_run(id, spec, &e);
    }
    let Some(info) = &c.facts.info else { return CheckResult::not_run(id, spec, "no media information") };
    let Some(v) = &info.video else {
        return if c.expects_video() {
            CheckResult::findings(id, spec, Status::Fail, vec![finding("no video stream")])
        } else {
            CheckResult::findings(id, spec, Status::Fail, vec![]).with_reason("no video stream, none expected")
        };
    };
    let mut ev = Vec::new();
    if c.expect.video == Some(false) {
        ev.push(finding("a video stream that the render plan does not expect"));
    }
    if v.width == 0 || v.height == 0 {
        ev.push(finding(format!("frame size {}x{}", v.width, v.height)));
    }
    if v.frame_rate.num <= 0 || v.frame_rate.den <= 0 {
        ev.push(finding("no usable frame rate"));
    }
    if let Some(w) = c.expect.width.filter(|w| *w != v.width) {
        ev.push(finding(format!("width {} instead of {w}", v.width)));
    }
    if let Some(h) = c.expect.height.filter(|h| *h != v.height) {
        ev.push(finding(format!("height {} instead of {h}", v.height)));
    }
    if let Some(r) = c.expect.frame_rate
        && (i128::from(r.num) * i128::from(v.frame_rate.den) != i128::from(v.frame_rate.num) * i128::from(r.den))
    {
        ev.push(finding(format!("frame rate {}/{} instead of {}/{}", v.frame_rate.num, v.frame_rate.den, r.num, r.den)));
    }
    if let Some(want) = c.expect.video_codec.as_deref().filter(|w| !label_matches(&v.codec, w)) {
        ev.push(finding(format!("codec {} instead of {want}", v.codec)));
    }
    if c.facts.mp4.as_ref().and_then(|m| m.video.as_ref()).is_some_and(|m| m.timescale == 0) {
        ev.push(finding("the video track has no time base"));
    }
    let mut r = CheckResult::findings(id, spec, Status::Fail, ev);
    r.detail.insert("codec".into(), json!(v.codec));
    r.detail.insert("width".into(), json!(v.width));
    r.detail.insert("height".into(), json!(v.height));
    r.detail.insert("frameRate".into(), json!(format!("{}/{}", v.frame_rate.num, v.frame_rate.den)));
    r.detail.insert("pixelAspect".into(), json!(format!("{}:{}", v.par.0, v.par.1)));
    r.detail.insert("pixelFormat".into(), json!(v.pixel_format));
    if let Some(b) = v.bitrate {
        r.detail.insert("bitrate".into(), json!(b));
    }
    if let Some(m) = c.facts.mp4.as_ref().and_then(|m| m.video.as_ref()) {
        r.detail.insert("timeBase".into(), json!(format!("1/{}", m.timescale)));
    }
    r
}

fn probe_color(c: &Ctx, id: &str, spec: &CheckSpec) -> CheckResult {
    if let Some(e) = c.not_open() {
        return CheckResult::not_run(id, spec, &e);
    }
    let Some(v) = c.facts.info.as_ref().and_then(|i| i.video.as_ref()) else { return CheckResult::not_run(id, spec, "no video stream") };
    let mut ev = Vec::new();
    if v.color.transfer == Transfer::Pq && v.hdr.is_none() {
        ev.push(finding("PQ (HDR10) transfer without mastering display / content light metadata"));
    }
    let mut r = CheckResult::findings(id, spec, Status::Warn, ev);
    r.detail.insert("primaries".into(), json!(format!("{:?}", v.color.primaries)));
    r.detail.insert("transfer".into(), json!(format!("{:?}", v.color.transfer)));
    r.detail.insert("matrix".into(), json!(format!("{:?}", v.color.matrix)));
    r.detail.insert("range".into(), json!(format!("{:?}", v.color.range)));
    if let Some(h) = &v.hdr {
        r.detail.insert("hdr".into(), json!(h));
    }
    r
}

fn probe_audio(c: &Ctx, id: &str, spec: &CheckSpec) -> CheckResult {
    if let Some(e) = c.not_open() {
        return CheckResult::not_run(id, spec, &e);
    }
    let Some(info) = &c.facts.info else { return CheckResult::not_run(id, spec, "no media information") };
    let mut ev = Vec::new();
    if info.audio_streams.is_empty() && c.expect.audio_tracks.is_some_and(|n| n > 0) {
        ev.push(finding("no audio stream"));
    }
    for (i, a) in info.audio_streams.iter().enumerate() {
        if a.sample_rate == 0 {
            ev.push(finding(format!("audio stream {i}: no sample rate")));
        }
        if a.channels == 0 {
            ev.push(finding(format!("audio stream {i}: no channels")));
        }
        if let Some(sr) = c.expect.sample_rate.filter(|sr| *sr != a.sample_rate) {
            ev.push(finding(format!("audio stream {i}: {} Hz instead of {sr} Hz", a.sample_rate)));
        }
        if let Some(ch) = c.expect.audio_channels.filter(|ch| *ch != a.channels) {
            ev.push(finding(format!("audio stream {i}: {} channel(s) instead of {ch}", a.channels)));
        }
        if let Some(want) = c.expect.audio_codec.as_deref().filter(|w| !label_matches(&a.codec, w)) {
            ev.push(finding(format!("audio stream {i}: codec {} instead of {want}", a.codec)));
        }
    }
    let mut r = CheckResult::findings(id, spec, Status::Fail, ev);
    if info.audio_streams.is_empty() && c.expect.audio_tracks.is_none_or(|n| n == 0) {
        r = r.with_reason("no audio stream, none expected");
    }
    let streams: Vec<_> =
        info.audio_streams.iter().map(|a| json!({"codec": a.codec, "sampleRate": a.sample_rate, "channels": a.channels, "bits": a.bits_per_sample})).collect();
    r.detail.insert("streams".into(), json!(streams));
    r
}

fn probe_consistency(c: &Ctx, id: &str, spec: &CheckSpec) -> CheckResult {
    if let Some(e) = c.not_open() {
        return CheckResult::not_run(id, spec, &e);
    }
    let Some(info) = &c.facts.info else { return CheckResult::not_run(id, spec, "no media information") };
    let mut fail = Vec::new();
    let mut warn = Vec::new();
    if info.duration.0 <= 0 {
        fail.push(finding("the file reports no duration"));
    }
    let Some(m) = &c.facts.mp4 else {
        let mut r = CheckResult::findings(id, spec, Status::Fail, fail);
        if r.status == Status::Pass {
            r = CheckResult::not_run(id, spec, c.no_samples().as_deref().unwrap_or("no container facts"));
        }
        return r;
    };
    if let Some(v) = &m.video {
        if v.samples == 0 {
            fail.push(finding("the video track has no samples"));
        }
        if v.entries > 1 {
            warn.push(finding(format!("the video codec setup changes mid-stream ({} sample descriptions)", v.entries)));
        }
        if v.samples > 1 && v.odd_durations * 100 > v.samples {
            warn.push(finding(format!("variable frame rate: {} of {} frame durations differ from the nominal one", v.odd_durations, v.samples)));
        }
        if !v.dts_backwards.is_empty() || !v.duplicate_pts.is_empty() {
            fail.push(finding("broken video timestamps (see integrity.frames)"));
        }
    }
    for a in &m.audio {
        if a.samples == 0 {
            fail.push(finding(format!("audio track {} has no samples", a.track)));
        }
        if let Some(info_a) = info.audio_streams.iter().find(|s| s.channels != a.channels && a.channels > 0 && s.channels > 0 && info.audio_streams.len() == 1)
        {
            warn.push(finding(format!("audio channels: the sample description says {}, the stream decodes {}", a.channels, info_a.channels)));
        }
    }
    let status = if !fail.is_empty() { Status::Fail } else { Status::Warn };
    let mut r = CheckResult::findings(id, spec, status, fail.into_iter().chain(warn).collect());
    r.detail.insert("fragmented".into(), json!(m.fragmented));
    r.detail.insert("quicktime".into(), json!(m.quicktime));
    if let Some(v) = &m.video {
        r.detail.insert("videoNominalFrameDuration".into(), json!(v.nominal_duration));
        r.detail.insert("videoOddFrameDurations".into(), json!(v.odd_durations));
    }
    r
}

fn decode_events(s: &StreamDecode, what: &str, rate: Option<FrameRate>, frame_index: bool) -> (Vec<Event>, u64) {
    let mut ev = Vec::new();
    let mut dropped = 0;
    if let Some(u) = &s.unsupported {
        ev.push(finding(format!("{what} track {}: cannot decode: {u}", s.track)));
    }
    let (e, d) = events(&s.errors, rate, frame_index, &format!("{what} track {}: ", s.track));
    ev.extend(e);
    dropped += d;
    (ev, dropped)
}

fn integrity_decode(c: &Ctx, id: &str, spec: &CheckSpec) -> CheckResult {
    if let Some(r) = c.no_decode() {
        return CheckResult::not_run(id, spec, &r);
    }
    let Some(d) = &c.facts.decode else { return CheckResult::not_run(id, spec, "the full decode did not run") };
    // a stream with no decoder means the file was not verified: NOT_RUN, never PASS
    let unsupported: Vec<&StreamDecode> = d.video.iter().chain(d.audio.iter()).filter(|s| s.unsupported.is_some()).collect();
    let mut ev = Vec::new();
    let mut dropped = 0;
    if let Some(v) = &d.video {
        let (e, n) = decode_events(v, "video", c.rate, true);
        ev.extend(e);
        dropped += n;
    }
    for a in &d.audio {
        let (e, n) = decode_events(a, "audio", c.rate, false);
        ev.extend(e);
        dropped += n;
    }
    let errors = d.video.iter().chain(d.audio.iter()).map(|s| s.errors.total).sum::<u64>();
    let mut r = if errors > 0 {
        judged(id, spec, Status::Fail, ev, dropped)
    } else if !unsupported.is_empty() {
        let why: Vec<String> = unsupported.iter().filter_map(|s| s.unsupported.clone().map(|u| format!("track {}: {u}", s.track))).collect();
        CheckResult::not_run(id, spec, &format!("no decoder: {}", why.join("; ")))
    } else {
        CheckResult::findings(id, spec, Status::Fail, vec![])
    };
    let streams: Vec<_> =
        d.video
            .iter()
            .map(|s| json!({"track": s.track, "kind": "video", "decoder": s.decoder, "samples": s.samples, "decoded": s.decoded, "errors": s.errors.total}))
            .chain(d.audio.iter().map(
                |s| json!({"track": s.track, "kind": "audio", "decoder": s.decoder, "samples": s.samples, "decoded": s.decoded, "errors": s.errors.total}),
            ))
            .collect();
    r.detail.insert("streams".into(), json!(streams));
    r
}

fn integrity_frame_count(c: &Ctx, id: &str, spec: &CheckSpec) -> CheckResult {
    let Some(want) = c.expect.frames else { return CheckResult::not_run(id, spec, "no render plan given (expected frame count)") };
    if let Some(r) = c.no_decode() {
        return CheckResult::not_run(id, spec, &r);
    }
    let Some(v) = c.facts.decode.as_ref().and_then(|d| d.video.as_ref()) else {
        return CheckResult::not_run(id, spec, "no video stream to count");
    };
    if v.unsupported.is_some() {
        return CheckResult::not_run(id, spec, "the video stream could not be decoded");
    }
    let got = i128::from(v.decoded);
    let diff = (got - i128::from(want)) as f64;
    let mut r = CheckResult::measured(id, spec, diff);
    r.detail.insert("decodedFrames".into(), json!(v.decoded));
    r.detail.insert("plannedFrames".into(), json!(want));
    r.detail.insert("containerSamples".into(), json!(v.samples));
    r
}

fn integrity_duration(c: &Ctx, id: &str, spec: &CheckSpec) -> CheckResult {
    let Some(want) = c.expect.duration_ms() else { return CheckResult::not_run(id, spec, "no render plan given (frames and frame rate)") };
    if let Some(r) = c.no_samples() {
        return CheckResult::not_run(id, spec, &r);
    }
    let Some(v) = c.facts.mp4.as_ref().and_then(|m| m.video.as_ref()) else { return CheckResult::not_run(id, spec, "no video track") };
    let got = v.end_ms - v.start_ms;
    let mut r = CheckResult::measured(id, spec, got - want);
    r.detail.insert("durationMs".into(), json!(got));
    r.detail.insert("plannedMs".into(), json!(want));
    r
}

fn integrity_frames(c: &Ctx, id: &str, spec: &CheckSpec) -> CheckResult {
    if let Some(r) = c.no_decode() {
        return CheckResult::not_run(id, spec, &r);
    }
    let Some(m) = &c.facts.mp4 else { return CheckResult::not_run(id, spec, "no container facts") };
    let Some(v) = &m.video else { return CheckResult::not_run(id, spec, "no video track") };
    let Some(d) = &c.facts.decode else { return CheckResult::not_run(id, spec, "the full decode did not run") };
    let Some(vd) = &d.video else { return CheckResult::not_run(id, spec, "no video stream decoded") };
    if vd.unsupported.is_some() {
        return CheckResult::not_run(id, spec, "the video stream could not be decoded");
    }
    let mut fail = Vec::new();
    let mut dropped = 0;
    for (list, frame_index) in [(&v.dts_backwards, false), (&v.duplicate_pts, true)] {
        let (e, n) = events(list, c.rate, frame_index, "");
        fail.extend(e);
        dropped += n;
    }
    // pictures the decoder did not return (damaged or missing in the stream)
    if vd.errors.is_empty() && vd.decoded < vd.samples as u64 {
        fail.push(finding(format!("{} of {} pictures were not returned by the decoder", vd.samples as u64 - vd.decoded, vd.samples)));
    }
    if d.size_changes > 0 {
        fail.push(finding(format!("the picture size changes {} time(s)", d.size_changes)));
    }
    let (warn, n) = events(&v.gaps, c.rate, true, "");
    dropped += n;
    let status = if !fail.is_empty() { Status::Fail } else { Status::Warn };
    let mut r = judged(id, spec, status, fail.into_iter().chain(warn).collect(), dropped);
    r.detail.insert("longestRepeatRun".into(), json!(d.longest_repeat));
    r.detail.insert("repeatedPictures".into(), json!(d.repeated));
    if let Some((w, h)) = d.decoded_size {
        r.detail.insert("decodedSize".into(), json!(format!("{w}x{h}")));
    }
    r
}

fn integrity_av(c: &Ctx, id: &str, spec: &CheckSpec) -> CheckResult {
    if let Some(r) = c.no_samples() {
        return CheckResult::not_run(id, spec, &r);
    }
    let Some(m) = &c.facts.mp4 else { return CheckResult::not_run(id, spec, "no container facts") };
    let Some(v) = &m.video else { return CheckResult::not_run(id, spec, "no video track") };
    let Some(a) = m.audio.first() else { return CheckResult::not_run(id, spec, "no audio track") };
    let mut r = CheckResult::measured(id, spec, a.end_ms - v.end_ms);
    r.detail.insert("videoStartMs".into(), json!(v.start_ms));
    r.detail.insert("videoEndMs".into(), json!(v.end_ms));
    r.detail.insert("audioStartMs".into(), json!(a.start_ms));
    r.detail.insert("audioEndMs".into(), json!(a.end_ms));
    if (a.start_ms - v.start_ms).abs() > 0.5 {
        r.push_event(finding(format!("audio starts {:.1} ms after video", a.start_ms - v.start_ms)));
    }
    r
}

fn integrity_streams(c: &Ctx, id: &str, spec: &CheckSpec) -> CheckResult {
    if !c.expect.has_stream_plan() {
        return CheckResult::not_run(id, spec, "no render plan given (expected streams)");
    }
    if let Some(e) = c.not_open() {
        return CheckResult::not_run(id, spec, &e);
    }
    let Some(info) = &c.facts.info else { return CheckResult::not_run(id, spec, "no media information") };
    let mut ev = Vec::new();
    if let Some(want) = c.expect.video
        && want != info.video.is_some()
    {
        ev.push(finding(if want { "the video stream is missing" } else { "an unexpected video stream" }));
    }
    if let Some(n) = c.expect.audio_tracks.filter(|n| *n as usize != info.audio_streams.len()) {
        ev.push(finding(format!("{} audio track(s) instead of {n}", info.audio_streams.len())));
    }
    if let Some(ch) = c.expect.audio_channels {
        for (i, a) in info.audio_streams.iter().enumerate().filter(|(_, a)| a.channels != ch) {
            ev.push(finding(format!("audio track {i}: {} channel(s) instead of {ch}", a.channels)));
        }
    }
    if let Some(n) = c.expect.subtitle_streams {
        match &c.facts.mp4 {
            Some(m) if m.subtitles != n as usize => ev.push(finding(format!("{} subtitle stream(s) instead of {n}", m.subtitles))),
            Some(_) => {}
            None if n > 0 => return CheckResult::not_run(id, spec, "subtitle streams are counted in MP4 / MOV only"),
            None => {}
        }
    }
    CheckResult::findings(id, spec, Status::Fail, ev)
}

/// Worst first: a failure outweighs a missing measurement, which outweighs a warning.
fn rank(s: Status) -> u8 {
    match s {
        Status::Fail => 3,
        Status::NotRun => 2,
        Status::Warn => 1,
        Status::Pass => 0,
    }
}

impl Ctx<'_> {
    /// The render plan rules out audio and the file has none.
    fn no_audio_expected(&self) -> bool {
        let none_planned = self.expect.audio_tracks == Some(0);
        none_planned && self.facts.info.as_ref().is_some_and(|i| i.audio_streams.is_empty())
    }

    /// The render plan rules out video and the file has none.
    fn no_video_expected(&self) -> bool {
        self.expect.video == Some(false) && self.facts.info.as_ref().is_some_and(|i| i.video.is_none())
    }

    /// The audio analysis of every track, or why the audio checks cannot run.
    fn audio(&self) -> Result<&[AudioFacts], String> {
        if let Some(r) = self.no_decode() {
            return Err(r);
        }
        let Some(d) = &self.facts.decode else { return Err("the full decode did not run".into()) };
        if d.audio.is_empty() {
            return Err("no audio track".into());
        }
        let unsupported: Vec<String> = d.audio.iter().filter_map(|s| s.unsupported.as_ref().map(|u| format!("track {}: {u}", s.track))).collect();
        if !unsupported.is_empty() {
            return Err(format!("no decoder: {}", unsupported.join("; ")));
        }
        if d.audio_facts.len() < d.audio.len() || d.audio_facts.iter().any(|a| a.frames == 0) {
            return Err("an audio track decoded no samples".into());
        }
        Ok(&d.audio_facts)
    }

    /// The picture analysis, or why the picture checks cannot run.
    fn picture(&self) -> Result<&PictureFacts, String> {
        if let Some(r) = self.no_decode() {
            return Err(r);
        }
        let Some(d) = &self.facts.decode else { return Err("the full decode did not run".into()) };
        let Some(v) = &d.video else { return Err("no video stream".into()) };
        if let Some(u) = &v.unsupported {
            return Err(format!("no decoder: {u}"));
        }
        match &d.picture {
            Some(p) if p.analysed > 0 => Ok(p),
            _ => Err("no picture was decoded".into()),
        }
    }
}

/// An audio check measured per track: the worst track gives the result, every track is listed.
fn per_track(c: &Ctx, id: &str, spec: &CheckSpec, measure: impl Fn(&AudioFacts) -> Result<(f64, Option<String>), String>) -> CheckResult {
    if c.no_audio_expected() {
        return CheckResult::not_applicable(id, spec, "the render plan has no audio and the file has none");
    }
    let tracks = match c.audio() {
        Ok(t) => t,
        Err(r) => return CheckResult::not_run(id, spec, &r),
    };
    let mut worst: Option<CheckResult> = None;
    let mut listed = Vec::new();
    for a in tracks {
        let r = match measure(a) {
            Ok((v, note)) => {
                let mut r = CheckResult::measured(id, spec, v);
                if let Some(n) = note {
                    r.push_event(finding(format!("audio track {}: {n}", a.track)));
                }
                r
            }
            Err(why) => CheckResult::not_run(id, spec, &format!("audio track {}: {why}", a.track)),
        };
        listed.push(json!({"track": a.track, "value": r.value, "status": r.status, "reason": r.reason}));
        if worst.as_ref().is_none_or(|w| rank(r.status) > rank(w.status)) {
            worst = Some(r);
        }
    }
    let mut r = worst.unwrap_or_else(|| CheckResult::not_run(id, spec, "no audio track"));
    r.detail.insert("tracks".into(), json!(listed));
    r
}

/// Lowest level reported for silence, in dB (the measurement itself is −∞).
const FLOOR_DB: f64 = -150.0;

fn audio_loudness(c: &Ctx, id: &str, spec: &CheckSpec) -> CheckResult {
    per_track(c, id, spec, |a| {
        if a.seconds() < audio::MIN_LOUDNESS_S {
            return Err(format!("shorter than {} s: no loudness gating block", audio::MIN_LOUDNESS_S));
        }
        if a.integrated_lufs.is_finite() {
            Ok((a.integrated_lufs, None))
        } else if a.integrated_lufs == f64::NEG_INFINITY {
            Ok((-70.0, Some("no signal above the −70 LUFS gate (reported as −70 LUFS)".into())))
        } else {
            Err(format!("loudness is not a number ({}): samples that are not finite", a.integrated_lufs))
        }
    })
}

fn audio_range(c: &Ctx, id: &str, spec: &CheckSpec) -> CheckResult {
    per_track(c, id, spec, |a| {
        if a.seconds() < audio::MIN_RANGE_S {
            return Err(format!("shorter than {} s: no short-term loudness window", audio::MIN_RANGE_S));
        }
        Ok((a.loudness_range_lu, None))
    })
}

fn audio_true_peak(c: &Ctx, id: &str, spec: &CheckSpec) -> CheckResult {
    per_track(c, id, spec, |a| {
        if a.true_peak_dbtp.is_finite() {
            Ok((a.true_peak_dbtp.max(FLOOR_DB), None))
        } else if a.true_peak_dbtp == f64::NEG_INFINITY {
            Ok((FLOOR_DB, Some(format!("digital silence (reported as {FLOOR_DB} dBTP)"))))
        } else {
            // +∞ or NaN: corrupt (infinite or NaN) samples, never a passing value
            Err(format!("true peak is not a finite number ({}): samples that are not finite", a.true_peak_dbtp))
        }
    })
}

fn audio_clipping(c: &Ctx, id: &str, spec: &CheckSpec) -> CheckResult {
    if c.no_audio_expected() {
        return CheckResult::not_applicable(id, spec, "the render plan has no audio and the file has none");
    }
    let tracks = match c.audio() {
        Ok(t) => t,
        Err(r) => return CheckResult::not_run(id, spec, &r),
    };
    let total = tracks.iter().fold(0u64, |a, t| a.saturating_add(t.clipped));
    let mut r = CheckResult::measured(id, spec, total as f64);
    for t in tracks {
        let (ev, dropped) = events(&t.clip_runs, c.rate, false, &format!("audio track {}: ", t.track));
        ev.into_iter().for_each(|e| r.push_event(e));
        r.events_dropped = r.events_dropped.saturating_add(dropped);
    }
    let per: Vec<_> = tracks.iter().map(|t| json!({"track": t.track, "clipped": t.clipped, "samplePeakDbfs": finite(t.sample_peak_dbfs)})).collect();
    r.detail.insert("tracks".into(), json!(per));
    r.detail.insert("clipLevel".into(), json!(audio::CLIP_LEVEL));
    r
}

/// A level for the report: JSON has no −∞.
fn finite(db: f64) -> Option<f64> {
    db.is_finite().then_some(db)
}

fn audio_silence(c: &Ctx, id: &str, spec: &CheckSpec) -> CheckResult {
    if c.no_audio_expected() {
        return CheckResult::not_applicable(id, spec, "the render plan has no audio and the file has none");
    }
    let tracks = match c.audio() {
        Ok(t) => t,
        Err(r) => return CheckResult::not_run(id, spec, &r),
    };
    let mut fail = Vec::new();
    let mut warn = Vec::new();
    let mut dropped = 0;
    for t in tracks {
        if t.sample_peak_dbfs.is_nan() || t.sample_peak_dbfs < audio::SILENCE_DBFS {
            fail.push(finding(format!("audio track {} is silent (peak below {} dBFS)", t.track, audio::SILENCE_DBFS)));
            continue;
        }
        let (ev, n) = events(&t.silence, c.rate, false, &format!("audio track {}: ", t.track));
        warn.extend(ev);
        dropped += n;
    }
    let status = if fail.is_empty() { Status::Warn } else { Status::Fail };
    let mut r = judged(id, spec, status, fail.into_iter().chain(warn).collect(), dropped);
    r.detail.insert("thresholdDbfs".into(), json!(audio::SILENCE_DBFS));
    r.detail.insert("minimumSeconds".into(), json!(audio::SILENCE_MIN_S));
    r
}

fn audio_layout(c: &Ctx, id: &str, spec: &CheckSpec) -> CheckResult {
    if c.no_audio_expected() {
        return CheckResult::not_applicable(id, spec, "the render plan has no audio and the file has none");
    }
    let tracks = match c.audio() {
        Ok(t) => t,
        Err(r) => return CheckResult::not_run(id, spec, &r),
    };
    let mut fail = Vec::new();
    let mut warn = Vec::new();
    for t in tracks {
        if let Some(want) = c.expect.audio_channels.filter(|w| *w as usize != t.channels) {
            fail.push(finding(format!("audio track {}: {} decoded channel(s) instead of {want}", t.track, t.channels)));
        }
        // a channel without signal next to one with signal: a routing or layout mistake
        let loudest = t.channel_peak_dbfs.iter().copied().filter(|d| d.is_finite()).fold(f64::NEG_INFINITY, f64::max);
        if loudest >= audio::SILENCE_DBFS {
            for (ch, p) in t.channel_peak_dbfs.iter().enumerate().filter(|(_, p)| **p < audio::SILENCE_DBFS) {
                let level = if p.is_finite() { format!("{p:.1} dBFS") } else { "digital silence".into() };
                warn.push(finding(format!("audio track {}: channel {ch} is silent ({level}) while others carry signal", t.track)));
            }
        }
    }
    let status = if fail.is_empty() { Status::Warn } else { Status::Fail };
    let mut r = CheckResult::findings(id, spec, status, fail.into_iter().chain(warn).collect());
    if c.expect.audio_channels.is_none() && r.status == Status::Pass {
        r = r.with_reason("channel count not compared: no render plan given; channel levels checked");
    }
    let per: Vec<_> = tracks
        .iter()
        .map(|t| {
            json!({"track": t.track, "channels": t.channels, "sampleRate": t.sample_rate, "dualMono": t.identical_channels,
                "channelPeakDbfs": t.channel_peak_dbfs.iter().map(|d| finite(*d)).collect::<Vec<_>>()})
        })
        .collect();
    r.detail.insert("tracks".into(), json!(per));
    r
}

fn picture_ready<'a>(c: &'a Ctx, id: &str, spec: &CheckSpec) -> Result<&'a PictureFacts, Box<CheckResult>> {
    if c.no_video_expected() {
        return Err(Box::new(CheckResult::not_applicable(id, spec, "the render plan has no video and the file has none")));
    }
    c.picture().map_err(|r| Box::new(CheckResult::not_run(id, spec, &r)))
}

fn video_black(c: &Ctx, id: &str, spec: &CheckSpec) -> CheckResult {
    let p = match picture_ready(c, id, spec) {
        Ok(p) => p,
        Err(r) => return *r,
    };
    let mut r = if p.black_frames >= p.analysed {
        CheckResult::findings(id, spec, Status::Fail, vec![finding(format!("every picture is black ({} of {})", p.black_frames, p.analysed))])
    } else {
        // black at the very start or end is a fade or a slate; inside the programme it is a defect
        let inner: Vec<_> = p.black_runs.iter().filter(|b| b.first > 0 && b.first.saturating_add(b.frames) < p.analysed).copied().collect();
        let (ev, dropped) = events(&picture::black_findings(&inner), c.rate, true, "");
        let more = p.black_runs_total.saturating_sub(p.black_runs.len() as u64);
        judged(id, spec, Status::Warn, ev, dropped.saturating_add(more))
    };
    let head = p.black_runs.first().filter(|b| b.first == 0).map_or(0, |b| b.frames);
    let tail = p.black_runs.last().filter(|b| b.first > 0 && b.first.saturating_add(b.frames) >= p.analysed).map_or(0, |b| b.frames);
    r.detail.insert("blackFrames".into(), json!(p.black_frames));
    r.detail.insert("headBlackFrames".into(), json!(head));
    r.detail.insert("tailBlackFrames".into(), json!(tail));
    r.detail.insert("pixelThreshold".into(), json!(picture::BLACK_PIXEL));
    r.detail.insert("pictureThreshold".into(), json!(picture::BLACK_PICTURE));
    r
}

fn video_clip(c: &Ctx, id: &str, spec: &CheckSpec, white: bool) -> CheckResult {
    let p = match picture_ready(c, id, spec) {
        Ok(p) => p,
        Err(r) => return *r,
    };
    if p.luma_samples == 0 {
        return CheckResult::not_run(id, spec, "no luma samples");
    }
    let (n, worst) = if white { (p.above_white, p.worst_white) } else { (p.below_black, p.worst_black) };
    let mut r = CheckResult::measured(id, spec, n as f64 * 100.0 / p.luma_samples as f64);
    if let Some((frame, pct, ms)) = worst {
        let at = position(&Found { index: frame, ms, end_ms: None, message: String::new() }, c.rate, true);
        r.push_event(Event { at, end: None, message: format!("worst picture: {pct:.2} % of its pixels") });
    }
    let (_, (under, over)) = picture::limits(p.bits, p.full_range);
    let limit = if white { format!("above {over:.1}") } else { format!("below {under:.1}") };
    r.detail.insert("range".into(), json!(if p.full_range { "full" } else { "limited" }));
    r.detail.insert("bits".into(), json!(p.bits));
    r.detail.insert("limit".into(), json!(limit));
    r.detail.insert("pictures".into(), json!(p.analysed));
    r
}

fn video_contact_sheet(c: &Ctx, id: &str, spec: &CheckSpec) -> CheckResult {
    let p = match picture_ready(c, id, spec) {
        Ok(p) => p,
        Err(r) => return *r,
    };
    let Some(s) = &p.sheet else { return CheckResult::not_run(id, spec, p.sheet_error.as_deref().unwrap_or("no contact sheet")) };
    let mut r = CheckResult::findings(id, spec, Status::Fail, vec![]);
    let frames: Vec<_> = s
        .frames
        .iter()
        .map(|(f, ms)| {
            let at = position(&Found { index: *f, ms: *ms, end_ms: None, message: String::new() }, c.rate, true);
            json!({"frame": f, "timecode": at.timecode})
        })
        .collect();
    r.detail.insert("frames".into(), json!(frames));
    r.detail.insert("columns".into(), json!(s.columns));
    r.detail.insert("width".into(), json!(s.width));
    r.detail.insert("height".into(), json!(s.height));
    r.detail.insert("sha256".into(), json!(crate::hash::sha256_hex(&s.png)));
    r
}

/// A result for every check the profile lists.
pub fn evaluate(profile: &QcProfile, facts: &Facts, expect: &Expectation) -> Vec<CheckResult> {
    let rate = facts.info.as_ref().and_then(|i| i.video.as_ref()).map(|v| v.frame_rate);
    let c = Ctx { facts, expect, rate };
    profile
        .checks
        .iter()
        .map(|(id, spec)| match id.as_str() {
            "probe.open" => probe_open(&c, id, spec),
            "probe.video" => probe_video(&c, id, spec),
            "probe.color" => probe_color(&c, id, spec),
            "probe.audio" => probe_audio(&c, id, spec),
            "probe.consistency" => probe_consistency(&c, id, spec),
            "integrity.decode" => integrity_decode(&c, id, spec),
            "integrity.frameCount" => integrity_frame_count(&c, id, spec),
            "integrity.duration" => integrity_duration(&c, id, spec),
            "integrity.frames" => integrity_frames(&c, id, spec),
            "integrity.avOffset" => integrity_av(&c, id, spec),
            "integrity.streams" => integrity_streams(&c, id, spec),
            "audio.silence" => audio_silence(&c, id, spec),
            "audio.clipping" => audio_clipping(&c, id, spec),
            "audio.truePeak" => audio_true_peak(&c, id, spec),
            "audio.loudness" => audio_loudness(&c, id, spec),
            "audio.loudnessRange" => audio_range(&c, id, spec),
            "audio.layout" => audio_layout(&c, id, spec),
            "video.contactSheet" => video_contact_sheet(&c, id, spec),
            "video.black" => video_black(&c, id, spec),
            "video.clipBlack" => video_clip(&c, id, spec, false),
            "video.clipWhite" => video_clip(&c, id, spec, true),
            "video.referenceSsim" => CheckResult::not_run(id, spec, NO_REFERENCES),
            _ => CheckResult::not_run(id, spec, "unknown check"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspect::{DecodeFacts, Mp4Facts, TrackFacts};

    fn facts(unsupported: Option<&str>) -> Facts {
        let video = StreamDecode { track: 0, samples: 24, unsupported: unsupported.map(str::to_string), ..StreamDecode::default() };
        Facts {
            info: Some(cfa4_media::MediaInfo {
                name: "x.mp4".into(),
                kind: cfa4_media::MediaKind::Movie,
                duration: cfa4_time::Tick::from_seconds_f64(1.0),
                video: None,
                audio_streams: vec![],
                container: "MPEG-4".into(),
                start_timecode: None,
                file_size: None,
            }),
            mp4: Some(Mp4Facts { video: Some(TrackFacts { samples: 24, timescale: 24, ..TrackFacts::default() }), ..Mp4Facts::default() }),
            decode: Some(DecodeFacts { video: Some(video), ..DecodeFacts::default() }),
            ..Facts::default()
        }
    }

    /// An AV1 (or any) stream this build has no decoder for: NOT_RUN with the reason, never PASS.
    #[test]
    fn missing_decoder_is_not_run() {
        let p = QcProfile::builtin("delivery-web").unwrap();
        let expect = Expectation { frames: Some(24), ..Expectation::default() };
        let rs = evaluate(&p, &facts(Some("no decoder for av1 video")), &expect);
        let get = |id: &str| rs.iter().find(|r| r.id == id).unwrap();
        assert_eq!(get("integrity.decode").status, Status::NotRun);
        assert!(get("integrity.decode").reason.as_deref().unwrap().contains("no decoder for av1"));
        assert_eq!(get("integrity.frameCount").status, Status::NotRun);
        assert_eq!(get("integrity.frames").status, Status::NotRun);
        // the same facts with a working decoder that returned nothing: a failure, not a pass
        let rs = evaluate(&p, &facts(None), &expect);
        let get = |id: &str| rs.iter().find(|r| r.id == id).unwrap();
        assert_eq!(get("integrity.frameCount").status, Status::Fail);
        assert_eq!(get("integrity.frames").status, Status::Fail);
    }

    /// Review of #306: an infinite or NaN measurement (samples that are not finite) is never
    /// turned into a passing value; only −∞ (digital silence) maps to the floor.
    #[test]
    fn non_finite_audio_measurements_do_not_pass() {
        let p = QcProfile::builtin("delivery-web").unwrap();
        let mut a = crate::audio::AudioAnalyzer::new(0, 48_000, 0, None, 0.0);
        a.push(&[vec![0.1; 48_000], vec![0.1; 48_000]]);
        let tone = a.finish();
        for (peak, lufs, want) in
            [(f64::INFINITY, f64::INFINITY, Status::NotRun), (f64::NAN, f64::NAN, Status::NotRun), (f64::NEG_INFINITY, f64::NEG_INFINITY, Status::Pass)]
        {
            let mut f = facts(None);
            let mut t = tone.clone();
            t.true_peak_dbtp = peak;
            t.integrated_lufs = lufs;
            if let Some(d) = f.decode.as_mut() {
                d.audio = vec![StreamDecode { track: 1, samples: 47, decoded: 47, ..StreamDecode::default() }];
                d.audio_facts = vec![t];
            }
            let rs = evaluate(&p, &f, &Expectation::default());
            let r = rs.iter().find(|r| r.id == "audio.truePeak").unwrap();
            assert_eq!(r.status, want, "{peak}: {r:?}");
            let l = rs.iter().find(|r| r.id == "audio.loudness").unwrap();
            assert_ne!(l.status, Status::Pass, "{lufs}: {l:?}");
        }
    }

    #[test]
    fn audio_checks_need_decoded_audio() {
        let p = QcProfile::builtin("delivery-web").unwrap();
        let rs = evaluate(&p, &facts(None), &Expectation::default());
        let loud = rs.iter().find(|r| r.id == "audio.loudness").unwrap();
        assert_eq!((loud.status, loud.reason.as_deref()), (Status::NotRun, Some("no audio track")));
        // a video-only plan and a file without audio: the audio checks do not apply
        let rs = evaluate(&p, &facts(None), &Expectation { audio_tracks: Some(0), ..Expectation::default() });
        for id in ["audio.loudness", "audio.truePeak", "audio.clipping", "audio.layout"] {
            let r = rs.iter().find(|r| r.id == id).unwrap();
            assert_eq!(r.status, Status::Pass, "{id}");
            assert!(r.reason.as_deref().unwrap().starts_with(crate::report::NOT_APPLICABLE), "{id}");
        }
    }
}
