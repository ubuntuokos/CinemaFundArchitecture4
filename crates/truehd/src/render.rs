//! CFA4: a Dolby Atmos object renderer for the 16-channel object presentation (not part of the
//! upstream crate).
//!
//! The object presentation carries up to 16 PCM channels, one per object, and Object Audio
//! Metadata (OAMD) in the access units' Evolution frames: the first objects are bed channels
//! (a speaker each), the rest dynamic objects with a position in the room, a gain, a size, a zone
//! and a snap flag. Rendering to loudspeakers is left to the playback device; this renderer makes
//! plain, documented choices:
//!
//! - A bed channel goes to the loudspeaker with its label when the layout has one, else it is
//!   panned from the label's position. LFE beds go to the layout's LFE and are dropped when it has
//!   none.
//! - Dynamic objects are panned by distance: every full-range loudspeaker of the allowed zone gets
//!   the weight 1 / (d² + ε)², d its distance to the object in the unit room cube; weights under
//!   5 % of the total are dropped and the rest normalised to constant power. Size widens ε, snap
//!   sends the object to its nearest loudspeaker, elevation off flattens it to ear height.
//! - A metadata update starts at its sample offset and ramps linearly to the new gains over its
//!   ramp duration. Objects are silent until their first update.
//!
//! Not modelled: screen-referenced scaling, divergence, trims, binaural (headphone) rendering and
//! intermediate spatial format (ISF) programs, for which [`events`] returns `None` so that the
//! caller can fall back to a channel presentation.

use crate::structs::channel::ChannelLabel;
use crate::structs::oamd::{GAIN_MINUS_INFINITY, ObjectAudioMetadataPayload, SpeakerLabels};

/// One loudspeaker of a [`Layout`].
#[derive(Clone, Debug, PartialEq)]
pub struct Speaker {
    pub label: ChannelLabel,
    /// Position in the unit room cube: x left → right, y front → back, z ear height → ceiling.
    pub position: [f32; 3],
    pub lfe: bool,
}

/// An output loudspeaker layout, in CFA4's channel order (L R C LFE back-L back-R side-L side-R,
/// then the overhead loudspeakers).
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub name: &'static str,
    pub speakers: Vec<Speaker>,
}

fn sp(label: ChannelLabel, x: f32, y: f32, z: f32) -> Speaker {
    Speaker { label, position: [x, y, z], lfe: false }
}

fn lfe() -> Speaker {
    Speaker { label: ChannelLabel::LFE, position: [0.5, 0.0, 0.0], lfe: true }
}

impl Layout {
    /// L R.
    pub fn stereo() -> Self {
        use ChannelLabel::*;
        Self { name: "2.0", speakers: vec![sp(L, 0.0, 0.0, 0.0), sp(R, 1.0, 0.0, 0.0)] }
    }

    /// L R C LFE Ls Rs.
    pub fn surround_5_1() -> Self {
        use ChannelLabel::*;
        Self {
            name: "5.1",
            speakers: vec![sp(L, 0.0, 0.0, 0.0), sp(R, 1.0, 0.0, 0.0), sp(C, 0.5, 0.0, 0.0), lfe(), sp(Ls, 0.0, 0.75, 0.0), sp(Rs, 1.0, 0.75, 0.0)],
        }
    }

    /// L R C LFE Lb Rb Ls Rs (back surrounds before side surrounds, as in WAV).
    pub fn surround_7_1() -> Self {
        use ChannelLabel::*;
        Self {
            name: "7.1",
            speakers: vec![
                sp(L, 0.0, 0.0, 0.0),
                sp(R, 1.0, 0.0, 0.0),
                sp(C, 0.5, 0.0, 0.0),
                lfe(),
                sp(Lb, 0.0, 1.0, 0.0),
                sp(Rb, 1.0, 1.0, 0.0),
                sp(Ls, 0.0, 0.5, 0.0),
                sp(Rs, 1.0, 0.5, 0.0),
            ],
        }
    }

    /// 7.1 and four overhead loudspeakers: top front left / right, top back left / right.
    pub fn surround_7_1_4() -> Self {
        use ChannelLabel::*;
        let mut l = Self::surround_7_1();
        l.name = "7.1.4";
        l.speakers.extend([sp(Tfl, 0.0, 0.25, 1.0), sp(Tfr, 1.0, 0.25, 1.0), sp(Tbl, 0.0, 0.75, 1.0), sp(Tbr, 1.0, 0.75, 1.0)]);
        l
    }

    pub fn channels(&self) -> usize {
        self.speakers.len()
    }

