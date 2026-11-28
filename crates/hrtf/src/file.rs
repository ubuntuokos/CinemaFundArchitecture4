// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Sirac Ozmen
//
// Ported to Rust for CFA4 from Maul Audio (https://github.com/Rawframe-Project/maul-audio, commit
// aeb383458250759f92a3e724a5bd006505e45b4d, src/hrtf_file.{h,c}, src/file_fields.{h,c},
// src/crc32.c). CFA4 changes: Rust types and errors; UTF-8 checked with `std::str::from_utf8`.

//! The `.maudhrtf` file (Maul Audio `docs/internals/file-formats.md`, version 3), read in place:
//! every count, size, ring, text and the checksum checked. Numbers are little-endian, assembled
//! byte by byte.
//!
//! Layout: the magic `MAUDHRTF`, the version (u32), a CRC-32 of every byte from offset 16, then
//! tagged fields (tag u32, length u32, payload padded with zeros to 8 bytes): `head` (rate, taps,
//! rings, directions as u32, scale and distance as f32), `name`, `lics` (UTF-8), `ring` (per ring
//! an f32 elevation and a u32 azimuth count), `dlay` (per direction two u16 delays in 1/256 sample)
//! and `taps` (per direction the left ear's then the right ear's int16 taps).

use crate::{Error, Result};

const HEADER_BYTES: usize = 16;
const VERSION: u32 = 3;
const HEAD_BYTES: usize = 24;
const MIN_RATE: u32 = 8_000;
const MAX_RATE: u32 = 384_000;
const MIN_TAPS: u32 = 8;
const MAX_TAPS: u32 = 1_024;
const MAX_RINGS: u32 = 181;
const MAX_DIRS: u32 = 65_536;
const MAX_NAME: usize = 256;
const MAX_LICENSE: usize = 65_536;
const MIN_DISTANCE: f32 = 0.05;
const MAX_DISTANCE: f32 = 100.0;
const FIELD_ALIGN: usize = 8;
const FIELD_FRAME: usize = 8;

const fn tag(t: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*t)
}

/// The fields, in the order a writer puts them.
const TAGS: [u32; 6] = [tag(b"head"), tag(b"name"), tag(b"lics"), tag(b"ring"), tag(b"dlay"), tag(b"taps")];
const HEAD: usize = 0;
const NAME: usize = 1;
const LICENSE: usize = 2;
const RINGS: usize = 3;
const DELAYS: usize = 4;
const SAMPLES: usize = 5;

fn u16_at(b: &[u8], at: usize) -> u32 {
    match b.get(at..at + 2) {
        Some(s) => u32::from(s[0]) | u32::from(s[1]) << 8,
        None => 0,
    }
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    match b.get(at..at + 4) {
        Some(s) => u32::from_le_bytes([s[0], s[1], s[2], s[3]]),
        None => 0,
    }
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_bits(u32_at(b, at))
}

/// The CRC-32 (IEEE, reflected), a bit at a time: files are read once, at load.
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

fn field_size(length: u64) -> u64 {
    FIELD_FRAME as u64 + length.div_ceil(FIELD_ALIGN as u64) * FIELD_ALIGN as u64
}

/// A well-formed file's fields, borrowed from its bytes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HrtfFile<'a> {
    pub sample_rate: u32,
    pub taps: u32,
    pub ring_count: u32,
    pub direction_count: u32,
    pub scale: f32,
    /// Metres from the head's centre at which the set was measured.
    pub distance: f32,
    pub name: &'a str,
    pub license: &'a str,
    rings: &'a [u8],
    delays: &'a [u8],
    samples: &'a [u8],
}

