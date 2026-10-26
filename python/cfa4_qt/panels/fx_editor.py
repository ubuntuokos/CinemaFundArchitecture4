"""Clip / Track Fx Editor windows, as egui's (``audio_fx_editor.rs``): graphical editors for the
audio effects that need one (Parametric Equalizer curve, Graphic Equalizer 10/20/30 bands,
Multiband Compressor, Dynamics). Opened from Effect Controls ("Custom Setup ▸ Edit…") and from a
track's effect slot in the Audio Track Mixer.

The curves come from the real DSP through the engine (``audio.fxEditor``), so what is drawn is
what is heard. A drag previews its values (sent as ``overrides``, the project is untouched) and
commits one ``effects.setParam`` / ``mixer.setInsert`` on release; Escape during a drag drops it.

Object names (``<fx>`` = effect id): ``fxEditor.<fx>.plot``, ``fxEditor.<fx>.node.<band>`` (EQ
nodes: hp, low, b1, b2, mid, b4, b5, high, lp), ``fxEditor.<fx>.toggle.<param>``,
``fxEditor.<fx>.param.<param>`` (sliders, graphic EQ bands), ``fxEditor.<fx>.xo1…3`` (crossover
handles), ``fxEditor.<fx>.curve[.<band>]`` (transfer curves), ``fxEditor.<fx>.reset``,
``fxEditor.<fx>.close``.
"""

from __future__ import annotations

import math
from collections.abc import Callable
from typing import Any

