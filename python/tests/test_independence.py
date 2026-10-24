"""Prevent accidental reintroduction of FilmCraft distribution branding in CFA4 metadata."""

from __future__ import annotations

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


def test_own_linux_identity() -> None:
    assert all(f.is_file() for f in ACTIVE)
    for path in ACTIVE:
        text = path.read_text(encoding="utf-8")
        assert "ai.storyteller.filmcraft" not in text, path
        assert "getartcraft.com" not in text, path
        assert "https://github.com/storytold/filmcraft/issues" not in text, path


def test_new_icon_attributed() -> None:
    icon = ROOT / "assets/cfa4/hicolor/scalable/apps" / f"{APP_ID}.svg"
    assert icon.is_file() and icon.with_suffix(".svg.attribution").is_file()
    assert str(icon.relative_to(ROOT)) in (ROOT / "ATTRIBUTION.md").read_text(encoding="utf-8")


def test_release_gate_and_no_inherited_icon_in_package_script() -> None:
    script = (ROOT / "packaging/linux/package.sh").read_text(encoding="utf-8")
    assert "CFA4_ALLOW_LOCAL_PACKAGING" in script
    assert "assets/app-icon/hicolor" not in script
    assert f"APP_ID={APP_ID}" in script
    assert 'BASENAME="cfa4-$VERSION-linux-$ARCH"' in script
