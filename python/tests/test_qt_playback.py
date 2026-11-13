"""CFA4 Qt 6 GUI playback against the real Rust engine (when it is built): draft decoding at reduced
playback resolution (``docs/qt-parity.md`` 5.4)."""

from __future__ import annotations

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


def test_draft_frames_follow_egui_rule():
    """egui's ``draft_playback``: only while playing, at 1/2 resolution or lower, with the
    preference on."""
    from types import SimpleNamespace

    from cfa4_qt.panels.monitor import Monitor

    m: Any = SimpleNamespace(draft_decode=True, playing=True)
    assert Monitor.draft_frames(m, 0.5) and Monitor.draft_frames(m, 0.25)
    assert not Monitor.draft_frames(m, 1.0), "full resolution is never draft"
    m.playing = False
    assert not Monitor.draft_frames(m, 0.5), "a paused frame is never draft"
    m.playing, m.draft_decode = True, False
    assert not Monitor.draft_frames(m, 0.5), "off unless enabled"


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_draft_decoding_while_playing_at_reduced_resolution(app, tmp_path: Path):
    """Settings ▸ Playback ▸ Draft decoding: the Program monitor asks the engine for draft frames
    while playing at 1/2 or lower, full frames at Full resolution and when stopped, and none at all
    with the preference off (egui's monitors)."""
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.show()
    engine.start()
    asked: list[dict[str, Any]] = []
    replies: list[dict[str, Any]] = []
    call = engine.call

    def spy(method: str, params: Any = None, on_reply: Any = None, on_error: Any = None, **kw: Any) -> Any:
        if method == "frame.render" and isinstance(params, dict) and params.get("target", "program") == "program":
            asked.append(dict(params))
            inner = on_reply

            def got(r: Any, b: Any) -> None:
                if isinstance(r, dict):
                    replies.append(r)
                if inner is not None:
                    inner(r, b)

            on_reply = got
        return call(method, params, on_reply=on_reply, on_error=on_error, **kw)

    engine.call = spy  # type: ignore[method-assign]
    mon = win.monitor
    try:
        assert wait_until(app, lambda: win.sequence is not None and mon.has_content(), 60_000)

        def play_and_watch() -> list[dict[str, Any]]:
            asked.clear()
            replies.clear()
            mon.play()
            assert wait_until(app, lambda: len(asked) >= 3, 20_000), asked
            mon.stop()
            return list(asked)

        # off by default: no draft frames
        mon.set_resolution("half")
        assert not any(p.get("draft") for p in play_and_watch())
        done: list = []
        engine.execute("prefs.set", {"key": "playback.draftDecode", "value": True}, on_reply=lambda r, _b: done.append(r))
        assert wait_until(app, lambda: done, 10_000)
        # on, at 1/2: draft frames while playing, and the engine says so
        playing = play_and_watch()
        assert any(p.get("draft") is True for p in playing), playing
        assert wait_until(app, lambda: any(r.get("draft") is True for r in replies), 10_000), replies
        # stopped: the paused frame is a full decode
        asked.clear()
        mon.request_frame()
        assert wait_until(app, lambda: asked, 10_000) and not asked[-1].get("draft")
        # Full resolution: never draft
        mon.set_resolution("full")
        assert not any(p.get("draft") for p in play_and_watch())
    finally:
        mon.stop()
        win._quit_confirmed = True
        win.close()
        engine.stop()


