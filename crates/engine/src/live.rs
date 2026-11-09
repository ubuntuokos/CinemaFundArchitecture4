//! Live graphics (credits plan C10): prepared cues of CFA4 graphics – credits from a
//! spreadsheet, or a graphic clip of the sequence (a title, a lower third) – taken to air and
//! stepped through like a character generator: PREVIEW, TAKE, NEXT / PREVIOUS, HOLD /
//! CONTINUE, UPDATE, OUT. The same layout and graphics renderer as the editor (no second
//! graphics engine); the program picture goes to a local page for OBS's Browser Source
//! (`cfa4-live`, 127.0.0.1 only), and obs-websocket can point OBS at it. OBS never drives CFA4.
//!
//! The live state belongs to the session, not the project: nothing here is undone or saved.
//! The program clock is the wall clock (`live.tick` advances it; `at` sets a moment, for tests
//! and agents).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use web_time::Instant;

use cfa4_credits::{Behaviour, PageOut};
use cfa4_live::{AudioMode, FrameStore, LiveServer, ObsClient};
use cfa4_project::{ClipId, ItemKind, MediaRef, TrackItem};
use cfa4_time::Tick;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, has_seq, str_p, u64_p};
use crate::{EngineError, Result, Session};

/// Most cues prepared at once.
pub const MAX_CUES: usize = 200;
/// Widest picture rendered for the program (the page scales it to the source's size).
pub const MAX_WIDTH: u32 = 3840;
/// Most decoded pictures kept for the program.
const MAX_PICTURES: usize = 64;
/// Largest picture file read.
const MAX_PICTURE_BYTES: u64 = 256 << 20;
/// Name of the Browser Source CFA4 creates in OBS.
pub const OBS_SOURCE: &str = "CFA4 Live";
pub const OBS_AUDIO_SOURCE: &str = "CFA4 Live Audio";

/// What a cue shows.
#[derive(Clone)]
enum Source {
    /// Credits from a spreadsheet, credits file or template: the parameters of
    /// `graphics.credits.preview`, read again on UPDATE.
    Credits { params: Value, base: Option<PathBuf> },
    /// A graphic clip of the sequence, as it was when added (UPDATE takes it again).
    Clip { clip: ClipId, item: Box<TrackItem> },
}

#[derive(Clone)]
struct Cue {
    id: u64,
    name: String,
    source: Source,
    /// Credits: the laid-out pages; a clip: none (the clip is the one page).
    pages: Vec<PageOut>,
    /// Card pages wait at their last full moment until NEXT (default) instead of fading out.
    hold_cards: bool,
    audio: Option<PathBuf>,
}

impl Cue {
    fn page_count(&self) -> usize {
        match self.source {
            Source::Credits { .. } => self.pages.len(),
            Source::Clip { .. } => 1,
        }
    }

    fn page_duration(&self, page: usize) -> Tick {
        match &self.source {
            Source::Credits { .. } => self.pages.get(page).map_or(Tick::ZERO, |p| p.duration),
            Source::Clip { item, .. } => item.duration,
        }
    }

    fn duration(&self) -> Tick {
        (0..self.page_count()).fold(Tick::ZERO, |a, i| Tick(a.0.saturating_add(self.page_duration(i).0)))
    }

    fn kind(&self) -> &'static str {
        match self.source {
            Source::Credits { .. } => "credits",
            Source::Clip { .. } => "clip",
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "kind": self.kind(),
            "pages": self.page_count(),
            "duration": self.duration().0,
            "seconds": self.duration().0 as f64 / cfa4_time::TICKS_PER_SECOND as f64,
            "audio": self.audio.as_ref().map(|p| p.to_string_lossy().into_owned()),
            "holdCards": self.hold_cards,
        })
    }
}

/// The cue on air.
#[derive(Clone)]
struct OnAir {
    cue: u64,
    page: usize,
    /// When the page's moment 0 was (shifted on CONTINUE by the time held).
    page_start: Instant,
    /// Frozen page time while held.
    held: Option<Duration>,
    /// Time of the earlier pages (for the sound's position).
    before: Duration,
    /// The last page has run out: nothing more to show until OUT or the next TAKE.
    finished: bool,
}

/// An obs-websocket link and what it was asked to set up.
struct ObsLink {
    client: ObsClient,
    host: String,
    port: u16,
    source: String,
    scene: Option<String>,
    last: Value,
}

/// The session's live graphics.
pub struct LiveState {
    cues: Vec<Cue>,
    next_id: u64,
    preview: Option<u64>,
    program: Option<OnAir>,
    audio_mode: AudioMode,
    store: Arc<FrameStore>,
    server: Option<LiveServer>,
    obs: Option<ObsLink>,
    /// Width of the program picture (0: the sequence's, at most [`MAX_WIDTH`]).
    width: u32,
    /// Decoded pictures (straight RGBA8) by file; None: it could not be read.
    pictures: HashMap<PathBuf, Option<Arc<(u32, u32, Vec<u8>)>>>,
}

impl Default for LiveState {
    fn default() -> Self {
        LiveState {
            cues: Vec::new(),
            next_id: 1,
            preview: None,
            program: None,
            audio_mode: AudioMode::default(),
            store: FrameStore::new(),
            server: None,
            obs: None,
            width: 0,
            pictures: HashMap::new(),
        }
    }
}

