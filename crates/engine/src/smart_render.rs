//! Smart Render of a sequence (CFA4-SMART-CUT-RENDER-R1, WP-06; CFA4-SCE-001 v1.5 §11): the
//! `export.plan` and `export.start` commands with `"scope": "timeline"`.
//!
//! The sequence's video is turned into an [`EditDecisionList`]: each output range is either a gap,
//! one clip shown as its source pictures are, or a range the timeline changes. A range is shown as
//! its source pictures only when nothing could change them: exactly one clip is visible, it is an
//! H.264 MP4 / MOV file at the sequence's frame size, pixel aspect and frame rate, in Rec. 709
//! with a plain colour pipeline, at 100 % speed, with every effect at its defaults (Motion, Opacity
//! and Time Remapping included), no transition, no Interpret Footage override, on the frame grid.
//! Anything else is rendered ([`RenderCause`]): the planner (WP-05) copies the whole GOPs of the
//! unchanged ranges and re-encodes their boundaries, and the timeline executor (WP-06,
//! `cfa4_export::timeline_render`) renders the rest through the export pipeline.
//!
//! The output is the sequence's video only (no audio), from its start to the end of its last video
//! clip. As with a Smart Cut, `export.start` re-encodes or renders only with `"confirm": true`, and
//! writes the file only after it was read back and checked.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::sync::Arc;

use cfa4_export::audio_copy::{mux_audio_tracks, verify_audio_tracks};
use cfa4_export::boundary_reencode::BoundaryEncoding;
use cfa4_export::smart_cut::{Mp4CopySource, SmartCutError};
use cfa4_export::timeline_render::{
    RenderedFrames, RenderedPicture, TimelineOutput, TimelineSource, check_mp4_timeline, execute_mp4_timeline, verify_mp4_timeline,
};
use cfa4_project::{ItemId, ItemKind, MediaRef, Project, Sequence, TrackItem};
use cfa4_smart_edit::capabilities::{Codec, Container, Registry};
use cfa4_smart_edit::edit_decision::{EditClip, EditContent, EditDecisionList, EditItem, RenderCause};
use cfa4_smart_edit::plan::{CopyGuarantee, PlanSegment, SegmentMode};
use cfa4_smart_edit::smart_render::{SmartRenderPlan, SmartRenderRequest, plan_smart_render};
use cfa4_smart_edit::timestamp::{MediaTime, Rational};
use cfa4_time::{FrameRate, Tick};
use serde_json::{Value, json};

use crate::commands::{bad, bool_p, str_p};
use crate::export_tools::sequence_param;
use crate::smart_cut::{POLICY, Source, encoding_p, guarantee_p, open_source};
use crate::{EngineError, Result, Session};

/// A sequence's Smart Render plan, with the sources its copied ranges read.
struct Planned {
    seq: ItemId,
    plan: SmartRenderPlan,
    /// Opened sources, by `source_id` (the file path the media pool opens).
    sources: BTreeMap<String, Source>,
    output: TimelineOutput,
}

/// The sequence's frame duration as a fraction of a second.
fn frame_duration(rate: FrameRate) -> Rational {
    Rational::new(rate.den, rate.num)
}

/// Why effect `e` changes the pictures of a clip as large as the `frame` (none: it is enabled at
/// its defaults, or disabled). Automatic points (Motion's centred position and anchor) are compared
/// as the renderer resolves them.
fn effect_cause(e: &cfa4_project::EffectInstance, frame: (u32, u32)) -> Option<RenderCause> {
    if !e.enabled {
        return None;
    }
    let default = e.def().map(|d| {
        let mut d = d.instance();
        cfa4_project::resolve_auto_points(&mut d, frame, frame);
        d
    });
    let mut resolved = e.clone();
    cfa4_project::resolve_auto_points(&mut resolved, frame, frame);
    if default.as_ref() == Some(&resolved) {
        return None;
    }
    Some(match e.effect.as_str() {
        "motion" => RenderCause::Transform,
        "opacity" => RenderCause::Composite,
        "time_remap" => RenderCause::Speed,
        _ => RenderCause::Effect,
    })
}

