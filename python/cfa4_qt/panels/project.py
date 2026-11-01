"""Project panel (egui's ``panels/project.rs`` and ``project_views.rs``): the project's bins and
items in List view (the Metadata Display columns, sortable and resizable, bins twirled open) or
Icon view (thumbnail cards with hover scrub, the In/Out bar and badges, Sort Icons) or Freeform
view (cards placed freely, stacks, arrangements), with the
breadcrumb and Up one level, the search and item count, the Preview Area, inline rename (F2, a
slow second click, the menus), bins opened in place, in a tab or in a window, and the footer and
panel menu (views, thumbnail size, New Item, New Bin, Find, Clear, Font Size, view presets).

What a view shows comes from ``project.panel`` (``apps/cfa4-cli/src/serve_project.rs``); the
view settings are engine preferences (``project.view.*``, ``project.columns.*``, ``project.sort``,
``project.viewPreset.*``). Without that method (an older engine) the bin tree of
``project.inspect`` is shown as a plain list.
"""

from __future__ import annotations

import math
from pathlib import Path
from typing import Any, ClassVar

from PySide6.QtCore import (
    QEvent,
    QMimeData,
    QModelIndex,
    QPoint,
    QPointF,
    QRect,
    QRectF,
    QSize,
    Qt,
    QTimer,
    QUrl,
    Signal,
)
from PySide6.QtGui import (
    QAction,
    QColor,
    QDesktopServices,
    QDrag,
    QFont,
    QIcon,
    QImage,
    QKeyEvent,
    QPainter,
    QPainterPath,
    QPen,
    QPixmap,
)
from PySide6.QtWidgets import (
    QAbstractItemView,
    QApplication,
    QCheckBox,
    QDialog,
    QDialogButtonBox,
    QFormLayout,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QMenu,
    QPushButton,
    QScrollArea,
    QSlider,
    QSpinBox,
    QStackedWidget,
    QStyle,
    QStyledItemDelegate,
    QStyleOptionViewItem,
    QTabBar,
    QToolButton,
    QTreeWidget,
    QTreeWidgetItem,
    QTreeWidgetItemIterator,
    QVBoxLayout,
    QWidget,
)

from ..i18n import N_, fmt, tr
from ..timecode import TICKS_PER_SECOND
from .media_browser import FILES_MIME, dropped_files
from .timeline import ITEM_MIME, LABEL_COLORS
from .timeline_media import TimelineMedia

ITEM_ROLE = Qt.ItemDataRole.UserRole
BIN_ROLE = Qt.ItemDataRole.UserRole + 1
TYPE_ROLE = Qt.ItemDataRole.UserRole + 2
ROW_ROLE = Qt.ItemDataRole.UserRole + 3
BIN_MIME = "application/x-cfa4-bin"  # comma-separated bin ids dragged in the Project panel

BIN_COLOR = QColor(237, 150, 58)
OFFLINE = QColor(0xE8, 0x5C, 0x5C)
INOUT = QColor(0x5A, 0x9B, 0xF0)
MIN_ICON, MAX_ICON = 48, 400

NEW_ITEMS = (
    (N_("Sequence…"), "file.newSequence"),
    (N_("Adjustment Layer…"), "file.newAdjustmentLayer"),
    (N_("Bars and Tone…"), "file.newBarsAndTone"),
    (N_("Black Video…"), "file.newBlackVideo"),
    (N_("Color Matte…"), "file.newColorMatte"),
    (N_("Universal Counting Leader…"), "file.newCountingLeader"),
    (N_("Transparent Video…"), "file.newTransparentVideo"),
    (N_("Demo Footage"), "file.importDemoFootage"),
)
SORT_ICONS = (
    N_("Name"),
    N_("Label"),
    N_("Media Type"),
    N_("Frame Rate"),
    N_("Media Start"),
    N_("Media End"),
    N_("Media Duration"),
    N_("Video Info"),
    N_("Audio Info"),
    N_("Date Created"),
)
FONT_SIZES = (("small", N_("Small"), 10.5), ("medium", N_("Medium (default)"), 12.0), ("large", N_("Large"), 14.0), ("extraLarge", N_("Extra Large"), 16.0))
TOGGLES = (
    ("previewArea", N_("Preview Area"), ""),
    ("thumbnails", N_("Thumbnails"), ""),
    ("thumbnailsShowEffects", N_("Thumbnails show effects applied"), ""),
    ("hoverScrub", N_("Hover Scrub"), "Shift+H"),
    ("thumbnailControlsAllDevices", N_("Thumbnail controls for all pointing devices"), ""),
)
OPEN_HOW = (("inPlace", N_("Open in Place")), ("newTab", N_("Open in New Tab")), ("newWindow", N_("Open in New Window")))
# Settings ▸ General ▸ Bins spelling -> ours
HOW = {"openInPlace": "inPlace", "openNewTab": "newTab", "openNewWindow": "newWindow"}


def _duration(ticks: Any) -> str:
    try:
        secs = int(ticks) / TICKS_PER_SECOND
    except (TypeError, ValueError):
        return ""
    if secs <= 0:
        return ""
    m, s = divmod(secs, 60)
    h, m = divmod(int(m), 60)
    return f"{h:d}:{m:02d}:{s:05.2f}" if h else f"{m:d}:{s:05.2f}"


def card_duration(ticks: Any) -> str:
    """egui's card duration: m:ss, or h:mm:ss from an hour."""
    try:
        secs = max(0, int(int(ticks) / TICKS_PER_SECOND))
    except (TypeError, ValueError):
        return ""
    return f"{secs // 3600}:{secs // 60 % 60:02d}:{secs % 60:02d}" if secs >= 3600 else f"{secs // 60}:{secs % 60:02d}"


def _int(v: Any) -> int | None:
    return v if isinstance(v, int) and not isinstance(v, bool) else None


def label_color(row: dict[str, Any]) -> QColor:
    c = QColor(str(row.get("labelColor") or ""))
    if c.isValid():
        return c
    return QColor(LABEL_COLORS.get(str(row.get("label") or "").lower(), "#8f6fd8"))


def fallback_rows(node: Any, depth: int = 0, expanded: set[int] | None = None, needle: str = "") -> list[dict[str, Any]]:
    """The List view's rows from ``project.inspect``'s bin tree (an engine without
    ``project.panel``): every bin open, as the panel showed before."""
    out: list[dict[str, Any]] = []
    for child in (node or {}).get("children") or [] if isinstance(node, dict) else []:
        if not isinstance(child, dict):
            continue
        if "item" in child:
            if needle and needle not in str(child.get("name") or "").lower():
                continue
            out.append(
                {
                    "item": child.get("item"),
                    "name": str(child.get("name") or ""),
                    "depth": depth,
                    "type": str(child.get("type") or ""),
                    "sequence": str(child.get("type") or "").lower() == "sequence",
                    "label": child.get("label"),
                    "duration": child.get("duration"),
                    "cells": {"Media Type": str(child.get("type") or ""), "Media Duration": _duration(child.get("duration"))},
                }
            )
        elif depth < 64:
            out.append({"bin": child.get("id"), "name": str(child.get("name") or ""), "depth": depth, "open": True})
            out.extend(fallback_rows(child, depth + 1, expanded, needle))
    return out


# ----------------------------------------------------------------------------------- icons
_ICONS: dict[tuple[str, str, int], QIcon] = {}


def kind_icon(kind: str, color: QColor | None = None, size: int = 16) -> QIcon:
    """The small glyph of an item kind or a bin, drawn in code (film, audio, image, sequence,
    adjust, bin)."""
    key = (kind, (color or QColor()).name(), size)
    icon = _ICONS.get(key)
    if icon is not None:
        return icon
    pm = QPixmap(size, size)
    pm.fill(Qt.GlobalColor.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.RenderHint.Antialiasing)
    c = color or QColor(0x9A, 0xA4, 0xB2)
    s = size / 16.0
    p.setPen(QPen(c, 1.2 * s))
    if kind == "bin":
        path = QPainterPath()
        path.moveTo(1.5 * s, 4 * s)
        path.lineTo(6 * s, 4 * s)
        path.lineTo(7.5 * s, 5.5 * s)
        path.lineTo(14.5 * s, 5.5 * s)
        path.lineTo(14.5 * s, 13 * s)
        path.lineTo(1.5 * s, 13 * s)
        path.closeSubpath()
        p.fillPath(path, c)
    elif kind == "audio":
        for i, h in enumerate((4, 8, 12, 7, 3)):
            x = (3 + i * 2.5) * s
            p.drawLine(QPointF(x, (8 - h / 2) * s), QPointF(x, (8 + h / 2) * s))
    elif kind == "image":
        p.drawRect(QRectF(2 * s, 3 * s, 12 * s, 10 * s))
        p.drawPolyline([QPointF(3 * s, 12 * s), QPointF(7 * s, 7 * s), QPointF(10 * s, 10 * s), QPointF(13 * s, 6 * s)])
    elif kind == "sequence":
        for i, w in enumerate((10, 7, 12)):
            p.fillRect(QRectF(2 * s, (3.5 + i * 3.5) * s, w * s, 2.2 * s), c)
    elif kind == "adjust":
        p.drawEllipse(QRectF(2.5 * s, 2.5 * s, 11 * s, 11 * s))
        half = QPainterPath()
        half.moveTo(8 * s, 2.5 * s)
        half.arcTo(QRectF(2.5 * s, 2.5 * s, 11 * s, 11 * s), 90, 180)
        half.closeSubpath()
        p.fillPath(half, c)
    else:  # film
        p.drawRect(QRectF(2.5 * s, 2.5 * s, 11 * s, 11 * s))
        for y in (4.5, 7.5, 10.5):
            p.fillRect(QRectF(3.5 * s, y * s, 1.5 * s, 1.2 * s), c)
            p.fillRect(QRectF(11 * s, y * s, 1.5 * s, 1.2 * s), c)
    p.end()
    icon = QIcon(pm)
    _ICONS[key] = icon
    return icon


def chip_icon(color: QColor, kind: str, size: int = 16) -> QIcon:
    """The Name column's lead: the label chip, then the type glyph."""
    pm = QPixmap(size * 2 + 4, size)
    pm.fill(Qt.GlobalColor.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.RenderHint.Antialiasing)
    p.setBrush(color)
    p.setPen(Qt.PenStyle.NoPen)
    p.drawRoundedRect(QRectF(2, size / 2 - 5, 10, 10), 2, 2)
    kind_icon(kind, None, size).paint(p, QRect(size + 4, 0, size, size))
    p.end()
    return QIcon(pm)


# ----------------------------------------------------------------------------------- drag and drop
def drag_mime(items: list[int], bins: list[int]) -> QMimeData:
    mime = QMimeData()
    if items:
        mime.setData(ITEM_MIME, ",".join(str(i) for i in items).encode())
    if bins:
        mime.setData(BIN_MIME, ",".join(str(b) for b in bins).encode())
    return mime


def _ids(mime: QMimeData, kind: str) -> list[int]:
    if not mime.hasFormat(kind):
        return []
    out = []
    for part in bytes(mime.data(kind).data()).decode(errors="replace").split(","):
        try:
            out.append(int(part))
        except ValueError:
            continue
    return out


class _Drops:
    """Drops on a view: files are imported, project items and bins move into the bin under the
    pointer (a bin dropped on empty space moves into the bin the view shows)."""

    files_dropped: Any
    moved: Any

    def _files(self, e: Any) -> bool:
        m = e.mimeData()
        return not m.hasFormat(ITEM_MIME) and not m.hasFormat(BIN_MIME) and (m.hasFormat(FILES_MIME) or m.hasUrls())

    def _ours(self, e: Any) -> bool:
        m = e.mimeData()
        return m.hasFormat(ITEM_MIME) or m.hasFormat(BIN_MIME)

    def _target_bin(self, e: Any) -> int | None:
        raise NotImplementedError

    def accept_drag(self, e: Any) -> bool:
        if self._files(e) or self._ours(e):
            e.acceptProposedAction()
            return True
        return False

    def drop(self, e: Any) -> bool:
        if self._files(e):
            paths = dropped_files(e.mimeData())
            if paths:
                self.files_dropped.emit(paths)
            e.acceptProposedAction()
            return True
        if self._ours(e):
            m = e.mimeData()
            items, bins = _ids(m, ITEM_MIME), _ids(m, BIN_MIME)
            target = self._target_bin(e)
            if target is None:
                items = []  # dropped on empty space: only bins move (out of their parent)
            self.moved.emit(items + bins, target)
            e.acceptProposedAction()
            return True
        return False


class ProjectTree(QTreeWidget, _Drops):
    """The List view."""

    files_dropped = Signal(list)  # paths dragged from the Media Browser or a file manager
    moved = Signal(list, object)  # (item and bin ids, target bin or None for the shown bin)

    def mimeData(self, items: list[QTreeWidgetItem]) -> QMimeData:
        return drag_mime(
            [i.data(0, ITEM_ROLE) for i in items if i.data(0, ITEM_ROLE) is not None],
            [i.data(0, BIN_ROLE) for i in items if i.data(0, BIN_ROLE) is not None],
        )

    def mimeTypes(self) -> list[str]:
        return [ITEM_MIME, BIN_MIME]

    def _target_bin(self, e: Any) -> int | None:
        it = self.itemAt(e.position().toPoint())
        while it is not None and it.data(0, BIN_ROLE) is None:
            it = it.parent()
        return it.data(0, BIN_ROLE) if it is not None else None

    def dragEnterEvent(self, e: Any) -> None:
        if not self.accept_drag(e):
            super().dragEnterEvent(e)

    def dragMoveEvent(self, e: Any) -> None:
        if not self.accept_drag(e):
            super().dragMoveEvent(e)

    def dropEvent(self, e: Any) -> None:
        if not self.drop(e):
            super().dropEvent(e)


class IconList(QListWidget, _Drops):
    """The Icon view: cards in a wrapping grid."""

    files_dropped = Signal(list)
    moved = Signal(list, object)
    hovered = Signal(object, object)  # item id (or None) and the media time (ticks: 64-bit) under the pointer

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setMouseTracking(True)

    def mimeData(self, items: list[QListWidgetItem]) -> QMimeData:
        return drag_mime(
            [i.data(ITEM_ROLE) for i in items if i.data(ITEM_ROLE) is not None],
            [i.data(BIN_ROLE) for i in items if i.data(BIN_ROLE) is not None],
        )

    def mimeTypes(self) -> list[str]:
        return [ITEM_MIME, BIN_MIME]

    def _target_bin(self, e: Any) -> int | None:
        it = self.itemAt(e.position().toPoint())
        return it.data(BIN_ROLE) if it is not None else None

    def dragEnterEvent(self, e: Any) -> None:
        if not self.accept_drag(e):
            super().dragEnterEvent(e)

    def dragMoveEvent(self, e: Any) -> None:
        if not self.accept_drag(e):
            super().dragMoveEvent(e)

    def dropEvent(self, e: Any) -> None:
        if not self.drop(e):
            super().dropEvent(e)

    def mouseMoveEvent(self, e: Any) -> None:
        super().mouseMoveEvent(e)
        pos = e.position().toPoint()
        it = self.itemAt(pos)
        row = it.data(ROW_ROLE) if it is not None else None
        if not isinstance(row, dict) or row.get("item") is None:
            self.hovered.emit(None, 0)
            return
        rect = self.visualItemRect(it)
        dur = _int(row.get("duration")) or 0
        frame = max(1, _int(row.get("frame")) or 1)
        f = min(1.0, max(0.0, (pos.x() - rect.left() - 6) / max(1.0, rect.width() - 12)))
        t = int(dur * f) // frame * frame
        self.hovered.emit(row.get("item"), max(0, min(t, dur - frame)))

    def leaveEvent(self, e: Any) -> None:
        super().leaveEvent(e)
        self.hovered.emit(None, 0)


