// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Sirac Ozmen
//
// Ported to Rust for CFA4 from Maul Audio (https://github.com/Rawframe-Project/maul-audio, commit
// aeb383458250759f92a3e724a5bd006505e45b4d, tools/sofa_to_maudhrtf.py). CFA4 changes: the SOFA file
// is read with the `sofar` crate instead of h5py; the FFTs are `cfa4_audio_dsp::fft` (f32) instead
// of numpy's; a set measured at several distances keeps the distance most directions were
// measured at instead of asking; a file's `Data.Delay` is added to the onsets; a missing licence
// is an empty text (the user's own file, never shipped).

//! A SOFA file (AES69, `SimpleFreeFieldHRIR`) as an HRTF set: the user's own set for the headphone
//! monitor (`docs/plans/CFA4-BINAURAL-MONITOR-001.md` §3.1, B-04).
//!
//! Each measured response's onset is found where its low-passed version (below 1.2 to 2 kHz, where
//! the interaural delay is a delay rather than pinna detail) first reaches a tenth of its peak; the
//! response is made minimum-phase and cut to 128 taps at 48 kHz (as many at other rates for the
//! same length); the measured directions are then resampled onto elevation rings 5° apart, each
//! with round(72 cos(elevation)) evenly spaced azimuths, by inverse-angle weighting of the three
//! nearest measured directions (the nearest alone within 0.1°). The result is a `.maudhrtf` file
//! ([`sofa_to_maudhrtf`]), so a SOFA set is loaded exactly as the built-in one.

use cfa4_audio_dsp::fft::Fft;

use crate::file::crc32;
use crate::{Error, HrtfSet, Result};

const MAGIC: &[u8; 8] = b"MAUDHRTF";
const VERSION: u32 = 3;
const ALIGN: usize = 8;
const RING_STEP: f32 = 5.0;
const AZIMUTHS_AT_EQUATOR: f64 = 72.0;
const TAPS_AT_48K: f64 = 128.0;
const FADE_TAPS: usize = 16;
const FFT_SIZE: usize = 8192;
const ONSET_FRACTION: f32 = 0.1;
const ONSET_PASS_HZ: f32 = 1200.0;
const ONSET_STOP_HZ: f32 = 2000.0;
const NEAREST_ONLY_DEGREES: f64 = 0.1;
const NEIGHBOURS: usize = 3;
const DELAY_UNITS: f32 = 256.0;
const DISTANCE_TOLERANCE: f32 = 0.01;
const MIN_DISTANCE: f32 = 0.05;
const MAX_DISTANCE: f32 = 100.0;
/// The most measurements read (the largest public sets have about 12 000), so a hostile file
/// cannot ask for hours of work.
const MAX_MEASUREMENTS: usize = 100_000;
/// The longest measured response read (taps).
const MAX_IR_TAPS: usize = 65_536;

fn bad(why: impl Into<String>) -> Error {
    Error::Sofa(why.into())
}

/// A dataset's values as f64 (float of 32 or 64 bits, or a 32 / 64-bit integer, little-endian),
/// with its dimensions and text attributes.
struct Dataset {
    values: Vec<f64>,
    dims: Vec<usize>,
    attributes: Vec<(String, String)>,
}

