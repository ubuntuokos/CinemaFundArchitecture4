#!/usr/bin/env python3
"""AV2-00 audit harness: AVM v1.0.0 oracle, test corpus and decoder comparison.

Builds the AOM reference software (AVM) at the pinned v1.0.0 commit into a
private prefix, encodes a small synthetic corpus, decodes it with avmdec and
compares every output frame, plane by plane, with the candidate decoders
(rav2d, dav2d) when their binaries are given. Writes a JSON evidence file.

Nothing is installed system-wide and no media is written into the repository:
everything goes under --work (default: target/av2-00).

Usage:
    python3 scripts/dev/av2-00-audit.py [--work DIR] [--avm-build DIR]
                                        [--rav2d BIN] [--dav2d BIN] [--json FILE]

Requirements: git, cmake, ninja, nasm, a C/C++ compiler, ffmpeg (any version,
only used to generate synthetic Y4M sources).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import subprocess
import sys
import time
from pathlib import Path

AVM_REPO = "https://github.com/AOMediaCodec/avm.git"
AVM_TAG = "v1.0.0"
AVM_COMMIT = "966a7d7cd6fcf60360caf5dc413b2aeeb65e144d"

# source name -> (lavfi source, pixel format, frame count)
SOURCES = {
    "cif8": ("testsrc2=size=352x288:rate=30", "yuv420p", 10),
    "cif10": ("mandelbrot=size=352x288:rate=30", "yuv420p10le", 10),
    "cif444": ("testsrc2=size=352x288:rate=30", "yuv444p", 6),
    "cif400": ("testsrc2=size=352x288:rate=30", "gray", 6),
    "hd8": ("testsrc2=size=1920x1080:rate=30", "yuv420p", 3),
    "odd8": ("testsrc2=size=66x50:rate=30", "yuv420p", 6),
}

# vector name, source name, extra avmenc arguments, what the vector exercises
VECTORS = [
    ("420-8bit-default", "cif8", [], "4:2:0 8 bit, default tools, inter"),
    ("420-10bit-default", "cif10", ["--input-bit-depth=10", "--bit-depth=10"], "4:2:0 10 bit"),
    ("420-8bit-intra-only", "cif8", ["--kf-max-dist=0"], "every frame a key frame"),
    ("420-8bit-film-grain", "cif8", ["--denoise-noise-level=50"], "film grain synthesis"),
    ("420-8bit-tiles-2x2", "hd8", ["--tile-columns=1", "--tile-rows=1"], "1080p, 2x2 tiles"),
    ("420-8bit-odd-size", "odd8", [], "66x50, non-multiple-of-8 size"),
    ("444-8bit", "cif444", ["--profile=4"], "4:4:4 (Main_444_10_IP1)"),
    ("400-8bit", "cif400", ["--monochrome"], "4:0:0 monochrome"),
]

BASE_ENC = ["--good", "--cpu-used=5", "--end-usage=q", "--qp=150", "--lag-in-frames=0"]


def run(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, check=False, capture_output=True, text=True, **kw)


def tool_version(cmd: list[str]) -> str:
    try:
        out = run(cmd)
    except OSError:
        return "missing"
    text = (out.stdout or out.stderr).strip().splitlines()
    return text[0] if text else "unknown"


def build_avm(work: Path) -> Path:
    src = work / "avm"
    build = work / "avm-build"
    if not src.exists():
        r = run(["git", "clone", "--depth", "1", "--branch", AVM_TAG, AVM_REPO, str(src)])
        if r.returncode != 0:
            sys.exit(f"AVM clone failed: {r.stderr}")
    head = run(["git", "-C", str(src), "rev-parse", "HEAD"]).stdout.strip()
    if head != AVM_COMMIT:
        sys.exit(f"AVM {AVM_TAG} resolves to {head}, expected {AVM_COMMIT}")
    build.mkdir(parents=True, exist_ok=True)
    for cmd in (
        ["cmake", str(src), "-G", "Ninja", "-DCMAKE_BUILD_TYPE=Release", "-DENABLE_TESTS=0", "-DENABLE_DOCS=0"],
        ["ninja", "avmenc", "avmdec"],
    ):
        r = run(cmd, cwd=build)
        if r.returncode != 0:
            sys.exit(f"AVM build step {cmd[0]} failed:\n{r.stdout[-2000:]}\n{r.stderr[-2000:]}")
    return build


def make_sources(work: Path) -> dict[str, Path]:
    out = {}
    src_dir = work / "sources"
    src_dir.mkdir(parents=True, exist_ok=True)
    for name, (lavfi, pix_fmt, frames) in SOURCES.items():
        path = src_dir / f"{name}.y4m"
        if not path.exists():
            r = run(
                [
                    "ffmpeg",
                    "-loglevel",
                    "error",
                    "-y",
                    "-f",
                    "lavfi",
                    "-i",
                    lavfi,
                    "-frames:v",
                    str(frames),
                    "-pix_fmt",
                    pix_fmt,
                    "-strict",
                    "-1",
                    str(path),
                ]
            )
            if r.returncode != 0:
                sys.exit(f"ffmpeg source {name} failed: {r.stderr}")
        out[name] = path
    return out


def y4m_frames(path: Path) -> list[list[str]]:
    """Return per-frame lists of per-plane sha256 digests from a Y4M file."""
    data = path.read_bytes()
    header_end = data.find(b"\n")
    if header_end < 0:
        raise ValueError(f"{path}: not a Y4M file")
    params = data[:header_end].split()
    width = height = 0
    chroma = "420"
    for p in params[1:]:
        if p.startswith(b"W"):
            width = int(p[1:])
        elif p.startswith(b"H"):
            height = int(p[1:])
        elif p.startswith(b"C"):
            chroma = p[1:].decode()
    bps = 2 if ("p10" in chroma or "p12" in chroma or "p16" in chroma) else 1
    if chroma.startswith("mono"):
        planes = [width * height]
    elif chroma.startswith("444"):
        planes = [width * height] * 3
    elif chroma.startswith("422"):
        cw = (width + 1) // 2
        planes = [width * height, cw * height, cw * height]
    else:
        cw, ch = (width + 1) // 2, (height + 1) // 2
        planes = [width * height, cw * ch, cw * ch]
    planes = [n * bps for n in planes]
    frames = []
    pos = header_end + 1
    while pos < len(data):
        line_end = data.find(b"\n", pos)
        if line_end < 0 or not data[pos:line_end].startswith(b"FRAME"):
            break
        pos = line_end + 1
        digests = []
        for size in planes:
            digests.append(hashlib.sha256(data[pos : pos + size]).hexdigest())
            pos += size
        frames.append(digests)
    return frames


def decode(name: str, cmd: list[str], out: Path) -> dict:
    t0 = time.monotonic()
    r = run(cmd)
    elapsed = round(time.monotonic() - t0, 3)
    result = {"decoder": name, "command": " ".join(cmd), "exit_code": r.returncode, "seconds": elapsed}
    if r.returncode != 0 or not out.exists():
        result["status"] = "FAIL"
        result["stderr_tail"] = (r.stderr or "")[-800:]
        return result
    try:
        result["frames"] = y4m_frames(out)
    except (ValueError, OSError) as err:
        result["status"] = "FAIL"
        result["stderr_tail"] = str(err)
    return result


def compare(ref: list[list[str]], cand: list[list[str]]) -> dict:
    mismatched = [i for i, (a, b) in enumerate(zip(ref, cand)) if a != b]
    status = "PASS" if len(ref) == len(cand) and not mismatched else "FAIL"
    return {
        "status": status,
        "reference_frames": len(ref),
        "candidate_frames": len(cand),
        "mismatched_frames": mismatched[:20],
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--work", type=Path, default=Path("target/av2-00"))
    ap.add_argument("--avm-build", type=Path, help="existing AVM v1.0.0 build directory")
    ap.add_argument("--rav2d", help="rav2d CLI binary")
    ap.add_argument("--dav2d", help="dav2d CLI binary")
    ap.add_argument("--json", type=Path, help="evidence output (default: WORK/av2-00-results.json)")
    args = ap.parse_args()

    work = args.work.resolve()
    work.mkdir(parents=True, exist_ok=True)
    build = args.avm_build.resolve() if args.avm_build else build_avm(work)
    avmenc, avmdec = build / "avmenc", build / "avmdec"
    for exe in (avmenc, avmdec):
        if not os.access(exe, os.X_OK):
            sys.exit(f"missing {exe}")

    sources = make_sources(work)
    vec_dir = work / "vectors"
    dec_dir = work / "decoded"
    vec_dir.mkdir(exist_ok=True)
    dec_dir.mkdir(exist_ok=True)

    results = []
    for name, src, extra, purpose in VECTORS:
        _, _, frames = SOURCES[src]
        obu = vec_dir / f"{name}.obu"
        entry = {"vector": name, "purpose": purpose, "source": src, "frames": frames, "encoder_args": BASE_ENC + extra}
        t0 = time.monotonic()
        enc = run([str(avmenc), *BASE_ENC, *extra, f"--limit={frames}", "--obu", "-o", str(obu), str(sources[src])])
        entry["encode_seconds"] = round(time.monotonic() - t0, 3)
        if enc.returncode != 0 or not obu.exists():
            entry["status"] = "ENCODE_FAIL"
            entry["stderr_tail"] = (enc.stderr or "")[-800:]
            results.append(entry)
            print(f"{name}: ENCODE_FAIL", flush=True)
            continue
        entry["bitstream_sha256"] = hashlib.sha256(obu.read_bytes()).hexdigest()
        entry["bitstream_bytes"] = obu.stat().st_size

        ref_out = dec_dir / f"{name}.avmdec.y4m"
        ref = decode("avmdec", [str(avmdec), "-o", str(ref_out), str(obu)], ref_out)
        entry["reference"] = {k: v for k, v in ref.items() if k != "frames"}
        if "frames" not in ref:
            entry["status"] = "REFERENCE_FAIL"
            results.append(entry)
            print(f"{name}: REFERENCE_FAIL", flush=True)
            continue
        entry["reference_frame_sha256"] = ref["frames"]

        entry["candidates"] = {}
        for cand_name, binary, mk in (
            ("rav2d", args.rav2d, lambda b, o, i: [b, "-t", "1", "-o", o, i]),
            ("dav2d", args.dav2d, lambda b, o, i: [b, "-i", i, "-o", o]),
        ):
            if not binary:
                entry["candidates"][cand_name] = {"status": "SKIPPED", "reason": "binary not given"}
                continue
            out = dec_dir / f"{name}.{cand_name}.y4m"
            res = decode(cand_name, mk(binary, str(out), str(obu)), out)
            summary = {k: v for k, v in res.items() if k != "frames"}
            if "frames" in res:
                summary.update(compare(ref["frames"], res["frames"]))
            entry["candidates"][cand_name] = summary
        entry["status"] = "OK"
        results.append(entry)
        print(f"{name}: " + ", ".join(f"{k}={v.get('status')}" for k, v in entry["candidates"].items()), flush=True)

    evidence = {
        "audit": "AV2-00",
        "generated_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "host": {"os": platform.platform(), "machine": platform.machine(), "cpu_count": os.cpu_count()},
        "tools": {
            "cmake": tool_version(["cmake", "--version"]),
            "cc": tool_version(["cc", "--version"]),
            "ffmpeg": tool_version(["ffmpeg", "-version"]),
        },
        "avm": {"tag": AVM_TAG, "commit": AVM_COMMIT, "build_dir": str(build)},
        "candidates": {"rav2d": args.rav2d or None, "dav2d": args.dav2d or None},
        "vectors": results,
    }
    out_json = args.json or work / "av2-00-results.json"
    out_json.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    print(f"evidence: {out_json}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
