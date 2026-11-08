// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 CFA4 contributors
//
// CFA4 ScreenXizer, cylinder (mpv / libplacebo user shader).
//
// Spreads a normal picture over three screens side by side: the output frame is the left side
// screen, the main screen and the right side screen, `side_width` of the frame each side. The
// main screen shows the middle `main_source` share of the picture at its own scale; the picture's
// remaining left and right edges are stretched over the side screens following a sine (cylinder)
// profile, most stretched at the far ends. Play it with the output aspect of the whole screen
// row, for example `--video-aspect-override=48:9` for three 16:9 screens.
// Clean-room CFA4 code, see README.md.

//!PARAM side_width
//!DESC Width of one side screen, share of the output frame
//!TYPE float
//!MINIMUM 0.0
//!MAXIMUM 0.45
0.3333

//!PARAM main_source
//!DESC Share of the picture width shown on the main screen
//!TYPE float
//!MINIMUM 0.2
//!MAXIMUM 1.0
0.8

//!PARAM curve
//!DESC 0 = linear stretch on the side screens, 1 = cylinder wrap
//!TYPE float
//!MINIMUM 0.0
//!MAXIMUM 1.0
1.0

//!HOOK MAIN
//!BIND HOOKED
//!DESC CFA4 ScreenXizer (cylinder)

vec4 hook() {
    vec2 pos = HOOKED_pos;
    float s = clamp(side_width, 0.0, 0.45);
    float k = clamp(main_source, 0.0, 1.0);
    float x = pos.x;
    float src;
    if (s <= 0.0 || (x >= s && x <= 1.0 - s)) {
        src = 0.5 + (x - 0.5) / (1.0 - 2.0 * s) * k;
    } else {
        // 0 at the main screen's edge, 1 at the far end of the side screen
        float q = clamp(x < s ? (s - x) / s : (x - (1.0 - s)) / s, 0.0, 1.0);
        float a = clamp(curve, 0.0, 1.0) * 1.5707963;
        float g = a < 1e-3 ? q : sin(a * q) / sin(a);
        float d = 0.5 * k + 0.5 * (1.0 - k) * g;
        src = x < s ? 0.5 - d : 0.5 + d;
    }
    return HOOKED_tex(vec2(src, pos.y));
}
