#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""The pinned FFmpeg version is written in one place only (docs/plans/CFA4-FFMPEG-9.1-001.md, F0, R1).

`packaging/ffmpeg-version.env` sets it; scripts, workflows, packaging, build.rs and the Python
tools read it from there. This check fails when the version (CFA4_FFMPEG_VERSION or
CFA4_FFMPEG_MIN_VERSION) appears in any other tracked file, except the files listed in
`packaging/ffmpeg-version-allowlist.txt` (historical records, provenance of ported FFmpeg source,
test data), and when the old versioned script names still exist.

    python3 scripts/ci/check_ffmpeg_version.py [--root DIR]

What counts as a use of the version: a line that mentions FFmpeg (any case, also in names such
as `ffmpeg-X.Y.Z`, `CFA4_FFMPEG_…`) and has the version as a token of its own (not part of a longer
number: X.Y.Z does not match 1X.Y.Z or X.Y.Z1). Short versions such as 9.1 are common in other
meanings (section numbers, Auro-3D channel layouts), so lines without FFmpeg are not checked; plan ids such
as CFA4-FFMPEG-9.1-001 are not versions.
"""

from __future__ import annotations

import argparse
import fnmatch
import re
import subprocess
import sys
from pathlib import Path

VERSION_FILE = "packaging/ffmpeg-version.env"
ALLOWLIST_FILE = "packaging/ffmpeg-version-allowlist.txt"
PLAN_ID = re.compile(r"CFA4-FFMPEG-[0-9][0-9.]*-[0-9]+")
OLD_SCRIPTS = (
    "scripts/dev/fetch-ffmpeg-*.sh",
    "scripts/dev/build-ffmpeg-[0-9]*.sh",
    "scripts/ci/install-ffmpeg-[0-9]*.sh",
)


def read_versions(root: Path) -> list[str]:
    values: dict[str, str] = {}
    for line in (root / VERSION_FILE).read_text(encoding="utf-8").splitlines():
        name, sep, value = line.partition("=")
        if sep and name.strip() in ("CFA4_FFMPEG_VERSION", "CFA4_FFMPEG_MIN_VERSION"):
            values[name.strip()] = value.strip()
    if "CFA4_FFMPEG_VERSION" not in values:
        raise SystemExit(f"error: CFA4_FFMPEG_VERSION is not set in {VERSION_FILE}")
    return sorted(set(values.values()))


def read_allowlist(root: Path) -> list[str]:
    patterns = []
    for line in (root / ALLOWLIST_FILE).read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            patterns.append(line)
    return patterns


def allowed(path: str, patterns: list[str]) -> bool:
    return any(fnmatch.fnmatchcase(path, p) for p in patterns)


def tracked_files(root: Path) -> list[str]:
    out = subprocess.run(["git", "-C", str(root), "ls-files", "-z"], check=True, capture_output=True).stdout
    return [p for p in out.decode("utf-8").split("\0") if p]


def pattern_for(versions: list[str]) -> re.Pattern[str]:
    alt = "|".join(re.escape(v) for v in versions)
    return re.compile(rf"(?<![\w.§])n?(?:{alt})(?![\w]|\.\d)")


def uses_version(line: str, token: re.Pattern[str]) -> bool:
    line = PLAN_ID.sub("", line)
    return "ffmpeg" in line.lower() and token.search(line) is not None


def scan(root: Path, files: list[str], versions: list[str], patterns: list[str]) -> list[str]:
    token = pattern_for(versions)
    found = []
    for path in files:
        if path in (VERSION_FILE, ALLOWLIST_FILE) or allowed(path, patterns):
            continue
        full = root / path
        if not full.is_file():
            continue
        try:
            text = full.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        for number, line in enumerate(text.splitlines(), 1):
            if uses_version(line, token):
                found.append(f"{path}:{number}: {line.strip()[:160]}")
    return found


def old_script_names(files: list[str]) -> list[str]:
    return [p for p in files if any(fnmatch.fnmatchcase(p, g) for g in OLD_SCRIPTS)]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args(argv)
    root = args.root.resolve()
    versions = read_versions(root)
    files = tracked_files(root)
    problems = scan(root, files, versions, read_allowlist(root))
    old = old_script_names(files)
    for line in problems:
        print(f"error: FFmpeg version written outside {VERSION_FILE}: {line}", file=sys.stderr)
    for path in old:
        print(f"error: versioned script name (use the unversioned one): {path}", file=sys.stderr)
    if problems or old:
        print(f"Read the version from {VERSION_FILE}, or list a historical file in {ALLOWLIST_FILE}.", file=sys.stderr)
        return 1
    print(f"OK: FFmpeg {', '.join(versions)} is written only in {VERSION_FILE} (and the allowlisted files).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
