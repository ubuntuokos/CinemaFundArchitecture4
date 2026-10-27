"""egui's panel docking commands (``crates/ui-egui/src/dock_ui.rs``) on the Qt GUI's docks:
``window.closePanel {panel?}``, ``window.floatPanel {panel?, rect?}``,
``window.dockPanel {panel?, zone?, anchor?, before?}`` and ``window.dockAction {action}`` (the
shared docking transaction as JSON: ``{"Move": {panel, anchor, placement}}``, ``Float``,
``Close``, ``Activate``, ``Open``, ``OpenAt``, ``MoveFloating``), plus the panel menu egui opens on
a panel's tab (Close Panel, Undock Panel, Restore Workspace).

Panels are named as in egui (``EffectControls``, ``Effect Controls`` or ``effect_controls``);
without ``panel`` the focused panel is meant. The Program monitor is the Qt window's central
widget: other panels dock beside it, it does not move itself.
"""

from __future__ import annotations

import math
from collections.abc import Callable
from typing import TYPE_CHECKING, Any

from PySide6.QtCore import QPoint, QRect, Qt
from PySide6.QtWidgets import QDockWidget, QMainWindow, QMenu, QTabBar, QWidget

from .commands import focused_panel
from .i18n import N_, tr

if TYPE_CHECKING:
    from .main_window import MainWindow

COMMANDS = ("window.dockAction", "window.dockPanel", "window.floatPanel", "window.closePanel")
LABELS = {
    "window.dockAction": N_("Docking Action"),
    "window.dockPanel": N_("Dock Panel"),
    "window.floatPanel": N_("Undock Panel"),
    "window.closePanel": N_("Close Panel"),
}
PROGRAM = "Program"
ZONES = {
    "left": Qt.DockWidgetArea.LeftDockWidgetArea,
    "right": Qt.DockWidgetArea.RightDockWidgetArea,
    "top": Qt.DockWidgetArea.TopDockWidgetArea,
    "bottom": Qt.DockWidgetArea.BottomDockWidgetArea,
}
# a floating panel is at least this big (egui's floating_rect)
MIN_FLOAT = (160, 100)


class DockError(ValueError):
    """A docking command that cannot be done (unknown panel, bad parameter)."""


def _key(name: str) -> str:
    return name.lower().replace(" ", "").replace("_", "").replace("-", "")


def panel_title(win: MainWindow, name: Any) -> str:
    """The Qt dock title of an egui panel name (or ``Program``)."""
    if not isinstance(name, str) or not name:
        raise DockError("unknown panel")
    for title in [*win.docks, PROGRAM]:
        if _key(title) == _key(name):
            return title
    raise DockError(f"unknown panel `{name}`")


def panel_id(title: str) -> str:
    """egui's panel id (``PanelKind`` name) of a Qt dock title."""
    return title.replace(" ", "")


def _target(win: MainWindow, params: dict[str, Any]) -> str:
    if "panel" in params:
        return panel_title(win, params["panel"])
    focused = focused_panel()
    if focused is None:
        raise DockError("no panel has focus: name one with `panel`")
    return panel_title(win, focused)


def _dock(win: MainWindow, title: str) -> QDockWidget:
    dock = win.docks.get(title)
    if dock is None:
        raise DockError("the Program monitor is the Qt window's central monitor: dock other panels beside it")
    return dock


def _rect(value: Any) -> tuple[int, int, int, int]:
    if not isinstance(value, list) or len(value) != 4:
        raise DockError("rect must contain four finite numbers")
    out = []
    for v in value:
        if isinstance(v, bool) or not isinstance(v, (int, float)) or not math.isfinite(v) or abs(v) > 1e6:
            raise DockError("rect must contain four finite numbers")
        out.append(round(v))
    x, y, w, h = out
    return x, y, max(MIN_FLOAT[0], w), max(MIN_FLOAT[1], h)


def default_float_rect(win: MainWindow) -> list[float]:
    """egui's: 420 × 320 in the middle of the window."""
    c = win.rect().center()
    return [c.x() - 210.0, c.y() - 160.0, 420.0, 320.0]


def close_panel(win: MainWindow, title: str) -> None:
    _dock(win, title).close()


def float_panel(win: MainWindow, title: str, rect: Any = None) -> None:
    dock = _dock(win, title)
    x, y, w, h = _rect(rect if rect is not None else default_float_rect(win))
    dock.show()
    dock.setFloating(True)
    # rect is in the window's coordinates, as egui's are in its viewport
    top_left = win.mapToGlobal(QPoint(x, y))
    dock.setGeometry(QRect(top_left.x(), top_left.y(), w, h))
    dock.raise_()


