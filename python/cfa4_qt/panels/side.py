"""Smaller panels: History, Events and the Libraries placeholder."""

from __future__ import annotations

import time
from typing import Any

from PySide6.QtCore import Qt, Signal
from PySide6.QtWidgets import (
    QLabel,
    QListWidget,
    QListWidgetItem,
    QPlainTextEdit,
    QVBoxLayout,
    QWidget,
)

from ..i18n import fmt, tr


class HistoryPanel(QWidget):
    """egui's History: Open, then every undo step (the current one highlighted), then the
    redo steps dimmed; a click undoes or redoes to that step."""

    command = Signal(str, dict)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.History")
        self.list = QListWidget(self)
        self.list.setObjectName("history.list")
        self.list.itemClicked.connect(self._clicked)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(2, 2, 2, 2)
        layout.addWidget(self.list)

    def set_history(self, history: Any) -> None:
        self.list.clear()
        if not isinstance(history, dict):
            return
        undo = [str(x) for x in history.get("undo") or []]
        redo = [str(x) for x in reversed(history.get("redo") or [])]
        rows = [(tr("Open"), -len(undo))] + [(tr(label), i + 1 - len(undo)) for i, label in enumerate(undo)]
        for label, steps in rows:
            it = QListWidgetItem(label)
            it.setData(Qt.ItemDataRole.UserRole, steps)
            self.list.addItem(it)
            if steps == 0:
                it.setSelected(True)
                self.list.setCurrentItem(it)
        for i, label in enumerate(redo):
            it = QListWidgetItem(tr(label))
            it.setData(Qt.ItemDataRole.UserRole, i + 1)
            it.setForeground(self.palette().placeholderText())
            self.list.addItem(it)
        self.list.scrollToBottom()

    def _clicked(self, item: QListWidgetItem) -> None:
        steps = int(item.data(Qt.ItemDataRole.UserRole) or 0)
        for _ in range(abs(steps)):
            self.command.emit("edit.undo" if steps < 0 else "edit.redo", {})


class EventsPanel(QWidget):
    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.Events")
        self.text = QPlainTextEdit(self)
        self.text.setObjectName("events.log")
        self.text.setReadOnly(True)
        self.text.setMaximumBlockCount(2000)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(2, 2, 2, 2)
        layout.addWidget(self.text)

    def log(self, message: str, error: bool = False) -> None:
        stamp = time.strftime("%H:%M:%S")
        self.text.appendPlainText(f"{stamp}  {'⚠ ' if error else ''}{message}")


class LibrariesPanel(QLabel):
    """Libraries: egui shows this panel as a placeholder for a later milestone, and so does the Qt
    GUI (``crates/ui-egui/src/dock.rs`` ``placeholder``)."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(fmt("{panel} — coming in a later milestone", panel=tr("Libraries")), parent)
        self.setObjectName("panel.Libraries")
        self.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.setWordWrap(True)
        self.setEnabled(False)  # drawn dimmed, as egui's placeholder text
