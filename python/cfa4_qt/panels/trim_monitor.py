"""The Trim Monitor (egui's ``trim_monitor.rs``): while edit points are selected (trim mode) the
Program monitor shows the outgoing clip's last frame and the incoming clip's first frame side by
side, with their timecodes, the Out/In shift counters and the trim buttons (−N, −1, Apply Default
Transitions, +1, +N; N = Preferences ▸ Trim ▸ Large Trim Offset). Dragging in the outgoing picture
trims the outgoing side, in the incoming picture the incoming side, between them rolls (6 px a
frame, one trim on release; Escape abandons the drag).

Dynamic trimming: in trim mode J/L/K, Shift+J/L, Space and Shift+K drive the engine's ``trim.*``
state machine (``route``), advanced here with ``trim.tick`` while it runs; Escape cancels a
dynamic trim. Everything the monitor shows comes from the engine's ``trim.monitor``.

Automation ids as egui's: ``trimMonitor.outgoing``, ``.incoming``, ``.roll``, ``.outTimecode``,
``.inTimecode``, ``.outShift``, ``.inShift``, ``.backwardMany``, ``.backward``,
``.applyTransition``, ``.forward``, ``.forwardMany``, ``.playAround``, ``.exit``; the monitor
itself is ``trimMonitor``.
"""

from __future__ import annotations

from typing import Any

from PySide6.QtCore import QElapsedTimer, QPointF, QRectF, Qt, QTimer, Signal
from PySide6.QtGui import QColor, QImage, QKeyEvent, QKeySequence, QMouseEvent, QPainter, QPaintEvent, QPen, QShortcut
from PySide6.QtWidgets import QHBoxLayout, QLabel, QSizePolicy, QToolButton, QVBoxLayout, QWidget

from .. import theme
from ..engine import EngineClient
from ..i18n import N_, tr
from ..timecode import drop_frame, format_timecode, frame_ticks
from .transport import icon_color, transport_icon

RIPPLE_RED = QColor(0xE5, 0x3E, 0x3E)
ROLL_YELLOW = QColor(0xF2, 0xC4, 0x3D)
PX_PER_FRAME = 6.0  # a drag in the Trim Monitor: 6 px a frame (egui's)
TICK_MS = 16

# transport command → (trim command, its params); egui's ``route_transport``
_SHUTTLE = {
    "playback.forward": ("trim.shuttle", {"direction": "forward"}),
    "playback.reverse": ("trim.shuttle", {"direction": "reverse"}),
    "playback.slowForward": ("trim.shuttle", {"direction": "forward", "slow": True}),
    "playback.slowReverse": ("trim.shuttle", {"direction": "reverse", "slow": True}),
}


def shift_text(frames: Any) -> str:
    """A shift counter as egui shows it: +3, 0, −2 (ASCII minus)."""
    n = int(frames) if isinstance(frames, int) else 0
    return f"+{n}" if n > 0 else str(n)


def dynamic_text(dynamic: Any) -> str:
    """The dynamic trim indicator: TRIM ■ held, TRIM ▶ 2× forward, TRIM ◀ 1× backward."""
    sp = dynamic.get("speed") if isinstance(dynamic, dict) else None
    sp = float(sp) if isinstance(sp, (int, float)) else 0.0
    if sp == 0.0:
        return "TRIM ■"
    n = abs(sp)
    text = f"{n:g}"
    return f"TRIM ▶ {text}×" if sp > 0 else f"TRIM ◀ {text}×"


def _fit(area: QRectF, w: float, h: float) -> QRectF:
    s = min(area.width() / max(w, 1.0), area.height() / max(h, 1.0))
    r = QRectF(0, 0, w * s, h * s)
    r.moveCenter(area.center())
    return r


