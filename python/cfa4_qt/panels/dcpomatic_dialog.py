"""File ▸ DCP-o-matic…: make a DCP, and KDMs for an encrypted one, with DCP-o-matic (ADR 0011).

DCP-o-matic runs in the engine as separate programs (``cfa4-dcpomatic``); this dialog only drives
the engine's commands:

- ``dcpomatic.status`` (shown at the top: installed or not, version);
- ``dcpomatic.makeDcp`` (DCP tab): content files, a new film folder, name, content kind, standard,
  resolution, container, frame rate, encryption; it runs as a background job (Progress panel);
- ``dcpomatic.makeKdm`` (KDM tab): the film folder of an encrypted DCP, a projector certificate,
  the output folder, cinema and screen names, validity (UTC) and formulation.

Automation ids: ``dcpomatic.status``, ``dcpomatic.tabs``; DCP tab ``dcpomatic.dcp.content``,
``dcpomatic.dcp.add``, ``dcpomatic.dcp.remove``, ``dcpomatic.dcp.output``,
``dcpomatic.dcp.outputBrowse``, ``dcpomatic.dcp.name``, ``dcpomatic.dcp.kind``,
``dcpomatic.dcp.standard``, ``dcpomatic.dcp.resolution``, ``dcpomatic.dcp.container``,
``dcpomatic.dcp.fps``, ``dcpomatic.dcp.encrypt``, ``dcpomatic.dcp.make``; KDM tab
``dcpomatic.kdm.film``, ``dcpomatic.kdm.filmBrowse``, ``dcpomatic.kdm.certificate``,
``dcpomatic.kdm.certificateBrowse``, ``dcpomatic.kdm.output``, ``dcpomatic.kdm.outputBrowse``,
``dcpomatic.kdm.cinema``, ``dcpomatic.kdm.screen``, ``dcpomatic.kdm.validFrom``,
``dcpomatic.kdm.validCount``, ``dcpomatic.kdm.validUnit``, ``dcpomatic.kdm.formulation``,
``dcpomatic.kdm.make``; ``dcpomatic.result``, ``dcpomatic.close``.
"""

from __future__ import annotations

import os
from typing import Any

from PySide6.QtWidgets import (
    QCheckBox,
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QFileDialog,
    QFormLayout,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QPlainTextEdit,
    QPushButton,
    QSpinBox,
    QTabWidget,
    QVBoxLayout,
    QWidget,
)

from ..i18n import N_, fmt, tr

# (engine value, label) for each combo
CONTENT_KINDS = (
    ("test", N_("Test")),
    ("feature", N_("Feature")),
    ("short", N_("Short")),
    ("trailer", N_("Trailer")),
    ("teaser", N_("Teaser")),
    ("advertisement", N_("Advertisement")),
    ("transitional", N_("Transitional")),
    ("rating", N_("Rating")),
    ("policy", N_("Policy")),
    ("psa", N_("Public Service Announcement")),
)
STANDARDS = (("smpte", "SMPTE"), ("interop", "Interop"))
RESOLUTIONS = ((None, N_("From the content")), ("2k", "2K"), ("4k", "4K"))
CONTAINERS = ((None, N_("From the content")), ("flat", N_("Flat (1.85)")), ("scope", N_("Scope (2.39)")), ("full", N_("Full (1.90)")))
FRAME_RATES = (None, 24, 25, 30, 48, 50, 60)
UNITS = (("hours", N_("hours")), ("days", N_("days")), ("weeks", N_("weeks")), ("years", N_("years")))
FORMULATIONS = (
    ("modifiedTransitional1", "Modified Transitional 1"),
    ("multipleModifiedTransitional1", "Multiple Modified Transitional 1"),
    ("dciAny", "DCI Any"),
    ("dciSpecific", "DCI Specific"),
)


def describe_status(r: Any) -> str:
    """The ``dcpomatic.status`` reply as one line."""
    if not isinstance(r, dict):
        return ""
    if r.get("available"):
        text = fmt("DCP-o-matic {version} is installed.", version=r.get("version") or "?")
        if not r.get("kdm"):
            text += " " + tr("KDMs need dcpomatic2_kdm_cli, which was not found.")
        return text
    return fmt(
        "DCP-o-matic is not available: {error} (DCP-o-matic {version} or newer is needed).",
        error=r.get("error") or "?",
        version=r.get("minimumVersion") or "2.18.20",
    )


