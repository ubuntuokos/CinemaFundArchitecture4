//! Standard effects with a GPU implementation (`cfa4_gpu` runs them in WGSL).
//!
//! Each effect here is evaluated in two steps: [`FxOp::eval`] reads the effect's parameters at a
//! time (keyframes, defaults, `px_scale`, the working image size) into plain numbers, and
//! [`FxOp::apply`] runs the CPU reference on a working [`Image`]. `effects::apply` uses exactly
//! these two steps, so the CPU render and the GPU plan agree on every parameter (and on how
//! hostile values are clamped) by construction. The GPU compositor receives the evaluated
//! [`FxOp`]s in a [`crate::plan::LayerFx`] and reproduces `apply` per pixel; the plan only hands an
//! op to the GPU when [`FxOp::gpu_ok`] says the shader covers it (finite numbers, no minifying
//! resample), otherwise the clip is rendered on the CPU as before.

use cfa4_color::{hsl_to_rgb, linear_to_srgb, luma709, rgb_to_hsl};
use cfa4_geom::{Affine, Vec2};
use cfa4_project::EffectInstance;
use rayon::prelude::*;

use crate::effects::{FxCtx, b, choice, color, dec, enc, f, gaussian_boxes, grade_space, on, point, text};
use crate::image::Image;
pub use crate::vfx::glitch::{STREAM_SCAN, STREAM_VHS, Vhs, stream_base, vhs_grey};

/// A bilinear resample of the working image through an affine map (`Image::transformed` at a
/// magnification or mild minification: no mip pre-filter).
#[derive(Clone, Debug, PartialEq)]
pub struct Resample {
    /// Destination pixel → source pixel (None: singular map, the result is transparent).
    pub inv: Option<Affine>,
    /// Destination pixels written: x0..x1, y0..y1 (the rest becomes transparent).
    pub rect: [u32; 4],
    /// Alpha scale applied afterwards (Transform's Opacity; 1 = none).
    pub opacity: f32,
}

/// One evaluated effect.
#[derive(Clone, Debug, PartialEq)]
pub enum FxOp {
    BrightnessContrast {
        br: f32,
        co: f32,
    },
    ProcAmp {
        br: f32,
        co: f32,
        hue: f32,
        sat: f32,
    },
    Tint {
        black: [f32; 3],
        white: [f32; 3],
        amount: f32,
    },
    BlackWhite,
    ColorBalance {
        sh: [f32; 3],
        md: [f32; 3],
        hi: [f32; 3],
        preserve: bool,
    },
    LeaveColor {
        amount: f32,
        key_hue: f32,
        tol: f32,
        soft: f32,
    },
    ChangeToColor {
        from_hue: f32,
        to_hue: f32,
        tol: f32,
        soft: f32,
    },
    ColorPass {
        key: [f32; 3],
        sim: f32,
        reverse: bool,
    },
    Gamma {
        g: f32,
    },
    Levels {
        ib: f32,
        iw: f32,
        ob: f32,
        ow: f32,
        g: f32,
    },
    Extract {
        lo: f32,
        hi: f32,
        soft: f32,
        invert: bool,
    },
    Invert {
        channel: u32,
        blend: f32,
    },
    Posterize {
        n: f32,
    },
    /// Three box passes per axis approximating a Gaussian (radii per pass; empty: axis untouched).
    Gaussian {
        rx: Vec<u32>,
        ry: Vec<u32>,
        repeat: bool,
    },
    /// `steps` bilinear (edge-clamped) taps along `(dx, dy)` centred on the pixel.
    DirectionalBlur {
        dx: f32,
        dy: f32,
        steps: u32,
    },
    /// Skin Smoothing ([`skin_mask`], [`skin_retouch`]): a guided filter (luma guide) over
    /// repeat-edge box-Gaussians of the given radii, inside the skin mask, with Keep Texture's
    /// share of the removed detail added back. `show` draws the mask instead.
    SkinSmoothing {
        rx: Vec<u32>,
        ry: Vec<u32>,
        eps: f32,
        amount: f32,
        keep: f32,
        mask: SkinMask,
        show: bool,
    },
    /// Low Light ([`low_light_gain`]): the illumination (brightest channel) smoothed by a
    /// self-guided filter over repeat-edge box-Gaussians of the given radii, then one gain per
    /// pixel, `illumination ^ -k` capped at `max_gain`. `show` draws the illumination instead.
    LowLight {
        rx: Vec<u32>,
        ry: Vec<u32>,
        k: f32,
        max_gain: f32,
        show: bool,
    },
    /// Unsharp mask over a repeat-edge Gaussian of the given box radii (per axis).
    Unsharp {
        rx: Vec<u32>,
        ry: Vec<u32>,
        amount: f32,
        threshold: f32,
    },
    /// Crop / Edge Feather: alpha falloff `feather` px inside the rectangle (0: half-pixel AA).
    Crop {
        x0: f32,
        x1: f32,
        y0: f32,
        y1: f32,
        feather: f32,
    },
    Resample(Resample),
    HFlip,
    VFlip,
    /// Swap Eyes: the left and right views of an over/under (`sbs` false) or side-by-side frame
    /// trade places ([`swap_eyes_src`]).
    SwapEyes {
        sbs: bool,
    },
    /// Stereo to 2D: one view (the left, or the right with `right`) stretched over the whole frame
    /// along the split axis ([`stereo_view_sample`]).
    StereoTo2d {
        sbs: bool,
        right: bool,
    },
    /// Stereo Parallax: the left view moves `shift` pixels right and the right view as far left
    /// (negative: the other way), each inside its own view ([`stereo_parallax_sample`]); uncovered
    /// pixels become transparent or, with `repeat`, repeat the view's edge.
    StereoParallax {
        sbs: bool,
        shift: f32,
        repeat: bool,
    },
    /// Six-Point Pin: the frame mapped between two curved edges ([`six_point_source`]). `edges` is
    /// the top edge's start, control and end point, then the bottom edge's, as x, y in working
    /// pixels; `inv` is the inverse of the corners' homography (row-major), the Newton start.
    SixPointPin {
        edges: [f32; 12],
        inv: [f32; 9],
        identity: bool,
    },
    /// CRT: a curved tube (`curve` = barrel factor, 0 = flat) showing the picture through
    /// `lines` scanlines (`scan` = depth of the gaps), a phosphor mask (`mask` 0 none, 1 aperture
    /// grille, 2 shadow mask, 3 slot mask; `mask_amt` = how dark the other phosphors are, `pitch` =
    /// one triad in working pixels), darker corners and a per-frame brightness `gain` (flicker).
    /// See [`crt_warp`] and [`crt_shade`].
    Crt {
        scan: f32,
        lines: f32,
        mask: u32,
        mask_amt: f32,
        pitch: f32,
        curve: f32,
        corner: f32,
        gain: f32,
    },
    /// Oil Paint: a generalized Kuwahara filter (the mean of each sector of a `radius` neighbourhood,
    /// weighted towards the calmest sectors with sharpness `q`; see [`oil_sectors`]) on the
    /// premultiplied image, then the style's canvas texture ([`oil_texture`]) scaled by `texture`.
    /// `style` is the index into `cfa4_project::effect::OIL_STYLES`.
    OilPaint {
        style: u32,
        radius: u32,
        q: f32,
        texture: f32,
        seed: u32,
    },
    /// Film Grain: smooth value noise ([`grain_noise`]) on display-encoded colour, weighted by a
    /// shadows / midtones / highlights response. `inv_cell` is 1 / the grain cell in working
    /// pixels; `seed` already folds in the frame when the grain is animated.
    /// Halftone: `screens` hold (cos, sin) of the C, M, Y, K screen angles (monochrome uses the
    /// last one); `inv_cell` is 1 / the cell in working pixels, `soft` the half width of the spot
    /// edge in cell units ([`halftone_ink`]); colours are display-encoded.
    Halftone {
        mono: bool,
        lines: bool,
        inv_cell: f32,
        soft: f32,
        screens: [[f32; 2]; 4],
        ink: [f32; 3],
        paper: [f32; 3],
        blend: f32,
    },
    /// Ordered dither ([`dither_px`]): `pattern` 0–2 Bayer 2^(n+1), 3 noise; `block` px per dither
    /// pixel; `palette` 0 quantizes each channel to `levels`, else [`DITHER_PALETTES`] entry.
    Dither {
        pattern: u32,
        block: u32,
        threshold: f32,
        palette: u32,
        levels: f32,
        blend: f32,
    },
    /// Bit Crush ([`crate::vfx::glitch::bit_crush`]): `top` = 2^bits − 1, Bayer cells of `cell` px.
    BitCrush {
        top: f32,
        dither: bool,
        cell: f32,
    },
    /// Block Glitch ([`crate::vfx::glitch::block_glitch`]): `rng` is the generator's start state.
    BlockGlitch {
        count: u32,
        tw: u32,
        th: u32,
        maxd: i32,
        snap: bool,
        swap: bool,
        rng: u32,
    },
    /// Databend ([`crate::vfx::glitch::databend`]).
    Databend {
        rng: u32,
        breaks: u32,
        shift: f32,
        color: f32,
        block: u32,
    },
    /// VHS ([`crate::vfx::glitch::vhs`]).
    Vhs(Vhs),
    /// Scan Line Jitter ([`crate::vfx::glitch::scan_line_jitter`]).
    ScanLineJitter {
        vertical: bool,
        disp: f32,
        thr: f32,
        seed: u32,
        tick: u32,
    },
    /// Tile Jitter ([`crate::vfx::glitch::tile_jitter`]).
    TileJitter {
        dx: f32,
        dy: f32,
        tiles: f32,
        across: bool,
    },
    FilmGrain {
        amount: f32,
        inv_cell: f32,
        shadows: f32,
        midtones: f32,
        highlights: f32,
        color: f32,
        seed: u32,
    },
    Mirror {
        cx: f32,
        cy: f32,
        nx: f32,
        ny: f32,
    },
    /// Wrap-around shift (dx, dy already reduced into 0..w, 0..h), mixed with the original.
    Offset {
        dx: f32,
        dy: f32,
        blend: f32,
    },
    /// ASC CDL on display-encoded colour (slope, offset, power, saturation).
    AscCdl {
        slope: [f32; 3],
        offset: [f32; 3],
        power: [f32; 3],
        sat: f32,
    },
    /// Channel Mixer: rows (r, g, b, constant) per output channel, on display-encoded colour.
    ChannelMix {
        m: [[f32; 4]; 3],
    },
    /// Color Replace: `sim` already ×1.2; `replace_hsl` the HSL of the replacement colour.
    ColorReplace {
        sim: f32,
        solid: bool,
        target: [f32; 3],
        replace: [f32; 3],
        replace_hsl: [f32; 3],
    },
    AlphaAdjust {
        opacity: f32,
        ignore: bool,
        invert: bool,
        mask_only: bool,
    },
    Vignette {
        amount: f32,
        midpoint: f32,
        roundness: f32,
        feather: f32,
        target: [f32; 3],
    },
    VideoLimiter {
        max: f32,
        comp: f32,
        axis: u32,
        warn: bool,
        warning_color: [f32; 3],
    },
    Lumetri {
        gains: [f32; 3],
        exposure: f32,
        contrast: f32,
        hl: f32,
        sh: f32,
        wh: f32,
        bl: f32,
        sat: f32,
        creative_on: bool,
        faded: f32,
        vib: f32,
        st: [f32; 3],
        ht: [f32; 3],
        vignette_on: bool,
        va: f32,
        vmid: f32,
        vround: f32,
        vfeather: f32,
        aspect: f32,
        gpu_capable: bool,
    },
}

/// Effect ids [`FxOp::eval`] understands (the GPU-capable standard effects).
pub const GPU_EFFECTS: &[&str] = &[
    "brightness_contrast",
    "proc_amp",
    "tint",
    "black_white",
    "color_balance",
    "leave_color",
    "change_to_color",
    "color_pass",
    "gamma_correction",
    "levels",
    "extract",
    "invert",
    "posterize",
    "gaussian_blur",
    "gaussian_blur_legacy",
    "camera_blur",
    "directional_blur",
    "directional_blur_legacy",
    "sharpen",
    "unsharp_mask",
    "crop",
    "edge_feather",
    "transform",
    "horizontal_flip",
    "vertical_flip",
    "stereo_swap_eyes",
    "stereo_to_2d",
    "stereo_parallax",
    "six_point_pin",
    "film_grain",
    "low_light",
    "skin_smoothing",
    "halftone",
    "dither",
    "bit_crush",
    "block_glitch",
    "databend",
    "vhs",
    "scan_line_jitter",
    "tile_jitter",
    "crt",
    "oil_paint",
    "mirror",
    "offset",
    "asc_cdl",
    "channel_mix",
    "color_replace",
    "alpha_adjust",
    "vignette",
    "video_limiter",
    "lumetri",
];

/// PCG3D (Jarzynski & Olano, "Hash Functions for GPU Rendering", JCGT 2020): three rounds of
/// 32-bit multiply-xorshift mixing; the first lane is the hash. `fx.wgsl` (`pcg3d`) mirrors it
/// bit for bit (u32 arithmetic wraps on both sides).
pub fn pcg3d(x: u32, y: u32, z: u32) -> u32 {
    let mut v = [x, y, z].map(|c| c.wrapping_mul(1_664_525).wrapping_add(1_013_904_223));
    let mix = |v: &mut [u32; 3]| {
        v[0] = v[0].wrapping_add(v[1].wrapping_mul(v[2]));
        v[1] = v[1].wrapping_add(v[2].wrapping_mul(v[0]));
        v[2] = v[2].wrapping_add(v[0].wrapping_mul(v[1]));
    };
    mix(&mut v);
    for c in &mut v {
        *c ^= *c >> 16;
    }
    mix(&mut v);
    v[0]
}

/// A lattice value in [−0.5, 0.5) for cell (`ix`, `iy`) of noise layer `layer` (24 bits, exact
/// in f32).
fn lattice(ix: i32, iy: i32, seed: u32, layer: u32) -> f32 {
    let h = pcg3d(ix as u32, iy as u32, seed ^ layer.wrapping_mul(0x9E37_79B9));
    (h >> 8) as f32 / 16_777_216.0 - 0.5
}

/// Smooth value noise in [−0.5, 0.5] at pixel (`x`, `y`): lattice values on a grid of
/// 1 / `inv_cell` pixels, blended with a smoothstep. `fx.wgsl` (`grain_noise`) mirrors this.
pub fn grain_noise(x: usize, y: usize, inv_cell: f32, seed: u32, layer: u32) -> f32 {
    let fx = (x as f32 + 0.5) * inv_cell;
    let fy = (y as f32 + 0.5) * inv_cell;
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let (ix, iy) = (x0 as i32, y0 as i32);
    let a = lattice(ix, iy, seed, layer);
    let b = lattice(ix.wrapping_add(1), iy, seed, layer);
    let c = lattice(ix, iy.wrapping_add(1), seed, layer);
    let d = lattice(ix.wrapping_add(1), iy.wrapping_add(1), seed, layer);
    let top = a + (b - a) * sx;
    let bot = c + (d - c) * sx;
    top + (bot - top) * sy
}