impl Dataset {
    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

fn dataset(parsed: &sofar::hdf::ParsedHdf<'_>, name: &'static str) -> Result<Option<Dataset>> {
    let Some(object) = parsed.get_child(name) else { return Ok(None) };
    let object = object.map_err(|_| bad(format!("{name} cannot be read")))?;
    let float = object.dt.class_and_version & 0x0F == 1;
    let integer = object.dt.class_and_version & 0x0F == 0;
    let size = object.dt.size as usize;
    let data = &object.data;
    let values: Vec<f64> = match (float, integer, size) {
        (true, _, 8) => data.chunks_exact(8).map(|b| f64::from_le_bytes(b.try_into().unwrap_or([0; 8]))).collect(),
        (true, _, 4) => data.chunks_exact(4).map(|b| f64::from(f32::from_le_bytes(b.try_into().unwrap_or([0; 4])))).collect(),
        (_, true, 8) => data.chunks_exact(8).map(|b| i64::from_le_bytes(b.try_into().unwrap_or([0; 8])) as f64).collect(),
        (_, true, 4) => data.chunks_exact(4).map(|b| f64::from(i32::from_le_bytes(b.try_into().unwrap_or([0; 4])))).collect(),
        _ => return Err(bad(format!("{name} is not stored as numbers"))),
    };
    if values.iter().any(|v| !v.is_finite()) {
        return Err(bad(format!("{name} holds a value that is not a number")));
    }
    let dims = object.ds.dimension_size.iter().map(|&d| usize::try_from(d).unwrap_or(usize::MAX)).collect();
    let attributes = object.parsed_attributes.iter().filter_map(|a| a.value.clone().map(|v| (a.name.clone(), v))).collect();
    Ok(Some(Dataset { values, dims, attributes }))
}

/// The measurements of a SOFA file: per measurement the two ears' responses, the direction
/// (azimuth counter-clockwise from ahead and elevation, degrees) and the distance.
struct Measured {
    rate: f32,
    taps: usize,
    ir: Vec<f32>,
    delay: Vec<f32>,
    azimuth: Vec<f64>,
    elevation: Vec<f64>,
    radius: Vec<f32>,
    name: String,
    license: String,
}

/// Reads the HDF5 datasets as stored (`sofar`'s HDF5 layer; its reader would convert the positions
/// to f32 and resample the responses).
fn read(bytes: &[u8]) -> Result<Measured> {
    let parsed = sofar::hdf::parse_with_children(bytes).map_err(|_| bad("not an HDF5 / SOFA file"))?;
    let root = |name: &str| parsed.root.parsed_attributes.iter().find(|a| a.name == name).and_then(|a| a.value.clone()).unwrap_or_default();
    if root("Conventions") != "SOFA" {
        return Err(bad("not a SOFA file"));
    }
    let convention = root("SOFAConventions");
    if convention != "SimpleFreeFieldHRIR" {
        return Err(bad(format!("a {} file, not SimpleFreeFieldHRIR", if convention.is_empty() { "SOFA" } else { &convention })));
    }
    let rate = dataset(&parsed, "Data.SamplingRate")?.and_then(|d| d.values.first().copied()).ok_or_else(|| bad("no Data.SamplingRate"))?;
    if !(8_000.0..=384_000.0).contains(&rate) {
        return Err(bad("the sampling rate is not 8 to 384 kHz"));
    }
    let ir = dataset(&parsed, "Data.IR")?.ok_or_else(|| bad("no Data.IR"))?;
    let (m, r, n) = match ir.dims[..] {
        [m, r, n] => (m, r, n),
        _ => return Err(bad("Data.IR is not M × R × N")),
    };
    if r != 2 || m == 0 || n < 8 || m > MAX_MEASUREMENTS || n > MAX_IR_TAPS || ir.values.len() != m * 2 * n {
        return Err(bad("Data.IR is not M × 2 × N with sensible sizes"));
    }
    let pos = dataset(&parsed, "SourcePosition")?.ok_or_else(|| bad("no SourcePosition"))?;
    let units = pos.attribute("Units").unwrap_or("").to_ascii_lowercase();
    if !(units.contains("metre") || units.contains("meter")) {
        return Err(bad("SourcePosition is not in metres"));
    }
    let cartesian = pos.attribute("Type").unwrap_or("spherical").eq_ignore_ascii_case("cartesian");
    let per = match pos.values.len() {
        3 => 0,
        l if l == 3 * m => 3,
        _ => return Err(bad("SourcePosition does not give one position per measurement")),
    };
    let (mut azimuth, mut elevation, mut radius) = (Vec::with_capacity(m), Vec::with_capacity(m), Vec::with_capacity(m));
    for i in 0..m {
        let (p, q, t) = (pos.values[i * per], pos.values[i * per + 1], pos.values[i * per + 2]);
        // x ahead, y left, z up; spherical is azimuth, elevation (degrees), radius
        let (a, e, d) = if cartesian { (q.atan2(p).to_degrees(), t.atan2(p.hypot(q)).to_degrees(), (p * p + q * q + t * t).sqrt()) } else { (p, q, t) };
        if !(d > 0.0 && d.is_finite()) {
            return Err(bad("a source position is at the listener"));
        }
        azimuth.push(a);
        elevation.push(e);
        radius.push(d as f32);
    }
    // Data.Delay: one per ear for every measurement, or one per ear for all, in samples
    let dv = dataset(&parsed, "Data.Delay")?.map(|d| d.values).unwrap_or_default();
    let delay: Vec<f32> = (0..m * 2)
        .map(|k| match dv.len() {
            2 => dv[k % 2],
            l if l == 2 * m => dv[k],
            _ => 0.0,
        })
        .map(|d| d.max(0.0) as f32)
        .collect();
    let name = [root("DatabaseName"), root("ListenerShortName")].iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
    Ok(Measured {
        rate: rate as f32,
        taps: n,
        ir: ir.values.iter().map(|&v| v as f32).collect(),
        delay,
        azimuth,
        elevation,
        radius,
        name,
        license: root("License"),
    })
}

/// The measurement distance most directions share, and which were measured at it.
fn choose_distance(radius: &[f32]) -> Result<(f32, Vec<bool>)> {
    let mut best: Option<(usize, f32)> = None;
    for &reference in radius {
        let count = radius.iter().filter(|&&d| (d - reference).abs() <= DISTANCE_TOLERANCE * reference).count();
        if best.is_none_or(|(c, d)| count > c || (count == c && reference > d)) {
            best = Some((count, reference));
        }
    }
    let (_, reference) = best.ok_or_else(|| bad("no measurements"))?;
    let kept: Vec<bool> = radius.iter().map(|&d| (d - reference).abs() <= DISTANCE_TOLERANCE * reference).collect();
    let mut at: Vec<f32> = radius.iter().zip(&kept).filter(|(_, k)| **k).map(|(d, _)| *d).collect();
    at.sort_by(f32::total_cmp);
    let distance = at[at.len() / 2];
    if !(MIN_DISTANCE..=MAX_DISTANCE).contains(&distance) {
        return Err(bad("the measurement distance is outside 0.05 to 100 m"));
    }
    Ok((distance, kept))
}

/// Planned transforms and their buffers for one response at a time.
struct Spectra {
    fft: Fft,
    re: Vec<f32>,
    im: Vec<f32>,
}

impl Spectra {
    fn new(taps: usize) -> Self {
        let n = FFT_SIZE.max((2 * taps).next_power_of_two());
        Spectra { fft: Fft::new(n), re: vec![0.0; n], im: vec![0.0; n] }
    }