impl<'a> HrtfFile<'a> {
    /// Checks `data` as a `.maudhrtf` file and describes it.
    pub fn read(data: &'a [u8]) -> Result<Self> {
        if data.len() < HEADER_BYTES || &data[..8] != b"MAUDHRTF" {
            return Err(Error::Format("no MAUDHRTF header"));
        }
        if u32_at(data, 8) != VERSION {
            return Err(Error::Version);
        }
        if crc32(&data[HEADER_BYTES..]) != u32_at(data, 12) {
            return Err(Error::Format("checksum mismatch"));
        }
        let mut fields: [Option<&'a [u8]>; 6] = [None; 6];
        let mut at = HEADER_BYTES;
        while at < data.len() {
            if data.len() - at < FIELD_FRAME {
                return Err(Error::Format("field frame cut short"));
            }
            let t = u32_at(data, at);
            let length = u32_at(data, at + 4) as u64;
            let Some(known) = TAGS.iter().position(|&x| x == t) else {
                return Err(Error::Version);
            };
            let size = field_size(length);
            if fields[known].is_some() || size > (data.len() - at) as u64 {
                return Err(Error::Format("field repeated or past the end"));
            }
            // size fits in the remaining bytes, so these fit in usize
            let start = at + FIELD_FRAME;
            let end = start + length as usize;
            let padded_end = at + size as usize;
            if data[end..padded_end].iter().any(|&b| b != 0) {
                return Err(Error::Format("field padding not zero"));
            }
            fields[known] = Some(&data[start..end]);
            at = padded_end;
        }
        let (Some(head), Some(rings), Some(delays), Some(samples)) = (fields[HEAD], fields[RINGS], fields[DELAYS], fields[SAMPLES]) else {
            return Err(Error::Format("a required field is missing"));
        };
        if head.len() != HEAD_BYTES {
            return Err(Error::Format("head field of the wrong size"));
        }
        let name = fields[NAME].unwrap_or(&[]);
        let license = fields[LICENSE].unwrap_or(&[]);
        let sample_rate = u32_at(head, 0);
        let taps = u32_at(head, 4);
        let ring_count = u32_at(head, 8);
        let direction_count = u32_at(head, 12);
        let scale = f32_at(head, 16);
        let distance = f32_at(head, 20);
        let counts_valid = (MIN_RATE..=MAX_RATE).contains(&sample_rate)
            && (MIN_TAPS..=MAX_TAPS).contains(&taps)
            && (1..=MAX_RINGS).contains(&ring_count)
            && (1..=MAX_DIRS).contains(&direction_count)
            && scale.is_finite()
            && scale > 0.0
            && (MIN_DISTANCE..=MAX_DISTANCE).contains(&distance)
            && name.len() <= MAX_NAME
            && license.len() <= MAX_LICENSE;
        if !counts_valid {
            return Err(Error::Format("a count is out of range"));
        }
        let dirs = u64::from(direction_count);
        if rings.len() as u64 != 8 * u64::from(ring_count) || delays.len() as u64 != 4 * dirs || samples.len() as u64 != 4 * dirs * u64::from(taps) {
            return Err(Error::Format("a field's length does not match its counts"));
        }
        let name = std::str::from_utf8(name).map_err(|_| Error::Format("name is not UTF-8"))?;
        let license = std::str::from_utf8(license).map_err(|_| Error::Format("licence is not UTF-8"))?;
        let file = HrtfFile { sample_rate, taps, ring_count, direction_count, scale, distance, name, license, rings, delays, samples };
        if !file.rings_valid() {
            return Err(Error::Format("rings out of order or not adding up"));
        }
        Ok(file)
    }

    /// Elevations rising strictly within [-90, 90], every ring with an azimuth, the counts adding
    /// up to the header's.
    fn rings_valid(&self) -> bool {
        let mut directions = 0u64;
        let mut previous = f32::NEG_INFINITY;
        for ring in 0..self.ring_count {
            let elevation = self.elevation(ring);
            let azimuths = self.azimuths(ring);
            if elevation.partial_cmp(&previous) != Some(std::cmp::Ordering::Greater)
                || !(-90.0..=90.0).contains(&elevation)
                || azimuths == 0
                || azimuths > MAX_DIRS
            {
                return false;
            }
            previous = elevation;
            directions += u64::from(azimuths);
        }
        directions == u64::from(self.direction_count)
    }

    /// A ring's elevation in degrees.
    pub fn elevation(&self, ring: u32) -> f32 {
        f32_at(self.rings, 8 * ring as usize)
    }

    /// A ring's azimuth count.
    pub fn azimuths(&self, ring: u32) -> u32 {
        u32_at(self.rings, 8 * ring as usize + 4)
    }

    /// A direction's delay for ear 0 (left) or 1 (right), in samples.
    pub fn delay(&self, direction: u32, ear: usize) -> f32 {
        u16_at(self.delays, 4 * direction as usize + 2 * ear) as f32 / 256.0
    }

    /// A direction's response for an ear, as floats, `taps` of them.
    pub fn response(&self, direction: u32, ear: usize, out: &mut [f32]) {
        let taps = self.taps as usize;
        let at = 2 * taps * (2 * direction as usize + ear);
        for (tap, o) in out.iter_mut().take(taps).enumerate() {
            let bits = u16_at(self.samples, at + 2 * tap) as u16;
            *o = f32::from(bits as i16) * self.scale;
        }
    }
}
