//! `dialog.defaults`: the starting parameters of a menu command's dialog (Paste / Remove
//! Attributes, New Offline File, Nest…, Make / Edit Subclip, Modify ▸ Audio Channels and
//! Timecode, Frame Hold Options, Field Options, Clip Speed / Duration), so every frontend opens
//! the same dialog egui does (`crates/ui-egui/src/panels/clip_dialogs.rs` `defaults`). `params`
//! is the JSON the command takes; `info` what the dialog shows besides (effects offered, frame
//! rate, channel count, the clip's duration).

use cfa4_project::{AudioChannelMap, AudioChannels, ItemKind, TrackKind};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, str_p};
use crate::{Result, Session};

/// The commands that have a dialog: egui's `clip_dialogs` (Edit / Clip / File) and
/// `menu_dialogs` (the menu long tail).
pub const DIALOGS: [&str; 27] = [
    "edit.pasteAttributes",
    "edit.removeAttributes",
    "file.newOfflineFile",
    "clip.nest",
    "clip.makeSubclip",
    "clip.editSubclip",
    "clip.audioChannels",
    "clip.modifyTimecode",
    "clip.frameHoldOptions",
    "clip.fieldOptions",
    "clip.speedDuration",
    "edit.find",
    "file.newSearchBin",
    "file.projectSettings.general",
    "file.projectSettings.scratchDisks",
    "project.ingestSettings",
    "file.mediaProperties",
    "clip.editOffline",
    "clip.sourceSettings",
    "clip.updateMetadata",
    "clip.automateToSequence",
    "clip.sceneEditDetection",
    "sequence.normalizeMixTrack",
    "sequence.simplify",
    "sequence.transcribe",
    "file.saveAsTemplate",
    "markers.addFlashCue",
];

/// Further menu dialogs (egui's `menu_dialogs`, Sequence ▸ Color Management… of `color_dialogs`,
/// Export Frame of `frame_export`, AAF / OMF of `interchange_export`) that run a command of their own.
const MORE: [&str; 6] = ["file.newColorMatte", "project.matteColor", "sequence.colorSettings", "file.exportFrame", "file.exportAaf", "file.exportOmf"];

/// The sample rates AAF / OMF offer (egui's `interchange_export::RATES`).
const INTERCHANGE_RATES: [u32; 4] = [32_000, 44_100, 48_000, 96_000];

/// The command a menu item's dialog runs: the Project Settings items share one.
fn dialog_command(id: &str) -> &str {
    match id {
        "file.projectSettings.general" | "file.projectSettings.scratchDisks" | "project.ingestSettings" => "project.settings",
        _ => id,
    }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "dialog.defaults",
        label: "Dialog Defaults",
        menu: &[],
        shortcut: None,
        params: r#"{"command":"clip.speedDuration|edit.find|file.projectSettings.general|…"(a menu command with a dialog),"timeline":bool? (Find: the Timeline has focus),"searchText":str? (New Search Bin: the Project panel's search)}"#,
        enabled: always,
        run: dialog_defaults,
        journal: false,
    }]
}

fn dialog_defaults(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "dialog.defaults";
    let id = str_p(p, "command").ok_or_else(|| bad(CMD, "`command` is required"))?;
    if !DIALOGS.contains(&id) && !MORE.contains(&id) {
        return Err(bad(CMD, format!("`{id}` has no dialog")));
    }
    // a command that can't run now opens no dialog (egui's)
    if let Some(c) = crate::find_command(id) {
        (c.enabled)(s).map_err(crate::EngineError::Other)?;
    }
    let (params, info) = match defaults(s, id) {
        Some(x) => x,
        None => menu_defaults(s, id, p)?,
    };
    Ok(json!({"command": dialog_command(id), "params": params, "info": info}))
}

/// Run a query command for what a dialog shows (Properties, Source Settings).
fn query(s: &mut Session, id: &str) -> Result<Value> {
    let c = crate::find_command(id).ok_or_else(|| bad("dialog.defaults", format!("no command `{id}`")))?;
    (c.run)(s, &json!({}))
}

