"""CFA4's own identity in the Qt GUI: the CFA4 logo, never an inherited FilmCraft/ArtCraft mark.

The logo is the original CFA4 artwork in ``assets/cfa4`` (see its ``.attribution`` sidecar).
"""

from __future__ import annotations

from pathlib import Path

from PySide6.QtGui import QIcon, QPixmap

ROOT = Path(__file__).resolve().parents[2]
LOGO_SVG = ROOT / "assets" / "cfa4" / "hicolor" / "scalable" / "apps" / "io.github.ubuntuokos.cfa4.svg"
# the freedesktop icon name an installed package provides (same file, installed by packaging)
ICON_NAME = "io.github.ubuntuokos.cfa4"


def app_icon() -> QIcon:
    """The CFA4 logo: the checkout's SVG, else the installed icon theme entry, else empty."""
    if LOGO_SVG.is_file():
        icon = QIcon(str(LOGO_SVG))
        if not icon.isNull():
            return icon
    return QIcon.fromTheme(ICON_NAME)


def logo_pixmap(size: int) -> QPixmap:
    return app_icon().pixmap(size, size)
