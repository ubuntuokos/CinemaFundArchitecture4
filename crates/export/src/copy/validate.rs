//! Strict Validator (SCE-001 v1.5 §18): read the written output back and prove every copied
//! payload byte-identical to the source, with the same timing and sync flags.

use std::fs::File;
use std::sync::atomic::Ordering;

use cfa4_isobmff::Mp4File;

use super::strict::{CANCEL_EVERY, codec_name, interleave};
use super::{CopyError, CopyErrorCode as Code, StrictValidation};
use crate::Progress;

fn fail(message: String) -> CopyError {
    CopyError::new(Code::StrictValidationFailed, message)
}

/// Compare the output (`out`, its track `k` holding source track `tracks[k]`) with the source.
pub(super) fn validate_strict(f: &Mp4File, src: &File, tracks: &[usize], out: &File, progress: &Progress) -> Result<StrictValidation, CopyError> {
    let g = cfa4_isobmff::open(out).map_err(|e| fail(format!("the written output cannot be read back: {e}")))?;
    if g.tracks.len() != tracks.len() {
        return Err(fail(format!("the output has {} tracks, {} were copied", g.tracks.len(), tracks.len())));
    }
    for (k, &t) in tracks.iter().enumerate() {
        let (Some(a), Some(b)) = (f.tracks.get(t), g.tracks.get(k)) else {
            return Err(fail(format!("track {t} is missing from the output")));
        };
        if a.kind != b.kind || a.timescale != b.timescale {
            return Err(fail(format!("track {t}: kind or timescale changed")));
        }
        if a.entries.first() != b.entries.first() {
            return Err(fail(format!("track {t}: the sample description changed ({} → {})", codec_name(f, t), codec_name(&g, k))));
        }
        if a.edits != b.edits || a.edit_offset != b.edit_offset {
            return Err(fail(format!("track {t}: the edit list changed")));
        }
        if (a.matrix, a.enabled, a.alternate_group, a.volume, &a.language) != (b.matrix, b.enabled, b.alternate_group, b.volume, &b.language) {
            return Err(fail(format!("track {t}: the track header (matrix, enabled flag, alternate group, volume or language) changed")));
        }
        if b.sample_groups.len() != a.sample_groups.len() || b.sdtp != a.sdtp {
            return Err(fail(format!("track {t}: sample groups or dependency flags changed")));
        }
        if a.samples.len() != b.samples.len() {
            return Err(fail(format!("track {t}: {} samples in the source, {} in the output", a.samples.len(), b.samples.len())));
        }
        for (i, (x, y)) in a.samples.iter().zip(&b.samples).enumerate() {
            if (x.size, x.dts, x.pts, x.duration, x.is_sync) != (y.size, y.dts, y.pts, y.duration, y.is_sync) {
                return Err(fail(format!("track {t} sample {i}: size, timing or sync flag changed")));
            }
        }
    }
    // payload bytes, in the order they were written
    let mut packets = 0u64;
    let mut payload_bytes = 0u64;
    for (n, (t, i)) in interleave(f, tracks).into_iter().enumerate() {
        if n % CANCEL_EVERY == 0 && progress.cancel.load(Ordering::Relaxed) {
            return Err(CopyError::new(Code::Cancelled, "cancelled"));
        }
        let k = tracks.iter().position(|&x| x == t).unwrap_or(usize::MAX);
        let a = f.read_sample(src, t, i).map_err(|e| CopyError::new(Code::IoFailed, format!("reading source track {t} sample {i}: {e}")))?;
        let b = g.read_sample(out, k, i).map_err(|e| fail(format!("reading output track {k} sample {i}: {e}")))?;
        if a != b {
            return Err(fail(format!("track {t} sample {i}: the payload differs from the source")));
        }
        packets += 1;
        payload_bytes += a.len() as u64;
        progress.done.fetch_add(1, Ordering::Relaxed);
    }
    Ok(StrictValidation {
        result: "PAYLOAD_BIT_EXACT: PASS".into(),
        packets,
        payload_bytes,
        not_run: vec!["VALIDATION_DECODE_NOT_AVAILABLE: the copied streams were not decoded (payloads are byte-identical to the source)".into()],
    })
}