def paint_clip(p: QPainter, view: BinView, row: dict[str, Any], th: QRectF, pal: Any, show_name: bool = True, show_dur: bool = True, editing: bool = False) -> None:
    """A clip card's thumbnail (hover scrub, poster frame), In/Out bar, badges, name and duration
    (egui's ``card``), for the Icon and Freeform views."""
    text_y = th.bottom() + 12
    p.fillRect(th, QColor(12, 12, 12))
    item = row.get("item")
    hover = view.hover if view.hover_scrub else None
    at = hover[1] if hover is not None and hover[0] == item else None
    img = view.thumbnail(row, at, int(th.height()))
    if img is not None and not img.isNull():
        scale = min(th.width() / max(1, img.width()), th.height() / max(1, img.height()))
        w, h = img.width() * scale, img.height() * scale
        p.drawImage(QRectF(th.center().x() - w / 2, th.center().y() - h / 2, w, h), img)
    else:
        g = min(28.0, th.height()) * 0.8
        kind_icon(str(row.get("icon") or "film"), pal.placeholderText().color(), 32).paint(
            p, QRect(int(th.center().x() - g / 2), int(th.center().y() - g / 2), int(g), int(g))
        )
    dur = _int(row.get("duration")) or 0
    mark_in, mark_out = _int(row.get("in")), _int(row.get("out"))
    if (mark_in is not None or mark_out is not None) and dur > 0:
        frame = _int(row.get("frame")) or 0

        def fx(t: int) -> float:
            return th.left() + th.width() * min(1.0, max(0.0, t / dur))

        a = fx(mark_in or 0)
        b = fx(mark_out + frame if mark_out is not None else dur)
        p.fillRect(QRectF(a, th.bottom() - 4, max(2.0, b - a), 4), INOUT)
    if at is not None and dur > 0:
        x = th.left() + th.width() * at / dur
        p.setPen(QPen(INOUT, 1))
        p.drawLine(QPointF(x, th.top()), QPointF(x, th.bottom()))
    bx = th.right() - 4
    for has, kind in ((row.get("hasAudio"), "audio"), (row.get("hasVideo") and not row.get("sequence"), "film"), (row.get("sequence"), "sequence")):
        if has:
            br = QRectF(bx - 15, th.bottom() - 17, 15, 12)
            p.fillRect(br, QColor(0, 0, 0, 160))
            kind_icon(kind, QColor(0x8F, 0xB8, 0xF5), 16).paint(p, br.adjusted(1.5, 0, -1.5, 0).toRect())
            bx -= 17
    offline = row.get("offline")
    if offline:
        band = QRectF(th.left(), th.bottom() - 20, th.width(), 20)
        p.fillRect(band, QColor(0x5C, 0x10, 0x16, 230))
        p.setPen(QColor("white"))
        p.drawText(band.adjusted(8, 0, 0, 0), Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter, tr(str(offline)))
    if row.get("proxy"):
        pr = QRectF(th.right() - 22, th.top() + 4, 18, 14)
        p.fillRect(pr, QColor(70, 70, 70))
        p.setPen(QColor("white"))
        p.drawText(pr, Qt.AlignmentFlag.AlignCenter, "P")
    dw = 0
    if show_dur:
        dtext = card_duration(dur)
        dw = p.fontMetrics().horizontalAdvance(dtext)
        p.setPen(pal.placeholderText().color())
        p.drawText(QRectF(th.right() - dw, text_y - 9, dw, 18), Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter, dtext)
    if show_name and not editing:
        p.fillRect(QRectF(th.left(), text_y - 5, 8, 10), label_color(row))
        p.setPen(OFFLINE if offline else pal.text().color())
        name_r = QRectF(th.left() + 12, text_y - 9, max(0.0, th.width() - dw - 18), 18)
        name = p.fontMetrics().elidedText(str(row.get("name") or ""), Qt.TextElideMode.ElideRight, int(name_r.width()))
        p.drawText(name_r, Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter, name)


class CardDelegate(QStyledItemDelegate):
    """Paints an Icon view card (egui's ``card`` / ``bin_card``) and places the rename field
    under its thumbnail."""

    def __init__(self, view: BinView) -> None:
        super().__init__(view)
        self.view = view

    def sizeHint(self, _option: QStyleOptionViewItem, _index: QModelIndex) -> QSize:
        s = self.view.icon_size
        return QSize(s + 20, round(s * 9 / 16) + 34)

    def _thumb_rect(self, r: QRect) -> QRectF:
        s = self.view.icon_size
        return QRectF(r.left() + 10, r.top() + 6, s, s * 9 / 16)

    def updateEditorGeometry(self, editor: QWidget, option: QStyleOptionViewItem, _index: QModelIndex) -> None:
        th = self._thumb_rect(option.rect)
        editor.setGeometry(QRect(int(th.left()), int(th.bottom()) + 2, int(th.width()), 20))

    def paint(self, p: QPainter, option: QStyleOptionViewItem, index: QModelIndex) -> None:
        row = index.data(ROW_ROLE)
        if not isinstance(row, dict):
            return
        pal = option.palette
        selected = bool(option.state & QStyle.StateFlag.State_Selected)
        th = self._thumb_rect(option.rect)
        bg = QRectF(th.left() - 6, th.top() - 4, th.width() + 12, th.height() + 26)
        p.save()
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        p.setPen(QPen(pal.highlight().color(), 1.5) if selected else Qt.PenStyle.NoPen)
        p.setBrush(pal.alternateBase() if selected else pal.base())
        p.drawRoundedRect(bg, 4, 4)
        font = QFont(option.font)
        font.setPixelSize(max(8, round(self.view.font_points - 1)))
        p.setFont(font)
        if row.get("bin") is not None:
            p.fillRect(th, QColor(26, 26, 26))
            g = th.height() * 0.6
            kind_icon("bin", BIN_COLOR, 64).paint(p, QRect(int(th.center().x() - g / 2), int(th.center().y() - g / 2), int(g), int(g)))
            if not self.view.editing(index):
                p.setPen(pal.text().color())
                name_r = QRectF(th.left(), th.bottom() + 2, th.width() - 50, 18)
                p.drawText(name_r, Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter, str(row.get("name") or ""))
                p.setPen(pal.placeholderText().color())
                n = _int(row.get("count")) or 0
                p.drawText(QRectF(th.left(), th.bottom() + 2, th.width(), 18), Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter, fmt("{n} items", n=n))
            p.restore()
            return
        paint_clip(p, self.view, row, th, pal, editing=self.view.editing(index))
        p.restore()


class FreeformCanvas(QWidget):
    """Freeform view (egui's ``freeform_view``): clip cards where the user put them (the others on
    a grid below), dragged to move with the selection or their stack, stacks with a ×N badge,
    marquee selection, the snap grid, hover scrub and inline rename. Positions, sizes, stacks and
    arrangements are item metadata the engine keeps (``project.freeform.*``)."""

    PAD = 10.0

    def __init__(self, view: BinView) -> None:
        super().__init__(view)
        self.view = view
        self.setMouseTracking(True)
        self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)
        self.cards: list[dict[str, Any]] = []
        self.rows: dict[int, dict[str, Any]] = {}
        self.opts: dict[str, Any] = {}
        self.selected: set[int] = set()
        self._press: QPointF | None = None
        self._drag: int | None = None  # the card being moved
        self._offset = QPointF()
        self._marquee: QRectF | None = None
        self._editor: QLineEdit | None = None

    # ----- data -----
    def set_data(self, cards: Any, rows: list[dict[str, Any]], opts: Any, selection: set[int]) -> None:
        self.rows = {r["item"]: r for r in rows if isinstance(r.get("item"), int)}
        self.cards = [c for c in cards or [] if self._card_ok(c) and c["item"] in self.rows] if isinstance(cards, list) else []
        self.opts = opts if isinstance(opts, dict) else {}
        self.selected = set(selection)
        w = h = 0.0
        for c in self.cards:
            w = max(w, c["x"] + c["size"] + 40)
            h = max(h, c["y"] + c["size"] * 9 / 16 + 60)
        self.setMinimumSize(int(min(100_000, w + self.PAD)), int(min(100_000, h + self.PAD)))
        self.update()

    @staticmethod
    def _card_ok(c: Any) -> bool:
        if not isinstance(c, dict) or not isinstance(c.get("item"), int):
            return False
        for k in ("x", "y", "size"):
            v = c.get(k)
            if not isinstance(v, (int, float)) or isinstance(v, bool) or not math.isfinite(v) or abs(v) > 100_000:
                return False
        return c["size"] > 0

    def _opt(self, key: str, default: Any) -> Any:
        v = self.opts.get(key)
        if isinstance(default, bool):
            return v if isinstance(v, bool) else default
        return v if isinstance(v, (int, float)) and not isinstance(v, bool) and math.isfinite(v) else default

    def stack_of(self, item: int) -> int | None:
        c = next((c for c in self.cards if c["item"] == item), None)
        s = c.get("stack") if c is not None else None
        return s if isinstance(s, int) and not isinstance(s, bool) else None

    def moving(self, item: int) -> list[int]:
        """The cards a drag of ``item`` moves: it, the selection it is in and its stack, the
        stack's first card leading so the others keep their offsets."""
        items = [item]
        if item in self.selected:
            items += [i for i in sorted(self.selected) if i != item and i in self.rows]
        stack = self.stack_of(item)
        if stack is not None:
            items += [c["item"] for c in self.cards if c.get("stack") == stack and c["item"] not in items]
            if stack in items:
                items.remove(stack)
                items.insert(0, stack)
        return items

    # ----- geometry -----
    def thumb_rect(self, c: dict[str, Any], off: QPointF | None = None) -> QRectF:
        o = off or QPointF()
        return QRectF(self.PAD + c["x"] + o.x(), self.PAD + c["y"] + o.y(), c["size"], c["size"] * 9 / 16)

    def card_rect(self, c: dict[str, Any], off: QPointF | None = None) -> QRectF:
        th = self.thumb_rect(c, off)
        return QRectF(th.left() - 6, th.top() - 6, th.width() + 12, th.height() + 28)

    def _order(self) -> list[dict[str, Any]]:
        # a stack's first card is drawn last, on top of the cards stacked behind it
        return sorted(self.cards, key=lambda c: c.get("stack") == c["item"])

    def card_at(self, pos: QPointF) -> dict[str, Any] | None:
        for c in reversed(self._order()):
            if self.card_rect(c, self._off(c)).contains(pos):
                return c
        return None

    def _off(self, c: dict[str, Any]) -> QPointF:
        if self._drag is not None and c["item"] in self.moving(self._drag):
            return self._offset
        return QPointF()

    # ----- painting -----
    def paintEvent(self, _e: Any) -> None:
        p = QPainter(self)
        pal = self.palette()
        p.fillRect(self.rect(), pal.base())
        if self._opt("snap", False):
            g = max(4.0, float(self._opt("grid", 20.0)))
            grid = QColor(pal.mid().color())
            grid.setAlpha(90)
            p.setPen(QPen(grid, 0.5))
            x = self.PAD
            while x < self.width():
                p.drawLine(QPointF(x, 0), QPointF(x, self.height()))
                x += g
            y = self.PAD
            while y < self.height():
                p.drawLine(QPointF(0, y), QPointF(self.width(), y))
                y += g
        counts: dict[int, int] = {}
        for c in self.cards:
            st = c.get("stack")
            if isinstance(st, int):
                counts[st] = counts.get(st, 0) + 1
        font = QFont(self.font())
        font.setPixelSize(max(8, round(self.view.font_points - 1)))
        p.setFont(font)
        names, durs = bool(self._opt("showNames", True)), bool(self._opt("showDurations", True))
        for c in self._order():
            item = c["item"]
            row = self.rows.get(item)
            if row is None:
                continue
            th = self.thumb_rect(c, self._off(c))
            p.save()
            p.setRenderHint(QPainter.RenderHint.Antialiasing)
            selected = item in self.selected
            p.setPen(QPen(pal.highlight().color(), 1.5) if selected else Qt.PenStyle.NoPen)
            p.setBrush(pal.alternateBase() if selected else pal.window())
            p.drawRoundedRect(QRectF(th.left() - 6, th.top() - 6, th.width() + 12, th.height() + 28), 4, 4)
            editing = self._editor is not None and self._editor.property("item") == item
            paint_clip(p, self.view, row, th, pal, names, durs, editing)
            if c.get("stack") == item and counts.get(item):
                br = QRectF(th.left() + 4, th.top() + 4, 26, 16)
                p.setPen(Qt.PenStyle.NoPen)
                p.setBrush(pal.highlight())
                p.drawRoundedRect(br, 3, 3)
                p.setPen(QColor("white"))
                p.drawText(br, Qt.AlignmentFlag.AlignCenter, f"×{counts[item]}")
            p.restore()
        if self._marquee is not None:
            fill = QColor(pal.highlight().color())
            fill.setAlpha(40)
            p.setPen(QPen(pal.highlight().color(), 1))
            p.setBrush(fill)
            p.drawRect(self._marquee.normalized())
        p.end()

    # ----- mouse -----
    def mousePressEvent(self, e: Any) -> None:
        self.setFocus()
        pos = e.position()
        if e.button() != Qt.MouseButton.LeftButton:
            c = self.card_at(pos)
            if c is not None and c["item"] not in self.selected:
                self._select({c["item"]})
            return
        c = self.card_at(pos)
        ctrl = bool(e.modifiers() & (Qt.KeyboardModifier.ControlModifier | Qt.KeyboardModifier.ShiftModifier))
        self._press = pos
        if c is None:
            self._marquee = QRectF(pos, pos)
            if not ctrl:
                self.view.selected_bin = None
                self._select(set())
            return
        item = c["item"]
        if ctrl:
            self._select(self.selected ^ {item})
        elif item not in self.selected:
            self._select({item})
        self._drag = item
        self._offset = QPointF()

    def mouseMoveEvent(self, e: Any) -> None:
        pos = e.position()
        if self._marquee is not None and self._press is not None:
            self._marquee = QRectF(self._press, pos)
            self.update()
            return
        if self._drag is not None and self._press is not None:
            self._offset = pos - self._press
            # leaving the panel turns the move into an item drag (to the Timeline, a monitor)
            port = self.view.free_scroll.viewport()  # the canvas's parent: its position is the scroll
            if not port.rect().contains((pos + QPointF(self.pos())).toPoint()):
                items = self.moving(self._drag)
                self._cancel()
                drag = QDrag(self)
                drag.setMimeData(drag_mime(items, []))
                drag.exec(Qt.DropAction.CopyAction | Qt.DropAction.MoveAction)
                return
            self.update()
            return
        self._hover(pos)

    def _hover(self, pos: QPointF) -> None:
        c = self.card_at(pos)
        row = self.rows.get(c["item"]) if c is not None else None
        if c is None or row is None:
            self.view._hovered(None, 0)
            return
        th = self.thumb_rect(c)
        dur = _int(row.get("duration")) or 0
        frame = max(1, _int(row.get("frame")) or 1)
        if not th.contains(pos) or dur <= 0:
            self.view._hovered(None, 0)
            return
        f = min(1.0, max(0.0, (pos.x() - th.left()) / max(1.0, th.width())))
        t = int(dur * f) // frame * frame
        self.view._hovered(c["item"], max(0, min(t, dur - frame)))

    def mouseReleaseEvent(self, e: Any) -> None:
        if self._marquee is not None:
            r = self._marquee.normalized()
            self._marquee = None
            hit = {c["item"] for c in self.cards if r.intersects(self.card_rect(c))}
            ctrl = bool(e.modifiers() & (Qt.KeyboardModifier.ControlModifier | Qt.KeyboardModifier.ShiftModifier))
            if hit or r.width() > 2 or r.height() > 2:
                self._select(self.selected | hit if ctrl else hit)
            self._press = None
            self.update()
            return
        drag, off = self._drag, self._offset
        self._drag, self._press, self._offset = None, None, QPointF()
        if drag is not None and math.hypot(off.x(), off.y()) > 1.0:
            items = self.moving(drag)
            lead = next((c for c in self.cards if c["item"] == items[0]), None)
            if lead is not None:
                width = self.view.free_scroll.viewport().width()
                self.view.panel.command.emit(
                    "project.freeform.move",
                    {"items": items, "x": max(0.0, lead["x"] + off.x()), "y": max(0.0, lead["y"] + off.y()), "width": width},
                )
                # show it there until the engine's layout comes back
                for c in self.cards:
                    if c["item"] in items:
                        c["x"], c["y"] = max(0.0, c["x"] + off.x()), max(0.0, c["y"] + off.y())
        self.update()

    def _cancel(self) -> None:
        self._drag, self._press, self._offset, self._marquee = None, None, QPointF(), None
        self.update()

    def keyPressEvent(self, e: Any) -> None:
        if e.key() == Qt.Key.Key_Escape and (self._drag is not None or self._marquee is not None):
            # Escape puts the cards back and drops nothing (#580)
            self._cancel()
            return
        if e.key() == Qt.Key.Key_F2 and self.selected:
            self.edit(min(self.selected))
            return
        super().keyPressEvent(e)

    def mouseDoubleClickEvent(self, e: Any) -> None:
        c = self.card_at(e.position())
        if c is None:
            self.view.panel.invoke.emit("file.import")
            return
        self._cancel()
        self.view._open(c["item"], None, self.rows.get(c["item"]))

    def leaveEvent(self, e: Any) -> None:
        super().leaveEvent(e)
        self.view._hovered(None, 0)

    def contextMenuEvent(self, e: Any) -> None:
        c = self.card_at(QPointF(e.pos()))
        panel = self.view.panel
        if c is not None:
            m = panel.card_menu(self.view, c["item"], self.rows.get(c["item"]) or {})
        else:
            m = panel.canvas_menu(self.view)
        m.popup(e.globalPos())

    def _select(self, items: set[int]) -> None:
        self.selected = set(items)
        self.update()
        self.view.panel.command.emit("project.select", {"items": sorted(self.selected)})

    # ----- rename -----
    def edit(self, item: int) -> bool:
        c = next((c for c in self.cards if c["item"] == item), None)
        if c is None:
            return False
        self._end_edit(None)
        th = self.thumb_rect(c)
        ed = QLineEdit(self)
        ed.setObjectName(f"{self.view.prefix}.freeform.rename")
        ed.setProperty("item", item)
        ed.setText(str((self.rows.get(item) or {}).get("name") or ""))
        ed.setGeometry(QRect(int(th.left()), int(th.bottom()) + 2, int(th.width()), 20))
        ed.selectAll()
        ed.editingFinished.connect(lambda e=ed: self._end_edit(e))
        self._editor = ed
        ed.show()
        ed.setFocus()
        self.update()
        return True

    def _end_edit(self, ed: QLineEdit | None) -> None:
        cur = self._editor
        if cur is None or (ed is not None and ed is not cur):
            return
        self._editor = None
        if ed is not None:
            item = cur.property("item")
            name = cur.text().strip()
            if isinstance(item, int) and name and name != (self.rows.get(item) or {}).get("name"):
                self.view.panel.command.emit("clip.rename", {"item": item, "name": name})
        cur.hide()
        cur.deleteLater()
        self.update()


