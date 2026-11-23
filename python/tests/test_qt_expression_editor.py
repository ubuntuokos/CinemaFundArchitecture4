"""CFX expression editor (CFA4-AX-INTERPRETER-001 X4, N3-6′): highlighting, function list, validation with the
place of an error, compile state, approval-gated Bake and automation ids. A fake engine answers the way
``expr.*`` does; the real engine is used too when a built one is found."""

from __future__ import annotations

import os
from collections.abc import Callable, Iterator
from typing import Any

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
QtCore = pytest.importorskip("PySide6.QtCore")
QtWidgets = pytest.importorskip("PySide6.QtWidgets")

from cfa4_qt.panels import expression_editor as ee

SRC = "reads @density;\nwrites @hot;\n@hot = @density * 2.0; // double\n"


@pytest.fixture(scope="module")
def app() -> Iterator[Any]:
    yield QtWidgets.QApplication.instance() or QtWidgets.QApplication([])


class FakeEngine:
    """Answers expr.* / metadata.get / vdb.inspect synchronously; remembers every call."""

    running = True

    def __init__(self) -> None:
        self.calls: list[tuple[str, dict[str, Any]]] = []
        self.saved: dict[int, dict[str, Any]] = {}
        self.approved: set[int] = set()
        self.check_answer: dict[str, Any] = {"ok": True, "reads": ["density"], "writes": ["hot"], "warnings": []}

    def execute(
        self,
        command: str,
        params: dict[str, Any] | None = None,
        on_reply: Callable[..., None] | None = None,
        on_error: Callable[[str], None] | None = None,
    ) -> int:
        p = params or {}
        self.calls.append((command, p))
        reply: Any = {}
        if command == "metadata.get":
            reply = {"volume": {"path": "/x/t.vdb"}}
        elif command == "vdb.inspect":
            reply = {
                "inspection": {
                    "grids": [
                        {"name": "density", "type": "float"},
                        {"name": "vel", "type": "Vec3f"},
                        {"name": "id", "type": "int32"},
                    ]
                }
            }
        elif command == "expr.check":
            reply = self.check_answer
        elif command == "expr.save":
            ident = p.get("id") or 7
            self.saved[ident] = {**p, "id": ident}
            self.approved.add(ident)
            reply = {"expression": {"id": ident, "approved": True}}
        elif command == "expr.list":
            reply = {
                "expressions": [
                    {"id": i, "name": e["name"], "approved": i in self.approved} for i, e in self.saved.items()
                ]
            }
        elif command == "expr.get":
            e = self.saved[p["id"]]
            reply = {"expression": {**e, "approved": e["id"] in self.approved}}
        elif command == "expr.approve":
            self.approved.add(p["id"])
        elif command in ("expr.run_preview", "expr.bake"):
            reply = {"report": {"voxels": 8000, "elapsedMs": 12}}
        if on_reply is not None:
            on_reply(reply, None)
        return 1

    def names(self) -> list[str]:
        return [c for c, _ in self.calls]


def make(app: Any, item: int | None = 3) -> tuple[Any, FakeEngine]:
    eng = FakeEngine()
    w = ee.ExpressionEditor(eng, lambda: item)  # type: ignore[arg-type]
    w.refresh()
    return w, eng


def test_outputs_and_grids_are_read_strictly() -> None:
    assert ee.parse_outputs("hot:float, tint : vec3") == {"hot": "float", "tint": "vec3"}
    assert ee.parse_outputs("") == {}
    for bad in ("hot", "hot:int", "1a:float", "a:float b:vec3", "a:"):
        assert ee.parse_outputs(bad) is None
    assert ee.outputs_text({"b": "vec3", "a": "float"}) == "a:float, b:vec3"
    grids = {
        "grids": [
            {"name": "d", "type": "float"},
            {"name": "v", "type": "vec3s"},
            {"name": "i", "type": "int32"},
            {"name": 5},
            "x",
            {"name": "d", "type": "vec3f"},
        ]
    }
    assert ee.grid_attrs(grids) == {"d": "float", "v": "vec3"}
    assert ee.grid_attrs(None) == {} and ee.grid_attrs({"grids": "no"}) == {}