from PySide6.QtCore import QPointF, QRectF, Qt, Signal
from PySide6.QtGui import (
    QCloseEvent,
    QColor,
    QFont,
    QKeyEvent,
    QMouseEvent,
    QPainter,
    QPainterPath,
    QPaintEvent,
    QPen,
    QWheelEvent,
)
from PySide6.QtWidgets import (
    QDialog,
    QFrame,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QSizePolicy,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from .. import theme
from ..hot_number import HotSpin
from ..i18n import fmt, tr

# effects with a graphical editor (egui's ``has_editor``)
HAS_EDITOR = frozenset({"parametric_eq", "graphic_eq", "graphic_eq_20", "graphic_eq_30", "multiband_compressor", "dynamics_rack"})

F_MIN = 20.0
F_MAX = 20000.0
RESPONSE_BLUE = QColor(0x4D, 0xA3, 0xFF)
TRANSFER_ORANGE = QColor(0xFF, 0xB3, 0x47)
BAND_COLOURS = (QColor(0x3D, 0x6E, 0xB4), QColor(0x3D, 0xA0, 0x6E), QColor(0xB4, 0x9A, 0x3D), QColor(0xB4, 0x4F, 0x3D))

# (node label, parameter prefix, has gain)
PEQ_NODES = (
    ("HP", "hp", False),
    ("L", "low", True),
    ("1", "b1", True),
    ("2", "b2", True),
    ("3", "mid", True),
    ("4", "b4", True),
    ("5", "b5", True),
    ("H", "high", True),
    ("LP", "lp", False),
)
GRAPHIC_BANDS = {"graphic_eq": 10, "graphic_eq_20": 20, "graphic_eq_30": 30}


def finite(value: Any, fallback: float = 0.0) -> float:
    try:
        v = float(value)
    except (TypeError, ValueError):
        return fallback
    return v if math.isfinite(v) else fallback


def fx_of(x: float, r: QRectF) -> float:
    u = max(0.0, min(1.0, (x - r.left()) / max(1.0, r.width())))
    return F_MIN * (F_MAX / F_MIN) ** u


def x_of(f: float, r: QRectF) -> float:
    f = max(F_MIN, min(F_MAX, finite(f, F_MIN)))
    return r.left() + math.log(f / F_MIN) / math.log(F_MAX / F_MIN) * r.width()


def y_of(db: float, r: QRectF, rng: float) -> float:
    return r.center().y() - max(-rng, min(rng, finite(db))) / rng * r.height() * 0.5


def db_of(y: float, r: QRectF, rng: float) -> float:
    return (r.center().y() - y) / max(1.0, r.height() * 0.5) * rng


def small_font(px: float) -> QFont:
    f = QFont()
    f.setPixelSize(max(1, round(px)))
    return f


def target_key(target: dict[str, Any]) -> tuple[Any, ...]:
    if "clip" in target:
        return ("clip", target.get("clip"), target.get("effect"))
    return ("insert", target.get("strip"), target.get("slot"))


class FxEditor(QDialog):
    """One editor window for one target: ``{"clip", "effect"}`` (a clip effect by index) or
    ``{"strip", "slot"}`` (a mixer insert). ``query(params, on_reply, on_error)`` asks the engine
    for ``audio.fxEditor``; edits leave as ``command``."""

    command = Signal(str, dict)
    closed = Signal(object)

    def __init__(self, target: dict[str, Any], query: Callable[..., None], parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.target = dict(target)
        self.query = query
        self.data: dict[str, Any] = {}
        self.fx = ""
        self.drafts: dict[str, float] = {}
        # committed values shown until an answer asked after their edit arrives
        self.settling: dict[str, float] = {}
        self.syncs: list[Any] = []
        self.plots: list[QWidget] = []
        self._busy = False
        self._again = False
        self._gone = False
        self.setModal(False)
        self.setSizeGripEnabled(False)
        self.setObjectName("fxEditor")
        self.outer = QVBoxLayout(self)
        self.outer.setContentsMargins(8, 8, 8, 8)
        self.outer.setSpacing(6)
        self.body: QWidget | None = None
        line = QFrame(self)
        line.setFrameShape(QFrame.Shape.HLine)
        self.outer.addWidget(line)
        row = QHBoxLayout()
        self.close_button = QPushButton(tr("Close"), self)
        self.close_button.setObjectName("fxEditor.close")
        self.close_button.setAutoDefault(False)
        self.close_button.clicked.connect(self.close)
        row.addWidget(self.close_button)
        row.addStretch(1)
        self.outer.addLayout(row)

    @property
    def key(self) -> tuple[Any, ...]:
        return target_key(self.target)

    # ----- engine -----
    def refresh(self) -> None:
        if self._gone:
            return
        if self._busy:
            self._again = True
            return
        self._busy = True
        params = dict(self.target)
        if self.drafts:
            params["overrides"] = dict(self.drafts)
        sent = dict(self.settling)
        self.query(params, lambda r: self._done(r, sent), self._failed)

    def _done(self, result: Any, settled: dict[str, float] | None = None) -> None:
        self._busy = False
        for pid, value in (settled or {}).items():
            if self.settling.get(pid) == value:
                del self.settling[pid]
        if isinstance(result, dict):
            self.set_state(result)
        if self._again:
            self._again = False
            self.refresh()

    def _failed(self, _message: str) -> None:
        # the effect is gone (removed, its clip deleted) or has no editor any more: egui closes
        self._busy = False
        self._again = False
        self.close()

    def set_state(self, data: dict[str, Any]) -> None:
        fx = str(data.get("effect") or "")
        if fx not in HAS_EDITOR:
            self.close()
            return
        self.data = data
        if fx != self.fx:
            self.fx = fx
            self._build()
        kind = tr("Clip Fx Editor") if data.get("kind") == "clip" else tr("Track Fx Editor")
        place = str(data.get("track") or "")
        if data.get("clip") is not None:
            place = f"{place}, {data.get('clip')}"
        self.setWindowTitle(f"{kind} - {tr(str(data.get('name') or fx))}: {place}")
        for w in self.syncs:
            w.sync()
        for p in self.plots:
            p.update()

    # ----- values -----
    def info(self, pid: str) -> dict[str, Any]:
        params = self.data.get("params")
        v = params.get(pid) if isinstance(params, dict) else None
        return v if isinstance(v, dict) else {}

    def v(self, pid: str) -> float:
        if pid in self.drafts:
            return self.drafts[pid]
        if pid in self.settling:
            return self.settling[pid]
        return finite(self.info(pid).get("value"))

    def on(self, pid: str) -> bool:
        return self.v(pid) >= 0.5

    def range(self, pid: str) -> tuple[float, float, str]:
        i = self.info(pid)
        lo, hi = finite(i.get("min"), 0.0), finite(i.get("max"), 1.0)
        return (lo, hi if hi > lo else lo + 1.0, str(i.get("unit") or ""))

    def preview(self, pid: str, value: float) -> None:
        """A live drag: show ``value`` (the curve through the engine) without editing."""
        self.drafts[pid] = value
        self.refresh()

    def commit(self, pid: str, value: float) -> None:
        """One edit of the project (one undo step)."""
        is_bool = bool(self.info(pid).get("bool"))
        val: Any = value >= 0.5 if is_bool else value
        if "clip" in self.target:
            self.command.emit("effects.setParam", {"clip": self.target["clip"], "effect": self.target["effect"], "param": pid, "value": val})
        else:
            self.command.emit("mixer.setInsert", {"strip": self.target["strip"], "slot": self.target["slot"], "params": {pid: val}})
        # the edit is queued before the next query: show the value until that answer does
        self.drafts.pop(pid, None)
        self.settling[pid] = value
        self.refresh()

    def release(self, pids: list[str]) -> None:
        for pid in pids:
            if pid in self.drafts:
                self.commit(pid, self.drafts[pid])

    def cancel(self, pids: list[str]) -> None:
        for pid in pids:
            self.drafts.pop(pid, None)
        self.refresh()
        for w in self.syncs:
            w.sync()
        for p in self.plots:
            p.update()

    # ----- layout -----
    def _build(self) -> None:
        if self.body is not None:
            self.body.setParent(None)
            self.body.deleteLater()
        self.syncs, self.plots = [], []
        self.body = QWidget(self)
        self.outer.insertWidget(0, self.body)
        self.close_button.setObjectName(f"fxEditor.{self.fx}.close")
        if self.fx == "parametric_eq":
            self._parametric(self.body)
        elif self.fx in GRAPHIC_BANDS:
            self._graphic(self.body)
        elif self.fx == "multiband_compressor":
            self._multiband(self.body)
        else:
            self._dynamics(self.body)
        self.adjustSize()

    def toggle(self, parent: QWidget, pid: str, label: str) -> Toggle:
        t = Toggle(self, parent, pid, label)
        self.syncs.append(t)
        return t

    def slider(self, parent: QWidget, pid: str, label: str, log: bool = False, track_w: int = 120) -> FxSlider:
        s = FxSlider(self, parent, pid, label, log, track_w)
        self.syncs.append(s)
        self.plots.append(s)
        return s

    def _parametric(self, body: QWidget) -> None:
        col = QVBoxLayout(body)
        col.setContentsMargins(0, 0, 0, 0)
        plot = ResponsePlot(self, body, 620, 260, 30.0, nodes=True)
        col.addWidget(plot)
        row = QHBoxLayout()
        for label, pre, _gain in PEQ_NODES:
            row.addWidget(self.toggle(body, f"{pre}_on", label))
        row.addStretch(1)
        col.addLayout(row)
        col.addWidget(self.slider(body, "master_gain", tr("Master Gain")))

    def _graphic(self, body: QWidget) -> None:
        n = GRAPHIC_BANDS.get(self.fx, 10)
        w = max(420, n * 26)
        col = QVBoxLayout(body)
        col.setContentsMargins(0, 0, 0, 0)
        col.addWidget(ResponsePlot(self, body, w, 140, 24.0))
        bands = QWidget(body)
        bands.setFixedSize(w, 170)
        colw = w / n
        labels = self.data.get("labels") if isinstance(self.data.get("labels"), list) else []
        for i in range(n):
            label = str(labels[i]) if i < len(labels) else ""
            fader = BandFader(self, bands, f"b{i + 1}", label)
            fader.setGeometry(round(colw * i), 0, max(1, round(colw * (i + 1)) - round(colw * i)), 170)
            self.syncs.append(fader)
        col.addWidget(bands)
        row = QHBoxLayout()
        row.addWidget(self.slider(body, "gain", tr("Master Gain")), 1)
        reset = QPushButton(tr("Reset"), body)
        reset.setObjectName(f"fxEditor.{self.fx}.reset")
        reset.setAutoDefault(False)
        reset.clicked.connect(lambda: self._reset_bands(n))
        row.addWidget(reset)
        col.addLayout(row)

    def _reset_bands(self, n: int) -> None:
        for i in range(n):
            pid = f"b{i + 1}"
            if self.v(pid) != 0.0:
                self.commit(pid, 0.0)

    def _multiband(self, body: QWidget) -> None:
        col = QVBoxLayout(body)
        col.setContentsMargins(0, 0, 0, 0)
        col.addWidget(CrossoverStrip(self, body))
        bands = QHBoxLayout()
        for b in range(1, 5):
            box = QWidget(body)
            box.setFixedWidth(150)
            v = QVBoxLayout(box)
            v.setContentsMargins(0, 0, 0, 0)
            v.setSpacing(2)
            title = QLabel(fmt("Band {n}", n=b), box)
            f = title.font()
            f.setBold(True)
            title.setFont(f)
            v.addWidget(title)
            v.addWidget(TransferPlot(self, box, 110, b - 1, f"fxEditor.{self.fx}.curve.{b}"))
            row = QHBoxLayout()
            row.addWidget(self.toggle(box, f"b{b}_solo", tr("Solo")))
            row.addWidget(self.toggle(box, f"b{b}_bypass", tr("Bypass")))
            row.addStretch(1)
            v.addLayout(row)
            v.addWidget(self.slider(box, f"b{b}_threshold", tr("Thr"), track_w=56))
            v.addWidget(self.slider(box, f"b{b}_ratio", tr("Ratio"), track_w=56))
            v.addWidget(self.slider(box, f"b{b}_attack", tr("Att"), log=True, track_w=56))
            v.addWidget(self.slider(box, f"b{b}_release", tr("Rel"), log=True, track_w=56))
            v.addWidget(self.slider(box, f"b{b}_gain", tr("Gain"), track_w=56))
            v.addStretch(1)
            bands.addWidget(box)
        col.addLayout(bands)
        line = QFrame(body)
        line.setFrameShape(QFrame.Shape.HLine)
        col.addWidget(line)
        row = QHBoxLayout()
        row.addWidget(self.slider(body, "output", tr("Output Gain")), 1)
        row.addWidget(self.toggle(body, "lim_on", tr("Limiter")))
        row.addWidget(self.toggle(body, "link", tr("Link Channels")))
        col.addLayout(row)
        row = QHBoxLayout()
        row.addWidget(self.slider(body, "lim_threshold", tr("Limiter Threshold")), 1)
        row.addWidget(self.slider(body, "lim_release", tr("Limiter Release"), log=True), 1)
        col.addLayout(row)

    def _dynamics(self, body: QWidget) -> None:
        row = QHBoxLayout(body)
        row.setContentsMargins(0, 0, 0, 0)
        row.addWidget(TransferPlot(self, body, 220, 0, f"fxEditor.{self.fx}.curve"), 0, Qt.AlignmentFlag.AlignTop)

        def column(items: list[tuple[str, str, str, bool] | None]) -> QWidget:
            w = QWidget(body)
            w.setFixedWidth(330)
            v = QVBoxLayout(w)
            v.setContentsMargins(0, 0, 0, 0)
            v.setSpacing(2)
            for it in items:
                if it is None:
                    line = QFrame(w)
                    line.setFrameShape(QFrame.Shape.HLine)
                    v.addWidget(line)
                    continue
                kind, pid, label, log = it
                v.addWidget(self.toggle(w, pid, label) if kind == "t" else self.slider(w, pid, label, log), 0, Qt.AlignmentFlag.AlignLeft if kind == "t" else Qt.AlignmentFlag(0))
            v.addStretch(1)
            return w

        row.addWidget(column([
            ("t", "gate_on", tr("Auto Gate"), False),
            ("s", "gate_threshold", tr("Threshold"), False),
            ("s", "gate_attack", tr("Attack"), True),
            ("s", "gate_release", tr("Release"), True),
            ("s", "gate_hold", tr("Hold"), False),
            None,
            ("t", "comp_on", tr("Compressor"), False),
            ("s", "comp_threshold", tr("Threshold"), False),
            ("s", "comp_ratio", tr("Ratio"), False),
            ("s", "comp_attack", tr("Attack"), True),
            ("s", "comp_release", tr("Release"), True),
            ("t", "comp_auto", tr("Auto Makeup"), False),
            ("s", "comp_makeup", tr("Makeup"), False),
        ]))
        row.addWidget(column([
            ("t", "exp_on", tr("Expander"), False),
            ("s", "exp_threshold", tr("Threshold"), False),
            ("s", "exp_ratio", tr("Ratio"), False),
            None,
            ("t", "lim_on", tr("Limiter"), False),
            ("s", "lim_threshold", tr("Threshold"), False),
            ("s", "lim_release", tr("Release"), True),
            ("t", "soft_clip", tr("Soft Clip"), False),
            None,
            ("s", "output", tr("Output Gain"), False),
        ]))

    def closeEvent(self, e: QCloseEvent) -> None:
        self._gone = True
        self.closed.emit(self)
        super().closeEvent(e)


class Toggle(QToolButton):
    """egui's selectable label for an on/off parameter."""

    def __init__(self, ed: FxEditor, parent: QWidget, pid: str, label: str) -> None:
        super().__init__(parent)
        self.ed = ed
        self.pid = pid
        self.setObjectName(f"fxEditor.{ed.fx}.toggle.{pid}")
        self.setText(label)
        self.setCheckable(True)
        self.setAutoRaise(True)
        self.clicked.connect(lambda _c=False: ed.commit(pid, 0.0 if ed.on(pid) else 1.0))

    def sync(self) -> None:
        self.blockSignals(True)
        self.setChecked(self.ed.on(self.pid))
        self.blockSignals(False)


class FxSlider(QWidget):
    """egui's slider: a track with a handle (drag previews, release commits), the value as a hot
    number (typed: commits; scrubbed: previews, commits at its end) and the label."""

    def __init__(self, ed: FxEditor, parent: QWidget, pid: str, label: str, log: bool, track_w: int = 120) -> None:
        super().__init__(parent)
        self.TRACK_W = track_w
        self.ed = ed
        self.pid = pid
        lo, hi, unit = ed.range(pid)
        self.log = log and lo > 0.0
        self.dragging = False
        self.setObjectName(f"fxEditor.{ed.fx}.param.{pid}")
        self.setFixedHeight(22)
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)
        self.setSizePolicy(QSizePolicy.Policy.Preferred, QSizePolicy.Policy.Fixed)
        h = QHBoxLayout(self)
        h.setContentsMargins(self.TRACK_W + 6, 0, 0, 0)
        h.setSpacing(4)
        # egui shows up to two decimals: whole numbers on wide ranges
        decimals = 0 if hi - lo >= 1000 else 1 if hi - lo >= 20 else 2
        self.spin = HotSpin(self, lo, hi, decimals, max((hi - lo) / 300.0, 0.01))
        self.spin.setObjectName(f"fxEditor.{ed.fx}.param.{pid}.value")
        self.spin.edited.connect(self._edited)
        self.spin.released.connect(lambda: ed.release([pid]))
        h.addWidget(self.spin)
        text = QLabel(f"{label} {unit}".strip(), self)
        h.addWidget(text)
        h.addStretch(1)

    def track(self) -> QRectF:
        return QRectF(6, self.height() / 2 - 2, self.TRACK_W - 12, 4)

    def frac(self, v: float) -> float:
        lo, hi, _ = self.ed.range(self.pid)
        if self.log:
            return math.log(max(lo, min(hi, v)) / lo) / math.log(hi / lo)
        return (max(lo, min(hi, v)) - lo) / (hi - lo)

    def value_at(self, x: float) -> float:
        lo, hi, _ = self.ed.range(self.pid)
        t = self.track()
        u = max(0.0, min(1.0, (x - t.left()) / max(1.0, t.width())))
        v = lo * (hi / lo) ** u if self.log else lo + u * (hi - lo)
        return round(v, 2)

    def sync(self) -> None:
        self.spin.set_value(self.ed.v(self.pid))
        self.update()

    def _edited(self, value: float, begin: bool) -> None:
        if self.spin._scrubbing:
            self.ed.preview(self.pid, value)
        elif begin:
            self.ed.commit(self.pid, value)

    def paintEvent(self, _e: QPaintEvent) -> None:
        t = theme.current()
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        tr_ = self.track()
        p.setPen(Qt.PenStyle.NoPen)
        p.setBrush(t.c("field_border"))
        p.drawRoundedRect(tr_, 2, 2)
        x = tr_.left() + self.frac(self.ed.v(self.pid)) * tr_.width()
        p.setBrush(t.c("accent"))
        p.drawRoundedRect(QRectF(tr_.left(), tr_.top(), x - tr_.left(), tr_.height()), 2, 2)
        p.setBrush(t.c("eq_knob_active") if self.dragging else t.c("eq_knob"))
        p.drawEllipse(QPointF(x, tr_.center().y()), 6, 6)

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() == Qt.MouseButton.LeftButton and e.position().x() <= self.TRACK_W:
            self.dragging = True
            self.setFocus(Qt.FocusReason.MouseFocusReason)
            self.ed.preview(self.pid, self.value_at(e.position().x()))
            self.update()

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        if self.dragging:
            self.ed.preview(self.pid, self.value_at(e.position().x()))
            self.update()

    def mouseReleaseEvent(self, _e: QMouseEvent) -> None:
        if self.dragging:
            self.dragging = False
            self.ed.release([self.pid])
            self.update()

    def keyPressEvent(self, e: QKeyEvent) -> None:
        if e.key() == Qt.Key.Key_Escape and self.dragging:
            self.dragging = False
            self.ed.cancel([self.pid])
            return
        super().keyPressEvent(e)


def paint_frequency_grid(p: QPainter, r: QRectF, rng: float) -> None:
    t = theme.current()
    p.fillRect(r, t.c("plot_bg"))
    p.setFont(small_font(9))
    for f in (50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0):
        x = x_of(f, r)
        p.setPen(QPen(t.c("plot_grid"), 1))
        p.drawLine(QPointF(x, r.top()), QPointF(x, r.bottom()))
        p.setPen(t.c("text_faint"))
        label = f"{f / 1000:g}k" if f >= 1000 else f"{f:g}"
        p.drawText(QPointF(x + 2, r.bottom() - 3), label)
    step = 12.0 if rng > 30.0 else 6.0
    d = -rng
    while d <= rng + 1e-9:
        y = y_of(d, r, rng)
        p.setPen(QPen(t.c("plot_axis") if abs(d) < 1e-9 else t.c("plot_grid"), 1))
        p.drawLine(QPointF(r.left(), y), QPointF(r.right(), y))
        p.setPen(t.c("text_faint"))
        p.drawText(QPointF(r.left() + 2, y - 2), f"{d:+.0f}")
        d += step


def points(value: Any) -> list[tuple[float, float]]:
    out = []
    for pt in value if isinstance(value, list) else []:
        if isinstance(pt, list) and len(pt) >= 2:
            x, y = finite(pt[0], math.nan), finite(pt[1], math.nan)
            if math.isfinite(x) and math.isfinite(y):
                out.append((x, y))
    return out


class ResponsePlot(QWidget):
    """The frequency response on a log-frequency grid; the parametric EQ's nodes on it."""

    def __init__(self, ed: FxEditor, parent: QWidget, w: int, h: int, rng: float, nodes: bool = False) -> None:
        super().__init__(parent)
        self.ed = ed
        self.rng = rng
        self.setObjectName(f"fxEditor.{ed.fx}.plot")
        self.setFixedSize(w, h)
        ed.plots.append(self)
        self.nodes: list[EqNode] = []
        if nodes:
            for label, pre, gain in PEQ_NODES:
                n = EqNode(ed, self, label, pre, gain)
                self.nodes.append(n)
                ed.syncs.append(n)

    def area(self) -> QRectF:
        return QRectF(self.rect())

    def paintEvent(self, _e: QPaintEvent) -> None:
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        r = self.area()
        paint_frequency_grid(p, r, self.rng)
        pts = points(self.ed.data.get("response"))
        if len(pts) >= 2:
            path = QPainterPath()
            for i, (f, db) in enumerate(pts):
                q = QPointF(x_of(f, r), y_of(db, r, self.rng))
                path.moveTo(q) if i == 0 else path.lineTo(q)
            p.setPen(QPen(RESPONSE_BLUE, 1.5))
            p.setBrush(Qt.BrushStyle.NoBrush)
            p.drawPath(path)


class EqNode(QWidget):
    """A parametric EQ band's node: drag sets frequency (and gain), the wheel its Q, a
    double-click switches the band on or off."""

    SIZE = 16

    def __init__(self, ed: FxEditor, plot: ResponsePlot, label: str, pre: str, gain: bool) -> None:
        super().__init__(plot)
        self.ed = ed
        self.plot = plot
        self.label = label
        self.pre = pre
        self.gain = gain
        self.dragging = False
        self.setObjectName(f"fxEditor.{ed.fx}.node.{pre}")
        self.setFixedSize(self.SIZE, self.SIZE)
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)

    def pids(self) -> list[str]:
        return [f"{self.pre}_freq", f"{self.pre}_gain"] if self.gain else [f"{self.pre}_freq"]

    def centre(self) -> QPointF:
        r = self.plot.area()
        g = self.ed.v(f"{self.pre}_gain") if self.gain else 0.0
        return QPointF(x_of(self.ed.v(f"{self.pre}_freq"), r), y_of(g, r, self.plot.rng))

    def sync(self) -> None:
        c = self.centre()
        self.move(round(c.x() - self.SIZE / 2), round(c.y() - self.SIZE / 2))
        self.update()

    def paintEvent(self, _e: QPaintEvent) -> None:
        t = theme.current()
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        col = t.c("eq_node") if self.ed.on(f"{self.pre}_on") else t.c("eq_node_off")
        p.setPen(QPen(col, 1.5))
        p.drawEllipse(QPointF(self.SIZE / 2, self.SIZE / 2), 7, 7)
        p.setFont(small_font(8.5))
        p.drawText(QRectF(self.rect()), Qt.AlignmentFlag.AlignCenter, self.label)

    def _drag_to(self, e: QMouseEvent) -> None:
        pos = self.plot.mapFromGlobal(e.globalPosition().toPoint())
        r = self.plot.area()
        lo, hi, _ = self.ed.range(f"{self.pre}_freq")
        self.ed.drafts[f"{self.pre}_freq"] = max(lo, min(hi, float(round(fx_of(pos.x(), r)))))
        if self.gain:
            glo, ghi, _ = self.ed.range(f"{self.pre}_gain")
            db = round(db_of(pos.y(), r, self.plot.rng) * 10.0) / 10.0
            self.ed.drafts[f"{self.pre}_gain"] = max(glo, min(ghi, db))
        self.ed.refresh()
        self.sync()

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() == Qt.MouseButton.LeftButton:
            self.dragging = True
            self._moved = False

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        if self.dragging:
            self._moved = True
            self._drag_to(e)

    def mouseReleaseEvent(self, _e: QMouseEvent) -> None:
        if self.dragging:
            self.dragging = False
            if getattr(self, "_moved", False):
                self.ed.release(self.pids())

    def mouseDoubleClickEvent(self, _e: QMouseEvent) -> None:
        pid = f"{self.pre}_on"
        self.ed.commit(pid, 0.0 if self.ed.on(pid) else 1.0)

    def wheelEvent(self, e: QWheelEvent) -> None:
        if not self.gain:
            return
        s = e.angleDelta().y()
        if s == 0:
            return
        qid = f"{self.pre}_q"
        lo, hi, _ = self.ed.range(qid)
        q = max(lo, min(hi, self.ed.v(qid) * (1.1 if s > 0 else 1.0 / 1.1)))
        self.ed.commit(qid, round(q * 100.0) / 100.0)
        e.accept()

    def keyPressEvent(self, e: QKeyEvent) -> None:
        if e.key() == Qt.Key.Key_Escape and self.dragging:
            self.dragging = False
            self.ed.cancel(self.pids())
            return
        super().keyPressEvent(e)


