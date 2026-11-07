// SPDX-License-Identifier: LGPL-2.1-or-later
// Ported from FFmpeg's DTS decoder (libavcodec/dca*.c, Copyright (C) 2016 foo86 and the FFmpeg
// developers, LGPL-2.1-or-later) by the `dca` crate 0.9.0 of harletty-bridge
// (https://github.com/harletty/harletty-bridge, commit 74cf6ed), which published it as
// Apache-2.0. As FFmpeg-derived code it is used here under LGPL-2.1-or-later (docs/licensing.md
// §5). Modified for CFA4: see crates/dts/README.md. The harletty-bridge header follows.
//
// (harletty-bridge: SPDX-License-Identifier: Apache-2.0)

/// Speaker assignment for a decoded bed channel. Mirrors `eac3::BedChannel` so
/// the bridge can map both decoders' output through the same `labels.rs` path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BedChannel {
    FrontLeft,
    FrontRight,
    Center,
    LowFrequencyEffects,
    SurroundLeft,
    SurroundRight,
    RearCenter,
    RearLeft,
    RearRight,
    WideLeft,
    WideRight,
}
