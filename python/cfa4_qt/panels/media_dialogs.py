"""Media management dialogs, as egui's ``panels/media_dialogs.rs``: Link Media (offline media /
relink), Make Offline, Create Proxies and Project Manager. Each runs the engine's commands
(``media.*``, ``file.projectManager``) through ``engine.execute``; ``status(text)`` reports what a
finished command did (the status bar) and ``changed()`` asks the window to refresh.

Automation ids (egui's):

- Link Media: ``linkMedia.row.<n>``, ``linkMedia.match.<fileName|extension|clipId|duration|mediaStart|metadata>``,
  ``linkMedia.alignTimecode``, ``linkMedia.relinkOthers``, ``linkMedia.folder``, ``linkMedia.browse``,
  ``linkMedia.exactName``, ``linkMedia.search``, ``linkMedia.candidate.<n>``, ``linkMedia.preview``,
  ``linkMedia.link``, ``linkMedia.locate``, ``linkMedia.offline``, ``linkMedia.offlineAll``, ``linkMedia.cancel``.
- Make Offline: ``makeOffline.keep``, ``makeOffline.delete``, ``makeOffline.ok``, ``makeOffline.cancel``.
- Create Proxies: ``proxies.preset.<id>``, ``proxies.destination``, ``proxies.browse``, ``proxies.ok``, ``proxies.cancel``.
- Project Manager: ``pm.seq.<id>``, ``pm.mode.<collect|consolidate>``, ``pm.preset.<id>``, ``pm.excludeUnused``,
  ``pm.handles``, ``pm.includeProxies``, ``pm.includePreviews``, ``pm.destination``, ``pm.browse``,
  ``pm.calculate``, ``pm.sizes``, ``pm.ok``, ``pm.cancel``.
"""

from __future__ import annotations

from typing import Any

from PySide6.QtCore import Qt
from PySide6.QtGui import QImage, QPixmap
from PySide6.QtWidgets import (
    QAbstractItemView,
    QButtonGroup,
    QCheckBox,
    QDialog,
    QFileDialog,
    QHBoxLayout,
    QHeaderView,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QPushButton,
    QRadioButton,
    QSpinBox,
    QTableWidget,
    QTableWidgetItem,
    QVBoxLayout,
    QWidget,
)

from ..i18n import N_, fmt, tr

MATCH = [
    ("fileName", N_("File Name")),
    ("extension", N_("File Extension")),
    ("clipId", N_("Clip ID (fingerprint)")),
    ("duration", N_("Duration")),
    ("mediaStart", N_("Media Start")),
    ("metadata", N_("Frame Size / Rate")),
]
WARN = "color: #e08a6a;"
DEFAULT_PROXY_PRESET = "prores_proxy_quarter"


def _str(v: Any) -> str:
    return v if isinstance(v, str) else "" if v is None else str(v)


def _int(v: Any) -> int | None:
    return v if isinstance(v, int) and not isinstance(v, bool) else None


def mb(b: int) -> str:
    """egui's sizes: GB from 1 GiB, else MB."""
    b = max(0, b)
    return f"{b / (1 << 30):.2f} GB" if b >= 1 << 30 else f"{b / (1 << 20):.1f} MB"


def missing_rows(answer: Any) -> list[dict[str, Any]]:
    """``media.linkMedia``'s ``missing`` list, damaged entries left out."""
    rows = answer.get("missing") if isinstance(answer, dict) else None
    return [r for r in rows if isinstance(r, dict) and _int(r.get("item")) is not None] if isinstance(rows, list) else []


def candidates_of(answer: Any) -> list[tuple[str, bool, bool | None, str]]:
    """``media.search``'s first result: (path, matches, same file?, problems)."""
    results = answer.get("results") if isinstance(answer, dict) else None
    first = results[0] if isinstance(results, list) and results and isinstance(results[0], dict) else {}
    out = []
    for c in first.get("candidates") or [] if isinstance(first.get("candidates"), list) else []:
        if not isinstance(c, dict) or not isinstance(c.get("path"), str):
            continue
        idm = c.get("identityMatch")
        probs = "; ".join(p for p in c.get("problems") or [] if isinstance(p, str)) if isinstance(c.get("problems"), list) else ""
        out.append((c["path"], c.get("ok") is True, idm if isinstance(idm, bool) else None, probs))
    return out


