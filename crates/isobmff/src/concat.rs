//! Joining MP4 / MOV files of the same streams one after the other without re-encoding
//! (CFA4-LOSSLESS-MEDIA-TOOLS-001 K1: several cut ranges of one source as one file).
//!
//! Every sample of every part is copied unchanged. Each part keeps its own presentation: the
//! joined track's edit list has one entry per part (merged where a part continues the previous
//! one), so a part's leading gap, its audio priming and a track shorter than the part stay exactly
//! as they were. Parts are refused, never adapted, when their tracks do not match (another track
//! layout, timescale or sample description), when a part is fragmented, or when an edit list is
//! more than a leading gap and one edit at normal speed. The written file is read back and every
//! sample is compared with its part before it is returned.

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

/// Push an edit, merging it with the previous one where it continues it.
fn push_edit(edits: &mut Vec<Edit>, e: Edit, movie_ts: u32, track_ts: u32) {
    if e.segment_duration == 0 {
        return;
    }
    if let Some(prev) = edits.last_mut() {
        if prev.media_time < 0 && e.media_time < 0 {
            prev.segment_duration += e.segment_duration;
            return;
        }
        // exact only when the previous segment is a whole number of media ticks
        let exact = (u128::from(prev.segment_duration) * u128::from(track_ts)) % u128::from(movie_ts.max(1)) == 0;
        if prev.media_time >= 0
            && e.media_time >= 0
            && exact
            && prev.media_time.saturating_add(rescale(prev.segment_duration, movie_ts, track_ts) as i64) == e.media_time
        {
            prev.segment_duration += e.segment_duration;
            return;
        }
    }
    edits.push(e);
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

    // the edit list of every joined track
    let last = files.len() - 1;
    let mut offsets = vec![0i64; first.tracks.len()];
    let mut edits: Vec<Vec<Edit>> = vec![Vec::new(); first.tracks.len()];
    for (k, f) in files.iter().enumerate() {
        for (i, t) in f.tracks.iter().enumerate() {
            let x = &shown[k][i];
            let part_len = lengths[k];
            let lead = rescale(x.lead, f.timescale, movie_ts).min(part_len);
            let mut seg = rescale(x.len, t.timescale, movie_ts).min(part_len - lead);
            // a track that ends within one movie tick of the part's end runs to it (no gap)
            if part_len - lead - seg <= 1 && x.len > 0 {
                seg = part_len - lead;
            }
            push_edit(&mut edits[i], Edit { segment_duration: lead, media_time: -1, media_rate: NORMAL_RATE }, movie_ts, t.timescale);
            if x.len > 0 {
                let media_time = offsets[i].checked_add(x.start).ok_or_else(|| fail("media time overflow"))?;
                push_edit(&mut edits[i], Edit { segment_duration: seg, media_time, media_rate: NORMAL_RATE }, movie_ts, t.timescale);
            }
            let rest = part_len - lead - if x.len > 0 { seg } else { 0 };
            if k != last {
                push_edit(&mut edits[i], Edit { segment_duration: rest, media_time: -1, media_rate: NORMAL_RATE }, movie_ts, t.timescale);
            }
            let media: u64 = t.samples.iter().map(|s| u64::from(s.duration)).sum();
            offsets[i] = offsets[i].checked_add(i64::try_from(media).map_err(|_| fail("media too long"))?).ok_or_else(|| fail("media too long"))?;
        }
    }

    // the parts' chapters, moved to where each part starts
    let mut starts = Vec::with_capacity(files.len());
    let mut at = 0u64;
    let mut chapters: Vec<Chapter> = Vec::new();
    for (f, len) in files.iter().zip(&lengths) {
        starts.push(at);
        let base = rescale(at, movie_ts, HUNDRED_NS);
        for c in &f.metadata.chapters {
            chapters.push(Chapter { start_100ns: c.start_100ns.saturating_add(base), title: c.title.clone() });
        }
        at = at.saturating_add(*len);
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
            let pick = f
                .tracks
                .iter()
                .enumerate()
                .filter(|(i, t)| next[*i] < t.samples.len())
                .min_by_key(|(i, t)| i128::from(t.samples[next[*i]].dts) * i128::from(movie_ts) / i128::from(t.timescale.max(1)));
            let Some((i, t)) = pick else { break };
            let s = t.samples[next[i]];
            let data = f.read_sample(parts[k], i, next[i]).map_err(|e| fail(format!("part {}: {e}", k + 1)))?;
            let offset = i32::try_from(s.pts - s.dts).map_err(|_| fail("composition offset out of range"))?;
            w.write_sample(i, WriteSample { data: &data, duration: s.duration, composition_offset: offset, is_sync: s.is_sync })?;
            next[i] += 1;
        }
    }
    let bytes = w.finish()?.into_inner();
    let samples = verify(&bytes, &files, parts, &edits, &chapters)?;
    Ok(Joined { bytes, timescale: movie_ts, starts, duration: at, samples })
}

/// Read the joined file back: every sample of every part, in order and unchanged, the planned
/// edit lists and chapters, the first part's sample descriptions.
fn verify(bytes: &[u8], files: &[Mp4File], parts: &[&[u8]], edits: &[Vec<Edit>], chapters: &[Chapter]) -> Result<Vec<u64>> {
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
        if first.tracks[i].pcm_chunked {
            // uncompressed sound is regrouped into chunks: its bytes, in order, are what counts
            let mut want = Vec::new();
            for (k, f) in files.iter().enumerate() {
                for si in 0..f.tracks[i].samples.len() {
                    want.extend(f.read_sample(parts[k], i, si)?);
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
            let src = &f.tracks[i];
            for (si, s) in src.samples.iter().enumerate() {
                let o = t.samples.get(j).ok_or_else(|| fail(format!("verification: track {n} is missing samples of part {}", k + 1)))?;
                if o.duration != s.duration || o.pts - o.dts != s.pts - s.dts || o.is_sync != s.is_sync || o.size != s.size {
                    return Err(fail(format!("verification: track {n}, part {}: sample {} differs", k + 1, si + 1)));
                }
                if out.read_sample(bytes, i, j)? != f.read_sample(parts[k], i, si)? {
                    return Err(fail(format!("verification: track {n}, part {}: the data of sample {} differs", k + 1, si + 1)));
                }
                j += 1;
            }
        }
        if j != t.samples.len() {
            return Err(fail(format!("verification: track {n} has more samples than its parts")));
        }
        counts.push(j as u64);
    }
    if out.metadata.chapters.len() != chapters.len() || out.metadata.chapters.iter().zip(chapters).any(|(a, b)| a.start_100ns != b.start_100ns) {
        return Err(fail("verification: the chapters are not as planned"));
    }
    Ok(counts)
}
