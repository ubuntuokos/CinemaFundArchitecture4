// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 CFA4 contributors
//
// CFA4 black bar filler (mpv / libplacebo user shader).
//
// A picture of aspect `content_aspect` letterboxed (or pillarboxed) in the frame is stretched over
// the black bars. The stretch follows a sine profile, as if the picture were wrapped on a cylinder:
// the middle keeps its proportions as far as possible and the edges are stretched the most.
// `warp` = 0 is a plain uniform stretch, 1 is the strongest edge-weighted stretch.
// Clean-room CFA4 code, see README.md.

//!PARAM content_aspect
//!DESC Aspect ratio of the picture inside the bars (2.39 = scope)
//!TYPE float
//!MINIMUM 0.5
//!MAXIMUM 4.0
2.39

//!PARAM warp
//!DESC 0 = uniform stretch, 1 = keep the middle, stretch the edges
//!TYPE float
//!MINIMUM 0.0
//!MAXIMUM 1.0
1.0

//!HOOK MAIN
//!BIND HOOKED
//!DESC CFA4 black bar filler

const float HALF_PI = 1.5707963;

// The angle a (at most pi/2, where the profile stays monotonic) with sin(a) / a = c, so that the
// middle of the picture keeps a 1:1 scale: bisection, since sin(a) - c a is positive below the
// root and negative above it.
float keep_angle(float c) {
    if (c <= 2.0 / 3.1415927) {
        return HALF_PI;
    }
    float lo = 1e-3;
    float hi = HALF_PI;
    for (int i = 0; i < 24; i++) {
        float m = 0.5 * (lo + hi);
        if (sin(m) > c * m) {
            lo = m;
        } else {
            hi = m;
        }
    }
    return 0.5 * (lo + hi);
}

// Output position t in -1..1 across the frame to the picture position in -c..c.
float profile(float t, float c, float a) {
    if (a < 1e-3) {
        return c * t;
    }
    return c * sin(a * t) / sin(a);
}

vec4 hook() {
    vec2 pos = HOOKED_pos;
    float frame_aspect = HOOKED_size.x / max(HOOKED_size.y, 1.0);
    float ca = max(content_aspect, 0.01);
    bool letterbox = ca >= frame_aspect;
    // the share of the frame the picture covers along the barred axis
    float c = letterbox ? frame_aspect / ca : ca / frame_aspect;
    if (c >= 1.0) {
        return HOOKED_tex(pos);
    }
    float a = clamp(warp, 0.0, 1.0) * keep_angle(c);
    if (letterbox) {
        pos.y = 0.5 + 0.5 * profile(2.0 * pos.y - 1.0, c, a);
    } else {
        pos.x = 0.5 + 0.5 * profile(2.0 * pos.x - 1.0, c, a);
    }
    return HOOKED_tex(pos);
}
