"""Audio Meters: a peak meter per channel of what is playing (dB scale, peak hold) and the
BS.1770 / EBU R128 loudness readout, as egui's panel (crates/ui-egui/src/panels/meters.rs).

The levels and the loudness are the engine's: every ``audio.render`` a monitor plays carries the
sample peak of each channel every 1/100 s and the loudness so far (see ``Monitor``). This panel
only picks the blocks being heard by the playback clock and draws them with egui's ballistics:
instant attack, a 20 dB/s fall, and a peak-hold line that falls 6 dB/s.
"""

from __future__ import annotations

import functools
import math
from dataclasses import dataclass, field
from typing import Any

from PySide6.QtCore import QElapsedTimer, QPointF, QRectF, QSizeF, Qt, QTimer
from PySide6.QtGui import QColor, QFont, QFontMetricsF, QPainter, QPaintEvent, QPen
from PySide6.QtWidgets import QSizePolicy, QWidget

from .. import theme
from ..i18n import tr

FLOOR = -90.0
RELEASE_DB_PER_S = 20.0
HOLD_FALL_DB_PER_S = 6.0
SCALE_PX = 11
READOUT_PX = 12

# egui's colours (theme.rs / widgets.rs meter_bar)
TP_OVER = QColor(0xE0, 0x50, 0x40)
RED = QColor(0xE3, 0x48, 0x50)
YELLOW = QColor(0xF0, 0xF0, 0x4F)
DANGER = QColor(0xE3, 0x48, 0x50)

CHANNEL_NAMES = {
    1: ("M",),
    2: ("L", "R"),
    6: ("L", "R", "C", "LFE", "Ls", "Rs"),
}


def channel_names(n: int) -> tuple[str, ...]:
    return CHANNEL_NAMES.get(n) or tuple(str(i + 1) for i in range(max(1, n)))


def to_db(peak: Any) -> float:
    try:
        x = float(peak)
    except (TypeError, ValueError):
        return FLOOR
    return 20.0 * math.log10(max(x, 1e-6))


def lufs_text(v: Any) -> str:
    return f"{v:.1f}" if isinstance(v, (int, float)) and math.isfinite(v) and v > -70.0 else "—"


def scale_font() -> QFont:
    f = QFont()
    f.setPixelSize(SCALE_PX)
    return f


def readout_font(mono: bool = False) -> QFont:
    f = QFont("monospace") if mono else QFont()
    if mono:
        f.setStyleHint(QFont.StyleHint.Monospace)
    f.setPixelSize(READOUT_PX)
    return f


def foot_font(n: int) -> QFont:
    f = QFont()
    f.setPixelSize(13 if n == 2 else SCALE_PX)
    return f


def foot(name: str, n: int) -> str:
    """The text under a bar: an "S" under each bar of a stereo meter, the channel names under
    any other (egui's)."""
    return "S" if n == 2 else name


def _anchored(size: QSizeF, at: QPointF, h: str) -> QRectF:
    """A rect of ``size`` whose right / centre / left (``h``) and vertical centre sit at ``at``."""
    x = {"right": at.x() - size.width(), "center": at.x() - size.width() / 2, "left": at.x()}[h]
    return QRectF(x, at.y() - size.height() / 2, size.width(), size.height())


def _size(text: str, font: QFont) -> QSizeF:
    w, h = _measure(text, font.pixelSize(), font.styleHint() == QFont.StyleHint.Monospace, font.family())
    return QSizeF(w, h)


@functools.lru_cache(maxsize=512)
def _measure(text: str, px: int, mono: bool, family: str) -> tuple[float, float]:
    f = QFont(family)
    if mono:
        f.setStyleHint(QFont.StyleHint.Monospace)
    f.setPixelSize(px)
    fm = QFontMetricsF(f)
    return fm.horizontalAdvance(text), fm.height()


def _overlaps(a: QRectF, b: QRectF) -> bool:
    i = a.intersected(b)
    return i.width() > 0 and i.height() > 0


@dataclass
class Parts:
    area: QRectF
    scale: list[tuple[int, QRectF]] = field(default_factory=list)
    unit: QRectF | None = None
    names: list[QRectF] = field(default_factory=list)
    readout: tuple[QRectF, bool] | None = None


def bar_pitch(area: QRectF, n: int) -> tuple[float, float]:
    n = max(1, n)
    gap = 4.0 if area.width() >= n * 6.0 + 4.0 * (n - 1) else 1.0
    w = max(1.0, (area.width() - gap * (n - 1)) / n)
    return w, w + gap