class BandFader(QWidget):
    """A graphic EQ band: a ±24 dB fader (0.5 dB steps), its value and its frequency;
    double-click: 0 dB."""

    def __init__(self, ed: FxEditor, parent: QWidget, pid: str, label: str) -> None:
        super().__init__(parent)
        self.ed = ed
        self.pid = pid
        self.label = label
        self.dragging = False
        self.setObjectName(f"fxEditor.{ed.fx}.param.{pid}")
        self.setToolTip(label)
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)

    def track(self) -> QRectF:
        return QRectF(self.width() / 2 - 3, 5, 6, 130)

    def sync(self) -> None:
        self.update()

    def paintEvent(self, _e: QPaintEvent) -> None:
        t = theme.current()
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        tr_ = self.track()
        x = tr_.center().x()
        p.setPen(Qt.PenStyle.NoPen)
        p.setBrush(t.c("eq_track"))
        p.drawRoundedRect(tr_, 2, 2)
        p.setPen(QPen(t.c("eq_tick"), 1))
        p.drawLine(QPointF(x - 5, tr_.center().y()), QPointF(x + 5, tr_.center().y()))
        v = self.ed.v(self.pid)
        ky = tr_.center().y() - max(-24.0, min(24.0, v)) / 24.0 * tr_.height() * 0.5
        kw = min(float(self.width()), 18.0)
        p.setPen(Qt.PenStyle.NoPen)
        p.setBrush(t.c("eq_knob_active") if self.dragging or self.underMouse() else t.c("eq_knob"))
        p.drawRoundedRect(QRectF(x - kw / 2, ky - 4, kw, 8), 2, 2)
        p.setFont(small_font(8.5))
        p.setPen(t.c("text_dim"))
        p.drawText(QRectF(0, self.height() - 30, self.width(), 14), Qt.AlignmentFlag.AlignCenter, f"{v:+.0f}")
        p.setPen(t.c("text_faint"))
        short = self.label.replace(" Hz", "").replace(" kHz", "k")
        p.drawText(QRectF(0, self.height() - 15, self.width(), 14), Qt.AlignmentFlag.AlignCenter, short)

    def _value_at(self, y: float) -> float:
        tr_ = self.track()
        db = max(-24.0, min(24.0, (tr_.center().y() - y) / (tr_.height() * 0.5) * 24.0))
        return round(db * 2.0) / 2.0

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() == Qt.MouseButton.LeftButton:
            self.dragging = True
            self._moved = False

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        if self.dragging:
            self._moved = True
            self.ed.preview(self.pid, self._value_at(e.position().y()))
            self.update()

    def mouseReleaseEvent(self, _e: QMouseEvent) -> None:
        if self.dragging:
            self.dragging = False
            if getattr(self, "_moved", False):
                self.ed.release([self.pid])
            self.update()

    def mouseDoubleClickEvent(self, _e: QMouseEvent) -> None:
        self.ed.commit(self.pid, 0.0)

    def keyPressEvent(self, e: QKeyEvent) -> None:
        if e.key() == Qt.Key.Key_Escape and self.dragging:
            self.dragging = False
            self.ed.cancel([self.pid])
            return
        super().keyPressEvent(e)


