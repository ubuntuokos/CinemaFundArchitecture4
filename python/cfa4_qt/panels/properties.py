"""The Properties panel, as egui's (crates/ui-egui/src/panels/effect_controls.rs ``properties_panel``,
and graphics.rs ``properties`` for a graphic clip).

The first selected clip: a header with its label colour and name, then collapsible Transform
(Position, Anchor point, Scale, Rotation, Opacity) and Crop sections for a video clip and an Audio
section (Level, Pan) for the first selected audio clip, each value an egui hot number (a scrub is one
undo step), each row with Premiere's keyframe navigator (previous / add-remove / next) and each
section with a reset; a Speed button at the foot opens Speed/Duration. A graphic clip shows the
graphic editor of Essential Graphics ▸ Edit instead. The values come from ``effects.inspect`` (and
``graphics.inspect``); every change is an engine command.
"""

from __future__ import annotations

from typing import Any

from PySide6.QtCore import QElapsedTimer, QSettings, QSize, Qt, QTimer, Signal
from PySide6.QtGui import QColor
from PySide6.QtWidgets import (
    QFrame,
    QGridLayout,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QScrollArea,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from ..hot_number import HotSpin
from ..i18n import N_, fmt, tr
from .essential_graphics import EssentialGraphicsPanel, SectionHeader
from .timeline import LABEL_COLORS

SEND_DELAY_MS = 30  # scrub values are coalesced this long
GESTURE_GAP_MS = 600  # values further apart than this are separate undo steps
LABEL_W = 150  # egui's label column

# section -> rows: (label, effect, param, kind, speed, (lo, hi), decimals, suffix)
TRANSFORM = [
    (N_("Position"), "motion", "position", "point", 1.0, (-100_000.0, 100_000.0), 0, ""),
    (N_("Anchor point"), "motion", "anchor", "point", 1.0, (-100_000.0, 100_000.0), 0, ""),
    (N_("Scale"), "motion", "scale", "float", 0.5, (0.0, 10_000.0), 0, " %"),
    (N_("Rotation"), "motion", "rotation", "float", 0.5, (-36_000.0, 36_000.0), 1, " °"),
    (N_("Opacity"), "opacity", "opacity", "float", 0.5, (0.0, 100.0), 0, " %"),
]
CROP = [
    (N_("Left"), "crop", "left", "float", 0.2, (0.0, 100.0), 1, " %"),
    (N_("Top"), "crop", "top", "float", 0.2, (0.0, 100.0), 1, " %"),
    (N_("Right"), "crop", "right", "float", 0.2, (0.0, 100.0), 1, " %"),
    (N_("Bottom"), "crop", "bottom", "float", 0.2, (0.0, 100.0), 1, " %"),
]
AUDIO = [
    (N_("Level"), "volume", "level", "float", 0.1, (-96.0, 15.0), 1, " dB"),
    (N_("Pan"), "panner", "balance", "float", 0.5, (-100.0, 100.0), 0, ""),
]
# the section's reset resets this effect; Pan has no keyframe navigator (egui's)
SECTIONS = [(N_("Transform"), "motion", TRANSFORM, "video"), (N_("Crop"), "crop", CROP, "video"), (N_("Audio"), "volume", AUDIO, "audio")]
NO_NAV = {("panner", "balance")}


def _effect(clip: dict[str, Any] | None, effect: str) -> dict[str, Any] | None:
    return next((e for e in (clip or {}).get("effects") or [] if isinstance(e, dict) and e.get("effect") == effect), None)


def _param(clip: dict[str, Any] | None, effect: str, param: str) -> dict[str, Any] | None:
    e = _effect(clip, effect)
    return next((p for p in (e or {}).get("params") or [] if isinstance(p, dict) and p.get("id") == param), None)


class PropRow:
    """One row: label, hot value(s) and the keyframe navigator."""

    def __init__(self, panel: PropertiesPanel, grid: QGridLayout, row: int, spec: tuple[Any, ...]) -> None:
        label, self.effect, self.param, kind, speed, (lo, hi), decimals, suffix = spec
        self.panel = panel
        self.kind = kind
        self.audio = self.effect in ("volume", "panner")
        parent = grid.parentWidget()
        name = f"properties.{self.effect}.{self.param}"
        self.label = QLabel(tr(label), parent)
        self.label.setObjectName(f"{name}.label")
        self.label.setEnabled(False)  # egui's dim label colour
        box = QWidget(parent)
        h = QHBoxLayout(box)
        h.setContentsMargins(0, 0, 0, 0)
        h.setSpacing(10)
        self.spins: list[HotSpin] = []
        for i, axis in enumerate(("X", "Y") if kind == "point" else ("",)):
            spin = HotSpin(box, lo, hi, decimals, speed, f" {axis}" if axis else suffix)
            spin.setObjectName(f"{name}.{axis.lower()}" if axis else name)
            spin.edited.connect(lambda v, begin, i=i: self._edited(i, v, begin))
            self.spins.append(spin)
            h.addWidget(spin)
        h.addStretch(1)
        grid.addWidget(self.label, row, 0)
        grid.addWidget(box, row, 1)
        self.prev = self.add = self.next = None
        if (self.effect, self.param) not in NO_NAV:
            nav = QWidget(parent)
            n = QHBoxLayout(nav)
            n.setContentsMargins(0, 0, 0, 0)
            n.setSpacing(0)
            self.prev = self._button(nav, "◀", N_("Go to previous keyframe"), f"{name}.prevKeyframe", lambda: self._go(-1))
            self.add = self._button(nav, "◇", "", f"{name}.addKeyframe", self._add_remove)
            self.next = self._button(nav, "▶", N_("Go to next keyframe"), f"{name}.nextKeyframe", lambda: self._go(1))
            for b in (self.prev, self.add, self.next):
                n.addWidget(b)
            grid.addWidget(nav, row, 2)
        self.info: dict[str, Any] | None = None

    @staticmethod
    def _button(parent: QWidget, text: str, tip: str, name: str, slot: Any) -> QToolButton:
        b = QToolButton(parent)
        b.setObjectName(name)
        b.setText(text)
        if tip:
            b.setToolTip(tr(tip))
        b.setAutoRaise(True)
        b.setFixedSize(16, 22)
        b.setStyleSheet("QToolButton { border: none; padding: 0; }")
        b.clicked.connect(slot)
        return b

    def clip(self) -> dict[str, Any] | None:
        return self.panel.audio_clip if self.audio else self.panel.clip

    def show(self) -> None:
        clip = self.clip()
        self.info = _param(clip, self.effect, self.param)
        v = (self.info or {}).get("valueAtPlayhead")
        values = v if isinstance(v, list) else [v]
        # an automatic coordinate (null) shows where the render puts it, so editing the other
        # one keeps the picture where it is
        auto = (self.info or {}).get("auto")
        auto = auto if isinstance(auto, list) else []
        for i, (spin, x) in enumerate(zip(self.spins, values, strict=False)):
            fallback = auto[i] if i < len(auto) and isinstance(auto[i], (int, float)) else 0.0
            spin.set_value(x if isinstance(x, (int, float)) else fallback)
        if self.add is None:
            return
        keys = [k for k in (self.info or {}).get("keyframes") or [] if isinstance(k, dict)]
        animated = bool((self.info or {}).get("animated"))
        here = any(k.get("mediaTime") == (clip or {}).get("mediaTimeAtPlayhead") for k in keys)
        self.add.setText("◆" if here else "◇")
        tip = N_("Remove keyframe") if here else N_("Add keyframe") if animated else N_("Turn on animation and add keyframe")
        self.add.setToolTip(tr(tip))
        times = self._times()
        ph = int((clip or {}).get("playhead") or 0)
        for b, on in ((self.prev, any(t < ph for t in times)), (self.next, any(t > ph for t in times))):
            assert b is not None
            b.setVisible(animated)
            b.setEnabled(on)

    def _times(self) -> list[int]:
        return sorted(int(k["time"]) for k in (self.info or {}).get("keyframes") or [] if isinstance(k, dict) and isinstance(k.get("time"), int))

    def _go(self, d: int) -> None:
        ph = int((self.clip() or {}).get("playhead") or 0)
        times = [t for t in self._times() if (t < ph if d < 0 else t > ph)]
        if times:
            self.panel.seek.emit(times[-1] if d < 0 else times[0])

    def _add_remove(self) -> None:
        clip = self.clip()
        if clip is None:
            return
        self.panel.ensure_effect(clip, self.effect)
        self.panel.send("effects.addKeyframe", {"clip": clip["clip"], "effect": self.effect, "param": self.param})

    def _edited(self, i: int, value: float, begin: bool) -> None:
        clip = self.clip()
        if clip is None:
            return
        if begin:
            self.panel.new_gesture()
        out: Any = value
        if self.kind == "point":
            out = [s.value() for s in self.spins]
            out[i] = value
        self.panel.set_param(clip, self.effect, self.param, out)


class PropertiesPanel(QWidget):
    command = Signal(str, dict)
    run = Signal(str, dict)  # a command that needs a file (the graphic editor's Install…)
    seek = Signal(object)  # ticks
    invoke = Signal(str)  # a command with its dialog (Speed/Duration…)

    def __init__(self, engine: Any = None, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.Inspector")
        self.engine = engine
        self.settings: QSettings | None = None
        self.sequence: dict[str, Any] | None = None
        self.clip: dict[str, Any] | None = None  # effects.inspect of the first selected clip
        self.audio_clip: dict[str, Any] | None = None  # … of the first selected audio clip
        self.video = False  # the first selected clip is on a video track
        self.collapsed: set[str] = set()
        self._pending: dict[tuple[int, str, str, int | None], Any] = {}
        self._applied: set[tuple[int, str]] = set()  # effects applied since the last inspect
        self._send_timer = QTimer(self, singleShot=True, interval=SEND_DELAY_MS)
        self._send_timer.timeout.connect(self.flush)
        self._last_send = QElapsedTimer()
        self._new_gesture = False
        self._busy = False
        self._again = False

        self.empty = QLabel(tr("Select a clip to see its properties"), self)
        self.empty.setObjectName("properties.placeholder")
        self.empty.setWordWrap(True)
        self.empty.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.empty.setEnabled(False)
        self.graphic = EssentialGraphicsPanel(engine, self)
        self.graphic.tabs.hide()
        self.graphic.set_tab("edit")
        self.graphic.command.connect(self.command)
        self.graphic.run.connect(self.run)
        self.graphic.hide()

        self.body = QWidget(self)
        self.lay = QVBoxLayout(self.body)
        self.lay.setContentsMargins(14, 8, 14, 12)
        self.lay.setSpacing(0)
        head = QHBoxLayout()
        head.setSpacing(8)
        self.swatch = QLabel(self.body)
        self.swatch.setFixedSize(12, 12)
        self.title = QLabel(self.body)
        self.title.setObjectName("properties.name")
        font = self.title.font()
        font.setBold(True)
        self.title.setFont(font)
        head.addWidget(self.swatch)
        head.addWidget(self.title, 1)
        self.lay.addLayout(head)
        self.lay.addSpacing(6)
        self.rows: list[PropRow] = []
        self.sections: dict[str, tuple[SectionHeader, QWidget, str, QToolButton]] = {}
        self._labels: list[QLabel] = []
        for title, reset_effect, specs, kind in SECTIONS:
            header = SectionHeader(title, True, self.body)
            header.setObjectName(f"properties.section.{title}")
            header.twirled.connect(self.twirl)
            reset = QToolButton(header)
            reset.setObjectName(f"properties.section.{title}.reset")
            reset.setText("↺")
            reset.setToolTip(tr("Reset Effect"))
            reset.setAutoRaise(True)
            reset.setFixedSize(20, 20)
            reset.clicked.connect(lambda _c=False, e=reset_effect, a=kind == "audio": self._reset(e, a))
            header.extra = reset
            rows = QWidget(self.body)
            grid = QGridLayout(rows)
            grid.setContentsMargins(0, 0, 0, 0)
            grid.setHorizontalSpacing(6)
            grid.setVerticalSpacing(4)
            grid.setColumnStretch(1, 1)
            for i, spec in enumerate(specs):
                row = PropRow(self, grid, i, spec)
                self.rows.append(row)
                self._labels.append(row.label)
            self.lay.addWidget(header)
            self.lay.addWidget(rows)
            self.lay.addSpacing(4)
            self.sections[title] = (header, rows, kind, reset)
        self.lay.addSpacing(12)
        self.speed = QPushButton(self.body)
        self.speed.setObjectName("properties.speed")
        self.speed.setFixedSize(118, 26)
        self.speed.clicked.connect(lambda: self.invoke.emit("clip.speedDuration"))
        self.lay.addWidget(self.speed)
        self.lay.addStretch(1)
        self.scroll = QScrollArea(self)
        self.scroll.setWidgetResizable(True)
        self.scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.scroll.setFrameShape(QFrame.Shape.NoFrame)
        self.scroll.setWidget(self.body)
        self.scroll.hide()
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.addWidget(self.empty, 1)
        layout.addWidget(self.graphic, 1)
        layout.addWidget(self.scroll, 1)

    def attach_settings(self, settings: QSettings) -> None:
        self.settings = settings
        self.graphic.attach_settings(settings)
        raw = settings.value("properties/collapsed", [])
        self.collapsed = {str(s) for s in (raw if isinstance(raw, list) else [raw] if raw else [])}

    def sizeHint(self) -> QSize:
        return QSize(300, 500)

    def resizeEvent(self, e: Any) -> None:
        super().resizeEvent(e)
        w = max(90, min(LABEL_W, int(self.scroll.viewport().width() * 0.38)))
        for label in self._labels:
            label.setFixedWidth(w)

    def scrubbing(self) -> bool:
        return any(s._scrubbing for s in self.findChildren(HotSpin))

    # ----- loading -----
    def selected(self) -> list[tuple[dict[str, Any], str]]:
        """The selected clips with their track kind, in timeline order (video first)."""
        seq = self.sequence or {}
        sel = set(seq.get("selection") or [])
        return [(c, kind) for kind in ("video", "audio") for t in seq.get(kind) or [] for c in t.get("items") or [] if c.get("clip") in sel]

    def set_sequence(self, seq: dict[str, Any] | None) -> None:
        self.sequence = seq
        if not self.selected():
            self._show(None, None, None)
        self.empty.setVisible(not self.selected())
        self.refresh()

    def refresh(self) -> None:
        """Ask the engine about the selection (a graphic clip first, as egui's)."""
        clips = self.selected()
        if not clips or self.engine is None or not getattr(self.engine, "running", False) or not self.isVisible():
            return
        if self._busy or self.scrubbing():
            self._again = True
            return
        self._busy = True
        first, kind = clips[0]
        audio = next((c for c, k in clips if k == "audio"), None)
        got: dict[str, Any] = {}

        def finish() -> None:
            self._busy = False
            self.video = kind == "video"
            self._show(got.get("graphic"), got.get("clip"), got.get("audio"))
            if self._again:
                self._again = False
                self.refresh()

        def effects() -> None:
            def clip_done(r: Any, _b: bytes | None) -> None:
                got["clip"] = r if isinstance(r, dict) else None
                if audio is None or audio is first:
                    got["audio"] = got["clip"] if audio is first else None
                    finish()
                else:
                    self.engine.execute("effects.inspect", {"clip": audio["clip"]}, on_reply=audio_done, on_error=lambda _m: audio_done(None, None))

            def audio_done(r: Any, _b: bytes | None) -> None:
                got["audio"] = r if isinstance(r, dict) else None
                finish()

            self.engine.execute("effects.inspect", {"clip": first["clip"]}, on_reply=clip_done, on_error=lambda _m: clip_done(None, None))

        def graphic_done(r: Any, _b: bytes | None) -> None:
            if isinstance(r, dict) and r.get("clip") is not None:
                got["graphic"] = r
                finish()
            else:
                effects()

        self.engine.call("graphics.inspect", {}, on_reply=graphic_done, on_error=lambda _m: graphic_done(None, None))

    def showEvent(self, e: Any) -> None:
        super().showEvent(e)
        self.refresh()

    def _show(self, graphic: dict[str, Any] | None, clip: dict[str, Any] | None, audio: dict[str, Any] | None) -> None:
        self.flush()  # values scrubbed on the clip shown so far go to that clip
        self._applied.clear()
        has = bool(self.selected())
        self.empty.setVisible(not has)
        self.graphic.setVisible(has and graphic is not None)
        self.scroll.setVisible(has and graphic is None)
        if graphic is not None:
            self.graphic.set_state(graphic)
        self.clip = clip if clip is not None and clip.get("clip") is not None else None
        self.audio_clip = audio if audio is not None and audio.get("clip") is not None else None
        if not has or graphic is not None or (self.clip is None and self.audio_clip is None):
            return
        self.title.setText(str((self.clip or self.audio_clip or {}).get("name") or ""))
        label = next((c.get("label") for c, _k in self.selected() if c.get("clip") == (self.clip or {}).get("clip")), None)
        color = QColor(LABEL_COLORS.get(str(label or "").lower(), "#5b7fb8" if self.video else "#4f9a6a"))
        self.swatch.setStyleSheet(f"background: {color.name()}; border-radius: 2px;")
        for title, (header, rows, kind, reset) in self.sections.items():
            shown = self.video if kind == "video" else self.audio_clip is not None
            header.open = title not in self.collapsed
            header.setVisible(shown)
            header.update()
            rows.setVisible(shown and header.open)
            target = self.audio_clip if kind == "audio" else self.clip
            reset.setVisible(_effect(target, "volume" if kind == "audio" else "motion" if title == "Transform" else "crop") is not None)
        for row in self.rows:
            row.show()
        speed = float((self.clip or self.audio_clip or {}).get("speed") or 1.0)
        self.speed.setText(fmt("Speed {n}%", n=f"{speed * 100:.0f}"))

    def twirl(self, title: str) -> None:
        self.collapsed ^= {title}
        if self.settings is not None:
            self.settings.setValue("properties/collapsed", sorted(self.collapsed))
        self._show(None, self.clip, self.audio_clip)

    # ----- edits -----
    def ensure_effect(self, clip: dict[str, Any], effect: str) -> None:
        """Apply ``effect`` (Crop) first when the clip doesn't have it yet (egui's)."""
        key = (int(clip["clip"]), effect)
        if _effect(clip, effect) is None and key not in self._applied:
            self._applied.add(key)
            self.send("effects.apply", {"clips": [clip["clip"]], "effect": effect})

    def _reset(self, effect: str, audio: bool) -> None:
        clip = self.audio_clip if audio else self.clip
        e = _effect(clip, effect)
        if clip is not None and e is not None:
            self.send("effects.reset", {"clip": clip["clip"], "index": e.get("index")})

    def set_param(self, clip: dict[str, Any], effect: str, param: str, value: Any) -> None:
        self.ensure_effect(clip, effect)
        monitor = getattr(self.window(), "monitor", None)
        at = None  # while playing, the shown frame: the engine's playhead stays where playback started
        if monitor is not None and getattr(monitor, "playing", False) and isinstance(getattr(monitor, "playhead", None), int):
            at = monitor.playhead
        self._pending[(int(clip["clip"]), effect, param, at)] = value
        self._send_timer.start()

    def new_gesture(self) -> None:
        """The next value starts its own undo step, however soon it follows the last one."""
        self.flush()
        self._new_gesture = True

    def send(self, command: str, params: dict[str, Any]) -> None:
        self.flush()
        self.command.emit(command, params)

    def flush(self) -> None:
        self._send_timer.stop()
        if not self._pending:
            return
        begin = self._new_gesture or not self._last_send.isValid() or self._last_send.elapsed() > GESTURE_GAP_MS
        self._new_gesture = False
        for (clip, effect, param, at), value in self._pending.items():
            params = {"clip": clip, "effect": effect, "param": param, "value": value, "merge": True, "begin": begin}
            if at is not None:
                params["time"] = at
            self.command.emit("effects.setParam", params)
        self._pending.clear()
        self._last_send.restart()


__all__ = ["PropertiesPanel"]
