from __future__ import annotations

from pathlib import Path

from cfa4_devtools.health import missing_requirements
from cfa4_devtools.qt_app import engine_version, locate_engine


def test_missing_requirements_are_explicit() -> None:
    data = {
        "platform": "Linux",
        "architecture": "x86_64",
        "python_ok": True,
        "rustc": "rustc 1.95",
        "cargo": "cargo 1.95",
        "qt6_system": "6.8",
        "pyside6_installed": False,
        "ffmpeg": "ffmpeg",
        "ffprobe": "ffprobe",
    }
    assert missing_requirements(data) == []
    data["cargo"] = None
    data["qt6_system"] = None
    assert missing_requirements(data) == ["cargo", "Qt 6 (pkg-config Qt6Core or PySide6)"]
    data["architecture"] = "aarch64"
    assert "x86_64 (AMD64)" in missing_requirements(data)


def test_explicit_engine_path_is_checked(tmp_path: Path) -> None:
    assert locate_engine(str(tmp_path / "not-exists")) is None
    fake_binary = tmp_path / "cfa4-cli"
    fake_binary.write_text("placeholder", encoding="utf-8")
    assert locate_engine(str(fake_binary)) == fake_binary


def test_engine_version_reports_unbuilt_binary() -> None:
    assert "not built" in engine_version(None)
