"""Sequence ▸ Sequence Profiles…: the built-in and user sequence profiles (the engine's
``sequence_profiles.rs``), with New…, Edit…, Duplicate, Delete and Apply to Sequence, and the
profile editor: description, frame size, display and pixel aspect (either one recomputes the
other), frame rate as numerator / denominator, interlaced and 360° flags.

Everything goes through the engine's ``sequenceProfiles.*`` commands; the editor works on a
draft, so Cancel leaves the profile exactly as it was, and OK sends one ``sequenceProfiles.save``
(the engine checks the description is unique, names the file and numbers it on a clash). A
refused save keeps the editor open with the engine's message.

Automation ids: ``sequenceProfiles.list``, ``sequenceProfiles.details``, ``sequenceProfiles.new``,
``sequenceProfiles.edit``, ``sequenceProfiles.duplicate``, ``sequenceProfiles.delete``,
``sequenceProfiles.apply``, ``sequenceProfiles.close``; the editor's ``profileEditor.`` +
``description``, ``width``, ``height``, ``darNum``, ``darDen``, ``parNum``, ``parDen``, ``fpsNum``,
``fpsDen``, ``fpsLabel``, ``interlaced``, ``spherical``, ``error``, ``ok``, ``cancel``.
"""

from __future__ import annotations

import math
from collections.abc import Callable
from dataclasses import dataclass, replace
from typing import Any

