"""External timecode sync in the Qt GUI (plan TC-6): the Program monitor's transport goes to the
engine, the engine's locate / stop events move the monitor, the status bar shows the chase state."""

from __future__ import annotations

import os
from collections.abc import Callable, Iterator
from pathlib import Path
from typing import Any

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
pytest.importorskip("PySide6.QtCore")
pytest.importorskip("PySide6.QtWidgets")

from PySide6 import QtCore, QtWidgets

from cfa4_qt.engine import EngineClient, locate_engine
from cfa4_qt.timecode_sync import TimecodeSyncLink, status_text

REAL = locate_engine()


@pytest.fixture(scope="module")
def app() -> Iterator[QtCore.QCoreApplication]:
    yield QtWidgets.QApplication.instance() or QtWidgets.QApplication([])


class FakeEngine:
    running = True

    def __init__(self) -> None:
        self.calls: list[tuple[str, dict[str, Any]]] = []

    def call(self, method: str, params: dict[str, Any] | None = None, on_reply: Any = None, on_error: Any = None) -> int:
        self.calls.append((method, params or {}))
        return len(self.calls)

    def execute(self, command: str, params: dict[str, Any] | None = None, on_reply: Any = None, on_error: Any = None) -> int:
        return self.call(command, params, on_reply, on_error)


class FakeMonitor(QtCore.QObject):
    playback_started = QtCore.Signal()
    playback_stopped = QtCore.Signal()

    def __init__(self) -> None:
        super().__init__()
        self.playing = False
        self.speed = 1.0
        self.playhead = 0
        self.log: list[tuple[str, Any]] = []

    def seek(self, ticks: int) -> None:
        if self.playing:
            self.stop(notify=False)
        self.playhead = ticks
        self.log.append(("seek", ticks))

    def play(self, speed: float = 1.0) -> None:
        self.playing, self.speed = True, speed
        self.log.append(("play", speed))
        self.playback_started.emit()

    def stop(self, notify: bool = True) -> None:
        if not self.playing:
            return
        self.playing, self.speed = False, 1.0
        self.log.append(("stop", notify))
        self.playback_stopped.emit()


def transports(engine: FakeEngine) -> list[dict[str, Any]]:
    return [p for m, p in engine.calls if m == "timecodeSync.transport"]


def test_transport_follows_the_monitor(app: QtCore.QCoreApplication) -> None:
    engine, mon, parent = FakeEngine(), FakeMonitor(), QtWidgets.QWidget()
    link = TimecodeSyncLink(engine, mon, parent)
    mon.playhead = 1000
    mon.play(2.0)
    assert transports(engine)[-1] == {"playing": True, "time": 1000, "speed": 2.0}
    # playing: the engine runs the clock on, the poll sends nothing new
    mon.playhead = 5000
    link._check()
    assert len(transports(engine)) == 1
    mon.stop()
    assert transports(engine)[-1] == {"playing": False, "time": 5000, "speed": 1.0}
    # a seek while stopped is reported by the poll, once
    mon.playhead = 7000
    link._check()
    link._check()
    assert [t["time"] for t in transports(engine)] == [1000, 5000, 7000]
    engine.running = False
    mon.playhead = 9000
    link._check()
    assert len(transports(engine)) == 3, "nothing is sent while the engine is down"


def test_locate_and_stop_events_move_the_monitor(app: QtCore.QCoreApplication) -> None:
    engine, mon, parent = FakeEngine(), FakeMonitor(), QtWidgets.QWidget()
    link = TimecodeSyncLink(engine, mon, parent)
    assert link.on_event({"event": "timecodeSync.locate", "time": 2540, "play": True})
    assert mon.log == [("seek", 2540), ("play", 1.0)]
    assert link.on_event({"event": "timecodeSync.locate", "time": 3000, "play": False})
    assert mon.playing is False and mon.playhead == 3000
    assert link.on_event({"event": "timecodeSync.locate", "time": "bad"})
    assert mon.playhead == 3000
    mon.play()
    assert link.on_event({"event": "timecodeSync.stop"})
    assert mon.playing is False
    assert not link.on_event({"event": "projectChanged"})


def test_status_text() -> None:
    assert status_text(None) == ("", "", False)
    off = {"chase": False, "generateLtc": False, "chaseStatus": None, "ltcOutRunning": False, "mtcOutRunning": False, "error": None}
    assert status_text(off)[0] == ""
    chase = {**off, "chase": True, "chaseStatus": {"state": "locked", "timecode": "01:00:00:00"}}
    assert status_text(chase) == ("TC: Chase Locked 01:00:00:00", "", False)
    both = {**chase, "ltcOutRunning": True, "mtcOutRunning": True, "warning": "no MTC code"}
    text, tip, err = status_text(both)
    assert text.endswith("Sending LTC + MTC") and tip == "no MTC code" and not err
    broken = {**off, "generateLtc": True, "error": "LTC output: no output `x`"}
    assert status_text(broken) == ("TC: Error", "LTC output: no output `x`", True)