class CrossoverStrip(QWidget):
    """The multiband compressor's spectrum strip: four bands between three crossover handles."""

    def __init__(self, ed: FxEditor, parent: QWidget) -> None:
        super().__init__(parent)
        self.ed = ed
        self.setFixedSize(640, 70)
        ed.plots.append(self)
        self.handles = [CrossoverHandle(ed, self, k) for k in range(3)]
        ed.syncs.extend(self.handles)

    def edges(self) -> list[float]:
        r = QRectF(self.rect())
        return [r.left(), *(x_of(self.ed.v(f"xo{k + 1}"), r) for k in range(3)), r.right()]

    def paintEvent(self, _e: QPaintEvent) -> None:
        t = theme.current()
        p = QPainter(self)
        r = QRectF(self.rect())
        p.fillRect(r, t.c("plot_bg"))
        e = self.edges()
        p.setFont(small_font(10))
        for b in range(4):
            br = QRectF(QPointF(e[b], r.top() + 8), QPointF(e[b + 1], r.bottom() - 8))
            c = QColor(BAND_COLOURS[b])
            c.setAlphaF(0.35)
            p.fillRect(br, c)
            p.setPen(t.c("text"))
            p.drawText(br, Qt.AlignmentFlag.AlignCenter, fmt("Band {n}", n=b + 1))
        p.setFont(small_font(9))
        for k in range(3):
            h = self.handles[k]
            x = e[k + 1]
            p.setPen(QPen(t.c("crossover_handle"), 2.5 if h.dragging or h.underMouse() else 1.5))
            p.drawLine(QPointF(x, r.top()), QPointF(x, r.bottom()))
            p.setPen(t.c("text_dim"))
            p.drawText(QPointF(x + 3, r.top() + 11), f"{self.ed.v(f'xo{k + 1}'):.0f} Hz")


