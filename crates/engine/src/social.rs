//! Social video delivery: platform safe zones, Auto Reframe Sequence and the Social
//! Deliverables export (one edit out as 16:9, 9:16, 1:1 and 4:5 files in one step).
//!
//! | command | menu | what it does |
//! |---|---|---|
//! | `sequence.autoReframe` | Sequence ▸ Auto Reframe Sequence… | copies a sequence at another aspect; every picture clip gets Auto Reframe |
//! | `export.socialDeliverables` | File ▸ Export ▸ Social Deliverables… | reframes the sequence to each asked aspect and queues one export per aspect (loudness normalised) |
//!
//! **Safe zones** ([`SafeZone`]) are the parts of a vertical frame that TikTok, Instagram Reels
//! and YouTube Shorts cover with their own buttons and captions. The insets are CFA4's own
//! conservative approximations of the platforms' published guidance, as fractions of a
//! 1080 × 1920 frame; the Program monitor draws them (View ▸ Social Safe Zones) and the delivery
//! check ([`crate::delivery_check`]) flags text inside them.
//!
//! The reframed copy keeps every clip, effect, keyframe and marker of the original; its clips get
//! new ids. Motion positions move with the frame's centre; pictures are scaled up to cover the
//! new frame and Auto Reframe pans them to the subject. Titles keep their size, so a title made
//! for 16:9 may need moving: the delivery check reports layers that leave the frame. The copy's
//! metadata records its source sequence and aspect, so Social Deliverables reuses only its own
//! copies.

use std::sync::Arc;

use cfa4_geom::Vec2;
use cfa4_project::{ClipId, ItemId, ItemKind, Label, ParamValue, Sequence, TrackKind};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, bool_p, f64_p, has_seq, str_p};
use crate::{EngineError, Result, Session};

/// A platform's safe zone in a vertical frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SafeZone {
    TikTok,
    Reels,
    Shorts,
}

impl SafeZone {
    pub const ALL: [SafeZone; 3] = [SafeZone::TikTok, SafeZone::Reels, SafeZone::Shorts];

    /// Control-channel / parameter name.
    pub fn id(self) -> &'static str {
        match self {
            SafeZone::TikTok => "tiktok",
            SafeZone::Reels => "reels",
            SafeZone::Shorts => "shorts",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SafeZone::TikTok => "TikTok",
            SafeZone::Reels => "Instagram Reels",
            SafeZone::Shorts => "YouTube Shorts",
        }
    }

    pub fn from_id(s: &str) -> Option<SafeZone> {
        SafeZone::ALL.into_iter().find(|z| z.id().eq_ignore_ascii_case(s))
    }

    /// Covered margins (top, right, bottom, left) as fractions of the frame's height / width.
    pub fn insets(self) -> [f64; 4] {
        // pixels of a 1080 × 1920 frame: top, right, bottom, left
        let px: [f64; 4] = match self {
            SafeZone::TikTok => [160.0, 150.0, 480.0, 60.0],
            SafeZone::Reels => [220.0, 120.0, 420.0, 60.0],
            SafeZone::Shorts => [180.0, 140.0, 400.0, 60.0],
        };
        [px[0] / 1920.0, px[1] / 1080.0, px[2] / 1920.0, px[3] / 1080.0]
    }

    /// The uncovered rectangle of a `w` × `h` frame: [x0, y0, x1, y1] in pixels.
    pub fn safe_rect(self, w: f64, h: f64) -> [f64; 4] {
        let [t, r, b, l] = self.insets();
        [l * w, t * h, w - r * w, h - b * h]
    }
}

/// The intersection of every platform's safe rectangle (text inside it is safe everywhere).
pub fn strictest_safe_rect(w: f64, h: f64) -> [f64; 4] {
    SafeZone::ALL.iter().fold([0.0, 0.0, w, h], |a, z| {
        let r = z.safe_rect(w, h);
        [a[0].max(r[0]), a[1].max(r[1]), a[2].min(r[2]), a[3].min(r[3])]
    })
}

/// Aspect names the commands accept and their ratio (width, height).
pub const ASPECTS: [(&str, u32, u32); 4] = [("16:9", 16, 9), ("9:16", 9, 16), ("1:1", 1, 1), ("4:5", 4, 5)];

fn aspect_p(v: &str) -> Option<(&'static str, u32, u32)> {
    let v = v.trim().replace(['x', 'X', '×', '/'], ":");
    ASPECTS.iter().copied().find(|(n, _, _)| *n == v)
}