/// Ink coverage (0–1) of a halftone screen at pixel centre (`px`, `py`) for tone `v` (0 = no
/// ink, 1 = solid): the pixel rotated by (`cs`, `sn`) into cells of 1 / `inv_cell` pixels and
/// thresholded against a spot function in [−1, 1] whose area above `1 − 2v` is about `v` — the
/// cosine dot (dots that merge into a checkerboard at 50 % and into holes beyond) or a triangle
/// across the cell for lines — with a linear edge `soft` (spot units) wide on each side.
/// `fx.wgsl` (`halftone_ink`) mirrors this.
pub fn halftone_ink(px: f32, py: f32, cs: f32, sn: f32, inv_cell: f32, soft: f32, lines: bool, v: f32) -> f32 {
    if v.is_nan() || v <= 0.0 {
        return 0.0;
    }
    if v >= 1.0 {
        return 1.0;
    }
    let u = (px * cs + py * sn) * inv_cell;
    let w = (py * cs - px * sn) * inv_cell;
    let fu = u - u.floor() - 0.5;
    let fw = w - w.floor() - 0.5;
    let tau = std::f32::consts::TAU;
    let spot = if lines { 1.0 - 4.0 * fw.abs() } else { -((tau * fu).cos() + (tau * fw).cos()) * 0.5 };
    // the threshold reaches past the spot's range so 0 and 1 are exactly no ink and solid ink
    let th = (1.0 - 2.0 * v) * (1.0 + soft);
    ((spot - th) / (2.0 * soft) + 0.5).clamp(0.0, 1.0)
}

/// CRT tube coordinates (−1…1 across the frame) of pixel centre (`px`, `py`) in a `w` × `h`
/// frame, bent by the barrel factor `curve`; `None` outside the curved screen. `fx.wgsl`
/// (`crt_warp`) mirrors this.
pub fn crt_warp(px: f32, py: f32, w: f32, h: f32, curve: f32) -> Option<(f32, f32)> {
    let u = px / w * 2.0 - 1.0;
    let v = py / h * 2.0 - 1.0;
    let cu = u * (1.0 + curve * v * v);
    let cv = v * (1.0 + curve * u * u);
    (cu.abs() <= 1.0 && cv.abs() <= 1.0).then_some((cu, cv))
}

/// The per-channel brightness of the CRT look at tube coordinates (`cu`, `cv`) and output pixel
/// centre (`px`, `py`): the scanline beam profile, the phosphor mask and the corner falloff (see
/// [`FxOp::Crt`]). `fx.wgsl` (`crt_shade`) mirrors this.
#[allow(clippy::too_many_arguments)]
pub fn crt_shade(cu: f32, cv: f32, px: f32, py: f32, scan: f32, lines: f32, mask: u32, mask_amt: f32, pitch: f32, corner: f32) -> [f32; 3] {
    // the beam: brightest in the middle of each scanline, darkest between two
    let t = (cv + 1.0) * 0.5 * lines;
    let beam = 1.0 - scan * (1.0 - (std::f32::consts::PI * (t - t.floor())).sin());
    // the phosphors: one lit, the other two dimmed by `mask_amt`
    let mut m = [1.0f32; 3];
    if mask > 0 && mask_amt > 0.0 {
        let odd = |v: f32| v - 2.0 * (v * 0.5).floor() >= 1.0;
        let tx = px / pitch;
        let col = tx.floor();
        let mut lit = true;
        let mut sx = tx;
        if mask == 2 {
            // shadow mask: each row of triads shifted by half a triad
            if odd((py / pitch * 2.0).floor()) {
                sx += 0.5;
            }
        } else if mask == 3 {
            // slot mask: grille stripes broken by staggered horizontal gaps
            let ty = py / pitch + if odd(col) { 0.5 } else { 0.0 };
            lit = ty - ty.floor() >= 0.25;
        }
        let k = (((sx - sx.floor()) * 3.0).floor() as usize).min(2);
        m = [1.0 - mask_amt; 3];
        if lit && let Some(c) = m.get_mut(k) {
            *c = 1.0;
        }
    }
    // the tube's corners: dark at the rim of the screen
    let rim = ((1.0 - cu * cu) * (1.0 - cv * cv)).max(0.0).sqrt().sqrt();
    let k = beam * (1.0 - corner * (1.0 - rim));
    [m[0] * k, m[1] * k, m[2] * k]
}

/// The Kuwahara sectors of Oil Paint `style` that offset (`dx`, `dy`) belongs to, as a bit mask
/// (0: outside the brush of radius `r`). Facet uses four square quadrants; Knife four quadrants
/// of a disc; the other styles eight octants of a disc. Boundary taps belong to both neighbours
/// and the centre to every sector; integer tests only, so `fx.wgsl` (`oil_sectors`) matches
/// exactly.
pub fn oil_sectors(style: u32, dx: i32, dy: i32, r: i32) -> u32 {
    if style != 0 && dx * dx + dy * dy > r * r {
        return 0;
    }
    let (e, n, w, s) = (dx >= 0, dy >= 0, dx <= 0, dy <= 0);
    if style == 0 || style == 4 {
        return (e && n) as u32 | ((w && n) as u32) << 1 | ((w && s) as u32) << 2 | ((e && s) as u32) << 3;
    }
    (e && n && dx >= dy) as u32
        | ((e && n && dy >= dx) as u32) << 1
        | ((w && n && dy >= -dx) as u32) << 2
        | ((w && n && -dx >= dy) as u32) << 3
        | ((w && s && -dx >= -dy) as u32) << 4
        | ((w && s && -dy >= -dx) as u32) << 5
        | ((e && s && -dy >= dx) as u32) << 6
        | ((e && s && dx >= -dy) as u32) << 7
}

/// The canvas texture of Oil Paint `style` at pixel (`x`, `y`) for brush radius `r`, within
/// ±0.6: a woven canvas, dry-brush streaks (Dry Brush), mottled plaster (Fresco) or sponge
/// pores (Sponge). `fx.wgsl` (`oil_texture`) mirrors this.
pub fn oil_texture(style: u32, x: usize, y: usize, r: u32, seed: u32) -> f32 {
    let weave = ((x / 2 + y / 2) % 2) as f32 - 0.5;
    let r = r.max(1) as f32;
    match style {
        2 => {
            let (ix, iy) = (((x as f32 + 0.5) / (8.0 * r)).floor() as i32, ((y as f32 + 0.5) / (r / 3.0).max(1.0)).floor() as i32);
            0.2 * weave + lattice(ix, iy, seed, 0)
        }
        3 => 0.2 * weave + grain_noise(x, y, 1.0 / (4.0 * r), seed, 0),
        5 => (2.0 * grain_noise(x, y, 2.0 / r, seed, 0)).clamp(-0.5, 0.5),
        _ => 0.4 * weave,
    }
}

/// The fixed palettes of Dither (`cfa4_project::effect::DITHER_PALETTES` after "Input Colors"),
/// 0xRRGGBB in display encoding. `fx.wgsl` (`DITHER_COLORS` / `dither_palette`) holds the same
/// table.
pub const DITHER_PALETTES: &[&[u32]] = &[
    // Black & White
    &[0x000000, 0xFFFFFF],
    // Green Phosphor
    &[0x000000, 0x33FF33],
    // Amber Monitor
    &[0x000000, 0xFFB000],
    // Game Boy
    &[0x0F380F, 0x306230, 0x8BAC0F, 0x9BBC0F],
    // CGA (palette 1, high intensity)
    &[0x000000, 0x55FFFF, 0xFF55FF, 0xFFFFFF],
    // EGA
    &[
        0x000000, 0x0000AA, 0x00AA00, 0x00AAAA, 0xAA0000, 0xAA00AA, 0xAA5500, 0xAAAAAA, 0x555555, 0x5555FF, 0x55FF55, 0x55FFFF, 0xFF5555, 0xFF55FF, 0xFFFF55,
        0xFFFFFF,
    ],
    // ZX Spectrum
    &[0x000000, 0x0000D7, 0xD70000, 0xD700D7, 0x00D700, 0x00D7D7, 0xD7D700, 0xD7D7D7, 0x0000FF, 0xFF0000, 0xFF00FF, 0x00FF00, 0x00FFFF, 0xFFFF00, 0xFFFFFF],
    // PICO-8
    &[
        0x000000, 0x1D2B53, 0x7E2553, 0x008751, 0xAB5236, 0x5F574F, 0xC2C3C7, 0xFFF1E8, 0xFF004D, 0xFFA300, 0xFFEC27, 0x00E436, 0x29ADFF, 0x83769C, 0xFF77A8,
        0xFFCCAA,
    ],
];

fn rgb_of(c: u32) -> [f32; 3] {
    [((c >> 16) & 0xFF) as f32 / 255.0, ((c >> 8) & 0xFF) as f32 / 255.0, (c & 0xFF) as f32 / 255.0]
}

/// The dither threshold in (−0.5, 0.5) of dither cell (`cx`, `cy`): a Bayer matrix of side
/// 2^(`pattern` + 1), or hashed noise for pattern 3.
pub fn dither_threshold(cx: u32, cy: u32, pattern: u32) -> f32 {
    if pattern >= 3 {
        return (pcg3d(cx, cy, 0x6469_7468) >> 8) as f32 / 16_777_216.0 - 0.5;
    }
    let n = pattern + 1;
    let mut v = 0u32;
    // the lowest coordinate bit picks the most significant digit: neighbouring thresholds lie
    // far apart (dispersed dots)
    for i in 0..n {
        let (xb, yb) = ((cx >> i) & 1, (cy >> i) & 1);
        v = (v << 2) | (((xb ^ yb) << 1) | yb);
    }
    (v as f32 + 0.5) / (1u32 << (2 * n)) as f32 - 0.5
}

/// One dithered, display-encoded colour `e` with threshold offset `d` (already pattern +
/// threshold): per-channel quantization to `levels`, or the nearest entry of palette `palette`
/// (two-colour palettes split on luma). `fx.wgsl` (`dither_px`) mirrors this.
pub fn dither_px(e: [f32; 3], d: f32, palette: u32, levels: f32) -> [f32; 3] {
    let Some(pal) = palette.checked_sub(1).and_then(|i| DITHER_PALETTES.get(i as usize)) else {
        let n = levels - 1.0;
        return e.map(|v| ((v * n + d + 0.5).floor().clamp(0.0, n)) / n);
    };
    if pal.len() == 2 {
        let l = 0.2126 * e[0] + 0.7152 * e[1] + 0.0722 * e[2];
        let i = usize::from(l + d > 0.5);
        return pal.get(i).map_or(e, |c| rgb_of(*c));
    }
    // spread the threshold over the palette's mean step
    let spread = 1.0 / (pal.len() as f32).cbrt();
    let t = e.map(|v| v + d * spread);
    let mut best = (f32::MAX, e);
    for c in pal.iter().map(|c| rgb_of(*c)) {
        let dd = (t[0] - c[0]) * (t[0] - c[0]) + (t[1] - c[1]) * (t[1] - c[1]) + (t[2] - c[2]) * (t[2] - c[2]);
        if dd < best.0 {
            best = (dd, c);
        }
    }
    best.1
}

/// The source pixel of output pixel (`x`, `y`) when the two views of a `w`×`h` stereo frame swap
/// places. Each view is `w / 2` columns (side by side) or `h / 2` rows (over/under); with an odd
/// size the last column / row belongs to neither view and stays where it is, so the swap is its
/// own inverse. `fx.wgsl` (`OP_SWAP_EYES`) mirrors this.
pub fn swap_eyes_src(x: usize, y: usize, w: usize, h: usize, sbs: bool) -> (usize, usize) {
    let swap = |v: usize, n: usize| {
        let half = n / 2;
        if v < half {
            v + half
        } else if v < 2 * half {
            v - half
        } else {
            v
        }
    };
    if sbs { (swap(x, w), y) } else { (x, swap(y, h)) }
}

/// Output pixel (`x`, `y`) of Stereo to 2D on `src`: the kept view (`w / 2` columns or `h / 2`
/// rows, the right / bottom one with `right`) stretched over the whole size along the split axis,
/// linearly interpolated along that axis and clamped to the view so the other eye never bleeds in.
/// The caller checks that the split size is at least 2. `fx.wgsl` (`OP_STEREO_TO_2D`) mirrors
/// this.
pub fn stereo_view_sample(src: &Image, x: usize, y: usize, sbs: bool, right: bool) -> [f32; 4] {
    let (n, at) = if sbs { (src.w, x) } else { (src.h, y) };
    let vw = n / 2;
    let v0 = if right { vw } else { 0 };
    let scale = vw as f32 / n as f32;
    let f = (v0 as f32 + (at as f32 + 0.5) * scale - 0.5).clamp(v0 as f32, (v0 + vw - 1) as f32);
    let lo = f.floor() as usize;
    let hi = (lo + 1).min(v0 + vw - 1);
    let t = f - lo as f32;
    let (a, b) = if sbs { (src.get(lo, y), src.get(hi, y)) } else { (src.get(x, lo), src.get(x, hi)) };
    let mut o = [0.0; 4];
    for k in 0..4 {
        o[k] = a[k] + (b[k] - a[k]) * t;
    }
    o
}

/// Output pixel (`x`, `y`) of Stereo Parallax on `src`: inside the left view the content moves
/// `shift` pixels right, inside the right view `shift` pixels left, linearly interpolated and never
/// crossing into the other view. Pixels outside both views (an odd last column / row) keep their
/// value. Uncovered pixels fade to transparent, or repeat the view's edge with `repeat`.
/// `fx.wgsl` (`OP_STEREO_PARALLAX`) mirrors this.
pub fn stereo_parallax_sample(src: &Image, x: usize, y: usize, sbs: bool, shift: f32, repeat: bool) -> [f32; 4] {
    let (w, h) = (src.w, src.h);
    let (vw, x0, left) = if sbs {
        let vw = w / 2;
        if x < vw {
            (vw, 0, true)
        } else if x < 2 * vw {
            (vw, vw, false)
        } else {
            return src.get(x, y);
        }
    } else {
        let half = h / 2;
        if y < half {
            (w, 0, true)
        } else if y < 2 * half {
            (w, 0, false)
        } else {
            return src.get(x, y);
        }
    };
    let off = if left { shift } else { -shift };
    let mut fx = (x - x0) as f32 - off;
    if repeat {
        fx = fx.clamp(0.0, (vw - 1) as f32);
    }
    let lo = fx.floor();
    let t = fx - lo;
    let tap = |i: f32| {
        if i >= 0.0 && i < vw as f32 { src.get(x0 + i as usize, y) } else { [0.0; 4] }
    };
    let (a, b) = (tap(lo), tap(lo + 1.0));
    let mut o = [0.0; 4];
    for k in 0..4 {
        o[k] = a[k] + (b[k] - a[k]) * t;
    }
    o
}

/// Newton steps of [`six_point_source`]: the corners' homography starts close enough that the
/// error squares each step, so eight reach f32 precision for any sensible screen shape.
pub const SIX_POINT_STEPS: usize = 8;

