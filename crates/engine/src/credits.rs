//! Credits sequences from a spreadsheet (`graphics.credits.*`), on [`cfa4_credits`].
//!
//! - `graphics.credits.import` reads a credits spreadsheet (CSV / TSV / `.xlsx` / `.ods` file, or
//!   inline CSV text),
//!   lays it out with the real text engine and places one graphic clip per page from the
//!   playhead (or `time`), one after another with the page gaps, as one undo step. Scroll pages
//!   roll (Roll, start and end off screen); card pages fade in and out. Every page stays an
//!   ordinary, editable graphic. Pictures (`{{Pic logo.png}}`) are their own clips above the
//!   page that move with its roll or fade with its card; the page keeps a transparent
//!   placeholder layer for each so its size and roll include them.
//! - `graphics.credits.preview` does the same without changing the project and reports pages,
//!   durations and warnings.
//! - `graphics.credits.styles` lists the style names and returns a starter styling (JSON).
//! - **Linked credits.** Every placed page remembers its credits document (`.cfa4credits`:
//!   source, worksheet, styling, the accepted table) and a fingerprint of each layer it made.
//!   `graphics.credits.save` writes the document to a file (portable: the table is embedded);
//!   `graphics.credits.import` also opens such a file. `graphics.credits.diff` compares the
//!   spreadsheet as it is now with the accepted table, row by row, and lists layers edited by
//!   hand; `graphics.credits.refresh` applies the accepted changes, keeps the hand edits whose
//!   block is unchanged, and replaces the pages in place, as one undo step.

use cfa4_credits::doc::{CreditsDoc, SourceRef};
use cfa4_credits::layout::{Frame, Measure};
use cfa4_credits::{Laid, Styling};
use cfa4_geom::Vec2;
use cfa4_project::graphic::eval_layer;
use cfa4_project::{ClipId, CreditsLink, EffectInstance, Interpolation, ItemId, Keyframe, LayerStamp, Param, ParamValue};
use cfa4_render::graphic_clip::layer_local_bounds;
use cfa4_time::Tick;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, has_seq, str_p, u64_p};
use crate::graphics::place_video_clip;
use crate::{EngineError, Result, Session};

/// Largest spreadsheet or styling file read from disk.
const MAX_FILE: u64 = 16 << 20;

/// Measures text layers with the text engine the renderer uses.
struct EngineMeasure {
    canvas: (u32, u32),
    /// Folder that relative picture names are read from.
    base: Option<std::path::PathBuf>,
    /// Picture sizes already looked up.
    sizes: std::cell::RefCell<Vec<(std::path::PathBuf, Option<(f64, f64)>)>>,
}

/// Largest picture file read for credits.
const MAX_PICTURE: u64 = 256 << 20;

/// The file a picture tag names: absolute, or relative to `base`.
fn picture_path(base: Option<&std::path::Path>, file: &str) -> Option<std::path::PathBuf> {
    let f = std::path::Path::new(file.trim());
    let p = if f.is_absolute() { f.to_path_buf() } else { base?.join(f) };
    p.is_file().then_some(p)
}

impl Measure for EngineMeasure {
    fn bounds(&self, layer: &EffectInstance) -> Option<[f64; 4]> {
        let spec = eval_layer(layer, Tick::ZERO, self.canvas)?;
        let b = layer_local_bounds(&spec);
        Some(b.map(f64::from))
    }

    fn picture_size(&self, file: &str) -> Option<(f64, f64)> {
        let path = picture_path(self.base.as_deref(), file)?;
        let mut cache = self.sizes.borrow_mut();
        if let Some((_, sz)) = cache.iter().find(|(p, _)| *p == path) {
            return *sz;
        }
        // decoded with the same still decoder the import uses, so what is measured can be placed
        let sz = std::fs::metadata(&path)
            .ok()
            .filter(|m| m.len() <= MAX_PICTURE)
            .and_then(|_| std::fs::read(&path).ok())
            .and_then(|b| cfa4_media::still::decode_raster(file, &b).ok())
            .filter(|r| r.width > 0 && r.height > 0)
            .map(|r| (f64::from(r.width), f64::from(r.height)));
        cache.push((path, sz));
        sz
    }
}

fn read_file(cmd: &str, path: &str) -> Result<Vec<u8>> {
    let meta = std::fs::metadata(path).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
    if meta.len() > MAX_FILE {
        return Err(bad(cmd, format!("{path} is larger than {} MiB", MAX_FILE >> 20)));
    }
    std::fs::read(path).map_err(|e| bad(cmd, format!("{path}: {e}")))
}

