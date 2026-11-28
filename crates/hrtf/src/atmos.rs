// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the CFA4 contributors

//! Scene adapters for object audio (`docs/plans/CFA4-BINAURAL-MONITOR-001.md` §3.2): Dolby TrueHD
//! Atmos (the events of `cfa4_truehd::cfa4::ObjectDecoder`) and IAB frames (SMPTE ST 2098-2,
//! `cfa4_iab`) into a [`SceneRenderer`].
//!
//! - **TrueHD.** A bed object plays from its OAMD speaker label's direction; a dynamic object is a
//!   [`Placement::Room`] point. Snap puts it on the nearest loudspeaker of the 7.1.4 reference
//!   layout (where a 7.1.4 playback would put it), elevation off flattens it to ear height. The
//!   OAMD zones restrict loudspeakers, of which headphones have none, so they are not used. The
//!   OAMD headphone element's render mode (`hp_render_mode` 0 … 2) is not mapped: what its values
//!   mean is not in any source CFA4 may follow, so every object takes the caller's mode
//!   (Dolby's default, [`HeadphoneMode::Mid`], unless told otherwise).
//! - **IAB.** The Beds, with their activation and BedRemaps, are rendered by `cfa4_iab` to 7.1.4
//!   and each of its 12 feeds becomes a fixed loudspeaker of the scene (objects 0 … 11); the
//!   activated Objects keep their own position, gain and spread per sub-block and take the next
//!   slots, matched across frames by their MetaID. Snap and zones as for TrueHD.

use cfa4_iab::Frame;
use cfa4_truehd::render::{Event, Layout as TrueHdLayout, ObjectState, Source};
use cfa4_truehd::structs::oamd::SpeakerLabels;

use crate::scene::{HeadphoneMode, MAX_OBJECTS, Placement, SceneRenderer, SceneState, room_direction};
use crate::{Error, Result, Speaker};

/// The scene loudspeaker of an OAMD bed label.
pub fn truehd_speaker(label: SpeakerLabels) -> Speaker {
    use SpeakerLabels as S;
    match label {
        S::L => Speaker::L,
        S::R => Speaker::R,
        S::C => Speaker::C,
        S::LFE | S::LFE2 => Speaker::Lfe,
        S::Lss => Speaker::Lss,
        S::Rss => Speaker::Rss,
        S::Lrs => Speaker::Lrs,
        S::Rrs => Speaker::Rrs,
        S::Lfh => Speaker::Ltf,
        S::Rfh => Speaker::Rtf,
        S::Lts => Speaker::Ltm,
        S::Rts => Speaker::Rtm,
        S::Lrh => Speaker::Ltr,
        S::Rrh => Speaker::Rtr,
        S::Lw => Speaker::At(6_000, 0),
        S::Rw => Speaker::At(-6_000, 0),
    }
}

/// The room-cube position of the 7.1.4 loudspeaker nearest `p`.
fn snapped(p: [f32; 3]) -> [f32; 3] {
    let layout = TrueHdLayout::surround_7_1_4();
    let d2 = |q: [f32; 3]| q.iter().zip(p).map(|(a, b)| (a - b) * (a - b)).sum::<f32>();
    layout.speakers.iter().filter(|s| !s.lfe).map(|s| s.position).min_by(|a, b| d2(*a).total_cmp(&d2(*b))).unwrap_or(p)
}

/// A TrueHD object state as a scene state.
pub fn truehd_state(state: &ObjectState, mode: HeadphoneMode) -> SceneState {
    let placement = match state.source {
        Source::Bed(label) => Placement::Speaker(truehd_speaker(label)),
        Source::Point { position, size, zone: _, snap, elevation } => {
            let mut p = position;
            if !elevation {
                p[2] = 0.0;
            }
            if snap { Placement::Room { position: snapped(p), size: 0.0 } } else { Placement::Room { position: p, size } }
        }
    };
    SceneState { placement, gain: state.gain, mode }
}

/// Schedules TrueHD events (offsets from the start of the stream) on `scene`. Events for objects
/// past the scene's limit are skipped.
pub fn schedule_truehd(scene: &mut SceneRenderer, events: &[Event], mode: HeadphoneMode) {
    for e in events {
        // past the limit: the object stays silent, as the loudspeaker renderer does
        let _ = scene.schedule(e.offset, e.object, truehd_state(&e.state, mode), e.ramp);
    }
}

/// Feeds IAB frames to a [`SceneRenderer`], keeping the Objects' slots across frames.
pub struct IabScene {
    layout: cfa4_iab::Layout,
    /// The MetaID of each object slot past the beds.
    slots: Vec<u32>,
    mode: HeadphoneMode,
    /// The current frame's signals, one per scene object.
    signals: Vec<Vec<f32>>,
}

