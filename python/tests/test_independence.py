"""Prevent accidental reintroduction of upstream branding into CFA4."""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
APP_ID = "io.github.ubuntuokos.cfa4"
ACTIVE = [
    ROOT / "packaging/linux" / f"{APP_ID}.desktop",
    ROOT / "packaging/linux" / f"{APP_ID}.metainfo.xml.in",
    ROOT / "packaging/linux" / f"{APP_ID}.mime.xml",
    ROOT / "packaging/linux/flatpak" / f"{APP_ID}.yml",
    ROOT / "packaging/linux/flatpak" / f"{APP_ID}.bundle.yml",
    ROOT / "packaging/linux/nfpm.yaml",
]

# Built from parts so this file does not match its own patterns.
_CRAFT = "cr" + "aft"
BRAND = re.compile(
    rf"(?i)(film|art|photo|vector|pdf|effect|design|cad|light|sound){_CRAFT}"
    rf"|{_CRAFT}[-_ ]?(ui|libs|docpath|fonts|rules|common)"
    rf"|ai\.story" + "teller"
)

# Files that must keep upstream names: copyright/licence notices (MIT and Apache-2.0 require
# them), the provenance ledgers of vendored code and the plain-text provenance in README.md.
LEGAL = {
    "LICENSE-MIT",
    "LICENSE-APACHE",
    "NOTICE",
    "README.md",
    "python/cfa4_devtools/dependency_audit.py",
    # Read-only recognition of project/preset files saved by the upstream application (A1).
    "crates/project/src/legacy.rs",
}
LEGAL_NAMES = {"NOTICE", "LICENSE-MIT", "LICENSE-APACHE", "SOURCE_MANIFEST.json"}
SKIP_DIRS = {".git", "target", ".venv", "venv", "node_modules", "__pycache__", "plan", "build", "dist"}


def _repo_files() -> list[Path]:
    files = []
    for path in ROOT.rglob("*"):
        rel = path.relative_to(ROOT)
        if any(part in SKIP_DIRS or part.startswith(".pytest") for part in rel.parts):
            continue
        if path.is_file():
            files.append(path)
    return files


def test_own_linux_identity() -> None:
    assert all(f.is_file() for f in ACTIVE)
    for path in ACTIVE:
        text = path.read_text(encoding="utf-8")
        assert not BRAND.search(text), path


def test_no_upstream_brand_in_repository() -> None:
    offenders = []
    for path in _repo_files():
        rel = path.relative_to(ROOT).as_posix()
        if BRAND.search(rel):
            offenders.append(f"path: {rel}")
            continue
        if rel in LEGAL or path.name in LEGAL_NAMES:
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        match = BRAND.search(text)
        if match:
            line = text.count("\n", 0, match.start()) + 1
            offenders.append(f"{rel}:{line}: {match.group(0)}")
    assert not offenders, "upstream branding found:\n" + "\n".join(offenders[:50])


def test_new_icon_attributed() -> None:
    icon = ROOT / "assets/cfa4/hicolor/scalable/apps" / f"{APP_ID}.svg"
    assert icon.is_file() and icon.with_suffix(".svg.attribution").is_file()
    assert str(icon.relative_to(ROOT)) in (ROOT / "ATTRIBUTION.md").read_text(encoding="utf-8")


def test_release_gate_and_no_inherited_icon_in_package_script() -> None:
    script = (ROOT / "packaging/linux/package.sh").read_text(encoding="utf-8")
    assert "CFA4_ALLOW_LOCAL_PACKAGING" in script
    assert "assets/app-icon" not in script
    assert f"APP_ID={APP_ID}" in script
    assert 'BASENAME="cfa4-$VERSION-linux-$ARCH"' in script
