"""Parts of the Credits workspace (plan C8): the pages' strip along time and the properties of
the chosen page or block. They only show what the engine's preview (``graphics.credits.preview``)
says; nothing here lays out credits.
"""

from __future__ import annotations

import math
from typing import Any

from PySide6.QtCore import QRectF, Qt, Signal
from PySide6.QtGui import QColor, QMouseEvent, QPainter, QPaintEvent, QPen
from PySide6.QtWidgets import QFormLayout, QLabel, QSizePolicy, QVBoxLayout, QWidget

from ..i18n import N_, tr

# colours of the strip by page type (scroll, card, crawl); neutral, readable on light and dark
KIND_COLOURS = {"scroll": QColor("#4f7cac"), "card": QColor("#8a6fb0"), "crawl": QColor("#5f9c74")}
OTHER_COLOUR = QColor("#808080")
STRIP_HEIGHT = 34


def _num(value: Any) -> float:
    try:
        v = float(value)
    except (TypeError, ValueError, OverflowError):
        return 0.0
    return v if math.isfinite(v) else 0.0


def page_spans(pages: list[dict[str, Any]]) -> list[tuple[float, float]]:
    """Each page's start and end (ticks) when the pages follow each other with their gaps (a
    negative gap overlaps the next page), as the engine places them."""
    spans: list[tuple[float, float]] = []
    t = 0.0
    for p in pages:
        d = max(0.0, _num(p.get("duration")))
        spans.append((t, t + d))
        t = max(0.0, t + d + _num(p.get("gapAfter")))
    return spans


class TimelineStrip(QWidget):
    """The pages along time, as long as they last; clicking one chooses it. The mark is the
    moment the preview shows."""

    page_clicked = Signal(int)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("credits.timeline")
        self.setMinimumHeight(STRIP_HEIGHT)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Fixed)
        self.setToolTip(tr("The pages along time; click one to choose it"))
        self.pages: list[dict[str, Any]] = []
        self.spans: list[tuple[float, float]] = []
        self.current = -1
        self.at = 0.5

    def set_pages(self, pages: list[dict[str, Any]]) -> None:
        self.pages = [p for p in pages if isinstance(p, dict)]
        self.spans = page_spans(self.pages)
        self.update()

    def set_current(self, index: int, at: float) -> None:
        self.current, self.at = index, min(1.0, max(0.0, _num(at)))
        self.update()

    def total(self) -> float:
        return max((e for _s, e in self.spans), default=0.0)

    def _rect(self, i: int) -> QRectF:
        total = self.total()
        if total <= 0 or not (0 <= i < len(self.spans)):
            return QRectF()
        w = max(1.0, self.width() - 2.0)
        s, e = self.spans[i]
        return QRectF(1.0 + s / total * w, 3.0, max(2.0, (e - s) / total * w), self.height() - 6.0)

    def index_at(self, x: float) -> int:
        """The page under `x` (the topmost, so an overlapping later page wins); -1 for none."""
        for i in reversed(range(len(self.spans))):
            r = self._rect(i)
            if r.left() <= x <= r.right():
                return i
        return -1

    def paintEvent(self, _e: QPaintEvent) -> None:
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        pal = self.palette()
        p.fillRect(self.rect(), pal.base())
        if not self.spans or self.total() <= 0:
            p.setPen(pal.placeholderText().color())
            p.drawText(self.rect(), Qt.AlignmentFlag.AlignCenter, tr("No pages"))
            return
        for i, page in enumerate(self.pages):
            r = self._rect(i)
            colour = KIND_COLOURS.get(str(page.get("behaviour")), OTHER_COLOUR)
            p.setBrush(colour.lighter(130) if i == self.current else colour)
            p.setPen(QPen(pal.highlight().color(), 2) if i == self.current else QPen(colour.darker(140), 1))
            p.drawRoundedRect(r, 3, 3)
            if r.width() > 40:
                p.setPen(QColor("white"))
                p.drawText(r.adjusted(4, 0, -4, 0), Qt.AlignmentFlag.AlignVCenter | Qt.AlignmentFlag.AlignLeft, str(page.get("name") or i + 1))
        cur = self._rect(self.current)
        if not cur.isNull():
            x = cur.left() + cur.width() * self.at
            p.setPen(QPen(pal.highlight().color(), 2))
            p.drawLine(int(x), 0, int(x), self.height())

    def mousePressEvent(self, e: QMouseEvent) -> None:
        i = self.index_at(e.position().x())
        if i >= 0:
            self.page_clicked.emit(i)


