"""egui's dock gutters (``dock.rs`` ``layout_inner``): the divider of every split of the panel
layout, ``dock.gutter.<path>`` on the control channel, where the path is egui's: empty for the
outermost split, then ``a`` / ``b`` for the first / second side of each split inside it.

Qt's own dock separators are not widgets, so a :class:`DockGutter` lies over each one. The splits
are read back from the window as a guillotine cut of the visible panels (tabbed panels are one
group; the central widget, the Program monitor, is a group too): the first cut that no panel
crosses is the outermost split, its left / top side ``a``. Dragging a gutter or an arrow key while
it has focus (8 px a press, as egui's ``ResizeHandle``) moves the split, each side keeping 40 px,
through ``QMainWindow.resizeDocks``.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import TYPE_CHECKING

from PySide6.QtCore import QEvent, QObject, QPoint, QRect, Qt, QTimer
from PySide6.QtGui import QColor, QKeyEvent, QMouseEvent, QPainter, QPaintEvent
from PySide6.QtWidgets import QDockWidget, QTabBar, QWidget

from . import theme
from .i18n import tr

if TYPE_CHECKING:
    from PySide6.QtWidgets import QMainWindow

KEY_STEP = 8  # egui's ResizeHandle keyboard step
MIN_SIDE = 40  # egui's split.resized(.., 40.0)
HIT_MARGIN = 3  # egui's hit margin along the movement axis


@dataclass
class Leaf:
    rect: QRect
    dock: QDockWidget | None  # None: the central widget


@dataclass
class Split:
    path: str
    vertical: bool  # egui's: True when the sides are above and below each other
    gap: QRect  # the divider between the two sides
    a: list[Leaf] = field(default_factory=list)
    b: list[Leaf] = field(default_factory=list)


def _cut(leaves: list[Leaf], vertical: bool) -> tuple[list[Leaf], list[Leaf]] | None:
    """The first cut across ``leaves`` that no leaf crosses: left / right (``vertical`` False)
    or top / bottom."""

    def lo(r: QRect) -> int:
        return r.top() if vertical else r.left()

    def hi(r: QRect) -> int:
        return r.bottom() if vertical else r.right()

    for at in sorted({hi(leaf.rect) for leaf in leaves}):
        a = [leaf for leaf in leaves if hi(leaf.rect) <= at]
        b = [leaf for leaf in leaves if lo(leaf.rect) > at]
        if a and b and len(a) + len(b) == len(leaves):
            return a, b
    return None


def splits(leaves: list[Leaf], path: str = "") -> list[Split]:
    """egui's splits of the layout ``leaves`` make, outermost first."""
    if len(leaves) < 2:
        return []
    for vertical in (False, True):
        cut = _cut(leaves, vertical)
        if cut is None:
            continue
        a, b = cut
        ua, ub = _union(a), _union(b)
        both = ua.united(ub)
        if vertical:
            gap = QRect(both.left(), ua.bottom() + 1, both.width(), max(1, ub.top() - ua.bottom() - 1))
        else:
            gap = QRect(ua.right() + 1, both.top(), max(1, ub.left() - ua.right() - 1), both.height())
        return [Split(path, vertical, gap, a, b), *splits(a, path + "a"), *splits(b, path + "b")]
    return []


def _union(leaves: list[Leaf]) -> QRect:
    r = QRect(leaves[0].rect)
    for leaf in leaves[1:]:
        r = r.united(leaf.rect)
    return r


