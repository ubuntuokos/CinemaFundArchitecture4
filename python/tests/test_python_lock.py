"""The hash-pinned Python lock stays in step with pyproject.toml (#6, F1-01)."""

from __future__ import annotations

import re
import tomllib
from pathlib import Path

from packaging.requirements import Requirement
from packaging.utils import canonicalize_name
from packaging.version import Version

ROOT = Path(__file__).resolve().parents[2]
LOCK = ROOT / "requirements" / "linux-amd64.txt"
OTIO_LOCK = ROOT / "requirements" / "otio-oracle.txt"
PIN_RE = re.compile(r"^([A-Za-z0-9][A-Za-z0-9._-]*)==(\S+)\s*\\?$")


def parse_lock(text: str) -> dict[str, tuple[str, int]]:
    """Map each pinned package to (version, number of hashes)."""
    pins: dict[str, tuple[str, int]] = {}
    current: str | None = None
    for line in text.splitlines():
        if match := PIN_RE.match(line):
            current = canonicalize_name(match.group(1))
            pins[current] = (match.group(2), 0)
        elif current and line.strip().startswith("--hash=sha256:"):
            version, count = pins[current]
            pins[current] = (version, count + 1)
        elif line and not line.startswith((" ", "#")):
            current = None
    return pins


def test_lock_pins_every_dev_and_qt_requirement_within_range() -> None:
    extras = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))["project"]["optional-dependencies"]
    pins = parse_lock(LOCK.read_text(encoding="utf-8"))
    for spec in extras["dev"] + extras["qt"]:
        requirement = Requirement(spec)
        name = canonicalize_name(requirement.name)
        assert name in pins, f"{name} missing from {LOCK.name}; run scripts/dev/lock-python.sh"
        assert requirement.specifier.contains(Version(pins[name][0])), f"{name} pin outside {requirement.specifier}"


def test_every_pin_has_a_hash() -> None:
    pins = parse_lock(LOCK.read_text(encoding="utf-8"))
    assert pins
    assert [name for name, (_, hashes) in pins.items() if hashes == 0] == []


def test_otio_oracle_lock_pins_the_otio_extra_with_hashes() -> None:
    # OTIO-1.3: the CI `otio-oracle` job installs the test oracle from its own hash-pinned lock.
    extras = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))["project"]["optional-dependencies"]
    pins = parse_lock(OTIO_LOCK.read_text(encoding="utf-8"))
    for spec in extras["otio"]:
        requirement = Requirement(spec)
        name = canonicalize_name(requirement.name)
        assert name in pins, f"{name} missing from {OTIO_LOCK.name}; run scripts/dev/lock-python.sh"
        assert requirement.specifier.contains(Version(pins[name][0])), f"{name} pin outside {requirement.specifier}"
    assert [name for name, (_, hashes) in pins.items() if hashes == 0] == []


def test_otio_stays_out_of_the_main_lock() -> None:
    # The oracle is a development tool only (owner decision 3 in the OTIO plan): not in the dev/Qt environment.
    assert "opentimelineio" not in parse_lock(LOCK.read_text(encoding="utf-8"))


def test_webengine_stays_in_the_lock() -> None:
    # Owner decision 2026-10-25: Qt WebEngine is needed later; the full PySide6 metapackage stays.
    assert "pyside6-webengine" in parse_lock(LOCK.read_text(encoding="utf-8"))


def test_parse_lock_ignores_hostile_lines() -> None:
    assert parse_lock("--hash=sha256:00\n==1\nfoo\n  --hash=sha256:11\n") == {}
