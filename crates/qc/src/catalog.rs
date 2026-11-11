//! The checks CFA4 QC knows: stable ids, what each measures and in which unit.
//!
//! A profile may only name checks listed here, so a typo in a profile is an error instead of a
//! check that silently never runs. Ids are part of the report format: never rename one, add a new
//! id instead.

use serde::Serialize;

/// What a check reports as its measured value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Measure {
    /// A number in [`CheckInfo::unit`], compared with the profile's bounds.
    Value,
    /// No number: the check passes or fails on what it finds (events, missing streams).
    Findings,
}

/// One known check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckInfo {
    pub id: &'static str,
    /// Plan section the check belongs to (4.1 input, 4.2 integrity, 4.3 audio, 4.4 picture).
    pub group: &'static str,
    pub title: &'static str,
    pub measure: Measure,
    /// Unit of the measured value ("" for [`Measure::Findings`]).
    pub unit: &'static str,
}

const fn value(id: &'static str, group: &'static str, title: &'static str, unit: &'static str) -> CheckInfo {
    CheckInfo { id, group, title, measure: Measure::Value, unit }
}

const fn findings(id: &'static str, group: &'static str, title: &'static str) -> CheckInfo {
    CheckInfo { id, group, title, measure: Measure::Findings, unit: "" }
}

/// Every check, in report order.
pub const CHECKS: &[CheckInfo] = &[
    // 4.1 input pre-check
    findings("probe.open", "input", "File opens and lists its streams"),
    findings("probe.video", "input", "Video stream parameters (codec, size, rate, time base, pixel format)"),
    findings("probe.color", "input", "Colour primaries, transfer, matrix and HDR metadata"),
    findings("probe.audio", "input", "Audio sample rate, channel count and layout"),
    findings("probe.consistency", "input", "Missing or contradicting critical metadata"),
    // 4.2 integrity after render
    findings("integrity.decode", "integrity", "Full decode of every stream"),
    value("integrity.frameCount", "integrity", "Frame count minus the render plan's", "frames"),
    value("integrity.duration", "integrity", "Duration minus the render plan's", "ms"),
    findings("integrity.frames", "integrity", "Damaged, missing or unexpectedly repeated frames"),
    value("integrity.avOffset", "integrity", "Audio end minus video end", "ms"),
    findings("integrity.streams", "integrity", "Expected audio tracks, channel layout and subtitle streams"),
    // 4.3 audio
    findings("audio.silence", "audio", "Silence and unexpected audio gaps"),
    value("audio.clipping", "audio", "Clipped samples", "samples"),
    value("audio.truePeak", "audio", "Maximum true peak (ITU-R BS.1770)", "dBTP"),
    value("audio.loudness", "audio", "Integrated loudness (ITU-R BS.1770 / EBU R128)", "LUFS"),
    value("audio.loudnessRange", "audio", "Loudness range (EBU Tech 3342)", "LU"),
    findings("audio.layout", "audio", "Channel layout and track choice against the export settings"),
    // 4.4 picture
    findings("video.contactSheet", "picture", "Contact sheet of selected frames"),
    findings("video.black", "picture", "Black or blank frames"),
    value("video.clipBlack", "picture", "Pixels clipped to black", "%"),
    value("video.clipWhite", "picture", "Pixels clipped to white", "%"),
    value("video.referenceSsim", "picture", "SSIM of export frames against preview references", "SSIM"),
];

/// The check with this id.
pub fn check(id: &str) -> Option<&'static CheckInfo> {
    CHECKS.iter().find(|c| c.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_units_match_measure() {
        for (i, c) in CHECKS.iter().enumerate() {
            assert!(CHECKS.iter().skip(i + 1).all(|d| d.id != c.id), "duplicate id {}", c.id);
            assert_eq!(c.unit.is_empty(), c.measure == Measure::Findings, "{}", c.id);
        }
        assert!(check("audio.loudness").is_some());
        assert!(check("audio.loudnes").is_none());
    }
}
