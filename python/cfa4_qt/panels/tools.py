"""Tools panel: the egui editor's 17 tools in its 8 groups, with its keys.

A group shows its current tool; the others of the group are in a flyout (right-click, or hold
the button). The tool decides what a drag on the timeline does; every edit it makes is an
engine command (see ``TimelineCanvas``). Tools that draw on the Program monitor or edit
keyframes there (Pen, Rectangle, Ellipse, Type) and Remix are not in the Qt GUI yet: they are
listed disabled, so the gap shows (docs/qt-parity.md).
"""

from __future__ import annotations

from dataclasses import dataclass

from PySide6.QtCore import QEvent, QPointF, QRectF, QSize, Qt, Signal
from PySide6.QtGui import QAction, QColor, QIcon, QPainter, QPainterPath, QPen, QPixmap, QPolygonF
from PySide6.QtWidgets import QMenu, QSizePolicy, QToolButton, QVBoxLayout, QWidget

from ..i18n import N_, tr


@dataclass(frozen=True)
class ToolDef:
    id: str
    label: str
    key: str  # egui's key (crates/ui-egui/src/state.rs Tool::shortcut)
    available: bool = True


# egui's groups (Tool::groups), in order
GROUPS: tuple[tuple[ToolDef, ...], ...] = (
    (ToolDef("selection", N_("Selection Tool"), "V"),),
    (ToolDef("trackSelectForward", N_("Track Select Forward Tool"), "A"), ToolDef("trackSelectBackward", N_("Track Select Backward Tool"), "Shift+A")),
    (
        ToolDef("ripple", N_("Ripple Edit Tool"), "B"),
        ToolDef("rolling", N_("Rolling Edit Tool"), "N"),
        ToolDef("rateStretch", N_("Rate Stretch Tool"), "R"),
        ToolDef("remix", N_("Remix Tool"), "", available=False),
    ),
    (ToolDef("razor", N_("Razor Tool"), "C"),),
    (ToolDef("slip", N_("Slip Tool"), "Y"), ToolDef("slide", N_("Slide Tool"), "U")),
    (
        ToolDef("pen", N_("Pen Tool"), "P", available=False),
        ToolDef("rectangle", N_("Rectangle Tool"), "", available=False),
        ToolDef("ellipse", N_("Ellipse Tool"), "", available=False),
    ),
    (ToolDef("hand", N_("Hand Tool"), "H"), ToolDef("zoom", N_("Zoom Tool"), "Z")),
    (ToolDef("type", N_("Type Tool"), "T", available=False), ToolDef("verticalType", N_("Vertical Type Tool"), "", available=False)),
)
TOOLS = {t.id: t for g in GROUPS for t in g}
NOT_YET = N_("Not yet in the Qt GUI (see docs/qt-parity.md)")


def _paint(tool: str, p: QPainter, s: float) -> None:
    """Simple line icons on an s × s canvas (original drawings)."""
    def pt(x: float, y: float) -> QPointF:
        return QPointF(x * s, y * s)

    def arrow(x0: float, x1: float, y: float) -> None:
        p.drawLine(pt(x0, y), pt(x1, y))
        d = 0.12 if x1 > x0 else -0.12
        p.drawLine(pt(x1, y), pt(x1 - d, y - 0.1))
        p.drawLine(pt(x1, y), pt(x1 - d, y + 0.1))

    if tool == "selection":
        p.drawPolygon(QPolygonF([pt(0.3, 0.15), pt(0.3, 0.8), pt(0.45, 0.65), pt(0.58, 0.88), pt(0.68, 0.83), pt(0.55, 0.6), pt(0.75, 0.6)]))
    elif tool in ("trackSelectForward", "trackSelectBackward"):
        fwd = tool == "trackSelectForward"
        for y in (0.3, 0.5, 0.7):
            p.drawLine(pt(0.45 if fwd else 0.2, y), pt(0.8 if fwd else 0.55, y))
        x = 0.25 if fwd else 0.75
        p.drawPolygon(QPolygonF([pt(x, 0.4), pt(x + (0.15 if fwd else -0.15), 0.5), pt(x, 0.6)]))
    elif tool == "ripple":
        p.drawLine(pt(0.5, 0.2), pt(0.5, 0.8))
        p.drawLine(pt(0.5, 0.2), pt(0.62, 0.2))
        p.drawLine(pt(0.5, 0.8), pt(0.62, 0.8))
        arrow(0.45, 0.15, 0.5)
        arrow(0.55, 0.85, 0.5)
    elif tool == "rolling":
        p.drawLine(pt(0.45, 0.2), pt(0.45, 0.8))
        p.drawLine(pt(0.55, 0.2), pt(0.55, 0.8))
        arrow(0.4, 0.12, 0.5)
        arrow(0.6, 0.88, 0.5)
    elif tool == "rateStretch":
        p.drawRect(QRectF(pt(0.3, 0.3), pt(0.7, 0.7)))
        arrow(0.3, 0.1, 0.5)
        arrow(0.7, 0.9, 0.5)
    elif tool == "remix":
        path = QPainterPath(pt(0.15, 0.5))
        for i, x in enumerate((0.3, 0.45, 0.6, 0.75, 0.85)):
            path.lineTo(pt(x, 0.3 if i % 2 == 0 else 0.7))
        p.drawPath(path)
    elif tool == "razor":
        p.drawLine(pt(0.25, 0.8), pt(0.7, 0.2))
        p.drawPolygon(QPolygonF([pt(0.55, 0.25), pt(0.75, 0.15), pt(0.85, 0.3), pt(0.65, 0.4)]))
    elif tool in ("slip", "slide"):
        p.drawRect(QRectF(pt(0.3, 0.3), pt(0.7, 0.7)))
        if tool == "slip":
            arrow(0.38, 0.62, 0.5)
        else:
            arrow(0.25, 0.05, 0.5)
            arrow(0.75, 0.95, 0.5)
    elif tool == "pen":
        p.drawPolygon(QPolygonF([pt(0.5, 0.15), pt(0.68, 0.55), pt(0.5, 0.85), pt(0.32, 0.55)]))
        p.drawLine(pt(0.5, 0.15), pt(0.5, 0.55))
    elif tool == "rectangle":
        p.drawRect(QRectF(pt(0.2, 0.3), pt(0.8, 0.7)))
    elif tool == "ellipse":
        p.drawEllipse(QRectF(pt(0.2, 0.3), pt(0.8, 0.7)))
    elif tool == "hand":
        for i, x in enumerate((0.32, 0.44, 0.56, 0.68)):
            top = 0.25 if i in (1, 2) else 0.32
            p.drawLine(pt(x, top), pt(x, 0.6))
        p.drawArc(QRectF(pt(0.3, 0.45), pt(0.7, 0.85)), 180 * 16, 180 * 16)
    elif tool == "zoom":
        p.drawEllipse(QRectF(pt(0.2, 0.2), pt(0.6, 0.6)))
        p.drawLine(pt(0.55, 0.55), pt(0.82, 0.82))
        p.drawLine(pt(0.32, 0.4), pt(0.48, 0.4))
        p.drawLine(pt(0.4, 0.32), pt(0.4, 0.48))
    elif tool in ("type", "verticalType"):
        if tool == "type":
            p.drawLine(pt(0.25, 0.25), pt(0.75, 0.25))
            p.drawLine(pt(0.5, 0.25), pt(0.5, 0.8))
        else:
            p.drawLine(pt(0.3, 0.25), pt(0.3, 0.75))
            p.drawLine(pt(0.3, 0.5), pt(0.75, 0.5))


