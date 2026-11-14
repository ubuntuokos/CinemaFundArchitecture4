//! Markers from a CSV file or an EDL (CFA4-LOSSLESS-MEDIA-TOOLS-001 1.1, K6), the counterpart of
//! `markers.exportCsv`.
//!
//! - `markers.importCsv {"path", "target"?: "program"|"source", "item"?, "allowRateChange"?}` reads
//!   the columns `markers.exportCsv` writes (by header name, any order): `Name`, `Comment`, `Kind`,
//!   `Color`, `Start Ticks` / `Start Timecode`, `Duration Ticks` / `End Timecode` /
//!   `Duration Frames`, `Frame Rate`. Ticks win where both are present (they are exact at any
//!   rate). The markers go to the active sequence, or with `"target": "source"` / `"item"` to a
//!   media item (its segments).
//! - `markers.importEdl {"path", "item"?}` reads each event of a CMX 3600 EDL as a range marker of
//!   the media item (the Source clip by default): its source in and out, minus the clip's start
//!   timecode, named by `* SEGMENT NAME:`, `* TO CLIP NAME:`, `* FROM CLIP NAME:` or the event
//!   number. `markers.exportEdl {"path", "item"?}` writes a media item's range markers that way, so
//!   segments go out and come back without loss.
//!
//! The time base is checked, never converted silently: a CSV whose `Frame Rate` differs from the
//! target's is refused unless `"allowRateChange": true` (then its ticks are kept, or its timecodes
//! read at its own rate); an EDL whose timecodes cannot belong to the clip's rate is refused. A
//! marker outside the target (before its start, past a source's end) refuses the whole file.
//! Nothing is written unless every row can be; the import is one undo step, and a marker already
//! there (same start, length and name) is not added twice.

use cfa4_project::{ItemId, Label, Marker, MarkerId, MarkerKind};
use cfa4_time::{FrameRate, Tick, format_timecode_frames, parse_timecode};
use serde_json::{Value, json};

use crate::clip_ops::source_view;
use crate::commands::{bad, bool_p, str_p};
use crate::{EngineError, Result, Session};

/// Larger files are refused (an input-derived allocation).
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_MARKERS: usize = 100_000;