from PySide6.QtCore import Qt, Signal
from PySide6.QtWidgets import (
    QCheckBox,
    QDialog,
    QFormLayout,
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

from ..i18n import fmt, tr

MAX_FRAME_SIDE = 32_768  # cfa4_project::MAX_FRAME_SIDE
MAX_TERM = 2**31 - 1  # QSpinBox's range; profile terms are u32 in the engine
DESCRIPTION_MAX = 128  # sequence_profiles::DESCRIPTION_MAX
MAX_PIXEL_ASPECT = 8.0  # cfa4_project::MAX_PIXEL_ASPECT
FPS_MAX = 1000.0


def _int(v: Any, default: int, lo: int = 1, hi: int = MAX_TERM) -> int:
    return v if isinstance(v, int) and not isinstance(v, bool) and lo <= v <= hi else default


def _pair(v: Any, default: tuple[int, int]) -> tuple[int, int]:
    if isinstance(v, (list, tuple)) and len(v) == 2:
        return (_int(v[0], default[0]), _int(v[1], default[1]))
    return default


def reduced(a: int, b: int) -> tuple[int, int] | None:
    """``(a, b)`` in lowest terms, or None when a term is not positive or the result is too big."""
    if a <= 0 or b <= 0:
        return None
    g = math.gcd(a, b)
    a, b = a // g, b // g
    return (a, b) if a <= MAX_TERM and b <= MAX_TERM else None


def display_aspect_for(width: int, height: int, par: tuple[int, int]) -> tuple[int, int] | None:
    """The engine's ``display_aspect_for``: ``width × par : height`` in lowest terms."""
    return reduced(width * par[0], height * par[1])


def pixel_aspect_for(width: int, height: int, dar: tuple[int, int]) -> tuple[int, int] | None:
    """The engine's ``pixel_aspect_for``: the pixels that show the frame at ``dar``."""
    return reduced(dar[0] * height, dar[1] * width)


def description_key(text: str) -> str:
    """Descriptions compare trimmed, white space runs as one space, ignoring case."""
    return " ".join(text.split()).casefold()


def fps_label(num: int, den: int) -> str:
    if num <= 0 or den <= 0:
        return "—"
    return str(num // den) if num % den == 0 else f"{num / den:.3f}".rstrip("0").rstrip(".")


@dataclass
class ProfileDraft:
    description: str = ""
    width: int = 1920
    height: int = 1080
    dar: tuple[int, int] = (16, 9)
    par: tuple[int, int] = (1, 1)
    fps: tuple[int, int] = (25, 1)
    interlaced: bool = False
    spherical: bool = False
    builtin: bool = False


def draft_from(p: Any) -> ProfileDraft:
    """A draft from a ``sequenceProfiles.list`` entry; damaged values read as the defaults."""
    p = p if isinstance(p, dict) else {}
    d = ProfileDraft(
        description=str(p.get("description") or ""),
        width=_int(p.get("width"), 1920, 1, MAX_FRAME_SIDE),
        height=_int(p.get("height"), 1080, 1, MAX_FRAME_SIDE),
        par=_pair(p.get("pixelAspect"), (1, 1)),
        fps=_pair(p.get("fps"), (25, 1)),
        interlaced=p.get("interlaced") is True,
        spherical=p.get("spherical") is True,
        builtin=p.get("builtin") is True,
    )
    d.dar = display_aspect_for(d.width, d.height, d.par) or _pair(p.get("displayAspect"), (16, 9))
    return d


def profile_params(d: ProfileDraft) -> dict[str, Any]:
    """The draft as ``sequenceProfiles.save``'s ``profile``."""
    return {
        "description": " ".join(d.description.split()),
        "width": d.width,
        "height": d.height,
        "displayAspect": list(d.dar),
        "pixelAspect": list(d.par),
        "fps": list(d.fps),
        "interlaced": d.interlaced,
        "spherical": d.spherical,
    }


def problem(d: ProfileDraft, profiles: list[dict[str, Any]], original: str | None) -> str:
    """Why the engine would refuse the draft (the same checks, shown before OK), or ""."""
    desc = " ".join(d.description.split())
    if not desc:
        return tr("Enter a description.")
    if len(desc) > DESCRIPTION_MAX:
        return fmt("The description must be at most {n} characters.", n=DESCRIPTION_MAX)
    own = description_key(original) if original else None
    for p in profiles:
        key = description_key(str(p.get("description") or ""))
        # the edited user profile may keep its own description; a built-in one may not
        if key == description_key(desc) and not (key == own and p.get("builtin") is False):
            return fmt("A profile is already called “{name}”.", name=desc)
    if d.width * d.height > 268_435_456:
        return tr("The frame is too large.")
    r = d.par[0] / d.par[1] if d.par[1] > 0 else 0
    if not 1 / MAX_PIXEL_ASPECT <= r <= MAX_PIXEL_ASPECT:
        return tr("The pixel aspect ratio must be at most 8:1 either way.")
    if display_aspect_for(d.width, d.height, d.par) != reduced(*d.dar):
        return tr("The display aspect ratio does not match the frame size and pixel aspect ratio.")
    if d.fps[1] <= 0 or d.fps[0] / d.fps[1] > FPS_MAX:
        return tr("The frame rate must be at most 1000 fps.")
    return ""


class ProfileEditor(QDialog):
    """Edits ``draft`` (a copy; the caller's profile is untouched until the engine saves it).
    ``original`` is the edited profile's description (None: a new profile)."""

    saved = Signal(dict)  # the engine's saved profile

    def __init__(
        self,
        parent: QWidget | None,
        engine: Any,
        draft: ProfileDraft,
        profiles: list[dict[str, Any]],
        original: str | None,
    ) -> None:
        super().__init__(parent)
        self.setObjectName("profileEditor")
        self.setWindowTitle(tr("Edit Sequence Profile") if original else tr("New Sequence Profile"))
        self.engine = engine
        self.start = replace(draft)
        self.d = replace(draft)
        self.profiles = profiles
        self.original = original
        self._syncing = False
        lay = QVBoxLayout(self)
        form = QFormLayout()
        self.description = QLineEdit(self.d.description, self)
        self.description.setObjectName("profileEditor.description")
        self.description.setMaxLength(DESCRIPTION_MAX)
        self.description.textEdited.connect(self._described)
        form.addRow(tr("Description:"), self.description)
        self.width_box = self._spin("width", 1, MAX_FRAME_SIDE)
        self.height_box = self._spin("height", 1, MAX_FRAME_SIDE)
        form.addRow(tr("Frame Size:"), self._row(self.width_box, "×", self.height_box))
        self.dar_num, self.dar_den = self._spin("darNum"), self._spin("darDen")
        form.addRow(tr("Display Aspect Ratio:"), self._row(self.dar_num, ":", self.dar_den))
        self.par_num, self.par_den = self._spin("parNum"), self._spin("parDen")
        form.addRow(tr("Pixel Aspect Ratio:"), self._row(self.par_num, ":", self.par_den))
        self.fps_num, self.fps_den = self._spin("fpsNum"), self._spin("fpsDen")
        self.fps_text = QLabel(self)
        self.fps_text.setObjectName("profileEditor.fpsLabel")
        form.addRow(tr("Frame Rate:"), self._row(self.fps_num, "/", self.fps_den, self.fps_text))
        self.interlaced = QCheckBox(tr("Interlaced"), self)
        self.interlaced.setObjectName("profileEditor.interlaced")
        self.interlaced.setToolTip(tr("CFA4 sequences are progressive: applying the profile keeps its size and rate"))
        self.interlaced.toggled.connect(lambda on: self._set("interlaced", on))
        self.spherical = QCheckBox(tr("360° spherical video"), self)
        self.spherical.setObjectName("profileEditor.spherical")
        self.spherical.toggled.connect(lambda on: self._set("spherical", on))
        form.addRow(self.interlaced)
        form.addRow(self.spherical)
        lay.addLayout(form)
        if draft.builtin:
            note = QLabel(tr("Built-in profiles are kept: this is saved as a new profile."), self)
            note.setWordWrap(True)
            lay.addWidget(note)
        self.error = QLabel(self)
        self.error.setObjectName("profileEditor.error")
        self.error.setWordWrap(True)
        self.error.setStyleSheet("color: #e06c6c")
        lay.addWidget(self.error)
        buttons = QHBoxLayout()
        buttons.addStretch(1)
        self.cancel = QPushButton(tr("Cancel"), self)
        self.cancel.setObjectName("profileEditor.cancel")
        self.cancel.clicked.connect(self.reject)
        self.ok = QPushButton(tr("OK"), self)
        self.ok.setObjectName("profileEditor.ok")
        self.ok.setDefault(True)
        self.ok.clicked.connect(self.apply)
        buttons.addWidget(self.cancel)
        buttons.addWidget(self.ok)
        lay.addLayout(buttons)
        self.width_box.valueChanged.connect(lambda v: self._size("width", v))
        self.height_box.valueChanged.connect(lambda v: self._size("height", v))
        self.par_num.valueChanged.connect(lambda _v: self._par())
        self.par_den.valueChanged.connect(lambda _v: self._par())
        self.dar_num.valueChanged.connect(lambda _v: self._dar())
        self.dar_den.valueChanged.connect(lambda _v: self._dar())
        self.fps_num.valueChanged.connect(lambda _v: self._fps())
        self.fps_den.valueChanged.connect(lambda _v: self._fps())
        self._sync()

    def _spin(self, name: str, lo: int = 1, hi: int = MAX_TERM) -> QSpinBox:
        s = QSpinBox(self)
        s.setObjectName(f"profileEditor.{name}")
        s.setRange(lo, hi)
        s.setKeyboardTracking(False)
        return s

    def _row(self, *parts: QWidget | str) -> QHBoxLayout:
        row = QHBoxLayout()
        for p in parts:
            row.addWidget(QLabel(p, self) if isinstance(p, str) else p)
        row.addStretch(1)
        return row

    # ----- edits: size and pixel aspect set the display aspect; the display aspect sets the pixels
    def _described(self, text: str) -> None:
        self.d.description = text
        self._check()

    def _set(self, attr: str, value: Any) -> None:
        if not self._syncing:
            setattr(self.d, attr, value)
            self._check()

    def _size(self, attr: str, value: int) -> None:
        if self._syncing:
            return
        setattr(self.d, attr, value)
        self.d.dar = display_aspect_for(self.d.width, self.d.height, self.d.par) or self.d.dar
        self._sync()

    def _par(self) -> None:
        if self._syncing:
            return
        self.d.par = (self.par_num.value(), self.par_den.value())
        self.d.dar = display_aspect_for(self.d.width, self.d.height, self.d.par) or self.d.dar
        self._sync()

    def _dar(self) -> None:
        if self._syncing:
            return
        self.d.dar = (self.dar_num.value(), self.dar_den.value())
        par = pixel_aspect_for(self.d.width, self.d.height, self.d.dar)
        if par is not None:
            self.d.par = par
        self._sync(keep_dar=True)

    def _fps(self) -> None:
        if not self._syncing:
            self.d.fps = (self.fps_num.value(), self.fps_den.value())
            self._sync()

    def _sync(self, keep_dar: bool = False) -> None:
        """Show the draft; the display aspect in lowest terms unless it is being typed."""
        self._syncing = True
        d = self.d
        values = [
            (self.width_box, d.width),
            (self.height_box, d.height),
            (self.par_num, d.par[0]),
            (self.par_den, d.par[1]),
        ]
        values += [(self.fps_num, d.fps[0]), (self.fps_den, d.fps[1])]
        if not keep_dar:
            values += [(self.dar_num, d.dar[0]), (self.dar_den, d.dar[1])]
        for box, v in values:
            box.setValue(max(box.minimum(), min(box.maximum(), v)))
        self.interlaced.setChecked(d.interlaced)
        self.spherical.setChecked(d.spherical)
        self.fps_text.setText(fmt("{rate} fps", rate=fps_label(*d.fps)))
        self._syncing = False
        self._check()

    def _check(self) -> None:
        why = problem(self.d, self.profiles, self.original)
        self.error.setText(why)
        self.ok.setEnabled(not why)

    # ----- OK / Cancel
    def apply(self) -> None:
        if problem(self.d, self.profiles, self.original):
            return
        params: dict[str, Any] = {"profile": profile_params(self.d)}
        if self.original:
            params["original"] = self.original

        def done(r: Any, _b: Any = None) -> None:
            self.saved.emit(r if isinstance(r, dict) else {})
            self.accept()

        def failed(message: str) -> None:
            self.error.setText(str(message))
            self.ok.setEnabled(True)

        self.ok.setEnabled(False)
        self.engine.execute("sequenceProfiles.save", params, on_reply=done, on_error=failed)

    def reject(self) -> None:
        # Cancel: nothing was sent; the draft goes back to the profile as it was
        self.d = replace(self.start)
        super().reject()


class SequenceProfiles(QDialog):
    """The profile list. ``status(text)`` reports results (the status bar); ``changed()`` runs
    after Apply to Sequence changed the active sequence."""

    def __init__(
        self,
        parent: QWidget | None,
        engine: Any,
        status: Callable[[str], None],
        changed: Callable[[], None],
        has_sequence: bool,
    ) -> None:
        super().__init__(parent)
        self.setObjectName("sequenceProfiles")
        self.setWindowTitle(tr("Sequence Profiles"))
        self.resize(560, 460)
        self.engine = engine
        self.status = status
        self.changed = changed
        self.profiles: list[dict[str, Any]] = []
        self.editor: ProfileEditor | None = None
        lay = QHBoxLayout(self)
        left = QVBoxLayout()
        self.list = QListWidget(self)
        self.list.setObjectName("sequenceProfiles.list")
        self.list.currentRowChanged.connect(lambda _r: self._show())
        self.list.itemDoubleClicked.connect(lambda _i: self.edit())
        left.addWidget(self.list, 1)
        self.details = QLabel(self)
        self.details.setObjectName("sequenceProfiles.details")
        self.details.setWordWrap(True)
        left.addWidget(self.details)
        lay.addLayout(left, 1)
        side = QVBoxLayout()
        self.buttons: dict[str, QPushButton] = {}
        for key, label, slot in (
            ("new", tr("New…"), self.new),
            ("edit", tr("Edit…"), self.edit),
            ("duplicate", tr("Duplicate"), self.duplicate),
            ("delete", tr("Delete"), self.delete),
            ("apply", tr("Apply to Sequence"), self.apply),
        ):
            b = QPushButton(label, self)
            b.setObjectName(f"sequenceProfiles.{key}")
            b.clicked.connect(slot)
            side.addWidget(b)
            self.buttons[key] = b
        self.has_sequence = has_sequence
        side.addStretch(1)
        close = QPushButton(tr("Close"), self)
        close.setObjectName("sequenceProfiles.close")
        close.clicked.connect(self.accept)
        side.addWidget(close)
        lay.addLayout(side)
        self.reload()

    def reload(self, select: str = "") -> None:
        def got(r: Any, _b: Any = None) -> None:
            self.fill(r, select)

        self.engine.execute("sequenceProfiles.list", {}, on_reply=got, on_error=lambda m: self.status(str(m)))

    def fill(self, listing: Any, select: str = "") -> None:
        raw = listing.get("profiles") if isinstance(listing, dict) else None
        self.profiles = [p for p in raw if isinstance(p, dict)] if isinstance(raw, list) else []
        want = description_key(select or self.current_description())
        self.list.blockSignals(True)
        self.list.clear()
        row = 0
        for i, p in enumerate(self.profiles):
            desc = str(p.get("description") or "")
            item = QListWidgetItem(desc if p.get("builtin") is not False else fmt("{name} (user)", name=desc))
            item.setData(Qt.ItemDataRole.UserRole, desc)
            self.list.addItem(item)
            if want and description_key(desc) == want:
                row = i
        self.list.blockSignals(False)
        if self.profiles:
            self.list.setCurrentRow(row)
        self._show()

    def current(self) -> dict[str, Any] | None:
        r = self.list.currentRow()
        return self.profiles[r] if 0 <= r < len(self.profiles) else None

    def current_description(self) -> str:
        p = self.current()
        return str(p.get("description") or "") if p else ""

    def _show(self) -> None:
        p = self.current()
        user = bool(p) and p.get("builtin") is False
        self.buttons["edit"].setEnabled(bool(p))
        self.buttons["duplicate"].setEnabled(bool(p))
        self.buttons["delete"].setEnabled(user)
        self.buttons["apply"].setEnabled(bool(p) and self.has_sequence)
        if not p:
            self.details.setText("")
            return
        d = draft_from(p)
        lines = [
            fmt(
                "{w} × {h}, display {dn}:{dd}, pixels {pn}:{pd}",
                w=d.width,
                h=d.height,
                dn=d.dar[0],
                dd=d.dar[1],
                pn=d.par[0],
                pd=d.par[1],
            ),
            fmt("{rate} fps", rate=fps_label(*d.fps))
            + (", " + tr("Interlaced") if d.interlaced else "")
            + (", " + tr("360° spherical video") if d.spherical else ""),
        ]
        if user and p.get("file"):
            lines.append(str(p.get("file")))
        self.details.setText("\n".join(lines))

    def _open_editor(self, draft: ProfileDraft, original: str | None) -> None:
        editor = ProfileEditor(self, self.engine, draft, self.profiles, original)
        editor.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        editor.saved.connect(lambda r: self._saved(r))
        self.editor = editor
        editor.open()

    def _saved(self, r: dict[str, Any]) -> None:
        p = r.get("profile") if isinstance(r.get("profile"), dict) else {}
        desc = str(p.get("description") or "")
        self.status(fmt("Saved sequence profile “{name}”", name=desc))
        self.fill(r.get("profiles"), desc)

    def new(self) -> None:
        cur = self.current()
        base = replace(draft_from(cur), builtin=False) if cur else ProfileDraft()
        self._open_editor(replace(base, description=""), None)

    def edit(self) -> None:
        cur = self.current()
        if cur:
            self._open_editor(draft_from(cur), self.current_description())

    def duplicate(self) -> None:
        desc = self.current_description()
        if not desc:
            return

        def done(r: Any, _b: Any = None) -> None:
            if isinstance(r, dict):
                self._saved(r)

        self.engine.execute(
            "sequenceProfiles.duplicate", {"description": desc}, on_reply=done, on_error=lambda m: self.status(str(m))
        )

    def delete(self) -> None:
        p = self.current()
        if not p or p.get("builtin") is not False:
            return
        desc = str(p.get("description") or "")

        def done(r: Any, _b: Any = None) -> None:
            self.status(fmt("Deleted sequence profile “{name}”", name=desc))
            self.fill(r)

        self.engine.execute(
            "sequenceProfiles.delete", {"description": desc}, on_reply=done, on_error=lambda m: self.status(str(m))
        )

    def apply(self) -> None:
        desc = self.current_description()
        if not desc or not self.has_sequence:
            return

        def done(r: Any, _b: Any = None) -> None:
            skipped = r.get("notApplied") if isinstance(r, dict) and isinstance(r.get("notApplied"), list) else []
            names = {"interlaced": tr("Interlaced"), "spherical": tr("360° spherical video")}
            text = fmt("Applied sequence profile “{name}”", name=desc)
            if skipped:
                text += " — " + fmt(
                    "not supported by CFA4 sequences yet: {what}",
                    what=", ".join(names.get(str(s), str(s)) for s in skipped),
                )
            self.status(text)
            self.changed()

        self.engine.execute(
            "sequenceProfiles.apply", {"description": desc}, on_reply=done, on_error=lambda m: self.status(str(m))
        )


__all__ = [
    "ProfileDraft",
    "ProfileEditor",
    "SequenceProfiles",
    "display_aspect_for",
    "draft_from",
    "pixel_aspect_for",
    "problem",
    "profile_params",
]
