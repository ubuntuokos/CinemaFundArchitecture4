"""CFA4 Qt 6 GUI: protocol framing, registry helpers, i18n, and the main window against a fake
engine (always) and the real Rust engine (when it is built)."""

from __future__ import annotations

import io
import json
import os
import stat
import sys
import textwrap
from pathlib import Path

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
QtCore = pytest.importorskip("PySide6.QtCore")
QtWidgets = pytest.importorskip("PySide6.QtWidgets")
QtGui = pytest.importorskip("PySide6.QtGui")

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
    finally:
        win._quit_confirmed = True
        win.close()


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
    titles = {"Source", "Go to Start", "Go to End", "Step Back 1 Frame", "Step Forward 1 Frame", "Mark In", "Mark Out", "Insert", "Overwrite", "Play/Stop", "Project", "Effects", "Inspector", "History", "Timeline", "Events", "Selection Tool", "Razor Tool", "New Bin", "New Sequence…", "Import…"}
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


def test_layout_has_no_orphaned_tab_bars(app, fake_engine: Path, tmp_path: Path):
    """Regression: tabifying docks before show() left a stale, visible copy of a tab bar drawn
    over the panels. Every visible tab bar must belong to exactly one tab group."""
    from cfa4_qt.main_window import MainWindow

    def bars(win) -> list[tuple[str, ...]]:
        return [
            tuple(tb.tabText(i) for i in range(tb.count())) for tb in win.findChildren(QtWidgets.QTabBar) if tb.isVisible()
        ]

    for _ in range(2):  # first start (default layout), then with the saved layout
        settings = QtCore.QSettings(str(tmp_path / "layout.ini"), QtCore.QSettings.Format.IniFormat)
        win = MainWindow(EngineClient(fake_engine), settings)
        win.resize(1440, 900)
        win.show()
        assert wait_until(app, lambda w=win: len(bars(w)) >= 3, 5_000)
        wait_until(app, lambda: False, 300)
        found = bars(win)
        assert len(found) == len(set(found)) == 3, found
        win.reset_layout.trigger()
        wait_until(app, lambda: False, 300)
        found = bars(win)
        assert len(found) == len(set(found)) == 3, found
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
    assert exec_line.endswith(" %F") and "%" not in exec_line[:-3]
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
    monkeypatch.setenv("CFA4_DEV_DESKTOP_ENTRY", "1")
    assert de.requested_by_env()


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