fn styling_p(cmd: &str, p: &Value) -> Result<Styling> {
    match p.get("styling") {
        None | Some(Value::Null) => Ok(Styling::default()),
        Some(Value::Object(_)) => serde_json::from_value(p.get("styling").cloned().unwrap_or(Value::Null)).map_err(|e| bad(cmd, format!("styling: {e}"))),
        Some(Value::String(s)) => {
            let text = if s.trim_start().starts_with('{') { s.clone() } else { String::from_utf8_lossy(&read_file(cmd, s)?).into_owned() };
            Styling::from_json(&text).map_err(|e| bad(cmd, e.to_string()))
        }
        Some(_) => Err(bad(cmd, "`styling` is an object, JSON text or the path of a .json file")),
    }
}

/// Information about the worksheets of the file that was read.
type SheetInfo = cfa4_credits::office::Sheet;

/// The credits document a command names: a `.cfa4credits` file, a spreadsheet (`path`, read
/// now) or inline CSV text (`csv`). `styling` and `pictureDir` override the document's.
/// Returns the document, the worksheet information and the folder pictures are read from.
fn doc_p(cmd: &str, p: &Value) -> Result<(CreditsDoc, SheetInfo, Option<std::path::PathBuf>)> {
    let styling = if p.get("styling").is_some_and(|v| !v.is_null()) { Some(styling_p(cmd, p)?) } else { None };
    let (mut doc, info, file_dir) = match (str_p(p, "csv"), str_p(p, "path")) {
        (Some(text), _) => {
            let mut info = cfa4_credits::read_rows(text.as_bytes(), None).map_err(|e| bad(cmd, e.to_string()))?;
            let table = std::mem::take(&mut info.rows);
            (CreditsDoc { table, ..Default::default() }, info, None)
        }
        (None, Some(path)) => {
            if path.to_lowercase().ends_with(".numbers") {
                return Err(bad(cmd, "Apple Numbers files are not read; export the sheet as .xlsx or CSV"));
            }
            let bytes = read_file(cmd, path)?;
            let dir = std::path::Path::new(path).parent().map(std::path::Path::to_path_buf);
            if cfa4_credits::doc::is_doc(&bytes) {
                let d = CreditsDoc::from_bytes(&bytes).map_err(|e| bad(cmd, e.to_string()))?;
                (d, SheetInfo::default(), dir)
            } else {
                let mut info = cfa4_credits::read_rows(&bytes, str_p(p, "sheet")).map_err(|e| bad(cmd, e.to_string()))?;
                let table = std::mem::take(&mut info.rows);
                let abs = std::fs::canonicalize(path).map(|x| x.to_string_lossy().into_owned()).unwrap_or_else(|_| path.to_string());
                let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let sheet = (!info.chosen.is_empty()).then(|| info.chosen.clone());
                (CreditsDoc { table, source: Some(SourceRef { path: abs, name, sheet }), ..Default::default() }, info, dir)
            }
        }
        (None, None) => return Err(bad(cmd, "need `path` (.csv, .tsv, .xlsx, .ods or .cfa4credits) or `csv` (the table as text)")),
    };
    if let Some(st) = styling {
        doc.styling = st;
    }
    if let Some(d) = str_p(p, "pictureDir") {
        doc.picture_dir = Some(d.to_string());
    }
    let base = doc.picture_dir.as_ref().map(std::path::PathBuf::from).or(file_dir);
    Ok((doc, info, base))
}

/// Lay out a credits document for the active sequence.
fn lay_doc(s: &Session, cmd: &str, doc: &CreditsDoc, base: Option<std::path::PathBuf>) -> Result<Laid> {
    let credits = doc.credits().map_err(|e| bad(cmd, e.to_string()))?;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let frame = Frame { width: q.settings.width, height: q.settings.height, rate: q.settings.frame_rate };
    cfa4_credits::layout(&credits, &doc.styling, frame, &EngineMeasure { canvas: (frame.width, frame.height), base, sizes: Default::default() })
        .map_err(|e| bad(cmd, e.to_string()))
}

/// Read and lay out the credits a command names, for the active sequence.
fn laid(s: &Session, cmd: &str, p: &Value) -> Result<(Laid, SheetInfo)> {
    let (doc, info, base) = doc_p(cmd, p)?;
    Ok((lay_doc(s, cmd, &doc, base)?, info))
}

fn warnings_json(l: &Laid) -> Value {
    Value::Array(l.warnings.iter().map(|w| json!({"row": w.row, "message": w.message})).collect())
}

