"""Live studio preview (CFA4-LIVE-STUDIO-001 S0): the program picture of the live studio host.

The engine runs the live studio (libobs, or CFA4's software backend) in its own supervised
process, ``cfa4-studio-host`` (``live.studio.host.*``, apps/cfa4-cli/src/serve_live.rs). This
window starts it, shows its newest program frame (``live.studio.frame``, RGBA8, through the same
shared-memory frame rings as ``frame.render``) and its status: backend, version, crashes and
restarts. A crash of the host is a status line here, never an error of the GUI or the engine.

S0 is self-contained on purpose: a separate window that touches no shared Qt6 file, so it does not
collide with the Qt6 work. In S1 it becomes part of the Élő (Live) workspace (plan D5, D7).

Run it on its own: ``python -m cfa4_qt.panels.live_studio [--engine PATH]``.
"""

from __future__ import annotations

import argparse
import sys
from typing import Any

from PySide6.QtCore import Qt, QTimer
from PySide6.QtGui import QImage, QPixmap
from PySide6.QtWidgets import QComboBox, QHBoxLayout, QLabel, QPushButton, QSizePolicy, QVBoxLayout, QWidget

from ..engine import EngineClient
from ..i18n import fmt, tr

RING = "live-studio"  # frame.render-style shm ring name (serve_shm.rs: 1–16 letters, digits, -)
FRAME_MS = 33  # ask for a frame about 30 times a second (one request in flight at a time)
STATUS_MS = 500
BACKENDS = ("auto", "libobs", "software")
MAX_SIDE = 8192


def rgba_image(meta: Any, data: bytes | memoryview | None) -> QImage | None:
    """The frame of a ``live.studio.frame`` reply as a QImage (a copy), or None when the reply
    does not describe a whole RGBA8 frame (a hostile or torn reply is dropped, never shown)."""
    if not isinstance(meta, dict) or data is None or meta.get("format") != "rgba8":
        return None
    w, h = meta.get("w"), meta.get("h")
    if not (isinstance(w, int) and isinstance(h, int)):
        return None
    if not all(not isinstance(v, bool) and 0 < v <= MAX_SIDE for v in (w, h)):
        return None
    if len(data) != w * h * 4:
        return None
    return QImage(bytes(data), w, h, w * 4, QImage.Format.Format_RGBA8888).copy()


def status_text(status: Any) -> str:
    """One line for the host status (``live.studio.host.status``)."""
    if not isinstance(status, dict):
        return tr("Live studio: stopped")
    state = status.get("state")
    if state in (None, "stopped"):
        return tr("Live studio: stopped")
    backend = status.get("backend") or "?"
    version = status.get("version") or ""
    parts = [fmt("Live studio: {state}", state=str(state)), f"{backend} {version}".strip()]
    crashes = status.get("crashes")
    if isinstance(crashes, int) and crashes > 0:
        parts.append(
            fmt(
                "restarted after {count} crashes (last: {exit})", count=crashes, exit=str(status.get("lastExit") or "?")
            )
        )
    if status.get("fallbackReason"):
        parts.append(fmt("software backend: {reason}", reason=str(status["fallbackReason"])))
    if state == "failed" and status.get("error"):
        parts.append(str(status["error"]))
    return " · ".join(parts)


