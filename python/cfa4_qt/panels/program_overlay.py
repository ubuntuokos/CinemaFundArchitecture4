"""On-monitor editing on the Program monitor, as egui's: the transform box of the selected Motion
or Transform effect, the selected mask's handles, and the mask pen.

What is drawn comes from the engine's ``program.overlay`` query (sequence frame pixels, clip pixels
and the matrices between them); this module only maps the frame onto the shown picture and sends
the edits back as ordinary commands:

* Transform box (Motion or Transform selected in Effect Controls, on a selected clip under the
  playhead): drag inside it for Position, a handle for Scale about the anchor point (with Uniform
  Scale off a side handle changes only the width, a top or bottom one only the height), just
  outside it for Rotation, the anchor point for Anchor Point (the picture stays where it is). One
  drag is one undo step (``effects.setParam`` with a ``merge`` key). A click beside the box lets go
  of it; double-clicking a clip's picture selects its Motion.
* Mask (selected in Effect Controls, its clip selected and under the playhead): drag inside it to
  move it, a vertex or a Bezier handle to reshape it (Alt breaks the handles), the top handle for
  Feather, the bottom one for Expansion, the corner handle to round its sharp corners; Alt-click a
  vertex: smooth / corner, Ctrl-click a vertex: delete it, Ctrl-click an edge: add one there; Shift
  keeps a drag to 45°; Esc lets go of the mask.
* Pen (Effect Controls' "Free draw bezier"): click to place points, drag for curves (Shift: 45°),
  click the first point or press Enter to close the mask; Backspace / Delete / Ctrl+Z take back the
  last point, Esc cancels.

Automation: ``overlay.hit(name)`` locates ``transform.box|anchor|handle.<n>|rotate.<n>``,
``mask.body|vertex.<i>|in.<i>|out.<i>|feather|expansion|roundness`` in the view's coordinates.
"""

from __future__ import annotations

import math
from collections.abc import Callable
from typing import Any

from PySide6.QtCore import QObject, QPointF, QRectF, Qt, Signal
from PySide6.QtGui import QColor, QKeyEvent, QMouseEvent, QPainter, QPainterPath, QPen, QPolygonF, QTransform
from PySide6.QtWidgets import QWidget

from .. import theme

MASK_COLOR = QColor(0x5A, 0xB4, 0xFF)
# how close (points) the pointer must be to a handle or the anchor point to grab it
GRAB = 8.0
ANCHOR_GRAB = 11.0
ANCHOR_RADIUS = 7.0
CORNER = 5.0
EDGE = 4.0
ROTATE_REACH = 24.0
REACH_INSET = 12.0 + CORNER
TRANSFORM_MERGE = "programTransform"


def finite(v: Any, fallback: float = 0.0) -> float:
    try:
        f = float(v)
    except (TypeError, ValueError):
        return fallback
    return f if math.isfinite(f) else fallback


def point(v: Any) -> QPointF | None:
    if isinstance(v, list) and len(v) == 2 and all(isinstance(x, int | float) and not isinstance(x, bool) for x in v):
        x, y = float(v[0]), float(v[1])
        if math.isfinite(x) and math.isfinite(y):
            return QPointF(x, y)
    return None


def points(v: Any) -> list[QPointF]:
    return [p for p in (point(x) for x in v) if p is not None] if isinstance(v, list) else []


def transform(v: Any) -> QTransform | None:
    """An engine ``[a, b, c, d, e, f]`` affine (x' = a x + c y + e, y' = b x + d y + f)."""
    if not (isinstance(v, list) and len(v) == 6):
        return None
    vals = [finite(x, math.nan) for x in v]
    if not all(math.isfinite(x) for x in vals):
        return None
    t = QTransform(vals[0], vals[1], vals[2], vals[3], vals[4], vals[5])
    return t if t.isInvertible() else None


def linear(t: QTransform) -> QTransform:
    return QTransform(t.m11(), t.m12(), t.m21(), t.m22(), 0.0, 0.0)


def unscale(t: QTransform, d: QPointF) -> QPointF:
    """A screen delta in the space ``t`` maps to the screen (its linear part only)."""
    inv, ok = linear(t).inverted()
    return inv.map(d) if ok else QPointF()


def length(p: QPointF) -> float:
    return math.hypot(p.x(), p.y())


def normalized(p: QPointF) -> QPointF:
    n = length(p)
    return QPointF(p.x() / n, p.y() / n) if n > 1e-9 else QPointF()


def dot(a: QPointF, b: QPointF) -> float:
    return a.x() * b.x() + a.y() * b.y()


