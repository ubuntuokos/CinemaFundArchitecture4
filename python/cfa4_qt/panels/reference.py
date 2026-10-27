"""Reference Monitor, as egui's (crates/ui-egui/src/panels/reference.rs): a second view of the
active sequence for comparing shots while grading. It is either ganged to the Program monitor
(follows the playhead) or parked on its own frame, and shows the picture (Composite Video) or
Lumetri Scopes of that frame (its own choice of scopes, right-click; the other scope settings
are the Lumetri Scopes panel's). Frames and traces are the engine's.
"""

from __future__ import annotations

import json
from dataclasses import asdict
from typing import Any

from PySide6.QtCore import QSettings
from PySide6.QtGui import QImage
from PySide6.QtWidgets import QComboBox, QHBoxLayout, QLabel, QMenu, QStackedWidget, QToolButton, QVBoxLayout, QWidget

from .. import theme
from ..i18n import N_, tr
from ..timecode import format_timecode, frame_ticks
from .monitor import FrameView
from .scopes import KINDS, ORDER, ScopesState, ScopesView
from .transport import icon_color, transport_icon


class ReferenceMonitor(QWidget):
    def __init__(self, engine: Any, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("panel.ReferenceMonitor")
        self.engine = engine
        self.settings: QSettings | None = None
        self.sequence: dict[str, Any] | None = None
        self.revision: Any = -1
        self.playhead = 0
        self.ganged = False  # egui's default
        self.scope_kinds = ["vectorscopeYuv", "waveform"]
        self.scope_settings: Any = None  # the Lumetri Scopes panel's settings (a callable)
        self.time = 0
        self._busy = False
        self._wanted: tuple[Any, ...] | None = None
        self._shown: tuple[Any, ...] | None = None
        self.view = FrameView("reference", parent=self)
        self.scopes = ScopesView(engine, "reference.scopes", self)
        self.scopes.menu_requested.connect(lambda pos: self.scopes_menu().exec(pos))
        self.stack = QStackedWidget(self)
        self.stack.addWidget(self.view)
        self.stack.addWidget(self.scopes)
        self.timecode = QLabel("00:00:00:00", self)
        self.timecode.setObjectName("reference.timecode")
        self.back = self._button("reference.stepBack", N_("Step Back 1 Frame"), "stepBack", lambda: self.step(-1))
        self.forward = self._button("reference.stepForward", N_("Step Forward 1 Frame"), "stepForward", lambda: self.step(1))
        self.match = QToolButton(self)
        self.match.setObjectName("reference.matchPlayhead")
        self.match.setText(tr("Match Playhead"))
        self.match.clicked.connect(self.match_playhead)
        self.gang = QToolButton(self)
        self.gang.setObjectName("reference.gang")
        self.gang.setText(tr("Gang to Program"))
        self.gang.setCheckable(True)
        self.gang.clicked.connect(self.toggle_gang)
        self.display = QComboBox(self)
        self.display.setObjectName("reference.display")
        self.display.addItem(tr("Composite Video"), "composite")
        self.display.addItem(tr("Lumetri Scopes"), "scopes")
        self.display.activated.connect(lambda _i: self.set_display(str(self.display.currentData())))
        bar = QHBoxLayout()
        bar.setContentsMargins(10, 2, 8, 4)
        bar.setSpacing(4)
        bar.addWidget(self.timecode)
        bar.addSpacing(12)
        for w in (self.back, self.forward, self.match, self.gang):
            bar.addWidget(w)
        bar.addStretch(1)
        bar.addWidget(self.display)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(4, 4, 4, 0)
        layout.setSpacing(2)
        layout.addWidget(self.stack, 1)
        layout.addLayout(bar)
        self._restyle()
        self._sync_gang()

    def _button(self, name: str, tip: str, icon: str, slot: Any) -> QToolButton:
        b = QToolButton(self)
        b.setObjectName(name)
        b.setToolTip(tr(tip))
        b.setIcon(transport_icon(icon, icon_color(self)))
        b.setAutoRaise(True)
        b.clicked.connect(slot)
        return b

    def _restyle(self) -> None:
        sheet = f"color: {theme.current().timecode}; font-family: monospace; font-size: 14px;"
        if self.timecode.styleSheet() != sheet:
            self.timecode.setStyleSheet(sheet)

    def changeEvent(self, e) -> None:
        super().changeEvent(e)
        if e.type() == e.Type.PaletteChange and hasattr(self, "timecode"):
            self._restyle()

    # ----- state -----
    def attach_settings(self, settings: QSettings | None) -> None:
        self.settings = settings
        raw = settings.value("panels/reference") if settings is not None else None
        try:
            data = json.loads(raw) if isinstance(raw, str) else {}
        except ValueError:
            data = {}
        if isinstance(data, dict):
            self.ganged = bool(data.get("ganged", False))
            t = data.get("time")  # saved state is untrusted
            self.time = int(t) if isinstance(t, (int, float)) and not isinstance(t, bool) and abs(t) < 2**62 else 0
            raw = data.get("scopes")
            kinds = [k for k in raw if isinstance(k, str) and k in ORDER] if isinstance(raw, list) else []
            self.scope_kinds = sorted(set(kinds), key=ORDER.index) or self.scope_kinds
            if isinstance(data.get("display"), str) and data["display"] in ("composite", "scopes"):
                self.display.setCurrentIndex(self.display.findData(data["display"]))
                self.stack.setCurrentIndex(1 if data["display"] == "scopes" else 0)
        self._sync_gang()

    def _save(self) -> None:
        if self.settings is not None:
            self.settings.setValue(
                "panels/reference",
                json.dumps({"ganged": self.ganged, "time": self.time, "display": self.display.currentData(), "scopes": self.scope_kinds}),
            )

    def set_display(self, display: str) -> None:
        self.display.setCurrentIndex(max(0, self.display.findData(display)))
        self.stack.setCurrentIndex(1 if display == "scopes" else 0)
        self._save()
        self.request()

    def scopes_menu(self) -> QMenu:
        """Right-click on the scopes: which scopes the Reference Monitor shows."""
        m = QMenu(self)
        for k, label in KINDS:
            a = m.addAction(tr(label))
            a.setObjectName(f"reference.scope.{k}")
            a.setCheckable(True)
            a.setChecked(k in self.scope_kinds)
            a.triggered.connect(lambda _c=False, kk=k: self.toggle_scope(kk))
        return m

    def toggle_scope(self, kind: str) -> None:
        if kind in self.scope_kinds:
            if len(self.scope_kinds) > 1:
                self.scope_kinds.remove(kind)
        else:
            self.scope_kinds = sorted([*self.scope_kinds, kind], key=ORDER.index)
        self._save()
        self.request()

    def _sync_gang(self) -> None:
        self.gang.setChecked(self.ganged)

    def frame_ticks(self) -> int:
        return frame_ticks((self.sequence or {}).get("settings"))

    def frame_time(self) -> int:
        """The time shown: the playhead when ganged, else the parked frame (inside the sequence)."""
        ft = self.frame_ticks()
        t = self.playhead if self.ganged else self.time
        end = max(ft, int((self.sequence or {}).get("duration") or ft))
        return max(0, min(t // ft * ft, end - ft))

    def set_sequence(self, seq: dict[str, Any] | None, revision: int) -> None:
        self.sequence = seq
        # the sequence is part of what a cached frame shows: opening another one doesn't change the revision
        self.revision = ((seq or {}).get("id"), revision)
        if seq is not None and isinstance(seq.get("playhead"), int):
            self.playhead = seq["playhead"]
        self.request()

    def set_playhead(self, ticks: int) -> None:
        self.playhead = int(ticks)
        if self.ganged:
            self.request()

    def step(self, frames: int) -> None:
        ft = self.frame_ticks()
        self.time = max(0, self.frame_time() + frames * ft)
        self.ganged = False
        self._sync_gang()
        self._save()
        self.request()

    def match_playhead(self) -> None:
        self.time = self.playhead
        self.ganged = False
        self._sync_gang()
        self._save()
        self.request()

    def toggle_gang(self) -> None:
        if self.ganged:
            self.time = self.frame_time()
        self.ganged = not self.ganged
        self._sync_gang()
        self._save()
        self.request()

    # ----- frames -----
    def request(self) -> None:
        if self.sequence is None:
            self.view.set_image(None, tr("(no sequences)"))
            return
        t = self.frame_time()
        self.timecode.setText(format_timecode(t, self.frame_ticks()))
        if not self.isVisible():
            return
        if self.display.currentData() == "scopes":
            base = self.scope_settings() if callable(self.scope_settings) else None
            st = ScopesState(**{**(asdict(base) if base is not None else {}), "shown": list(self.scope_kinds)})
            self.scopes.st = st
            self.scopes.set_frame(t, self.revision)
            return
        w, h = self.view.wanted_size()
        self._wanted = (t, self.revision, w, h)
        self._pump()

    def showEvent(self, e) -> None:
        super().showEvent(e)
        self.request()

    def resizeEvent(self, e) -> None:
        super().resizeEvent(e)
        self.request()

    def _pump(self) -> None:
        if self._busy or self._wanted is None or not getattr(self.engine, "running", False):
            return
        key, self._wanted = self._wanted, None
        if key == self._shown:
            return
        t, _rev, w, h = key
        self._busy = True
        self.engine.call(
            "frame.render",
            {"time": t, "maxWidth": w, "maxHeight": h},
            on_reply=lambda r, b, k=key: self._on_frame(k, r, b),
            on_error=lambda m: self._on_error(m),
        )

    def _on_frame(self, key: tuple[Any, ...], result: Any, data: bytes | None) -> None:
        self._busy = False
        if isinstance(result, dict) and data:
            w, h = int(result.get("w") or 0), int(result.get("h") or 0)
            if w > 0 and h > 0 and len(data) == w * h * 4:
                image = QImage(data, w, h, w * 4, QImage.Format.Format_RGBA8888).copy()
                full = (int(result.get("fullW") or 0), int(result.get("fullH") or 0))
                self.view.set_image(image, "", full, result.get("pixelAspect") if isinstance(result.get("pixelAspect"), (int, float)) else None)
                self._shown = key
        self._pump()

    def _on_error(self, message: str) -> None:
        self._busy = False
        self.view.set_image(None, message)
        self._pump()

