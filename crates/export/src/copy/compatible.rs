//! Compatible Stream Copy executor (CFA4-SCE-001 v1.5 §2.2, §10, §12.4): MP4 / QuickTime H.264,
//! HEVC and AAC → MPEG-TS / M2TS through the allowlisted adapters of [`super::adapter`] and the
//! native transport stream muxer, which leaves the payloads it is given unchanged.
//!
//! Nothing is re-encoded. The output is published only after the Compatible Validator has read it
//! back with the native demuxer and found, for every access unit, the source's NAL units (or AAC
//! frame) once the allowlisted framing is removed, with the same timestamps and sync flags
//! (`BITSTREAM_EQUIVALENT: PASS`).
//!
//! MP4 and QuickTime targets need no adapter: they are a Strict Copy, which is also a valid
//! Compatible Copy, and are handed to [`super::stream_copy_strict`].

use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::atomic::Ordering;

use cfa4_isobmff::{Mp4File, Track};
use cfa4_mpegts::{MuxCodec, MuxFormat, Muxer};

use super::adapter::{ALLOWLIST_VERSION, Adapter, AnnexB};
use super::strict::{CANCEL_EVERY, codec_name, interleave, io, kind_name, select_tracks, stamp};
use super::{CopiedTrack, CopyContainer, CopyError, CopyErrorCode as Code, CopyReport, StrictCopyRequest, StrictValidation, Transformation};
use crate::Progress;
use crate::atomic::AtomicOutput;

/// 90 kHz transport stream clock.
const TS_HZ: i128 = 90_000;
/// 27 MHz system clock ticks per 90 kHz tick.
const SYS_PER_90K: i128 = 300;
/// Most AAC frames an edit list may hide at the start as encoder priming (2112 samples, the
/// longest common delay, is a little over two 1024-sample frames).
const MAX_PRIMING_FRAMES: i64 = 3;

/// How a source track's media times map onto the presentation timeline.
#[derive(Clone, Copy, Debug)]
pub(super) struct EditPlan {
    /// Leading empty edit, movie timescale.
    empty: i128,
    /// Media time shown at the start of the media edit, track timescale.
    media_time: i128,
    movie_ts: i128,
    track_ts: i128,
    /// Media time the edit list hides before its start (AAC encoder priming only), track
    /// timescale.
    pub(super) priming: i64,
}

impl EditPlan {
    /// `t` (track timescale) on the presentation timeline, in 90 kHz rounded to nearest.
    fn at_90k(&self, t: i64) -> i128 {
        let num = (i128::from(t) - self.media_time) * self.movie_ts * TS_HZ + self.empty * self.track_ts * TS_HZ;
        round_div(num, self.movie_ts * self.track_ts)
    }
}

/// `n / d` rounded to nearest (`d > 0`).
fn round_div(n: i128, d: i128) -> i128 {
    if n >= 0 { (n + d / 2) / d } else { -((-n + d / 2) / d) }
}