/// Where markers are read from and written to.
pub(crate) struct Target {
    /// The sequence, or the media item whose markers are its segments.
    pub(crate) holder: Holder,
    pub(crate) name: String,
    pub(crate) rate: FrameRate,
    pub(crate) drop_frame: bool,
    /// Timecode of time zero, in frames at `rate`.
    pub(crate) start_tc: i64,
    /// The times a marker may take.
    pub(crate) span: (Tick, Tick),
    pub(crate) markers: Vec<Marker>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Holder {
    Sequence(ItemId),
    Media(ItemId),
}

impl Holder {
    fn json(self) -> Value {
        match self {
            Holder::Sequence(i) => json!({"sequence": i.0}),
            Holder::Media(i) => json!({"item": i.0}),
        }
    }
}

/// The target of a marker command: `item` (a media item), `"target": "source"` (the Source clip),
/// or the active sequence.
pub(crate) fn target(s: &Session, p: &Value, cmd: &str) -> Result<Target> {
    let item = match (p.get("item"), str_p(p, "target")) {
        (Some(v), _) if !v.is_null() => Some(crate::commands::item_p(p, "item").ok_or_else(|| bad(cmd, "`item` must be a project item id"))?),
        (_, Some("source")) => Some(s.state.source_item.ok_or_else(|| bad(cmd, "no Source clip is open"))?),
        (_, None | Some("program")) => None,
        (_, Some(_)) => return Err(bad(cmd, "target must be program or source")),
    };
    if let Some(item) = item {
        let view = source_view(s, item).ok_or_else(|| bad(cmd, format!("no media item {}", item.0)))?;
        let media = s.project.item(view.media).and_then(|i| i.as_media()).ok_or_else(|| bad(cmd, format!("item {} is not a media clip", item.0)))?;
        let name = s.project.item(view.media).map(|i| i.name.clone()).unwrap_or_default();
        return Ok(Target {
            holder: Holder::Media(view.media),
            name,
            rate: view.rate,
            drop_frame: false,
            start_tc: media.info.start_timecode.unwrap_or(0),
            span: (Tick::ZERO, view.end.max(Tick::ZERO)),
            markers: media.markers.clone(),
        });
    }
    let id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    seq.settings.validate().map_err(|e| bad(cmd, e))?;
    Ok(Target {
        holder: Holder::Sequence(id),
        name: s.project.item(id).map(|i| i.name.clone()).unwrap_or_else(|| "Sequence".into()),
        rate: seq.settings.frame_rate,
        drop_frame: seq.settings.drop_frame,
        start_tc: seq.start_timecode,
        span: (Tick::ZERO, Tick::MAX),
        markers: seq.markers.clone(),
    })
}

/// Enabled when there is a sequence or a Source clip to take the markers from.
pub(crate) fn has_target(s: &Session) -> std::result::Result<(), String> {
    // an explicit `item` names any media item of the project, so one is enough
    let media = || s.project.items.values().any(|i| matches!(i.kind, cfa4_project::ItemKind::Media(_)));
    if s.state.active_sequence.is_some() || s.state.source_item.is_some() || media() { Ok(()) } else { Err("open a sequence or a Source clip first".into()) }
}

/// A marker to add, before it has an id.
struct New {
    start: Tick,
    duration: Tick,
    name: String,
    comment: String,
    kind: MarkerKind,
    color: Label,
}

fn read(s: &Session, path: &str, cmd: &str) -> Result<String> {
    if path.trim().is_empty() || path.contains('\0') {
        return Err(bad(cmd, "need a non-empty `path` without NUL characters"));
    }
    let bytes = s.services.read_file(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(bad(cmd, format!("{path} is larger than 16 MiB")));
    }
    let text = String::from_utf8(bytes).map_err(|_| bad(cmd, format!("{path} is not UTF-8 text")))?;
    Ok(text.strip_prefix('\u{feff}').map(str::to_string).unwrap_or(text))
}

/// Most fields in one CSV record (`markers.exportCsv` writes 12).
const MAX_FIELDS: usize = 256;
/// Most characters in one CSV field.
const MAX_FIELD_CHARS: usize = 64 * 1024;

/// RFC 4180 records: quoted fields may hold commas, quotes (`""`) and line breaks. The limits
/// (records, fields per record, characters per field) apply while parsing, so a hostile file is
/// refused before it is held in memory. An error when a quoted field is never closed (a cut-off
/// file) or a limit is passed.
fn csv_records(text: &str) -> std::result::Result<Vec<Vec<String>>, String> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut chars_in_field = 0usize;
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let end_field = |row: &mut Vec<String>, field: &mut String, rows: &Vec<Vec<String>>| -> std::result::Result<(), String> {
        if row.len() >= MAX_FIELDS {
            return Err(format!("row {} has more than {MAX_FIELDS} fields", rows.len() + 1));
        }
        row.push(std::mem::take(field));
        Ok(())
    };
    while let Some(c) = chars.next() {
        let mut push = |field: &mut String, c: char| {
            chars_in_field += 1;
            if chars_in_field > MAX_FIELD_CHARS {
                return Err(format!("row {} has a field longer than {MAX_FIELD_CHARS} characters", rows.len() + 1));
            }
            field.push(c);
            Ok(())
        };
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    push(&mut field, '"')?;
                }
                '"' => quoted = false,
                _ => push(&mut field, c)?,
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => {
                end_field(&mut row, &mut field, &rows)?;
                chars_in_field = 0;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' | '\r' => {
                end_field(&mut row, &mut field, &rows)?;
                chars_in_field = 0;
                if row.iter().any(|f| !f.is_empty()) {
                    // the header and at most MAX_MARKERS markers
                    if rows.len() > MAX_MARKERS {
                        return Err(format!("the CSV has more than {MAX_MARKERS} rows"));
                    }
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            _ => push(&mut field, c)?,
        }
    }
    if quoted {
        return Err("a quoted CSV field is never closed (is the file cut off?)".into());
    }
    end_field(&mut row, &mut field, &rows)?;
    if row.iter().any(|f| !f.is_empty()) {
        if rows.len() > MAX_MARKERS {
            return Err(format!("the CSV has more than {MAX_MARKERS} rows"));
        }
        rows.push(row);
    }
    Ok(rows)
}

/// A text field as `markers.exportCsv` wrote it, without its spreadsheet-formula guard (the
/// exact inverse of [`crate::marker_export::needs_guard`]: a `'` the exporter added is always
/// followed by text that needs the guard, and any other `'` is kept).
fn unguard(v: &str) -> String {
    match v.strip_prefix('\'') {
        Some(rest) if crate::marker_export::needs_guard(rest) => rest.to_string(),
        _ => v.to_string(),
    }
}

