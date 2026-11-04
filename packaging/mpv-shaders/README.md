# CFA4 playback shaders for mpv

User shaders for **projection-time** corrections. They are not editor effects: they run while a
finished film plays, in [mpv](https://mpv.io/) or in anything else that loads mpv-format user
shaders through libplacebo (for example FFmpeg's `libplacebo` filter). The editor's own
counterparts are the Stereoscopic 3D effects (Swap Eyes, Stereo to 2D, Stereo Parallax) and
Distort > Six-Point Pin.

The functions follow the list of the
[Cinema Shader Pack](https://github.com/VoidXH/Cinema-Shader-Pack) (MPC-HC HLSL shaders). Its
licence is not GPL-compatible, so none of its code is used here: these shaders are CFA4's own
clean-room GLSL, written from the one-line feature descriptions and published under
GPL-3.0-or-later like the rest of CFA4.

| Shader | What it does |
|---|---|
| `cfa4-dual-projector-convergence.glsl` | Two projectors fed from one over/under or side-by-side frame (3D or dual-projector HDR): moves and scales the second view inside its half until the two projections line up. |
| `cfa4-black-bar-filler.glsl` | Stretches a letterboxed (or pillarboxed) picture over the black bars, keeping the middle 1:1 and stretching the edges the most (cylinder profile). |
| `cfa4-spot-correction.glsl` | Corrects an elliptical spot whose colour is off (a hot spot, a tinted patch): per-channel gain and offset with a soft rim. |
| `cfa4-screenxizer-cylinder.glsl` | Spreads a normal picture over three screens: the middle on the main screen, the edges stretched over the two side screens. |

## Use

```sh
mpv --vo=gpu-next --glsl-shader=/path/to/cfa4-black-bar-filler.glsl \
    --glsl-shader-opts=content_aspect=2.39,warp=1 film.mkv
```

The shaders need mpv's **`gpu-next`** video output (mpv 0.35 or newer; libplacebo): their
parameters are `//!PARAM` blocks, which the older `--vo=gpu` does not understand, so editing the
defaults does not make them work there. Several shaders can be chained with
`--glsl-shaders-append=…`. The parameters (at the top of each file, with their ranges and
defaults) are set with `--glsl-shader-opts`, or by editing the default value under the block. Positions are fractions of the
frame (0 to 1, origin at the top left) unless the description says otherwise.

- **Dual projector convergence:** `layout` 0 = over/under, 1 = side by side; `shift_x`, `shift_y`
  in % of the view, `scale` in %. The first view (top or left) is the reference.
- **Black bar filler:** `content_aspect` is the picture's own aspect ratio inside the bars (2.39
  for scope in a 16:9 frame); `warp` 0 is a uniform stretch, 1 keeps the middle at its own scale.
  The shader sees stored pixels, so for anamorphic video (non-square pixels, e.g. a 720×576 16:9
  DVD) set `frame_aspect` to the displayed aspect of the whole frame (1.7778); 0 takes it from the
  pixel size.
- **Spot correction:** `center_x`, `center_y`, `radius_x`, `radius_y`, `feather`, then
  `gain_r/g/b` (multipliers) and `offset_r/g/b` (added after the gain). The values act on the
  video signal as decoded, before mpv's colour management.
- **ScreenXizer:** `side_width` is one side screen's share of the output frame, `main_source` the
  share of the picture shown on the main screen, `curve` 0 = linear, 1 = cylinder wrap. The shader
  keeps the frame size, so play it with the aspect ratio of the whole screen row, for example
  `--video-aspect-override=48:9 --keepaspect=no` for three 16:9 screens.

## Tests

`python/tests/test_mpv_shaders.py` checks the files' structure and renders a coordinate ramp through
every shader with FFmpeg's `libplacebo` filter on a Vulkan device (lavapipe is enough), comparing
each pixel with the documented mapping. Without such a device the render checks skip.
