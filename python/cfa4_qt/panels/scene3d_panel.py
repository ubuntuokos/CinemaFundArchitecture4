"""The 3D asset panel (CFA4-OPENUSD-3D-001 U1-UI / U1-A6) and the 3D export dialog (CFA4-OPENUSD-3D-EXPORT-001
U3a-A8): the Qt 6 face of the engine's ``scene3d.*`` commands. They hold no logic of their own; every
action is one engine command.

Reached from Tools ▸ 3D Tools (``tools.3d.assets``, ``tools.3d.export``), so a project without 3D
content shows nothing of it (3D is video-project dependent, VISION §8 decision).

**Asset panel** (``scene3d.panel``): the asset table ``scene3d.assets`` (name, format, route, status,
file), the selected asset's unsupported / approximated report ``scene3d.report``, ``scene3d.import``
(file picker, then ``scene3d.import``), ``scene3d.route.A`` / ``scene3d.route.B`` (the path choice:
A external EXR replacement, B native import; B is off for USD), ``scene3d.relink`` (a changed file asks
before ``allowChanged``), ``scene3d.check`` (status column: ok / missing / changed, shown in red),
``scene3d.remove``, ``scene3d.export`` (opens the export dialog) and ``scene3d.message`` (the last
error or result).

**Export dialog** (``scene3d.exportDialog``): ``scene3d.exportDialog.asset``, ``.format`` (glb, gltf,
obj, stl, stl-ascii), ``.dir``, ``.browse``, ``.name``, ``.copyTextures``, ``.embedTextures`` (glb only),
``.overwrite``, ``.report`` (the loss report), ``.result``, ``.ok`` (Export; after a report that needs
approval it becomes "Approve and write", which sends ``approve``), ``.cancel``. The report always
comes first: a lossy export writes nothing until the user approved that very report; changing any
option goes back to Export.
"""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QAbstractItemView,
    QCheckBox,
    QComboBox,
    QDialog,
    QFileDialog,
    QGridLayout,
    QHBoxLayout,
    QHeaderView,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QMessageBox,
    QPushButton,
    QTableWidget,
    QTableWidgetItem,
    QVBoxLayout,
    QWidget,
)

from ..i18n import N_, fmt, tr

ERROR_STYLE = "color: #e06060;"
OK_STYLE = "color: #60b060;"
FORMATS = (
    ("glb", N_("glTF binary (.glb)")),
    ("gltf", N_("glTF + .bin (.gltf)")),
    ("obj", N_("OBJ + MTL (.obj)")),
    ("stl", N_("STL binary (.stl)")),
    ("stl-ascii", N_("STL text (.stl)")),
)
MODEL_FILTER = "3D models (*.glb *.gltf *.obj *.stl *.usd *.usda *.usdc *.usdz);;All files (*)"
SEVERITY = {"unsupported": N_("not carried"), "approximated": N_("approximated")}
Executor = Any  # the EngineClient: ``execute(command, params, on_reply, on_error)``


def _text(v: Any) -> str:
    return v if isinstance(v, str) else ""


def _rows(v: Any) -> list[dict[str, Any]]:
    return [x for x in v if isinstance(x, dict)] if isinstance(v, list) else []


def report_lines(report: Any) -> list[tuple[str, str]]:
    """``(severity, text)`` per report entry, ``unsupported`` first (no silent loss)."""
    rows = [
        (_text(r.get("severity")), f"{_text(r.get('path'))}: {_text(r.get('message'))}".strip(": "))
        for r in _rows(report)
    ]
    return sorted(rows, key=lambda r: r[0] != "unsupported")


def fill_report(widget: QListWidget, report: Any, dropped: int = 0) -> None:
    widget.clear()
    for sev, text in report_lines(report):
        item = QListWidgetItem(f"[{tr(SEVERITY.get(sev, sev))}] {text}")
        if sev == "unsupported":
            item.setForeground(Qt.GlobalColor.red)
        widget.addItem(item)
    if dropped:
        widget.addItem(QListWidgetItem(fmt("{n} more entries were not listed.", n=dropped)))


