//! The live studio's scenes and sources (CFA4-LIVE-STUDIO-001 phase S1).
//!
//! The scenes live in the project (`Project::studio`, plan §I.4): every command here that changes
//! them is one undoable project edit, saved with the project. The engine's front end
//! (`cfa4-cli serve`) sends the current scene to the studio host after every change
//! ([`program_layers`] renders its pictures with CFA4's own code).
//!
//! | command | params | what |
//! |---|---|---|
//! | `live.studio.scenes` | – | every scene with its sources, and the current one |
//! | `live.studio.scene.add` | `{name?}` | a new empty scene (the current one when there was none) |
//! | `live.studio.scene.remove` | `{id}` | removes a scene and its sources |
//! | `live.studio.scene.rename` | `{id, name}` | |
//! | `live.studio.scene.select` | `{id}` | puts the scene on the program |
//! | `live.studio.scene.move` | `{id, index}` | its place in the scene list |
//! | `live.studio.source.add` | `{scene?, type, name?, …kind fields, transform?, visible?}` | a source on top of the scene (default: the current one) |
//! | `live.studio.source.set` | `{id, name?, visible?, transform?, …kind fields, merge?}` | changes a source (`merge`: calls with the same key share one undo step, as a drag does) |
//! | `live.studio.source.remove` | `{id}` | |
//! | `live.studio.source.move` | `{id, index}` | its place in the stack (0: bottom) |
//!
//! Kind fields: `color` `{color: "#RRGGBB[AA]" \| [r,g,b,a], width, height}`, `image` `{path}`,
//! `text` `{text, font?, size?, color?}`. A transform is `{x, y, scaleX, scaleY, rotation,
//! crop: [l, t, r, b]}`; any field may be left out (it keeps its value).

use std::sync::Arc;

use cfa4_project::studio::{self as model, LiveStudio, SourceKind, SourceTransform, StudioScene, StudioSource};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, str_p, u64_p};
use crate::{Result, Session};

/// The default canvas of a new colour source (the studio's default video).
const DEFAULT_COLOR_SIZE: (u32, u32) = (1920, 1080);
/// Largest picture file read for an image source.
const MAX_PICTURE_BYTES: u64 = 512 << 20;

fn studio_json(st: &LiveStudio) -> Value {
    json!({
        "current": st.current,
        "scenes": st.scenes.iter().map(|sc| scene_json(sc, st.current == Some(sc.id))).collect::<Vec<_>>(),
    })
}

fn scene_json(sc: &StudioScene, current: bool) -> Value {
    json!({"id": sc.id, "name": sc.name, "current": current, "sources": sc.sources.iter().map(source_json).collect::<Vec<_>>()})
}

fn source_json(src: &StudioSource) -> Value {
    let mut v = serde_json::to_value(src).unwrap_or_else(|_| json!({}));
    if let Some(o) = v.as_object_mut() {
        o.insert("type".into(), json!(src.kind.type_name()));
    }
    v
}

/// A colour: `"#RRGGBB"`, `"#RRGGBBAA"` or `[r, g, b]` / `[r, g, b, a]` (0–255).
fn color_p(cmd: &str, v: &Value) -> Result<[u8; 4]> {
    let err = || bad(cmd, format!("a colour is \"#RRGGBB\", \"#RRGGBBAA\" or [r, g, b, a] (0-255), not {v}"));
    match v {
        Value::String(s) => {
            let hex = s.strip_prefix('#').unwrap_or(s);
            if !(hex.len() == 6 || hex.len() == 8) || !hex.is_ascii() {
                return Err(err());
            }
            let byte = |i: usize| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).ok_or_else(err);
            Ok([byte(0)?, byte(2)?, byte(4)?, if hex.len() == 8 { byte(6)? } else { 255 }])
        }
        Value::Array(a) if a.len() == 3 || a.len() == 4 => {
            let mut c = [255u8; 4];
            for (i, x) in a.iter().enumerate() {
                c[i] = x.as_u64().and_then(|n| u8::try_from(n).ok()).ok_or_else(err)?;
            }
            Ok(c)
        }
        _ => Err(err()),
    }
}

