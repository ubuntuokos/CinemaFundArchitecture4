"""Varispeed chase and the LTC user bits in the Qt GUI (plan TC-8): ``timecodeSync.speed`` nudges
the Program monitor's speed, the status bar shows the user bits."""

from __future__ import annotations

import os
from collections.abc import Iterator

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
pytest.importorskip("PySide6.QtCore")
pytest.importorskip("PySide6.QtWidgets")

from PySide6 import QtCore, QtWidgets

from cfa4_qt.timecode_sync import TimecodeSyncLink, status_text, user_bits_text
from test_qt_timecode_sync import FakeEngine, FakeMonitor, transports


@pytest.fixture(scope="module")
def app() -> Iterator[QtCore.QCoreApplication]:
    yield QtWidgets.QApplication.instance() or QtWidgets.QApplication([])


def test_speed_events_nudge_a_playing_monitor(app: QtCore.QCoreApplication) -> None:
    engine, mon, parent = FakeEngine(), FakeMonitor(), QtWidgets.QWidget()
    link = TimecodeSyncLink(engine, mon, parent)
    # stopped: a speed event does not start playback
    assert link.on_event({"event": "timecodeSync.speed", "speed": 1.02})
    assert not mon.playing and transports(engine) == []
    mon.playhead = 500
    mon.play()
    assert link.on_event({"event": "timecodeSync.speed", "speed": 1.02})
    assert mon.playing and mon.speed == 1.02
    assert transports(engine)[-1] == {"playing": True, "time": 500, "speed": 1.02}
    # hostile values are ignored
    for bad in (True, "fast", None, 50.0, float("nan")):
        assert link.on_event({"event": "timecodeSync.speed", "speed": bad})
    assert mon.speed == 1.02
    assert link.on_event({"event": "timecodeSync.speed", "speed": 1})
    assert mon.speed == 1.0


def test_user_bits_show_in_the_status() -> None:
    assert user_bits_text("87654321") == "87 65 43 21"
    assert user_bits_text("deadbeef") == "DE AD BE EF"
    for bad in (None, 5, "", "123", "GGGGGGGG", "123456789"):
        assert user_bits_text(bad) == ""
    st = {"chase": True, "chaseStatus": {"state": "locked", "timecode": "01:00:00:00"}, "userBits": "87654321"}
    text, _tip, err = status_text(st)
    assert text == "TC: Chase Locked 01:00:00:00 · User bits 87 65 43 21" and not err
    # chase off: no user bits either
    assert status_text({**st, "chase": False})[0] == ""
