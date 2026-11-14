//! Joining MP4 / MOV files of the same streams one after the other without re-encoding
//! (CFA4-LOSSLESS-MEDIA-TOOLS-001 K1: several cut ranges of one source as one file).
//!
//! Every written sample is copied unchanged. Each joined track has a single edit (part 1's leading
//! gap and one edit at normal speed), because CFA4's own reader and many players apply only the
//! first edit of a list. Pictures are copied whole and each part must continue the previous one
//! exactly (no leading gap, no shortened end, the same reordering delay); otherwise the join is
//! refused. Sound starts each part with the sample nearest to where the part begins and ends where
//! it ends (uncompressed sound to the frame), so a part's priming and tail are left out and the
//! sound stays within one picture of the pictures. Parts are refused, never adapted, when their
//! tracks do not match (another track layout, timescale or sample description), when a part is
//! fragmented, or when an edit list is more than a leading gap and one edit at normal speed. The
//! written file is read back and every written sample is compared with its part before it is
//! returned.

use std::io::Cursor;

use crate::codec::{CodecConfig, SampleEntry};
use crate::demux::{Chapter, Edit, Mp4File, Track, TrackKind, open};
use crate::error::{Error, Result};
use crate::mux::{Brand, MAX_CHAPTERS, Mp4Writer, TrackConfig, WriteSample, WriterOptions};

/// Most parts joined into one file.
pub const MAX_PARTS: usize = 1000;

const NORMAL_RATE: i32 = 0x1_0000;
const HUNDRED_NS: u32 = 10_000_000;

/// A joined file and where each part starts in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Joined {
    pub bytes: Vec<u8>,
    /// Movie timescale of the joined file (the first video track's media timescale).
    pub timescale: u32,
    /// Start of each part in the joined file, in [`Joined::timescale`] units.
    pub starts: Vec<u64>,
    /// Presented length of the joined file, in [`Joined::timescale`] units.
    pub duration: u64,
    /// Samples written per track.
    pub samples: Vec<u64>,
    /// Largest distance between where a part's sound is shown and where it belongs, in 100 ns
    /// units (sound starts each part with its nearest sample; at most one picture's duration).
    pub max_sync_error_100ns: u64,
}

fn fail(msg: impl Into<String>) -> Error {
    Error::Mux(format!("join: {}", msg.into()))
}

fn rescale(v: u64, from: u32, to: u32) -> u64 {
    if from == 0 {
        return 0;
    }
    (u128::from(v) * u128::from(to) / u128::from(from)).min(u128::from(u64::MAX)) as u64
}

/// How a track of a part is presented, in that track's media timescale except `lead`.
struct Presented {
    /// Leading empty edit, in the part's movie timescale.
    lead: u64,
    /// First media time shown (raw media time of the part's track).
    start: i64,
    /// Media ticks shown from `start`.
    len: u64,
}

fn presented(t: &Track, movie_ts: u32, part: usize) -> Result<Presented> {
    let end = t.samples.iter().map(|s| s.pts.saturating_add(i64::from(s.duration))).max().unwrap_or(0).max(0) as u64;
    let normal: Vec<&Edit> = t.edits.iter().filter(|e| e.media_time >= 0).collect();
    let leads: Vec<&Edit> = t.edits.iter().filter(|e| e.media_time < 0).collect();
    let shape_ok = match t.edits.as_slice() {
        [] | [_] => true,
        [a, b] => a.media_time < 0 && b.media_time >= 0,
        _ => false,
    };
    if !shape_ok || normal.len() > 1 || normal.iter().any(|e| e.media_rate != NORMAL_RATE) {
        return Err(fail(format!(
            "part {part}: track {} has an edit list of {} entries; only a leading gap and one edit at normal speed can be joined",
            t.id,
            t.edits.len()
        )));
    }
    let lead = leads.first().map_or(0, |e| e.segment_duration);
    let Some(e) = normal.first() else {
        if t.edits.is_empty() {
            return Ok(Presented { lead: 0, start: 0, len: end });
        }
        // only an empty edit: the track shows nothing in this part
        return Ok(Presented { lead, start: 0, len: 0 });
    };
    let natural = end.saturating_sub(e.media_time.max(0) as u64);
    let edited = rescale(e.segment_duration, movie_ts, t.timescale);
    // an edit that ends within one movie tick of the media's end shows the media to its end (the
    // movie timescale is often coarser than the track's)
    let slack = rescale(1, movie_ts, t.timescale).max(1);
    let len = if edited.abs_diff(natural) <= slack { natural } else { edited.min(natural) };
    Ok(Presented { lead, start: e.media_time, len })
}

