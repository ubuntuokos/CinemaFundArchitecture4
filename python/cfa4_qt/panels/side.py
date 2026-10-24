"""Smaller panels: Effects browser, Inspector (selected clips), History and Events."""

from __future__ import annotations

import time
from typing import Any

from PySide6.QtCore import Qt, Signal
from PySide6.QtWidgets import (
    QFormLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QPlainTextEdit,
    QPushButton,
    QTreeWidget,
    QTreeWidgetItem,
    QVBoxLayout,
    QWidget,
)

from ..i18n import tr
from ..timecode import format_timecode, frame_ticks


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


class InspectorPanel(QWidget):
    """The selected clips: timing, speed, gain and effects (read from ``sequence.inspect``)."""

    command = Signal(str, dict)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.Inspector")
        self.form = QFormLayout()
        self.effects = QListWidget(self)
        self.effects.setObjectName("inspector.effects")
        self.remove = QPushButton(tr("Remove Effect"), self)
        self.remove.setObjectName("inspector.removeEffect")
        self.remove.clicked.connect(self._remove_effect)
        self.empty = QLabel(tr("Select a clip in the Timeline."), self)
        self.empty.setWordWrap(True)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(6, 6, 6, 6)
        layout.addWidget(self.empty)
        layout.addLayout(self.form)
        layout.addWidget(QLabel(tr("Effects"), self))
        layout.addWidget(self.effects, 1)
        layout.addWidget(self.remove)
        self._clip: dict[str, Any] | None = None

    def set_sequence(self, seq: dict[str, Any] | None) -> None:
        while self.form.rowCount():
            self.form.removeRow(0)
        self.effects.clear()
        self._clip = None
        selected = set((seq or {}).get("selection") or [])
        clips = [
            c
            for kind in ("video", "audio")
            for t in (seq or {}).get(kind) or []
            for c in t.get("items") or []
            if c.get("clip") in selected
        ]
        self.empty.setVisible(not clips)
        self.remove.setEnabled(False)
        if not clips:
            return
        ft = frame_ticks((seq or {}).get("settings"))
        if len(clips) > 1:
            self.form.addRow(tr("Selected"), QLabel(str(len(clips))))
        clip = clips[0]
        self._clip = clip
        rows = [
            ("Name", str(clip.get("name") or "")),
            ("Start", format_timecode(int(clip.get("start") or 0), ft)),
            ("End", format_timecode(int(clip.get("end") or 0), ft)),
            ("Duration", format_timecode(int(clip.get("duration") or 0), ft)),
            ("Source In", format_timecode(int(clip.get("sourceIn") or 0), ft)),
            ("Speed", f"{float(clip.get('speed') or 1.0) * 100:.1f}%" + (" ⟲" if clip.get("reverse") else "")),
            ("Gain", f"{float(clip.get('gainDb') or 0.0):+.1f} dB"),
            ("Enabled", tr("Yes") if clip.get("enabled", True) else tr("No")),
        ]
        for label, value in rows:
            v = QLabel(value)
            v.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
            self.form.addRow(tr(label), v)
        for idx, e in enumerate(clip.get("effects") or []):
            params = e.get("params") or {}
            text = f"{tr(str(e.get('effect')))}" + ("" if e.get("enabled", True) else f"  ({tr('off')})")
            if params:
                text += "\n    " + ", ".join(f"{k}={v.get('value')}" for k, v in list(params.items())[:6])
            it = QListWidgetItem(text)
            it.setData(Qt.ItemDataRole.UserRole, idx)
            self.effects.addItem(it)
        self.remove.setEnabled(self.effects.count() > 0)

    def _remove_effect(self) -> None:
        it = self.effects.currentItem()
        if it is None or self._clip is None:
            return
        self.command.emit(
            "effects.remove", {"clip": self._clip.get("clip"), "index": it.data(Qt.ItemDataRole.UserRole)}
        )


class HistoryPanel(QWidget):
    command = Signal(str, dict)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.History")
        self.list = QListWidget(self)
        self.list.setObjectName("history.list")
        layout = QVBoxLayout(self)
        layout.setContentsMargins(2, 2, 2, 2)
        layout.addWidget(self.list)

    def set_history(self, history: Any) -> None:
        self.list.clear()
        if not isinstance(history, dict):
            return
        for label in history.get("undo") or []:
            self.list.addItem(tr(str(label)))
        for label in reversed(history.get("redo") or []):
            it = QListWidgetItem(tr(str(label)))
            it.setForeground(self.palette().placeholderText())
            self.list.addItem(it)
        self.list.scrollToBottom()


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
