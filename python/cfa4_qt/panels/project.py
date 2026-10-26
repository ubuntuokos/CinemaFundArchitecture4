"""Project panel (egui's ``panels/project.rs`` and ``project_views.rs``): the project's bins and
items in List view (the Metadata Display columns, sortable and resizable, bins twirled open) or
Icon view (thumbnail cards with hover scrub, the In/Out bar and badges, Sort Icons), with the
breadcrumb and Up one level, the search and item count, the Preview Area, inline rename (F2, a
slow second click, the menus), bins opened in place, in a tab or in a window, and the footer and
panel menu (views, thumbnail size, New Item, New Bin, Find, Clear, Font Size, view presets).

What a view shows comes from ``project.panel`` (``apps/cfa4-cli/src/serve_project.rs``); the
view settings are engine preferences (``project.view.*``, ``project.columns.*``, ``project.sort``,
``project.viewPreset.*``). Without that method (an older engine) the bin tree of
``project.inspect`` is shown as a plain list.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from PySide6.QtCore import QMimeData, QModelIndex, QPoint, QPointF, QRect, QRectF, QSize, Qt, QTimer, QUrl, Signal
from PySide6.QtGui import QAction, QColor, QDesktopServices, QFont, QIcon, QImage, QPainter, QPainterPath, QPen, QPixmap
from PySide6.QtWidgets import (
    QAbstractItemView,
    QApplication,
    QDialog,
    QDialogButtonBox,
    QHBoxLayout,
    QInputDialog,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QMenu,
    QPushButton,
    QSlider,
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
        text_y = th.bottom() + 12
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
        p.fillRect(th, QColor(12, 12, 12))
        item = row.get("item")
        hover = self.view.hover if self.view.hover_scrub else None
        at = hover[1] if hover is not None and hover[0] == item else None
        img = self.view.thumbnail(row, at, int(th.height()))
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
        dtext = card_duration(dur)
        dw = p.fontMetrics().horizontalAdvance(dtext)
        p.setPen(pal.placeholderText().color())
        p.drawText(QRectF(th.right() - dw, text_y - 9, dw, 18), Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter, dtext)
        if not self.view.editing(index):
            p.fillRect(QRectF(th.left(), text_y - 5, 8, 10), label_color(row))
            p.setPen(OFFLINE if offline else pal.text().color())
            name_r = QRectF(th.left() + 12, text_y - 9, max(0.0, th.width() - dw - 18), 18)
            name = p.fontMetrics().elidedText(str(row.get("name") or ""), Qt.TextElideMode.ElideRight, int(name_r.width()))
            p.drawText(name_r, Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter, name)
        p.restore()


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
        self.expanded: set[int] = set()
        self.selected_bin: int | None = None
        self.hover: tuple[Any, int] | None = None
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

        self.stack = QStackedWidget(self)
        self.stack.addWidget(self.tree)
        self.stack.addWidget(self.icons)

        foot = QHBoxLayout()
        foot.setContentsMargins(4, 0, 4, 0)
        self.view_buttons: dict[str, QToolButton] = {}
        for key, text, tip in (("list", "☰", N_("List View")), ("icon", "▦", N_("Icon View"))):
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

    # ----- state -----
    @property
    def view(self) -> dict[str, Any]:
        v = self.panel.view.get("view")
        return v if isinstance(v, dict) else {}

    @property
    def mode(self) -> str:
        m = self.own_mode if self.own_mode is not None else self.view.get("mode")
        return m if m in ("list", "icon") else "list"

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
        if self.mode == "icon":
            return [it.data(ITEM_ROLE) for it in self.icons.selectedItems() if it.data(ITEM_ROLE) is not None]
        return [it.data(0, ITEM_ROLE) for it in self.tree.selectedItems() if it.data(0, ITEM_ROLE) is not None]

    # ----- data -----
    def refresh(self) -> None:
        engine = self.panel.engine
        params: dict[str, Any] = {"mode": self.mode, "expanded": sorted(self.expanded), "filter": self.filter.text()}
        if self.bin is not None:
            params["bin"] = self.bin
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
        self.data = data if isinstance(data, dict) else {}
        rows = [r for r in self.data.get("rows") or [] if isinstance(r, dict)]
        selection = {i for i in self.data.get("selection") or [] if isinstance(i, int)}
        self._apply_mode()
        self._updating = True
        try:
            if self.mode == "icon" and not self.data.get("fallback"):
                self._fill_icons(rows, selection)
            else:
                self._fill_list(rows, selection)
        finally:
            self._updating = False
        self._update_head(selection)
        if self.top is not None:
            self.panel._update_tab_names()

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
        self.stack.setCurrentWidget(self.icons if mode == "icon" else self.tree)
        self.sort_button.setVisible(mode == "icon")
        self.size_slider.blockSignals(True)
        self.size_slider.setValue(self.icon_size)
        self.size_slider.blockSignals(False)
        font = QFont(self.font())
        font.setPixelSize(round(self.font_points))  # egui's points are logical pixels
        self.tree.setFont(font)
        self.icons.setFont(font)

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
class MetadataDisplayDialog(QDialog):
    """Metadata Display…: the List view's columns, in order (egui's ``project_dialogs``)."""

    def __init__(self, available: list[str], shown: list[str], parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("project.metadataDisplay")
        self.setWindowTitle(tr("Metadata Display"))
        self.list = QListWidget(self)
        self.list.setObjectName("project.metadataDisplay.columns")
        self.list.setDragDropMode(QAbstractItemView.DragDropMode.InternalMove)
        for name in [*shown, *(a for a in available if a not in shown)]:
            it = QListWidgetItem(tr(name))
            it.setData(ITEM_ROLE, name)
            it.setFlags(it.flags() | Qt.ItemFlag.ItemIsUserCheckable)
            it.setCheckState(Qt.CheckState.Checked if name in shown else Qt.CheckState.Unchecked)
            if name == "Name":
                it.setFlags(it.flags() & ~Qt.ItemFlag.ItemIsUserCheckable)
            self.list.addItem(it)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel, self)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        layout = QVBoxLayout(self)
        layout.addWidget(QLabel(tr("Drag to reorder; tick the columns to show."), self))
        layout.addWidget(self.list, 1)
        layout.addWidget(buttons)

    def columns(self) -> list[str]:
        out = []
        for i in range(self.list.count()):
            it = self.list.item(i)
            if it.checkState() == Qt.CheckState.Checked or it.data(ITEM_ROLE) == "Name":
                out.append(str(it.data(ITEM_ROLE)))
        return out


class ManagePresetsDialog(QDialog):
    """Manage Saved View Presets: rename or delete a slot."""

    def __init__(self, panel: ProjectPanel, presets: list[dict[str, Any]]) -> None:
        super().__init__(panel)
        self.panel = panel
        self.setObjectName("project.managePresets")
        self.setWindowTitle(tr("Manage Saved View Presets"))
        self.list = QListWidget(self)
        for p in presets:
            if isinstance(p, dict) and p.get("name"):
                it = QListWidgetItem(f"{p.get('slot')}. {p.get('name')}")
                it.setData(ITEM_ROLE, p.get("slot"))
                it.setData(ROW_ROLE, p.get("name"))
                self.list.addItem(it)
        self.name = QLineEdit(self)
        self.list.currentItemChanged.connect(lambda it, _o: it is not None and self.name.setText(str(it.data(ROW_ROLE) or "")))
        rename = QPushButton(tr("Rename"), self)
        rename.clicked.connect(self._rename)
        delete = QPushButton(tr("Delete"), self)
        delete.clicked.connect(self._delete)
        close = QDialogButtonBox(QDialogButtonBox.StandardButton.Close, self)
        close.rejected.connect(self.reject)
        row = QHBoxLayout()
        row.addWidget(self.name, 1)
        row.addWidget(rename)
        row.addWidget(delete)
        layout = QVBoxLayout(self)
        layout.addWidget(self.list, 1)
        layout.addLayout(row)
        layout.addWidget(close)

    def _slot(self) -> int | None:
        it = self.list.currentItem()
        return it.data(ITEM_ROLE) if it is not None else None

    def _rename(self) -> None:
        slot, name = self._slot(), self.name.text().strip()
        if isinstance(slot, int) and name:
            self.panel.set_view_cmd("project.viewPreset.rename", {"slot": slot, "name": name})
            it = self.list.currentItem()
            it.setText(f"{slot}. {name}")
            it.setData(ROW_ROLE, name)

    def _delete(self) -> None:
        slot = self._slot()
        if isinstance(slot, int):
            self.panel.set_view_cmd("project.viewPreset.delete", {"slot": slot})
            self.list.takeItem(self.list.currentRow())


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
        if dlg.exec() == QDialog.DialogCode.Accepted:
            self.set_view_cmd("project.columns.set", {"columns": dlg.columns()})

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
        for key, label in (("list", N_("List")), ("icon", N_("Icon"))):
            item(f"view.{key}", label, lambda _c=False, k=key: view.set_mode(k), True, view.mode == key)
        item("view.freeform", N_("Freeform"), None, False, False)
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
        item("freeformOptions", N_("Freeform View Options…"), None, False)
        self._load_presets()

    def _load_presets(self) -> None:
        engine = self.engine
        if engine is None or not getattr(engine, "running", False):
            return

        def got(r: Any, _b: Any) -> None:
            self.presets = [p for p in (r or {}).get("presets") or [] if isinstance(p, dict)] if isinstance(r, dict) else []

        engine.execute("project.viewPreset.list", {}, on_reply=got, on_error=lambda _m: None)

    def save_preset_as(self) -> None:
        n = next((p.get("slot") for p in self.presets if isinstance(p, dict) and not p.get("name")), 1)
        name, ok = QInputDialog.getText(self, tr("Save As New View Preset"), tr("Name:"), text=fmt("Project View Preset {n}", n=n))
        if ok and name.strip():
            self.set_view_cmd("project.viewPreset.saveAs", {"name": name.strip()})

    def manage_presets(self) -> None:
        ManagePresetsDialog(self, self.presets).exec()
        self._load_presets()