def sequences_of(node: Any, depth: int = 0) -> list[tuple[int, str]]:
    """(item, name) of the sequences in ``project.inspect``'s bin tree, in tree order."""
    if not isinstance(node, dict) or depth > 64:
        return []
    out: list[tuple[int, str]] = []
    for child in node.get("children") or [] if isinstance(node.get("children"), list) else []:
        if not isinstance(child, dict):
            continue
        if "item" in child:
            if str(child.get("type") or "").lower() == "sequence" and _int(child.get("item")) is not None:
                out.append((child["item"], _str(child.get("name"))))
        else:
            out += sequences_of(child, depth + 1)
    return out


def candidate_mark(ok: bool, same: bool | None) -> str:
    if ok:
        return tr("✔ same file") if same is True else tr("✔ matches")
    return tr("✖ different file") if same is False else "✖"


class _Dialog(QDialog):
    def __init__(self, parent: QWidget | None, engine: Any, name: str, title: str, status: Any = None, changed: Any = None) -> None:
        super().__init__(parent)
        self.engine = engine
        self.status = status or (lambda _t: None)
        self.changed = changed or (lambda: None)
        self.setObjectName(name)
        self.setWindowTitle(tr(title))
        self.lay = QVBoxLayout(self)
        self.message = QLabel(self)
        self.message.setObjectName(f"{name}.message")
        self.message.setWordWrap(True)
        self.message.setStyleSheet(WARN)
        self.message.hide()

    def run(self, cid: str, params: dict[str, Any], ok: Any, failed: Any = None) -> None:
        def err(m: Any) -> None:
            (failed or self.say)(str(m))

        self.engine.execute(cid, params, on_reply=lambda r, _b: ok(r), on_error=err)

    def say(self, text: str) -> None:
        self.message.setText(text)
        self.message.setVisible(bool(text))

    def button(self, name: str, text: str, slot: Any, accent: bool = False) -> QPushButton:
        b = QPushButton(tr(text), self)
        b.setObjectName(name)
        b.setAutoDefault(False)
        if accent:
            b.setDefault(True)
        b.clicked.connect(lambda _c=False: slot())
        return b

    def heading(self, text: str) -> QLabel:
        label = QLabel(tr(text), self)
        f = label.font()
        f.setBold(True)
        label.setFont(f)
        self.lay.addWidget(label)
        return label

    def browse_folder(self, edit: QLineEdit) -> None:
        d = QFileDialog.getExistingDirectory(self, tr("Browse…"), edit.text())
        if d:
            edit.setText(d)