class CrossoverHandle(QWidget):
    def __init__(self, ed: FxEditor, strip: CrossoverStrip, k: int) -> None:
        super().__init__(strip)
        self.ed = ed
        self.strip = strip
        self.pid = f"xo{k + 1}"
        self.dragging = False
        self.setObjectName(f"fxEditor.{ed.fx}.{self.pid}")
        self.setFixedSize(10, strip.height())
        self.setCursor(Qt.CursorShape.SizeHorCursor)
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)
        self.setAttribute(Qt.WidgetAttribute.WA_Hover)

    def sync(self) -> None:
        x = x_of(self.ed.v(self.pid), QRectF(self.strip.rect()))
        self.move(round(x - 5), 0)

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() == Qt.MouseButton.LeftButton:
            self.dragging = True
            self._moved = False

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        if self.dragging:
            self._moved = True
            pos = self.strip.mapFromGlobal(e.globalPosition().toPoint())
            lo, hi, _ = self.ed.range(self.pid)
            self.ed.preview(self.pid, max(lo, min(hi, float(round(fx_of(pos.x(), QRectF(self.strip.rect())))))))
            self.sync()
            self.strip.update()

    def mouseReleaseEvent(self, _e: QMouseEvent) -> None:
        if self.dragging:
            self.dragging = False
            if getattr(self, "_moved", False):
                self.ed.release([self.pid])
            self.strip.update()

    def keyPressEvent(self, e: QKeyEvent) -> None:
        if e.key() == Qt.Key.Key_Escape and self.dragging:
            self.dragging = False
            self.ed.cancel([self.pid])
            return
        super().keyPressEvent(e)


