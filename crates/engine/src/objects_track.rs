//! `objects.track` (Object Mask plan, OM-A5): follow an object's box through the media with the
//! classic trackers of `masks.track` (feature points or correlation filter), as a background job.
//!
//! The box of the object's sample nearest the start time is followed frame by frame; each frame
//! becomes an [`ObjectSample`] (media time, source pixels, score 1) written in one merged undo step
//! per job by [`poll`]. Tracking stops when the object is lost; what was tracked stays.

use cfa4_geom::{Affine, Rect, Vec2};
use cfa4_project::object::{ObjectId, ObjectSample};
use cfa4_project::{ItemId, TrackMethod, Tracker};
use cfa4_time::Tick;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

use crate::commands::{CommandSpec, always, bad, str_p};
use crate::{EngineError, Result, Session};

/// Longest side of the frames the tracker works on (speed; the result is in source pixels).
const TRACK_MAX_WIDTH: f64 = 960.0;
/// Frames one job follows at most.
pub const MAX_TRACK_FRAMES: usize = 100_000;

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "objects.track",
        label: "Track Object",
        menu: &[],
        shortcut: None,
        params: r#"{"id":n,"direction":"forward"|"backward"?,"frames":n?,"mediaTime":ticks?,"method":"position|positionRotation|positionScaleRotation"?,"tracker":"points|correlation"?,"wait":bool?}"#,
        enabled: always,
        run: track,
        journal: true,
    }]
}

/// A running job: samples arrive in `samples` and are written by [`poll`].
pub struct PendingObjectTrack {
    pub job: u64,
    pub item: ItemId,
    pub object: ObjectId,
    samples: Arc<Mutex<Vec<ObjectSample>>>,
    applied: usize,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn corners(r: &Rect) -> Vec<Vec2> {
    vec![Vec2::new(r.x, r.y), Vec2::new(r.x + r.w, r.y), Vec2::new(r.x + r.w, r.y + r.h), Vec2::new(r.x, r.y + r.h)]
}

/// Bounding box of `pts`; `None` when a coordinate is not finite or the box is empty.
fn bounds(pts: &[Vec2]) -> Option<Rect> {
    if pts.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return None;
    }
    let x0 = pts.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
    let y0 = pts.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    let x1 = pts.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
    let y1 = pts.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
    let r = Rect::new(x0, y0, x1 - x0, y1 - y0);
    (r.w >= 1.0 && r.h >= 1.0 && r.x.abs() <= 1.0e7 && r.y.abs() <= 1.0e7 && r.w <= 1.0e7 && r.h <= 1.0e7).then_some(r)
}