fn behaviour_name(b: cfa4_credits::Behaviour) -> &'static str {
    match b {
        cfa4_credits::Behaviour::Card => "card",
        cfa4_credits::Behaviour::Scroll => "scroll",
    }
}

fn preview(s: &mut Session, p: &Value) -> Result<Value> {
    let (l, sheet) = laid(s, "graphics.credits.preview", p)?;
    let total: i64 = l.pages.iter().fold(0i64, |a, pg| a.saturating_add(pg.duration.0).saturating_add(pg.gap_after.0));
    Ok(json!({
        "pages": l.pages.iter().map(|pg| json!({
            "name": pg.name,
            "behaviour": behaviour_name(pg.behaviour),
            "duration": pg.duration.0,
            "seconds": pg.duration.0 as f64 / cfa4_time::TICKS_PER_SECOND as f64,
            "gapAfter": pg.gap_after.0,
            "layers": pg.layers.len(),
            "contentHeight": pg.content_height,
        })).collect::<Vec<_>>(),
        "duration": total.max(0),
        "sheets": sheet.sheets,
        "sheet": sheet.chosen,
        "warnings": warnings_json(&l),
    }))
}

/// Keep only the first of the undo steps made since `before`, under `label`; on an error, undo it.
fn fold_undo(s: &mut Session, before: usize, label: &str, result: &Result<()>) {
    if s.history.undo.len() > before + 1 {
        s.history.undo.truncate(before + 1);
    }
    if let Some(e) = s.history.undo.get_mut(before) {
        e.0 = label.to_string();
    }
    if result.is_err() && s.history.undo.len() > before {
        let _ = s.execute("edit.undo", json!({}));
    }
}

/// Layers edited by hand, carried into a refresh: key → the layer as the user left it.
type Overrides = Vec<(String, EffectInstance)>;

/// Place laid-out pages one after another from `t` (each with its pictures) and link them to
/// `doc`. Returns the placed pages as JSON and the end time.
fn place_pages(
    s: &mut Session,
    cmd: &str,
    label: &str,
    laid: Laid,
    doc: &CreditsDoc,
    base: Option<std::path::PathBuf>,
    start: Tick,
    track: Option<u64>,
    overrides: &Overrides,
) -> Result<(Vec<Value>, Tick)> {
    let rate = s.sequence_rate();
    let frame_h = s.active_sequence().map(|q| f64::from(q.settings.height)).unwrap_or(1080.0);
    let doc_text = doc.to_compact();
    let mut t = start;
    let mut placed = Vec::new();
    let mut items: Vec<(std::path::PathBuf, ItemId)> = Vec::new();
    let mut group = 0u64;
    for (index, pg) in laid.pages.into_iter().enumerate() {
        let mut pp = json!({"time": t.0});
        if let Some(tr) = track {
            pp["track"] = json!(tr);
        }
        // fingerprints of the layers as generated, then the hand edits put back
        let mut layers = pg.layers.clone();
        let mut stamps = Vec::with_capacity(layers.len());
        let mut kept = 0usize;
        for e in &mut layers {
            let uid = cfa4_project::gtemplate::layer_uid(e);
            let key = pg.keys.iter().find(|(u, _)| *u == uid).map(|(_, k)| k.clone()).unwrap_or_default();
            stamps.push(LayerStamp { key: key.clone(), uid, fingerprint: cfa4_credits::doc::layer_fingerprint(e) });
            if let Some((_, old)) = overrides.iter().find(|(k, _)| !key.is_empty() && *k == key) {
                let mut o = old.clone();
                o.layer.get_or_insert_with(Default::default).uid = uid;
                *e = o;
                kept += 1;
            }
        }
        let dur = pg.duration;
        let source =
            move |pr: &mut cfa4_project::Project, (w, h, rate): (u32, u32, cfa4_time::FrameRate)| (crate::graphics::graphic_source(pr, w, h, rate), dur);
        // the asked track if it is free there, else the first free one above
        let clip = match place_video_clip(s, cmd, &pg.name, &pp, label, layers.clone(), source) {
            Ok(c) => c,
            Err(_) if track.is_some() => place_video_clip(s, cmd, &pg.name, &json!({"time": t.0}), label, layers, source)?,
            Err(e) => return Err(e),
        };
        if group == 0 {
            group = clip.0;
        }
        let mut pics = Vec::new();
        for pic in &pg.pictures {
            if let Some(c) = place_picture(s, &base, &mut items, pic, &pg, t, frame_h)? {
                pics.push(c.0);
            }
        }
        let mut meta = pg.meta.clone();
        meta.credits = Some(Box::new(CreditsLink { group, page: index as u32, doc: doc_text.clone(), pictures: pics.clone(), layers: stamps }));
        s.edit_sequence(label, move |q, _, _| {
            let (_, it) = q.find_item_mut(clip).ok_or(cfa4_edit::EditError::NoItem(clip))?;
            it.graphic = Some(Box::new(meta));
            Ok(())
        })?;
        placed.push(json!({"clip": clip.0, "name": pg.name, "start": t.0, "duration": dur.0, "behaviour": behaviour_name(pg.behaviour), "pictures": pics, "keptEdits": kept}));
        t = rate.snap(Tick(t.0.saturating_add(dur.0).saturating_add(pg.gap_after.0).max(0)));
    }
    Ok((placed, t))
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "graphics.credits.import";
    let (doc, info, base) = doc_p(CMD, p)?;
    let l = lay_doc(s, CMD, &doc, base.clone())?;
    let warnings = warnings_json(&l);
    let t = crate::commands::time_p(s, p, "").unwrap_or_else(|| s.playhead());
    let before = s.history.undo.len();
    let mut out = (Vec::new(), t);
    let result = place_pages(s, CMD, "Import Credits", l, &doc, base, t, u64_p(p, "track"), &Vec::new()).map(|r| out = r);
    fold_undo(s, before, "Import Credits", &result);
    result?;
    s.state.graphic_layers.clear();
    let group = out.0.first().and_then(|v| v["clip"].as_u64());
    Ok(json!({"clips": out.0, "end": out.1.0, "group": group, "sheet": info.chosen, "warnings": warnings}))
}