fn side_p(cmd: &str, p: &Value, key: &str) -> Result<Option<u32>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| (1..=model::MAX_SOURCE_SIDE).contains(n))
            .map(Some)
            .ok_or_else(|| bad(cmd, format!("`{key}` must be a whole number of pixels, 1 to {}", model::MAX_SOURCE_SIDE))),
    }
}

fn name_p(cmd: &str, p: &Value) -> Result<Option<String>> {
    match p.get("name") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let s = s.trim();
            if s.chars().count() > model::MAX_NAME_CHARS {
                return Err(bad(cmd, format!("a name is at most {} characters", model::MAX_NAME_CHARS)));
            }
            Ok(Some(s.to_string()))
        }
        Some(_) => Err(bad(cmd, "`name` must be a string")),
    }
}

/// Applies the kind fields of `p` to `kind` (the type stays).
fn apply_kind(cmd: &str, kind: &mut SourceKind, p: &Value) -> Result<()> {
    match kind {
        SourceKind::Color { color, width, height } => {
            if let Some(c) = p.get("color").filter(|v| !v.is_null()) {
                *color = color_p(cmd, c)?;
            }
            if let Some(w) = side_p(cmd, p, "width")? {
                *width = w;
            }
            if let Some(h) = side_p(cmd, p, "height")? {
                *height = h;
            }
        }
        SourceKind::Image { path } => {
            if let Some(v) = p.get("path").filter(|v| !v.is_null()) {
                let s = v.as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| bad(cmd, "`path` must name a picture file"))?;
                if s.len() > model::MAX_PATH_BYTES || s.contains('\0') {
                    return Err(bad(cmd, "the picture path is too long or not a path"));
                }
                *path = s.to_string();
            }
        }
        SourceKind::Text { text, font, size, color } => {
            if let Some(v) = p.get("text").filter(|v| !v.is_null()) {
                let s = v.as_str().ok_or_else(|| bad(cmd, "`text` must be a string"))?;
                if s.chars().count() > model::MAX_TEXT_CHARS {
                    return Err(bad(cmd, format!("a text is at most {} characters", model::MAX_TEXT_CHARS)));
                }
                *text = s.to_string();
            }
            if let Some(v) = p.get("font").filter(|v| !v.is_null()) {
                let s = v.as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| bad(cmd, "`font` must name a font family"))?;
                *font = s.chars().take(model::MAX_NAME_CHARS).collect();
            }
            if let Some(v) = p.get("size").filter(|v| !v.is_null()) {
                *size = v
                    .as_f64()
                    .filter(|x| x.is_finite() && (model::MIN_TEXT_SIZE..=model::MAX_TEXT_SIZE).contains(x))
                    .ok_or_else(|| bad(cmd, format!("`size` must be {} to {} pixels", model::MIN_TEXT_SIZE, model::MAX_TEXT_SIZE)))?;
            }
            if let Some(c) = p.get("color").filter(|v| !v.is_null()) {
                *color = color_p(cmd, c)?;
            }
        }
    }
    Ok(())
}

