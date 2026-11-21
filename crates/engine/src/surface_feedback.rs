//! Tally feedback toward control surfaces (CFA4-DECKLINK-READY-001 V4.2).
//!
//! A [`TallyState`] says what is on air; this module turns it into what a device's LED shows,
//! according to the LED the profile describes:
//!
//! | LED | program | preview | aux | nothing |
//! |---|---|---|---|---|
//! | colour (`"led": "color"`) | red | green | yellow | off |
//! | on/off only (`"led": "onOff"`, the default) | on | off | off | off |
//!
//! When several states are set, [`TallyState::dominant`] picks one: program beats preview, preview
//! beats aux. The LED a device has comes from its profile document, `feedback.tally.led`; a
//! profile that does not say gets the on/off LED, the one every device can show.
//!
//! `surface.feedback.test` computes this for a given device and state without any hardware and
//! returns the [`SurfaceFeedback`] and the LED output. Sending it to a device belongs to the I/O
//! host (ADR-0012, feedback encoding in L5), which does not write to devices yet.

use std::path::Path;

use cfa4_surface_model::{DeviceId, FeedbackPriority, FeedbackTarget, FeedbackValue, MonoNs, SurfaceFeedback, TallyLevel, TallyState};
use serde::Serialize;
use serde_json::{Value, json};

use crate::{EngineError, Result, Session};

/// The LED a device shows tally with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TallyLed {
    /// A colour LED: red, green or yellow.
    Color,
    /// A single on/off LED.
    OnOff,
}

/// The colour a colour LED shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TallyColor {
    Off,
    Red,
    Green,
    Yellow,
}

/// What the LED does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "led", content = "value")]
pub enum LedOutput {
    Color(TallyColor),
    OnOff(bool),
}

/// The LED a profile document describes (`feedback.tally.led`); on/off when it says nothing or
/// something unknown.
pub fn led_of(profile: &Value) -> TallyLed {
    match profile.pointer("/feedback/tally/led").and_then(Value::as_str) {
        Some("color") => TallyLed::Color,
        _ => TallyLed::OnOff,
    }
}

/// What `led` shows for `state`.
pub fn map_tally(state: TallyState, led: TallyLed) -> LedOutput {
    let level = state.dominant();
    match led {
        TallyLed::Color => LedOutput::Color(match level {
            Some(TallyLevel::Program) => TallyColor::Red,
            Some(TallyLevel::Preview) => TallyColor::Green,
            Some(TallyLevel::Aux) => TallyColor::Yellow,
            None => TallyColor::Off,
        }),
        TallyLed::OnOff => LedOutput::OnOff(level == Some(TallyLevel::Program)),
    }
}

/// The feedback the engine produces for a tally change of source `index` on `device`.
pub fn tally_feedback(device: &str, index: u16, state: TallyState, at: MonoNs) -> SurfaceFeedback {
    SurfaceFeedback {
        device: DeviceId(device.to_string()),
        target: FeedbackTarget::Tally { index },
        value: FeedbackValue::Tally(state),
        priority: FeedbackPriority::Immediate,
        at,
    }
}

fn bad(msg: &str) -> EngineError {
    EngineError::Other(format!("invalid_params: {msg}"))
}

/// The active profile document of `device` under `data_dir`, when there is a readable one.
fn active_profile(data_dir: &Path, device: &str) -> Option<Value> {
    let dir = crate::surface_profile::dir_for(data_dir);
    serde_json::from_slice(&std::fs::read(crate::surface_profile::active_path(&dir, device)).ok()?).ok()
}