class TransferPlot(QWidget):
    """A compressor's input → output curve (−60 … 0 dB) for one band."""

    LO = -60.0

    def __init__(self, ed: FxEditor, parent: QWidget, size: int, band: int, name: str) -> None:
        super().__init__(parent)
        self.ed = ed
        self.band = band
        self.setObjectName(name)
        self.setFixedSize(size, size)
        ed.plots.append(self)

    def map(self, x: float, y: float) -> QPointF:
        r = QRectF(self.rect())
        lo = self.LO
        fy = max(-0.05, min(1.05, (y - lo) / -lo))
        return QPointF(r.left() + (x - lo) / -lo * r.width(), r.bottom() - fy * r.height())

    def paintEvent(self, _e: QPaintEvent) -> None:
        t = theme.current()
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        r = QRectF(self.rect())
        lo = self.LO
        p.fillRect(r, t.c("plot_bg"))
        p.setPen(QPen(t.c("plot_grid"), 1))
        for d in (-48.0, -36.0, -24.0, -12.0):
            p.drawLine(self.map(d, lo), self.map(d, 0.0))
            p.drawLine(self.map(lo, d), self.map(0.0, d))
        p.setPen(QPen(t.c("plot_axis"), 1))
        p.drawLine(self.map(lo, lo), self.map(0.0, 0.0))
        curves = self.ed.data.get("transfer")
        pts = points(curves[self.band]) if isinstance(curves, list) and self.band < len(curves) else []
        if len(pts) >= 2:
            path = QPainterPath()
            for i, (x, y) in enumerate(pts):
                q = self.map(x, y)
                path.moveTo(q) if i == 0 else path.lineTo(q)
            p.setPen(QPen(TRANSFER_ORANGE, 1.5))
            p.setBrush(Qt.BrushStyle.NoBrush)
            p.drawPath(path)
        p.setFont(small_font(8.5))
        p.setPen(t.c("text_faint"))
        p.drawText(r.adjusted(3, 2, -3, -2), Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignTop, tr("out"))
        p.drawText(r.adjusted(3, 2, -3, -2), Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignBottom, tr("in"))