def test_status_label(app: QtCore.QCoreApplication) -> None:
    engine, mon, parent = FakeEngine(), FakeMonitor(), QtWidgets.QWidget()
    link = TimecodeSyncLink(engine, mon, parent)
    link.refresh_status()
    assert engine.calls[-1][0] == "timecodeSync.status"
    link.show_status({"chase": True, "chaseStatus": {"state": "noSignal"}, "error": "no input"})
    assert link.label.text() == "TC: Chase No signal" and link.label.toolTip() == "no input" and not link.label.isHidden()
    link.show_status({"chase": False})
    assert link.label.isHidden()


def wait_until(app: QtCore.QCoreApplication, predicate: Callable[[], object], timeout_ms: int = 10_000) -> bool:
    timer = QtCore.QElapsedTimer()
    timer.start()
    while not predicate():
        if timer.elapsed() > timeout_ms:
            return False
        app.processEvents(QtCore.QEventLoop.ProcessEventsFlag.AllEvents, 20)
    return True


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_transport_devices_and_status(app: QtCore.QCoreApplication, tmp_path: Path) -> None:
    """The engine takes the transport, lists the devices the settings can name, and its status
    shows a device error (no sound card here) without the GUI or the engine failing."""
    assert REAL is not None
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    got: dict[str, Any] = {}
    engine.started.connect(lambda _h: got.setdefault("started", True))
    engine.start()
    try:
        assert wait_until(app, lambda: "started" in got, 60_000)
        engine.call("timecodeSync.transport", {"playing": False, "time": 0}, on_reply=lambda r, _b: got.setdefault("transport", r))
        engine.call("timecodeSync.devices", {}, on_reply=lambda r, _b: got.setdefault("devices", r))
        engine.call("timecodeSync.transport", {"playing": "yes"}, on_error=lambda m: got.setdefault("bad", m))
        assert wait_until(app, lambda: {"transport", "devices", "bad"} <= got.keys())
        assert got["transport"] == {"active": False}
        assert set(got["devices"]) == {"ltcInputs", "ltcOutputs", "mtcInputs", "mtcOutputs"}
        engine.execute("prefs.set", {"key": "timecodeSync.generateLtc", "value": True}, on_reply=lambda r, _b: got.setdefault("set", r), on_error=lambda m: got.setdefault("set", m))
        assert wait_until(app, lambda: "set" in got)
        st: dict[str, Any] = {}

        def poll() -> bool:
            engine.execute("timecodeSync.status", {}, on_reply=lambda r, _b: st.update(r))
            return st.get("generateLtc") is True and bool(st.get("ltcOutRunning") or st.get("error"))

        assert wait_until(app, poll, 20_000), st
    finally:
        engine.stop()


def test_timecode_dialog_reads_ltc(app: QtCore.QCoreApplication) -> None:
    """Clip ▸ Modify ▸ Timecode: Read from LTC Audio fills Set Timecode from the engine's
    clip.readLtcTimecode (without applying it); an error shows in the dialog."""
    from cfa4_qt.panels.clip_dialogs import ClipDialog

    sent: list[tuple[str, dict[str, Any]]] = []
    answer: dict[str, Any] = {"error": None, "result": {"timecode": "14:29:59:15", "applied": False}}

    def run(command: str, params: dict[str, Any], then: Any) -> None:
        sent.append((command, params))
        then(answer["error"], answer["result"])

    d = ClipDialog(None, {"command": "clip.modifyTimecode", "params": {"timecode": "00:00:00:00", "tapeName": "", "reset": False}}, run)
    button = d.findChild(QtWidgets.QPushButton, "timecode.readLtc")
    field = d.findChild(QtWidgets.QLineEdit, "timecode.timecode")
    assert button is not None and field is not None
    button.click()
    assert sent == [("clip.readLtcTimecode", {"apply": False})]
    assert field.text() == "14:29:59:15" and d.params["timecode"] == "14:29:59:15"
    answer.update(error="no LTC timecode found in the first 60 s of the clip's audio", result=None)
    button.click()
    assert d.error.text().startswith("no LTC") and field.text() == "14:29:59:15" and button.isEnabled()
