"""Monitor rulers and guides, as egui's ``panels/monitor_view.rs``: rulers (frame pixels) along the
top and left of the picture; drag out of the top ruler for a horizontal guide, out of the left one
for a vertical guide; drag an unlocked guide to move it, back onto a ruler (out of the picture
area) to remove it, Esc during a drag puts it back. View ▸ Add Guide… / Clear Guides / Lock Guides,
Guide Templates ▸ Save Guides as Template… / Manage Guides… (templates are the engine's
``guides.templates`` preference, shared with egui), and Snap in Program Monitor's
:func:`snap_move` (graphic drags snap to guides, the frame edges and centre).

Guides are view state (``MonitorView.guides``: ``{"vertical", "position"}`` in frame pixels).
Automation ids (egui's): ``<monitor>.ruler.top`` / ``.left`` (:meth:`GuideLayer.ruler_rect`),
``<monitor>.guide.<n>`` (:meth:`GuideLayer.guide_rect`), the dialogs ``guides.add`` (``.vertical``,
``.horizontal``, ``.position``, ``.ok``, ``.cancel``), ``guides.save`` (``.name``, ``.ok``,
``.cancel``) and ``guides.manage`` (``.list``, ``.apply``, ``.delete``, ``.close``).
"""

from __future__ import annotations

import math
from collections.abc import Callable
from typing import TYPE_CHECKING, Any

from PySide6.QtCore import QPointF, QRectF, Qt
from PySide6.QtGui import QColor, QFont, QKeyEvent, QMouseEvent, QPainter, QPen
from PySide6.QtWidgets import (
    QDialog,
    QDoubleSpinBox,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QPushButton,
    QRadioButton,
    QVBoxLayout,
    QWidget,
)

from .. import theme
from ..engine import EngineClient
from ..i18n import fmt, tr

if TYPE_CHECKING:
    from .monitor import FrameView

RULER = 16.0
GUIDE_COLOR = QColor(0x3D, 0xA5, 0xFF)
SNAP_COLOR = QColor(0xFF, 0x4F, 0xD8)
SNAP_PX = 6.0  # snap distance (screen px)
HIT = 3.0  # a guide's grab distance either side (screen px)
MAX_GUIDES = 256
MAX_POS = 1e6


def guide(raw: Any) -> dict[str, Any] | None:
    """A saved guide, checked: ``{"vertical": bool, "position": finite px}``."""
    if not isinstance(raw, dict):
        return None
    pos = raw.get("position")
    if isinstance(pos, bool) or not isinstance(pos, (int, float)) or abs(pos) > MAX_POS or not math.isfinite(pos):
        return None
    return {"vertical": raw.get("vertical") is True, "position": float(pos)}


def guides_of(raw: Any) -> list[dict[str, Any]]:
    return [g for g in (guide(x) for x in (raw if isinstance(raw, list) else [])[:MAX_GUIDES]) if g is not None]


def nice_step(px_per_unit: float) -> float:
    """egui's: the first step whose labels are at least 50 px apart."""
    return next((s for s in (1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000) if s * px_per_unit >= 50.0), 5000.0)


def snap_axis(edges: tuple[float, float, float], targets: list[float], thr: float) -> tuple[float, float] | None:
    """The smallest move (within ``thr``) bringing one of ``edges`` onto a target: (move, target)."""
    best: tuple[float, float] | None = None
    for e in edges:
        for t in targets:
            d = t - e
            if abs(d) <= thr and (best is None or abs(d) < abs(best[0])):
                best = (d, t)
    return best


