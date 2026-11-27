#!/usr/bin/env python3
"""Coverage-guided fuzzing of VVdeC as the cfa4-vvc-decode helper drives it (ADR 0013, VVC-TEST-017).

Builds the pinned VVdeC (packaging/vvc-decode/vvdec.json) with clang, AddressSanitizer,
UndefinedBehaviorSanitizer and libFuzzer coverage (production code paths: not VVdeC's own
FUZZING_BUILD_MODE_UNSAFE_FOR_PRODUCTION), links packaging/vvc-decode/fuzz/vvdec_fuzz.c into a
libFuzzer binary, seeds a corpus from the JVET conformance streams (scripts/dev/vvc-conformance.py
fetch) and runs it.

    python3 scripts/dev/vvc-fuzz.py build [--prefix DIR] [--jobs N]
    python3 scripts/dev/vvc-fuzz.py seeds [--max-bytes N]        # corpus from the conformance streams
    python3 scripts/dev/vvc-fuzz.py run [--seconds N] [--jobs N] # fuzz; crashes land in <prefix>/artifacts

Everything lives under target/vvc-fuzz (never committed). A crash or timeout input is reproduced
with `target/vvc-fuzz/vvdec_fuzz <file>`; the fix goes to VVdeC upstream or into the helper's input
checks, with the input (or a minimal one) as a regression test in crates/codecs/tests/vvc_hostile.rs.
Requirements: git, cmake, clang (with the libFuzzer runtime).
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
HARNESS = ROOT / "packaging" / "vvc-decode" / "fuzz" / "vvdec_fuzz.c"
FIXTURES = Path(os.environ["CFA4_FIXTURES_DIR"]) if os.environ.get("CFA4_FIXTURES_DIR") else ROOT / "target" / "fixtures"
CORPUS_SRC = FIXTURES / "vvc-conformance"
SANITIZE = "-fsanitize=address,undefined -fno-sanitize-recover=undefined -fno-omit-frame-pointer"
COVERAGE = "-fsanitize=fuzzer-no-link"


def run(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, check=False, capture_output=True, text=True, **kw)


def die(msg: str) -> None:
    sys.exit(f"vvc-fuzz: {msg}")


def prefix_of(args: argparse.Namespace) -> Path:
    return (args.prefix or ROOT / "target" / "vvc-fuzz").resolve()


def cmd_build(args: argparse.Namespace) -> int:
    pin = json.loads(PIN.read_text(encoding="utf-8"))
    prefix = prefix_of(args)
    exe = prefix / "vvdec_fuzz"
    stamp = prefix / "STAMP"
    want = f"{pin['commit']} {HARNESS.stat().st_mtime_ns} {Path(__file__).stat().st_mtime_ns}"
    if exe.is_file() and stamp.is_file() and stamp.read_text().strip() == want:
        print(f"{exe} is up to date")
        return 0
    src, build, inst = prefix / "src", prefix / "build", prefix / "install"
    if not src.exists():
        prefix.mkdir(parents=True, exist_ok=True)
        for cmd in (
            ["git", "init", "-q", str(src)],
            ["git", "-C", str(src), "fetch", "-q", "--depth", "1", pin["repo"], pin["commit"]],
            ["git", "-C", str(src), "checkout", "-q", "FETCH_HEAD"],
        ):
            r = run(cmd)
            if r.returncode != 0:
                shutil.rmtree(src, ignore_errors=True)  # an unfinished checkout must not block the retry
                die(f"{' '.join(cmd[:3])} failed: {r.stderr.strip()}")
    if run(["git", "-C", str(src), "rev-parse", "HEAD"]).stdout.strip() != pin["commit"]:
        die(f"{src} is not at {pin['commit']}")
    shutil.rmtree(build, ignore_errors=True)
    # -DNDEBUG: the product build has VVdeC's debug-only checks (CHECKD, abort() in a debug build) compiled out;
    # with them on, the first of those stops the run before the memory errors behind it can show
    flags = f"{SANITIZE} {COVERAGE} -O1 -g -DNDEBUG"
    jobs = str(args.jobs or os.cpu_count() or 2)
    for cmd in (
        [
            "cmake", "-S", str(src), "-B", str(build), "-DCMAKE_BUILD_TYPE=RelWithDebInfo", "-DVVDEC_ENABLE_LINK_TIME_OPT=OFF",  # LTO objects are bitcode the fuzz link cannot use
            "-DCMAKE_C_COMPILER=clang", "-DCMAKE_CXX_COMPILER=clang++",
            f"-DCMAKE_C_FLAGS={flags}", f"-DCMAKE_CXX_FLAGS={flags}",
            "-DBUILD_SHARED_LIBS=OFF", "-DVVDEC_LIBRARY_ONLY=ON", "-DVVDEC_ENABLE_WERROR=OFF",
            f"-DCMAKE_INSTALL_PREFIX={inst}", "-DCMAKE_INSTALL_LIBDIR=lib",
        ],
        ["cmake", "--build", str(build), "--parallel", jobs],
        ["cmake", "--install", str(build)],
    ):
        r = run(cmd, cwd=src)
        if r.returncode != 0:
            die(f"{' '.join(cmd[:2])} failed:\n{r.stdout[-2000:]}\n{r.stderr[-2000:]}")
    r = run(
        ["clang++", "-O1", "-g", "-x", "c", "-std=c11", "-fsanitize=address,undefined,fuzzer", "-fno-sanitize-recover=undefined",
         f"-I{inst / 'include'}", str(HARNESS), "-x", "none", str(inst / "lib" / "libvvdec.a"), "-lpthread", "-o", str(exe)]
    )
    if r.returncode != 0:
        die(f"linking the fuzzer failed:\n{r.stderr[-4000:]}")
    stamp.write_text(want + "\n")
    print(f"libFuzzer binary: {exe}")
    return 0


def cmd_seeds(args: argparse.Namespace) -> int:
    prefix = prefix_of(args)
    out = prefix / "corpus"
    out.mkdir(parents=True, exist_ok=True)
    n = 0
    for f in sorted(CORPUS_SRC.glob("*/*")):
        if f.suffix.lower() in (".bit", ".bin", ".vvc", ".266") and 0 < f.stat().st_size <= args.max_bytes:
            shutil.copyfile(f, out / f"{f.parent.name}.bit")
            n += 1
    if not n:
        die(f"no conformance streams in {CORPUS_SRC}; run scripts/dev/vvc-conformance.py fetch")
    print(f"{n} seed streams in {out}")
    return 0


def cmd_run(args: argparse.Namespace) -> int:
    prefix = prefix_of(args)
    exe = prefix / "vvdec_fuzz"
    if not exe.is_file():
        die("not built: run `vvc-fuzz.py build` first")
    corpus, art = prefix / "corpus", prefix / "artifacts"
    corpus.mkdir(exist_ok=True)
    art.mkdir(exist_ok=True)
    cmd = [
        str(exe), str(corpus), f"-artifact_prefix={art}/", f"-max_total_time={args.seconds}", "-timeout=20",
        "-rss_limit_mb=4096", "-max_len=262144", f"-jobs={args.jobs}", f"-workers={args.jobs}", "-print_final_stats=1",
    ]
    env = dict(os.environ, ASAN_OPTIONS="detect_leaks=0:abort_on_error=0", UBSAN_OPTIONS="print_stacktrace=1")
    return subprocess.run(cmd, env=env, cwd=prefix).returncode


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("build")
    b.add_argument("--prefix", type=Path)
    b.add_argument("--jobs", type=int)
    b.set_defaults(func=cmd_build)
    s = sub.add_parser("seeds")
    s.add_argument("--prefix", type=Path)
    s.add_argument("--max-bytes", type=int, default=40000)
    s.set_defaults(func=cmd_seeds)
    r = sub.add_parser("run")
    r.add_argument("--prefix", type=Path)
    r.add_argument("--seconds", type=int, default=300)
    r.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 2) - 1))
    r.set_defaults(func=cmd_run)
    args = ap.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
