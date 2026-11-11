//! The rule catalogue. Ids are stable: a rule's meaning never changes under the same id; a changed
//! rule gets a new id and the profiles move to it in a new profile version.

use serde::Serialize;

/// One rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Rule {
    pub id: &'static str,
    pub title: &'static str,
    /// The work package of the plan that implements it natively (`DCP-05`, …).
    pub task: &'static str,
}

const fn r(id: &'static str, title: &'static str, task: &'static str) -> Rule {
    Rule { id, title, task }
}

pub const AM_PRESENT: &str = "DCP.AM.001";
pub const AM_DOCUMENT: &str = "DCP.AM.002";
pub const AM_UNIQUE_IDS: &str = "DCP.AM.003";
pub const AM_PATH_SAFE: &str = "DCP.AM.004";
pub const AM_FILE_EXISTS: &str = "DCP.AM.005";
pub const AM_HAS_PKL: &str = "DCP.AM.006";
pub const PKL_DOCUMENT: &str = "DCP.PKL.001";
pub const PKL_IN_ASSETMAP: &str = "DCP.PKL.002";
pub const PKL_SIZE: &str = "DCP.PKL.003";
pub const PKL_HASH: &str = "DCP.PKL.004";
pub const CPL_DOCUMENT: &str = "DCP.CPL.001";
pub const CPL_IN_PKL: &str = "DCP.CPL.002";
pub const CPL_ASSETS_IN_PKL: &str = "DCP.CPL.003";
pub const CPL_DURATIONS: &str = "DCP.CPL.004";
pub const CPL_REEL_SYNC: &str = "DCP.CPL.005";
pub const STD_CONSISTENT: &str = "DCP.STD.001";
pub const MXF_TRACK_FILES: &str = "DCP.MXF.001";
pub const J2K_CODESTREAM: &str = "DCP.J2K.001";
pub const AUD_PCM: &str = "DCP.AUD.001";
pub const AUD_LOUDNESS: &str = "DCP.AUD.002";
pub const IAB_TRACK: &str = "DCP.IAB.001";
pub const SUB_DOCUMENTS: &str = "DCP.SUB.001";
pub const NAME_ISDCF: &str = "DCP.NAME.001";
pub const SIG_SIGNATURE: &str = "DCP.SIG.001";
pub const SIG_CERTIFICATES: &str = "DCP.SIG.002";
pub const MARK_MARKERS: &str = "DCP.MARK.001";

pub const RULES: &[Rule] = &[
    r(AM_PRESENT, "The package has one readable asset map (ASSETMAP.xml or ASSETMAP)", "DCP-03"),
    r(AM_DOCUMENT, "The asset map is well-formed and complete", "DCP-03"),
    r(AM_UNIQUE_IDS, "Asset map ids are unique", "DCP-05"),
    r(AM_PATH_SAFE, "Every chunk path is relative and stays inside the package", "DCP-03"),
    r(AM_FILE_EXISTS, "Every chunk file exists and is a regular file", "DCP-03"),
    r(AM_HAS_PKL, "The asset map declares at least one packing list", "DCP-05"),
    r(PKL_DOCUMENT, "Every packing list is well-formed and complete", "DCP-03"),
    r(PKL_IN_ASSETMAP, "Every packing list asset is in the asset map", "DCP-05"),
    r(PKL_SIZE, "Every file has the size its packing list declares", "DCP-05"),
    r(PKL_HASH, "Every file has the SHA-1 its packing list declares", "DCP-05"),
    r(CPL_DOCUMENT, "Every composition playlist is well-formed and complete", "DCP-03"),
    r(CPL_IN_PKL, "Every composition playlist is in a packing list", "DCP-05"),
    r(CPL_ASSETS_IN_PKL, "Every reel asset is in a packing list (or the package is a VF)", "DCP-05"),
    r(CPL_DURATIONS, "EntryPoint + Duration fit in IntrinsicDuration", "DCP-05"),
    r(CPL_REEL_SYNC, "The assets of a reel have the same duration and edit rate", "DCP-05"),
    r(STD_CONSISTENT, "Every document is of the profile's standard (SMPTE or Interop)", "DCP-03"),
    r(MXF_TRACK_FILES, "Track files are valid MXF of the declared kind", "DCP-06"),
    r(J2K_CODESTREAM, "Picture codestreams follow the DCI JPEG 2000 profiles", "DCP-06"),
    r(AUD_PCM, "Sound track files are 24-bit 48/96 kHz PCM with a valid channel layout", "DCP-07"),
    r(AUD_LOUDNESS, "Loudness and peaks are measured and within the profile", "DCP-07"),
    r(IAB_TRACK, "Immersive audio track files are valid IAB", "DCP-07"),
    r(SUB_DOCUMENTS, "Subtitle and caption documents and their fonts are valid", "DCP-08"),
    r(NAME_ISDCF, "The content title follows the ISDCF naming convention", "DCP-09"),
    r(SIG_SIGNATURE, "Signatures of CPL and PKL verify", "DCP-10"),
    r(SIG_CERTIFICATES, "The signer certificate chain is valid", "DCP-10"),
    r(MARK_MARKERS, "Composition markers are present and in range", "DCP-05"),
];

pub fn find(id: &str) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.id == id)
}
