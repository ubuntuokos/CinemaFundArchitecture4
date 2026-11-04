"""The status bar as egui's (``crates/ui-egui/src/lib.rs`` ``status_bar`` / ``job_status``): the
tool's hint on the left, the running job (export, preview render) on the right with its progress
bar and a cancel button (``status.job.progress`` / ``status.job.cancel``)."""

from __future__ import annotations

from typing import Any

from PySide6.QtCore import Qt, Signal
from PySide6.QtWidgets import QHBoxLayout, QLabel, QProgressBar, QToolButton, QWidget

from .i18n import N_, fmt, tr
from .panels.export import format_eta

# egui's ``hint_text`` (the Linux wording: Alt and Ctrl), by the Tools panel's tool ids
TOOL_HINTS: dict[str, str] = {
    "selection": N_("Click to select, or click in empty space and drag to marquee select. Use Shift, Alt, and Ctrl for other options."),
    "trackSelectForward": N_("Click to select all clips to the right in all tracks. Shift-click for a single track."),
    "trackSelectBackward": N_("Click to select all clips to the left in all tracks. Shift-click for a single track."),
    "ripple": N_("Drag an edit point to ripple trim; later clips move to keep the gap closed."),
    "rolling": N_("Drag an edit point to roll it: the out of one clip and the in of the next move together."),
    "rateStretch": N_("Drag an edge to change the clip's speed so it fills the new duration."),
    "remix": N_("Drag the edge of a music clip to remix it to the new duration at musically matching beats."),
    "razor": N_("Click to split a clip. Shift-click to split all tracks."),
    "slip": N_("Drag a clip to slip its source in/out without moving it."),
    "slide": N_("Drag a clip to slide it between its neighbours."),
    "hand": N_("Drag to scroll the timeline."),
    "zoom": N_("Click to zoom in; Alt-click to zoom out."),
}


def tool_hint(tool: str) -> str:
    """The status bar's hint for ``tool`` (none for the drawing and type tools, as egui)."""
    hint = TOOL_HINTS.get(tool)
    return tr(hint) if hint else ""


def running_job(jobs: Any) -> dict[str, Any] | None:
    """The newest job that has not finished (egui's ``watch_jobs``)."""
    if not isinstance(jobs, list):
        return None
    return next((j for j in reversed(jobs) if isinstance(j, dict) and j.get("finished") is not True), None)


def job_text(job: dict[str, Any]) -> str:
    """``Exporting… 42% · 0:12 left``; a preview render names itself (egui's verb)."""
    label = str(job.get("label") or "")
    verb = tr(label) if label.startswith("Rendering") else tr("Exporting")
    try:
        f = min(1.0, max(0.0, float(job.get("progress") or 0.0)))
    except (TypeError, ValueError):
        f = 0.0
    eta = job.get("etaSeconds")
    left = fmt(" · {time} left", time=format_eta(eta)) if isinstance(eta, (int, float)) and eta >= 0 else ""
    return f"{verb}… {f * 100:.0f}%{left}"


class JobStatus(QWidget):
    """The running job at the status bar's right end; hidden while nothing runs."""

    cancel = Signal(int)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("status.job")
        self.job: dict[str, Any] | None = None
        self.text = QLabel(self)
        self.text.setObjectName("status.job.text")
        self.bar = QProgressBar(self)
        self.bar.setObjectName("status.job.progress")
        self.bar.setRange(0, 1000)
        self.bar.setTextVisible(False)
        self.bar.setFixedSize(120, 6)
        self.stop = QToolButton(self)
        self.stop.setObjectName("status.job.cancel")
        self.stop.setText("✕")
        self.stop.setAutoRaise(True)
        self.stop.setToolTip(tr("Cancel"))
        self.stop.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        self.stop.clicked.connect(self._cancel)
        row = QHBoxLayout(self)
        row.setContentsMargins(0, 0, 4, 0)
        row.setSpacing(8)
        row.addWidget(self.text)
        row.addWidget(self.bar, 0, Qt.AlignmentFlag.AlignVCenter)
        row.addWidget(self.stop)
        self.hide()

    def set_jobs(self, jobs: Any) -> dict[str, Any] | None:
        """Show the running job, or hide; returns it."""
        job = running_job(jobs)
        self.job = job
        if job is None:
            self.hide()
            return None
        text = job_text(job)
        self.text.setText(text)
        try:
            f = min(1.0, max(0.0, float(job.get("progress") or 0.0)))
        except (TypeError, ValueError):
            f = 0.0
        self.bar.setValue(round(f * 1000))
        self.bar.setAccessibleName(text)
        self.stop.setAccessibleName(f"{tr('Cancel')} {tr(str(job.get('label') or ''))}")
        self.show()
        return job

    def _cancel(self) -> None:
        if self.job is not None and isinstance(self.job.get("id"), int):
            self.cancel.emit(int(self.job["id"]))
