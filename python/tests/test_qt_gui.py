"""CFA4 Qt 6 GUI: protocol framing, registry helpers, i18n, and the main window against a fake
engine (always) and the real Rust engine (when it is built)."""

from __future__ import annotations

import io
import json
import os
import stat
import struct
import sys
import textwrap
from pathlib import Path

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
QtCore = pytest.importorskip("PySide6.QtCore")
QtWidgets = pytest.importorskip("PySide6.QtWidgets")
QtGui = pytest.importorskip("PySide6.QtGui")
QtTest = pytest.importorskip("PySide6.QtTest")

from cfa4_qt.commands import Command, CommandRegistry, parse_params, parse_value, qt_shortcut
from cfa4_qt.engine import EngineClient, EngineError, FrameReader, locate_engine
from cfa4_qt.i18n import Translator, parse_catalog, system_language
from cfa4_qt.timecode import TICKS_PER_SECOND, format_timecode, frame_ticks

ROOT = Path(__file__).resolve().parents[2]

FAKE_ENGINE = textwrap.dedent(
    r'''
    #!PYTHON
    """Speaks the `cfa4-cli serve` protocol with a tiny in-memory sequence."""
    import json, sys
    if sys.argv[1:2] == ["help"]:
        print("USAGE\n  serve [--autosave]            headless engine server")
        sys.exit(0)
    TPS = 254_016_000_000
    state = {"revision": 1, "selection": [], "playhead": 0, "log": []}
    def seq():
        return {"id": 2, "name": "Seq", "duration": 4 * TPS, "playhead": state["playhead"],
                "settings": {"width": 64, "height": 36, "frame_rate": {"num": 25, "den": 1}, "sample_rate": 48000},
                "markers": [], "in": None, "out": None, "selection": state["selection"],
                "video": [{"id": 10, "name": "V1", "items": [{"clip": 100, "item": 1, "name": "A.mp4", "start": 0,
                           "end": 2 * TPS, "duration": 2 * TPS, "sourceIn": 0, "speed": 1.0, "effects": [], "label": "Iris"}]}],
                "audio": [{"id": 20, "name": "A1", "items": []}]}
    project = {"name": "Demo", "path": None, "root": {"id": 0, "name": "root", "children": [
        {"item": 1, "name": "A.mp4", "type": "Movie", "duration": 2 * TPS},
        {"item": 2, "name": "Seq", "type": "Sequence", "duration": 4 * TPS}]}, "activeSequence": 2}
    commands = [
        {"id": "file.save", "label": "Save", "menu": ["File"], "shortcut": "Cmd+S", "params": "{\"path\":str?}", "enabled": True},
        {"id": "edit.undo", "label": "Undo", "menu": ["Edit"], "shortcut": "Cmd+Z", "params": "{}", "enabled": False},
        {"id": "timeline.razor", "label": "Razor", "menu": [], "shortcut": None, "params": "{\"time\":ticks}", "enabled": True},
    ]
    out = sys.stdout.buffer
    def send(obj, binary=None):
        if binary is not None:
            obj["binary"] = len(binary)
        out.write(json.dumps(obj).encode() + b"\n")
        if binary is not None:
            out.write(binary)
        out.flush()
    for line in sys.stdin.buffer:
        req = json.loads(line)
        rid, m, p = req.get("id"), req["method"], req.get("params") or {}
        state["log"].append(m)
        if m == "hello":
            send({"id": rid, "ok": True, "result": {"protocol": 1, "engine": "fake engine", "recovery": []}})
        elif m == "engine.commands":
            send({"id": rid, "ok": True, "result": commands})
        elif m == "session.state":
            send({"id": rid, "ok": True, "result": {"dirty": state.get("dirty", False), "revision": state["revision"]}})
        elif m == "frame.render":
            send({"id": rid, "ok": True, "result": {"w": 4, "h": 2, "format": "rgba8", "offline": []}}, bytes(32))
        elif m == "audio.render":
            send({"id": rid, "ok": True, "result": {"channels": 2, "frames": 1, "sampleRate": 48000}}, bytes(8))
        elif m == "app.quit":
            send({"id": rid, "ok": True, "result": {"quit": True}})
            break
        elif m == "engine.execute":
            c, cp = p.get("command"), p.get("params") or {}
            if c == "project.inspect":
                send({"id": rid, "ok": True, "result": project})
            elif c == "sequence.inspect":
                send({"id": rid, "ok": True, "result": seq()})
            elif c == "history.list":
                send({"id": rid, "ok": True, "result": {"undo": ["Razor"], "redo": []}})
            elif c == "effects.list":
                send({"id": rid, "ok": True, "result": [{"id": "gaussian_blur", "name": "Gaussian Blur", "kind": "Video", "path": "Video Effects/Blur"}]})
            elif c == "timeline.select":
                state["selection"] = cp.get("clips") or []
                state["revision"] += 1
                send({"id": rid, "ok": True, "result": None})
                send({"event": "projectChanged", "revision": state["revision"]})
            elif c == "test.makeDirty":
                state["dirty"] = True
                send({"id": rid, "ok": True, "result": None})
            elif c in ("file.save", "file.openDemoProject"):
                state["dirty"] = False
                send({"id": rid, "ok": True, "result": None})
            elif c == "playhead.set":
                state["playhead"] = int(cp.get("time") or 0)
                send({"id": rid, "ok": True, "result": None})
            else:
                send({"id": rid, "ok": False, "error": f"unknown command `{c}`"})
        else:
            send({"id": rid, "ok": False, "error": "unknown method"})
    '''
).replace("#!PYTHON", "#!" + sys.executable)


@pytest.fixture(scope="module")
def app():
    instance = QtWidgets.QApplication.instance() or QtWidgets.QApplication([])
    yield instance


@pytest.fixture(autouse=True)
def no_modal_dialogs(monkeypatch):
    """A modal dialog in a test would wait for a click forever: fail fast and name it instead."""

    def refuse(self, *_args, **_kwargs):
        raise AssertionError(f"modal dialog opened: {type(self).__name__} {self.objectName()!r} {self.windowTitle()!r}")

    monkeypatch.setattr(QtWidgets.QDialog, "exec", refuse)
    monkeypatch.setattr(QtWidgets.QMessageBox, "exec", refuse)


@pytest.fixture
def fake_engine(tmp_path: Path) -> Path:
    path = tmp_path / "fake-cfa4-cli"
    path.write_text(FAKE_ENGINE.lstrip(), encoding="utf-8")
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


def wait_until(app, predicate, timeout_ms: int = 10_000) -> bool:
    timer = QtCore.QElapsedTimer()
    timer.start()
    while not predicate():
        if timer.elapsed() > timeout_ms:
            return False
        app.processEvents(QtCore.QEventLoop.ProcessEventsFlag.AllEvents, 20)
    return True


# ----- framing -----
def test_frame_reader_reads_json_and_binary_tails():
    stream = io.BytesIO(b'{"id":1,"ok":true}\n{"id":2,"ok":true,"binary":3}\nabc{"event":"toast"}\n')
    reader = FrameReader(stream)
    assert reader.read() == ({"id": 1, "ok": True}, None)
    assert reader.read() == ({"id": 2, "ok": True, "binary": 3}, b"abc")
    assert reader.read() == ({"event": "toast"}, None)
    assert reader.read() is None


@pytest.mark.parametrize(
    "data",
    [b"not json\n", b"[1]\n", b'{"binary":-1}\n', b'{"binary":"x"}\n', b'{"binary":true}\n', b'{"binary":10}\nabc'],
)
def test_frame_reader_rejects_bad_framing(data: bytes):
    with pytest.raises(EngineError):
        FrameReader(io.BytesIO(data)).read()


def _script(path: Path, body: str) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(f"#!{sys.executable}\n" + body, encoding="utf-8")
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


def test_locate_engine_skips_engines_without_serve(tmp_path: Path, monkeypatch, fake_engine: Path):
    """Regression: an older cfa4-cli on PATH (no `serve`) was picked and exited with 2."""
    from cfa4_qt import engine as engine_mod

    old = _script(tmp_path / "bin" / "cfa4-cli", "import sys\nprint('USAGE\\n  exec <id>')\n")
    monkeypatch.delenv("CFA4_ENGINE", raising=False)
    monkeypatch.setenv("PATH", str(old.parent))
    rejected: list[str] = []
    # explicit old engine: refused with the reason
    assert locate_engine(old, rejected) is None
    assert "serve" in rejected[0] and str(old) in rejected[0]
    # the checkout's build beats PATH; an old one on PATH alone is refused
    root = tmp_path / "checkout"
    (root / "python" / "cfa4_qt").mkdir(parents=True)
    monkeypatch.setattr(engine_mod, "__file__", str(root / "python" / "cfa4_qt" / "engine.py"))
    assert locate_engine() is None
    build = root / "target" / "release" / "cfa4-cli"
    build.parent.mkdir(parents=True)
    build.write_bytes(fake_engine.read_bytes())
    build.chmod(0o755)
    assert engine_mod.engine_candidates()[0] == build
    assert locate_engine() == build


def test_locate_engine_honours_explicit_and_env(tmp_path: Path, monkeypatch, fake_engine: Path):
    assert locate_engine(tmp_path / "missing") is None
    assert locate_engine(fake_engine) == fake_engine
    monkeypatch.setenv("CFA4_ENGINE", str(fake_engine))
    assert locate_engine() == fake_engine


# ----- registry helpers -----
@pytest.mark.parametrize(
    ("egui", "qt"),
    [
        ("Cmd+K", "Ctrl+K"),
        ("Cmd+Shift+ArrowLeft", "Ctrl+Shift+Left"),
        ("Alt+Delete", "Alt+Del"),
        ("Space", "Space"),
        ("Cmd++", "Ctrl++"),
        ("Num3", "3"),
        ("Cmd+Comma", "Ctrl+,"),
        ("", ""),
        (None, ""),
        ("Shift", ""),
    ],
)
def test_qt_shortcut(egui, qt):
    assert qt_shortcut(egui) == qt


def test_parse_params_reads_keys_and_optional_marks():
    specs = parse_params('{"time":ticks,"track":"V1"|id?,"moves":[{"clip":id,"time":t}],"opts":{"a":1,"b":2}?}')
    assert [(s.name, s.optional) for s in specs] == [("time", False), ("track", True), ("moves", False), ("opts", True)]
    assert parse_params("{}") == []
    assert parse_params("nonsense") == []
    assert parse_params(None) == []


def test_parse_value():
    assert parse_value("3.5") == 3.5
    assert parse_value("[1,2]") == [1, 2]
    assert parse_value("Selects") == "Selects"
    assert parse_value("  ") is None


def test_command_needs_params():
    razor = Command.from_json({"id": "timeline.razor", "label": "Razor", "params": '{"time":ticks,"clip":id?}'})
    undo = Command.from_json({"id": "edit.undo", "label": "Undo", "params": "{}"})
    assert razor is not None and razor.needs_params
    assert undo is not None and not undo.needs_params
    assert Command.from_json({"label": "no id"}) is None


def test_registry_builds_menus_and_shortcuts(app):
    win = QtWidgets.QMainWindow()
    fired = []
    reg = CommandRegistry(lambda c: fired.append(c.id))
    reg.load(
        [
            {"id": "file.save", "label": "Save", "menu": ["File"], "shortcut": "Cmd+S", "params": "{}", "enabled": True},
            {"id": "file.new", "label": "Project", "menu": ["File", "New"], "shortcut": None, "params": "{}", "enabled": True},
            {"id": "x.dup", "label": "Dup", "menu": [], "shortcut": "Cmd+S", "params": "{}", "enabled": True},
            {"id": "edit.undo", "label": "Undo", "menu": ["Edit"], "shortcut": "Cmd+Z", "params": "{}", "enabled": False},
            "garbage",
        ]
    )
    bound = reg.build_menus(win.menuBar(), win)
    titles = [a.text() for a in win.menuBar().actions()]
    assert titles == ["File", "Edit"]
    assert "file.save" in bound and "x.dup" not in bound  # first claim keeps Ctrl+S
    assert not reg.actions["edit.undo"].isEnabled()
    reg.update_enabled([{"id": "edit.undo", "enabled": True}])
    assert reg.actions["edit.undo"].isEnabled()
    reg.actions["file.save"].trigger()
    assert fired == ["file.save"]


# ----- i18n and time -----
def test_catalog_parsing_and_lookup(tmp_path: Path):
    plain, ctx = parse_catalog("# comment\n\tSave\tMentés\nnoun\tCut\tVágás\n\tbroken line\n\tLine\\nTwo\tSor\\nKettő\n")
    assert plain == {"Save": "Mentés", "Line\nTwo": "Sor\nKettő"}
    assert ctx == {("noun", "Cut"): "Vágás"}
    t = Translator("hu")
    assert t.tr("No such English text 123") == "No such English text 123"
    assert t.tr("Save") == "Mentés"
    assert t.fmt("Save changes to '{name}' before closing?", name="X").endswith("?")
    assert Translator("xx").language == "en"


def test_system_language(monkeypatch):
    for var in ("CFA4_LANG", "LC_ALL", "LC_MESSAGES", "LANG"):
        monkeypatch.delenv(var, raising=False)
    monkeypatch.setenv("LANG", "hu_HU.UTF-8")
    assert system_language() == "hu"
    monkeypatch.setenv("LANG", "pt_PT.UTF-8")
    assert system_language() == "pt-br"
    monkeypatch.setenv("CFA4_LANG", "zz")
    assert system_language() == "en"


def test_timecode():
    ft = frame_ticks({"frame_rate": {"num": 25, "den": 1}})
    assert ft == TICKS_PER_SECOND // 25
    assert format_timecode(TICKS_PER_SECOND * 3661 + ft * 7, ft) == "01:01:01:07"
    assert frame_ticks({"frame_rate": {"num": 0, "den": 0}}) == TICKS_PER_SECOND // 30
    assert frame_ticks(None) == TICKS_PER_SECOND // 30
    assert format_timecode(-5, 0) == "00:00:00:00"


# ----- window against the fake engine -----
def test_main_window_with_fake_engine(app, fake_engine: Path, tmp_path: Path):
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(fake_engine)
    win = MainWindow(engine, settings)
    engine.start()
    try:
        assert wait_until(app, lambda: win.timeline.canvas.sequence is not None)
        assert wait_until(app, lambda: "file.save" in win.registry.actions)
        menus = [a.text() for a in win.menuBar().actions()]
        assert "File" in menus and "Edit" in menus
        assert not win.registry.actions["edit.undo"].isEnabled()
        assert win.project_panel.tree.topLevelItemCount() == 2
        assert win.effects.tree.topLevelItemCount() == 1
        assert wait_until(app, lambda: win.monitor.view.image is not None)
        assert win.monitor.view.image.width() == 4
        # clicking a clip selects it in the engine; the projectChanged event redraws from it
        win.timeline.canvas.resize(800, 200)
        canvas = win.timeline.canvas
        row = canvas.rows[0]
        x = int(canvas.x_of(TICKS_PER_SECOND))
        y = row.top + row.height // 2
        from PySide6.QtTest import QTest

        QTest.mouseClick(canvas, QtCore.Qt.MouseButton.LeftButton, QtCore.Qt.KeyboardModifier.NoModifier, QtCore.QPoint(x, y))
        assert wait_until(app, lambda: (win.sequence or {}).get("selection") == [100])
        assert wait_until(app, lambda: not win.inspector.empty.isVisibleTo(win.inspector))
    finally:
        win._quit_confirmed = True
        win.close()
    assert not engine.running


def test_engine_stop_fails_pending_requests(app, fake_engine: Path):
    engine = EngineClient(fake_engine)
    engine.start()
    assert wait_until(app, lambda: bool(engine.hello))
    engine.stop()
    errors = []
    engine.call("hello", on_error=errors.append)
    assert errors and "not running" in errors[0]


def test_replacing_the_project_asks_about_unsaved_changes(app, fake_engine: Path, tmp_path: Path, monkeypatch):
    """Regression: File ▸ Open / New / Demo Project replaced a dirty project without asking.
    The engine's dirty flag is asked for after the edit still in flight, not the last refresh."""
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(fake_engine)
    win = MainWindow(engine, settings)
    engine.start()
    sent: list[str] = []
    execute = engine.execute

    def recording(command, params=None, on_reply=None, on_error=None):
        sent.append(command)
        return execute(command, params, on_reply=on_reply, on_error=on_error)

    monkeypatch.setattr(engine, "execute", recording)
    answers: list[QtWidgets.QMessageBox.StandardButton] = []
    asked: list[str] = []

    def answer(box):
        asked.append(box.text())
        return answers.pop(0)

    monkeypatch.setattr(QtWidgets.QMessageBox, "exec", answer)
    demo = Command("file.openDemoProject", "Demo Project", ["File", "New"], "", "{}", True)
    try:
        assert wait_until(app, lambda: bool(engine.hello))
        # clean: no question, the command runs
        win.run_command(demo)
        assert wait_until(app, lambda: "file.openDemoProject" in sent)
        assert not asked
        # dirty (the refresh has not seen it yet): Cancel keeps the project
        sent.clear()
        engine.execute("test.makeDirty", {})
        answers.append(QtWidgets.QMessageBox.StandardButton.Cancel)
        win.run_command(demo)
        assert wait_until(app, lambda: bool(asked))
        app.processEvents(QtCore.QEventLoop.ProcessEventsFlag.AllEvents, 50)
        assert "file.openDemoProject" not in sent
        assert "continuing" in asked[0]
        # Save: saves, then replaces
        answers.append(QtWidgets.QMessageBox.StandardButton.Save)
        win.project = {"name": "Demo", "path": str(tmp_path / "a.fcproj")}
        win.run_command(demo)
        assert wait_until(app, lambda: "file.openDemoProject" in sent)
        assert sent.index("file.save") < sent.index("file.openDemoProject")
        # an Export window left open names the old project's sequence: it is put away
        class Dialog:
            hidden = False

            def hide(self):
                Dialog.hidden = True

        win.export_dialog = Dialog()  # type: ignore[assignment]
        # Don't Save: replaces without saving
        sent.clear()
        engine.execute("test.makeDirty", {})
        answers.append(QtWidgets.QMessageBox.StandardButton.Discard)
        win.run_command(demo)
        assert wait_until(app, lambda: "file.openDemoProject" in sent)
        assert "file.save" not in sent
        assert Dialog.hidden
        win.export_dialog = None
        # Revert never offers Save (it would overwrite the version reverted to): Cancel keeps the edits
        revert = Command("file.revert", "Revert", ["File"], "", "{}", True)
        buttons: list[QtWidgets.QMessageBox.StandardButtons] = []
        monkeypatch.setattr(
            QtWidgets.QMessageBox, "exec", lambda box: (buttons.append(box.standardButtons()), answer(box))[1]
        )
        sent.clear()
        asked.clear()
        engine.execute("test.makeDirty", {})
        answers.append(QtWidgets.QMessageBox.StandardButton.Cancel)
        win.run_command(revert)
        assert wait_until(app, lambda: bool(asked))
        app.processEvents(QtCore.QEventLoop.ProcessEventsFlag.AllEvents, 50)
        assert "file.revert" not in sent and "revert" in asked[0]
        assert not buttons[0] & QtWidgets.QMessageBox.StandardButton.Save
        answers.append(QtWidgets.QMessageBox.StandardButton.Discard)
        win.run_command(revert)
        assert wait_until(app, lambda: "file.revert" in sent)
        assert "file.save" not in sent
    finally:
        win._quit_confirmed = True
        win.close()


def test_deferred_effect_edits_reach_the_engine_before_save_or_close(app, fake_engine: Path, tmp_path: Path, monkeypatch):
    """Regression: a value in Effect Controls' 120 ms send delay was not sent before Save or the
    unsaved-changes check, so closing right after an edit lost it."""
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(fake_engine)
    win = MainWindow(engine, settings)
    engine.start()
    sent: list[str] = []
    execute, call = engine.execute, engine.call

    def recording(command, params=None, on_reply=None, on_error=None):
        sent.append(command)
        return execute(command, params, on_reply=on_reply, on_error=on_error)

    def recording_call(method, params=None, on_reply=None, on_error=None):
        if method != "engine.execute":  # execute() goes through call()
            sent.append(method)
        return call(method, params, on_reply=on_reply, on_error=on_error)

    monkeypatch.setattr(engine, "execute", recording)
    monkeypatch.setattr(engine, "call", recording_call)
    scale = {"id": "scale", "label": "Scale", "kind": "float", "animatable": True, "animated": False,
             "value": 100.0, "valueAtPlayhead": 100.0, "keyframes": []}
    clip = {"clip": 100, "name": "A", "start": 0, "end": 9000, "playhead": 0,
            "effects": [{"index": 0, "effect": "motion", "name": "Motion", "enabled": True, "params": [scale]}]}
    try:
        assert wait_until(app, lambda: bool(engine.hello) and "file.save" in win.registry.commands)
        ec = win.effect_controls
        ec._show(clip)
        ec.set_param(ec.rows[(0, "scale")], 150.0)
        sent.clear()
        win.confirm_unsaved(lambda: None)
        assert sent[:2] == ["effects.setParam", "session.state"]
        ec.set_param(ec.rows[(0, "scale")], 160.0)
        sent.clear()
        win.project = {"name": "Demo", "path": str(tmp_path / "a.fcproj")}
        win.run_command(win.registry.commands["file.save"])
        assert sent[:2] == ["effects.setParam", "file.save"]
    finally:
        win._quit_confirmed = True
        win.close()


