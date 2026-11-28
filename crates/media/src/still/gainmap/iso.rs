// SPDX-License-Identifier: GPL-3.0-or-later AND (MIT OR Apache-2.0)
//
// The ISO 21496-1 gain map metadata block, re-expressed in Rust from libultrahdr
// (https://github.com/google/libultrahdr, commit c6fd15cdb2d05d3aaed84b817e4c4a274a4ce600,
// lib/src/gainmapmetadata.cpp `decodeGainmapMetadata` and `gainmapMetadataFractionToFloat`,
// lib/include/ultrahdr/gainmapmetadata.h).
// Copyright (c) 2022 The Android Open Source Project. libultrahdr is distributed under both the MIT
// licence and the Apache License 2.0 (its LICENSE file); this port is used under either. CFA4's
// changes are GPL-3.0-or-later. Recorded in docs/licensing.md section 5 (owner decision 2026-11-28, F2g E3).

//! The ISO 21496-1 gain map metadata block (the payload after the `urn:iso:std:iso:ts:21496:-1`
//! identifier): a big-endian minimum version and writer version (u16 each), then, in the gain map
//! image, a flags byte (bit 7: three channels, bit 6: apply in the base colour space, bit 3: one
//! common denominator, bit 2: the base is the HDR rendition) and the rationals: the base and alternate
//! HDR headroom (log2), then per channel the gain map minimum and maximum (log2), gamma, base offset
//! and alternate offset. A block that holds only the two versions (the one in the primary image)
//! announces that the file has a gain map.

use super::{Channel, GainMapParams, Origin};

/// What an ISO block holds.
#[derive(Debug, PartialEq)]
pub enum Block {
    /// Only the version fields (the primary image's announcement).
    VersionOnly,
    Params(GainMapParams),
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let v = self.b.get(self.pos..self.pos + N).and_then(|s| <[u8; N]>::try_from(s).ok());
        let v = v.ok_or_else(|| format!("truncated at byte {}", self.pos))?;
        self.pos += N;
        Ok(v)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.bytes::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_be_bytes(self.bytes()?))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.bytes()?))
    }
    fn s32(&mut self) -> Result<i32, String> {
        Ok(i32::from_be_bytes(self.bytes()?))
    }
}

/// A rational with a nonzero denominator as f32.
fn ratio(n: f64, d: u32, what: &str) -> Result<f32, String> {
    if d == 0 {
        return Err(format!("{what} denominator is 0"));
    }
    Ok((n / f64::from(d)) as f32)
}

/// Parse an ISO 21496-1 block.
pub fn parse(b: &[u8]) -> Result<Block, String> {
    let mut r = Reader { b, pos: 0 };
    let min_version = r.u16()?;
    if min_version != 0 {
        return Err(format!("unsupported minimum version {min_version}"));
    }
    let _writer_version = r.u16()?;
    if r.pos == b.len() {
        return Ok(Block::VersionOnly);
    }
    let flags = r.u8()?;
    let channels = if flags & 0x80 != 0 { 3 } else { 1 };
    let use_base_color_space = flags & 0x40 != 0;
    let backward = flags & 0x04 != 0;
    let common = flags & 0x08 != 0;

    let mut ch = [Channel::default(); 3];
    let (base_headroom, alternate_headroom);
    if common {
        let d = r.u32()?;
        if d == 0 {
            return Err("common denominator is 0".into());
        }
        base_headroom = ratio(f64::from(r.u32()?), d, "base headroom")?;
        alternate_headroom = ratio(f64::from(r.u32()?), d, "alternate headroom")?;
        for c in ch.iter_mut().take(channels) {
            c.min = ratio(f64::from(r.s32()?), d, "gain map min")?;
            c.max = ratio(f64::from(r.s32()?), d, "gain map max")?;
            c.gamma = ratio(f64::from(r.u32()?), d, "gamma")?;
            c.base_offset = ratio(f64::from(r.s32()?), d, "base offset")?;
            c.alternate_offset = ratio(f64::from(r.s32()?), d, "alternate offset")?;
        }
    } else {
        let (n, d) = (r.u32()?, r.u32()?);
        base_headroom = ratio(f64::from(n), d, "base headroom")?;
        let (n, d) = (r.u32()?, r.u32()?);
        alternate_headroom = ratio(f64::from(n), d, "alternate headroom")?;
        for c in ch.iter_mut().take(channels) {
            let (n, d) = (r.s32()?, r.u32()?);
            c.min = ratio(f64::from(n), d, "gain map min")?;
            let (n, d) = (r.s32()?, r.u32()?);
            c.max = ratio(f64::from(n), d, "gain map max")?;
            let (n, d) = (r.u32()?, r.u32()?);
            c.gamma = ratio(f64::from(n), d, "gamma")?;
            let (n, d) = (r.s32()?, r.u32()?);
            c.base_offset = ratio(f64::from(n), d, "base offset")?;
            let (n, d) = (r.s32()?, r.u32()?);
            c.alternate_offset = ratio(f64::from(n), d, "alternate offset")?;
        }
    }
    // a one-channel map applies the same values to all three channels
    for k in channels..3 {
        ch[k] = ch[0];
    }
    let p = GainMapParams { channels: ch, base_headroom, alternate_headroom, use_base_color_space, backward, origin: Origin::Iso };
    p.validate()?;
    Ok(Block::Params(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(common: bool) -> Vec<u8> {
        let mut v = vec![0, 0, 0, 0, 0x40 | if common { 0x08 } else { 0 }];
        let mut p = |n: i64, d: u32| {
            v.extend((n as u32).to_be_bytes());
            if !common {
                v.extend(d.to_be_bytes());
            }
        };
        for (n, d) in [(0, 64), (128, 64), (0, 64), (192, 64), (64, 64), (1, 64), (1, 64)] {
            p(n, d);
        }
        if common {
            // the common denominator follows the flags
            v.splice(5..5, 64u32.to_be_bytes());
        }
        v
    }

    #[test]
    fn one_channel_both_layouts() {
        for common in [false, true] {
            let Block::Params(p) = parse(&block(common)).unwrap() else { panic!("params expected") };
            assert_eq!((p.base_headroom, p.alternate_headroom), (0.0, 2.0));
            for c in p.channels {
                assert_eq!((c.min, c.max, c.gamma, c.base_offset, c.alternate_offset), (0.0, 3.0, 1.0, 1.0 / 64.0, 1.0 / 64.0));
            }
            assert!(p.use_base_color_space && !p.backward);
        }
    }

    #[test]
    fn version_only_and_damage() {
        assert_eq!(parse(&[0, 0, 0, 0]), Ok(Block::VersionOnly));
        assert!(parse(&[0, 1, 0, 0]).is_err());
        let b = block(false);
        for n in 0..b.len() {
            if n != 4 {
                assert!(parse(&b[..n]).is_err(), "truncated at {n}");
            }
        }
        let mut z = block(false);
        z[9..13].copy_from_slice(&[0; 4]); // base headroom denominator
        assert!(parse(&z).is_err());
    }
}
