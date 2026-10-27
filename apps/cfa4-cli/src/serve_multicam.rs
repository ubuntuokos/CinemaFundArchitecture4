//! The Qt GUI's Multi-Camera view and dialogs (`crates/ui-egui/src/panels/multicam.rs` in egui):
//! `multicam.frame` renders the angle grid page shown at a sequence time, `multicam.angleFrame`
//! one angle (Edit Cameras thumbnails), and `multicam.selection` lists the clips the Synchronize
//! / Merge Clips / Create Multi-Camera Source Sequence dialogs work on.

use cfa4_engine::Session;
use cfa4_engine::multicam;
use cfa4_project::ItemId;
use cfa4_time::Tick;
use serde_json::{Value, json};

/// Largest grid image edge asked for, as `frame.render`'s `maxWidth` / `maxHeight`.
const MAX_EDGE: f64 = 8192.0;

fn edge(p: &Value, k: &str) -> Result<Option<f64>, String> {
    match p.get(k) {
        None => Ok(None),
        Some(v) => {
            v.as_f64().filter(|x| x.is_finite() && (16.0..=MAX_EDGE).contains(x)).map(Some).ok_or_else(|| format!("`{k}` must be a number in 16..={MAX_EDGE}"))
        }
    }
}

fn scale_p(p: &Value, k: &str) -> Result<f32, String> {
    match p.get(k) {
        None => Ok(1.0),
        Some(v) => v.as_f64().filter(|x| x.is_finite() && *x > 0.0).map(|x| x.min(1.0) as f32).ok_or_else(|| format!("`{k}` must be a positive number")),
    }
}

/// `multicam.frame {"time", "maxWidth", "maxHeight", "scale"?, "playing"?}`: the grid page of the
/// multi-camera clip at `time` (the Program monitor's shown time), every cell at the size it is
/// shown (never above `scale`, the playback resolution; lower while playing with Auto-Adjust
/// Multi-Camera Playback Quality), with `multicam.grid`'s layout and cells. Without a
/// multi-camera clip there the reply has `clip: null` and no picture.
pub fn frame(s: &Session, p: &Value, t: Tick) -> Result<(Value, Vec<u8>), String> {
    let mut info = multicam::grid_at(s, t);
    let Some(src) = info["source"].as_u64().map(ItemId) else {
        return Ok((info, Vec::new()));
    };
    let q = s.project.sequence(src).ok_or("the multi-camera source is gone")?;
    let (sw, sh) = (f64::from(q.settings.width.max(1)), f64::from(q.settings.height.max(1)));
    let cols = info["cols"].as_u64().unwrap_or(1).max(1) as f64;
    let rows = info["rows"].as_u64().unwrap_or(1).max(1) as f64;
    // the cell's on-screen width in pixels: the grid fitted into maxWidth × maxHeight
    let mut cell_px = sw;
    if let Some(mw) = edge(p, "maxWidth")? {
        cell_px = cell_px.min(mw / cols);
    }
    if let Some(mh) = edge(p, "maxHeight")? {
        cell_px = cell_px.min(mh / rows * sw / sh);
    }
    let view = &s.state.multicam_view;
    let per_page = (cols * rows) as usize;
    let playing = p.get("playing").and_then(Value::as_bool).unwrap_or(false);
    let scale = cfa4_render::multicam::grid_cell_scale(q.settings.width, cell_px as f32, scale_p(p, "scale")?, playing, view.auto_quality, per_page);
    let mt = Tick(info["sourceTime"].as_i64().unwrap_or(0));
    let side = view.layout;
    let page = view.page;
    let provider = s.media.provider(s.project.clone(), s.services.clone());
    let (img, _) = cfa4_render::multicam::render_grid_page(&s.project, src, mt, scale, side, page, &provider).ok_or("the multi-camera source is gone")?;
    info["w"] = json!(img.w);
    info["h"] = json!(img.h);
    info["cellScale"] = json!(scale);
    info["format"] = json!("rgba8");
    Ok((info, img.over_black_rgba8()))
}

