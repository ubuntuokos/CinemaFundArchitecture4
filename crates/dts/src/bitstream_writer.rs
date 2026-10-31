// SPDX-License-Identifier: LGPL-2.1-or-later
// Ported from FFmpeg's DTS decoder (libavcodec/dca*.c, Copyright (C) 2016 foo86 and the FFmpeg
// developers, LGPL-2.1-or-later) by the `dca` crate 0.9.0 of harletty-bridge
// (https://github.com/harletty/harletty-bridge, commit 74cf6ed), which published it as
// Apache-2.0. As FFmpeg-derived code it is used here under LGPL-2.1-or-later (docs/licensing.md
// §5). Modified for CFA4: see crates/dts/README.md. The harletty-bridge header follows.
//
// (harletty-bridge: SPDX-License-Identifier: Apache-2.0)
//
// Minimal MSB-first bit writer, the inverse of `BitReader`. Used to synthesize
// bitstreams in unit tests.

#[derive(Default)]
pub(crate) struct BitWriter {
    bytes: Vec<u8>,
    cur: u8,
    nbits: u8,
}

impl BitWriter {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn write(&mut self, bits: usize, value: u32) {
        for i in (0..bits).rev() {
            let bit = ((value >> i) & 1) as u8;
            self.cur = (self.cur << 1) | bit;
            self.nbits += 1;
            if self.nbits == 8 {
                self.bytes.push(self.cur);
                self.cur = 0;
                self.nbits = 0;
            }
        }
    }

    pub(crate) fn finish(mut self) -> Vec<u8> {
        if self.nbits > 0 {
            self.cur <<= 8 - self.nbits;
            self.bytes.push(self.cur);
        }
        self.bytes
    }
}
