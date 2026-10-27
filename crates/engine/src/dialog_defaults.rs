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

/// The commands that have a dialog.
pub const DIALOGS: [&str; 11] = [
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
];

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "dialog.defaults",
        label: "Dialog Defaults",
        menu: &[],
        shortcut: None,
        params: r#"{"command":"edit.pasteAttributes|edit.removeAttributes|file.newOfflineFile|clip.nest|clip.makeSubclip|clip.editSubclip|clip.audioChannels|clip.modifyTimecode|clip.frameHoldOptions|clip.fieldOptions|clip.speedDuration"}"#,
        enabled: always,
        run: dialog_defaults,
        journal: false,
    }]
}

fn dialog_defaults(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "dialog.defaults";
    let id = str_p(p, "command").ok_or_else(|| bad(CMD, "`command` is required"))?;
    if !DIALOGS.contains(&id) {
        return Err(bad(CMD, format!("`{id}` has no dialog")));
    }
    // a command that can't run now opens no dialog (egui's)
    if let Some(c) = crate::find_command(id) {
        (c.enabled)(s).map_err(crate::EngineError::Other)?;
    }
    let (params, info) = defaults(s, id);
    Ok(json!({"command": id, "params": params, "info": info}))
}

fn format_name(f: AudioChannels) -> &'static str {
    match f {
        AudioChannels::Mono => "mono",
        AudioChannels::Stereo => "stereo",
        AudioChannels::Surround51 => "5.1",
        AudioChannels::Adaptive => "adaptive",
    }
}

/// Initial parameters (and display info) of a command's dialog.
fn defaults(s: &Session, id: &str) -> (Value, Value) {
    match id {
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
            let Some(it) = sub else { return (json!({}), Value::Null) };
            let ItemKind::Subclip { parent, range, restrict_trims } = &it.kind else { return (json!({}), Value::Null) };
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
                    let rate = s.project.item(i.item).and_then(|p| p.as_media()).map(|m| m.frame_rate()).unwrap_or_default();
                    cfa4_time::format_timecode_frames(rate.frame_at(i.frame_hold.unwrap_or(i.source_in)), rate, false)
                })
                .unwrap_or_else(|| "00:00:00:00".into());
            (json!({"enabled": enabled, "holdOn": "in", "timecode": tc, "holdFilters": hold_filters}), Value::Null)
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
        _ => (json!({}), Value::Null),
    }
}
