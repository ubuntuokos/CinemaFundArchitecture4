//! A reference renderer: an IAFrame → loudspeaker feeds.
//!
//! ST 2098-2 leaves rendering to the cinema processor (ST 2098-3 describes expectations only).
//! This renderer follows the bitstream's rules for which elements play (10.4.2, 10.6.1: UseCase
//! activation, conditional replacements, BedRemap) and places sound with plain, documented
//! choices:
//!
//! - A Bed channel goes to the loudspeaker with its ChannelID when the layout has one; otherwise
//!   it is panned from the channel's nominal position in the room ([`channel_position`]). LFE
//!   channels go to the layout's LFE and are dropped when it has none.
//! - Objects and unmatched channels are panned by distance: each full-range loudspeaker gets a
//!   weight 1 / (d² + ε)² from its distance d to the source, scaled by the zone gain of its zone;
//!   weights under 5 % of the total are dropped (no far loudspeaker leaks) and the rest normalised
//!   to constant power. Spread widens ε; snap sends the object to its nearest loudspeaker within
//!   the tolerance.
//! - Gains change per sub-block with a linear ramp across the sub-block. Decorrelation is not
//!   applied, and ObjectZoneDefinition19 zones are not used (the nine zones of Table 25 are).

use crate::{Bed, Frame, Object, Pan, Result, Snap, ZoneGain};

/// One loudspeaker of a [`Layout`].
#[derive(Clone, Debug, PartialEq)]
pub struct Speaker {
    /// ChannelID (Table 20) of the loudspeaker.
    pub id: u32,
    pub label: &'static str,
    /// Position in the unit room cube: x left → right, y front → back, z ear height → ceiling.
    pub position: [f32; 3],
    pub lfe: bool,
}

/// An output loudspeaker layout, in output channel order.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub name: &'static str,
    pub speakers: Vec<Speaker>,
    /// UseCase codes this layout is the Target Environment of.
    pub use_cases: Vec<u8>,
}

/// Nominal position of a ChannelID (Table 20) in the unit room cube, and whether it is an LFE
/// channel; `None` for reserved codes.
pub fn channel_position(id: u32) -> Option<([f32; 3], bool)> {
    let p = |x: f32, y: f32, z: f32| Some(([x, y, z], false));
    match id {
        0x00 => p(0.0, 0.0, 0.0),
        0x01 => p(0.25, 0.0, 0.0),
        0x02 => p(0.5, 0.0, 0.0),
        0x03 => p(0.75, 0.0, 0.0),
        0x04 => p(1.0, 0.0, 0.0),
        0x05 => p(0.0, 0.5, 0.0),
        0x06 => p(0.0, 0.75, 0.0),
        0x07 => p(0.15, 1.0, 0.0),
        0x08 => p(0.85, 1.0, 0.0),
        0x09 => p(1.0, 0.5, 0.0),
        0x0A => p(1.0, 0.75, 0.0),
        0x0B => p(0.0, 0.5, 1.0),
        0x0C => p(1.0, 0.5, 1.0),
        0x0D | 0x86 | 0x87 => Some(([0.5, 0.0, 0.0], true)),
        0x0E => p(0.0, 0.0, 1.0),
        0x0F => p(1.0, 0.0, 1.0),
        0x10 => p(0.5, 0.0, 1.0),
        0x11 => p(0.0, 0.75, 1.0),
        0x12 => p(1.0, 0.75, 1.0),
        0x13 => p(0.0, 0.5, 1.0),
        0x14 => p(1.0, 0.5, 1.0),
        0x15 => p(0.15, 1.0, 1.0),
        0x16 => p(0.85, 1.0, 1.0),
        0x17 => p(0.5, 0.5, 1.0),
        0x80 => p(0.25, 0.25, 1.0),
        0x81 => p(0.75, 0.25, 1.0),
        0x82 => p(0.25, 0.75, 1.0),
        0x83 => p(0.75, 0.75, 1.0),
        0x84 => p(0.0, 0.5, 1.0),
        0x85 => p(1.0, 0.5, 1.0),
        0x88 => p(0.0, 0.25, 0.0),
        0x89 => p(1.0, 0.25, 0.0),
        _ => None,
    }
}

