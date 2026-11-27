"""CFA4's pinned FFmpeg version, read from packaging/ffmpeg-version.env.

That file is the one place the version is set (docs/plans/CFA4-FFMPEG-9.1-001.md, F0, R1); the
Python tools read it instead of writing the number themselves.
"""

from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
VERSION_FILE = ROOT / "packaging" / "ffmpeg-version.env"


def pinned(key: str = "CFA4_FFMPEG_VERSION", path: Path = VERSION_FILE) -> str:
    """The value of `key` (a `CFA4_FFMPEG_*` line) in the version file."""
    for line in path.read_text(encoding="utf-8").splitlines():
        name, sep, value = line.partition("=")
        if sep and name.strip() == key:
            return value.strip()
    raise KeyError(f"{key} is not set in {path}")


def pinned_version() -> str:
    """The FFmpeg version CFA4 builds, ships and tests against (MAJOR.MINOR.PATCH)."""
    return pinned("CFA4_FFMPEG_VERSION")


def pinned_prefix() -> Path:
    """Where scripts/dev/build-ffmpeg.sh and fetch-ffmpeg.sh install the base build."""
    return Path(f"/opt/cfa4/ffmpeg-{pinned_version()}")
