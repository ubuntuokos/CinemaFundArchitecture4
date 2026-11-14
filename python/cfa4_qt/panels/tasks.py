"""Task Manager (ACS-5, ``docs/ai`` adaptive capability plan in CFA4-AI_modules): everything the
engine runs or holds back, in one place, before and while it runs.

Three sources, each read on its own so that one an engine does not have yet never hides the others:

- ``jobs.list``: the background jobs (proxies, transcodes, analysis, renders) with progress, time
  left and status; a job that waits for resources says why in its status (``Waiting: …``).
- ``export.queue.list``: the export queue; an export held back for resources lists the reasons in
  ``waitingFor``.
- ``tasks.list``: the shared task queue (``cfa4-tasks``): its name (``label``, else the id), state,
  class, the RAM and CPU it asked for and what it waits for. An engine without the queue answers with an error; the panel then
  says the queue is not reported yet and shows the other two.

Each running or waiting job and export can be cancelled from its row (``jobs.cancel``,
``export.queue.cancel``). A task whose scheduled start was missed (the computer slept) asks the user
on its row: run now, next slot or cancel (``tasks.decide``). The summary line on top says how many run, how many wait and the first
reason one waits, so the user knows before a task starts why it does not start yet.
"""

from __future__ import annotations

from datetime import datetime
from typing import Any

from PySide6.QtCore import QRect, QSettings, Qt, QTimer, Signal
from PySide6.QtWidgets import QCheckBox, QHBoxLayout, QLabel, QToolButton, QTreeWidget, QTreeWidgetItem, QVBoxLayout, QWidget

from ..i18n import N_, fmt, tr
from .export import format_eta

COLUMNS = (N_("Task"), N_("State"), N_("Details"), N_("Time left"), "")
ACTION_COL = 4
GROUPS = (("jobs", N_("Background jobs")), ("exports", N_("Exports")), ("queue", N_("Shared task queue")))
TASK_STATES = {
    "scheduled": N_("Scheduled"),
    "queued": N_("Queued"),
    "waiting": N_("Waiting for resources"),
    "blocked": N_("Blocked"),
    "running": N_("Running"),
    "paused": N_("Paused"),
    "cancelling": N_("Cancelling"),
    "succeeded": N_("Done"),
    "failed": N_("Failed"),
    "cancelled": N_("Cancelled"),
    "decide": N_("Needs your decision"),
}
TASK_CLASSES = {"interactive": N_("Interactive"), "background": N_("Background"), "speculative": N_("Speculative"), "finalRender": N_("Final render")}
EXPORT_STATES = {"ready": N_("Ready"), "encoding": N_("Encoding"), "done": N_("Done"), "failed": N_("Failed"), "cancelled": N_("Cancelled")}
WAITING = "waiting"
# the user's choices on a task that has not started (``tasks.decide``), e.g. after a missed start
DECISIONS = (("runNow", N_("Run now"), "▶"), ("nextSlot", N_("Next slot"), "⏭"), ("cancel", N_("Cancel"), "✕"))
POLL_MS = 1000
MAX_ROWS = 500  # per source: what an engine sends is untrusted


def _strings(value: Any) -> list[str]:
    return [str(v) for v in value if isinstance(v, (str, int, float))] if isinstance(value, list) else []


def _number(value: Any) -> float | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    f = float(value)
    return f if f == f and f not in (float("inf"), float("-inf")) else None


def _bytes(value: Any) -> str:
    n = _number(value)
    if n is None or n <= 0:
        return ""
    return fmt("{n} GiB", n=f"{n / (1 << 30):.1f}") if n >= 1 << 30 else fmt("{n} MiB", n=f"{n / (1 << 20):.0f}")


def _when(ms: Any) -> str:
    """A UTC millisecond time in local time, or "" when it is not one."""
    n = _number(ms)
    if n is None or not 0 <= n < 253_402_300_800_000:  # up to the year 9999
        return ""
    try:
        return datetime.fromtimestamp(n / 1000).strftime("%Y-%m-%d %H:%M")
    except (OverflowError, OSError, ValueError):
        return ""


