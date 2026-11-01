"""CFA4 Qt 6 GUI Trim Monitor and dynamic trimming against the real Rust engine (when it is built):
``docs/qt-parity.md`` 5.6 and Play Around (5.3), as egui's ``trim_monitor.rs``."""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
QtCore = pytest.importorskip("PySide6.QtCore")
QtGui = pytest.importorskip("PySide6.QtGui")
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


def test_shift_and_dynamic_texts_as_egui():
    from cfa4_qt.panels.trim_monitor import dynamic_text, shift_text

    assert [shift_text(n) for n in (3, 0, -2, None)] == ["+3", "0", "-2", "0"]
    assert dynamic_text({"speed": 0}) == "TRIM ■"
    assert dynamic_text({"speed": 2.0}) == "TRIM ▶ 2×"
    assert dynamic_text({"speed": -0.25}) == "TRIM ◀ 0.25×"


def _drag(widget: Any, dx: int, escape: bool = False) -> None:
    """A left-button drag of ``dx`` pixels across ``widget`` (optionally abandoned with Escape)."""
    ev = QtGui.QMouseEvent
    t = QtCore.QEvent.Type
    left, none = QtCore.Qt.MouseButton.LeftButton, QtCore.Qt.MouseButton.NoButton
    mods = QtCore.Qt.KeyboardModifier.NoModifier
    a = QtCore.QPointF(widget.width() / 2 - dx / 2, widget.height() / 2)
    b = QtCore.QPointF(a.x() + dx, a.y())

    def at(p):
        return widget.mapToGlobal(p.toPoint()).toPointF()

    QtWidgets.QApplication.sendEvent(widget, ev(t.MouseButtonPress, a, at(a), left, left, mods))
    QtWidgets.QApplication.sendEvent(widget, ev(t.MouseMove, b, at(b), none, left, mods))
    if escape:
        key = QtGui.QKeyEvent(t.KeyPress, QtCore.Qt.Key.Key_Escape, mods)
        QtWidgets.QApplication.sendEvent(widget, key)
    QtWidgets.QApplication.sendEvent(widget, ev(t.MouseButtonRelease, b, at(b), left, none, mods))


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_trim_monitor_and_dynamic_trimming_as_egui(app, tmp_path: Path):
    """Trim mode as egui's: selecting an edit point turns the Program monitor into the two-up Trim
    Monitor (both frames, clip names, timecodes, shift counters, −N/−1/+1/+N, Apply Default
    Transitions); its buttons and drags trim; J/L trim dynamically and K commits one undo step,
    Escape cancels; Space loops around the edit; the ✕ leaves trim mode."""
    from cfa4_qt.main_window import MainWindow
    from cfa4_qt.panels.trim_monitor import RIPPLE_RED, ROLL_YELLOW
    from cfa4_qt.timecode import format_timecode, frame_ticks

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1600, 1000)
    win.show()
    engine.start()
    mon = win.monitor
    trim = mon.trim

    def v1() -> list[dict[str, Any]]:
        return list(((win.sequence or {}).get("video") or [{}])[0].get("items") or [])

    def done_exec(cid: str, params: dict[str, Any]) -> Any:
        out: list = []
        engine.execute(cid, params, on_reply=lambda r, _b: out.append(r), on_error=lambda m: out.append(("error", m)))
        assert wait_until(app, lambda: out, 10_000), cid
        win.schedule_refresh()
        return out[0]

    def info(key: str) -> Any:
        return trim.info.get(key)

    try:
        assert wait_until(app, lambda: win.sequence is not None and len(win.registry.commands) > 100, 60_000)
        assert "playback.playAround" in {c[0] for c in win.local_commands()}
        frame = frame_ticks((win.sequence or {}).get("settings"))
        first = v1()[0]
        done_exec("timeline.razor", {"time": (first["start"] + first["end"]) // 2 // frame * frame, "clip": first["clip"]})
        assert wait_until(app, lambda: len(v1()) > 1 and v1()[0]["end"] < first["end"], 10_000)
        left, right = v1()[0], v1()[1]
        assert trim.isHidden(), "no edit points: the Program monitor"
        # not in trim mode: Shift+L shuttles slowly, Space plays
        win.transport("playback.slowForward")
        assert mon.playing and mon.speed == 0.25
        win.transport("playback.stop")
        assert not mon.playing

        # an edit point: the Trim Monitor covers the Program monitor
        done_exec("trim.selectEditPoint", {"clip": left["clip"], "edge": "out", "kind": "ripple"})
        assert wait_until(app, lambda: trim.isVisible() and info("kind") == "ripple", 10_000)
        assert trim.geometry() == mon.rect()
        assert trim.out_name.text() == f"Out: {left['name']} (V1)" and trim.in_name.text() == f"In: {right['name']} (V1)"
        assert trim.out_tc.text() == format_timecode(left["end"] - frame, frame)
        assert trim.in_tc.text() == format_timecode(right["start"], frame)
        assert (trim.out_shift.text(), trim.in_shift.text()) == ("0", "0")
        assert trim.outgoing.hot and not trim.incoming.hot and trim.outgoing.color == RIPPLE_RED
        assert [trim.buttons[k].text() for k in ("backwardMany", "backward", "forward", "forwardMany")] == ["−5", "−1", "+1", "+5"]
        assert wait_until(app, lambda: trim.outgoing.image is not None and trim.incoming.image is not None, 20_000)
        assert not trim.outgoing.image.isNull()

        # a regular trim (yellow); a drag in the incoming picture trims the incoming side (its In
        # point, 6 px a frame)
        done_exec("trim.selectEditPoint", {"clip": left["clip"], "edge": "out", "kind": "trim"})
        assert wait_until(app, lambda: info("kind") == "trim" and trim.outgoing.color == ROLL_YELLOW, 10_000)
        _drag(trim.incoming, 30)
        assert wait_until(app, lambda: (win.sequence or {}).get("editPoints") == [{"clip": right["clip"], "out": False, "kind": "trim"}], 10_000)
        assert wait_until(app, lambda: trim.in_shift.text() == "+5" and v1()[1]["start"] == right["start"] + 5 * frame, 10_000)
        assert trim.incoming.hot and not trim.outgoing.hot
        # a gap before it now: the outgoing side shows (gap)
        assert trim.out_name.text() == "Out: (gap)" and trim.outgoing.text == "(gap)" and trim.out_tc.text() == "--:--:--:--"
        # −1 trims one frame back
        trim.buttons["backward"].click()
        assert wait_until(app, lambda: trim.in_shift.text() == "+4" and v1()[1]["start"] == right["start"] + 4 * frame, 10_000)
        # Escape abandons a drag
        before = [dict(c) for c in v1()]
        _drag(trim.incoming, 30, escape=True)
        wait_until(app, lambda: False, 300)
        assert v1() == before and trim.in_shift.text() == "+4"
        done_exec("trim.nudge", {"frames": -4})
        assert wait_until(app, lambda: v1()[1]["start"] == right["start"] and trim.out_name.text() != "Out: (gap)", 10_000)
        # between the pictures: a roll (yellow)
        _drag(trim.roll, -12)
        assert wait_until(app, lambda: info("kind") == "roll", 10_000)
        assert trim.outgoing.hot and trim.incoming.hot and trim.outgoing.color == ROLL_YELLOW

        # J/L in trim mode: a dynamic trim, K commits it as one undo step
        cut0 = v1()[0]["end"]
        win.transport("playback.forward")
        assert not mon.playing, "the Program monitor does not play in trim mode"
        assert wait_until(app, lambda: isinstance(info("dynamic"), dict) and trim.dynamic.text() == "TRIM ▶ 1×", 10_000)
        assert trim.escape.isEnabled()
        assert wait_until(app, lambda: v1()[0]["end"] > cut0, 10_000), "the sequence follows the moving edit"
        win.transport("playback.stop")
        assert wait_until(app, lambda: info("dynamic") is None and trim.dynamic.text() == "", 10_000)
        assert not trim.escape.isEnabled()
        labels: list = []
        engine.execute("history.list", {}, on_reply=lambda r, _b: labels.append(r))
        assert wait_until(app, lambda: labels, 10_000)
        assert "Dynamic Rolling Edit" in str(labels[0])
        # the committed edit, as the engine has it (the window's copy may still be refreshing)
        seq: list = []
        engine.execute("sequence.inspect", {}, on_reply=lambda r, _b: seq.append(r))
        assert wait_until(app, lambda: seq, 10_000)
        cut1 = seq[0]["video"][0]["items"][0]["end"]
        assert wait_until(app, lambda: v1()[0]["end"] == cut1, 10_000), "the window follows the commit"
        assert cut1 > cut0
        # …and Escape cancels one, restoring the edit
        win.transport("playback.reverse")
        assert wait_until(app, lambda: v1()[0]["end"] < cut1, 10_000)
        trim.escape.activated.emit()
        assert wait_until(app, lambda: info("dynamic") is None and v1()[0]["end"] == cut1, 10_000)

        # Space: loop playback around the edit (the playhead moves; the monitor stays two-up)
        win.transport("playback.toggle")
        assert wait_until(app, lambda: isinstance(info("playAround"), dict), 10_000)
        start = info("playAround")["start"]
        assert wait_until(app, lambda: mon.playhead > start, 10_000) and trim.isVisible()
        win.transport("playback.toggle")
        assert wait_until(app, lambda: info("playAround") is None, 10_000)

        # ✕ leaves trim mode
        trim.exit_button.click()
        assert wait_until(app, lambda: not (win.sequence or {}).get("editPoints") and trim.isHidden(), 10_000)
        win.transport("playback.toggle")
        assert mon.playing, "out of trim mode Space plays the Program monitor again"
        mon.stop()
    finally:
        mon.stop()
        win._quit_confirmed = True
        win.close()
        engine.stop()
