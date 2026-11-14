#!/usr/bin/env python3
"""Build the out-of-process AV2 decoder helper (ADR 0010, A track).

Fetches dav2d at the commit pinned in packaging/av2-decode/dav2d.json, builds it as a static
library, and compiles packaging/av2-decode/cfa4-av2-decode.c against it into a private prefix
(default target/av2-decoder: bin/cfa4-av2-decode + DAV2D_COMMIT stamp). Nothing is installed
system-wide; packaging into /opt/cfa4 is plan step AV2-10.

`build --sanitize` makes the same helper with AddressSanitizer and UndefinedBehaviorSanitizer
(dav2d and the helper both) into target/av2-decoder-asan; it is for fuzzing only, never shipped.
`fuzz` feeds mutated corpus units, garbage and request sequences to a helper and fails on a
sanitizer report, a crash by signal or a hang (G1: fuzzing of the native helper). Inputs that
fail are kept in target/av2-fuzz/failures.

Usage:
    python3 scripts/dev/av2-decoder.py build [--prefix DIR] [--keep-work] [--sanitize]
    python3 scripts/dev/av2-decoder.py fuzz [--helper EXE] [--rounds N] [--seed N] [--all]
    python3 scripts/dev/av2-decoder.py replay target/av2-fuzz/failures/<name>.requests

Then: CFA4_REQUIRE_AV2_ORACLE=1 cargo test -p cfa4-codecs --features av2-external --test av2_external
Requirements: git, meson, ninja, nasm, a C compiler, pkg-config is not needed.
"""

from __future__ import annotations

import argparse
import json
import os
import random
import shutil
import struct
import subprocess
import sys
import threading
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PIN = ROOT / "packaging" / "av2-decode" / "dav2d.json"
HELPER = ROOT / "packaging" / "av2-decode" / "cfa4-av2-decode.c"


def run(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, check=False, capture_output=True, text=True, **kw)


def die(msg: str) -> None:
    sys.exit(f"av2-decoder: {msg}")


def default_prefix(sanitize: bool = False) -> Path:
    return ROOT / "target" / ("av2-decoder-asan" if sanitize else "av2-decoder")


SANITIZE_FLAGS = ["-fsanitize=address,undefined", "-fno-sanitize-recover=undefined", "-fno-omit-frame-pointer", "-g"]


def cmd_build(args: argparse.Namespace) -> int:
    pin = json.loads(PIN.read_text(encoding="utf-8"))
    prefix = (args.prefix or default_prefix(args.sanitize)).resolve()
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
                    "--buildtype=debugoptimized" if args.sanitize else "--buildtype=release",
                    *(["-Db_sanitize=address,undefined", "-Db_lundef=false"] if args.sanitize else []),
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
        *(SANITIZE_FLAGS if args.sanitize else []),
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


# ---- fuzzing -------------------------------------------------------------------------------

SANITIZER_MARKS = (
    "ERROR: AddressSanitizer",
    "ERROR: LeakSanitizer",
    "runtime error:",
    "SUMMARY: UndefinedBehaviorSanitizer",
)
REQUEST_TIMEOUT = 20.0


class HelperDied(Exception):
    pass


