//! `qc.contactSheet` (Kdenlive plan K37): a PNG contact sheet of evenly spaced frames of a
//! sequence or a clip, drawn by the same code as the QC report's sheet ([`cfa4_qc::picture`]).
//! Nothing in the project changes.

use std::path::Path;

use cfa4_project::ItemId;
use cfa4_render::RenderOptions;
use cfa4_time::Tick;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, str_p, u64_p};
use crate::{EngineError, Result, Session};

const CMD: &str = "qc.contactSheet";
const MAX_FRAMES: usize = 64;

fn contact_sheet(s: &mut Session, p: &Value) -> Result<Value> {
    let frames = match p.get("frames") {
        None | Some(Value::Null) => 12,
        Some(v) => v.as_u64().filter(|n| (1..=MAX_FRAMES as u64).contains(n)).ok_or_else(|| bad(CMD, format!("`frames` must be 1 to {MAX_FRAMES}")))? as usize,
    };
    let columns = match p.get("columns") {
        None | Some(Value::Null) => cfa4_qc::picture::SHEET_COLUMNS,
        Some(v) => v.as_u64().filter(|n| (1..=16).contains(n)).ok_or_else(|| bad(CMD, "`columns` must be 1 to 16"))? as usize,
    };
    let thumb = match p.get("thumb") {
        None | Some(Value::Null) => cfa4_qc::picture::THUMB,
        Some(v) => v.as_u64().filter(|n| (16..=640).contains(n)).ok_or_else(|| bad(CMD, "`thumb` must be 16 to 640 pixels"))? as usize,
    };
    let (item, is_sequence) = match (u64_p(p, "item"), u64_p(p, "sequence")) {
        (Some(_), Some(_)) => return Err(bad(CMD, "give either `item` or `sequence`, not both")),
        (Some(i), None) => {
            let id = ItemId(i);
            let it = s.project.item(id).ok_or_else(|| bad(CMD, "no such item"))?;
            (id, s.project.sequence(id).is_some() && matches!(it.kind, cfa4_project::ItemKind::Sequence(_)))
        }
        (None, Some(q)) => {
            let id = ItemId(q);
            s.project.sequence(id).ok_or_else(|| bad(CMD, "no such sequence"))?;
            (id, true)
        }
        (None, None) => (crate::export_tools::sequence_param(s, p, CMD)?, true),
    };
    let (duration, rate, name) = if is_sequence {
        let q = s.project.sequence(item).ok_or(EngineError::NoSequence)?;
        (q.duration(), q.settings.frame_rate, s.project.item(item).map(|i| i.name.clone()).unwrap_or_default())
    } else {
        let it = s.project.item(item).ok_or_else(|| bad(CMD, "no such item"))?;
        (it.duration(), it.frame_rate(), it.name.clone())
    };
    if duration <= Tick::ZERO {
        return Err(bad(CMD, "nothing to show: the duration is zero"));
    }
    let path = match str_p(p, "path").filter(|x| !x.trim().is_empty()) {
        Some(x) => x.to_string(),
        None => {
            let dir = crate::export_tools::default_export_dir(s);
            let base = crate::export_tools::file_safe(&name);
            let path = dir.join(format!("{base}.ContactSheet.png"));
            path.to_string_lossy().into_owned()
        }
    };
    if Path::new(&path).extension().and_then(|e| e.to_str()).is_none_or(|e| !e.eq_ignore_ascii_case("png")) {
        return Err(bad(CMD, "`path` must end in .png"));
    }
    let provider = s.media.full_res_provider(s.project.clone(), s.services.clone());
    let _ = cfa4_media::pending::take();
    let mut thumbs = Vec::with_capacity(frames);
    let mut shown = Vec::with_capacity(frames);
    for i in 0..frames {
        // the middle of each of `frames` equal parts, on a frame boundary
        let mid = Tick(((duration.0 as i128 * (2 * i as i128 + 1)) / (2 * frames as i128)) as i64);
        let last = rate.frame_at(duration - Tick(1)).max(0);
        let frame = rate.frame_at(mid).clamp(0, last);
        let time = rate.tick_of(frame);
        let img = if is_sequence {
            let (w, h) = s.project.sequence(item).map(|q| (q.settings.width, q.settings.height)).unwrap_or((1920, 1080));
            let scale = thumb as f32 / w.max(h).max(1) as f32;
            Some(cfa4_render::render_sequence(&s.project, item, time, RenderOptions { scale: scale.min(1.0), captions: true, ..Default::default() }, &provider))
        } else {
            cfa4_render::render_item(&s.project, item, time, 1.0, &provider)
        };
        let img = img.ok_or_else(|| bad(CMD, format!("frame {frame} cannot be decoded")))?;
        let (px, w, h) = downscale(&img.to_rgba8(), img.w, img.h, thumb);
        thumbs.push((u64::try_from(frame).unwrap_or(0), Some(time.seconds() * 1000.0), px, w, h));
        shown.push(json!({"frame": frame, "time": time.0}));
    }
    if cfa4_media::pending::take() {
        return Err(EngineError::Other("a frame is still loading; try again in a moment".into()));
    }
    let sheet = cfa4_qc::picture::compose_columns(&thumbs, columns).map_err(|e| EngineError::Other(format!("contact sheet: {e}")))?;
    s.services.write_file(&path, &sheet.png).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    Ok(json!({"path": path, "width": sheet.width, "height": sheet.height, "columns": sheet.columns, "frames": shown}))
}

/// `rgba` (w x h) shrunk, nearest neighbour, so its longer side is at most `max` (never enlarged).
fn downscale(rgba: &[u8], w: usize, h: usize, max: usize) -> (Vec<u8>, usize, usize) {
    if w == 0 || h == 0 || rgba.len() != w * h * 4 {
        return (vec![0, 0, 0, 255], 1, 1);
    }
    let long = w.max(h);
    if long <= max {
        return (rgba.to_vec(), w, h);
    }
    let (tw, th) = ((w * max / long).max(1), (h * max / long).max(1));
    let mut out = Vec::with_capacity(tw * th * 4);
    for y in 0..th {
        for x in 0..tw {
            let o = ((y * h / th) * w + x * w / tw) * 4;
            out.extend_from_slice(rgba.get(o..o + 4).unwrap_or(&[0, 0, 0, 255]));
        }
    }
    (out, tw, th)
}

pub fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Export Contact Sheet",
        menu: &[],
        shortcut: None,
        params: r#"{"sequence":id?|"item":id?,"frames":n=12 (1-64),"columns":n=4 (1-16),"thumb":px=160 (16-640),"path":str.png?}"#,
        enabled: always,
        run: contact_sheet,
        journal: false,
    }]
}
