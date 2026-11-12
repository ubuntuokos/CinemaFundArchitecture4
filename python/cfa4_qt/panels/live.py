"""Live Graphics window (credits plan C10): prepared cues, PREVIEW and PROGRAM monitors, and the
character-generator controls TAKE, NEXT, PREVIOUS, HOLD / CONTINUE, UPDATE and OUT; the live
page for OBS's Browser Source (127.0.0.1 only), the sound mode and the OBS connection.

Everything is the engine's (``live.*`` commands, ``crates/engine/src/live.rs``): this window only
asks. While the page runs or something is on air it keeps the program moving (``live.tick``,
one request at a time, so a slow frame never piles up), also when the window is closed.
"""

from __future__ import annotations

import base64
from pathlib import Path
from typing import Any

from PySide6.QtCore import Qt, QTimer
from PySide6.QtGui import QGuiApplication, QImage, QPixmap
from PySide6.QtWidgets import (
    QComboBox,
    QDialog,
    QFileDialog,
    QFormLayout,
    QGridLayout,
    QGroupBox,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QPushButton,
    QSpinBox,
    QVBoxLayout,
    QWidget,
)

from ..engine import EngineClient
from ..i18n import N_, tr
from .credits import SHEET_FILTER, format_seconds

AUDIO_MODES = (("embedded", N_("In the page (OBS takes it from the Browser Source)")), ("separate", N_("Separate (a media source in OBS)")), ("ignore", N_("No sound")))
# the program moves this often at most (ms); a frame that takes longer only slows it down
TICK_MS = 40
# the monitors are drawn this often (ms) and this wide (px)
MONITOR_MS = 250
MONITOR_WIDTH = 480


def cue_text(cue: dict[str, Any]) -> str:
    """A cue in the list: its name, pages and length."""
    name = str(cue.get("name") or "")
    pages = cue.get("pages") if isinstance(cue.get("pages"), int) else 0
    kind = tr("credits") if cue.get("kind") == "credits" else tr("graphic")
    text = f"{name} — {kind}, " + tr("{n} pages").replace("{n}", str(pages)) + f", {format_seconds(cue.get('duration'))}"
    if cue.get("audio"):
        text += " ♪"
    return text


def png_pixmap(result: Any) -> QPixmap | None:
    """The engine's picture (``pngBase64``) as a pixmap; None if there is none."""
    if not isinstance(result, dict):
        return None
    try:
        data = base64.b64decode(result.get("pngBase64") or "", validate=True)
    except (ValueError, TypeError):
        return None
    image = QImage.fromData(data, "PNG")
    return None if image.isNull() else QPixmap.fromImage(image)


