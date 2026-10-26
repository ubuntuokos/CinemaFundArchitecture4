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
//! - `graphics.credits.validate` checks credits (names, fonts and missing letters, title-safe
//!   area, contrast, text size, scroll speed, card length, pictures); the preview reports the
//!   same `issues`.
//! - **Fit to music.** `graphics.credits.music` analyses a music clip (length, where the sound
//!   ends, beats) and offers lengths without changing anything; `graphics.credits.fit` fits
//!   placed credits to a length (cards keep theirs, rolls stretch their gaps), optionally with
//!   cards on the beat and the credits moved to where the music starts; the setting stays in the
//!   credits document, so refreshing keeps it.
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

/// Fonts as the text engine finds them.
struct EngineFonts;

impl cfa4_credits::validate::FontInfo for EngineFonts {
    fn check(&self, family: &str, style: &str, text: &str) -> cfa4_credits::validate::FontCheck {
        let r = cfa4_text::resolve(family, style);
        let face = cfa4_text::fonts::face(r.face);
        let mut uncovered: Vec<char> = Vec::new();
        for c in text.chars().filter(|c| !c.is_whitespace() && !c.is_control()) {
            if !uncovered.contains(&c) && uncovered.len() < 32 && !face.has_char(c) {
                uncovered.push(c);
            }
        }
        cfa4_credits::validate::FontCheck { missing: r.missing, uncovered }
    }
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
    read_file_max(cmd, path, MAX_FILE)
}

fn read_file_max(cmd: &str, path: &str, max: u64) -> Result<Vec<u8>> {
    let meta = std::fs::metadata(path).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
    if meta.len() > max {
        return Err(bad(cmd, format!("{path} is larger than {} MiB", max >> 20)));
    }
    std::fs::read(path).map_err(|e| bad(cmd, format!("{path}: {e}")))
}

/// The largest file a credits command reads: a spreadsheet, or a credits document (which may be
/// larger, up to the document limit).
fn max_read() -> u64 {
    MAX_FILE.max(cfa4_credits::doc::MAX_DOC_BYTES as u64)
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
            let bytes = read_file_max(cmd, path, max_read())?;
            let dir = std::path::Path::new(path).parent().map(std::path::Path::to_path_buf);
            if cfa4_credits::doc::is_doc(&bytes) {
                let mut d = CreditsDoc::from_bytes(&bytes).map_err(|e| bad(cmd, e.to_string()))?;
                // pictures are next to the document unless it says otherwise: remember that
                // folder, so a refresh (which starts from the spreadsheet) still finds them
                if d.picture_dir.is_none()
                    && let Some(dir) = &dir
                {
                    let abs = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.clone());
                    d.picture_dir = Some(abs.to_string_lossy().into_owned());
                }
                (d, SheetInfo::default(), dir)
            } else if bytes.len() as u64 > MAX_FILE {
                return Err(bad(cmd, format!("{path} is larger than {} MiB", MAX_FILE >> 20)));
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
    match p.get("fit") {
        None => {}
        Some(Value::Null) => doc.fit = None,
        Some(v) => {
            let f: cfa4_credits::fit::Fit = serde_json::from_value(v.clone()).map_err(|e| bad(cmd, format!("fit: {e}")))?;
            doc.fit = (f.total > 0).then_some(f);
        }
    }
    let base = doc.picture_dir.as_ref().map(std::path::PathBuf::from).or(file_dir);
    Ok((doc, info, base))
}

/// Lay out a credits document for the active sequence.
fn lay_doc(s: &Session, cmd: &str, doc: &CreditsDoc, base: Option<std::path::PathBuf>) -> Result<Laid> {
    lay_doc_report(s, cmd, doc, base).map(|(l, _)| l)
}

