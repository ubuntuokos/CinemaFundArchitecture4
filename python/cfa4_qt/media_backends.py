"""Settings ▸ Media Backends: which media backends this CFA4 uses, and the restricted codecs.

CFA4's own Rust engine always goes first. The supplementary backends (FFmpeg, GStreamer; ADR 0005)
take over only for files the engine cannot open, in the order shown. The panel reads
``media.backends`` and switches a backend with ``prefs.set media.<id>Backend`` (new files only).

The restricted-codecs section follows the Ubuntu model (``docs/licensing.md`` §7): patent-restricted
components such as x264 and x265 are never part of the base package; the panel shows whether they
are installed and how to install them, and leaves the decision to the user.
"""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from PySide6.QtCore import Qt
from PySide6.QtGui import QGuiApplication
from PySide6.QtWidgets import (
    QCheckBox,
    QDialog,
    QDialogButtonBox,
    QFormLayout,
    QGroupBox,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QScrollArea,
    QVBoxLayout,
    QWidget,
)

from .i18n import N_, fmt, tr

Execute = Callable[..., Any]

# English texts the engine reports, translated here (the engine stays English)
ROLES = {
    "builtin": N_("Own Rust decoders and encoders: always used first, and the fallback when a supplementary backend fails"),
    "ffmpeg": N_("Supplementary decoder for formats the own engine does not read yet (10-bit and 4:2:2 camera media, AVI, DV, WMV, MPEG-4 Part 2…)"),
    "gstreamer": N_("Supplementary decoder from the system's GStreamer (Ogg Theora, FLV, WMV and other formats); LGPL, from Ubuntu's repositories"),
}
PURPOSES = {
    "libx264": N_("H.264 software encoding (x264)"),
    "libx265": N_("HEVC software encoding (x265)"),
}
NAMES = {"builtin": N_("CFA4 engine")}


def install_command(package: str) -> str:
    return f"sudo apt install {package}"