class LinkMediaDialog(_Dialog):
    """The clips whose media is missing; match options; a folder search with candidates and a
    preview; Link / Locate… / Offline / Offline All / Cancel (Cancel leaves the rest offline)."""

    def __init__(self, parent: QWidget | None, engine: Any, status: Any = None, changed: Any = None) -> None:
        super().__init__(parent, engine, "linkMedia", N_("Link Media"), status, changed)
        self.resize(760, 560)
        self.rows: list[dict[str, Any]] = []
        self.skipped: set[int] = set()
        self.row = 0
        self.candidates: list[tuple[str, bool, bool | None, str]] = []
        self.intro = QLabel(self)
        self.intro.setWordWrap(True)
        self.lay.addWidget(self.intro)
        self.table = QTableWidget(0, 4, self)
        self.table.setObjectName("linkMedia.rows")
        self.table.setHorizontalHeaderLabels([tr("Clip Name"), tr("File Name"), tr("File Path"), tr("Status")])
        self.table.setSelectionBehavior(QAbstractItemView.SelectionBehavior.SelectRows)
        self.table.setSelectionMode(QAbstractItemView.SelectionMode.SingleSelection)
        self.table.setEditTriggers(QAbstractItemView.EditTrigger.NoEditTriggers)
        self.table.verticalHeader().hide()
        self.table.horizontalHeader().setSectionResizeMode(2, QHeaderView.ResizeMode.Stretch)
        self.table.setMaximumHeight(200)
        self.table.currentCellChanged.connect(lambda r, _c, _pr, _pc: self._pick_row(r))
        self.lay.addWidget(self.table)
        match = QHBoxLayout()
        match.addWidget(QLabel(tr("Match file properties:"), self))
        self.match: dict[str, QCheckBox] = {}
        for key, label in MATCH:
            b = QCheckBox(tr(label), self)
            b.setObjectName(f"linkMedia.match.{key}")
            b.setChecked(key != "mediaStart")  # egui's defaults
            match.addWidget(b)
            self.match[key] = b
        match.addStretch(1)
        self.lay.addLayout(match)
        opts = QHBoxLayout()
        self.align = QCheckBox(tr("Align Timecode"), self)
        self.align.setObjectName("linkMedia.alignTimecode")
        self.others = QCheckBox(tr("Relink others automatically"), self)
        self.others.setObjectName("linkMedia.relinkOthers")
        self.others.setChecked(True)
        opts.addWidget(self.align)
        opts.addWidget(self.others)
        opts.addStretch(1)
        self.lay.addLayout(opts)
        search = QHBoxLayout()
        search.addWidget(QLabel(tr("Search in:"), self))
        self.folder = QLineEdit(self)
        self.folder.setObjectName("linkMedia.folder")
        self.folder.setPlaceholderText(tr("Folder"))
        search.addWidget(self.folder, 1)
        search.addWidget(self.button("linkMedia.browse", N_("Browse…"), lambda: self.browse_folder(self.folder)))
        self.exact = QCheckBox(tr("Exact name matches only"), self)
        self.exact.setObjectName("linkMedia.exactName")
        self.exact.setChecked(True)
        search.addWidget(self.exact)
        search.addWidget(self.button("linkMedia.search", N_("Search"), self.search))
        self.lay.addLayout(search)
        found = QHBoxLayout()
        left = QVBoxLayout()
        self.none = QLabel(tr("No candidates yet — Search a folder or Locate the file."), self)
        self.none.setEnabled(False)
        left.addWidget(self.none)
        self.list = QListWidget(self)
        self.list.setObjectName("linkMedia.candidates")
        self.list.setMaximumHeight(108)
        self.list.currentRowChanged.connect(lambda _r: self._show_candidate())
        left.addWidget(self.list)
        found.addLayout(left, 1)
        self.preview = QLabel(tr("Preview"), self)
        self.preview.setObjectName("linkMedia.preview")
        self.preview.setFixedSize(192, 108)
        self.preview.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.preview.setStyleSheet("background: rgb(12, 12, 12); color: gray;")
        found.addWidget(self.preview)
        self.lay.addLayout(found)
        self.lay.addWidget(self.message)
        buttons = QHBoxLayout()
        for key, label, slot in (
            ("offlineAll", N_("Offline All"), self.offline_all),
            ("offline", N_("Offline"), self.offline_one),
            ("cancel", N_("Cancel"), self.offline_all),
            ("locate", N_("Locate…"), self.locate),
        ):
            buttons.addWidget(self.button(f"linkMedia.{key}", label, slot))
        self.link_button = self.button("linkMedia.link", N_("Link"), self.link, accent=True)
        self.link_button.setEnabled(False)
        buttons.addWidget(self.link_button)
        buttons.addStretch(1)
        self.lay.addLayout(buttons)
        self.reload()

    # ----- the list of missing media -----
    def reload(self) -> None:
        self.run("media.linkMedia", {}, self._set_rows)

    def _set_rows(self, answer: Any) -> None:
        self.rows = [r for r in missing_rows(answer) if r["item"] not in self.skipped]
        if not self.rows:
            if self.skipped:
                self.offline_all()  # the ones left are offline; the project stops asking
            else:
                self.accept()
            return
        n = len(self.rows)
        self.intro.setText(fmt("{n} clip(s) can't find their media. Locate them, search a folder, or leave them offline.", n=n))
        self.table.blockSignals(True)
        self.table.setRowCount(n)
        for k, r in enumerate(self.rows):
            status = tr("Offline") if r.get("status") == "offline" else tr("Missing")
            for col, text in enumerate((_str(r.get("name")), _str(r.get("fileName")), _str(r.get("path")), status)):
                cell = QTableWidgetItem(text)
                if col == 3:
                    cell.setForeground(Qt.GlobalColor.red)
                self.table.setItem(k, col, cell)
        self.row = min(self.row, n - 1)
        self.table.selectRow(self.row)
        self.table.blockSignals(False)

    def _pick_row(self, r: int) -> None:
        if 0 <= r < len(self.rows) and r != self.row:
            self.row = r
            self._set_candidates([])

    def item(self) -> int | None:
        return self.rows[self.row]["item"] if 0 <= self.row < len(self.rows) else None

    def match_params(self) -> dict[str, Any]:
        return {
            "match": {k: b.isChecked() for k, b in self.match.items()},
            "alignTimecode": self.align.isChecked(),
            "relinkOthers": self.others.isChecked(),
        }

    # ----- candidates -----
    def _set_candidates(self, found: list[tuple[str, bool, bool | None, str]]) -> None:
        self.candidates = found
        self.list.blockSignals(True)
        self.list.clear()
        for k, (path, ok, same, probs) in enumerate(found):
            it = QListWidgetItem(f"{candidate_mark(ok, same)}  {path}")
            it.setData(Qt.ItemDataRole.UserRole, f"linkMedia.candidate.{k}")
            it.setForeground(Qt.GlobalColor.green if ok else Qt.GlobalColor.darkYellow)
            if probs:
                it.setToolTip(probs)
            self.list.addItem(it)
        first = next((k for k, c in enumerate(found) if c[1]), -1)
        self.list.setCurrentRow(first)
        self.list.blockSignals(False)
        self.none.setVisible(not found)
        self._show_candidate()

    def candidate(self) -> str | None:
        k = self.list.currentRow()
        return self.candidates[k][0] if 0 <= k < len(self.candidates) else None

    def _show_candidate(self) -> None:
        path = self.candidate()
        self.link_button.setEnabled(path is not None)
        self.preview.setPixmap(QPixmap())
        self.preview.setText(tr("Preview"))
        if path is None or not hasattr(self.engine, "call"):
            return

        def got(r: Any, data: bytes | None, want: str = path) -> None:
            w, h = (r.get("w"), r.get("h")) if isinstance(r, dict) else (None, None)
            if want != self.candidate() or not isinstance(w, int) or not isinstance(h, int) or not data or len(data) < w * h * 4 or w <= 0 or h <= 0:
                return
            img = QImage(data, w, h, w * 4, QImage.Format.Format_RGBA8888).copy()
            self.preview.setPixmap(QPixmap.fromImage(img).scaled(self.preview.size(), Qt.AspectRatioMode.KeepAspectRatio, Qt.TransformationMode.SmoothTransformation))

        self.engine.call("mediaBrowser.thumbnail", {"path": path, "fraction": 1 / 3, "width": 192}, on_reply=got, on_error=lambda _m: None)

    # ----- actions -----
    def search(self) -> None:
        item = self.item()
        if item is None:
            return
        p = {**self.match_params(), "folder": self.folder.text(), "item": item, "exactName": self.exact.isChecked()}

        def got(r: Any) -> None:
            found = candidates_of(r)
            self._set_candidates(found)
            self.say("" if found else tr("Nothing found with that name."))

        self.run("media.search", p, got)

    def relink_to(self, path: str) -> None:
        item = self.item()
        if item is None:
            return

        def done(r: Any) -> None:
            n = len(r.get("relinked") or []) if isinstance(r, dict) and isinstance(r.get("relinked"), list) else 0
            self.say("")
            self._set_candidates([])
            self.status(fmt("Linked {n} clip(s)", n=n))
            self.changed()
            self.reload()

        self.run("media.relink", {**self.match_params(), "item": item, "path": path}, done)

    def link(self) -> None:
        path = self.candidate()
        if path is not None:
            self.relink_to(path)

    def locate(self) -> None:
        start = _str(self.rows[self.row].get("path")) if 0 <= self.row < len(self.rows) else ""
        path, _f = QFileDialog.getOpenFileName(self, tr("Locate…"), start, tr("All files") + " (*)")
        if path:
            self.relink_to(path)

    def offline_one(self) -> None:
        item = self.item()
        if item is None:
            return
        self.skipped.add(item)
        self._set_candidates([])
        self._set_rows({"missing": self.rows})

    def offline_all(self) -> None:
        self.run("media.offlineAll", {}, lambda _r: (self.changed(), self.accept()), lambda m: (self.say(m), self.reject()))

    def reject(self) -> None:
        # Escape / the title bar's close: the rest stays offline (egui's Cancel)
        if self.isVisible() and self.result() == 0 and not getattr(self, "_leaving", False):
            self._leaving = True
            self.offline_all()
            return
        super().reject()


