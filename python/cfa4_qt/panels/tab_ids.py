"""A panel's own tab bar with egui's tab ids for the control channel."""

from __future__ import annotations

from collections.abc import Sequence

from PySide6.QtCore import QRect
from PySide6.QtWidgets import QTabBar, QWidget


class IdTabBar(QTabBar):
    """A ``QTabBar`` whose tabs are painted parts with egui's ids (``text.tab.Captions``,
    ``essentialGraphics.tab.browse``, …): ``ui.elements`` lists them and ``ui.click`` picks one."""

    def __init__(self, ids: Sequence[str], parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.tab_ids = list(ids)

    def automation_parts(self) -> list[tuple[str, QRect, str]]:
        out: list[tuple[str, QRect, str]] = []
        for i, pid in enumerate(self.tab_ids[: self.count()]):
            r = self.tabRect(i)
            if self.isTabVisible(i) and not r.isEmpty():
                out.append((pid, r, self.tabText(i)))
        return out


__all__ = ["IdTabBar"]
