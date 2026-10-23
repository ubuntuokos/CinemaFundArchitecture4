"""Inspect the Linux Rust/Python/Qt6 developer environment without modifying it."""

from __future__ import annotations

import argparse
import importlib.util
import json
import platform
import shutil
import subprocess
import sys
from typing import Any


def version_of(command: list[str]) -> str | None:
    """Read a version string; a missing or hanging tool is not treated as success."""
    if shutil.which(command[0]) is None:
        return None
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=8, check=False)
    except (OSError, subprocess.TimeoutExpired):
        return None
    if result.returncode:
        return None
    return (result.stdout or result.stderr).strip() or None


def report() -> dict[str, Any]:
    """Return factual probes; all operations are read-only."""
    python_ok = sys.version_info >= (3, 12)
    qt_binding = importlib.util.find_spec("PySide6") is not None
    system_qt = version_of(["pkg-config", "--modversion", "Qt6Core"])
    return {
        "platform": platform.system(),
        "architecture": platform.machine(),
        "python": platform.python_version(),
        "python_ok": python_ok,
        "rustc": version_of(["rustc", "--version"]),
        "cargo": version_of(["cargo", "--version"]),
        "qt6_system": system_qt,
        "pyside6_installed": qt_binding,
        "cmake": version_of(["cmake", "--version"]),
        "ninja": version_of(["ninja", "--version"]),
        "ffmpeg": version_of(["ffmpeg", "-version"]),
        "ffprobe": version_of(["ffprobe", "-version"]),
    }


def missing_requirements(data: dict[str, Any]) -> list[str]:
    missing = []
    if data["platform"] != "Linux":
        missing.append("Linux")
    if data.get("architecture") != "x86_64":
        missing.append("x86_64 (AMD64)")
    if not data["python_ok"]:
        missing.append("Python >= 3.12")
    if data["rustc"] is None:
        missing.append("rustc")
    if data["cargo"] is None:
        missing.append("cargo")
    if data["qt6_system"] is None and not data["pyside6_installed"]:
        missing.append("Qt 6 (pkg-config Qt6Core or PySide6)")
    if data["ffmpeg"] is None:
        missing.append("ffmpeg")
    if data["ffprobe"] is None:
        missing.append("ffprobe")
    return missing


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", action="store_true", help="Machine-readable environment report")
    parser.add_argument("--strict", action="store_true", help="Fail when required tools are missing")
    options = parser.parse_args(argv)
    data = report()
    missing = missing_requirements(data)
    data["missing"] = missing
    if options.json:
        print(json.dumps(data, ensure_ascii=False, indent=2))
    else:
        for key, value in data.items():
            print(f"{key}: {value}")
    return 1 if options.strict and missing else 0


if __name__ == "__main__":
    raise SystemExit(main())