def move_floating(win: MainWindow, title: str, rect: Any) -> None:
    dock = _dock(win, title)
    if not dock.isFloating():
        raise DockError(f"{title} is not floating")
    float_panel(win, title, rect)


def activate(win: MainWindow, title: str) -> None:
    if title == PROGRAM:
        win.monitor.setFocus()
        return
    dock = _dock(win, title)
    dock.show()
    dock.raise_()


def _rearrange(win: MainWindow, arrange: Callable[[QMainWindow, dict[str, QDockWidget]], None]) -> None:
    """Re-dock in a hidden stand-in window and restore its state here: tabifying or splitting the
    real window's docks directly leaves orphaned tab bars on screen (Qt 6.10.3+), as
    ``MainWindow._default_layout_state`` says."""
    from .main_window import LAYOUT_VERSION

    helper, stand_ins = win.stand_in_window((max(1, win.width()), max(1, win.height())))
    try:
        # laid out (never on screen), or the saved state keeps zero sizes
        helper.setAttribute(Qt.WidgetAttribute.WA_DontShowOnScreen)
        helper.setDockOptions(helper.dockOptions() & ~QMainWindow.DockOption.AnimatedDocks)
        helper.show()
        helper.restoreState(win.saveState(LAYOUT_VERSION), LAYOUT_VERSION)
        arrange(helper, stand_ins)
        helper.layout().activate()
        state = helper.saveState(LAYOUT_VERSION)
        helper.hide()
    finally:
        helper.deleteLater()
    win.restore_layout(state)


