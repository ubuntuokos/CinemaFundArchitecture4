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
