"""The graphics tools on the Program monitor, as egui's (crates/ui-egui/src/panels/graphics.rs):

* Selection tool: a click selects a graphic layer (its box with eight handles and the anchor
  point); dragging it moves it (Snap in Program Monitor, Ctrl moves freely), the anchor point moves
  alone (the layer stays), a handle scales point text about its anchor point, resizes a paragraph
  text box, changes a shape's Size or stretches a path; a rectangle's corner circles round its
  corners. A click beside every layer lets go of the layers.
* Rectangle / Ellipse tools: drag out a shape. Pen tool: click points, click the first point again
  (or press Enter) to close the path, Esc cancels.
* Type tool: a click on empty picture starts a new point-text layer (in the selected graphic clip
  under the playhead, else a new graphic clip) with a caret; a drag starts paragraph text in the
  dragged box; a click on a text layer puts the caret there. Vertical Type: point text only.
  Double-clicking a text layer with the Selection tool edits it with all its text selected.
  Typing edits the text (``graphics.setText``, one undo step per typing session); arrows,
  Home / End, Shift selection, Ctrl+A / C / X / V, Return (a new line) and Esc (stop) work as in
  a text field.

The geometry comes from the engine (``program.overlay`` ``graphics``: layer pixels → frame, the
graphic canvas → frame, each layer's box); every edit is one ``graphics.*`` command.
"""

from __future__ import annotations

import math
from typing import TYPE_CHECKING, Any

from PySide6.QtCore import QPointF, QRectF, Qt
from PySide6.QtGui import (
    QColor,
    QGuiApplication,
    QKeyEvent,
    QMouseEvent,
    QPainter,
    QPainterPath,
    QPen,
    QPolygonF,
    QTransform,
)

from .. import theme

if TYPE_CHECKING:
    from .program_overlay import ProgramOverlay

TEXT_TOOLS = ("type", "verticalType")
DRAW_TOOLS = ("rectangle", "ellipse", "pen", *TEXT_TOOLS)
GRAPHICS_TOOLS = ("selection", *DRAW_TOOLS)
MAX_TEXT = 100_000  # characters a text layer takes from typing or pasting
HANDLE_GRAB = 7.0
CORNER_GRAB = 6.0
CORNER_INSET = 12.0  # corner-radius handles sit at least this far inside a rectangle's corners
NEAR_ANCHOR = 60.0  # a handle this close to the anchor point scales as if it were this far
PEN_CLOSE = 8.0
MOVED = 2.0  # a press that travels less than this is a click


def _num(v: Any, fallback: float = 0.0) -> float:
    if isinstance(v, bool) or not isinstance(v, int | float):
        return fallback
    f = float(v)
    return f if abs(f) < 1e12 and math.isfinite(f) else fallback


def _pt(v: Any) -> QPointF:
    return QPointF(_num(v[0]), _num(v[1])) if isinstance(v, list) and len(v) == 2 else QPointF()


def _matrix(v: Any) -> QTransform | None:
    if not (isinstance(v, list) and len(v) == 6):
        return None
    vals = [_num(x, math.nan) for x in v]
    if not all(math.isfinite(x) for x in vals):
        return None
    t = QTransform(*vals)
    return t if t.isInvertible() else None


def _len(p: QPointF) -> float:
    return math.hypot(p.x(), p.y())


def _unit(p: QPointF) -> QPointF:
    n = _len(p)
    return QPointF(p.x() / n, p.y() / n) if n > 1e-9 else QPointF()


def _dot(a: QPointF, b: QPointF) -> float:
    return a.x() * b.x() + a.y() * b.y()


def _linear_inverse(t: QTransform, d: QPointF) -> QPointF:
    """A screen offset in the units ``t`` maps to the screen (egui's ``unscale``)."""
    inv, ok = QTransform(t.m11(), t.m12(), t.m21(), t.m22(), 0.0, 0.0).inverted()
    return inv.map(d) if ok else QPointF()


def handle_points(quad: list[QPointF]) -> list[QPointF]:
    """The corners TL, TR, BR, BL (0–3), then the middles of the top, right, bottom, left edges."""
    return [*quad, *((quad[i] + quad[(i + 1) % 4]) / 2 for i in range(4))]


def drag_sides(box: list[float], n: int, dx: float, dy: float, least: float) -> list[float]:
    """``box`` (x0, y0, x1, y1) with handle ``n`` moved by (dx, dy): that side or corner follows,
    the opposite one stays, and the box keeps at least ``least`` across (egui's ``drag_sides``)."""
    x0, y0, x1, y1 = box
    if n in (0, 3, 7):
        x0 = min(x0 + dx, x1 - least)
    if n in (1, 2, 5):
        x1 = max(x1 + dx, x0 + least)
    if n in (0, 1, 4):
        y0 = min(y0 + dy, y1 - least)
    if n in (2, 3, 6):
        y1 = max(y1 + dy, y0 + least)
    return [x0, y0, x1, y1]


def text_lines(text: Any) -> list[dict[str, Any]]:
    """A text layer's lines (``program.overlay``): character ranges, baseline, ascent / descent
    and caret stops ``(index, x)``, in layer pixels; damaged entries left out."""
    out = []
    for ln in (text.get("lines") if isinstance(text, dict) else None) or []:
        if not isinstance(ln, dict):
            continue
        stops = []
        for c in ln.get("carets") or []:
            if isinstance(c, list) and len(c) == 2 and isinstance(c[0], int) and not isinstance(c[0], bool):
                stops.append((max(0, min(c[0], MAX_TEXT * 4)), _num(c[1])))
        start = ln.get("start") if isinstance(ln.get("start"), int) else 0
        end = ln.get("end") if isinstance(ln.get("end"), int) else start
        out.append({"start": start, "end": max(start, end), "baseline": _num(ln.get("baseline")), "x": _num(ln.get("x")),
                    "width": _num(ln.get("width")), "ascent": _num(ln.get("ascent")), "descent": _num(ln.get("descent")),
                    "carets": stops})
    return out