class Helper:
    """One helper process speaking the frame protocol v1 (see cfa4-av2-decode.c)."""

    def __init__(self, exe: Path) -> None:
        env = dict(os.environ)
        env.setdefault("ASAN_OPTIONS", "detect_leaks=1:abort_on_error=0:exitcode=86")
        env.setdefault("UBSAN_OPTIONS", "halt_on_error=1:print_stacktrace=1:exitcode=87")
        self.p = subprocess.Popen(
            [str(exe)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env
        )
        self.stderr: list[bytes] = []
        threading.Thread(target=lambda: self.stderr.extend(iter(self.p.stderr.readline, b"")), daemon=True).start()
        hello = self._read(16)
        if hello[:8] != b"CFA4AV2D":
            raise HelperDied("bad hello")
        self._read(struct.unpack("<I", hello[12:16])[0])

    def _read(self, n: int) -> bytes:
        box: list[bytes] = []
        t = threading.Thread(target=lambda: box.append(self.p.stdout.read(n)), daemon=True)
        t.start()
        t.join(REQUEST_TIMEOUT)
        if t.is_alive():
            self.p.kill()
            raise TimeoutError(f"no reply within {REQUEST_TIMEOUT} s")
        if not box or len(box[0]) != n:
            raise HelperDied("helper closed its output")
        return box[0]

    def request(self, tag: bytes, payload: bytes) -> list[bytes]:
        try:
            self.p.stdin.write(tag + struct.pack("<I", len(payload)) + payload)
            self.p.stdin.flush()
        except (BrokenPipeError, OSError) as err:
            raise HelperDied(str(err)) from err
        tags = []
        while True:
            head = self._read(8)
            body = self._read(struct.unpack("<I", head[4:])[0])
            tags.append(head[:4])
            if head[:4] == b"DONE":
                return tags
            if head[:4] not in (b"FRME", b"ERRO"):
                raise HelperDied(f"unknown reply {head[:4]!r}")
            del body

    def close(self) -> tuple[int, str]:
        try:
            self.p.stdin.close()
        except OSError:
            pass
        try:
            code = self.p.wait(REQUEST_TIMEOUT)
        except subprocess.TimeoutExpired:
            self.p.kill()
            code = self.p.wait()
        return code, b"".join(self.stderr).decode("utf-8", "replace")


def temporal_units(data: bytes) -> list[bytes]:
    """Split an Annex B stream at its temporal delimiters (OBU type 2)."""
    starts, pos = [], 0
    while pos < len(data):
        size, shift, start = 0, 0, pos
        while pos < len(data):
            b = data[pos]
            pos += 1
            size |= (b & 0x7F) << shift
            shift += 7
            if b < 0x80 or shift > 56:
                break
        if pos < len(data) and (data[pos] >> 2) & 31 == 2:
            starts.append(start)
        pos += size
    starts.append(len(data))
    return [data[a:b] for a, b in zip(starts, starts[1:], strict=False) if b > a]


def mutate(rng: random.Random, unit: bytes, units: list[bytes]) -> bytes:
    u = bytearray(unit)
    kind = rng.randrange(6)
    if kind == 0 and u:  # bit flips
        for _ in range(1 + rng.randrange(8)):
            u[rng.randrange(len(u))] ^= 1 << rng.randrange(8)
    elif kind == 1 and u:  # special bytes
        for _ in range(1 + rng.randrange(4)):
            u[rng.randrange(len(u))] = rng.choice((0x00, 0x7F, 0x80, 0xFF))
    elif kind == 2:  # truncation
        del u[rng.randrange(len(u) + 1) :]
    elif kind == 3 and units:  # splice with another unit
        other = rng.choice(units)
        cut = rng.randrange(len(u) + 1)
        u = u[:cut] + other[rng.randrange(len(other) + 1) :]
    elif kind == 4:  # random garbage
        u = bytearray(rng.randbytes(rng.randrange(1, 4096)))
    else:  # duplicated tail
        u += u[rng.randrange(len(u) + 1) :]
    return bytes(u)


MANIFEST = ROOT / "crates" / "testkit" / "data" / "av2-corpus.json"
SCOPE = "Main_420_10_IP0"


def corpus_streams(corpus: Path, everything: bool) -> list[tuple[str, bytes]]:
    """The corpus bitstreams; by default only the committed decoding scope (plan II.3)."""
    vectors = json.loads(MANIFEST.read_text(encoding="utf-8"))["vectors"]
    names = [v["name"] for v in vectors if everything or v["profile"] == SCOPE]
    return [(f"{n}.obu", (corpus / f"{n}.obu").read_bytes()) for n in names if (corpus / f"{n}.obu").is_file()]


def run_script(exe: Path, script: list[tuple[bytes, bytes]]) -> tuple[str | None, str]:
    """Send one request sequence to a fresh helper; return (problem or None, stderr)."""
    problem = None
    try:
        h = Helper(exe)
    except (HelperDied, TimeoutError) as err:
        die(f"{exe} does not start: {err}")
    try:
        for tag, payload in script:
            h.request(tag, payload)
    except TimeoutError as err:
        problem = f"hang: {err}"
    except HelperDied as err:
        problem = f"helper died: {err}"
    code, stderr = h.close()
    if problem is None and code != 0:
        problem = f"exit status {code}"
    if any(m in stderr for m in SANITIZER_MARKS):
        problem = f"sanitizer report (exit {code})"
    return problem, stderr


def cmd_replay(args: argparse.Namespace) -> int:
    exe = (args.helper or default_prefix(True) / "bin" / "cfa4-av2-decode").resolve()
    data, pos, script = args.requests.read_bytes(), 0, []
    while pos + 8 <= len(data):
        n = struct.unpack("<I", data[pos + 4 : pos + 8])[0]
        script.append((data[pos : pos + 4], data[pos + 8 : pos + 8 + n]))
        pos += 8 + n
    problem, stderr = run_script(exe, script)
    print("\n".join(stderr.splitlines()[-40:]))
    print(f"{args.requests.name}: {len(script)} requests, {problem or 'ok'}")
    return 1 if problem else 0


def cmd_fuzz(args: argparse.Namespace) -> int:
    exe = (args.helper or default_prefix(True) / "bin" / "cfa4-av2-decode").resolve()
    if not exe.is_file():
        die(f"{exe} missing; run: build --sanitize")
    streams = corpus_streams(args.corpus.resolve(), args.all)
    if not streams:
        die(f"no .obu files in {args.corpus}; run scripts/dev/av2-oracle.py corpus")
    failures_dir = ROOT / "target" / "av2-fuzz" / "failures"
    rng = random.Random(args.seed)
    failures = 0
    for rnd in range(args.rounds):
        name, data = streams[rnd % len(streams)]
        units = temporal_units(data)
        script: list[tuple[bytes, bytes]] = []
        for i, unit in enumerate(units):
            if rng.random() < 0.6:
                unit = mutate(rng, unit, units)
            script.append((b"DATA", struct.pack("<q", i) + unit))
            r = rng.random()
            if r < 0.05:
                script.append((b"RSET", b""))
            elif r < 0.10:
                script.append((b"FLSH", b""))
        script.append((b"FLSH", b""))
        problem, stderr = run_script(exe, script)
        if problem:
            failures += 1
            failures_dir.mkdir(parents=True, exist_ok=True)
            base = failures_dir / f"round{rnd:05d}-{name}"
            base.with_suffix(".requests").write_bytes(b"".join(t + struct.pack("<I", len(p)) + p for t, p in script))
            base.with_suffix(".stderr.txt").write_text(stderr, encoding="utf-8")
            print(f"round {rnd} ({name}): {problem}; input kept in {base}.requests")
            print("\n".join(stderr.splitlines()[-30:]))
        elif rnd % 50 == 0:
            print(f"round {rnd} ({name}): ok")
    print(f"fuzz: {args.rounds} rounds over {len(streams)} streams, seed {args.seed}, {failures} failure(s)")
    return 1 if failures else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("build", help="build dav2d and the helper into a private prefix")
    b.add_argument("--prefix", type=Path)
    b.add_argument("--keep-work", action="store_true", help="keep the dav2d checkout and build dir")
    b.add_argument("--sanitize", action="store_true", help="ASan+UBSan build into target/av2-decoder-asan")
    b.set_defaults(func=cmd_build)
    f = sub.add_parser("fuzz", help="fuzz a helper (default: the sanitized build) with mutated corpus units")
    f.add_argument("--helper", type=Path)
    f.add_argument("--corpus", type=Path, default=ROOT / "target" / "fixtures" / "av2" / "vectors")
    f.add_argument("--rounds", type=int, default=300)
    f.add_argument("--seed", type=int, default=20261114)
    f.add_argument("--all", action="store_true", help=f"every corpus vector, not only {SCOPE}")
    f.set_defaults(func=cmd_fuzz)
    p = sub.add_parser("replay", help="send a kept failure's request sequence to a helper again")
    p.add_argument("requests", type=Path)
    p.add_argument("--helper", type=Path)
    p.set_defaults(func=cmd_replay)
    args = ap.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
