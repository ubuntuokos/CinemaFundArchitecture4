//! Strict Bit-Exact Stream Copy executor (SCE-001 v1.5 §9): MP4 / QuickTime source → MP4 /
//! QuickTime output, payloads unchanged.

use std::fs::File;
use std::sync::atomic::Ordering;
use std::time::SystemTime;

use cfa4_isobmff::{Brand, Mp4File, Mp4Writer, TrackConfig, TrackKind, WriteSample, WriterOptions};

use super::validate::validate_strict;
use super::{CopiedTrack, CopyContainer, CopyError, CopyErrorCode as Code, CopyReport, SkippedTrack, StrictCopyRequest};
use crate::Progress;
use crate::atomic::AtomicOutput;

/// Largest single sample read into memory: anything bigger in an index is hostile or corrupt.
pub(super) const MAX_SAMPLE_BYTES: u32 = 256 << 20;

/// How often (in samples) the cancel flag is checked.
pub(super) const CANCEL_EVERY: usize = 64;

fn io(e: impl std::fmt::Display) -> CopyError {
    CopyError::new(Code::IoFailed, e.to_string())
}

/// The source file's size and modification time, to notice a change during the copy.
fn stamp(f: &File) -> Option<(u64, Option<SystemTime>)> {
    let m = f.metadata().ok()?;
    Some((m.len(), m.modified().ok()))
}

/// The `tkhd` identity matrix (16.16, 16.16, 2.30).
const IDENTITY: [i32; 9] = [0x1_0000, 0, 0, 0, 0x1_0000, 0, 0, 0, 0x4000_0000];

pub(super) fn kind_name(k: TrackKind) -> &'static str {
    match k {
        TrackKind::Video => "video",
        TrackKind::Audio => "audio",
        TrackKind::Timecode => "timecode",
        TrackKind::Subtitle => "subtitle",
        TrackKind::Other => "other",
    }
}

pub(super) fn codec_name(f: &Mp4File, t: usize) -> String {
    f.tracks.get(t).and_then(|t| t.codec()).map_or_else(|| "unknown".to_string(), |c| c.name().to_string())
}

/// The tracks to copy, or why the request cannot be copied strictly.
fn select_tracks(f: &Mp4File, wanted: Option<&[usize]>, skipped: &mut Vec<SkippedTrack>) -> Result<Vec<usize>, CopyError> {
    let candidates: Vec<usize> = match wanted {
        Some(w) => {
            for &t in w {
                let Some(tr) = f.tracks.get(t) else {
                    return Err(CopyError::new(Code::ContainerUnsupported, format!("the source has no track {t} ({} tracks)", f.tracks.len())));
                };
                if !matches!(tr.kind, TrackKind::Video | TrackKind::Audio) {
                    return Err(CopyError::new(
                        Code::CodecUnsupported,
                        format!("track {t} is a {} track; this phase copies video and audio", kind_name(tr.kind)),
                    ));
                }
            }
            let mut seen = std::collections::BTreeSet::new();
            if let Some(&d) = w.iter().find(|&&t| !seen.insert(t)) {
                return Err(CopyError::new(Code::ContainerUnsupported, format!("track {d} is selected twice")));
            }
            w.to_vec()
        }
        None => {
            let mut v = Vec::new();
            for (i, tr) in f.tracks.iter().enumerate() {
                if matches!(tr.kind, TrackKind::Video | TrackKind::Audio) {
                    v.push(i);
                } else {
                    skipped.push(SkippedTrack { source_track: i, reason: format!("{} track (this phase copies video and audio)", kind_name(tr.kind)) });
                }
            }
            v
        }
    };
    let mut out = Vec::with_capacity(candidates.len());
    for t in candidates {
        let Some(tr) = f.tracks.get(t) else { continue };
        if tr.samples.is_empty() {
            if wanted.is_some() {
                return Err(CopyError::new(Code::ContainerUnsupported, format!("track {t} has no samples")));
            }
            skipped.push(SkippedTrack { source_track: t, reason: "no samples".into() });
            continue;
        }
        check_track(f, t)?;
        out.push(t);
    }
    if out.is_empty() {
        return Err(CopyError::new(Code::ContainerUnsupported, "nothing to copy: the source has no video or audio track with samples"));
    }
    Ok(out)
}

