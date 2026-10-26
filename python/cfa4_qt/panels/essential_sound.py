"""The Essential Sound panel, as egui's (crates/ui-egui/src/panels/essential_sound.rs).

Browse lists the built-in and saved presets of each audio type (a click applies one to the
selected audio clips); Edit assigns an audio type, then shows the type's sections (Loudness,
Repair, Clarity, Creative, Ducking, Duration, Pan) with their switches and sliders, the preset
row (apply, save, delete) and the Clip Volume / Mute footer. The engine reads it out
(``essentialSound.inspect``) and every control is an ``essentialSound.*`` command; a slider
drag is one undo step (``begin`` on the press).
"""

from __future__ import annotations

import math
from collections.abc import Callable
from typing import Any

from PySide6.QtCore import QEvent, QPointF, QRectF, QSettings, Qt, QTimer, Signal
from PySide6.QtGui import QColor, QFont, QKeyEvent, QMouseEvent, QPainter, QPainterPath, QPaintEvent, QPen
from PySide6.QtWidgets import (
    QComboBox,
    QDoubleSpinBox,
    QFrame,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPushButton,
    QScrollArea,
    QSizePolicy,
    QStackedWidget,
    QTabBar,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from .. import theme
from ..i18n import N_, fmt, tr
from .effect_controls import SEND_DELAY_MS

# (command id, label) of the four audio types, egui's order
TYPES = (("dialogue", N_("Dialogue")), ("music", N_("Music")), ("sfx", N_("SFX")), ("ambience", N_("Ambience")))
SECTION_LABELS = (N_("Loudness"), N_("Repair"), N_("Clarity"), N_("Creative"), N_("Ducking"), N_("Duration"), N_("Pan"))
SECTION_KEYS = {
    "Loudness": "loudness.enabled",
    "Repair": "repair.enabled",
    "Clarity": "clarity.enabled",
    "Creative": "creative.enabled",
    "Ducking": "ducking.enabled",
    "Pan": "pan.enabled",
}
CLOSED_BY_DEFAULT = {"Loudness", "Pan", "Duration"}
# the built-in preset, EQ and reverb names, for the catalog (shown through tr())
BUILTIN_NAMES = (
    N_("(Default)"),
    N_("Balanced Voice"),
    N_("Clean Up Noisy Dialogue"),
    N_("Podcast Voice"),
    N_("Dry Up a Roomy Voice"),
    N_("Phone Call"),
    N_("Radio Announcer"),
    N_("Voice in a Hall"),
    N_("Duck Under Dialogue"),
    N_("Duck Deep Under Dialogue"),
    N_("Duck Under Voice and Effects"),
    N_("From the Left"),
    N_("From the Right"),
    N_("In a Room"),
    N_("Far Away"),
    N_("Wide Outdoor"),
    N_("Interior Room Tone"),
    N_("Presence Lift"),
    N_("Warm Voice"),
    N_("Clear Muddiness"),
    N_("Soften Harshness"),
    N_("Bright Air"),
    N_("Radio Voice"),
    N_("Telephone"),
    N_("Small Room"),
    N_("Warm Room"),
    N_("Medium Room"),
    N_("Bright Studio"),
    N_("Large Hall"),
    N_("Stone Church"),
    N_("Open Air"),
)
SWITCH_OFF = QColor(0x55, 0x55, 0x55)
TRACK = QColor(0x6E, 0x6E, 0x6E)
KNOB = QColor(0xB0, 0xB0, 0xB0)


def snake(part: str) -> str:
    return "".join("_" + ch.lower() if ch.isupper() else ch for ch in part)


def setting(st: dict[str, Any], key: str) -> Any:
    """A setting by egui's dotted key (``repair.humHz``) from the engine's snake_case settings."""
    cur: Any = st
    for part in key.split("."):
        if not isinstance(cur, dict):
            return None
        cur = cur.get(snake(part))
    return cur


def number(value: Any, fallback: float = 0.0) -> float:
    try:
        v = float(value)
    except (TypeError, ValueError):
        return fallback
    return v if math.isfinite(v) else fallback


def paint_type(p: QPainter, r: QRectF, kind: str, color: QColor) -> None:
    """egui's audio-type glyphs: speech bubble, eighth note, burst, waves."""

    def at(x: float, y: float) -> QPointF:
        return QPointF(r.left() + x * r.width(), r.top() + y * r.height())

    p.save()
    p.setRenderHint(QPainter.RenderHint.Antialiasing)
    p.setPen(QPen(color, 1.4))
    p.setBrush(Qt.BrushStyle.NoBrush)
    if kind == "dialogue":
        p.drawRoundedRect(QRectF(at(0.08, 0.12), at(0.92, 0.68)), r.width() * 0.16, r.width() * 0.16)
        p.drawPolyline([at(0.3, 0.68), at(0.24, 0.9), at(0.48, 0.68)])
    elif kind == "music":
        p.setBrush(color)
        p.drawEllipse(at(0.32, 0.78), r.width() * 0.15, r.width() * 0.15)
        p.setBrush(Qt.BrushStyle.NoBrush)
        p.drawLine(at(0.45, 0.78), at(0.45, 0.1))
        p.drawPolyline([at(0.45, 0.1), at(0.78, 0.3), at(0.78, 0.48)])
    elif kind == "sfx":
        c = r.center()
        for k in range(12):
            a = k * math.tau / 12
            n = (0.5 if k % 2 == 0 else 0.3) * r.width()
            p.drawLine(c, c + QPointF(math.cos(a) * n, math.sin(a) * n))
    else:
        for k, y in enumerate((0.3, 0.55, 0.8)):
            path = QPainterPath()
            for i in range(13):
                x = i / 12
                q = at(0.05 + x * 0.9, y + 0.07 * math.sin(x * math.tau + k))
                path.moveTo(q) if i == 0 else path.lineTo(q)
            p.drawPath(path)
    p.restore()


class Switch(QWidget):
    """egui's pill switch."""

    clicked = Signal()

    def __init__(self, name: str, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName(name)
        self.setFixedSize(34, 21)
        self.on = False
        self.setCursor(Qt.CursorShape.PointingHandCursor)

    def set_on(self, on: bool, active: bool = True) -> None:
        self.on = on
        self.setEnabled(active)
        self.update()

    def paintEvent(self, _e: QPaintEvent) -> None:
        t = theme.current()
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        r = QRectF(2, 3, 30, 15)
        rad = r.height() / 2
        if self.on:
            p.setPen(Qt.PenStyle.NoPen)
            p.setBrush(t.c("accent") if self.isEnabled() else SWITCH_OFF)
            p.drawRoundedRect(r, rad, rad)
            p.setBrush(QColor("white"))
            p.drawEllipse(QPointF(r.right() - rad, r.center().y()), rad - 3, rad - 3)
        else:
            p.setPen(QPen(SWITCH_OFF if self.isEnabled() else t.c("separator"), 1.5))
            p.setBrush(Qt.BrushStyle.NoBrush)
            p.drawRoundedRect(r.adjusted(0.75, 0.75, -0.75, -0.75), rad, rad)
            p.drawEllipse(QPointF(r.left() + rad, r.center().y()), rad - 3, rad - 3)

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() == Qt.MouseButton.LeftButton and self.isEnabled():
            self.clicked.emit()


class SoundSlider(QWidget):
    """egui's Essential Sound slider: the label and value above a 1 pt track with a ring knob
    (or, without a label, the track and the value on one row). The value is typed too."""

    changed = Signal(str, float, bool)  # key, value, begin (a new undo step)

    def __init__(self, key: str, label: str, lo: float, hi: float, decimals: int, suffix: str, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.key = key
        self.label = label
        self.lo, self.hi = lo, hi
        self.value = lo
        self.active = True
        self.dragging = False
        self.setObjectName(f"essentialSound.{key}")
        self.setFixedHeight(46 if label else 26)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Fixed)
        self.spin = QDoubleSpinBox(self)
        self.spin.setObjectName(f"essentialSound.{key}.value")
        self.spin.setRange(lo, hi)
        self.spin.setDecimals(decimals)
        self.spin.setSingleStep(10 ** -decimals if decimals else 10)
        self.spin.setSuffix(suffix)
        self.spin.setKeyboardTracking(False)
        self.spin.setAlignment(Qt.AlignmentFlag.AlignRight)
        self.spin.setFixedWidth(74)
        self.spin.valueChanged.connect(lambda v: self.changed.emit(self.key, float(v), True))
        self._style = ""

    def resizeEvent(self, _e: Any) -> None:
        y = 0 if self.label else (self.height() - 22) // 2
        self.spin.setGeometry(self.width() - self.spin.width(), y, self.spin.width(), 22)

    def set_value(self, value: Any, active: bool) -> None:
        t = theme.current()
        style = (
            f"QDoubleSpinBox {{ background: transparent; border: none; padding: 0 2px; color: {t.hot_text if active else t.text_faint}; }}"
            " QDoubleSpinBox::up-button, QDoubleSpinBox::down-button { width: 0px; border: none; }"
        )
        if style != self._style:
            self._style = style
            self.spin.setStyleSheet(style)
        self.active = active
        self.spin.setEnabled(active)
        if not self.dragging:
            self.value = max(self.lo, min(self.hi, number(value, self.lo)))
            if not self.spin.hasFocus():
                self.spin.blockSignals(True)
                self.spin.setValue(self.value)
                self.spin.blockSignals(False)
        self.update()

    def track(self) -> QRectF:
        if self.label:
            return QRectF(0, 32 - 1, self.width(), 2)
        return QRectF(42, self.height() / 2 - 1, max(10.0, self.width() - 42 - self.spin.width() - 6), 2)

    def paintEvent(self, _e: QPaintEvent) -> None:
        t = theme.current()
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        if self.label:
            p.setPen(t.c("text") if self.active else t.c("text_faint"))
            p.drawText(QRectF(0, 0, self.width() - self.spin.width(), 20), Qt.AlignmentFlag.AlignVCenter, self.label)
        tr_ = self.track()
        f = (self.value - self.lo) / (self.hi - self.lo) if self.hi > self.lo else 0.0
        kx = tr_.left() + max(0.0, min(1.0, f)) * tr_.width()
        p.fillRect(tr_, TRACK if self.active else t.c("separator"))
        p.setBrush(t.c("panel_bg"))
        p.setPen(QPen(t.c("hot_text") if self.dragging else (KNOB if self.active else t.c("text_faint")), 1.6))
        p.drawEllipse(QPointF(kx, tr_.center().y()), 6, 6)

    def _set_from(self, x: float, begin: bool) -> None:
        tr_ = self.track()
        f = max(0.0, min(1.0, (x - tr_.left()) / max(1.0, tr_.width())))
        v = round(self.lo + f * (self.hi - self.lo), max(1, self.spin.decimals()))
        v = round(v, self.spin.decimals())
        if v == self.value and not begin:
            return
        self.value = v
        self.spin.blockSignals(True)
        self.spin.setValue(v)
        self.spin.blockSignals(False)
        self.update()
        self.changed.emit(self.key, v, begin)

    def mousePressEvent(self, e: QMouseEvent) -> None:
        tr_ = self.track()
        if self.active and e.button() == Qt.MouseButton.LeftButton and tr_.left() - 7 <= e.position().x() <= tr_.right() + 7:
            self.dragging = True
            self._set_from(e.position().x(), True)

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        if self.dragging:
            self._set_from(e.position().x(), False)

    def mouseReleaseEvent(self, _e: QMouseEvent) -> None:
        if self.dragging:
            self.dragging = False
            self.update()


class SectionHeader(QWidget):
    """A section's twirl row: chevron, bold title, the section switch at the right."""

    twirled = Signal(str)

    def __init__(self, title: str, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.title = title
        self.open = False
        self.setObjectName(f"essentialSound.section.{title}")
        self.setFixedHeight(44)
        self.switch: Switch | None = None
        if title in SECTION_KEYS:
            self.switch = Switch(f"essentialSound.section.{title}.toggle", self)

    def resizeEvent(self, _e: Any) -> None:
        if self.switch is not None:
            self.switch.move(self.width() - self.switch.width(), (self.height() - self.switch.height()) // 2)

    def paintEvent(self, _e: QPaintEvent) -> None:
        t = theme.current()
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        p.setPen(t.c("separator"))
        p.drawLine(0, 0, self.width(), 0)
        mid = self.height() / 2
        p.setPen(Qt.PenStyle.NoPen)
        p.setBrush(t.c("text"))
        if self.open:
            p.drawPolygon([QPointF(5, mid - 3), QPointF(15, mid - 3), QPointF(10, mid + 3)])
        else:
            p.drawPolygon([QPointF(7, mid - 5), QPointF(13, mid), QPointF(7, mid + 5)])
        font = QFont(self.font())
        font.setPixelSize(13)
        font.setWeight(QFont.Weight.DemiBold)
        p.setFont(font)
        p.setPen(t.c("text"))
        p.drawText(QRectF(30, 0, self.width() - 80, self.height()), Qt.AlignmentFlag.AlignVCenter, tr(self.title))

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() == Qt.MouseButton.LeftButton:
            self.twirled.emit(self.title)


class TypeButton(QPushButton):
    """A big audio-type button (Edit, untyped selection) or a small duck-against toggle."""

    def __init__(self, kind: str, label: str, big: bool, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.kind = kind
        self.label = label
        self.big = big
        self.setFixedHeight(46 if big else 26)
        if not big:
            self.setFixedWidth(30)
            self.setCheckable(True)
            self.setToolTip(label)
        self.setFlat(True)
        self.setCursor(Qt.CursorShape.PointingHandCursor)

    def paintEvent(self, _e: QPaintEvent) -> None:
        t = theme.current()
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        hover = self.underMouse() and self.isEnabled()
        if self.big:
            r = QRectF(self.rect()).adjusted(2.5, 2.5, -2.5, -2.5)
            p.setPen(QPen(t.c("field_border"), 1))
            p.setBrush(t.c("hover") if hover else t.c("field_bg"))
            p.drawRoundedRect(r, 4, 4)
            paint_type(p, QRectF(15, self.height() / 2 - 11, 22, 22), self.kind, t.c("text"))
            font = QFont(self.font())
            font.setPixelSize(13)
            font.setWeight(QFont.Weight.DemiBold)
            p.setFont(font)
            p.setPen(t.c("text"))
            p.drawText(QRectF(50, 0, self.width() - 54, self.height()), Qt.AlignmentFlag.AlignVCenter, self.label)
        else:
            on = self.isChecked()
            if on or hover:
                p.setPen(Qt.PenStyle.NoPen)
                p.setBrush(t.c("pressed") if on else t.c("hover"))
                p.drawRoundedRect(QRectF(self.rect()), 4, 4)
            paint_type(p, QRectF(self.rect()).adjusted(6, 6, -6, -6), self.kind, t.c("text") if on else t.c("text_faint"))


class NameField(QLineEdit):
    """The preset name: Enter saves (not the editor's Enter shortcut), Escape cancels."""

    cancelled = Signal()

    def event(self, e: QEvent) -> bool:
        if e.type() == QEvent.Type.ShortcutOverride and isinstance(e, QKeyEvent) and e.key() in (
            Qt.Key.Key_Return,
            Qt.Key.Key_Enter,
            Qt.Key.Key_Escape,
        ):
            e.accept()
            return True
        return super().event(e)

    def keyPressEvent(self, e: QKeyEvent) -> None:
        if e.key() == Qt.Key.Key_Escape:
            self.cancelled.emit()
            return
        super().keyPressEvent(e)


def _hint(text: str, parent: QWidget, size: float = 11.5, dim: bool = False) -> QLabel:
    label = QLabel(text, parent)
    label.setWordWrap(True)
    t = theme.current()
    label.setStyleSheet(f"color: {t.text_dim if dim else t.text_faint}; font-size: {size}px;")
    return label


class EssentialSoundPanel(QWidget):
    command = Signal(str, dict)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.EssentialSound")
        self.settings: QSettings | None = None
        self.data: dict[str, Any] = {}
        self.st: dict[str, Any] = {}
        self.toggled: set[str] = set()  # sections twirled away from their default
        self.draft = False  # the Save Preset name row is shown
        self._shape: tuple[Any, ...] | None = None
        self._browse_shape: tuple[Any, ...] | None = None
        self._updaters: list[Callable[[dict[str, Any]], None]] = []
        self._pending: dict[str, Any] = {}
        self._send_timer = QTimer(self, singleShot=True, interval=SEND_DELAY_MS)
        self._send_timer.timeout.connect(self.flush)

        self.tabs = QTabBar(self)
        self.tabs.setObjectName("essentialSound.tabs")
        self.tabs.setDrawBase(False)
        self.tabs.setExpanding(False)
        self.tabs.setStyleSheet("QTabBar { background: transparent; }")  # the app sheet paints dock tab bars
        self.tabs.addTab(tr("Browse"))
        self.tabs.addTab(tr("Edit"))
        self.tabs.setCurrentIndex(1)
        self.tabs.currentChanged.connect(self._tab_changed)
        self.browse = QWidget(self)
        self.browse_layout = QVBoxLayout(self.browse)
        self.browse_layout.setContentsMargins(14, 4, 14, 14)
        self.browse_scroll = self._scroll(self.browse)
        self.edit = QWidget(self)
        self.edit_layout = QVBoxLayout(self.edit)
        self.edit_layout.setContentsMargins(14, 2, 12, 24)
        self.edit_layout.setSpacing(2)
        self.edit_scroll = self._scroll(self.edit)
        self.placeholder = QLabel(tr("Select audio clips to edit them here"), self)
        self.placeholder.setObjectName("essentialSound.placeholder")
        self.placeholder.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.placeholder.setEnabled(False)
        self.footer = self._footer()
        edit_page = QWidget(self)
        ev = QVBoxLayout(edit_page)
        ev.setContentsMargins(0, 0, 0, 0)
        ev.setSpacing(0)
        ev.addWidget(self.placeholder, 1)
        ev.addWidget(self.edit_scroll, 1)
        ev.addWidget(self.footer)
        self.stack = QStackedWidget(self)
        self.stack.addWidget(self.browse_scroll)
        self.stack.addWidget(edit_page)
        self.stack.setCurrentIndex(1)
        lay = QVBoxLayout(self)
        lay.setContentsMargins(6, 4, 0, 0)
        row = QHBoxLayout()
        row.addWidget(self.tabs)
        row.addStretch(1)
        lay.addLayout(row)
        lay.addWidget(self.stack, 1)
        self.set_state(None)

    def _scroll(self, body: QWidget) -> QScrollArea:
        s = QScrollArea(self)
        s.setWidgetResizable(True)
        s.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        s.setFrameShape(QFrame.Shape.NoFrame)
        s.setWidget(body)
        return s

    def _footer(self) -> QWidget:
        w = QWidget(self)
        w.setObjectName("essentialSound.footer")
        v = QVBoxLayout(w)
        v.setContentsMargins(14, 6, 14, 8)
        v.setSpacing(2)
        line = QFrame(w)
        line.setFrameShape(QFrame.Shape.HLine)
        line.setFrameShadow(QFrame.Shadow.Plain)
        line.setStyleSheet(f"color: {theme.current().separator};")
        v.addWidget(line)
        row = QHBoxLayout()
        self.volume_on = Switch("essentialSound.volume.on", w)
        self.volume_on.clicked.connect(lambda: self.set("volume.on", not bool(setting(self.st, "volume.on"))))
        row.addWidget(self.volume_on)
        row.addWidget(QLabel(tr("Clip Volume"), w))
        row.addStretch(1)
        v.addLayout(row)
        self.volume = SoundSlider("volume.levelDb", "", -24.0, 15.0, 1, " dB", w)
        self.volume.changed.connect(self._slid)
        v.addWidget(self.volume)
        row = QHBoxLayout()
        self.mute = Switch("essentialSound.mute", w)
        self.mute.clicked.connect(lambda: self.set("mute", not bool(setting(self.st, "mute"))))
        row.addWidget(self.mute)
        row.addWidget(QLabel(tr("Mute"), w))
        row.addStretch(1)
        v.addLayout(row)
        return w

    def attach_settings(self, settings: QSettings) -> None:
        self.settings = settings
        raw = settings.value("essentialSound/twirled", [])
        self.toggled = {str(s) for s in (raw if isinstance(raw, list) else [raw] if raw else [])}
        tab = settings.value("essentialSound/tab", "Edit")
        self.tabs.setCurrentIndex(0 if tab == "Browse" else 1)

    # ----- state -----
    @property
    def clips(self) -> list[dict[str, Any]]:
        return [c for c in self.data.get("clips") or [] if isinstance(c, dict)]

    def clip_ids(self) -> list[Any]:
        return [c.get("clip") for c in self.clips]

    def kind(self) -> str | None:
        """The selection's audio type id, when every clip has the same one."""
        t = self.data.get("type")
        return next((k for k, label in TYPES if label == t), None)

    def presets(self, kind: str) -> list[dict[str, Any]]:
        label = dict(TYPES)[kind]
        return [p for p in (self.data.get("presets") or {}).get(label) or [] if isinstance(p, dict)]

    def is_open(self, section: str) -> bool:
        return (section not in CLOSED_BY_DEFAULT) != (section in self.toggled)

    def set_state(self, data: Any) -> None:
        """Show what ``essentialSound.inspect`` returned (or nothing)."""
        data = data if isinstance(data, dict) else {}
        if self.clip_ids() != [c.get("clip") for c in data.get("clips") or [] if isinstance(c, dict)]:
            self.flush()  # edits made so far go to the clips they were made on
        self.data = data
        self.st = next((c["settings"] for c in self.clips if isinstance(c.get("settings"), dict)), {})
        for key, v in self._pending.items():  # a drag still in its send delay shows its value
            self._put(key, v)
        browse = (len(self.clips) > 0, repr(self.data.get("presets")))
        if browse != self._browse_shape:
            self._browse_shape = browse
            self._build_browse()
        kind = self.kind()
        shape = (
            tuple(self.clip_ids()),
            self.data.get("type"),
            kind is not None and bool(self.st),
            frozenset(s for s in SECTION_LABELS if self.is_open(s)),
            self.draft,
        )
        if shape != self._shape:
            self._shape = shape
            self._build_edit()
        for update in self._updaters:
            update(self.st)
        has = bool(self.clips)
        self.placeholder.setVisible(not has)
        self.edit_scroll.setVisible(has)
        typed = has and kind is not None and bool(self.st)
        self.footer.setVisible(typed)
        if typed:
            self.volume_on.set_on(bool(setting(self.st, "volume.on")))
            self.volume.set_value(setting(self.st, "volume.levelDb"), bool(setting(self.st, "volume.on")))
            self.mute.set_on(bool(setting(self.st, "mute")))

    def _put(self, key: str, value: Any) -> None:
        cur = self.st
        parts = [snake(p) for p in key.split(".")]
        for part in parts[:-1]:
            nxt = cur.get(part)
            if not isinstance(nxt, dict):
                return
            cur = nxt
        cur[parts[-1]] = value

    # ----- sending -----
    def send(self, command: str, params: dict[str, Any]) -> None:
        self.flush()
        if "clips" not in params and command != "essentialSound.deletePreset":
            params = {**params, "clips": self.clip_ids()}
        self.command.emit(command, params)

    def set(self, key: str, value: Any, begin: bool = True) -> None:
        """One setting; ``begin`` starts a new undo step (a drag's later values merge into it)."""
        if begin:
            self.flush()
            self.send("essentialSound.set", {"key": key, "value": value, "begin": True})
            return
        self._pending[key] = value
        self._send_timer.start()

    def flush(self) -> None:
        self._send_timer.stop()
        pending, self._pending = self._pending, {}
        for key, value in pending.items():
            self.command.emit("essentialSound.set", {"key": key, "value": value, "begin": False, "clips": self.clip_ids()})

    def _slid(self, key: str, value: float, begin: bool) -> None:
        if key == "ducking.fadeS":  # edited in ms, stored in seconds
            value = round(value / 1000.0, 2)
        self._put(key, value)
        self.set(key, value, begin)

    # ----- Browse -----
    def _clear(self, layout: QVBoxLayout) -> None:
        while layout.count():
            item = layout.takeAt(0)
            w = item.widget()
            if w is not None:
                w.hide()  # gone now, not only at the next deferred delete
                w.setParent(None)
                w.deleteLater()
            elif item.layout() is not None:
                self._clear(item.layout())  # type: ignore[arg-type]

    def _build_browse(self) -> None:
        lay = self.browse_layout
        self._clear(lay)
        has = bool(self.clips)
        if not has:
            lay.addWidget(_hint(tr("Select audio clips, then click a preset to apply it."), self.browse))
        for kind, label in TYPES:
            head = QWidget(self.browse)
            head.setFixedHeight(30)
            hl = QHBoxLayout(head)
            hl.setContentsMargins(24, 8, 0, 0)
            title = QLabel(tr(label), head)
            title.setStyleSheet("font-weight: 600;")
            hl.addWidget(title)
            hl.addStretch(1)
            head.paintEvent = lambda _e, h=head, k=kind: self._paint_head(h, k)  # type: ignore[method-assign]
            lay.addWidget(head)
            for p in self.presets(kind):
                name = str(p.get("name") or "")
                text = tr(name) if p.get("builtin") else fmt("{name} (user)", name=name)
                b = QPushButton(text, self.browse)
                b.setObjectName(f"essentialSound.browse.{label}.{name}")
                b.setFlat(True)
                b.setEnabled(has)
                b.setStyleSheet(
                    "QPushButton { text-align: left; padding: 3px 4px 3px 24px; border: none; background: transparent; }"
                    f" QPushButton:hover {{ background: {theme.current().hover}; }}"
                )
                b.clicked.connect(lambda _c=False, k=kind, n=name: self.send("essentialSound.applyPreset", {"type": k, "preset": n}))
                lay.addWidget(b)
        lay.addStretch(1)

    def _paint_head(self, head: QWidget, kind: str) -> None:
        p = QPainter(head)
        paint_type(p, QRectF(0, head.height() / 2 - 4, 16, 16), kind, theme.current().c("text"))

    # ----- Edit -----
    def _build_edit(self) -> None:
        lay = self.edit_layout
        self._clear(lay)
        self._updaters = []
        if not self.clips:
            return
        body = self.edit
        title = QLabel(tr("Multiple Clips Selected") if len(self.clips) > 1 else str(self.clips[0].get("name") or ""), body)
        title.setObjectName("essentialSound.title")
        title.setStyleSheet("font-size: 14px; font-weight: 600;")
        lay.addWidget(title)
        lay.addSpacing(6)
        kind = self.kind()
        if kind is None or not self.st:
            mixed = self.data.get("type") == "mixed"
            lay.addWidget(
                _hint(
                    tr("The selected clips have different audio types. Assign one type to edit them together.")
                    if mixed
                    else tr("Assign an audio type to the selection:"),
                    body,
                    12,
                    dim=True,
                )
            )
            lay.addSpacing(6)
            for k, label in TYPES:
                b = TypeButton(k, tr(label), True, body)
                b.setObjectName(f"essentialSound.type.{label}")
                b.clicked.connect(lambda _c=False, kk=k: self.send("essentialSound.setType", {"type": kk}))
                lay.addWidget(b)
            lay.addStretch(1)
            return
        self._typed_body(kind)
        lay.addStretch(1)

    def _row(self) -> QHBoxLayout:
        row = QHBoxLayout()
        row.setContentsMargins(0, 0, 0, 0)
        row.setSpacing(6)
        self.edit_layout.addLayout(row)
        return row

    def _typed_body(self, kind: str) -> None:
        body = self.edit
        label = dict(TYPES)[kind]
        # the type badge and Clear Audio Type
        row = self._row()
        badge = QWidget(body)
        badge.setFixedSize(22, 30)
        badge.paintEvent = lambda _e, w=badge: self._paint_badge(w, kind)  # type: ignore[method-assign]
        row.addWidget(badge)
        row.addWidget(QLabel(tr(label), body))
        row.addStretch(1)
        clear = QPushButton(tr("Clear Audio Type"), body)
        clear.setObjectName("essentialSound.clearType")
        clear.clicked.connect(lambda: self.send("essentialSound.clearType", {}))
        row.addWidget(clear)
        self.edit_layout.addSpacing(6)
        # Preset: [▾] save delete
        self.edit_layout.addWidget(_hint(tr("Preset:"), body, 12, dim=True))
        row = self._row()
        mine = self.presets(kind)
        combo = QComboBox(body)
        combo.setObjectName("essentialSound.preset")
        combo.setSizeAdjustPolicy(QComboBox.SizeAdjustPolicy.AdjustToMinimumContentsLengthWithIcon)
        combo.setMinimumContentsLength(8)
        combo.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Fixed)
        combo.activated.connect(
            lambda i: combo.itemData(i)
            and combo.itemData(i) != self.st.get("preset")
            and self.send("essentialSound.applyPreset", {"preset": combo.itemData(i)})
        )
        row.addWidget(combo, 1)
        save = QToolButton(body)
        save.setObjectName("essentialSound.preset.save")
        save.setText("⤓")
        save.setAutoRaise(True)
        save.setToolTip(tr("Save Preset"))
        save.clicked.connect(self._open_draft)
        row.addWidget(save)
        delete = QToolButton(body)
        delete.setObjectName("essentialSound.preset.delete")
        delete.setText("🗑")
        delete.setAutoRaise(True)
        delete.setToolTip(tr("Delete Preset"))
        delete.clicked.connect(lambda: self.send("essentialSound.deletePreset", {"name": self.st.get("preset"), "type": kind}))
        row.addWidget(delete)

        def update(st: dict[str, Any]) -> None:
            # "(Custom)" once a setting is changed by hand (the engine clears the preset)
            current = str(st.get("preset") or "")
            items = ([(tr("(Custom)"), None)] if not current else []) + [
                (tr(str(p.get("name") or "")) if p.get("builtin") else str(p.get("name") or ""), p.get("name")) for p in mine
            ]
            if [(combo.itemText(i), combo.itemData(i)) for i in range(combo.count())] != items and not combo.view().isVisible():
                combo.clear()
                for text, name in items:
                    combo.addItem(text, name)
            combo.setCurrentIndex(max(0, combo.findData(current)) if current else 0)
            delete.setEnabled(any(not p.get("builtin") and p.get("name") == current for p in mine))

        self._updaters.append(update)
        if self.draft:
            row = self._row()
            name = NameField(body)
            name.setObjectName("essentialSound.preset.name")
            name.setPlaceholderText(tr("Preset name"))
            ok = QPushButton(tr("Save"), body)
            ok.setObjectName("essentialSound.preset.ok")

            def save_draft() -> None:
                if name.text().strip():
                    self.draft = False
                    self.send("essentialSound.savePreset", {"name": name.text().strip()})

            name.returnPressed.connect(save_draft)
            name.cancelled.connect(self._close_draft)
            ok.clicked.connect(save_draft)
            row.addWidget(name, 1)
            row.addWidget(ok)
            QTimer.singleShot(0, name.setFocus)
        self.edit_layout.addSpacing(10)
        for sec in self.data.get("sections", {}).get(label) or []:
            self._section(str(sec), kind)
        line = QFrame(body)
        line.setFrameShape(QFrame.Shape.HLine)
        line.setStyleSheet(f"color: {theme.current().separator};")
        self.edit_layout.addWidget(line)

    def _paint_badge(self, w: QWidget, kind: str) -> None:
        p = QPainter(w)
        paint_type(p, QRectF(1, w.height() / 2 - 9, 18, 18), kind, theme.current().c("text"))

    def _open_draft(self) -> None:
        self.draft = True
        self.set_state(self.data)

    def _close_draft(self) -> None:
        self.draft = False
        self.set_state(self.data)

    def _twirl(self, section: str) -> None:
        self.toggled ^= {section}
        if self.settings is not None:
            self.settings.setValue("essentialSound/twirled", sorted(self.toggled))
        self.set_state(self.data)

    def _tab_changed(self, i: int) -> None:
        self.stack.setCurrentIndex(i)
        if self.settings is not None:
            self.settings.setValue("essentialSound/tab", "Browse" if i == 0 else "Edit")

    def _section(self, sec: str, kind: str) -> None:
        body = self.edit
        head = SectionHeader(sec, body)
        head.open = self.is_open(sec)
        head.twirled.connect(self._twirl)
        key = SECTION_KEYS.get(sec)
        if head.switch is not None and key is not None:
            head.switch.clicked.connect(lambda k=key: self.set(k, not bool(setting(self.st, k))))
            sw = head.switch
            self._updaters.append(lambda st, s=sw, k=key: s.set_on(bool(setting(st, k))))
        self.edit_layout.addWidget(head)
        if not head.open:
            return
        self.edit_layout.addSpacing(4)

        def on(st: dict[str, Any]) -> bool:
            return key is None or setting(st, key) is not False

        if sec == "Loudness":
            self._loudness()
        elif sec == "Repair":
            self._slot(tr("Reduce Noise"), "repair.noise", on)
            self._slot(tr("Reduce Rumble"), "repair.rumble", on)
            self._slot(tr("DeHum"), "repair.dehum", on)
            self._choice("repair.humHz", ["50 Hz", "60 Hz"], [50, 60], lambda st: on(st) and bool(setting(st, "repair.dehum.on")))
            self._slot(tr("DeEss"), "repair.deess", on)
            self._slot(tr("Reduce Reverb"), "repair.reverb", on)
        elif sec == "Clarity":
            self._slot(tr("Dynamics"), "clarity.dynamics", on)
            eq = lambda st: on(st) and bool(setting(st, "clarity.eq.on"))
            self._switch_row(tr("EQ"), "clarity.eq.on", on)
            self._combo("clarity.eqPreset", [str(n) for n in self.data.get("eqPresets") or []], eq)
            self._slider(tr("Amount"), "clarity.eq.amount", 0.0, 10.0, 1, "", eq)
            enhance = lambda st: on(st) and bool(setting(st, "clarity.enhance.on"))
            self._switch_row(tr("Enhance Speech"), "clarity.enhance.on", on)
            self._choice("clarity.enhanceTone", [tr("Low Tone"), tr("High Tone")], [0, 1], enhance)
            self._slider(tr("Mix"), "clarity.enhance.amount", 0.0, 10.0, 1, "", enhance)
        elif sec == "Creative":
            reverb = lambda st: on(st) and bool(setting(st, "creative.reverb.on"))
            self._switch_row(tr("Reverb"), "creative.reverb.on", on)
            self.edit_layout.addWidget(_hint(tr("Preset:"), body, 12, dim=True))
            self._combo("creative.reverbPreset", [str(n) for n in self.data.get("reverbPresets") or []], reverb)
            self._slider(tr("Amount"), "creative.reverb.amount", 0.0, 10.0, 1, "", reverb)
            if kind == "ambience":
                self._slot(tr("Stereo Width"), "creative.width", on)
        elif sec == "Ducking":
            self._ducking(on)
        elif sec == "Duration":
            self.edit_layout.addWidget(
                _hint(tr("Remixing music to a target duration is not available yet. Trim the clip and add a crossfade instead."), body)
            )
        elif sec == "Pan":
            self._slider(tr("Pan"), "pan.value", -100.0, 100.0, 1, "", on)
        self.edit_layout.addSpacing(8)

    def _switch_row(self, label: str, key: str, active: Callable[[dict[str, Any]], bool]) -> None:
        row = self._row()
        sw = Switch(f"essentialSound.{key}", self.edit)
        sw.clicked.connect(lambda: self.set(key, not bool(setting(self.st, key))))
        text = QLabel(label, self.edit)
        row.addWidget(sw)
        row.addSpacing(6)
        row.addWidget(text)
        row.addStretch(1)

        def update(st: dict[str, Any]) -> None:
            sw.set_on(bool(setting(st, key)), active(st))
            text.setEnabled(active(st))

        self._updaters.append(update)

    def _slider(self, label: str, key: str, lo: float, hi: float, decimals: int, suffix: str, active: Callable[[dict[str, Any]], bool]) -> None:
        s = SoundSlider(key, label, lo, hi, decimals, suffix, self.edit)
        s.changed.connect(self._slid)
        self.edit_layout.addWidget(s)
        scale = 1000.0 if key == "ducking.fadeS" else 1.0
        self._updaters.append(lambda st: s.set_value(number(setting(st, key)) * scale, active(st)))

    def _slot(self, label: str, key: str, active: Callable[[dict[str, Any]], bool]) -> None:
        """A switch row and its 0…10 amount slider."""
        self._switch_row(label, f"{key}.on", active)
        self._slider("", f"{key}.amount", 0.0, 10.0, 1, "", lambda st: active(st) and bool(setting(st, f"{key}.on")))

    def _choice(self, key: str, labels: list[str], values: list[Any], active: Callable[[dict[str, Any]], bool]) -> None:
        """egui's segmented choice (50 Hz | 60 Hz, Low Tone | High Tone)."""
        row = self._row()
        row.addSpacing(42)
        buttons = []
        for i, (text, value) in enumerate(zip(labels, values, strict=True)):
            b = QToolButton(self.edit)
            b.setObjectName(f"essentialSound.{key}.{i}")
            b.setText(text)
            b.setCheckable(True)
            b.setFixedSize(84, 22)
            b.clicked.connect(lambda _c=False, v=value: self.set(key, v))
            row.addWidget(b)
            buttons.append((b, value))
        row.addStretch(1)

        def update(st: dict[str, Any]) -> None:
            cur = setting(st, key)
            for b, value in buttons:
                b.setChecked(cur == value)
                b.setEnabled(active(st))

        self._updaters.append(update)

    def _combo(self, key: str, names: list[str], active: Callable[[dict[str, Any]], bool]) -> None:
        combo = QComboBox(self.edit)
        combo.setObjectName(f"essentialSound.{key}")
        combo.setSizeAdjustPolicy(QComboBox.SizeAdjustPolicy.AdjustToMinimumContentsLengthWithIcon)
        combo.setMinimumContentsLength(8)
        for n in names:
            combo.addItem(tr(n), n)
        combo.activated.connect(lambda i: combo.itemData(i) != setting(self.st, key) and self.set(key, combo.itemData(i)))
        self.edit_layout.addWidget(combo)

        def update(st: dict[str, Any]) -> None:
            if not combo.view().isVisible():
                combo.setCurrentIndex(max(0, combo.findData(setting(st, key))))
            combo.setEnabled(active(st))

        self._updaters.append(update)

    def _button(self, text: str, name: str, slot: Callable[[], None]) -> QPushButton:
        b = QPushButton(text, self.edit)
        b.setObjectName(name)
        b.clicked.connect(slot)
        return b

    def _loudness(self) -> None:
        row = self._row()
        row.addWidget(self._button(tr("Auto-Match"), "essentialSound.autoMatch", lambda: self.send("essentialSound.autoMatch", {})))
        text = QLabel(self.edit)
        text.setObjectName("essentialSound.loudness")
        text.setEnabled(False)
        row.addWidget(text, 1)

        def update(st: dict[str, Any]) -> None:
            m, tg = setting(st, "loudness.measuredLufs"), setting(st, "loudness.targetLufs")
            if m is not None and tg is not None:
                text.setText(f"{number(m):.1f} → {number(tg):.1f} LUFS ({number(setting(st, 'loudness.gainDb')):+.1f} dB)")
            else:
                text.setText(tr("Not matched yet"))

        self._updaters.append(update)
        if len(self.clips) > 1:
            self.edit_layout.addWidget(_hint(fmt("Matches each of the {n} clips on its own.", n=len(self.clips)), self.edit, 11))

    def _ducking(self, on: Callable[[dict[str, Any]], bool]) -> None:
        self.edit_layout.addWidget(_hint(tr("Duck against:"), self.edit, 12, dim=True))
        row = self._row()
        toggles = []
        for k, label in TYPES:
            b = TypeButton(k, tr(label), False, self.edit)
            b.setObjectName(f"essentialSound.ducking.against.{label}")

            def flip(_c: bool = False, lab: str = label) -> None:
                cur = [str(x) for x in setting(self.st, "ducking.against") or []]
                self.set("ducking.against", [x for x in cur if x != lab] if lab in cur else [*cur, lab])

            b.clicked.connect(flip)
            row.addWidget(b)
            toggles.append((b, label))
        untagged = QToolButton(self.edit)
        untagged.setObjectName("essentialSound.ducking.against.Untagged")
        untagged.setText(tr("Untagged"))
        untagged.setCheckable(True)
        untagged.setFixedHeight(26)
        untagged.clicked.connect(lambda: self.set("ducking.againstUntyped", not bool(setting(self.st, "ducking.againstUntyped"))))
        row.addSpacing(4)
        row.addWidget(untagged)
        row.addStretch(1)

        def update(st: dict[str, Any]) -> None:
            against = [str(x) for x in setting(st, "ducking.against") or []]
            for b, label in toggles:
                b.setChecked(label in against)
                b.setEnabled(on(st))
            untagged.setChecked(bool(setting(st, "ducking.againstUntyped")))
            untagged.setEnabled(on(st))

        self._updaters.append(update)
        self._slider(tr("Sensitivity"), "ducking.sensitivity", 0.0, 10.0, 1, "", on)
        self._slider(tr("Reduce By"), "ducking.reduceDb", -40.0, 0.0, 1, " dB", on)
        self._slider(tr("Fades"), "ducking.fadeS", 0.0, 5000.0, 0, " ms", on)
        row = self._row()
        gen = self._button(tr("Generate Keyframes"), "essentialSound.generateDucking", lambda: self.send("essentialSound.generateDucking", {}))
        row.addWidget(gen)
        row.addStretch(1)
        self._updaters.append(lambda st: gen.setEnabled(on(st)))