def test_save_as_asks_before_the_added_suffix_replaces_a_project(app, tmp_path: Path, monkeypatch):
    """Regression: Save As returning ``cut`` (All files filter) wrote over an existing
    ``cut.fcproj`` without asking: the dialog only checked the name without the suffix."""
    from cfa4_qt.dialogs import path_params

    (tmp_path / "cut.fcproj").write_text("{}", encoding="utf-8")
    cmd = Command("file.saveAs", "Save As…", ["File"], "", '{"path":"string"}', True)
    parent = QtWidgets.QWidget()
    monkeypatch.setattr(QtWidgets.QFileDialog, "getSaveFileName", lambda *_a, **_k: (str(tmp_path / "cut"), ""))
    answers = [QtWidgets.QMessageBox.StandardButton.Cancel, QtWidgets.QMessageBox.StandardButton.Yes]
    monkeypatch.setattr(QtWidgets.QMessageBox, "exec", lambda _box: answers.pop(0))
    assert path_params(cmd, parent, None) == {"__cancel__": True}
    assert path_params(cmd, parent, None) == {"path": str(tmp_path / "cut.fcproj")}
    # a new name is not asked about
    monkeypatch.setattr(QtWidgets.QFileDialog, "getSaveFileName", lambda *_a, **_k: (str(tmp_path / "new"), ""))
    assert path_params(cmd, parent, None) == {"path": str(tmp_path / "new.fcproj")}
    assert not answers


def test_recovery_prompt_reads_the_engines_camel_case_keys():
    """Regression: the prompt read snake_case keys, so every candidate was 'Untitled' from 1970."""
    from cfa4_qt.main_window import recovery_text

    text = recovery_text({"id": "s1", "savedUnix": 1_790_000_000, "projectName": "Interjú", "cleanExit": True})
    assert "Interjú" in text and "1970" not in text and "was closed" in text
    crashed = recovery_text({"savedUnix": 1_790_000_000, "projectName": "B", "cleanExit": False})
    assert "quit unexpectedly" in crashed
    assert "unknown time" in recovery_text({"savedUnix": "junk"})
    assert "unknown time" in recovery_text({"savedUnix": 10**30})


def _focus_panel(app, win: QtWidgets.QMainWindow, name: str | None) -> None:
    target = win.findChild(QtWidgets.QLineEdit, f"focus.{name}") if name else win.findChild(QtWidgets.QLineEdit, "focus.none")
    target.setFocus()
    wait_until(app, lambda: QtWidgets.QApplication.focusWidget() is target, 2_000)
    assert QtWidgets.QApplication.focusWidget() is target


def test_panel_only_shortcuts_follow_the_focused_panel(app):
    """Regression: History ▸ Left (Undo) was bound window-wide, so Left undid instead of stepping;
    then, with panel keys dropped, Delete in the Timeline ran Project ▸ Clear instead of clearing clips."""
    ran: list[str] = []
    win = QtWidgets.QMainWindow()
    central = QtWidgets.QWidget()
    lay = QtWidgets.QVBoxLayout(central)
    for name in ("Timeline", "Project", "History", None):
        panel = QtWidgets.QWidget()
        panel.setObjectName(f"panel.{name}" if name else "elsewhere")
        field = QtWidgets.QLineEdit(panel)
        field.setObjectName(f"focus.{name}" if name else "focus.none")
        lay.addWidget(panel)
    win.setCentralWidget(central)
    reg = CommandRegistry(lambda c: ran.append(c.id))
    reg.load(
        [
            {"id": "edit.undo", "label": "Undo", "menu": ["Edit"], "shortcut": "Cmd+Z", "shortcuts": ["Cmd+Z", "Left"],
             "panelShortcuts": [{"keys": "Left", "panel": "History"}], "params": "{}", "enabled": True},
            {"id": "playhead.stepBack", "label": "Step Back", "menu": [], "shortcut": "Left", "shortcuts": ["Left"],
             "params": "{}", "enabled": True},
            {"id": "project.delete", "label": "Clear", "menu": ["Edit"], "shortcut": "Delete",
             "shortcuts": ["Delete", "Backspace"], "panelShortcuts": [{"keys": "Backspace", "panel": "Project"}],
             "params": "{}", "enabled": True},
            {"id": "edit.clear", "label": "Clear", "menu": ["Edit"], "shortcut": "Delete", "shortcuts": ["Delete"],
             "panelShortcuts": [{"keys": "Delete", "panel": "Timeline"}], "params": "{}", "enabled": True},
        ]
    )
    assert reg.commands["edit.undo"].shortcuts == ["Cmd+Z"]
    assert reg.commands["edit.undo"].panel_shortcuts == [("Left", "History")]
    reg.build_menus(win.menuBar(), win)
    reg.build_menus(win.menuBar(), win)  # a rebuild replaces the dispatchers instead of doubling them
    win.show()
    win.activateWindow()
    left = QtGui.QKeySequence("Left")
    assert left not in reg.actions["edit.undo"].shortcuts()
    assert len(win.findChildren(QtGui.QAction, "shortcut.Left")) == 1
    # panel-scoped keys count as taken, so the window's own shortcuts leave them alone
    assert {"Left", "Del", "Backspace", "Ctrl+Z"} <= reg.claimed_keys

    def press(key: str, panel: str | None) -> str | None:
        _focus_panel(app, win, panel)
        ran.clear()
        win.findChild(QtGui.QAction, f"shortcut.{key}").trigger()
        return ran[0] if ran else None

    assert press("Del", "Timeline") == "edit.clear"
    assert press("Del", "Project") == "project.delete"
    assert press("Del", None) == "project.delete"
    assert press("Left", "History") == "edit.undo"
    assert press("Left", "Timeline") == "playhead.stepBack"
    reg.update_enabled([{"id": "edit.clear", "enabled": False}])
    assert press("Del", "Timeline") is None  # a disabled command does not fall through to another
    win.close()


def test_export_asks_before_replacing_and_reports_folder_errors(app, tmp_path: Path, monkeypatch):
    """Regressions: Export silently truncated an existing output; an output folder that could not
    be created raised out of the slot and nothing was shown."""
    from cfa4_qt.panels.export import ExportDialog

    class Engine:
        """Answers export.resolve as the engine does: an extension-less path gets ``.mp4``; ``.png``
        is an image sequence."""

        running = False  # the queue poll timer outlives this test: it must not raise later

        def __init__(self):
            self.sent = []

        def execute(self, command, params=None, on_reply=None, on_error=None):
            if command == "export.resolve":
                path = str((params or {}).get("path"))
                if on_reply is not None:
                    fmt = "png" if path.endswith(".png") else "h264"
                    sidecar = (params or {}).get("captionSidecar")
                    on_reply({"settings": {"path": path if Path(path).suffix else path + ".mp4", "format": fmt,
                                           "captionSidecar": sidecar}}, None)
                return 0
            self.sent.append((command, dict(params or {})))
            return 0

    engine = Engine()
    dlg = ExportDialog(engine)  # type: ignore[arg-type]
    out = tmp_path / "out.mp4"
    out.write_bytes(b"previous export")
    monkeypatch.setattr(dlg, "params", lambda: {"path": str(out)})
    answers = [QtWidgets.QMessageBox.StandardButton.Cancel, QtWidgets.QMessageBox.StandardButton.Yes]
    monkeypatch.setattr(QtWidgets.QMessageBox, "exec", lambda _box: answers.pop(0))
    dlg._enqueue(True)
    assert engine.sent == [] and out.read_bytes() == b"previous export"
    dlg._enqueue(True)
    assert [c for c, _ in engine.sent] == ["export.queue.add"]
    dlg._enqueue(False)  # already confirmed for this path: no second question
    assert len(engine.sent) == 2 and not answers
    # typed without its extension: the file the engine will write is the one asked about
    answers.append(QtWidgets.QMessageBox.StandardButton.Cancel)
    monkeypatch.setattr(dlg, "params", lambda: {"path": str(tmp_path / "out")})
    dlg._replace_confirmed = set()
    dlg._enqueue(True)
    assert len(engine.sent) == 2 and not answers
    # an image sequence writes numbered frames: an earlier export's frames are asked about
    from cfa4_qt.panels.export import existing_outputs

    seq_dir = tmp_path / "frames"
    seq_dir.mkdir()
    (seq_dir / "cut000.png").write_bytes(b"x")
    (seq_dir / "cut_notes.png").write_bytes(b"x")
    assert existing_outputs(seq_dir / "cut.png", "png") == [seq_dir / "cut000.png"]
    assert existing_outputs(seq_dir / "cut.png", "h264") == []
    assert existing_outputs(seq_dir / "other.png", "png") == []
    # MXF OP-Atom writes each audio channel to stem_A1.mxf…: those count even without stem.mxf
    (seq_dir / "cut_A1.mxf").write_bytes(b"x")
    (seq_dir / "cut_A1.mxf.bak").write_bytes(b"x")
    assert existing_outputs(seq_dir / "cut.mxf", "mxf-opatom") == [seq_dir / "cut_A1.mxf"]
    assert existing_outputs(seq_dir / "cut.mxf", "mxf-op1a") == []
    # a folder that can be written but not listed is not "no frames there": the export stops
    def unlistable(_self):
        raise PermissionError("listing denied")

    with monkeypatch.context() as m:
        m.setattr(Path, "iterdir", unlistable)
        with pytest.raises(OSError):
            existing_outputs(seq_dir / "cut.png", "png")
        m.setattr(dlg, "params", lambda: {"path": str(seq_dir / "cut.png")})
        dlg._enqueue(True)
        assert len(engine.sent) == 2 and not answers
    # the engine keeps the sidecar kind's case (cut.SRT) except for WebVTT
    from cfa4_qt.panels.export import sidecar_output

    assert sidecar_output("/x/cut.mp4", "SRT") == Path("/x/cut.SRT")
    assert sidecar_output("/x/cut.mp4", "WebVTT") == Path("/x/cut.vtt")
    assert sidecar_output("/x/cut.mp4", "none") is None
    # Browse returning cut.png confirms that file only, not the earlier frames
    monkeypatch.setattr(QtWidgets.QFileDialog, "getSaveFileName", lambda *_a, **_k: (str(seq_dir / "cut.png"), ""))
    dlg._browse()
    monkeypatch.setattr(dlg, "params", lambda: {"path": str(seq_dir / "cut.png")})
    answers.append(QtWidgets.QMessageBox.StandardButton.Cancel)
    dlg._enqueue(True)
    assert len(engine.sent) == 2 and not answers
    answers.append(QtWidgets.QMessageBox.StandardButton.Yes)
    dlg._enqueue(True)
    assert len(engine.sent) == 3 and not answers
    dlg._enqueue(True)  # the frames are confirmed now
    assert len(engine.sent) == 4
    # Browse returning out.mov for an .mp4 format confirmed out.mov, not the existing out.mp4
    dlg.format.addItem("H.264", "h264")
    dlg.format.setCurrentIndex(dlg.format.count() - 1)
    dlg.formats["h264"] = {"extension": "mp4"}
    monkeypatch.setattr(QtWidgets.QFileDialog, "getSaveFileName", lambda *_a, **_k: (str(tmp_path / "out.mov"), ""))
    dlg._browse()
    assert dlg.path.text() == str(out)
    monkeypatch.setattr(dlg, "params", lambda: {"path": str(out)})
    answers.append(QtWidgets.QMessageBox.StandardButton.Cancel)
    dlg._enqueue(True)
    assert len(engine.sent) == 4 and not answers
    # a caption sidecar the export writes next to the video is asked about too
    (tmp_path / "talk.srt").write_text("1\n")
    monkeypatch.setattr(dlg, "params", lambda: {"path": str(tmp_path / "talk.mp4"), "captionSidecar": "srt"})
    answers.append(QtWidgets.QMessageBox.StandardButton.Cancel)
    dlg._enqueue(True)
    assert len(engine.sent) == 4 and not answers
    # a regular file where the folder should be: an error in the dialog, nothing queued
    blocker = tmp_path / "file"
    blocker.write_text("x")
    monkeypatch.setattr(dlg, "params", lambda: {"path": str(blocker / "sub" / "out.mp4")})
    dlg._enqueue(True)
    assert len(engine.sent) == 4
    assert "output folder" in dlg.summary.text()
    dlg.close()


def test_effect_controls_pending_edits_keep_their_clip_and_time(app):
    """Regressions: a coalesced edit was sent to whichever clip was shown when the 120 ms timer
    fired, and an animated one at the engine's playhead then rather than where it was made."""
    from cfa4_qt.panels.effect_controls import EffectControlsPanel

    def snapshot(clip: int, playhead: int, animated: bool) -> dict:
        scale = {"id": "scale", "label": "Scale", "kind": "float", "animatable": True, "animated": animated,
                 "value": 100.0, "valueAtPlayhead": 100.0, "keyframes": []}
        return {"clip": clip, "name": f"C{clip}", "start": 1000, "end": 9000, "playhead": playhead,
                "effects": [{"index": 0, "effect": "motion", "name": "Motion", "enabled": True, "params": [scale]}]}

    panel = EffectControlsPanel()
    sent: list[tuple[str, dict]] = []
    panel.command.connect(lambda c, p: sent.append((c, p)))
    panel._show(snapshot(1, 2000, False))
    panel.set_param(panel.rows[(0, "scale")], 150.0)
    panel._show(snapshot(2, 2000, False))  # another clip selected before the timer fired
    assert [(c, p["clip"], p["value"], "time" in p) for c, p in sent] == [("effects.setParam", 1, 150.0, False)]
    sent.clear()
    panel._show(snapshot(2, 3000, True))
    panel.set_param(panel.rows[(0, "scale")], 120.0)
    panel._show(snapshot(2, 7000, True))  # the playhead moved to another keyframe meanwhile
    assert [(p["clip"], p["time"]) for _c, p in sent] == [(2, 3000)]
    sent.clear()
    panel._show(snapshot(2, 20_000, True))  # past the clip: pinned to it, as the engine pins
    panel.set_param(panel.rows[(0, "scale")], 110.0)
    panel.send("effects.reset", {"clip": 2, "index": 0})  # pending values go before another edit
    assert [c for c, _p in sent] == ["effects.setParam", "effects.reset"] and sent[0][1]["time"] == 8999
    panel.close()


def test_effect_controls_key_animated_edits_at_the_playing_frame(app):
    """During Program playback neither the engine playhead nor the panel's snapshot moves: an
    edit is keyed at the frame the monitor shows."""
    from cfa4_qt.panels.effect_controls import EffectControlsPanel

    class Monitor:
        playing = True
        playhead = 6000

    host = QtWidgets.QMainWindow()
    host.monitor = Monitor()  # type: ignore[attr-defined]
    panel = EffectControlsPanel(host)
    host.setCentralWidget(panel)
    sent: list[dict] = []
    panel.command.connect(lambda _c, p: sent.append(p))
    scale = {"id": "scale", "label": "Scale", "kind": "float", "animatable": True, "animated": True,
             "value": 100.0, "valueAtPlayhead": 100.0, "keyframes": []}
    panel._show({"clip": 3, "name": "C", "start": 1000, "end": 9000, "playhead": 2000,
                 "effects": [{"index": 0, "effect": "motion", "name": "Motion", "enabled": True, "params": [scale]}]})
    panel.set_param(panel.rows[(0, "scale")], 130.0, immediate=True)
    assert sent[-1]["time"] == 6000
    host.monitor.playing = False  # type: ignore[attr-defined]
    panel.set_param(panel.rows[(0, "scale")], 140.0, immediate=True)
    assert sent[-1]["time"] == 2000
    host.close()


def test_effect_controls_toggle_keyframes_at_the_playing_frame(app):
    """Add/Remove Keyframe during playback acted on playback's start: it deleted the keyframe
    there, or added one at the stale engine playhead. It stops on the shown frame and acts there."""
    from cfa4_qt.panels.effect_controls import EffectControlsPanel

    class Monitor:
        playing = True
        playhead = 6010

        def stop(self):
            self.playing = False
            self.playhead = 6000

        def frame(self):
            return 100

    host = QtWidgets.QMainWindow()
    host.monitor = Monitor()  # type: ignore[attr-defined]
    panel = EffectControlsPanel(host)
    host.setCentralWidget(panel)
    sent: list[tuple[str, dict]] = []
    panel.command.connect(lambda c, p: sent.append((c, p)))

    def show(keyframes):
        scale = {"id": "scale", "label": "Scale", "kind": "float", "animatable": True, "animated": True,
                 "value": 100.0, "valueAtPlayhead": 100.0, "keyframes": keyframes}
        panel._show({"clip": 3, "name": "C", "start": 1000, "end": 9000, "playhead": 2000, "mediaTimeAtPlayhead": 1000,
                     "effects": [{"index": 0, "effect": "motion", "name": "Motion", "enabled": True, "params": [scale]}]})

    show([{"time": 2000, "mediaTime": 1000, "value": 1.0}])
    panel.rows[(0, "scale")]._add_remove()
    assert sent[-1] == ("effects.addKeyframe", {"clip": 3, "effect": 0, "param": "scale", "time": 6000})
    host.monitor.playing = True  # type: ignore[attr-defined]
    show([{"time": 2000, "mediaTime": 1000, "value": 1.0}, {"time": 6000, "mediaTime": 5000, "value": 2.0}])
    panel.rows[(0, "scale")]._add_remove()
    assert sent[-1] == ("effects.deleteKeyframe", {"clip": 3, "effect": 0, "param": "scale", "mediaTime": 5000})
    # the stopwatch stops playback too, so the engine freezes the value of the shown frame
    host.monitor.playing = True  # type: ignore[attr-defined]
    panel.rows[(0, "scale")]._toggle_animation()
    assert host.monitor.playing is False  # type: ignore[attr-defined]
    assert sent[-1] == ("effects.toggleAnimation", {"clip": 3, "effect": 0, "param": "scale"})
    # so do the parameter and effect Reset buttons, which key the default at the engine playhead
    for name in ("effectControls.0.scale.reset", "effectControls.0.reset"):
        host.monitor.playing = True  # type: ignore[attr-defined]
        button = panel.findChild(QtWidgets.QToolButton, name)
        assert button is not None, name
        button.click()
        assert host.monitor.playing is False  # type: ignore[attr-defined]
    assert sent[-1] == ("effects.reset", {"clip": 3, "index": 0})
    host.close()


# ----- the real engine, when built -----
REAL = locate_engine()


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_demo_project(app, tmp_path: Path):
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1280, 800)
    win.show()
    engine.start()
    try:
        assert wait_until(app, lambda: win.timeline.canvas.sequence is not None, 60_000)
        assert wait_until(app, lambda: len(win.registry.commands) > 100, 30_000)
        assert wait_until(app, lambda: win.monitor.view.image is not None, 60_000)
        seq = win.sequence or {}
        clips = [c for t in seq.get("video") or [] for c in t.get("items") or []]
        assert clips, json.dumps(seq)[:400]
        # an edit through the engine shows up in the panels
        before = sum(len(t.get("items") or []) for t in seq.get("video") or [])
        first = clips[0]
        cut = (int(first["start"]) + int(first["end"])) // 2
        win.execute("timeline.razor", {"time": cut, "clip": first["clip"]})
        assert wait_until(
            app, lambda: sum(len(t.get("items") or []) for t in (win.sequence or {}).get("video") or []) == before + 1, 30_000
        )
        assert wait_until(app, lambda: "Razor" in " ".join(win.history.list.item(i).text() for i in range(win.history.list.count())), 10_000)

        # History: Open first, the current step selected; a click undoes or redoes to a step
        def videos() -> int:
            return sum(len(t.get("items") or []) for t in (win.sequence or {}).get("video") or [])

        lst = win.history.list
        assert lst.item(0).text() == "Open"
        steps = lst.count() - 1
        assert steps >= 1 and lst.item(steps).isSelected(), [lst.item(i).text() for i in range(lst.count())]
        lst.itemClicked.emit(lst.item(0))
        assert wait_until(app, lambda: videos() == before and lst.count() == steps + 1 and lst.item(0).isSelected(), 30_000)
        lst.itemClicked.emit(lst.item(lst.count() - 1))
        assert wait_until(app, lambda: videos() == before + 1 and lst.item(lst.count() - 1).isSelected(), 30_000)
    finally:
        win._quit_confirmed = True
        win.close()


def _qt_literals() -> set[str]:
    """Every literal passed to tr()/fmt() and every UI label tuple string in the Qt GUI."""
    import re

    found: set[str] = set()
    for path in (ROOT / "python" / "cfa4_qt").rglob("*.py"):
        src = path.read_text(encoding="utf-8")
        found.update(m.group(1) for m in re.finditer(r'\b(?:tr|fmt|N_)\(\s*"((?:[^"\\]|\\.)*)"', src))
        found.update(m.group(1) for m in re.finditer(r"\b(?:tr|fmt|N_)\(\s*'((?:[^'\\]|\\.)*)'", src))
    return found


def test_hungarian_catalog_covers_the_qt_gui():
    """The owner works in Hungarian: every Qt GUI string and panel title has a translation."""
    hu = Translator("hu")
    titles = {"Source", "Go to Start", "Go to End", "Step Back 1 Frame", "Step Forward 1 Frame", "Mark In", "Mark Out", "Insert", "Overwrite", "Play/Stop", "Project", "Effects", "Properties", "History", "Timeline", "Events", "Selection Tool", "Razor Tool", "New Bin", "New Sequence…", "Import…"}
    missing = sorted(s for s in _qt_literals() | titles if not hu.has(s))
    assert not missing, missing


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_hungarian_catalog_covers_every_engine_menu():
    import subprocess

    out = subprocess.run([str(REAL), "commands", "--json", "--compact"], capture_output=True, check=True, timeout=120)
    hu = Translator("hu")
    missing: set[str] = set()
    for cmd in json.loads(out.stdout):
        menu = cmd.get("menu") or []
        for text in [*menu, cmd["label"]] if menu else []:
            if hu.tr(text) == text and not text.isupper() and not text.endswith(("XML…", "EDL…", "AAF…", "OMF…", "OpenTimelineIO…", "Avid Log Exchange…", "FCPXML…")):
                missing.add(text)
    missing -= {"Proxy", "Remix"}  # same word in Hungarian
    assert not missing, sorted(missing)