fn speakers(list: &[(u32, &'static str)]) -> Vec<Speaker> {
    list.iter().filter_map(|&(id, label)| channel_position(id).map(|(position, lfe)| Speaker { id, label, position, lfe })).collect()
}

impl Layout {
    /// L R.
    pub fn stereo() -> Layout {
        Layout { name: "2.0", speakers: speakers(&[(0x00, "L"), (0x04, "R")]), use_cases: vec![crate::use_case::ITU_A] }
    }
    /// L R C LFE Ls Rs (WAV / SMPTE order).
    pub fn surround_5_1() -> Layout {
        Layout {
            name: "5.1",
            speakers: speakers(&[(0x00, "L"), (0x04, "R"), (0x02, "C"), (0x0D, "LFE"), (0x06, "Ls"), (0x0A, "Rs")]),
            use_cases: vec![crate::use_case::SURROUND_5_1],
        }
    }
    /// L R C LFE Lrs Rrs Lss Rss (WAV order: back surrounds before side surrounds).
    pub fn surround_7_1() -> Layout {
        Layout {
            name: "7.1",
            speakers: speakers(&[(0x00, "L"), (0x04, "R"), (0x02, "C"), (0x0D, "LFE"), (0x07, "Lrs"), (0x08, "Rrs"), (0x05, "Lss"), (0x09, "Rss")]),
            use_cases: vec![crate::use_case::SURROUND_7_1_DS],
        }
    }
    /// 7.1 and four overhead loudspeakers: Ltf Rtf Ltb Rtb (ITU-R BS.2051 system J, 4+7+0).
    pub fn surround_7_1_4() -> Layout {
        let mut l = Layout::surround_7_1();
        l.name = "7.1.4";
        l.speakers.extend(speakers(&[(0x80, "Ltf"), (0x81, "Rtf"), (0x82, "Ltb"), (0x83, "Rtb")]));
        l.use_cases = vec![crate::use_case::ITU_J];
        l
    }
    pub fn channels(&self) -> usize {
        self.speakers.len()
    }
    /// Whether an element with this UseCase (`None`: unconditional) is activated here.
    pub fn activates(&self, use_case: Option<u8>) -> bool {
        use_case.is_none_or(|u| u == crate::use_case::ALWAYS || self.use_cases.contains(&u))
    }
}

/// Zone of Table 25 a loudspeaker belongs to.
fn zone9(p: [f32; 3]) -> usize {
    let [x, y, z] = p;
    if z >= 0.5 {
        if x < 0.5 { 7 } else { 8 }
    } else if y < 0.1 {
        if x < 0.4 {
            0
        } else if x <= 0.6 {
            1
        } else {
            2
        }
    } else if y > 0.9 {
        if x < 0.5 { 5 } else { 6 }
    } else if x < 0.5 {
        3
    } else {
        4
    }
}

/// Constant-power distance panning of a point at `p` (see the module notes).
fn pan_point(layout: &Layout, p: [f32; 3], spread: [f32; 3], zones: Option<&[ZoneGain; 9]>, snap: Snap) -> Vec<f32> {
    let zone = |s: &Speaker| zones.map_or(1.0, |z| z.get(zone9(s.position)).map_or(1.0, |g| g.linear()));
    let mut g = vec![0.0; layout.speakers.len()];
    if let Snap::On(tol) = snap {
        let tol = tol.map_or(1.0, |t| f32::from(t.min(4095)) / 4095.0);
        let nearest = layout
            .speakers
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.lfe && zone(s) > 0.0)
            .map(|(i, s)| (i, s.position.iter().zip(p).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max)))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, d)) = nearest
            && d <= tol
            && let Some(v) = g.get_mut(i)
        {
            *v = 1.0;
            return g;
        }
    }
    let eps = 0.004 + 0.5 * spread.iter().map(|s| s * s).sum::<f32>() / 3.0;
    for (v, s) in g.iter_mut().zip(&layout.speakers) {
        if s.lfe {
            continue;
        }
        let d2: f32 = s.position.iter().zip(p).map(|(a, b)| (a - b) * (a - b)).sum();
        *v = zone(s) / ((d2 + eps) * (d2 + eps));
    }
    let sum: f32 = g.iter().sum();
    for v in &mut g {
        if *v < 0.05 * sum {
            *v = 0.0;
        }
    }
    let norm = g.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut g {
            *v /= norm;
        }
    }
    g
}

/// Loudspeaker gains of a Bed channel or remap destination.
fn route(layout: &Layout, id: u32) -> Vec<f32> {
    let mut g = vec![0.0; layout.speakers.len()];
    if let Some(i) = layout.speakers.iter().position(|s| s.id == id) {
        g[i] = 1.0;
        return g;
    }
    match channel_position(id) {
        Some((_, true)) => {
            if let Some(i) = layout.speakers.iter().position(|s| s.lfe) {
                g[i] = 1.0;
            }
            g
        }
        Some((p, false)) => pan_point(layout, p, [0.0; 3], None, Snap::Off),
        None => g,
    }
}

/// The Beds that render: activated, all ancestors activated, no activated child Bed.
fn rendered_beds<'a>(list: &'a [Bed], layout: &Layout, out: &mut Vec<&'a Bed>) {
    for b in list.iter().filter(|b| layout.activates(b.use_case)) {
        if b.beds.iter().any(|c| layout.activates(c.use_case)) {
            rendered_beds(&b.beds, layout, out);
        } else {
            out.push(b);
        }
    }
}