/// Frame size of a reframed sequence: the source's short side stays, the long side follows the
/// aspect; both even (encoders want even sizes).
pub fn reframed_size(w: u32, h: u32, aw: u32, ah: u32) -> (u32, u32) {
    let short = w.min(h).max(2) as u64;
    let (aw, ah) = (aw.max(1) as u64, ah.max(1) as u64);
    let even = |x: u64| (x.min(16_384) / 2 * 2).max(2) as u32;
    if aw >= ah { (even(short * aw / ah), even(short)) } else { (even(short), even(short * ah / aw)) }
}

/// The aspect name of a frame size, when it is one of [`ASPECTS`] (within 1 %).
pub fn aspect_of(w: u32, h: u32) -> Option<&'static str> {
    let r = w as f64 / h.max(1) as f64;
    ASPECTS.iter().find(|(_, a, b)| (r / (*a as f64 / *b as f64) - 1.0).abs() < 0.01).map(|(n, _, _)| *n)
}

/// Whether a project item puts a camera picture on a video track (what Auto Reframe works on).
fn is_picture(kind: &ItemKind) -> bool {
    match kind {
        ItemKind::Media(m) => m.info.has_video(),
        ItemKind::Subclip { .. } | ItemKind::Sequence(_) => true,
        ItemKind::AdjustmentLayer { .. } | ItemKind::Graphic { .. } => false,
    }
}

/// Copy sequence `seq` as a new project item named `name` (one undo step labelled `label`); `f`
/// changes the copy before it is added. Clips, transitions and captions of the copy get new ids
/// (ids are project-wide); everything else is kept. The copy is selected in the Project panel.
pub(crate) fn copy_sequence(
    s: &mut Session,
    seq: ItemId,
    name: &str,
    label: &str,
    f: impl FnOnce(&mut cfa4_project::Project, &mut Sequence) -> Result<()>,
) -> Result<ItemId> {
    let src = s.project.sequence(seq).ok_or(EngineError::NoSequence)?.clone();
    let color = s.project.item(seq).map(|i| i.label).unwrap_or(Label::Forest);
    s.edit(label, |p, st| {
        let mut q = src;
        let mut map = std::collections::HashMap::new();
        for t in q.all_tracks_mut() {
            for it in &mut t.items {
                let id = ClipId(p.alloc_id());
                map.insert(it.id, id);
                it.id = id;
            }
        }
        for t in q.all_tracks_mut() {
            for tr in &mut t.transitions {
                tr.from = tr.from.and_then(|c| map.get(&c).copied());
                tr.to = tr.to.and_then(|c| map.get(&c).copied());
            }
        }
        for ct in &mut q.caption_tracks {
            for c in &mut ct.captions {
                c.id = ClipId(p.alloc_id());
            }
        }
        f(p, &mut q)?;
        let id = p.add_item(name, color, ItemKind::Sequence(std::sync::Arc::new(q)), None);
        st.project_selection = vec![id];
        Ok(id)
    })
}

