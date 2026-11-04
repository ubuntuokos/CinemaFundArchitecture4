"""CFA4 Qt 6 GUI frames through shared memory (``docs/qt-parity.md`` 5.5): the monitors ask
``frame.render`` for ``shm`` frames and show them straight from the engine's mapped ring, without a
copy through the pipe or into a QImage; anything unexpected falls back to the pipe."""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
QtCore = pytest.importorskip("PySide6.QtCore")
QtWidgets = pytest.importorskip("PySide6.QtWidgets")

from cfa4_qt.engine import SHM_PREFIX, EngineClient, FrameMaps, locate_engine

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


def test_frame_maps_take_only_the_engines_ring_files(tmp_path: Path):
    """A ring file is mapped once, removed from its directory, and read in place; other paths,
    symlinks, out-of-range slots and malformed replies give None."""
    ring = tmp_path / f"{SHM_PREFIX}1-program-0-0"
    ring.write_bytes(bytes(range(256)) * 16)
    os.chmod(ring, 0o600)
    maps = FrameMaps()
    assert maps.enabled
    first = maps.view({"path": str(ring), "offset": 256, "len": 256})
    assert first is not None and bytes(first) == bytes(range(256))
    assert not ring.exists(), "removed once mapped"
    again = maps.view({"path": str(ring), "offset": 512, "len": 4})
    assert again is not None and bytes(again) == bytes([0, 1, 2, 3]), "the mapping is kept"
    assert maps.view({"path": str(ring), "offset": 4000, "len": 200}) is None, "past the end"
    assert maps.view({"path": str(ring), "offset": -1, "len": 4}) is None
    other = tmp_path / "secret"
    other.write_bytes(b"x" * 64)
    link = tmp_path / f"{SHM_PREFIX}link"
    link.symlink_to(other)
    for bad in (
        {"path": str(other), "offset": 0, "len": 4},
        {"path": str(link), "offset": 0, "len": 4},
        {"path": f"relative/{SHM_PREFIX}x", "offset": 0, "len": 4},
        {"path": str(ring), "offset": "0", "len": 4},
        {"path": str(ring), "offset": 0, "len": True},
        None,
        "path",
    ):
        assert maps.view(bad) is None, bad
    assert other.exists() and link.is_symlink(), "nothing else is touched"


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_monitor_shows_frames_from_shared_memory(app, tmp_path: Path, monkeypatch):
    """The Program monitor's frames come through the shared-memory ring: no binary tail, the
    picture is the engine's frame, the ring file leaves the directory once mapped and none is
    left when the engine ends; when mapping fails the monitor asks through the pipe again."""
    from cfa4_qt.main_window import MainWindow

    monkeypatch.setenv("XDG_RUNTIME_DIR", str(tmp_path / "run"))
    (tmp_path / "run").mkdir(mode=0o700)
    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1200, 800)
    win.show()
    engine.start()
    mon = win.monitor
    replies: list[tuple[dict[str, Any], dict[str, Any], int]] = []
    real_call = engine.call

    def spy(method: str, params: dict[str, Any] | None = None, on_reply: Any = None, on_error: Any = None) -> int:
        if method != "frame.render":
            return real_call(method, params, on_reply, on_error)
        asked = dict(params or {})

        def got(result: Any, data: bytes | None) -> None:
            if isinstance(result, dict):
                replies.append((asked, result, len(data or b"")))
            if on_reply is not None:
                on_reply(result, data)

        return real_call(method, params, got, on_error)

    engine.call = spy  # type: ignore[method-assign]
    try:
        assert wait_until(app, lambda: win.sequence is not None and mon.view.image is not None, 60_000)
        mon.seek(mon.frame() * 48)
        assert wait_until(app, lambda: any(r[0].get("time") == mon.frame() * 48 for r in replies), 20_000)
        asked, result, tail = next(r for r in replies if r[0].get("time") == mon.frame() * 48)
        assert asked.get("shm") == "program"
        assert "shm" in result and tail == 0, "no binary tail"
        image = mon.view.image
        assert image is not None and (image.width(), image.height()) == (result["w"], result["h"])
        assert mon._frame_memory is not None and len(mon._frame_memory) == result["w"] * result["h"] * 4
        assert not os.path.exists(result["shm"]["path"]), "removed once mapped"
        # the same frame through the pipe is the picture shown
        out: list[bytes] = []
        params = {k: v for k, v in asked.items() if k != "shm"}
        real_call("frame.render", params, on_reply=lambda _r, data: out.append(data or b""))
        assert wait_until(app, lambda: out, 10_000)
        shown = image.constBits()
        assert bytes(shown) == out[0], "the engine's frame, read in place"

        # mapping fails: the monitor asks again through the pipe and keeps showing frames. Nothing of
        # the monitor's may still be in flight: a frame it asked for before (a resize re-render)
        # failing to map would turn shared memory off before the seek asks through it
        assert wait_until(app, lambda: not mon._frame_busy and mon._frame_wanted is None, 20_000)
        monkeypatch.setattr(FrameMaps, "_map", staticmethod(lambda _path: None))
        engine.frames._maps.clear()
        replies.clear()
        mon.seek(mon.frame() * 96)
        assert wait_until(app, lambda: any(r[0].get("time") == mon.frame() * 96 and r[2] > 0 for r in replies), 20_000)
        assert not engine.frames.enabled
        assert [("shm" in r[0]) for r in replies if r[0].get("time") == mon.frame() * 96] == [True, False], "asked again"
        replies.clear()
        mon.seek(mon.frame() * 120)
        assert wait_until(app, lambda: any(r[0].get("time") == mon.frame() * 120 for r in replies), 20_000)
        asked, result, tail = next(r for r in replies if r[0].get("time") == mon.frame() * 120)
        assert "shm" not in asked and tail == result["w"] * result["h"] * 4, "through the pipe"
    finally:
        engine.call = real_call  # type: ignore[method-assign]
        win._quit_confirmed = True
        win.close()
        engine.stop()
    left = [p.name for p in (tmp_path / "run").iterdir() if p.name.startswith(SHM_PREFIX)]
    assert not left, f"ring files left behind: {left}"