def test_playback_meter_counts_as_egui():
    """egui's ``meter_counts_frames_not_refreshes`` and ``…_but_not_speed_steps_or_loops``."""
    from cfa4_qt.panels.monitor import PlaybackMeter

    m = PlaybackMeter()
    m.start(1.0)
    for f, exact in ((0, True), (0, True), (0, True), (1, False), (1, True), (2, False), (2, False), (3, True)):
        m.refresh(f, exact)
    assert m.counts() == (3, 1), "the frame due counts as shown once it is on screen"
    m.finish()
    # frame 1 arrived late but was on screen while due: shown; frame 2 never: dropped
    assert (m.shown, m.dropped) == (3, 1)
    m.start(1.0)
    m.refresh(10, True)
    m.refresh(14, True)  # no refresh during 11..13
    m.refresh(0, True)  # a loop restart is not a drop
    m.finish()
    assert (m.shown, m.dropped) == (3, 3)
    m.start(2.0)  # 2× steps over every other frame on purpose
    for f in (0, 2, 4, 6):
        m.refresh(f, True)
    m.finish()
    assert (m.shown, m.dropped) == (4, 0)
    m.start(-1.0)
    for f in (9, 8, 6):
        m.refresh(f, True)
    m.finish()
    assert (m.shown, m.dropped) == (3, 1), "backwards: the skipped frame 7 is dropped"
    m.start(1.0)  # egui's meter_resync_does_not_count_hidden_time
    m.refresh(0, True)
    m.resync(50, False)  # hidden from 1 to 49: neither shown nor dropped
    m.refresh(50, True)
    m.refresh(51, True)
    m.finish()
    assert (m.shown, m.dropped) == (2, 0)


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_perf_stats_counts_shown_and_dropped_frames(app, tmp_path: Path):
    """``perf.stats`` adds egui's ``playback``: playing, speed, shown, dropped, dropRate,
    resolution and draft decoding of the Program monitor (and the Source monitor's under
    ``source``); the counts cover the last playback."""
    from cfa4_qt.control import ControlServer
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.show()
    engine.start()
    server = ControlServer(win)
    mon = win.monitor
    replies: list[tuple[bool, Any]] = []

    def stats() -> dict[str, Any]:
        replies.clear()
        server.dispatch("perf.stats", {}, lambda ok, r: replies.append((ok, r)))
        assert wait_until(app, lambda: bool(replies), 20_000)
        ok, r = replies[0]
        assert ok is True, r
        return r

    try:
        assert wait_until(app, lambda: win.sequence is not None and mon.has_content(), 60_000)
        before = stats()["playback"]
        assert before["playing"] is False and before["shown"] == 0 and before["dropped"] == 0 and before["dropRate"] == 0.0
        assert before["resolution"] in ("Full", "1/2", "1/4", "1/8", "1/16") and isinstance(before["draftDecode"], bool)
        assert before["source"]["playing"] is False
        mon.play()
        assert wait_until(app, lambda: mon.meter.counts()[0] >= 5, 30_000), mon.meter.counts()
        during = stats()["playback"]
        assert during["playing"] is True and during["speed"] == 1.0 and during["shown"] >= 5
        mon.stop()
        after = stats()
        p = after["playback"]
        assert p["playing"] is False and p["shown"] >= during["shown"]
        assert p["dropRate"] == pytest.approx(p["dropped"] / (p["shown"] + p["dropped"]))
        # drawing the picture while it played (qt-parity 5.5): raster on Qt's offscreen platform
        assert p["present"]["path"] == "raster" and p["present"]["frames"] >= 1, p["present"]
        assert 0 <= p["present"]["meanMs"] <= p["present"]["maxMs"], p["present"]
        assert "frames" in after or len(after) > 1, "the engine's own stats are kept"
        # a loop starting its range again is the same playback: the counts go on
        shown = mon.meter.counts()[0]
        mon.play()
        assert wait_until(app, lambda: mon.meter.counts()[0] >= 2, 30_000)
        mon._halt()
        mon.playhead = 0
        mon._loop_restart = True
        try:
            mon.play()
        finally:
            mon._loop_restart = False
        assert mon.meter.counts()[0] >= 2, "kept across the loop restart"
        assert mon.meter.current is not None and mon.meter.current[0] == 0, "the loop's first frame falls due"
        mon.stop()
        # a new playback starts the counts again (with the frame already on screen); the speed stays
        mon.play(2.0)
        assert mon.meter.counts()[0] + mon.meter.counts()[1] <= 1, (mon.meter.counts(), shown)
        mon.stop()
        p = stats()["playback"]
        assert p["playing"] is False and p["speed"] == 2.0, p
        # the picture on screen counts as a frame only while it is that frame of the current edit
        assert wait_until(app, lambda: mon._shows(mon.playhead), 20_000)
        mon.revision += 1
        assert not mon._shows(mon.playhead), "a picture from an older edit is stale"
        mon.revision -= 1
        # a stall up to the stop point: the frames passed over still fell due (dropped)
        mon.play()
        assert wait_until(app, lambda: mon.meter.counts()[0] >= 1, 30_000)
        ft = mon.frame()
        due = mon.meter.current[0] if mon.meter.current else mon.playhead // ft
        dropped = mon.meter.dropped
        mon.stop_at = (due + 12) * ft + ft // 2  # between two ticks of the frame grid
        mon._play_from += 20 * ft  # the clock is past the stop point at the next tick
        refreshed: list[int] = []
        refresh = mon._meter_refresh
        mon._meter_refresh = lambda frame, exact: (refreshed.append(frame), refresh(frame, exact))  # type: ignore[method-assign]
        try:
            mon._tick()
        finally:
            del mon._meter_refresh
        assert not mon.playing
        assert refreshed == [due + 12], ("the last frame due before the stop point", refreshed, due)
        assert mon.meter.dropped - dropped >= 10, (mon.meter.shown, mon.meter.dropped, dropped)
        # a hidden monitor: neither the hidden frames nor the one due when it shows again count
        mon.meter.start(1.0)
        mon._meter_hidden = False
        mon._meter_refresh(10, True)
        mon.view.isVisible = lambda: False  # type: ignore[method-assign]
        try:
            mon._meter_refresh(40, False)
            mon._meter_refresh(60, False)
        finally:
            del mon.view.isVisible
        mon._meter_refresh(80, False)
        mon._meter_refresh(81, True)
        mon.meter.finish()
        assert (mon.meter.shown, mon.meter.dropped) == (1, 1), (mon.meter.shown, mon.meter.dropped)
        # stopped while hidden: the frame due then is neither shown nor dropped
        mon.play()
        mon.meter.start(1.0)
        mon._meter_refresh(10, True)
        mon.view.isVisible = lambda: False  # type: ignore[method-assign]
        try:
            mon._meter_refresh(40, False)
            mon.stop()
        finally:
            del mon.view.isVisible
        assert (mon.meter.shown, mon.meter.dropped) == (0, 0), (mon.meter.shown, mon.meter.dropped)
    finally:
        win._quit_confirmed = True
        win.close()
        engine.stop()
