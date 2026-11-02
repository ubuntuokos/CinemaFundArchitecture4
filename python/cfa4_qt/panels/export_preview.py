"""The Export window's preview column (egui's ``export_mode::preview_column``): the sequence's
frame at the playhead in the output frame's aspect, fitted as the Scaling setting fits it (black
bars, cropped or stretched), "Audio only" for sound-only formats, the Scaling choice and the
Summary (output file, format, video, audio, source, range) with the estimated file size.

Automation ids as egui's: ``export.preview``, ``export.scaling``, ``export.summaryBox``
(egui's ``export.summary`` is the Qt window's one-line summary) and ``export.estimate``.
"""

from __future__ import annotations

from typing import Any

from PySide6.QtCore import QRectF, QSize, Qt
from PySide6.QtGui import QColor, QImage, QPainter, QPaintEvent
from PySide6.QtWidgets import QComboBox, QFormLayout, QGroupBox, QLabel, QSizePolicy, QWidget

from ..i18n import N_, fmt, tr

SCALINGS = (("scaleToFit", N_("Scale To Fit")), ("scaleToFill", N_("Scale To Fill")), ("stretchToFill", N_("Stretch To Fill")))


def fit(area: QRectF, w: float, h: float) -> QRectF:
    """The largest ``w``×``h`` rectangle centred in ``area``."""
    if w <= 0 or h <= 0 or area.width() <= 0 or area.height() <= 0:
        return QRectF(area)
    s = min(area.width() / w, area.height() / h)
    return QRectF(area.center().x() - w * s / 2, area.center().y() - h * s / 2, w * s, h * s)


def picture_rect(pic: QRectF, src_w: float, src_h: float, scaling: str) -> QRectF:
    """Where the sequence picture lands in the output frame ``pic`` (egui's preview: Scale To Fit
    letterboxes, Scale To Fill crops, Stretch To Fill distorts)."""
    if scaling == "stretchToFill" or src_w <= 0 or src_h <= 0:
        return QRectF(pic)
    if scaling == "scaleToFill":
        s = max(pic.width() / src_w, pic.height() / src_h)
        return QRectF(pic.center().x() - src_w * s / 2, pic.center().y() - src_h * s / 2, src_w * s, src_h * s)
    return fit(pic, src_w, src_h)


class PreviewView(QWidget):
    """The output frame with the sequence picture in it."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("export.preview")
        self.setMinimumHeight(160)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Preferred)
        self.image: QImage | None = None
        self.source = (0, 0)  # the sequence frame's size
        self.output = (0, 0)  # the export's frame size
        self.scaling = "scaleToFit"
        self.audio_only = False

    def sizeHint(self) -> QSize:
        return QSize(360, 220)

    def pic(self) -> QRectF:
        area = QRectF(self.rect()).adjusted(2, 2, -2, -2)
        w, h = self.output if self.output[0] > 0 and self.output[1] > 0 else self.source
        return fit(area, float(w), float(h)) if w > 0 and h > 0 else area

    def picture(self) -> QRectF:
        return picture_rect(self.pic(), float(self.source[0]), float(self.source[1]), self.scaling)

    def paintEvent(self, _e: QPaintEvent) -> None:
        p = QPainter(self)
        pic = self.pic()
        p.fillRect(pic, QColor(0, 0, 0))
        if self.audio_only:
            p.setPen(self.palette().placeholderText().color())
            p.drawText(pic, Qt.AlignmentFlag.AlignCenter, tr("Audio only"))
        elif self.image is not None and not self.image.isNull():
            p.setRenderHint(QPainter.RenderHint.SmoothPixmapTransform)
            p.setClipRect(pic)
            p.drawImage(self.picture(), self.image)
        p.end()


class PreviewColumn(QWidget):
    """The preview, Scaling and Summary under it; the Export window fills them in."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        lay = QFormLayout(self)
        lay.setContentsMargins(0, 0, 0, 0)
        self.view = PreviewView(self)
        lay.addRow(self.view)
        self.scaling = QComboBox(self)
        self.scaling.setObjectName("export.scaling")
        for value, text in SCALINGS:
            self.scaling.addItem(tr(text), value)
        self.scaling.currentIndexChanged.connect(self._scaling_changed)
        lay.addRow(tr("Scaling"), self.scaling)
        self.form = lay
        box = QGroupBox(tr("Summary"), self)
        box.setObjectName("export.summaryBox")
        f = QFormLayout(box)
        self.lines: dict[str, QLabel] = {}
        for key, label in (("output", N_("Output")), ("format", ""), ("video", ""), ("audio", ""), ("source", N_("Source")), ("range", N_("Range"))):
            v = QLabel(box)
            v.setObjectName(f"export.summary.{key}")
            v.setWordWrap(key in ("output", "video"))
            v.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
            f.addRow(tr(label) if label else "", v)
            self.lines[key] = v
        lay.addRow(box)
        self.estimate = QLabel(self)
        self.estimate.setObjectName("export.estimate")
        self.estimate.setEnabled(False)  # the theme's dim text
        lay.addRow(self.estimate)

    def _scaling_changed(self) -> None:
        self.view.scaling = str(self.scaling.currentData() or "scaleToFit")
        self.view.update()

    def set_video(self, video: bool) -> None:
        self.view.audio_only = not video
        self.form.setRowVisible(self.scaling, video)
        self.view.update()

    def set_frame(self, image: QImage | None, full: tuple[int, int]) -> None:
        self.view.image = image
        if full[0] > 0 and full[1] > 0:
            self.view.source = full
        self.view.update()

    def show_resolved(self, r: dict[str, Any], path: str, range_label: str, keep_scaling: bool) -> None:
        """Fill the Summary from an ``export.resolve`` answer; the Scaling shown follows it unless
        ``keep_scaling`` (the user chose one here)."""
        out = r.get("output") if isinstance(r.get("output"), dict) else {}
        w, h = out.get("width"), out.get("height")
        if isinstance(w, int) and isinstance(h, int):
            self.view.output = (w, h)
            self.view.update()
        s = r.get("summary") if isinstance(r.get("summary"), dict) else {}
        self.lines["output"].setText(path)
        for key in ("format", "video", "audio"):
            self.lines[key].setText(str(s.get(key) or ""))
        self.lines["source"].setText(str(r.get("source") or ""))
        rng = r.get("range") if isinstance(r.get("range"), dict) else {}
        secs = rng.get("seconds")
        self.lines["range"].setText(f"{range_label} ({float(secs):.2f} s)" if isinstance(secs, (int, float)) else range_label)
        size = s.get("estimated_size")
        self.estimate.setText(fmt("Estimated file size: {size}", size=size if isinstance(size, str) and size else "–"))
        sc = (r.get("settings") or {}).get("scaling") if isinstance(r.get("settings"), dict) else None
        i = self.scaling.findData(sc)
        if i >= 0 and i != self.scaling.currentIndex() and not keep_scaling:
            self.scaling.blockSignals(True)
            self.scaling.setCurrentIndex(i)
            self.scaling.blockSignals(False)
            self._scaling_changed()