/// [`lay_doc`], with the fitting report when the document is fitted to a length.
fn lay_doc_report(s: &Session, cmd: &str, doc: &CreditsDoc, base: Option<std::path::PathBuf>) -> Result<(Laid, Option<cfa4_credits::fit::FitReport>)> {
    let credits = doc.credits().map_err(|e| bad(cmd, e.to_string()))?;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let frame = Frame { width: q.settings.width, height: q.settings.height, rate: q.settings.frame_rate };
    let measure = EngineMeasure { canvas: (frame.width, frame.height), base, sizes: Default::default() };
    match doc.fit.as_ref().filter(|f| f.total > 0) {
        Some(fit) => {
            let (_, l, r) = cfa4_credits::fit::fit_layout(&credits, &doc.styling, frame, &measure, fit).map_err(|e| bad(cmd, e.to_string()))?;
            Ok((l, Some(r)))
        }
        None => cfa4_credits::layout(&credits, &doc.styling, frame, &measure).map(|l| (l, None)).map_err(|e| bad(cmd, e.to_string())),
    }
}

/// The checks on laid-out credits (`cfa4_credits::validate`), as JSON.
fn issues_json(s: &Session, cmd: &str, doc: &CreditsDoc, base: Option<std::path::PathBuf>, laid: &Laid) -> Result<Value> {
    let credits = doc.credits().map_err(|e| bad(cmd, e.to_string()))?;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let frame = Frame { width: q.settings.width, height: q.settings.height, rate: q.settings.frame_rate };
    let measure = EngineMeasure { canvas: (frame.width, frame.height), base, sizes: Default::default() };
    let issues = cfa4_credits::validate::validate(&credits, laid, frame, &measure, &EngineFonts);
    Ok(serde_json::to_value(issues).unwrap_or(Value::Array(vec![])))
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
    const CMD: &str = "graphics.credits.preview";
    let (doc, sheet, base) = doc_p(CMD, p)?;
    let l = lay_doc(s, CMD, &doc, base.clone())?;
    let issues = issues_json(s, CMD, &doc, base, &l)?;
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
        "issues": issues,
    }))
}

/// Check credits: the spreadsheet or file a command names, or placed credits (`clip` or the
/// selection, as they were accepted).
fn validate_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "graphics.credits.validate";
    let (doc, base) = if p.get("path").is_some() || p.get("csv").is_some() {
        let (d, _, b) = doc_p(CMD, p)?;
        (d, b)
    } else {
        let d = group_doc(CMD, &group_p(s, CMD, p)?)?;
        let b = d
            .picture_dir
            .as_ref()
            .map(std::path::PathBuf::from)
            .or_else(|| d.source.as_ref().and_then(|x| std::path::Path::new(&x.path).parent().map(std::path::Path::to_path_buf)));
        (d, b)
    };
    let l = lay_doc(s, CMD, &doc, base.clone())?;
    let issues = issues_json(s, CMD, &doc, base, &l)?;
    let count = |sev: &str| issues.as_array().map(|a| a.iter().filter(|i| i["severity"] == sev).count()).unwrap_or(0);
    Ok(json!({"errors": count("error"), "warnings": count("warning"), "infos": count("info"), "issues": issues}))
}

/// Layers edited by hand, carried into a refresh: key → the layer as the user left it, and the
/// layer as it was generated (when known), so only what the user changed is carried over.
type Overrides = Vec<(String, EffectInstance, Option<EffectInstance>)>;

