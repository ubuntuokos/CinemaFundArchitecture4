"""CFA4 Qt 6 GUI: Sequence ▸ Sequence Profiles… and the profile editor (``panels/sequence_profiles.py``)
over the engine's ``sequenceProfiles.*`` commands."""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
QtCore = pytest.importorskip("PySide6.QtCore")
QtWidgets = pytest.importorskip("PySide6.QtWidgets")

from cfa4_qt.engine import EngineClient, locate_engine
from cfa4_qt.panels import sequence_profiles as sp

REAL = locate_engine()


@pytest.fixture(scope="module")
def app():
    yield QtWidgets.QApplication.instance() or QtWidgets.QApplication([])


@pytest.fixture(autouse=True)
def no_modal_dialogs(monkeypatch):
    def refuse(self, *_args, **_kwargs):
        raise AssertionError(f"modal dialog opened: {type(self).__name__} {self.objectName()!r}")

    monkeypatch.setattr(QtWidgets.QDialog, "exec", refuse)


BUILTIN = {
    "description": "HD 1080p 25 fps",
    "width": 1920,
    "height": 1080,
    "displayAspect": [16, 9],
    "pixelAspect": [1, 1],
    "fps": [25, 1],
    "builtin": True,
    "file": None,
}
USER = {
    "description": "My HDV",
    "width": 1440,
    "height": 1080,
    "displayAspect": [16, 9],
    "pixelAspect": [4, 3],
    "fps": [25, 1],
    "builtin": False,
    "file": "/x/my_hdv.json",
}


class Engine:
    """Answers ``sequenceProfiles.*`` from a list, as the engine does (the uniqueness check too)."""

    running = True

    def __init__(self, profiles: list[dict[str, Any]]):
        self.profiles = [dict(p) for p in profiles]
        self.sent: list[tuple[str, dict[str, Any]]] = []

    def listing(self) -> dict[str, Any]:
        return {"dir": "/x", "profiles": [dict(p) for p in self.profiles]}

    def execute(self, command, params=None, on_reply=None, on_error=None):
        params = dict(params or {})
        self.sent.append((command, params))
        reply: Any = None
        if command == "sequenceProfiles.list":
            reply = self.listing()
        elif command == "sequenceProfiles.save":
            p, original = params["profile"], params.get("original")
            key = sp.description_key(p["description"])
            if any(
                sp.description_key(q["description"]) == key
                and not (q["builtin"] is False and original and sp.description_key(original) == key)
                for q in self.profiles
            ):
                on_error and on_error(f"a sequence profile is already called “{p['description']}”")
                return 0
            self.profiles = [
                q
                for q in self.profiles
                if not (
                    original
                    and q["builtin"] is False
                    and sp.description_key(q["description"]) == sp.description_key(original)
                )
            ]
            saved = {**p, "builtin": False, "file": f"/x/{key}.json"}
            self.profiles.append(saved)
            reply = {"profile": saved, "profiles": self.listing()}
        elif command == "sequenceProfiles.delete":
            self.profiles = [q for q in self.profiles if q["description"] != params["description"]]
            reply = self.listing()
        elif command == "sequenceProfiles.apply":
            reply = {"profile": params["description"], "notApplied": ["interlaced"]}
        if on_reply:
            on_reply(reply, None)
        return 0


def test_aspects_and_checks_match_the_engine():
    assert sp.display_aspect_for(1440, 1080, (4, 3)) == (16, 9)
    assert sp.pixel_aspect_for(720, 576, (16, 9)) == (64, 45)
    assert sp.display_aspect_for(0, 1080, (1, 1)) is None
    d = sp.draft_from(USER)
    assert (d.width, d.par, d.dar, d.builtin) == (1440, (4, 3), (16, 9), False)
    assert sp.profile_params(sp.ProfileDraft(description="  A   b "))["description"] == "A b"
    damaged = sp.draft_from({"width": -5, "pixelAspect": "x", "fps": [0, 1], "builtin": "yes"})
    assert (damaged.width, damaged.par, damaged.fps, damaged.builtin) == (1920, (1, 1), (25, 1), False)
    profiles = [BUILTIN, USER]
    ok = sp.ProfileDraft(description="New one")
    assert sp.problem(ok, profiles, None) == ""
    assert sp.problem(sp.ProfileDraft(description="  "), profiles, None)
    assert sp.problem(sp.ProfileDraft(description="my  hdv"), profiles, None)
    assert (
        sp.problem(sp.ProfileDraft(description="My HDV", width=1440, par=(4, 3)), profiles, "My HDV") == ""
    )  # its own name
    assert sp.problem(
        sp.ProfileDraft(description="HD 1080p 25 fps"), profiles, "HD 1080p 25 fps"
    )  # a built-in keeps its name
    assert sp.problem(sp.ProfileDraft(description="A", dar=(4, 3)), profiles, None)
    assert sp.problem(sp.ProfileDraft(description="A", par=(9, 1), dar=(8, 1)), profiles, None)
    assert sp.problem(sp.ProfileDraft(description="A", fps=(1001, 1)), profiles, None)