/// What the timeline does to the pictures of `item` (none: they can be copied), and the media file
/// that shows them.
fn clip_causes<'p>(project: &'p Project, seq: &Sequence, item: &TrackItem) -> (Vec<RenderCause>, Option<&'p str>) {
    let mut causes = BTreeSet::new();
    if item.speed != 1.0 || item.reverse || item.frame_hold.is_some() {
        causes.insert(RenderCause::Speed);
    }
    causes.extend(item.effects.iter().filter_map(|e| effect_cause(e, (seq.settings.width, seq.settings.height))));
    if item.multicam.is_some() {
        causes.insert(RenderCause::Indirect);
    }
    if item.graphic.is_some() {
        causes.insert(RenderCause::Overlay);
    }
    let media = match project.item(item.item).map(|i| &i.kind) {
        Some(ItemKind::Sequence(_)) => {
            causes.insert(RenderCause::Indirect);
            None
        }
        Some(ItemKind::Media(_) | ItemKind::Subclip { .. }) => project.resolve_media(item.item).map(|(_, m, _)| m),
        _ => None,
    };
    let path = match media {
        Some(m) => {
            let s = &seq.settings;
            let i = &m.interpret;
            if i.frame_rate.is_some() {
                causes.insert(RenderCause::FrameRate);
            }
            if i.par.is_some() {
                causes.insert(RenderCause::Transform);
            }
            if i.color_space.is_some() || !s.color.is_plain() {
                causes.insert(RenderCause::ColorConversion);
            }
            match &m.info.video {
                Some(v) => {
                    if (v.width, v.height) != (s.width, s.height) || reduced(v.par) != reduced(s.par) {
                        causes.insert(RenderCause::Transform);
                    }
                    if !same_rate(v.frame_rate, s.frame_rate) {
                        causes.insert(RenderCause::FrameRate);
                    }
                    if cfa4_color::ColorSpace::from_info(&v.color) != cfa4_color::ColorSpace::Rec709 || v.hdr.is_some() {
                        causes.insert(RenderCause::ColorConversion);
                    }
                }
                None => {
                    causes.insert(RenderCause::Source);
                }
            }
            match &m.media {
                MediaRef::File { path } if is_mp4(path) && !m.offline => Some(path.as_str()),
                _ => {
                    causes.insert(RenderCause::Source);
                    None
                }
            }
        }
        None => {
            causes.insert(RenderCause::Source);
            None
        }
    };
    (causes.into_iter().collect(), path)
}

fn is_mp4(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.ends_with(".mp4") || p.ends_with(".m4v") || p.ends_with(".mov")
}

fn reduced((a, b): (u32, u32)) -> (u64, u64) {
    let (a, b) = (u64::from(a.max(1)), u64::from(b.max(1)));
    let (mut x, mut y) = (a, b);
    while y != 0 {
        (x, y) = (y, x % y);
    }
    (a / x, b / x)
}

fn same_rate(a: FrameRate, b: FrameRate) -> bool {
    a.num > 0 && a.den > 0 && i128::from(a.num) * i128::from(b.den) == i128::from(b.num) * i128::from(a.den)
}

/// An index key for every codec configuration of `src`: the sample entry's `avcC` bytes, so two
/// sources share an output stream only when their configurations are byte for byte the same.
fn configs(src: &Source) -> Vec<String> {
    let avcc = src.file.tracks.get(src.track).and_then(|t| t.codec()).and_then(|c| match c {
        cfa4_isobmff::CodecConfig::Avc(a) => Some(a.to_bytes()),
        _ => None,
    });
    let Some(avcc) = avcc else { return Vec::new() };
    let hex: String = avcc.iter().map(|b| format!("{b:02x}")).collect();
    let count = src.stream().and_then(|s| s.frames.iter().map(|f| f.config_id).max()).unwrap_or(0);
    (0..=count).map(|i| format!("avcC:{hex}#{i}")).collect()
}