/// Strict-copy preconditions of one track: one sample description, sane sizes and timestamps.
fn check_track(f: &Mp4File, t: usize) -> Result<(), CopyError> {
    let Some(tr) = f.tracks.get(t) else { return Ok(()) };
    if tr.entries.is_empty() {
        return Err(CopyError::new(Code::CodecUnsupported, format!("track {t} has no sample description")));
    }
    if tr.timescale == 0 {
        return Err(CopyError::new(Code::TimestampInvalid, format!("track {t} has a zero timescale")));
    }
    if tr.pcm_chunked {
        // the demuxer merged uncompressed audio into one sample per chunk: the per-sample table
        // cannot be rebuilt exactly from it yet
        return Err(CopyError::new(Code::CodecUnsupported, format!("track {t} is uncompressed audio; Strict Stream Copy of PCM tracks comes later")));
    }
    if tr.entries.first().is_some_and(|e| matches!(&e.format.0, b"encv" | b"enca" | b"enct" | b"encs")) {
        return Err(CopyError::new(Code::CodecUnsupported, format!("track {t} is encrypted; Strict Stream Copy cannot carry its protection scheme yet")));
    }
    // the MP4 writer writes an identity matrix, an enabled track and no alternate group
    if tr.matrix != IDENTITY {
        return Err(CopyError::new(
            Code::ContainerUnsupported,
            format!("track {t} has a display matrix (rotation or mirroring) that the writer cannot carry yet"),
        ));
    }
    if !tr.enabled || tr.alternate_group != 0 {
        return Err(CopyError::new(Code::ContainerUnsupported, format!("track {t} is disabled or in an alternate group, which the writer cannot carry yet")));
    }
    // nor sample groups (`roll`, `sync`, `rap `) and sample dependency flags
    if !tr.sample_groups.is_empty() || !tr.sdtp.is_empty() {
        return Err(CopyError::new(
            Code::ContainerUnsupported,
            format!("track {t} has sample groups or sample dependency flags that the writer cannot carry yet"),
        ));
    }
    // a sample table starts at decode time 0; a later start (a fragment's `tfdt`) is lost
    if tr.samples.first().is_some_and(|s| s.dts != 0) {
        return Err(CopyError::new(Code::TimestampInvalid, format!("track {t} does not start at decode time 0")));
    }
    let mut last_dts = i64::MIN;
    for (i, s) in tr.samples.iter().enumerate() {
        if s.description_index != 1 {
            // the MP4 writer keeps one sample entry per track (SCE-001 v1.4 note 13)
            return Err(CopyError::new(
                Code::ParamsMismatch,
                format!(
                    "track {t} switches to sample description {} at sample {i}; a track with several codec configurations cannot be copied yet",
                    s.description_index
                ),
            ));
        }
        if s.size > MAX_SAMPLE_BYTES {
            return Err(CopyError::new(Code::BitstreamUnsupported, format!("track {t} sample {i} is {} bytes", s.size)));
        }
        if s.dts < last_dts {
            return Err(CopyError::new(Code::TimestampInvalid, format!("track {t}: decode time goes backwards at sample {i}")));
        }
        last_dts = s.dts;
        if s.pts.checked_sub(s.dts).and_then(|o| i32::try_from(o).ok()).is_none() {
            return Err(CopyError::new(Code::TimestampInvalid, format!("track {t} sample {i}: composition offset out of range")));
        }
    }
    Ok(())
}

/// Every selected sample in decode-time order across tracks: (track, sample index).
pub(super) fn interleave(f: &Mp4File, tracks: &[usize]) -> Vec<(usize, usize)> {
    let mut v: Vec<(usize, usize)> = tracks.iter().flat_map(|&t| (0..f.tracks.get(t).map_or(0, |tr| tr.samples.len())).map(move |i| (t, i))).collect();
    let key = |&(t, i): &(usize, usize)| {
        let tr = f.tracks.get(t);
        (tr.and_then(|tr| tr.samples.get(i)).map_or(0, |s| s.dts), tr.map_or(1, |tr| tr.timescale.max(1)))
    };
    v.sort_by(|a, b| {
        let (da, sa) = key(a);
        let (db, sb) = key(b);
        (i128::from(da) * i128::from(sb)).cmp(&(i128::from(db) * i128::from(sa))).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1))
    });
    v
}

