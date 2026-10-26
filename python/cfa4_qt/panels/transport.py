"""A monitor's transport row, as egui's (crates/ui-egui/src/panels/monitor.rs ``transport``):
the buttons centred, a Button Editor (+) at the right to hide or show each one, and the buttons
that don't fit in a "More transport controls" menu."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass

from PySide6.QtCore import QEvent, QPointF, QRectF, QSize, Qt, Signal
from PySide6.QtGui import QAction, QColor, QIcon, QPainter, QPainterPath, QPalette, QPen, QPixmap, QPolygonF
from PySide6.QtWidgets import QMenu, QToolButton, QWidget

from ..i18n import N_, tr

BUTTON = 28  # egui's TRANSPORT_BUTTON
ICON = 18
ICON_COLOR = QColor(0xC8, 0xC8, 0xC8)
ACCENT = QColor(0x40, 0x96, 0xF3)


@dataclass(frozen=True)
class TransportDef:
    key: str  # widget name (``<monitor>.transport.<key>``)
    command: str  # egui's command id (the Button Editor remembers hidden ones by it)
    icon: str
    label: str  # with its key, as egui's tooltip


PROGRAM_BUTTONS: tuple[TransportDef, ...] = (
    TransportDef("marker", "markers.add", "marker", N_("Add Marker (M)")),
    TransportDef("markIn", "markers.markIn", "markIn", N_("Mark In (I)")),
    TransportDef("markOut", "markers.markOut", "markOut", N_("Mark Out (O)")),
    TransportDef("goIn", "markers.goToIn", "goIn", N_("Go to In (Shift+I)")),
    TransportDef("stepBack", "playhead.stepBack", "stepBack", N_("Step Back 1 Frame (Left)")),
    TransportDef("toggle", "playback.toggle", "play", N_("Play-Stop Toggle (Space)")),
    TransportDef("stepForward", "playhead.stepForward", "stepForward", N_("Step Forward 1 Frame (Right)")),
    TransportDef("goOut", "markers.goToOut", "goOut", N_("Go to Out (Shift+O)")),
    TransportDef("lift", "sequence.lift", "lift", N_("Lift (;)")),
    TransportDef("extract", "sequence.extract", "extract", N_("Extract (')")),
    TransportDef("exportFrame", "exportFrame", "camera", N_("Export Frame")),
    TransportDef("proxies", "media.toggleProxies", "proxy", N_("Toggle Proxies")),
    TransportDef("loop", "playback.loop", "loop", N_("Loop")),
)
SOURCE_BUTTONS: tuple[TransportDef, ...] = (
    TransportDef("marker", "markers.add", "marker", N_("Add Marker (M)")),
    TransportDef("markIn", "src.markIn", "markIn", N_("Mark In (I)")),
    TransportDef("markOut", "src.markOut", "markOut", N_("Mark Out (O)")),
    TransportDef("goIn", "src.goIn", "goIn", N_("Go to In (Shift+I)")),
    TransportDef("stepBack", "src.stepBack", "stepBack", N_("Step Back 1 Frame (Left)")),
    TransportDef("toggle", "source.playback.toggle", "play", N_("Play-Stop Toggle (Space)")),
    TransportDef("stepForward", "src.stepFwd", "stepForward", N_("Step Forward 1 Frame (Right)")),
    TransportDef("goOut", "src.goOut", "goOut", N_("Go to Out (Shift+O)")),
    TransportDef("insert", "source.insert", "insert", N_("Insert (,)")),
    TransportDef("overwrite", "source.overwrite", "overwrite", N_("Overwrite (.)")),
    TransportDef("exportFrame", "exportFrame", "camera", N_("Export Frame")),
    TransportDef("proxies", "media.toggleProxies", "proxy", N_("Toggle Proxies")),
)


def _icon_path(name: str) -> tuple[QPainterPath, QPainterPath]:
    """(outline, filled) paths of an 18×18 icon."""
    line, fill = QPainterPath(), QPainterPath()

    def poly(*pts: tuple[float, float], closed: bool = True, filled: bool = False) -> None:
        target = fill if filled else line
        q = QPolygonF([QPointF(x, y) for x, y in pts])
        if closed:
            target.addPolygon(q)
            target.closeSubpath()
        else:
            target.moveTo(q[0])
            for p in list(q)[1:]:
                target.lineTo(p)

    def bracket(x: float, right: bool) -> None:
        d = -4.0 if right else 4.0
        poly((x + d, 3), (x, 3), (x, 15), (x + d, 15), closed=False)

    if name == "marker":
        poly((4, 3), (14, 3), (14, 11), (9, 15.5), (4, 11), filled=True)
    elif name == "markIn":
        bracket(6, False)
    elif name == "markOut":
        bracket(12, True)
    elif name == "goIn":
        bracket(3, False)
        poly((15, 4.5), (15, 13.5), (8, 9), filled=True)
    elif name == "goOut":
        bracket(15, True)
        poly((3, 4.5), (3, 13.5), (10, 9), filled=True)
    elif name == "stepBack":
        fill.addRect(QRectF(3.5, 4, 2, 10))
        poly((14.5, 4), (14.5, 14), (7, 9), filled=True)
    elif name == "stepForward":
        fill.addRect(QRectF(12.5, 4, 2, 10))
        poly((3.5, 4), (3.5, 14), (11, 9), filled=True)
    elif name == "play":
        poly((5, 3), (5, 15), (15, 9), filled=True)
    elif name == "pause":
        fill.addRect(QRectF(4.5, 3.5, 3, 11))
        fill.addRect(QRectF(10.5, 3.5, 3, 11))
    elif name == "lift":
        poly((2, 12), (2, 15), (16, 15), (16, 12), closed=False)
        poly((9, 11), (9, 3), closed=False)
        poly((5.5, 6.5), (9, 3), (12.5, 6.5), closed=False)
    elif name == "extract":
        poly((2, 12), (2, 15), (6, 15), closed=False)
        poly((12, 15), (16, 15), (16, 12), closed=False)
        poly((9, 13), (9, 3), closed=False)
        poly((5.5, 6.5), (9, 3), (12.5, 6.5), closed=False)
    elif name == "insert":
        poly((2, 11), (2, 15), (6, 15), closed=False)
        poly((12, 15), (16, 15), (16, 11), closed=False)
        poly((9, 3), (9, 12), closed=False)
        poly((5.5, 8.5), (9, 12), (12.5, 8.5), closed=False)
    elif name == "overwrite":
        fill.addRect(QRectF(2, 13, 14, 2.5))
        poly((9, 2.5), (9, 10.5), closed=False)
        poly((5.5, 7), (9, 10.5), (12.5, 7), closed=False)
    elif name == "camera":
        line.addRoundedRect(QRectF(2, 5.5, 14, 9.5), 1.5, 1.5)
        line.addEllipse(QPointF(9, 10.2), 2.8, 2.8)
        poly((6, 5.5), (7.5, 3.5), (10.5, 3.5), (12, 5.5), closed=False)
    elif name == "proxy":
        line.addRoundedRect(QRectF(2.5, 3, 13, 12), 2, 2)
        poly((7, 12.5), (7, 5.5), (10, 5.5), (11.5, 6.8), (10, 8.8), (7, 8.8), closed=False)
    elif name == "loop":
        line.moveTo(14.5, 9)
        line.arcTo(QRectF(3.5, 3.5, 11, 11), 0, 270)
        poly((14.5, 5.2), (14.5, 9.2), (11, 7.6), filled=True)
    elif name == "wrench":
        line.addEllipse(QPointF(12, 6), 3.2, 3.2)
        poly((9.7, 8.3), (3, 15), closed=False)
    elif name == "plus":
        poly((9, 3.5), (9, 14.5), closed=False)
        poly((3.5, 9), (14.5, 9), closed=False)
    return line, fill


def icon_color(widget: QWidget) -> QColor:
    """Icons in the theme's button text colour (light on a dark theme, dark on a light one)."""
    c = widget.palette().color(QPalette.ColorRole.ButtonText)
    return c if c.isValid() else ICON_COLOR