def test_editor_recomputes_aspects_and_cancel_keeps_the_profile(app):
    engine = Engine([BUILTIN, USER])
    draft = sp.draft_from(USER)
    ed = sp.ProfileEditor(None, engine, draft, engine.profiles, "My HDV")
    try:
        ed.width_box.setValue(1920)  # the display aspect follows the size
        assert (ed.dar_num.value(), ed.dar_den.value()) == (64, 27)
        ed.par_num.setValue(1)
        ed.par_den.setValue(1)
        assert (ed.dar_num.value(), ed.dar_den.value()) == (16, 9)
        ed.dar_num.setValue(4)  # the pixels follow the display aspect
        ed.dar_den.setValue(3)
        assert ed.d.par == (3, 4) and ed.ok.isEnabled()
        ed.fps_num.setValue(30_000)
        ed.fps_den.setValue(1001)
        assert ed.fps_text.text().startswith("29.97")
        ed.description.setText("HD 1080p 25 fps")
        ed.description.textEdited.emit("HD 1080p 25 fps")
        assert not ed.ok.isEnabled() and "already" in ed.error.text()
        ed.reject()
        assert ed.d == draft and not [c for c, _ in engine.sent if c == "sequenceProfiles.save"]
    finally:
        ed.deleteLater()


def test_manager_new_edit_duplicate_delete_and_apply(app):
    engine = Engine([BUILTIN, USER])
    messages: list[str] = []
    changed: list[bool] = []
    dlg = sp.SequenceProfiles(None, engine, messages.append, lambda: changed.append(True), has_sequence=True)
    try:
        assert dlg.list.count() == 2 and dlg.list.item(1).text().endswith("(user)")
        dlg.list.setCurrentRow(0)
        assert not dlg.buttons["delete"].isEnabled()  # built-in
        dlg.new()
        ed = dlg.editor
        assert ed is not None and ed.d.description == "" and not ed.ok.isEnabled()
        ed.description.setText("Studio")
        ed.description.textEdited.emit("Studio")
        ed.apply()
        assert engine.sent[-1] == ("sequenceProfiles.save", {"profile": sp.profile_params(ed.d)})
        assert dlg.current_description() == "Studio" and "Studio" in messages[-1]
        # edit the user profile under a new description: sent with `original`
        dlg.edit()
        ed = dlg.editor
        ed.description.setText("Studio 2")
        ed.description.textEdited.emit("Studio 2")
        ed.apply()
        assert engine.sent[-1][1]["original"] == "Studio"
        assert [p["description"] for p in engine.profiles].count("Studio") == 0
        # a refused save keeps the editor open with the engine's message
        dlg.edit()
        ed = dlg.editor
        ed.profiles = []  # the dialog's own check passes; the engine refuses
        ed.description.setText("My HDV")
        ed.description.textEdited.emit("My HDV")
        ed.apply()
        assert "already called" in ed.error.text() and ed.ok.isEnabled()
        ed.reject()
        dlg.list.setCurrentRow(1)
        dlg.delete()
        assert engine.sent[-1] == ("sequenceProfiles.delete", {"description": "My HDV"})
        dlg.list.setCurrentRow(0)
        dlg.apply()
        assert engine.sent[-1] == ("sequenceProfiles.apply", {"description": "HD 1080p 25 fps"})
        assert changed and "not supported" in messages[-1]
    finally:
        dlg.deleteLater()


def test_manager_without_a_sequence_cannot_apply(app):
    dlg = sp.SequenceProfiles(None, Engine([BUILTIN]), lambda _t: None, lambda: None, has_sequence=False)
    try:
        assert not dlg.buttons["apply"].isEnabled()
    finally:
        dlg.deleteLater()


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_sequence_profiles(app, tmp_path: Path):
    """The dialog over the real engine: save a profile, find its file, apply it."""
    engine = EngineClient(REAL, ["--demo", "--autosave", "--data-dir", str(tmp_path / "data")])
    engine.start()
    try:
        dlg = sp.SequenceProfiles(None, engine, lambda _t: None, lambda: None, has_sequence=True)
        timer = QtCore.QElapsedTimer()
        timer.start()
        while dlg.list.count() == 0 and timer.elapsed() < 10_000:
            app.processEvents(QtCore.QEventLoop.ProcessEventsFlag.AllEvents, 20)
        assert dlg.list.count() > 10
        dlg.new()
        ed = dlg.editor
        ed.description.setText("Real one")
        ed.description.textEdited.emit("Real one")
        ed.apply()
        timer.restart()
        while dlg.current_description() != "Real one" and timer.elapsed() < 10_000:
            app.processEvents(QtCore.QEventLoop.ProcessEventsFlag.AllEvents, 20)
        assert (tmp_path / "data" / "sequence-profiles" / "real_one.json").is_file()
        dlg.deleteLater()
    finally:
        engine.stop()