def test_engine_that_dies_does_not_block_the_gui(app, tmp_path: Path):
    """Regression: the "engine stopped" message was a blocking QMessageBox.critical, whose nested
    event loop froze the GUI (and hung unattended runs) when the engine exited."""
    from cfa4_qt.main_window import MainWindow

    dying = tmp_path / "dying-cfa4-cli"
    dying.write_text(f"#!{sys.executable}\nimport sys\nsys.exit(3)\n", encoding="utf-8")
    dying.chmod(dying.stat().st_mode | stat.S_IXUSR)
    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(dying)
    win = MainWindow(engine, settings)
    engine.start()
    try:
        # reaching the assertions at all proves the event loop kept running
        assert wait_until(app, lambda: getattr(win, "engine_stopped_box", None) is not None, 10_000)
        assert win.engine_stopped_box.isVisible()
        assert "3" in win.engine_stopped_box.text()
    finally:
        win._quit_confirmed = True
        win.close()


def _items(project: dict) -> list[dict]:
    out: list[dict] = []

    def walk(b: dict) -> None:
        for c in b.get("children") or []:
            if "item" in c:
                out.append(c)
            else:
                walk(c)

    walk((project or {}).get("root") or {})
    return out


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_source_monitor(app, tmp_path: Path):
    from cfa4_qt.commands import Command
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1280, 800)
    win.show()
    engine.start()
    try:
        assert wait_until(app, lambda: win.project is not None and len(win.registry.commands) > 100, 60_000)
        movie = next(i for i in _items(win.project) if i.get("type") == "Movie")
        src = win.source_monitor
        assert not src.has_content()
        win.execute("source.open", {"item": movie["item"]})
        assert wait_until(app, lambda: src.has_content() and src.view.image is not None, 60_000)
        assert win.active_monitor == "source"
        assert win.docks["Source"].isVisible()
        ft = src.frame()
        # Step Forward with the Source monitor focused moves the source playhead, not the program's
        program_before = win.monitor.playhead
        win.run_command(win.registry.commands["playhead.stepForward"])
        win.run_command(win.registry.commands["playhead.stepForward"])
        assert wait_until(app, lambda: (win.source_state or {}).get("playhead") == 2 * ft, 10_000), win.source_state
        assert win.monitor.playhead == program_before
        # Mark In / Mark Out go to the source clip
        win.run_command(win.registry.commands["markers.markIn"])
        assert wait_until(app, lambda: (win.source_state or {}).get("markIn") == 2 * ft, 10_000), win.source_state
        src.seek(10 * ft)
        win.run_command(win.registry.commands["markers.markOut"])
        assert wait_until(app, lambda: (win.source_state or {}).get("markOut") == 10 * ft, 10_000), win.source_state
        assert src.scrub.mark_in == 2 * ft and src.scrub.mark_out == 10 * ft
        # the program's own In point stayed untouched
        assert (win.sequence or {}).get("in") is None
        # Overwrite puts the marked range into the sequence
        count = lambda: sum(len(t.get("items") or []) for t in (win.sequence or {}).get("video") or [])
        before = count()
        src.buttons["overwrite"].click()
        assert wait_until(app, lambda: count() > before or bool(win.events.text.toPlainText().strip()), 20_000)
        assert count() > before, win.events.text.toPlainText()
        assert isinstance(win.registry.commands.get("source.overwrite"), Command)
    finally:
        win._quit_confirmed = True
        win.close()


# Source/Effect Controls/Metadata, Project/Info/Effects/Markers/History, Timeline/Events (egui's)
TAB_GROUPS = 4  # Source, Project, Properties (with the closed Lumetri Color, as egui), Timeline


def test_layout_has_no_orphaned_tab_bars(app, fake_engine: Path, tmp_path: Path):
    """Regression: tabifying docks before show() left a stale, visible copy of a tab bar drawn
    over the panels. Every visible tab bar must belong to exactly one tab group."""
    from cfa4_qt.main_window import MainWindow

    def bars(win) -> list[tuple[str, ...]]:
        return [
            tuple(tb.tabText(i) for i in range(tb.count()))
            for tb in win.findChildren(QtWidgets.QTabBar)
            if tb.isVisible() and tb.parent() is win  # the dock groups' bars, not a panel's own tabs
        ]

    for _ in range(2):  # first start (default layout), then with the saved layout
        settings = QtCore.QSettings(str(tmp_path / "layout.ini"), QtCore.QSettings.Format.IniFormat)
        win = MainWindow(EngineClient(fake_engine), settings)
        win.resize(1440, 900)
        win.show()
        assert wait_until(app, lambda w=win: len(bars(w)) >= TAB_GROUPS, 5_000)
        wait_until(app, lambda: False, 300)
        found = bars(win)
        assert len(found) == len(set(found)) == TAB_GROUPS, found
        win.reset_layout.trigger()
        wait_until(app, lambda: False, 300)
        found = bars(win)
        assert len(found) == len(set(found)) == TAB_GROUPS, found
        win._quit_confirmed = True
        win.close()


# ----- export window -----
def test_export_helpers(tmp_path: Path, monkeypatch):
    from cfa4_qt.panels.export import default_output_dir, format_eta, safe_file_name, with_extension

    assert with_extension("/a/b.mp4", "mov") == "/a/b.mov"
    assert with_extension("/a/b", "mp4") == "/a/b.mp4"
    assert with_extension("/a/b.mp4", "") == "/a/b.mp4"
    assert safe_file_name("Main/Edit\\x") == "Main_Edit_x"
    assert safe_file_name(" .. ") == "export"
    assert format_eta(75) == "1:15" and format_eta(3725) == "1:02:05" and format_eta("x") == ""
    project = tmp_path / "p" / "film.fcproj"
    project.parent.mkdir()
    assert default_output_dir(str(project)) == project.parent
    monkeypatch.setenv("XDG_VIDEOS_DIR", str(tmp_path))
    assert default_output_dir(None) == tmp_path


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_export_through_the_dialog_and_reimport(app, tmp_path: Path):
    """ADR F8 phase 6: export from the Qt GUI with sound, then import the file back."""
    import shutil
    import subprocess

    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.show()
    engine.start()
    try:
        assert wait_until(app, lambda: win.sequence is not None and len(win.registry.commands) > 100, 60_000)
        # File ▸ Export ▸ Media… opens the export window instead of a parameter form
        win.run_command(win.registry.commands["file.exportMedia"])
        dlg = win.export_dialog
        assert dlg is not None and dlg.isVisible()
        assert wait_until(app, lambda: dlg.preset.count() > 0 and dlg.format.count() > 0, 30_000)
        # untouched, the preset decides codec, bitrate mode, sound and hardware encoding
        untouched = dlg.params()
        for key in ("bitrateMode", "audioCodec", "audio", "hardwareEncoding", "burnCaptions"):
            assert key not in untouched, untouched
        assert untouched["sequence"] == win.sequence["id"]
        dlg.hardware.setChecked(True)
        assert dlg.params()["hardwareEncoding"] == "auto"
        dlg.hardware.setChecked(False)
        assert dlg.params()["hardwareEncoding"] == "off"

        dlg.range.setCurrentIndex(dlg.range.findData("custom"))
        dlg.start.setValue(0.0)
        dlg.end.setValue(1.0)
        dlg.width_spin.setValue(160)
        dlg.height_spin.setValue(90)
        out = tmp_path / "out" / "cut.mp4"
        dlg.path.setText(str(out))
        assert wait_until(app, lambda: "160×90" in dlg.summary.text(), 10_000), dlg.summary.text()
        done: list[tuple[str, bool]] = []
        dlg.finished_export.connect(lambda p, ok: done.append((p, ok)))
        dlg.export_button.click()
        assert wait_until(app, lambda: bool(done), 240_000), dlg.summary.text()
        assert done[0] == (str(out), True)
        assert out.is_file() and out.stat().st_size > 0
        if shutil.which("ffprobe"):
            probe = subprocess.run(
                ["ffprobe", "-v", "error", "-show_entries", "stream=codec_type,width,height", "-of", "json", str(out)],
                capture_output=True, check=True, timeout=60,
            )
            streams = json.loads(probe.stdout)["streams"]
            kinds = {s["codec_type"] for s in streams}
            assert kinds == {"video", "audio"}, streams
            video = next(s for s in streams if s["codec_type"] == "video")
            assert (video["width"], video["height"]) == (160, 90)
        # the export goes back into the project
        before = len(_items(win.project))
        win.execute("file.import", {"paths": [str(out)]})
        assert wait_until(app, lambda: len(_items(win.project)) == before + 1, 30_000)
        assert any(i.get("name") == "cut.mp4" for i in _items(win.project))
    finally:
        if win.export_dialog is not None:
            win.export_dialog.hide()
        win._quit_confirmed = True
        win.close()


def test_tick_signals_carry_64_bit_times(app):
    """Regression: Signal(int) is a 32-bit C int in PySide6, so a playhead past 8.5 ms (ticks are
    254 016 000 000 per second) wrapped to a wrong, often negative time while scrubbing the
    timeline and in playback."""
    import re

    from cfa4_qt.panels.monitor import ProgramMonitor
    from cfa4_qt.panels.timeline import TimelinePanel

    late = 3600 * TICKS_PER_SECOND + 17  # one hour in
    timeline = TimelinePanel()
    got: list[int] = []
    timeline.canvas.seek.connect(got.append)
    timeline.canvas.seek.emit(late)
    monitor = ProgramMonitor(EngineClient(Path(sys.executable)))
    monitor.playhead_moved.connect(got.append)
    monitor.scrub.seek.connect(got.append)
    monitor.playhead_moved.emit(late)
    monitor.scrub.seek.emit(late)
    assert got == [late, late, late]
    # and no tick-carrying Signal(int) comes back anywhere in the GUI
    for path in (ROOT / "python" / "cfa4_qt").rglob("*.py"):
        assert not re.search(r"Signal\([^)]*\bint\b", path.read_text(encoding="utf-8")), path


# ----- Effect Controls -----
@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_effect_controls(app, tmp_path: Path):
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1440, 900)
    win.show()
    engine.start()
    ec = win.effect_controls

    def param(effect: str, pid: str) -> dict:
        for e in (ec.clip or {}).get("effects") or []:
            if e.get("effect") == effect:
                return next(p for p in e["params"] if p["id"] == pid)
        raise AssertionError(f"{effect}.{pid} not shown")

    def history() -> list[str]:
        return [win.history.list.item(i).text() for i in range(win.history.list.count())]

    try:
        assert wait_until(app, lambda: win.sequence is not None and len(win.registry.commands) > 100, 60_000)
        clip = win.sequence["video"][0]["items"][1]
        assert ec.clip is None and "Timeline" in ec.title.text()
        win.execute("timeline.select", {"clips": [clip["clip"]]})
        assert wait_until(app, lambda: ec.clip_id == clip["clip"] and (0, "scale") in ec.rows, 30_000)
        motion = next(e["index"] for e in ec.clip["effects"] if e["effect"] == "motion")
        scale = ec.rows[(motion, "scale")]
        assert scale.kind == "float" and not scale.animated

        # egui's hot number: a scrub of one value is one undo step; a typed value (a calculation
        # too) is its own
        steps = len(history())
        line = scale.spins[0].lineEdit()
        y = line.height() // 2
        QtTest.QTest.mousePress(line, QtCore.Qt.MouseButton.LeftButton, QtCore.Qt.KeyboardModifier.NoModifier, QtCore.QPoint(10, y))
        for x in (20, 40, 60):
            QtTest.QTest.mouseMove(line, QtCore.QPoint(x, y))
            wait_until(app, lambda: False, 30)
        QtTest.QTest.mouseRelease(line, QtCore.Qt.MouseButton.LeftButton, QtCore.Qt.KeyboardModifier.NoModifier, QtCore.QPoint(60, y))
        assert wait_until(app, lambda: param("motion", "scale")["value"] > 100.0, 10_000), param("motion", "scale")
        assert wait_until(app, lambda: len(history()) == steps + 1, 10_000), history()
        QtTest.QTest.mouseClick(line, QtCore.Qt.MouseButton.LeftButton, QtCore.Qt.KeyboardModifier.NoModifier, QtCore.QPoint(10, y))
        line.selectAll()
        QtTest.QTest.keyClicks(line, "270/2")
        QtTest.QTest.keyClick(line, QtCore.Qt.Key.Key_Return)
        assert wait_until(app, lambda: param("motion", "scale")["value"] == 135.0, 10_000), param("motion", "scale")
        assert wait_until(app, lambda: len(history()) == steps + 2, 10_000), history()

        # animate: the stopwatch puts a keyframe at the playhead; a change elsewhere adds another
        win._seek_program(clip["start"])
        assert wait_until(app, lambda: ec.clip and ec.clip.get("playhead") == clip["start"], 10_000)
        scale.stopwatch.click()
        assert wait_until(app, lambda: param("motion", "scale")["animated"], 10_000)
        # the engine puts the playhead on a frame boundary: use the time it reports
        win._seek_program(clip["start"] + (clip["end"] - clip["start"]) // 2)
        assert wait_until(app, lambda: ec.clip.get("playhead") != clip["start"], 10_000)
        later = ec.clip["playhead"]
        assert clip["start"] < later < clip["end"]
        scale.spins[0].setValue(60.0)
        assert wait_until(app, lambda: len(param("motion", "scale")["keyframes"]) == 2, 10_000), param("motion", "scale")
        assert scale.strip.isVisible() and scale.next_key.isVisible()
        times = sorted(k["time"] for k in param("motion", "scale")["keyframes"])
        # keyframe navigation moves the Program playhead to the keyframes
        scale.prev_key.click()
        assert wait_until(app, lambda: ec.clip.get("playhead") == times[0], 10_000), (ec.clip.get("playhead"), times)
        scale.next_key.click()
        assert wait_until(app, lambda: ec.clip.get("playhead") == times[1], 10_000)
        # the keyframe button removes the keyframe under the playhead
        scale.add_key.click()
        assert wait_until(app, lambda: len(param("motion", "scale")["keyframes"]) == 1, 10_000)

        # a choice parameter and the effect's on/off switch
        opacity = next(e["index"] for e in ec.clip["effects"] if e["effect"] == "opacity")
        blend = ec.rows[(opacity, "blend")]
        blend.editor.setCurrentIndex(3)
        assert wait_until(app, lambda: param("opacity", "blend")["value"] == 3, 10_000)
        check = ec.effect_boxes[opacity].findChild(QtWidgets.QCheckBox, f"effectControls.{opacity}.enabled")
        was = check.isChecked()
        check.click()
        assert wait_until(
            app, lambda: next(e for e in ec.clip["effects"] if e["index"] == opacity)["enabled"] != was, 10_000
        )

        # deselecting clears the panel
        win.execute("edit.deselectAll", {})
        assert wait_until(app, lambda: ec.clip is None and not ec.rows, 10_000)
    finally:
        win._quit_confirmed = True
        win.close()



def test_file_menu_quits_and_the_window_shows_the_cfa4_logo(app, fake_engine: Path, tmp_path: Path):
    """File ▸ Quit exists (Ctrl+Q, asks to save like the close button), and the GUI uses the CFA4
    logo, not an inherited upstream one."""
    from cfa4_qt import branding
    from cfa4_qt.main_window import MainWindow

    assert branding.LOGO.is_file() and all(branding.icon_file(n).is_file() for n in branding.ICON_SIZES)
    icon = branding.app_icon()
    assert not icon.isNull() and {s.width() for s in icon.availableSizes()} >= {16, 32, 48, 256}
    assert branding.logo_pixmap(180).width() == 180
    # inherited marks anywhere in the repository: test_independence.py

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(fake_engine)
    win = MainWindow(engine, settings)
    win.show()
    engine.start()
    try:
        assert wait_until(app, lambda: "file.save" in win.registry.actions)
        file_menu = next(a.menu() for a in win.menuBar().actions() if a.menu() and a.menu().objectName() == "menu.File")
        last = [a for a in file_menu.actions() if not a.isSeparator()][-1]
        assert last is win.quit_action and last.text() == "Quit"
        assert win.quit_action.shortcuts() and win.quit_action.shortcuts()[0].toString() == "Ctrl+Q"
        win._quit_confirmed = True
        win.quit_action.trigger()
        assert wait_until(app, lambda: not win.isVisible(), 5_000)
    finally:
        if win.isVisible():
            win._quit_confirmed = True
            win.close()


# ----- issues found comparing with the egui editor (2026-10-25) -----
def test_ruler_labels_fall_on_whole_timecode():
    """Regression: at 23.976 fps the ruler read 00:00:01:23, 00:00:03:23 ... (wall-clock seconds
    shown as timecode). Labels must fall on whole timecode units: 00:00:05:00, 00:00:10:00."""
    from cfa4_qt.panels.timeline import ruler_step

    for num, den in ((24000, 1001), (24, 1), (25, 1), (30000, 1001), (50, 1), (60000, 1001)):
        ft = TICKS_PER_SECOND * den // num
        fps = round(num / den)
        for pps in (1, 5, 20, 40, 120, 300, 1000, 4000):
            step = ruler_step(ft, pps, 90)
            assert step * pps / TICKS_PER_SECOND >= 90 or step >= 3600 * fps * ft
            frames = step // ft
            # whole seconds, or a divisor of a second: every label lands on a frame :00
            assert frames % fps == 0 or fps % frames == 0, (num, den, pps, frames)
            labels = [format_timecode(i * step, ft) for i in range(1, 6)]
            if frames % fps == 0:
                assert all(lab.endswith(":00") for lab in labels), labels
    ft = TICKS_PER_SECOND * 1001 // 24000
    assert [format_timecode(i * ruler_step(ft, 40, 90), ft) for i in (1, 2)] == ["00:00:05:00", "00:00:10:00"]


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_every_track_is_reachable_in_a_short_timeline(app, tmp_path: Path):
    """Regression: the timeline had no vertical scrolling, so in a short Timeline panel Audio 2
    (the demo's music) showed as a sliver and Audio 3 not at all."""
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1600, 900)
    win.show()
    engine.start()
    try:
        assert wait_until(app, lambda: win.sequence is not None, 60_000)
        panel = win.timeline
        canvas = panel.canvas
        canvas.setFixedHeight(200)  # short, like the default dock
        wait_until(app, lambda: False, 200)
        names = [r.name for r in canvas.rows]
        assert names[-3:] == ["Audio 1", "Audio 2", "Audio 3"], names
        last = canvas.rows[-1]
        assert last.top + last.height > canvas.height(), "the demo must not fit, or this test proves nothing"
        assert panel.vscroll.isVisible() and panel.vscroll.maximum() == canvas.max_scroll_y() > 0
        panel.vscroll.setValue(panel.vscroll.maximum())
        wait_until(app, lambda: False, 100)
        last = canvas.rows[-1]
        assert last.top + last.height <= canvas.height(), (last.top, canvas.height())
        # the music clip on Audio 2 can now be hit, and nothing is hit under the ruler
        a2 = next(r for r in canvas.rows if r.name == "Audio 2")
        clip = a2.track["items"][0]
        rect = canvas.clip_rect(a2, clip)
        row, hit_clip, _edge = canvas.hit(rect.center().toPoint())
        assert row is a2 and hit_clip is clip
        assert canvas.row_at(5) is None
        # Shift+wheel and the wheel over the track headers scroll vertically too
        panel.vscroll.setValue(0)
        ev = QtGui.QWheelEvent(
            QtCore.QPointF(20, 100), QtCore.QPointF(20, 100), QtCore.QPoint(0, 0), QtCore.QPoint(0, -240),
            QtCore.Qt.MouseButton.NoButton, QtCore.Qt.KeyboardModifier.NoModifier, QtCore.Qt.ScrollPhase.NoScrollPhase, False,
        )
        canvas.wheelEvent(ev)
        assert canvas.scroll_y > 0 and panel.vscroll.value() == canvas.scroll_y
        # a tall panel shows everything and hides the bar
        canvas.setMinimumHeight(0)
        canvas.setMaximumHeight(16_777_215)
        canvas.setFixedHeight(900)
        wait_until(app, lambda: False, 200)
        assert canvas.scroll_y == 0 and not panel.vscroll.isVisible()
    finally:
        win._quit_confirmed = True
        win.close()


def test_app_identifies_as_cfa4_to_the_desktop():
    """Regression: the GUI registered as "cfa4", which matches no .desktop file, so KDE's
    taskbar showed "python3" and no CFA4 logo. It must use the packaged app id."""
    src = (ROOT / "python" / "cfa4_qt" / "__main__.py").read_text(encoding="utf-8")
    assert 'setDesktopFileName("io.github.ubuntuokos.cfa4")' in src
    assert (ROOT / "packaging" / "linux" / "io.github.ubuntuokos.cfa4.desktop").is_file()