/// A source track's edit list as a transport stream can carry it: one leading empty edit and one
/// media edit at normal rate that shows the media from its first sample (AAC: after at most
/// [`MAX_PRIMING_FRAMES`] of encoder priming, which the copy keeps and reports) to its end. A
/// transport stream has no edit list, so anything else would show what the source hides or lose
/// what it shows: refused.
pub(super) fn edit_plan(f: &Mp4File, t: usize, tr: &Track, aac: bool) -> Result<EditPlan, CopyError> {
    let refuse = |why: String| Err(CopyError::new(Code::TimestampInvalid, format!("track {t}: {why}; a transport stream cannot carry that edit list")));
    let track_ts = i128::from(tr.timescale);
    if track_ts == 0 {
        return refuse("a zero media timescale".into());
    }
    let mut plan = EditPlan { empty: 0, media_time: 0, movie_ts: 1, track_ts, priming: 0 };
    if tr.edits.is_empty() {
        return Ok(plan);
    }
    plan.movie_ts = i128::from(f.timescale);
    if plan.movie_ts == 0 {
        return refuse("a zero movie timescale".into());
    }
    let mut edits = tr.edits.as_slice();
    if let Some((e, rest)) = edits.split_first()
        && e.media_time == -1
    {
        plan.empty = i128::from(e.segment_duration);
        edits = rest;
    }
    let [e] = edits else { return refuse(if edits.is_empty() { "no media edit".into() } else { "several media edits".into() }) };
    if e.media_rate != 0x1_0000 {
        return refuse("the edit plays at another rate".into());
    }
    if e.media_time < 0 {
        return refuse(format!("media time {}", e.media_time));
    }
    plan.media_time = i128::from(e.media_time);
    let first = tr.samples.iter().map(|s| i128::from(s.pts)).min().unwrap_or(0);
    let end = tr.samples.iter().map(|s| i128::from(s.pts) + i128::from(s.duration)).max().unwrap_or(0);
    let longest = tr.samples.iter().map(|s| i128::from(s.duration)).max().unwrap_or(0);
    // the start: nothing the edit hides may be shown, except AAC encoder priming
    let hidden = plan.media_time - first;
    if hidden > 0 {
        if !aac || hidden > longest * i128::from(MAX_PRIMING_FRAMES) {
            return refuse(format!("the edit hides the first {hidden} / {track_ts} s of the media, which would be shown"));
        }
        plan.priming = i64::try_from(hidden).unwrap_or(i64::MAX);
    }
    // the end: the edit must run to the end of the media (within one sample), or the cut-off end
    // would be shown
    let left = (end - plan.media_time) * plan.movie_ts;
    let shown = i128::from(e.segment_duration) * track_ts;
    if shown + longest * plan.movie_ts < left {
        return refuse("the edit ends before the media does".into());
    }
    Ok(plan)
}

/// Transport stream timestamps of every copied sample: `stamps[k][i]` = (pts, dts) in 90 kHz,
/// shifted so that the earliest decode time is zero.
pub(super) fn stamps(plans: &[EditPlan], f: &Mp4File, tracks: &[usize]) -> Result<Vec<Vec<(i64, i64)>>, CopyError> {
    let bad = |t: usize, why: &str| CopyError::new(Code::TimestampInvalid, format!("track {t}: {why}"));
    let mut all: Vec<Vec<(i128, i128)>> = Vec::with_capacity(tracks.len());
    for (&t, plan) in tracks.iter().zip(plans) {
        let Some(tr) = f.tracks.get(t) else { return Err(CopyError::new(Code::ContainerUnsupported, format!("no track {t}"))) };
        let v: Vec<(i128, i128)> = tr.samples.iter().map(|s| (plan.at_90k(s.pts), plan.at_90k(s.dts))).collect();
        if v.iter().any(|&(p, d)| d > p) {
            return Err(bad(t, "a sample is decoded after it is shown"));
        }
        if v.windows(2).any(|w| w[1].1 < w[0].1) {
            return Err(bad(t, "decode times go backwards"));
        }
        all.push(v);
    }
    let base = all.iter().flatten().map(|&(_, d)| d).min().unwrap_or(0);
    let mut out = Vec::with_capacity(all.len());
    for (v, &t) in all.iter().zip(tracks) {
        let mut w = Vec::with_capacity(v.len());
        for &(p, d) in v {
            let (p, d) = (p - base, d - base);
            if p >= 1 << 40 {
                return Err(bad(t, "a timestamp is too late for the transport stream"));
            }
            w.push((p as i64, d as i64));
        }
        out.push(w);
    }
    Ok(out)
}

/// Most bytes `a` can make of a `size`-byte sample (start codes, delimiter, parameter sets).
fn unit_bound(a: &Adapter, size: u32, sync: bool) -> u64 {
    let size = u64::from(size);
    match a {
        Adapter::AnnexB(b) => {
            let ls = b.length_size.clamp(1, 4) as u64;
            // every NAL unit takes at least ls + 1 bytes and gains 4 − ls
            let growth = size / (ls + 1) * (4 - ls);
            let params: u64 = if sync { b.parameter_sets.iter().map(|p| p.len() as u64 + 4).sum() } else { 0 };
            size + growth + 8 + params
        }
        Adapter::Adts(_) => size + 7,
    }
}

