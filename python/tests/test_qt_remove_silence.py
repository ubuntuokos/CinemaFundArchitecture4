"""Tools ▸ Audio Tools ▸ Remove Silence… (F6 / V1-A, plan CFA4-V1-SILENCE-REMOVAL-001): the dialog
only calls engine commands. A fake engine checks the requests, a real engine the whole path:
Analyze → list → take a region out → Remove selected → undo."""

from __future__ import annotations

import math
import os
import struct
import wave
from pathlib import Path
from typing import Any

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest

pytest.importorskip("PySide6")

from PySide6 import QtCore, QtWidgets  # noqa: E402

from cfa4_qt.engine import EngineClient, locate_engine  # noqa: E402
from cfa4_qt.silence_dialog import RemoveSilenceDialog, open_remove_silence, seconds  # noqa: E402
from cfa4_qt.timecode import TICKS_PER_SECOND  # noqa: E402

REAL = locate_engine()


@pytest.fixture(scope="module")
def app() -> Any:
    return QtWidgets.QApplication.instance() or QtWidgets.QApplication([])


class FakeEngine:
    """Answers at once; the analysis is a job that `jobs.list` reports finished on the second poll."""

    def __init__(self) -> None:
        self.calls: list[tuple[str, dict[str, Any]]] = []
        self.polls = 0
        self.fail: str | None = None
        self.model: dict[str, Any] = {
            "id": "silero-vad",
            "license": "MIT (Silero VAD, model and code)",
            "licenseUrl": "https://github.com/snakers4/silero-vad/blob/master/LICENSE",
            "size": 2_845_718,
            "installed": False,
            "detectorAvailable": True,
            "downloadAvailable": True,
        }
        self.regions: list[Any] = [
            {"id": 11, "start": 0, "duration": TICKS_PER_SECOND, "keep": False},
            {"id": 12, "start": 3 * TICKS_PER_SECOND, "duration": TICKS_PER_SECOND // 2, "keep": True},
        ]

    def execute(
        self, command: str, params: dict[str, Any] | None = None, on_reply: Any = None, on_error: Any = None
    ) -> int:
        self.calls.append((command, params or {}))
        reply: Any = {}
        if command == "audio.detectSpeech":
            self.polls = 0
            reply = {"job": 7}
        elif command == "jobs.list":
            self.polls += 1
            done = self.polls >= 2
            result = {"error": self.fail} if done and self.fail else ({"path": "x"} if done else None)
            reply = [{"id": 7, "progress": 1.0 if done else 0.4, "finished": done, "result": result}]
        elif command == "speech.vadModel":
            reply = dict(self.model)
        elif command == "speech.downloadVadModel":
            if not (params or {}).get("licenseAccepted"):
                if on_error:
                    on_error("the model's licence must be shown and accepted first")
                return 0
            self.model["installed"] = True
        elif command == "silence.mark":
            reply = {"regions": self.regions, "ticks": 1.5 * TICKS_PER_SECOND}
        elif command == "clip.removeSilence":
            reply = {"removed": len((params or {}).get("regions", []))}
        if on_reply:
            on_reply(reply, None)
        return 0


def analyze(d: RemoveSilenceDialog) -> None:
    """Click Analyze and let the dialog's poll timer run until the job is done."""
    d.run_analyze()
    while d.running():
        d._poll()


class FakeWindow(QtWidgets.QMainWindow):
    def __init__(self) -> None:
        super().__init__()
        self.engine = FakeEngine()
        self.refreshes = 0
        self.seeks: list[int] = []

    def schedule_refresh(self) -> None:
        self.refreshes += 1

    def _seek_program(self, ticks: int) -> None:
        self.seeks.append(ticks)


@pytest.fixture
def win(app: Any) -> Any:
    """A window the dialogs are children of, deleted before the application goes."""
    w = FakeWindow()
    yield w
    w.close()
    w.deleteLater()
    app.sendPostedEvents(None, QtCore.QEvent.Type.DeferredDelete)


def test_the_dialog_sends_ticks_and_never_deletes_on_analyze(app: Any, win: FakeWindow) -> None:
    d = RemoveSilenceDialog(win, win)
    d.set_ui_state({"minSilenceSeconds": 0.75, "paddingSeconds": 0.2, "scope": "sequence"})
    analyze(d)
    cmd, p = next(c for c in win.engine.calls if c[0] == "audio.detectSpeech")
    assert cmd == "audio.detectSpeech"
    assert p == {
        "scope": "sequence",
        "detector": "energy",
        "background": True,
        "minSilence": round(0.75 * TICKS_PER_SECOND),
        "padding": round(0.2 * TICKS_PER_SECOND),
    }
    assert [c for c, _ in win.engine.calls if c == "clip.removeSilence"] == []
    # the kept region (keep: true from the engine) is unchecked, the other is checked
    assert d.checked_ids() == [11]
    assert d.remove.isEnabled()
    d.threshold.setValue(-35.0)
    d.auto_threshold.setChecked(False)
    assert d.params()["thresholdDb"] == -35.0


def test_unchecking_keeps_a_region_and_remove_sends_only_the_checked_ones(app: Any, win: FakeWindow) -> None:
    d = RemoveSilenceDialog(win, win)
    analyze(d)
    d.regions.item(0).setCheckState(QtCore.Qt.CheckState.Unchecked)
    assert win.engine.calls[-1] == ("silence.keep", {"ids": [11], "keep": True})
    d.select_all.click()
    assert win.engine.calls[-1] == ("silence.keep", {"ids": [12], "keep": False})
    d.ripple.setChecked(False)
    d.run_remove()
    assert win.engine.calls[-1] == ("clip.removeSilence", {"regions": [11, 12], "ripple": False, "mode": "cut"})
    assert d.regions.count() == 0 and not d.remove.isEnabled()


def test_remove_with_nothing_checked_sends_nothing(app: Any, win: FakeWindow) -> None:
    d = RemoveSilenceDialog(win, win)
    analyze(d)
    d.select_none.click()
    n = len(win.engine.calls)
    d.run_remove()
    assert len(win.engine.calls) == n
    assert d.status.text() == "No region is checked."


def test_the_analysis_is_a_job_that_can_be_cancelled_and_its_errors_are_shown(app: Any, win: FakeWindow) -> None:
    d = RemoveSilenceDialog(win, win)
    d.run_analyze()
    assert d.running() and d.analyze.text() == "Cancel"
    assert [c for c, _ in win.engine.calls if c == "silence.mark"] == [], "nothing is marked while it runs"
    d.run_analyze()  # the button is now Cancel
    assert win.engine.calls[-1] == ("jobs.cancel", {"job": 7})
    win.engine.fail = "cancelled"
    d._poll()
    d._poll()
    assert not d.running() and d.analyze.text() == "Analyze"
    assert d.status.text() == "Analysis cancelled."
    assert d.regions.count() == 0
    assert [c for c, _ in win.engine.calls if c == "silence.mark"] == []
    win.engine.fail = "reading the clip's audio: boom"
    analyze(d)
    assert d.status.text() == "reading the clip's audio: boom" and not d.running()


def test_mark_mode_and_hostile_replies(app: Any, win: FakeWindow) -> None:
    d = RemoveSilenceDialog(win, win)
    analyze(d)
    d.set_ui_state({"mode": "mark"})
    d.run_remove()
    assert win.engine.calls[-1][1]["mode"] == "mark"
    assert len(d.ui_state()["regions"]) == 2, "mark only keeps the list"
    # a malformed engine answer gives an empty list, not an exception
    win.engine.regions = [
        None,
        {"id": "x"},
        {"id": 1, "start": True, "duration": 5},
        {"id": 2, "start": 1.5, "duration": 2},
    ]
    analyze(d)
    assert d.ui_state()["regions"] == []
    assert seconds(True) == 0.0 and seconds("x") == 0.0 and seconds(TICKS_PER_SECOND) == 1.0


def test_the_speech_model_is_offered_but_downloaded_only_after_the_licence_is_accepted(app: Any, win: FakeWindow) -> None:
    d = RemoveSilenceDialog(win, win)
    assert [d.detector.itemData(i) for i in range(d.detector.count())] == ["energy", "silero"]
    assert d.params()["detector"] == "energy" and not d.model_box.isVisibleTo(d)
    d.set_ui_state({"detector": "silero"})
    assert d.model_box.isVisibleTo(d), "the model's record is shown for it"
    assert "MIT" in d.model_info.text() and "2.8 MB" in d.model_info.text()
    assert not d.threshold.isEnabled() and not d.auto_threshold.isEnabled(), "the model decides what is speech"
    assert "thresholdDb" not in d.params() and d.params()["detector"] == "silero"
    # not downloaded: Analyze says so and asks the engine nothing, there is no fall back to energy
    n = len(win.engine.calls)
    d.run_analyze()
    assert len(win.engine.calls) == n and "not downloaded" in d.status.text()
    assert not d.running()
    # the download needs the acceptance, in the dialog and in the engine
    assert not d.download.isEnabled()
    d.run_download()
    assert [c for c, _ in win.engine.calls if c == "speech.downloadVadModel"] == []
    assert d.status.text() == "Accept the licence first."
    d.accept_licence.setChecked(True)
    assert d.download.isEnabled()
    d.download.click()
    assert win.engine.calls[-2] == ("speech.downloadVadModel", {"licenseAccepted": True})
    assert d.model_installed() and not d.download.isVisibleTo(d) and not d.accept_licence.isVisibleTo(d)
    assert d.ui_state()["modelInstalled"] is True and d.ui_state()["detector"] == "silero"
    # installed: the analysis runs with the model detector
    analyze(d)
    detect = [p for c, p in win.engine.calls if c == "audio.detectSpeech"][-1]
    assert detect["detector"] == "silero" and "thresholdDb" not in detect
    assert len(d.ui_state()["regions"]) == 2
    d.set_ui_state({"detector": "energy"})
    assert d.auto_threshold.isEnabled() and not d.model_box.isVisibleTo(d)


def test_a_build_without_the_detector_keeps_energy_and_a_failed_download_is_shown(app: Any, win: FakeWindow) -> None:
    win.engine.model["detectorAvailable"] = False
    d = RemoveSilenceDialog(win, win)
    item = d.detector.model().item(d.detector.findData("silero"))  # type: ignore[attr-defined]
    assert item is not None and not item.isEnabled()
    d.set_ui_state({"detector": "silero"})
    d.refresh_model()
    assert d.detector.currentData() == "energy", "a build without the model detector never selects it"
    win.engine.model.update({"detectorAvailable": True, "downloadAvailable": False})
    d.set_ui_state({"detector": "silero", "licenceAccepted": True})
    assert not d.download.isEnabled(), "no download in a build without the download feature"
    # a hostile engine answer is not an exception
    win.engine.model = {"size": True, "licenseUrl": 5, "detectorAvailable": True}
    d.refresh_model()
    assert "? MB" in d.model_info.text()


def test_selecting_a_row_moves_the_playhead_and_the_dialog_is_one_per_window(app: Any, win: FakeWindow) -> None:
    d = RemoveSilenceDialog(win, win)
    analyze(d)
    d.regions.setCurrentRow(1)
    assert win.seeks[-1] == 3 * TICKS_PER_SECOND
    assert open_remove_silence(win) is open_remove_silence(win)
    assert open_remove_silence(win).isVisible()
    assert d.objectName() == "dialog.removeSilence"
    for name in ("analyze", "regions", "remove", "ripple", "mode", "scope", "clear", "selectAll", "selectNone"):
        assert d.findChild(QtCore.QObject, f"removeSilence.{name}") is not None, name


def _speech_wav(path: Path) -> None:
    """5 s: tone 0.5–1.5 s and 3.0–4.0 s, silence between and around."""
    sr = 48_000
    frames = bytearray()
    for i in range(5 * sr):
        t = i / sr
        v = 0.3 * math.sin(2 * math.pi * 440 * t) if 0.5 <= t < 1.5 or 3.0 <= t < 4.0 else 0.0001 * (i % 2)
        s = int(v * 32767)
        frames += struct.pack("<hh", s, s)
    with wave.open(str(path), "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(sr)
        w.writeframes(bytes(frames))


def wait_until(app: Any, predicate: Any, timeout_ms: int = 10_000) -> bool:
    timer = QtCore.QElapsedTimer()
    timer.start()
    while not predicate():
        if timer.elapsed() > timeout_ms:
            return False
        app.processEvents(QtCore.QEventLoop.ProcessEventsFlag.AllEvents, 20)
    return True


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_analyze_take_one_out_remove_and_undo(app: Any, tmp_path: Path) -> None:
    from cfa4_qt.main_window import MainWindow

    assert REAL is not None
    speech = tmp_path / "speech.wav"
    _speech_wav(speech)
    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1600, 1000)
    win.show()
    engine.start()

    def audio_end() -> int:
        seq = win.sequence or {}
        return max((it["end"] for tr in seq.get("audio", []) for it in tr.get("items", [])), default=0)

    def silence_markers() -> list[dict[str, Any]]:
        return [
            m for m in (win.sequence or {}).get("markers", []) if str(m.get("comment", "")).startswith("Silence region")
        ]

    try:
        assert wait_until(app, lambda: win.sequence is not None and len(win.registry.commands) > 100, 60_000)
        assert "clip.removeSilence" in win.registry.commands
        win.execute("file.newSequence", {"name": "talk", "audio": 2, "video": 1})
        win.execute("file.import", {"paths": [str(speech)]})

        def item_id() -> int | None:
            def walk(b: dict[str, Any]) -> int | None:
                for c in b.get("children") or []:
                    if c.get("item") is not None and c.get("name") == "speech.wav":
                        return int(c["item"])
                    if "item" not in c and (r := walk(c)) is not None:
                        return r
                return None

            return walk((win.project or {}).get("root") or {})

        assert wait_until(app, lambda: item_id() is not None, 30_000)
        win.execute("timeline.place", {"item": item_id(), "audioTrack": "A1", "seconds": 0.0})
        assert wait_until(app, lambda: audio_end() > 0, 20_000)
        assert win.sequence is not None
        clip = win.sequence["audio"][0]["items"][0]["clip"]
        win.execute("timeline.select", {"clips": [clip]})
        wait_until(app, lambda: False, 300)
        full = audio_end()

        d = open_remove_silence(win)
        d.set_ui_state({"minSilenceSeconds": 0.5, "paddingSeconds": 0.1})
        d.analyze.click()
        assert wait_until(app, lambda: d.regions.count() == 3, 20_000), d.status.text()
        assert wait_until(app, lambda: len(silence_markers()) == 3, 10_000), "the regions are markers on the timeline"
        assert audio_end() == full, "analysing deletes nothing"
        # take the middle region out of the removal
        d.regions.item(1).setCheckState(QtCore.Qt.CheckState.Unchecked)
        assert wait_until(app, lambda: d.checked_ids() and d.ui_state()["status"] != "", 5_000)
        wait_until(app, lambda: False, 500)
        d.remove.click()
        assert wait_until(app, lambda: audio_end() < full, 20_000)
        shorter = audio_end()
        removed = (full - shorter) / TICKS_PER_SECOND
        assert 0.8 < removed < 1.4, f"0.4 s at the start and 0.9 s at the end go, the middle stays: {removed:.2f} s"
        win.execute("edit.undo", {})
        assert wait_until(app, lambda: audio_end() == full, 20_000)
    finally:
        engine.stop()
        win.close()
        win.deleteLater()
        app.sendPostedEvents(None, QtCore.QEvent.Type.DeferredDelete)