// ------------------------------------------------------------------------------------------------
// Linked credits: save, diff, refresh
// ------------------------------------------------------------------------------------------------

/// A placed credits group: its pages in time order (clip, page link, start, track index).
struct Group {
    id: u64,
    pages: Vec<(ClipId, CreditsLink, Tick, usize)>,
}

fn link_of(it: &cfa4_project::TrackItem) -> Option<&CreditsLink> {
    it.graphic.as_ref().and_then(|g| g.credits.as_deref())
}

/// The credits group of `clip` (default: the first selected credits page).
fn group_p(s: &Session, cmd: &str, p: &Value) -> Result<Group> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let pick = |c: ClipId| q.find_item(c).and_then(|(_, it)| link_of(it)).map(|l| l.group);
    let gid = match u64_p(p, "clip") {
        Some(c) => pick(ClipId(c)).ok_or_else(|| bad(cmd, format!("clip {c} is not a credits page")))?,
        None => s.state.selection.iter().find_map(|c| pick(*c)).ok_or_else(|| bad(cmd, "select a credits page (or give `clip`)"))?,
    };
    let mut pages = Vec::new();
    for (ti, tr) in q.video_tracks.iter().enumerate() {
        for it in &tr.items {
            if let Some(l) = link_of(it).filter(|l| l.group == gid) {
                pages.push((it.id, l.clone(), it.start, ti));
            }
        }
    }
    pages.sort_by_key(|(_, l, t, _)| (*t, l.page));
    Ok(Group { id: gid, pages })
}

fn group_doc(cmd: &str, g: &Group) -> Result<CreditsDoc> {
    let first = g.pages.first().ok_or_else(|| bad(cmd, "the credits have no pages left"))?;
    CreditsDoc::from_bytes(first.1.doc.as_bytes()).map_err(|e| bad(cmd, e.to_string()))
}

fn has_credits_selection(s: &Session) -> std::result::Result<(), String> {
    has_seq(s)?;
    let q = s.active_sequence().ok_or("no sequence is open")?;
    if s.state.selection.iter().any(|c| q.find_item(*c).is_some_and(|(_, it)| link_of(it).is_some())) { Ok(()) } else { Err("select a credits page".into()) }
}

/// Write a credits document: of a placed group (`clip` or the selection), or of a spreadsheet
/// (`path`/`csv` and the other import parameters given as `from`).
fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "graphics.credits.save";
    let out = str_p(p, "to").or_else(|| str_p(p, "file")).ok_or_else(|| bad(CMD, "need `to` (the .cfa4credits file to write)"))?.to_string();
    let doc = match p.get("from") {
        Some(from) if from.is_object() => doc_p(CMD, from)?.0,
        _ => group_doc(CMD, &group_p(s, CMD, p)?)?,
    };
    let out = if std::path::Path::new(&out).extension().is_some() { out } else { format!("{out}.{}", cfa4_credits::doc::EXTENSION) };
    s.services.write_file(&out, doc.to_json().as_bytes()).map_err(|e| EngineError::Other(format!("{out}: {e}")))?;
    Ok(json!({"path": out, "rows": doc.table.len()}))
}