/// Applies `p.transform` (any subset of its fields) to `t`.
fn apply_transform(cmd: &str, t: &mut SourceTransform, p: &Value) -> Result<()> {
    let Some(tp) = p.get("transform").filter(|v| !v.is_null()) else { return Ok(()) };
    if !tp.is_object() {
        return Err(bad(cmd, "`transform` must be an object {x, y, scaleX, scaleY, rotation, crop}"));
    }
    let num = |key: &str, max: f64| -> Result<Option<f64>> {
        match tp.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_f64()
                .filter(|x| x.is_finite() && x.abs() <= max)
                .map(Some)
                .ok_or_else(|| bad(cmd, format!("transform `{key}` must be a number within ±{max}"))),
        }
    };
    let mut n = *t;
    if let Some(v) = num("x", model::MAX_POSITION)? {
        n.x = v;
    }
    if let Some(v) = num("y", model::MAX_POSITION)? {
        n.y = v;
    }
    if let Some(v) = num("scaleX", model::MAX_SCALE)? {
        n.scale_x = v;
    }
    if let Some(v) = num("scaleY", model::MAX_SCALE)? {
        n.scale_y = v;
    }
    if let Some(v) = num("rotation", 1e6)? {
        n.rotation = v;
    }
    match tp.get("crop") {
        None | Some(Value::Null) => {}
        Some(Value::Array(a)) if a.len() == 4 => {
            for (i, x) in a.iter().enumerate() {
                n.crop[i] = x
                    .as_u64()
                    .and_then(|v| u32::try_from(v).ok())
                    .filter(|v| *v <= model::MAX_SOURCE_SIDE)
                    .ok_or_else(|| bad(cmd, format!("transform `crop` is four whole numbers of pixels, 0 to {}", model::MAX_SOURCE_SIDE)))?;
            }
        }
        Some(_) => return Err(bad(cmd, "transform `crop` is [left, top, right, bottom]")),
    }
    *t = n.checked().ok_or_else(|| bad(cmd, "the transform is not finite"))?;
    Ok(())
}

fn scene_id_p(s: &Session, cmd: &str, p: &Value) -> Result<u64> {
    let id = u64_p(p, "id").ok_or_else(|| bad(cmd, "name the scene (`id`)"))?;
    s.project.studio.scene(id).map(|sc| sc.id).ok_or_else(|| bad(cmd, format!("there is no scene {id}")))
}

fn source_id_p(s: &Session, cmd: &str, p: &Value) -> Result<(u64, usize, u64)> {
    let id = u64_p(p, "id").ok_or_else(|| bad(cmd, "name the source (`id`)"))?;
    let (scene, index) = s.project.studio.find_source(id).ok_or_else(|| bad(cmd, format!("there is no source {id}")))?;
    Ok((scene, index, id))
}

fn index_p(cmd: &str, p: &Value, len: usize) -> Result<usize> {
    let i = u64_p(p, "index").ok_or_else(|| bad(cmd, "give the new place (`index`, from 0)"))?;
    usize::try_from(i).ok().filter(|i| *i < len).ok_or_else(|| bad(cmd, format!("`index` must be 0 to {}", len.saturating_sub(1))))
}

// ------------------------------------------------------------------------------------- commands

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(studio_json(&s.project.studio))
}

fn scene_add(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "live.studio.scene.add";
    if s.project.studio.scenes.len() >= model::MAX_SCENES {
        return Err(bad(CMD, format!("at most {} scenes", model::MAX_SCENES)));
    }
    let name = name_p(CMD, p)?.filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Scene {}", s.project.studio.scenes.len() + 1));
    let id = s.edit("Add Studio Scene", |pr, _| {
        let id = pr.alloc_id();
        pr.studio.scenes.push(StudioScene { id, name, sources: Vec::new() });
        if pr.studio.current.is_none() {
            pr.studio.current = Some(id);
        }
        Ok(id)
    })?;
    let st = &s.project.studio;
    Ok(st.scene(id).map(|sc| scene_json(sc, st.current == Some(id))).unwrap_or(Value::Null))
}

fn scene_remove(s: &mut Session, p: &Value) -> Result<Value> {
    let id = scene_id_p(s, "live.studio.scene.remove", p)?;
    s.edit("Remove Studio Scene", |pr, _| {
        let st = &mut pr.studio;
        let i = st.scenes.iter().position(|sc| sc.id == id).unwrap_or(0);
        st.scenes.retain(|sc| sc.id != id);
        if st.current == Some(id) {
            st.current = st.scenes.get(i).or(st.scenes.last()).map(|sc| sc.id);
        }
        Ok(())
    })?;
    Ok(json!({"removed": id, "current": s.project.studio.current}))
}

