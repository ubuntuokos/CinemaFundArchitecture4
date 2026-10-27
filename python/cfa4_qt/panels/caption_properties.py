"""The Properties panel for the first selected caption, as egui's (crates/ui-egui/src/panels/text.rs
``caption_properties``): a "C1 · track" header, the caption's text and its In / Out timecodes, each
sent when the field loses focus with the same ``captions.*`` commands as the Captions tab, and its
duration. The style belongs to the caption track and stays in the Captions tab.
"""

from __future__ import annotations

from typing import Any

from PySide6.QtCore import Qt, Signal
from PySide6.QtGui import QFontDatabase
from PySide6.QtWidgets import QGridLayout, QLabel, QLineEdit, QPlainTextEdit, QVBoxLayout, QWidget

from ..i18n import N_, tr

EDGE_LABEL_W = 70  # egui's label column
TC_W = 100  # egui's timecode field
EDGES = [("in", N_("In"), "startTimecode"), ("out", N_("Out"), "endTimecode")]


class CaptionText(QPlainTextEdit):
    """The caption's text; sent when it loses focus (egui's ``lost_focus``)."""

    done = Signal()

    def focusOutEvent(self, e: Any) -> None:
        super().focusOutEvent(e)
        self.done.emit()


class CaptionProperties(QWidget):
    command = Signal(str, dict)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("properties.caption")
        self.caption: dict[str, Any] | None = None
        lay = QVBoxLayout(self)
        lay.setContentsMargins(14, 8, 14, 8)
        lay.setSpacing(4)
        self.header = QLabel(self)
        self.header.setObjectName("properties.caption.header")
        self.header.setFixedHeight(30)
        font = self.header.font()
        font.setBold(True)
        self.header.setFont(font)
        lay.addWidget(self.header)
        lay.addWidget(self._dim(tr("Text")))
        self.text = CaptionText(self)
        self.text.setObjectName("properties.caption.text")
        self.text.setTabChangesFocus(True)
        rows = self.text.fontMetrics().lineSpacing() * 3 + 12
        self.text.setFixedHeight(rows)
        self.text.done.connect(self._text_done)
        lay.addWidget(self.text)
        lay.addSpacing(8)
        grid = QGridLayout()
        grid.setHorizontalSpacing(6)
        grid.setVerticalSpacing(4)
        grid.setColumnStretch(2, 1)
        mono = QFontDatabase.systemFont(QFontDatabase.SystemFont.FixedFont)
        self.edges: dict[str, QLineEdit] = {}
        for row, (edge, label, key) in enumerate(EDGES):
            name = self._dim(tr(label))
            name.setFixedWidth(EDGE_LABEL_W)
            field = QLineEdit(self)
            field.setObjectName(f"properties.caption.{edge}")
            field.setFont(mono)
            field.setFixedWidth(TC_W)
            field.editingFinished.connect(lambda e=edge, k=key: self._tc_done(e, k))
            grid.addWidget(name, row, 0)
            grid.addWidget(field, row, 1)
            self.edges[edge] = field
        name = self._dim(tr("Duration"))
        name.setFixedWidth(EDGE_LABEL_W)
        self.duration = QLabel(self)
        self.duration.setObjectName("properties.caption.duration")
        self.duration.setFont(mono)
        grid.addWidget(name, 2, 0)
        grid.addWidget(self.duration, 2, 1)
        lay.addLayout(grid)
        lay.addStretch(1)

    def _dim(self, text: str) -> QLabel:
        label = QLabel(text, self)
        label.setEnabled(False)
        label.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter)
        return label

    def set_caption(self, track: dict[str, Any], caption: dict[str, Any]) -> None:
        """Show ``caption`` of ``track`` (``captions.list`` rows); a field being edited keeps its text."""
        self.caption = caption
        self.header.setText(f"{track.get('label') or 'C1'} · {track.get('name') or ''}")
        if not self.text.hasFocus():
            self.text.setPlainText(str(caption.get("text") or ""))
        for edge, field in self.edges.items():
            if not field.hasFocus():
                field.setText(str(caption.get(edge) or ""))
        self.duration.setText(str(caption.get("duration") or ""))

    def _text_done(self) -> None:
        c = self.caption
        text = self.text.toPlainText()
        if c is not None and text != str(c.get("text") or ""):
            c["text"] = text  # one send per edit, however often focus moves before the engine answers
            self.command.emit("captions.setText", {"caption": c.get("id"), "text": text})

    def _tc_done(self, edge: str, key: str) -> None:
        c = self.caption
        value = self.edges[edge].text().strip()
        if c is not None and value and value != str(c.get(edge) or ""):
            c[edge] = value
            self.command.emit("captions.setTimes", {"caption": c.get("id"), key: value})


__all__ = ["CaptionProperties"]