    pub fn labels(&self) -> Vec<ChannelLabel> {
        self.speakers.iter().map(|s| s.label).collect()
    }
}

/// What an object is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    /// A bed channel: plays from its speaker.
    Bed(SpeakerLabels),
    /// A dynamic object.
    Point {
        /// Unit room cube (see [`Speaker::position`]).
        position: [f32; 3],
        /// 0 (a point) ..= 1 (the whole room).
        size: f32,
        /// OAMD zone constraint: 0 all, 1 no back, 2 no sides, 3 centre back, 4 screen only,
        /// 5 surround only.
        zone: u8,
        snap: bool,
        elevation: bool,
    },
}

/// One object's state from one metadata update.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjectState {
    pub source: Source,
    /// Linear gain; 0 for an inactive object.
    pub gain: f32,
}

/// The CFA4 label a bed speaker plays from, when a layout has it.
fn bed_label(s: SpeakerLabels) -> ChannelLabel {
    use SpeakerLabels as S;
    match s {
        S::L => ChannelLabel::L,
        S::R => ChannelLabel::R,
        S::C => ChannelLabel::C,
        S::LFE | S::LFE2 => ChannelLabel::LFE,
        S::Lss => ChannelLabel::Ls,
        S::Rss => ChannelLabel::Rs,
        S::Lrs => ChannelLabel::Lb,
        S::Rrs => ChannelLabel::Rb,
        S::Lfh => ChannelLabel::Tfl,
        S::Rfh => ChannelLabel::Tfr,
        S::Lts => ChannelLabel::Tsl,
        S::Rts => ChannelLabel::Tsr,
        S::Lrh => ChannelLabel::Tbl,
        S::Rrh => ChannelLabel::Tbr,
        S::Lw => ChannelLabel::Lw,
        S::Rw => ChannelLabel::Rw,
    }
}

/// A bed speaker's position in the unit room cube (OAMD gives it in -1 ..= 1 with y to the front).
fn bed_position(s: SpeakerLabels) -> [f32; 3] {
    let [x, y, z] = *s.pos();
    [(x + 1.0) / 2.0, (1.0 - y) / 2.0, z.clamp(0.0, 1.0)]
}

/// Whether a loudspeaker at `p` is in OAMD zone `zone`. The zones restrict the ear-height
/// loudspeakers; "screen only" also excludes the overhead ones. "Centre back" is not narrowed.
fn in_zone(zone: u8, p: [f32; 3]) -> bool {
    let [_, y, z] = p;
    let overhead = z >= 0.5;
    let screen = !overhead && y < 0.1;
    let back = !overhead && y > 0.9;
    let side = !overhead && !screen && !back;
    match zone {
        1 => !back,
        2 => !side,
        4 => screen,
        5 => !screen,
        _ => true,
    }
}

/// Constant-power distance panning of a point (see the module notes).
fn pan(layout: &Layout, p: [f32; 3], size: f32, zone: u8, snap: bool) -> Vec<f32> {
    let mut g = vec![0.0f32; layout.channels()];
    let usable = |s: &Speaker| !s.lfe && in_zone(zone, s.position);
    // a zone without loudspeakers in this layout constrains nothing
    let any = layout.speakers.iter().any(usable);
    let ok = |s: &Speaker| !s.lfe && (!any || in_zone(zone, s.position));
    let dist2 = |s: &Speaker| s.position.iter().zip(p).map(|(a, b)| (a - b) * (a - b)).sum::<f32>();
    if snap {
        let nearest = layout.speakers.iter().enumerate().filter(|(_, s)| ok(s)).min_by(|a, b| dist2(a.1).total_cmp(&dist2(b.1)));
        if let Some((i, _)) = nearest
            && let Some(v) = g.get_mut(i)
        {
            *v = 1.0;
        }
        return g;
    }
    let size = if size.is_finite() { size.clamp(0.0, 1.0) } else { 0.0 };
    let eps = 0.004 + 0.5 * size * size;
    for (v, s) in g.iter_mut().zip(&layout.speakers) {
        if ok(s) {
            let d2 = dist2(s);
            *v = 1.0 / ((d2 + eps) * (d2 + eps));
        }
    }
    let sum: f32 = g.iter().sum();
    for v in &mut g {
        if *v < 0.05 * sum {
            *v = 0.0;
        }
    }
    let norm = g.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 && norm.is_finite() {
        for v in &mut g {
            *v /= norm;
        }
    }
    g
}