def snap_move(mv: Any, pic: QRectF, frame: tuple[int, int], moving: QRectF, off: QPointF) -> tuple[QPointF, list[tuple[bool, float]]]:
    """Snap in Program Monitor: ``off`` adjusted so the moved box's edges or centre meet the frame
    edges / centre or a shown guide, and the lines snapped to (vertical?, screen coordinate)."""
    if not getattr(mv, "snap", False):
        return off, []
    kx, ky = pic.width() / max(1, frame[0]), pic.height() / max(1, frame[1])
    tx = [pic.left(), pic.center().x(), pic.right()]
    ty = [pic.top(), pic.center().y(), pic.bottom()]
    if mv.show_guides:
        for g in mv.guides:
            (tx if g["vertical"] else ty).append((pic.left() + g["position"] * kx) if g["vertical"] else (pic.top() + g["position"] * ky))
    m = moving.translated(off)
    x, y = off.x(), off.y()
    lines: list[tuple[bool, float]] = []
    sx = snap_axis((m.left(), m.center().x(), m.right()), tx, SNAP_PX)
    if sx is not None:
        x += sx[0]
        lines.append((True, sx[1]))
    sy = snap_axis((m.top(), m.center().y(), m.bottom()), ty, SNAP_PX)
    if sy is not None:
        y += sy[0]
        lines.append((False, sy[1]))
    return QPointF(x, y), lines


