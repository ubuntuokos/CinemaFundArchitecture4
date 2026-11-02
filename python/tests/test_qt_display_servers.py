"""qt-parity 1.9: the Qt GUI on a real display server, X11 (Xvfb, the ``xcb`` platform) and Wayland
(weston's headless backend, the ``wayland`` platform), not only Qt's ``offscreen`` platform.

Each case starts ``python -m cfa4_qt --demo --control 0`` against its own display server, drives it
through the control channel (``ui.inspect``, ``ui.resize`` through the window system, ``ui.key``,
``ui.screenshot``) and quits with ``app.quit``; the process must exit cleanly. A case skips when its
display server or PySide6's platform plugin is missing, and fails instead under
``CFA4_REQUIRE_DISPLAY_SERVERS=1`` (CI's Qt 6 GUI job, which installs them). Without a built
engine, ``test_qt_gui``'s fake one serves the window. The real desktops of every Ubuntu 26.04
flavour stay a manual check (VISION §3).
"""

from __future__ import annotations

import ast
import base64
import json
import os
import re
import shutil
import socket
import stat
import subprocess
import sys
import tempfile
import textwrap
import threading
import time
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]


def _engine(tmp_path: Path) -> str:
    """The real engine when there is one (CFA4_ENGINE, target/, PATH), else ``test_qt_gui``'s fake
    ``cfa4-cli serve`` (CI's Qt job builds no Rust): the display server is what is under test."""
    path = os.environ.get("CFA4_ENGINE", "").strip()
    if path and os.access(path, os.X_OK):
        return path
    for build in ("release", "debug"):
        candidate = ROOT / "target" / build / "cfa4-cli"
        if os.access(candidate, os.X_OK):
            return str(candidate)
    found = shutil.which("cfa4-cli")
    if found:
        return found
    tree = ast.parse((Path(__file__).with_name("test_qt_gui.py")).read_text(encoding="utf-8"))
    assign = next(node for node in tree.body if isinstance(node, ast.Assign)
                  and any(isinstance(t, ast.Name) and t.id == "FAKE_ENGINE" for t in node.targets))
    source = max((c.value for c in ast.walk(assign) if isinstance(c, ast.Constant) and isinstance(c.value, str)), key=len)
    fake = tmp_path / "fake-cfa4-cli"
    fake.write_text(textwrap.dedent(source).replace("#!PYTHON", "#!" + sys.executable).lstrip(), encoding="utf-8")
    fake.chmod(fake.stat().st_mode | stat.S_IXUSR)
    return str(fake)


def _skip(reason: str) -> None:
    """Skip, or fail where CI promised the display servers (``CFA4_REQUIRE_DISPLAY_SERVERS=1``)."""
    if os.environ.get("CFA4_REQUIRE_DISPLAY_SERVERS") == "1":
        pytest.fail(reason)
    pytest.skip(reason)


def _plugin_loads(platform: str, env: dict[str, str]) -> bool:
    """Whether PySide6's platform plugin starts on this display (its system libraries are there)."""
    code = "from PySide6.QtWidgets import QApplication; QApplication([]); print('ok')"
    run = subprocess.run([sys.executable, "-c", code], env={**env, "QT_QPA_PLATFORM": platform},
                         capture_output=True, text=True, timeout=60, check=False)
    return run.returncode == 0 and "ok" in run.stdout