/// The same stream settings: equal sample descriptions apart from the informative bit rates
/// (`btrt`, the bit rates of an AAC `esds`), which every cut measures anew.
fn same_stream(a: &[SampleEntry], b: &[SampleEntry]) -> bool {
    let plain = |e: &SampleEntry| {
        let mut e = e.clone();
        e.bitrate = None;
        if let CodecConfig::Aac(c) = &mut e.codec {
            (c.max_bitrate, c.avg_bitrate, c.buffer_size) = (0, 0, 0);
        }
        e
    };
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| plain(x) == plain(y))
}

fn check_tracks(first: &Mp4File, part: &Mp4File, k: usize) -> Result<()> {
    if part.fragmented {
        return Err(fail(format!("part {k} is a fragmented file")));
    }
    if part.is_quicktime != first.is_quicktime {
        return Err(fail(format!("part {k} is another container flavour (MP4 and MOV cannot be mixed)")));
    }
    if part.tracks.len() != first.tracks.len() {
        return Err(fail(format!("part {k} has {} tracks, part 1 has {}", part.tracks.len(), first.tracks.len())));
    }
    for (i, (a, b)) in first.tracks.iter().zip(&part.tracks).enumerate() {
        let n = i + 1;
        if !matches!(b.kind, TrackKind::Video | TrackKind::Audio) {
            return Err(fail(format!("part {k}: track {n} is neither video nor audio")));
        }
        if a.kind != b.kind || a.timescale != b.timescale {
            return Err(fail(format!("part {k}: track {n} is another kind or timescale than in part 1")));
        }
        if b.entries.len() != 1 || !same_stream(&a.entries, &b.entries) {
            return Err(fail(format!("part {k}: track {n} has other stream settings (sample description) than in part 1")));
        }
        if b.samples.is_empty() {
            return Err(fail(format!("part {k}: track {n} has no samples")));
        }
        if b.pcm_chunked && !matches!(b.codec(), Some(CodecConfig::Pcm(_))) {
            return Err(fail(format!("part {k}: track {n} is a packed sound format that cannot be joined")));
        }
    }
    Ok(())
}

/// One stretch of a source sample written to the joined track: the whole sample, or for
/// uncompressed sound `frames` frames from frame `from` of the chunk.
#[derive(Clone, Copy)]
struct Piece {
    si: usize,
    from: u64,
    frames: u64,
}

/// The pieces of one track: per part, what is written; how far the written sound is from where
/// the part shows it (worst case, in the track's timescale).
struct TrackPlan {
    pieces: Vec<Vec<Piece>>,
    /// Concatenated media time of the first shown sample (`media_time` of the one edit).
    media_time: i64,
    /// Leading gap of part 1, in the joined movie timescale.
    lead: u64,
    /// End of the last presented media tick written (concatenated media time).
    media_end: i64,
    max_error: u64,
}

/// Bytes of one frame of a chunked uncompressed sound sample.
fn frame_bytes(t: &Track, si: usize) -> Result<u64> {
    let s = &t.samples[si];
    if s.duration == 0 || !u64::from(s.size).is_multiple_of(u64::from(s.duration)) {
        return Err(fail(format!("track {}: a sound chunk is not a whole number of frames", t.id)));
    }
    Ok(u64::from(s.size) / u64::from(s.duration))
}

