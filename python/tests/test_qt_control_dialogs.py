"""CFA4 Qt 6 GUI: the Project panel's inline rename and dialogs and the Media Browser's Edit
Columns as control-channel state (``docs/qt-parity.md`` 1.11), as egui's ``UiState``
``project_panel.rename`` / ``project_panel.dialog`` and ``media_browser.edit_columns``."""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
QtCore = pytest.importorskip("PySide6.QtCore")
QtWidgets = pytest.importorskip("PySide6.QtWidgets")

QtGui = pytest.importorskip("PySide6.QtGui")

from cfa4_qt.engine import EngineClient, locate_engine

REAL = locate_engine()


@pytest.fixture(scope="module")
def app():
    yield QtWidgets.QApplication.instance() or QtWidgets.QApplication([])


@pytest.fixture(autouse=True)
def no_modal_dialogs(monkeypatch):
    """The dialogs must not block: the control channel reads and fills them while they are open."""

    def refuse(self, *_args, **_kwargs):
        raise AssertionError(f"modal dialog opened: {type(self).__name__} {self.objectName()!r}")

    monkeypatch.setattr(QtWidgets.QDialog, "exec", refuse)
    monkeypatch.setattr(QtWidgets.QMessageBox, "exec", refuse)
    monkeypatch.setattr(QtWidgets.QInputDialog, "getText", refuse)


