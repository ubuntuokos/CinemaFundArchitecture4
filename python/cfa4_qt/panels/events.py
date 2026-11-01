"""Events panel (egui's ``panels/events.rs``): warnings and errors of commands, background jobs and
the auto-save worker from the engine's event log (``events.list``), with the Qt GUI's own messages
(an engine that stopped, a dialog that could not open, …) merged in. Newest first, filtered by
level (All Events / Warnings and Errors / Errors Only), the counts per level, Clear All, and the
selected entry's details below the list.

Automation ids as egui's: ``events.filter``, ``events.clearAll``, ``events.list`` (rows carry
their entry id), ``events.details``; ``events.count.<error|warning|info>``. State as egui's
``ui.panels.events``: ``level`` and ``selected``.
"""

from __future__ import annotations

import time
from dataclasses import dataclass
from typing import Any

from PySide6.QtCore import QPointF, QRectF, Qt, QTimer, Signal
from PySide6.QtGui import QColor, QIcon, QPainter, QPixmap, QPolygonF
from PySide6.QtWidgets import (
    QComboBox,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QTreeWidget,
    QTreeWidgetItem,
    QVBoxLayout,
    QWidget,
)

from ..i18n import N_, tr

LEVELS = ("info", "warning", "error")
LEVEL_LABELS = {"info": N_("Info"), "warning": N_("Warning"), "error": N_("Error")}
FILTER_LABELS = {"info": N_("All Events"), "warning": N_("Warnings and Errors"), "error": N_("Errors Only")}
COLORS = {"info": QColor(0x57, 0x94, 0xEC), "warning": QColor(0xE8, 0xB3, 0x3C), "error": QColor(0xE0, 0x4F, 0x4F)}
LOCAL_IDS = 1 << 40  # the GUI's own entries: ids that never meet the engine's
LOCAL_LIMIT = 1000  # as the engine's EventLog::LIMIT
MATCH_MS = 10_000  # a GUI message the engine also logged (same source and text) is shown once


@dataclass
class Entry:
    id: int
    level: str
    source: str
    message: str
    count: int
    time_ms: int