def seg_dist(p: QPointF, a: QPointF, b: QPointF) -> float:
    ab = b - a
    l2 = dot(ab, ab)
    t = max(0.0, min(1.0, dot(p - a, ab) / l2)) if l2 > 0 else 0.0
    return length(p - (a + ab * t))


def inside(poly: list[QPointF], p: QPointF) -> bool:
    return len(poly) >= 3 and QPolygonF(poly).containsPoint(p, Qt.FillRule.WindingFill)


def constrain_45(origin: QPointF, to: QPointF) -> QPointF:
    """``to`` moved onto the nearest 45° ray from ``origin``, keeping its distance along it."""
    v = to - origin
    if length(v) == 0:
        return to
    step = math.pi / 4
    a = round(math.atan2(v.y(), v.x()) / step) * step
    d = QPointF(math.cos(a), math.sin(a))
    return origin + d * dot(v, d)


def handle_points(quad: list[QPointF]) -> list[QPointF]:
    """The eight handles of a box: corners TL, TR, BR, BL, then the middles of the top, right,
    bottom and left edges."""
    tl, tr, br, bl = quad
    return [tl, tr, br, bl, (tl + tr) / 2, (tr + br) / 2, (br + bl) / 2, (bl + tl) / 2]


def cubic(p0: QPointF, c1: QPointF, c2: QPointF, p3: QPointF, t: float) -> QPointF:
    u = 1 - t
    return p0 * (u * u * u) + c1 * (3 * u * u * t) + c2 * (3 * u * t * t) + p3 * (t * t * t)


