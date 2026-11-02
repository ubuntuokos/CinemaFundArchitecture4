"""CFA4 OTIO-1.1: the OpenTimelineIO oracle.

The comparison and manifest checks run everywhere. The tests that need the official
``opentimelineio`` package skip when it is not installed (``pip install -e '.[otio]'``). When
``CFA4_OTIO_ORACLE_DIR`` points at fixtures written by the Rust test ``oracle_fixtures``, those are
checked too.
"""
from __future__ import annotations

import importlib.util
import json
import os
import tempfile
import unittest
from fractions import Fraction
from pathlib import Path
from unittest.mock import patch

from cfa4_devtools import otio_oracle as oracle

HAVE_OTIO = importlib.util.find_spec("opentimelineio") is not None
TPS = 254_016_000_000


def ticks(seconds: Fraction | int) -> int:
    return int(Fraction(seconds) * TPS)


def expected_entry(**over):
    entry = {
        "file": "a.otio",
        "name": "Seq",
        "markers": 0,
        "tracks": [
            {"kind": "Video", "name": "V1", "items": [
                {"name": "one", "start_ticks": ticks(1), "duration_ticks": ticks(2), "markers": 1},
            ]},
            {"kind": "Audio", "name": "A1", "items": []},
        ],
    }
    entry.update(over)
    return entry


def observed_view(start=Fraction(1), duration=Fraction(2), markers=1):
    return {
        "name": "Seq",
        "markers": 0,
        "tracks": [
            {"kind": "Video", "name": "V1", "items": [
                {"name": "one", "start": start, "duration": duration, "markers": markers},
            ]},
            {"kind": "Audio", "name": "A1", "items": []},
        ],
    }


class CompareTests(unittest.TestCase):
    def test_identical_views_agree(self):
        self.assertEqual(oracle.compare(expected_entry(), observed_view(), TPS), [])

    def test_float_noise_below_tolerance_is_accepted(self):
        noisy = observed_view(start=Fraction(1) + Fraction(1, 10**8))
        self.assertEqual(oracle.compare(expected_entry(), noisy, TPS), [])

    def test_one_sample_offset_is_reported(self):
        late = observed_view(start=Fraction(1) + Fraction(1, 48_000))
        problems = oracle.compare(expected_entry(), late, TPS)
        self.assertEqual(len(problems), 1)
        self.assertIn("start", problems[0])

    def test_structure_differences_are_reported(self):
        obs = observed_view(markers=0)
        obs["tracks"].pop()
        obs["tracks"][0]["items"][0]["name"] = "other"
        problems = "\n".join(oracle.compare(expected_entry(), obs, TPS))
        self.assertIn("track count 1 != 2", problems)
        self.assertIn("name 'other'", problems)
        self.assertIn("0 markers, expected 1", problems)

    def test_stack_items_skip_the_marker_count(self):
        self.assertEqual(oracle.compare(expected_entry(), observed_view(markers=None), TPS), [])


class ManifestTests(unittest.TestCase):
    def write(self, directory: Path, manifest) -> None:
        (directory / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")

    def good(self):
        return {"version": 1, "ticks_per_second": TPS, "fixtures": [expected_entry()]}

    def test_valid_manifest_loads(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.write(Path(tmp), self.good())
            self.assertEqual(oracle.load_manifest(Path(tmp))["ticks_per_second"], TPS)

    def test_hostile_manifests_are_rejected(self):
        bad = [
            {**self.good(), "version": 2},
            {**self.good(), "ticks_per_second": True},
            {**self.good(), "ticks_per_second": 0},
            {**self.good(), "fixtures": []},
            {**self.good(), "fixtures": [expected_entry(file="../escape.otio")]},
            {**self.good(), "fixtures": [expected_entry(file="/etc/passwd.otio")]},
            {**self.good(), "fixtures": [expected_entry(file="notes.txt")]},
            [],
        ]
        for manifest in bad:
            with self.subTest(manifest=manifest), tempfile.TemporaryDirectory() as tmp:
                self.write(Path(tmp), manifest)
                with self.assertRaises(oracle.OracleError):
                    oracle.load_manifest(Path(tmp))

    def test_missing_and_broken_manifests_are_errors_not_crashes(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(oracle.OracleError):
                oracle.load_manifest(Path(tmp))
            (Path(tmp) / "manifest.json").write_bytes(b"\xff{")
            with self.assertRaises(oracle.OracleError):
                oracle.load_manifest(Path(tmp))

    def test_missing_library_exits_with_2(self):
        with (
            patch.object(oracle, "load_otio", side_effect=oracle.OracleError("not installed")),
            tempfile.TemporaryDirectory() as tmp,
        ):
            self.assertEqual(oracle.main([tmp]), 2)


@unittest.skipUnless(HAVE_OTIO, "opentimelineio is not installed")
class OracleWithLibraryTests(unittest.TestCase):
    def build(self, directory: Path) -> None:
        import opentimelineio as otio

        rate = 25
        timeline = otio.schema.Timeline(name="Seq")
        video = otio.schema.Track(name="V1", kind=otio.schema.TrackKind.Video)
        video.append(otio.schema.Gap(source_range=otio.opentime.TimeRange(
            otio.opentime.RationalTime(0, rate), otio.opentime.RationalTime(25, rate))))
        clip = otio.schema.Clip(name="one", source_range=otio.opentime.TimeRange(
            otio.opentime.RationalTime(100, rate), otio.opentime.RationalTime(50, rate)))
        clip.markers.append(otio.schema.Marker(name="m"))
        video.append(clip)
        timeline.tracks.append(video)
        timeline.tracks.append(otio.schema.Track(name="A1", kind=otio.schema.TrackKind.Audio))
        otio.adapters.write_to_file(timeline, str(directory / "a.otio"))
        manifest = {"version": 1, "ticks_per_second": TPS, "fixtures": [expected_entry()]}
        (directory / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")

    def test_matching_file_passes(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.build(Path(tmp))
            report = oracle.run(Path(tmp))
            self.assertEqual(report["result"], "PASS", report)

    def test_shifted_clip_and_garbage_file_fail(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            self.build(directory)
            doc = json.loads((directory / "a.otio").read_text(encoding="utf-8"))
            gap = doc["tracks"]["children"][0]["children"][0]
            gap["source_range"]["duration"]["value"] = 26
            (directory / "a.otio").write_text(json.dumps(doc), encoding="utf-8")
            report = oracle.run(directory)
            self.assertEqual(report["result"], "FAIL")
            self.assertIn("start", report["fixtures"][0]["problems"][0])
            (directory / "a.otio").write_bytes(b"\x00garbage")
            report = oracle.run(directory)
            self.assertEqual(report["result"], "FAIL")
            self.assertIn("could not process", report["fixtures"][0]["problems"][0])

    @unittest.skipUnless(os.environ.get("CFA4_OTIO_ORACLE_DIR"), "CFA4_OTIO_ORACLE_DIR is not set")
    def test_cfa4_exported_fixtures_pass(self):
        report = oracle.run(Path(os.environ["CFA4_OTIO_ORACLE_DIR"]))
        self.assertEqual(report["result"], "PASS", json.dumps(report, indent=2))


if __name__ == "__main__":
    unittest.main()