class MakeOfflineDialog(_Dialog):
    def __init__(self, parent: QWidget | None, engine: Any, count: int, status: Any = None, changed: Any = None) -> None:
        super().__init__(parent, engine, "makeOffline", N_("Make Offline"), status, changed)
        text = QLabel(fmt("Make {n} selected clip(s) offline. Their clips show the offline slate until you link them again.", n=count), self)
        text.setWordWrap(True)
        self.lay.addWidget(text)
        group = QButtonGroup(self)
        self.keep = QRadioButton(tr("Media files remain on disk"), self)
        self.keep.setObjectName("makeOffline.keep")
        self.keep.setChecked(True)
        self.delete = QRadioButton(tr("Media files are deleted"), self)
        self.delete.setObjectName("makeOffline.delete")
        for b in (self.keep, self.delete):
            group.addButton(b)
            self.lay.addWidget(b)
        self.lay.addWidget(self.message)
        row = QHBoxLayout()
        row.addWidget(self.button("makeOffline.cancel", N_("Cancel"), self.reject))
        row.addWidget(self.button("makeOffline.ok", N_("OK"), self.ok, accent=True))
        row.addStretch(1)
        self.lay.addLayout(row)

    def ok(self) -> None:
        self.run("media.makeOffline", {"deleteFiles": self.delete.isChecked()}, lambda _r: (self.changed(), self.accept()))