def dcp_params(content: list[str], output: str, name: str, kind: str, standard: str, resolution: str | None, container: str | None, fps: int | None, encrypt: bool) -> dict[str, Any]:
    """The ``dcpomatic.makeDcp`` parameters; unset choices are left to DCP-o-matic."""
    p: dict[str, Any] = {"content": [os.path.expanduser(c) for c in content], "output": os.path.expanduser(output), "contentKind": kind, "standard": standard, "encrypt": encrypt}
    if name.strip():
        p["name"] = name.strip()
    if resolution:
        p["resolution"] = resolution
    if container:
        p["container"] = container
    if fps:
        p["fps"] = fps
    return p


def kdm_params(film: str, certificate: str, output: str, cinema: str, screen: str, valid_from: str, count: int, unit: str, formulation: str) -> dict[str, Any]:
    """The ``dcpomatic.makeKdm`` parameters."""
    p: dict[str, Any] = {
        "film": os.path.expanduser(film),
        "certificate": os.path.expanduser(certificate),
        "output": os.path.expanduser(output),
        "validFrom": valid_from.strip() or "now",
        "validFor": {"count": count, "unit": unit},
        "formulation": formulation,
    }
    if cinema.strip():
        p["cinema"] = cinema.strip()
    if screen.strip():
        p["screen"] = screen.strip()
    return p


def _combo(parent: QWidget, name: str, items: Any) -> QComboBox:
    c = QComboBox(parent)
    c.setObjectName(name)
    for value, label in items:
        c.addItem(tr(label), value)
    return c


