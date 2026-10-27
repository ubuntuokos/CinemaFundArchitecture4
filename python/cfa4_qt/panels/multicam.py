"""Multi-camera editing, as egui's ``panels/multicam.rs``: the Program monitor's Multi-Camera view
(the angle grid beside the program), live switching while playing, and the Synchronize Clips,
Merge Clips, Create Multi-Camera Source Sequence and Edit Cameras dialogs.

The grid is one engine picture (``multicam.frame``): every shown angle of the multi-camera clip at
the Program monitor's time, rendered at its cell's size and tiled. Clicking an angle (or keys
1–9) cuts to it while recording (playing with Multi-Camera Record on) and switches the clip at the
playhead when stopped; Ctrl+click switches the video only.

Automation ids (egui's):
- Multi-Camera view: ``program.multicam.grid``, ``program.multicam.angle.<n>`` (n = 1-based camera
  in shown order), ``program.multicam.record``, ``program.multicam.pagePrev``,
  ``program.multicam.pageNext`` (when there is more than one page).
- Edit Cameras dialog (``editCameras``): ``editCameras.name.<angle>``, ``editCameras.enabled.<angle>``,
  ``editCameras.thumb.<angle>``, ``editCameras.ok``, ``editCameras.cancel``.
- Synchronize: ``sync.method.<in|out|timecode|marker|audio>``, ``sync.ignoreHours``, ``sync.marker``,
  ``sync.offset``, ``sync.track``, ``sync.ok``, ``sync.cancel``.
- Merge Clips: ``merge.name``, ``merge.method.<…>``, ``merge.ignoreHours``, ``merge.marker``,
  ``merge.offset``, ``merge.removeVideoAudio``, ``merge.ok``, ``merge.cancel``.
- Create Multi-Camera Source Sequence: ``mcam.name``, ``mcam.method.<…>``, ``mcam.ignoreHours``,
  ``mcam.marker``, ``mcam.offset``, ``mcam.audio.<camera1|all|switch>``,
  ``mcam.cameraNames.<clip|track|metadata>``, ``mcam.processedBin``, ``mcam.ok``, ``mcam.cancel``.
UI commands ``multicam.toggleView`` (Shift+0), ``multicam.recordToggle`` (0) and
``multicam.editCamerasDialog``.
"""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from PySide6.QtCore import QObject, QPointF, QRectF, Qt, Signal
from PySide6.QtGui import QColor, QImage, QMouseEvent, QPainter, QPaintEvent, QPen, QPixmap
from PySide6.QtWidgets import (
    QButtonGroup,
    QCheckBox,
    QDialog,
    QGroupBox,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPushButton,
    QRadioButton,
    QScrollArea,
    QSpinBox,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from .. import theme
from ..engine import EngineClient
from ..i18n import N_, fmt, tr

YELLOW = QColor(0xF2, 0xC9, 0x4C)  # the active angle
RED = QColor(0xE0, 0x3A, 0x3A)  # the active angle while recording
DIM_RED = QColor(0x9A, 0x3A, 0x3A)  # Multi-Camera Record on, not recording
PAGE_BAR = 24
# Synchronize Point choices: (key, label)
METHODS: tuple[tuple[str, str], ...] = (
    ("in", N_("In Points")),
    ("out", N_("Out Points")),
    ("timecode", N_("Timecode")),
    ("marker", N_("Clip Marker")),
    ("audio", N_("Audio")),
)


def rgba_image(result: Any, data: bytes | None) -> QImage | None:
    """The RGBA8 picture of a ``multicam.frame`` / ``multicam.angleFrame`` reply."""
    if not isinstance(result, dict) or not data:
        return None
    w, h = result.get("w"), result.get("h")
    if not isinstance(w, int) or not isinstance(h, int) or w <= 0 or h <= 0 or len(data) != w * h * 4:
        return None
    return QImage(data, w, h, w * 4, QImage.Format.Format_RGBA8888).copy()


def whole(value: Any, default: int = 0) -> int:
    return value if isinstance(value, int) and not isinstance(value, bool) else default


class AngleCell(QWidget):
    """One angle of the grid: a click cuts to it (Ctrl: the video only)."""

    def __init__(self, grid: MulticamGrid, camera: int) -> None:
        super().__init__(grid)
        self.grid = grid
        self.camera = camera
        self.setObjectName(f"program.multicam.angle.{camera}")
        self.setAttribute(Qt.WidgetAttribute.WA_Hover)
        self.setMouseTracking(True)
        self.setCursor(Qt.CursorShape.PointingHandCursor)

    def mousePressEvent(self, e: QMouseEvent) -> None:
        if e.button() == Qt.MouseButton.LeftButton:
            ctrl = bool(e.modifiers() & (Qt.KeyboardModifier.ControlModifier | Qt.KeyboardModifier.MetaModifier))
            self.grid.cut.emit(self.camera, ctrl)
            e.accept()
            return
        super().mousePressEvent(e)

    def enterEvent(self, e: Any) -> None:
        super().enterEvent(e)
        self.update()

    def leaveEvent(self, e: Any) -> None:
        super().leaveEvent(e)
        self.update()

    def paintEvent(self, _e: QPaintEvent) -> None:
        if self.underMouse():
            p = QPainter(self)
            p.setPen(QPen(QColor(255, 255, 255, 90), 1.0))
            p.drawRect(QRectF(self.rect()).adjusted(1.5, 1.5, -1.5, -1.5))


class MulticamGrid(QWidget):
    """The Multi-Camera view's angle grid (left of the program, or the whole picture area without
    Show Multi-Camera Preview Monitor)."""

    cut = Signal(object, bool)  # camera (1-based, in shown order), video only
    page = Signal(str)  # multicam.prevPage / multicam.nextPage
    record_clicked = Signal()
    info_changed = Signal()  # a grid reply came in (the Multi-Camera view settings with it)

    def __init__(self, engine: EngineClient, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("program.multicam.grid")
        self.setMinimumWidth(120)
        self.engine = engine
        self.info: dict[str, Any] = {}
        self.image: QImage | None = None
        self.record_on = True  # Multi-Camera Record (egui's default)
        self.cells: list[AngleCell] = []
        self._busy = False
        self._wanted: tuple[int, bool, float, int] | None = None
        self._shown: tuple[Any, ...] | None = None
        self.record_button = QToolButton(self)
        self.record_button.setObjectName("program.multicam.record")
        self.record_button.setToolTip(tr("Multi-Camera Record On/Off (0)"))
        self.record_button.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        self.record_button.setAutoRaise(True)
        self.record_button.clicked.connect(self.record_clicked)
        self.prev_button = self._page_button("program.multicam.pagePrev", "◀", N_("Previous page"), "multicam.prevPage")
        self.next_button = self._page_button("program.multicam.pageNext", "▶", N_("Next page"), "multicam.nextPage")
        self.page_label = QLabel("", self)
        self.page_label.setObjectName("program.multicam.pageLabel")
        self.page_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._restyle_record()
        self._place()

    def _page_button(self, name: str, text: str, tip: str, command: str) -> QToolButton:
        b = QToolButton(self)
        b.setObjectName(name)
        b.setText(text)
        b.setToolTip(tr(tip))
        b.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        b.clicked.connect(lambda: self.page.emit(command))
        return b

    # ----- state -----
    @property
    def recording(self) -> bool:
        return bool(self.info.get("recording"))

    @property
    def has_clip(self) -> bool:
        return isinstance(self.info.get("source"), int)

    def view_settings(self) -> dict[str, Any]:
        v = self.info.get("view")
        return v if isinstance(v, dict) else {}

    def set_record(self, on: bool) -> None:
        self.record_on = bool(on)
        self._restyle_record()

    def _restyle_record(self) -> None:
        if self.recording:
            dot, text = RED, "REC"
        elif self.record_on:
            dot, text = DIM_RED, tr("Record")
        else:
            dot, text = QColor(theme.current().text_faint), tr("Record")
        pm = QPixmap(10, 10)
        pm.fill(Qt.GlobalColor.transparent)
        p = QPainter(pm)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        p.setPen(Qt.PenStyle.NoPen)
        p.setBrush(dot)
        p.drawEllipse(QPointF(5, 5), 4.5, 4.5)
        p.end()
        self.record_button.setIcon(pm)
        self.record_button.setText(text)
        self.record_button.setToolButtonStyle(Qt.ToolButtonStyle.ToolButtonTextBesideIcon)
        self.record_button.setStyleSheet("QToolButton { color: white; background: rgba(0,0,0,140); border-radius: 3px; padding: 1px 6px; }")
        self.record_button.adjustSize()

    # ----- frames -----
    def invalidate(self) -> None:
        """The grid's layout, page or cameras changed: ask again even at the same time."""
        self._shown = None

    def request(self, t: int, playing: bool, scale: float, revision: int) -> None:
        self._wanted = (int(t), bool(playing), float(scale), int(revision))
        self._pump()

    def _pump(self) -> None:
        if self._busy or self._wanted is None or not self.engine.running:
            return
        t, playing, scale, revision = self._wanted
        self._wanted = None
        dpr = max(1.0, self.devicePixelRatioF())
        area = self.grid_area(paged=int(self.info.get("pages") or 1) > 1)
        width, height = max(16, int(area.width() * dpr)), max(16, int(area.height() * dpr))
        key = (t, playing, scale, revision, width, height)
        if key == self._shown:
            return
        self._busy = True
        self.engine.call(
            "multicam.frame",
            {"time": t, "maxWidth": min(8192, width), "maxHeight": min(8192, height), "scale": scale, "playing": playing},
            on_reply=lambda r, b, k=key: self._on_frame(k, r, b),
            on_error=self._on_error,
        )

    def _on_frame(self, key: tuple[Any, ...], result: Any, data: bytes | None) -> None:
        self._busy = False
        if isinstance(result, dict):
            self.info = result
            self.image = rgba_image(result, data)
            self._shown = key
            self._restyle_record()
            self._place()
            self.update()
            self.info_changed.emit()
        self._pump()

    def _on_error(self, _message: str) -> None:
        self._busy = False
        self.info = {}
        self.image = None
        self._place()
        self.update()
        self._pump()

    # ----- layout -----
    def grid_area(self, paged: bool) -> QRectF:
        r = QRectF(self.rect())
        return r.adjusted(0, 0, 0, -PAGE_BAR) if paged else r

    def picture_rect(self) -> QRectF:
        """Where the grid picture is drawn: fitted (aspect kept) into the grid area."""
        area = self.grid_area(int(self.info.get("pages") or 1) > 1).adjusted(2, 2, -2, -2)
        img = self.image
        aw, ah = (img.width(), img.height()) if img is not None else (16, 9)
        if aw <= 0 or ah <= 0 or area.width() <= 0 or area.height() <= 0:
            return QRectF()
        k = min(area.width() / aw, area.height() / ah)
        pic = QRectF(0, 0, aw * k, ah * k)
        pic.moveCenter(area.center())
        return pic

    def cell_rects(self) -> list[tuple[dict[str, Any], QRectF]]:
        cols, rows = max(1, whole(self.info.get("cols"), 1)), max(1, whole(self.info.get("rows"), 1))
        pic = self.picture_rect()
        cw, ch = pic.width() / cols, pic.height() / rows
        out = []
        for c in self.info.get("cells") or []:
            if not isinstance(c, dict):
                continue
            i = whole(c.get("cell"))
            if not 0 <= i < cols * rows:
                continue
            out.append((c, QRectF(pic.left() + (i % cols) * cw, pic.top() + (i // cols) * ch, cw, ch)))
        return out

    def _place(self) -> None:
        cells = self.cell_rects() if self.has_clip else []
        while len(self.cells) > len(cells):
            self.cells.pop().deleteLater()
        for k, (c, r) in enumerate(cells):
            camera = whole(c.get("camera"), k + 1)
            if k < len(self.cells):
                cell = self.cells[k]
                if cell.camera != camera:
                    cell.camera = camera
                    cell.setObjectName(f"program.multicam.angle.{camera}")
            else:
                cell = AngleCell(self, camera)
                self.cells.append(cell)
            cell.setGeometry(r.toAlignedRect())
            cell.setToolTip(fmt("Camera {n} — {name}", n=camera, name=str(c.get("name") or "")))
            cell.show()
        self.record_button.move(8, 8)
        self.record_button.raise_()
        pages = whole(self.info.get("pages"), 1)
        paged = self.has_clip and pages > 1
        for w in (self.prev_button, self.next_button, self.page_label):
            w.setVisible(paged)
        if paged:
            page = whole(self.info.get("page"))
            self.page_label.setText(fmt("Page {page} of {pages}", page=page + 1, pages=pages))
            y = self.height() - PAGE_BAR + 2
            mid = self.width() // 2
            self.page_label.setGeometry(mid - 50, y, 100, PAGE_BAR - 4)
            self.prev_button.setGeometry(mid - 83, y, 26, 18)
            self.next_button.setGeometry(mid + 57, y, 26, 18)
            self.prev_button.setEnabled(page > 0)
            self.next_button.setEnabled(page + 1 < pages)

    def resizeEvent(self, e: Any) -> None:
        super().resizeEvent(e)
        self._place()

    def paintEvent(self, _e: QPaintEvent) -> None:
        p = QPainter(self)
        T = theme.current()
        p.fillRect(self.rect(), T.c("panel_bg"))
        if not self.has_clip:
            p.setPen(T.c("text_dim"))
            p.drawText(self.rect(), Qt.AlignmentFlag.AlignCenter | Qt.TextFlag.TextWordWrap, tr("No multi-camera clip at the playhead"))
            return
        pic = self.picture_rect()
        p.fillRect(pic, QColor("#000000"))
        if self.image is not None:
            p.setRenderHint(QPainter.RenderHint.SmoothPixmapTransform)
            p.drawImage(pic, self.image)
        f = p.font()
        f.setPixelSize(11)
        p.setFont(f)
        for c, r in self.cell_rects():
            if c.get("active") is True:
                p.setPen(QPen(RED if self.recording else YELLOW, 3.0))
                p.setBrush(Qt.BrushStyle.NoBrush)
                p.drawRect(r.adjusted(1.5, 1.5, -1.5, -1.5))
            label = f"{whole(c.get('camera'))}  {c.get('name') or ''}"
            m = p.fontMetrics()
            lr = QRectF(r.left() + 6, r.bottom() - m.height() - 12, m.horizontalAdvance(label) + 8, m.height() + 4)
            p.setPen(Qt.PenStyle.NoPen)
            p.setBrush(QColor(0, 0, 0, 160))
            p.drawRoundedRect(lr, 3, 3)
            p.setPen(QColor("#ffffff"))
            p.drawText(lr, Qt.AlignmentFlag.AlignCenter, label)


class MulticamController(QObject):
    """Live switching (egui's on_play / on_stop): a recording pass starts when the Program
    monitor plays forward at normal speed in the Multi-Camera view with Multi-Camera Record on,
    and ends (one undo step) when playback stops. Cuts carry the shown time: while playing, the
    engine's playhead is still where playback started."""

    message = Signal(str, bool)  # text, is an error
    refresh = Signal()

    def __init__(self, engine: EngineClient, monitor: Any) -> None:
        super().__init__(monitor)
        self.engine = engine
        self.monitor = monitor
        self.recording = False
        monitor.playback_started.connect(self._started)
        monitor.playback_stopped.connect(self._stopped)
        grid: MulticamGrid = monitor.grid
        grid.cut.connect(self.cut)
        grid.page.connect(self.view_command)
        grid.record_clicked.connect(self.toggle_record)

    @property
    def grid(self) -> MulticamGrid:
        return self.monitor.grid

    def toggle_record(self, on: bool | None = None) -> None:
        self.grid.set_record(not self.grid.record_on if on is None else bool(on))
        self.message.emit(tr("Multi-Camera Record on") if self.grid.record_on else tr("Multi-Camera Record off"), False)

    def _failed(self, message: str) -> None:
        self.message.emit(message, True)

    def _started(self) -> None:
        m = self.monitor
        if not (m.mv.multicam and self.grid.record_on and not self.recording and m.speed == 1.0 and self.grid.has_clip):
            return
        self.recording = True
        self.engine.execute("multicam.recordStart", {"time": int(m.playhead)}, on_reply=lambda _r, _b: self.monitor.request_frame(), on_error=self._failed)

    def _stopped(self) -> None:
        if not self.recording:
            return
        self.recording = False

        def done(result: Any, _b: bytes | None) -> None:
            n = whole(result.get("cuts")) if isinstance(result, dict) else 0
            if n > 0:
                self.message.emit(fmt("Recorded {n} multi-camera cut(s)", n=n), False)
            self.grid.invalidate()
            self.refresh.emit()

        self.engine.execute("multicam.recordStop", {"time": int(self.monitor.playhead)}, on_reply=done, on_error=self._failed)

    def timed(self, params: dict[str, Any]) -> dict[str, Any]:
        """``params`` with the shown time while the Program monitor plays."""
        return {**params, "time": int(self.monitor.playhead)} if self.monitor.playing else params

    def cut(self, camera: int, video_only: bool = False) -> None:
        self.engine.execute("multicam.cut", self.timed({"camera": int(camera), "videoOnly": bool(video_only)}), on_reply=self._changed, on_error=self._failed)

    def view_command(self, command: str, params: dict[str, Any] | None = None) -> None:
        """A Multi-Camera view setting (page, layout, Selection Top Down …): the grid asks again."""
        self.engine.execute(command, params or {}, on_reply=self._changed, on_error=self._failed)

    def _changed(self, _r: Any = None, _b: bytes | None = None) -> None:
        self.grid.invalidate()
        self.monitor.request_frame()
        self.refresh.emit()


class SyncDialog(QDialog):
    """Synchronize Clips / Merge Clips / Create Multi-Camera Source Sequence (egui's
    ``show_dialog``): ``kind`` is ``synchronize``, ``merge`` or ``multicam``."""

    def __init__(self, parent: QWidget, engine: EngineClient, kind: str, done: Callable[[str], None]) -> None:
        super().__init__(parent)
        self.engine = engine
        self.kind = kind
        self.done_message = done
        self.pre = {"merge": "merge", "multicam": "mcam"}.get(kind, "sync")
        self.command = {"merge": "clip.mergeClips", "multicam": "clip.createMulticam"}.get(kind, "clip.synchronize")
        self.setObjectName(self.pre)
        title = {"merge": N_("Merge Clips"), "multicam": N_("Create Multi-Camera Source Sequence")}.get(kind, N_("Synchronize Clips"))
        self.setWindowTitle(tr(title))
        self.items: list[int] = []
        self.ready = False
        pre = self.pre
        lay = QVBoxLayout(self)
        self.name: QLineEdit | None = None
        self.summary = QLabel("", self)
        self.summary.setObjectName(f"{pre}.summary")
        self.summary.setWordWrap(True)
        self.summary.setEnabled(False)
        if pre != "sync":
            row = QHBoxLayout()
            row.addWidget(QLabel(tr("Clip Name:") if pre == "merge" else tr("Sequence Name:"), self))
            self.name = QLineEdit(self)
            self.name.setObjectName(f"{pre}.name")
            self.name.setMinimumWidth(300)
            row.addWidget(self.name, 1)
            lay.addLayout(row)
        lay.addWidget(self.summary)
        box = QGroupBox(tr("Synchronize Point"), self)
        grid = QVBoxLayout(box)
        self.methods = QButtonGroup(self)
        self.method_buttons: dict[str, QRadioButton] = {}
        self.ignore_hours = QCheckBox(tr("Ignore Hours"), self)
        self.ignore_hours.setObjectName(f"{pre}.ignoreHours")
        self.marker = QLineEdit(self)
        self.marker.setObjectName(f"{pre}.marker")
        self.marker.setPlaceholderText(tr("first marker"))
        self.marker.setMaximumWidth(160)
        for key, label in METHODS:
            row = QHBoxLayout()
            b = QRadioButton(tr(label), self)
            b.setObjectName(f"{pre}.method.{key}")
            b.toggled.connect(self._method_changed)
            self.methods.addButton(b)
            self.method_buttons[key] = b
            row.addWidget(b)
            if key == "timecode":
                row.addWidget(self.ignore_hours)
            elif key == "marker":
                row.addWidget(self.marker)
            row.addStretch(1)
            grid.addLayout(row)
        row = QHBoxLayout()
        row.addWidget(QLabel(tr("Offset Audio by:") if pre == "merge" else tr("Offset:"), self))
        self.offset = QSpinBox(self)
        self.offset.setObjectName(f"{pre}.offset")
        self.offset.setRange(-100_000, 100_000)
        self.offset.setSuffix(tr(" frames"))
        row.addWidget(self.offset)
        row.addStretch(1)
        grid.addLayout(row)
        lay.addWidget(box)
        self.track: QLineEdit | None = None
        self.remove_audio: QCheckBox | None = None
        self.audio: dict[str, QRadioButton] = {}
        self.camera_names: dict[str, QRadioButton] = {}
        self.processed_bin: QCheckBox | None = None
        if pre == "sync":
            row = QHBoxLayout()
            row.addWidget(QLabel(tr("Reference track:"), self))
            self.track = QLineEdit(self)
            self.track.setObjectName("sync.track")
            self.track.setPlaceholderText(tr("lowest (V1…)"))
            self.track.setMaximumWidth(110)
            row.addWidget(self.track)
            row.addStretch(1)
            lay.addLayout(row)
        elif pre == "merge":
            self.remove_audio = QCheckBox(tr("Remove Audio from AV Clip"), self)
            self.remove_audio.setObjectName("merge.removeVideoAudio")
            lay.addWidget(self.remove_audio)
        else:
            box = QGroupBox(tr("Audio"), self)
            v = QVBoxLayout(box)
            self.audio = self._radios(v, "mcam.audio", (("camera1", N_("Camera 1")), ("all", N_("All Cameras")), ("switch", N_("Switch Audio"))), "camera1")
            v.addWidget(QLabel(tr("Camera Names"), self))
            self.camera_names = self._radios(
                v, "mcam.cameraNames", (("clip", N_("Clip Names")), ("track", N_("Enumerate Cameras")), ("metadata", N_("Camera Angle Metadata"))), "clip"
            )
            lay.addWidget(box)
            self.processed_bin = QCheckBox(tr("Move source clips to Processed Clips bin"), self)
            self.processed_bin.setObjectName("mcam.processedBin")
            lay.addWidget(self.processed_bin)
        self.error = QLabel("", self)
        self.error.setObjectName(f"{pre}.message")
        self.error.setStyleSheet("color: #e08a6a;")
        self.error.setWordWrap(True)
        self.error.hide()
        lay.addWidget(self.error)
        row = QHBoxLayout()
        row.addStretch(1)
        cancel = QPushButton(tr("Cancel"), self)
        cancel.setObjectName(f"{pre}.cancel")
        cancel.clicked.connect(self.reject)
        row.addWidget(cancel)
        self.ok = QPushButton(tr("OK"), self)
        self.ok.setObjectName(f"{pre}.ok")
        self.ok.setDefault(True)
        self.ok.clicked.connect(self.apply)
        row.addWidget(self.ok)
        lay.addLayout(row)
        self.method_buttons["in"].setChecked(True)
        self._method_changed()
        engine.call("multicam.selection", {}, on_reply=lambda r, _b: self._load(r), on_error=self.show_error)

    def _radios(self, lay: QVBoxLayout, prefix: str, choices: tuple[tuple[str, str], ...], default: str) -> dict[str, QRadioButton]:
        row = QHBoxLayout()
        group = QButtonGroup(self)
        out: dict[str, QRadioButton] = {}
        for key, label in choices:
            b = QRadioButton(tr(label), self)
            b.setObjectName(f"{prefix}.{key}")
            b.setChecked(key == default)
            group.addButton(b)
            row.addWidget(b)
            out[key] = b
        row.addStretch(1)
        lay.addLayout(row)
        return out

    def method(self) -> str:
        return next((k for k, b in self.method_buttons.items() if b.isChecked()), "in")

    def _method_changed(self, *_a: Any) -> None:
        m = self.method()
        self.ignore_hours.setEnabled(m == "timecode")
        self.marker.setEnabled(m == "marker")

    def _load(self, result: Any) -> None:
        if not isinstance(result, dict):
            return
        items = [i for i in result.get("items") or [] if isinstance(i, dict) and isinstance(i.get("id"), int)]
        self.items = [int(i["id"]) for i in items]
        names = [str(i.get("name") or "") for i in items]
        if self.pre == "sync":
            self.summary.setText(fmt("{n} clip(s) selected; the clip on the reference track stays put.", n=whole(result.get("selected"))))
        else:
            self.summary.setText(fmt("{n} clip(s): {names}", n=len(names), names=", ".join(names)))
            first = names[0] if names else ""
            if self.name is not None and not self.name.text():
                self.name.setText(fmt("{first} - Merged", first=first) if self.pre == "merge" else fmt("{first} Multicam", first=first))
        self.ready = True

    def show_error(self, message: str) -> None:
        self.error.setText(message)
        self.error.show()

    def values(self) -> dict[str, Any]:
        p: dict[str, Any] = {"method": self.method(), "offset": self.offset.value(), "ignoreHours": self.ignore_hours.isChecked()}
        if self.marker.text():
            p["marker"] = self.marker.text()
        if self.pre in ("merge", "mcam"):
            p["items"] = list(self.items)
            if self.name is not None and self.name.text():
                p["name"] = self.name.text()
        if self.pre == "merge" and self.remove_audio is not None:
            p["removeVideoAudio"] = self.remove_audio.isChecked()
        elif self.pre == "mcam":
            p["audio"] = next((k for k, b in self.audio.items() if b.isChecked()), "camera1")
            p["cameraNames"] = next((k for k, b in self.camera_names.items() if b.isChecked()), "clip")
            p["processedBin"] = bool(self.processed_bin and self.processed_bin.isChecked())
        elif self.track is not None and self.track.text():
            p["track"] = self.track.text()
        return p

    def apply(self) -> None:
        def done(result: Any, _b: bytes | None) -> None:
            r = result if isinstance(result, dict) else {}
            if self.pre == "sync":
                text = fmt("Synchronized {n} clip group(s)", n=r.get("moved", 0))
            elif self.pre == "merge":
                text = tr("Merged clip created")
            else:
                cams = r.get("cameras")
                text = fmt("Multi-camera source sequence with {n} camera(s)", n=len(cams) if isinstance(cams, list) else 0)
            self.done_message(text)
            self.accept()

        self.engine.execute(self.command, self.values(), on_reply=done, on_error=self.show_error)


class EditCamerasDialog(QDialog):
    """Edit Cameras (egui's ``show_edit_cameras``): a thumbnail, name and on/off switch per camera
    of the multi-camera clip at the playhead (``multicam.editCameras``)."""

    THUMB_W = 96

    def __init__(self, parent: QWidget, engine: EngineClient, info: dict[str, Any], done: Callable[[], None]) -> None:
        super().__init__(parent)
        self.setObjectName("editCameras")
        self.setWindowTitle(tr("Edit Cameras"))
        self.engine = engine
        self.done_callback = done
        self.sequence = whole(info.get("source"))
        source_time = whole(info.get("sourceTime"))
        lay = QVBoxLayout(self)
        note = QLabel(tr("Cameras shown in the Multi-Camera view (uncheck to hide)."), self)
        note.setEnabled(False)
        lay.addWidget(note)
        body = QWidget(self)
        rows = QVBoxLayout(body)
        self.enabled: dict[int, QCheckBox] = {}
        self.names: dict[int, QLineEdit] = {}
        self.thumbs: dict[int, QLabel] = {}
        for cam in info.get("cameras") or []:
            if not isinstance(cam, dict) or not isinstance(cam.get("angle"), int):
                continue
            a = int(cam["angle"])
            row = QHBoxLayout()
            on = QCheckBox("", body)
            on.setObjectName(f"editCameras.enabled.{a}")
            on.setChecked(cam.get("enabled") is not False)
            row.addWidget(on)
            thumb = QLabel(body)
            thumb.setObjectName(f"editCameras.thumb.{a}")
            thumb.setFixedSize(self.THUMB_W, self.THUMB_W * 9 // 16)
            thumb.setAlignment(Qt.AlignmentFlag.AlignCenter)
            thumb.setStyleSheet(f"background: {theme.current().tl_bg}; color: {theme.current().text_dim}; font-size: 10px;")
            if cam.get("video") is False:
                thumb.setText(tr("Audio"))
            row.addWidget(thumb)
            row.addWidget(QLabel(str(a + 1), body))
            name = QLineEdit(str(cam.get("name") or ""), body)
            name.setObjectName(f"editCameras.name.{a}")
            name.setMinimumWidth(180)
            row.addWidget(name, 1)
            rows.addLayout(row)
            self.enabled[a], self.names[a], self.thumbs[a] = on, name, thumb
            if cam.get("video") is not False:
                engine.call(
                    "multicam.angleFrame",
                    {"sequence": self.sequence, "angle": a, "time": source_time, "maxWidth": self.THUMB_W * 2},
                    on_reply=lambda r, b, lab=thumb: self._thumb(lab, r, b),
                )
        scroll = QScrollArea(self)
        scroll.setWidgetResizable(True)
        scroll.setWidget(body)
        scroll.setMaximumHeight(420)
        lay.addWidget(scroll)
        self.error = QLabel("", self)
        self.error.setStyleSheet("color: #e08a6a;")
        self.error.hide()
        lay.addWidget(self.error)
        row = QHBoxLayout()
        row.addStretch(1)
        cancel = QPushButton(tr("Cancel"), self)
        cancel.setObjectName("editCameras.cancel")
        cancel.clicked.connect(self.reject)
        row.addWidget(cancel)
        ok = QPushButton(tr("OK"), self)
        ok.setObjectName("editCameras.ok")
        ok.setDefault(True)
        ok.clicked.connect(self.apply)
        row.addWidget(ok)
        lay.addLayout(row)

    def _thumb(self, label: QLabel, result: Any, data: bytes | None) -> None:
        img = rgba_image(result, data)
        if img is not None:
            label.setPixmap(QPixmap.fromImage(img).scaled(label.size(), Qt.AspectRatioMode.KeepAspectRatio, Qt.TransformationMode.SmoothTransformation))

    def values(self) -> list[dict[str, Any]]:
        return [{"angle": a, "name": self.names[a].text(), "enabled": self.enabled[a].isChecked()} for a in sorted(self.names)]

    def apply(self) -> None:
        def failed(message: str) -> None:
            self.error.setText(message)
            self.error.show()

        def done(_r: Any, _b: bytes | None) -> None:
            self.done_callback()
            self.accept()

        self.engine.execute("multicam.editCameras", {"sequence": self.sequence, "cameras": self.values()}, on_reply=done, on_error=failed)