/// Plan which samples of every part one track writes. The joined track has a single edit (CFA4's
/// own reader and many players apply only the first one), so every part's media must follow the
/// previous part's: pictures are copied whole and must continue exactly; sound starts each part
/// with the sample (for uncompressed sound, the frame) closest to where the part begins and ends it
/// where the part ends, so its priming and tail are left out and it stays within `tol` of the
/// pictures.
#[allow(clippy::too_many_arguments)]
fn plan_track(files: &[Mp4File], shown: &[Vec<Presented>], i: usize, starts: &[u64], lengths: &[u64], movie_ts: u32, tol: u64) -> Result<TrackPlan> {
    let n = i + 1;
    let ts = files[0].tracks[i].timescale;
    let to_track = |v: u64| i128::from(rescale(v, movie_ts, ts));
    // exact for the main picture track; others may round by one movie tick
    let slack: i128 = if ts == movie_ts { 0 } else { i128::from(rescale(1, movie_ts, ts).max(1)) };
    let lead = rescale(shown[0][i].lead, files[0].timescale, movie_ts);
    let lead_tr = to_track(lead);
    let first = &files[0].tracks[i];
    let media_time = shown[0][i].start - first.samples[0].dts;
    if media_time < 0 {
        return Err(fail(format!("track {n}: part 1 shows media before its first sample")));
    }
    let mut written: i128 = 0;
    let mut pieces = Vec::with_capacity(files.len());
    let mut max_error = 0u64;
    let mut media_end: i128 = 0;
    for (k, f) in files.iter().enumerate() {
        let t = &f.tracks[i];
        let x = &shown[k][i];
        if x.len == 0 {
            return Err(fail(format!("part {}: track {n} shows nothing", k + 1)));
        }
        let part_lead = rescale(x.lead, f.timescale, movie_ts);
        let begin = to_track(starts[k] + part_lead);
        let end = to_track(starts[k] + lengths[k]);
        // where the next written media is shown in the joined timeline
        let at = lead_tr + written - i128::from(media_time);
        // error of starting the part with a sample of decode time `d`
        let err = |d: i64| at - begin + i128::from(x.start) - i128::from(d);
        let mut part = Vec::new();
        if t.kind == TrackKind::Video {
            let e = err(t.samples[0].dts);
            if e.abs() > slack {
                return Err(fail(format!(
                    "part {}: its pictures do not continue part {} (a leading gap, a shortened end or another reordering delay); export the ranges as separate files",
                    k + 1,
                    k
                )));
            }
            max_error = max_error.max(e.unsigned_abs() as u64);
            let base = written - i128::from(t.samples[0].dts);
            for (si, s) in t.samples.iter().enumerate() {
                part.push(Piece { si, from: 0, frames: 0 });
                written += i128::from(s.duration);
                media_end = media_end.max(base + i128::from(s.pts) + i128::from(s.duration));
            }
        } else if t.pcm_chunked {
            let lo = i128::from(t.samples[0].pts);
            let hi = t.samples.iter().map(|s| i128::from(s.pts) + i128::from(s.duration)).max().unwrap_or(lo);
            let want = at - begin + i128::from(x.start);
            let p = want.clamp(lo, hi);
            max_error = max_error.max((want - p).unsigned_abs() as u64);
            if (want - p).unsigned_abs() > tol.into() {
                return Err(fail(format!("part {}: the sound of track {n} does not reach the part's start; export the ranges as separate files", k + 1)));
            }
            let mut left = (end - at).min(hi - p).max(0);
            let mut pos = p;
            for (si, s) in t.samples.iter().enumerate() {
                let (s0, s1) = (i128::from(s.pts), i128::from(s.pts) + i128::from(s.duration));
                if left == 0 {
                    break;
                }
                if s1 <= pos {
                    continue;
                }
                let from = (pos - s0).max(0);
                let frames = (s1 - s0 - from).min(left);
                frame_bytes(t, si)?;
                part.push(Piece { si, from: from as u64, frames: frames as u64 });
                written += frames;
                left -= frames;
                pos = s0 + from + frames;
            }
        } else {
            // the sample whose start is closest to where the part begins
            let j = t.samples.iter().enumerate().min_by_key(|(_, s)| err(s.dts).abs()).map_or(0, |(j, _)| j);
            let e = err(t.samples[j].dts);
            if e.unsigned_abs() > tol.into() {
                return Err(fail(format!(
                    "part {}: the sound of track {n} would be {} ms off the pictures; export the ranges as separate files",
                    k + 1,
                    e.unsigned_abs() * 1000 / u128::from(ts.max(1))
                )));
            }
            max_error = max_error.max(e.unsigned_abs() as u64);
            for (si, s) in t.samples.iter().enumerate().skip(j) {
                let mid = lead_tr + written - i128::from(media_time) + i128::from(s.duration) / 2;
                if mid >= end {
                    break;
                }
                part.push(Piece { si, from: 0, frames: 0 });
                written += i128::from(s.duration);
            }
        }
        media_end = media_end.max(written);
        pieces.push(part);
    }
    let media_end = i64::try_from(media_end).map_err(|_| fail("media too long"))?;
    Ok(TrackPlan { pieces, media_time, lead, media_end, max_error })
}

