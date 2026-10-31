"""CFA4's mpv / libplacebo playback shaders (packaging/mpv-shaders).

The structure checks always run. The render checks push a coordinate ramp (red = x, green = y)
through each shader with FFmpeg's libplacebo filter, which runs mpv-format user shaders, and
compare every pixel with the mapping the shader documents. They skip when FFmpeg, its libplacebo
filter or a Vulkan device (lavapipe is enough) is missing.
"""

from __future__ import annotations

import math
import re
import shutil
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
SHADERS = ROOT / "packaging/mpv-shaders"
NAMES = [
    "cfa4-dual-projector-convergence.glsl",
    "cfa4-black-bar-filler.glsl",
    "cfa4-spot-correction.glsl",
    "cfa4-screenxizer-cylinder.glsl",
]
# 8-bit output, bilinear taps on the GPU
TOL = 3.0 / 255.0


def params(text: str) -> dict[str, tuple[str, float, float, float]]:
    """`//!PARAM` blocks: name -> (type, minimum, maximum, default)."""
    out = {}
    lines = text.splitlines()
    for i, line in enumerate(lines):
        if not line.startswith("//!PARAM "):
            continue
        name = line.split()[1]
        meta = {}
        j = i + 1
        while j < len(lines) and lines[j].startswith("//!"):
            key, _, value = lines[j][3:].partition(" ")
            meta[key] = value.strip()
            j += 1
        out[name] = (meta.get("TYPE", ""), float(meta["MINIMUM"]), float(meta["MAXIMUM"]), float(lines[j]))
    return out


def test_shader_files_are_complete() -> None:
    assert sorted(p.name for p in SHADERS.glob("*.glsl")) == sorted(NAMES)
    readme = (SHADERS / "README.md").read_text(encoding="utf-8")
    for name in NAMES:
        text = (SHADERS / name).read_text(encoding="utf-8")
        assert text.startswith("// SPDX-License-Identifier: GPL-3.0-or-later\n"), name
        assert "//!HOOK MAIN" in text and "//!BIND HOOKED" in text and "vec4 hook()" in text, name
        ps = params(text)
        assert ps, name
        for p, (kind, lo, hi, default) in ps.items():
            assert kind in ("int", "float"), (name, p)
            assert lo <= default <= hi, (name, p)
            # every parameter is a uniform the hook body uses
            assert re.search(rf"\b{p}\b", text.split("//!HOOK MAIN", 1)[1]), (name, p)
        assert f"`{name}`" in readme, name


def with_params(name: str, values: dict[str, float], out: Path) -> Path:
    """A copy of the shader with other parameter defaults (FFmpeg cannot set them)."""
    lines = (SHADERS / name).read_text(encoding="utf-8").splitlines()
    known = params("\n".join(lines))
    for p in values:
        assert p in known, p
    current = None
    for i, line in enumerate(lines):
        if line.startswith("//!PARAM "):
            current = line.split()[1]
        elif current and not line.startswith("//!"):
            if current in values:
                v = values[current]
                lines[i] = str(int(v)) if known[current][0] == "int" else repr(float(v))
            current = None
    out.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return out


def render(shader: Path, w: int, h: int, blue: float = 0.5) -> list[list[tuple[float, float, float]]] | None:
    """The coordinate ramp through `shader`, or None when the GPU path is unavailable."""
    ffmpeg = shutil.which("ffmpeg")
    if ffmpeg is None:
        return None
    src = bytearray()
    for y in range(h):
        for x in range(w):
            src += bytes((round(255 * x / (w - 1)), round(255 * y / (h - 1)), round(255 * blue), 255))
    cmd = [
        ffmpeg,
        "-hide_banner",
        "-loglevel",
        "error",
        "-init_hw_device",
        "vulkan",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "-s",
        f"{w}x{h}",
        "-i",
        "pipe:0",
        "-vf",
        f"hwupload,libplacebo=custom_shader_path={shader}:format=rgba:dithering=none,hwdownload,format=rgba",
        "-f",
        "rawvideo",
        "pipe:1",
    ]
    try:
        # three frames, the last one is read: on lavapipe the first downloaded frame can start with
        # a few stale bytes
        r = subprocess.run(cmd, input=bytes(src) * 3, capture_output=True, timeout=120, check=False)
    except (OSError, subprocess.TimeoutExpired):
        return None
    size = w * h * 4
    if r.returncode != 0 or len(r.stdout) != 3 * size:
        return None
    px = r.stdout[2 * size :]
    return [[tuple(px[(y * w + x) * 4 + k] / 255.0 for k in range(3)) for x in range(w)] for y in range(h)]


def ramp(u: float, n: int) -> float:
    """The ramp's value where the shader samples at texture coordinate `u` (bilinear, clamped)."""
    return min(max((u * n - 0.5) / (n - 1), 0.0), 1.0)


def gpu(name: str, values: dict[str, float], w: int, h: int, tmp: Path, blue: float = 0.5):
    out = render(with_params(name, values, tmp / name), w, h, blue)
    if out is None:
        pytest.skip("FFmpeg with libplacebo and a Vulkan device is needed")
    return out


def compare(out, expect, skip=lambda x, y: False) -> None:
    worst = (0.0, None)
    for y, row in enumerate(out):
        for x, got in enumerate(row):
            if skip(x, y):
                continue
            want = expect(x, y)
            d = max(abs(a - b) for a, b in zip(got, want))
            if d > worst[0]:
                worst = (d, (x, y, got, want))
    assert worst[0] <= TOL, worst


