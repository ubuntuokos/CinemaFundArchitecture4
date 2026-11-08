"""CFA4 Qt 6 GUI export queue order against the real Rust engine (when it is built):
``docs/qt-parity.md`` 4.2, egui's queue Up / Down (``export.queue.move``)."""

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


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_export_queue_up_and_down_as_egui(app, tmp_path: Path):
    """Up / Down move the selected export in the queue (the engine's order, which the queue
    encodes in); the selection follows it; Up is off at the top, Down at the bottom, and both
    with no single export selected."""
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.show()
    engine.start()

    def done_exec(cid: str, params: dict[str, Any]) -> Any:
        out: list = []
        engine.execute(cid, params, on_reply=lambda r, _b: out.append(r), on_error=lambda m: out.append(("error", m)))
        assert wait_until(app, lambda: out, 10_000), cid
        assert not (isinstance(out[0], tuple) and out[0][0] == "error"), out[0]
        return out[0]

    dlg = None
    try:
        assert wait_until(app, lambda: win.sequence is not None and len(win.registry.commands) > 100, 60_000)
        names = ["a.mov", "b.mov", "c.mov"]
        for n in names:
            done_exec("export.queue.add", {"preset": "Apple ProRes 422 HQ", "path": str(tmp_path / n), "range": "entire"})
        win.run_command(win.registry.commands["file.exportMedia"])
        dlg = win.export_dialog
        assert dlg is not None and dlg.isVisible()

        def order() -> list[str]:
            return [Path(dlg.table.item(r, 1).text()).name for r in range(dlg.table.rowCount())]

        assert wait_until(app, lambda: order() == names, 10_000), order()
        up, down = dlg.queue_up, dlg.queue_down
        assert (up.objectName(), down.objectName()) == ("export.queue.up", "export.queue.down")
        assert not up.isEnabled() and not down.isEnabled(), "nothing selected"

        dlg.table.selectRow(2)
        assert up.isEnabled() and not down.isEnabled(), "the last one goes up only"
        up.click()
        assert wait_until(app, lambda: order() == ["a.mov", "c.mov", "b.mov"], 10_000), order()
        assert dlg._selected_row() == 1, "the selection follows the export"
        up.click()
        assert wait_until(app, lambda: order() == ["c.mov", "a.mov", "b.mov"], 10_000), order()
        assert dlg._selected_row() == 0 and not up.isEnabled() and down.isEnabled()
        down.click()
        assert wait_until(app, lambda: order() == ["a.mov", "c.mov", "b.mov"], 10_000), order()
        # the engine's queue (what encodes first) has the same order
        listed = done_exec("export.queue.list", {})
        assert [Path(i["path"]).name for i in listed["items"]] == ["a.mov", "c.mov", "b.mov"]
        # two selected: no move
        dlg.table.clearSelection()
        dlg.table.setSelectionMode(QtWidgets.QAbstractItemView.SelectionMode.MultiSelection)
        dlg.table.selectRow(0)
        dlg.table.selectRow(1)
        assert not up.isEnabled() and not down.isEnabled()
    finally:
        if dlg is not None:
            dlg.close()
        win._quit_confirmed = True
        win.close()
        engine.stop()