    fn load(&mut self, response: &[f32]) {
        self.re.fill(0.0);
        self.im.fill(0.0);
        let n = self.re.len();
        self.re[..response.len().min(n)].copy_from_slice(&response[..response.len().min(n)]);
    }

    /// The fractional sample where the low-passed response (flat to 1.2 kHz, a half-cosine down
    /// to nothing at 2 kHz, zero phase) first reaches a tenth of its peak, interpolated linearly.
    fn onset(&mut self, response: &[f32], rate: f32) -> f32 {
        self.load(response);
        let n = self.re.len();
        self.fft.forward(&mut self.re, &mut self.im);
        for k in 0..n {
            let bin = k.min(n - k);
            let hz = bin as f32 * rate / n as f32;
            let ramp = ((hz - ONSET_PASS_HZ) / (ONSET_STOP_HZ - ONSET_PASS_HZ)).clamp(0.0, 1.0);
            let g = 0.5 * (1.0 + (std::f32::consts::PI * ramp).cos());
            self.re[k] *= g;
            self.im[k] *= g;
        }
        self.fft.inverse(&mut self.re, &mut self.im);
        let len = response.len().min(n);
        let magnitude: Vec<f32> = self.re[..len].iter().map(|v| (v / n as f32).abs()).collect();
        let peak = magnitude.iter().copied().fold(0.0f32, f32::max);
        if peak <= 0.0 {
            return 0.0;
        }
        let threshold = ONSET_FRACTION * peak;
        let index = magnitude.iter().position(|&v| v >= threshold).unwrap_or(0);
        if index == 0 {
            return 0.0;
        }
        let (before, after) = (magnitude[index - 1], magnitude[index]);
        index as f32 - 1.0 + (threshold - before) / (after - before)
    }

