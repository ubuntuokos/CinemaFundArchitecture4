"""Effect Controls: the selected clip's effects and their parameters, with keyframes.

Laid out as egui's panel: the effect list on the left, the keyframe lane on the right with a
time ruler over the clip's stretch of the sequence (click or drag it to move the playhead) and the
clip's bar, a divider between them that sets the list's width, and the timecode under them. A
selected video clip is listed with the audio linked to it ("Video" and "Audio" headings), the
fixed effects (Motion, Opacity, Volume…) first and a line above the first added one. An effect
twirls open or shut, its "fx" switch turns it off, a click selects it, its context menu saves it
as a preset or clears it.

All of it is the engine's: ``effects.inspect`` describes the clip (definitions, the value at the
playhead, keyframes in media and timeline time) and every change is an engine command
(``effects.setParam``, ``effects.toggleAnimation``, ``effects.addKeyframe``,
``effects.deleteKeyframe``, ``effects.setInterpolation``, ``effects.moveKeyframes``,
``effects.toggleEnabled``, ``effects.reset``, ``effects.resetParam``, ``effects.remove``,
``effects.select``, ``presets.save``). Editing an animated parameter writes a keyframe at the
playhead, as in egui. A drag of one value is one undo step: changes are sent with ``merge`` and a
new step is begun when the user pauses.

Keyframes in the lane: a click selects one (Shift adds or removes) and moves the playhead to it;
a drag on the lane's background draws a marquee (Shift adds to the selection); dragging a
selected keyframe moves the whole selection along the timeline, on frames and inside the clips
(Shift snaps to the playhead and other keyframes), one ``effects.moveKeyframes`` on release; Esc
cancels; right-click sets the interpolation or clears the keyframe. Edit ▸ Select All with the
panel focused selects every keyframe it shows.
"""

from __future__ import annotations

import itertools
import math
from typing import Any