/// The source position (`u`, `v` in 0..1 of the frame) that Six-Point Pin draws at output point
/// (`qx`, `qy`), or `None` outside the warped frame. The frame's top edge runs along the quadratic
/// Bézier `edges[0..6]` (start, control, end), its bottom edge along `edges[6..12]`, and each
/// column `u` is the straight line between the two edges at `u`. Newton's method solves
/// `P(u, v) = q` from the corners' inverse homography `inv`. `fx.wgsl` (`OP_SIX_POINT_PIN`)
/// mirrors this in the same f32 order.
pub fn six_point_source(edges: &[f32; 12], inv: &[f32; 9], qx: f32, qy: f32) -> Option<(f32, f32)> {
    let e = |i: usize| edges.get(i).copied().unwrap_or(0.0);
    // the point and the derivative of the Bézier edge starting at index `o`
    let curve = |o: usize, t: f32| {
        let (s, c, d) = (1.0 - t, 2.0 * t * (1.0 - t), t * t);
        let p = (e(o) * s * s + e(o + 2) * c + e(o + 4) * d, e(o + 1) * s * s + e(o + 3) * c + e(o + 5) * d);
        let dp = (2.0 * s * (e(o + 2) - e(o)) + 2.0 * t * (e(o + 4) - e(o + 2)), 2.0 * s * (e(o + 3) - e(o + 1)) + 2.0 * t * (e(o + 5) - e(o + 3)));
        (p, dp)
    };
    let m = |i: usize| inv.get(i).copied().unwrap_or(0.0);
    let den = m(6) * qx + m(7) * qy + m(8);
    let (mut u, mut v) = (0.5f32, 0.5f32);
    if den.abs() > 1e-12 {
        let (su, sv) = ((m(0) * qx + m(1) * qy + m(2)) / den, (m(3) * qx + m(4) * qy + m(5)) / den);
        if su.is_finite() && sv.is_finite() {
            (u, v) = (su.clamp(-1.0, 2.0), sv.clamp(-1.0, 2.0));
        }
    }
    let residual = |u: f32, v: f32| {
        let ((t, dt), (b, db)) = (curve(0, u), curve(6, u));
        let r = (t.0 + v * (b.0 - t.0) - qx, t.1 + v * (b.1 - t.1) - qy);
        // columns of the Jacobian: d/du and d/dv
        let ju = (dt.0 + v * (db.0 - dt.0), dt.1 + v * (db.1 - dt.1));
        let jv = (b.0 - t.0, b.1 - t.1);
        (r, ju, jv)
    };
    for _ in 0..SIX_POINT_STEPS {
        let (r, ju, jv) = residual(u, v);
        let det = ju.0 * jv.1 - jv.0 * ju.1;
        if !det.is_finite() || det.abs() <= 1e-12 {
            break;
        }
        u = (u - (r.0 * jv.1 - jv.0 * r.1) / det).clamp(-1.0, 2.0);
        v = (v - (ju.0 * r.1 - r.0 * ju.1) / det).clamp(-1.0, 2.0);
    }
    let (r, _, _) = residual(u, v);
    let inside = (-1e-3..=1.0 + 1e-3).contains(&u) && (-1e-3..=1.0 + 1e-3).contains(&v);
    (inside && r.0 * r.0 + r.1 * r.1 <= 0.01).then_some((u, v))
}

/// Skin Smoothing's mask: the skin tone's Rec. 601 chroma, how far from it still counts as skin
/// (`tol`), the falloff after that (`soft`), or every pixel (`whole`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkinMask {
    pub cb: f32,
    pub cr: f32,
    pub tol: f32,
    pub soft: f32,
    pub whole: bool,
}

/// Rec. 601 luma of a straight sRGB colour: Skin Smoothing's guide.
#[inline]
pub fn skin_guide(c: [f32; 3]) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

/// Rec. 601 chroma (Cb, Cr) of a straight sRGB colour. Skin sits in a small, well-known patch of
/// this plane whatever the exposure, so the mask is a distance here and ignores brightness.
#[inline]
pub fn skin_chroma(c: [f32; 3]) -> (f32, f32) {
    let y = skin_guide(c);
    (0.564 * (c[2] - y), 0.713 * (c[0] - y))
}

/// How much of a straight sRGB colour is skin, 0..1. `fx.wgsl` (`skin_mask`) mirrors this.
#[inline]
pub fn skin_mask(c: [f32; 3], m: &SkinMask) -> f32 {
    if m.whole {
        return 1.0;
    }
    let (cb, cr) = skin_chroma(c);
    let d = ((cb - m.cb) * (cb - m.cb) + (cr - m.cr) * (cr - m.cr)).sqrt();
    let t = ((d - m.tol) / m.soft.max(1e-4)).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

/// The guided filter's per-pixel model from the box means: `ma = [mean g, mean g², …]` packs the
/// guide's mean and mean square and the input's red and green means, `mb` the input's blue mean
/// and the guide × input means. Returns `(a, b)` per channel, `smoothed = a · g + b`. `fx.wgsl`
/// (`OP_SKIN_COEF_A` / `_B`) mirrors this.
#[inline]
pub fn skin_coeffs(ma: [f32; 4], mb: [f32; 4], eps: f32) -> ([f32; 3], [f32; 3]) {
    let var = (ma[1] - ma[0] * ma[0]).max(0.0);
    let mp = [ma[2], ma[3], mb[0]];
    let mut a = [0.0; 3];
    let mut b = [0.0; 3];
    for k in 0..3 {
        a[k] = (mb[k + 1] - ma[0] * mp[k]) / (var + eps);
        b[k] = mp[k] - a[k] * ma[0];
    }
    (a, b)
}

/// Skin Smoothing's output for straight sRGB `c` (mask `m`) given the guided filter's averaged
/// coefficients: the smoothed colour plus Keep Texture's share of the detail it removed, mixed in
/// by Amount × mask. `fx.wgsl` (`OP_SKIN_APPLY`) mirrors this.
#[inline]
pub fn skin_retouch(c: [f32; 3], m: f32, a: [f32; 3], b: [f32; 3], amount: f32, keep: f32) -> [f32; 3] {
    let k = amount * m;
    let g = skin_guide(c);
    let mut out = [0.0; 3];
    for ch in 0..3 {
        let smoothed = a[ch] * g + b[ch];
        let retouched = smoothed + (c[ch] - smoothed) * keep;
        out[ch] = c[ch] + (retouched - c[ch]) * k;
    }
    out
}

/// Whether Skin Smoothing (not showing its mask) changes anything: some smoothing radius, some
/// Amount, and less than all of the texture kept.
pub fn skin_smoothing_runs(rx: &[u32], ry: &[u32], amount: f32, keep: f32) -> bool {
    (rx.iter().chain(ry).any(|r| *r > 0)) && amount > 0.0 && keep < 1.0
}

/// Variance (display 0..1, squared) below which Low Light's illumination counts as flat and is
/// smoothed by the guided filter; above it an edge (a lamp against a wall) is kept.
pub const LOW_LIGHT_EPS: f32 = 0.01;

/// The illumination estimate of a straight sRGB colour for Low Light: its brightest channel.
#[inline]
pub fn low_light_peak(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]).clamp(0.0, 1.0)
}

/// Low Light's per-pixel model of the self-guided filter from the box means of the peak `m` and
/// its square `m2`: `(a, b)` with `smoothed = a · peak + b`. `fx.wgsl` (`OP_LOW_LIGHT_COEF`)
/// mirrors this.
#[inline]
pub fn low_light_coeffs(m: f32, m2: f32) -> (f32, f32) {
    let var = (m2 - m * m).max(0.0);
    let a = var / (var + LOW_LIGHT_EPS);
    (a, m - a * m)
}

/// Low Light's output for straight sRGB `c` under the smoothed illumination `t`. `fx.wgsl`
/// (`OP_LOW_LIGHT_APPLY`) mirrors this.
#[inline]
pub fn low_light_gain(c: [f32; 3], t: f32, k: f32, max_gain: f32, show: bool) -> [f32; 3] {
    let t = t.clamp(1e-3, 1.0);
    if show {
        return [t, t, t];
    }
    let g = t.powf(-k).min(max_gain);
    [(c[0] * g).min(1.0), (c[1] * g).min(1.0), (c[2] * g).min(1.0)]
}

/// `Image::transformed` without the mip path: the destination rectangle it writes and the inverse.
fn resample(w: usize, h: usize, m: &Affine, opacity: f32) -> Resample {
    let Some(inv) = m.inverse() else { return Resample { inv: None, rect: [0; 4], opacity } };
    let b = m.bounds(&cfa4_geom::Rect::new(0.0, 0.0, w as f64, h as f64));
    let y0 = (b.y.floor().max(0.0) as usize).min(h);
    let y1 = (b.bottom().ceil().max(0.0) as usize).min(h);
    let x0 = (b.x.floor().max(0.0) as usize).min(w);
    let x1 = (b.right().ceil().max(0.0) as usize).min(w);
    Resample { inv: Some(inv), rect: [x0 as u32, x1 as u32, y0.min(y1) as u32, y1 as u32], opacity }
}

/// Whether `Image::transformed` would take its mip (downsample) path for `m` on a `w`×`h` image.
fn minifies(m: &Affine, w: usize, h: usize) -> bool {
    let sx = (m.a * m.a + m.b * m.b).sqrt();
    let sy = (m.c * m.c + m.d * m.d).sqrt();
    let minify = 1.0 / sx.min(sy).max(1e-6);
    // (a non-finite factor: keep such maps off the GPU; `apply` still hands them to the CPU's
    // `Image::transformed`, which decides as before)
    !minify.is_finite() || minify >= 2.0 && w >= 4 && h >= 4
}

