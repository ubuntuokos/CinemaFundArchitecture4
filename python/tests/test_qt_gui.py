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
            send({"id": rid, "ok": True, "result": {"dirty": False, "revision": state["revision"]}})
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
    """Regression: an older filmcraft-cli on PATH (no `serve`) was picked and exited with 2."""
    from cfa4_qt import engine as engine_mod

    old = _script(tmp_path / "bin" / "filmcraft-cli", "import sys\nprint('USAGE\\n  exec <id>')\n")
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
    build = root / "target" / "release" / "filmcraft-cli"
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


# ----- the real engine, when built -----
REAL = locate_engine()


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p filmcraft-cli)")
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


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p filmcraft-cli)")
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


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p filmcraft-cli)")
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


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p filmcraft-cli)")
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
    logo, not an inherited FilmCraft/ArtCraft one."""
    from cfa4_qt import branding
    from cfa4_qt.main_window import MainWindow

    assert branding.LOGO_SVG.is_file() and "cfa4" in branding.LOGO_SVG.name
    assert not branding.app_icon().isNull()
    for path in (ROOT / "python" / "cfa4_qt").rglob("*.py"):
        text = path.read_text(encoding="utf-8").lower()
        assert "ai.storyteller" not in text and "app-icon" not in text, path

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
