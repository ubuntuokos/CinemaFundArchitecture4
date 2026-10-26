"""Lumetri Scopes, as egui's (crates/ui-egui/src/panels/scopes.rs): Vectorscope YUV / HLS,
Histogram, Parade (RGB / YUV / RGB-White) and Waveform (RGB / Luma / YC / YC no Chroma) of the
Program frame, in a grid when several are shown, with egui's graticules, labels, footer (colour
space, wrench, Clamp Signal, 8 Bit / Float / HDR) and settings menu (presets, the scopes, types,
colour space, brightness, vectorscope targets). The traces are the engine's (``scopes.image``).
"""

from __future__ import annotations

import json
import math
from dataclasses import asdict, dataclass, field
from typing import Any

from PySide6.QtCore import QPointF, QRectF, QSettings, Qt, Signal
from PySide6.QtGui import QColor, QContextMenuEvent, QFont, QImage, QPainter, QPaintEvent, QPen
from PySide6.QtWidgets import QCheckBox, QComboBox, QHBoxLayout, QLabel, QMenu, QToolButton, QVBoxLayout, QWidget

from .. import theme
from ..i18n import N_, fmt, tr
from .transport import icon_color, transport_icon

KINDS = (
    ("vectorscopeYuv", N_("Vectorscope YUV")),
    ("vectorscopeHls", N_("Vectorscope HLS")),
    ("histogram", N_("Histogram")),
    ("parade", N_("Parade")),
    ("waveform", N_("Waveform")),
)
WAVEFORM_TYPES = (("rgb", N_("RGB")), ("luma", N_("Luma")), ("yc", N_("YC")), ("ycNoChroma", N_("YC no Chroma")))
PARADE_TYPES = (("rgb", N_("RGB")), ("yuv", N_("YUV")), ("rgbWhite", N_("RGB-White")))
COLOR_SPACES = (("auto", N_("Auto")), ("rec601", N_("Rec. 601")), ("rec709", N_("Rec. 709")), ("rec2100", N_("Rec. 2100")))
BRIGHTNESS = (("dimmed", N_("Dimmed")), ("normal", N_("Normal")), ("bright", N_("Bright")))
SCALES = (("bits8", N_("8 Bit")), ("float", N_("Float")), ("hdr", N_("HDR")))
TARGETS = (("75", "75%"), ("100", "100%"))
# egui's SCOPE_PRESETS: (name, scopes, waveform type, parade type, scale, clamp)
PRESETS = (
    (N_("Waveform (RGB)"), ["waveform"], "rgb", "rgb", "bits8", True),
    (N_("Waveform (Luma)"), ["waveform"], "luma", "rgb", "bits8", True),
    (N_("Parade (RGB)"), ["parade"], "rgb", "rgb", "bits8", True),
    (N_("Vectorscope (YUV)"), ["vectorscopeYuv"], "rgb", "rgb", "bits8", True),
    (N_("Vectorscope + Waveform YC"), ["vectorscopeYuv", "waveform"], "yc", "rgb", "bits8", True),
    (N_("Four Scopes (YUV, float, no clamp)"), ["vectorscopeYuv", "histogram", "parade", "waveform"], "yc", "yuv", "float", False),
    (N_("Four Scopes (RGB, 8 bit)"), ["vectorscopeYuv", "histogram", "parade", "waveform"], "rgb", "rgb", "bits8", True),
    (N_("HLS Vectorscope + Histogram"), ["vectorscopeHls", "histogram"], "rgb", "rgb", "bits8", True),
)
ORDER = [k for k, _l in KINDS]
GRATICULE = QColor(0x2A, 0x2A, 0x2A)
LABEL = QColor(0xB0, 0xB0, 0xB0)
TITLE = QColor(110, 110, 110)
FOOTER_H = 30


