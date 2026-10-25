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

use cfa4_credits::layout::{Frame, Measure};
use cfa4_credits::{Laid, Styling};
use cfa4_geom::Vec2;
use cfa4_project::graphic::eval_layer;
use cfa4_project::{ClipId, EffectInstance, Interpolation, ItemId, Keyframe, Param, ParamValue};
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

/// The folder pictures are read from: `pictureDir`, else the spreadsheet's folder.
fn picture_base(p: &Value) -> Option<std::path::PathBuf> {
    if let Some(d) = str_p(p, "pictureDir") {
        return Some(std::path::PathBuf::from(d));
    }
    str_p(p, "path").and_then(|f| std::path::Path::new(f).parent().map(std::path::Path::to_path_buf))
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

/// Read and lay out the spreadsheet a command names, for the active sequence.
fn laid(s: &Session, cmd: &str, p: &Value) -> Result<(Laid, cfa4_credits::office::Sheet)> {
    let styling = styling_p(cmd, p)?;
    let bytes = match (str_p(p, "csv"), str_p(p, "path")) {
        (Some(text), _) => text.as_bytes().to_vec(),
        (None, Some(path)) => {
            if path.to_lowercase().ends_with(".numbers") {
                return Err(bad(cmd, "Apple Numbers files are not read; export the sheet as .xlsx or CSV"));
            }
            read_file(cmd, path)?
        }
        (None, None) => return Err(bad(cmd, "need `path` (.csv, .tsv, .xlsx or .ods) or `csv` (the table as text)")),
    };
    let (credits, sheet) = cfa4_credits::read_with(&bytes, &styling, str_p(p, "sheet")).map_err(|e| bad(cmd, e.to_string()))?;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let frame = Frame { width: q.settings.width, height: q.settings.height, rate: q.settings.frame_rate };
    let l = cfa4_credits::layout(
        &credits,
        &styling,
        frame,
        &EngineMeasure { canvas: (frame.width, frame.height), base: picture_base(p), sizes: Default::default() },
    )
    .map_err(|e| bad(cmd, e.to_string()))?;
    Ok((l, sheet))
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

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "graphics.credits.import";
    let (l, sheet) = laid(s, CMD, p)?;
    let warnings = warnings_json(&l);
    let rate = s.sequence_rate();
    let mut t = crate::commands::time_p(s, p, "").unwrap_or_else(|| s.playhead());
    let before = s.history.undo.len();
    let mut placed = Vec::new();
    let base = picture_base(p);
    let frame_h = s.active_sequence().map(|q| f64::from(q.settings.height)).unwrap_or(1080.0);
    let mut items: Vec<(std::path::PathBuf, ItemId)> = Vec::new();
    let result = (|| -> Result<()> {
        for pg in l.pages {
            let mut pp = json!({"time": t.0});
            if let Some(tr) = u64_p(p, "track") {
                pp["track"] = json!(tr);
            }
            let dur = pg.duration;
            let meta = pg.meta.clone();
            let clip = place_video_clip(s, CMD, &pg.name, &pp, "Import Credits", pg.layers.clone(), move |pr, (w, h, rate)| {
                (crate::graphics::graphic_source(pr, w, h, rate), dur)
            })?;
            s.edit_sequence("Import Credits", move |q, _, _| {
                let (_, it) = q.find_item_mut(clip).ok_or(cfa4_edit::EditError::NoItem(clip))?;
                it.graphic = Some(Box::new(meta));
                Ok(())
            })?;
            let mut pics = Vec::new();
            for pic in &pg.pictures {
                if let Some(c) = place_picture(s, &base, &mut items, pic, &pg, t, frame_h)? {
                    pics.push(c.0);
                }
            }
            placed.push(json!({"clip": clip.0, "name": pg.name, "start": t.0, "duration": dur.0, "behaviour": behaviour_name(pg.behaviour), "pictures": pics}));
            t = rate.snap(Tick(t.0.saturating_add(dur.0).saturating_add(pg.gap_after.0).max(0)));
        }
        Ok(())
    })();
    // one undo step for the whole sequence of pages
    if s.history.undo.len() > before + 1 {
        s.history.undo.truncate(before + 1);
    }
    if let Some(e) = s.history.undo.get_mut(before) {
        e.0 = "Import Credits".to_string();
    }
    if let Err(e) = result {
        if s.history.undo.len() > before {
            let _ = s.execute("edit.undo", json!({}));
        }
        return Err(e);
    }
    s.state.graphic_layers.clear();
    Ok(json!({"clips": placed, "end": t.0, "sheet": sheet.chosen, "warnings": warnings}))
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
    const P: &str = r#"{"path":str (.csv, .tsv, .xlsx, .ods)?,"sheet":name|number (a worksheet of an .xlsx / .ods; default the first)?,"pictureDir":str (folder of {{Pic file}} pictures; default the spreadsheet's folder)?,"csv":str (the table as text)?,"styling":object|JSON text|path of a .json?,"time":ticks?,"track":index?}"#;
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
