"""CFA4 Qt 6 GUI: the Project panel's inline rename and dialogs and the Media Browser's Edit
Columns as egui's ``ProjectPanelUi::{rename, dialog}`` and ``MediaBrowserUi::edit_columns``,
read and set through the control channel's ``ui.set`` (``docs/qt-parity.md`` 1.11)."""

from __future__ import annotations

import os
from typing import Any

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
QtCore = pytest.importorskip("PySide6.QtCore")
QtWidgets = pytest.importorskip("PySide6.QtWidgets")

PROJECT = {
    "name": "P",
    "root": {
        "id": 0,
        "children": [
            {"item": 1, "name": "clip", "type": "Video"},
            {"item": 2, "name": "other", "type": "Video"},
            {"id": 5, "name": "Bin A", "children": []},
        ],
    },
}


@pytest.fixture(scope="module")
def app():
    yield QtWidgets.QApplication.instance() or QtWidgets.QApplication([])


@pytest.fixture(autouse=True)
def no_modal_dialogs(monkeypatch):
    """The dialogs open without a nested event loop, or the control channel could not reach them."""

    def refuse(self, *_args, **_kwargs):
        raise AssertionError(f"modal dialog opened: {type(self).__name__} {self.objectName()!r}")

    monkeypatch.setattr(QtWidgets.QDialog, "exec", refuse)
    monkeypatch.setattr(QtWidgets.QMessageBox, "exec", refuse)


@pytest.fixture
def panel(app):
    from cfa4_qt.panels.project import ProjectPanel

    p = ProjectPanel(None)
    p.resize(600, 400)
    p.show()
    p.set_project(PROJECT)
    app.processEvents()
    commands: list[tuple[str, dict[str, Any]]] = []
    views: list[tuple[str, dict[str, Any]]] = []
    p.command.connect(lambda cid, params: commands.append((cid, params)))
    p.set_view_cmd = lambda cid, params, refresh=True: views.append((cid, params))  # type: ignore[method-assign]
    p.sent = commands  # type: ignore[attr-defined]
    p.views_sent = views  # type: ignore[attr-defined]
    yield p
    p.close_dialog()
    p.close()
    p.deleteLater()
    app.processEvents()


def test_inline_rename_is_read_typed_and_cancelled_as_egui(app, panel):
    assert panel.ui_state()["rename"] is None
    panel.set_ui_state({"rename": {"item": 1, "text": "renamed"}})
    assert panel.ui_state()["rename"] == {"item": 1, "bin": None, "text": "renamed", "panel": "project"}
    # typing more into the same rename keeps the editor
    panel.set_ui_state({"rename": {"item": 1, "text": "renamed again"}})
    assert panel.ui_state()["rename"]["text"] == "renamed again"
    # a bin, in the panel named
    panel.set_ui_state({"rename": {"bin": 5, "text": "Bin B", "panel": "project"}})
    assert panel.ui_state()["rename"] == {"item": None, "bin": 5, "text": "Bin B", "panel": "project"}
    # null cancels: nothing renamed
    panel.set_ui_state({"rename": None})
    app.processEvents()
    assert panel.ui_state()["rename"] is None
    assert not [c for c in panel.sent if c[0] in ("clip.rename", "project.renameBin")], panel.sent
    for bad, text in (
        ({"item": 1, "bin": 5}, "an item or a bin"),
        ({}, "an item or a bin"),
        ({"item": 99}, "not shown"),
        ({"item": 1, "panel": "projectBin.9"}, "no Project panel"),
        ({"item": 1, "text": 3}, "strings"),
        ({"item": 1, "cursor": 0}, "unknown field"),
    ):
        with pytest.raises((ValueError, TypeError), match=text):
            panel.set_ui_state({"rename": bad})
    assert panel.ui_state()["rename"] is None