impl LiveState {
    fn cue(&self, id: u64) -> Option<&Cue> {
        self.cues.iter().find(|c| c.id == id)
    }

    fn index(&self, id: u64) -> Option<usize> {
        self.cues.iter().position(|c| c.id == id)
    }
}

fn secs(d: Duration) -> f64 {
    d.as_secs_f64()
}

fn ticks(d: Duration) -> Tick {
    Tick((d.as_secs_f64() * cfa4_time::TICKS_PER_SECOND as f64).min(i64::MAX as f64 / 2.0) as i64)
}

fn dur(t: Tick) -> Duration {
    Duration::from_secs_f64((t.0.max(0) as f64 / cfa4_time::TICKS_PER_SECOND as f64).min(1e9))
}

/// A seconds parameter (finite, not negative, at most a day).
fn secs_p(p: &Value, key: &str) -> Option<Duration> {
    p.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()).map(|v| Duration::from_secs_f64(v.clamp(0.0, 86_400.0)))
}

fn cue_p(s: &Session, cmd: &str, p: &Value) -> Result<u64> {
    let id = u64_p(p, "id").or(s.live.preview).ok_or_else(|| bad(cmd, "name a cue (`id`) or preview one first"))?;
    s.live.cue(id).map(|c| c.id).ok_or_else(|| bad(cmd, format!("there is no cue {id}")))
}

/// Build a cue's pages from its source.
fn load(s: &Session, cmd: &str, source: &Source) -> Result<Vec<PageOut>> {
    match source {
        Source::Credits { params, .. } => {
            let (doc, _, base) = crate::credits::doc_p(s, cmd, params)?;
            let l = crate::credits::lay_doc(s, cmd, &doc, base)?;
            if l.pages.is_empty() {
                return Err(bad(cmd, "the credits have no pages"));
            }
            Ok(l.pages)
        }
        Source::Clip { .. } => Ok(Vec::new()),
    }
}

fn clip_source(s: &Session, cmd: &str, clip: ClipId) -> Result<Source> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (_, it) = q.find_item(clip).ok_or_else(|| bad(cmd, format!("there is no clip {}", clip.0)))?;
    if it.graphic.is_none() {
        return Err(bad(cmd, "only graphic clips (titles, lower thirds, credits pages) can go live"));
    }
    Ok(Source::Clip { clip, item: Box::new(it.clone()) })
}

/// A clip's media file (for `audioClip`).
fn clip_file(s: &Session, clip: ClipId) -> Option<PathBuf> {
    let q = s.active_sequence()?;
    let (_, it) = q.find_item(clip)?;
    match &s.project.item(it.item)?.kind {
        ItemKind::Media(m) => match &m.media {
            MediaRef::File { path } => Some(PathBuf::from(path)),
            MediaRef::Generator(_) => None,
        },
        _ => None,
    }
}

fn audio_p(s: &Session, cmd: &str, p: &Value) -> Result<Option<PathBuf>> {
    if let Some(path) = str_p(p, "audio").filter(|x| !x.trim().is_empty()) {
        let f = PathBuf::from(path);
        if !f.is_file() {
            return Err(bad(cmd, format!("the sound file {path} was not found")));
        }
        return Ok(Some(f));
    }
    if let Some(c) = u64_p(p, "audioClip") {
        return clip_file(s, ClipId(c)).map(Some).ok_or_else(|| bad(cmd, format!("clip {c} has no sound file")));
    }
    Ok(None)
}

fn cue_add(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "live.cue.add";
    if s.live.cues.len() >= MAX_CUES {
        return Err(bad(CMD, format!("at most {MAX_CUES} cues")));
    }
    let source = match u64_p(p, "clip") {
        Some(c) => clip_source(s, CMD, ClipId(c))?,
        None => {
            let (_, _, base) = crate::credits::doc_p(s, CMD, p)?;
            let mut params = p.clone();
            if let Some(o) = params.as_object_mut() {
                for k in ["name", "audio", "audioClip", "holdCards", "clip"] {
                    o.remove(k);
                }
            }
            Source::Credits { params, base }
        }
    };
    let pages = load(s, CMD, &source)?;
    let audio = audio_p(s, CMD, p)?;
    let name = str_p(p, "name").map(str::trim).filter(|n| !n.is_empty()).map(str::to_string).unwrap_or_else(|| match &source {
        Source::Credits { params, .. } => str_p(params, "path")
            .and_then(|x| std::path::Path::new(x).file_stem().map(|f| f.to_string_lossy().into_owned()))
            .or_else(|| pages.first().map(|pg| pg.name.clone()))
            .unwrap_or_else(|| "Credits".into()),
        Source::Clip { item, clip } => item.graphic.as_ref().map(|_| format!("Graphic {}", clip.0)).unwrap_or_default(),
    });
    let id = s.live.next_id;
    s.live.next_id += 1;
    let hold_cards = p.get("holdCards").and_then(Value::as_bool).unwrap_or(true);
    let cue = Cue { id, name: name.chars().take(200).collect(), source, pages, hold_cards, audio };
    let out = cue.to_json();
    s.live.cues.push(cue);
    if s.live.preview.is_none() {
        s.live.preview = Some(id);
    }
    publish_status(s);
    Ok(out)
}