impl FxOp {
    /// Evaluate `e` at `cx.t` for a `w`×`h` working image. None for effects without a GPU
    /// implementation (and for disabled effects).
    pub fn eval(e: &EffectInstance, cx: &FxCtx<'_>, w: usize, h: usize) -> Option<FxOp> {
        if !e.enabled {
            return None;
        }
        if let Some(op) = crate::vfx::glitch::eval(e, cx, w, h) {
            return Some(op);
        }
        Some(match e.effect.as_str() {
            "brightness_contrast" => FxOp::BrightnessContrast { br: f(e, "brightness", cx) / 100.0 * 0.4, co: 1.0 + f(e, "contrast", cx) / 100.0 },
            "proc_amp" => FxOp::ProcAmp {
                br: f(e, "brightness", cx) / 100.0 * 0.4,
                co: f(e, "contrast", cx) / 100.0,
                hue: f(e, "hue", cx) / 360.0,
                sat: f(e, "saturation", cx) / 100.0,
            },
            "tint" => {
                let (bl, wh) = (color(e, "black", cx), color(e, "white", cx));
                FxOp::Tint { black: [bl[0], bl[1], bl[2]], white: [wh[0], wh[1], wh[2]], amount: f(e, "amount", cx) / 100.0 }
            }
            "black_white" => FxOp::BlackWhite,
            "color_balance" => {
                let g = |k: &str| f(e, k, cx) / 100.0 * 0.25;
                FxOp::ColorBalance {
                    sh: [g("shadow_r"), g("shadow_g"), g("shadow_b")],
                    md: [g("mid_r"), g("mid_g"), g("mid_b")],
                    hi: [g("hi_r"), g("hi_g"), g("hi_b")],
                    preserve: b(e, "preserve"),
                }
            }
            "leave_color" => {
                let key = color(e, "color", cx);
                FxOp::LeaveColor {
                    amount: f(e, "amount", cx) / 100.0,
                    key_hue: rgb_to_hsl(key[0], key[1], key[2])[0],
                    tol: f(e, "tolerance", cx) / 100.0,
                    soft: f(e, "softness", cx) / 100.0 + 1e-4,
                }
            }
            "change_to_color" => {
                let (from, to) = (color(e, "from", cx), color(e, "to", cx));
                FxOp::ChangeToColor {
                    from_hue: rgb_to_hsl(from[0], from[1], from[2])[0],
                    to_hue: rgb_to_hsl(to[0], to[1], to[2])[0],
                    tol: f(e, "hue_tol", cx) / 100.0,
                    soft: f(e, "softness", cx) / 100.0 * 0.3 + 1e-4,
                }
            }
            "color_pass" => {
                let key = color(e, "color", cx);
                FxOp::ColorPass { key: [key[0], key[1], key[2]], sim: f(e, "similarity", cx) / 100.0, reverse: b(e, "reverse") }
            }
            "gamma_correction" => FxOp::Gamma { g: f(e, "gamma", cx) / 10.0 },
            "levels" => {
                let ib = f(e, "in_black", cx) / 255.0;
                FxOp::Levels {
                    ib,
                    iw: (f(e, "in_white", cx) / 255.0).max(ib + 1e-3),
                    ob: f(e, "out_black", cx) / 255.0,
                    ow: f(e, "out_white", cx) / 255.0,
                    g: 100.0 / f(e, "gamma", cx).max(1.0),
                }
            }
            "extract" => {
                let softness = f(e, "softness", cx);
                FxOp::Extract {
                    lo: f(e, "black", cx) / 255.0,
                    hi: f(e, "white", cx) / 255.0,
                    // Zero softness is a hard cutoff. A tiny substitute fade amplifies
                    // CPU/GPU transfer-function rounding and introduces unwanted grey edges.
                    soft: if softness <= 0.0 { 0.0 } else { softness / 100.0 * 0.2 + 1e-4 },
                    invert: b(e, "invert"),
                }
            }
            "invert" => FxOp::Invert { channel: choice(e, "channel"), blend: f(e, "blend", cx) / 100.0 },
            "posterize" => FxOp::Posterize { n: f(e, "levels", cx).max(2.0) - 1.0 },
            "gaussian_blur" | "gaussian_blur_legacy" => {
                let r = f(e, "blurriness", cx) * cx.px_scale * 0.5;
                let dims = choice(e, "dimensions");
                let (rx, ry) = gaussian_boxes(w, h, if dims == 2 { 0.0 } else { r }, if dims == 1 { 0.0 } else { r });
                FxOp::Gaussian { rx, ry, repeat: b(e, "repeat_edge") }
            }
            "camera_blur" => {
                let r = f(e, "percent", cx) * cx.px_scale * 0.3;
                let (rx, ry) = gaussian_boxes(w, h, r, r);
                FxOp::Gaussian { rx, ry, repeat: true }
            }
            "directional_blur" | "directional_blur_legacy" => {
                let len = f(e, "length", cx) * cx.px_scale * 2.0;
                let dir = (f(e, "direction", cx) as f64).to_radians();
                if len < 0.5 {
                    FxOp::DirectionalBlur { dx: 0.0, dy: 0.0, steps: 0 }
                } else {
                    let steps = (len.ceil() as usize).clamp(2, 64) as u32;
                    FxOp::DirectionalBlur { dx: (dir.sin() as f32) * len, dy: (-dir.cos() as f32) * len, steps }
                }
            }
            "sharpen" => {
                // below a sigma of ~0.58 the three boxes all round to radius 0 and the mask is a
                // no-op, so Sharpen vanished at 1/2 playback resolution and below (#1351)
                let r = cx.px_scale.max(0.6);
                let (rx, ry) = gaussian_boxes(w, h, r, r);
                FxOp::Unsharp { rx, ry, amount: f(e, "amount", cx) / 100.0, threshold: 0.0 }
            }
            "unsharp_mask" => {
                let r = f(e, "radius", cx) * cx.px_scale;
                let (rx, ry) = gaussian_boxes(w, h, r, r);
                FxOp::Unsharp { rx, ry, amount: f(e, "amount", cx) / 100.0, threshold: f(e, "threshold", cx) / 255.0 }
            }
            "crop" => {
                let l = f(e, "left", cx) / 100.0;
                let t = f(e, "top", cx) / 100.0;
                let r = f(e, "right", cx) / 100.0;
                let bt = f(e, "bottom", cx) / 100.0;
                if b(e, "zoom") && l + r < 0.99 && t + bt < 0.99 {
                    let (fw, fh) = (w as f64, h as f64);
                    let m = Affine::scale(1.0 / (1.0 - (l + r) as f64), 1.0 / (1.0 - (t + bt) as f64))
                        .then_apply(&Affine::translate(-(l as f64) * fw, -(t as f64) * fh));
                    if minifies(&m, w, h) {
                        // negative crops zoom out: the CPU's mip path (see `transform`)
                        return Some(FxOp::Resample(Resample { inv: Some(m), rect: [u32::MAX; 4], opacity: 1.0 }));
                    }
                    FxOp::Resample(resample(w, h, &m, 1.0))
                } else {
                    FxOp::crop_rect(w, h, l, t, r, bt, f(e, "feather", cx) * cx.px_scale)
                }
            }
            "edge_feather" => {
                let amt = f(e, "amount", cx) / 100.0 * (w.min(h) as f32) * 0.5;
                FxOp::crop_rect(w, h, 0.0, 0.0, 0.0, 0.0, amt)
            }
            "transform" => {
                let img = Image { w, h, px: Vec::new() };
                let anchor = point(e, "anchor", cx, &img);
                let pos = point(e, "position", cx, &img);
                let sh = f(e, "scale_height", cx) as f64 / 100.0;
                let sw = if b(e, "uniform_scale") { sh } else { f(e, "scale_width", cx) as f64 / 100.0 };
                let rot = f(e, "rotation", cx) as f64;
                let skew = (f(e, "skew", cx) as f64).to_radians().tan();
                let skew_axis = f(e, "skew_axis", cx) as f64;
                let op = f(e, "opacity", cx) / 100.0;
                let sk = Affine::rotate_deg(skew_axis)
                    .then_apply(&Affine { a: 1.0, b: 0.0, c: skew, d: 1.0, e: 0.0, f: 0.0 })
                    .then_apply(&Affine::rotate_deg(-skew_axis));
                let m = Affine::translate(pos.x, pos.y)
                    .then_apply(&Affine::rotate_deg(rot))
                    .then_apply(&sk)
                    .then_apply(&Affine::scale(sw, sh))
                    .then_apply(&Affine::translate(-anchor.x, -anchor.y));
                if minifies(&m, w, h) {
                    // the CPU's mip path (`Image::transformed` with the forward map; `rect`
                    // u32::MAX marks it): CPU only
                    return Some(FxOp::Resample(Resample { inv: Some(m), rect: [u32::MAX; 4], opacity: op }));
                }
                FxOp::Resample(resample(w, h, &m, op))
            }
            "horizontal_flip" => FxOp::HFlip,
            "vertical_flip" => FxOp::VFlip,
            "stereo_swap_eyes" => FxOp::SwapEyes { sbs: choice(e, "frame_layout") == 1 },
            "skin_smoothing" => {
                let r = (f(e, "radius", cx) * cx.px_scale).clamp(0.0, 500.0);
                let (rx, ry) = if r < 0.5 { (Vec::new(), Vec::new()) } else { gaussian_boxes(w, h, r, r) };
                // Smoothness is the luma standard deviation (display 0..1) still counted as flat
                // skin: 2 % at 0 (only sensor noise), 20 % at 100 (everything but a hard edge)
                let sd = 0.02 + 0.18 * (f(e, "smoothness", cx) / 100.0).clamp(0.0, 1.0);
                let tone = color(e, "skin_tone", cx);
                let (cb, cr) = skin_chroma(enc(dec([tone[0], tone[1], tone[2]])));
                let tol = (f(e, "tolerance", cx) / 100.0).clamp(0.0, 1.0) * 0.3;
                FxOp::SkinSmoothing {
                    rx,
                    ry,
                    eps: sd * sd,
                    amount: (f(e, "amount", cx) / 100.0).clamp(0.0, 1.0),
                    keep: (f(e, "texture", cx) / 100.0).clamp(0.0, 1.0),
                    mask: SkinMask { cb, cr, tol, soft: (f(e, "softness", cx) / 100.0).clamp(0.0, 1.0) * 0.3, whole: b(e, "whole_image") },
                    show: b(e, "show_mask"),
                }
            }
            "low_light" => {
                let r = (f(e, "radius", cx) * cx.px_scale).clamp(0.0, 500.0);
                let (rx, ry) = if r < 0.5 { (Vec::new(), Vec::new()) } else { gaussian_boxes(w, h, r, r) };
                FxOp::LowLight {
                    rx,
                    ry,
                    k: (f(e, "amount", cx) / 100.0).clamp(0.0, 1.0) * 0.85,
                    max_gain: (f(e, "max_gain", cx) / 100.0).clamp(1.0, 20.0),
                    show: b(e, "show_illumination"),
                }
            }
            "stereo_to_2d" => FxOp::StereoTo2d { sbs: choice(e, "frame_layout") == 1, right: choice(e, "eye") == 1 },
            "stereo_parallax" => {
                let sbs = choice(e, "frame_layout") == 1;
                // percent of the view width, split between the two eyes
                let pct = f(e, "shift", cx);
                let pct = if pct.is_finite() { pct.clamp(-10.0, 10.0) } else { 0.0 };
                let vw = if sbs { w / 2 } else { w };
                FxOp::StereoParallax { sbs, shift: pct / 100.0 * vw as f32 / 2.0, repeat: b(e, "repeat_edge") }
            }
            "six_point_pin" => {
                use crate::vfx::distort::{h_inverse, square_to_quad};
                let img = Image { w, h, px: Vec::new() };
                let names = ["upper_left", "upper_center", "upper_right", "lower_left", "lower_center", "lower_right"];
                let p = names.map(|n| crate::vfx::pv(e, n, cx, &img));
                let (fw, fh) = (w as f64, h as f64);
                let rest = [(0.0, 0.0), (fw / 2.0, 0.0), (fw, 0.0), (0.0, fh), (fw / 2.0, fh), (fw, fh)];
                let identity = p.iter().zip(rest).all(|(a, (x, y))| (a.x - x).abs() < 1e-6 && (a.y - y).abs() < 1e-6);
                // the quadratic Bézier through start, middle (at t = 1/2) and end
                let ctrl = |a: Vec2, m: Vec2, b: Vec2| Vec2::new(2.0 * m.x - (a.x + b.x) / 2.0, 2.0 * m.y - (a.y + b.y) / 2.0);
                let [ul, uc, ur, ll, lc, lr] = p;
                let pts = [ul, ctrl(ul, uc, ur), ur, ll, ctrl(ll, lc, lr), lr];
                let mut edges = [0f32; 12];
                for (d, v) in edges.chunks_exact_mut(2).zip(pts) {
                    d.copy_from_slice(&[v.x as f32, v.y as f32]);
                }
                let inv = h_inverse(&square_to_quad([ul, ur, lr, ll])).map_or([0.0; 9], |m| m.map(|v| v as f32));
                FxOp::SixPointPin { edges, inv, identity }
            }
            "halftone" => {
                let cell = (f(e, "cell_size", cx).clamp(2.0, 128.0) * cx.px_scale).max(1.0);
                let sharp = (f(e, "sharpness", cx) / 100.0).clamp(0.0, 1.0);
                let sc = |k: &str| {
                    let a = f(e, k, cx) as f64;
                    let a = if a.is_finite() { a.to_radians() } else { 0.0 };
                    [a.cos() as f32, a.sin() as f32]
                };
                let mono = choice(e, "mode") == 1;
                let (ink, paper) = (color(e, "ink", cx), color(e, "paper", cx));
                FxOp::Halftone {
                    mono,
                    lines: choice(e, "pattern") == 1,
                    inv_cell: 1.0 / cell,
                    // plus about a pixel of edge (the spot changes by up to ~π per cell / px), so
                    // the dots stay antialiased
                    soft: (1.0 - sharp) * 0.5 + 3.0 / cell,
                    screens: if mono {
                        let m = sc("mono_angle");
                        [m; 4]
                    } else {
                        [sc("cyan_angle"), sc("magenta_angle"), sc("yellow_angle"), sc("black_angle")]
                    },
                    ink: [ink[0], ink[1], ink[2]],
                    paper: [paper[0], paper[1], paper[2]],
                    blend: (f(e, "blend", cx) / 100.0).clamp(0.0, 1.0),
                }
            }
            "dither" => FxOp::Dither {
                pattern: choice(e, "pattern").min(3),
                // (`as` saturates: NaN → 0, then at least 1)
                block: ((f(e, "pixel_size", cx).clamp(1.0, 16.0) * cx.px_scale).round() as u32).max(1),
                threshold: (f(e, "threshold", cx) / 100.0).clamp(-0.5, 0.5),
                palette: choice(e, "palette").min(DITHER_PALETTES.len() as u32),
                levels: f(e, "levels", cx).round().clamp(2.0, 16.0),
                blend: (f(e, "blend", cx) / 100.0).clamp(0.0, 1.0),
            },
            "crt" => {
                let pct = |k: &str| {
                    let v = f(e, k, cx) / 100.0;
                    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
                };
                let lines = f(e, "line_count", cx);
                let mask = choice(e, "mask").min(3);
                // (`as` saturates: NaN → 0)
                let user = f(e, "seed", cx).round().clamp(0.0, 1e6) as u32;
                let t = crate::effects::noise_frame_seed(cx);
                // 24 bits, exact in f32: [0, 1)
                let n = (pcg3d(user, (t ^ (t >> 32)) as u32, 0x6372_7421) >> 8) as f32 / 16_777_216.0;
                FxOp::Crt {
                    scan: pct("scanlines"),
                    lines: if lines.is_finite() { lines.round().clamp(60.0, 1080.0) } else { 240.0 },
                    mask,
                    mask_amt: if mask == 0 { 0.0 } else { pct("mask_strength") },
                    // a triad is three working pixels at full resolution
                    pitch: (3.0 * cx.px_scale).max(1.0),
                    // up to a quarter: the corners bend in by about a fifth of the frame
                    curve: pct("curvature") * 0.25,
                    corner: pct("corner_darkening"),
                    gain: 1.0 - pct("flicker") * 0.3 * n,
                }
            }
            "oil_paint" => {
                let pct = |k: &str| {
                    let v = f(e, k, cx) / 100.0;
                    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
                };
                let style = choice(e, "style").min(5);
                // (`as` saturates: NaN → 0, then at least 1; 24 px covers a 2× render)
                let radius = ((f(e, "brush_size", cx).clamp(1.0, 12.0) * cx.px_scale).round() as u32).clamp(1, 24);
                FxOp::OilPaint {
                    style,
                    radius,
                    // Knife: always the calmest sector, hard edges
                    q: if style == 4 { 16.0 } else { 1.0 + 15.0 * pct("detail") },
                    texture: pct("texture"),
                    seed: f(e, "seed", cx).round().clamp(0.0, 1e6) as u32,
                }
            }
            "film_grain" => {
                let pct = |k: &str| (f(e, k, cx) / 100.0).clamp(0.0, 2.0);
                // a grain cell under a quarter pixel only aliases; 0 / NaN sizes land here too
                let cell = (f(e, "size", cx).clamp(0.5, 10.0) * cx.px_scale).max(0.25);
                // (`as` saturates: NaN → 0)
                let user = f(e, "seed", cx).round().clamp(0.0, 1e6) as u32;
                let frame = if b(e, "animated") {
                    let t = crate::effects::noise_frame_seed(cx);
                    (t ^ (t >> 32)) as u32
                } else {
                    0
                };
                FxOp::FilmGrain {
                    amount: pct("intensity").min(1.0),
                    inv_cell: 1.0 / cell,
                    shadows: pct("shadows"),
                    midtones: pct("midtones"),
                    highlights: pct("highlights"),
                    color: pct("color").min(1.0),
                    seed: pcg3d(user, frame, 0x6772_6169),
                }
            }
            "mirror" => {
                let img = Image { w, h, px: Vec::new() };
                let c = point(e, "center", cx, &img);
                let ang = (f(e, "angle", cx) as f64).to_radians();
                FxOp::Mirror { cx: c.x as f32, cy: c.y as f32, nx: ang.cos() as f32, ny: ang.sin() as f32 }
            }
            "offset" => {
                let img = Image { w, h, px: Vec::new() };
                let s = point(e, "shift", cx, &img);
                let (dx, dy) = (s.x - w as f64 / 2.0, s.y - h as f64 / 2.0);
                FxOp::Offset { dx: dx.rem_euclid(w.max(1) as f64) as f32, dy: dy.rem_euclid(h.max(1) as f64) as f32, blend: f(e, "blend", cx) / 100.0 }
            }
            "asc_cdl" => {
                use crate::vfx::fv;
                FxOp::AscCdl {
                    slope: [fv(e, "r_slope", cx), fv(e, "g_slope", cx), fv(e, "b_slope", cx)],
                    offset: [fv(e, "r_offset", cx), fv(e, "g_offset", cx), fv(e, "b_offset", cx)],
                    power: [fv(e, "r_power", cx), fv(e, "g_power", cx), fv(e, "b_power", cx)],
                    sat: fv(e, "saturation", cx),
                }
            }
            "channel_mix" => {
                use crate::vfx::{bv, fv};
                let g = |k: &str| fv(e, k, cx) / 100.0;
                let mut m = [[g("rr"), g("rg"), g("rb"), g("rc")], [g("gr"), g("gg"), g("gb"), g("gc")], [g("br"), g("bg"), g("bb"), g("bc")]];
                if bv(e, "monochrome") {
                    m = [m[0]; 3];
                }
                FxOp::ChannelMix { m }
            }
            "color_replace" => {
                use crate::vfx::{bv, cv, fv};
                let t = cv(e, "target", cx);
                let r = cv(e, "replace", cx);
                FxOp::ColorReplace {
                    sim: fv(e, "similarity", cx) / 100.0 * 1.2,
                    solid: bv(e, "solid"),
                    target: [t[0], t[1], t[2]],
                    replace: [r[0], r[1], r[2]],
                    replace_hsl: rgb_to_hsl(r[0], r[1], r[2]),
                }
            }
            "alpha_adjust" => {
                use crate::vfx::{bv, fv};
                FxOp::AlphaAdjust { opacity: fv(e, "opacity", cx) / 100.0, ignore: bv(e, "ignore"), invert: bv(e, "invert"), mask_only: bv(e, "mask_only") }
            }
            "vignette" => {
                use crate::vfx::{cv, fv};
                let amt = fv(e, "amount", cx) / 100.0;
                let mid = fv(e, "midpoint", cx) / 100.0;
                let round = fv(e, "roundness", cx) / 100.0;
                let raw_feather = fv(e, "feather", cx);
                let feather = if raw_feather.is_finite() { (raw_feather / 100.0).max(0.01) } else { raw_feather };
                let col = cv(e, "color", cx);
                let target = if amt < 0.0 { [col[0], col[1], col[2]] } else { [1.0; 3] };
                FxOp::Vignette { amount: amt, midpoint: mid, roundness: round, feather, target }
            }
            "video_limiter" => {
                use crate::vfx::{bv, cv};
                use cfa4_project::ParamValue;
                let clip_level = match e.param("clip_level").map(|p| p.value.clone()).or_else(|| crate::vfx::def_value(e, "clip_level")) {
                    Some(ParamValue::Choice(c)) => c as f32,
                    Some(ParamValue::Float(f)) => f as f32,
                    _ => 0.0,
                };
                let comp_val = match e.param("compression").map(|p| p.value.clone()).or_else(|| crate::vfx::def_value(e, "compression")) {
                    Some(ParamValue::Choice(c)) => [0.0, 0.03, 0.05, 0.10, 0.20][(c as usize).min(4)],
                    Some(ParamValue::Float(f)) => f as f32,
                    _ => 0.03,
                };
                let axis = match e.param("axis").map(|p| p.value.clone()).or_else(|| crate::vfx::def_value(e, "axis")) {
                    Some(ParamValue::Choice(c)) => c,
                    Some(ParamValue::Float(f)) => {
                        if f.is_nan() {
                            u32::MAX
                        } else {
                            f as u32
                        }
                    }
                    _ => 3,
                };
                let warn = bv(e, "gamut_warning");
                let c = cv(e, "warning_color", cx);
                let wc = dec([c[0], c[1], c[2]]);
                FxOp::VideoLimiter { max: 1.0 + clip_level / 100.0, comp: comp_val, axis, warn, warning_color: wc }
            }
            "lumetri" => {
                use crate::effects::{curve_param, is_identity_curve, wheel_rgb};
                let (basic_on, creative_on, vignette_on) = (on(e, "basic_on"), on(e, "creative_on"), on(e, "vignette_on"));
                let gs = grade_space(e, cx, "hdr_white");
                let is_hdr = gs.is_hdr();
                let has_input_lut = basic_on && !text(e, "input_lut").is_empty();
                let has_look_lut = creative_on && !text(e, "look_lut").is_empty();
                let look = if creative_on && !has_look_lut { choice(e, "look") } else { 0 };
                let sharpen = if creative_on { f(e, "sharpen", cx) / 100.0 } else { 0.0 };

                let curves_on = on(e, "curves_on");
                let has_curves = curves_on && {
                    ["curve_luma", "curve_red", "curve_green", "curve_blue"].iter().any(|id| curve_param(e, id).is_some_and(|c| !is_identity_curve(&c)))
                        || ["hue_vs_sat", "hue_vs_hue", "hue_vs_luma", "luma_vs_sat", "sat_vs_sat"]
                            .iter()
                            .any(|id| curve_param(e, id).is_some_and(|c| !c.is_empty()))
                };

                let wheels_on = on(e, "wheels_on");
                let has_wheels = wheels_on && {
                    let v2 = |id: &str| e.param(id).map(|p| p.vec2_at(cx.t)).unwrap_or_default();
                    let (ws, wm, wh) = (wheel_rgb(v2("wheel_shadows")), wheel_rgb(v2("wheel_midtones")), wheel_rgb(v2("wheel_highlights")));
                    let (ls, lm, lh) = (f(e, "wheel_shadows_l", cx) / 100.0, f(e, "wheel_midtones_l", cx) / 100.0, f(e, "wheel_highlights_l", cx) / 100.0);
                    ws.iter().chain(&wm).chain(&wh).any(|v| v.abs() > 1e-5) || (ls.abs() + lm.abs() + lh.abs() > 1e-5)
                };

                let hsl_on = b(e, "hsl_on");

                let gpu_capable = !is_hdr && !has_input_lut && !has_look_lut && look == 0 && sharpen.abs() <= 1e-3 && !has_curves && !has_wheels && !hsl_on;

                let bf = |id: &str| if basic_on { f(e, id, cx) } else { 0.0 };
                let temp = bf("temperature") / 100.0;
                let tint = bf("tint") / 100.0;
                let exposure = 2f32.powf(bf("exposure"));
                let contrast = bf("contrast") / 100.0;
                let hl = bf("highlights") / 100.0;
                let sh = bf("shadows") / 100.0;
                let wh = bf("whites") / 100.0;
                let bl = bf("blacks") / 100.0;
                let sat = if basic_on { f(e, "saturation", cx) / 100.0 } else { 1.0 } * if creative_on { f(e, "creative_sat", cx) / 100.0 } else { 1.0 };
                let vib = if creative_on { f(e, "vibrance", cx) / 100.0 } else { 0.0 };
                let faded = if creative_on { f(e, "faded_film", cx) / 100.0 } else { 0.0 };
                let st = color(e, "shadow_tint", cx);
                let ht = color(e, "highlight_tint", cx);
                let va = if vignette_on { f(e, "vignette_amount", cx) } else { 0.0 };
                let vmid = f(e, "vignette_midpoint", cx) / 100.0;
                let vround = f(e, "vignette_roundness", cx) / 100.0;
                let vfeather = f(e, "vignette_feather", cx) / 100.0;
                let gains = [1.0 + 0.35 * temp, 1.0 - 0.3 * tint, 1.0 - 0.35 * temp];
                let aspect = if h > 0 { (w as f32 / h as f32).max(1e-4) } else { 1.0 };

                FxOp::Lumetri {
                    gains,
                    exposure,
                    contrast,
                    hl,
                    sh,
                    wh,
                    bl,
                    sat,
                    creative_on,
                    faded,
                    vib,
                    st: [st[0], st[1], st[2]],
                    ht: [ht[0], ht[1], ht[2]],
                    vignette_on,
                    va,
                    vmid,
                    vround,
                    vfeather,
                    aspect,
                    gpu_capable,
                }
            }
            _ => return None,
        })
    }

