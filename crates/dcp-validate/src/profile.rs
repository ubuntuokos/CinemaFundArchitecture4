//! Validation profiles (plan §5): a versioned set of rules, each required or not. The eight
//! built-in profiles are defined here; a profile file format (`cfa4-dcp-profile/1`) follows with
//! DCP-11 and deserialises into the same [`Profile`].

use cfa4_dcp::Standard;
use serde::{Deserialize, Serialize};

use crate::rules::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileRule {
    pub rule: String,
    /// A required rule that is not `Pass` / `NotApplicable` keeps the verdict from `PASS`.
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// `quick-scan`, `full-integrity`, …
    pub id: String,
    pub version: u32,
    pub title: String,
    /// The standard every document must have (`DCP.STD.001`); `None` accepts both.
    pub standard: Option<Standard>,
    pub rules: Vec<ProfileRule>,
}

/// The ids of the built-in profiles.
pub const BUILTIN: &[&str] =
    &["quick-scan", "full-integrity", "smpte-cinema", "interop-cinema", "subtitle-accessibility", "audio-quality", "security", "delivery-preflight"];

const QUICK: &[&str] = &[AM_PRESENT, AM_DOCUMENT, AM_PATH_SAFE, AM_FILE_EXISTS, PKL_DOCUMENT, CPL_DOCUMENT];
const INTEGRITY: &[&str] = &[AM_UNIQUE_IDS, AM_HAS_PKL, PKL_IN_ASSETMAP, PKL_SIZE, PKL_HASH, CPL_IN_PKL, CPL_ASSETS_IN_PKL, CPL_DURATIONS, CPL_REEL_SYNC];
const CINEMA: &[&str] = &[STD_CONSISTENT, MXF_TRACK_FILES, J2K_CODESTREAM, PIC_FORMAT, AUD_PCM, MARK_MARKERS];

impl Profile {
    /// A built-in profile by id.
    pub fn builtin(id: &str) -> Option<Profile> {
        let (title, standard, required, optional): (&str, Option<Standard>, Vec<&str>, Vec<&str>) = match id {
            "quick-scan" => ("Quick Scan", None, QUICK.to_vec(), vec![]),
            "full-integrity" => ("Full Integrity", None, [QUICK, INTEGRITY].concat(), vec![]),
            "smpte-cinema" => ("SMPTE Cinema", Some(Standard::Smpte), [QUICK, INTEGRITY, CINEMA].concat(), vec![IAB_TRACK, SUB_DOCUMENTS, NAME_ISDCF]),
            "interop-cinema" => ("Interop Cinema", Some(Standard::Interop), [QUICK, INTEGRITY, CINEMA].concat(), vec![SUB_DOCUMENTS, NAME_ISDCF]),
            "subtitle-accessibility" => ("Subtitle & Accessibility", None, [QUICK, &[SUB_DOCUMENTS]].concat(), vec![]),
            "audio-quality" => ("Audio Quality", None, [QUICK, &[AUD_PCM, AUD_LOUDNESS]].concat(), vec![IAB_TRACK]),
            "security" => ("Security", None, [QUICK, &[SIG_SIGNATURE, SIG_CERTIFICATES]].concat(), vec![]),
            "delivery-preflight" => (
                "Delivery Preflight",
                None,
                [QUICK, INTEGRITY, CINEMA].concat(),
                vec![AUD_LOUDNESS, IAB_TRACK, SUB_DOCUMENTS, NAME_ISDCF, SIG_SIGNATURE, SIG_CERTIFICATES],
            ),
            _ => return None,
        };
        let rules = required
            .iter()
            .map(|r| ProfileRule { rule: (*r).to_string(), required: true })
            .chain(optional.iter().map(|r| ProfileRule { rule: (*r).to_string(), required: false }))
            .collect();
        Some(Profile { id: id.to_string(), version: 1, title: title.to_string(), standard, rules })
    }

    pub fn all() -> Vec<Profile> {
        BUILTIN.iter().filter_map(|id| Profile::builtin(id)).collect()
    }

    pub fn contains(&self, rule: &str) -> bool {
        self.rules.iter().any(|r| r.rule == rule)
    }
}