/// The spreadsheet a document came from, as it is now.
fn source_rows(cmd: &str, doc: &CreditsDoc, alt_dir: Option<&std::path::Path>) -> Result<Vec<Vec<String>>> {
    let src = doc.source.as_ref().ok_or_else(|| bad(cmd, "these credits were not made from a spreadsheet file, so there is nothing to refresh from"))?;
    let mut candidates = vec![std::path::PathBuf::from(&src.path)];
    if let Some(d) = alt_dir {
        candidates.push(d.join(&src.name));
    }
    let path = candidates
        .into_iter()
        .find(|c| c.is_file())
        .ok_or_else(|| bad(cmd, format!("the spreadsheet is gone: {} (give `source` with its new place)", src.path)))?;
    let bytes = read_file(cmd, &path.to_string_lossy())?;
    let info = cfa4_credits::read_rows(&bytes, src.sheet.as_deref()).map_err(|e| bad(cmd, e.to_string()))?;
    Ok(info.rows)
}

/// What differs: the row changes and the layers edited by hand.
struct Pending {
    group: Group,
    doc: CreditsDoc,
    new_rows: Vec<Vec<String>>,
    changes: Vec<cfa4_credits::diff::Change>,
    /// (key, page, layer name, text, the edited layer)
    manual: Vec<(String, u32, String, String, EffectInstance)>,
}

fn pending(s: &Session, cmd: &str, p: &Value) -> Result<Pending> {
    let group = group_p(s, cmd, p)?;
    let mut doc = group_doc(cmd, &group)?;
    if let Some(src) = str_p(p, "source") {
        let name = std::path::Path::new(src).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let sheet = doc.source.as_ref().and_then(|x| x.sheet.clone());
        doc.source = Some(SourceRef { path: src.to_string(), name, sheet });
    }
    let alt = s.path.as_ref().and_then(|p| std::path::Path::new(p).parent().map(std::path::Path::to_path_buf));
    let new_rows = source_rows(cmd, &doc, alt.as_deref())?;
    let changes = cfa4_credits::diff::diff(&doc.table, &new_rows);
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let mut manual = Vec::new();
    for (clip, link, _, _) in &group.pages {
        let Some((_, it)) = q.find_item(*clip) else { continue };
        for st in &link.layers {
            let Some(e) = it.effects.iter().find(|e| cfa4_project::gtemplate::layer_uid(e) == st.uid) else { continue };
            if cfa4_credits::doc::layer_fingerprint(e) != st.fingerprint {
                let txt = |k: &str| match e.params.get(k).map(|x| &x.value) {
                    Some(ParamValue::Text(t)) => t.clone(),
                    _ => String::new(),
                };
                manual.push((st.key.clone(), link.page, txt("name"), txt("text"), e.clone()));
            }
        }
    }
    Ok(Pending { group, doc, new_rows, changes, manual })
}

fn rows_json(rows: &[Vec<String>]) -> Value {
    Value::Array(rows.iter().map(|r| json!(r)).collect())
}

fn diff_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "graphics.credits.diff";
    let pd = pending(s, CMD, p)?;
    // which hand edits survive when every change is accepted (their block is unchanged)
    let all: Vec<usize> = pd.changes.iter().map(|c| c.id).collect();
    let merged = cfa4_credits::diff::merge(&pd.doc.table, &pd.new_rows, &pd.changes, &all);
    let keys_after: Vec<String> = {
        let d2 = CreditsDoc { table: merged, ..pd.doc.clone() };
        let base = d2.picture_dir.as_ref().map(std::path::PathBuf::from);
        lay_doc(s, CMD, &d2, base).map(|l| l.pages.into_iter().flat_map(|pg| pg.keys.into_iter().map(|(_, k)| k)).collect()).unwrap_or_default()
    };
    Ok(json!({
        "group": pd.group.id,
        "source": pd.doc.source.as_ref().map(|x| x.path.clone()),
        "changes": pd.changes.iter().map(|c| json!({
            "id": c.id,
            "kind": c.kind,
            // 1-based spreadsheet rows, as people count them
            "oldRows": [c.old_rows.0 + 1, c.old_rows.1],
            "newRows": [c.new_rows.0 + 1, c.new_rows.1],
            "before": rows_json(&c.before),
            "after": rows_json(&c.after),
        })).collect::<Vec<_>>(),
        "manual": pd.manual.iter().map(|(k, page, name, text, _)| json!({
            "key": k, "page": page, "layer": name, "text": text, "survivesAll": keys_after.contains(k),
        })).collect::<Vec<_>>(),
    }))
}