@dataclass
class ScopesState:
    shown: list[str] = field(default_factory=lambda: ["parade"])
    waveform_type: str = "rgb"
    parade_type: str = "rgb"
    color_space: str = "auto"
    brightness: str = "normal"
    scale: str = "bits8"
    clamp: bool = True
    targets: str = "75"
    preset: str = ""

    def toggle(self, kind: str) -> None:
        if kind in self.shown:
            if len(self.shown) > 1:
                self.shown.remove(kind)
        else:
            self.shown.append(kind)
            self.shown.sort(key=ORDER.index)

    def apply_preset(self, name: str) -> bool:
        p = next((p for p in PRESETS if p[0] == name), None)
        if p is None:
            return False
        self.shown, self.waveform_type, self.parade_type, self.scale, self.clamp = list(p[1]), p[2], p[3], p[4], p[5]
        self.preset = name
        return True

    def params(self, kind: str, rows: int) -> dict[str, Any]:
        return {
            "kind": kind,
            "rows": rows,
            "waveformType": self.waveform_type,
            "paradeType": self.parade_type,
            "colorSpace": self.color_space,
            "brightness": self.brightness,
            "scale": self.scale,
            "clamp": self.clamp,
        }

    @classmethod
    def load(cls, settings: QSettings | None, key: str) -> ScopesState:
        st = cls()
        raw = settings.value(key) if settings is not None else None
        try:
            data = json.loads(raw) if isinstance(raw, str) else None
        except ValueError:
            data = None
        if isinstance(data, dict):
            ok = {
                "waveform_type": dict(WAVEFORM_TYPES),
                "parade_type": dict(PARADE_TYPES),
                "color_space": dict(COLOR_SPACES),
                "brightness": dict(BRIGHTNESS),
                "scale": dict(SCALES),
                "targets": dict(TARGETS),
            }
            for name, allowed in ok.items():
                if isinstance(data.get(name), str) and data[name] in allowed:  # saved state is untrusted
                    setattr(st, name, data[name])
            raw_shown = data.get("shown")
            shown = [k for k in raw_shown if isinstance(k, str) and k in ORDER] if isinstance(raw_shown, list) else []
            st.shown = sorted(set(shown), key=ORDER.index) or st.shown
            st.clamp = bool(data.get("clamp", True))
            st.preset = str(data.get("preset") or "")
        return st

    def save(self, settings: QSettings | None, key: str) -> None:
        if settings is not None:
            settings.setValue(key, json.dumps(asdict(self)))