/// The bytes a piece writes.
fn piece_data(f: &Mp4File, part: &[u8], i: usize, p: Piece) -> Result<Vec<u8>> {
    let mut data = f.read_sample(part, i, p.si)?;
    let t = &f.tracks[i];
    if t.pcm_chunked {
        let fb = frame_bytes(t, p.si)?;
        let a = usize::try_from(p.from * fb).map_err(|_| fail("sound chunk too large"))?;
        let b = usize::try_from((p.from + p.frames) * fb).map_err(|_| fail("sound chunk too large"))?;
        data = data.get(a..b).ok_or_else(|| fail("sound chunk shorter than its frames"))?.to_vec();
    }
    Ok(data)
}

/// Join `parts` (whole MP4 / MOV files) one after the other.
pub fn concat(parts: &[&[u8]]) -> Result<Joined> {
    if parts.is_empty() {
        return Err(fail("no parts"));
    }
    if parts.len() > MAX_PARTS {
        return Err(fail(format!("{} parts; at most {MAX_PARTS}", parts.len())));
    }
    let files: Vec<Mp4File> = parts.iter().enumerate().map(|(k, p)| open(*p).map_err(|e| fail(format!("part {}: {e}", k + 1)))).collect::<Result<_>>()?;
    let first = &files[0];
    for (k, f) in files.iter().enumerate() {
        check_tracks(first, f, k + 1)?;
    }
    let video = first.tracks.iter().position(|t| t.kind == TrackKind::Video);
    let movie_ts = video.map_or_else(|| first.tracks.iter().map(|t| t.timescale).max().unwrap_or(1000), |v| first.tracks[v].timescale).max(1);

    // how every track of every part is shown, and each part's length in the joined movie
    let mut shown: Vec<Vec<Presented>> = Vec::with_capacity(files.len());
    let mut lengths = Vec::with_capacity(files.len());
    for (k, f) in files.iter().enumerate() {
        let p: Vec<Presented> = f.tracks.iter().map(|t| presented(t, f.timescale, k + 1)).collect::<Result<_>>()?;
        let span = |(t, x): (&Track, &Presented)| rescale(x.lead, f.timescale, movie_ts) + rescale(x.len, t.timescale, movie_ts);
        let len = match video {
            Some(v) => span((&f.tracks[v], &p[v])),
            None => f.tracks.iter().zip(&p).map(span).max().unwrap_or(0),
        };
        if len == 0 {
            return Err(fail(format!("part {} shows nothing", k + 1)));
        }
        shown.push(p);
        lengths.push(len);
    }
    let mut starts = Vec::with_capacity(files.len());
    let mut total = 0u64;
    for len in &lengths {
        starts.push(total);
        total = total.saturating_add(*len);
    }

    // the samples every track writes and its single edit
    let frame = video.map(|v| u64::from(first.tracks[v].samples[0].duration));
    let mut plans = Vec::with_capacity(first.tracks.len());
    let mut edits: Vec<Vec<Edit>> = Vec::with_capacity(first.tracks.len());
    let mut max_error_100ns = 0u64;
    for (i, t) in first.tracks.iter().enumerate() {
        let tol = match frame {
            Some(d) => rescale(d, movie_ts, t.timescale),
            None => u64::from(t.samples[0].duration),
        }
        .max(1);
        let plan = plan_track(&files, &shown, i, &starts, &lengths, movie_ts, tol)?;
        max_error_100ns = max_error_100ns.max(rescale(plan.max_error, t.timescale, HUNDRED_NS));
        let mut e = Vec::new();
        if plan.lead > 0 {
            e.push(Edit { segment_duration: plan.lead, media_time: -1, media_rate: NORMAL_RATE });
        }
        let room = total - plan.lead.min(total);
        let media = rescale(u64::try_from(plan.media_end - plan.media_time).unwrap_or(0), t.timescale, movie_ts);
        // a track that ends within one movie tick of the end runs to it (no gap)
        let seg = if room.abs_diff(media) <= 1 { room } else { media.min(room) };
        if seg > 0 {
            e.push(Edit { segment_duration: seg, media_time: plan.media_time, media_rate: NORMAL_RATE });
        }
        edits.push(e);
        plans.push(plan);
    }

    // the parts' chapters, moved to where each part starts
    let mut chapters: Vec<Chapter> = Vec::new();
    for (f, at) in files.iter().zip(&starts) {
        let base = rescale(*at, movie_ts, HUNDRED_NS);
        for c in &f.metadata.chapters {
            chapters.push(Chapter { start_100ns: c.start_100ns.saturating_add(base), title: c.title.clone() });
        }
    }
    chapters.truncate(MAX_CHAPTERS);

    let brand = if first.is_quicktime { Brand::Mov } else { Brand::Mp4 };
    let mut opts = WriterOptions::new(brand);
    opts.movie_timescale = movie_ts;
    opts.metadata = first.metadata.entries.clone();
    opts.chapters = chapters.clone();
    let mut w = Mp4Writer::new(Cursor::new(Vec::new()), opts)?;
    for (i, t) in first.tracks.iter().enumerate() {
        let mut cfg = TrackConfig::new(t.entries[0].clone(), t.timescale);
        cfg.language = t.language.clone();
        cfg.edits = edits[i].clone();
        cfg.handler_name = (!t.handler_name.is_empty()).then(|| t.handler_name.clone());
        cfg.matrix = Some(t.matrix);
        cfg.enabled = t.enabled;
        cfg.alternate_group = t.alternate_group;
        cfg.volume = Some(t.volume);
        w.add_track(cfg)?;
    }
    // samples interleaved by decode time within each part
    for (k, f) in files.iter().enumerate() {
        let mut next = vec![0usize; f.tracks.len()];
        loop {
            let pick = (0..f.tracks.len()).filter(|i| next[*i] < plans[*i].pieces[k].len()).min_by_key(|i| {
                let t = &f.tracks[*i];
                let p = plans[*i].pieces[k][next[*i]];
                (i128::from(t.samples[p.si].dts) + i128::from(p.from)) * i128::from(movie_ts) / i128::from(t.timescale.max(1))
            });
            let Some(i) = pick else { break };
            let p = plans[i].pieces[k][next[i]];
            let s = f.tracks[i].samples[p.si];
            let data = piece_data(f, parts[k], i, p).map_err(|e| fail(format!("part {}: {e}", k + 1)))?;
            let offset = i32::try_from(s.pts - s.dts).map_err(|_| fail("composition offset out of range"))?;
            w.write_sample(i, WriteSample { data: &data, duration: s.duration, composition_offset: offset, is_sync: s.is_sync })?;
            next[i] += 1;
        }
    }
    let bytes = w.finish()?.into_inner();
    let samples = verify(&bytes, &files, parts, &plans, &edits, &chapters)?;
    Ok(Joined { bytes, timescale: movie_ts, starts, duration: total, samples, max_sync_error_100ns: max_error_100ns })
}

