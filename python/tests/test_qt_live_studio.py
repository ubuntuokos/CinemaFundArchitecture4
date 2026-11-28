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
from cfa4_qt.panels.live_studio_workspace import StudioWorkspace, scenes_of, sources_of

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


# ----- S1c: the Live Studio workspace -----


def test_scene_and_source_replies_are_read_defensively() -> None:
    assert scenes_of(None) == ([], None)
    assert scenes_of({"scenes": "x"}) == ([], None)
    scenes, current = scenes_of({"current": 2, "scenes": [{"id": 2, "name": "A"}, "junk", {"name": "no id"}]})
    assert [s["id"] for s in scenes] == [2] and current == 2
    assert scenes_of({"current": True, "scenes": []})[1] is None
    # the stack is listed bottom first; the list shows the top first
    assert [s["id"] for s in sources_of({"sources": [{"id": 1}, {"id": 2}, 3]})] == [2, 1]
    assert sources_of(None) == []


class FakeEngine:
    """Answers ``live.studio.*`` commands from a small in-memory studio, like the engine does."""

    def __init__(self) -> None:
        self.running = True
        self.calls: list[tuple[str, dict[str, Any]]] = []
        self.scenes: list[dict[str, Any]] = []
        self.current: int | None = None
        self.next_id = 1
        self.event = type("Sig", (), {"connect": lambda self, f: None})()
        self.frames = type("F", (), {"enabled": False})()

    def call(self, *_a: Any, **_k: Any) -> int:
        return 0

    def execute(
        self, command: str, params: dict[str, Any] | None = None, on_reply: Any = None, on_error: Any = None
    ) -> int:
        params = params or {}
        self.calls.append((command, params))
        result: Any = {}
        if command == "live.studio.scene.add":
            sid = self.next_id
            self.next_id += 1
            self.scenes.append({"id": sid, "name": params.get("name") or f"Scene {sid}", "sources": []})
            self.current = self.current or sid
        elif command == "live.studio.scene.select":
            self.current = params["id"]
        elif command == "live.studio.source.add":
            sid = self.next_id
            self.next_id += 1
            kind = {"type": params["type"], **{k: v for k, v in params.items() if k in ("path", "color")}}
            src = {
                "id": sid,
                "name": params["type"],
                "type": params["type"],
                "kind": kind,
                "visible": True,
                "transform": {"x": 0.0, "y": 0.0, "scaleX": 1.0, "scaleY": 1.0, "rotation": 0.0, "crop": [0, 0, 0, 0]},
            }
            next(s for s in self.scenes if s["id"] == self.current)["sources"].append(src)
            result = {"source": src}
        elif command == "live.studio.source.set":
            for sc in self.scenes:
                for src in sc["sources"]:
                    if src["id"] == params["id"]:
                        if "visible" in params:
                            src["visible"] = params["visible"]
                        if "name" in params:
                            src["name"] = params["name"]
                        src["transform"].update(params.get("transform", {}))
        elif command == "live.studio.source.remove":
            for sc in self.scenes:
                sc["sources"] = [s for s in sc["sources"] if s["id"] != params["id"]]
        elif command == "live.studio.scenes":
            result = {"current": self.current, "scenes": self.scenes}
        if on_reply is not None:
            on_reply(result, None)
        return 0


def test_workspace_edits_through_engine_commands(app: Any) -> None:
    engine = FakeEngine()
    ws = StudioWorkspace(engine, auto_start=False)  # type: ignore[arg-type]
    try:
        ws.refresh()
        assert ws.scene_list.count() == 0 and not ws.source_add.isEnabled()
        ws.add_scene("Opening")
        assert wait_until(app, lambda: ws.scene_list.count() == 1)
        assert ws.source_add.isEnabled()
        ws.add_source("color")
        ws.add_source("text")
        assert wait_until(app, lambda: ws.source_list.count() == 2)
        assert ws.source_list.item(0).text() == "text", "the newest source is on top"
        assert ws.props.isEnabled() and ws.selected_source == engine.scenes[0]["sources"][1]["id"]
        # visibility goes through the checkbox of the list row
        ws.source_list.item(0).setCheckState(QtCore.Qt.CheckState.Unchecked)
        assert engine.scenes[0]["sources"][1]["visible"] is False
        # a transform edit is one merged undo step
        ws.transform_widgets["x"].setValue(120.0)
        cmd, params = engine.calls[-1]
        assert (
            cmd == "live.studio.source.set" and params["transform"] == {"x": 120.0} and params["merge"] == "transform.x"
        )
        # a type-specific form per source type
        ws.source_list.setCurrentRow(1)
        assert "color" in ws.kind_widgets and "text" not in ws.kind_widgets
        ws.remove_source()
        assert wait_until(app, lambda: ws.source_list.count() == 1)
        # selecting another scene puts it on the program
        ws.add_scene("Second")
        assert wait_until(app, lambda: ws.scene_list.count() == 2)
        ws.scene_list.setCurrentRow(1)
        assert ("live.studio.scene.select", {"id": engine.scenes[1]["id"]}) in engine.calls
        assert engine.current == engine.scenes[1]["id"]
    finally:
        ws.close()


@pytest.mark.skipif(
    REAL is None or not (os.environ.get("CFA4_STUDIO_HOST") or (REAL.parent / "cfa4-studio-host").exists()),
    reason="needs a built cfa4-cli and cfa4-studio-host",
)
def test_workspace_with_the_real_engine(app: Any) -> None:
    assert REAL is not None
    engine = EngineClient(REAL)
    engine.start()
    ws = StudioWorkspace(engine, auto_start=False)
    try:
        ws.preview.backend.setCurrentIndex(ws.preview.backend.findData("software"))
        ws.resize(900, 600)
        ws.show()
        ws.add_scene("One")
        assert wait_until(app, lambda: ws.scene_list.count() == 1)
        ws.add_source("color")
        assert wait_until(app, lambda: ws.source_list.count() == 1 and ws.selected_source is not None), (
            ws.message.text()
        )
        ws.visible_box.setChecked(False)
        assert wait_until(app, lambda: ws.source_list.item(0).checkState() == QtCore.Qt.CheckState.Unchecked)
        ws.add_source("image")  # needs a file: the engine says so, the workspace shows it and stays usable
        assert wait_until(app, lambda: bool(ws.message.text()))
        assert ws.source_list.count() == 1 and engine.running
        # undo from outside (the main window's Edit menu) refreshes the lists on projectChanged
        engine.execute("edit.undo")
        engine.execute("edit.undo")
        assert wait_until(app, lambda: ws.source_list.count() == 0), "undo of the source is shown"
        ws.preview.start_button.click()
        assert wait_until(app, lambda: ws.preview._frame is not None), ws.preview.status.text()
    finally:
        ws.close()
        engine.stop()