def _split(mw: QMainWindow, other: QDockWidget, dock: QDockWidget, zone: str) -> None:
    """Put ``dock`` beside ``other`` (and its tab mates), or (zone center) in a tab with it."""
    dock.setFloating(False)
    dock.show()
    if zone == "center":
        mw.tabifyDockWidget(other, dock)
        dock.raise_()
        return
    # Qt splits only a lone dock (beside a tabbed one it adds a tab): the anchor's tab mates step
    # out to another area while it splits and come back as its tabs, so the whole group ends up
    # beside the panel as in egui
    mates = [d for d in mw.tabifiedDockWidgets(other) if d is not dock]
    o = Qt.Orientation.Horizontal if zone in ("left", "right") else Qt.Orientation.Vertical
    size = other.width() if o == Qt.Orientation.Horizontal else other.height()
    area = mw.dockWidgetArea(other)
    spare = Qt.DockWidgetArea.RightDockWidgetArea if area != Qt.DockWidgetArea.RightDockWidgetArea else Qt.DockWidgetArea.LeftDockWidgetArea
    # closed panels keep a placeholder in their old group, which keeps it a tab group too
    closed = [d for d in mw.findChildren(QDockWidget) if d.isHidden() and d is not dock and mw.dockWidgetArea(d) == area]
    for d in [*mates, *closed]:
        mw.addDockWidget(spare, d)
    for d in closed:
        d.hide()
    mw.removeDockWidget(dock)
    mw.addDockWidget(area, dock)
    dock.show()
    mw.splitDockWidget(other, dock, o)
    if zone in ("left", "top"):
        mw.splitDockWidget(dock, other, o)  # the anchor goes after: the panel lands before it
    half = max(1, size // 2)  # egui splits the space in two
    mw.resizeDocks([dock, other], [half, half], o)
    for d in mates:
        mw.tabifyDockWidget(other, d)
        d.show()
    other.raise_()


def _place(win: MainWindow, title: str, zone: str, anchor: str) -> None:
    """Beside (or, zone center, in a tab with) ``anchor``."""
    _dock(win, title)
    if anchor == PROGRAM:
        area = ZONES.get(zone)
        if area is None:
            raise DockError("the Program monitor takes no tabs: dock beside it (left, right, top or bottom)")

        def edge(mw: QMainWindow, docks: dict[str, QDockWidget]) -> None:
            dock = docks[title]
            dock.setFloating(False)
            mw.removeDockWidget(dock)
            mw.addDockWidget(area, dock)
            dock.show()

        _rearrange(win, edge)
        return
    other = _dock(win, anchor)
    if anchor == title:
        raise DockError("a panel can't dock to itself")
    if other.isHidden():
        raise DockError(f"{anchor} is not open")
    if other.isFloating():
        raise DockError(f"{anchor} is floating: dock it first")
    _rearrange(win, lambda mw, docks: _split(mw, docks[anchor], docks[title], zone))


def dock_panel(win: MainWindow, title: str, zone: Any = "center", anchor: Any = None, before: Any = None) -> None:
    """egui's ``window.dockPanel``: beside or in a tab with ``anchor``, or at a window edge."""
    if not isinstance(zone, str) or (zone.lower() not in ZONES and zone.lower() not in ("center", "tab")):
        raise DockError("unknown docking zone")
    z = "center" if zone.lower() == "tab" else zone.lower()
    dock = _dock(win, title)
    if anchor is None:
        if before is not None:
            raise DockError("before needs an anchor panel")
        if z == "center":
            dock.setFloating(False)
            dock.show()
            dock.raise_()
            return
        _place(win, title, z, PROGRAM)  # at that edge of the window
        return
    a = panel_title(win, anchor)
    if before is not None:
        panel_title(win, before)  # Qt appends the tab; the name is still checked
        z = "center"
    _place(win, title, z, a)


def _placement(value: Any) -> tuple[str, Any]:
    """``{"Tab": {"before": …}}`` or ``{"Split": "Left"}`` -> (zone, before)."""
    if isinstance(value, dict) and len(value) == 1:
        kind, body = next(iter(value.items()))
        if kind == "Tab" and isinstance(body, dict):
            return "center", body.get("before")
        if kind == "Split" and isinstance(body, str) and (body.lower() in ZONES or body.lower() == "center"):
            return body.lower(), None
    raise DockError("invalid docking action: placement must be {\"Tab\": {\"before\": …}} or {\"Split\": zone}")


def dock_action(win: MainWindow, action: Any) -> str:
    """``window.dockAction``: one shared docking transaction. Returns the panel it moved."""
    if not isinstance(action, dict) or len(action) != 1:
        raise DockError("missing docking action")
    kind, body = next(iter(action.items()))
    if not isinstance(body, dict):
        raise DockError("invalid docking action")
    title = panel_title(win, body.get("panel"))
    if kind == "Close":
        close_panel(win, title)
    elif kind == "Activate":
        activate(win, title)
    elif kind == "Float":
        float_panel(win, title, body.get("rect"))
    elif kind == "MoveFloating":
        move_floating(win, title, body.get("rect"))
    elif kind in ("Move", "OpenAt"):
        zone, before = _placement(body.get("placement"))
        if before is not None:
            panel_title(win, before)
        _place(win, title, zone, panel_title(win, body.get("anchor")))
    elif kind == "Open":
        anchor = body.get("anchor")
        if anchor is None:
            dock_panel(win, title)
        else:
            _place(win, title, "center", panel_title(win, anchor))
    else:
        raise DockError(f"invalid docking action: {kind} is not available in the Qt GUI")
    return panel_id(title)


def run(win: MainWindow, cid: str, params: dict[str, Any]) -> dict[str, Any] | None:
    """Run one of :data:`COMMANDS`; ``{"panel": id}`` (None for ``window.dockAction``)."""
    if cid == "window.dockAction":
        if "action" not in params:
            raise DockError("missing docking action")
        dock_action(win, params["action"])
        return None
    title = _target(win, params)
    if cid == "window.closePanel":
        close_panel(win, title)
    elif cid == "window.floatPanel":
        float_panel(win, title, params.get("rect"))
    elif cid == "window.dockPanel":
        dock_panel(win, title, params.get("zone", "center"), params.get("anchor"), params.get("before"))
    else:
        raise DockError(f"unknown docking command `{cid}`")
    return {"panel": panel_id(title)}


def dock_at(win: MainWindow, global_pos: QPoint) -> str | None:
    """The dock title whose tab or title bar is under ``global_pos``."""
    w = win.childAt(win.mapFromGlobal(global_pos))
    while w is not None and w is not win:
        if isinstance(w, QTabBar):
            i = w.tabAt(w.mapFromGlobal(global_pos))
            text = w.tabText(i) if i >= 0 else ""
            for title, dock in win.docks.items():
                if dock.windowTitle() == text:
                    return title
            return None
        if isinstance(w, QDockWidget):
            for title, dock in win.docks.items():
                if dock is w:
                    return title
            return None
        w = w.parentWidget()
    return None


def panel_menu(win: MainWindow, title: str, parent: QWidget | None = None) -> QMenu:
    """egui's panel menu on a panel's tab: Close Panel, Undock Panel, Restore Workspace."""
    menu = QMenu(parent or win)
    pid = panel_id(title)
    menu.setObjectName(f"panel.menu.{pid}")
    for name, label, slot in (
        ("close", N_("Close Panel"), lambda: run(win, "window.closePanel", {"panel": pid})),
        ("undock", N_("Undock Panel"), lambda: run(win, "window.floatPanel", {"panel": pid})),
        ("restoreWorkspace", N_("Restore Workspace"), lambda: win.workspaces.switch(win.workspaces.current, win.restore_layout)),
    ):
        a = menu.addAction(tr(label))
        a.setObjectName(f"panel.menu.{pid}.{name}")
        a.triggered.connect(lambda _c=False, s=slot: s())
    return menu
