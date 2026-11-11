//! The audio of a timeline Smart Render (WP-09b of `CFA4-SMART-CUT-RENDER-R1`, SCE-001 §9).
//!
//! A sequence's audio is a mix (clips, levels, effects, transitions), so it cannot be copied
//! packet for packet: it is mixed through the export's audio path ([`crate::audio_out`], the same
//! mix a full render writes) and encoded as AAC. The encoder delay (priming) is hidden by the
//! edit list, so the first mixed sample is presented with the first video picture, and the
//! presentation lasts exactly as long as the video. [`crate::audio_copy::mux_audio_tracks`] adds
//! the track next to the video and [`crate::audio_copy::verify_audio_tracks`] checks it.

use std::sync::Arc;

use cfa4_project::{ItemId, Project};
use cfa4_render::SourceProvider;
use cfa4_time::TimeRange;

use crate::audio_copy::AudioTrackInput;
use crate::audio_out::AudioOut;
use crate::smart_cut::SmartCutError;
use crate::{ExportSettings, Format};

fn encode_error(e: impl std::fmt::Display) -> SmartCutError {
    SmartCutError::Encode { detail: format!("audio: {e}") }
}

/// Whether the sequence has any audio to mix (an enabled clip on an enabled audio track).
pub fn sequence_has_audio(project: &Project, seq: ItemId) -> bool {
    project.sequence(seq).is_some_and(|q| q.audio_tracks.iter().any(|t| t.enabled && t.items.iter().any(|i| i.enabled)))
}

/// The sequence's mix over `range`, encoded as AAC with the export's default audio settings
/// (sequence sample rate, stereo or 5.1 as the sequence mix is). `None` when the sequence has no
/// audio or the range is empty.
pub fn encode_sequence_audio(
    project: &Arc<Project>,
    seq: ItemId,
    range: TimeRange,
    sources: &dyn SourceProvider,
) -> Result<Option<AudioTrackInput>, SmartCutError> {
    if !sequence_has_audio(project, seq) {
        return Ok(None);
    }
    let settings = ExportSettings::default();
    let mut mix = AudioOut::new(Arc::clone(project), seq, &settings, range).map_err(encode_error)?;
    let total = mix.remaining();
    if total == 0 {
        return Ok(None);
    }
    let (sr, channels) = (mix.sr, mix.channels as u32);
    let mut enc = crate::aac_factory(Format::H264, sr, channels, &settings).ok_or_else(|| encode_error("no AAC encoder"))?.map_err(encode_error)?;
    let mut packets = Vec::new();
    while let Some(planar) = mix.pull(mix.out.saturating_add(i64::from(sr.max(1))), sources) {
        packets.extend(enc.encode(&planar).map_err(encode_error)?);
    }
    packets.extend(enc.flush().map_err(encode_error)?);
    let frame = enc.frame_size();
    let priming = enc.priming();
    let coded = (packets.len() as u64).saturating_mul(u64::from(frame));
    if coded < u64::from(priming) + total {
        return Err(encode_error(format!("{coded} samples encoded for {total} mixed and {priming} of priming")));
    }
    Ok(Some(AudioTrackInput {
        track_id: 0,
        codec: "aac".into(),
        entry: enc.sample_entry(),
        timescale: sr,
        language: "und".into(),
        lead_in: 0,
        media_time: i64::from(priming),
        presented: total,
        samples: packets.into_iter().map(|p| (p, frame)).collect(),
        pcm: false,
        // the first packet starts `priming` samples before the first mixed one, which is
        // presented with the first picture
        first_at: (-i128::from(priming), i128::from(sr.max(1))),
    }))
}