/// The loudspeaker gains of an object, its own gain included.
pub fn speaker_gains(layout: &Layout, state: &ObjectState) -> Vec<f32> {
    let gain = if state.gain.is_finite() { state.gain.max(0.0) } else { 0.0 };
    let mut g = match state.source {
        Source::Bed(s) => {
            let label = bed_label(s);
            let lfe = matches!(s, SpeakerLabels::LFE | SpeakerLabels::LFE2);
            let mut g = vec![0.0; layout.channels()];
            match layout.speakers.iter().position(|sp| sp.label == label && sp.lfe == lfe) {
                Some(i) => {
                    if let Some(v) = g.get_mut(i) {
                        *v = 1.0;
                    }
                    g
                }
                // an LFE without an LFE loudspeaker is dropped
                None if lfe => g,
                None => pan(layout, bed_position(s), 0.0, 0, false),
            }
        }
        Source::Point { position, size, zone, snap, elevation } => {
            let [x, y, z] = position.map(|c| if c.is_finite() { c } else { 0.0 });
            let z = if elevation { z.clamp(0.0, 1.0) } else { 0.0 };
            pan(layout, [x.clamp(0.0, 1.0), y.clamp(0.0, 1.0), z], size, zone, snap)
        }
    };
    for v in &mut g {
        *v *= gain;
    }
    g
}

/// One object's gains over time: `from` at `start`, ramping linearly to `to` over `len` samples.
#[derive(Clone, Debug, Default)]
struct Track {
    from: Vec<f32>,
    to: Vec<f32>,
    start: u64,
    len: u64,
}

impl Track {
    fn at(&self, t: u64) -> Vec<f32> {
        let done = t.saturating_sub(self.start);
        if done >= self.len {
            return self.to.clone();
        }
        let f = done as f32 / self.len as f32;
        self.from.iter().zip(&self.to).map(|(a, b)| a + (b - a) * f).collect()
    }
}

/// A metadata update waiting for its sample.
#[derive(Clone, Debug)]
struct Pending {
    at: u64,
    object: usize,
    gains: Vec<f32>,
    ramp: u64,
}

/// Most objects rendered: the object presentation has at most 16 channels.
pub const MAX_OBJECTS: usize = 16;

/// Most queued updates. A stream that schedules more (say, far in the future) loses the oldest.
const MAX_PENDING: usize = 4096;

/// Renders objects to a [`Layout`], keeping each object's gains and ramps across calls.
#[derive(Clone, Debug)]
pub struct Renderer {
    layout: Layout,
    tracks: Vec<Track>,
    pending: Vec<Pending>,
}