    /// The minimum-phase response with the same magnitude (real cepstrum), cut to `out.len()` taps
    /// with a half-cosine fade over its last 16.
    fn minimum_phase(&mut self, response: &[f32], out: &mut [f32]) {
        self.load(response);
        let n = self.re.len();
        self.fft.forward(&mut self.re, &mut self.im);
        for k in 0..n {
            let mag = self.re[k].hypot(self.im[k]).max(1e-12);
            self.re[k] = mag.ln();
            self.im[k] = 0.0;
        }
        self.fft.inverse(&mut self.re, &mut self.im);
        // fold the cepstrum onto its causal half
        for k in 0..n {
            let w = if k == 0 || k == n / 2 {
                1.0
            } else if k < n / 2 {
                2.0
            } else {
                0.0
            };
            self.re[k] *= w / n as f32;
            self.im[k] = 0.0;
        }
        self.fft.forward(&mut self.re, &mut self.im);
        for k in 0..n {
            let (a, b) = (self.re[k], self.im[k]);
            let e = a.exp();
            self.re[k] = e * b.cos();
            self.im[k] = e * b.sin();
        }
        self.fft.inverse(&mut self.re, &mut self.im);
        let taps = out.len();
        for (i, o) in out.iter_mut().enumerate() {
            *o = self.re[i] / n as f32;
        }
        for i in 1..=FADE_TAPS.min(taps) {
            out[taps - FADE_TAPS.min(taps) + i - 1] *= 0.5 * (1.0 + (std::f32::consts::PI * i as f32 / FADE_TAPS as f32).cos());
        }
    }
}

fn unit(azimuth: f64, elevation: f64) -> [f64; 3] {
    let (a, e) = (azimuth.to_radians(), elevation.to_radians());
    [e.cos() * a.cos(), e.cos() * a.sin(), e.sin()]
}

/// The rings: elevations -90° … 90° every 5°, and each one's azimuth count.
fn rings() -> (Vec<f32>, Vec<u32>) {
    let elevations: Vec<f32> = (0..=36).map(|i| -90.0 + RING_STEP * i as f32).collect();
    let counts = elevations.iter().map(|&e| ((AZIMUTHS_AT_EQUATOR * f64::from(e).to_radians().cos()).round() as u32).max(1)).collect();
    (elevations, counts)
}

/// Converts a SOFA file (`SimpleFreeFieldHRIR`) into a `.maudhrtf` file (see the module notes).
/// A file that is not one, or not usable, is an error, never a panic.
pub fn sofa_to_maudhrtf(bytes: &[u8]) -> Result<Vec<u8>> {
    // the HDF5 layer is third-party code fed a user's file: a panic in it is a refused file
    let all = std::panic::catch_unwind(|| read(bytes)).map_err(|_| bad("the HDF5 data is malformed"))??;
    let (distance, kept) = choose_distance(&all.radius)?;
    let rate = all.rate;
    let taps = ((TAPS_AT_48K * f64::from(rate) / 48_000.0).round() as usize).max(8);
    if taps > 1_024 {
        return Err(bad("the sampling rate needs more than 1024 taps"));
    }
    let n = all.taps;
    let mut spectra = Spectra::new(n);
    let mut directions = Vec::new();
    let mut onsets = Vec::new();
    let mut responses = Vec::new();
    let mut cut = vec![0.0f32; taps];
    for (i, _) in kept.iter().enumerate().filter(|(_, k)| **k) {
        directions.push(unit(all.azimuth[i], all.elevation[i]));
        for ear in 0..2 {
            let ir = &all.ir[(i * 2 + ear) * n..(i * 2 + ear + 1) * n];
            onsets.push(spectra.onset(ir, rate) + all.delay[i * 2 + ear]);
            spectra.minimum_phase(ir, &mut cut);
            responses.extend_from_slice(&cut);
        }
    }
    let (ring_elevations, ring_counts) = rings();
    let wanted: Vec<[f64; 3]> =
        ring_elevations.iter().zip(&ring_counts).flat_map(|(&e, &c)| (0..c).map(move |k| unit(f64::from(k) * 360.0 / f64::from(c), f64::from(e)))).collect();
    let mut delays = vec![0.0f32; wanted.len() * 2];
    let mut out = vec![0.0f32; wanted.len() * 2 * taps];
    let mut order: Vec<(f64, usize)> = Vec::with_capacity(directions.len());
    for (w, dir) in wanted.iter().enumerate() {
        order.clear();
        order.extend(directions.iter().enumerate().map(|(j, d)| ((d[0] * dir[0] + d[1] * dir[1] + d[2] * dir[2]).clamp(-1.0, 1.0), j)));
        let k = NEIGHBOURS.min(order.len());
        order.select_nth_unstable_by(k - 1, |a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        order[..k].sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let angles: Vec<f64> = order[..k].iter().map(|(c, _)| c.acos().to_degrees()).collect();
        let weights: Vec<f64> = if angles[0] <= NEAREST_ONLY_DEGREES {
            vec![1.0]
        } else {
            let inv: Vec<f64> = angles.iter().map(|a| 1.0 / a).collect();
            let sum: f64 = inv.iter().sum();
            inv.iter().map(|v| v / sum).collect()
        };
        for ear in 0..2 {
            let mut d = 0.0f64;
            let dst = &mut out[(w * 2 + ear) * taps..(w * 2 + ear + 1) * taps];
            for (&(_, j), &g) in order.iter().zip(&weights) {
                d += g * f64::from(onsets[j * 2 + ear]);
                let src = &responses[(j * 2 + ear) * taps..(j * 2 + ear + 1) * taps];
                for (o, s) in dst.iter_mut().zip(src) {
                    *o += (g * f64::from(*s)) as f32;
                }
            }
            delays[w * 2 + ear] = d as f32;
        }
    }
    let least = delays.iter().copied().fold(f32::INFINITY, f32::min);
    delays.iter_mut().for_each(|d| *d -= least);
    Ok(encode(rate, distance, taps, &ring_elevations, &ring_counts, &delays, &out, &all.name, &all.license))
}

fn field(body: &mut Vec<u8>, tag: &[u8; 4], payload: &[u8]) {
    body.extend_from_slice(tag);
    body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    body.extend_from_slice(payload);
    body.resize(body.len() + (ALIGN - payload.len() % ALIGN) % ALIGN, 0);
}

#[allow(clippy::too_many_arguments)]
fn encode(rate: f32, distance: f32, taps: usize, elevations: &[f32], counts: &[u32], delays: &[f32], responses: &[f32], name: &str, license: &str) -> Vec<u8> {
    let peak = responses.iter().fold(0.0f32, |a, v| a.max(v.abs()));
    let scale = if peak > 0.0 { peak / 32_767.0 } else { 1.0 };
    let directions: u32 = counts.iter().sum();
    let mut head = Vec::with_capacity(24);
    for v in [rate.round() as u32, taps as u32, counts.len() as u32, directions] {
        head.extend_from_slice(&v.to_le_bytes());
    }
    head.extend_from_slice(&scale.to_le_bytes());
    head.extend_from_slice(&distance.to_le_bytes());
    let mut name_bytes = name.as_bytes();
    let mut cut = name_bytes.len().min(256);
    while !name.is_char_boundary(cut) {
        cut -= 1;
    }
    name_bytes = &name_bytes[..cut];
    let mut license_bytes = license.as_bytes();
    let mut cut = license_bytes.len().min(65_536);
    while !license.is_char_boundary(cut) {
        cut -= 1;
    }
    license_bytes = &license_bytes[..cut];
    let rings: Vec<u8> = elevations.iter().zip(counts).flat_map(|(e, c)| e.to_le_bytes().into_iter().chain(c.to_le_bytes())).collect();
    let units: Vec<u8> = delays.iter().flat_map(|d| ((d * DELAY_UNITS).round().clamp(0.0, 65_535.0) as u16).to_le_bytes()).collect();
    let samples: Vec<u8> = responses.iter().flat_map(|s| ((s / scale).round().clamp(-32_768.0, 32_767.0) as i16).to_le_bytes()).collect();
    let mut body = Vec::new();
    field(&mut body, b"head", &head);
    if !name_bytes.is_empty() {
        field(&mut body, b"name", name_bytes);
    }
    if !license_bytes.is_empty() {
        field(&mut body, b"lics", license_bytes);
    }
    field(&mut body, b"ring", &rings);
    field(&mut body, b"dlay", &units);
    field(&mut body, b"taps", &samples);
    let mut out = Vec::with_capacity(16 + body.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&crc32(&body).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

impl HrtfSet {
    /// Loads a SOFA file (`SimpleFreeFieldHRIR`), at its own rate or resampled to `sample_rate`
    /// (see [`sofa_to_maudhrtf`]).
    pub fn from_sofa(bytes: &[u8], sample_rate: Option<u32>) -> Result<Self> {
        HrtfSet::from_maudhrtf(&sofa_to_maudhrtf(bytes)?, sample_rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garbage_is_an_error_not_a_crash() {
        for bytes in [&b""[..], b"\x89HDF\r\n\x1a\n", &[0u8; 4096][..], b"MAUDHRTF"] {
            assert!(HrtfSet::from_sofa(bytes, None).is_err());
        }
    }

    /// Every one-byte change and every cut of a real SOFA file (`CFA4_SOFA`, the first 64 KiB
    /// mutated) is a set or an error, never a crash.
    #[test]
    fn a_damaged_sofa_file_is_an_error_not_a_crash() {
        let Some(path) = std::env::var_os("CFA4_SOFA") else {
            eprintln!("skipped: set CFA4_SOFA to a SimpleFreeFieldHRIR file");
            return;
        };
        let bytes = std::fs::read(path).expect("read");
        let mut refused = 0;
        for at in (0..bytes.len().min(65_536)).step_by(499) {
            let mut b = bytes.clone();
            b[at] ^= 0xA5;
            refused += usize::from(sofa_to_maudhrtf(&b).is_err());
            refused += usize::from(sofa_to_maudhrtf(&bytes[..at]).is_err());
        }
        println!("{refused} damaged files refused");
    }

    #[test]
    fn a_minimum_phase_impulse_stays_an_impulse_and_an_onset_is_found() {
        let mut s = Spectra::new(256);
        let mut ir = vec![0.0f32; 256];
        ir[40] = 1.0;
        let mut out = vec![0.0f32; 128];
        s.minimum_phase(&ir, &mut out);
        assert!((out[0] - 1.0).abs() < 1e-3, "{}", out[0]);
        assert!(out[1..].iter().all(|v| v.abs() < 1e-3));
        // the low-pass spreads the onset early by the same amount for any delay: the difference
        // between two onsets is the delay between them (the interaural delay)
        let at = s.onset(&ir, 48_000.0);
        let mut later = vec![0.0f32; 256];
        later[60] = 1.0;
        let gap = s.onset(&later, 48_000.0) - at;
        assert!(at > 0.0 && at < 40.0 && (gap - 20.0).abs() < 0.01, "onset {at}, gap {gap}");
    }

    #[test]
    fn the_rings_are_the_builtin_sets() {
        let (e, c) = rings();
        assert_eq!(e.len(), 37);
        assert_eq!((c[0], c[18], c[36]), (1, 72, 1));
        let set = HrtfSet::builtin(48_000).expect("set");
        assert_eq!(set.direction_count(), c.iter().sum::<u32>() as usize);
    }

    /// A real SOFA file, when `CFA4_SOFA` names one: it loads, every ring direction has a response
    /// and the left ear leads for a source on the left.
    #[test]
    fn a_real_sofa_file_loads() {
        let Some(path) = std::env::var_os("CFA4_SOFA") else {
            eprintln!("skipped: set CFA4_SOFA to a SimpleFreeFieldHRIR file");
            return;
        };
        let bytes = std::fs::read(path).expect("read");
        let started = std::time::Instant::now();
        let set = HrtfSet::from_sofa(&bytes, Some(48_000)).expect("SOFA set");
        println!("{} directions in {:.2} s: {}", set.direction_count(), started.elapsed().as_secs_f64(), set.name());
        let n = set.neighbours(90.0, 0.0);
        let mut r = vec![0.0; set.taps()];
        let left = set.blend(&n, 0, &mut r);
        let right = set.blend(&n, 1, &mut r);
        assert!(left < right, "left ear delay {left} ≥ right {right} for a source on the left");
    }

    /// Against Maul Audio's own converter (`tools/sofa_to_maudhrtf.py`, numpy f64), when
    /// `CFA4_SOFA` names a SOFA file and `CFA4_SOFA_MAUDHRTF` that tool's output for it: the same
    /// rings, rate, taps and distance, and for at least 97 % of the directions delays within 0.1
    /// sample and responses within 1 % of the set's peak (our FFTs are f32). The rest are ring
    /// directions where the third-nearest measured direction is a tie (as at -5° between KEMAR's
    /// 0° and -10° rings): which of the tied ones each tool takes depends on the last bit of a
    /// cosine, and both choices are as right.
    #[test]
    fn the_conversion_matches_maul_audios_tool() {
        let (Some(sofa), Some(maul)) = (std::env::var_os("CFA4_SOFA"), std::env::var_os("CFA4_SOFA_MAUDHRTF")) else {
            eprintln!("skipped: set CFA4_SOFA and CFA4_SOFA_MAUDHRTF (sofa_to_maudhrtf.py's output for it)");
            return;
        };
        let ours = sofa_to_maudhrtf(&std::fs::read(sofa).expect("read")).expect("convert");
        let theirs = std::fs::read(maul).expect("read");
        let (a, b) = (crate::file::HrtfFile::read(&ours).expect("ours"), crate::file::HrtfFile::read(&theirs).expect("theirs"));
        assert_eq!((a.sample_rate, a.taps, a.ring_count, a.direction_count), (b.sample_rate, b.taps, b.ring_count, b.direction_count));
        assert!((a.distance - b.distance).abs() < 1e-4, "{} vs {}", a.distance, b.distance);
        assert_eq!((a.name, a.license), (b.name, b.license));
        let peak = b.scale * 32_767.0;
        let taps = a.taps as usize;
        let (mut ra, mut rb) = (vec![0.0f32; taps], vec![0.0f32; taps]);
        let mut differ = 0;
        for d in 0..a.direction_count {
            let mut same = true;
            for ear in 0..2 {
                a.response(d, ear, &mut ra);
                b.response(d, ear, &mut rb);
                let tap = ra.iter().zip(&rb).fold(0.0f32, |w, (x, y)| w.max((x - y).abs() / peak));
                same &= (a.delay(d, ear) - b.delay(d, ear)).abs() <= 0.1 && tap <= 0.01;
            }
            differ += usize::from(!same);
        }
        println!("{differ} of {} directions differ from Maul Audio's tool", a.direction_count);
        assert!(differ * 100 <= 3 * a.direction_count as usize, "{differ} directions differ");
    }
}