class PreviewArea(QWidget):
    """The selected item's poster frame and its properties (egui's ``preview_area``)."""

    def __init__(self, view: BinView) -> None:
        super().__init__(view)
        self.view = view
        self.row: dict[str, Any] | None = None
        self.setFixedHeight(92)

    def paintEvent(self, _e: Any) -> None:
        p = QPainter(self)
        pal = self.palette()
        r = QRectF(self.rect())
        p.fillRect(r, pal.alternateBase())
        row = self.row
        if row is None:
            p.setPen(pal.placeholderText().color())
            p.drawText(r, Qt.AlignmentFlag.AlignCenter, tr("Select an item to preview it"))
            return
        th = QRectF(6, 6, (r.height() - 12) * 16 / 9, r.height() - 12)
        p.fillRect(th, QColor(10, 10, 10))
        img = self.view.thumbnail(row, None, int(th.height()))
        if img is not None and not img.isNull():
            scale = min(th.width() / max(1, img.width()), th.height() / max(1, img.height()))
            w, h = img.width() * scale, img.height() * scale
            p.drawImage(QRectF(th.center().x() - w / 2, th.center().y() - h / 2, w, h), img)
        cells = row.get("cells") if isinstance(row.get("cells"), dict) else {}
        video = f", {cells.get('Video Info')}" if row.get("hasVideo") and cells.get("Video Info") else ""
        lines = [
            str(row.get("name") or ""),
            f"{tr(str(row.get('type') or ''))}{video}",
            ", ".join(str(x) for x in (cells.get("Media Duration"), cells.get("Frame Rate")) if x),
            str(cells.get("Audio Info") or ""),
            fmt("Video Usage: {vu}  Audio Usage: {au}", vu=row.get("videoUsage", 0), au=row.get("audioUsage", 0)),
        ]
        y = 6.0
        for i, line in enumerate(lines):
            if not line:
                continue
            f = QFont(self.font())
            f.setBold(i == 0)
            p.setFont(f)
            p.setPen(pal.text().color() if i == 0 else pal.placeholderText().color())
            p.drawText(QRectF(th.right() + 10, y, r.width() - th.right() - 16, 15), Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter, line)
            y += 15