fn kind_of(v: &str) -> Option<MarkerKind> {
    let v = v.trim();
    [MarkerKind::Comment, MarkerKind::Chapter, MarkerKind::Segmentation, MarkerKind::WebLink, MarkerKind::FlashCue]
        .into_iter()
        .find(|k| format!("{k:?}").eq_ignore_ascii_case(v) || (v.eq_ignore_ascii_case("Web Link") && *k == MarkerKind::WebLink))
}

/// A label by its name or by the user's own name for it (what `markers.exportCsv` writes).
fn color_of(s: &Session, v: &str) -> Option<Label> {
    let v = v.trim();
    if v.is_empty() {
        return Some(Label::Green);
    }
    Label::from_name(v).or_else(|| {
        Label::ALL.iter().copied().find(|l| s.prefs.labels.colors.get(&crate::settings::label_id(*l)).is_some_and(|c| c.name.trim().eq_ignore_ascii_case(v)))
    })
}

fn rate_of(v: &str) -> Option<FrameRate> {
    let v = v.trim();
    let r = match v.split_once('/') {
        Some((n, d)) => FrameRate::new(n.trim().parse().ok()?, d.trim().parse().ok()?),
        None => FrameRate::from_f64(v.parse().ok().filter(|f: &f64| f.is_finite())?),
    };
    (r.num > 0 && r.den > 0 && r.frame_duration() > Tick::ZERO).then_some(r)
}

fn same_rate(a: FrameRate, b: FrameRate) -> bool {
    i128::from(a.num) * i128::from(b.den) == i128::from(b.num) * i128::from(a.den)
}

fn csv_markers(s: &Session, t: &Target, text: &str, allow_rate_change: bool, cmd: &str) -> Result<Vec<New>> {
    let rows = csv_records(text).map_err(|e| bad(cmd, format!("{e}; nothing was imported")))?;
    let Some((header, rows)) = rows.split_first() else {
        return Err(bad(cmd, "the CSV is empty"));
    };
    if rows.len() > MAX_MARKERS {
        return Err(bad(cmd, format!("the CSV has {} rows; at most {MAX_MARKERS}", rows.len())));
    }
    let col = |name: &str| header.iter().position(|h| h.trim().eq_ignore_ascii_case(name));
    let (c_start_ticks, c_start_tc) = (col("Start Ticks"), col("Start Timecode"));
    if c_start_ticks.is_none() && c_start_tc.is_none() {
        return Err(bad(cmd, "the CSV has neither a `Start Ticks` nor a `Start Timecode` column"));
    }
    let (c_dur_ticks, c_end_tc, c_dur_frames) = (col("Duration Ticks"), col("End Timecode"), col("Duration Frames"));
    let (c_name, c_comment, c_kind, c_color, c_rate) = (col("Name"), col("Comment"), col("Kind"), col("Color"), col("Frame Rate"));
    let mut out = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let n = i + 2; // the line number a spreadsheet shows (the header is line 1)
        let get = |c: Option<usize>| c.and_then(|c| row.get(c)).map(|v| v.trim()).filter(|v| !v.is_empty());
        let row_bad = |msg: String| bad(cmd, format!("row {n}: {msg}; nothing was imported"));
        // the time base of this row
        let rate = match get(c_rate) {
            None => t.rate,
            Some(v) => {
                let r = rate_of(v).ok_or_else(|| row_bad(format!("`{v}` is not a frame rate")))?;
                if !same_rate(r, t.rate) && !allow_rate_change {
                    return Err(row_bad(format!(
                        "the CSV was written at {} fps, the target runs at {} fps; run with \"allowRateChange\": true to keep the marker times",
                        r.label(),
                        t.rate.label()
                    )));
                }
                r
            }
        };
        let tc_ticks = |v: &str| -> Result<Tick> {
            let df = (t.drop_frame || v.contains([';', ','])) && rate.supports_drop_frame();
            let f = parse_timecode(v, rate, df, 0).map_err(|_| row_bad(format!("`{v}` is not a timecode at {} fps", rate.label())))?;
            // the target's start timecode is at the target's rate
            let origin = t.rate.tick_of(t.start_tc);
            Ok(Tick(rate.tick_of(f).0.saturating_sub(origin.0)))
        };
        let ticks = |v: &str| v.parse::<i64>().map(Tick).map_err(|_| row_bad(format!("`{v}` is not a whole number of ticks")));
        let start = match (get(c_start_ticks), get(c_start_tc)) {
            (Some(v), _) => ticks(v)?,
            (None, Some(v)) => tc_ticks(v)?,
            (None, None) => return Err(row_bad("no start time".into())),
        };
        let duration = match (get(c_dur_ticks), get(c_end_tc), get(c_dur_frames)) {
            (Some(v), _, _) => ticks(v)?,
            (None, Some(v), _) => Tick(tc_ticks(v)?.0.saturating_sub(start.0)),
            (None, None, Some(v)) => rate.tick_of(v.parse::<i64>().map_err(|_| row_bad(format!("`{v}` is not a whole number of frames")))?),
            (None, None, None) => Tick::ZERO,
        };
        if start < t.span.0 || duration < Tick::ZERO || start.0.checked_add(duration.0).is_none_or(|e| e > t.span.1.0) {
            return Err(row_bad(format!("the marker ({} for {}) lies outside {}", start.0, duration.0, t.name)));
        }
        let kind = match get(c_kind) {
            None => MarkerKind::Comment,
            Some(v) => kind_of(v).ok_or_else(|| row_bad(format!("unknown marker kind `{v}`")))?,
        };
        let color = color_of(s, get(c_color).unwrap_or("")).ok_or_else(|| row_bad(format!("unknown marker colour `{}`", get(c_color).unwrap_or(""))))?;
        let text = |c: Option<usize>| c.and_then(|c| row.get(c)).map(|v| unguard(v)).unwrap_or_default();
        out.push(New { start, duration, name: text(c_name), comment: text(c_comment), kind, color });
    }
    Ok(out)
}

