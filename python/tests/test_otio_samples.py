"""CFA4 OTIO-1.2: the OpenTimelineIO sample writer.

The path and tick helpers are tested everywhere; writing the samples needs the official
``opentimelineio`` package and skips without it (``pip install --require-hashes -r
requirements/otio-oracle.txt`` or ``pip install -e '.[otio]'``). The import side is the Rust test
``oracle_samples`` (``CFA4_OTIO_SAMPLES_DIR``).
"""
from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from fractions import Fraction
from pathlib import Path
from unittest.mock import patch

from cfa4_devtools import otio_oracle, otio_samples

HAVE_OTIO = importlib.util.find_spec("opentimelineio") is not None


class HelperTests(unittest.TestCase):
    def test_file_paths_as_cfa4_opens_them(self):
        self.assertEqual(otio_samples.file_path("file:///footage/shot%20a.mov"), "/footage/shot a.mov")
        self.assertEqual(otio_samples.file_path("/abs/x.mov"), "/abs/x.mov")
        self.assertEqual(otio_samples.file_path("clips/b.mov"), f"{otio_samples.BASE_DIR}/clips/b.mov")

    def test_ticks_are_exact_for_ntsc_frames(self):
        frame = Fraction(1001, 30000)
        self.assertEqual(otio_samples.ticks(frame), otio_samples.TICKS_PER_SECOND * 1001 // 30000)
        self.assertEqual(otio_samples.ticks(Fraction(1)), otio_samples.TICKS_PER_SECOND)

    def test_missing_library_is_exit_two(self):
        with (
            patch.object(otio_samples, "load_otio", side_effect=otio_oracle.OracleError("missing")),
            tempfile.TemporaryDirectory() as tmp,
        ):
            self.assertEqual(otio_samples.main([tmp]), 2)


@unittest.skipUnless(HAVE_OTIO, "opentimelineio is not installed")
class WriteTests(unittest.TestCase):
    def test_every_sample_is_written_and_reads_back(self):
        import opentimelineio as otio

        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            self.assertEqual(otio_samples.main([str(directory)]), 0)
            manifest = json.loads((directory / "manifest.json").read_text(encoding="utf-8"))
            self.assertEqual(manifest["version"], 1)
            self.assertEqual(manifest["kind"], "otio-samples")
            self.assertEqual(manifest["ticks_per_second"], otio_samples.TICKS_PER_SECOND)
            files = [s["file"] for s in manifest["samples"]]
            self.assertEqual(len(files), len(set(files)))
            self.assertEqual(len(files), len(otio_samples.RATES) + 6)
            for sample in manifest["samples"]:
                with self.subTest(sample["file"]):
                    self.assertEqual(Path(sample["file"]).name, sample["file"])
                    otio.adapters.read_from_file(str(directory / sample["file"]))
                    self.assertTrue(sample["sequences"])
                    for seq in sample["sequences"]:
                        items = [i for t in seq["tracks"] for i in t["items"]]
                        self.assertTrue(items, f"{seq['name']}: no items")
                        for item in items:
                            self.assertGreater(item["duration_ticks"], 0)

    def test_schema_coverage(self):
        """Every schema the CFA4 importer reads appears in at least one sample."""
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            otio_samples.main([str(directory)])
            text = "".join(p.read_text(encoding="utf-8") for p in directory.glob("*.otio"))
        for schema in ("Timeline.1", "Stack.1", "SerializableCollection.1", "Track.1", "Clip.2", "Gap.1",
                       "Transition.1", "Marker.2", "ExternalReference.1", "ImageSequenceReference.1",
                       "GeneratorReference.1", "MissingReference.1", "LinearTimeWarp.1", "FreezeFrame.1",
                       "Effect.1"):
            self.assertIn(f'"OTIO_SCHEMA": "{schema}"', text, schema)

    def test_expectations_follow_otio_timing(self):
        import opentimelineio as otio

        samples = otio_samples.Samples(otio)
        seq = samples.expect(samples.per_rate("24", 24.0))[0]
        v1 = seq["tracks"][0]
        # gap 12, shot_a 48, shot_b 36, gap 5, shot_c 20 frames at 24 fps
        starts = [i["start_ticks"] for i in v1["items"]]
        self.assertEqual(starts, [otio_samples.ticks(Fraction(n, 24)) for n in (12, 60, 101)])
        # fade in at the start of shot_a, a centred cut, a fade out at the end of shot_c
        self.assertEqual([t["start_ticks"] for t in v1["transitions"]],
                         [otio_samples.ticks(Fraction(n, 24)) for n in (12, 54, 117)])
        # clip markers are in media time, relative to the available range start
        self.assertEqual(v1["items"][0]["markers"][0]["start_ticks"], otio_samples.ticks(Fraction(20, 24)))
        self.assertEqual(seq["start_timecode"], 24 * 3600)


if __name__ == "__main__":
    unittest.main()