def transport_icon(name: str, color: QColor = ICON_COLOR, size: int = ICON) -> QIcon:
    pm = QPixmap(size * 2, size * 2)
    pm.fill(Qt.GlobalColor.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.RenderHint.Antialiasing)
    p.scale(size * 2 / 18.0, size * 2 / 18.0)
    line, fill = _icon_path(name)
    p.setPen(QPen(color, 1.5, Qt.PenStyle.SolidLine, Qt.PenCapStyle.RoundCap, Qt.PenJoinStyle.RoundJoin))
    p.drawPath(line)
    p.setPen(Qt.PenStyle.NoPen)
    p.setBrush(color)
    p.drawPath(fill)
    p.end()
    pm.setDevicePixelRatio(2.0)
    return QIcon(pm)


class TransportBar(QWidget):
    """The buttons, laid out by hand as egui does: centred with 6 px gaps when they all fit, 2 px
    before any goes to the overflow menu."""

    triggered = Signal(str)  # command
    hidden_changed = Signal(list)

    def __init__(self, prefix: str, defs: tuple[TransportDef, ...], hidden: list[str], parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName(f"{prefix}.transport")
        self.setFixedHeight(BUTTON + 6)
        self.prefix = prefix
        self.defs = defs
        self.hidden = list(hidden)
        self.buttons: dict[str, QToolButton] = {}
        self.icons: dict[str, tuple[str, QColor | None]] = {}  # icons set since (play / pause…)
        self.checked: dict[str, Callable[[], bool]] = {}
        for d in defs:
            b = QToolButton(self)
            b.setObjectName(f"{prefix}.transport.{d.key}")
            b.setToolTip(tr(d.label))
            b.setAccessibleName(tr(d.label))
            b.setIcon(transport_icon(d.icon, icon_color(self)))
            b.setIconSize(QSize(20 if d.key == "toggle" else ICON, 20 if d.key == "toggle" else ICON))
            b.setFixedSize(BUTTON, BUTTON)
            b.setAutoRaise(True)
            b.setFocusPolicy(Qt.FocusPolicy.NoFocus)
            b.clicked.connect(lambda _c=False, c=d.command: self.triggered.emit(c))
            self.buttons[d.key] = b
        self.more = QToolButton(self)
        self.more.setObjectName(f"{prefix}.transport.more")
        self.more.setText("⋯")
        self.more.setToolTip(tr("More transport controls"))
        self.more.setFixedSize(BUTTON, BUTTON)
        self.more.setAutoRaise(True)
        self.more.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        self.more.setPopupMode(QToolButton.ToolButtonPopupMode.InstantPopup)
        self.more_menu = QMenu(self.more)
        self.more.setMenu(self.more_menu)
        self.editor = QToolButton(self)
        self.editor.setObjectName(f"{prefix}.transport.editor")
        self.editor.setIcon(transport_icon("plus", QColor(0x8C, 0x8C, 0x8C)))
        self.editor.setToolTip(tr("Button Editor"))
        self.editor.setFixedSize(22, 22)
        self.editor.setAutoRaise(True)
        self.editor.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        self.editor.setPopupMode(QToolButton.ToolButtonPopupMode.InstantPopup)
        self.editor_menu = QMenu(self.editor)
        self.editor_menu.aboutToShow.connect(self._fill_editor)
        self.editor.setMenu(self.editor_menu)
        self.overflow: list[TransportDef] = []

    def set_icon(self, key: str, icon: str, color: QColor | None = None) -> None:
        if key in self.buttons:
            self.icons[key] = (icon, color)
            self.buttons[key].setIcon(transport_icon(icon, color or icon_color(self)))

    def changeEvent(self, e) -> None:
        super().changeEvent(e)
        if e.type() == QEvent.Type.PaletteChange and hasattr(self, "editor"):
            for d in self.defs:
                icon, color = self.icons.get(d.key, (d.icon, None))
                self.buttons[d.key].setIcon(transport_icon(icon, color or icon_color(self)))
            self.editor.setIcon(transport_icon("plus", icon_color(self)))

    def shown(self) -> list[TransportDef]:
        return [d for d in self.defs if d.command not in self.hidden]

    def set_hidden(self, hidden: list[str]) -> None:
        self.hidden = list(hidden)
        self._layout()
        self.hidden_changed.emit(list(self.hidden))

    def _fill_editor(self) -> None:
        m = self.editor_menu
        m.clear()
        for d in self.defs:
            a = QAction(tr(d.label), m, checkable=True)
            a.setObjectName(f"{self.prefix}.transport.editor.{d.command}")
            a.setChecked(d.command not in self.hidden)
            a.toggled.connect(lambda on, c=d.command: self.set_hidden([h for h in self.hidden if h != c] + ([] if on else [c])))
            m.addAction(a)
        m.addSeparator()
        reset = m.addAction(tr("Reset"))
        reset.setObjectName(f"{self.prefix}.transport.editor.reset")
        from .monitor_view import DEFAULT_HIDDEN

        reset.triggered.connect(lambda: self.set_hidden(list(DEFAULT_HIDDEN)))

    def resizeEvent(self, e) -> None:
        super().resizeEvent(e)
        self._layout()

    def showEvent(self, e) -> None:
        super().showEvent(e)
        self._layout()

    def _layout(self) -> None:
        shown = self.shown()
        width = self.width()
        self.editor.move(max(0, width - 30), (self.height() - 22) // 2)
        self.editor.setVisible(width >= 30)
        available = max(0.0, width - 46.0)
        n = len(shown)
        gap = 6 if n * BUTTON + max(0, n - 1) * 6 <= available else 2
        fits = n
        while fits > 0 and fits * BUTTON + (fits - 1) * gap + (0 if fits == n else BUTTON + gap) > available:
            fits -= 1
        visible, self.overflow = shown[:fits], shown[fits:]
        total = len(visible) * (BUTTON + gap) - gap + ((BUTTON + gap) if self.overflow else 0)
        x = int(width / 2 - 15 - max(total, BUTTON) / 2)
        y = (self.height() - BUTTON) // 2
        keep = {d.key for d in visible}
        for d in self.defs:
            b = self.buttons[d.key]
            b.setVisible(d.key in keep)
        for d in visible:
            self.buttons[d.key].move(x, y)
            x += BUTTON + gap
        self.more.setVisible(bool(self.overflow))
        if self.overflow:
            self.more.move(x, y)
            self.more_menu.clear()
            for d in self.overflow:
                a = self.more_menu.addAction(transport_icon(d.icon, icon_color(self)), tr(d.label))
                a.setObjectName(f"{self.prefix}.transport.more.{d.command}")
                if d.command in self.checked:
                    a.setCheckable(True)
                    a.setChecked(self.checked[d.command]())
                a.triggered.connect(lambda _c=False, c=d.command: self.triggered.emit(c))