class LiveStudioPreview(QWidget):
    """Start/stop the live studio host and show its program output."""

    def __init__(self, engine: EngineClient, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.engine = engine
        self.setObjectName("live_studio")
        self.setWindowTitle(tr("Live studio preview"))
        self._waiting = False
        self._last_seq = -1
        self._frame: QImage | None = None

        self.backend = QComboBox(self)
        self.backend.setObjectName("live_studio.backend")
        for b in BACKENDS:
            self.backend.addItem(b, b)
        self.start_button = QPushButton(tr("Start studio"), self)
        self.start_button.setObjectName("live_studio.start")
        self.stop_button = QPushButton(tr("Stop studio"), self)
        self.stop_button.setObjectName("live_studio.stop")
        self.picture = QLabel(self)
        self.picture.setObjectName("live_studio.picture")
        self.picture.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.picture.setMinimumSize(320, 180)
        self.picture.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Expanding)
        self.picture.setStyleSheet("background: black;")
        self.status = QLabel(tr("Live studio: stopped"), self)
        self.status.setObjectName("live_studio.status")
        self.status.setWordWrap(True)

        row = QHBoxLayout()
        row.addWidget(self.backend)
        row.addWidget(self.start_button)
        row.addWidget(self.stop_button)
        row.addStretch(1)
        layout = QVBoxLayout(self)
        layout.addLayout(row)
        layout.addWidget(self.picture, 1)
        layout.addWidget(self.status)

        self.start_button.clicked.connect(self.start_host)
        self.stop_button.clicked.connect(self.stop_host)
        self.frame_timer = QTimer(self)
        self.frame_timer.setInterval(FRAME_MS)
        self.frame_timer.timeout.connect(self.request_frame)
        self.status_timer = QTimer(self)
        self.status_timer.setInterval(STATUS_MS)
        self.status_timer.timeout.connect(self.request_status)

    # ----- host -----
    def start_host(self) -> None:
        params = {"backend": self.backend.currentData() or "auto"}
        self.engine.call("live.studio.host.start", params, on_reply=self._started, on_error=self._failed)

    def stop_host(self) -> None:
        self.frame_timer.stop()
        self.engine.call("live.studio.host.stop", {}, on_reply=lambda r, _b: self.show_status(r), on_error=self._failed)

    def _started(self, result: Any, _data: bytes | None) -> None:
        self.show_status(result)
        self._last_seq = -1
        self.frame_timer.start()
        self.status_timer.start()

    def _failed(self, message: str) -> None:
        self._waiting = False
        self.status.setText(fmt("Live studio: {error}", error=message))

    def request_status(self) -> None:
        if self.engine.running:
            self.engine.call("live.studio.host.status", {}, on_reply=lambda r, _b: self.show_status(r))

    def show_status(self, status: Any) -> None:
        self.status.setText(status_text(status))
        if isinstance(status, dict) and status.get("state") in ("stopped", "failed"):
            self.frame_timer.stop()

    # ----- frames -----
    def request_frame(self) -> None:
        if self._waiting or not self.engine.running:
            return
        self._waiting = True
        params = {"shm": RING} if self.engine.frames.enabled else {}
        self.engine.call("live.studio.frame", params, on_reply=self._got_frame, on_error=self._no_frame)

    def _no_frame(self, _message: str) -> None:
        # no frame yet (host starting or restarting): the status line says why
        self._waiting = False

    def _got_frame(self, result: Any, data: bytes | None) -> None:
        self._waiting = False
        if not isinstance(result, dict) or result.get("seq") == self._last_seq:
            return
        pixels: bytes | memoryview | None = data
        if "shm" in result:
            pixels = self.engine.frames.view(result["shm"])
        image = rgba_image(result, pixels)
        if image is None:
            return
        self._last_seq = result.get("seq", -1)
        self.show_frame(image)

    def show_frame(self, image: QImage) -> None:
        self._frame = image
        self._scale()

    def resizeEvent(self, event: Any) -> None:
        super().resizeEvent(event)
        self._scale()

    def _scale(self) -> None:
        if self._frame is None:
            return
        pix = QPixmap.fromImage(self._frame).scaled(
            self.picture.size(), Qt.AspectRatioMode.KeepAspectRatio, Qt.TransformationMode.SmoothTransformation
        )
        self.picture.setPixmap(pix)

    def closeEvent(self, event: Any) -> None:
        self.frame_timer.stop()
        self.status_timer.stop()
        if self.engine.running:
            self.engine.call("live.studio.host.stop", {})
        super().closeEvent(event)


def main(argv: list[str] | None = None) -> int:
    from PySide6.QtWidgets import QApplication

    from ..engine import EngineError, locate_engine

    parser = argparse.ArgumentParser(
        prog="python -m cfa4_qt.panels.live_studio", description="CFA4 live studio preview (S0)"
    )
    parser.add_argument("--engine", help="path of cfa4-cli (default: the GUI's own search)")
    parser.add_argument("--backend", choices=BACKENDS, default="auto")
    parser.add_argument("--start", action="store_true", help="start the host at once")
    args = parser.parse_args(argv)
    app = QApplication.instance() or QApplication(sys.argv[:1])
    rejected: list[str] = []
    path = locate_engine(args.engine, rejected)
    if path is None:
        print("no CFA4 engine (cfa4-cli with `serve`) found:\n  " + "\n  ".join(rejected), file=sys.stderr)
        return 2
    engine = EngineClient(path)
    try:
        engine.start()
    except EngineError as exc:
        print(exc, file=sys.stderr)
        return 2
    window = LiveStudioPreview(engine)
    window.backend.setCurrentIndex(BACKENDS.index(args.backend))
    window.resize(960, 600)
    window.show()
    if args.start:
        window.start_host()
    code = app.exec()
    engine.stop()
    return int(code)


if __name__ == "__main__":
    raise SystemExit(main())
