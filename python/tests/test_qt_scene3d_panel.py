"""CFA4 Qt 6 GUI: the 3D asset panel (U1-A6) and the 3D export dialog (U3a-A8) against a stand-in
engine that answers the ``scene3d.*`` commands, so no engine build is needed."""

from __future__ import annotations

import os
from collections.abc import Iterator
from typing import Any

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
QtWidgets = pytest.importorskip("PySide6.QtWidgets")

from cfa4_qt.panels.scene3d_panel import ExportDialog, Scene3dPanel, report_lines  # noqa: E402


@pytest.fixture(scope="module")
def app() -> Iterator[Any]:
    yield QtWidgets.QApplication.instance() or QtWidgets.QApplication([])


GLB = {
    "id": 7,
    "name": "rock.glb",
    "path": "/a/rock.glb",
    "format": "gltf",
    "route": "A",
    "present": True,
    "stats": {"meshes": 2, "triangles": 120, "materials": 1, "cameras": 0, "animations": 0},
    "report": [
        {"severity": "approximated", "path": "materials[0]", "message": "sheen dropped to base color"},
        {"severity": "unsupported", "path": "extensions", "message": "KHR_draco_mesh_compression"},
    ],
    "dropped": 3,
}
USD = {"id": 8, "name": "set.usd", "path": "/a/set.usd", "format": "usd", "route": "A", "present": False, "report": []}


def cell(r: int, c: int, panel: Scene3dPanel) -> str:
    item = panel.table.item(r, c)
    assert item is not None
    return item.text()


class FakeEngine:
    def __init__(self, assets: list[dict[str, Any]], answers: dict[str, Any] | None = None) -> None:
        self.assets = assets
        self.answers = answers or {}
        self.sent: list[tuple[str, dict[str, Any]]] = []

    def execute(
        self, command: str, params: dict[str, Any] | None = None, on_reply: Any = None, on_error: Any = None
    ) -> int:
        params = params or {}
        self.sent.append((command, params))
        answer = self.answers.get(command)
        if callable(answer):
            answer = answer(params)
        if isinstance(answer, Exception):
            on_error(str(answer))
        elif command == "scene3d.list":
            on_reply({"assets": self.assets, "capabilities": {}}, None)
        else:
            on_reply(answer if answer is not None else {}, None)
        return 1

    def commands(self, name: str) -> list[dict[str, Any]]:
        return [p for c, p in self.sent if c == name]


def test_report_lines_put_unsupported_first() -> None:
    lines = report_lines(GLB["report"])
    assert [s for s, _ in lines] == ["unsupported", "approximated"]
    assert lines[0][1] == "extensions: KHR_draco_mesh_compression"
    assert report_lines("nonsense") == []


def test_panel_lists_assets_report_and_route_buttons(app: Any) -> None:
    panel = Scene3dPanel(None, FakeEngine([GLB, USD]))
    assert panel.objectName() == "scene3d.panel"
    assert panel.table.rowCount() == 2
    assert cell(0, 0, panel) == "rock.glb"
    assert panel.report.count() == 3  # two entries and the dropped counter
    assert "unsupported" not in panel.report.item(0).text() and "not carried" in panel.report.item(0).text()
    assert "120 triangles" in panel.summary.text()
    # route A is current: only B is offered; export is on
    assert not panel.route_a.isEnabled() and panel.route_b.isEnabled() and panel.export_button.isEnabled()
    panel.table.selectRow(1)  # USD: no native import, no export, missing file shown
    assert not panel.route_b.isEnabled() and not panel.export_button.isEnabled()
    assert cell(1, 3, panel) == "missing"
    assert panel.relink_button.isEnabled()


def test_panel_empty_project(app: Any) -> None:
    panel = Scene3dPanel(None, FakeEngine([]))
    assert panel.table.rowCount() == 0
    assert "No 3D assets" in panel.summary.text()
    assert not panel.export_button.isEnabled() and not panel.check_button.isEnabled()
    assert panel.import_button.isEnabled()


