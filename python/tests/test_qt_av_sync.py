"""CFA4 Qt 6 GUI A/V sync against the real Rust engine (when it is built): ``docs/qt-parity.md``
5.2, as egui's ``advance_playback``: while a device plays, the samples it has played are the
playback clock and the picture follows the sound (a frame of the picture is never ahead of the
sound or more than a frame behind it); a device that fails or stops consuming samples hands the
clock back to the wall clock, and playback goes on without sound."""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
QtCore = pytest.importorskip("PySide6.QtCore")
QtWidgets = pytest.importorskip("PySide6.QtWidgets")
QtMultimedia = pytest.importorskip("PySide6.QtMultimedia")

from cfa4_qt.engine import EngineClient, locate_engine

REAL = locate_engine()
TPS = 254_016_000_000


@pytest.fixture(scope="module")
def app():
    yield QtWidgets.QApplication.instance() or QtWidgets.QApplication([])


@pytest.fixture(autouse=True)
def no_modal_dialogs(monkeypatch):
    def refuse(self, *_args, **_kwargs):
        raise AssertionError(f"modal dialog opened: {type(self).__name__} {self.objectName()!r}")

    monkeypatch.setattr(QtWidgets.QDialog, "exec", refuse)
    monkeypatch.setattr(QtWidgets.QMessageBox, "exec", refuse)


def wait_until(app, predicate, timeout_ms: int = 10_000) -> bool:
    timer = QtCore.QElapsedTimer()
    timer.start()
    while not predicate():
        if timer.elapsed() > timeout_ms:
            return False
        app.processEvents(QtCore.QEventLoop.ProcessEventsFlag.AllEvents, 20)
    return True


class FakeDevice:
    """An output that takes any format (the test machine has no sound server)."""

    def isNull(self) -> bool:
        return False

    def isFormatSupported(self, _fmt: Any) -> bool:
        return True