class CreateProxiesDialog(_Dialog):
    def __init__(self, parent: QWidget | None, engine: Any, items: list[int], status: Any = None, changed: Any = None) -> None:
        super().__init__(parent, engine, "createProxies", N_("Create Proxies"), status, changed)
        self.items = items
        self.preset = DEFAULT_PROXY_PRESET
        self.lay.addWidget(QLabel(fmt("{n} clip(s) selected.", n=len(items)), self))
        self.heading(N_("Format and size"))
        self.group = QButtonGroup(self)
        self.presets = QVBoxLayout()
        self.lay.addLayout(self.presets)
        self.heading(N_("Destination"))
        row = QHBoxLayout()
        self.destination = QLineEdit(self)
        self.destination.setObjectName("proxies.destination")
        self.destination.setPlaceholderText(tr("Next to the original media, in a Proxies folder"))
        self.destination.setMinimumWidth(320)
        row.addWidget(self.destination, 1)
        row.addWidget(self.button("proxies.browse", N_("Browse…"), lambda: self.browse_folder(self.destination)))
        self.lay.addLayout(row)
        note = QLabel(tr("Proxies are made in the background and attached when done. Export always uses full-resolution media."), self)
        note.setWordWrap(True)
        note.setEnabled(False)
        self.lay.addWidget(note)
        self.lay.addWidget(self.message)
        row = QHBoxLayout()
        row.addWidget(self.button("proxies.cancel", N_("Cancel"), self.reject))
        row.addWidget(self.button("proxies.ok", N_("OK"), self.ok, accent=True))
        row.addStretch(1)
        self.lay.addLayout(row)
        self.run("media.proxyPresets", {}, self._set_presets)

    def _set_presets(self, presets: Any) -> None:
        for p in presets if isinstance(presets, list) else []:
            if not isinstance(p, dict) or not isinstance(p.get("id"), str) or p.get("proxy") is not True:
                continue
            b = QRadioButton(_str(p.get("label")) or p["id"], self)
            b.setObjectName(f"proxies.preset.{p['id']}")
            b.setChecked(p["id"] == self.preset)
            b.toggled.connect(lambda on, pid=p["id"]: on and setattr(self, "preset", pid))
            self.group.addButton(b)
            self.presets.addWidget(b)

    def ok(self) -> None:
        dest = self.destination.text().strip()
        params = {"items": self.items, "preset": self.preset, "destination": dest or None}

        def done(r: Any) -> None:
            outputs = r.get("outputs") if isinstance(r, dict) else None
            self.status(fmt("Creating {n} proxy file(s)…", n=len(outputs) if isinstance(outputs, list) else 0))
            self.changed()
            self.accept()

        self.run("media.createProxies", params, done)


