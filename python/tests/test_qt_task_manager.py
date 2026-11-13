"""CFA4 Qt 6 Task Manager panel (ACS-5): background jobs, exports and the shared task queue in one
place, with why a task waits before it starts, time left while it runs and cancel."""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
QtCore = pytest.importorskip("PySide6.QtCore")
QtWidgets = pytest.importorskip("PySide6.QtWidgets")

from cfa4_qt.panels import tasks as tm  # noqa: E402

GIB = 1 << 30


@pytest.fixture(scope="module")
def app():
    yield QtWidgets.QApplication.instance() or QtWidgets.QApplication([])


class FakeEngine:
    running = True

    def __init__(self) -> None:
        self.pending: list[tuple[str, dict[str, Any], Any, Any]] = []

    def execute(self, command, params=None, on_reply=None, on_error=None):
        self.pending.append((command, params or {}, on_reply, on_error))
        return len(self.pending)

    def take(self, command):
        for i, (c, _p, ok, err) in enumerate(self.pending):
            if c == command:
                del self.pending[i]
                return ok, err
        raise AssertionError(f"no pending {command}")


JOBS = [
    {"id": 1, "label": "Proxy", "progress": 0.5, "etaSeconds": 75, "status": "", "finished": False},
    {"id": 2, "label": "Scene detection", "progress": 0, "status": "Waiting: not enough memory (needs 2.0 GiB)", "finished": False},
    {"id": 3, "label": "Transcode", "finished": True, "result": {"error": "cancelled"}},
]
EXPORTS = {"items": [
    {"id": 7, "sequenceName": "Main", "status": "encoding", "etaSeconds": 3700},
    {"id": 8, "sequenceName": "Trailer", "status": "ready", "waitingFor": ["VRAM: needs 6.0 GiB"]},
    {"id": 9, "sequenceName": "Old", "status": "done"},
]}
TASKS = [
    {"id": "export-1", "state": "running", "class": "finalRender", "ramBytes": 4 * GIB, "cpuMilli": 4000, "waitingFor": []},
    {"id": "job-2", "state": "waiting", "class": "background", "ramBytes": 2 * GIB, "cpuMilli": 1000, "waitingFor": ["not enough memory"]},
]


def test_rows_from_hostile_replies_never_raise():
    for bad in (None, 3, "x", [None, 1, "a"], {"items": "no"}, [{"progress": "nan", "etaSeconds": float("inf")}]):
        tm.job_rows(bad)
        tm.export_rows(bad)
        tm.task_rows(bad)
    row = tm.job_rows([{"id": 1, "progress": 1e300, "etaSeconds": float("nan")}])[0]
    assert (row["state"], row["eta"]) == ("running", "")
    assert len(tm.job_rows([{"id": i} for i in range(tm.MAX_ROWS + 50)])) == tm.MAX_ROWS


def test_rows_say_why_a_task_waits_and_how_long_one_runs():
    jobs = tm.job_rows(JOBS)
    assert [(r["state"], r["eta"]) for r in jobs] == [("running", "1:15"), ("waiting", ""), ("cancelled", "")]
    assert jobs[1]["detail"] == "not enough memory (needs 2.0 GiB)"
    assert jobs[0]["action"] == ("jobs.cancel", {"job": 1}) and jobs[2]["action"] is None
    exports = tm.export_rows(EXPORTS)
    assert [(r["state"], r["eta"]) for r in exports] == [("encoding", "1:01:40"), ("waiting", ""), ("done", "")]
    assert exports[1]["detail"] == "VRAM: needs 6.0 GiB" and exports[1]["action"] == ("export.queue.cancel", {"id": 8})
    queue = tm.task_rows(TASKS)
    assert queue[0]["detail"] == "Final render · 4.0 GiB · 4 CPU" and queue[1]["detail"] == "not enough memory"
    text = tm.summary(jobs + exports)
    assert text.startswith("2 running · 2 waiting") and "Scene detection waits: not enough memory" in text


def test_panel_shows_three_sources_cancels_and_survives_an_engine_without_the_queue(app, tmp_path: Path):
    settings = QtCore.QSettings(str(tmp_path / "s.ini"), QtCore.QSettings.Format.IniFormat)
    engine = FakeEngine()
    panel = tm.TaskManagerPanel(engine)
    panel.attach_settings(settings)
    sent: list[tuple[str, dict]] = []
    panel.command.connect(lambda c, p: sent.append((c, p)))
    panel.resize(800, 400)
    panel.show()
    panel.watch(True)
    assert panel.poll.isActive()
    panel.refresh()  # a second poll while the first is unanswered asks nothing more
    assert sorted(c for c, *_ in engine.pending) == ["export.queue.list", "jobs.list", "tasks.list"]
    engine.take("jobs.list")[0](JOBS, None)
    engine.take("export.queue.list")[0](EXPORTS, None)
    engine.take("tasks.list")[1]("unknown command `tasks.list`")
    jobs, exports, queue = (panel.groups[k] for k in ("jobs", "exports", "queue"))
    assert (jobs.childCount(), exports.childCount()) == (2, 2), "finished ones hidden"
    assert jobs.child(1).text(1) == "Waiting for resources" and "memory" in jobs.child(1).text(2)
    assert queue.childCount() == 1 and queue.child(0).text(0) == "Not reported by this engine yet."
    assert "waiting" in panel.summary.text()
    ids = [p[0] for p in panel.tree.automation_parts()]
    assert "taskManager.job.2" in ids and "taskManager.export.8" in ids
    cancel = panel.findChild(QtWidgets.QToolButton, "taskManager.export.8.cancel")
    assert cancel is not None
    cancel.click()
    assert sent == [("export.queue.cancel", {"id": 8})]

    panel.finished.setChecked(True)
    assert (jobs.childCount(), exports.childCount()) == (3, 3)
    assert settings.value("panels/taskManager/showFinished") in (True, "true")
    assert panel.findChild(QtWidgets.QToolButton, "taskManager.job.3.cancel") is None, "a finished job has no cancel"

    panel.refresh()
    engine.take("tasks.list")[0](TASKS, None)
    assert queue.childCount() == 2 and queue.child(1).text(1) == "Waiting for resources"
    panel.watch(False)
    assert not panel.poll.isActive()
    panel.close()


def test_task_manager_is_a_window_panel():
    from cfa4_qt.window_menu import PANELS

    assert "Task Manager" in [p.title for p in PANELS]