/// egui's `menu_dialogs::defaults`.
fn menu_defaults(s: &mut Session, id: &str, p: &Value) -> Result<(Value, Value)> {
    use crate::commands::bool_p;
    let ops: Vec<Value> = cfa4_project::FindOp::ALL.iter().map(|o| json!([o.name(), o.label()])).collect();
    Ok(match dialog_command(id) {
        "edit.find" => {
            let timeline = bool_p(p, "timeline").unwrap_or(false) && s.active_sequence().is_some();
            let mut rows: Vec<Value> = match &s.state.find {
                Some(f) if f.scope == "project" => f.query.rows.iter().map(|r| json!({"column": r.column, "operator": r.op.name(), "text": r.text})).collect(),
                _ => Vec::new(),
            };
            while rows.len() < 2 {
                rows.push(json!({"column": "Name", "operator": "contains", "text": ""}));
            }
            (
                json!({"scope": if timeline { "timeline" } else { "project" }, "rows": rows, "matchAll": true, "caseSensitive": false, "in": "all"}),
                json!({"columns": cfa4_project::find::COLUMNS, "operators": ops}),
            )
        }
        "file.newSearchBin" => (
            json!({"name": "", "column": "All", "operator": "contains", "text": str_p(p, "searchText").unwrap_or_default()}),
            json!({"columns": cfa4_project::find::COLUMNS, "operators": ops}),
        ),
        "project.settings" => {
            let tab = match id {
                "file.projectSettings.scratchDisks" => "scratchDisks",
                "project.ingestSettings" => "ingest",
                _ => "general",
            };
            let st = s.project.settings.clone();
            let general = crate::project_tools::general_json(&st);
            let ingest = serde_json::to_value(&st.ingest).unwrap_or_default();
            let sc = &st.scratch;
            (
                json!({
                    "tab": tab,
                    "renderer": general["renderer"], "videoDisplay": general["videoDisplay"], "audioDisplay": general["audioDisplay"],
                    "captureFormat": general["captureFormat"], "titleSafe": general["titleSafe"], "actionSafe": general["actionSafe"],
                    "captured": sc.captured, "videoPreviews": sc.video_previews, "audioPreviews": sc.audio_previews, "autoSave": sc.auto_save,
                    "ingest": ingest.clone(),
                }),
                json!({
                    "scratch": crate::project_tools::scratch_paths(s), "ingest": ingest, "project": s.project.name,
                    "renderers": general["renderers"],
                    "videoDisplays": cfa4_time::TimeDisplay::ALL.iter().map(|t| t.label()).collect::<Vec<_>>(),
                }),
            )
        }
        "file.mediaProperties" => (json!({}), query(s, "file.mediaProperties")?),
        "clip.sourceSettings" => (json!({}), query(s, "clip.sourceSettings")?),
        "clip.editOffline" => {
            let item = s.state.project_selection.first().copied().and_then(|i| s.project.item(i)).cloned();
            let mut p = json!({});
            if let Some(it) = item {
                p["item"] = json!(it.id.0);
                p["mediaName"] = json!(it.name);
                for (k, key) in crate::project_tools::OFFLINE_FIELDS {
                    p[k] = json!(it.metadata.get(key).cloned().unwrap_or_default());
                }
            }
            (p, Value::Null)
        }
        "clip.updateMetadata" => (json!({}), json!({"count": s.state.project_selection.len().max(s.state.selection.len())})),
        "clip.automateToSequence" => (
            json!({"ordering": "sort", "placement": "sequentially", "method": "insert", "overlapFrames": 30, "stillFrames": 150, "videoTransition": true, "audioTransition": true, "ignoreAudio": false, "ignoreVideo": false}),
            json!({"sequence": s.active_sequence().map(|_| s.state.active_sequence.and_then(|q| s.project.item(q)).map(|i| i.name.clone()).unwrap_or_default())}),
        ),
        "clip.sceneEditDetection" => (json!({"applyCuts": true, "createSubclips": false, "generateMarkers": false, "sensitivity": 50.0}), Value::Null),
        "sequence.normalizeMixTrack" => (json!({"db": 0.0}), Value::Null),
        "sequence.simplify" => {
            let name = s.state.active_sequence.and_then(|q| s.project.item(q)).map(|i| format!("{} (Simplified)", i.name)).unwrap_or_default();
            (
                json!({"name": name, "removeDisabled": true, "removeEmptyTracks": true, "closeGaps": false, "moveClipsDown": false, "removeVideoEffects": false, "removeAudioEffects": false, "removeText": false, "keep": "both"}),
                Value::Null,
            )
        }
        "sequence.transcribe" => {
            let tracks: Vec<String> = s.active_sequence().map(|q| (1..=q.audio_tracks.len()).map(|i| format!("A{i}")).collect()).unwrap_or_default();
            (json!({"language": "auto", "track": "mix", "diarize": true}), json!({"tracks": tracks}))
        }
        "file.saveAsTemplate" => (json!({"name": s.project.name}), Value::Null),
        "markers.addFlashCue" => (json!({"name": "", "comment": ""}), Value::Null),
        "file.newColorMatte" => {
            let st = s.active_sequence().map(|q| q.settings.clone()).unwrap_or_default();
            (json!({"color": "#000000", "name": "Color Matte", "width": st.width, "height": st.height, "seconds": 5.0}), Value::Null)
        }
        "project.matteColor" => {
            let matte = match s.state.project_selection.as_slice() {
                [id] => s.project.item(*id).and_then(|it| match &it.kind {
                    ItemKind::Media(m) => match &m.media {
                        cfa4_project::MediaRef::Generator(cfa4_media::Generator::ColorMatte { color }) => Some((it.id, *color)),
                        _ => None,
                    },
                    _ => None,
                }),
                _ => None,
            };
            let (item, color) = matte.ok_or_else(|| crate::EngineError::Other("select one Color Matte".into()))?;
            (json!({"item": item.0, "color": cfa4_color::to_hex(color)}), Value::Null)
        }
        "sequence.colorSettings" => {
            let c = s.active_sequence().map(|q| q.settings.color).unwrap_or(cfa4_color::ColorPipeline::REC709);
            let spaces: Vec<Value> = cfa4_color::WorkingSpace::ALL.iter().map(|w| json!([w.id(), w.label()])).collect();
            (json!({"workingSpace": c.working.id(), "wideGamut": c.wide_gamut, "autoToneMap": c.auto_tone_map}), json!({"workingSpaces": spaces}))
        }
        "file.exportFrame" => frame_export_defaults(s, p)?,
        "file.exportAaf" | "file.exportOmf" => {
            let omf = id == "file.exportOmf";
            let title = s.state.active_sequence.and_then(|q| s.project.item(q)).map(|i| i.name.clone()).unwrap_or_default();
            let rate = s.active_sequence().map(|q| q.settings.sample_rate).filter(|r| INTERCHANGE_RATES.contains(r)).unwrap_or(48_000);
            (
                json!({"title": title, "mixdownVideo": false, "breakoutToMono": false, "audio": "embedded", "audioFormat": "wav", "sampleRate": rate,
                       "bitDepth": 16, "trimAudio": omf, "handles": if omf { 30 } else { 0 }, "renderAudioEffects": false}),
                json!({"omf": omf, "sampleRates": INTERCHANGE_RATES}),
            )
        }
        _ => (json!({}), Value::Null),
    })
}

