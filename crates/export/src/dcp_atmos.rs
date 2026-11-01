//! Dolby Atmos TrueHD as the immersive audio of a DCP (#347): the object presentation of a `.thd`
//! stream becomes IAB frames (SMPTE ST 2098-2) with the stream's objects kept as objects.
//!
//! - **Bed.** Bed objects labelled L R C LFE Lss Rss Lrs Rrs Lts Rts become the channels of one
//!   bed: 9.1OH when the stream has a top surround, else 7.1 DS, the two layouts ISDCF Doc 15
//!   (IAB Application Profile 1) allows. A bed without rear surrounds (Atmos 5.1) feeds each side
//!   surround to the side and the rear channel at -3 dB. LFE2 joins LFE. Bed objects of other
//!   loudspeakers (front / rear heights, wides) become objects at their loudspeaker, snapped.
//! - **Objects.** Every dynamic object that has had a metadata update becomes an IAB object: its
//!   position (the OAMD unit room cube is the IAB one: x left → right, y screen → back, z floor →
//!   ceiling), gain (capped at unity, the most an IAB gain can be), size as a 1D spread, snap. A
//!   ramp is followed sub-block by sub-block. Elevation off puts the object on the floor. OAMD
//!   zone constraints are not carried (Profile 1 has no zone definitions).
//! - **Timing.** The stream's 48 kHz samples are cut into the composition's edit units from its
//!   first sample; the last partial frame is padded with silence. Every asset is DLC coded,
//!   24 bit.
//!
//! The stream is read and decoded as the frames are written, so a feature-length file needs no
//! more memory than a few frames.

use std::io::{BufReader, Read, Seek, SeekFrom};

use cfa4_iab::{AudioData, Bed, BedChannel, Coding, Decorrelation, Description, Frame, FrameRate, Gain, Object, Pan, Snap, Spread};
use cfa4_truehd::cfa4::{ObjectDecoder, samples_per_au};
use cfa4_truehd::render::{Event, MAX_OBJECTS, ObjectState, Source};
use cfa4_truehd::structs::oamd::SpeakerLabels;

use crate::{ExportError, Result};

/// Bytes read from the stream at a time.
const CHUNK: usize = 256 * 1024;

/// ChannelIDs (ST 2098-2 Table 20) of the 7.1 DS bed: L R C LFE Lss Rss Lrs Rrs; 9.1OH adds the
/// top surrounds Lts Rts.
const IDS_9_1_OH: [u32; 10] = [0x00, 0x04, 0x02, 0x0D, 0x05, 0x09, 0x07, 0x08, 0x0B, 0x0C];

/// The bed ChannelID of an Atmos bed loudspeaker; `None` for one the Profile 1 beds lack.
fn bed_channel(s: SpeakerLabels) -> Option<u32> {
    use SpeakerLabels as S;
    Some(match s {
        S::L => 0x00,
        S::R => 0x04,
        S::C => 0x02,
        S::LFE | S::LFE2 => 0x0D,
        S::Lss => 0x05,
        S::Rss => 0x09,
        S::Lrs => 0x07,
        S::Rrs => 0x08,
        S::Lts => 0x0B,
        S::Rts => 0x0C,
        _ => return None,
    })
}

/// A bed loudspeaker's position in the unit room cube (OAMD gives it in -1 ..= 1, y to the front).
fn speaker_position(s: SpeakerLabels) -> [f32; 3] {
    let [x, y, z] = *s.pos();
    [(x + 1.0) / 2.0, (1.0 - y) / 2.0, z.clamp(0.0, 1.0)]
}

