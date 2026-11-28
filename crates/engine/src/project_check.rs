//! File ▸ Check Project…: a read-only health report on the open project (plan KDN-23).
//!
//! It works from the project model, never from a project file, and changes nothing, so it is safe
//! to run at any time and is also the answer the control channel gives to `project.check`.
//!
//! | section | kind | what is found |
//! |---|---|---|
//! | `missing` | `missing` / `offline` / `proxyMissing` | media whose file is gone, made offline, or only its proxy is gone (same list as `media.findMissing`) |
//! | `integrity` | `danglingClip` | a timeline clip whose project item no longer exists |
//! | | `unknownEffect` | an effect on a clip that this build has no definition for (it is skipped when rendering) |
//! | | `pastMediaEnd` | a clip that plays past the end of its source media |
//! | | `unusedMedia` | a media item no sequence uses |
//! | `performance` | `noProxy` | media of 4K or more, used on a timeline, without a proxy |
//! | | `heavyClip` | a clip with more than `maxEffects` (default 8) enabled effects |
//!
//! The result also carries per-sequence statistics (`sequences`: clips, enabled effects, video and
//! audio tracks) and `counts` per kind. `ok` is true when there is no `missing` or `integrity`
//! finding; performance notes are advice only.

use std::collections::BTreeSet;

use cfa4_media::MediaKind;
use cfa4_project::{ItemId, ItemKind, MediaRef};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, u64_p};
use crate::{Result, Session};

const UHD_WIDTH: u32 = 3840;
const UHD_HEIGHT: u32 = 2160;

struct Finding {
    section: &'static str,
    kind: &'static str,
    item: Option<u64>,
    clip: Option<u64>,
    sequence: Option<u64>,
    message: String,
}

impl Finding {
    fn json(&self) -> Value {
        json!({
            "section": self.section, "kind": self.kind, "item": self.item,
            "clip": self.clip, "sequence": self.sequence, "message": self.message,
        })
    }
}

fn check(s: &mut Session, p: &Value) -> Result<Value> {
    let max_effects = u64_p(p, "maxEffects").unwrap_or(8).min(1000) as usize;
    let mut found: Vec<Finding> = Vec::new();

    for m in crate::relink::scan(&s.project, &s.media, &*s.services) {
        let message = match m.status {
            "offline" => format!("{} was made offline", m.name),
            "proxyMissing" => format!("the proxy of {} is missing", m.name),
            _ => format!("{} is missing ({})", m.name, m.path),
        };
        found.push(Finding {
            section: "missing",
            kind: match m.status {
                "offline" => "offline",
                "proxyMissing" => "proxyMissing",
                _ => "missing",
            },
            item: Some(m.item),
            clip: None,
            sequence: None,
            message,
        });
    }

    let mut used: BTreeSet<ItemId> = BTreeSet::new();
    let mut stats = Vec::new();
    for seq in s.project.sequences() {
        let ItemKind::Sequence(q) = &seq.kind else { continue };
        let frame = q.settings.frame_rate.frame_duration();
        let (mut clips, mut effects) = (0usize, 0usize);
        for t in q.all_tracks() {
            for it in &t.items {
                clips += 1;
                let enabled = it.effects.iter().filter(|e| e.enabled).count();
                effects += enabled;
                let Some(pi) = s.project.item(it.item) else {
                    found.push(Finding {
                        section: "integrity",
                        kind: "danglingClip",
                        item: Some(it.item.0),
                        clip: Some(it.id.0),
                        sequence: Some(seq.id.0),
                        message: format!("clip {} points to a project item that does not exist", it.name),
                    });
                    continue;
                };
                used.insert(it.item);
                if let ItemKind::Subclip { parent, .. } = &pi.kind {
                    used.insert(*parent);
                }
                for e in it.effects.iter().filter(|e| e.def().is_none()) {
                    found.push(Finding {
                        section: "integrity",
                        kind: "unknownEffect",
                        item: Some(it.item.0),
                        clip: Some(it.id.0),
                        sequence: Some(seq.id.0),
                        message: format!("clip {} uses the unknown effect `{}`", it.name, e.effect),
                    });
                }
                if enabled > max_effects {
                    found.push(Finding {
                        section: "performance",
                        kind: "heavyClip",
                        item: Some(it.item.0),
                        clip: Some(it.id.0),
                        sequence: Some(seq.id.0),
                        message: format!("clip {} has {enabled} enabled effects", it.name),
                    });
                }
                if let Some((root, m, _)) = s.project.resolve_media(it.item) {
                    used.insert(root);
                    let finite = matches!(m.info.kind, MediaKind::Movie | MediaKind::AudioOnly);
                    let speed = it.speed.abs().max(0.0001);
                    let need = it.source_in.0 as f64 + it.duration.0 as f64 * speed;
                    if finite && m.info.duration.0 > 0 && need > m.info.duration.0 as f64 + frame.0 as f64 {
                        found.push(Finding {
                            section: "integrity",
                            kind: "pastMediaEnd",
                            item: Some(root.0),
                            clip: Some(it.id.0),
                            sequence: Some(seq.id.0),
                            message: format!("clip {} plays past the end of its media", it.name),
                        });
                    }
                    let big = m.info.video.as_ref().is_some_and(|v| v.width >= UHD_WIDTH || v.height >= UHD_HEIGHT);
                    if big && m.proxy.is_none() && matches!(m.media, MediaRef::File { .. }) {
                        let dup = found.iter().any(|f| f.kind == "noProxy" && f.item == Some(root.0));
                        if !dup {
                            found.push(Finding {
                                section: "performance",
                                kind: "noProxy",
                                item: Some(root.0),
                                clip: None,
                                sequence: Some(seq.id.0),
                                message: format!("{} is 4K or larger and has no proxy", pi.name),
                            });
                        }
                    }
                }
            }
        }
        stats.push(json!({
            "sequence": seq.id.0, "name": seq.name, "clips": clips, "effects": effects,
            "videoTracks": q.video_tracks.len(), "audioTracks": q.audio_tracks.len(),
        }));
    }

    for it in s.project.items.values() {
        if matches!(it.kind, ItemKind::Media(_)) && !used.contains(&it.id) {
            found.push(Finding {
                section: "integrity",
                kind: "unusedMedia",
                item: Some(it.id.0),
                clip: None,
                sequence: None,
                message: format!("{} is not used in any sequence", it.name),
            });
        }
    }

    let mut counts = serde_json::Map::new();
    for f in &found {
        let n = counts.get(f.kind).and_then(Value::as_u64).unwrap_or(0);
        counts.insert(f.kind.into(), json!(n + 1));
    }
    // unused media is tidy-up advice, not damage
    let ok = !found.iter().any(|f| f.section == "missing" || (f.section == "integrity" && f.kind != "unusedMedia"));
    Ok(json!({
        "ok": ok,
        "count": found.len(),
        "counts": counts,
        "findings": found.iter().map(Finding::json).collect::<Vec<_>>(),
        "sequences": stats,
    }))
}

pub fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "project.check",
        label: "Check Project…",
        menu: &["File"],
        shortcut: None,
        params: r#"{"maxEffects":n=8}"#,
        enabled: always,
        run: check,
        journal: false,
    }]
}
