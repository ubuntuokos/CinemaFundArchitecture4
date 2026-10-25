"""CFA4 OTIO-1.1: check CFA4-written OpenTimelineIO files with the official OTIO library.

The official ``opentimelineio`` Python package is used as a black-box test oracle, the way
ffmpeg/ffprobe are used for media: it is never linked into or shipped with CFA4
(docs/plans/OTIO-INTEGRATION-2026-10-24.md).

Input is a directory written by the Rust test ``oracle_fixtures``::

    CFA4_OTIO_ORACLE_DIR=target/fixtures/otio-oracle \\
        cargo test -p cfa4-interchange --test otio oracle_fixtures
    python -m cfa4_devtools.otio_oracle target/fixtures/otio-oracle

``manifest.json`` lists every exported file and what the CFA4 project holds: per track (video
tracks first, then audio, as exported) each clip's name, timeline start and duration in exact
engine ticks, and its marker count. For each file the oracle

1. reads it with ``opentimelineio`` (schema and JSON validity as OTIO sees them),
2. writes it back to a string and reads that again (the library can serialise what it read),
3. compares tracks, clip names, marker counts and each clip's position and length, computed by
   OTIO's own timing rules (``range_in_parent``), with the manifest.

Exit status: 0 every fixture passes, 1 at least one fails, 2 the oracle could not run
(``opentimelineio`` missing, unreadable or invalid manifest).
"""
from __future__ import annotations

import argparse
import json
import sys
from fractions import Fraction
from pathlib import Path
from typing import Any

#: Largest allowed difference between OTIO's and CFA4's time, in seconds. OTIO stores times as
#: floating-point frame counts, so exact equality is not expected; one microsecond is far below a
#: single audio sample at 48 kHz (about 20.8 µs).
TOLERANCE_SECONDS = Fraction(1, 1_000_000)
MAX_MANIFEST_BYTES = 4 * 1024 * 1024
MAX_FIXTURE_BYTES = 64 * 1024 * 1024
MAX_FIXTURES = 1000


class OracleError(RuntimeError):
    """The oracle cannot run at all (missing library, bad manifest)."""


def load_otio() -> Any:
    """Import the official library, or explain how to install it."""
    try:
        import opentimelineio  # optional development dependency, imported only when needed
    except ImportError as exc:
        raise OracleError(
            "the opentimelineio package is not installed; install the development extra: "
            "pip install -e '.[otio]'"
        ) from exc
    return opentimelineio


def load_manifest(directory: Path) -> dict[str, Any]:
    path = directory / "manifest.json"
    try:
        size = path.stat().st_size
    except OSError as exc:
        raise OracleError(f"{path}: {exc}") from exc
    if size > MAX_MANIFEST_BYTES:
        raise OracleError(f"{path}: manifest larger than {MAX_MANIFEST_BYTES} bytes")
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise OracleError(f"{path}: {exc}") from exc
    if not isinstance(manifest, dict) or manifest.get("version") != 1:
        raise OracleError(f"{path}: not a version 1 oracle manifest")
    tps = manifest.get("ticks_per_second")
    if not isinstance(tps, int) or isinstance(tps, bool) or tps <= 0:
        raise OracleError(f"{path}: ticks_per_second must be a positive integer")
    fixtures = manifest.get("fixtures")
    if not isinstance(fixtures, list) or not fixtures or len(fixtures) > MAX_FIXTURES:
        raise OracleError(f"{path}: fixtures must be a non-empty list of at most {MAX_FIXTURES}")
    for entry in fixtures:
        name = entry.get("file") if isinstance(entry, dict) else None
        # plain file names only: the manifest must not point outside its directory
        if not isinstance(name, str) or not name.endswith(".otio") or Path(name).name != name:
            raise OracleError(f"{path}: invalid fixture file name {name!r}")
    return manifest


def _seconds_of(rational_time: Any) -> Fraction:
    """Seconds of an OTIO RationalTime as an exact fraction of its (float) value and rate."""
    rate = Fraction(rational_time.rate)
    if rate <= 0:
        raise ValueError(f"non-positive rate {rational_time.rate!r}")
    return Fraction(rational_time.value) / rate