def layout_leaves(win: QMainWindow) -> list[Leaf]:
    """The visible docked panels (one per tab group, with the group's tab bar above it) and the
    central widget, in window coordinates."""
    out: list[Leaf] = []
    seen: set[tuple[int, int, int, int]] = set()
    inside = win.rect()
    central = win.centralWidget()
    if central is not None and central.isVisible() and central.width() > 0 and central.height() > 0:
        out.append(Leaf(QRect(central.mapTo(win, QPoint(0, 0)), central.size()), None))
        seen.add(out[-1].rect.getRect())
    for dock in win.findChildren(QDockWidget):
        if (
            dock.parentWidget() is not win
            or dock.isFloating()
            or not dock.isVisible()
            or dock.width() <= 0
            or dock.height() <= 0
        ):
            continue
        r = QRect(dock.mapTo(win, QPoint(0, 0)), dock.size())
        if r.getRect() in seen or not inside.contains(r):
            continue  # a tab of a group already in, or a tab Qt parked outside the window
        seen.add(r.getRect())
        out.append(Leaf(r, dock))
    # a tab group's bar sits above its panel: it belongs to that panel's side of every split
    for bar in win.findChildren(QTabBar):
        if bar.parent() is not win or not bar.isVisible():
            continue
        b = bar.geometry()
        for leaf in out:
            if (
                leaf.dock is not None
                and 0 <= leaf.rect.top() - b.bottom() <= 1
                and leaf.rect.left() <= b.center().x() <= leaf.rect.right()
            ):
                leaf.rect = leaf.rect.united(b)
                break
    return out


class DockGutter(QWidget):
    """One split's divider, over Qt's separator."""

    def __init__(self, gutters: DockGutters, split: Split) -> None:
        super().__init__(gutters.win)
        self.gutters = gutters
        self.split = split
        self.setFocusPolicy(Qt.FocusPolicy.StrongFocus)
        self.setMouseTracking(True)
        self.setAttribute(Qt.WidgetAttribute.WA_Hover)
        self._drag: int | None = None
        self.place(split)

    def place(self, split: Split) -> None:
        self.split = split
        self.setObjectName(f"dock.gutter.{split.path}")
        self.setAccessibleName("gutter")
        self.setAccessibleDescription(tr("Height:") if split.vertical else tr("Width:"))
        self.setCursor(Qt.CursorShape.SplitVCursor if split.vertical else Qt.CursorShape.SplitHCursor)
        if self._drag is not None:
            return  # stays where the drag started, so the pointer's position stays meaningful
        g = split.gap
        if split.vertical:
            g = g.adjusted(0, -HIT_MARGIN, 0, HIT_MARGIN)
        else:
            g = g.adjusted(-HIT_MARGIN, 0, HIT_MARGIN, 0)
        self.setGeometry(g)
        self.show()
        self.raise_()
        self.update()

    def _along(self, e: QMouseEvent) -> int:
        p = e.position().toPoint()
        return p.y() if self.split.vertical else p.x()

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() == Qt.MouseButton.LeftButton:
            self.setFocus(Qt.FocusReason.MouseFocusReason)
            self._drag = self._along(e)
            self.update()

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        if self._drag is None:
            return
        at = self._along(e)
        if at != self._drag:
            self.gutters.move(self.split.path, at - self._drag)
            self._drag = at

    def mouseReleaseEvent(self, _e: QMouseEvent) -> None:
        if self._drag is None:
            return
        self._drag = None
        self.place(self.split)

    def _arrow(self, e: QKeyEvent) -> int:
        """The step an arrow along the split's axis moves it, 0 for any other key."""
        back, forward = (Qt.Key.Key_Up, Qt.Key.Key_Down) if self.split.vertical else (Qt.Key.Key_Left, Qt.Key.Key_Right)
        if e.modifiers() != Qt.KeyboardModifier.NoModifier or e.key() not in (back, forward):
            return 0
        return KEY_STEP if e.key() == forward else -KEY_STEP

    def keyPressEvent(self, e: QKeyEvent) -> None:
        step = self._arrow(e)
        if step:
            self.gutters.move(self.split.path, step)
            return
        super().keyPressEvent(e)

    def event(self, e: QEvent) -> bool:
        if e.type() == QEvent.Type.ShortcutOverride and isinstance(e, QKeyEvent) and self._arrow(e):
            e.accept()  # the arrows move the divider, not the playhead (egui's focus lock filter)
            return True
        if e.type() in (QEvent.Type.HoverEnter, QEvent.Type.HoverLeave, QEvent.Type.FocusIn, QEvent.Type.FocusOut):
            self.update()
        return super().event(e)

    def paintEvent(self, _e: QPaintEvent) -> None:
        # egui's ResizeHandleStyle: focus at 0.4 hovered, 0.9 dragged, full when focused; nothing otherwise
        if self._drag is not None:
            alpha = 0.9
        elif self.hasFocus():
            alpha = 1.0
        elif self.underMouse():
            alpha = 0.4
        else:
            return
        c = QColor(theme.current().c("focus"))
        c.setAlphaF(alpha)
        r = self.rect()
        r = (
            r.adjusted(0, HIT_MARGIN, 0, -HIT_MARGIN)
            if self.split.vertical
            else r.adjusted(HIT_MARGIN, 0, -HIT_MARGIN, 0)
        )
        QPainter(self).fillRect(r, c)