def line_of(lines: list[dict[str, Any]], pos: int) -> int:
    li = 0
    for i, ln in enumerate(lines):
        if ln["start"] <= pos:
            li = i
    return li


def caret_xy(lines: list[dict[str, Any]], pos: int) -> tuple[float, float, int]:
    """Where the caret before character ``pos`` is: x, baseline and line (egui's ``caret``)."""
    if not lines:
        return 0.0, 0.0, 0
    li = line_of(lines, pos)
    ln = lines[li]
    x = min(ln["carets"], key=lambda c: abs(c[0] - pos))[1] if ln["carets"] else ln["x"]
    return x, ln["baseline"], li


def caret_hit(lines: list[dict[str, Any]], x: float, y: float, vertical: bool = False) -> int:
    """The caret stop nearest layer point (x, y) (egui's ``hit``)."""
    def dist(ln: dict[str, Any]) -> float:
        top, bottom = ln["baseline"] - ln["ascent"], ln["baseline"] + ln["descent"]
        d = top - y if y < top else y - bottom if y > bottom else 0.0
        if vertical:
            half = ln["ascent"] * 0.5
            d += ln["x"] - half - x if x < ln["x"] - half else max(0.0, x - ln["x"] - ln["width"] - half)
        return d

    if not lines:
        return 0
    ln = min(lines, key=dist)
    return min(ln["carets"], key=lambda c: abs(c[1] - x))[0] if ln["carets"] else ln["start"]


def selection_rects(lines: list[dict[str, Any]], a: int, b: int) -> list[tuple[float, float, float, float]]:
    """The selected text from ``a`` to ``b`` as one box per line (egui's ``selection_rects``)."""
    a, b = min(a, b), max(a, b)
    out = []
    for ln in lines:
        s, e = max(a, ln["start"]), min(b, ln["end"])
        if s > e or (s == e and not (a < ln["start"] and b > ln["end"])):
            continue
        xs = [x for p, x in ln["carets"] if s <= p <= e]
        if len(xs) < 2 and b <= ln["end"]:
            continue
        if not xs:
            continue
        x0, x1 = min(xs), max(xs)
        if b > ln["end"]:
            x1 += ln["ascent"] * 0.25  # the selected newline
        out.append((min(x0, x1), ln["baseline"] - ln["ascent"], x1, ln["baseline"] + ln["descent"]))
    return out


def word_left(s: str, i: int) -> int:
    i = min(max(i, 0), len(s))
    while i > 0 and s[i - 1].isspace():
        i -= 1
    while i > 0 and not s[i - 1].isspace():
        i -= 1
    return i


def word_right(s: str, i: int) -> int:
    i = min(max(i, 0), len(s))
    while i < len(s) and s[i].isspace():
        i += 1
    while i < len(s) and not s[i].isspace():
        i += 1
    return i


class Layer:
    """A graphic layer in the picture with its layer → view transform (egui's ``LayerView``)."""

    def __init__(self, d: dict[str, Any], to_screen: QTransform, canvas_to_screen: QTransform) -> None:
        self.d = d
        self.clip = d.get("clip")
        self.layer = d.get("layer")
        self.to_screen = to_screen
        self.canvas_to_screen = canvas_to_screen
        lb = d.get("local") if isinstance(d.get("local"), list) and len(d["local"]) == 4 else [0, 0, 0, 0]
        self.local = [_num(x) for x in lb]
        self.text = d.get("text") if isinstance(d.get("text"), dict) else None
        self.shape = d.get("shape") if isinstance(d.get("shape"), dict) else None
        self.position = _pt(d.get("position"))
        self.anchor = _pt(d.get("anchor"))
        sc = d.get("scale") if isinstance(d.get("scale"), list) and len(d["scale"]) == 2 else [100, 100]
        self.scale = (_num(sc[0], 100.0), _num(sc[1], 100.0))  # width, height in %

    def map(self, x: float, y: float) -> QPointF:
        return self.to_screen.map(QPointF(x, y))

    def quad(self) -> list[QPointF]:
        x0, y0, x1, y1 = self.local
        return [self.map(x0, y0), self.map(x1, y0), self.map(x1, y1), self.map(x0, y1)]

    def screen_scale(self) -> float:
        t = self.to_screen
        return math.sqrt(abs(t.m11() * t.m22() - t.m12() * t.m21()))

    def to_local(self, p: QPointF) -> QPointF | None:
        inv, ok = self.to_screen.inverted()
        return inv.map(p) if ok else None

    def hit(self, p: QPointF) -> bool:
        q = self.to_local(p)
        if q is None:
            return False
        pad = 4.0 / max(self.screen_scale(), 1e-6)
        x0, y0, x1, y1 = self.local
        return x0 - pad <= q.x() <= x1 + pad and y0 - pad <= q.y() <= y1 + pad

    def anchor_screen(self) -> QPointF:
        return self.map(self.anchor.x(), self.anchor.y())

    def paragraph(self) -> bool:
        return bool(self.text and self.text.get("paragraph"))

    def corner_handles(self) -> tuple[float, list[QPointF]] | None:
        """A rectangle's drawn corner radius and its four corner-radius handles on screen; None
        for other layers and for a rectangle too small on screen to hold them."""
        if not self.shape or self.shape.get("shape") != 0:
            return None
        q = self.quad()
        if _len(q[1] - q[0]) < 4 * CORNER_INSET or _len(q[3] - q[0]) < 4 * CORNER_INSET:
            return None
        size = self.shape.get("size") if isinstance(self.shape.get("size"), list) and len(self.shape["size"]) == 2 else [0, 0]
        r = max(0.0, min(_num(self.shape.get("cornerRadius")), _num(size[0]) / 2, _num(size[1]) / 2))
        return r, self.corner_points(r)

    def corner_points(self, r: float) -> list[QPointF]:
        x0, y0, x1, y1 = self.local
        d = min(max(r, CORNER_INSET / max(self.screen_scale(), 1e-6)), (x1 - x0) / 2, (y1 - y0) / 2)
        return [self.map(x0 + d, y0 + d), self.map(x1 - d, y0 + d), self.map(x1 - d, y1 - d), self.map(x0 + d, y1 - d)]

    def corner_drag(self, r0: float, n: int, start: QPointF, cur: QPointF) -> float:
        a, b = self.to_local(start), self.to_local(cur)
        if a is None or b is None:
            return r0
        sx, sy = ((1, 1), (-1, 1), (-1, -1), (1, -1))[min(max(n, 0), 3)]
        inward = ((b.x() - a.x()) * sx + (b.y() - a.y()) * sy) / 2
        x0, y0, x1, y1 = self.local
        half = max(0.0, min(x1 - x0, y1 - y0) / 2)
        return max(0.0, min(r0 + inward, half))

    def round_rect(self, r: float) -> QPolygonF:
        x0, y0, x1, y1 = self.local
        pts = []
        for cx, cy, start in ((x1 - r, y0 + r, -90.0), (x1 - r, y1 - r, 0.0), (x0 + r, y1 - r, 90.0), (x0 + r, y0 + r, 180.0)):
            for i in range(9):
                a = math.radians(start + 90.0 * i / 8)
                pts.append(self.map(cx + r * math.cos(a), cy + r * math.sin(a)))
        return QPolygonF(pts)


