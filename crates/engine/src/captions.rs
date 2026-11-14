//! Caption commands: caption tracks, caption editing, caption file import/export.
//!
//! Ids (`captions.*`) follow Premiere's Sequence ▸ Captions menu and the Text panel's Captions tab.
//! Captions are addressed by id (`caption`, `captions`); without one, commands use the caption
//! selection (`EditorState::caption_selection`) or the caption under the playhead. Caption tracks
//! are addressed by id or `"C1"`, `"C2"`… (top first); the default is the first caption track.

use serde_json::{Value, json};

use cfa4_captions::{Document, Format, WriteOptions};
use cfa4_edit as edit;
use cfa4_edit::Edge;
use cfa4_project::{CaptionAlign, CaptionAnchor, CaptionFormat, CaptionStyle, CaptionTrack, ClipId, SequenceSettings, TrackId};
use cfa4_time::{Tick, TimeDisplay, format_time};

use crate::commands::{CommandSpec, always, bad, bool_p, f64_p, has_seq, str_p, time_p, u64_p};
use crate::{EngineError, Result, Session};

type Run = fn(&mut Session, &Value) -> Result<Value>;
type Enabled = fn(&Session) -> std::result::Result<(), String>;

fn spec(
    id: &'static str,
    label: &'static str,
    menu: &'static [&'static str],
    shortcut: Option<&'static str>,
    params: &'static str,
    enabled: Enabled,
    run: Run,
) -> CommandSpec {
    CommandSpec { id, label, menu, shortcut, params, enabled, run, journal: true }
}

fn has_caption_track(s: &Session) -> std::result::Result<(), String> {
    has_seq(s)?;
    if s.active_sequence().is_some_and(|q| !q.caption_tracks.is_empty()) { Ok(()) } else { Err("the sequence has no caption track".into()) }
}

fn has_captions(s: &Session) -> std::result::Result<(), String> {
    has_caption_track(s)?;
    if s.active_sequence().is_some_and(|q| q.caption_tracks.iter().any(|t| !t.captions.is_empty())) { Ok(()) } else { Err("there are no captions".into()) }
}

/// The caption track named by `track` (id or "C1"…), else the first one.
pub fn track_param(s: &Session, p: &Value) -> Option<TrackId> {
    let seq = s.active_sequence()?;
    match p.get("track") {
        Some(Value::Number(n)) => n.as_u64().map(TrackId).filter(|t| seq.caption_track(*t).is_some()),
        Some(Value::String(name)) => {
            let idx: usize = name.trim_start_matches(['C', 'c']).parse().ok()?;
            seq.caption_tracks.get(idx.checked_sub(1)?).map(|t| t.id)
        }
        _ => seq.caption_tracks.first().map(|t| t.id),
    }
}

fn caption_ids(s: &Session, p: &Value) -> Vec<ClipId> {
    if let Some(a) = p.get("captions").and_then(Value::as_array) {
        return a.iter().filter_map(Value::as_u64).map(ClipId).collect();
    }
    if let Some(c) = u64_p(p, "caption") {
        return vec![ClipId(c)];
    }
    if !s.state.caption_selection.is_empty() {
        return s.state.caption_selection.clone();
    }
    // the caption under the playhead on the first caption track that has one
    let t = s.playhead();
    s.active_sequence().and_then(|q| q.caption_tracks.iter().find_map(|tr| tr.caption_at(t).map(|c| c.id))).into_iter().collect()
}

fn one_caption(s: &Session, p: &Value, cmd: &str) -> Result<ClipId> {
    caption_ids(s, p).first().copied().ok_or_else(|| bad(cmd, "no caption (pass `caption` or select one)"))
}

fn parse_color(v: &Value) -> Option<[u8; 4]> {
    if let Some(a) = v.as_array() {
        let c: Vec<u8> = a.iter().filter_map(|x| x.as_u64().map(|n| n.min(255) as u8)).collect();
        return match c.len() {
            3 => Some([c[0], c[1], c[2], 255]),
            4 => Some([c[0], c[1], c[2], c[3]]),
            _ => None,
        };
    }
    let h = v.as_str()?;
    let h = h.strip_prefix('#').unwrap_or(h);
    if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let b = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        6 => Some([b(0)?, b(2)?, b(4)?, 255]),
        8 => Some([b(0)?, b(2)?, b(4)?, b(6)?]),
        _ => None,
    }
}