/// Copy sequence `seq` at aspect `(aw, ah)` (one undo step); returns the new sequence.
pub fn reframe_sequence(s: &mut Session, seq: ItemId, (name, aw, ah): (&str, u32, u32), new_name: Option<String>) -> Result<ItemId> {
    let src = s.project.sequence(seq).ok_or(EngineError::NoSequence)?;
    let old_name = s.project.item(seq).map(|i| i.name.clone()).unwrap_or_else(|| "Sequence".into());
    let def = cfa4_project::find_effect("auto_reframe").ok_or_else(|| EngineError::Other("Auto Reframe is not available".into()))?;
    let (w, h) = reframed_size(src.settings.width, src.settings.height, aw, ah);
    let new_name = new_name.unwrap_or_else(|| format!("{old_name} ({})", name.replace(':', "×")));
    let id = copy_sequence(s, seq, &new_name, "Auto Reframe Sequence", |p, q| {
        // Motion positions are in sequence pixels: move them with the frame's centre so every
        // clip keeps its place relative to the centre (keyframes included)
        let (ow, oh, seq_par) = (q.settings.width, q.settings.height, cfa4_project::sane_par(q.settings.par));
        let dx = (w as f64 - q.settings.width as f64) / 2.0;
        let dy = (h as f64 - q.settings.height as f64) / 2.0;
        // a picture that filled the old frame must cover the new one: Auto Reframe pans its
        // source-sized raster to the subject at the new aspect, and Motion scales it up by this
        // (plus a pixel, so the resampled edge rows are not half transparent)
        let cover = if (w, h) == (q.settings.width, q.settings.height) {
            1.0
        } else {
            (w as f64 / q.settings.width.max(1) as f64).max(h as f64 / q.settings.height.max(1) as f64).max(1.0) * (1.0 + 2.0 / w.min(h).max(1) as f64)
        };
        let aspect = match name {
            "9:16" => 1,
            "1:1" => 2,
            "4:5" => 3,
            _ => 4,
        };
        q.settings.width = w;
        q.settings.height = h;
        for t in q.video_tracks.iter_mut().filter(|t| t.kind == TrackKind::Video) {
            for it in &mut t.items {
                if let Some(pos) = it.effect_mut("motion").and_then(|m| m.params.get_mut("position")) {
                    pos.map_values(|v| match v {
                        ParamValue::Vec2(v) if v.x.is_finite() && v.y.is_finite() => ParamValue::Vec2(Vec2::new(v.x + dx, v.y + dy)),
                        v => v,
                    });
                }
                let picture = p.item(it.item).is_some_and(|x| is_picture(&x.kind));
                if !picture || it.effects.iter().any(|e| e.effect == "auto_reframe") {
                    continue;
                }
                // Scale to Frame Size refits the picture to the new frame itself: keep the old fit
                let mut cover = cover;
                if it.scale_to_frame
                    && let Some((sw, sh)) = cfa4_render::source_size(p, it.item)
                {
                    let r = cfa4_project::par_ratio(cfa4_project::sane_par(cfa4_render::source_par(p, it.item).unwrap_or(seq_par)))
                        / cfa4_project::par_ratio(seq_par);
                    let fit = |fw: f64, fh: f64| (fw / (sw.max(1) as f64 * r)).min(fh / sh.max(1) as f64);
                    let (new_fit, old_fit) = (fit(w as f64, h as f64), fit(ow as f64, oh as f64));
                    if new_fit > 0.0 && old_fit.is_finite() {
                        cover *= old_fit / new_fit;
                    }
                }
                if let Some(m) = it.effect_mut("motion") {
                    for k in ["scale", "scale_width"] {
                        if let Some(sc) = m.params.get_mut(k) {
                            sc.map_values(|v| match v {
                                ParamValue::Float(f) if f.is_finite() => ParamValue::Float(f * cover),
                                v => v,
                            });
                        }
                    }
                }
                let mut fx = def.instance();
                if let Some(a) = fx.params.get_mut("aspect") {
                    a.value = ParamValue::Choice(aspect);
                }
                let pos = it.effects.iter().position(|e| e.def().is_some_and(|d| d.intrinsic)).unwrap_or(it.effects.len());
                it.effects.insert(pos, fx);
            }
        }
        Ok(())
    })?;
    // where the copy came from, so Social Deliverables reuses only its own copies (part of the
    // step just made: undo removes the copy with it)
    if let Some(i) = Arc::make_mut(&mut s.project).items.get_mut(&id) {
        i.metadata.insert(REFRAMED_FROM.into(), seq.0.to_string());
        i.metadata.insert(REFRAMED_ASPECT.into(), name.to_string());
    }
    s.bump();
    Ok(id)
}

/// Metadata of a reframed copy: the source sequence's id…
const REFRAMED_FROM: &str = "Reframed From";
/// …and the aspect it was reframed to.
const REFRAMED_ASPECT: &str = "Reframed Aspect";

/// The reframed copy of `seq` at aspect `name` an earlier run made, if it still exists.
fn reframed_copy(s: &Session, seq: ItemId, name: &str) -> Option<ItemId> {
    let from = seq.0.to_string();
    s.project
        .items
        .values()
        .find(|i| {
            matches!(i.kind, ItemKind::Sequence(_))
                && i.metadata.get(REFRAMED_FROM) == Some(&from)
                && i.metadata.get(REFRAMED_ASPECT).map(String::as_str) == Some(name)
        })
        .map(|i| i.id)
}

