//! AMWA AS-07 (MXF Archive and Preservation, Appendix J "Baseband Shim"): a check of a read
//! file's essence against the shim.
//!
//! The shim wants one picture track of JPEG 2000 or ST 384 uncompressed pictures (component
//! depth 8, 10, 12 or 16 bits) in an OP1a file with frame or line wrapping, and linear PCM sound
//! at 48 kHz, 16 or 24 bit. The check reports every breach; the DMS-AS-07 metadata and the
//! content integrity checks of the specification are not covered.

use crate::{Codec, MxfFile, OperationalPattern, TrackKind, Wrapping};

/// Component depths the shim allows.
pub const DEPTHS: &[u32] = &[8, 10, 12, 16];

/// The breaches of the Baseband Shim in `f`; empty when the file conforms.
pub fn baseband_problems(f: &MxfFile) -> Vec<String> {
    let mut p: Vec<String> = Vec::new();
    if f.operational_pattern != (OperationalPattern::Generalized { item: 1, package: 1 }) {
        p.push(format!("the operational pattern is {}, the shim needs OP1a", f.operational_pattern.name()));
    }
    let pictures: Vec<_> = f.tracks.iter().filter(|t| t.kind == TrackKind::Picture).collect();
    match pictures.len() {
        0 => p.push("the file has no picture track".into()),
        1 => {}
        n => p.push(format!("the file has {n} picture tracks, the shim needs one")),
    }
    for t in pictures {
        if !matches!(t.codec, Codec::Jpeg2000 | Codec::Uncompressed) {
            p.push(format!("picture track {} is not JPEG 2000 or ST 384 uncompressed", t.track_id));
        }
        if t.wrapping == Wrapping::Clip {
            p.push(format!("picture track {} is clip wrapped, the shim needs frame wrapping", t.track_id));
        }
        match &t.picture {
            Some(pi) if !DEPTHS.contains(&pi.component_depth) => {
                p.push(format!("picture track {} has {} bit components, the shim allows 8, 10, 12 or 16", t.track_id, pi.component_depth));
            }
            _ => {}
        }
    }
    for t in f.tracks.iter().filter(|t| t.kind == TrackKind::Sound) {
        if t.codec != Codec::Pcm {
            p.push(format!("sound track {} is not linear PCM", t.track_id));
        }
        if let Some(s) = &t.sound {
            if s.sample_rate.num != 48_000 * s.sample_rate.den.max(1) || s.sample_rate.den == 0 {
                p.push(format!("sound track {} is {}/{} Hz, the shim needs 48 kHz", t.track_id, s.sample_rate.num, s.sample_rate.den));
            }
            if s.bits != 16 && s.bits != 24 {
                p.push(format!("sound track {} is {} bit, the shim needs 16 or 24 bit", t.track_id, s.bits));
            }
        }
    }
    p
}

/// `Ok` when `f` conforms to the Baseband Shim, else all breaches in one message.
pub fn check_baseband(f: &MxfFile) -> Result<(), String> {
    let p = baseband_problems(f);
    if p.is_empty() { Ok(()) } else { Err(format!("AS-07 Baseband Shim: {}", p.join("; "))) }
}
