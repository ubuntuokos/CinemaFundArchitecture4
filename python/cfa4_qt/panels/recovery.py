"""The crash-recovery prompt as egui's (``crates/ui-egui/src/panels/file_dialogs.rs``
``show_recovery``): every session with unsaved changes (``file.recoveryList``), one chosen by its
radio button when there are several, and Discard (that one; the prompt stays for the others),
Not Now (asked again at the next start) or Recover. Automation ids: ``recovery.item.<n>``,
``recovery.discard``, ``recovery.later``, ``recovery.recover``."""

from __future__ import annotations

import datetime
from collections.abc import Callable
from typing import Any

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QButtonGroup,
    QDialog,
    QFrame,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QRadioButton,
    QVBoxLayout,
    QWidget,
)

from .. import theme
from ..i18n import fmt, tr


def saved_when(cand: dict[str, Any]) -> str:
    """The engine's local ``savedAt``, else ``savedUnix`` in local time, else "an unknown time"."""
    when = cand.get("savedAt")
    if isinstance(when, str) and when:
        return when
    try:
        return datetime.datetime.fromtimestamp(int(cand.get("savedUnix")), tz=datetime.UTC).astimezone().strftime("%Y-%m-%d %H:%M:%S")
    except (TypeError, ValueError, OverflowError, OSError):
        return tr("an unknown time")


def recovery_lines(cand: dict[str, Any]) -> tuple[str, str, str]:
    """egui's three lines: why there are unsaved changes, the question, and where the project is."""
    name = str(cand.get("projectName") or tr("Untitled"))
    if cand.get("cleanExit") is True:
        why = fmt("CFA4 was closed while “{name}” had unsaved changes.", name=name)
    else:
        why = fmt("CFA4 quit unexpectedly while “{name}” had unsaved changes.", name=name)
    question = fmt("Recover unsaved changes from {when}?", when=saved_when(cand))
    path = cand.get("projectPath")
    where = str(path) if isinstance(path, str) and path else tr("The project had not been saved yet.")
    return why, question, where


class RecoveryDialog(QDialog):
    """Opened without blocking; ``run(command, params, then)`` sends Recover / Discard to the
    engine, ``then`` getting the reply."""

    def __init__(self, candidates: list[dict[str, Any]], run: Callable[[str, dict[str, Any], Callable[[Any], None]], None], parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("recovery")
        self.setWindowTitle(tr("Recover Unsaved Changes"))
        self.setMinimumWidth(520)
        self.run = run
        self.candidates: list[dict[str, Any]] = []
        self.choice = 0
        title = QLabel(tr("Recover Unsaved Changes"), self)
        f = title.font()
        f.setPointSizeF(f.pointSizeF() * 1.35)
        f.setBold(True)
        title.setFont(f)
        line = QFrame(self)
        line.setFrameShape(QFrame.Shape.HLine)
        self.why = QLabel(self)
        self.why.setObjectName("recovery.why")
        self.why.setWordWrap(True)
        self.question = QLabel(self)
        self.question.setObjectName("recovery.question")
        self.question.setWordWrap(True)
        self.where = QLabel(self)
        self.where.setObjectName("recovery.where")
        self.where.setWordWrap(True)
        self.where.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        self.count = QLabel(self)
        self.count.setObjectName("recovery.count")
        self.items = QWidget(self)
        self.items_layout = QVBoxLayout(self.items)
        self.items_layout.setContentsMargins(0, 0, 0, 0)
        self.group = QButtonGroup(self)
        self.group.idClicked.connect(self._choose)
        self.discard = QPushButton(tr("Discard"), self)
        self.discard.setObjectName("recovery.discard")
        self.discard.setAutoDefault(False)
        self.later = QPushButton(tr("Not Now"), self)
        self.later.setObjectName("recovery.later")
        self.later.setAutoDefault(False)
        self.recover = QPushButton(tr("Recover"), self)
        self.recover.setObjectName("recovery.recover")
        self.recover.setDefault(True)
        self.discard.clicked.connect(self._discard)
        self.later.clicked.connect(self.reject)
        self.recover.clicked.connect(self._recover)
        buttons = QHBoxLayout()
        buttons.addWidget(self.discard)
        buttons.addStretch(1)
        buttons.addWidget(self.later)
        buttons.addWidget(self.recover)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(28, 24, 28, 20)
        layout.addWidget(title)
        layout.addWidget(line)
        layout.addWidget(self.why)
        layout.addWidget(self.question)
        layout.addWidget(self.where)
        layout.addSpacing(8)
        layout.addWidget(self.count)
        layout.addWidget(self.items)
        layout.addSpacing(18)
        layout.addLayout(buttons)
        self.set_candidates(candidates)

    def set_candidates(self, candidates: Any) -> None:
        """The sessions to offer (``file.recoveryList``); none closes the prompt."""
        self.candidates = [c for c in candidates if isinstance(c, dict)] if isinstance(candidates, list) else []
        if not self.candidates:
            self.accept()
            return
        self.choice = min(self.choice, len(self.candidates) - 1)
        for b in self.group.buttons():
            self.group.removeButton(b)
            b.deleteLater()
        many = len(self.candidates) > 1
        self.count.setVisible(many)
        self.items.setVisible(many)
        self.count.setText(fmt("{n} sessions have unsaved changes:", n=len(self.candidates)))
        if many:
            for i, c in enumerate(self.candidates):
                r = QRadioButton(f"{c.get('projectName') or tr('Untitled')} — {saved_when(c)}", self.items)
                r.setObjectName(f"recovery.item.{i}")
                self.group.addButton(r, i)
                self.items_layout.addWidget(r)
        self._show()

    def _choose(self, i: int) -> None:
        self.choice = i
        self._show()

    def _show(self) -> None:
        why, question, where = recovery_lines(self.candidates[self.choice])
        self.why.setText(why)
        self.question.setText(question)
        self.where.setText(where)
        self.where.setStyleSheet(f"color: {theme.current().text_dim};")
        self.count.setStyleSheet(f"color: {theme.current().text_dim};")
        b = self.group.button(self.choice)
        if b is not None:
            b.setChecked(True)

    def chosen(self) -> dict[str, Any]:
        return self.candidates[self.choice]

    def _recover(self) -> None:
        self.run("file.recover", {"id": self.chosen().get("id")}, lambda _r: None)
        self.accept()

    def _discard(self) -> None:
        """That session's journal goes; the prompt stays for the others (egui's)."""
        gone = self.chosen().get("id")
        self.choice = 0
        self.run("file.discardRecovery", {"id": gone}, lambda _r: self.run("file.recoveryList", {}, self.set_candidates))
