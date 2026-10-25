"""CFA4's own identity in the Qt GUI: the CFA4 logo, never an inherited FilmCraft/ArtCraft mark.

The logo is the project owner's official logo ``assets/cfa4/cfa4-official-logo.png`` (see the ``.attribution``
sidecars); the app icons in ``assets/cfa4/hicolor`` are its emblem, made by
``scripts/dev/cfa4_icons.py``.
"""

from __future__ import annotations

from pathlib import Path

from PySide6.QtCore import QSize, Qt
from PySide6.QtGui import QIcon, QPixmap

ROOT = Path(__file__).resolve().parents[2]
ASSETS = ROOT / "assets" / "cfa4"
# the freedesktop icon name an installed package provides (the same files, installed by packaging)
ICON_NAME = "io.github.ubuntuokos.cfa4"
LOGO = ASSETS / "cfa4-official-logo.png"
ICON_SIZES = (16, 22, 24, 32, 48, 64, 128, 256, 512)


def icon_file(size: int) -> Path:
    return ASSETS / "hicolor" / f"{size}x{size}" / "apps" / f"{ICON_NAME}.png"


def app_icon() -> QIcon:
    """The CFA4 icon (the logo's emblem) in every size of the checkout, else the installed icon
    theme entry, else empty."""
    icon = QIcon()
    for size in ICON_SIZES:
        path = icon_file(size)
        if path.is_file():
            icon.addFile(str(path), QSize(size, size))
    if not icon.isNull():
        return icon
    return QIcon.fromTheme(ICON_NAME)


def logo_pixmap(width: int) -> QPixmap:
    """The full CFA4 logo (emblem and wordmark) at ``width`` pixels, for the About box; the
    icon when the logo file is not there (an installed package ships only the icons)."""
    full = QPixmap(str(LOGO)) if LOGO.is_file() else QPixmap()
    if full.isNull():
        return app_icon().pixmap(width, width)
    return full.scaledToWidth(width, Qt.TransformationMode.SmoothTransformation)
