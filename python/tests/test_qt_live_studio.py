"""Live studio preview (CFA4-LIVE-STUDIO-001 S0): the Qt6 window starts the engine's live studio
host and shows the empty scene's program frames; a crashed host is restarted by the engine and
the window keeps showing frames, without an error of its own."""

from __future__ import annotations

import os
import signal
from collections.abc import Callable, Iterator
from typing import Any

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
QtCore = pytest.importorskip("PySide6.QtCore")
QtWidgets = pytest.importorskip("PySide6.QtWidgets")

from cfa4_qt.engine import EngineClient, locate_engine
from cfa4_qt.panels.live_studio import LiveStudioPreview, rgba_image, status_text

REAL = locate_engine()


@pytest.fixture(scope="module")
def app() -> Iterator[Any]:
    yield QtWidgets.QApplication.instance() or QtWidgets.QApplication([])


def wait_until(app: Any, predicate: Callable[[], object], timeout_ms: int = 15_000) -> bool:
    timer = QtCore.QElapsedTimer()
    timer.start()
    while not predicate():
        if timer.elapsed() > timeout_ms:
            return False
        app.processEvents(QtCore.QEventLoop.ProcessEventsFlag.AllEvents, 20)
    return True


def test_frames_are_checked_before_they_are_shown(app: Any) -> None:
    meta = {"w": 4, "h": 2, "format": "rgba8"}
    image = rgba_image(meta, bytes([10, 20, 30, 255]) * 8)
    assert image is not None and (image.width(), image.height()) == (4, 2)
    assert image.pixelColor(0, 0).red() == 10
    cases: list[tuple[Any, bytes | None]] = [
        (meta, bytes(31)),  # short
        ({**meta, "format": "bgra8"}, bytes(32)),
        ({**meta, "w": 0}, bytes(0)),
        ({**meta, "w": True}, bytes(32)),
        ({**meta, "h": 100_000}, bytes(32)),
        ("frame", bytes(32)),
        (meta, None),
    ]
    for bad_meta, data in cases:
        assert rgba_image(bad_meta, data) is None


def test_status_lines() -> None:
    assert "stopped" in status_text(None).lower() or "leállítva" in status_text(None)
    line = status_text(
        {"state": "running", "backend": "libobs", "version": "33.0.1", "crashes": 2, "lastExit": "signal: 9 (SIGKILL)"}
    )
    assert "libobs 33.0.1" in line and "SIGKILL" in line
    assert "nope" in status_text({"state": "running", "backend": "software", "fallbackReason": "nope"})
    assert "gave up" in status_text({"state": "failed", "error": "gave up"})


@pytest.mark.skipif(REAL is None, reason="needs a built cfa4-cli (and cfa4-studio-host next to it)")
def test_the_window_shows_the_empty_scene_and_survives_a_host_crash(app: Any) -> None:
    assert REAL is not None
    engine = EngineClient(REAL)
    engine.start()
    window = LiveStudioPreview(engine)
    try:
        window.backend.setCurrentIndex(window.backend.findData("software"))
        window.resize(480, 320)
        window.show()
        window.start_button.click()
        assert wait_until(app, lambda: window._frame is not None), window.status.text()
        frame = window._frame
        assert frame is not None and (frame.width(), frame.height()) == (1920, 1080)
        assert frame.pixelColor(10, 10).black() == 255, "the empty scene is black"
        assert not window.picture.pixmap().isNull()

        # kill the host: the engine restarts it, the window carries on showing frames
        status: dict[str, Any] = {}
        engine.call("live.studio.host.status", {}, on_reply=lambda r, _b: status.update(r))
        assert wait_until(app, lambda: isinstance(status.get("pid"), int))
        os.kill(status["pid"], signal.SIGKILL)
        assert wait_until(app, lambda: "1" in window.status.text() and "SIGKILL" in window.status.text()), (
            window.status.text()
        )
        seen = window._last_seq
        assert wait_until(app, lambda: window._last_seq != seen), "frames again after the restart"
        assert engine.running, "the engine is unaffected"

        window.stop_button.click()
        assert wait_until(app, lambda: not window.frame_timer.isActive())
    finally:
        window.close()
        engine.stop()