def tool_icon(tool: str, color: QColor, size: int = 32) -> QIcon:
    pm = QPixmap(size, size)
    pm.fill(Qt.GlobalColor.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.RenderHint.Antialiasing)
    p.setPen(QPen(color, max(1.5, size / 16)))
    _paint(tool, p, size)
    p.end()
    return QIcon(pm)


class ToolsPanel(QWidget):
    """A narrow vertical strip, as egui's Tools panel."""

    tool_selected = Signal(str)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.Tools")
        self.current = "selection"
        self.group_tool = [g[0].id for g in GROUPS]  # the tool each group button shows
        self.buttons: list[QToolButton] = []
        self.actions: dict[str, QAction] = {}
        color = self.palette().buttonText().color()
        layout = QVBoxLayout(self)
        layout.setContentsMargins(2, 4, 2, 4)
        layout.setSpacing(2)
        for gi, group in enumerate(GROUPS):
            b = QToolButton(self)
            b.setObjectName(f"tools.group.{gi}")
            b.setCheckable(True)
            b.setAutoRaise(True)
            b.setIconSize(QSize(22, 22))
            b.setSizePolicy(QSizePolicy.Policy.Fixed, QSizePolicy.Policy.Fixed)
            if len(group) > 1:
                menu = QMenu(b)
                menu.setToolTipsVisible(True)
                for t in group:
                    menu.addAction(self._action(t, color))
                b.setMenu(menu)
                b.setPopupMode(QToolButton.ToolButtonPopupMode.DelayedPopup)
                b.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
                b.customContextMenuRequested.connect(lambda _p, btn=b: btn.showMenu())
            else:
                self._action(group[0], color)
            b.clicked.connect(lambda _c=False, i=gi: self.select(self.group_tool[i]))
            layout.addWidget(b, 0, Qt.AlignmentFlag.AlignHCenter)
            self.buttons.append(b)
        layout.addStretch(1)
        self.setMinimumWidth(36)
        self._refresh()

    def _action(self, t: ToolDef, color: QColor) -> QAction:
        text = tr(t.label) + (f" ({t.key})" if t.key else "")
        a = QAction(tool_icon(t.id, color), text, self)
        a.setObjectName(f"tools.{t.id}")
        a.setCheckable(True)
        if t.available:
            a.triggered.connect(lambda _c=False, i=t.id: self.select(i))
        else:
            a.setEnabled(False)
            a.setToolTip(tr(NOT_YET))
        self.actions[t.id] = a
        return a

    def shortcut_pairs(self) -> list[tuple[QAction, str]]:
        """(action, key) for the window to bind where no engine command uses the key."""
        return [(self.actions[t.id], t.key) for g in GROUPS for t in g if t.key and t.available]

    def select(self, tool: str) -> None:
        t = TOOLS.get(tool)
        if t is None or not t.available:
            return
        self.current = tool
        for gi, group in enumerate(GROUPS):
            if any(x.id == tool for x in group):
                self.group_tool[gi] = tool
        self._refresh()
        self.tool_selected.emit(tool)

    def changeEvent(self, e) -> None:
        super().changeEvent(e)
        if e.type() == QEvent.Type.PaletteChange and self.buttons:
            self._refresh()

    def _refresh(self) -> None:
        color = self.palette().buttonText().color()
        for tid, a in self.actions.items():
            a.setIcon(tool_icon(tid, color))
        for gi, b in enumerate(self.buttons):
            t = TOOLS[self.group_tool[gi]]
            b.setIcon(tool_icon(t.id, color))
            b.setToolTip(tr(t.label) + (f" ({t.key})" if t.key else "") + ("" if len(GROUPS[gi]) == 1 else "  ▸"))
            b.setChecked(t.id == self.current)
            b.setEnabled(any(x.available for x in GROUPS[gi]))
        for tid, a in self.actions.items():
            a.setChecked(tid == self.current)