class GuideLayer:
    """Rulers and guides on a :class:`FrameView` (drawing and the mouse)."""

    def __init__(self, view: FrameView, changed: Callable[[], None]) -> None:
        self.view = view
        self.changed = changed
        # a drag: (index or None for a new guide, vertical, position before, live position or None)
        self.drag: list[Any] | None = None

    @property
    def mv(self) -> Any:
        return self.view.mv

    def prefix(self) -> str:
        return self.view.objectName().removesuffix(".view")

    # ----- geometry -----
    def rulers_on(self) -> bool:
        return bool(self.mv.show_rulers) and self.view.shown is not None

    def area(self) -> QRectF:
        """The picture area: the view less the rulers."""
        r = QRectF(self.view.rect())
        return r.adjusted(RULER, RULER, 0, 0) if self.rulers_on() else r

    def ruler_rect(self, name: str) -> QRectF:
        r = QRectF(self.view.rect())
        if name == "top":
            return QRectF(r.left() + RULER, r.top(), r.width() - RULER, RULER)
        return QRectF(r.left(), r.top() + RULER, RULER, r.height() - RULER)

    def scale(self) -> tuple[QRectF, float, float]:
        pic = self.view.picture_rect()
        fw, fh = self.view.full
        return pic, pic.width() / max(1, fw), pic.height() / max(1, fh)

    def coord(self, g: dict[str, Any]) -> float:
        pic, kx, ky = self.scale()
        return pic.left() + g["position"] * kx if g["vertical"] else pic.top() + g["position"] * ky

    def guide_rect(self, i: int) -> QRectF | None:
        if not (0 <= i < len(self.mv.guides)) or not self.mv.show_guides:
            return None
        g = self.mv.guides[i]
        c = self.coord(g)
        a = self.area()
        r = QRectF(c - HIT, a.top(), 2 * HIT, a.height()) if g["vertical"] else QRectF(a.left(), c - HIT, a.width(), 2 * HIT)
        return r.intersected(a)

    def position_at(self, vertical: bool, at: QPointF) -> float:
        pic, kx, ky = self.scale()
        return float(round((at.x() - pic.left()) / kx)) if vertical else float(round((at.y() - pic.top()) / ky))

    # ----- drawing -----
    def paint(self, p: QPainter) -> None:
        if self.view.shown is None:
            return
        a = self.area()
        if self.mv.show_guides and self.mv.guides:
            p.save()
            p.setClipRect(a)
            p.setPen(QPen(GUIDE_COLOR, 1.0))
            for g in self.mv.guides:
                c = self.coord(g)
                if g["vertical"]:
                    p.drawLine(QPointF(c, a.top()), QPointF(c, a.bottom()))
                else:
                    p.drawLine(QPointF(a.left(), c), QPointF(a.right(), c))
            p.restore()
        if self.drag is not None and self.drag[0] is None and self.drag[3] is not None:
            p.save()
            p.setClipRect(a)
            p.setPen(QPen(GUIDE_COLOR, 1.0))
            c = self.coord({"vertical": self.drag[1], "position": self.drag[3]})
            if self.drag[1]:
                p.drawLine(QPointF(c, a.top()), QPointF(c, a.bottom()))
            else:
                p.drawLine(QPointF(a.left(), c), QPointF(a.right(), c))
            p.restore()
        if self.rulers_on():
            self.paint_rulers(p)

    def paint_rulers(self, p: QPainter) -> None:
        T = theme.current()
        top, left = self.ruler_rect("top"), self.ruler_rect("left")
        bg = QColor(T.tl_ruler_bg)
        p.fillRect(top, bg)
        p.fillRect(left, bg)
        p.fillRect(QRectF(0, 0, RULER, RULER), bg)
        pic, kx, ky = self.scale()
        f = QFont(p.font())
        f.setPixelSize(8)
        p.setFont(f)
        tick, text = QColor(T.tl_ruler_tick), QColor(T.tl_ruler_text)
        for r, vertical, k, o in ((top, False, kx, pic.left()), (left, True, ky, pic.top())):
            if k <= 0:
                continue
            p.save()
            p.setClipRect(r)
            step = nice_step(k)
            lo, hi = (r.top(), r.bottom()) if vertical else (r.left(), r.right())
            first = math.floor((lo - o) / k / step * 5.0)
            last = math.ceil((hi - o) / k / step * 5.0)
            for i in range(first, min(last, first + 4000) + 1):
                v = i * step / 5.0
                s = o + v * k
                major = i % 5 == 0
                ln = RULER * (0.6 if major else 0.25)
                p.setPen(QPen(tick, 1.0))
                if vertical:
                    p.drawLine(QPointF(r.right() - ln, s), QPointF(r.right(), s))
                    if major:
                        p.setPen(text)
                        p.drawText(QPointF(r.left() + 1, s + 9), str(int(v)))
                else:
                    p.drawLine(QPointF(s, r.bottom() - ln), QPointF(s, r.bottom()))
                    if major:
                        p.setPen(text)
                        p.drawText(QPointF(s + 2, r.top() + 9), str(int(v)))
            p.restore()

    # ----- mouse -----
    def press(self, e: QMouseEvent) -> bool:
        if e.button() != Qt.MouseButton.LeftButton or self.view.shown is None:
            return False
        at = e.position()
        if self.rulers_on():
            for name, vertical in (("top", False), ("left", True)):
                if self.ruler_rect(name).contains(at):
                    self.drag = [None, vertical, None, None]
                    return True
        if self.mv.show_guides and not self.mv.lock_guides:
            for i in range(len(self.mv.guides) - 1, -1, -1):
                r = self.guide_rect(i)
                if r is not None and r.contains(at):
                    g = self.mv.guides[i]
                    self.drag = [i, g["vertical"], g["position"], g["position"]]
                    return True
        return False

    def hover_cursor(self, at: QPointF) -> Qt.CursorShape | None:
        if self.view.shown is None:
            return None
        if self.rulers_on():
            if self.ruler_rect("top").contains(at):
                return Qt.CursorShape.SizeVerCursor
            if self.ruler_rect("left").contains(at):
                return Qt.CursorShape.SizeHorCursor
        if self.mv.show_guides and not self.mv.lock_guides:
            for i in range(len(self.mv.guides)):
                r = self.guide_rect(i)
                if r is not None and r.contains(at):
                    return Qt.CursorShape.SizeHorCursor if self.mv.guides[i]["vertical"] else Qt.CursorShape.SizeVerCursor
        return None

    def move(self, e: QMouseEvent) -> bool:
        if self.drag is None:
            return False
        at = e.position()
        index, vertical = self.drag[0], self.drag[1]
        if self.area().contains(at):
            pos = self.position_at(vertical, at)
            self.drag[3] = pos
            if index is not None and index < len(self.mv.guides):
                self.mv.guides[index]["position"] = pos
        else:
            self.drag[3] = None
        self.view.update()
        return True

    def release(self, e: QMouseEvent) -> bool:
        if self.drag is None:
            return False
        self.move(e)
        index, vertical, _before, pos = self.drag
        self.drag = None
        if index is None:
            if pos is not None and len(self.mv.guides) < MAX_GUIDES:
                self.mv.guides.append({"vertical": vertical, "position": pos})
                self.mv.show_guides = True
        elif pos is None and index < len(self.mv.guides):
            del self.mv.guides[index]  # dropped on a ruler / outside the picture: removed
        self.view.update()
        self.changed()
        return True

    def key(self, e: QKeyEvent) -> bool:
        """Esc during a drag: no new guide, a moved one goes back."""
        if self.drag is None or e.key() != Qt.Key.Key_Escape:
            return False
        index, _v, before, _pos = self.drag
        if index is not None and before is not None and index < len(self.mv.guides):
            self.mv.guides[index]["position"] = before
        self.drag = None
        self.view.update()
        return True


