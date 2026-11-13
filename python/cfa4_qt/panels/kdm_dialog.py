"""File ▸ Load KDM…: the content keys of an encrypted DCP (#933, #1029).

The user picks a KDM (SMPTE ST 430-1 XML) and the PEM private key of its recipient; the engine's
``dcp.loadKdm`` decrypts the content keys into memory, and the encrypted track files play from
then on. The engine checks the KDM's signature and certificate chain (ST 430-1 / 430-2 / 430-3). An unsigned
KDM, or one changed after signing, is always refused; one whose content is intact but whose
certificate chain does not check out loads only when "Load even if the certificate chain is not
verified" is ticked (``allowUnverified``, #1028 / #1075, owner decision 2026-11-13). The dialog shows what the KDM holds (title, key types, validity) and how the
signature check went, never a key. Forget DCP Keys (``dcp.clearKeys``) drops them again. Keys are
never written anywhere.

Automation ids: ``kdm.kdmPath``, ``kdm.kdmBrowse``, ``kdm.keyPath``, ``kdm.keyBrowse``,
``kdm.allowUnverified``, ``kdm.load``, ``kdm.forget``, ``kdm.result``, ``kdm.close``.
"""

from __future__ import annotations

import os
from collections.abc import Callable
from typing import Any

from PySide6.QtWidgets import (
    QCheckBox,
    QDialog,
    QDialogButtonBox,
    QFileDialog,
    QFormLayout,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPlainTextEdit,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from ..i18n import fmt, tr


def describe(r: Any) -> str:
    """The engine's ``dcp.loadKdm`` reply as text: title, one line per key (never the key), the
    composition, then how the signature check went (the signer's chain, or the problem)."""
    if not isinstance(r, dict):
        return ""
    title = str(r.get("title") or r.get("annotation") or "KDM")
    keys = r.get("keys") if isinstance(r.get("keys"), list) else []
    lines = [fmt("{title}: {count} keys loaded", title=title, count=len(keys))]
    for k in keys:
        if isinstance(k, dict):
            lines.append(f"{k.get('type', '?')}  {k.get('keyId', '')}  {k.get('notValidBefore', '')} – {k.get('notValidAfter', '')}")
    if isinstance(r.get("composition"), str):
        lines.append(f"CPL {r['composition']}")
    sig = r.get("signature")
    if isinstance(sig, dict):
        if sig.get("valid") is True:
            chain = sig.get("chain") if isinstance(sig.get("chain"), list) else []
            lines.append(fmt("Signature verified: {chain}", chain=" ← ".join(str(c) for c in chain)))
        else:
            lines.append(fmt("Signature not verified: {problem}", problem=str(sig.get("problem") or "?")))
    return "\n".join(lines)


class KdmDialog(QDialog):
    """Load a KDM and its private key; forget the loaded keys."""

    def __init__(self, engine: Any, parent: QWidget | None = None, changed: Callable[[], None] | None = None):
        super().__init__(parent)
        self.engine = engine
        self.changed = changed or (lambda: None)
        self.setObjectName("kdm")
        self.setWindowTitle(tr("Load KDM"))
        self.kdm_path = QLineEdit(self)
        self.kdm_path.setObjectName("kdm.kdmPath")
        self.key_path = QLineEdit(self)
        self.key_path.setObjectName("kdm.keyPath")
        form = QFormLayout()
        for edit, label, name, filt in (
            (self.kdm_path, tr("KDM file"), "kdm.kdmBrowse", "KDM (*.xml)"),
            (self.key_path, tr("Private key (PEM)"), "kdm.keyBrowse", "PEM (*.pem *.key)"),
        ):
            browse = QPushButton(tr("Browse…"), self)
            browse.setObjectName(name)
            browse.clicked.connect(lambda _c=False, e=edit, f=filt, t=label: self._browse(e, f, t))
            row = QHBoxLayout()
            row.addWidget(edit, 1)
            row.addWidget(browse)
            form.addRow(label, row)
        self.allow_unverified = QCheckBox(tr("Load even if the certificate chain is not verified"), self)
        self.allow_unverified.setObjectName("kdm.allowUnverified")
        form.addRow("", self.allow_unverified)
        note = QLabel(tr("The keys stay in memory only, until Forget DCP Keys or the end of the session."), self)
        note.setWordWrap(True)
        self.result = QPlainTextEdit(self)
        self.result.setObjectName("kdm.result")
        self.result.setReadOnly(True)
        buttons = QDialogButtonBox(self)
        self.load_button = buttons.addButton(tr("Load KDM"), QDialogButtonBox.ButtonRole.AcceptRole)
        self.load_button.setObjectName("kdm.load")
        self.forget_button = buttons.addButton(tr("Forget DCP Keys"), QDialogButtonBox.ButtonRole.ResetRole)
        self.forget_button.setObjectName("kdm.forget")
        close = buttons.addButton(QDialogButtonBox.StandardButton.Close)
        close.setObjectName("kdm.close")
        close.setText(tr("Close"))
        self.load_button.clicked.connect(self.load)
        self.forget_button.clicked.connect(self.forget)
        close.clicked.connect(self.reject)
        lay = QVBoxLayout(self)
        lay.addLayout(form)
        lay.addWidget(note)
        lay.addWidget(self.result, 1)
        lay.addWidget(buttons)
        self.resize(640, 360)

    def _browse(self, edit: QLineEdit, filt: str, title: str) -> None:
        path, _f = QFileDialog.getOpenFileName(self, title, os.path.dirname(edit.text()), f"{filt};;{tr('All files')} (*)")
        if path:
            edit.setText(path)

    def load(self) -> None:
        kdm, key = self.kdm_path.text().strip(), self.key_path.text().strip()
        if not kdm or not key:
            self.result.setPlainText(tr("Choose the KDM file and the private key."))
            return
        params: dict[str, Any] = {"kdm": os.path.expanduser(kdm), "privateKey": os.path.expanduser(key)}
        if self.allow_unverified.isChecked():
            params["allowUnverified"] = True
        self.load_button.setEnabled(False)

        def got(r: Any, _b: bytes | None) -> None:
            self.load_button.setEnabled(True)
            self.result.setPlainText(describe(r))
            self.changed()

        def failed(m: str) -> None:
            self.load_button.setEnabled(True)
            self.result.setPlainText(str(m))

        self.engine.execute("dcp.loadKdm", params, on_reply=got, on_error=failed)

    def forget(self) -> None:
        def got(_r: Any, _b: bytes | None) -> None:
            self.result.setPlainText(tr("DCP keys forgotten"))
            self.changed()

        self.engine.execute("dcp.clearKeys", {}, on_reply=got, on_error=lambda m: self.result.setPlainText(str(m)))