class ProgramOverlay(QObject):
    """The overlay of one Program FrameView. ``send(command, params)`` runs an engine command
    (and refreshes the window), ``engine`` answers ``program.overlay``."""

    pen_finished = Signal()

    def __init__(self, view: QWidget, engine: Any, send: Callable[[str, dict[str, Any]], None]) -> None:
        super().__init__(view)
        self.view = view
        self.engine = engine
        self.send = send
        self.data: dict[str, Any] | None = None
        self.enabled = True  # the Selection tool (egui draws the overlay only with it)
        self.pen: dict[str, Any] | None = None
        self.drag: dict[str, Any] | None = None
        self.gesture = 0
        self.selected_vertex: int | None = None
        self.round_radius: float | None = None
        self._busy = False
        self._again = False

    # ----- engine -----
    def refresh(self) -> None:
        if self.engine is None or not getattr(self.engine, "running", False):
            return
        if self._busy:
            self._again = True
            return
        self._busy = True
        params: dict[str, Any] = {}
        if self.pen is not None:
            params["clip"] = self.pen["clip"]
        if self.round_radius:
            params["roundRadius"] = self.round_radius
        self.engine.execute("program.overlay", params, on_reply=self._on_reply, on_error=lambda _m: self._on_reply(None, None))

    def _on_reply(self, result: Any, _b: Any) -> None:
        self._busy = False
        self.data = result if isinstance(result, dict) else None
        if self.pen is not None and self.data is not None:
            m = transform(self.data.get("clipToFrame"))
            if m is not None:
                self.pen["toFrame"] = m
        self.view.update()
        if self._again:
            self._again = False
            self.refresh()

    # ----- geometry -----
    def frame_to_view(self) -> QTransform | None:
        data = self.data or {}
        size = data.get("frame")
        if not (isinstance(size, list) and len(size) == 2):
            return None
        fw, fh = finite(size[0]), finite(size[1])
        if fw <= 0 or fh <= 0:
            return None
        pic = self.view.picture_rect()
        if pic.width() <= 0 or pic.height() <= 0:
            return None
        t = QTransform()
        t.translate(pic.x(), pic.y())
        t.scale(pic.width() / fw, pic.height() / fh)
        return t

    def to_view(self, m: Any) -> QTransform | None:
        """Clip (or Position) space → this view, for an engine matrix into frame pixels."""
        inner = m if isinstance(m, QTransform) else transform(m)
        view = self.frame_to_view()
        if inner is None or view is None:
            return None
        return inner * view

    def placement(self) -> dict[str, Any] | None:
        """The transform box on screen: quad, handles (kept inside the monitor), anchor point."""
        t = (self.data or {}).get("transform")
        if not isinstance(t, dict) or not self.enabled or self.pen is not None:
            return None
        to_screen, outer = self.to_view(t.get("toFrame")), self.to_view(t.get("outer"))
        frame = self.frame_to_view()
        if to_screen is None or outer is None or frame is None:
            return None
        quad = [frame.map(q) for q in points(t.get("quad"))]
        anchor = point(t.get("anchorAt"))
        if len(quad) != 4 or anchor is None:
            return None
        reach = QRectF(self.view.rect()).adjusted(REACH_INSET, REACH_INSET, -REACH_INSET, -REACH_INSET)
        handles = handle_points(quad)
        if reach.width() > 0 and reach.height() > 0:
            handles = [QPointF(min(max(h.x(), reach.left()), reach.right()), min(max(h.y(), reach.top()), reach.bottom()))
                       for h in handles]
        return {"t": t, "toScreen": to_screen, "outer": outer, "quad": quad, "handles": handles,
                "anchorAt": frame.map(anchor), "position": point(t.get("position")) or QPointF(),
                "anchor": point(t.get("anchor")) or QPointF(), "height": finite(t.get("height"), 100.0),
                "width": finite(t.get("width"), 100.0), "rotation": finite(t.get("rotation")),
                "uniform": t.get("uniform") is not False}

    def mask_geometry(self) -> dict[str, Any] | None:
        m = (self.data or {}).get("mask")
        if not isinstance(m, dict) or self.pen is not None:
            return None
        to_screen = self.to_view(m.get("toFrame"))
        if to_screen is None:
            return None
        contour = [to_screen.map(p) for p in points(m.get("contour"))]
        verts = []
        for v in m.get("vertices") or []:
            if isinstance(v, dict) and point(v.get("p")) is not None:
                verts.append({"p": to_screen.map(point(v.get("p"))), "in": to_screen.map(point(v.get("in")) or point(v.get("p"))),
                              "out": to_screen.map(point(v.get("out")) or point(v.get("p"))), "corner": bool(v.get("corner")),
                              "clip": point(v.get("p")), "clipIn": point(v.get("in")), "clipOut": point(v.get("out"))})
        handles = {}
        for which in ("feather", "expansion"):
            h = m.get(f"{which}Handle")
            at, n = (point(h.get("at")), point(h.get("normal"))) if isinstance(h, dict) else (None, None)
            if at is None or n is None:
                continue
            dist = finite(h.get("dist"))
            base = to_screen.map(at + n * dist)
            stem = normalized(to_screen.map(at + n) - to_screen.map(at))
            if length(stem) == 0:
                stem = QPointF(0, -1)
            handles[which] = {"base": base, "pos": base + stem * 14.0, "normal": n}
        rnd = m.get("roundness")
        roundness = None
        if isinstance(rnd, dict) and point(rnd.get("corner")) is not None and point(rnd.get("bisector")) is not None:
            corner, bis = point(rnd.get("corner")), point(rnd.get("bisector"))
            sin_half = max(1e-9, finite(rnd.get("sinHalf"), 1.0))
            radius = self.round_radius or 0.0
            anchor = to_screen.map(corner)
            d = normalized(to_screen.map(corner + bis) - anchor)
            if length(d) == 0:
                d = normalized(QPointF(1, 1))
            pos = to_screen.map(corner + bis * (radius / sin_half)) + d * 18.0
            roundness = {"anchor": anchor, "pos": pos, "bisector": bis, "sinHalf": sin_half,
                         "maxRadius": max(0.0, finite(rnd.get("maxRadius")))}
        return {"m": m, "toScreen": to_screen, "contour": contour, "vertices": verts, "handles": handles,
                "roundness": roundness,
                "expansion": [to_screen.map(p) for p in points(m.get("expansionContour"))],
                "feather": [to_screen.map(p) for p in points(m.get("featherContour"))],
                "rounded": [to_screen.map(p) for p in points(m.get("roundedContour"))]}

    def hit(self, name: str) -> QPointF | None:
        """Where a named handle is on the view (for tests and agents)."""
        kind, _, rest = name.partition(".")
        if kind == "transform":
            pl = self.placement()
            if pl is None:
                return None
            if rest == "box":  # inside the box, a quarter of the way in from its top-left corner (clear of the anchor point)
                return pl["quad"][0] * 0.75 + pl["quad"][2] * 0.25
            if rest == "anchor":
                return pl["anchorAt"]
            what, _, n = rest.partition(".")
            if what == "handle" and n.isdigit() and int(n) < 8:
                return pl["handles"][int(n)]
            if what == "rotate" and n.isdigit() and int(n) < 4:
                q = pl["quad"][int(n)]
                mid = (pl["quad"][0] + pl["quad"][2]) / 2
                return q + normalized(q - mid) * 14.0
            return None
        if kind == "mask":
            g = self.mask_geometry()
            if g is None:
                return None
            what, _, n = rest.partition(".")
            if what == "body":
                c = g["contour"]
                return QPolygonF(c).boundingRect().center() if c else None
            if what in ("vertex", "in", "out") and n.isdigit() and int(n) < len(g["vertices"]):
                return g["vertices"][int(n)]["p" if what == "vertex" else what]
            if what in ("feather", "expansion") and what in g["handles"]:
                return g["handles"][what]["pos"]
            if what == "roundness" and g["roundness"] is not None:
                return g["roundness"]["pos"]
        return None

    # ----- painting -----
    def paint(self, p: QPainter) -> None:
        p.save()
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        if self.pen is not None:
            self._paint_pen(p)
        else:
            pl = self.placement()
            if pl is not None:
                self._paint_transform(p, pl)
            g = self.mask_geometry()
            if g is not None:
                self._paint_mask(p, g)
        p.restore()

    def _paint_transform(self, p: QPainter, pl: dict[str, Any]) -> None:
        accent = theme.current().c("accent")
        p.setPen(QPen(accent, 1))
        p.setBrush(Qt.BrushStyle.NoBrush)
        p.drawPolygon(QPolygonF(pl["quad"]))
        for n, h in enumerate(pl["handles"]):
            s = CORNER if n < 4 else EDGE
            p.setPen(QPen(accent, 1) if n < 4 else Qt.PenStyle.NoPen)
            p.setBrush(QColor("white"))
            p.drawRect(QRectF(h.x() - s / 2, h.y() - s / 2, s, s))
        a = pl["anchorAt"]
        p.setPen(QPen(accent, 1))
        p.setBrush(Qt.BrushStyle.NoBrush)
        p.drawEllipse(a, ANCHOR_RADIUS, ANCHOR_RADIUS)
        p.drawLine(a - QPointF(ANCHOR_RADIUS, 0), a + QPointF(ANCHOR_RADIUS, 0))
        p.drawLine(a - QPointF(0, ANCHOR_RADIUS), a + QPointF(0, ANCHOR_RADIUS))

    def _paint_mask(self, p: QPainter, g: dict[str, Any]) -> None:
        col = MASK_COLOR
        p.setBrush(Qt.BrushStyle.NoBrush)
        if len(g["contour"]) >= 2:
            p.setPen(QPen(col, 1.5))
            p.drawPolygon(QPolygonF(g["contour"]))
        for key, alpha in (("expansion", 0.8), ("feather", 0.6)):
            if len(g[key]) >= 2:
                c = QColor(col)
                c.setAlphaF(alpha)
                pen = QPen(c, 1, Qt.PenStyle.DashLine)
                p.setPen(pen)
                p.drawPolygon(QPolygonF(g[key]))
        if self.drag is not None and self.drag.get("kind") == "roundness" and len(g["rounded"]) >= 2:
            p.setPen(QPen(QColor("white"), 1.5))
            p.drawPolygon(QPolygonF(g["rounded"]))
        for which, h in g["handles"].items():
            p.setPen(QPen(col, 1))
            p.drawLine(h["base"], h["pos"])
            p.setPen(Qt.PenStyle.NoPen)
            p.setBrush(col)
            c = h["pos"]
            if which == "feather":
                p.drawEllipse(c, 4.0, 4.0)
            else:
                p.drawPolygon(QPolygonF([c + QPointF(0, -4.5), c + QPointF(4.5, 0), c + QPointF(0, 4.5), c + QPointF(-4.5, 0)]))
            p.setBrush(Qt.BrushStyle.NoBrush)
        r = g["roundness"]
        if r is not None:
            dim = QColor(col)
            dim.setAlphaF(0.6)
            p.setPen(QPen(dim, 1))
            p.drawLine(r["anchor"], r["pos"])
            p.setPen(QPen(col, 1.5))
            p.setBrush(QColor("white"))
            p.drawEllipse(r["pos"], 4.0, 4.0)
        for v in g["vertices"]:
            if v["corner"]:
                continue
            for which in ("in", "out"):
                p.setPen(QPen(col, 1))
                p.drawLine(v["p"], v[which])
                p.setBrush(QColor("white"))
                p.drawEllipse(v[which], 3.5, 3.5)
        for i, v in enumerate(g["vertices"]):
            p.setPen(QPen(col, 1))
            p.setBrush(col if self.selected_vertex == i else QColor("white"))
            p.drawRect(QRectF(v["p"].x() - 3.5, v["p"].y() - 3.5, 7, 7))
        if self.drag is not None and self.drag.get("label"):
            p.setPen(theme.current().c("hot_text"))
            p.drawText(self.drag.get("label_at", QPointF()) + QPointF(10, -10), self.drag["label"])

    def _paint_pen(self, p: QPainter) -> None:
        pen = self.pen or {}
        m = pen.get("toFrame")
        to_screen = self.to_view(m) if m is not None else None
        if to_screen is None:
            return
        col = MASK_COLOR
        pts = pen.get("points") or []
        screen = [to_screen.map(QPointF(q[0], q[1])) for q in pts]
        if len(pts) >= 2:
            path = QPainterPath(screen[0])
            for b, qa, qb in zip(screen[1:], pts, pts[1:], strict=False):
                c1 = to_screen.map(QPointF(qa[0] + qa[2], qa[1] + qa[3]))
                c2 = to_screen.map(QPointF(qb[0] - qb[2], qb[1] - qb[3]))
                path.cubicTo(c1, c2, b)
            p.setPen(QPen(col, 1.5))
            p.setBrush(Qt.BrushStyle.NoBrush)
            p.drawPath(path)
        hover = pen.get("hover")
        if hover is not None and screen:
            dim = QColor(col)
            dim.setAlphaF(0.5)
            p.setPen(QPen(dim, 1))
            p.drawLine(screen[-1], hover)
        for i, (q, s) in enumerate(zip(pts, screen, strict=False)):
            if q[2] or q[3]:
                for sign in (1, -1):
                    h = to_screen.map(QPointF(q[0] + q[2] * sign, q[1] + q[3] * sign))
                    p.setPen(QPen(col, 1))
                    p.drawLine(s, h)
                    p.setBrush(QColor("white"))
                    p.drawEllipse(h, 3.0, 3.0)
            p.setPen(QPen(col, 1))
            p.setBrush(col if i == 0 else QColor("white"))
            p.drawRect(QRectF(s.x() - 3.5, s.y() - 3.5, 7, 7))

    # ----- pen -----
    def start_pen(self, clip: int, effect: int) -> None:
        self.pen = {"clip": clip, "effect": effect, "points": [], "toFrame": None}
        self.view.setCursor(Qt.CursorShape.CrossCursor)
        self.refresh()

    def stop_pen(self) -> None:
        if self.pen is None:
            return
        self.pen = None
        self.drag = None
        self.view.unsetCursor()
        self.view.update()
        self.pen_finished.emit()

    def _pen_clip_point(self, at: QPointF) -> QPointF | None:
        m = (self.pen or {}).get("toFrame")
        to_screen = self.to_view(m) if m is not None else None
        if to_screen is None:
            return None
        inv, ok = to_screen.inverted()
        return inv.map(at) if ok else None

    def _pen_screen(self, q: list[float]) -> QPointF | None:
        m = (self.pen or {}).get("toFrame")
        to_screen = self.to_view(m) if m is not None else None
        return to_screen.map(QPointF(q[0], q[1])) if to_screen is not None else None

    def _pen_snap(self, at: QPointF, shift: bool) -> QPointF:
        pts = (self.pen or {}).get("points") or []
        if shift and pts:
            last = self._pen_screen(pts[-1])
            if last is not None:
                return constrain_45(last, at)
        return at

    def commit_pen(self) -> None:
        pen = self.pen
        if pen is None or len(pen["points"]) < 3:
            return
        vertices = [{"p": [q[0], q[1]], "in": [-q[2], -q[3]], "out": [q[2], q[3]]} for q in pen["points"]]
        self.send("masks.add", {"clip": pen["clip"], "effect": pen["effect"], "shape": "bezier",
                                "path": {"closed": True, "vertices": vertices}})
        self.stop_pen()

    def _pen_press(self, e: QMouseEvent) -> bool:
        pen = self.pen or {}
        if e.button() != Qt.MouseButton.LeftButton or pen.get("toFrame") is None:
            return True
        at = e.position()
        pts = pen["points"]
        first = self._pen_screen(pts[0]) if pts else None
        if len(pts) >= 3 and first is not None and length(first - at) < 8:
            self.commit_pen()
            return True
        c = self._pen_clip_point(self._pen_snap(at, bool(e.modifiers() & Qt.KeyboardModifier.ShiftModifier)))
        if c is not None:
            pts.append([c.x(), c.y(), 0.0, 0.0])
            self.drag = {"kind": "pen"}
        self.view.update()
        return True

    def _pen_move(self, e: QMouseEvent) -> None:
        pen = self.pen or {}
        at = e.position()
        shift = bool(e.modifiers() & Qt.KeyboardModifier.ShiftModifier)
        pen["hover"] = at
        if self.drag is not None and self.drag.get("kind") == "pen" and pen.get("points"):
            last = pen["points"][-1]
            start = self._pen_screen(last)
            target = constrain_45(start, at) if shift and start is not None else at
            c = self._pen_clip_point(target)
            if c is not None:
                tx, ty = c.x() - last[0], c.y() - last[1]
                if math.hypot(tx, ty) > 2.0:
                    last[2], last[3] = tx, ty
        self.view.update()

    # ----- input -----
    def press(self, e: QMouseEvent) -> bool:
        """A press on the view: True when the overlay took it."""
        if self.pen is not None:
            return self._pen_press(e)
        if e.button() != Qt.MouseButton.LeftButton:
            return False
        at = e.position()
        self.gesture += 1
        mods = e.modifiers()
        alt = bool(mods & Qt.KeyboardModifier.AltModifier)
        ctrl = bool(mods & Qt.KeyboardModifier.ControlModifier)
        g = self.mask_geometry()
        if g is not None:
            m = g["m"]
            target = {"clip": m.get("clip"), "effect": m.get("effect"), "mask": m.get("mask")}
            for i, v in enumerate(g["vertices"]):
                if v["corner"]:
                    continue
                for which in ("in", "out"):
                    if length(v[which] - at) <= 6:
                        self.drag = {"kind": "tangent", "vertex": i, "handle": which, "last": at, "target": target,
                                     "alt": alt, "toScreen": g["toScreen"]}
                        return True
            for i, v in enumerate(g["vertices"]):
                if abs(v["p"].x() - at.x()) <= 6.5 and abs(v["p"].y() - at.y()) <= 6.5:
                    if alt:
                        self.send("masks.toggleVertexSmooth", {**target, "vertex": i})
                    elif ctrl:
                        self.send("masks.removeVertex", {**target, "vertex": i})
                    else:
                        self.selected_vertex = i
                        self.drag = {"kind": "vertex", "vertex": i, "press": at, "applied": QPointF(), "target": target,
                                     "toScreen": g["toScreen"]}
                    self.view.update()
                    return True
            for which, h in g["handles"].items():
                if length(h["pos"] - at) <= 7.5:
                    start = finite(m.get(which))
                    self.drag = {"kind": which, "press": at, "start": start, "normal": h["normal"], "target": target,
                                 "toScreen": g["toScreen"]}
                    return True
            r = g["roundness"]
            if r is not None and length(r["pos"] - at) <= 7.5:
                self.drag = {"kind": "roundness", "press": at, "r": r, "target": target, "toScreen": g["toScreen"]}
                return True
            if ctrl and self._add_vertex(g, at, target):
                return True
            if inside(g["contour"], at):
                self.drag = {"kind": "body", "press": at, "applied": QPointF(), "target": target, "toScreen": g["toScreen"]}
                return True
            self.drag = {"kind": "deselectMask", "press": at}
            return True
        pl = self.placement()
        if pl is not None:
            grab = self._grab_at(pl, at)
            if grab is not None:
                angle = math.degrees(math.atan2((at - pl["anchorAt"]).y(), (at - pl["anchorAt"]).x()))
                self.drag = {"kind": "transform", "grab": grab, "start": at, "pl": pl, "angle": angle, "turned": 0.0,
                             "begin": True}
                return True
            area = QPolygonF(pl["quad"] + pl["handles"]).boundingRect().adjusted(
                -2 * ROTATE_REACH, -2 * ROTATE_REACH, 2 * ROTATE_REACH, 2 * ROTATE_REACH)
            if area.contains(at):
                self.drag = {"kind": "deselectEffect", "press": at}
                return True
        return False

    def move(self, e: QMouseEvent) -> bool:
        if self.pen is not None:
            self._pen_move(e)
            return True
        d = self.drag
        if d is None:
            self._hover_cursor(e.position())
            return False
        at = e.position()
        shift = bool(e.modifiers() & Qt.KeyboardModifier.ShiftModifier)
        kind = d["kind"]
        if kind == "transform":
            self._transform_move(d, at)
        elif kind in ("vertex", "body"):
            raw = at - d["press"]
            want = constrain_45(QPointF(), raw) if shift else raw
            step = want - d["applied"]
            if length(step) > 0:
                d["applied"] = want
                delta = unscale(d["toScreen"], step)
                params = {**d["target"], "delta": [delta.x(), delta.y()], "merge": f"{kind}-{self.gesture}"}
                if kind == "vertex":
                    self.send("masks.moveVertex", {**params, "vertex": d["vertex"]})
                else:
                    self.send("masks.translate", params)
        elif kind == "tangent":
            step = at - d["last"]
            d["last"] = at
            delta = unscale(d["toScreen"], step)
            self.send("masks.moveVertex", {**d["target"], "vertex": d["vertex"], "handle": d["handle"],
                                           "delta": [delta.x(), delta.y()], "breakHandles": d["alt"],
                                           "merge": f"tan-{self.gesture}"})
        elif kind in ("feather", "expansion"):
            delta = unscale(d["toScreen"], at - d["press"])
            along = dot(delta, d["normal"])
            v = max(0.0, d["start"] + 2 * along) if kind == "feather" else d["start"] + along
            d["label"], d["label_at"] = f"{v:.1f}", at
            self.send("masks.set", {**d["target"], kind: v, "merge": f"{kind}-{self.gesture}"})
        elif kind == "roundness":
            r = d["r"]
            delta = unscale(d["toScreen"], at - d["press"])
            dist = dot(delta, r["bisector"])
            radius = max(0.0, min(r["maxRadius"], dist * r["sinHalf"])) if math.isfinite(dist) else 0.0
            d["radius"] = radius
            d["label"], d["label_at"] = f"{radius:.1f}", at
            self.round_radius = radius
            self.refresh()
        self.view.update()
        return True

    def release(self, e: QMouseEvent) -> bool:
        if self.pen is not None:
            self.drag = None
            return True
        d, self.drag = self.drag, None
        if d is None:
            return False
        moved = length(e.position() - d.get("press", d.get("start", e.position()))) >= 3
        if d["kind"] == "roundness":
            self.round_radius = None
            radius = round(d.get("radius", 0.0) * 10) / 10
            if radius > 0:
                self.send("masks.roundCorners", {**d["target"], "radius": radius})
        elif d["kind"] == "deselectMask" and not moved:
            self.send("masks.select", {"none": True})
        elif d["kind"] == "deselectEffect" and not moved:
            self.send("effects.select", {"none": True})
        self.view.update()
        return True

    def double_click(self, e: QMouseEvent) -> bool:
        """Double-clicking a clip's picture selects it and its Motion (its box comes up)."""
        if self.pen is not None or not self.enabled or e.button() != Qt.MouseButton.LeftButton:
            return self.pen is not None
        frame = self.frame_to_view()
        if frame is None or not self.view.picture_rect().contains(e.position()):
            return False
        for c in (self.data or {}).get("clips") or []:
            if not isinstance(c, dict):
                continue
            quad = [frame.map(q) for q in points(c.get("quad"))]
            if len(quad) == 4 and inside(quad, e.position()):
                clip = c.get("clip")
                self.send("timeline.select", {"clips": [clip]})
                self.send("effects.select", {"clip": clip, "effect": "motion"})
                return True
        return False

    def claims_key(self, e: QKeyEvent) -> bool:
        """Keys the overlay takes ahead of the app's shortcuts (Enter closes the pen's mask, not
        Render Effects In to Out; Ctrl+Z takes back a pen point, not an edit)."""
        k, mods = e.key(), e.modifiers()
        if self.pen is not None:
            ctrl_z = k == Qt.Key.Key_Z and mods & Qt.KeyboardModifier.ControlModifier and not mods & Qt.KeyboardModifier.ShiftModifier
            return bool(ctrl_z) or k in (Qt.Key.Key_Escape, Qt.Key.Key_Return, Qt.Key.Key_Enter, Qt.Key.Key_Backspace,
                                          Qt.Key.Key_Delete)
        return k == Qt.Key.Key_Escape and (self.drag is not None or self.mask_geometry() is not None)

    def key(self, e: QKeyEvent) -> bool:
        if self.pen is not None:
            pts = self.pen["points"]
            if e.key() == Qt.Key.Key_Escape:
                self.stop_pen()
            elif e.key() in (Qt.Key.Key_Return, Qt.Key.Key_Enter):
                self.commit_pen()
            elif e.key() in (Qt.Key.Key_Backspace, Qt.Key.Key_Delete) or (
                    e.key() == Qt.Key.Key_Z and e.modifiers() & Qt.KeyboardModifier.ControlModifier
                    and not e.modifiers() & Qt.KeyboardModifier.ShiftModifier):
                if pts:
                    pts.pop()
                if not pts:
                    self.stop_pen()
            else:
                return False
            self.view.update()
            return True
        if e.key() == Qt.Key.Key_Escape:
            if self.drag is not None:
                self.drag = None
                self.round_radius = None
                self.view.update()
                return True
            if self.mask_geometry() is not None:
                self.send("masks.select", {"none": True})
                return True
        return False

    # ----- transform -----
    @staticmethod
    def _grab_at(pl: dict[str, Any], p: QPointF) -> Any:
        if length(pl["anchorAt"] - p) <= ANCHOR_GRAB:
            return "anchor"
        for n, h in enumerate(pl["handles"]):
            if length(h - p) <= GRAB:
                return n
        if inside(pl["quad"], p):
            return "move"
        q = pl["quad"]
        if any(seg_dist(p, q[i], q[(i + 1) % 4]) <= ROTATE_REACH for i in range(4)):
            return "rotate"
        return None

    @staticmethod
    def _scale_factors(pl: dict[str, Any], n: int, start: QPointF, cur: QPointF) -> tuple[float, float]:
        tl, tr, _br, bl = pl["quad"]
        ux, uy = normalized(tr - tl), normalized(bl - tl)
        frm, to = start - pl["anchorAt"], cur - pl["anchorAt"]

        def along(direction: QPointF) -> float:
            a = dot(frm, direction)
            return 1.0 if abs(a) < 1.0 else max(0.0, dot(to, direction) / a)

        uniform = pl["uniform"]
        if n <= 3:
            if uniform:
                f = along(normalized(frm))
                return f, f
            return along(ux), along(uy)
        if n in (4, 6):
            return (along(uy), along(uy)) if uniform else (1.0, along(uy))
        return (along(ux), along(ux)) if uniform else (along(ux), 1.0)

    def _transform_move(self, d: dict[str, Any], cur: QPointF) -> None:
        pl = d["pl"]
        t = pl["t"]
        off = cur - pl["anchorAt"]
        if d["grab"] == "rotate" and length(off) >= 1:
            a = math.degrees(math.atan2(off.y(), off.x()))
            d["turned"] += (a - d["angle"] + 180) % 360 - 180
            d["angle"] = a
        moved = cur - d["start"]

        def pt(v: QPointF) -> list[float] | None:
            if not (math.isfinite(v.x()) and math.isfinite(v.y())):
                return None
            return [max(-100_000.0, min(100_000.0, v.x())), max(-100_000.0, min(100_000.0, v.y()))]

        def scale(v: float) -> float | None:
            return max(0.0, min(10_000.0, v)) if math.isfinite(v) else None

        height = "scale_height" if t.get("effectId") == "transform" else "scale"
        grab = d["grab"]
        if grab == "move":
            changes = [("position", pt(pl["position"] + unscale(pl["outer"], moved)))]
        elif grab == "anchor":
            changes = [("anchor", pt(pl["anchor"] + unscale(pl["toScreen"], moved))),
                       ("position", pt(pl["position"] + unscale(pl["outer"], moved)))]
        elif grab == "rotate":
            r = pl["rotation"] + d["turned"]
            changes = [("rotation", max(-36_000.0, min(36_000.0, r)) if math.isfinite(r) else None)]
        else:
            fx, fy = self._scale_factors(pl, grab, d["start"], cur)
            if pl["uniform"] or grab in (4, 6):
                changes = [(height, scale(pl["height"] * fy))]
            elif grab in (5, 7):
                changes = [("scale_width", scale(pl["width"] * fx))]
            else:
                changes = [(height, scale(pl["height"] * fy)), ("scale_width", scale(pl["width"] * fx))]
        for param, value in changes:
            if value is None:
                continue
            self.send("effects.setParam", {"clip": t.get("clip"), "effect": t.get("effect"), "param": param, "value": value,
                                           "merge": TRANSFORM_MERGE, "begin": d["begin"]})
            d["begin"] = False

    def _hover_cursor(self, at: QPointF) -> None:
        pl = self.placement()
        grab = self._grab_at(pl, at) if pl is not None else None
        if grab is None:
            if self.pen is None and not getattr(self.view, "hand", False):
                self.view.unsetCursor()
            return
        if grab == "move":
            self.view.setCursor(Qt.CursorShape.SizeAllCursor)
        elif grab in ("anchor", "rotate"):
            self.view.setCursor(Qt.CursorShape.CrossCursor if grab == "rotate" else Qt.CursorShape.ArrowCursor)
        else:
            mid = (pl["quad"][0] + pl["quad"][2]) / 2
            v = handle_points(pl["quad"])[grab] - mid
            deg = math.degrees(math.atan2(v.y(), v.x())) % 180
            if not 22.5 <= deg < 157.5:
                shape = Qt.CursorShape.SizeHorCursor
            elif deg < 67.5:
                shape = Qt.CursorShape.SizeFDiagCursor
            elif deg < 112.5:
                shape = Qt.CursorShape.SizeVerCursor
            else:
                shape = Qt.CursorShape.SizeBDiagCursor
            self.view.setCursor(shape)

    def _add_vertex(self, g: dict[str, Any], at: QPointF, target: dict[str, Any]) -> bool:
        """Ctrl-click on an edge adds a vertex there."""
        verts = g["vertices"]
        n = len(verts)
        best: tuple[int, float] | None = None
        for i in range(n):
            a, b = verts[i], verts[(i + 1) % n]
            prev = a["p"]
            dist = math.inf
            for k in range(1, 17):
                q = cubic(a["p"], a["out"], b["in"], b["p"], k / 16)
                dist = min(dist, seg_dist(at, prev, q))
                prev = q
            if best is None or dist < best[1]:
                best = (i, dist)
        if best is None or best[1] >= 8:
            return False
        inv, ok = g["toScreen"].inverted()
        if not ok:
            return False
        c = inv.map(at)
        self.send("masks.addVertex", {**target, "after": best[0], "at": [c.x(), c.y()]})
        return True
