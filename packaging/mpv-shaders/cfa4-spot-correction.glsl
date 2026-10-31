// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 CFA4 contributors
//
// CFA4 spot correction (mpv / libplacebo user shader).
//
// Corrects an elliptical area whose colour is off on the screen (a hot spot, a tinted patch of a
// projector panel or of the screen material): inside the ellipse each channel is multiplied by
// its gain and moved by its offset, fading out over the feathered rim. Positions and radii are
// fractions of the frame (0..1, origin top left).
// Clean-room CFA4 code, see README.md.

//!PARAM center_x
//!DESC Spot centre, horizontal (0 = left, 1 = right)
//!TYPE float
//!MINIMUM 0.0
//!MAXIMUM 1.0
0.5

//!PARAM center_y
//!DESC Spot centre, vertical (0 = top, 1 = bottom)
//!TYPE float
//!MINIMUM 0.0
//!MAXIMUM 1.0
0.5

//!PARAM radius_x
//!DESC Horizontal radius, fraction of the frame width
//!TYPE float
//!MINIMUM 0.001
//!MAXIMUM 1.0
0.2

//!PARAM radius_y
//!DESC Vertical radius, fraction of the frame height
//!TYPE float
//!MINIMUM 0.001
//!MAXIMUM 1.0
0.2

//!PARAM feather
//!DESC Soft rim, share of the radius (0 = hard edge)
//!TYPE float
//!MINIMUM 0.0
//!MAXIMUM 1.0
0.5

//!PARAM gain_r
//!TYPE float
//!MINIMUM 0.0
//!MAXIMUM 4.0
1.0

//!PARAM gain_g
//!TYPE float
//!MINIMUM 0.0
//!MAXIMUM 4.0
1.0

//!PARAM gain_b
//!TYPE float
//!MINIMUM 0.0
//!MAXIMUM 4.0
1.0

//!PARAM offset_r
//!TYPE float
//!MINIMUM -1.0
//!MAXIMUM 1.0
0.0

//!PARAM offset_g
//!TYPE float
//!MINIMUM -1.0
//!MAXIMUM 1.0
0.0

//!PARAM offset_b
//!TYPE float
//!MINIMUM -1.0
//!MAXIMUM 1.0
0.0

//!HOOK MAIN
//!BIND HOOKED
//!DESC CFA4 spot correction

vec4 hook() {
    vec4 c = HOOKED_tex(HOOKED_pos);
    vec2 r = max(vec2(radius_x, radius_y), vec2(1e-3));
    float d = length((HOOKED_pos - vec2(center_x, center_y)) / r);
    float f = clamp(feather, 0.0, 1.0);
    // 1 inside, 0 outside, smooth over the rim
    float w = f > 0.0 ? 1.0 - smoothstep(1.0 - f, 1.0, d) : (d <= 1.0 ? 1.0 : 0.0);
    vec3 fixed_rgb = max(c.rgb * vec3(gain_r, gain_g, gain_b) + vec3(offset_r, offset_g, offset_b), 0.0);
    return vec4(mix(c.rgb, fixed_rgb, w), c.a);
}