from PySide6.QtCore import QElapsedTimer, QPoint, QPointF, QRectF, Qt, QTimer, Signal
from PySide6.QtGui import QColor, QFont, QIcon, QKeyEvent, QMouseEvent, QPainter, QPaintEvent, QPen, QPixmap, QPolygonF
from PySide6.QtWidgets import (
    QApplication,
    QButtonGroup,
    QCheckBox,
    QColorDialog,
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QFrame,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QMenu,
    QPlainTextEdit,
    QPushButton,
    QRadioButton,
    QScrollArea,
    QSizePolicy,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from .. import theme
from ..hot_number import HotSpin
from ..i18n import N_, fmt, tr
from ..timecode import TICKS_PER_SECOND, format_timecode
from .fx_editor import HAS_EDITOR

# a pause longer than this starts a new undo step for the next change of the same value
GESTURE_GAP_MS = 800
SEND_DELAY_MS = 120
INTERPOLATIONS = (
    ("linear", N_("Linear")),
    ("bezier", N_("Bezier")),
    ("autoBezier", N_("Auto Bezier")),
    ("continuousBezier", N_("Continuous Bezier")),
    ("hold", N_("Hold")),
    ("easeIn", N_("Ease In")),
    ("easeOut", N_("Ease Out")),
)
UNEDITABLE = {"curve", "wheel", "path"}
ROW_H = 24
HEAD_H = 18
RULER_H = 20
CLIP_BAR_H = 16
MIN_LIST_W = 200
MIN_LANE_W = 80
DIVIDER_W = 6
LANE_PAD_L = 4
LANE_PAD_R = 6
# keyframe hit radius and the Shift snap distance, in pixels
KEY_HIT = 7.0
SNAP_PX = 8.0
UNSELECTED_KEY = QColor("#b0b0b0")


def color_to_qcolor(value: Any) -> QColor:
    if isinstance(value, list) and len(value) >= 3:
        try:
            r, g, b = (max(0.0, min(1.0, float(v))) for v in value[:3])
            a = max(0.0, min(1.0, float(value[3]))) if len(value) > 3 else 1.0
            return QColor.fromRgbF(r, g, b, a)
        except (TypeError, ValueError):
            pass
    return QColor("#000000")


def eyedropper_icon() -> QIcon:
    """A pipette: a slanted tube with its bulb, drawn in the text colour."""
    pm = QPixmap(16, 16)
    pm.fill(Qt.GlobalColor.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.RenderHint.Antialiasing)
    c = theme.current().c("text_dim")
    p.setPen(QPen(c, 2, Qt.PenStyle.SolidLine, Qt.PenCapStyle.RoundCap))
    p.drawLine(QPointF(3, 13), QPointF(10, 6))
    p.setPen(Qt.PenStyle.NoPen)
    p.setBrush(c)
    p.drawEllipse(QPointF(11.5, 4.5), 2.8, 2.8)
    p.end()
    return QIcon(pm)


def qcolor_to_value(color: QColor) -> list[float]:
    return [round(color.redF(), 6), round(color.greenF(), 6), round(color.blueF(), 6), round(color.alphaF(), 6)]


def finite(value: Any, fallback: float = 0.0) -> float:
    try:
        v = float(value)
    except (TypeError, ValueError):
        return fallback
    return v if math.isfinite(v) else fallback


def as_int(value: Any, fallback: int = 0) -> int:
    """An integer from engine JSON (ticks, ids), never a bool or a float."""
    return value if isinstance(value, int) and not isinstance(value, bool) else fallback


def span(clip: dict[str, Any] | None) -> tuple[int, int]:
    """A clip snapshot's [start, end) on the timeline; never empty."""
    c = clip or {}
    start = as_int(c.get("start"))
    return start, max(as_int(c.get("end"), start + 1), start + 1)


def frame_of(clip: dict[str, Any] | None) -> int:
    return max(1, as_int((clip or {}).get("frame"), TICKS_PER_SECOND // 30))


def snap_frame(t: int, frame: int) -> int:
    """The frame boundary nearest ``t``."""
    return (t + frame // 2) // frame * frame


def param_text(info: dict[str, Any], value: Any) -> str:
    """Read-only text for a parameter without an inline editor (a LUT, a curve), as egui's."""
    if isinstance(info.get("display"), str):
        return info["display"]
    kind = info.get("type")
    if kind == "curve" and isinstance(value, list):
        if not value or value == [[0.0, 0.0], [1.0, 1.0]]:
            return tr("Default")
        return fmt("Custom ({n} points)", n=len(value))
    if isinstance(value, str):
        return value or tr("None")
    return tr("Edited in the Lumetri and mask tools (not yet in the Qt GUI)")


def ruler_steps(base: int) -> list[int]:
    """Candidate ruler intervals in frames (egui's), ``base`` frames to the timecode second."""
    base = max(1, base)
    return [1, 2, 5, 10, base // 2, base, base * 2, base * 5, base * 10, base * 15, base * 30, base * 60, base * 120,
            base * 300, base * 600, base * 1800, base * 3600]


def paint_key(p: QPainter, c: QPointF, interpolation: Any, color: QColor) -> None:
    """A keyframe as egui draws it: a square for Hold, a diamond for Linear, a dot for the eases."""
    p.setPen(Qt.PenStyle.NoPen)
    p.setBrush(color)
    if interpolation == "hold":
        p.drawRect(QRectF(c.x() - 4.5, c.y() - 4.5, 9, 9))
    elif interpolation in (None, "linear"):
        r = 5.5
        p.drawPolygon(QPolygonF([QPointF(c.x(), c.y() - r), QPointF(c.x() + r, c.y()), QPointF(c.x(), c.y() + r),
                                 QPointF(c.x() - r, c.y())]))
    else:
        p.drawEllipse(c, 5.5, 5.5)


# commands the engine runs at its playhead (animated values are evaluated or keyed there)
AT_PLAYHEAD = frozenset({"effects.toggleAnimation", "effects.resetParam", "effects.reset"})


class LinedRow(QWidget):
    """A row of the effect list: a thin line under it, so it can be followed into the lane."""

    def paintEvent(self, _e: QPaintEvent) -> None:
        p = QPainter(self)
        t = theme.current()
        if getattr(self, "highlight", False):
            p.fillRect(self.rect(), t.c("row_selected"))
        elif self.underMouse() and getattr(self, "hoverable", False):
            p.fillRect(self.rect(), t.c("hover"))
        if getattr(self, "accent_line", False):
            accent = t.c("accent")
            accent.setAlphaF(0.8)
            p.fillRect(QRectF(0, 0, self.width(), 2), accent)
        sep = t.c("separator")
        sep.setAlphaF(0.55)
        p.setPen(sep)
        p.drawLine(0, self.height() - 1, self.width(), self.height() - 1)


class EffectHeader(LinedRow):
    """An effect's row: twirl, "fx" switch, name, reset; a click selects the effect."""

    def __init__(self, panel: EffectControlsPanel, parent: QWidget, clip: int, effect: dict[str, Any], name: str,
                 first_added: bool) -> None:
        super().__init__(parent)
        self.panel = panel
        self.clip = clip
        self.index = as_int(effect.get("index"))
        self.effect_id = str(effect.get("effect") or "")
        self.intrinsic = bool(effect.get("intrinsic"))
        self.label = tr(str(effect.get("name") or self.effect_id))
        self.hoverable = True
        self.highlight = bool(effect.get("selected"))
        self.accent_line = first_added
        self.setObjectName(name)
        self.setFixedHeight(ROW_H)
        self.setAttribute(Qt.WidgetAttribute.WA_Hover)
        h = QHBoxLayout(self)
        h.setContentsMargins(2, 0, 4, 0)
        h.setSpacing(2)
        self.twirl = QToolButton(self)
        self.twirl.setObjectName(f"{name}.twirl")
        self.twirl.setAutoRaise(True)
        self.twirl.setToolTip(tr("Twirl open or shut"))
        self.twirl.clicked.connect(lambda: panel.toggle_collapsed(self.clip, self.index))
        self.fx = QToolButton(self)
        self.fx.setObjectName(f"{name}.enabled")
        self.fx.setText("fx")
        self.fx.setCheckable(True)
        self.fx.setAutoRaise(True)
        self.fx.setToolTip(tr("Toggle the effect on or off"))
        self.fx.toggled.connect(lambda _on: panel.send("effects.toggleEnabled", {"clip": self.clip, "index": self.index}))
        title = QLabel(self.label, self)
        font = title.font()
        font.setBold(True)
        title.setFont(font)
        title.setSizePolicy(QSizePolicy.Policy.Ignored, QSizePolicy.Policy.Preferred)
        reset = QToolButton(self)
        reset.setObjectName(f"{name}.reset")
        reset.setText("↺")
        reset.setAutoRaise(True)
        reset.setToolTip(tr("Reset Effect"))
        reset.clicked.connect(lambda: panel.send("effects.reset", {"clip": self.clip, "index": self.index}))
        h.addWidget(self.twirl)
        h.addWidget(self.fx)
        h.addWidget(title, 1)
        h.addWidget(reset)
        self.set_state(bool(effect.get("enabled", True)), bool(effect.get("selected")))

    def set_open(self, is_open: bool) -> None:
        self.twirl.setText("▾" if is_open else "▸")

    def set_state(self, enabled: bool, selected: bool) -> None:
        self.fx.blockSignals(True)
        self.fx.setChecked(enabled)
        self.fx.blockSignals(False)
        font = self.fx.font()
        font.setStrikeOut(not enabled)
        self.fx.setFont(font)
        if self.highlight != selected:
            self.highlight = selected
            self.update()

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() == Qt.MouseButton.LeftButton:
            self.panel.send("effects.select", {"clip": self.clip, "effect": self.index})
        super().mousePressEvent(e)

    def contextMenuEvent(self, e: Any) -> None:
        menu = QMenu(self)
        save = menu.addAction(tr("Save Preset…"))
        save.setObjectName("effectControls.menu.savePreset")
        save.triggered.connect(lambda: self.panel.save_preset(self.clip, [self.index], self.label))
        if not self.intrinsic:
            clear = menu.addAction(tr("Clear"))
            clear.setObjectName("effectControls.menu.clear")
            clear.triggered.connect(lambda: self.panel.send("effects.remove", {"clip": self.clip, "index": self.index}))
        self.panel.last_menu = menu
        menu.exec(e.globalPos())


class CustomSetupRow(LinedRow):
    """Premiere's "Custom Setup ▸ Edit…" row of an audio effect with a graphical editor (egui's
    ``custom_setup_row``): opens its Clip Fx Editor window."""

    def __init__(self, panel: EffectControlsPanel, parent: QWidget, clip: int, index: int, effect: str) -> None:
        super().__init__(parent)
        self.setFixedHeight(ROW_H)
        label = QLabel(tr("Custom Setup"), self)
        label.setStyleSheet(f"color: {theme.current().text_dim};")
        label.move(42, 0)
        label.setFixedHeight(ROW_H)
        self.edit = QPushButton(tr("Edit…"), self)
        self.edit.setObjectName(f"effectControls.effect.{effect}.edit")
        self.edit.setAutoDefault(False)
        self.edit.setGeometry(160, 2, 60, ROW_H - 4)
        self.edit.clicked.connect(lambda: panel.fx_editor.emit({"clip": clip, "effect": index}))


class ParamRow(LinedRow):
    """One parameter: stopwatch, label, editor, keyframe navigation (◀ ◆ ▶) and reset. Its
    keyframes are drawn in the panel's lane, on this row's line."""

    def __init__(self, panel: EffectControlsPanel, parent: QWidget, clip: int, effect: int, info: dict[str, Any],
                 name: str, mask: int | None = None) -> None:
        super().__init__(parent)
        self.panel = panel
        self.clip = clip
        self.effect = effect
        self.mask = mask
        self.info = info
        self.id = str(info.get("id"))
        self.kind = str(info.get("type") or "")
        self.snapshot: dict[str, Any] = {}
        self.setObjectName(name)
        self.setFixedHeight(ROW_H)
        h = QHBoxLayout(self)
        h.setContentsMargins(0, 0, 4, 0)
        h.setSpacing(2)
        # twirl-down for the value / velocity graphs (animated scalar and point parameters)
        self.graph_twirl = QToolButton(self)
        self.graph_twirl.setObjectName(f"{name}.graphs")
        self.graph_twirl.setAutoRaise(True)
        self.graph_twirl.setText("▸")
        self.graph_twirl.setToolTip(tr("Show graphs"))
        self.graph_twirl.setFixedWidth(14)
        policy = self.graph_twirl.sizePolicy()
        policy.setRetainSizeWhenHidden(True)
        self.graph_twirl.setSizePolicy(policy)
        self.graph_twirl.hide()
        self.graph_twirl.clicked.connect(lambda: self.panel.toggle_graph(self))
        self.graph: GraphRow | None = None
        self.stopwatch = QToolButton(self)
        self.stopwatch.setObjectName(f"{name}.animate")
        self.stopwatch.setText("⏱")
        self.stopwatch.setCheckable(True)
        self.stopwatch.setAutoRaise(True)
        self.stopwatch.setToolTip(tr("Toggle Animation"))
        animatable = bool(info.get("animatable")) and (self.kind not in UNEDITABLE or (self.kind == "path" and mask is not None))
        self.stopwatch.setEnabled(animatable)
        if not animatable:
            self.stopwatch.setText("")
        self.stopwatch.clicked.connect(self._toggle_animation)
        label = QLabel(tr(str(info.get("label") or self.id)), self)
        label.setToolTip(label.text())
        label.setMinimumWidth(40)
        label.setSizePolicy(QSizePolicy.Policy.Ignored, QSizePolicy.Policy.Preferred)
        unit = str(info.get("unit") or "")
        self.editor: QWidget
        self.spins: list[HotSpin] = []
        if self.kind in ("float", "angle"):
            spin = self._spin(self, info, unit) if self.kind == "float" else self._spin(
                self, {"speed": 0.5, "decimals": 1, "min": -36000, "max": 36000}, "°")
            self.spins = [spin]
            self.editor = spin
        elif self.kind == "point":
            box = QWidget(self)
            bh = QHBoxLayout(box)
            bh.setContentsMargins(0, 0, 0, 0)
            bh.setSpacing(2)
            point = {"speed": 1.0, "decimals": 1, "min": -100_000, "max": 100_000}
            self.spins = [self._spin(box, point, ""), self._spin(box, point, "")]
            for s in self.spins:
                bh.addWidget(s)
            self.editor = box
        elif self.kind == "bool":
            check = QCheckBox(self)
            check.toggled.connect(lambda on: self.panel.set_param(self, on, immediate=True))
            self.editor = check
        elif self.kind == "choice":
            combo = QComboBox(self)
            for i, opt in enumerate(info.get("options") or []):
                combo.addItem(tr(str(opt)), i)
            combo.currentIndexChanged.connect(lambda i: self.panel.set_param(self, int(i), immediate=True))
            self.editor = combo
        elif self.kind == "color":
            button = QPushButton(self)
            button.clicked.connect(self._pick_color)
            self.editor = button
        elif self.kind == "path" and mask is not None:
            self.editor = MaskTracking(self, name)
        elif self.kind == "text" and not self.id.endswith("_lut"):
            line = QLineEdit(self)
            line.editingFinished.connect(lambda: self.panel.set_param(self, line.text(), immediate=True))
            self.editor = line
        else:
            note = QLabel(self)
            note.setEnabled(False)
            self.editor = note
        self.editor.setObjectName(f"{name}.value")
        if not self.spins:
            self.editor.setMinimumWidth(56)
            self.editor.setSizePolicy(QSizePolicy.Policy.Ignored, self.editor.sizePolicy().verticalPolicy())
        self.prev_key = self._nav_button("◀", N_("Go to Previous Keyframe"), f"{name}.prevKeyframe", self._prev)
        self.add_key = self._nav_button("◇", N_("Add/Remove Keyframe"), f"{name}.keyframe", self._add_remove)
        self.next_key = self._nav_button("▶", N_("Go to Next Keyframe"), f"{name}.nextKeyframe", self._next)
        reset = QToolButton(self)
        reset.setObjectName(f"{name}.reset")
        reset.setText("↺")
        reset.setAutoRaise(True)
        reset.setToolTip(tr("Reset Parameter"))
        reset.clicked.connect(lambda: self.panel.send(
            "effects.resetParam", {"clip": self.clip, "effect": self.effect, "param": self.id}
        ))
        h.addWidget(self.graph_twirl)
        h.addWidget(self.stopwatch)
        h.addWidget(label, 2)
        h.addWidget(self.editor, 3)
        self.eyedropper: QToolButton | None = None
        if self.kind == "color":
            # arm it, then click a pixel in the Program monitor (Esc cancels), as in egui
            self.eyedropper = QToolButton(self)
            self.eyedropper.setObjectName(f"{name}.eyedropper")
            self.eyedropper.setIcon(eyedropper_icon())
            self.eyedropper.setCheckable(True)
            self.eyedropper.setAutoRaise(True)
            self.eyedropper.setToolTip(tr("Eyedropper (pick a color from the Program monitor)"))
            self.eyedropper.clicked.connect(lambda on: self.panel.arm_eyedropper(self.target() if on else None))
            h.addWidget(self.eyedropper)
        for b in (self.prev_key, self.add_key, self.next_key):
            h.addWidget(b)
        h.addWidget(reset)
        if mask is not None:  # a mask's parameters have no reset button, as in egui
            policy = reset.sizePolicy()
            policy.setRetainSizeWhenHidden(True)
            reset.setSizePolicy(policy)
            reset.hide()
        self.animated = False
        self.keyframes: list[dict[str, Any]] = []

    @property
    def graphable(self) -> bool:
        return self.kind in ("float", "angle", "point")

    @property
    def key(self) -> tuple[int, int, str]:
        return (self.clip, self.effect, self.id if self.mask is None else f"mask{self.mask}.{self.id}")

    def target(self) -> dict[str, Any]:
        """The parameter in the shape the effects.* commands take (with its mask, if any)."""
        out: dict[str, Any] = {"clip": self.clip, "effect": self.effect, "param": self.id}
        if self.mask is not None:
            out["mask"] = self.mask
        return out

    def _spin(self, parent: QWidget, info: dict[str, Any], suffix: str) -> HotSpin:
        """egui's hot number: drag to scrub (one undo step), click to type a value or a calculation."""
        lo = finite(info.get("min"), -1_000_000.0)
        hi = finite(info.get("max"), 1_000_000.0)
        soft_lo, soft_hi = finite(info.get("softMin"), lo), finite(info.get("softMax"), hi)
        speed = finite(info.get("speed"), max(0.01, (soft_hi - soft_lo) / 400.0))  # egui's
        decimals = int(finite(info.get("decimals"), 2 if hi - lo > 10 else 3))
        spin = HotSpin(parent, lo, hi, max(0, min(6, decimals)), speed, f" {suffix}" if suffix else "")
        # narrow docks: let the value fields shrink instead of pushing the buttons out of view
        spin.setSizePolicy(QSizePolicy.Policy.Ignored, QSizePolicy.Policy.Fixed)
        spin.edited.connect(lambda _v, begin, s=spin: self._edited(begin, s))
        return spin

    def _edited(self, begin: bool, spin: HotSpin) -> None:
        if begin:  # a typed value or a new scrub: its own undo step
            self.panel.new_gesture()
        self.panel.set_param(self, self._spin_value(), immediate=begin and not spin._scrubbing)

    def _spin_value(self) -> Any:
        if self.kind == "point":
            return [self.spins[0].value(), self.spins[1].value()]
        return self.spins[0].value()

    def _nav_button(self, text: str, tip: str, name: str, slot: Any) -> QToolButton:
        b = QToolButton(self)
        b.setObjectName(name)
        b.setText(text)
        b.setToolTip(tr(tip))
        b.setAutoRaise(True)
        b.setFixedWidth(18)
        # hidden while the parameter is not animated, keeping its place so values line up
        policy = b.sizePolicy()
        policy.setRetainSizeWhenHidden(True)
        b.setSizePolicy(policy)
        b.clicked.connect(slot)
        return b

    def mousePressEvent(self, e: QMouseEvent) -> None:
        # clicking one of an effect's properties selects the effect, as its header does
        if e.button() == Qt.MouseButton.LeftButton and self.mask is None:
            self.panel.send("effects.select", {"clip": self.clip, "effect": self.effect})
        super().mousePressEvent(e)

    # ----- engine values -> widgets -----
    def show_values(self, info: dict[str, Any], clip: dict[str, Any]) -> None:
        self.info = info
        self.snapshot = clip
        self.animated = bool(info.get("animated"))
        self.keyframes = [k for k in info.get("keyframes") or [] if isinstance(k, dict)]
        value = info.get("valueAtPlayhead") if self.animated else info.get("value")
        if self.eyedropper is not None:
            self.eyedropper.setChecked(self.panel.eyedropper == self.target())
        self.stopwatch.blockSignals(True)
        self.stopwatch.setChecked(self.animated)
        self.stopwatch.blockSignals(False)
        editing = self.editor.hasFocus() or any(s.hasFocus() for s in self.spins)
        if not editing:
            self._set_editor(value)
        for b in (self.prev_key, self.add_key, self.next_key):
            b.setVisible(self.animated)
        self.graph_twirl.setVisible(self.animated and self.graphable)
        if self.animated:
            here = self._keyframe_here(clip)
            self.add_key.setText("◆" if here is not None else "◇")
            times = self._times()
            ph = as_int(clip.get("playhead"))
            self.prev_key.setEnabled(any(t < ph for t in times))
            self.next_key.setEnabled(any(t > ph for t in times))

    def _set_editor(self, value: Any) -> None:
        w = self.editor
        w.blockSignals(True)
        for s in self.spins:
            s.blockSignals(True)
        try:
            if self.kind in ("float", "angle"):
                self.spins[0].set_value(finite(value))
            elif self.kind == "point":
                xy = value if isinstance(value, list) and len(value) == 2 else [None, None]
                # an automatic coordinate shows where the render puts it (editing the other keeps it)
                auto = self.info.get("auto") if isinstance(self.info.get("auto"), list) else [0.0, 0.0]
                for s, v, a in zip(self.spins, xy, [*auto, 0.0, 0.0][:2], strict=True):
                    s.set_value(finite(v, finite(a)) if v is not None else finite(a))
                    s.setToolTip(tr("automatic") if v is None else "")
            elif isinstance(w, QCheckBox):
                w.setChecked(bool(value))
            elif isinstance(w, QComboBox):
                w.setCurrentIndex(max(0, int(finite(value))))
            elif isinstance(w, QPushButton):
                c = color_to_qcolor(value)
                w.setText(c.name().upper())
                w.setStyleSheet(f"background: {c.name()}; color: {'#000' if c.lightness() > 128 else '#fff'};")
                w.setProperty("cfa4_color", value)
            elif isinstance(w, QLineEdit):
                w.setText("" if value is None else str(value))
            elif isinstance(w, MaskTracking):
                pass
            elif isinstance(w, QLabel):
                w.setText(param_text(self.info, value))
        finally:
            w.blockSignals(False)
            for s in self.spins:
                s.blockSignals(False)

    def _keyframe_here(self, clip: dict[str, Any]) -> dict[str, Any] | None:
        m = clip.get("mediaTimeAtPlayhead")
        return next((k for k in self.keyframes if k.get("mediaTime") == m), None)

    # ----- widgets -> engine -----
    def _toggle_animation(self) -> None:
        self.panel.send("effects.toggleAnimation", self.target())

    def _pick_color(self) -> None:
        current = color_to_qcolor(self.editor.property("cfa4_color"))
        color = QColorDialog.getColor(current, self.editor, tr(str(self.info.get("label") or self.id)),
                                      QColorDialog.ColorDialogOption.ShowAlphaChannel)
        if color.isValid():
            self.panel.set_param(self, qcolor_to_value(color), immediate=True)

    def _add_remove(self) -> None:
        params = self.target()
        monitor = getattr(self.panel.window(), "monitor", None)
        if monitor is not None and getattr(monitor, "playing", False):
            # the snapshot and the engine playhead are still where playback started: stop on the
            # shown frame and toggle the keyframe there
            monitor.stop()
            at = self.panel._edit_time(monitor.playhead, self.snapshot)
            half = max(1, int(monitor.frame()) // 2)
            here = None
            if at is not None:
                here = next((k for k in self.keyframes if isinstance(k.get("time"), int) and abs(k["time"] - at) < half),
                            None)
            add = {**params, "time": at} if at is not None else params
        else:
            here = self._keyframe_here(self.snapshot)
            add = params
        if here is not None:
            self.panel.send("effects.deleteKeyframe", {**params, "mediaTime": here["mediaTime"]})
        else:
            self.panel.send("effects.addKeyframe", add)

    def _times(self) -> list[int]:
        return sorted(int(k["time"]) for k in self.keyframes if as_int(k.get("time"), -1) == k.get("time"))

    def _prev(self) -> None:
        ph = as_int(self.snapshot.get("playhead"))
        earlier = [t for t in self._times() if t < ph]
        if earlier:
            self.panel.seek.emit(earlier[-1])

    def _next(self) -> None:
        ph = as_int(self.snapshot.get("playhead"))
        later = [t for t in self._times() if t > ph]
        if later:
            self.panel.seek.emit(later[0])


MASK_MODES = (N_("None"), N_("Add"), N_("Subtract"), N_("Intersect"), N_("Lighten"), N_("Darken"), N_("Difference"))
TRACK_METHODS = (N_("Position"), N_("Position & Rotation"), N_("Position, Scale & Rotation"))
MASK_GLYPHS = {"ellipse": "◯", "polygon": "▭", "bezier": "✎"}


class MaskTracking(QWidget):
    """The Mask Path row's value: track the mask backward continuously / one frame, forward one
    frame / continuously (a click while a track runs stops it), the tracking method, progress."""

    def __init__(self, row: ParamRow, name: str) -> None:
        super().__init__(row)
        self.row = row
        base = name.rsplit(".", 1)[0]
        h = QHBoxLayout(self)
        h.setContentsMargins(0, 0, 0, 0)
        h.setSpacing(1)
        self.job: int | None = None
        self.buttons: list[QToolButton] = []
        for text, direction, frames, key, tip in (
            ("◀◀", "backward", None, "back", N_("Track selected mask backward")),
            ("◀|", "backward", 1, "backFrame", N_("Track selected mask backward 1 frame")),
            ("|▶", "forward", 1, "fwdFrame", N_("Track selected mask forward 1 frame")),
            ("▶▶", "forward", None, "fwd", N_("Track selected mask forward")),
        ):
            b = QToolButton(self)
            b.setObjectName(f"{base}.track.{key}")
            b.setText(text)
            b.setAutoRaise(True)
            b.setToolTip(tr(tip))
            b.setProperty("cfa4_tip", tip)
            b.clicked.connect(lambda _c=False, d=direction, f=frames: self.track(d, f))
            h.addWidget(b)
            self.buttons.append(b)
        self.method = QToolButton(self)
        self.method.setObjectName(f"{base}.trackMethod")
        self.method.setText("🔧")
        self.method.setAutoRaise(True)
        self.method.setPopupMode(QToolButton.ToolButtonPopupMode.InstantPopup)
        self.method_menu = QMenu(self.method)
        self.method.setMenu(self.method_menu)
        h.addWidget(self.method)
        self.progress = QLabel(self)
        h.addWidget(self.progress)
        h.addStretch(1)

    def set_state(self, mask: dict[str, Any]) -> None:
        job = mask.get("trackJob")
        self.job = job if as_int(job, -1) == job and job >= 0 else None
        for b in self.buttons:
            b.setToolTip(tr("Stop tracking") if self.job is not None else tr(str(b.property("cfa4_tip"))))
        method = str(mask.get("trackMethod") or "")
        self.method.setToolTip(fmt("Tracking method: {method}", method=tr(method)))
        self.method_menu.clear()
        for label in TRACK_METHODS:
            a = self.method_menu.addAction(tr(label))
            a.setCheckable(True)
            a.setChecked(label == method)
            a.triggered.connect(lambda _c=False, m=label: self.row.panel.send(
                "masks.set", {"clip": self.row.clip, "effect": self.row.effect, "mask": self.row.mask, "trackMethod": m}))
        progress = mask.get("trackProgress")
        if self.job is not None and isinstance(progress, list) and len(progress) == 2:
            self.progress.setText(f"{as_int(progress[0])}/{as_int(progress[1])}")
        else:
            self.progress.setText("")

    def track(self, direction: str, frames: int | None) -> None:
        r = self.row
        if self.job is not None:
            r.panel.send("jobs.cancel", {"job": self.job})
            return
        target = {"clip": r.clip, "effect": r.effect, "mask": r.mask}
        params: dict[str, Any] = {**target, "direction": direction}
        if frames is not None:
            params["frames"] = frames
        r.panel.send("masks.select", target)
        r.panel.send("masks.track", params)


class MaskToolsRow(LinedRow):
    """Under a maskable effect: create an ellipse, a 4-point polygon, or a free-drawn Bezier mask."""

    def __init__(self, panel: EffectControlsPanel, parent: QWidget, clip: int, effect: int, name: str) -> None:
        super().__init__(parent)
        self.setFixedHeight(ROW_H)
        self.setObjectName(f"{name}.maskTools")
        h = QHBoxLayout(self)
        h.setContentsMargins(30, 0, 4, 0)
        h.setSpacing(2)
        self.buttons: dict[str, QToolButton] = {}
        for text, kind, tip in (("◯", "ellipse", N_("Create ellipse mask")), ("▭", "polygon", N_("Create 4-point polygon mask")),
                                ("✎", "pen", N_("Free draw bezier"))):
            b = QToolButton(self)
            b.setObjectName(f"{name}.mask.{kind}")
            b.setText(text)
            b.setAutoRaise(True)
            b.setToolTip(tr(tip))
            if kind == "pen":
                b.setCheckable(True)
                b.toggled.connect(lambda on: panel.mask_pen(clip, effect, on))
            else:
                b.clicked.connect(lambda _c=False, k=kind: panel.send("masks.add", {"clip": clip, "effect": effect, "shape": k}))
            h.addWidget(b)
            self.buttons[kind] = b
        h.addStretch(1)


class MaskHeader(LinedRow):
    """A mask's row: twirl, shape glyph, name and mode; a click selects it (again: deselects);
    its context menu deletes it."""

    def __init__(self, panel: EffectControlsPanel, parent: QWidget, clip: int, effect: int, mask: dict[str, Any],
                 name: str) -> None:
        super().__init__(parent)
        self.panel = panel
        self.clip, self.effect, self.mask = clip, effect, as_int(mask.get("mask"))
        self.hoverable = True
        self.setObjectName(name)
        self.setFixedHeight(ROW_H)
        self.setAttribute(Qt.WidgetAttribute.WA_Hover)
        h = QHBoxLayout(self)
        h.setContentsMargins(16, 0, 26, 0)
        h.setSpacing(4)
        self.twirl = QToolButton(self)
        self.twirl.setObjectName(f"{name}.twirl")
        self.twirl.setAutoRaise(True)
        self.twirl.clicked.connect(lambda: panel.toggle_mask(clip, effect, self.mask))
        self.glyph = QLabel(self)
        self.name = QLabel(self)
        self.name.setSizePolicy(QSizePolicy.Policy.Ignored, QSizePolicy.Policy.Preferred)
        self.mode = QComboBox(self)
        self.mode.setObjectName(f"{name}.mode")
        self.mode.setFixedWidth(96)
        for label in MASK_MODES:
            self.mode.addItem(tr(label), label)
        self.mode.activated.connect(lambda i: panel.send("masks.set", {**self.target(), "mode": MASK_MODES[i]}))
        h.addWidget(self.twirl)
        h.addWidget(self.glyph)
        h.addWidget(self.name, 1)
        h.addWidget(self.mode)
        self.selected = False
        self.set_state(mask)

    def target(self) -> dict[str, Any]:
        return {"clip": self.clip, "effect": self.effect, "mask": self.mask}

    def set_open(self, is_open: bool) -> None:
        self.twirl.setText("▾" if is_open else "▸")

    def set_state(self, mask: dict[str, Any]) -> None:
        self.selected = bool(mask.get("selected"))
        self.highlight = self.selected
        self.glyph.setText(MASK_GLYPHS.get(str(mask.get("shape")), "✎"))
        t = theme.current()
        self.glyph.setStyleSheet(f"color: {t.accent if self.selected else t.text_dim};")
        self.name.setText(str(mask.get("name") or ""))
        mode = str(mask.get("mode") or "Add")
        i = MASK_MODES.index(mode) if mode in MASK_MODES else 1
        self.mode.blockSignals(True)
        self.mode.setCurrentIndex(i)
        self.mode.blockSignals(False)
        self.update()

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() == Qt.MouseButton.LeftButton:
            self.panel.send("masks.select", {"none": True} if self.selected else self.target())
        super().mousePressEvent(e)

    def contextMenuEvent(self, e: Any) -> None:
        menu = QMenu(self)
        delete = menu.addAction(tr("Delete Mask"))
        delete.setObjectName("effectControls.menu.deleteMask")
        delete.triggered.connect(lambda: self.panel.send("masks.remove", self.target()))
        self.panel.last_menu = menu
        menu.exec(e.globalPos())


class MaskInvertedRow(LinedRow):
    def __init__(self, panel: EffectControlsPanel, parent: QWidget, header: MaskHeader, name: str) -> None:
        super().__init__(parent)
        self.setFixedHeight(ROW_H)
        h = QHBoxLayout(self)
        h.setContentsMargins(40, 0, 4, 0)
        label = QLabel(tr("Inverted"), self)
        label.setSizePolicy(QSizePolicy.Policy.Ignored, QSizePolicy.Policy.Preferred)
        self.check = QCheckBox(self)
        self.check.setObjectName(f"{name}.inverted")
        self.check.toggled.connect(lambda on: panel.send("masks.set", {**header.target(), "inverted": on}))
        h.addWidget(label, 2)
        h.addWidget(self.check, 3)

    def set_state(self, mask: dict[str, Any]) -> None:
        self.check.blockSignals(True)
        self.check.setChecked(bool(mask.get("inverted")))
        self.check.blockSignals(False)


def components(value: Any) -> list[float | None]:
    """The curves a graphed value draws: a scalar's one, a point's X and Y (None: not finite)."""
    vals = value if isinstance(value, list) else [value]
    out: list[float | None] = []
    for v in vals[:2]:
        f = finite(v, math.nan) if isinstance(v, int | float) and not isinstance(v, bool) else math.nan
        out.append(f if math.isfinite(f) else None)
    return out


def eases(interpolation: Any) -> tuple[bool, bool]:
    """Whether a keyframe's curve eases (has an influence handle) out of it and into it."""
    flat = ("linear", "hold", None)
    return interpolation not in (*flat, "easeIn"), interpolation not in (*flat, "easeOut")


class GraphRow(LinedRow):
    """The value and velocity graphs of an animated scalar or point parameter, under its row:
    labels and the range here, the curves in the lane (keyframes drag vertically for their value,
    Bezier influence handles horizontally). A point draws an X and a Y curve and its velocity is
    the speed along its path."""

    VALUE_H = 110
    VELOCITY_H = 64

    def __init__(self, panel: EffectControlsPanel, parent: QWidget, row: ParamRow) -> None:
        super().__init__(parent)
        self.panel = panel
        self.row = row
        self.setObjectName(f"{row.objectName()}.graph")
        self.setFixedHeight(self.VALUE_H + self.VELOCITY_H)
        self.values: list[list[float | None]] = []
        self.seconds = 0.0
        self.range: tuple[float, float] | None = None
        self.velocity: list[float] = []
        self.vmax = 1e-6

    @property
    def decimals(self) -> int:
        return max(0, min(6, int(finite(self.row.info.get("decimals"), 1)))) if self.row.kind == "float" else 1

    def set_samples(self, values: Any, seconds: Any) -> None:
        rows = [components(v) for v in values] if isinstance(values, list) else []
        self.values = rows
        self.seconds = max(0.0, finite(seconds))
        known = [v for r in rows for v in r if v is not None]
        known += [v for k in self.row.keyframes for v in components(k.get("value")) if v is not None]
        lo, hi = (min(known), max(known)) if known else (0.0, 0.0)
        if hi - lo < 1e-6:
            lo, hi = lo - 1.0, hi + 1.0
        pad = (hi - lo) * 0.12
        lo, hi = lo - pad, hi + pad
        if self.row.kind == "float":
            lo = max(lo, finite(self.row.info.get("min"), lo))
            hi = min(hi, finite(self.row.info.get("max"), hi))
        # while the graph stays open its range only widens, so a key dragged past an edge is not
        # drawn back on that edge
        if self.range is not None:
            lo, hi = min(lo, self.range[0]), max(hi, self.range[1])
        if hi - lo < 1e-9:
            hi = lo + 1.0
        self.range = (lo, hi)
        n = max(1, len(rows) - 1)
        secs = max(1e-9, self.seconds / n)
        vel: list[float] = []
        for a, b in itertools.pairwise(rows):
            d = [(y - x) / secs for x, y in zip(a, b, strict=False) if x is not None and y is not None]
            vel.append(d[0] if len(d) == 1 else math.sqrt(sum(v * v for v in d)) if d else math.nan)
        self.velocity = vel
        self.vmax = max([abs(v) for v in vel if math.isfinite(v)] + [1e-6]) * 1.15
        self.update()
        self.panel.lane.update()

    def paintEvent(self, e: QPaintEvent) -> None:
        super().paintEvent(e)
        p = QPainter(self)
        t = theme.current()
        small = QFont(self.font())
        small.setPointSizeF(max(6.0, small.pointSizeF() * 0.85))
        p.setFont(small)
        vh = self.VALUE_H
        right = QRectF(0, 0, self.width() - 30, 0)
        p.setPen(t.c("text_dim"))
        if self.range is not None:
            lo, hi = self.range
            p.drawText(QRectF(right.x(), 4, right.width(), 14), Qt.AlignmentFlag.AlignRight, f"{hi:.{self.decimals}f}")
            p.drawText(QRectF(right.x(), vh - 18, right.width(), 14), Qt.AlignmentFlag.AlignRight, f"{lo:.{self.decimals}f}")
        p.drawText(QPointF(44, vh / 2 + 4), tr("Value"))
        if len(self.values[0] if self.values else []) > 1:
            p.setPen(t.c("accent"))
            p.drawText(QPointF(44, vh / 2 + 18), "X")
            p.setPen(Y_CURVE)
            p.drawText(QPointF(58, vh / 2 + 18), "Y")
        p.setPen(t.c("text_dim"))
        p.drawText(QPointF(44, vh + self.VELOCITY_H / 2 + 4), tr("Velocity"))
        p.drawText(QRectF(right.x(), vh + 4, right.width(), 14), Qt.AlignmentFlag.AlignRight, f"{self.vmax:.1f}/s")


Y_CURVE = QColor(0x60, 0xC0, 0x80)
VELOCITY_CURVE = QColor(0xD0, 0xA0, 0x40)


class KeyLane(QWidget):
    """The keyframe lane beside the effect list: each animated row's keyframes on its line, the
    playhead, and the selection, marquee and drag gestures."""

    def __init__(self, panel: EffectControlsPanel, parent: QWidget) -> None:
        super().__init__(parent)
        self.panel = panel
        self.setObjectName("effectControls.lane")
        self.setMinimumWidth(MIN_LANE_W)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Expanding)
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)
        self.press: QPointF | None = None
        self.pressed_key: tuple[Any, ...] | None = None
        self.marquee: tuple[QPointF, QPointF, list[tuple[Any, ...]]] | None = None
        # a key drag: {"keys", "anchor", "low", "high", "delta", "snap"}
        self.group: dict[str, Any] | None = None
        self.toggle_on_release = False
        # a graph drag: {"kind": "value"|"handle", "point" | "handle": ..., "offset": px}
        self.graph_drag: dict[str, Any] | None = None

    def resizeEvent(self, e: Any) -> None:
        super().resizeEvent(e)
        self.panel.ruler.update()

    # ----- geometry -----
    def lane_x(self, clip: dict[str, Any] | None, t: int) -> float:
        start, end = span(clip)
        w = max(1.0, self.width() - LANE_PAD_L - LANE_PAD_R)
        return LANE_PAD_L + (t - start) / (end - start) * w

    def ticks_per_px(self, clip: dict[str, Any] | None) -> float:
        start, end = span(clip)
        return (end - start) / max(1.0, self.width() - LANE_PAD_L - LANE_PAD_R)

    def row_y(self, row: QWidget) -> tuple[float, float] | None:
        """A row's top and height in lane coordinates (None while hidden)."""
        if not row.isVisibleTo(self.panel):
            return None
        top = row.mapTo(self.parentWidget(), QPoint(0, 0)).y() - self.y()
        return float(top), float(row.height())

    def points(self) -> list[dict[str, Any]]:
        """Every keyframe shown: its reference, centre, interpolation, timeline time and clip."""
        out: list[dict[str, Any]] = []
        for row in self.panel.param_rows():
            if not row.animated:
                continue
            geo = self.row_y(row)
            if geo is None:
                continue
            y = geo[0] + geo[1] / 2
            for k in row.keyframes:
                t, m = k.get("time"), k.get("mediaTime")
                if as_int(t, -1) != t or as_int(m, -1) != m:
                    continue
                ref = (row.clip, row.effect, row.id, row.mask, m)
                out.append({"ref": ref, "pos": QPointF(self.lane_x(row.snapshot, t), y), "interp": k.get("interpolation"),
                            "time": t, "clip": row.snapshot})
        return out

    # ----- graphs -----
    def graph_areas(self, g: GraphRow) -> tuple[QRectF, QRectF] | None:
        """The value and the velocity plot of an open graph, in lane coordinates."""
        geo = self.row_y(g)
        if geo is None:
            return None
        top = geo[0]
        w = max(1.0, self.width() - LANE_PAD_L - LANE_PAD_R)
        value = QRectF(LANE_PAD_L, top + 4, w, g.VALUE_H - 8)
        velocity = QRectF(LANE_PAD_L, top + g.VALUE_H + 2, w, g.VELOCITY_H - 6)
        return value, velocity

    def graphs(self) -> list[GraphRow]:
        return [r.graph for r in self.panel.param_rows() if r.graph is not None and r.graph.isVisibleTo(self.panel)]

    @staticmethod
    def value_y(area: QRectF, rng: tuple[float, float], v: float) -> float:
        lo, hi = rng
        return area.bottom() - (v - lo) / (hi - lo) * area.height()

    @staticmethod
    def y_value(area: QRectF, rng: tuple[float, float], y: float) -> float:
        lo, hi = rng
        return lo + (area.bottom() - y) / max(1e-9, area.height()) * (hi - lo)

    def graph_points(self) -> list[dict[str, Any]]:
        """Every keyframe value on the open graphs: graph, keyframe, curve, centre."""
        out: list[dict[str, Any]] = []
        for g in self.graphs():
            areas = self.graph_areas(g)
            if areas is None or g.range is None:
                continue
            for i, k in enumerate(g.row.keyframes):
                t = k.get("time")
                if as_int(t, -1) != t:
                    continue
                x = self.lane_x(g.row.snapshot, t)
                for ci, v in enumerate(components(k.get("value"))):
                    if v is not None:
                        out.append({"graph": g, "key": k, "index": i, "comp": ci, "value": v,
                                    "pos": QPointF(x, self.value_y(areas[0], g.range, v))})
        return out

    def graph_handles(self) -> list[dict[str, Any]]:
        """Bezier influence handles: flat tangents whose length is the influence times the
        neighbouring segment (on a point's X curve only, as both coordinates share them)."""
        out: list[dict[str, Any]] = []
        for pt in self.graph_points():
            if pt["comp"] != 0:
                continue
            keys = pt["graph"].row.keyframes
            out_on, in_on = eases(pt["key"].get("interpolation"))
            i = pt["index"]
            for side, on, nb in ((1, out_on, keys[i + 1] if i + 1 < len(keys) else None),
                                 (-1, in_on and i > 0, keys[i - 1] if i > 0 else None)):
                if not on or nb is None or as_int(nb.get("time"), -1) != nb.get("time"):
                    continue
                seg = abs(self.lane_x(pt["graph"].row.snapshot, nb["time"]) - pt["pos"].x())
                infl = finite(pt["key"].get("outInfluence" if side > 0 else "inInfluence"), 1 / 3)
                reach = max(seg * 0.01, min(seg, infl * seg))
                out.append({**pt, "side": side, "seg": seg, "infl": infl,
                            "handle": QPointF(pt["pos"].x() + side * reach, pt["pos"].y())})
        return out

    def _graph_hit(self, pos: QPointF) -> dict[str, Any] | None:
        for h in self.graph_handles():
            if abs(h["handle"].x() - pos.x()) <= 5 and abs(h["handle"].y() - pos.y()) <= 5:
                return {"kind": "handle", "target": h, "offset": 0.0}
        for pt in self.graph_points():
            if abs(pt["pos"].x() - pos.x()) <= 6 and abs(pt["pos"].y() - pos.y()) <= 6:
                return {"kind": "value", "target": pt, "offset": 0.0}
        return None

    def _finish_graph_drag(self, drag: dict[str, Any]) -> None:
        target = drag["target"]
        row: ParamRow = target["graph"].row
        params: dict[str, Any] = {**row.target(), "mediaTime": target["key"].get("mediaTime")}
        if drag["kind"] == "handle":
            seg = max(1.0, target["seg"])
            infl = max(0.01, min(1.0, (target["infl"] * target["seg"] + drag["offset"] * target["side"]) / seg))
            params["outInfluence" if target["side"] > 0 else "inInfluence"] = round(infl, 6)
        else:
            areas = self.graph_areas(target["graph"])
            rng = target["graph"].range
            if areas is None or rng is None:
                return
            value = self.y_value(areas[0], rng, target["pos"].y() + drag["offset"])
            if row.kind == "float":
                value = max(finite(row.info.get("min"), value), min(finite(row.info.get("max"), value), value))
            comps = components(target["key"].get("value"))
            if len(comps) == 1:
                params["value"] = value
            else:
                comps[target["comp"]] = value
                params["value"] = comps
        self.panel.send("effects.setKeyframe", params)

    def _paint_graphs(self, p: QPainter, t: Any) -> None:
        drag = self.graph_drag or {}
        for g in self.graphs():
            areas = self.graph_areas(g)
            if areas is None or g.range is None:
                continue
            area, varea = areas
            p.setPen(Qt.PenStyle.NoPen)
            p.fillRect(area, t.c("tl_track_bg"))
            p.fillRect(varea, t.c("tl_track_bg"))
            grid = t.c("separator")
            p.setPen(QPen(grid, 1))
            for i in (1, 2, 3):
                y = area.top() + area.height() * i / 4
                p.drawLine(QPointF(area.left(), y), QPointF(area.right(), y))
            n = max(1, len(g.values) - 1)
            for c, color in ((0, t.c("accent")), (1, Y_CURVE)):
                line = QPolygonF()
                for i, vals in enumerate(g.values):
                    v = vals[c] if c < len(vals) else None
                    if v is not None:
                        line.append(QPointF(area.left() + area.width() * i / n, self.value_y(area, g.range, v)))
                if line.size() > 1:
                    p.setPen(QPen(color, 1.5))
                    p.drawPolyline(line)
            p.setPen(QPen(t.c("text_faint"), 1))
            p.drawLine(QPointF(varea.left(), varea.center().y()), QPointF(varea.right(), varea.center().y()))
            vel = QPolygonF()
            for i, v in enumerate(g.velocity):
                if math.isfinite(v):
                    vel.append(QPointF(varea.left() + varea.width() * (i + 0.5) / n,
                                       varea.center().y() - v / g.vmax * varea.height() / 2))
            if vel.size() > 1:
                p.setPen(QPen(VELOCITY_CURVE, 1.2))
                p.drawPolyline(vel)
        for h in self.graph_handles():
            moving = drag.get("kind") == "handle" and drag["target"]["handle"] == h["handle"]
            end = QPointF(h["handle"])
            if moving:
                reach = max(h["seg"] * 0.01, min(h["seg"], h["infl"] * h["seg"] + drag["offset"] * h["side"]))
                end.setX(h["pos"].x() + h["side"] * reach)
            p.setPen(QPen(t.c("text_dim"), 1))
            p.drawLine(h["pos"], end)
            p.setPen(Qt.PenStyle.NoPen)
            p.setBrush(t.c("hot_text"))
            p.drawEllipse(end, 3.5, 3.5)
        for pt in self.graph_points():
            c = QPointF(pt["pos"])
            moving = drag.get("kind") == "value" and drag["target"]["pos"] == pt["pos"] and drag["target"]["comp"] == pt["comp"]
            if moving:
                c.setY(c.y() + drag["offset"])
            p.setPen(Qt.PenStyle.NoPen)
            p.setBrush(t.c("hot_text") if moving else t.c("text_dim"))
            p.drawEllipse(c, 4.5, 4.5)
            if moving:
                areas = self.graph_areas(pt["graph"])
                if areas is not None and pt["graph"].range is not None:
                    p.setPen(t.c("hot_text"))
                    value = self.y_value(areas[0], pt["graph"].range, c.y())
                    p.drawText(c + QPointF(8, -10), f"{value:.{pt['graph'].decimals}f}")

    def key_at(self, pos: QPointF) -> dict[str, Any] | None:
        best, dist = None, KEY_HIT
        for pt in self.points():
            d = abs(pt["pos"].x() - pos.x())
            if d <= dist and abs(pt["pos"].y() - pos.y()) <= ROW_H / 2:
                best, dist = pt, d
        return best

    # ----- painting -----
    def paintEvent(self, _e: QPaintEvent) -> None:
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        t = theme.current()
        p.fillRect(self.rect(), t.c("tl_bg"))
        sep = t.c("separator")
        sep.setAlphaF(0.55)
        p.setPen(sep)
        for row in self.panel.lined_rows():
            geo = self.row_y(row)
            if geo is not None:
                y = geo[0] + geo[1] - 1
                p.drawLine(QPointF(0, y), QPointF(self.width(), y))
        selection = set(self.panel.keyframe_selection)
        shift = self._group_shift()
        for pt in self.points():
            c = QPointF(pt["pos"])
            if pt["ref"] in shift:
                c.setX(c.x() + shift[pt["ref"]])
            color = t.c("hot_text") if pt["ref"] in selection else UNSELECTED_KEY
            paint_key(p, c, pt["interp"], color)
        self._paint_graphs(p, t)
        clip = self.panel.clip
        if clip is not None:
            start, end = span(clip)
            ph = as_int(clip.get("playhead"), start - 1)
            if start <= ph <= end:
                p.setPen(QPen(t.c("playhead"), 1))
                x = self.lane_x(clip, ph)
                p.drawLine(QPointF(x, 0), QPointF(x, self.height()))
        if self.group is not None and self.group.get("snap") is not None:
            p.setPen(QPen(t.c("accent"), 1))
            x = self.group["snap"]
            p.drawLine(QPointF(x, 0), QPointF(x, self.height()))
        if self.marquee is not None:
            a, b, _base = self.marquee
            r = QRectF(a, b).normalized()
            fill = t.c("accent")
            fill.setAlphaF(0.15)
            p.fillRect(r, fill)
            p.setPen(QPen(t.c("accent"), 1))
            p.setBrush(Qt.BrushStyle.NoBrush)
            p.drawRect(r)

    def _group_shift(self) -> dict[tuple[Any, ...], float]:
        """How far (pixels) each dragged keyframe is drawn from where it is."""
        if self.group is None or not self.group["delta"]:
            return {}
        out = {}
        for pt in self.points():
            if pt["ref"] in self.group["keys"]:
                out[pt["ref"]] = self.group["delta"] / self.ticks_per_px(pt["clip"])
        return out

    # ----- gestures -----
    def mousePressEvent(self, e: QMouseEvent) -> None:
        pos = e.position()
        key = self.key_at(pos)
        shift = bool(e.modifiers() & Qt.KeyboardModifier.ShiftModifier)
        self.setFocus(Qt.FocusReason.MouseFocusReason)
        if e.button() == Qt.MouseButton.RightButton:
            if key is not None:
                self.key_menu(key, e.globalPosition().toPoint())
            return
        if e.button() != Qt.MouseButton.LeftButton:
            return
        hit = self._graph_hit(pos)
        if hit is not None:
            self.graph_drag = hit
            self.press = pos
            self.pressed_key = None
            return
        self.press = pos
        self.pressed_key = key["ref"] if key else None
        self.toggle_on_release = False
        sel = self.panel.keyframe_selection
        if key is None:
            return
        if key["ref"] not in sel:
            self.panel.set_keyframe_selection([*sel, key["ref"]] if shift else [key["ref"]])
        elif shift:
            self.toggle_on_release = True

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        if self.press is None:
            return
        pos = e.position()
        if self.graph_drag is not None:
            delta = pos - self.press
            self.graph_drag["offset"] = delta.x() if self.graph_drag["kind"] == "handle" else delta.y()
            self.update()
            return
        shift = bool(e.modifiers() & Qt.KeyboardModifier.ShiftModifier)
        moved = (pos - self.press).manhattanLength() >= QApplication.startDragDistance()
        if self.pressed_key is None:
            if self.marquee is None and moved:
                base = list(self.panel.keyframe_selection) if shift else []
                self.marquee = (self.press, pos, base)
            if self.marquee is not None:
                self.marquee = (self.marquee[0], pos, self.marquee[2])
                self._marquee_select()
                self.update()
            return
        if self.group is None and moved:
            self._begin_group()
        if self.group is not None:
            self._update_group(pos, shift)
            self.update()

    def mouseReleaseEvent(self, e: QMouseEvent) -> None:
        if e.button() != Qt.MouseButton.LeftButton or self.press is None:
            return
        pressed, self.press = self.pressed_key, None
        if self.graph_drag is not None:
            drag, self.graph_drag = self.graph_drag, None
            if drag["offset"]:
                self._finish_graph_drag(drag)
            self.update()
            return
        if self.marquee is not None:
            self.marquee = None
            self.update()
            return
        if self.group is not None:
            group, self.group = self.group, None
            if group["delta"]:
                self.panel.move_keyframes(group["keys"], group["delta"])
            self.update()
            return
        shift = bool(e.modifiers() & Qt.KeyboardModifier.ShiftModifier)
        if pressed is None:
            if not shift:
                self.panel.set_keyframe_selection([])
            return
        if self.toggle_on_release:
            self.panel.set_keyframe_selection([k for k in self.panel.keyframe_selection if k != pressed])
        elif not shift:
            self.panel.set_keyframe_selection([pressed])
        pt = next((p for p in self.points() if p["ref"] == pressed), None)
        if pt is not None:
            self.panel.seek.emit(pt["time"])

    def keyPressEvent(self, e: QKeyEvent) -> None:
        if e.key() == Qt.Key.Key_Escape and (self.group is not None or self.marquee is not None
                                              or self.graph_drag is not None):
            self.cancel()
            return
        super().keyPressEvent(e)

    def cancel(self) -> None:
        """Esc: the drag or marquee ends where it started."""
        if self.marquee is not None:
            self.panel.set_keyframe_selection(self.marquee[2])
        self.group = None
        self.marquee = None
        self.graph_drag = None
        self.press = None
        self.update()

    def _marquee_select(self) -> None:
        if self.marquee is None:
            return
        a, b, base = self.marquee
        r = QRectF(a, b).normalized()
        chosen = list(base)
        for pt in self.points():
            if r.contains(pt["pos"]) and pt["ref"] not in chosen:
                chosen.append(pt["ref"])
        self.panel.set_keyframe_selection(chosen)

    def _begin_group(self) -> None:
        keys = list(self.panel.keyframe_selection)
        pts = {pt["ref"]: pt for pt in self.points()}
        anchor = pts.get(self.pressed_key) if self.pressed_key is not None else None
        if anchor is None:
            return
        # the selection moves by one timeline delta that keeps every key inside its clip
        low, high = -(1 << 62), 1 << 62
        for ref in keys:
            pt = pts.get(ref)
            if pt is None:
                continue
            start, end = span(pt["clip"])
            last = end - frame_of(pt["clip"])
            low = max(low, start - pt["time"])
            high = min(high, max(start, last) - pt["time"])
        if low > high:
            return
        self.group = {"keys": keys, "anchor": anchor, "low": low, "high": high, "delta": 0, "snap": None}

    def _update_group(self, pos: QPointF, shift: bool) -> None:
        g = self.group
        if g is None or self.press is None:
            return
        anchor = g["anchor"]
        offset = (pos.x() - self.press.x()) * self.ticks_per_px(anchor["clip"])
        if not math.isfinite(offset):
            return
        frame = frame_of(anchor["clip"])
        requested = snap_frame(anchor["time"] + round(offset), frame) - anchor["time"]
        g["delta"] = max(g["low"], min(g["high"], requested))
        g["snap"] = None
        if not shift:
            return
        pts = self.points()
        moving = [pt for pt in pts if pt["ref"] in g["keys"]]
        targets = [as_int((self.panel.clip or {}).get("playhead"))] + [pt["time"] for pt in pts if pt["ref"] not in g["keys"]]
        closest = SNAP_PX
        for pt in moving:
            per_px = self.ticks_per_px(pt["clip"])
            for target in targets:
                delta = target - pt["time"]
                distance = abs((delta - requested) / per_px)
                if distance <= closest and g["low"] <= delta <= g["high"]:
                    closest = distance
                    g["delta"] = delta
                    g["snap"] = pt["pos"].x() + delta / per_px

    def key_menu(self, key: dict[str, Any], at: QPoint) -> None:
        clip, effect, param, mask, media = key["ref"]
        params: dict[str, Any] = {"clip": clip, "effect": effect, "param": param, "mediaTime": media}
        if mask is not None:
            params["mask"] = mask
        menu = QMenu(self)
        title = menu.addAction(tr("Temporal Interpolation"))
        title.setEnabled(False)
        for k, label in INTERPOLATIONS:
            a = menu.addAction(tr(label))
            a.setObjectName(f"effectControls.keyframeMenu.{k}")
            a.setCheckable(True)
            a.setChecked(key["interp"] == k)
            a.triggered.connect(lambda _c=False, kk=k: self.panel.send("effects.setInterpolation",
                                                                       {**params, "interpolation": kk}))
        menu.addSeparator()
        clear = menu.addAction(tr("Clear"))
        clear.setObjectName("effectControls.keyframeMenu.clear")
        clear.triggered.connect(lambda: self.panel.send("effects.deleteKeyframe", params))
        self.panel.last_menu = menu
        menu.exec(at)


class Ruler(QWidget):
    """The lane's header: sequence timecode over the clip's stretch of the timeline, the clip's
    bar under it, and the playhead's handle; click or drag to move the playhead."""

    def __init__(self, panel: EffectControlsPanel, parent: QWidget) -> None:
        super().__init__(parent)
        self.panel = panel
        self.setObjectName("effectControls.ruler")
        self.setFixedHeight(RULER_H + 4 + CLIP_BAR_H)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Fixed)

    def lane_range(self) -> tuple[float, float]:
        """Where the lane's time axis is, in this widget's x."""
        lane = self.panel.lane
        left = self.mapFromGlobal(lane.mapToGlobal(QPoint(0, 0))).x() + LANE_PAD_L
        return float(left), float(max(1, lane.width() - LANE_PAD_L - LANE_PAD_R))

    def x_of(self, t: int) -> float:
        start, end = span(self.panel.clip)
        left, w = self.lane_range()
        return left + (t - start) / (end - start) * w

    def paintEvent(self, _e: QPaintEvent) -> None:
        clip = self.panel.clip
        if clip is None:
            return
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        t = theme.current()
        left, w = self.lane_range()
        p.fillRect(QRectF(left - LANE_PAD_L, 0, w + LANE_PAD_L + LANE_PAD_R, self.height()), t.c("tl_bg"))
        p.setClipRect(QRectF(left - LANE_PAD_L, 0, w + LANE_PAD_L + LANE_PAD_R, self.height()))
        start, end = span(clip)
        frame = frame_of(clip)
        drop = bool(clip.get("dropFrame"))
        frame_px = w * frame / (end - start)
        steps = [s for s in ruler_steps(round(TICKS_PER_SECOND / frame)) if s > 0]
        label_step = next((s for s in steps if s * frame_px >= 80), None)
        minor = next((s for s in steps if s * frame_px >= 8 and (label_step is None or label_step % s == 0)), None)
        f0, f1 = start // frame, end // frame
        small = QFont(self.font())
        small.setPointSizeF(max(6.0, small.pointSizeF() * 0.8))
        p.setFont(small)
        base = RULER_H - 1
        for step, h, labelled in ((minor, 3, False), (label_step, 7, True)):
            if step is None:
                continue
            f = f0 // step * step
            for _ in range(4096):  # a step is at least 8 px wide; the cap is for damaged numbers
                if f > f1:
                    break
                x = self.x_of(f * frame)
                p.setPen(t.c("tl_ruler_tick"))
                p.drawLine(QPointF(x, base - h), QPointF(x, base))
                if labelled:
                    p.setPen(t.c("tl_ruler_text"))
                    text = format_timecode(f * frame, frame, drop)
                    tw = p.fontMetrics().horizontalAdvance(text)
                    p.drawText(QPointF(x - tw / 2, 10), text)
                f += step
        bar = QRectF(left, RULER_H + 4 + 2, w, CLIP_BAR_H - 4)
        p.setPen(Qt.PenStyle.NoPen)
        p.setBrush(t.c("clip_bar_bg"))
        p.drawRoundedRect(bar, 2, 2)
        p.setPen(t.c("text"))
        p.drawText(bar.adjusted(4, 0, -4, 0), Qt.AlignmentFlag.AlignVCenter | Qt.AlignmentFlag.AlignLeft,
                   str(clip.get("name") or ""))
        ph = as_int(clip.get("playhead"), start - 1)
        if start <= ph <= end:
            x = self.x_of(ph)
            top, tip = RULER_H - 13, RULER_H
            p.setPen(Qt.PenStyle.NoPen)
            p.setBrush(t.c("playhead"))
            p.drawPolygon(QPolygonF([QPointF(x - 5, top), QPointF(x + 5, top), QPointF(x + 5, tip - 5), QPointF(x, tip),
                                     QPointF(x - 5, tip - 5)]))
            p.setPen(QPen(t.c("playhead"), 1))
            p.drawLine(QPointF(x, tip), QPointF(x, self.height()))

    def _seek(self, x: float) -> None:
        clip = self.panel.clip
        if clip is None:
            return
        start, end = span(clip)
        left, w = self.lane_range()
        f = max(0.0, min(1.0, (x - left) / w))
        t = snap_frame(start + round(f * (end - start)), frame_of(clip))
        self.panel.seek.emit(max(start, min(t, end)))

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() == Qt.MouseButton.LeftButton:
            self._seek(e.position().x())

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        if e.buttons() & Qt.MouseButton.LeftButton:
            self._seek(e.position().x())


class Divider(QWidget):
    """The line between the effect list and the lane; dragging it sets the list's width."""

    def __init__(self, panel: EffectControlsPanel, parent: QWidget) -> None:
        super().__init__(parent)
        self.panel = panel
        self.setObjectName("effectControls.divider")
        self.setFixedWidth(DIVIDER_W)
        self.setCursor(Qt.CursorShape.SplitHCursor)
        self.setAttribute(Qt.WidgetAttribute.WA_Hover)
        self._grab: float | None = None

    def paintEvent(self, _e: QPaintEvent) -> None:
        p = QPainter(self)
        t = theme.current()
        p.setPen(t.c("accent") if self.underMouse() or self._grab is not None else t.c("separator"))
        p.drawLine(DIVIDER_W // 2, 0, DIVIDER_W // 2, self.height())

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() == Qt.MouseButton.LeftButton:
            self._grab = e.globalPosition().x() - self.panel.split

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        if self._grab is not None:
            self.panel.set_split(int(e.globalPosition().x() - self._grab), dragged=True)

    def mouseReleaseEvent(self, _e: QMouseEvent) -> None:
        self._grab = None
        self.update()


class SavePresetDialog(QDialog):
    """egui's Save Preset: name, how keyframes are kept (Scale, Anchor to In / Out Point, Without
    keyframes) and a description."""

    KEYFRAMES = (("scale", N_("Scale")), ("anchorIn", N_("Anchor to In Point")), ("anchorOut", N_("Anchor to Out Point")),
                 ("none", N_("Without keyframes")))

    def __init__(self, parent: QWidget | None, name: str) -> None:
        super().__init__(parent)
        self.setObjectName("savePreset")
        self.setWindowTitle(tr("Save Preset"))
        v = QVBoxLayout(self)
        row = QHBoxLayout()
        row.addWidget(QLabel(tr("Name:"), self))
        self.name = QLineEdit(name, self)
        self.name.setObjectName("savePreset.name")
        row.addWidget(self.name, 1)
        v.addLayout(row)
        head = QLabel(tr("Type"), self)
        font = head.font()
        font.setBold(True)
        head.setFont(font)
        v.addWidget(head)
        self.group = QButtonGroup(self)
        for i, (key, label) in enumerate(self.KEYFRAMES):
            r = QRadioButton(tr(label), self)
            r.setObjectName(f"savePreset.keyframes.{key}")
            r.setChecked(i == 0)
            self.group.addButton(r, i)
            v.addWidget(r)
        v.addWidget(QLabel(tr("Description:"), self))
        self.description = QPlainTextEdit(self)
        self.description.setObjectName("savePreset.description")
        self.description.setFixedHeight(70)
        v.addWidget(self.description)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel, self)
        self.ok = buttons.button(QDialogButtonBox.StandardButton.Ok)
        self.ok.setObjectName("savePreset.ok")
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        v.addWidget(buttons)
        self.name.textChanged.connect(lambda s: self.ok.setEnabled(bool(s.strip())))
        self.ok.setEnabled(bool(name.strip()))

    def values(self) -> dict[str, Any]:
        key = self.KEYFRAMES[max(0, self.group.checkedId())][0]
        return {"name": self.name.text().strip(), "description": self.description.toPlainText(), "keyframes": key}


class EffectControlsPanel(QWidget):
    command = Signal(str, dict)
    seek = Signal(object)  # ticks; object, not a 32-bit int
    eyedropper_armed = Signal(object)  # the colour parameter (effects.* target) or None
    fx_editor = Signal(dict)  # open the Clip Fx Editor of {"clip", "effect"}

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.EffectControls")
        self.engine: Any = None
        self.clip: dict[str, Any] | None = None
        self.clip_id: int | None = None
        self.clips: list[dict[str, Any]] = []
        self._shape: tuple[Any, ...] | None = None
        # the shown clip's rows by (effect index, param id); the linked audio's in `linked_rows`
        self.rows: dict[tuple[int, str], ParamRow] = {}
        self.linked_rows: dict[tuple[int, str], ParamRow] = {}
        self.headers: dict[tuple[int, int], EffectHeader] = {}
        self.effect_rows: dict[tuple[int, int], list[ParamRow]] = {}
        self.custom_rows: dict[tuple[int, int], CustomSetupRow] = {}
        self.collapsed: set[tuple[int, int]] = set()
        self.open_graphs: set[tuple[int, int, str]] = set()
        self.collapsed_masks: set[tuple[int, int, int]] = set()
        # (clip, effect, mask) -> (header, its rows, inverted row)
        self.masks: dict[tuple[int, int, int], tuple[MaskHeader, list[ParamRow], MaskInvertedRow]] = {}
        self.mask_tools: dict[tuple[int, int], MaskToolsRow] = {}
        self.keyframe_selection: list[tuple[Any, ...]] = []
        self.split = 0  # the list's width
        self.user_split = 0  # the width the divider was dragged to (0: egui's default)
        self.last_menu: QMenu | None = None
        self.eyedropper: dict[str, Any] | None = None  # the colour parameter an armed eyedropper fills
        # (clip, effect index, param id, timeline time or None) -> value: an edit stays bound to the
        # clip and, when animated, the time it was made at, whatever the panel shows by the flush
        self._pending: dict[tuple[int, int, str, int | None, int | None], Any] = {}
        self._send_timer = QTimer(self, singleShot=True, interval=SEND_DELAY_MS)
        self._send_timer.timeout.connect(self.flush)
        self._last_send = QElapsedTimer()
        self._busy = False
        self._again = False

        self.title = QLabel(tr("Select a clip in the Timeline."), self)
        self.title.setObjectName("effectControls.title")
        self.title.setAlignment(Qt.AlignmentFlag.AlignCenter)
        # header: the two pill tabs over the list, the ruler over the lane
        self.header = QWidget(self)
        hh = QHBoxLayout(self.header)
        hh.setContentsMargins(0, 0, 0, 0)
        hh.setSpacing(0)
        self.pills = QWidget(self.header)
        ph = QHBoxLayout(self.pills)
        ph.setContentsMargins(8, 4, 4, 4)
        ph.setSpacing(6)
        self.source_tab = QLabel(self.pills)
        self.source_tab.setObjectName("effectControls.sourceTab")
        self.sequence_tab = QLabel(self.pills)
        self.sequence_tab.setObjectName("effectControls.sequenceTab")
        for pill in (self.source_tab, self.sequence_tab):
            pill.setSizePolicy(QSizePolicy.Policy.Ignored, QSizePolicy.Policy.Fixed)
            pill.setFixedHeight(24)
            ph.addWidget(pill, 1)
        self.ruler = Ruler(self, self.header)
        hh.addWidget(self.pills)
        hh.addSpacing(DIVIDER_W)
        hh.addWidget(self.ruler, 1)
        # body: the list and the lane scroll together; the divider between them
        self.body = QWidget(self)
        content = QHBoxLayout(self.body)
        content.setContentsMargins(0, 0, 0, 0)
        content.setSpacing(0)
        self.column = QWidget(self.body)
        self.body_layout = QVBoxLayout(self.column)
        self.body_layout.setContentsMargins(0, 0, 0, 0)
        self.body_layout.setSpacing(0)
        self.body_layout.addStretch(1)
        self.divider = Divider(self, self.body)
        self.lane = KeyLane(self, self.body)
        content.addWidget(self.column)
        content.addWidget(self.divider)
        content.addWidget(self.lane, 1)
        self.scroll = QScrollArea(self)
        self.scroll.setWidgetResizable(True)
        self.scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.scroll.setFrameShape(QFrame.Shape.NoFrame)
        self.scroll.setWidget(self.body)
        self.scroll.verticalScrollBar().valueChanged.connect(lambda _v: self.ruler.update())
        self.timecode = QLabel(self)
        self.timecode.setObjectName("effectControls.timecode")
        self.timecode.setContentsMargins(10, 2, 0, 2)
        mono = QFont("monospace")
        mono.setStyleHint(QFont.StyleHint.Monospace)
        mono.setPointSizeF(mono.pointSizeF() * 1.1 if mono.pointSizeF() > 0 else 11)
        self.timecode.setFont(mono)
        self.timecode.setStyleSheet(f"color: {theme.current().timecode};")
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(0)
        # a transition selected in the Timeline (no clip): its own view (egui's transition_controls.rs)
        from .transition_controls import TransitionControls

        self.transition_view = TransitionControls(self)
        self.transition_view.hide()
        layout.addWidget(self.title)
        layout.addWidget(self.header)
        layout.addWidget(self.scroll, 1)
        layout.addWidget(self.transition_view, 1)
        layout.addWidget(self.timecode)
        self._set_visible(False)

    def _set_visible(self, has_clip: bool) -> None:
        self.title.setVisible(not has_clip)
        for w in (self.header, self.scroll, self.timecode):
            w.setVisible(has_clip)

    # ----- layout -----
    def resizeEvent(self, e: Any) -> None:
        super().resizeEvent(e)
        self.set_split(self.user_split)

    def set_split(self, width: int, dragged: bool = False) -> None:
        """The list's width (egui's default: 58 % of the panel, at least 260); each side keeps
        its minimum."""
        if dragged:
            self.user_split = width
        if width <= 0:
            width = max(260, int(self.width() * 0.58))
        hi = max(MIN_LIST_W, self.width() - MIN_LANE_W - DIVIDER_W - 10)
        width = max(MIN_LIST_W, min(width, hi))
        self.split = width
        self.column.setFixedWidth(width)
        self.pills.setFixedWidth(width)
        self.transition_view.set_split(width)
        self.lane.update()
        self.ruler.update()

    def arm_eyedropper(self, target: dict[str, Any] | None) -> None:
        """Arm (or with None disarm) the eyedropper of a colour parameter: the Program monitor's
        next click fills it (``effects.eyedropper``)."""
        self.eyedropper = target
        for row in self.param_rows():
            if row.eyedropper is not None:
                row.eyedropper.setChecked(target is not None and row.target() == target)
        self.eyedropper_armed.emit(target)

    def param_rows(self) -> list[ParamRow]:
        return [*self.rows.values(), *self.linked_rows.values()]

    def lined_rows(self) -> list[QWidget]:
        return [w for w in self.column.findChildren(LinedRow)]

    def row(self, effect: int, param: str) -> ParamRow:
        return self.rows[(effect, param)]

    # ----- loading -----
    def attach(self, engine: Any) -> None:
        self.engine = engine

    def refresh(self, selection: list[Any] | None, transitions: list[Any] | None = None) -> None:
        """Ask the engine about the selected clip (a video one first, as egui) and the audio
        linked to it; with no clip selected, about the selected transition (or clear the panel)."""
        clips = [c for c in selection or [] if isinstance(c, int)]
        transitions = [t for t in transitions or [] if isinstance(t, int)]
        if not (clips or transitions) or self.engine is None or not self.engine.running:
            self._show(None)
            return
        if self._busy:
            self._again = True
            return
        self._busy = True
        if clips:
            self.engine.execute("effects.inspect", {}, on_reply=self._on_reply, on_error=lambda _m: self._on_reply(None, None))
        else:
            self.engine.execute("transitions.inspect", {}, on_reply=self._on_transition,
                                on_error=lambda _m: self._on_transition(None, None))

    def _on_transition(self, result: Any, _b: bytes | None) -> None:
        self._done(None, None, result if isinstance(result, dict) else None)

    def _on_reply(self, result: Any, _b: bytes | None) -> None:
        clip = result if isinstance(result, dict) else None
        audio = clip.get("linkedAudio") if clip else None
        if as_int(audio, -1) >= 0 and self.engine is not None and self.engine.running:
            self.engine.execute(
                "effects.inspect", {"clip": audio},
                on_reply=lambda r, _b2: self._done(clip, r if isinstance(r, dict) else None),
                on_error=lambda _m: self._done(clip, None),
            )
            return
        self._done(clip, None)

    def _done(self, clip: dict[str, Any] | None, audio: dict[str, Any] | None,
              transition: dict[str, Any] | None = None) -> None:
        self._busy = False
        self._show(clip, audio)
        if clip is None and transition is not None:
            self.title.hide()
            self.transition_view.show_transition(transition)
            self.transition_view.show()
        if self._again:
            self._again = False
            seq = getattr(self.window(), "sequence", None)
            seq = seq if isinstance(seq, dict) else {}
            self.refresh(seq.get("selection"), seq.get("transitionSelection"))

    def _show(self, clip: dict[str, Any] | None, audio: dict[str, Any] | None = None) -> None:
        self.flush()  # edits made on the clip shown so far go to that clip, at their own time
        self.clip = clip
        self.clip_id = as_int(clip.get("clip"), -1) if clip else None
        if self.clip_id is not None and self.clip_id < 0:
            self.clip_id, self.clip = None, None
        self.clips = [c for c in (self.clip, audio) if c is not None and as_int(c.get("clip"), -1) >= 0]
        self.transition_view.hide()
        self._set_visible(self.clip is not None)
        if self.clip is None:
            self.title.setText(tr("Select a clip in the Timeline."))
            self._rebuild([])
            self.keyframe_selection = []
            return
        name = str(self.clip.get("name") or "")
        self.source_tab.setText(fmt("Source · {name}", name=name))
        self.sequence_tab.setText(f"{self.clip.get('sequenceName') or ''} · {name}")
        t = theme.current()
        self.source_tab.setStyleSheet(f"border: 1px solid {t.separator}; border-radius: 4px; padding-left: 8px;"
                                      f" color: {t.text_dim};")
        self.sequence_tab.setStyleSheet(f"background: {t.row_selected}; border-radius: 4px; padding-left: 8px;"
                                        f" color: {t.text};")
        self.timecode.setText(format_timecode(as_int(self.clip.get("playhead")), frame_of(self.clip),
                                              bool(self.clip.get("dropFrame"))))
        shape = tuple(
            (as_int(c.get("clip")), tuple((e.get("effect"), bool(e.get("intrinsic")), bool(e.get("maskable")),
                                           len(e.get("maskList") or []),
                                           tuple(p.get("id") for p in e.get("params") or [] if isinstance(p, dict)))
                                          for e in c.get("effects") or [] if isinstance(e, dict)))
            for c in self.clips
        )
        if shape != self._shape:
            self._rebuild(self.clips)
            self._shape = shape
        for c in self.clips:
            cid = as_int(c.get("clip"))
            rows = self.rows if c is self.clip else self.linked_rows
            for e in c.get("effects") or []:
                if not isinstance(e, dict):
                    continue
                index = as_int(e.get("index"))
                header = self.headers.get((cid, index))
                if header is not None:
                    header.set_state(bool(e.get("enabled", True)), bool(e.get("selected")))
                for info in e.get("params") or []:
                    if isinstance(info, dict):
                        row = rows.get((index, str(info.get("id"))))
                        if row is not None:
                            row.show_values(info, c)
                            self._apply_graph(row)
                for m in e.get("maskList") or []:
                    if not isinstance(m, dict):
                        continue
                    k = as_int(m.get("mask"))
                    entry = self.masks.get((cid, index, k))
                    if entry is None:
                        continue
                    mh, mrows, inverted = entry
                    mh.set_state(m)
                    inverted.set_state(m)
                    infos = {str(i.get("id")): i for i in m.get("params") or [] if isinstance(i, dict)}
                    for row in mrows:
                        if row.id in infos:
                            row.show_values(infos[row.id], c)
                            self._apply_graph(row)
                        if isinstance(row.editor, MaskTracking):
                            row.editor.set_state(m)
        # keep only selected keyframes that still exist
        shown = {pt["ref"] for pt in self.lane.points()} if self.lane.isVisible() else None
        if shown is not None:
            self.keyframe_selection = [k for k in self.keyframe_selection if k in shown]
        self.lane.update()
        self.ruler.update()

    def _rebuild(self, clips: list[dict[str, Any]]) -> None:
        while self.body_layout.count() > 1:
            item = self.body_layout.takeAt(0)
            w = item.widget() if item is not None else None
            if w is not None:
                w.setParent(None)
                w.deleteLater()
        self.rows, self.linked_rows, self.headers, self.effect_rows = {}, {}, {}, {}
        self.custom_rows = {}
        self.masks, self.mask_tools = {}, {}
        self._shape = None
        for n, c in enumerate(clips):
            cid = as_int(c.get("clip"))
            prefix = "effectControls" if n == 0 else "effectControls.audio"
            rows = self.rows if n == 0 else self.linked_rows
            heading = QLabel(tr("Audio") if c.get("kind") == "audio" else tr("Video"), self.column)
            heading.setObjectName(f"{prefix}.heading")
            heading.setFixedHeight(HEAD_H)
            heading.setContentsMargins(8, 0, 0, 0)
            font = heading.font()
            font.setBold(True)
            heading.setFont(font)
            heading.setStyleSheet(f"color: {theme.current().text_dim};")
            self._add(heading)
            effects = [e for e in c.get("effects") or [] if isinstance(e, dict)]
            # the fixed effects first, as Premiere lists them
            effects.sort(key=lambda e: not e.get("intrinsic"))
            after_fixed = False
            for e in effects:
                index = as_int(e.get("index"))
                first_added = False
                if e.get("intrinsic"):
                    after_fixed = True
                else:
                    first_added, after_fixed = after_fixed, False
                header = EffectHeader(self, self.column, cid, e, f"{prefix}.{index}", first_added)
                self.headers[(cid, index)] = header
                self._add(header)
                if str(e.get("effect") or "") in HAS_EDITOR:
                    custom = CustomSetupRow(self, self.column, cid, index, str(e.get("effect")))
                    self.custom_rows[(cid, index)] = custom
                    self._add(custom)
                params: list[ParamRow] = []
                for info in e.get("params") or []:
                    if isinstance(info, dict):
                        row = ParamRow(self, self.column, cid, index, info, f"{prefix}.{index}.{info.get('id')}")
                        rows[(index, str(info.get("id")))] = row
                        params.append(row)
                        self._add(row)
                        if row.graphable:
                            row.graph = GraphRow(self, self.column, row)
                            row.graph.hide()
                            self._add(row.graph)
                if e.get("maskable"):
                    tools = MaskToolsRow(self, self.column, cid, index, f"{prefix}.{index}")
                    self.mask_tools[(cid, index)] = tools
                    self._add(tools)
                    for m in e.get("maskList") or []:
                        if not isinstance(m, dict):
                            continue
                        k = as_int(m.get("mask"))
                        base = f"{prefix}.{index}.mask{k}"
                        mh = MaskHeader(self, self.column, cid, index, m, base)
                        self._add(mh)
                        mrows: list[ParamRow] = []
                        for info in m.get("params") or []:
                            if not isinstance(info, dict):
                                continue
                            row = ParamRow(self, self.column, cid, index, info, f"{base}.{info.get('id')}", mask=k)
                            rows[(index, f"mask{k}.{info.get('id')}")] = row
                            mrows.append(row)
                            self._add(row)
                            if row.graphable:
                                row.graph = GraphRow(self, self.column, row)
                                row.graph.hide()
                                self._add(row.graph)
                        inverted = MaskInvertedRow(self, self.column, mh, base)
                        self._add(inverted)
                        self.masks[(cid, index, k)] = (mh, mrows, inverted)
                self.effect_rows[(cid, index)] = params
                self._apply_collapsed(cid, index)

    def _add(self, w: QWidget) -> None:
        self.body_layout.insertWidget(self.body_layout.count() - 1, w)

    def _apply_collapsed(self, clip: int, index: int) -> None:
        is_open = (clip, index) not in self.collapsed
        header = self.headers.get((clip, index))
        if header is not None:
            header.set_open(is_open)
        for row in self.effect_rows.get((clip, index), []):
            row.setVisible(is_open)
            self._apply_graph(row)
        tools = self.mask_tools.get((clip, index))
        if tools is not None:
            tools.setVisible(is_open)
        custom = self.custom_rows.get((clip, index))
        if custom is not None:
            custom.setVisible(is_open)
        for (c, i, k), (mh, mrows, inverted) in self.masks.items():
            if (c, i) == (clip, index):
                mh.setVisible(is_open)
                mask_open = is_open and (c, i, k) not in self.collapsed_masks
                mh.set_open((c, i, k) not in self.collapsed_masks)
                for row in [*mrows, inverted]:
                    row.setVisible(mask_open)
                for row in mrows:
                    self._apply_graph(row)
        self.lane.update()

    def _apply_graph(self, row: ParamRow) -> None:
        if row.graph is None:
            return
        shown = row.key in self.open_graphs and row.animated and row.isVisibleTo(self)
        row.graph_twirl.setText("▾" if row.key in self.open_graphs else "▸")
        if shown != row.graph.isVisibleTo(self):
            row.graph.setVisible(shown)
            if not shown:
                row.graph.range = None
        if shown:
            self._fetch_graph(row)

    def _fetch_graph(self, row: ParamRow) -> None:
        if self.engine is None or not getattr(self.engine, "running", False) or row.graph is None:
            return
        graph = row.graph
        n = max(8, min(2000, self.lane.width() // 2))
        self.engine.execute(
            "effects.graph", {**row.target(), "samples": n},
            on_reply=lambda r, _b: graph.set_samples((r or {}).get("values"), (r or {}).get("seconds"))
            if isinstance(r, dict) and row.graph is graph else None,
            on_error=lambda _m: None,
        )

    def toggle_graph(self, row: ParamRow) -> None:
        """A parameter's twirl opens or shuts its value and velocity graphs."""
        self.open_graphs ^= {row.key}
        self._apply_graph(row)
        self.lane.update()

    def toggle_mask(self, clip: int, index: int, mask: int) -> None:
        self.collapsed_masks ^= {(clip, index, mask)}
        self._apply_collapsed(clip, index)

    def mask_pen(self, clip: int, effect: int, on: bool) -> None:
        """The pen: place a free-drawn Bezier mask's points on the Program monitor."""
        monitor = getattr(self.window(), "monitor", None)
        overlay = getattr(monitor, "overlay", None)
        if overlay is not None and hasattr(overlay, "start_pen"):
            if on:
                overlay.start_pen(clip, effect)
            else:
                overlay.stop_pen()

    def pen_done(self) -> None:
        """The pen on the Program monitor closed or cancelled its mask: its button goes off."""
        for tools in self.mask_tools.values():
            pen = tools.buttons.get("pen")
            if pen is not None and pen.isChecked():
                pen.blockSignals(True)
                pen.setChecked(False)
                pen.blockSignals(False)

    def toggle_collapsed(self, clip: int, index: int) -> None:
        """Only the twirl arrow opens or shuts an effect (the rest of its header selects it)."""
        self.collapsed ^= {(clip, index)}
        self._apply_collapsed(clip, index)

    # ----- keyframes -----
    def set_keyframe_selection(self, keys: list[tuple[Any, ...]]) -> None:
        self.keyframe_selection = list(dict.fromkeys(keys))
        self.lane.update()

    def select_all_keyframes(self) -> bool:
        """Edit ▸ Select All with the panel focused: every keyframe it shows. False when the panel
        is not focused or shows none (the Timeline selects its clips then)."""
        focus = QApplication.focusWidget()
        if focus is None or not (focus is self or self.isAncestorOf(focus)):
            return False
        keys = [pt["ref"] for pt in self.lane.points()]
        if not keys:
            return False
        self.set_keyframe_selection(keys)
        return True

    def move_keyframes(self, keys: list[tuple[Any, ...]], delta: int) -> None:
        """One ``effects.moveKeyframes`` for the dragged selection; the moved keys stay selected."""
        refs = []
        for clip, effect, param, mask, media in keys:
            ref: dict[str, Any] = {"clip": clip, "effect": effect, "param": param, "time": media}
            if mask is not None:
                ref["mask"] = mask
            refs.append(ref)
        if not refs or self.engine is None:
            return
        self.flush()
        win = self.window()

        def done(result: Any, _b: bytes | None) -> None:
            moved = result.get("keyframes") if isinstance(result, dict) else None
            if isinstance(moved, list):
                self.set_keyframe_selection([
                    (k.get("clip"), k.get("effect"), k.get("param"), k.get("mask"), k.get("time"))
                    for k in moved if isinstance(k, dict)
                ])
            refresh = getattr(win, "schedule_refresh", None)
            if callable(refresh):
                refresh()

        def failed(message: str) -> None:
            report = getattr(win, "_command_failed", None)
            if callable(report):
                report("effects.moveKeyframes", message)

        self.engine.execute("effects.moveKeyframes", {"keyframes": refs, "delta": int(delta)}, on_reply=done,
                            on_error=failed)

    def save_preset(self, clip: int, effects: list[int], name: str) -> None:
        dlg = SavePresetDialog(self, name)
        dlg.accepted.connect(lambda: self.send("presets.save", {"clip": clip, "effects": effects, **dlg.values()}))
        self.last_dialog = dlg  # kept until the next one (and reachable by tests and agents)
        dlg.open()

    # ----- edits -----
    def set_param(self, row: ParamRow, value: Any, immediate: bool = False) -> None:
        """Send a new value; spin-box drags are coalesced and become one undo step."""
        if self.clip_id is None:
            return
        at = self._edit_time(None, row.snapshot or self.clip) if row.animated else None
        self._pending[(row.clip, row.effect, row.id, row.mask, at)] = value
        if immediate:
            self.flush()
        else:
            self._send_timer.start()

    def _edit_time(self, playhead: int | None = None, clip: dict[str, Any] | None = None) -> int | None:
        """Where an animated edit is keyed: ``playhead`` when given, the Program monitor's frame while
        it plays (the engine playhead and this snapshot stay where playback started), else the shown
        snapshot's playhead; pinned to the clip as the engine pins its own."""
        clip = clip or self.clip or {}
        ph, start, end = clip.get("playhead"), clip.get("start"), clip.get("end")
        monitor = getattr(self.window(), "monitor", None)
        if isinstance(playhead, int):
            ph = playhead
        elif monitor is not None and getattr(monitor, "playing", False) and isinstance(getattr(monitor, "playhead", None), int):
            ph = monitor.playhead
        if not all(isinstance(v, int) for v in (ph, start, end)):
            return None
        return max(start, min(ph, max(start, end - 1)))

    def new_gesture(self) -> None:
        """The next value starts its own undo step, however soon it follows the last one."""
        self.flush()
        self._new_gesture = True

    def send(self, command: str, params: dict[str, Any]) -> None:
        """Any other edit from the panel: the coalesced values go first, so they don't land on a
        reset, removed or reordered effect."""
        self.flush()
        if command in AT_PLAYHEAD:
            # the engine acts at its playhead, which stays where playback started: stop on the
            # shown frame first (the stop seeks the engine there)
            monitor = getattr(self.window(), "monitor", None)
            if monitor is not None and getattr(monitor, "playing", False):
                monitor.stop()
        self.command.emit(command, params)

    def flush(self) -> None:
        """Send the coalesced values now (also before a save or an unsaved-changes check)."""
        self._send_timer.stop()
        if not self._pending:
            return
        begin = getattr(self, "_new_gesture", False) or not self._last_send.isValid() or self._last_send.elapsed() > GESTURE_GAP_MS
        self._new_gesture = False
        for (clip, effect, param, mask, at), value in self._pending.items():
            params = {"clip": clip, "effect": effect, "param": param, "value": value, "merge": True, "begin": begin}
            if mask is not None:
                params["mask"] = mask
            if at is not None:
                params["time"] = at
            self.command.emit("effects.setParam", params)
        self._pending.clear()
        self._last_send.restart()