fn cue_remove(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "live.cue.remove";
    let id = u64_p(p, "id").ok_or_else(|| bad(CMD, "name the cue (`id`)"))?;
    let i = s.live.index(id).ok_or_else(|| bad(CMD, format!("there is no cue {id}")))?;
    if s.live.program.as_ref().is_some_and(|o| o.cue == id) {
        return Err(bad(CMD, "the cue is on air: take it OUT first"));
    }
    s.live.cues.remove(i);
    if s.live.preview == Some(id) {
        s.live.preview = s.live.cues.get(i).or(s.live.cues.last()).map(|c| c.id);
    }
    publish_status(s);
    Ok(json!({"removed": id}))
}

/// UPDATE: read the cue's spreadsheet (or clip) again; new credits parameters may replace the
/// old ones. On air, the page stays (or the last one, when there are fewer now).
fn cue_update(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "live.update";
    let id = match u64_p(p, "id") {
        Some(id) => id,
        None => s.live.program.as_ref().map(|o| o.cue).or(s.live.preview).ok_or_else(|| bad(CMD, "nothing on air or in preview"))?,
    };
    let i = s.live.index(id).ok_or_else(|| bad(CMD, format!("there is no cue {id}")))?;
    let source = match s.live.cues[i].source.clone() {
        Source::Credits { mut params, base } => {
            if let (Some(o), Some(newp)) = (params.as_object_mut(), p.as_object()) {
                for (k, v) in newp {
                    if !matches!(k.as_str(), "id" | "name" | "audio" | "audioClip" | "holdCards") {
                        o.insert(k.clone(), v.clone());
                    }
                }
            }
            Source::Credits { params, base }
        }
        Source::Clip { clip, .. } => clip_source(s, CMD, clip)?,
    };
    let pages = load(s, CMD, &source)?;
    let audio = if p.get("audio").is_some() || p.get("audioClip").is_some() { audio_p(s, CMD, p)? } else { s.live.cues[i].audio.clone() };
    let cue = &mut s.live.cues[i];
    cue.source = source;
    cue.pages = pages;
    cue.audio = audio;
    let n = cue.page_count().max(1);
    if let Some(o) = s.live.program.as_mut().filter(|o| o.cue == id) {
        o.page = o.page.min(n - 1);
    }
    let out = s.live.cues[i].to_json();
    tick_program(s, None)?;
    Ok(out)
}

fn preview(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "live.preview";
    let id = if p.get("next").and_then(Value::as_bool).unwrap_or(false) {
        next_after(s, s.live.preview.or(s.live.program.as_ref().map(|o| o.cue))).ok_or_else(|| bad(CMD, "there is no next cue"))?
    } else {
        cue_p(s, CMD, p)?
    };
    s.live.preview = Some(id);
    publish_status(s);
    Ok(status(s))
}

fn next_after(s: &Session, id: Option<u64>) -> Option<u64> {
    match id.and_then(|i| s.live.index(i)) {
        Some(i) => s.live.cues.get(i + 1).map(|c| c.id),
        None => s.live.cues.first().map(|c| c.id),
    }
}

/// TAKE: the preview cue goes on air from its first page; the next cue comes into preview.
fn take(s: &mut Session, _p: &Value) -> Result<Value> {
    const CMD: &str = "live.take";
    let id = s.live.preview.ok_or_else(|| bad(CMD, "nothing in preview: add a cue first"))?;
    if s.live.cue(id).is_none() {
        return Err(bad(CMD, format!("there is no cue {id}")));
    }
    s.live.program = Some(OnAir { cue: id, page: 0, page_start: Instant::now(), held: None, before: Duration::ZERO, finished: false });
    s.live.preview = next_after(s, Some(id)).or(Some(id));
    let audio = s.live.cue(id).and_then(|c| c.audio.clone());
    s.live.store.set_audio(if s.live.audio_mode == AudioMode::Ignore { None } else { audio });
    if s.live.audio_mode == AudioMode::Separate
        && let Some(link) = s.live.obs.as_mut()
    {
        // a new cue's sound from its start; OBS may not have the source (left alone then)
        let _ = link.client.restart_media(OBS_AUDIO_SOURCE);
    }
    tick_program(s, None)?;
    Ok(status(s))
}

fn step(s: &mut Session, forward: bool) -> Result<Value> {
    let cmd = if forward { "live.next" } else { "live.previous" };
    let o = s.live.program.clone().ok_or_else(|| bad(cmd, "nothing on air"))?;
    let cue = s.live.cue(o.cue).ok_or_else(|| bad(cmd, "the cue on air was removed"))?;
    let n = cue.page_count();
    if forward && (o.page + 1 >= n || o.finished) {
        // past the last page: the next cue, as TAKE
        if s.live.preview.is_some_and(|p| p != o.cue) {
            return take(s, &Value::Null);
        }
        return Err(bad(cmd, "this is the last page and no other cue is in preview"));
    }
    if !forward && o.page == 0 {
        // the first page from its start
        if let Some(on) = s.live.program.as_mut() {
            on.page_start = Instant::now();
            on.held = on.held.map(|_| Duration::ZERO);
            on.finished = false;
        }
        tick_program(s, None)?;
        return Ok(status(s));
    }
    let page = if forward { o.page + 1 } else { o.page - 1 };
    let before = (0..page).fold(Duration::ZERO, |a, i| a + dur(cue.page_duration(i)));
    if let Some(on) = s.live.program.as_mut() {
        on.page = page;
        on.page_start = Instant::now();
        on.held = on.held.map(|_| Duration::ZERO);
        on.before = before;
        on.finished = false;
    }
    tick_program(s, None)?;
    Ok(status(s))
}

