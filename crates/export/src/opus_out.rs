//! Opus Audio: the sequence mix alone, coded by CFA4's Opus encoder (CELT, 48 kHz, 20 ms frames,
//! constant bit rate) in an Ogg Opus file (RFC 7845: `OpusHead` with the encoder delay as
//! pre-skip, `OpusTags` with the export metadata, the last page's granule position trimming the
//! padding of the final frame).

use std::sync::Arc;
use std::sync::atomic::Ordering;

use cfa4_ogg::OggWriter;
use cfa4_opus::Encoder;
use cfa4_project::{ItemId, Project};
use cfa4_render::SourceProvider;
use cfa4_time::TimeRange;

use crate::audio_out::{AudioOut, interleave};
use crate::settings::opus_bitrate_kbps;
use crate::{ExportError, ExportSettings, Out, Progress, Result};

/// Ogg logical bitstream serial number of the Opus stream.
const SERIAL: u32 = 0x0CFA_0004;

/// The `OpusTags` header: vendor string and user comments (RFC 7845 §5.2).
pub(crate) fn opus_tags(comments: &[String]) -> Vec<u8> {
    let vendor = format!("CFA4 {}", env!("CARGO_PKG_VERSION"));
    let mut t = b"OpusTags".to_vec();
    t.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    t.extend_from_slice(vendor.as_bytes());
    t.extend_from_slice(&(comments.len() as u32).to_le_bytes());
    for c in comments {
        t.extend_from_slice(&(c.len() as u32).to_le_bytes());
        t.extend_from_slice(c.as_bytes());
    }
    t
}

/// Encode the mix of `range` to `settings.path`; returns (bytes written, samples per channel).
pub(crate) fn export(
    project: &Arc<Project>,
    seq: ItemId,
    settings: &ExportSettings,
    range: TimeRange,
    sources: &dyn SourceProvider,
    progress: &Progress,
) -> Result<(u64, u64)> {
    let cancelled = || progress.cancel.load(Ordering::Relaxed);
    let mut a = AudioOut::new(project.clone(), seq, settings, range)?;
    if a.sr != 48_000 || !(1..=2).contains(&a.channels) {
        return Err(ExportError::Unsupported(format!("Opus: {} Hz with {} channels (the encoder takes 48 kHz mono or stereo)", a.sr, a.channels)));
    }
    let ch = a.channels;
    let mut enc =
        Encoder::new(ch, opus_bitrate_kbps(settings.audio.bitrate_kbps).saturating_mul(1000)).map_err(|e| ExportError::Unsupported(format!("Opus: {e}")))?;
    let frames = a.remaining();
    let chunk = i64::from(a.sr);
    if !settings.part_of_batch {
        progress.total.store(frames.div_ceil(chunk as u64).max(1), Ordering::Relaxed);
        progress.set_status(format!("Exporting audio ({})", settings.format.label()));
    }
    a.measure(settings, sources, &cancelled)?;
    *progress.loudness.lock().unwrap_or_else(|e| e.into_inner()) = a.loudness;

    let io = |e: std::io::Error| ExportError::Io(e.to_string());
    let mut ogg = OggWriter::new(Out::create(settings)?, SERIAL);
    ogg.write_packet(&enc.head(a.sr).to_bytes(), 0).map_err(io)?;
    ogg.flush_page().map_err(io)?;
    ogg.write_packet(&opus_tags(&settings.metadata.vorbis_comments()), 0).map_err(io)?;
    ogg.flush_page().map_err(io)?;

    // Granule positions count the pre-skip; the final one trims the padding of the last frame.
    let end = frames.saturating_add(Encoder::LOOKAHEAD as u64);
    let frame_len = Encoder::FRAME_SIZE * ch;
    let mut pending: Vec<f32> = Vec::with_capacity(chunk as usize * ch + frame_len);
    let mut coded = 0u64;
    let mut write_frames = |pending: &mut Vec<f32>, ogg: &mut OggWriter<Out>, coded: &mut u64, flush: bool| -> Result<()> {
        let mut at = 0;
        while pending.len() - at >= frame_len || (flush && *coded < end) {
            let take = (pending.len() - at).min(frame_len);
            let pkt = enc.encode(&pending[at..at + take]);
            at += take;
            *coded += Encoder::FRAME_SIZE as u64;
            ogg.write_packet(&pkt, (*coded).min(end)).map_err(io)?;
        }
        pending.drain(..at);
        Ok(())
    };
    while let Some(planar) = a.pull(a.out.saturating_add(chunk), sources) {
        if cancelled() {
            return Err(ExportError::Cancelled);
        }
        pending.extend(interleave(&planar));
        write_frames(&mut pending, &mut ogg, &mut coded, false)?;
        if !settings.part_of_batch {
            progress.done.fetch_add(1, Ordering::Relaxed);
        }
    }
    write_frames(&mut pending, &mut ogg, &mut coded, true)?;
    let n = ogg.finish().map_err(io)?.finish(settings)?;
    if settings.part_of_batch {
        progress.done.store(1, Ordering::Relaxed);
    }
    Ok((n, frames))
}