fn refresh(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "graphics.credits.refresh";
    let pd = pending(s, CMD, p)?;
    let accept: Vec<usize> = match p.get("accept") {
        None | Some(Value::Null) => pd.changes.iter().map(|c| c.id).collect(),
        Some(Value::String(a)) if a == "all" => pd.changes.iter().map(|c| c.id).collect(),
        Some(Value::String(a)) if a == "none" => Vec::new(),
        Some(Value::Array(ids)) => ids.iter().filter_map(Value::as_u64).map(|v| v as usize).collect(),
        Some(_) => return Err(bad(CMD, "`accept` is \"all\", \"none\" or a list of change ids")),
    };
    let keep_manual = p.get("keepManual").and_then(Value::as_bool).unwrap_or(true);
    let table = cfa4_credits::diff::merge(&pd.doc.table, &pd.new_rows, &pd.changes, &accept);
    let mut doc = CreditsDoc { table, ..pd.doc.clone() };
    if let Some(st) = p.get("styling").filter(|v| !v.is_null()) {
        doc.styling = styling_p(CMD, &json!({"styling": st}))?;
    }
    let base = doc
        .picture_dir
        .as_ref()
        .map(std::path::PathBuf::from)
        .or_else(|| doc.source.as_ref().and_then(|x| std::path::Path::new(&x.path).parent().map(std::path::Path::to_path_buf)));
    let laid = lay_doc(s, CMD, &doc, base.clone())?;
    let warnings = warnings_json(&laid);
    let overrides: Overrides = if keep_manual { pd.manual.iter().map(|(k, _, _, _, e)| (k.clone(), e.clone())).collect() } else { Vec::new() };
    let start = pd.group.pages.first().map(|x| x.2).unwrap_or_else(|| s.playhead());
    let track = pd.group.pages.first().map(|x| x.3 as u64);
    let mut old: Vec<u64> = Vec::new();
    for (clip, link, _, _) in &pd.group.pages {
        old.push(clip.0);
        old.extend(link.pictures.iter().copied());
    }
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    old.retain(|c| q.find_item(ClipId(*c)).is_some());
    let before = s.history.undo.len();
    let mut out = (Vec::new(), start);
    let result = s
        .execute("edit.clear", json!({"clips": old}))
        .map(|_| ())
        .and_then(|_| place_pages(s, CMD, "Refresh Credits", laid, &doc, base, start, track, &overrides).map(|r| out = r));
    fold_undo(s, before, "Refresh Credits", &result);
    result?;
    s.state.graphic_layers.clear();
    let kept: u64 = out.0.iter().filter_map(|v| v["keptEdits"].as_u64()).sum();
    Ok(json!({
        "clips": out.0,
        "end": out.1.0,
        "accepted": accept.len(),
        "changes": pd.changes.len(),
        "keptEdits": kept,
        "lostEdits": if keep_manual { overrides.len() as u64 - kept.min(overrides.len() as u64) } else { pd.manual.len() as u64 },
        "warnings": warnings,
    }))
}