def test_error_places_are_clipped_and_hostile_values_ignored() -> None:
    assert ee.error_range(SRC, {"line": 3, "col": 8, "len": 8}) == (SRC.index("@density *"), 8)
    assert ee.error_range("ab", {"line": 1, "col": 1, "len": 99}) == (0, 2)
    assert ee.error_range("a\nb", {"line": 1, "col": 1, "len": 5}) == (0, 1), "the mark stops at the end of its line"
    for bad in (
        {},
        {"line": 0, "col": 1, "len": 1},
        {"line": 9, "col": 1, "len": 1},
        {"line": "1", "col": 1, "len": 1},
        {"line": True, "col": 1, "len": 1},
    ):
        assert ee.error_range(SRC, bad) is None
    assert ee.diagnostic_text({"code": "E-ATTR-1", "line": 3, "col": 7, "messageEn": "bad"}).startswith("E-ATTR-1 3:7")


def test_highlighting_marks_every_kind_and_tracks_block_comments(app: Any) -> None:
    edit = QtWidgets.QPlainTextEdit()
    doc = edit.document()
    h = ee.ExpressionHighlighter(doc)
    spans, state = h.spans("reads @density; float t = sin(1.5) + P.x; // note", 0)
    kinds = {k for _, _, k in spans}
    assert {"keyword", "attribute", "type", "function", "number", "builtin", "comment"} <= kinds
    assert state == 0
    text = "a /* open"
    spans, state = h.spans(text, 0)
    assert state == 1 and spans[-1][2] == "comment"
    spans, state = h.spans("still */ x = 1;", 1)
    assert state == 0 and spans[0] == (0, 8, "comment")
    # a keyword inside a comment or a name that only contains one is not marked
    spans, _ = h.spans("// if for", 0)
    assert [k for _, _, k in spans] == ["comment"]
    spans, _ = h.spans("format = 1;", 0)
    assert "keyword" not in {k for _, _, k in spans} and "type" not in {k for _, _, k in spans}
    # whole-document highlighting does not raise
    doc.setPlainText(SRC + "/* x\ny */ 1")


def test_function_list_covers_the_language_and_inserts(app: Any) -> None:
    from cfa4_qt.panels.expression_editor import FUNCTIONS

    assert {"noise", "lerp", "fit", "normalize", "rand"} <= {n for n, _ in FUNCTIONS}
    w, _ = make(app)
    assert w.functions.count() == len(FUNCTIONS)
    w.functions.itemActivated.emit(w.functions.item(0))
    assert w.source.toPlainText().startswith("abs(")


def test_the_window_has_the_automation_ids(app: Any) -> None:
    w, _ = make(app)
    names = {c.objectName() for c in w.findChildren(QtCore.QObject) if c.objectName()}
    for ident in (
        "dialog.expression",
        "expr.source",
        "expr.status",
        "expr.saved",
        "expr.functions",
        "expr.name",
        "expr.domain",
        "expr.outputs",
    ):
        assert ident in names or w.objectName() == ident
    for ident in ("expr.new", "expr.save", "expr.delete", "expr.approve", "expr.preview", "expr.bake"):
        assert ident in names