fn hold(s: &mut Session, held: bool) -> Result<Value> {
    let cmd = if held { "live.hold" } else { "live.continue" };
    let on = s.live.program.as_mut().ok_or_else(|| bad(cmd, "nothing on air"))?;
    match (held, on.held) {
        (true, None) => on.held = Some(on.page_start.elapsed()),
        (false, Some(t)) => {
            on.page_start = Instant::now().checked_sub(t).unwrap_or_else(Instant::now);
            on.held = None;
        }
        _ => {}
    }
    publish_status(s);
    Ok(status(s))
}

fn out(s: &mut Session, _p: &Value) -> Result<Value> {
    s.live.program = None;
    s.live.store.clear();
    s.live.store.set_audio(None);
    publish_status(s);
    Ok(status(s))
}

/// The page time to show now (`at`: this many seconds into the page instead of the clock).
fn page_elapsed(o: &OnAir, at: Option<Duration>) -> Duration {
    at.or(o.held).unwrap_or_else(|| o.page_start.elapsed())
}

/// The moment of the page to draw for `elapsed`, and whether the page has run out.
fn page_moment(s: &Session, cue: &Cue, page: usize, elapsed: Duration) -> (Tick, bool) {
    let rate = s.sequence_rate();
    let fd = rate.frame_duration();
    let d = cue.page_duration(page);
    let last = Tick(d.0.saturating_sub(fd.0).max(0));
    let t = rate.snap(ticks(elapsed));
    let card = matches!(&cue.source, Source::Credits { .. }) && cue.pages.get(page).is_some_and(|p| p.behaviour == Behaviour::Card);
    if card && cue.hold_cards {
        // a card stays at its last full moment (before it fades out) until NEXT or OUT
        let fade_out = cue.pages.get(page).map_or(Tick::ZERO, |p| p.fade_out);
        let hold_at = Tick(d.0.saturating_sub(fade_out.0).saturating_sub(fd.0).max(0));
        return (t.min(hold_at), false);
    }
    (t.min(last), t >= d)
}

/// Draw the program now and put it on the page; run out pages advance on their own.
fn tick_program(s: &mut Session, at: Option<Duration>) -> Result<Option<(Vec<u8>, u32, u32, usize)>> {
    const CMD: &str = "live.tick";
    loop {
        let Some(o) = s.live.program.clone() else {
            s.live.store.clear();
            publish_status(s);
            return Ok(None);
        };
        let Some(cue) = s.live.cue(o.cue).cloned() else {
            s.live.program = None;
            continue;
        };
        if o.finished {
            s.live.store.clear();
            publish_status(s);
            return Ok(None);
        }
        let elapsed = page_elapsed(&o, at);
        let (t, ran_out) = page_moment(s, &cue, o.page, elapsed);
        if ran_out && at.is_none() && o.held.is_none() {
            let Some(on) = s.live.program.as_mut() else { continue };
            if o.page + 1 < cue.page_count() {
                // the next page starts where this one ended
                let d = dur(cue.page_duration(o.page));
                on.page += 1;
                on.before = on.before.saturating_add(d);
                on.page_start = on.page_start.checked_add(d).unwrap_or_else(Instant::now);
                continue;
            }
            on.finished = true;
            continue;
        }
        let frame = draw(s, CMD, &cue, o.page, t, 0)?;
        let png = cfa4_export::encode_png(frame.0.clone(), frame.1, frame.2).map_err(|x| EngineError::Other(x.to_string()))?;
        s.live.store.publish(png);
        publish_status(s);
        return Ok(Some((frame.0, frame.1, frame.2, o.page)));
    }
}

/// The output width: the setting, else the sequence's (at most [`MAX_WIDTH`]).
fn out_width(s: &Session, want: u32) -> u32 {
    let seq = s.active_sequence().map_or(1920, |q| q.settings.width);
    let w = if want > 0 {
        want
    } else if s.live.width > 0 {
        s.live.width
    } else {
        seq
    };
    w.clamp(16, MAX_WIDTH)
}

/// A cue's page at `t`, as straight RGBA8.
fn draw(s: &mut Session, cmd: &str, cue: &Cue, page: usize, t: Tick, width: u32) -> Result<(Vec<u8>, u32, u32)> {
    let w = out_width(s, width);
    match &cue.source {
        Source::Credits { base, .. } => {
            let pg = cue.pages.get(page).ok_or_else(|| bad(cmd, format!("the cue has no page {page}")))?;
            let (mut rgba, w, h, k) = crate::credits::render_page(s, cmd, pg, t, w, false, false)?;
            draw_files(s, pg, base.as_deref(), t, &mut rgba, w, h, k);
            Ok((rgba, w, h))
        }
        Source::Clip { item, .. } => {
            let (cw, ch) = s.active_sequence().map(|q| (q.settings.width, q.settings.height)).ok_or(EngineError::NoSequence)?;
            let h = ((f64::from(w) * f64::from(ch) / f64::from(cw.max(1))).round() as u32).max(1);
            let k = f64::from(w) / f64::from(cw.max(1));
            let mut img = cfa4_render::image::Image::new(w as usize, h as usize);
            cfa4_render::graphic_clip::render_graphic(item, item.source_in + t, (cw, ch), &cfa4_geom::Affine::scale(k, k), &mut img);
            Ok((img.to_rgba8(), w, h))
        }
    }
}