class _Base(QDialog):
    """A non-modal tool window: ``run`` sends an engine command and calls back on the GUI thread's
    next event, never blocking (the control channel reads the widgets while they are open)."""

    def __init__(self, parent: QWidget | None, engine: Executor, name: str, title: str) -> None:
        super().__init__(parent)
        self.engine = engine
        self.setObjectName(name)
        self.setWindowTitle(tr(title))
        self.setModal(False)
        self.lay = QVBoxLayout(self)
        self.message = QLabel(self)
        self.message.setObjectName(f"{name}.message")
        self.message.setWordWrap(True)
        self.message.hide()

    def run(
        self,
        command: str,
        params: dict[str, Any],
        ok: Callable[[Any], None],
        failed: Callable[[str], None] | None = None,
    ) -> None:
        self.engine.execute(
            command, params, on_reply=lambda r, _b=None: ok(r), on_error=lambda m: (failed or self.say_error)(str(m))
        )

    def say_error(self, text: str) -> None:
        self.say(text, error=True)

    def say(self, text: str, error: bool = False) -> None:
        self.message.setStyleSheet(ERROR_STYLE if error else OK_STYLE)
        self.message.setText(text)
        self.message.setVisible(bool(text))

    def button(self, name: str, text: str, slot: Callable[[], None]) -> QPushButton:
        b = QPushButton(tr(text), self)
        b.setObjectName(name)
        b.setAutoDefault(False)
        b.clicked.connect(lambda _c=False: slot())
        return b