/// Read the joined file back: every planned sample of every part, in order and unchanged, the
/// planned single edit of every track, the chapters, the first part's sample descriptions.
fn verify(bytes: &[u8], files: &[Mp4File], parts: &[&[u8]], plans: &[TrackPlan], edits: &[Vec<Edit>], chapters: &[Chapter]) -> Result<Vec<u64>> {
    let out = open(bytes).map_err(|e| fail(format!("the joined file cannot be read back: {e}")))?;
    let first = &files[0];
    if out.tracks.len() != first.tracks.len() {
        return Err(fail("verification: the joined file has another number of tracks"));
    }
    let mut counts = Vec::with_capacity(out.tracks.len());
    for (i, t) in out.tracks.iter().enumerate() {
        let n = i + 1;
        if !same_stream(&t.entries, &first.tracks[i].entries) || t.timescale != first.tracks[i].timescale {
            return Err(fail(format!("verification: track {n} has other stream settings")));
        }
        if t.edits != edits[i] {
            return Err(fail(format!("verification: track {n} has another edit list than planned")));
        }
        let lead = rescale(plans[i].lead, out.timescale, t.timescale) as i64;
        if !edits[i].iter().any(|e| e.media_time >= 0) || t.edit_offset != lead - plans[i].media_time {
            return Err(fail(format!("verification: track {n} is not shown from where planned")));
        }
        if first.tracks[i].pcm_chunked {
            // uncompressed sound is regrouped into chunks: its bytes, in order, are what counts
            let mut want = Vec::new();
            for (k, f) in files.iter().enumerate() {
                for p in &plans[i].pieces[k] {
                    want.extend(piece_data(f, parts[k], i, *p)?);
                }
            }
            let mut got = Vec::with_capacity(want.len());
            for j in 0..t.samples.len() {
                got.extend(out.read_sample(bytes, i, j)?);
            }
            if got != want {
                return Err(fail(format!("verification: the sound of track {n} differs")));
            }
            counts.push(t.samples.len() as u64);
            continue;
        }
        let mut j = 0usize;
        for (k, f) in files.iter().enumerate() {
            for p in &plans[i].pieces[k] {
                let s = &f.tracks[i].samples[p.si];
                let o = t.samples.get(j).ok_or_else(|| fail(format!("verification: track {n} is missing samples of part {}", k + 1)))?;
                if o.duration != s.duration || o.pts - o.dts != s.pts - s.dts || o.is_sync != s.is_sync || o.size != s.size {
                    return Err(fail(format!("verification: track {n}, part {}: sample {} differs", k + 1, p.si + 1)));
                }
                if out.read_sample(bytes, i, j)? != f.read_sample(parts[k], i, p.si)? {
                    return Err(fail(format!("verification: track {n}, part {}: the data of sample {} differs", k + 1, p.si + 1)));
                }
                j += 1;
            }
        }
        if j != t.samples.len() {
            return Err(fail(format!("verification: track {n} has more samples than planned")));
        }
        counts.push(j as u64);
    }
    if out.metadata.chapters.len() != chapters.len() || out.metadata.chapters.iter().zip(chapters).any(|(a, b)| a.start_100ns != b.start_100ns) {
        return Err(fail("verification: the chapters are not as planned"));
    }
    Ok(counts)
}