    fn crop_rect(w: usize, h: usize, l: f32, t: f32, r: f32, b: f32, feather: f32) -> FxOp {
        let (w, h) = (w as f32, h as f32);
        FxOp::Crop { x0: l * w, x1: w * (1.0 - r), y0: t * h, y1: h * (1.0 - b), feather: feather.max(0.0) }
    }

    /// Whether this op leaves every pixel as it is, so both the CPU and the GPU skip it (only the
    /// effects whose neutral settings would otherwise still round-trip the colours answer).
    pub fn is_identity(&self) -> bool {
        match self {
            FxOp::Crt { scan, mask_amt, curve, corner, gain, .. } => *scan == 0.0 && *mask_amt == 0.0 && *curve == 0.0 && *corner == 0.0 && *gain == 1.0,
            _ => false,
        }
    }

    /// Whether the GPU shader reproduces [`apply`](Self::apply) for this op: every number finite
    /// (the CPU's NaN / infinity behaviour is left to the CPU) and resamples that the CPU does
    /// not pre-filter.
    pub fn gpu_ok(&self) -> bool {
        let fin = |v: &[f32]| v.iter().all(|x| x.is_finite());
        match self {
            FxOp::BrightnessContrast { br, co } => fin(&[*br, *co]),
            FxOp::ProcAmp { br, co, hue, sat } => fin(&[*br, *co, *hue, *sat]),
            FxOp::Tint { black, white, amount } => fin(black) && fin(white) && amount.is_finite(),
            FxOp::BlackWhite | FxOp::HFlip | FxOp::VFlip | FxOp::SwapEyes { .. } | FxOp::StereoTo2d { .. } => true,
            FxOp::StereoParallax { shift, .. } => shift.is_finite(),
            // f32 pixel positions keep sub-pixel precision far beyond any frame size
            FxOp::SixPointPin { edges, inv, .. } => edges.iter().all(|v| v.is_finite() && v.abs() < 1e6) && fin(inv),
            FxOp::ColorBalance { sh, md, hi, .. } => fin(sh) && fin(md) && fin(hi),
            FxOp::LeaveColor { amount, key_hue, tol, soft } => fin(&[*amount, *key_hue, *tol, *soft]),
            FxOp::ChangeToColor { from_hue, to_hue, tol, soft } => fin(&[*from_hue, *to_hue, *tol, *soft]),
            FxOp::ColorPass { key, sim, .. } => fin(key) && sim.is_finite(),
            FxOp::Gamma { g } => g.is_finite(),
            FxOp::Levels { ib, iw, ob, ow, g } => fin(&[*ib, *iw, *ob, *ow, *g]),
            FxOp::Extract { lo, hi, soft, .. } => fin(&[*lo, *hi, *soft]),
            FxOp::Invert { blend, .. } => blend.is_finite(),
            FxOp::Posterize { n } => n.is_finite(),
            FxOp::Gaussian { .. } => true,
            FxOp::DirectionalBlur { dx, dy, .. } => fin(&[*dx, *dy]),
            FxOp::Unsharp { amount, threshold, .. } => fin(&[*amount, *threshold]),
            FxOp::LowLight { k, max_gain, .. } => fin(&[*k, *max_gain]),
            FxOp::SkinSmoothing { eps, amount, keep, mask, .. } => fin(&[*eps, *amount, *keep, mask.cb, mask.cr, mask.tol, mask.soft]),
            FxOp::Crop { x0, x1, y0, y1, feather } => fin(&[*x0, *x1, *y0, *y1, *feather]),
            FxOp::Resample(r) => {
                r.rect[0] != u32::MAX
                    && r.opacity.is_finite()
                    && r.inv.as_ref().is_none_or(|m| [m.a, m.b, m.c, m.d, m.e, m.f].iter().all(|v| v.is_finite() && v.abs() < 1e7))
            }
            FxOp::Mirror { cx, cy, nx, ny } => fin(&[*cx, *cy, *nx, *ny]),
            FxOp::Offset { dx, dy, blend } => fin(&[*dx, *dy, *blend]),
            FxOp::AscCdl { slope, offset, power, sat } => fin(slope) && fin(offset) && fin(power) && sat.is_finite(),
            FxOp::ChannelMix { m } => m.iter().all(|r| fin(r)),
            FxOp::ColorReplace { sim, target, replace, replace_hsl, .. } => sim.is_finite() && fin(target) && fin(replace) && fin(replace_hsl),
            FxOp::AlphaAdjust { opacity, .. } => opacity.is_finite(),
            FxOp::Vignette { amount, midpoint, roundness, feather, target } => fin(&[*amount, *midpoint, *roundness, *feather]) && fin(target),
            FxOp::Halftone { inv_cell, soft, screens, ink, paper, blend, .. } => {
                fin(&[*inv_cell, *soft, *blend]) && screens.iter().all(|s| fin(s)) && fin(ink) && fin(paper)
            }
            FxOp::Dither { threshold, levels, blend, .. } => fin(&[*threshold, *levels, *blend]),
            FxOp::BitCrush { top, cell, .. } => fin(&[*top, *cell]),
            FxOp::BlockGlitch { .. } => true,
            FxOp::Databend { shift, color, .. } => fin(&[*shift, *color]),
            FxOp::Vhs(v) => fin(&[v.alpha, v.wobble, v.inv_len, v.phase, v.snow, v.dim]),
            FxOp::ScanLineJitter { disp, thr, .. } => fin(&[*disp, *thr]),
            FxOp::TileJitter { dx, dy, tiles, .. } => fin(&[*dx, *dy, *tiles]),
            FxOp::Crt { scan, lines, mask_amt, pitch, curve, corner, gain, .. } => fin(&[*scan, *lines, *mask_amt, *pitch, *curve, *corner, *gain]),
            FxOp::OilPaint { q, texture, .. } => fin(&[*q, *texture]),
            FxOp::FilmGrain { amount, inv_cell, shadows, midtones, highlights, color, .. } => {
                fin(&[*amount, *inv_cell, *shadows, *midtones, *highlights, *color])
            }
            FxOp::VideoLimiter { max, comp, axis, warning_color, .. } => *axis <= 3 && max.is_finite() && comp.is_finite() && fin(warning_color),
            FxOp::Lumetri { gains, exposure, contrast, hl, sh, wh, bl, sat, faded, vib, st, ht, va, vmid, vround, vfeather, aspect, gpu_capable, .. } => {
                *gpu_capable
                    && fin(gains)
                    && fin(&[*exposure, *contrast, *hl, *sh, *wh, *bl, *sat, *faded, *vib])
                    && fin(st)
                    && fin(ht)
                    && fin(&[*va, *vmid, *vround, *vfeather, *aspect])
            }
        }
    }

