"""Timecode and Progress, as egui's panels (crates/ui-egui/src/panels/timecode.rs, events.rs).

Timecode: the time of the active monitor in large type, plus optional smaller rows; each row is
Current Time, Media Time, Duration, In/Out Duration or Remaining of the active monitor, the
Program or the Source, in any time display. The values are the engine's (``time.readout``).
Right-click a row to change it; "+" adds a row. The rows are remembered.

Progress: the engine's background jobs (``jobs.list``) with a bar, status and time left, and a
cancel button (``jobs.cancel``); finished jobs on request.
"""

from __future__ import annotations

import json
from typing import Any

from PySide6.QtCore import QRectF, QSettings, Qt, QTimer, Signal
from PySide6.QtGui import QAction, QContextMenuEvent, QFont, QPainter, QPaintEvent, QPen
from PySide6.QtWidgets import QCheckBox, QFrame, QLabel, QMenu, QScrollArea, QToolButton, QVBoxLayout, QWidget

from .. import theme
from ..i18n import N_, fmt, tr
from .export import format_eta

TC_MODES = (
    ("current", N_("Current Time")),
    ("media", N_("Media Time")),
    ("duration", N_("Duration")),
    ("inOut", N_("In/Out Duration")),
    ("remaining", N_("Remaining")),
)
TC_SOURCES = (("active", N_("Active Monitor")), ("program", N_("Program")), ("source", N_("Source")))
TC_DISPLAYS = (
    ("Timecode", N_("Timecode")),
    ("Feet35", N_("Feet + Frames 35mm")),
    ("Feet16", N_("Feet + Frames 16mm")),
    ("Frames", N_("Frames")),
    ("AudioSamples", N_("Audio Samples")),
    ("Seconds", N_("Seconds")),
)
DEFAULT_ROW = {"source": "active", "mode": "current", "display": "Timecode"}