def wait_until(app, predicate, timeout_ms: int = 10_000) -> bool:
    timer = QtCore.QElapsedTimer()
    timer.start()
    while not predicate():
        if timer.elapsed() > timeout_ms:
            return False
        app.processEvents(QtCore.QEventLoop.ProcessEventsFlag.AllEvents, 20)
    return True


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_project_rename_and_dialogs_through_the_control_channel(app, tmp_path: Path):
    """``ui.set {projectPanel: {rename, dialog}, mediaBrowser: {editColumns}}`` as egui's: each
    opens the rename or dialog with the given draft, ``ui.inspect`` reads the draft back, null
    leaves it without a change, OK runs its command; a value the panel would not take is refused
    and opens nothing."""
    from cfa4_qt.control import ControlServer
    from cfa4_qt.main_window import MainWindow
    from cfa4_qt.panels.project import ITEM_ROLE, ManagePresetsDialog

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1600, 1000)
    win.show()
    engine.start()
    server = ControlServer(win)
    pp, mb = win.project_panel, win.media_browser

    def call(method: str, params: dict[str, Any]) -> tuple[bool, Any]:
        from cfa4_qt.control import ControlError

        out: list = []
        try:
            server.dispatch(method, params, lambda ok, v: out.append((ok, v)))
        except ControlError as e:  # what the channel answers with
            out.append((False, str(e)))
        assert out, method
        return out[0]

    def ui() -> dict[str, Any]:
        ok, r = call("ui.inspect", {})
        assert ok, r
        return r["ui"]

    def ok(params: dict[str, Any]) -> dict[str, Any]:
        good, r = call("ui.set", params)
        assert good, (params, r)
        return r["ui"]

    def refused(params: dict[str, Any], text: str) -> None:
        before = ui()
        good, r = call("ui.set", params)
        assert not good and text in str(r), (params, r)
        assert ui() == before, f"{params} changed something"

    def rows() -> dict[int, dict[str, Any]]:
        return {r["item"]: r for r in pp.main.data.get("rows") or [] if isinstance(r, dict) and isinstance(r.get("item"), int)}

    try:
        assert wait_until(app, lambda: win.sequence is not None and pp.main.data.get("rows"), 60_000)
        ok({"projectPanel": {"bin": 1}})  # Footage (the demo's root holds bins only)
        assert wait_until(app, lambda: len(rows()) > 1, 10_000)
        first = ui()
        assert first["projectPanel"]["rename"] is None and first["projectPanel"]["dialog"] is None
        assert first["mediaBrowser"]["editColumns"] is None
        item = next(i for i, r in rows().items() if not r.get("isBin") and r.get("name"))
        name = rows()[item]["name"]

        # rename: the inline editor on that item with the text typed so far; null leaves it as it was
        u = ok({"projectPanel": {"rename": {"item": item, "text": "Renamed by agent"}}})
        assert u["projectPanel"]["rename"] == {"item": item, "bin": None, "text": "Renamed by agent", "panel": "project"}
        found = pp.main.rename_editor()
        assert found is not None and found[0].isVisible() and found[0].text() == "Renamed by agent"
        u = ok({"projectPanel": {"rename": {"item": item, "text": "Typed more"}}})
        assert u["projectPanel"]["rename"]["text"] == "Typed more"
        assert ok({"projectPanel": {"rename": None}})["projectPanel"]["rename"] is None
        wait_until(app, lambda: False, 300)
        assert rows()[item]["name"] == name, "null renames nothing"
        refused({"projectPanel": {"rename": {"item": item, "bin": 1}}}, "item id or a bin id")
        refused({"projectPanel": {"rename": {"item": 987654}}}, "no item 987654")
        refused({"projectPanel": {"rename": {"item": item, "panel": "projectBin.7"}}}, "no panel")
        refused({"projectPanel": {"rename": {"item": item, "caret": 1}}}, "unknown field")
        # …and Enter renames, as typing it would
        ok({"projectPanel": {"rename": {"item": item, "text": "Sunset (agent)"}}})
        ed = pp.main.rename_editor()[0]
        for kind in (QtCore.QEvent.Type.KeyPress, QtCore.QEvent.Type.KeyRelease):
            QtWidgets.QApplication.sendEvent(ed, QtGui.QKeyEvent(kind, QtCore.Qt.Key.Key_Return, QtCore.Qt.KeyboardModifier.NoModifier))
        assert wait_until(app, lambda: rows().get(item, {}).get("name") == "Sunset (agent)", 10_000)
        assert ui()["projectPanel"]["rename"] is None

        # Metadata Display: the columns in order and the field filter
        u = ok({"projectPanel": {"dialog": {"metadataDisplay": {"columns": ["Name", "Label", "Media Duration"], "filter": "dur"}}}})
        assert u["projectPanel"]["dialog"] == {"metadataDisplay": {"columns": ["Name", "Label", "Media Duration"], "filter": "dur"}}
        dlg = pp.dialog
        assert dlg is not None and dlg.isVisible() and dlg.objectName() == "project.metadataDisplay"
        shown = [dlg.list.item(i).data(ITEM_ROLE) for i in range(dlg.list.count()) if not dlg.list.item(i).isHidden()]
        assert shown and all("dur" in str(s).lower() for s in shown), shown
        refused({"projectPanel": {"dialog": {"metadataDisplay": {"columns": ["Name", "Nonsense"]}}}}, "no field 'Nonsense'")
        dlg.accept()
        assert wait_until(app, lambda: [c.get("name") for c in (pp.view.get("view") or {}).get("columns") or []] == ["Name", "Label", "Media Duration"], 10_000)
        assert ui()["projectPanel"]["dialog"] is None

        # Save As New View Preset → Manage Saved View Presets (rename, delete)
        u = ok({"projectPanel": {"dialog": {"savePresetAs": {"name": "Agent view"}}}})
        assert u["projectPanel"]["dialog"] == {"savePresetAs": {"name": "Agent view"}}
        pp.dialog.accept()
        pp._load_presets()  # the panel menu loads them as it opens
        assert wait_until(app, lambda: any(p.get("name") == "Agent view" for p in pp.presets), 10_000)
        slot = next(p["slot"] for p in pp.presets if p.get("name") == "Agent view")
        u = ok({"projectPanel": {"dialog": {"managePresets": {"selected": slot - 1}}}})
        assert u["projectPanel"]["dialog"] == {"managePresets": {"selected": slot - 1, "name": "Agent view"}}
        manage = pp.dialog
        assert isinstance(manage, ManagePresetsDialog) and manage.list.count() == 10
        assert manage.list.item(slot - 1).text() == f"{slot}. Agent view"
        ok({"projectPanel": {"dialog": {"managePresets": {"name": "Renamed view"}}}})
        manage.buttons["rename"].click()
        assert wait_until(app, lambda: manage.list.item(slot - 1).text() == f"{slot}. Renamed view", 10_000)
        manage.buttons["delete"].click()
        assert wait_until(app, lambda: manage.list.item(slot - 1).text() == f"{slot}. (empty)", 10_000)
        assert not manage.buttons["rename"].isEnabled() and manage.buttons["overwrite"].isEnabled()
        refused({"projectPanel": {"dialog": {"managePresets": {"selected": 10}}}}, "slot from 0 to 9")
        # another kind replaces the open dialog; null closes it
        u = ok({"projectPanel": {"dialog": {"freeformOptions": {"options": {"grid": 40, "snap": True}}}}})
        opts = u["projectPanel"]["dialog"]["freeformOptions"]["options"]
        assert opts["grid"] == 40 and opts["snap"] is True and not manage.isVisible()
        pp.dialog.accept()
        assert wait_until(app, lambda: (pp.view.get("freeform") or {}).get("grid") == 40, 10_000)
        u = ok({"projectPanel": {"dialog": {"saveArrangement": {"name": "Layout A", "bin": None}}}})
        assert u["projectPanel"]["dialog"] == {"saveArrangement": {"name": "Layout A", "bin": None}}
        assert ok({"projectPanel": {"dialog": None}})["projectPanel"]["dialog"] is None
        refused({"projectPanel": {"dialog": {"freeformOptions": {"options": {"grid": 1}}}}}, "from 4 to 200")
        refused({"projectPanel": {"dialog": {"settings": {}}}}, "kind one of")
        refused({"projectPanel": {"dialog": {"savePresetAs": {"name": 3}}}}, "takes a string")

        # Media Browser ▸ Edit Columns…: the columns being chosen (Name stays, first)
        assert wait_until(app, lambda: bool(mb.data.get("columns")), 20_000)
        u = ok({"mediaBrowser": {"editColumns": ["Size", "Media Type"]}})
        assert u["mediaBrowser"]["editColumns"] == ["Name", "Size", "Media Type"]
        cols = mb.columns_dialog
        assert cols is not None and cols.isVisible()
        assert ok({"mediaBrowser": {"editColumns": ["Name", "Media Type"]}})["mediaBrowser"]["editColumns"] == ["Name", "Media Type"]
        assert mb.columns_dialog is cols, "the open dialog takes the new draft"
        refused({"mediaBrowser": {"editColumns": ["Name", "Colour"]}}, "no column 'Colour'")
        cols.accept()
        assert wait_until(app, lambda: [str(c) for c in mb.data.get("columns") or []] == ["Name", "Media Type"], 10_000)
        assert ui()["mediaBrowser"]["editColumns"] is None
        ok({"mediaBrowser": {"editColumns": ["Name"]}})
        assert ok({"mediaBrowser": {"editColumns": None}})["mediaBrowser"]["editColumns"] is None
        assert [str(c) for c in mb.data.get("columns") or []] == ["Name", "Media Type"], "null keeps the columns"
    finally:
        win._quit_confirmed = True
        win.close()
        engine.stop()