def test_project_dialogs_open_read_and_set_as_egui(app, panel):
    panel.view = {"available": ["Name", "Label", "Media Duration", "Frame Rate"], "view": {"columns": [{"name": "Name"}, {"name": "Label"}]}}
    panel.presets = [{"slot": 1, "name": "Mine"}, {"slot": 2, "name": "Edit"}, {"slot": 3}]
    assert panel.ui_state()["dialog"] is None

    # Metadata Display: the columns in order (Name stays first) and the field filter
    panel.set_ui_state({"dialog": {"metadataDisplay": {}}})
    assert panel.ui_state()["dialog"] == {"metadataDisplay": {"columns": ["Name", "Label"], "filter": ""}}
    panel.set_ui_state({"dialog": {"metadataDisplay": {"columns": ["Frame Rate", "Label"], "filter": "rate"}}})
    dlg = panel.dialog[1]
    assert dlg.objectName() == "project.metadataDisplay" and dlg.isVisible()
    assert panel.ui_state()["dialog"] == {"metadataDisplay": {"columns": ["Name", "Frame Rate", "Label"], "filter": "rate"}}
    from cfa4_qt.panels.project import ITEM_ROLE

    shown = [str(dlg.list.item(i).data(ITEM_ROLE)) for i in range(dlg.list.count()) if not dlg.list.isRowHidden(i)]
    assert shown == ["Frame Rate"], shown
    with pytest.raises(ValueError, match="no field 'Mood'"):
        panel.set_ui_state({"dialog": {"metadataDisplay": {"columns": ["Mood"]}}})
    assert panel.dialog is not None, "a bad field leaves an already open dialog open"
    dlg.accept()
    app.processEvents()
    assert panel.views_sent[-1] == ("project.columns.set", {"columns": ["Name", "Frame Rate", "Label"]})
    assert panel.ui_state()["dialog"] is None

    # Save As New View Preset: the name typed; OK saves it
    panel.set_ui_state({"dialog": {"savePresetAs": {"name": "Rough cut"}}})
    assert panel.ui_state()["dialog"] == {"savePresetAs": {"name": "Rough cut"}}
    panel.dialog[1].accept()
    app.processEvents()
    assert panel.views_sent[-1] == ("project.viewPreset.saveAs", {"name": "Rough cut"})

    # Manage Saved View Presets: the selected slot (0-based) fills the name
    panel.set_ui_state({"dialog": {"managePresets": {"selected": 1}}})
    assert panel.ui_state()["dialog"] == {"managePresets": {"selected": 1, "name": "Edit"}}
    panel.set_ui_state({"dialog": {"managePresets": {"name": "Final"}}})
    assert panel.ui_state()["dialog"] == {"managePresets": {"selected": 1, "name": "Final"}}
    with pytest.raises(ValueError, match="slot 2 is empty"):
        panel.set_ui_state({"dialog": {"managePresets": {"selected": 2}}})

    # opening another dialog closes this one (one dialog at a time, as egui)
    panel.set_ui_state({"dialog": {"freeformOptions": {"options": {"grid": 40, "snap": True, "cardSize": 9999}}}})
    opts = panel.ui_state()["dialog"]["freeformOptions"]["options"]
    assert opts["grid"] == 40 and opts["snap"] is True and opts["cardSize"] == 480, opts
    with pytest.raises(ValueError, match="true or false"):
        panel.set_ui_state({"dialog": {"freeformOptions": {"options": {"snap": "yes"}}}})
    panel.dialog[1].accept()
    app.processEvents()
    assert panel.views_sent[-1][0] == "project.freeform.options" and panel.views_sent[-1][1]["grid"] == 40

    # Save Arrangement: the name and the bin it is saved for
    panel.set_ui_state({"dialog": {"saveArrangement": {"name": "Story", "bin": 5}}})
    assert panel.ui_state()["dialog"] == {"saveArrangement": {"name": "Story", "bin": 5}}
    panel.dialog[1].accept()
    app.processEvents()
    assert panel.sent[-1] == ("project.freeform.saveArrangement", {"name": "Story", "bin": 5})

    # null closes without running anything; a bad kind or field opens nothing
    panel.set_ui_state({"dialog": {"savePresetAs": {"name": "Unwanted"}}})
    sent = len(panel.views_sent)
    panel.set_ui_state({"dialog": None})
    app.processEvents()
    assert panel.ui_state()["dialog"] is None and len(panel.views_sent) == sent
    for bad, text in (({"findBin": {}}, "kinds"), ({"savePresetAs": {"title": "x"}}, "unknown field"), ({"savePresetAs": "x"}, "field: value")):
        with pytest.raises((ValueError, TypeError), match=text):
            panel.set_ui_state({"dialog": bad})
        assert panel.ui_state()["dialog"] is None, bad


def test_media_browser_edit_columns_as_egui(app):
    from cfa4_qt.panels.media_browser import MediaBrowserPanel

    mb = MediaBrowserPanel(None)
    mb.show()
    sent: list[dict[str, Any]] = []
    mb.settings_cmd = sent.append  # type: ignore[method-assign]
    mb.data = {"columns": ["Name", "Size"], "allColumns": ["Name", "Media Type", "Size", "Date Modified"]}
    try:
        assert mb.ui_state()["editColumns"] is None
        mb.set_ui_state({"editColumns": ["Date Modified", "Size"]})
        assert mb.columns_dialog is not None and mb.columns_dialog.isVisible()
        assert mb.ui_state()["editColumns"] == ["Name", "Date Modified", "Size"]
        with pytest.raises(ValueError, match="no column 'Mood'"):
            mb.set_ui_state({"editColumns": ["Mood"]})
        assert mb.ui_state()["editColumns"] == ["Name", "Date Modified", "Size"], "refused without a change"
        mb.columns_dialog.accept()
        app.processEvents()
        assert sent == [{"columns": ["Name", "Date Modified", "Size"]}] and mb.ui_state()["editColumns"] is None
        # null cancels
        mb.set_ui_state({"editColumns": ["Media Type"]})
        mb.set_ui_state({"editColumns": None})
        app.processEvents()
        assert mb.ui_state()["editColumns"] is None and len(sent) == 1
    finally:
        mb.close()
        mb.deleteLater()
        app.processEvents()