# ----- desktop entry for checkout runs -----
def test_desktop_entry_is_opt_in_and_never_overrides_a_package(tmp_path: Path, monkeypatch):
    """Regression: run from a checkout, KDE's panel showed "python3" (no .desktop for the app
    id). `cfa4-qt --install-desktop-entry` writes one; packaged entries and runs are untouched."""
    import io

    from cfa4_qt import desktop_entry as de
    from cfa4_qt.__main__ import main

    home = tmp_path / "data"
    system = tmp_path / "system"
    monkeypatch.setenv("XDG_DATA_HOME", str(home))
    monkeypatch.setenv("XDG_DATA_DIRS", str(system))
    for var in ("APPIMAGE", "FLATPAK_ID", *de.ENV_FLAGS):
        monkeypatch.delenv(var, raising=False)
    entry = home / "applications" / "io.github.ubuntuokos.cfa4.desktop"
    assert not de.requested_by_env()
    assert main(["--install-desktop-entry"]) == 0
    text = entry.read_text(encoding="utf-8")
    assert "Icon=io.github.ubuntuokos.cfa4" in text and "Name=CFA4 (development)" in text
    exec_line = next(line for line in text.splitlines() if line.startswith("Exec="))
    assert exec_line.endswith(" %f") and "%" not in exec_line[:-3]
    assert (home / "icons" / "hicolor" / "48x48" / "apps" / "io.github.ubuntuokos.cfa4.png").is_file()
    assert (home / "icons" / "hicolor" / "scalable" / "apps" / "io.github.ubuntuokos.cfa4.svg").is_file()
    # idempotent
    out = io.StringIO()
    assert de.install(out) and "up to date" in out.getvalue()
    # an installed package's entry wins: nothing is rewritten
    entry.unlink()
    (system / "applications").mkdir(parents=True)
    (system / "applications" / "io.github.ubuntuokos.cfa4.desktop").write_text("[Desktop Entry]\n", encoding="utf-8")
    assert de.install(io.StringIO()) and not entry.exists()
    # AppImage / Flatpak runs never write
    (system / "applications" / "io.github.ubuntuokos.cfa4.desktop").unlink()
    monkeypatch.setenv("FLATPAK_ID", "io.github.ubuntuokos.cfa4")
    assert not de.install(io.StringIO()) and not entry.exists()
    monkeypatch.delenv("FLATPAK_ID")
    # paths a desktop entry cannot carry are refused, not written broken
    with pytest.raises(ValueError):
        de.exec_line(["/tmp/a%b/cfa4-qt"])
    # quoted and escaped per the Desktop Entry spec, not the shell: a literal backslash survives
    # the key file's string decoding (shell quoting turned "\\t" into a tab)
    assert de.exec_line(["/opt/cfa4 dev/cfa4-qt"]) == '"/opt/cfa4 dev/cfa4-qt"'
    assert de.escape_exec_argument('/a\\tb/"x"$') == '"/a\\\\\\\\tb/\\\\"x\\\\"\\\\$"'
    monkeypatch.setenv("CFA4_DEV_DESKTOP_ENTRY", "1")
    assert de.requested_by_env()



# ----- Window menu (as the egui editor's) -----
def test_window_menu_lists_every_egui_panel_with_its_shortcut(app, fake_engine: Path, tmp_path: Path):
    """The owner compared the menus: egui's Window menu has Workspaces, New Project Panel and all
    28 panels with Shift+1..9; the Qt GUI had a "Panels" menu with 8. Same entries, same order,
    same keys now; panels the Qt GUI lacks are listed disabled."""
    from cfa4_qt.main_window import MainWindow
    from cfa4_qt.window_menu import PANELS

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(fake_engine)
    win = MainWindow(engine, settings)
    win.resize(1400, 900)
    win.show()
    engine.start()
    try:
        assert wait_until(app, lambda: "file.save" in win.registry.actions)
        top = [a.menu().objectName() for a in win.menuBar().actions() if a.menu()]
        assert "menu.Panels" not in top
        assert top.index("menu.Window") == top.index("menu.Help") - 1, top
        menu = next(a.menu() for a in win.menuBar().actions() if a.menu() and a.menu().objectName() == "menu.Window")
        entries = [a for a in menu.actions() if not a.isSeparator()]
        assert entries[0].menu() is not None and entries[0].menu().objectName() == "menu.Window.Workspaces"
        assert entries[1].text() == "New Project Panel" and not entries[1].isEnabled()
        panels = entries[2:]
        assert [a.text() for a in panels] == [p.title for p in PANELS]
        have = {"Project", "Effects", "History", "Source", "Effect Controls", "Program", "Timeline", "Properties", "Events", "Tools", "Audio Meters", "Info", "Markers", "Metadata", "Project Notes", "Timecode", "Progress", "Audio Track Mixer", "Audio Clip Mixer", "Reference Monitor", "Lumetri Scopes", "Lumetri Color", "Media Browser", "Text", "Essential Sound", "Essential Graphics", "Text to Speech"}
        for a, p in zip(panels, PANELS, strict=True):
            assert a.isEnabled() == (p.title in have), p.title
            if p.shortcut and a.isEnabled():
                assert a.shortcut().toString() == QtGui.QKeySequence(p.shortcut).toString(), p.title
            if not a.isEnabled():
                assert "Qt GUI" in a.toolTip()
        # a panel entry closes the panel when it is open in front, and shows it again
        project = next(a for a in panels if a.text() == "Project")
        win.docks["Project"].raise_()
        wait_until(app, lambda: False, 100)
        project.trigger()
        assert wait_until(app, lambda: win.docks["Project"].isHidden(), 2_000)
        project.trigger()
        assert wait_until(app, lambda: not win.docks["Project"].isHidden(), 2_000)
        # View carries Zoom to Sequence and Language
        view = next(a.menu() for a in win.menuBar().actions() if a.menu() and a.menu().objectName() == "menu.View")
        assert win.fit_action in view.actions()
        assert any(a.menu() and a.menu().objectName() == "menu.View.Language" for a in view.actions())
        # a rebuild (language change) leaves exactly one live action per Window shortcut
        win.set_language("hu")
        win.set_language("en")
        keys = [s.toString() for a in win.actions() for s in a.shortcuts()]
        assert keys.count(QtGui.QKeySequence("Shift+1").toString()) == 1, keys
        # regression: the docks kept callbacks into the deleted actions, so showing a panel after
        # a rebuild raised "Internal C++ object already deleted"
        app.sendPostedEvents(None, QtCore.QEvent.Type.DeferredDelete)
        errors: list[BaseException] = []
        hook, sys.excepthook = sys.excepthook, lambda _t, e, _tb: errors.append(e)
        try:
            win.docks["Project"].hide()
            win.docks["Project"].show()
            wait_until(app, lambda: False, 100)
        finally:
            sys.excepthook = hook
        assert not errors, errors
        live = next(a for a in win.findChildren(QtGui.QAction, "window.panel.Project"))
        assert live.isChecked()
    finally:
        win._quit_confirmed = True
        win.close()


def test_workspaces_save_switch_and_reset(app, fake_engine: Path, tmp_path: Path):
    from cfa4_qt.main_window import MainWindow
    from cfa4_qt.window_menu import DEFAULT_WORKSPACE

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    win = MainWindow(EngineClient(fake_engine), settings)
    win.resize(1400, 900)
    win.show()
    try:
        wait_until(app, lambda: False, 200)
        ws = win.workspaces
        assert ws.current == DEFAULT_WORKSPACE and ws.names() == []
        # my own layout: History closed, saved as "Vágás"
        win.docks["History"].close()
        ws.save_as("Vágás")
        assert ws.current == "Vágás" and ws.names() == ["Vágás"]
        # back to Editing: History is there again
        ws.switch(DEFAULT_WORKSPACE, win.restore_layout)
        assert wait_until(app, lambda: not win.docks["History"].isHidden(), 2_000)
        # my workspace restores my layout; Reset to Saved Layout returns to it after changes
        ws.switch("Vágás", win.restore_layout)
        assert wait_until(app, lambda: win.docks["History"].isHidden(), 2_000)
        win.docks["History"].show()
        win.reset_layout.trigger()
        assert wait_until(app, lambda: win.docks["History"].isHidden(), 2_000)
        # the menu lists it, checked
        win.rebuild_window_menu()
        menu = next(a.menu() for a in win.menuBar().actions() if a.menu() and a.menu().objectName() == "menu.Window")
        sub = next(a.menu() for a in menu.actions() if a.menu())
        mine = next(a for a in sub.actions() if a.text() == "Vágás")
        assert mine.isChecked()
    finally:
        win._quit_confirmed = True
        win.close()


# ----- timeline: track headers, thumbnails, waveforms (egui parity) -----
@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_timeline_track_headers_thumbnails_and_waveforms(app, tmp_path: Path):
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1600, 1000)
    win.show()
    engine.start()
    canvas = win.timeline.canvas

    def track(short: str) -> dict:
        kind = "video" if short[0] == "V" else "audio"
        return (win.sequence or {}).get(kind)[int(short[1:]) - 1]

    def press(short: str, key: str) -> None:
        row = next(r for r in canvas.rows if r.short == short)
        cell = next(c for c in canvas.header_cells(row) if c.key == key)
        QtTest.QTest.mouseClick(canvas, QtCore.Qt.MouseButton.LeftButton, pos=cell.rect.center())

    try:
        assert wait_until(app, lambda: win.sequence is not None and (win.sequence or {}).get("targeting"), 60_000)
        canvas.setMinimumHeight(480)
        wait_until(app, lambda: False, 200)
        shorts = [r.short for r in canvas.rows]
        assert shorts == ["V3", "V2", "V1", "A1", "A2", "A3"], shorts
        # V1 / A1 are patched and every track is targeted, as egui's defaults
        v1 = next(r for r in canvas.rows if r.short == "V1")
        cells = {c.key: c for c in canvas.header_cells(v1)}
        assert cells["patch"].on and cells["target"].on and cells["enabled"].on and not cells["locked"].on
        # the switches change the engine's tracks (one command each, undoable)
        press("V2", "locked")
        assert wait_until(app, lambda: track("V2").get("locked") is True, 10_000)
        press("A2", "muted")
        assert wait_until(app, lambda: track("A2").get("muted") is True, 10_000)
        press("A1", "solo")
        assert wait_until(app, lambda: track("A1").get("solo") is True, 10_000)
        press("V3", "enabled")
        assert wait_until(app, lambda: track("V3").get("enabled") is False, 10_000)
        # targeting and source patch
        press("V2", "target")
        assert wait_until(app, lambda: track("V2")["id"] not in win.sequence["targeting"]["targeted"], 10_000)
        press("V2", "patch")
        assert wait_until(app, lambda: win.sequence["targeting"]["videoDest"] == track("V2")["id"], 10_000)
        # clips get thumbnails (video) and waveforms (audio) from the engine
        assert wait_until(app, lambda: any(v is not None for v in canvas.media.thumbs.values()), 30_000)
        assert wait_until(app, lambda: any(v for v in canvas.media.peaks.values()), 30_000)
        peaks = next(v for v in canvas.media.peaks.values() if v)
        assert all(lo <= hi for lo, hi in peaks) and any(hi - lo > 0.001 for lo, hi in peaks)
        # hovering a switch explains it
        row = next(r for r in canvas.rows if r.short == "A1")
        cell = next(c for c in canvas.header_cells(row) if c.key == "muted")
        QtTest.QTest.mouseMove(canvas, cell.rect.center())
        assert wait_until(app, lambda: canvas.toolTip() == "Mute Track", 2_000), canvas.toolTip()
    finally:
        win._quit_confirmed = True
        win.close()


def test_default_layout_follows_the_window_size(app, fake_engine: Path, tmp_path: Path):
    """Regression: the default layout had fixed pixel sizes, so on the owner's large screen the
    timeline got 13-24 % of the window (egui gives it the lower part of the screen)."""
    from cfa4_qt.main_window import MainWindow

    for size in ((1366, 768), (2000, 1125), (3840, 2160)):
        settings = QtCore.QSettings(str(tmp_path / f"{size[0]}.ini"), QtCore.QSettings.Format.IniFormat)
        win = MainWindow(EngineClient(fake_engine), settings)
        win.resize(*size)
        win.show()
        try:
            wait_until(app, lambda: False, 400)
            share = win.docks["Timeline"].height() / win.height()
            assert 0.33 <= share <= 0.48, (size, share)
            left = win.docks["Source"].width() / win.width()
            assert 0.24 <= left <= 0.36, (size, left)
            # the Program monitor keeps a usable size
            assert win.monitor.width() >= 0.35 * win.width() and win.monitor.height() >= 0.3 * win.height()
        finally:
            win._quit_confirmed = True
            win.close()


def test_engine_ending_after_the_client_is_gone_raises_nothing(app, fake_engine: Path):
    """Regression (reference machine, at exit): the window and its EngineClient were deleted
    first, the engine ended after, and the reader thread's last "engine stopped" message raised
    RuntimeError("Signal source has been deleted") in the background."""
    import threading
    import time

    import shiboken6

    errors: list[BaseException] = []
    previous = threading.excepthook
    threading.excepthook = lambda a: errors.append(a.exc_value)
    try:
        engine = EngineClient(fake_engine)
        engine.start()
        assert wait_until(app, lambda: bool(engine.hello), 10_000)
        process = engine._process
        shiboken6.delete(engine)
        process.kill()
        process.wait(timeout=10)
        time.sleep(0.8)  # the reader and stderr threads see EOF and finish
        assert errors == [], errors
    finally:
        threading.excepthook = previous


# ----- Tools panel and the tools on the timeline (egui parity) -----
@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_tools_panel_and_every_timeline_tool(app, tmp_path: Path):
    from cfa4_qt.main_window import MainWindow
    from cfa4_qt.panels.tools import GROUPS, TOOLS

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1800, 1000)
    win.show()
    engine.start()
    canvas = win.timeline.canvas
    T = TICKS_PER_SECOND
    Qt = QtCore.Qt

    def v1() -> list[dict]:
        return list((win.sequence or {})["video"][0]["items"])

    def drag(a: QtCore.QPoint, b: QtCore.QPoint, mods=Qt.KeyboardModifier.NoModifier) -> None:
        QtTest.QTest.mousePress(canvas, Qt.MouseButton.LeftButton, mods, a)
        for i in range(1, 6):
            QtTest.QTest.mouseMove(canvas, a + (b - a) * i / 5)
        QtTest.QTest.mouseRelease(canvas, Qt.MouseButton.LeftButton, mods, b)

    def edge(clip: dict, which: str) -> QtCore.QPoint:
        row = next(r for r in canvas.rows if r.short == "V1")
        r = canvas.clip_rect(row, clip)
        x = r.left() + 2 if which == "in" else r.right() - 2
        return QtCore.QPoint(int(x), int(r.center().y()))

    def body(clip: dict) -> QtCore.QPoint:
        row = next(r for r in canvas.rows if r.short == "V1")
        return canvas.clip_rect(row, clip).center().toPoint()

    def px(seconds: float) -> int:
        return round(seconds * canvas.pps)

    def undo() -> None:
        before = (win.sequence or {}).get("video")
        win.execute("edit.undo", {})
        wait_until(app, lambda: (win.sequence or {}).get("video") != before, 10_000)

    try:
        assert wait_until(app, lambda: win.sequence is not None and len(win.registry.commands) > 100, 60_000)
        canvas.setMinimumHeight(420)
        canvas.pps = 60.0
        canvas.scroll_ticks = 0
        wait_until(app, lambda: False, 300)
        # the panel: egui's 8 groups; unsupported tools listed disabled
        assert len(win.tools.buttons) == len(GROUPS) == 8
        assert not win.tools.actions["pen"].isEnabled() and win.tools.actions["slip"].isEnabled()
        assert "Tools" in win.docks and not win.docks["Tools"].isHidden()
        # egui's keys pick the tools
        for key, tool in (("B", "ripple"), ("N", "rolling"), ("R", "rateStretch"), ("Y", "slip"), ("U", "slide"), ("A", "trackSelectForward"), ("H", "hand"), ("Z", "zoom"), ("C", "razor"), ("V", "selection")):
            QtTest.QTest.keyClick(win, QtGui.QKeySequence(key)[0].key())
            assert wait_until(app, lambda t=tool: canvas.tool == t and win.tools.current == t, 2_000), (key, canvas.tool)

        # the later V1 clips: nothing sync-locked sits after them (the engine refuses an edit
        # that would break sync, and the GUI shows that refusal in Events)
        # the music on A2 spans the whole sequence: a ripple would break sync with it, so turn
        # the other tracks' sync lock off with their header switch, as a user would
        for short in ("V2", "V3", "A2", "A3"):
            row = next(r for r in canvas.rows if r.short == short)
            cell = next(x for x in canvas.header_cells(row) if x.key == "syncLock")
            if cell.on:
                QtTest.QTest.mouseClick(canvas, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, cell.rect.center())
        kinds = {"V": "video", "A": "audio"}
        assert wait_until(
            app,
            lambda: all(not win.sequence[kinds[x[0]]][int(x[1]) - 1].get("syncLock") for x in ("V2", "V3", "A2", "A3")),
            10_000,
        )
        clips = v1()
        a, b, c = clips[3], clips[4], clips[5]
        ia, ib, ic = 3, 4, 5
        # Ripple Edit: shortening B's end pulls C (and everything after) earlier
        win.tools.select("ripple")
        drag(edge(b, "out"), edge(b, "out") - QtCore.QPoint(px(1.0), 0))
        assert wait_until(app, lambda: v1()[ic]["start"] < c["start"], 10_000), win.events.text.toPlainText()[-300:]
        assert v1()[ib]["end"] == v1()[ic]["start"], "no gap after a ripple"
        undo()
        # Rolling Edit: the A|B cut moves; both change, the sequence keeps its length
        win.tools.select("rolling")
        duration = win.sequence["duration"]
        drag(edge(a, "out"), edge(a, "out") + QtCore.QPoint(px(0.5), 0))
        assert wait_until(app, lambda: v1()[ia]["end"] > a["end"], 10_000), win.events.text.toPlainText()[-300:]
        assert v1()[ib]["start"] == v1()[ia]["end"] and win.sequence["duration"] == duration
        undo()
        # Rate Stretch: dragging the last clip's end out makes it longer and slower
        win.tools.select("rateStretch")
        drag(edge(c, "out"), edge(c, "out") + QtCore.QPoint(px(1.0), 0))
        assert wait_until(app, lambda: v1()[ic]["duration"] > c["duration"], 10_000), win.events.text.toPlainText()[-300:]
        assert v1()[ic]["speed"] < 1.0
        undo()
        # Slip: the clip stays, its media moves
        win.tools.select("slip")
        drag(body(b), body(b) + QtCore.QPoint(px(0.5), 0))
        assert wait_until(app, lambda: v1()[ib]["sourceIn"] != b["sourceIn"], 10_000), win.events.text.toPlainText()[-300:]
        assert (v1()[ib]["start"], v1()[ib]["duration"]) == (b["start"], b["duration"])
        undo()
        # Slide: the clip moves between its neighbours, which give and take
        win.tools.select("slide")
        drag(body(b), body(b) + QtCore.QPoint(px(0.5), 0))
        assert wait_until(app, lambda: v1()[ib]["start"] > b["start"], 10_000), win.events.text.toPlainText()[-300:]
        assert v1()[ia]["end"] == v1()[ib]["start"] and v1()[ib]["duration"] == b["duration"]
        undo()
        # Track Select Forward: everything ending after the click, on every track
        win.tools.select("trackSelectForward")
        QtTest.QTest.mouseClick(canvas, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, body(c))
        t = canvas.time_at(body(c).x())
        expect = {cl["clip"] for k in ("video", "audio") for tr in win.sequence[k] for cl in tr["items"] if cl["end"] > t}
        assert wait_until(app, lambda: set(win.sequence.get("selection") or []) == expect, 10_000)
        # Hand scrolls, Zoom zooms (Alt: out)
        win.tools.select("hand")
        canvas.scroll_ticks = 5 * T
        drag(QtCore.QPoint(800, 200), QtCore.QPoint(600, 200))
        assert canvas.scroll_ticks > 5 * T
        win.tools.select("zoom")
        pps = canvas.pps
        QtTest.QTest.mouseClick(canvas, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, QtCore.QPoint(700, 200))
        assert canvas.pps > pps
        QtTest.QTest.mouseClick(canvas, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.AltModifier, QtCore.QPoint(700, 200))
        assert abs(canvas.pps - pps) < 1e-6
        # unsupported tools cannot be picked
        win.tools.select("pen")
        assert win.tools.current == "zoom" and canvas.tool == "zoom"
        assert all(TOOLS[t].available for g in GROUPS for t in (x.id for x in g) if win.tools.actions[t].isEnabled())
    finally:
        win._quit_confirmed = True
        win.close()