def handle_drag(v: Layer, n: int, start: QPointF, cur: QPointF, keep_ratio: bool) -> tuple[list[QPointF], dict[str, Any]]:
    """What dragging handle ``n`` of layer ``v`` from ``start`` to ``cur`` does (egui's
    ``HandleDrag``): the box it leaves on screen and the ``graphics.set`` properties. Point text
    scales about its anchor point, a paragraph box is resized (the text re-wraps), a shape's Size
    follows the handle, a path stretches away from its opposite side."""
    n = min(max(n, 0), 7)
    quad = v.quad()
    a, b = v.to_local(start), v.to_local(cur)
    dx, dy = (b.x() - a.x(), b.y() - a.y()) if a is not None and b is not None else (0.0, 0.0)

    def box_quad(r: list[float]) -> list[QPointF]:
        return [v.map(r[0], r[1]), v.map(r[2], r[1]), v.map(r[2], r[3]), v.map(r[0], r[3])]

    if v.text is None and v.shape is not None and v.shape.get("shape") != 3:
        frm = list(v.local)
        rect = drag_sides(frm, n, dx, dy, 1.0)
        w, h = frm[2] - frm[0], frm[3] - frm[1]
        if keep_ratio and n < 4 and w > 0 and h > 0:
            fw, fh = (rect[2] - rect[0]) / w, (rect[3] - rect[1]) / h
            f = max(fw if abs(fw - 1) >= abs(fh - 1) else fh, 1 / w, 1 / h)
            if n in (0, 3):
                rect[0] = rect[2] - w * f
            else:
                rect[2] = rect[0] + w * f
            if n in (0, 1):
                rect[1] = rect[3] - h * f
            else:
                rect[3] = rect[1] + h * f
        size = v.shape.get("size") if isinstance(v.shape.get("size"), list) and len(v.shape["size"]) == 2 else [0, 0]

        def ratio(x: float, y: float) -> float:
            return x / y if y > 0 else 1.0

        kx, ky = ratio(rect[2] - rect[0], frm[2] - frm[0]), ratio(rect[3] - rect[1], frm[3] - frm[1])
        ox, oy = rect[0] - frm[0] * kx, rect[1] - frm[1] * ky
        props = {"size": [_num(size[0]) * kx, _num(size[1]) * ky], "anchor": [v.anchor.x() - ox, v.anchor.y() - oy]}
        return box_quad(rect), props
    if v.text is not None and v.paragraph():
        bw, bh = _num(v.text.get("boxWidth")), _num(v.text.get("boxHeight"))
        frm = [0.0, 0.0, bw, bh] if bh > 0 else [0.0, v.local[1], bw, v.local[3]]
        rect = drag_sides(frm, n, dx, dy, max(_num(v.text.get("size")) * 0.5, 1.0))
        props = {"box_width": rect[2] - rect[0],
                 "anchor": [v.anchor.x() - (rect[0] - frm[0]), v.anchor.y() - (rect[1] - frm[1])]}
        if bh > 0 or n not in (5, 7):
            props["box_height"] = rect[3] - rect[1]
        return box_quad(rect), props
    if v.text is not None:
        # every handle scales both axes alike, by the travel along one side of the box
        anchor = v.anchor_screen()
        axis = _unit(quad[1] - quad[0]) if n in (5, 7) else _unit(quad[3] - quad[0])
        handle = handle_points(quad)[n]
        frm = _dot(handle - anchor, axis)
        centre = (quad[0] + quad[2]) / 2
        away = math.copysign(1.0, frm) if abs(frm) > 0.5 else (-1.0 if _dot(centre - handle, axis) < 0 else 1.0)
        reach = abs(frm) + (NEAR_ANCHOR if abs(frm) < NEAR_ANCHOR else 0.0)
        f = 1.0 + _dot(cur - start, axis) * away / reach
        f = max(f, 0.01) if math.isfinite(f) else 1.0
        return [anchor + (q - anchor) * f for q in quad], {"scale": v.scale[1] * f, "scale_width": v.scale[0] * f}
    # a path (or anything else): stretch away from the opposite handle
    pts = handle_points(quad)
    pin = pts[(n + 2) % 4] if n < 4 else pts[4 + (n - 4 + 2) % 4]
    ux, uy = _unit(quad[1] - quad[0]), _unit(quad[3] - quad[0])

    def along(direction: QPointF) -> float:
        frm = _dot(start - pin, direction)
        return 1.0 if abs(frm) < 1.0 else max(_dot(cur - pin, direction) / frm, 0.01)

    if n < 4 and keep_ratio:
        fx = fy = along(_unit(start - pin))
    elif n < 4:
        fx, fy = along(ux), along(uy)
    elif n in (4, 6):
        fx, fy = 1.0, along(uy)
    else:
        fx, fy = along(ux), 1.0

    def stretch(p: QPointF) -> QPointF:
        w = p - pin
        return pin + ux * (_dot(w, ux) * fx) + uy * (_dot(w, uy) * fy)

    a0 = v.anchor_screen()
    delta = _linear_inverse(v.canvas_to_screen, stretch(a0) - a0)
    props = {"scale": v.scale[1] * fy, "scale_width": v.scale[0] * fx,
             "position": [v.position.x() + delta.x(), v.position.y() + delta.y()]}
    if n >= 4 or not keep_ratio:
        props["uniform_scale"] = False
    return [stretch(q) for q in quad], props