fn hex(c: [u8; 4]) -> String {
    format!("#{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3])
}

/// The caption format of a file, if it is one (extension `.srt` / `.vtt` / `.scc` / `.mcc` /
/// `.stl` / `.ttml` / `.dfxp` / `.xml`, confirmed by content).
pub fn detect(path: &str, bytes: &[u8]) -> Option<Format> {
    let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if !matches!(ext.as_deref(), Some("srt" | "vtt" | "scc" | "webvtt" | "mcc" | "stl" | "ttml" | "dfxp" | "xml")) {
        return None;
    }
    cfa4_captions::detect(bytes, ext.as_deref())
}

/// Import a caption file as a new caption track in the active sequence (a new sequence is made
/// when none is open), snapped to the sequence's frames. One undo step.
pub fn import(s: &mut Session, path: &str, bytes: &[u8], format: Format, name: Option<&str>) -> Result<Value> {
    let doc = cfa4_captions::parse(bytes, format).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    import_document(s, path, doc, format.name(), format.track_format(), format == Format::Scc, name)
}

/// Bytes of a timed text document read from an MXF file, at most this many.
const MAX_TIMED_TEXT_FILE: u64 = 1 << 30;

/// Whether `bytes` is a D-Cinema subtitle XML file (ST 428-7 `SubtitleReel`, `.xml`).
pub fn is_dcst(path: &str, bytes: &[u8]) -> bool {
    let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    ext.as_deref() == Some("xml") && cfa4_captions::dcst::sniff(&cfa4_captions::decode_text(&bytes[..bytes.len().min(4096)]))
}

/// A timed text XML document (ST 428-7 D-Cinema subtitles or IMSC1 TTML) as captions, with the
/// format name and the reel's language.
/// A parsed timed text document: its cues, the image subtitles to place (ST 428-7 only), the
/// format name and the language.
struct TimedTextDoc {
    doc: Document,
    format: &'static str,
    language: Option<String>,
    images: Vec<cfa4_captions::dcst::ImageSubtitle>,
    intrinsic_resolution: Option<String>,
}

fn parse_timed_text(xml: &[u8]) -> std::result::Result<TimedTextDoc, String> {
    let text = cfa4_captions::decode_text(xml);
    if cfa4_captions::dcst::sniff(&text) {
        let reel = cfa4_captions::dcst::parse_reel(&text).map_err(|e| e.to_string())?;
        return Ok(TimedTextDoc {
            doc: reel.document,
            format: "D-Cinema subtitles (ST 428-7)",
            language: reel.language,
            images: reel.images,
            intrinsic_resolution: reel.intrinsic_resolution,
        });
    }
    let doc = cfa4_captions::ttml::parse(&text).map_err(|e| e.to_string())?;
    Ok(TimedTextDoc { doc, format: "IMSC1 / TTML", language: None, images: Vec::new(), intrinsic_resolution: None })
}

/// Import a D-Cinema subtitle XML file (ST 428-7) as a caption track. Its image subtitles come
/// from the PNG files named by their UUID (`<uuid>.png`) beside the XML.
pub fn import_dcst(s: &mut Session, path: &str, bytes: &[u8]) -> Result<Value> {
    let td = parse_timed_text(bytes).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    let dir = std::path::Path::new(path).parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    s.edit_group("Import Subtitles", |s| {
        import_reel(s, path, td, None, &mut |s, id| {
            let png = dir.join(format!("{}.png", uuid_text(&id))).to_string_lossy().into_owned();
            let size = s.services.file_size(&png).map_err(|e| format!("{png}: {e}"))?;
            if size > MAX_SUBTITLE_PNG {
                return Err(format!("{png}: larger than {MAX_SUBTITLE_PNG} bytes"));
            }
            let bytes = s.services.read_file(&png).map_err(|e| format!("{png}: {e}"))?;
            Ok((png, bytes))
        })
    })
}

/// Import a DCP or IMF timed text track file (SMPTE ST 429-5 / ST 2067-2 MXF) as a caption track;
/// its PNG image subtitles are written beside it and placed on video tracks above the others.
/// `None` when `path` is not an MXF timed text track file (other MXF files import as media).
pub fn import_timed_text_mxf(s: &mut Session, path: &str) -> Option<Result<Value>> {
    let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if ext.as_deref() != Some("mxf") {
        return None;
    }
    let src: Box<dyn cfa4_mxf::ByteSource> = match s.services.reader(path) {
        Some(Ok(r)) => Box::new(MxfReader(r)),
        Some(Err(_)) => return None,
        None => {
            // hosts without random access: only small files are read whole
            let b = s.services.read_file(path).ok()?;
            if b.len() as u64 > MAX_TIMED_TEXT_FILE {
                return None;
            }
            Box::new(b)
        }
    };
    let mut head = vec![0u8; src.len().min(65_536 + 1024) as usize];
    src.read_at(0, &mut head).ok()?;
    if !cfa4_mxf::is_timed_text(&head) {
        return None;
    }
    let parsed = cfa4_mxf::open_timed_text(src.as_ref()).map_err(|e| e.to_string()).and_then(|tt| {
        let xml = tt.read_document(src.as_ref()).map_err(|e| e.to_string())?;
        let mut td = parse_timed_text(&xml)?;
        td.doc.warnings.extend(tt.warnings.iter().cloned());
        Ok((td, tt))
    });
    Some(match parsed {
        Ok((td, tt)) => {
            let stem = std::path::Path::new(path).file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Subtitles".into());
            let name = match &td.language {
                Some(l) => format!("{stem} ({l})"),
                None => stem,
            };
            s.edit_group("Import Subtitles", |s| {
                import_reel(s, path, td, Some(&name), &mut |s, id| {
                    let i = tt.resources.iter().position(|r| r.id == id).ok_or_else(|| format!("resource urn:uuid:{} is not in the file", uuid_text(&id)))?;
                    if tt.resources.get(i).and_then(|r| r.data).is_some_and(|d| d.size > MAX_SUBTITLE_PNG) {
                        return Err(format!("resource urn:uuid:{} is larger than {MAX_SUBTITLE_PNG} bytes", uuid_text(&id)));
                    }
                    let bytes = tt.read_resource(src.as_ref(), i).map_err(|e| e.to_string())?;
                    if !bytes.starts_with(PNG_MAGIC) {
                        return Err(format!("resource urn:uuid:{} is not a PNG image", uuid_text(&id)));
                    }
                    let out = write_extracted(s, path, &uuid_text(&id), &bytes).map_err(|e| e.to_string())?;
                    Ok((out, bytes))
                })
            })
        }
        Err(e) => Err(EngineError::Other(format!("{path}: {e}"))),
    })
}

/// The largest PNG subtitle image read (a 4K RGBA frame compresses well below this).
const MAX_SUBTITLE_PNG: u64 = 64 << 20;
const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
/// Video tracks made for overlapping image subtitles at most.
const MAX_IMAGE_TRACKS: usize = 8;

/// `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` in lower case.
fn uuid_text(id: &[u8; 16]) -> String {
    let h: String = id.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

/// Write a PNG extracted from the timed text file `path` beside it as `<stem>_<uuid>.png` (`" (N)"`
/// added when another file has that name); an identical file already there is used as it is.
fn write_extracted(s: &Session, path: &str, uuid: &str, bytes: &[u8]) -> Result<String> {
    let original = std::path::Path::new(path);
    let stem = original.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "subtitle".into());
    for number in 1..=1_000 {
        let name = if number == 1 { format!("{stem}_{uuid}.png") } else { format!("{stem}_{uuid} ({number}).png") };
        let candidate = original.with_file_name(name).to_string_lossy().into_owned();
        if s.services.file_size(&candidate).is_ok_and(|n| n == bytes.len() as u64) && s.services.read_file(&candidate).is_ok_and(|b| b == bytes) {
            return Ok(candidate);
        }
        match s.services.write_file_new(&candidate, bytes) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(EngineError::Other(format!("{candidate}: {e}"))),
        }
    }
    Err(EngineError::Other(format!("{path}: no unused subtitle image file name after 1000 attempts")))
}

/// A PNG subtitle resource by UUID: the file it is in and its bytes.
type FetchPng<'a> = dyn FnMut(&Session, [u8; 16]) -> std::result::Result<(String, Vec<u8>), String> + 'a;

/// Import a parsed timed text document: its cues as a caption track (also when it has neither
/// cues nor images, so the import is visible), its image subtitles as still clips on new video
/// tracks above the others.
fn import_reel(s: &mut Session, path: &str, td: TimedTextDoc, name: Option<&str>, fetch: &mut FetchPng<'_>) -> Result<Value> {
    let TimedTextDoc { doc, format, images, intrinsic_resolution, .. } = td;
    let stem = std::path::Path::new(path).file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Subtitles".into());
    let mut report = if !doc.cues.is_empty() || images.is_empty() {
        import_document(s, path, doc, format, CaptionFormat::Subtitle, false, name)?
    } else {
        let seq = target_sequence(s, &stem)?;
        json!({"format": format, "sequence": seq.0, "captions": 0, "warnings": doc.warnings})
    };
    if images.is_empty() {
        return Ok(report);
    }
    let (placed, tracks, warnings) = place_images(s, name.unwrap_or(&stem), &images, intrinsic_resolution.as_deref(), fetch)?;
    report["images"] = json!(placed);
    report["imageTracks"] = json!(tracks);
    if let Some(w) = report["warnings"].as_array_mut() {
        w.extend(warnings.into_iter().map(Value::from));
    }
    Ok(report)
}

