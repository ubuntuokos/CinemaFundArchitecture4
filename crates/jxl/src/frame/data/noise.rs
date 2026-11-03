// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-frame 0.13.3.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

#[derive(Debug)]
pub struct NoiseParameters {
    pub lut: [f32; 8],
}

impl<Ctx> crate::common::Bundle<Ctx> for NoiseParameters {
    type Error = crate::frame::Error;

    fn parse(bitstream: &mut crate::bitstream::Bitstream, _: Ctx) -> crate::frame::Result<Self> {
        let mut lut = [0.0f32; 8];
        for slot in &mut lut {
            *slot = bitstream.read_bits(10)? as f32 / (1 << 10) as f32;
        }

        Ok(Self { lut })
    }
}