fn scene_rename(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "live.studio.scene.rename";
    let id = scene_id_p(s, CMD, p)?;
    let name = name_p(CMD, p)?.filter(|n| !n.is_empty()).ok_or_else(|| bad(CMD, "give the new `name`"))?;
    s.edit("Rename Studio Scene", |pr, _| {
        if let Some(sc) = pr.studio.scene_mut(id) {
            sc.name = name;
        }
        Ok(())
    })?;
    let st = &s.project.studio;
    Ok(st.scene(id).map(|sc| scene_json(sc, st.current == Some(id))).unwrap_or(Value::Null))
}

fn scene_select(s: &mut Session, p: &Value) -> Result<Value> {
    let id = scene_id_p(s, "live.studio.scene.select", p)?;
    if s.project.studio.current != Some(id) {
        s.edit("Select Studio Scene", |pr, _| {
            pr.studio.current = Some(id);
            Ok(())
        })?;
    }
    Ok(json!({"current": id}))
}

fn scene_move(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "live.studio.scene.move";
    let id = scene_id_p(s, CMD, p)?;
    let to = index_p(CMD, p, s.project.studio.scenes.len())?;
    s.edit("Move Studio Scene", |pr, _| {
        let scenes = &mut pr.studio.scenes;
        if let Some(from) = scenes.iter().position(|sc| sc.id == id) {
            let sc = scenes.remove(from);
            scenes.insert(to.min(scenes.len()), sc);
        }
        Ok(())
    })?;
    Ok(json!({"order": s.project.studio.scenes.iter().map(|sc| sc.id).collect::<Vec<_>>()}))
}

fn source_add(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "live.studio.source.add";
    let scene = match u64_p(p, "scene") {
        Some(id) => s.project.studio.scene(id).map(|sc| sc.id).ok_or_else(|| bad(CMD, format!("there is no scene {id}")))?,
        None => s.project.studio.current.ok_or_else(|| bad(CMD, "there is no scene yet: add one (live.studio.scene.add) or name it (`scene`)"))?,
    };
    if s.project.studio.scene(scene).is_some_and(|sc| sc.sources.len() >= model::MAX_SOURCES) {
        return Err(bad(CMD, format!("at most {} sources in a scene", model::MAX_SOURCES)));
    }
    let mut kind = match str_p(p, "type") {
        Some("color") => SourceKind::Color { color: [255, 255, 255, 255], width: DEFAULT_COLOR_SIZE.0, height: DEFAULT_COLOR_SIZE.1 },
        Some("image") => {
            if str_p(p, "path").is_none_or(|x| x.trim().is_empty()) {
                return Err(bad(CMD, "an image source needs the picture file (`path`)"));
            }
            SourceKind::Image { path: String::new() }
        }
        Some("text") => SourceKind::text(""),
        Some(other) => return Err(bad(CMD, format!("unknown source type `{other}` (color, image or text)"))),
        None => return Err(bad(CMD, "give the source `type`: color, image or text")),
    };
    apply_kind(CMD, &mut kind, p)?;
    let mut transform = SourceTransform::default();
    apply_transform(CMD, &mut transform, p)?;
    let visible = p.get("visible").and_then(Value::as_bool).unwrap_or(true);
    let name = name_p(CMD, p)?.filter(|n| !n.is_empty()).unwrap_or_else(|| match &kind {
        SourceKind::Color { .. } => "Color".into(),
        SourceKind::Image { path } => std::path::Path::new(path).file_name().map_or_else(|| "Image".into(), |f| f.to_string_lossy().into_owned()),
        SourceKind::Text { text, .. } => {
            let first: String = text.lines().next().unwrap_or("").chars().take(40).collect();
            if first.trim().is_empty() { "Text".into() } else { first }
        }
    });
    let id = s.edit("Add Studio Source", |pr, _| {
        let id = pr.alloc_id();
        if let Some(sc) = pr.studio.scene_mut(scene) {
            sc.sources.push(StudioSource { id, name, kind, transform, visible });
        }
        Ok(id)
    })?;
    let (sc, i) = s.project.studio.find_source(id).unwrap_or((scene, 0));
    let out = s.project.studio.scene(sc).and_then(|x| x.sources.get(i)).map(source_json).unwrap_or(Value::Null);
    Ok(json!({"scene": scene, "source": out}))
}