/// Place one picture of a page as its own clip above the page: it rolls with a scroll page
/// (linear Motion position keyframes that match the graphic's Roll exactly) or fades with a card.
fn place_picture(
    s: &mut Session,
    base: &Option<std::path::PathBuf>,
    items: &mut Vec<(std::path::PathBuf, ItemId)>,
    pic: &cfa4_credits::PicturePlace,
    pg: &cfa4_credits::PageOut,
    t: Tick,
    frame_h: f64,
) -> Result<Option<ClipId>> {
    const CMD: &str = "graphics.credits.import";
    let Some(path) = picture_path(base.as_deref(), &pic.file) else { return Ok(None) };
    let item = match items.iter().find(|(p, _)| *p == path) {
        Some((_, i)) => *i,
        None => {
            let r = s.execute("file.import", json!({"paths": [path.to_string_lossy()]}))?;
            let Some(i) = r["items"].as_array().and_then(|a| a.first()).and_then(Value::as_u64).map(ItemId) else { return Ok(None) };
            items.push((path.clone(), i));
            i
        }
    };
    let dur = pg.duration;
    let name = std::path::Path::new(&pic.file).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| pic.file.clone());
    let clip = place_video_clip(s, CMD, &name, &json!({"time": t.0}), "Import Credits", Vec::new(), move |_, _| (item, dur))?;
    let src_h = s.project.source_size(item).map(|(_, h)| f64::from(h)).filter(|h| *h > 0.0).unwrap_or(pic.size.1);
    let scale = pic.size.1 / src_h * 100.0;
    let (cx, cy) = pic.center;
    let kf = |time: Tick, v: ParamValue| Keyframe { time, value: v, interp: Interpolation::Linear, out_influence: 1.0 / 3.0, in_influence: 1.0 / 3.0 };
    let position = match pg.behaviour {
        // the Roll moves the content from just below the frame (offset = frame height − content
        // top, which is 0) to just above it (offset = −content bottom), at constant speed
        cfa4_credits::Behaviour::Scroll => Param {
            value: ParamValue::Vec2(Vec2::new(cx, cy + frame_h)),
            keyframes: vec![kf(Tick::ZERO, ParamValue::Vec2(Vec2::new(cx, cy + frame_h))), kf(dur, ParamValue::Vec2(Vec2::new(cx, cy - pg.content_height)))],
        },
        cfa4_credits::Behaviour::Card => Param::new(ParamValue::Vec2(Vec2::new(cx, cy))),
    };
    let (fin, fout) = (pg.fade_in, pg.fade_out);
    let card = pg.behaviour == cfa4_credits::Behaviour::Card;
    s.edit_sequence("Import Credits", move |q, _, _| {
        let (_, it) = q.find_item_mut(clip).ok_or(cfa4_edit::EditError::NoItem(clip))?;
        if let Some(m) = it.effects.iter_mut().find(|e| e.effect == "motion") {
            m.params.insert("position".into(), position);
            m.params.insert("scale".into(), Param::new(ParamValue::Float(scale.clamp(0.0, 10_000.0))));
        }
        if card
            && (fin.0 > 0 || fout.0 > 0)
            && let Some(o) = it.effects.iter_mut().find(|e| e.effect == "opacity")
        {
            let mut k = vec![];
            k.push(kf(Tick::ZERO, ParamValue::Float(if fin.0 > 0 { 0.0 } else { 100.0 })));
            if fin.0 > 0 {
                k.push(kf(fin, ParamValue::Float(100.0)));
            }
            if fout.0 > 0 {
                let start = Tick(dur.0.saturating_sub(fout.0));
                if start > fin {
                    k.push(kf(start, ParamValue::Float(100.0)));
                }
                k.push(kf(dur, ParamValue::Float(0.0)));
            }
            k.dedup_by_key(|x| x.time);
            o.params.insert("opacity".into(), Param { value: ParamValue::Float(100.0), keyframes: k });
        }
        Ok(())
    })?;
    Ok(Some(clip))
}

/// A picture of one page at a moment, without changing the project: the page's text layers
/// with the roll or fades applied, and each picture's place outlined (pictures themselves are
/// clips of their own after the import).
fn thumbnail(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "graphics.credits.thumbnail";
    let (l, _) = laid(s, CMD, p)?;
    let index = u64_p(p, "page").unwrap_or(0) as usize;
    let pages = l.pages.len();
    let pg = l.pages.into_iter().nth(index).ok_or_else(|| bad(CMD, format!("there is no page {index} (pages: {pages})")))?;
    let (cw, ch) = s.active_sequence().map(|q| (q.settings.width, q.settings.height)).ok_or(EngineError::NoSequence)?;
    let rate = s.sequence_rate();
    let at = p.get("at").and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(0.5).clamp(0.0, 1.0);
    let w = u64_p(p, "width").unwrap_or(480).clamp(16, 1920) as u32;
    let h = ((f64::from(w) * f64::from(ch) / f64::from(cw.max(1))).round() as u32).max(1);
    let mut layers = pg.layers;
    for e in &mut layers {
        if e.effect == "graphic_shape" {
            e.params.insert("stroke".into(), Param::new(ParamValue::Bool(true)));
            e.params.insert("stroke_color".into(), Param::new(ParamValue::Color([0.6, 0.6, 0.6, 1.0])));
            e.params.insert("stroke_width".into(), Param::new(ParamValue::Float(f64::from(ch) / 360.0)));
        }
    }
    let mut pr = cfa4_project::Project::new("credits preview");
    let g = pr.add_item("G", cfa4_project::Label::Rose, cfa4_project::ItemKind::Graphic { width: cw, height: ch, rate }, None);
    let range = cfa4_time::TimeRange::new(Tick::ZERO, pg.duration);
    let mut it = pr.make_track_item(g, cfa4_project::TrackKind::Video, Tick::ZERO, range, rate).ok_or_else(|| bad(CMD, "bad preview item"))?;
    it.effects.extend(layers);
    it.graphic = Some(Box::new(pg.meta));
    let t = rate.snap(Tick((pg.duration.0 as f64 * at) as i64)).min(Tick(pg.duration.0.saturating_sub(rate.frame_duration().0).max(0)));
    let mut img = cfa4_render::image::Image::new(w as usize, h as usize);
    let k = f64::from(w) / f64::from(cw.max(1));
    cfa4_render::graphic_clip::render_graphic(&it, it.source_in + t, (cw, ch), &cfa4_geom::Affine::scale(k, k), &mut img);
    let png = cfa4_export::encode_png(img.to_rgba8(), w, h).map_err(|x| EngineError::Other(x.to_string()))?;
    Ok(json!({"width": w, "height": h, "page": index, "pages": pages, "time": t.0, "pngBase64": cfa4_project::gtemplate::base64_encode(&png)}))
}

