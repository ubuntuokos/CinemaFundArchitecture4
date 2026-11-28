// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Sirac Ozmen
//
// Ported to Rust for CFA4 from Maul Audio (https://github.com/Rawframe-Project/maul-audio, commit
// aeb383458250759f92a3e724a5bd006505e45b4d, src/hrtf.c, src/hrtf_core.h, src/hrtf_lookup.{h,c}).
// CFA4 changes: owned Vec storage instead of one allocator block; the built-in set.

//! A loaded HRTF set and the lookup of a direction on its rings: the two rings around the
//! elevation and on each the two azimuths around the azimuth, weighted bilinearly. Rings of evenly
//! spaced azimuths make the lookup a division, not a search.

use crate::file::HrtfFile;
use crate::resample::{Resampler, resampled_taps};
use crate::{Error, Result};

/// The built-in set: SADIE II, subject D1 (Neumann KU100), 48 kHz (Apache-2.0, `assets/hrtf/`).
const BUILTIN: &[u8] = include_bytes!("../../../assets/hrtf/sadie2-ku100-48k.maudhrtf");

/// The largest response a resampled set may have (taps), so a hostile rate cannot ask for
/// gigabytes: 1024 taps at 8 kHz resampled to 384 kHz.
const MAX_RESAMPLED_TAPS: u32 = 1024 * 48 + 1200;

/// One HRTF set at one sample rate, read only once loaded and shared by any number of renderers.
#[derive(Clone, Debug)]
pub struct HrtfSet {
    sample_rate: u32,
    taps: usize,
    distance: f32,
    elevations: Vec<f32>,
    azimuths: Vec<u32>,
    first_direction: Vec<u32>,
    /// Per direction, left then right: the onset delay in samples.
    delays: Vec<f32>,
    /// Per direction, left then right: the minimum-phase response, `taps` of them.
    responses: Vec<f32>,
    name: String,
    license: String,
}

/// Four measured directions and their weights, which add up to 1: two azimuths on the ring at or
/// below the elevation, two on the ring above.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Neighbours {
    pub direction: [u32; 4],
    pub weight: [f32; 4],
}

impl HrtfSet {
    /// The built-in SADIE II KU100 set, at `sample_rate` (resampled from 48 kHz when it differs).
    pub fn builtin(sample_rate: u32) -> Result<Self> {
        Self::from_maudhrtf(BUILTIN, Some(sample_rate))
    }

    /// Loads a `.maudhrtf` file, at its own rate or resampled to `sample_rate`.
    pub fn from_maudhrtf(bytes: &[u8], sample_rate: Option<u32>) -> Result<Self> {
        let file = HrtfFile::read(bytes)?;
        let rate = sample_rate.unwrap_or(file.sample_rate);
        if !(8_000..=384_000).contains(&rate) {
            return Err(Error::Invalid("sample rate must be 8000 to 384000 Hz"));
        }
        let taps = resampled_taps(file.taps, file.sample_rate, rate);
        if taps > MAX_RESAMPLED_TAPS {
            return Err(Error::Invalid("resampled responses too long"));
        }
        let rings = file.ring_count as usize;
        let dirs = file.direction_count as usize;
        let mut elevations = Vec::with_capacity(rings);
        let mut azimuths = Vec::with_capacity(rings);
        let mut first_direction = Vec::with_capacity(rings);
        let mut first = 0u32;
        for ring in 0..file.ring_count {
            elevations.push(file.elevation(ring));
            azimuths.push(file.azimuths(ring));
            first_direction.push(first);
            first += file.azimuths(ring);
        }
        let ratio = rate as f32 / file.sample_rate as f32;
        let resampler = (rate != file.sample_rate).then(|| Resampler::new(file.taps, file.sample_rate, rate));
        let taps = taps as usize;
        let mut delays = vec![0.0f32; 2 * dirs];
        let mut responses = vec![0.0f32; 2 * dirs * taps];
        let mut scratch = vec![0.0f32; file.taps as usize];
        for direction in 0..file.direction_count {
            for ear in 0..2 {
                let at = 2 * direction as usize + ear;
                delays[at] = file.delay(direction, ear) * ratio;
                let response = &mut responses[at * taps..(at + 1) * taps];
                match &resampler {
                    None => file.response(direction, ear, response),
                    Some(r) => {
                        file.response(direction, ear, &mut scratch);
                        r.run(&scratch, response);
                    }
                }
            }
        }
        debug_assert!(resampler.as_ref().is_none_or(|r| r.out_taps() as usize == taps));
        Ok(HrtfSet {
            sample_rate: rate,
            taps,
            distance: file.distance,
            elevations,
            azimuths,
            first_direction,
            delays,
            responses,
            name: file.name.to_owned(),
            license: file.license.to_owned(),
        })
    }