/// egui's `frame_export::open`: the monitor frame captured now (`target`, `item` / `sequence`,
/// `time` as `file.exportFrame` takes them), a file name from it, PNG 8-bit into the project's
/// folder; `info` has the frame's timecode and size and the still formats.
fn frame_export_defaults(s: &Session, p: &Value) -> Result<(Value, Value)> {
    let t = crate::keyboard::frame_export_target(s, p)?;
    let (rate, drop_frame) = if t.source {
        let v = crate::clip_ops::source_view(s, t.item).ok_or_else(|| bad("file.exportFrame", "Source clip is unavailable"))?;
        (v.rate, false)
    } else {
        let q = s.project.sequence(t.item).ok_or(crate::EngineError::NoSequence)?;
        (q.settings.frame_rate, q.settings.drop_frame)
    };
    let timecode = cfa4_time::format_time(t.time, rate, drop_frame, cfa4_time::TimeDisplay::Timecode, 48_000);
    let stem = if t.source { std::path::Path::new(&t.name).file_stem().and_then(|x| x.to_str()).unwrap_or(&t.name) } else { &t.name };
    let name: String = stem.trim().chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect();
    let folder = s.path.as_deref().and_then(|x| std::path::Path::new(x).parent()).map(|x| x.to_string_lossy().into_owned()).unwrap_or_else(|| ".".into());
    let formats: Vec<Value> = crate::keyboard::StillFormat::ALL.iter().map(|f| json!([f.extension(), f.label(), f.supports_16()])).collect();
    let mut params = json!({"name": name, "format": "png", "depth": 8, "folder": folder, "import": false,
                            "target": if t.source { "source" } else { "program" }, "time": t.time.0});
    params[if t.source { "item" } else { "sequence" }] = json!(t.item.0);
    Ok((params, json!({"timecode": timecode, "source": t.source, "itemName": t.name, "width": t.width, "height": t.height, "formats": formats})))
}