/// The active sequence, or a new one named `stem` when there is none.
fn target_sequence(s: &mut Session, stem: &str) -> Result<cfa4_project::ItemId> {
    if let Some(id) = s.state.active_sequence.filter(|id| s.project.sequence(*id).is_some()) {
        return Ok(id);
    }
    let id = s.edit("New Sequence", |p, _| Ok(p.new_sequence(stem, SequenceSettings::default(), 3, 3, None)))?;
    s.state.active_sequence = Some(id);
    s.state.open_sequences.push(id);
    Ok(id)
}

/// Scale (percent) for PNGs made for `intrinsic` (`2K`/`4K`, ST 428-7 §5.1.1) shown in a
/// `width`-pixel-wide sequence: 200 for 2K images in a 4K sequence, 50 the other way round.
fn image_scale(intrinsic: Option<&str>, width: u32) -> f64 {
    let four_k = width > 3000;
    match intrinsic.map(|r| r.trim().to_ascii_uppercase()) {
        Some(r) if r == "2K" && four_k => 200.0,
        Some(r) if r == "4K" && !four_k => 50.0,
        _ => 100.0,
    }
}

/// Import the PNG of each image subtitle (one bin) and place them in the active sequence: each
/// at its times on the first new video track free there, positioned by its alignment, with its
/// fades as Opacity keyframes. Returns the clips placed, the tracks added and the warnings.
fn place_images(
    s: &mut Session,
    name: &str,
    images: &[cfa4_captions::dcst::ImageSubtitle],
    intrinsic: Option<&str>,
    fetch: &mut FetchPng<'_>,
) -> Result<(usize, usize, Vec<String>)> {
    use cfa4_project::{Keyframe, ParamValue, TrackKind};
    use cfa4_time::TimeRange;
    let mut warnings = Vec::new();
    let seq_id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let bin_name = format!("{name} images");
    let bin = s.edit(&format!("New Bin {bin_name}"), |p, _| Ok(p.add_bin(&bin_name, None)))?;
    // `None`: the resource failed (warned once; the subtitles that use it are skipped)
    let mut items: std::collections::HashMap<[u8; 16], Option<cfa4_project::ItemId>> = std::collections::HashMap::new();
    for img in images {
        if items.contains_key(&img.resource) {
            continue;
        }
        let item = match fetch(s, img.resource) {
            Ok((file, bytes)) => match crate::commands::import_bytes(s, &file, bytes.into(), Some(bin)) {
                Ok(id) => Some(id),
                Err(e) => {
                    warnings.push(format!("{file}: {e}"));
                    None
                }
            },
            Err(e) => {
                warnings.push(e);
                None
            }
        };
        items.insert(img.resource, item);
    }
    let (placed, tracks, skipped) = s.edit("Place Subtitle Images", |p, _| {
        let (w, h, rate) = {
            let q = p.sequence(seq_id).ok_or(EngineError::NoSequence)?;
            (q.settings.width, q.settings.height, q.settings.frame_rate)
        };
        let scale = image_scale(intrinsic, w);
        let frame = rate.frame_duration();
        let mut tracks: Vec<cfa4_project::Track> = Vec::new();
        let (mut placed, mut skipped) = (0usize, 0usize);
        for img in images {
            let Some(&Some(item)) = items.get(&img.resource) else { continue };
            let start = rate.snap_nearest(img.start);
            let dur = Tick(rate.snap_nearest(img.end).0.saturating_sub(start.0)).max(frame);
            let range = TimeRange::new(start, dur);
            let free = tracks.iter().position(|t| !t.items.iter().any(|i| i.range().overlaps(&range)));
            let at = match free {
                Some(i) => i,
                None if tracks.len() < MAX_IMAGE_TRACKS => {
                    let label = if tracks.is_empty() { name.to_string() } else { format!("{name} {}", tracks.len() + 1) };
                    tracks.push(cfa4_project::Track::new(cfa4_project::TrackId(p.alloc_id()), TrackKind::Video, label));
                    tracks.len() - 1
                }
                None => {
                    skipped += 1;
                    continue;
                }
            };
            let Some(mut ti) = p.make_track_item(item, TrackKind::Video, start, TimeRange::new(Tick::ZERO, dur), rate) else { continue };
            let (iw, ih) = p.source_size(item).unwrap_or((w, h));
            let k = scale / 100.0;
            let (cx, cy) = img.centre((f64::from(w), f64::from(h)), (f64::from(iw) * k, f64::from(ih) * k));
            if let Some(m) = ti.effects.iter_mut().find(|e| e.effect == "motion") {
                if let Some(pp) = m.param_mut("position") {
                    pp.value = ParamValue::Vec2(cfa4_geom::Vec2::new(cx, cy));
                }
                if let Some(sp) = m.param_mut("scale") {
                    sp.value = ParamValue::Float(scale);
                }
            }
            if let Some(o) = ti.effects.iter_mut().find(|e| e.effect == "opacity").and_then(|o| o.param_mut("opacity")) {
                o.keyframes = fade_keyframes(ti.duration, rate.snap_nearest(img.fade_in), rate.snap_nearest(img.fade_out))
                    .into_iter()
                    .map(|(t, v)| Keyframe::new(t, ParamValue::Float(v)))
                    .collect();
            }
            for e in &mut ti.effects {
                cfa4_project::resolve_auto_points(e, (w, h), (iw, ih));
            }
            if let Some(spot) = &img.spot {
                ti.name = format!("{} #{spot}", ti.name);
            }
            if let Some(t) = tracks.get_mut(at) {
                t.items.push(ti);
                placed += 1;
            }
        }
        let added = tracks.len();
        let q = p.sequence_mut(seq_id).ok_or(EngineError::NoSequence)?;
        for mut t in tracks {
            t.sort();
            q.video_tracks.push(t);
        }
        q.check().map_err(EngineError::Other)?;
        Ok((placed, added, skipped))
    })?;
    if skipped > 0 {
        warnings.push(format!("{skipped} image subtitle(s) overlap more than {MAX_IMAGE_TRACKS} others and were not placed"));
    }
    Ok((placed, tracks, warnings))
}