def test_checking_shows_the_compile_state_or_the_place_of_the_error(app: Any) -> None:
    w, eng = make(app)
    assert w.attrs == {"density": "float", "vel": "vec3"}
    assert [w.domain.itemText(i) for i in range(w.domain.count())] == ["density", "vel"]
    w.outputs.setText("hot:float")
    w.source.setPlainText(SRC)
    w.check()
    cmd, params = [c for c in eng.calls if c[0] == "expr.check"][-1]
    assert (
        params["source"] == SRC
        and params["attrs"] == {"density": "float", "vel": "vec3"}
        and params["outputs"] == {"hot": "float"}
    )
    assert w.compiled and w.status.property("state") == "ok"
    assert "density" in w.status.text() and "hot" in w.status.text()
    assert w.source.extraSelections() == []
    eng.check_answer = {
        "ok": False,
        "error": {
            "code": "E-ATTR-1",
            "line": 3,
            "col": 8,
            "len": 8,
            "messageEn": "undeclared @density",
            "messageHu": "x",
        },
    }
    w.check()
    assert not w.compiled and w.status.property("state") == "error"
    assert w.status.text().startswith("E-ATTR-1 3:8")
    sel = w.source.extraSelections()
    assert len(sel) == 1 and sel[0].cursor.selectedText() == "@density"
    # an unreadable outputs field is reported without asking the engine
    n = len(eng.calls)
    w.outputs.setText("hot")
    w.check()
    assert len(eng.calls) == n and w.status.property("state") == "error" and not w.compiled
    # an empty text is idle, not an error
    w.outputs.setText("hot:float")
    w.source.setPlainText("")
    w.check()
    assert w.status.text() == "" and not w.compiled


def test_save_approve_preview_and_bake_follow_the_state(app: Any) -> None:
    w, eng = make(app)
    w.name.setText("Hot")
    w.domain.setEditText("density")
    w.outputs.setText("hot:float")
    w.source.setPlainText(SRC)
    w.check()
    assert w.save_button.isEnabled() and w.preview_button.isEnabled()
    assert not w.bake_button.isEnabled() and not w.delete_button.isEnabled(), "nothing saved yet"
    w.save()
    assert w.current_id == 7
    assert ("expr.save", {"name": "Hot", "source": SRC, "domain": "density", "outputs": {"hot": "float"}}) in eng.calls
    assert w.saved.count() == 1 and w.bake_button.isEnabled(), "the author's machine approved what it saved"
    # preview runs the text in the window with the selected volume and asks for a render
    w.run("expr.run_preview")
    cmd, params = eng.calls[-1]
    assert (
        cmd == "expr.run_preview"
        and params["item"] == 3
        and params["render"] is True
        and "name" not in params
        and params["source"] == SRC
    )
    assert "8,000 voxels" in w.message.text()
    # bake runs the saved expression by id
    w.run("expr.bake")
    assert eng.calls[-1] == ("expr.bake", {"id": 7, "item": 3})
    # a text from elsewhere is not approved: Bake stays off until Approve
    eng.approved.clear()
    w.new_expression()
    w.saved.setCurrentRow(0)
    assert w.current_id == 7 and not w.bake_button.isEnabled() and w.approve_button.isEnabled()
    assert "Not approved" in w.approval.text() or "Nincs" in w.approval.text()
    w.approve()
    assert 7 in eng.approved and w.bake_button.isEnabled() and not w.approve_button.isEnabled()
    # opening the window and checking never ran anything
    assert not any(c in ("expr.bake", "expr.run_preview") for c in eng.names()[: eng.names().index("expr.save")])


def test_preview_needs_a_selected_volume(app: Any) -> None:
    w, eng = make(app, item=None)
    w.name.setText("a")
    w.domain.setEditText("d")
    w.source.setPlainText(SRC)
    w.check()
    w.run("expr.run_preview")
    assert "expr.run_preview" not in eng.names() and w.message.text()


def test_screenshot_of_the_window(app: Any, tmp_path: Any) -> None:
    w, eng = make(app)
    w.name.setText("Hot")
    w.domain.setEditText("density")
    w.outputs.setText("hot:float")
    w.source.setPlainText(SRC)
    w.check()
    w.resize(900, 520)
    w.show()
    app.processEvents()
    path = tmp_path / "expr.png"
    assert w.grab().save(str(path)) and path.stat().st_size > 1000
    if os.environ.get("CFA4_EXPR_SHOT"):
        w.grab().save(os.environ["CFA4_EXPR_SHOT"])
