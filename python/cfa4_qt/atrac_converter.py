"""Tools ▸ Audio Tools ▸ AT9/AT3 Converter… (owner request, 2026-11-14).

Converts Sony ATRAC audio files (AT3 with ATRAC3 or ATRAC3plus, AT9, OMA) and other audio files
to WAV, and audio files (WAV, MP3, …) to AT3 (ATRAC3 at 66, 105 or 132 kbit/s), with CFA4's own
decoders and encoder. The dialog only collects the files and settings: every conversion is the
engine command ``audio.atracConvert``, run as a background job one file after another and
followed through ``jobs.list``.

Every widget has an automation id (``atracConverter.*``) and the dialog's state is readable and
settable as JSON (``ui_state`` / ``set_ui_state``).
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from PySide6.QtCore import QTimer
from PySide6.QtWidgets import (
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QFileDialog,
    QFormLayout,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QProgressBar,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from .dialogs import confirm_replace
from .i18n import tr

INPUT_FILTER = "Audio (*.at3 *.at9 *.oma *.omg *.aa3 *.wav *.mp3 *.flac *.ogg *.m4a);;All files (*)"
# (format, label) of the outputs; ATRAC3plus, AT9 and MP3 encoding come later
OUTPUTS = (("at3", "AT3 (ATRAC3, PSP / PS3)"), ("wav", "WAV (PCM)"))
RATES = ((132, "132 kbit/s (LP2)"), (105, "105 kbit/s"), (66, "66 kbit/s (LP4, joint stereo)"))
BITS = ((16, "16-bit"), (24, "24-bit"), (32, "32-bit float"))
POLL_MS = 200


def output_path(source: str, folder: str, fmt: str) -> str:
    """The output of ``source``: same name with the new extension, in ``folder`` (or next to the
    source); a source that already has that extension gets ``_converted``."""
    src = Path(source)
    stem = src.stem if src.suffix.lower() != f".{fmt}" else f"{src.stem}_converted"
    return str(Path(folder or str(src.parent)) / f"{stem}.{fmt}")


class AtracConverterDialog(QDialog):
    def __init__(self, engine: Any, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.engine = engine
        self.setObjectName("dialog.atracConverter")
        self.setWindowTitle(tr("AT9/AT3 Converter"))
        self.resize(560, 420)
        self._queue: list[tuple[str, str]] = []
        self._job: int | None = None
        self._done = 0
        self._total = 0
        self._current = ""
        self._errors: list[str] = []
        self._timer = QTimer(self)
        self._timer.setInterval(POLL_MS)
        self._timer.timeout.connect(self._poll)

        root = QVBoxLayout(self)
        root.addWidget(QLabel(tr("Files to convert:")))
        self.files = QListWidget()
        self.files.setObjectName("atracConverter.files")
        self.files.setSelectionMode(QListWidget.SelectionMode.ExtendedSelection)
        root.addWidget(self.files, 1)
        row = QHBoxLayout()
        self.add = QPushButton(tr("Add Files…"))
        self.add.setObjectName("atracConverter.add")
        self.add.clicked.connect(self._add_files)
        self.remove = QPushButton(tr("Remove"))
        self.remove.setObjectName("atracConverter.remove")
        self.remove.clicked.connect(self._remove_selected)
        row.addWidget(self.add)
        row.addWidget(self.remove)
        row.addStretch(1)
        root.addLayout(row)

        form = QFormLayout()
        self.format = QComboBox()
        self.format.setObjectName("atracConverter.format")
        for fmt, label in OUTPUTS:
            self.format.addItem(tr(label), fmt)
        self.format.currentIndexChanged.connect(self._format_changed)
        form.addRow(tr("Convert to:"), self.format)
        self.rate = QComboBox()
        self.rate.setObjectName("atracConverter.bitrate")
        for kbps, label in RATES:
            self.rate.addItem(tr(label), kbps)
        form.addRow(tr("Bit rate:"), self.rate)
        self.bits = QComboBox()
        self.bits.setObjectName("atracConverter.bits")
        for b, label in BITS:
            self.bits.addItem(tr(label), b)
        form.addRow(tr("Sample format:"), self.bits)
        folder_row = QHBoxLayout()
        self.folder = QLineEdit()
        self.folder.setObjectName("atracConverter.folder")
        self.folder.setPlaceholderText(tr("Next to each source file"))
        browse = QPushButton(tr("Browse…"))
        browse.setObjectName("atracConverter.browse")
        browse.clicked.connect(self._browse)
        folder_row.addWidget(self.folder, 1)
        folder_row.addWidget(browse)
        form.addRow(tr("Output folder:"), folder_row)
        root.addLayout(form)
        note = QLabel(
            tr("AT9 and ATRAC3plus files convert to WAV; encoding AT9, ATRAC3plus and MP3 is not available yet.")
        )
        note.setObjectName("atracConverter.note")
        note.setWordWrap(True)
        root.addWidget(note)

        self.progress = QProgressBar()
        self.progress.setObjectName("atracConverter.progress")
        self.progress.setRange(0, 1000)
        root.addWidget(self.progress)
        self.status = QLabel("")
        self.status.setObjectName("atracConverter.status")
        self.status.setWordWrap(True)
        root.addWidget(self.status)

        buttons = QDialogButtonBox()
        self.convert = buttons.addButton(tr("Convert"), QDialogButtonBox.ButtonRole.AcceptRole)
        self.convert.setObjectName("atracConverter.convert")
        self.convert.clicked.connect(self.start)
        self.cancel = buttons.addButton(tr("Cancel"), QDialogButtonBox.ButtonRole.RejectRole)
        self.cancel.setObjectName("atracConverter.cancel")
        self.cancel.clicked.connect(self._cancel)
        root.addWidget(buttons)
        self._format_changed()

    # ----- state -----
    def target(self) -> str:
        return str(self.format.currentData())

    def ui_state(self) -> dict[str, Any]:
        return {
            "files": [self.files.item(i).text() for i in range(self.files.count())],
            "format": self.target(),
            "bitrateKbps": self.rate.currentData(),
            "bits": self.bits.currentData(),
            "folder": self.folder.text(),
            "running": self.running(),
            "status": self.status.text(),
        }

    def set_ui_state(self, patch: dict[str, Any]) -> None:
        if "files" in patch:
            self.files.clear()
            self.files.addItems([str(f) for f in patch["files"]])
        for key, box in (("format", self.format), ("bitrateKbps", self.rate), ("bits", self.bits)):
            if key in patch:
                i = box.findData(patch[key])
                if i >= 0:
                    box.setCurrentIndex(i)
        if "folder" in patch:
            self.folder.setText(str(patch["folder"]))

    def running(self) -> bool:
        return self._job is not None or bool(self._queue)

    def params(self, source: str, output: str) -> dict[str, Any]:
        p: dict[str, Any] = {"input": source, "output": output, "format": self.target()}
        if self.target() == "at3":
            p["bitrateKbps"] = self.rate.currentData()
        else:
            p["bits"] = self.bits.currentData()
        return p

    # ----- actions -----
    def _format_changed(self, _i: int = 0) -> None:
        at3 = self.target() == "at3"
        self.rate.setEnabled(at3)
        self.bits.setEnabled(not at3)

    def _add_files(self) -> None:
        paths, _ = QFileDialog.getOpenFileNames(self, tr("Add Files"), "", INPUT_FILTER)
        self.files.addItems(paths)

    def _remove_selected(self) -> None:
        for item in self.files.selectedItems():
            self.files.takeItem(self.files.row(item))

    def _browse(self) -> None:
        d = QFileDialog.getExistingDirectory(self, tr("Output Folder"), self.folder.text())
        if d:
            self.folder.setText(d)

    def start(self) -> None:
        if self.running():
            return
        jobs: list[tuple[str, str]] = []
        for i in range(self.files.count()):
            src = self.files.item(i).text()
            out = output_path(src, self.folder.text().strip(), self.target())
            if Path(out).exists() and not confirm_replace(self, tr("AT9/AT3 Converter"), out):
                continue
            jobs.append((src, out))
        if not jobs:
            self.status.setText(tr("Add the files to convert first."))
            return
        self._queue = jobs
        self._done = 0
        self._errors = []
        self._total = len(jobs)
        self._set_running(True)
        self._next()

    def _set_running(self, on: bool) -> None:
        for w in (self.convert, self.add, self.remove, self.format, self.folder):
            w.setEnabled(not on)
        if on:
            self.rate.setEnabled(False)
            self.bits.setEnabled(False)
        else:
            self._format_changed()
        self.cancel.setText(tr("Stop") if on else tr("Close"))

    def _next(self) -> None:
        if not self._queue:
            self._job = None
            self._timer.stop()
            self._set_running(False)
            self.progress.setValue(1000)
            done = tr("Done: {n} file(s) converted.").replace("{n}", str(self._done))
            self.status.setText("\n".join([done, *self._errors]))
            return
        src, out = self._queue.pop(0)
        self._current = Path(src).name
        self.status.setText(tr("Converting {name}…").replace("{name}", Path(src).name))

        def started(r: Any, _b: bytes | None) -> None:
            self._job = int(r["job"]) if isinstance(r, dict) and "job" in r else None
            if self._job is None:
                self._next()
            else:
                self._timer.start()

        def failed(msg: str) -> None:
            self._errors.append(f"{Path(src).name}: {msg}")
            self._next()

        self.engine.execute("audio.atracConvert", self.params(src, out), on_reply=started, on_error=failed)

    def _poll(self) -> None:
        job = self._job
        if job is None:
            return

        def got(r: Any, _b: bytes | None) -> None:
            entry = next((j for j in (r or []) if isinstance(j, dict) and j.get("id") == job), None)
            if entry is None:
                return
            share = 1000 // max(self._total, 1)
            self.progress.setValue(self._done * share + int(float(entry.get("progress") or 0.0) * share))
            if not entry.get("finished"):
                return
            self._timer.stop()
            result = entry.get("result") or {}
            if isinstance(result, dict) and result.get("error"):
                self._errors.append(f"{self._current}: {result['error']}")
            else:
                self._done += 1
            self._job = None
            self._next()

        self.engine.execute("jobs.list", {}, on_reply=got, on_error=lambda _m: None)

    def _cancel(self) -> None:
        if not self.running():
            self.reject()
            return
        self._queue = []
        if self._job is not None:
            self.engine.execute("jobs.cancel", {"job": self._job}, on_error=lambda _m: None)
        self.status.setText(tr("Stopping…"))


def open_atrac_converter(win: Any) -> AtracConverterDialog:
    """Shows the converter (one per window)."""
    dlg = getattr(win, "atrac_converter", None)
    if dlg is None:
        dlg = AtracConverterDialog(win.engine, win)
        win.atrac_converter = dlg
    dlg.show()
    dlg.raise_()
    dlg.activateWindow()
    return dlg
