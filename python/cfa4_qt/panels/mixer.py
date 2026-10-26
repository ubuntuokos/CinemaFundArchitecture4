"""Audio Track Mixer, as egui's (crates/ui-egui/src/panels/mixer.rs).

One strip per audio track, submix and the Mix: effect and send slots (behind the ▸ disclosure),
input mapping, output assignment, pan knob (or the 5.1 panner's values), automation mode,
M / S / R (◆ solo safe on submixes), fader with egui's taper and dB scale, meters, the fader value
and the strip name; a footer with the time, the duration and the transport. Every control is a
``mixer.*`` command: fader and knob drags are ``mixer.touch`` while held and ``mixer.release``
when let go, so they are heard live while playing and recorded as automation in Latch / Touch /
Write mode. What the strips show is ``mixer.inspect`` at the playhead; the meters are the
engine's per-strip peaks of the audio being played (``audio.render`` ``meter: true``).
"""

from __future__ import annotations

import itertools
import math
from typing import Any

from PySide6.QtCore import QPointF, QRectF, QSize, Qt, Signal
from PySide6.QtGui import QColor, QFont, QIcon, QMouseEvent, QPainter, QPaintEvent, QPen, QPixmap, QWheelEvent
from PySide6.QtWidgets import QHBoxLayout, QLabel, QMenu, QScrollArea, QToolButton, QVBoxLayout, QWidget

from .. import theme
from ..hot_number import HotSpin
from ..i18n import N_, tr
from ..timecode import format_timecode, frame_ticks
from .fx_editor import HAS_EDITOR
from .meters import AudioMetersPanel, to_db
from .transport import transport_icon

# egui's fader law: (dB, position 0 … 1), most of the travel around unity gain
TAPER = ((-96.0, 0.0), (-60.0, 0.05), (-40.0, 0.14), (-30.0, 0.24), (-20.0, 0.38), (-12.0, 0.52), (-6.0, 0.64), (0.0, 0.76), (6.0, 0.88), (15.0, 1.0))
SCALE = (15.0, 6.0, 0.0, -6.0, -12.0, -20.0, -30.0, -40.0, -60.0, -96.0)
FADER_MIN_DB, FADER_MAX_DB = -96.0, 15.0
MAX_INSERTS = MAX_SENDS = 5
STRIP_W = 98
SLOT_H = 15
RED = QColor(0xD8, 0x50, 0x3F)


def record_icon(on: bool) -> QIcon:
    """The mixer's record button: a red dot, dimmed while idle (egui's)."""
    pm = QPixmap(28, 28)
    pm.fill(Qt.GlobalColor.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.RenderHint.Antialiasing)
    c = QColor(RED)
    if not on:
        c.setAlphaF(0.55)
    p.setPen(Qt.PenStyle.NoPen)
    p.setBrush(c)
    p.drawEllipse(QPointF(14, 14), 12, 12)
    p.end()
    return QIcon(pm)
MUTE_ON = QColor(0x2D, 0x9D, 0x78)
SOLO_ON = QColor(0xF0, 0xF0, 0x4F)
MODES = ("Off", "Read", "Latch", "Touch", "Write")
INPUT_MAPS = ("Stereo", "Left", "Right", "Swap", "Mono")


def db_to_pos(db: float) -> float:
    if db <= TAPER[0][0]:
        return 0.0
    for (d0, p0), (d1, p1) in itertools.pairwise(TAPER):
        if db <= d1:
            return p0 + (p1 - p0) * (db - d0) / (d1 - d0)
    return 1.0


def pos_to_db(p: float) -> float:
    p = min(1.0, max(0.0, p))
    for (d0, p0), (d1, p1) in itertools.pairwise(TAPER):
        if p <= p1:
            db = d0 + (d1 - d0) * (p - p0) / (p1 - p0)
            return FADER_MIN_DB if db <= FADER_MIN_DB + 0.5 else db
    return FADER_MAX_DB


def db_text(db: float) -> str:
    return "-∞" if db <= FADER_MIN_DB else f"{db:.1f}"


def hot(parent: QWidget, lo: float, hi: float, speed: float, centered: bool = False, decimals: int = 1, suffix: str = "") -> HotSpin:
    """egui's hot number on a painted strip: a fader or pan value to scrub or type."""
    spin = HotSpin(parent, lo, hi, decimals, speed, suffix)
    f = QFont(spin.font())
    f.setPixelSize(11)
    spin.setFont(f)
    if lo <= FADER_MIN_DB:
        spin.setSpecialValueText("-∞")
    if centered:
        spin.setAlignment(Qt.AlignmentFlag.AlignCenter)
    return spin


def place(spin: HotSpin, rect: QRectF | None, value: float) -> None:
    if rect is None:
        spin.hide()
        return
    spin.setGeometry(rect.toRect())
    spin.set_value(max(spin.minimum(), value))
    spin.show()