/// A decoded picture, read once.
fn picture(s: &mut Session, path: &std::path::Path) -> Option<Arc<(u32, u32, Vec<u8>)>> {
    if let Some(p) = s.live.pictures.get(path) {
        return p.clone();
    }
    let read = std::fs::metadata(path)
        .ok()
        .filter(|m| m.len() <= MAX_PICTURE_BYTES)
        .and_then(|_| std::fs::read(path).ok())
        .and_then(|bytes| cfa4_media::still::decode_raster(&path.to_string_lossy(), &bytes).ok())
        .filter(|r| r.width > 0 && r.height > 0 && r.rgba.len() == r.width as usize * r.height as usize * 4)
        .map(|r| Arc::new((r.width, r.height, r.rgba)));
    if s.live.pictures.len() >= MAX_PICTURES {
        s.live.pictures.clear();
    }
    s.live.pictures.insert(path.to_path_buf(), read.clone());
    read
}

/// Draw `src` (straight RGBA8) over `dst` into the rectangle `x0, y0, w, h` (output pixels),
/// with `alpha`; sampled nearest, clipped to the picture.
fn blit(dst: &mut [u8], dw: u32, dh: u32, src: &(u32, u32, Vec<u8>), rect: (f64, f64, f64, f64), alpha: f64) {
    let (sw, sh, data) = (src.0 as f64, src.1 as f64, &src.2);
    let (x0, y0, w, h) = rect;
    if !(w.is_finite() && h.is_finite() && x0.is_finite() && y0.is_finite()) || w < 0.5 || h < 0.5 || alpha <= 0.0 {
        return;
    }
    let a = alpha.min(1.0);
    let (xa, xb) = (x0.max(0.0).floor() as i64, (x0 + w).min(f64::from(dw)).ceil() as i64);
    let (ya, yb) = (y0.max(0.0).floor() as i64, (y0 + h).min(f64::from(dh)).ceil() as i64);
    for y in ya.max(0)..yb.min(i64::from(dh)) {
        let sy = (((y as f64 + 0.5 - y0) / h) * sh).floor().clamp(0.0, sh - 1.0) as usize;
        for x in xa.max(0)..xb.min(i64::from(dw)) {
            let sx = (((x as f64 + 0.5 - x0) / w) * sw).floor().clamp(0.0, sw - 1.0) as usize;
            let si = (sy * src.0 as usize + sx) * 4;
            let di = (y as usize * dw as usize + x as usize) * 4;
            let (Some(sp), true) = (data.get(si..si + 4), di + 4 <= dst.len()) else { continue };
            let sa = f64::from(sp[3]) / 255.0 * a;
            let da = f64::from(dst[di + 3]) / 255.0;
            let oa = sa + da * (1.0 - sa);
            if oa <= 0.0 {
                continue;
            }
            for c in 0..3 {
                let v = (f64::from(sp[c]) * sa + f64::from(dst[di + c]) * da * (1.0 - sa)) / oa;
                dst[di + c] = v.round().clamp(0.0, 255.0) as u8;
            }
            dst[di + 3] = (oa * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// A card's fade at `t` for something that comes in `delay` after the page starts.
fn card_alpha(pg: &PageOut, t: Tick, delay: Tick) -> f64 {
    if pg.behaviour != Behaviour::Card {
        return 1.0;
    }
    let local = t.0 - delay.0;
    if local < 0 {
        return 0.0;
    }
    let fin = if pg.fade_in.0 > 0 { (local as f64 / pg.fade_in.0 as f64).min(1.0) } else { 1.0 };
    let left = pg.duration.0 - t.0;
    let fout = if pg.fade_out.0 > 0 { (left as f64 / pg.fade_out.0 as f64).clamp(0.0, 1.0) } else { 1.0 };
    fin.min(fout)
}

/// The page's picture background and its pictures (decoded files; videos are left out).
#[allow(clippy::too_many_arguments)]
fn draw_files(s: &mut Session, pg: &PageOut, base: Option<&std::path::Path>, t: Tick, rgba: &mut Vec<u8>, w: u32, h: u32, k: f64) {
    let Some((cw, ch)) = s.active_sequence().map(|q| (q.settings.width, q.settings.height)) else { return };
    let (fw, fh) = (f64::from(cw), f64::from(ch));
    // a picture background goes under what is drawn already
    if let Some(bg) = pg.background.as_ref().filter(|b| b.kind == cfa4_credits::BackgroundKind::Image)
        && let Some(path) = crate::credits::picture_path(base, &bg.file)
        && let Some(pic) = picture(s, &path)
    {
        let (pw, ph) = (f64::from(pic.0), f64::from(pic.1));
        let scale = match bg.fit {
            cfa4_credits::BackgroundFit::Fill => (fw / pw).max(fh / ph),
            cfa4_credits::BackgroundFit::Fit => (fw / pw).min(fh / ph),
        };
        let (dw, dh) = (pw * scale * k, ph * scale * k);
        let mut under = vec![0u8; rgba.len()];
        let opacity = (if bg.opacity.is_finite() { bg.opacity } else { 100.0 }).clamp(0.0, 100.0) / 100.0;
        blit(&mut under, w, h, &pic, ((f64::from(w) - dw) / 2.0, (f64::from(h) - dh) / 2.0, dw, dh), opacity);
        // the page's own pixels over the background
        let top = std::mem::replace(rgba, under);
        blit(rgba, w, h, &(w, h, top), (0.0, 0.0, f64::from(w), f64::from(h)), 1.0);
    }
    let (dx, dy) =
        cfa4_project::graphic_design::roll_offset(&pg.meta.roll, t, pg.duration, (cw, ch), [0.0, 0.0, fw.max(pg.content_width), pg.content_height.max(0.0)]);
    for pp in pg.pictures.iter().filter(|p| !p.video) {
        let Some(path) = crate::credits::picture_path(base, &pp.file) else { continue };
        let Some(pic) = picture(s, &path) else { continue };
        let (cx, cy) = (pp.center.0 + dx, pp.center.1 + dy);
        let (pw, ph) = (pp.size.0, pp.size.1);
        let rect = ((cx - pw / 2.0) * k, (cy - ph / 2.0) * k, pw * k, ph * k);
        blit(rgba, w, h, &pic, rect, card_alpha(pg, t, pp.delay));
    }
}

/// The live status: cues, preview, program, sound, server and OBS.
fn status(s: &Session) -> Value {
    let program = s.live.program.as_ref().map(|o| {
        let cue = s.live.cue(o.cue);
        json!({
            "cue": o.cue,
            "name": cue.map(|c| c.name.clone()),
            "page": o.page,
            "pages": cue.map_or(0, Cue::page_count),
            "elapsed": secs(page_elapsed(o, None)),
            "held": o.held.is_some(),
            "finished": o.finished,
        })
    });
    let (sw, sh) = s.active_sequence().map_or((1920, 1080), |q| (q.settings.width, q.settings.height));
    let w = out_width(s, 0);
    json!({
        "cues": s.live.cues.iter().map(Cue::to_json).collect::<Vec<_>>(),
        "preview": s.live.preview,
        "program": program,
        "onAir": s.live.program.is_some(),
        "audioMode": s.live.audio_mode.name(),
        "width": w,
        "height": ((f64::from(w) * f64::from(sh) / f64::from(sw.max(1))).round() as u32).max(1),
        "server": s.live.server.as_ref().map(|srv| json!({"port": srv.port(), "url": srv.url()})),
        "obs": s.live.obs.as_ref().map(|o| json!({"host": o.host, "port": o.port, "version": o.client.version, "source": o.source, "scene": o.scene, "last": o.last})),
        "seq": s.live.store.seq(),
    })
}

/// What the page reads (`/status.json`).
fn publish_status(s: &Session) {
    let o = s.live.program.as_ref();
    let audio_time = o.map_or(0.0, |o| secs(o.before + page_elapsed(o, None)));
    s.live.store.set_status(json!({
        "onAir": o.is_some_and(|o| !o.finished),
        "held": o.is_some_and(|o| o.held.is_some()),
        "cue": o.map(|o| o.cue),
        "page": o.map(|o| o.page),
        "audioMode": s.live.audio_mode.name(),
        "audioTime": audio_time,
    }));
}

fn status_cmd(s: &mut Session, _p: &Value) -> Result<Value> {
    Ok(status(s))
}

fn tick_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let r = tick_program(s, secs_p(p, "at"))?;
    let o = s.live.program.as_ref();
    Ok(json!({"seq": s.live.store.seq(), "onAir": o.is_some(), "page": r.map(|x| x.3), "finished": o.is_some_and(|o| o.finished)}))
}

/// A picture of the program (as on air now, or `at` seconds into its page) or of the preview
/// cue (`page`, `at`), for the Live window's monitors.
fn frame_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "live.frame";
    let which = str_p(p, "which").unwrap_or("program");
    let width = u64_p(p, "width").unwrap_or(640).clamp(16, u64::from(MAX_WIDTH)) as u32;
    let at = secs_p(p, "at");
    let (cue, page, elapsed) = match which {
        "program" => match s.live.program.clone() {
            Some(o) if !o.finished => (s.live.cue(o.cue).cloned(), o.page, page_elapsed(&o, at)),
            _ => (None, 0, Duration::ZERO),
        },
        "preview" => {
            let cue = s.live.preview.and_then(|id| s.live.cue(id).cloned());
            (cue, u64_p(p, "page").unwrap_or(0) as usize, at.unwrap_or(Duration::from_secs(2)))
        }
        other => return Err(bad(CMD, format!("`which` is program or preview, not {other}"))),
    };
    let Some(cue) = cue else {
        let png = cfa4_live::store::CLEAR_PNG;
        return Ok(json!({"which": which, "empty": true, "pngBase64": cfa4_project::gtemplate::base64_encode(png)}));
    };
    let page = page.min(cue.page_count().saturating_sub(1));
    let (t, _) = page_moment(s, &cue, page, elapsed);
    let (rgba, w, h) = draw(s, CMD, &cue, page, t, width)?;
    let png = cfa4_export::encode_png(rgba, w, h).map_err(|x| EngineError::Other(x.to_string()))?;
    Ok(
        json!({"which": which, "empty": false, "cue": cue.id, "page": page, "time": t.0, "width": w, "height": h, "pngBase64": cfa4_project::gtemplate::base64_encode(&png)}),
    )
}

fn audio_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "live.audio";
    if let Some(m) = str_p(p, "mode") {
        s.live.audio_mode = AudioMode::parse(m).ok_or_else(|| bad(CMD, format!("the audio mode is embedded, separate or ignore, not {m}")))?;
        let audio = s.live.program.as_ref().and_then(|o| s.live.cue(o.cue)).and_then(|c| c.audio.clone());
        s.live.store.set_audio(if s.live.audio_mode == AudioMode::Ignore { None } else { audio });
    }
    if let Some(w) = u64_p(p, "width") {
        s.live.width = w.clamp(16, u64::from(MAX_WIDTH)) as u32;
    }
    publish_status(s);
    Ok(status(s))
}

fn server_start(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "live.server.start";
    let port = u64_p(p, "port").unwrap_or(0);
    let port = u16::try_from(port).map_err(|_| bad(CMD, "a port is 0 to 65535"))?;
    if let Some(srv) = s.live.server.as_ref()
        && (port == 0 || srv.port() == port)
    {
        return Ok(status(s));
    }
    if let Some(mut old) = s.live.server.take() {
        old.stop();
    }
    let srv = LiveServer::start(port, s.live.store.clone()).map_err(|e| bad(CMD, format!("cannot listen on 127.0.0.1:{port}: {e}")))?;
    s.live.server = Some(srv);
    publish_status(s);
    Ok(status(s))
}

fn server_stop(s: &mut Session, _p: &Value) -> Result<Value> {
    if let Some(mut srv) = s.live.server.take() {
        srv.stop();
    }
    Ok(status(s))
}

/// What went wrong in OBS, in words that say what to do about it.
fn obs_message(e: &cfa4_live::ObsError) -> String {
    match e {
        // obs-websocket's InvalidInputKind: OBS without the obs-browser plugin (Ubuntu's own
        // package lacks it)
        cfa4_live::ObsError::Request { code: 605, request, .. } if request == "CreateInput" => {
            "this OBS has no Browser Source (the obs-browser plugin; Ubuntu's own OBS package leaves it out): install OBS from obsproject.com's PPA or Flatpak, or add a Browser Source by hand with the live page's address".into()
        }
        other => other.to_string(),
    }
}

/// Point OBS at the page (and, for the separate sound, a media source at `/audio`). Each part
/// is tried on its own; what failed is in `browserError` / `audioError`.
fn obs_point(s: &mut Session, cmd: &str) -> Result<Value> {
    let Some(url) = s.live.server.as_ref().map(LiveServer::url) else {
        return Err(bad(cmd, "start the live page first (live.server.start)"));
    };
    let st = status(s);
    let (w, h) = (st["width"].as_u64().unwrap_or(1920) as u32, st["height"].as_u64().unwrap_or(1080) as u32);
    let mode = s.live.audio_mode;
    let link = s.live.obs.as_mut().ok_or_else(|| bad(cmd, "not connected to OBS (live.obs.connect)"))?;
    let scene = link.scene.clone();
    let mut done = json!({"source": link.source, "url": url});
    match link.client.point_browser_source(&link.source, &url, w, h, scene.as_deref(), mode == AudioMode::Embedded) {
        Ok(r) => done["browser"] = json!(format!("{r:?}")),
        Err(e) => done["browserError"] = json!(obs_message(&e)),
    }
    if mode == AudioMode::Separate {
        match link.client.point_media_source(OBS_AUDIO_SOURCE, &format!("{url}audio"), scene.as_deref()) {
            Ok(r) => done["audio"] = json!(format!("{r:?}")),
            Err(e) => done["audioError"] = json!(obs_message(&e)),
        }
    }
    link.last = done.clone();
    Ok(done)
}

fn obs_connect(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "live.obs.connect";
    let host = str_p(p, "host").unwrap_or("127.0.0.1").to_string();
    // OBS is driven on this machine only, as the page is served on it only
    if !matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1") {
        return Err(bad(CMD, "OBS is reached on this machine only (127.0.0.1)"));
    }
    let port = u16::try_from(u64_p(p, "port").unwrap_or(u64::from(cfa4_live::obs::DEFAULT_PORT))).map_err(|_| bad(CMD, "a port is 1 to 65535"))?;
    let timeout = Duration::from_secs_f64(p.get("timeout").and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(5.0).clamp(0.2, 30.0));
    let client = ObsClient::connect(&host, port, str_p(p, "password"), timeout).map_err(|e| bad(CMD, e.to_string()))?;
    if let Some(mut old) = s.live.obs.take() {
        old.client.close();
    }
    let source = str_p(p, "source").map(str::trim).filter(|x| !x.is_empty()).unwrap_or(OBS_SOURCE).chars().take(200).collect();
    let scene = str_p(p, "scene").map(str::trim).filter(|x| !x.is_empty()).map(str::to_string);
    s.live.obs = Some(ObsLink { client, host, port, source, scene, last: Value::Null });
    if s.live.server.is_some() {
        obs_point(s, CMD)?;
    }
    Ok(status(s))
}

