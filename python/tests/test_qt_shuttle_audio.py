"""CFA4 Qt 6 GUI sound while shuttling against the real Rust engine (when it is built):
``docs/qt-parity.md`` 5.3, as egui's ``shuttle.rs``: J / L at 0.5–4× forward is heard, mixed at
``device rate / speed`` and played at the device rate (pitch-corrected by Preferences ▸ Audio ▸
Maintain pitch while shuttling); faster, slower and backwards it stays silent."""

from __future__ import annotations

import itertools
import os
from pathlib import Path
from typing import Any

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
QtCore = pytest.importorskip("PySide6.QtCore")
QtWidgets = pytest.importorskip("PySide6.QtWidgets")

from cfa4_qt.engine import EngineClient, locate_engine

REAL = locate_engine()


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


def test_which_speeds_are_heard_as_egui():
    from cfa4_qt.panels.monitor import shuttle_sounds

    assert [shuttle_sounds(s) for s in (0.5, 1.0, 2.0, 4.0)] == [True] * 4
    assert [shuttle_sounds(s) for s in (0.25, 8.0, -1.0, -2.0, 0.0)] == [False] * 5


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_shuttle_is_heard_at_its_speed(app, tmp_path: Path):
    """L L plays 2× with sound: consecutive blocks of the timeline mixed at half the device rate
    (the engine's ``mixRate``), from the playhead, without meters and without an automation pass;
    8×, ¼× and backwards are silent; 1× plays as before (metered)."""
    from cfa4_qt.main_window import MainWindow
    from cfa4_qt.timecode import frame_ticks

    TPS = 254_016_000_000
    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1400, 900)
    win.show()
    engine.start()
    mon = win.monitor
    asked: list[dict[str, Any]] = []
    replies: list[tuple[dict[str, Any], int]] = []
    real_call = engine.call

    def spy(method: str, params: dict[str, Any] | None = None, on_reply: Any = None, on_error: Any = None) -> int:
        if method != "audio.render":
            return real_call(method, params, on_reply, on_error)
        asked.append(dict(params or {}))

        def got(result: Any, data: bytes | None) -> None:
            if isinstance(result, dict):
                replies.append((result, len(data or b"")))
            if on_reply is not None:
                on_reply(result, data)

        return real_call(method, params, got, on_error)

    engine.call = spy  # type: ignore[method-assign]
    sent: list[tuple[str, dict[str, Any]]] = []
    mon.command.connect(lambda c, p: sent.append((c, p)))
    try:
        assert wait_until(app, lambda: win.sequence is not None and len(win.registry.commands) > 100, 60_000)
        rate = mon.sample_rate()
        frame = frame_ticks((win.sequence or {}).get("settings"))
        mon.seek(frame * 240)
        assert wait_until(app, lambda: mon.playhead == frame * 240, 5_000)
        from_t = mon.playhead

        # L L: 2× forward, heard
        mon.shuttle(1)
        mon.shuttle(1)
        assert mon.playing and mon.speed == 2.0
        mon.stop()
        assert wait_until(app, lambda: len(replies) == len(asked), 20_000), "the 1× block answered"
        mon.seek(from_t)
        asked.clear()
        replies.clear()
        sent.clear()
        mon.play(2.0)
        assert wait_until(app, lambda: len(replies) >= 3, 20_000), asked
        mon.stop()
        assert all(p.get("speed") == 2.0 and "meter" not in p for p in asked), asked
        mix = rate // 2
        assert asked[0]["start"] == from_t * mix // TPS, (asked[0], from_t)
        for a, b in itertools.pairwise(asked):
            assert b["start"] == a["start"] + a["frames"], "one continuous stream"
        for result, size in replies:
            assert (result.get("sampleRate"), result.get("mixRate"), result.get("channels")) == (rate, mix, 2), result
            assert "meter" not in result
            assert size == int(result["frames"]) * 2 * 4
        assert mon.audio_rate == rate, "the sink plays at the device rate"
        assert not [c for c, _p in sent if c == "mixer.recordStart"], "no automation pass while shuttling"

        # 8×, ¼× and backwards: silent
        for speed in (8.0, 0.25, -1.0):
            asked.clear()
            mon.seek(frame * 240)
            mon.play(speed)
            mon._feed_audio()
            wait_until(app, lambda: False, 150)
            mon.stop()
            assert not asked, f"{speed}× is silent"

        # 1× plays as before: metered, at the device rate
        asked.clear()
        replies.clear()
        mon.seek(frame * 240)
        mon.play(1.0)
        assert wait_until(app, lambda: replies, 20_000)
        mon.stop()
        assert asked[0].get("meter") is True and "speed" not in asked[0]
        assert replies[0][0].get("sampleRate") == rate and "mixRate" not in replies[0][0]
    finally:
        engine.call = real_call  # type: ignore[method-assign]
        mon.stop()
        win._quit_confirmed = True
        win.close()
        engine.stop()