class Strip(QWidget):
    """One mixer strip, painted as egui paints it; the dropdowns open menus."""

    command = Signal(str, dict)

    def __init__(self, panel: TrackMixerPanel) -> None:
        super().__init__(panel.strips_box)
        self.panel = panel
        self.data: dict[str, Any] = {}
        self.levels: list[float] = []
        self.holds: list[float] = []
        self.setFixedWidth(STRIP_W)
        self.setMouseTracking(True)
        self._drag: str | None = None  # "fader" | "pan"
        self._last = QPointF()
        self._value: float = 0.0  # the dragged value, shown while the engine catches up
        self.regions: dict[str, QRectF] = {}
        self.hot_volume = hot(self, FADER_MIN_DB, FADER_MAX_DB, 0.1)
        self.hot_volume.edited.connect(lambda v, _b: self.command.emit("mixer.setValue", {"strip": self.sid(), "lane": "volume", "value": v}))
        self.hot_pan = hot(self, -100.0, 100.0, 1.0, centered=True)
        self.hot_pan.edited.connect(lambda v, _b: self.command.emit("mixer.setValue", {"strip": self.sid(), "lane": "pan", "value": v}))
        # the 5.1 panner's Center % and LFE
        self.hot_center = hot(self, 0.0, 100.0, 1.0, decimals=0, suffix="%")
        self.hot_center.edited.connect(lambda v, _b: self.command.emit("mixer.setValue", {"strip": self.sid(), "lane": "pan51.center", "value": v}))
        self.hot_lfe = hot(self, FADER_MIN_DB, FADER_MAX_DB, 0.5)
        self.hot_lfe.edited.connect(lambda v, _b: self.command.emit("mixer.setValue", {"strip": self.sid(), "lane": "pan51.lfe", "value": v}))
        self._puck = (0.0, 0.0)  # the dragged 5.1 puck

    # ----- data -----
    @property
    def ref(self) -> str:
        return str(self.data.get("ref") or "")

    @property
    def is_master(self) -> bool:
        return self.data.get("kind") == "mix"

    def sid(self) -> Any:
        return self.data.get("id")

    def at(self, key: str, default: float = 0.0) -> float:
        at = self.data.get("at") or {}
        v = at.get(key, self.data.get(key, default))
        try:
            return float(v)
        except (TypeError, ValueError):
            return default

    def volume(self) -> float:
        return self._value if self._drag == "fader" else self.at("volumeDb")

    def pan(self) -> float:
        return self._value if self._drag == "pan" else self.at("pan")

    def pan51(self) -> dict[str, float]:
        """The 5.1 panner's puck (x left … right, y rear … front), Center % and LFE dB."""
        d = self.data.get("pan51") or {}

        def num(k: str, default: float) -> float:
            try:
                return float(d.get(k, default))
            except (TypeError, ValueError):
                return default

        x, y = self._puck if self._drag == "pan51" else (num("x", 0.0), num("y", 100.0))
        return {"x": x, "y": y, "center": num("center", 100.0), "lfeDb": num("lfeDb", FADER_MIN_DB)}

    def set_data(self, data: dict[str, Any]) -> None:
        self.data = data
        self.setObjectName(f"mixer.{self.ref}")
        self.hot_volume.setObjectName(f"mixer.{self.ref}.value")
        self.hot_pan.setObjectName(f"mixer.{self.ref}.panValue")
        self.hot_center.setObjectName(f"mixer.{self.ref}.pan51.center")
        self.hot_lfe.setObjectName(f"mixer.{self.ref}.pan51.lfe")
        self.update()

    def channels(self) -> int:
        return 6 if self.data.get("channels") == "5.1" else 2

    # ----- layout -----
    def layout_regions(self) -> dict[str, QRectF]:
        """Where each control goes (egui's strip layout, top to bottom)."""
        w = self.width()
        r: dict[str, QRectF] = {}
        y = 4.0
        if self.panel.fx_open:
            for k in range(MAX_INSERTS):
                r[f"fx.{k}"] = QRectF(4, y, w - 8, SLOT_H - 2)
                y += SLOT_H
            y += 4
            for k in range(MAX_SENDS):
                if not self.is_master:
                    r[f"send.{k}"] = QRectF(4, y, w - 8, SLOT_H - 2)
                y += SLOT_H
            y += 6
        if not self.is_master:
            r["input"] = QRectF(4, y, w - 8, 20)
            y += 24
            r["output"] = QRectF(4, y, w - 8, 20)
            y += 26
            if self.data.get("pans51"):  # egui's pan51_panner: the listening square, Center, LFE
                r["pan51"] = QRectF(4, y, 46, 46)
                r["pan51.center"] = QRectF(54, y + 12, w - 58, 14)
                r["pan51.lfe"] = QRectF(54, y + 38, w - 58, 14)
            else:
                r["pan"] = QRectF(w / 2 - 19, y + 1, 38, 38)
                r["panValue"] = QRectF(w / 2 - 28, y + 40, 56, 16)
            y += 62
        else:
            y += 112
        r["mode"] = QRectF(4, y, w - 8, 20)
        y += 26
        if not self.is_master:
            bw = 18
            bx = w / 2 - bw * 1.5 - 6
            for i, key in enumerate(("mute", "solo", "third")):
                r[key] = QRectF(bx + i * (bw + 6), y, bw, 16)
        y += 24
        h = self.height()
        r["area"] = QRectF(0, y + 6, w, max(0.0, h - 20 - 18 - 4 - (y + 6)))
        area = r["area"]
        r["track"] = QRectF(33, area.top() + 8, 4, max(0.0, area.height() - 16))
        r["meter"] = QRectF(46, area.top(), 24, area.height())
        r["value"] = QRectF(4, h - 20 - 18, 56, 18)
        r["name"] = QRectF(0, h - 20, w, 20)
        return r

    def needed_height(self) -> float:
        return 4 + (10 * SLOT_H + 10 if self.panel.fx_open else 0) + 112 + 26 + 24 + 6 + 170 + 42

    def fader_y(self, db: float) -> float:
        t = self.regions["track"]
        return t.bottom() - db_to_pos(db) * t.height()

    def puck_pos(self, x: float, y: float) -> QPointF:
        inner = self.regions["pan51"].adjusted(5, 5, -5, -5)
        return QPointF(inner.center().x() + x / 100 * inner.width() / 2, inner.center().y() - y / 100 * inner.height() / 2)

    # ----- painting -----
    def paintEvent(self, _e: QPaintEvent) -> None:
        self.regions = self.layout_regions()
        r = self.regions
        place(self.hot_volume, r["value"], self.volume())
        place(self.hot_pan, r.get("panValue"), self.pan())
        p51 = self.pan51()
        place(self.hot_center, r.get("pan51.center"), p51["center"])
        place(self.hot_lfe, r.get("pan51.lfe"), p51["lfeDb"])
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        T = theme.current()
        p.fillRect(self.rect(), T.c("control_surface"))
        small = QFont(self.font())
        small.setPixelSize(10)
        p.setFont(small)

        def box(rect: QRectF, text: str, filled: bool = True, dim: bool = False, arrow: bool = True) -> None:
            p.setPen(QPen(T.c("field_border"), 1))
            p.setBrush(T.c("field_bg"))
            p.drawRoundedRect(rect.adjusted(0.5, 0.5, -0.5, -0.5), 2, 2)
            p.setPen(T.c("text_faint") if (not filled or dim) else T.c("text"))
            p.drawText(rect.adjusted(4, 0, -12, 0), Qt.AlignmentFlag.AlignVCenter | Qt.AlignmentFlag.AlignLeft, text)
            if arrow:
                p.setPen(QPen(T.c("text_dim"), 1.2))
                c = QPointF(rect.right() - 7, rect.center().y())
                p.drawLine(c + QPointF(-3, -1.5), c + QPointF(0, 1.5))
                p.drawLine(c + QPointF(0, 1.5), c + QPointF(3, -1.5))

        inserts = {int(e.get("slot", i)): e for i, e in enumerate(self.data.get("inserts") or [])}
        sends = {int(s.get("send", i)): s for i, s in enumerate(self.data.get("sends") or [])}
        for k in range(MAX_INSERTS):
            if f"fx.{k}" in r:
                e = inserts.get(k)
                text = ("▸ " if e and e.get("postFader") else "") + str((e or {}).get("name") or (e or {}).get("effect") or "") if e else ""
                box(r[f"fx.{k}"], text, e is not None, bool(e) and not e.get("enabled", True))
        for k in range(MAX_SENDS):
            if f"send.{k}" in r:
                s = sends.get(k)
                text = f"{tr('pre ') if s.get('preFader') else ''}{s.get('target')} {db_text(float(s.get('levelDb') or 0))}" if s else ""
                box(r[f"send.{k}"], text, s is not None, bool(s) and bool(s.get("muted")))
        if "input" in r:
            box(r["input"], tr(str(self.data.get("inputMap") or "Stereo")))
            out = str(self.data.get("output") or "Mix")
            box(r["output"], tr("Mix") if out == "Mix" else self.panel.strip_name(out))
            if "pan51" in r:
                sq = r["pan51"]
                grey = (lambda g: QColor(g, g, g))
                p.setPen(QPen(T.c("tab_text_active") if self._drag == "pan51" else grey(96 if T.dark else 150), 1))
                p.setBrush(grey(22 if T.dark else 196))
                p.drawRoundedRect(sq.adjusted(0.5, 0.5, -0.5, -0.5), 3, 3)
                p.setPen(Qt.PenStyle.NoPen)
                p.setBrush(grey(144 if T.dark else 100))
                for sx, sy in ((-100, 100), (0, 100), (100, 100), (-100, -100), (100, -100)):
                    c = self.puck_pos(sx, sy)
                    p.drawRoundedRect(QRectF(c.x() - 2, c.y() - 2, 4, 4), 1, 1)
                p.setBrush(T.c("accent") if self._drag == "pan51" else T.c("hot_text"))
                p.drawEllipse(self.puck_pos(p51["x"], p51["y"]), 4, 4)
                tiny = QFont(small)
                tiny.setPixelSize(9)
                p.setFont(tiny)
                p.setPen(T.c("text_dim"))
                p.drawText(QRectF(r["pan51.center"].left() + 2, sq.top(), 60, 12), Qt.AlignmentFlag.AlignVCenter, tr("Center"))
                p.drawText(QRectF(r["pan51.lfe"].left() + 2, sq.top() + 26, 60, 12), Qt.AlignmentFlag.AlignVCenter, "LFE")
                p.setFont(small)
            else:
                kc = r["pan"].center()
                ring = T.c("tab_text_active") if self._drag == "pan" else T.c("text_dim")
                p.setPen(QPen(ring, 2))
                p.setBrush(Qt.BrushStyle.NoBrush)
                p.drawEllipse(kc, 16, 16)
                ang = self.pan() / 100.0 * math.radians(135)
                p.drawLine(kc, kc + QPointF(math.sin(ang) * 14, -math.cos(ang) * 14))
                p.setPen(T.c("hot_text"))
                p.drawText(QRectF(kc.x() - 30, kc.y() + 10, 16, 14), Qt.AlignmentFlag.AlignCenter, "L")
                p.drawText(QRectF(kc.x() + 14, kc.y() + 10, 16, 14), Qt.AlignmentFlag.AlignCenter, "R")
        mode = str(self.data.get("mode") or "Read")
        box(r["mode"], tr(mode))
        if self.panel.recording and mode in ("Latch", "Touch", "Write"):
            p.setPen(QPen(RED, 1))
            p.setBrush(Qt.BrushStyle.NoBrush)
            p.drawRoundedRect(r["mode"], 4, 4)
        if "mute" in r:
            submix = self.data.get("kind") == "submix"
            for key, letter, on, color in (
                ("mute", "M", bool((self.data.get("at") or {}).get("mute", self.data.get("muted"))), MUTE_ON),
                ("solo", "S", bool(self.data.get("solo")), SOLO_ON),
                ("third", "◆" if submix else "R", bool(self.data.get("soloSafe") if submix else self.data.get("recordArm")), QColor(0xC8, 0xC8, 0xC8) if submix else RED),
            ):
                rect = r[key]
                p.setPen(QPen(T.c("button_border"), 1))
                p.setBrush(color if on else T.c("field_bg"))
                p.drawRoundedRect(rect, 3, 3)
                p.setPen(QColor("#111111") if on else T.c("text_dim"))
                p.drawText(rect, Qt.AlignmentFlag.AlignCenter, letter)
        self._paint_fader(p, T)
        nr = r["name"]
        p.setPen(QPen(T.c("separator"), 1))
        p.drawLine(QPointF(nr.left() + 4, nr.top()), QPointF(nr.right() - 4, nr.top()))
        name_font = QFont(self.font())
        name_font.setPixelSize(12)
        p.setFont(name_font)
        p.setPen(T.c("text"))
        if self.is_master:
            p.drawText(nr, Qt.AlignmentFlag.AlignCenter, tr("Mix"))
        else:
            p.drawText(nr.adjusted(6, 0, 0, 0), Qt.AlignmentFlag.AlignVCenter, self.ref)
            p.drawText(nr.adjusted(30, 0, -2, 0), Qt.AlignmentFlag.AlignVCenter, str(self.data.get("name") or ""))

    def _paint_fader(self, p: QPainter, T: theme.Tokens) -> None:
        r = self.regions
        area, track = r["area"], r["track"]
        if area.height() <= 40:
            return
        small = QFont(self.font())
        small.setPixelSize(9)
        p.setFont(small)
        tall = track.height() > 150
        p.setPen(T.c("text_dim"))
        p.drawText(QRectF(0, area.top() - 8, 26, 12), Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter, "dB")
        for d in SCALE:
            if not tall and d not in (15.0, 0.0, -12.0, -30.0, -96.0):
                continue
            y = self.fader_y(d)
            p.setPen(T.c("text_dim"))
            p.drawText(QRectF(0, y - 6, 26, 12), Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter, "-∞" if d <= FADER_MIN_DB else str(int(d)))
            p.setPen(QPen(T.c("text_faint"), 1))
            p.drawLine(QPointF(28, y), QPointF(31, y))
        p.setPen(QPen(T.c("separator"), 1))
        p.setBrush(QColor(0x10, 0x10, 0x10) if T.dark else QColor(0xC4, 0xC4, 0xC4))
        p.drawRoundedRect(track, 2, 2)
        vol = self.volume()
        cy = self.fader_y(vol)
        cap = QRectF(track.center().x() - 9, cy - 13, 18, 26)
        r["cap"] = cap
        col = T.c("accent") if self._drag == "fader" else (QColor(0xE0, 0xE0, 0xE0) if T.dark else QColor(0x32, 0x32, 0x32))
        p.setPen(QPen(col, 2))
        p.setBrush(QColor(0x2A, 0x2A, 0x2A) if T.dark else QColor(0xCD, 0xCD, 0xCD))
        p.drawRoundedRect(cap.adjusted(1, 1, -1, -1), 3, 3)
        p.drawLine(QPointF(cap.left() + 5, cy), QPointF(cap.right() - 5, cy))
        # meters (0 … -54 dB scale on the right), a clip light over each bar
        m = r["meter"]
        p.fillRect(m, T.c("meter_bg"))
        n = max(1, len(self.levels) or self.channels())
        gap = 1.0 if n > 2 else 2.0
        bw = (m.width() - gap * (n - 1)) / n
        for c in range(n):
            lv = self.levels[c] if c < len(self.levels) else -90.0
            hd = self.holds[c] if c < len(self.holds) else -90.0
            br = QRectF(m.left() + c * (bw + gap), m.top() + 6, bw, m.height() - 6)
            AudioMetersPanel._bar(p, br, lv, hd)
            p.fillRect(QRectF(br.left(), m.top(), bw, 4), RED if hd >= -0.1 else (QColor(0x30, 0x30, 0x30) if T.dark else QColor(0x9B, 0x9B, 0x9B)))
        top, h = m.top() + 6, m.height() - 6
        step = 6 if tall else 18
        small.setPixelSize(8)
        p.setFont(small)
        d = 0
        while d >= -54:
            y = top + h * (-d / 60.0)
            p.setPen(T.c("text_dim"))
            p.drawText(QRectF(72, y - 6, 19, 12), Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter, str(d))
            d -= step

    # ----- input -----
    def region_at(self, pos: QPointF) -> str | None:
        for key in ("cap", "pan", *[k for k in self.regions if k not in ("area", "track", "meter", "name", "cap", "pan")]):
            rect = self.regions.get(key)
            if rect is not None and rect.contains(pos):
                return key
        return None

    def mousePressEvent(self, e: QMouseEvent) -> None:
        key = self.region_at(e.position())
        if e.button() != Qt.MouseButton.LeftButton or key is None:
            return
        sid = self.sid()
        if key == "pan51":
            p51 = self.pan51()
            self._drag, self._puck, self._last = "pan51", (p51["x"], p51["y"]), e.position()
            for lane, v in (("pan51.x", p51["x"]), ("pan51.y", p51["y"])):
                self.command.emit("mixer.touch", {"strip": sid, "lane": lane, "value": v, "time": self.panel.now})
            self.update()
            return
        if key == "cap" or key == "pan":
            # read the strip's value first: while dragging, volume() / pan() answer the dragged one
            self._value = self.volume() if key == "cap" else self.pan()
            self._drag = "fader" if key == "cap" else "pan"
            self._last = e.position()
            self.command.emit("mixer.touch", {"strip": sid, "lane": "volume" if key == "cap" else "pan", "value": self._value, "time": self.panel.now})
            return
        menu = self.menu_for(key)
        if menu is not None:
            rect = self.regions[key]
            menu.exec(self.mapToGlobal(rect.bottomLeft().toPoint()))
            return
        if key == "mute":
            muted = bool((self.data.get("at") or {}).get("mute", self.data.get("muted")))
            if self.panel.recording and self.data.get("mode") in ("Latch", "Touch", "Write"):
                self.command.emit("mixer.touch", {"strip": sid, "lane": "mute", "value": 0.0 if muted else 1.0, "time": self.panel.now})
                self.command.emit("mixer.release", {"strip": sid, "lane": "mute", "time": self.panel.now})
            else:
                self.command.emit("mixer.setStrip", {"strip": sid, "muted": not bool(self.data.get("muted"))})
        elif key == "solo":
            self.command.emit("mixer.setStrip", {"strip": sid, "solo": not bool(self.data.get("solo"))})
        elif key == "third":
            if self.data.get("kind") == "submix":
                self.command.emit("mixer.setStrip", {"strip": sid, "soloSafe": not bool(self.data.get("soloSafe"))})
            else:
                self.command.emit("mixer.setStrip", {"strip": sid, "recordArm": not bool(self.data.get("recordArm"))})

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        if self._drag is None:
            key = self.region_at(e.position())
            self.setToolTip(self.tip(key))
            return
        d = e.position() - self._last
        self._last = e.position()
        if self._drag == "pan51":
            inner = self.regions["pan51"].adjusted(5, 5, -5, -5)
            x, y = self._puck
            nx = min(100.0, max(-100.0, x + d.x() / (inner.width() / 2) * 100))
            ny = min(100.0, max(-100.0, y - d.y() / (inner.height() / 2) * 100))
            self._puck = (nx, ny)
            if nx != x:
                self.command.emit("mixer.touch", {"strip": self.sid(), "lane": "pan51.x", "value": nx, "time": self.panel.now})
            if ny != y:
                self.command.emit("mixer.touch", {"strip": self.sid(), "lane": "pan51.y", "value": ny, "time": self.panel.now})
            self.update()
            return
        if self._drag == "fader":
            fine = 0.1 if e.modifiers() & Qt.KeyboardModifier.ControlModifier else 1.0
            t = self.regions["track"]
            self._value = pos_to_db(db_to_pos(self._value) - d.y() * fine / max(1.0, t.height()))
            lane = "volume"
        else:
            self._value = min(100.0, max(-100.0, self._value + d.x() - d.y()))
            lane = "pan"
        self.command.emit("mixer.touch", {"strip": self.sid(), "lane": lane, "value": self._value, "time": self.panel.now})
        self.update()

    def mouseReleaseEvent(self, e: QMouseEvent) -> None:
        if self._drag is not None:
            lanes = ["pan51.x", "pan51.y"] if self._drag == "pan51" else ["volume" if self._drag == "fader" else "pan"]
            for lane in lanes:
                self.command.emit("mixer.release", {"strip": self.sid(), "lane": lane, "time": self.panel.now})
            self._drag = None
            self.update()

    def mouseDoubleClickEvent(self, e: QMouseEvent) -> None:
        key = self.region_at(e.position())
        if key is not None and key.startswith("fx."):
            k = int(key.split(".")[1])
            inserts = {int(e.get("slot", i)): e for i, e in enumerate(self.data.get("inserts") or [])}
            e = inserts.get(k)
            if e is not None and str(e.get("effect") or "") in HAS_EDITOR:
                self.panel.fx_editor.emit({"strip": self.sid(), "slot": k})
        elif key == "cap":
            self.command.emit("mixer.setValue", {"strip": self.sid(), "lane": "volume", "value": 0.0})
        elif key == "pan":
            self.command.emit("mixer.setValue", {"strip": self.sid(), "lane": "pan", "value": 0.0})
        elif key == "pan51":  # front centre
            self.command.emit("mixer.setValue", {"strip": self.sid(), "lane": "pan51.x", "value": 0.0})
            self.command.emit("mixer.setValue", {"strip": self.sid(), "lane": "pan51.y", "value": 100.0})

    def wheelEvent(self, e: QWheelEvent) -> None:
        e.ignore()  # the panel scrolls

    def tip(self, key: str | None) -> str:
        return {
            "cap": tr("Volume (drag; double-click: 0 dB)"),
            "pan": tr("Pan / balance (drag; double-click: centre)"),
            "pan51": tr("5.1 panner (drag the puck; double-click: front centre)"),
            "mute": tr("Mute Track"),
            "solo": tr("Solo Track"),
            "third": tr("Solo safe") if self.data.get("kind") == "submix" else tr("Enable track for recording"),
            "input": tr("Input channel mapping"),
            "output": tr("Track output assignment"),
            "mode": tr("Automation mode"),
        }.get(key or "", "")

    def menu_for(self, key: str) -> QMenu | None:
        sid = self.sid()
        menu = QMenu(self)
        menu.setObjectName(f"mixer.{self.ref}.{key}.menu")

        def item(label: str, cmd: str, params: dict[str, Any], checked: bool | None = None, name: str = "") -> None:
            a = menu.addAction(label)
            a.setObjectName(f"mixer.{self.ref}.{key}.{name or label}")
            if checked is not None:
                a.setCheckable(True)
                a.setChecked(checked)
            a.triggered.connect(lambda _c=False: self.command.emit(cmd, params))

        if key == "input":
            for m in INPUT_MAPS:
                item(tr(m), "mixer.setStrip", {"strip": sid, "inputMap": m}, self.data.get("inputMap") == m, m)
        elif key == "output":
            item(tr("Mix"), "mixer.setStrip", {"strip": sid, "output": "Mix"}, self.data.get("output") == "Mix", "Mix")
            for s in self.panel.targets(self.data):
                item(str(s.get("name")), "mixer.setStrip", {"strip": sid, "output": s.get("ref")}, self.data.get("output") == s.get("ref"), str(s.get("ref")))
        elif key == "mode":
            for m in MODES:
                item(tr(m), "mixer.setStrip", {"strip": sid, "mode": m}, self.data.get("mode") == m, m)
        elif key.startswith("fx."):
            k = int(key.split(".")[1])
            inserts = {int(e.get("slot", i)): e for i, e in enumerate(self.data.get("inserts") or [])}
            e = inserts.get(k)
            if e is not None:
                if str(e.get("effect") or "") in HAS_EDITOR:
                    edit = menu.addAction(tr("Edit…"))
                    edit.setObjectName(f"mixer.{self.ref}.{key}.edit")
                    edit.triggered.connect(lambda _c=False: self.panel.fx_editor.emit({"strip": sid, "slot": k}))
                    menu.addSeparator()
                item(tr("Bypass"), "mixer.setInsert", {"strip": sid, "slot": k, "enabled": not e.get("enabled", True)}, not e.get("enabled", True), "bypass")
                item(tr("Post-Fader"), "mixer.setInsert", {"strip": sid, "slot": k, "postFader": not e.get("postFader")}, bool(e.get("postFader")), "postFader")
                menu.addSeparator()
                item(tr("Remove Effect"), "mixer.removeInsert", {"strip": sid, "slot": k}, None, "remove")
            elif k == len(inserts):
                folders: dict[str, QMenu] = {}
                for fx in self.panel.audio_effects:
                    folder = str(fx.get("path") or "").split("/")[-1] if "/" in str(fx.get("path") or "") else ""
                    target = menu
                    if folder:
                        if folder not in folders:
                            folders[folder] = menu.addMenu(tr(folder))
                        target = folders[folder]
                    fid = fx.get("id")
                    a = target.addAction(tr(str(fx.get("name") or fid)))
                    a.setObjectName(f"mixer.{self.ref}.fx.{k}.{fid}")
                    a.triggered.connect(lambda _c=False, f=fid: self.command.emit("mixer.addInsert", {"strip": sid, "effect": f}))
            else:
                menu.addAction(tr("Fill the slots above first")).setEnabled(False)
        elif key.startswith("send."):
            k = int(key.split(".")[1])
            sends = {int(s.get("send", i)): s for i, s in enumerate(self.data.get("sends") or [])}
            s = sends.get(k)
            if s is not None:
                item(tr("Pre-Fader"), "mixer.setSend", {"strip": sid, "send": k, "preFader": not s.get("preFader")}, bool(s.get("preFader")), "preFader")
                item(tr("Mute Send"), "mixer.setSend", {"strip": sid, "send": k, "muted": not s.get("muted")}, bool(s.get("muted")), "mute")
                menu.addSeparator()
                item(tr("Remove Send"), "mixer.removeSend", {"strip": sid, "send": k}, None, "remove")
            elif k == len(sends):
                for t in self.panel.targets(self.data):
                    item(f"{t.get('ref')} ({t.get('name')})", "mixer.addSend", {"strip": sid, "target": t.get("ref")}, None, str(t.get("ref")))
                if self.data.get("kind") != "submix":
                    a = menu.addAction(tr("New Submix"))
                    a.setObjectName(f"mixer.{self.ref}.send.{k}.newSubmix")
                    n = sum(1 for x in self.panel.strips_data if x.get("kind") == "submix")
                    a.triggered.connect(lambda: (self.command.emit("mixer.addSubmix", {}), self.command.emit("mixer.addSend", {"strip": sid, "target": f"S{n + 1}"})))
            else:
                menu.addAction(tr("Fill the slots above first")).setEnabled(False)
        else:
            return None
        return menu