/// The newly generated layer `new` with the user's changes to it: every parameter (and the
/// per-character styles, and on/off) the user changed from `base` (the layer as generated before)
/// is taken from `user`; the rest – where the layout put it, above all – stays new. Without
/// `base`, the user's layer as a whole.
fn rebase_edit(new: &EffectInstance, user: &EffectInstance, base: Option<&EffectInstance>) -> EffectInstance {
    let Some(base) = base else { return user.clone() };
    let mut out = new.clone();
    for (k, v) in &user.params {
        if base.params.get(k) != Some(v) {
            out.params.insert(k.clone(), v.clone());
        }
    }
    let runs = |e: &EffectInstance| e.layer.as_ref().map(|l| l.runs.clone()).unwrap_or_default();
    if runs(user) != runs(base) {
        out.layer.get_or_insert_with(Default::default).runs = runs(user);
    }
    if user.enabled != base.enabled {
        out.enabled = user.enabled;
    }
    out
}

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
            if let Some((_, old, base)) = overrides.iter().find(|(k, _, _)| !key.is_empty() && *k == key) {
                let mut o = rebase_edit(e, old, base.as_ref());
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
    // one undo step, and nothing left behind if a page cannot be placed
    let track = u64_p(p, "track");
    let out = s.edit_group("Import Credits", |s| place_pages(s, CMD, "Import Credits", l, &doc, base, t, track, &Vec::new()))?;
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
    let text = doc.to_file_text().map_err(|e| bad(CMD, e.to_string()))?;
    s.services.write_file(&out, text.as_bytes()).map_err(|e| EngineError::Other(format!("{out}: {e}")))?;
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
    let manual = manual_edits(s, &group)?;
    Ok(Pending { group, doc, new_rows, changes, manual })
}

/// Layers of a group edited by hand: (key, page, layer name, text, the edited layer).
fn manual_edits(s: &Session, group: &Group) -> Result<Vec<(String, u32, String, String, EffectInstance)>> {
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
    Ok(manual)
}

/// The folder a document's pictures are read from.
fn doc_base(doc: &CreditsDoc) -> Option<std::path::PathBuf> {
    doc.picture_dir
        .as_ref()
        .map(std::path::PathBuf::from)
        .or_else(|| doc.source.as_ref().and_then(|x| std::path::Path::new(&x.path).parent().map(std::path::Path::to_path_buf)))
}

/// Replace a placed group with `doc` laid out anew, from `start` (default: where it was), keeping
/// `overrides`; one undo step under `label`. Returns the placed pages and the end.
fn replace_group(
    s: &mut Session,
    cmd: &str,
    label: &str,
    group: &Group,
    doc: &CreditsDoc,
    laid: Laid,
    overrides: &Overrides,
    start: Option<Tick>,
) -> Result<(Vec<Value>, Tick)> {
    let start = start.or_else(|| group.pages.first().map(|x| x.2)).unwrap_or_else(|| s.playhead());
    let track = group.pages.first().map(|x| x.3 as u64);
    let mut old: Vec<u64> = Vec::new();
    for (clip, link, _, _) in &group.pages {
        old.push(clip.0);
        old.extend(link.pictures.iter().copied());
    }
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    old.retain(|c| q.find_item(ClipId(*c)).is_some());
    // one undo step; on any failure the old pages are back as they were
    let out = s.edit_group(label, |s| {
        s.execute("edit.clear", json!({"clips": old}))?;
        place_pages(s, cmd, label, laid, doc, doc_base(doc), start, track, overrides)
    })?;
    s.state.graphic_layers.clear();
    Ok(out)
}

