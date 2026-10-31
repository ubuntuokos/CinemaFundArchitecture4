// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 CFA4 contributors
//
// CFA4 dual projector convergence (mpv / libplacebo user shader).
//
// For two projectors fed from one over/under or side-by-side frame (stereo 3D or dual-projector
// HDR): the second view (bottom or right half) is moved and scaled inside its own half until its
// projection lands on the first one. The first view is the reference and stays as it is. Pixels
// the moved view no longer covers become black; nothing from the other half leaks in.
// Clean-room CFA4 code, see README.md.

//!PARAM layout
//!DESC Frame layout: 0 = over/under, 1 = side by side
//!TYPE int
//!MINIMUM 0
//!MAXIMUM 1
0

//!PARAM shift_x
//!DESC Horizontal shift of the second view, % of the view width
//!TYPE float
//!MINIMUM -20.0
//!MAXIMUM 20.0
0.0

//!PARAM shift_y
//!DESC Vertical shift of the second view, % of the view height
//!TYPE float
//!MINIMUM -20.0
//!MAXIMUM 20.0
0.0

//!PARAM scale
//!DESC Size of the second view, % (100 = unchanged)
//!TYPE float
//!MINIMUM 80.0
//!MAXIMUM 120.0
100.0

//!HOOK MAIN
//!BIND HOOKED
//!DESC CFA4 dual projector convergence

vec4 hook() {
    vec2 pos = HOOKED_pos;
    // the view this pixel belongs to, its origin and size in texture coordinates
    vec2 vsize = layout == 1 ? vec2(0.5, 1.0) : vec2(1.0, 0.5);
    vec2 origin = layout == 1 ? vec2(pos.x < 0.5 ? 0.0 : 0.5, 0.0) : vec2(0.0, pos.y < 0.5 ? 0.0 : 0.5);
    if (origin.x == 0.0 && origin.y == 0.0) {
        return HOOKED_tex(pos);
    }
    vec2 local = (pos - origin) / vsize;
    float k = max(scale, 1.0) / 100.0;
    vec2 src = (local - 0.5 - vec2(shift_x, shift_y) / 100.0) / k + 0.5;
    if (any(lessThan(src, vec2(0.0))) || any(greaterThan(src, vec2(1.0)))) {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }
    // keep the bilinear taps inside the view
    vec2 half_texel = 0.5 / (vsize * HOOKED_size);
    src = clamp(src, half_texel, 1.0 - half_texel);
    return HOOKED_tex(origin + src * vsize);
}
