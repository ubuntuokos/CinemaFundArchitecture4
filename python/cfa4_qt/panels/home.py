"""Import mode (egui's ``import_mode``, the header's Home and Import): large cards that import
media, demo footage, the demo project or Bars and Tone, the recent projects (newest first, the
file name with its folder beside it; a click opens one) and the CFA4 project links. It covers the
editing panels until Edit (or a card) is chosen.

Automation ids as egui's: ``import.<command>`` for the cards, ``import.recent.<i>`` and
``import.link.<github|reportIssue>``; the page itself is ``import.page``.
"""

from __future__ import annotations

from pathlib import Path

from PySide6.QtCore import QSize, Qt, QUrl, Signal
from PySide6.QtGui import QDesktopServices, QFont
from PySide6.QtWidgets import (
    QGridLayout,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QScrollArea,
    QSizePolicy,
    QStyle,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from ..i18n import N_, tr

GITHUB = "https://github.com/ubuntuokos/CinemaFundArchitecture4"
ISSUES = "https://github.com/ubuntuokos/CinemaFundArchitecture4/issues"

# (title, command, subtitle, icon) as egui's cards
CARDS = (
    (N_("Browse files…"), "file.import", N_("Movies, audio and stills from disk"), QStyle.StandardPixmap.SP_DirOpenIcon),
    (N_("Demo footage"), "file.importDemoFootage", N_("Six procedural 1080p clips with sound"), QStyle.StandardPixmap.SP_MediaPlay),
    (N_("Demo project"), "file.openDemoProject", N_("A cut sequence with transitions, effects and music"), QStyle.StandardPixmap.SP_FileDialogContentsView),
    (N_("Bars and Tone"), "file.newBarsAndTone", N_("SMPTE HD bars with 1 kHz reference tone"), QStyle.StandardPixmap.SP_FileDialogListView),
)
# (id, label, url) as egui's ``links::ALL``
LINKS = (("github", N_("CFA4 on GitHub"), GITHUB), ("reportIssue", N_("Report a CFA4 issue…"), ISSUES))
RECENT_ROWS = 6


def recent_name(path: str) -> str:
    """A recent project as listed: its file name."""
    return Path(path).name or path


def _heading(text: str, size: float, parent: QWidget) -> QLabel:
    label = QLabel(text, parent)
    f = QFont(label.font())
    f.setPointSizeF(size)
    f.setWeight(QFont.Weight.DemiBold)
    label.setFont(f)
    return label


class HomePage(QScrollArea):
    """The Import page; ``command`` runs a card's command, ``open_recent`` a recent project."""

    command = Signal(str)
    open_recent = Signal(str)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("import.page")
        self.setWidgetResizable(True)
        self.setFrameShape(QScrollArea.Shape.NoFrame)
        body = QWidget(self)
        body.setObjectName("import.body")
        lay = QVBoxLayout(body)
        lay.setContentsMargins(24, 16, 24, 24)
        lay.setSpacing(8)
        lay.addWidget(_heading(tr("Import"), 18, body))
        sub = QLabel(tr("Add media to your project. Drop files anywhere in the window, or choose a source below."), body)
        sub.setEnabled(False)  # the theme's dim text
        sub.setWordWrap(True)
        lay.addWidget(sub)
        cards = QGridLayout()
        cards.setSpacing(16)
        self.cards: dict[str, QToolButton] = {}
        for i, (title, cmd, subtitle, icon) in enumerate(CARDS):
            b = QToolButton(body)
            b.setObjectName(f"import.{cmd}")
            b.setToolButtonStyle(Qt.ToolButtonStyle.ToolButtonTextUnderIcon)
            b.setIcon(self.style().standardIcon(icon))
            b.setIconSize(QSize(48, 48))
            b.setText(f"{tr(title)}\n{tr(subtitle)}")
            b.setToolTip(tr(subtitle))
            b.setMinimumSize(220, 130)
            b.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Fixed)
            # egui's card: a filled rounded box, lighter under the pointer
            b.setStyleSheet("QToolButton { background: palette(button); border: none; border-radius: 8px; padding: 12px; }"
                            " QToolButton:hover { background: palette(midlight); }")
            b.clicked.connect(lambda _c=False, c=cmd: self.command.emit(c))
            cards.addWidget(b, 0, i)
            self.cards[cmd] = b
        lay.addSpacing(12)
        lay.addLayout(cards)
        lay.addSpacing(16)
        self.recent_heading = _heading(tr("Recent"), 12, body)
        lay.addWidget(self.recent_heading)
        self.recent_box = QVBoxLayout()
        self.recent_box.setSpacing(0)
        lay.addLayout(self.recent_box)
        self.recent: list[QPushButton] = []
        self.recent_paths: list[str] = []
        lay.addSpacing(16)
        lay.addWidget(_heading(tr("CFA4 project links"), 12, body))
        links = QHBoxLayout()
        links.setSpacing(12)
        self.links: dict[str, QPushButton] = {}
        for i, (lid, label, url) in enumerate(LINKS):
            b = QPushButton(tr(label), body)
            b.setObjectName(f"import.link.{lid}")
            b.setToolTip(url)
            b.setDefault(i == 0)  # the primary (accent) button, as egui's first link
            b.setAutoDefault(False)
            b.clicked.connect(lambda _c=False, u=url: QDesktopServices.openUrl(QUrl(u)))
            links.addWidget(b)
            self.links[lid] = b
        links.addStretch(1)
        lay.addLayout(links)
        lay.addStretch(1)
        self.setWidget(body)
        self.recent_heading.hide()  # until there is a recent project

    def set_recent(self, paths: list[str]) -> None:
        """The recent projects (General ▸ ``recentProjects``, newest first); the section is left
        out while there are none."""
        shown = [p for p in paths if isinstance(p, str) and p][:RECENT_ROWS]
        if shown == self.recent_paths and len(self.recent) == len(shown):
            return
        for b in self.recent:
            self.recent_box.removeWidget(b)
            b.deleteLater()
        self.recent = []
        self.recent_paths = shown
        body = self.widget() or self
        for i, path in enumerate(shown):
            folder = str(Path(path).parent)
            b = QPushButton(f"{recent_name(path)}    {folder}", body)
            b.setObjectName(f"import.recent.{i}")
            b.setFlat(True)
            b.setToolTip(path)
            b.setStyleSheet("QPushButton { text-align: left; padding: 4px 8px; border: none; }"
                            " QPushButton:hover { background: palette(midlight); }")
            b.setIcon(self.style().standardIcon(QStyle.StandardPixmap.SP_FileIcon))
            b.clicked.connect(lambda _c=False, p=path: self.open_recent.emit(p))
            self.recent_box.addWidget(b)
            self.recent.append(b)
        self.recent_heading.setVisible(bool(shown))