/// `surface.feedback.test`: what a tally state looks like on a device's LED, without hardware.
pub fn cmd_test(s: &mut Session, p: &Value) -> Result<Value> {
    let target: FeedbackTarget =
        serde_json::from_value(p.get("target").cloned().ok_or_else(|| bad("`target` is required"))?).map_err(|e| bad(&format!("target: {e}")))?;
    let value: FeedbackValue =
        serde_json::from_value(p.get("value").cloned().ok_or_else(|| bad("`value` is required"))?).map_err(|e| bad(&format!("value: {e}")))?;
    let (FeedbackTarget::Tally { index }, FeedbackValue::Tally(state)) = (target, value) else {
        return Err(bad("only tally feedback (`target.type` and `value.type` \"tally\") can be tested yet"));
    };
    let device = p.get("device").and_then(Value::as_str).unwrap_or("");
    let led = match p.get("led").and_then(Value::as_str) {
        Some("color") => TallyLed::Color,
        Some("onOff") => TallyLed::OnOff,
        Some(_) => return Err(bad("`led` is \"color\" or \"onOff\"")),
        None => s.prefs_path.as_ref().and_then(|path| path.parent()).and_then(|data| active_profile(data, device)).map_or(TallyLed::OnOff, |doc| led_of(&doc)),
    };
    let feedback = tally_feedback(device, index, state, MonoNs(0));
    Ok(json!({
        "feedback": feedback,
        "dominant": state.dominant(),
        "led": led,
        "output": map_tally(state, led),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(program: bool, preview: bool, aux: bool) -> TallyState {
        TallyState { program, preview, aux }
    }

    #[test]
    fn a_colour_led_shows_red_green_yellow_and_program_wins() {
        let c = |t| map_tally(t, TallyLed::Color);
        assert_eq!(c(st(true, false, false)), LedOutput::Color(TallyColor::Red));
        assert_eq!(c(st(false, true, false)), LedOutput::Color(TallyColor::Green));
        assert_eq!(c(st(false, false, true)), LedOutput::Color(TallyColor::Yellow));
        assert_eq!(c(st(false, false, false)), LedOutput::Color(TallyColor::Off));
        assert_eq!(c(st(true, true, true)), LedOutput::Color(TallyColor::Red), "program beats preview and aux");
        assert_eq!(c(st(false, true, true)), LedOutput::Color(TallyColor::Green), "preview beats aux");
    }

    #[test]
    fn an_on_off_led_is_on_for_program_only() {
        let o = |t| map_tally(t, TallyLed::OnOff);
        assert_eq!(o(st(true, false, false)), LedOutput::OnOff(true));
        assert_eq!(o(st(true, true, false)), LedOutput::OnOff(true));
        assert_eq!(o(st(false, true, false)), LedOutput::OnOff(false));
        assert_eq!(o(st(false, false, true)), LedOutput::OnOff(false));
        assert_eq!(o(st(false, false, false)), LedOutput::OnOff(false));
    }

    #[test]
    fn the_profile_names_the_led_and_defaults_to_on_off() {
        assert_eq!(led_of(&json!({"feedback": {"tally": {"led": "color"}}})), TallyLed::Color);
        assert_eq!(led_of(&json!({"feedback": {"tally": {"led": "onOff"}}})), TallyLed::OnOff);
        assert_eq!(led_of(&json!({"feedback": {"tally": {"led": "rainbow"}}})), TallyLed::OnOff);
        assert_eq!(led_of(&json!({"feedback": {}})), TallyLed::OnOff);
        assert_eq!(led_of(&json!(null)), TallyLed::OnOff);
    }

    #[test]
    fn the_test_command_reports_feedback_and_output_without_hardware() {
        let mut s = Session::new(std::sync::Arc::new(crate::FsServices));
        let params = json!({
            "device": "midi:pad:0",
            "led": "color",
            "target": {"type": "tally", "index": 2},
            "value": {"type": "tally", "value": {"program": true, "preview": true}},
        });
        let r = cmd_test(&mut s, &params).expect("tally test");
        assert_eq!(r["feedback"]["device"], "midi:pad:0");
        assert_eq!(r["feedback"]["target"], json!({"type": "tally", "index": 2}));
        assert_eq!(r["feedback"]["priority"], "immediate");
        assert_eq!(r["dominant"], "program");
        assert_eq!(r["output"], json!({"led": "color", "value": "red"}));
        // without a profile or a `led`, the on/off LED
        let mut p2 = params.clone();
        p2.as_object_mut().map(|m| m.remove("led"));
        assert_eq!(cmd_test(&mut s, &p2).expect("default")["output"], json!({"led": "onOff", "value": true}));
        // hostile input is an error, not a crash
        for bad in [
            json!({}),
            json!({"target": 3, "value": 4}),
            json!({"target": {"type": "timecode"}, "value": {"type": "tick", "value": 1}}),
            json!({"led": "x", "target": {"type": "tally", "index": 0}, "value": {"type": "tally", "value": {}}}),
        ] {
            assert!(cmd_test(&mut s, &bad).is_err(), "{bad}");
        }
    }
}
