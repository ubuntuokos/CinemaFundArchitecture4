"""CFA4-RENDER-INTEGRATION-PLAN-001 F2: EXR compression fixtures for the OpenImageIO POC (H-OIIO-1).

The F0V ``exr_sequence`` fixture is written by ``ffmpeg``, whose encoder knows only none, RLE and
ZIP (F2 pre-registration, open point 4). The pre-registered OpenImageIO hypothesis also covers PIZ,
PXR24, B44(A) and DWA(A/B), so this script writes one procedural 32-bit float image in every
OpenEXR compression, deterministically (fixed seed), to ``<workspace>/target/fixtures/render-f2/``
(``CFA4_FIXTURES_DIR`` overrides the root). The media is original work and is never committed.

    python3 python/cfa4_devtools/render_f2_exr_fixtures.py list
    python3 python/cfa4_devtools/render_f2_exr_fixtures.py generate
    python3 python/cfa4_devtools/render_f2_exr_fixtures.py selfcheck

``selfcheck`` re-reads every file with OpenImageIO and prints the largest absolute pixel error
against the source. That is a check of the *fixtures* (a lossless codec must give 0), not an F2
result: no F2 measurement starts before the pre-registration is approved. The lossy codecs'
tolerance is not set here; the pre-registration fixes it from the baseline before measuring.

Needs the ``OpenImageIO`` and ``numpy`` Python modules, which are an F2 POC dependency and not a
CFA4 dependency (run them in a throw-away venv). When they are missing every step is ``SKIPPED``,
never PASS; ``CFA4_REQUIRE_ORACLES=1`` turns that into a failure. Standard library otherwise.
"""

from __future__ import annotations

import argparse
import importlib
import os
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
WIDTH, HEIGHT, CHANNELS = 640, 360, 3
SEED = 20261123

# OpenEXR compression name -> lossless for a 32-bit float image? PXR24 drops the low 8 mantissa bits
# of float and DWA is lossy. B44 acts only on half channels, so float passes through it unchanged:
# it is listed lossy (what the codec is), and the selfcheck prints the error actually measured.
CODECS: dict[str, bool] = {
    "none": True,
    "rle": True,
    "zips": True,
    "zip": True,
    "piz": True,
    "pxr24": False,
    "b44": False,
    "b44a": False,
    "dwaa": False,
    "dwab": False,
}


def fixtures_dir() -> Path:
    root = (
        Path(os.environ["CFA4_FIXTURES_DIR"]) if os.environ.get("CFA4_FIXTURES_DIR") else ROOT / "target" / "fixtures"
    )
    return root / "render-f2"


def output(codec: str) -> Path:
    return fixtures_dir() / f"exr_{codec}.exr"


def _modules() -> tuple[Any, Any] | None:
    try:
        return importlib.import_module("OpenImageIO"), importlib.import_module("numpy")
    except ImportError:
        return None


def source_pixels(np: Any) -> Any:
    """Smooth ramps, a hard edge, super-white values and seeded noise: what compressors differ on."""
    y, x = np.mgrid[0:HEIGHT, 0:WIDTH].astype("float32")
    img = np.stack([x / WIDTH, y / HEIGHT, (x + y) / (WIDTH + HEIGHT)], axis=-1)
    img[:, WIDTH // 2 :, :] *= 4.0  # values above 1.0 (scene-referred highlights)
    img[HEIGHT // 3 : 2 * HEIGHT // 3, WIDTH // 4 : WIDTH // 4 + 8, :] = 0.0  # a sharp edge
    noise = np.random.RandomState(SEED).standard_normal(img.shape).astype("float32") * 0.01
    return (img + noise).astype("float32")


def _skip(what: str) -> int:
    print(f"SKIPPED {what}: OpenImageIO and numpy Python modules are not installed (F2 POC dependency)")
    return 1 if os.environ.get("CFA4_REQUIRE_ORACLES") == "1" else 0


def generate() -> int:
    mods = _modules()
    if mods is None:
        return _skip("generate")
    oiio, np = mods
    src = source_pixels(np)
    fixtures_dir().mkdir(parents=True, exist_ok=True)
    failed = 0
    for codec in CODECS:
        buf = oiio.ImageBuf(oiio.ImageSpec(WIDTH, HEIGHT, CHANNELS, "float"))
        buf.set_pixels(oiio.ROI(), src)
        buf.specmod().attribute("compression", codec)
        ok = buf.write(str(output(codec)))
        print(f"{'OK     ' if ok else 'FAILED '}{output(codec).name}")
        failed += 0 if ok else 1
    return 1 if failed else 0


def selfcheck() -> int:
    mods = _modules()
    if mods is None:
        return _skip("selfcheck")
    oiio, np = mods
    src = source_pixels(np)
    bad = 0
    for codec, lossless in CODECS.items():
        path = output(codec)
        if not path.exists():
            print(f"MISSING {path.name} (run generate)")
            bad += 1
            continue
        buf = oiio.ImageBuf(str(path))
        got = buf.get_pixels(oiio.FLOAT)
        err = float(np.abs(got - src).max())
        used = buf.spec().get_string_attribute("compression")
        verdict = "OK" if (err == 0.0 if lossless else True) and used.split(":")[0] == codec else "FAIL"
        print(
            f"{verdict:7}{path.name} compression={used} max_abs_err={err:.6g} ({'lossless' if lossless else 'lossy'})"
        )
        bad += verdict == "FAIL"
    return 1 if bad else 0


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("command", choices=["list", "generate", "selfcheck"])
    args = ap.parse_args(argv)
    if args.command == "list":
        for codec, lossless in CODECS.items():
            print(f"{output(codec).name}\t{'lossless' if lossless else 'lossy'}")
        return 0
    return generate() if args.command == "generate" else selfcheck()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