class TrackMixerPanel(QWidget):
    command = Signal(str, dict)
    transport = Signal(str)  # a footer button: goIn, goOut, toggle, inToOut, loop
    fx_editor = Signal(dict)  # open the Track Fx Editor of {"strip", "slot"}

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.AudioTrackMixer")
        self.fx_open = False
        self.recording = False
        self.now = 0
        self.hidden: set[str] = set()
        self.meter_input_only = False
        self.input_level = 0.0  # the voice-over input's peak (linear), for Meter Input(s) Only
        self.strips_data: list[dict[str, Any]] = []
        self.audio_effects: list[dict[str, Any]] = []
        self.sequence: dict[str, Any] | None = None
        self.side = QWidget(self)
        side = QVBoxLayout(self.side)
        side.setContentsMargins(2, 4, 0, 0)
        side.setSpacing(4)
        self.fx_button = QToolButton(self.side)
        self.fx_button.setObjectName("mixer.showEffects")
        self.fx_button.setText("▸")
        self.fx_button.setAutoRaise(True)
        self.fx_button.setToolTip(tr("Show/Hide Effects and Sends"))
        self.fx_button.clicked.connect(self.toggle_fx)
        self.menu_button = QToolButton(self.side)
        self.menu_button.setObjectName("mixer.menu")
        self.menu_button.setText("≡")
        self.menu_button.setAutoRaise(True)
        self.menu_button.setToolTip(tr("Audio Track Mixer menu"))
        self.menu_button.setPopupMode(QToolButton.ToolButtonPopupMode.InstantPopup)
        self.menu_button.setStyleSheet("QToolButton::menu-indicator { image: none; }")
        self.menu_button.setMenu(QMenu(self.menu_button))
        self.menu_button.menu().aboutToShow.connect(self._fill_menu)
        for b in (self.fx_button, self.menu_button):
            b.setFixedSize(16, 20)
            side.addWidget(b)
        side.addStretch(1)
        self.side.setFixedWidth(18)
        self.strips_box = QWidget()
        self.strips_layout = QHBoxLayout(self.strips_box)
        self.strips_layout.setContentsMargins(0, 2, 0, 2)
        self.strips_layout.setSpacing(2)
        self.strips_layout.addStretch(1)
        self.scroll = QScrollArea(self)
        self.scroll.setWidgetResizable(True)
        self.scroll.setWidget(self.strips_box)
        self.scroll.setFrameShape(QScrollArea.Shape.NoFrame)
        self.strips: list[Strip] = []
        body = QHBoxLayout()
        body.setContentsMargins(0, 0, 0, 0)
        body.setSpacing(0)
        body.addWidget(self.side)
        body.addWidget(self.scroll, 1)
        # footer: time, transport, duration
        self.time_label = QLabel("00:00:00:00", self)
        self.time_label.setObjectName("mixer.time")
        self.duration_label = QLabel("", self)
        self.duration_label.setObjectName("mixer.duration")
        footer = QHBoxLayout()
        footer.setContentsMargins(12, 2, 12, 2)
        footer.addWidget(self.time_label)
        footer.addStretch(1)
        self.buttons: dict[str, QToolButton] = {}
        for key, icon, tip in (
            ("goIn", "goIn", N_("Go to In Point")),
            ("goOut", "goOut", N_("Go to Out Point")),
            ("toggle", "play", N_("Play-Stop Toggle (Space)")),
            ("inToOut", "play", N_("Play In to Out")),
            ("loop", "loop", N_("Loop")),
        ):
            b = QToolButton(self)
            b.setObjectName(f"mixer.transport.{key}")
            b.setIcon(transport_icon(icon))
            b.setIconSize(QSize(14, 14))
            b.setAutoRaise(True)
            b.setToolTip(tr(tip))
            b.setFocusPolicy(Qt.FocusPolicy.NoFocus)
            b.clicked.connect(lambda _c=False, k=key: self.transport.emit(k))
            footer.addWidget(b)
            self.buttons[key] = b
        rec = QToolButton(self)
        rec.setObjectName("mixer.transport.voiceover.recordToggle")
        rec.setIcon(record_icon(False))
        rec.setIconSize(QSize(14, 14))
        rec.setAutoRaise(True)
        rec.setCheckable(True)
        rec.setToolTip(tr("Record (voice-over on the record-enabled track)"))
        rec.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        rec.clicked.connect(lambda _c=False: (rec.setChecked(not rec.isChecked()), self.transport.emit("voiceover.recordToggle")))
        footer.addWidget(rec)
        self.buttons["voiceover.recordToggle"] = rec
        footer.addStretch(1)
        footer.addWidget(self.duration_label)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(0)
        layout.addLayout(body, 1)
        layout.addLayout(footer)
        self._restyle()

    def _restyle(self) -> None:
        T = theme.current()
        self.time_label.setStyleSheet(f"color: {T.timecode}; font-family: monospace; font-size: 13px;")
        self.duration_label.setStyleSheet(f"color: {T.text_dim}; font-family: monospace; font-size: 12px;")

    def changeEvent(self, e) -> None:
        super().changeEvent(e)
        if e.type() == e.Type.PaletteChange and hasattr(self, "duration_label"):
            self._restyle()

    # ----- data -----
    def strip_name(self, ref: str) -> str:
        return next((str(s.get("name")) for s in self.strips_data if s.get("ref") == ref), ref)

    def targets(self, strip: dict[str, Any]) -> list[dict[str, Any]]:
        """Submixes a strip may route to (all for tracks; only later ones for submixes)."""
        subs = [s for s in self.strips_data if s.get("kind") == "submix"]
        if strip.get("kind") == "submix":
            i = next((k for k, s in enumerate(subs) if s.get("id") == strip.get("id")), -1)
            return subs[i + 1 :]
        return subs

    def set_effects(self, effects: Any) -> None:
        self.audio_effects = [e for e in effects or [] if isinstance(e, dict) and e.get("kind") == "Audio" and str(e.get("path") or "").startswith("Audio Effects")]

    def set_state(self, result: Any, sequence: dict[str, Any] | None, playhead: int) -> None:
        self.sequence = sequence
        self.now = playhead
        data = result if isinstance(result, dict) else {}
        self.recording = bool(data.get("recording"))
        self.strips_data = [s for s in data.get("strips") or [] if isinstance(s, dict)]
        shown = [s for s in self.strips_data if str(s.get("ref")) not in self.hidden]
        while len(self.strips) < len(shown):
            st = Strip(self)
            st.command.connect(self.command.emit)
            self.strips_layout.insertWidget(len(self.strips), st)
            self.strips.append(st)
        for i, st in enumerate(self.strips):
            if i < len(shown):
                st.set_data(shown[i])
                st.show()
            else:
                st.hide()
        need = int(max((st.needed_height() for st in self.strips[: len(shown)]), default=0))
        self.strips_box.setMinimumHeight(need)
        ft = frame_ticks((sequence or {}).get("settings"))
        self.time_label.setText(format_timecode(playhead, ft))
        self.duration_label.setText(format_timecode(int((sequence or {}).get("duration") or 0), ft))

    def set_levels(self, peaks: dict[Any, list[float]], dt: float) -> None:
        """egui's ballistics on each strip: instant attack, 20 dB/s fall, holds 6 dB/s."""
        if getattr(self, "meter_input_only", False):
            # armed tracks show the recording input; other meters are idle
            peaks = {
                st.sid(): [self.input_level] * max(1, st.channels())
                for st in self.strips
                if isinstance(st, Strip) and st.data.get("recordArm") and st.data.get("kind") == "audio"
            }
        for st in self.strips:
            if not st.isVisible():
                continue
            fresh = peaks.get(st.sid())
            n = len(fresh) if fresh else (len(st.levels) or (st.channels() if isinstance(st, Strip) else int(st.data.get("channels") or 2)))
            if len(st.levels) != n:
                st.levels, st.holds = [-90.0] * n, [-90.0] * n
            changed = False
            for c in range(n):
                lv, hd = st.levels[c], st.holds[c]
                st.levels[c] = max(-90.0, lv - 20.0 * dt)
                st.holds[c] = max(-90.0, hd - 6.0 * dt)
                if fresh and c < len(fresh):
                    db = to_db(fresh[c])
                    st.levels[c] = max(st.levels[c], db)
                    st.holds[c] = max(st.holds[c], db)
                changed |= (lv, hd) != (st.levels[c], st.holds[c])
            if changed:
                st.update()

    # ----- side buttons -----
    def toggle_fx(self) -> None:
        self.fx_open = not self.fx_open
        self.fx_button.setText("▾" if self.fx_open else "▸")
        for st in self.strips:
            st.update()
        self.strips_box.setMinimumHeight(int(max((st.needed_height() for st in self.strips if st.isVisible()), default=0)))

    def _fill_menu(self) -> None:
        m = self.menu_button.menu()
        m.clear()
        sub = m.addMenu(tr("Show/Hide Tracks"))
        sub.setObjectName("mixer.menu.showHide")
        for s in self.strips_data:
            if s.get("kind") == "mix":
                continue
            ref = str(s.get("ref"))
            a = sub.addAction(f"{ref}  {s.get('name')}")
            a.setObjectName(f"mixer.showHide.{ref}")
            a.setCheckable(True)
            a.setChecked(ref not in self.hidden)
            a.toggled.connect(lambda on, r=ref: self.set_hidden(r, not on))
        sub.addSeparator()
        a = sub.addAction(tr("Show All"))
        a.setObjectName("mixer.showHide.all")
        a.triggered.connect(self.show_all)
        meter = m.addAction(tr("Meter Input(s) Only"))
        meter.setObjectName("mixer.menu.meterInputOnly")
        meter.setCheckable(True)
        meter.setChecked(self.meter_input_only)
        meter.toggled.connect(self.set_meter_input_only)

    def set_meter_input_only(self, on: bool) -> None:
        """Meter Input(s) Only (egui's): armed tracks meter the recording input, the rest is idle."""
        self.meter_input_only = bool(on)

    def set_voiceover(self, recording: bool) -> None:
        """The record button shows a take in progress."""
        b = self.buttons.get("voiceover.recordToggle")
        if b is not None:
            b.setIcon(record_icon(recording))
            b.setChecked(recording)

    def set_hidden(self, ref: str, hidden: bool) -> None:
        (self.hidden.add if hidden else self.hidden.discard)(ref)
        self.set_state({"strips": self.strips_data, "recording": self.recording}, self.sequence, self.now)

    def show_all(self) -> None:
        self.hidden.clear()
        self.set_state({"strips": self.strips_data, "recording": self.recording}, self.sequence, self.now)