class GraphicsTools:
    """The graphics tools of one Program monitor overlay (``ProgramOverlay.graphics``)."""

    def __init__(self, overlay: ProgramOverlay) -> None:
        self.overlay = overlay
        self.tool = "selection"
        self.drag: dict[str, Any] | None = None
        self.pen_points: list[QPointF] = []  # the Pen tool's path so far, in graphic canvas pixels
        self.hover: QPointF | None = None
        # the text being typed into: clip, layer, its text, caret and anchor (character offsets);
        # "seen": the layer has been in the picture (a new one is on its way until then)
        self.edit: dict[str, Any] | None = None

    # ----- data -----
    def data(self) -> dict[str, Any]:
        g = (self.overlay.data or {}).get("graphics")
        return g if isinstance(g, dict) else {}

    def layers(self) -> list[Layer]:
        """The layers in the picture, front-most first."""
        view = self.overlay.frame_to_view()
        if view is None:
            return []
        out = []
        for d in self.data().get("layers") or []:
            if not isinstance(d, dict):
                continue
            m, c = _matrix(d.get("toFrame")), _matrix(d.get("canvasToFrame"))
            if m is not None and c is not None:
                out.append(Layer(d, m * view, c * view))
        return out

    def selected(self) -> list[Layer]:
        g = self.data()
        clip, chosen = g.get("clip"), g.get("selectedLayers")
        chosen = chosen if isinstance(chosen, list) else []
        return [v for v in self.layers() if clip is not None and v.clip == clip and v.layer in chosen]

    def canvas_target(self) -> tuple[Any, QTransform] | None:
        """Where a new layer goes and graphic canvas → view: the selected graphic clip under the
        playhead (its Motion applies), else the frame."""
        t = self.data().get("canvasTarget")
        view = self.overlay.frame_to_view()
        m = _matrix(t.get("toFrame")) if isinstance(t, dict) else None
        if view is None or m is None:
            return None
        return t.get("clip"), m * view

    def to_canvas(self, at: QPointF) -> tuple[Any, QPointF] | None:
        target = self.canvas_target()
        if target is None:
            return None
        inv, ok = target[1].inverted()
        return (target[0], inv.map(at)) if ok else None

    def active(self) -> bool:
        return self.tool in GRAPHICS_TOOLS

    def set_tool(self, tool: str) -> None:
        self.tool = tool
        if tool != "pen":
            self.pen_points = []
        if tool not in ("selection", *TEXT_TOOLS):
            self.edit = None
        self.drag = None
        self.overlay.view.update()

    # ----- text editing -----
    def edit_view(self) -> Layer | None:
        e = self.edit
        if e is None:
            return None
        v = next((x for x in self.layers() if x.clip == e["clip"] and x.layer == e["layer"] and x.text is not None), None)
        if v is not None:
            e["seen"] = True
        elif e.get("seen"):
            self.edit = None  # the layer went (undo, deleted): the edit ends
        return v

    def start_edit(self, v: Layer, at: QPointF | None, select_all: bool = False) -> None:
        text = str((v.text or {}).get("text") or "")[:MAX_TEXT]
        caret = len(text)
        q = v.to_local(at) if at is not None else None
        if q is not None:
            caret = min(caret_hit(text_lines(v.text), q.x(), q.y(), bool((v.text or {}).get("vertical"))), len(text))
        self.edit = {"clip": v.clip, "layer": v.layer, "text": text, "caret": caret, "anchor": 0 if select_all else caret,
                     "seen": True}
        if select_all:
            self.edit["caret"] = len(text)
        self.overlay.send("graphics.selectLayer", {"clip": v.clip, "layers": [v.layer]})
        self._focus()

    def _focus(self) -> None:
        view = self.overlay.view
        view.setAttribute(Qt.WidgetAttribute.WA_InputMethodEnabled, True)
        view.setFocus()
        view.update()

    def end_edit(self) -> None:
        if self.edit is not None:
            self.edit = None
            self.overlay.view.setAttribute(Qt.WidgetAttribute.WA_InputMethodEnabled, False)
            self.overlay.view.update()

    def new_text(self, at: QPointF, corner: QPointF | None) -> None:
        """A new text layer at ``at`` (point text; paragraph text in the box to ``corner``), then
        typing goes into it (egui's ``new_text_action``)."""
        a = self.to_canvas(at)
        if a is None:
            return
        frame = (self.overlay.data or {}).get("frame")
        fh = _num(frame[1], 1080.0) if isinstance(frame, list) and len(frame) == 2 else 1080.0
        params: dict[str, Any] = {"text": "", "position": [a[1].x(), a[1].y()], "size": round(100.0 * fh / 1080.0)}
        b = self.to_canvas(corner) if corner is not None else None
        if b is not None:
            w, h = abs(b[1].x() - a[1].x()), abs(b[1].y() - a[1].y())
            if w >= 4 and h >= 4:  # smaller was meant as a click
                params["position"] = [min(a[1].x(), b[1].x()), min(a[1].y(), b[1].y())]
                params["box"] = [w, h]
        if a[0] is not None:
            params["clip"] = a[0]
        command = "graphics.newVerticalText" if self.tool == "verticalType" else "graphics.newText"
        engine = self.overlay.engine
        if engine is None:
            return

        def made(r: Any, _b: Any) -> None:
            if isinstance(r, dict) and r.get("clip") is not None and isinstance(r.get("layer"), int):
                self.edit = {"clip": r["clip"], "layer": r["layer"], "text": "", "caret": 0, "anchor": 0, "seen": False}
                self.overlay.send("graphics.selectLayer", {"clip": r["clip"], "layers": [r["layer"]]})
                self._focus()

        engine.execute(command, params, on_reply=made, on_error=lambda _m: None)

    def _set_text(self, text: str) -> None:
        e = self.edit
        if e is None or text == e["text"]:
            return
        e["text"] = text[:MAX_TEXT]
        e["caret"], e["anchor"] = min(e["caret"], len(e["text"])), min(e["anchor"], len(e["text"]))
        # one undo step per typing session: its first change starts it (the engine merges into any
        # Edit Text before it, even another layer's or an earlier session's)
        merge = e.get("sent") is True
        e["sent"] = True
        self.overlay.send("graphics.setText", {"clip": e["clip"], "layer": e["layer"], "text": e["text"], "merge": merge})

    def type_text(self, s: str) -> None:
        """Text from an input method (or an agent) typed at the caret."""
        self._replace(s)
        self.overlay.view.update()

    def _replace(self, s: str) -> None:
        """Typed or pasted ``s`` replaces the selection."""
        e = self.edit
        if e is None:
            return
        s = "".join(c for c in s if c == "\n" or c.isprintable())
        a, b = sorted((e["caret"], e["anchor"]))
        text = e["text"][:a] + s + e["text"][b:]
        if len(text) > MAX_TEXT:
            return
        e["caret"] = e["anchor"] = a + len(s)
        self._set_text(text)

    def text_key(self, ev: QKeyEvent) -> bool:
        """A key while text is edited (egui's ``text_edit``)."""
        e = self.edit
        if e is None:
            return False
        k, mods = ev.key(), ev.modifiers()
        ctrl = bool(mods & Qt.KeyboardModifier.ControlModifier)
        alt = bool(mods & Qt.KeyboardModifier.AltModifier)
        shift = bool(mods & Qt.KeyboardModifier.ShiftModifier)
        text = e["text"]
        a, b = sorted((e["caret"], e["anchor"]))
        v = self.edit_view()
        lines = text_lines(v.text) if v is not None else []
        moved: int | None = None
        if k == Qt.Key.Key_Escape:
            self.end_edit()
            return True
        if k in (Qt.Key.Key_Return, Qt.Key.Key_Enter):
            self._replace("\n")
        elif k == Qt.Key.Key_Backspace:
            if a != b:
                self._replace("")
            elif e["caret"] > 0:
                p = word_left(text, e["caret"]) if ctrl or alt else e["caret"] - 1
                e["caret"] = e["anchor"] = p
                self._set_text(text[:p] + text[b:])
        elif k == Qt.Key.Key_Delete:
            if a != b:
                self._replace("")
            elif e["caret"] < len(text):
                self._set_text(text[:e["caret"]] + text[e["caret"] + 1:])
        elif ctrl and k == Qt.Key.Key_A:
            e["anchor"], e["caret"] = 0, len(text)
        elif ctrl and k in (Qt.Key.Key_C, Qt.Key.Key_X):
            if a < b:
                QGuiApplication.clipboard().setText(text[a:b])
                if k == Qt.Key.Key_X:
                    self._replace("")
        elif ctrl and k == Qt.Key.Key_V:
            self._replace(QGuiApplication.clipboard().text())
        elif k == Qt.Key.Key_Left:
            ln = lines[caret_xy(lines, e["caret"])[2]] if lines else None
            moved = (a if not shift and a != b else ln["start"] if ctrl and ln else word_left(text, e["caret"]) if alt
                     else max(0, e["caret"] - 1))
        elif k == Qt.Key.Key_Right:
            ln = lines[caret_xy(lines, e["caret"])[2]] if lines else None
            moved = (b if not shift and a != b else ln["end"] if ctrl and ln else word_right(text, e["caret"]) if alt
                     else min(len(text), e["caret"] + 1))
        elif k in (Qt.Key.Key_Up, Qt.Key.Key_Down):
            x, _base, li = caret_xy(lines, e["caret"])
            n = li - 1 if k == Qt.Key.Key_Up else li + 1
            if 0 <= n < len(lines):
                moved = caret_hit(lines, x, lines[n]["baseline"])
            else:
                moved = 0 if k == Qt.Key.Key_Up else len(text)
        elif k in (Qt.Key.Key_Home, Qt.Key.Key_End):
            if lines:
                ln = lines[caret_xy(lines, e["caret"])[2]]
                moved = ln["start"] if k == Qt.Key.Key_Home else ln["end"]
            else:
                moved = 0 if k == Qt.Key.Key_Home else len(text)
        elif ev.text() and not ctrl and any(c.isprintable() for c in ev.text()):
            self._replace(ev.text())
        else:
            return k not in (Qt.Key.Key_Shift, Qt.Key.Key_Control, Qt.Key.Key_Alt, Qt.Key.Key_Meta, Qt.Key.Key_Tab)
        if moved is not None:
            e["caret"] = min(max(moved, 0), len(e["text"]))
            if not shift:
                e["anchor"] = e["caret"]
        self.overlay.view.update()
        return True

    def _caret_at(self, v: Layer, at: QPointF) -> int | None:
        q = v.to_local(at)
        if q is None or self.edit is None:
            return None
        return min(caret_hit(text_lines(v.text), q.x(), q.y(), bool((v.text or {}).get("vertical"))), len(self.edit["text"]))

    def hit(self, name: str) -> QPointF | None:
        """``layer.<clip>.<layer>`` (its middle), ``….handle.<n>``, ``….anchor``, ``….corner.<n>``,
        the view coordinates of an element (for tests and agents)."""
        parts = name.split(".")
        if len(parts) < 3 or parts[0] != "layer":
            return None
        v = next((x for x in self.layers() if str(x.clip) == parts[1] and str(x.layer) == parts[2]), None)
        if v is None:
            return None
        q = v.quad()
        if len(parts) == 3:
            return (q[0] + q[2]) / 2
        if parts[3] == "anchor":
            return v.anchor_screen()
        if parts[3] == "handle" and len(parts) == 5 and parts[4].isdigit() and int(parts[4]) < 8:
            return handle_points(q)[int(parts[4])]
        if parts[3] == "corner" and len(parts) == 5 and parts[4].isdigit() and int(parts[4]) < 4:
            ch = v.corner_handles()
            return ch[1][int(parts[4])] if ch is not None else None
        return None

    # ----- painting -----
    def paint(self, p: QPainter) -> None:
        if not self.active():
            return
        accent = theme.current().c("accent")
        white = QColor("white")
        p.setBrush(Qt.BrushStyle.NoBrush)
        ev = self.edit_view()
        if ev is not None:
            p.setPen(QPen(theme.current().c("danger"), 1))
            p.drawPolygon(QPolygonF(ev.quad()))
        for v in self.selected():
            if ev is not None and v.clip == ev.clip and v.layer == ev.layer:
                continue
            q = v.quad()
            p.setPen(QPen(accent, 1))
            p.setBrush(Qt.BrushStyle.NoBrush)
            p.drawPolygon(QPolygonF(q))
            overflow = v.paragraph() and bool((v.text or {}).get("overflow"))
            for n, c in enumerate(q):
                r = QRectF(c.x() - 3.5, c.y() - 3.5, 7, 7)
                if overflow and n == 2:  # text the box hides: a red plus
                    p.fillRect(r.adjusted(-1, -1, 1, 1), theme.current().c("danger"))
                    p.setPen(QPen(white, 1))
                    p.drawLine(c - QPointF(3, 0), c + QPointF(3, 0))
                    p.drawLine(c - QPointF(0, 3), c + QPointF(0, 3))
                else:
                    p.fillRect(r, white)
                    p.setPen(QPen(accent, 1))
                    p.drawRect(r)
            for m in handle_points(q)[4:]:
                p.fillRect(QRectF(m.x() - 2.5, m.y() - 2.5, 5, 5), white)
            ch = v.corner_handles()
            for c in ch[1] if ch is not None else []:
                p.setPen(QPen(accent, 1))
                p.setBrush(white)
                p.drawEllipse(c, 3.0, 3.0)
            p.setBrush(Qt.BrushStyle.NoBrush)
            a = v.anchor_screen()
            p.setPen(QPen(accent, 1))
            p.drawEllipse(a, 4.0, 4.0)
            p.drawLine(a - QPointF(6, 0), a + QPointF(6, 0))
            p.drawLine(a - QPointF(0, 6), a + QPointF(0, 6))
        self._paint_drag(p)
        self._paint_pen(p, accent)
        if ev is not None:
            self._paint_caret(p, ev)

    def _paint_caret(self, p: QPainter, v: Layer) -> None:
        e = self.edit or {}
        lines = text_lines(v.text)
        n = len(str((v.text or {}).get("text") or ""))
        caret, anchor = min(e.get("caret", 0), n), min(e.get("anchor", 0), n)
        p.setPen(Qt.PenStyle.NoPen)
        p.setBrush(QColor(80, 140, 255, 90))
        for x0, y0, x1, y1 in selection_rects(lines, anchor, caret):
            p.drawPolygon(QPolygonF([v.map(x0, y0), v.map(x1, y0), v.map(x1, y1), v.map(x0, y1)]))
        x, base, li = caret_xy(lines, caret)
        asc, desc = (lines[li]["ascent"], lines[li]["descent"]) if lines else (_num((v.text or {}).get("size"), 40.0), 0.0)
        top, bottom = v.map(x, base - asc), v.map(x, base + desc)
        p.setBrush(Qt.BrushStyle.NoBrush)
        p.setPen(QPen(QColor("white"), 1.5))
        p.drawLine(top, bottom)
        p.setPen(QPen(QColor("black"), 0.5))
        p.drawLine(top + QPointF(1, 0), bottom + QPointF(1, 0))

    def _paint_drag(self, p: QPainter) -> None:
        d = self.drag
        if d is None or d.get("cur") is None:
            return
        white = QPen(QColor("white"), 1)
        p.setPen(white)
        p.setBrush(Qt.BrushStyle.NoBrush)
        start, cur, kind, v = d["start"], d["cur"], d["kind"], d.get("view")
        if kind == "move" and v is not None:
            off = d.get("off", cur - start)
            p.drawPolygon(QPolygonF([q + off for q in v.quad()]))
        elif kind == "handle" and v is not None:
            p.drawPolygon(QPolygonF(handle_drag(v, d["n"], start, cur, d.get("shift", False))[0]))
        elif kind == "anchor" and v is not None:
            a = v.anchor_screen() + (cur - start)
            p.drawEllipse(a, 4.0, 4.0)
            p.drawLine(a - QPointF(6, 0), a + QPointF(6, 0))
            p.drawLine(a - QPointF(0, 6), a + QPointF(0, 6))
        elif kind == "corner" and v is not None:
            ch = v.corner_handles()
            if ch is not None:
                r = v.corner_drag(ch[0], d["n"], start, cur)
                p.drawPolygon(v.round_rect(r))
                for c in v.corner_points(r):
                    p.drawEllipse(c, 3.0, 3.0)
        elif kind == "typeClick" and self.tool == "type" and not d.get("onText"):
            p.setPen(QPen(theme.current().c("danger"), 1))
            p.drawRect(QRectF(start, cur).normalized())
        elif kind == "shape":
            r = QRectF(start, cur).normalized()
            if self.tool == "ellipse":
                p.drawEllipse(r)
            else:
                p.drawRect(r)

    def _paint_pen(self, p: QPainter, accent: QColor) -> None:
        if self.tool != "pen" or not self.pen_points:
            return
        target = self.canvas_target()
        if target is None:
            return
        pts = [target[1].map(q) for q in self.pen_points]
        path = QPainterPath(pts[0])
        for q in pts[1:]:
            path.lineTo(q)
        p.setPen(QPen(accent, 1))
        p.setBrush(Qt.BrushStyle.NoBrush)
        p.drawPath(path)
        if self.hover is not None:
            dim = QColor(accent)
            dim.setAlphaF(0.5)
            p.setPen(QPen(dim, 1))
            p.drawLine(pts[-1], self.hover)
        for i, q in enumerate(pts):
            p.fillRect(QRectF(q.x() - 3, q.y() - 3, 6, 6), accent if i == 0 else QColor("white"))

    # ----- mouse -----
    def press(self, e: QMouseEvent) -> bool:
        if not self.active() or e.button() != Qt.MouseButton.LeftButton:
            return False
        at = e.position()
        shift = bool(e.modifiers() & Qt.KeyboardModifier.ShiftModifier)
        ev = self.edit_view()
        if ev is not None and ev.hit(at) and self.tool in ("selection", *TEXT_TOOLS):
            # in the text being edited: the caret goes there (Shift extends the selection)
            c = self._caret_at(ev, at)
            if c is not None and self.edit is not None:
                self.edit["caret"] = c
                if not shift:
                    self.edit["anchor"] = c
            self.drag = {"kind": "textSelect", "view": ev, "start": at, "cur": None}
            self.overlay.view.update()
            return True
        if self.tool in TEXT_TOOLS:
            for v in self.selected():  # the selected layers' handles still work
                for n, h in enumerate(handle_points(v.quad())):
                    if _len(h - at) <= HANDLE_GRAB:
                        self.drag = {"kind": "handle", "n": n, "view": v, "start": at, "cur": None}
                        return True
            on_text = next((x for x in self.layers() if x.text is not None and x.hit(at)), None)
            self.drag = {"kind": "typeClick", "start": at, "cur": None, "onText": on_text}
            return True
        if self.edit is not None:
            self.end_edit()
        if self.tool in ("rectangle", "ellipse"):
            self.drag = {"kind": "shape", "start": at, "cur": None}
            return True
        if self.tool == "pen":
            self.drag = {"kind": "penClick", "start": at, "cur": None}
            return True
        # Selection: the selected layers' anchor point, handles, corner circles; then any layer
        sel = self.selected()
        for v in sel:
            if _len(v.anchor_screen() - at) <= HANDLE_GRAB:
                self.drag = {"kind": "anchor", "view": v, "start": at, "cur": None}
                return True
        for v in sel:
            for n, h in enumerate(handle_points(v.quad())):
                if _len(h - at) <= HANDLE_GRAB:
                    self.drag = {"kind": "handle", "n": n, "view": v, "start": at, "cur": None}
                    return True
        for v in sel:
            ch = v.corner_handles()
            for n, c in enumerate(ch[1] if ch is not None else []):
                if _len(c - at) <= CORNER_GRAB:
                    self.drag = {"kind": "corner", "n": n, "view": v, "start": at, "cur": None}
                    return True
        v = next((x for x in self.layers() if x.hit(at)), None)
        if v is not None:
            if not any(s.clip == v.clip and s.layer == v.layer for s in sel):
                self.overlay.send("graphics.selectLayer", {"clip": v.clip, "layers": [v.layer]})
            self.drag = {"kind": "move", "view": v, "start": at, "cur": None}
            return True
        if sel:  # a click beside every layer lets go of them
            self.drag = {"kind": "deselect", "start": at, "cur": None, "clip": sel[0].clip}
            return True
        return False

    def move(self, e: QMouseEvent) -> bool:
        at = e.position()
        d = self.drag
        if d is None:
            if not self.active():
                return False
            self.hover = at
            self._hover_cursor(at)
            if self.tool == "pen" and self.pen_points:
                self.overlay.view.update()
            return self.tool in DRAW_TOOLS
        d["cur"] = at
        d["shift"] = bool(e.modifiers() & Qt.KeyboardModifier.ShiftModifier)
        if d["kind"] == "textSelect" and self.edit is not None:
            c = self._caret_at(d["view"], at)
            if c is not None:
                self.edit["caret"] = c
        if d["kind"] == "move" and d.get("view") is not None:
            off = at - d["start"]
            lines: list[tuple[bool, float]] = []
            if not e.modifiers() & Qt.KeyboardModifier.ControlModifier:
                off, lines = self.overlay._snap({"quad": d["view"].quad()}, off)
            d["off"] = off
            self.overlay.snap_lines = lines
        self.overlay.view.update()
        return True

    def release(self, e: QMouseEvent) -> bool:
        d, self.drag = self.drag, None
        self.overlay.snap_lines = []
        if d is None:
            return False
        cur = e.position()
        moved = _len(cur - d["start"]) > MOVED
        kind, v = d["kind"], d.get("view")
        send = self.overlay.send
        if kind == "move" and moved and v is not None:
            delta = _linear_inverse(v.canvas_to_screen, d.get("off", cur - d["start"]))
            send("graphics.set", {"clip": v.clip, "layer": v.layer,
                                  "props": {"position": [v.position.x() + delta.x(), v.position.y() + delta.y()]}})
        elif kind == "move" and v is not None:
            send("graphics.selectLayer", {"clip": v.clip, "layers": [v.layer]})
        elif kind == "handle" and moved and v is not None:
            props = handle_drag(v, d["n"], d["start"], cur, bool(e.modifiers() & Qt.KeyboardModifier.ShiftModifier))[1]
            send("graphics.set", {"clip": v.clip, "layer": v.layer, "props": props})
        elif kind == "anchor" and moved and v is not None:
            # the anchor point moves over the layer; Position follows it, so the layer stays
            canvas = _linear_inverse(v.canvas_to_screen, cur - d["start"])
            local = _linear_inverse(v.to_screen, cur - d["start"])
            send("graphics.set", {"clip": v.clip, "layer": v.layer, "props": {
                "position": [v.position.x() + canvas.x(), v.position.y() + canvas.y()],
                "anchor": [v.anchor.x() + local.x(), v.anchor.y() + local.y()]}})
        elif kind == "corner" and moved and v is not None:
            ch = v.corner_handles()
            if ch is not None:
                r = v.corner_drag(ch[0], d["n"], d["start"], cur)
                send("graphics.set", {"clip": v.clip, "layer": v.layer, "props": {"corner_radius": r}})
        elif kind == "deselect" and not moved:
            send("graphics.selectLayer", {"clip": d["clip"], "layers": []})
        elif kind == "shape" and moved:
            a, b = self.to_canvas(d["start"]), self.to_canvas(cur)
            if a is not None and b is not None:
                c = (a[1] + b[1]) / 2
                params: dict[str, Any] = {"shape": self.tool, "position": [c.x(), c.y()],
                                          "size": [abs(b[1].x() - a[1].x()), abs(b[1].y() - a[1].y())]}
                if a[0] is not None:
                    params["clip"] = a[0]
                send("graphics.newShape", params)
        elif kind == "penClick":
            self._pen_click(cur)
        elif kind == "typeClick":
            on_text = d.get("onText")
            if on_text is not None and not moved:
                self.start_edit(on_text, cur)
            elif on_text is None:
                self.end_edit()
                # vertical text is always point text: a click, never a drawn box
                self.new_text(d["start"], cur if moved and self.tool == "type" else None)
        self.overlay.view.update()
        return True

    def _pen_click(self, at: QPointF) -> None:
        target = self.canvas_target()
        if target is None:
            return
        if len(self.pen_points) >= 3 and _len(target[1].map(self.pen_points[0]) - at) <= PEN_CLOSE:
            self.finish_pen()
            return
        c = self.to_canvas(at)
        if c is not None and len(self.pen_points) < 10_000:
            self.pen_points.append(c[1])

    def finish_pen(self) -> None:
        """The path closes: ``graphics.newShape`` path about the points' centre."""
        pts, self.pen_points = self.pen_points, []
        target = self.canvas_target()
        if len(pts) < 3 or target is None:
            return
        cx, cy = sum(q.x() for q in pts) / len(pts), sum(q.y() for q in pts) / len(pts)
        params: dict[str, Any] = {"shape": "path", "position": [cx, cy], "points": [[q.x() - cx, q.y() - cy] for q in pts]}
        if target[0] is not None:
            params["clip"] = target[0]
        self.overlay.send("graphics.newShape", params)

    def double_click(self, e: QMouseEvent) -> bool:
        """The Selection tool: double-clicking a text layer edits it, all its text selected."""
        if self.tool != "selection" or e.button() != Qt.MouseButton.LeftButton:
            return False
        v = next((x for x in self.layers() if x.text is not None and x.hit(e.position())), None)
        if v is None:
            return False
        self.drag = None
        self.start_edit(v, e.position(), select_all=True)
        return True

    def claims_key(self, e: QKeyEvent) -> bool:
        if self.edit is not None:  # typing goes into the text, not to the app's shortcuts
            return True
        return self.tool == "pen" and bool(self.pen_points) and e.key() in (
            Qt.Key.Key_Escape, Qt.Key.Key_Return, Qt.Key.Key_Enter)

    def key(self, e: QKeyEvent) -> bool:
        if self.edit is not None:
            return self.text_key(e)
        if self.tool != "pen" or not self.pen_points:
            return False
        if e.key() == Qt.Key.Key_Escape:
            self.pen_points = []
        elif e.key() in (Qt.Key.Key_Return, Qt.Key.Key_Enter):
            if len(self.pen_points) < 3:
                return True
            self.finish_pen()
        else:
            return False
        self.overlay.view.update()
        return True

    def _hover_cursor(self, at: QPointF) -> None:
        view = self.overlay.view
        ev = self.edit_view()
        if self.tool in TEXT_TOOLS or ev is not None and ev.hit(at):
            view.setCursor(Qt.CursorShape.IBeamCursor)
            return
        if self.tool in DRAW_TOOLS:
            view.setCursor(Qt.CursorShape.CrossCursor)
            return
        for v in self.selected():
            centre = (v.quad()[0] + v.quad()[2]) / 2
            for h in handle_points(v.quad()):
                if _len(h - at) <= HANDLE_GRAB:
                    deg = math.degrees(math.atan2((h - centre).y(), (h - centre).x())) % 180
                    view.setCursor(Qt.CursorShape.SizeHorCursor if not 22.5 <= deg < 157.5 else
                                   Qt.CursorShape.SizeFDiagCursor if deg < 67.5 else
                                   Qt.CursorShape.SizeVerCursor if deg < 112.5 else Qt.CursorShape.SizeBDiagCursor)
                    return
        if any(v.hit(at) for v in self.layers()):
            view.setCursor(Qt.CursorShape.SizeAllCursor)