/// Opacity keyframes (clip time, percent) for fades of `fade_in` and `fade_out` in a `dur`-long
/// clip: 0 % at the start and end, 100 % between; fades longer than the clip share it.
fn fade_keyframes(dur: Tick, fade_in: Tick, fade_out: Tick) -> Vec<(Tick, f64)> {
    let d = dur.0.max(0);
    let (mut fi, mut fo) = (fade_in.0.clamp(0, d), fade_out.0.clamp(0, d));
    if fi.saturating_add(fo) > d {
        // in proportion; i128 so the product cannot overflow
        fi = (i128::from(fi) * i128::from(d) / i128::from(fi.saturating_add(fo)).max(1)) as i64;
        fo = d - fi;
    }
    let mut k = Vec::new();
    if fi > 0 {
        k.push((Tick::ZERO, 0.0));
    }
    k.push((Tick(fi), 100.0));
    if fo > 0 {
        if d - fo > fi {
            k.push((Tick(d - fo), 100.0));
        }
        k.push((Tick(d), 0.0));
    }
    if k.len() == 1 { Vec::new() } else { k }
}

/// Import the closed captions carried in an MXF file's ANC data (SMPTE ST 436-1) as caption
/// tracks, one per channel that holds captions: from ST 334-2 caption distribution packets
/// CEA-608 CC1–CC4 and CEA-708 services 1–6 ("{stem} (CC1)", "{stem} (SERVICE2)"); then from
/// the CEA-608 byte pairs of ST 334-1 packets and of digitised line 21 VI lines the CC1–CC4
/// channels the packets did not already give. `None` when `path` is not
/// an MXF file with ANC or VBI data or that data holds no captions; the file's picture and sound
/// import as media either way.
pub fn import_anc_captions(s: &mut Session, path: &str) -> Option<Result<Vec<Value>>> {
    let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if ext.as_deref() != Some("mxf") {
        return None;
    }
    let src: Box<dyn cfa4_mxf::ByteSource> = match s.services.reader(path) {
        Some(Ok(r)) => Box::new(MxfReader(r)),
        Some(Err(_)) => return None,
        None => {
            // hosts without random access: only small files are read whole
            let b = s.services.read_file(path).ok()?;
            if b.len() as u64 > MAX_TIMED_TEXT_FILE {
                return None;
            }
            Box::new(b)
        }
    };
    let mut head = vec![0u8; src.len().min(65_536 + 1024) as usize];
    src.read_at(0, &mut head).ok()?;
    if !cfa4_mxf::has_anc(&head) {
        return None;
    }
    let anc = match cfa4_mxf::read_anc(src.as_ref()) {
        Ok(a) => a,
        Err(e) => return Some(Err(EngineError::Other(format!("{path}: ANC data: {e}")))),
    };
    let rate = cfa4_time::FrameRate::new(i64::from(anc.edit_rate.num), i64::from(anc.edit_rate.den));
    let stem = std::path::Path::new(path).file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Captions".into());
    let mut reports = Vec::new();
    let mut warnings = anc.warnings.clone();
    // channels taken from caption distribution packets: line 21 / ST 334-1 data of the same
    // channel is the same captions and is not imported twice
    let mut done: Vec<String> = Vec::new();
    let cdps = anc.cdps();
    if !cdps.is_empty() {
        let (channels, w) = cfa4_captions::mcc::decode_cdp_channels(cdps.into_iter().map(|(f, b)| (f as i64, b)), rate);
        warnings.extend(w);
        for ch in channels {
            // the file's warnings go with its first track
            let doc = Document { cues: ch.cues, warnings: std::mem::take(&mut warnings), ..Default::default() };
            let format = if ch.cea708 { CaptionFormat::Cea708 } else { CaptionFormat::Cea608 };
            match import_document(s, path, doc, "MXF ANC captions", format, false, Some(&format!("{stem} ({})", ch.name))) {
                Ok(r) => reports.push(r),
                Err(e) => return Some(Err(e)),
            }
            done.push(ch.name);
        }
    }
    let fields = line21_pairs(&anc);
    for (name, field, channel2) in [("CC1", 0usize, false), ("CC2", 0, true), ("CC3", 1, false), ("CC4", 1, true)] {
        if done.iter().any(|d| d == name) {
            continue;
        }
        let Some(pairs) = fields.get(field) else { continue };
        let cues = cfa4_captions::scc::decode_channel(pairs.iter().copied(), rate, channel2);
        if cues.is_empty() {
            continue;
        }
        let doc = Document { cues, warnings: std::mem::take(&mut warnings), ..Default::default() };
        match import_document(s, path, doc, "MXF line 21 captions", CaptionFormat::Cea608, false, Some(&format!("{stem} ({name})"))) {
            Ok(r) => reports.push(r),
            Err(e) => return Some(Err(e)),
        }
    }
    if reports.is_empty() { None } else { Some(Ok(reports)) }
}

/// The CEA-608 byte pairs `(frame, b1, b2)` of field 1 and field 2 in an MXF file's ST 334-1
/// packets and digitised line 21 VI lines (the first line per frame and field that decodes), in
/// frame order.
fn line21_pairs(anc: &cfa4_mxf::AncData) -> [Vec<(i64, u8, u8)>; 2] {
    let mut by_frame: std::collections::BTreeMap<(usize, bool), (u8, u8)> = std::collections::BTreeMap::new();
    for p in anc.cea608() {
        by_frame.entry((p.frame, p.field2)).or_insert((p.bytes[0], p.bytes[1]));
    }
    for (frame, lines) in anc.vi.iter().enumerate() {
        for l in lines {
            let decoded = match l.coding {
                // luma only
                1 | 4 | 7 => cfa4_captions::cea608::decode_line21(&l.samples),
                // luma and colour difference, Cb Y Cr Y: the luma samples are the odd ones
                3 | 6 | 9 => cfa4_captions::cea608::decode_line21(&l.samples.iter().skip(1).step_by(2).copied().collect::<Vec<u16>>()),
                _ => None,
            };
            if let Some((a, b)) = decoded {
                by_frame.entry((frame, l.field2())).or_insert((a, b));
            }
        }
    }
    let mut out = [Vec::new(), Vec::new()];
    for ((frame, field2), (a, b)) in by_frame {
        if let Some(v) = out.get_mut(usize::from(field2)) {
            v.push((frame as i64, a, b));
        }
    }
    // BTreeMap order is (frame, field): each field's pairs are already in frame order
    out
}

/// The host's random-access reader as an MXF byte source.
struct MxfReader(cfa4_media::SharedReader);

impl cfa4_mxf::ByteSource for MxfReader {
    fn len(&self) -> u64 {
        self.0.len()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        self.0.read_at(offset, buf)
    }
}