def _cancel(cmd: str, params: dict[str, Any]) -> list[tuple[str, str, str, str, dict[str, Any]]]:
    return [("cancel", tr("Cancel"), "✕", cmd, params)]


def job_rows(jobs: Any) -> list[dict[str, Any]]:
    """``jobs.list`` as rows."""
    out = []
    for j in (jobs if isinstance(jobs, list) else [])[:MAX_ROWS]:
        if not isinstance(j, dict):
            continue
        jid = j.get("id")
        done = j.get("finished") is True
        result = j.get("result") if isinstance(j.get("result"), dict) else {}
        err = result.get("error") if isinstance(result.get("error"), str) else None
        status = str(j.get("status") or "")
        waiting = not done and status.startswith("Waiting:")
        if done:
            state = "cancelled" if err and "cancel" in err.lower() else "failed" if err else "succeeded"
            detail = "" if state == "cancelled" else err or ""
        else:
            state = WAITING if waiting else "running"
            frac = min(1.0, max(0.0, _number(j.get("progress")) or 0.0))
            reasons = status.removeprefix("Waiting:").strip()
            detail = reasons if waiting else f"{frac * 100:.0f} %" + (f" · {status}" if status else "")
        eta = _number(j.get("etaSeconds"))
        out.append({
            "kind": "job",
            "id": jid,
            "name": tr(str(j.get("label") or N_("Job"))),
            "state": state,
            "detail": detail,
            "eta": format_eta(eta) if eta is not None and eta >= 0 and state == "running" else "",
            "finished": done,
            "actions": [] if done else _cancel("jobs.cancel", {"job": jid}),
        })
    return out


def export_rows(result: Any) -> list[dict[str, Any]]:
    """``export.queue.list`` as rows; ``waitingFor`` (when the engine sends it) says why one waits."""
    items = result.get("items") if isinstance(result, dict) else None
    out = []
    for it in (items if isinstance(items, list) else [])[:MAX_ROWS]:
        if not isinstance(it, dict):
            continue
        status = str(it.get("status") or "").lower()
        waiting_for = _strings(it.get("waitingFor"))
        done = status in ("done", "failed", "cancelled")
        state = WAITING if waiting_for and not done else status
        eta = _number(it.get("etaSeconds"))
        detail = "; ".join(waiting_for) if state == WAITING else str(it.get("error") or it.get("path") or "")
        out.append({
            "kind": "export",
            "id": it.get("id"),
            "name": str(it.get("sequenceName") or it.get("path") or tr("Export")),
            "state": state,
            "detail": detail,
            "eta": format_eta(eta) if eta is not None and eta >= 0 and status == "encoding" else "",
            "finished": done,
            "actions": [] if done else _cancel("export.queue.cancel", {"id": it.get("id")}),
        })
    return out


def task_rows(tasks: Any) -> list[dict[str, Any]]:
    """``tasks.list`` as rows (read only: the jobs and exports above own their tasks)."""
    out = []
    for t in (tasks if isinstance(tasks, list) else [])[:MAX_ROWS]:
        if not isinstance(t, dict):
            continue
        state = str(t.get("state") or "")
        waiting_for = _strings(t.get("waitingFor"))
        asked = " · ".join(
            p for p in (
                tr(TASK_CLASSES.get(str(t.get("class") or ""), str(t.get("class") or ""))),
                _bytes(t.get("ramBytes")),
                fmt("{n} CPU", n=f"{(_number(t.get('cpuMilli')) or 0) / 1000:g}") if (_number(t.get("cpuMilli")) or 0) > 0 else "",
            ) if p
        )
        decide = t.get("needsDecision") is True and isinstance(t.get("id"), str)
        offered = set(_strings(t.get("decisions")))
        at = _when(t.get("startAt"))
        if decide:
            detail = fmt("Missed its start at {time}", time=at) if at else tr("Missed its start")
        elif state == "scheduled" and at:
            detail = fmt("Starts at {time}", time=at)
        else:
            detail = "; ".join(waiting_for) if waiting_for else asked
        out.append({
            "kind": "task",
            "id": t.get("id"),
            "name": str(t.get("label") or t.get("id") or "")[:200],
            "state": "decide" if decide else state,
            "detail": detail,
            "eta": "",
            "finished": state in ("succeeded", "failed", "cancelled"),
            "actions": [(key, tr(label), icon, "tasks.decide", {"id": t["id"], "action": key}) for key, label, icon in DECISIONS if key in offered] if decide else [],
        })
    return out