def observe(otio: Any, timeline: Any) -> dict[str, Any]:
    """The manifest-shaped view of an OTIO timeline, as OTIO itself computes it."""
    tracks = []
    for track in timeline.tracks:
        if not isinstance(track, otio.schema.Track):
            raise TypeError(f"top-level stack child {type(track).__name__} is not a Track")
        items = []
        for child in track:
            if isinstance(child, (otio.schema.Transition, otio.schema.Gap)):
                continue
            placed = child.range_in_parent()
            items.append({
                "name": child.name,
                "start": _seconds_of(placed.start_time),
                "duration": _seconds_of(placed.duration),
                # A nested sequence (Stack) keeps its clip-instance markers in CFA4 metadata, not
                # in OTIO markers, so the count is only comparable for clips.
                "markers": len(child.markers) if isinstance(child, otio.schema.Clip) else None,
            })
        tracks.append({"kind": str(track.kind), "name": track.name, "items": items})
    return {"name": timeline.name, "markers": len(timeline.tracks.markers), "tracks": tracks}


def compare(expected: dict[str, Any], observed: dict[str, Any], ticks_per_second: int,
            tolerance: Fraction = TOLERANCE_SECONDS) -> list[str]:
    """Differences between a manifest entry and an observed timeline (empty when they agree)."""
    problems: list[str] = []
    if expected.get("name") != observed.get("name"):
        problems.append(f"timeline name {observed.get('name')!r} != {expected.get('name')!r}")
    if expected.get("markers") != observed.get("markers"):
        problems.append(f"timeline markers {observed.get('markers')} != {expected.get('markers')}")
    exp_tracks = expected.get("tracks") or []
    obs_tracks = observed.get("tracks") or []
    if len(exp_tracks) != len(obs_tracks):
        problems.append(f"track count {len(obs_tracks)} != {len(exp_tracks)}")
    for ti, (et, ot) in enumerate(zip(exp_tracks, obs_tracks)):
        where = f"track {ti} ({et.get('kind')} {et.get('name')!r})"
        if et.get("kind") != ot.get("kind"):
            problems.append(f"{where}: kind {ot.get('kind')!r}")
        if et.get("name") != ot.get("name"):
            problems.append(f"{where}: name {ot.get('name')!r}")
        e_items, o_items = et.get("items") or [], ot.get("items") or []
        if len(e_items) != len(o_items):
            problems.append(f"{where}: {len(o_items)} items, expected {len(e_items)}")
        for ii, (ei, oi) in enumerate(zip(e_items, o_items)):
            at = f"{where} item {ii} ({ei.get('name')!r})"
            if ei.get("name") != oi.get("name"):
                problems.append(f"{at}: name {oi.get('name')!r}")
            for key in ("start", "duration"):
                want = Fraction(int(ei[f"{key}_ticks"]), ticks_per_second)
                got = oi[key]
                if abs(got - want) > tolerance:
                    problems.append(f"{at}: {key} {float(got):.9f} s != {float(want):.9f} s")
            if oi.get("markers") is not None and oi["markers"] != ei.get("markers"):
                problems.append(f"{at}: {oi['markers']} markers, expected {ei.get('markers')}")
    return problems


def check_fixture(otio: Any, directory: Path, entry: dict[str, Any], ticks_per_second: int) -> list[str]:
    path = directory / entry["file"]
    try:
        if path.stat().st_size > MAX_FIXTURE_BYTES:
            return [f"larger than {MAX_FIXTURE_BYTES} bytes"]
        timeline = otio.adapters.read_from_file(str(path))
        if not isinstance(timeline, otio.schema.Timeline):
            return [f"top-level object is {type(timeline).__name__}, not a Timeline"]
        again = otio.adapters.read_from_string(otio.adapters.write_to_string(timeline))
        problems = compare(entry, observe(otio, timeline), ticks_per_second)
        if not again.is_equivalent_to(timeline):
            problems.append("re-serialised timeline differs from the one read")
        return problems
    except Exception as exc:  # noqa: BLE001 - any library failure is a FAIL for this fixture
        return [f"opentimelineio could not process the file: {type(exc).__name__}: {exc}"]


def run(directory: Path) -> dict[str, Any]:
    otio = load_otio()
    manifest = load_manifest(directory)
    tps = manifest["ticks_per_second"]
    results = []
    for entry in manifest["fixtures"]:
        problems = check_fixture(otio, directory, entry, tps)
        results.append({"file": entry["file"], "status": "FAIL" if problems else "PASS", "problems": problems})
    return {
        "oracle": "opentimelineio",
        "oracle_version": getattr(otio, "__version__", "unknown"),
        "directory": str(directory),
        "result": "FAIL" if any(r["status"] == "FAIL" for r in results) else "PASS",
        "fixtures": results,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("directory", type=Path, help="directory with manifest.json and the .otio files")
    args = parser.parse_args(argv)
    try:
        report = run(args.directory)
    except OracleError as exc:
        print(f"otio_oracle: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(report, indent=2, ensure_ascii=False))
    return 0 if report["result"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