class TimecodePanel(QWidget):
    """One large row, then smaller ones."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.Timecode")
        self.settings: QSettings | None = None
        self.rows: list[dict[str, str]] = [dict(DEFAULT_ROW)]
        self.show_name = True
        self.values: list[dict[str, Any]] = []  # the engine's readout of each row
        self.active = "program"  # which monitor "Active Monitor" means now
        self.add = QToolButton(self)
        self.add.setObjectName("timecode.addRow")
        self.add.setText("+")
        self.add.setAutoRaise(True)
        self.add.setToolTip(tr("Add a timecode row"))
        self.add.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        self.add.clicked.connect(self.add_row)
        self.setMinimumHeight(60)

    # ----- rows -----
    def attach_settings(self, settings: QSettings | None) -> None:
        self.settings = settings
        raw = settings.value("panels/timecode") if settings is not None else None
        try:
            data = json.loads(raw) if isinstance(raw, str) else None
        except ValueError:
            data = None
        if isinstance(data, dict):
            rows = [r for r in data.get("rows") or [] if isinstance(r, dict)]
            ok = [
                {k: str(r.get(k, DEFAULT_ROW[k])) for k in DEFAULT_ROW}
                for r in rows
                if r.get("source", "active") in dict(TC_SOURCES) and r.get("mode", "current") in dict(TC_MODES) and r.get("display", "Timecode") in dict(TC_DISPLAYS)
            ]
            self.rows = ok or [dict(DEFAULT_ROW)]
            self.show_name = bool(data.get("showName", True))

    def _save(self) -> None:
        if self.settings is not None:
            self.settings.setValue("panels/timecode", json.dumps({"rows": self.rows, "showName": self.show_name}))

    def add_row(self) -> None:
        """A row with the last row's settings, Duration by default (egui's "+")."""
        self.rows.append({**self.rows[-1], "mode": "duration"})
        self._save()
        self.changed()

    def set_row(self, i: int, **changes: str) -> None:
        if 0 <= i < len(self.rows):
            self.rows[i] = {**self.rows[i], **changes}
            self._save()
            self.changed()

    def remove_row(self, i: int) -> None:
        if 0 <= i < len(self.rows) and len(self.rows) > 1:
            del self.rows[i]
            self._save()
            self.changed()

    def changed(self) -> None:
        """The rows changed: the window asks the engine again."""
        self.update()
        cb = getattr(self, "on_change", None)
        if callable(cb):
            cb()

    def request(self) -> dict[str, Any]:
        """``time.readout`` params for the rows (Active Monitor resolved)."""
        return {"rows": [{**r, "source": self.active if r["source"] == "active" else r["source"]} for r in self.rows]}

    def set_values(self, result: Any) -> None:
        rows = result.get("rows") if isinstance(result, dict) else None
        values = [r for r in rows or [] if isinstance(r, dict)]
        if values != self.values:
            self.values = values
            self.update()
            if values:
                self.setAccessibleDescription(" · ".join(str(v.get("value")) for v in values))

    # ----- drawing -----
    def row_rects(self) -> list[tuple[QRectF, float]]:
        """Where each row goes and its font size (egui's sizes: the first scales with the panel)."""
        out = []
        y = 10.0
        w, h = self.width(), self.height()
        for i in range(len(self.rows)):
            size = min(64.0, max(22.0, h * 0.32), w / 7.0) if i == 0 else 20.0
            rh = size * 1.25 + (16.0 if self.show_name else 4.0)
            if y + rh > h - 24 and i > 0:
                break
            out.append((QRectF(8, y, w - 16, rh), size))
            y += rh + 8
        return out

    def caption(self, v: dict[str, Any]) -> str:
        src, mode, name = tr(str(v.get("source") or "")), tr(str(v.get("mode") or "")), v.get("name")
        if name is None:
            return fmt("{mode} · no clip", mode=mode) if v.get("source") == "Source" else fmt("{mode} · no sequence", mode=mode)
        return f"{src} · {mode} · {name}"

    def paintEvent(self, _e: QPaintEvent) -> None:
        p = QPainter(self)
        T = theme.current()
        p.fillRect(self.rect(), T.c("panel_bg"))
        for i, (r, size) in enumerate(self.row_rects()):
            v = self.values[i] if i < len(self.values) else {"value": "--:--:--:--"}
            mono = QFont("monospace")
            mono.setStyleHint(QFont.StyleHint.Monospace)
            mono.setPixelSize(int(size))
            p.setFont(mono)
            p.setPen(T.c("timecode") if i == 0 else T.c("text"))
            p.drawText(QRectF(r.left(), r.top(), r.width(), size * 1.25), Qt.AlignmentFlag.AlignHCenter | Qt.AlignmentFlag.AlignTop, str(v.get("value")))
            if self.show_name and "mode" in v:
                f = QFont(self.font())
                f.setPixelSize(11)
                p.setFont(f)
                p.setPen(T.c("text_dim"))
                p.drawText(QRectF(r.left(), r.top() + size * 1.25 + 2, r.width(), 14), Qt.AlignmentFlag.AlignHCenter | Qt.AlignmentFlag.AlignTop, self.caption(v))

    def resizeEvent(self, e) -> None:
        super().resizeEvent(e)
        self.add.setGeometry(self.width() - 28, self.height() - 26, 22, 22)

    def row_at(self, y: float) -> int:
        return next((i for i, (r, _s) in enumerate(self.row_rects()) if r.top() <= y <= r.bottom()), -1)

    def contextMenuEvent(self, e: QContextMenuEvent) -> None:
        i = self.row_at(e.pos().y())
        if i >= 0:
            self.menu_for(i).exec(e.globalPos())

    def menu_for(self, i: int) -> QMenu:
        """egui's row menu: the modes, the sources, the displays, Remove Row."""
        row = self.rows[i]
        menu = QMenu(self)

        def pick(key: str, value: str, label: str) -> None:
            a = QAction(tr(label), menu, checkable=True)
            a.setObjectName(f"timecode.row.{i}.{key}.{value}")
            a.setChecked(row[key] == value)
            a.triggered.connect(lambda _c=False: self.set_row(i, **{key: value}))
            menu.addAction(a)

        for k, label in TC_MODES:
            pick("mode", k, label)
        menu.addSeparator()
        for k, label in TC_SOURCES:
            pick("source", k, label)
        menu.addSeparator()
        for k, label in TC_DISPLAYS:
            pick("display", k, label)
        if len(self.rows) > 1:
            menu.addSeparator()
            rm = menu.addAction(tr("Remove Row"))
            rm.setObjectName(f"timecode.row.{i}.remove")
            rm.triggered.connect(lambda: self.remove_row(i))
        return menu


class JobRow(QWidget):
    cancel = Signal(object)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setFixedHeight(48)
        self.job: dict[str, Any] = {}
        self.stop = QToolButton(self)
        self.stop.setText("✕")
        self.stop.setAutoRaise(True)
        self.stop.setToolTip(tr("Cancel"))
        self.stop.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        self.stop.clicked.connect(lambda: self.cancel.emit(self.job.get("id")))

    def set_job(self, job: dict[str, Any]) -> None:
        self.job = job
        self.setObjectName(f"progress.job.{job.get('id')}")
        self.stop.setObjectName(f"progress.cancel.{job.get('id')}")
        self.stop.setVisible(job.get("finished") is not True)
        self.setAccessibleName(f"{tr(str(job.get('label') or 'Job'))}: {self.status()}")
        self.update()

    def status(self) -> str:
        j = self.job
        done = j.get("finished") is True
        err = (j.get("result") or {}).get("error") if isinstance(j.get("result"), dict) else None
        if done and isinstance(err, str):
            return tr("Cancelled") if "cancel" in err.lower() else fmt("Failed: {e}", e=err)
        if done:
            return tr("Done")
        frac = min(1.0, max(0.0, float(j.get("progress") or 0.0)))
        eta = j.get("etaSeconds")
        left = ""
        if isinstance(eta, (int, float)) and eta >= 0:
            left = fmt(" · {time} left", time=format_eta(float(eta)))
        s = str(j.get("status") or "")
        n = f"{int(j.get('done') or 0)} / {int(j.get('total') or 0)}"
        return f"{frac * 100:.0f} %  {s}{left}" if s else f"{frac * 100:.0f} %  ({n}){left}"

    def resizeEvent(self, e) -> None:
        super().resizeEvent(e)
        self.stop.setGeometry(self.width() - 28, 14, 20, 20)

    def paintEvent(self, _e: QPaintEvent) -> None:
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        T = theme.current()
        j = self.job
        r = QRectF(self.rect()).adjusted(1, 1, -1, -1)
        p.setPen(Qt.PenStyle.NoPen)
        p.setBrush(T.c("row_alt"))
        p.drawRoundedRect(r, 3, 3)
        f = QFont(self.font())
        f.setPixelSize(12)
        f.setBold(True)
        p.setFont(f)
        p.setPen(T.c("text"))
        p.drawText(QRectF(8, 4, self.width() - 44, 16), Qt.AlignmentFlag.AlignVCenter, tr(str(j.get("label") or "Job")))
        done = j.get("finished") is True
        err = isinstance((j.get("result") or {}).get("error") if isinstance(j.get("result"), dict) else None, str)
        bar = QRectF(8, 24, self.width() - 44, 6)
        p.setPen(Qt.PenStyle.NoPen)
        p.setBrush(T.c("field_bg"))
        p.drawRoundedRect(bar, 3, 3)
        frac = 1.0 if done and not err else min(1.0, max(0.0, float(j.get("progress") or 0.0)))
        p.setBrush(T.c("danger") if err and done else T.c("accent"))
        p.drawRoundedRect(QRectF(bar.left(), bar.top(), bar.width() * frac, bar.height()), 3, 3)
        f.setBold(False)
        f.setPixelSize(11)
        p.setFont(f)
        p.setPen(QPen(T.c("danger") if err else T.c("text_dim")))
        p.drawText(QRectF(8, 32, self.width() - 16, 14), Qt.AlignmentFlag.AlignVCenter, self.status())



class ProgressPanel(QWidget):
    command = Signal(str, dict)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.Progress")
        self.settings: QSettings | None = None
        self.jobs: list[dict[str, Any]] = []
        self.finished = QCheckBox(tr("Show finished jobs"), self)
        self.finished.setObjectName("progress.showFinished")
        self.finished.toggled.connect(self._toggled)
        self.list = QWidget(self)
        self.lay = QVBoxLayout(self.list)
        self.lay.setContentsMargins(0, 0, 0, 0)
        self.lay.setSpacing(4)
        self.empty_text = QLabel(tr("No background jobs."), self.list)
        self.empty_text.setObjectName("progress.empty")
        self.lay.addWidget(self.empty_text)
        self.lay.addStretch(1)
        scroll = QScrollArea(self)
        scroll.setWidgetResizable(True)
        scroll.setFrameShape(QFrame.Shape.NoFrame)
        scroll.setWidget(self.list)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(8, 6, 8, 6)
        layout.addWidget(self.finished)
        layout.addWidget(scroll, 1)
        self.rows: list[JobRow] = []
        self.poll = QTimer(self)
        self.poll.setInterval(250)  # while a job runs, as egui repaints

    def attach_settings(self, settings: QSettings | None) -> None:
        self.settings = settings
        on = settings.value("panels/progress/showFinished", False) if settings is not None else False
        self.finished.setChecked(on in (True, "true", "1", 1))

    def _toggled(self, on: bool) -> None:
        if self.settings is not None:
            self.settings.setValue("panels/progress/showFinished", bool(on))
        self.set_jobs(self.jobs)

    def set_jobs(self, jobs: Any) -> None:
        self.jobs = [j for j in jobs if isinstance(j, dict)] if isinstance(jobs, list) else []
        shown = [j for j in reversed(self.jobs) if self.finished.isChecked() or j.get("finished") is not True]
        while len(self.rows) < len(shown):
            row = JobRow(self.list)
            row.cancel.connect(lambda jid: self.command.emit("jobs.cancel", {"job": jid}))
            self.lay.insertWidget(len(self.rows), row)
            self.rows.append(row)
        for i, row in enumerate(self.rows):
            if i < len(shown):
                row.set_job(shown[i])
                row.show()
            else:
                row.hide()
        self.empty_text.setVisible(not shown)
        active = any(j.get("finished") is not True for j in self.jobs)
        self.empty_text.setText("" if active else tr("No background jobs."))
        if active and not self.poll.isActive():
            self.poll.start()
        elif not active:
            self.poll.stop()