def test_audio_meter_layout_never_overlaps(app):
    """egui's meter layout rules, at any size: what doesn't fit is left out, nothing overlaps."""
    from cfa4_qt.panels.meters import _anchored, _overlaps, _size, parts, readout_font, readout_rows

    def at(w: float, h: float) -> QtCore.QRectF:
        return QtCore.QRectF(100, 50, w, h)

    roomy = parts(at(116, 400), ("L", "R"))
    assert [db for db, _ in roomy.scale] == [-3 * k for k in range(20)]
    assert roomy.unit is not None and len(roomy.names) == 2 and roomy.readout and roomy.readout[1]
    assert not parts(at(116, 80), ("L", "R")).names
    assert parts(at(40, 400), ("L", "R")).readout is None
    for names in (("L", "R"), ("L", "R", "C", "LFE", "Ls", "Rs")):
        for w in range(20, 401, 9):
            for h in range(10, 801, 23):
                rect = at(w, h)
                p = parts(rect, names)
                texts = [r for _, r in p.scale] + ([p.unit] if p.unit else []) + list(p.names)
                if p.readout:
                    r, units = p.readout
                    for _label, left, right in readout_rows(r, units):
                        lab = _anchored(_size(_label, readout_font()), left, "left")
                        val = _anchored(_size("-88.8 dBTP" if units else "-88.8", readout_font(True)), right, "right")
                        assert not _overlaps(lab, val) and r.contains(lab) and r.contains(val), (w, h)
                    texts.append(r)
                for i, a in enumerate(texts):
                    assert rect.contains(a), (w, h, a)
                    assert not _overlaps(a, p.area), (w, h, a)
                    for b in texts[i + 1 :]:
                        assert not _overlaps(a, b), (w, h, a, b)


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_audio_meters_show_what_plays(app, tmp_path: Path, monkeypatch):
    """The Audio Meters sit right of the Timeline and show the engine's levels and loudness of
    the sound being played (even without an audio device), then fall after Stop."""
    from cfa4_qt.main_window import MainWindow
    from cfa4_qt.panels import monitor as monitor_module
    from cfa4_qt.panels.meters import FLOOR

    # no audio device here (QtMultimedia's device query crashes in headless containers)
    monkeypatch.setattr(monitor_module, "HAVE_AUDIO", False)

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1800, 1000)
    win.show()
    engine.start()
    try:
        assert wait_until(app, lambda: bool(win.sequence) and win.monitor.has_content(), 20_000)
        meters, timeline = win.docks["Audio Meters"], win.docks["Timeline"]
        assert wait_until(app, lambda: meters.isVisible() and meters.width() < 200, 5_000)
        assert meters.geometry().left() >= timeline.geometry().right()
        assert abs(meters.geometry().top() - timeline.geometry().top()) < 40
        assert win.meters.levels == [FLOOR, FLOOR]
        win.monitor.seek(10 * TICKS_PER_SECOND)
        win.monitor.play()
        assert wait_until(app, lambda: max(win.meters.levels) > -60.0, 10_000), win.meters.levels
        assert wait_until(app, lambda: isinstance(win.meters.loudness[2], float), 15_000), win.meters.loudness
        assert all(lv <= 12.0 for lv in win.meters.levels)
        assert "LUFS" in win.meters.accessibleDescription()
        grab = win.meters.grab().toImage()
        assert grab.width() > 0
        win.monitor.stop()
        top = max(win.meters.levels)
        assert wait_until(app, lambda: max(win.meters.levels) < top - 10.0, 5_000), (top, win.meters.levels)
        assert isinstance(win.meters.loudness[2], float), "the integrated loudness stays after Stop"
    finally:
        win.close()
        engine.stop()


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_program_monitor_controls_as_egui(app, tmp_path: Path, monkeypatch):
    """egui's Program monitor: typed timecode, magnification with wheel zoom and pan, playback
    and paused resolution, display modes, safe margins, the transport row with its Button Editor
    and overflow menu, J/K/L shuttle, Play In to Out and Export Frame. The view is remembered."""
    from cfa4_qt.main_window import MainWindow
    from cfa4_qt.panels import monitor as monitor_module
    from cfa4_qt.panels.monitor_view import MonitorView
    from cfa4_qt.panels.transport import PROGRAM_BUTTONS

    monkeypatch.setattr(monitor_module, "HAVE_AUDIO", False)
    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1800, 1000)
    win.show()
    engine.start()
    mon = win.monitor
    try:
        assert wait_until(app, lambda: bool(win.sequence) and mon.view.image is not None and len(win.registry.commands) > 100, 30_000)
        ft = mon.frame()
        base = round(TICKS_PER_SECOND / ft)
        # the controls row
        assert mon.zoom_button.text() == "Fit" and mon.res_button.text() == "1/2"
        assert mon.view.full == (1920, 1080)
        # a typed timecode, read by the engine
        mon.timecode.setFocus()
        mon.timecode.setText("00:00:02:00")
        QtTest.QTest.keyClick(mon.timecode, QtCore.Qt.Key.Key_Return)
        assert wait_until(app, lambda: mon.playhead == 2 * base * ft, 10_000), mon.playhead
        assert wait_until(app, lambda: (win.sequence or {}).get("playhead") == 2 * base * ft, 10_000)
        mon.timecode.setText("+5")
        QtTest.QTest.keyClick(mon.timecode, QtCore.Qt.Key.Key_Return)
        assert wait_until(app, lambda: mon.playhead == (2 * base + 5) * ft, 10_000), mon.playhead
        # magnification: 200 % shows two screen pixels a frame pixel; the frame asked for grows
        fit_width = mon.view.picture_rect().width()
        mon.set_zoom(2.0)
        assert mon.zoom_button.text() == "200%"
        assert abs(mon.view.picture_rect().width() - 2 * 1920 / mon.view.devicePixelRatioF()) < 1
        # paused at Full: the frame comes at the size shown (up to the frame's own size)
        assert wait_until(app, lambda: mon.view.image is not None and mon.view.image.width() == 1920, 20_000), mon.view.image.width()
        # a pan stays inside the picture
        mon.view.pan_by(10_000, 0)
        pic, area = mon.view.picture_rect(), QtCore.QRectF(mon.view.rect())
        assert abs(pic.left() - area.left()) < 1, (pic, area)
        # the wheel zooms about the pointer, from Fit too
        mon.set_zoom(None)
        assert mon.view.picture_rect().width() == fit_width
        mon.view.zoom_about(1.1, mon.view.rect().center().toPointF())
        assert mon.mv.zoom is not None and abs(mon.view.picture_rect().width() - fit_width * 1.1) < 1
        mon.set_zoom(None)
        # Paused Resolution 1/4: a quarter of the frame
        mon.set_resolution("quarter", paused=True)
        assert wait_until(app, lambda: mon.view.image is not None and mon.view.image.width() == 480, 20_000), mon.view.image.width()
        # display modes: one channel as grey
        mon.set_view(display="red")
        assert mon.view.shown.format() == QtGui.QImage.Format.Format_Grayscale8
        mon.set_view(display="composite", safe_margins=True)
        assert mon.view.shown.format() != QtGui.QImage.Format.Format_Grayscale8
        # remembered
        saved = MonitorView.load(settings, "monitor/program")
        assert saved.paused_res == "quarter" and saved.safe_margins and saved.zoom is None
        # the wrench menu, as egui's
        mon._fill_wrench_menu()
        names = {a.objectName() for a in mon.wrench.menu().actions()}
        assert {"program.settings.display.composite", "program.settings.safeMargins", "program.settings.showTransport", "program.settings.loop"} <= names
        # the View menu acts on the focused monitor
        view_menu = next(a.menu() for a in win.menuBar().actions() if a.menu() and a.menu().objectName() == "menu.View")
        titles = [a.text() for a in view_menu.actions()]
        assert "Playback Resolution" in titles and "Magnification" in titles and "Safe Margins" in titles
        # the transport row: egui's buttons, Loop only from the Button Editor
        shown = [d.command for d in mon.transport.shown()]
        assert shown == [d.command for d in PROGRAM_BUTTONS if d.command != "playback.loop"]
        assert all(mon.buttons[d.key].isVisible() for d in mon.transport.shown())
        mon.transport.set_hidden([*mon.mv.hidden, "markers.add"])
        assert not mon.buttons["marker"].isVisible()
        assert "markers.add" in MonitorView.load(settings, "monitor/program").hidden
        mon.transport.set_hidden(["playback.loop"])
        # narrow: what doesn't fit goes to the overflow menu
        mon.transport.resize(220, mon.transport.height())
        assert mon.transport.overflow and mon.transport.more.isVisible()
        mon.transport.resize(900, mon.transport.height())
        assert not mon.transport.overflow
        # Mark In / Mark Out buttons mark the sequence
        mon.seek(2 * base * ft)
        mon.buttons["markIn"].click()
        assert wait_until(app, lambda: (win.sequence or {}).get("in") == 2 * base * ft, 10_000), win.sequence.get("in")
        mon.seek(3 * base * ft)
        mon.buttons["markOut"].click()
        assert wait_until(app, lambda: (win.sequence or {}).get("out") == 3 * base * ft, 10_000)
        assert mon.duration_label.text() == format_timecode(base * ft + ft, ft)
        # Go to In
        mon.buttons["goIn"].click()
        assert mon.playhead == 2 * base * ft
        # Play In to Out: plays the marked second once and rests at Out
        mon.play_in_to_out()
        assert mon.playing
        assert wait_until(app, lambda: not mon.playing, 10_000)
        assert mon.playhead == 3 * base * ft
        # J / K / L
        mon.shuttle(1)
        assert mon.playing and mon.speed == 1.0
        mon.shuttle(1)
        assert mon.speed == 2.0
        mon.shuttle(-1)
        assert mon.speed == -1.0
        mon.stop()
        assert not mon.playing and mon.speed == 1.0
        # Export Frame asks for the file and writes it
        out = tmp_path / "still.png"
        monkeypatch.setattr(QtWidgets.QFileDialog, "getSaveFileName", lambda *a, **k: (str(out), "PNG (*.png)"))
        mon.buttons["exportFrame"].click()
        assert wait_until(app, lambda: out.exists() and out.stat().st_size > 0, 30_000), win.events.text.toPlainText()
    finally:
        win._quit_confirmed = True
        win.close()
        engine.stop()