fn edl_markers(t: &Target, text: &str, cmd: &str) -> Result<Vec<New>> {
    let ranges = cfa4_interchange::edl_source_ranges(text, t.rate).map_err(|e| bad(cmd, format!("{e}; nothing was imported")))?;
    if ranges.is_empty() {
        return Err(bad(cmd, "the EDL has no events"));
    }
    if ranges.len() > MAX_MARKERS {
        return Err(bad(cmd, format!("the EDL has {} events; at most {MAX_MARKERS}", ranges.len())));
    }
    let mut out = Vec::with_capacity(ranges.len());
    for r in &ranges {
        let a = t.rate.tick_of(r.src_in.saturating_sub(t.start_tc));
        let b = t.rate.tick_of(r.src_out.saturating_sub(t.start_tc));
        let tc = |f: i64| format_timecode_frames(f, t.rate, false);
        if b <= a {
            return Err(bad(
                cmd,
                format!("event {}: the source out {} is not after the source in {}; nothing was imported", r.event, tc(r.src_out), tc(r.src_in)),
            ));
        }
        if a < t.span.0 || b > t.span.1 {
            return Err(bad(cmd, format!("event {}: {}–{} lies outside {}; nothing was imported", r.event, tc(r.src_in), tc(r.src_out), t.name)));
        }
        let name = r.segment.clone().or_else(|| r.clip.clone()).unwrap_or_else(|| format!("Event {}", r.event));
        out.push(New { start: a, duration: Tick(b.0 - a.0), name, comment: String::new(), kind: MarkerKind::Comment, color: Label::Green });
    }
    Ok(out)
}

/// Add the markers in one undo step; those already there are skipped.
fn write(s: &mut Session, t: &Target, label: &str, new: Vec<New>, cmd: &str) -> Result<Value> {
    if new.is_empty() {
        return Err(bad(cmd, "the file holds no markers"));
    }
    let holder = t.holder;
    let total = new.len();
    let added = s.edit(label, |pr, _| {
        if pr.next_id.checked_add(new.len() as u64).is_none() {
            return Err(bad(cmd, "project ids are exhausted"));
        }
        let ids: Vec<u64> = new.iter().map(|_| pr.alloc_id()).collect();
        let list: &mut Vec<Marker> = match holder {
            Holder::Sequence(id) => &mut pr.sequence_mut(id).ok_or(EngineError::NoSequence)?.markers,
            Holder::Media(id) => &mut pr.item_mut(id).and_then(|i| i.as_media_mut()).ok_or_else(|| bad(cmd, format!("no media item {}", id.0)))?.markers,
        };
        let mut added = Vec::new();
        // the markers already there (and those added now), looked up in constant time
        let mut there: std::collections::HashSet<(Tick, Tick, String)> = list.iter().map(|x| (x.start, x.duration, x.name.clone())).collect();
        for (m, id) in new.into_iter().zip(ids) {
            if !there.insert((m.start, m.duration, m.name.clone())) {
                continue;
            }
            list.push(Marker { id: MarkerId(id), start: m.start, duration: m.duration, name: m.name, comment: m.comment, kind: m.kind, color: m.color });
            added.push(id);
        }
        list.sort_by_key(|x| (x.start, x.id.0));
        Ok(added)
    })?;
    let mut out = holder.json();
    out["markers"] = json!(added);
    out["skipped"] = json!(total - added.len());
    Ok(out)
}