/// Copy the selected tracks of `req.source` with unchanged payloads, validate the result and
/// publish it at `req.output`. Nothing is published unless every check passed.
pub fn stream_copy_strict(req: &StrictCopyRequest, progress: &Progress) -> Result<CopyReport, CopyError> {
    let brand = match req.container {
        CopyContainer::Mp4 => Brand::Mp4,
        CopyContainer::Mov => Brand::Mov,
        CopyContainer::MpegTs | CopyContainer::M2ts => {
            return Err(CopyError::new(
                Code::PayloadModificationRequired,
                "a transport stream needs Annex B video and ADTS audio, but MP4 / QuickTime samples are length-prefixed and raw: \
                 Strict Stream Copy cannot write it (Compatible Stream Copy can, SCE-001 §2.1)",
            ));
        }
        CopyContainer::Matroska => return Err(CopyError::new(Code::ContainerUnsupported, "Matroska output is not implemented for Stream Copy yet")),
    };
    if let (Ok(a), Ok(b)) = (req.source.canonicalize(), req.output.canonicalize())
        && a == b
    {
        return Err(CopyError::new(Code::ContainerUnsupported, format!("{}: the output would replace the source", req.output.display())));
    }
    let src = File::open(&req.source).map_err(|e| io(format!("{}: {e}", req.source.display())))?;
    let before = stamp(&src);
    let f = cfa4_isobmff::open(&src).map_err(|e| {
        CopyError::new(
            Code::ContainerUnsupported,
            format!("{}: not a readable MP4 / QuickTime file ({e}); this phase copies from MP4 and QuickTime", req.source.display()),
        )
    })?;
    let mut skipped = Vec::new();
    let tracks = select_tracks(&f, req.tracks.as_deref(), &mut skipped)?;
    if f.timescale == 0 && tracks.iter().any(|&t| f.tracks.get(t).is_some_and(|tr| !tr.edits.is_empty())) {
        return Err(CopyError::new(Code::TimestampInvalid, "the source has edit lists but a zero movie timescale"));
    }
    let order = interleave(&f, &tracks);
    let total = order.len() as u64;
    progress.total.store(total.saturating_mul(2), Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);

    let out = AtomicOutput::create(&req.output, req.overwrite).map_err(|e| io(format!("{}: {e}", req.output.display())))?;
    let mut opts = WriterOptions::new(brand);
    if f.timescale > 0 {
        // edit lists are in the movie timescale: keep it so they copy unchanged
        opts.movie_timescale = f.timescale;
    }
    opts.metadata = f.metadata.entries.clone();
    opts.chapters = f.metadata.chapters.clone();
    let mut w = Mp4Writer::new(out, opts).map_err(io)?;
    let mut map = vec![usize::MAX; f.tracks.len()];
    for &t in &tracks {
        let Some(tr) = f.tracks.get(t) else { continue };
        let Some(entry) = tr.entries.first() else { continue };
        let mut cfg = TrackConfig::new(entry.clone(), tr.timescale);
        cfg.language = tr.language.clone();
        cfg.edits = tr.edits.clone();
        cfg.handler_name = Some(tr.handler_name.clone());
        let k = w.add_track(cfg).map_err(|e| {
            CopyError::new(Code::ContainerUnsupported, format!("track {t} ({}) cannot be written to {:?}: {e}", codec_name(&f, t), req.container))
        })?;
        if let Some(m) = map.get_mut(t) {
            *m = k;
        }
    }
    for (n, &(t, i)) in order.iter().enumerate() {
        if n % CANCEL_EVERY == 0 && progress.cancel.load(Ordering::Relaxed) {
            return Err(CopyError::new(Code::Cancelled, "cancelled"));
        }
        let Some(s) = f.tracks.get(t).and_then(|tr| tr.samples.get(i)).copied() else { continue };
        let data = f.read_sample(&src, t, i).map_err(|e| io(format!("reading track {t} sample {i}: {e}")))?;
        let offset = i32::try_from(s.pts.saturating_sub(s.dts)).map_err(|_| CopyError::new(Code::TimestampInvalid, "composition offset out of range"))?;
        let k = map.get(t).copied().unwrap_or(usize::MAX);
        w.write_sample(k, WriteSample { data: &data, duration: s.duration, composition_offset: offset, is_sync: s.is_sync }).map_err(io)?;
        progress.done.fetch_add(1, Ordering::Relaxed);
    }
    let out = if req.faststart { w.finish_faststart() } else { w.finish() }.map_err(io)?;

    let validation = validate_strict(&f, &src, &tracks, out.file(), progress)?;
    if stamp(&src) != before {
        return Err(CopyError::new(Code::SourceChanged, format!("{} changed while it was being copied", req.source.display())));
    }
    let copied = tracks
        .iter()
        .map(|&t| {
            let tr = f.tracks.get(t);
            CopiedTrack {
                source_track: t,
                kind: tr.map_or("other", |tr| kind_name(tr.kind)).to_string(),
                codec: codec_name(&f, t),
                samples: tr.map_or(0, |tr| tr.samples.len() as u64),
                payload_bytes: tr.map_or(0, |tr| tr.samples.iter().map(|s| u64::from(s.size)).sum()),
            }
        })
        .collect();
    let bytes = out.commit().map_err(|e| CopyError::new(Code::AtomicCommitFailed, format!("{}: {e}", req.output.display())))?;
    Ok(CopyReport { mode: "stream_copy_strict".into(), output: req.output.clone(), container: req.container, bytes, tracks: copied, skipped, validation })
}