# ----- templates (the engine's guides.templates preference) -----
def load_templates(engine: EngineClient, done: Callable[[list[dict[str, Any]]], None]) -> None:
    def got(r: Any, _b: bytes | None) -> None:
        out = []
        for t in r if isinstance(r, list) else []:
            if isinstance(t, dict) and isinstance(t.get("name"), str):
                out.append({"name": t["name"], "guides": guides_of(t.get("guides"))})
        done(out)

    engine.execute("prefs.get", {"key": "guides.templates"}, on_reply=got, on_error=lambda _m: done([]))


def store_templates(engine: EngineClient, templates: list[dict[str, Any]], done: Callable[[], None], failed: Callable[[str], None]) -> None:
    engine.execute("prefs.set", {"key": "guides.templates", "value": templates}, on_reply=lambda _r, _b: done(), on_error=failed)


def _row_buttons(dialog: QDialog, prefix: str, names: list[tuple[str, str]]) -> dict[str, QPushButton]:
    row = QHBoxLayout()
    row.addStretch(1)
    out = {}
    for key, label in names:
        b = QPushButton(tr(label), dialog)
        b.setObjectName(f"{prefix}.{key}")
        b.setAutoDefault(False)
        row.addWidget(b)
        out[key] = b
    dialog.layout().addLayout(row)
    return out


class AddGuideDialog(QDialog):
    """View ▸ Add Guide…: orientation and position (frame pixels)."""

    def __init__(self, parent: QWidget, middle: float, add: Callable[[bool, float], None]) -> None:
        super().__init__(parent)
        self.setObjectName("guides.add")
        self.setWindowTitle(tr("Add Guide"))
        self.add = add
        lay = QVBoxLayout(self)
        o = QHBoxLayout()
        o.addWidget(QLabel(tr("Orientation:"), self))
        self.vertical = QRadioButton(tr("Vertical"), self)
        self.vertical.setObjectName("guides.add.vertical")
        self.vertical.setChecked(True)
        self.horizontal = QRadioButton(tr("Horizontal"), self)
        self.horizontal.setObjectName("guides.add.horizontal")
        o.addWidget(self.vertical)
        o.addWidget(self.horizontal)
        o.addStretch(1)
        lay.addLayout(o)
        p = QHBoxLayout()
        p.addWidget(QLabel(tr("Position:"), self))
        self.position = QDoubleSpinBox(self)
        self.position.setObjectName("guides.add.position")
        self.position.setRange(-MAX_POS, MAX_POS)
        self.position.setDecimals(0)
        self.position.setSuffix(" px")
        self.position.setValue(middle)
        p.addWidget(self.position)
        p.addStretch(1)
        lay.addLayout(p)
        b = _row_buttons(self, "guides.add", [("cancel", "Cancel"), ("ok", "OK")])
        b["cancel"].clicked.connect(self.reject)
        b["ok"].setDefault(True)
        b["ok"].clicked.connect(self._ok)

    def _ok(self) -> None:
        self.add(self.vertical.isChecked(), float(self.position.value()))
        self.accept()