/// Hand edits with the layer each was generated as: `doc` (as accepted before) laid out again.
fn baselined(s: &Session, cmd: &str, doc: &CreditsDoc, manual: &[(String, u32, String, String, EffectInstance)]) -> Overrides {
    let generated: Vec<(String, EffectInstance)> = lay_doc(s, cmd, doc, doc_base(doc))
        .map(|l| {
            l.pages
                .into_iter()
                .flat_map(|pg| {
                    let keys = pg.keys;
                    pg.layers.into_iter().filter_map(move |e| {
                        let uid = cfa4_project::gtemplate::layer_uid(&e);
                        keys.iter().find(|(u, _)| *u == uid).map(|(_, k)| (k.clone(), e))
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    manual
        .iter()
        .map(|(k, _, _, _, e)| {
            let base = generated.iter().find(|(gk, _)| gk == k).map(|(_, g)| g.clone());
            (k.clone(), e.clone(), base)
        })
        .collect()
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
        // a page of the group, so a later refresh can name exactly these credits
        "clip": pd.group.pages.first().map(|x| x.0.0),
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
    let laid = lay_doc(s, CMD, &doc, doc_base(&doc))?;
    let warnings = warnings_json(&laid);
    let overrides: Overrides = if keep_manual { baselined(s, CMD, &pd.doc, &pd.manual) } else { Vec::new() };
    let out = replace_group(s, CMD, "Refresh Credits", &pd.group, &doc, laid, &overrides, None)?;
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

// ------------------------------------------------------------------------------------------------
// Fit to music
// ------------------------------------------------------------------------------------------------

/// Longest music analysed for beats (longer music still gives its length and where it ends).
const MAX_BEAT_SECONDS: i64 = 15 * 60;
/// Below this level (−50 dBFS RMS over 50 ms) the music counts as silent.
const SILENCE_RMS: f32 = 0.003_16;

/// The audio clip a command means: `audio`, else the first selected audio clip, else the audio
/// clip that overlaps the credits most (without credits: the one at the playhead).
fn audio_clip(s: &Session, cmd: &str, p: &Value, group: Option<&Group>) -> Result<ClipId> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let is_audio = |c: ClipId| q.find_item(c).and_then(|(t, _)| q.track(t)).is_some_and(|t| t.kind == cfa4_project::TrackKind::Audio);
    if let Some(c) = u64_p(p, "audio") {
        let c = ClipId(c);
        return if is_audio(c) { Ok(c) } else { Err(bad(cmd, format!("clip {} is not an audio clip", c.0))) };
    }
    if let Some(c) = s.state.selection.iter().copied().find(|c| is_audio(*c)) {
        return Ok(c);
    }
    // the audio clip that overlaps the credits most (or plays at the playhead)
    let (a, b) = match group {
        Some(g) => {
            let a = g.pages.first().map(|x| x.2).unwrap_or_else(|| s.playhead());
            let b = g.pages.iter().filter_map(|(c, _, _, _)| q.find_item(*c).map(|(_, it)| it.start + it.duration)).max().unwrap_or(a);
            (a, b.max(a + q.settings.frame_rate.frame_duration()))
        }
        None => {
            let t = s.playhead();
            (t, t + q.settings.frame_rate.frame_duration())
        }
    };
    q.audio_tracks
        .iter()
        .flat_map(|t| t.items.iter())
        .map(|it| (it.id, (it.start + it.duration).min(b).0 - it.start.max(a).0))
        .filter(|(_, overlap)| *overlap > 0)
        .max_by_key(|(c, overlap)| (*overlap, std::cmp::Reverse(c.0)))
        .map(|(c, _)| c)
        .ok_or_else(|| bad(cmd, "select the music (an audio clip), or give `audio`"))
}

/// What the music clip is like: its length, where its sound ends, its beats (clip time).
fn music_info(s: &Session, cmd: &str, clip: ClipId) -> Result<(String, Tick, cfa4_credits::fit::Music, f64)> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (_, it) = q.find_item(clip).ok_or_else(|| bad(cmd, "no such clip"))?;
    let (item, source_in, dur, start, name) = (it.item, it.source_in, it.duration, it.start, it.name.clone());
    let sr = q.settings.sample_rate.max(1);
    let provider = s.media.provider(s.project.clone(), s.services.clone());
    let src = cfa4_render::SourceProvider::source(&provider, item).ok_or_else(|| bad(cmd, "the music's media is offline"))?;
    let m0 = source_in.to_units_floor(i64::from(sr));
    let len = dur.to_units_floor(i64::from(sr)).max(0);
    // where the sound ends: RMS over 50 ms windows, read in 10-second pieces
    let win = (sr / 20).max(1) as i64;
    let chunk = i64::from(sr) * 10;
    let mut last_loud: Option<i64> = None;
    let mut off = 0i64;
    while off < len {
        let n = chunk.min(len - off);
        let buf = src.audio(m0 + off, n as usize, sr).map_err(|e| bad(cmd, format!("reading the music: {e}")))?;
        let mut w0 = 0i64;
        while w0 < n {
            let w1 = (w0 + win).min(n);
            let mut sum = 0f64;
            let mut cnt = 0usize;
            for ch in &buf.channels {
                for v in ch.get(w0 as usize..w1 as usize).unwrap_or(&[]) {
                    sum += f64::from(*v) * f64::from(*v);
                    cnt += 1;
                }
            }
            if cnt > 0 && (sum / cnt as f64).sqrt() as f32 > SILENCE_RMS {
                last_loud = Some(off + w1);
            }
            w0 = w1;
        }
        off += n;
    }
    let to_tick = |samples: i64| Tick::from_units(samples, i64::from(sr));
    let sound_end = last_loud.map(to_tick);
    // beats, as Remix finds them
    let mut beats = Vec::new();
    let mut bpm = 0.0;
    if len > 0
        && len <= MAX_BEAT_SECONDS * i64::from(sr)
        && let Ok(buf) = src.audio(m0, len as usize, sr)
    {
        let refs: Vec<&[f32]> = buf.channels.iter().map(Vec::as_slice).collect();
        if let Ok(a) = cfa4_audio_dsp::remix::analyze(&refs, sr) {
            beats = a.beats.iter().map(|b| to_tick(*b)).collect();
            bpm = a.bpm;
        }
    }
    Ok((name, start, cfa4_credits::fit::Music { duration: dur, sound_end, beats }, bpm))
}

fn group_or_none(s: &Session, cmd: &str, p: &Value) -> Option<Group> {
    if u64_p(p, "clip").is_some()
        || s.state.selection.iter().any(|c| s.active_sequence().and_then(|q| q.find_item(*c)).is_some_and(|(_, it)| link_of(it).is_some()))
    {
        group_p(s, cmd, p).ok()
    } else {
        None
    }
}

/// Analyse the music and offer lengths. Changes nothing.
fn music(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "graphics.credits.music";
    let group = group_or_none(s, CMD, p);
    let audio = audio_clip(s, CMD, p, group.as_ref())?;
    let (name, start, m, bpm) = music_info(s, CMD, audio)?;
    let suggestions = cfa4_credits::fit::suggestions(&m);
    let beat = cfa4_credits::fit::beat_period(&m.beats).map(|t| t.0);
    let (natural, fit) = match &group {
        Some(g) => {
            let mut doc = group_doc(CMD, g)?;
            let fit = doc.fit.take();
            let l = lay_doc(s, CMD, &doc, doc_base(&doc))?;
            (Some(cfa4_credits::fit::total_length(&l).0), fit)
        }
        None => (None, None),
    };
    Ok(json!({
        "audio": audio.0,
        "name": name,
        "start": start.0,
        "duration": m.duration.0,
        "soundEnd": m.sound_end.map(|t| t.0),
        "bpm": if bpm > 0.0 { json!((bpm * 10.0).round() / 10.0) } else { Value::Null },
        "beat": beat,
        "beats": m.beats.len(),
        "suggestions": suggestions,
        "credits": group.as_ref().map(|g| g.id),
        "natural": natural,
        "fit": fit,
    }))
}

/// Fit placed credits to a length – the music's, or one given – in place, as one undo step.
fn fit_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "graphics.credits.fit";
    let group = group_p(s, CMD, p)?;
    let mut doc = group_doc(CMD, &group)?;
    let off = p.get("off").and_then(Value::as_bool).unwrap_or(false);
    let mut start = None;
    if off {
        doc.fit = None;
    } else {
        let wants_music = p.get("use").is_some()
            || p.get("audio").is_some()
            || p.get("cardsOnBeat").and_then(Value::as_bool).unwrap_or(false)
            || p.get("alignToAudio").and_then(Value::as_bool).unwrap_or(false);
        let music = if wants_music || (p.get("total").is_none() && p.get("seconds").is_none()) {
            let audio = audio_clip(s, CMD, p, Some(&group))?;
            Some(music_info(s, CMD, audio)?)
        } else {
            None
        };
        let total = if let Some(t) = p.get("total").and_then(Value::as_i64) {
            Tick(t)
        } else if let Some(sec) = p.get("seconds").and_then(Value::as_f64).filter(|v| v.is_finite()) {
            Tick::from_seconds_f64(sec)
        } else {
            let (_, _, m, _) = music.as_ref().ok_or_else(|| bad(CMD, "need `total`, `seconds` or music"))?;
            let want = str_p(p, "use").unwrap_or("musicLength");
            let sug = cfa4_credits::fit::suggestions(m);
            Tick(
                sug.iter()
                    .find(|x| x.id == want)
                    .ok_or_else(|| {
                        bad(CMD, format!("no `{want}` length for this music (offered: {})", sug.iter().map(|x| x.id).collect::<Vec<_>>().join(", ")))
                    })?
                    .total,
            )
        };
        if total.0 <= 0 || total.0 > 24 * 3600 * cfa4_time::TICKS_PER_SECOND {
            return Err(bad(CMD, "the length must be between one frame and 24 hours"));
        }
        let card_beat = if p.get("cardsOnBeat").and_then(Value::as_bool).unwrap_or(false) {
            let (_, _, m, _) = music.as_ref().ok_or_else(|| bad(CMD, "cards on the beat need music"))?;
            cfa4_credits::fit::beat_period(&m.beats).ok_or_else(|| bad(CMD, "no beat was found in the music"))?.0
        } else {
            0
        };
        if p.get("alignToAudio").and_then(Value::as_bool).unwrap_or(false)
            && let Some((_, st, _, _)) = &music
        {
            start = Some(*st);
        }
        let name = music.as_ref().map(|m| m.0.clone()).unwrap_or_default();
        doc.fit = Some(cfa4_credits::fit::Fit { total: total.0, card_beat, music: name });
    }
    let (laid, report) = lay_doc_report(s, CMD, &doc, doc_base(&doc))?;
    let warnings = warnings_json(&laid);
    let manual = manual_edits(s, &group)?;
    let overrides: Overrides = baselined(s, CMD, &group_doc(CMD, &group)?, &manual);
    let label = if off { "Unfit Credits" } else { "Fit Credits to Music" };
    let out = replace_group(s, CMD, label, &group, &doc, laid, &overrides, start)?;
    Ok(json!({"clips": out.0, "end": out.1.0, "fit": doc.fit, "report": report, "warnings": warnings}))
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
            id: "graphics.credits.validate",
            label: "Check Credits",
            menu: &[],
            shortcut: None,
            params: r#"as graphics.credits.preview, or {"clip":id?} for placed credits (default the selection) → {errors, warnings, infos, issues:[{code, severity, page?, row?, values, message}]}"#,
            enabled: has_seq,
            run: validate_cmd,
            journal: false,
        },
        CommandSpec {
            id: "graphics.credits.music",
            label: "Analyse Music for Credits",
            menu: &[],
            shortcut: None,
            params: r#"{"audio":clip? (default: the selected audio clip, else the one under the credits),"clip":credits page?} → {audio, name, start, duration, soundEnd, bpm, beat, beats, suggestions:[{id: musicLength|soundEnd|lastBar, total}], natural, fit}; changes nothing"#,
            enabled: has_seq,
            run: music,
            journal: false,
        },
        CommandSpec {
            id: "graphics.credits.fit",
            label: "Fit Credits to Music…",
            menu: &["Graphics and Titles"],
            shortcut: None,
            params: r#"{"clip":credits page?,"total":ticks?|"seconds":n?|"use":"musicLength|soundEnd|lastBar"="musicLength","audio":clip?,"cardsOnBeat":bool=false,"alignToAudio":bool=false,"off":bool=false}"#,
            enabled: has_credits_selection,
            run: fit_cmd,
            journal: true,
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
