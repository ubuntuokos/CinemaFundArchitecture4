"""CFA4 OTIO-1.2: OpenTimelineIO samples for the CFA4 importer.

The conversion helpers are checked everywhere. The tests that need the official ``opentimelineio``
package skip when it is not installed (``pip install -e '.[otio]'``); the import side is the Rust
test ``oracle_samples``.
"""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from cfa4_devtools import otio_samples as samples
from cfa4_devtools.otio_oracle import OracleError

HAVE_OTIO = importlib.util.find_spec("opentimelineio") is not None


class TicksTest(unittest.TestCase):
    def test_common_rates_are_exact(self):
        self.assertEqual(samples.ticks_of(SimpleNamespace(value=25, rate=25)), samples.TICKS_PER_SECOND)
        self.assertEqual(
            samples.ticks_of(SimpleNamespace(value=30000, rate=samples.NTSC)), 1001 * samples.TICKS_PER_SECOND
        )
        self.assertEqual(samples.ticks_of(SimpleNamespace(value=1, rate=48_000)), samples.TICKS_PER_SECOND // 48_000)

    def test_non_positive_rate_is_an_error(self):
        with self.assertRaises(ValueError):
            samples.ticks_of(SimpleNamespace(value=1, rate=0))


class FilePathTest(unittest.TestCase):
    def test_paths_as_cfa4_opens_them(self):
        self.assertEqual(samples.file_path("file:///footage/shot%20a.mov"), "/footage/shot a.mov")
        self.assertEqual(samples.file_path("/abs/x.mov"), "/abs/x.mov")
        self.assertEqual(samples.file_path("clips/b.mov"), f"{samples.BASE_DIR}/clips/b.mov")


class LegacyMarkerTest(unittest.TestCase):
    def test_marker2_becomes_marker1_with_range(self):
        doc = {
            "OTIO_SCHEMA": "Clip.1",
            "markers": [{"OTIO_SCHEMA": "Marker.2", "name": "m", "comment": "c", "marked_range": {"x": 1}}],
        }
        out = samples._marker_v1(doc)
        self.assertEqual(out["markers"], [{"OTIO_SCHEMA": "Marker.1", "name": "m", "range": {"x": 1}}])
        self.assertEqual(doc["markers"][0]["OTIO_SCHEMA"], "Marker.2", "the input is not changed")


class MissingLibraryTest(unittest.TestCase):
    def test_missing_library_exits_with_2(self):
        with (
            patch.object(samples, "load_otio", side_effect=OracleError("not installed")),
            tempfile.TemporaryDirectory() as d,
        ):
            self.assertEqual(samples.main([d]), 2)


@unittest.skipUnless(HAVE_OTIO, "opentimelineio is not installed")
class WriteTest(unittest.TestCase):
    def test_samples_cover_every_schema_the_reader_handles(self):
        with tempfile.TemporaryDirectory() as d:
            manifest = samples.write(Path(d))
            text = "".join((Path(d) / s["file"]).read_text(encoding="utf-8") for s in manifest["samples"])
            stored = json.loads((Path(d) / "samples.json").read_text(encoding="utf-8"))
        self.assertEqual(stored, manifest)
        for schema in (
            "Timeline.1",
            "Stack.1",
            "Track.1",
            "Clip.1",
            "Clip.2",
            "Gap.1",
            "Transition.1",
            "Marker.1",
            "Marker.2",
            "ExternalReference.1",
            "MissingReference.1",
            "GeneratorReference.1",
            "LinearTimeWarp.1",
            "FreezeFrame.1",
            "Effect.1",
            "SerializableCollection.1",
            "ImageSequenceReference.1",
        ):
            self.assertIn(f'"OTIO_SCHEMA": "{schema}"', text, schema)

    def test_expectations_follow_otio_timing(self):
        import opentimelineio as otio

        tl = samples.expect_timeline(otio, samples.transitions(otio))
        # interleaved V1, A1, V2, A2 come out grouped by kind, each kind in order
        self.assertEqual(
            [(t["kind"], t["name"]) for t in tl["tracks"]],
            [("Video", "V1"), ("Video", "V2"), ("Audio", "A1"), ("Audio", "A2")],
        )
        v1 = tl["tracks"][0]
        self.assertEqual(v1["transitions"], 2)
        # transitions take no time in the track: "b" starts where "a" ends
        a, b = v1["items"]
        self.assertEqual(b["start_ticks"], a["start_ticks"] + a["duration_ticks"])
        nested = samples.expect_timeline(otio, samples.nested(otio))
        inner = nested["tracks"][0]["items"][0]
        self.assertEqual((inner["name"], inner["markers"]), ("Inner", []))
        self.assertEqual(inner["start_ticks"], 5 * samples.TICKS_PER_SECOND // 25)

    def test_start_timecode_and_media_time_markers(self):
        import opentimelineio as otio

        tl = samples.expect_timeline(otio, samples.start_timecode(otio, 30))
        self.assertEqual((tl["rate"], tl["start_timecode"]), (30, 30 * 3600))
        # the clip marker sits 10 frames into media whose available range starts at frame 1000
        one = tl["tracks"][0]["items"][0]
        self.assertEqual(one["markers"], [110 * samples.TICKS_PER_SECOND // 30])
        # the dissolve is centred on the cut after "one" (10-frame gap + 50 frames), 6 + 6 frames long
        self.assertEqual(
            tl["tracks"][0]["transition_ranges"],
            [[54 * samples.TICKS_PER_SECOND // 30, 12 * samples.TICKS_PER_SECOND // 30]],
        )

    def test_media_expectations(self):
        import opentimelineio as otio

        items = samples.expect_timeline(otio, samples.media_paths(otio))["tracks"][0]["items"]
        media = {i["name"]: i["media"] for i in items}
        self.assertEqual(media["multi"]["path"], "/full/multi.mov", "the active reference")
        self.assertEqual(media["url"]["path"], "/footage/shot a.mov")
        self.assertEqual(media["relative"]["path"], f"{samples.BASE_DIR}/clips/b.mov")
        self.assertEqual(media["red"], {"ref": "generator", "generator": "ColorMatte", "offline": False})
        refs = samples.expect_timeline(otio, samples.references(otio))["tracks"][0]["items"]
        self.assertEqual({i["name"]: i["media"]["offline"] for i in refs}["missing"], True)
        fx = {i["name"]: i for i in samples.expect_timeline(otio, samples.time_effects(otio))["tracks"][0]["items"]}
        self.assertEqual((fx["fast"]["speed"], fx["reverse"]["reverse"], fx["hold"]["frame_hold"]), (2.0, True, True))
        self.assertFalse(fx["disabled"]["enabled"])

    def test_bare_track_is_one_sequence_named_after_it(self):
        import opentimelineio as otio

        (tl,) = samples.timelines_of(otio, samples.bare_track(otio))
        want = samples.expect_timeline(otio, tl)
        self.assertEqual((want["name"], want["rate"], want["start_timecode"]), ("Solo", 30, 0))

    def test_image_sequence_expects_its_first_frame(self):
        import opentimelineio as otio

        tl = samples.media_paths(otio)
        clip = next(c for c in tl.find_clips() if c.name == "frames")
        self.assertEqual(
            samples.expect_media(otio, clip),
            {"ref": "file", "path": "/frames/shot.0001.exr", "image_sequence": True, "offline": False},
        )

    def test_legacy_markers_read_back_at_the_same_time(self):
        import opentimelineio as otio

        tl = samples.edit(otio)
        new = otio.adapters.read_from_string(samples.serialise(otio, tl, None))
        old = otio.adapters.read_from_string(samples.serialise(otio, tl, {"Clip": 1, "Marker": 1}))
        self.assertEqual(samples.expect_timeline(otio, old), samples.expect_timeline(otio, new))


if __name__ == "__main__":
    unittest.main()