    /// The CPU reference.
    pub fn apply(&self, img: &mut Image) {
        if img.w == 0 || img.h == 0 {
            return;
        }
        use crate::vfx::glitch;
        match self {
            FxOp::BitCrush { top, dither, cell } => glitch::bit_crush(img, *top, *dither, *cell),
            FxOp::BlockGlitch { count, tw, th, maxd, snap, swap, rng } => glitch::block_glitch(img, *count, *tw, *th, *maxd, *snap, *swap, *rng),
            FxOp::Databend { rng, breaks, shift, color, block } => glitch::databend(img, *rng, *breaks, *shift, *color, *block),
            FxOp::Vhs(v) => glitch::vhs(img, v),
            FxOp::ScanLineJitter { vertical, disp, thr, seed, tick } => glitch::scan_line_jitter(img, *vertical, *disp, *thr, *seed, *tick),
            FxOp::TileJitter { dx, dy, tiles, across } => glitch::tile_jitter(img, *dx, *dy, *tiles, *across),
            FxOp::BrightnessContrast { br, co } => {
                let (br, co) = (*br, *co);
                img.map_rgb(|c, _, _| {
                    let c = enc(c);
                    dec(c.map(|v| (v - 0.5) * co + 0.5 + br))
                });
            }
            FxOp::ProcAmp { br, co, hue, sat } => {
                let (br, co, hue, sat) = (*br, *co, *hue, *sat);
                img.map_rgb(|c, _, _| {
                    let c = enc(c);
                    let mut hsl = rgb_to_hsl(c[0], c[1], c[2]);
                    hsl[0] = (hsl[0] + hue).rem_euclid(1.0);
                    hsl[1] = (hsl[1] * sat).clamp(0.0, 1.0);
                    let c = hsl_to_rgb(hsl[0], hsl[1], hsl[2]);
                    dec(c.map(|v| (v - 0.5) * co + 0.5 + br))
                });
            }
            FxOp::Tint { black: bl, white: wh, amount } => {
                let amt = *amount;
                img.map_rgb(|c, _, _| {
                    let l = linear_to_srgb(luma709(c[0], c[1], c[2]).max(0.0));
                    let t = [bl[0] + (wh[0] - bl[0]) * l, bl[1] + (wh[1] - bl[1]) * l, bl[2] + (wh[2] - bl[2]) * l];
                    let e = enc(c);
                    dec([e[0] + (t[0] - e[0]) * amt, e[1] + (t[1] - e[1]) * amt, e[2] + (t[2] - e[2]) * amt])
                });
            }
            FxOp::BlackWhite => img.map_rgb(|c, _, _| {
                let l = luma709(c[0], c[1], c[2]);
                [l, l, l]
            }),
            FxOp::ColorBalance { sh, md, hi, preserve } => {
                let preserve = *preserve;
                img.map_rgb(|c, _, _| {
                    let c = enc(c);
                    let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
                    let ws = (1.0 - l).powi(2);
                    let wh = l.powi(2);
                    let wm = 1.0 - ws - wh;
                    let mut o = [0.0; 3];
                    for k in 0..3 {
                        o[k] = c[k] + sh[k] * ws + md[k] * wm.max(0.0) + hi[k] * wh;
                    }
                    if preserve {
                        let l2 = 0.2126 * o[0] + 0.7152 * o[1] + 0.0722 * o[2];
                        let d = l - l2;
                        o = o.map(|v| v + d);
                    }
                    dec(o)
                });
            }
            FxOp::LeaveColor { amount, key_hue, tol, soft } => {
                let (amt, kh, tol, soft) = (*amount, *key_hue, *tol, *soft);
                img.map_rgb(|c, _, _| {
                    let ec = enc(c);
                    let h = rgb_to_hsl(ec[0], ec[1], ec[2])[0];
                    let d = (h - kh).abs().min(1.0 - (h - kh).abs()) * 2.0;
                    let keep = 1.0 - ((d - tol) / soft).clamp(0.0, 1.0);
                    let l = luma709(c[0], c[1], c[2]);
                    let k = amt * (1.0 - keep);
                    [c[0] + (l - c[0]) * k, c[1] + (l - c[1]) * k, c[2] + (l - c[2]) * k]
                });
            }
            FxOp::ChangeToColor { from_hue: fh, to_hue: th, tol, soft } => {
                let (fh, th, tol, soft) = (*fh, *th, *tol, *soft);
                img.map_rgb(|c, _, _| {
                    let ec = enc(c);
                    let mut hsl = rgb_to_hsl(ec[0], ec[1], ec[2]);
                    let d = (hsl[0] - fh).abs().min(1.0 - (hsl[0] - fh).abs());
                    let w = 1.0 - ((d - tol) / soft).clamp(0.0, 1.0);
                    hsl[0] = (hsl[0] + (th - fh) * w).rem_euclid(1.0);
                    dec(hsl_to_rgb(hsl[0], hsl[1], hsl[2]))
                });
            }
            FxOp::ColorPass { key, sim, reverse } => {
                let (sim, rev) = (*sim, *reverse);
                img.map_rgb(|c, _, _| {
                    let ec = enc(c);
                    let d = ((ec[0] - key[0]).powi(2) + (ec[1] - key[1]).powi(2) + (ec[2] - key[2]).powi(2)).sqrt();
                    let pass = (d <= sim * 1.2) != rev;
                    if pass {
                        c
                    } else {
                        let l = luma709(c[0], c[1], c[2]);
                        [l, l, l]
                    }
                });
            }
            FxOp::Gamma { g } => {
                let g = *g;
                img.map_rgb(|c, _, _| dec(enc(c).map(|v| v.max(0.0).powf(g))));
            }
            FxOp::Levels { ib, iw, ob, ow, g } => {
                let (ib, iw, ob, ow, g) = (*ib, *iw, *ob, *ow, *g);
                img.map_rgb(|c, _, _| dec(enc(c).map(|v| ob + (((v - ib) / (iw - ib)).clamp(0.0, 1.0)).powf(g) * (ow - ob))));
            }
            FxOp::Extract { lo, hi, soft, invert } => {
                let (lo, hi, soft, inv) = (*lo, *hi, *soft, *invert);
                img.map_rgb(|c, _, _| {
                    let l = linear_to_srgb(luma709(c[0], c[1], c[2]).max(0.0));
                    let inside = if soft <= 0.0 {
                        // Match the feathered function's limiting value at the cutoffs.
                        if l > lo && l < hi { 1.0 } else { 0.0 }
                    } else {
                        ((l - lo) / soft).clamp(0.0, 1.0).min(((hi - l) / soft).clamp(0.0, 1.0))
                    };
                    let v = if inv { 1.0 - inside } else { inside };
                    [v, v, v]
                });
            }
            FxOp::Invert { channel, blend } => {
                let (ch, blend) = (*channel, *blend);
                if ch == 4 {
                    img.px.par_chunks_mut(4).for_each(|p| {
                        let a = p[3];
                        let na = 1.0 - a;
                        let k = if a > 1e-6 { na / a } else { 0.0 };
                        for c in &mut p[..3] {
                            *c *= k;
                        }
                        p[3] = na * (1.0 - blend) + a * blend;
                    });
                } else {
                    img.map_rgb(|c, _, _| {
                        let ec = enc(c);
                        let mut o = ec;
                        for k in 0..3 {
                            if ch == 0 || ch as usize == k + 1 {
                                o[k] = 1.0 - ec[k];
                            }
                        }
                        dec([o[0] + (ec[0] - o[0]) * blend, o[1] + (ec[1] - o[1]) * blend, o[2] + (ec[2] - o[2]) * blend])
                    });
                }
            }
            FxOp::Posterize { n } => {
                let n = *n;
                img.map_rgb(|c, _, _| dec(enc(c).map(|v| (v * n).round() / n)));
            }
            FxOp::Gaussian { rx, ry, repeat } => crate::effects::box_blur(img, rx, ry, *repeat),
            FxOp::DirectionalBlur { dx, dy, steps } => crate::effects::directional_taps(img, *dx, *dy, *steps as usize),
            FxOp::Unsharp { rx, ry, amount, threshold } => crate::effects::unsharp_boxes(img, rx, ry, *amount, *threshold),
            FxOp::SkinSmoothing { rx, ry, eps, amount, keep, mask, show } => {
                let (amount, keep, eps, mask) = (*amount, *keep, *eps, *mask);
                let straight = |p: &[f32]| enc(Image::unpremul([p[0], p[1], p[2], p[3]]));
                if *show {
                    img.px.par_chunks_mut(4).for_each(|p| {
                        let m = skin_mask(straight(p), &mask);
                        let l = dec([m, m, m]);
                        for ch in 0..3 {
                            p[ch] = l[ch] * p[3];
                        }
                    });
                    return;
                }
                if !skin_smoothing_runs(rx, ry, amount, keep) {
                    return;
                }
                // the guide's and the input's statistics in two planes, box-blurred; then the
                // model's coefficients in two more, box-blurred
                let (mut sa, mut sb) = (img.clone(), img.clone());
                sa.px.par_chunks_mut(4).zip(sb.px.par_chunks_mut(4)).for_each(|(a, b)| {
                    let c = straight(a);
                    let g = skin_guide(c);
                    a.copy_from_slice(&[g, g * g, c[0], c[1]]);
                    b.copy_from_slice(&[c[2], g * c[0], g * c[1], g * c[2]]);
                });
                crate::effects::box_blur(&mut sa, rx, ry, true);
                crate::effects::box_blur(&mut sb, rx, ry, true);
                sa.px.par_chunks_mut(4).zip(sb.px.par_chunks_mut(4)).for_each(|(a, b)| {
                    let (ca, cb) = skin_coeffs([a[0], a[1], a[2], a[3]], [b[0], b[1], b[2], b[3]], eps);
                    a.copy_from_slice(&[ca[0], ca[1], ca[2], 0.0]);
                    b.copy_from_slice(&[cb[0], cb[1], cb[2], 0.0]);
                });
                crate::effects::box_blur(&mut sa, rx, ry, true);
                crate::effects::box_blur(&mut sb, rx, ry, true);
                img.px.par_chunks_mut(4).zip(sa.px.par_chunks(4).zip(sb.px.par_chunks(4))).for_each(|(p, (a, b))| {
                    let c = straight(p);
                    let m = skin_mask(c, &mask);
                    if amount * m <= 0.0 {
                        return;
                    }
                    let l = dec(skin_retouch(c, m, [a[0], a[1], a[2]], [b[0], b[1], b[2]], amount, keep));
                    for ch in 0..3 {
                        p[ch] = l[ch] * p[3];
                    }
                });
            }
            FxOp::LowLight { rx, ry, k, max_gain, show } => {
                if *k <= 0.0 && !*show {
                    return;
                }
                // the peak and its square, box-blurred; then the model's (a, b), box-blurred
                let mut stats = img.clone();
                stats.px.par_chunks_mut(4).for_each(|p| {
                    let t = low_light_peak(enc(Image::unpremul([p[0], p[1], p[2], p[3]])));
                    p.copy_from_slice(&[t, t * t, 0.0, 0.0]);
                });
                crate::effects::box_blur(&mut stats, rx, ry, true);
                stats.px.par_chunks_mut(4).for_each(|p| {
                    let (a, b) = low_light_coeffs(p[0], p[1]);
                    p.copy_from_slice(&[a, b, 0.0, 0.0]);
                });
                crate::effects::box_blur(&mut stats, rx, ry, true);
                let (k, max_gain, show) = (*k, *max_gain, *show);
                img.px.par_chunks_mut(4).zip(stats.px.par_chunks(4)).for_each(|(p, ab)| {
                    let c = enc(Image::unpremul([p[0], p[1], p[2], p[3]]));
                    let t = ab[0] * low_light_peak(c) + ab[1];
                    let l = dec(low_light_gain(c, t, k, max_gain, show));
                    for ch in 0..3 {
                        p[ch] = l[ch] * p[3];
                    }
                });
            }
            FxOp::Crop { x0, x1, y0, y1, feather } => crate::effects::crop_px(img, *x0, *x1, *y0, *y1, *feather),
            FxOp::Resample(r) => {
                let out = match (&r.inv, r.rect[0]) {
                    (Some(m), u32::MAX) => img.transformed(img.w, img.h, m), // mip path (CPU only)
                    (Some(inv), _) => resample_cpu(img, inv, r.rect),
                    (None, _) => Image::new(img.w, img.h),
                };
                *img = out;
                img.scale_alpha(r.opacity);
            }
            FxOp::HFlip => {
                let w = img.w;
                img.px.par_chunks_mut(w * 4).for_each(|row| {
                    for x in 0..w / 2 {
                        for k in 0..4 {
                            row.swap(x * 4 + k, (w - 1 - x) * 4 + k);
                        }
                    }
                });
            }
            FxOp::VFlip => {
                let (w, h) = (img.w, img.h);
                for y in 0..h / 2 {
                    let (a, bb) = img.px.split_at_mut((h - 1 - y) * w * 4);
                    a[y * w * 4..(y + 1) * w * 4].swap_with_slice(&mut bb[..w * 4]);
                }
            }
            FxOp::SwapEyes { sbs } => {
                let (sbs, src) = (*sbs, img.clone());
                let (w, h) = (img.w, img.h);
                img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
                    for x in 0..w {
                        let (sx, sy) = swap_eyes_src(x, y, w, h, sbs);
                        let s = (sy * w + sx) * 4;
                        if let (Some(d), Some(v)) = (row.get_mut(x * 4..x * 4 + 4), src.px.get(s..s + 4)) {
                            d.copy_from_slice(v);
                        }
                    }
                });
            }
            FxOp::StereoTo2d { sbs, right } => {
                let (sbs, right) = (*sbs, *right);
                let (w, h) = (img.w, img.h);
                if (if sbs { w } else { h }) < 2 {
                    return;
                }
                let src = img.clone();
                img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
                    for x in 0..w {
                        if let Some(d) = row.get_mut(x * 4..x * 4 + 4) {
                            d.copy_from_slice(&stereo_view_sample(&src, x, y, sbs, right));
                        }
                    }
                });
            }
            FxOp::StereoParallax { sbs, shift, repeat } => {
                let (sbs, shift, repeat) = (*sbs, *shift, *repeat);
                let src = img.clone();
                let w = img.w;
                img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
                    for x in 0..w {
                        if let Some(d) = row.get_mut(x * 4..x * 4 + 4) {
                            d.copy_from_slice(&stereo_parallax_sample(&src, x, y, sbs, shift, repeat));
                        }
                    }
                });
            }
            FxOp::SixPointPin { edges, inv, identity } => {
                if *identity {
                    return;
                }
                let src = img.clone();
                let (w, fw, fh) = (img.w, img.w as f32, img.h as f32);
                img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
                    for x in 0..w {
                        let px = match six_point_source(edges, inv, x as f32 + 0.5, y as f32 + 0.5) {
                            Some((u, v)) => src.sample_bilinear(u * fw, v * fh),
                            None => [0.0; 4],
                        };
                        if let Some(d) = row.get_mut(x * 4..x * 4 + 4) {
                            d.copy_from_slice(&px);
                        }
                    }
                });
            }
            FxOp::Mirror { cx, cy, nx, ny } => {
                let (c, n) = ((*cx as f64, *cy as f64), (*nx as f64, *ny as f64));
                let src = img.clone();
                let w = img.w;
                img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
                    for x in 0..w {
                        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                        let d = (px - c.0) * n.0 + (py - c.1) * n.1;
                        if d > 0.0 {
                            let (rx, ry) = (px - 2.0 * d * n.0, py - 2.0 * d * n.1);
                            row[x * 4..x * 4 + 4].copy_from_slice(&src.sample_bilinear(rx as f32, ry as f32));
                        }
                    }
                });
            }
            FxOp::Offset { dx, dy, blend } => {
                let (dx, dy, blend) = (*dx as f64, *dy as f64, *blend);
                let src = img.clone();
                let (w, h) = (img.w as f64, img.h as f64);
                let wi = img.w;
                img.px.par_chunks_mut(wi * 4).enumerate().for_each(|(y, row)| {
                    for x in 0..wi {
                        let u = (x as f64 + 0.5 - dx).rem_euclid(w);
                        let v = (y as f64 + 0.5 - dy).rem_euclid(h);
                        let p = src.sample_bilinear_clamped(u as f32, v as f32);
                        let o = src.get(x, y);
                        for k in 0..4 {
                            row[x * 4 + k] = p[k] + (o[k] - p[k]) * blend;
                        }
                    }
                });
            }
            FxOp::AscCdl { slope, offset, power, sat } => {
                let (s, o, pw, sat) = (*slope, *offset, *power, *sat);
                img.map_rgb(|c, _, _| dec(crate::vfx::cdl(enc(c), s, o, pw, sat)));
            }
            FxOp::ChannelMix { m } => {
                let m = *m;
                img.map_rgb(|c, _, _| {
                    let v = enc(c);
                    dec([0, 1, 2].map(|k| (m[k][0] * v[0] + m[k][1] * v[1] + m[k][2] * v[2] + m[k][3]).clamp(0.0, 1.0)))
                });
            }
            FxOp::ColorReplace { sim, solid, target: t, replace: r, replace_hsl: rh } => {
                let (sim, solid) = (*sim, *solid);
                img.map_rgb(|c, _, _| {
                    let v = enc(c);
                    let d = ((v[0] - t[0]).powi(2) + (v[1] - t[1]).powi(2) + (v[2] - t[2]).powi(2)).sqrt();
                    let k = 1.0 - crate::vfx::smoothstep(sim * 0.85, sim.max(1e-4), d);
                    if k <= 0.0 {
                        return c;
                    }
                    let target = if solid {
                        [r[0], r[1], r[2]]
                    } else {
                        let l = rgb_to_hsl(v[0], v[1], v[2])[2];
                        hsl_to_rgb(rh[0], rh[1], l)
                    };
                    dec(crate::vfx::lerp3(v, target, k))
                });
            }
            FxOp::AlphaAdjust { opacity, ignore, invert, mask_only } => {
                let (op, ignore, invert, mask_only) = (*opacity, *ignore, *invert, *mask_only);
                img.px.par_chunks_mut(4).for_each(|p| {
                    let c = Image::unpremul([p[0], p[1], p[2], p[3]]);
                    let mut a = if ignore { 1.0 } else { p[3] };
                    if invert {
                        a = 1.0 - a;
                    }
                    a = (a * op).clamp(0.0, 1.0);
                    if mask_only {
                        let g = cfa4_color::srgb_to_linear(a);
                        p.copy_from_slice(&[g, g, g, 1.0]);
                    } else {
                        p.copy_from_slice(&[c[0] * a, c[1] * a, c[2] * a, a]);
                    }
                });
            }
            FxOp::Halftone { mono, lines, inv_cell, soft, screens, ink, paper, blend } => {
                let (mono, lines, inv, soft, sc, ink, paper, blend) = (*mono, *lines, *inv_cell, *soft, *screens, *ink, *paper, *blend);
                if ![inv, soft, blend].iter().all(|v| v.is_finite()) || soft <= 0.0 {
                    return;
                }
                img.map_rgb(|c, x, y| {
                    let e = enc(c);
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let cov = |s: [f32; 2], v: f32| halftone_ink(px, py, s[0], s[1], inv, soft, lines, v);
                    let out = if mono {
                        let k = cov(sc[3], 1.0 - (0.2126 * e[0] + 0.7152 * e[1] + 0.0722 * e[2]));
                        [paper[0] + (ink[0] - paper[0]) * k, paper[1] + (ink[1] - paper[1]) * k, paper[2] + (ink[2] - paper[2]) * k]
                    } else {
                        // CMYK with full under-colour removal: neutral tones print with black only
                        let e = e.map(|v| v.clamp(0.0, 1.0));
                        let k = 1.0 - e[0].max(e[1]).max(e[2]);
                        let ch = |v: f32| if k < 1.0 { (1.0 - v - k) / (1.0 - k) } else { 0.0 };
                        let kk = cov(sc[3], k);
                        let ik = 1.0 - kk;
                        [(1.0 - cov(sc[0], ch(e[0]))) * ik, (1.0 - cov(sc[1], ch(e[1]))) * ik, (1.0 - cov(sc[2], ch(e[2]))) * ik]
                    };
                    dec([out[0] + (e[0] - out[0]) * blend, out[1] + (e[1] - out[1]) * blend, out[2] + (e[2] - out[2]) * blend])
                });
            }
            FxOp::Dither { pattern, block, threshold, palette, levels, blend } => {
                let (pattern, block, t, palette, levels, blend) = (*pattern, (*block).max(1) as usize, *threshold, *palette, *levels, *blend);
                if ![t, levels, blend].iter().all(|v| v.is_finite()) || levels < 2.0 {
                    return;
                }
                let src = img.clone();
                let (w, h) = (img.w, img.h);
                img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
                    for x in 0..w {
                        let Some(p) = row.get_mut(x * 4..x * 4 + 4) else { continue };
                        // every pixel of a dither block takes the block's centre pixel
                        let (bx, by) = (x / block, y / block);
                        let sx = (bx * block + block / 2).min(w - 1);
                        let sy = (by * block + block / 2).min(h - 1);
                        let s = (sy * w + sx) * 4;
                        let Some(q) = src.px.get(s..s + 4) else { continue };
                        // the dithered block sample, mixed with this pixel's own original
                        let (ab, ao) = (q[3], p[3]);
                        let dithered = (ab > 1e-6).then(|| {
                            let e = enc([q[0] / ab, q[1] / ab, q[2] / ab]);
                            dither_px(e, dither_threshold(bx as u32, by as u32, pattern) + t, palette, levels)
                        });
                        let original = (ao > 1e-6).then(|| enc([p[0] / ao, p[1] / ao, p[2] / ao]));
                        let c = match (dithered, original) {
                            (Some(o), Some(e)) => [o[0] + (e[0] - o[0]) * blend, o[1] + (e[1] - o[1]) * blend, o[2] + (e[2] - o[2]) * blend],
                            (Some(c), None) | (None, Some(c)) => c,
                            (None, None) => [0.0; 3],
                        };
                        let a = ab + (ao - ab) * blend;
                        if a <= 1e-6 {
                            p.copy_from_slice(&[0.0; 4]);
                            continue;
                        }
                        let c = dec(c);
                        p.copy_from_slice(&[c[0] * a, c[1] * a, c[2] * a, a]);
                    }
                });
            }
            FxOp::Crt { scan, lines, mask, mask_amt, pitch, curve, corner, gain } => {
                let (scan, lines, mask, amt, pitch, curve, corner, gain) = (*scan, *lines, *mask, *mask_amt, *pitch, *curve, *corner, *gain);
                if ![scan, lines, amt, pitch, curve, corner, gain].iter().all(|v| v.is_finite()) || self.is_identity() {
                    return;
                }
                let src = img.clone();
                let (w, fw, fh) = (img.w, img.w as f32, img.h as f32);
                img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
                    for x in 0..w {
                        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                        let Some(d) = row.get_mut(x * 4..x * 4 + 4) else { continue };
                        let Some((cu, cv)) = crt_warp(px, py, fw, fh, curve) else {
                            d.copy_from_slice(&[0.0; 4]);
                            continue;
                        };
                        // a flat tube shows each pixel as it is
                        let q = if curve == 0.0 { src.get(x, y) } else { src.sample_bilinear_clamped((cu + 1.0) * 0.5 * fw, (cv + 1.0) * 0.5 * fh) };
                        let a = q[3];
                        if a <= 1e-6 {
                            d.copy_from_slice(&q);
                            continue;
                        }
                        let e = enc([q[0] / a, q[1] / a, q[2] / a]);
                        let k = crt_shade(cu, cv, px, py, scan, lines, mask, amt, pitch, corner);
                        let c = dec([e[0] * k[0] * gain, e[1] * k[1] * gain, e[2] * k[2] * gain]);
                        d.copy_from_slice(&[c[0] * a, c[1] * a, c[2] * a, a]);
                    }
                });
            }
            FxOp::OilPaint { style, radius, q, texture, seed } => {
                let (style, r, q, tex, seed) = (*style, (*radius).clamp(1, 24) as i32, *q, *texture, *seed);
                if !q.is_finite() || !tex.is_finite() {
                    return;
                }
                let n = if style == 0 || style == 4 { 4 } else { 8 };
                let taps: Vec<(isize, isize, u32)> =
                    (-r..=r).flat_map(|dy| (-r..=r).map(move |dx| (dx as isize, dy as isize, oil_sectors(style, dx, dy, r)))).filter(|t| t.2 != 0).collect();
                let src = img.clone();
                let w = img.w;
                img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
                    for x in 0..w {
                        let mut sum = [[0.0f32; 4]; 8];
                        let mut sq = [[0.0f32; 3]; 8];
                        let mut cnt = [0.0f32; 8];
                        for &(dx, dy, m) in &taps {
                            let c = src.get_clamped(x as isize + dx, y as isize + dy);
                            for k in 0..n {
                                if (m >> k) & 1 == 1 {
                                    for i in 0..4 {
                                        sum[k][i] += c[i];
                                    }
                                    for i in 0..3 {
                                        sq[k][i] += c[i] * c[i];
                                    }
                                    cnt[k] += 1.0;
                                }
                            }
                        }
                        // each sector's mean and spread (the summed variance of its colour)
                        let mut mean = [[0.0f32; 4]; 8];
                        let mut sd = [0.0f32; 8];
                        let mut lo = f32::INFINITY;
                        for k in 0..n {
                            let c = cnt[k].max(1.0);
                            mean[k] = sum[k].map(|v| v / c);
                            let var = (0..3).map(|i| sq[k][i] / c - mean[k][i] * mean[k][i]).sum::<f32>();
                            sd[k] = var.max(0.0).sqrt();
                            lo = lo.min(sd[k]);
                        }
                        let mut o = [0.0f32; 4];
                        let mut wsum = 0.0f32;
                        for k in 0..n {
                            let wk = 1.0 / (1.0 + (sd[k] / (lo + 1e-4)).powf(q));
                            for i in 0..4 {
                                o[i] += mean[k][i] * wk;
                            }
                            wsum += wk;
                        }
                        let o = o.map(|v| v / wsum.max(1e-12));
                        let Some(d) = row.get_mut(x * 4..x * 4 + 4) else { continue };
                        let a = o[3];
                        if a <= 1e-6 {
                            d.copy_from_slice(&[0.0; 4]);
                            continue;
                        }
                        let t = 1.0 + tex * oil_texture(style, x, y, r as u32, seed);
                        let e = enc([o[0] / a, o[1] / a, o[2] / a]);
                        let c = dec([e[0] * t, e[1] * t, e[2] * t]);
                        d.copy_from_slice(&[c[0] * a, c[1] * a, c[2] * a, a]);
                    }
                });
            }
            FxOp::FilmGrain { amount, inv_cell, shadows, midtones, highlights, color, seed } => {
                let (amt, inv, sh, md, hi, col, seed) = (*amount, *inv_cell, *shadows, *midtones, *highlights, *color, *seed);
                if !amt.is_finite() || amt <= 0.0 || ![inv, sh, md, hi, col].iter().all(|v| v.is_finite()) {
                    return;
                }
                img.map_rgb(|c, x, y| {
                    let e = enc(c);
                    let l = (0.2126 * e[0] + 0.7152 * e[1] + 0.0722 * e[2]).clamp(0.0, 1.0);
                    let m = 1.0 - l;
                    let k = amt * (sh * m * m + md * 2.0 * l * m + hi * l * l);
                    let n0 = grain_noise(x, y, inv, seed, 0);
                    let mut n = [n0; 3];
                    if col > 0.0 {
                        for (i, v) in n.iter_mut().enumerate() {
                            *v = n0 + (grain_noise(x, y, inv, seed, i as u32 + 1) - n0) * col;
                        }
                    }
                    dec([e[0] + n[0] * k, e[1] + n[1] * k, e[2] + n[2] * k])
                });
            }
            FxOp::Vignette { amount, midpoint, roundness, feather, target } => {
                let (amt, mid, round, feather, target) = (*amount, *midpoint, *roundness, *feather, *target);
                if !amt.is_finite() || !mid.is_finite() || !round.is_finite() || !feather.is_finite() || !target.iter().all(|c| c.is_finite()) {
                    return;
                }
                if amt.abs() < 1e-5 {
                    return;
                }
                let (w, h) = (img.w as f32, img.h as f32);
                let aspect = w / h;
                img.map_rgb(|c, x, y| {
                    let mut nx = (x as f32 + 0.5) / w * 2.0 - 1.0;
                    let ny = (y as f32 + 0.5) / h * 2.0 - 1.0;
                    // roundness 100 = circle; 0 = an ellipse following the frame; −100 = squarer
                    if round > 0.0 {
                        nx *= 1.0 + (aspect - 1.0) * round;
                    }
                    let p = if round < 0.0 { 2.0 + (-round) * 6.0 } else { 2.0 };
                    // 0 at the centre, 1 at the corners
                    let d = (nx.abs().powf(p) + ny.abs().powf(p)).powf(1.0 / p) / 2f32.powf(1.0 / p);
                    let edge = crate::vfx::smoothstep(mid - feather * 0.5, mid + feather * 0.5, d);
                    dec(crate::vfx::lerp3(enc(c), target, edge * amt.abs()))
                });
            }
            FxOp::VideoLimiter { max, comp, axis, warn, warning_color } => {
                let (max, comp, axis, warn, wc) = (*max, *comp, *axis, *warn, *warning_color);
                if !max.is_finite() || !comp.is_finite() || !wc.iter().all(|c| c.is_finite()) {
                    return;
                }
                img.map_rgb(|c, _, _| {
                    let v = [linear_to_enc(c[0]), linear_to_enc(c[1]), linear_to_enc(c[2])];
                    let out = limit(v, max, comp, axis);
                    if warn && v.iter().zip(&out).any(|(a, b)| (a - b).abs() > 1e-4) {
                        return wc;
                    }
                    out.map(enc_to_linear)
                });
            }
            FxOp::Lumetri {
                gains,
                exposure,
                contrast,
                hl,
                sh,
                wh,
                bl,
                sat,
                creative_on,
                faded,
                vib,
                st,
                ht,
                vignette_on,
                va,
                vmid,
                vround,
                vfeather,
                aspect,
                ..
            } => {
                let gains = *gains;
                let (exposure, contrast, hl, sh, wh, bl, sat) = (*exposure, *contrast, *hl, *sh, *wh, *bl, *sat);
                let (creative_on, faded, vib, st, ht) = (*creative_on, *faded, *vib, *st, *ht);
                let (vignette_on, va, vmid, vround, vfeather, aspect) = (*vignette_on, *va, *vmid, *vround, *vfeather, *aspect);
                let (w, h) = (img.w as f32, img.h as f32);
                img.map_rgb(|c, x, y| {
                    // white balance + exposure in linear light
                    let lin = [c[0] * gains[0] * exposure, c[1] * gains[1] * exposure, c[2] * gains[2] * exposure];
                    let mut v = enc(lin);
                    // whites / blacks: endpoints
                    let b0 = -bl * 0.15;
                    let w0 = 1.0 - wh * 0.15;
                    v = v.map(|q| (q - b0) / (w0 - b0).max(1e-3));
                    // highlights / shadows: luma-weighted lift/compress, hue preserving
                    let l = 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
                    let ws = (1.0 - l).clamp(0.0, 1.0).powi(3);
                    let whl = l.clamp(0.0, 1.0).powi(3);
                    let nl = (l + sh * 0.35 * ws + hl * 0.35 * whl).max(0.0);
                    if l > 1e-5 {
                        let k = nl / l;
                        v = v.map(|q| q * k);
                    }
                    // contrast: smooth S-curve around mid grey
                    if contrast.abs() > 1e-4 {
                        let k = 1.0 + contrast;
                        v = v.map(|q| {
                            let q = q.clamp(0.0, 1.0);
                            let s = q * q * (3.0 - 2.0 * q);
                            if k >= 1.0 { q + (s - q) * (k - 1.0) } else { 0.5 + (q - 0.5) * k }
                        });
                    }
                    // faded film: lift blacks and compress
                    if faded > 0.0 {
                        v = v.map(|q| q * (1.0 - 0.25 * faded) + 0.12 * faded);
                    }
                    // split tone
                    if creative_on {
                        let l2 = (0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2]).clamp(0.0, 1.0);
                        for k in 0..3 {
                            v[k] += (st[k] - 0.5) * 0.3 * (1.0 - l2) + (ht[k] - 0.5) * 0.3 * l2;
                        }
                    }
                    // saturation & vibrance
                    let l3 = 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
                    let cur_sat = v[0].max(v[1]).max(v[2]) - v[0].min(v[1]).min(v[2]);
                    let s = sat * (1.0 + vib * (1.0 - cur_sat.clamp(0.0, 1.0)));
                    v = v.map(|q| l3 + (q - l3) * s);
                    // vignette
                    if vignette_on && va.abs() > 1e-4 {
                        let nx = (x as f32 / w - 0.5) * 2.0 * lumetri_vignette_aspect(aspect, vround);
                        let ny = (y as f32 / h - 0.5) * 2.0;
                        let d = (nx * nx + ny * ny).sqrt() / std::f32::consts::SQRT_2;
                        let edge = ((d - vmid * 0.9) / (vfeather.max(0.01) * 0.9)).clamp(0.0, 1.0);
                        let e2 = edge * edge * (3.0 - 2.0 * edge);
                        let k = 1.0 + va * 0.2 * e2;
                        v = v.map(|q| if va < 0.0 { q * k.max(0.0) } else { q + (1.0 - q) * (k - 1.0) });
                    }
                    dec(v)
                });
            }
        }
    }
}

