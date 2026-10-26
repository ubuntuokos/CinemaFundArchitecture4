"""Project panel: the bin tree from ``project.inspect``."""

from __future__ import annotations

from typing import Any

from PySide6.QtCore import QMimeData, Qt, Signal
from PySide6.QtWidgets import (
    QAbstractItemView,
    QHBoxLayout,
    QLineEdit,
    QToolButton,
    QTreeWidget,
    QTreeWidgetItem,
    QTreeWidgetItemIterator,
    QVBoxLayout,
    QWidget,
)

from ..i18n import N_, tr
from ..timecode import TICKS_PER_SECOND
from .timeline import ITEM_MIME

ITEM_ROLE = Qt.ItemDataRole.UserRole
BIN_ROLE = Qt.ItemDataRole.UserRole + 1
TYPE_ROLE = Qt.ItemDataRole.UserRole + 2


def _duration(ticks: Any) -> str:
    try:
        secs = int(ticks) / TICKS_PER_SECOND
    except (TypeError, ValueError):
        return ""
    if secs <= 0:
        return ""
    m, s = divmod(secs, 60)
    h, m = divmod(int(m), 60)
    return f"{h:d}:{m:02d}:{s:05.2f}" if h else f"{m:d}:{s:05.2f}"


class ProjectTree(QTreeWidget):
    def mimeData(self, items: list[QTreeWidgetItem]) -> QMimeData:
        mime = QMimeData()
        ids = [str(i.data(0, ITEM_ROLE)) for i in items if i.data(0, ITEM_ROLE) is not None]
        mime.setData(ITEM_MIME, ",".join(ids).encode())
        return mime

    def mimeTypes(self) -> list[str]:
        return [ITEM_MIME]


class ProjectPanel(QWidget):
    command = Signal(str, dict)
    invoke = Signal(str)  # a command id to run through the GUI's dialog path (import, new bin, ...)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.Project")
        self._updating = False
        self.filter = QLineEdit(self)
        self.filter.setObjectName("project.search")
        self.filter.setPlaceholderText(tr("Search"))
        self.filter.textChanged.connect(self._apply_filter)
        self.tree = ProjectTree(self)
        self.tree.setObjectName("project.tree")
        self.tree.setHeaderLabels([tr("Name"), tr("Type"), tr("Duration")])
        self.tree.setSelectionMode(QAbstractItemView.SelectionMode.ExtendedSelection)
        self.tree.setDragEnabled(True)
        self.tree.setDragDropMode(QAbstractItemView.DragDropMode.DragOnly)
        self.tree.itemDoubleClicked.connect(self._open)
        self.tree.itemSelectionChanged.connect(self._selection_changed)
        self.tree.setColumnWidth(0, 200)
        buttons = QHBoxLayout()
        for cid, text in (
            ("file.import", N_("Import…")),
            ("file.newBin", N_("New Bin")),
            ("file.newSequence", N_("New Sequence…")),
        ):
            b = QToolButton(self)
            b.setObjectName(f"project.button.{cid}")
            b.setText(tr(text))
            b.clicked.connect(lambda _c=False, c=cid: self.invoke.emit(c))
            buttons.addWidget(b)
        buttons.addStretch(1)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(2, 2, 2, 2)
        layout.addWidget(self.filter)
        layout.addWidget(self.tree, 1)
        layout.addLayout(buttons)

    def set_project(self, project: dict[str, Any] | None) -> None:
        self._updating = True
        expanded = {
            it.data(0, BIN_ROLE) for it in self._all_items() if it.data(0, BIN_ROLE) is not None and it.isExpanded()
        }
        selected = {it.data(0, ITEM_ROLE) for it in self.tree.selectedItems()}
        self.tree.clear()
        root = (project or {}).get("root") or {}
        active = (project or {}).get("activeSequence")

        def add(parent: QTreeWidgetItem | None, node: dict[str, Any]) -> None:
            for child in node.get("children") or []:
                if "item" in child:
                    it = QTreeWidgetItem(
                        [
                            str(child.get("name") or ""),
                            tr(str(child.get("type") or "")),
                            _duration(child.get("duration")),
                        ]
                    )
                    it.setData(0, ITEM_ROLE, child.get("item"))
                    it.setData(0, TYPE_ROLE, child.get("type"))
                    if child.get("item") == active:
                        f = it.font(0)
                        f.setBold(True)
                        it.setFont(0, f)
                    it.setFlags(it.flags() | Qt.ItemFlag.ItemIsDragEnabled)
                else:
                    it = QTreeWidgetItem([str(child.get("name") or ""), tr("Bin"), ""])
                    it.setData(0, BIN_ROLE, child.get("id"))
                    it.setFlags(it.flags() & ~Qt.ItemFlag.ItemIsDragEnabled)
                    add(it, child)
                if parent is None:
                    self.tree.addTopLevelItem(it)
                else:
                    parent.addChild(it)
                if it.data(0, BIN_ROLE) in expanded or not expanded:
                    it.setExpanded(True)
                if it.data(0, ITEM_ROLE) in selected:
                    reselect.append(it)

        # selected only once in the tree: an item in a bin is built before its bin is added, and
        # selecting an item outside a tree does nothing (the selection was lost on every refresh)
        reselect: list[QTreeWidgetItem] = []
        add(None, root)
        for it in reselect:
            it.setSelected(True)
        self._apply_filter(self.filter.text())
        self._updating = False

    def _all_items(self) -> list[QTreeWidgetItem]:
        out = []
        it = QTreeWidgetItemIterator(self.tree)
        while it.value():
            out.append(it.value())
            it += 1
        return out

    def _apply_filter(self, text: str) -> None:
        needle = text.strip().lower()
        for it in self._all_items():
            if it.data(0, ITEM_ROLE) is None:
                continue
            it.setHidden(bool(needle) and needle not in it.text(0).lower())

    def _open(self, item: QTreeWidgetItem, _col: int) -> None:
        iid = item.data(0, ITEM_ROLE)
        if iid is None:
            return
        if str(item.data(0, TYPE_ROLE) or "").lower() == "sequence":
            self.command.emit("sequence.open", {"item": iid})
        else:
            self.command.emit("source.open", {"item": iid})

    def selected_items(self) -> list[int]:
        return [it.data(0, ITEM_ROLE) for it in self.tree.selectedItems() if it.data(0, ITEM_ROLE) is not None]

    def _selection_changed(self) -> None:
        if self._updating:
            return
        items = [it.data(0, ITEM_ROLE) for it in self.tree.selectedItems() if it.data(0, ITEM_ROLE) is not None]
        self.command.emit("project.select", {"items": items})