BLOCK_FIELDS = (
    ("style", N_("Style")),
    ("rows", N_("Rows")),
    ("cells", N_("Names")),
    ("tails", N_("Tails")),
    ("pictures", N_("Pictures")),
    ("spine", N_("Spine")),
)
PAGE_FIELDS = (
    ("name", N_("Page")),
    ("kind", N_("Type")),
    ("style", N_("Style")),
    ("rows", N_("Rows")),
    ("blocks", N_("Blocks")),
    ("duration", N_("Duration")),
    ("background", N_("Background")),
    ("transition", N_("Into the next")),
)


class PropertiesPanel(QWidget):
    """What the chosen page, and the chosen block on it, are made of (read only: the spreadsheet
    is where they change)."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("credits.properties")
        outer = QVBoxLayout(self)
        outer.setContentsMargins(4, 0, 4, 0)
        form = QFormLayout()
        outer.addLayout(form)
        self.page_title = QLabel(f"<b>{tr('Page')}</b>", self)
        form.addRow(self.page_title)
        self.fields: dict[str, QLabel] = {}
        for key, label in PAGE_FIELDS:
            self.fields[f"page.{key}"] = self._value(form, tr(label), f"credits.properties.page.{key}")
        self.block_title = QLabel(f"<b>{tr('Block')}</b>", self)
        form.addRow(self.block_title)
        for key, label in BLOCK_FIELDS:
            self.fields[f"block.{key}"] = self._value(form, tr(label), f"credits.properties.block.{key}")
        self.block_text = QLabel(self)
        self.block_text.setObjectName("credits.properties.block.text")
        self.block_text.setWordWrap(False)
        self.block_text.setAlignment(Qt.AlignmentFlag.AlignTop | Qt.AlignmentFlag.AlignLeft)
        self.block_text.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        outer.addWidget(self.block_text)
        hint = QLabel(tr("Change them in the spreadsheet; the credits follow when you save it."), self)
        hint.setWordWrap(True)
        hint.setEnabled(False)
        outer.addWidget(hint)
        outer.addStretch(1)

    def _value(self, form: QFormLayout, label: str, name: str) -> QLabel:
        v = QLabel(self)
        v.setObjectName(name)
        v.setWordWrap(True)
        v.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        form.addRow(label, v)
        return v

    def value(self, key: str) -> str:
        w = self.fields.get(key)
        return w.text() if w else ""

    def show_page(self, values: dict[str, str]) -> None:
        for key, _l in PAGE_FIELDS:
            self.fields[f"page.{key}"].setText(values.get(key, ""))

    def show_block(self, block: dict[str, Any] | None) -> None:
        b = block if isinstance(block, dict) else {}
        row, last = b.get("row"), b.get("lastRow")
        rows = (f"{row}–{last}" if isinstance(last, int) and isinstance(row, int) and last > row else str(row)) if isinstance(row, int) else ""
        count = b.get("bodyCount")
        values = {
            "style": str(b.get("style") or ""),
            "rows": rows,
            "cells": str(count) if isinstance(count, int) else "",
            "tails": str(b.get("tails")) if isinstance(b.get("tails"), int) and b.get("tails") else "",
            "pictures": str(b.get("pictures")) if isinstance(b.get("pictures"), int) and b.get("pictures") else "",
            "spine": (f"{_num(b.get('spine')):+g} px" + (f" ({tr('parallel')})" if b.get("parallel") else "")) if b else "",
        }
        for key, _l in BLOCK_FIELDS:
            self.fields[f"block.{key}"].setText(values[key])
        lines = [str(x) for x in (b.get("body") or []) if isinstance(x, str)]
        more = (count - len(lines)) if isinstance(count, int) else 0
        text = "\n".join(([str(b["head"])] if b.get("head") else []) + lines)
        if more > 0:
            text += "\n" + tr("{n} more").replace("{n}", str(more))
        self.block_text.setText(text)


def block_label(block: dict[str, Any]) -> str:
    """A block in a line: its head, or its first name, and how many names follow."""
    head = str(block.get("head") or "")
    body = [str(x) for x in block.get("body") or [] if isinstance(x, str)]
    first = head or (body[0] if body else "")
    count = block.get("bodyCount") if isinstance(block.get("bodyCount"), int) else len(body)
    extra = count - (0 if head else 1)
    text = first or tr("(empty block)")
    return f"{text}  (+{extra})" if extra > 0 else text