class Scene3dPanel(_Base):
    """The project's 3D assets and what the engine knows about them."""

    COLUMNS = (N_("Name"), N_("Format"), N_("Route"), N_("Status"), N_("File"))

    def __init__(self, parent: QWidget | None, engine: Executor, changed: Callable[[], None] | None = None) -> None:
        super().__init__(parent, engine, "scene3d.panel", N_("3D Assets"))
        self.changed = changed or (lambda: None)
        self.resize(820, 520)
        self.assets: list[dict[str, Any]] = []
        self.status: dict[int, tuple[str, str]] = {}  # asset id -> (ok|missing|changed, message)
        self.export_dialog: ExportDialog | None = None
        self.export_when_listed = False  # Tools ▸ Export 3D Asset…: open the dialog once the list is here
        self.table = QTableWidget(0, len(self.COLUMNS), self)
        self.table.setObjectName("scene3d.assets")
        self.table.setHorizontalHeaderLabels([tr(c) for c in self.COLUMNS])
        self.table.setSelectionBehavior(QAbstractItemView.SelectionBehavior.SelectRows)
        self.table.setSelectionMode(QAbstractItemView.SelectionMode.SingleSelection)
        self.table.setEditTriggers(QAbstractItemView.EditTrigger.NoEditTriggers)
        self.table.verticalHeader().hide()
        self.table.horizontalHeader().setSectionResizeMode(4, QHeaderView.ResizeMode.Stretch)
        self.table.itemSelectionChanged.connect(self._selected)
        self.lay.addWidget(self.table)
        self.summary = QLabel(self)
        self.summary.setObjectName("scene3d.summary")
        self.summary.setWordWrap(True)
        self.lay.addWidget(self.summary)
        self.lay.addWidget(QLabel(tr("Not carried or approximated (unsupported list):"), self))
        self.report = QListWidget(self)
        self.report.setObjectName("scene3d.report")
        self.report.setMaximumHeight(140)
        self.lay.addWidget(self.report)
        self.lay.addWidget(self.message)
        row = QHBoxLayout()
        self.import_button = self.button("scene3d.import", N_("Import…"), self.import_file)
        self.route_a = self.button("scene3d.route.A", N_("Route A: external EXR"), lambda: self.set_route("A"))
        self.route_b = self.button("scene3d.route.B", N_("Route B: native import"), lambda: self.set_route("B"))
        self.relink_button = self.button("scene3d.relink", N_("Relink…"), self.relink)
        self.check_button = self.button("scene3d.check", N_("Check"), self.check)
        self.remove_button = self.button("scene3d.remove", N_("Remove"), self.remove)
        self.export_button = self.button("scene3d.export", N_("Export…"), self.open_export)
        for b in (
            self.import_button,
            self.route_a,
            self.route_b,
            self.relink_button,
            self.check_button,
            self.remove_button,
            self.export_button,
        ):
            row.addWidget(b)
        row.addStretch(1)
        self.lay.addLayout(row)
        self._buttons()
        self.refresh()

    # ----- state -----
    def current(self) -> dict[str, Any] | None:
        rows = self.table.selectionModel().selectedRows() if self.table.selectionModel() else []
        i = rows[0].row() if rows else -1
        return self.assets[i] if 0 <= i < len(self.assets) else None

    def refresh(self, select: int | None = None) -> None:
        keep = select if select is not None else (self.current() or {}).get("id")
        self.run("scene3d.list", {}, lambda r: self._listed(r, keep))

    def _listed(self, reply: Any, keep: Any) -> None:
        self.assets = [
            a
            for a in _rows((reply or {}).get("assets") if isinstance(reply, dict) else None)
            if isinstance(a.get("id"), int)
        ]
        self.status = {k: v for k, v in self.status.items() if any(a["id"] == k for a in self.assets)}
        self.table.blockSignals(True)
        self.table.setRowCount(len(self.assets))
        for r, a in enumerate(self.assets):
            state, why = self.status.get(a["id"], ("ok" if a.get("present") else "missing", ""))
            if a.get("present") is False:
                state, why = "missing", why or tr("The file is missing; relink it.")
            cells = [
                _text(a.get("name")),
                _text(a.get("format")),
                _text(a.get("route")),
                tr(state),
                _text(a.get("path")),
            ]
            for c, text in enumerate(cells):
                item = QTableWidgetItem(text)
                if c == 3:
                    item.setToolTip(why)
                    if state != "ok":
                        item.setForeground(Qt.GlobalColor.red)
                self.table.setItem(r, c, item)
            if a["id"] == keep:
                self.table.selectRow(r)
        self.table.blockSignals(False)
        if self.current() is None and self.assets:
            self.table.selectRow(0)
        self._selected()
        if self.export_when_listed:
            self.export_when_listed = False
            if self.current() is not None and self.export_button.isEnabled():
                self.open_export()
            else:
                self.say_error(tr("There is no 3D asset that can be exported. Import one first."))

    def _selected(self) -> None:
        a = self.current()
        if a is None:
            self.report.clear()
            self.summary.setText(
                tr("No 3D assets in this project. Import a glTF, OBJ or STL file (or a USD file on route A).")
            )
        else:
            raw = a.get("stats")
            stats: dict[str, Any] = raw if isinstance(raw, dict) else {}
            self.summary.setText(
                fmt(
                    "{meshes} meshes, {triangles} triangles, {materials} materials, {cameras} cameras, {animations} animations.",
                    meshes=stats.get("meshes", 0),
                    triangles=stats.get("triangles", 0),
                    materials=stats.get("materials", 0),
                    cameras=stats.get("cameras", 0),
                    animations=stats.get("animations", 0),
                )
            )
            fill_report(self.report, a.get("report"), int(a.get("dropped") or 0))
        self._buttons()

    def _buttons(self) -> None:
        a = self.current()
        has = a is not None
        usd = a is not None and _text(a.get("format")) == "usd"
        route = _text(a.get("route")).upper() if a is not None else ""
        self.route_a.setEnabled(has and route != "A")
        self.route_b.setEnabled(has and route != "B" and not usd)
        self.route_b.setToolTip(tr("USD has no native import in this build.") if usd else "")
        for b in (self.relink_button, self.remove_button, self.export_button):
            b.setEnabled(has)
        self.export_button.setEnabled(has and not usd)
        self.export_button.setToolTip(tr("USD cannot be exported yet.") if usd else "")
        self.check_button.setEnabled(bool(self.assets))

    def _done(self, text: str, select: int | None = None) -> None:
        self.say(text)
        self.changed()
        self.refresh(select)

    # ----- actions -----
    def import_file(self, path: str | None = None) -> None:
        path = path or QFileDialog.getOpenFileName(self, tr("Import 3D Asset"), "", tr(MODEL_FILTER))[0]
        if not path:
            return
        self.run(
            "scene3d.import",
            {"path": path},
            lambda r: self._done(
                fmt("Imported {name}.", name=_text(r.get("name")) if isinstance(r, dict) else path),
                r.get("id") if isinstance(r, dict) else None,
            ),
        )

    def set_route(self, route: str) -> None:
        a = self.current()
        if a is not None:
            self.run(
                "scene3d.route",
                {"id": a["id"], "route": route},
                lambda _r: self._done(fmt("Route {route} set.", route=route), a["id"]),
            )

    def relink(self, path: str | None = None) -> None:
        a = self.current()
        if a is None:
            return
        path = (
            path or QFileDialog.getOpenFileName(self, tr("Relink 3D Asset"), _text(a.get("path")), tr(MODEL_FILTER))[0]
        )
        if not path:
            return

        def failed(message: str) -> None:
            if "allowChanged" not in message:
                self.say_error(message)
                return
            self.say_error(message)
            ask = QMessageBox(
                QMessageBox.Icon.Question,
                tr("Relink 3D Asset"),
                tr("The file differs from the one imported. Read it again and replace the asset's data?"),
                parent=self,
            )
            ask.setObjectName("scene3d.relink.confirm")
            ask.setStandardButtons(QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No)
            ask.setModal(False)
            ask.buttonClicked.connect(
                lambda b: (
                    self.relink_changed(a["id"], path)
                    if ask.standardButton(b) == QMessageBox.StandardButton.Yes
                    else None
                )
            )
            ask.open()
            self.confirm = ask

        self.run(
            "scene3d.relink", {"id": a["id"], "path": path}, lambda _r: self._done(tr("Relinked."), a["id"]), failed
        )

    def relink_changed(self, asset: int, path: str) -> None:
        self.run(
            "scene3d.relink",
            {"id": asset, "path": path, "allowChanged": True},
            lambda _r: self._done(tr("Relinked; the changed file was read again."), asset),
        )

    def check(self) -> None:
        self.run("scene3d.check", {}, self._checked)

    def _checked(self, reply: Any) -> None:
        for r in _rows((reply or {}).get("assets") if isinstance(reply, dict) else None):
            if isinstance(r.get("id"), int):
                self.status[r["id"]] = (_text(r.get("status")) or "ok", _text(r.get("message")))
        problems = (reply or {}).get("problems", 0) if isinstance(reply, dict) else 0
        self.say(
            tr("All 3D files are as imported.")
            if not problems
            else fmt("{n} asset(s) have a problem; see the Status column.", n=problems),
            error=bool(problems),
        )
        self.refresh()

    def remove(self) -> None:
        a = self.current()
        if a is not None:
            self.run(
                "scene3d.remove",
                {"id": a["id"]},
                lambda _r: self._done(tr("Removed the reference (the file is untouched).")),
            )

    def open_export(self) -> None:
        a = self.current()
        if a is None:
            return
        if self.export_dialog is not None:
            self.export_dialog.close()
        d = self.export_dialog = ExportDialog(self, self.engine, self.assets, a["id"])
        d.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        d.finished.connect(lambda _r: setattr(self, "export_dialog", None))
        d.open()


