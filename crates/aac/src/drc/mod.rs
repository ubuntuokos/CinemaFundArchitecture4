//! MPEG-D DRC (ISO/IEC 23003-4) decoder: dynamic range control, loudness normalization and the
//! peak limiter, as used by xHE-AAC ([`crate::usac`]).
//!
//! Supported: `uniDrcConfig()` / `loudnessInfoSet()` versions 0 and 1, `uniDrcGain()` with every
//! gain coding profile, spline and linear interpolation, gain scaling/offset modifiers, the DRC set
//! selection for the base channel layout (default or requested effect type, target loudness) and
//! a 5 ms look-ahead peak limiter. Not supported (such sets are not selected): multi-band DRC,
//! parametric DRC, ducking, EQ and loudness EQ, downmix-specific sets, characteristic mapping.
//!
//! Ported from Android libxaac `decoder/drc_src` (<https://github.com/ittiam-systems/libxaac>):
//!
//! ```text
//! Copyright (C) 2018 The Android Open Source Project
//!
//! Licensed under the Apache License, Version 2.0 (the "License");
//! you may not use this file except in compliance with the License.
//! You may obtain a copy of the License at:
//!
//! http://www.apache.org/licenses/LICENSE-2.0
//!
//! Unless required by applicable law or agreed to in writing, software
//! distributed under the License is distributed on an "AS IS" BASIS,
//! WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//! See the License for the specific language governing permissions and
//! limitations under the License.
//!
//! Originally developed and contributed by Ittiam Systems Pvt. Ltd, Bangalore
//! ```

pub mod config;
pub mod gain;
pub mod limiter;
pub mod select;

pub use config::{DrcConfig, LoudnessInfoSet};
pub use select::{DrcSettings, Selection};

use crate::{Error, Result};
use config::{Coefficients, EFFECT_CLIPPING, EFFECT_DUCK_OTHER, EFFECT_DUCK_SELF, EFFECT_FADE};
use gain::{GainBuffer, GainMapping, Node, TimeParams};
use limiter::PeakLimiter;

/// One processed channel group of the selected DRC set.
#[derive(Debug, Clone)]
struct Group {
    sequence: usize,
    map: GainMapping,
    buffer: GainBuffer,
    /// Channels of the group.
    channels: Vec<usize>,
}

/// Applies the selected DRC set, the loudness normalization gain and the peak limiter to decoded
/// PCM, one codec frame at a time.
#[derive(Debug, Clone)]
pub struct DrcProcessor {
    config: Option<DrcConfig>,
    selection: Selection,
    coefficients: Option<Coefficients>,
    time: TimeParams,
    groups: Vec<Group>,
    norm_gain: f32,
    limiter: Option<PeakLimiter>,
}

impl DrcProcessor {
    /// `drc_config`: `uniDrcConfig()` bytes, `loudness`: `loudnessInfoSet()` bytes (both optional);
    /// `frame`: samples per codec frame.
    pub fn new(
        drc_config: Option<&[u8]>,
        loudness: Option<&[u8]>,
        channels: usize,
        sample_rate: u32,
        frame: usize,
        settings: &DrcSettings,
    ) -> Result<DrcProcessor> {
        if frame == 0 || frame > gain::MAX_FRAME || sample_rate < 1000 {
            return Err(Error::InvalidConfig("DRC: unsupported frame size or sampling rate"));
        }
        let config = drc_config.map(DrcConfig::parse).transpose()?;
        let loud = loudness.map(LoudnessInfoSet::parse).transpose()?.unwrap_or_default();
        let selection = select::select(config.as_ref(), &loud, settings);
        let mut p = DrcProcessor {
            config: None,
            selection,
            coefficients: None,
            time: TimeParams { frame_size: frame, delta_tmin: gain::delta_tmin_default(sample_rate) },
            groups: Vec::new(),
            norm_gain: 10f32.powf(selection.loudness_normalization_db / 20.0),
            limiter: None,
        };
        if let (Some(cfg), Some(k)) = (&config, selection.instructions)
            && let Some(ins) = cfg.instructions.get(k)
            && let Some(coef) = cfg.coefficients_for(ins.location)
        {
            if let Some(fs) = coef.frame_size {
                p.time.frame_size = (fs as usize).clamp(1, gain::MAX_FRAME);
            }
            let modification = ins.effect & (EFFECT_DUCK_OTHER | EFFECT_DUCK_SELF) == 0 && ins.effect != EFFECT_FADE && ins.effect != EFFECT_CLIPPING;
            for (g, &(set, modifiers)) in ins.groups.iter().enumerate() {
                let Some(gs) = coef.gain_sets.get(set) else { continue };
                if gs.band_sequences.len() > 1 {
                    return Err(Error::Unsupported("multi-band MPEG-D DRC"));
                }
                let map = GainMapping {
                    modifiers,
                    modification,
                    compress: settings.compress,
                    boost: settings.boost,
                    clipping: (ins.effect == EFFECT_CLIPPING)
                        .then(|| (ins.limiter_peak_target.unwrap_or(0.0), selection.loudness_normalization_db))
                        .filter(|_| ins.limiter_peak_target.is_some()),
                    spline: gs.spline,
                    delta_tmin: gs.time_delta_min.unwrap_or(p.time.delta_tmin),
                };
                let channels_of: Vec<usize> =
                    ins.channel_group.iter().enumerate().filter(|(_, cg)| **cg == g as i32).map(|(c, _)| c).filter(|&c| c < channels).collect();
                p.groups.push(Group {
                    sequence: gs.band_sequences.first().copied().unwrap_or(0),
                    map,
                    buffer: GainBuffer::new(p.time.frame_size),
                    channels: channels_of,
                });
            }
            p.coefficients = Some(coef.clone());
        }
        if settings.peak_limiter && settings.target_loudness.is_some() && selection.output_peak_level_db > 0.0 {
            p.limiter = Some(PeakLimiter::new(channels, sample_rate));
        }
        p.config = config;
        Ok(p)
    }