/// Add parsed captions as a new caption track in the active sequence (a new sequence is made when
/// none is open), snapped to the sequence's frames. One undo step.
fn import_document(
    s: &mut Session,
    path: &str,
    mut doc: Document,
    format_name: &str,
    track_format: CaptionFormat,
    black_background: bool,
    name: Option<&str>,
) -> Result<Value> {
    let stem = std::path::Path::new(path).file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Captions".into());
    let track_name = name.map(str::to_string).unwrap_or_else(|| stem.clone());
    let seq_id = target_sequence(s, &stem)?;
    let rate = s.project.sequence(seq_id).map(|q| q.settings.frame_rate).unwrap_or_default();
    doc.snap_to_frames(rate);
    let n = doc.cues.len();
    let warnings = doc.warnings.clone();
    let tid = s.edit(&format!("Import Captions {stem}"), |p, st| {
        let tid = TrackId(p.alloc_id());
        let mut next = p.next_id;
        let mut alloc = || {
            let v = next;
            next += 1;
            v
        };
        let mut track = cfa4_captions::track_from_document(&doc, tid, &track_name, track_format, &mut alloc);
        if black_background {
            track.style.background_color = [0, 0, 0, 255];
        }
        p.next_id = next;
        let q = p.sequence_mut(seq_id).ok_or(EngineError::NoSequence)?;
        q.caption_tracks.insert(0, track);
        q.check().map_err(EngineError::Other)?;
        st.caption_selection.clear();
        Ok(tid)
    })?;
    Ok(json!({"format": format_name, "sequence": seq_id.0, "track": tid.0, "captions": n, "warnings": warnings}))
}

/// Delete captions (Clear / Ripple Delete with captions selected).
pub fn delete(s: &mut Session, ids: &[ClipId], ripple: bool) -> Result<Value> {
    if ids.is_empty() {
        return Err(bad("captions.delete", "no captions"));
    }
    let ids = ids.to_vec();
    let n = s.edit_sequence(if ripple { "Ripple Delete Captions" } else { "Delete Captions" }, |q, _, st| {
        let n = edit::captions::delete_captions(q, &ids, ripple)?;
        st.caption_selection.retain(|c| !ids.contains(c));
        Ok(n)
    })?;
    Ok(json!({"deleted": n}))
}

/// Copy the selected captions to the caption clipboard (Edit ▸ Copy / Cut with captions selected).
/// Returns how many were copied.
pub fn copy(s: &mut Session) -> usize {
    let Some(q) = s.active_sequence() else { return 0 };
    let sel = &s.state.caption_selection;
    let picked: Vec<(usize, cfa4_project::Caption)> =
        q.caption_tracks.iter().enumerate().flat_map(|(ti, t)| t.captions.iter().filter(|c| sel.contains(&c.id)).map(move |c| (ti, c.clone()))).collect();
    let min = picked.iter().map(|(_, c)| c.start).min().unwrap_or_default();
    s.state.caption_clipboard = picked.into_iter().map(|(ti, c)| (ti, cfa4_project::Caption { start: c.start - min, ..c })).collect();
    s.state.clipboard.clear();
    s.state.clipboard_markers.clear();
    s.state.caption_clipboard.len()
}

/// Paste the caption clipboard at the playhead, each caption on the caption track it was copied
/// from (the last one when that track is gone), overwriting what is there. Paste Insert first opens
/// a gap the copied span long on those tracks. The pasted captions are selected and the playhead
/// moves to their end.
pub fn paste(s: &mut Session, insert: bool) -> Result<Value> {
    let at = s.playhead();
    let clip = s.state.caption_clipboard.clone();
    let span = clip.iter().map(|(_, c)| c.end()).max().unwrap_or_default();
    let mut by_track: Vec<(TrackId, Vec<cfa4_project::Caption>)> = Vec::new();
    {
        let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
        for (ti, c) in clip {
            let Some(t) = q.caption_tracks.get(ti).or(q.caption_tracks.last()) else {
                return Err(bad("edit.paste", "the sequence has no caption track to paste into"));
            };
            let c = cfa4_project::Caption { start: c.start + at, ..c };
            match by_track.iter_mut().find(|(id, _)| *id == t.id) {
                Some((_, v)) => v.push(c),
                None => by_track.push((t.id, vec![c])),
            }
        }
    }
    let ids = s.edit_sequence(if insert { "Paste Insert Captions" } else { "Paste Captions" }, |q, ctx, st| {
        let mut ids = Vec::new();
        for (tid, caps) in &by_track {
            if insert && let Some(tr) = q.caption_track_mut(*tid).filter(|t| !t.locked) {
                edit::captions::insert_gap(tr, at, span, ctx);
            }
            ids.extend(edit::captions::overwrite_captions(q, *tid, caps, ctx)?);
        }
        st.caption_selection = ids.clone();
        st.selection.clear();
        Ok(ids)
    })?;
    s.set_playhead(at + span);
    Ok(json!({"captions": ids.iter().map(|c| c.0).collect::<Vec<_>>()}))
}

fn caption_json(s: &Session, c: &cfa4_project::Caption) -> Value {
    let q = s.active_sequence();
    let (rate, df) = q.map(|q| (q.settings.frame_rate, q.settings.drop_frame)).unwrap_or_default();
    let tc = |t: Tick| format_time(t, rate, df, TimeDisplay::Timecode, 48_000);
    json!({
        "id": c.id.0,
        "start": c.start.0,
        "end": c.end().0,
        "in": tc(c.start),
        "out": tc(c.end()),
        "duration": tc(c.duration),
        "text": c.text,
        "speaker": c.speaker,
        "settings": c.settings,
    })
}

fn track_json(s: &Session, t: &CaptionTrack, index: usize) -> Value {
    json!({
        "id": t.id.0,
        "label": format!("C{}", index + 1),
        "name": t.name,
        "format": t.format.label(),
        "language": t.language,
        "enabled": t.enabled,
        "locked": t.locked,
        "syncLock": t.sync_lock,
        "style": {
            "font": t.style.font, "size": t.style.size, "color": hex(t.style.color),
            "background": t.style.background, "backgroundColor": hex(t.style.background_color),
            "align": format!("{:?}", t.style.align).to_lowercase(), "anchor": format!("{:?}", t.style.anchor).to_lowercase(),
            "margin": t.style.margin, "lineSpacing": t.style.line_spacing, "outline": t.style.outline, "outlineColor": hex(t.style.outline_color),
        },
        "captions": t.captions.iter().map(|c| caption_json(s, c)).collect::<Vec<_>>(),
    })
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_p(p, "path").ok_or_else(|| bad("captions.export", "need `path`"))?.to_string();
    let ext = std::path::Path::new(&path).extension().and_then(|e| e.to_str()).unwrap_or("");
    let format = str_p(p, "format").and_then(Format::from_name).or_else(|| Format::from_name(ext)).unwrap_or(Format::Srt);
    let tid = track_param(s, p).ok_or_else(|| bad("captions.export", "no caption track"))?;
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let track = seq.caption_track(tid).ok_or_else(|| bad("captions.export", "no such caption track"))?;
    let doc: Document = cfa4_captions::document_from_track(track, Tick::ZERO);
    let df = bool_p(p, "dropFrame").unwrap_or(true);
    let bytes = cfa4_captions::write(&doc, format, WriteOptions { drop_frame: df, rate: Some(seq.settings.frame_rate) });
    let n = doc.cues.len();
    s.services.write_file(&path, &bytes).map_err(|e| EngineError::Other(e.to_string()))?;
    Ok(json!({"path": path, "format": format.name(), "captions": n, "bytes": bytes.len()}))
}