class SaveGuidesDialog(QDialog):
    """View ▸ Guide Templates ▸ Save Guides as Template…"""

    def __init__(self, parent: QWidget, engine: EngineClient, guides: list[dict[str, Any]], said: Callable[[str], None]) -> None:
        super().__init__(parent)
        self.setObjectName("guides.save")
        self.setWindowTitle(tr("Save Guides as Template"))
        self.engine = engine
        self.guides = [dict(g) for g in guides]
        self.said = said
        lay = QVBoxLayout(self)
        n = QHBoxLayout()
        n.addWidget(QLabel(tr("Name:"), self))
        self.name = QLineEdit(tr("Guides"), self)
        self.name.setObjectName("guides.save.name")
        n.addWidget(self.name)
        lay.addLayout(n)
        self.error = QLabel("", self)
        self.error.setStyleSheet("color: #e08a6a;")
        self.error.hide()
        lay.addWidget(self.error)
        b = _row_buttons(self, "guides.save", [("cancel", "Cancel"), ("ok", "OK")])
        b["cancel"].clicked.connect(self.reject)
        b["ok"].setDefault(True)
        b["ok"].clicked.connect(self.save)

    def _fail(self, message: str) -> None:
        self.error.setText(message)
        self.error.show()

    def save(self) -> None:
        name = self.name.text().strip()
        if not name:
            self._fail(tr("The template needs a name."))
            return

        def got(templates: list[dict[str, Any]]) -> None:
            kept = [t for t in templates if t["name"] != name] + [{"name": name, "guides": self.guides}]

            def done() -> None:
                self.said(fmt("Saved guide template “{name}”", name=name))
                self.accept()

            store_templates(self.engine, kept, done, self._fail)

        load_templates(self.engine, got)


class ManageGuidesDialog(QDialog):
    """View ▸ Guide Templates ▸ Manage Guides…: apply or delete a saved template."""

    def __init__(self, parent: QWidget, engine: EngineClient, apply: Callable[[list[dict[str, Any]]], None]) -> None:
        super().__init__(parent)
        self.setObjectName("guides.manage")
        self.setWindowTitle(tr("Manage Guides"))
        self.engine = engine
        self.apply_guides = apply
        self.templates: list[dict[str, Any]] = []
        self.ready = False
        lay = QVBoxLayout(self)
        self.empty = QLabel(tr("No saved guide templates."), self)
        self.empty.hide()
        lay.addWidget(self.empty)
        self.list = QListWidget(self)
        self.list.setObjectName("guides.manage.list")
        lay.addWidget(self.list)
        b = _row_buttons(self, "guides.manage", [("apply", "Apply"), ("delete", "Delete"), ("close", "Close")])
        self.apply_button, self.delete_button = b["apply"], b["delete"]
        b["apply"].clicked.connect(self.apply)
        b["delete"].clicked.connect(self.delete)
        b["close"].clicked.connect(self.reject)
        self.list.currentRowChanged.connect(lambda _r: self._enable())
        self.reload()

    def reload(self) -> None:
        def got(templates: list[dict[str, Any]]) -> None:
            self.templates = templates
            self.list.clear()
            for t in templates:
                self.list.addItem(t["name"])
            self.empty.setVisible(not templates)
            self._enable()
            self.ready = True

        load_templates(self.engine, got)

    def _enable(self) -> None:
        on = 0 <= self.list.currentRow() < len(self.templates)
        self.apply_button.setEnabled(on)
        self.delete_button.setEnabled(on)

    def select(self, name: str) -> None:
        for i, t in enumerate(self.templates):
            if t["name"] == name:
                self.list.setCurrentRow(i)

    def apply(self) -> None:
        i = self.list.currentRow()
        if 0 <= i < len(self.templates):
            self.apply_guides([dict(g) for g in self.templates[i]["guides"]])
            self.accept()

    def delete(self) -> None:
        i = self.list.currentRow()
        if 0 <= i < len(self.templates):
            name = self.templates[i]["name"]
            kept = [t for t in self.templates if t["name"] != name]
            store_templates(self.engine, kept, self.reload, lambda _m: self.reload())