    /// The selected DRC set and loudness normalization.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Selected `drcSetId` (`None` = no DRC).
    pub fn drc_set_id(&self) -> Option<i32> {
        let k = self.selection.instructions?;
        self.config.as_ref()?.instructions.get(k).map(|i| i.drc_set_id)
    }

    /// Output delay added by the peak limiter, in samples.
    pub fn delay(&self) -> usize {
        self.limiter.as_ref().map_or(0, PeakLimiter::delay)
    }

    /// Advance by one frame without output (a decoded and discarded pre-roll frame).
    pub fn skip(&mut self, gain_payload: Option<&[u8]>) -> Result<()> {
        self.apply_drc(gain_payload, &mut [])
    }

    /// Process one frame of planar PCM in place. `gain_payload` is the frame's `uniDrcGain()`.
    pub fn process(&mut self, gain_payload: Option<&[u8]>, pcm: &mut [Vec<f32>]) -> Result<()> {
        self.apply_drc(gain_payload, pcm)?;
        if self.norm_gain != 1.0 {
            for ch in pcm.iter_mut() {
                for s in ch.iter_mut() {
                    *s *= self.norm_gain;
                }
            }
        }
        if let Some(l) = &mut self.limiter {
            l.process(pcm);
        }
        Ok(())
    }

    /// Decode the frame's DRC gains and apply them to `pcm` (which may be empty).
    fn apply_drc(&mut self, gain_payload: Option<&[u8]>, pcm: &mut [Vec<f32>]) -> Result<()> {
        if let Some(coef) = &self.coefficients {
            let sequences = match gain_payload {
                Some(p) => Some(gain::parse_gain(p, coef, self.time)?),
                None => None,
            };
            for g in &mut self.groups {
                let nodes = match sequences.as_ref().and_then(|s| s.get(g.sequence)) {
                    Some(n) => n.clone(),
                    // no payload: hold the last gain
                    None => vec![Node { time: self.time.frame_size as i32 - 1, gain_db: g.buffer.last_gain_db(), slope: 0.0 }],
                };
                g.buffer.push(&nodes, &g.map);
                let gains = g.buffer.current();
                for &c in &g.channels {
                    if let Some(ch) = pcm.get_mut(c) {
                        for (s, &k) in ch.iter_mut().zip(gains) {
                            *s *= k;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usac::config::UsacConfig;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// `Test_DRC.mp4` (Fraunhofer IIS): one gain set, DRC set 1, programme loudness -29 LKFS.
    const TEST_DRC_ASC: &str = "f94623213cc0d3c10000000a47a522008240c0020908009dfc010000c0085a00113840020000244011732c00";

    #[test]
    fn selects_the_drc_set_and_normalizes() {
        let cfg = UsacConfig::from_asc(&hex(TEST_DRC_ASC)).unwrap();
        let p = DrcProcessor::new(cfg.uni_drc_config(), cfg.loudness_info(), 1, 48_000, 1024, &DrcSettings::default()).unwrap();
        assert_eq!(p.drc_set_id(), Some(1));
        assert_eq!(p.selection().loudness_normalization_db, 13.0);
        assert_eq!(p.delay(), 0);
        // DRC off and no normalization: nothing to do
        let off = DrcSettings { target_loudness: None, drc: false, ..DrcSettings::default() };
        let mut p = DrcProcessor::new(cfg.uni_drc_config(), cfg.loudness_info(), 1, 48_000, 1024, &off).unwrap();
        assert_eq!(p.drc_set_id(), None);
        let mut pcm = vec![vec![0.25f32; 1024]];
        p.process(None, &mut pcm).unwrap();
        assert!(pcm[0].iter().all(|&s| s == 0.25));
    }

    #[test]
    fn hostile_input_does_not_panic() {
        let cfg = UsacConfig::from_asc(&hex(TEST_DRC_ASC)).unwrap();
        let (drc, loud) = (cfg.uni_drc_config().unwrap().to_vec(), cfg.loudness_info().unwrap().to_vec());
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for i in 0..2000 {
            let mut d = drc.clone();
            let mut l = loud.clone();
            let k = rand() as usize;
            if let Some(b) = d.get_mut(k % drc.len()) {
                *b ^= 1 << (k % 8);
            }
            if let Some(b) = l.get_mut((k >> 8) % loud.len()) {
                *b ^= rand() as u8;
            }
            d.truncate(drc.len() - i % 4);
            let Ok(mut p) = DrcProcessor::new(Some(&d), Some(&l), 2, 48_000, 1024, &DrcSettings::default()) else { continue };
            let mut pcm = vec![vec![0.5f32; 1024]; 2];
            for _ in 0..4 {
                let payload: Vec<u8> = (0..rand() % 16).map(|_| rand() as u8).collect();
                let _ = p.process(Some(&payload), &mut pcm);
                let _ = p.skip(Some(&payload));
            }
            assert!(pcm.iter().flatten().all(|s| s.is_finite()));
        }
    }
}
