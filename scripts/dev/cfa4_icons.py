#!/usr/bin/env python3
"""Regenerate the CFA4 app icons from the owner's logo (assets/cfa4/cfa4-official-logo.png).

The icon is the logo's emblem (the A with the film strip) on a transparent background; the
wordmark is left out because it is unreadable at icon sizes. Writes:

  assets/cfa4/cfa4-emblem.png                          the emblem, transparent, full size
  assets/cfa4/hicolor/<n>x<n>/apps/io.github.ubuntuokos.cfa4.png   16 … 512 px
  assets/cfa4/hicolor/scalable/apps/io.github.ubuntuokos.cfa4.svg  the 512 px emblem in an SVG
      (the logo is raster artwork; the SVG keeps the scalable path packaging and the egui
      development launcher already use)

Needs Pillow, NumPy and SciPy (development only: `pip install pillow numpy scipy`).
"""

from __future__ import annotations

import base64
import io
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageFilter
from scipy import ndimage

ROOT = Path(__file__).resolve().parents[2]
CFA4 = ROOT / "assets" / "cfa4"
APP_ID = "io.github.ubuntuokos.cfa4"
SIZES = (16, 22, 24, 32, 48, 64, 128, 256, 512)


def emblem(logo: Image.Image) -> Image.Image:
    im = np.asarray(logo.convert("RGB")).astype(np.int16)
    top = im[: int(im.shape[0] * 0.585)]  # above the "CFA4" wordmark
    # background: light, unsaturated pixels (white and the soft floor shadow); the film holes are
    # cream/gold, so saturated, and stay
    light = (top.min(axis=2) >= 196) & ((top.max(axis=2) - top.min(axis=2)) < 28)
    labels, n = ndimage.label(light)
    sizes = ndimage.sum(light, labels, range(1, n + 1))
    border = set(np.unique(np.concatenate([labels[0], labels[-1], labels[:, 0], labels[:, -1]]))) - {0}
    big = {i + 1 for i, size in enumerate(sizes) if size > 1500}  # inside the A; not highlights
    alpha = np.where(np.isin(labels, list(border | big)), 0, 255).astype(np.uint8)
    rgba = Image.fromarray(top.astype(np.uint8)).convert("RGBA")
    rgba.putalpha(Image.fromarray(alpha).filter(ImageFilter.GaussianBlur(0.8)))
    box = rgba.getbbox()
    if box is None:
        raise SystemExit("cfa4_icons: the logo has no emblem above the wordmark")
    part = rgba.crop(box)
    side = int(max(part.size) * 1.06)
    canvas = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    canvas.paste(part, ((side - part.width) // 2, (side - part.height) // 2), part)
    return canvas


def main() -> int:
    logo = Image.open(CFA4 / "cfa4-official-logo.png")
    art = emblem(logo)
    art.save(CFA4 / "cfa4-emblem.png", optimize=True)
    for n in SIZES:
        out = CFA4 / "hicolor" / f"{n}x{n}" / "apps" / f"{APP_ID}.png"
        out.parent.mkdir(parents=True, exist_ok=True)
        art.resize((n, n), Image.Resampling.LANCZOS).save(out, optimize=True)
    buf = io.BytesIO()
    art.resize((512, 512), Image.Resampling.LANCZOS).save(buf, format="PNG", optimize=True)
    data = base64.b64encode(buf.getvalue()).decode()
    svg = (
        '<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" '
        'viewBox="0 0 512 512" width="512" height="512" role="img" aria-label="CFA4">\n'
        "  <title>CFA4</title>\n"
        f'  <image width="512" height="512" xlink:href="data:image/png;base64,{data}" href="data:image/png;base64,{data}"/>\n'
        "</svg>\n"
    )
    (CFA4 / "hicolor" / "scalable" / "apps" / f"{APP_ID}.svg").write_text(svg, encoding="ascii")
    print(f"cfa4_icons: {len(SIZES)} PNG sizes, SVG and emblem written under {CFA4}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
