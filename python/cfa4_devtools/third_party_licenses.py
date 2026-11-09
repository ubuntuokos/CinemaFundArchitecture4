"""Third-party licence notices for the CFA4 Linux packages (issue #220, the rest of #6).

The packages ship `usr/share/doc/cfa4/THIRD-PARTY-LICENSES.txt`: every Rust crate compiled into
`cfa4` and `cfa4-cli` that is not CFA4's own workspace code, with its licence expression and the
licence, copyright and notice files the crate itself ships. MIT, BSD, Apache-2.0 and similar
licences require exactly that in binary redistributions.

Input is `cargo metadata` for the packaged features; the crate sources must be on disk (they are
after `cargo build`, in the registry or in `vendor/`). The output is deterministic: crates sorted by
name and version, identical licence texts written once and referenced by number.

    cargo metadata --format-version 1 --locked --offline \
        --filter-platform x86_64-unknown-linux-gnu \
        --features cfa4/gstreamer,cfa4-cli/gstreamer > target/metadata.json
    python -m cfa4_devtools.third_party_licenses target/metadata.json \
        -o target/THIRD-PARTY-LICENSES.txt

`packaging/env.sh` (`copy_docs`) runs this for every package format. Without a licence file for a
crate the notice says so and names the licence from its manifest; `--strict` turns that into an
error. This is a notice file, not a legal review: `cargo deny check licenses` stays the gate on
which licences are allowed.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

ROOT_PACKAGES = ("cfa4", "cfa4-cli")
# Files a crate ships for its licence, copyright or attribution (LICENSE, LICENSE-MIT,
# LICENCE.txt, COPYING, NOTICE, COPYRIGHT, UNLICENSE, AUTHORS is not a licence and stays out).
LICENSE_FILE = re.compile(r"^(licen[cs]e|copying|notice|copyright|unlicense)([._-].*)?$", re.IGNORECASE)
# A crate's licence file is text; anything larger is not a licence (and would bloat the package).
MAX_FILE_BYTES = 256 * 1024
RULE = "=" * 78


@dataclass
class Crate:
    name: str
    version: str
    license: str
    repository: str
    authors: list[str]
    files: list[tuple[str, int]] = field(default_factory=list)  # (file name, text number)


class NoticeError(Exception):
    pass


def shipped_package_ids(metadata: dict[str, Any], roots: tuple[str, ...] = ROOT_PACKAGES) -> set[str]:
    """Package ids reachable from the root binaries through normal (non-dev, non-build) edges."""
    packages = {p["id"]: p for p in metadata.get("packages", [])}
    resolve = metadata.get("resolve") or {}
    nodes = {n["id"]: n for n in resolve.get("nodes", [])}
    members = set(metadata.get("workspace_members", []))
    start = [pid for pid in members if packages.get(pid, {}).get("name") in roots]
    missing = set(roots) - {packages[pid]["name"] for pid in start}
    if missing:
        raise NoticeError(f"root packages not in the workspace: {', '.join(sorted(missing))}")
    seen: set[str] = set()
    stack = list(start)
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes.get(pid, {}).get("deps", []):
            kinds = dep.get("dep_kinds") or [{"kind": None}]
            if any(k.get("kind") is None for k in kinds):
                stack.append(dep["pkg"])
    return seen


def license_files(manifest_path: Path, license_file: str | None) -> list[Path]:
    crate_dir = manifest_path.parent
    found: dict[str, Path] = {}
    if crate_dir.is_dir():
        for entry in crate_dir.iterdir():
            if entry.is_file() and LICENSE_FILE.match(entry.name):
                found[entry.name] = entry
    if license_file:
        extra = (crate_dir / license_file).resolve()
        if extra.is_file():
            found.setdefault(extra.name, extra)
    return [found[k] for k in sorted(found, key=str.lower)]


def read_text(path: Path) -> str:
    data = path.read_bytes()
    if len(data) > MAX_FILE_BYTES:
        raise NoticeError(f"{path}: {len(data)} bytes is too large for a licence file")
    text = data.decode("utf-8", errors="replace").replace("\r\n", "\n").replace("\r", "\n")
    return text.strip("\n") + "\n"


def collect(metadata: dict[str, Any], roots: tuple[str, ...] = ROOT_PACKAGES) -> tuple[list[Crate], list[str]]:
    """The shipped third-party crates and the unique licence texts they reference."""
    packages = {p["id"]: p for p in metadata.get("packages", [])}
    members = set(metadata.get("workspace_members", []))
    texts: list[str] = []
    index: dict[str, int] = {}
    crates: list[Crate] = []
    for pid in shipped_package_ids(metadata, roots):
        if pid in members:
            continue  # CFA4's own code: GPL-3.0-or-later, covered by LICENSE and copyright
        pkg = packages.get(pid)
        if pkg is None:
            raise NoticeError(f"package {pid} is in the resolve graph but not in packages")
        crate = Crate(
            name=pkg["name"],
            version=pkg["version"],
            license=(pkg.get("license") or "").strip() or "(no licence expression in Cargo.toml)",
            repository=(pkg.get("repository") or "").strip(),
            authors=[a for a in pkg.get("authors") or [] if a],
        )
        for path in license_files(Path(pkg["manifest_path"]), pkg.get("license_file")):
            text = read_text(path)
            digest = hashlib.sha256(text.encode()).hexdigest()
            if digest not in index:
                texts.append(text)
                index[digest] = len(texts)
            crate.files.append((path.name, index[digest]))
        crates.append(crate)
    crates.sort(key=lambda c: (c.name, _version_key(c.version)))
    return _renumber(crates, texts)


def _version_key(version: str) -> tuple[Any, ...]:
    core = version.split("+", 1)[0].split("-", 1)[0]
    return tuple(int(p) if p.isdigit() else 0 for p in core.split(".")) + (version,)


def _renumber(crates: list[Crate], texts: list[str]) -> tuple[list[Crate], list[str]]:
    """Number the texts in order of first use, so the output does not depend on traversal order."""
    order: dict[int, int] = {}
    for crate in crates:
        for _, number in crate.files:
            order.setdefault(number, len(order) + 1)
    new_texts = [""] * len(order)
    for old, new in order.items():
        new_texts[new - 1] = texts[old - 1]
    for crate in crates:
        crate.files = [(name, order[number]) for name, number in crate.files]
    return crates, new_texts


def render(crates: list[Crate], texts: list[str]) -> str:
    out = [
        "CFA4 – third-party licence notices",
        "",
        "The cfa4 and cfa4-cli programs in this package contain the following third-party",
        "Rust crates. Each entry gives the crate's licence (an SPDX expression from its",
        "Cargo.toml; with OR the recipient may choose) and the licence, copyright and notice",
        "files the crate ships; their texts follow, each written once.",
        "",
        "CFA4's own code is GPL-3.0-or-later (LICENSE). Code inherited from the upstream",
        "project keeps its MIT OR Apache-2.0 notices (NOTICE, LICENSE-MIT, LICENSE-APACHE).",
        "Fonts, icons and other assets are listed in ATTRIBUTION.md and copyright.",
        "",
        f"Crates: {len(crates)}. Licence texts: {len(texts)}.",
        "",
        RULE,
        "Crates",
        RULE,
        "",
    ]
    for crate in crates:
        out.append(f"{crate.name} {crate.version}")
        out.append(f"  Licence: {crate.license}")
        if crate.repository:
            out.append(f"  Source: {crate.repository}")
        if crate.authors:
            out.append(f"  Authors: {'; '.join(crate.authors)}")
        if crate.files:
            for name, number in crate.files:
                out.append(f"  {name}: text [{number}]")
        else:
            out.append("  The crate ships no licence file; its licence is the expression above.")
        out.append("")
    out += [RULE, "Licence texts", RULE, ""]
    for number, text in enumerate(texts, start=1):
        out.append(f"--- [{number}] " + "-" * (72 - len(str(number))))
        out.append("")
        out.append(text.rstrip("\n"))
        out.append("")
    return "\n".join(out).rstrip("\n") + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("metadata", type=Path, help="cargo metadata JSON (- for stdin)")
    parser.add_argument("-o", "--output", type=Path, required=True)
    parser.add_argument("--strict", action="store_true", help="fail when a crate ships no licence file")
    args = parser.parse_args(argv)
    try:
        raw = sys.stdin.read() if str(args.metadata) == "-" else args.metadata.read_text(encoding="utf-8")
        crates, texts = collect(json.loads(raw))
    except (OSError, ValueError, KeyError, NoticeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    without = [f"{c.name} {c.version}" for c in crates if not c.files]
    if without:
        print(f"note: {len(without)} crate(s) ship no licence file: {', '.join(without)}", file=sys.stderr)
        if args.strict:
            return 1
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(render(crates, texts), encoding="utf-8")
    print(f"wrote {args.output}: {len(crates)} crates, {len(texts)} licence texts")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