def test_theme_tokens_follow_egui_and_the_appearance_prefs(app):
    """egui's Darkest / Dark / Light tokens, picked as egui picks them from Settings ▸ Appearance;
    a highlight colour of the user's own tints the other blues, accessible contrast brightens."""
    from cfa4_qt import theme

    assert theme.tokens_from_prefs({}).kind == "darkest"
    assert theme.tokens_from_prefs({"appearanceMode": "dark", "darkTheme": "dark"}).panel_bg == "#323232"
    light = theme.tokens_from_prefs({"appearanceMode": "light", "lightTheme": "light", "darkTheme": "darkest"})
    assert light.kind == "light" and light.panel_bg == "#e8e8e8"
    same = theme.tokens_from_prefs({"highlightColor": "#2f6bdf"})
    assert same.playhead == theme.DARKEST.playhead, "the theme's own highlight changes nothing"
    red = theme.tokens_from_prefs({"highlightColor": "#d03030"})
    assert red.accent == "#d03030" and red.playhead != theme.DARKEST.playhead
    assert QtGui.QColor(red.playhead).hue() in range(350, 360) or QtGui.QColor(red.playhead).hue() < 10
    assert theme.tokens_from_prefs({"accessibleContrast": True}).text_dim == theme.DARKEST.text


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_dark_theme_and_egui_panel_chrome(app, tmp_path: Path):
    """The window takes the theme from the engine's prefs and switches with View ▸ Appearance;
    panels in a tab group are named once, on their tab; Tools and Audio Meters get a slim grip."""
    from cfa4_qt import theme
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1600, 900)
    win.show()
    engine.start()
    try:
        assert wait_until(app, lambda: bool(win.appearance), 20_000)
        assert theme.current().kind == "darkest"
        assert app.palette().color(QtGui.QPalette.ColorRole.Window).name() == "#1d1d1d"
        assert win.tabPosition(QtCore.Qt.DockWidgetArea.LeftDockWidgetArea) == QtWidgets.QTabWidget.TabPosition.North
        assert wait_until(app, lambda: win.docks["Source"].property("titleMode") == "tab", 5_000)
        assert win.docks["Source"].titleBarWidget().height() == 0
        assert win.docks["Tools"].property("titleMode") == "grip"
        assert win.docks["Audio Meters"].titleBarWidget().height() == 6
        appearance = next(a.menu() for a in win.menuBar().actions() if a.menu() and a.menu().objectName() == "menu.View").findChild(QtWidgets.QMenu, "menu.View.Appearance")
        assert [a.objectName() for a in appearance.actions()] == ["view.theme.darkest", "view.theme.dark", "view.theme.light", "view.appearanceMode.next"]
        win.set_theme("light")
        assert wait_until(app, lambda: theme.current().kind == "light", 10_000)
        assert app.palette().color(QtGui.QPalette.ColorRole.Window).name() == "#e8e8e8"
        assert win.appearance.get("appearanceMode") == "light"
        win.set_theme("dark")
        assert wait_until(app, lambda: theme.current().kind == "dark", 10_000)
        win.set_theme("darkest")
        assert wait_until(app, lambda: theme.current().kind == "darkest", 10_000)
    finally:
        win._quit_confirmed = True
        win.close()
        engine.stop()


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_markers_info_metadata_and_project_notes(app, tmp_path: Path):
    """egui's Markers (colour filter, go to, recolour), Metadata (editable log fields), Info and
    Project Notes panels, in egui's Editing tab groups."""
    from cfa4_qt.main_window import MainWindow
    from cfa4_qt.panels.project import ITEM_ROLE

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1600, 900)
    win.show()
    engine.start()
    try:
        assert wait_until(app, lambda: bool(win.sequence) and win.project is not None, 30_000)
        # egui's groups
        assert {win.docks["Metadata"], win.docks["Audio Clip Mixer"]} <= set(win.tabifiedDockWidgets(win.docks["Source"]))
        assert {win.docks[n] for n in ("Info", "Effects", "Markers", "History")} <= set(win.tabifiedDockWidgets(win.docks["Project"]))
        assert not win.docks["Project Notes"].isVisible()
        # Markers: every sequence marker, newest data from the engine
        markers = win.sequence["markers"]
        assert wait_until(app, lambda: len(win.markers.list.markers) == len(markers), 10_000)
        greens = [m for m in markers if m["color"] == "Green"]
        assert greens
        win.markers.chips["Green"].click()
        assert wait_until(app, lambda: len(win.markers.list.markers) == len(markers) - len(greens), 10_000)
        assert "Green" in win.hidden_markers
        win.markers.chips["Green"].click()
        assert wait_until(app, lambda: len(win.markers.list.markers) == len(markers), 10_000)
        target = win.markers.list.markers[2]
        win.docks["Markers"].raise_()
        assert wait_until(app, lambda: win.markers.list.isVisible(), 5_000)
        QtTest.QTest.mouseClick(win.markers.list, QtCore.Qt.MouseButton.LeftButton, QtCore.Qt.KeyboardModifier.NoModifier, win.markers.list.row_rect(2).center())
        assert wait_until(app, lambda: (win.sequence or {}).get("playhead") == target["start"], 10_000)
        menu = win.markers.list.menu_for(2)
        next(a for a in menu.findChildren(QtGui.QAction) if a.objectName() == "markers.menu.color.Teal").trigger()
        assert wait_until(app, lambda: next(m for m in win.sequence["markers"] if m["id"] == target["id"])["color"] == "Teal", 10_000)
        # Metadata and Info follow the Project panel's selection
        movie = next(i for i in _items(win.project) if i.get("name") == "Ocean_Sunset.mp4")
        tree = win.project_panel.tree
        node = next(it for it in tree.findItems("Ocean_Sunset.mp4", QtCore.Qt.MatchFlag.MatchRecursive) if it.data(0, ITEM_ROLE) == movie["item"])
        tree.setFocus()
        tree.setCurrentItem(node)
        node.setSelected(True)
        assert win.project_panel.selected_items() == [movie["item"]]
        assert wait_until(app, lambda: win.metadata.item == movie["item"], 10_000), (win.project_panel.selected_items(), win.target_item, win.inspector_source, win.metadata.item)
        assert win.metadata.header.text() == "Clip: Ocean_Sunset.mp4"
        scene = win.metadata.edits["Scene"]
        scene.setText("12A")
        scene.editingFinished.emit()
        assert wait_until(app, lambda: any(f["name"] == "Scene" and f["value"] == "12A" for f in win.metadata.fields), 10_000)
        assert wait_until(app, lambda: "Ocean_Sunset.mp4" in win.info.text.text() and "1920 x 1080" in win.info.text.text(), 10_000)
        assert "Video 1:" in win.info.text.text() and "Playhead:" in win.info.text.text()
        # Project Notes: from Window ▸ Project Notes; typing is saved in the project
        win.docks["Project Notes"].show()
        win.docks["Project Notes"].raise_()
        win.notes.edit.setFocus()
        QtTest.QTest.keyClicks(win.notes.edit, "Check the mix")
        win.notes.flush()
        got: list[str] = []
        assert wait_until(app, lambda: (engine.execute("project.notes", {}, on_reply=lambda r, _b: got.append(r.get("text"))) and False) or "Check the mix" in got, 10_000)
    finally:
        win._quit_confirmed = True
        win.close()
        engine.stop()


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_timecode_and_progress_panels(app, tmp_path: Path):
    """egui's Timecode panel (rows read out by the engine, row menu, "+", remembered) and
    Progress panel (the engine's jobs, cancel)."""
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1600, 900)
    win.show()
    engine.start()
    panel = win.timecode_panel
    try:
        assert wait_until(app, lambda: bool(win.sequence), 30_000)
        assert not win.docks["Timecode"].isVisible() and not win.docks["Progress"].isVisible()
        win.docks["Timecode"].show()
        win.docks["Timecode"].raise_()
        ft = win.monitor.frame()
        base = round(TICKS_PER_SECOND / ft)
        win._seek_program(2 * base * ft)
        assert wait_until(app, lambda: [v.get("value") for v in panel.values] == ["00:00:02:00"], 10_000), panel.values
        assert panel.values[0]["mode"] == "Current Time" and panel.values[0]["source"] == "Program"
        # "+" adds a Duration row; the menu turns it into Frames of the current time
        panel.add.click()
        assert [r["mode"] for r in panel.rows] == ["current", "duration"]
        menu = panel.menu_for(1)
        next(a for a in menu.actions() if a.objectName() == "timecode.row.1.mode.current").trigger()
        menu = panel.menu_for(1)
        next(a for a in menu.actions() if a.objectName() == "timecode.row.1.display.Frames").trigger()
        assert wait_until(app, lambda: len(panel.values) == 2 and panel.values[1].get("value") == str(2 * base), 10_000), panel.values
        grab = panel.grab()
        assert grab.width() > 0
        # remembered
        from cfa4_qt.panels.readouts import TimecodePanel

        again = TimecodePanel()
        again.attach_settings(settings)
        assert again.rows == panel.rows
        # Progress: the engine's background jobs
        win.docks["Progress"].show()
        win.docks["Progress"].raise_()
        assert wait_until(app, lambda: win.progress.empty_text.isVisible(), 5_000)
        win.progress.set_jobs([{"id": 7, "label": "Render", "progress": 0.5, "finished": False, "status": "", "done": 1, "total": 2}])
        row = win.progress.rows[0]
        assert row.isVisible() and row.status().startswith("50 %") and row.stop.isVisibleTo(row)
        win.progress.set_jobs([{"id": 7, "label": "Render", "finished": True, "result": {"error": "cancelled"}}])
        assert not win.progress.rows[0].isVisible()
        win.progress.finished.setChecked(True)
        assert win.progress.rows[0].isVisible() and win.progress.rows[0].status() == "Cancelled"
    finally:
        win._quit_confirmed = True
        win.close()
        engine.stop()


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_audio_track_mixer(app, tmp_path: Path, monkeypatch):
    """egui's Audio Track Mixer: a strip per track and the Mix, fader (egui's taper, touch while
    dragged, double-click 0 dB), M / S / R, automation mode, effect slots, and per-strip meters
    of the audio being played."""
    from cfa4_qt.main_window import MainWindow
    from cfa4_qt.panels import monitor as monitor_module
    from cfa4_qt.panels.mixer import db_to_pos, pos_to_db

    assert db_to_pos(0.0) == pytest.approx(0.76) and pos_to_db(0.76) == pytest.approx(0.0)
    assert pos_to_db(0.0) == -96.0 and db_to_pos(15.0) == 1.0
    monkeypatch.setattr(monitor_module, "HAVE_AUDIO", False)
    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1700, 1000)
    win.show()
    engine.start()
    mixer = win.mixer
    try:
        assert wait_until(app, lambda: bool(win.sequence) and len(win.registry.commands) > 100, 30_000)
        assert not win.docks["Audio Track Mixer"].isVisible()
        win.docks["Audio Track Mixer"].show()
        win.docks["Audio Track Mixer"].raise_()
        assert wait_until(app, lambda: [s.ref for s in mixer.strips if s.isVisible()] == ["A1", "A2", "A3", "Mix"], 10_000)
        a1 = mixer.strips[0]

        def strip(ref: str) -> dict:
            return next(s for s in mixer.strips_data if s.get("ref") == ref)

        assert wait_until(app, lambda: a1.regions.get("cap") is not None, 5_000)
        cap = a1.regions["cap"].center().toPoint()
        Qt = QtCore.Qt
        QtTest.QTest.mousePress(a1, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, cap)
        for i in range(1, 6):
            QtTest.QTest.mouseMove(a1, cap + QtCore.QPoint(0, 8 * i))
        QtTest.QTest.mouseRelease(a1, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, cap + QtCore.QPoint(0, 40))
        assert wait_until(app, lambda: strip("A1")["volumeDb"] < -1.0, 10_000), strip("A1")["volumeDb"]
        QtTest.QTest.mouseDClick(a1, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, a1.regions["cap"].center().toPoint())
        assert wait_until(app, lambda: abs(strip("A1")["volumeDb"]) < 0.01, 10_000), strip("A1")["volumeDb"]
        # egui's hot values: type the fader value (a calculation too), scrub the pan value
        from cfa4_qt.hot_number import HotSpin

        value = a1.findChild(HotSpin, "mixer.A1.value")
        line = value.lineEdit()
        QtTest.QTest.mouseClick(line, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, QtCore.QPoint(8, line.height() // 2))
        line.selectAll()
        QtTest.QTest.keyClicks(line, "-12/2")
        QtTest.QTest.keyClick(line, Qt.Key.Key_Return)
        assert wait_until(app, lambda: abs(strip("A1")["volumeDb"] + 6) < 0.01, 10_000), strip("A1")["volumeDb"]
        # a click on the cap without moving keeps the strip's level (not the last drag's)
        cap = a1.regions["cap"].center().toPoint()
        QtTest.QTest.mousePress(a1, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, cap)
        QtTest.QTest.mouseRelease(a1, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, cap)
        wait_until(app, lambda: False, 300)
        assert abs(strip("A1")["volumeDb"] + 6) < 0.01, strip("A1")["volumeDb"]
        pan = a1.findChild(HotSpin, "mixer.A1.panValue").lineEdit()
        y = pan.height() // 2
        QtTest.QTest.mousePress(pan, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, QtCore.QPoint(20, y))
        for x in (25, 35, 45):
            QtTest.QTest.mouseMove(pan, QtCore.QPoint(x, y))
        QtTest.QTest.mouseRelease(pan, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, QtCore.QPoint(45, y))
        assert wait_until(app, lambda: abs(strip("A1")["pan"] - 25) < 0.01, 10_000), strip("A1")["pan"]
        # routed to a 5.1 submix: egui's 5.1 panner (puck, double-click front centre, Center, LFE)
        win.execute("mixer.addSubmix", {"name": "Surround", "channels": "5.1"})
        assert wait_until(app, lambda: any(s.get("kind") == "submix" for s in mixer.strips_data), 10_000)
        sub = next(s for s in mixer.strips_data if s.get("kind") == "submix")
        win.execute("mixer.setStrip", {"strip": strip("A1")["id"], "output": sub["ref"]})
        assert wait_until(app, lambda: strip("A1").get("pans51") is True and "pan51" in a1.regions, 10_000), strip("A1")
        sq = a1.regions["pan51"]
        start = a1.puck_pos(0, 100).toPoint()
        QtTest.QTest.mousePress(a1, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, start)
        for i in range(1, 5):
            QtTest.QTest.mouseMove(a1, start + QtCore.QPoint(-3 * i, 3 * i))
        QtTest.QTest.mouseRelease(a1, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, start + QtCore.QPoint(-12, 12))
        assert wait_until(app, lambda: strip("A1")["pan51"]["x"] < -20 and strip("A1")["pan51"]["y"] < 80, 10_000), strip("A1")["pan51"]
        QtTest.QTest.mouseDClick(a1, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, sq.center().toPoint())
        assert wait_until(app, lambda: strip("A1")["pan51"]["x"] == 0 and strip("A1")["pan51"]["y"] == 100, 10_000), strip("A1")["pan51"]
        line = a1.findChild(HotSpin, "mixer.A1.pan51.center").lineEdit()
        QtTest.QTest.mouseClick(line, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, QtCore.QPoint(6, line.height() // 2))
        line.selectAll()
        QtTest.QTest.keyClicks(line, "50")
        QtTest.QTest.keyClick(line, Qt.Key.Key_Return)
        assert wait_until(app, lambda: strip("A1")["pan51"]["center"] == 50, 10_000), strip("A1")["pan51"]
        win.execute("mixer.setStrip", {"strip": strip("A1")["id"], "output": "Mix"})
        assert wait_until(app, lambda: "pan" in a1.regions and not strip("A1").get("pans51"), 10_000)
        # M / S / R
        QtTest.QTest.mouseClick(a1, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, a1.regions["mute"].center().toPoint())
        assert wait_until(app, lambda: strip("A1")["muted"] is True, 10_000)
        QtTest.QTest.mouseClick(a1, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, a1.regions["mute"].center().toPoint())
        assert wait_until(app, lambda: strip("A1")["muted"] is False, 10_000)
        # automation mode
        menu = a1.menu_for("mode")
        next(a for a in menu.actions() if a.objectName() == "mixer.A1.mode.Touch").trigger()
        assert wait_until(app, lambda: strip("A1")["mode"] == "Touch", 10_000)
        # an effect in the first insert slot
        mixer.fx_button.click()
        assert mixer.fx_open
        assert wait_until(app, lambda: "fx.0" in a1.layout_regions(), 2_000)
        menu = a1.menu_for("fx.0")
        acts = [a for m in [menu, *menu.findChildren(QtWidgets.QMenu)] for a in m.actions() if a.objectName().startswith("mixer.A1.fx.0.")]
        assert acts, [a.text() for a in menu.actions()]
        acts[0].trigger()
        assert wait_until(app, lambda: len(strip("A1")["inserts"]) == 1, 10_000), win.events.text.toPlainText()[-400:]
        # per-strip meters while the Program plays
        win.monitor.seek(10 * TICKS_PER_SECOND)
        win.monitor.play()
        assert wait_until(app, lambda: any(max(s.levels or [-90]) > -60 for s in mixer.strips if s.isVisible()), 15_000), [s.levels for s in mixer.strips]
        mix = next(s for s in mixer.strips if s.ref == "Mix")
        assert wait_until(app, lambda: max(mix.levels or [-90]) > -60, 10_000)
        win.monitor.stop()
        assert mixer.grab().width() > 0
    finally:
        win._quit_confirmed = True
        win.close()
        engine.stop()


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_audio_clip_mixer(app, tmp_path: Path):
    """egui's Audio Clip Mixer: a strip per audio track for the clip under the playhead; the fader
    sets the clip's Volume (a keyframe with Write Keyframes on), M / S, the clip automation mode."""
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1700, 1000)
    win.show()
    engine.start()
    cm = win.clip_mixer
    Qt = QtCore.Qt
    try:
        assert wait_until(app, lambda: bool(win.sequence) and len(win.registry.commands) > 100, 30_000)
        win.docks["Audio Clip Mixer"].raise_()
        win._seek_program(2 * TICKS_PER_SECOND)
        assert wait_until(app, lambda: [s.ref for s in cm.strips if s.isVisible()] == ["A1", "A2", "A3"], 10_000)
        a1 = cm.strips[0]
        assert wait_until(app, lambda: a1.clip() is not None and a1.regions.get("cap") is not None, 10_000), a1.data
        before = a1.volume()
        cap = a1.regions["cap"].center().toPoint()
        QtTest.QTest.mousePress(a1, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, cap)
        for i in range(1, 6):
            QtTest.QTest.mouseMove(a1, cap + QtCore.QPoint(0, -6 * i))
        QtTest.QTest.mouseRelease(a1, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, cap + QtCore.QPoint(0, -30))
        assert wait_until(app, lambda: (a1.clip() or {}).get("volumeDb", before) > before + 0.5, 10_000), (before, a1.data)
        # the hot value under the fader takes a typed value
        from cfa4_qt.hot_number import HotSpin

        line = a1.findChild(HotSpin, "clipMixer.A1.value").lineEdit()
        QtTest.QTest.mouseClick(line, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, QtCore.QPoint(8, line.height() // 2))
        line.selectAll()
        QtTest.QTest.keyClicks(line, "-3")
        QtTest.QTest.keyClick(line, Qt.Key.Key_Return)
        assert wait_until(app, lambda: abs((a1.clip() or {}).get("volumeDb", 0) + 3) < 0.01, 10_000), a1.data
        QtTest.QTest.mouseClick(a1, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, a1.regions["mute"].center().toPoint())
        assert wait_until(app, lambda: a1.data.get("muted") is True, 10_000)
        QtTest.QTest.mouseClick(a1, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, a1.regions["keyframe"].center().toPoint())
        assert a1.write_keyframes
        assert cm.grab().width() > 0
    finally:
        win._quit_confirmed = True
        win.close()
        engine.stop()


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_reference_monitor(app, tmp_path: Path):
    """egui's Reference Monitor: ganged to the Program playhead, or parked (step, Match Playhead)."""
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1600, 900)
    win.show()
    engine.start()
    ref = win.reference
    try:
        assert wait_until(app, lambda: bool(win.sequence), 30_000)
        assert not win.docks["Reference Monitor"].isVisible()
        win.docks["Reference Monitor"].show()
        win.docks["Reference Monitor"].raise_()
        assert wait_until(app, lambda: ref.view.image is not None, 20_000)
        assert not ref.ganged, "parked by default, as egui"
        ref.gang.click()
        ft = ref.frame_ticks()
        win._seek_program(48 * ft)
        assert wait_until(app, lambda: ref.frame_time() == 48 * ft, 10_000)
        assert ref.timecode.text() == format_timecode(48 * ft, ft)
        ref.forward.click()
        assert not ref.ganged and ref.frame_time() == 49 * ft
        win._seek_program(10 * ft)
        assert wait_until(app, lambda: win.monitor.playhead == 10 * ft, 5_000)
        assert ref.frame_time() == 49 * ft, "parked"
        ref.match.click()
        assert ref.frame_time() == 10 * ft
        ref.gang.click()
        assert ref.ganged
        win._seek_program(20 * ft)
        assert wait_until(app, lambda: ref.frame_time() == 20 * ft, 10_000)
        # Lumetri Scopes of the reference frame: its own scopes (egui's default two)
        ref.set_display("scopes")
        assert wait_until(app, lambda: set(ref.scopes.images) == {"vectorscopeYuv", "waveform"}, 30_000), ref.scopes.message
        next(a for a in ref.scopes_menu().actions() if a.objectName() == "reference.scope.histogram").trigger()
        assert ref.scope_kinds == ["vectorscopeYuv", "histogram", "waveform"]
        assert wait_until(app, lambda: "histogram" in ref.scopes.images, 30_000)
    finally:
        win._quit_confirmed = True
        win.close()
        engine.stop()


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_lumetri_scopes(app, tmp_path: Path):
    """egui's Lumetri Scopes: the engine's trace images of the Program frame for every scope,
    egui's 4:3 grid, presets, settings menu and footer, remembered."""
    from cfa4_qt.main_window import MainWindow
    from cfa4_qt.panels.scopes import ScopesState, layout

    cells = layout(QtCore.QRectF(0, 0, 800, 600), 4)
    assert len(cells) == 4 and abs(cells[0].width() - cells[1].width()) < 1 and cells[0].top() != cells[2].top()
    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1600, 1000)
    win.show()
    engine.start()
    panel = win.scopes
    try:
        assert wait_until(app, lambda: bool(win.sequence), 30_000)
        win._seek_program(10 * TICKS_PER_SECOND)
        assert not win.docks["Lumetri Scopes"].isVisible()
        win.docks["Lumetri Scopes"].show()
        win.docks["Lumetri Scopes"].raise_()
        assert wait_until(app, lambda: "parade" in panel.view.images, 20_000), panel.view.message
        menu = panel.settings_menu()
        presets = menu.findChild(QtWidgets.QMenu, "scopes.menu.presets")
        next(a for a in presets.actions() if a.text().startswith("Four Scopes (RGB")).trigger()
        assert panel.st.shown == ["vectorscopeYuv", "histogram", "parade", "waveform"]
        assert wait_until(app, lambda: all(k in panel.view.images for k in panel.st.shown), 30_000), panel.view.message
        vec = panel.view.images["vectorscopeYuv"][2]
        assert len(vec["targets"]) == 2 and vec["extent"] > 0
        assert panel.view.images["waveform"][1].width() > 0
        panel.clamp.setChecked(False)
        assert not panel.st.clamp and panel.st.preset == ""
        assert wait_until(app, lambda: panel.view.images["waveform"][2]["lo"] < 0, 20_000)
        assert ScopesState.load(settings, "panels/scopes").shown == panel.st.shown
        assert panel.grab().width() > 0
    finally:
        win._quit_confirmed = True
        win.close()
        engine.stop()


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_lumetri_color(app, tmp_path: Path):
    """egui's Lumetri Color: the selected video clip's Lumetri effect by section; a slider drag is
    one undo step, curves take points, the wheels an offset, the section switches and menus act."""
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1600, 1000)
    win.show()
    engine.start()
    lc = win.lumetri
    Qt = QtCore.Qt
    mouse = QtTest.QTest

    def value(pid: str):
        return (lc.params.get(pid) or {}).get("value")

    def history() -> list[str]:
        return [win.history.list.item(i).text() for i in range(win.history.list.count())]

    try:
        assert wait_until(app, lambda: win.sequence is not None and len(win.registry.commands) > 100, 60_000)
        assert not win.docks["Lumetri Color"].isVisible(), "closed in the Editing workspace, as egui"
        win.docks["Lumetri Color"].show()
        win.docks["Lumetri Color"].raise_()
        assert wait_until(app, lambda: lc.placeholder.isVisible(), 10_000)
        clip = win.sequence["video"][0]["items"][1]
        win._seek_program(clip["start"] + 1)
        win.execute("timeline.select", {"clips": [clip["clip"]]})
        assert wait_until(app, lambda: lc.clip_id == clip["clip"] and lc.add_button.isVisible(), 20_000), lc.data
        lc.add_button.click()
        assert wait_until(app, lambda: lc.effect is not None and lc.scroll.isVisible(), 20_000), lc.data
        assert lc.bodies["Basic Correction"].isVisible() and not lc.bodies["Curves"].isVisible()
        assert not lc.sliders["hdr_white"].isVisible(), "HDR rows only in an HDR sequence"

        # a slider drag is one undo step
        assert wait_until(app, lambda: history()[-1:] == ["Apply Lumetri Color"], 10_000), history()
        steps = len(history())
        s = lc.sliders["exposure"]
        lc.scroll.ensureWidgetVisible(s)
        app.processEvents()
        t = s.track()
        y = int(t.center().y())
        mouse.mousePress(s, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, QtCore.QPoint(int(t.center().x()), y))
        for i in range(1, 6):
            mouse.mouseMove(s, QtCore.QPoint(int(t.center().x() + 8 * i), y))
        mouse.mouseRelease(s, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, QtCore.QPoint(int(t.center().x() + 40), y))
        assert wait_until(app, lambda: (value("exposure") or 0) > 0.5, 10_000), lc.params.get("exposure")
        assert wait_until(app, lambda: len(history()) == steps + 1, 10_000), history()
        s.spin.setValue(-1.5)  # typed value
        assert wait_until(app, lambda: value("exposure") == -1.5, 10_000)
        mouse.mouseDClick(s, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, QtCore.QPoint(int(t.left() + 5), y))
        assert wait_until(app, lambda: value("exposure") == 0.0, 10_000), "double-click: the default"
        # the typed value and the double-click are undo steps of their own, however quick
        assert wait_until(app, lambda: len(history()) == steps + 3, 10_000), history()
        # scrubbing the hot value is one undo step too
        steps = len(history())
        line = s.spin.lineEdit()
        ly = line.height() // 2
        mouse.mousePress(line, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, QtCore.QPoint(10, ly))
        for x in (15, 25, 40):
            mouse.mouseMove(line, QtCore.QPoint(x, ly))
            wait_until(app, lambda: False, 30)
        mouse.mouseRelease(line, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, QtCore.QPoint(40, ly))
        assert wait_until(app, lambda: (value("exposure") or 0) > 0.2, 10_000), lc.params.get("exposure")
        assert wait_until(app, lambda: len(history()) == steps + 1, 10_000), history()[-4:]

        # the Look menu, the section switch
        look = lc.combos["look"]
        look.setCurrentIndex(1)
        look.activated.emit(1)
        assert wait_until(app, lambda: value("look") == 1, 10_000)
        assert lc.headers["HSL Secondary"].on is False
        head = lc.headers["HSL Secondary"]
        lc.scroll.ensureWidgetVisible(head)
        mouse.mouseClick(head, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, head.switch_rect().center().toPoint())
        assert wait_until(app, lambda: value("hsl_on") is True and lc.headers["HSL Secondary"].on, 10_000)

        # Curves: open the section, add a point on the luma curve, then the red curve
        curves = lc.headers["Curves"]
        mouse.mouseClick(curves, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, QtCore.QPoint(40, 20))
        assert lc.bodies["Curves"].isVisible()
        ed = lc.rgb_curve
        lc.scroll.ensureWidgetVisible(ed)
        app.processEvents()
        a = ed.area()
        at = QtCore.QPoint(int(a.left() + a.width() * 0.5), int(a.bottom() - a.height() * 0.75))
        mouse.mousePress(ed, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, at)
        mouse.mouseRelease(ed, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, at)
        assert wait_until(app, lambda: len(value("curve_luma") or []) == 3 and (lc.params["curve_luma"]["line"][64] or 0) > 0.6, 10_000), lc.params.get("curve_luma")
        lc.channel_buttons[1].click()
        assert ed.pid == "curve_red" and len(ed.points) == 2
        mouse.mouseDClick(ed, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, at)
        assert wait_until(app, lambda: len(value("curve_red") or []) == 2, 10_000)

        # Color Wheels: drag the midtones puck
        mouse.mouseClick(lc.headers["Color Wheels & Match"], Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, QtCore.QPoint(40, 20))
        wheel = next(w for w in lc.wheels if w.pid == "wheel_midtones")
        lc.scroll.ensureWidgetVisible(wheel)
        app.processEvents()
        c, _ro, r_in, _track = wheel.geometry_()
        mouse.mouseClick(wheel, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, QtCore.QPoint(int(c.x() + r_in / 2), int(c.y())))
        assert wait_until(app, lambda: (value("wheel_midtones") or [0, 0])[0] > 0.3, 10_000), value("wheel_midtones")

        # fx: the effect off and on
        lc.fx.click()
        assert wait_until(app, lambda: lc.data.get("enabled") is False, 10_000)
        assert lc.grab().width() > 0
        # the open sections are remembered
        assert "Curves" in str(settings.value("lumetri/openSections"))
    finally:
        win._quit_confirmed = True
        win.close()
        engine.stop()


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_media_browser(app, tmp_path: Path):
    """egui's Media Browser: locations, the directory's files in List or Thumbnail view, back / up,
    select, open, Favorites, Edit Columns, and files dropped on the Timeline or Project panel."""
    import struct
    import zlib

    from cfa4_qt.main_window import MainWindow
    from cfa4_qt.panels.media_browser import dropped_files, files_mime

    def png(path: Path, w: int, h: int) -> None:
        raw = b"".join(b"\x00" + b"".join(bytes([x * 4 % 256, 90, 200]) for x in range(w)) for _ in range(h))

        def chunk(kind: bytes, data: bytes) -> bytes:
            return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

        ihdr = struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)
        path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))

    media = tmp_path / "media"
    (media / "Shots").mkdir(parents=True)
    png(media / "still.png", 64, 36)
    (media / "notes.txt").write_text("not media", encoding="utf-8")
    assert dropped_files(files_mime(["/a/b.mov", "/c d.wav"])) == ["/a/b.mov", "/c d.wav"]

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1600, 1000)
    win.show()
    engine.start()
    mb = win.media_browser
    Qt = QtCore.Qt

    def names() -> list[str]:
        return [mb.list.topLevelItem(i).text(0) for i in range(mb.list.topLevelItemCount())]

    try:
        assert wait_until(app, lambda: win.sequence is not None and len(win.registry.commands) > 100, 60_000)
        win.docks["Media Browser"].raise_()
        assert wait_until(app, lambda: mb.isVisible() and bool(mb.data.get("dir")), 20_000), mb.data
        mb.path.setFocus()
        mb.path.setText(str(media))
        QtTest.QTest.keyClick(mb.path, Qt.Key.Key_Return)
        assert wait_until(app, lambda: mb.data.get("dir") == str(media) and names() == ["Shots", "still.png"], 20_000), (mb.data.get("dir"), names())
        headers = [mb.list.headerItem().text(i) for i in range(mb.list.columnCount())]
        assert headers[0] == "Name" and "Video Info" in headers
        info = headers.index("Video Info")
        assert wait_until(app, lambda: mb.list.topLevelItem(1).text(info) == "64 x 36", 20_000)
        assert mb.nav["up"].isEnabled() and mb.nav["back"].isEnabled()
        tops = [mb.tree.topLevelItem(i).text(0) for i in range(mb.tree.topLevelItemCount())]
        assert tops == ["Favorites", "Local Drives", "Network", "Recent Directories"]

        # select, then open a folder and come back up
        mb.list.topLevelItem(1).setSelected(True)
        assert wait_until(app, lambda: mb.data.get("selection") == [str(media / "still.png")], 10_000), mb.data.get("selection")
        assert mb.count.text() == "1 of 1 items selected"
        mb.open_entry(mb.list.topLevelItem(0).data(0, QtCore.Qt.ItemDataRole.UserRole + 1))
        assert wait_until(app, lambda: mb.data.get("dir") == str(media / "Shots") and names() == [], 10_000)
        mb.nav["up"].click()
        assert wait_until(app, lambda: mb.data.get("dir") == str(media), 10_000)

        # Favorites: the current directory joins the tree
        mb.send("mediaBrowser.favorite", {})
        assert wait_until(app, lambda: any(r.get("path") == str(media) for r in mb.roots.get("favorites") or []), 10_000), mb.roots

        # Edit Columns: Media Type in front of Size
        mb.settings_cmd({"columns": ["Name", "Media Type"]})
        assert wait_until(app, lambda: mb.list.columnCount() == 2 and mb.list.topLevelItem(1).text(1) == "Image", 10_000)

        # Thumbnail View draws the still from the engine
        mb.view_buttons["thumbnails"].click()
        assert wait_until(app, lambda: mb.stack.currentWidget() is mb.grid and any(not p.isNull() for p in mb.thumbs.values()), 20_000), mb.thumbs
        assert mb.size_slider.isVisible()

        # a file dropped on the Timeline is imported and placed there; on the Project panel, imported
        v1 = next(r for r in win.timeline.canvas.rows if r.short == "V1")
        win.place_files([str(media / "still.png")], 0, v1.track_id)
        placed = lambda: [c for t in win.sequence["video"] for c in t["items"] if "still" in str(c.get("name"))]
        assert wait_until(app, lambda: bool(placed()), 20_000), win.events.text.toPlainText()
        assert placed()[0]["start"] == 0
        assert wait_until(app, lambda: any("still" in str(i.get("name")) for i in _items(win.project or {})), 10_000)
        assert mb.grab().width() > 0
    finally:
        win._quit_confirmed = True
        win.close()
        engine.stop()


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_text_panel(app, tmp_path: Path):
    """egui's Text panel: Transcript (Transcribe when there is none, else the words), Captions (a new caption track,
    a caption added at the playhead, its text edited in the list) and Graphics (the sequence's text
    layers, edited in place)."""
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1600, 1000)
    win.show()
    engine.start()
    tp = win.text_panel

    def captions() -> list[dict]:
        return [c for t in tp.captions.tracks() for c in t.get("captions") or []]

    try:
        assert wait_until(app, lambda: win.sequence is not None and len(win.registry.commands) > 100, 60_000)
        assert win.docks["Text"].isVisible() or not tp.isVisible(), "tabbed with Properties"
        win.docks["Text"].raise_()
        assert wait_until(app, lambda: tp.isVisible() and tp.tab() == "Transcript", 10_000)
        assert wait_until(app, lambda: tp.transcript.stack.currentWidget() is tp.transcript.empty, 10_000), tp.transcript.data
        assert tp.transcript.generate.isVisible()
        # an imported transcript: speaker paragraphs of words; click, Shift+click, search
        clip = win.sequence["video"][0]["items"][0]
        sec = 254_016_000_000
        words = [
            {"text": t, "start": clip["sourceIn"] + i * sec // 2, "end": clip["sourceIn"] + i * sec // 2 + sec // 3, "speaker": int(i > 1)}
            for i, t in enumerate(["Hello", "there", "my", "friend"])
        ]
        win.execute("transcript.set", {"item": clip["item"], "transcript": {"language": "en", "words": words}})
        assert wait_until(app, lambda: len(tp.transcript.words) == 4, 10_000), tp.transcript.data
        assert tp.transcript.stack.currentWidget() is tp.transcript.body
        assert "Speaker 2" in tp.transcript.view.toPlainText() and "friend" in tp.transcript.view.toPlainText()
        assert not tp.transcript.extract.isEnabled()
        tp.transcript.select(1)
        tp.transcript.select(2, extend=True)
        assert tp.transcript.sel == (1, 2) and tp.transcript.extract.isEnabled()
        tp.transcript.search.setText("friend")
        assert wait_until(app, lambda: tp.transcript.hits == {3}, 10_000), tp.transcript.hits

        # Captions: none yet, then a track, a caption at the playhead and its text
        tp.set_tab("Captions")
        assert wait_until(app, lambda: tp.captions.stack.currentWidget() is tp.captions.empty and tp.captions.isVisible(), 10_000)
        tp.findChild(QtWidgets.QPushButton, "text.captions.newTrack").click()
        assert wait_until(app, lambda: tp.captions.stack.currentWidget() is tp.captions.body, 10_000), tp.captions.data
        assert tp.captions.note.isVisible() and tp.captions.add.isEnabled()
        tp.captions.add.click()
        assert wait_until(app, lambda: tp.captions.table.rowCount() == 1, 10_000), tp.captions.data
        assert tp.captions.table.item(0, 1).text() == captions()[0]["in"]
        # typed in the cell: Enter confirms it (not the editor's Enter shortcut, Render Effects In to Out)
        table = tp.captions.table
        table.setCurrentCell(0, 4)
        table.editItem(table.item(0, 4))
        editor = QtWidgets.QApplication.focusWidget()
        assert isinstance(editor, QtWidgets.QLineEdit), editor
        editor.selectAll()
        QtTest.QTest.keyClicks(editor, "Hello captions")
        QtTest.QTest.keyClick(editor, QtCore.Qt.Key.Key_Return)
        assert wait_until(app, lambda: [c.get("text") for c in captions()] == ["Hello captions"], 10_000), captions()
        tp.captions.search.setText("nothing like it")
        assert tp.captions.table.rowCount() == 0 and tp.captions.note.text() == "No matching captions."
        tp.captions.search.clear()

        # Graphics: a text layer shows up and edits in place
        win.execute("graphics.newText", {"text": "Hello Qt"})
        tp.set_tab("Graphics")
        assert wait_until(app, lambda: [g.get("text") for g in tp.graphics.rows] == ["Hello Qt"], 20_000), tp.graphics.rows
        assert tp.graphics.table.rowCount() == 1 and tp.graphics.table.item(0, 0).text()
        tp.graphics.table.item(0, 2).setText("Hi Qt")
        assert wait_until(app, lambda: [g.get("text") for g in tp.graphics.rows] == ["Hi Qt"], 20_000), tp.graphics.rows
        assert tp.grab().width() > 0
    finally:
        win._quit_confirmed = True
        win.close()
        engine.stop()


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_essential_sound(app, tmp_path: Path):
    """egui's Essential Sound panel: assign an audio type, the type's sections with switches and
    sliders (a drag is one undo step), presets (Edit and Browse), Clip Volume, Clear Audio Type."""
    from cfa4_qt.main_window import MainWindow
    from cfa4_qt.panels.essential_sound import SoundSlider, Switch, setting

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1600, 1000)
    win.show()
    engine.start()
    es = win.essential_sound
    mouse = QtTest.QTest
    Qt = QtCore.Qt

    def history() -> list[str]:
        return [win.history.list.item(i).text() for i in range(win.history.list.count())]

    def find(cls, name):
        return es.findChild(cls, name)

    try:
        assert wait_until(app, lambda: win.sequence is not None and len(win.registry.commands) > 100, 60_000)
        assert not win.docks["Essential Sound"].isVisible(), "closed in the Editing workspace, as egui"
        win.docks["Essential Sound"].show()
        win.docks["Essential Sound"].raise_()
        assert wait_until(app, lambda: es.placeholder.isVisible(), 10_000)
        clip = win.sequence["audio"][0]["items"][0]
        win.execute("timeline.select", {"clips": [clip["clip"]]})
        assert wait_until(app, lambda: es.clip_ids() == [clip["clip"]] and find(QtWidgets.QPushButton, "essentialSound.type.Dialogue") is not None, 20_000), es.data
        assert not es.footer.isVisible()
        find(QtWidgets.QPushButton, "essentialSound.type.Dialogue").click()
        assert wait_until(app, lambda: es.data.get("type") == "Dialogue" and es.footer.isVisible(), 10_000), es.data
        heads = [w.title for w in es.edit.findChildren(QtWidgets.QWidget) if type(w).__name__ == "SectionHeader"]
        assert heads == ["Loudness", "Repair", "Clarity", "Creative"]
        assert find(SoundSlider, "essentialSound.repair.noise.amount") is not None, "Repair opens by default"
        assert find(QtWidgets.QPushButton, "essentialSound.autoMatch") is None, "Loudness is closed by default"

        # a switch, then a slider drag as one undo step
        noise = find(SoundSlider, "essentialSound.repair.noise.amount")
        assert not noise.active
        find(Switch, "essentialSound.repair.noise.on").clicked.emit()
        assert wait_until(app, lambda: setting(es.st, "repair.noise.on") is True and noise.active, 10_000)
        assert wait_until(app, lambda: history()[-2:] == ["Set Audio Type to Dialogue", "Essential Sound"], 10_000), history()
        steps = len(history())
        es.edit_scroll.ensureWidgetVisible(noise)
        app.processEvents()
        tr_ = noise.track()
        y = int(tr_.center().y())
        mouse.mousePress(noise, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, QtCore.QPoint(int(tr_.left() + tr_.width() * 0.2), y))
        for f in (0.4, 0.6, 0.8):
            mouse.mouseMove(noise, QtCore.QPoint(int(tr_.left() + tr_.width() * f), y))
            wait_until(app, lambda: False, 30)
        mouse.mouseRelease(noise, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, QtCore.QPoint(int(tr_.left() + tr_.width() * 0.8), y))
        assert wait_until(app, lambda: abs(float(setting(es.st, "repair.noise.amount") or 0) - 8.0) < 0.2, 10_000), es.st
        assert wait_until(app, lambda: len(history()) == steps + 1, 10_000), history()[-4:]
        assert es.findChild(QtWidgets.QComboBox, "essentialSound.preset").currentText() == "(Custom)"

        # Loudness twirls open; a preset applies; Browse applies too
        es._twirl("Loudness")
        assert find(QtWidgets.QPushButton, "essentialSound.autoMatch") is not None
        combo = es.findChild(QtWidgets.QComboBox, "essentialSound.preset")
        combo.activated.emit(combo.findData("Podcast Voice"))
        assert wait_until(app, lambda: es.st.get("preset") == "Podcast Voice", 10_000), es.st.get("preset")
        es.tabs.setCurrentIndex(0)
        b = find(QtWidgets.QPushButton, "essentialSound.browse.Dialogue.Phone Call")
        assert b is not None and b.isEnabled()
        b.click()
        assert wait_until(app, lambda: es.st.get("preset") == "Phone Call", 10_000), es.st.get("preset")
        es.tabs.setCurrentIndex(1)

        # Clip Volume and Mute, then Clear Audio Type
        es.volume_on.clicked.emit()
        assert wait_until(app, lambda: setting(es.st, "volume.on") is True and es.volume.active, 10_000)
        es.volume.spin.setValue(-6.0)
        assert wait_until(app, lambda: setting(es.st, "volume.levelDb") == -6.0, 10_000), es.st.get("volume")
        es.mute.clicked.emit()
        assert wait_until(app, lambda: setting(es.st, "mute") is True, 10_000)
        assert es.grab().width() > 0
        find(QtWidgets.QPushButton, "essentialSound.clearType").click()
        assert wait_until(app, lambda: es.data.get("type") is None and not es.footer.isVisible(), 10_000), es.data.get("type")
    finally:
        win._quit_confirmed = True
        win.close()
        engine.stop()


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_essential_graphics(app, tmp_path: Path):
    """egui's Essential Graphics panel: Browse lists and applies templates (a card dropped on a
    video track places it there); Edit shows the selected graphic's layers and properties, a hot
    number scrub is one undo step, typed values, stopwatches and the source text go to the engine."""
    from cfa4_qt.hot_number import HotSpin
    from cfa4_qt.main_window import MainWindow
    from cfa4_qt.panels.essential_graphics import template_mime

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1600, 1000)
    win.show()
    engine.start()
    eg = win.essential_graphics
    mouse = QtTest.QTest
    Qt = QtCore.Qt

    def history() -> list[str]:
        return [win.history.list.item(i).text() for i in range(win.history.list.count())]

    def graphics_on(track: int) -> list[dict]:
        items = ((win.sequence or {}).get("video") or [{}] * (track + 1))[track].get("items") or []
        return [c for c in items if any(str(e.get("effect")).startswith("graphic_") for e in c.get("effects") or [])]

    try:
        assert wait_until(app, lambda: win.sequence is not None and len(win.registry.commands) > 100, 60_000)
        assert not win.docks["Essential Graphics"].isHidden(), "open in the Editing workspace, as egui"
        win.docks["Essential Graphics"].raise_()
        assert wait_until(app, lambda: eg.tab() == "browse" and eg.browser.list.count() > 0, 20_000)
        total = eg.browser.list.count()
        eg.browser.search.setText("lower third")
        assert 0 < eg.browser.list.count() < total
        eg.browser.list.item(0).setSelected(True)
        assert eg.browser.apply_button.isEnabled()
        steps = len(history())
        eg.browser.apply_button.click()
        assert wait_until(app, lambda: len(history()) > steps and eg.editor.clip is not None and eg.tab() == "edit", 20_000), history()[-3:]
        assert eg.editor.data.get("template"), "a template graphic shows its Template Properties"
        assert eg.findChild(QtWidgets.QWidget, "graphics.section.TemplateProperties") is not None

        # a new text graphic: layers and properties
        win.execute("graphics.newText", {"text": "Hello Qt"})
        assert wait_until(app, lambda: eg.editor.param("text") == "Hello Qt" and eg.editor.layer == 0, 20_000), eg.editor.data
        clip = eg.editor.clip
        eg.findChild(QtWidgets.QToolButton, "graphics.NewRectangle").click()
        assert wait_until(app, lambda: len(eg.editor.data.get("layers") or []) == 2, 10_000)
        win.execute("graphics.selectLayer", {"clip": clip, "layers": [0]})
        assert wait_until(app, lambda: eg.editor.layer == 0 and eg.editor.data.get("kind") == "text", 10_000)

        # a scrub of Opacity (half a percent a pixel) is one undo step
        spin = eg.findChild(HotSpin, "graphics.prop.opacity")
        eg.editor.scroll.ensureWidgetVisible(spin)
        app.processEvents()
        line = spin.lineEdit()
        steps = len(history())
        y = line.height() // 2
        mouse.mousePress(line, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, QtCore.QPoint(30, y))
        for x in (25, 15, 5):
            mouse.mouseMove(line, QtCore.QPoint(x, y))
            wait_until(app, lambda: False, 30)
        mouse.mouseRelease(line, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, QtCore.QPoint(5, y))
        assert wait_until(app, lambda: abs(float(eg.editor.param("opacity") or 0) - 88) < 0.5, 10_000), eg.editor.param("opacity")
        assert wait_until(app, lambda: len(history()) == steps + 1, 10_000), history()[-4:]

        # a click types a value; Enter sets it (and runs no menu shortcut)
        spin = eg.findChild(HotSpin, "graphics.prop.scale")
        eg.editor.scroll.ensureWidgetVisible(spin)
        app.processEvents()
        line = spin.lineEdit()
        mouse.mouseClick(line, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, QtCore.QPoint(10, line.height() // 2))
        assert line.hasFocus()
        line.selectAll()
        mouse.keyClicks(line, "50")
        mouse.keyClick(line, Qt.Key.Key_Return)
        assert wait_until(app, lambda: abs(float(eg.editor.param("scale") or 0) - 50) < 0.01, 10_000), eg.editor.param("scale")

        # a stopwatch animates the property
        eg.findChild(QtWidgets.QToolButton, "graphics.prop.rotation.stopwatch").click()
        assert wait_until(app, lambda: ((eg.editor.data.get("params") or {}).get("rotation") or {}).get("animated") is True, 10_000)

        # the source text
        source = eg.findChild(QtWidgets.QPlainTextEdit, "graphics.sourceText")
        eg.editor.scroll.ensureWidgetVisible(source)
        source.setFocus()
        source.moveCursor(QtGui.QTextCursor.MoveOperation.End)
        mouse.keyClicks(source, "!")
        assert wait_until(app, lambda: eg.editor.param("text") == "Hello Qt!", 10_000), eg.editor.param("text")

        # a template card dropped on V2 places the template there; not on an audio track
        canvas = win.timeline.canvas
        assert graphics_on(1) == []
        mime = template_mime(str(eg.browser.entries[0]["id"]))
        a1 = next(r for r in canvas.rows if r.kind == "audio")
        pos = QtCore.QPoint(canvas.width() - 60, a1.top + a1.height // 2)
        canvas.dragMoveEvent(QtGui.QDragMoveEvent(pos, Qt.DropAction.CopyAction, mime, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier))
        assert canvas._drop_preview is None
        v2 = next(r for r in canvas.rows if r.short == "V2")
        pos = QtCore.QPoint(canvas.width() - 60, v2.top + v2.height // 2)
        canvas.dragMoveEvent(QtGui.QDragMoveEvent(pos, Qt.DropAction.CopyAction, mime, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier))
        canvas.dropEvent(QtGui.QDropEvent(QtCore.QPointF(pos), Qt.DropAction.CopyAction, mime, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier))
        assert wait_until(app, lambda: len(graphics_on(1)) == 1, 20_000), win.sequence["video"][1]
    finally:
        win._quit_confirmed = True
        win.close()


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_text_to_speech(app, tmp_path: Path):
    """egui's Text to Speech panel: voices grouped Natural / Basic, the natural voices' download
    prompt, Add pause, Preview, Add to timeline, Save regenerates the selected narration, New, and
    selecting a narration clip loads it and brings the panel forward."""
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1600, 1000)
    win.show()
    engine.start()
    p = win.tts

    def narrations() -> list[dict]:
        return [c for t in (win.sequence or {}).get("audio") or [] for c in t.get("items") or [] if str(c.get("name") or "").startswith("Narration")]

    def history() -> list[str]:
        return [win.history.list.item(i).text() for i in range(win.history.list.count())]

    try:
        assert wait_until(app, lambda: win.sequence is not None and bool(p.catalog), 60_000)
        assert win.docks["Text to Speech"].isHidden(), "closed in the Editing workspace, as egui"
        win.docks["Text to Speech"].show()
        win.docks["Text to Speech"].raise_()
        model = p.voice_combo.model()
        rows = [model.item(i) for i in range(model.rowCount())]
        assert rows[0].text() == "Natural voices" and not rows[0].isEnabled()
        assert any(r.text() == "Basic voices (built in)" and not r.isEnabled() for r in rows)
        assert p.voice == "basic-female" and not p.download_box.isVisible()

        # a natural voice needs the one-time download first
        p.voice_combo.setCurrentIndex(p.voice_combo.findData("kokoro-heart"))
        p.voice_combo.activated.emit(p.voice_combo.currentIndex())
        assert p.download_box.isVisible() and not p.hear.isEnabled()
        p.download_button.click()
        assert p.download_ok.isVisible() and "SHA-256" in p.download_note.text()
        p.download_cancel.click()
        assert p.download_button.isVisible()
        p.voice_combo.setCurrentIndex(p.voice_combo.findData("basic-male"))
        p.voice_combo.activated.emit(p.voice_combo.currentIndex())
        assert not p.download_box.isVisible() and p.hear.isEnabled()

        # the script, Preview, Add to timeline
        assert not p.preview.isEnabled()
        p.text.setPlainText("Hello from the Qt panel.")
        p.add_pause.click()
        assert p.text.toPlainText() == "Hello from the Qt panel. [pause 1s] "
        p.text.insertPlainText("Goodbye.")
        assert p.preview.isEnabled() and p.save.text().endswith("Add to timeline")
        p.preview.click()
        assert wait_until(app, lambda: p.pending is None and bool(win.statusBar().currentMessage()) and p.preview.isEnabled(), 30_000)
        before = len(narrations())
        p.save.click()
        assert wait_until(app, lambda: len(narrations()) == before + 1 and p.loaded_from is not None, 30_000), win.statusBar().currentMessage()
        assert p.save.text().endswith("Save") and p.new_button.isVisible()
        clip = narrations()[-1]["clip"]

        # Save regenerates it in place with the new pace
        p.pace_slider.setValue(30)
        assert p.pace == 1.5
        p.save.click()
        assert wait_until(app, lambda: "Edit Narration" in history(), 30_000), history()[-3:]
        assert len(narrations()) == before + 1

        # New starts an empty script; selecting the clip loads it and brings the panel forward
        p.new_button.click()
        assert p.text.toPlainText() == "" and p.loaded_from is None
        win.docks["Text to Speech"].hide()
        win.execute("timeline.select", {"clips": [clip]})
        assert wait_until(app, lambda: p.loaded_from is not None and p.text.toPlainText().startswith("Hello from the Qt panel."), 20_000)
        assert wait_until(app, lambda: not win.docks["Text to Speech"].isHidden(), 5_000)
        assert abs(p.pace - 1.5) < 1e-6
    finally:
        win._quit_confirmed = True
        win.close()


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_properties_panel(app, tmp_path: Path):
    """egui's Properties panel: the selected clip's Transform, Crop and Audio values as hot numbers
    (a scrub is one undo step, a typed value its own), the keyframe navigator, a section's reset,
    Crop applied on the first edit, the Speed button, and a graphic clip's editor."""
    from cfa4_qt.hot_number import HotSpin
    from cfa4_qt.i18n import tr
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1600, 1000)
    win.show()
    engine.start()
    props = win.inspector
    mouse = QtTest.QTest
    Qt = QtCore.Qt

    def history() -> list[str]:
        return [win.history.list.item(i).text() for i in range(win.history.list.count())]

    def spin(name: str) -> HotSpin:
        s = props.findChild(HotSpin, name)
        assert s is not None, name
        return s

    def param(clip: dict | None, effect: str, pid: str):
        e = next((e for e in (clip or {}).get("effects") or [] if e.get("effect") == effect), None)
        p = next((p for p in (e or {}).get("params") or [] if p.get("id") == pid), None)
        return (p or {}).get("valueAtPlayhead")

    try:
        assert wait_until(app, lambda: win.sequence is not None and len(win.registry.commands) > 100, 60_000)
        win.docks["Properties"].raise_()
        assert wait_until(app, lambda: props.empty.isVisibleTo(props))
        video = next(c for t in win.sequence["video"] for c in t.get("items") or [])
        win.execute("timeline.select", {"clips": [video["clip"]]})
        win._seek_program(int(video["start"]) + (int(video["end"]) - int(video["start"])) // 2)
        assert wait_until(app, lambda: props.clip is not None and props.clip.get("clip") == video["clip"] and props.scroll.isVisibleTo(props), 20_000)
        assert props.title.text() == video.get("name")
        assert props.speed.text().startswith(tr("Speed {n}%").split("{")[0])
        for title in ("Transform", "Crop"):
            assert props.sections[title][0].isVisibleTo(props)

        # a Scale scrub (half a percent a pixel) is one undo step
        scale = spin("properties.motion.scale")
        props.scroll.ensureWidgetVisible(scale)
        app.processEvents()
        start = scale.value()
        line = scale.lineEdit()
        steps = len(history())
        y = line.height() // 2
        mouse.mousePress(line, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, QtCore.QPoint(10, y))
        for x in (15, 25, 35):
            mouse.mouseMove(line, QtCore.QPoint(x, y))
            wait_until(app, lambda: False, 30)
        mouse.mouseRelease(line, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier, QtCore.QPoint(35, y))
        assert wait_until(app, lambda: param(props.clip, "motion", "scale") in (start + 12, start + 13), 10_000), param(props.clip, "motion", "scale")
        assert wait_until(app, lambda: len(history()) == steps + 1, 10_000), history()[-4:]

        # Position X typed as a calculation: its own step, Y kept
        x = spin("properties.motion.position.x")
        y0 = spin("properties.motion.position.y").value()
        x.edited.emit(x.valueFromText("100*2"), True)
        assert wait_until(app, lambda: param(props.clip, "motion", "position") == [200.0, y0], 10_000), param(props.clip, "motion", "position")
        assert wait_until(app, lambda: len(history()) == steps + 2, 10_000), history()[-4:]

        # the keyframe navigator: the diamond turns animation on and adds a keyframe; another one
        # later; the previous-keyframe arrow goes back to it
        add = props.findChild(QtWidgets.QToolButton, "properties.opacity.opacity.addKeyframe")
        prev = props.findChild(QtWidgets.QToolButton, "properties.opacity.opacity.prevKeyframe")
        assert add.toolTip() == tr("Turn on animation and add keyframe") and not prev.isVisibleTo(props)
        add.click()
        assert wait_until(app, lambda: add.text() == "◆" and prev.isVisibleTo(props), 10_000)
        first = int(props.clip["playhead"])
        win._seek_program(first + 10 * frame_ticks(win.sequence.get("settings")))
        assert wait_until(app, lambda: add.text() == "◇" and prev.isEnabled() and add.toolTip() == tr("Add keyframe"), 10_000)
        prev.click()
        assert wait_until(app, lambda: int((props.clip or {}).get("playhead") or 0) == first and add.text() == "◆", 10_000)

        # the Transform reset puts Scale back
        props.findChild(QtWidgets.QToolButton, "properties.section.Transform.reset").click()
        assert wait_until(app, lambda: param(props.clip, "motion", "scale") == 100.0, 10_000)

        # Crop: the first value applies the effect, then the reset shows
        assert not props.sections["Crop"][3].isVisibleTo(props)
        spin("properties.crop.left").edited.emit(10.0, True)
        assert wait_until(app, lambda: param(props.clip, "crop", "left") == 10.0 and props.sections["Crop"][3].isVisibleTo(props), 10_000)

        # a section twirls closed and stays closed
        props.sections["Crop"][0].twirled.emit("Crop")
        assert not props.sections["Crop"][1].isVisibleTo(props)
        assert settings.value("properties/collapsed") in (["Crop"], "Crop")

        # an audio clip: Level and Pan
        audio = next(c for t in win.sequence["audio"] for c in t.get("items") or [])
        win.execute("timeline.select", {"clips": [audio["clip"]]})
        assert wait_until(app, lambda: props.audio_clip is not None and props.audio_clip.get("clip") == audio["clip"], 20_000)
        assert props.sections["Audio"][0].isVisibleTo(props)
        with_video = any(c[1] == "video" for c in props.selected())  # a linked clip selects both
        assert props.sections["Transform"][0].isVisibleTo(props) == with_video
        spin("properties.volume.level").edited.emit(-6.0, True)
        assert wait_until(app, lambda: param(props.audio_clip, "volume", "level") == -6.0, 10_000), param(props.audio_clip, "volume", "level")

        # the Speed button opens Speed/Duration
        opened: list[str] = []
        win.invoke = lambda cid: opened.append(cid)  # type: ignore[method-assign]
        props.invoke.disconnect()
        props.invoke.connect(win.invoke)
        props.speed.click()
        assert opened == ["clip.speedDuration"]

        # a graphic clip shows the graphic editor
        win.execute("graphics.newText", {"text": "Props"})
        assert wait_until(app, lambda: props.graphic.isVisibleTo(props) and props.graphic.editor.param("text") == "Props", 20_000)
        assert not props.scroll.isVisibleTo(props) and not props.graphic.tabs.isVisibleTo(props)
    finally:
        win._quit_confirmed = True
        win.close()


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_effects_panel(app, tmp_path: Path):
    """egui's Effects panel: Premiere's top folders, sub-folders, badges, Presets and Lumetri
    Presets with engine-rendered thumbnails (and the wide panel's grid), search, double-click
    applies to the selection, a row dropped on a Timeline clip applies to that clip."""
    from cfa4_qt.main_window import MainWindow
    from cfa4_qt.panels.effects import ROLE_BADGES, ROLE_ID, effect_mime

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1600, 1000)
    win.show()
    engine.start()
    fx = win.effects
    Qt = QtCore.Qt

    def history() -> list[str]:
        return [win.history.list.item(i).text() for i in range(win.history.list.count())]

    def tops() -> list[str]:
        return [str(fx.tree.topLevelItem(i).data(0, ROLE_ID)) for i in range(fx.tree.topLevelItemCount())]

    def find(eid: str):
        it = QtWidgets.QTreeWidgetItemIterator(fx.tree)
        while it.value():
            if it.value().data(0, ROLE_ID) == eid:
                return it.value()
            it += 1
        return None

    try:
        assert wait_until(app, lambda: win.sequence is not None and len(win.registry.commands) > 100, 60_000)
        win.docks["Effects"].raise_()
        assert wait_until(app, lambda: "Lumetri Presets" in tops() and "Presets" in tops(), 20_000), tops()
        assert tops() == ["Presets", "Lumetri Presets", "Audio Effects", "Audio Transitions", "Video Effects", "Video Transitions", "Legacy"]
        # sub-folders, and the loose audio effects (Volume) after them
        audio = fx.tree.topLevelItem(2)
        kinds = [audio.child(i).data(0, QtCore.Qt.ItemDataRole.UserRole + 1) for i in range(audio.childCount())]
        assert "folder" in kinds and kinds[-1] != "folder" and kinds.index("folder") < len(kinds) - 1
        # badges and search
        fx.search.setText("gaussian blur")
        blur = find("gaussian_blur")
        assert blur is not None and blur.parent().isExpanded() and not blur.isHidden()
        assert blur.data(0, ROLE_BADGES), "Gaussian Blur carries its badges"
        assert find("crop") is None
        fx.search.clear()

        # a Lumetri preset folder: rows get engine thumbnails; the wide panel shows its grid
        lumetri = fx.tree.topLevelItem(1)
        lumetri.setExpanded(True)
        folder = lumetri.child(0)
        folder.setExpanded(True)
        row = folder.child(0)
        name = str(row.data(0, ROLE_ID))[len("lumetri:"):]
        assert wait_until(app, lambda: not find(f"lumetri:{name}").icon(0).isNull(), 20_000)
        win.docks["Effects"].setFloating(True)
        win.docks["Effects"].resize(900, 500)
        assert wait_until(app, lambda: fx.grid.isVisible() and fx.grid.count() == folder.childCount(), 10_000)
        assert wait_until(app, lambda: not fx.grid.item(0).icon().isNull(), 20_000)
        assert settings.value("effects/expanded") is not None

        # double-click a Lumetri preset with a clip selected: applied
        video = next(c for t in win.sequence["video"] for c in t.get("items") or [])
        win.execute("timeline.select", {"clips": [video["clip"]]})
        assert wait_until(app, lambda: video["clip"] in ((win.sequence or {}).get("selection") or []), 10_000)
        steps = len(history())
        fx.tree.itemDoubleClicked.emit(find(f"lumetri:{name}"), 0)
        assert wait_until(app, lambda: len(history()) == steps + 1 and name in history()[-1], 10_000), history()[-3:]

        # an effect dropped on a clip goes on that clip and selects it
        canvas = win.timeline.canvas
        row0 = next(r for r in canvas.rows if r.kind == "video" and r.track.get("items"))
        clip = row0.track["items"][0]
        r = canvas.clip_rect(row0, clip)
        pos = QtCore.QPoint(int(r.center().x()), int(r.center().y()))
        mime = effect_mime("gaussian_blur")
        canvas.dragMoveEvent(QtGui.QDragMoveEvent(pos, Qt.DropAction.CopyAction, mime, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier))
        assert canvas._effect_target is not None and canvas._effect_target[2] == ""
        canvas.dropEvent(QtGui.QDropEvent(QtCore.QPointF(pos), Qt.DropAction.CopyAction, mime, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier))

        def has_blur() -> bool:
            c = next((c for t in win.sequence["video"] for c in t.get("items") or [] if c.get("clip") == clip["clip"]), {})
            return any(e.get("effect") == "gaussian_blur" for e in c.get("effects") or [])

        assert wait_until(app, has_blur, 10_000)
        assert wait_until(app, lambda: win.sequence.get("selection") == [clip["clip"]] or clip["clip"] in win.sequence.get("selection"), 10_000)
        # a transition dragged near a clip's start marks that edge
        cross = next(e["id"] for e in fx.effects if e.get("kind") == "VideoTransition")
        near_in = QtCore.QPoint(int(r.left() + 4), int(r.center().y()))
        tmime = effect_mime(cross)  # kept: the event doesn't own its mime data
        canvas.dragMoveEvent(QtGui.QDragMoveEvent(near_in, Qt.DropAction.CopyAction, tmime, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier))
        assert canvas._effect_target is not None and canvas._effect_target[2] == "in"
        canvas.dragLeaveEvent(None)
    finally:
        win._quit_confirmed = True
        win.close()


def test_malformed_saved_panel_state_does_not_stop_the_window(app, fake_engine: Path, tmp_path: Path):
    """Saved panel state is untrusted (AGENTS.md): valid JSON of the wrong shapes falls back to
    the defaults instead of raising while the window is built."""
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    settings.setValue("panels/scopes", json.dumps({"waveform_type": [], "shown": {"a": 1}, "scale": {}}))
    settings.setValue("panels/timecode", json.dumps({"rows": [{"source": []}, {"mode": {}}]}))
    settings.setValue("panels/reference", json.dumps({"time": "invalid", "scopes": 1, "display": []}))
    settings.sync()
    win = MainWindow(EngineClient(fake_engine), settings)
    try:
        assert win.timecode_panel.rows == [{"source": "active", "mode": "current", "display": "Timecode"}]
        assert win.reference.time == 0
        settings.setValue("panels/timecode", json.dumps({"rows": 1}))
        settings.setValue("panels/reference", json.dumps({"time": float("nan")}))
        win.timecode_panel.attach_settings(settings)
        win.reference.attach_settings(settings)
        assert win.reference.time == 0
        from cfa4_qt.panels.monitor_view import MonitorView

        for pan in ([None, 0], ["invalid", 0], [1], [float("inf"), 0], [True, 0], 3):
            settings.setValue("view", json.dumps({"pan": pan}))
            assert MonitorView.load(settings, "view").pan == [0.0, 0.0]
        settings.setValue("view", json.dumps({"pan": [2, -1.5]}))
        assert MonitorView.load(settings, "view").pan == [2.0, -1.5]
    finally:
        win._quit_confirmed = True
        win.close()


def test_lumetri_curve_double_click_keeps_the_end_points(app):
    """An added curve point is stored after the two end points: double-click removes it, and
    double-clicking an end point (by position, not by list index) does not."""
    from cfa4_qt.panels.lumetri import CurveEditor

    ed = CurveEditor("curve_luma", False, QtGui.QColor("white"))
    ed.resize(240, 240)
    ed.show()
    sent: list = []
    ed.changed.connect(lambda pid, pts, imm: sent.append(pts))
    ed.points = [[0.0, 0.0], [1.0, 1.0], [0.5, 0.75]]
    end = ed.to_screen([1.0, 1.0]).toPoint()
    QtTest.QTest.mouseDClick(ed, QtCore.Qt.MouseButton.LeftButton, QtCore.Qt.KeyboardModifier.NoModifier, end)
    assert ed.points == [[0.0, 0.0], [1.0, 1.0], [0.5, 0.75]]
    mid = ed.to_screen([0.5, 0.75]).toPoint()
    QtTest.QTest.mouseDClick(ed, QtCore.Qt.MouseButton.LeftButton, QtCore.Qt.KeyboardModifier.NoModifier, mid)
    assert ed.points == [[0.0, 0.0], [1.0, 1.0]]
    ed.close()


def test_caption_colours_and_transcript_word_follow_the_engine_formats():
    """Caption colours come as ``#rrggbbaa`` (QColor reads eight digits as ``#aarrggbb``); the
    word under the shown playhead is found as the engine's ``word_at`` finds it."""
    from cfa4_qt.panels.text import rgba_color, word_at

    assert rgba_color("#ff0000ff").getRgb()[:3] == (255, 0, 0)
    assert rgba_color("#00ff00").getRgb()[:3] == (0, 255, 0)
    assert rgba_color("nonsense").getRgb()[:3] == (255, 255, 255)
    words = [{"i": 0, "start": 0, "end": 10}, {"i": 1, "start": 10, "end": 20}, {"i": 2, "start": 30, "end": 40}]
    assert [word_at(words, t) for t in (-1, 0, 9, 10, 25, 35, 40)] == [None, 0, 0, 1, None, 2, None]


def test_long_waveforms_come_in_pieces_of_one_density(app):
    """A clip longer than one ``media.peaks`` request: its pieces share one density, so the
    joined peaks spread evenly over the whole clip."""
    from cfa4_qt.panels.timeline_media import MAX_BUCKETS, MAX_PEAK_SECONDS, TimelineMedia

    asked: list[dict] = []

    class Engine:
        running = True

        def call(self, _method, params, on_reply, on_error):
            asked.append(params)
            on_reply({"buckets": params["buckets"]}, struct.pack(f"<{params['buckets'] * 2}f", *([0.5] * params["buckets"] * 2)))

    media = TimelineMedia()
    media.attach(Engine())
    span = MAX_PEAK_SECONDS * TICKS_PER_SECOND
    peaks = None
    for _ in range(5):
        peaks = media.waveform(7, 0, 2 * span + span // 2)
        if peaks is not None:
            break
    assert peaks is not None
    assert [p["buckets"] for p in asked] == [MAX_BUCKETS, MAX_BUCKETS, MAX_BUCKETS // 2]
    assert len(peaks) == MAX_BUCKETS * 5 // 2

    # More ranges on screen than the cache keeps: what one paint shows is never evicted, so
    # repainting after each reply does not ask for the same ranges again and again.
    from cfa4_qt.panels.timeline_media import MAX_PEAKS

    asked.clear()
    media = TimelineMedia()
    media.attach(Engine())
    ranges = [(i * TICKS_PER_SECOND, (i + 1) * TICKS_PER_SECOND) for i in range(MAX_PEAKS + 50)]
    for _ in range(6):
        media.begin_paint()
        shown = [media.waveform(8, a, b) for a, b in ranges]
    assert all(w is not None for w in shown)
    assert len(asked) == len(ranges)


def test_program_playback_runs_a_mixer_automation_pass(app, fake_engine: Path, tmp_path: Path):
    """egui's: playing the Program forward at normal speed starts an automation pass and stopping
    writes it, at the shown times; the Reference Monitor's frames are told apart by sequence."""
    from cfa4_qt.main_window import MainWindow

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(fake_engine)
    win = MainWindow(engine, settings)
    engine.start()
    try:
        assert wait_until(app, lambda: win.monitor.has_content())
        sent: list = []
        win.monitor.command.connect(lambda c, p: sent.append((c, p)))
        win.monitor.play(1.0)
        assert sent[-1][0] == "mixer.recordStart"
        wait_until(app, lambda: False, 100)
        win.monitor.stop()
        stops = [p for c, p in sent if c == "mixer.recordStop"]
        assert len(stops) == 1 and stops[0]["time"] >= sent[0][1]["time"]
        win.monitor.play(2.0)  # fast: no pass
        win.monitor.stop()
        assert [c for c, _p in sent].count("mixer.recordStart") == 1
        calls: list = []
        real_call = engine.call
        engine.call = lambda method, *a, **k: (calls.append(method), real_call(method, *a, **k))[1]  # type: ignore[method-assign]
        win.monitor.seek(win.monitor.frame() * 20)
        win.monitor.play(-1.0)  # shuttle: silent, so no audio is decoded to catch up
        win.monitor._feed_audio()
        win.monitor.stop()
        engine.call = real_call  # type: ignore[method-assign]
        assert "audio.render" not in calls
        media = win.timeline.canvas.media
        media.peaks[(1, 0, 10, 100, 32, 0)] = [(0.0, 0.5)]
        win._command_done("timeline.select")
        assert media.peaks
        win._command_done("media.replaceFootage")  # same item id, other media
        assert not media.peaks
        win.reference.set_sequence({"id": 1}, 5)
        first = win.reference.revision
        win.reference.set_sequence({"id": 2}, 5)
        assert win.reference.revision != first
    finally:
        win._quit_confirmed = True
        win.close()


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_dropping_several_items_places_them_one_after_another(app, tmp_path: Path):
    """Several Project items dropped on the Timeline go one after another as one undo step,
    not each at the drop time (where each would overwrite the last)."""
    from cfa4_qt.main_window import MainWindow
    from cfa4_qt.panels.timeline import ITEM_MIME

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1600, 1000)
    win.show()
    engine.start()
    Qt = QtCore.Qt
    try:
        assert wait_until(app, lambda: win.sequence is not None and len(win.registry.commands) > 100, 60_000)
        canvas = win.timeline.canvas
        clips = [c for t in win.sequence["video"] for c in t.get("items") or []]
        items = list(dict.fromkeys(c["item"] for c in clips))[:2]
        assert len(items) == 2
        end = max(int(c["end"]) for t in win.sequence["video"] for c in t.get("items") or [])
        v1 = next(r for r in canvas.rows if r.kind == "video")
        canvas.scroll_ticks = max(0, end - 2 * TICKS_PER_SECOND)
        x = int(canvas.x_of(end + TICKS_PER_SECOND))
        pos = QtCore.QPoint(x, v1.top + v1.height // 2)
        mime = QtCore.QMimeData()
        mime.setData(ITEM_MIME, ",".join(str(i) for i in items).encode())
        before = len(clips)
        steps = win.history.list.count()
        canvas.dragMoveEvent(QtGui.QDragMoveEvent(pos, Qt.DropAction.CopyAction, mime, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier))
        canvas.dropEvent(QtGui.QDropEvent(QtCore.QPointF(pos), Qt.DropAction.CopyAction, mime, Qt.MouseButton.LeftButton, Qt.KeyboardModifier.NoModifier))

        def placed() -> list[dict]:
            track = next(t for t in win.sequence["video"] if t.get("id") == v1.track_id)
            return sorted((c for c in track.get("items") or [] if int(c["start"]) > end), key=lambda c: int(c["start"]))

        assert wait_until(app, lambda: len(placed()) == 2, 10_000), placed()
        a, b = placed()
        assert int(b["start"]) == int(a["end"])
        assert wait_until(app, lambda: win.history.list.count() == steps + 1, 10_000)
        assert len([c for t in win.sequence["video"] for c in t.get("items") or []]) == before + 2
    finally:
        win._quit_confirmed = True
        win.close()


def test_meters_keep_responding_after_playback_loops(app):
    """A loop restarts the played blocks from the in point: the meters follow the new pass
    instead of waiting for it to pass the previous pass's end."""
    from collections import deque
    from types import SimpleNamespace

    from cfa4_qt.panels.meters import FLOOR, AudioMetersPanel

    meters = AudioMetersPanel()
    mon = SimpleNamespace(playing=True, audio_generation=1, now=48000)
    mon.play_sample = lambda: mon.now
    mon.meter_blocks = deque([(0, 4800, [[0.5, 0.5]] * 10, [])])
    meters.watch(mon)
    meters.tick()
    assert meters.levels[0] > FLOOR
    meters.levels = [FLOOR, FLOOR]
    mon.audio_generation, mon.now = 2, 4800  # looped back to the start
    mon.meter_blocks = deque([(0, 4800, [[0.5, 0.5]], [])])
    meters.tick()
    assert meters.levels[0] > FLOOR


def test_drop_frame_timecode_is_shown_as_the_engine_parses_it():
    """A 29.97 drop-frame sequence shows HH;MM;SS;FF counted as the engine counts it, so the
    shown value typed back into the monitor stays on the same frame."""
    from cfa4_qt.timecode import drop_frame, format_timecode, frame_ticks

    df = {"frame_rate": {"num": 30000, "den": 1001}, "drop_frame": True}
    ft = frame_ticks(df)
    assert drop_frame(df) and not drop_frame({**df, "drop_frame": False})
    assert not drop_frame({"frame_rate": {"num": 25, "den": 1}, "drop_frame": True})
    assert format_timecode(107_892 * ft, ft, True) == "01;00;00;00"
    assert format_timecode(1_800 * ft, ft, True) == "00;01;00;02"
    assert format_timecode(17_982 * ft, ft, True) == "00;10;00;00"
    assert format_timecode(1_800 * ft, ft) == "00:01:00:00"


def test_scopes_take_turns_when_frames_come_faster_than_replies(app):
    """Several scopes during playback: a newer frame arriving while one scope renders must not
    start over with the first scope, or the others never show."""
    from cfa4_qt.panels.scopes import ScopesView

    calls: list = []

    class Engine:
        running = True

        def call(self, _method, params, on_reply, on_error):
            calls.append((params["kind"], on_reply))

    view = ScopesView(Engine())
    view.st.shown = ["vectorscope", "waveform"]
    view.resize(400, 300)
    view.show()
    for t in range(6):
        view.set_frame(t, 1)  # a newer frame while one scope is rendering
        _kind, reply = calls[t]  # the oldest request still waiting is answered
        reply({"w": 1, "h": 1}, b"\0\0\0\0")
    kinds = [k for k, _r in calls]
    assert {"vectorscope", "waveform"} <= set(kinds[2:]), kinds