pub(crate) fn import_csv(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "markers.importCsv";
    let path = str_p(p, "path").ok_or_else(|| bad(cmd, "need `path`"))?.to_string();
    let t = target(s, p, cmd)?;
    let text = read(s, &path, cmd)?;
    let new = csv_markers(s, &t, &text, bool_p(p, "allowRateChange").unwrap_or(false), cmd)?;
    write(s, &t, "Import Markers", new, cmd)
}

pub(crate) fn import_edl(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "markers.importEdl";
    let path = str_p(p, "path").ok_or_else(|| bad(cmd, "need `path`"))?.to_string();
    let mut q = p.clone();
    if q.get("item").is_none_or(Value::is_null) {
        q["target"] = json!("source");
    }
    let t = target(s, &q, cmd)?;
    let text = read(s, &path, cmd)?;
    let new = edl_markers(&t, &text, cmd)?;
    write(s, &t, "Import Segments from EDL", new, cmd)
}

/// One EDL line field: the reel name of an event (letters, digits and `_`, at most 8).
fn reel_of(name: &str) -> String {
    let r: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(8).collect::<String>().to_ascii_uppercase();
    if r.is_empty() { "AX".into() } else { r }
}

/// One line of a comment: line breaks would end it.
fn one_line(v: &str) -> String {
    v.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().trim().to_string()
}

pub(crate) fn export_edl(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "markers.exportEdl";
    let path = str_p(p, "path")
        .filter(|v| !v.trim().is_empty() && !v.contains('\0'))
        .ok_or_else(|| bad(cmd, "need a non-empty `path` without NUL characters"))?
        .to_string();
    let mut q = p.clone();
    if q.get("item").is_none_or(Value::is_null) {
        q["target"] = json!("source");
    }
    let t = target(s, &q, cmd)?;
    let mut segments: Vec<&Marker> = t.markers.iter().filter(|m| m.duration > Tick::ZERO && m.start >= Tick::ZERO).collect();
    if segments.is_empty() {
        return Err(bad(cmd, format!("{} has no segments (range markers)", t.name)));
    }
    if segments.len() > 999 {
        return Err(bad(cmd, "a CMX 3600 EDL holds at most 999 events"));
    }
    segments.sort_by_key(|m| (m.start, m.id.0));
    let rate = t.rate;
    let tc = |f: i64| format_timecode_frames(f, rate, false);
    let reel = reel_of(&t.name);
    let mut out = format!("TITLE: {}\r\nFCM: NON-DROP FRAME\r\n\r\n", one_line(&t.name));
    let mut rec = rate.timecode_base().saturating_mul(3600); // 01:00:00:00
    for (n, m) in segments.iter().enumerate() {
        let a = rate.frame_at(m.start).saturating_add(t.start_tc);
        let end = Tick(m.start.0.saturating_add(m.duration.0));
        let b = rate.frame_at(end).saturating_add(t.start_tc).max(a.saturating_add(1));
        // the EDL holds whole frames: a segment that does not start or end on one is refused
        if rate.tick_of(rate.frame_at(m.start)) != m.start || rate.tick_of(rate.frame_at(end)) != end {
            return Err(bad(cmd, format!("segment `{}` does not start and end on a frame boundary, which an EDL cannot hold; nothing was written", m.name)));
        }
        let len = b - a;
        out.push_str(&format!("{:03}  {reel:<8} V     C        {} {} {} {}\r\n", n + 1, tc(a), tc(b), tc(rec), tc(rec.saturating_add(len))));
        out.push_str(&format!("* FROM CLIP NAME: {}\r\n", one_line(&t.name)));
        if !m.name.trim().is_empty() {
            out.push_str(&format!("* SEGMENT NAME: {}\r\n", one_line(&m.name)));
        }
        out.push_str("\r\n");
        rec = rec.saturating_add(len);
    }
    s.services.write_file(&path, out.as_bytes()).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    let mut r = t.holder.json();
    r["path"] = json!(path);
    r["events"] = json!(segments.len());
    Ok(r)
}

#[cfg(test)]
#[path = "marker_import_tests.rs"]
mod tests;