/// The edit decision list of the sequence's video, opening each copyable source once.
fn edit_list(s: &Session, seq_id: ItemId, cmd: &str) -> Result<(EditDecisionList, BTreeMap<String, Source>)> {
    let project = &s.project;
    let seq = project.sequence(seq_id).ok_or_else(|| bad(cmd, "no such sequence"))?;
    let rate = seq.settings.frame_rate;
    let frame = rate.frame_duration();
    if rate.num <= 0 || rate.den <= 0 || rate.tick_of(1) != frame {
        return Err(bad(cmd, format!("the sequence frame rate {}/{} is not a whole number of ticks per frame", rate.num, rate.den)));
    }
    let tracks: Vec<_> = seq.video_tracks.iter().filter(|t| t.enabled).collect();
    let end = tracks.iter().flat_map(|t| t.items.iter().filter(|i| i.enabled)).map(TrackItem::end).max().unwrap_or(Tick::ZERO);
    let frames = rate.frame_at(end - Tick(1)).saturating_add(1);
    if end <= Tick::ZERO || frames <= 0 {
        return Err(bad(cmd, "the sequence has no video to export"));
    }
    // frame boundaries where what is shown can change; a frame an off-grid edit falls inside is
    // rendered as a whole
    let mut cuts = BTreeSet::from([0i64, frames]);
    let mut dirty = BTreeSet::new();
    let times = tracks.iter().flat_map(|t| {
        t.items.iter().filter(|i| i.enabled).flat_map(|i| [i.start, i.end()]).chain(t.transitions.iter().flat_map(|x| [x.start, x.start + x.duration]))
    });
    for t in times {
        let f = rate.frame_at(t).clamp(0, frames);
        cuts.insert(f);
        if rate.tick_of(f) != t && f < frames {
            dirty.insert(f);
            cuts.insert(f + 1);
        }
    }
    let cuts: Vec<i64> = cuts.into_iter().collect();
    let mut sources: BTreeMap<String, Source> = BTreeMap::new();
    let mut failed: BTreeSet<String> = BTreeSet::new();
    let mut items = Vec::new();
    for w in cuts.windows(2) {
        let [fa, fb] = [w[0], w[1]];
        let (a, b) = (rate.tick_of(fa), rate.tick_of(fb));
        let visible: Vec<&TrackItem> = tracks.iter().filter_map(|t| t.items.iter().find(|i| i.enabled && i.start <= a && a < i.end())).collect();
        let Some(top) = visible.last() else {
            items.push(EditItem { output_in: a, duration: Tick(b.0 - a.0), content: EditContent::Gap, render_causes: Vec::new() });
            continue;
        };
        let (mut causes, path) = clip_causes(project, seq, top);
        if visible.len() > 1 {
            causes.push(RenderCause::Composite);
        }
        if tracks.iter().any(|t| t.transitions.iter().any(|x| x.start < b && a < x.start + x.duration)) {
            causes.push(RenderCause::Transition);
        }
        if dirty.contains(&fa) {
            causes.push(RenderCause::Composite);
        }
        // the file, opened once (a file Smart Cut cannot read is rendered)
        let followed = path.map(|p| s.media.follow(p).unwrap_or_else(|_| p.to_string()));
        let src = match &followed {
            Some(p) if !failed.contains(p) => {
                if !sources.contains_key(p) {
                    match open_source(s, p, cmd) {
                        Ok(src) => {
                            sources.insert(p.clone(), src);
                        }
                        Err(_) => {
                            failed.insert(p.clone());
                        }
                    }
                }
                sources.get(p)
            }
            _ => None,
        };
        // the timeline executor copies H.264 only: a clip of another codec is rendered
        let src = src.filter(|src| src.stream().is_some_and(|x| x.codec == Codec::H264));
        let source_in = src.and_then(|src| src.stream_time(top.source_time_at(a)));
        if src.is_none() || source_in.is_none() {
            causes.push(RenderCause::Source);
        }
        causes.sort();
        causes.dedup();
        let content = match (src, source_in) {
            (Some(src), Some(source_in)) => EditContent::Clip(EditClip {
                source_id: src.path.clone(),
                stream_id: src.stream().map(|x| x.stream_id).unwrap_or_default(),
                container: src.container,
                source_in,
                configs: configs(src),
                format: src.format.clone(),
            }),
            // a picture the planner never reads: the renderer draws it
            _ => EditContent::Clip(EditClip {
                source_id: followed.unwrap_or_else(|| top.name.clone()),
                stream_id: 0,
                container: Container::Mp4,
                source_in: top.source_time_at(a),
                configs: Vec::new(),
                format: None,
            }),
        };
        items.push(EditItem { output_in: a, duration: Tick(b.0 - a.0), content, render_causes: causes });
    }
    let edl = EditDecisionList { timeline_id: format!("sequence-{}", seq_id.0), frame_duration: frame_duration(rate), items };
    Ok((edl, sources))
}