/// `multicam.angleFrame {"sequence", "angle", "time"?, "maxWidth"}`: one angle of a multi-camera
/// source sequence at its own `time` (default: the source time at the playhead), for Edit
/// Cameras. An audio-only angle replies `video: false` without a picture.
pub fn angle_frame(s: &Session, p: &Value) -> Result<(Value, Vec<u8>), String> {
    let src = p.get("sequence").and_then(Value::as_u64).map(ItemId).ok_or("need `sequence`")?;
    let angle = p.get("angle").and_then(Value::as_u64).and_then(|a| usize::try_from(a).ok()).ok_or("need `angle`")?;
    let q = s.project.sequence(src).ok_or("no such sequence")?;
    if angle >= q.cameras().cameras.len() {
        return Err(format!("no angle {angle}"));
    }
    let t = match p.get("time") {
        Some(v) => Tick(v.as_i64().ok_or("`time` must be an integer tick count")?),
        None => Tick(multicam::inspect_at(s, s.playhead())["sourceTime"].as_i64().unwrap_or(0)),
    };
    if q.angle_video_track_index(angle).is_none() {
        return Ok((json!({"sequence": src.0, "angle": angle, "video": false}), Vec::new()));
    }
    let width = edge(p, "maxWidth")?.unwrap_or(96.0);
    let scale = (width / f64::from(q.settings.width.max(1))).clamp(1.0 / 32.0, 1.0) as f32;
    let provider = s.media.provider(s.project.clone(), s.services.clone());
    let img = cfa4_render::multicam::render_angle_thumbnail(&s.project, src, angle, t, scale, &provider).ok_or("no such sequence")?;
    Ok((json!({"sequence": src.0, "angle": angle, "video": true, "w": img.w, "h": img.h, "format": "rgba8"}), img.over_black_rgba8()))
}

/// `multicam.selection`: the clips Merge Clips and Create Multi-Camera Source Sequence would use
/// (the Project panel's selection, or the media of the clips selected in the Timeline), with
/// their names, and how many Timeline clips are selected (Synchronize).
pub fn selection(s: &Session) -> Value {
    let items: Vec<Value> = multicam::selected_items(s).iter().filter_map(|i| s.project.item(*i).map(|it| json!({"id": i.0, "name": it.name}))).collect();
    json!({"items": items, "selected": s.state.selection.len()})
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfa4_engine::multicam::multicam_clip_at;

    /// The first two video clips of the project (not sequences).
    fn video_clips(s: &Session) -> Vec<ItemId> {
        s.project.items.values().filter(|i| i.has_video() && s.project.sequence(i.id).is_none()).map(|i| i.id).take(2).collect()
    }

    /// Two camera clips stacked into a multi-camera source sequence, cut into a new sequence.
    fn multicam_session() -> (Session, u64) {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).expect("demo");
        let clips: Vec<u64> = video_clips(&s).iter().map(|i| i.0).collect();
        assert_eq!(clips.len(), 2, "the demo project has two video clips");
        s.execute("project.select", json!({"items": clips})).expect("select");
        let made = s.execute("clip.createMulticam", json!({"items": clips, "method": "in", "name": "Cams"})).expect("multicam");
        let src = made["sequence"].as_u64().expect("sequence id");
        // a sequence cut from it: the multi-camera source overwritten at 0
        s.execute("source.open", json!({"item": src})).expect("open");
        let at = s.playhead();
        s.execute("source.overwrite", json!({})).expect("overwrite");
        s.set_playhead(at); // the overwrite moved it past the clip
        (s, src)
    }

    #[test]
    fn the_grid_frame_tiles_the_angles_at_the_cell_size() {
        let (s, src) = multicam_session();
        let at = s.playhead();
        assert!(multicam_clip_at(&s, at).is_some(), "the multi-camera clip is at the playhead");
        let (info, bytes) = frame(&s, &json!({"maxWidth": 640, "maxHeight": 360}), at).expect("grid");
        assert_eq!(info["source"], json!(src));
        assert_eq!((info["cols"].as_u64(), info["rows"].as_u64()), (Some(2), Some(1)), "two angles side by side");
        let (w, h) = (info["w"].as_u64().unwrap_or(0), info["h"].as_u64().unwrap_or(0));
        assert!(w > 0 && w <= 640 && h <= 360, "{w}×{h}");
        assert_eq!(bytes.len() as u64, w * h * 4);
        assert_eq!(info["cells"].as_array().map(Vec::len), Some(2));
        // no multi-camera clip there: no picture
        let (none, b) = frame(&s, &json!({}), Tick(i64::MAX / 2)).expect("empty");
        assert!(none["clip"].is_null() && b.is_empty());
        assert!(frame(&s, &json!({"maxWidth": 0}), at).is_err());
    }

    #[test]
    fn an_angle_thumbnail_and_the_dialog_selection() {
        let (mut s, src) = multicam_session();
        let (info, bytes) = angle_frame(&s, &json!({"sequence": src, "angle": 1, "maxWidth": 96})).expect("thumb");
        assert_eq!(info["video"], json!(true));
        assert_eq!(bytes.len() as u64, info["w"].as_u64().unwrap_or(0) * info["h"].as_u64().unwrap_or(0) * 4);
        assert!(angle_frame(&s, &json!({"sequence": src, "angle": 99})).is_err());
        assert!(angle_frame(&s, &json!({"sequence": 123_456, "angle": 0})).is_err());
        s.state.project_selection = video_clips(&s);
        let sel = selection(&s);
        assert_eq!(sel["items"].as_array().map(Vec::len), Some(2));
        assert!(sel["items"][0]["name"].is_string());
    }
}