class DockGutters(QObject):
    """Keeps a :class:`DockGutter` over every split of ``win``'s panels."""

    def __init__(self, win: QMainWindow) -> None:
        super().__init__(win)
        self.win = win
        self.gutters: dict[str, DockGutter] = {}
        self._timer = QTimer(self)
        self._timer.setSingleShot(True)
        self._timer.setInterval(0)
        self._timer.timeout.connect(self.relayout)
        win.installEventFilter(self)
        for dock in win.findChildren(QDockWidget):
            self.watch(dock)

    def watch(self, w: QWidget) -> None:
        w.installEventFilter(self)

    def eventFilter(self, obj: QObject, e: QEvent) -> bool:
        if e.type() in (
            QEvent.Type.Resize,
            QEvent.Type.Move,
            QEvent.Type.Show,
            QEvent.Type.Hide,
            QEvent.Type.ChildAdded,
        ):
            if e.type() == QEvent.Type.ChildAdded and obj is self.win:
                child = e.child()
                if isinstance(child, QDockWidget):
                    self.watch(child)
            self._timer.start()
        return False

    def splits(self) -> list[Split]:
        return splits(layout_leaves(self.win))

    def relayout(self) -> None:
        now = {s.path: s for s in self.splits()}
        for path in list(self.gutters):
            if path not in now:
                self.gutters.pop(path).deleteLater()
        for path, s in now.items():
            g = self.gutters.get(path)
            if g is None:
                self.gutters[path] = DockGutter(self, s)
            else:
                g.place(s)

    def move(self, path: str, delta: int) -> None:
        """Move split ``path`` by ``delta`` px (right / down positive), each side keeping
        ``MIN_SIDE``."""
        s = next((x for x in self.splits() if x.path == path), None)
        if s is None or delta == 0:
            return
        ua, ub = _union(s.a), _union(s.b)
        if s.vertical:
            delta = max(-(ua.height() - MIN_SIDE), min(delta, ub.height() - MIN_SIDE))
            edge_a, edge_b = ua.bottom(), ub.top()
            size = lambda r: r.height()
            orient = Qt.Orientation.Vertical
            touches_a = lambda r: r.bottom() == edge_a
            touches_b = lambda r: r.top() == edge_b
        else:
            delta = max(-(ua.width() - MIN_SIDE), min(delta, ub.width() - MIN_SIDE))
            edge_a, edge_b = ua.right(), ub.left()
            size = lambda r: r.width()
            orient = Qt.Orientation.Horizontal
            touches_a = lambda r: r.right() == edge_a
            touches_b = lambda r: r.left() == edge_b
        if delta == 0:
            return
        # Qt sizes a panel within its dock area: a panel on side a grows, unless side a holds the
        # central widget (the Program monitor, which takes up what is left), when a panel on side b
        # shrinks instead; that is the side whose dock area meets the split
        if any(leaf.dock is None for leaf in s.a):
            pick = next((leaf for leaf in s.b if leaf.dock is not None and touches_b(leaf.rect)), None)
            want = -delta
        else:
            pick = next((leaf for leaf in s.a if leaf.dock is not None and touches_a(leaf.rect)), None)
            want = delta
        if pick is not None and pick.dock is not None:
            self.win.resizeDocks([pick.dock], [size(pick.rect) + want], orient)
            self.win.layout().activate()  # now, so the next step of a drag starts from it
        self.relayout()