fn auto_reframe(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "sequence.autoReframe";
    let seq = crate::export_tools::sequence_param(s, p, CMD)?;
    let a = str_p(p, "aspect").unwrap_or("9:16");
    let aspect = aspect_p(a).ok_or_else(|| bad(CMD, format!("unknown aspect `{a}`; use 16:9, 9:16, 1:1 or 4:5")))?;
    let id = reframe_sequence(s, seq, aspect, str_p(p, "name").map(str::to_string))?;
    if bool_p(p, "open").unwrap_or(true) {
        s.execute("sequence.open", json!({"item": id.0}))?;
    }
    let q = s.project.sequence(id).ok_or(EngineError::NoSequence)?;
    Ok(json!({"sequence": id.0, "width": q.settings.width, "height": q.settings.height}))
}

fn deliverables(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "export.socialDeliverables";
    let seq = crate::export_tools::sequence_param(s, p, CMD)?;
    let q = s.project.sequence(seq).ok_or(EngineError::NoSequence)?;
    let (w, h) = (q.settings.width, q.settings.height);
    let own = aspect_of(w, h);
    let asked: Vec<String> = match p.get("aspects").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        None => ASPECTS.iter().map(|(n, _, _)| n.to_string()).collect(),
    };
    if asked.is_empty() {
        return Err(bad(CMD, "no aspects"));
    }
    let mut aspects = Vec::new();
    for a in &asked {
        let x = aspect_p(a).ok_or_else(|| bad(CMD, format!("unknown aspect `{a}`; use 16:9, 9:16, 1:1 or 4:5")))?;
        if !aspects.contains(&x) {
            aspects.push(x);
        }
    }
    let lufs = match p.get("loudnessLufs") {
        Some(Value::Bool(false)) => None,
        _ => Some(f64_p(p, "loudnessLufs").unwrap_or(-14.0)),
    };
    if lufs.is_some_and(|l| !(-70.0..=0.0).contains(&l)) {
        return Err(bad(CMD, "loudnessLufs must be between -70 and 0"));
    }
    // the sequence itself stands for its own aspect; the others are reframed copies (reused
    // when an earlier run made them and they still exist)
    let base = s.project.item(seq).map(|i| i.name.clone()).unwrap_or_default();
    let mut seqs = Vec::new();
    let mut made = Vec::new();
    for x in &aspects {
        if Some(x.0) == own {
            seqs.push((x.0, seq));
            continue;
        }
        let name = format!("{base} ({})", x.0.replace(':', "×"));
        let existing = reframed_copy(s, seq, x.0);
        let id = match existing {
            Some(id) if bool_p(p, "reuse").unwrap_or(true) => id,
            _ => {
                let id = reframe_sequence(s, seq, *x, Some(name))?;
                made.push(id.0);
                id
            }
        };
        seqs.push((x.0, id));
    }
    let mut q = json!({"sequences": seqs.iter().map(|(_, i)| i.0).collect::<Vec<_>>()});
    for k in ["preset", "path", "settings", "start", "wait", "range"] {
        if let Some(v) = p.get(k) {
            q[k] = v.clone();
        }
    }
    if let Some(l) = lufs {
        q["loudnessLufs"] = json!(l);
    }
    let r = s.execute("export.queue.add", q)?;
    Ok(json!({
        "deliverables": seqs.iter().map(|(a, i)| json!({"aspect": a, "sequence": i.0})).collect::<Vec<_>>(),
        "reframed": made,
        "queued": r["added"].clone(),
        "loudnessLufs": lufs,
    }))
}

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "sequence.autoReframe",
            label: "Auto Reframe Sequence…",
            menu: &["Sequence"],
            shortcut: None,
            params: r#"{"sequence":id?,"aspect":"9:16|1:1|4:5|16:9"="9:16","name":str?,"open":bool=true}"#,
            enabled: has_seq,
            run: auto_reframe,
            journal: true,
        },
        CommandSpec {
            id: "export.socialDeliverables",
            label: "Social Deliverables…",
            menu: &["File", "Export"],
            shortcut: None,
            params: r#"{"sequence":id?,"aspects":["16:9"|"9:16"|"1:1"|"4:5"]?,"loudnessLufs":f64|false=-14,"preset":str?,"path":folder?,"reuse":bool=true,"start":bool=false,"wait":bool?}"#,
            enabled: has_seq,
            run: deliverables,
            journal: true,
        },
    ]
}

/// Agents: the safe-zone table (for the control channel and the delivery check).
pub fn safe_zones_json() -> Value {
    Value::Array(
        SafeZone::ALL
            .iter()
            .map(|z| {
                let [t, r, b, l] = z.insets();
                json!({"id": z.id(), "label": z.label(), "top": t, "right": r, "bottom": b, "left": l})
            })
            .collect(),
    )
}