/// Horizontal scale of the Lumetri vignette's coordinates for a Roundness of `vround`
/// (−1 … 1) on a frame `aspect` wide. At 0 the vignette is an ellipse that follows the
/// frame; positive values move it towards a circle (1 = a true circle), negative values
/// stretch it into a wider oval. Non-finite or out-of-range input never yields NaN.
pub fn lumetri_vignette_aspect(aspect: f32, vround: f32) -> f32 {
    if !aspect.is_finite() || aspect <= 0.0 || !vround.is_finite() {
        return 1.0;
    }
    aspect.powf(vround.clamp(-1.0, 1.0))
}

/// sRGB encoding that keeps values above 1 (super-whites) instead of clamping them.
#[inline]
fn linear_to_enc(v: f32) -> f32 {
    if v <= 1.0 { cfa4_color::linear_to_srgb(v.max(0.0)) } else { 1.0 + (v - 1.0) / 2.4 }
}
#[inline]
fn enc_to_linear(v: f32) -> f32 {
    if v <= 1.0 { cfa4_color::srgb_to_linear(v.max(0.0)) } else { 1.0 + (v - 1.0) * 2.4 }
}

#[inline]
fn knee(v: f32, max: f32, comp: f32) -> f32 {
    let k = max * (1.0 - comp);
    if comp <= 0.0 || v <= k {
        return v.min(max);
    }
    let r = max - k;
    k + r * (1.0 - (-(v - k) / r).exp())
}

pub(crate) fn limit(v: [f32; 3], max: f32, comp: f32, axis: u32) -> [f32; 3] {
    let m = cfa4_color::Matrix::Bt709;
    let ycc = cfa4_color::rgb_to_ycbcr(v[0], v[1], v[2], m);
    let (kr, kb) = (0.2126f32, 0.0722f32);
    let to_rgb = |y: f32, cb: f32, cr: f32| {
        let r = y + 2.0 * (1.0 - kr) * cr;
        let b = y + 2.0 * (1.0 - kb) * cb;
        let g = (y - kr * r - kb * b) / (1.0 - kr - kb);
        [r, g, b]
    };
    let (mut y, mut cb, mut cr) = (ycc[0], ycc[1], ycc[2]);
    if axis != 1 {
        y = knee(y.max(0.0), max, comp);
    }
    if axis != 0 {
        // scale chroma so every channel fits 0…max (the gamut of legal R′G′B′)
        let rgb = to_rgb(y, cb, cr);
        let mut s = 1.0f32;
        for &ch in &rgb {
            let d = ch - y;
            if ch > max && d > 1e-6 {
                s = s.min(((max - y) / d).max(0.0));
            }
            if ch < 0.0 && d < -1e-6 {
                s = s.min((y / -d).max(0.0));
            }
        }
        if axis == 3 && s < 1.0 - 1e-4 {
            // smart limit: soft chroma compression rather than a hard cut
            s = knee(s, 1.0, comp.max(0.03)).min(1.0);
        }
        cb *= s;
        cr *= s;
    }
    to_rgb(y, cb, cr).map(|q| q.clamp(0.0, max))
}