fn output_container(p: &Value, cmd: &str) -> Result<Container> {
    match str_p(p, "output") {
        Some(o) if o.to_ascii_lowercase().ends_with(".mov") => Ok(Container::Mov),
        Some(o) if is_mp4(o) => Ok(Container::Mp4),
        Some(o) => Err(bad(cmd, format!("{o}: the output must be an .mp4, .m4v or .mov file"))),
        None => Ok(Container::Mp4),
    }
}

fn plan_timeline(s: &Session, p: &Value, cmd: &str, encoding: BoundaryEncoding) -> Result<Planned> {
    let seq = sequence_param(s, p, cmd)?;
    let guarantee: CopyGuarantee = guarantee_p(p, cmd)?;
    let output_container = output_container(p, cmd)?;
    let (edl, sources) = edit_list(s, seq, cmd)?;
    let request = SmartRenderRequest { codec: Codec::H264, output_container, guarantee, output_config: None };
    let indexes: Vec<_> = sources.values().map(|x| x.index.clone()).collect();
    let plan = plan_smart_render(&Registry::native(), &indexes, &edl, &request, POLICY).map_err(|e| bad(cmd, format!("{e:?}")))?;
    let q = s.project.sequence(seq).ok_or_else(|| bad(cmd, "no such sequence"))?;
    let output = TimelineOutput { width: q.settings.width, height: q.settings.height, frame_duration: edl.frame_duration, encoding };
    Ok(Planned { seq, plan, sources, output })
}

fn timeline_sources(planned: &Planned) -> BTreeMap<String, TimelineSource<'_>> {
    planned
        .sources
        .iter()
        .filter_map(|(id, src)| {
            let index = src.stream()?;
            Some((id.clone(), TimelineSource { index, source: Mp4CopySource { file: &src.file, bytes: &*src.bytes, track: src.track } }))
        })
        .collect()
}

fn rendered_frames(p: &SmartRenderPlan) -> u64 {
    p.plan.segments.iter().filter(|s| s.mode == SegmentMode::Render).map(PlanSegment::frames).fold(0, u64::saturating_add)
}

fn summary(p: &SmartRenderPlan) -> Value {
    let rendered = rendered_frames(p);
    json!({
        "lossless": p.plan.is_lossless(),
        "requiresConfirmation": p.plan.requires_confirmation(),
        "copiedFrames": p.plan.copied_frames(),
        "reencodedFrames": p.plan.reencoded_frames().saturating_sub(rendered),
        "renderedFrames": rendered,
    })
}

/// `export.plan` with `"scope": "timeline"`.
pub(crate) fn plan(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "export.plan";
    let encoding = encoding_p(p, cmd)?;
    let planned = plan_timeline(s, p, cmd, encoding)?;
    let problem = check_mp4_timeline(&planned.plan.plan, &timeline_sources(&planned), &planned.output).err().map(|e| e.to_string());
    Ok(json!({
        "sequence": planned.seq.0,
        "plan": planned.plan,
        "summary": summary(&planned.plan),
        "timelineRender": {"ready": problem.is_none(), "problem": problem},
    }))
}

