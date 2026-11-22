"""Tools ▸ Audio Tools ▸ AT9/AT3 Converter: output naming, the request each file sends to the
engine (`audio.atracConvert`), the job queue followed through `jobs.list`, cancelling, and the
dialog's JSON state. Runs offscreen with PySide6 and a fake engine; the conversion itself is
tested in the engine (`crates/engine/src/atrac_convert_tests.rs`).
"""

import os
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import Any

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

try:
    from PySide6.QtWidgets import QApplication

    from cfa4_qt.atrac_converter import AtracConverterDialog, output_path
except ImportError:  # pragma: no cover - PySide6 missing
    QApplication = None  # type: ignore[assignment,misc]


class FakeEngine:
    """Answers at once: each `audio.atracConvert` starts a job that `jobs.list` reports finished
    on the second poll."""

    def __init__(self, fail: str | None = None) -> None:
        self.calls: list[tuple[str, dict[str, Any]]] = []
        self.polls: dict[int, int] = {}
        self.fail = fail

    def execute(
        self, command: str, params: dict[str, Any] | None = None, on_reply: Any = None, on_error: Any = None
    ) -> int:
        params = params or {}
        self.calls.append((command, params))
        if command == "audio.atracConvert":
            job = len([c for c in self.calls if c[0] == command])
            self.polls[job] = 0
            if on_reply:
                on_reply({"job": job}, None)
        elif command == "jobs.list":
            jobs = []
            for job in self.polls:
                self.polls[job] += 1
                done = self.polls[job] >= 2
                result = {"error": self.fail} if done and self.fail else ({"path": "x"} if done else None)
                jobs.append({"id": job, "progress": 1.0 if done else 0.5, "finished": done, "result": result})
            if on_reply:
                on_reply(jobs, None)
        return 0


@unittest.skipIf(QApplication is None, "PySide6 is not installed")
class ConverterDialog(unittest.TestCase):
    app: Any = None

    @classmethod
    def setUpClass(cls) -> None:
        cls.app = QApplication.instance() or QApplication([])

    def test_output_names(self) -> None:
        self.assertEqual(output_path("/a/song.at9", "", "wav"), str(Path("/a/song.wav")))
        self.assertEqual(output_path("/a/song.wav", "/out", "at3"), str(Path("/out/song.at3")))
        self.assertEqual(output_path("/a/song.WAV", "", "wav"), str(Path("/a/song_converted.wav")))

    def test_queue_runs_each_file(self) -> None:
        with TemporaryDirectory() as d:
            engine = FakeEngine()
            dlg = AtracConverterDialog(engine)
            dlg.set_ui_state({"files": [f"{d}/a.wav", f"{d}/b.mp3"], "format": "at3", "bitrateKbps": 66})
            self.assertFalse(dlg.bits.isEnabled())
            dlg.start()
            self.assertTrue(dlg.running())
            for _ in range(10):
                dlg._poll()
            self.assertFalse(dlg.running())
            sent = [p for c, p in engine.calls if c == "audio.atracConvert"]
            self.assertEqual(
                sent,
                [
                    {"input": f"{d}/a.wav", "output": str(Path(d) / "a.at3"), "format": "at3", "bitrateKbps": 66},
                    {"input": f"{d}/b.mp3", "output": str(Path(d) / "b.at3"), "format": "at3", "bitrateKbps": 66},
                ],
            )
            self.assertEqual(dlg.progress.value(), 1000)
            self.assertIn("2", dlg.status.text())
            # to WAV: the sample format instead of the rate
            dlg.set_ui_state({"format": "wav", "bits": 24, "folder": "/out"})
            self.assertEqual(dlg.ui_state()["format"], "wav")
            self.assertEqual(
                dlg.params("/x/s.at9", "/out/s.wav"),
                {"input": "/x/s.at9", "output": "/out/s.wav", "format": "wav", "bits": 24},
            )

    def test_errors_and_cancel(self) -> None:
        engine = FakeEngine(fail="no audio")
        dlg = AtracConverterDialog(engine)
        dlg.start()  # nothing to convert
        self.assertFalse(dlg.running())
        dlg.set_ui_state({"files": ["/nowhere/a.wav", "/nowhere/b.wav"]})
        dlg.start()
        dlg._cancel()  # stops after the running job
        self.assertIn(("jobs.cancel", {"job": 1}), engine.calls)
        for _ in range(5):
            dlg._poll()
        self.assertFalse(dlg.running())
        self.assertEqual(len([c for c, _ in engine.calls if c == "audio.atracConvert"]), 1)
        self.assertIn("no audio", dlg.status.text())


if __name__ == "__main__":
    unittest.main()