class MediaBackendsDialog(QDialog):
    """Non-modal: opened from Edit ▸ Preferences ▸ Media Backends… and Help ▸ Restricted Codecs…"""

    def __init__(self, execute: Execute, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("dialog.mediaBackends")
        self.setWindowTitle(tr("Media Backends"))
        self.resize(640, 600)
        self._execute = execute
        self.backends: list[dict[str, Any]] = []
        self.switches: dict[str, QCheckBox] = {}
        self.restricted_box: QGroupBox | None = None

        outer = QVBoxLayout(self)
        intro = QLabel(
            tr(
                "CFA4's own engine opens every file it can. The supplementary backends below take over, in this order, only for files the engine cannot open yet. Switching a backend off affects files opened afterwards."
            ),
            self,
        )
        intro.setWordWrap(True)
        intro.setObjectName("mediaBackends.intro")
        outer.addWidget(intro)

        self.error = QLabel("", self)
        self.error.setObjectName("mediaBackends.error")
        self.error.setWordWrap(True)
        self.error.setStyleSheet("color: #e57373;")
        self.error.hide()
        outer.addWidget(self.error)

        self._want_restricted = False
        scroll = QScrollArea(self)
        self.scroll = scroll
        scroll.setWidgetResizable(True)
        self.body = QWidget(scroll)
        self.body_layout = QVBoxLayout(self.body)
        self.body_layout.addStretch(1)
        scroll.setWidget(self.body)
        outer.addWidget(scroll, 1)

        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Close, self)
        self.refresh_button = QPushButton(tr("Refresh"), self)
        self.refresh_button.setObjectName("mediaBackends.refresh")
        buttons.addButton(self.refresh_button, QDialogButtonBox.ButtonRole.ActionRole)
        self.refresh_button.clicked.connect(self.refresh)
        buttons.rejected.connect(self.close)
        outer.addWidget(buttons)

    # ----- engine -----
    def refresh(self) -> None:
        self._execute("media.backends", {}, on_reply=lambda r, _b: self.set_backends(r), on_error=self._show_error)

    def _show_error(self, message: str) -> None:
        self.error.setText(fmt("The engine did not answer: {message}", message=message))
        self.error.show()

    def set_enabled(self, backend_id: str, enabled: bool) -> None:
        pref = next((b.get("preference") for b in self.backends if b.get("id") == backend_id), None)
        if not pref:
            return
        self._execute(
            "prefs.set",
            {"key": pref, "value": bool(enabled)},
            on_reply=lambda _r, _b: self.refresh(),
            on_error=self._show_error,
        )

    # ----- view -----
    def set_backends(self, result: Any) -> None:
        self.error.hide()
        backends = result.get("backends") if isinstance(result, dict) else None
        self.backends = [b for b in backends or [] if isinstance(b, dict)]
        while self.body_layout.count() > 1:
            item = self.body_layout.takeAt(0)
            widget = item.widget() if item is not None else None
            if widget is not None:
                widget.deleteLater()
        self.switches.clear()
        self.restricted_box = None
        for i, b in enumerate(self.backends):
            self.body_layout.insertWidget(i, self._backend_box(i, b))
        restricted = [(b, r) for b in self.backends for r in b.get("restricted") or [] if isinstance(r, dict)]
        if restricted:
            self.restricted_box = self._restricted_box(restricted)
            self.body_layout.insertWidget(len(self.backends), self.restricted_box)
            if self._want_restricted:
                self._want_restricted = False
                self.scroll.ensureWidgetVisible(self.restricted_box)

    def _backend_box(self, index: int, b: dict[str, Any]) -> QGroupBox:
        bid = str(b.get("id") or "")
        name = tr(NAMES.get(bid, str(b.get("name") or bid)))
        box = QGroupBox(f"{index + 1}. {name}", self.body)
        box.setObjectName(f"mediaBackends.{bid}")
        form = QFormLayout(box)
        role = QLabel(tr(ROLES.get(bid, str(b.get("role") or ""))), box)
        role.setWordWrap(True)
        form.addRow(role)
        form.addRow(tr("Status"), self._status_label(box, bid, b))
        for key, label in (("version", N_("Version")), ("license", N_("Licence")), ("location", N_("Location"))):
            if b.get(key):
                value = QLabel(str(b[key]), box)
                value.setObjectName(f"mediaBackends.{bid}.{key}")
                value.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
                value.setWordWrap(True)
                form.addRow(tr(label), value)
        if bid == "builtin":
            hw = tr("on") if b.get("hardwareDecoding") else tr("off")
            form.addRow(tr("Hardware decoding"), QLabel(hw, box))
        caps = [str(c) for c in b.get("capabilities") or []]
        if caps:
            text = QLabel(", ".join(caps), box)
            text.setObjectName(f"mediaBackends.{bid}.capabilities")
            text.setWordWrap(True)
            form.addRow(tr("Capabilities"), text)
        if b.get("canDisable"):
            switch = QCheckBox(tr("Use this backend"), box)
            switch.setObjectName(f"mediaBackends.{bid}.enabled")
            switch.setChecked(bool(b.get("enabled")))
            switch.setEnabled(bool(b.get("compiledIn")))
            switch.toggled.connect(lambda on, i=bid: self.set_enabled(i, on))
            self.switches[bid] = switch
            form.addRow(switch)
        return box

    def _status_label(self, parent: QWidget, bid: str, b: dict[str, Any]) -> QLabel:
        if not b.get("compiledIn"):
            text = tr("Not part of this build")
        elif b.get("available") and b.get("enabled"):
            text = tr("In use")
        elif b.get("available"):
            text = tr("Available, switched off")
        else:
            text = tr("Not available")
        problem = str(b.get("problem") or "")
        label = QLabel(text + (f"\n{problem}" if problem and not b.get("available") else ""), parent)
        label.setObjectName(f"mediaBackends.{bid}.status")
        label.setWordWrap(True)
        return label

    def _restricted_box(self, items: list[tuple[dict[str, Any], dict[str, Any]]]) -> QGroupBox:
        box = QGroupBox(tr("Restricted codecs"), self.body)
        box.setObjectName("mediaBackends.restricted")
        layout = QVBoxLayout(box)
        note = QLabel(
            tr(
                "Some codecs are free software but implement patented formats. Like Ubuntu, CFA4 does not install them by default: the base package works without them, and you decide whether to install them. Their use may be restricted in some countries; check that it is permitted where you are."
            ),
            box,
        )
        note.setWordWrap(True)
        layout.addWidget(note)
        form = QFormLayout()
        packages: list[str] = []
        for backend, r in items:
            name = str(r.get("name") or "")
            mark = "✓ " + tr("installed") if r.get("available") else "✗ " + tr("not installed")
            value = QLabel(f"{tr(PURPOSES.get(name, str(r.get('purpose') or name)))} — {mark}", box)
            value.setObjectName(f"mediaBackends.restricted.{name}")
            form.addRow(name, value)
            pkg = backend.get("restrictedPackage")
            if pkg and pkg not in packages:
                packages.append(str(pkg))
        layout.addLayout(form)
        missing = any(not r.get("available") for _b, r in items)
        if packages and missing:
            command = install_command(" ".join(packages))
            row = QHBoxLayout()
            how = QLabel(fmt("To install them: {command}", command=command), box)
            how.setObjectName("mediaBackends.restricted.install")
            how.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
            how.setWordWrap(True)
            row.addWidget(how, 1)
            copy = QPushButton(tr("Copy command"), box)
            copy.setObjectName("mediaBackends.restricted.copy")
            copy.clicked.connect(lambda _c=False, t=command: QGuiApplication.clipboard().setText(t))
            row.addWidget(copy)
            layout.addLayout(row)
            later = QLabel(tr("After installing, restart CFA4."), box)
            later.setWordWrap(True)
            layout.addWidget(later)
        return box

    def show_restricted(self) -> None:
        """Help ▸ Restricted Codecs…: the same panel, scrolled to the restricted codecs."""
        self._want_restricted = True
        self.show()
        self.raise_()
        self.refresh()