class FakeSink:
    """A QAudioSink whose played time the test sets: ``us`` is what the device has played."""

    made: list[FakeSink] = []  # noqa: RUF012 - the test reads the sinks the monitor opened

    def __init__(self, _device: Any, fmt: Any, _parent: Any) -> None:
        self.rate = fmt.sampleRate()
        self.us = 0
        self.size = 0
        self.written = bytearray()
        self.failed = False
        self.stopped = False
        FakeSink.made.append(self)

    def setBufferSize(self, n: int) -> None:
        self.size = n

    def bufferSize(self) -> int:
        return self.size

    def bytesFree(self) -> int:
        played = self.us * self.rate // 1_000_000 * 8
        return max(0, self.size - max(0, len(self.written) - played))

    def start(self) -> Any:
        sink = self

        class Io:
            def write(self, data: bytes) -> int:
                n = min(len(data), sink.bytesFree())
                sink.written += data[:n]
                return n

        return Io()

    def processedUSecs(self) -> int:
        return self.us

    def error(self) -> Any:
        e = QtMultimedia.QAudio.Error
        return e.FatalError if self.failed else e.NoError

    def stop(self) -> None:
        self.stopped = True

    def deleteLater(self) -> None:
        pass


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_picture_follows_the_sound_as_egui(app, tmp_path: Path, monkeypatch):
    """The device is the clock: before it plays, the picture waits at the first frame; then the
    Program shows the frame holding the sample being heard, whatever the wall clock says, and
    that sample is the engine's mix at that time (A/V offset under a frame). A device that stops
    consuming samples hands over to the wall clock where the sound got to; one that fails, at the
    frame shown. ``ui.inspect`` reports the clock and the offset."""
    import cfa4_qt.panels.monitor as monitor_mod
    from cfa4_qt.control import ControlServer
    from cfa4_qt.main_window import MainWindow
    from cfa4_qt.timecode import frame_ticks

    monkeypatch.setattr(monitor_mod, "output_device", lambda _name: FakeDevice())
    FakeSink.made.clear()
    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1400, 900)
    win.show()
    engine.start()
    mon = win.monitor
    mon.sink_factory = FakeSink
    server = ControlServer(win)
    notes: list[str] = []
    mon.message.connect(notes.append)

    def inspect() -> dict[str, Any]:
        return server.inspect()["playback"]["program"]

    try:
        assert wait_until(app, lambda: win.sequence is not None and len(win.registry.commands) > 100, 60_000)
        ft = frame_ticks((win.sequence or {}).get("settings"))
        rate = mon.sample_rate()
        mon.seek(ft * 240)
        start = mon.playhead
        assert inspect()["audioClock"] is False and inspect()["avOffset"] is None

        mon.play(1.0)
        assert len(FakeSink.made) == 1 and mon.audio_clock
        sink = FakeSink.made[0]
        assert sink.rate == rate
        # the sound is on its way but nothing is heard yet: the picture waits at the first frame
        assert wait_until(app, lambda: len(sink.written) > 0, 20_000), "the first block"
        wait_until(app, lambda: False, 200)
        assert mon.playhead == start
        assert inspect()["audioClock"] is True

        # the device has played 2 s (the wall clock far less): the picture is there, at the frame
        # holding the sample being heard
        for us in (400_000, 1_234_567, 2_000_000):
            sink.us = us
            heard = start + us * TPS // 1_000_000
            assert wait_until(app, lambda h=heard: mon.playhead == h // ft * ft, 5_000), (us, mon.playhead, heard)
            offset = mon.av_offset()
            assert offset is not None and 0 <= offset < ft, offset
            assert inspect()["avOffset"] == offset
        # what the device was given at the sample heard is the engine's mix at the picture's time
        k = sink.us * rate // 1_000_000
        assert wait_until(app, lambda: len(sink.written) >= (k + 480) * 8, 10_000), "fed ahead of the device"
        given = bytes(sink.written[k * 8 : (k + 480) * 8])
        assert len(given) == 480 * 8
        out: list[bytes] = []
        engine.call(
            "audio.render",
            {"start": start * rate // TPS + k, "frames": 480},
            on_reply=lambda _r, data: out.append(data or b""),
        )
        assert wait_until(app, lambda: out, 10_000)
        assert out[0] == given, "the samples at the heard position are the mix at that time"
        assert any(b != 0 for b in given), "the demo plays sound there"

        # the device stops consuming samples: after AUDIO_STALL_S the wall clock takes over from
        # where the sound got to, without sound
        stalled_at = start + sink.us * TPS // 1_000_000
        assert wait_until(app, lambda: not mon.audio_clock, 5_000)
        assert sink.stopped and notes and "not responding" in notes[-1]
        assert mon.playing and mon.playhead >= stalled_at // ft * ft
        moved = mon.playhead
        assert wait_until(app, lambda: mon.playhead > moved, 5_000), "playback goes on by the wall clock"
        assert inspect()["audioClock"] is False and inspect()["avOffset"] is None
        mon.stop()

        # a device that fails: the wall clock takes over at the frame shown
        notes.clear()
        mon.seek(start)
        mon.play(1.0)
        sink = FakeSink.made[-1]
        sink.us = 1_000_000
        assert wait_until(app, lambda: mon.playhead == (start + TPS) // ft * ft, 5_000)
        shown = mon.playhead
        sink.failed = True
        assert wait_until(app, lambda: not mon.audio_clock, 5_000)
        assert notes and "failed" in notes[-1]
        assert mon.playhead >= shown and mon.playhead < shown + TPS // 2, "resumes at the frame shown"
        mon.stop()

        # stopping and playing again starts the device clock afresh from the playhead
        mon.play(1.0)
        assert mon.audio_clock and FakeSink.made[-1].us == 0
        wait_until(app, lambda: False, 200)
        assert mon.playhead == (mon._play_from // ft) * ft
        mon.stop()
        assert not mon.audio_clock
    finally:
        mon.stop()
        win._quit_confirmed = True
        win.close()
        engine.stop()