/// A left-justified 24-bit sample (ST 429-18 7.2.3).
fn sample24(x: f32) -> i32 {
    let x = if x.is_finite() { x } else { 0.0 };
    ((x.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32).saturating_mul(256)
}

/// An IAB gain (5.5: 2^(-code / 64)) for a linear gain, capped at unity.
fn iab_gain(g: f32) -> Gain {
    if !g.is_finite() || g <= 0.0 {
        return Gain::Silent;
    }
    if g >= 1.0 {
        return Gain::Unity;
    }
    let code = (-64.0 * g.log2()).round();
    if code >= 1023.0 { Gain::Silent } else { Gain::Code(code.max(1.0) as u16) }
}

/// ObjectPosX / Y codes (Formula 1: DistanceXY = (code - 32767) / 32768) and ObjectPosZ.
fn position_codes(p: [f32; 3]) -> [u16; 3] {
    let c = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    let xy = |v: f32| (c(v) * 32768.0 + 32767.0).round().min(65535.0) as u16;
    [xy(p[0]), xy(p[1]), (c(p[2]) * 65535.0).round() as u16]
}

/// One object's metadata over time: the state it ramps from, the one it ramps to, the ramp.
#[derive(Clone, Copy, Debug)]
struct Track {
    from: ObjectState,
    to: ObjectState,
    start: u64,
    ramp: u64,
}

impl Track {
    /// The object's state at sample `t`.
    fn at(&self, t: u64) -> ObjectState {
        let done = t.saturating_sub(self.start);
        if self.ramp == 0 || done >= self.ramp {
            return self.to;
        }
        let k = done as f32 / self.ramp as f32;
        let lerp = |a: f32, b: f32| a + (b - a) * k;
        let gain = lerp(self.from.gain, self.to.gain);
        let source = match (self.from.source, self.to.source) {
            (Source::Point { position: a, size: sa, .. }, Source::Point { position: b, size: sb, zone, snap, elevation }) => {
                Source::Point { position: [lerp(a[0], b[0]), lerp(a[1], b[1]), lerp(a[2], b[2])], size: lerp(sa, sb), zone, snap, elevation }
            }
            (_, s) => s,
        };
        ObjectState { source, gain }
    }
}

/// The objects' metadata: updates not yet reached and every object's track.
#[derive(Default)]
struct Metadata {
    pending: Vec<Event>,
    tracks: Vec<Option<Track>>,
}

impl Metadata {
    fn push(&mut self, events: Vec<Event>) {
        self.pending.extend(events.into_iter().filter(|e| e.object < MAX_OBJECTS));
        // stable: updates at one offset keep their stream order
        self.pending.sort_by_key(|e| e.offset);
    }

    /// Applies the updates up to sample `t` (inclusive).
    fn advance(&mut self, t: u64) {
        let n = self.pending.partition_point(|e| e.offset <= t);
        for e in self.pending.drain(..n) {
            if self.tracks.len() <= e.object {
                self.tracks.resize(e.object + 1, None);
            }
            let Some(slot) = self.tracks.get_mut(e.object) else { continue };
            let from = slot.map_or(e.state, |tr| tr.at(e.offset));
            *slot = Some(Track { from, to: e.state, start: e.offset, ramp: e.ramp });
        }
    }

    fn state(&self, object: usize, t: u64) -> Option<ObjectState> {
        self.tracks.get(object).copied().flatten().map(|tr| tr.at(t))
    }
}

/// A Dolby Atmos TrueHD stream read as IAB frames, one edit unit at a time.
pub(crate) struct AtmosSource {
    path: String,
    reader: BufReader<std::fs::File>,
    decoder: ObjectDecoder,
    rate: FrameRate,
    /// Decoded object samples not yet framed, from sample `base` of the stream.
    buf: Vec<Vec<f32>>,
    base: u64,
    meta: Metadata,
    eof: bool,
    sample_rate: u32,
}

impl AtmosSource {
    /// Whether `head` (the first bytes of a file) is a TrueHD stream: a major sync (format sync
    /// 0xF8726FBA, 4 bytes into an access unit) in it. A cut stream starts before one.
    pub(crate) fn sniff(head: &[u8]) -> bool {
        head.windows(4).skip(4).any(|w| w == [0xF8, 0x72, 0x6F, 0xBA])
    }

    pub(crate) fn open(path: &str, rate: FrameRate) -> Result<AtmosSource> {
        let f = std::fs::File::open(path).map_err(|e| ExportError::Io(format!("{path}: {e}")))?;
        Ok(AtmosSource {
            path: path.to_string(),
            reader: BufReader::with_capacity(CHUNK, f),
            decoder: ObjectDecoder::new(),
            rate,
            buf: Vec::new(),
            base: 0,
            meta: Metadata::default(),
            eof: false,
            sample_rate: 0,
        })
    }

    fn err(&self, e: impl std::fmt::Display) -> ExportError {
        ExportError::Io(format!("{}: {e}", self.path))
    }

    /// Decodes until `n` samples are buffered or the stream ends.
    fn fill(&mut self, n: usize) -> Result<()> {
        let mut chunk = vec![0u8; CHUNK];
        while !self.eof && self.buf.first().is_none_or(|b| b.len() < n) {
            let got = self.reader.read(&mut chunk).map_err(|e| self.err(e))?;
            if got == 0 {
                self.eof = true;
                break;
            }
            let d = self.decoder.decode(chunk.get(..got).unwrap_or_default()).map_err(|e| self.err(e))?;
            if d.objects.is_empty() {
                continue;
            }
            if d.sample_rate != 48_000 {
                return Err(ExportError::Unsupported(format!("{}: the Atmos stream is {} Hz; DCP immersive audio is 48 kHz", self.path, d.sample_rate)));
            }
            self.sample_rate = d.sample_rate;
            let have = self.buf.first().map_or(0, Vec::len);
            if self.buf.len() < d.objects.len() {
                self.buf.resize(d.objects.len(), vec![0.0; have]);
            }
            let added = d.objects.first().map_or(0, Vec::len);
            for (c, b) in self.buf.iter_mut().enumerate() {
                match d.objects.get(c) {
                    Some(o) => b.extend_from_slice(o),
                    None => b.resize(b.len() + added, 0.0),
                }
            }
            self.meta.push(d.events);
        }
        if self.eof && self.buf.is_empty() && self.base == 0 {
            return Err(self.err("no Dolby Atmos audio in this TrueHD stream"));
        }
        Ok(())
    }

    /// The next edit unit as an IAB frame; `None` once the stream has ended.
    pub(crate) fn next_frame(&mut self) -> Result<Option<Frame>> {
        let n = self.rate.samples(false, 0);
        self.fill(n)?;
        let have = self.buf.first().map_or(0, Vec::len);
        if have == 0 && self.eof {
            return Ok(None);
        }
        let take = have.min(n);
        let unit: Vec<Vec<f32>> = self
            .buf
            .iter_mut()
            .map(|b| {
                let mut v: Vec<f32> = b.drain(..take).collect();
                v.resize(n, 0.0);
                v
            })
            .collect();
        let frame = frame_of(self.rate, self.base, &mut self.meta, &unit);
        self.base = self.base.saturating_add(take as u64);
        Ok(Some(frame))
    }
}

/// The IAB frame of one edit unit of object audio (`unit`, one signal per object) starting at
/// sample `base` of the stream, with the objects' metadata from `meta`.
fn frame_of(rate: FrameRate, base: u64, meta: &mut Metadata, unit: &[Vec<f32>]) -> Frame {
    let n = rate.samples(false, 0);
    let sizes = rate.sub_block_sizes(false, 0);
    let starts: Vec<u64> = sizes.iter().scan(base, |t, &s| Some(std::mem::replace(t, t.saturating_add(s as u64)))).collect();
    // each object's state at the start of each sub-block
    let mut states: Vec<Vec<Option<ObjectState>>> = vec![Vec::with_capacity(starts.len()); unit.len()];
    for &t in &starts {
        meta.advance(t);
        for (o, s) in states.iter_mut().enumerate() {
            s.push(meta.state(o, t));
        }
    }
    // the bed: the bed objects' labels at the frame's start
    let label = |o: usize| match states.get(o).and_then(|s| s.first()).copied().flatten() {
        Some(ObjectState { source: Source::Bed(l), .. }) => Some(l),
        _ => None,
    };
    let labels: Vec<Option<SpeakerLabels>> = (0..unit.len()).map(label).collect();
    let has = |l: SpeakerLabels| labels.contains(&Some(l));
    let tops = has(SpeakerLabels::Lts) || has(SpeakerLabels::Rts);
    let ids: &[u32] = if tops { &IDS_9_1_OH } else { &IDS_9_1_OH[..8] };
    let k = std::f32::consts::FRAC_1_SQRT_2;
    // (object, gain) feeding each bed channel
    let mut feeds: Vec<Vec<(usize, f32)>> = vec![Vec::new(); ids.len()];
    let mut feed = |id: u32, o: usize, g: f32| {
        if let Some(i) = ids.iter().position(|&x| x == id)
            && let Some(f) = feeds.get_mut(i)
        {
            f.push((o, g));
        }
    };
    let mut objects: Vec<usize> = Vec::new();
    for (o, &l) in labels.iter().enumerate() {
        match l {
            Some(SpeakerLabels::Lss) if !has(SpeakerLabels::Lrs) => {
                feed(0x05, o, k);
                feed(0x07, o, k);
            }
            Some(SpeakerLabels::Rss) if !has(SpeakerLabels::Rrs) => {
                feed(0x09, o, k);
                feed(0x08, o, k);
            }
            Some(l) => match bed_channel(l) {
                Some(id) => feed(id, o, 1.0),
                None => objects.push(o),
            },
            // a dynamic object, once it has metadata
            None if states.get(o).is_some_and(|s| s.iter().any(Option::is_some)) => objects.push(o),
            None => {}
        }
    }
    // assets: bed channels first (sums of their feeds, with the bed objects' gains), then objects
    let gain_of = |o: usize, sb: usize| states.get(o).and_then(|s| s.get(sb)).copied().flatten().map_or(0.0, |s| s.gain);
    let mut audio = Vec::new();
    let mut channels = Vec::new();
    for (i, (&id, f)) in ids.iter().zip(&feeds).enumerate() {
        if f.is_empty() {
            channels.push(BedChannel { id, audio_id: 0, gain: Gain::Unity, decorrelation: None });
            continue;
        }
        let mut samples = vec![0.0f32; n];
        let mut at = 0usize;
        for (sb, &size) in sizes.iter().enumerate() {
            for &(o, g) in f {
                let g = g * gain_of(o, sb);
                let src = unit.get(o).map(Vec::as_slice).unwrap_or_default();
                for (d, s) in samples.iter_mut().skip(at).take(size).zip(src.iter().skip(at)) {
                    *d += s * g;
                }
            }
            at += size;
        }
        let audio_id = i as u32 + 1;
        audio.push(AudioData { id: audio_id, coding: Coding::Dlc, samples: samples.iter().map(|&x| sample24(x)).collect() });
        channels.push(BedChannel { id, audio_id, gain: Gain::Unity, decorrelation: None });
    }
    let mut iab_objects = Vec::new();
    for (j, &o) in objects.iter().enumerate() {
        let audio_id = (ids.len() + j) as u32 + 1;
        let src = unit.get(o).map(Vec::as_slice).unwrap_or_default();
        audio.push(AudioData { id: audio_id, coding: Coding::Dlc, samples: src.iter().map(|&x| sample24(x)).collect() });
        let pan = (0..sizes.len())
            .map(|sb| {
                let st = states.get(o).and_then(|s| s.get(sb)).copied().flatten();
                Some(object_pan(st))
            })
            .collect();
        iab_objects.push(Object {
            meta_id: o as u32 + 2,
            audio_id,
            use_case: None,
            pan,
            description: Description::default(),
            objects: Vec::new(),
            zones19: None,
        });
    }
    Frame {
        version: 1,
        sample_rate: 48_000,
        bit_depth: 24,
        frame_rate: rate,
        frame_index: 0,
        max_rendered: (ids.len() + iab_objects.len()) as u32,
        beds: vec![Bed { meta_id: 1, use_case: None, channels, description: Description::default(), beds: Vec::new(), remaps: Vec::new() }],
        objects: iab_objects,
        audio,
        authoring_tool: None,
        user_data: Vec::new(),
    }
}

/// An IAB object's panning for an object state (silent before its first update).
fn object_pan(st: Option<ObjectState>) -> Pan {
    let silent = Pan {
        gain: Gain::Silent,
        position: position_codes([0.5, 0.5, 0.0]),
        snap: Snap::Off,
        zones: None,
        spread: Spread::OneD(0),
        decorrelation: Decorrelation::None,
    };
    let Some(st) = st else { return silent };
    let (position, size, snap) = match st.source {
        Source::Point { position, size, snap, elevation, .. } => {
            let z = if elevation { position[2] } else { 0.0 };
            ([position[0], position[1], z], size, snap)
        }
        // a bed loudspeaker no Profile 1 bed has: an object snapped to that loudspeaker
        Source::Bed(l) => (speaker_position(l), 0.0, true),
    };
    let size = if size.is_finite() { size.clamp(0.0, 1.0) } else { 0.0 };
    Pan {
        gain: iab_gain(st.gain),
        position: position_codes(position),
        snap: if snap { Snap::On(None) } else { Snap::Off },
        zones: None,
        spread: Spread::OneD((size * 4095.0).round() as u16),
        decorrelation: Decorrelation::None,
    }
}

/// What a TrueHD Atmos source holds: (seconds, sample rate, most bed channels, most objects),
/// looking at the first `frames` edit units at `rate`.
pub(crate) fn probe(path: &str, rate: FrameRate, frames: usize) -> Result<(f64, u32, u32, u32)> {
    let mut src = AtmosSource::open(path, rate)?;
    let (mut beds, mut objects) = (0u32, 0u32);
    for _ in 0..frames {
        let Some(f) = src.next_frame()? else { break };
        beds = beds.max(f.beds.iter().map(|b| b.channels.iter().filter(|c| c.audio_id != 0).count() as u32).sum());
        objects = objects.max(f.objects.len() as u32);
    }
    let rate_hz = src.sample_rate;
    let per_au = samples_per_au(rate_hz).unwrap_or(40);
    let aus = count_access_units(path)?;
    let seconds = if rate_hz > 0 { (aus.saturating_mul(per_au.unsigned_abs())) as f64 / f64::from(rate_hz) } else { 0.0 };
    Ok((seconds, rate_hz, beds, objects))
}

/// Access units of a raw TrueHD stream, by following the length in each one's first two bytes.
fn count_access_units(path: &str) -> Result<u64> {
    let f = std::fs::File::open(path).map_err(|e| ExportError::Io(format!("{path}: {e}")))?;
    let mut r = BufReader::with_capacity(CHUNK, f);
    let mut n = 0u64;
    let mut head = [0u8; 2];
    loop {
        match r.read_exact(&mut head) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(ExportError::Io(format!("{path}: {e}"))),
        }
        let len = ((i64::from(head[0] & 0x0F) << 8) | i64::from(head[1])) * 2;
        if len < 4 {
            break;
        }
        n += 1;
        r.seek_relative(len - 2).map_err(|e| ExportError::Io(format!("{path}: {e}")))?;
    }
    // a read past the end is not an access unit
    let pos = r.stream_position().map_err(|e| ExportError::Io(format!("{path}: {e}")))?;
    let end = r.seek(SeekFrom::End(0)).map_err(|e| ExportError::Io(format!("{path}: {e}")))?;
    if pos > end {
        n = n.saturating_sub(1);
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f32, y: f32, z: f32, gain: f32) -> ObjectState {
        ObjectState { source: Source::Point { position: [x, y, z], size: 0.0, zone: 0, snap: false, elevation: true }, gain }
    }

    /// A real Dolby Atmos TrueHD stream (`CFA4_ATMOS_THD`, e.g. the Altitude trailer; skipped
    /// when unset): up to a minute of frames that parse back, stay in Profile 1 and carry sound.
    #[test]
    fn a_real_atmos_stream_becomes_profile_1_iab_frames() {
        let Ok(path) = std::env::var("CFA4_ATMOS_THD") else { return };
        let mut src = AtmosSource::open(&path, FrameRate::R24).unwrap();
        let (mut energy, mut moved) = (0.0f64, false);
        let mut first_pos: Vec<Option<[u16; 3]>> = Vec::new();
        for i in 0..1500 {
            let Some(f) = src.next_frame().unwrap() else { break };
            let back = cfa4_iab::parse(&cfa4_iab::write(&f).unwrap()).unwrap();
            assert_eq!(back, f, "frame {i}");
            assert_eq!(f.beds.len(), 1);
            let n = f.beds[0].channels.len();
            assert!(n == 8 || n == 10, "7.1 DS or 9.1OH, got {n}");
            assert!(f.objects.len() <= 118 && f.max_rendered <= 128);
            assert!(f.audio.iter().all(|a| a.coding == Coding::Dlc && a.samples.len() == 2000));
            for o in &f.objects {
                let idx = o.meta_id as usize;
                if first_pos.len() <= idx {
                    first_pos.resize(idx + 1, None);
                }
                for p in o.pan.iter().flatten().filter(|p| p.gain != Gain::Silent) {
                    match first_pos[idx] {
                        None => first_pos[idx] = Some(p.position),
                        Some(q) if q != p.position => moved = true,
                        _ => {}
                    }
                }
            }
            let out = cfa4_iab::render(&f, &cfa4_iab::Layout::surround_5_1()).unwrap();
            energy += out.iter().flatten().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>();
        }
        assert!(energy > 1.0, "the rendered frames carry sound ({energy})");
        assert!(first_pos.iter().any(Option::is_some), "the stream has objects");
        eprintln!("{path}: objects moved: {moved}");
    }

    fn bed(l: SpeakerLabels, gain: f32) -> ObjectState {
        ObjectState { source: Source::Bed(l), gain }
    }

    /// A 5.1.2 bed (no rear surrounds) and two objects, one moving: a 9.1OH bed with the side
    /// surrounds split to the rears, the moving object followed sub-block by sub-block, a
    /// never-updated object left out.
    #[test]
    fn a_bed_and_objects_make_one_iab_frame() {
        use SpeakerLabels as S;
        let labels = [S::L, S::R, S::C, S::LFE, S::Lss, S::Rss, S::Lts, S::Rts];
        let mut events: Vec<Event> = labels.iter().enumerate().map(|(o, &l)| Event { offset: 0, object: o, state: bed(l, 1.0), ramp: 0 }).collect();
        // object 8 sweeps left to right over the frame (2000 samples at 24 fps); object 9 is silent
        events.push(Event { offset: 0, object: 8, state: point(0.0, 0.5, 0.0, 1.0), ramp: 0 });
        events.push(Event { offset: 0, object: 8, state: point(1.0, 0.5, 0.0, 0.5), ramp: 2000 });
        let mut meta = Metadata::default();
        meta.push(events);
        let unit: Vec<Vec<f32>> = (0..10).map(|c| (0..2000).map(|i| ((i * (c + 1)) as f32 * 0.003).sin() * 0.25).collect()).collect();
        let f = frame_of(FrameRate::R24, 0, &mut meta, &unit);
        let back = cfa4_iab::parse(&cfa4_iab::write(&f).unwrap()).unwrap();
        assert_eq!(back, f);
        let ids: Vec<u32> = f.beds[0].channels.iter().map(|c| c.id).collect();
        assert_eq!(ids, IDS_9_1_OH, "the top surrounds make it 9.1OH");
        let asset = |id: u32| {
            let ch = f.beds[0].channels.iter().find(|c| c.id == id).unwrap();
            f.audio_data(ch.audio_id).unwrap().samples.clone()
        };
        // L and Lts as they are; Lss on the side and the rear surround at -3 dB
        assert_eq!(asset(0x00)[777], sample24(unit[0][777]));
        assert_eq!(asset(0x0B)[777], sample24(unit[6][777]));
        let k = std::f32::consts::FRAC_1_SQRT_2;
        assert_eq!(asset(0x05)[777], sample24(unit[4][777] * k));
        assert_eq!(asset(0x07)[777], sample24(unit[4][777] * k));
        // one object: object 8, its own samples, moving right with its gain ramping down
        assert_eq!(f.objects.len(), 1);
        let o = &f.objects[0];
        assert_eq!(o.meta_id, 10);
        assert_eq!(f.audio_data(o.audio_id).unwrap().samples[1234], sample24(unit[8][1234]));
        assert_eq!(o.pan.len(), 8);
        let xs: Vec<f32> = o.pan.iter().map(|p| p.as_ref().unwrap().position()[0]).collect();
        assert!(xs[0] < 1e-4 && xs.windows(2).all(|w| w[1] > w[0]), "{xs:?}");
        assert!((xs[4] - 0.5).abs() < 1e-3, "half way at sub-block 4: {xs:?}");
        assert!((o.pan[4].as_ref().unwrap().gain.linear() - 0.75).abs() < 0.01);
        assert_eq!(f.max_rendered, 11);
        // the IAB renderer plays it (the 5.1 fallback): every channel carries sound
        let out = cfa4_iab::render(&f, &cfa4_iab::Layout::surround_5_1()).unwrap();
        assert_eq!(out.len(), 6);
        assert!(out.iter().all(|c| c.iter().any(|v| v.abs() > 1e-3)));
    }

    /// A 7.1 bed (with rear surrounds) and LFE2 joining LFE: 7.1 DS.
    #[test]
    fn a_7_1_bed_is_7_1_ds_and_lfe2_joins_lfe() {
        use SpeakerLabels as S;
        let labels = [S::L, S::R, S::C, S::LFE, S::Lss, S::Rss, S::Lrs, S::Rrs, S::LFE2];
        let mut meta = Metadata::default();
        meta.push(labels.iter().enumerate().map(|(o, &l)| Event { offset: 0, object: o, state: bed(l, 1.0), ramp: 0 }).collect());
        let unit: Vec<Vec<f32>> = (0..9).map(|c| vec![0.05 * (c + 1) as f32; 2000]).collect();
        let f = frame_of(FrameRate::R24, 0, &mut meta, &unit);
        let ids: Vec<u32> = f.beds[0].channels.iter().map(|c| c.id).collect();
        assert_eq!(ids, IDS_9_1_OH[..8]);
        assert!(f.objects.is_empty());
        let lfe = f.beds[0].channels.iter().find(|c| c.id == 0x0D).unwrap();
        assert_eq!(f.audio_data(lfe.audio_id).unwrap().samples[9], sample24(0.2 + 0.45));
        let lrs = f.beds[0].channels.iter().find(|c| c.id == 0x07).unwrap();
        assert_eq!(f.audio_data(lrs.audio_id).unwrap().samples[9], sample24(0.35));
    }

    #[test]
    fn gains_and_positions_code_as_st_2098_2() {
        assert_eq!(iab_gain(1.0), Gain::Unity);
        assert_eq!(iab_gain(2.0), Gain::Unity, "capped at unity");
        assert_eq!(iab_gain(0.0), Gain::Silent);
        assert_eq!(iab_gain(f32::NAN), Gain::Silent);
        assert_eq!(iab_gain(0.5), Gain::Code(64));
        assert!((iab_gain(0.3).linear() - 0.3).abs() < 0.004);
        let codes = position_codes([0.0, 1.0, 0.5]);
        let pan = Pan { gain: Gain::Unity, position: codes, snap: Snap::Off, zones: None, spread: Spread::None, decorrelation: Decorrelation::None };
        let p = pan.position();
        assert!(p[0].abs() < 1e-4 && (p[1] - 1.0).abs() < 1e-4 && (p[2] - 0.5).abs() < 1e-4, "{p:?}");
        assert_eq!(position_codes([f32::NAN, -3.0, 9.0]), [32767, 32767, 65535]);
    }

    #[test]
    fn ramps_interpolate_and_later_updates_start_from_where_the_ramp_was() {
        let mut m = Metadata::default();
        m.push(vec![
            Event { offset: 100, object: 3, state: point(0.0, 0.0, 0.0, 1.0), ramp: 0 },
            Event { offset: 200, object: 3, state: point(1.0, 0.0, 0.0, 0.5), ramp: 100 },
            Event { offset: 250, object: 3, state: point(1.0, 1.0, 0.0, 0.5), ramp: 0 },
        ]);
        m.advance(0);
        assert!(m.state(3, 0).is_none(), "silent before its first update");
        m.advance(150);
        assert_eq!(m.state(3, 150), Some(point(0.0, 0.0, 0.0, 1.0)));
        m.advance(225);
        let Some(ObjectState { source: Source::Point { position, .. }, gain }) = m.state(3, 225) else { panic!() };
        assert!((position[0] - 0.25).abs() < 1e-6 && (gain - 0.875).abs() < 1e-6);
        m.advance(300);
        assert_eq!(m.state(3, 300), Some(point(1.0, 1.0, 0.0, 0.5)));
        // an update for an object past the stream's limit is dropped
        m.push(vec![Event { offset: 400, object: MAX_OBJECTS, state: point(0.0, 0.0, 0.0, 1.0), ramp: 0 }]);
        m.advance(500);
        assert!(m.state(MAX_OBJECTS, 500).is_none());
    }

    #[test]
    fn bed_loudspeakers_without_a_profile_1_channel_snap_as_objects() {
        let pan = object_pan(Some(ObjectState { source: Source::Bed(SpeakerLabels::Lfh), gain: 1.0 }));
        assert_eq!(pan.snap, Snap::On(None));
        let p = pan.position();
        assert!(p[0] < 1e-4 && p[1] < 1e-4 && (p[2] - 1.0).abs() < 1e-4, "front left height: {p:?}");
        assert_eq!(bed_channel(SpeakerLabels::Lts), Some(0x0B));
        assert_eq!(bed_channel(SpeakerLabels::Rts), Some(0x0C));
        assert_eq!(bed_channel(SpeakerLabels::LFE2), Some(0x0D));
        assert_eq!(bed_channel(SpeakerLabels::Lw), None);
        // elevation off puts an object on the floor; a silent object before any update
        let mut st = point(0.2, 0.3, 0.9, 1.0);
        if let Source::Point { elevation, .. } = &mut st.source {
            *elevation = false;
        }
        assert!(object_pan(Some(st)).position()[2].abs() < 1e-6);
        assert_eq!(object_pan(None).gain, Gain::Silent);
    }
}