fn styles(_: &mut Session, p: &Value) -> Result<Value> {
    let styling = styling_p("graphics.credits.styles", p)?;
    let (pages, contents) = styling.names();
    let starter = Styling {
        page_styles: cfa4_credits::style::builtin_page_styles(),
        content_styles: cfa4_credits::style::builtin_content_styles(),
        ..Styling::default()
    };
    Ok(json!({
        "pageStyles": pages,
        "contentStyles": contents,
        "starter": serde_json::to_value(&starter).unwrap_or(Value::Null),
        "columns": ["Head", "Body", "Tail", "Vertical Gap", "Content Style", "Spine Position", "Page Style", "Page Runtime", "Page Gap"],
    }))
}

pub fn commands() -> Vec<CommandSpec> {
    const P: &str = r#"{"path":str (.csv, .tsv, .xlsx, .ods, .cfa4credits)?,"sheet":name|number (a worksheet of an .xlsx / .ods; default the first)?,"pictureDir":str (folder of {{Pic file}} pictures; default the spreadsheet's folder)?,"csv":str (the table as text)?,"styling":object|JSON text|path of a .json?,"time":ticks?,"track":index?}"#;
    vec![
        CommandSpec {
            id: "graphics.credits.import",
            label: "Import Credits Spreadsheet…",
            menu: &["Graphics and Titles"],
            shortcut: None,
            params: P,
            enabled: has_seq,
            run: import,
            journal: true,
        },
        CommandSpec {
            id: "graphics.credits.preview",
            label: "Preview Credits",
            menu: &[],
            shortcut: None,
            params: P,
            enabled: has_seq,
            run: preview,
            journal: false,
        },
        CommandSpec {
            id: "graphics.credits.thumbnail",
            label: "Credits Page Picture",
            menu: &[],
            shortcut: None,
            params: r#"as graphics.credits.preview, plus {"page":index=0,"at":0..1=0.5 (moment in the page),"width":px=480}"#,
            enabled: has_seq,
            run: thumbnail,
            journal: false,
        },
        CommandSpec {
            id: "graphics.credits.save",
            label: "Save Credits File…",
            menu: &[],
            shortcut: None,
            params: r#"{"to":path (.cfa4credits),"clip":id? (a page of placed credits; default the selection),"from":{import parameters}? (save a spreadsheet instead)}"#,
            enabled: has_seq,
            run: save,
            journal: false,
        },
        CommandSpec {
            id: "graphics.credits.diff",
            label: "Compare Credits with Spreadsheet",
            menu: &[],
            shortcut: None,
            params: r#"{"clip":id? (a page of the credits; default the selection),"source":path? (the spreadsheet's new place)}"#,
            enabled: has_credits_selection,
            run: diff_cmd,
            journal: false,
        },
        CommandSpec {
            id: "graphics.credits.refresh",
            label: "Refresh Credits from Spreadsheet…",
            menu: &["Graphics and Titles"],
            shortcut: None,
            params: r#"{"clip":id?,"source":path?,"accept":"all"|"none"|[change id]="all","keepManual":bool=true,"styling":object|JSON text|path?}"#,
            enabled: has_credits_selection,
            run: refresh,
            journal: true,
        },
        CommandSpec {
            id: "graphics.credits.styles",
            label: "Credits Styles",
            menu: &[],
            shortcut: None,
            params: r#"{"styling":object|JSON text|path?}"#,
            enabled: always,
            run: styles,
            journal: false,
        },
    ]
}

#[cfg(test)]
#[path = "credits_tests.rs"]
mod tests;