class ExportDialog(_Base):
    """``scene3d.export`` for one asset: the loss report first, ``approve`` only after the user saw it."""

    def __init__(
        self,
        parent: QWidget | None,
        engine: Executor,
        assets: list[dict[str, Any]],
        asset_id: int | None = None,
        folder: str = "",
    ) -> None:
        super().__init__(parent, engine, "scene3d.exportDialog", N_("Export 3D Asset"))
        self.resize(640, 520)
        self.needs_approval = False
        self.written: list[str] = []
        p = "scene3d.exportDialog"
        grid = QGridLayout()
        self.asset = QComboBox(self)
        self.asset.setObjectName(f"{p}.asset")
        for a in assets:
            if _text(a.get("format")) != "usd":
                self.asset.addItem(_text(a.get("name")), a.get("id"))
        i = self.asset.findData(asset_id)
        self.asset.setCurrentIndex(max(i, 0))
        self.format = QComboBox(self)
        self.format.setObjectName(f"{p}.format")
        for key, label in FORMATS:
            self.format.addItem(tr(label), key)
        self.dir = QLineEdit(folder, self)
        self.dir.setObjectName(f"{p}.dir")
        self.dir.setPlaceholderText(tr("Output folder (must exist)"))
        browse = self.button(f"{p}.browse", N_("Browse…"), self.browse)
        self.name = QLineEdit(self)
        self.name.setObjectName(f"{p}.name")
        self.name.setPlaceholderText(tr("File name without extension (default: the asset's name)"))
        for r, (label, w) in enumerate(
            ((N_("Asset"), self.asset), (N_("Format"), self.format), (N_("Folder"), self.dir), (N_("Name"), self.name))
        ):
            grid.addWidget(QLabel(tr(label), self), r, 0)
            grid.addWidget(w, r, 1)
        grid.addWidget(browse, 2, 2)
        self.lay.addLayout(grid)
        self.copy_textures = QCheckBox(tr("Copy the texture images next to the file"), self)
        self.copy_textures.setObjectName(f"{p}.copyTextures")
        self.copy_textures.setChecked(True)
        self.embed = QCheckBox(tr("Embed the images in the .glb"), self)
        self.embed.setObjectName(f"{p}.embedTextures")
        self.overwrite = QCheckBox(tr("Replace files that already exist"), self)
        self.overwrite.setObjectName(f"{p}.overwrite")
        for w in (self.copy_textures, self.embed, self.overwrite):
            self.lay.addWidget(w)
        self.lay.addWidget(QLabel(tr("Loss report (what this format cannot carry):"), self))
        self.report = QListWidget(self)
        self.report.setObjectName(f"{p}.report")
        self.lay.addWidget(self.report)
        self.outcome = QLabel(self)
        self.outcome.setObjectName(f"{p}.result")
        self.outcome.setWordWrap(True)
        self.lay.addWidget(self.outcome)
        self.lay.addWidget(self.message)
        buttons = QHBoxLayout()
        self.ok = QPushButton(tr("Export"), self)
        self.ok.setObjectName(f"{p}.ok")
        self.ok.setDefault(True)
        self.ok.clicked.connect(lambda _c=False: self.send())
        self.cancel = QPushButton(tr("Close"), self)
        self.cancel.setObjectName(f"{p}.cancel")
        self.cancel.clicked.connect(self.reject)
        buttons.addWidget(self.ok)
        buttons.addWidget(self.cancel)
        buttons.addStretch(1)
        self.lay.addLayout(buttons)
        for sig in (
            self.asset.currentIndexChanged,
            self.format.currentIndexChanged,
            self.dir.textChanged,
            self.name.textChanged,
            self.copy_textures.toggled,
            self.embed.toggled,
            self.overwrite.toggled,
        ):
            sig.connect(lambda *_a: self._reset())
        self.format.currentIndexChanged.connect(lambda *_a: self._embed_enabled())
        self._embed_enabled()
        self._reset()

    def _embed_enabled(self) -> None:
        glb = self.format.currentData() == "glb"
        self.embed.setEnabled(glb)
        if not glb:
            self.embed.setChecked(False)

    def browse(self) -> None:
        d = QFileDialog.getExistingDirectory(self, tr("Output folder"), self.dir.text())
        if d:
            self.dir.setText(d)

    def _reset(self) -> None:
        """An option changed: the shown report no longer belongs to what would be written."""
        self.needs_approval = False
        self.ok.setText(tr("Export"))
        self.ok.setEnabled(self.asset.count() > 0 and bool(self.dir.text().strip()))
        self.report.clear()
        self.outcome.setText("")

    def params(self, approve: bool = False) -> dict[str, Any]:
        out: dict[str, Any] = {
            "id": self.asset.currentData(),
            "dir": self.dir.text().strip(),
            "format": self.format.currentData(),
            "copyTextures": self.copy_textures.isChecked(),
            "overwrite": self.overwrite.isChecked(),
        }
        if self.name.text().strip():
            out["name"] = self.name.text().strip()
        if self.embed.isEnabled() and self.embed.isChecked():
            out["embedTextures"] = True
        if approve:
            out["approve"] = True
        return out

    def send(self) -> None:
        approve = self.needs_approval
        self.say("")
        self.ok.setEnabled(False)
        self.run("scene3d.export", self.params(approve), lambda r: self._answered(r, approve), self._failed)

    def _failed(self, message: str) -> None:
        self.ok.setEnabled(True)
        self.say_error(message)

    def _answered(self, reply: Any, approved: bool) -> None:
        r = reply if isinstance(reply, dict) else {}
        fill_report(self.report, r.get("report"), int(r.get("dropped") or 0))
        self.ok.setEnabled(True)
        if r.get("written") is True:
            self.needs_approval = False
            self.written = [str(p) for p in r.get("paths", [])] if isinstance(r.get("paths"), list) else []
            self.ok.setText(tr("Export"))
            self.outcome.setText(fmt("Written: {paths}", paths=", ".join(self.written)))
            self.say(tr("Exported (with the losses listed above).") if approved else tr("Exported without loss."))
            return
        self.needs_approval = bool(r.get("needsApproval"))
        if self.needs_approval:
            self.ok.setText(tr("Approve and write"))
            self.outcome.setText(
                tr("Nothing was written. This format cannot carry everything listed above; approve to write it anyway.")
            )
        else:
            self.say_error(_text(r.get("message")) or tr("Nothing was written."))