fn track(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "objects.track";
    let id = ObjectId(p.get("id").and_then(Value::as_u64).ok_or_else(|| bad(C, "need `id`"))?);
    let (item, obj) = s
        .project
        .objects
        .iter()
        .find_map(|(i, v)| v.iter().find(|t| t.id == id).map(|t| (*i, t.clone())))
        .ok_or_else(|| bad(C, format!("no object {}", id.0)))?;
    if s.object_jobs.iter().any(|j| j.object == id) {
        return Err(bad(C, "this object is already being tracked"));
    }
    let method = match str_p(p, "method") {
        Some(m) => TrackMethod::from_name(m).ok_or_else(|| bad(C, format!("unknown method `{m}`")))?,
        None => TrackMethod::Position,
    };
    let tracker = match str_p(p, "tracker") {
        Some(t) => Tracker::from_name(t).ok_or_else(|| bad(C, format!("unknown tracker `{t}`: points or correlation")))?,
        None => Tracker::Points,
    };
    let backward = match str_p(p, "direction").unwrap_or("forward") {
        "forward" => false,
        "backward" => true,
        d => return Err(bad(C, format!("direction `{d}`: forward or backward"))),
    };
    let samples = obj.samples();
    // start: the sample at `mediaTime`, else the last (forward) or first (backward) one
    let start = match p.get("mediaTime") {
        Some(v) => {
            let t = Tick(v.as_i64().ok_or_else(|| bad(C, "`mediaTime` is ticks"))?);
            samples.iter().min_by_key(|x| (i128::from(x.time.0) - i128::from(t.0)).abs()).cloned()
        }
        None if backward => samples.first().cloned(),
        None => samples.last().cloned(),
    }
    .ok_or_else(|| bad(C, "the object has no box to start from"))?;
    let pitem = s.project.item(item).ok_or_else(|| bad(C, "the item is gone"))?;
    let fd = pitem.frame_rate().frame_duration();
    if fd <= Tick::ZERO {
        return Err(bad(C, "the item has no frame rate"));
    }
    let end = pitem.duration();
    let mt0 = start.time;
    let room = if backward { mt0.0 / fd.0 } else { (end - mt0 - Tick(1)).0 / fd.0 };
    let steps = match p.get("frames") {
        Some(v) => v.as_u64().ok_or_else(|| bad(C, "`frames` is a count"))?.min(MAX_TRACK_FRAMES as u64) as i64,
        None => MAX_TRACK_FRAMES as i64,
    }
    .min(room)
    .max(0) as usize;
    if steps == 0 {
        return Err(bad(C, if backward { "already at the first frame" } else { "already at the last frame" }));
    }
    let src = s.source(item).ok_or_else(|| EngineError::Other("the item has no media to track".into()))?;
    let size = cfa4_render::source_size(&s.project, item).ok_or_else(|| bad(C, "the item has no picture"))?;
    let scale = (TRACK_MAX_WIDTH / f64::from(size.0.max(1))).min(1.0) as f32;
    let jid = s.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
    let label = format!("Track {} ({})", obj.label, if backward { "backward" } else { "forward" });
    let job = crate::Job { id: jid, label, progress: Default::default(), result: Default::default() };
    job.progress.total.store(steps as u64, std::sync::atomic::Ordering::Relaxed);
    let out: Arc<Mutex<Vec<ObjectSample>>> = Arc::default();
    let (prog, res, sink) = (job.progress.clone(), job.result.clone(), out.clone());
    let start_box = start.bbox;
    let run = move || {
        use std::sync::atomic::Ordering;
        let t0 = web_time::Instant::now();
        let gray = |t: Tick| -> Option<(cfa4_render::track::Prepared, f64)> {
            let f = src.video_frame(cfa4_media::FrameRequest { time: t, scale }).ok()?;
            let g = cfa4_render::track::Gray::from_rgba8(f.width as usize, f.height as usize, &f.to_rgba8());
            Some((cfa4_render::track::Prepared::new(g), f64::from(f.width) / f64::from(size.0.max(1))))
        };
        let mut err: Option<String> = None;
        let mut done = 0u64;
        match gray(mt0) {
            None => err = Some("can't decode the media".into()),
            Some((mut prev, mut k_prev)) => {
                let mut pts = corners(&start_box);
                let mut corr = if tracker == Tracker::Correlation {
                    let region: Vec<Vec2> = pts.iter().map(|q| *q * k_prev).collect();
                    let c = cfa4_render::cftrack::CorrTracker::new(prev.base(), &region, method == TrackMethod::PositionScaleRotation);
                    if c.is_none() {
                        err = Some("can't track: the box is empty or outside the picture".into());
                    }
                    c
                } else {
                    None
                };
                let steps = if err.is_some() { 0 } else { steps };
                for step in 1..=steps {
                    if prog.cancel.load(Ordering::Relaxed) {
                        err = Some("stopped".into());
                        break;
                    }
                    let t = if backward { mt0 - Tick(fd.0 * step as i64) } else { mt0 + Tick(fd.0 * step as i64) };
                    let Some((next, k)) = gray(t) else {
                        err = Some(format!("can't decode frame {step}"));
                        break;
                    };
                    let transform = if let Some(c) = corr.as_mut() {
                        match c.update(next.base()) {
                            Some(st) => st.transform,
                            None => {
                                err = Some(format!("lost track after {} frame(s): the object is hidden or left the picture", step - 1));
                                break;
                            }
                        }
                    } else {
                        let region: Vec<Vec2> = pts.iter().map(|q| *q * k_prev).collect();
                        match cfa4_render::track::track_step(&prev, &next, &region, method) {
                            Some(st) => st.transform,
                            None => {
                                err = Some(format!("lost track after {} frame(s): not enough detail inside the box", step - 1));
                                break;
                            }
                        }
                    };
                    // source pixels → frame pixels → next frame pixels → source pixels
                    let m = Affine::scale(k_prev, k_prev).then_apply(&transform).then_apply(&Affine::scale(1.0 / k, 1.0 / k));
                    let moved: Vec<Vec2> = pts.iter().map(|q| m.apply(*q)).collect();
                    let Some(bbox) = bounds(&moved) else {
                        err = Some(format!("lost track after {} frame(s): the box collapsed", step - 1));
                        break;
                    };
                    pts = corners(&bbox);
                    lock(&sink).push(ObjectSample { time: t, bbox, score: 1.0, mask: None });
                    done += 1;
                    prog.done.store(done, Ordering::Relaxed);
                    *lock(&prog.status) = format!("Frame {step} of {steps}");
                    prev = next;
                    k_prev = k;
                }
            }
        }
        let secs = t0.elapsed().as_secs_f64();
        let r = match &err {
            Some(e) if e != "stopped" => Err(e.clone()),
            _ => Ok(cfa4_export::Report {
                path: String::new(),
                frames: done,
                seconds: secs,
                bytes: 0,
                render_fps: done as f64 / secs.max(1e-6),
                extra_files: Vec::new(),
            }),
        };
        *lock(&prog.status) = match &err {
            Some(e) => format!("{e} ({done} frame(s) tracked)"),
            None => format!("Tracked {done} frame(s) in {secs:.1}s"),
        };
        prog.finished.store(true, Ordering::Relaxed);
        *lock(&res) = Some(r);
    };
    let run = crate::export_tools::guard_job(job.progress.clone(), job.result.clone(), run);
    s.jobs.push(job);
    s.object_jobs.push(PendingObjectTrack { job: jid, item, object: id, samples: out, applied: 0 });
    let wait = crate::commands::bool_p(p, "wait").unwrap_or(false);
    if wait || cfg!(target_arch = "wasm32") {
        run();
        poll(s);
    } else {
        crate::background::start(s, jid, crate::background::Need::background(512, 1), "cfa4-object-track", run)?;
    }
    let st = s.jobs.iter().find(|j| j.id == jid).map(crate::Job::to_json).unwrap_or(Value::Null);
    Ok(json!({"job": jid, "frames": steps, "status": st}))
}

