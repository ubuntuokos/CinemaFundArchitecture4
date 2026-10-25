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


def test_recovery_prompt_reads_the_engines_camel_case_keys():
    """Regression: the prompt read snake_case keys, so every candidate was 'Untitled' from 1970."""
    from cfa4_qt.main_window import recovery_text

    text = recovery_text({"id": "s1", "savedUnix": 1_790_000_000, "projectName": "Interjú", "cleanExit": True})
    assert "Interjú" in text and "1970" not in text and "was closed" in text
    crashed = recovery_text({"savedUnix": 1_790_000_000, "projectName": "B", "cleanExit": False})
    assert "quit unexpectedly" in crashed
    assert "unknown time" in recovery_text({"savedUnix": "junk"})
    assert "unknown time" in recovery_text({"savedUnix": 10**30})


def test_panel_only_shortcuts_are_not_bound_window_wide(app):
    """Regression: History ▸ Left (Undo) was bound window-wide, so Left undid instead of stepping."""
    win = QtWidgets.QMainWindow()
    reg = CommandRegistry(lambda c: None)
    reg.load(
        [
            {"id": "edit.undo", "label": "Undo", "menu": ["Edit"], "shortcut": "Cmd+Z", "shortcuts": ["Cmd+Z", "Left"],
             "panelShortcuts": [{"keys": "Left", "panel": "History"}], "params": "{}", "enabled": True},
            {"id": "playhead.stepBack", "label": "Step Back", "menu": [], "shortcut": "Left", "shortcuts": ["Left"],
             "params": "{}", "enabled": True},
        ]
    )
    assert reg.commands["edit.undo"].shortcuts == ["Cmd+Z"]
    reg.build_menus(win.menuBar(), win)
    left = QtGui.QKeySequence("Left")
    assert left in reg.actions["playhead.stepBack"].shortcuts()
    assert left not in reg.actions["edit.undo"].shortcuts()


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


# Source/Effect Controls, Project/Effects, Properties/History, Timeline/Events
TAB_GROUPS = 4


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

        # a drag of one value (several changes in a row) is one undo step
        steps = len(history())
        for v in (110.0, 120.0, 135.0):
            scale.spins[0].setValue(v)
            app.processEvents()
        assert wait_until(app, lambda: param("motion", "scale")["value"] == 135.0, 10_000), param("motion", "scale")
        assert wait_until(app, lambda: len(history()) == steps + 1, 10_000), history()

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
        have = {"Project", "Effects", "History", "Source", "Effect Controls", "Program", "Timeline", "Properties", "Events", "Tools"}
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