fn source_set(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "live.studio.source.set";
    let (scene, index, id) = source_id_p(s, CMD, p)?;
    let Some(old) = s.project.studio.scene(scene).and_then(|sc| sc.sources.get(index)).cloned() else {
        return Err(bad(CMD, format!("there is no source {id}")));
    };
    let mut new = old.clone();
    if let Some(name) = name_p(CMD, p)?.filter(|n| !n.is_empty()) {
        new.name = name;
    }
    if let Some(v) = p.get("visible").filter(|v| !v.is_null()) {
        new.visible = v.as_bool().ok_or_else(|| bad(CMD, "`visible` must be true or false"))?;
    }
    if let Some(t) = str_p(p, "type")
        && t != new.kind.type_name()
    {
        return Err(bad(CMD, format!("a {} source cannot become a {t} source; add a new one", new.kind.type_name())));
    }
    apply_kind(CMD, &mut new.kind, p)?;
    apply_transform(CMD, &mut new.transform, p)?;
    let changed = new != old;
    if changed {
        let body = |pr: &mut cfa4_project::Project, _: &mut crate::EditorState| {
            if let Some(slot) = pr.studio.scene_mut(scene).and_then(|sc| sc.sources.get_mut(index)) {
                *slot = new;
            }
            Ok(())
        };
        match str_p(p, "merge") {
            Some(key) => s.edit_merged("Change Studio Source", &format!("live.studio.source:{id}:{key}"), body)?,
            None => s.edit("Change Studio Source", body)?,
        }
    }
    let out = s.project.studio.scene(scene).and_then(|sc| sc.sources.get(index)).map(source_json).unwrap_or(Value::Null);
    Ok(json!({"source": out, "changed": changed}))
}

fn source_remove(s: &mut Session, p: &Value) -> Result<Value> {
    let (scene, index, id) = source_id_p(s, "live.studio.source.remove", p)?;
    s.edit("Remove Studio Source", |pr, _| {
        if let Some(sc) = pr.studio.scene_mut(scene)
            && index < sc.sources.len()
        {
            sc.sources.remove(index);
        }
        Ok(())
    })?;
    Ok(json!({"removed": id, "scene": scene}))
}

fn source_move(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "live.studio.source.move";
    let (scene, from, _) = source_id_p(s, CMD, p)?;
    let len = s.project.studio.scene(scene).map_or(0, |sc| sc.sources.len());
    let to = index_p(CMD, p, len)?;
    if to != from {
        s.edit("Move Studio Source", |pr, _| {
            if let Some(sc) = pr.studio.scene_mut(scene)
                && from < sc.sources.len()
            {
                let src = sc.sources.remove(from);
                sc.sources.insert(to.min(sc.sources.len()), src);
            }
            Ok(())
        })?;
    }
    let order: Vec<u64> = s.project.studio.scene(scene).map(|sc| sc.sources.iter().map(|x| x.id).collect()).unwrap_or_default();
    Ok(json!({"scene": scene, "order": order}))
}

fn has_scenes(s: &Session) -> std::result::Result<(), String> {
    if s.project.studio.scenes.is_empty() { Err("the live studio has no scenes".into()) } else { Ok(()) }
}