@contextmanager
def _x11() -> Iterator[dict[str, str]]:
    xvfb = shutil.which("Xvfb")
    if xvfb is None:
        _skip("Xvfb is not installed (apt install xvfb)")
    read, write = os.pipe()
    server = subprocess.Popen([xvfb, "-displayfd", str(write), "-screen", "0", "1600x1000x24", "-nolisten", "tcp"],
                              pass_fds=(write,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    os.close(write)
    try:
        with os.fdopen(read) as fd:
            number = fd.readline().strip()
        if not number.isdigit():
            _skip("Xvfb did not start")
        env = {k: v for k, v in os.environ.items() if k not in ("WAYLAND_DISPLAY", "QT_QPA_PLATFORM")}
        yield {**env, "DISPLAY": f":{number}"}
    finally:
        server.terminate()
        server.wait(timeout=10)


@contextmanager
def _wayland() -> Iterator[dict[str, str]]:
    weston = shutil.which("weston")
    if weston is None:
        _skip("weston is not installed (apt install weston)")
    with tempfile.TemporaryDirectory(prefix="cfa4-wayland-") as runtime:
        os.chmod(runtime, 0o700)
        env = {k: v for k, v in os.environ.items() if k not in ("DISPLAY", "QT_QPA_PLATFORM")}
        env["XDG_RUNTIME_DIR"] = runtime
        server = subprocess.Popen([weston, "--backend=headless", "--socket=cfa4-test", "--width=1600", "--height=1000",
                                   "--idle-time=0"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            socket_path = Path(runtime) / "cfa4-test"
            deadline = time.monotonic() + 20
            while not socket_path.exists() and server.poll() is None and time.monotonic() < deadline:
                time.sleep(0.05)
            if not socket_path.exists():
                _skip("weston's headless backend did not start")
            yield {**env, "WAYLAND_DISPLAY": "cfa4-test"}
        finally:
            server.terminate()
            server.wait(timeout=10)


class _Client:
    def __init__(self, port: int) -> None:
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=60)
        self.file = self.sock.makefile("rb")
        self.n = 0

    def call(self, method: str, params: dict | None = None) -> dict:
        self.n += 1
        self.sock.sendall(json.dumps({"id": self.n, "method": method, "params": params or {}}).encode() + b"\n")
        reply = json.loads(self.file.readline())
        assert reply.get("id") == self.n, reply
        assert reply.get("ok") is True, reply
        return reply.get("result")

    def close(self) -> None:
        self.file.close()
        self.sock.close()


def _drive(platform: str, env: dict[str, str], data_dir: Path) -> None:
    engine = _engine(data_dir)
    env = {**env, "QT_QPA_PLATFORM": platform, "PYTHONPATH": str(ROOT / "python"), "CFA4_ENGINE": engine}
    env.pop("CFA4_CONTROL_PORT", None)
    if not _plugin_loads(platform, env):
        _skip(f"PySide6's {platform} platform plugin does not load here (its system libraries are missing)")
    app = subprocess.Popen([sys.executable, "-m", "cfa4_qt", "--demo", "--no-autosave", "--data-dir", str(data_dir),
                            "--lang", "en", "--control", "0"],
                           env=env, cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    client = None
    try:
        assert app.stderr is not None
        log: list[str] = []
        ports: list[int] = []
        started = threading.Event()

        def read_log() -> None:  # keeps the pipe drained, so a chatty run never blocks on it
            for line in app.stderr or ():
                log.append(line)
                found = re.search(r"control channel on 127\.0\.0\.1:(\d+)", line)
                if found and not ports:
                    ports.append(int(found.group(1)))
                    started.set()
            started.set()

        threading.Thread(target=read_log, daemon=True).start()
        started.wait(timeout=90)
        assert ports, "".join(log) or "cfa4-qt did not open its control channel"
        port = ports[0]
        client = _Client(port)
        deadline = time.monotonic() + 60
        while not client.call("engine.commands") and time.monotonic() < deadline:
            time.sleep(0.2)
        state = client.call("ui.inspect")
        assert state["tool"] == "selection"
        # through the window system: the size the server gave the window comes back
        size = client.call("ui.resize", {"width": 1280, "height": 800})
        assert (size["width"], size["height"]) == (1280, 800), size
        # a tool shortcut reaches the timeline without the window taking the keyboard from the
        # desktop (Wayland never lets a client take it itself)
        client.call("ui.focus", {"id": "timeline.canvas"})
        focused = client.call("ui.inspect")["input"]["nativeWindowFocused"]
        client.call("ui.key", {"key": "C"})
        state = client.call("ui.inspect")
        assert state["tool"] == "razor"
        # routing the key does not make the window claim the desktop's keyboard focus
        assert state["input"]["nativeWindowFocused"] == focused
        shot = client.call("ui.screenshot")
        assert (shot["width"], shot["height"]) == (1280, 800)
        png = base64.b64decode(shot["png"])
        assert png.startswith(b"\x89PNG") and len(png) > 20_000, "the window drew its panels"
        client.call("app.quit")
        assert app.wait(timeout=60) == 0
    finally:
        if client is not None:
            client.close()
        if app.poll() is None:
            app.kill()
            app.wait(timeout=10)


def test_qt_gui_on_x11(tmp_path: Path) -> None:
    with _x11() as env:
        _drive("xcb", env, tmp_path)


def test_qt_gui_on_wayland(tmp_path: Path) -> None:
    with _wayland() as env:
        _drive("wayland", env, tmp_path)