CLIP_STRIP_MIN_W = 66


class ClipStrip(QWidget):
    """One Audio Clip Mixer strip: the clip under the playhead on one audio track."""

    command = Signal(str, dict)

    def __init__(self, panel: ClipMixerPanel) -> None:
        super().__init__(panel.box)
        self.panel = panel
        self.data: dict[str, Any] = {}
        self.levels: list[float] = []
        self.holds: list[float] = []
        self.write_keyframes = False
        self._drag: str | None = None
        self._last = QPointF()
        self._value = 0.0
        self.regions: dict[str, QRectF] = {}
        self.hot_volume = hot(self, FADER_MIN_DB, FADER_MAX_DB, 0.1)
        self.hot_volume.edited.connect(self._typed_volume)
        self.hot_volume.released.connect(lambda: self._release("volume"))
        self.setMinimumWidth(CLIP_STRIP_MIN_W)
        self.setMaximumWidth(STRIP_W)

    @property
    def ref(self) -> str:
        return str(self.data.get("ref") or "")

    def sid(self) -> Any:
        return self.data.get("track")

    def clip(self) -> dict[str, Any] | None:
        c = self.data.get("clip")
        return c if isinstance(c, dict) else None

    def volume(self) -> float:
        return self._value if self._drag == "fader" else float((self.clip() or {}).get("volumeDb") or 0.0)

    def pan(self) -> float:
        return self._value if self._drag == "pan" else float((self.clip() or {}).get("pan") or 0.0)

    def set_data(self, data: dict[str, Any]) -> None:
        self.data = data
        self.setObjectName(f"clipMixer.{self.ref}")
        self.hot_volume.setObjectName(f"clipMixer.{self.ref}.value")
        self.update()

    def layout_regions(self) -> dict[str, QRectF]:
        w, h = self.width(), self.height()
        r: dict[str, QRectF] = {"mode": QRectF(4, 2, w - 8, 16)}
        r["pan"] = QRectF(w / 2 - 15, 21, 30, 30)
        y = 72.0
        step = min(24.0, (w - 6 - 18) / 2)
        for k, key in enumerate(("mute", "solo", "keyframe")):
            r[key] = QRectF(w / 2 - step - 9 + k * step, y, 18, 16)
        r["area"] = QRectF(0, y + 30, w, max(0.0, h - 46 - (y + 30)))
        r["track"] = QRectF(33, r["area"].top() + 8, 4, max(0.0, r["area"].height() - 16))
        r["meter"] = QRectF(46, r["area"].top(), max(4.0, w - 52), r["area"].height())
        r["value"] = QRectF(4, h - 42, 50, 16)
        r["name"] = QRectF(0, h - 20, w, 20)
        return r

    def fader_y(self, db: float) -> float:
        t = self.regions["track"]
        return t.bottom() - db_to_pos(db) * t.height()

    def paintEvent(self, _e: QPaintEvent) -> None:
        self.regions = r = self.layout_regions()
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        T = theme.current()
        active = self.clip() is not None
        place(self.hot_volume, r["value"], self.volume())
        self.hot_volume.setEnabled(active)

        def dim(c: QColor) -> QColor:
            if active:
                return c
            c = QColor(c)
            c.setAlphaF(0.4)
            return c

        p.fillRect(self.rect(), T.c("control_surface"))
        f = QFont(self.font())
        f.setPixelSize(10)
        p.setFont(f)
        p.setPen(QPen(T.c("field_border"), 1))
        p.setBrush(T.c("field_bg"))
        p.drawRoundedRect(r["mode"].adjusted(0.5, 0.5, -0.5, -0.5), 2, 2)
        p.setPen(T.c("text"))
        p.drawText(r["mode"].adjusted(4, 0, -4, 0), Qt.AlignmentFlag.AlignVCenter, tr(str(self.data.get("mode") or "Read")))
        kc = r["pan"].center()
        p.setPen(QPen(dim(T.c("text_dim")), 2))
        p.setBrush(Qt.BrushStyle.NoBrush)
        p.drawEllipse(kc, 14, 14)
        ang = self.pan() / 100.0 * math.radians(135)
        p.drawLine(kc, kc + QPointF(math.sin(ang) * 12, -math.cos(ang) * 12))
        p.setPen(dim(T.c("hot_text")))
        p.drawText(QRectF(kc.x() - 26, kc.y() + 9, 16, 14), Qt.AlignmentFlag.AlignCenter, "L")
        p.drawText(QRectF(kc.x() + 10, kc.y() + 9, 16, 14), Qt.AlignmentFlag.AlignCenter, "R")
        p.drawText(QRectF(kc.x() - 28, kc.y() + 19, 56, 14), Qt.AlignmentFlag.AlignCenter, f"{self.pan():.1f}")
        for key, letter, on, color in (
            ("mute", "M", bool(self.data.get("muted")), MUTE_ON),
            ("solo", "S", bool(self.data.get("solo")), SOLO_ON),
            ("keyframe", "◇", self.write_keyframes, T.c("accent")),
        ):
            p.setPen(QPen(T.c("button_border"), 1))
            p.setBrush(color if on else T.c("field_bg"))
            p.drawRoundedRect(r[key], 3, 3)
            p.setPen(QColor("#111111") if on and key != "keyframe" else (QColor("#ffffff") if on else T.c("text_dim")))
            p.drawText(r[key], Qt.AlignmentFlag.AlignCenter, letter)
        area, track = r["area"], r["track"]
        if area.height() > 40:
            f.setPixelSize(9)
            p.setFont(f)
            for d in SCALE:
                y = self.fader_y(d)
                p.setPen(T.c("text_dim"))
                p.drawText(QRectF(0, y - 6, 26, 12), Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter, "-∞" if d <= FADER_MIN_DB else str(int(d)))
            p.setPen(Qt.PenStyle.NoPen)
            p.setBrush(QColor(0x10, 0x10, 0x10) if T.dark else QColor(0xC4, 0xC4, 0xC4))
            p.drawRoundedRect(track, 2, 2)
            cy = self.fader_y(self.volume())
            cap = QRectF(track.center().x() - 9, cy - 13, 18, 26)
            r["cap"] = cap
            col = dim(T.c("accent") if self._drag == "fader" else (QColor(0xE0, 0xE0, 0xE0) if T.dark else QColor(0x32, 0x32, 0x32)))
            p.setPen(QPen(col, 2))
            p.setBrush(QColor(0x2A, 0x2A, 0x2A) if T.dark else QColor(0xCD, 0xCD, 0xCD))
            p.drawRoundedRect(cap.adjusted(1, 1, -1, -1), 3, 3)
            p.drawLine(QPointF(cap.left() + 5, cy), QPointF(cap.right() - 5, cy))
            m = r["meter"]
            p.fillRect(m, T.c("meter_bg"))
            n = max(1, len(self.levels) or int(self.data.get("channels") or 2))
            gap = 1.0 if n > 2 else 2.0
            bw = (m.width() - gap * (n - 1)) / n
            for c in range(n):
                lv = self.levels[c] if c < len(self.levels) else -90.0
                hd = self.holds[c] if c < len(self.holds) else -90.0
                br = QRectF(m.left() + c * (bw + gap), m.top() + 6, bw, m.height() - 6)
                AudioMetersPanel._bar(p, br, lv, hd)
                p.fillRect(QRectF(br.left(), m.top(), bw, 4), RED if hd >= -0.1 else (QColor(0x30, 0x30, 0x30) if T.dark else QColor(0x9B, 0x9B, 0x9B)))
        f.setPixelSize(12)
        p.setFont(f)
        p.setPen(T.c("text"))
        p.drawText(r["name"].adjusted(6, 0, 0, 0), Qt.AlignmentFlag.AlignVCenter, self.ref)
        p.drawText(r["name"].adjusted(30, 0, -2, 0), Qt.AlignmentFlag.AlignVCenter, str(self.data.get("name") or ""))

    # ----- input -----
    def _set(self, effect: str, param: str, value: float, begin: bool) -> None:
        """egui's clip mixer move: a live gesture during an automation pass, else the clip's
        Volume / Panner (a keyframe at the playhead with Write Keyframes on)."""
        clip = self.clip()
        if self.panel.recording:
            lane = "pan" if effect == "panner" else "volume"
            self.command.emit("clipMixer.touch", {"track": self.sid(), "lane": lane, "value": value, "time": self.panel.now})
        elif clip is not None:
            self.command.emit(
                "clipMixer.set",
                {
                    "clip": clip.get("id"),
                    "effect": effect,
                    "param": param,
                    "value": value,
                    "keyframe": self.write_keyframes,
                    "begin": begin,
                    "time": self.panel.now,  # the shown playhead: the engine's stays put while playing
                },
            )

    def _typed_volume(self, value: float, begin: bool) -> None:
        """The hot value: a scrub step or a typed value (which ends its gesture at once)."""
        if self.clip() is None:
            return
        self._set("volume", "level", value, begin)
        if begin and not self.hot_volume._scrubbing:
            self._release("volume")

    def _release(self, effect: str) -> None:
        if self.panel.recording:
            lane = "pan" if effect == "panner" else "volume"
            self.command.emit("clipMixer.release", {"track": self.sid(), "lane": lane, "time": self.panel.now})

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() != Qt.MouseButton.LeftButton:
            return
        pos = e.position()
        r = self.regions
        if r.get("cap") is not None and r["cap"].adjusted(-3, -3, 3, 3).contains(pos) and self.clip() is not None:
            self._drag, self._value, self._last = "fader", self.volume(), pos
            self._set("volume", "level", self._value, True)
        elif r.get("pan") is not None and r["pan"].contains(pos) and self.clip() is not None:
            self._drag, self._value, self._last = "pan", self.pan(), pos
            self._set("panner", "balance", self._value, True)
        elif r["mode"].contains(pos):
            menu = QMenu(self)
            for m in MODES:
                a = menu.addAction(tr(m))
                a.setObjectName(f"clipMixer.{self.ref}.mode.{m}")
                a.setCheckable(True)
                a.setChecked(self.data.get("mode") == m)
                a.triggered.connect(lambda _c=False, mm=m: self.command.emit("clipMixer.setMode", {"track": self.sid(), "mode": mm}))
            menu.exec(self.mapToGlobal(r["mode"].bottomLeft().toPoint()))
        elif r["mute"].contains(pos):
            self.command.emit("mixer.setStrip", {"strip": self.sid(), "muted": not bool(self.data.get("muted"))})
        elif r["solo"].contains(pos):
            self.command.emit("mixer.setStrip", {"strip": self.sid(), "solo": not bool(self.data.get("solo"))})
        elif r["keyframe"].contains(pos):
            self.write_keyframes = not self.write_keyframes
            self.update()

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        if self._drag is None:
            r = self.regions
            pos = e.position()
            tip = ""
            if r.get("keyframe") is not None and r["keyframe"].contains(pos):
                tip = tr("Write keyframes")
            elif r.get("mute") is not None and r["mute"].contains(pos):
                tip = tr("Mute Track")
            elif r.get("solo") is not None and r["solo"].contains(pos):
                tip = tr("Solo Track")
            self.setToolTip(tip)
            return
        d = e.position() - self._last
        self._last = e.position()
        if self._drag == "fader":
            self._value = pos_to_db(db_to_pos(self._value) - d.y() / max(1.0, self.regions["track"].height()))
            self._set("volume", "level", self._value, False)
        else:
            self._value = min(100.0, max(-100.0, self._value + d.x() - d.y()))
            self._set("panner", "balance", self._value, False)
        self.update()

    def mouseReleaseEvent(self, _e: QMouseEvent) -> None:
        if self._drag is not None:
            self._release("volume" if self._drag == "fader" else "panner")
            self._drag = None
            self.update()

    def mouseDoubleClickEvent(self, e: QMouseEvent) -> None:
        r = self.regions
        if r.get("cap") is not None and r["cap"].contains(e.position()):
            self._set("volume", "level", 0.0, True)
        elif r.get("pan") is not None and r["pan"].contains(e.position()):
            self._set("panner", "balance", 0.0, True)