class DcpomaticDialog(QDialog):
    """Make a DCP or KDMs with DCP-o-matic through the engine."""

    def __init__(self, engine: Any, parent: QWidget | None = None):
        super().__init__(parent)
        self.engine = engine
        self.setObjectName("dcpomatic")
        self.setWindowTitle(tr("DCP-o-matic"))
        self.status = QLabel(self)
        self.status.setObjectName("dcpomatic.status")
        self.status.setWordWrap(True)
        self.tabs = QTabWidget(self)
        self.tabs.setObjectName("dcpomatic.tabs")
        self.tabs.addTab(self._dcp_tab(), tr("DCP"))
        self.tabs.addTab(self._kdm_tab(), tr("KDM"))
        self.result = QPlainTextEdit(self)
        self.result.setObjectName("dcpomatic.result")
        self.result.setReadOnly(True)
        self.result.setMaximumHeight(110)
        buttons = QDialogButtonBox(self)
        close = buttons.addButton(QDialogButtonBox.StandardButton.Close)
        close.setObjectName("dcpomatic.close")
        close.setText(tr("Close"))
        close.clicked.connect(self.reject)
        lay = QVBoxLayout(self)
        lay.addWidget(self.status)
        lay.addWidget(self.tabs, 1)
        lay.addWidget(self.result)
        lay.addWidget(buttons)
        self.resize(680, 600)
        self.refresh_status()

    # ---------- building ----------

    def _path_row(self, edit: QLineEdit, name: str, title: str, folder: bool, filt: str = "") -> QHBoxLayout:
        browse = QPushButton(tr("Browse…"), self)
        browse.setObjectName(name)
        browse.clicked.connect(lambda _c=False: self._browse(edit, title, folder, filt))
        row = QHBoxLayout()
        row.addWidget(edit, 1)
        row.addWidget(browse)
        return row

    def _dcp_tab(self) -> QWidget:
        w = QWidget(self)
        self.content = QListWidget(w)
        self.content.setObjectName("dcpomatic.dcp.content")
        add = QPushButton(tr("Add Content…"), w)
        add.setObjectName("dcpomatic.dcp.add")
        add.clicked.connect(self.add_content)
        remove = QPushButton(tr("Remove"), w)
        remove.setObjectName("dcpomatic.dcp.remove")
        remove.clicked.connect(self.remove_content)
        side = QVBoxLayout()
        side.addWidget(add)
        side.addWidget(remove)
        side.addStretch(1)
        content_row = QHBoxLayout()
        content_row.addWidget(self.content, 1)
        content_row.addLayout(side)
        self.dcp_output = QLineEdit(w)
        self.dcp_output.setObjectName("dcpomatic.dcp.output")
        self.dcp_name = QLineEdit(w)
        self.dcp_name.setObjectName("dcpomatic.dcp.name")
        self.kind = _combo(w, "dcpomatic.dcp.kind", CONTENT_KINDS)
        self.standard = _combo(w, "dcpomatic.dcp.standard", STANDARDS)
        self.resolution = _combo(w, "dcpomatic.dcp.resolution", RESOLUTIONS)
        self.container = _combo(w, "dcpomatic.dcp.container", CONTAINERS)
        self.fps = _combo(w, "dcpomatic.dcp.fps", [(r, str(r) if r else N_("From the content")) for r in FRAME_RATES])
        self.encrypt = QCheckBox(tr("Encrypt (KDMs are made from the film folder, so keep it)"), w)
        self.encrypt.setObjectName("dcpomatic.dcp.encrypt")
        self.make_dcp_button = QPushButton(tr("Make DCP"), w)
        self.make_dcp_button.setObjectName("dcpomatic.dcp.make")
        self.make_dcp_button.clicked.connect(self.make_dcp)
        form = QFormLayout()
        form.addRow(tr("Content"), content_row)
        form.addRow(tr("Film folder (new)"), self._path_row(self.dcp_output, "dcpomatic.dcp.outputBrowse", tr("Film folder (new)"), True))
        form.addRow(tr("Name"), self.dcp_name)
        form.addRow(tr("Content kind"), self.kind)
        form.addRow(tr("Standard"), self.standard)
        form.addRow(tr("Resolution"), self.resolution)
        form.addRow(tr("Container"), self.container)
        form.addRow(tr("Frame rate"), self.fps)
        form.addRow("", self.encrypt)
        lay = QVBoxLayout(w)
        lay.addLayout(form)
        lay.addWidget(self.make_dcp_button)
        return w

    def _kdm_tab(self) -> QWidget:
        w = QWidget(self)
        self.kdm_film = QLineEdit(w)
        self.kdm_film.setObjectName("dcpomatic.kdm.film")
        self.kdm_cert = QLineEdit(w)
        self.kdm_cert.setObjectName("dcpomatic.kdm.certificate")
        self.kdm_output = QLineEdit(w)
        self.kdm_output.setObjectName("dcpomatic.kdm.output")
        self.kdm_cinema = QLineEdit(w)
        self.kdm_cinema.setObjectName("dcpomatic.kdm.cinema")
        self.kdm_screen = QLineEdit(w)
        self.kdm_screen.setObjectName("dcpomatic.kdm.screen")
        self.valid_from = QLineEdit("now", w)
        self.valid_from.setObjectName("dcpomatic.kdm.validFrom")
        self.valid_from.setPlaceholderText("now / YYYY-MM-DD HH:MM:SS")
        self.valid_count = QSpinBox(w)
        self.valid_count.setObjectName("dcpomatic.kdm.validCount")
        self.valid_count.setRange(1, 9999)
        self.valid_count.setValue(2)
        self.valid_unit = _combo(w, "dcpomatic.kdm.validUnit", UNITS)
        self.valid_unit.setCurrentIndex(2)
        valid_row = QHBoxLayout()
        valid_row.addWidget(self.valid_count)
        valid_row.addWidget(self.valid_unit, 1)
        self.formulation = _combo(w, "dcpomatic.kdm.formulation", FORMULATIONS)
        self.make_kdm_button = QPushButton(tr("Make KDM"), w)
        self.make_kdm_button.setObjectName("dcpomatic.kdm.make")
        self.make_kdm_button.clicked.connect(self.make_kdm)
        form = QFormLayout()
        form.addRow(tr("Film folder"), self._path_row(self.kdm_film, "dcpomatic.kdm.filmBrowse", tr("Film folder"), True))
        form.addRow(tr("Projector certificate"), self._path_row(self.kdm_cert, "dcpomatic.kdm.certificateBrowse", tr("Projector certificate"), False, "PEM (*.pem *.crt *.cer)"))
        form.addRow(tr("KDM folder"), self._path_row(self.kdm_output, "dcpomatic.kdm.outputBrowse", tr("KDM folder"), True))
        form.addRow(tr("Cinema"), self.kdm_cinema)
        form.addRow(tr("Screen"), self.kdm_screen)
        form.addRow(tr("Valid from (UTC)"), self.valid_from)
        form.addRow(tr("Valid for"), valid_row)
        form.addRow(tr("Formulation"), self.formulation)
        lay = QVBoxLayout(w)
        lay.addLayout(form)
        lay.addWidget(self.make_kdm_button)
        lay.addStretch(1)
        return w

    def _browse(self, edit: QLineEdit, title: str, folder: bool, filt: str) -> None:
        start = os.path.dirname(edit.text()) if edit.text() else ""
        if folder:
            path = QFileDialog.getExistingDirectory(self, title, start)
        else:
            path, _f = QFileDialog.getOpenFileName(self, title, start, f"{filt};;{tr('All files')} (*)")
        if path:
            edit.setText(path)

    # ---------- actions ----------

    def refresh_status(self) -> None:
        self.engine.execute(
            "dcpomatic.status", {}, on_reply=lambda r, _b: self.status.setText(describe_status(r)), on_error=lambda m: self.status.setText(str(m))
        )

    def add_content(self) -> None:
        paths, _f = QFileDialog.getOpenFileNames(self, tr("Add Content…"), "", f"{tr('All files')} (*)")
        for p in paths:
            self.content.addItem(p)

    def remove_content(self) -> None:
        for item in self.content.selectedItems():
            self.content.takeItem(self.content.row(item))

    def make_dcp(self) -> None:
        content = [self.content.item(i).text() for i in range(self.content.count())]
        if not content or not self.dcp_output.text().strip():
            self.result.setPlainText(tr("Add the content files and choose a new film folder."))
            return
        params = dcp_params(
            content,
            self.dcp_output.text().strip(),
            self.dcp_name.text(),
            self.kind.currentData(),
            self.standard.currentData(),
            self.resolution.currentData(),
            self.container.currentData(),
            self.fps.currentData(),
            self.encrypt.isChecked(),
        )

        def got(r: Any, _b: bytes | None) -> None:
            job = r.get("job") if isinstance(r, dict) else None
            self.result.setPlainText(fmt("DCP-o-matic job {job} started; follow it in the Progress panel.", job=job))
            if self.encrypt.isChecked():
                self.kdm_film.setText(params["output"])

        self.engine.execute("dcpomatic.makeDcp", params, on_reply=got, on_error=lambda m: self.result.setPlainText(str(m)))

    def make_kdm(self) -> None:
        if not (self.kdm_film.text().strip() and self.kdm_cert.text().strip() and self.kdm_output.text().strip()):
            self.result.setPlainText(tr("Choose the film folder, the projector certificate and the KDM folder."))
            return
        params = kdm_params(
            self.kdm_film.text(),
            self.kdm_cert.text(),
            self.kdm_output.text(),
            self.kdm_cinema.text(),
            self.kdm_screen.text(),
            self.valid_from.text(),
            self.valid_count.value(),
            self.valid_unit.currentData(),
            self.formulation.currentData(),
        )
        self.make_kdm_button.setEnabled(False)

        def got(r: Any, _b: bytes | None) -> None:
            self.make_kdm_button.setEnabled(True)
            files = r.get("kdms") if isinstance(r, dict) else None
            files = files if isinstance(files, list) else []
            self.result.setPlainText("\n".join([fmt("{count} KDM written", count=len(files)), *map(str, files)]))

        def failed(m: str) -> None:
            self.make_kdm_button.setEnabled(True)
            self.result.setPlainText(str(m))

        self.engine.execute("dcpomatic.makeKdm", params, on_reply=got, on_error=failed)