def layout(area: QRectF, n: int) -> list[QRectF]:
    """Cells for ``n`` scopes: the grid whose cells are closest to 4:3 (egui's layout)."""
    if n <= 0:
        return []
    best, score = 1, math.inf
    for cols in range(1, n + 1):
        rows = -(-n // cols)
        a = (area.width() / cols) / max(1.0, area.height() / rows)
        s = abs(math.log(max(1e-6, a / 1.33))) + (0.15 if cols * rows > n else 0.0)
        if s < score:
            best, score = cols, s
    rows = -(-n // best)
    cw, ch = area.width() / best, area.height() / rows
    return [QRectF(area.left() + (i % best) * cw, area.top() + (i // best) * ch, cw, ch).adjusted(2, 2, -2, -2) for i in range(n)]


def kind_title(kind: str, st: ScopesState) -> str:
    if kind == "parade":
        return fmt("Parade ({kind})", kind=tr(dict(PARADE_TYPES)[st.parade_type]))
    if kind == "waveform":
        return fmt("Waveform ({kind})", kind=tr(dict(WAVEFORM_TYPES)[st.waveform_type]))
    return tr(dict(KINDS)[kind])


class ScopesView(QWidget):
    """The scopes of one frame, drawn in a grid (the panel's body; the Reference Monitor's
    Lumetri Scopes display)."""

    menu_requested = Signal(object)  # global position

    def __init__(self, engine: Any, name: str = "scopes", parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName(f"{name}.view")
        self.engine = engine
        self.st = ScopesState()
        self.time: int | None = None
        self.revision: Any = -1  # what the frame shows: (sequence id, project revision)
        self.images: dict[str, tuple[tuple[Any, ...], QImage, dict[str, Any]]] = {}
        self._busy = False
        self._queue: list[tuple[str, tuple[Any, ...], dict[str, Any]]] = []
        self.message = ""
        self.setMinimumSize(120, 90)

    def set_frame(self, time: int | None, revision: Any) -> None:
        self.time, self.revision = time, revision
        self.request()

    def cells(self) -> list[tuple[str, QRectF]]:
        return list(zip(self.st.shown, layout(QRectF(self.rect()), len(self.st.shown)), strict=False))

    def request(self) -> None:
        if self.time is None or not self.isVisible():
            self.update()
            return
        settings = tuple(sorted(asdict(self.st).items(), key=lambda kv: kv[0]))
        settings = tuple((k, tuple(v) if isinstance(v, list) else v) for k, v in settings)
        self._queue = []
        for kind, cell in self.cells():
            rows = int(min(256, max(64, (cell.height() - 20) * self.devicePixelRatioF())))
            key = (self.time, self.revision, settings, kind, rows)
            if self.images.get(kind, ((),))[0] != key:
                self._queue.append((kind, key, {**self.st.params(kind, rows), "time": self.time}))
        self._pump()
        self.update()

    def _pump(self) -> None:
        if self._busy or not self._queue or not getattr(self.engine, "running", False):
            return
        kind, key, params = self._queue.pop(0)
        self._busy = True
        self.engine.call(
            "scopes.image",
            params,
            on_reply=lambda r, b, k=kind, kk=key: self._done(k, kk, r, b),
            on_error=lambda m: self._failed(m),
        )

    def _done(self, kind: str, key: tuple[Any, ...], result: Any, data: bytes | None) -> None:
        self._busy = False
        if isinstance(result, dict) and data:
            w, h = int(result.get("w") or 0), int(result.get("h") or 0)
            if w > 0 and h > 0 and len(data) == w * h * 4:
                img = QImage(data, w, h, w * 4, QImage.Format.Format_RGBA8888_Premultiplied).copy()
                self.images[kind] = (key, img, result)
                self.message = ""
        self.update()
        self._pump()

    def _failed(self, message: str) -> None:
        self._busy = False
        self.message = message
        self.update()
        self._pump()

    def resizeEvent(self, e) -> None:
        super().resizeEvent(e)
        self.request()

    def showEvent(self, e) -> None:
        super().showEvent(e)
        self.request()

    def contextMenuEvent(self, e: QContextMenuEvent) -> None:
        self.menu_requested.emit(e.globalPos())

    # ----- drawing -----
    def paintEvent(self, _e: QPaintEvent) -> None:
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        p.fillRect(self.rect(), QColor(0, 0, 0))
        if self.time is None:
            p.setPen(theme.current().c("text_faint"))
            p.drawText(self.rect(), Qt.AlignmentFlag.AlignCenter, tr("(no sequence)"))
            return
        cells = self.cells()
        if len(cells) > 1:
            p.setPen(QPen(QColor(24, 24, 24), 1))
            for _k, c in cells:
                p.drawRect(c.adjusted(-1, -1, 1, 1))
        for kind, cell in cells:
            self._draw_one(p, kind, cell)
        if self.message:
            p.setPen(theme.current().c("danger"))
            p.drawText(self.rect().adjusted(6, 6, -6, -6), Qt.AlignmentFlag.AlignBottom | Qt.TextFlag.TextWordWrap, self.message)

    def _draw_one(self, p: QPainter, kind: str, cell: QRectF) -> None:
        img_entry = self.images.get(kind)
        meta = img_entry[2] if img_entry else {}
        small = QFont(self.font())
        small.setPixelSize(9)
        if kind in ("waveform", "parade", "histogram"):
            plot = cell.adjusted(34, 10, -34, -10)
            if plot.width() < 8 or plot.height() < 8:
                return
            lo, hi = (float(meta.get("lo", 0.0)), float(meta.get("hi", 1.0))) if meta else ((0.0, 1.0) if self.st.clamp or kind == "histogram" else (-0.1, 1.1))
            self._level_graticule(p, plot, lo, hi, meta)
            if img_entry:
                p.drawImage(plot, img_entry[1])
            if kind == "parade":
                n = int(meta.get("parades") or (4 if self.st.parade_type == "rgbWhite" else 3))
                p.setPen(QPen(QColor(40, 40, 40), 1))
                for i in range(1, n):
                    x = plot.left() + plot.width() * i / n
                    p.drawLine(QPointF(x, plot.top()), QPointF(x, plot.bottom()))
            p.setFont(small)
            p.setPen(TITLE)
            p.drawText(plot.topLeft() + QPointF(4, 11), kind_title(kind, self.st))
        else:
            side = max(8.0, min(cell.width(), cell.height()) - 16)
            sq = QRectF(cell.center().x() - side / 2, cell.center().y() - side / 2, side, side)
            self._vector_graticule(p, sq, kind == "vectorscopeHls", meta)
            if img_entry:
                p.drawImage(sq, img_entry[1])
            p.setFont(small)
            p.setPen(TITLE)
            p.drawText(cell.topLeft() + QPointF(6, 13), kind_title(kind, self.st))

    def _level_graticule(self, p: QPainter, plot: QRectF, lo: float, hi: float, meta: dict[str, Any]) -> None:
        f = QFont(self.font())
        f.setPixelSize(10)
        p.setFont(f)

        def y_of(v: float) -> float:
            return plot.bottom() - (v - lo) / max(1e-9, hi - lo) * plot.height()

        ticks = meta.get("nitsTicks") if meta.get("nits") else None
        if ticks:
            for nits, pq in ticks:
                y = y_of(float(pq))
                p.setPen(QPen(QColor(255, 200, 80, 90) if float(nits) == 203.0 else GRATICULE, 1))
                p.drawLine(QPointF(plot.left(), y), QPointF(plot.right(), y))
                p.setPen(LABEL)
                p.drawText(QRectF(plot.left() - 34, y - 7, 30, 14), Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter, str(int(nits)))
            f.setPixelSize(9)
            p.setFont(f)
            p.drawText(QRectF(plot.right() + 4, plot.top(), 34, 12), Qt.AlignmentFlag.AlignLeft, "cd/m²")
            return
        for i in range(11):
            v = i / 10
            y = y_of(v)
            p.setPen(QPen(GRATICULE, 1))
            p.drawLine(QPointF(plot.left(), y), QPointF(plot.right(), y))
            p.setPen(LABEL)
            p.drawText(QRectF(plot.left() - 34, y - 7, 30, 14), Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter, str(i * 10))
            right = f"{v:.1f}" if self.st.scale == "float" else str(round(v * 255))
            p.drawText(QRectF(plot.right() + 4, y - 7, 30, 14), Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter, right)

    def _vector_graticule(self, p: QPainter, sq: QRectF, hls: bool, meta: dict[str, Any]) -> None:
        c = sq.center()
        extent = float(meta.get("extent") or 0.6)
        scale = sq.width() / 2 / extent

        def at(cb: float, cr: float) -> QPointF:
            return QPointF(c.x() + cb * scale, c.y() - cr * scale)

        ring = 0.5 * scale
        p.setBrush(Qt.BrushStyle.NoBrush)
        p.setPen(QPen(QColor(64, 64, 64), 1))
        p.drawEllipse(c, ring, ring)
        p.setPen(QPen(GRATICULE, 1))
        p.drawEllipse(c, ring * 0.5, ring * 0.5)
        p.drawLine(QPointF(c.x() - ring, c.y()), QPointF(c.x() + ring, c.y()))
        p.drawLine(QPointF(c.x(), c.y() - ring), QPointF(c.x(), c.y() + ring))
        p.setPen(QPen(QColor(64, 64, 64), 1))
        for d in range(0, 360, 10):
            a = math.radians(d)
            ix, iy = math.cos(a), math.sin(a)
            length = 0.94 if d % 30 == 0 else 0.97
            p.drawLine(QPointF(c.x() + ix * ring * length, c.y() - iy * ring * length), QPointF(c.x() + ix * ring, c.y() - iy * ring))
        f = QFont(self.font())
        f.setPixelSize(10)
        p.setFont(f)
        if hls:
            p.setPen(LABEL)
            for t in meta.get("hls") or []:
                pt = at(float(t["cb"]) * 1.12, float(t["cr"]) * 1.12)
                p.drawText(QRectF(pt.x() - 12, pt.y() - 7, 24, 14), Qt.AlignmentFlag.AlignCenter, str(t["name"]))
            return
        a = math.radians(float(meta.get("skinToneDeg") or 123.0))
        p.setPen(QPen(QColor(220, 170, 120, 110), 1))
        p.drawLine(c, QPointF(c.x() + math.cos(a) * ring, c.y() - math.sin(a) * ring))
        for group in meta.get("targets") or []:
            amp = float(group.get("amplitude") or 1.0)
            strong = (amp == 0.75) == (self.st.targets == "75")
            for tg in group.get("points") or []:
                pt = at(float(tg["cb"]), float(tg["cr"]))
                rgb = tg.get("rgb") or [0, 0, 0]
                col = QColor(*(min(255, int(float(v) / amp * 200) + 40) for v in rgb[:3]))
                if not strong:
                    col.setAlphaF(0.45)
                size = 9 if strong else 5
                p.setPen(QPen(col, 1))
                p.drawRect(QRectF(pt.x() - size / 2, pt.y() - size / 2, size, size))
                if strong:
                    v = QPointF(float(tg["cb"]), -float(tg["cr"]))
                    norm = math.hypot(v.x(), v.y()) or 1.0
                    off = QPointF(v.x() / norm * 12, v.y() / norm * 12)
                    q = pt + off
                    p.drawText(QRectF(q.x() - 12, q.y() - 7, 24, 14), Qt.AlignmentFlag.AlignCenter, str(tg["name"]))


class ScopesPanel(QWidget):
    """The Lumetri Scopes panel: the view and egui's footer."""

    def __init__(self, engine: Any, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.LumetriScopes")
        self.settings: QSettings | None = None
        self.view = ScopesView(engine, "scopes", self)
        self.view.menu_requested.connect(lambda pos: self.settings_menu().exec(pos))
        self.space = QLabel(self)
        self.space.setObjectName("scopes.space")
        self.wrench = QToolButton(self)
        self.wrench.setObjectName("scopes.wrench")
        self.wrench.setToolTip(tr("Scope settings"))
        self.wrench.setAutoRaise(True)
        self.wrench.setIcon(transport_icon("wrench", icon_color(self)))
        self.wrench.clicked.connect(lambda: self.settings_menu().exec(self.wrench.mapToGlobal(self.wrench.rect().bottomLeft())))
        self.clamp = QCheckBox(tr("Clamp Signal"), self)
        self.clamp.setObjectName("scopes.clamp")
        self.clamp.toggled.connect(lambda on: self._change(clamp=bool(on), preset=""))
        self.scale = QComboBox(self)
        self.scale.setObjectName("scopes.scale")
        for k, label in SCALES:
            self.scale.addItem(tr(label), k)
        self.scale.activated.connect(lambda _i: self._change(scale=str(self.scale.currentData()), preset=""))
        footer = QHBoxLayout()
        footer.setContentsMargins(8, 2, 6, 2)
        footer.addWidget(self.space)
        footer.addStretch(1)
        footer.addWidget(self.wrench)
        footer.addWidget(self.clamp)
        footer.addWidget(self.scale)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(0)
        layout.addWidget(self.view, 1)
        layout.addLayout(footer)
        self._sync()

    @property
    def st(self) -> ScopesState:
        return self.view.st

    def attach_settings(self, settings: QSettings | None) -> None:
        self.settings = settings
        self.view.st = ScopesState.load(settings, "panels/scopes")
        self._sync()

    def _sync(self) -> None:
        self.clamp.blockSignals(True)
        self.clamp.setChecked(self.st.clamp)
        self.clamp.blockSignals(False)
        self.scale.setCurrentIndex(max(0, self.scale.findData(self.st.scale)))
        space = dict(COLOR_SPACES)[self.st.color_space]
        meta_space = next((m[2].get("colorSpace") for m in self.view.images.values() if m[2].get("colorSpace")), None)
        self.space.setText(tr(meta_space) if self.st.color_space == "auto" and meta_space else tr(space))

    def _change(self, **changes: Any) -> None:
        for k, v in changes.items():
            setattr(self.st, k, v)
        self.st.save(self.settings, "panels/scopes")
        self._sync()
        self.view.request()

    def set_frame(self, time: int | None, revision: Any) -> None:
        self.view.set_frame(time, revision)
        self._sync()

    def settings_menu(self) -> QMenu:
        """egui's settings menu (wrench and right-click)."""
        st = self.st
        m = QMenu(self)
        m.setObjectName("scopes.menu")

        def item(menu: QMenu, oid: str, label: str, on: bool, apply: Any) -> None:
            a = menu.addAction(label)
            a.setObjectName(oid)
            a.setCheckable(True)
            a.setChecked(on)
            a.triggered.connect(lambda _c=False: (apply(), self.st.save(self.settings, "panels/scopes"), self._sync(), self.view.request()))

        presets = m.addMenu(tr("Presets"))
        presets.setObjectName("scopes.menu.presets")
        for i, pr in enumerate(PRESETS):
            item(presets, f"scopes.menu.preset.{i}", tr(pr[0]), st.preset == pr[0], lambda n=pr[0]: st.apply_preset(n))
        m.addSeparator()
        for k, _label in KINDS:
            item(m, f"scopes.menu.{k}", kind_title(k, st), k in st.shown, lambda kk=k: st.toggle(kk))
        m.addSeparator()

        def sub(oid: str, title: str, options: tuple[tuple[str, str], ...], attr: str, clears: bool) -> None:
            menu = m.addMenu(tr(title))
            menu.setObjectName(f"scopes.menu.{oid}")
            for k, label in options:

                def apply(kk: str = k) -> None:
                    setattr(st, attr, kk)
                    if clears:
                        st.preset = ""

                item(menu, f"scopes.menu.{oid}.{k}", tr(label), getattr(st, attr) == k, apply)

        sub("paradeType", N_("Parade Type"), PARADE_TYPES, "parade_type", True)
        sub("waveformType", N_("Waveform Type"), WAVEFORM_TYPES, "waveform_type", True)
        sub("targets", N_("Vectorscope Targets"), TARGETS, "targets", False)
        m.addSeparator()
        sub("colorSpace", N_("Colour Space"), COLOR_SPACES, "color_space", False)
        sub("brightness", N_("Brightness"), BRIGHTNESS, "brightness", False)
        sub("scale", N_("Signal Scale"), SCALES, "scale", True)

        def toggle_clamp() -> None:
            st.clamp = not st.clamp
            st.preset = ""

        item(m, "scopes.menu.clamp", tr("Clamp Signal"), st.clamp, toggle_clamp)
        return m