/// The multiplex rate: the lowest at which the muxer's system clock delivers every access unit
/// before its decode time (simulated with an upper bound of every unit's packets and the tables
/// sent before it), plus 10 %.
pub(super) fn mux_rate(f: &Mp4File, tracks: &[usize], adapters: &[Adapter], ts: &[Vec<(i64, i64)>], format: MuxFormat) -> Result<u64, CopyError> {
    let mut units: Vec<(i128, u64, bool)> = Vec::new();
    for (k, &t) in tracks.iter().enumerate() {
        let (Some(tr), Some(st), Some(a)) = (f.tracks.get(t), ts.get(k), adapters.get(k)) else { continue };
        for (s, &(_, d)) in tr.samples.iter().zip(st) {
            let pes = unit_bound(a, s.size, s.is_sync) + 19;
            // at least 176 payload bytes in every packet but the last (PCR / flags / stuffing)
            units.push((i128::from(d), pes.div_ceil(176), matches!(a, Adapter::AnnexB(_)) && s.is_sync));
        }
    }
    units.sort_by_key(|u| u.0);
    let packet: i128 = if format == MuxFormat::M2ts { 192 } else { 188 };
    let delay = i128::from(Muxer::<Vec<u8>>::DELAY);
    let fits = |rate: i128| {
        // 27 MHz ticks per packet, rounded up (the muxer rounds down)
        let tick = (packet * 27_000_000 + rate - 1) / rate;
        let (mut clock, mut psi): (i128, Option<i128>) = (0, None);
        for &(dts, packets, key) in &units {
            clock = clock.max(dts * SYS_PER_90K);
            if key || psi.is_none_or(|p| clock - p >= 100 * 27_000) {
                psi = Some(clock);
                clock += 2 * tick;
            }
            clock += i128::from(packets) * tick;
            if clock > (dts + delay) * SYS_PER_90K {
                return false;
            }
        }
        true
    };
    // bytes per second, as the muxer counts them (program_mux_rate: 50 bytes/s units, 22 bits)
    let (mut lo, hi) = (50 * 64_i128, 50 * ((1_i128 << 22) - 1));
    if !fits(hi) {
        return Err(CopyError::new(Code::ContainerUnsupported, "the streams peak above what a transport stream can deliver within its 0.7 s buffer"));
    }
    let mut hi_ok = hi;
    while lo < hi_ok {
        let mid = (lo + hi_ok) / 2;
        if fits(mid) { hi_ok = mid } else { lo = mid + 1 }
    }
    let rate = (hi_ok * 11 / 10).min(hi).max(125_000);
    Ok(rate as u64 * 8)
}

/// mpegts' random access over a file.
struct TsRead<'a>(&'a File);

impl cfa4_mpegts::ByteSource for TsRead<'_> {
    fn len(&self) -> u64 {
        cfa4_isobmff::ByteSource::len(self.0)
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        cfa4_isobmff::ByteSource::read_at(self.0, offset, buf)
    }
}