def ago(ms: int, now_ms: int | None = None) -> str:
    """egui's ``ago``: just now, 12 s ago, 3 min ago, 2 h ago."""
    now = int(time.time() * 1000) if now_ms is None else now_ms
    s = max(0, now - int(ms)) // 1000
    if s <= 4:
        return tr("just now")
    if s <= 59:
        return tr("{n} s ago").format(n=s)
    if s <= 3599:
        return tr("{n} min ago").format(n=s // 60)
    return tr("{n} h ago").format(n=s // 3600)


def glyph(level: str, size: int = 14) -> QIcon:
    """egui's level glyph, drawn in code: a dot (info), a triangle (warning), a square (error)."""
    pm = QPixmap(size * 2, size * 2)
    pm.fill(Qt.GlobalColor.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.RenderHint.Antialiasing)
    p.scale(2, 2)
    c = QPointF(size / 2, size / 2)
    col = COLORS.get(level, COLORS["info"])
    p.setPen(Qt.PenStyle.NoPen)
    p.setBrush(col)
    if level == "warning":
        p.drawPolygon(QPolygonF([c + QPointF(0, -6), c + QPointF(6, 5), c + QPointF(-6, 5)]))
    elif level == "error":
        p.drawRoundedRect(QRectF(c.x() - 5, c.y() - 5, 10, 10), 2, 2)
    else:
        p.drawEllipse(c, 5, 5)
    p.setPen(QColor(0, 0, 0))
    f = p.font()
    f.setPixelSize(8)
    f.setBold(True)
    p.setFont(f)
    p.drawText(QRectF(c.x() - 5, c.y() - 5 + (1 if level == "warning" else 0), 10, 10), Qt.AlignmentFlag.AlignCenter, "i" if level == "info" else "!")
    p.end()
    return QIcon(pm)


def parse_entries(result: Any) -> list[Entry]:
    """The engine's ``events.list`` entries (bad ones left out)."""
    out: list[Entry] = []
    for e in (result or {}).get("entries") or [] if isinstance(result, dict) else []:
        if not isinstance(e, dict) or not isinstance(e.get("id"), int) or e.get("level") not in LEVELS:
            continue
        count = e.get("count")
        when = e.get("timeMs")
        out.append(Entry(e["id"], e["level"], str(e.get("source") or ""), str(e.get("message") or ""), count if isinstance(count, int) and count > 0 else 1, when if isinstance(when, int) else 0))
    return out


class _LogText:
    """The whole log as plain text, newest last (what the old text log offered to tests and
    diagnostics)."""

    def __init__(self, panel: EventsPanel) -> None:
        self._panel = panel

    def toPlainText(self) -> str:
        return "\n".join(f"{'⚠ ' if e.level != 'info' else ''}{e.message}" for e in reversed(self._panel.entries()))


class EventsPanel(QWidget):
    """The Events panel; ``changed`` fires when the shown state (level, selection) changes."""

    changed = Signal()

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.Events")
        self.engine: Any = None
        self.level = "info"
        self.selected: int | None = None
        self.engine_entries: list[Entry] = []
        self.engine_counts: dict[str, int] = {}
        self.local: list[Entry] = []
        self._next_local = LOCAL_IDS
        self._busy = False
        self._again = False
        self.text = _LogText(self)

        bar = QHBoxLayout()
        bar.setContentsMargins(0, 0, 0, 0)
        self.filter = QComboBox(self)
        self.filter.setObjectName("events.filter")
        for lv in LEVELS:
            self.filter.addItem(tr(FILTER_LABELS[lv]), lv)
        self.filter.setMinimumWidth(150)
        self.filter.activated.connect(lambda i: self.set_level(self.filter.itemData(i)))
        bar.addWidget(self.filter)
        self.counts: dict[str, QLabel] = {}
        for lv in ("error", "warning", "info"):
            icon = QLabel(self)
            icon.setPixmap(glyph(lv).pixmap(14, 14))
            n = QLabel("0", self)
            n.setObjectName(f"events.count.{lv}")
            n.setEnabled(False)  # the theme's dim text
            n.setToolTip(tr(LEVEL_LABELS[lv]))
            bar.addWidget(icon)
            bar.addWidget(n)
            self.counts[lv] = n
        bar.addStretch(1)
        self.clear_button = QPushButton(tr("Clear All"), self)
        self.clear_button.setObjectName("events.clearAll")
        self.clear_button.clicked.connect(self.clear_all)
        bar.addWidget(self.clear_button)

        self.list = QTreeWidget(self)
        self.list.setObjectName("events.list")
        self.list.setHeaderHidden(True)
        self.list.setRootIsDecorated(False)
        self.list.setColumnCount(2)
        self.list.setAlternatingRowColors(True)
        self.list.setUniformRowHeights(True)
        self.list.itemClicked.connect(self._clicked)
        self.empty = QLabel(tr("No events."), self)
        self.empty.setObjectName("events.empty")
        self.empty.setEnabled(False)
        self.details = QLabel("", self)
        self.details.setObjectName("events.details")
        self.details.setWordWrap(True)
        self.details.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        self.details.setMinimumHeight(60)
        self.details.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignTop)
        self.details.setStyleSheet("padding: 4px 6px;")
        self.details.hide()

        layout = QVBoxLayout(self)
        layout.setContentsMargins(8, 6, 8, 6)
        layout.setSpacing(4)
        layout.addLayout(bar)
        layout.addWidget(self.list, 1)
        layout.addWidget(self.empty)
        layout.addWidget(self.details)
        # the "… ago" times move on while the panel is shown
        self._ago = QTimer(self)
        self._ago.setInterval(5000)
        self._ago.timeout.connect(self._rebuild)
        self._rebuild()

    # ----- data -----
    def attach(self, engine: Any) -> None:
        self.engine = engine

    def refresh(self) -> None:
        """Read the engine's event log again (after a command, a toast, a finished job)."""
        if self.engine is None or not getattr(self.engine, "running", False):
            return
        if self._busy:
            self._again = True
            return
        self._busy = True

        def got(r: Any, _b: Any) -> None:
            self._busy = False
            entries = parse_entries(r)
            counts = r.get("counts") if isinstance(r, dict) else None
            counts = {lv: int(counts.get(lv) or 0) for lv in LEVELS} if isinstance(counts, dict) else {}
            if (entries, counts) != (self.engine_entries, self.engine_counts):  # most refreshes change nothing
                self.engine_entries, self.engine_counts = entries, counts
                self._rebuild()
            if self._again:
                self._again = False
                self.refresh()

        def failed(_m: str) -> None:
            self._busy = False

        self.engine.execute("events.list", {}, on_reply=got, on_error=failed)

    def log(self, message: str, error: bool = False, source: str = "app") -> None:
        """A message of the Qt GUI itself (errors it reports, results it announces)."""
        now = int(time.time() * 1000)
        level = "error" if error else "info"
        last = self.local[-1] if self.local else None
        if last is not None and (last.level, last.source, last.message) == (level, source, message):
            last.count += 1  # repeats bump the count, as the engine's log
            last.time_ms = now
        else:
            self.local.append(Entry(self._next_local, level, source, message, 1, now))
            self._next_local += 1
            del self.local[:-LOCAL_LIMIT]
        self._rebuild()
        self.refresh()

    def _shown_local(self) -> list[Entry]:
        """The GUI's entries the engine did not log too (a failed command is in both)."""
        seen = {(e.source, e.message): e.time_ms for e in self.engine_entries}
        out = []
        for e in self.local:
            t = seen.get((e.source, e.message))
            if t is None:
                # the GUI reports a failed command as "command: message"
                cid, _, rest = e.message.partition(": ")
                t = seen.get((cid, rest)) if rest else None
            if t is not None and abs(t - e.time_ms) <= MATCH_MS:
                continue
            out.append(e)
        return out

    def entries(self) -> list[Entry]:
        """Every entry, newest first."""
        merged = self.engine_entries + self._shown_local()
        return sorted(merged, key=lambda e: (e.time_ms, e.id), reverse=True)

    def shown(self) -> list[Entry]:
        """The entries at or above the filter's level, newest first."""
        lo = LEVELS.index(self.level)
        return [e for e in self.entries() if LEVELS.index(e.level) >= lo]

    def count(self, level: str) -> int:
        local = sum(1 for e in self._shown_local() if e.level == level)
        engine = self.engine_counts.get(level, sum(1 for e in self.engine_entries if e.level == level))
        return engine + local

    # ----- actions -----
    def set_level(self, level: Any) -> None:
        if level not in LEVELS:
            return
        if level != self.level:
            self.level = level
            self._rebuild()
            self.changed.emit()

    def select(self, entry_id: int | None) -> None:
        if entry_id != self.selected:
            self.selected = entry_id
            self._rebuild()
            self.changed.emit()

    def _clicked(self, item: QTreeWidgetItem, _col: int) -> None:
        eid = item.data(0, Qt.ItemDataRole.UserRole)
        if isinstance(eid, int):
            # a second click deselects; the list is rebuilt after the click has been handled
            QTimer.singleShot(0, self, lambda: self.select(None if eid == self.selected else eid))

    def clear_all(self) -> None:
        """Clear All: the engine's log (``events.clear``) and the GUI's messages."""
        self.local.clear()
        self.engine_entries = []
        self.engine_counts = {}
        self.selected = None
        self._rebuild()
        self.changed.emit()
        if self.engine is not None and getattr(self.engine, "running", False):
            self.engine.execute("events.clear", {}, on_reply=lambda _r, _b: self.refresh(), on_error=lambda _m: self.refresh())

    # ----- view -----
    def showEvent(self, e) -> None:
        super().showEvent(e)
        self._ago.start()
        self._rebuild()

    def hideEvent(self, e) -> None:
        super().hideEvent(e)
        self._ago.stop()

    def _rebuild(self) -> None:
        shown = self.shown()
        i = self.filter.findData(self.level)
        if i >= 0 and i != self.filter.currentIndex():
            self.filter.setCurrentIndex(i)
        for lv, label in self.counts.items():
            label.setText(str(self.count(lv)))
        now = int(time.time() * 1000)
        scroll = self.list.verticalScrollBar().value()
        self.list.clear()
        current = None
        for e in shown:
            msg = f"{e.message}  (×{e.count})" if e.count > 1 else e.message
            it = QTreeWidgetItem([msg, ago(e.time_ms, now)])
            it.setIcon(0, glyph(e.level))
            it.setData(0, Qt.ItemDataRole.UserRole, e.id)
            it.setToolTip(0, e.message)
            it.setTextAlignment(1, Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter)
            it.setForeground(1, self.palette().placeholderText())
            self.list.addTopLevelItem(it)
            if e.id == self.selected:
                current = (it, e)
        self.list.resizeColumnToContents(1)
        w = self.list.viewport().width() - self.list.columnWidth(1)
        self.list.setColumnWidth(0, max(80, w))
        self.list.verticalScrollBar().setValue(scroll)
        self.empty.setVisible(not shown)
        if current is not None:
            it, e = current
            self.list.setCurrentItem(it)
            it.setSelected(True)
            self.details.setText(f"{tr(LEVEL_LABELS[e.level])} · {e.source} · {ago(e.time_ms, now)}\n{e.message}")
            self.details.show()
        else:
            self.list.clearSelection()
            self.details.clear()
            self.details.hide()

    def resizeEvent(self, e) -> None:
        super().resizeEvent(e)
        w = self.list.viewport().width() - self.list.columnWidth(1)
        self.list.setColumnWidth(0, max(80, w))

    # ----- state (egui's ui.panels.events) -----
    def state(self) -> dict[str, Any]:
        return {"level": self.level, "selected": self.selected}

    def set_state(self, patch: dict[str, Any]) -> None:
        if "level" in patch:
            self.set_level(patch["level"])
        if "selected" in patch:
            self.select(patch["selected"])
