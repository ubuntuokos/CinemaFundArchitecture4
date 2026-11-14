"""Where Qt / PySide6 may be bundled (#220, F1-01; owner decision 2026-11-09, qt-parity 1.10).

The .deb and the tarball use PySide6 from Ubuntu's archive (python3-pyside6.*), whose packages carry
the LGPL-3.0 notices: they bundle nothing. The AppImage and the Flatpak, which cannot depend on
those packages, carry PySide6 / Qt 6, and only through packaging/linux/stage-pyside6.py: from the
hash-pinned wheels of a local wheelhouse (no pip install, no download), without Qt WebEngine
(Chromium's notices), with the LGPL texts and the source offer. The from-source Flatpak manifest, whose
build has no network, gets that wheelhouse as flatpak-builder `file` sources: exactly the wheels
stage-pyside6.py stages, each with a SHA-256 pinned in requirements/linux-amd64.txt, into the directory
its --wheels reads (stage-pyside6.py checks the hashes again; owner decision 2026-11-14, #1099).
A change to that must change
docs/audit/F1-01-SBOM-2026-10-25.md §4 together with this test.
"""

from __future__ import annotations

import importlib.util
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LINUX = ROOT / "packaging" / "linux"
QT_PAYLOAD = re.compile(r"pyside6|shiboken6|libqt6|qtwebengine|pip3?\s+install", re.IGNORECASE)
# the only Qt payload lines a package script or manifest may have: staging through the one script,
# into (or from) the one directory, with its licences
STAGED = re.compile(r"stage-pyside6\.py|lib/cfa4/pyside6|share/doc/cfa4/pyside6|CFA4_PYSIDE6_WHEELS")
FORBIDDEN = re.compile(r"qtwebengine|pip3?\s+(install|download)|curl |wget ", re.IGNORECASE)
# a flatpak-builder source of the wheelhouse: the wheel's url (then its sha256 and dest lines)
WHEEL_URL = re.compile(r"^\s*url:\s*https://files\.pythonhosted\.org/packages/\S+/(?P<file>[^/\s]+\.whl)\s*$")


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


def locked_wheels() -> dict[str, set[str]]:
    """``distribution -> {sha256, …}`` of the wheels stage-pyside6.py stages, from the lockfile."""
    return stage_module().pinned_hashes(ROOT / "requirements" / "linux-amd64.txt")


def flatpak_wheel_sources(path: Path) -> tuple[list[tuple[str, str, str]], set[str]]:
    """A manifest's wheel sources as ``(file name, sha256, dest)``, and the lines they take up."""
    lines = code_lines(path)
    sources, taken = [], set()
    for i, line in enumerate(lines):
        m = WHEEL_URL.match(line)
        if not m:
            continue
        fields = {}
        for following in lines[i + 1 : i + 3]:
            f = re.match(r"^\s*(sha256|dest):\s*(\S+)\s*$", following)
            if not f:
                break
            fields[f.group(1)] = f.group(2)
            taken.add(following)
        sources.append((m.group("file"), fields.get("sha256", ""), fields.get("dest", "")))
        taken.add(line)
    return sources, taken


def stage_module():
    spec = importlib.util.spec_from_file_location("stage_pyside6", LINUX / "stage-pyside6.py")
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


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


def test_tarball_installer_copies_only_cfa4_code() -> None:
    for script in (LINUX / "install.sh", ROOT / "packaging" / "env.sh"):
        for line in code_lines(script):
            assert not QT_PAYLOAD.search(line), f"{script.name}: {line.strip()}"


