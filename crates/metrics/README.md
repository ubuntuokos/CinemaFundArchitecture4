# cfa4-metrics

Image fidelity metrics for comparing CFA4's output with a reference render: the scoring layer of
the output-comparison harness ([docs/gaps.md](../../docs/gaps.md) G1). Layer L1, depends only on
`cfa4-color`, no `unsafe`, no panics outside tests.

## API

```rust
use cfa4_metrics::{ciede2000, delta_e_srgb8, srgb8_to_lab, ssim_u8, ssim_u16, Lab};

let de = ciede2000(&Lab::new(50.0, 2.6772, -79.7751), &Lab::new(50.0, 0.0, -82.7485)); // 2.0425
let stats = delta_e_srgb8(&ours_rgb, &reference_rgb, width, height, 3)?;              // mean, max, above_jnd
let s = ssim_u8(&ours_y, &reference_y, width, height)?;                               // 0..1, 1 = identical
let s10 = ssim_u16(&ours_y10, &reference_y10, width, height, 10)?;
```

| Function | What it computes |
|---|---|
| `ciede2000`, `ciede2000_with` | ΔE₀₀ between two CIELAB colours (CIE 142-2001), optional kL / kC / kH |
| `delta_e_srgb8` | per-pixel ΔE₀₀ of two 8-bit sRGB images (RGB or RGBA, alpha ignored): mean, max, share of pixels above 1 |
| `xyz_to_lab`, `linear_rgb_to_lab`, `srgb8_to_lab` | CIELAB (D65) from XYZ, from linear RGB in any `cfa4_color::Gamut`, from 8-bit sRGB |
| `ssim`, `ssim_u8`, `ssim_u16` | mean SSIM of one plane: 11 × 11 Gaussian window (σ = 1.5), K1 = 0.01, K2 = 0.03, valid positions only |

Inputs are checked: mismatched or wrong-length buffers, images smaller than the window, sizes
above 16K × 16K and a non-positive data range return a `MetricError`.

## Verification

`tests/oracle.rs`:

- CIEDE2000: all 34 pairs of Sharma, Wu and Dalal (2005), Table 1, within 5 × 10⁻⁵ (the table's
  rounding), and symmetric.
- SSIM: identical to scikit-image 0.26.0 `structural_similarity(..., gaussian_weights=True,
  sigma=1.5, use_sample_covariance=False)` within 10⁻¹² on 8-bit and 10-bit test images.
- sRGB → Lab and image ΔE: within 0.01 / 0.001 of scikit-image `rgb2lab` / `deltaE_ciede2000`
  (scikit-image uses the rounded IEC 61966-2-1 matrix; ours is derived from the BT.709
  chromaticities in `cfa4-color`).

## Provenance

The CIEDE2000 formula and the Gaussian SSIM window follow `oximedia-quality` of
[OxiMedia](https://github.com/cool-japan/oximedia) 0.2.1 (Apache-2.0, `LICENSE-oximedia`),
rewritten for CFA4; see [docs/licensing.md](../../docs/licensing.md) §5. OxiMedia's VMAF is a
weighted approximation, not Netflix's trained model, and was not taken.
