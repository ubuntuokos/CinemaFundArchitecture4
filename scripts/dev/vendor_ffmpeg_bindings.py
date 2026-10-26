#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Vendors the safe FFmpeg bindings of the `cfa4-ffmpeg` backend (ADR 0005) into
`vendor/ffmpeg-bindings/`, byte-for-byte from pinned upstream commits, with the provenance ledger
`python/cfa4_devtools/dependency_audit.py` verifies (F1-01/F1-02: no Cargo Git sources).

    python3 scripts/dev/vendor_ffmpeg_bindings.py /path/to/checkouts

`/path/to/checkouts` holds a Git checkout of each upstream (`rsmedia`, `rsmpeg`, `rusty_ffmpeg`)
at any revision; the script reads the pinned commit with `git show` and never builds anything.
Copied: the Rust sources, the build script, the licence texts and the README. Not copied: tests,
examples, benches, media assets and fonts (rsmedia's tests embed a proprietary font). Each
crate's Cargo.toml is rewritten: path dependencies between the three, no dev-dependencies, no
example or bench targets; the original manifest's Git blob ID is recorded.
"""
from __future__ import annotations

import hashlib
import json
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEST = ROOT / "vendor" / "ffmpeg-bindings"

# name: (upstream repository, pinned commit, copied top-level entries)
CRATES = {
    "rusty_ffmpeg": ("https://github.com/phial3/rusty_ffmpeg", "045c0262cea4f83d504d5c588e2e98e03655e7bb",
                     ["src", "build.rs", "LICENSE", "README.md"]),
    "rsmpeg": ("https://github.com/phial3/rsmpeg", "709e97446315eab89cd200452e4ef08cc7554fbe",
               ["src", "LICENSE", "README.md"]),
    "rsmedia": ("https://github.com/dromara/rsmedia", "eb0dbd0a702ea6be59fa70035ec358ce9d9888d3",
                ["src", "LICENSE-APACHE.md", "LICENSE-MIT.md", "LICENSE.md", "README.md"]),
}
# path dependencies that replace the upstream Git ones
PATHS = {"rusty_ffmpeg": "../rusty_ffmpeg", "rsmpeg": "../rsmpeg"}


def git(repo: Path, *args: str) -> bytes:
    return subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True).stdout


def blob(data: bytes) -> str:
    return hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()


def toml_value(v: object) -> str:
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (int, float)):
        return str(v)
    if isinstance(v, str):
        return json.dumps(v, ensure_ascii=False)
    if isinstance(v, list):
        return "[" + ", ".join(toml_value(x) for x in v) + "]"
    if isinstance(v, dict):
        return "{ " + ", ".join(f"{k} = {toml_value(x)}" for k, x in v.items()) + " }"
    raise TypeError(v)


def manifest(name: str, original: dict) -> str:
    """The adapted Cargo.toml: package metadata, library, features and normal/build dependencies."""
    pkg = {k: v for k, v in original["package"].items() if k not in ("exclude", "include", "documentation")}
    lines = [f"# Vendored by scripts/dev/vendor_ffmpeg_bindings.py from {CRATES[name][0]} @ {CRATES[name][1]}.",
             "# Adapted: Git dependencies replaced by local paths; tests, examples and benches not vendored.",
             "[package]"]
    lines += [f"{k} = {toml_value(v)}" for k, v in pkg.items() if not isinstance(v, dict)]
    lines.append("publish = false")
    if "lib" in original:
        lines += ["", "[lib]"] + [f"{k} = {toml_value(v)}" for k, v in original["lib"].items()]
    for table in ("dependencies", "build-dependencies"):
        deps = original.get(table) or {}
        if deps:
            lines += ["", f"[{table}]"]
            for dep, spec in deps.items():
                if isinstance(spec, dict) and "git" in spec:
                    spec = {k: v for k, v in spec.items() if k not in ("git", "branch", "rev", "tag")}
                    spec = {"path": PATHS[dep], **spec}
                lines.append(f"{dep} = {toml_value(spec)}")
    for target, spec in (original.get("target") or {}).items():
        for table in ("dependencies", "build-dependencies"):
            if spec.get(table):
                lines += ["", f"[target.{toml_value(target)}.{table}]"]
                lines += [f"{d} = {toml_value(s)}" for d, s in spec[table].items()]
    if original.get("features"):
        lines += ["", "[features]"] + [f"{k} = {toml_value(v)}" for k, v in original["features"].items()]
    return "\n".join(lines) + "\n"


def vendor(name: str, checkout: Path) -> None:
    url, rev, entries = CRATES[name]
    out = DEST / name
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    listing = git(checkout, "ls-tree", "-r", "--name-only", rev).decode().splitlines()
    files: dict[str, str] = {}
    for path in listing:
        top = path.split("/", 1)[0]
        if top not in entries:
            continue
        data = git(checkout, "show", f"{rev}:{path}")
        target = out / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        files[path] = blob(data)
    original_bytes = git(checkout, "show", f"{rev}:Cargo.toml")
    (out / "Cargo.toml").write_text(manifest(name, tomllib.loads(original_bytes.decode())), encoding="utf-8")
    ledger = {
        "source_repository": url,
        "source_commit": rev,
        "original_crate_manifest_blob_sha1": blob(original_bytes),
        "note": "Sources, build script, licences and README copied byte-for-byte from the pinned upstream commit by "
                "scripts/dev/vendor_ffmpeg_bindings.py; Cargo.toml adapted (Git dependencies to local paths, no "
                "tests, examples or benches). Used only by cfa4-ffmpeg with its `ffmpeg` feature (ADR 0005).",
        "files": dict(sorted(files.items())),
    }
    (out / "SOURCE_MANIFEST.json").write_text(json.dumps(ledger, indent=2) + "\n", encoding="utf-8")
    print(f"{name}: {len(files)} files from {url} @ {rev}")


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__.strip().splitlines()[4], file=sys.stderr)
        return 2
    base = Path(sys.argv[1])
    for name in CRATES:
        checkout = base / name
        if not (checkout / ".git").exists():
            print(f"missing checkout: {checkout}", file=sys.stderr)
            return 1
        if not re.fullmatch(r"[0-9a-f]{40}", CRATES[name][1]):
            return 1
        vendor(name, checkout)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