def test_dual_projector_convergence_moves_only_the_second_view(tmp_path: Path) -> None:
    w, h = 128, 64
    shift, scale = 6.0, 110.0
    out = gpu("cfa4-dual-projector-convergence.glsl", {"layout": 1, "shift_x": shift, "scale": scale}, w, h, tmp_path)
    k = scale / 100.0

    def local(x: int) -> float:
        return (((x + 0.5) / w - 0.5) / 0.5 - 0.5 - shift / 100.0) / k + 0.5

    def expect(x, y):
        v = (y + 0.5) / h
        if x < w // 2:
            return (ramp((x + 0.5) / w, w), ramp(v, h), 0.5)
        lx, ly = local(x), (v - 0.5) / k + 0.5
        if not (0.0 <= lx <= 1.0 and 0.0 <= ly <= 1.0):
            return (0.0, 0.0, 0.0)
        half = 0.5 / (w / 2)
        return (ramp(0.5 + min(max(lx, half), 1 - half) * 0.5, w), ramp(ly, h), 0.5)

    # the black border is a step: leave a texel either side of it out
    def near_edge(x, y):
        lx, ly = local(x), ((y + 0.5) / h - 0.5) / k + 0.5
        step = 2.0 / (w * k)
        return x >= w // 2 and (min(abs(lx), abs(lx - 1)) < step or min(abs(ly), abs(ly - 1)) < 2.0 / (h * k))

    compare(out, expect, near_edge)
    # shifted right, the moved view leaves a black strip along its left edge
    assert out[h // 2][w // 2] == (0.0, 0.0, 0.0)


def bar_profile(t: float, c: float, warp: float) -> float:
    if c <= 2 / math.pi:
        a = math.pi / 2
    else:
        lo, hi = 1e-3, math.pi / 2
        for _ in range(60):
            m = (lo + hi) / 2
            lo, hi = (m, hi) if math.sin(m) > c * m else (lo, m)
        a = (lo + hi) / 2
    a *= warp
    return c * t if a < 1e-3 else c * math.sin(a * t) / math.sin(a)


@pytest.mark.parametrize("warp", [0.0, 1.0])
def test_black_bar_filler_stretches_scope_over_the_bars(tmp_path: Path, warp: float) -> None:
    w, h = 160, 90
    out = gpu("cfa4-black-bar-filler.glsl", {"content_aspect": 2.39, "warp": warp}, w, h, tmp_path)
    c = (w / h) / 2.39

    def expect(x, y):
        v = 0.5 + 0.5 * bar_profile(2 * (y + 0.5) / h - 1, c, warp)
        return (ramp((x + 0.5) / w, w), ramp(v, h), 0.5)

    compare(out, expect)
    if warp == 1.0:
        # the middle keeps its 1:1 scale: one row on from the centre, one row on in the picture
        mid = bar_profile(2 / h, c, warp) * h / 2
        assert abs(mid - 1.0) < 1e-3


def test_black_bar_filler_leaves_pillarbox_free_frames_alone(tmp_path: Path) -> None:
    w, h = 64, 48
    out = gpu("cfa4-black-bar-filler.glsl", {"content_aspect": 1.0}, w, h, tmp_path)
    c = 1.0 / (w / h)
    compare(
        out, lambda x, y: (ramp(0.5 + 0.5 * bar_profile(2 * (x + 0.5) / w - 1, c, 1.0), w), ramp((y + 0.5) / h, h), 0.5)
    )


def test_spot_correction_only_changes_the_spot(tmp_path: Path) -> None:
    w, h = 96, 64
    cx, cy, rx, ry, feather = 0.4, 0.6, 0.25, 0.3, 0.5
    values = {
        "center_x": cx,
        "center_y": cy,
        "radius_x": rx,
        "radius_y": ry,
        "feather": feather,
        "gain_b": 1.8,
        "offset_r": -0.1,
    }
    out = gpu("cfa4-spot-correction.glsl", values, w, h, tmp_path)

    def smoothstep(e0, e1, x):
        t = min(max((x - e0) / (e1 - e0), 0.0), 1.0)
        return t * t * (3 - 2 * t)

    def expect(x, y):
        u, v = (x + 0.5) / w, (y + 0.5) / h
        r, g, b = ramp(u, w), ramp(v, h), 0.5
        d = math.hypot((u - cx) / rx, (v - cy) / ry)
        k = 1 - smoothstep(1 - feather, 1, d)
        return (r + (max(r - 0.1, 0.0) - r) * k, g, b + (min(b * 1.8, 1.0) - b) * k)

    compare(out, expect)
    assert out[0][w - 1][2] == pytest.approx(0.5, abs=TOL), "outside the spot nothing changes"


@pytest.mark.parametrize("curve", [0.0, 1.0])
def test_screenxizer_spreads_the_edges_over_the_side_screens(tmp_path: Path, curve: float) -> None:
    w, h = 192, 36
    s, k = 0.25, 0.7
    out = gpu("cfa4-screenxizer-cylinder.glsl", {"side_width": s, "main_source": k, "curve": curve}, w, h, tmp_path)

    def src(u):
        if s <= u <= 1 - s:
            return 0.5 + (u - 0.5) / (1 - 2 * s) * k
        q = min(max((s - u) / s if u < s else (u - (1 - s)) / s, 0.0), 1.0)
        a = curve * math.pi / 2
        g = q if a < 1e-3 else math.sin(a * q) / math.sin(a)
        d = 0.5 * k + 0.5 * (1 - k) * g
        return 0.5 - d if u < s else 0.5 + d

    compare(out, lambda x, y: (ramp(src((x + 0.5) / w), w), ramp((y + 0.5) / h, h), 0.5))
    # the far ends show the picture's own edges
    assert out[h // 2][0][0] < 2 / 255 and out[h // 2][w - 1][0] > 253 / 255