# ----------------------------------------------------------------------------------- one view
class BinView(QWidget):
    """One panel instance: the Project panel itself (``prefix`` ``project``) or a bin opened in a
    tab or a window (``projectBin.<k>``)."""

    def __init__(self, panel: ProjectPanel, prefix: str, top: int | None = None, mode: str | None = None, icon_size: int = 110) -> None:
        super().__init__(panel)
        self.panel = panel
        self.prefix = prefix
        self.top = top  # the bin a tab or window opened (None: the project's root)
        self.bin: int | None = top  # the bin shown in place
        self.own_mode = mode  # a tab's own view (None: the panel's View setting)
        self.own_size = icon_size
        self.data: dict[str, Any] = {}
        self._held: Any = None  # project.panel data waiting for an in-place rename to finish
        self._watched: list[QAbstractItemView] = []
        self.expanded: set[int] = set()
        self.selected_bin: int | None = None
        self.hover: tuple[Any, int] | None = None
        self.arrangements: list[str] = []  # Freeform: the bin's saved arrangements
        self._updating = False
        self._shown: Any = None
        self.setObjectName(f"{prefix}.view")

        head = QHBoxLayout()
        head.setContentsMargins(4, 0, 4, 0)
        self.up = QToolButton(self)
        self.up.setObjectName(f"{prefix}.up")
        self.up.setText("⤴")
        self.up.setAutoRaise(True)
        self.up.setToolTip(tr("Up one level"))
        self.up.clicked.connect(self.go_up)
        self.crumb = QLabel(self)
        self.crumb.setObjectName(f"{prefix}.breadcrumb")
        self.crumb.setTextInteractionFlags(Qt.TextInteractionFlag.NoTextInteraction)
        head.addWidget(self.up)
        head.addWidget(self.crumb, 1)
        self.menu_button = QToolButton(self)
        self.menu_button.setObjectName(f"{prefix}.menu")
        self.menu_button.setText("≡")
        self.menu_button.setAutoRaise(True)
        self.menu_button.setPopupMode(QToolButton.ToolButtonPopupMode.InstantPopup)
        self.menu_button.setMenu(QMenu(self.menu_button))
        self.menu_button.menu().aboutToShow.connect(lambda: panel.fill_panel_menu(self.menu_button.menu(), self))
        head.addWidget(self.menu_button)

        search = QHBoxLayout()
        search.setContentsMargins(4, 0, 4, 0)
        self.filter = QLineEdit(self)
        self.filter.setObjectName(f"{prefix}.search")
        self.filter.setPlaceholderText(tr("Search"))
        self.filter.setClearButtonEnabled(True)
        self.filter.textChanged.connect(lambda _t: self._search_timer.start())
        self._search_timer = QTimer(self)
        self._search_timer.setSingleShot(True)
        self._search_timer.setInterval(150)
        self._search_timer.timeout.connect(self.refresh)
        find = QToolButton(self)
        find.setObjectName(f"{prefix}.find")
        find.setText("🔍")
        find.setAutoRaise(True)
        find.setToolTip(tr("Find…"))
        find.clicked.connect(lambda: panel.invoke.emit("edit.find"))
        self.count = QLabel(self)
        self.count.setObjectName(f"{prefix}.count")
        search.addWidget(self.filter, 1)
        search.addWidget(find)
        search.addWidget(self.count)

        self.preview = PreviewArea(self)
        self.preview.setObjectName(f"{prefix}.previewArea")
        self.preview.hide()

        self.tree = ProjectTree(self)
        self.tree.setObjectName(f"{prefix}.list" if prefix != "project" else "project.tree")
        self.tree.setSelectionMode(QAbstractItemView.SelectionMode.ExtendedSelection)
        self.tree.setDragEnabled(True)
        self.tree.setDragDropMode(QAbstractItemView.DragDropMode.DragDrop)
        self.tree.setDefaultDropAction(Qt.DropAction.MoveAction)
        self.tree.setAcceptDrops(True)
        self.tree.viewport().setAcceptDrops(True)
        self.tree.setEditTriggers(QAbstractItemView.EditTrigger.EditKeyPressed | QAbstractItemView.EditTrigger.SelectedClicked)
        self.tree.setAlternatingRowColors(True)
        self.tree.setIconSize(QSize(36, 16))
        self.tree.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
        self.tree.customContextMenuRequested.connect(lambda pos: self._context_menu(self.tree, pos))
        header = self.tree.header()
        header.setSectionsClickable(True)
        header.setSortIndicatorShown(True)
        header.setStretchLastSection(False)
        header.sectionClicked.connect(self._sort_by)
        header.sectionResized.connect(self._column_resized)
        header.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
        header.customContextMenuRequested.connect(self._header_menu)
        self._resized: dict[str, int] = {}
        self._resize_timer = QTimer(self)
        self._resize_timer.setSingleShot(True)
        self._resize_timer.setInterval(400)
        self._resize_timer.timeout.connect(self._send_resizes)
        self.tree.itemExpanded.connect(lambda it: self._twirl(it, True))
        self.tree.itemCollapsed.connect(lambda it: self._twirl(it, False))

        self.icons = IconList(self)
        self.icons.setObjectName(f"{prefix}.icons")
        self.icons.setViewMode(QListWidget.ViewMode.IconMode)
        self.icons.setResizeMode(QListWidget.ResizeMode.Adjust)
        self.icons.setMovement(QListWidget.Movement.Static)
        self.icons.setUniformItemSizes(True)
        self.icons.setWrapping(True)
        self.icons.setSpacing(4)
        self.icons.setSelectionMode(QAbstractItemView.SelectionMode.ExtendedSelection)
        self.icons.setDragEnabled(True)
        self.icons.setDragDropMode(QAbstractItemView.DragDropMode.DragDrop)
        self.icons.setAcceptDrops(True)
        self.icons.viewport().setAcceptDrops(True)
        self.icons.setEditTriggers(QAbstractItemView.EditTrigger.EditKeyPressed | QAbstractItemView.EditTrigger.SelectedClicked)
        self.icons.setItemDelegate(CardDelegate(self))
        self.icons.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
        self.icons.customContextMenuRequested.connect(lambda pos: self._context_menu(self.icons, pos))
        self.icons.hovered.connect(self._hovered)

        for v in (self.tree, self.icons):
            v.files_dropped.connect(lambda paths: panel.command.emit("mediaBrowser.import", {"paths": paths}))
            v.moved.connect(self._moved)
        self.tree.itemDoubleClicked.connect(lambda it, _c: self._open(it.data(0, ITEM_ROLE), it.data(0, BIN_ROLE), it.data(0, ROW_ROLE)))
        self.icons.itemDoubleClicked.connect(lambda it: self._open(it.data(ITEM_ROLE), it.data(BIN_ROLE), it.data(ROW_ROLE)))
        self.tree.itemSelectionChanged.connect(self._selection_changed)
        self.icons.itemSelectionChanged.connect(self._selection_changed)
        self.tree.itemChanged.connect(lambda it, col: col == 0 and self._renamed(it.data(0, ITEM_ROLE), it.data(0, BIN_ROLE), it.text(0)))
        self.icons.itemChanged.connect(lambda it: self._renamed(it.data(ITEM_ROLE), it.data(BIN_ROLE), it.text()))

        self.canvas = FreeformCanvas(self)
        self.canvas.setObjectName(f"{prefix}.freeform")
        self.free_scroll = QScrollArea(self)
        self.free_scroll.setObjectName(f"{prefix}.freeformScroll")
        self.free_scroll.setWidget(self.canvas)
        self.free_scroll.setWidgetResizable(True)
        self.free_scroll.viewport().installEventFilter(self)
        self._width_timer = QTimer(self)
        self._width_timer.setSingleShot(True)
        self._width_timer.setInterval(250)
        self._width_timer.timeout.connect(self.refresh)

        self.stack = QStackedWidget(self)
        self.stack.addWidget(self.tree)
        self.stack.addWidget(self.icons)
        self.stack.addWidget(self.free_scroll)

        foot = QHBoxLayout()
        foot.setContentsMargins(4, 0, 4, 0)
        self.view_buttons: dict[str, QToolButton] = {}
        for key, text, tip in (("list", "☰", N_("List View")), ("icon", "▦", N_("Icon View")), ("freeform", "⧉", N_("Freeform View"))):
            b = QToolButton(self)
            b.setObjectName(f"{prefix}.view.{key}")
            b.setText(text)
            b.setToolTip(tr(tip))
            b.setCheckable(True)
            b.setAutoRaise(True)
            b.clicked.connect(lambda _c=False, k=key: self.set_mode(k))
            foot.addWidget(b)
            self.view_buttons[key] = b
        self.size_slider = QSlider(Qt.Orientation.Horizontal, self)
        self.size_slider.setObjectName(f"{prefix}.iconSize")
        self.size_slider.setRange(MIN_ICON, MAX_ICON)
        self.size_slider.setToolTip(tr("Zoom"))
        self.size_slider.setMaximumWidth(120)
        self.size_slider.valueChanged.connect(self._size_changed)
        self.size_slider.sliderReleased.connect(self._size_released)
        foot.addWidget(self.size_slider)
        self.sort_button = QToolButton(self)
        self.sort_button.setObjectName(f"{prefix}.sortIcons")
        self.sort_button.setText("⇅")
        self.sort_button.setAutoRaise(True)
        self.sort_button.setToolTip(tr("Sort Icons"))
        self.sort_button.setPopupMode(QToolButton.ToolButtonPopupMode.InstantPopup)
        self.sort_button.setMenu(QMenu(self.sort_button))
        self.sort_button.menu().aboutToShow.connect(lambda: self._fill_sort_menu(self.sort_button.menu()))
        foot.addWidget(self.sort_button)
        foot.addStretch(1)
        for key, text, tip in (
            ("clip.automateToSequence", "⇶", N_("Automate to Sequence")),
            ("find", "🔍", N_("Find")),
            ("file.newBin", "📁", N_("New Bin")),
            ("new-item", "✚", N_("New Item")),
            ("project.delete", "🗑", N_("Clear (Delete)")),
        ):
            b = QToolButton(self)
            b.setObjectName(f"{prefix}.button.{key}")
            b.setText(text)
            b.setToolTip(tr(tip))
            b.setAutoRaise(True)
            if key == "new-item":
                b.setPopupMode(QToolButton.ToolButtonPopupMode.InstantPopup)
                menu = QMenu(b)
                b.setMenu(menu)
                menu.aboutToShow.connect(lambda m=menu: self._fill_new_item(m))
            else:
                b.clicked.connect(lambda _c=False, k=key: self._footer(k))
            foot.addWidget(b)

        layout = QVBoxLayout(self)
        layout.setContentsMargins(2, 2, 2, 2)
        layout.setSpacing(3)
        layout.addLayout(head)
        layout.addLayout(search)
        layout.addWidget(self.preview)
        layout.addWidget(self.stack, 1)
        layout.addLayout(foot)
        self._apply_mode()

    def eventFilter(self, obj: Any, e: Any) -> bool:
        # a wider or narrower Freeform view re-wraps the cards nobody placed
        if obj is self.free_scroll.viewport() and e.type() == QEvent.Type.Resize and self.mode == "freeform":
            self._width_timer.start()
        return super().eventFilter(obj, e)

    # ----- state -----
    @property
    def view(self) -> dict[str, Any]:
        v = self.panel.view.get("view")
        return v if isinstance(v, dict) else {}

    @property
    def mode(self) -> str:
        m = self.own_mode if self.own_mode is not None else self.view.get("mode")
        return m if m in ("list", "icon", "freeform") else "list"

    @property
    def icon_size(self) -> int:
        s = self.own_size if self.own_mode is not None else self.view.get("iconSize")
        return int(min(MAX_ICON, max(MIN_ICON, s))) if isinstance(s, (int, float)) and not isinstance(s, bool) else 110

    @property
    def font_points(self) -> float:
        key = self.view.get("fontSize")
        return next((pts for k, _l, pts in FONT_SIZES if k == key), 12.0)

    @property
    def hover_scrub(self) -> bool:
        return bool(self.panel.view.get("hoverScrub"))

    def editing(self, index: QModelIndex) -> bool:
        return self.icons.state() == QAbstractItemView.State.EditingState and self.icons.currentIndex() == index

    def thumbnail(self, row: dict[str, Any], at: int | None, height: int) -> QImage | None:
        item = row.get("item")
        if not isinstance(item, int) or not row.get("hasVideo"):
            return None
        t = at if at is not None else _int(row.get("poster")) or 0
        q = TICKS_PER_SECOND // 4  # egui caches thumbnails on a quarter-second grid
        return self.panel.media.thumbnail(item, t // q * q if at is None else t, height)

    def selected_items(self) -> list[int]:
        if self.mode == "freeform" and not self.data.get("fallback"):
            return sorted(self.canvas.selected)
        if self.mode == "icon":
            return [it.data(ITEM_ROLE) for it in self.icons.selectedItems() if it.data(ITEM_ROLE) is not None]
        return [it.data(0, ITEM_ROLE) for it in self.tree.selectedItems() if it.data(0, ITEM_ROLE) is not None]

    # ----- data -----
    def refresh(self) -> None:
        engine = self.panel.engine
        params: dict[str, Any] = {"mode": self.mode, "expanded": sorted(self.expanded), "filter": self.filter.text()}
        if self.bin is not None:
            params["bin"] = self.bin
        if self.mode == "freeform":
            params["width"] = max(100, self.free_scroll.viewport().width() - 2 * int(FreeformCanvas.PAD))
        if engine is None or not getattr(engine, "running", False):
            self._fallback()
            return
        engine.call("project.panel", params, on_reply=lambda r, _b: self.set_data(r), on_error=lambda _m: self._fallback())

    def _fallback(self) -> None:
        project = self.panel.project or {}
        rows = fallback_rows(project.get("root"), 0, None, self.filter.text().strip().lower())
        total = sum(1 for r in rows if r.get("item") is not None)
        self.set_data(
            {
                "bin": (project.get("root") or {}).get("id") if isinstance(project.get("root"), dict) else None,
                "project": project.get("name") or "",
                "path": [],
                "mode": "list",
                "columns": [{"name": "Name", "width": 200}, {"name": "Media Type", "width": 90}, {"name": "Media Duration", "width": 100}],
                "total": total,
                "selection": self.selected_items(),
                "rows": rows,
                "fallback": True,
            }
        )

    def set_data(self, data: Any) -> None:
        if self._renaming():
            # rebuilding the rows would drop the rename being typed (New Bin's name, another
            # panel's edit arriving meanwhile): the latest data waits until the editor closes
            self._held = data
            return
        self._held = None
        self.data = data if isinstance(data, dict) else {}
        rows = [r for r in self.data.get("rows") or [] if isinstance(r, dict)]
        selection = {i for i in self.data.get("selection") or [] if isinstance(i, int)}
        self._apply_mode()
        self._updating = True
        try:
            if self.mode == "freeform" and not self.data.get("fallback"):
                self.canvas.set_data(self.data.get("cards"), rows, self.data.get("freeform"), selection)
                self._load_arrangements()
            elif self.mode == "icon" and not self.data.get("fallback"):
                self._fill_icons(rows, selection)
            else:
                self._fill_list(rows, selection)
        finally:
            self._updating = False
        self._update_head(selection)
        if self.top is not None:
            self.panel._update_tab_names()

    def _renaming(self) -> bool:
        view = self.icons if self.mode == "icon" else self.tree if self.mode == "list" else None
        if view is None or view.state() != QAbstractItemView.State.EditingState:
            return False
        if view not in self._watched:
            self._watched.append(view)
            view.itemDelegate().closeEditor.connect(lambda *_a: QTimer.singleShot(0, self, self._release_held))
        return True

    def _release_held(self) -> None:
        held = getattr(self, "_held", None)
        if held is not None and not self._renaming():
            self.set_data(held)

    def _load_arrangements(self) -> None:
        engine = self.panel.engine
        if engine is None or not getattr(engine, "running", False):
            return

        def got(r: Any, _b: Any) -> None:
            names = r.get("arrangements") if isinstance(r, dict) else None
            self.arrangements = [n for n in names or [] if isinstance(n, str)]

        engine.execute("project.freeform.arrangements", {"bin": self.data.get("bin")}, on_reply=got, on_error=lambda _m: None)

    def _update_head(self, selection: set[int]) -> None:
        d = self.data
        crumb = str(d.get("project") or "")
        for b in d.get("path") or []:
            if isinstance(b, dict):
                crumb += f" / {b.get('name')}"
        self.crumb.setText(crumb)
        self.up.setEnabled(d.get("bin") is not None and d.get("bin") != (self.top if self.top is not None else d.get("root")))
        total = _int(d.get("total")) or 0
        sel = len(selection)
        self.count.setText(fmt("{sel} of {total} items selected", sel=sel, total=total) if sel else fmt("{total} items", total=total))
        show_preview = bool(self.panel.view.get("previewArea"))
        self.preview.setVisible(show_preview)
        if show_preview:
            pv = d.get("preview")
            self.preview.row = pv if isinstance(pv, dict) else None
            self.preview.update()

    def _apply_mode(self) -> None:
        mode = self.mode
        for k, b in self.view_buttons.items():
            b.setChecked(k == mode)
        fallback = bool(self.data.get("fallback"))
        self.stack.setCurrentWidget(self.free_scroll if mode == "freeform" and not fallback else self.icons if mode == "icon" else self.tree)
        self.sort_button.setVisible(mode == "icon")
        self.size_slider.setVisible(mode != "freeform")
        self.size_slider.blockSignals(True)
        self.size_slider.setValue(self.icon_size)
        self.size_slider.blockSignals(False)
        font = QFont(self.font())
        font.setPixelSize(round(self.font_points))  # egui's points are logical pixels
        self.tree.setFont(font)
        self.icons.setFont(font)
        self.canvas.setFont(font)

    def _fill_list(self, rows: list[dict[str, Any]], selection: set[int]) -> None:
        tree = self.tree
        columns = [c for c in self.data.get("columns") or [] if isinstance(c, dict) and isinstance(c.get("name"), str)]
        if not columns:
            columns = [{"name": "Name", "width": 200}]
        names = [c["name"] for c in columns]
        shape = (tuple(names), tuple(c.get("width") for c in columns))
        scroll = tree.verticalScrollBar().value()
        tree.clear()
        if self._shown != shape:
            self._shown = shape
            tree.setColumnCount(len(names))
            tree.setHeaderLabels([tr(n) for n in names])
            header = tree.header()
            header.blockSignals(True)
            for i, c in enumerate(columns):
                w = c.get("width")
                if isinstance(w, (int, float)) and not isinstance(w, bool):
                    header.resizeSection(i, int(min(800, max(30, w))))
            header.blockSignals(False)
        sort = self.data.get("sort") if isinstance(self.data.get("sort"), dict) else {}
        header = tree.header()
        if sort.get("column") in names:
            header.setSortIndicator(names.index(sort["column"]), Qt.SortOrder.DescendingOrder if sort.get("descending") else Qt.SortOrder.AscendingOrder)
        else:
            header.setSortIndicator(-1, Qt.SortOrder.AscendingOrder)
        thumbs = bool(self.panel.view.get("thumbnails")) and not self.data.get("fallback")
        row_h = int(max(44, self.font_points * 3.6)) if thumbs else 0
        tree.setIconSize(QSize(int(row_h * 16 / 9), row_h) if thumbs else QSize(36, 16))
        parents: list[QTreeWidgetItem] = []
        reselect: list[QTreeWidgetItem] = []
        for row in rows:
            depth = _int(row.get("depth")) or 0
            del parents[depth:]
            parent = parents[-1] if parents else None
            if row.get("bin") is not None:
                it = QTreeWidgetItem([str(row.get("name") or "")] + [""] * (len(names) - 1))
                it.setData(0, BIN_ROLE, row.get("bin"))
                it.setData(0, ROW_ROLE, row)
                it.setIcon(0, kind_icon("bin", BIN_COLOR))
                it.setFlags(it.flags() | Qt.ItemFlag.ItemIsEditable | Qt.ItemFlag.ItemIsDragEnabled | Qt.ItemFlag.ItemIsDropEnabled)
                it.setChildIndicatorPolicy(QTreeWidgetItem.ChildIndicatorPolicy.ShowIndicator)
                if row.get("bin") == self.selected_bin:
                    reselect.append(it)
            else:
                cells = row.get("cells") if isinstance(row.get("cells"), dict) else {}
                texts = []
                for n in names:
                    if n == "Name":
                        texts.append(str(row.get("name") or ""))
                    elif n == "Label":
                        texts.append(tr(str(row.get("label") or "")))
                    elif n == "Media Type":
                        texts.append(tr(str(cells.get(n) or row.get("type") or "")))
                    else:
                        texts.append(str(cells.get(n) or ""))
                it = QTreeWidgetItem(texts)
                it.setData(0, ITEM_ROLE, row.get("item"))
                it.setData(0, TYPE_ROLE, row.get("type"))
                it.setData(0, ROW_ROLE, row)
                img = self.thumbnail(row, None, row_h) if thumbs else None
                if img is not None and not img.isNull():
                    it.setIcon(0, QIcon(QPixmap.fromImage(img)))
                else:
                    it.setIcon(0, chip_icon(label_color(row), str(row.get("icon") or "film")))
                if "Label" in names:
                    it.setIcon(names.index("Label"), kind_icon("bin", label_color(row), 10))
                if row.get("item") == (self.panel.project or {}).get("activeSequence"):
                    f = it.font(0)
                    f.setBold(True)
                    it.setFont(0, f)
                if row.get("offline"):
                    it.setForeground(0, OFFLINE)
                    it.setToolTip(0, tr(str(row.get("offline"))))
                it.setFlags((it.flags() | Qt.ItemFlag.ItemIsEditable | Qt.ItemFlag.ItemIsDragEnabled) & ~Qt.ItemFlag.ItemIsDropEnabled)
                if row.get("item") in selection:
                    reselect.append(it)
            if parent is None:
                tree.addTopLevelItem(it)
            else:
                parent.addChild(it)
            if row.get("bin") is not None:
                parents.append(it)
        # expansion after the rows are in (an item only expands inside its tree)
        for p in parents_all(tree):
            row = p.data(0, ROW_ROLE)
            if isinstance(row, dict) and row.get("open"):
                p.setExpanded(True)
        for it in reselect:
            it.setSelected(True)
        tree.verticalScrollBar().setValue(scroll)

    def _fill_icons(self, rows: list[dict[str, Any]], selection: set[int]) -> None:
        lst = self.icons
        lst.clear()
        size = self.icon_size
        lst.setGridSize(QSize(size + 24, round(size * 9 / 16) + 38))
        for row in rows:
            it = QListWidgetItem(str(row.get("name") or ""))
            it.setData(ROW_ROLE, row)
            if row.get("bin") is not None:
                it.setData(BIN_ROLE, row.get("bin"))
                selected = row.get("bin") == self.selected_bin
            else:
                it.setData(ITEM_ROLE, row.get("item"))
                it.setData(TYPE_ROLE, row.get("type"))
                selected = row.get("item") in selection
            it.setFlags(it.flags() | Qt.ItemFlag.ItemIsEditable | Qt.ItemFlag.ItemIsDragEnabled)
            lst.addItem(it)
            if selected:
                it.setSelected(True)

    # ----- interactions -----
    def _selection_changed(self) -> None:
        if self._updating:
            return
        view = self.icons if self.mode == "icon" else self.tree
        items = self.selected_items()
        if self.mode == "icon":
            bins = [it.data(BIN_ROLE) for it in self.icons.selectedItems() if it.data(BIN_ROLE) is not None]
        else:
            bins = [it.data(0, BIN_ROLE) for it in self.tree.selectedItems() if it.data(0, BIN_ROLE) is not None]
        self.selected_bin = bins[-1] if bins and not items else None
        del view
        self.panel.command.emit("project.select", {"items": items})

    def _open(self, item: Any, bin_id: Any, row: Any) -> None:
        if isinstance(bin_id, int):
            mods = QApplication.keyboardModifiers()
            prefs = self.data.get("open") if isinstance(self.data.get("open"), dict) else {}
            key = "ctrl" if mods & Qt.KeyboardModifier.ControlModifier else "alt" if mods & Qt.KeyboardModifier.AltModifier else "plain"
            how = HOW.get(str(prefs.get(key) or ""), {"ctrl": "newTab", "alt": "newWindow"}.get(key, "inPlace"))
            self.panel.open_bin(self, bin_id, how)
            return
        if not isinstance(item, int):
            return
        row = row if isinstance(row, dict) else {}
        if row.get("matte"):
            # like Premiere: double-clicking a Color Matte opens its colour picker
            self.panel.command.emit("project.select", {"items": [item]})
            self.panel.invoke.emit("project.matteColor")
        elif row.get("sequence") or str(row.get("type") or "").lower() == "sequence":
            self.panel.command.emit("sequence.open", {"item": item})
        else:
            self.panel.command.emit("source.open", {"item": item})

    def _renamed(self, item: Any, bin_id: Any, text: str) -> None:
        if self._updating:
            return
        name = text.strip()
        if not name:
            self.refresh()
            return
        if isinstance(item, int):
            self.panel.command.emit("clip.rename", {"item": item, "name": name})
        elif isinstance(bin_id, int):
            self.panel.command.emit("project.renameBin", {"bin": bin_id, "name": name})

    def rename(self, item: int | None = None, bin_id: int | None = None) -> bool:
        """Start the inline rename of an item or bin (the selection when neither is given)."""
        if item is None and bin_id is None:
            sel = self.selected_items()
            item, bin_id = (sel[0], None) if sel else (None, self.selected_bin)
        if self.mode == "freeform":
            return item is not None and self.canvas.edit(item)
        if self.mode == "icon":
            for i in range(self.icons.count()):
                it = self.icons.item(i)
                if (item is not None and it.data(ITEM_ROLE) == item) or (bin_id is not None and it.data(BIN_ROLE) == bin_id):
                    self.icons.setCurrentItem(it)
                    self.icons.editItem(it)
                    return True
            return False
        for it in all_items(self.tree):
            if (item is not None and it.data(0, ITEM_ROLE) == item) or (bin_id is not None and it.data(0, BIN_ROLE) == bin_id):
                self.tree.setCurrentItem(it)
                self.tree.editItem(it, 0)
                return True
        return False

    def rename_editor(self) -> tuple[QLineEdit, int | None, int | None] | None:
        """The inline rename being typed here: its editor and the item or bin it renames."""
        if self.mode == "freeform":
            ed = self.canvas._editor
            item = ed.property("item") if ed is not None else None
            return (ed, item, None) if ed is not None and isinstance(item, int) else None
        view = self.icons if self.mode == "icon" else self.tree
        if view.state() != QAbstractItemView.State.EditingState:
            return None
        ed = next((e for e in view.viewport().findChildren(QLineEdit) if e.isVisible()), None)
        if ed is None:
            return None
        if self.mode == "icon":
            it = self.icons.currentItem()
            ids = (it.data(ITEM_ROLE), it.data(BIN_ROLE)) if it is not None else (None, None)
        else:
            tit = self.tree.currentItem()
            ids = (tit.data(0, ITEM_ROLE), tit.data(0, BIN_ROLE)) if tit is not None else (None, None)
        item, bin_id = (i if isinstance(i, int) else None for i in ids)
        return ed, item, None if item is not None else bin_id

    def cancel_rename(self) -> None:
        """Leave the inline rename without renaming (Escape)."""
        if self.mode == "freeform":
            self.canvas._end_edit(None)
            return
        found = self.rename_editor()
        if found is not None:
            for kind in (QEvent.Type.KeyPress, QEvent.Type.KeyRelease):
                QApplication.sendEvent(found[0], QKeyEvent(kind, Qt.Key.Key_Escape, Qt.KeyboardModifier.NoModifier))

    def _twirl(self, it: QTreeWidgetItem, opened: bool) -> None:
        if self._updating:
            return
        b = it.data(0, BIN_ROLE)
        if not isinstance(b, int):
            return
        if opened:
            self.expanded.add(b)
        else:
            self.expanded.discard(b)
        self.refresh()

    def _moved(self, ids: list[int], target: Any) -> None:
        bin_id = target if isinstance(target, int) else self.data.get("bin")
        if bin_id == self.data.get("root"):
            bin_id = None
        ids = [i for i in ids if i != bin_id]
        if ids:
            self.panel.command.emit("project.moveToBin", {"items": ids, "bin": bin_id})

    def _hovered(self, item: Any, t: int) -> None:
        new = (item, t) if item is not None else None
        if new != self.hover:
            self.hover = new
            self.icons.viewport().update()
            self.canvas.update()

    def navigate(self, bin_id: int | None) -> None:
        self.bin = bin_id
        self.selected_bin = None
        self.refresh()

    def go_up(self) -> None:
        parent = self.data.get("parent")
        if self.top is not None and self.data.get("bin") == self.top:
            return
        self.navigate(parent if isinstance(parent, int) and parent != self.data.get("root") else self.top)

    def set_mode(self, mode: str) -> None:
        if self.own_mode is not None:
            self.own_mode = mode
            self.refresh()
        else:
            self.panel.set_view({"view": mode})

    def _size_changed(self, value: int) -> None:
        if self.own_mode is not None:
            self.own_size = value
        else:
            v = self.panel.view.get("view")
            if isinstance(v, dict):
                v["iconSize"] = value
        if self.mode == "icon":
            size = self.icon_size
            self.icons.setGridSize(QSize(size + 24, round(size * 9 / 16) + 38))
            self.icons.doItemsLayout()
        if not self.size_slider.isSliderDown():
            self._size_released()

    def _size_released(self) -> None:
        if self.own_mode is None:
            self.panel.set_view({"iconSize": self.size_slider.value()})

    def _sort_by(self, index: int) -> None:
        names = [c.get("name") for c in self.data.get("columns") or [] if isinstance(c, dict)]
        if 0 <= index < len(names) and not self.data.get("fallback"):
            self.panel.set_view_cmd("project.sort", {"column": names[index]})

    def _column_resized(self, index: int, _old: int, new: int) -> None:
        names = [c.get("name") for c in self.data.get("columns") or [] if isinstance(c, dict)]
        if 0 <= index < len(names) and isinstance(names[index], str) and not self.data.get("fallback"):
            self._resized[names[index]] = new
            self._resize_timer.start()

    def _send_resizes(self) -> None:
        resized, self._resized = self._resized, {}
        for column, width in resized.items():
            self.panel.set_view_cmd("project.columns.resize", {"column": column, "width": width}, refresh=False)

    def _header_menu(self, pos: QPoint) -> None:
        m = QMenu(self)
        m.setObjectName(f"{self.prefix}.headerMenu")
        m.addAction(tr("Metadata Display…"), self.panel.metadata_display)
        m.popup(self.tree.header().mapToGlobal(pos))

    def _fill_sort_menu(self, m: QMenu) -> None:
        m.clear()
        cur = self.view.get("iconSort") if isinstance(self.view.get("iconSort"), dict) else {}
        for col, label in (("", N_("User Order")), *((c, c) for c in SORT_ICONS)):
            a = m.addAction(tr(label))
            a.setObjectName(f"{self.prefix}.sortIcons.{col or 'userOrder'}")
            a.setCheckable(True)
            a.setChecked(cur.get("column", "") == col)
            a.triggered.connect(lambda _c=False, c=col: self.panel.set_view({"iconSort": {"column": c, "descending": False}}))
        m.addSeparator()
        a = m.addAction(tr("Descending"))
        a.setObjectName(f"{self.prefix}.sortIcons.descending")
        a.setCheckable(True)
        descending = bool(cur.get("descending"))
        a.setChecked(descending)
        a.triggered.connect(lambda _c=False, d=not descending: self.panel.set_view({"iconSort": {"descending": d}}))

    def _fill_new_item(self, m: QMenu) -> None:
        m.clear()
        for label, cid in NEW_ITEMS:
            a = m.addAction(tr(label))
            a.setObjectName(f"{self.prefix}.newItem.{cid}")
            a.triggered.connect(lambda _c=False, c=cid: self.panel.invoke.emit(c))

    def parent_param(self) -> int | None:
        b = self.data.get("bin")
        return b if isinstance(b, int) and b != self.data.get("root") else None

    def _footer(self, key: str) -> None:
        if key == "find":
            self.panel.invoke.emit("edit.find")
        elif key == "file.newBin":
            self.panel.new_bin(self, self.parent_param())
        elif key == "project.delete":
            self.panel.clear(self)
        else:
            self.panel.invoke.emit(key)

    # ----- menus -----
    def _context_menu(self, view: QAbstractItemView, pos: QPoint) -> None:
        if view is self.tree:
            it = self.tree.itemAt(pos)
            item = it.data(0, ITEM_ROLE) if it is not None else None
            bin_id = it.data(0, BIN_ROLE) if it is not None else None
            row = it.data(0, ROW_ROLE) if it is not None else None
        else:
            li = self.icons.itemAt(pos)
            item = li.data(ITEM_ROLE) if li is not None else None
            bin_id = li.data(BIN_ROLE) if li is not None else None
            row = li.data(ROW_ROLE) if li is not None else None
        if isinstance(item, int):
            m = self.panel.item_menu(self, item, row if isinstance(row, dict) else {})
        elif isinstance(bin_id, int):
            m = self.panel.bin_menu(self, bin_id)
        else:
            m = self.panel.background_menu(self)
        m.popup(view.viewport().mapToGlobal(pos))


def all_items(tree: QTreeWidget) -> list[QTreeWidgetItem]:
    out = []
    it = QTreeWidgetItemIterator(tree)
    while it.value():
        out.append(it.value())
        it += 1
    return out


def parents_all(tree: QTreeWidget) -> list[QTreeWidgetItem]:
    return [it for it in all_items(tree) if it.data(0, BIN_ROLE) is not None]


# ----------------------------------------------------------------------------------- dialogs
# The Project panel's dialogs keep their drafts where the control channel reads and sets them
# (egui's ``UiState::project_panel.dialog``): ``state()`` gives egui's fields, ``set_state`` takes
# them and raises ValueError, changing nothing, on a field the dialog would not take.
def _fields(fields: Any, known: tuple[str, ...], kind: str) -> dict[str, Any]:
    if not isinstance(fields, dict):
        raise TypeError(f"dialog.{kind} takes {{{', '.join(known)}}}")
    bad = [k for k in fields if k not in known]
    if bad:
        raise ValueError(f"dialog.{kind}: unknown field {bad[0]!r} (known: {', '.join(known)})")
    return fields


def _text(fields: dict[str, Any], key: str, kind: str) -> str | None:
    v = fields.get(key)
    if key in fields and (not isinstance(v, str) or len(v) > 4096):
        raise ValueError(f"dialog.{kind}.{key} takes a string")
    return v if isinstance(v, str) else None


class MetadataDisplayDialog(QDialog):
    """Metadata Display…: the List view's columns, in order, and the field filter (egui's
    ``project_dialogs::metadata_display``)."""

    KIND = "metadataDisplay"

    def __init__(self, available: list[str], shown: list[str], parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("project.metadataDisplay")
        self.setWindowTitle(tr("Metadata Display"))
        self.available = [*shown, *(a for a in available if a not in shown)]
        self.filter = QLineEdit(self)
        self.filter.setObjectName("project.metadataDisplay.filter")
        self.filter.setPlaceholderText(tr("Search fields"))
        self.filter.textChanged.connect(self._filter)
        self.list = QListWidget(self)
        self.list.setObjectName("project.metadataDisplay.columns")
        self.list.setDragDropMode(QAbstractItemView.DragDropMode.InternalMove)
        self._fill(shown)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel, self)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        layout = QVBoxLayout(self)
        show = QHBoxLayout()
        show.addWidget(QLabel(tr("Show:"), self))
        show.addWidget(self.filter, 1)
        layout.addLayout(show)
        layout.addWidget(QLabel(tr("Drag to reorder; tick the columns to show."), self))
        layout.addWidget(self.list, 1)
        layout.addWidget(buttons)

    def _fill(self, shown: list[str]) -> None:
        self.list.clear()
        for name in [*shown, *(a for a in self.available if a not in shown)]:
            it = QListWidgetItem(tr(name))
            it.setData(ITEM_ROLE, name)
            it.setFlags(it.flags() | Qt.ItemFlag.ItemIsUserCheckable)
            it.setCheckState(Qt.CheckState.Checked if name in shown else Qt.CheckState.Unchecked)
            if name == "Name":
                it.setFlags(it.flags() & ~Qt.ItemFlag.ItemIsUserCheckable)
            self.list.addItem(it)
        self._filter(self.filter.text())

    def _filter(self, text: str) -> None:
        f = text.strip().lower()
        for i in range(self.list.count()):
            it = self.list.item(i)
            name = str(it.data(ITEM_ROLE))
            it.setHidden(bool(f) and f not in name.lower() and f not in it.text().lower())

    def columns(self) -> list[str]:
        out = []
        for i in range(self.list.count()):
            it = self.list.item(i)
            if it.checkState() == Qt.CheckState.Checked or it.data(ITEM_ROLE) == "Name":
                out.append(str(it.data(ITEM_ROLE)))
        return out

    def state(self) -> dict[str, Any]:
        return {"columns": self.columns(), "filter": self.filter.text()}

    def set_state(self, fields: Any) -> None:
        f = _fields(fields, ("columns", "filter"), self.KIND)
        cols = f.get("columns")
        if "columns" in f:
            if not isinstance(cols, list) or not all(isinstance(c, str) for c in cols):
                raise ValueError("dialog.metadataDisplay.columns takes a list of field names")
            unknown = [c for c in cols if c not in self.available]
            if unknown:
                raise ValueError(f"dialog.metadataDisplay.columns: no field {unknown[0]!r}")
        text = _text(f, "filter", self.KIND)
        if text is not None:
            self.filter.setText(text)
        if isinstance(cols, list):
            self._fill(list(dict.fromkeys(["Name", *cols] if "Name" not in cols else cols)))


class NameDialog(QDialog):
    """Save As New View Preset and Freeform ▸ Save Arrangement…: a name (and the bin the
    arrangement belongs to)."""

    def __init__(self, kind: str, title: str, name: str, note: str = "", bin_id: int | None = None, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.KIND = kind
        self.bin = bin_id
        self.setObjectName(f"project.{kind}")
        self.setWindowTitle(title)
        self.name = QLineEdit(name, self)
        self.name.setObjectName(f"project.{kind}.name")
        self.name.selectAll()
        form = QFormLayout()
        form.addRow(tr("Name:"), self.name)
        self.buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel, self)
        self.buttons.accepted.connect(self.accept)
        self.buttons.rejected.connect(self.reject)
        ok = self.buttons.button(QDialogButtonBox.StandardButton.Ok)
        ok.setObjectName(f"project.{kind}.ok")
        self.name.textChanged.connect(lambda t: ok.setEnabled(bool(t.strip())))
        ok.setEnabled(bool(name.strip()))
        layout = QVBoxLayout(self)
        layout.addLayout(form)
        if note:
            hint = QLabel(note, self)
            hint.setEnabled(False)
            layout.addWidget(hint)
        layout.addWidget(self.buttons)

    def text(self) -> str:
        return self.name.text().strip()

    def state(self) -> dict[str, Any]:
        out: dict[str, Any] = {"name": self.name.text()}
        if self.KIND == "saveArrangement":
            out["bin"] = self.bin
        return out

    def set_state(self, fields: Any) -> None:
        f = _fields(fields, ("name", "bin") if self.KIND == "saveArrangement" else ("name",), self.KIND)
        b = f.get("bin")
        if b is not None and (isinstance(b, bool) or not isinstance(b, int) or b < 0):
            raise ValueError("dialog.saveArrangement.bin takes a bin id or null")
        name = _text(f, "name", self.KIND)
        if name is not None:
            self.name.setText(name)
        if "bin" in f:
            self.bin = b


class FreeformOptionsDialog(QDialog):
    """Freeform View Options…: the grid, snapping, card names and durations and the default card
    size (``project.freeform.options``)."""

    KIND = "freeformOptions"
    RANGES: ClassVar[dict[str, tuple[int, int]]] = {"grid": (4, 200), "cardSize": (48, 480)}

    def __init__(self, opts: dict[str, Any], parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("project.freeformOptions")
        self.setWindowTitle(tr("Freeform View Options"))

        def num(key: str, default: float) -> float:
            v = opts.get(key)
            return float(v) if isinstance(v, (int, float)) and not isinstance(v, bool) and math.isfinite(v) else default

        self.grid = QSpinBox(self)
        self.grid.setObjectName("project.freeformOptions.grid")
        self.grid.setRange(*self.RANGES["grid"])
        self.grid.setValue(int(min(200, max(4, num("grid", 20)))))
        self.card = QSpinBox(self)
        self.card.setObjectName("project.freeformOptions.cardSize")
        self.card.setRange(*self.RANGES["cardSize"])
        self.card.setValue(int(min(480, max(48, num("cardSize", 128)))))
        self.snap = QCheckBox(tr("Snap to grid"), self)
        self.snap.setObjectName("project.freeformOptions.snap")
        self.snap.setChecked(opts.get("snap") is True)
        self.names = QCheckBox(tr("Show clip names"), self)
        self.names.setObjectName("project.freeformOptions.showNames")
        self.names.setChecked(opts.get("showNames") is not False)
        self.durations = QCheckBox(tr("Show clip durations"), self)
        self.durations.setObjectName("project.freeformOptions.showDurations")
        self.durations.setChecked(opts.get("showDurations") is not False)
        form = QFormLayout()
        form.addRow(tr("Grid spacing:"), self.grid)
        form.addRow(tr("Default clip size:"), self.card)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel, self)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        layout = QVBoxLayout(self)
        layout.addLayout(form)
        for w in (self.snap, self.names, self.durations):
            layout.addWidget(w)
        layout.addWidget(buttons)

    def values(self) -> dict[str, Any]:
        return {
            "grid": self.grid.value(),
            "snap": self.snap.isChecked(),
            "showNames": self.names.isChecked(),
            "showDurations": self.durations.isChecked(),
            "cardSize": self.card.value(),
        }

    def state(self) -> dict[str, Any]:
        return {"options": self.values()}

    def set_state(self, fields: Any) -> None:
        f = _fields(fields, ("options",), self.KIND)
        if "options" not in f:
            return
        o = _fields(f["options"], ("grid", "snap", "showNames", "showDurations", "cardSize"), f"{self.KIND}.options")
        for k, (lo, hi) in self.RANGES.items():
            v = o.get(k)
            if k in o and (isinstance(v, bool) or not isinstance(v, (int, float)) or not math.isfinite(v) or not lo <= v <= hi):
                raise ValueError(f"dialog.freeformOptions.options.{k} takes a number from {lo} to {hi}")
        for k in ("snap", "showNames", "showDurations"):
            if k in o and not isinstance(o[k], bool):
                raise ValueError(f"dialog.freeformOptions.options.{k} takes true or false")
        for k, w in (("grid", self.grid), ("cardSize", self.card)):
            if k in o:
                w.setValue(round(o[k]))
        for k, box in (("snap", self.snap), ("showNames", self.names), ("showDurations", self.durations)):
            if k in o:
                box.setChecked(o[k])


PRESET_SLOTS = 10  # cfa4_engine::project_panel::PRESET_SLOTS


class ManagePresetsDialog(QDialog):
    """Manage Saved View Presets as egui's: the ten slots (empty ones too), the selected slot's
    name, Rename, Restore, Delete, Overwrite with Current View, Done."""

    KIND = "managePresets"

    def __init__(self, panel: ProjectPanel, presets: list[dict[str, Any]]) -> None:
        super().__init__(panel)
        self.panel = panel
        self.setObjectName("project.managePresets")
        self.setWindowTitle(tr("Manage Saved View Presets"))
        self.names: list[str | None] = [None] * PRESET_SLOTS
        self.list = QListWidget(self)
        self.list.setObjectName("project.managePresets.slots")
        for i in range(PRESET_SLOTS):
            self.list.addItem(QListWidgetItem())
        self.name = QLineEdit(self)
        self.name.setObjectName("project.managePresets.name")
        self.list.currentRowChanged.connect(self._selected)
        self.buttons: dict[str, QPushButton] = {}
        for key, label, run in (
            ("rename", N_("Rename"), self._rename),
            ("restore", N_("Restore"), lambda: self._run("project.viewPreset.restore", {})),
            ("delete", N_("Delete"), lambda: self._run("project.viewPreset.delete", {})),
            ("overwrite", N_("Overwrite with Current View"), self._overwrite),
        ):
            b = QPushButton(tr(label), self)
            b.setObjectName(f"project.managePresets.{key}")
            b.clicked.connect(run)
            self.buttons[key] = b
        done = QDialogButtonBox(self)
        ok = done.addButton(tr("Done"), QDialogButtonBox.ButtonRole.AcceptRole)
        ok.setObjectName("project.managePresets.done")
        done.accepted.connect(self.accept)
        row = QHBoxLayout()
        row.addWidget(QLabel(tr("Name:"), self))
        row.addWidget(self.name, 1)
        row.addWidget(self.buttons["rename"])
        actions = QHBoxLayout()
        for key in ("restore", "delete", "overwrite"):
            actions.addWidget(self.buttons[key])
        layout = QVBoxLayout(self)
        layout.addWidget(self.list, 1)
        layout.addLayout(row)
        layout.addLayout(actions)
        layout.addWidget(done)
        self.set_presets(presets)
        self.list.setCurrentRow(0)
        self._selected(0)

    def set_presets(self, presets: list[dict[str, Any]]) -> None:
        """The slots as the engine has them; the name field follows the selected slot (egui
        refreshes it after each action)."""
        self.names = [None] * PRESET_SLOTS
        for p in presets:
            slot = p.get("slot") if isinstance(p, dict) else None
            if isinstance(slot, int) and 1 <= slot <= PRESET_SLOTS and isinstance(p.get("name"), str) and p["name"]:
                self.names[slot - 1] = p["name"]
        for i, name in enumerate(self.names):
            self.list.item(i).setText(f"{i + 1}. {name}" if name else fmt("{n}. (empty)", n=i + 1))
        self._selected(self.list.currentRow())

    def _selected(self, row: int) -> None:
        filled = 0 <= row < PRESET_SLOTS and self.names[row] is not None
        self.name.setText(self.names[row] or "" if 0 <= row < PRESET_SLOTS else "")
        for w in (self.name, self.buttons["rename"], self.buttons["restore"], self.buttons["delete"]):
            w.setEnabled(filled)

    def _run(self, cid: str, params: dict[str, Any]) -> None:
        row = self.list.currentRow()
        if 0 <= row < PRESET_SLOTS:
            self.panel.preset_cmd(cid, {"slot": row + 1, **params})

    def _rename(self) -> None:
        name = self.name.text().strip()
        if name:
            self._run("project.viewPreset.rename", {"name": name})

    def _overwrite(self) -> None:
        name = self.name.text().strip()
        self._run("project.viewPreset.saveAs", {"name": name or None})

    def state(self) -> dict[str, Any]:
        return {"selected": max(0, self.list.currentRow()), "name": self.name.text()}

    def set_state(self, fields: Any) -> None:
        f = _fields(fields, ("selected", "name"), self.KIND)
        sel = f.get("selected")
        if "selected" in f and (isinstance(sel, bool) or not isinstance(sel, int) or not 0 <= sel < PRESET_SLOTS):
            raise ValueError(f"dialog.managePresets.selected takes a slot from 0 to {PRESET_SLOTS - 1}")
        name = _text(f, "name", self.KIND)
        if isinstance(sel, int):
            self.list.setCurrentRow(sel)
        if name is not None:
            self.name.setText(name)


# ----------------------------------------------------------------------------------- the panel
class ProjectPanel(QWidget):
    command = Signal(str, dict)
    invoke = Signal(str)  # a command id to run through the GUI's dialog path (import, new bin, ...)
    run = Signal(str, dict)  # a command with some parameters known, the rest asked (dialogs)
    message = Signal(str)  # an error for the status bar

    def __init__(self, engine: Any = None, parent: QWidget | None = None) -> None:
        if parent is None and isinstance(engine, QWidget):  # ProjectPanel(parent): no engine
            engine, parent = None, engine
        super().__init__(parent)
        self.setObjectName("panel.Project")
        self.engine = engine
        self.project: dict[str, Any] | None = None
        self.view: dict[str, Any] = {}  # project.view.get
        self.presets: list[dict[str, Any]] = []
        self.dialog: QDialog | None = None  # the open dialog (egui's ``project_panel.dialog``)
        self.media = TimelineMedia(self)
        self.media.attach(engine)
        self.media.updated.connect(self._media_updated)
        self.tabs = QTabBar(self)
        self.tabs.setObjectName("project.tabs")
        self.tabs.setTabsClosable(True)
        self.tabs.setExpanding(False)
        self.tabs.setDocumentMode(True)
        self.tabs.addTab(tr("Project"))
        self.tabs.setTabButton(0, QTabBar.ButtonPosition.RightSide, None)
        self.tabs.setTabButton(0, QTabBar.ButtonPosition.LeftSide, None)
        self.tabs.currentChanged.connect(lambda i: self.stack.setCurrentIndex(max(0, i)))
        self.tabs.tabCloseRequested.connect(self.close_tab)
        self.tabs.hide()
        self.main = BinView(self, "project")
        self.stack = QStackedWidget(self)
        self.stack.addWidget(self.main)
        self.windows: list[QWidget] = []
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(0)
        layout.addWidget(self.tabs)
        layout.addWidget(self.stack, 1)
        self._media_timer = QTimer(self)
        self._media_timer.setSingleShot(True)
        self._media_timer.setInterval(60)
        self._media_timer.timeout.connect(self._repaint_thumbs)

    # the List view of the Project panel itself (tests, the main window's focus tracking)
    @property
    def tree(self) -> ProjectTree:
        return self.main.tree

    @property
    def filter(self) -> QLineEdit:
        return self.main.filter

    def views(self) -> list[BinView]:
        out = [self.stack.widget(i) for i in range(self.stack.count())]
        out += [w.findChild(BinView) for w in self.windows]
        return [v for v in out if isinstance(v, BinView)]

    def hover_mark(self, is_in: bool) -> bool:
        """I / O over a hover-scrubbed card sets that item's In / Out at the hovered time (egui's
        ``hover_mark``); returns False when no card is being scrubbed, so the keys keep their usual
        meaning."""
        for v in self.views():
            hover = v.hover
            shown = v.stack.currentWidget()
            if hover is None or not v.hover_scrub or shown not in (v.icons, v.free_scroll) or not shown.isVisible():
                continue
            item, t = hover
            if not isinstance(item, int) or isinstance(item, bool) or not isinstance(t, int):
                continue
            self.command.emit("project.setMarks", {"item": item, "in" if is_in else "out": max(0, t)})
            return True
        return False

    def shown(self) -> BinView:
        w = self.stack.currentWidget()
        return w if isinstance(w, BinView) else self.main

    # ----- data -----
    def set_project(self, project: dict[str, Any] | None) -> None:
        old = (self.project or {}).get("path"), (self.project or {}).get("name")
        self.project = project
        if (old[0], old[1]) != ((project or {}).get("path"), (project or {}).get("name")):
            self.media.clear()
        self.tabs.setTabText(0, fmt("Project: {name}", name=(project or {}).get("name") or ""))
        self.refresh_all()

    def refresh_all(self) -> None:
        engine = self.engine
        if engine is not None and getattr(engine, "running", False):
            engine.execute("project.view.get", {}, on_reply=lambda r, _b: self._got_view(r), on_error=lambda _m: self._got_view(None))
        else:
            self._got_view(None)

    def _got_view(self, result: Any) -> None:
        self.view = result if isinstance(result, dict) else {}
        for v in self.views():
            v.refresh()
        self._update_tab_names()

    def _media_updated(self) -> None:
        self._media_timer.start()

    def _repaint_thumbs(self) -> None:
        for v in self.views():
            self.media.begin_paint()
            if v.mode == "icon":
                v.icons.viewport().update()
            elif self.view.get("thumbnails") and v.isVisible():
                v.refresh()
            if v.preview.isVisible():
                v.preview.update()

    def selected_items(self) -> list[int]:
        return self.shown().selected_items()

    def set_view(self, params: dict[str, Any]) -> None:
        self.set_view_cmd("project.view.set", params)

    def set_view_cmd(self, cid: str, params: dict[str, Any], refresh: bool = True) -> None:
        """A view setting (an engine preference: no project change follows, so refresh here)."""
        engine = self.engine
        if engine is None or not getattr(engine, "running", False):
            return
        engine.execute(
            cid,
            params,
            on_reply=lambda _r, _b: refresh and self.refresh_all(),
            on_error=lambda m: self.message.emit(f"{cid}: {m}"),
        )

    # ----- bins -----
    def open_bin(self, view: BinView, bin_id: int, how: str) -> None:
        if how == "inPlace":
            view.navigate(bin_id)
            return
        floating = how == "newWindow"
        for i in range(1, self.stack.count()):
            v = self.stack.widget(i)
            if not floating and isinstance(v, BinView) and v.top == bin_id:
                self.tabs.setCurrentIndex(i)
                return
        k = len(self.views())
        mode = view.mode
        size = view.icon_size
        if floating:
            win = QWidget(self, Qt.WindowType.Window)
            win.setObjectName(f"projectBin.{k}.window")
            win.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
            bv = BinView(self, f"projectBin.{k}", bin_id, mode, size)
            lay = QVBoxLayout(win)
            lay.setContentsMargins(0, 0, 0, 0)
            lay.addWidget(bv)
            win.resize(520, 360)
            win.destroyed.connect(lambda _o=None, w=win: self.windows.remove(w) if w in self.windows else None)
            self.windows.append(win)
            bv.refresh()
            win.show()
            self._update_tab_names()
            return
        bv = BinView(self, f"projectBin.{k}", bin_id, mode, size)
        self.stack.addWidget(bv)
        i = self.tabs.addTab("")
        self.tabs.show()
        bv.refresh()
        self._update_tab_names()
        self.tabs.setCurrentIndex(i)

    # ----- view state for the control channel (egui's ``ProjectPanelUi``) -----
    def _opened(self) -> list[tuple[BinView, bool, QWidget | None]]:
        """Bins opened in tabs, then in windows: (view, floating, window)."""
        out: list[tuple[BinView, bool, QWidget | None]] = []
        for i in range(1, self.stack.count()):
            v = self.stack.widget(i)
            if isinstance(v, BinView):
                out.append((v, False, None))
        for w in self.windows:
            v = w.findChild(BinView)
            if isinstance(v, BinView):
                out.append((v, True, w))
        return out

    def ui_state(self) -> dict[str, Any]:
        tabs = [{"bin": v.top, "floating": floating, "view": v.mode, "iconSize": v.icon_size, "nav": v.bin if v.bin != v.top else None}
                for v, floating, _w in self._opened()]
        i = self.tabs.currentIndex()
        return {"bin": self.main.bin, "tabs": tabs, "activeTab": i - 1 if i > 0 else None, "selectedBin": self.shown().selected_bin,
                "rename": self.rename_state(), "dialog": self.dialog_state()}

    def rename_state(self) -> dict[str, Any] | None:
        """egui's ``project_panel.rename``: the item or bin being renamed in place, the text typed
        so far and the panel instance editing it."""
        for v in self.views():
            found = v.rename_editor()
            if found is not None:
                ed, item, bin_id = found
                return {"item": item, "bin": bin_id, "text": ed.text(), "panel": v.prefix}
        return None

    def set_rename(self, value: Any) -> None:
        """null leaves any inline rename unrenamed; ``{item | bin, text, panel}`` starts one there
        (the shown view when ``panel`` is left out) and types ``text`` into it."""
        def an_id(v: Any) -> bool:
            return v is None or (isinstance(v, int) and not isinstance(v, bool) and v >= 0)

        if value is None:
            for v in self.views():
                v.cancel_rename()
            return
        if not isinstance(value, dict):
            raise TypeError("rename takes null or {item, bin, text, panel}")
        bad = [k for k in value if k not in ("item", "bin", "text", "panel")]
        if bad:
            raise ValueError(f"rename: unknown field {bad[0]!r} (known: item, bin, text, panel)")
        item, bin_id, text, prefix = value.get("item"), value.get("bin"), value.get("text"), value.get("panel")
        if not an_id(item) or not an_id(bin_id) or (item is None) == (bin_id is None):
            raise ValueError("rename takes an item id or a bin id")
        if text is not None and (not isinstance(text, str) or len(text) > 4096):
            raise ValueError("rename.text takes a string")
        views = {v.prefix: v for v in self.views()}
        if prefix is None:
            current = self.rename_state()
            prefix = current["panel"] if current is not None else self.shown().prefix
        view = views.get(prefix) if isinstance(prefix, str) else None
        if view is None:
            raise ValueError(f"rename.panel: no panel {prefix!r} (open: {', '.join(views)})")
        found = view.rename_editor()
        if found is None or found[1:] != (item, bin_id):
            if not view.rename(item, bin_id):
                raise ValueError(f"rename: no {'item' if item is not None else 'bin'} {item if item is not None else bin_id} in {prefix}")
            found = view.rename_editor()
        if found is not None and text is not None:
            found[0].setText(text)

    def set_ui_state(self, patch: dict[str, Any]) -> None:
        """Merge ``patch`` ({bin, tabs, activeTab, selectedBin, rename, dialog}) into the view
        state: ``tabs`` opens and closes bin tabs and windows to match the list, ``rename`` and
        ``dialog`` as egui's (``set_rename``, ``set_dialog_state``). ValueError names a bad field."""
        def bin_id(v: Any) -> bool:
            return v is None or (isinstance(v, int) and not isinstance(v, bool) and v >= 0)

        known = ("bin", "tabs", "activeTab", "selectedBin", "rename", "dialog")
        bad = [k for k in patch if k not in known]
        if bad:
            raise ValueError(f"unknown field {bad[0]!r} (known: {', '.join(known)})")
        for k in ("bin", "selectedBin", "activeTab"):
            if not bin_id(patch.get(k)):
                raise ValueError(f"{k} takes an id or null")
        tabs = patch.get("tabs")
        wanted: list[dict[str, Any]] = []
        if tabs is not None:
            if not isinstance(tabs, list) or len(tabs) > 64:
                raise ValueError("tabs takes a list of {bin, floating, view, iconSize, nav}")
            for t in tabs:
                if not isinstance(t, dict) or not isinstance(t.get("bin"), int) or isinstance(t.get("bin"), bool) or not bin_id(t.get("nav")):
                    raise ValueError("each tab needs a bin id")
                if t.get("view", "list") not in ("list", "icon", "freeform"):
                    raise ValueError("a tab's view is list, icon or freeform")
                size = t.get("iconSize", 110)
                if isinstance(size, bool) or not isinstance(size, (int, float)):
                    raise TypeError("a tab's iconSize is a number")
                wanted.append({"bin": t["bin"], "floating": t.get("floating") is True, "view": t.get("view", "list"),
                               "iconSize": int(min(MAX_ICON, max(MIN_ICON, size))), "nav": t.get("nav")})
        if "bin" in patch:
            self.main.navigate(patch["bin"])
        if tabs is not None:
            have = self._opened()
            keep = 0  # the opened bins that already match the list's start stay
            while keep < min(len(have), len(wanted)) and (have[keep][0].top, have[keep][1]) == (wanted[keep]["bin"], wanted[keep]["floating"]):
                keep += 1
            for v, floating, w in reversed(have[keep:]):
                if floating and w is not None:
                    self.windows.remove(w)
                    w.close()
                else:
                    self.close_tab(self.stack.indexOf(v))
            for t in wanted[keep:]:
                self.open_bin(self.main, t["bin"], "newWindow" if t["floating"] else "newTab")
            for (v, _f, _w), t in zip(self._opened(), wanted):
                if (v.own_mode, v.own_size) != (t["view"], t["iconSize"]):
                    v.own_mode, v.own_size = t["view"], t["iconSize"]
                    v.refresh()
                if t["nav"] is not None and v.bin != t["nav"]:
                    v.navigate(t["nav"])
        if "activeTab" in patch:
            a = patch["activeTab"]
            count = self.stack.count() - 1
            if a is not None and a >= count:
                raise ValueError(f"activeTab: there are {count} bin tabs")
            self.tabs.setCurrentIndex(0 if a is None else a + 1)
        if "selectedBin" in patch:
            v = self.shown()
            v.selected_bin = patch["selectedBin"]
            v.refresh()
        if "dialog" in patch:
            self.set_dialog_state(patch["dialog"])
        if "rename" in patch:
            self.set_rename(patch["rename"])

    def close_tab(self, index: int) -> None:
        if index <= 0 or index >= self.stack.count():
            return
        w = self.stack.widget(index)
        self.stack.removeWidget(w)
        w.deleteLater()
        self.tabs.removeTab(index)
        self.tabs.setVisible(self.tabs.count() > 1)

    def _bin_name(self, bin_id: Any) -> str:
        def find(node: Any) -> str | None:
            if not isinstance(node, dict):
                return None
            if node.get("id") == bin_id:
                return str(node.get("name") or "")
            for c in node.get("children") or []:
                if isinstance(c, dict) and "item" not in c:
                    found = find(c)
                    if found is not None:
                        return found
            return None

        return find((self.project or {}).get("root")) or ""

    def _view_name(self, v: BinView) -> str:
        # the view's own reply names its bin when it shows it (fresher than project.inspect)
        path = v.data.get("path") if v.data.get("bin") == v.top else None
        if isinstance(path, list) and path and isinstance(path[-1], dict):
            return str(path[-1].get("name") or "")
        return self._bin_name(v.top)

    def _update_tab_names(self) -> None:
        for i in range(1, self.stack.count()):
            v = self.stack.widget(i)
            if isinstance(v, BinView):
                self.tabs.setTabText(i, fmt("Bin: {name}", name=self._view_name(v)))
        for w in self.windows:
            v = w.findChild(BinView)
            if v is not None:
                w.setWindowTitle(fmt("Bin: {name}", name=self._view_name(v)))

    def new_bin(self, view: BinView, parent: int | None) -> None:
        """New Bin: a bin named "New Bin" in the shown bin, then its name ready to type."""
        engine = self.engine
        if engine is None or not getattr(engine, "running", False):
            self.invoke.emit("file.newBin")
            return

        def made(result: Any, _b: Any) -> None:
            b = result.get("bin") if isinstance(result, dict) else None
            if parent is not None:
                view.expanded.add(parent)
            view.selected_bin = b if isinstance(b, int) else None
            self.command.emit("project.select", {"items": []})
            if isinstance(b, int):
                self._rename_when_shown(view, b)

        engine.execute("file.newBin", {"name": tr("New Bin"), "parent": parent}, on_reply=made, on_error=lambda m: self.message.emit(f"file.newBin: {m}"))

    def _rename_when_shown(self, view: BinView, bin_id: int, tries: int = 20) -> None:
        if view.rename(None, bin_id) or tries <= 0:
            return
        view.refresh()
        QTimer.singleShot(100, self, lambda: self._rename_when_shown(view, bin_id, tries - 1))

    def clear(self, view: BinView) -> None:
        items = view.selected_items()
        if items:
            self.command.emit("project.delete", {"items": items})
        elif view.selected_bin is not None:
            self.command.emit("project.delete", {"items": [view.selected_bin]})
            view.selected_bin = None

    def metadata_display(self) -> None:
        available = [a for a in self.view.get("available") or [] if isinstance(a, str)]
        shown = [c.get("name") for c in (self.view.get("view") or {}).get("columns") or [] if isinstance(c, dict) and isinstance(c.get("name"), str)]
        dlg = MetadataDisplayDialog(available or shown, shown, self)
        self._open_dialog(dlg, lambda: self.set_view_cmd("project.columns.set", {"columns": dlg.columns()}))

    def reveal_project(self) -> None:
        path = (self.project or {}).get("path")
        if isinstance(path, str) and path:
            QDesktopServices.openUrl(QUrl.fromLocalFile(str(Path(path).parent)))

    # ----- menus -----
    def item_menu(self, view: BinView, item: int, row: dict[str, Any]) -> QMenu:
        m = QMenu(self)
        m.setObjectName("project.itemMenu")

        def add(key: str, label: str, fn: Any, enabled: bool = True) -> None:
            a = m.addAction(tr(label))
            a.setObjectName(f"project.itemMenu.{key}")
            a.setEnabled(enabled)
            a.triggered.connect(fn)

        sel = {"items": [item]}

        def select_then(cid: str, params: dict[str, Any] | None = None, dialog: bool = False) -> Any:
            def go() -> None:
                self.command.emit("project.select", sel)
                if dialog:
                    self.invoke.emit(cid)
                else:
                    self.command.emit(cid, params or {})

            return go

        add("rename", N_("Rename"), lambda: view.rename(item, None))
        add("openInSource", N_("Open in Source Monitor"), lambda: self.command.emit("source.open", {"item": item}))
        if row.get("sequence"):
            add("openInTimeline", N_("Open in Timeline"), lambda: self.command.emit("sequence.open", {"item": item}))
        add("newSequenceFromClip", N_("New Sequence From Clip"), lambda: self.run.emit("file.newSequence", {"fromItem": item}))
        add("duplicate", N_("Duplicate"), select_then("edit.duplicate"))
        hover = view.hover if view.hover is not None and view.hover[0] == item else None
        add("setPosterFrame", N_("Set Poster Frame"), select_then("clip.setPosterFrame", {"item": item, "time": hover[1]} if hover else {}))
        add("interpretFootage", N_("Interpret Footage…"), lambda: self.run.emit("clip.interpretFootage", {"items": [item]}))
        if row.get("media"):
            add("makeSubclip", N_("Make Subclip…"), select_then("clip.makeSubclip", dialog=True))
        if row.get("subclip"):
            add("editSubclip", N_("Edit Subclip…"), select_then("clip.editSubclip", dialog=True))
            add("convertToMaster", N_("Convert to Master Clip"), select_then("clip.editSubclip", {"item": item, "convertToMaster": True}))
        labels = m.addMenu(tr("Label"))
        labels.setObjectName("project.itemMenu.label")
        for name, color in LABEL_COLORS.items():
            title = name.capitalize()
            a = labels.addAction(kind_icon("bin", QColor(color), 12), tr(title))
            a.setObjectName(f"project.itemMenu.label.{title}")
            a.triggered.connect(select_then("edit.label", {"label": title}))
        add("file.reveal", N_("Reveal in Finder"), lambda: self.command.emit("file.reveal", {"items": [item]}), bool(row.get("file")))
        if row.get("file") and row.get("media"):
            m.addSeparator()
            add("linkMedia", N_("Link Media…"), select_then("media.linkMedia", dialog=True))
            add("makeOffline", N_("Make Offline…"), select_then("media.makeOffline", dialog=True))
            proxy = m.addMenu(tr("Proxy"))
            proxy.setObjectName("project.itemMenu.proxy")
            for label, cid in (
                (N_("Create Proxies…"), "media.createProxies"),
                (N_("Attach Proxies…"), "media.attachProxies"),
                (N_("Reconnect Full Resolution Media…"), "media.reconnectFullRes"),
                (N_("Detach Proxies"), "media.detachProxies"),
            ):
                a = proxy.addAction(tr(label))
                a.setObjectName(f"project.itemMenu.{cid}")
                a.triggered.connect(select_then(cid, dialog=True))
        m.addSeparator()
        add("clear", N_("Clear"), lambda: self.command.emit("project.delete", {"items": [item]}))
        return m

    def card_menu(self, view: BinView, item: int, row: dict[str, Any]) -> QMenu:
        """A Freeform card's menu: Stack, Unstack, Clip Size, then the item's own menu."""
        sel = view.selected_items()
        items = sel if item in sel else [item]
        m = self.item_menu(view, item, row)
        first = m.actions()[0] if m.actions() else None
        entries: list[QAction] = []

        def add(key: str, label: str, cid: str, params: dict[str, Any], enabled: bool = True, menu: QMenu | None = None) -> None:
            a = QAction(tr(label), menu or m)
            a.setObjectName(f"{view.prefix}.cardMenu.{key}")
            a.setEnabled(enabled)
            a.triggered.connect(lambda _c=False: self.command.emit(cid, params))
            if menu is not None:
                menu.addAction(a)
            else:
                entries.append(a)

        add("stack", N_("Stack"), "project.freeform.stack", {"items": items}, len(items) > 1)
        add("unstack", N_("Unstack"), "project.freeform.unstack", {"items": items}, view.canvas.stack_of(item) is not None)
        m.insertActions(first, entries)
        size = QMenu(tr("Clip Size"), m)
        size.setObjectName(f"{view.prefix}.cardMenu.clipSize")
        add("larger", N_("Larger"), "project.freeform.resize", {"items": items, "step": 1}, menu=size)
        add("smaller", N_("Smaller"), "project.freeform.resize", {"items": items, "step": -1}, menu=size)
        card = (self.view.get("freeform") or {}).get("cardSize") if isinstance(self.view.get("freeform"), dict) else None
        default = card if isinstance(card, (int, float)) and not isinstance(card, bool) else 128
        add("default", N_("Default"), "project.freeform.resize", {"items": items, "size": default}, menu=size)
        m.insertMenu(first, size)
        m.insertSeparator(first)
        return m

    def canvas_menu(self, view: BinView) -> QMenu:
        """The Freeform background's menu: grid, arrangements, options, then the usual entries."""
        m = self.background_menu(view)
        first = m.actions()[0] if m.actions() else None
        bin_id = view.data.get("bin")
        top = QMenu(m)

        def add(menu: QMenu, key: str, label: str, fn: Any, enabled: bool = True) -> None:
            a = menu.addAction(tr(label))
            a.setObjectName(f"{view.prefix}.canvasMenu.{key}")
            a.setEnabled(enabled)
            a.triggered.connect(fn)

        add(top, "alignToGrid", N_("Align to Grid"), lambda: self.command.emit("project.freeform.alignToGrid", {"bin": bin_id}))
        add(top, "resetToGrid", N_("Reset to Grid"), lambda: self.command.emit("project.freeform.reset", {"bin": bin_id}))
        top.addSeparator()
        add(top, "saveArrangement", N_("Save Arrangement…"), lambda: self.save_arrangement(view))
        for key, label, cid in (
            ("restoreArrangement", N_("Restore Arrangement"), "project.freeform.restoreArrangement"),
            ("deleteArrangement", N_("Delete Arrangement"), "project.freeform.deleteArrangement"),
        ):
            sub = top.addMenu(tr(label))
            sub.setObjectName(f"{view.prefix}.canvasMenu.{key}")
            sub.setEnabled(bool(view.arrangements))
            for name in view.arrangements:
                a = sub.addAction(name)
                a.triggered.connect(lambda _c=False, n=name, c=cid: self.command.emit(c, {"name": n, "bin": bin_id}))
        top.addSeparator()
        add(top, "options", N_("Freeform View Options…"), self.freeform_options)
        top.addSeparator()
        m.insertActions(first, top.actions())
        return m

    def save_arrangement(self, view: BinView) -> None:
        b = view.data.get("bin")
        dlg = NameDialog("saveArrangement", tr("Save Arrangement"), fmt("Arrangement {n}", n=len(view.arrangements) + 1),
                         bin_id=b if isinstance(b, int) else None, parent=self)

        def ok() -> None:
            if dlg.text():
                self.command.emit("project.freeform.saveArrangement", {"name": dlg.text(), "bin": dlg.bin})

        self._open_dialog(dlg, ok)

    def freeform_options(self) -> None:
        opts = self.view.get("freeform") if isinstance(self.view.get("freeform"), dict) else {}
        dlg = FreeformOptionsDialog(opts, self)
        self._open_dialog(dlg, lambda: self.set_view_cmd("project.freeform.options", dlg.values()))

    def bin_menu(self, view: BinView, bin_id: int) -> QMenu:
        m = QMenu(self)
        m.setObjectName(f"{view.prefix}.binMenu")
        for how, label in OPEN_HOW:
            a = m.addAction(tr(label))
            a.setObjectName(f"{view.prefix}.binMenu.{how}")
            a.triggered.connect(lambda _c=False, h=how: self.open_bin(view, bin_id, h))
        m.addSeparator()
        a = m.addAction(tr("Rename"))
        a.setObjectName(f"{view.prefix}.binMenu.rename")
        a.triggered.connect(lambda: view.rename(None, bin_id))
        a = m.addAction(tr("New Bin"))
        a.setObjectName(f"{view.prefix}.binMenu.newBin")
        a.triggered.connect(lambda: self.new_bin(view, bin_id))
        m.addSeparator()
        a = m.addAction(tr("Clear"))
        a.setObjectName(f"{view.prefix}.binMenu.clear")
        a.triggered.connect(lambda: self.command.emit("project.delete", {"items": [bin_id]}))
        return m

    def background_menu(self, view: BinView) -> QMenu:
        m = QMenu(self)
        m.setObjectName(f"{view.prefix}.backgroundMenu")
        a = m.addAction(tr("New Bin"))
        a.triggered.connect(lambda: self.new_bin(view, view.parent_param()))
        for label, cid in ((N_("New Search Bin"), "file.newSearchBin"), (N_("Find…"), "edit.find"), (N_("Automate to Sequence…"), "clip.automateToSequence"), (N_("Import…"), "file.import")):
            a = m.addAction(tr(label))
            a.triggered.connect(lambda _c=False, c=cid: self.invoke.emit(c))
        sub = m.addMenu(tr("New Item"))
        view._fill_new_item(sub)
        return m

    def fill_panel_menu(self, m: QMenu, view: BinView) -> None:
        """The Project panel's ≡ menu (egui's ``panel_menu``)."""
        m.clear()
        has_sel = bool(view.selected_items()) or view.selected_bin is not None

        def item(key: str, label: str, fn: Any, enabled: bool = True, checked: bool | None = None, shortcut: str = "") -> QAction:
            a = m.addAction(tr(label))
            a.setObjectName(f"project.menu.{key}")
            a.setEnabled(enabled)
            if checked is not None:
                a.setCheckable(True)
                a.setChecked(checked)
            if shortcut:
                a.setShortcut(shortcut)
                a.setShortcutVisibleInContextMenu(True)
            if fn is not None:
                a.triggered.connect(fn)
            return a

        item("closeProject", N_("Close Project"), lambda: self.invoke.emit("file.closeProject"))
        item("closeAllOtherProjects", N_("Close All Other Projects"), None, False)
        item("saveProject", N_("Save Project"), lambda: self.invoke.emit("file.save"))
        item("refreshProject", N_("Refresh Project"), None, False)
        item("revealProject", N_("Reveal Project in Finder…"), self.reveal_project, bool((self.project or {}).get("path")))
        m.addSeparator()
        item("newBin", N_("New Bin"), lambda: self.new_bin(view, view.parent_param()))
        item("newBinFromSelection", N_("New Bin From Selection"), lambda: self.command.emit("file.newBinFromSelection", {}), bool(view.selected_items()))
        item("newSearchBin", N_("New Search Bin"), lambda: self.invoke.emit("file.newSearchBin"))
        item("newProjectShortcut", N_("New Project Shortcut"), None, False)
        item("rename", N_("Rename"), lambda: view.rename(), has_sel)
        item("delete", N_("Delete"), lambda: self.clear(view), has_sel)
        m.addSeparator()
        item("automateToSequence", N_("Automate to Sequence…"), lambda: self.invoke.emit("clip.automateToSequence"), bool(view.selected_items()))
        item("find", N_("Find…"), lambda: self.invoke.emit("edit.find"))
        m.addSeparator()
        for key, label in (("list", N_("List")), ("icon", N_("Icon")), ("freeform", N_("Freeform"))):
            item(f"view.{key}", label, lambda _c=False, k=key: view.set_mode(k), True, view.mode == key)
        m.addSeparator()
        for key, label, shortcut in TOGGLES:
            on = bool(self.view.get(key))
            item(key, label, lambda _c=False, k=key, v=not on: self.set_view({k: v}), True, on, shortcut)
        m.addSeparator()
        fs = m.addMenu(tr("Font Size"))
        fs.setObjectName("project.menu.fontSize")
        cur = (self.view.get("view") or {}).get("fontSize")
        for key, label, _pts in FONT_SIZES:
            a = fs.addAction(tr(label))
            a.setObjectName(f"project.menu.fontSize.{key}")
            a.setCheckable(True)
            a.setChecked(cur == key)
            a.triggered.connect(lambda _c=False, k=key: self.set_view({"fontSize": k}))
        m.addSeparator()
        sort = (self.view.get("view") or {}).get("sort") or {}
        item("refreshSortOrder", N_("Refresh Sort Order"), lambda: self.set_view_cmd("project.sort", {"column": sort.get("column", "Name"), "descending": bool(sort.get("descending"))}))
        m.addSeparator()
        item("metadataDisplay", N_("Metadata Display…"), self.metadata_display)
        current = self.view.get("currentPreset")
        filled = [p for p in self.presets if isinstance(p, dict) and p.get("name")]
        item("saveViewPreset", N_("Save Current View Preset"), lambda: self.set_view_cmd("project.viewPreset.save", {}), isinstance(current, int) and any(p.get("slot") == current for p in filled))
        item("saveViewPresetAs", N_("Save As New View Preset"), self.save_preset_as)
        restore = m.addMenu(tr("Restore View Preset"))
        restore.setObjectName("project.menu.restoreViewPreset")
        restore.setEnabled(bool(filled))
        for p in self.presets:
            if not isinstance(p, dict):
                continue
            slot = p.get("slot")
            a = restore.addAction(str(p.get("name") or fmt("Project View Preset {n}", n=slot)))
            a.setObjectName(f"project.menu.restoreViewPreset.{slot}")
            a.setEnabled(bool(p.get("name")))
            a.setCheckable(True)
            a.setChecked(current == slot)
            a.triggered.connect(lambda _c=False, s=slot: self.set_view_cmd("project.viewPreset.restore", {"slot": s}))
        item("manageViewPresets", N_("Manage Saved View Presets"), self.manage_presets, bool(filled))
        m.addSeparator()
        item("freeformOptions", N_("Freeform View Options…"), self.freeform_options, view.mode == "freeform")
        self._load_presets()

    def _load_presets(self) -> None:
        engine = self.engine
        if engine is None or not getattr(engine, "running", False):
            return

        def got(r: Any, _b: Any) -> None:
            self.presets = [p for p in (r or {}).get("presets") or [] if isinstance(p, dict)] if isinstance(r, dict) else []
            self._got_presets()

        engine.execute("project.viewPreset.list", {}, on_reply=got, on_error=lambda _m: None)

    def _got_presets(self) -> None:
        if isinstance(self.dialog, ManagePresetsDialog):
            self.dialog.set_presets(self.presets)

    def preset_cmd(self, cid: str, params: dict[str, Any]) -> None:
        """A Manage Saved View Presets action, then the slots again (the engine answers in order)."""
        self.set_view_cmd(cid, params)
        self._load_presets()

    def save_preset_as(self) -> None:
        n = next((p.get("slot") for p in self.presets if isinstance(p, dict) and not p.get("name")), 1)
        dlg = NameDialog("savePresetAs", tr("Save As New View Preset"), fmt("Project View Preset {n}", n=n),
                         tr("Saves the view, columns, sort, thumbnail and font size."), parent=self)

        def ok() -> None:
            if dlg.text():
                self.set_view_cmd("project.viewPreset.saveAs", {"name": dlg.text()})

        self._open_dialog(dlg, ok)

    def manage_presets(self) -> None:
        self._open_dialog(ManagePresetsDialog(self, self.presets), lambda: None)
        self._load_presets()

    # ----- the open dialog (egui's ``project_panel.dialog``) -----
    def _open_dialog(self, dlg: QDialog, on_ok: Any) -> None:
        """Show one of the panel's dialogs without blocking (the control channel reads and fills
        it while it is open); OK runs ``on_ok``. Opening one closes the one open before."""
        self.close_dialog()
        self.dialog = dlg
        dlg.accepted.connect(on_ok)
        dlg.finished.connect(lambda _r, d=dlg: self._dialog_closed(d))
        dlg.open()

    def _dialog_closed(self, dlg: QDialog) -> None:
        if self.dialog is dlg:
            self.dialog = None
        dlg.deleteLater()

    def close_dialog(self) -> None:
        dlg, self.dialog = self.dialog, None
        if dlg is not None:
            dlg.reject()

    def dialog_state(self) -> dict[str, Any] | None:
        dlg = self.dialog
        return {dlg.KIND: dlg.state()} if dlg is not None else None  # type: ignore[attr-defined]

    def set_dialog_state(self, value: Any) -> None:
        """egui's ``ui.set {projectPanel: {dialog}}``: null closes the open dialog; ``{kind:
        fields}`` opens that dialog (unless it is the one open) and fills it in."""
        if value is None:
            self.close_dialog()
            return
        openers = {
            "metadataDisplay": self.metadata_display,
            "savePresetAs": self.save_preset_as,
            "managePresets": self.manage_presets,
            "freeformOptions": self.freeform_options,
            "saveArrangement": lambda: self.save_arrangement(self.shown()),
        }
        if not isinstance(value, dict) or len(value) != 1 or next(iter(value)) not in openers:
            raise ValueError(f"dialog takes null or {{kind: fields}} with kind one of {', '.join(openers)}")
        kind, fields = next(iter(value.items()))
        opened = self.dialog is None or getattr(self.dialog, "KIND", None) != kind
        if opened:
            openers[kind]()
        try:
            self.dialog.set_state(fields)  # type: ignore[union-attr]
        except (ValueError, TypeError):
            if opened:
                self.close_dialog()
            raise