def state_label(state: str) -> str:
    return tr(TASK_STATES.get(state) or EXPORT_STATES.get(state) or state)


def summary(rows: list[dict[str, Any]]) -> str:
    """How many run and wait, and why the first one waits (the notice before a task starts)."""
    live = [r for r in rows if not r["finished"] and r["kind"] != "task"]
    running = sum(1 for r in live if r["state"] in ("running", "encoding"))
    waiting = [r for r in live if r["state"] == WAITING]
    if not live:
        return tr("Nothing is running.")
    text = fmt("{running} running · {waiting} waiting", running=running, waiting=len(waiting))
    if waiting and waiting[0]["detail"]:
        text += " — " + fmt("{name} waits: {why}", name=waiting[0]["name"], why=waiting[0]["detail"])
    return text


class TaskTree(QTreeWidget):
    """The rows; each is the automation part ``taskManager.<kind>.<id>`` (its state as the label)."""

    def automation_parts(self) -> list[tuple[str, QRect, str]]:
        out = []
        offset = self.viewport().pos()
        for g in range(self.topLevelItemCount()):
            group = self.topLevelItem(g)
            if group is None or not group.isExpanded():
                continue
            for i in range(group.childCount()):
                item = group.child(i)
                key = item.data(0, Qt.ItemDataRole.UserRole) if item is not None else None
                if item is None or item.isHidden() or not isinstance(key, str):
                    continue
                rect = self.visualItemRect(item).intersected(self.viewport().rect())
                if rect.isValid():
                    out.append((key, rect.translated(offset), item.text(1)))
        return out


