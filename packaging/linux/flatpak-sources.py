#!/usr/bin/env python3
"""The downloaded sources of a CFA4 Flatpak manifest, for an offline flatpak-builder run.

flatpak-bundle.sh builds with `flatpak-builder --disable-download`, so every `archive` or `file`
source with a `url` (the krb5 tarball of the bundle manifest) must already sit in the builder's
download cache, `<state-dir>/downloads/<sha256>/<file name>`. This helper puts it there from a
local directory of reviewed files (CFA4_FLATPAK_SOURCES), checking each one's SHA-256 against the
manifest, and never downloads anything itself:

  flatpak-sources.py list MANIFEST                  # "<sha256> <file name> <url>" per source
  flatpak-sources.py check MANIFEST SOURCES         # every source present with its SHA-256
  flatpak-sources.py seed MANIFEST SOURCES STATE    # check, then copy into STATE/downloads

Exit status 1 names each missing or mismatching file. The manifests are CFA4's own and use one
`key: value` per line, so a line parser is enough (the build hosts have no PyYAML).
"""

from __future__ import annotations

import hashlib
import re
import shutil
import sys
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlparse

_ENTRY = re.compile(r"^(\s*)- type:\s*(\S+)\s*$")
_KEY = re.compile(r"^\s*(url|sha256|dest-filename):\s*(\S+)\s*$")
_SHA256 = re.compile(r"^[0-9a-f]{64}$")


@dataclass(frozen=True)
class Source:
    url: str
    sha256: str
    name: str


def parse(manifest: Path) -> list[Source]:
    """The `archive` and `file` sources of MANIFEST that flatpak-builder would download."""
    sources: list[Source] = []
    kind: str | None = None
    fields: dict[str, str] = {}

    def close() -> None:
        if kind in ("archive", "file") and "url" in fields:
            url, sha = fields["url"], fields.get("sha256", "")
            if not _SHA256.match(sha):
                raise SystemExit(f"error: {manifest}: {url} has no 64-hex sha256")
            name = fields.get("dest-filename") or Path(urlparse(url).path).name
            sources.append(Source(url, sha, name))

    for line in manifest.read_text(encoding="utf-8").splitlines():
        entry = _ENTRY.match(line)
        if entry:
            close()
            kind, fields = entry.group(2), {}
            continue
        key = _KEY.match(line)
        if key and kind is not None:
            fields[key.group(1)] = key.group(2).strip("'\"")
    close()
    return sources


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def check(sources: list[Source], directory: Path) -> list[str]:
    """One message per source missing from DIRECTORY or with another SHA-256."""
    problems = []
    for source in sources:
        path = directory / source.name
        if not path.is_file():
            problems.append(f"{path} missing ({source.url}, sha256 {source.sha256})")
        elif (actual := _sha256(path)) != source.sha256:
            problems.append(f"{path}: sha256 {actual}, the manifest pins {source.sha256}")
    return problems


def seed(sources: list[Source], directory: Path, state: Path) -> None:
    for source in sources:
        dest = state / "downloads" / source.sha256 / source.name
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(directory / source.name, dest)
        print(f"seeded {source.name} ({source.sha256[:12]}…) for flatpak-builder --disable-download")


def main(argv: list[str]) -> int:
    usage = "usage: flatpak-sources.py list MANIFEST | check MANIFEST SOURCES | seed MANIFEST SOURCES STATE"
    if len(argv) < 2 or (argv[0], len(argv)) not in {("list", 2), ("check", 3), ("seed", 4)}:
        print(usage, file=sys.stderr)
        return 2
    sources = parse(Path(argv[1]))
    if argv[0] == "list":
        for source in sources:
            print(source.sha256, source.name, source.url)
        return 0
    directory = Path(argv[2])
    problems = check(sources, directory)
    for problem in problems:
        print(f"error: Flatpak source {problem}", file=sys.stderr)
    if problems:
        print("Put the reviewed files in CFA4_FLATPAK_SOURCES (flatpak-builder runs with --disable-download).",
              file=sys.stderr)
        return 1
    if argv[0] == "check":
        for source in sources:
            print(f"OK: Flatpak source {source.name}")
        return 0
    seed(sources, directory, Path(argv[3]))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