/// The body of `Image::transformed` (no mip path) for a precomputed inverse and rectangle.
fn resample_cpu(src: &Image, inv: &Affine, rect: [u32; 4]) -> Image {
    let (w, h) = (src.w, src.h);
    let mut out = Image::new(w, h);
    let [x0, x1, y0, y1] = rect.map(|v| v as usize);
    let (x1, y1) = (x1.min(w), y1.min(h));
    let (x0, y0) = (x0.min(x1), y0.min(y1));
    let axis_aligned = inv.b == 0.0 && inv.c == 0.0;
    out.px.par_chunks_mut(w * 4).enumerate().skip(y0).take(y1 - y0).for_each(|(y, row)| {
        let py = y as f64 + 0.5;
        for x in x0..x1 {
            let px = x as f64 + 0.5;
            let (u, v) =
                if axis_aligned { (inv.a * px + inv.e, inv.d * py + inv.f) } else { (inv.a * px + inv.c * py + inv.e, inv.b * px + inv.d * py + inv.f) };
            if u < -1.0 || v < -1.0 || u > w as f64 + 1.0 || v > h as f64 + 1.0 {
                continue;
            }
            row[x * 4..x * 4 + 4].copy_from_slice(&src.sample_bilinear(u as f32, v as f32));
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfa4_project::{ParamValue, find_effect};
    use cfa4_time::Tick;

    #[test]
    fn extract_zero_softness_has_no_fractional_edge() {
        let cx = FxCtx {
            t: Tick::ZERO,
            px_scale: 1.0,
            seconds: 0.0,
            timecode: "",
            clip_name: "",
            project: None,
            env: None,
            frame_rate: 24.0,
            working: cfa4_color::WorkingSpace::Rec709,
        };
        // Both sides of each cutoff, including samples inside the former artificial fade.
        let lo = 90.0 / 255.0;
        let hi = 160.0 / 255.0;
        let samples = [lo - 0.00005, lo + 0.00005, hi - 0.00005, hi + 0.00005];
        for softness in [0.0, -1.0] {
            for invert in [false, true] {
                for alpha in [1.0, 0.5] {
                    let mut e = find_effect("extract").unwrap().instance();
                    for (key, value) in [("black", 90.0), ("white", 160.0), ("softness", softness)] {
                        e.params.get_mut(key).unwrap().value = ParamValue::Float(value);
                    }
                    e.params.get_mut("invert").unwrap().value = ParamValue::Bool(invert);
                    let mut img = Image {
                        w: 4,
                        h: 1,
                        px: samples
                            .iter()
                            .flat_map(|v| {
                                let c = cfa4_color::srgb_to_linear(*v) * alpha;
                                [c, c, c, alpha]
                            })
                            .collect(),
                    };
                    FxOp::eval(&e, &cx, 4, 1).unwrap().apply(&mut img);
                    for (pixel, inside) in img.px.as_chunks::<4>().0.iter().zip([false, true, true, false]) {
                        let value = if inside != invert { alpha } else { 0.0 };
                        assert_eq!(*pixel, [value, value, value, alpha], "softness={softness}, invert={invert}");
                    }
                }
            }
        }
    }

    /// A picture where every pixel is distinct, so a swap that moves the wrong pixel shows.
    fn numbered(w: usize, h: usize) -> Image {
        let mut img = Image::new(w, h);
        for (i, p) in img.px.chunks_mut(4).enumerate() {
            p.copy_from_slice(&[i as f32, (i % w) as f32, (i / w.max(1)) as f32, 1.0]);
        }
        img
    }

    fn swap_eyes(layout: u32) -> EffectInstance {
        let mut e = find_effect("stereo_swap_eyes").unwrap().instance();
        e.params.get_mut("frame_layout").unwrap().value = ParamValue::Choice(layout);
        e
    }

    fn eval_ctx() -> FxCtx<'static> {
        FxCtx {
            t: Tick::ZERO,
            px_scale: 1.0,
            seconds: 0.0,
            timecode: "",
            clip_name: "",
            project: None,
            env: None,
            frame_rate: 24.0,
            working: cfa4_color::WorkingSpace::Rec709,
        }
    }

    #[test]
    fn swap_eyes_trades_the_views() {
        let src = numbered(6, 4);
        // over/under: the top two rows come from the bottom half and back
        let mut ou = src.clone();
        FxOp::eval(&swap_eyes(0), &eval_ctx(), 6, 4).unwrap().apply(&mut ou);
        assert_eq!(ou.get(1, 0), src.get(1, 2));
        assert_eq!(ou.get(5, 3), src.get(5, 1));
        // side by side: the left three columns come from the right half and back
        let mut sbs = src.clone();
        FxOp::eval(&swap_eyes(1), &eval_ctx(), 6, 4).unwrap().apply(&mut sbs);
        assert_eq!(sbs.get(0, 1), src.get(3, 1));
        assert_eq!(sbs.get(5, 2), src.get(2, 2));
    }

    #[test]
    fn swap_eyes_twice_is_identity_and_odd_edges_stay() {
        for (w, h) in [(7, 5), (1, 1), (2, 1), (1, 2), (67, 41)] {
            for layout in [0, 1, 9] {
                let src = numbered(w, h);
                let op = FxOp::eval(&swap_eyes(layout), &eval_ctx(), w, h).unwrap();
                assert!(op.gpu_ok());
                let mut img = src.clone();
                op.apply(&mut img);
                op.apply(&mut img);
                assert_eq!(img.px, src.px, "{w}x{h} layout {layout}");
            }
        }
        let src = numbered(7, 5);
        let mut img = src.clone();
        FxOp::SwapEyes { sbs: true }.apply(&mut img);
        assert_eq!(img.get(6, 2), src.get(6, 2), "the odd last column belongs to neither view");
        let mut img = src.clone();
        FxOp::SwapEyes { sbs: false }.apply(&mut img);
        assert_eq!(img.get(3, 4), src.get(3, 4), "the odd last row belongs to neither view");
    }

    #[test]
    fn swap_eyes_on_an_empty_image_is_a_no_op() {
        let mut img = Image::new(0, 0);
        FxOp::SwapEyes { sbs: true }.apply(&mut img);
        assert!(img.px.is_empty());
    }

    fn stereo_to_2d(layout: u32, eye: u32) -> EffectInstance {
        let mut e = find_effect("stereo_to_2d").unwrap().instance();
        e.params.get_mut("frame_layout").unwrap().value = ParamValue::Choice(layout);
        e.params.get_mut("eye").unwrap().value = ParamValue::Choice(eye);
        e
    }

    #[test]
    fn stereo_to_2d_stretches_the_kept_view() {
        let mut row = Image::new(4, 1);
        for (x, v) in [0.0, 1.0, 2.0, 3.0].into_iter().enumerate() {
            row.px[x * 4..x * 4 + 4].copy_from_slice(&[v, v, v, 1.0]);
        }
        let run = |eye| {
            let mut img = row.clone();
            FxOp::eval(&stereo_to_2d(1, eye), &eval_ctx(), 4, 1).unwrap().apply(&mut img);
            (0..4).map(|x| img.get(x, 0)[0]).collect::<Vec<_>>()
        };
        assert_eq!(run(0), [0.0, 0.25, 0.75, 1.0], "left view 0..1 over four columns");
        assert_eq!(run(1), [2.0, 2.25, 2.75, 3.0], "right view 2..3 over four columns");
    }

    #[test]
    fn stereo_to_2d_never_mixes_in_the_other_eye() {
        for (w, h, layout) in [(9, 6, 0), (9, 7, 0), (10, 5, 1), (11, 5, 1), (2, 2, 1), (2, 2, 0)] {
            let mut src = Image::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let left = if layout == 1 { x < w / 2 } else { y < h / 2 };
                    let c = if left { [1.0, 0.0, 0.0, 1.0] } else { [0.0, 0.0, 1.0, 1.0] };
                    src.px[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&c);
                }
            }
            for eye in [0, 1] {
                let mut img = src.clone();
                let op = FxOp::eval(&stereo_to_2d(layout, eye), &eval_ctx(), w, h).unwrap();
                assert!(op.gpu_ok());
                op.apply(&mut img);
                let want = if eye == 0 { [1.0, 0.0, 0.0, 1.0] } else { [0.0, 0.0, 1.0, 1.0] };
                assert!(img.px.chunks(4).all(|p| p == want), "{w}x{h} layout {layout} eye {eye}");
            }
        }
    }

    #[test]
    fn stereo_to_2d_without_two_views_is_a_no_op() {
        for (w, h, layout) in [(1, 5, 1), (5, 1, 0), (1, 1, 0), (1, 1, 1)] {
            let src = numbered(w, h);
            let mut img = src.clone();
            FxOp::eval(&stereo_to_2d(layout, 1), &eval_ctx(), w, h).unwrap().apply(&mut img);
            assert_eq!(img.px, src.px, "{w}x{h} layout {layout}");
        }
    }

    fn six_point(pts: &[(&str, f64, f64)]) -> EffectInstance {
        let mut e = find_effect("six_point_pin").unwrap().instance();
        for (k, x, y) in pts {
            e.params.get_mut(*k).unwrap().value = ParamValue::Vec2(Vec2::new(*x, *y));
        }
        e
    }

    #[test]
    fn six_point_pin_defaults_leave_the_frame_alone() {
        let op = FxOp::eval(&six_point(&[]), &eval_ctx(), 9, 7).unwrap();
        assert!(matches!(op, FxOp::SixPointPin { identity: true, .. }), "{op:?}");
        let mut img = numbered(9, 7);
        op.apply(&mut img);
        assert_eq!(img, numbered(9, 7));
    }

    #[test]
    fn six_point_pin_inverts_the_curved_map() {
        let e = six_point(&[("upper_center", 50.0, 12.0), ("lower_center", 45.0, 70.0), ("lower_right", 96.0, 78.0)]);
        let Some(FxOp::SixPointPin { edges, inv, identity: false }) = FxOp::eval(&e, &eval_ctx(), 100, 80) else { panic!() };
        // the forward map: the top Bézier passes through its middle point at u = 1/2
        let bez = |o: usize, t: f32| {
            let (s, c, d) = ((1.0 - t) * (1.0 - t), 2.0 * t * (1.0 - t), t * t);
            (edges[o] * s + edges[o + 2] * c + edges[o + 4] * d, edges[o + 1] * s + edges[o + 3] * c + edges[o + 5] * d)
        };
        assert_eq!(bez(0, 0.5), (50.0, 12.0));
        assert_eq!(bez(6, 0.5), (45.0, 70.0));
        for (u, v) in [(0.1f32, 0.2f32), (0.5, 0.5), (0.9, 0.95), (0.02, 0.98), (0.7, 0.01)] {
            let (t, b) = (bez(0, u), bez(6, u));
            let q = (t.0 + v * (b.0 - t.0), t.1 + v * (b.1 - t.1));
            let (su, sv) = six_point_source(&edges, &inv, q.0, q.1).unwrap_or_else(|| panic!("{u} {v}"));
            assert!((su - u).abs() < 1e-5 && (sv - v).abs() < 1e-5, "{u} {v}: {su} {sv}");
        }
        // above the bent top edge there is no picture
        assert_eq!(six_point_source(&edges, &inv, 50.0, 6.0), None);
        let mut img = Image::filled(100, 80, [0.5, 0.25, 0.125, 1.0]);
        FxOp::eval(&e, &eval_ctx(), 100, 80).unwrap().apply(&mut img);
        assert_eq!(img.get(50, 5), [0.0; 4]);
        assert_eq!(img.get(50, 40), [0.5, 0.25, 0.125, 1.0]);
        assert_eq!(img.get(50, 75), [0.0; 4], "below the bent bottom edge");
    }

    #[test]
    fn six_point_pin_survives_hostile_points() {
        for pts in [
            vec![("upper_center", f64::INFINITY, 0.0)],
            vec![("lower_left", 1e300, -1e300), ("upper_right", f64::NEG_INFINITY, 3.0)],
            // every point on top of each other: no area, nothing drawn
            ["upper_left", "upper_center", "upper_right", "lower_left", "lower_center", "lower_right"].map(|k| (k, 4.0, 4.0)).to_vec(),
            // edges crossing over (a folded frame)
            vec![("upper_left", 9.0, 7.0), ("lower_right", 0.0, 0.0), ("upper_center", 4.0, 30.0)],
        ] {
            let op = FxOp::eval(&six_point(&pts), &eval_ctx(), 9, 7).unwrap();
            let mut img = numbered(9, 7);
            op.apply(&mut img);
            assert!(img.px.iter().all(|v| v.is_finite()), "{pts:?}");
        }
    }

    fn parallax(layout: u32, shift: f64, repeat: bool) -> EffectInstance {
        let mut e = find_effect("stereo_parallax").unwrap().instance();
        e.params.get_mut("frame_layout").unwrap().value = ParamValue::Choice(layout);
        e.params.get_mut("shift").unwrap().value = ParamValue::Float(shift);
        e.params.get_mut("repeat_edge").unwrap().value = ParamValue::Bool(repeat);
        e
    }

    #[test]
    fn stereo_parallax_moves_the_views_apart() {
        // side by side, 2 × 10 columns: 10 % of a 10-pixel view = 1 px, half of it per eye
        let mut row = Image::new(20, 1);
        for x in 0..20 {
            row.px[x * 4..x * 4 + 4].copy_from_slice(&[x as f32, 0.0, 0.0, 1.0]);
        }
        let run = |shift, repeat| {
            let mut img = row.clone();
            FxOp::eval(&parallax(1, shift, repeat), &eval_ctx(), 20, 1).unwrap().apply(&mut img);
            img
        };
        let img = run(10.0, false);
        // left view content 0.5 px right: column 3 shows 2.5, column 0 half faded from the edge
        assert_eq!(img.get(3, 0), [2.5, 0.0, 0.0, 1.0]);
        assert_eq!(img.get(0, 0), [0.0, 0.0, 0.0, 0.5]);
        // right view content 0.5 px left, never reaching into the left view
        assert_eq!(img.get(13, 0), [13.5, 0.0, 0.0, 1.0]);
        assert_eq!(img.get(19, 0), [9.5, 0.0, 0.0, 0.5]);
        let img = run(10.0, true);
        assert_eq!(img.get(0, 0), [0.0, 0.0, 0.0, 1.0], "repeat edge");
        assert_eq!(img.get(19, 0), [19.0, 0.0, 0.0, 1.0], "repeat edge");
        let img = run(0.0, false);
        assert_eq!(img.px, row.px, "no shift is the identity");
    }

    #[test]
    fn stereo_parallax_over_under_and_odd_edges() {
        let src = numbered(9, 7);
        for (layout, shift) in [(0, 4.0), (1, -7.5), (0, 1e9), (1, f64::NAN)] {
            let op = FxOp::eval(&parallax(layout, shift, false), &eval_ctx(), 9, 7).unwrap();
            assert!(op.gpu_ok(), "{layout} {shift}");
            let mut img = src.clone();
            op.apply(&mut img);
            assert!(img.px.iter().all(|v| v.is_finite()));
            if layout == 0 {
                assert_eq!(img.get(4, 6), src.get(4, 6), "the odd last row belongs to neither view");
            } else {
                assert_eq!(img.get(8, 3), src.get(8, 3), "the odd last column belongs to neither view");
            }
        }
        // over/under: both views shift horizontally in opposite directions
        let mut img = numbered(8, 4);
        FxOp::StereoParallax { sbs: false, shift: 1.0, repeat: true }.apply(&mut img);
        let src = numbered(8, 4);
        assert_eq!(img.get(3, 0), src.get(2, 0));
        assert_eq!(img.get(3, 3), src.get(4, 3));
    }
}
