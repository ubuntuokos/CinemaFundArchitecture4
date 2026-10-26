"""Smaller panels: Effects browser, History and Events."""

from __future__ import annotations

import time
from typing import Any

from PySide6.QtCore import Qt, Signal
from PySide6.QtWidgets import (
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QPlainTextEdit,
    QTreeWidget,
    QTreeWidgetItem,
    QVBoxLayout,
    QWidget,
)

from ..i18n import tr


class EffectsPanel(QWidget):
    """Effects and transitions from ``effects.list``; double-click applies to the selected clips."""

    command = Signal(str, dict)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.Effects")
        self.search = QLineEdit(self)
        self.search.setObjectName("effects.search")
        self.search.setPlaceholderText(tr("Search"))
        self.search.textChanged.connect(self._filter)
        self.tree = QTreeWidget(self)
        self.tree.setObjectName("effects.tree")
        self.tree.setHeaderHidden(True)
        self.tree.itemDoubleClicked.connect(self._apply)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(2, 2, 2, 2)
        layout.addWidget(self.search)
        layout.addWidget(self.tree, 1)

    def set_effects(self, effects: Any) -> None:
        self.tree.clear()
        folders: dict[str, QTreeWidgetItem] = {}
        for e in effects if isinstance(effects, list) else []:
            if isinstance(e, str):
                e = {"id": e, "name": e}
            if not isinstance(e, dict):
                continue
            folder = str(e.get("path") or e.get("folder") or e.get("kind") or "")
            parent = folders.get(folder)
            if parent is None:
                parent = QTreeWidgetItem([" / ".join(tr(part) for part in folder.split("/")) if folder else tr("Effects")])
                self.tree.addTopLevelItem(parent)
                folders[folder] = parent
            name = str(e.get("name") or e.get("id") or "")
            it = QTreeWidgetItem([tr(name)])
            it.setData(0, Qt.ItemDataRole.UserRole, e.get("id") or name)
            it.setData(0, Qt.ItemDataRole.UserRole + 1, str(e.get("kind") or ""))
            parent.addChild(it)
        self.tree.sortItems(0, Qt.SortOrder.AscendingOrder)

    def _filter(self, text: str) -> None:
        needle = text.strip().lower()
        for i in range(self.tree.topLevelItemCount()):
            folder = self.tree.topLevelItem(i)
            visible = 0
            for j in range(folder.childCount()):
                child = folder.child(j)
                hide = bool(needle) and needle not in child.text(0).lower()
                child.setHidden(hide)
                visible += not hide
            folder.setHidden(visible == 0)
            folder.setExpanded(bool(needle))

    def _apply(self, item: QTreeWidgetItem, _col: int) -> None:
        eid = item.data(0, Qt.ItemDataRole.UserRole)
        if eid is None:
            return
        kind = str(item.data(0, Qt.ItemDataRole.UserRole + 1) or "")
        if "Transition" in kind:
            # a transition goes on the edit at the playhead, as the default transition does
            self.command.emit("effects.setDefaultTransition", {"effect": eid})
            audio = kind.startswith("Audio")
            self.command.emit("sequence.applyAudioTransition" if audio else "sequence.applyVideoTransition", {})
        else:
            self.command.emit("effects.apply", {"effect": eid})


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
