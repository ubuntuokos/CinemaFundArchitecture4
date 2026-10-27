"""Clip ▸ Remix ▸ Remix Properties…, as egui's ``panels/remix.rs``: Target Duration, Segments and
Variations of the selected remixed clip. OK runs ``clip.remix.properties`` (one undo step); the
Remix tool in the Timeline sets the target by dragging a clip's Out edge instead.

Automation ids (egui's): the dialog ``remixProperties``, ``remixProperties.duration`` (seconds),
``remixProperties.segments``, ``remixProperties.variations`` (sliders 0 … 100),
``remixProperties.info`` (the current duration), ``remixProperties.ok``, ``remixProperties.cancel``.
"""

from __future__ import annotations

import math
from collections.abc import Callable
from typing import Any

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QDialog,
    QDoubleSpinBox,
    QFormLayout,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QSlider,
    QVBoxLayout,
    QWidget,
)

from ..engine import EngineClient
from ..i18n import fmt, tr


def number(value: Any, default: float) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return default
    x = float(value)
    return x if math.isfinite(x) else default


class RemixPropertiesDialog(QDialog):
    """``state`` is ``clip.remix.properties``' answer without parameters (the selected clip)."""

    def __init__(self, parent: QWidget, engine: EngineClient, state: dict[str, Any], done: Callable[[str], None]) -> None:
        super().__init__(parent)
        self.setObjectName("remixProperties")
        self.setWindowTitle(tr("Remix Properties"))
        self.engine = engine
        self.done_message = done
        self.clip = state.get("clip")
        seconds = number(state.get("seconds"), 0.0)
        lay = QVBoxLayout(self)
        form = QFormLayout()
        self.duration = QDoubleSpinBox(self)
        self.duration.setObjectName("remixProperties.duration")
        self.duration.setRange(1.0, 36_000.0)
        self.duration.setDecimals(2)
        self.duration.setSingleStep(0.1)
        self.duration.setSuffix(" s")
        self.duration.setValue(number(state.get("targetSeconds"), seconds))
        form.addRow(tr("Target Duration:"), self.duration)
        self.segments = self._slider("remixProperties.segments", number(state.get("segments"), 50.0))
        form.addRow(tr("Segments:"), self._with_hint(self.segments))
        self.variations = self._slider("remixProperties.variations", number(state.get("variations"), 50.0))
        form.addRow(tr("Variations:"), self._with_hint(self.variations))
        lay.addLayout(form)
        self.info = QLabel(fmt("Current duration {seconds} s", seconds=f"{seconds:.2f}"), self)
        self.info.setObjectName("remixProperties.info")
        self.info.setEnabled(False)
        lay.addWidget(self.info)
        self.error = QLabel("", self)
        self.error.setStyleSheet("color: #e08a6a;")
        self.error.setWordWrap(True)
        self.error.hide()
        lay.addWidget(self.error)
        row = QHBoxLayout()
        row.addStretch(1)
        cancel = QPushButton(tr("Cancel"), self)
        cancel.setObjectName("remixProperties.cancel")
        cancel.clicked.connect(self.reject)
        row.addWidget(cancel)
        ok = QPushButton(tr("OK"), self)
        ok.setObjectName("remixProperties.ok")
        ok.setDefault(True)
        ok.clicked.connect(self.apply)
        row.addWidget(ok)
        lay.addLayout(row)
        self.setMinimumWidth(320)

    def _slider(self, name: str, value: float) -> QSlider:
        s = QSlider(Qt.Orientation.Horizontal, self)
        s.setObjectName(name)
        s.setRange(0, 100)
        s.setValue(round(min(100.0, max(0.0, value))))
        return s

    def _with_hint(self, slider: QSlider) -> QWidget:
        w = QWidget(self)
        h = QHBoxLayout(w)
        h.setContentsMargins(0, 0, 0, 0)
        h.addWidget(slider, 1)
        hint = QLabel(tr("Fewer · More"), w)
        hint.setEnabled(False)
        h.addWidget(hint)
        return w

    def values(self) -> dict[str, Any]:
        return {"clip": self.clip, "seconds": self.duration.value(), "segments": float(self.segments.value()), "variations": float(self.variations.value())}

    def apply(self) -> None:
        def done(result: Any, _b: bytes | None) -> None:
            r = result if isinstance(result, dict) else {}
            self.done_message(fmt("Remixed to {seconds} s", seconds=f"{number(r.get('seconds'), 0.0):.2f}"))
            self.accept()

        def failed(message: str) -> None:
            self.error.setText(message)
            self.error.show()

        self.engine.execute("clip.remix.properties", self.values(), on_reply=done, on_error=failed)