class ClipMixerPanel(QWidget):
    """egui's Audio Clip Mixer: one strip per audio track, controlling the clip under the
    playhead (its Volume and Panner). The strips narrow so every track fits, down to the
    narrowest usable strip; below that the row scrolls."""

    command = Signal(str, dict)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.AudioClipMixer")
        self.recording = False
        self.now = 0
        self.box = QWidget()
        self.row = QHBoxLayout(self.box)
        self.row.setContentsMargins(6, 4, 2, 4)
        self.row.setSpacing(2)
        self.row.addStretch(1)
        self.scroll = QScrollArea(self)
        self.scroll.setObjectName("clipMixer.scroll")
        self.scroll.setWidgetResizable(True)
        self.scroll.setFrameShape(QScrollArea.Shape.NoFrame)
        self.scroll.setWidget(self.box)
        self.empty = QLabel(tr("(no sequence)"), self)
        self.empty.setAlignment(Qt.AlignmentFlag.AlignCenter)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.addWidget(self.empty)
        layout.addWidget(self.scroll, 1)
        self.strips: list[ClipStrip] = []
        self.box.setMinimumHeight(260)

    def set_state(self, result: Any) -> None:
        data = result if isinstance(result, dict) else {}
        self.recording = bool(data.get("recording"))
        self.now = int(data.get("time") or 0)
        rows = [s for s in data.get("strips") or [] if isinstance(s, dict)]
        self.empty.setVisible(not rows)
        while len(self.strips) < len(rows):
            st = ClipStrip(self)
            st.command.connect(self.command.emit)
            self.row.insertWidget(len(self.strips), st, 1)
            self.strips.append(st)
        for i, st in enumerate(self.strips):
            if i < len(rows):
                st.set_data(rows[i])
                st.show()
            else:
                st.hide()

    def set_levels(self, peaks: dict[Any, list[float]], dt: float) -> None:
        TrackMixerPanel.set_levels(self, peaks, dt)  # same ballistics, keyed by track id
