"""CFA4 Qt 6 GUI Events panel against the real Rust engine (when it is built), as egui's
``panels/events.rs``."""

from __future__ import annotations

import os
from pathlib import Path

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


def test_ago_as_egui():
    from cfa4_qt.panels.events import ago

    now = 10_000_000
    assert [ago(now - ms, now) for ms in (0, 4_999, 5_000, 59_999, 60_000, 3_599_999, 3_600_000)] == [
        "just now", "just now", "5 s ago", "59 s ago", "1 min ago", "59 min ago", "1 h ago"]
    assert ago(now + 5_000, now) == "just now", "a clock that went back is not negative"


def test_parse_entries_leaves_out_bad_ones():
    from cfa4_qt.panels.events import parse_entries

    good = {"id": 3, "level": "error", "source": "clip.rename", "message": "need a name", "count": 2, "timeMs": 7}
    got = parse_entries({"entries": [good, {"id": "x", "level": "error"}, {"id": 4, "level": "loud"}, "junk"]})
    assert [(e.id, e.level, e.count, e.time_ms) for e in got] == [(3, "error", 2, 7)]
    assert parse_entries(None) == [] and parse_entries({"entries": None}) == []


@pytest.mark.skipif(REAL is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_events_panel_as_egui(app, tmp_path: Path):
    """The engine's event log, newest first, with the GUI's own messages; a failed command shows
    once (the engine and the GUI both report it); repeats count up; the filter, the counts, the
    selected entry's details and Clear All as egui's."""
    from cfa4_qt.main_window import MainWindow
    from cfa4_qt.panels.events import LOCAL_IDS

    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = EngineClient(REAL, ["--demo", "--data-dir", str(tmp_path / "data")])
    win = MainWindow(engine, settings)
    win.resize(1400, 900)
    win.show()
    engine.start()
    ev = win.events
    win.docks["Events"].show()
    win.docks["Events"].raise_()

    def rows() -> list[tuple[str, int]]:
        return [(ev.list.topLevelItem(i).text(0), ev.list.topLevelItem(i).data(0, QtCore.Qt.ItemDataRole.UserRole)) for i in range(ev.list.topLevelItemCount())]

    def counts() -> tuple[str, str, str]:
        return (ev.counts["error"].text(), ev.counts["warning"].text(), ev.counts["info"].text())

    try:
        assert wait_until(app, lambda: win.sequence is not None and len(win.registry.commands) > 100, 60_000)
        assert [ev.filter.itemText(i) for i in range(3)] == ["All Events", "Warnings and Errors", "Errors Only"]
        ev.clear_button.click()
        assert wait_until(app, lambda: not rows() and counts() == ("0", "0", "0") and ev.empty.isVisible(), 10_000)
        # refreshes and a selected clip (not a narration) log nothing: the Text to Speech panel no
        # longer asks tts.inspect without a narration clip (it failed on every refresh)
        clip = next(it for t in win.sequence["video"] for it in t["items"])
        win.execute("timeline.select", {"clips": [clip["clip"]]})
        assert wait_until(app, lambda: clip["clip"] in ((win.sequence or {}).get("selection") or []), 10_000)
        for _ in range(3):
            win.schedule_refresh()
            wait_until(app, lambda: False, 150)
        ev.refresh()
        wait_until(app, lambda: False, 300)
        assert not rows(), rows()
        # a failed command: the engine logs it and the GUI reports it, shown once
        win.execute("clip.rename", {})
        assert wait_until(app, lambda: len(rows()) == 1 and rows()[0][1] < LOCAL_IDS and counts() == ("1", "0", "0"), 10_000), (rows(), ev.local)
        assert ev.local, "the GUI reported it too"
        err_id = rows()[0][1]
        win.execute("clip.rename", {})
        assert wait_until(app, lambda: len(rows()) == 1 and rows()[0][0].endswith("(×2)"), 10_000), rows()
        # a disabled command is a warning; the GUI's own message is info, newest first
        done: list = []
        engine.execute("edit.redo", {}, on_reply=lambda r, _b: done.append(r), on_error=lambda m: done.append(m))
        assert wait_until(app, lambda: done, 10_000)
        ev.refresh()
        assert wait_until(app, lambda: counts() == ("1", "1", "0"), 10_000), counts()
        ev.log("Hello from the GUI")
        assert wait_until(app, lambda: counts() == ("1", "1", "1") and rows()[0][0] == "Hello from the GUI", 10_000), rows()
        assert ev.text.toPlainText().endswith("Hello from the GUI")
        # the filter: Errors Only, then Warnings and Errors
        ev.filter.setCurrentIndex(2)
        ev.filter.activated.emit(2)
        assert ev.level == "error" and len(rows()) == 1 and rows()[0][1] == err_id
        ev.set_level("warning")
        assert len(rows()) == 2 and ev.filter.currentData() == "warning"
        ev.set_level("info")
        assert len(rows()) == 3
        # a click selects the entry and shows its details; a second click deselects
        item = next(ev.list.topLevelItem(i) for i in range(3) if ev.list.topLevelItem(i).data(0, QtCore.Qt.ItemDataRole.UserRole) == err_id)
        ev.list.itemClicked.emit(item, 0)
        assert wait_until(app, lambda: ev.selected == err_id and ev.details.isVisible(), 10_000)
        assert ev.details.text().startswith("Error · clip.rename · just now\n")
        assert ev.list.currentItem().data(0, QtCore.Qt.ItemDataRole.UserRole) == err_id
        item = next(ev.list.topLevelItem(i) for i in range(3) if ev.list.topLevelItem(i).data(0, QtCore.Qt.ItemDataRole.UserRole) == err_id)
        ev.list.itemClicked.emit(item, 0)
        assert wait_until(app, lambda: ev.selected is None and ev.details.isHidden(), 10_000)
        # the state as egui's ui.panels.events
        ev.set_state({"level": "error", "selected": err_id})
        assert ev.state() == {"level": "error", "selected": err_id} and ev.details.isVisible()
        # Clear All: the engine's log and the GUI's messages
        ev.clear_button.click()
        assert wait_until(app, lambda: not rows() and counts() == ("0", "0", "0") and not ev.local and ev.details.isHidden(), 10_000)
        listed: list = []
        engine.execute("events.list", {}, on_reply=lambda r, _b: listed.append(r))
        assert wait_until(app, lambda: listed, 10_000) and listed[0]["entries"] == []
    finally:
        win._quit_confirmed = True
        win.close()
        engine.stop()