fn delta_p(s: &Session, p: &Value) -> Option<Tick> {
    if let Some(d) = p.get("delta").and_then(Value::as_i64) {
        return Some(Tick(d));
    }
    p.get("deltaFrames").and_then(Value::as_i64).map(|f| s.sequence_rate().tick_of(f))
}

pub fn commands() -> Vec<CommandSpec> {
    vec![
        spec(
            "captions.newTrack",
            "Add New Caption Track…",
            &["Sequence", "Captions"],
            Some("Cmd+Alt+A"),
            r#"{"format":"Subtitle|CEA-608|CEA-708|Teletext","name":str?,"language":str?}"#,
            has_seq,
            |s, p| {
                let format = str_p(p, "format").and_then(CaptionFormat::from_name).unwrap_or_default();
                let name = str_p(p, "name").map(str::to_string).unwrap_or_else(|| format.label().to_string());
                let lang = str_p(p, "language").unwrap_or("en").to_string();
                let seq = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
                let id = s.edit("Add Caption Track", |pr, _| {
                    let id = TrackId(pr.alloc_id());
                    let mut t = CaptionTrack::new(id, name, format);
                    t.language = lang;
                    if format == CaptionFormat::Cea608 {
                        t.style.background_color = [0, 0, 0, 255];
                    }
                    pr.sequence_mut(seq).ok_or(EngineError::NoSequence)?.caption_tracks.insert(0, t);
                    Ok(id)
                })?;
                Ok(json!({"track": id.0}))
            },
        ),
        spec("captions.deleteTrack", "Delete Caption Track", &[], None, r#"{"track":id|"C1"}"#, has_caption_track, |s, p| {
            let tid = track_param(s, p).ok_or_else(|| bad("captions.deleteTrack", "no such caption track"))?;
            s.edit_sequence("Delete Caption Track", |q, _, st| {
                q.caption_tracks.retain(|t| t.id != tid);
                st.caption_selection.retain(|c| q.find_caption(*c).is_some());
                Ok(())
            })?;
            Ok(Value::Null)
        }),
        spec(
            "captions.setTrack",
            "Caption Track Settings",
            &[],
            None,
            r#"{"track":id|"C1","name":str?,"format":str?,"language":str?,"enabled":bool?,"locked":bool?,"syncLock":bool?}"#,
            has_caption_track,
            |s, p| {
                let tid = track_param(s, p).ok_or_else(|| bad("captions.setTrack", "no such caption track"))?;
                let p = p.clone();
                s.edit("Caption Track Settings", |pr, st| {
                    let seq = st.active_sequence.ok_or(EngineError::NoSequence)?;
                    let t = pr.sequence_mut(seq).and_then(|q| q.caption_track_mut(tid)).ok_or(EngineError::NoSequence)?;
                    if let Some(n) = str_p(&p, "name") {
                        t.name = n.to_string();
                    }
                    if let Some(f) = str_p(&p, "format") {
                        t.format = CaptionFormat::from_name(f).ok_or_else(|| bad("captions.setTrack", "unknown format"))?;
                    }
                    if let Some(l) = str_p(&p, "language") {
                        t.language = l.to_string();
                    }
                    if let Some(b) = bool_p(&p, "enabled") {
                        t.enabled = b;
                    }
                    if let Some(b) = bool_p(&p, "locked") {
                        t.locked = b;
                    }
                    if let Some(b) = bool_p(&p, "syncLock") {
                        t.sync_lock = b;
                    }
                    Ok(())
                })?;
                Ok(Value::Null)
            },
        ),
        spec(
            "captions.setStyle",
            "Caption Track Style",
            &[],
            None,
            r##"{"track":id|"C1","font":str?,"size":f32?,"color":"#rrggbb[aa]"?,"background":bool?,"backgroundColor":"#rrggbbaa"?,"align":"left|center|right"?,"anchor":"top|middle|bottom"?,"margin":0..0.45 (fraction of frame height)?,"lineSpacing":f32?,"outline":f32?,"outlineColor":str?,"reset":bool?}"##,
            has_caption_track,
            |s, p| {
                let tid = track_param(s, p).ok_or_else(|| {
                    bad(
                        "captions.setStyle",
                        format!(
                            "no such caption track ({}) — pass the track id from captions.list, or \"C1\"…",
                            p.get("track").map(|v| v.to_string()).unwrap_or_else(|| "no `track` given".into())
                        ),
                    )
                })?;
                let p = p.clone();
                s.edit("Caption Style", |pr, st| {
                    let seq = st.active_sequence.ok_or(EngineError::NoSequence)?;
                    let t = pr.sequence_mut(seq).and_then(|q| q.caption_track_mut(tid)).ok_or(EngineError::NoSequence)?;
                    let y = &mut t.style;
                    if bool_p(&p, "reset").unwrap_or(false) {
                        *y = CaptionStyle::default();
                    }
                    if let Some(v) = str_p(&p, "font") {
                        y.font = v.to_string();
                    }
                    if let Some(v) = f64_p(&p, "size") {
                        y.size = (v as f32).clamp(4.0, 400.0);
                    }
                    if let Some(c) = p.get("color").and_then(parse_color) {
                        y.color = c;
                    }
                    if let Some(b) = bool_p(&p, "background") {
                        y.background = b;
                    }
                    if let Some(c) = p.get("backgroundColor").and_then(parse_color) {
                        y.background_color = c;
                    }
                    if let Some(c) = p.get("outlineColor").and_then(parse_color) {
                        y.outline_color = c;
                    }
                    if let Some(v) = f64_p(&p, "outline") {
                        y.outline = (v as f32).clamp(0.0, 40.0);
                    }
                    if let Some(v) = f64_p(&p, "margin") {
                        y.margin = (v as f32).clamp(0.0, 0.45);
                    }
                    if let Some(v) = f64_p(&p, "lineSpacing") {
                        y.line_spacing = (v as f32).clamp(0.8, 4.0);
                    }
                    match str_p(&p, "align") {
                        Some("left") => y.align = CaptionAlign::Left,
                        Some("right") => y.align = CaptionAlign::Right,
                        Some("center") => y.align = CaptionAlign::Center,
                        Some(o) => return Err(bad("captions.setStyle", format!("unknown align `{o}`"))),
                        None => {}
                    }
                    match str_p(&p, "anchor") {
                        Some("top") => y.anchor = CaptionAnchor::Top,
                        Some("middle") => y.anchor = CaptionAnchor::Middle,
                        Some("bottom") => y.anchor = CaptionAnchor::Bottom,
                        Some(o) => return Err(bad("captions.setStyle", format!("unknown anchor `{o}`"))),
                        None => {}
                    }
                    Ok(())
                })?;
                Ok(Value::Null)
            },
        ),
        spec(
            "captions.add",
            "Add Caption at Playhead",
            &["Sequence", "Captions"],
            Some("Cmd+Alt+C"),
            r#"{"track":id|"C1"?,"text":str?,"time":ticks?,"seconds":f64?,"durationSeconds":f64=3}"#,
            has_seq,
            |s, p| {
                // a caption track is created when the sequence has none
                if s.active_sequence().is_some_and(|q| q.caption_tracks.is_empty()) {
                    s.execute("captions.newTrack", json!({}))?;
                }
                let tid = track_param(s, p).ok_or_else(|| bad("captions.add", "no such caption track"))?;
                let t = time_p(s, p, "").unwrap_or(s.playhead());
                let rate = s.sequence_rate();
                let dur = Tick::from_seconds_f64(f64_p(p, "durationSeconds").unwrap_or(edit::captions::DEFAULT_SECONDS as f64));
                let dur = rate.snap_nearest(dur).max(rate.frame_duration());
                let text = str_p(p, "text").unwrap_or("New caption").to_string();
                let t = rate.snap(t);
                let id = s.edit_sequence("Add Caption", |q, ctx, st| {
                    let id = edit::captions::add_caption(q, tid, t, dur, &text, ctx)?;
                    st.caption_selection = vec![id];
                    st.selection.clear();
                    Ok(id)
                })?;
                Ok(json!({"caption": id.0}))
            },
        ),
        spec("captions.split", "Split Caption", &[], None, r#"{"caption":id?,"time":ticks?}"#, has_captions, |s, p| {
            let t = time_p(s, p, "").unwrap_or(s.playhead());
            let explicit = u64_p(p, "caption").map(ClipId);
            let ids = s.edit_sequence("Split Caption", |q, ctx, st| {
                let ids = match explicit {
                    Some(c) => vec![edit::captions::split_caption(q, c, t, ctx)?],
                    None => edit::captions::split_captions_at(q, &[], t, ctx),
                };
                if ids.is_empty() {
                    return Err(edit::EditError::Nothing.into());
                }
                st.caption_selection = ids.clone();
                Ok(ids)
            })?;
            Ok(json!({"captions": ids.iter().map(|c| c.0).collect::<Vec<_>>()}))
        }),
        spec("captions.merge", "Merge Captions", &[], None, r#"{"captions":[id]?}"#, has_captions, |s, p| {
            let ids = caption_ids(s, p);
            let id = s.edit_sequence("Merge Captions", |q, _, st| {
                let id = edit::captions::merge_captions(q, &ids)?;
                st.caption_selection = vec![id];
                Ok(id)
            })?;
            Ok(json!({"caption": id.0}))
        }),
        spec("captions.setText", "Edit Caption Text", &[], None, r#"{"caption":id?,"text":str?,"speaker":str|null?}"#, has_captions, |s, p| {
            let id = one_caption(s, p, "captions.setText")?;
            let text = str_p(p, "text").map(str::to_string);
            let speaker: Option<Option<String>> = match p.get("speaker") {
                None => None,
                Some(Value::Null) => Some(None),
                Some(v) => Some(v.as_str().map(str::to_string)),
            };
            if text.is_none() && speaker.is_none() {
                return Err(bad("captions.setText", "need `text` or `speaker`"));
            }
            s.edit_sequence("Edit Caption", |q, _, _| Ok(edit::captions::set_caption(q, id, text.as_deref(), speaker.as_ref().map(|o| o.as_deref()))?))?;
            Ok(Value::Null)
        }),
        spec(
            "captions.setTimes",
            "Set Caption In/Out",
            &[],
            None,
            r#"{"caption":id?,"startTime|startFrame|startSeconds|startTimecode":…,"endTime|endFrame|endSeconds|endTimecode":…}"#,
            has_captions,
            |s, p| {
                let id = one_caption(s, p, "captions.setTimes")?;
                let cur = s
                    .active_sequence()
                    .and_then(|q| q.find_caption(id))
                    .map(|(_, c)| (c.start, c.end()))
                    .ok_or(EngineError::Edit(edit::EditError::NoItem(id)))?;
                let rate = s.sequence_rate();
                let start = time_p(s, p, "start").map(|t| rate.snap_nearest(t)).unwrap_or(cur.0);
                let end = time_p(s, p, "end").map(|t| rate.snap_nearest(t)).unwrap_or(cur.1);
                s.edit_sequence("Caption Timing", |q, ctx, _| Ok(edit::captions::set_caption_times(q, id, start, end, ctx)?))?;
                Ok(json!({"start": start.0, "end": end.0}))
            },
        ),
        spec("captions.trim", "Trim Caption", &[], None, r#"{"caption":id,"edge":"in|out","delta":ticks|"deltaFrames":i64}"#, has_captions, |s, p| {
            let id = one_caption(s, p, "captions.trim")?;
            let edge = if str_p(p, "edge") == Some("in") { Edge::In } else { Edge::Out };
            let d = delta_p(s, p).ok_or_else(|| bad("captions.trim", "need `delta` or `deltaFrames`"))?;
            let applied = s.edit_sequence("Trim Caption", |q, ctx, _| Ok(edit::captions::trim_caption(q, id, edge, d, ctx)?))?;
            Ok(json!({"applied": applied.0}))
        }),
        spec("captions.move", "Move Captions", &[], None, r#"{"captions":[id]?,"delta":ticks|"deltaFrames":i64}"#, has_captions, |s, p| {
            let ids = caption_ids(s, p);
            let d = delta_p(s, p).ok_or_else(|| bad("captions.move", "need `delta` or `deltaFrames`"))?;
            let applied = s.edit_sequence("Move Captions", |q, _, _| Ok(edit::captions::move_captions(q, &ids, d)?))?;
            Ok(json!({"applied": applied.0}))
        }),
        spec("captions.duplicate", "Duplicate Captions", &[], None, r#"{"captions":[id]?,"delta":ticks|"deltaFrames":i64}"#, has_captions, |s, p| {
            // Option-drag in the Timeline: copies `delta` later (or earlier) on their own
            // tracks, overwriting what is there; the originals stay
            let mut ids = caption_ids(s, p);
            // each caption is copied once, however often it is named
            let mut seen = std::collections::HashSet::new();
            ids.retain(|id| seen.insert(*id));
            if ids.is_empty() {
                return Err(EngineError::Edit(edit::EditError::Nothing));
            }
            let d = delta_p(s, p).ok_or_else(|| bad("captions.duplicate", "need `delta` or `deltaFrames`"))?;
            let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
            let mut by_track: Vec<(TrackId, Vec<cfa4_project::Caption>)> = Vec::new();
            for id in &ids {
                let (tid, c) = q.find_caption(*id).ok_or(EngineError::Edit(edit::EditError::NoItem(*id)))?;
                match by_track.iter_mut().find(|(t, _)| *t == tid) {
                    Some((_, v)) => v.push(c.clone()),
                    None => by_track.push((tid, vec![c.clone()])),
                }
            }
            // never before the start of the sequence
            let earliest = by_track.iter().flat_map(|(_, v)| v.iter().map(|c| c.start)).min().unwrap_or_default();
            let d = d.max(-earliest);
            if d == Tick::ZERO {
                return Err(EngineError::Edit(edit::EditError::Nothing));
            }
            let new = s.edit_sequence("Duplicate Captions", |q, ctx, st| {
                let mut new = Vec::new();
                for (tid, caps) in &by_track {
                    let moved: Vec<_> = caps.iter().map(|c| cfa4_project::Caption { start: c.start + d, ..c.clone() }).collect();
                    new.extend(edit::captions::overwrite_captions(q, *tid, &moved, ctx)?);
                }
                st.caption_selection = new.clone();
                st.selection.clear();
                Ok(new)
            })?;
            Ok(json!({"captions": new.iter().map(|c| c.0).collect::<Vec<_>>(), "delta": d.0}))
        }),
        spec("captions.delete", "Delete Captions", &[], None, r#"{"captions":[id]?,"ripple":bool=false}"#, has_captions, |s, p| {
            let ids = caption_ids(s, p);
            delete(s, &ids, bool_p(p, "ripple").unwrap_or(false))
        }),
        spec("captions.select", "Select Captions", &[], None, r#"{"captions":[id],"add":bool?}"#, has_seq, |s, p| {
            let ids: Vec<ClipId> =
                p.get("captions").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).map(ClipId).collect()).unwrap_or_default();
            let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
            let ids: Vec<ClipId> = ids.into_iter().filter(|c| seq.find_caption(*c).is_some()).collect();
            if bool_p(p, "add").unwrap_or(false) {
                for c in ids {
                    if !s.state.caption_selection.contains(&c) {
                        s.state.caption_selection.push(c);
                    }
                }
            } else {
                s.state.caption_selection = ids;
                s.state.selection.clear();
            }
            Ok(json!({"captions": s.state.caption_selection.iter().map(|c| c.0).collect::<Vec<_>>()}))
        }),
        spec("captions.goTo", "Go to Caption", &[], None, r#"{"caption":id}"#, has_captions, |s, p| {
            let id = one_caption(s, p, "captions.goTo")?;
            let start = s.active_sequence().and_then(|q| q.find_caption(id)).map(|(_, c)| c.start).ok_or(EngineError::Edit(edit::EditError::NoItem(id)))?;
            s.set_playhead(start);
            s.state.caption_selection = vec![id];
            s.state.selection.clear();
            Ok(json!({"time": start.0}))
        }),
        spec("captions.next", "Go to Next Caption Segment", &["Sequence", "Captions"], Some("Cmd+Alt+Down"), "{}", has_captions, |s, _| step(s, true)),
        spec("captions.previous", "Go to Previous Caption Segment", &["Sequence", "Captions"], Some("Cmd+Alt+Up"), "{}", has_captions, |s, _| step(s, false)),
        spec("captions.showAll", "Show All Caption Tracks", &["Sequence", "Captions"], None, "{}", has_caption_track, |s, _| show_all(s, true)),
        spec("captions.hideAll", "Hide All Caption Tracks", &["Sequence", "Captions"], None, "{}", has_caption_track, |s, _| show_all(s, false)),
        spec("captions.import", "Import Captions…", &[], None, r#"{"path":str,"name":str?}"#, always, |s, p| {
            let path = str_p(p, "path").ok_or_else(|| bad("captions.import", "need `path`"))?.to_string();
            let bytes = s.services.read_file(&path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
            let ext = std::path::Path::new(&path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
            let format = cfa4_captions::detect(&bytes, ext.as_deref()).ok_or_else(|| EngineError::Other(format!("{path}: not a caption file")))?;
            import(s, &path, &bytes, format, str_p(p, "name"))
        }),
        spec(
            "captions.export",
            "Captions…",
            &["File", "Export"],
            None,
            r#"{"path":str,"format":"srt|vtt|scc|mcc|stl|ttml|dfxp"? (default: from the extension),"track":id|"C1"?,"dropFrame":bool=true}"#,
            has_caption_track,
            export,
        ),
        CommandSpec {
            id: "captions.list",
            label: "List Captions",
            menu: &[],
            shortcut: None,
            params: r#"{"track":id|"C1"?}"#,
            enabled: always,
            run: |s, p| {
                let Some(seq) = s.active_sequence() else { return Ok(json!({"tracks": []})) };
                // a `track` that names no caption track is an error, never every track
                let only = match p.get("track") {
                    Some(_) => Some(track_param(s, p).ok_or_else(|| bad("captions.list", "no such caption track"))?),
                    None => None,
                };
                let tracks: Vec<Value> =
                    seq.caption_tracks.iter().enumerate().filter(|(_, t)| only.is_none_or(|o| o == t.id)).map(|(i, t)| track_json(s, t, i)).collect();
                Ok(json!({"tracks": tracks, "selection": s.state.caption_selection.iter().map(|c| c.0).collect::<Vec<_>>()}))
            },
            journal: false,
        },
    ]
}

fn step(s: &mut Session, forward: bool) -> Result<Value> {
    let t = s.playhead();
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let mut starts: Vec<(Tick, ClipId)> = seq.caption_tracks.iter().filter(|t| t.enabled).flat_map(|t| t.captions.iter().map(|c| (c.start, c.id))).collect();
    starts.sort();
    let hit = if forward { starts.iter().find(|(st, _)| *st > t).copied() } else { starts.iter().rev().find(|(st, _)| *st < t).copied() };
    let (time, id) = hit.ok_or_else(|| EngineError::Other("no more captions".into()))?;
    s.set_playhead(time);
    s.state.caption_selection = vec![id];
    Ok(json!({"time": time.0, "caption": id.0}))
}

fn show_all(s: &mut Session, on: bool) -> Result<Value> {
    s.edit(if on { "Show All Caption Tracks" } else { "Hide All Caption Tracks" }, |p, st| {
        let seq = st.active_sequence.ok_or(EngineError::NoSequence)?;
        for t in &mut p.sequence_mut(seq).ok_or(EngineError::NoSequence)?.caption_tracks {
            t.enabled = on;
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

#[cfg(test)]
#[path = "captions_tests.rs"]
mod tests;