class Monitor(QLabel):
    """A monitor: the picture on a checkerboard-free dark field, scaled to fit."""

    def __init__(self, name: str, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName(name)
        self.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.setMinimumSize(MONITOR_WIDTH // 2, MONITOR_WIDTH * 9 // 32)
        self.setStyleSheet("background: #111; color: #888;")
        self.pixmap_: QPixmap | None = None

    def show_pixmap(self, pm: QPixmap | None, empty_text: str) -> None:
        self.pixmap_ = pm
        if pm is None:
            self.setPixmap(QPixmap())
            self.setText(empty_text)
        else:
            self.setText("")
            self.setPixmap(pm.scaled(self.size(), Qt.AspectRatioMode.KeepAspectRatio, Qt.TransformationMode.SmoothTransformation))


class LiveDialog(QDialog):
    def __init__(self, engine: EngineClient, parent: QWidget | None = None, selected_clips: Any = None) -> None:
        super().__init__(parent)
        self.setObjectName("dialog.live")
        self.setWindowTitle(tr("Live Graphics"))
        self.setModal(False)
        self.engine = engine
        self.selected_clips = selected_clips  # callable → the timeline's selected clip ids
        self.state: dict[str, Any] = {}
        self._ticking = False
        self._monitor_busy = {"preview": False, "program": False}

        # ----- cues -----
        self.cues = QListWidget(self)
        self.cues.setObjectName("live.cues")
        self.cues.setToolTip(tr("Prepared cues; double-click one to put it in preview"))
        self.cues.itemDoubleClicked.connect(self._cue_double_clicked)
        add_credits = QPushButton(tr("Add Credits…"), self)
        add_credits.setObjectName("live.addCredits")
        add_credits.setToolTip(tr("A credits spreadsheet, credits file or Cinecred project as a cue"))
        add_credits.clicked.connect(self._browse_credits)
        add_clip = QPushButton(tr("Add Selected Graphic"), self)
        add_clip.setObjectName("live.addClip")
        add_clip.setToolTip(tr("The graphic clip selected on the timeline (a title, a lower third) as a cue"))
        add_clip.clicked.connect(self.add_selected_clip)
        self.remove_button = QPushButton(tr("Remove"), self)
        self.remove_button.setObjectName("live.remove")
        self.remove_button.clicked.connect(self._remove)
        cue_buttons = QGridLayout()
        cue_buttons.addWidget(add_credits, 0, 0)
        cue_buttons.addWidget(add_clip, 0, 1)
        cue_buttons.addWidget(self.remove_button, 1, 1)
        left = QVBoxLayout()
        left.addWidget(QLabel(tr("Cues"), self))
        left.addWidget(self.cues, 1)
        left.addLayout(cue_buttons)

        # ----- monitors -----
        self.preview_monitor = Monitor("live.previewMonitor", self)
        self.program_monitor = Monitor("live.programMonitor", self)
        self.preview_label = QLabel(self)
        self.preview_label.setObjectName("live.previewLabel")
        self.program_label = QLabel(self)
        self.program_label.setObjectName("live.programLabel")
        monitors = QGridLayout()
        monitors.addWidget(QLabel(f"<b>{tr('PREVIEW')}</b>", self), 0, 0)
        monitors.addWidget(QLabel(f"<b style='color:#d33'>{tr('PROGRAM')}</b>", self), 0, 1)
        monitors.addWidget(self.preview_monitor, 1, 0)
        monitors.addWidget(self.program_monitor, 1, 1)
        monitors.addWidget(self.preview_label, 2, 0)
        monitors.addWidget(self.program_label, 2, 1)
        monitors.setRowStretch(1, 1)

        # ----- controls -----
        self.buttons: dict[str, QPushButton] = {}
        controls = QHBoxLayout()
        for cid, label, tip in (
            ("live.take", N_("TAKE"), N_("The preview cue goes on air from its first page")),
            ("live.previous", N_("PREVIOUS"), N_("The page before (on the first page: from its start)")),
            ("live.next", N_("NEXT"), N_("The next page; after the last one, the preview cue")),
            ("live.hold", N_("HOLD"), N_("Stop the program where it is")),
            ("live.continue", N_("CONTINUE"), N_("Go on from where it stopped")),
            ("live.update", N_("UPDATE"), N_("Read the spreadsheet (or the clip) again; on air it changes at once")),
            ("live.out", N_("OUT"), N_("Take everything off air")),
        ):
            b = QPushButton(tr(label), self)
            b.setObjectName(f"live.button.{cid.removeprefix('live.')}")
            b.setToolTip(tr(tip))
            b.setMinimumHeight(40)
            b.clicked.connect(lambda _c=False, c=cid: self.run(c))
            controls.addWidget(b)
            self.buttons[cid] = b
        self.buttons["live.take"].setStyleSheet("font-weight: bold;")
        self.buttons["live.out"].setStyleSheet("font-weight: bold; color: #d33;")

        # ----- output: page, sound, OBS -----
        self.page_button = QPushButton(tr("Start Live Page"), self)
        self.page_button.setObjectName("live.page")
        self.page_button.setCheckable(True)
        self.page_button.setToolTip(tr("Serve the program on this computer for OBS's Browser Source (127.0.0.1 only)"))
        self.page_button.toggled.connect(self._page_toggled)
        self.url = QLineEdit(self)
        self.url.setObjectName("live.url")
        self.url.setReadOnly(True)
        self.url.setPlaceholderText(tr("not running"))
        copy = QPushButton(tr("Copy"), self)
        copy.setObjectName("live.copyUrl")
        copy.clicked.connect(lambda: QGuiApplication.clipboard().setText(self.url.text()))
        page_row = QHBoxLayout()
        page_row.addWidget(self.page_button)
        page_row.addWidget(self.url, 1)
        page_row.addWidget(copy)
        self.audio = QComboBox(self)
        self.audio.setObjectName("live.audio")
        for mode, label in AUDIO_MODES:
            self.audio.addItem(tr(label), mode)
        self.audio.setCurrentIndex(2)
        self.audio.currentIndexChanged.connect(lambda _i: self.run("live.audio", {"mode": self.audio.currentData()}))
        self.obs_port = QSpinBox(self)
        self.obs_port.setObjectName("live.obs.port")
        self.obs_port.setRange(1, 65535)
        self.obs_port.setValue(4455)
        self.obs_password = QLineEdit(self)
        self.obs_password.setObjectName("live.obs.password")
        self.obs_password.setEchoMode(QLineEdit.EchoMode.Password)
        self.obs_password.setPlaceholderText(tr("OBS ▸ Tools ▸ WebSocket Server Settings"))
        self.obs_button = QPushButton(tr("Connect to OBS"), self)
        self.obs_button.setObjectName("live.obs.connect")
        self.obs_button.setToolTip(tr("Connect to OBS on this computer and point its “CFA4 Live” Browser Source at the live page"))
        self.obs_button.clicked.connect(self.connect_obs)
        self.obs_status = QLabel(self)
        self.obs_status.setObjectName("live.obs.status")
        self.obs_status.setWordWrap(True)
        obs_row = QHBoxLayout()
        obs_row.addWidget(QLabel(tr("Port"), self))
        obs_row.addWidget(self.obs_port)
        obs_row.addWidget(QLabel(tr("Password"), self))
        obs_row.addWidget(self.obs_password, 1)
        obs_row.addWidget(self.obs_button)
        output = QGroupBox(tr("Output"), self)
        of = QFormLayout(output)
        of.addRow(tr("Live page"), page_row)
        of.addRow(tr("Sound"), self.audio)
        of.addRow(tr("OBS"), obs_row)
        of.addRow("", self.obs_status)

        self.status = QLabel(self)
        self.status.setObjectName("live.status")
        self.status.setWordWrap(True)

        top = QHBoxLayout()
        left_w = QWidget(self)
        left_w.setLayout(left)
        left_w.setMaximumWidth(420)
        top.addWidget(left_w)
        right = QVBoxLayout()
        right.addLayout(monitors, 1)
        right.addLayout(controls)
        top.addLayout(right, 1)
        layout = QVBoxLayout(self)
        layout.addLayout(top, 1)
        layout.addWidget(output)
        layout.addWidget(self.status)
        self.resize(1200, 640)

        self.tick_timer = QTimer(self)
        self.tick_timer.setInterval(TICK_MS)
        self.tick_timer.timeout.connect(self._tick)
        self.monitor_timer = QTimer(self)
        self.monitor_timer.setInterval(MONITOR_MS)
        self.monitor_timer.timeout.connect(self.refresh_monitors)
        self._apply_state({})

    # ----- engine round trips -----
    def run(self, cid: str, params: dict[str, Any] | None = None) -> None:
        """Run a live command; its reply is the live status."""

        def done(result: Any, _b: bytes | None) -> None:
            if isinstance(result, dict) and "cues" in result:
                self._apply_state(result)
                self.status.setText("")
            else:
                self.refresh()
            self.refresh_monitors()

        self.engine.execute(cid, params or {}, on_reply=done, on_error=self._error)

    def _error(self, message: str) -> None:
        self.status.setText(message)

    def refresh(self) -> None:
        self.engine.execute("live.status", {}, on_reply=lambda r, _b: self._apply_state(r if isinstance(r, dict) else {}), on_error=self._error)

    def _apply_state(self, state: dict[str, Any]) -> None:
        self.state = state
        cues = [c for c in state.get("cues") or [] if isinstance(c, dict)]
        preview, program = state.get("preview"), state.get("program") if isinstance(state.get("program"), dict) else None
        current = self.cues.currentItem().data(Qt.ItemDataRole.UserRole) if self.cues.currentItem() else None
        self.cues.blockSignals(True)
        try:
            self.cues.clear()
            for c in cues:
                item = QListWidgetItem(cue_text(c))
                item.setData(Qt.ItemDataRole.UserRole, c.get("id"))
                if program and c.get("id") == program.get("cue"):
                    item.setText("● " + item.text())
                    item.setForeground(Qt.GlobalColor.red)
                elif c.get("id") == preview:
                    item.setText("▶ " + item.text())
                self.cues.addItem(item)
                if c.get("id") == current:
                    self.cues.setCurrentItem(item)
        finally:
            self.cues.blockSignals(False)
        names = {c.get("id"): str(c.get("name") or "") for c in cues}
        self.preview_label.setText(names.get(preview, "") if preview is not None else tr("Nothing in preview"))
        if program:
            page = (program.get("page") or 0) + 1
            text = f"{program.get('name') or ''} — " + tr("page {page} of {pages}").replace("{page}", str(page)).replace("{pages}", str(program.get("pages") or 0))
            if program.get("held"):
                text += " — " + tr("held")
            if program.get("finished"):
                text += " — " + tr("ended")
            self.program_label.setText(text)
        else:
            self.program_label.setText(tr("Off air"))
        on_air = program is not None
        held = bool(program and program.get("held"))
        self.buttons["live.take"].setEnabled(preview is not None)
        for cid in ("live.next", "live.previous", "live.out"):
            self.buttons[cid].setEnabled(on_air)
        self.buttons["live.hold"].setEnabled(on_air and not held)
        self.buttons["live.continue"].setEnabled(held)
        self.buttons["live.update"].setEnabled(bool(cues))
        self.remove_button.setEnabled(bool(cues))
        server = state.get("server") if isinstance(state.get("server"), dict) else None
        self.url.setText(str(server.get("url")) if server else "")
        self.page_button.blockSignals(True)
        self.page_button.setChecked(server is not None)
        self.page_button.setText(tr("Stop Live Page") if server else tr("Start Live Page"))
        self.page_button.blockSignals(False)
        mode = state.get("audioMode")
        idx = self.audio.findData(mode)
        if idx >= 0 and idx != self.audio.currentIndex():
            self.audio.blockSignals(True)
            self.audio.setCurrentIndex(idx)
            self.audio.blockSignals(False)
        obs = state.get("obs") if isinstance(state.get("obs"), dict) else None
        text = tr("Connected to OBS {version}; source “{source}”").replace("{version}", str(obs.get("version") or "")).replace("{source}", str(obs.get("source") or "")) if obs else ""
        last = obs.get("last") if obs and isinstance(obs.get("last"), dict) else {}
        # what OBS could not set up, in the engine's words (no Browser Source plugin, say)
        problems = [str(last[k]) for k in ("browserError", "audioError") if last.get(k)]
        self.obs_status.setText("\n".join([text, *problems]) if text else "")
        self.obs_button.setText(tr("Point OBS Again") if obs else tr("Connect to OBS"))
        # the program keeps moving while it is on air or the page is served
        if on_air or server:
            if not self.tick_timer.isActive():
                self.tick_timer.start()
        else:
            self.tick_timer.stop()

    def _tick(self) -> None:
        if self._ticking:
            return
        self._ticking = True

        def done(result: Any, _b: bytes | None) -> None:
            self._ticking = False
            program = self.state.get("program") if isinstance(self.state.get("program"), dict) else None
            if isinstance(result, dict) and program is not None and (result.get("page") != program.get("page") or result.get("finished") != program.get("finished")):
                self.refresh()

        def failed(message: str) -> None:
            self._ticking = False
            self.status.setText(message)
            self.tick_timer.stop()

        self.engine.execute("live.tick", {}, on_reply=done, on_error=failed)

    def refresh_monitors(self) -> None:
        if not self.isVisible():
            return
        for which, monitor, empty in (("preview", self.preview_monitor, tr("Nothing in preview")), ("program", self.program_monitor, tr("Off air"))):
            if self._monitor_busy[which]:
                continue
            self._monitor_busy[which] = True

            def done(result: Any, _b: bytes | None, w: str = which, m: Monitor = monitor, e: str = empty) -> None:
                self._monitor_busy[w] = False
                pm = None if (isinstance(result, dict) and result.get("empty")) else png_pixmap(result)
                m.show_pixmap(pm, e)

            def failed(_m: str, w: str = which) -> None:
                self._monitor_busy[w] = False

            self.engine.execute("live.frame", {"which": which, "width": MONITOR_WIDTH}, on_reply=done, on_error=failed)

    # ----- cues -----
    def add_credits(self, path: str) -> None:
        self.run_add({"path": path})

    def run_add(self, params: dict[str, Any]) -> None:
        def done(_r: Any, _b: bytes | None) -> None:
            self.refresh()
            self.refresh_monitors()

        self.engine.execute("live.cue.add", params, on_reply=done, on_error=self._error)

    def _browse_credits(self) -> None:
        path, _ = QFileDialog.getOpenFileName(self, tr("Add Credits…"), str(Path.home()), tr(SHEET_FILTER))
        if path:
            self.add_credits(path)

    def add_selected_clip(self) -> None:
        clips = list(self.selected_clips() or []) if callable(self.selected_clips) else []
        if not clips:
            self.status.setText(tr("Select a graphic clip on the timeline first."))
            return
        for c in clips:
            self.run_add({"clip": c})

    def _cue_double_clicked(self, item: QListWidgetItem) -> None:
        cid = item.data(Qt.ItemDataRole.UserRole)
        if isinstance(cid, int):
            self.run("live.preview", {"id": cid})

    def _remove(self) -> None:
        item = self.cues.currentItem()
        cid = item.data(Qt.ItemDataRole.UserRole) if item else None
        if isinstance(cid, int):
            self.engine.execute("live.cue.remove", {"id": cid}, on_reply=lambda _r, _b: self.refresh(), on_error=self._error)

    # ----- output -----
    def _page_toggled(self, on: bool) -> None:
        self.run("live.server.start" if on else "live.server.stop")

    def connect_obs(self) -> None:
        self.status.setText(tr("Connecting to OBS…"))
        if (self.state.get("obs") or None) is not None:
            self.run("live.obs.setup")
            return
        params: dict[str, Any] = {"port": self.obs_port.value()}
        if self.obs_password.text():
            params["password"] = self.obs_password.text()
        self.run("live.obs.connect", params)

    # ----- window -----
    def open_window(self) -> None:
        self.show()
        self.raise_()
        self.activateWindow()
        self.refresh()
        self.monitor_timer.start()
        self.refresh_monitors()

    def hideEvent(self, e: Any) -> None:
        # the monitors stop; the program keeps moving (tick) while it is on air or served
        self.monitor_timer.stop()
        super().hideEvent(e)