class TrimPicture(QWidget):
    """One side's frame (or the roll strip between them): the picture fitted to the sequence's
    shape, outlined in the trim colour when that side moves; a drag reports whole frames."""

    dragged = Signal(str, object)  # which ("out" | "in" | "roll"), frames

    def __init__(self, which: str, name: str, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.which = which
        self.setObjectName(name)
        self.image: QImage | None = None
        self.text = ""  # the message instead of a picture: (gap), ♪ name
        self.hot = False
        self.color = ROLL_YELLOW
        self.aspect = (16.0, 9.0)
        self.frame_size: tuple[float, float] | None = None  # the media's own size
        self._press: QPointF | None = None
        self._dx = 0.0
        self.setCursor(Qt.CursorShape.SizeHorCursor)
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)
        if which == "roll":
            self.setFixedWidth(14)
        else:
            self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Expanding)
            self.setMinimumSize(80, 45)

    def drag_frames(self) -> int:
        return round(self._dx / PX_PER_FRAME)

    def picture_rect(self) -> QRectF:
        if self.which == "roll":
            return QRectF(self.rect())
        return _fit(QRectF(self.rect()), *self.aspect)

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() == Qt.MouseButton.LeftButton:
            self._press = e.position()
            self._dx = 0.0
            self.setFocus(Qt.FocusReason.MouseFocusReason)
        e.accept()

    def mouseMoveEvent(self, e: QMouseEvent) -> None:
        if self._press is not None:
            self._dx = e.position().x() - self._press.x()
            self.update()
        e.accept()

    def mouseReleaseEvent(self, e: QMouseEvent) -> None:
        if self._press is not None and e.button() == Qt.MouseButton.LeftButton:
            n = self.drag_frames()
            self._press = None
            self._dx = 0.0
            self.update()
            if n != 0:
                self.dragged.emit(self.which, n)
        e.accept()

    def keyPressEvent(self, e: QKeyEvent) -> None:
        if e.key() == Qt.Key.Key_Escape and self._press is not None:
            self._press = None  # Escape abandons the drag (egui's #580)
            self._dx = 0.0
            self.update()
            e.accept()
            return
        super().keyPressEvent(e)

    def paintEvent(self, _e: QPaintEvent) -> None:
        p = QPainter(self)
        T = theme.current()
        if self.which != "roll":
            pic = self.picture_rect()
            p.fillRect(pic, QColor("#000000") if self.text == tr("(gap)") else T.c("monitor_bg"))
            if self.image is not None and not self.image.isNull() and not self.text:
                fw, fh = self.frame_size or (float(self.image.width()), float(self.image.height()))
                p.setRenderHint(QPainter.RenderHint.SmoothPixmapTransform)
                p.drawImage(_fit(pic, fw, fh), self.image)
            elif self.text:
                p.setPen(T.c("text_faint" if self.text == tr("(gap)") else "text_dim"))
                p.drawText(pic, Qt.AlignmentFlag.AlignCenter | Qt.TextFlag.TextWordWrap, self.text)
            if self.hot:
                p.setPen(QPen(self.color, 2.0))
                p.setBrush(Qt.BrushStyle.NoBrush)
                p.drawRect(pic.adjusted(-1, -1, 1, 1))
        if self._press is not None:
            f = p.font()
            f.setPixelSize(14)
            f.setBold(True)
            p.setFont(f)
            p.setPen(self.color)
            r = QRectF(0, self.height() - 28, self.width(), 24)
            p.drawText(r, Qt.AlignmentFlag.AlignCenter, shift_text(self.drag_frames()))