def test_panel_commands(app: Any) -> None:
    changed: list[int] = []
    eng = FakeEngine(
        [GLB],
        {
            "scene3d.import": {"id": 9, "name": "new.obj"},
            "scene3d.check": {"assets": [{"id": 7, "status": "changed", "message": "x"}], "problems": 1},
        },
    )
    panel = Scene3dPanel(None, eng, changed=lambda: changed.append(1))
    panel.import_file("/x/new.obj")
    assert eng.commands("scene3d.import") == [{"path": "/x/new.obj"}]
    assert "Imported new.obj" in panel.message.text() and changed
    panel.route_b.click()
    assert eng.commands("scene3d.route") == [{"id": 7, "route": "B"}]
    panel.check_button.click()
    assert cell(0, 3, panel) == "changed"
    assert "problem" in panel.message.text()
    panel.remove_button.click()
    assert eng.commands("scene3d.remove") == [{"id": 7}]


def test_relink_changed_file_asks_then_allows(app: Any) -> None:
    def relink(p: dict[str, Any]) -> Any:
        return {} if p.get("allowChanged") else RuntimeError("the file differs; pass `allowChanged` to read it again")

    eng = FakeEngine([GLB], {"scene3d.relink": relink})
    panel = Scene3dPanel(None, eng)
    panel.relink("/b/rock.glb")
    assert eng.commands("scene3d.relink") == [{"id": 7, "path": "/b/rock.glb"}]
    assert panel.confirm is not None  # asks before allowChanged
    panel.relink_changed(7, "/b/rock.glb")
    assert eng.commands("scene3d.relink")[-1]["allowChanged"] is True


LOSSY = {
    "written": False,
    "needsApproval": True,
    "lossless": False,
    "report": [{"severity": "unsupported", "path": "animations", "message": "STL has no animation"}],
}


def test_export_report_first_then_approve(app: Any) -> None:
    def export(p: dict[str, Any]) -> Any:
        return {**LOSSY, "written": True, "paths": ["/out/rock.stl"]} if p.get("approve") else LOSSY

    eng = FakeEngine([GLB], {"scene3d.export": export})
    d = ExportDialog(None, eng, [GLB, USD], 7, "/out")
    assert d.asset.count() == 1  # USD is not offered
    d.format.setCurrentIndex(d.format.findData("stl"))
    assert not d.embed.isEnabled()
    d.ok.click()
    first = eng.commands("scene3d.export")[0]
    assert first == {"id": 7, "dir": "/out", "format": "stl", "copyTextures": True, "overwrite": False}
    assert "approve" not in first
    assert d.report.count() == 1 and d.needs_approval and d.ok.text() == "Approve and write"
    assert "Nothing was written" in d.outcome.text()
    d.ok.click()
    assert eng.commands("scene3d.export")[1]["approve"] is True
    assert d.written == ["/out/rock.stl"] and "Written" in d.outcome.text()
    assert d.ok.text() == "Export"


def test_export_option_change_drops_the_approval(app: Any) -> None:
    eng = FakeEngine([GLB], {"scene3d.export": LOSSY})
    d = ExportDialog(None, eng, [GLB], 7, "/out")
    d.ok.click()
    assert d.needs_approval
    d.name.setText("other")
    assert not d.needs_approval and d.ok.text() == "Export" and d.report.count() == 0
    d.ok.click()
    assert "approve" not in eng.commands("scene3d.export")[-1]
    assert eng.commands("scene3d.export")[-1]["name"] == "other"


def test_export_lossless_and_errors(app: Any) -> None:
    eng = FakeEngine([GLB], {"scene3d.export": {"written": True, "paths": ["/out/rock.glb"], "report": []}})
    d = ExportDialog(None, eng, [GLB], 7, "")
    assert not d.ok.isEnabled()  # no folder yet
    d.dir.setText("/out")
    d.embed.setChecked(True)
    d.ok.click()
    sent = eng.commands("scene3d.export")[0]
    assert sent["embedTextures"] is True and sent["format"] == "glb"
    assert "without loss" in d.message.text()
    eng.answers["scene3d.export"] = RuntimeError("rock.glb already exists; pass overwrite")
    d.overwrite.setChecked(True)
    d.overwrite.setChecked(False)
    d.ok.click()
    assert "already exists" in d.message.text() and d.ok.isEnabled()
