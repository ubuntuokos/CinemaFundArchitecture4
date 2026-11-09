"""No release format bundles Qt, PySide6 or Qt WebEngine (#220, F1-01).

The Qt GUI uses PySide6 from Ubuntu's archive (python3-pyside6.*); Ubuntu's packages carry the
LGPL-3.0 and the Qt WebEngine (Chromium) third-party notices. Bundling any of it would make those
notices and the LGPL relinking obligation CFA4's own, so the packaging must change together with
docs/audit/F1-01-SBOM-2026-10-25.md §4 before this test may.
"""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LINUX = ROOT / "packaging" / "linux"
QT_PAYLOAD = re.compile(r"pyside6|shiboken6|libqt6|qtwebengine|pip3?\s+install", re.IGNORECASE)


def code_lines(path: Path) -> list[str]:
    """The file's lines without shell/YAML comments."""
    lines = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.lstrip().startswith("#"):
            lines.append(line)
    return lines


def nfpm_contents(path: Path) -> list[str]:
    """The src/dst lines of an nfpm manifest's contents section."""
    lines, inside = [], False
    for line in code_lines(path):
        if not line.startswith((" ", "\t")):
            inside = line.startswith("contents:")
        elif inside and re.match(r"\s*-?\s*(src|dst):", line):
            lines.append(line)
    return lines


def test_deb_ships_no_qt_payload() -> None:
    for manifest in ("nfpm.yaml", "nfpm-restricted.yaml"):
        contents = nfpm_contents(LINUX / manifest)
        assert contents, f"{manifest}: no contents entries parsed"
        for line in contents:
            assert not QT_PAYLOAD.search(line), f"{manifest}: {line.strip()}"


def test_deb_takes_pyside6_from_ubuntu() -> None:
    text = (LINUX / "nfpm.yaml").read_text(encoding="utf-8")
    for module in ("qtcore", "qtgui", "qtwidgets"):
        assert f"python3-pyside6.{module}" in text


def test_staging_copies_only_cfa4_code() -> None:
    # package.sh stages every format (tar, deb, AppImage, Flatpak bundle); install.sh installs the tar.
    scripts = [LINUX / name for name in ("package.sh", "install.sh", "flatpak-bundle.sh")]
    for script in [*scripts, ROOT / "packaging" / "env.sh"]:
        for line in code_lines(script):
            assert not QT_PAYLOAD.search(line), f"{script.name}: {line.strip()}"


def test_flatpak_manifests_add_no_qt_module() -> None:
    for manifest in sorted((LINUX / "flatpak").glob("*.yml")):
        for line in code_lines(manifest):
            assert not QT_PAYLOAD.search(line), f"{manifest.name}: {line.strip()}"


def test_payload_pattern_catches_a_bundled_wheel() -> None:
    assert QT_PAYLOAD.search("  - src: ${STAGE}/usr/lib/cfa4/python/PySide6/")
    assert QT_PAYLOAD.search('pip install --target "$QT_PY" PySide6')
    assert QT_PAYLOAD.search("cp libQt6WebEngineCore.so.6 $APPDIR/usr/lib")
    assert not QT_PAYLOAD.search('cp -R "$ROOT/python/cfa4_qt" "$QT_PY/"')
