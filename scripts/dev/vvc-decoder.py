#!/usr/bin/env python3
"""Build the out-of-process VVC (H.266) decoder helper (ADR 0013, VVC-02).

Fetches VVdeC at the commit pinned in packaging/vvc-decode/vvdec.json, builds it as a static
library, and compiles packaging/vvc-decode/cfa4-vvc-decode.c against it into a private prefix
(default target/vvc-decoder: bin/cfa4-vvc-decode + VVDEC_COMMIT stamp). Nothing is installed
system-wide; packaging into the cfa4-restricted-extras package is plan step VVC-07.

Usage:
    python3 scripts/dev/vvc-decoder.py build [--prefix DIR] [--keep-work] [--jobs N]

Then: cargo test -p cfa4-codecs --features vvc-external --test vvc_external
Requirements: git, cmake (3.12+), a C11 and a C++17 compiler.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PIN = ROOT / "packaging" / "vvc-decode" / "vvdec.json"
HELPER = ROOT / "packaging" / "vvc-decode" / "cfa4-vvc-decode.c"


def run(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, check=False, capture_output=True, text=True, **kw)


def die(msg: str) -> None:
    sys.exit(f"vvc-decoder: {msg}")


def default_prefix() -> Path:
    return ROOT / "target" / "vvc-decoder"


def cmd_build(args: argparse.Namespace) -> int:
    pin = json.loads(PIN.read_text(encoding="utf-8"))
    prefix = (args.prefix or default_prefix()).resolve()
    stamp = prefix / "VVDEC_COMMIT"
    exe = prefix / "bin" / "cfa4-vvc-decode"
    helper_stamp = prefix / "HELPER_MTIME"
    helper_mtime = str(HELPER.stat().st_mtime_ns)
    if (
        stamp.is_file()
        and stamp.read_text().strip() == pin["commit"]
        and exe.is_file()
        and helper_stamp.is_file()
        and helper_stamp.read_text().strip() == helper_mtime
    ):
        print(f"cfa4-vvc-decode already built in {prefix}")
        return 0
    work = prefix / "work"
    src, build, inst = work / "src", work / "build", work / "install"
    if not src.exists():
        work.mkdir(parents=True, exist_ok=True)
        for cmd in (
            ["git", "init", "-q", str(src)],
            ["git", "-C", str(src), "fetch", "-q", "--depth", "1", pin["repo"], pin["commit"]],
            ["git", "-C", str(src), "checkout", "-q", "FETCH_HEAD"],
        ):
            r = run(cmd)
            if r.returncode != 0:
                die(f"{' '.join(cmd[:3])} failed: {r.stderr.strip()}")
    head = run(["git", "-C", str(src), "rev-parse", "HEAD"]).stdout.strip()
    if head != pin["commit"]:
        die(f"{src} is at {head or 'unknown'}, expected {pin['commit']}")
    if not (inst / "lib" / "libvvdec.a").exists():
        shutil.rmtree(build, ignore_errors=True)
        jobs = str(args.jobs or os.cpu_count() or 2)
        for cmd in (
            [
                "cmake",
                "-S",
                str(src),
                "-B",
                str(build),
                "-DCMAKE_BUILD_TYPE=Release",
                "-DBUILD_SHARED_LIBS=OFF",
                "-DVVDEC_LIBRARY_ONLY=ON",
                "-DVVDEC_ENABLE_WERROR=OFF",
                f"-DCMAKE_INSTALL_PREFIX={inst}",
                "-DCMAKE_INSTALL_LIBDIR=lib",
            ],
            ["cmake", "--build", str(build), "--parallel", jobs],
            ["cmake", "--install", str(build)],
        ):
            r = run(cmd, cwd=src)
            if r.returncode != 0:
                die(f"{' '.join(cmd[:2])} failed:\n{r.stdout[-2000:]}\n{r.stderr[-2000:]}")
    (prefix / "bin").mkdir(parents=True, exist_ok=True)
    cc = os.environ.get("CC", "cc")
    cxx = os.environ.get("CXX", "c++")
    obj = prefix / "cfa4-vvc-decode.o"
    r = run(
        [
            cc,
            "-O2",
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Werror",
            f"-I{inst / 'include'}",
            "-c",
            str(HELPER),
            "-o",
            str(obj),
        ]
    )
    if r.returncode != 0:
        die(f"compiling the helper failed:\n{r.stderr[-4000:]}")
    # VVdeC is C++ and built with link-time optimisation: link with the C++ driver and -flto.
    r = run([cxx, "-O2", "-flto=auto", str(obj), str(inst / "lib" / "libvvdec.a"), "-lpthread", "-lm", "-o", str(exe)])
    obj.unlink(missing_ok=True)
    if r.returncode != 0:
        die(f"linking the helper failed:\n{r.stderr[-4000:]}")
    stamp.write_text(pin["commit"] + "\n")
    helper_stamp.write_text(helper_mtime + "\n")
    if not args.keep_work:
        shutil.rmtree(work, ignore_errors=True)
    print(f"cfa4-vvc-decode (VVdeC {pin['version']} {pin['commit']}) -> {exe}")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("build", help="build VVdeC and the helper into a private prefix")
    b.add_argument("--prefix", type=Path)
    b.add_argument("--keep-work", action="store_true", help="keep the VVdeC checkout and build dir")
    b.add_argument("--jobs", type=int, help="parallel build jobs (default: all CPUs)")
    b.set_defaults(func=cmd_build)
    args = ap.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
