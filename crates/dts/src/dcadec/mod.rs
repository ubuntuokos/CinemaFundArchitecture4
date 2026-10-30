// SPDX-License-Identifier: LGPL-2.1-or-later
// Ported from FFmpeg's DTS decoder (libavcodec/dca*.c, Copyright (C) 2016 foo86 and the FFmpeg
// developers, LGPL-2.1-or-later) by the `dca` crate 0.9.0 of harletty-bridge
// (https://github.com/harletty/harletty-bridge, commit 74cf6ed), which published it as
// Apache-2.0. As FFmpeg-derived code it is used here under LGPL-2.1-or-later (docs/licensing.md
// §5). Modified for CFA4: see crates/dts/README.md. The harletty-bridge header follows.
//
// (harletty-bridge: SPDX-License-Identifier: Apache-2.0)
//
// DCA core decoder internals: generated tables, Huffman VLCs, and the subband
// DSP decode (ported incrementally from ffmpeg's dca_core.c / dcadsp.c).

/// Define an elementwise loop over sample buffers that runs as AVX-512 or
/// AVX2 code where the CPU has it: the same loop compiled three times. The
/// arithmetic is integer, so all give the same samples; the baseline x86-64
/// target has no instructions for the 24-bit clips and the 64-bit products
/// these loops are made of, and AVX2 none for the 64-bit arithmetic shifts
/// and full 64-bit products that AVX-512 (DQ) adds on twice the width.
macro_rules! sample_loop {
    ($(#[$doc:meta])* fn $name:ident($($arg:ident: $ty:ty),* $(,)?) $body:block) => {
        $(#[$doc])*
        fn $name($($arg: $ty),*) {
            #[inline(always)]
            fn body($($arg: $ty),*) $body
            body($($arg),*)
        }
    };
}

pub(crate) mod buffers;
pub(crate) mod core;
pub(crate) mod exss;
pub(crate) mod huffman;
pub(crate) mod synth;
pub(crate) mod tables;
pub(crate) mod xll;
pub(crate) mod xmeta;
pub(crate) mod xrender;