class TrimMonitor(QWidget):
    """The two-up Trim Monitor over the Program monitor; ``command`` runs an engine command (the
    window refreshes after it), ``changed`` asks the window to refresh while a dynamic trim runs,
    ``playhead`` reports the playhead while playing around the edit."""

    command = Signal(str, dict)
    changed = Signal()
    playhead = Signal(object)

    def __init__(self, engine: EngineClient, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("trimMonitor")
        self.setAutoFillBackground(True)
        self.engine = engine
        self.info: dict[str, Any] = {}
        self.sequence: dict[str, Any] | None = None
        self.revision = -1
        self._clock = QElapsedTimer()
        self._clock.start()
        self._tick_busy = False
        self._last_change = QElapsedTimer()
        self._last_change.start()
        self._last_offset: Any = None
        self._dirty = False  # the dynamic trim moved since the window last refreshed
        self._timer = QTimer(self)
        self._timer.setInterval(TICK_MS)
        self._timer.timeout.connect(self._tick)
        # side → (key asked, busy)
        self._frame_key: dict[str, tuple[Any, ...] | None] = {"out": None, "in": None}
        self._frame_busy: dict[str, bool] = {"out": False, "in": False}

        lay = QVBoxLayout(self)
        lay.setContentsMargins(6, 4, 6, 4)
        lay.setSpacing(2)
        head = QHBoxLayout()
        self.out_name = QLabel("", self)
        self.out_name.setObjectName("trimMonitor.outName")
        self.dynamic = QLabel("", self)
        self.dynamic.setObjectName("trimMonitor.dynamic")
        self.dynamic.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.in_name = QLabel("", self)
        self.in_name.setObjectName("trimMonitor.inName")
        head.addWidget(self.out_name, 1)
        head.addWidget(self.dynamic)
        head.addWidget(self.in_name, 1)
        lay.addLayout(head)
        pics = QHBoxLayout()
        pics.setSpacing(0)
        self.outgoing = TrimPicture("out", "trimMonitor.outgoing", self)
        self.outgoing.setToolTip(tr("Outgoing clip (drag to trim the Out point)"))
        self.roll = TrimPicture("roll", "trimMonitor.roll", self)
        self.roll.setToolTip(tr("Drag to roll the edit"))
        self.incoming = TrimPicture("in", "trimMonitor.incoming", self)
        self.incoming.setToolTip(tr("Incoming clip (drag to trim the In point)"))
        for w in (self.outgoing, self.roll, self.incoming):
            w.dragged.connect(self.drag_trim)
            pics.addWidget(w, 0 if w is self.roll else 1)
        lay.addLayout(pics, 1)

        counters = QHBoxLayout()
        counters.setContentsMargins(4, 0, 4, 0)

        def counter(name: str) -> QLabel:
            label = QLabel("", self)
            label.setObjectName(f"trimMonitor.{name}")
            f = label.font()
            f.setPixelSize(14)
            f.setBold(True)
            label.setFont(f)
            return label

        self.out_tc = counter("outTimecode")
        self.out_shift = counter("outShift")
        self.in_shift = counter("inShift")
        self.in_tc = counter("inTimecode")
        counters.addWidget(self.out_tc)
        counters.addStretch(1)
        counters.addWidget(self.out_shift)
        counters.addSpacing(20)
        counters.addWidget(self.in_shift)
        counters.addStretch(1)
        counters.addWidget(self.in_tc)
        lay.addLayout(counters)

        bar = QHBoxLayout()
        bar.setSpacing(4)
        bar.addStretch(1)
        self.buttons: dict[str, QToolButton] = {}
        for key, tip, width in (
            ("playAround", N_("Play Around Edit (Space loops, Shift+K once)"), 30),
            ("backwardMany", N_("Trim Backward Many"), 40),
            ("backward", N_("Trim Backward"), 40),
            ("applyTransition", N_("Apply Default Transitions to Selection (Shift+D)"), 176),
            ("forward", N_("Trim Forward"), 40),
            ("forwardMany", N_("Trim Forward Many"), 40),
        ):
            b = QToolButton(self)
            b.setObjectName(f"trimMonitor.{key}")
            b.setToolTip(tr(tip))
            b.setMinimumWidth(width)
            b.setFocusPolicy(Qt.FocusPolicy.NoFocus)
            b.clicked.connect(lambda _c=False, k=key: self.press(k))
            bar.addWidget(b)
            self.buttons[key] = b
        self.buttons["backward"].setText("−1")
        self.buttons["forward"].setText("+1")
        self.buttons["applyTransition"].setText(tr("Apply Default Transitions"))
        bar.addStretch(1)
        self.exit_button = QToolButton(self)
        self.exit_button.setObjectName("trimMonitor.exit")
        self.exit_button.setText("✕")
        self.exit_button.setToolTip(tr("Exit Trim Mode"))
        self.exit_button.setAutoRaise(True)
        self.exit_button.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        self.exit_button.clicked.connect(lambda: self.command.emit("trim.clear", {}))
        bar.addWidget(self.exit_button)
        lay.addLayout(bar)
        # Escape cancels a dynamic trim (only then: it does not take Escape from anything else)
        self.escape = QShortcut(QKeySequence(Qt.Key.Key_Escape), self, context=Qt.ShortcutContext.WindowShortcut)
        self.escape.setEnabled(False)
        self.escape.activated.connect(self.cancel_dynamic)
        self.hide()
        self._show_info()

    # ----- state -----
    def clock(self) -> float:
        """The UI clock (seconds) the engine's trim state machine runs on."""
        return self._clock.nsecsElapsed() / 1e9

    @property
    def playing(self) -> bool:
        """Dynamic trimming or playing around the edit."""
        return bool(self.info.get("dynamic") or self.info.get("playAround"))

    def set_sequence(self, seq: dict[str, Any] | None, revision: int) -> None:
        """Shown while the active sequence has edit points (egui's Program panel in trim mode)."""
        self.sequence = seq
        self.revision = revision
        active = bool(seq and [e for e in seq.get("editPoints") or [] if isinstance(e, dict)])
        self.setVisible(active)
        if active:
            parent = self.parentWidget()
            if parent is not None:
                self.setGeometry(parent.rect())
            self.raise_()
            self.refresh()
        else:
            self.info = {}
            self._timer.stop()
            self.escape.setEnabled(False)
            self._show_info()

    def refresh(self) -> None:
        """Ask the engine for ``trim.monitor`` (after an edit, or a step of dynamic trimming)."""
        if not self.engine.running:
            return
        self.engine.execute("trim.monitor", {}, on_reply=lambda r, _b: self.set_info(r), on_error=lambda _m: None)

    def set_info(self, info: Any) -> None:
        self.info = info if isinstance(info, dict) and info.get("active") else {}
        self.escape.setEnabled(isinstance(self.info.get("dynamic"), dict))
        self._show_info()
        if self.playing and not self._timer.isActive():
            self._timer.start()

    def _settings(self) -> dict[str, Any]:
        return (self.sequence or {}).get("settings") or {}

    def _show_info(self) -> None:
        info = self.info
        kind = info.get("kind") or "trim"
        out_edge = info.get("edge") == "out"
        hot_out, hot_in = (True, True) if kind == "roll" else (out_edge, not out_edge)
        col = RIPPLE_RED if kind == "ripple" else ROLL_YELLOW
        frame, drop = frame_ticks(self._settings()), drop_frame(self._settings())
        T = theme.current()
        sw, sh = self._settings().get("width"), self._settings().get("height")
        aspect = (float(sw), float(sh)) if isinstance(sw, int) and isinstance(sh, int) and sw > 0 and sh > 0 else (16.0, 9.0)
        for side, pic, hot, label, fmt in (
            (info.get("outgoing"), self.outgoing, hot_out, self.out_name, N_("Out: {name}")),
            (info.get("incoming"), self.incoming, hot_in, self.in_name, N_("In: {name}")),
        ):
            pic.hot, pic.color = hot and bool(info), col
            pic.aspect = aspect
            if isinstance(side, dict):
                name = f"{side.get('name') or ''} ({side.get('track') or ''})"
                pic.text = "" if side.get("video") else f"♪  {side.get('name') or ''}"
            else:
                name = tr("(gap)")
                pic.text = tr("(gap)")
                pic.image = None
            label.setText(tr(fmt).format(name=name) if info else "")
            pic.update()
        self.roll.color = col
        dyn = info.get("dynamic")
        self.dynamic.setText(dynamic_text(dyn) if isinstance(dyn, dict) else "")
        self.dynamic.setStyleSheet(f"color: {col.name()}; font-weight: 600;")

        def tc(side: Any) -> str:
            t = side.get("time") if isinstance(side, dict) else None
            return format_timecode(t, frame, drop) if isinstance(t, int) else "--:--:--:--"

        self.out_tc.setText(tc(info.get("outgoing")))
        self.in_tc.setText(tc(info.get("incoming")))
        self.out_shift.setText(shift_text(info.get("outShift")))
        self.in_shift.setText(shift_text(info.get("inShift")))
        hot_text = T.c("hot_text").name()
        dim = T.c("text_dim").name()
        self.out_tc.setStyleSheet(f"color: {hot_text};")
        self.in_tc.setStyleSheet(f"color: {hot_text};")
        self.out_shift.setStyleSheet(f"color: {col.name() if hot_out else dim};")
        self.in_shift.setStyleSheet(f"color: {col.name() if hot_in else dim};")
        n = info.get("largeTrimOffset")
        n = n if isinstance(n, int) else 5
        self.buttons["backwardMany"].setText(f"−{n}")
        self.buttons["forwardMany"].setText(f"+{n}")
        self.buttons["playAround"].setIcon(transport_icon("pause" if info.get("playAround") else "loop", icon_color(self)))
        self._request_frames()

    # ----- pictures -----
    def _request_frames(self) -> None:
        for which, pic, side in (("out", self.outgoing, self.info.get("outgoing")), ("in", self.incoming, self.info.get("incoming"))):
            if not isinstance(side, dict) or not side.get("video") or not isinstance(side.get("item"), int):
                continue
            t = side.get("mediaTime")
            if not isinstance(t, int):
                continue
            r = pic.picture_rect()
            w, h = max(16, int(r.width())), max(9, int(r.height()))
            key = (side["item"], t, w, h, self.revision)
            if key == self._frame_key[which] or self._frame_busy[which] or not self.engine.running:
                continue
            self._frame_busy[which] = True
            self._frame_key[which] = key
            params = {"target": "source", "item": side["item"], "time": t, "maxWidth": w, "maxHeight": h}
            self.engine.call(
                "frame.render",
                params,
                on_reply=lambda res, data, wh=which: self._on_frame(wh, res, data),
                on_error=lambda m, wh=which: self._on_frame_error(wh, m),
            )

    def _on_frame(self, which: str, result: Any, data: bytes | None) -> None:
        self._frame_busy[which] = False
        pic = self.outgoing if which == "out" else self.incoming
        if isinstance(result, dict) and data:
            w, h = int(result.get("w") or 0), int(result.get("h") or 0)
            if w > 0 and h > 0 and len(data) == w * h * 4:
                pic.image = QImage(data, w, h, w * 4, QImage.Format.Format_RGBA8888).copy()
                fw, fh = result.get("fullW"), result.get("fullH")
                par = result.get("pixelAspect")
                par = float(par) if isinstance(par, (int, float)) and par > 0 else 1.0
                pic.frame_size = (float(fw) * par, float(fh)) if isinstance(fw, int) and isinstance(fh, int) and fw > 0 and fh > 0 else None
                pic.update()
        self._request_frames()

    def _on_frame_error(self, which: str, _message: str) -> None:
        self._frame_busy[which] = False
        pic = self.outgoing if which == "out" else self.incoming
        pic.image = None
        pic.update()

    def resizeEvent(self, e) -> None:
        super().resizeEvent(e)
        self._request_frames()

    # ----- trimming -----
    def press(self, key: str) -> None:
        """A trim button (egui's)."""
        if key == "playAround":
            self.run("trim.playAround", {"clock": self.clock(), "toggle": True})
        elif key == "applyTransition":
            self.command.emit("trim.applyDefaultTransition", {})
        else:
            self.command.emit(f"trim.{key}", {})

    def drag_trim(self, which: str, frames: int) -> None:
        """A drag in the Trim Monitor: re-target the primary edit point to the dragged side (a
        roll between them), then trim by ``frames`` (egui's ``drag_trim``)."""
        eps = [e for e in (self.sequence or {}).get("editPoints") or [] if isinstance(e, dict)]
        if not eps or not self.info:
            return
        ep = eps[0]
        out_side, in_side = self.info.get("outgoing"), self.info.get("incoming")
        out_c = out_side.get("clip") if isinstance(out_side, dict) else None
        in_c = in_side.get("clip") if isinstance(in_side, dict) else None
        kind = ep.get("kind") or "trim"
        if which in ("out", "in"):
            clip = out_c if which == "out" else in_c
            target = (clip, which, "ripple" if kind == "roll" else kind) if clip is not None else None
        else:
            clip = out_c if out_c is not None else in_c
            target = (clip, "out" if out_c is not None else "in", "roll") if clip is not None else None
        if target is None:
            return
        clip, edge, kind2 = target
        same = clip == ep.get("clip") and (edge == "out") == bool(ep.get("out")) and kind2 == kind
        if len(eps) == 1 and not same:
            self.command.emit("trim.selectEditPoint", {"clip": clip, "edge": edge, "kind": kind2})
        self.command.emit("trim.nudge", {"frames": int(frames)})

    def route(self, command: str, trimming: bool) -> bool:
        """J/K/L/Space/Shift+K in trim mode (egui's ``route_transport``); False: not handled."""
        if command in _SHUTTLE and trimming:
            cid, params = _SHUTTLE[command]
            self.run(cid, {**params, "clock": self.clock()})
        elif command == "playback.stop" and self.playing:
            self.run("trim.shuttleStop", {"clock": self.clock()})
        elif command == "playback.toggle" and (trimming or self.info.get("playAround")):
            self.run("trim.playAround", {"clock": self.clock(), "toggle": True})
        elif command == "playback.playAround":
            self.run("trim.playAround", {"clock": self.clock(), "loop": False})
        else:
            return False
        return True

    def cancel_dynamic(self) -> bool:
        """Escape while a dynamic trim runs: drop it, restoring the edit."""
        if not isinstance(self.info.get("dynamic"), dict):
            return False
        self.run("trim.cancelDynamic", {})
        return True

    def run(self, cid: str, params: dict[str, Any]) -> None:
        """A trim playback command, then the monitor's state (and the tick while it plays)."""

        def done(r: Any, _b: bytes | None) -> None:
            if isinstance(r, dict) and isinstance(r.get("playhead"), int):
                self.playhead.emit(r["playhead"])
            self.changed.emit()
            self.refresh()

        self.engine.execute(cid, params, on_reply=done, on_error=lambda _m: self.refresh())

    def _tick(self) -> None:
        """Advance dynamic trimming / loop playback to the UI clock (egui's ``advance``)."""
        if not self.playing or not self.engine.running:
            self._timer.stop()
            return
        if self._tick_busy:
            return
        self._tick_busy = True

        def done(r: Any, _b: bytes | None) -> None:
            self._tick_busy = False
            if not isinstance(r, dict):
                return
            if isinstance(r.get("playhead"), int):
                self.playhead.emit(r["playhead"])
            dyn = r.get("dynamic")
            offset = dyn.get("offsetFrames") if isinstance(dyn, dict) else None
            if not r.get("playing"):
                self._timer.stop()
                self.changed.emit()
                self.refresh()
            else:
                if offset != self._last_offset or (isinstance(dyn, dict) and dyn != {k: (self.info.get("dynamic") or {}).get(k) for k in dyn}):
                    self._last_offset = offset
                    self._dirty = True
                # the sequence moves with the edit: refresh it at most every 60 ms
                if self._dirty and self._last_change.elapsed() >= 60:
                    self._dirty = False
                    self._last_change.restart()
                    self.changed.emit()
                    self.refresh()

        def failed(_m: str) -> None:
            self._tick_busy = False
            self._timer.stop()

        self.engine.execute("trim.tick", {"clock": self.clock()}, on_reply=done, on_error=failed)
