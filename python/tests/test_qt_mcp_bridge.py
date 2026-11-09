"""CFA4 Qt 6 GUI driven over MCP (``docs/qt-parity.md`` 1.11): ``cfa4-cli mcp --bridge`` speaks to the
Qt window's control channel as it does to egui's, so an agent's MCP tools (commands, the document,
the ``ui_*`` tools, screenshots, a long export) work on the Qt GUI too."""

from __future__ import annotations

import base64
import json
import os
import queue
import subprocess
import threading
from pathlib import Path
from typing import Any

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
QtCore = pytest.importorskip("PySide6.QtCore")
QtWidgets = pytest.importorskip("PySide6.QtWidgets")

from cfa4_qt.engine import EngineClient, locate_engine

REAL = locate_engine()
PNG = b"\x89PNG\r\n\x1a\n"


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


class McpClient:
    """``cfa4-cli mcp --bridge`` on stdio; the Qt event loop keeps running while it waits, since
    the control server it bridges to lives in this process."""

    def __init__(self, app, cli: Path, port: int) -> None:
        self.app = app
        self.proc = subprocess.Popen(
            [str(cli), "mcp", "--bridge", f"127.0.0.1:{port}"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        self.lines: queue.Queue[dict[str, Any]] = queue.Queue()
        self.seen: list[dict[str, Any]] = []
        self.n = 0
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self) -> None:
        assert self.proc.stdout is not None
        for line in self.proc.stdout:
            if line.strip():
                self.lines.put(json.loads(line))

    def send(self, msg: dict[str, Any]) -> None:
        assert self.proc.stdin is not None
        self.proc.stdin.write(json.dumps(msg).encode() + b"\n")
        self.proc.stdin.flush()

    def ask(self, method: str, params: dict[str, Any] | None = None, timeout_ms: int = 30_000) -> dict[str, Any]:
        self.n += 1
        n = self.n
        self.send({"jsonrpc": "2.0", "id": n, "method": method, "params": params or {}})
        got: list[dict[str, Any]] = []

        def answered() -> bool:
            while not self.lines.empty():
                m = self.lines.get()
                self.seen.append(m)
                if m.get("id") == n:
                    got.append(m)
            return bool(got)

        assert wait_until(self.app, answered, timeout_ms), f"no answer to {method}"
        assert "error" not in got[0], got[0]
        return got[0]["result"]

    def init(self) -> dict[str, Any]:
        r = self.ask("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "qt-test", "version": "1"}})
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        return r

    def tool(self, name: str, arguments: dict[str, Any] | None = None, timeout_ms: int = 30_000) -> dict[str, Any]:
        r = self.ask("tools/call", {"name": name, "arguments": arguments or {}}, timeout_ms)
        assert r.get("isError") is not True, (name, r)
        return r

    def json(self, name: str, arguments: dict[str, Any] | None = None, timeout_ms: int = 30_000) -> Any:
        r = self.tool(name, arguments, timeout_ms)
        return json.loads(next(c["text"] for c in r["content"] if c["type"] == "text"))

    def png(self, name: str, arguments: dict[str, Any] | None = None) -> bytes:
        r = self.tool(name, arguments)
        image = next(c for c in r["content"] if c["type"] == "image")
        assert image["mimeType"] == "image/png"
        return base64.b64decode(image["data"])

    def close(self) -> None:
        if self.proc.stdin is not None:
            self.proc.stdin.close()
        try:
            self.proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_mcp_bridge_drives_the_qt_window(app, tmp_path: Path):
    """egui's MCP bridge on the Qt window: command_list / command_run / doc_inspect reach the
    engine through the window, the ui_* tools read and operate it, ui_screenshot and
    render_preview give PNGs (the latter put the playhead back), and a long export
    (``file.exportMedia`` with ``wait``) runs as an app job until its file is written."""
    from cfa4_qt.control import ControlServer
    from cfa4_qt.main_window import MainWindow
    from cfa4_qt.panels.tools import GROUPS

    razor_id = f"tools.group.{next(i for i, g in enumerate(GROUPS) if g[0].id == 'razor')}"
    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1400, 900)
    win.show()
    engine.start()
    server = ControlServer(win)
    assert server.listen(0)
    mcp = McpClient(app, REAL, server.port())
    try:
        assert wait_until(app, lambda: bool(win.sequence) and len(win.registry.commands) > 100, 60_000)
        assert mcp.init()["serverInfo"]["name"] == "cfa4"
        # engine commands through the window
        cmds = mcp.json("command_list")
        ids = {c["id"] for c in (cmds if isinstance(cmds, list) else cmds.get("commands", []))}
        assert "sequence.addEdit" in ids and "file.exportMedia" in ids
        mcp.json("command_run", {"id": "project.setNotes", "params": {"text": "from MCP"}})
        assert wait_until(app, lambda: win.notes.edit.toPlainText() == "from MCP", 20_000)
        doc = mcp.json("doc_inspect")
        assert doc["sequence"] is not None and doc["sequence"].get("id") == win.sequence.get("id")
        assert doc["project"]["activeSequence"] == win.sequence.get("id")
        # the ui_* tools
        assert mcp.json("ui_inspect")["tool"] == "selection"
        els = mcp.json("ui_elements", {"prefix": "tools."})
        assert any(e["id"] == razor_id for e in els)
        mcp.json("ui_click", {"id": razor_id})
        assert wait_until(app, lambda: win.timeline.canvas.tool == "razor", 5_000)
        assert mcp.json("ui_control", {"method": "ui.set", "params": {"tool": "selection"}})["tool"] == "selection"
        assert win.timeline.canvas.tool == "selection"
        # the bridge sends each optional field, null when not given: the defaults apply, as in egui
        mcp.json("ui_drag", {"from": {"id": razor_id}, "to": {"id": razor_id}})
        assert wait_until(app, lambda: win.timeline.canvas.tool == "razor", 5_000)
        mcp.json("ui_control", {"method": "ui.set", "params": {"tool": "selection"}})
        mcp.json("ui_control", {"method": "ui.focus", "params": {"id": "project.search"}})
        mcp.json("ui_type", {"text": "abc"})
        assert wait_until(app, lambda: win.project_panel.filter.text() == "abc", 5_000)
        win.project_panel.filter.clear()
        # pictures: the window, a panel, and the Program monitor at a time, the playhead put back
        assert mcp.png("ui_screenshot").startswith(PNG)
        assert mcp.png("ui_screenshot", {"panel": "Timeline"}).startswith(PNG)
        before = win.sequence.get("playhead")
        assert mcp.png("render_preview").startswith(PNG)
        assert mcp.png("render_preview", {"seconds": 1.0}).startswith(PNG)
        assert wait_until(app, lambda: (win.sequence or {}).get("playhead") == before, 10_000), "the playhead is put back"
        # a long export: started as an app job and polled until the file is written
        out = tmp_path / "from-mcp.mp4"
        params = {"path": str(out), "format": "h264", "width": 320, "height": 180, "audio": False,
                  "range": "custom", "startSeconds": 0, "endSeconds": 0.5, "wait": True}
        r = mcp.json("command_run", {"id": "file.exportMedia", "params": params}, timeout_ms=120_000)
        assert isinstance(r.get("job"), int) and "result" in r, r
        assert out.exists() and out.stat().st_size > 0
    finally:
        mcp.close()
        server.close()
        win._quit_confirmed = True
        win.close()
        engine.stop()