fn format_name(f: AudioChannels) -> &'static str {
    match f {
        AudioChannels::Mono => "mono",
        AudioChannels::Stereo => "stereo",
        AudioChannels::Surround51 => "5.1",
        AudioChannels::Adaptive => "adaptive",
    }
}

/// Initial parameters (and display info) of a clip dialog; None for a menu dialog.
fn defaults(s: &Session, id: &str) -> Option<(Value, Value)> {
    Some(match id {
        "edit.pasteAttributes" | "edit.removeAttributes" => {
            // effects offered: the copied clips' (Paste) or the selected clips' (Remove) standard effects
            let mut names: Vec<(String, String)> = Vec::new();
            let mut add = |e: &cfa4_project::EffectInstance| {
                if let Some(d) = e.def().filter(|d| !d.intrinsic && !cfa4_project::graphic::is_layer_id(d.id))
                    && !names.iter().any(|n| n.0 == d.id)
                {
                    names.push((d.id.to_string(), d.name.to_string()));
                }
            };
            let mut has_v = false;
            let mut has_a = false;
            if id == "edit.pasteAttributes" {
                for (k, _, it) in &s.state.clipboard {
                    has_v |= *k == TrackKind::Video;
                    has_a |= *k == TrackKind::Audio;
                    it.effects.iter().for_each(&mut add);
                }
            } else if let Some(q) = s.active_sequence() {
                for c in &s.state.selection {
                    if let Some((tid, it)) = q.find_item(*c) {
                        let v = q.video_tracks.iter().any(|t| t.id == tid);
                        has_v |= v;
                        has_a |= !v;
                        it.effects.iter().for_each(&mut add);
                    }
                }
            }
            let mut p = json!({"motion": has_v, "opacity": has_v, "timeRemapping": has_v, "volume": has_a, "channelVolume": has_a, "panner": has_a,
                "effects": names.iter().map(|n| n.0.clone()).collect::<Vec<_>>()});
            if id == "edit.pasteAttributes" {
                p["scaleTimes"] = json!(true);
            }
            (p, json!({"video": has_v, "audio": has_a, "effects": names}))
        }
        "file.newOfflineFile" => {
            let st = s.active_sequence().map(|q| q.settings.clone()).unwrap_or_default();
            (
                json!({"fileName": "Offline.mov", "name": "", "tapeName": "", "description": "", "video": true, "audio": true, "width": st.width, "height": st.height,
                    "fps": st.frame_rate.as_f64(), "timecode": "00:00:00:00", "seconds": 10.0}),
                Value::Null,
            )
        }
        "clip.nest" => (json!({"name": crate::commands::next_nested_name(s)}), Value::Null),
        "clip.makeSubclip" => {
            let d = crate::clip_ops::subclip_defaults(s, &json!({})).unwrap_or_else(|| json!({}));
            (json!({"name": d["name"], "startFrame": d["startFrame"], "endFrame": d["endFrame"], "restrictTrims": true}), json!({"fps": d["fps"]}))
        }
        "clip.editSubclip" => {
            let sub = s.state.project_selection.iter().find_map(|i| s.project.item(*i).filter(|it| matches!(it.kind, ItemKind::Subclip { .. })));
            let Some(it) = sub else { return Some((json!({}), Value::Null)) };
            let ItemKind::Subclip { parent, range, restrict_trims } = &it.kind else { return Some((json!({}), Value::Null)) };
            let rate = s.project.item(*parent).and_then(|p| p.as_media()).map(|m| m.frame_rate()).unwrap_or_default();
            (
                json!({"item": it.id.0, "startFrame": rate.frame_at(range.start), "endFrame": rate.frame_at(range.end()), "restrictTrims": restrict_trims, "convertToMaster": false}),
                json!({"name": it.name, "fps": rate.as_f64()}),
            )
        }
        "clip.audioChannels" => {
            let item = s.state.project_selection.iter().find_map(|i| s.project.item(*i).and_then(|it| it.as_media()).filter(|m| m.info.has_audio()));
            match item {
                Some(m) => {
                    let n = u16::try_from(m.info.audio().map_or(2, |a| a.channels)).unwrap_or(u16::MAX);
                    let map = m.interpret.audio_channels.clone().unwrap_or_else(|| AudioChannelMap::for_format(AudioChannels::Stereo, n));
                    (json!({"format": format_name(map.format), "clips": map.clips}), json!({"channels": n, "items": true}))
                }
                None => (json!({"channels": [0, 1]}), json!({"channels": 2, "items": false})),
            }
        }
        "clip.modifyTimecode" => {
            let it = s.state.project_selection.iter().find_map(|i| s.project.item(*i).and_then(|it| Some((it, it.as_media()?))));
            let (tc, tape) = it
                .map(|(it, m)| {
                    let f = m.info.start_timecode.unwrap_or(0);
                    (cfa4_time::format_timecode_frames(f, m.frame_rate(), false), it.metadata.get("Tape Name").cloned().unwrap_or_default())
                })
                .unwrap_or_else(|| ("00:00:00:00".into(), String::new()));
            (json!({"timecode": tc, "tapeName": tape}), Value::Null)
        }
        "clip.frameHoldOptions" => {
            let it = s.active_sequence().and_then(|q| s.state.selection.iter().find_map(|c| q.find_item(*c).map(|x| x.1.clone())));
            let enabled = it.as_ref().is_none_or(|i| i.frame_hold.is_some());
            let hold_filters = it.as_ref().is_some_and(|i| i.hold_filters);
            let tc = it
                .as_ref()
                .map(|i| {
                    let m = s.project.item(i.item).and_then(|p| p.as_media());
                    let rate = m.map(|m| m.frame_rate()).unwrap_or_default();
                    // the source timecode `sourceTimecode` reads back: the media's start timecode on
                    let start_tc = if i.frame_hold.is_some() { m.and_then(|m| m.info.start_timecode).unwrap_or(0) } else { 0 };
                    cfa4_time::format_timecode_frames(rate.frame_at(i.frame_hold.unwrap_or(i.source_in)).saturating_add(start_tc), rate, false)
                })
                .unwrap_or_else(|| "00:00:00:00".into());
            // a clip already held keeps its frame: OK (or a Hold Filters change) holds the same one
            let hold_on = if it.as_ref().is_some_and(|i| i.frame_hold.is_some()) { "sourceTimecode" } else { "in" };
            (json!({"enabled": enabled, "holdOn": hold_on, "timecode": tc, "holdFilters": hold_filters}), Value::Null)
        }
        "clip.fieldOptions" => (json!({"reverseFieldDominance": false, "processing": "none"}), Value::Null),
        "clip.speedDuration" => {
            // the first selected clip's current speed; its duration at that speed for the readout
            let q = s.active_sequence();
            let it = q.and_then(|q| s.state.selection.iter().find_map(|c| q.find_item(*c).map(|x| x.1.clone())));
            let rate = q.map(|q| q.settings.frame_rate).unwrap_or_default();
            let speed = it.as_ref().map_or(1.0, |i| i.speed);
            let speed = if speed.is_finite() { speed } else { 1.0 };
            let interp = it.as_ref().map_or(cfa4_project::TimeInterpolation::default(), |i| i.time_interpolation);
            (
                json!({"speed": (speed.abs() * 100.0 * 100.0).round() / 100.0, "reverse": speed < 0.0 || it.as_ref().is_some_and(|i| i.reverse), "ripple": false, "interpolation": interp.name()}),
                json!({"duration": it.as_ref().map_or(0, |i| i.duration.0), "speed": speed.abs(), "fps": rate.as_f64(), "fpsNum": rate.num, "fpsDen": rate.den}),
            )
        }
        _ => return None,
    })
}