class ProjectManagerDialog(_Dialog):
    def __init__(
        self, parent: QWidget | None, engine: Any, sequences: list[tuple[int, str]], chosen: list[int], destination: str, status: Any = None, changed: Any = None
    ) -> None:
        super().__init__(parent, engine, "projectManager", N_("Project Manager"), status, changed)
        self.mode = "collect"
        self.preset = "prores_lt"
        self.heading(N_("Sequences"))
        self.seqs: dict[int, QCheckBox] = {}
        for sid, name in sequences:
            b = QCheckBox(name, self)
            b.setObjectName(f"pm.seq.{sid}")
            b.setChecked(sid in chosen)
            b.toggled.connect(lambda _on: self._changed())
            self.lay.addWidget(b)
            self.seqs[sid] = b
        self.heading(N_("Resulting Project"))
        group = QButtonGroup(self)
        self.modes: dict[str, QRadioButton] = {}
        for key, label in (("collect", N_("Collect Files and Copy to New Location")), ("consolidate", N_("Consolidate and Transcode"))):
            b = QRadioButton(tr(label), self)
            b.setObjectName(f"pm.mode.{key}")
            b.setChecked(key == self.mode)
            b.toggled.connect(lambda on, k=key: on and self._set_mode(k))
            group.addButton(b)
            self.lay.addWidget(b)
            self.modes[key] = b
        self.preset_box = QWidget(self)
        self.preset_lay = QVBoxLayout(self.preset_box)
        self.preset_lay.setContentsMargins(20, 0, 0, 0)
        self.preset_group = QButtonGroup(self)
        self.lay.addWidget(self.preset_box)
        self.heading(N_("Options"))
        self.exclude = QCheckBox(tr("Exclude Unused Clips"), self)
        self.exclude.setObjectName("pm.excludeUnused")
        self.exclude.setChecked(True)
        self.exclude.toggled.connect(lambda _on: self._changed())
        self.lay.addWidget(self.exclude)
        row = QHBoxLayout()
        self.handles_label = QLabel(tr("Include Handles:"), self)
        row.addWidget(self.handles_label)
        self.handles = QSpinBox(self)
        self.handles.setObjectName("pm.handles")
        self.handles.setRange(0, 600)
        self.handles.setValue(30)
        self.handles.setSuffix(tr(" frames"))
        self.handles.valueChanged.connect(lambda _v: self._changed())
        row.addWidget(self.handles)
        row.addStretch(1)
        self.lay.addLayout(row)
        self.proxies = QCheckBox(tr("Include Proxies"), self)
        self.proxies.setObjectName("pm.includeProxies")
        self.proxies.setChecked(True)
        self.lay.addWidget(self.proxies)
        self.previews = QCheckBox(tr("Include Preview Files"), self)
        self.previews.setObjectName("pm.includePreviews")
        self.lay.addWidget(self.previews)
        self.heading(N_("Destination Path"))
        row = QHBoxLayout()
        self.destination = QLineEdit(destination, self)
        self.destination.setObjectName("pm.destination")
        self.destination.setMinimumWidth(360)
        self.destination.textChanged.connect(lambda _t: self._update_ok())
        row.addWidget(self.destination, 1)
        row.addWidget(self.button("pm.browse", N_("Browse…"), lambda: self.browse_folder(self.destination)))
        self.lay.addLayout(row)
        row = QHBoxLayout()
        self.sizes = QLabel(tr("Disk space: —"), self)
        self.sizes.setObjectName("pm.sizes")
        row.addWidget(self.sizes)
        row.addWidget(self.button("pm.calculate", N_("Calculate"), self.calculate))
        row.addStretch(1)
        self.lay.addLayout(row)
        self.lay.addWidget(self.message)
        row = QHBoxLayout()
        row.addWidget(self.button("pm.cancel", N_("Cancel"), self.reject))
        self.ok_button = self.button("pm.ok", N_("OK"), self.ok, accent=True)
        row.addWidget(self.ok_button)
        row.addStretch(1)
        self.lay.addLayout(row)
        self._set_mode(self.mode)
        self.run("media.proxyPresets", {}, self._set_presets)

    def _set_presets(self, presets: Any) -> None:
        for p in presets if isinstance(presets, list) else []:
            if not isinstance(p, dict) or not isinstance(p.get("id"), str) or p.get("proxy") is not False:
                continue
            b = QRadioButton(_str(p.get("label")) or p["id"], self)
            b.setObjectName(f"pm.preset.{p['id']}")
            b.setChecked(p["id"] == self.preset)
            b.toggled.connect(lambda on, pid=p["id"]: on and self._set_preset(pid))
            self.preset_group.addButton(b)
            self.preset_lay.addWidget(b)

    def _set_preset(self, pid: str) -> None:
        self.preset = pid
        self._changed()

    def _set_mode(self, mode: str) -> None:
        self.mode = mode
        consolidate = mode == "consolidate"
        self.preset_box.setVisible(consolidate)
        self.handles.setEnabled(consolidate)
        self.handles_label.setEnabled(consolidate)
        self.proxies.setEnabled(not consolidate)
        self._changed()

    def _changed(self) -> None:
        # the settings changed: the estimate is stale
        self.sizes.setText(tr("Disk space: —"))
        self._update_ok()

    def _update_ok(self) -> None:
        if hasattr(self, "ok_button"):
            self.ok_button.setEnabled(bool(self.destination.text()) and bool(self.sequences()))

    def sequences(self) -> list[int]:
        return [sid for sid, b in self.seqs.items() if b.isChecked()]

    def params(self, dry: bool) -> dict[str, Any]:
        return {
            "destination": self.destination.text(),
            "mode": self.mode,
            "sequences": self.sequences(),
            "excludeUnused": self.exclude.isChecked(),
            "handles": self.handles.value(),
            "preset": self.preset,
            "includeProxies": self.proxies.isChecked(),
            "includePreviews": self.previews.isChecked(),
            "dryRun": dry,
        }

    def calculate(self) -> None:
        def got(r: Any) -> None:
            r = r if isinstance(r, dict) else {}
            files = r.get("files")
            self.sizes.setText(
                fmt(
                    "Disk space: original {original} · resulting {result} ({n} files)",
                    original=mb(_int(r.get("originalBytes")) or 0),
                    result=mb(_int(r.get("resultBytes")) or 0),
                    n=len(files) if isinstance(files, list) else 0,
                )
            )
            self.say("")

        self.run("file.projectManager", self.params(True), got)

    def ok(self) -> None:
        def done(r: Any) -> None:
            self.status(fmt("Project Manager: writing {path}", path=_str(r.get("project") if isinstance(r, dict) else "")))
            self.accept()

        self.run("file.projectManager", self.params(False), done)


__all__ = ["CreateProxiesDialog", "LinkMediaDialog", "MakeOfflineDialog", "ProjectManagerDialog", "candidates_of", "mb", "missing_rows", "sequences_of"]