def parts(rect: QRectF, names: tuple[str, ...]) -> Parts:
    """Where the panel's parts go (egui's ``parts``). What doesn't fit is left out rather than
    drawn over something else: as the panel narrows, the readout drops its units and then goes,
    and the dB scale goes once the bars would get too thin; as it gets shorter, the scale numbers
    thin out to every 6, 12 or 24 dB and then go, and the names under the bars go."""
    n = max(1, len(names))
    readout: bool | None = None
    if rect.height() > 260.0:
        room = rect.width() - 20.0
        label = _size("TP", readout_font()).width()

        def fits(value: str) -> bool:
            return label + 6.0 + _size(value, readout_font(True)).width() <= room

        readout = True if fits("-88.8 dBTP") else (False if fits("-88.8") else None)
    lufs_h = 64.0 if readout is not None else 0.0
    feet = rect.height() >= 90.0
    top = rect.top() + 8.0
    bottom = max(top, rect.bottom() - lufs_h - (30.0 if feet else 8.0))
    h = bottom - top
    widest = _size("-57", scale_font())
    column = max(widest.width() + 10.0, 26.0)
    step = None
    if rect.width() - 8.0 - column >= n * 6.0 + 4.0 * (n - 1):
        step = next((s for s in (3, 6, 12, 24) if h * s / 60.0 >= widest.height()), None)
    right = rect.right() - column if step else rect.right() - 8.0
    area = QRectF(QPointF(rect.left() + 8.0, top), QPointF(max(right, rect.left() + 8.0), bottom))
    scale: list[tuple[int, QRectF]] = []
    if step:
        for k in range(57 // step + 1):
            db = -(k * step)
            y = area.top() + h * (k * step) / 60.0
            scale.append((db, _anchored(_size(str(db), scale_font()), QPointF(rect.right() - 6.0, y), "right")))
    w, pitch = bar_pitch(area, n)
    feet_names = [
        _anchored(_size(foot(name, n), foot_font(n)), QPointF(area.left() + c * pitch + w / 2.0, area.bottom() + 16.0), "center")
        for c, name in enumerate(names)
    ]
    name_rects = feet_names if feet and all(r.width() < pitch for r in feet_names) else []
    unit = None
    if feet and step:
        u = _anchored(_size("dB", scale_font()), QPointF(rect.right() - 6.0, area.bottom() + 10.0), "right")
        if not any(_overlaps(u, r) for r in [*name_rects, *(r for _, r in scale)]):
            unit = u
    ro = None
    if readout is not None:
        ro = (QRectF(QPointF(rect.left() + 4.0, rect.bottom() - lufs_h), QPointF(rect.right() - 4.0, rect.bottom() - 4.0)), readout)
    return Parts(area, scale, unit, name_rects, ro)


def readout_rows(r: QRectF, units: bool) -> list[tuple[str, QPointF, QPointF]]:
    """The four readout rows: label and where its left text and right value sit."""
    return [(label, QPointF(r.left() + 6.0, r.top() + 8.0 + k * 14.0), QPointF(r.right() - 6.0, r.top() + 8.0 + k * 14.0)) for k, label in enumerate(("M", "S", "I", "TP"))]


class AudioMetersPanel(QWidget):
    """Narrow panel right of the Timeline, as egui's."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.AudioMeters")
        self.setSizePolicy(QSizePolicy.Policy.Preferred, QSizePolicy.Policy.Expanding)
        self.setMinimumWidth(24)
        self.monitors: list[Any] = []
        self.levels: list[float] = [FLOOR, FLOOR]  # dB, per channel
        self.holds: list[float] = [FLOOR, FLOOR]
        self.loudness: list[Any] = [None, None, None, None]  # M, S, I LUFS, TP dBTP
        self._heard: tuple[Any, int | None] | None = None  # (monitor, end of the blocks shown)
        self._clock = QElapsedTimer()
        self._clock.start()
        self._timer = QTimer(self)
        self._timer.setInterval(16)
        self._timer.timeout.connect(self.tick)
        self._timer.start()
        self.setToolTip(tr("Audio Meters"))

    def watch(self, *monitors: Any) -> None:
        self.monitors = list(monitors)

    # ----- levels -----
    def tick(self) -> None:
        dt = min(0.1, self._clock.restart() / 1000.0)
        changed = False
        for i in range(len(self.levels)):
            lv, hd = self.levels[i], self.holds[i]
            self.levels[i] = max(FLOOR, lv - RELEASE_DB_PER_S * dt)
            self.holds[i] = max(FLOOR, hd - HOLD_FALL_DB_PER_S * dt)
            changed |= (lv, hd) != (self.levels[i], self.holds[i])
        mon = next((m for m in self.monitors if getattr(m, "playing", False)), None)
        if mon is not None:
            now = mon.play_sample()
            consumed = self._heard[1] if self._heard and self._heard[0] is mon else None
            peaks, loud, consumed = self.heard(mon.meter_blocks, consumed, now)
            if peaks:
                if len(peaks) != len(self.levels):
                    self.levels = [FLOOR] * len(peaks)
                    self.holds = [FLOOR] * len(peaks)
                for i, p in enumerate(peaks):
                    db = to_db(p)
                    self.levels[i] = max(self.levels[i], db)
                    self.holds[i] = max(self.holds[i], db)
                changed = True
            if loud is not None and loud != self.loudness:
                self.loudness = loud
                changed = True
            self._heard = (mon, consumed)
        else:
            self._heard = None
        if changed:
            self.update()
            self._update_description()

    @staticmethod
    def heard(blocks: Any, consumed: int | None, now: int) -> tuple[list[float], list[Any] | None, int | None]:
        """The highest peak of each channel in the blocks heard by ``now`` that were not shown
        yet (ending after ``consumed``; a block that arrives late still shows), the loudness at
        ``now``, and the new ``consumed``."""
        peaks: list[float] = []
        loud = None
        for start, block, bpeaks, bloud in blocks:
            for k, values in enumerate(bpeaks):
                b0 = start + k * block
                if b0 > now:
                    break
                b1 = b0 + block
                if consumed is None or b1 > consumed:
                    if len(peaks) < len(values):
                        peaks.extend([0.0] * (len(values) - len(peaks)))
                    for i, v in enumerate(values):
                        peaks[i] = max(peaks[i], float(v or 0.0))
                    consumed = b1 if consumed is None else max(consumed, b1)
                if k < len(bloud) and isinstance(bloud[k], list):
                    loud = list(bloud[k])
        return peaks, loud, consumed

    def _update_description(self) -> None:
        names = channel_names(len(self.levels))
        m, s, i, tp = (self.loudness + [None] * 4)[:4]
        chans = ", ".join(f"{n} {lv:.1f} dB" for n, lv in zip(names, self.levels, strict=False))
        self.setAccessibleDescription(f"{chans}; M {lufs_text(m)} S {lufs_text(s)} I {lufs_text(i)} LUFS TP {lufs_text(tp)} dBTP")

    # ----- drawing -----
    def paintEvent(self, e: QPaintEvent) -> None:
        p = QPainter(self)
        T = theme.current()
        text_color, text_dim, hot = T.c("text"), T.c("text_dim"), T.c("hot_text")
        rect = QRectF(self.rect())
        names = channel_names(len(self.levels))
        pt = parts(rect, names)
        area = pt.area
        if pt.readout is not None:
            lr, units = pt.readout
            p.fillRect(lr, T.c("meter_readout_bg"))
            m, s, i, tp = (self.loudness + [None] * 4)[:4]
            values = {"M": m, "S": s, "I": i, "TP": tp}
            over = isinstance(tp, (int, float)) and tp > -1.0
            for label, left, right in readout_rows(lr, units):
                p.setFont(readout_font())
                p.setPen(text_dim)
                size = _size(label, readout_font())
                p.drawText(_anchored(size, left, "left"), Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter, label)
                unit = "" if not units else (" dBTP" if label == "TP" else " LUFS")
                text = lufs_text(values[label]) + unit
                p.setFont(readout_font(True))
                p.setPen(TP_OVER if label == "TP" and over else hot if label == "I" else text_color)
                vs = _size(text, readout_font(True))
                p.drawText(_anchored(vs, right, "right"), Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter, text)
        p.fillRect(QRectF(QPointF(rect.left() + 4.0, rect.top() + 4.0), QPointF(rect.right() - 4.0, area.bottom() + 4.0)), T.c("meter_bg"))
        w, pitch = bar_pitch(area, len(names))
        for c in range(len(names)):
            r = QRectF(area.left() + c * pitch, area.top(), w, area.height())
            self._bar(p, r, self.levels[c], self.holds[c])
        p.setPen(text_dim)
        p.setFont(scale_font())
        for db, r in pt.scale:
            p.drawText(r, Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter, str(db))
        if pt.unit is not None:
            p.drawText(pt.unit, Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter, "dB")
        p.setFont(foot_font(len(names)))
        for r, name in zip(pt.names, names, strict=False):
            p.drawText(r, Qt.AlignmentFlag.AlignCenter, foot(name, len(names)))
        p.end()

    @staticmethod
    def _bar(p: QPainter, rect: QRectF, db: float, peak_db: float) -> None:
        """egui's meter_bar: green gradient (#579f51 → #70dc5d), yellow above −12 dB, red above
        −3 dB, and the peak-hold line (red near 0 dB)."""

        def norm(d: float) -> float:
            return min(1.0, max(0.0, (d + 60.0) / 60.0))

        h = rect.height() * norm(db)
        if h > 0:
            n = 40
            for i in range(n):
                f0, f1 = i / n, (i + 1) / n
                if f0 * rect.height() > h:
                    break
                y1 = rect.bottom() - f0 * rect.height()
                y0 = rect.bottom() - min(f1 * rect.height(), h)
                here = -60.0 + f1 * 60.0
                if here > -3.0:
                    c = RED
                elif here > -12.0:
                    c = YELLOW
                else:
                    k = f1 / 0.8

                    def lerp(a: int, b: int, k: float = k) -> int:
                        return int(a + (b - a) * k)

                    c = QColor(lerp(0x57, 0x70), lerp(0x9F, 0xDC), lerp(0x51, 0x5D))
                p.fillRect(QRectF(QPointF(rect.left(), y0), QPointF(rect.right(), y1)), c)
        py = min(rect.bottom() - 1.0, rect.bottom() - rect.height() * norm(peak_db))
        p.setPen(QPen(DANGER if peak_db > -0.5 else YELLOW, 1.0))
        p.drawLine(QPointF(rect.left(), py), QPointF(rect.right(), py))