fn has_sources(s: &Session) -> std::result::Result<(), String> {
    if s.project.studio.scenes.iter().any(|sc| !sc.sources.is_empty()) { Ok(()) } else { Err("the live studio has no sources".into()) }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    let c = |id, label, params, enabled, run| CommandSpec { id, label, menu: &[], shortcut: None, params, enabled, run, journal: true };
    vec![
        CommandSpec {
            id: "live.studio.scenes",
            label: "List Studio Scenes",
            menu: &[],
            shortcut: None,
            params: "{} → {current, scenes:[{id, name, current, sources:[{id, name, type, kind, transform, visible}]}]}",
            enabled: always,
            run: list,
            journal: false,
        },
        c("live.studio.scene.add", "Add Studio Scene", r#"{"name":string?}"#, always, scene_add),
        c("live.studio.scene.remove", "Remove Studio Scene", r#"{"id":scene}"#, has_scenes, scene_remove),
        c("live.studio.scene.rename", "Rename Studio Scene", r#"{"id":scene,"name":string}"#, has_scenes, scene_rename),
        c("live.studio.scene.select", "Select Studio Scene", r#"{"id":scene}"#, has_scenes, scene_select),
        c("live.studio.scene.move", "Move Studio Scene", r#"{"id":scene,"index":n}"#, has_scenes, scene_move),
        c(
            "live.studio.source.add",
            "Add Studio Source",
            r##"{"scene":id?,"type":"color"|"image"|"text","name":string?,"color":"#RRGGBB[AA]"|[r,g,b,a]?,"width":px?,"height":px?,"path":file?,"text":string?,"font":family?,"size":px?,"transform":{"x","y","scaleX","scaleY","rotation","crop":[l,t,r,b]}?,"visible":bool?}"##,
            has_scenes,
            source_add,
        ),
        c(
            "live.studio.source.set",
            "Change Studio Source",
            r#"{"id":source,"name":string?,"visible":bool?,"transform":{…}?,"color"?,"width"?,"height"?,"path"?,"text"?,"font"?,"size"?,"merge":key?}"#,
            has_sources,
            source_set,
        ),
        c("live.studio.source.remove", "Remove Studio Source", r#"{"id":source}"#, has_sources, source_remove),
        c("live.studio.source.move", "Move Studio Source", r#"{"id":source,"index":n}"#, has_sources, source_move),
    ]
}

// ------------------------------------------------------------------------------- program layers

/// The pixels of a layer for the studio host.
#[derive(Clone, Debug, PartialEq)]
pub enum LayerPicture {
    /// A solid colour of this size (the host makes the pixels).
    Color { rgba: [u8; 4], width: u32, height: u32 },
    /// Straight sRGB RGBA8 pixels, `width * height * 4` bytes.
    Pixels { width: u32, height: u32, rgba: Arc<Vec<u8>> },
}

/// A visible source of the current scene, rendered.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgramLayer {
    pub source: u64,
    pub picture: LayerPicture,
    pub transform: SourceTransform,
}

/// What a source needs to be drawn: the same key, the same picture. Image sources include the
/// file's size and modification time, so a changed file is read again.
pub fn picture_key(kind: &SourceKind) -> String {
    let mut key = serde_json::to_string(kind).unwrap_or_default();
    if let SourceKind::Image { path } = kind
        && let Ok(m) = std::fs::metadata(path)
    {
        let mtime = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
        key.push_str(&format!("|{}|{mtime}", m.len()));
    }
    key
}

/// Renders one source's picture with CFA4's own code: a picture file through CFA4's decoders, a
/// text through the title renderer (the same text layer as Graphics), cut to its ink.
pub fn render_source(kind: &SourceKind) -> std::result::Result<LayerPicture, String> {
    match kind {
        SourceKind::Color { color, width, height } => Ok(LayerPicture::Color { rgba: *color, width: *width, height: *height }),
        SourceKind::Image { path } => {
            let meta = std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?;
            if meta.len() > MAX_PICTURE_BYTES {
                return Err(format!("{path}: larger than {} MiB", MAX_PICTURE_BYTES >> 20));
            }
            let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
            let r = cfa4_media::still::decode_raster(path, &bytes).map_err(|e| format!("{path}: {e}"))?;
            if r.width == 0 || r.height == 0 || r.width > model::MAX_SOURCE_SIDE || r.height > model::MAX_SOURCE_SIDE {
                return Err(format!("{path}: picture size {}x{} is outside 1..={}", r.width, r.height, model::MAX_SOURCE_SIDE));
            }
            if r.rgba.len() != r.width as usize * r.height as usize * 4 {
                return Err(format!("{path}: the decoder returned a damaged picture"));
            }
            Ok(LayerPicture::Pixels { width: r.width, height: r.height, rgba: Arc::new(r.rgba) })
        }
        SourceKind::Text { text, font, size, color } => render_text(text, font, *size, *color),
    }
}

fn render_text(text: &str, font: &str, size: f64, color: [u8; 4]) -> std::result::Result<LayerPicture, String> {
    use cfa4_project::{Param, ParamValue};
    if text.trim().is_empty() {
        return Err("the text is empty".into());
    }
    let size = size.clamp(model::MIN_TEXT_SIZE, model::MAX_TEXT_SIZE);
    let lines = text.lines().count().max(1) as f64;
    let widest = text.lines().map(|l| l.chars().count()).max().unwrap_or(1).max(1) as f64;
    let pad = (size * 0.5).ceil();
    let cap = f64::from(model::MAX_SOURCE_SIDE);
    // generous: cut to the ink afterwards
    let w = (widest * size * 1.2 + 2.0 * pad).ceil().min(cap) as usize;
    let h = (lines * size * 1.6 + 2.0 * pad).ceil().min(cap) as usize;
    let mut layer = cfa4_project::graphic::new_text_layer(text, cfa4_geom::Vec2::new(pad, pad + size), size);
    let c = color.map(|v| f32::from(v) / 255.0);
    layer.params.insert("fill_color".into(), Param::new(ParamValue::Color(c)));
    layer.params.insert("font".into(), Param::new(ParamValue::Text(font.into())));
    let mut img = cfa4_render::image::Image::new(w, h);
    cfa4_render::graphic_clip::render_graphic_layers(&[layer], cfa4_time::Tick::ZERO, (w as u32, h as u32), &cfa4_geom::Affine::IDENTITY, &mut img);
    let rgba = img.to_rgba8();
    // the ink's bounds
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (i, px) in rgba.chunks_exact(4).enumerate() {
        if px[3] > 0 {
            let (x, y) = (i % w, i / w);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return Err("the text has no visible characters in this font".into());
    }
    let (cw, ch) = (x1 - x0, y1 - y0);
    let mut out = Vec::with_capacity(cw * ch * 4);
    for y in y0..y1 {
        out.extend_from_slice(&rgba[(y * w + x0) * 4..(y * w + x1) * 4]);
    }
    Ok(LayerPicture::Pixels { width: cw as u32, height: ch as u32, rgba: Arc::new(out) })
}

/// The visible sources of the current scene, bottom first, rendered by `render` (normally
/// [`render_source`]; the caller may cache by [`picture_key`]). A source that cannot be drawn
/// (a missing picture file…) is left out and reported with its id and the reason.
pub fn program_layers(
    studio: &LiveStudio,
    mut render: impl FnMut(&SourceKind) -> std::result::Result<LayerPicture, String>,
) -> (Vec<ProgramLayer>, Vec<(u64, String)>) {
    let mut layers = Vec::new();
    let mut problems = Vec::new();
    let Some(scene) = studio.current_scene() else { return (layers, problems) };
    for src in scene.sources.iter().filter(|x| x.visible) {
        match render(&src.kind) {
            Ok(picture) => layers.push(ProgramLayer { source: src.id, picture, transform: src.transform }),
            Err(e) => problems.push((src.id, e)),
        }
    }
    (layers, problems)
}

#[cfg(test)]
#[path = "studio_tests.rs"]
mod tests;