/// The export pipeline's pictures of the sequence, as 8-bit 4:2:0.
struct SequenceFrames {
    pipeline: cfa4_export::Pipeline,
    provider: crate::media_pool::PoolProvider,
}

impl RenderedFrames for SequenceFrames {
    fn picture(&mut self, _segment: &PlanSegment, output_frame: u64) -> std::result::Result<RenderedPicture, String> {
        let f = i64::try_from(output_frame).map_err(|_| format!("frame {output_frame} is out of range"))?;
        let (rgba, _) = self.pipeline.frame(f, &self.provider);
        let (w, h) = (self.pipeline.w as usize, self.pipeline.h as usize);
        if rgba.len() != w * h * 4 {
            return Err(format!("frame {f} was not rendered"));
        }
        let (mut y, mut u, mut v) = (Vec::new(), Vec::new(), Vec::new());
        cfa4_export::rgba_to_yuv420_8(&rgba, w, h, &mut y, &mut u, &mut v);
        Ok(RenderedPicture { width: self.pipeline.w, height: self.pipeline.h, y, u, v })
    }
}

/// `export.start` with `"scope": "timeline"`.
pub(crate) fn start(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "export.start";
    let output = str_p(p, "output").filter(|o| !o.is_empty()).map(str::to_string).ok_or_else(|| bad(cmd, "`output` (the file to write) is required"))?;
    let encoding = encoding_p(p, cmd)?;
    let planned = plan_timeline(s, p, cmd, encoding)?;
    if planned.sources.contains_key(&output) || planned.sources.values().any(|x| x.path == output) {
        return Err(bad(cmd, "the output must not be a source file of the sequence"));
    }
    let plan = &planned.plan;
    if plan.plan.requires_confirmation() && !bool_p(p, "confirm").unwrap_or(false) {
        return Err(bad(
            cmd,
            format!(
                "the export renders {} and re-encodes {} frames (copies {}); run it with \"confirm\": true",
                rendered_frames(plan),
                plan.plan.reencoded_frames().saturating_sub(rendered_frames(plan)),
                plan.plan.copied_frames()
            ),
        ));
    }
    if !plan.executable {
        return Err(EngineError::Other(format!("Smart Render cannot run: {}", plan.blockers.join("; "))));
    }
    let sources = timeline_sources(&planned);
    let other = |e: SmartCutError| EngineError::Other(e.to_string());
    check_mp4_timeline(&plan.plan, &sources, &planned.output).map_err(other)?;
    let settings = cfa4_export::ExportSettings::default();
    let pipeline = cfa4_export::Pipeline::new(Arc::clone(&s.project), planned.seq, &settings, false).map_err(|e| EngineError::Other(e.to_string()))?;
    if (pipeline.w, pipeline.h) != (planned.output.width, planned.output.height) {
        return Err(EngineError::Other(format!(
            "the export pipeline renders {}×{}, the sequence is {}×{}",
            pipeline.w, pipeline.h, planned.output.width, planned.output.height
        )));
    }
    let provider = s.media.full_res_provider(Arc::clone(&s.project), s.services.clone());
    let mut frames = SequenceFrames { pipeline, provider };
    let (out, report) = execute_mp4_timeline(&plan.plan, &sources, &mut frames, &planned.output, Cursor::new(Vec::new())).map_err(other)?;
    let bytes = out.into_inner();
    let verified = verify_mp4_timeline(&plan.plan, &sources, &planned.output, bytes.as_slice()).map_err(other)?;
    if (verified.samples, verified.copied, verified.reencoded, verified.rendered) != (report.samples, report.copied, report.reencoded, report.rendered) {
        return Err(EngineError::Other("Smart Render verification read back another file than was written".into()));
    }
    // the sequence's audio: mixed and encoded (a mix cannot be copied), presented with the video
    let mut audio = Vec::new();
    let bytes = if bool_p(p, "audio").unwrap_or(true) {
        // the written video's length, as the export range of the mix
        let length = i64::try_from(verified.duration)
            .ok()
            .and_then(|d| MediaTime::new(d, Rational::new(1, i64::from(verified.timescale.max(1)))).to_tick())
            .ok_or_else(|| EngineError::Other("the sequence duration does not fit the output".into()))?;
        let range = cfa4_time::TimeRange::new(Tick::ZERO, length);
        let provider = s.media.full_res_provider(Arc::clone(&s.project), s.services.clone());
        match cfa4_export::timeline_audio::encode_sequence_audio(&s.project, planned.seq, range, &provider).map_err(other)? {
            Some(track) => {
                let tracks = [track];
                let with_audio = mux_audio_tracks(&bytes, &tracks, (verified.duration, verified.timescale), cfa4_isobmff::Brand::Mp4).map_err(other)?;
                audio = verify_audio_tracks(&with_audio, &bytes, &tracks).map_err(other)?;
                with_audio
            }
            None => bytes,
        }
    } else {
        bytes
    };
    s.services.write_file(&output, &bytes).map_err(|e| EngineError::Other(format!("{output}: {e}")))?;
    Ok(json!({
        "sequence": planned.seq.0,
        "output": output,
        "bytes": bytes.len(),
        "report": verified,
        "verified": true,
        "summary": summary(plan),
        "audio": {"verified": audio},
    }))
}