def test_appimage_and_flatpak_take_qt_only_through_the_staging_script() -> None:
    files = [LINUX / "package.sh", LINUX / "flatpak-bundle.sh", *sorted((LINUX / "flatpak").glob("*.yml"))]
    for path in files:
        # the pinned wheel sources are checked line by line in the next test
        wheel_lines = flatpak_wheel_sources(path)[1] if path.suffix == ".yml" else set()
        for line in code_lines(path):
            if QT_PAYLOAD.search(line) and line not in wheel_lines:
                assert STAGED.search(line) and not FORBIDDEN.search(line), f"{path.name}: {line.strip()}"
    package = (LINUX / "package.sh").read_text(encoding="utf-8")
    appimage = package[package.index("# ---- AppImage") :]
    tarball = package[package.index("# ---- .tar.gz") : package.index("# ---- AppImage")]
    assert "stage-pyside6.py" in appimage and "stage-pyside6.py" not in tarball
    assert "stage-pyside6.py" in (LINUX / "flatpak-bundle.sh").read_text(encoding="utf-8")


def test_flatpak_wheel_sources_are_the_locked_wheels_for_the_staging_script() -> None:
    locked = locked_wheels()
    for path in sorted((LINUX / "flatpak").glob("*.yml")):
        sources, _ = flatpak_wheel_sources(path)
        if not sources:
            continue
        text = "\n".join(code_lines(path))
        read_from = set(re.findall(r"stage-pyside6\.py\s+--wheels\s+(\S+)", text))
        assert len(read_from) == 1, f"{path.name}: wheel sources, but not one stage-pyside6.py --wheels"
        names = set()
        for file, digest, dest in sources:
            name = file.split("-", 1)[0].replace("_", "-").lower()
            names.add(name)
            assert name in locked, f"{path.name}: {file} is not a wheel stage-pyside6.py stages"
            assert digest in locked[name], f"{path.name}: {file} sha256 {digest!r} is not pinned in the lockfile"
            assert dest in read_from, f"{path.name}: {file} goes to {dest!r}, not where stage-pyside6.py reads"
        assert names == set(locked), (
            f"{path.name}: wheel sources {sorted(names)}, stage-pyside6.py stages {sorted(locked)}"
        )


def test_wheel_source_pattern_takes_only_pypi_wheels() -> None:
    assert WHEEL_URL.match(
        "        url: https://files.pythonhosted.org/packages/7b/3c/x/shiboken6-6.12.0-cp310-abi3-manylinux_2_34_x86_64.whl"
    )
    assert not WHEEL_URL.match("        url: https://example.org/shiboken6-6.12.0-cp310-abi3-manylinux_2_34_x86_64.whl")
    assert not WHEEL_URL.match("        url: https://files.pythonhosted.org/packages/7b/3c/x/PySide6-6.12.0.tar.gz")


def test_staging_script_is_offline_pinned_and_leaves_out_webengine() -> None:
    text = (LINUX / "stage-pyside6.py").read_text(encoding="utf-8")
    assert not re.search(r"urllib|http\.client|requests|pip3?\s+(install|download)|subprocess\.run\(\[\"pip", text)
    assert "--hash=sha256:" in text and "refusing it" in text
    stage = stage_module()
    assert not any("WebEngine" in m or "Quick" in m or "Qml" in m for m in stage.MODULES)
    assert not stage.wanted("PySide6/QtWebEngineCore.abi3.so")
    assert not stage.wanted("PySide6/Qt/libexec/QtWebEngineProcess")
    assert not stage.wanted("PySide6/Qt/resources/qtwebengine_resources.pak")
    assert stage.wanted("PySide6/QtWidgets.abi3.so") and stage.wanted("PySide6/Qt/plugins/platforms/libqxcb.so")


def test_payload_pattern_catches_a_bundled_wheel() -> None:
    assert QT_PAYLOAD.search("  - src: ${STAGE}/usr/lib/cfa4/python/PySide6/")
    assert QT_PAYLOAD.search('pip install --target "$QT_PY" PySide6')
    assert QT_PAYLOAD.search("cp libQt6WebEngineCore.so.6 $APPDIR/usr/lib")
    assert not QT_PAYLOAD.search('cp -R "$ROOT/python/cfa4_qt" "$QT_PY/"')
    assert not STAGED.search('pip install --target "$QT_PY" PySide6')