fn obs_setup(s: &mut Session, _p: &Value) -> Result<Value> {
    obs_point(s, "live.obs.setup")?;
    Ok(status(s))
}

fn obs_disconnect(s: &mut Session, _p: &Value) -> Result<Value> {
    if let Some(mut o) = s.live.obs.take() {
        o.client.close();
    }
    Ok(status(s))
}

fn on_air(s: &Session) -> std::result::Result<(), String> {
    if s.live.program.is_some() { Ok(()) } else { Err("nothing on air".into()) }
}

fn has_cues(s: &Session) -> std::result::Result<(), String> {
    if s.live.preview.is_some() { Ok(()) } else { Err("no live cue prepared".into()) }
}

fn spec(
    id: &'static str,
    label: &'static str,
    menu: &'static [&'static str],
    params: &'static str,
    enabled: crate::commands::Enabled,
    run: crate::commands::Run,
) -> CommandSpec {
    // live state is the session's, not the project's: nothing is journalled or undone
    CommandSpec { id, label, menu, shortcut: None, params, enabled, run, journal: false }
}

pub fn commands() -> Vec<CommandSpec> {
    const CUE: &str = r#"{"name":str?,"clip":graphic clip id? (a title, lower third or credits page of the sequence),"path"|"csv"|"template"|"styling"|"sheet"|"pictureDir": credits as graphics.credits.preview?,"audio":path?|"audioClip":clip id? (the cue's sound),"holdCards":bool=true (a card waits for NEXT instead of fading out)}"#;
    vec![
        spec("live.window", "Live Graphics…", &["Graphics and Titles"], "{} → as live.status (the Qt GUI opens the Live window)", always, status_cmd),
        spec(
            "live.status",
            "Live Status",
            &[],
            "{} → {cues, preview, program: {cue, name, page, pages, elapsed, held, finished}, onAir, audioMode, width, height, server: {port, url}, obs, seq}",
            always,
            status_cmd,
        ),
        spec("live.cue.add", "Add Live Cue", &[], CUE, has_seq, cue_add),
        spec("live.cue.remove", "Remove Live Cue", &[], r#"{"id":cue}"#, always, cue_remove),
        spec("live.preview", "Preview Live Cue", &[], r#"{"id":cue?|"next":true}"#, has_cues, preview),
        spec("live.take", "Take", &[], "{} → the preview cue on air from its first page; the next cue into preview", has_cues, take),
        spec("live.next", "Next Live Page", &[], "{} → the next page of the cue on air (after the last: TAKE the preview cue)", on_air, |s, _| step(s, true)),
        spec("live.previous", "Previous Live Page", &[], "{} → the page before (on the first: from its start)", on_air, |s, _| step(s, false)),
        spec("live.hold", "Hold", &[], "{} → the program stops where it is", on_air, |s, _| hold(s, true)),
        spec("live.continue", "Continue", &[], "{} → the program goes on from where it stopped", on_air, |s, _| hold(s, false)),
        spec(
            "live.update",
            "Update Live Cue",
            &[],
            r#"{"id":cue? (default: on air, else preview), and new credits parameters?} → reads the spreadsheet (or clip) again; on air it changes at once"#,
            has_seq,
            cue_update,
        ),
        spec("live.out", "Out", &[], "{} → nothing on air (a transparent picture)", always, out),
        spec(
            "live.tick",
            "Live Tick",
            &[],
            r#"{"at":seconds into the page?} → draws the program for the page and advances run-out pages → {seq, onAir, page, finished}"#,
            has_seq,
            tick_cmd,
        ),
        spec(
            "live.frame",
            "Live Picture",
            &[],
            r#"{"which":"program"|"preview"="program","width":px=640,"page":index? (preview),"at":seconds? } → {pngBase64, width, height, cue, page, time, empty}"#,
            has_seq,
            frame_cmd,
        ),
        spec("live.audio", "Live Sound", &[], r#"{"mode":"embedded"|"separate"|"ignore"?,"width":px? (program picture width)}"#, always, audio_cmd),
        spec(
            "live.server.start",
            "Start Live Page",
            &[],
            r#"{"port":0..65535=0 (any free port)} → the page for OBS's Browser Source on http://127.0.0.1:port/"#,
            always,
            server_start,
        ),
        spec("live.server.stop", "Stop Live Page", &[], "{}", always, server_stop),
        spec(
            "live.obs.connect",
            "Connect to OBS",
            &[],
            r#"{"host":"127.0.0.1","port":4455,"password":str?,"source":"CFA4 Live","scene":str? (default the program scene),"timeout":seconds=5} → connects and, with the live page running, points OBS's Browser Source at it"#,
            always,
            obs_connect,
        ),
        spec(
            "live.obs.setup",
            "Point OBS at the Live Page",
            &[],
            "{} → creates or updates OBS's Browser Source (and the sound's media source in the separate mode)",
            always,
            obs_setup,
        ),
        spec("live.obs.disconnect", "Disconnect from OBS", &[], "{}", always, obs_disconnect),
    ]
}

#[cfg(test)]
#[path = "live_tests.rs"]
mod tests;
