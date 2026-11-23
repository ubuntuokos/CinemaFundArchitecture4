#!/usr/bin/env python3
"""Realistic-bitrate VVC fixtures for crates/codecs/tests/vvc_perf.rs (ADR 0013, VVC-TEST-021).

The JVET conformance streams are stress streams (many at extreme bitrates), so the performance
condition is measured on content a camera or an export would make: the synthetic 10-bit test
picture of the VVC-00 measurement (mandelbrot blended with testsrc2, temporal noise), encoded by
VVenC at the pinned version with the presets the export profiles use.

    python3 scripts/dev/vvc-perf-fixture.py build      # VVenC into target/vvc-perf/vvenc (pinned)
    python3 scripts/dev/vvc-perf-fixture.py encode     # target/fixtures/vvc/perf-1080p-{faster,medium}.266 (+ 2160p faster)

VVenC (BSD-3-Clause-Clear) is a restricted component (cfa4-restricted-extras): it is built here only
as a test tool and never ships through this script. Requirements: git, cmake, a C++17 compiler,
ffmpeg (any version; the raw picture source only).
"""
from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
REPO = "https://github.com/fraunhoferhhi/vvenc.git"
TAG, COMMIT = "v1.14.0", "9428ea8636ae7f443ecde89999d16b2dfc421524"
WORK = ROOT / "target" / "vvc-perf"
OUT = ROOT / "target" / "fixtures" / "vvc"
SOURCE = (
    "mandelbrot=size={w}x{h}:rate=30,format=yuv420p10le[a];testsrc2=size={w}x{h}:rate=30,format=yuv420p10le[b];"
    "[a][b]blend=all_mode=average,noise=alls=6:allf=t"
)


def run(cmd: list[str], **kw) -> None:
    r = subprocess.run(cmd, check=False, capture_output=True, text=True, **kw)
    if r.returncode != 0:
        sys.exit(f"vvc-perf-fixture: {' '.join(cmd[:3])} failed:\n{r.stdout[-1500:]}\n{r.stderr[-1500:]}")


def app() -> Path:
    return WORK / "vvenc" / "bin" / "vvencapp"


def build(jobs: int) -> None:
    if app().is_file():
        return
    src, bld, inst = WORK / "src", WORK / "build", WORK / "vvenc"
    if not src.exists():
        src.mkdir(parents=True)
        run(["git", "init", "-q", str(src)])
        run(["git", "-C", str(src), "fetch", "-q", "--depth", "1", REPO, COMMIT])
        run(["git", "-C", str(src), "checkout", "-q", "FETCH_HEAD"])
    head = subprocess.run(["git", "-C", str(src), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    if head != COMMIT:
        sys.exit(f"vvc-perf-fixture: {src} is at {head}, expected {COMMIT} ({TAG})")
    run(["cmake", "-S", str(src), "-B", str(bld), "-DCMAKE_BUILD_TYPE=Release", "-DBUILD_SHARED_LIBS=OFF", f"-DCMAKE_INSTALL_PREFIX={inst}"])
    run(["cmake", "--build", str(bld), "--parallel", str(jobs), "--target", "install"])


def encode(w: int, h: int, frames: int, preset: str, qp: int, name: str) -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    out = OUT / name
    if out.is_file():
        return
    raw = WORK / f"src-{w}x{h}-{frames}.yuv"
    if not raw.is_file():
        run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i", SOURCE.format(w=w, h=h), "-frames:v", str(frames), "-pix_fmt", "yuv420p10le", "-f", "rawvideo", str(raw)])
    run([str(app()), "-i", str(raw), "-s", f"{w}x{h}", "-c", "yuv420_10", "--fps", "30", "-f", str(frames), "--preset", preset, "-q", str(qp), "-t", "4", "-o", str(out)])
    print(out)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd", choices=["build", "encode"])
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 2)
    a = ap.parse_args()
    WORK.mkdir(parents=True, exist_ok=True)
    build(a.jobs)
    if a.cmd == "encode":
        encode(1920, 1080, 300, "faster", 32, "perf-1080p-faster.266")
        encode(1920, 1080, 300, "medium", 32, "perf-1080p-medium.266")
        encode(3840, 2160, 64, "faster", 32, "perf-2160p-faster.266")
    return 0


if __name__ == "__main__":
    sys.exit(main())