impl Renderer {
    pub fn new(layout: Layout) -> Self {
        Self { layout, tracks: Vec::new(), pending: Vec::new() }
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Drops all object state (after a seek).
    pub fn reset(&mut self) {
        self.tracks.clear();
        self.pending.clear();
    }

    /// Sets `object` to `state` from sample `at` on, ramping over `ramp` samples.
    pub fn schedule(&mut self, at: u64, object: usize, state: &ObjectState, ramp: u64) {
        if object >= MAX_OBJECTS {
            return;
        }
        if self.pending.len() >= MAX_PENDING {
            self.pending.remove(0);
        }
        let gains = speaker_gains(&self.layout, state);
        // stable: of two updates for the same sample the later one wins
        let i = self.pending.partition_point(|p| p.at <= at);
        self.pending.insert(i, Pending { at, object, gains, ramp });
    }

    fn apply(&mut self, p: Pending) {
        let n = self.layout.channels();
        if self.tracks.len() <= p.object {
            self.tracks.resize(p.object + 1, Track { from: vec![0.0; n], to: vec![0.0; n], start: 0, len: 0 });
        }
        if let Some(t) = self.tracks.get_mut(p.object) {
            let now = t.at(p.at);
            *t = Track { from: now, to: p.gains, start: p.at, len: p.ramp };
        }
    }

    /// Mixes the objects' samples (`objects[o][k]` is object o's sample at `start + k`) into
    /// `out`, one vector per loudspeaker, each extended by the longest object's length.
    pub fn render(&mut self, start: u64, objects: &[&[f32]], out: &mut [Vec<f32>]) {
        let n = objects.iter().map(|o| o.len()).max().unwrap_or(0);
        let base: Vec<usize> = out.iter().map(|o| o.len()).collect();
        for o in out.iter_mut() {
            o.resize(o.len() + n, 0.0);
        }
        let end = start.saturating_add(n as u64);
        let mut t = start;
        while t < end {
            while self.pending.first().is_some_and(|p| p.at <= t) {
                let p = self.pending.remove(0);
                self.apply(p);
            }
            let next = self.pending.first().map_or(end, |p| p.at.min(end));
            self.mix(start, t, next, objects, out, &base);
            t = next;
        }
    }

    /// Mixes samples `t0 .. t1` (absolute; `start` is the absolute time of `objects[_][0]`).
    fn mix(&self, start: u64, t0: u64, t1: u64, objects: &[&[f32]], out: &mut [Vec<f32>], base: &[usize]) {
        let (k0, k1) = ((t0 - start) as usize, (t1 - start) as usize);
        for (track, samples) in self.tracks.iter().zip(objects) {
            let Some(src) = samples.get(k0..k1.min(samples.len())) else { continue };
            let ramping = track.start.saturating_add(track.len) > t0;
            for (ch, o) in out.iter_mut().enumerate() {
                let b = base.get(ch).copied().unwrap_or(0);
                let Some(dst) = o.get_mut(b + k0..b + k0 + src.len()) else { continue };
                if !ramping {
                    let g = track.to.get(ch).copied().unwrap_or(0.0);
                    if g != 0.0 {
                        dst.iter_mut().zip(src).for_each(|(d, s)| *d += s * g);
                    }
                    continue;
                }
                let (a, z) = (track.from.get(ch).copied().unwrap_or(0.0), track.to.get(ch).copied().unwrap_or(0.0));
                if a == 0.0 && z == 0.0 {
                    continue;
                }
                for (k, (d, s)) in dst.iter_mut().zip(src).enumerate() {
                    let done = (t0 + k as u64).saturating_sub(track.start);
                    let g = if done >= track.len { z } else { a + (z - a) * (done as f32 / track.len as f32) };
                    *d += s * g;
                }
            }
        }
    }
}

/// One metadata update from an OAMD payload: object `object` takes `state` at `offset` samples
/// after the access unit that carried it, ramping over `ramp` samples.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub offset: u64,
    pub object: usize,
    pub state: ObjectState,
    pub ramp: u64,
}

/// The updates of an OAMD payload for its first `objects` objects, or `None` for a program this
/// renderer cannot place (intermediate spatial format objects). A payload without an object
/// element gives no updates.
pub fn events(oamd: &ObjectAudioMetadataPayload, objects: usize) -> Option<Vec<Event>> {
    let prog = &oamd.program_assignment;
    if prog.num_isf_objects > 0 {
        return None;
    }
    let Some(element) = &oamd.object_element else { return Some(Vec::new()) };
    let beds: Vec<SpeakerLabels> =
        prog.bed_assignment.iter().flat_map(|b| b.to_index_vec()).filter_map(|i| u8::try_from(i).ok().and_then(SpeakerLabels::from_u8)).collect();
    let info = &element.md_update_info;
    let base = oamd.evo_sample_offset.saturating_add(info.sample_offset as u64);
    let mut out = Vec::new();
    for (b, block) in info.block_update_info.iter().enumerate() {
        let offset = base.saturating_add(u64::from(block.block_offset_factor_bits) * 32);
        let ramp = u64::from(block.ramp_duration);
        for (o, data) in element.object_data.iter().enumerate().take(objects.min(MAX_OBJECTS)) {
            let Some(blk) = data.get(b) else { continue };
            let basic = &blk.object_basic_info;
            let gain = if blk.b_object_not_active || basic.object_gain == GAIN_MINUS_INFINITY { 0.0 } else { 10f32.powf(f32::from(basic.object_gain) / 20.0) };
            let source = if o < prog.num_bed_objects {
                // a bed object without a label (a malformed bed count) stays silent
                let Some(&label) = beds.get(o) else { continue };
                Source::Bed(label)
            } else {
                let r = &blk.object_render_info;
                let mut p = r.pos3d;
                if let Some(e) = oamd.extended_object_element.as_ref().and_then(|e| e.ext_prec_pos_block.get(o)).and_then(|v| v.get(b)) {
                    p[0] += e.ext_prec_pos3d_x;
                    p[1] += e.ext_prec_pos3d_y;
                    p[2] += e.ext_prec_pos3d_z;
                }
                Source::Point {
                    position: [p[0] as f32, p[1] as f32, p[2] as f32],
                    size: r.object_size[0] as f32,
                    zone: r.zone_constraints_idx,
                    snap: r.b_object_snap,
                    elevation: r.b_enable_elevation,
                }
            };
            out.push(Event { offset, object: o, state: ObjectState { source, gain }, ramp });
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests;