/// Write newly tracked samples (one merged undo step per job) and drop finished jobs. Called from
/// `Session::poll_persistence` and after synchronous runs.
pub fn poll(s: &mut Session) {
    use std::sync::atomic::Ordering;
    let mut i = 0;
    while i < s.object_jobs.len() {
        let finished = s.jobs.iter().find(|j| j.id == s.object_jobs[i].job).is_none_or(|j| j.progress.finished.load(Ordering::Relaxed));
        let new: Vec<ObjectSample> = {
            let pj = &s.object_jobs[i];
            lock(&pj.samples)[pj.applied..].to_vec()
        };
        if !new.is_empty() {
            let n_new = new.len();
            let (item, object, key) = (s.object_jobs[i].item, s.object_jobs[i].object, format!("object-track-{}", s.object_jobs[i].job));
            let r = s.edit_merged("Track Object", &key, move |pr, _| {
                let t =
                    pr.objects.get_mut(&item).and_then(|v| v.iter_mut().find(|t| t.id == object)).ok_or_else(|| bad("objects.track", "the object is gone"))?;
                for sample in new {
                    t.insert_sample(sample).map_err(|e| bad("objects.track", e.to_string()))?;
                }
                Ok(())
            });
            match r {
                Ok(()) => s.object_jobs[i].applied += n_new,
                Err(e) => {
                    s.error_toast("objects.track", e.to_string());
                    if let Some(j) = s.jobs.iter().find(|j| j.id == s.object_jobs[i].job) {
                        j.progress.cancel.store(true, Ordering::Relaxed);
                    }
                    s.object_jobs.remove(i);
                    continue;
                }
            }
        }
        let total = lock(&s.object_jobs[i].samples).len();
        if finished && s.object_jobs[i].applied >= total {
            s.object_jobs.remove(i);
            continue;
        }
        i += 1;
    }
}