class TaskManagerPanel(QWidget):
    """Window ▸ Task Manager."""

    command = Signal(str, dict)

    def __init__(self, engine: Any, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.TaskManager")
        self.engine = engine
        self.settings: QSettings | None = None
        self.sources: dict[str, list[dict[str, Any]]] = {"jobs": [], "exports": [], "queue": []}
        self.unavailable: dict[str, str] = {}
        self.pending: set[str] = set()
        self.summary = QLabel(tr("Nothing is running."), self)
        self.summary.setObjectName("taskManager.summary")
        self.summary.setWordWrap(True)
        self.finished = QCheckBox(tr("Show finished"), self)
        self.finished.setObjectName("taskManager.showFinished")
        self.finished.toggled.connect(self._toggled)
        self.tree = TaskTree(self)
        self.tree.setObjectName("taskManager.tree")
        self.tree.setColumnCount(len(COLUMNS))
        self.tree.setHeaderLabels([tr(c) if c else "" for c in COLUMNS])
        self.tree.setRootIsDecorated(True)
        self.tree.setUniformRowHeights(True)
        self.groups: dict[str, QTreeWidgetItem] = {}
        for key, label in GROUPS:
            item = QTreeWidgetItem([tr(label)])
            item.setData(0, Qt.ItemDataRole.UserRole, f"taskManager.group.{key}")
            self.tree.addTopLevelItem(item)
            item.setExpanded(True)
            self.groups[key] = item
        top = QHBoxLayout()
        top.addWidget(self.summary, 1)
        top.addWidget(self.finished)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(8, 6, 8, 6)
        layout.addLayout(top)
        layout.addWidget(self.tree, 1)
        self.poll = QTimer(self)
        self.poll.setInterval(POLL_MS)
        self.poll.timeout.connect(self.refresh)

    def attach_settings(self, settings: QSettings | None) -> None:
        self.settings = settings
        on = settings.value("panels/taskManager/showFinished", False) if settings is not None else False
        self.finished.setChecked(on in (True, "true", "1", 1))

    def _toggled(self, on: bool) -> None:
        if self.settings is not None:
            self.settings.setValue("panels/taskManager/showFinished", bool(on))
        self.render()

    # ----- reading the engine -----
    def watch(self, shown: bool) -> None:
        """Poll while the panel is open."""
        if shown:
            self.refresh()
            self.poll.start()
        else:
            self.poll.stop()

    def refresh(self) -> None:
        if not getattr(self.engine, "running", False):
            return
        for key, cmd, parse in (("jobs", "jobs.list", job_rows), ("exports", "export.queue.list", export_rows), ("queue", "tasks.list", task_rows)):
            if key in self.pending:
                continue  # one request per source at a time
            self.pending.add(key)
            self.engine.execute(
                cmd,
                {},
                on_reply=lambda r, _b, key=key, parse=parse: self.set_source(key, parse(r)),
                on_error=lambda m, key=key: self.set_unavailable(key, str(m)),
            )

    def set_source(self, key: str, rows: list[dict[str, Any]]) -> None:
        self.pending.discard(key)
        self.unavailable.pop(key, None)
        self.sources[key] = rows
        self.render()

    def set_unavailable(self, key: str, message: str) -> None:
        self.pending.discard(key)
        self.sources[key] = []
        self.unavailable[key] = message
        self.render()

    # ----- drawing -----
    def render(self) -> None:
        show_finished = self.finished.isChecked()
        for key, _label in GROUPS:
            group = self.groups[key]
            rows = [r for r in self.sources[key] if show_finished or not r["finished"]]
            if key in self.unavailable:
                note = tr("Not reported by this engine yet.") if key == "queue" else fmt("Not available: {why}", why=self.unavailable[key])
                rows = [{"kind": key, "id": "unavailable", "name": note, "state": "", "detail": "", "eta": "", "finished": False, "actions": []}]
            while group.childCount() > len(rows):
                group.removeChild(group.child(group.childCount() - 1))
            while group.childCount() < len(rows):
                group.addChild(QTreeWidgetItem())
            for i, r in enumerate(rows):
                item = group.child(i)
                self._fill(item, r)
            live = sum(1 for r in self.sources[key] if not r["finished"])
            group.setText(1, fmt("{n} active", n=live) if live else "")
        self.summary.setText(summary(self.sources["jobs"] + self.sources["exports"]))
        self.summary.setAccessibleDescription(self.summary.text())

    def _fill(self, item: QTreeWidgetItem, r: dict[str, Any]) -> None:
        key = f"taskManager.{r['kind']}.{r['id']}"
        item.setData(0, Qt.ItemDataRole.UserRole, key)
        for col, text in enumerate((r["name"], state_label(r["state"]) if r["state"] else "", r["detail"], r["eta"])):
            item.setText(col, text)
        item.setToolTip(2, r["detail"])
        actions = r["actions"]
        current = self.tree.itemWidget(item, ACTION_COL)
        if not actions:
            if current is not None:
                self.tree.removeItemWidget(item, ACTION_COL)
            return
        signature = f"{key}:" + ",".join(a[0] for a in actions)
        if current is not None and current.property("signature") == signature:
            return
        cell = QWidget()
        cell.setProperty("signature", signature)
        lay = QHBoxLayout(cell)
        lay.setContentsMargins(0, 0, 0, 0)
        lay.setSpacing(2)
        for akey, label, icon, cmd, params in actions:
            b = QToolButton(cell)
            b.setObjectName(f"{key}.{akey}")
            b.setText(icon)
            b.setAutoRaise(True)
            b.setToolTip(label)
            b.setAccessibleName(f"{label}: {r['name']}")
            b.clicked.connect(lambda _c=False, cmd=cmd, params=params: self._act(cmd, params))
            lay.addWidget(b)
        lay.addStretch(1)
        self.tree.setItemWidget(item, ACTION_COL, cell)

    def _act(self, cmd: str, params: dict[str, Any]) -> None:
        self.command.emit(cmd, dict(params))
        QTimer.singleShot(150, self.refresh)