/// Objects of the scene the IAB Beds take (the 7.1.4 feeds).
pub const IAB_BED_FEEDS: usize = 12;

impl IabScene {
    /// An adapter whose Objects take `mode`.
    pub fn new(mode: HeadphoneMode) -> Self {
        IabScene { layout: cfa4_iab::Layout::surround_7_1_4(), slots: Vec::new(), mode, signals: Vec::new() }
    }

    /// Forgets the Objects' slots (after a seek, with [`SceneRenderer::reset`]).
    pub fn reset(&mut self) {
        self.slots.clear();
    }

    fn slot(&mut self, meta_id: u32) -> Option<usize> {
        let i = match self.slots.iter().position(|&m| m == meta_id) {
            Some(i) => i,
            None => {
                self.slots.push(meta_id);
                self.slots.len() - 1
            }
        };
        let slot = IAB_BED_FEEDS + i;
        (slot < MAX_OBJECTS).then_some(slot)
    }

    /// Renders one frame starting at sample `start` into `left` and `right` (overwritten; at most
    /// [`Frame::samples`] of them).
    pub fn render(&mut self, scene: &mut SceneRenderer, frame: &Frame, start: u64, left: &mut [f32], right: &mut [f32]) -> Result<()> {
        if frame.sample_rate != scene.hrtf().sample_rate() {
            return Err(Error::Invalid("the IAB frame's rate differs from the HRTF set's"));
        }
        let n = frame.samples();
        let beds = cfa4_iab::render_beds(frame, &self.layout).map_err(|_| Error::Invalid("IAB beds do not render"))?;
        self.signals.resize_with(IAB_BED_FEEDS, Vec::new);
        for (i, (speaker, feed)) in self.layout.speakers.iter().zip(beds).enumerate().take(IAB_BED_FEEDS) {
            let placement = if speaker.lfe {
                Placement::Speaker(Speaker::Lfe)
            } else {
                let (a, e, _) = room_direction(speaker.position, 1.0);
                Placement::Speaker(Speaker::At((a * 100.0).round() as i32, (e * 100.0).round() as i32))
            };
            // the same state every frame: the scene ignores an unchanged position
            scene.schedule(start, i, SceneState { placement, gain: 1.0, mode: self.mode }, 0)?;
            self.signals[i] = feed;
        }
        let sizes = frame.frame_rate.sub_block_sizes(frame.rate96(), frame.frame_index);
        let mut live = vec![false; self.slots.len()];
        for o in cfa4_iab::rendered_objects_of(frame, &self.layout) {
            let Some(slot) = self.slot(o.meta_id) else { continue };
            if let Some(l) = live.get_mut(slot - IAB_BED_FEEDS) {
                *l = true;
            } else {
                live.resize(slot - IAB_BED_FEEDS + 1, false);
                live[slot - IAB_BED_FEEDS] = true;
            }
            if self.signals.len() <= slot {
                self.signals.resize_with(slot + 1, Vec::new);
            }
            const SCALE: f32 = 1.0 / 2_147_483_648.0;
            let signal = &mut self.signals[slot];
            signal.clear();
            match frame.audio_data(o.audio_id) {
                Some(a) => signal.extend(a.samples.iter().map(|&s| s as f32 * SCALE)),
                None => signal.resize(n, 0.0),
            }
            let mut at = start;
            for (sb, &len) in sizes.iter().enumerate() {
                if let Some(Some(p)) = o.pan.get(sb) {
                    let ext = p.spread.extent();
                    let position = p.position();
                    let placement = match p.snap {
                        cfa4_iab::Snap::On(_) => Placement::Room { position: snapped(position), size: 0.0 },
                        cfa4_iab::Snap::Off => Placement::Room { position, size: (ext[0] + ext[1] + ext[2]) / 3.0 },
                    };
                    // IAB ramps gains across each sub-block
                    let ramp = if sb == 0 { 0 } else { len as u64 };
                    scene.schedule(at, slot, SceneState { placement, gain: p.gain.linear(), mode: self.mode }, ramp)?;
                }
                at += len as u64;
            }
        }
        // an Object absent from this frame is silent in it
        for (i, l) in live.iter().enumerate() {
            if !l && let Some(s) = self.signals.get_mut(IAB_BED_FEEDS + i) {
                s.clear();
            }
        }
        let inputs: Vec<&[f32]> = self.signals.iter().map(|s| s.as_slice()).collect();
        let frames = n.min(left.len()).min(right.len());
        scene.render(start, &inputs, &mut left[..frames], &mut right[..frames]);
        Ok(())
    }
}