    /// The rate the responses are at.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Taps per response.
    pub fn taps(&self) -> usize {
        self.taps
    }

    /// Metres from the head's centre at which the set was measured.
    pub fn distance(&self) -> f32 {
        self.distance
    }

    /// Measured directions.
    pub fn direction_count(&self) -> usize {
        self.delays.len() / 2
    }

    /// The dataset and subject.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The dataset's licence and attribution text, which travels with the set.
    pub fn license(&self) -> &str {
        &self.license
    }

    /// The largest onset delay of any direction and ear, in samples.
    pub fn largest_delay(&self) -> f32 {
        self.delays.iter().copied().fold(0.0, f32::max)
    }

    /// The ring at or below an elevation already within the rings' range, short of the last ring
    /// when there are two or more.
    fn ring_below(&self, elevation: f32) -> usize {
        let mut low = 0;
        let mut high = self.elevations.len() - 1;
        while high - low > 1 {
            let middle = low + (high - low) / 2;
            if self.elevations[middle] <= elevation {
                low = middle;
            } else {
                high = middle;
            }
        }
        low
    }

    /// The two directions around an azimuth on a ring, and how far towards the second it lies.
    fn around_on_ring(&self, ring: usize, azimuth: f32) -> (u32, u32, f32) {
        let count = self.azimuths[ring];
        let position = azimuth * count as f32 / 360.0;
        let index = (position as u32).min(count - 1);
        let fraction = position - index as f32;
        let first = self.first_direction[ring];
        (first + index, first + if index + 1 == count { 0 } else { index + 1 }, fraction)
    }

    /// The neighbours of a direction in the set's convention: azimuth in degrees counterclockwise
    /// from straight ahead (any value; it wraps), elevation in degrees up from the horizontal
    /// plane (clamped to the set's lowest and highest rings). A non-finite angle counts as 0.
    pub fn neighbours(&self, azimuth_degrees: f32, elevation_degrees: f32) -> Neighbours {
        let azimuth_degrees = if azimuth_degrees.is_finite() { azimuth_degrees } else { 0.0 };
        let elevation_degrees = if elevation_degrees.is_finite() { elevation_degrees } else { 0.0 };
        let mut azimuth = azimuth_degrees % 360.0;
        if azimuth < 0.0 {
            azimuth += 360.0;
        }
        if azimuth >= 360.0 {
            azimuth = 0.0;
        }
        let last = self.elevations.len() - 1;
        let elevation = elevation_degrees.clamp(self.elevations[0], self.elevations[last]);
        let below = self.ring_below(elevation);
        let above = if below == last { below } else { below + 1 };
        let up = if above == below { 0.0 } else { (elevation - self.elevations[below]) / (self.elevations[above] - self.elevations[below]) };
        let (d0, d1, a0) = self.around_on_ring(below, azimuth);
        let (d2, d3, a1) = self.around_on_ring(above, azimuth);
        Neighbours { direction: [d0, d1, d2, d3], weight: [(1.0 - up) * (1.0 - a0), (1.0 - up) * a0, up * (1.0 - a1), up * a1] }
    }

    /// Blends an ear's (0 left, 1 right) responses into `response` (at least `taps` long) and
    /// returns its delay in samples.
    pub fn blend(&self, neighbours: &Neighbours, ear: usize, response: &mut [f32]) -> f32 {
        let taps = self.taps;
        let len = taps.min(response.len());
        let response = &mut response[..len];
        response.fill(0.0);
        let mut delay = 0.0;
        for i in 0..4 {
            let weight = neighbours.weight[i];
            let at = 2 * neighbours.direction[i] as usize + (ear & 1);
            if let Some(source) = self.responses.get(at * taps..(at + 1) * taps) {
                for (r, s) in response.iter_mut().zip(source) {
                    *r += weight * s;
                }
            }
            delay += weight * self.delays.get(at).copied().unwrap_or(0.0);
        }
        delay
    }
}