fn rendered_objects<'a>(list: &'a [Object], layout: &Layout, out: &mut Vec<&'a Object>) {
    for o in list.iter().filter(|o| layout.activates(o.use_case)) {
        if o.objects.iter().any(|c| layout.activates(c.use_case)) {
            rendered_objects(&o.objects, layout, out);
        } else {
            out.push(o);
        }
    }
}

/// Adds `x` to the outputs with one gain vector per sub-block, ramping linearly from the previous
/// sub-block's gains across each sub-block.
fn mix(out: &mut [Vec<f32>], x: &[i32], gains: &[Vec<f32>], sizes: &[usize]) {
    const SCALE: f32 = 1.0 / 2_147_483_648.0;
    let mut start = 0;
    for (b, (g, &len)) in gains.iter().zip(sizes).enumerate() {
        let prev = if b == 0 { g } else { gains.get(b - 1).unwrap_or(g) };
        let end = (start + len).min(x.len());
        let span = x.get(start..end).unwrap_or_default();
        for (ch, o) in out.iter_mut().enumerate() {
            let (g0, g1) = (prev.get(ch).copied().unwrap_or(0.0), g.get(ch).copied().unwrap_or(0.0));
            if g0 == 0.0 && g1 == 0.0 {
                continue;
            }
            let Some(dst) = o.get_mut(start..end) else { continue };
            let step = (g1 - g0) / len.max(1) as f32;
            for (k, (d, &s)) in dst.iter_mut().zip(span).enumerate() {
                let gk = if g0 == g1 { g1 } else { g0 + step * (k + 1) as f32 };
                *d += s as f32 * SCALE * gk;
            }
        }
        start = end;
    }
}

/// Renders `frame` to the loudspeakers of `layout`: one buffer per loudspeaker, [`Frame::samples`]
/// samples each, full scale ±1.0.
pub fn render(frame: &Frame, layout: &Layout) -> Result<Vec<Vec<f32>>> {
    let n = frame.samples();
    let sizes = frame.frame_rate.sub_block_sizes(frame.rate96(), frame.frame_index);
    let ns = layout.speakers.len();
    let mut out = vec![vec![0.0f32; n]; ns];
    let scaled = |v: Vec<f32>, k: f32| v.into_iter().map(|g| g * k).collect::<Vec<f32>>();

    let mut beds = Vec::new();
    rendered_beds(&frame.beds, layout, &mut beds);
    for bed in beds {
        let remap = bed.remaps.iter().find(|r| layout.activates(Some(r.use_case)));
        for (i, ch) in bed.channels.iter().enumerate() {
            let Some(a) = frame.audio_data(ch.audio_id) else { continue };
            let k = ch.gain.linear();
            let gains: Vec<Vec<f32>> = match remap {
                None => vec![scaled(route(layout, ch.id), k); sizes.len()],
                Some(r) => {
                    let mut cur = None;
                    let mut per_block = Vec::with_capacity(sizes.len());
                    for sb in 0..sizes.len() {
                        if let Some(Some(b)) = r.blocks.get(sb) {
                            cur = Some(b);
                        }
                        let mut g = vec![0.0; ns];
                        for (o, dest) in cur.map(|b| b.destinations.as_slice()).unwrap_or_default().iter().enumerate() {
                            let rg = cur.and_then(|b| b.gains.get(o)).and_then(|row| row.get(i)).map_or(0.0, |g| g.linear());
                            if rg != 0.0 {
                                for (a, b) in g.iter_mut().zip(route(layout, *dest)) {
                                    *a += rg * b;
                                }
                            }
                        }
                        per_block.push(scaled(g, k));
                    }
                    per_block
                }
            };
            mix(&mut out, &a.samples, &gains, &sizes);
        }
    }

    let mut objects = Vec::new();
    rendered_objects(&frame.objects, layout, &mut objects);
    for o in objects {
        let Some(a) = frame.audio_data(o.audio_id) else { continue };
        let mut cur: Option<&Pan> = None;
        let mut gains = Vec::with_capacity(sizes.len());
        for sb in 0..sizes.len() {
            if let Some(Some(p)) = o.pan.get(sb) {
                cur = Some(p);
            }
            let g = match cur {
                Some(p) => scaled(pan_point(layout, p.position(), p.spread.extent(), p.zones.as_ref(), p.snap), p.gain.linear()),
                None => vec![0.0; ns],
            };
            gains.push(g);
        }
        mix(&mut out, &a.samples, &gains, &sizes);
    }
    Ok(out)
}