/// Most access points listed per clip (a long clip with a key frame every picture would make the
/// reply large; the timeline cannot show more than one per pixel anyway).
const MAX_MARKS_PER_CLIP: usize = 4096;

/// `export.gopMarks`: the access points of every video clip of the sequence, in sequence time, for
/// the timeline's GOP overlay (WP-08, §10.1). A clip that is not a readable H.264 / HEVC / AV1
/// MP4 / MOV file, or that plays at another speed, has no marks. `in` says whether the clip starts
/// on a clean access point (a Smart Cut copies from there; otherwise its head is re-encoded).
pub(crate) fn gop_marks(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "export.gopMarks";
    let seq_id = sequence_param(s, p, cmd)?;
    let project = Arc::clone(&s.project);
    let seq = project.sequence(seq_id).ok_or_else(|| bad(cmd, "no such sequence"))?;
    let mut sources: BTreeMap<String, Option<Source>> = BTreeMap::new();
    let mut clips = Vec::new();
    for (ti, track) in seq.video_tracks.iter().enumerate() {
        for item in track.items.iter().filter(|i| i.enabled) {
            if item.speed != 1.0 || item.reverse || item.frame_hold.is_some() {
                continue;
            }
            let Some(path) = clip_causes(&project, seq, item).1 else { continue };
            let path = s.media.follow(path).unwrap_or_else(|_| path.to_string());
            if !sources.contains_key(&path) {
                let opened = open_source(s, &path, cmd).ok();
                sources.insert(path.clone(), opened);
            }
            let Some(Some(src)) = sources.get(&path) else { continue };
            let Some(stream) = src.stream() else { continue };
            let tb = stream.timebase;
            let mut points = Vec::new();
            let mut clean_in = false;
            for f in stream.frames.iter().filter(|f| f.random_access.is_access_point()) {
                let media = f.pts.and_then(|v| MediaTime::new(v, tb).to_tick()).and_then(|t| src.media_time(t));
                let Some(at) = media.and_then(|m| m.0.checked_sub(item.source_in.0)).and_then(|d| item.start.0.checked_add(d)).map(Tick) else {
                    continue;
                };
                if at < item.start || at >= item.end() {
                    continue;
                }
                clean_in |= at == item.start && f.random_access.is_clean();
                if points.len() < MAX_MARKS_PER_CLIP {
                    points.push(json!({"time": at.0, "clean": f.random_access.is_clean()}));
                }
            }
            clips.push(json!({"clip": item.id.0, "track": ti, "codec": stream.codec, "in": clean_in, "points": points}));
        }
    }
    Ok(json!({"sequence": seq_id.0, "clips": clips}))
}
