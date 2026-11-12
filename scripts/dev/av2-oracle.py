#!/usr/bin/env python3
"""AV2 oracle harness: pinned AVM build, corpus generation and manifest upkeep.

The committed corpus manifest (crates/testkit/data/av2-corpus.json) is the single source of
truth: it lists the pinned AVM commits, the synthetic sources, the avmenc arguments, and for
every vector the bitstream SHA-256 and the per-frame, per-plane SHA-256 of the avmdec v1.0.0
output. Media is never committed; everything this script makes goes under target/.

Subcommands:
    build     Build avmenc/avmdec at a pinned AVM commit into a private prefix
              (default target/av2-oracle/avm-<ref>; bin/ + AVM_COMMIT stamp). Never system-wide.
    corpus    Generate the synthetic sources (ffmpeg) and encode the vectors with the pinned
              avmenc into the fixture dir (default target/fixtures/av2/vectors), then check each
              bitstream against the manifest. A drifted bitstream is reported and removed, so the
              tests never compare against the wrong reference.
    manifest  Rebuild the manifest's vector hashes from an AV2-00 evidence file
              (docs/audit/av2-00-results.json), keeping everything else.

Usage:
    python3 scripts/dev/av2-oracle.py build [--ref v1.0.0|av2-normative] [--prefix DIR]
    python3 scripts/dev/av2-oracle.py corpus [--prefix DIR] [--out DIR] [--allow-drift]
    python3 scripts/dev/av2-oracle.py manifest --from docs/audit/av2-00-results.json

Then: CFA4_REQUIRE_AV2_ORACLE=1 cargo test -p cfa4-testkit av2
Requirements: git, cmake, ninja, nasm, a C/C++ compiler, ffmpeg (sources only).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "crates" / "testkit" / "data" / "av2-corpus.json"
AVM_REPO = "https://github.com/AOMediaCodec/avm.git"


def load_manifest() -> dict:
    return json.loads(MANIFEST.read_text(encoding="utf-8"))


def run(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, check=False, capture_output=True, text=True, **kw)


def die(msg: str) -> None:
    sys.exit(f"av2-oracle: {msg}")


def pin(manifest: dict, ref: str) -> dict:
    pins = {p["ref"]: p for p in [manifest["oracle"], *manifest.get("cross_check", [])]}
    if ref not in pins:
        die(f"unknown AVM ref {ref!r}; pinned: {', '.join(pins)}")
    return pins[ref]


def default_prefix(ref: str) -> Path:
    return ROOT / "target" / "av2-oracle" / f"avm-{ref}"


def cmd_build(args: argparse.Namespace) -> int:
    manifest = load_manifest()
    p = pin(manifest, args.ref)
    prefix = (args.prefix or default_prefix(args.ref)).resolve()
    stamp = prefix / "AVM_COMMIT"
    if stamp.is_file() and stamp.read_text().strip() == p["commit"] and (prefix / "bin" / "avmdec").is_file():
        print(f"AVM {args.ref} already built in {prefix}")
        return 0
    work = prefix / "work"
    src, build = work / "src", work / "build"
    if not src.exists():
        work.mkdir(parents=True, exist_ok=True)
        # Fetch exactly the pinned commit, whatever branch or tag points at it today.
        for cmd in (
            ["git", "init", "-q", str(src)],
            ["git", "-C", str(src), "fetch", "-q", "--depth", "1", AVM_REPO, p["commit"]],
            ["git", "-C", str(src), "checkout", "-q", "FETCH_HEAD"],
        ):
            r = run(cmd)
            if r.returncode != 0:
                die(f"{' '.join(cmd[:3])} failed: {r.stderr.strip()}")
    head = run(["git", "-C", str(src), "rev-parse", "HEAD"]).stdout.strip()
    if head != p["commit"]:
        die(f"{src} is at {head or 'unknown'}, expected {p['commit']}")
    build.mkdir(parents=True, exist_ok=True)
    for cmd in (
        ["cmake", str(src), "-G", "Ninja", "-DCMAKE_BUILD_TYPE=Release", "-DENABLE_TESTS=0", "-DENABLE_DOCS=0"],
        ["ninja", "avmenc", "avmdec"],
    ):
        r = run(cmd, cwd=build)
        if r.returncode != 0:
            die(f"{cmd[0]} failed:\n{r.stdout[-2000:]}\n{r.stderr[-2000:]}")
    (prefix / "bin").mkdir(parents=True, exist_ok=True)
    for exe in ("avmenc", "avmdec"):
        shutil.copy2(build / exe, prefix / "bin" / exe)
    stamp.write_text(p["commit"] + "\n")
    if not args.keep_work:
        shutil.rmtree(work, ignore_errors=True)
    print(f"AVM {args.ref} ({p['commit']}) -> {prefix / 'bin'}")
    return 0


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def cmd_corpus(args: argparse.Namespace) -> int:
    manifest = load_manifest()
    oracle = manifest["oracle"]
    prefix = (args.prefix or default_prefix(oracle["ref"])).resolve()
    stamp = prefix / "AVM_COMMIT"
    if not stamp.is_file() or stamp.read_text().strip() != oracle["commit"]:
        die(f"{prefix} holds no AVM {oracle['ref']} build; run the build subcommand first")
    avmenc = prefix / "bin" / "avmenc"
    out = (args.out or ROOT / "target" / "fixtures" / "av2" / "vectors").resolve()
    sources_dir = out.parent / "sources"
    out.mkdir(parents=True, exist_ok=True)
    sources_dir.mkdir(parents=True, exist_ok=True)

    sources = {}
    for name, s in manifest["sources"].items():
        path = sources_dir / f"{name}.y4m"
        if not path.exists():
            tmp = path.with_suffix(".tmp.y4m")
            r = run(
                [
                    "ffmpeg",
                    "-loglevel",
                    "error",
                    "-y",
                    "-f",
                    "lavfi",
                    "-i",
                    s["lavfi"],
                    "-frames:v",
                    str(s["frames"]),
                    "-pix_fmt",
                    s["pix_fmt"],
                    "-strict",
                    "-1",
                    str(tmp),
                ]
            )
            if r.returncode != 0:
                tmp.unlink(missing_ok=True)
                die(f"ffmpeg source {name} failed: {r.stderr.strip()}")
            tmp.replace(path)
        sources[name] = path

    drift = []
    for v in manifest["vectors"]:
        obu = out / f"{v['name']}.obu"
        if not (obu.exists() and sha256(obu) == v["bitstream_sha256"]):
            frames = manifest["sources"][v["source"]]["frames"]
            tmp = obu.with_suffix(".tmp.obu")
            r = run(
                [
                    str(avmenc),
                    *manifest["encoder_base_args"],
                    *v["encoder_args"],
                    f"--limit={frames}",
                    "--obu",
                    "-o",
                    str(tmp),
                    str(sources[v["source"]]),
                ]
            )
            if r.returncode != 0 or not tmp.exists():
                tmp.unlink(missing_ok=True)
                die(f"avmenc {v['name']} failed: {r.stderr[-800:]}")
            tmp.replace(obu)
        got = sha256(obu)
        if got == v["bitstream_sha256"]:
            print(f"{v['name']}: OK")
        else:
            drift.append(v["name"])
            print(f"{v['name']}: DRIFT bitstream {got} != manifest {v['bitstream_sha256']}")
            if not args.allow_drift:
                obu.unlink()
    if drift:
        print(
            f"{len(drift)} vector(s) differ from the manifest (ffmpeg or compiler differences in the "
            "sources/encoder); they are not usable as references until the manifest is re-validated."
        )
        return 0 if args.allow_drift else 1
    print(f"corpus: {out}")
    return 0


def cmd_manifest(args: argparse.Namespace) -> int:
    manifest = load_manifest()
    evidence = json.loads(Path(args.source).read_text(encoding="utf-8"))
    if evidence.get("avm", {}).get("commit") != manifest["oracle"]["commit"]:
        die("the evidence was not made with the pinned oracle commit")
    by_name = {e["vector"]: e for e in evidence["vectors"]}
    for v in manifest["vectors"]:
        e = by_name.get(v["name"])
        if not e or e.get("status") != "OK":
            die(f"no usable evidence for {v['name']}")
        if e["encoder_args"] != manifest["encoder_base_args"] + v["encoder_args"]:
            die(f"{v['name']}: evidence encoder arguments differ from the manifest")
        v["bitstream_sha256"] = e["bitstream_sha256"]
        v["bitstream_bytes"] = e["bitstream_bytes"]
        v["geometry"] = e["reference_geometry"]
        v["frames"] = e["reference_frame_sha256"]
    MANIFEST.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(f"updated {MANIFEST.relative_to(ROOT)}")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("build", help="build the pinned AVM into a private prefix")
    b.add_argument("--ref", default="v1.0.0", help="pinned ref from the manifest (v1.0.0 or av2-normative)")
    b.add_argument("--prefix", type=Path)
    b.add_argument("--keep-work", action="store_true", help="keep the source and build trees")
    c = sub.add_parser("corpus", help="generate and check the corpus bitstreams")
    c.add_argument("--prefix", type=Path, help="AVM v1.0.0 prefix (default target/av2-oracle/avm-v1.0.0)")
    c.add_argument("--out", type=Path)
    c.add_argument("--allow-drift", action="store_true", help="keep drifted bitstreams and exit 0")
    m = sub.add_parser("manifest", help="refresh vector hashes from AV2-00 evidence")
    m.add_argument("--from", dest="source", required=True)
    args = ap.parse_args()
    return {"build": cmd_build, "corpus": cmd_corpus, "manifest": cmd_manifest}[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main())