/// Copy the selected tracks of `req.source` into an MPEG-TS / M2TS file through the allowlisted
/// adapters, validate it and publish it at `req.output` (MP4 / QuickTime targets: a Strict Copy).
pub fn stream_copy_compatible(req: &StrictCopyRequest, progress: &Progress) -> Result<CopyReport, CopyError> {
    let format = match req.container {
        CopyContainer::MpegTs => MuxFormat::Ts,
        CopyContainer::M2ts => MuxFormat::M2ts,
        CopyContainer::Mp4 | CopyContainer::Mov => {
            let mut r = super::stream_copy_strict(req, progress)?;
            r.mode = "stream_copy_compatible".into();
            return Ok(r);
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
    let tracks = select_tracks(&f, req.tracks.as_deref(), &mut skipped, false)?;
    // allowlist eligibility of every track before anything is written (§10 steps 3–5)
    let mut adapters = Vec::with_capacity(tracks.len());
    let mut codecs = Vec::with_capacity(tracks.len());
    for &t in &tracks {
        let codec = f.tracks.get(t).and_then(|tr| tr.codec()).ok_or_else(|| CopyError::new(Code::CodecUnsupported, format!("track {t} has no codec")))?;
        let a = Adapter::for_codec(codec).map_err(|e| CopyError::new(e.code, format!("track {t}: {}", e.message)))?;
        codecs.push(match &a {
            Adapter::AnnexB(b) if b.hevc => MuxCodec::Hevc,
            Adapter::AnnexB(_) => MuxCodec::H264,
            Adapter::Adts(_) => MuxCodec::AacAdts,
        });
        adapters.push(a);
    }
    let mut plans = Vec::with_capacity(tracks.len());
    for (&t, a) in tracks.iter().zip(&adapters) {
        let Some(tr) = f.tracks.get(t) else { return Err(CopyError::new(Code::ContainerUnsupported, format!("no track {t}"))) };
        plans.push(edit_plan(&f, t, tr, matches!(a, Adapter::Adts(_)))?);
    }
    let ts = stamps(&plans, &f, &tracks)?;
    let rate = mux_rate(&f, &tracks, &adapters, &ts, format)?;
    let order = interleave(&f, &tracks);
    progress.total.store((order.len() as u64).saturating_mul(2), Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);

    let out = AtomicOutput::create(&req.output, req.overwrite).map_err(|e| io(format!("{}: {e}", req.output.display())))?;
    let mut m = Muxer::new(BufWriter::new(out), format, &codecs, rate)
        .map_err(|e| CopyError::new(Code::ContainerUnsupported, format!("the transport stream muxer refused the streams: {e}")))?;
    for (n, &(t, i)) in order.iter().enumerate() {
        if n % CANCEL_EVERY == 0 && progress.cancel.load(Ordering::Relaxed) {
            return Err(CopyError::new(Code::Cancelled, "cancelled"));
        }
        let Some(k) = tracks.iter().position(|&x| x == t) else { continue };
        let (Some(s), Some(a), Some(&(pts, dts))) = (f.tracks.get(t).and_then(|tr| tr.samples.get(i)), adapters.get(k), ts.get(k).and_then(|v| v.get(i)))
        else {
            continue;
        };
        let data = f.read_sample(&src, t, i).map_err(|e| io(format!("reading track {t} sample {i}: {e}")))?;
        let unit = a.apply(&data, s.is_sync).map_err(|e| CopyError::new(e.code, format!("track {t} sample {i}: {}", e.message)))?;
        m.write(k, &unit, pts, (dts != pts).then_some(dts), s.is_sync).map_err(|e| mux_error(e, format!("track {t} sample {i}")))?;
        progress.done.fetch_add(1, Ordering::Relaxed);
    }
    let mut w = m.finish().map_err(|e| mux_error(e, "finishing the transport stream".into()))?;
    w.flush().map_err(io)?;
    let out = w.into_inner().map_err(|e| io(e.error().to_string()))?;

    let validation = validate_compatible(&f, &src, &tracks, &adapters, out.file(), format, progress)?;
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
    let transformations = tracks
        .iter()
        .zip(&adapters)
        .zip(&plans)
        .map(|((&t, a), plan)| Transformation {
            source_track: t,
            adapter: a.allowed().id.to_string(),
            allowlist_version: ALLOWLIST_VERSION,
            notes: if plan.priming > 0 {
                vec![format!(
                    "AAC_PRIMING_KEPT: the source edit list hides {} / {} s of encoder priming; a transport stream has no edit list, so those samples are kept, \
                     presented just before the start on the same timeline",
                    plan.priming, plan.track_ts
                )]
            } else {
                Vec::new()
            },
        })
        .collect();
    let bytes = out.commit().map_err(|e| CopyError::new(Code::AtomicCommitFailed, format!("{}: {e}", req.output.display())))?;
    Ok(CopyReport {
        mode: "stream_copy_compatible".into(),
        output: req.output.clone(),
        container: req.container,
        bytes,
        tracks: copied,
        skipped,
        validation,
        transformations,
    })
}

/// A muxer refusal: I/O, or a stream it cannot carry (timestamps and framing are checked before,
/// so this is not expected).
fn mux_error(e: cfa4_mpegts::MuxError, at: String) -> CopyError {
    match e {
        cfa4_mpegts::MuxError::Io(e) => io(format!("{at}: {e}")),
        cfa4_mpegts::MuxError::Unsupported(m) => CopyError::new(Code::ContainerUnsupported, format!("{at}: the transport stream muxer refused it: {m}")),
    }
}

fn fail(message: String) -> CopyError {
    CopyError::new(Code::CompatibleValidationFailed, message)
}

/// The Compatible Validator (§10, §12.6/7): demux the written stream and prove, without the
/// copier's own computations, that every access unit is its source sample once the allowlisted
/// framing is removed, presented at the time the source's edit list presents it, and delivered
/// before its decode time.
fn validate_compatible(
    f: &Mp4File,
    src: &File,
    tracks: &[usize],
    adapters: &[Adapter],
    out: &File,
    format: MuxFormat,
    progress: &Progress,
) -> Result<StrictValidation, CopyError> {
    let read = TsRead(out);
    let g = cfa4_mpegts::open(&read).map_err(|e| fail(format!("the written transport stream cannot be read back: {e}")))?;
    if !g.warnings.is_empty() {
        return Err(fail(format!("the written transport stream has problems: {}", g.warnings.join("; "))));
    }
    if g.streams.len() != tracks.len() {
        return Err(fail(format!("the output has {} streams, {} were copied", g.streams.len(), tracks.len())));
    }
    // the presentation timeline of the source, from its edit lists, and of the output
    let mut want: Vec<Vec<(i128, i128)>> = Vec::with_capacity(tracks.len());
    let mut streams = Vec::with_capacity(tracks.len());
    for (k, &t) in tracks.iter().enumerate() {
        let tr = f.tracks.get(t).ok_or_else(|| fail(format!("track {t} is missing from the source")))?;
        want.push(source_timeline(f, tr, matches!(adapters.get(k), Some(Adapter::Adts(_)))).map_err(|e| fail(format!("track {t}: {e}")))?);
        // the muxer gives stream k PID 0x100 + k
        let pid = 0x100 + u16::try_from(k).unwrap_or(u16::MAX - 0x100);
        let gk = g
            .streams
            .iter()
            .position(|st| st.id == cfa4_mpegts::StreamId::Pid(pid))
            .ok_or_else(|| fail(format!("track {t}: no stream on PID {pid:#x} in the output")))?;
        streams.push((gk, pid));
    }
    let want_base = want.iter().flatten().map(|&(_, d)| d).min().unwrap_or(0);
    let got_base = streams.iter().filter_map(|&(gk, _)| g.streams.get(gk)).flat_map(|st| &st.units).filter_map(|u| u.dts.or(u.pts)).min().unwrap_or(0);

    let (mut packets, mut payload_bytes, mut n) = (0u64, 0u64, 0usize);
    for (k, &t) in tracks.iter().enumerate() {
        let (Some(tr), Some(&(gk, _)), Some(a), Some(want)) = (f.tracks.get(t), streams.get(k), adapters.get(k), want.get(k)) else {
            return Err(fail(format!("track {t} is missing from the output")));
        };
        let Some(st) = g.streams.get(gk) else { return Err(fail(format!("track {t} is missing from the output"))) };
        if st.units.len() != tr.samples.len() {
            return Err(fail(format!("track {t}: {} samples in the source, {} access units in the output", tr.samples.len(), st.units.len())));
        }
        let asc = match tr.codec() {
            Some(cfa4_isobmff::CodecConfig::Aac(c)) => c.asc.as_slice(),
            _ => &[],
        };
        for (i, ((s, u), &(wp, wd))) in tr.samples.iter().zip(&st.units).zip(want).enumerate() {
            if n % CANCEL_EVERY == 0 && progress.cancel.load(Ordering::Relaxed) {
                return Err(CopyError::new(Code::Cancelled, "cancelled"));
            }
            n += 1;
            let (Some(p), d) = (u.pts, u.dts.or(u.pts)) else { return Err(fail(format!("track {t} sample {i}: no presentation time"))) };
            let d = d.unwrap_or(p);
            // one 90 kHz tick of rounding
            if (i128::from(p - got_base) - (wp - want_base)).abs() > 1 || (i128::from(d - got_base) - (wd - want_base)).abs() > 1 {
                return Err(fail(format!("track {t} sample {i}: not presented when the source presents it")));
            }
            let source = f.read_sample(src, t, i).map_err(|e| io(format!("reading source track {t} sample {i}: {e}")))?;
            let unit = g.read_unit(&read, gk, i).map_err(|e| fail(format!("reading output stream {k} unit {i}: {e}")))?;
            match a {
                Adapter::AnnexB(b) => {
                    if u.key != s.is_sync {
                        return Err(fail(format!("track {t} sample {i}: the sync flag changed")));
                    }
                    annexb_equivalent(b, &source, &unit, s.is_sync).map_err(|e| fail(format!("track {t} sample {i}: {e}")))?;
                }
                Adapter::Adts(_) => {
                    if !adts_equivalent(asc, &source, &unit) {
                        return Err(fail(format!("track {t} sample {i}: the ADTS frame read back is not the source frame under its configuration's header")));
                    }
                }
            }
            packets += 1;
            payload_bytes += source.len() as u64;
            progress.done.fetch_add(1, Ordering::Relaxed);
        }
    }
    let pids: Vec<u16> = streams.iter().map(|&(_, pid)| pid).collect();
    check_arrival(out, if format == MuxFormat::M2ts { 192 } else { 188 }, &pids)?;
    Ok(StrictValidation {
        result: "BITSTREAM_EQUIVALENT: PASS".into(),
        packets,
        payload_bytes,
        not_run: vec![
            "VALIDATION_DECODE_NOT_AVAILABLE: the copied streams were not decoded (every access unit is the source's once the allowlisted framing is removed)"
                .into(),
        ],
    })
}

/// The source's presentation (pts, dts) of every sample in 90 kHz (unrounded origin), from the
/// edit list itself; an error when the edit list hides media the copy would show (beyond AAC
/// encoder priming) or cannot be carried.
pub(super) fn source_timeline(f: &Mp4File, tr: &Track, aac: bool) -> Result<Vec<(i128, i128)>, String> {
    let mts = i128::from(tr.timescale);
    if mts == 0 {
        return Err("zero media timescale".into());
    }
    if !tr.edits.is_empty() {
        let movie = i128::from(f.timescale);
        let empty: i128 = tr.edits.iter().take_while(|e| e.media_time == -1).map(|e| i128::from(e.segment_duration)).sum();
        let media: Vec<_> = tr.edits.iter().filter(|e| e.media_time != -1).collect();
        let ([e], true) = (media.as_slice(), movie > 0 && tr.edits.iter().skip_while(|e| e.media_time == -1).all(|e| e.media_time != -1)) else {
            return Err("an edit list a transport stream cannot carry".into());
        };
        let first = tr.samples.iter().map(|s| i128::from(s.pts)).min().unwrap_or(0);
        let longest = tr.samples.iter().map(|s| i128::from(s.duration)).max().unwrap_or(0);
        let hidden = i128::from(e.media_time) - first;
        if e.media_rate != 0x1_0000 || (hidden > 0 && !(aac && hidden <= longest * i128::from(MAX_PRIMING_FRAMES))) {
            return Err("the source edit list hides media the output shows".into());
        }
        let end = tr.samples.iter().map(|s| i128::from(s.pts) + i128::from(s.duration)).max().unwrap_or(0);
        if i128::from(e.segment_duration) * mts + longest * movie < (end - i128::from(e.media_time)) * movie {
            return Err("the source edit list ends before the media the output shows".into());
        }
        // t → (t − media_time) / mts + empty / movie
        let den = mts * movie;
        let offset_num = empty * mts - i128::from(e.media_time) * movie;
        let scale = movie;
        return Ok(tr
            .samples
            .iter()
            .map(|s| {
                let at = |x: i64| round_div((i128::from(x) * scale + offset_num) * TS_HZ, den);
                (at(s.pts), at(s.dts))
            })
            .collect());
    }
    Ok(tr.samples.iter().map(|s| (round_div(i128::from(s.pts) * TS_HZ, mts), round_div(i128::from(s.dts) * TS_HZ, mts))).collect())
}

/// Read-back check of one Annex B access unit against its length-prefixed source sample: a
/// delimiter first (the sample's own, or a standard one with the picture's TemporalId), then the
/// sample entry's parameter sets exactly when the sample is a sync sample lacking a parameter set
/// type a decoder needs there, then the source NAL units unchanged; and every sync sample read
/// back must carry each needed parameter set type.
pub(super) fn annexb_equivalent(b: &AnnexB, source: &[u8], unit: &[u8], sync: bool) -> Result<(), String> {
    let ty = |n: &[u8]| n.first().map(|h| if b.hevc { (h >> 1) & 0x3F } else { h & 0x1F });
    let (aud, needed): (u8, &[u8]) = if b.hevc { (35, &[32, 33, 34]) } else { (9, &[7, 8]) };
    let src = cfa4_bitstream::length_prefixed_nals(source, b.length_size).map_err(|e| format!("the source sample cannot be split ({e:?})"))?;
    let got = cfa4_bitstream::annexb_nals(unit);
    let Some((first, after)) = got.split_first() else { return Err("an empty access unit".into()) };
    let rest: &[&[u8]] = match src.split_first() {
        Some((s0, tail)) if ty(s0) == Some(aud) => {
            if first != s0 {
                return Err("the sample's own delimiter changed".into());
            }
            tail
        }
        _ => {
            let standard: &[u8] = if b.hevc { &[0x46, 0, 0x50] } else { &[0x09, 0xF0] };
            let tid_ok = !b.hevc || first.get(1).copied() == Some(src.first().and_then(|n| n.get(1)).map_or(1, |x| (x & 0x07).max(1)));
            if first.len() != standard.len() || first.first() != standard.first() || first.get(2) != standard.get(2) || !tid_ok {
                return Err("the access unit does not start with the expected delimiter".into());
            }
            &src
        }
    };
    let lacking = sync && needed.iter().any(|&t| !rest.iter().any(|n| ty(n) == Some(t)));
    let ps: Vec<&[u8]> = b.parameter_sets.iter().map(Vec::as_slice).collect();
    let body: &[&[u8]] = if lacking {
        if after.len() < ps.len() || after[..ps.len()] != ps[..] {
            return Err("the sample entry's parameter sets were not put before a sync sample lacking them".into());
        }
        &after[ps.len()..]
    } else {
        after
    };
    if body != rest {
        return Err("the NAL units read back are not the source's".into());
    }
    if sync && needed.iter().any(|&t| !after.iter().any(|n| ty(n) == Some(t))) {
        return Err("a sync sample read back lacks a parameter set a decoder needs to start there".into());
    }
    Ok(())
}

/// Read-back check of one ADTS frame: the header the AudioSpecificConfig `asc` calls for (MPEG-4,
/// no CRC, one raw data block, the frame's length), then the source frame unchanged.
pub(super) fn adts_equivalent(asc: &[u8], source: &[u8], frame: &[u8]) -> bool {
    let (Some(&a0), Some(&a1), Some(h)) = (asc.first(), asc.get(1), frame.get(..7)) else { return false };
    let (aot, sf, ch) = (a0 >> 3, ((a0 & 7) << 1) | (a1 >> 7), (a1 >> 3) & 0x0F);
    let len = (usize::from(h[3] & 0x03) << 11) | (usize::from(h[4]) << 3) | usize::from(h[5] >> 5);
    h[0] == 0xFF
        && h[1] == 0xF1
        && aot >= 1
        && h[2] >> 6 == aot - 1
        && (h[2] >> 2) & 0x0F == sf
        && ((h[2] & 1) << 2 | h[3] >> 6) == ch
        && h[6] & 0x03 == 0
        && len == frame.len()
        && frame.get(7..) == Some(source)
}

/// A transport packet's adaptation field (after its length byte), if it has one.
fn adaptation(p: &[u8]) -> Option<&[u8]> {
    (p[3] & 0x20 != 0).then(|| p.get(5..5 + usize::from(p[4])).unwrap_or(&[]))
}

/// Every packet of the copied streams must arrive, on the clock its PCRs define (interpolated
/// between them, ISO/IEC 13818-1 §2.4.2.2), no later than the decode time of the PES packet it
/// carries: otherwise the decoder's buffer runs dry (the multiplex rate was too low).
pub(super) fn check_arrival(out: &File, size: u64, pids: &[u16]) -> Result<(), CopyError> {
    use std::io::{BufReader, Read, Seek};
    const WRAP: i128 = (1 << 33) * SYS_PER_90K;
    let skip = (size - 188) as usize;
    let scan = |each: &mut dyn FnMut(u64, &[u8]) -> Result<(), CopyError>| -> Result<(), CopyError> {
        let mut f = out;
        f.seek(std::io::SeekFrom::Start(0)).map_err(io)?;
        let mut r = BufReader::with_capacity(1 << 20, f);
        let mut buf = vec![0u8; size as usize];
        let mut index = 0u64;
        loop {
            match r.read_exact(&mut buf) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(e) => return Err(io(e)),
            }
            each(index, &buf[skip..])?;
            index += 1;
        }
    };
    // the PCRs (packet index, 27 MHz, unwrapped)
    let mut pcrs: Vec<(u64, i128)> = Vec::new();
    scan(&mut |i, p| {
        if let Some(af) = adaptation(p)
            && af.len() >= 7
            && af[0] & 0x10 != 0
        {
            let base = (i128::from(af[1]) << 25) | (i128::from(af[2]) << 17) | (i128::from(af[3]) << 9) | (i128::from(af[4]) << 1) | i128::from(af[5] >> 7);
            let ext = (i128::from(af[5] & 1) << 8) | i128::from(af[6]);
            let mut t = base * SYS_PER_90K + ext;
            if let Some(&(_, last)) = pcrs.last() {
                t += (last - t + WRAP / 2).div_euclid(WRAP) * WRAP;
            }
            pcrs.push((i, t));
        }
        Ok(())
    })?;
    if pcrs.len() < 2 {
        return Ok(());
    }
    let arrival = |i: u64| -> i128 {
        // the PCR segment around packet i (the first or last one beyond them), end of the packet
        let at = pcrs.partition_point(|&(j, _)| j <= i).clamp(1, pcrs.len() - 1);
        let ((i0, t0), (i1, t1)) = (pcrs[at - 1], pcrs[at]);
        let span = i128::from(i1 - i0).max(1);
        t0 + (i128::from(i) + 1 - i128::from(i0)) * (t1 - t0) / span
    };
    let mut dts: Vec<Option<i128>> = vec![None; pids.len()];
    scan(&mut |i, p| {
        let pid = u16::from(p[1] & 0x1F) << 8 | u16::from(p[2]);
        let Some(k) = pids.iter().position(|&x| x == pid) else { return Ok(()) };
        if p[1] & 0x40 != 0 {
            let start = 4 + adaptation(p).map_or(0, |af| 1 + af.len());
            let pes = p.get(start..).unwrap_or(&[]);
            let stamp = |o: usize| {
                pes.get(o..o + 5).map(|b| {
                    (i128::from(b[0] >> 1 & 7) << 30)
                        | (i128::from(b[1]) << 22)
                        | (i128::from(b[2] >> 1) << 15)
                        | (i128::from(b[3]) << 7)
                        | i128::from(b[4] >> 1)
                })
            };
            dts[k] = match pes.get(7).map(|f| f >> 6) {
                Some(3) => stamp(14),
                Some(2) => stamp(9),
                _ => None,
            };
        }
        if let Some(Some(d)) = dts.get(k) {
            let late = (arrival(i) - d * SYS_PER_90K).rem_euclid(WRAP);
            // modulo the 33-bit wrap: within half a wrap after the decode time is late
            if late > 0 && late < WRAP / 2 {
                return Err(fail(format!("packet {i} of PID {pid:#x} arrives {} ms after its decode time: the multiplex rate is too low", late / 27_000)));
            }
        }
        Ok(())
    })
}
