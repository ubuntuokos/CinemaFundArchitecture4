#!/usr/bin/env python3
"""Build the out-of-process AV2 decoder helper (ADR 0010, A track).

Fetches dav2d at the commit pinned in packaging/av2-decode/dav2d.json, builds it as a static
library, and compiles packaging/av2-decode/cfa4-av2-decode.c against it into a private prefix
(default target/av2-decoder: bin/cfa4-av2-decode + DAV2D_COMMIT stamp). Nothing is installed
system-wide; packaging into /opt/cfa4 is plan step AV2-10.

Usage:
    python3 scripts/dev/av2-decoder.py build [--prefix DIR] [--keep-work]

Then: CFA4_REQUIRE_AV2_ORACLE=1 cargo test -p cfa4-codecs --features av2-external --test av2_external
Requirements: git, meson, ninja, nasm, a C compiler, pkg-config is not needed.
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
PIN = ROOT / "packaging" / "av2-decode" / "dav2d.json"
HELPER = ROOT / "packaging" / "av2-decode" / "cfa4-av2-decode.c"


def run(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, check=False, capture_output=True, text=True, **kw)


def die(msg: str) -> None:
    sys.exit(f"av2-decoder: {msg}")


def default_prefix() -> Path:
    return ROOT / "target" / "av2-decoder"


def cmd_build(args: argparse.Namespace) -> int:
    pin = json.loads(PIN.read_text(encoding="utf-8"))
    prefix = (args.prefix or default_prefix()).resolve()
    stamp = prefix / "DAV2D_COMMIT"
    exe = prefix / "bin" / "cfa4-av2-decode"
    helper_stamp = prefix / "HELPER_MTIME"
    helper_mtime = str(HELPER.stat().st_mtime_ns)
    if (
        stamp.is_file()
        and stamp.read_text().strip() == pin["commit"]
        and exe.is_file()
        and helper_stamp.is_file()
        and helper_stamp.read_text().strip() == helper_mtime
    ):
        print(f"cfa4-av2-decode already built in {prefix}")
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
    if not (inst / "lib").exists() and not (inst / "lib64").exists():
        shutil.rmtree(build, ignore_errors=True)
        for cmd, cwd in (
            (
                [
                    "meson",
                    "setup",
                    str(build),
                    str(src),
                    "--buildtype=release",
                    "-Ddefault_library=static",
                    "-Denable_tools=false",
                    "-Denable_tests=false",
                    f"--prefix={inst}",
                    "--libdir=lib",
                ],
                src,
            ),
            (["ninja", "-C", str(build), "install"], src),
        ):
            r = run(cmd, cwd=cwd)
            if r.returncode != 0:
                die(f"{cmd[0]} failed:\n{r.stdout[-2000:]}\n{r.stderr[-2000:]}")
    (prefix / "bin").mkdir(parents=True, exist_ok=True)
    cc = os.environ.get("CC", "cc")
    cmd = [
        cc,
        "-O2",
        "-std=c11",
        "-Wall",
        "-Wextra",
        "-Werror",
        f"-I{inst / 'include'}",
        str(HELPER),
        str(inst / "lib" / "libdav2d.a"),
        "-lpthread",
        "-lm",
        "-ldl",
        "-o",
        str(exe),
    ]
    r = run(cmd)
    if r.returncode != 0:
        die(f"compiling the helper failed:\n{r.stderr[-4000:]}")
    stamp.write_text(pin["commit"] + "\n")
    helper_stamp.write_text(helper_mtime + "\n")
    if not args.keep_work:
        shutil.rmtree(work, ignore_errors=True)
    print(f"cfa4-av2-decode (dav2d {pin['commit']}) -> {exe}")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("build", help="build dav2d and the helper into a private prefix")
    b.add_argument("--prefix", type=Path)
    b.add_argument("--keep-work", action="store_true", help="keep the dav2d checkout and build dir")
    b.set_defaults(func=cmd_build)
    args = ap.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
